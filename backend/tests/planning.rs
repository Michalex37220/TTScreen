//! Planning horaire : validation, priorités et effets sur ce que reçoivent les écrans.
//! Les tests d'intégration utilisent des règles « toute la semaine, toute la journée » (toujours actives) ou
//! « tous les jours sauf aujourd'hui » (jamais actives) : le résultat ne dépend donc pas de l'heure du test.
//! Les cas horaires précis (début inclus, fin exclue, fuseaux, passage d'heure) sont testés dans `src/scheduling.rs`.

mod common;

use axum::http::Method;
use chrono::Datelike;
use common::*;
use serde_json::{json, Value};

fn always(target: (&str, &Value), version: i64) -> Value {
    let mut rule = json!({"version_id": version, "days": [0, 1, 2, 3, 4, 5, 6], "start": "00:00", "end": "24:00"});
    rule[target.0] = target.1.clone();
    rule
}

/// Règle qui n'est jamais active : tous les jours SAUF le jour courant (dans le fuseau UTC des tests).
fn never(target: (&str, &Value), version: i64) -> Value {
    let today = chrono::Utc::now().weekday().num_days_from_monday() as i64;
    let days: Vec<i64> = (0..7).filter(|d| *d != today).collect();
    let mut rule = json!({"version_id": version, "days": days, "start": "00:00", "end": "24:00"});
    rule[target.0] = target.1.clone();
    rule
}

// =====================================================================================
// CRUD
// =====================================================================================
#[tokio::test]
async fn schedule_rule_crud_and_shape() {
    let app = TestApp::new().await;
    let s = new_screen(&app, "S").await;
    let v = publish_text(&app, "P", "x").await;
    let body = json!({"screen_id": s["id"], "version_id": v, "days": [4, 0, 2], "start": "08:30", "end": "12:00"});
    let r = app.post_json("/api/schedules", &body).await;
    assert_eq!(r.code, 201, "{}", r.text());
    let rule = r.json();
    assert_eq!(rule["screen_id"], s["id"]);
    assert!(rule["group_id"].is_null());
    assert_eq!(rule["version_id"], v);
    assert_eq!(rule["days"], json!([0, 2, 4]), "jours triés");
    assert_eq!((rule["start"].as_str(), rule["end"].as_str()), (Some("08:30"), Some("12:00")));
    assert!(rule["created_at"].is_string());
    // liste
    let list = app.get("/api/schedules").await.json();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], rule["id"]);
    // suppression
    assert_eq!(app.delete(&format!("/api/schedules/{}", rule["id"])).await.code, 204);
    assert_eq!(app.get("/api/schedules").await.json(), json!([]));
    assert_eq!(app.delete(&format!("/api/schedules/{}", rule["id"])).await.code, 404);
}

#[tokio::test]
async fn schedule_for_a_group() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let v = publish_text(&app, "P", "x").await;
    let r = app.post_json("/api/schedules", &always(("group_id", &g["id"]), v)).await;
    assert_eq!(r.code, 201, "{}", r.text());
    assert_eq!(r.json()["group_id"], g["id"]);
    assert!(r.json()["screen_id"].is_null());
    assert_eq!((r.json()["start"].as_str(), r.json()["end"].as_str()), (Some("00:00"), Some("24:00")));
}

#[tokio::test]
async fn schedule_validation() {
    let app = TestApp::new().await;
    let s = new_screen(&app, "S").await;
    let g = new_group(&app, "G").await;
    let v = publish_text(&app, "P", "x").await;
    let ok = json!({"screen_id": s["id"], "version_id": v, "days": [0], "start": "08:00", "end": "09:00"});
    assert_eq!(app.post_json("/api/schedules", &ok).await.code, 201);
    let mutate = |f: &dyn Fn(&mut Value)| {
        let mut b = ok.clone();
        f(&mut b);
        b
    };
    let cases = [
        ("deux cibles", mutate(&|b| b["group_id"] = g["id"].clone())),
        ("aucune cible", mutate(&|b| b["screen_id"] = Value::Null)),
        ("fin avant début", mutate(&|b| { b["start"] = json!("18:00"); b["end"] = json!("08:00"); })),
        ("plage vide", mutate(&|b| { b["start"] = json!("08:00"); b["end"] = json!("08:00"); })),
        ("heure invalide", mutate(&|b| b["start"] = json!("8h"))),
        ("fin > 24:00", mutate(&|b| b["end"] = json!("24:30"))),
        ("jours vides", mutate(&|b| b["days"] = json!([]))),
        ("jour 7", mutate(&|b| b["days"] = json!([7]))),
        ("version 0", mutate(&|b| b["version_id"] = json!(0))),
        ("écran 0", mutate(&|b| b["screen_id"] = json!(0))),
    ];
    for (label, body) in cases {
        let r = app.post_json("/api/schedules", &body).await;
        assert_eq!(r.code, 422, "{label} : {}", r.text());
        assert!(r.json()["detail"][0]["msg"].is_string(), "{label}");
    }
    // champ obligatoire manquant
    assert_eq!(app.post_json("/api/schedules", &json!({"screen_id": s["id"]})).await.code, 422);
}

