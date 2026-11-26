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
- **Écrans** : un jeton distinct par écran, révocable et renouvelable ; état connecté / déconnecté en direct ; renommage, suppression, changement de groupe.
- **Groupes d'écrans** : plusieurs écrans affichent le même contenu ; une seule affectation met à jour tout le groupe.
- **Planning horaire** : par écran ou par groupe, « tel contenu, tels jours, de HH:MM à HH:MM » (fuseau configurable) ; hors plage, retour au contenu habituel.
- **Tableau de bord** : écrans connectés ou non, écrans sans contenu, espace médias, en un coup d'œil.
- **Productivité** : duplication de playlist, changement de son propre mot de passe.
- **Lecteur** : lecture en boucle, passage automatique à l'élément suivant, récupération des nouvelles versions toutes les 10 secondes, sans coupure.

## Architecture

```
TTScreen/
├── backend/    API Rust (axum + SeaORM), tests `cargo test`
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
| Backend | Rust (édition 2021) : axum, tokio, SeaORM (dépendances verrouillées par `Cargo.lock`) |
| Base de données | SQLite en développement, PostgreSQL en production |
| Stockage | Fichiers sur disque, nommés par leur SHA-256 (`<sha256>.<ext>` uniquement) |
| Interface et lecteur | HTML + JavaScript, servis par l'API |
| Production | Docker Compose, Caddy (HTTPS automatique, sert les médias, borne les corps) |

Principe de diffusion : l'administrateur **publie** une version, l'**affecte** à un écran, et le lecteur de cet écran interroge régulièrement `GET /api/player/manifest` avec son jeton. Quand la version change, la nouvelle playlist démarre à la fin de l'élément en cours.

---

## Port Rust : ce qui change

Le backend a été réécrit de Python (FastAPI/SQLAlchemy) vers Rust (axum/SeaORM) **sans changer le contrat HTTP** : mêmes routes, mêmes codes de réponse, mêmes messages d'erreur en français, même format JSON ; l'interface `/admin/` et le lecteur `/player/` n'ont pas été modifiés. Les modules suivent l'ancien paquet `app/` (`config`, `db`, `models`, `schemas`, `auth`, `audit`, `deps`, `ratelimit`, `security`, `storage`, `publishing`, `api`).

- **Données existantes** : mêmes noms de tables et de colonnes, mêmes types ; les hachés de mots de passe (`pbkdf2_sha256$…`) sont compatibles (vérifié par un test sur un vecteur produit par `hashlib`). La reprise d'une base existante (SQLite ou PostgreSQL) n'a **pas encore été validée sur de vraies données** : sauvegardez d'abord (voir « Sauvegardes ») et essayez sur une copie.
- **Supprimé** : `/docs`, `/redoc`, `/openapi.json` (pas de génération automatique de schéma) ; `UVICORN_WORKERS` (inutile).
- **Ajouté** : `TTSCREEN_HOST`, `TTSCREEN_PORT`, `RUST_LOG`, sous-commande `ttscreen healthcheck` (sonde Docker sans curl/python), arrêt propre sur SIGTERM, `Cache-Control: immutable` sur `/files/`, détection d'envois identiques simultanés sans erreur 500.
- **Tests** : les scénarios des 39 tests pytest sont repris (mêmes assertions) (`backend/tests/api.rs`, `security.rs`) et complétés par des tests unitaires par module et des tests par fonctionnalité (`features.rs`).

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
2. **Entrer dans l'environnement de développement** (Rust, Node, Docker Compose, Caddy, ffmpeg) :
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

Prérequis : **Rust stable récent** (installé via [rustup](https://rustup.rs), 1.80 ou plus) et git. Aucun serveur de base de données n'est nécessaire en développement (SQLite embarqué).

```bash
git clone https://github.com/Michalex37220/TTScreen.git && cd TTScreen
cp .env.example .env
# Définissez TTSCREEN_ADMIN_TOKEN dans .env (24 car. min.) :
openssl rand -base64 32 | tr "+/" "-_" | tr -d "="

