//! Port de `backend/tests/test_security.py` : un test (ou plus) par faille corrigée (F1…F11).

mod common;

use axum::http::Method;
use common::*;
use sea_orm::ConnectionTrait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use ttscreen::{build_state, db, Settings};

/// Application « durcie » : jeton historique seulement tant qu'aucun admin nommé n'existe,
/// pas de limitation de débit sauf demande explicite.
async fn mk(tweak: impl FnOnce(&mut Settings)) -> TestApp {
    TestApp::with(|s| {
        s.legacy_token_always = false;
        s.rate_limit_login = 0;
        s.rate_limit_api = 0;
        tweak(s);
    })
    .await
}

// ---------- F1 : plus de jeton admin par défaut ----------
#[tokio::test]
async fn f1_no_default_admin_token() {
    assert_eq!(Settings::default().admin_token, "");
    let mut app = mk(|s| s.admin_token = String::new()).await;
    app.auth = Some("dev-admin-token".into());
    assert_eq!(app.get("/api/auth/me").await.code, 401);
}

#[tokio::test]
async fn f1_weak_token_refused_in_every_environment() {
    for env in ["development", "production"] {
        for weak in ["dev-admin-token", "court", "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"] {
            let dir = tempfile::tempdir().unwrap();
            let s = Settings { env: env.into(), admin_token: weak.into(), ..base_settings(dir.path()) };
            assert!(build_state(s).await.is_err(), "{env} / {weak}");
        }
    }
}

#[tokio::test]
async fn f1_production_needs_a_way_to_log_in() {
    let dir = tempfile::tempdir().unwrap();
    let s = Settings { env: "production".into(), admin_token: String::new(), ..base_settings(dir.path()) };
    assert!(build_state(s).await.is_err(), "ni jeton ni compte d'amorçage");
    let dir = tempfile::tempdir().unwrap();
    let s = Settings {
        env: "production".into(),
        admin_token: String::new(),
        bootstrap_admin_user: "root".into(),
        bootstrap_admin_password: "motdepasse8".into(),
        ..base_settings(dir.path())
    };
    assert!(build_state(s).await.is_ok());
}

// ---------- F2 : /api/media est authentifié et borné AVANT la lecture du corps ----------
#[tokio::test]
async fn f2_upload_body_is_never_read_before_auth_and_size_checks() {
    let mut app = mk(|_| {}).await;
    let viewer = session_for(&app, "vee", "viewer").await;
    let editor = session_for(&app, "edy", "editor").await;
    let mut huge = png();
    huge.extend(vec![0u8; 2 * 1024 * 1024]);

    // anonyme
    app.auth = None;
    let flag = Arc::new(AtomicBool::new(false));
    let r = app.req(Method::POST, "/api/media").multipart("x.png", &png()).track_body(flag.clone()).send().await;
    assert_eq!(r.code, 401);
    assert!(!flag.load(Ordering::SeqCst), "anonyme : corps jamais lu");

    // jeton invalide
    let flag = Arc::new(AtomicBool::new(false));
    let r = app
        .req(Method::POST, "/api/media")
        .bearer("pas-un-jeton")
        .multipart("x.png", &png())
        .track_body(flag.clone())
        .send()
        .await;
    assert_eq!(r.code, 401);
    assert!(!flag.load(Ordering::SeqCst), "jeton invalide : corps jamais lu");

    // rôle insuffisant
    let flag = Arc::new(AtomicBool::new(false));
    let r = app
        .req(Method::POST, "/api/media")
        .bearer(&viewer)
        .multipart("x.png", &png())
        .track_body(flag.clone())
        .send()
        .await;
    assert_eq!(r.code, 403);
    assert!(!flag.load(Ordering::SeqCst), "viewer : corps jamais lu");

    // éditeur légitime mais taille déclarée excessive : 413 sans lire le corps
    let flag = Arc::new(AtomicBool::new(false));
    let r = app
        .req(Method::POST, "/api/media")
        .bearer(&editor)
        .multipart("gros.png", &huge)
        .track_body(flag.clone())
        .send()
        .await;
    assert_eq!(r.code, 413);
    assert!(!flag.load(Ordering::SeqCst), "taille déclarée trop grande : corps jamais lu");

    // et un envoi valide fonctionne (le corps est alors bien lu)
    let flag = Arc::new(AtomicBool::new(false));
    let r = app
        .req(Method::POST, "/api/media")
        .bearer(&editor)
        .multipart("ok.png", &png())
        .track_body(flag.clone())
        .send()
        .await;
    assert_eq!(r.code, 200, "{}", r.text());
    assert!(flag.load(Ordering::SeqCst));
}