#[tokio::test]
async fn schedule_targets_must_exist() {
    let app = TestApp::new().await;
    let s = new_screen(&app, "S").await;
    let v = publish_text(&app, "P", "x").await;
    let base = |target: Value| {
        let mut b = json!({"version_id": v, "days": [0], "start": "08:00", "end": "09:00"});
        for (k, val) in target.as_object().unwrap() {
            b[k] = val.clone();
        }
        b
    };
    let r = app.post_json("/api/schedules", &base(json!({"screen_id": 999}))).await;
    assert_eq!((r.code, r.detail()), (404, "Écran introuvable".to_string()));
    let r = app.post_json("/api/schedules", &base(json!({"group_id": 999}))).await;
    assert_eq!((r.code, r.detail()), (404, "Groupe introuvable".to_string()));
    let r = app.post_json("/api/schedules", &json!({"screen_id": s["id"], "version_id": 999, "days": [0], "start": "08:00", "end": "09:00"})).await;
    assert_eq!((r.code, r.detail()), (404, "Version introuvable".to_string()));
}

#[tokio::test]
async fn the_number_of_rules_per_target_is_capped() {
    let app = TestApp::new().await;
    let s = new_screen(&app, "S").await;
    let other = new_screen(&app, "Autre").await;
    let v = publish_text(&app, "P", "x").await;
    let rule = |sid: &Value| json!({"screen_id": sid, "version_id": v, "days": [0], "start": "08:00", "end": "09:00"});
    for _ in 0..100 {
        assert_eq!(app.post_json("/api/schedules", &rule(&s["id"])).await.code, 201);
    }
    let r = app.post_json("/api/schedules", &rule(&s["id"])).await;
    assert_eq!(r.code, 409, "{}", r.text());
    assert!(r.detail().contains("100"));
    assert_eq!(app.post_json("/api/schedules", &rule(&other["id"])).await.code, 201, "le plafond est par cible");
}

#[tokio::test]
async fn viewers_can_read_the_planning_but_not_change_it() {
    let app = TestApp::new().await;
    let s = new_screen(&app, "S").await;
    let v = publish_text(&app, "P", "x").await;
    let rule = app.post_json("/api/schedules", &always(("screen_id", &s["id"]), v)).await.json();
    let viewer = session_for(&app, "vee", "viewer").await;
    assert_eq!(app.get_as(&viewer, "/api/schedules").await.code, 200);
    let r = app.req(Method::POST, "/api/schedules").bearer(&viewer).json(&always(("screen_id", &s["id"]), v)).send().await;
    assert_eq!(r.code, 403);
    let r = app.req(Method::DELETE, &format!("/api/schedules/{}", rule["id"])).bearer(&viewer).send().await;
    assert_eq!(r.code, 403);
    let mut anon = TestApp::new().await;
    anon.auth = None;
    assert_eq!(anon.get("/api/schedules").await.code, 401);
}

