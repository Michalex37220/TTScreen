"""Authentification par comptes et rôles.

Deux façons de s'authentifier sur les routes d'administration :
- un **compte nommé** (username + mot de passe) : POST /api/auth/login renvoie un jeton de
  session, stocké haché en base (table user_sessions) comme les jetons d'écran ;
- le **jeton admin historique** (TTSCREEN_ADMIN_TOKEN), toujours accepté pour amorcer le
  système : il équivaut à un compte « admin » de rôle admin.

Aucune dépendance externe : le hachage des mots de passe est fait en PBKDF2-HMAC-SHA256
(stdlib hashlib), ce qui évite d'ajouter un paquet à l'environnement Nix figé.
"""
from __future__ import annotations

import hashlib
import hmac
import secrets
from dataclasses import dataclass
from datetime import datetime, timedelta

from sqlalchemy import select
from sqlalchemy.orm import Session

from .models import User, UserSession, utcnow

# Nombre d'itérations PBKDF2-HMAC-SHA256 (recommandation OWASP : 600 000).
# hashlib.pbkdf2_hmac est en C : ~0,3 s par hachage, acceptable pour un login
# et dissuasif contre les attaques par force brute.
PBKDF2_ITERATIONS = 600_000

# Hiérarchie des rôles : chaque rôle englobe les précédents
ROLES = ("viewer", "editor", "admin")
ROLE_LEVEL = {r: i for i, r in enumerate(ROLES)}


def hash_token(token: str) -> str:
    """Empreinte SHA-256 d'un jeton : c'est ce qui est stocké en base, jamais le jeton en clair."""
    return hashlib.sha256(token.encode()).hexdigest()


def hash_password(password: str) -> str:
    """Hache un mot de passe (PBKDF2-HMAC-SHA256 + sel aléatoire) au format `pbkdf2_sha256$iter$salt$hash`."""
    salt = secrets.token_bytes(16)
    digest = hashlib.pbkdf2_hmac("sha256", password.encode(), salt, PBKDF2_ITERATIONS)
    return f"pbkdf2_sha256${PBKDF2_ITERATIONS}${salt.hex()}${digest.hex()}"


def verify_password(password: str, stored: str) -> bool:
    """Vérifie un mot de passe contre le haché stocké, en temps constant (hmac.compare_digest)."""
    try:
        _algo, iters, salt_hex, digest_hex = stored.split("$")
        digest = hashlib.pbkdf2_hmac("sha256", password.encode(), bytes.fromhex(salt_hex), int(iters))
        return hmac.compare_digest(digest.hex(), digest_hex)
    except (ValueError, AttributeError):
        return False  # haché mal formé : on refuse


@dataclass(frozen=True)
class Principal:
    """Identité authentifiée derrière une requête (compte nommé ou jeton admin historique)."""

    username: str
    role: str      # viewer | editor | admin
    legacy: bool   # True = jeton admin historique, pas un compte en base


def role_allows(role: str, minimum: str) -> bool:
    """True si `role` atteint au moins le niveau `minimum` (viewer < editor < admin)."""
    return ROLE_LEVEL.get(role, -1) >= ROLE_LEVEL.get(minimum, 99)


def create_session(db: Session, user: User, ttl: timedelta) -> tuple[str, datetime]:
    """Crée une session pour un compte : le jeton en clair n'est renvoyé qu'une fois au client,
    seule son empreinte est conservée en base."""
    token = secrets.token_urlsafe(32)
    expires_at = utcnow() + ttl
    db.add(UserSession(user_id=user.id, token_hash=hash_token(token),
                       created_at=utcnow(), expires_at=expires_at))
    return token, expires_at


def resolve_token(db: Session, token: str, admin_token: str) -> Principal | None:
    """Transforme un jeton Bearer en identité, ou None s'il est invalide.

    Le jeton admin historique est comparé en temps constant et donne un principal admin.
    Un jeton de session n'est accepté que si le compte existe, est actif et la session non expirée.
    """
    if secrets.compare_digest(token, admin_token):
        return Principal(username="admin", role="admin", legacy=True)
    session = db.scalar(select(UserSession).where(UserSession.token_hash == hash_token(token)))
    if session is None:
        return None
    user = db.get(User, session.user_id)
    if user is None or not user.active or session.expires_at < utcnow():
        return None
    return Principal(username=user.username, role=user.role, legacy=False)


def drop_sessions(db: Session, user_id: int) -> None:
    """Révoque toutes les sessions d'un compte (désactivation, suppression, changement de mot de passe)."""
    for s in db.scalars(select(UserSession).where(UserSession.user_id == user_id)):
        db.delete(s)


def bootstrap_admin(db: Session, username: str, password: str) -> None:
    """Crée (ou réinitialise) le compte admin d'amorçage si TTSCREEN_BOOTSTRAP_ADMIN_* sont renseignés.

    Utile en production pour créer un premier compte nommé sans exposer le jeton historique.
    Sans ces variables, rien n'est créé : le jeton admin historique reste la méthode d'amorçage.
    """
    if not username or not password:
        return
    user = db.scalar(select(User).where(User.username == username))
    if user is None:
        db.add(User(username=username, password_hash=hash_password(password), role="admin"))
    else:
        user.password_hash = hash_password(password)
        user.role = "admin"
        user.active = True
