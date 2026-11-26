#!/usr/bin/env bash
# Démarre l'API en local (SQLite + stockage local). Rechargement automatique si `cargo-watch` est installé.
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f .env ] && set -a && . ./.env && set +a
# Plus de jeton par défaut connu : on en génère un aléatoire pour cette session si aucun n'est fourni
if [ -z "${TTSCREEN_ADMIN_TOKEN:-}" ]; then
  TTSCREEN_ADMIN_TOKEN="$(head -c 32 /dev/urandom | base64 | tr '+/' '-_' | tr -d '=\n')"
  export TTSCREEN_ADMIN_TOKEN
  echo "Jeton admin (cette session) : $TTSCREEN_ADMIN_TOKEN"
fi
cd backend
mkdir -p data/media
export TTSCREEN_HOST="${TTSCREEN_HOST:-127.0.0.1}" TTSCREEN_PORT="${TTSCREEN_PORT:-8000}"
if command -v cargo-watch >/dev/null 2>&1; then
  exec cargo watch -x run
fi
exec cargo run
