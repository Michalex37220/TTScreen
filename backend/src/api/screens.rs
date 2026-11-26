//! Écrans : création, groupe, contenu, jetons.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use chrono::Duration;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait};
use serde_json::Value;

use crate::app::AppState;
use crate::audit::log_action;
use crate::auth::{hash_token, new_token};
use crate::deps::{ClientIp, Editor, Guard, Viewer};
use crate::error::ApiError;
use crate::extract::{ApiJson, Id};
use crate::models::{playlist_version, schedule_rule, screen, screen_group, utcnow};
use crate::publishing::player_payload;
use crate::scheduling::{load_ctx, ResolveCtx};
use crate::schemas::{AssignIn, ScreenCreated, ScreenIn, ScreenOut, ScreenRename, SetGroupIn};

/// Un écran est « en ligne » s'il a interrogé le serveur dans les 60 dernières secondes.
pub fn online_window() -> Duration {
    Duration::seconds(60)
}

/// Vue d'un écran : état, groupe et contenu effectivement diffusé (groupe + planning pris en compte).
pub fn screen_out(s: &screen::Model, ctx: &ResolveCtx) -> ScreenOut {
    let online = s.last_seen.map_or(false, |seen| utcnow() - seen < online_window());
    let group = ctx.group_of(s);
    let resolution = ctx.resolve(s);
    ScreenOut {
        id: s.id,
        name: s.name.clone(),
        version_id: s.version_id,
        online,
        last_seen: s.last_seen,
        has_token: s.token_hash.is_some(),
        group_id: group.map(|g| g.id),
        group_name: group.map(|g| g.name.clone()),
        effective_version_id: resolution.version_id,
        effective_source: resolution.source.map(|src| src.as_str().to_string()),
    }
}

async fn find_screen<C: sea_orm::ConnectionTrait>(db: &C, sid: i32) -> Result<screen::Model, ApiError> {
    screen::Entity::find_by_id(sid).one(db).await?.ok_or_else(|| ApiError::new(404, "Écran introuvable"))
}

async fn require_group<C: sea_orm::ConnectionTrait>(db: &C, gid: i64) -> Result<screen_group::Model, ApiError> {
    screen_group::Entity::find_by_id(gid as i32)
        .one(db)
        .await?
        .ok_or_else(|| ApiError::new(404, "Groupe introuvable"))
}

/// Relit l'écran après modification pour renvoyer son état à jour (groupe/contenu effectif compris).
async fn fresh_out(st: &AppState, sid: i32) -> Result<ScreenOut, ApiError> {
    let ctx = load_ctx(&st.db, st.tz).await?;
    let s = find_screen(&st.db, sid).await?;
    Ok(screen_out(&s, &ctx))
}

/// Crée un écran (éventuellement directement dans un groupe) ; son jeton n'est affiché qu'une seule fois.
pub async fn create_screen(
    Guard(principal): Editor,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<ScreenIn>,
) -> Result<(StatusCode, Json<ScreenCreated>), ApiError> {
    if let Some(gid) = body.group_id {
        require_group(&st.db, gid).await?;
    }
    let token = new_token();
    let txn = st.db.begin().await?;
    let s = screen::ActiveModel {
        name: Set(body.name.clone()),
        token_hash: Set(Some(hash_token(&token))),
        created_at: Set(utcnow()),
        group_id: Set(body.group_id.map(|g| g as i32)),
        ..Default::default()
    }
    .insert(&txn)
    .await?;
    log_action(&txn, &ip, Some(&principal), "screen.create", Some(body.name), None).await?;
    txn.commit().await?;
    Ok((StatusCode::CREATED, Json(ScreenCreated { screen: fresh_out(&st, s.id).await?, token })))
}

pub async fn list_screens(_g: Viewer, State(st): State<AppState>) -> Result<Json<Vec<ScreenOut>>, ApiError> {
    let ctx = load_ctx(&st.db, st.tz).await?;
    let rows = screen::Entity::find().order_by_asc(screen::Column::Id).all(&st.db).await?;
    Ok(Json(rows.iter().map(|s| screen_out(s, &ctx)).collect()))
}

