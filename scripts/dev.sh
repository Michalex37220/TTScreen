#!/usr/bin/env bash
# Démarre l'API en local (SQLite + stockage local), rechargement automatique.
# version de développement quasiment jamais mis a jour car j'utiliser le protoc définie dans le README
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f .env ] && set -a && . ./.env && set +a
# Plus de jeton par défaut connu : on en génère un aléatoire pour cette session si aucun n'est fourni
if [ -z "${TTSCREEN_ADMIN_TOKEN:-}" ]; then
  TTSCREEN_ADMIN_TOKEN="$(python3 -c 'import secrets; print(secrets.token_urlsafe(32))')"
  export TTSCREEN_ADMIN_TOKEN
  echo "Jeton admin (cette session) : $TTSCREEN_ADMIN_TOKEN"
fi
mkdir -p data/media
cd backend
exec python -m uvicorn app.main:app --reload --host 127.0.0.1 --port 8000
