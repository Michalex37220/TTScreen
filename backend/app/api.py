# Tout les points de l'api décrit dans la documentation complete et dans le README
from __future__ import annotations

import re
import secrets
from datetime import timedelta
from typing import Annotated

from fastapi import APIRouter, Body, Depends, File, HTTPException, Path, Query, Request, UploadFile
from fastapi.responses import FileResponse, RedirectResponse
from sqlalchemy import func, select
from sqlalchemy.orm import Session

from . import publishing
from .audit import log_action
from .auth import (LEGACY_USERNAME, Principal, create_session, drop_sessions, dummy_hash, hash_password,
                   legacy_allowed, safe_equal, verify_password)
from .deps import (get_bearer, get_db, get_principal, hash_token, reject_if_blocked,
                   require_admin, require_editor, require_viewer, require_screen)
from .models import (AuditLog, Media, Playlist, PlaylistItem, PlaylistVersion, Screen, User,
                     UserSession, utcnow)
from .schemas import (MAX_ID, MAX_ITEMS, AssignIn, AuditOut, ItemIn, LoginIn, MediaOut, MeOut, PlaylistIn, PlaylistOut,
                      ScreenCreated, ScreenIn, ScreenOut, SessionOut, UserIn, UserOut, UserPatch,
                      VersionOut)
from .storage import save_upload

# Identifiant de ressource borné (INTEGER 32 bits) : refuse tôt les valeurs hors plage (sinon 500 côté base)
IntId = Annotated[int, Path(ge=1, le=MAX_ID)]
# Nom stocké par l'application : SHA-256 hexadécimal + extension autorisée, rien d'autre
STORED_NAME_RE = re.compile(r"^[0-9a-f]{64}\.(png|jpg|webp|gif|mp4|webm)$")

ONLINE_WINDOW = timedelta(seconds=60)
DEFAULT_DURATION = 10

# Les routes d'administration sont découpées en trois routers selon le rôle minimal exigé :
# - viewer (lecture seule) : lister les médias/playlists/versions/écrans, prévisualiser
# - editor : en plus, créer/modifier le contenu, publier, gérer les écrans
# - admin  : en plus, gérer les comptes et consulter le journal d'audit
# Le jeton admin historique (TTSCREEN_ADMIN_TOKEN) a le rôle admin et ouvre les trois routers.
public = APIRouter()
auth = APIRouter(prefix="/api/auth")
viewer = APIRouter(prefix="/api", dependencies=[Depends(require_viewer)])
editor = APIRouter(prefix="/api", dependencies=[Depends(require_editor)])
admin = APIRouter(prefix="/api", dependencies=[Depends(require_admin)])
player = APIRouter(prefix="/api/player")


# ---------- helpers ----------
def _media_out(m: Media) -> MediaOut:
    return MediaOut(id=m.id, original_name=m.original_name, kind=m.kind, mime=m.mime,
                    size=m.size, sha256=m.sha256, url=f"/files/{m.stored_name}")


def _screen_out(s: Screen) -> dict:
    online = bool(s.last_seen and utcnow() - s.last_seen < ONLINE_WINDOW)
    return dict(id=s.id, name=s.name, version_id=s.version_id, online=online,
                last_seen=s.last_seen, has_token=s.token_hash is not None)


def _playlist(db: Session, pid: int) -> Playlist:
    p = db.get(Playlist, pid)
    if p is None:
        raise HTTPException(404, "Playlist introuvable")
    return p


# ---------- public ----------
@public.get("/", include_in_schema=False)
def root():
    return RedirectResponse("/admin/")


@public.get("/api/health")
def health():
    return {"status": "ok"}


@public.get("/files/{stored_name}")
def get_file(stored_name: str, request: Request):
    if not STORED_NAME_RE.fullmatch(stored_name):
        raise HTTPException(404)
    path = request.app.state.settings.media_dir / stored_name
    if not path.is_file():
        raise HTTPException(404)
    return FileResponse(path, headers={"X-Content-Type-Options": "nosniff"})


