#!/usr/bin/env bash
# Déploie (ou redéploie) TTScreen en production avec Docker Compose.
#   ./scripts/deploy.sh              construit l'image sur ce serveur, puis démarre
#   ./scripts/deploy.sh --no-build   utilise l'image déjà présente (voir scripts/ship-image.sh)
# Vérifie d'abord deploy/.env : rien n'est démarré si la configuration est dangereuse ou incomplète.
set -euo pipefail
cd "$(dirname "$0")/.."

BUILD=1
[ "${1:-}" = "--no-build" ] && BUILD=0
ENV_FILE=deploy/.env
COMPOSE=(docker compose -f deploy/docker-compose.prod.yml --env-file "$ENV_FILE")

err()  { echo "ERREUR : $*" >&2; exit 1; }
warn() { echo "ATTENTION : $*" >&2; }
get()  { # lit une variable de deploy/.env (dernière occurrence, guillemets retirés)
  { grep -E "^$1=" "$ENV_FILE" || true; } | tail -n1 | cut -d= -f2- | sed -e 's/^"//' -e 's/"$//' -e "s/^'//" -e "s/'\$//"
}

command -v docker >/dev/null 2>&1 || err "Docker n'est pas installé (voir docs/DEPLOIEMENT.md, étape 2)."
docker compose version >/dev/null 2>&1 || err "Le plugin « docker compose » (v2) est requis."
[ -f "$ENV_FILE" ] || err "$ENV_FILE introuvable : copiez deploy/.env.example vers deploy/.env et remplissez-le."

# Le fichier contient des secrets : lisible par son propriétaire uniquement
perms=$(stat -c '%a' "$ENV_FILE" 2>/dev/null || echo "")
if [ -n "$perms" ] && [ "${perms: -1}" != "0" ]; then
  warn "$ENV_FILE est lisible par d'autres utilisateurs ($perms) : correction (chmod 600)."
  chmod 600 "$ENV_FILE"
fi

# ---- contrôles de configuration ----
DOMAIN=$(get DOMAIN)
[ -n "$DOMAIN" ] || err "DOMAIN n'est pas défini dans $ENV_FILE."
case "$DOMAIN" in *exemple*|*example*|localhost|*.local) err "DOMAIN=$DOMAIN est une valeur d'exemple : mettez votre vrai nom de domaine." ;; esac
case "$DOMAIN" in *://*|*/*|*" "*) err "DOMAIN doit être un nom d'hôte seul (sans http:// ni /)." ;; esac

PG=$(get POSTGRES_PASSWORD)
[ "${#PG}" -ge 20 ] || err "POSTGRES_PASSWORD : 20 caractères minimum (openssl rand -base64 32 | tr '+/' '-_' | tr -d '=')."
[[ "$PG" =~ ^[A-Za-z0-9_.~-]+$ ]] || err "POSTGRES_PASSWORD ne doit contenir que lettres, chiffres, _ . - ~ (il est inséré dans une URL)."

TOKEN=$(get TTSCREEN_ADMIN_TOKEN)
BUSER=$(get TTSCREEN_BOOTSTRAP_ADMIN_USER)
BPASS=$(get TTSCREEN_BOOTSTRAP_ADMIN_PASSWORD)
if [ -n "$TOKEN" ]; then
  distinct=$(printf %s "$TOKEN" | fold -w1 | sort -u | wc -l)
  { [ "${#TOKEN}" -ge 24 ] && [ "$distinct" -ge 10 ] && [ "$TOKEN" != "dev-admin-token" ]; } \
    || err "TTSCREEN_ADMIN_TOKEN trop faible (24 caractères minimum, au moins 10 différents)."
