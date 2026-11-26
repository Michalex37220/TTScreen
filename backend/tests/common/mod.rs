//! Outils communs aux tests d'intégration : un client en mémoire qui appelle le vrai routeur
//! (équivalent du `TestClient` + `conftest.py` de la version Python).

#![allow(dead_code)]

use axum::body::{Body, Bytes};
use axum::extract::ConnectInfo;
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, Request};
use axum::Router;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;
use ttscreen::{build_router, build_state, AppState, Settings};

/// 24 caractères variés au minimum (exigé dans tous les environnements).
pub const TEST_TOKEN: &str = "test-admin-token-0123456789abcdef";

pub fn png() -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
    v.extend(vec![0u8; 64]);
    v
}

pub fn mp4() -> Vec<u8> {
    let mut v = b"\x00\x00\x00\x18ftypmp42".to_vec();
    v.extend(vec![0u8; 64]);
    v
}

/// Racine du dépôt (le crate est dans `backend/`).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Réglages de test : mêmes valeurs que `conftest.py` (jeton valide, 1 Mo max, PBKDF2 rapide).
pub fn base_settings(dir: &Path) -> Settings {
    Settings {
        database_url: format!("sqlite:///{}", dir.join("t.db").display()),
        media_dir: dir.join("media"),
        admin_token: TEST_TOKEN.to_string(),
        max_upload_mb: 1,
        // les tests historiques créent des comptes admin tout en gardant le jeton
        legacy_token_always: true,
        // 1 000 itérations au lieu de 600 000 ; la vérification lit le nombre stocké dans le haché
        pbkdf2_iterations: 1000,
        // les plannings des tests sont évalués en UTC : pas de dépendance au fuseau de la machine
        timezone: "UTC".to_string(),
        ..Settings::default()
    }
}

pub struct TestApp {
    pub router: Router,
    pub state: AppState,
    /// Jeton Bearer envoyé par défaut (comme `client.headers["Authorization"]`) ; `None` = anonyme.
    pub auth: Option<String>,
    dir: Option<TempDir>,
    dir_path: PathBuf,
}

impl TestApp {
    pub async fn new() -> TestApp {
        TestApp::with(|_| {}).await
    }

    /// Application neuve (base SQLite temporaire) dont les réglages sont ajustés par `tweak`.
    pub async fn with(tweak: impl FnOnce(&mut Settings)) -> TestApp {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let mut settings = base_settings(dir.path());
        tweak(&mut settings);
        let path = dir.path().to_path_buf();
        TestApp::build(settings, Some(dir), path).await
    }

    /// « Redémarrage » : nouvelle application sur les mêmes données (même base, mêmes médias).
    pub async fn restart(&self, tweak: impl FnOnce(&mut Settings)) -> TestApp {
        let mut settings = (*self.state.settings).clone();
        tweak(&mut settings);
        TestApp::build(settings, None, self.dir_path.clone()).await
    }

    async fn build(settings: Settings, dir: Option<TempDir>, dir_path: PathBuf) -> TestApp {
        let state = build_state(settings).await.expect("démarrage de l'application");
        TestApp {
            router: build_router(state.clone()),
            state,
            auth: Some(TEST_TOKEN.to_string()),
            dir,
            dir_path,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir_path
    }

    pub fn media_dir(&self) -> PathBuf {
        self.dir_path.join("media")
    }

    pub fn req(&self, method: Method, path: &str) -> Req<'_> {
        Req {
            app: self,
            method,
            path: path.to_string(),
            token: self.auth.clone(),
            headers: Vec::new(),
            body: Vec::new(),
            content_type: None,
            tracker: None,
            peer: "127.0.0.1:40000".parse().unwrap(),
            send_length: true,
            claimed_length: None,
        }
    }

