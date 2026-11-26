//! Authentification par comptes et rôles (équivalent de `auth.py`).
//!
//! Deux façons de s'authentifier sur les routes d'administration :
//! - un **compte nommé** (identifiant + mot de passe) : `POST /api/auth/login` renvoie un jeton de
//!   session, stocké haché en base (table `user_sessions`) comme les jetons d'écran ;
//! - le **jeton admin historique** (`TTSCREEN_ADMIN_TOKEN`), accepté pour amorcer le système :
//!   il équivaut à un compte « admin ».
//!
//! Le format des hachés de mots de passe est celui de la version Python
//! (`pbkdf2_sha256$itérations$sel_hex$haché_hex`) : les comptes existants continuent de fonctionner.

use chrono::{Duration, NaiveDateTime};
use pbkdf2::pbkdf2_hmac;
use rand::{rngs::OsRng, RngCore};
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, PaginatorTrait, QueryFilter, Set};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::models::{user, user_session, utcnow};

/// Itérations PBKDF2-HMAC-SHA256 (recommandation OWASP : 600 000).
pub const PBKDF2_ITERATIONS: u32 = 600_000;

/// Nom réservé du principal « jeton admin historique » : impossible à usurper par un compte nommé
/// (les journaux distinguent ainsi sans ambiguïté un compte « admin » du jeton historique).
pub const LEGACY_USERNAME: &str = "legacy-admin";
pub const RESERVED_USERNAMES: [&str; 2] = [LEGACY_USERNAME, "anonyme"];

/// Hiérarchie des rôles : chaque rôle englobe les précédents.
pub const ROLES: [&str; 3] = ["viewer", "editor", "admin"];

pub fn is_reserved(username: &str) -> bool {
    let lower = username.to_lowercase();
    RESERVED_USERNAMES.iter().any(|r| *r == lower)
}

fn role_index(role: &str) -> Option<usize> {
    ROLES.iter().position(|r| *r == role)
}

/// True si `role` atteint au moins le niveau `minimum` (viewer < editor < admin).
/// Un rôle inconnu n'a aucun droit ; un niveau minimum inconnu n'est atteint par personne.
pub fn role_allows(role: &str, minimum: &str) -> bool {
    match (role_index(role), role_index(minimum)) {
        (Some(have), Some(need)) => have >= need,
        _ => false,
    }
}

/// Comparaison en temps constant sur des octets : accepte n'importe quel caractère Unicode.
pub fn safe_equal(a: &str, b: &str) -> bool {
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

/// Empreinte SHA-256 d'un jeton : c'est ce qui est stocké en base, jamais le jeton en clair.
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Jeton aléatoire sûr pour les URL (32 octets, 43 caractères), comme `secrets.token_urlsafe(32)`.
pub fn new_token() -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let mut buf = [0u8; 32];
    OsRng.fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// Hache un mot de passe (PBKDF2-HMAC-SHA256 + sel aléatoire) au format `pbkdf2_sha256$iter$sel$haché`.
pub fn hash_password(password: &str, iterations: u32) -> String {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let mut digest = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, iterations, &mut digest);
    format!("pbkdf2_sha256${iterations}${}${}", hex::encode(salt), hex::encode(digest))
}

/// Haché factice (même coût PBKDF2) pour égaliser le temps de réponse quand l'identifiant n'existe pas.
pub fn dummy_hash(iterations: u32) -> String {
    hash_password(&new_token(), iterations)
}

/// Vérifie un mot de passe contre le haché stocké, en temps constant. Un haché mal formé est refusé.
pub fn verify_password(password: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    if parts.len() != 4 {
        return false;
    }
    let Ok(iterations) = parts[1].parse::<u32>() else { return false };
    if iterations == 0 {
        return false;
    }
    let Ok(salt) = hex::decode(parts[2]) else { return false };
    let mut digest = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, iterations, &mut digest);
    bool::from(hex::encode(digest).as_bytes().ct_eq(parts[3].as_bytes()))
}

/// Identité authentifiée derrière une requête (compte nommé ou jeton admin historique).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    pub username: String,
    /// viewer | editor | admin
    pub role: String,
    /// True = jeton admin historique, pas un compte en base.
    pub legacy: bool,
}

impl Principal {
    pub fn legacy_admin() -> Principal {
        Principal { username: LEGACY_USERNAME.to_string(), role: "admin".to_string(), legacy: true }
    }
}

/// Crée une session : le jeton en clair n'est renvoyé qu'une fois au client, seule son empreinte est conservée.
pub async fn create_session<C: ConnectionTrait>(
    db: &C,
    user_id: i32,
    ttl: Duration,
) -> Result<(String, NaiveDateTime), DbErr> {
    let token = new_token();
    let now = utcnow();
    let expires_at = now + ttl;
    user_session::ActiveModel {
        user_id: Set(user_id),
        token_hash: Set(hash_token(&token)),
        created_at: Set(now),
        expires_at: Set(expires_at),
        ..Default::default()
    }
    .insert(db)
    .await?;
    Ok((token, expires_at))
}