fi
if [ -n "$BUSER" ] || [ -n "$BPASS" ]; then
  [ -n "$BUSER" ] && [ -n "$BPASS" ] || err "TTSCREEN_BOOTSTRAP_ADMIN_USER et _PASSWORD vont ensemble."
  [[ "$BUSER" =~ ^[A-Za-z0-9_.-]{3,50}$ ]] || err "TTSCREEN_BOOTSTRAP_ADMIN_USER : 3 à 50 caractères parmi A-Z a-z 0-9 _ . -"
  [ "${#BPASS}" -ge 8 ] || err "TTSCREEN_BOOTSTRAP_ADMIN_PASSWORD : 8 caractères minimum."
fi
if [ -z "$TOKEN" ] && [ -z "$BUSER" ]; then
  # Normal après le premier démarrage (mot de passe d'amorçage retiré du fichier) : les comptes sont en base.
  # Sur une première installation, l'API refusera de démarrer et ses journaux seront affichés ci-dessous.
  warn "Ni jeton ni compte d'amorçage dans $ENV_FILE : l'API ne démarre que si un compte admin existe déjà en base."
fi
[ -n "$BPASS" ] && warn "TTSCREEN_BOOTSTRAP_ADMIN_PASSWORD est encore dans $ENV_FILE : retirez-le une fois le compte créé."
[ "$(get TTSCREEN_BOOTSTRAP_ADMIN_RESET)" = "true" ] && warn "TTSCREEN_BOOTSTRAP_ADMIN_RESET=true : à retirer après récupération d'accès."
[ "$(get TTSCREEN_LEGACY_TOKEN_ALWAYS)" = "true" ] && warn "TTSCREEN_LEGACY_TOKEN_ALWAYS=true : le jeton historique reste valide malgré les comptes nommés."

# ---- construction et démarrage ----
if [ "$BUILD" = "1" ]; then
  docker tag ttscreen-api:latest ttscreen-api:previous 2>/dev/null || true   # pour ./scripts/rollback.sh
  echo "==> Construction de l'image (la première compilation Rust prend plusieurs minutes)…"
  "${COMPOSE[@]}" build api
else
  docker image inspect ttscreen-api:latest >/dev/null 2>&1 \
    || err "Image ttscreen-api:latest absente : lancez d'abord ./scripts/ship-image.sh depuis votre poste."
fi
echo "==> Démarrage…"
"${COMPOSE[@]}" up -d --no-build --remove-orphans

echo "==> Attente de l'API (jusqu'à 3 min)…"
cid=$("${COMPOSE[@]}" ps -q api)
status=starting
for _ in $(seq 1 90); do
  status=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "$cid" 2>/dev/null || echo gone)
  [ "$status" = "healthy" ] && break
  [ "$status" = "gone" ] && break
  sleep 2
done
if [ "$status" != "healthy" ]; then
  echo "--- dernières lignes du journal de l'API ---" >&2
  "${COMPOSE[@]}" logs --tail 40 api >&2 || true
  err "L'API n'est pas devenue saine (état : $status)."
fi

echo "==> L'API répond. Test HTTPS public (le certificat peut mettre une minute à être émis)…"
if command -v curl >/dev/null 2>&1; then
  ok=0
  for _ in 1 2 3 4 5 6; do
    if curl -fsS --max-time 10 "https://$DOMAIN/api/health" >/dev/null 2>&1; then ok=1; break; fi
    sleep 10
  done
  if [ "$ok" = "1" ]; then echo "    https://$DOMAIN/api/health : OK"
  else warn "https://$DOMAIN/api/health ne répond pas encore : vérifiez le DNS (A/AAAA), les ports 80/443 ouverts, puis : ${COMPOSE[*]} logs caddy"; fi
fi
cat <<MSG

TTScreen est démarré.
  Administration : https://$DOMAIN/admin/
  Lecteur (écrans) : https://$DOMAIN/player/
  Journaux         : ${COMPOSE[*]} logs -f api
  Sauvegarde       : ./scripts/backup.sh   (à planifier : voir docs/DEPLOIEMENT.md)
MSG
