#!/usr/bin/env bash
# Sauvegarde PostgreSQL + médias (à lancer sur le serveur ; planifiée par un timer systemd ou cron,
# voir docs/DEPLOIEMENT.md). À effectuer aussi avant chaque mise à jour (update.sh le fait).
#
# Variables facultatives :
#   BACKUP_DIR=./backups        dossier de destination
#   BACKUP_KEEP_DAYS=14         durée de conservation locale
#   BACKUP_REMOTE=user@hôte:/chemin/   copie hors serveur via rsync/SSH (FORTEMENT recommandé : une
#                                       sauvegarde sur le même disque ne protège pas d'une panne du serveur)
set -euo pipefail
cd "$(dirname "$0")/.."
umask 077
COMPOSE=(docker compose -f deploy/docker-compose.prod.yml --env-file deploy/.env)
DIR="${BACKUP_DIR:-backups}"
KEEP="${BACKUP_KEEP_DAYS:-14}"
STAMP=$(date +%F-%H%M)
DB="$DIR/db-$STAMP.sql.gz"
MEDIA="$DIR/media-$STAMP.tgz"
mkdir -p "$DIR"

echo "==> Base de données"
"${COMPOSE[@]}" exec -T db pg_dump -U ttscreen ttscreen | gzip > "$DB.part"
gzip -t "$DB.part"                                   # archive lisible
[ "$(gzip -dc "$DB.part" | wc -c)" -gt 1000 ] || { rm -f "$DB.part"; echo "ERREUR : dump suspect (trop petit)." >&2; exit 1; }
mv "$DB.part" "$DB"

echo "==> Médias"
VOLUME=$(docker volume ls -q --filter label=com.docker.compose.project=ttscreen --filter label=com.docker.compose.volume=media | head -n1)
[ -n "$VOLUME" ] || VOLUME=ttscreen_media
docker run --rm -v "$VOLUME":/m:ro -v "$PWD/$DIR":/b alpine tar czf "/b/media-$STAMP.tgz.part" -C /m .
tar tzf "$MEDIA.part" >/dev/null                     # archive lisible
mv "$MEDIA.part" "$MEDIA"

find "$DIR" -maxdepth 1 -type f \( -name 'db-*.sql.gz' -o -name 'media-*.tgz' \) -mtime +"$KEEP" -delete

if [ -n "${BACKUP_REMOTE:-}" ]; then
  echo "==> Copie hors serveur vers $BACKUP_REMOTE"
  rsync -a --partial "$DB" "$MEDIA" "$BACKUP_REMOTE"
else
  echo "NB : BACKUP_REMOTE non défini, les sauvegardes restent sur ce serveur uniquement." >&2
fi
echo "Sauvegarde OK : $DB ($(du -h "$DB" | cut -f1)), $MEDIA ($(du -h "$MEDIA" | cut -f1))"