cd backend
cargo test                                # tests automatisés (unitaires + intégration)
cargo run                                 # démarre l'API sur http://127.0.0.1:8000
```

Lancez ces commandes **depuis le dossier `backend/`** : les chemins `./data/...` sont relatifs au dossier de lancement, donc la base et les médias sont créés dans `backend/data/` (ignoré par Git). Pour les déplacer, renseignez des chemins absolus dans `TTSCREEN_DATABASE_URL` et `TTSCREEN_MEDIA_DIR`. Sous Windows natif, préférez WSL.

Sur Linux/macOS, `./scripts/dev.sh` et `./scripts/test.sh` font la même chose. `dev.sh` génère automatiquement un jeton admin aléatoire pour la session si `TTSCREEN_ADMIN_TOKEN` n'est pas défini, et relance l'API à chaque modification si `cargo-watch` est installé (`cargo install cargo-watch`). Pour un binaire optimisé : `cargo build --release` (résultat : `backend/target/release/ttscreen`).

### Docker

Essai local avec SQLite (accessible seulement depuis votre machine) :

```bash
# TTSCREEN_ADMIN_TOKEN est obligatoire (plus de valeur par défaut) :
export TTSCREEN_ADMIN_TOKEN="$(openssl rand -base64 32 | tr '+/' '-_' | tr -d '=')"
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
| `TTSCREEN_DATABASE_URL` | `sqlite:///./data/ttscreen.db` | Base de données. PostgreSQL : `postgresql://utilisateur:mdp@hôte:5432/base` (le préfixe `postgresql+psycopg://` de la version Python est aussi accepté). |
| `TTSCREEN_MEDIA_DIR` | `./data/media` | Dossier de stockage des médias. |
| `TTSCREEN_MAX_UPLOAD_MB` | `200` | Taille maximale d'un fichier téléversé (contrôlée avant lecture du corps). |
| `TTSCREEN_ENV` | `development` | Mettre `production` pour exiger un moyen de connexion au démarrage. |
| `TTSCREEN_SESSION_TTL_H` | `12` | Durée de validité d'une session utilisateur, en heures. |
| `TTSCREEN_RATE_WINDOW_S` | `60` | Taille de la fenêtre glissante de limitation de débit, en secondes. |
| `TTSCREEN_RATE_LIMIT_API` | `600` | Requêtes / fenêtre / IP sur le reste de `/api` (`0` = illimité). |
| `TTSCREEN_RATE_LIMIT_PLAYER` | `120` | Requêtes / fenêtre / **jeton d'écran** sur `/api/player/manifest` (un quota par IP s'ajoute). |
| `TTSCREEN_RATE_LIMIT_LOGIN` | `20` | Tentatives de connexion / fenêtre / IP (anti force brute). |
| `TTSCREEN_RATE_LIMIT_AUTHFAIL` | `20` | Jetons Bearer invalides / fenêtre / IP avant blocage 429, sur **toutes** les routes. |
| `TTSCREEN_RATE_LIMIT_LOGIN_USER` | `10` | Échecs de connexion / fenêtre / **identifiant** avant blocage 429. |
| `TTSCREEN_FRONTEND_DIR` / `TTSCREEN_PLAYER_DIR` | dossiers du dépôt | Emplacement de l'interface et du lecteur (utile en conteneur). |
| `TTSCREEN_HOST` / `TTSCREEN_PORT` | `127.0.0.1` / `8000` | Adresse d'écoute (l'image Docker fixe `0.0.0.0`). *Ajout du port Rust.* |
| `FORWARDED_ALLOW_IPS` | `127.0.0.1` | Adresses ou réseaux (`10.0.0.0/8`) dont l'en-tête `X-Forwarded-For` est cru ; `*` = tous. Remplace l'option `--forwarded-allow-ips` d'uvicorn. |
| `TTSCREEN_TIMEZONE` | `Europe/Paris` | Fuseau horaire (nom IANA) dans lequel les plannings sont interprétés. Une valeur invalide empêche le démarrage. |
| `RUST_LOG` | `info` | Niveau de journalisation (`debug`, `warn`…). *Ajout du port Rust.* |
| ~~`UVICORN_WORKERS`~~ | — | Supprimée : un seul processus Rust exploite tous les cœurs. |

