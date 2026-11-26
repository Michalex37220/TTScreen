//! Comptes utilisateurs et journal d'audit (rôle admin uniquement).

use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set, TransactionTrait};
use serde::Deserialize;
use serde_json::json;

use super::hash_blocking;
use crate::app::AppState;
use crate::audit::log_action;
use crate::auth::{count_admins, drop_sessions};
use crate::deps::{Admin, ClientIp, Guard};
use crate::error::{ApiError, FieldError};
use crate::extract::{ApiJson, Id};
use crate::models::{audit_log, user, utcnow};
use crate::schemas::{AuditOut, UserIn, UserOut, UserPatch};

fn user_out(u: &user::Model) -> UserOut {
    UserOut {
        id: u.id,
        username: u.username.clone(),
        role: u.role.clone(),
        active: u.active,
        created_at: u.created_at,
        last_login: u.last_login,
    }
}

/// Le mot de passe n'est jamais stocké en clair (haché PBKDF2, voir `auth.rs`).
pub async fn create_user(
    Guard(principal): Admin,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<UserIn>,
) -> Result<(StatusCode, Json<UserOut>), ApiError> {
    let taken = user::Entity::find().filter(user::Column::Username.eq(body.username.as_str())).one(&st.db).await?;
    if taken.is_some() {
        return Err(ApiError::new(409, "Nom d'utilisateur déjà pris"));
    }
    let password_hash = hash_blocking(&st, body.password.clone()).await?;
    let txn = st.db.begin().await?;
    let u = user::ActiveModel {
        username: Set(body.username.clone()),
        password_hash: Set(password_hash),
        role: Set(body.role.as_str().to_string()),
        active: Set(true),
        created_at: Set(utcnow()),
        ..Default::default()
    }
    .insert(&txn)
    .await?;
    log_action(&txn, &ip, Some(&principal), "user.create", Some(u.username.clone()), Some(format!("rôle={}", u.role)))
        .await?;
    txn.commit().await?;
    Ok((StatusCode::CREATED, Json(user_out(&u))))
}

pub async fn list_users(_g: Admin, State(st): State<AppState>) -> Result<Json<Vec<UserOut>>, ApiError> {
    let rows = user::Entity::find().order_by_asc(user::Column::Id).all(&st.db).await?;
    Ok(Json(rows.iter().map(user_out).collect()))
}

