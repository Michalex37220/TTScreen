//! TTscreen : système d'affichage dynamique (port Rust de l'API FastAPI d'origine).
//!
//! L'organisation des modules suit celle du paquet Python `app/` :
//! `config`, `db`, `models`, `schemas`, `auth`, `audit`, `deps`, `ratelimit`, `security`,
//! `storage`, `publishing`, `api` (routes) et `app` (assemblage, ex-`main.py`).
//! Le contrat HTTP (routes, codes, messages d'erreur en français, JSON) est identique :
//! l'interface d'administration (`frontend/`) et le lecteur (`player/`) fonctionnent sans changement.

pub mod api;
pub mod app;
pub mod audit;
pub mod auth;
pub mod config;
pub mod db;
pub mod deps;
pub mod error;
pub mod extract;
pub mod models;
pub mod proxy;
pub mod publishing;
pub mod ratelimit;
pub mod scheduling;
pub mod schemas;
pub mod security;
pub mod storage;

pub use app::{build_router, build_state, create_app, AppState};
pub use config::Settings;
