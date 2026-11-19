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

from sqlalchemy import func, select
from sqlalchemy.orm import Session

from .models import User, UserSession, utcnow

# Nombre d'itérations PBKDF2-HMAC-SHA256 (recommandation OWASP : 600 000).
# hashlib.pbkdf2_hmac est en C : ~0,3 s par hachage, acceptable pour un login
# et dissuasif contre les attaques par force brute.
PBKDF2_ITERATIONS = 600_000

# Nom réservé du principal « jeton admin historique » : impossible à usurper par un compte nommé
# (les journaux distinguent ainsi sans ambiguïté un compte « admin » du jeton historique).
LEGACY_USERNAME = "legacy-admin"
RESERVED_USERNAMES = frozenset({LEGACY_USERNAME, "anonyme"})

# Hiérarchie des rôles : chaque rôle englobe les précédents
ROLES = ("viewer", "editor", "admin")
ROLE_LEVEL = {r: i for i, r in enumerate(ROLES)}


def safe_equal(a: str, b: str) -> bool:
    """Comparaison en temps constant sur des octets : accepte n'importe quel caractère Unicode
    (compare_digest lève TypeError sur des str non ASCII, ce qui donnait un HTTP 500)."""
    return secrets.compare_digest(a.encode("utf-8", "surrogatepass"), b.encode("utf-8", "surrogatepass"))


def hash_token(token: str) -> str:
    """Empreinte SHA-256 d'un jeton : c'est ce qui est stocké en base, jamais le jeton en clair."""
    return hashlib.sha256(token.encode()).hexdigest()


def hash_password(password: str) -> str:
    """Hache un mot de passe (PBKDF2-HMAC-SHA256 + sel aléatoire) au format `pbkdf2_sha256$iter$salt$hash`."""
    salt = secrets.token_bytes(16)
    digest = hashlib.pbkdf2_hmac("sha256", password.encode(), salt, PBKDF2_ITERATIONS)
    return f"pbkdf2_sha256${PBKDF2_ITERATIONS}${salt.hex()}${digest.hex()}"


_DUMMY_HASH: str | None = None


def dummy_hash() -> str:
    """Haché factice (même coût PBKDF2) pour égaliser le temps de réponse quand l'identifiant n'existe pas."""
    global _DUMMY_HASH
    if _DUMMY_HASH is None:
        _DUMMY_HASH = hash_password(secrets.token_urlsafe(16))
    return _DUMMY_HASH


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


def legacy_allowed(db: Session, legacy_always: bool = False) -> bool:
    """Le jeton admin historique n'est accepté que tant qu'aucun compte admin nommé actif n'existe
    (sauf TTSCREEN_LEGACY_TOKEN_ALWAYS) : il sert à amorcer, pas à rester une porte dérobée permanente."""
    if legacy_always:
        return True
    n = db.scalar(select(func.count(User.id)).where(User.role == "admin", User.active.is_(True)))
    return not n


def resolve_token(db: Session, token: str, admin_token: str, legacy_always: bool = False) -> Principal | None:
    """Transforme un jeton Bearer en identité, ou None s'il est invalide.

    Le jeton admin historique (s'il est configuré et encore autorisé) est comparé en temps constant.
    Un jeton de session n'est accepté que si le compte existe, est actif et la session non expirée.
    """
    if admin_token and safe_equal(token, admin_token) and legacy_allowed(db, legacy_always):
        return Principal(username=LEGACY_USERNAME, role="admin", legacy=True)
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


def bootstrap_admin(db: Session, username: str, password: str, reset: bool = False) -> None:
    """Crée le compte admin d'amorçage si TTSCREEN_BOOTSTRAP_ADMIN_* sont renseignés ET qu'aucun admin n'existe.

    Un compte existant n'est JAMAIS modifié au redémarrage (sinon un mot de passe changé via l'interface
    serait écrasé et un compte désactivé volontairement serait réactivé). Pour récupérer l'accès,
    positionner explicitement TTSCREEN_BOOTSTRAP_ADMIN_RESET=true (réinitialise mot de passe, rôle, état).
    """
    if not username or not password:
        return
    if username.lower() in RESERVED_USERNAMES:
        raise RuntimeError(f"Nom d'utilisateur d'amorçage réservé : {username!r}")
    user = db.scalar(select(User).where(User.username == username))
    if user is None:
        if reset or not db.scalar(select(func.count(User.id)).where(User.role == "admin")):
            db.add(User(username=username, password_hash=hash_password(password), role="admin"))
    elif reset:
        user.password_hash = hash_password(password)
        user.role = "admin"
        user.active = True
        drop_sessions(db, user.id)
