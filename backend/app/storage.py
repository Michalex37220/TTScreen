from __future__ import annotations

import hashlib
import os
import tempfile
from pathlib import Path

from fastapi import HTTPException, UploadFile

# (kind, mime, extension)
_ALLOWED = {
    "png": ("image", "image/png", "png"),
    "jpeg": ("image", "image/jpeg", "jpg"),
    "webp": ("image", "image/webp", "webp"),
    "gif": ("image", "image/gif", "gif"),
    "mp4": ("video", "video/mp4", "mp4"),
    "webm": ("video", "video/webm", "webm"),
}


def sniff(head: bytes):
    """Détermine le type réel d'après les octets magiques (pas le nom ni le Content-Type)."""
    if head.startswith(b"\x89PNG\r\n\x1a\n"):
        return _ALLOWED["png"]
    if head.startswith(b"\xff\xd8\xff"):
        return _ALLOWED["jpeg"]
    if head[:4] == b"RIFF" and head[8:12] == b"WEBP":
        return _ALLOWED["webp"]
    if head[:6] in (b"GIF87a", b"GIF89a"):
        return _ALLOWED["gif"]
    if head[4:8] == b"ftyp":
        return _ALLOWED["mp4"]
    if head[:4] == b"\x1a\x45\xdf\xa3":
        return _ALLOWED["webm"]
    return None


def save_upload(upload: UploadFile, media_dir: Path, max_bytes: int):
    """Écrit le fichier dans un temporaire, calcule SHA-256, puis renomme atomiquement."""
    media_dir.mkdir(parents=True, exist_ok=True)
    tmp_dir = media_dir / ".tmp"
    tmp_dir.mkdir(exist_ok=True)
    h = hashlib.sha256()
    size = 0
    head = b""
    fd, tmp_path = tempfile.mkstemp(dir=tmp_dir)
    try:
        with os.fdopen(fd, "wb") as out:
            while chunk := upload.file.read(1024 * 1024):
                if len(head) < 16:
                    head += chunk[: 16 - len(head)]
                size += len(chunk)
                if size > max_bytes:
                    raise HTTPException(413, "Fichier trop volumineux")
                h.update(chunk)
                out.write(chunk)
        info = sniff(head)
        if info is None:
            raise HTTPException(415, "Type de fichier non autorisé (PNG, JPEG, WebP, GIF, MP4, WebM)")
        if size == 0:
            raise HTTPException(400, "Fichier vide")
        kind, mime, ext = info
        digest = h.hexdigest()
        stored_name = f"{digest}.{ext}"
        final = media_dir / stored_name
        if final.exists():
            os.unlink(tmp_path)
        else:
            os.replace(tmp_path, final)  # remplacement atomique
        return {"kind": kind, "mime": mime, "sha256": digest, "size": size, "stored_name": stored_name}
    except BaseException:
        if os.path.exists(tmp_path):
            os.unlink(tmp_path)
        raise
