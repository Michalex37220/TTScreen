//! Connexion, déconnexion et identité courante (`/api/auth/...`).

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use chrono::Duration;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait};

use super::{hash_blocking, verify_blocking};
use crate::app::AppState;
use crate::audit::log_action;
use crate::auth::{create_session, hash_token, legacy_allowed, safe_equal, Principal, LEGACY_USERNAME};
use crate::deps::{reject_if_blocked, Bearer, ClientIp, Guard, Viewer};
use crate::auth::drop_sessions_except;
use crate::error::ApiError;
use crate::extract::ApiJson;
use crate::models::{user, user_session, utcnow};
use crate::schemas::{LoginIn, MeOut, PasswordChangeIn, SessionOut};

/// Connexion. Deux modes :
/// - `username` + `password` : compte nommé → jeton de session (haché en base, révocable, durée limitée) ;
/// - `token` seul : jeton admin historique → renvoyé tel quel (pas de session en base).
///
/// Les connexions réussies et échouées sont journalisées.
pub async fn login(
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<LoginIn>,
) -> Result<Json<SessionOut>, ApiError> {
    let s = &st.settings;
    let no_username = body.username.as_deref().map_or(true, str::is_empty);

    if let (Some(token), true) = (body.token.as_deref(), no_username) {
        // Comparaison sur octets (pas d'erreur sur un jeton non ASCII) ; refusé si désactivé ou si un
        // compte admin nommé existe déjà (voir `auth::legacy_allowed`)
        let accepted = !s.admin_token.is_empty()
            && safe_equal(token, &s.admin_token)
            && legacy_allowed(&st.db, s.legacy_token_always).await?;
        if accepted {
            let legacy = Principal::legacy_admin();
            log_action(&st.db, &ip, Some(&legacy), "connexion", Some("jeton admin".into()), None).await?;
            return Ok(Json(SessionOut {
                token: token.to_string(),
                username: LEGACY_USERNAME.to_string(),
                role: "admin".to_string(),
                expires_at: None,
            }));
        }
        log_action(&st.db, &ip, None, "connexion_echouee", Some("jeton admin".into()), None).await?;
        return Err(ApiError::new(401, "Identifiants invalides"));
    }

    // Compte nommé : même message d'erreur ET même durée dans tous les cas pour ne pas révéler quel
    // identifiant existe (PBKDF2 est toujours exécuté, contre un haché factice si le compte n'existe pas)
    let uname = body.username.clone().unwrap_or_default();
    let user_key = format!("loginuser:{}", uname.to_lowercase());
    reject_if_blocked(&st, &user_key, s.rate_limit_login_user)?; // anti force brute par identifiant
    let account = user::Entity::find().filter(user::Column::Username.eq(uname.as_str())).one(&st.db).await?;
    let stored = match &account {
        Some(u) => u.password_hash.clone(),
        None => (*st.dummy_hash).clone(),
    };
    let password_ok = verify_blocking(&st, body.password.clone().unwrap_or_default(), stored).await;
    let account = match account {
        Some(u) if u.active && password_ok => u,
        _ => {
            st.limiter.check(&user_key, s.rate_limit_login_user);
            log_action(&st.db, &ip, None, "connexion_echouee", body.username.clone(), None).await?;
            return Err(ApiError::new(401, "Identifiants invalides"));
        }
    };

    let txn = st.db.begin().await?;
    let mut am: user::ActiveModel = account.clone().into();
    am.last_login = Set(Some(utcnow()));
    am.update(&txn).await?;
    let (token, expires_at) = create_session(&txn, account.id, Duration::hours(s.session_ttl_h)).await?;
    let principal = Principal { username: account.username.clone(), role: account.role.clone(), legacy: false };
    log_action(&txn, &ip, Some(&principal), "connexion", Some(account.username.clone()), None).await?;
    txn.commit().await?;
    Ok(Json(SessionOut { token, username: account.username, role: account.role, expires_at: Some(expires_at) }))
}

/// Révoque la session courante : le jeton cesse immédiatement de fonctionner.
pub async fn logout(
    Guard(principal): Viewer,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    Bearer(token): Bearer,
) -> Result<StatusCode, ApiError> {
    let txn = st.db.begin().await?;
    if !principal.legacy {
        // le jeton admin historique n'a pas de session en base
        if let Some(t) = token {
            user_session::Entity::delete_many()
                .filter(user_session::Column::TokenHash.eq(hash_token(&t)))
                .exec(&txn)
                .await?;
        }
    }
    log_action(&txn, &ip, Some(&principal), "deconnexion", Some(principal.username.clone()), None).await?;
    txn.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Change son PROPRE mot de passe (rôle quelconque). Exige le mot de passe actuel ; les autres sessions du compte
/// sont révoquées (la session courante reste valide). Les tentatives ratées comptent dans le même quota que la
/// connexion, pour qu'un jeton de session volé ne serve pas à deviner le mot de passe.
pub async fn change_password(
    Guard(principal): Viewer,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    Bearer(token): Bearer,
    ApiJson(body): ApiJson<PasswordChangeIn>,
) -> Result<StatusCode, ApiError> {
    if principal.legacy {
        return Err(ApiError::new(400, "Le jeton admin historique n'a pas de mot de passe : créez un compte nommé"));
    }
    let user_key = format!("loginuser:{}", principal.username.to_lowercase());
    reject_if_blocked(&st, &user_key, st.settings.rate_limit_login_user)?;
    let account = user::Entity::find()
        .filter(user::Column::Username.eq(principal.username.as_str()))
        .one(&st.db)
        .await?
        .ok_or_else(|| ApiError::unauthenticated())?;
    if !verify_blocking(&st, body.current_password.clone(), account.password_hash.clone()).await {
        st.limiter.check(&user_key, st.settings.rate_limit_login_user);
        log_action(&st.db, &ip, Some(&principal), "mot_de_passe_refuse", Some(account.username.clone()), None).await?;
        return Err(ApiError::new(403, "Mot de passe actuel incorrect"));
    }
    let new_hash = hash_blocking(&st, body.new_password.clone()).await?;
    let keep = token.as_deref().map(hash_token);
    let txn = st.db.begin().await?;
    let id = account.id;
    let username = account.username.clone();
    let mut am: user::ActiveModel = account.into();
    am.password_hash = Set(new_hash);
    am.update(&txn).await?;
    drop_sessions_except(&txn, id, keep.as_deref()).await?;
    log_action(&txn, &ip, Some(&principal), "user.password", Some(username), Some("changé par l'utilisateur".into())).await?;
    txn.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Identité derrière le jeton courant (l'interface s'en sert pour afficher le rôle et l'onglet Comptes).
pub async fn me(Guard(principal): Viewer) -> Json<MeOut> {
    Json(MeOut { username: principal.username, role: principal.role, legacy: principal.legacy })
}
