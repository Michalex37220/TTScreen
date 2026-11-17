from __future__ import annotations

from fastapi import Depends, Header, HTTPException, Request
from sqlalchemy import select
from sqlalchemy.orm import Session

from .auth import Principal, hash_token, resolve_token, role_allows
from .models import Screen


def get_db(request: Request):
    db = request.app.state.session_factory()
    try:
        yield db
    finally:
        db.close()


# Check si le header est correct (pour des raisons de sécurité)
def _bearer(authorization: str | None) -> str | None:
    if authorization and authorization.lower().startswith("bearer "):
        return authorization[7:].strip()
    return None


def get_bearer(authorization: str | None = Header(default=None)) -> str | None:
    """Jeton brut de l'en-tête Authorization (ou None s'il est absent/mal formé)."""
    return _bearer(authorization)


def get_principal(request: Request, token: str | None = Depends(get_bearer),
                  db: Session = Depends(get_db)) -> Principal:
    """Résout le jeton Bearer en identité (compte nommé ou jeton admin historique), ou lève 401.

    C'est la base de tout le contrôle d'accès : les routes d'administration exigent un principal,
    les routes d'écran passent par `require_screen` (jetons d'écran, sans aucun droit admin).
    """
    principal = resolve_token(db, token, request.app.state.settings.admin_token) if token else None
    if principal is None:
        raise HTTPException(401, "Authentification requise ou session expirée",
                            headers={"WWW-Authenticate": "Bearer"})
    return principal


def require_role(minimum: str):
    """Fabrique une dépendance exigeant au moins le rôle `minimum`.

    Hiérarchie : viewer (lecture seule) < editor (création/modification du contenu)
    < admin (gestion des comptes et consultation du journal).
    """

    def dep(principal: Principal = Depends(get_principal)) -> Principal:
        if not role_allows(principal.role, minimum):
            raise HTTPException(403, "Rôle insuffisant pour cette opération")
        return principal

    return dep


# Dépendances prêtes à l'emploi, utilisées en `dependencies=[Depends(...)]` sur les routers d'api.py
require_viewer = require_role("viewer")
require_editor = require_role("editor")
require_admin = require_role("admin")


# Verification du Token de connection grace au calcul du hash
def require_screen(
    authorization: str | None = Header(default=None), db: Session = Depends(get_db)
) -> Screen:
    token = _bearer(authorization)
    screen = db.scalar(select(Screen).where(Screen.token_hash == hash_token(token))) if token else None
    if screen is None:
        raise HTTPException(401, "Jeton d'écran invalide ou révoqué")
    return screen
