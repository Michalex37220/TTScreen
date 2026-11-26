# Contexte de build : racine du dépôt  (docker compose -f deploy/docker-compose*.yml ...)
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev build-base
WORKDIR /build
# 1) Compilation des dépendances seules (couche mise en cache tant que Cargo.toml/Cargo.lock ne changent pas)
COPY backend/Cargo.toml backend/Cargo.loc[k] ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && : > src/lib.rs \
    && if [ -f Cargo.lock ]; then cargo build --release --locked; else cargo build --release; fi \
    && rm -rf src
# 2) Compilation de l'application
COPY backend/src ./src
# --locked : versions exactes de Cargo.lock (build reproductible) dès que le fichier est commité
RUN touch src/main.rs src/lib.rs \
    && if [ -f Cargo.lock ]; then cargo build --release --locked; else cargo build --release; fi

FROM alpine:3.20
RUN addgroup -S app && adduser -S -G app -u 10001 app && mkdir -p /data/media && chown -R app:app /data
COPY --from=build /build/target/release/ttscreen /usr/local/bin/ttscreen
WORKDIR /app
COPY frontend ./frontend
COPY player ./player
# FORWARDED_ALLOW_IPS : adresses autorisées à fournir X-Forwarded-For (défaut : loopback uniquement).
# docker-compose.prod.yml le surcharge car l'API n'y est joignable que par Caddy.
ENV TTSCREEN_FRONTEND_DIR=/app/frontend TTSCREEN_PLAYER_DIR=/app/player \
    TTSCREEN_MEDIA_DIR=/data/media TTSCREEN_DATABASE_URL=sqlite:////data/ttscreen.db \
    TTSCREEN_HOST=0.0.0.0 TTSCREEN_PORT=8000 FORWARDED_ALLOW_IPS=127.0.0.1 RUST_LOG=info
USER app
EXPOSE 8000
# La sonde est le binaire lui-même (aucun curl/python dans l'image)
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s \
  CMD ["/usr/local/bin/ttscreen", "healthcheck"]
# Un seul processus suffit : le runtime asynchrone utilise tous les cœurs (plus de UVICORN_WORKERS)
CMD ["/usr/local/bin/ttscreen"]
