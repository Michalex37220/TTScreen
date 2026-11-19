from __future__ import annotations

import logging
import os
from pathlib import Path

from fastapi import FastAPI
from sqlalchemy import func, select
from fastapi.staticfiles import StaticFiles

from . import api
from .auth import bootstrap_admin
from .config import Settings
from .db import Base, make_session_factory
from .models import User
from .ratelimit import RateLimiter, make_rate_limit_middleware
from .security import security_headers, upload_guard

log = logging.getLogger("ttscreen")


def create_app(settings: Settings | None = None) -> FastAPI:
    settings = settings or Settings.from_env()
    # Contrôle du jeton admin historique dans TOUS les environnements (plus de valeur par défaut connue).
    # Un jeton vide désactive simplement le jeton historique.
    tok = settings.admin_token
    if tok and (tok == "dev-admin-token" or len(tok) < 24 or len(set(tok)) < 10):
        raise RuntimeError("TTSCREEN_ADMIN_TOKEN trop faible : 24 caractères minimum et variés "
                           "(python3 -c \"import secrets; print(secrets.token_urlsafe(32))\"), ou laissez-le vide")
    settings.media_dir.mkdir(parents=True, exist_ok=True)
    if settings.database_url.startswith("sqlite:///./"):
        settings.media_dir.parent.mkdir(parents=True, exist_ok=True)
    engine, factory = make_session_factory(settings.database_url)
    from . import models  # noqa: F401  (enregistre les tables, y compris users / user_sessions / audit_log)
    Base.metadata.create_all(engine)

    # Compte admin d'amorçage facultatif (TTSCREEN_BOOTSTRAP_ADMIN_USER / _PASSWORD) :
    # permet de créer un premier compte nommé sans connaître le jeton historique
    with factory() as db:
        bootstrap_admin(db, settings.bootstrap_admin_user, settings.bootstrap_admin_password,
                        reset=settings.bootstrap_admin_reset)
        db.commit()
        if not tok and not db.scalar(select(func.count(User.id))):
            msg = ("Aucun moyen de se connecter : définissez TTSCREEN_ADMIN_TOKEN ou "
                   "TTSCREEN_BOOTSTRAP_ADMIN_USER/PASSWORD")
            if settings.env == "production":
                raise RuntimeError(msg)
            log.warning(msg)

    # Si jamais vous vous demandez pourquoi ca s'apelle ttscreen RTFM
    # Documentation OpenAPI publique désactivée en production (réduit la surface de reconnaissance)
    docs = {} if settings.env != "production" else dict(docs_url=None, redoc_url=None, openapi_url=None)
    app = FastAPI(title="TTScreen API", **docs)
    app.state.settings = settings
    app.state.session_factory = factory
    # Limitation de débit : fenêtre glissante en mémoire (voir ratelimit.py pour les limites par route)
    app.state.limiter = RateLimiter(settings.rate_window_s)
    # Ordre : le dernier enregistré est le plus externe -> en-têtes, puis débit, puis pré-auth des envois
    app.middleware("http")(upload_guard)
    app.middleware("http")(make_rate_limit_middleware(app.state.limiter))
    app.middleware("http")(security_headers)
    app.include_router(api.public)
    app.include_router(api.auth)
    app.include_router(api.viewer)
    app.include_router(api.editor)
    app.include_router(api.admin)
    app.include_router(api.player)
    root = Path(__file__).resolve().parents[2]
    fe_dir = Path(os.getenv("TTSCREEN_FRONTEND_DIR", root / "frontend"))
    if fe_dir.is_dir():
        app.mount("/admin", StaticFiles(directory=fe_dir, html=True), name="admin")
    player_dir = Path(os.getenv("TTSCREEN_PLAYER_DIR", root / "player"))
    if player_dir.is_dir():
        app.mount("/player", StaticFiles(directory=player_dir, html=True), name="player")
    return app


app = create_app()
