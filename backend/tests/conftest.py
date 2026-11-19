import pytest
from fastapi.testclient import TestClient

from app.config import Settings
from app.main import create_app

PNG = b"\x89PNG\r\n\x1a\n" + b"\x00" * 64
MP4 = b"\x00\x00\x00\x18ftypmp42" + b"\x00" * 64
TEST_TOKEN = "test-admin-token-0123456789abcdef"  # >= 24 caractères variés (exigé dans tous les environnements)
ADMIN = {"Authorization": f"Bearer {TEST_TOKEN}"}


@pytest.fixture(autouse=True)
def _fast_hash(monkeypatch):
    # Itérations PBKDF2 réduites en test pour rester rapide : le hachage réel fait
    # 600 000 itérations (voir app/auth.py) et verify_password lit le nombre stocké dans le haché.
    monkeypatch.setattr("app.auth.PBKDF2_ITERATIONS", 1000)


@pytest.fixture
def client(tmp_path):
    s = Settings(database_url=f"sqlite:///{tmp_path}/t.db", media_dir=tmp_path / "media",
                 admin_token=TEST_TOKEN, max_upload_mb=1,
                 legacy_token_always=True)  # les tests historiques créent des comptes admin tout en gardant le jeton
    with TestClient(create_app(s)) as c:
        c.headers.update(ADMIN)
        yield c


def upload(client, name, data, mime="application/octet-stream"):
    return client.post("/api/media", files={"file": (name, data, mime)})


def make_user(client, username, password="motdepasse8", role="viewer"):
    """Crée un compte via l'API d'administration (le fixture client est déjà admin)."""
    return client.post("/api/users", json={"username": username, "password": password, "role": role})


def login(client, username, password):
    """Connexion par compte nommé ; renvoie la réponse (200 attendu si identifiants bons)."""
    return client.post("/api/auth/login", json={"username": username, "password": password})


def auth_headers(token):
    return {"Authorization": f"Bearer {token}"}