// ---------- F3 : force brute sur les jetons et identifiants ----------
#[tokio::test]
async fn f3_invalid_tokens_are_throttled_per_ip() {
    let mut app = mk(|s| s.rate_limit_authfail = 5).await;
    let valid = app.auth.take().unwrap();
    let mut codes = Vec::new();
    for i in 0..8 {
        codes.push(app.get_as(&format!("guess-{i}"), "/api/auth/me").await.code);
    }
    assert_eq!(codes, [401, 401, 401, 401, 401, 429, 429, 429]);
    // même un jeton valide est refusé depuis une IP bloquée
    assert_eq!(app.get_as(&valid, "/api/auth/me").await.code, 429);
}

#[tokio::test]
async fn f3_failed_token_attempts_are_audited_without_the_token() {
    let app = mk(|s| s.rate_limit_authfail = 5).await;
    for i in 0..3 {
        app.get_as(&format!("guess-{i}"), "/api/auth/me").await;
    }
    let entries = audit_entries(&app).await;
    let failures: Vec<&Value> = entries.iter().filter(|e| e["action"] == "auth_echouee").collect();
    assert_eq!(failures.len(), 3);
    assert!(!serde_json::to_string(&entries).unwrap().contains("guess-0"), "le jeton n'est jamais journalisé");
}

#[tokio::test]
async fn f3_login_is_throttled_per_username_across_ips() {
    let app = mk(|s| s.rate_limit_login_user = 3).await;
    make_user(&app, "bob", "motdepasse8", "viewer").await;
    let mut codes = Vec::new();
    for _ in 0..4 {
        codes.push(login(&app, "bob", "mauvais123").await.code);
    }
    assert_eq!(codes, [401, 401, 401, 429]);
    assert_eq!(login(&app, "bob", "motdepasse8").await.code, 429, "même le bon mot de passe est bloqué");
    // un autre identifiant n'est pas affecté
    assert_eq!(login(&app, "alice", "x").await.code, 401);
    // et l'attaquant change d'adresse : le compte reste protégé
    let from_elsewhere = app
        .req(Method::POST, "/api/auth/login")
        .json(&json!({"username": "bob", "password": "motdepasse8"}))
        .peer("203.0.113.77:5555")
        .send()
        .await;
    assert_eq!(from_elsewhere.code, 429);
}

// ---------- F4 : X-Forwarded-For ----------
#[test]
fn f4_deployment_files_restrict_forwarded_headers() {
    let root = repo_root();
    let dockerfile = std::fs::read_to_string(root.join("deploy/Dockerfile")).unwrap();
    assert!(dockerfile.contains("FORWARDED_ALLOW_IPS"), "le Dockerfile doit fixer FORWARDED_ALLOW_IPS");
    assert!(!dockerfile.contains("FORWARDED_ALLOW_IPS=*"), "jamais « tous » par défaut dans l'image");
    let prod = std::fs::read_to_string(root.join("deploy/docker-compose.prod.yml")).unwrap();
    assert!(prod.contains("FORWARDED_ALLOW_IPS"));
    // le service `api` n'est exposé sur aucun port de l'hôte (Caddy est le seul point d'entrée)
    let api_block = prod.split("\n  api:").nth(1).expect("service api");
    let api_block = api_block.split("\n  caddy:").next().unwrap();
    assert!(
        !api_block.lines().any(|l| l.trim_start().starts_with("ports:")),
        "api ne doit pas publier de port"
    );
}