# ---------- authentification (comptes, sessions, rôles) ----------
@public.post("/api/auth/login", response_model=SessionOut)
def login(body: LoginIn, request: Request, db: Session = Depends(get_db)):
    """Connexion. Deux modes :
    - `username` + `password` : compte nommé → jeton de session (haché en base, révocable, durée limitée) ;
    - `token` seul : jeton admin historique TTSCREEN_ADMIN_TOKEN → renvoyé tel quel (pas de session en base).
    Les connexions réussies et échouées sont journalisées (table audit_log).
    """
    s = request.app.state.settings
    if body.token is not None and not body.username:
        # Comparaison sur octets (pas de 500 sur un jeton non ASCII) ; refusé si désactivé ou si un
        # compte admin nommé existe déjà (voir auth.legacy_allowed)
        if s.admin_token and safe_equal(body.token, s.admin_token) and legacy_allowed(db, s.legacy_token_always):
            log_action(db, request, Principal(LEGACY_USERNAME, "admin", legacy=True), "connexion", target="jeton admin")
            db.commit()
            return SessionOut(token=body.token, username=LEGACY_USERNAME, role="admin", expires_at=None)
        log_action(db, request, None, "connexion_echouee", target="jeton admin")
        db.commit()
        raise HTTPException(401, "Identifiants invalides")
    # Compte nommé : même message d'erreur ET même durée dans tous les cas pour ne pas révéler quel
    # identifiant existe (PBKDF2 est toujours exécuté, contre un haché factice si le compte n'existe pas)
    uname = (body.username or "")
    user_key = f"loginuser:{uname.lower()}"
    reject_if_blocked(request, user_key, s.rate_limit_login_user)  # anti force brute par identifiant
    user = db.scalar(select(User).where(User.username == uname))
    stored = user.password_hash if user is not None else dummy_hash()
    password_ok = verify_password(body.password or "", stored)
    if user is None or not user.active or not password_ok:
        request.app.state.limiter.check(user_key, s.rate_limit_login_user)
        log_action(db, request, None, "connexion_echouee", target=body.username)
        db.commit()
        raise HTTPException(401, "Identifiants invalides")
    user.last_login = utcnow()
    token, expires_at = create_session(db, user, timedelta(hours=s.session_ttl_h))
    log_action(db, request, Principal(user.username, user.role, legacy=False), "connexion", target=user.username)
    db.commit()
    return SessionOut(token=token, username=user.username, role=user.role, expires_at=expires_at)


@auth.post("/logout", status_code=204, dependencies=[Depends(require_viewer)])
def logout(request: Request, token: str | None = Depends(get_bearer),
           principal: Principal = Depends(get_principal), db: Session = Depends(get_db)):
    """Révoque la session courante : le jeton cesse immédiatement de fonctionner."""
    if not principal.legacy:  # le jeton admin historique n'a pas de session en base
        session = db.scalar(select(UserSession).where(UserSession.token_hash == hash_token(token)))
        if session is not None:
            db.delete(session)
    log_action(db, request, principal, "deconnexion", target=principal.username)
    db.commit()


@auth.get("/me", response_model=MeOut, dependencies=[Depends(require_viewer)])
def me(principal: Principal = Depends(get_principal)):
    """Identité derrière le jeton courant (l'interface s'en sert pour afficher le rôle et masquer l'onglet Comptes)."""
    return MeOut(username=principal.username, role=principal.role, legacy=principal.legacy)


# ---------- médias ----------
@editor.post("/media", response_model=MediaOut)
def upload_media(request: Request, file: UploadFile = File(...),
                 principal: Principal = Depends(get_principal), db: Session = Depends(get_db)):
    s = request.app.state.settings
    info = save_upload(file, s.media_dir, s.max_upload_mb * 1024 * 1024)
    existing = db.scalar(select(Media).where(Media.sha256 == info["sha256"]))
    if existing:
        return _media_out(existing)  # doublon : rien n'est écrit ni journalisé
    name = (file.filename or "fichier")[:255]
    m = Media(original_name=name, **info)
    db.add(m)
    log_action(db, request, principal, "media.upload", target=name,
               detail=f"{info['kind']}, {info['size']} o, sha256={info['sha256'][:12]}…")
    db.commit()
    return _media_out(m)


@viewer.get("/media", response_model=list[MediaOut])
def list_media(db: Session = Depends(get_db)):
    return [_media_out(m) for m in db.scalars(select(Media).order_by(Media.id.desc()))]


