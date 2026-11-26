//! Schémas d'entrée/sortie de l'API et règles de validation (équivalent de `schemas.py` / pydantic).

use chrono::NaiveDateTime;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

use crate::auth::is_reserved;
use crate::error::FieldError;

/// Les identifiants sont des INTEGER 32 bits côté base : on refuse plus grand (sinon erreur → 500).
pub const MAX_ID: i64 = (1 << 31) - 1;
/// Éléments par playlist.
pub const MAX_ITEMS: usize = 500;

/// Validation après désérialisation (bornes, longueurs, motifs).
pub trait Validate {
    fn validate(&self) -> Result<(), Vec<FieldError>>;
}

// ---------- aides de validation (messages calqués sur pydantic) ----------
fn plural(n: usize) -> &'static str {
    if n == 1 {
        "character"
    } else {
        "characters"
    }
}

fn check_len(errs: &mut Vec<FieldError>, loc: &[Value], value: &str, min: usize, max: usize) {
    let n = value.chars().count();
    if n < min {
        errs.push(FieldError::new(
            loc.to_vec(),
            "string_too_short",
            format!("String should have at least {min} {}", plural(min)),
        ));
    } else if n > max {
        errs.push(FieldError::new(
            loc.to_vec(),
            "string_too_long",
            format!("String should have at most {max} {}", plural(max)),
        ));
    }
}

fn check_range(errs: &mut Vec<FieldError>, loc: &[Value], value: i64, min: i64, max: i64) {
    if value < min {
        errs.push(FieldError::new(
            loc.to_vec(),
            "greater_than_equal",
            format!("Input should be greater than or equal to {min}"),
        ));
    } else if value > max {
        errs.push(FieldError::new(
            loc.to_vec(),
            "less_than_equal",
            format!("Input should be less than or equal to {max}"),
        ));
    }
}

fn loc(path: &[&str]) -> Vec<Value> {
    path.iter().map(|p| json!(p)).collect()
}

fn into_result(errs: Vec<FieldError>) -> Result<(), Vec<FieldError>> {
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

// ---------- playlists ----------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemType {
    Text,
    Image,
    Video,
}

impl ItemType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ItemType::Text => "text",
            ItemType::Image => "image",
            ItemType::Video => "video",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ItemIn {
    #[serde(rename = "type")]
    pub item_type: ItemType,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub media_id: Option<i64>,
    #[serde(default)]
    pub duration_s: Option<i64>,
}

impl ItemIn {
    fn validate_at(&self, prefix: &[Value], errs: &mut Vec<FieldError>) {
        let at = |field: &str| {
            let mut l = prefix.to_vec();
            l.push(json!(field));
            l
        };
        if let Some(t) = &self.text {
            if t.chars().count() > 2000 {
                errs.push(FieldError::new(at("text"), "string_too_long", "String should have at most 2000 characters"));
            }
        }
        if let Some(m) = self.media_id {
            check_range(errs, &at("media_id"), m, 1, MAX_ID);
        }
        if let Some(d) = self.duration_s {
            check_range(errs, &at("duration_s"), d, 1, 3600);
        }
    }
}

impl Validate for ItemIn {
    fn validate(&self) -> Result<(), Vec<FieldError>> {
        let mut errs = Vec::new();
        self.validate_at(&[json!("body")], &mut errs);
        into_result(errs)
    }
}

