//! Médias : téléversement, liste, suppression.

use axum::extract::multipart::{MultipartError, MultipartRejection};
use axum::extract::{Multipart, State};
use axum::http::StatusCode;
use axum::Json;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set, TransactionTrait};
use serde_json::json;

use crate::app::AppState;
use crate::audit::{log_action, truncate};
use crate::deps::{ClientIp, Editor, Guard, Viewer};
use crate::error::{ApiError, FieldError};
use crate::extract::Id;
use crate::models::{media, playlist_item, playlist_version, utcnow};
use crate::publishing::media_url;
use crate::schemas::MediaOut;
use crate::storage::UploadSaver;

pub fn media_out(m: &media::Model) -> MediaOut {
    MediaOut {
        id: m.id,
        original_name: m.original_name.clone(),
        kind: m.kind.clone(),
        mime: m.mime.clone(),
        size: m.size as i64,
        sha256: m.sha256.clone(),
        url: media_url(m),
    }
}

fn file_field_error(kind: &'static str, msg: impl Into<String>) -> ApiError {
    ApiError::validation(vec![FieldError::new(vec![json!("body"), json!("file")], kind, msg)])
}

fn multipart_error(e: MultipartError) -> ApiError {
    if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::new(413, "Fichier trop volumineux")
    } else {
        file_field_error("multipart_invalid", e.body_text())
    }
}

/// Envoi d'un média. L'authentification (rôle editor) et la taille déclarée ont déjà été vérifiées
/// AVANT toute lecture du corps par `security::upload_guard`.
pub async fn upload_media(
    Guard(principal): Editor,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<Json<MediaOut>, ApiError> {
    let mut multipart = multipart.map_err(|e| file_field_error("missing", e.body_text()))?;
    let mut saved = None;
    while let Some(mut field) = multipart.next_field().await.map_err(multipart_error)? {
        if field.name() == Some("file") {
            let filename = field.file_name().map(str::to_string);
            let mut saver = UploadSaver::begin(&st.settings.media_dir, st.settings.max_upload_bytes()).await?;
            while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
                saver.push(&chunk).await?;
            }
            saved = Some((saver.finish().await?, filename));
            break;
        }
    }
    let Some((info, filename)) = saved else {
        return Err(file_field_error("missing", "Field required"));
    };

    // Doublon (même SHA-256) : le média existant est renvoyé, rien n'est écrit ni journalisé
    let existing = media::Entity::find().filter(media::Column::Sha256.eq(info.sha256.as_str())).one(&st.db).await?;
    if let Some(m) = existing {
        return Ok(Json(media_out(&m)));
    }

    let name = truncate(filename.as_deref().filter(|f| !f.is_empty()).unwrap_or("fichier"), 255);
    let txn = st.db.begin().await?;
    let inserted = media::ActiveModel {
        original_name: Set(name.clone()),
        stored_name: Set(info.stored_name.clone()),
        sha256: Set(info.sha256.clone()),
        kind: Set(info.kind.to_string()),
        mime: Set(info.mime.to_string()),
        size: Set(info.size as i32),
        created_at: Set(utcnow()),
        ..Default::default()
    }
    .insert(&txn)
    .await;
    let model = match inserted {
        Ok(m) => m,
        Err(e) => {
            // Deux envois identiques simultanés : l'autre a gagné, on renvoie son média
            drop(txn);
            let again = media::Entity::find().filter(media::Column::Sha256.eq(info.sha256.as_str())).one(&st.db).await?;
            return match again {
                Some(m) => Ok(Json(media_out(&m))),
                None => Err(e.into()),
            };
        }
    };
    log_action(
        &txn,
        &ip,
        Some(&principal),
        "media.upload",
        Some(name),
        Some(format!("{}, {} o, sha256={}…", info.kind, info.size, &info.sha256[..12])),
    )
    .await?;
    txn.commit().await?;
    Ok(Json(media_out(&model)))
}

/// Médias du plus récent au plus ancien.
pub async fn list_media(_g: Viewer, State(st): State<AppState>) -> Result<Json<Vec<MediaOut>>, ApiError> {
    let rows = media::Entity::find().order_by_desc(media::Column::Id).all(&st.db).await?;
    Ok(Json(rows.iter().map(media_out).collect()))
}

pub async fn delete_media(
    Guard(principal): Editor,
    Id(media_id): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<StatusCode, ApiError> {
    let m = media::Entity::find_by_id(media_id)
        .one(&st.db)
        .await?
        .ok_or_else(|| ApiError::new(404, "Média introuvable"))?;
    let used_in_playlist = playlist_item::Entity::find()
        .filter(playlist_item::Column::MediaId.eq(media_id))
        .one(&st.db)
        .await?;
    if used_in_playlist.is_some() {
        return Err(ApiError::new(409, "Média utilisé dans une playlist"));
    }
    let version_media: Vec<serde_json::Value> = playlist_version::Entity::find()
        .select_only()
        .column(playlist_version::Column::MediaIds)
        .into_tuple()
        .all(&st.db)
        .await?;
    let in_version = version_media
        .iter()
        .any(|ids| ids.as_array().map_or(false, |a| a.iter().any(|x| x.as_i64() == Some(media_id as i64))));
    if in_version {
        return Err(ApiError::new(409, "Média utilisé dans une version publiée"));
    }
    let path = st.settings.media_dir.join(&m.stored_name);
    let txn = st.db.begin().await?;
    log_action(&txn, &ip, Some(&principal), "media.delete", Some(m.original_name.clone()), None).await?;
    media::Entity::delete_by_id(media_id).exec(&txn).await?;
    txn.commit().await?;
    let _ = tokio::fs::remove_file(path).await;
    Ok(StatusCode::NO_CONTENT)
}
