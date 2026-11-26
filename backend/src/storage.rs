//! Stockage des médias (équivalent de `storage.py`) : type détecté d'après les octets magiques,
//! écriture en flux dans un temporaire, SHA-256 calculé au fil de l'eau, renommage atomique.
//! Les fichiers sont nommés `<sha256>.<ext>` : le même contenu n'est stocké qu'une fois.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

use crate::error::ApiError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaType {
    /// image | video
    pub kind: &'static str,
    pub mime: &'static str,
    pub ext: &'static str,
}

const PNG: MediaType = MediaType { kind: "image", mime: "image/png", ext: "png" };
const JPEG: MediaType = MediaType { kind: "image", mime: "image/jpeg", ext: "jpg" };
const WEBP: MediaType = MediaType { kind: "image", mime: "image/webp", ext: "webp" };
const GIF: MediaType = MediaType { kind: "image", mime: "image/gif", ext: "gif" };
const MP4: MediaType = MediaType { kind: "video", mime: "video/mp4", ext: "mp4" };
const WEBM: MediaType = MediaType { kind: "video", mime: "video/webm", ext: "webm" };

/// Extensions autorisées dans un nom de fichier stocké.
pub const STORED_EXTENSIONS: [&str; 6] = ["png", "jpg", "webp", "gif", "mp4", "webm"];

/// Détermine le type réel d'après les octets magiques (pas le nom ni le Content-Type).
pub fn sniff(head: &[u8]) -> Option<MediaType> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(PNG);
    }
    if head.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(JPEG);
    }
    if head.get(..4) == Some(&b"RIFF"[..]) && head.get(8..12) == Some(&b"WEBP"[..]) {
        return Some(WEBP);
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Some(GIF);
    }
    if head.get(4..8) == Some(&b"ftyp"[..]) {
        return Some(MP4);
    }
    if head.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return Some(WEBM);
    }
    None
}

/// Un nom de fichier stocké ne peut être QUE `<sha256 hexadécimal minuscule>.<extension autorisée>`.
pub fn is_stored_name(name: &str) -> bool {
    let Some((hash, ext)) = name.split_once('.') else { return false };
    hash.len() == 64
        && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && STORED_EXTENSIONS.contains(&ext)
}

/// Résultat d'un envoi enregistré.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedMedia {
    pub kind: &'static str,
    pub mime: &'static str,
    pub sha256: String,
    pub size: i64,
    pub stored_name: String,
}

/// Écrit un envoi en flux. Usage : `begin`, `push` pour chaque morceau, puis `finish`.
/// Le fichier temporaire est supprimé dans tous les cas d'échec (y compris abandon du futur).
pub struct UploadSaver {
    media_dir: PathBuf,
    tmp_path: PathBuf,
    file: Option<tokio::fs::File>,
    hasher: Sha256,
    size: u64,
    head: Vec<u8>,
    max_bytes: u64,
    done: bool,
}

impl UploadSaver {
    pub async fn begin(media_dir: &Path, max_bytes: u64) -> Result<UploadSaver, ApiError> {
        let tmp_dir = media_dir.join(".tmp");
        tokio::fs::create_dir_all(&tmp_dir).await?;
        let tmp_path = tmp_dir.join(format!("upload-{}", hex::encode(rand_bytes())));
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(&tmp_path).await?;
        Ok(UploadSaver {
            media_dir: media_dir.to_path_buf(),
            tmp_path,
            file: Some(file),
            hasher: Sha256::new(),
            size: 0,
            head: Vec::with_capacity(16),
            // la colonne `size` est un INTEGER 32 bits : jamais plus de 2 Gio, quelle que soit la configuration
            max_bytes: max_bytes.min(i32::MAX as u64),
            done: false,
        })
    }

    pub async fn push(&mut self, chunk: &[u8]) -> Result<(), ApiError> {
        if self.head.len() < 16 {
            let need = 16 - self.head.len();
            self.head.extend_from_slice(&chunk[..chunk.len().min(need)]);
        }
        self.size += chunk.len() as u64;
        if self.size > self.max_bytes {
            return Err(ApiError::new(413, "Fichier trop volumineux"));
        }
        self.hasher.update(chunk);
        match self.file.as_mut() {
            Some(f) => f.write_all(chunk).await?,
            None => return Err(ApiError::internal("écriture après la fin de l'envoi")),
        }
        Ok(())
    }