/// Corps de `PUT /api/playlists/{id}/items` : tableau JSON de 500 éléments au plus.
impl Validate for Vec<ItemIn> {
    fn validate(&self) -> Result<(), Vec<FieldError>> {
        let mut errs = Vec::new();
        if self.len() > MAX_ITEMS {
            errs.push(FieldError::new(
                loc(&["body"]),
                "too_long",
                format!("List should have at most {MAX_ITEMS} items after validation, not {}", self.len()),
            ));
            return Err(errs);
        }
        for (i, item) in self.iter().enumerate() {
            item.validate_at(&[json!("body"), json!(i)], &mut errs);
        }
        into_result(errs)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistIn {
    pub name: String,
}

impl Validate for PlaylistIn {
    fn validate(&self) -> Result<(), Vec<FieldError>> {
        let mut errs = Vec::new();
        check_len(&mut errs, &loc(&["body", "name"]), &self.name, 1, 120);
        into_result(errs)
    }
}

#[derive(Debug, Serialize)]
pub struct ItemOut {
    #[serde(rename = "type")]
    pub item_type: String,
    pub text: Option<String>,
    pub media_id: Option<i32>,
    pub duration_s: Option<i32>,
    pub id: i32,
    pub position: i32,
}

#[derive(Debug, Serialize)]
pub struct PlaylistOut {
    pub id: i32,
    pub name: String,
    pub items: Vec<ItemOut>,
}

#[derive(Debug, Serialize)]
pub struct MediaOut {
    pub id: i32,
    pub original_name: String,
    pub kind: String,
    pub mime: String,
    pub size: i64,
    pub sha256: String,
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct VersionOut {
    pub id: i32,
    pub playlist_id: i32,
    pub number: i32,
    pub created_at: NaiveDateTime,
    pub manifest: Value,
}

// ---------- écrans ----------
/// Création d'un écran ; `group_id` (facultatif) le place directement dans un groupe.
#[derive(Debug, Clone, Deserialize)]
pub struct ScreenIn {
    pub name: String,
    #[serde(default)]
    pub group_id: Option<i64>,
}

impl Validate for ScreenIn {
    fn validate(&self) -> Result<(), Vec<FieldError>> {
        let mut errs = Vec::new();
        check_len(&mut errs, &loc(&["body", "name"]), &self.name, 1, 120);
        if let Some(g) = self.group_id {
            check_range(&mut errs, &loc(&["body", "group_id"]), g, 1, MAX_ID);
        }
        into_result(errs)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScreenRename {
    pub name: String,
}

impl Validate for ScreenRename {
    fn validate(&self) -> Result<(), Vec<FieldError>> {
        let mut errs = Vec::new();
        check_len(&mut errs, &loc(&["body", "name"]), &self.name, 1, 120);
        into_result(errs)
    }
}

#[derive(Debug, Serialize)]
pub struct ScreenOut {
    pub id: i32,
    pub name: String,
    /// Contenu fixé sur cet écran lui-même (prioritaire sur le groupe).
    pub version_id: Option<i32>,
    pub online: bool,
    pub last_seen: Option<NaiveDateTime>,
    pub has_token: bool,
    pub group_id: Option<i32>,
    pub group_name: Option<String>,
    /// Version réellement diffusée à cet instant (groupe, planning et contenu fixé pris en compte).
    pub effective_version_id: Option<i32>,
    /// `screen_schedule` | `screen` | `group_schedule` | `group` | null
    pub effective_source: Option<String>,
}

/// Écran + jeton, affiché une seule fois.
#[derive(Debug, Serialize)]
pub struct ScreenCreated {
    #[serde(flatten)]
    pub screen: ScreenOut,
    pub token: String,
}

/// Champ obligatoire mais pouvant valoir `null` (= retirer l'affectation).
fn required_nullable<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Option::<i64>::deserialize(d)
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssignIn {
    #[serde(deserialize_with = "required_nullable")]
    pub version_id: Option<i64>,
}

impl Validate for AssignIn {
    fn validate(&self) -> Result<(), Vec<FieldError>> {
        let mut errs = Vec::new();
        if let Some(v) = self.version_id {
            check_range(&mut errs, &loc(&["body", "version_id"]), v, 1, MAX_ID);
        }
        into_result(errs)
    }
}

/// Place un écran dans un groupe (`group_id` obligatoire, `null` = sortir du groupe).
#[derive(Debug, Clone, Deserialize)]
pub struct SetGroupIn {
    #[serde(deserialize_with = "required_nullable")]
    pub group_id: Option<i64>,
}

impl Validate for SetGroupIn {
    fn validate(&self) -> Result<(), Vec<FieldError>> {
        let mut errs = Vec::new();
        if let Some(g) = self.group_id {
            check_range(&mut errs, &loc(&["body", "group_id"]), g, 1, MAX_ID);
        }
        into_result(errs)
    }
}

// ---------- groupes d'écrans ----------
#[derive(Debug, Clone, Deserialize)]
pub struct GroupIn {
    pub name: String,
}

impl Validate for GroupIn {
    fn validate(&self) -> Result<(), Vec<FieldError>> {
        let mut errs = Vec::new();
        check_len(&mut errs, &loc(&["body", "name"]), &self.name, 1, 120);
        into_result(errs)
    }
}

#[derive(Debug, Serialize)]
pub struct GroupOut {
    pub id: i32,
    pub name: String,
    /// Contenu fixé sur le groupe.
    pub version_id: Option<i32>,
    pub screen_count: i64,
}

// ---------- planning ----------
/// Règle de planning pour un écran OU un groupe (exactement l'un des deux).
/// `days` : 0 = lundi … 6 = dimanche ; `start`/`end` : `HH:MM` (heure locale), `end` peut valoir `24:00`.
#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleIn {
    #[serde(default)]
    pub screen_id: Option<i64>,
    #[serde(default)]
    pub group_id: Option<i64>,
    pub version_id: i64,
    pub days: Vec<i64>,
    pub start: String,
    pub end: String,
}

impl Validate for ScheduleIn {
    fn validate(&self) -> Result<(), Vec<FieldError>> {
        let mut errs = Vec::new();
        if self.screen_id.is_some() == self.group_id.is_some() {
            errs.push(FieldError::new(
                loc(&["body"]),
                "value_error",
                "Value error, Indiquez soit screen_id, soit group_id (un seul des deux)",
            ));
        }
        if let Some(s) = self.screen_id {
            check_range(&mut errs, &loc(&["body", "screen_id"]), s, 1, MAX_ID);
        }
        if let Some(g) = self.group_id {
            check_range(&mut errs, &loc(&["body", "group_id"]), g, 1, MAX_ID);
        }
        check_range(&mut errs, &loc(&["body", "version_id"]), self.version_id, 1, MAX_ID);
        if self.days.is_empty() || self.days.len() > 7 || self.days.iter().any(|d| !(0..=6).contains(d)) {
            errs.push(FieldError::new(
                loc(&["body", "days"]),
                "value_error",
                "Value error, days : 1 à 7 jours parmi 0 (lundi) … 6 (dimanche)",
            ));
        }
        let start = crate::scheduling::parse_hhmm(&self.start).filter(|m| *m < crate::scheduling::END_OF_DAY);
        let end = crate::scheduling::parse_hhmm(&self.end);
        if start.is_none() {
            errs.push(FieldError::new(loc(&["body", "start"]), "value_error", "Value error, start : heure au format HH:MM (00:00 à 23:59)"));
        }
        if end.is_none() {
            errs.push(FieldError::new(loc(&["body", "end"]), "value_error", "Value error, end : heure au format HH:MM (00:00 à 24:00)"));
        }
        if let (Some(s), Some(e)) = (start, end) {
            if e <= s {
                errs.push(FieldError::new(
                    loc(&["body", "end"]),
                    "value_error",
                    "Value error, end doit être postérieure à start (pour passer minuit, créez deux règles)",
                ));
            }
        }
        into_result(errs)
    }
}

#[derive(Debug, Serialize)]
pub struct ScheduleOut {
    pub id: i32,
    pub screen_id: Option<i32>,
    pub group_id: Option<i32>,
    pub version_id: i32,
    pub days: Vec<u8>,
    pub start: String,
    pub end: String,
    pub created_at: NaiveDateTime,
}

// ---------- tableau de bord ----------
#[derive(Debug, Serialize)]
pub struct ScreenStats {
    pub total: i64,
    pub online: i64,
    pub offline: i64,
    /// Écrans qui n'ont, à cet instant, aucun contenu à afficher.
    pub without_content: i64,
}

#[derive(Debug, Serialize)]
pub struct MediaStats {
    pub count: i64,
    pub bytes: i64,
}

#[derive(Debug, Serialize)]
pub struct StatsOut {
    pub screens: ScreenStats,
    pub groups: i64,
    pub playlists: i64,
    pub versions: i64,
    pub schedules: i64,
    pub media: MediaStats,
}

// ---------- comptes, sessions et journal ----------
/// Changement de son propre mot de passe.
#[derive(Debug, Clone, Deserialize)]
pub struct PasswordChangeIn {
    pub current_password: String,
    pub new_password: String,
}

impl Validate for PasswordChangeIn {
    fn validate(&self) -> Result<(), Vec<FieldError>> {
        let mut errs = Vec::new();
        check_len(&mut errs, &loc(&["body", "current_password"]), &self.current_password, 1, 255);
        check_len(&mut errs, &loc(&["body", "new_password"]), &self.new_password, 8, 255);
        into_result(errs)
    }
}

/// Deux modes de connexion : compte nommé (`username` + `password`) ou jeton admin historique (`token` seul).
#[derive(Debug, Clone, Deserialize)]
pub struct LoginIn {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
}

impl Validate for LoginIn {
    fn validate(&self) -> Result<(), Vec<FieldError>> {
        let mut errs = Vec::new();
        for (name, value, max) in [
            ("username", &self.username, 50),
            ("password", &self.password, 255),
            ("token", &self.token, 255),
        ] {
            if let Some(v) = value {
                if v.chars().count() > max {
                    errs.push(FieldError::new(
                        loc(&["body", name]),
                        "string_too_long",
                        format!("String should have at most {max} characters"),
                    ));
                }
            }
        }
        into_result(errs)
    }
}

#[derive(Debug, Serialize)]
pub struct SessionOut {
    pub token: String,
    pub username: String,
    pub role: String,
    /// `None` pour le jeton admin historique (pas de session en base).
    pub expires_at: Option<NaiveDateTime>,
}

#[derive(Debug, Serialize)]
pub struct MeOut {
    pub username: String,
    pub role: String,
    /// True = authentifié avec le jeton admin historique.
    pub legacy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Viewer,
    Editor,
    Admin,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Editor => "editor",
            Role::Admin => "admin",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserIn {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub role: Role,
}

/// Motif `^[A-Za-z0-9_.-]+$` (ASCII uniquement).
pub fn valid_username_chars(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

impl Validate for UserIn {
    fn validate(&self) -> Result<(), Vec<FieldError>> {
        let mut errs = Vec::new();
        check_len(&mut errs, &loc(&["body", "username"]), &self.username, 3, 50);
        if errs.is_empty() && !valid_username_chars(&self.username) {
            errs.push(FieldError::new(
                loc(&["body", "username"]),
                "string_pattern_mismatch",
                "String should match pattern '^[A-Za-z0-9_.-]+$'",
            ));
        }
        if errs.is_empty() && is_reserved(&self.username) {
            errs.push(FieldError::new(
                loc(&["body", "username"]),
                "value_error",
                "Value error, Nom d'utilisateur réservé",
            ));
        }
        // 8 caractères minimum : refuse les mots de passe triviaux
        check_len(&mut errs, &loc(&["body", "password"]), &self.password, 8, 255);
        into_result(errs)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserPatch {
    #[serde(default)]
    pub role: Option<Role>,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub password: Option<String>,
}

impl Validate for UserPatch {
    fn validate(&self) -> Result<(), Vec<FieldError>> {
        let mut errs = Vec::new();
        if let Some(p) = &self.password {
            check_len(&mut errs, &loc(&["body", "password"]), p, 8, 255);
        }
        into_result(errs)
    }
}

#[derive(Debug, Serialize)]
pub struct UserOut {
    pub id: i32,
    pub username: String,
    pub role: String,
    pub active: bool,
    pub created_at: NaiveDateTime,
    pub last_login: Option<NaiveDateTime>,
}

#[derive(Debug, Serialize)]
pub struct AuditOut {
    pub id: i32,
    pub created_at: NaiveDateTime,
    pub username: String,
    pub role: Option<String>,
    pub action: String,
    pub target: Option<String>,
    pub detail: Option<String>,
    pub ip: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(json: Value) -> ItemIn {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn playlist_name_bounds() {
        assert!(PlaylistIn { name: "A".into() }.validate().is_ok());
        assert!(PlaylistIn { name: "é".repeat(120) }.validate().is_ok(), "120 caractères, pas 120 octets");
        assert!(PlaylistIn { name: "x".repeat(121) }.validate().is_err());
        let e = PlaylistIn { name: String::new() }.validate().unwrap_err();
        assert_eq!(e[0].msg, "String should have at least 1 character");
    }

    #[test]
    fn screen_name_bounds() {
        assert!(ScreenIn { name: "Entrée".into(), group_id: None }.validate().is_ok());
        assert!(ScreenIn { name: String::new(), group_id: None }.validate().is_err());
        assert!(ScreenIn { name: "x".repeat(121), group_id: None }.validate().is_err());
    }

    #[test]
    fn item_type_must_be_one_of_three() {
        assert!(serde_json::from_value::<ItemIn>(json!({"type": "text", "text": "a"})).is_ok());
        assert!(serde_json::from_value::<ItemIn>(json!({"type": "audio"})).is_err());
        assert!(serde_json::from_value::<ItemIn>(json!({"text": "a"})).is_err(), "type obligatoire");
    }

    #[test]
    fn item_numeric_bounds() {
        assert!(item(json!({"type":"text","text":"a","duration_s":1})).validate().is_ok());
        assert!(item(json!({"type":"text","text":"a","duration_s":3600})).validate().is_ok());
        assert!(item(json!({"type":"text","text":"a","duration_s":0})).validate().is_err());
        assert!(item(json!({"type":"text","text":"a","duration_s":3601})).validate().is_err());
        assert!(item(json!({"type":"image","media_id":1})).validate().is_ok());
        assert!(item(json!({"type":"image","media_id":0})).validate().is_err());
        assert!(item(json!({"type":"image","media_id":2147483647})).validate().is_ok());
        assert!(item(json!({"type":"image","media_id":2147483648_i64})).validate().is_err());
        assert!(item(json!({"type":"image","media_id": (1_i64 << 40)})).validate().is_err());
    }

    #[test]
    fn item_text_is_capped_at_2000_characters() {
        assert!(item(json!({"type":"text","text":"x".repeat(2000)})).validate().is_ok());
        assert!(item(json!({"type":"text","text":"x".repeat(2001)})).validate().is_err());
    }

    #[test]
    fn item_list_is_capped_at_500_and_errors_carry_the_index() {
        let ok: Vec<ItemIn> = (0..500).map(|_| item(json!({"type":"text","text":"x"}))).collect();
        assert!(ok.validate().is_ok());
        let too_many: Vec<ItemIn> = (0..501).map(|_| item(json!({"type":"text","text":"x"}))).collect();
        assert!(too_many.validate().is_err());
        let bad = vec![item(json!({"type":"text","text":"x"})), item(json!({"type":"text","text":"x","duration_s":0}))];
        let errs = bad.validate().unwrap_err();
        assert_eq!(errs[0].loc, vec![json!("body"), json!(1), json!("duration_s")]);
    }

    #[test]
    fn assign_requires_the_field_but_accepts_null() {
        assert!(serde_json::from_value::<AssignIn>(json!({})).is_err(), "champ obligatoire");
        let a: AssignIn = serde_json::from_value(json!({"version_id": null})).unwrap();
        assert_eq!(a.version_id, None);
        assert!(a.validate().is_ok());
        let b: AssignIn = serde_json::from_value(json!({"version_id": 3})).unwrap();
        assert_eq!(b.version_id, Some(3));
        assert!(AssignIn { version_id: Some(0) }.validate().is_err());
        assert!(AssignIn { version_id: Some(1 << 40) }.validate().is_err());
    }

    #[test]
    fn user_in_rules() {
        let u = |name: &str, pw: &str| UserIn { username: name.into(), password: pw.into(), role: Role::Viewer };
        assert!(u("emma", "motdepasse8").validate().is_ok());
        assert!(u("e.m-m_a9", "motdepasse8").validate().is_ok());
        assert!(u("ab", "motdepasse8").validate().is_err(), "3 caractères minimum");
        assert!(u(&"a".repeat(51), "motdepasse8").validate().is_err());
        assert!(u("emma", "court").validate().is_err(), "mot de passe < 8");
        assert!(u("emma", "1234567").validate().is_err());
        assert!(u("emma", "12345678").validate().is_ok());
        assert!(u("emma", &"p".repeat(256)).validate().is_err());
    }

    #[test]
    fn username_pattern_is_ascii_only() {
        for bad in ["em ma", "emma!", "émma", "em/ma", "emma\n", "a@b.c"] {
            let r = UserIn { username: bad.into(), password: "motdepasse8".into(), role: Role::Viewer }.validate();
            assert!(r.is_err(), "{bad:?}");
        }
    }

    #[test]
    fn reserved_usernames_are_refused_whatever_the_case() {
        for name in ["legacy-admin", "LEGACY-ADMIN", "anonyme", "Anonyme"] {
            let e = UserIn { username: name.into(), password: "motdepasse8".into(), role: Role::Admin }
                .validate()
                .unwrap_err();
            assert!(e[0].msg.contains("réservé"), "{name}");
        }
    }

    #[test]
    fn role_defaults_to_viewer_and_rejects_unknown_values() {
        let u: UserIn = serde_json::from_value(json!({"username":"emma","password":"motdepasse8"})).unwrap();
        assert_eq!(u.role, Role::Viewer);
        assert!(serde_json::from_value::<UserIn>(json!({"username":"emma","password":"motdepasse8","role":"root"})).is_err());
        assert_eq!(Role::Editor.as_str(), "editor");
    }

    #[test]
    fn user_patch_password_rule() {
        assert!(UserPatch { role: None, active: None, password: None }.validate().is_ok());
        assert!(UserPatch { role: None, active: None, password: Some("court".into()) }.validate().is_err());
        assert!(UserPatch { role: Some(Role::Admin), active: Some(false), password: Some("motdepasse8".into()) }
            .validate()
            .is_ok());
    }

    #[test]
    fn login_field_lengths() {
        let l = |u: Option<&str>, p: Option<&str>, t: Option<&str>| LoginIn {
            username: u.map(Into::into),
            password: p.map(Into::into),
            token: t.map(Into::into),
        };
        assert!(l(None, None, None).validate().is_ok());
        assert!(l(Some(&"u".repeat(51)), None, None).validate().is_err());
        assert!(l(None, Some(&"p".repeat(256)), None).validate().is_err());
        assert!(l(None, None, Some(&"t".repeat(256))).validate().is_err());
        assert!(l(Some(&"u".repeat(50)), Some(&"p".repeat(255)), Some(&"t".repeat(255))).validate().is_ok());
    }

    #[test]
    fn screen_created_flattens_the_screen_fields() {
        let out = ScreenCreated {
            screen: ScreenOut {
                id: 2,
                name: "Entrée".into(),
                version_id: None,
                online: false,
                last_seen: None,
                has_token: true,
                group_id: None,
                group_name: None,
                effective_version_id: None,
                effective_source: None,
            },
            token: "abc".into(),
        };
        let v = serde_json::to_value(&out).unwrap();
        assert_eq!(v["id"], 2);
        assert_eq!(v["token"], "abc");
        assert_eq!(v["has_token"], true);
        assert!(v["version_id"].is_null() && v["last_seen"].is_null());
    }

    #[test]
    fn item_out_serializes_type_key() {
        let v = serde_json::to_value(ItemOut {
            item_type: "text".into(), text: Some("a".into()), media_id: None, duration_s: Some(5), id: 1, position: 0,
        })
        .unwrap();
        assert_eq!(v["type"], "text");
        assert!(v.get("item_type").is_none());
    }

    fn schedule(json: Value) -> ScheduleIn {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn schedule_needs_exactly_one_target() {
        let ok = json!({"screen_id": 1, "version_id": 2, "days": [0, 1], "start": "08:00", "end": "12:00"});
        assert!(schedule(ok.clone()).validate().is_ok());
        let mut group = ok.clone();
        group["screen_id"] = Value::Null;
        group["group_id"] = json!(3);
        assert!(schedule(group).validate().is_ok());
        let mut both = ok.clone();
        both["group_id"] = json!(3);
        assert!(schedule(both).validate().is_err(), "deux cibles");
        let mut none = ok.clone();
        none["screen_id"] = Value::Null;
        assert!(schedule(none).validate().is_err(), "aucune cible");
    }

    #[test]
    fn schedule_time_rules() {
        let base = |start: &str, end: &str| {
            schedule(json!({"screen_id": 1, "version_id": 2, "days": [0], "start": start, "end": end}))
        };
        assert!(base("00:00", "24:00").validate().is_ok());
        assert!(base("23:59", "24:00").validate().is_ok());
        assert!(base("08:00", "08:01").validate().is_ok());
        assert!(base("12:00", "12:00").validate().is_err(), "plage vide");
        assert!(base("18:00", "08:00").validate().is_err(), "passer minuit = deux règles");
        assert!(base("24:00", "24:00").validate().is_err(), "start ne peut pas valoir 24:00");
        assert!(base("8:00", "12:00").validate().is_err());
        assert!(base("08:00", "25:00").validate().is_err());
        assert!(base("08:00", "midi").validate().is_err());
    }

    #[test]
    fn schedule_days_and_ids() {
        let with_days = |days: Value| {
            schedule(json!({"screen_id": 1, "version_id": 2, "days": days, "start": "08:00", "end": "09:00"}))
        };
        assert!(with_days(json!([0, 1, 2, 3, 4, 5, 6])).validate().is_ok());
        assert!(with_days(json!([])).validate().is_err());
        assert!(with_days(json!([7])).validate().is_err());
        assert!(with_days(json!([-1])).validate().is_err());
        assert!(with_days(json!([0, 0, 0, 0, 0, 0, 0, 0])).validate().is_err(), "plus de 7 valeurs");
        let bad_ids = schedule(json!({"screen_id": 0, "version_id": 0, "days": [0], "start": "08:00", "end": "09:00"}));
        assert!(bad_ids.validate().unwrap_err().len() >= 2);
        assert!(serde_json::from_value::<ScheduleIn>(json!({"screen_id": 1, "days": [0], "start": "08:00", "end": "09:00"})).is_err(), "version_id obligatoire");
    }

    #[test]
    fn group_and_screen_rename_names() {
        assert!(GroupIn { name: "Hall".into() }.validate().is_ok());
        assert!(GroupIn { name: String::new() }.validate().is_err());
        assert!(GroupIn { name: "x".repeat(121) }.validate().is_err());
        assert!(ScreenRename { name: String::new() }.validate().is_err());
        assert!(ScreenRename { name: "Accueil".into() }.validate().is_ok());
    }

    #[test]
    fn screen_in_accepts_an_optional_group() {
        let s: ScreenIn = serde_json::from_value(json!({"name": "A"})).unwrap();
        assert_eq!(s.group_id, None);
        assert!(s.validate().is_ok());
        let s: ScreenIn = serde_json::from_value(json!({"name": "A", "group_id": 0})).unwrap();
        assert!(s.validate().is_err());
    }

    #[test]
    fn set_group_requires_the_field_but_accepts_null() {
        assert!(serde_json::from_value::<SetGroupIn>(json!({})).is_err());
        let g: SetGroupIn = serde_json::from_value(json!({"group_id": null})).unwrap();
        assert!(g.validate().is_ok());
        assert!(SetGroupIn { group_id: Some(0) }.validate().is_err());
        assert!(SetGroupIn { group_id: Some(3) }.validate().is_ok());
    }

    #[test]
    fn password_change_rules() {
        let c = |cur: &str, new: &str| PasswordChangeIn { current_password: cur.into(), new_password: new.into() };
        assert!(c("ancien", "nouveau-secret").validate().is_ok());
        assert!(c("", "nouveau-secret").validate().is_err());
        assert!(c("ancien", "court").validate().is_err());
        assert!(c("ancien", &"p".repeat(256)).validate().is_err());
    }

    #[test]
    fn max_id_is_2_pow_31_minus_1() {
        assert_eq!(MAX_ID, 2_147_483_647);
    }
}
