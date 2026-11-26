//! Tables de la base (équivalent de `models.py`). Mêmes noms de tables et de colonnes qu'à l'origine.
//!
//! Les dates sont des « UTC naïfs » (`NaiveDateTime`), cohérents entre SQLite et PostgreSQL.

use chrono::{NaiveDateTime, Timelike};

/// UTC naïf, à la microseconde (comme `datetime.utcnow()` côté Python et la précision de PostgreSQL).
pub fn utcnow() -> NaiveDateTime {
    let now = chrono::Utc::now().naive_utc();
    now.with_nanosecond(now.nanosecond() / 1000 * 1000).unwrap_or(now)
}

pub mod media {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "media")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub original_name: String,
        #[sea_orm(unique)]
        pub stored_name: String,
        #[sea_orm(unique)]
        pub sha256: String,
        /// image | video
        pub kind: String,
        pub mime: String,
        /// Octets (INTEGER 32 bits, comme à l'origine : un fichier ne peut pas dépasser 2 Gio).
        pub size: i32,
        pub created_at: DateTime,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod playlist {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "playlists")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub name: String,
        pub created_at: DateTime,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod playlist_item {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "playlist_items")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub playlist_id: i32,
        pub position: i32,
        /// text | image | video
        #[sea_orm(column_name = "type")]
        pub item_type: String,
        #[sea_orm(column_type = "Text", nullable)]
        pub text: Option<String>,
        pub media_id: Option<i32>,
        pub duration_s: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::playlist::Entity",
            from = "Column::PlaylistId",
            to = "super::playlist::Column::Id"
        )]
        Playlist,
        #[sea_orm(
            belongs_to = "super::media::Entity",
            from = "Column::MediaId",
            to = "super::media::Column::Id"
        )]
        Media,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// Version immuable : le manifeste est un instantané, jamais modifié.
pub mod playlist_version {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "playlist_versions")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub playlist_id: i32,
        pub number: i32,
        pub manifest: Json,
        pub media_ids: Json,
        pub created_at: DateTime,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::playlist::Entity",
            from = "Column::PlaylistId",
            to = "super::playlist::Column::Id"
        )]
        Playlist,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod screen {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "screens")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub name: String,
        #[sea_orm(unique, nullable)]
        pub token_hash: Option<String>,
        /// Contenu fixé sur cet écran (prioritaire sur celui de son groupe).
        pub version_id: Option<i32>,
        pub last_seen: Option<DateTime>,
        pub created_at: DateTime,
        /// Groupe d'écrans (contenu partagé) ; `None` = écran isolé.
        pub group_id: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::playlist_version::Entity",
            from = "Column::VersionId",
            to = "super::playlist_version::Column::Id"
        )]
        PlaylistVersion,
        #[sea_orm(
            belongs_to = "super::screen_group::Entity",
            from = "Column::GroupId",
            to = "super::screen_group::Column::Id"
        )]
        ScreenGroup,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// Groupe d'écrans : tous ses écrans affichent le même contenu (sauf écran ayant son propre contenu fixé).
pub mod screen_group {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "screen_groups")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        #[sea_orm(unique)]
        pub name: String,
        /// Version diffusée sur tout le groupe.
        pub version_id: Option<i32>,
        pub created_at: DateTime,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::playlist_version::Entity",
            from = "Column::VersionId",
            to = "super::playlist_version::Column::Id"
        )]
        PlaylistVersion,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// Règle de planning : « cette version, ces jours, de HH:MM à HH:MM » pour un écran OU un groupe.
/// `days` est un masque de bits (bit 0 = lundi … bit 6 = dimanche) ; les minutes sont comptées depuis minuit,
/// heure locale (fuseau `TTSCREEN_TIMEZONE`). `end_min` va jusqu'à 1440 (= 24:00).
pub mod schedule_rule {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "schedule_rules")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub screen_id: Option<i32>,
        pub group_id: Option<i32>,
        pub version_id: i32,
        pub days: i32,
        pub start_min: i32,
        pub end_min: i32,
        pub created_at: DateTime,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::screen::Entity",
            from = "Column::ScreenId",
            to = "super::screen::Column::Id"
        )]
        Screen,
        #[sea_orm(
            belongs_to = "super::screen_group::Entity",
            from = "Column::GroupId",
            to = "super::screen_group::Column::Id"
        )]
        ScreenGroup,
        #[sea_orm(
            belongs_to = "super::playlist_version::Entity",
            from = "Column::VersionId",
            to = "super::playlist_version::Column::Id"
        )]
        PlaylistVersion,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// Compte utilisateur nommé. Le mot de passe n'est jamais stocké en clair : uniquement un haché PBKDF2.
/// role : viewer (lecture seule) < editor (contenu) < admin (comptes + journal).
pub mod user {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "users")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        #[sea_orm(unique)]
        pub username: String,
        pub password_hash: String,
        pub role: String,
        pub active: bool,
        pub created_at: DateTime,
        pub last_login: Option<DateTime>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Session de connexion : seul le haché du jeton est stocké (comme les jetons d'écran).
/// La suppression de la ligne révoque immédiatement le jeton.
pub mod user_session {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "user_sessions")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub user_id: i32,
        #[sea_orm(unique)]
        pub token_hash: String,
        pub created_at: DateTime,
        pub expires_at: DateTime,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::user::Entity",
            from = "Column::UserId",
            to = "super::user::Column::Id"
        )]
        User,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// Journal des opérations sensibles (qui a fait quoi, quand, depuis où).
/// `username`/`role` sont des instantanés : la ligne reste lisible même si le compte est supprimé ensuite.
pub mod audit_log {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "audit_log")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub created_at: DateTime,
        pub username: String,
        pub role: Option<String>,
        /// ex. playlist.publish, user.create
        pub action: String,
        /// ex. "playlist:3"
        pub target: Option<String>,
        #[sea_orm(column_type = "Text", nullable)]
        pub detail: Option<String>,
        /// IPv4 ou IPv6 du client
        pub ip: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utcnow_has_microsecond_precision_at_most() {
        for _ in 0..50 {
            assert_eq!(utcnow().nanosecond() % 1000, 0);
        }
    }

    #[test]
    fn utcnow_is_monotonic_enough_and_naive_utc() {
        let a = utcnow();
        let b = utcnow();
        assert!(b >= a);
        let drift = (chrono::Utc::now().naive_utc() - b).num_seconds().abs();
        assert!(drift < 5);
    }
}
