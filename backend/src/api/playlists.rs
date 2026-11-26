//! Playlists, publication et versions.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set, TransactionTrait,
};
use serde_json::Value;
use std::collections::HashMap;

use super::DEFAULT_DURATION;
use crate::app::AppState;
use crate::audit::log_action;
use crate::deps::{ClientIp, Editor, Guard, Viewer};
use crate::error::ApiError;
use crate::extract::{ApiJson, Id};
use crate::models::{playlist, playlist_item, playlist_version, schedule_rule, screen, screen_group, utcnow};
use crate::publishing::{self, PublishError};
use crate::schemas::{ItemIn, ItemOut, ItemType, PlaylistIn, PlaylistOut, VersionOut};

impl From<PublishError> for ApiError {
    fn from(e: PublishError) -> Self {
        match e {
            PublishError::Empty => ApiError::new(422, "Playlist vide"),
            PublishError::MissingMedia(id) => ApiError::internal(format!("média {id} introuvable à la publication")),
            PublishError::Db(e) => ApiError::from(e),
        }
    }
}

fn item_out(it: &playlist_item::Model) -> ItemOut {
    ItemOut {
        item_type: it.item_type.clone(),
        text: it.text.clone(),
        media_id: it.media_id,
        duration_s: it.duration_s,
        id: it.id,
        position: it.position,
    }
}

fn playlist_out(p: &playlist::Model, items: &[playlist_item::Model]) -> PlaylistOut {
    PlaylistOut { id: p.id, name: p.name.clone(), items: items.iter().map(item_out).collect() }
}

fn version_out(v: &playlist_version::Model) -> VersionOut {
    VersionOut {
        id: v.id,
        playlist_id: v.playlist_id,
        number: v.number,
        created_at: v.created_at,
        manifest: v.manifest.clone(),
    }
}

async fn find_playlist<C: sea_orm::ConnectionTrait>(db: &C, pid: i32) -> Result<playlist::Model, ApiError> {
    playlist::Entity::find_by_id(pid)
        .one(db)
        .await?
        .ok_or_else(|| ApiError::new(404, "Playlist introuvable"))
}

pub async fn create_playlist(
    Guard(principal): Editor,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<PlaylistIn>,
) -> Result<(StatusCode, Json<PlaylistOut>), ApiError> {
    let txn = st.db.begin().await?;
    let p = playlist::ActiveModel { name: Set(body.name.clone()), created_at: Set(utcnow()), ..Default::default() }
        .insert(&txn)
        .await?;
    log_action(&txn, &ip, Some(&principal), "playlist.create", Some(body.name), None).await?;
    txn.commit().await?;
    Ok((StatusCode::CREATED, Json(playlist_out(&p, &[]))))
}

pub async fn list_playlists(_g: Viewer, State(st): State<AppState>) -> Result<Json<Vec<PlaylistOut>>, ApiError> {
    let playlists = playlist::Entity::find().order_by_asc(playlist::Column::Id).all(&st.db).await?;
    let items = playlist_item::Entity::find()
        .order_by_asc(playlist_item::Column::PlaylistId)
        .order_by_asc(playlist_item::Column::Position)
        .order_by_asc(playlist_item::Column::Id)
        .all(&st.db)
        .await?;
    let mut by_playlist: HashMap<i32, Vec<ItemOut>> = HashMap::new();
    for it in &items {
        by_playlist.entry(it.playlist_id).or_default().push(item_out(it));
    }
    Ok(Json(
        playlists
            .into_iter()
            .map(|p| PlaylistOut { id: p.id, name: p.name, items: by_playlist.remove(&p.id).unwrap_or_default() })
            .collect(),
    ))
}

pub async fn get_playlist(_g: Viewer, Id(pid): Id, State(st): State<AppState>) -> Result<Json<PlaylistOut>, ApiError> {
    let p = find_playlist(&st.db, pid).await?;
    let items = publishing::load_items(&st.db, pid).await?;
    Ok(Json(playlist_out(&p, &items)))
}

