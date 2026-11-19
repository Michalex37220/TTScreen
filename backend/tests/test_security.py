"""Tests de non-régression des correctifs de l'audit de sécurité (F1 à F11 + points complémentaires)."""
import time
from pathlib import Path

import pytest
from conftest import MP4, PNG, TEST_TOKEN, auth_headers
from fastapi.testclient import TestClient

from app.config import Settings
from app.main import create_app

ROOT = Path(__file__).resolve().parents[2]
BEARER = {"Authorization": f"Bearer {TEST_TOKEN}"}


def mk(tmp_path, **kw):
    base = dict(database_url=f"sqlite:///{tmp_path}/s.db", media_dir=tmp_path / "media",
                admin_token=TEST_TOKEN, max_upload_mb=1, rate_limit_login=0, rate_limit_api=0)
    base.update(kw)
    return TestClient(create_app(Settings(**base)), raise_server_exceptions=False)


def audit(c):
    return c.get("/api/audit?limit=1000", headers=BEARER).json()


# ---------- F1 : plus de jeton par défaut ----------
def test_f1_no_default_token_and_weak_tokens_refused_everywhere(tmp_path):
    c = mk(tmp_path, admin_token="")  # défaut : jeton historique désactivé
    assert c.get("/api/auth/me", headers={"Authorization": "Bearer dev-admin-token"}).status_code == 401
    assert Settings().admin_token == ""
    for weak in ("dev-admin-token", "court", "x" * 32):
        with pytest.raises(RuntimeError):
            create_app(Settings(database_url=f"sqlite:///{tmp_path}/w.db", media_dir=tmp_path / "m", admin_token=weak))
    prod = dict(database_url=f"sqlite:///{tmp_path}/p.db", media_dir=tmp_path / "pm", env="production")
    with pytest.raises(RuntimeError):  # production sans aucun moyen de se connecter
        create_app(Settings(**prod))
    create_app(Settings(bootstrap_admin_user="root", bootstrap_admin_password="motdepasse8", **prod))


# ---------- F2 : pas de lecture du corps avant authentification ----------
def test_f2_upload_rejected_before_body_is_parsed(tmp_path, monkeypatch):
    import starlette.formparsers as fp
    parsed = []
    orig = fp.MultiPartParser.parse

    async def spy(self):
        parsed.append(1)
        return await orig(self)

    monkeypatch.setattr(fp.MultiPartParser, "parse", spy)
    c = mk(tmp_path)
    big = PNG + b"0" * (2 * 1024 * 1024)
    assert c.post("/api/media", files={"file": ("a.png", PNG, "image/png")}).status_code == 401
    assert c.post("/api/media", files={"file": ("a.png", PNG, "image/png")},
                  headers={"Authorization": "Bearer nope"}).status_code == 401
    assert parsed == []  # le corps n'a jamais été analysé
    # viewer : 403 avant lecture
    c.post("/api/users", headers=BEARER, json={"username": "vee", "password": "motdepasse8", "role": "viewer"})
    tok = c.post("/api/auth/login", json={"username": "vee", "password": "motdepasse8"}).json()["token"]
    assert c.post("/api/media", files={"file": ("a.png", PNG, "image/png")}, headers=auth_headers(tok)).status_code == 403
    # éditeur mais fichier trop gros : 413 sans lecture du corps
    assert c.post("/api/media", files={"file": ("b.png", big, "image/png")}, headers=BEARER).status_code == 413
    assert parsed == []
    # éditeur, fichier valide : accepté
    assert c.post("/api/media", files={"file": ("a.png", PNG, "image/png")}, headers=BEARER).status_code == 200
    assert parsed == [1]


# ---------- F3 : force brute sur les jetons / identifiants ----------
def test_f3_bad_bearers_are_throttled_and_logged_on_any_route(tmp_path):
    c = mk(tmp_path, rate_limit_authfail=5)
    codes = [c.get("/api/auth/me", headers=auth_headers(f"guess-{i}")).status_code for i in range(8)]
    assert codes == [401] * 5 + [429] * 3
    assert c.get("/api/auth/me", headers=BEARER).status_code == 429  # l'IP est bloquée pour la fenêtre
    c2 = mk(tmp_path / "x", rate_limit_authfail=5)
    for i in range(3):
        c2.get("/api/auth/me", headers=auth_headers(f"guess-{i}"))
    log = audit(c2)
    assert [a["action"] for a in log].count("auth_echouee") == 3
    assert "guess-0" not in str(log)  # jamais de jeton dans le journal


def test_f3_login_throttled_per_username(tmp_path):
    c = mk(tmp_path, rate_limit_login_user=3)
    c.post("/api/users", headers=BEARER, json={"username": "bob", "password": "motdepasse8"})
    bad = [c.post("/api/auth/login", json={"username": "bob", "password": "faux-faux-1"}).status_code for _ in range(4)]
    assert bad == [401, 401, 401, 429]
    assert c.post("/api/auth/login", json={"username": "bob", "password": "motdepasse8"}).status_code == 429
    assert c.post("/api/auth/login", json={"username": "alice", "password": "x"}).status_code == 401  # autre compte non impacté