/// Nombre de comptes admin (actifs seulement si `active_only`).
pub async fn count_admins<C: ConnectionTrait>(db: &C, active_only: bool, except_id: Option<i32>) -> Result<u64, DbErr> {
    let mut q = user::Entity::find().filter(user::Column::Role.eq("admin"));
    if active_only {
        q = q.filter(user::Column::Active.eq(true));
    }
    if let Some(id) = except_id {
        q = q.filter(user::Column::Id.ne(id));
    }
    q.count(db).await
}

/// Le jeton admin historique n'est accepté que tant qu'aucun compte admin nommé actif n'existe
/// (sauf `TTSCREEN_LEGACY_TOKEN_ALWAYS`) : il sert à amorcer, pas à rester une porte dérobée permanente.
pub async fn legacy_allowed<C: ConnectionTrait>(db: &C, legacy_always: bool) -> Result<bool, DbErr> {
    if legacy_always {
        return Ok(true);
    }
    Ok(count_admins(db, true, None).await? == 0)
}

/// Transforme un jeton Bearer en identité, ou `None` s'il est invalide.
///
/// Le jeton admin historique (configuré et encore autorisé) est comparé en temps constant.
/// Un jeton de session n'est accepté que si le compte existe, est actif et la session non expirée.
pub async fn resolve_token<C: ConnectionTrait>(
    db: &C,
    token: &str,
    admin_token: &str,
    legacy_always: bool,
) -> Result<Option<Principal>, DbErr> {
    if !admin_token.is_empty() && safe_equal(token, admin_token) && legacy_allowed(db, legacy_always).await? {
        return Ok(Some(Principal::legacy_admin()));
    }
    let session = user_session::Entity::find()
        .filter(user_session::Column::TokenHash.eq(hash_token(token)))
        .one(db)
        .await?;
    let Some(session) = session else { return Ok(None) };
    let account = user::Entity::find_by_id(session.user_id).one(db).await?;
    match account {
        Some(u) if u.active && session.expires_at >= utcnow() => {
            Ok(Some(Principal { username: u.username, role: u.role, legacy: false }))
        }
        _ => Ok(None),
    }
}

/// Révoque toutes les sessions d'un compte (désactivation, suppression, changement de mot de passe).
pub async fn drop_sessions<C: ConnectionTrait>(db: &C, user_id: i32) -> Result<(), DbErr> {
    user_session::Entity::delete_many()
        .filter(user_session::Column::UserId.eq(user_id))
        .exec(db)
        .await?;
    Ok(())
}

/// Révoque les sessions d'un compte sauf celle dont l'empreinte est `keep_hash` (session courante).
pub async fn drop_sessions_except<C: ConnectionTrait>(db: &C, user_id: i32, keep_hash: Option<&str>) -> Result<(), DbErr> {
    let mut q = user_session::Entity::delete_many().filter(user_session::Column::UserId.eq(user_id));
    if let Some(h) = keep_hash {
        q = q.filter(user_session::Column::TokenHash.ne(h));
    }
    q.exec(db).await?;
    Ok(())
}

