//! Groupes d'écrans et gestion des écrans : création, appartenance, héritage du contenu, priorités.

mod common;

use axum::http::Method;
use common::*;
use serde_json::json;

// =====================================================================================
// Groupes : CRUD
// =====================================================================================
#[tokio::test]
async fn group_crud_and_shapes() {
    let app = TestApp::new().await;
    let r = app.post_json("/api/groups", &json!({"name": "Hall"})).await;
    assert_eq!(r.code, 201);
    let g = r.json();
    assert_eq!(g["name"], "Hall");
    assert!(g["version_id"].is_null());
    assert_eq!(g["screen_count"], 0);
    let gid = g["id"].as_i64().unwrap();
    // renommer
    let r = app.put_json(&format!("/api/groups/{gid}"), &json!({"name": "Hall d'entrée"})).await;
    assert_eq!((r.code, r.json()["name"].clone()), (200, json!("Hall d'entrée")));
    // lister, triés par nom
    new_group(&app, "Accueil").await;
    let list = app.get("/api/groups").await.json();
    let names: Vec<&str> = list.as_array().unwrap().iter().map(|g| g["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Accueil", "Hall d'entrée"]);
    // supprimer
    assert_eq!(app.delete(&format!("/api/groups/{gid}")).await.code, 204);
    assert_eq!(app.get("/api/groups").await.json().as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn group_names_are_unique_and_validated() {
    let app = TestApp::new().await;
    let a = new_group(&app, "Hall").await;
    let b = new_group(&app, "Cafétéria").await;
    let r = app.post_json("/api/groups", &json!({"name": "Hall"})).await;
    assert_eq!(r.code, 409);
    assert_eq!(r.detail(), "Un groupe porte déjà ce nom");
    let r = app.put_json(&format!("/api/groups/{}", b["id"]), &json!({"name": "Hall"})).await;
    assert_eq!(r.code, 409, "renommer en un nom déjà pris");
    let r = app.put_json(&format!("/api/groups/{}", a["id"]), &json!({"name": "Hall"})).await;
    assert_eq!(r.code, 200, "garder son propre nom est permis");
    for bad in [json!({"name": ""}), json!({"name": "x".repeat(121)}), json!({}), json!({"name": 3})] {
        assert_eq!(app.post_json("/api/groups", &bad).await.code, 422, "{bad}");
    }
}

#[tokio::test]
async fn unknown_groups_are_404() {
    let app = TestApp::new().await;
    assert_eq!(app.put_json("/api/groups/999", &json!({"name": "x"})).await.code, 404);
    assert_eq!(app.delete("/api/groups/999").await.code, 404);
    assert_eq!(app.post_json("/api/groups/999/assign", &json!({"version_id": null})).await.code, 404);
    assert_eq!(app.get("/api/groups/0").await.code, 405, "pas de lecture individuelle : la liste suffit");
    assert_eq!(app.delete("/api/groups/0").await.code, 422);
}

#[tokio::test]
async fn viewers_can_list_groups_but_not_change_them() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let viewer = session_for(&app, "vee", "viewer").await;
    assert_eq!(app.get_as(&viewer, "/api/groups").await.code, 200);
    let post = |path: String, body: serde_json::Value| {
        let (app, viewer) = (&app, viewer.clone());
        async move { app.req(Method::POST, &path).bearer(&viewer).json(&body).send().await.code }
    };
    assert_eq!(post("/api/groups".into(), json!({"name": "Autre"})).await, 403);
    assert_eq!(post(format!("/api/groups/{}/assign", g["id"]), json!({"version_id": null})).await, 403);
    let r = app.req(Method::DELETE, &format!("/api/groups/{}", g["id"])).bearer(&viewer).send().await;
    assert_eq!(r.code, 403);
    let mut anon = TestApp::new().await;
    anon.auth = None;
    assert_eq!(anon.get("/api/groups").await.code, 401);
}

// =====================================================================================
// Appartenance d'un écran à un groupe
// =====================================================================================
#[tokio::test]
async fn a_screen_can_be_created_inside_a_group() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let r = app.post_json("/api/screens", &json!({"name": "Entrée", "group_id": g["id"]})).await;
    assert_eq!(r.code, 201, "{}", r.text());
    let s = r.json();
    assert_eq!((s["group_id"].clone(), s["group_name"].clone()), (g["id"].clone(), json!("Hall")));
    assert_eq!(app.get("/api/groups").await.json()[0]["screen_count"], 1);
    // groupe inexistant
    let r = app.post_json("/api/screens", &json!({"name": "X", "group_id": 999})).await;
    assert_eq!((r.code, r.detail()), (404, "Groupe introuvable".to_string()));
    assert_eq!(app.post_json("/api/screens", &json!({"name": "X", "group_id": 0})).await.code, 422);
}

