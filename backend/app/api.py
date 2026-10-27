from __future__ import annotations

import secrets
from datetime import timedelta

from fastapi import APIRouter, Depends, File, HTTPException, Request, UploadFile
from fastapi.responses import FileResponse, RedirectResponse
from sqlalchemy import select
from sqlalchemy.orm import Session

from . import publishing
from .deps import get_db, hash_token, require_admin, require_screen
from .models import Media, Playlist, PlaylistItem, PlaylistVersion, Screen, utcnow
from .schemas import (AssignIn, ItemIn, MediaOut, PlaylistIn, PlaylistOut, ScreenCreated,
                      ScreenIn, ScreenOut, VersionOut)
from .storage import save_upload

ONLINE_WINDOW = timedelta(seconds=60)
DEFAULT_DURATION = 10

admin = APIRouter(prefix="/api", dependencies=[Depends(require_admin)])
player = APIRouter(prefix="/api/player")
public = APIRouter()


# ---------- helpers ----------
def _media_out(m: Media) -> MediaOut:
    return MediaOut(id=m.id, original_name=m.original_name, kind=m.kind, mime=m.mime,
                    size=m.size, sha256=m.sha256, url=f"/files/{m.stored_name}")


def _screen_out(s: Screen) -> dict:
    online = bool(s.last_seen and utcnow() - s.last_seen < ONLINE_WINDOW)
    return dict(id=s.id, name=s.name, version_id=s.version_id, online=online,
                last_seen=s.last_seen, has_token=s.token_hash is not None)


def _playlist(db: Session, pid: int) -> Playlist:
    p = db.get(Playlist, pid)
    if p is None:
        raise HTTPException(404, "Playlist introuvable")
    return p


# ---------- public ----------
@public.get("/", include_in_schema=False)
def root():
    return RedirectResponse("/admin/")


@public.get("/api/health")
def health():
    return {"status": "ok"}


@public.get("/files/{stored_name}")
def get_file(stored_name: str, request: Request):
    # Noms = SHA-256 + extension (non devinables). À durcir par URL signées à l'étape 7.
    if "/" in stored_name or ".." in stored_name:
        raise HTTPException(404)
    path = request.app.state.settings.media_dir / stored_name
    if not path.is_file():
        raise HTTPException(404)
    return FileResponse(path, headers={"X-Content-Type-Options": "nosniff"})


# ---------- médias ----------
@admin.post("/media", response_model=MediaOut)
def upload_media(request: Request, file: UploadFile = File(...), db: Session = Depends(get_db)):
    s = request.app.state.settings
    info = save_upload(file, s.media_dir, s.max_upload_mb * 1024 * 1024)
    existing = db.scalar(select(Media).where(Media.sha256 == info["sha256"]))
    if existing:
        return _media_out(existing)
    name = (file.filename or "fichier")[:255]
    m = Media(original_name=name, **info)
    db.add(m)
    db.commit()
    return _media_out(m)


@admin.get("/media", response_model=list[MediaOut])
def list_media(db: Session = Depends(get_db)):
    return [_media_out(m) for m in db.scalars(select(Media).order_by(Media.id.desc()))]


@admin.delete("/media/{media_id}", status_code=204)
def delete_media(media_id: int, request: Request, db: Session = Depends(get_db)):
    m = db.get(Media, media_id)
    if m is None:
        raise HTTPException(404, "Média introuvable")
    if db.scalar(select(PlaylistItem.id).where(PlaylistItem.media_id == media_id).limit(1)):
        raise HTTPException(409, "Média utilisé dans une playlist")
    for v in db.scalars(select(PlaylistVersion)):
        if media_id in v.media_ids:
            raise HTTPException(409, "Média utilisé dans une version publiée")
    path = request.app.state.settings.media_dir / m.stored_name
    db.delete(m)
    db.commit()
    path.unlink(missing_ok=True)


# ---------- playlists ----------
@admin.post("/playlists", response_model=PlaylistOut, status_code=201)
def create_playlist(body: PlaylistIn, db: Session = Depends(get_db)):
    p = Playlist(name=body.name)
    db.add(p)
    db.commit()
    return p


@admin.get("/playlists", response_model=list[PlaylistOut])
def list_playlists(db: Session = Depends(get_db)):
    return list(db.scalars(select(Playlist).order_by(Playlist.id)))


@admin.get("/playlists/{pid}", response_model=PlaylistOut)
def get_playlist(pid: int, db: Session = Depends(get_db)):
    return _playlist(db, pid)


@admin.put("/playlists/{pid}", response_model=PlaylistOut)
def rename_playlist(pid: int, body: PlaylistIn, db: Session = Depends(get_db)):
    p = _playlist(db, pid)
    p.name = body.name
    db.commit()
    return p


@admin.delete("/playlists/{pid}", status_code=204)
def delete_playlist(pid: int, db: Session = Depends(get_db)):
    p = _playlist(db, pid)
    vids = [v.id for v in p.versions]
    if vids and db.scalar(select(Screen.id).where(Screen.version_id.in_(vids)).limit(1)):
        raise HTTPException(409, "Une version de cette playlist est affectée à un écran")
    db.delete(p)
    db.commit()