// =====================================================================================
// Effets sur le contenu
// =====================================================================================
#[tokio::test]
async fn an_active_screen_rule_overrides_fixed_and_group_content() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let (vg, vs, vr) = (
        publish_text(&app, "G", "GROUPE").await,
        publish_text(&app, "S", "FIXE").await,
        publish_text(&app, "R", "PLANNING").await,
    );
    app.post_json(&format!("/api/groups/{}/assign", g["id"]), &json!({"version_id": vg})).await;
    let s = app.post_json("/api/screens", &json!({"name": "S", "group_id": g["id"]})).await.json();
    let tok = s["token"].as_str().unwrap();
    app.post_json(&format!("/api/screens/{}/assign", s["id"]), &json!({"version_id": vs})).await;
    assert_eq!(shown_text(&app, tok).await.as_deref(), Some("FIXE"));

    let rule = app.post_json("/api/schedules", &always(("screen_id", &s["id"]), vr)).await.json();
    assert_eq!(shown_text(&app, tok).await.as_deref(), Some("PLANNING"));
    let out = screen_by_id(&app, &s["id"]).await;
    assert_eq!((out["effective_version_id"].clone(), out["effective_source"].clone()), (json!(vr), json!("screen_schedule")));
    assert_eq!(out["version_id"], vs, "le contenu fixé reste affiché à part");

    // supprimer la règle : retour au contenu fixé
    app.delete(&format!("/api/schedules/{}", rule["id"])).await;
    assert_eq!(shown_text(&app, tok).await.as_deref(), Some("FIXE"));
}

#[tokio::test]
async fn an_inactive_rule_changes_nothing() {
    let app = TestApp::new().await;
    let (vs, vr) = (publish_text(&app, "S", "FIXE").await, publish_text(&app, "R", "PLANNING").await);
    let s = new_screen(&app, "S").await;
    app.post_json(&format!("/api/screens/{}/assign", s["id"]), &json!({"version_id": vs})).await;
    let r = app.post_json("/api/schedules", &never(("screen_id", &s["id"]), vr)).await;
    assert_eq!(r.code, 201);
    assert_eq!(shown_text(&app, s["token"].as_str().unwrap()).await.as_deref(), Some("FIXE"));
    assert_eq!(screen_by_id(&app, &s["id"]).await["effective_source"], "screen");
}

#[tokio::test]
async fn a_screen_with_only_an_active_rule_shows_it_and_nothing_otherwise() {
    let app = TestApp::new().await;
    let v = publish_text(&app, "R", "PLANNING").await;
    let s = new_screen(&app, "S").await;
    let tok = s["token"].as_str().unwrap();
    assert_eq!(shown_text(&app, tok).await, None);
    app.post_json("/api/schedules", &always(("screen_id", &s["id"]), v)).await;
    assert_eq!(shown_text(&app, tok).await.as_deref(), Some("PLANNING"));
}

#[tokio::test]
async fn a_group_rule_applies_to_screens_without_their_own_content() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let (vg, vr, vs) = (
        publish_text(&app, "G", "GROUPE").await,
        publish_text(&app, "R", "PLANNING-GROUPE").await,
        publish_text(&app, "S", "FIXE").await,
    );
    app.post_json(&format!("/api/groups/{}/assign", g["id"]), &json!({"version_id": vg})).await;
    let free = app.post_json("/api/screens", &json!({"name": "Libre", "group_id": g["id"]})).await.json();
    let fixed = app.post_json("/api/screens", &json!({"name": "Fixé", "group_id": g["id"]})).await.json();
    app.post_json(&format!("/api/screens/{}/assign", fixed["id"]), &json!({"version_id": vs})).await;
    app.post_json("/api/schedules", &always(("group_id", &g["id"]), vr)).await;

    assert_eq!(shown_text(&app, free["token"].as_str().unwrap()).await.as_deref(), Some("PLANNING-GROUPE"));
    assert_eq!(screen_by_id(&app, &free["id"]).await["effective_source"], "group_schedule");
    // l'écran qui a son contenu fixé ne suit pas le planning du groupe
    assert_eq!(shown_text(&app, fixed["token"].as_str().unwrap()).await.as_deref(), Some("FIXE"));
    assert_eq!(screen_by_id(&app, &fixed["id"]).await["effective_source"], "screen");
}

#[tokio::test]
async fn a_group_rule_falls_back_to_the_group_content_when_inactive() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let (vg, vr) = (publish_text(&app, "G", "GROUPE").await, publish_text(&app, "R", "PLANNING").await);
    app.post_json(&format!("/api/groups/{}/assign", g["id"]), &json!({"version_id": vg})).await;
    app.post_json("/api/schedules", &never(("group_id", &g["id"]), vr)).await;
    let s = app.post_json("/api/screens", &json!({"name": "S", "group_id": g["id"]})).await.json();
    assert_eq!(shown_text(&app, s["token"].as_str().unwrap()).await.as_deref(), Some("GROUPE"));
}

