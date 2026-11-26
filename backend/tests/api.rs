//! Port de `backend/tests/test_api.py` : parcours complets de l'API.

mod common;

use axum::http::Method;
use common::*;
use serde_json::{json, Value};
use ttscreen::{build_state, Settings};

async fn list_users(app: &TestApp) -> Vec<Value> {
    app.get("/api/users").await.json().as_array().unwrap().clone()
}

#[tokio::test]
async fn admin_requires_auth() {
    let mut app = TestApp::new().await;
    app.auth = None;
    assert_eq!(app.get("/api/playlists").await.code, 401);
    assert_eq!(app.get("/api/health").await.code, 200);
    assert_eq!(app.req(Method::GET, "/api/playlists").bearer("wrong").send().await.code, 401);
}

#[tokio::test]
async fn upload_validation() {
    let app = TestApp::new().await;
    assert_eq!(app.upload("x.png", b"pas une image").await.code, 415);
    assert!([400, 415].contains(&app.upload("vide.png", b"").await.code));
    let mut big = png();
    big.extend(vec![0u8; 2 * 1024 * 1024]);
    assert_eq!(app.upload("gros.png", &big).await.code, 413);
    let ok = app.upload("a.png", &png()).await;
    assert_eq!(ok.code, 200);
    assert_eq!(ok.json()["kind"], "image");
    // dédoublonnage : même contenu, même média
    assert_eq!(app.upload("copie.png", &png()).await.json()["id"], ok.json()["id"]);
    let file = app.get(ok.json()["url"].as_str().unwrap()).await;
    assert_eq!(file.body.to_vec(), png());
}

#[tokio::test]
async fn item_validation() {
    let app = TestApp::new().await;
    let pid = app.post_json("/api/playlists", &json!({"name": "P"})).await.json()["id"].as_i64().unwrap();
    let img = app.upload("a.png", &png()).await.json();
    let put = |body: Value| {
        let app = &app;
        async move { app.put_json(&format!("/api/playlists/{pid}/items"), &body).await.code }
    };
    assert_eq!(put(json!([{"type": "text", "text": "  "}])).await, 422);
    assert_eq!(put(json!([{"type": "image", "media_id": 999}])).await, 422);
    assert_eq!(put(json!([{"type": "video", "media_id": img["id"]}])).await, 422); // mauvais type
    assert_eq!(put(json!([{"type": "text", "text": "ok", "duration_s": 0}])).await, 422);
    assert_eq!(put(json!([{"type": "image", "media_id": img["id"]}])).await, 200);
}

#[tokio::test]
async fn publish_is_immutable() {
    let app = TestApp::new().await;
    let (pid, img, _) = mk_playlist(&app).await;
    let v1 = app.post(&format!("/api/playlists/{pid}/publish")).await.json();
    assert_eq!(v1["number"], 1);
    assert_eq!(v1["manifest"]["items"].as_array().unwrap().len(), 3);
    app.put_json(&format!("/api/playlists/{pid}/items"), &json!([{"type": "text", "text": "Autre"}])).await;
    let v2 = app.post(&format!("/api/playlists/{pid}/publish")).await.json();
    assert_eq!(v2["number"], 2);
    let old = app.get(&format!("/api/versions/{}", v1["id"])).await.json();
    assert_eq!(old["items"].as_array().unwrap().len(), 3, "la v1 n'a pas bougé");
    assert_eq!(old["items"][1]["media"]["sha256"], img["sha256"]);
}

#[tokio::test]
async fn publish_empty_refused() {
    let app = TestApp::new().await;
    let pid = app.post_json("/api/playlists", &json!({"name": "Vide"})).await.json()["id"].as_i64().unwrap();
    let r = app.post(&format!("/api/playlists/{pid}/publish")).await;
    assert_eq!(r.code, 422);
    assert_eq!(r.detail(), "Playlist vide");
}

