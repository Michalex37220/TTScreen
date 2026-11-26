# Port Rust : notes et liste de vérification

## État de vérification (à lire en premier)

| Partie | État |
|---|---|
| Port Rust initial (auth, médias, playlists, écrans, audit, limitation de débit…) | **Compilé et testé par vous** : tests unitaires, `security` et `features` passés sur SQLite (les échecs rencontrés étaient des erreurs de tests, corrigées). |
| **Ajouts postérieurs** : groupes d'écrans, planning horaire, gestion des écrans (renommer/supprimer/changer de groupe), changement de mot de passe, duplication de playlist, tableau de bord, migration `group_id` | **Jamais compilés ni exécutés** (pas de toolchain Rust là où ils ont été écrits). Seules des vérifications statiques ont été faites : accolades, imports, helpers de tests, routes ↔ handlers. Prévoyez de petites corrections au premier `cargo build`. |
| Interface web (`frontend/`) | **Testée dans un vrai navigateur** (Chromium via Playwright) **contre un faux backend** : `./scripts/test-ui.sh`, 20 vérifications, aucune erreur JavaScript. Elle n'a pas été testée contre le vrai backend Rust. |
| PostgreSQL, build Docker, scripts de déploiement | Non exercés (voir `docs/DEPLOIEMENT.md`). |

Pour valider les ajouts : `cargo test --lib`, puis `cargo test --test groups --test planning --test account_stats`, puis l'ensemble (`cargo test`).

## Première mise en route

```bash
cd backend
cargo build            # corrige d'abord les erreurs de compilation éventuelles
cargo test             # unitaires (src/*) + intégration (tests/*)
cargo clippy --all-targets
git add Cargo.lock     # à committer (le Dockerfile l'utilise s'il existe)
```

Ordre de dépannage conseillé si les tests échouent :
1. `cargo test --lib` (unitaires, sans réseau ni HTTP) ;
2. `cargo test --test api` puis `--test security` puis `--test features`.

## Points les plus susceptibles de demander un ajustement

| Zone | Pourquoi |
|---|---|
| `models.rs` (SeaORM) | attributs `#[sea_orm(...)]` (`nullable`, `column_type`, `column_name`) et clés étrangères `belongs_to` |
| `db.rs` | création du schéma (`Schema::create_table_from_entity`, `Index::create()`), URL SQLite (`sqlite:<chemin>?mode=rwc`) |
| `api/media.rs` | API multipart d'axum 0.8 (`Field`, `MultipartError::status()`), `into_tuple()` sur une colonne JSON |
| `api/public.rs` | `ServeFile::oneshot` pour les fichiers/pages statiques ; routes jokers `/{*rel}` d'axum 0.8 |
| `deps.rs`, `extract.rs` | extracteurs `FromRequestParts`/`FromRequest` avec `async fn` dans l'impl (axum 0.8) |
| `scheduling.rs`, `schemas.rs` | `chrono_tz::Tz` (parse, `from_utc_datetime`), types entiers des masques de jours |
| `db.rs` (`ensure_column`) | migration par `SELECT … LIMIT 0` puis `ALTER TABLE` : validée seulement par un test SQLite |
| `models.rs` | trois relations `belongs_to` dans `schedule_rule`, relation `ScreenGroup` dans `screen` |
| `api/schedules.rs` | `.filter(match …)` renvoyant deux expressions de colonnes différentes |
| PostgreSQL | le chemin PostgreSQL n'a été exercé par aucun test (les tests utilisent SQLite) |

## Écarts volontaires avec la version Python

- Pas de `/docs`, `/redoc`, `/openapi.json`.
- `TTSCREEN_PBKDF2_ITERATIONS` n'existe pas : seule la structure `Settings` (tests) peut baisser le coût.
- `Cache-Control: public, max-age=31536000, immutable` ajouté sur `/files/…` (décrit dans le README d'origine, assuré à l'origine par Caddy seulement).
- Deux envois identiques simultanés renvoient le même média (la version Python pouvait répondre 500).
- Corps JSON invalide : 422 avec `detail` sous forme de liste (`loc`, `msg`, `type`), comme FastAPI ; les textes de `msg` ne sont pas identiques mot pour mot à ceux de pydantic.

## Évolutions fonctionnelles après le port

- Groupes d'écrans (`api/groups.rs`, table `screen_groups`, colonne `screens.group_id` ajoutée automatiquement aux bases existantes).
- Planning horaire (`scheduling.rs`, `api/schedules.rs`, table `schedule_rules`, variable `TTSCREEN_TIMEZONE`). Priorité : planning de l'écran › contenu de l'écran › planning du groupe › contenu du groupe.
- Écrans : renommer, supprimer, changer de groupe ; `effective_version_id` / `effective_source` dans leur représentation.
- `POST /api/auth/password`, `POST /api/playlists/{id}/duplicate`, `GET /api/stats`.
- Le lecteur (`player/`) n'a pas changé : il reçoit déjà la version à afficher à chaque interrogation.
- Non inclus volontairement : message d'urgence global, statistiques de diffusion, notifications d'écran hors ligne, planning au-delà de minuit en une seule règle.