/// Modifie un compte (rôle, état actif, mot de passe).
///
/// On ne peut pas se désactiver soi-même, et il doit toujours rester au moins un administrateur actif.
/// Toute modification révoque les sessions existantes du compte (reconnexion obligatoire).
pub async fn update_user(
    Guard(principal): Admin,
    Id(uid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<UserPatch>,
) -> Result<Json<UserOut>, ApiError> {
    let u = user::Entity::find_by_id(uid)
        .one(&st.db)
        .await?
        .ok_or_else(|| ApiError::new(404, "Utilisateur introuvable"))?;
    if u.username == principal.username && body.active == Some(false) {
        return Err(ApiError::new(409, "Vous ne pouvez pas désactiver votre propre compte"));
    }
    let new_role = body.role.map_or_else(|| u.role.clone(), |r| r.as_str().to_string());
    let new_active = body.active.unwrap_or(u.active);
    if u.role == "admin" && (!new_active || new_role != "admin") && count_admins(&st.db, true, Some(u.id)).await? == 0 {
        return Err(ApiError::new(409, "Il doit rester au moins un administrateur actif"));
    }
    let new_hash = match &body.password {
        Some(p) => Some(hash_blocking(&st, p.clone()).await?),
        None => None,
    };

    let txn = st.db.begin().await?;
    let mut am: user::ActiveModel = u.clone().into();
    if body.role.is_some() {
        am.role = Set(new_role.clone());
    }
    if body.active.is_some() {
        am.active = Set(new_active);
    }
    if let Some(h) = new_hash {
        am.password_hash = Set(h);
    }
    let updated = am.update(&txn).await?;
    drop_sessions(&txn, updated.id).await?;
    let detail = format!(
        "rôle={}, actif={}{}",
        updated.role,
        if updated.active { "True" } else { "False" },
        if body.password.is_some() { ", mot de passe changé" } else { "" }
    );
    log_action(&txn, &ip, Some(&principal), "user.update", Some(updated.username.clone()), Some(detail)).await?;
    txn.commit().await?;
    Ok(Json(user_out(&updated)))
}

/// Supprime un compte (et révoque ses sessions). Interdit de se supprimer soi-même
/// ou de supprimer le dernier administrateur actif.
pub async fn delete_user(
    Guard(principal): Admin,
    Id(uid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<StatusCode, ApiError> {
    let u = user::Entity::find_by_id(uid)
        .one(&st.db)
        .await?
        .ok_or_else(|| ApiError::new(404, "Utilisateur introuvable"))?;
    if u.username == principal.username {
        return Err(ApiError::new(409, "Vous ne pouvez pas supprimer votre propre compte"));
    }
    if u.role == "admin" && count_admins(&st.db, true, Some(u.id)).await? == 0 {
        return Err(ApiError::new(409, "Impossible de supprimer le dernier administrateur actif"));
    }
    let txn = st.db.begin().await?;
    drop_sessions(&txn, u.id).await?;
    log_action(&txn, &ip, Some(&principal), "user.delete", Some(u.username.clone()), None).await?;
    user::Entity::delete_by_id(u.id).exec(&txn).await?;
    txn.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub action: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

fn query_error(field: &str, kind: &'static str, msg: &str) -> ApiError {
    ApiError::validation(vec![FieldError::new(vec![json!("query"), json!(field)], kind, msg)])
}

/// Valide `limit` (1..=1000, défaut 100) et `offset` (≥ 0, défaut 0).
pub fn audit_paging(q: &AuditQuery) -> Result<(u64, u64), ApiError> {
    let limit = q.limit.unwrap_or(100);
    if limit < 1 {
        return Err(query_error("limit", "greater_than_equal", "Input should be greater than or equal to 1"));
    }
    if limit > 1000 {
        return Err(query_error("limit", "less_than_equal", "Input should be less than or equal to 1000"));
    }
    let offset = q.offset.unwrap_or(0);
    if offset < 0 {
        return Err(query_error("offset", "greater_than_equal", "Input should be greater than or equal to 0"));
    }
    Ok((limit as u64, offset as u64))
}

/// Journal des opérations sensibles, du plus récent au plus ancien. Filtre optionnel par `action`.
pub async fn list_audit(
    _g: Admin,
    State(st): State<AppState>,
    query: Result<Query<AuditQuery>, QueryRejection>,
) -> Result<Json<Vec<AuditOut>>, ApiError> {
    let Query(q) = query.map_err(|e| query_error("query", "invalid", &e.body_text()))?;
    let (limit, offset) = audit_paging(&q)?;
    let mut select = audit_log::Entity::find().order_by_desc(audit_log::Column::Id);
    if let Some(action) = q.action.as_deref().filter(|a| !a.is_empty()) {
        select = select.filter(audit_log::Column::Action.eq(action));
    }
    let rows = select.offset(offset).limit(limit).all(&st.db).await?;
    Ok(Json(
        rows.into_iter()
            .map(|a| AuditOut {
                id: a.id,
                created_at: a.created_at,
                username: a.username,
                role: a.role,
                action: a.action,
                target: a.target,
                detail: a.detail,
                ip: a.ip,
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(limit: Option<i64>, offset: Option<i64>) -> AuditQuery {
        AuditQuery { action: None, limit, offset }
    }

    #[test]
    fn audit_paging_defaults_and_bounds() {
        assert_eq!(audit_paging(&q(None, None)).unwrap(), (100, 0));
        assert_eq!(audit_paging(&q(Some(1), Some(0))).unwrap(), (1, 0));
        assert_eq!(audit_paging(&q(Some(1000), Some(50))).unwrap(), (1000, 50));
        for (l, o) in [(Some(0), None), (Some(-1), None), (Some(1001), None), (None, Some(-1))] {
            assert_eq!(audit_paging(&q(l, o)).unwrap_err().status.as_u16(), 422, "{l:?} {o:?}");
        }
    }
}
