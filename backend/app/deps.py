from __future__ import annotations

from fastapi import Depends, Header, HTTPException, Request
from sqlalchemy import select
from sqlalchemy.orm import Session

from .audit import log_action
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


def client_ip(request: Request) -> str:
    return request.client.host if request.client else "inconnu"


def reject_if_blocked(request: Request, key: str, limit: int) -> None:
    """429 si `key` a atteint `limit` échecs enregistrés dans la fenêtre (anti force brute)."""
    retry = request.app.state.limiter.blocked(key, limit)
    if retry:
        raise HTTPException(429, "Trop de requêtes, réessayez plus tard.", headers={"Retry-After": str(retry)})


def record_auth_failure(request: Request, db: Session, kind: str) -> None:
    """Compte un jeton invalide pour l'IP appelante et le journalise (sans jamais écrire le jeton)."""
    s = request.app.state.settings
    request.app.state.limiter.check(f"authfail:{client_ip(request)}", s.rate_limit_authfail)
    log_action(db, request, None, "auth_echouee", target=kind)
    db.commit()


def get_principal(request: Request, token: str | None = Depends(get_bearer),
                  db: Session = Depends(get_db)) -> Principal:
    """Résout le jeton Bearer en identité (compte nommé ou jeton admin historique), ou lève 401.

    C'est la base de tout le contrôle d'accès : les routes d'administration exigent un principal,
    les routes d'écran passent par `require_screen` (jetons d'écran, sans aucun droit admin).
    Les jetons invalides sont comptés par IP : au-delà du seuil, 429 (y compris sur les routes
    autres que /login, par lesquelles on pourrait sinon deviner le jeton admin).
    """
    s = request.app.state.settings
    principal = None
    if token:
        reject_if_blocked(request, f"authfail:{client_ip(request)}", s.rate_limit_authfail)
        principal = resolve_token(db, token, s.admin_token, s.legacy_token_always)
        if principal is None:
            record_auth_failure(request, db, "bearer")
    if principal is None:
        raise HTTPException(401, "Authentification requise ou session expirée",
                            headers={"WWW-Authenticate": "Bearer"})
    return principal


def require_role(minimum: str):
    """Fabrique une dépendance exigeant au moins le rôle `minimum`.

    Hiérarchie : viewer (lecture seule) < editor (création/modification du contenu)
    < admin (gestion des comptes et consultation du journal).
    """

    def dep(request: Request, principal: Principal = Depends(get_principal),
            db: Session = Depends(get_db)) -> Principal:
        if not role_allows(principal.role, minimum):
            log_action(db, request, principal, "acces_refuse",
                       target=f"{request.method} {request.url.path}", detail=f"rôle requis : {minimum}")
            db.commit()
            raise HTTPException(403, "Rôle insuffisant pour cette opération")
        return principal

    return dep


# Dépendances prêtes à l'emploi, utilisées en `dependencies=[Depends(...)]` sur les routers d'api.py
require_viewer = require_role("viewer")
require_editor = require_role("editor")
require_admin = require_role("admin")


# Verification du Token de connection grace au calcul du hash
def require_screen(
    request: Request, authorization: str | None = Header(default=None), db: Session = Depends(get_db)
) -> Screen:
    token = _bearer(authorization)
    s = request.app.state.settings
    screen = None
    if token:
        reject_if_blocked(request, f"authfail:{client_ip(request)}", s.rate_limit_authfail)
        screen = db.scalar(select(Screen).where(Screen.token_hash == hash_token(token)))
        if screen is None:
            record_auth_failure(request, db, "ecran")
    if screen is None:
        raise HTTPException(401, "Jeton d'écran invalide ou révoqué")
    return screen
