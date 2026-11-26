//! Garde-fous HTTP transverses (équivalent de `security.py`) : pré-authentification des envois de
//! fichiers et en-têtes de sécurité.
//!
//! Pourquoi un garde en amont pour `/api/media` : le corps multipart est volumineux. Sans ce garde, un
//! client anonyme pourrait faire lire/stocker jusqu'à la taille maximale avant de recevoir 401. Ici, ni
//! l'authentification complète ni la taille déclarée ne nécessitent de lire le corps.

use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::app::AppState;
use crate::audit::log_action;
use crate::auth::{resolve_token, role_allows};
use crate::deps::{bearer_from_headers, ClientIp};
use crate::error::ApiError;

pub const UPLOAD_PATH: &str = "/api/media";
/// Marge pour l'enveloppe multipart autour du fichier.
pub const MULTIPART_OVERHEAD: u64 = 64 * 1024;

/// CSP stricte : tout est servi par la même origine, aucun script/style distant ni en ligne
/// (les attributs `style=""` posés par l'interface imposent 'unsafe-inline' pour les styles seulement).
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
img-src 'self' data: blob:; media-src 'self' blob:; object-src 'none'; \
base-uri 'none'; form-action 'self'; frame-ancestors 'self'";

pub const SECURITY_HEADERS: [(&str, &str); 4] = [
    ("x-content-type-options", "nosniff"),
    ("x-frame-options", "SAMEORIGIN"),
    ("referrer-policy", "no-referrer"),
    ("cross-origin-resource-policy", "same-origin"),
];

/// Ce qu'on lit de la requête avant d'avoir le droit de toucher à son corps.
/// (Extrait de façon synchrone : `Request` n'est pas `Sync`, on ne le garde pas à travers un `.await`.)
struct UploadHead {
    ip: String,
    token: Option<String>,
    declared_len: Option<u64>,
}

fn upload_head(req: &Request) -> UploadHead {
    let declared_len = req
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|v| v.parse::<u64>().ok());
    UploadHead {
        ip: req.extensions().get::<ClientIp>().map(|c| c.0.clone()).unwrap_or_else(|| "inconnu".into()),
        token: bearer_from_headers(req.headers()),
        declared_len,
    }
}

/// Authentifie (rôle editor minimum) et borne la taille AVANT toute lecture du corps.
/// Renvoie `Some(réponse)` pour refuser, `None` pour laisser passer.
async fn precheck_upload(st: &AppState, head: UploadHead) -> Option<Response> {
    let s = &st.settings;
    let UploadHead { ip, token, declared_len } = head;
    let key = format!("authfail:{ip}");
    if token.is_some() {
        let retry = st.limiter.blocked(&key, s.rate_limit_authfail);
        if retry > 0 {
            return Some(ApiError::too_many_requests(retry).into_response());
        }
    }
    let principal = match &token {
        Some(t) => match resolve_token(&st.db, t, &s.admin_token, s.legacy_token_always).await {
            Ok(p) => p,
            Err(e) => return Some(ApiError::from(e).into_response()),
        },
        None => None,
    };
    let Some(principal) = principal else {
        if token.is_some() {
            st.limiter.check(&key, s.rate_limit_authfail);
            if let Err(e) = log_action(&st.db, &ip, None, "auth_echouee", Some("bearer".into()), None).await {
                return Some(ApiError::from(e).into_response());
            }
        }
        return Some(ApiError::unauthenticated().into_response());
    };
    if !role_allows(&principal.role, "editor") {
        let logged = log_action(
            &st.db,
            &ip,
            Some(&principal),
            "acces_refuse",
            Some(format!("POST {UPLOAD_PATH}")),
            Some("rôle requis : editor".into()),
        )
        .await;
        if let Err(e) = logged {
            return Some(ApiError::from(e).into_response());
        }
        return Some(ApiError::new(403, "Rôle insuffisant pour cette opération").into_response());
    }
    if let Some(len) = declared_len {
        if len > s.max_upload_bytes().saturating_add(MULTIPART_OVERHEAD) {
            return Some(ApiError::new(413, "Fichier trop volumineux").into_response());
        }
    }
    None
}

pub async fn upload_guard(State(st): State<AppState>, req: Request, next: Next) -> Response {
    if req.method() == Method::POST && req.uri().path() == UPLOAD_PATH {
        let head = upload_head(&req);
        if let Some(rejected) = precheck_upload(&st, head).await {
            return rejected;
        }
    }
    next.run(req).await
}

/// Ajoute les en-têtes de sécurité à toute réponse (sans écraser ceux déjà posés par une route).
pub async fn security_headers(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    for (name, value) in SECURITY_HEADERS {
        let name = HeaderName::from_static(name);
        if !headers.contains_key(&name) {
            headers.insert(name, HeaderValue::from_static(value));
        }
    }
    let csp = HeaderName::from_static("content-security-policy");
    if !headers.contains_key(&csp) {
        headers.insert(csp, HeaderValue::from_static(CSP));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csp_is_strict() {
        for needle in ["default-src 'self'", "script-src 'self'", "object-src 'none'", "base-uri 'none'", "frame-ancestors 'self'"] {
            assert!(CSP.contains(needle), "{needle}");
        }
        assert!(!CSP.contains("script-src 'self' 'unsafe-inline'") && !CSP.contains("unsafe-eval"));
        assert!(!CSP.contains('\n'), "la CSP doit tenir sur une ligne (valeur d'en-tête)");
        HeaderValue::from_str(CSP).expect("valeur d'en-tête valide");
    }

    #[test]
    fn security_header_table_is_valid_and_complete() {
        let names: Vec<&str> = SECURITY_HEADERS.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            ["x-content-type-options", "x-frame-options", "referrer-policy", "cross-origin-resource-policy"]
        );
        for (n, v) in SECURITY_HEADERS {
            HeaderName::from_static(n);
            HeaderValue::from_static(v);
        }
    }

    #[test]
    fn upload_limits() {
        assert_eq!(MULTIPART_OVERHEAD, 65_536);
        assert_eq!(UPLOAD_PATH, "/api/media");
    }
}