Générer un jeton solide : `openssl rand -base64 32 | tr "+/" "-_" | tr -d "="`. Ne commitez jamais `.env` (il est dans `.gitignore`).

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
1. Saisissez un nom (et, si vous voulez, un groupe) puis cliquez sur **Créer l'écran**.
2. **Copiez le jeton affiché : il ne l'est qu'une fois.** L'adresse à ouvrir sur l'écran est donnée avec.
3. Chaque écran indique **ce qu'il affiche actuellement et pourquoi** (contenu propre, groupe, planning). Choisissez un **contenu propre** et cliquez sur **Affecter** pour qu'il l'emporte sur celui du groupe.
4. **Renommer**, **Déplacer** (changer de groupe), **Planning**, **Voir l'écran** (aperçu exact), **Nouveau jeton**, **Révoquer** et **Supprimer** complètent la ligne.

**Groupes**
1. Onglet **Groupes** : donnez un nom et cliquez sur **Créer le groupe**.
2. Ajoutez des écrans avec **Ajouter au groupe** (ou choisissez le groupe à la création d'un écran).
3. Choisissez une version et cliquez sur **Affecter** : **tous les écrans du groupe** l'affichent. Depuis un brouillon publié, le bouton **Diffuser** de l'onglet Playlists propose aussi les groupes.
4. Supprimer un groupe ne supprime pas ses écrans : ils perdent seulement le contenu hérité.

**Priorité du contenu** (du plus précis au plus général) : planning de l'écran › contenu propre de l'écran › planning du groupe › contenu du groupe. Un écran qui a son propre contenu n'est donc pas affecté par les changements du groupe ; retirez son contenu propre (« — aucun — » puis **Affecter**) pour qu'il suive de nouveau le groupe.

**Planning** (bouton **Planning** d'un écran ou d'un groupe)
- Une règle = une version, des jours de la semaine et une plage horaire (début inclus, fin exclue ; « jusqu'à minuit » pour 24:00). Pour traverser minuit, créez deux règles.
- Hors des plages, l'écran revient à son contenu habituel. Si plusieurs règles se chevauchent, la plus récente l'emporte.
- Les heures sont celles du **fuseau du serveur** (`TTSCREEN_TIMEZONE`, par défaut `Europe/Paris`), heure d'été comprise.
- Le lecteur interroge le serveur toutes les 10 secondes : le changement de contenu est effectif en moins d'une minute, sans coupure.

**Tableau de bord** : écrans connectés, écrans sans contenu (à surveiller), nombre de groupes, playlists, règles de planning et espace occupé par les médias.

**Mon mot de passe** (en haut à droite, comptes nommés) : exige l'ancien mot de passe et déconnecte vos autres sessions.

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

Le port Rust n'embarque pas de Swagger : cette section fait foi (`/docs`, `/redoc` et `/openapi.json` n'existent pas). Les messages d'erreur sont en français.

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

#### Changer son propre mot de passe

```http
  POST /api/auth/password
```

| Parameter          | Type     | Description                                            |
| :----------------- | :------- | :----------------------------------------------------- |
| `current_password` | `string` | **Required**. Mot de passe actuel                      |
| `new_password`     | `string` | **Required**. Nouveau mot de passe (8 à 255 caractères) |

`204` en cas de succès ; les **autres** sessions du compte sont révoquées. `403` si le mot de passe actuel est faux (les échecs comptent dans le quota de connexion par identifiant : `429` au-delà). `400` avec le jeton admin historique (pas de mot de passe).

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

#### Dupliquer une playlist

```http
  POST /api/playlists/${id}/duplicate
```

Rôle editor. Crée « Copie de … » avec les mêmes éléments (sans aucune version) et renvoie `201` avec la nouvelle playlist.

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

Un écran a la forme :

```json
{ "id": 2, "name": "Entrée", "version_id": null, "online": true, "last_seen": "2026-03-02T09:00:01",
  "has_token": true, "group_id": 1, "group_name": "Hall",
  "effective_version_id": 7, "effective_source": "group" }
```

- `version_id` : contenu **propre** de l'écran (prioritaire sur le groupe).
- `effective_version_id` / `effective_source` : ce que l'écran affiche **à cet instant** ; source : `screen_schedule`, `screen`, `group_schedule`, `group` ou `null` (rien).
- `online` vaut `true` si le lecteur a interrogé le serveur dans les 60 dernières secondes ; `last_seen` est en UTC.

| Requête | Rôle | Description |
| :------ | :--- | :---------- |
| `POST /api/screens` | editor | Corps `{"name", "group_id"?}` (`name` 1 à 120 caractères). `201` avec l'écran et son **`token`, affiché une seule fois**. `404` si le groupe n'existe pas. |
| `GET /api/screens` | viewer | Liste des écrans. |
| `PUT /api/screens/${id}` | editor | Renomme : `{"name"}`. |
| `DELETE /api/screens/${id}` | editor | Supprime l'écran et ses règles de planning ; son jeton cesse de fonctionner. `204`. |
| `POST /api/screens/${id}/group` | editor | `{"group_id": 3}` place l'écran dans un groupe, `{"group_id": null}` l'en sort. Champ obligatoire. |
| `POST /api/screens/${id}/assign` | editor | `{"version_id": 5}` fixe un contenu propre ; `null` le retire (l'écran suit alors son groupe). Revenir en arrière = affecter une ancienne version. |
| `GET /api/screens/${id}/manifest` | viewer | Ce que voit l'écran à cet instant (groupe et planning compris), au [format manifeste](#format-du-manifeste), **sans** le marquer connecté. `404` si rien à afficher. |
| `POST /api/screens/${id}/rotate-token` | editor | Nouveau jeton (renvoyé une seule fois) ; l'ancien cesse de fonctionner immédiatement. |
| `POST /api/screens/${id}/revoke-token` | editor | L'écran ne peut plus se connecter ; pour le réactiver, générez un nouveau jeton. |

Identifiants : entiers de 1 à 2³¹−1 (sinon `422`).

### Groupes d'écrans

Un groupe : `{"id", "name", "version_id", "screen_count"}`. Le nom est unique (`409` sinon).

| Requête | Rôle | Description |
| :------ | :--- | :---------- |
| `GET /api/groups` | viewer | Liste, triée par nom. |
| `POST /api/groups` | editor | `{"name"}` (1 à 120 caractères). `201`. |
| `PUT /api/groups/${id}` | editor | Renomme : `{"name"}`. |
| `POST /api/groups/${id}/assign` | editor | `{"version_id": 5 \| null}` : version diffusée sur tout le groupe. Les écrans qui ont un contenu propre ou un planning actif la gardent. |
| `DELETE /api/groups/${id}` | editor | Supprime le groupe et ses règles de planning ; ses écrans sont conservés (sans groupe). `204`. |

Supprimer une playlist est refusé (`409`) si l'une de ses versions est affectée à un écran, à un groupe ou utilisée dans un planning.

### Planning

Une règle : `{"id", "screen_id", "group_id", "version_id", "days", "start", "end", "created_at"}` ; exactement l'un de `screen_id` / `group_id` est renseigné. `days` : liste de 0 (lundi) à 6 (dimanche). `start` / `end` : `HH:MM` en heure du fuseau du serveur ; `end` peut valoir `24:00` ; `end` doit être postérieure à `start`.

| Requête | Rôle | Description |
| :------ | :--- | :---------- |
| `GET /api/schedules` | viewer | Toutes les règles. |
| `POST /api/schedules` | editor | `{"screen_id" \| "group_id", "version_id", "days", "start", "end"}`. `201`. `404` si la cible ou la version n'existe pas, `409` au-delà de 100 règles par cible. |
| `DELETE /api/schedules/${id}` | editor | `204`. |

Priorité du contenu d'un écran : planning de l'écran › contenu propre › planning du groupe › contenu du groupe ; si plusieurs règles d'une même cible sont actives, la plus récente l'emporte.

### Tableau de bord

```http
  GET /api/stats
```

Rôle viewer. Renvoie `{"screens": {"total", "online", "offline", "without_content"}, "groups", "playlists", "versions", "schedules", "media": {"count", "bytes"}}`.

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

> **Guide complet pas à pas : [`docs/DEPLOIEMENT.md`](docs/DEPLOIEMENT.md)** (préparation du serveur, pare-feu, sauvegardes planifiées, mise à jour, retour arrière, installation sans Docker, dépannage).

Architecture : **Caddy** (HTTPS automatique, sert directement les médias, borne les corps : 1 Mo partout sauf 210 Mo sur `POST /api/media`) → **API Rust** (conteneur Alpine, lecture seule, sans privilèges, port non publié) → **PostgreSQL** (non publié). Seuls les ports 80 et 443 sont exposés. Journaux avec rotation, limites mémoire, vérifications de santé et ordre de démarrage sont déjà configurés dans `deploy/docker-compose.prod.yml`.

Résumé :

```bash
git clone https://github.com/Michalex37220/TTScreen.git && cd TTScreen
cp deploy/.env.example deploy/.env && chmod 600 deploy/.env
nano deploy/.env                 # DOMAIN, POSTGRES_PASSWORD, et un compte d'amorçage (ou un jeton admin)
./scripts/deploy.sh              # vérifie la configuration, construit, démarre, teste le HTTPS
```

`./scripts/deploy.sh` refuse de démarrer si la configuration est dangereuse ou incomplète (domaine d'exemple, mot de passe de base trop court ou contenant des caractères qui casseraient l'URL, jeton faible, compte d'amorçage incomplet). Sur un petit serveur (≤ 2 Go de RAM), la compilation Rust peut manquer de mémoire : construisez l'image sur votre poste avec `./scripts/ship-image.sh utilisateur@serveur`.

En production (`TTSCREEN_ENV=production`) :
- l'API **refuse de démarrer** si `TTSCREEN_ADMIN_TOKEN` est renseigné mais fait moins de 24 caractères ou est trop répétitif, et **exige au moins un moyen de connexion** (jeton ou compte d'amorçage) ;
- Caddy transmet `X-Forwarded-For` de confiance (`FORWARDED_ALLOW_IPS="*"`, l'API n'étant pas publiée) pour que la limitation de débit voie la vraie IP des clients.

Ne publiez jamais le port 8000 ni celui de PostgreSQL. Validez le Caddyfile avant déploiement : `caddy validate --config deploy/Caddyfile`.

> Après un `git pull` sur une installation existante, lancez **une fois** `./scripts/untrack-generated-files.sh` puis committez, afin de retirer secrets, base SQLite et artefacts du suivi Git. Si un secret réel a déjà été commité, changez-le : l'historique Git le conserve.

## Sauvegardes et mises à jour

| Action | Commande |
|---|---|
| Sauvegarde (base + médias, vérifiée, rotation 14 jours) | `./scripts/backup.sh` |
| Sauvegarde copiée hors serveur | `BACKUP_REMOTE=utilisateur@hôte:/chemin/ ./scripts/backup.sh` |
| Planification quotidienne | timer systemd fourni : `deploy/systemd/ttscreen-backup.timer` |
| Restauration (destructive, confirmation demandée) | `./scripts/restore.sh backups/db-….sql.gz [backups/media-….tgz]` |
| Mise à jour (sauvegarde + `git pull` + redéploiement) | `./scripts/update.sh` |
| Retour à la version précédente | `./scripts/rollback.sh` |

Une sauvegarde qui reste sur le même disque ne protège pas d'une panne du serveur : configurez `BACKUP_REMOTE` et **testez une restauration** avant d'en avoir besoin. Il n'y a pas encore de migrations automatiques du schéma : lisez les notes de version (et `docs/SECURITY_PATCH.md`) avant de mettre à jour.

## Sécurité

Voir aussi [`docs/SECURITY_PATCH.md`](docs/SECURITY_PATCH.md) pour le détail des correctifs de l'audit de sécurité.

Déjà en place :
- **Plus de jeton admin par défaut** : `TTSCREEN_ADMIN_TOKEN` vide = désactivé ; s'il est renseigné, 24 caractères minimum et variés exigés **dans tous les environnements** (démarrage refusé sinon). En production, l'API refuse de démarrer sans aucun moyen de connexion.
- **Jeton historique à extinction automatique** : refusé dès qu'un compte admin nommé actif existe (sauf `TTSCREEN_LEGACY_TOKEN_ALWAYS=true`) ; apparaît comme `legacy-admin` (nom réservé) dans le journal.
- **Comptes utilisateurs et rôles** : `admin` (tout), `editor` (contenu, publication, écrans), `viewer` (lecture seule). Mots de passe hachés en PBKDF2-HMAC-SHA256 (600 000 itérations, jamais en clair), vérification toujours exécutée (haché factice) pour ne pas divulguer l'existence d'un identifiant par le temps de réponse. Sessions stockées hachées, révoquables et à durée limitée (`TTSCREEN_SESSION_TTL_H`).
- **Amorçage sans écrasement** : le compte `TTSCREEN_BOOTSTRAP_ADMIN_*` n'est créé que s'il n'existe aucun admin ; un redémarrage ne modifie jamais un compte existant. `TTSCREEN_BOOTSTRAP_ADMIN_RESET=true` pour réinitialiser explicitement (récupération d'accès).
- **Journal d'audit transactionnel** : connexions réussies/échouées, jetons invalides (`auth_echouee`), accès refusés (`acces_refuse`), publications, affectations, gestion des écrans et des comptes… tracés avec auteur, date et IP, dans la même transaction que l'opération (`GET /api/audit`).
- **Limitation de débit renforcée** : fenêtre glissante par IP sur l'API, par jeton d'écran **et** par IP sur `/api/player/manifest`, limite stricte sur les connexions par IP **et par identifiant**, et comptage des jetons Bearer invalides par IP sur **toutes** les routes. `429` + `Retry-After` au-delà.
- **Envoi de médias pré-authentifié** : rôle editor exigé et `Content-Length` borné **avant** toute lecture du corps (middleware `upload_guard`) ; Caddy limite les corps à 1 Mo partout, 210 Mo seulement sur `POST /api/media`.
- **Médias servis de façon restreinte** : `/files/` n'accepte que les noms `<sha256>.<ext>` (application et Caddy), cache immuable, CSP `default-src 'none'`.
- **En-têtes de sécurité** : CSP stricte, `X-Content-Type-Options: nosniff`, `X-Frame-Options: SAMEORIGIN`, `Referrer-Policy: no-referrer`, `Cross-Origin-Resource-Policy: same-origin`.
- **Jeton distinct par écran**, haché en base, renouvelable et révocable.
- **Données sensibles hors Git** : `.gitignore` (`.env`, `backend/data/`, `*.db`, médias, caches), script `scripts/untrack-generated-files.sh`, dépendances verrouillées (`Cargo.lock`), outils de test séparés (`[dev-dependencies]`, absents du binaire livré).
- **Conteneur API sans privilèges**, en lecture seule ; HTTPS via Caddy ; IP client fiable derrière le proxy (`FORWARDED_ALLOW_IPS` configurable).

Limites actuelles, à connaître :
- La limitation de débit est **en mémoire** (un seul processus) ; prévoir un compteur partagé (Redis) si vous déployez plusieurs instances derrière un répartiteur.
- Les médias sous `/files/` sont lisibles sans authentification (noms non devinables). Des URL signées sont prévues.
- Le jeton de session saisi dans l'interface est conservé dans le `sessionStorage` du navigateur (effacé à la fermeture de l'onglet, pas de cookie httpOnly) ; la CSP réduit le risque d'XSS mais ne l'élimine pas.
- Aucun verrouillage de compte long terme : le blocage par identifiant dure une fenêtre (60 s).

