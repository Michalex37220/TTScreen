"""Garde-fous HTTP transverses : pré-authentification des envois de fichiers et en-têtes de sécurité.

Pourquoi un garde en amont pour /api/media : FastAPI analyse le corps multipart (et le copie dans un
fichier temporaire) AVANT de résoudre les dépendances d'authentification de la route. Sans ce garde,
un client anonyme pouvait faire lire/stocker jusqu'à la taille maximale du proxy avant de recevoir 401.
"""
from __future__ import annotations

from fastapi import Request
from fastapi.concurrency import run_in_threadpool
from fastapi.responses import JSONResponse

from .audit import log_action
from .auth import resolve_token, role_allows

UPLOAD_PATH = "/api/media"
MULTIPART_OVERHEAD = 64 * 1024  # marge pour l'enveloppe multipart autour du fichier

# CSP stricte : tout est servi par la même origine, aucun script/style distant ni en ligne
# (les attributs style="" posés par l'interface imposent 'unsafe-inline' pour les styles seulement).
CSP = ("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; "
       "img-src 'self' data: blob:; media-src 'self' blob:; object-src 'none'; "
       "base-uri 'none'; form-action 'self'; frame-ancestors 'self'")
SECURITY_HEADERS = {
    "X-Content-Type-Options": "nosniff",
    "X-Frame-Options": "SAMEORIGIN",
    "Referrer-Policy": "no-referrer",
    "Cross-Origin-Resource-Policy": "same-origin",
}
DOC_PATHS = ("/docs", "/redoc", "/openapi.json")  # pages de doc FastAPI : incompatibles avec la CSP (dev seulement)


def _precheck_upload(request: Request) -> JSONResponse | None:
    """Authentifie (rôle editor minimum) et borne la taille AVANT toute lecture du corps."""
    from .deps import client_ip  # import tardif : deps importe audit/auth, pas ce module

    app = request.app
    s = app.state.settings
    limiter = app.state.limiter
    auth = request.headers.get("authorization", "")
    token = auth[7:].strip() if auth.lower().startswith("bearer ") else ""
    key = f"authfail:{client_ip(request)}"
    if token:
        retry = limiter.blocked(key, s.rate_limit_authfail)
        if retry:
            return JSONResponse({"detail": "Trop de requêtes, réessayez plus tard."}, status_code=429,
                                headers={"Retry-After": str(retry)})
    with app.state.session_factory() as db:
        principal = resolve_token(db, token, s.admin_token, s.legacy_token_always) if token else None
        if principal is None:
            if token:
                limiter.check(key, s.rate_limit_authfail)
                log_action(db, request, None, "auth_echouee", target="bearer")
                db.commit()
            return JSONResponse({"detail": "Authentification requise ou session expirée"}, status_code=401,
                                headers={"WWW-Authenticate": "Bearer"})
        if not role_allows(principal.role, "editor"):
            log_action(db, request, principal, "acces_refuse", target=f"POST {UPLOAD_PATH}", detail="rôle requis : editor")
            db.commit()
            return JSONResponse({"detail": "Rôle insuffisant pour cette opération"}, status_code=403)
    length = request.headers.get("content-length", "")
    if length.isdigit() and int(length) > s.max_upload_mb * 1024 * 1024 + MULTIPART_OVERHEAD:
        return JSONResponse({"detail": "Fichier trop volumineux"}, status_code=413)
    return None


async def upload_guard(request: Request, call_next):
    if request.method == "POST" and request.url.path == UPLOAD_PATH:
        rejected = await run_in_threadpool(_precheck_upload, request)
        if rejected is not None:
            return rejected
    return await call_next(request)


async def security_headers(request: Request, call_next):
    response = await call_next(request)
    for k, v in SECURITY_HEADERS.items():
        response.headers.setdefault(k, v)
    if not request.url.path.startswith(DOC_PATHS):
        response.headers.setdefault("Content-Security-Policy", CSP)
    return response
