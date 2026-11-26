//! Routes publiques : santé, médias, pages statiques (interface d'administration et lecteur).

use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderName, HeaderValue};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use serde_json::{json, Value};
use std::path::{Component, Path as FsPath};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::app::AppState;
use crate::error::ApiError;
use crate::storage::is_stored_name;

pub async fn root() -> Redirect {
    Redirect::temporary("/admin/")
}

pub async fn health() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

/// Sert un fichier via `ServeFile` : types MIME, requêtes `Range` (lecture vidéo), `Last-Modified`.
async fn serve_file(path: std::path::PathBuf, req: Request) -> Response {
    match ServeFile::new(path).oneshot(req).await {
        Ok(response) => response.map(Body::new).into_response(),
        Err(never) => match never {},
    }
}

/// Seuls les noms produits par l'application sont servis (`<sha256>.<ext autorisée>`) :
/// ni `.tmp`, ni aucun autre fichier du volume.
pub async fn get_file(State(st): State<AppState>, Path(stored_name): Path<String>, req: Request) -> Response {
    if !is_stored_name(&stored_name) {
        return ApiError::new(404, "Not Found").into_response();
    }
    let path = st.settings.media_dir.join(&stored_name);
    match tokio::fs::metadata(&path).await {
        Ok(meta) if meta.is_file() => {}
        _ => return ApiError::new(404, "Not Found").into_response(),
    }
    let mut response = serve_file(path, req).await;
    let headers = response.headers_mut();
    headers.insert(HeaderName::from_static("x-content-type-options"), HeaderValue::from_static("nosniff"));
    // Le nom du fichier est son SHA-256 : le contenu ne change jamais, le cache peut être éternel
    headers.insert(
        HeaderName::from_static("cache-control"),
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    response
}

/// Sert `rel` sous `dir` (comme `StaticFiles(html=True)`) : un dossier donne son `index.html`,
/// tout chemin sortant du dossier (`..`) est refusé.
async fn serve_dir(dir: &FsPath, rel: &str, req: Request) -> Response {
    let mut path = dir.to_path_buf();
    for component in FsPath::new(rel).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            _ => return not_found_text(),
        }
    }
    if rel.is_empty() || rel.ends_with('/') || path.is_dir() {
        path.push("index.html");
    }
    if !path.is_file() {
        return not_found_text();
    }
    serve_file(path, req).await
}

fn not_found_text() -> Response {
    (axum::http::StatusCode::NOT_FOUND, "Not Found").into_response()
}

pub async fn admin_redirect() -> Redirect {
    Redirect::temporary("/admin/")
}

pub async fn admin_index(State(st): State<AppState>, req: Request) -> Response {
    serve_dir(&st.settings.frontend_dir, "", req).await
}

pub async fn admin_file(State(st): State<AppState>, Path(rel): Path<String>, req: Request) -> Response {
    serve_dir(&st.settings.frontend_dir, &rel, req).await
}

pub async fn player_redirect() -> Redirect {
    Redirect::temporary("/player/")
}

pub async fn player_index(State(st): State<AppState>, req: Request) -> Response {
    serve_dir(&st.settings.player_dir, "", req).await
}

pub async fn player_file(State(st): State<AppState>, Path(rel): Path<String>, req: Request) -> Response {
    serve_dir(&st.settings.player_dir, &rel, req).await
}

/// Route inconnue : même format d'erreur que FastAPI.
pub async fn not_found() -> ApiError {
    ApiError::new(404, "Not Found")
}
