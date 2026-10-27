# Contexte de build : racine du dépôt  (docker compose -f deploy/docker-compose*.yml ...)
FROM python:3.12-alpine AS build
WORKDIR /build
COPY backend/requirements.txt .
RUN pip install --no-cache-dir --prefix=/install -r requirements.txt

FROM python:3.12-alpine
RUN addgroup -S app && adduser -S -G app -u 10001 app && mkdir -p /data/media && chown -R app:app /data
COPY --from=build /install /usr/local
WORKDIR /app
COPY backend/app ./app
COPY frontend ./frontend
COPY player ./player
ENV PYTHONUNBUFFERED=1 PYTHONDONTWRITEBYTECODE=1 \
    SIGNAGE_FRONTEND_DIR=/app/frontend SIGNAGE_PLAYER_DIR=/app/player \
    SIGNAGE_MEDIA_DIR=/data/media SIGNAGE_DATABASE_URL=sqlite:////data/signage.db
USER app
EXPOSE 8000
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s \
  CMD python -c "import urllib.request,sys; urllib.request.urlopen('http://127.0.0.1:8000/api/health', timeout=3)" || exit 1
# 1 worker suffit largement (20 écrans ≈ 2 requêtes/s). Plusieurs workers : voir README (création des tables).
CMD ["sh", "-c", "exec uvicorn app.main:app --host 0.0.0.0 --port 8000 --proxy-headers --workers ${UVICORN_WORKERS:-1}"]