#[tokio::test]
async fn rules_of_a_group_do_not_apply_to_screens_outside_it() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let v = publish_text(&app, "R", "PLANNING").await;
    app.post_json("/api/schedules", &always(("group_id", &g["id"]), v)).await;
    let outsider = new_screen(&app, "Isolé").await;
    assert_eq!(shown_text(&app, outsider["token"].as_str().unwrap()).await, None);
}

#[tokio::test]
async fn the_most_recent_active_rule_wins() {
    let app = TestApp::new().await;
    let (v1, v2) = (publish_text(&app, "A", "ANCIENNE").await, publish_text(&app, "B", "RÉCENTE").await);
    let s = new_screen(&app, "S").await;
    app.post_json("/api/schedules", &always(("screen_id", &s["id"]), v1)).await;
    app.post_json("/api/schedules", &always(("screen_id", &s["id"]), v2)).await;
    assert_eq!(shown_text(&app, s["token"].as_str().unwrap()).await.as_deref(), Some("RÉCENTE"));
}

// =====================================================================================
// Nettoyage et intégrité
// =====================================================================================
#[tokio::test]
async fn deleting_a_screen_or_a_group_removes_their_rules() {
    let app = TestApp::new().await;
    let g = new_group(&app, "G").await;
    let s = new_screen(&app, "S").await;
    let keep = new_screen(&app, "Garde").await;
    let v = publish_text(&app, "P", "x").await;
    app.post_json("/api/schedules", &always(("group_id", &g["id"]), v)).await;
    app.post_json("/api/schedules", &always(("screen_id", &s["id"]), v)).await;
    let kept = app.post_json("/api/schedules", &always(("screen_id", &keep["id"]), v)).await.json();
    assert_eq!(app.get("/api/schedules").await.json().as_array().unwrap().len(), 3);
    app.delete(&format!("/api/groups/{}", g["id"])).await;
    app.delete(&format!("/api/screens/{}", s["id"])).await;
    let left = app.get("/api/schedules").await.json();
    assert_eq!(left.as_array().unwrap().len(), 1);
    assert_eq!(left[0]["id"], kept["id"]);
}

#[tokio::test]
async fn a_playlist_used_in_a_schedule_cannot_be_deleted_and_its_media_stays_protected() {
    let app = TestApp::new().await;
    let (pid, img, _) = mk_playlist(&app).await;
    let v = app.post(&format!("/api/playlists/{pid}/publish")).await.json()["id"].as_i64().unwrap();
    let s = new_screen(&app, "S").await;
    let rule = app.post_json("/api/schedules", &always(("screen_id", &s["id"]), v)).await.json();
    let r = app.delete(&format!("/api/playlists/{pid}")).await;
    assert_eq!(r.code, 409);
    assert!(r.detail().contains("planning"), "{}", r.detail());
    assert_eq!(app.delete(&format!("/api/media/{}", img["id"])).await.code, 409);
    app.delete(&format!("/api/schedules/{}", rule["id"])).await;
    assert_eq!(app.delete(&format!("/api/playlists/{pid}")).await.code, 204);
}

#[tokio::test]
async fn planning_changes_are_audited() {
    let app = TestApp::new().await;
    let s = new_screen(&app, "S").await;
    let v = publish_text(&app, "P", "x").await;
    let rule = app.post_json("/api/schedules", &always(("screen_id", &s["id"]), v)).await.json();
    app.delete(&format!("/api/schedules/{}", rule["id"])).await;
    let entries = audit_entries(&app).await;
    let created = entries.iter().find(|e| e["action"] == "schedule.create").expect("création journalisée");
    assert_eq!(created["target"], format!("screen:{}", s["id"]));
    assert!(created["detail"].as_str().unwrap().contains("00:00-24:00"));
    assert!(actions(&entries).contains(&"schedule.delete".to_string()));
}

#[tokio::test]
async fn the_timezone_setting_is_validated_at_startup() {
    use ttscreen::{build_state, Settings};
    for (tz, ok) in [
        ("Europe/Paris", true),
        ("UTC", true),
        ("America/Toronto", true),
        ("Pacific/Kiritimati", true),
        ("Mars/Phobos", false),
        ("Paris", false),
        ("", false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings { timezone: tz.to_string(), ..base_settings(dir.path()) };
        assert_eq!(build_state(s).await.is_ok(), ok, "{tz:?}");
    }
}
