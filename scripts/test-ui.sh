#!/usr/bin/env bash
# Teste l'interface web (frontend/) dans un vrai navigateur, contre un FAUX backend en mémoire (tests/ui/mock_server.py).
# Il vérifie l'interface, pas l'API : celle-ci est testée par `cargo test`.
# Prérequis : pip install playwright && playwright install chromium
set -euo pipefail
cd "$(dirname "$0")/.."
PORT="${TTSCREEN_UI_PORT:-8765}"
python3 tests/ui/mock_server.py "$PWD" "$PORT" >/dev/null 2>&1 &
MOCK=$!
trap 'kill $MOCK 2>/dev/null || true' EXIT
sleep 1
TTSCREEN_UI_BASE="http://127.0.0.1:$PORT" python3 tests/ui/run.py