#[tokio::test]
async fn f4_forwarded_for_is_ignored_from_untrusted_peers() {
    let app = mk(|s| s.forwarded_allow_ips = "10.0.0.0/8".into()).await;
    // pair non fiable : l'en-tête est ignoré (impossible de choisir son IP)
    app.req(Method::POST, "/api/auth/login")
        .json(&json!({"username": "x", "password": "y"}))
        .peer("203.0.113.9:1234")
        .header("x-forwarded-for", "1.2.3.4")
        .send()
        .await;
    // pair fiable (le proxy) : l'en-tête désigne le vrai client
    app.req(Method::POST, "/api/auth/login")
        .json(&json!({"username": "x", "password": "y"}))
        .peer("10.0.0.2:1234")
        .header("x-forwarded-for", "198.51.100.4")
        .send()
        .await;
    let entries = audit_entries(&app).await;
    let ips: Vec<&str> = entries.iter().filter(|e| e["action"] == "connexion_echouee").map(|e| e["ip"].as_str().unwrap()).collect();
    assert!(ips.contains(&"203.0.113.9"), "{ips:?}");
    assert!(ips.contains(&"198.51.100.4"), "{ips:?}");
    assert!(!ips.contains(&"1.2.3.4"), "{ips:?}");
}

#[tokio::test]
async fn f4_rate_limit_cannot_be_bypassed_by_spoofing_xff() {
    let app = mk(|s| {
        s.rate_limit_api = 3;
        s.forwarded_allow_ips = "10.0.0.0/8".into();
    })
    .await;
    let mut codes = Vec::new();
    for i in 0..5 {
        let r = app
            .req(Method::GET, "/api/playlists")
            .peer("203.0.113.9:1234")
            .header("x-forwarded-for", &format!("9.9.9.{i}"))
            .send()
            .await;
        codes.push(r.code);
    }
    assert_eq!(codes, [200, 200, 200, 429, 429]);
}

// ---------- F5 : le jeton historique ne reste pas une porte dérobée ----------
#[tokio::test]
async fn f5_legacy_token_disabled_once_a_named_admin_exists() {
    let mut app = mk(|_| {}).await;
    let me = app.get("/api/auth/me").await.json();
    assert_eq!((me["username"].as_str(), me["legacy"].as_bool()), (Some("legacy-admin"), Some(true)));
    // nom réservé : impossible d'usurper le principal historique
    assert_eq!(make_user(&app, "legacy-admin", "motdepasse8", "admin").await.code, 422);
    assert_eq!(make_user(&app, "ANONYME", "motdepasse8", "viewer").await.code, 422);
    assert_eq!(make_user(&app, "emma", "motdepasse8", "admin").await.code, 201);
    // désormais, le jeton historique est refusé partout
    assert_eq!(app.get("/api/auth/me").await.code, 401);
    let r = app.req(Method::POST, "/api/auth/login").json(&json!({"token": TEST_TOKEN})).send().await;
    assert_eq!(r.code, 401);
    // le compte nommé fonctionne
    let tok = login(&app, "emma", "motdepasse8").await.json()["token"].as_str().unwrap().to_string();
    app.auth = Some(tok);
    let me = app.get("/api/auth/me").await.json();
    assert_eq!((me["username"].as_str(), me["legacy"].as_bool()), (Some("emma"), Some(false)));
}

#[tokio::test]
async fn f5_legacy_token_always_keeps_the_old_behaviour() {
    let app = mk(|s| s.legacy_token_always = true).await;
    assert_eq!(make_user(&app, "emma", "motdepasse8", "admin").await.code, 201);
    assert_eq!(app.get("/api/auth/me").await.code, 200, "toujours accepté si demandé explicitement");
}

// ---------- F6 : /api/player/manifest ne contourne pas la limitation ----------
#[tokio::test]
async fn f6_player_manifest_is_limited_per_ip_even_with_rotating_tokens() {
    let app = mk(|s| {
        s.rate_limit_api = 5;
        s.rate_limit_authfail = 0;
    })
    .await;
    let mut codes = Vec::new();
    for i in 0..10 {
        codes.push(app.req(Method::GET, "/api/player/manifest").bearer(&format!("faux-{i}")).send().await.code);
    }
    assert_eq!(codes[..5], [401, 401, 401, 401, 401]);
    assert!(codes[5..].iter().all(|c| *c == 429), "{codes:?}");
}

