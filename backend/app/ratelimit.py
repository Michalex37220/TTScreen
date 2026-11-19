"""Limitation de débit (rate limiting) de l'API, en mémoire et sans dépendance externe.

Principe : fenêtre glissante par clé. Chaque requête ajoute son horodatage dans la file de
sa clé ; si la file contient déjà `limit` requêtes dans la fenêtre, la requête est refusée
(HTTP 429 avec en-tête Retry-After).

La mémoire est locale au processus : suffisant pour le déploiement prévu (un seul processus
uvicorn, ~2 requêtes/s pour 20 écrans). Pour passer à plusieurs processus, remplacer le
dictionnaire par un compteur partagé (Redis) en ne changeant que la méthode `check`.
"""
from __future__ import annotations

import threading
import time
from collections import deque

from fastapi import Request
from fastapi.responses import JSONResponse

from .auth import hash_token


class RateLimiter:
    """Compteur à fenêtre glissante, protégé par un verrou (uvicorn peut servir en threads)."""

    def __init__(self, window_s: int):
        self.window_s = window_s
        self._hits: dict[str, deque[float]] = {}  # clé -> horodatages des requêtes récentes
        self._lock = threading.Lock()
        self._last_gc = float("-inf")  # dernier nettoyage complet (au plus un par fenêtre : évite un O(n) à chaque requête)

    def blocked(self, key: str, limit: int) -> int:
        """Comme `check` mais SANS enregistrer de requête : renvoie les secondes d'attente si `key`
        a déjà atteint `limit` dans la fenêtre, sinon 0. Sert à bloquer après N échecs enregistrés."""
        if limit <= 0:
            return 0
        now = time.monotonic()
        with self._lock:
            dq = self._hits.get(key)
            if not dq:
                return 0
            while dq and now - dq[0] >= self.window_s:
                dq.popleft()
            if len(dq) >= limit:
                return max(1, int(self.window_s - (now - dq[0])) + 1)
            return 0

    def check(self, key: str, limit: int) -> int:
        """Enregistre une requête pour `key` et renvoie 0 si elle est autorisée,
        sinon le nombre de secondes à attendre avant de réessayer."""
        if limit <= 0:
            return 0  # limite désactivée (0 = illimité), pratique pour les tests
        now = time.monotonic()
        with self._lock:
            dq = self._hits.setdefault(key, deque())
            while dq and now - dq[0] >= self.window_s:  # jette les requêtes sorties de la fenêtre
                dq.popleft()
            if len(dq) >= limit:
                return max(1, int(self.window_s - (now - dq[0])) + 1)
            dq.append(now)
            # Nettoyage des clés mortes, au plus une fois par fenêtre (les clés expirent après `window_s`),
            # pour ne pas reconstruire tout le dictionnaire à chaque requête sous le verrou.
            if len(self._hits) > 10000 and now - self._last_gc >= self.window_s:
                self._hits = {k: v for k, v in self._hits.items() if v and now - v[-1] < self.window_s}
                self._last_gc = now
            return 0


def make_rate_limit_middleware(limiter: RateLimiter):
    """Fabrique le middleware HTTP de limitation de débit.

    - `/api/health` n'est jamais limité (sonde de supervision) ;
    - `POST /api/auth/login` : limite stricte **par IP** (anti force brute sur les mots de passe) ;
    - `/api/player/manifest` : limite **par jeton d'écran** (plusieurs écrans derrière la même NAT
      ont des jetons distincts) ET limite générale par IP, pour qu'un client qui varie le jeton à
      chaque requête ne contourne pas le quota ni ne fasse gonfler le dictionnaire ;
    - le reste de `/api` : limite générale par IP.
    """

    async def middleware(request: Request, call_next):
        path = request.url.path
        if not path.startswith("/api") or path == "/api/health":
            return await call_next(request)
        settings = request.app.state.settings
        ip = request.client.host if request.client else "inconnu"
        if path == "/api/auth/login":
            key, limit = f"login:{ip}", settings.rate_limit_login
        elif path == "/api/player/manifest":
            auth = request.headers.get("authorization", "")
            token = auth[7:].strip() if auth.lower().startswith("bearer ") else ""
            # D'abord le quota par IP (borne le nombre de clés créables), puis celui par jeton
            retry = limiter.check(f"ip:{ip}", settings.rate_limit_api)
            key, limit = f"screen:{hash_token(token)}", settings.rate_limit_player
            if not retry:
                retry = limiter.check(key, limit)
            return await _respond(request, call_next, retry)
        else:
            key, limit = f"ip:{ip}", settings.rate_limit_api
        retry = limiter.check(key, limit)
        return await _respond(request, call_next, retry)

    async def _respond(request, call_next, retry):
        if retry:
            return JSONResponse({"detail": "Trop de requêtes, réessayez plus tard."},
                                status_code=429, headers={"Retry-After": str(retry)})
        return await call_next(request)

    return middleware
