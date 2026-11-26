#!/usr/bin/env bash
# Restaure une sauvegarde. DESTRUCTIF : remplace la base et les médias actuels.
#   ./scripts/restore.sh backups/db-AAAA-MM-JJ-HHMM.sql.gz [backups/media-AAAA-MM-JJ-HHMM.tgz]
# Testez régulièrement une restauration (sur une machine de test) : une sauvegarde jamais restaurée n'est pas fiable.
set -euo pipefail
cd "$(dirname "$0")/.."
DB="${1:-}"; MEDIA="${2:-}"
[ -f "$DB" ] || { echo "Usage : $0 <db-….sql.gz> [media-….tgz]" >&2; exit 1; }
[ -z "$MEDIA" ] || [ -f "$MEDIA" ] || { echo "Fichier médias introuvable : $MEDIA" >&2; exit 1; }
gzip -t "$DB"
[ -z "$MEDIA" ] || tar tzf "$MEDIA" >/dev/null
COMPOSE=(docker compose -f deploy/docker-compose.prod.yml --env-file deploy/.env)

echo "Ceci va REMPLACER la base de données${MEDIA:+ et les médias} de ce serveur par la sauvegarde :"
echo "  $DB${MEDIA:+  +  $MEDIA}"
read -r -p "Tapez RESTAURER pour continuer : " answer
[ "$answer" = "RESTAURER" ] || { echo "Annulé."; exit 1; }

echo "==> Arrêt de l'API et de Caddy"
"${COMPOSE[@]}" stop caddy api
"${COMPOSE[@]}" up -d db
for _ in $(seq 1 30); do "${COMPOSE[@]}" exec -T db pg_isready -U ttscreen -d postgres >/dev/null 2>&1 && break; sleep 2; done

echo "==> Base de données"
"${COMPOSE[@]}" exec -T db psql -U ttscreen -d postgres -v ON_ERROR_STOP=1 \
  -c "DROP DATABASE IF EXISTS ttscreen WITH (FORCE)" -c "CREATE DATABASE ttscreen OWNER ttscreen"
gzip -dc "$DB" | "${COMPOSE[@]}" exec -T db psql -U ttscreen -d ttscreen -v ON_ERROR_STOP=1 -q >/dev/null

if [ -n "$MEDIA" ]; then
  echo "==> Médias"
  VOLUME=$(docker volume ls -q --filter label=com.docker.compose.project=ttscreen --filter label=com.docker.compose.volume=media | head -n1)
  [ -n "$VOLUME" ] || VOLUME=ttscreen_media
  docker run --rm -v "$VOLUME":/m -v "$(cd "$(dirname "$MEDIA")" && pwd)":/b:ro alpine \
    sh -c 'find /m -mindepth 1 -delete && tar xzf "/b/$0" -C /m' "$(basename "$MEDIA")"
fi

echo "==> Redémarrage"
"${COMPOSE[@]}" up -d
echo "Restauration terminée. Vérifiez : ${COMPOSE[*]} logs --tail 30 api"
