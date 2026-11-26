#!/usr/bin/env bash
# Revient à l'image de l'API précédente (celle d'avant le dernier ./scripts/deploy.sh).
# Les données ne sont pas touchées. Si la nouvelle version avait modifié le schéma de base, restaurez
# aussi la sauvegarde (./scripts/restore.sh).
set -euo pipefail
cd "$(dirname "$0")/.."
docker image inspect ttscreen-api:previous >/dev/null 2>&1 || { echo "ERREUR : aucune image précédente." >&2; exit 1; }
docker tag ttscreen-api:previous ttscreen-api:latest
docker compose -f deploy/docker-compose.prod.yml --env-file deploy/.env up -d --no-build api
echo "Retour à l'image précédente effectué. Vérifiez : docker compose -f deploy/docker-compose.prod.yml logs --tail 30 api"
echo "Pensez à revenir aussi au code correspondant (git checkout <commit>) avant le prochain déploiement."