#[tokio::test]
async fn f6_player_manifest_failures_are_audited() {
    let app = mk(|_| {}).await;
    app.req(Method::GET, "/api/player/manifest").bearer("faux-jeton-ecran").send().await;
    let entries = audit_entries(&app).await;
    let e = entries.iter().find(|e| e["action"] == "auth_echouee").expect("échec journalisé");
    assert_eq!(e["target"], "ecran");
}

// ---------- F7 : jetons non ASCII ----------
#[tokio::test]
async fn f7_non_ascii_tokens_are_a_clean_401_not_a_500() {
    let app = mk(|_| {}).await;
    let r = app.req(Method::POST, "/api/auth/login").json(&json!({"token": "é☃"})).send().await;
    assert_eq!(r.code, 401);
    let r = app
        .req(Method::GET, "/api/auth/me")
        .no_auth()
        .header_bytes("authorization", b"Bearer \xe9")
        .send()
        .await;
    assert_eq!(r.code, 401);
    let r = app.req(Method::GET, "/api/player/manifest").no_auth().header_bytes("authorization", b"Bearer \xe9\xff").send().await;
    assert_eq!(r.code, 401);
}

// ---------- F8 : durée de réponse égalisée pour les identifiants inconnus ----------
#[tokio::test]
async fn f8_unknown_user_still_runs_a_full_password_verification() {
    let app = mk(|_| {}).await;
    make_user(&app, "real", "motdepasse8", "viewer").await;
    let counter = app.state.password_verifications.clone();
    let before = counter.load(Ordering::SeqCst);
    assert_eq!(login(&app, "fantome", "motdepasse8").await.code, 401);
    assert_eq!(counter.load(Ordering::SeqCst), before + 1, "PBKDF2 exécuté même si le compte n'existe pas");
    assert_eq!(login(&app, "real", "faux-mot-de-passe").await.code, 401);
    assert_eq!(counter.load(Ordering::SeqCst), before + 2);
    // même message dans tous les cas : aucune fuite sur l'existence d'un identifiant
    let a = login(&app, "fantome", "motdepasse8").await;
    let b = login(&app, "real", "faux-mot-de-passe").await;
    assert_eq!(a.json(), b.json());
}

// ---------- F9 : le compte d'amorçage n'écrase rien au redémarrage ----------
#[tokio::test]
async fn f9_bootstrap_never_overwrites_existing_accounts_on_restart() {
    let cfg = |s: &mut Settings| {
        s.admin_token = String::new();
        s.legacy_token_always = false;
        s.bootstrap_admin_user = "root".into();
        s.bootstrap_admin_password = "motdepasse8".into();
    };
    let app1 = TestApp::with(cfg).await;
    let tok = login(&app1, "root", "motdepasse8").await.json()["token"].as_str().unwrap().to_string();
    let uid = app1.get_as(&tok, "/api/users").await.json()[0]["id"].as_i64().unwrap();
    // l'admin change son mot de passe via l'interface
    let r = app1
        .req(Method::PATCH, &format!("/api/users/{uid}"))
        .bearer(&tok)
        .json(&json!({"password": "nouveau-secret-9"}))
        .send()
        .await;
    assert_eq!(r.code, 200);

    // redémarrage avec les MÊMES variables d'amorçage : rien n'est écrasé
    let app2 = app1.restart(|_| {}).await;
    assert_eq!(login(&app2, "root", "nouveau-secret-9").await.code, 200);
    assert_eq!(login(&app2, "root", "motdepasse8").await.code, 401, "l'ancien mot de passe ne revient pas");

    // récupération explicite : RESET=true réinitialise
    let app3 = app2.restart(|s| s.bootstrap_admin_reset = true).await;
    assert_eq!(login(&app3, "root", "motdepasse8").await.code, 200);
}

