from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Settings:
    # Base de donné (modifiable mais a vos risques)
    database_url: str = "sqlite:///./data/ttscreen.db"
    media_dir: Path = Path("./data/media")
    admin_token: str = "dev-admin-token"
    max_upload_mb: int = 200
    env: str = "development"
    # Limitation de débit (fenêtre glissante en mémoire, voir app/ratelimit.py) ; 0 = désactivé
    rate_window_s: int = 60      # taille de la fenêtre glissante en secondes
    rate_limit_api: int = 600     # requêtes / fenêtre / IP sur le reste de /api
    rate_limit_player: int = 120  # requêtes / fenêtre / jeton d'écran sur /api/player/manifest
    rate_limit_login: int = 20    # tentatives de connexion / fenêtre / IP (anti force brute)
    session_ttl_h: int = 12       # durée de validité d'une session utilisateur, en heures
    # Compte admin d'amorçage facultatif (créé au démarrage si les deux variables sont renseignées)
    bootstrap_admin_user: str = ""
    bootstrap_admin_password: str = ""

    @classmethod
    def from_env(cls) -> "Settings":
        return cls(
            database_url=os.getenv("TTSCREEN_DATABASE_URL", cls.database_url),
            media_dir=Path(os.getenv("TTSCREEN_MEDIA_DIR", str(cls.media_dir))),
            admin_token=os.getenv("TTSCREEN_ADMIN_TOKEN", cls.admin_token),
            max_upload_mb=int(os.getenv("TTSCREEN_MAX_UPLOAD_MB", cls.max_upload_mb)),
            env=os.getenv("TTSCREEN_ENV", cls.env),
            rate_window_s=int(os.getenv("TTSCREEN_RATE_WINDOW_S", cls.rate_window_s)),
            rate_limit_api=int(os.getenv("TTSCREEN_RATE_LIMIT_API", cls.rate_limit_api)),
            rate_limit_player=int(os.getenv("TTSCREEN_RATE_LIMIT_PLAYER", cls.rate_limit_player)),
            rate_limit_login=int(os.getenv("TTSCREEN_RATE_LIMIT_LOGIN", cls.rate_limit_login)),
            session_ttl_h=int(os.getenv("TTSCREEN_SESSION_TTL_H", cls.session_ttl_h)),
            bootstrap_admin_user=os.getenv("TTSCREEN_BOOTSTRAP_ADMIN_USER", cls.bootstrap_admin_user),
            bootstrap_admin_password=os.getenv("TTSCREEN_BOOTSTRAP_ADMIN_PASSWORD", cls.bootstrap_admin_password),
        )