    pub async fn get(&self, path: &str) -> Resp {
        self.req(Method::GET, path).send().await
    }
    pub async fn post(&self, path: &str) -> Resp {
        self.req(Method::POST, path).send().await
    }
    pub async fn post_json(&self, path: &str, body: &Value) -> Resp {
        self.req(Method::POST, path).json(body).send().await
    }
    pub async fn put_json(&self, path: &str, body: &Value) -> Resp {
        self.req(Method::PUT, path).json(body).send().await
    }
    pub async fn patch_json(&self, path: &str, body: &Value) -> Resp {
        self.req(Method::PATCH, path).json(body).send().await
    }
    pub async fn delete(&self, path: &str) -> Resp {
        self.req(Method::DELETE, path).send().await
    }
    pub async fn upload(&self, name: &str, data: &[u8]) -> Resp {
        self.req(Method::POST, "/api/media").multipart(name, data).send().await
    }
    /// GET avec un autre jeton que celui par défaut.
    pub async fn get_as(&self, token: &str, path: &str) -> Resp {
        self.req(Method::GET, path).bearer(token).send().await
    }
}

pub struct Req<'a> {
    app: &'a TestApp,
    method: Method,
    path: String,
    token: Option<String>,
    headers: Vec<(HeaderName, HeaderValue)>,
    body: Vec<u8>,
    content_type: Option<String>,
    tracker: Option<Arc<AtomicBool>>,
    peer: SocketAddr,
    send_length: bool,
    claimed_length: Option<u64>,
}

impl<'a> Req<'a> {
    pub fn no_auth(mut self) -> Self {
        self.token = None;
        self
    }
    pub fn bearer(mut self, token: &str) -> Self {
        self.token = Some(token.to_string());
        self
    }
    pub fn json(mut self, value: &Value) -> Self {
        self.body = serde_json::to_vec(value).unwrap();
        self.content_type = Some("application/json".into());
        self
    }
    pub fn raw(mut self, content_type: &str, body: &[u8]) -> Self {
        self.body = body.to_vec();
        self.content_type = Some(content_type.to_string());
        self
    }
    pub fn multipart(self, filename: &str, data: &[u8]) -> Self {
        self.multipart_field("file", filename, data)
    }
    pub fn multipart_field(mut self, field: &str, filename: &str, data: &[u8]) -> Self {
        let boundary = "----ttscreen-test-boundary";
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\n\
                 Content-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        self.body = body;
        self.content_type = Some(format!("multipart/form-data; boundary={boundary}"));
        self
    }
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((HeaderName::from_bytes(name.as_bytes()).unwrap(), HeaderValue::from_str(value).unwrap()));
        self
    }
    pub fn header_bytes(mut self, name: &str, value: &[u8]) -> Self {
        self.headers.push((HeaderName::from_bytes(name.as_bytes()).unwrap(), HeaderValue::from_bytes(value).unwrap()));
        self
    }
    /// Adresse de la connexion TCP directe (le « pair »).
    pub fn peer(mut self, addr: &str) -> Self {
        self.peer = addr.parse().unwrap();
        self
    }
    /// Le corps est envoyé en flux ; `flag` passe à `true` dès que le serveur commence à le lire.
    pub fn track_body(mut self, flag: Arc<AtomicBool>) -> Self {
        self.tracker = Some(flag);
        self
    }
    pub fn no_content_length(mut self) -> Self {
        self.send_length = false;
        self
    }
    pub fn claim_length(mut self, n: u64) -> Self {
        self.claimed_length = Some(n);
        self
    }

    pub async fn send(self) -> Resp {
        let mut builder = Request::builder().method(self.method.clone()).uri(self.path.as_str());
        if let Some(t) = &self.token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        for (k, v) in &self.headers {
            builder = builder.header(k, v);
        }
        if let Some(ct) = &self.content_type {
            builder = builder.header(header::CONTENT_TYPE, ct.as_str());
        }
        // Un vrai client (hyper) envoie Content-Length ; `oneshot` ne l'ajoute pas tout seul
        let length = self.claimed_length.or(if self.send_length && !self.body.is_empty() {
            Some(self.body.len() as u64)
        } else {
            None
        });
        if let Some(n) = length {
            builder = builder.header(header::CONTENT_LENGTH, n.to_string());
        }
        let body = match self.tracker {
            Some(flag) => {
                let data = Bytes::from(self.body);
                Body::from_stream(futures_util::stream::once(async move {
                    flag.store(true, Ordering::SeqCst);
                    Ok::<Bytes, std::convert::Infallible>(data)
                }))
            }
            None => Body::from(self.body),
        };
        let mut request = builder.body(body).expect("requête valide");
        request.extensions_mut().insert(ConnectInfo(self.peer));
        let response = self.app.router.clone().oneshot(request).await.expect("routeur infaillible");
        let (parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.expect("corps de réponse");
        Resp { code: parts.status.as_u16(), headers: parts.headers, body: bytes }
    }
}