#[tokio::test]
async fn screen_flow_and_rollback() {
    let app = TestApp::new().await;
    let (pid, _, _) = mk_playlist(&app).await;
    let v1 = app.post(&format!("/api/playlists/{pid}/publish")).await.json();
    app.put_json(&format!("/api/playlists/{pid}/items"), &json!([{"type": "text", "text": "V2"}])).await;
    let v2 = app.post(&format!("/api/playlists/{pid}/publish")).await.json();
    let sc = app.post_json("/api/screens", &json!({"name": "Entrée"})).await.json();
    let tok = sc["token"].as_str().unwrap().to_string();
    let manifest = || app.req(Method::GET, "/api/player/manifest").bearer(&tok).send();

    assert_eq!(manifest().await.code, 404, "rien d'affecté");
    app.post_json(&format!("/api/screens/{}/assign", sc["id"]), &json!({"version_id": v2["id"]})).await;
    assert_eq!(manifest().await.json()["number"], 2);
    app.post_json(&format!("/api/screens/{}/assign", sc["id"]), &json!({"version_id": v1["id"]})).await; // retour arrière
    assert_eq!(manifest().await.json()["number"], 1);
    assert_eq!(app.get("/api/screens").await.json()[0]["online"], true);
}

#[tokio::test]
async fn screen_token_rotation_and_revocation() {
    let app = TestApp::new().await;
    let sc = app.post_json("/api/screens", &json!({"name": "S"})).await.json();
    let old = sc["token"].as_str().unwrap().to_string();
    let rotated = app.post(&format!("/api/screens/{}/rotate-token", sc["id"])).await.json();
    let new = rotated["token"].as_str().unwrap().to_string();
    let manifest = |t: String| {
        let app = &app;
        async move { app.req(Method::GET, "/api/player/manifest").bearer(&t).send().await.code }
    };
    assert_eq!(manifest(old).await, 401);
    assert_eq!(manifest(new.clone()).await, 404, "authentifié, pas de version");
    app.post(&format!("/api/screens/{}/revoke-token", sc["id"])).await;
    assert_eq!(manifest(new.clone()).await, 401);
    // un jeton d'écran ne donne aucun accès admin
    assert_eq!(app.get_as(&new, "/api/playlists").await.code, 401);
}

#[tokio::test]
async fn media_delete_protection() {
    let app = TestApp::new().await;
    let (pid, img, _) = mk_playlist(&app).await;
    let url = format!("/api/media/{}", img["id"]);
    assert_eq!(app.delete(&url).await.code, 409);
    app.post(&format!("/api/playlists/{pid}/publish")).await;
    app.put_json(&format!("/api/playlists/{pid}/items"), &json!([{"type": "text", "text": "x"}])).await;
    let r = app.delete(&url).await;
    assert_eq!(r.code, 409, "encore dans la v1");
    assert_eq!(r.detail(), "Média utilisé dans une version publiée");
}

#[tokio::test]
async fn screen_preview_manifest() {
    let mut app = TestApp::new().await;
    let (pid, _, _) = mk_playlist(&app).await;
    let v = app.post(&format!("/api/playlists/{pid}/publish")).await.json();
    let sc = app.post_json("/api/screens", &json!({"name": "S"})).await.json();
    let url = format!("/api/screens/{}/manifest", sc["id"]);
    assert_eq!(app.get(&url).await.code, 404);
    app.post_json(&format!("/api/screens/{}/assign", sc["id"]), &json!({"version_id": v["id"]})).await;
    assert_eq!(app.get(&url).await.json()["number"], 1);
    // la prévisualisation ne simule pas une connexion
    assert_eq!(app.get("/api/screens").await.json()[0]["online"], false);
    app.auth = None;
    assert_eq!(app.get(&url).await.code, 401);
}

#[tokio::test]
async fn player_page_served() {
    let app = TestApp::new().await;
    let r = app.get("/player/").await;
    assert_eq!(r.code, 200);
    assert!(r.text().contains("player.js"));
}

