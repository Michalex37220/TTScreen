from conftest import ADMIN, MP4, PNG, auth_headers, login, make_user, upload


def mk_playlist(client):
    pid = client.post("/api/playlists", json={"name": "Hall"}).json()["id"]
    img = upload(client, "a.png", PNG).json()
    vid = upload(client, "b.mp4", MP4).json()
    client.put(f"/api/playlists/{pid}/items", json=[
        {"type": "text", "text": "Bienvenue", "duration_s": 5},
        {"type": "image", "media_id": img["id"]},
        {"type": "video", "media_id": vid["id"]},
    ])
    return pid, img, vid


def test_admin_requires_auth(client):
    client.headers.pop("Authorization")
    assert client.get("/api/playlists").status_code == 401
    assert client.get("/api/health").status_code == 200
    client.headers["Authorization"] = "Bearer wrong"
    assert client.get("/api/playlists").status_code == 401


def test_upload_validation(client):
    assert upload(client, "x.png", b"pas une image").status_code == 415
    assert upload(client, "vide.png", b"").status_code in (400, 415)
    assert upload(client, "gros.png", PNG + b"\x00" * (2 * 1024 * 1024)).status_code == 413
    ok = upload(client, "a.png", PNG)
    assert ok.status_code == 200 and ok.json()["kind"] == "image"
    assert upload(client, "copie.png", PNG).json()["id"] == ok.json()["id"]  # dédoublonnage
    assert client.get(ok.json()["url"]).content == PNG


def test_item_validation(client):
    pid = client.post("/api/playlists", json={"name": "P"}).json()["id"]
    img = upload(client, "a.png", PNG).json()
    put = lambda body: client.put(f"/api/playlists/{pid}/items", json=body).status_code
    assert put([{"type": "text", "text": "  "}]) == 422
    assert put([{"type": "image", "media_id": 999}]) == 422
    assert put([{"type": "video", "media_id": img["id"]}]) == 422  # mauvais type
    assert put([{"type": "text", "text": "ok", "duration_s": 0}]) == 422
    assert put([{"type": "image", "media_id": img["id"]}]) == 200


def test_publish_is_immutable(client):
    pid, img, _ = mk_playlist(client)
    v1 = client.post(f"/api/playlists/{pid}/publish").json()
    assert v1["number"] == 1 and len(v1["manifest"]["items"]) == 3
    client.put(f"/api/playlists/{pid}/items", json=[{"type": "text", "text": "Autre"}])
    v2 = client.post(f"/api/playlists/{pid}/publish").json()
    assert v2["number"] == 2
    old = client.get(f"/api/versions/{v1['id']}").json()
    assert len(old["items"]) == 3  # la v1 n'a pas bougé
    assert old["items"][1]["media"]["sha256"] == img["sha256"]


def test_publish_empty_refused(client):
    pid = client.post("/api/playlists", json={"name": "Vide"}).json()["id"]
    assert client.post(f"/api/playlists/{pid}/publish").status_code == 422


def test_screen_flow_and_rollback(client):
    pid, _, _ = mk_playlist(client)
    v1 = client.post(f"/api/playlists/{pid}/publish").json()
    client.put(f"/api/playlists/{pid}/items", json=[{"type": "text", "text": "V2"}])
    v2 = client.post(f"/api/playlists/{pid}/publish").json()
    sc = client.post("/api/screens", json={"name": "Entrée"}).json()
    tok = {"Authorization": f"Bearer {sc['token']}"}

    assert client.get("/api/player/manifest", headers=tok).status_code == 404  # rien d'affecté
    client.post(f"/api/screens/{sc['id']}/assign", json={"version_id": v2["id"]})
    assert client.get("/api/player/manifest", headers=tok).json()["number"] == 2
    client.post(f"/api/screens/{sc['id']}/assign", json={"version_id": v1["id"]})  # rollback
    assert client.get("/api/player/manifest", headers=tok).json()["number"] == 1
    assert client.get("/api/screens").json()[0]["online"] is True


