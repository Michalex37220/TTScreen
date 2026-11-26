//! Publication de versions immuables (équivalent de `publishing.py`).

use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder, Set};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

use crate::models::{media, playlist, playlist_item, playlist_version, utcnow};

#[derive(Debug)]
pub enum PublishError {
    /// Playlist sans élément (→ 422 « Playlist vide »).
    Empty,
    /// Un élément référence un média introuvable.
    MissingMedia(i32),
    Db(DbErr),
}

impl From<DbErr> for PublishError {
    fn from(e: DbErr) -> Self {
        PublishError::Db(e)
    }
}

/// Éléments d'une playlist dans l'ordre de lecture.
pub async fn load_items<C: ConnectionTrait>(db: &C, playlist_id: i32) -> Result<Vec<playlist_item::Model>, DbErr> {
    playlist_item::Entity::find()
        .filter(playlist_item::Column::PlaylistId.eq(playlist_id))
        .order_by_asc(playlist_item::Column::Position)
        .order_by_asc(playlist_item::Column::Id)
        .all(db)
        .await
}

/// Médias référencés par les éléments, indexés par identifiant.
pub async fn load_media_map<C: ConnectionTrait>(
    db: &C,
    items: &[playlist_item::Model],
) -> Result<HashMap<i32, media::Model>, DbErr> {
    let ids: Vec<i32> = items.iter().filter_map(|i| i.media_id).collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = media::Entity::find().filter(media::Column::Id.is_in(ids)).all(db).await?;
    Ok(rows.into_iter().map(|m| (m.id, m)).collect())
}

pub fn media_url(m: &media::Model) -> String {
    format!("/files/{}", m.stored_name)
}

/// Manifeste d'une playlist + identifiants (triés, sans doublon) des médias utilisés.
pub fn build_manifest(
    playlist_id: i32,
    name: &str,
    items: &[playlist_item::Model],
    media_by_id: &HashMap<i32, media::Model>,
) -> Result<(Value, Vec<i32>), PublishError> {
    let mut entries = Vec::with_capacity(items.len());
    let mut media_ids: Vec<i32> = Vec::new();
    for it in items {
        let mut entry = Map::new();
        entry.insert("type".into(), json!(it.item_type));
        entry.insert("duration_s".into(), json!(it.duration_s));
        if it.item_type == "text" {
            entry.insert("text".into(), json!(it.text));
        } else {
            let mid = it.media_id.unwrap_or(0);
            let m = media_by_id.get(&mid).ok_or(PublishError::MissingMedia(mid))?;
            entry.insert(
                "media".into(),
                json!({"id": m.id, "url": media_url(m), "sha256": m.sha256, "size": m.size, "mime": m.mime}),
            );
            media_ids.push(m.id);
        }
        entries.push(Value::Object(entry));
    }
    media_ids.sort_unstable();
    media_ids.dedup();
    Ok((json!({"playlist_id": playlist_id, "name": name, "items": entries}), media_ids))
}

/// Brouillon courant au format lecteur : `version_id` et `number` valent `null`.
pub async fn preview<C: ConnectionTrait>(db: &C, p: &playlist::Model) -> Result<Value, PublishError> {
    let items = load_items(db, p.id).await?;
    let media_by_id = load_media_map(db, &items).await?;
    let (manifest, _) = build_manifest(p.id, &p.name, &items, &media_by_id)?;
    Ok(merge_header(Value::Null, Value::Null, manifest))
}

/// Crée la version suivante. Le commit est fait par l'appelant, dans la même transaction que l'audit.
pub async fn publish<C: ConnectionTrait>(db: &C, p: &playlist::Model) -> Result<playlist_version::Model, PublishError> {
    let items = load_items(db, p.id).await?;
    if items.is_empty() {
        return Err(PublishError::Empty);
    }
    let media_by_id = load_media_map(db, &items).await?;
    let (manifest, media_ids) = build_manifest(p.id, &p.name, &items, &media_by_id)?;
    let last = playlist_version::Entity::find()
        .filter(playlist_version::Column::PlaylistId.eq(p.id))
        .order_by_desc(playlist_version::Column::Number)
        .one(db)
        .await?;
    let version = playlist_version::ActiveModel {
        playlist_id: Set(p.id),
        number: Set(last.map_or(0, |v| v.number) + 1),
        manifest: Set(manifest),
        media_ids: Set(json!(media_ids)),
        created_at: Set(utcnow()),
        ..Default::default()
    }
    .insert(db)
    .await?;
    Ok(version)
}

/// `{"version_id", "number", ...manifeste}` : ce que reçoit le lecteur.
fn merge_header(version_id: Value, number: Value, manifest: Value) -> Value {
    let mut out = Map::new();
    out.insert("version_id".into(), version_id);
    out.insert("number".into(), number);
    if let Value::Object(m) = manifest {
        for (k, v) in m {
            out.insert(k, v);
        }
    }
    Value::Object(out)
}