#[tokio::test]
async fn draft_preview_and_admin_page() {
    let mut app = TestApp::new().await;
    let (pid, _, _) = mk_playlist(&app).await;
    let d = app.get(&format!("/api/playlists/{pid}/preview")).await.json();
    assert!(d["version_id"].is_null());
    assert!(d["number"].is_null());
    assert_eq!(d["items"].as_array().unwrap().len(), 3);
    assert_eq!(app.get(&format!("/api/playlists/{pid}/versions")).await.json(), json!([]));
    assert!(app.get("/admin/").await.text().contains("app.js"));
    app.auth = None;
    assert_eq!(app.get(&format!("/api/playlists/{pid}/preview")).await.code, 401);
}

#[tokio::test]
async fn production_refuses_weak_admin_token() {
    let try_token = |token: &'static str| async move {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings {
            env: "production".into(),
            admin_token: token.into(),
            ..base_settings(dir.path())
        };
        build_state(s).await.is_ok()
    };
    assert!(!try_token("dev-admin-token").await);
    assert!(!try_token("court").await);
    assert!(!try_token("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx").await, "assez long mais sans variété");
    assert!(try_token("Zq8vN3xK1mP7wR5tY2uL9aB4").await, "accepté");
}

// ---------- comptes, rôles et sessions ----------
#[tokio::test]
async fn login_logout_and_me() {
    let app = TestApp::new().await;
    assert_eq!(make_user(&app, "emma", "motdepasse8", "editor").await.code, 201);
    assert_eq!(login(&app, "emma", "mauvais123").await.code, 401);
    let r = login(&app, "emma", "motdepasse8").await;
    assert_eq!(r.code, 200);
    assert_eq!(r.json()["role"], "editor");
    let tok = r.json()["token"].as_str().unwrap().to_string();
    let me = app.get_as(&tok, "/api/auth/me").await.json();
    assert_eq!(me, json!({"username": "emma", "role": "editor", "legacy": false}));
    // la session ouvre les routes d'administration (lecture), mais pas les routes admin
    assert_eq!(app.get_as(&tok, "/api/playlists").await.code, 200);
    assert_eq!(app.get_as(&tok, "/api/users").await.code, 403);
    // déconnexion : le jeton cesse immédiatement de fonctionner
    assert_eq!(app.req(Method::POST, "/api/auth/logout").bearer(&tok).send().await.code, 204);
    assert_eq!(app.get_as(&tok, "/api/auth/me").await.code, 401);
    // le jeton admin historique reste accepté en login (mode token), rôle admin, sans expiration
    let r = app.req(Method::POST, "/api/auth/login").json(&json!({"token": TEST_TOKEN})).send().await;
    assert_eq!(r.code, 200);
    assert_eq!(r.json()["role"], "admin");
    assert!(r.json()["expires_at"].is_null());
    let me = app.get_as(r.json()["token"].as_str().unwrap(), "/api/auth/me").await.json();
    assert_eq!(me["legacy"], true);
    let bad = app.req(Method::POST, "/api/auth/login").json(&json!({"token": "faux-jeton"})).send().await;
    assert_eq!(bad.code, 401);
}

