#!/usr/bin/env bash
# Sauvegarde PostgreSQL + médias dans ./backups (à lancer sur le VPS, idéalement via cron).
set -euo pipefail
cd "$(dirname "$0")/.."
D=$(date +%F-%H%M); mkdir -p backups
docker compose -f deploy/docker-compose.prod.yml exec -T db pg_dump -U signage signage | gzip > "backups/db-$D.sql.gz"
docker run --rm -v signage_media:/m:ro -v "$PWD/backups":/b alpine tar czf "/b/media-$D.tgz" -C /m .
find backups -type f -mtime +14 -delete
echo "Sauvegarde OK : backups/db-$D.sql.gz, backups/media-$D.tgz"