pub fn player_payload(v: &playlist_version::Model) -> Value {
    merge_header(json!(v.id), json!(v.number), v.manifest.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: i32, ext: &str, kind: &str) -> media::Model {
        media::Model {
            id,
            original_name: format!("f{id}"),
            stored_name: format!("{}.{ext}", format!("{id:x}").repeat(64).chars().take(64).collect::<String>()),
            sha256: "ab".repeat(32),
            kind: kind.into(),
            mime: format!("{kind}/x"),
            size: 123,
            created_at: utcnow(),
        }
    }

    fn it(pos: i32, t: &str, text: Option<&str>, media_id: Option<i32>, dur: Option<i32>) -> playlist_item::Model {
        playlist_item::Model {
            id: pos + 100,
            playlist_id: 1,
            position: pos,
            item_type: t.into(),
            text: text.map(Into::into),
            media_id,
            duration_s: dur,
        }
    }

    fn media_map(list: Vec<media::Model>) -> HashMap<i32, media::Model> {
        list.into_iter().map(|x| (x.id, x)).collect()
    }

    #[test]
    fn manifest_shape_matches_the_documented_format() {
        let items = vec![
            it(0, "text", Some("Bienvenue"), None, Some(5)),
            it(1, "image", None, Some(1), Some(10)),
            it(2, "video", None, Some(2), None),
        ];
        let (manifest, ids) = build_manifest(7, "Hall", &items, &media_map(vec![m(1, "png", "image"), m(2, "mp4", "video")])).unwrap();
        assert_eq!(manifest["playlist_id"], 7);
        assert_eq!(manifest["name"], "Hall");
        let list = manifest["items"].as_array().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0], json!({"type": "text", "duration_s": 5, "text": "Bienvenue"}));
        assert_eq!(list[1]["type"], "image");
        assert_eq!(list[1]["media"]["id"], 1);
        assert!(list[1]["media"]["url"].as_str().unwrap().starts_with("/files/"));
        assert_eq!(list[1]["media"]["sha256"], "ab".repeat(32));
        assert_eq!(list[1]["media"]["size"], 123);
        assert_eq!(list[2]["duration_s"], Value::Null, "vidéo sans durée : lecture complète");
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn manifest_keys_keep_the_original_order() {
        let (manifest, _) = build_manifest(1, "P", &[it(0, "text", Some("a"), None, Some(5))], &HashMap::new()).unwrap();
        let keys: Vec<&String> = manifest.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["playlist_id", "name", "items"]);
        let entry_keys: Vec<&String> = manifest["items"][0].as_object().unwrap().keys().collect();
        assert_eq!(entry_keys, ["type", "duration_s", "text"]);
    }

    #[test]
    fn media_ids_are_sorted_and_deduplicated() {
        let items = vec![
            it(0, "image", None, Some(5), Some(1)),
            it(1, "image", None, Some(2), Some(1)),
            it(2, "image", None, Some(5), Some(1)),
        ];
        let (_, ids) = build_manifest(1, "P", &items, &media_map(vec![m(5, "png", "image"), m(2, "png", "image")])).unwrap();
        assert_eq!(ids, vec![2, 5]);
    }

    #[test]
    fn missing_media_is_an_error_not_a_panic() {
        let r = build_manifest(1, "P", &[it(0, "image", None, Some(9), Some(1))], &HashMap::new());
        assert!(matches!(r, Err(PublishError::MissingMedia(9))));
    }

    #[test]
    fn player_payload_puts_version_fields_first() {
        let v = playlist_version::Model {
            id: 3,
            playlist_id: 1,
            number: 2,
            manifest: json!({"playlist_id": 1, "name": "Accueil", "items": []}),
            media_ids: json!([]),
            created_at: utcnow(),
        };
        let p = player_payload(&v);
        assert_eq!((p["version_id"].as_i64(), p["number"].as_i64()), (Some(3), Some(2)));
        assert_eq!(p["name"], "Accueil");
        let keys: Vec<&String> = p.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["version_id", "number", "playlist_id", "name", "items"]);
    }

    #[test]
    fn draft_header_has_null_version_and_number() {
        let p = merge_header(Value::Null, Value::Null, json!({"playlist_id": 1, "name": "x", "items": []}));
        assert!(p["version_id"].is_null() && p["number"].is_null());
        assert_eq!(p["playlist_id"], 1);
    }

    #[test]
    fn media_url_uses_the_stored_name() {
        assert_eq!(media_url(&m(1, "png", "image")), format!("/files/{}", m(1, "png", "image").stored_name));
    }
}