def test_screen_token_rotation_and_revocation(client):
    sc = client.post("/api/screens", json={"name": "S"}).json()
    old = {"Authorization": f"Bearer {sc['token']}"}
    new_tok = client.post(f"/api/screens/{sc['id']}/rotate-token").json()["token"]
    assert client.get("/api/player/manifest", headers=old).status_code == 401
    new = {"Authorization": f"Bearer {new_tok}"}
    assert client.get("/api/player/manifest", headers=new).status_code == 404  # authentifié, pas de version
    client.post(f"/api/screens/{sc['id']}/revoke-token")
    assert client.get("/api/player/manifest", headers=new).status_code == 401
    # un jeton d'écran ne donne aucun accès admin
    assert client.get("/api/playlists", headers=new).status_code == 401


def test_media_delete_protection(client):
    pid, img, _ = mk_playlist(client)
    assert client.delete(f"/api/media/{img['id']}").status_code == 409
    client.post(f"/api/playlists/{pid}/publish")
    client.put(f"/api/playlists/{pid}/items", json=[{"type": "text", "text": "x"}])
    assert client.delete(f"/api/media/{img['id']}").status_code == 409  # encore dans la v1


def test_screen_preview_manifest(client):
    pid, _, _ = mk_playlist(client)
    v = client.post(f"/api/playlists/{pid}/publish").json()
    sc = client.post("/api/screens", json={"name": "S"}).json()
    assert client.get(f"/api/screens/{sc['id']}/manifest").status_code == 404
    client.post(f"/api/screens/{sc['id']}/assign", json={"version_id": v["id"]})
    assert client.get(f"/api/screens/{sc['id']}/manifest").json()["number"] == 1
    assert client.get("/api/screens").json()[0]["online"] is False  # la prévisualisation ne simule pas une connexion
    client.headers.pop("Authorization")
    assert client.get(f"/api/screens/{sc['id']}/manifest").status_code == 401


def test_player_page_served(client):
    r = client.get("/player/")
    assert r.status_code == 200 and "player.js" in r.text


def test_draft_preview_and_admin_page(client):
    pid, _, _ = mk_playlist(client)
    d = client.get(f"/api/playlists/{pid}/preview").json()
    assert d["version_id"] is None and len(d["items"]) == 3
    assert client.get(f"/api/playlists/{pid}/versions").json() == []  # rien n'a été publié
    assert "app.js" in client.get("/admin/").text
    client.headers.pop("Authorization")
    assert client.get(f"/api/playlists/{pid}/preview").status_code == 401


def test_production_refuses_weak_admin_token(tmp_path):
    import pytest
    from app.config import Settings
    from app.main import create_app
    base = dict(database_url=f"sqlite:///{tmp_path}/p.db", media_dir=tmp_path / "m", env="production")
    with pytest.raises(RuntimeError):
        create_app(Settings(admin_token="dev-admin-token", **base))
    with pytest.raises(RuntimeError):
        create_app(Settings(admin_token="court", **base))
    create_app(Settings(admin_token="x" * 32, **base))  # accepté


# ---------- comptes, rôles et sessions ----------
def test_password_hashing():
    from app import auth
    h = auth.hash_password("secret123")
    assert h.startswith("pbkdf2_sha256$") and h != "secret123"  # jamais de stockage en clair
    assert auth.verify_password("secret123", h)
    assert not auth.verify_password("secret124", h)
    assert not auth.verify_password("secret123", "haché mal formé")


def test_login_logout_and_me(client):
    assert make_user(client, "emma", "motdepasse8", "editor").status_code == 201
    # mauvais mot de passe : 401 et tentative journalisée
    assert login(client, "emma", "mauvais123").status_code == 401
    r = login(client, "emma", "motdepasse8")
    assert r.status_code == 200 and r.json()["role"] == "editor"
    tok = r.json()["token"]
    me = client.get("/api/auth/me", headers=auth_headers(tok))
    assert me.json() == {"username": "emma", "role": "editor", "legacy": False}
    # la session ouvre les routes d'administration (lecture), mais pas les routes admin
    assert client.get("/api/playlists", headers=auth_headers(tok)).status_code == 200
    assert client.get("/api/users", headers=auth_headers(tok)).status_code == 403
    # déconnexion : le jeton cesse immédiatement de fonctionner
    assert client.post("/api/auth/logout", headers=auth_headers(tok)).status_code == 204
    assert client.get("/api/auth/me", headers=auth_headers(tok)).status_code == 401
    # le jeton admin historique reste accepté en login (mode token), avec le rôle admin et sans expiration
    r = client.post("/api/auth/login", json={"token": "test-admin"})
    assert r.status_code == 200 and r.json()["role"] == "admin" and r.json()["expires_at"] is None
    me = client.get("/api/auth/me", headers=auth_headers(r.json()["token"]))
    assert me.json()["legacy"] is True
    assert client.post("/api/auth/login", json={"token": "faux-jeton"}).status_code == 401