@editor.delete("/media/{media_id}", status_code=204)
def delete_media(media_id: IntId, request: Request, principal: Principal = Depends(get_principal),
                 db: Session = Depends(get_db)):
    m = db.get(Media, media_id)
    if m is None:
        raise HTTPException(404, "Média introuvable")
    if db.scalar(select(PlaylistItem.id).where(PlaylistItem.media_id == media_id).limit(1)):
        raise HTTPException(409, "Média utilisé dans une playlist")
    for v in db.scalars(select(PlaylistVersion)):
        if media_id in v.media_ids:
            raise HTTPException(409, "Média utilisé dans une version publiée")
    path = request.app.state.settings.media_dir / m.stored_name
    log_action(db, request, principal, "media.delete", target=m.original_name)
    db.delete(m)
    db.commit()
    path.unlink(missing_ok=True)


# ---------- playlists ----------
@editor.post("/playlists", response_model=PlaylistOut, status_code=201)
def create_playlist(body: PlaylistIn, request: Request, principal: Principal = Depends(get_principal),
                    db: Session = Depends(get_db)):
    p = Playlist(name=body.name)
    db.add(p)
    log_action(db, request, principal, "playlist.create", target=body.name)
    db.commit()
    return p


@viewer.get("/playlists", response_model=list[PlaylistOut])
def list_playlists(db: Session = Depends(get_db)):
    return list(db.scalars(select(Playlist).order_by(Playlist.id)))


@viewer.get("/playlists/{pid}", response_model=PlaylistOut)
def get_playlist(pid: IntId, db: Session = Depends(get_db)):
    return _playlist(db, pid)


@editor.put("/playlists/{pid}", response_model=PlaylistOut)
def rename_playlist(pid: IntId, body: PlaylistIn, request: Request,
                    principal: Principal = Depends(get_principal), db: Session = Depends(get_db)):
    p = _playlist(db, pid)
    p.name = body.name
    log_action(db, request, principal, "playlist.rename", target=f"playlist:{pid}", detail=body.name)
    db.commit()
    return p


@editor.delete("/playlists/{pid}", status_code=204)
def delete_playlist(pid: IntId, request: Request, principal: Principal = Depends(get_principal),
                    db: Session = Depends(get_db)):
    p = _playlist(db, pid)
    vids = [v.id for v in p.versions]
    if vids and db.scalar(select(Screen.id).where(Screen.version_id.in_(vids)).limit(1)):
        raise HTTPException(409, "Une version de cette playlist est affectée à un écran")
    log_action(db, request, principal, "playlist.delete", target=f"playlist:{pid}", detail=p.name)
    db.delete(p)
    db.commit()


@editor.put("/playlists/{pid}/items", response_model=PlaylistOut)
def set_items(pid: IntId, items: Annotated[list[ItemIn], Body(max_length=MAX_ITEMS)], request: Request, principal: Principal = Depends(get_principal),
              db: Session = Depends(get_db)):
    """Remplace la liste ordonnée complète des éléments (l'ordre du tableau = l'ordre de lecture)."""
    p = _playlist(db, pid)
    new = []
    for pos, it in enumerate(items):
        if it.type == "text":
            if not (it.text and it.text.strip()):
                raise HTTPException(422, f"Élément {pos}: texte requis")
            media_id = None
        else:
            m = db.get(Media, it.media_id) if it.media_id else None
            if m is None:
                raise HTTPException(422, f"Élément {pos}: média introuvable")
            if m.kind != it.type:
                raise HTTPException(422, f"Élément {pos}: le média est de type {m.kind}")
            media_id = m.id
        duration = it.duration_s if it.type == "video" else (it.duration_s or DEFAULT_DURATION)
        new.append(PlaylistItem(position=pos, type=it.type, text=it.text if it.type == "text" else None,
                                media_id=media_id, duration_s=duration))
    p.items = new
    log_action(db, request, principal, "playlist.items", target=f"playlist:{pid}",
               detail=f"{len(items)} élément(s)")
    db.commit()
    return p


# ---------- publication ----------
@editor.post("/playlists/{pid}/publish", response_model=VersionOut, status_code=201)
def publish_playlist(pid: IntId, request: Request, principal: Principal = Depends(get_principal),
                     db: Session = Depends(get_db)):
    p = _playlist(db, pid)
    try:
        version = publishing.publish(db, p)
    except ValueError as e:
        raise HTTPException(422, str(e))
    # Opération sensible : on la trace dans le journal (même transaction que la publication)
    log_action(db, request, principal, "playlist.publish", target=f"playlist:{pid}", detail=f"v{version.number}")
    db.commit()
    return version