pub struct Resp {
    pub code: u16,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl Resp {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
    }
    /// Message d'erreur `detail` (chaîne).
    pub fn detail(&self) -> String {
        self.json()["detail"].as_str().unwrap_or_default().to_string()
    }
}

// ---------- helpers métier ----------
pub async fn make_user(app: &TestApp, username: &str, password: &str, role: &str) -> Resp {
    app.post_json("/api/users", &json!({"username": username, "password": password, "role": role})).await
}

pub async fn login(app: &TestApp, username: &str, password: &str) -> Resp {
    app.req(Method::POST, "/api/auth/login")
        .json(&json!({"username": username, "password": password}))
        .send()
        .await
}

/// Crée un compte et renvoie son jeton de session.
pub async fn session_for(app: &TestApp, username: &str, role: &str) -> String {
    let r = make_user(app, username, "motdepasse8", role).await;
    assert_eq!(r.code, 201, "création du compte {username} : {}", r.text());
    let l = login(app, username, "motdepasse8").await;
    assert_eq!(l.code, 200, "connexion de {username} : {}", l.text());
    l.json()["token"].as_str().unwrap().to_string()
}

/// Playlist « Hall » : un texte, une image, une vidéo. Renvoie (id, image, vidéo).
pub async fn mk_playlist(app: &TestApp) -> (i64, Value, Value) {
    let pid = app.post_json("/api/playlists", &json!({"name": "Hall"})).await.json()["id"].as_i64().unwrap();
    let img = app.upload("a.png", &png()).await.json();
    let vid = app.upload("b.mp4", &mp4()).await.json();
    let r = app
        .put_json(
            &format!("/api/playlists/{pid}/items"),
            &json!([
                {"type": "text", "text": "Bienvenue", "duration_s": 5},
                {"type": "image", "media_id": img["id"]},
                {"type": "video", "media_id": vid["id"]},
            ]),
        )
        .await;
    assert_eq!(r.code, 200, "{}", r.text());
    (pid, img, vid)
}

pub async fn audit_entries(app: &TestApp) -> Vec<Value> {
    app.get("/api/audit?limit=1000").await.json().as_array().cloned().unwrap_or_default()
}

pub fn actions(entries: &[Value]) -> Vec<String> {
    entries.iter().map(|e| e["action"].as_str().unwrap_or_default().to_string()).collect()
}

/// Publie une playlist à un seul texte ; renvoie l'identifiant de la version.
pub async fn publish_text(app: &TestApp, name: &str, text: &str) -> i64 {
    let pid = app.post_json("/api/playlists", &json!({"name": name})).await.json()["id"].as_i64().unwrap();
    let r = app.put_json(&format!("/api/playlists/{pid}/items"), &json!([{"type": "text", "text": text}])).await;
    assert_eq!(r.code, 200, "{}", r.text());
    let v = app.post(&format!("/api/playlists/{pid}/publish")).await;
    assert_eq!(v.code, 201, "{}", v.text());
    v.json()["id"].as_i64().unwrap()
}

pub async fn new_screen(app: &TestApp, name: &str) -> Value {
    let r = app.post_json("/api/screens", &json!({"name": name})).await;
    assert_eq!(r.code, 201, "{}", r.text());
    r.json()
}

pub async fn new_group(app: &TestApp, name: &str) -> Value {
    let r = app.post_json("/api/groups", &json!({"name": name})).await;
    assert_eq!(r.code, 201, "{}", r.text());
    r.json()
}

/// Texte du premier élément du manifeste reçu par l'écran (`None` si 404 : rien à afficher).
pub async fn shown_text(app: &TestApp, screen_token: &str) -> Option<String> {
    let r = app.get_as(screen_token, "/api/player/manifest").await;
    match r.code {
        200 => r.json()["items"][0]["text"].as_str().map(str::to_string),
        404 => None,
        other => panic!("manifeste : code inattendu {other} : {}", r.text()),
    }
}

pub async fn screen_by_id(app: &TestApp, id: &Value) -> Value {
    app.get("/api/screens").await.json().as_array().unwrap().iter().find(|s| s["id"] == *id).cloned().expect("écran")
}