def test_roles_enforcement(client):
    make_user(client, "lea", "motdepasse8", "viewer")
    make_user(client, "yan", "motdepasse8", "editor")
    viewer = auth_headers(login(client, "lea", "motdepasse8").json()["token"])
    editor = auth_headers(login(client, "yan", "motdepasse8").json()["token"])
    # viewer : lecture autorisée, écriture refusée (403 et pas 401 : il est bien authentifié)
    assert client.get("/api/media", headers=viewer).status_code == 200
    assert client.post("/api/playlists", headers=viewer, json={"name": "x"}).status_code == 403
    assert client.get("/api/audit", headers=viewer).status_code == 403
    # editor : peut créer une playlist et publier (422 ici car vide, mais pas 403), mais pas gérer les comptes
    pid = client.post("/api/playlists", headers=editor, json={"name": "P"}).json()["id"]
    assert client.post(f"/api/playlists/{pid}/publish", headers=editor).status_code == 422
    assert client.get("/api/users", headers=editor).status_code == 403
    assert client.post("/api/users", headers=editor,
                       json={"username": "z", "password": "motdepasse8"}).status_code == 403
    # un jeton d'écran n'a toujours aucun accès aux routes d'administration
    sc = client.post("/api/screens", json={"name": "S"}).json()
    assert client.get("/api/playlists", headers=auth_headers(sc["token"])).status_code == 401


def test_user_management_rules(client):
    assert make_user(client, "emma", "motdepasse8", "admin").status_code == 201
    assert make_user(client, "emma", "motdepasse8").status_code == 409  # nom déjà pris
    assert client.post("/api/users", json={"username": "court", "password": "short"}).status_code == 422  # mdp < 8
    emma = next(u for u in client.get("/api/users").json() if u["username"] == "emma")
    # le dernier administrateur actif ne peut ni être supprimé ni désactivé
    assert client.delete(f"/api/users/{emma['id']}").status_code == 409
    assert client.patch(f"/api/users/{emma['id']}", json={"active": False}).status_code == 409
    # un deuxième admin permet de modifier le premier ; la désactivation révoque ses sessions
    make_user(client, "boss", "motdepasse8", "admin")
    boss = next(u for u in client.get("/api/users").json() if u["username"] == "boss")
    tok = login(client, "emma", "motdepasse8").json()["token"]
    assert client.get("/api/auth/me", headers=auth_headers(tok)).status_code == 200
    assert client.patch(f"/api/users/{emma['id']}", json={"active": False}).status_code == 200
    assert client.get("/api/auth/me", headers=auth_headers(tok)).status_code == 401  # session révoquée
    assert login(client, "emma", "motdepasse8").status_code == 401  # compte désactivé : plus de connexion
    # on ne peut pas se supprimer soi-même (agissons en tant que boss via sa session)
    boss_tok = login(client, "boss", "motdepasse8").json()["token"]
    assert client.delete(f"/api/users/{boss['id']}", headers=auth_headers(boss_tok)).status_code == 409
    # mais boss peut supprimer emma
    assert client.delete(f"/api/users/{emma['id']}", headers=auth_headers(boss_tok)).status_code == 204
    # supprimer un compte non-admin : OK ; compte inconnu : 404
    make_user(client, "temp", "motdepasse8", "viewer")
    temp = next(u for u in client.get("/api/users").json() if u["username"] == "temp")
    assert client.delete(f"/api/users/{temp['id']}").status_code == 204
    assert client.delete("/api/users/9999").status_code == 404


