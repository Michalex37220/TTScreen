# TTscreen

**Un système d'affichage dynamique, gratuit et open source, créé pour un club de Tennis de Table.**

TTscreen permet de gérer à distance des écrans depuis une interface web : vous déposez vos images et vidéos, composez une playlist (texte, images, vidéos), la prévisualisez, la publiez, puis choisissez sur quels écrans la diffuser. Chaque écran joue sa playlist en boucle et se met à jour tout seul.

### Documentation complète : https://alexismarechalbrix.page.gd/ttscreen.html

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
| Interface d'administration (médias, playlists, prévisualisation, publication, écrans) | ✅ Fait |
| Lecteur web (texte, images, vidéos, boucle, mise à jour à distance) | ✅ Fait |
| Docker (Alpine), Caddy HTTPS, PostgreSQL, script de sauvegarde | 🟡 Écrit, à valider sur votre serveur |
| Cache hors ligne du lecteur (téléchargement local, SHA-256, remplacement atomique) | ⏳ À faire |
| Comptes utilisateurs et rôles (admin / éditeur / lecture seule), sessions révoquables | ✅ Fait (le jeton admin historique sert à l'amorçage et cesse de fonctionner dès qu'un compte admin nommé existe) |
| Groupes d'écrans | ⏳ En fonction des usages |
| Limitation de débit (par IP, par jeton d'écran, anti force brute sur login et jetons) et journal d'audit | ✅ Fait |
| Interface en React/TypeScript | ⏳ Possible mais peu probable car j'aime pas React (l'interface actuelle est en JavaScript simple) |

## Fonctionnalités

- **Bibliothèque de médias** : téléversement d'images (PNG, JPEG, WebP, GIF) et de vidéos (MP4, WebM). Le type réel du fichier est vérifié (pas seulement l'extension), la taille est limitée, les doublons sont détectés par SHA-256. L'envoi est authentifié **avant** toute lecture du corps.
- **Playlists** : texte, images et vidéos ordonnés (500 éléments max par playlist), avec une durée d'affichage par élément.
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
├── scripts/    dev.sh, test.sh, backup.sh, untrack-generated-files.sh
├── docs/       Documentation complémentaire (dont SECURITY_PATCH.md)
├── flake.nix   Environnement de développement NixOS
└── .env.example
```

| Brique | Choix |
|---|---|
| Backend | Python 3.11+, FastAPI, SQLAlchemy 2 (dépendances épinglées) |
| Base de données | SQLite en développement, PostgreSQL en production |
| Stockage | Fichiers sur disque, nommés par leur SHA-256 (`<sha256>.<ext>` uniquement) |
| Interface et lecteur | HTML + JavaScript, servis par l'API |
| Production | Docker Compose, Caddy (HTTPS automatique, sert les médias, borne les corps) |

Principe de diffusion : l'administrateur **publie** une version, l'**affecte** à un écran, et le lecteur de cet écran interroge régulièrement `GET /api/player/manifest` avec son jeton. Quand la version change, la nouvelle playlist démarre à la fin de l'élément en cours.

---

## Installation

Dans tous les cas, l'API démarre sur `http://127.0.0.1:8000` et l'interface est à la racine (`/` redirige vers `/admin/`). (attention ceci est uniquement pour les parametres de base, cela peut etre modifie voir le deploiement en prod)

> **Important depuis le patch de sécurité** : il n'y a **plus de jeton admin par défaut**. Définissez toujours `TTSCREEN_ADMIN_TOKEN` (24 caractères minimum, variés) ou un compte d'amorçage, même en développement.

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
   # Éditez .env et définissez TTSCREEN_ADMIN_TOKEN (ou laissez dev.sh en générer un)
   ./scripts/test.sh    # lance les tests automatisés
   ./scripts/dev.sh     # démarre l'API avec rechargement automatique (génère un jeton si absent)
   ```

Pour utiliser Docker sur NixOS, activez le démon : `virtualisation.docker.enable = true;` et ajoutez votre utilisateur au groupe `docker` [documentation officiel](https://wiki.nixos.org/wiki/Docker/en).

### Hors NixOS (Linux, macOS, Windows/WSL)

Prérequis : **Python 3.11 ou plus récent** et git.

```bash
git clone https://github.com/Michalex37220/TTScreen.git && cd TTScreen
cp .env.example .env
# Définissez TTSCREEN_ADMIN_TOKEN dans .env (24 car. min.) :
python3 -c "import secrets; print(secrets.token_urlsafe(32))"

cd backend
python3 -m venv .venv
. .venv/bin/activate                      # Windows (PowerShell) : .venv\Scripts\Activate.ps1
pip install -r requirements.txt           # dépendances d'exécution épinglées
pip install -r requirements-dev.txt       # outils de test (pytest, httpx) — dev uniquement

