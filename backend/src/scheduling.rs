//! Quelle version un écran doit-il afficher maintenant ? (groupes + planning)
//!
//! Priorité, du plus précis au plus général :
//! 1. une règle de planning **de l'écran** active à cet instant ;
//! 2. le contenu **fixé sur l'écran** ;
//! 3. une règle de planning **du groupe** de l'écran, active à cet instant ;
//! 4. le contenu **fixé sur le groupe**.
//!
//! Si plusieurs règles d'une même cible sont actives en même temps, la plus récente (identifiant le plus élevé) gagne.
//! Les jours et heures sont interprétés dans le fuseau configuré (`TTSCREEN_TIMEZONE`).

use chrono::{Datelike, NaiveDateTime, TimeZone, Timelike};
use chrono_tz::Tz;
use sea_orm::{ConnectionTrait, DbErr, EntityTrait, QueryOrder};
use std::collections::HashMap;

use crate::models::{schedule_rule, screen, screen_group};

/// Les sept jours (bit 0 = lundi … bit 6 = dimanche).
pub const ALL_DAYS: i32 = 0b111_1111;
/// 24:00, borne de fin d'une plage qui va jusqu'à minuit.
pub const END_OF_DAY: i32 = 24 * 60;

/// `[0, 2, 4]` → masque de bits. Les valeurs hors 0..=6 sont ignorées (la validation les refuse en amont).
pub fn days_to_mask(days: &[i64]) -> i32 {
    days.iter().filter(|d| (0..=6).contains(*d)).fold(0, |mask, d| mask | (1 << d))
}

/// Masque de bits → jours triés (0 = lundi … 6 = dimanche).
pub fn mask_to_days(mask: i32) -> Vec<u8> {
    (0..7u8).filter(|d| mask & (1 << d) != 0).collect()
}

/// `"08:30"` → 510. Accepte `00:00` à `23:59`, et `24:00` (= 1440, valable comme heure de fin).
pub fn parse_hhmm(s: &str) -> Option<i32> {
    let (h, m) = s.split_once(':')?;
    if h.len() != 2 || m.len() != 2 || !h.bytes().all(|b| b.is_ascii_digit()) || !m.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (h, m): (i32, i32) = (h.parse().ok()?, m.parse().ok()?);
    if m > 59 || h > 24 || (h == 24 && m != 0) {
        return None;
    }
    Some(h * 60 + m)
}

/// 510 → `"08:30"` ; 1440 → `"24:00"`.
pub fn format_hhmm(minutes: i32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Heure locale (fuseau `tz`) correspondant à un instant UTC « naïf ».
pub fn local_now(tz: Tz, utc: NaiveDateTime) -> NaiveDateTime {
    tz.from_utc_datetime(&utc).naive_local()
}

/// La règle est-elle active à cet instant local ? (début inclus, fin exclue)
pub fn rule_active(rule: &schedule_rule::Model, now_local: NaiveDateTime) -> bool {
    let day = now_local.weekday().num_days_from_monday();
    let minute = (now_local.hour() * 60 + now_local.minute()) as i32;
    rule.days & (1 << day) != 0 && rule.start_min <= minute && minute < rule.end_min
}

/// D'où vient le contenu d'un écran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    ScreenSchedule,
    Screen,
    GroupSchedule,
    Group,
}

impl Source {
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::ScreenSchedule => "screen_schedule",
            Source::Screen => "screen",
            Source::GroupSchedule => "group_schedule",
            Source::Group => "group",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    pub version_id: Option<i32>,
    pub source: Option<Source>,
}

impl Resolution {
    const NONE: Resolution = Resolution { version_id: None, source: None };

    fn of(version_id: i32, source: Source) -> Resolution {
        Resolution { version_id: Some(version_id), source: Some(source) }
    }
}

/// Règle active la plus récente parmi celles qui satisfont `belongs`.
fn latest_active<'a>(
    rules: &'a [schedule_rule::Model],
    now_local: NaiveDateTime,
    belongs: impl Fn(&schedule_rule::Model) -> bool,
) -> Option<&'a schedule_rule::Model> {
    rules.iter().filter(|r| belongs(r) && rule_active(r, now_local)).max_by_key(|r| r.id)
}