# ---------- journal des opérations sensibles ----------
def test_audit_log(client):
    make_user(client, "emma", "motdepasse8", "editor")
    login(client, "emma", "motdepasse8")  # connexion journalisée
    login(client, "emma", "mauvais123")   # échec journalisé aussi
    pid, _, _ = mk_playlist(client)
    v = client.post(f"/api/playlists/{pid}/publish").json()
    sc = client.post("/api/screens", json={"name": "Entrée"}).json()
    client.post(f"/api/screens/{sc['id']}/assign", json={"version_id": v["id"]})
    client.post(f"/api/screens/{sc['id']}/rotate-token")
    entries = client.get("/api/audit").json()
    actions = {e["action"] for e in entries}
    # chaque opération sensible est tracée, avec l'auteur et l'IP
    assert {"connexion", "connexion_echouee", "user.create", "media.upload", "playlist.create",
            "playlist.items", "playlist.publish", "screen.create", "screen.assign",
            "screen.rotate_token"} <= actions
    connexion = next(e for e in entries if e["action"] == "connexion")
    assert connexion["username"] == "emma" and connexion["ip"] == "testclient"
    publish = next(e for e in entries if e["action"] == "playlist.publish")
    assert publish["detail"] == f"v{v['number']}" and publish["target"] == f"playlist:{pid}"
    # filtre par action + pagination
    assert len(client.get("/api/audit?action=connexion").json()) == 1
    assert len(client.get("/api/audit?limit=5&offset=0").json()) == 5
    # le journal est trié du plus récent au plus ancien
    assert entries[0]["id"] > entries[-1]["id"]


# ---------- limitation de débit ----------
def test_rate_limit(tmp_path):
    from fastapi.testclient import TestClient
    from app.config import Settings
    from app.main import create_app
    # Limites très basses pour le test : 5 requêtes / minute sur /api (hors health) et 2 logins / minute
    s = Settings(database_url=f"sqlite:///{tmp_path}/r.db", media_dir=tmp_path / "m",
                 admin_token="test-admin", rate_limit_api=5, rate_limit_login=2, rate_window_s=60)
    with TestClient(create_app(s)) as c:
        c.headers.update(ADMIN)
        codes = [c.get("/api/playlists").status_code for _ in range(7)]
        assert codes == [200] * 5 + [429, 429]  # au-delà de la limite : 429
        r = c.get("/api/playlists")
        assert r.status_code == 429 and "Retry-After" in r.headers
        assert r.json()["detail"] == "Trop de requêtes, réessayez plus tard."
        assert c.get("/api/health").status_code == 200  # la sonde de santé n'est jamais limitée
        # limite de connexion indépendante et plus stricte (anti force brute)
        logins = [c.post("/api/auth/login", json={"username": "x", "password": "y"}).status_code
                  for _ in range(3)]
        assert logins == [401, 401, 429]


def test_rate_limit_player_per_token(tmp_path):
    from fastapi.testclient import TestClient
    from app.config import Settings
    from app.main import create_app
    # La limite du manifeste est comptée par jeton d'écran, pas par IP :
    # deux écrans derrière la même IP ont chacun leur propre quota
    s = Settings(database_url=f"sqlite:///{tmp_path}/r.db", media_dir=tmp_path / "m",
                 admin_token="test-admin", rate_limit_api=1000, rate_limit_player=3, rate_window_s=60)
    with TestClient(create_app(s)) as c:
        c.headers.update(ADMIN)
        t1 = c.post("/api/screens", json={"name": "A"}).json()["token"]
        t2 = c.post("/api/screens", json={"name": "B"}).json()["token"]
        h1, h2 = auth_headers(t1), auth_headers(t2)
        assert [c.get("/api/player/manifest", headers=h1).status_code for _ in range(3)] == [404] * 3
        assert c.get("/api/player/manifest", headers=h1).status_code == 429  # quota de l'écran 1 épuisé
        assert c.get("/api/player/manifest", headers=h2).status_code == 404  # l'écran 2 a encore son quota
