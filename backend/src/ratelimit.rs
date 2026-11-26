//! Limitation de débit en mémoire (équivalent de `ratelimit.py`).
//!
//! Fenêtre glissante par clé : chaque requête ajoute son horodatage ; si la file contient déjà
//! `limit` requêtes dans la fenêtre, la requête est refusée (429 + `Retry-After`).
//! La mémoire est locale au processus ; pour plusieurs instances, remplacer `check_at`
//! par un compteur partagé (Redis).

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::app::AppState;
use crate::auth::hash_token;
use crate::deps::{bearer_from_headers, ClientIp};
use crate::error::ApiError;

/// Au-delà de ce nombre de clés, un nettoyage des clés mortes est tenté (au plus une fois par fenêtre).
const GC_THRESHOLD: usize = 10_000;

struct Inner {
    hits: HashMap<String, VecDeque<Instant>>,
    last_gc: Option<Instant>,
    gc_runs: usize,
}

pub struct RateLimiter {
    window: Duration,
    inner: Mutex<Inner>,
}

fn retry_after(window: Duration, oldest_age: Duration) -> u64 {
    let remaining = window.as_secs_f64() - oldest_age.as_secs_f64();
    (remaining as i64 + 1).max(1) as u64
}

impl RateLimiter {
    pub fn new(window_s: u64) -> Self {
        RateLimiter {
            window: Duration::from_secs(window_s),
            inner: Mutex::new(Inner { hits: HashMap::new(), last_gc: None, gc_runs: 0 }),
        }
    }

    /// Comme `check` mais SANS enregistrer de requête : secondes d'attente si `key` a déjà atteint
    /// `limit` dans la fenêtre, sinon 0. Sert à bloquer après N échecs enregistrés.
    pub fn blocked(&self, key: &str, limit: i64) -> u64 {
        self.blocked_at(key, limit, Instant::now())
    }

    pub fn blocked_at(&self, key: &str, limit: i64, now: Instant) -> u64 {
        if limit <= 0 {
            return 0;
        }
        let window = self.window;
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let Some(dq) = guard.hits.get_mut(key) else { return 0 };
        while let Some(front) = dq.front() {
            if now.saturating_duration_since(*front) >= window {
                dq.pop_front();
            } else {
                break;
            }
        }
        match dq.front() {
            Some(oldest) if dq.len() as i64 >= limit => retry_after(window, now.saturating_duration_since(*oldest)),
            _ => 0,
        }
    }

    /// Enregistre une requête pour `key` : 0 si elle est autorisée, sinon les secondes à attendre.
    pub fn check(&self, key: &str, limit: i64) -> u64 {
        self.check_at(key, limit, Instant::now())
    }

    pub fn check_at(&self, key: &str, limit: i64, now: Instant) -> u64 {
        if limit <= 0 {
            return 0; // limite désactivée (0 = illimité)
        }
        let window = self.window;
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let inner = &mut *guard;
        let dq = inner.hits.entry(key.to_string()).or_default();
        while let Some(front) = dq.front() {
            if now.saturating_duration_since(*front) >= window {
                dq.pop_front();
            } else {
                break;
            }
        }
        if let Some(oldest) = dq.front() {
            if dq.len() as i64 >= limit {
                return retry_after(window, now.saturating_duration_since(*oldest));
            }
        }
        dq.push_back(now);
        // Nettoyage des clés mortes, au plus une fois par fenêtre : pas de O(n) à chaque requête sous le verrou.
        let due = inner.last_gc.map_or(true, |t| now.saturating_duration_since(t) >= window);
        if inner.hits.len() > GC_THRESHOLD && due {
            inner.hits.retain(|_, v| v.back().map_or(false, |last| now.saturating_duration_since(*last) < window));
            inner.last_gc = Some(now);
            inner.gc_runs += 1;
        }
        0
    }

    pub fn key_count(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).hits.len()
    }

    pub fn gc_runs(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).gc_runs
    }
}