# ---------- F4 : IP réelle derrière Caddy (configuration) ----------
def test_f4_proxy_headers_trusted_only_from_caddy():
    df = (ROOT / "deploy/Dockerfile").read_text()
    prod = (ROOT / "deploy/docker-compose.prod.yml").read_text()
    assert "--forwarded-allow-ips" in df and "FORWARDED_ALLOW_IPS" in prod
    import re
    api_block = prod.split("\n  api:")[1].split("\n  caddy:")[0]
    assert not re.search(r"^\s+ports:", api_block, re.M)  # l'API n'est pas publiée : seul Caddy l'atteint


# ---------- F5 : jeton historique ----------
def test_f5_legacy_token_disabled_once_named_admin_exists_and_name_reserved(tmp_path):
    c = mk(tmp_path)
    me = c.get("/api/auth/me", headers=BEARER).json()
    assert me["legacy"] is True and me["username"] == "legacy-admin"
    assert c.post("/api/users", headers=BEARER, json={"username": "legacy-admin", "password": "motdepasse8"}).status_code == 422
    assert c.post("/api/users", headers=BEARER, json={"username": "admin", "password": "motdepasse8", "role": "admin"}).status_code == 201
    assert c.get("/api/auth/me", headers=BEARER).status_code == 401  # le jeton ne marche plus
    assert c.post("/api/auth/login", json={"token": TEST_TOKEN}).status_code == 401
    tok = c.post("/api/auth/login", json={"username": "admin", "password": "motdepasse8"}).json()["token"]
    assert c.get("/api/auth/me", headers=auth_headers(tok)).json()["legacy"] is False


def test_f5_legacy_token_can_be_kept_explicitly(tmp_path):
    c = mk(tmp_path, legacy_token_always=True)
    c.post("/api/users", headers=BEARER, json={"username": "boss", "password": "motdepasse8", "role": "admin"})
    assert c.get("/api/auth/me", headers=BEARER).status_code == 200


# ---------- F6 : limiteur ----------
def test_f6_manifest_has_ip_quota_even_with_rotating_tokens(tmp_path):
    c = mk(tmp_path, rate_limit_api=5, rate_limit_authfail=0)
    codes = [c.get("/api/player/manifest", headers=auth_headers(f"fake-{i}")).status_code for i in range(10)]
    assert codes[:5] == [401] * 5 and codes[5:] == [429] * 5


def test_f6_limiter_gc_is_not_run_on_every_request():
    from app.ratelimit import RateLimiter
    lim = RateLimiter(60)
    for i in range(10100):
        lim.check(f"k{i}", 5)  # > 10 000 clés vivantes
    ident = id(lim._hits)
    t = time.perf_counter()
    for i in range(300):
        lim.check(f"z{i}", 5)
    assert id(lim._hits) == ident  # pas de reconstruction du dictionnaire
    assert time.perf_counter() - t < 0.5


# ---------- F7 : jetons non ASCII ----------
def test_f7_non_ascii_token_gives_401_not_500(tmp_path):
    c = mk(tmp_path)
    assert c.post("/api/auth/login", json={"token": "é☃"}).status_code == 401
    assert c.get("/api/auth/me", headers={"Authorization": "Bearer é".encode("latin-1")}).status_code == 401


# ---------- F8 : durée de login homogène ----------
def test_f8_password_always_verified_even_for_unknown_user(tmp_path, monkeypatch):
    c = mk(tmp_path)
    calls = []
    import app.api as api
    real = api.verify_password
    monkeypatch.setattr(api, "verify_password", lambda p, s: calls.append(s) or real(p, s))
    assert c.post("/api/auth/login", json={"username": "inconnu", "password": "whatever1"}).status_code == 401
    assert len(calls) == 1 and calls[0].startswith("pbkdf2_sha256$")  # haché factice vérifié


# ---------- F9 : amorçage ----------
def test_f9_bootstrap_never_overwrites_existing_account_unless_reset(tmp_path):
    kw = dict(admin_token="", bootstrap_admin_user="root", bootstrap_admin_password="motdepasse8")
    c = mk(tmp_path, **kw)
    tok = c.post("/api/auth/login", json={"username": "root", "password": "motdepasse8"}).json()["token"]
    uid = c.get("/api/users", headers=auth_headers(tok)).json()[0]["id"]
    assert c.patch(f"/api/users/{uid}", headers=auth_headers(tok), json={"password": "nouveau-mdp-9"}).status_code == 200
    c2 = mk(tmp_path, **kw)  # redémarrage, mêmes variables d'environnement
    assert c2.post("/api/auth/login", json={"username": "root", "password": "nouveau-mdp-9"}).status_code == 200
    assert c2.post("/api/auth/login", json={"username": "root", "password": "motdepasse8"}).status_code == 401
    c3 = mk(tmp_path, bootstrap_admin_reset=True, **kw)  # récupération explicite
    assert c3.post("/api/auth/login", json={"username": "root", "password": "motdepasse8"}).status_code == 200