/// Fonction pure : aucune base de données, tout est passé en paramètre (donc testable à l'unité).
pub fn resolve(
    screen: &screen::Model,
    group: Option<&screen_group::Model>,
    rules: &[schedule_rule::Model],
    now_local: NaiveDateTime,
) -> Resolution {
    if let Some(r) = latest_active(rules, now_local, |r| r.screen_id == Some(screen.id)) {
        return Resolution::of(r.version_id, Source::ScreenSchedule);
    }
    if let Some(v) = screen.version_id {
        return Resolution::of(v, Source::Screen);
    }
    if let Some(g) = group {
        if let Some(r) = latest_active(rules, now_local, |r| r.group_id == Some(g.id)) {
            return Resolution::of(r.version_id, Source::GroupSchedule);
        }
        if let Some(v) = g.version_id {
            return Resolution::of(v, Source::Group);
        }
    }
    Resolution::NONE
}

/// Données nécessaires pour résoudre plusieurs écrans avec deux requêtes seulement.
pub struct ResolveCtx {
    pub groups: HashMap<i32, screen_group::Model>,
    pub rules: Vec<schedule_rule::Model>,
    pub now_local: NaiveDateTime,
}

impl ResolveCtx {
    pub fn group_of(&self, s: &screen::Model) -> Option<&screen_group::Model> {
        s.group_id.and_then(|g| self.groups.get(&g))
    }

    pub fn resolve(&self, s: &screen::Model) -> Resolution {
        resolve(s, self.group_of(s), &self.rules, self.now_local)
    }
}