Pour signaler une faille, n'ouvrez pas de ticket public : écrivez à l'adresse de la section [Contact](#contact).

---

## Contribuer

Les contributions sont les bienvenues : correctifs, tests, traductions, documentation, idées.

1. **Discutez d'abord** : ouvrez un ticket (bug ou proposition) avant un gros changement.
2. **Forkez** le dépôt et créez une branche : `git checkout -b ma-fonctionnalite`.
3. **Installez l'environnement** ([NixOS](#nixos) ou [hors NixOS](#hors-nixos-linux-macos-windowswsl)). Côté dépendances : tout passe par `cargo` (`backend/Cargo.toml`).
4. **Écrivez le code et ses tests** : toute fonctionnalité doit avoir des tests automatisés dans `backend/tests/` (y compris un test d'authentification pour tout nouvel endpoint d'administration). Lancez-les avec `./scripts/test.sh` (ou `cargo test` depuis `backend/`) ; `cargo clippy --all-targets` et `cargo fmt --check` doivent aussi passer. Une fonctionnalité sans test passant n'est pas considérée comme terminée.
5. **Vérifiez à la main** dans le navigateur : interface `/admin/`, prévisualisation, lecteur.
6. **Commitez** avec des messages clairs (une idée par commit), puis ouvrez une **demande de merge** qui décrit le problème, la solution et comment la tester.

Conventions :
- Aucun secret, jeton, mot de passe ni fichier `.env` dans Git (voir `.gitignore`).
- Backend : Rust, code commenté et messages d'erreur destinés à l'utilisateur en français. Outils de test uniquement dans `[dev-dependencies]` ; commitez `Cargo.lock`.
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