@viewer.get("/playlists/{pid}/preview")
def preview_draft(pid: IntId, db: Session = Depends(get_db)):
    """Brouillon courant au format lecteur, sans rien publier."""
    manifest, _ = publishing.build_manifest(_playlist(db, pid))
    return {"version_id": None, "number": None, **manifest}


@viewer.get("/playlists/{pid}/versions", response_model=list[VersionOut])
def list_versions(pid: IntId, db: Session = Depends(get_db)):
    return _playlist(db, pid).versions


@viewer.get("/versions/{vid}")
def get_version(vid: IntId, db: Session = Depends(get_db)):
    """Contenu exact d'une version (prévisualisation / test avant déploiement)."""
    v = db.get(PlaylistVersion, vid)
    if v is None:
        raise HTTPException(404, "Version introuvable")
    return publishing.player_payload(v)


# ---------- écrans ----------
@editor.post("/screens", response_model=ScreenCreated, status_code=201)
def create_screen(body: ScreenIn, request: Request, principal: Principal = Depends(get_principal),
                  db: Session = Depends(get_db)):
    token = secrets.token_urlsafe(32)
    s = Screen(name=body.name, token_hash=hash_token(token))
    db.add(s)
    log_action(db, request, principal, "screen.create", target=body.name)
    db.commit()
    return {**_screen_out(s), "token": token}


@viewer.get("/screens", response_model=list[ScreenOut])
def list_screens(db: Session = Depends(get_db)):
    return [_screen_out(s) for s in db.scalars(select(Screen).order_by(Screen.id))]


@editor.post("/screens/{sid}/assign", response_model=ScreenOut)
def assign_version(sid: IntId, body: AssignIn, request: Request,
                   principal: Principal = Depends(get_principal), db: Session = Depends(get_db)):
    """Affecte une version à un écran. Revenir en arrière = affecter une version antérieure."""
    s = db.get(Screen, sid)
    if s is None:
        raise HTTPException(404, "Écran introuvable")
    if body.version_id is not None and db.get(PlaylistVersion, body.version_id) is None:
        raise HTTPException(404, "Version introuvable")
    s.version_id = body.version_id
    log_action(db, request, principal, "screen.assign", target=f"screen:{sid}",
               detail=f"version_id={body.version_id}")
    db.commit()
    return _screen_out(s)


@viewer.get("/screens/{sid}/manifest")
def screen_preview(sid: IntId, db: Session = Depends(get_db)):
    """Ce que voit l'écran (version affectée), sans le marquer comme connecté."""
    s = db.get(Screen, sid)
    if s is None:
        raise HTTPException(404, "Écran introuvable")
    if s.version_id is None:
        raise HTTPException(404, "Aucune version affectée")
    return publishing.player_payload(db.get(PlaylistVersion, s.version_id))


@editor.post("/screens/{sid}/rotate-token", response_model=ScreenCreated)
def rotate_token(sid: IntId, request: Request, principal: Principal = Depends(get_principal),
                 db: Session = Depends(get_db)):
    """Génère un nouveau jeton d'écran : l'ancien cesse de fonctionner immédiatement."""
    s = db.get(Screen, sid)
    if s is None:
        raise HTTPException(404, "Écran introuvable")
    token = secrets.token_urlsafe(32)
    s.token_hash = hash_token(token)
    log_action(db, request, principal, "screen.rotate_token", target=f"screen:{sid}")
    db.commit()
    return {**_screen_out(s), "token": token}


@editor.post("/screens/{sid}/revoke-token", response_model=ScreenOut)
def revoke_token(sid: IntId, request: Request, principal: Principal = Depends(get_principal),
                 db: Session = Depends(get_db)):
    s = db.get(Screen, sid)
    if s is None:
        raise HTTPException(404, "Écran introuvable")
    s.token_hash = None
    log_action(db, request, principal, "screen.revoke_token", target=f"screen:{sid}")
    db.commit()
    return _screen_out(s)