pub async fn load_ctx<C: ConnectionTrait>(db: &C, tz: Tz) -> Result<ResolveCtx, DbErr> {
    let groups = screen_group::Entity::find().all(db).await?.into_iter().map(|g| (g.id, g)).collect();
    let rules = schedule_rule::Entity::find().order_by_asc(schedule_rule::Column::Id).all(db).await?;
    Ok(ResolveCtx { groups, rules, now_local: local_now(tz, crate::models::utcnow()) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }
    // 2026-03-02 est un lundi
    const MON: (i32, u32, u32) = (2026, 3, 2);
    fn mon(h: u32, m: u32) -> NaiveDateTime {
        at(MON.0, MON.1, MON.2, h, m)
    }
    fn sun(h: u32, m: u32) -> NaiveDateTime {
        at(2026, 3, 8, h, m)
    }

    fn screen(id: i32, version: Option<i32>, group: Option<i32>) -> screen::Model {
        screen::Model {
            id,
            name: format!("S{id}"),
            token_hash: None,
            version_id: version,
            last_seen: None,
            created_at: mon(0, 0),
            group_id: group,
        }
    }
    fn group(id: i32, version: Option<i32>) -> screen_group::Model {
        screen_group::Model { id, name: format!("G{id}"), version_id: version, created_at: mon(0, 0) }
    }
    fn rule(id: i32, screen_id: Option<i32>, group_id: Option<i32>, version: i32, days: &[i64], start: i32, end: i32) -> schedule_rule::Model {
        schedule_rule::Model {
            id,
            screen_id,
            group_id,
            version_id: version,
            days: days_to_mask(days),
            start_min: start,
            end_min: end,
            created_at: mon(0, 0),
        }
    }

    // ---------- conversions ----------
    #[test]
    fn day_masks_round_trip() {
        assert_eq!(days_to_mask(&[0, 1, 2, 3, 4, 5, 6]), ALL_DAYS);
        assert_eq!(days_to_mask(&[0, 4]), 0b0010001);
        assert_eq!(days_to_mask(&[]), 0);
        assert_eq!(days_to_mask(&[7, -1, 99]), 0, "valeurs hors plage ignorées");
        assert_eq!(days_to_mask(&[2, 2, 2]), 0b100);
        assert_eq!(mask_to_days(0b0010001), vec![0, 4]);
        assert_eq!(mask_to_days(ALL_DAYS), vec![0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(mask_to_days(0), Vec::<u8>::new());
    }

    #[test]
    fn hhmm_parsing_and_formatting() {
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("08:30"), Some(510));
        assert_eq!(parse_hhmm("23:59"), Some(1439));
        assert_eq!(parse_hhmm("24:00"), Some(1440));
        for bad in ["24:01", "25:00", "8:30", "08:5", "08:60", "08-30", "0830", "", "ab:cd", "08:30:00", " 08:30", "-1:30"] {
            assert_eq!(parse_hhmm(bad), None, "{bad:?}");
        }
        assert_eq!(format_hhmm(510), "08:30");
        assert_eq!(format_hhmm(0), "00:00");
        assert_eq!(format_hhmm(1440), "24:00");
        for m in [0, 1, 59, 60, 510, 1439, 1440] {
            assert_eq!(parse_hhmm(&format_hhmm(m)), Some(m));
        }
    }

    #[test]
    fn local_time_follows_the_timezone_including_dst() {
        let paris: Tz = "Europe/Paris".parse().unwrap();
        // hiver : UTC+1
        assert_eq!(local_now(paris, at(2026, 1, 15, 10, 0)), at(2026, 1, 15, 11, 0));
        // été : UTC+2
        assert_eq!(local_now(paris, at(2026, 7, 15, 10, 0)), at(2026, 7, 15, 12, 0));
        // le jour peut changer (lundi 23 h UTC = mardi 00 h à Paris en hiver)
        assert_eq!(local_now(paris, at(2026, 3, 2, 23, 30)), at(2026, 3, 3, 0, 30));
        assert_eq!(local_now("UTC".parse().unwrap(), at(2026, 3, 2, 23, 30)), at(2026, 3, 2, 23, 30));
    }

    // ---------- règle active ----------
    #[test]
    fn rule_window_is_start_inclusive_end_exclusive() {
        let r = rule(1, Some(1), None, 5, &[0], 8 * 60, 12 * 60); // lundi 08:00-12:00
        assert!(!rule_active(&r, mon(7, 59)));
        assert!(rule_active(&r, mon(8, 0)));
        assert!(rule_active(&r, mon(11, 59)));
        assert!(!rule_active(&r, mon(12, 0)));
    }

    #[test]
    fn rule_only_applies_on_selected_days() {
        let r = rule(1, Some(1), None, 5, &[0, 1, 2, 3, 4], 0, END_OF_DAY); // semaine, toute la journée
        assert!(rule_active(&r, mon(10, 0)));
        assert!(!rule_active(&r, sun(10, 0)));
        let weekend = rule(2, Some(1), None, 5, &[5, 6], 0, END_OF_DAY);
        assert!(rule_active(&weekend, sun(10, 0)));
        assert!(!rule_active(&weekend, mon(10, 0)));
    }

    #[test]
    fn a_rule_ending_at_24h_covers_the_last_minute_of_the_day() {
        let r = rule(1, Some(1), None, 5, &[0], 22 * 60, END_OF_DAY);
        assert!(rule_active(&r, mon(23, 59)));
        assert!(!rule_active(&r, mon(21, 59)));
    }

    #[test]
    fn a_rule_without_days_never_applies() {
        let r = rule(1, Some(1), None, 5, &[], 0, END_OF_DAY);
        for t in [mon(0, 0), mon(12, 0), sun(12, 0)] {
            assert!(!rule_active(&r, t));
        }
    }

    // ---------- priorités ----------
    #[test]
    fn nothing_assigned_means_no_content() {
        let r = resolve(&screen(1, None, None), None, &[], mon(10, 0));
        assert_eq!(r, Resolution::NONE);
        // groupe sans contenu non plus
        let r = resolve(&screen(1, None, Some(1)), Some(&group(1, None)), &[], mon(10, 0));
        assert_eq!(r, Resolution::NONE);
    }

    #[test]
    fn group_content_is_inherited_by_its_screens() {
        let r = resolve(&screen(1, None, Some(1)), Some(&group(1, Some(10))), &[], mon(10, 0));
        assert_eq!((r.version_id, r.source), (Some(10), Some(Source::Group)));
    }

    #[test]
    fn screen_content_overrides_the_group() {
        let r = resolve(&screen(1, Some(20), Some(1)), Some(&group(1, Some(10))), &[], mon(10, 0));
        assert_eq!((r.version_id, r.source), (Some(20), Some(Source::Screen)));
    }

    #[test]
    fn screen_without_group_uses_its_own_content() {
        let r = resolve(&screen(1, Some(20), None), None, &[], mon(10, 0));
        assert_eq!((r.version_id, r.source), (Some(20), Some(Source::Screen)));
    }

    #[test]
    fn group_schedule_beats_group_content_only_while_active() {
        let g = group(1, Some(10));
        let rules = [rule(1, None, Some(1), 11, &[0], 8 * 60, 12 * 60)];
        let s = screen(1, None, Some(1));
        let morning = resolve(&s, Some(&g), &rules, mon(9, 0));
        assert_eq!((morning.version_id, morning.source), (Some(11), Some(Source::GroupSchedule)));
        let afternoon = resolve(&s, Some(&g), &rules, mon(14, 0));
        assert_eq!((afternoon.version_id, afternoon.source), (Some(10), Some(Source::Group)), "retour au contenu du groupe");
    }

    #[test]
    fn screen_schedule_beats_everything() {
        let g = group(1, Some(10));
        let s = screen(1, Some(20), Some(1));
        let rules = [
            rule(1, None, Some(1), 11, &[0], 0, END_OF_DAY),
            rule(2, Some(1), None, 30, &[0], 9 * 60, 10 * 60),
        ];
        let during = resolve(&s, Some(&g), &rules, mon(9, 30));
        assert_eq!((during.version_id, during.source), (Some(30), Some(Source::ScreenSchedule)));
        // hors de la plage de l'écran : le contenu fixé sur l'écran reprend (il est plus précis que le planning du groupe)
        let outside = resolve(&s, Some(&g), &rules, mon(15, 0));
        assert_eq!((outside.version_id, outside.source), (Some(20), Some(Source::Screen)));
    }

    #[test]
    fn screen_without_fixed_content_falls_back_to_the_group_schedule_outside_its_own() {
        let g = group(1, Some(10));
        let s = screen(1, None, Some(1));
        let rules = [
            rule(1, None, Some(1), 11, &[0], 0, END_OF_DAY),
            rule(2, Some(1), None, 30, &[0], 9 * 60, 10 * 60),
        ];
        assert_eq!(resolve(&s, Some(&g), &rules, mon(9, 30)).version_id, Some(30));
        let outside = resolve(&s, Some(&g), &rules, mon(15, 0));
        assert_eq!((outside.version_id, outside.source), (Some(11), Some(Source::GroupSchedule)));
    }

    #[test]
    fn most_recent_rule_wins_when_several_overlap() {
        let s = screen(1, None, None);
        let rules = [
            rule(1, Some(1), None, 100, &[0], 0, END_OF_DAY),
            rule(7, Some(1), None, 700, &[0], 8 * 60, 18 * 60),
            rule(3, Some(1), None, 300, &[0], 0, END_OF_DAY),
        ];
        assert_eq!(resolve(&s, None, &rules, mon(10, 0)).version_id, Some(700));
        assert_eq!(resolve(&s, None, &rules, mon(20, 0)).version_id, Some(300), "7 inactive : la plus récente active");
    }

    #[test]
    fn rules_of_other_targets_are_ignored() {
        let s = screen(1, None, Some(1));
        let g = group(1, None);
        let rules = [
            rule(1, Some(2), None, 50, &[0], 0, END_OF_DAY), // autre écran
            rule(2, None, Some(2), 60, &[0], 0, END_OF_DAY), // autre groupe
        ];
        assert_eq!(resolve(&s, Some(&g), &rules, mon(10, 0)), Resolution::NONE);
    }

    #[test]
    fn group_rules_are_ignored_for_a_screen_outside_the_group() {
        let s = screen(1, None, None);
        let rules = [rule(1, None, Some(1), 60, &[0], 0, END_OF_DAY)];
        assert_eq!(resolve(&s, None, &rules, mon(10, 0)), Resolution::NONE);
    }

    #[test]
    fn dangling_group_reference_is_treated_as_no_group() {
        let ctx = ResolveCtx { groups: HashMap::new(), rules: vec![], now_local: mon(10, 0) };
        let s = screen(1, None, Some(99));
        assert!(ctx.group_of(&s).is_none());
        assert_eq!(ctx.resolve(&s), Resolution::NONE);
    }

    #[test]
    fn source_names_are_stable_api_values() {
        assert_eq!(Source::ScreenSchedule.as_str(), "screen_schedule");
        assert_eq!(Source::Screen.as_str(), "screen");
        assert_eq!(Source::GroupSchedule.as_str(), "group_schedule");
        assert_eq!(Source::Group.as_str(), "group");
    }
}
