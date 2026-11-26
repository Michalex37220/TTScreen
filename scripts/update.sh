#!/usr/bin/env bash
# Met à jour une installation existante : sauvegarde, récupération du code, redéploiement.
# En cas de problème : ./scripts/rollback.sh
set -euo pipefail
cd "$(dirname "$0")/.."
echo "==> Sauvegarde préalable"
./scripts/backup.sh
echo "==> Récupération de la nouvelle version"
git pull --ff-only
echo "==> Redéploiement"
./scripts/deploy.sh "$@"
docker image prune -f >/dev/null
echo "Mise à jour terminée. Retour arrière possible : ./scripts/rollback.sh"
