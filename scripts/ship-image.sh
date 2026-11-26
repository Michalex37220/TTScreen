#!/usr/bin/env bash
# Construit l'image sur VOTRE poste et l'envoie au serveur par SSH. Utile sur un petit VPS (≤ 2 Go de RAM) :
# la compilation Rust y échoue souvent faute de mémoire.
#   ./scripts/ship-image.sh utilisateur@serveur [/chemin/du/dépôt/sur/le/serveur]
# Le serveur doit déjà contenir le dépôt (git clone) et deploy/.env. Les architectures doivent correspondre
# (poste x86_64 → serveur x86_64) ; sinon : docker buildx build --platform linux/arm64 …
set -euo pipefail
cd "$(dirname "$0")/.."
TARGET="${1:?Usage : $0 utilisateur@serveur [chemin-du-dépôt]}"
REMOTE_DIR="${2:-TTScreen}"
echo "==> Construction locale"
docker build -f deploy/Dockerfile -t ttscreen-api:latest .
echo "==> Envoi de l'image à $TARGET"
docker save ttscreen-api:latest | gzip | ssh "$TARGET" "gunzip | docker load"
echo "==> Déploiement sur le serveur"
ssh "$TARGET" "cd '$REMOTE_DIR' && git pull --ff-only && ./scripts/deploy.sh --no-build"
