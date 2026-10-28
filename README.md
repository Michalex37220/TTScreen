# TTscreen

**Un système d'affichage dynamique, gratuit et open source, créé pour un club de Tennis de Table.**

TTscreen permet de gérer à distance entre 2 et 20 écrans depuis une interface web : vous déposez vos images et vidéos, composez une playlist (texte, images, vidéos), la prévisualisez, la publiez, puis choisissez sur quels écrans la diffuser. Chaque écran joue sa playlist en boucle et se met à jour tout seul.

## Sommaire

1. [État du projet](#état-du-projet)
2. [Fonctionnalités](#fonctionnalités)
3. [Architecture](#architecture)
4. [Installation](#installation)
   - [NixOS](#nixos)
   - [Hors NixOS (Linux, macOS, Windows/WSL)](#hors-nixos-linux-macos-windowswsl)
   - [Docker](#docker)
   - [Variables d'environnement](#variables-denvironnement)
5. [Utilisation](#utilisation)
6. [Brancher un écran (Raspberry Pi / mini-PC)](#brancher-un-écran-raspberry-pi--mini-pc)
7. [Référence de l'API](#référence-de-lapi)
8. [Déploiement en production](#déploiement-en-production)
9. [Sauvegardes et mises à jour](#sauvegardes-et-mises-à-jour)
10. [Sécurité](#sécurité)
11. [Contribuer](#contribuer)
12. [FAQ](#faq)
13. [Contact](#contact)
14. [Licence](#licence)

---

## État du projet

TTscreen est un **MVP fonctionnel en local**. Ce qui est réalisé et ce qui reste à faire :

| Domaine | État |
|---|---|
| API (médias, playlists, versions immuables, écrans, jetons par écran) | ✅ Fait, couvert par des tests |
| Interface d'administration (médias, playlists, prévisualisation, publication, écrans) | ✅ Fait (HTML/JS sans compilation) |
| Lecteur web (texte, images, vidéos, boucle, mise à jour à distance) | ✅ Fait |
| Docker (Alpine), Caddy HTTPS, PostgreSQL, script de sauvegarde | 🟡 Écrit, à valider sur votre serveur |
| Cache hors ligne du lecteur (téléchargement local, SHA-256, remplacement atomique) | ⏳ À faire |
| Comptes utilisateurs et rôles | ⏳ À faire (un jeton admin unique en attendant) |
| Groupes d'écrans | ⏳ À faire |
| Limitation de débit, journal des opérations sensibles | ⏳ À faire |
| Migrations de base de données (Alembic) | ⏳ À faire |
| Interface en React/TypeScript | ⏳ Prévue (l'interface actuelle est en JavaScript simple) |
| Redémarrage automatique du lecteur | ⏳ À faire (voir l'exemple systemd plus bas) |

> **À ne pas exposer sur Internet tel quel** tant que les comptes/rôles et la limitation de débit ne sont pas en place. Voir [Sécurité](#sécurité).

## Fonctionnalités

- **Bibliothèque de médias** : téléversement d'images (PNG, JPEG, WebP, GIF) et de vidéos (MP4, WebM). Le type réel du fichier est vérifié (pas seulement l'extension), la taille est limitée, les doublons sont détectés par SHA-256.
- **Playlists** : texte, images et vidéos ordonnés, avec une durée d'affichage par élément.
- **Prévisualisation** du brouillon et de chaque version avant diffusion.
- **Versions immuables** : chaque publication crée une version figée (v1, v2…). Retour arrière = rediffuser une ancienne version.
- **Écrans** : un jeton distinct par écran, révocable et renouvelable ; état connecté / déconnecté en direct.
- **Lecteur** : lecture en boucle, passage automatique à l'élément suivant, récupération des nouvelles versions toutes les 10 secondes, sans coupure.

## Architecture

```
TTScreen/
├── backend/    API FastAPI (Python), modèles SQLAlchemy, tests pytest
├── frontend/   Interface d'administration (servie sur /admin/)
├── player/     Lecteur d'affichage (servi sur /player/)
├── deploy/     Dockerfile (Alpine), Docker Compose, Caddyfile
├── scripts/    dev.sh, test.sh, backup.sh
├── docs/       Documentation complémentaire
├── flake.nix   Environnement de développement NixOS
└── .env.example
```

| Brique | Choix |
|---|---|
| Backend | Python 3.11+, FastAPI, SQLAlchemy 2 |
| Base de données | SQLite en développement, PostgreSQL en production |
| Stockage | Fichiers sur disque, nommés par leur SHA-256 |
| Interface et lecteur | HTML + JavaScript, servis par l'API |
| Production | Docker Compose, Caddy (HTTPS automatique) |

Principe de diffusion : l'administrateur **publie** une version, l'**affecte** à un écran, et le lecteur de cet écran interroge régulièrement `GET /api/player/manifest` avec son jeton. Quand la version change, la nouvelle playlist démarre à la fin de l'élément en cours.

---

## Installation

Dans tous les cas, l'API démarre sur `http://127.0.0.1:8000` et l'interface est à la racine (`/` redirige vers `/admin/`).

### NixOS

1. **Activer les flakes** (une seule fois). Dans `configuration.nix` :
   ```nix
   nix.settings.experimental-features = [ "nix-command" "flakes" ];
   ```
   puis `sudo nixos-rebuild switch`. Sans toucher à la configuration système :
   ```bash
   mkdir -p ~/.config/nix
   echo 'experimental-features = nix-command flakes' >> ~/.config/nix/nix.conf
   ```
2. **Entrer dans l'environnement de développement** (Python, Node, Docker Compose, Caddy, ffmpeg) :
   ```bash
   git clone https://github.com/Michalex37220/TTScreen.git && cd TTScreen
   nix develop          # le flake doit être suivi par git : git add flake.nix
   ```
3. **Configurer, tester, démarrer** :
   ```bash
   cp .env.example .env
   ./scripts/test.sh    # lance les tests automatisés
   ./scripts/dev.sh     # démarre l'API avec rechargement automatique
   ```

Pour utiliser Docker sur NixOS, activez le démon : `virtualisation.docker.enable = true;` et ajoutez votre utilisateur au groupe `docker`.

### Hors NixOS (Linux, macOS, Windows/WSL)

Prérequis : **Python 3.11 ou plus récent** et git.

```bash
git clone https://github.com/Michalex37220/TTScreen.git && cd TTScreen
cp .env.example .env

cd backend
python3 -m venv .venv
. .venv/bin/activate                      # Windows (PowerShell) : .venv\Scripts\Activate.ps1
pip install -r requirements.txt pytest httpx

python -m pytest -q                       # tests
python -m uvicorn app.main:app --reload --host 127.0.0.1 --port 8000
```

Lancez ces commandes **depuis le dossier `backend/`** : les chemins `./data/...` sont relatifs au dossier de lancement, donc la base et les médias sont créés dans `backend/data/` (ignoré par Git). Pour les déplacer, renseignez des chemins absolus dans `SIGNAGE_DATABASE_URL` et `SIGNAGE_MEDIA_DIR`. Sous Windows natif, préférez WSL.

Sur Linux/macOS, `./scripts/dev.sh` et `./scripts/test.sh` fonctionnent aussi, une fois le venv activé.

### Docker

Essai local avec SQLite (accessible seulement depuis votre machine) :

```bash
docker compose -f deploy/docker-compose.yml up --build
```

Puis ouvrez <http://127.0.0.1:8000>. Le jeton admin par défaut est `dev-admin-token` (ou la valeur de `SIGNAGE_ADMIN_TOKEN` si vous l'exportez). Les données sont dans le volume Docker `signage_data`. La production est décrite [plus bas](#déploiement-en-production).

### Variables d'environnement

| Variable | Défaut | Rôle |
|---|---|---|
| `SIGNAGE_ADMIN_TOKEN` | `dev-admin-token` | Jeton de l'administrateur. **Obligatoire et fort en production** (24 caractères minimum, sinon l'API refuse de démarrer). |
| `SIGNAGE_DATABASE_URL` | `sqlite:///./data/signage.db` | Base de données. PostgreSQL : `postgresql+psycopg://utilisateur:mdp@hôte:5432/base`. |
| `SIGNAGE_MEDIA_DIR` | `./data/media` | Dossier de stockage des médias. |
| `SIGNAGE_MAX_UPLOAD_MB` | `200` | Taille maximale d'un fichier téléversé. |
| `SIGNAGE_ENV` | `development` | Mettre `production` pour activer les contrôles de sécurité au démarrage. |
| `SIGNAGE_FRONTEND_DIR` / `SIGNAGE_PLAYER_DIR` | dossiers du dépôt | Emplacement de l'interface et du lecteur (utile en conteneur). |
| `UVICORN_WORKERS` | `1` | Nombre de processus dans le conteneur. 1 suffit largement pour 20 écrans. |

Générer un jeton solide : `python3 -c "import secrets; print(secrets.token_urlsafe(32))"`. Ne commitez jamais `.env` (il est dans `.gitignore`).

---

## Utilisation

### Avec l'interface web

Ouvrez <http://127.0.0.1:8000/> et saisissez le jeton admin (`SIGNAGE_ADMIN_TOKEN`). Trois onglets :

**Médias**
- Glissez vos images et vidéos dans la zone en pointillés, ou cliquez pour choisir plusieurs fichiers.
- Un média ne peut pas être supprimé tant qu'il est utilisé dans une playlist ou dans une version publiée.

**Playlists**
1. Saisissez un nom dans la colonne de gauche et cliquez sur **+**.
2. Ajoutez des éléments : **+ Texte** ou **+ Image / vidéo**.
3. Réglez la durée en secondes. Un texte ou une image sans durée dure 10 s. Une vidéo sans durée joue jusqu'à sa fin.
4. Réordonnez avec **↑ ↓**, retirez avec **✕**.
5. **Enregistrer et prévisualiser** : montre le brouillon en direct, sans rien publier.
6. **Publier une version** : crée une version figée (v1, v2…).
7. Dans **Versions publiées** : **Prévisualiser** pour tester, puis choisissez un écran et **Diffuser**.

**Écrans**
1. Saisissez un nom et cliquez sur **Créer l'écran**.
2. **Copiez le jeton affiché : il ne l'est qu'une fois.** L'adresse à ouvrir sur l'écran est donnée avec.
3. Choisissez la version à diffuser et cliquez sur **Affecter**.
4. **Voir l'écran** affiche exactement ce que voit l'écran. **Nouveau jeton** et **Révoquer** gèrent l'accès.

> **Enregistrer ≠ publier.** Les modifications d'une playlist n'atteignent jamais un écran tant que vous n'avez pas publié une nouvelle version **et** l'avoir affectée à l'écran.

**Revenir en arrière** : affectez à l'écran une version antérieure.

### Avec l'API

Exemple complet en ligne de commande :

```bash
A="Authorization: Bearer dev-admin-token"; U=http://127.0.0.1:8000

curl -H "$A" -F file=@photo.jpg $U/api/media                                  # 1. média (n'oubliez pas le @)
curl -H "$A" -H 'Content-Type: application/json' -d '{"name":"Accueil"}' $U/api/playlists
curl -H "$A" -X PUT -H 'Content-Type: application/json' $U/api/playlists/1/items \
  -d '[{"type":"text","text":"Bienvenue !","duration_s":8},{"type":"image","media_id":1,"duration_s":12}]'
curl -H "$A" -X POST $U/api/playlists/1/publish                               # 2. version
curl -H "$A" -H 'Content-Type: application/json' -d '{"name":"Entrée"}' $U/api/screens   # 3. écran (jeton affiché une fois)
curl -H "$A" -H 'Content-Type: application/json' -d '{"version_id":1}' $U/api/screens/1/assign
curl -H "Authorization: Bearer <JETON_ÉCRAN>" $U/api/player/manifest          # 4. ce que reçoit l'écran
```

Les identifiants (`1`, `2`…) sont ceux renvoyés par l'API. Si vous obtenez « Aucune version affectée », vérifiez que l'identifiant d'écran de la commande `assign` correspond à celui du jeton utilisé.

### Les adresses du lecteur

| Adresse | Usage |
|---|---|
| `/player/#token=<JETON_ÉCRAN>` | **Écran réel.** Marque l'écran comme connecté. |
| `/player/#screen=<id>` | Voir ce que voit un écran (jeton admin demandé une fois). Ne marque rien. |
| `/player/#version=<id>` | Tester une version avant de la diffuser. |
| `/player/#playlist=<id>` | Voir le brouillon courant d'une playlist. |

En mode prévisualisation, un badge vert en bas à gauche indique la version et l'élément en cours. Cliquer dans la page passe en plein écran.

---

## Brancher un écran (Raspberry Pi / mini-PC)

Sur la machine reliée à l'écran, lancez Chromium en mode kiosque sur l'adresse du lecteur :

```bash
chromium --kiosk --noerrdialogs --disable-infobars \
  --autoplay-policy=no-user-gesture-required \
  "https://VOTRE-DOMAINE/player/#token=<JETON_ÉCRAN>"
```

Selon la distribution, la commande est `chromium` ou `chromium-browser`. L'option d'autoplay est indispensable pour que les vidéos démarrent sans clic ; elles sont jouées **sans son**.

Exemple d'unité systemd utilisateur pour relancer le navigateur en cas de plantage (`~/.config/systemd/user/ttscreen.service`, à adapter et à tester sur votre matériel) :

```ini
[Unit]
Description=TTscreen player
After=graphical-session.target

[Service]
ExecStart=/usr/bin/chromium --kiosk --noerrdialogs --disable-infobars --autoplay-policy=no-user-gesture-required "https://VOTRE-DOMAINE/player/#token=<JETON_ÉCRAN>"
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
```

Activation : `systemctl --user enable --now ttscreen.service`. Ne laissez pas le jeton dans un dépôt Git ou un document partagé : en cas de fuite, utilisez **Nouveau jeton** ou **Révoquer** dans l'interface.

> Le cache hors ligne n'existe pas encore : si le serveur est injoignable, un lecteur déjà démarré continue sa playlist en mémoire, mais une page rechargée sans réseau reste vide.

---

## Référence de l'API

Documentation interactive générée automatiquement : **`/docs`** (Swagger) et **`/openapi.json`**. Les messages d'erreur sont en français.

### Authentification

| Type | En-tête | Où |
|---|---|---|
| Administrateur | `Authorization: Bearer <SIGNAGE_ADMIN_TOKEN>` | Toutes les routes `/api/...` sauf `/api/health` et `/api/player/...` |
| Écran | `Authorization: Bearer <jeton de l'écran>` | `/api/player/manifest` uniquement |

Un jeton d'écran ne donne **aucun** accès aux routes d'administration. Les jetons d'écran sont stockés hachés (SHA-256) : ils ne peuvent pas être relus après leur création.

### Codes de réponse

| Code | Signification |
|---|---|
| `200` / `201` / `204` | Succès / créé / succès sans contenu |
| `400` | Fichier vide |
| `401` | Jeton absent, invalide ou révoqué |
| `404` | Ressource introuvable (ou aucune version affectée à l'écran) |
| `409` | Conflit : média ou playlist encore utilisé |
| `413` | Fichier trop volumineux |
| `415` | Type de fichier non autorisé |
| `422` | Données invalides (le champ `detail` explique pourquoi) |

Format d'erreur : `{"detail": "Message"}`. Pour un `422` de validation, `detail` peut être une liste d'objets contenant un champ `msg`.

### Santé et fichiers (publics)

| Méthode | Route | Description |
|---|---|---|
| `GET` | `/api/health` | Renvoie `{"status":"ok"}`. Sert au healthcheck. |
| `GET` | `/files/{nom_stocké}` | Télécharge un média. Le nom est `<sha256>.<extension>`. Non authentifié (nom non devinable), en-tête `X-Content-Type-Options: nosniff`. |
| `GET` | `/` | Redirige vers `/admin/`. |

### Médias

| Méthode | Route | Description |
|---|---|---|
| `POST` | `/api/media` | Téléverse un fichier (`multipart/form-data`, champ **`file`**). |
| `GET` | `/api/media` | Liste les médias, du plus récent au plus ancien. |
| `DELETE` | `/api/media/{id}` | Supprime un média. `409` s'il est utilisé dans une playlist ou une version publiée. |

Formats acceptés (détectés d'après le contenu) : PNG, JPEG, WebP, GIF, MP4, WebM. Un fichier déjà présent (même SHA-256) renvoie le média existant.

Réponse (`MediaOut`) :
```json
{
  "id": 1,
  "original_name": "photo.jpg",
  "kind": "image",
  "mime": "image/jpeg",
  "size": 46200,
  "sha256": "5e49c01f08456c6a89dafb3a12acb2c974459ea8ce0604b860a1b5faf279a1b1",
  "url": "/files/5e49c01f08456c6a89dafb3a12acb2c974459ea8ce0604b860a1b5faf279a1b1.jpg"
}
```

### Playlists

| Méthode | Route | Corps | Description |
|---|---|---|---|
| `POST` | `/api/playlists` | `{"name": "Accueil"}` | Crée une playlist (`201`). |
| `GET` | `/api/playlists` | — | Liste les playlists avec leurs éléments. |
| `GET` | `/api/playlists/{id}` | — | Détail d'une playlist. |
| `PUT` | `/api/playlists/{id}` | `{"name": "Nouveau nom"}` | Renomme. |
| `DELETE` | `/api/playlists/{id}` | — | Supprime la playlist et ses versions. `409` si une de ses versions est affectée à un écran. |
| `PUT` | `/api/playlists/{id}/items` | liste d'éléments | **Remplace** la liste complète. L'ordre du tableau est l'ordre de lecture. |
| `GET` | `/api/playlists/{id}/preview` | — | Brouillon courant au format lecteur, sans rien publier. |

Élément (`ItemIn`) :

| Champ | Type | Règles |
|---|---|---|
| `type` | `"text"`, `"image"` ou `"video"` | Obligatoire |
| `text` | texte, 2000 caractères max | Obligatoire (non vide) pour `text` |
| `media_id` | entier | Obligatoire pour `image` et `video`, et le média doit être du même type |
| `duration_s` | entier de 1 à 3600 | Défaut **10** pour `text` et `image`. Pour `video`, vide = lecture complète. |

```bash
curl -H "$A" -X PUT -H 'Content-Type: application/json' $U/api/playlists/1/items -d '[
  {"type":"text",  "text":"Bienvenue !", "duration_s":8},
  {"type":"image", "media_id":1},
  {"type":"video", "media_id":2}
]'
```

### Publication et versions

| Méthode | Route | Description |
|---|---|---|
| `POST` | `/api/playlists/{id}/publish` | Crée une version immuable à partir de l'état enregistré (`201`). `422` si la playlist est vide. |
| `GET` | `/api/playlists/{id}/versions` | Liste les versions, de la plus ancienne à la plus récente. |
| `GET` | `/api/versions/{id}` | Contenu exact d'une version, au format lecteur. |

Réponse de `publish` et de la liste (`VersionOut`) : `{"id", "playlist_id", "number", "created_at", "manifest"}`.

**Format du manifeste** (renvoyé par `/api/versions/{id}`, `/api/screens/{id}/manifest` et `/api/player/manifest`) :

```json
{
  "version_id": 3,
  "number": 2,
  "playlist_id": 1,
  "name": "Accueil",
  "items": [
    { "type": "text", "duration_s": 8, "text": "Bienvenue !" },
    { "type": "image", "duration_s": 12,
      "media": { "id": 1, "url": "/files/823a…afabd.png", "sha256": "823a…afabd", "size": 263254, "mime": "image/png" } },
    { "type": "video", "duration_s": null,
      "media": { "id": 2, "url": "/files/9f2c….mp4", "sha256": "9f2c…", "size": 5242880, "mime": "video/mp4" } }
  ]
}
```
Pour le brouillon (`/preview`), `version_id` et `number` valent `null`.

### Écrans

| Méthode | Route | Corps | Description |
|---|---|---|---|
| `POST` | `/api/screens` | `{"name": "Entrée"}` | Crée un écran. La réponse contient **`token`**, affiché une seule fois (`201`). |
| `GET` | `/api/screens` | — | Liste les écrans et leur état. |
| `POST` | `/api/screens/{id}/assign` | `{"version_id": 3}` | Affecte une version. `{"version_id": null}` retire l'affectation. Revenir en arrière = affecter une ancienne version. |
| `GET` | `/api/screens/{id}/manifest` | — | Ce que voit l'écran, **sans** le marquer connecté. `404` si rien n'est affecté. |
| `POST` | `/api/screens/{id}/rotate-token` | — | Génère un nouveau jeton (l'ancien cesse de fonctionner). |
| `POST` | `/api/screens/{id}/revoke-token` | — | Supprime le jeton : l'écran ne peut plus se connecter. |

Réponse (`ScreenOut`) :
```json
{ "id": 2, "name": "Entrée", "version_id": 3, "online": true,
  "last_seen": "2026-10-21T21:06:10.261721", "has_token": true }
```
`online` vaut `true` si le lecteur a interrogé le serveur dans les 60 dernières secondes. `last_seen` est en UTC.

### Lecteur

| Méthode | Route | Authentification | Description |
|---|---|---|---|
| `GET` | `/api/player/manifest` | Jeton d'écran | Renvoie la version affectée (format manifeste) et met à jour `last_seen`. `401` si le jeton est invalide ou révoqué, `404` si rien n'est affecté. |

### Pages statiques

| Route | Contenu |
|---|---|
| `/admin/` | Interface d'administration |
| `/player/` | Lecteur d'affichage (voir [les adresses du lecteur](#les-adresses-du-lecteur)) |

---

## Déploiement en production

Architecture : **Caddy** (HTTPS automatique, sert directement les médias) → **API** (conteneur Alpine, lecture seule, sans privilèges) → **PostgreSQL**. Seuls les ports 80 et 443 sont exposés.

1. Faites pointer le DNS de votre domaine vers l'adresse du serveur et ouvrez les ports 80 et 443.
2. Installez Docker et Docker Compose, puis récupérez le dépôt.
3. Configurez les secrets (ils ne doivent jamais être dans Git) :
   ```bash
   cp deploy/.env.example deploy/.env
   python3 -c "import secrets; print(secrets.token_urlsafe(32))"   # à lancer deux fois
   nano deploy/.env      # DOMAIN, SIGNAGE_ADMIN_TOKEN, POSTGRES_PASSWORD
   ```
4. Démarrez :
   ```bash
   docker compose -f deploy/docker-compose.prod.yml up -d --build
   docker compose -f deploy/docker-compose.prod.yml logs -f
   ```
5. Ouvrez `https://VOTRE-DOMAINE/` et connectez-vous avec `SIGNAGE_ADMIN_TOKEN`.

En production (`SIGNAGE_ENV=production`), l'API **refuse de démarrer** si le jeton admin est celui par défaut ou fait moins de 24 caractères.

Sur le serveur, ne publiez pas le port 8000 : l'API n'est joignable que par Caddy.

## Sauvegardes et mises à jour

**Sauvegarde** (base PostgreSQL + médias dans `./backups`, rotation sur 14 jours) :
```bash
./scripts/backup.sh
```
À planifier par exemple avec cron : `0 3 * * * cd /chemin/TTScreen && ./scripts/backup.sh`. Copiez aussi les sauvegardes **hors du serveur**.

**Restauration** de la base : `gunzip -c backups/db-AAAA-MM-JJ-HHMM.sql.gz | docker compose -f deploy/docker-compose.prod.yml exec -T db psql -U signage signage`. Pour les médias, extrayez l'archive `.tgz` dans le volume `signage_media`. Testez une restauration avant d'en avoir besoin.

**Mise à jour** :
```bash
git pull
./scripts/backup.sh
docker compose -f deploy/docker-compose.prod.yml up -d --build
```
Il n'y a pas encore de migrations automatiques du schéma : lisez les notes de version avant de mettre à jour.

## Sécurité

Déjà en place :
- Routes d'administration protégées par jeton ; un jeton d'écran ne donne aucun droit d'administration.
- Jeton distinct par écran, haché en base, renouvelable et révocable.
- Téléversements validés (type réel, taille maximale, écriture atomique).
- Conteneur API sans privilèges, en lecture seule ; secrets hors Git ; refus de démarrer avec un jeton faible en production.
- En-têtes de sécurité et HTTPS via Caddy.

Limites actuelles, à connaître :
- Un **seul jeton admin partagé** : pas encore de comptes, de rôles ni de journal d'audit.
- **Pas de limitation de débit** sur l'API.
- Les médias sous `/files/` sont lisibles sans authentification (noms non devinables). Des URL signées sont prévues.
- Le jeton admin saisi dans l'interface est conservé dans le `sessionStorage` du navigateur (effacé à la fermeture de l'onglet).

Pour signaler une faille, n'ouvrez pas de ticket public : écrivez à l'adresse de la section [Contact](#contact).

---

## Contribuer

Les contributions sont les bienvenues : correctifs, tests, traductions, documentation, idées.

1. **Discutez d'abord** : ouvrez un ticket (bug ou proposition) avant un gros changement.
2. **Forkez** le dépôt et créez une branche : `git checkout -b ma-fonctionnalite`.
3. **Installez l'environnement** ([NixOS](#nixos) ou [hors NixOS](#hors-nixos-linux-macos-windowswsl)).
4. **Écrivez le code et ses tests** : toute fonctionnalité doit avoir des tests automatisés dans `backend/tests/`. Lancez-les avec `./scripts/test.sh` (ou `python -m pytest -q` depuis `backend/`). Une fonctionnalité sans test passant n'est pas considérée comme terminée.
5. **Vérifiez à la main** dans le navigateur : interface `/admin/`, prévisualisation, lecteur.
6. **Commitez** avec des messages clairs (une idée par commit), puis ouvrez une **demande de fusion** qui décrit le problème, la solution et comment la tester.

Conventions :
- Aucun secret, jeton, mot de passe ni fichier `.env` dans Git.
- Backend : Python typé, code et messages d'erreur destinés à l'utilisateur en français.
- Interface : pas d'injection de données dans le HTML (utilisez `textContent` ou les helpers existants), pas de dépendance externe chargée depuis un CDN.
- Les endpoints d'administration doivent rester protégés ; tout nouvel endpoint s'accompagne d'un test d'authentification.

Idées de contributions : cache hors ligne du lecteur, comptes et rôles, groupes d'écrans, migrations Alembic, limitation de débit, portage de l'interface en React/TypeScript, planification horaire des playlists.

## FAQ

**Combien d'écrans peut-on gérer ?**
Le projet vise de 2 à 20 écrans. À cette échelle, la charge est très faible (environ 2 requêtes par seconde pour 20 écrans).

**Quelle différence entre « Enregistrer » et « Publier » ?**
Enregistrer sauvegarde le brouillon de la playlist. Publier en crée une version figée. Un écran ne reçoit que des versions publiées **et** affectées.

**Mon écran affiche « Aucune version affectée ».**
Aucune version n'est affectée à *cet* écran. Dans l'onglet Écrans, choisissez une version et cliquez sur Affecter. Vérifiez qu'il s'agit du bon écran : plusieurs écrans ont des identifiants différents.

**J'ai modifié ma playlist mais l'écran n'a pas changé.**
Il faut publier une nouvelle version, puis l'affecter à l'écran. Le lecteur la prend en compte sous 10 secondes, à la fin de l'élément en cours.

**Comment revenir à une version précédente ?**
Affectez à l'écran l'ancienne version. Les versions publiées ne sont jamais modifiées ni supprimées.

**J'ai perdu le jeton d'un écran.**
Il n'est pas récupérable (seul son hachage est stocké). Cliquez sur **Nouveau jeton** : l'ancien cesse de fonctionner et vous en recevez un nouveau, à remettre dans l'adresse du lecteur de l'écran.

**J'ai perdu le jeton admin.**
C'est la valeur de `SIGNAGE_ADMIN_TOKEN` dans votre `.env` (ou `deploy/.env` en production). Pour le changer, modifiez-la et redémarrez l'API.

**Un écran apparaît « déconnecté » alors qu'il est allumé.**
L'écran doit ouvrir l'adresse avec `#token=…` (et non `#screen=…`, qui est une prévisualisation). L'état « connecté » signifie une requête dans les 60 dernières secondes ; vérifiez le réseau et que le jeton n'a pas été révoqué.

**Les vidéos n'ont pas de son.**
C'est voulu : les navigateurs n'autorisent la lecture automatique que sans son.

**Les vidéos ne démarrent pas sur l'écran.**
Lancez Chromium avec `--autoplay-policy=no-user-gesture-required` et vérifiez que le format est MP4 (H.264) ou WebM lisible par le Raspberry Pi / mini-PC.

**Quels formats et quelles tailles ?**
PNG, JPEG, WebP, GIF, MP4, WebM, jusqu'à 200 Mo par fichier (`SIGNAGE_MAX_UPLOAD_MB`). En production, Caddy limite le corps de requête à 210 Mo.

**Je ne peux pas supprimer un média (erreur 409).**
Il est utilisé par une playlist ou par une version publiée. Retirez-le des playlists ; s'il figure dans une version publiée, il est conservé pour permettre le retour arrière.

**Que se passe-t-il si Internet est coupé ?**
Un lecteur déjà démarré continue sa playlist avec ce qu'il a en mémoire et se resynchronise au retour du réseau. Le cache hors ligne persistant (qui survit à un rechargement ou un redémarrage) n'est pas encore implémenté.

**HTTPS est-il obligatoire ?**
En local (`127.0.0.1`), non. Sur un serveur public, oui : utilisez la configuration Caddy fournie.

**Puis-je utiliser autre chose qu'un Raspberry Pi ?**
Oui : tout appareil capable de lancer Chromium en plein écran (mini-PC, ancien portable, certaines télévisions avec navigateur récent).

**Comment mettre à jour TTscreen ?**
Voir [Sauvegardes et mises à jour](#sauvegardes-et-mises-à-jour).

**`nix develop` affiche « experimental Nix feature 'nix-command' is disabled ».**
Activez les flakes, voir [NixOS](#nixos).

## Contact

Une question, un bug, une idée, une faille de sécurité à signaler ?

| Canal | Adresse |
|---|---|
| E-mail | `alexismarechalbrix@proton.me` |
| XMPP | `canyougetsomeburger@jabber.fr` |
| Tickets | <https://github.com/Michalex37220/TTScreen/issues> |
