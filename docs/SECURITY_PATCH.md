# Correctifs de l'audit de sécurité

| Réf. | Problème | Correctif | Fichiers |
|---|---|---|---|
| F1 | Jeton admin par défaut `dev-admin-token` | Plus de valeur par défaut ; jeton vide = désactivé ; 24 car. variés exigés **dans tous les environnements** ; en production, refus de démarrer sans jeton ni compte d'amorçage | `config.py`, `main.py`, `docker-compose*.yml`, `scripts/dev.sh` |
| F2 | Corps d'un envoi analysé avant l'authentification | Garde ASGI qui authentifie (rôle editor) et borne `Content-Length` avant toute lecture ; Caddy : 1 Mo partout, 210 Mo seulement sur `POST /api/media` | `security.py`, `main.py`, `Caddyfile` |
| F3 | Limite de login contournable, force brute du jeton | Jetons invalides comptés par IP sur **toutes** les routes (429 + journal) ; blocage par identifiant au login | `deps.py`, `api.py`, `ratelimit.py`, `security.py` |
| F4 | IP client non fiable derrière Caddy | `--forwarded-allow-ips` configurable ; `FORWARDED_ALLOW_IPS="*"` dans le compose de prod (API non publiée) | `Dockerfile`, `docker-compose.prod.yml` |
| F5 | Jeton historique permanent et ambigu | Refusé dès qu'un admin nommé actif existe (sauf `TTSCREEN_LEGACY_TOKEN_ALWAYS=true`) ; principal `legacy-admin` (nom réservé) | `auth.py`, `api.py`, `schemas.py` |
| F6 | Limiteur : quota manifest contournable, reconstruction O(n) | Quota par IP en plus du quota par jeton ; nettoyage au plus 1×/fenêtre | `ratelimit.py` |
| F7 | `compare_digest` -> HTTP 500 sur jeton non ASCII | Comparaison sur octets (`safe_equal`) | `auth.py`, `api.py` |
| F8 | Énumération d'identifiants par le temps de réponse | PBKDF2 toujours exécuté (haché factice) | `auth.py`, `api.py` |
| F9 | Amorçage qui écrase mot de passe/rôle/état à chaque démarrage | Création seulement si aucun admin ; `TTSCREEN_BOOTSTRAP_ADMIN_RESET=true` pour récupérer l'accès | `auth.py` |
| F10 | Secrets, base et artefacts suivis par git | `.gitignore`, script `scripts/untrack-generated-files.sh`, dépendances d'exécution épinglées, outils de test séparés (`requirements-dev.txt`) | racine, `backend/` |
| F11 | Audit incomplet / non transactionnel | `publish` ne commit plus seul ; accès refusés (403) et jetons invalides journalisés | `publishing.py`, `deps.py`, `audit.py` |
| — | Complémentaires | Troncature des champs d'audit (PostgreSQL), identifiants bornés (422 au lieu de 500), 500 éléments max par playlist, `/files` limité aux noms `sha256.ext` (app **et** Caddy : plus d'accès à `.tmp`), CSP + en-têtes, `/docs` coupé en production, paramètres de fragment encodés dans le lecteur, balise `<title>` corrigée | divers |

## Changements à connaître avant de déployer

1. **`TTSCREEN_ADMIN_TOKEN` n'a plus de défaut.** Un jeton de moins de 24 caractères (ou très répétitif) fait échouer le démarrage, même en développement. `scripts/dev.sh` en génère un si besoin.
2. **Le jeton historique cesse de fonctionner dès qu'un compte admin nommé actif existe.** Pour le conserver : `TTSCREEN_LEGACY_TOKEN_ALWAYS=true`. Pour récupérer l'accès : `TTSCREEN_BOOTSTRAP_ADMIN_RESET=true` avec les variables d'amorçage, puis les retirer.
3. Dans les journaux, le jeton historique apparaît désormais sous le nom `legacy-admin` (et non `admin`).
4. `docker-compose.prod.yml` transmet maintenant à l'API les variables d'amorçage et de limitation de débit déjà documentées dans `.env.example` (elles n'étaient pas transmises auparavant).
5. Lancer **une fois** `scripts/untrack-generated-files.sh` puis commiter. L'historique git garde les anciennes versions : si un secret réel y a figuré, le changer.
6. Tests : `pip install -r backend/requirements-dev.txt && scripts/test.sh` (39 tests).

## Limites connues (non corrigées ici)

- Le login en mode « jeton historique » renvoie toujours le jeton lui-même (pas de session dédiée) : il reste un secret statique, d'où son extinction automatique (point 2).
- Le jeton de session est conservé dans `sessionStorage` (pas de cookie httpOnly) ; la CSP réduit le risque d'XSS mais ne l'élimine pas.
- Aucun verrouillage de compte long terme : le blocage par identifiant dure une fenêtre (60 s), ce qui permet à un tiers de ralentir la connexion d'un compte précis.
- Le `Caddyfile` n'a pas pu être exécuté lors de la rédaction : lancer `caddy validate --config deploy/Caddyfile` avant déploiement.
- Images de base Docker non épinglées par digest ; pas de hachés de dépendances.
