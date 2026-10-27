# Digital Signage — MVP local (étapes 1-2 : dépôt + API)

## Démarrage sur NixOS
```bash
nix develop                # shell avec Python, Node, Docker Compose, Caddy, ffmpeg
cp .env.example .env
./scripts/test.sh          # tests automatisés du backend
./scripts/dev.sh           # API sur http://127.0.0.1:8000  (docs : /docs)
```
Sans Nix : `cd backend && python -m venv .venv && . .venv/bin/activate && pip install -e '.[dev]'`.

## Essai manuel (jeton admin = SIGNAGE_ADMIN_TOKEN, défaut `dev-admin-token`)
```bash
A="Authorization: Bearer dev-admin-token"; U=http://127.0.0.1:8000
curl -sH "$A" -F file=@photo.jpg $U/api/media
curl -sH "$A" -H 'Content-Type: application/json' -d '{"name":"Hall"}' $U/api/playlists
curl -sH "$A" -X PUT -H 'Content-Type: application/json' \
  -d '[{"type":"text","text":"Bienvenue","duration_s":8},{"type":"image","media_id":1}]' $U/api/playlists/1/items
curl -sH "$A" -X POST $U/api/playlists/1/publish
curl -sH "$A" -H 'Content-Type: application/json' -d '{"name":"Entrée"}' $U/api/screens   # affiche le jeton UNE fois
curl -sH "$A" -H 'Content-Type: application/json' -d '{"version_id":1}' $U/api/screens/1/assign
curl -sH "Authorization: Bearer <jeton-écran>" $U/api/player/manifest
```

## Décisions
- Version immuable = instantané JSON du manifeste (URL, SHA-256, taille de chaque média).
- Retour arrière = réaffecter une version antérieure à l'écran.
- Jetons d'écran stockés hachés (SHA-256), rotation et révocation individuelles.
- `/files/<sha256>.<ext>` non authentifié (noms non devinables) : à remplacer par URL signées (étape 7).
- Jeton admin unique provisoire : comptes et rôles à l'étape 7.
- Interface web : http://127.0.0.1:8000/admin/ (HTML/JS sans build, servie par l'API).