/// Middleware HTTP de limitation de débit.
///
/// - `/api/health` n'est jamais limité (sonde de supervision) ;
/// - `POST /api/auth/login` : limite stricte **par IP** (anti force brute) ;
/// - `/api/player/manifest` : limite **par jeton d'écran** ET limite générale par IP, pour qu'un client
///   qui varie le jeton à chaque requête ne contourne pas le quota ni ne fasse gonfler la table ;
/// - le reste de `/api` : limite générale par IP.
pub async fn rate_limit_middleware(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    if !path.starts_with("/api") || path == "/api/health" {
        return next.run(req).await;
    }
    let s = &st.settings;
    let ip = req.extensions().get::<ClientIp>().map(|c| c.0.clone()).unwrap_or_else(|| "inconnu".into());
    let retry = if path == "/api/auth/login" {
        st.limiter.check(&format!("login:{ip}"), s.rate_limit_login)
    } else if path == "/api/player/manifest" {
        // D'abord le quota par IP (borne le nombre de clés créables), puis celui par jeton
        let by_ip = st.limiter.check(&format!("ip:{ip}"), s.rate_limit_api);
        if by_ip > 0 {
            by_ip
        } else {
            let token = bearer_from_headers(req.headers()).unwrap_or_default();
            st.limiter.check(&format!("screen:{}", hash_token(&token)), s.rate_limit_player)
        }
    } else {
        st.limiter.check(&format!("ip:{ip}"), s.rate_limit_api)
    };
    if retry > 0 {
        return ApiError::too_many_requests(retry).into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_the_limit_then_blocks() {
        let rl = RateLimiter::new(60);
        let t = Instant::now();
        for _ in 0..5 {
            assert_eq!(rl.check_at("k", 5, t), 0);
        }
        assert!(rl.check_at("k", 5, t) > 0);
        assert!(rl.check_at("k", 5, t) > 0);
    }

    #[test]
    fn keys_are_independent() {
        let rl = RateLimiter::new(60);
        let t = Instant::now();
        assert_eq!(rl.check_at("a", 1, t), 0);
        assert!(rl.check_at("a", 1, t) > 0);
        assert_eq!(rl.check_at("b", 1, t), 0);
    }

    #[test]
    fn window_slides_and_requests_expire() {
        let rl = RateLimiter::new(60);
        let t = Instant::now();
        assert_eq!(rl.check_at("k", 2, t), 0);
        assert_eq!(rl.check_at("k", 2, t + Duration::from_secs(30)), 0);
        assert!(rl.check_at("k", 2, t + Duration::from_secs(59)) > 0);
        // la première requête (t) sort de la fenêtre à t+60 : une place se libère
        assert_eq!(rl.check_at("k", 2, t + Duration::from_secs(60)), 0);
        assert!(rl.check_at("k", 2, t + Duration::from_secs(61)) > 0);
    }

    #[test]
    fn retry_after_is_at_least_one_and_bounded_by_the_window() {
        let rl = RateLimiter::new(60);
        let t = Instant::now();
        rl.check_at("k", 1, t);
        let r = rl.check_at("k", 1, t + Duration::from_secs(10));
        assert!((40..=51).contains(&r), "retry = {r}");
        let r = rl.check_at("k", 1, t + Duration::from_millis(59_999));
        assert_eq!(r, 1);
    }

    #[test]
    fn zero_or_negative_limit_means_unlimited() {
        let rl = RateLimiter::new(60);
        for _ in 0..1000 {
            assert_eq!(rl.check("k", 0), 0);
            assert_eq!(rl.check("k", -3), 0);
        }
        assert_eq!(rl.key_count(), 0, "rien n'est enregistré quand la limite est désactivée");
    }

    #[test]
    fn blocked_does_not_record_anything() {
        let rl = RateLimiter::new(60);
        let t = Instant::now();
        for _ in 0..10 {
            assert_eq!(rl.blocked_at("k", 3, t), 0);
        }
        assert_eq!(rl.key_count(), 0);
        for _ in 0..3 {
            rl.check_at("k", 3, t);
        }
        assert!(rl.blocked_at("k", 3, t) > 0, "3 échecs enregistrés : bloqué");
        assert_eq!(rl.blocked_at("k", 4, t), 0, "sous la limite : pas bloqué");
        assert_eq!(rl.blocked_at("k", 0, t), 0, "limite désactivée");
        assert_eq!(rl.blocked_at("k", 3, t + Duration::from_secs(61)), 0, "fenêtre écoulée");
    }

    #[test]
    fn gc_is_not_run_on_every_request() {
        let rl = RateLimiter::new(60);
        let t = Instant::now();
        for i in 0..10_100 {
            rl.check_at(&format!("k{i}"), 5, t);
        }
        assert_eq!(rl.gc_runs(), 1, "un seul nettoyage dès le dépassement du seuil");
        let start = std::time::Instant::now();
        for i in 0..300 {
            rl.check_at(&format!("z{i}"), 5, t);
        }
        assert_eq!(rl.gc_runs(), 1, "pas de reconstruction à chaque requête");
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn gc_removes_dead_keys_once_the_window_has_passed() {
        let rl = RateLimiter::new(60);
        let t = Instant::now();
        for i in 0..10_001 {
            rl.check_at(&format!("old{i}"), 5, t);
        }
        let later = t + Duration::from_secs(120);
        rl.check_at("fresh", 5, later);
        assert!(rl.key_count() < 100, "les clés expirées sont purgées ({} restantes)", rl.key_count());
        assert!(rl.check_at("fresh", 2, later) == 0, "la clé récente continue de fonctionner après le nettoyage");
    }

    #[test]
    fn limiter_is_usable_from_several_threads() {
        let rl = std::sync::Arc::new(RateLimiter::new(60));
        let t = Instant::now();
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let rl = rl.clone();
                std::thread::spawn(move || (0..50).filter(|_| rl.check_at("shared", 100, t) == 0).count())
            })
            .collect();
        let allowed: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(allowed, 100, "exactement `limit` requêtes passent, même en concurrence");
    }
}
