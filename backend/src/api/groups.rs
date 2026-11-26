//! Groupes d'écrans : un contenu commun à plusieurs écrans.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait,
};

use crate::app::AppState;
use crate::audit::log_action;
use crate::deps::{ClientIp, Editor, Guard, Viewer};
use crate::error::ApiError;
use crate::extract::{ApiJson, Id};
use crate::models::{playlist_version, schedule_rule, screen, screen_group, utcnow};
use crate::schemas::{AssignIn, GroupIn, GroupOut};

fn group_out(g: &screen_group::Model, screen_count: i64) -> GroupOut {
    GroupOut { id: g.id, name: g.name.clone(), version_id: g.version_id, screen_count }
}

async fn find_group<C: ConnectionTrait>(db: &C, gid: i32) -> Result<screen_group::Model, ApiError> {
    screen_group::Entity::find_by_id(gid)
        .one(db)
        .await?
        .ok_or_else(|| ApiError::new(404, "Groupe introuvable"))
}

async fn count_screens<C: ConnectionTrait>(db: &C, gid: i32) -> Result<i64, ApiError> {
    Ok(screen::Entity::find().filter(screen::Column::GroupId.eq(gid)).all(db).await?.len() as i64)
}

/// Deux groupes ne portent pas le même nom (hors le groupe `except`).
async fn ensure_name_free<C: ConnectionTrait>(db: &C, name: &str, except: Option<i32>) -> Result<(), ApiError> {
    let same = screen_group::Entity::find().filter(screen_group::Column::Name.eq(name)).one(db).await?;
    match same {
        Some(g) if Some(g.id) != except => Err(ApiError::new(409, "Un groupe porte déjà ce nom")),
        _ => Ok(()),
    }
}

pub async fn list_groups(_g: Viewer, State(st): State<AppState>) -> Result<Json<Vec<GroupOut>>, ApiError> {
    let groups = screen_group::Entity::find().order_by_asc(screen_group::Column::Name).all(&st.db).await?;
    let screens = screen::Entity::find().all(&st.db).await?;
    Ok(Json(
        groups
            .iter()
            .map(|g| group_out(g, screens.iter().filter(|s| s.group_id == Some(g.id)).count() as i64))
            .collect(),
    ))
}

pub async fn create_group(
    Guard(principal): Editor,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<GroupIn>,
) -> Result<(StatusCode, Json<GroupOut>), ApiError> {
    ensure_name_free(&st.db, &body.name, None).await?;
    let txn = st.db.begin().await?;
    let g = screen_group::ActiveModel { name: Set(body.name.clone()), created_at: Set(utcnow()), ..Default::default() }
        .insert(&txn)
        .await?;
    log_action(&txn, &ip, Some(&principal), "group.create", Some(body.name), None).await?;
    txn.commit().await?;
    Ok((StatusCode::CREATED, Json(group_out(&g, 0))))
}

pub async fn rename_group(
    Guard(principal): Editor,
    Id(gid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<GroupIn>,
) -> Result<Json<GroupOut>, ApiError> {
    let g = find_group(&st.db, gid).await?;
    ensure_name_free(&st.db, &body.name, Some(gid)).await?;
    let txn = st.db.begin().await?;
    let mut am: screen_group::ActiveModel = g.into();
    am.name = Set(body.name.clone());
    let g = am.update(&txn).await?;
    log_action(&txn, &ip, Some(&principal), "group.rename", Some(format!("group:{gid}")), Some(body.name)).await?;
    txn.commit().await?;
    let n = count_screens(&st.db, gid).await?;
    Ok(Json(group_out(&g, n)))
}

/// Affecte une version à tout le groupe (`null` = retirer). Les écrans ayant leur propre contenu fixé ou un
/// planning actif gardent leur contenu (voir `scheduling.rs`).
pub async fn assign_group(
    Guard(principal): Editor,
    Id(gid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<AssignIn>,
) -> Result<Json<GroupOut>, ApiError> {
    let g = find_group(&st.db, gid).await?;
    if let Some(vid) = body.version_id {
        if playlist_version::Entity::find_by_id(vid as i32).one(&st.db).await?.is_none() {
            return Err(ApiError::new(404, "Version introuvable"));
        }
    }
    let txn = st.db.begin().await?;
    let mut am: screen_group::ActiveModel = g.into();
    am.version_id = Set(body.version_id.map(|v| v as i32));
    let g = am.update(&txn).await?;
    let detail = match body.version_id {
        Some(v) => format!("version_id={v}"),
        None => "version_id=None".to_string(),
    };
    log_action(&txn, &ip, Some(&principal), "group.assign", Some(format!("group:{gid}")), Some(detail)).await?;
    txn.commit().await?;
    let n = count_screens(&st.db, gid).await?;
    Ok(Json(group_out(&g, n)))
}

/// Supprime le groupe : ses écrans sont conservés mais sortent du groupe (ils perdent le contenu hérité) ;
/// les règles de planning du groupe sont supprimées.
pub async fn delete_group(
    Guard(principal): Editor,
    Id(gid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<StatusCode, ApiError> {
    let g = find_group(&st.db, gid).await?;
    let members = screen::Entity::find().filter(screen::Column::GroupId.eq(gid)).all(&st.db).await?;
    let txn = st.db.begin().await?;
    let detail = format!("{} écran(s) retiré(s) du groupe", members.len());
    log_action(&txn, &ip, Some(&principal), "group.delete", Some(g.name.clone()), Some(detail)).await?;
    for s in members {
        let mut am: screen::ActiveModel = s.into();
        am.group_id = Set(None);
        am.update(&txn).await?;
    }
    schedule_rule::Entity::delete_many().filter(schedule_rule::Column::GroupId.eq(gid)).exec(&txn).await?;
    screen_group::Entity::delete_by_id(gid).exec(&txn).await?;
    txn.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
