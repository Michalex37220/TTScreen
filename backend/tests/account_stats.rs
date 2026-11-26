//! Changement de mot de passe, tableau de bord, duplication de playlist.

mod common;

use axum::http::Method;
use common::*;
use serde_json::json;

// =====================================================================================
// Changer son propre mot de passe
// =====================================================================================
async fn change(app: &TestApp, token: &str, current: &str, new: &str) -> Resp {
    app.req(Method::POST, "/api/auth/password")
        .bearer(token)
        .json(&json!({"current_password": current, "new_password": new}))
        .send()
        .await
}

#[tokio::test]
async fn a_user_can_change_their_own_password() {
    let app = TestApp::new().await;
    let tok = session_for(&app, "emma", "viewer").await; // viewer : aucun droit d'écriture requis
    let r = change(&app, &tok, "motdepasse8", "nouveau-secret-9").await;
    assert_eq!(r.code, 204, "{}", r.text());
    assert_eq!(login(&app, "emma", "motdepasse8").await.code, 401, "l'ancien mot de passe ne marche plus");
    assert_eq!(login(&app, "emma", "nouveau-secret-9").await.code, 200);
    assert_eq!(app.get_as(&tok, "/api/auth/me").await.code, 200, "la session courante reste valide");
    assert!(actions(&audit_entries(&app).await).contains(&"user.password".to_string()));
}

#[tokio::test]
async fn changing_the_password_revokes_the_other_sessions_only() {
    let app = TestApp::new().await;
    make_user(&app, "emma", "motdepasse8", "editor").await;
    let first = login(&app, "emma", "motdepasse8").await.json()["token"].as_str().unwrap().to_string();
    let second = login(&app, "emma", "motdepasse8").await.json()["token"].as_str().unwrap().to_string();
    assert_ne!(first, second);
    assert_eq!(change(&app, &first, "motdepasse8", "nouveau-secret-9").await.code, 204);
    assert_eq!(app.get_as(&first, "/api/auth/me").await.code, 200, "session qui a fait le changement");
    assert_eq!(app.get_as(&second, "/api/auth/me").await.code, 401, "autre session révoquée");
}

#[tokio::test]
async fn a_wrong_current_password_is_refused_and_audited() {
    let app = TestApp::new().await;
    let tok = session_for(&app, "emma", "viewer").await;
    let r = change(&app, &tok, "pas-le-bon-mdp", "nouveau-secret-9").await;
    assert_eq!(r.code, 403);
    assert_eq!(r.detail(), "Mot de passe actuel incorrect");
    assert_eq!(login(&app, "emma", "motdepasse8").await.code, 200, "rien n'a changé");
    assert!(actions(&audit_entries(&app).await).contains(&"mot_de_passe_refuse".to_string()));
}

#[tokio::test]
async fn password_change_validation_and_authentication() {
    let app = TestApp::new().await;
    let tok = session_for(&app, "emma", "viewer").await;
    assert_eq!(change(&app, &tok, "motdepasse8", "court").await.code, 422, "nouveau mot de passe < 8");
    assert_eq!(change(&app, &tok, "", "nouveau-secret-9").await.code, 422);
    let r = app.req(Method::POST, "/api/auth/password").bearer(&tok).json(&json!({"new_password": "nouveau-secret-9"})).send().await;
    assert_eq!(r.code, 422, "mot de passe actuel obligatoire");
    let anon = app.req(Method::POST, "/api/auth/password").no_auth().json(&json!({"current_password": "a", "new_password": "nouveau-secret-9"})).send().await;
    assert_eq!(anon.code, 401);
}

#[tokio::test]
async fn the_legacy_admin_token_has_no_password_to_change() {
    let app = TestApp::new().await;
    let r = change(&app, TEST_TOKEN, "x", "nouveau-secret-9").await;
    assert_eq!(r.code, 400);
    assert!(r.detail().contains("nommé"));
}