/// Crée le compte admin d'amorçage si `TTSCREEN_BOOTSTRAP_ADMIN_*` sont renseignés ET qu'aucun admin n'existe.
///
/// Un compte existant n'est JAMAIS modifié au redémarrage (sinon un mot de passe changé via l'interface
/// serait écrasé et un compte désactivé volontairement serait réactivé). Pour récupérer l'accès,
/// positionner explicitement `TTSCREEN_BOOTSTRAP_ADMIN_RESET=true` (réinitialise mot de passe, rôle, état).
pub async fn bootstrap_admin<C: ConnectionTrait>(
    db: &C,
    username: &str,
    password: &str,
    reset: bool,
    iterations: u32,
) -> anyhow::Result<()> {
    if username.is_empty() || password.is_empty() {
        return Ok(());
    }
    if is_reserved(username) {
        anyhow::bail!("Nom d'utilisateur d'amorçage réservé : {username:?}");
    }
    let existing = user::Entity::find().filter(user::Column::Username.eq(username)).one(db).await?;
    match existing {
        None => {
            if reset || count_admins(db, false, None).await? == 0 {
                user::ActiveModel {
                    username: Set(username.to_string()),
                    password_hash: Set(hash_password(password, iterations)),
                    role: Set("admin".to_string()),
                    active: Set(true),
                    created_at: Set(utcnow()),
                    ..Default::default()
                }
                .insert(db)
                .await?;
            }
        }
        Some(account) if reset => {
            let id = account.id;
            let mut am: user::ActiveModel = account.into();
            am.password_hash = Set(hash_password(password, iterations));
            am.role = Set("admin".to_string());
            am.active = Set(true);
            am.update(db).await?;
            drop_sessions(db, id).await?;
        }
        Some(_) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: u32 = 1000;

    #[test]
    fn password_hash_has_the_python_compatible_format() {
        let h = hash_password("secret123", FAST);
        let parts: Vec<&str> = h.split('$').collect();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "pbkdf2_sha256");
        assert_eq!(parts[1], "1000");
        assert_eq!(parts[2].len(), 32, "sel de 16 octets en hexadécimal");
        assert_eq!(parts[3].len(), 64, "SHA-256 en hexadécimal");
        assert_ne!(h, "secret123", "jamais de stockage en clair");
    }

    #[test]
    fn password_verification_accepts_only_the_right_password() {
        let h = hash_password("secret123", FAST);
        assert!(verify_password("secret123", &h));
        assert!(!verify_password("secret124", &h));
        assert!(!verify_password("", &h));
        assert!(!verify_password("SECRET123", &h));
    }

    #[test]
    fn malformed_hashes_are_refused_not_panicking() {
        for bad in ["haché mal formé", "", "a$b$c", "a$b$c$d$e", "pbkdf2_sha256$x$00$00", "pbkdf2_sha256$0$00$00",
                    "pbkdf2_sha256$1000$zz$00", "pbkdf2_sha256$-5$00$00"] {
            assert!(!verify_password("x", bad), "{bad:?}");
        }
    }

    #[test]
    fn hashes_are_salted() {
        assert_ne!(hash_password("same", FAST), hash_password("same", FAST));
    }

    #[test]
    fn verification_reads_the_iteration_count_from_the_hash() {
        // un haché produit avec un autre nombre d'itérations reste vérifiable
        let h = hash_password("pw-itérations", 1234);
        assert!(h.starts_with("pbkdf2_sha256$1234$"));
        assert!(verify_password("pw-itérations", &h));
    }

    #[test]
    fn known_vector_from_the_python_implementation() {
        // hashlib.pbkdf2_hmac("sha256", b"password", b"salt", 1, 32).hex() (vecteur RFC 7914 §11)
        let stored = "pbkdf2_sha256$1$73616c74$120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b";
        assert!(verify_password("password", stored));
        assert!(!verify_password("passw0rd", stored));
    }

    #[test]
    fn unicode_passwords_work() {
        let h = hash_password("mot-de-passe-é☃", FAST);
        assert!(verify_password("mot-de-passe-é☃", &h));
        assert!(!verify_password("mot-de-passe-e☃", &h));
    }

    #[test]
    fn dummy_hash_is_a_valid_hash_that_matches_nothing_guessable() {
        let d = dummy_hash(FAST);
        assert!(d.starts_with("pbkdf2_sha256$1000$"));
        assert!(!verify_password("", &d));
        assert!(!verify_password("whatever1", &d));
    }

    #[test]
    fn safe_equal_handles_unicode_and_lengths() {
        assert!(safe_equal("é☃", "é☃"));
        assert!(!safe_equal("é☃", "é☂"));
        assert!(!safe_equal("abc", "abcd"));
        assert!(safe_equal("", ""));
    }

    #[test]
    fn token_hash_is_sha256_hex() {
        assert_eq!(hash_token("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hash_token("abc").len(), 64);
    }

    #[test]
    fn new_tokens_are_unique_urlsafe_and_43_chars() {
        let a = new_token();
        let b = new_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn role_hierarchy() {
        assert!(role_allows("admin", "viewer") && role_allows("admin", "editor") && role_allows("admin", "admin"));
        assert!(role_allows("editor", "viewer") && role_allows("editor", "editor"));
        assert!(!role_allows("editor", "admin"));
        assert!(role_allows("viewer", "viewer"));
        assert!(!role_allows("viewer", "editor") && !role_allows("viewer", "admin"));
    }

    #[test]
    fn unknown_roles_have_no_rights() {
        assert!(!role_allows("root", "viewer"));
        assert!(!role_allows("", "viewer"));
        assert!(!role_allows("admin", "superuser"), "niveau inconnu : refusé");
    }

    #[test]
    fn reserved_usernames_are_case_insensitive() {
        assert!(is_reserved("legacy-admin") && is_reserved("LEGACY-ADMIN") && is_reserved("Anonyme"));
        assert!(!is_reserved("admin") && !is_reserved("emma"));
    }

    #[test]
    fn legacy_principal_is_an_admin_flagged_legacy() {
        let p = Principal::legacy_admin();
        assert_eq!((p.username.as_str(), p.role.as_str(), p.legacy), ("legacy-admin", "admin", true));
    }
}