@admin.put("/playlists/{pid}/items", response_model=PlaylistOut)
def set_items(pid: int, items: list[ItemIn], db: Session = Depends(get_db)):
    """Remplace la liste ordonnée complète des éléments (l'ordre du tableau = l'ordre de lecture)."""
    p = _playlist(db, pid)
    new = []
    for pos, it in enumerate(items):
        if it.type == "text":
            if not (it.text and it.text.strip()):
                raise HTTPException(422, f"Élément {pos}: texte requis")
            media_id = None
        else:
            m = db.get(Media, it.media_id) if it.media_id else None
            if m is None:
                raise HTTPException(422, f"Élément {pos}: média introuvable")
            if m.kind != it.type:
                raise HTTPException(422, f"Élément {pos}: le média est de type {m.kind}")
            media_id = m.id
        duration = it.duration_s if it.type == "video" else (it.duration_s or DEFAULT_DURATION)
        new.append(PlaylistItem(position=pos, type=it.type, text=it.text if it.type == "text" else None,
                                media_id=media_id, duration_s=duration))
    p.items = new
    db.commit()
    return p


# ---------- publication ----------
@admin.post("/playlists/{pid}/publish", response_model=VersionOut, status_code=201)
def publish_playlist(pid: int, db: Session = Depends(get_db)):
    p = _playlist(db, pid)
    try:
        return publishing.publish(db, p)
    except ValueError as e:
        raise HTTPException(422, str(e))


@admin.get("/playlists/{pid}/preview")
def preview_draft(pid: int, db: Session = Depends(get_db)):
    """Brouillon courant au format lecteur, sans rien publier."""
    manifest, _ = publishing.build_manifest(_playlist(db, pid))
    return {"version_id": None, "number": None, **manifest}


@admin.get("/playlists/{pid}/versions", response_model=list[VersionOut])
def list_versions(pid: int, db: Session = Depends(get_db)):
    return _playlist(db, pid).versions


@admin.get("/versions/{vid}")
def get_version(vid: int, db: Session = Depends(get_db)):
    """Contenu exact d'une version (prévisualisation / test avant déploiement)."""
    v = db.get(PlaylistVersion, vid)
    if v is None:
        raise HTTPException(404, "Version introuvable")
    return publishing.player_payload(v)


# ---------- écrans ----------
@admin.post("/screens", response_model=ScreenCreated, status_code=201)
def create_screen(body: ScreenIn, db: Session = Depends(get_db)):
    token = secrets.token_urlsafe(32)
    s = Screen(name=body.name, token_hash=hash_token(token))
    db.add(s)
    db.commit()
    return {**_screen_out(s), "token": token}


@admin.get("/screens", response_model=list[ScreenOut])
def list_screens(db: Session = Depends(get_db)):
    return [_screen_out(s) for s in db.scalars(select(Screen).order_by(Screen.id))]


@admin.post("/screens/{sid}/assign", response_model=ScreenOut)
def assign_version(sid: int, body: AssignIn, db: Session = Depends(get_db)):
    """Affecte une version à un écran. Revenir en arrière = affecter une version antérieure."""
    s = db.get(Screen, sid)
    if s is None:
        raise HTTPException(404, "Écran introuvable")
    if body.version_id is not None and db.get(PlaylistVersion, body.version_id) is None:
        raise HTTPException(404, "Version introuvable")
    s.version_id = body.version_id
    db.commit()
    return _screen_out(s)


@admin.get("/screens/{sid}/manifest")
def screen_preview(sid: int, db: Session = Depends(get_db)):
    """Ce que voit l'écran (version affectée), sans le marquer comme connecté."""
    s = db.get(Screen, sid)
    if s is None:
        raise HTTPException(404, "Écran introuvable")
    if s.version_id is None:
        raise HTTPException(404, "Aucune version affectée")
    return publishing.player_payload(db.get(PlaylistVersion, s.version_id))


@admin.post("/screens/{sid}/rotate-token", response_model=ScreenCreated)
def rotate_token(sid: int, db: Session = Depends(get_db)):
    s = db.get(Screen, sid)
    if s is None:
        raise HTTPException(404, "Écran introuvable")
    token = secrets.token_urlsafe(32)
    s.token_hash = hash_token(token)
    db.commit()
    return {**_screen_out(s), "token": token}


@admin.post("/screens/{sid}/revoke-token", response_model=ScreenOut)
def revoke_token(sid: int, db: Session = Depends(get_db)):
    s = db.get(Screen, sid)
    if s is None:
        raise HTTPException(404, "Écran introuvable")
    s.token_hash = None
    db.commit()
    return _screen_out(s)


# ---------- lecteur ----------
@player.get("/manifest")
def player_manifest(screen: Screen = Depends(require_screen), db: Session = Depends(get_db)):
    screen.last_seen = utcnow()
    db.commit()
    if screen.version_id is None:
        raise HTTPException(404, "Aucune version affectée")
    return publishing.player_payload(db.get(PlaylistVersion, screen.version_id))
