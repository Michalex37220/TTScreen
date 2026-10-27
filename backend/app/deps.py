from __future__ import annotations

import hashlib
import secrets

from fastapi import Depends, Header, HTTPException, Request
from sqlalchemy import select
from sqlalchemy.orm import Session

from .models import Screen


def get_db(request: Request):
    db = request.app.state.session_factory()
    try:
        yield db
    finally:
        db.close()


def _bearer(authorization: str | None) -> str | None:
    if authorization and authorization.lower().startswith("bearer "):
        return authorization[7:].strip()
    return None


def require_admin(request: Request, authorization: str | None = Header(default=None)) -> None:
    token = _bearer(authorization)
    expected = request.app.state.settings.admin_token
    if not token or not secrets.compare_digest(token, expected):
        raise HTTPException(401, "Authentification requise", headers={"WWW-Authenticate": "Bearer"})


def hash_token(token: str) -> str:
    return hashlib.sha256(token.encode()).hexdigest()


def require_screen(
    authorization: str | None = Header(default=None), db: Session = Depends(get_db)
) -> Screen:
    token = _bearer(authorization)
    screen = db.scalar(select(Screen).where(Screen.token_hash == hash_token(token))) if token else None
    if screen is None:
        raise HTTPException(401, "Jeton d'écran invalide ou révoqué")
    return screen