#[tokio::test]
async fn roles_enforcement() {
    let app = TestApp::new().await;
    let viewer = session_for(&app, "lea", "viewer").await;
    let editor = session_for(&app, "yan", "editor").await;
    let post = |tok: &str, path: String, body: Value| {
        let app = &app;
        let tok = tok.to_string();
        async move { app.req(Method::POST, &path).bearer(&tok).json(&body).send().await }
    };
    // viewer : lecture autorisée, écriture refusée (403 et pas 401 : il est bien authentifié)
    assert_eq!(app.get_as(&viewer, "/api/media").await.code, 200);
    assert_eq!(post(&viewer, "/api/playlists".into(), json!({"name": "x"})).await.code, 403);
    assert_eq!(app.get_as(&viewer, "/api/audit").await.code, 403);
    // editor : peut créer une playlist et publier (422 ici car vide, pas 403), mais pas gérer les comptes
    let pid = post(&editor, "/api/playlists".into(), json!({"name": "P"})).await.json()["id"].as_i64().unwrap();
    let publish = app.req(Method::POST, &format!("/api/playlists/{pid}/publish")).bearer(&editor).send().await;
    assert_eq!(publish.code, 422);
    assert_eq!(app.get_as(&editor, "/api/users").await.code, 403);
    assert_eq!(
        post(&editor, "/api/users".into(), json!({"username": "zed", "password": "motdepasse8"})).await.code,
        403
    );
    // un jeton d'écran n'a toujours aucun accès aux routes d'administration
    let sc = app.post_json("/api/screens", &json!({"name": "S"})).await.json();
    assert_eq!(app.get_as(sc["token"].as_str().unwrap(), "/api/playlists").await.code, 401);
}

#[tokio::test]
async fn user_management_rules() {
    let app = TestApp::new().await;
    assert_eq!(make_user(&app, "emma", "motdepasse8", "admin").await.code, 201);
    assert_eq!(make_user(&app, "emma", "motdepasse8", "viewer").await.code, 409, "nom déjà pris");
    let short = app.post_json("/api/users", &json!({"username": "court", "password": "short"})).await;
    assert_eq!(short.code, 422, "mot de passe < 8");
    let find = |list: &[Value], name: &str| list.iter().find(|u| u["username"] == name).unwrap().clone();
    let emma = find(&list_users(&app).await, "emma");
    // le dernier administrateur actif ne peut ni être supprimé ni désactivé
    assert_eq!(app.delete(&format!("/api/users/{}", emma["id"])).await.code, 409);
    assert_eq!(app.patch_json(&format!("/api/users/{}", emma["id"]), &json!({"active": false})).await.code, 409);
    // un deuxième admin permet de modifier le premier ; la désactivation révoque ses sessions
    make_user(&app, "boss", "motdepasse8", "admin").await;
    let boss = find(&list_users(&app).await, "boss");
    let tok = login(&app, "emma", "motdepasse8").await.json()["token"].as_str().unwrap().to_string();
    assert_eq!(app.get_as(&tok, "/api/auth/me").await.code, 200);
    assert_eq!(app.patch_json(&format!("/api/users/{}", emma["id"]), &json!({"active": false})).await.code, 200);
    assert_eq!(app.get_as(&tok, "/api/auth/me").await.code, 401, "session révoquée");
    assert_eq!(login(&app, "emma", "motdepasse8").await.code, 401, "compte désactivé : plus de connexion");
    // on ne peut pas se supprimer soi-même (agissons en tant que boss via sa session)
    let boss_tok = login(&app, "boss", "motdepasse8").await.json()["token"].as_str().unwrap().to_string();
    let del = |tok: String, id: Value| {
        let app = &app;
        async move { app.req(Method::DELETE, &format!("/api/users/{id}")).bearer(&tok).send().await.code }
    };
    assert_eq!(del(boss_tok.clone(), boss["id"].clone()).await, 409);
    // mais boss peut supprimer emma
    assert_eq!(del(boss_tok, emma["id"].clone()).await, 204);
    // supprimer un compte non-admin : OK ; compte inconnu : 404
    make_user(&app, "temp", "motdepasse8", "viewer").await;
    let temp = find(&list_users(&app).await, "temp");
    assert_eq!(app.delete(&format!("/api/users/{}", temp["id"])).await.code, 204);
    assert_eq!(app.delete("/api/users/9999").await.code, 404);
}

