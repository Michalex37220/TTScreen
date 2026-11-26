//! Planning : « cette version, ces jours, de HH:MM à HH:MM » pour un écran ou un groupe.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait};
use sea_orm::PaginatorTrait;

use crate::app::AppState;
use crate::audit::log_action;
use crate::deps::{ClientIp, Editor, Guard, Viewer};
use crate::error::ApiError;
use crate::extract::{ApiJson, Id};
use crate::models::{playlist_version, schedule_rule, screen, screen_group, utcnow};
use crate::scheduling::{days_to_mask, format_hhmm, mask_to_days, parse_hhmm};
use crate::schemas::{ScheduleIn, ScheduleOut};

/// Garde-fou contre un remplissage abusif de la table.
pub const MAX_RULES_PER_TARGET: u64 = 100;

pub fn schedule_out(r: &schedule_rule::Model) -> ScheduleOut {
    ScheduleOut {
        id: r.id,
        screen_id: r.screen_id,
        group_id: r.group_id,
        version_id: r.version_id,
        days: mask_to_days(r.days),
        start: format_hhmm(r.start_min),
        end: format_hhmm(r.end_min),
        created_at: r.created_at,
    }
}

pub async fn list_schedules(_g: Viewer, State(st): State<AppState>) -> Result<Json<Vec<ScheduleOut>>, ApiError> {
    let rows = schedule_rule::Entity::find().order_by_asc(schedule_rule::Column::Id).all(&st.db).await?;
    Ok(Json(rows.iter().map(schedule_out).collect()))
}

pub async fn create_schedule(
    Guard(principal): Editor,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<ScheduleIn>,
) -> Result<(StatusCode, Json<ScheduleOut>), ApiError> {
    // La validation a déjà garanti : exactement une cible, heures valides, début < fin
    let target = if let Some(sid) = body.screen_id {
        let sid = sid as i32;
        if screen::Entity::find_by_id(sid).one(&st.db).await?.is_none() {
            return Err(ApiError::new(404, "Écran introuvable"));
        }
        (Some(sid), None, format!("screen:{sid}"))
    } else {
        let gid = body.group_id.unwrap_or(0) as i32;
        if screen_group::Entity::find_by_id(gid).one(&st.db).await?.is_none() {
            return Err(ApiError::new(404, "Groupe introuvable"));
        }
        (None, Some(gid), format!("group:{gid}"))
    };
    if playlist_version::Entity::find_by_id(body.version_id as i32).one(&st.db).await?.is_none() {
        return Err(ApiError::new(404, "Version introuvable"));
    }
    let existing = schedule_rule::Entity::find()
        .filter(match (target.0, target.1) {
            (Some(s), _) => schedule_rule::Column::ScreenId.eq(s),
            (_, g) => schedule_rule::Column::GroupId.eq(g.unwrap_or(0)),
        })
        .count(&st.db)
        .await?;
    if existing >= MAX_RULES_PER_TARGET {
        return Err(ApiError::new(409, format!("Trop de règles pour cette cible ({MAX_RULES_PER_TARGET} maximum)")));
    }

    let start = parse_hhmm(&body.start).unwrap_or(0);
    let end = parse_hhmm(&body.end).unwrap_or(0);
    let txn = st.db.begin().await?;
    let rule = schedule_rule::ActiveModel {
        screen_id: Set(target.0),
        group_id: Set(target.1),
        version_id: Set(body.version_id as i32),
        days: Set(days_to_mask(&body.days)),
        start_min: Set(start),
        end_min: Set(end),
        created_at: Set(utcnow()),
        ..Default::default()
    }
    .insert(&txn)
    .await?;
    let detail = format!("version_id={} {}-{} jours={:?}", rule.version_id, body.start, body.end, mask_to_days(rule.days));
    log_action(&txn, &ip, Some(&principal), "schedule.create", Some(target.2), Some(detail)).await?;
    txn.commit().await?;
    Ok((StatusCode::CREATED, Json(schedule_out(&rule))))
}

pub async fn delete_schedule(
    Guard(principal): Editor,
    Id(rid): Id,
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<StatusCode, ApiError> {
    let rule = schedule_rule::Entity::find_by_id(rid)
        .one(&st.db)
        .await?
        .ok_or_else(|| ApiError::new(404, "Règle introuvable"))?;
    let target = match (rule.screen_id, rule.group_id) {
        (Some(s), _) => format!("screen:{s}"),
        (_, Some(g)) => format!("group:{g}"),
        _ => "?".to_string(),
    };
    let txn = st.db.begin().await?;
    log_action(&txn, &ip, Some(&principal), "schedule.delete", Some(target), Some(format!("règle {rid}"))).await?;
    schedule_rule::Entity::delete_by_id(rid).exec(&txn).await?;
    txn.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