python -m pytest -q                       # tests (certains peuvent déclencher des warnings ce qui est normal)
python -m uvicorn app.main:app --reload --host 127.0.0.1 --port 8000
```

Lancez ces commandes **depuis le dossier `backend/`** : les chemins `./data/...` sont relatifs au dossier de lancement, donc la base et les médias sont créés dans `backend/data/` (ignoré par Git). Pour les déplacer, renseignez des chemins absolus dans `TTSCREEN_DATABASE_URL` et `TTSCREEN_MEDIA_DIR`. Sous Windows natif, préférez WSL.

Sur Linux/macOS, `./scripts/dev.sh` et `./scripts/test.sh` fonctionnent aussi, une fois le venv activé. `dev.sh` génère automatiquement un jeton admin aléatoire pour la session si `TTSCREEN_ADMIN_TOKEN` n'est pas défini.

### Docker

Essai local avec SQLite (accessible seulement depuis votre machine) :

```bash
# TTSCREEN_ADMIN_TOKEN est obligatoire (plus de valeur par défaut) :
export TTSCREEN_ADMIN_TOKEN="$(python3 -c 'import secrets; print(secrets.token_urlsafe(32))')"
docker compose -f deploy/docker-compose.yml up --build
```

Puis ouvrez <http://127.0.0.1:8000> et connectez-vous avec la valeur de `TTSCREEN_ADMIN_TOKEN` que vous avez définie (champ identifiant laissé vide, champ mot de passe = jeton), ou créez un compte admin via les variables d'amorçage. Les données sont dans le volume Docker `ttscreen_data`. La production est décrite [plus bas](#déploiement-en-production).

### Variables d'environnement

| Variable | Défaut | Rôle |
|---|---|---|
| `TTSCREEN_ADMIN_TOKEN` | vide (désactivé) | Jeton admin historique. **24 caractères minimum et variés, exigés dans tous les environnements** (sinon l'API refuse de démarrer). Vide = désactivé. Refusé dès qu'un compte admin nommé actif existe (sauf `TTSCREEN_LEGACY_TOKEN_ALWAYS=true`). |
| `TTSCREEN_LEGACY_TOKEN_ALWAYS` | `false` | `true` = le jeton admin historique reste accepté même après la création d'un compte admin nommé. |
| `TTSCREEN_BOOTSTRAP_ADMIN_USER` / `TTSCREEN_BOOTSTRAP_ADMIN_PASSWORD` | vide | Si renseignés, créent un compte admin nommé au démarrage **uniquement si aucun admin n'existe**. Un compte existant n'est jamais modifié au redémarrage. |
| `TTSCREEN_BOOTSTRAP_ADMIN_RESET` | `false` | `true` = réinitialise explicitement le compte d'amorçage (mot de passe, rôle, état actif) — utile pour récupérer l'accès. À retirer ensuite. |
| `TTSCREEN_DATABASE_URL` | `sqlite:///./data/ttscreen.db` | Base de données. PostgreSQL : `postgresql+psycopg://utilisateur:mdp@hôte:5432/base`. |
| `TTSCREEN_MEDIA_DIR` | `./data/media` | Dossier de stockage des médias. |
| `TTSCREEN_MAX_UPLOAD_MB` | `200` | Taille maximale d'un fichier téléversé (contrôlée avant lecture du corps). |
| `TTSCREEN_ENV` | `development` | Mettre `production` pour couper `/docs`, `/redoc`, `/openapi.json` et exiger un moyen de connexion. |
| `TTSCREEN_SESSION_TTL_H` | `12` | Durée de validité d'une session utilisateur, en heures. |
| `TTSCREEN_RATE_WINDOW_S` | `60` | Taille de la fenêtre glissante de limitation de débit, en secondes. |
| `TTSCREEN_RATE_LIMIT_API` | `600` | Requêtes / fenêtre / IP sur le reste de `/api` (`0` = illimité). |
| `TTSCREEN_RATE_LIMIT_PLAYER` | `120` | Requêtes / fenêtre / **jeton d'écran** sur `/api/player/manifest` (un quota par IP s'ajoute). |
| `TTSCREEN_RATE_LIMIT_LOGIN` | `20` | Tentatives de connexion / fenêtre / IP (anti force brute). |
| `TTSCREEN_RATE_LIMIT_AUTHFAIL` | `20` | Jetons Bearer invalides / fenêtre / IP avant blocage 429, sur **toutes** les routes. |
| `TTSCREEN_RATE_LIMIT_LOGIN_USER` | `10` | Échecs de connexion / fenêtre / **identifiant** avant blocage 429. |
| `TTSCREEN_FRONTEND_DIR` / `TTSCREEN_PLAYER_DIR` | dossiers du dépôt | Emplacement de l'interface et du lecteur (utile en conteneur). |
| `UVICORN_WORKERS` | `1` | Nombre de processus dans le conteneur. 1 suffit largement pour 20 écrans. |