pub async fn rename_playlist(
    Guard(principal): Editor,
    Id(pid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<PlaylistIn>,
) -> Result<Json<PlaylistOut>, ApiError> {
    let p = find_playlist(&st.db, pid).await?;
    let txn = st.db.begin().await?;
    let mut am: playlist::ActiveModel = p.into();
    am.name = Set(body.name.clone());
    let p = am.update(&txn).await?;
    log_action(&txn, &ip, Some(&principal), "playlist.rename", Some(format!("playlist:{pid}")), Some(body.name)).await?;
    let items = publishing::load_items(&txn, pid).await?;
    txn.commit().await?;
    Ok(Json(playlist_out(&p, &items)))
}

/// Supprime aussi ses éléments et ses versions ; refusé (409) si une version est affectée à un écran.
pub async fn delete_playlist(
    Guard(principal): Editor,
    Id(pid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<StatusCode, ApiError> {
    let p = find_playlist(&st.db, pid).await?;
    let version_ids: Vec<i32> = playlist_version::Entity::find()
        .select_only()
        .column(playlist_version::Column::Id)
        .filter(playlist_version::Column::PlaylistId.eq(pid))
        .into_tuple()
        .all(&st.db)
        .await?;
    if !version_ids.is_empty() {
        let assigned = screen::Entity::find().filter(screen::Column::VersionId.is_in(version_ids.clone())).one(&st.db).await?;
        if assigned.is_some() {
            return Err(ApiError::new(409, "Une version de cette playlist est affectée à un écran"));
        }
        let in_group =
            screen_group::Entity::find().filter(screen_group::Column::VersionId.is_in(version_ids.clone())).one(&st.db).await?;
        if in_group.is_some() {
            return Err(ApiError::new(409, "Une version de cette playlist est affectée à un groupe d'écrans"));
        }
        let scheduled =
            schedule_rule::Entity::find().filter(schedule_rule::Column::VersionId.is_in(version_ids)).one(&st.db).await?;
        if scheduled.is_some() {
            return Err(ApiError::new(409, "Une version de cette playlist est utilisée dans un planning"));
        }
    }
    let txn = st.db.begin().await?;
    log_action(&txn, &ip, Some(&principal), "playlist.delete", Some(format!("playlist:{pid}")), Some(p.name.clone()))
        .await?;
    playlist_item::Entity::delete_many().filter(playlist_item::Column::PlaylistId.eq(pid)).exec(&txn).await?;
    playlist_version::Entity::delete_many().filter(playlist_version::Column::PlaylistId.eq(pid)).exec(&txn).await?;
    playlist::Entity::delete_by_id(pid).exec(&txn).await?;
    txn.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Copie la playlist (nom « Copie de … », mêmes éléments, aucune version) pour partir d'une base existante.
pub async fn duplicate_playlist(
    Guard(principal): Editor,
    Id(pid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<(StatusCode, Json<PlaylistOut>), ApiError> {
    let source = find_playlist(&st.db, pid).await?;
    let items = publishing::load_items(&st.db, pid).await?;
    let name: String = format!("Copie de {}", source.name).chars().take(120).collect();
    let txn = st.db.begin().await?;
    let copy = playlist::ActiveModel { name: Set(name), created_at: Set(utcnow()), ..Default::default() }
        .insert(&txn)
        .await?;
    let mut saved = Vec::with_capacity(items.len());
    for it in &items {
        saved.push(
            playlist_item::ActiveModel {
                playlist_id: Set(copy.id),
                position: Set(it.position),
                item_type: Set(it.item_type.clone()),
                text: Set(it.text.clone()),
                media_id: Set(it.media_id),
                duration_s: Set(it.duration_s),
                ..Default::default()
            }
            .insert(&txn)
            .await?,
        );
    }
    log_action(
        &txn,
        &ip,
        Some(&principal),
        "playlist.duplicate",
        Some(format!("playlist:{}", copy.id)),
        Some(format!("copie de playlist:{pid}")),
    )
    .await?;
    txn.commit().await?;
    Ok((StatusCode::CREATED, Json(playlist_out(&copy, &saved))))
}

/// Élément validé, prêt à être inséré.
struct NewItem {
    item_type: &'static str,
    text: Option<String>,
    media_id: Option<i32>,
    duration_s: Option<i32>,
}

/// Remplace la liste ordonnée complète des éléments (l'ordre du tableau = l'ordre de lecture).
pub async fn set_items(
    Guard(principal): Editor,
    Id(pid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(items): ApiJson<Vec<ItemIn>>,
) -> Result<Json<PlaylistOut>, ApiError> {
    let p = find_playlist(&st.db, pid).await?;
    let mut new_items: Vec<NewItem> = Vec::with_capacity(items.len());
    for (pos, it) in items.iter().enumerate() {
        if it.item_type == ItemType::Text {
            if !it.text.as_deref().map_or(false, |t| !t.trim().is_empty()) {
                return Err(ApiError::new(422, format!("Élément {pos}: texte requis")));
            }
            new_items.push(NewItem {
                item_type: "text",
                text: it.text.clone(),
                media_id: None,
                duration_s: it.duration_s.map(|d| d as i32).or(Some(DEFAULT_DURATION)),
            });
        } else {
            let found = match it.media_id {
                Some(mid) => crate::models::media::Entity::find_by_id(mid as i32).one(&st.db).await?,
                None => None,
            };
            let m = found.ok_or_else(|| ApiError::new(422, format!("Élément {pos}: média introuvable")))?;
            if m.kind != it.item_type.as_str() {
                return Err(ApiError::new(422, format!("Élément {pos}: le média est de type {}", m.kind)));
            }
            let duration = if it.item_type == ItemType::Video {
                it.duration_s.map(|d| d as i32)
            } else {
                it.duration_s.map(|d| d as i32).or(Some(DEFAULT_DURATION))
            };
            new_items.push(NewItem { item_type: it.item_type.as_str(), text: None, media_id: Some(m.id), duration_s: duration });
        }
    }

    let txn = st.db.begin().await?;
    playlist_item::Entity::delete_many().filter(playlist_item::Column::PlaylistId.eq(pid)).exec(&txn).await?;
    let mut saved = Vec::with_capacity(new_items.len());
    for (pos, n) in new_items.into_iter().enumerate() {
        let row = playlist_item::ActiveModel {
            playlist_id: Set(pid),
            position: Set(pos as i32),
            item_type: Set(n.item_type.to_string()),
            text: Set(n.text),
            media_id: Set(n.media_id),
            duration_s: Set(n.duration_s),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        saved.push(row);
    }
    log_action(
        &txn,
        &ip,
        Some(&principal),
        "playlist.items",
        Some(format!("playlist:{pid}")),
        Some(format!("{} élément(s)", saved.len())),
    )
    .await?;
    txn.commit().await?;
    Ok(Json(playlist_out(&p, &saved)))
}

/// Crée une version immuable. La publication et son entrée d'audit sont dans la MÊME transaction :
/// si l'audit échoue, rien n'est publié.
pub async fn publish_playlist(
    Guard(principal): Editor,
    Id(pid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<(StatusCode, Json<VersionOut>), ApiError> {
    let p = find_playlist(&st.db, pid).await?;
    let txn = st.db.begin().await?;
    let version = publishing::publish(&txn, &p).await?;
    log_action(
        &txn,
        &ip,
        Some(&principal),
        "playlist.publish",
        Some(format!("playlist:{pid}")),
        Some(format!("v{}", version.number)),
    )
    .await?;
    txn.commit().await?;
    Ok((StatusCode::CREATED, Json(version_out(&version))))
}

/// Brouillon courant au format lecteur, sans rien publier.
pub async fn preview_draft(_g: Viewer, Id(pid): Id, State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let p = find_playlist(&st.db, pid).await?;
    Ok(Json(publishing::preview(&st.db, &p).await?))
}

pub async fn list_versions(_g: Viewer, Id(pid): Id, State(st): State<AppState>) -> Result<Json<Vec<VersionOut>>, ApiError> {
    find_playlist(&st.db, pid).await?;
    let rows = playlist_version::Entity::find()
        .filter(playlist_version::Column::PlaylistId.eq(pid))
        .order_by_asc(playlist_version::Column::Number)
        .all(&st.db)
        .await?;
    Ok(Json(rows.iter().map(version_out).collect()))
}

/// Contenu exact d'une version (prévisualisation / test avant déploiement).
pub async fn get_version(_g: Viewer, Id(vid): Id, State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let v = playlist_version::Entity::find_by_id(vid)
        .one(&st.db)
        .await?
        .ok_or_else(|| ApiError::new(404, "Version introuvable"))?;
    Ok(Json(publishing::player_payload(&v)))
}