# ---------- comptes utilisateurs (rôle admin uniquement) ----------
@admin.post("/users", response_model=UserOut, status_code=201)
def create_user(body: UserIn, request: Request, principal: Principal = Depends(get_principal),
                db: Session = Depends(get_db)):
    """Crée un compte. Le mot de passe n'est jamais stocké en clair (haché PBKDF2, voir auth.py)."""
    if db.scalar(select(User).where(User.username == body.username)):
        raise HTTPException(409, "Nom d'utilisateur déjà pris")
    u = User(username=body.username, password_hash=hash_password(body.password), role=body.role)
    db.add(u)
    log_action(db, request, principal, "user.create", target=u.username, detail=f"rôle={u.role}")
    db.commit()
    return u


@admin.get("/users", response_model=list[UserOut])
def list_users(db: Session = Depends(get_db)):
    return list(db.scalars(select(User).order_by(User.id)))


@admin.patch("/users/{uid}", response_model=UserOut)
def update_user(uid: IntId, body: UserPatch, request: Request,
                principal: Principal = Depends(get_principal), db: Session = Depends(get_db)):
    """Modifie un compte (rôle, état actif, mot de passe).

    Règles de protection : on ne peut pas se désactiver soi-même, et il doit toujours
    rester au moins un administrateur actif. Toute modification révoque les sessions
    existantes du compte (reconnexion obligatoire).
    """
    u = db.get(User, uid)
    if u is None:
        raise HTTPException(404, "Utilisateur introuvable")
    if u.username == principal.username and body.active is False:
        raise HTTPException(409, "Vous ne pouvez pas désactiver votre propre compte")
    new_role = body.role if body.role is not None else u.role
    new_active = body.active if body.active is not None else u.active
    if u.role == "admin" and (not new_active or new_role != "admin"):
        others = db.scalar(select(func.count(User.id)).where(
            User.role == "admin", User.active.is_(True), User.id != u.id))
        if not others:
            raise HTTPException(409, "Il doit rester au moins un administrateur actif")
    if body.role is not None:
        u.role = body.role
    if body.active is not None:
        u.active = body.active
    if body.password is not None:
        u.password_hash = hash_password(body.password)
    drop_sessions(db, u.id)
    log_action(db, request, principal, "user.update", target=u.username,
               detail=f"rôle={u.role}, actif={u.active}" + (", mot de passe changé" if body.password else ""))
    db.commit()
    return u


@admin.delete("/users/{uid}", status_code=204)
def delete_user(uid: IntId, request: Request, principal: Principal = Depends(get_principal),
                db: Session = Depends(get_db)):
    """Supprime un compte (et révoque ses sessions). Interdit de se supprimer soi-même
    ou de supprimer le dernier administrateur actif."""
    u = db.get(User, uid)
    if u is None:
        raise HTTPException(404, "Utilisateur introuvable")
    if u.username == principal.username:
        raise HTTPException(409, "Vous ne pouvez pas supprimer votre propre compte")
    if u.role == "admin":
        others = db.scalar(select(func.count(User.id)).where(
            User.role == "admin", User.active.is_(True), User.id != u.id))
        if not others:
            raise HTTPException(409, "Impossible de supprimer le dernier administrateur actif")
    drop_sessions(db, u.id)
    log_action(db, request, principal, "user.delete", target=u.username)
    db.delete(u)
    db.commit()


# ---------- journal des opérations sensibles (rôle admin uniquement) ----------
@admin.get("/audit", response_model=list[AuditOut])
def list_audit(action: str | None = Query(default=None),
               limit: int = Query(default=100, ge=1, le=1000),
               offset: int = Query(default=0, ge=0),
               db: Session = Depends(get_db)):
    """Journal des opérations sensibles, du plus récent au plus ancien.
    Filtre optionnel par `action` (ex. action=playlist.publish)."""
    q = select(AuditLog).order_by(AuditLog.id.desc())
    if action:
        q = q.where(AuditLog.action == action)
    return list(db.scalars(q.offset(offset).limit(limit)))


# ---------- lecteur ----------
@player.get("/manifest")
def player_manifest(screen: Screen = Depends(require_screen), db: Session = Depends(get_db)):
    screen.last_seen = utcnow()
    db.commit()
    if screen.version_id is None:
        raise HTTPException(404, "Aucune version affectée")
    return publishing.player_payload(db.get(PlaylistVersion, screen.version_id))
