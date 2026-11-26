//! Connexion et création du schéma (équivalent de `db.py` + `Base.metadata.create_all`).

use sea_orm::sea_query::Index;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbErr, EntityTrait, Schema};
use std::path::PathBuf;
use std::time::Duration;

use crate::models::{
    audit_log, media, playlist, playlist_item, playlist_version, schedule_rule, screen, screen_group, user, user_session,
};

/// Traduit une URL « à la SQLAlchemy » (celle du README) en URL comprise par sqlx.
///
/// - `sqlite:///./data/x.db`  (relatif) et `sqlite:////abs/x.db` (absolu) → `sqlite:<chemin>?mode=rwc`
///   (`mode=rwc` : le fichier est créé s'il n'existe pas) ;
/// - `postgresql+psycopg://…`, `postgresql://…`, `postgres://…` → `postgres://…`.
pub fn to_connect_url(database_url: &str) -> Result<String, String> {
    let url = database_url.trim();
    if let Some(path) = url.strip_prefix("sqlite:///") {
        if path.is_empty() {
            return Err("URL SQLite sans chemin de fichier".to_string());
        }
        let encoded = path.replace('%', "%25").replace('?', "%3F").replace('#', "%23");
        return Ok(format!("sqlite:{encoded}?mode=rwc"));
    }
    if url.starts_with("sqlite:") {
        return Ok(url.to_string());
    }
    if let Some((scheme, rest)) = url.split_once("://") {
        let base = scheme.split('+').next().unwrap_or(scheme);
        if base == "postgresql" || base == "postgres" {
            return Ok(format!("postgres://{rest}"));
        }
    }
    Err(format!("Base de données non prise en charge : {database_url:?} (SQLite ou PostgreSQL)"))
}

/// Fichier SQLite visé par l'URL (pour créer son dossier parent), s'il s'agit de SQLite.
pub fn sqlite_file(database_url: &str) -> Option<PathBuf> {
    database_url.trim().strip_prefix("sqlite:///").map(PathBuf::from)
}

pub async fn connect(database_url: &str) -> Result<DatabaseConnection, DbErr> {
    let url = to_connect_url(database_url).map_err(DbErr::Custom)?;
    let sqlite = url.starts_with("sqlite:");
    let mut opts = ConnectOptions::new(url);
    opts.max_connections(if sqlite { 8 } else { 10 })
        .min_connections(1)
        .connect_timeout(Duration::from_secs(10))
        .sqlx_logging(false);
    Database::connect(opts).await
}

async fn create_table<E: EntityTrait>(db: &DatabaseConnection, entity: E) -> Result<(), DbErr> {
    let backend = db.get_database_backend();
    let mut stmt = Schema::new(backend).create_table_from_entity(entity);
    stmt.if_not_exists();
    db.execute(backend.build(&stmt)).await?;
    Ok(())
}

/// Ajoute `column` à `table` si elle n'existe pas encore (SQLite et PostgreSQL).
/// `table`, `column` et `ddl` sont des constantes du code, jamais des données utilisateur.
pub async fn ensure_column(db: &DatabaseConnection, table: &str, column: &str, ddl: &str) -> Result<(), DbErr> {
    if db.execute_unprepared(&format!("SELECT {column} FROM {table} LIMIT 0")).await.is_ok() {
        return Ok(());
    }
    db.execute_unprepared(&format!("ALTER TABLE {table} ADD COLUMN {column} {ddl}")).await?;
    Ok(())
}

