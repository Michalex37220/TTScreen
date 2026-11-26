//! Tests par fonctionnalité, en plus de la reprise des tests Python : médias, fichiers, playlists,
//! publication, écrans, comptes, journal, sessions, pages statiques, démarrage et persistance.

mod common;

use axum::http::Method;
use chrono::Duration;
use common::*;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use ttscreen::models::user;
use ttscreen::{auth, build_state, Settings};

fn webp() -> Vec<u8> {
    let mut v = b"RIFF\x24\0\0\0WEBPVP8 ".to_vec();
    v.extend(vec![0u8; 32]);
    v
}
fn gif() -> Vec<u8> {
    let mut v = b"GIF89a".to_vec();
    v.extend(vec![0u8; 32]);
    v
}
fn jpeg() -> Vec<u8> {
    let mut v = b"\xff\xd8\xff\xe0\0\x10JFIF\0".to_vec();
    v.extend(vec![0u8; 32]);
    v
}
fn webm() -> Vec<u8> {
    let mut v = b"\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01".to_vec();
    v.extend(vec![0u8; 32]);
    v
}

// =====================================================================================
// Médias
// =====================================================================================
#[tokio::test]
async fn every_allowed_format_is_accepted_with_the_right_kind_and_mime() {
    let app = TestApp::new().await;
    for (name, data, kind, mime, ext) in [
        ("a.png", png(), "image", "image/png", "png"),
        ("b.jpg", jpeg(), "image", "image/jpeg", "jpg"),
        ("c.webp", webp(), "image", "image/webp", "webp"),
        ("d.gif", gif(), "image", "image/gif", "gif"),
        ("e.mp4", mp4(), "video", "video/mp4", "mp4"),
        ("f.webm", webm(), "video", "video/webm", "webm"),
    ] {
        let r = app.upload(name, &data).await;
        assert_eq!(r.code, 200, "{name}: {}", r.text());
        let m = r.json();
        assert_eq!((m["kind"].as_str(), m["mime"].as_str()), (Some(kind), Some(mime)), "{name}");
        assert!(m["url"].as_str().unwrap().ends_with(&format!(".{ext}")), "{name}");
        assert_eq!(m["original_name"], name);
        assert_eq!(m["size"], data.len());
        assert_eq!(m["sha256"].as_str().unwrap().len(), 64);
    }
}

