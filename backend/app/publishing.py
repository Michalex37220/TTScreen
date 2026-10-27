from __future__ import annotations

from sqlalchemy import func, select
from sqlalchemy.orm import Session

from .models import Playlist, PlaylistVersion


def build_manifest(playlist: Playlist) -> tuple[dict, list[int]]:
    items, media_ids = [], []
    for it in playlist.items:
        entry: dict = {"type": it.type, "duration_s": it.duration_s}
        if it.type == "text":
            entry["text"] = it.text
        else:
            m = it.media
            entry["media"] = {
                "id": m.id, "url": f"/files/{m.stored_name}", "sha256": m.sha256,
                "size": m.size, "mime": m.mime,
            }
            media_ids.append(m.id)
        items.append(entry)
    return {"playlist_id": playlist.id, "name": playlist.name, "items": items}, sorted(set(media_ids))


def publish(db: Session, playlist: Playlist) -> PlaylistVersion:
    if not playlist.items:
        raise ValueError("Playlist vide")
    manifest, media_ids = build_manifest(playlist)
    last = db.scalar(select(func.max(PlaylistVersion.number)).where(PlaylistVersion.playlist_id == playlist.id))
    version = PlaylistVersion(
        playlist_id=playlist.id, number=(last or 0) + 1, manifest=manifest, media_ids=media_ids
    )
    db.add(version)
    db.commit()
    return version


def player_payload(version: PlaylistVersion) -> dict:
    return {"version_id": version.id, "number": version.number, **version.manifest}
