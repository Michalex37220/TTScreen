//! Dépendances de requête (équivalent de `deps.py`) : IP client, jeton Bearer, contrôle d'accès par rôle,
//! authentification des écrans.
//!
//! En FastAPI ce sont des `Depends(...)` ; ici ce sont des extracteurs axum. Dans chaque gestionnaire,
//! l'extracteur d'authentification est placé AVANT ceux du corps : un client non authentifié reçoit
//! 401/403 sans que son corps soit analysé, et l'authentification passe avant la validation (422).

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::http::HeaderMap;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::convert::Infallible;

use crate::app::AppState;
use crate::audit::log_action;
use crate::auth::{hash_token, resolve_token, role_allows, Principal, ROLES};
use crate::error::ApiError;
use crate::models::screen;

/// Les en-têtes HTTP sont décodés en latin-1 (comme Starlette) : un octet non ASCII donne un jeton
/// invalide (401) et jamais une erreur 500.
pub fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// Jeton d'une valeur d'en-tête `Authorization: Bearer <jeton>` ; `None` si absent, mal formé ou vide.
pub fn bearer(value: Option<&str>) -> Option<String> {
    let v = value?;
    match v.get(..7) {
        Some(prefix) if prefix.eq_ignore_ascii_case("bearer ") => {
            let token = v[7..].trim();
            if token.is_empty() {
                None
            } else {
                Some(token.to_string())
            }
        }
        _ => None,
    }
}

pub fn bearer_from_headers(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(AUTHORIZATION).map(|v| latin1(v.as_bytes()));
    bearer(raw.as_deref())
}

/// IP du client, calculée par `proxy::client_ip_middleware`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientIp(pub String);

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(parts.extensions.get::<ClientIp>().cloned().unwrap_or_else(|| ClientIp("inconnu".to_string())))
    }
}

pub fn client_ip_of(parts: &Parts) -> String {
    parts.extensions.get::<ClientIp>().map(|c| c.0.clone()).unwrap_or_else(|| "inconnu".to_string())
}

/// Jeton Bearer brut de la requête (ou `None`).
pub struct Bearer(pub Option<String>);

impl<S: Send + Sync> FromRequestParts<S> for Bearer {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Bearer(bearer_from_headers(&parts.headers)))
    }
}

/// 429 si `key` a atteint `limit` échecs enregistrés dans la fenêtre (anti force brute).
pub fn reject_if_blocked(st: &AppState, key: &str, limit: i64) -> Result<(), ApiError> {
    let retry = st.limiter.blocked(key, limit);
    if retry > 0 {
        return Err(ApiError::too_many_requests(retry));
    }
    Ok(())
}

/// Compte un jeton invalide pour l'IP appelante et le journalise (sans jamais écrire le jeton).
pub async fn record_auth_failure(st: &AppState, ip: &str, kind: &str) -> Result<(), ApiError> {
    st.limiter.check(&format!("authfail:{ip}"), st.settings.rate_limit_authfail);
    log_action(&st.db, ip, None, "auth_echouee", Some(kind.to_string()), None).await?;
    Ok(())
}

/// Résout le jeton Bearer en identité (compte nommé ou jeton admin historique), ou 401.
///
/// Base de tout le contrôle d'accès. Les jetons invalides sont comptés par IP : au-delà du seuil, 429
/// (sur toutes les routes, pas seulement /login, sinon on pourrait y deviner le jeton admin).
pub async fn authenticate(st: &AppState, ip: &str, token: Option<&str>) -> Result<Principal, ApiError> {
    let s = &st.settings;
    if let Some(token) = token {
        reject_if_blocked(st, &format!("authfail:{ip}"), s.rate_limit_authfail)?;
        if let Some(principal) = resolve_token(&st.db, token, &s.admin_token, s.legacy_token_always).await? {
            return Ok(principal);
        }
        record_auth_failure(st, ip, "bearer").await?;
    }
    Err(ApiError::unauthenticated())
}

/// Exige au moins le rôle d'indice `MIN` (0 = viewer, 1 = editor, 2 = admin).
///
/// Hiérarchie : viewer (lecture seule) < editor (création/modification du contenu) < admin (comptes, journal).
pub struct Guard<const MIN: usize>(pub Principal);

pub type Viewer = Guard<0>;
pub type Editor = Guard<1>;
pub type Admin = Guard<2>;

impl<const MIN: usize> FromRequestParts<AppState> for Guard<MIN> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, Self::Rejection> {
        let ip = client_ip_of(parts);
        let token = bearer_from_headers(&parts.headers);
        let principal = authenticate(st, &ip, token.as_deref()).await?;
        let minimum = ROLES.get(MIN).copied().unwrap_or("admin");
        if !role_allows(&principal.role, minimum) {
            log_action(
                &st.db,
                &ip,
                Some(&principal),
                "acces_refuse",
                Some(format!("{} {}", parts.method, parts.uri.path())),
                Some(format!("rôle requis : {minimum}")),
            )
            .await?;
            return Err(ApiError::new(403, "Rôle insuffisant pour cette opération"));
        }
        Ok(Guard(principal))
    }
}

/// Écran authentifié par son jeton propre (`/api/player/manifest`) : aucun droit d'administration.
pub struct ScreenAuth(pub screen::Model);

impl FromRequestParts<AppState> for ScreenAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, Self::Rejection> {
        let ip = client_ip_of(parts);
        if let Some(token) = bearer_from_headers(&parts.headers) {
            reject_if_blocked(st, &format!("authfail:{ip}"), st.settings.rate_limit_authfail)?;
            let found = screen::Entity::find()
                .filter(screen::Column::TokenHash.eq(hash_token(&token)))
                .one(&st.db)
                .await?;
            match found {
                Some(s) => return Ok(ScreenAuth(s)),
                None => record_auth_failure(st, &ip, "ecran").await?,
            }
        }
        Err(ApiError::new(401, "Jeton d'écran invalide ou révoqué"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn bearer_parsing() {
        assert_eq!(bearer(Some("Bearer abc")), Some("abc".into()));
        assert_eq!(bearer(Some("bearer abc")), Some("abc".into()));
        assert_eq!(bearer(Some("BEARER   abc  ")), Some("abc".into()));
        assert_eq!(bearer(Some("Bearer ")), None);
        assert_eq!(bearer(Some("Bearer    ")), None);
        assert_eq!(bearer(Some("Basic abc")), None);
        assert_eq!(bearer(Some("Bearerabc")), None);
        assert_eq!(bearer(Some("")), None);
        assert_eq!(bearer(None), None);
    }

    #[test]
    fn bearer_parsing_never_panics_on_multibyte_prefixes() {
        assert_eq!(bearer(Some("Béarer x")), None);
        assert_eq!(bearer(Some("é☃é☃é☃")), None);
    }

    #[test]
    fn latin1_decoding_of_non_ascii_header_bytes() {
        assert_eq!(latin1(&[b'B', 0xE9]), "Bé");
        let mut h = HeaderMap::new();
        h.insert(AUTHORIZATION, HeaderValue::from_bytes(b"Bearer \xe9").unwrap());
        assert_eq!(bearer_from_headers(&h), Some("é".to_string()));
    }

    #[test]
    fn no_authorization_header_means_no_token() {
        assert_eq!(bearer_from_headers(&HeaderMap::new()), None);
    }
}