// ---------- journal des opérations sensibles ----------
#[tokio::test]
async fn audit_log() {
    let app = TestApp::new().await;
    make_user(&app, "emma", "motdepasse8", "editor").await;
    login(&app, "emma", "motdepasse8").await; // connexion journalisée
    login(&app, "emma", "mauvais123").await; // échec journalisé aussi
    let (pid, _, _) = mk_playlist(&app).await;
    let v = app.post(&format!("/api/playlists/{pid}/publish")).await.json();
    let sc = app.post_json("/api/screens", &json!({"name": "Entrée"})).await.json();
    app.post_json(&format!("/api/screens/{}/assign", sc["id"]), &json!({"version_id": v["id"]})).await;
    app.post(&format!("/api/screens/{}/rotate-token", sc["id"])).await;
    let entries = audit_entries(&app).await;
    let got = actions(&entries);
    // chaque opération sensible est tracée, avec l'auteur et l'IP
    for expected in [
        "connexion", "connexion_echouee", "user.create", "media.upload", "playlist.create", "playlist.items",
        "playlist.publish", "screen.create", "screen.assign", "screen.rotate_token",
    ] {
        assert!(got.iter().any(|a| a == expected), "action manquante : {expected} (reçu : {got:?})");
    }
    let connexion = entries.iter().find(|e| e["action"] == "connexion").unwrap();
    assert_eq!(connexion["username"], "emma");
    assert_eq!(connexion["ip"], "127.0.0.1");
    let publish = entries.iter().find(|e| e["action"] == "playlist.publish").unwrap();
    assert_eq!(publish["detail"], format!("v{}", v["number"]));
    assert_eq!(publish["target"], format!("playlist:{pid}"));
    // filtre par action + pagination
    assert_eq!(app.get("/api/audit?action=connexion").await.json().as_array().unwrap().len(), 1);
    assert_eq!(app.get("/api/audit?limit=5&offset=0").await.json().as_array().unwrap().len(), 5);
    // le journal est trié du plus récent au plus ancien
    assert!(entries[0]["id"].as_i64() > entries[entries.len() - 1]["id"].as_i64());
}

// ---------- limitation de débit ----------
#[tokio::test]
async fn rate_limit() {
    // Limites très basses : 5 requêtes / minute sur /api (hors health) et 2 logins / minute
    let app = TestApp::with(|s| {
        s.rate_limit_api = 5;
        s.rate_limit_login = 2;
        s.rate_window_s = 60;
    })
    .await;
    let mut codes = Vec::new();
    for _ in 0..7 {
        codes.push(app.get("/api/playlists").await.code);
    }
    assert_eq!(codes, [200, 200, 200, 200, 200, 429, 429]);
    let r = app.get("/api/playlists").await;
    assert_eq!(r.code, 429);
    assert!(r.header("retry-after").is_some());
    assert_eq!(r.detail(), "Trop de requêtes, réessayez plus tard.");
    assert_eq!(app.get("/api/health").await.code, 200, "la sonde de santé n'est jamais limitée");
    // limite de connexion indépendante et plus stricte (anti force brute)
    let mut logins = Vec::new();
    for _ in 0..3 {
        logins.push(login(&app, "x", "y").await.code);
    }
    assert_eq!(logins, [401, 401, 429]);
}

#[tokio::test]
async fn rate_limit_player_per_token() {
    // Limite du manifeste comptée par jeton d'écran : deux écrans derrière la même IP ont chacun leur quota
    let app = TestApp::with(|s| {
        s.rate_limit_api = 1000;
        s.rate_limit_player = 3;
        s.rate_window_s = 60;
    })
    .await;
    let t1 = app.post_json("/api/screens", &json!({"name": "A"})).await.json()["token"].as_str().unwrap().to_string();
    let t2 = app.post_json("/api/screens", &json!({"name": "B"})).await.json()["token"].as_str().unwrap().to_string();
    let manifest = |t: &str| {
        let t = t.to_string();
        let app = &app;
        async move { app.req(Method::GET, "/api/player/manifest").bearer(&t).send().await.code }
    };
    for _ in 0..3 {
        assert_eq!(manifest(&t1).await, 404);
    }
    assert_eq!(manifest(&t1).await, 429, "quota de l'écran 1 épuisé");
    assert_eq!(manifest(&t2).await, 404, "l'écran 2 a encore son quota");
}
