//! Toutes les routes de l'API (équivalent de `api.py`), découpées par domaine.
//!
//! Contrôle d'accès, comme à l'origine :
//! - **viewer** (lecture seule) : lister médias/playlists/versions/écrans, prévisualiser ;
//! - **editor** : en plus, créer/modifier le contenu, publier, gérer les écrans ;
//! - **admin** : en plus, gérer les comptes et consulter le journal d'audit.
//!
//! Le jeton admin historique (`TTSCREEN_ADMIN_TOKEN`) a le rôle admin.

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, patch, post, put};
use axum::Router;
use std::sync::atomic::Ordering;

use crate::app::AppState;
use crate::auth;
use crate::error::ApiError;
use crate::security::MULTIPART_OVERHEAD;

pub mod account;
pub mod dashboard;
pub mod groups;
pub mod media;
pub mod player;
pub mod playlists;
pub mod public;
pub mod schedules;
pub mod screens;
pub mod users;

/// Durée d'affichage par défaut d'un texte ou d'une image (secondes).
pub const DEFAULT_DURATION: i32 = 10;

/// Vérifie un mot de passe hors du fil asynchrone (PBKDF2 est volontairement coûteux).
pub(crate) async fn verify_blocking(st: &AppState, password: String, stored: String) -> bool {
    st.password_verifications.fetch_add(1, Ordering::Relaxed);
    tokio::task::spawn_blocking(move || auth::verify_password(&password, &stored)).await.unwrap_or(false)
}

/// Hache un mot de passe hors du fil asynchrone.
pub(crate) async fn hash_blocking(st: &AppState, password: String) -> Result<String, ApiError> {
    let iterations = st.settings.pbkdf2_iterations;
    tokio::task::spawn_blocking(move || auth::hash_password(&password, iterations))
        .await
        .map_err(ApiError::internal)
}

pub fn router(st: &AppState) -> Router<AppState> {
    let upload_limit = st.settings.max_upload_bytes().saturating_add(MULTIPART_OVERHEAD) as usize;
    Router::new()
        // public
        .route("/", get(public::root))
        .route("/api/health", get(public::health))
        .route("/files/{stored_name}", get(public::get_file))
        .route("/admin", get(public::admin_redirect))
        .route("/admin/", get(public::admin_index))
        .route("/admin/{*rel}", get(public::admin_file))
        .route("/player", get(public::player_redirect))
        .route("/player/", get(public::player_index))
        .route("/player/{*rel}", get(public::player_file))
        // authentification
        .route("/api/auth/login", post(account::login))
        .route("/api/auth/logout", post(account::logout))
        .route("/api/auth/me", get(account::me))
        .route("/api/auth/password", post(account::change_password))
        // tableau de bord
        .route("/api/stats", get(dashboard::stats))
        // médias
        .route("/api/media", get(media::list_media).post(media::upload_media).layer(DefaultBodyLimit::max(upload_limit)))
        .route("/api/media/{media_id}", axum::routing::delete(media::delete_media))
        // playlists
        .route("/api/playlists", get(playlists::list_playlists).post(playlists::create_playlist))
        .route(
            "/api/playlists/{pid}",
            get(playlists::get_playlist).put(playlists::rename_playlist).delete(playlists::delete_playlist),
        )
        .route("/api/playlists/{pid}/items", put(playlists::set_items))
        .route("/api/playlists/{pid}/duplicate", post(playlists::duplicate_playlist))
        .route("/api/playlists/{pid}/publish", post(playlists::publish_playlist))
        .route("/api/playlists/{pid}/preview", get(playlists::preview_draft))
        .route("/api/playlists/{pid}/versions", get(playlists::list_versions))
        .route("/api/versions/{vid}", get(playlists::get_version))
        // écrans
        .route("/api/screens", get(screens::list_screens).post(screens::create_screen))
        .route("/api/screens/{sid}", put(screens::rename_screen).delete(screens::delete_screen))
        .route("/api/screens/{sid}/group", post(screens::set_screen_group))
        .route("/api/screens/{sid}/assign", post(screens::assign_version))
        .route("/api/screens/{sid}/manifest", get(screens::screen_preview))
        .route("/api/screens/{sid}/rotate-token", post(screens::rotate_token))
        .route("/api/screens/{sid}/revoke-token", post(screens::revoke_token))
        // groupes d'écrans
        .route("/api/groups", get(groups::list_groups).post(groups::create_group))
        .route("/api/groups/{gid}", put(groups::rename_group).delete(groups::delete_group))
        .route("/api/groups/{gid}/assign", post(groups::assign_group))
        // planning
        .route("/api/schedules", get(schedules::list_schedules).post(schedules::create_schedule))
        .route("/api/schedules/{rid}", axum::routing::delete(schedules::delete_schedule))
        // comptes et journal (admin)
        .route("/api/users", get(users::list_users).post(users::create_user))
        .route("/api/users/{uid}", patch(users::update_user).delete(users::delete_user))
        .route("/api/audit", get(users::list_audit))
        // lecteur
        .route("/api/player/manifest", get(player::player_manifest))
}
