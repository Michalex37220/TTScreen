from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Settings:
    database_url: str = "sqlite:///./data/signage.db"
    media_dir: Path = Path("./data/media")
    admin_token: str = "dev-admin-token"
    max_upload_mb: int = 200
    env: str = "development"

    @classmethod
    def from_env(cls) -> "Settings":
        return cls(
            database_url=os.getenv("SIGNAGE_DATABASE_URL", cls.database_url),
            media_dir=Path(os.getenv("SIGNAGE_MEDIA_DIR", str(cls.media_dir))),
            admin_token=os.getenv("SIGNAGE_ADMIN_TOKEN", cls.admin_token),
            max_upload_mb=int(os.getenv("SIGNAGE_MAX_UPLOAD_MB", cls.max_upload_mb)),
            env=os.getenv("SIGNAGE_ENV", cls.env),
        )