#[tokio::test]
async fn screens_move_between_groups_and_out_of_them() {
    let app = TestApp::new().await;
    let (a, b) = (new_group(&app, "A").await, new_group(&app, "B").await);
    let s = new_screen(&app, "S").await;
    let url = format!("/api/screens/{}/group", s["id"]);
    let r = app.post_json(&url, &json!({"group_id": a["id"]})).await;
    assert_eq!((r.code, r.json()["group_name"].clone()), (200, json!("A")));
    let r = app.post_json(&url, &json!({"group_id": b["id"]})).await;
    assert_eq!(r.json()["group_name"], "B");
    let counts: Vec<i64> = app.get("/api/groups").await.json().as_array().unwrap().iter().map(|g| g["screen_count"].as_i64().unwrap()).collect();
    assert_eq!(counts, [0, 1]);
    let r = app.post_json(&url, &json!({"group_id": null})).await;
    assert!(r.json()["group_id"].is_null() && r.json()["group_name"].is_null());
}

#[tokio::test]
async fn set_group_validation() {
    let app = TestApp::new().await;
    let s = new_screen(&app, "S").await;
    let url = format!("/api/screens/{}/group", s["id"]);
    assert_eq!(app.post_json(&url, &json!({"group_id": 999})).await.code, 404);
    assert_eq!(app.post_json(&url, &json!({"group_id": 0})).await.code, 422);
    assert_eq!(app.post_json(&url, &json!({})).await.code, 422, "champ obligatoire (même null)");
    assert_eq!(app.post_json("/api/screens/999/group", &json!({"group_id": null})).await.code, 404);
}

// =====================================================================================
// Contenu : héritage et priorités
// =====================================================================================
#[tokio::test]
async fn all_screens_of_a_group_show_the_group_content() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let v = publish_text(&app, "Promo", "SOLDES").await;
    let mut screens = Vec::new();
    for name in ["Entrée", "Caisse", "Vitrine"] {
        let r = app.post_json("/api/screens", &json!({"name": name, "group_id": g["id"]})).await.json();
        screens.push(r);
    }
    let outsider = new_screen(&app, "Isolé").await;
    // rien d'affecté : personne n'affiche rien
    for s in &screens {
        assert_eq!(shown_text(&app, s["token"].as_str().unwrap()).await, None);
    }
    // UNE affectation au groupe : tous les écrans du groupe changent, pas l'écran isolé
    let r = app.post_json(&format!("/api/groups/{}/assign", g["id"]), &json!({"version_id": v})).await;
    assert_eq!((r.code, r.json()["version_id"].clone(), r.json()["screen_count"].clone()), (200, json!(v), json!(3)));
    for s in &screens {
        assert_eq!(shown_text(&app, s["token"].as_str().unwrap()).await.as_deref(), Some("SOLDES"));
        let out = screen_by_id(&app, &s["id"]).await;
        assert_eq!(out["effective_version_id"], v);
        assert_eq!(out["effective_source"], "group");
        assert!(out["version_id"].is_null(), "le contenu propre de l'écran reste vide");
    }
    assert_eq!(shown_text(&app, outsider["token"].as_str().unwrap()).await, None);
}