Générer un jeton solide : `python3 -c "import secrets; print(secrets.token_urlsafe(32))"`. Ne commitez jamais `.env` (il est dans `.gitignore`).

---

## Utilisation

### Avec l'interface web

Ouvrez <http://127.0.0.1:8000/> et connectez-vous avec un **compte utilisateur** (identifiant + mot de passe) ou, si aucun compte admin nommé n'existe encore, avec le **jeton admin historique** (`TTSCREEN_ADMIN_TOKEN`, champ identifiant laissé vide). Dès qu'un compte admin nommé actif existe, le jeton historique est refusé (sauf `TTSCREEN_LEGACY_TOKEN_ALWAYS=true`) : connectez-vous avec le compte nommé. Onglets :

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

**Comptes** (onglet réservé au rôle admin)
- Créez des comptes avec un rôle : **viewer** (lecture seule), **editor** (contenu, publication, écrans), **admin** (tout, y compris cette page). Les noms `legacy-admin` et `anonyme` sont réservés.
- **Désactiver** révoque immédiatement les sessions du compte ; **Mot de passe** et **Supprimer** aussi. Il reste toujours au moins un administrateur actif.
- Le **journal des opérations sensibles** liste les connexions (réussies ou non), les jetons invalides, les accès refusés (403), publications, affectations, créations/suppressions, etc., avec l'auteur, la date et l'IP.

> **Enregistrer ≠ publier.** Les modifications d'une playlist n'atteignent jamais un écran tant que vous n'avez pas publié une nouvelle version **et** l'avoir affectée à l'écran.

**Revenir en arrière** : affectez à l'écran une version antérieure.

### Avec l'API

Exemple complet en ligne de commande (adaptez `A` avec votre jeton de session ou votre `TTSCREEN_ADMIN_TOKEN`) :

```bash
A="Authorization: Bearer $TTSCREEN_ADMIN_TOKEN"; U=http://127.0.0.1:8000

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

Documentation interactive générée automatiquement : **`/docs`** (Swagger) et **`/openapi.json`** — disponibles en développement, **désactivées en production** (`TTSCREEN_ENV=production`). Les messages d'erreur sont en français.

#### Authentification

| Type | En-tête | Utilisé pour |
| :--- | :------ | :----------- |
| Compte utilisateur | `Authorization: Bearer <jeton de session>` (obtenu via `POST /api/auth/login`) | Routes `/api/...` selon le rôle : `viewer` = lecture seule, `editor` = + contenu/publication/écrans, `admin` = + comptes et journal |
| Administrateur historique (amorçage) | `Authorization: Bearer <TTSCREEN_ADMIN_TOKEN>` | Toutes les routes `/api/...` sauf `/api/health` et `/api/player/manifest`. **Refusé dès qu'un compte admin nommé actif existe** (sauf `TTSCREEN_LEGACY_TOKEN_ALWAYS=true`). Apparaît comme `legacy-admin` dans le journal. |
| Écran | `Authorization: Bearer <jeton de l'écran>` | `/api/player/manifest` uniquement |

Un jeton d'écran ne donne **aucun** accès aux routes d'administration. Les jetons d'écran et de session sont stockés hachés (SHA-256) : ils ne peuvent plus être relus après leur création. Les sessions expirent (`TTSCREEN_SESSION_TTL_H`) et sont révoquées à la déconnexion, au changement de mot de passe ou à la désactivation du compte.

Les **jetons invalides sont comptés par IP sur toutes les routes** : au-delà de `TTSCREEN_RATE_LIMIT_AUTHFAIL`, réponse `429` et journalisation (`auth_echouee`, sans jamais écrire le jeton).

#### Codes de réponse

