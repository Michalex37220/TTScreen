from conftest import ADMIN, MP4, PNG, upload


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
