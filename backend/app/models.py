from __future__ import annotations

from datetime import datetime, timezone

from sqlalchemy import JSON, Boolean, DateTime, ForeignKey, Integer, String, Text, UniqueConstraint
from sqlalchemy.orm import Mapped, mapped_column, relationship

from .db import Base


def utcnow() -> datetime:
    """UTC naïf, cohérent entre SQLite et PostgreSQL."""
    return datetime.now(timezone.utc).replace(tzinfo=None)


class Media(Base):
    __tablename__ = "media"
    id: Mapped[int] = mapped_column(primary_key=True)
    original_name: Mapped[str] = mapped_column(String(255))
    stored_name: Mapped[str] = mapped_column(String(100), unique=True)
    sha256: Mapped[str] = mapped_column(String(64), unique=True, index=True)
    kind: Mapped[str] = mapped_column(String(10))  # image | video
    mime: Mapped[str] = mapped_column(String(50))
    size: Mapped[int] = mapped_column(Integer)
    created_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow)


class Playlist(Base):
    __tablename__ = "playlists"
    id: Mapped[int] = mapped_column(primary_key=True)
    name: Mapped[str] = mapped_column(String(120))
    created_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow)
    items: Mapped[list["PlaylistItem"]] = relationship(
        order_by="PlaylistItem.position", cascade="all, delete-orphan", lazy="selectin"
    )
    versions: Mapped[list["PlaylistVersion"]] = relationship(
        order_by="PlaylistVersion.number", cascade="all, delete-orphan"
    )


class PlaylistItem(Base):
    __tablename__ = "playlist_items"
    id: Mapped[int] = mapped_column(primary_key=True)
    playlist_id: Mapped[int] = mapped_column(ForeignKey("playlists.id"))
    position: Mapped[int] = mapped_column(Integer)
    type: Mapped[str] = mapped_column(String(10))  # text | image | video
    text: Mapped[str | None] = mapped_column(Text, nullable=True)
    media_id: Mapped[int | None] = mapped_column(ForeignKey("media.id"), nullable=True)
    duration_s: Mapped[int | None] = mapped_column(Integer, nullable=True)
    media: Mapped[Media | None] = relationship(lazy="selectin")


class PlaylistVersion(Base):
    """Version immuable : le manifeste est un instantané, jamais modifié."""

    __tablename__ = "playlist_versions"
    __table_args__ = (UniqueConstraint("playlist_id", "number"),)
    id: Mapped[int] = mapped_column(primary_key=True)
    playlist_id: Mapped[int] = mapped_column(ForeignKey("playlists.id"))
    number: Mapped[int] = mapped_column(Integer)
    manifest: Mapped[dict] = mapped_column(JSON)
    media_ids: Mapped[list] = mapped_column(JSON)
    created_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow)


class Screen(Base):
    __tablename__ = "screens"
    id: Mapped[int] = mapped_column(primary_key=True)
    name: Mapped[str] = mapped_column(String(120))
    token_hash: Mapped[str | None] = mapped_column(String(64), unique=True, nullable=True)
    version_id: Mapped[int | None] = mapped_column(ForeignKey("playlist_versions.id"), nullable=True)
    last_seen: Mapped[datetime | None] = mapped_column(DateTime, nullable=True)
    created_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow)


class User(Base):
    """Compte utilisateur nommé (remplace progressivement le jeton admin unique).

    Le mot de passe n'est jamais stocké en clair : uniquement un haché PBKDF2 (voir auth.py).
    role : viewer (lecture seule) < editor (contenu) < admin (comptes + journal).
    """

    __tablename__ = "users"
    id: Mapped[int] = mapped_column(primary_key=True)
    username: Mapped[str] = mapped_column(String(50), unique=True, index=True)
    password_hash: Mapped[str] = mapped_column(String(255))
    role: Mapped[str] = mapped_column(String(10), default="viewer")  # viewer | editor | admin
    active: Mapped[bool] = mapped_column(Boolean, default=True)       # False = compte désactivé
    created_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow)
    last_login: Mapped[datetime | None] = mapped_column(DateTime, nullable=True)


class UserSession(Base):
    """Session de connexion d'un compte : seul le haché du jeton est stocké (comme les jetons d'écran).

    La suppression de la ligne (déconnexion, désactivation du compte, changement de mot de passe)
    révoque immédiatement le jeton.
    """

    __tablename__ = "user_sessions"
    id: Mapped[int] = mapped_column(primary_key=True)
    user_id: Mapped[int] = mapped_column(ForeignKey("users.id"))
    token_hash: Mapped[str] = mapped_column(String(64), unique=True, index=True)
    created_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow)
    expires_at: Mapped[datetime] = mapped_column(DateTime)


class AuditLog(Base):
    """Journal des opérations sensibles (qui a fait quoi, quand, depuis où).

    username/role sont des instantanés : la ligne reste compréhensible même si le compte est supprimé ensuite.
    """

    __tablename__ = "audit_log"
    id: Mapped[int] = mapped_column(primary_key=True)
    created_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow, index=True)
    username: Mapped[str] = mapped_column(String(50))
    role: Mapped[str | None] = mapped_column(String(10), nullable=True)
    action: Mapped[str] = mapped_column(String(40), index=True)   # ex. playlist.publish, user.create
    target: Mapped[str | None] = mapped_column(String(120), nullable=True)  # ex. "playlist:3"
    detail: Mapped[str | None] = mapped_column(Text, nullable=True)
    ip: Mapped[str | None] = mapped_column(String(45), nullable=True)       # IPv4 ou IPv6 du client
