//! Configuration par variables d'environnement (équivalent de `config.py`).

use std::path::PathBuf;
use std::str::FromStr;

#[derive(Clone, Debug)]
pub struct Settings {
    /// Base de données (modifiable mais à vos risques). Formes acceptées : `sqlite:///./data/ttscreen.db`,
    /// `postgresql+psycopg://utilisateur:mdp@hôte:5432/base`, `postgresql://…`, `postgres://…`.
    pub database_url: String,
    pub media_dir: PathBuf,
    /// Jeton admin historique. Vide = désactivé (plus aucune valeur par défaut connue).
    /// Si renseigné : 24 caractères minimum, pas de valeur triviale, dans TOUS les environnements.
    pub admin_token: String,
    pub max_upload_mb: u64,
    pub env: String,
    /// Limitation de débit (fenêtre glissante en mémoire, voir `ratelimit.rs`) ; 0 = désactivé.
    pub rate_window_s: u64,
    pub rate_limit_api: i64,
    pub rate_limit_player: i64,
    pub rate_limit_login: i64,
    pub rate_limit_authfail: i64,
    pub rate_limit_login_user: i64,
    /// Durée de validité d'une session utilisateur, en heures.
    pub session_ttl_h: i64,
    /// Par défaut le jeton historique est refusé dès qu'un compte admin nommé actif existe.
    pub legacy_token_always: bool,
    /// Par défaut le compte d'amorçage n'est créé que s'il n'existe aucun admin ; `true` = réinitialise
    /// explicitement mot de passe/rôle/état de ce compte (récupération d'accès).
    pub bootstrap_admin_reset: bool,
    pub bootstrap_admin_user: String,
    pub bootstrap_admin_password: String,

    // --- ajouts du port Rust (voir README) ---
    /// Équivalent de `--forwarded-allow-ips` d'uvicorn : adresses/réseaux dont on accepte `X-Forwarded-For`
    /// (`*` = tous). Défaut : `127.0.0.1`.
    pub forwarded_allow_ips: String,
    /// Fuseau horaire (nom IANA, ex. `Europe/Paris`) dans lequel sont interprétés les plannings des écrans.
    pub timezone: String,
    /// Itérations PBKDF2 pour les NOUVEAUX hachés (la vérification lit le nombre stocké dans le haché).
    /// Ne pas descendre sous 600 000 en production ; réduit uniquement par les tests.
    pub pbkdf2_iterations: u32,
    pub frontend_dir: PathBuf,
    pub player_dir: PathBuf,
}

/// Racine du dépôt (le crate est dans `backend/`), utilisée pour trouver `frontend/` et `player/`.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

impl Default for Settings {
    fn default() -> Self {
        let root = repo_root();
        Settings {
            database_url: "sqlite:///./data/ttscreen.db".to_string(),
            media_dir: PathBuf::from("./data/media"),
            admin_token: String::new(),
            max_upload_mb: 200,
            env: "development".to_string(),
            rate_window_s: 60,
            rate_limit_api: 600,
            rate_limit_player: 120,
            rate_limit_login: 20,
            rate_limit_authfail: 20,
            rate_limit_login_user: 10,
            session_ttl_h: 12,
            legacy_token_always: false,
            bootstrap_admin_reset: false,
            bootstrap_admin_user: String::new(),
            bootstrap_admin_password: String::new(),
            forwarded_allow_ips: "127.0.0.1".to_string(),
            timezone: "Europe/Paris".to_string(),
            pbkdf2_iterations: crate::auth::PBKDF2_ITERATIONS,
            frontend_dir: root.join("frontend"),
            player_dir: root.join("player"),
        }
    }
}

/// Accès aux variables via une fonction (l'environnement du processus en production,
/// un simple dictionnaire dans les tests : pas d'état global à modifier).
struct Env<'a> {
    get: &'a dyn Fn(&str) -> Option<String>,
}

impl Env<'_> {
    /// Chaîne ; une variable absente OU vide donne la valeur par défaut.
    fn string(&self, name: &str, default: &str) -> String {
        match (self.get)(name) {
            Some(v) if !v.trim().is_empty() => v,
            _ => default.to_string(),
        }
    }

    /// Chaîne brute (vide autorisée) : jetons, mots de passe, identifiants.
    fn raw(&self, name: &str) -> String {
        (self.get)(name).unwrap_or_default()
    }

    fn num<T: FromStr>(&self, name: &str, default: T) -> Result<T, String> {
        match (self.get)(name) {
            Some(v) if !v.trim().is_empty() => v
                .trim()
                .parse::<T>()
                .map_err(|_| format!("{name} doit être un nombre entier (valeur reçue : {v:?})")),
            _ => Ok(default),
        }
    }

    fn flag(&self, name: &str) -> bool {
        matches!(
            (self.get)(name).unwrap_or_default().trim().to_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    }
}