// ---------- F10 : hygiène du dépôt ----------
#[test]
fn f10_repository_hygiene() {
    let root = repo_root();
    let gitignore = std::fs::read_to_string(root.join(".gitignore")).unwrap();
    for pattern in [".env", "backend/data/", "*.db", "target/"] {
        assert!(gitignore.lines().any(|l| l.trim() == pattern), "{pattern} doit être ignoré");
    }
    let manifest: toml::Table = std::fs::read_to_string(root.join("backend/Cargo.toml")).unwrap().parse().unwrap();
    let deps = manifest.get("dependencies").and_then(|v| v.as_table()).expect("[dependencies]");
    let dev = manifest.get("dev-dependencies").and_then(|v| v.as_table()).expect("[dev-dependencies]");
    for tool in ["tempfile", "toml", "futures-util"] {
        assert!(dev.contains_key(tool), "{tool} est un outil de test");
        assert!(!deps.contains_key(tool), "{tool} ne doit pas entrer dans le binaire livré");
    }
    for (name, spec) in deps.iter().chain(dev.iter()) {
        let version = match spec {
            toml::Value::String(v) => Some(v.as_str()),
            toml::Value::Table(t) => t.get("version").and_then(|v| v.as_str()),
            _ => None,
        };
        let version = version.unwrap_or_else(|| panic!("{name} sans version"));
        assert_ne!(version, "*", "{name} : version non bornée");
    }
}

// ---------- F11 : publication et audit dans la même transaction ----------
#[tokio::test]
async fn f11_publish_is_rolled_back_if_the_audit_write_fails() {
    let app = mk(|_| {}).await;
    let pid = app.post_json("/api/playlists", &json!({"name": "P"})).await.json()["id"].as_i64().unwrap();
    app.put_json(&format!("/api/playlists/{pid}/items"), &json!([{"type": "text", "text": "x"}])).await;
    // on casse le journal : l'écriture d'audit échouera
    app.state.db.execute_unprepared("DROP TABLE audit_log").await.unwrap();
    let r = app.post(&format!("/api/playlists/{pid}/publish")).await;
    assert_eq!(r.code, 500, "{}", r.text());
    // on répare le journal : aucune version fantôme n'a été validée
    db::init_schema(&app.state.db).await.unwrap();
    assert_eq!(app.get(&format!("/api/playlists/{pid}/versions")).await.json(), json!([]));
    // et la publication fonctionne de nouveau, avec sa ligne d'audit
    assert_eq!(app.post(&format!("/api/playlists/{pid}/publish")).await.code, 201);
    assert!(actions(&audit_entries(&app).await).contains(&"playlist.publish".to_string()));
}

#[tokio::test]
async fn f11_denied_access_is_audited_with_role_and_route() {
    let app = mk(|_| {}).await;
    let viewer = session_for(&app, "vee", "viewer").await;
    let r = app.req(Method::POST, "/api/playlists").bearer(&viewer).json(&json!({"name": "x"})).send().await;
    assert_eq!(r.code, 403);
    let entries = audit_entries(&app).await;
    let e = entries.iter().find(|e| e["action"] == "acces_refuse").expect("accès refusé journalisé");
    assert_eq!(e["username"], "vee");
    assert_eq!(e["role"], "viewer");
    assert!(e["target"].as_str().unwrap().contains("/api/playlists"));
    assert!(e["detail"].as_str().unwrap().contains("editor"));
}

// ---------- divers durcissements ----------
#[tokio::test]
async fn long_filenames_do_not_break_the_audit_log() {
    let app = mk(|_| {}).await;
    let name = format!("{}.png", "n".repeat(300));
    let r = app.upload(&name, &png()).await;
    assert_eq!(r.code, 200, "{}", r.text());
    assert_eq!(r.json()["original_name"].as_str().unwrap().chars().count(), 255);
    for e in audit_entries(&app).await {
        if let Some(t) = e["target"].as_str() {
            assert!(t.chars().count() <= 120, "cible tronquée à la colonne");
        }
    }
}

