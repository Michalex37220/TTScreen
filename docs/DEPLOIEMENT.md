# Déployer TTScreen sur un serveur externe

Ce guide mène d'un serveur vierge (VPS Debian/Ubuntu) à une installation en production : HTTPS automatique,
base PostgreSQL, sauvegardes planifiées, mises à jour et retour arrière.

> **Avant de commencer** : ces procédures et scripts n'ont pas pu être exécutés sur un vrai serveur au moment de leur
> écriture (seule la logique de contrôle de `deploy.sh` a été testée, avec un Docker simulé). Faites un premier essai
> sur un serveur jetable, notez ce qui diffère chez vous, et corrigez avant d'y mettre des données réelles.

```
Internet ──► Caddy (80/443, HTTPS auto) ──► API Rust (port 8000, non publié) ──► PostgreSQL (non publié)
                 │
                 └─► sert directement /files/* depuis le volume des médias (lecture seule)
```

## 0. Ce qu'il vous faut

| Élément | Recommandation |
|---|---|
| Serveur | 1 vCPU, 2 Go de RAM, 20 Go de disque (les médias consomment l'essentiel). Debian 12 / Ubuntu 22.04+, accès SSH. |
| Nom de domaine | Un sous-domaine (ex. `tv.monsite.fr`) dont vous pouvez modifier le DNS. |
| Ports | 80 et 443 joignables depuis Internet (80 sert à l'obtention du certificat). |
| Poste de travail | Facultatif : sert à compiler l'image si le serveur a peu de RAM (voir étape 5). |

## 1. DNS

Créez un enregistrement **A** (et **AAAA** si le serveur a une IPv6) pour votre domaine vers l'adresse du serveur.
Vérifiez la propagation : `dig +short tv.monsite.fr` doit renvoyer l'IP du serveur. Sans cela, Caddy ne pourra pas
obtenir de certificat.

## 2. Sécuriser le serveur

Connectez-vous en root (ou avec le compte fourni par l'hébergeur), puis :

```bash
# Utilisateur d'administration (remplacez « alex »)
adduser alex && usermod -aG sudo alex
# Copiez votre clé SSH publique pour cet utilisateur (depuis votre poste) :
#   ssh-copy-id alex@IP_DU_SERVEUR
```

**Testez la connexion `ssh alex@IP_DU_SERVEUR` dans un second terminal avant de continuer**, puis désactivez le
mot de passe et la connexion root :

```bash
sudo sed -i 's/^#\?PasswordAuthentication.*/PasswordAuthentication no/' /etc/ssh/sshd_config
sudo sed -i 's/^#\?PermitRootLogin.*/PermitRootLogin no/' /etc/ssh/sshd_config
sudo systemctl reload ssh      # (« sshd » sur certaines distributions)
```

Pare-feu et mises à jour automatiques de sécurité :

```bash
sudo apt update && sudo apt install -y ufw unattended-upgrades fail2ban
sudo ufw default deny incoming && sudo ufw default allow outgoing
sudo ufw allow OpenSSH            # AVANT d'activer le pare-feu, sinon vous vous coupez l'accès
sudo ufw allow 80/tcp && sudo ufw allow 443/tcp && sudo ufw allow 443/udp   # 443/udp = HTTP/3
sudo ufw enable
sudo dpkg-reconfigure -plow unattended-upgrades
```

> **Docker contourne `ufw`** pour les ports qu'il publie. C'est sans conséquence ici car `docker-compose.prod.yml`
> ne publie que 80 et 443. **N'ajoutez jamais** de `ports:` aux services `api` ou `db`.

Serveur à 1 Go de RAM ou moins : ajoutez du swap (`sudo fallocate -l 2G /swapfile && sudo chmod 600 /swapfile &&
sudo mkswap /swapfile && sudo swapon /swapfile`, puis une ligne `/swapfile none swap sw 0 0` dans `/etc/fstab`).

## 3. Installer Docker

```bash
curl -fsSL https://get.docker.com | sudo sh
sudo usermod -aG docker alex      # puis déconnectez-vous / reconnectez-vous
docker compose version            # doit afficher la v2
```

## 4. Récupérer le code et configurer

```bash
git clone https://github.com/Michalex37220/TTScreen.git && cd TTScreen
cp deploy/.env.example deploy/.env
chmod 600 deploy/.env
```

Générez les secrets (à lancer pour chaque valeur) :

```bash
openssl rand -base64 32 | tr '+/' '-_' | tr -d '='
```

Renseignez `deploy/.env` :

| Variable | Valeur |
|---|---|
| `DOMAIN` | votre domaine, **sans** `https://` |
| `POSTGRES_PASSWORD` | un secret généré ci-dessus (lettres, chiffres, `_ . - ~` uniquement : il est placé dans une URL) |
| `TTSCREEN_BOOTSTRAP_ADMIN_USER` / `_PASSWORD` | votre compte administrateur initial (mot de passe : 8 caractères minimum, mieux : long et unique) |

`TTSCREEN_ADMIN_TOKEN` (jeton historique) est facultatif quand un compte d'amorçage est défini ; laissez-le vide.

## 5. Déployer

```bash
./scripts/deploy.sh
```

Le script vérifie la configuration (et s'arrête avec un message clair si quelque chose est dangereux), construit l'image,
démarre les trois services, attend que l'API soit saine, puis teste `https://VOTRE-DOMAINE/api/health`.
La première compilation Rust prend plusieurs minutes.

**Petit serveur (≤ 2 Go) : compiler sur votre poste.** La compilation Rust peut manquer de mémoire. Depuis votre poste
(Docker installé, même architecture que le serveur) :

```bash
./scripts/ship-image.sh alex@IP_DU_SERVEUR TTScreen
```

L'image est construite localement, envoyée par SSH, puis le serveur lance `deploy.sh --no-build`.

## 6. Première connexion et comptes

1. Ouvrez `https://VOTRE-DOMAINE/admin/` et connectez-vous avec le compte d'amorçage.
2. Onglet **Comptes** : créez un compte nominatif par personne (rôles `viewer`, `editor`, `admin`).
3. **Retirez `TTSCREEN_BOOTSTRAP_ADMIN_PASSWORD` de `deploy/.env`** puis `./scripts/deploy.sh`. Le compte existe en base ;
   le mot de passe n'a plus de raison de rester sur le disque. (Mot de passe perdu : remettez-le avec
   `TTSCREEN_BOOTSTRAP_ADMIN_RESET=true`, redéployez, puis retirez les deux lignes.)
4. Créez vos écrans dans l'interface : chaque écran reçoit un jeton affiché **une seule fois**. Les URL des lecteurs
   sont décrites dans le README (« Les adresses du lecteur », « Brancher un écran »).

## 7. Sauvegardes (à ne pas sauter)

Test manuel :

```bash
./scripts/backup.sh
ls -lh backups/
```

Planification quotidienne (timer systemd ; adaptez le chemin `/opt/TTScreen` dans le fichier `.service` si le dépôt est ailleurs) :

```bash
sudo cp deploy/systemd/ttscreen-backup.* /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now ttscreen-backup.timer
systemctl list-timers | grep ttscreen
```

**Copie hors serveur** : une sauvegarde sur le même disque ne protège pas d'une panne ou d'une suppression du serveur.
Créez une clé SSH dédiée vers une autre machine, puis décommentez la ligne `Environment=BACKUP_REMOTE=…` du fichier `.service`
(ou lancez `BACKUP_REMOTE=utilisateur@hôte:/chemin/ ./scripts/backup.sh`).

**Testez une restauration** sur une machine de test, au moins une fois :

```bash
./scripts/restore.sh backups/db-AAAA-MM-JJ-HHMM.sql.gz backups/media-AAAA-MM-JJ-HHMM.tgz
```

## 7 bis. Fuseau horaire des plannings

Les règles de planning (« du lundi au vendredi de 8 h à 18 h ») sont interprétées dans le fuseau `TTSCREEN_TIMEZONE`
(défaut `Europe/Paris`, heure d'été comprise). Réglez-le dans `deploy/.env` (nom IANA : `Europe/Paris`,
`America/Montreal`…) ; une valeur invalide empêche l'API de démarrer, avec un message explicite.
Les écrans d'un même serveur partagent ce fuseau : pour des sites dans des fuseaux différents, utilisez un serveur par fuseau.

## 8. Mises à jour et retour arrière

```bash
./scripts/update.sh      # sauvegarde, git pull, redéploiement
./scripts/rollback.sh    # revient à l'image précédente si la nouvelle version pose problème
```

Il n'y a pas d'outil de migration général : lisez les notes de version avant de mettre à jour. La seule évolution de schéma à ce jour (colonne `screens.group_id`, tables `screen_groups` et `schedule_rules` pour les groupes et le planning) est appliquée **automatiquement et sans perte** au premier démarrage de la nouvelle version ; faites néanmoins une sauvegarde avant (`update.sh` la fait). Si une version
modifie le schéma, un retour arrière peut nécessiter aussi `restore.sh` avec la sauvegarde faite par `update.sh`.

## 9. Supervision

- **Disponibilité** : surveillez `https://VOTRE-DOMAINE/api/health` avec un service externe (UptimeRobot, Uptime Kuma…).
- **État** : `docker compose -f deploy/docker-compose.prod.yml ps` (les trois services doivent être `Up`/`healthy`).
- **Journaux** : `docker compose -f deploy/docker-compose.prod.yml logs -f api` (rotation 3 × 10 Mo par service).
- **Écrans hors ligne** : l'interface les affiche (un écran est « en ligne » s'il a contacté le serveur dans la dernière minute).
- **Disque** : `df -h` et `docker system df` ; les médias et les sauvegardes sont ce qui grossit.
- **Journal d'audit** : dans l'onglet **Comptes** (rôle admin) ou `GET /api/audit` : connexions, échecs, publications, accès refusés.

## 10. Liste de contrôle avant ouverture au public

- [ ] `https://VOTRE-DOMAINE/api/health` répond `{"status":"ok"}` avec un certificat valide
- [ ] Le mot de passe d'amorçage a été retiré de `deploy/.env` (et `RESET`/`LEGACY_TOKEN_ALWAYS` ne sont pas à `true`)
- [ ] `deploy/.env` est en `chmod 600` et absent de Git (`git status` ne doit pas le montrer)
- [ ] `sudo ufw status` : seuls SSH, 80, 443 ouverts ; connexion SSH par mot de passe désactivée
- [ ] `ss -tlnp` : rien d'autre que SSH, 80 et 443 n'écoute sur l'extérieur (ni 8000, ni 5432)
- [ ] Une sauvegarde existe **et a été restaurée avec succès** une fois, copie hors serveur configurée
- [ ] Un écran de test affiche bien une playlist publiée
- [ ] La surveillance externe est active

## 11. Restreindre l'administration à votre réseau (facultatif)

Les écrans n'ont besoin que de `/player/`, `/files/` et `/api/player/manifest`. Dans `deploy/Caddyfile`, un bloc commenté
(`@admin_interdit`) permet de réserver `/admin/` et le reste de `/api` à vos adresses IP. Décommentez-le, adaptez les
adresses, puis `docker compose -f deploy/docker-compose.prod.yml exec caddy caddy reload --config /etc/caddy/Caddyfile`.

## 12. Installation sans Docker (systemd + Caddy)

Pour un serveur où Docker n'est pas souhaité. SQLite est suffisant pour quelques dizaines d'écrans.

```bash
# Compilation (sur le serveur si assez de RAM, sinon sur un poste de même architecture puis copie du binaire)
cd backend && cargo build --release

# Utilisateur et dossiers
sudo useradd --system --home /var/lib/ttscreen --shell /usr/sbin/nologin ttscreen
sudo install -d -o root -g root /opt/ttscreen/bin /etc/ttscreen
sudo install -m 755 target/release/ttscreen /opt/ttscreen/bin/ttscreen
sudo cp -r ../frontend ../player /opt/ttscreen/

# Configuration (voir les commentaires du fichier)
sudo cp ../deploy/native/ttscreen.env.example /etc/ttscreen/ttscreen.env
sudo chown root:ttscreen /etc/ttscreen/ttscreen.env && sudo chmod 640 /etc/ttscreen/ttscreen.env
sudo nano /etc/ttscreen/ttscreen.env

# Service
sudo cp ../deploy/native/ttscreen.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now ttscreen
journalctl -u ttscreen -f

# HTTPS : installez Caddy (https://caddyserver.com/docs/install), puis
sudo cp ../deploy/native/Caddyfile /etc/caddy/Caddyfile   # remplacez le domaine
sudo systemctl reload caddy
```

Sauvegarde (SQLite) : arrêtez ou utilisez la sauvegarde en ligne, par exemple
`sqlite3 /var/lib/ttscreen/ttscreen.db ".backup '/chemin/sauvegarde.db'"`, et archivez `/var/lib/ttscreen/media`.
Mise à jour : recompilez, remplacez `/opt/ttscreen/bin/ttscreen`, `sudo systemctl restart ttscreen`.

## Dépannage

| Symptôme | Piste |
|---|---|
| `deploy.sh` refuse de démarrer | Lisez le message : il désigne la variable de `deploy/.env` à corriger. |
| Pas de certificat / erreur TLS | DNS (`dig +short DOMAINE`), ports 80/443 ouverts chez l'hébergeur **et** dans `ufw`, `docker compose … logs caddy`. Let's Encrypt limite les essais : attendez avant de réessayer en boucle. |
| L'API ne devient pas saine | `docker compose -f deploy/docker-compose.prod.yml logs --tail 50 api` : « Aucun moyen de se connecter » = renseigner le compte d'amorçage ou le jeton ; erreur de base = `POSTGRES_PASSWORD` changé après la première création du volume (la base garde l'ancien mot de passe). |
| `POSTGRES_PASSWORD` changé et la base refuse | Le mot de passe n'est lu qu'à la création du volume `pgdata`. Changez-le dans la base : `docker compose … exec db psql -U ttscreen -c "ALTER USER ttscreen PASSWORD '…'"`. |
| Compilation tuée (`signal: 9`, « killed ») | Mémoire insuffisante : ajoutez du swap, ou utilisez `./scripts/ship-image.sh`. |
| 429 « Trop de requêtes » pour tout un site | Les clients d'un même réseau partagent une IP. Augmentez `TTSCREEN_RATE_LIMIT_API` dans `deploy/.env` (les écrans ont en plus leur quota propre). |
| Envoi d'un gros média refusé (413) | Alignez `TTSCREEN_MAX_UPLOAD_MB` et `UPLOAD_MAX_BODY` (≈ +5 %, en `MB`) dans `deploy/.env`, puis `./scripts/deploy.sh`. |
| Tous les clients ont la même IP dans le journal d'audit | `FORWARDED_ALLOW_IPS` doit être `*` dans le compose de production (c'est le cas par défaut), l'API n'étant joignable que par Caddy. |
