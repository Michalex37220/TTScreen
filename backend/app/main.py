from __future__ import annotations

import os
from pathlib import Path

from fastapi import FastAPI
from fastapi.staticfiles import StaticFiles

from . import api
from .config import Settings
from .db import Base, make_session_factory


def create_app(settings: Settings | None = None) -> FastAPI:
    settings = settings or Settings.from_env()
    if settings.env == "production" and (settings.admin_token == "dev-admin-token" or len(settings.admin_token) < 24):
        raise RuntimeError("SIGNAGE_ADMIN_TOKEN absent, par défaut ou trop court (24 caractères minimum) en production")
    settings.media_dir.mkdir(parents=True, exist_ok=True)
    if settings.database_url.startswith("sqlite:///./"):
        settings.media_dir.parent.mkdir(parents=True, exist_ok=True)
    engine, factory = make_session_factory(settings.database_url)
    from . import models  # noqa: F401  (enregistre les tables)
    Base.metadata.create_all(engine)  # Alembic viendra avec PostgreSQL (étape 8)

    app = FastAPI(title="Signage API")
    app.state.settings = settings
    app.state.session_factory = factory
    app.include_router(api.public)
    app.include_router(api.admin)
    app.include_router(api.player)
    root = Path(__file__).resolve().parents[2]
    fe_dir = Path(os.getenv("SIGNAGE_FRONTEND_DIR", root / "frontend"))
    if fe_dir.is_dir():
        app.mount("/admin", StaticFiles(directory=fe_dir, html=True), name="admin")
    player_dir = Path(os.getenv("SIGNAGE_PLAYER_DIR", root / "player"))
    if player_dir.is_dir():
        app.mount("/player", StaticFiles(directory=player_dir, html=True), name="player")
    return app


app = create_app()
