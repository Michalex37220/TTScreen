import pytest
from fastapi.testclient import TestClient

from app.config import Settings
from app.main import create_app

PNG = b"\x89PNG\r\n\x1a\n" + b"\x00" * 64
MP4 = b"\x00\x00\x00\x18ftypmp42" + b"\x00" * 64
ADMIN = {"Authorization": "Bearer test-admin"}


@pytest.fixture
def client(tmp_path):
    s = Settings(database_url=f"sqlite:///{tmp_path}/t.db", media_dir=tmp_path / "media",
                 admin_token="test-admin", max_upload_mb=1)
    with TestClient(create_app(s)) as c:
        c.headers.update(ADMIN)
        yield c


def upload(client, name, data, mime="application/octet-stream"):
    return client.post("/api/media", files={"file": (name, data, mime)})