#[tokio::test]
async fn ids_and_lists_are_bounded() {
    let app = mk(|_| {}).await;
    assert_eq!(app.get("/api/playlists/99999999999").await.code, 422);
    assert_eq!(app.get("/api/playlists/0").await.code, 422);
    assert_eq!(app.get("/api/playlists/abc").await.code, 422);
    assert_eq!(app.get("/api/playlists/2147483647").await.code, 404, "borne haute valide mais inexistante");
    let pid = app.post_json("/api/playlists", &json!({"name": "P"})).await.json()["id"].as_i64().unwrap();
    let too_many: Vec<Value> = (0..501).map(|_| json!({"type": "text", "text": "x"})).collect();
    assert_eq!(app.put_json(&format!("/api/playlists/{pid}/items"), &Value::Array(too_many)).await.code, 422);
    let exactly: Vec<Value> = (0..500).map(|_| json!({"type": "text", "text": "x"})).collect();
    assert_eq!(app.put_json(&format!("/api/playlists/{pid}/items"), &Value::Array(exactly)).await.code, 200);
    let big: i64 = 1 << 40;
    let r = app.put_json(&format!("/api/playlists/{pid}/items"), &json!([{"type": "image", "media_id": big}])).await;
    assert_eq!(r.code, 422);
    let sc = app.post_json("/api/screens", &json!({"name": "S"})).await.json();
    let r = app.post_json(&format!("/api/screens/{}/assign", sc["id"]), &json!({"version_id": big})).await;
    assert_eq!(r.code, 422);
}

#[tokio::test]
async fn files_only_serves_application_generated_names() {
    let app = mk(|_| {}).await;
    let m = app.upload("a.png", &png()).await.json();
    assert_eq!(app.get(m["url"].as_str().unwrap()).await.code, 200);
    // un fichier quelconque du volume (ex. une sauvegarde) n'est jamais servi
    std::fs::write(app.media_dir().join("secret.txt"), "confidentiel").unwrap();
    std::fs::create_dir_all(app.media_dir().join(".tmp")).unwrap();
    std::fs::write(app.media_dir().join(".tmp").join("upload-1"), "partiel").unwrap();
    let missing = format!("{}.png", "0".repeat(64));
    let upper = format!("{}.png", "A".repeat(64));
    for bad in [
        "secret.txt", ".tmp", "..%2fsecret.txt", "..%2F..%2Fetc%2Fpasswd", missing.as_str(), upper.as_str(), "x.png",
    ] {
        assert_eq!(app.get(&format!("/files/{bad}")).await.code, 404, "{bad}");
    }
}

#[tokio::test]
async fn security_headers_are_present_on_every_kind_of_response() {
    let app = mk(|_| {}).await;
    for path in ["/api/health", "/admin/", "/player/", "/api/inconnu", "/files/x.png"] {
        let r = app.get(path).await;
        assert_eq!(r.header("x-content-type-options").as_deref(), Some("nosniff"), "{path}");
        assert_eq!(r.header("x-frame-options").as_deref(), Some("SAMEORIGIN"), "{path}");
        assert_eq!(r.header("referrer-policy").as_deref(), Some("no-referrer"), "{path}");
        assert_eq!(r.header("cross-origin-resource-policy").as_deref(), Some("same-origin"), "{path}");
        let csp = r.header("content-security-policy").unwrap_or_default();
        assert!(csp.contains("default-src 'self'") && csp.contains("frame-ancestors 'self'"), "{path}: {csp}");
    }
    // les erreurs d'authentification aussi
    let anon = app.req(Method::GET, "/api/playlists").no_auth().send().await;
    assert_eq!(anon.code, 401);
    assert!(anon.header("content-security-policy").is_some());
    assert_eq!(anon.header("www-authenticate").as_deref(), Some("Bearer"));
}

#[tokio::test]
async fn interactive_docs_are_not_exposed() {
    // Le port Rust n'embarque pas Swagger : /docs, /redoc et /openapi.json n'existent dans aucun environnement
    let dev = mk(|_| {}).await;
    let prod = mk(|s| s.env = "production".into()).await;
    for app in [&dev, &prod] {
        for path in ["/docs", "/redoc", "/openapi.json"] {
            assert_eq!(app.get(path).await.code, 404, "{path}");
        }
    }
}

#[test]
fn player_encodes_identifiers_in_urls() {
    let js = std::fs::read_to_string(repo_root().join("player/player.js")).unwrap();
    assert!(js.contains("encodeURIComponent"), "les valeurs de l'URL du lecteur doivent être encodées");
}
