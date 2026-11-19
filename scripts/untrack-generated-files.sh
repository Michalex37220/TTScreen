#!/usr/bin/env bash
# Retire de l'index git (sans les supprimer du disque) les fichiers qui n'auraient jamais dû être suivis :
# secrets, base SQLite, médias, bytecode, caches. À lancer une fois, puis commit.
# NB : l'historique git garde les anciennes versions ; si un secret RÉEL y a figuré, le changer.
set -euo pipefail
cd "$(dirname "$0")/.."
git rm -r --cached --ignore-unmatch -q \
  deploy/.env backend/data backend/.pytest_cache backend/app/__pycache__ backend/tests/__pycache__
git status --short | head -20
echo "Fait. Vérifiez puis : git commit -m 'Retire secrets, données et artefacts du suivi git'"
