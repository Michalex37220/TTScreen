from __future__ import annotations

from datetime import datetime
from typing import Literal

from pydantic import BaseModel, Field


class ItemIn(BaseModel):
    type: Literal["text", "image", "video"]
    text: str | None = Field(default=None, max_length=2000)
    media_id: int | None = None
    duration_s: int | None = Field(default=None, ge=1, le=3600)


class ItemOut(ItemIn):
    id: int
    position: int


class PlaylistIn(BaseModel):
    name: str = Field(min_length=1, max_length=120)


class PlaylistOut(BaseModel):
    id: int
    name: str
    items: list[ItemOut]


class MediaOut(BaseModel):
    id: int
    original_name: str
    kind: str
    mime: str
    size: int
    sha256: str
    url: str


class VersionOut(BaseModel):
    id: int
    playlist_id: int
    number: int
    created_at: datetime
    manifest: dict


class ScreenIn(BaseModel):
    name: str = Field(min_length=1, max_length=120)


class ScreenOut(BaseModel):
    id: int
    name: str
    version_id: int | None
    online: bool
    last_seen: datetime | None
    has_token: bool


class ScreenCreated(ScreenOut):
    token: str  # affiché une seule fois


class AssignIn(BaseModel):
    version_id: int | None


# ---------- comptes, sessions et journal ----------
class LoginIn(BaseModel):
    """Deux modes de connexion : compte nommé (username + password) ou jeton admin
    historique (token seul, si aucun compte n'existe encore)."""

    username: str | None = Field(default=None, max_length=50)
    password: str | None = Field(default=None, max_length=255)
    token: str | None = Field(default=None, max_length=255)


class SessionOut(BaseModel):
    token: str                    # jeton de session à renvoyer en Bearer (affiché une seule fois)
    username: str
    role: str
    expires_at: datetime | None   # None pour le jeton admin historique (pas de session en base)


class MeOut(BaseModel):
    username: str
    role: str
    legacy: bool                  # True = authentifié avec le jeton admin historique


class UserIn(BaseModel):
    username: str = Field(min_length=3, max_length=50, pattern=r"^[A-Za-z0-9_.-]+$")
    password: str = Field(min_length=8, max_length=255)  # 8 caractères minimum : refuse les mots de passe triviaux
    role: Literal["viewer", "editor", "admin"] = "viewer"


class UserPatch(BaseModel):
    role: Literal["viewer", "editor", "admin"] | None = None
    active: bool | None = None
    password: str | None = Field(default=None, min_length=8, max_length=255)


class UserOut(BaseModel):
    id: int
    username: str
    role: str
    active: bool
    created_at: datetime
    last_login: datetime | None


class AuditOut(BaseModel):
    id: int
    created_at: datetime
    username: str
    role: str | None
    action: str
    target: str | None
    detail: str | None
    ip: str | None