# ---------- F10 : hygiène du dépôt ----------
def test_f10_gitignore_and_requirements():
    gi = (ROOT / ".gitignore").read_text()
    for pat in (".env", "backend/data/", "*.db", "__pycache__/", ".venv/"):
        assert pat in gi
    reqs = (ROOT / "backend/requirements.txt").read_text()
    assert "pytest" not in reqs and "httpx" not in reqs  # pas d'outils de test dans l'image de production
    assert all("==" in line for line in reqs.splitlines() if line and not line.startswith("#"))


# ---------- F11 : audit ----------
def test_f11_publish_and_audit_are_one_transaction(tmp_path, monkeypatch):
    c = mk(tmp_path)
    pid = c.post("/api/playlists", headers=BEARER, json={"name": "P"}).json()["id"]
    c.put(f"/api/playlists/{pid}/items", headers=BEARER, json=[{"type": "text", "text": "hi", "duration_s": 5}])
    import app.api as api
    real = api.log_action

    def boom(db, request, principal, action, *a, **k):
        if action == "playlist.publish":
            raise RuntimeError("panne")
        return real(db, request, principal, action, *a, **k)

    monkeypatch.setattr(api, "log_action", boom)
    assert c.post(f"/api/playlists/{pid}/publish", headers=BEARER).status_code == 500
    monkeypatch.setattr(api, "log_action", real)
    assert c.get(f"/api/playlists/{pid}/versions", headers=BEARER).json() == []  # rien de publié sans audit
    assert c.post(f"/api/playlists/{pid}/publish", headers=BEARER).status_code == 201
    assert any(a["action"] == "playlist.publish" for a in audit(c))


def test_f11_denied_access_is_audited(tmp_path):
    c = mk(tmp_path)
    c.post("/api/users", headers=BEARER, json={"username": "vee", "password": "motdepasse8"})
    tok = c.post("/api/auth/login", json={"username": "vee", "password": "motdepasse8"}).json()["token"]
    assert c.post("/api/playlists", headers=auth_headers(tok), json={"name": "x"}).status_code == 403
    denied = [a for a in audit(c) if a["action"] == "acces_refuse"]
    assert denied and denied[0]["username"] == "vee" and "/api/playlists" in denied[0]["target"]


# ---------- points complémentaires ----------
def test_long_filename_is_truncated_in_audit_column(tmp_path):
    c = mk(tmp_path)
    r = c.post("/api/media", headers=BEARER, files={"file": ("n" * 250 + ".png", PNG, "image/png")})
    assert r.status_code == 200
    assert all(len(a["target"] or "") <= 120 for a in audit(c))


def test_ids_and_lists_are_bounded(tmp_path):
    c = mk(tmp_path)
    assert c.get("/api/playlists/99999999999", headers=BEARER).status_code == 422
    assert c.get("/api/playlists/0", headers=BEARER).status_code == 422
    pid = c.post("/api/playlists", headers=BEARER, json={"name": "P"}).json()["id"]
    too_many = [{"type": "text", "text": "x", "duration_s": 5}] * 501
    assert c.put(f"/api/playlists/{pid}/items", headers=BEARER, json=too_many).status_code == 422
    assert c.put(f"/api/playlists/{pid}/items", headers=BEARER,
                 json=[{"type": "image", "media_id": 2**40}]).status_code == 422
    assert c.post("/api/screens/1/assign", headers=BEARER, json={"version_id": 2**40}).status_code == 422


def test_files_only_serves_application_generated_names(tmp_path):
    c = mk(tmp_path)
    m = c.post("/api/media", headers=BEARER, files={"file": ("a.png", PNG, "image/png")}).json()
    assert c.get(m["url"]).status_code == 200
    (tmp_path / "media" / "secret.txt").write_text("x")
    for bad in ("secret.txt", ".tmp", "..%2fsecret.txt", "A" * 64 + ".png", "0" * 64 + ".png"):
        assert c.get(f"/files/{bad}").status_code == 404


def test_security_headers_and_docs_hidden_in_production(tmp_path):
    c = mk(tmp_path)
    r = c.get("/api/health")
    assert r.headers["x-content-type-options"] == "nosniff" and "default-src 'self'" in r.headers["content-security-policy"]
    assert "frame-ancestors" in c.get("/admin/").headers["content-security-policy"]
    assert c.get("/docs").status_code == 200  # développement : documentation disponible
    p = mk(tmp_path / "prod", env="production")
    assert p.get("/docs").status_code == 404 and p.get("/openapi.json").status_code == 404


def test_player_encodes_fragment_ids_and_has_valid_title():
    js = (ROOT / "player/player.js").read_text()
    assert "encodeURIComponent" in js and 'params.get("screen")}/manifest' not in js
    assert "<title>" in (ROOT / "player/index.html").read_text()