#[tokio::test]
async fn guessing_the_current_password_with_a_stolen_session_is_throttled() {
    let app = TestApp::with(|s| s.rate_limit_login_user = 3).await;
    let tok = session_for(&app, "emma", "viewer").await;
    let mut codes = Vec::new();
    for i in 0..4 {
        codes.push(change(&app, &tok, &format!("essai-{i}-xxxx"), "nouveau-secret-9").await.code);
    }
    assert_eq!(codes, [403, 403, 403, 429]);
    // même le bon mot de passe est bloqué pendant la fenêtre
    assert_eq!(change(&app, &tok, "motdepasse8", "nouveau-secret-9").await.code, 429);
}

// =====================================================================================
// Tableau de bord
// =====================================================================================
#[tokio::test]
async fn stats_on_an_empty_installation() {
    let app = TestApp::new().await;
    let r = app.get("/api/stats").await;
    assert_eq!(r.code, 200);
    assert_eq!(
        r.json(),
        json!({
            "screens": {"total": 0, "online": 0, "offline": 0, "without_content": 0},
            "groups": 0, "playlists": 0, "versions": 0, "schedules": 0,
            "media": {"count": 0, "bytes": 0}
        })
    );
}

#[tokio::test]
async fn stats_count_everything() {
    let app = TestApp::new().await;
    let (pid, img, vid) = mk_playlist(&app).await;
    let v = app.post(&format!("/api/playlists/{pid}/publish")).await.json()["id"].clone();
    let g = new_group(&app, "Hall").await;
    let online = new_screen(&app, "En ligne").await;
    let _idle = new_screen(&app, "Silencieux").await;
    app.post_json(&format!("/api/screens/{}/assign", online["id"]), &json!({"version_id": v})).await;
    app.post_json(&format!("/api/groups/{}/assign", g["id"]), &json!({"version_id": v})).await;
    app.get_as(online["token"].as_str().unwrap(), "/api/player/manifest").await; // devient « en ligne »
    let s = app.get("/api/stats").await.json();
    assert_eq!(s["screens"], json!({"total": 2, "online": 1, "offline": 1, "without_content": 1}));
    assert_eq!((s["groups"].clone(), s["playlists"].clone(), s["versions"].clone()), (json!(1), json!(1), json!(1)));
    assert_eq!(s["media"]["count"], 2);
    let expected = img["size"].as_i64().unwrap() + vid["size"].as_i64().unwrap();
    assert_eq!(s["media"]["bytes"], expected);
}

#[tokio::test]
async fn stats_count_content_inherited_from_a_group_or_a_schedule() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let v = publish_text(&app, "P", "x").await;
    let in_group = app.post_json("/api/screens", &json!({"name": "G", "group_id": g["id"]})).await.json();
    let scheduled = new_screen(&app, "Planifié").await;
    let _bare = new_screen(&app, "Nu").await;
    assert_eq!(app.get("/api/stats").await.json()["screens"]["without_content"], 3);
    app.post_json(&format!("/api/groups/{}/assign", g["id"]), &json!({"version_id": v})).await;
    assert_eq!(app.get("/api/stats").await.json()["screens"]["without_content"], 2);
    let rule = json!({"screen_id": scheduled["id"], "version_id": v, "days": [0,1,2,3,4,5,6], "start": "00:00", "end": "24:00"});
    app.post_json("/api/schedules", &rule).await;
    let s = app.get("/api/stats").await.json();
    assert_eq!(s["screens"]["without_content"], 1, "{s}");
    assert_eq!(s["schedules"], 1);
    let _ = in_group;
}

#[tokio::test]
async fn stats_are_readable_by_viewers_only_when_authenticated() {
    let app = TestApp::new().await;
    let viewer = session_for(&app, "vee", "viewer").await;
    assert_eq!(app.get_as(&viewer, "/api/stats").await.code, 200);
    let mut anon = TestApp::new().await;
    anon.auth = None;
    assert_eq!(anon.get("/api/stats").await.code, 401);
}