impl Settings {
    /// Lit la configuration depuis l'environnement du processus.
    pub fn from_env() -> Result<Settings, String> {
        Settings::from_lookup(&|k| std::env::var(k).ok())
    }

    /// Même chose, depuis une fonction de recherche arbitraire (utilisé par les tests).
    pub fn from_lookup(get: &dyn Fn(&str) -> Option<String>) -> Result<Settings, String> {
        let d = Settings::default();
        let e = Env { get };
        Ok(Settings {
            database_url: e.string("TTSCREEN_DATABASE_URL", &d.database_url),
            media_dir: PathBuf::from(e.string("TTSCREEN_MEDIA_DIR", &d.media_dir.to_string_lossy())),
            admin_token: e.raw("TTSCREEN_ADMIN_TOKEN"),
            max_upload_mb: e.num("TTSCREEN_MAX_UPLOAD_MB", d.max_upload_mb)?,
            env: e.string("TTSCREEN_ENV", &d.env),
            rate_window_s: e.num("TTSCREEN_RATE_WINDOW_S", d.rate_window_s)?,
            rate_limit_api: e.num("TTSCREEN_RATE_LIMIT_API", d.rate_limit_api)?,
            rate_limit_player: e.num("TTSCREEN_RATE_LIMIT_PLAYER", d.rate_limit_player)?,
            rate_limit_login: e.num("TTSCREEN_RATE_LIMIT_LOGIN", d.rate_limit_login)?,
            rate_limit_authfail: e.num("TTSCREEN_RATE_LIMIT_AUTHFAIL", d.rate_limit_authfail)?,
            rate_limit_login_user: e.num("TTSCREEN_RATE_LIMIT_LOGIN_USER", d.rate_limit_login_user)?,
            session_ttl_h: e.num("TTSCREEN_SESSION_TTL_H", d.session_ttl_h)?,
            legacy_token_always: e.flag("TTSCREEN_LEGACY_TOKEN_ALWAYS"),
            bootstrap_admin_reset: e.flag("TTSCREEN_BOOTSTRAP_ADMIN_RESET"),
            bootstrap_admin_user: e.raw("TTSCREEN_BOOTSTRAP_ADMIN_USER"),
            bootstrap_admin_password: e.raw("TTSCREEN_BOOTSTRAP_ADMIN_PASSWORD"),
            forwarded_allow_ips: e.string("FORWARDED_ALLOW_IPS", &d.forwarded_allow_ips),
            timezone: e.string("TTSCREEN_TIMEZONE", &d.timezone),
            pbkdf2_iterations: d.pbkdf2_iterations,
            frontend_dir: PathBuf::from(e.string("TTSCREEN_FRONTEND_DIR", &d.frontend_dir.to_string_lossy())),
            player_dir: PathBuf::from(e.string("TTSCREEN_PLAYER_DIR", &d.player_dir.to_string_lossy())),
        })
    }

    pub fn is_production(&self) -> bool {
        self.env == "production"
    }

