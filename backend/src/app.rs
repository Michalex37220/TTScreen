//! Assemblage de l'application (équivalent de `main.py`).

use axum::middleware;
use axum::Router;
use sea_orm::{DatabaseConnection, EntityTrait, PaginatorTrait};
use std::collections::HashSet;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use crate::config::Settings;
use crate::proxy::{client_ip_middleware, TrustedProxies};
use crate::ratelimit::{rate_limit_middleware, RateLimiter};
use crate::security::{security_headers, upload_guard};
use crate::{api, auth, db};

/// État partagé par toutes les routes (clonable à bas coût).
#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<Settings>,
    pub db: DatabaseConnection,
    pub limiter: Arc<RateLimiter>,
    pub trusted_proxies: Arc<TrustedProxies>,
    /// Fuseau horaire des plannings (`TTSCREEN_TIMEZONE`).
    pub tz: chrono_tz::Tz,
    /// Haché factice (même coût PBKDF2) vérifié quand l'identifiant n'existe pas.
    pub dummy_hash: Arc<String>,
    /// Nombre de vérifications de mot de passe effectuées (permet de tester l'égalisation des temps de connexion).
    pub password_verifications: Arc<AtomicU64>,
}

/// Contrôle du jeton admin historique dans TOUS les environnements : un jeton vide désactive simplement
/// le jeton historique ; sinon 24 caractères minimum et variés.
pub fn check_admin_token(token: &str) -> anyhow::Result<()> {
    if token.is_empty() {
        return Ok(());
    }
    let distinct = token.chars().collect::<HashSet<char>>().len();
    if token == "dev-admin-token" || token.chars().count() < 24 || distinct < 10 {
        anyhow::bail!(
            "TTSCREEN_ADMIN_TOKEN trop faible : 24 caractères minimum et variés \
             (openssl rand -base64 32), ou laissez-le vide"
        );
    }
    Ok(())
}

/// Vérifie la configuration, prépare dossiers et base, crée le compte d'amorçage.
/// Échoue (comme le `RuntimeError` d'origine) si la configuration est dangereuse.
pub async fn build_state(settings: Settings) -> anyhow::Result<AppState> {
    check_admin_token(&settings.admin_token)?;
    let tz: chrono_tz::Tz = settings.timezone.parse().map_err(|e| {
        anyhow::anyhow!("TTSCREEN_TIMEZONE invalide ({:?}) : {e} — utilisez un nom IANA, ex. Europe/Paris", settings.timezone)
    })?;
    tokio::fs::create_dir_all(&settings.media_dir).await?;
    if let Some(file) = db::sqlite_file(&settings.database_url) {
        if let Some(parent) = file.parent().filter(|p| !p.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(parent).await?;
        }
    }
    let conn = db::connect(&settings.database_url).await?;
    db::init_schema(&conn).await?;

    // Compte admin d'amorçage facultatif : permet de créer un premier compte nommé sans connaître
    // le jeton historique
    auth::bootstrap_admin(
        &conn,
        &settings.bootstrap_admin_user,
        &settings.bootstrap_admin_password,
        settings.bootstrap_admin_reset,
        settings.pbkdf2_iterations,
    )
    .await?;
    if settings.admin_token.is_empty() && crate::models::user::Entity::find().count(&conn).await? == 0 {
        let msg = "Aucun moyen de se connecter : définissez TTSCREEN_ADMIN_TOKEN ou \
                   TTSCREEN_BOOTSTRAP_ADMIN_USER/PASSWORD";
        if settings.is_production() {
            anyhow::bail!(msg);
        }
        tracing::warn!("{msg}");
    }

    let iterations = settings.pbkdf2_iterations;
    let dummy = tokio::task::spawn_blocking(move || auth::dummy_hash(iterations)).await?;
    Ok(AppState {
        limiter: Arc::new(RateLimiter::new(settings.rate_window_s)),
        trusted_proxies: Arc::new(TrustedProxies::parse(&settings.forwarded_allow_ips)),
        tz,
        settings: Arc::new(settings),
        db: conn,
        dummy_hash: Arc::new(dummy),
        password_verifications: Arc::new(AtomicU64::new(0)),
    })
}

/// Routes + middlewares. Ordre (le dernier ajouté est le plus externe), comme à l'origine :
/// IP client → en-têtes de sécurité → limitation de débit → pré-authentification des envois → route.
pub fn build_router(state: AppState) -> Router {
    api::router(&state)
        .fallback(api::public::not_found)
        .layer(middleware::from_fn_with_state(state.clone(), upload_guard))
        .layer(middleware::from_fn_with_state(state.clone(), rate_limit_middleware))
        .layer(middleware::from_fn(security_headers))
        .layer(middleware::from_fn_with_state(state.clone(), client_ip_middleware))
        .with_state(state)
}

/// Raccourci : état + routeur.
pub async fn create_app(settings: Settings) -> anyhow::Result<Router> {
    Ok(build_router(build_state(settings).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_admin_token_is_allowed_and_means_disabled() {
        assert!(check_admin_token("").is_ok());
    }

    #[test]
    fn weak_admin_tokens_are_refused() {
        assert!(check_admin_token("dev-admin-token").is_err());
        assert!(check_admin_token("court").is_err());
        assert!(check_admin_token(&"x".repeat(32)).is_err(), "assez long mais sans variété");
        let nine = "abcdefgh_".repeat(4); // 36 caractères mais 9 distincts seulement
        assert_eq!(nine.chars().collect::<std::collections::HashSet<_>>().len(), 9);
        assert!(check_admin_token(&nine).is_err(), "9 caractères distincts seulement");
        let ten = "abcdefghi_".repeat(3); // 30 caractères, 10 distincts : exactement le minimum accepté
        assert!(check_admin_token(&ten).is_ok());
        assert!(check_admin_token("Zq8vN3xK1mP7wR5tY2uL9aB").is_err(), "23 caractères");
    }

    #[test]
    fn strong_admin_tokens_are_accepted() {
        assert!(check_admin_token("Zq8vN3xK1mP7wR5tY2uL9aB4").is_ok());
        assert!(check_admin_token("test-admin-token-0123456789abcdef").is_ok());
        assert!(check_admin_token(&crate::auth::new_token()).is_ok());
    }
}