pub async fn rename_screen(
    Guard(principal): Editor,
    Id(sid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<ScreenRename>,
) -> Result<Json<ScreenOut>, ApiError> {
    let s = find_screen(&st.db, sid).await?;
    let txn = st.db.begin().await?;
    let mut am: screen::ActiveModel = s.into();
    am.name = Set(body.name.clone());
    am.update(&txn).await?;
    log_action(&txn, &ip, Some(&principal), "screen.rename", Some(format!("screen:{sid}")), Some(body.name)).await?;
    txn.commit().await?;
    Ok(Json(fresh_out(&st, sid).await?))
}

/// Supprime l'écran et ses règles de planning (son jeton cesse immédiatement de fonctionner).
pub async fn delete_screen(
    Guard(principal): Editor,
    Id(sid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<StatusCode, ApiError> {
    let s = find_screen(&st.db, sid).await?;
    let txn = st.db.begin().await?;
    log_action(&txn, &ip, Some(&principal), "screen.delete", Some(s.name.clone()), Some(format!("screen:{sid}"))).await?;
    schedule_rule::Entity::delete_many().filter(schedule_rule::Column::ScreenId.eq(sid)).exec(&txn).await?;
    screen::Entity::delete_by_id(sid).exec(&txn).await?;
    txn.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Place l'écran dans un groupe, ou l'en sort (`group_id: null`).
pub async fn set_screen_group(
    Guard(principal): Editor,
    Id(sid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<SetGroupIn>,
) -> Result<Json<ScreenOut>, ApiError> {
    let s = find_screen(&st.db, sid).await?;
    if let Some(gid) = body.group_id {
        require_group(&st.db, gid).await?;
    }
    let txn = st.db.begin().await?;
    let mut am: screen::ActiveModel = s.into();
    am.group_id = Set(body.group_id.map(|g| g as i32));
    am.update(&txn).await?;
    let detail = match body.group_id {
        Some(g) => format!("group_id={g}"),
        None => "group_id=None".to_string(),
    };
    log_action(&txn, &ip, Some(&principal), "screen.group", Some(format!("screen:{sid}")), Some(detail)).await?;
    txn.commit().await?;
    Ok(Json(fresh_out(&st, sid).await?))
}

/// Fixe une version sur cet écran (prioritaire sur le groupe). Revenir en arrière = affecter une version antérieure ;
/// `null` retire le contenu propre de l'écran (il reprend alors celui de son groupe, s'il en a un).
pub async fn assign_version(
    Guard(principal): Editor,
    Id(sid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<AssignIn>,
) -> Result<Json<ScreenOut>, ApiError> {
    let s = find_screen(&st.db, sid).await?;
    if let Some(vid) = body.version_id {
        if playlist_version::Entity::find_by_id(vid as i32).one(&st.db).await?.is_none() {
            return Err(ApiError::new(404, "Version introuvable"));
        }
    }
    let txn = st.db.begin().await?;
    let mut am: screen::ActiveModel = s.into();
    am.version_id = Set(body.version_id.map(|v| v as i32));
    am.update(&txn).await?;
    let detail = match body.version_id {
        Some(v) => format!("version_id={v}"),
        None => "version_id=None".to_string(),
    };
    log_action(&txn, &ip, Some(&principal), "screen.assign", Some(format!("screen:{sid}")), Some(detail)).await?;
    txn.commit().await?;
    Ok(Json(fresh_out(&st, sid).await?))
}

/// Ce que voit l'écran à cet instant (groupe et planning compris), sans le marquer comme connecté.
pub async fn screen_preview(_g: Viewer, Id(sid): Id, State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let s = find_screen(&st.db, sid).await?;
    let ctx = load_ctx(&st.db, st.tz).await?;
    let Some(vid) = ctx.resolve(&s).version_id else {
        return Err(ApiError::new(404, "Aucune version affectée"));
    };
    let v = playlist_version::Entity::find_by_id(vid)
        .one(&st.db)
        .await?
        .ok_or_else(|| ApiError::new(404, "Aucune version affectée"))?;
    Ok(Json(player_payload(&v)))
}

/// Nouveau jeton : l'ancien cesse de fonctionner immédiatement.
pub async fn rotate_token(
    Guard(principal): Editor,
    Id(sid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<Json<ScreenCreated>, ApiError> {
    let s = find_screen(&st.db, sid).await?;
    let token = new_token();
    let txn = st.db.begin().await?;
    let mut am: screen::ActiveModel = s.into();
    am.token_hash = Set(Some(hash_token(&token)));
    am.update(&txn).await?;
    log_action(&txn, &ip, Some(&principal), "screen.rotate_token", Some(format!("screen:{sid}")), None).await?;
    txn.commit().await?;
    Ok(Json(ScreenCreated { screen: fresh_out(&st, sid).await?, token }))
}

/// L'écran ne peut plus se connecter ; pour le réactiver, générer un nouveau jeton.
pub async fn revoke_token(
    Guard(principal): Editor,
    Id(sid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<Json<ScreenOut>, ApiError> {
    let s = find_screen(&st.db, sid).await?;
    let txn = st.db.begin().await?;
    let mut am: screen::ActiveModel = s.into();
    am.token_hash = Set(None);
    am.update(&txn).await?;
    log_action(&txn, &ip, Some(&principal), "screen.revoke_token", Some(format!("screen:{sid}")), None).await?;
    txn.commit().await?;
    Ok(Json(fresh_out(&st, sid).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::schedule_rule;
    use crate::scheduling::{days_to_mask, END_OF_DAY};
    use std::collections::HashMap;

    fn model(last_seen: Option<chrono::NaiveDateTime>, token: Option<&str>) -> screen::Model {
        screen::Model {
            id: 1,
            name: "Entrée".into(),
            token_hash: token.map(Into::into),
            version_id: Some(3),
            last_seen,
            created_at: utcnow(),
            group_id: None,
        }
    }

    fn empty_ctx() -> ResolveCtx {
        ResolveCtx { groups: HashMap::new(), rules: vec![], now_local: utcnow() }
    }

    #[test]
    fn never_seen_screen_is_offline() {
        assert!(!screen_out(&model(None, Some("h")), &empty_ctx()).online);
    }

    #[test]
    fn online_window_is_sixty_seconds() {
        let ctx = empty_ctx();
        assert!(screen_out(&model(Some(utcnow() - Duration::seconds(10)), Some("h")), &ctx).online);
        assert!(screen_out(&model(Some(utcnow() - Duration::seconds(59)), Some("h")), &ctx).online);
        assert!(!screen_out(&model(Some(utcnow() - Duration::seconds(61)), Some("h")), &ctx).online);
        assert!(!screen_out(&model(Some(utcnow() - Duration::hours(5)), Some("h")), &ctx).online);
    }

    #[test]
    fn has_token_reflects_revocation_and_never_leaks_the_hash() {
        let ctx = empty_ctx();
        assert!(screen_out(&model(None, Some("h")), &ctx).has_token);
        assert!(!screen_out(&model(None, None), &ctx).has_token);
        let v = serde_json::to_value(screen_out(&model(None, Some("secret-hash")), &ctx)).unwrap();
        assert!(!v.to_string().contains("secret-hash"));
    }

    #[test]
    fn fixed_content_is_reported_as_effective() {
        let out = screen_out(&model(None, Some("h")), &empty_ctx());
        assert_eq!((out.effective_version_id, out.effective_source.as_deref()), (Some(3), Some("screen")));
        assert_eq!((out.group_id, out.group_name), (None, None));
    }

    #[test]
    fn group_name_and_inherited_content_are_reported() {
        let mut ctx = empty_ctx();
        ctx.groups.insert(
            7,
            screen_group::Model { id: 7, name: "Hall".into(), version_id: Some(42), created_at: utcnow() },
        );
        let mut s = model(None, Some("h"));
        s.version_id = None;
        s.group_id = Some(7);
        let out = screen_out(&s, &ctx);
        assert_eq!((out.group_id, out.group_name.as_deref()), (Some(7), Some("Hall")));
        assert_eq!((out.effective_version_id, out.effective_source.as_deref()), (Some(42), Some("group")));
    }

    #[test]
    fn an_active_schedule_changes_the_effective_content() {
        let mut ctx = empty_ctx();
        ctx.rules.push(schedule_rule::Model {
            id: 1,
            screen_id: Some(1),
            group_id: None,
            version_id: 99,
            days: days_to_mask(&[0, 1, 2, 3, 4, 5, 6]),
            start_min: 0,
            end_min: END_OF_DAY,
            created_at: utcnow(),
        });
        let out = screen_out(&model(None, Some("h")), &ctx);
        assert_eq!((out.effective_version_id, out.effective_source.as_deref()), (Some(99), Some("screen_schedule")));
        assert_eq!(out.version_id, Some(3), "le contenu fixé reste visible séparément");
    }
}
