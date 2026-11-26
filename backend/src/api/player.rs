//! Lecteur : manifeste de la version affectée à l'écran authentifié par son jeton.

use axum::extract::State;
use axum::Json;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde_json::Value;

use crate::app::AppState;
use crate::deps::ScreenAuth;
use crate::error::ApiError;
use crate::models::{playlist_version, screen, utcnow};
use crate::publishing::player_payload;
use crate::scheduling::load_ctx;

/// Renvoie la version à afficher MAINTENANT (format manifeste) et met à jour `last_seen`.
/// Elle résulte de l'écran, de son groupe et des plannings (voir `scheduling.rs`).
/// 401 si le jeton est invalide ou révoqué, 404 si rien n'est à afficher.
pub async fn player_manifest(ScreenAuth(s): ScreenAuth, State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let ctx = load_ctx(&st.db, st.tz).await?;
    let version_id = ctx.resolve(&s).version_id;
    let mut am: screen::ActiveModel = s.into();
    am.last_seen = Set(Some(utcnow()));
    am.update(&st.db).await?;
    let Some(vid) = version_id else {
        return Err(ApiError::new(404, "Aucune version affectée"));
    };
    let v = playlist_version::Entity::find_by_id(vid)
        .one(&st.db)
        .await?
        .ok_or_else(|| ApiError::new(404, "Aucune version affectée"))?;
    Ok(Json(player_payload(&v)))
}
