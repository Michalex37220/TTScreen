from __future__ import annotations

import os
from pathlib import Path

from fastapi import FastAPI
from fastapi.staticfiles import StaticFiles

from . import api
from .auth import bootstrap_admin
from .config import Settings
from .db import Base, make_session_factory
from .ratelimit import RateLimiter, make_rate_limit_middleware


def create_app(settings: Settings | None = None) -> FastAPI:
    settings = settings or Settings.from_env()
    if settings.env == "production" and (settings.admin_token == "dev-admin-token" or len(settings.admin_token) < 24):
        raise RuntimeError("TTSCREEN_ADMIN_TOKEN absent, par défaut ou trop court (24 caractères minimum) en production") # uniquement valable en prod
    settings.media_dir.mkdir(parents=True, exist_ok=True)
    if settings.database_url.startswith("sqlite:///./"):
        settings.media_dir.parent.mkdir(parents=True, exist_ok=True)
    engine, factory = make_session_factory(settings.database_url)
    from . import models  # noqa: F401  (enregistre les tables, y compris users / user_sessions / audit_log)
    Base.metadata.create_all(engine)

    # Compte admin d'amorçage facultatif (TTSCREEN_BOOTSTRAP_ADMIN_USER / _PASSWORD) :
    # permet de créer un premier compte nommé sans connaître le jeton historique
    with factory() as db:
        bootstrap_admin(db, settings.bootstrap_admin_user, settings.bootstrap_admin_password)
        db.commit()

    # Si jamais vous vous demandez pourquoi ca s'apelle ttscreen RTFM
    app = FastAPI(title="TTScreen API")
    app.state.settings = settings
    app.state.session_factory = factory
    # Limitation de débit : fenêtre glissante en mémoire (voir ratelimit.py pour les limites par route)
    app.state.limiter = RateLimiter(settings.rate_window_s)
    app.middleware("http")(make_rate_limit_middleware(app.state.limiter))
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
