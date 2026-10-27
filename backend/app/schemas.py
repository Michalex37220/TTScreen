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