    pub async fn finish(mut self) -> Result<SavedMedia, ApiError> {
        if let Some(mut f) = self.file.take() {
            f.flush().await?;
        }
        let info = sniff(&self.head)
            .ok_or_else(|| ApiError::new(415, "Type de fichier non autorisé (PNG, JPEG, WebP, GIF, MP4, WebM)"))?;
        if self.size == 0 {
            return Err(ApiError::new(400, "Fichier vide"));
        }
        let digest = hex::encode(self.hasher.clone().finalize());
        let stored_name = format!("{digest}.{}", info.ext);
        let final_path = self.media_dir.join(&stored_name);
        if tokio::fs::try_exists(&final_path).await? {
            tokio::fs::remove_file(&self.tmp_path).await?; // doublon : déjà stocké
        } else {
            tokio::fs::rename(&self.tmp_path, &final_path).await?; // remplacement atomique
        }
        self.done = true;
        Ok(SavedMedia {
            kind: info.kind,
            mime: info.mime,
            sha256: digest,
            size: self.size as i64,
            stored_name,
        })
    }
}

impl Drop for UploadSaver {
    fn drop(&mut self) {
        if !self.done {
            let _ = std::fs::remove_file(&self.tmp_path);
        }
    }
}

fn rand_bytes() -> [u8; 16] {
    use rand::{rngs::OsRng, RngCore};
    let mut b = [0u8; 16];
    OsRng.fill_bytes(&mut b);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\0\0\0\0\0";

    #[test]
    fn sniff_recognises_every_allowed_format() {
        assert_eq!(sniff(PNG_BYTES), Some(PNG));
        assert_eq!(sniff(b"\xff\xd8\xff\xe0\0\x10JFIF\0\0\0\0\0"), Some(JPEG));
        assert_eq!(sniff(b"RIFF\x24\0\0\0WEBPVP8 "), Some(WEBP));
        assert_eq!(sniff(b"GIF87a\0\0\0\0\0\0\0\0\0\0"), Some(GIF));
        assert_eq!(sniff(b"GIF89a\0\0\0\0\0\0\0\0\0\0"), Some(GIF));
        assert_eq!(sniff(b"\x00\x00\x00\x18ftypmp42\0\0\0\0"), Some(MP4));
        assert_eq!(sniff(b"\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01\0\0\0\0\0\0\0"), Some(WEBM));
    }

    #[test]
    fn sniff_reports_kind_mime_and_extension() {
        let m = sniff(PNG_BYTES).unwrap();
        assert_eq!((m.kind, m.mime, m.ext), ("image", "image/png", "png"));
        let v = sniff(b"\x00\x00\x00\x18ftypmp42\0\0\0\0").unwrap();
        assert_eq!((v.kind, v.mime, v.ext), ("video", "video/mp4", "mp4"));
        assert_eq!(sniff(b"\xff\xd8\xff").unwrap().ext, "jpg", "jpeg est stocké en .jpg");
    }

    #[test]
    fn sniff_refuses_everything_else() {
        assert_eq!(sniff(b""), None);
        assert_eq!(sniff(b"pas une image"), None);
        assert_eq!(sniff(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), None);
        assert_eq!(sniff(b"<?php echo 1; ?>"), None);
        assert_eq!(sniff(b"%PDF-1.7"), None);
        assert_eq!(sniff(b"RIFF\x24\0\0\0WAVEfmt "), None, "RIFF sans WEBP");
        assert_eq!(sniff(b"RIFF"), None, "tête trop courte : pas de panique");
        assert_eq!(sniff(b"\x89PNG"), None);
    }

    #[test]
    fn stored_names_accept_only_sha256_dot_allowed_extension() {
        let h = "a".repeat(64);
        for ext in STORED_EXTENSIONS {
            assert!(is_stored_name(&format!("{h}.{ext}")), "{ext}");
        }
        assert!(!is_stored_name(&format!("{h}.txt")));
        assert!(!is_stored_name(&format!("{h}.jpeg")));
        assert!(!is_stored_name(&format!("{}.png", "A".repeat(64))), "majuscules refusées");
        assert!(!is_stored_name(&format!("{}.png", "a".repeat(63))));
        assert!(!is_stored_name(&format!("{}.png", "a".repeat(65))));
        assert!(!is_stored_name(&format!("{}.png", "g".repeat(64))));
        assert!(!is_stored_name(&format!("{h}.png.tmp")));
        assert!(!is_stored_name(&format!("../{h}.png")));
        assert!(!is_stored_name(&format!("{h}.png\n")));
        for bad in ["secret.txt", ".tmp", "", ".", "..", "png"] {
            assert!(!is_stored_name(bad), "{bad:?}");
        }
    }

    async fn save(dir: &Path, chunks: &[&[u8]], max: u64) -> Result<SavedMedia, ApiError> {
        let mut s = UploadSaver::begin(dir, max).await?;
        for c in chunks {
            s.push(c).await?;
        }
        s.finish().await
    }

    fn tmp_is_empty(dir: &Path) -> bool {
        std::fs::read_dir(dir.join(".tmp")).map(|d| d.count() == 0).unwrap_or(true)
    }

    #[tokio::test]
    async fn saves_under_its_sha256_name_and_leaves_no_temp_file() {
        let d = tempfile::tempdir().unwrap();
        let mut data = PNG_BYTES.to_vec();
        data.extend_from_slice(&[7u8; 1000]);
        let saved = save(d.path(), &[&data[..]], 1 << 20).await.unwrap();
        assert_eq!(saved.sha256, hex::encode(Sha256::digest(&data)));
        assert_eq!(saved.stored_name, format!("{}.png", saved.sha256));
        assert_eq!((saved.kind, saved.mime, saved.size), ("image", "image/png", data.len() as i64));
        assert_eq!(std::fs::read(d.path().join(&saved.stored_name)).unwrap(), data);
        assert!(tmp_is_empty(d.path()));
    }

    #[tokio::test]
    async fn chunk_boundaries_do_not_change_the_result() {
        let d = tempfile::tempdir().unwrap();
        let mut data = PNG_BYTES.to_vec();
        data.extend((0..5000u32).map(|i| (i % 251) as u8));
        let whole = save(d.path(), &[&data[..]], 1 << 20).await.unwrap();
        let d2 = tempfile::tempdir().unwrap();
        // découpage en morceaux de 3 octets : la détection du type doit encore voir les 16 premiers octets
        let pieces: Vec<&[u8]> = data.chunks(3).collect();
        let split = save(d2.path(), &pieces, 1 << 20).await.unwrap();
        assert_eq!(whole, split);
    }

    #[tokio::test]
    async fn identical_content_is_stored_once() {
        let d = tempfile::tempdir().unwrap();
        let a = save(d.path(), &[PNG_BYTES], 1 << 20).await.unwrap();
        let b = save(d.path(), &[PNG_BYTES], 1 << 20).await.unwrap();
        assert_eq!(a, b);
        let files: Vec<_> = std::fs::read_dir(d.path()).unwrap().filter_map(|e| e.ok()).collect();
        assert_eq!(files.iter().filter(|e| e.file_name().to_string_lossy().ends_with(".png")).count(), 1);
        assert!(tmp_is_empty(d.path()));
    }

    #[tokio::test]
    async fn oversized_upload_is_refused_with_413_and_cleaned_up() {
        let d = tempfile::tempdir().unwrap();
        let big = vec![0u8; 2000];
        let mut data = PNG_BYTES.to_vec();
        data.extend_from_slice(&big);
        let err = save(d.path(), &[&data[..]], 1000).await.unwrap_err();
        assert_eq!(err.status.as_u16(), 413);
        assert!(tmp_is_empty(d.path()), "le temporaire est supprimé");
    }

    #[tokio::test]
    async fn exactly_the_maximum_size_is_accepted() {
        let d = tempfile::tempdir().unwrap();
        let mut data = PNG_BYTES.to_vec();
        data.resize(1000, 0);
        assert!(save(d.path(), &[&data[..]], 1000).await.is_ok());
        data.push(0);
        assert_eq!(save(d.path(), &[&data[..]], 1000).await.unwrap_err().status.as_u16(), 413);
    }

    #[tokio::test]
    async fn wrong_type_is_415_and_empty_file_is_415_too() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(save(d.path(), &[&b"pas une image"[..]], 1 << 20).await.unwrap_err().status.as_u16(), 415);
        assert_eq!(save(d.path(), &[], 1 << 20).await.unwrap_err().status.as_u16(), 415);
        assert!(tmp_is_empty(d.path()));
    }

    #[tokio::test]
    async fn dropping_an_unfinished_upload_removes_the_temp_file() {
        let d = tempfile::tempdir().unwrap();
        {
            let mut s = UploadSaver::begin(d.path(), 1 << 20).await.unwrap();
            s.push(PNG_BYTES).await.unwrap();
            assert!(!tmp_is_empty(d.path()));
        }
        assert!(tmp_is_empty(d.path()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn temp_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let _s = UploadSaver::begin(d.path(), 10).await.unwrap();
        let entry = std::fs::read_dir(d.path().join(".tmp")).unwrap().next().unwrap().unwrap();
        assert_eq!(entry.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }
}