#[tokio::test]
async fn the_real_type_wins_over_the_file_name_and_dangerous_content_is_refused() {
    let app = TestApp::new().await;
    // un PNG déguisé en .exe est accepté pour ce qu'il est…
    let r = app.upload("malin.exe", &png()).await;
    assert_eq!(r.code, 200);
    assert_eq!(r.json()["kind"], "image");
    assert!(r.json()["url"].as_str().unwrap().ends_with(".png"));
    // …et du contenu actif déguisé en image est refusé
    for (name, data) in [
        ("x.png", b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>".to_vec()),
        ("y.jpg", b"<?php system($_GET['c']); ?>".to_vec()),
        ("z.png", b"<html><script>alert(1)</script></html>".to_vec()),
        ("doc.png", b"%PDF-1.7 ...".to_vec()),
    ] {
        assert_eq!(app.upload(name, &data).await.code, 415, "{name}");
    }
}

#[tokio::test]
async fn duplicate_upload_is_stored_and_audited_once() {
    let app = TestApp::new().await;
    let a = app.upload("premier.png", &png()).await.json();
    let b = app.upload("second.png", &png()).await.json();
    assert_eq!(a["id"], b["id"]);
    assert_eq!(b["original_name"], "premier.png", "le média d'origine est conservé");
    assert_eq!(app.get("/api/media").await.json().as_array().unwrap().len(), 1);
    let uploads = actions(&audit_entries(&app).await).iter().filter(|a| *a == "media.upload").count();
    assert_eq!(uploads, 1);
    let stored: Vec<_> = std::fs::read_dir(app.media_dir()).unwrap().filter_map(|e| e.ok()).collect();
    assert_eq!(stored.iter().filter(|e| e.file_name().to_string_lossy().ends_with(".png")).count(), 1);
}

#[tokio::test]
async fn no_temporary_files_are_left_after_uploads_good_or_bad() {
    let app = TestApp::new().await;
    app.upload("ok.png", &png()).await;
    app.upload("ko.png", b"pas une image").await;
    let mut big = png();
    big.extend(vec![0u8; 2 * 1024 * 1024]);
    app.upload("gros.png", &big).await;
    let tmp = app.media_dir().join(".tmp");
    let leftovers = std::fs::read_dir(&tmp).map(|d| d.count()).unwrap_or(0);
    assert_eq!(leftovers, 0);
}

#[tokio::test]
async fn upload_without_content_length_is_still_capped() {
    // Corps « chunked » (pas de taille déclarée) : la limite s'applique pendant la lecture
    let app = TestApp::new().await;
    let mut big = png();
    big.extend(vec![0u8; 2 * 1024 * 1024]);
    let r = app.req(Method::POST, "/api/media").multipart("gros.png", &big).no_content_length().send().await;
    assert_eq!(r.code, 413);
    assert_eq!(app.get("/api/media").await.json().as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn absurd_declared_length_is_refused_without_reading() {
    let app = TestApp::new().await;
    let flag = Arc::new(AtomicBool::new(false));
    let r = app
        .req(Method::POST, "/api/media")
        .multipart("x.png", &png())
        .claim_length(10 * 1024 * 1024 * 1024)
        .track_body(flag.clone())
        .send()
        .await;
    assert_eq!(r.code, 413);
    assert!(!flag.load(Ordering::SeqCst));
}

#[tokio::test]
async fn upload_requires_the_file_field_and_multipart() {
    let app = TestApp::new().await;
    let r = app.req(Method::POST, "/api/media").multipart_field("autre", "a.png", &png()).send().await;
    assert_eq!(r.code, 422);
    let r = app.req(Method::POST, "/api/media").json(&json!({"file": "x"})).send().await;
    assert_eq!(r.code, 422);
    let r = app.post("/api/media").await;
    assert_eq!(r.code, 422);
}

#[tokio::test]
async fn nameless_upload_gets_a_default_name() {
    let app = TestApp::new().await;
    let r = app.upload("", &png()).await;
    assert_eq!(r.code, 200);
    assert_eq!(r.json()["original_name"], "fichier");
}

#[tokio::test]
async fn media_list_is_newest_first_and_has_the_documented_shape() {
    let app = TestApp::new().await;
    let a = app.upload("a.png", &png()).await.json();
    let b = app.upload("b.mp4", &mp4()).await.json();
    let list = app.get("/api/media").await.json();
    assert_eq!(list[0]["id"], b["id"]);
    assert_eq!(list[1]["id"], a["id"]);
    for key in ["id", "original_name", "kind", "mime", "size", "sha256", "url"] {
        assert!(list[0].get(key).is_some(), "{key}");
    }
    assert!(list[0].get("stored_name").is_none(), "le nom interne n'est pas exposé hors de l'URL");
}

#[tokio::test]
async fn deleting_a_media_removes_the_row_and_the_file() {
    let app = TestApp::new().await;
    let m = app.upload("a.png", &png()).await.json();
    let url = m["url"].as_str().unwrap().to_string();
    assert_eq!(app.get(&url).await.code, 200);
    assert_eq!(app.delete(&format!("/api/media/{}", m["id"])).await.code, 204);
    assert_eq!(app.get(&url).await.code, 404);
    assert_eq!(app.get("/api/media").await.json(), json!([]));
    assert_eq!(app.delete(&format!("/api/media/{}", m["id"])).await.code, 404);
    assert!(actions(&audit_entries(&app).await).contains(&"media.delete".to_string()));
}

#[tokio::test]
async fn viewers_cannot_upload_or_delete_media() {
    let app = TestApp::new().await;
    let m = app.upload("a.png", &png()).await.json();
    let viewer = session_for(&app, "vee", "viewer").await;
    let r = app.req(Method::POST, "/api/media").bearer(&viewer).multipart("b.png", &png()).send().await;
    assert_eq!(r.code, 403);
    let r = app.req(Method::DELETE, &format!("/api/media/{}", m["id"])).bearer(&viewer).send().await;
    assert_eq!(r.code, 403);
}

// =====================================================================================
// Service des fichiers
// =====================================================================================
#[tokio::test]
async fn files_are_public_typed_and_cacheable() {
    let mut app = TestApp::new().await;
    let m = app.upload("a.png", &png()).await.json();
    app.auth = None; // les écrans lisent les médias sans jeton
    let r = app.get(m["url"].as_str().unwrap()).await;
    assert_eq!(r.code, 200);
    assert_eq!(r.header("content-type").as_deref(), Some("image/png"));
    assert_eq!(r.header("x-content-type-options").as_deref(), Some("nosniff"));
    assert!(r.header("cache-control").unwrap().contains("immutable"));
    assert_eq!(r.body.to_vec(), png());
}

#[tokio::test]
async fn video_files_support_range_requests_for_seeking() {
    let app = TestApp::new().await;
    let data = mp4();
    let m = app.upload("v.mp4", &data).await.json();
    let r = app.req(Method::GET, m["url"].as_str().unwrap()).header("range", "bytes=0-9").send().await;
    assert_eq!(r.code, 206);
    assert_eq!(r.body.len(), 10);
    assert_eq!(r.body.to_vec(), data[..10].to_vec());
    assert!(r.header("content-range").unwrap().starts_with("bytes 0-9/"));
    assert_eq!(r.header("content-type").as_deref(), Some("video/mp4"));
}

// =====================================================================================
// Playlists
// =====================================================================================
#[tokio::test]
async fn playlist_crud_and_shapes() {
    let app = TestApp::new().await;
    let created = app.post_json("/api/playlists", &json!({"name": "Accueil"})).await;
    assert_eq!(created.code, 201);
    let p = created.json();
    assert_eq!(p["name"], "Accueil");
    assert_eq!(p["items"], json!([]));
    let pid = p["id"].as_i64().unwrap();
    // renommer
    let renamed = app.put_json(&format!("/api/playlists/{pid}"), &json!({"name": "Hall d'entrée"})).await;
    assert_eq!(renamed.code, 200);
    assert_eq!(renamed.json()["name"], "Hall d'entrée");
    assert_eq!(app.get(&format!("/api/playlists/{pid}")).await.json()["name"], "Hall d'entrée");
    // lister
    app.post_json("/api/playlists", &json!({"name": "Second"})).await;
    let list = app.get("/api/playlists").await.json();
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(list[0]["id"], pid, "ordre de création");
    // supprimer
    assert_eq!(app.delete(&format!("/api/playlists/{pid}")).await.code, 204);
    assert_eq!(app.get(&format!("/api/playlists/{pid}")).await.code, 404);
    assert_eq!(app.get("/api/playlists").await.json().as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn unknown_playlist_is_404_everywhere() {
    let app = TestApp::new().await;
    assert_eq!(app.get("/api/playlists/999").await.code, 404);
    assert_eq!(app.put_json("/api/playlists/999", &json!({"name": "x"})).await.code, 404);
    assert_eq!(app.delete("/api/playlists/999").await.code, 404);
    assert_eq!(app.put_json("/api/playlists/999/items", &json!([])).await.code, 404);
    assert_eq!(app.post("/api/playlists/999/publish").await.code, 404);
    assert_eq!(app.get("/api/playlists/999/preview").await.code, 404);
    assert_eq!(app.get("/api/playlists/999/versions").await.code, 404);
    assert_eq!(app.get("/api/versions/999").await.code, 404);
}

#[tokio::test]
async fn playlist_and_screen_names_are_validated() {
    let app = TestApp::new().await;
    for body in [json!({"name": ""}), json!({"name": "x".repeat(121)}), json!({}), json!({"name": 5})] {
        assert_eq!(app.post_json("/api/playlists", &body).await.code, 422, "{body}");
        assert_eq!(app.post_json("/api/screens", &body).await.code, 422, "{body}");
    }
    assert_eq!(app.post_json("/api/playlists", &json!({"name": "x".repeat(120)})).await.code, 201);
    let r = app.req(Method::POST, "/api/playlists").raw("application/json", b"{pas du json").send().await;
    assert_eq!(r.code, 422);
    assert!(r.json()["detail"].is_array(), "format de détail des erreurs de validation");
    assert!(r.json()["detail"][0]["msg"].is_string());
}

#[tokio::test]
async fn items_keep_order_and_get_default_durations() {
    let app = TestApp::new().await;
    let pid = app.post_json("/api/playlists", &json!({"name": "P"})).await.json()["id"].as_i64().unwrap();
    let img = app.upload("a.png", &png()).await.json();
    let vid = app.upload("b.mp4", &mp4()).await.json();
    let r = app
        .put_json(
            &format!("/api/playlists/{pid}/items"),
            &json!([
                {"type": "video", "media_id": vid["id"]},
                {"type": "text", "text": "Bonjour"},
                {"type": "image", "media_id": img["id"], "duration_s": 3},
                {"type": "image", "media_id": img["id"]},
            ]),
        )
        .await;
    assert_eq!(r.code, 200, "{}", r.text());
    let items = r.json()["items"].as_array().unwrap().clone();
    let positions: Vec<i64> = items.iter().map(|i| i["position"].as_i64().unwrap()).collect();
    assert_eq!(positions, [0, 1, 2, 3]);
    let kinds: Vec<&str> = items.iter().map(|i| i["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["video", "text", "image", "image"]);
    assert!(items[0]["duration_s"].is_null(), "vidéo : durée = lecture complète");
    assert_eq!(items[1]["duration_s"], 10, "texte : 10 s par défaut");
    assert_eq!(items[2]["duration_s"], 3);
    assert_eq!(items[3]["duration_s"], 10, "image : 10 s par défaut");
    assert!(items[1]["media_id"].is_null() && items[0]["text"].is_null());
    // la lecture suivante renvoie le même contenu, dans le même ordre
    let again = app.get(&format!("/api/playlists/{pid}")).await.json();
    assert_eq!(again["items"].as_array().unwrap().len(), 4);
    assert_eq!(again["items"][1]["text"], "Bonjour");
}

#[tokio::test]
async fn set_items_replaces_everything_and_can_empty_the_playlist() {
    let app = TestApp::new().await;
    let (pid, _, _) = mk_playlist(&app).await;
    let r = app.put_json(&format!("/api/playlists/{pid}/items"), &json!([{"type": "text", "text": "seul"}])).await;
    assert_eq!(r.json()["items"].as_array().unwrap().len(), 1);
    let r = app.put_json(&format!("/api/playlists/{pid}/items"), &json!([])).await;
    assert_eq!(r.code, 200);
    assert_eq!(app.get(&format!("/api/playlists/{pid}")).await.json()["items"], json!([]));
}

#[tokio::test]
async fn item_errors_name_the_faulty_position_in_french() {
    let app = TestApp::new().await;
    let pid = app.post_json("/api/playlists", &json!({"name": "P"})).await.json()["id"].as_i64().unwrap();
    let img = app.upload("a.png", &png()).await.json();
    let put = |body: Value| {
        let app = &app;
        let url = format!("/api/playlists/{pid}/items");
        async move { app.put_json(&url, &body).await }
    };
    let r = put(json!([{"type": "text", "text": "ok"}, {"type": "text", "text": "  "}])).await;
    assert_eq!(r.detail(), "Élément 1: texte requis");
    let r = put(json!([{"type": "image", "media_id": 4242}])).await;
    assert_eq!(r.detail(), "Élément 0: média introuvable");
    let r = put(json!([{"type": "image"}])).await;
    assert_eq!(r.detail(), "Élément 0: média introuvable", "media_id absent");
    let r = put(json!([{"type": "video", "media_id": img["id"]}])).await;
    assert_eq!(r.detail(), "Élément 0: le média est de type image");
    assert_eq!(put(json!([{"type": "audio"}])).await.code, 422, "type inconnu");
    assert_eq!(put(json!([{"type": "text", "text": "x".repeat(2001)}])).await.code, 422);
    assert_eq!(put(json!({"type": "text"})).await.code, 422, "tableau obligatoire");
}

#[tokio::test]
async fn a_failed_set_items_leaves_the_previous_content_untouched() {
    let app = TestApp::new().await;
    let (pid, _, _) = mk_playlist(&app).await;
    let r = app
        .put_json(
            &format!("/api/playlists/{pid}/items"),
            &json!([{"type": "text", "text": "nouveau"}, {"type": "image", "media_id": 99999}]),
        )
        .await;
    assert_eq!(r.code, 422);
    assert_eq!(app.get(&format!("/api/playlists/{pid}")).await.json()["items"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn deleting_a_playlist_removes_items_and_versions_but_not_media() {
    let app = TestApp::new().await;
    let (pid, img, _) = mk_playlist(&app).await;
    let v = app.post(&format!("/api/playlists/{pid}/publish")).await.json();
    assert_eq!(app.delete(&format!("/api/playlists/{pid}")).await.code, 204);
    assert_eq!(app.get(&format!("/api/versions/{}", v["id"])).await.code, 404);
    // les médias, eux, restent disponibles et peuvent maintenant être supprimés
    assert_eq!(app.delete(&format!("/api/media/{}", img["id"])).await.code, 204);
}

#[tokio::test]
async fn playlist_with_a_version_assigned_to_a_screen_cannot_be_deleted() {
    let app = TestApp::new().await;
    let (pid, _, _) = mk_playlist(&app).await;
    let v = app.post(&format!("/api/playlists/{pid}/publish")).await.json();
    let sc = app.post_json("/api/screens", &json!({"name": "S"})).await.json();
    app.post_json(&format!("/api/screens/{}/assign", sc["id"]), &json!({"version_id": v["id"]})).await;
    assert_eq!(app.delete(&format!("/api/playlists/{pid}")).await.code, 409);
    // une fois l'écran désaffecté, la suppression est possible
    app.post_json(&format!("/api/screens/{}/assign", sc["id"]), &json!({"version_id": null})).await;
    assert_eq!(app.delete(&format!("/api/playlists/{pid}")).await.code, 204);
}

// =====================================================================================
// Publication
// =====================================================================================
#[tokio::test]
async fn version_numbers_are_per_playlist_and_listed_in_order() {
    let app = TestApp::new().await;
    let mk = |name: &'static str| {
        let app = &app;
        async move {
            let pid = app.post_json("/api/playlists", &json!({"name": name})).await.json()["id"].as_i64().unwrap();
            app.put_json(&format!("/api/playlists/{pid}/items"), &json!([{"type": "text", "text": name}])).await;
            pid
        }
    };
    let (a, b) = (mk("A").await, mk("B").await);
    for _ in 0..3 {
        assert_eq!(app.post(&format!("/api/playlists/{a}/publish")).await.code, 201);
    }
    assert_eq!(app.post(&format!("/api/playlists/{b}/publish")).await.json()["number"], 1);
    let versions = app.get(&format!("/api/playlists/{a}/versions")).await.json();
    let numbers: Vec<i64> = versions.as_array().unwrap().iter().map(|v| v["number"].as_i64().unwrap()).collect();
    assert_eq!(numbers, [1, 2, 3]);
    for v in versions.as_array().unwrap() {
        assert_eq!(v["playlist_id"], a);
        assert!(v["created_at"].is_string());
    }
}

#[tokio::test]
async fn manifest_has_the_documented_structure() {
    let app = TestApp::new().await;
    let (pid, img, vid) = mk_playlist(&app).await;
    let v = app.post(&format!("/api/playlists/{pid}/publish")).await.json();
    let manifest = &v["manifest"];
    assert_eq!(manifest["playlist_id"], pid);
    assert_eq!(manifest["name"], "Hall");
    let items = manifest["items"].as_array().unwrap();
    assert_eq!(items[0], json!({"type": "text", "duration_s": 5, "text": "Bienvenue"}));
    assert_eq!(items[1]["media"], json!({"id": img["id"], "url": img["url"], "sha256": img["sha256"], "size": img["size"], "mime": "image/png"}));
    assert_eq!(items[2]["media"]["mime"], "video/mp4");
    assert_eq!(items[2]["media"]["id"], vid["id"]);
    // ce que reçoit le lecteur = version_id + number + manifeste
    let payload = app.get(&format!("/api/versions/{}", v["id"])).await.json();
    assert_eq!(payload["version_id"], v["id"]);
    assert_eq!(payload["number"], 1);
    assert_eq!(payload["items"], manifest["items"]);
}

#[tokio::test]
async fn renaming_a_playlist_does_not_alter_published_versions() {
    let app = TestApp::new().await;
    let (pid, _, _) = mk_playlist(&app).await;
    let v = app.post(&format!("/api/playlists/{pid}/publish")).await.json();
    app.put_json(&format!("/api/playlists/{pid}"), &json!({"name": "Renommée"})).await;
    let old = app.get(&format!("/api/versions/{}", v["id"])).await.json();
    assert_eq!(old["name"], "Hall");
    let draft = app.get(&format!("/api/playlists/{pid}/preview")).await.json();
    assert_eq!(draft["name"], "Renommée");
}

#[tokio::test]
async fn viewers_can_read_but_not_publish() {
    let app = TestApp::new().await;
    let (pid, _, _) = mk_playlist(&app).await;
    let v = app.post(&format!("/api/playlists/{pid}/publish")).await.json();
    let viewer = session_for(&app, "vee", "viewer").await;
    assert_eq!(app.get_as(&viewer, &format!("/api/versions/{}", v["id"])).await.code, 200);
    assert_eq!(app.get_as(&viewer, &format!("/api/playlists/{pid}/preview")).await.code, 200);
    let r = app.req(Method::POST, &format!("/api/playlists/{pid}/publish")).bearer(&viewer).send().await;
    assert_eq!(r.code, 403);
}

// =====================================================================================
// Écrans et lecteur
// =====================================================================================
#[tokio::test]
async fn screen_creation_shows_the_token_once() {
    let app = TestApp::new().await;
    let r = app.post_json("/api/screens", &json!({"name": "Entrée"})).await;
    assert_eq!(r.code, 201);
    let sc = r.json();
    assert_eq!(sc["name"], "Entrée");
    assert_eq!(sc["has_token"], true);
    assert_eq!(sc["online"], false);
    assert!(sc["version_id"].is_null() && sc["last_seen"].is_null());
    assert_eq!(sc["token"].as_str().unwrap().len(), 43);
    let listed = app.get("/api/screens").await.json();
    assert!(listed[0].get("token").is_none(), "le jeton n'est plus jamais renvoyé");
    assert!(listed[0].get("token_hash").is_none());
    assert!(!app.get("/api/screens").await.text().contains(sc["token"].as_str().unwrap()));
}

#[tokio::test]
async fn assigning_validates_screen_and_version() {
    let app = TestApp::new().await;
    let sc = app.post_json("/api/screens", &json!({"name": "S"})).await.json();
    let url = format!("/api/screens/{}/assign", sc["id"]);
    assert_eq!(app.post_json(&url, &json!({"version_id": 4242})).await.code, 404);
    assert_eq!(app.post_json("/api/screens/999/assign", &json!({"version_id": null})).await.code, 404);
    assert_eq!(app.post_json(&url, &json!({})).await.code, 422, "version_id obligatoire (même null)");
    assert_eq!(app.post_json(&url, &json!({"version_id": 0})).await.code, 422);
    assert_eq!(app.post_json(&url, &json!({"version_id": null})).await.code, 200);
    assert_eq!(app.post("/api/screens/999/rotate-token").await.code, 404);
    assert_eq!(app.post("/api/screens/999/revoke-token").await.code, 404);
    assert_eq!(app.get("/api/screens/999/manifest").await.code, 404);
}

#[tokio::test]
async fn unassigning_a_screen_stops_its_content() {
    let app = TestApp::new().await;
    let (pid, _, _) = mk_playlist(&app).await;
    let v = app.post(&format!("/api/playlists/{pid}/publish")).await.json();
    let sc = app.post_json("/api/screens", &json!({"name": "S"})).await.json();
    let tok = sc["token"].as_str().unwrap().to_string();
    let url = format!("/api/screens/{}/assign", sc["id"]);
    app.post_json(&url, &json!({"version_id": v["id"]})).await;
    assert_eq!(app.get_as(&tok, "/api/player/manifest").await.code, 200);
    let r = app.post_json(&url, &json!({"version_id": null})).await;
    assert!(r.json()["version_id"].is_null());
    let r = app.get_as(&tok, "/api/player/manifest").await;
    assert_eq!(r.code, 404);
    assert_eq!(r.detail(), "Aucune version affectée");
}

#[tokio::test]
async fn player_manifest_returns_the_assigned_version_and_marks_the_screen_online() {
    let app = TestApp::new().await;
    let (pid, _, _) = mk_playlist(&app).await;
    let v = app.post(&format!("/api/playlists/{pid}/publish")).await.json();
    let sc = app.post_json("/api/screens", &json!({"name": "S"})).await.json();
    app.post_json(&format!("/api/screens/{}/assign", sc["id"]), &json!({"version_id": v["id"]})).await;
    assert_eq!(app.get("/api/screens").await.json()[0]["online"], false);
    let tok = sc["token"].as_str().unwrap();
    let m = app.get_as(tok, "/api/player/manifest").await;
    assert_eq!(m.code, 200);
    assert_eq!(m.json()["version_id"], v["id"]);
    assert_eq!(m.json()["items"].as_array().unwrap().len(), 3);
    let screens = app.get("/api/screens").await.json();
    assert_eq!(screens[0]["online"], true);
    assert!(screens[0]["last_seen"].is_string());
}

#[tokio::test]
async fn player_manifest_error_messages() {
    let mut app = TestApp::new().await;
    app.auth = None;
    let r = app.get("/api/player/manifest").await;
    assert_eq!(r.code, 401);
    assert_eq!(r.detail(), "Jeton d'écran invalide ou révoqué");
    let r = app.get_as("faux", "/api/player/manifest").await;
    assert_eq!(r.code, 401);
}

#[tokio::test]
async fn a_new_version_does_not_reach_a_screen_until_assigned() {
    let app = TestApp::new().await;
    let (pid, _, _) = mk_playlist(&app).await;
    let v1 = app.post(&format!("/api/playlists/{pid}/publish")).await.json();
    let sc = app.post_json("/api/screens", &json!({"name": "S"})).await.json();
    let tok = sc["token"].as_str().unwrap().to_string();
    app.post_json(&format!("/api/screens/{}/assign", sc["id"]), &json!({"version_id": v1["id"]})).await;
    app.put_json(&format!("/api/playlists/{pid}/items"), &json!([{"type": "text", "text": "V2"}])).await;
    app.post(&format!("/api/playlists/{pid}/publish")).await;
    assert_eq!(app.get_as(&tok, "/api/player/manifest").await.json()["number"], 1, "déploiement explicite");
}

// =====================================================================================
// Comptes, sessions et journal
// =====================================================================================
#[tokio::test]
async fn user_listing_never_exposes_password_material() {
    let app = TestApp::new().await;
    make_user(&app, "emma", "motdepasse8", "editor").await;
    let r = app.get("/api/users").await;
    let text = r.text();
    assert!(!text.contains("pbkdf2") && !text.contains("password") && !text.contains("motdepasse8"));
    let users = r.json();
    let u = &users[0];
    for key in ["id", "username", "role", "active", "created_at", "last_login"] {
        assert!(u.get(key).is_some(), "{key}");
    }
    assert!(u["last_login"].is_null());
    login(&app, "emma", "motdepasse8").await;
    assert!(app.get("/api/users").await.json()[0]["last_login"].is_string());
}

#[tokio::test]
async fn user_creation_rules() {
    let app = TestApp::new().await;
    let default_role = make_user(&app, "emma", "motdepasse8", "viewer").await;
    assert_eq!(default_role.json()["role"], "viewer");
    let r = app.post_json("/api/users", &json!({"username": "lea", "password": "motdepasse8"})).await;
    assert_eq!(r.json()["role"], "viewer", "viewer par défaut");
    assert_eq!(r.json()["active"], true);
    for (name, pw) in [("ab", "motdepasse8"), ("avec espace", "motdepasse8"), ("émoji", "motdepasse8"), ("ok-name", "1234567")] {
        assert_eq!(make_user(&app, name, pw, "viewer").await.code, 422, "{name} / {pw}");
    }
    assert_eq!(make_user(&app, "root", "motdepasse8", "superuser").await.code, 422, "rôle inconnu");
    assert_eq!(app.get("/api/users").await.json().as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn user_update_changes_role_password_and_revokes_sessions() {
    let app = TestApp::new().await;
    make_user(&app, "emma", "motdepasse8", "viewer").await;
    let id = app.get("/api/users").await.json()[0]["id"].clone();
    let tok = login(&app, "emma", "motdepasse8").await.json()["token"].as_str().unwrap().to_string();
    assert_eq!(app.get_as(&tok, "/api/audit").await.code, 403);
    let r = app.patch_json(&format!("/api/users/{id}"), &json!({"role": "admin", "password": "autre-secret-1"})).await;
    assert_eq!(r.code, 200);
    assert_eq!(r.json()["role"], "admin");
    assert_eq!(app.get_as(&tok, "/api/auth/me").await.code, 401, "l'ancienne session est révoquée");
    assert_eq!(login(&app, "emma", "motdepasse8").await.code, 401, "l'ancien mot de passe ne marche plus");
    let l = login(&app, "emma", "autre-secret-1").await;
    assert_eq!(l.code, 200);
    assert_eq!(l.json()["role"], "admin");
    // validations
    assert_eq!(app.patch_json(&format!("/api/users/{id}"), &json!({"password": "court"})).await.code, 422);
    assert_eq!(app.patch_json(&format!("/api/users/{id}"), &json!({"role": "root"})).await.code, 422);
    assert_eq!(app.patch_json("/api/users/9999", &json!({"active": true})).await.code, 404);
    // un patch vide est accepté et ne change rien
    assert_eq!(app.patch_json(&format!("/api/users/{id}"), &json!({})).await.json()["role"], "admin");
}

#[tokio::test]
async fn an_admin_cannot_disable_or_delete_themselves() {
    let app = TestApp::new().await;
    let admin = session_for(&app, "boss", "admin").await;
    let second = session_for(&app, "second", "admin").await; // pour que « dernier admin » ne soit pas la cause du 409
    let me_id = app.get_as(&admin, "/api/users").await.json().as_array().unwrap().iter()
        .find(|u| u["username"] == "boss").unwrap()["id"].clone();
    let r = app.req(Method::PATCH, &format!("/api/users/{me_id}")).bearer(&admin).json(&json!({"active": false})).send().await;
    assert_eq!(r.code, 409);
    assert!(r.detail().contains("propre compte"));
    let r = app.req(Method::DELETE, &format!("/api/users/{me_id}")).bearer(&admin).send().await;
    assert_eq!(r.code, 409);
    // l'autre admin, lui, peut le faire
    let r = app.req(Method::DELETE, &format!("/api/users/{me_id}")).bearer(&second).send().await;
    assert_eq!(r.code, 204);
}

#[tokio::test]
async fn demoting_the_last_admin_is_refused() {
    let app = TestApp::new().await;
    make_user(&app, "emma", "motdepasse8", "admin").await;
    let id = app.get("/api/users").await.json()[0]["id"].clone();
    let r = app.patch_json(&format!("/api/users/{id}"), &json!({"role": "editor"})).await;
    assert_eq!(r.code, 409);
    assert_eq!(r.detail(), "Il doit rester au moins un administrateur actif");
}

#[tokio::test]
async fn disabled_users_cannot_log_in_and_expired_sessions_are_rejected() {
    let app = TestApp::new().await;
    make_user(&app, "emma", "motdepasse8", "viewer").await;
    let account = user::Entity::find().one(&app.state.db).await.unwrap().unwrap();
    // session déjà expirée
    let (expired, _) = auth::create_session(&app.state.db, account.id, Duration::seconds(-5)).await.unwrap();
    assert_eq!(app.get_as(&expired, "/api/auth/me").await.code, 401);
    // session valide, puis compte désactivé directement en base : refus immédiat
    let (valid, expires_at) = auth::create_session(&app.state.db, account.id, Duration::hours(1)).await.unwrap();
    assert!(expires_at > ttscreen::models::utcnow());
    assert_eq!(app.get_as(&valid, "/api/auth/me").await.code, 200);
    let mut am: user::ActiveModel = account.into();
    am.active = Set(false);
    am.update(&app.state.db).await.unwrap();
    assert_eq!(app.get_as(&valid, "/api/auth/me").await.code, 401);
}

#[tokio::test]
async fn session_lifetime_follows_the_configuration() {
    let app = TestApp::with(|s| s.session_ttl_h = 2).await;
    make_user(&app, "emma", "motdepasse8", "viewer").await;
    let r = login(&app, "emma", "motdepasse8").await.json();
    let expires = chrono::NaiveDateTime::parse_from_str(r["expires_at"].as_str().unwrap(), "%Y-%m-%dT%H:%M:%S%.f").unwrap();
    let delta = (expires - ttscreen::models::utcnow()).num_minutes();
    assert!((118..=120).contains(&delta), "{delta} minutes");
}

#[tokio::test]
async fn login_edge_cases() {
    let app = TestApp::new().await;
    make_user(&app, "emma", "motdepasse8", "viewer").await;
    // corps invalide / trop long / vide
    let r = app.req(Method::POST, "/api/auth/login").raw("application/json", b"nope").send().await;
    assert_eq!(r.code, 422);
    let long = json!({"username": "u".repeat(51), "password": "x"});
    assert_eq!(app.req(Method::POST, "/api/auth/login").json(&long).send().await.code, 422);
    assert_eq!(app.req(Method::POST, "/api/auth/login").json(&json!({})).send().await.code, 401);
    // le nom d'utilisateur est sensible à la casse
    assert_eq!(login(&app, "EMMA", "motdepasse8").await.code, 401);
    // le login ne demande aucune authentification préalable
    let mut anon = TestApp::new().await;
    anon.auth = None;
    make_user_as_legacy(&anon).await;
    assert_eq!(login(&anon, "zoe", "motdepasse8").await.code, 200);
}

async fn make_user_as_legacy(app: &TestApp) {
    let r = app
        .req(Method::POST, "/api/users")
        .bearer(TEST_TOKEN)
        .json(&json!({"username": "zoe", "password": "motdepasse8", "role": "viewer"}))
        .send()
        .await;
    assert_eq!(r.code, 201);
}

#[tokio::test]
async fn logout_requires_authentication_and_is_audited() {
    let mut app = TestApp::new().await;
    let tok = session_for(&app, "emma", "viewer").await;
    app.auth = None;
    assert_eq!(app.req(Method::POST, "/api/auth/logout").send().await.code, 401);
    app.auth = Some(TEST_TOKEN.into());
    assert_eq!(app.req(Method::POST, "/api/auth/logout").bearer(&tok).send().await.code, 204);
    assert!(actions(&audit_entries(&app).await).contains(&"deconnexion".to_string()));
    // le jeton historique peut aussi appeler logout (rien à révoquer en base) sans erreur
    assert_eq!(app.post("/api/auth/logout").await.code, 204);
    assert_eq!(app.get("/api/auth/me").await.code, 200, "le jeton historique reste valide");
}

#[tokio::test]
async fn audit_filters_paginates_and_is_admin_only() {
    let app = TestApp::new().await;
    for i in 0..5 {
        app.post_json("/api/playlists", &json!({"name": format!("P{i}")})).await;
    }
    let creates = app.get("/api/audit?action=playlist.create").await.json();
    assert_eq!(creates.as_array().unwrap().len(), 5);
    assert!(creates.as_array().unwrap().iter().all(|e| e["action"] == "playlist.create"));
    // ordre décroissant + pagination sans chevauchement
    let page1 = app.get("/api/audit?action=playlist.create&limit=2&offset=0").await.json();
    let page2 = app.get("/api/audit?action=playlist.create&limit=2&offset=2").await.json();
    let p1: Vec<i64> = page1.as_array().unwrap().iter().map(|e| e["id"].as_i64().unwrap()).collect();
    let p2: Vec<i64> = page2.as_array().unwrap().iter().map(|e| e["id"].as_i64().unwrap()).collect();
    assert_eq!((p1.len(), p2.len()), (2, 2));
    assert!(p1[0] > p1[1] && p1[1] > p2[0] && p2[0] > p2[1]);
    // paramètres invalides
    for q in ["limit=0", "limit=1001", "offset=-1", "limit=abc", "offset=x"] {
        assert_eq!(app.get(&format!("/api/audit?{q}")).await.code, 422, "{q}");
    }
    assert_eq!(app.get("/api/audit?action=").await.code, 200, "filtre vide = pas de filtre");
    assert_eq!(app.get("/api/audit?action=n.existe.pas").await.json(), json!([]));
    // réservé aux admins
    let editor = session_for(&app, "edi", "editor").await;
    assert_eq!(app.get_as(&editor, "/api/audit").await.code, 403);
    let mut anon = TestApp::new().await;
    anon.auth = None;
    assert_eq!(anon.get("/api/audit").await.code, 401);
}

#[tokio::test]
async fn audit_rows_have_the_documented_fields() {
    let app = TestApp::new().await;
    app.post_json("/api/playlists", &json!({"name": "Accueil"})).await;
    let rows = app.get("/api/audit?action=playlist.create").await.json();
    let e = &rows[0];
    for key in ["id", "created_at", "username", "role", "action", "target", "detail", "ip"] {
        assert!(e.get(key).is_some(), "{key}");
    }
    assert_eq!(e["username"], "legacy-admin");
    assert_eq!(e["role"], "admin");
    assert_eq!(e["target"], "Accueil");
}

// =====================================================================================
// Pages statiques, routes inconnues, santé
// =====================================================================================
#[tokio::test]
async fn root_and_directory_redirects() {
    let app = TestApp::new().await;
    let r = app.get("/").await;
    assert_eq!(r.code, 307);
    assert_eq!(r.header("location").as_deref(), Some("/admin/"));
    let r = app.get("/admin").await;
    assert_eq!((r.code, r.header("location").as_deref()), (307, Some("/admin/")));
    let r = app.get("/player").await;
    assert_eq!((r.code, r.header("location").as_deref()), (307, Some("/player/")));
}

#[tokio::test]
async fn static_assets_are_served_with_the_right_content_type() {
    let app = TestApp::new().await;
    let r = app.get("/admin/app.js").await;
    assert_eq!(r.code, 200);
    assert!(r.header("content-type").unwrap().contains("javascript"));
    let r = app.get("/player/player.js").await;
    assert_eq!(r.code, 200);
    assert!(r.header("content-type").unwrap().contains("javascript"));
    let r = app.get("/admin/").await;
    assert!(r.header("content-type").unwrap().starts_with("text/html"));
    assert_eq!(app.get("/admin/index.html").await.code, 200);
}

#[tokio::test]
async fn static_files_cannot_escape_their_directory() {
    let app = TestApp::new().await;
    assert_eq!(app.get("/admin/nope.js").await.code, 404);
    assert_eq!(app.get("/admin/../Cargo.toml").await.code, 404);
    assert_eq!(app.get("/admin/..%2FCargo.toml").await.code, 404);
    assert_eq!(app.get("/admin/%2e%2e/%2e%2e/etc/passwd").await.code, 404);
    assert_eq!(app.get("/player/../../etc/passwd").await.code, 404);
}

#[tokio::test]
async fn unknown_routes_return_json_404() {
    let app = TestApp::new().await;
    let r = app.get("/api/n-existe-pas").await;
    assert_eq!(r.code, 404);
    assert_eq!(r.json(), json!({"detail": "Not Found"}));
    assert_eq!(app.get("/n-existe-pas").await.code, 404);
}

#[tokio::test]
async fn health_is_public_and_minimal() {
    let mut app = TestApp::new().await;
    app.auth = None;
    let r = app.get("/api/health").await;
    assert_eq!(r.code, 200);
    assert_eq!(r.json(), json!({"status": "ok"}));
}

// =====================================================================================
// Démarrage et persistance
// =====================================================================================
#[tokio::test]
async fn startup_creates_missing_directories() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("a/b/c");
    let s = Settings {
        database_url: format!("sqlite:///{}", nested.join("data.db").display()),
        media_dir: nested.join("media"),
        ..base_settings(dir.path())
    };
    build_state(s).await.expect("dossiers créés automatiquement");
    assert!(nested.join("media").is_dir());
    assert!(nested.join("data.db").is_file());
}

#[tokio::test]
async fn startup_refuses_unsupported_databases_and_reserved_bootstrap_names() {
    let dir = tempfile::tempdir().unwrap();
    let s = Settings { database_url: "mysql://u:p@h/d".into(), ..base_settings(dir.path()) };
    assert!(build_state(s).await.is_err());
    let dir = tempfile::tempdir().unwrap();
    let s = Settings {
        bootstrap_admin_user: "legacy-admin".into(),
        bootstrap_admin_password: "motdepasse8".into(),
        ..base_settings(dir.path())
    };
    assert!(build_state(s).await.is_err());
}

#[tokio::test]
async fn development_without_credentials_starts_with_a_warning_only() {
    let dir = tempfile::tempdir().unwrap();
    let s = Settings { admin_token: String::new(), ..base_settings(dir.path()) };
    assert!(build_state(s).await.is_ok());
}

#[tokio::test]
async fn bootstrap_creates_the_first_admin_only_when_none_exists() {
    let app = TestApp::with(|s| {
        s.admin_token = String::new();
        s.bootstrap_admin_user = "root".into();
        s.bootstrap_admin_password = "motdepasse8".into();
    })
    .await;
    let l = login(&app, "root", "motdepasse8").await;
    assert_eq!((l.code, l.json()["role"].as_str()), (200, Some("admin")));
    // un autre nom d'amorçage ne crée rien tant qu'un admin existe
    let app2 = app.restart(|s| s.bootstrap_admin_user = "autre".into()).await;
    assert_eq!(login(&app2, "autre", "motdepasse8").await.code, 401);
    assert_eq!(login(&app2, "root", "motdepasse8").await.code, 200);
}

#[tokio::test]
async fn data_survives_a_restart() {
    let app = TestApp::new().await;
    let (pid, img, _) = mk_playlist(&app).await;
    let v = app.post(&format!("/api/playlists/{pid}/publish")).await.json();
    let after = app.restart(|_| {}).await;
    assert_eq!(after.get(&format!("/api/playlists/{pid}")).await.json()["items"].as_array().unwrap().len(), 3);
    assert_eq!(after.get(&format!("/api/versions/{}", v["id"])).await.code, 200);
    assert_eq!(after.get(img["url"].as_str().unwrap()).await.code, 200);
}

#[tokio::test]
async fn password_hashes_in_the_database_are_never_plaintext() {
    let app = TestApp::new().await;
    make_user(&app, "emma", "motdepasse8", "viewer").await;
    let u = user::Entity::find().one(&app.state.db).await.unwrap().unwrap();
    assert!(u.password_hash.starts_with("pbkdf2_sha256$1000$"));
    assert!(!u.password_hash.contains("motdepasse8"));
    assert!(auth::verify_password("motdepasse8", &u.password_hash));
}

#[tokio::test]
async fn hashes_written_by_the_python_version_still_verify() {
    // Compte « hérité » : haché produit par hashlib.pbkdf2_hmac("sha256", b"password", b"salt", 1, 32)
    let app = TestApp::new().await;
    user::ActiveModel {
        username: Set("ancien".into()),
        password_hash: Set("pbkdf2_sha256$1$73616c74$120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b".into()),
        role: Set("viewer".into()),
        active: Set(true),
        created_at: Set(ttscreen::models::utcnow()),
        ..Default::default()
    }
    .insert(&app.state.db)
    .await
    .unwrap();
    assert_eq!(login(&app, "ancien", "password").await.code, 200);
    assert_eq!(login(&app, "ancien", "autre").await.code, 401);
}

#[tokio::test]
async fn concurrent_requests_are_handled() {
    let app = Arc::new(TestApp::new().await);
    let mut tasks = Vec::new();
    for i in 0..20 {
        let app = app.clone();
        tasks.push(tokio::spawn(async move {
            app.post_json("/api/playlists", &json!({"name": format!("P{i}")})).await.code
        }));
    }
    for t in tasks {
        assert_eq!(t.await.unwrap(), 201);
    }
    assert_eq!(app.get("/api/playlists").await.json().as_array().unwrap().len(), 20);
}

#[tokio::test]
async fn concurrent_identical_uploads_create_a_single_media() {
    let app = Arc::new(TestApp::new().await);
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let app = app.clone();
        tasks.push(tokio::spawn(async move { app.upload("a.png", &png()).await }));
    }
    let mut ids = Vec::new();
    for t in tasks {
        let r = t.await.unwrap();
        assert_eq!(r.code, 200, "{}", r.text());
        ids.push(r.json()["id"].as_i64().unwrap());
    }
    ids.dedup();
    assert_eq!(ids.len(), 1, "un seul média, quel que soit l'ordre d'arrivée");
    assert_eq!(app.get("/api/media").await.json().as_array().unwrap().len(), 1);
}