#[tokio::test]
async fn changing_the_group_content_changes_every_screen_at_once() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let (v1, v2) = (publish_text(&app, "A", "UN").await, publish_text(&app, "B", "DEUX").await);
    let s1 = app.post_json("/api/screens", &json!({"name": "S1", "group_id": g["id"]})).await.json();
    let s2 = app.post_json("/api/screens", &json!({"name": "S2", "group_id": g["id"]})).await.json();
    let url = format!("/api/groups/{}/assign", g["id"]);
    let assign = |v: serde_json::Value| {
        let (app, url) = (&app, url.clone());
        async move { app.post_json(&url, &json!({"version_id": v})).await }
    };
    assign(json!(v1)).await;
    assert_eq!(shown_text(&app, s1["token"].as_str().unwrap()).await.as_deref(), Some("UN"));
    assign(json!(v2)).await;
    for s in [&s1, &s2] {
        assert_eq!(shown_text(&app, s["token"].as_str().unwrap()).await.as_deref(), Some("DEUX"));
    }
    assign(json!(null)).await;
    for s in [&s1, &s2] {
        assert_eq!(shown_text(&app, s["token"].as_str().unwrap()).await, None, "contenu du groupe retiré");
    }
}

#[tokio::test]
async fn joining_and_leaving_a_group_changes_what_a_screen_shows() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let v = publish_text(&app, "P", "GROUPE").await;
    app.post_json(&format!("/api/groups/{}/assign", g["id"]), &json!({"version_id": v})).await;
    let s = new_screen(&app, "S").await;
    let tok = s["token"].as_str().unwrap();
    assert_eq!(shown_text(&app, tok).await, None);
    app.post_json(&format!("/api/screens/{}/group", s["id"]), &json!({"group_id": g["id"]})).await;
    assert_eq!(shown_text(&app, tok).await.as_deref(), Some("GROUPE"));
    app.post_json(&format!("/api/screens/{}/group", s["id"]), &json!({"group_id": null})).await;
    assert_eq!(shown_text(&app, tok).await, None);
}

#[tokio::test]
async fn a_screen_with_its_own_content_keeps_it_and_can_fall_back_to_the_group() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let (vg, vs) = (publish_text(&app, "G", "GROUPE").await, publish_text(&app, "S", "PROPRE").await);
    app.post_json(&format!("/api/groups/{}/assign", g["id"]), &json!({"version_id": vg})).await;
    let s = app.post_json("/api/screens", &json!({"name": "Spécial", "group_id": g["id"]})).await.json();
    let tok = s["token"].as_str().unwrap();
    assert_eq!(shown_text(&app, tok).await.as_deref(), Some("GROUPE"));
    // contenu fixé sur l'écran : il prime
    let r = app.post_json(&format!("/api/screens/{}/assign", s["id"]), &json!({"version_id": vs})).await;
    assert_eq!(r.json()["effective_source"], "screen");
    assert_eq!(shown_text(&app, tok).await.as_deref(), Some("PROPRE"));
    // le groupe change : l'écran n'est pas affecté
    let vg2 = publish_text(&app, "G2", "GROUPE-2").await;
    app.post_json(&format!("/api/groups/{}/assign", g["id"]), &json!({"version_id": vg2})).await;
    assert_eq!(shown_text(&app, tok).await.as_deref(), Some("PROPRE"));
    // retrait du contenu propre : retour au groupe
    app.post_json(&format!("/api/screens/{}/assign", s["id"]), &json!({"version_id": null})).await;
    assert_eq!(shown_text(&app, tok).await.as_deref(), Some("GROUPE-2"));
}