| Code | Signification |
| :--- | :------------ |
| `200` / `201` / `204` | Succès / créé / succès sans contenu |
| `400` | Fichier vide |
| `401` | Jeton absent, invalide, révoqué ou session expirée |
| `403` | Authentifié mais rôle insuffisant (journalisé : `acces_refuse`) |
| `404` | Ressource introuvable (ou aucune version affectée à l'écran) |
| `409` | Conflit : ressource encore utilisée, nom déjà pris, dernier admin… |
| `413` | Fichier trop volumineux (refusé avant lecture du corps) |
| `415` | Type de fichier non autorisé |
| `422` | Données invalides (identifiants bornés à 2³¹−1, max 500 éléments par playlist…) |
| `429` | Trop de requêtes (limitation de débit), réessayer après `Retry-After` |

Format d'erreur : `{"detail": "Message"}`. Pour un `422` de validation, `detail` peut être une liste d'objets contenant un champ `msg`.

### Santé et fichiers

#### Vérifier l'état du serveur

```http
  GET /api/health
```

Aucun paramètre, aucune authentification, jamais limité (sonde de supervision). Renvoie `{"status": "ok"}`.

#### Télécharger un média

```http
  GET /files/${stored_name}
```

| Parameter     | Type     | Description                                                                 |
| :------------ | :------- | :-------------------------------------------------------------------------- |
| `stored_name` | `string` | **Required**. Nom du fichier : `<sha256 hexadécimal>.<extension autorisée>` (`png`, `jpg`, `webp`, `gif`, `mp4`, `webm`) |

Non authentifié (le nom n'est pas devinable). Seuls les noms produits par l'application sont servis (ni `.tmp`, ni aucun autre fichier du volume, côté application **et** côté Caddy). Réponse avec les en-têtes `X-Content-Type-Options: nosniff`, `Cache-Control: public, max-age=31536000, immutable` et une CSP restrictive.

### Comptes, sessions et journal

#### Se connecter

```http
  POST /api/auth/login
```

```json
{"username": "emma", "password": "motdepasse"}
```

Ou, pour le jeton admin historique (champ identifiant vide côté interface, tant qu'aucun admin nommé actif n'existe) : `{"token": "<TTSCREEN_ADMIN_TOKEN>"}`. Renvoie `{"token": "...", "username": "...", "role": "...", "expires_at": "..."}` : le `token` est le jeton de session à passer en `Bearer`. Le mot de passe est toujours vérifié (contre un haché factice si le compte n'existe pas) pour égaliser le temps de réponse. Limité à `TTSCREEN_RATE_LIMIT_LOGIN` tentatives par fenêtre et par IP, et à `TTSCREEN_RATE_LIMIT_LOGIN_USER` échecs par fenêtre et par identifiant. Les connexions réussies et échouées sont journalisées.

#### Se déconnecter / identité courante

```http
  POST /api/auth/logout     # révoque la session courante (204)
  GET  /api/auth/me         # {"username", "role", "legacy"} du jeton courant
```

#### Gérer les comptes (rôle admin)

```http
  GET    /api/users                    # liste des comptes
  POST   /api/users                    # {"username", "password" (8 min.), "role": "viewer|editor|admin"} → 201 (noms réservés refusés)
  PATCH  /api/users/{id}               # {"role"?, "active"?, "password"?} — révoque les sessions du compte
  DELETE /api/users/{id}               # 204 (interdit de se supprimer soi-même ou le dernier admin actif)
```

#### Consulter le journal des opérations sensibles (rôle admin)

```http
  GET /api/audit?action=playlist.publish&limit=100&offset=0
```

Renvoie les entrées les plus récentes d'abord : `created_at`, `username`, `role`, `action`, `target`, `detail`, `ip`. Inclut désormais les connexions échouées, les jetons invalides (`auth_echouee`) et les accès refusés (`acces_refuse`).

### Médias

#### Téléverser un média

```http
  POST /api/media
```

| Parameter       | Type     | Description                                                                                     |
| :-------------- | :------- | :---------------------------------------------------------------------------------------------- |
| `Authorization` | `string` | **Required**. En-tête `Bearer <jeton de session éditeur/admin ou TTSCREEN_ADMIN_TOKEN>`        |
| `file`          | `file`   | **Required**. PNG, JPEG, WebP, GIF, MP4 ou WebM (`multipart/form-data`, 200 Mo max par défaut) |

L'authentification (rôle editor minimum) et la vérification de `Content-Length` sont faites **avant** toute lecture du corps : un client non authentifié ou un fichier trop gros reçoit `401`/`413` sans que le corps soit analysé. Le type est détecté d'après le contenu du fichier, pas son extension. Un fichier déjà présent (même SHA-256) renvoie le média existant.

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

```bash
curl -H "$A" -F file=@photo.jpg $U/api/media
```

#### Lister les médias

```http
  GET /api/media
```

Renvoie la liste des médias, du plus récent au plus ancien.

#### Supprimer un média

```http
  DELETE /api/media/${id}
```

`204` en cas de succès. `409` si le média est utilisé dans une playlist ou dans une version publiée. `422` si l'identifiant dépasse 2³¹−1.

### Playlists

#### Créer une playlist

```http
  POST /api/playlists
```

| Parameter       | Type     | Description                                          |
| :-------------- | :------- | :--------------------------------------------------- |
| `name`          | `string` | **Required**. Nom (1 à 120 caractères), corps JSON   |

Renvoie `201` avec `{"id": 1, "name": "Accueil", "items": []}`.

#### Lister les playlists

```http
  GET /api/playlists
```

#### Obtenir une playlist

```http
  GET /api/playlists/${id}
```

#### Renommer une playlist

```http
  PUT /api/playlists/${id}
```

#### Supprimer une playlist

```http
  DELETE /api/playlists/${id}
```

Supprime aussi ses versions. `204` en cas de succès, `409` si une de ses versions est affectée à un écran.

#### Définir le contenu d'une playlist

```http
  PUT /api/playlists/${id}/items
```

| *(corps)*       | `array`   | **Required**. Liste des éléments (**500 max**). **Remplace** tout le contenu ; l'ordre = l'ordre de lecture |

Chaque élément du tableau :

| Field        | Type      | Description                                                                                        |
| :----------- | :-------- | :------------------------------------------------------------------------------------------------- |
| `type`       | `string`  | **Required**. `text`, `image` ou `video`                                                           |
| `text`       | `string`  | **Required** si `type` = `text` (non vide, 2000 caractères max)                                    |
| `media_id`   | `integer` | **Required** si `type` = `image` ou `video` (1 à 2³¹−1) ; le média doit être du même type          |
| `duration_s` | `integer` | Durée en secondes (1 à 3600). Défaut **10** pour `text` et `image` ; vide pour `video` = lecture complète |

```bash
curl -H "$A" -X PUT -H 'Content-Type: application/json' $U/api/playlists/1/items -d '[
  {"type":"text",  "text":"Bienvenue !", "duration_s":8},
  {"type":"image", "media_id":1},
  {"type":"video", "media_id":2}
]'
```

#### Prévisualiser le brouillon

```http
  GET /api/playlists/${id}/preview
```

Renvoie l'état enregistré de la playlist au [format manifeste](#format-du-manifeste), sans rien publier. `version_id` et `number` valent `null`.

### Publication et versions

#### Publier une version

```http
  POST /api/playlists/${id}/publish
```

Crée une version immuable à partir de l'état enregistré (`201`). `422` si la playlist est vide. La publication et son entrée d'audit sont écrites dans la **même transaction** : si l'audit échoue, rien n'est publié. Réponse :

```json
{ "id": 3, "playlist_id": 1, "number": 2, "created_at": "2026-10-21T21:06:10.261721", "manifest": { "...": "..." } }
```

#### Lister les versions d'une playlist

```http
  GET /api/playlists/${id}/versions
```

Du plus ancien au plus récent, au même format que la réponse de publication.

#### Obtenir une version

```http
  GET /api/versions/${id}
```

Contenu exact de la version, au [format manifeste](#format-du-manifeste).

### Écrans

#### Créer un écran

```http
  POST /api/screens
```

| Parameter       | Type     | Description                                          |
| :-------------- | :------- | :--------------------------------------------------- |
| `name`          | `string` | **Required**. Nom de l'écran (1 à 120 caractères)    |

Renvoie `201` avec l'écran et son **`token`, affiché une seule fois** :

```json
{ "id": 2, "name": "Entrée", "version_id": null, "online": false,
  "last_seen": null, "has_token": true, "token": "dX1qW1K_…" }
```

#### Lister les écrans

```http
  GET /api/screens
```

Chaque écran a la forme `{"id", "name", "version_id", "online", "last_seen", "has_token"}`. `online` vaut `true` si le lecteur a interrogé le serveur dans les 60 dernières secondes ; `last_seen` est en UTC.

#### Affecter une version à un écran

```http
  POST /api/screens/${id}/assign
```

| Parameter       | Type              | Description                                                           |
| :-------------- | :---------------- | :-------------------------------------------------------------------- |
| `id`            | `integer`         | **Required**. Identifiant de l'écran (1 à 2³¹−1)                     |
| `version_id`    | `integer \| null` | **Required**. Version à diffuser (1 à 2³¹−1) ; `null` retire l'affectation (corps JSON) |

Revenir en arrière = affecter une ancienne version.

#### Voir ce que voit un écran

```http
  GET /api/screens/${id}/manifest
```

Renvoie la version affectée au [format manifeste](#format-du-manifeste), **sans** marquer l'écran comme connecté. `404` si rien n'est affecté.

#### Générer un nouveau jeton d'écran

```http
  POST /api/screens/${id}/rotate-token
```

L'ancien jeton cesse de fonctionner immédiatement. Le nouveau est renvoyé dans le champ `token`, une seule fois.

#### Révoquer le jeton d'un écran

```http
  POST /api/screens/${id}/revoke-token
```

L'écran ne peut plus se connecter. Pour le réactiver, générez un nouveau jeton.

### Lecteur

#### Récupérer la playlist à diffuser

```http
  GET /api/player/manifest
```

| Parameter       | Type     | Description                                                   |
| :-------------- | :------- | :------------------------------------------------------------ |
| `Authorization` | `string` | **Required**. En-tête `Bearer <jeton de l'écran>` (pas le jeton admin) |

Renvoie la version affectée à l'écran au [format manifeste](#format-du-manifeste) et met à jour `last_seen`. `401` si le jeton est invalide ou révoqué, `404` si rien n'est affecté. Double quota : par jeton d'écran **et** par IP.

#### Format du manifeste

Renvoyé par `/api/versions/${id}`, `/api/screens/${id}/manifest`, `/api/player/manifest` et `/api/playlists/${id}/preview` :

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

### Pages statiques

#### Interface d'administration

```http
  GET /admin/
```

`GET /` redirige vers cette page.

#### Lecteur d'affichage

```http
  GET /player/#${mode}=${valeur}
```

| Parameter | Type      | Description                                                                                  |
| :-------- | :-------- | :------------------------------------------------------------------------------------------- |
| `token`   | `string`  | Jeton d'écran : écran réel, marque l'écran comme connecté                                    |
| `screen`  | `integer` | Voir ce que voit un écran (jeton admin demandé une fois), ne marque rien                     |
| `version` | `integer` | Tester une version avant de la diffuser (jeton admin)                                        |
| `playlist`| `integer` | Voir le brouillon courant d'une playlist (jeton admin)                                       |

Un seul mode à la fois, par exemple `/player/#token=dX1q…`. Les valeurs du fragment sont encodées côté lecteur. Voir aussi [les adresses du lecteur](#les-adresses-du-lecteur).

---

## Déploiement en production

Architecture : **Caddy** (HTTPS automatique, sert directement les médias, borne les corps : 1 Mo partout sauf 210 Mo sur `POST /api/media`) → **API** (conteneur Alpine, lecture seule, sans privilèges, port non publié) → **PostgreSQL**. Seuls les ports 80 et 443 sont exposés.

1. Faites pointer le DNS de votre domaine vers l'adresse du serveur et ouvrez les ports 80 et 443.
2. Installez Docker et Docker Compose, puis récupérez le dépôt.
3. Configurez les secrets (ils ne doivent jamais être dans Git) :
   ```bash
   cp deploy/.env.example deploy/.env
   python3 -c "import secrets; print(secrets.token_urlsafe(32))"   # à lancer deux fois
   nano deploy/.env      # DOMAIN, POSTGRES_PASSWORD, et AU MOINS UN des deux :
                          #  - TTSCREEN_ADMIN_TOKEN (24 car. min. variés)
                          #  - TTSCREEN_BOOTSTRAP_ADMIN_USER + TTSCREEN_BOOTSTRAP_ADMIN_PASSWORD (recommandé)
   ```
   Le compte d'amorçage n'est créé que s'il n'existe aucun admin ; retirez `TTSCREEN_BOOTSTRAP_ADMIN_PASSWORD` du fichier une fois le compte créé.
4. Démarrez :
   ```bash
   docker compose -f deploy/docker-compose.prod.yml up -d --build
   docker compose -f deploy/docker-compose.prod.yml logs -f
   ```
5. Ouvrez `https://VOTRE-DOMAINE/` et connectez-vous avec le compte d'amorçage (ou le jeton historique tant qu'aucun admin nommé n'existe).

En production (`TTSCREEN_ENV=production`) :
- l'API **refuse de démarrer** si `TTSCREEN_ADMIN_TOKEN` est renseigné mais fait moins de 24 caractères ou est trop répétitif, et **exige au moins un moyen de connexion** (jeton ou compte d'amorçage) ;
- `/docs`, `/redoc` et `/openapi.json` sont désactivés ;
- Caddy transmet `X-Forwarded-For` de confiance (`FORWARDED_ALLOW_IPS="*"`, l'API n'étant pas publiée) pour que la limitation de débit voie la vraie IP des clients.

Sur le serveur, ne publiez pas le port 8000 : l'API n'est joignable que par Caddy. Validez le Caddyfile avant déploiement : `caddy validate --config deploy/Caddyfile`.

> Après un `git pull` sur une installation existante, lancez **une fois** `./scripts/untrack-generated-files.sh` puis committez, afin de retirer secrets, base SQLite et artefacts du suivi Git. Si un secret réel a déjà été commité, changez-le : l'historique Git le conserve.

## Sauvegardes et mises à jour

**Sauvegarde** (base PostgreSQL + médias dans `./backups`, rotation sur 14 jours) :
```bash
./scripts/backup.sh
```
À planifier par exemple avec cron : `0 3 * * * cd /chemin/TTScreen && ./scripts/backup.sh`. Copiez aussi les sauvegardes **hors du serveur**.

**Restauration** de la base : 
```bash
gunzip -c backups/db-AAAA-MM-JJ-HHMM.sql.gz | docker compose -f deploy/docker-compose.prod.yml exec -T db psql -U ttscreen ttscreen
```

. Pour les médias, extrayez l'archive `.tgz` dans le volume `ttscreen_media`. Testez une restauration avant d'en avoir besoin.

**Mise à jour** :
```bash
git pull
./scripts/backup.sh
docker compose -f deploy/docker-compose.prod.yml up -d --build
```
Il n'y a pas encore de migrations automatiques du schéma : lisez les notes de version (et `docs/SECURITY_PATCH.md`) avant de mettre à jour.

## Sécurité

Voir aussi [`docs/SECURITY_PATCH.md`](docs/SECURITY_PATCH.md) pour le détail des correctifs de l'audit de sécurité.

Déjà en place :
- **Plus de jeton admin par défaut** : `TTSCREEN_ADMIN_TOKEN` vide = désactivé ; s'il est renseigné, 24 caractères minimum et variés exigés **dans tous les environnements** (démarrage refusé sinon). En production, l'API refuse de démarrer sans aucun moyen de connexion.
- **Jeton historique à extinction automatique** : refusé dès qu'un compte admin nommé actif existe (sauf `TTSCREEN_LEGACY_TOKEN_ALWAYS=true`) ; apparaît comme `legacy-admin` (nom réservé) dans le journal.
- **Comptes utilisateurs et rôles** : `admin` (tout), `editor` (contenu, publication, écrans), `viewer` (lecture seule). Mots de passe hachés en PBKDF2-HMAC-SHA256 (600 000 itérations, jamais en clair), vérification toujours exécutée (haché factice) pour ne pas divulguer l'existence d'un identifiant par le temps de réponse. Sessions stockées hachées, révoquables et à durée limitée (`TTSCREEN_SESSION_TTL_H`).
- **Amorçage sans écrasement** : le compte `TTSCREEN_BOOTSTRAP_ADMIN_*` n'est créé que s'il n'existe aucun admin ; un redémarrage ne modifie jamais un compte existant. `TTSCREEN_BOOTSTRAP_ADMIN_RESET=true` pour réinitialiser explicitement (récupération d'accès).
- **Journal d'audit transactionnel** : connexions réussies/échouées, jetons invalides (`auth_echouee`), accès refusés (`acces_refuse`), publications, affectations, gestion des écrans et des comptes… tracés avec auteur, date et IP, dans la même transaction que l'opération (`GET /api/audit`).
- **Limitation de débit renforcée** : fenêtre glissante par IP sur l'API, par jeton d'écran **et** par IP sur `/api/player/manifest`, limite stricte sur les connexions par IP **et par identifiant**, et comptage des jetons Bearer invalides par IP sur **toutes** les routes. `429` + `Retry-After` au-delà.
- **Envoi de médias pré-authentifié** : rôle editor exigé et `Content-Length` borné **avant** toute lecture du corps (garde ASGI) ; Caddy limite les corps à 1 Mo partout, 210 Mo seulement sur `POST /api/media`.
- **Médias servis de façon restreinte** : `/files/` n'accepte que les noms `<sha256>.<ext>` (application et Caddy), cache immuable, CSP `default-src 'none'`.
- **En-têtes de sécurité** : CSP stricte, `X-Content-Type-Options: nosniff`, `X-Frame-Options: SAMEORIGIN`, `Referrer-Policy: no-referrer`, `Cross-Origin-Resource-Policy: same-origin`. `/docs` coupé en production.
- **Jeton distinct par écran**, haché en base, renouvelable et révocable.
- **Données sensibles hors Git** : `.gitignore` (`.env`, `backend/data/`, `*.db`, médias, caches), script `scripts/untrack-generated-files.sh`, dépendances d'exécution épinglées (`requirements.txt`), outils de test séparés (`requirements-dev.txt`).
- **Conteneur API sans privilèges**, en lecture seule ; HTTPS via Caddy ; IP client fiable derrière le proxy (`--forwarded-allow-ips` configurable).

Limites actuelles, à connaître :
- La limitation de débit est **en mémoire** (un seul processus uvicorn) ; prévoir un compteur partagé (Redis) si vous passez à plusieurs workers.
- Les médias sous `/files/` sont lisibles sans authentification (noms non devinables). Des URL signées sont prévues.
- Le jeton de session saisi dans l'interface est conservé dans le `sessionStorage` du navigateur (effacé à la fermeture de l'onglet, pas de cookie httpOnly) ; la CSP réduit le risque d'XSS mais ne l'élimine pas.
- Aucun verrouillage de compte long terme : le blocage par identifiant dure une fenêtre (60 s).

Pour signaler une faille, n'ouvrez pas de ticket public : écrivez à l'adresse de la section [Contact](#contact).

---

## Contribuer

Les contributions sont les bienvenues : correctifs, tests, traductions, documentation, idées.

1. **Discutez d'abord** : ouvrez un ticket (bug ou proposition) avant un gros changement.
2. **Forkez** le dépôt et créez une branche : `git checkout -b ma-fonctionnalite`.
3. **Installez l'environnement** ([NixOS](#nixos) ou [hors NixOS](#hors-nixos-linux-macos-windowswsl)). Côté dépendances : `pip install -r backend/requirements.txt` pour l'exécution, `pip install -r backend/requirements-dev.txt` pour les tests.
4. **Écrivez le code et ses tests** : toute fonctionnalité doit avoir des tests automatisés dans `backend/tests/` (y compris un test d'authentification pour tout nouvel endpoint d'administration). Lancez-les avec `./scripts/test.sh` (ou `python -m pytest -q` depuis `backend/`). Une fonctionnalité sans test passant n'est pas considérée comme terminée.
5. **Vérifiez à la main** dans le navigateur : interface `/admin/`, prévisualisation, lecteur.
6. **Commitez** avec des messages clairs (une idée par commit), puis ouvrez une **demande de merge** qui décrit le problème, la solution et comment la tester.

Conventions :
- Aucun secret, jeton, mot de passe ni fichier `.env` dans Git (voir `.gitignore`).
- Backend : Python typé, code et messages d'erreur destinés à l'utilisateur en français. Dépendances d'exécution épinglées dans `requirements.txt` ; ne mettez pas d'outils de test dedans.
- Interface : pas d'injection de données dans le HTML (utilisez `textContent` ou les helpers existants), pas de dépendance externe chargée depuis un CDN.
- Les endpoints d'administration doivent rester protégés ; tout nouvel endpoint s'accompagne d'un test d'authentification.

Idées de contributions : cache hors ligne du lecteur, groupes d'écrans, migrations Alembic, portage de l'interface en React/TypeScript, planification horaire des playlists, compteur de débit partagé (Redis), URL signées pour les médias.

## FAQ

**Combien d'écrans peut-on gérer ?**
Le projet vise de 2 à 20 écrans. À cette échelle, la charge est très faible (environ 2 requêtes par seconde pour 20 écrans) Mais peut en gérer bien plus en fonction de la configuration materielle du serveur et de la bande passante disponible.

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

**J'ai perdu le jeton admin / je ne peux plus me connecter.**
Le jeton admin historique est la valeur de `TTSCREEN_ADMIN_TOKEN` dans votre `.env` (ou `deploy/.env` en production). Il cesse de fonctionner dès qu'un compte admin nommé actif existe : connectez-vous alors avec ce compte. Si vous avez perdu les deux : définissez `TTSCREEN_BOOTSTRAP_ADMIN_RESET=true` avec `TTSCREEN_BOOTSTRAP_ADMIN_USER`/`_PASSWORD`, redémarrez l'API (cela réinitialise ce compte), puis retirez ces variables.

**Pourquoi l'API refuse-t-elle de démarrer ?**
Depuis le patch de sécurité, `TTSCREEN_ADMIN_TOKEN` n'a plus de valeur par défaut : s'il est renseigné, il doit faire 24 caractères minimum et variés (sinon `RuntimeError` au démarrage, dans tous les environnements). En production, il faut en outre au moins un moyen de connexion (jeton ou compte d'amorçage).

**Les vidéos ne démarrent pas sur l'écran.**
Lancez Chromium avec `--autoplay-policy=no-user-gesture-required` et vérifiez que le format est MP4 (H.264) ou WebM lisible par le Raspberry Pi / mini-PC.

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

Pour me contacter pour une raison ou une autre ?

| Canal | Adresse |
|---|---|
| E-mail | `alexismarechalbrix@proton.me` |
| XMPP | `canyougetsomeburger@jabber.fr` |
| Tickets | <https://github.com/Michalex37220/TTScreen/issues> |
| Mon site internet | <https://alexismarechalbrix.page.gd/> |
# FREE MALLOC