// =====================================================================================
// Dupliquer une playlist
// =====================================================================================
#[tokio::test]
async fn duplicating_a_playlist_copies_its_items_but_not_its_versions() {
    let app = TestApp::new().await;
    let (pid, img, vid) = mk_playlist(&app).await;
    app.post(&format!("/api/playlists/{pid}/publish")).await;
    let r = app.post(&format!("/api/playlists/{pid}/duplicate")).await;
    assert_eq!(r.code, 201, "{}", r.text());
    let copy = r.json();
    assert_ne!(copy["id"].as_i64(), Some(pid));
    assert_eq!(copy["name"], "Copie de Hall");
    let original = app.get(&format!("/api/playlists/{pid}")).await.json();
    let (a, b) = (original["items"].as_array().unwrap(), copy["items"].as_array().unwrap());
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b) {
        for key in ["type", "text", "media_id", "duration_s", "position"] {
            assert_eq!(x[key], y[key], "{key}");
        }
        assert_ne!(x["id"], y["id"], "de nouveaux éléments");
    }
    assert_eq!(b[1]["media_id"], img["id"]);
    assert_eq!(b[2]["media_id"], vid["id"]);
    // aucune version copiée ; l'original est intact
    assert_eq!(app.get(&format!("/api/playlists/{}/versions", copy["id"])).await.json(), json!([]));
    assert_eq!(app.get(&format!("/api/playlists/{pid}/versions")).await.json().as_array().unwrap().len(), 1);
    assert_eq!(app.get("/api/playlists").await.json().as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn a_copy_is_independent_from_the_original() {
    let app = TestApp::new().await;
    let (pid, _, _) = mk_playlist(&app).await;
    let copy = app.post(&format!("/api/playlists/{pid}/duplicate")).await.json();
    let cid = copy["id"].as_i64().unwrap();
    app.put_json(&format!("/api/playlists/{cid}/items"), &json!([{"type": "text", "text": "seul"}])).await;
    assert_eq!(app.get(&format!("/api/playlists/{pid}")).await.json()["items"].as_array().unwrap().len(), 3);
    assert_eq!(app.get(&format!("/api/playlists/{cid}")).await.json()["items"].as_array().unwrap().len(), 1);
    // la copie peut être publiée (et son média reste protégé par les deux playlists)
    assert_eq!(app.delete(&format!("/api/playlists/{cid}")).await.code, 204);
}

#[tokio::test]
async fn duplicate_names_are_truncated_to_120_characters_and_empty_playlists_copy_fine() {
    let app = TestApp::new().await;
    let long = app.post_json("/api/playlists", &json!({"name": "x".repeat(120)})).await.json();
    let copy = app.post(&format!("/api/playlists/{}/duplicate", long["id"])).await.json();
    assert_eq!(copy["name"].as_str().unwrap().chars().count(), 120);
    assert!(copy["name"].as_str().unwrap().starts_with("Copie de x"));
    assert_eq!(copy["items"], json!([]));
}

#[tokio::test]
async fn duplicate_requires_editor_rights_and_an_existing_playlist() {
    let app = TestApp::new().await;
    let (pid, _, _) = mk_playlist(&app).await;
    assert_eq!(app.post("/api/playlists/999/duplicate").await.code, 404);
    let viewer = session_for(&app, "vee", "viewer").await;
    let r = app.req(Method::POST, &format!("/api/playlists/{pid}/duplicate")).bearer(&viewer).send().await;
    assert_eq!(r.code, 403);
    assert!(actions(&audit_entries(&app).await).iter().all(|a| a != "playlist.duplicate"));
    app.post(&format!("/api/playlists/{pid}/duplicate")).await;
    assert!(actions(&audit_entries(&app).await).contains(&"playlist.duplicate".to_string()));
}