#[tokio::test]
async fn admin_preview_of_a_screen_follows_its_group() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let v = publish_text(&app, "P", "APERÇU").await;
    let s = app.post_json("/api/screens", &json!({"name": "S", "group_id": g["id"]})).await.json();
    let url = format!("/api/screens/{}/manifest", s["id"]);
    assert_eq!(app.get(&url).await.code, 404);
    app.post_json(&format!("/api/groups/{}/assign", g["id"]), &json!({"version_id": v})).await;
    let r = app.get(&url).await;
    assert_eq!((r.code, r.json()["items"][0]["text"].clone()), (200, json!("APERÇU")));
    assert_eq!(app.get("/api/screens").await.json()[0]["online"], false, "l'aperçu ne simule pas une connexion");
}

#[tokio::test]
async fn assigning_a_group_validates_the_version() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let url = format!("/api/groups/{}/assign", g["id"]);
    assert_eq!(app.post_json(&url, &json!({"version_id": 4242})).await.code, 404);
    assert_eq!(app.post_json(&url, &json!({"version_id": 0})).await.code, 422);
    assert_eq!(app.post_json(&url, &json!({})).await.code, 422);
    assert_eq!(app.post_json(&url, &json!({"version_id": null})).await.code, 200);
}

// =====================================================================================
// Suppression
// =====================================================================================
#[tokio::test]
async fn deleting_a_group_keeps_its_screens_but_they_lose_the_inherited_content() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let v = publish_text(&app, "P", "HÉRITÉ").await;
    app.post_json(&format!("/api/groups/{}/assign", g["id"]), &json!({"version_id": v})).await;
    let s = app.post_json("/api/screens", &json!({"name": "S", "group_id": g["id"]})).await.json();
    let tok = s["token"].as_str().unwrap();
    assert_eq!(shown_text(&app, tok).await.as_deref(), Some("HÉRITÉ"));
    assert_eq!(app.delete(&format!("/api/groups/{}", g["id"])).await.code, 204);
    let out = screen_by_id(&app, &s["id"]).await;
    assert!(out["group_id"].is_null() && out["effective_version_id"].is_null(), "{out}");
    assert_eq!(shown_text(&app, tok).await, None);
    assert_eq!(app.get("/api/screens").await.json().as_array().unwrap().len(), 1, "l'écran existe toujours");
    let audit = audit_entries(&app).await;
    let e = audit.iter().find(|e| e["action"] == "group.delete").unwrap();
    assert!(e["detail"].as_str().unwrap().contains("1 écran"));
}

#[tokio::test]
async fn a_playlist_used_by_a_group_cannot_be_deleted() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let pid = app.post_json("/api/playlists", &json!({"name": "P"})).await.json()["id"].as_i64().unwrap();
    app.put_json(&format!("/api/playlists/{pid}/items"), &json!([{"type": "text", "text": "x"}])).await;
    let v = app.post(&format!("/api/playlists/{pid}/publish")).await.json()["id"].clone();
    app.post_json(&format!("/api/groups/{}/assign", g["id"]), &json!({"version_id": v})).await;
    let r = app.delete(&format!("/api/playlists/{pid}")).await;
    assert_eq!(r.code, 409);
    assert!(r.detail().contains("groupe"), "{}", r.detail());
    app.post_json(&format!("/api/groups/{}/assign", g["id"]), &json!({"version_id": null})).await;
    assert_eq!(app.delete(&format!("/api/playlists/{pid}")).await.code, 204);
}

// =====================================================================================
// Gestion des écrans : renommer, supprimer
// =====================================================================================
#[tokio::test]
async fn screens_can_be_renamed() {
    let app = TestApp::new().await;
    let s = new_screen(&app, "Ancien").await;
    let url = format!("/api/screens/{}", s["id"]);
    let r = app.put_json(&url, &json!({"name": "Nouveau"})).await;
    assert_eq!((r.code, r.json()["name"].clone()), (200, json!("Nouveau")));
    assert_eq!(screen_by_id(&app, &s["id"]).await["name"], "Nouveau");
    assert_eq!(app.put_json(&url, &json!({"name": ""})).await.code, 422);
    assert_eq!(app.put_json(&url, &json!({"name": "x".repeat(121)})).await.code, 422);
    assert_eq!(app.put_json("/api/screens/999", &json!({"name": "x"})).await.code, 404);
    // le jeton n'est pas touché par un renommage
    assert_eq!(shown_text(&app, s["token"].as_str().unwrap()).await, None);
}

