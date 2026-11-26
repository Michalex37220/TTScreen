//! Tableau de bord : chiffres clés pour l'accueil de l'interface.

use axum::extract::State;
use axum::Json;
use sea_orm::{EntityTrait, PaginatorTrait, QuerySelect};

use super::screens::screen_out;
use crate::app::AppState;
use crate::deps::Viewer;
use crate::error::ApiError;
use crate::models::{media, playlist, playlist_version, schedule_rule, screen, screen_group};
use crate::scheduling::load_ctx;
use crate::schemas::{MediaStats, ScreenStats, StatsOut};

pub async fn stats(_g: Viewer, State(st): State<AppState>) -> Result<Json<StatsOut>, ApiError> {
    let ctx = load_ctx(&st.db, st.tz).await?;
    let screens = screen::Entity::find().all(&st.db).await?;
    let outs: Vec<_> = screens.iter().map(|s| screen_out(s, &ctx)).collect();
    let online = outs.iter().filter(|o| o.online).count() as i64;
    let without_content = outs.iter().filter(|o| o.effective_version_id.is_none()).count() as i64;
    let sizes: Vec<i32> = media::Entity::find().select_only().column(media::Column::Size).into_tuple().all(&st.db).await?;
    Ok(Json(StatsOut {
        screens: ScreenStats {
            total: outs.len() as i64,
            online,
            offline: outs.len() as i64 - online,
            without_content,
        },
        groups: screen_group::Entity::find().count(&st.db).await? as i64,
        playlists: playlist::Entity::find().count(&st.db).await? as i64,
        versions: playlist_version::Entity::find().count(&st.db).await? as i64,
        schedules: schedule_rule::Entity::find().count(&st.db).await? as i64,
        media: MediaStats { count: sizes.len() as i64, bytes: sizes.iter().map(|s| *s as i64).sum() },
    }))
}
