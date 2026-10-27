#!/usr/bin/env bash
# Démarre l'API en local (SQLite + stockage local), rechargement automatique.
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f .env ] && set -a && . ./.env && set +a
mkdir -p data/media
cd backend
exec python -m uvicorn app.main:app --reload --host 127.0.0.1 --port 8000