/// Crée les tables et index manquants (idempotent, sans migration : comme `create_all`).
pub async fn init_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    // Ordre : les tables référencées avant celles qui les référencent (clés étrangères)
    create_table(db, user::Entity).await?;
    create_table(db, user_session::Entity).await?;
    create_table(db, media::Entity).await?;
    create_table(db, playlist::Entity).await?;
    create_table(db, playlist_item::Entity).await?;
    create_table(db, playlist_version::Entity).await?;
    create_table(db, screen_group::Entity).await?;
    create_table(db, screen::Entity).await?;
    create_table(db, schedule_rule::Entity).await?;
    create_table(db, audit_log::Entity).await?;

    // Migrations légères pour les bases créées par une version antérieure (`CREATE TABLE IF NOT EXISTS`
    // n'ajoute pas de colonne à une table existante). Chaque étape est idempotente.
    ensure_column(db, "screens", "group_id", "INTEGER REFERENCES screen_groups(id)").await?;

    let backend = db.get_database_backend();
    let indexes = [
        // UniqueConstraint("playlist_id", "number") du modèle Python
        Index::create()
            .if_not_exists()
            .name("uq_playlist_versions_playlist_number")
            .table(playlist_version::Entity)
            .col(playlist_version::Column::PlaylistId)
            .col(playlist_version::Column::Number)
            .unique()
            .to_owned(),
        Index::create()
            .if_not_exists()
            .name("ix_audit_log_created_at")
            .table(audit_log::Entity)
            .col(audit_log::Column::CreatedAt)
            .to_owned(),
        Index::create()
            .if_not_exists()
            .name("ix_audit_log_action")
            .table(audit_log::Entity)
            .col(audit_log::Column::Action)
            .to_owned(),
    ];
    for idx in &indexes {
        db.execute(backend.build(idx)).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_relative_and_absolute_urls() {
        assert_eq!(to_connect_url("sqlite:///./data/ttscreen.db").unwrap(), "sqlite:./data/ttscreen.db?mode=rwc");
        assert_eq!(to_connect_url("sqlite:////data/ttscreen.db").unwrap(), "sqlite:/data/ttscreen.db?mode=rwc");
        assert_eq!(to_connect_url("sqlite:////tmp/a b/t.db").unwrap(), "sqlite:/tmp/a b/t.db?mode=rwc");
    }

    #[test]
    fn sqlite_special_characters_are_escaped() {
        assert_eq!(to_connect_url("sqlite:///a%b?c#d.db").unwrap(), "sqlite:a%25b%3Fc%23d.db?mode=rwc");
    }

    #[test]
    fn postgres_driver_suffix_is_stripped() {
        assert_eq!(
            to_connect_url("postgresql+psycopg://ttscreen:pw@db:5432/ttscreen").unwrap(),
            "postgres://ttscreen:pw@db:5432/ttscreen"
        );
        assert_eq!(to_connect_url("postgresql://u:p@h/d").unwrap(), "postgres://u:p@h/d");
        assert_eq!(to_connect_url("postgres://u:p@h/d").unwrap(), "postgres://u:p@h/d");
    }

    #[test]
    fn unsupported_urls_are_refused() {
        assert!(to_connect_url("mysql://u:p@h/d").is_err());
        assert!(to_connect_url("sqlite:///").is_err());
        assert!(to_connect_url("").is_err());
    }

    #[test]
    fn sqlite_file_is_extracted_only_for_sqlite() {
        assert_eq!(sqlite_file("sqlite:///./data/x.db"), Some(PathBuf::from("./data/x.db")));
        assert_eq!(sqlite_file("postgresql://u:p@h/d"), None);
    }

    #[tokio::test]
    async fn init_schema_is_idempotent_and_creates_every_table() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite:///{}", dir.path().join("t.db").display());
        let db = connect(&url).await.unwrap();
        init_schema(&db).await.unwrap();
        init_schema(&db).await.unwrap(); // deuxième appel : rien ne casse
        assert_eq!(user::Entity::find().all(&db).await.unwrap().len(), 0);
        assert_eq!(audit_log::Entity::find().all(&db).await.unwrap().len(), 0);
        assert_eq!(playlist_version::Entity::find().all(&db).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn old_databases_gain_the_group_column_without_losing_data() {
        use sea_orm::EntityTrait;
        let dir = tempfile::tempdir().unwrap();
        let db = connect(&format!("sqlite:///{}", dir.path().join("old.db").display())).await.unwrap();
        // schéma « d'avant les groupes » : table screens sans group_id, avec une ligne existante
        db.execute_unprepared(
            "CREATE TABLE screens (id INTEGER PRIMARY KEY AUTOINCREMENT, name VARCHAR NOT NULL, \
             token_hash VARCHAR UNIQUE, version_id INTEGER, last_seen DATETIME, created_at DATETIME NOT NULL)",
        )
        .await
        .unwrap();
        db.execute_unprepared("INSERT INTO screens (name, token_hash, created_at) VALUES ('Entrée', 'h', '2026-01-02 10:00:00')")
            .await
            .unwrap();
        init_schema(&db).await.unwrap();
        init_schema(&db).await.unwrap(); // idempotent
        let rows = screen::Entity::find().all(&db).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].name.as_str(), rows[0].group_id), ("Entrée", None));
        // la nouvelle colonne est utilisable
        db.execute_unprepared("INSERT INTO screen_groups (name, created_at) VALUES ('G', '2026-01-02 10:00:00')").await.unwrap();
        db.execute_unprepared("UPDATE screens SET group_id = 1").await.unwrap();
        assert_eq!(screen::Entity::find().one(&db).await.unwrap().unwrap().group_id, Some(1));
    }

    #[tokio::test]
    async fn ensure_column_is_a_noop_when_the_column_exists() {
        let dir = tempfile::tempdir().unwrap();
        let db = connect(&format!("sqlite:///{}", dir.path().join("t.db").display())).await.unwrap();
        init_schema(&db).await.unwrap();
        ensure_column(&db, "screens", "group_id", "INTEGER").await.unwrap();
        ensure_column(&db, "screens", "group_id", "INTEGER").await.unwrap();
    }

    #[tokio::test]
    async fn unique_constraints_are_enforced_by_the_database() {
        use crate::models::utcnow;
        use sea_orm::{ActiveModelTrait, Set};
        let dir = tempfile::tempdir().unwrap();
        let db = connect(&format!("sqlite:///{}", dir.path().join("t.db").display())).await.unwrap();
        init_schema(&db).await.unwrap();
        let mk = |name: &str| user::ActiveModel {
            username: Set(name.to_string()),
            password_hash: Set("x".into()),
            role: Set("viewer".into()),
            active: Set(true),
            created_at: Set(utcnow()),
            ..Default::default()
        };
        mk("emma").insert(&db).await.unwrap();
        assert!(mk("emma").insert(&db).await.is_err(), "username unique");
        mk("lea").insert(&db).await.unwrap();
    }
}