    /// Taille maximale d'un fichier téléversé, en octets.
    pub fn max_upload_bytes(&self) -> u64 {
        self.max_upload_mb.saturating_mul(1024 * 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> Result<Settings, String> {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Settings::from_lookup(&move |k| map.get(k).cloned())
    }

    #[test]
    fn defaults_match_the_python_dataclass() {
        let s = Settings::default();
        assert_eq!(s.database_url, "sqlite:///./data/ttscreen.db");
        assert_eq!(s.media_dir, PathBuf::from("./data/media"));
        assert_eq!(s.admin_token, "", "aucun jeton par défaut connu");
        assert_eq!(s.max_upload_mb, 200);
        assert_eq!(s.env, "development");
        assert_eq!(
            (s.rate_window_s, s.rate_limit_api, s.rate_limit_player, s.rate_limit_login),
            (60, 600, 120, 20)
        );
        assert_eq!((s.rate_limit_authfail, s.rate_limit_login_user, s.session_ttl_h), (20, 10, 12));
        assert!(!s.legacy_token_always && !s.bootstrap_admin_reset);
        assert_eq!(s.pbkdf2_iterations, 600_000);
        assert_eq!(s.forwarded_allow_ips, "127.0.0.1");
        assert_eq!(s.timezone, "Europe/Paris");
    }

    #[test]
    fn empty_environment_gives_defaults() {
        let s = lookup(&[]).unwrap();
        assert_eq!(s.max_upload_mb, 200);
        assert_eq!(s.env, "development");
        assert!(s.admin_token.is_empty());
    }

    #[test]
    fn every_documented_variable_is_read() {
        let s = lookup(&[
            ("TTSCREEN_DATABASE_URL", "postgresql+psycopg://u:p@db:5432/x"),
            ("TTSCREEN_MEDIA_DIR", "/data/media"),
            ("TTSCREEN_ADMIN_TOKEN", "Zq8vN3xK1mP7wR5tY2uL9aB4"),
            ("TTSCREEN_MAX_UPLOAD_MB", "5"),
            ("TTSCREEN_ENV", "production"),
            ("TTSCREEN_SESSION_TTL_H", "2"),
            ("TTSCREEN_RATE_WINDOW_S", "30"),
            ("TTSCREEN_RATE_LIMIT_API", "7"),
            ("TTSCREEN_RATE_LIMIT_PLAYER", "8"),
            ("TTSCREEN_RATE_LIMIT_LOGIN", "9"),
            ("TTSCREEN_RATE_LIMIT_AUTHFAIL", "10"),
            ("TTSCREEN_RATE_LIMIT_LOGIN_USER", "11"),
            ("TTSCREEN_BOOTSTRAP_ADMIN_USER", "root"),
            ("TTSCREEN_BOOTSTRAP_ADMIN_PASSWORD", "motdepasse8"),
            ("FORWARDED_ALLOW_IPS", "*"),
            ("TTSCREEN_TIMEZONE", "America/Montreal"),
            ("TTSCREEN_FRONTEND_DIR", "/app/frontend"),
            ("TTSCREEN_PLAYER_DIR", "/app/player"),
        ])
        .unwrap();
        assert_eq!(s.database_url, "postgresql+psycopg://u:p@db:5432/x");
        assert_eq!(s.media_dir, PathBuf::from("/data/media"));
        assert_eq!(s.admin_token, "Zq8vN3xK1mP7wR5tY2uL9aB4");
        assert_eq!((s.max_upload_mb, s.session_ttl_h, s.rate_window_s), (5, 2, 30));
        assert_eq!((s.rate_limit_api, s.rate_limit_player, s.rate_limit_login), (7, 8, 9));
        assert_eq!((s.rate_limit_authfail, s.rate_limit_login_user), (10, 11));
        assert!(s.is_production());
        assert_eq!((s.bootstrap_admin_user.as_str(), s.bootstrap_admin_password.as_str()), ("root", "motdepasse8"));
        assert_eq!(s.forwarded_allow_ips, "*");
        assert_eq!(s.timezone, "America/Montreal");
        assert_eq!(s.frontend_dir, PathBuf::from("/app/frontend"));
        assert_eq!(s.player_dir, PathBuf::from("/app/player"));
    }

    #[test]
    fn boolean_flags_accept_the_same_spellings_as_python() {
        for yes in ["1", "true", "TRUE", "yes", "On", " true "] {
            assert!(lookup(&[("TTSCREEN_LEGACY_TOKEN_ALWAYS", yes)]).unwrap().legacy_token_always, "{yes:?}");
        }
        for no in ["", "0", "false", "no", "off", "peut-être"] {
            assert!(!lookup(&[("TTSCREEN_LEGACY_TOKEN_ALWAYS", no)]).unwrap().legacy_token_always, "{no:?}");
        }
        assert!(lookup(&[("TTSCREEN_BOOTSTRAP_ADMIN_RESET", "true")]).unwrap().bootstrap_admin_reset);
    }

    #[test]
    fn invalid_numbers_are_reported_with_the_variable_name() {
        let err = lookup(&[("TTSCREEN_MAX_UPLOAD_MB", "beaucoup")]).unwrap_err();
        assert!(err.contains("TTSCREEN_MAX_UPLOAD_MB"), "{err}");
        assert!(lookup(&[("TTSCREEN_RATE_LIMIT_API", "-1")]).is_ok(), "négatif accepté (équivaut à illimité)");
    }

    #[test]
    fn blank_numeric_variable_falls_back_to_default() {
        // `TTSCREEN_RATE_LIMIT_API=` dans un .env ne doit pas empêcher le démarrage
        assert_eq!(lookup(&[("TTSCREEN_RATE_LIMIT_API", "  ")]).unwrap().rate_limit_api, 600);
    }

    #[test]
    fn max_upload_bytes_is_in_mebibytes() {
        let s = Settings { max_upload_mb: 2, ..Settings::default() };
        assert_eq!(s.max_upload_bytes(), 2 * 1024 * 1024);
    }
}
