//! Where the database is, read from `mongodb.toml`.
//!
//! # Where the file is looked for
//!
//! 1. `mongodb.toml` beside the executable: the shipped game's own folder, where
//!    the person running the game can see and change it, so it wins.
//! 2. `mongodb.toml` in the project folder, which is what a developer edits
//!    while playtesting from the editor.
//!
//! `RENZORA_MONGODB_URL`, when set, replaces the `url` from whichever file was
//! found (or the default), so a server or a CI run can point the game elsewhere
//! without a file.
//!
//! No file at all is fine: the default is a MongoDB on this machine with no
//! authentication, which is how a fresh local install comes up.
//!
//! No Bevy here, so it can be read and tested on its own.

use std::path::{Path, PathBuf};
use std::time::Duration;

pub const FILE_NAME: &str = "mongodb.toml";
pub const URL_VARIABLE: &str = "RENZORA_MONGODB_URL";

pub const DEFAULT_URL: &str = "mongodb://127.0.0.1:27017";
pub const DEFAULT_DATABASE: &str = "renzora";
pub const DEFAULT_COLLECTION: &str = "renzora_saves";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// `mongodb://user:password@host:port/` or `mongodb+srv://...` (Atlas).
    pub url: String,
    /// The database calls use unless they name another. A URL that names one
    /// (`mongodb://host/game`) sets it too; this key wins over that.
    pub database: String,
    /// The collection save slots go in.
    pub collection: String,
    /// Keep the game's save slots in MongoDB (the save backend). Off leaves
    /// them in the save folder while scripts still query the database.
    pub saves: bool,
    /// Let a networked client run the script functions. Off by default: a
    /// client is the player's machine, so a client that can query holds the
    /// credentials, and anyone can lift them from the file and run anything.
    /// Single-player and the server (host or dedicated) are never refused.
    pub allow_clients: bool,
    /// How long a call waits to reach a server. Short, because a save call
    /// holds the frame it is made in, and MongoDB's own default is 30 seconds.
    pub connect_timeout: Duration,
    /// Other servers a script can name with its `connection` argument.
    pub connections: Vec<(String, String)>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            url: DEFAULT_URL.to_string(),
            database: DEFAULT_DATABASE.to_string(),
            collection: DEFAULT_COLLECTION.to_string(),
            saves: true,
            allow_clients: false,
            connect_timeout: Duration::from_secs(2),
            connections: Vec::new(),
        }
    }
}

impl Config {
    /// The URL for a connection name: `default` (or empty) is `url`.
    pub fn connection_url(&self, name: &str) -> Option<&str> {
        if name.is_empty() || name == "default" {
            return Some(&self.url);
        }
        self.connections.iter().find(|(n, _)| n == name).map(|(_, u)| u.as_str())
    }
}

/// Where a [`Config`] came from, for the log line that says where data goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Defaults,
    File(PathBuf),
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Defaults => write!(f, "defaults (no {FILE_NAME} found)"),
            Self::File(path) => write!(f, "{}", path.display()),
        }
    }
}

/// Read a `mongodb.toml`. An unknown key is refused rather than ignored, so a
/// misspelling is reported instead of quietly using a default.
pub fn parse(text: &str) -> Result<Config, String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.message().to_string())?;
    let mut config = Config::default();
    for (key, value) in &table {
        match key.as_str() {
            "url" => config.url = string(key, value)?,
            "database" => config.database = string(key, value)?,
            "collection" => config.collection = string(key, value)?,
            "saves" => config.saves = value.as_bool().ok_or_else(|| format!("`{key}` must be true or false"))?,
            "allow_clients" => {
                config.allow_clients = value.as_bool().ok_or_else(|| format!("`{key}` must be true or false"))?
            }
            "connect_timeout_secs" => config.connect_timeout = seconds(key, value)?,
            "connections" => {
                let table = value
                    .as_table()
                    .ok_or_else(|| "`connections` must be a table of name = \"mongodb://...\"".to_string())?;
                for (name, url) in table {
                    config.connections.push((name.clone(), string(&format!("connections.{name}"), url)?));
                }
            }
            other => {
                return Err(format!(
                    "unknown key `{other}`; expected url, database, collection, saves, \
                     allow_clients, connect_timeout_secs or connections"
                ))
            }
        }
    }
    if config.database.trim().is_empty() || config.collection.trim().is_empty() {
        return Err("`database` and `collection` cannot be empty".into());
    }
    Ok(config)
}

/// Find and read the config, in the order the module docs give. A file that
/// exists but cannot be read is an error, not a reason to fall through to the
/// next place: a game silently using a different server is the worse outcome.
pub fn find(exe_dir: Option<&Path>, project_dir: Option<&Path>) -> Result<(Config, Source), String> {
    let candidates = [exe_dir, project_dir].into_iter().flatten().map(|dir| dir.join(FILE_NAME));
    let mut found = (Config::default(), Source::Defaults);
    for path in candidates {
        if !path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let config = parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        found = (config, Source::File(path));
        break;
    }
    if let Some(url) = std::env::var(URL_VARIABLE).ok().filter(|u| !u.trim().is_empty()) {
        found.0.url = url;
    }
    Ok(found)
}

/// The URL with any user name and password taken out, for logs and messages.
pub fn redacted(url: &str) -> String {
    let Some(scheme_end) = url.find("://").map(|i| i + 3) else {
        return url.to_string();
    };
    let rest = &url[scheme_end..];
    let authority_end = rest.find('/').unwrap_or(rest.len());
    match rest[..authority_end].rfind('@') {
        Some(at) => format!("{}{}", &url[..scheme_end], &rest[at + 1..]),
        None => url.to_string(),
    }
}

fn string(key: &str, value: &toml::Value) -> Result<String, String> {
    value.as_str().map(str::to_string).ok_or_else(|| format!("`{key}` must be a string"))
}

fn seconds(key: &str, value: &toml::Value) -> Result<Duration, String> {
    let secs = match value {
        toml::Value::Integer(n) => *n as f64,
        toml::Value::Float(f) => *f,
        _ => return Err(format!("`{key}` must be a number of seconds")),
    };
    if secs > 0.0 && secs.is_finite() {
        Ok(Duration::from_secs_f64(secs))
    } else {
        Err(format!("`{key}` must be more than 0"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_is_the_defaults() {
        assert_eq!(parse("").unwrap(), Config::default());
    }

    #[test]
    fn every_key_is_read() {
        let c = parse(
            r#"
            url = "mongodb+srv://game:pw@cluster0.example.net/"
            database = "mygame"
            collection = "slots"
            saves = false
            connect_timeout_secs = 1
            [connections]
            stats = "mongodb://127.0.0.1:27018"
            "#,
        )
        .unwrap();
        assert_eq!(c.database, "mygame");
        assert_eq!(c.collection, "slots");
        assert!(!c.saves);
        assert_eq!(c.connection_url("stats"), Some("mongodb://127.0.0.1:27018"));
        assert_eq!(c.connection_url(""), Some("mongodb+srv://game:pw@cluster0.example.net/"));
    }

    #[test]
    fn a_misspelt_key_is_refused_by_name() {
        assert!(parse(r#"databse = "x""#).unwrap_err().contains("databse"));
    }

    #[test]
    fn credentials_never_reach_a_log_line() {
        assert_eq!(redacted("mongodb://root:hunter2@127.0.0.1:27017/db"), "mongodb://127.0.0.1:27017/db");
        assert_eq!(redacted("mongodb://127.0.0.1:27017"), "mongodb://127.0.0.1:27017");
    }
}
