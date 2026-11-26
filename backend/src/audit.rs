//! Journal des opérations sensibles (équivalent de `audit.py`).
//!
//! Chaque opération importante (publication, affectation à un écran, gestion des comptes,
//! tentative de connexion…) ajoute une ligne dans `audit_log` via [`log_action`]. Appelée avec
//! une transaction, la ligne est validée avec l'opération auditée : elle ne peut pas être écrite
//! sans que l'opération réussisse, et l'opération ne réussit pas sans sa ligne d'audit.

use sea_orm::{ActiveModelTrait, ConnectionTrait, DbErr, Set};

use crate::auth::Principal;
use crate::models::{audit_log, utcnow};

/// Tronque à `max` caractères (et non octets), comme les colonnes `VARCHAR(n)`.
pub fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Ajoute une ligne au journal.
///
/// - `principal` : auteur (`None` pour une tentative de connexion échouée → « anonyme ») ;
/// - `action` : code court, ex. `playlist.publish`, `user.create`, `connexion_echouee` ;
/// - `target` : ressource visée, ex. `playlist:3` ;
/// - `ip` : adresse du client, pour situer l'action.
///
/// Les champs sont tronqués à la longueur des colonnes : sous PostgreSQL un VARCHAR dépassé lève une
/// erreur (SQLite l'ignore), ce qui ferait échouer l'opération auditée pour un nom de fichier trop long.
pub async fn log_action<C: ConnectionTrait>(
    db: &C,
    ip: &str,
    principal: Option<&Principal>,
    action: &str,
    target: Option<String>,
    detail: Option<String>,
) -> Result<(), DbErr> {
    audit_log::ActiveModel {
        created_at: Set(utcnow()),
        username: Set(truncate(principal.map_or("anonyme", |p| p.username.as_str()), 50)),
        role: Set(principal.map(|p| p.role.clone())),
        action: Set(truncate(action, 40)),
        target: Set(target.map(|t| truncate(&t, 120))),
        detail: Set(detail),
        ip: Set(if ip.is_empty() { None } else { Some(truncate(ip, 45)) }),
        ..Default::default()
    }
    .insert(db)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use sea_orm::{EntityTrait, QueryOrder};

    #[test]
    fn truncate_counts_characters_not_bytes() {
        assert_eq!(truncate("abcdef", 3), "abc");
        assert_eq!(truncate("é☃é☃", 3), "é☃é");
        assert_eq!(truncate("court", 50), "court");
        assert_eq!(truncate("", 5), "");
    }

    async fn fresh() -> (tempfile::TempDir, sea_orm::DatabaseConnection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::connect(&format!("sqlite:///{}", dir.path().join("a.db").display())).await.unwrap();
        db::init_schema(&conn).await.unwrap();
        (dir, conn)
    }

    #[tokio::test]
    async fn rows_are_written_with_author_snapshot() {
        let (_d, conn) = fresh().await;
        let p = Principal { username: "emma".into(), role: "editor".into(), legacy: false };
        log_action(&conn, "10.0.0.7", Some(&p), "playlist.publish", Some("playlist:3".into()), Some("v2".into()))
            .await
            .unwrap();
        let rows = audit_log::Entity::find().all(&conn).await.unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!((r.username.as_str(), r.role.as_deref(), r.action.as_str()), ("emma", Some("editor"), "playlist.publish"));
        assert_eq!((r.target.as_deref(), r.detail.as_deref(), r.ip.as_deref()), (Some("playlist:3"), Some("v2"), Some("10.0.0.7")));
    }

    #[tokio::test]
    async fn anonymous_attempts_are_logged_as_anonyme_without_role() {
        let (_d, conn) = fresh().await;
        log_action(&conn, "1.2.3.4", None, "connexion_echouee", None, None).await.unwrap();
        let r = audit_log::Entity::find().one(&conn).await.unwrap().unwrap();
        assert_eq!(r.username, "anonyme");
        assert_eq!(r.role, None);
    }

    #[tokio::test]
    async fn oversized_fields_are_truncated_to_column_sizes() {
        let (_d, conn) = fresh().await;
        let p = Principal { username: "u".repeat(80), role: "admin".into(), legacy: false };
        log_action(&conn, &"9".repeat(80), Some(&p), &"a".repeat(80), Some("t".repeat(300)), None).await.unwrap();
        let r = audit_log::Entity::find().order_by_asc(audit_log::Column::Id).one(&conn).await.unwrap().unwrap();
        assert_eq!(r.username.chars().count(), 50);
        assert_eq!(r.action.chars().count(), 40);
        assert_eq!(r.target.unwrap().chars().count(), 120);
        assert_eq!(r.ip.unwrap().chars().count(), 45);
    }

    #[tokio::test]
    async fn unknown_ip_is_stored_as_null() {
        let (_d, conn) = fresh().await;
        log_action(&conn, "", None, "x", None, None).await.unwrap();
        assert_eq!(audit_log::Entity::find().one(&conn).await.unwrap().unwrap().ip, None);
    }
}
