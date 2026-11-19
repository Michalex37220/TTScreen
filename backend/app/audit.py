"""Journal des opérations sensibles (audit log).

Chaque opération importante (publication, affectation à un écran, gestion des comptes,
tentative de connexion…) ajoute une ligne dans la table `audit_log` via `log_action`.
La ligne est validée par le commit de l'appelant : elle fait partie de la même transaction
que l'opération auditée, elle ne peut donc pas être écrite sans que l'opération réussisse.
"""
from __future__ import annotations

from fastapi import Request
from sqlalchemy.orm import Session

from .auth import Principal
from .models import AuditLog, utcnow


def log_action(db: Session, request: Request, principal: Principal | None, action: str,
               target: str | None = None, detail: str | None = None) -> None:
    """Ajoute une ligne au journal.

    - `principal` : identité de l'auteur (None pour une tentative de connexion échouée) ;
    - `action` : code court, ex. "playlist.publish", "user.create", "connexion_echouee" ;
    - `target` : ressource visée, ex. "playlist:3" ;
    - `ip` : adresse du client (request.client.host), pour situer l'action.
    """
    # Troncature aux longueurs des colonnes : sous PostgreSQL un VARCHAR dépassé lève une erreur
    # (SQLite l'ignore), ce qui faisait échouer l'opération auditée pour un nom de fichier long.
    db.add(AuditLog(
        created_at=utcnow(),
        username=(principal.username if principal else "anonyme")[:50],
        role=principal.role if principal else None,
        action=action[:40],
        target=target[:120] if target else None,
        detail=detail,
        ip=(request.client.host if request.client else None) and request.client.host[:45],
    ))