#[tokio::test]
async fn deleting_a_screen_invalidates_its_token() {
    let app = TestApp::new().await;
    let s = new_screen(&app, "S").await;
    let tok = s["token"].as_str().unwrap().to_string();
    assert_eq!(app.get_as(&tok, "/api/player/manifest").await.code, 404);
    assert_eq!(app.delete(&format!("/api/screens/{}", s["id"])).await.code, 204);
    assert_eq!(app.get_as(&tok, "/api/player/manifest").await.code, 401);
    assert_eq!(app.get("/api/screens").await.json(), json!([]));
    assert_eq!(app.delete(&format!("/api/screens/{}", s["id"])).await.code, 404);
    assert!(actions(&audit_entries(&app).await).contains(&"screen.delete".to_string()));
}

#[tokio::test]
async fn deleting_a_screen_leaves_its_group_and_other_screens_intact() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let a = app.post_json("/api/screens", &json!({"name": "A", "group_id": g["id"]})).await.json();
    app.post_json("/api/screens", &json!({"name": "B", "group_id": g["id"]})).await;
    app.delete(&format!("/api/screens/{}", a["id"])).await;
    assert_eq!(app.get("/api/groups").await.json()[0]["screen_count"], 1);
}

#[tokio::test]
async fn viewers_cannot_manage_screens() {
    let app = TestApp::new().await;
    let s = new_screen(&app, "S").await;
    let g = new_group(&app, "G").await;
    let viewer = session_for(&app, "vee", "viewer").await;
    let r = app.req(Method::PUT, &format!("/api/screens/{}", s["id"])).bearer(&viewer).json(&json!({"name": "x"})).send().await;
    assert_eq!(r.code, 403);
    let r = app.req(Method::DELETE, &format!("/api/screens/{}", s["id"])).bearer(&viewer).send().await;
    assert_eq!(r.code, 403);
    let r = app.req(Method::POST, &format!("/api/screens/{}/group", s["id"])).bearer(&viewer).json(&json!({"group_id": g["id"]})).send().await;
    assert_eq!(r.code, 403);
}

#[tokio::test]
async fn group_operations_are_audited() {
    let app = TestApp::new().await;
    let g = new_group(&app, "Hall").await;
    let s = new_screen(&app, "S").await;
    let v = publish_text(&app, "P", "x").await;
    app.put_json(&format!("/api/groups/{}", g["id"]), &json!({"name": "Hall 2"})).await;
    app.post_json(&format!("/api/groups/{}/assign", g["id"]), &json!({"version_id": v})).await;
    app.post_json(&format!("/api/screens/{}/group", s["id"]), &json!({"group_id": g["id"]})).await;
    app.put_json(&format!("/api/screens/{}", s["id"]), &json!({"name": "S2"})).await;
    let got = actions(&audit_entries(&app).await);
    for expected in ["group.create", "group.rename", "group.assign", "screen.group", "screen.rename"] {
        assert!(got.iter().any(|a| a == expected), "{expected} manquante : {got:?}");
    }
}

#[tokio::test]
async fn existing_screen_fields_are_still_present_for_compatibility() {
    let app = TestApp::new().await;
    let s = new_screen(&app, "S").await;
    for key in ["id", "name", "version_id", "online", "last_seen", "has_token", "token"] {
        assert!(s.get(key).is_some(), "{key}");
    }
    let listed = app.get("/api/screens").await.json();
    for key in ["id", "name", "version_id", "online", "last_seen", "has_token", "group_id", "group_name", "effective_version_id", "effective_source"] {
        assert!(listed[0].get(key).is_some(), "{key}");
    }
}
