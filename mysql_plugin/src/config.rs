//! Where the database is, read from `mysql.toml`.
//!
//! Free of Bevy and of the engine on purpose, like `db.rs`: the two together are
//! everything that talks to MySQL, and keeping them plain Rust means they can be
//! compiled and tried against a real server on their own, without an engine
//! build to load them into.
//!
//! # Where the file is looked for
//!
//! 1. `mysql.toml` beside the executable. That is the shipped game's own folder,
//!    which is where someone running the game on their machine can see and
//!    change it, and it wins so the person running the game has the last word.
//! 2. `mysql.toml` in the project folder, which is what a developer edits while
//!    playtesting from the editor.
//!
//! `RENZORA_MYSQL_URL`, when set, replaces the `url` from whichever file was
//! found (or the default when none was), so a server or a CI run can point the
//! game somewhere else without a file.
//!
//! No file at all is fine: the defaults are a MySQL on this machine with the
//! stock `root` account and no password, which is how a fresh local install
//! (and the official Docker image with `MYSQL_ALLOW_EMPTY_PASSWORD`) comes up.

use std::path::{Path, PathBuf};
use std::time::Duration;

pub const FILE_NAME: &str = "mysql.toml";
pub const URL_VARIABLE: &str = "RENZORA_MYSQL_URL";

pub const DEFAULT_URL: &str = "mysql://root@127.0.0.1:3306/renzora_saves";
pub const DEFAULT_TABLE: &str = "renzora_saves";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// `mysql://user:password@host:port/database`.
    pub url: String,
    /// The table slots are kept in. Checked to be a plain identifier, since it
    /// is written into SQL text: a table name cannot be a bound parameter.
    pub table: String,
    /// Create the database named in the URL when it does not exist yet. On by
    /// default because the database is local: the first save making it is less
    /// surprising than a game that cannot save until someone runs a `CREATE`.
    pub create_database: bool,
    /// How long to wait for the server to accept a connection. Short, because a
    /// save call holds the frame it is made in.
    pub connect_timeout: Duration,
    /// How long one read or write may take once connected.
    pub io_timeout: Duration,
    /// Keep the game's save slots in MySQL (the save backend). Off leaves
    /// saves in the save folder while scripts still query the database. Only
    /// read from a file: with no `mysql.toml` the slots are never claimed,
    /// whatever this default says.
    pub saves: bool,
    /// Let a networked client run the script functions. Off by default: a
    /// client is the player's machine, so a client that can query holds the
    /// credentials, and anyone can lift them from the file and run any SQL.
    /// Single-player and the server (host or dedicated) are never refused.
    pub allow_clients: bool,
    /// Other databases a script can name in a query, by name. `url` is the one
    /// called `default`.
    pub connections: Vec<(String, String)>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            url: DEFAULT_URL.to_string(),
            table: DEFAULT_TABLE.to_string(),
            create_database: true,
            connect_timeout: Duration::from_secs(2),
            io_timeout: Duration::from_secs(5),
            saves: true,
            allow_clients: false,
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

/// Where a [`Config`] came from, for the log line that says where saves go.
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

/// Read a `mysql.toml`.
///
/// Parsed into `toml`'s own table rather than derived into the struct: a
/// derive would be against the engine's serde (see the plugin docs on
/// `renzora::serde`), and `toml` here is built against a private copy, so the
/// two would not meet. Five keys are no hardship to read by hand.
///
/// An unknown key is refused rather than ignored, so `tabel = "x"` is reported
/// instead of quietly saving to the default table.
pub fn parse(text: &str) -> Result<Config, String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.message().to_string())?;
    let mut config = Config::default();
    for (key, value) in &table {
        match key.as_str() {
            "url" => config.url = string(key, value)?,
            "table" => config.table = string(key, value)?,
            "create_database" => {
                config.create_database = value
                    .as_bool()
                    .ok_or_else(|| format!("`{key}` must be true or false"))?
            }
            "connect_timeout_secs" => config.connect_timeout = seconds(key, value)?,
            "io_timeout_secs" => config.io_timeout = seconds(key, value)?,
            "saves" => {
                config.saves = value.as_bool().ok_or_else(|| format!("`{key}` must be true or false"))?
            }
            "allow_clients" => {
                config.allow_clients =
                    value.as_bool().ok_or_else(|| format!("`{key}` must be true or false"))?
            }
            "connections" => {
                let table = value
                    .as_table()
                    .ok_or_else(|| "`connections` must be a table of name = \"mysql://...\"".to_string())?;
                for (name, url) in table {
                    config.connections.push((name.clone(), string(&format!("connections.{name}"), url)?));
                }
            }
            other => {
                return Err(format!(
                    "unknown key `{other}`; expected url, table, create_database, \
                     connect_timeout_secs, io_timeout_secs, saves, allow_clients or connections"
                ))
            }
        }
    }
    if !is_identifier(&config.table) {
        return Err(format!(
            "`table` is \"{}\"; use 1 to 64 letters, digits or _",
            config.table
        ));
    }
    Ok(config)
}

/// Find and read the config, in the order the module docs give.
///
/// A file that exists but cannot be read or parsed is an error, not a reason to
/// fall through to the next place: a developer who wrote a `mysql.toml` meant
/// it, and a game silently saving to a different server is the worse outcome.
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
/// A game's log is often pasted into a bug report.
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

/// A name safe to put into SQL between backticks: what MySQL calls an unquoted
/// identifier, minus `$`, and no longer than MySQL allows.
pub fn is_identifier(name: &str) -> bool {
    (1..=64).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn string(key: &str, value: &toml::Value) -> Result<String, String> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("`{key}` must be a string"))
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
            url = "mysql://game:pw@db.local:3307/mygame"
            table = "saves_v2"
            create_database = false
            connect_timeout_secs = 1
            io_timeout_secs = 0.5
            "#,
        )
        .unwrap();
        assert_eq!(c.url, "mysql://game:pw@db.local:3307/mygame");
        assert_eq!(c.table, "saves_v2");
        assert!(!c.create_database);
        assert_eq!(c.connect_timeout, Duration::from_secs(1));
        assert_eq!(c.io_timeout, Duration::from_millis(500));
    }

    #[test]
    fn named_connections_are_read_and_default_is_url() {
        let c = parse(
            r#"
            url = "mysql://root@127.0.0.1/main"
            saves = false
            [connections]
            stats = "mysql://reader@stats.local/stats"
            "#,
        )
        .unwrap();
        assert!(!c.saves);
        assert_eq!(c.connection_url("default"), Some("mysql://root@127.0.0.1/main"));
        assert_eq!(c.connection_url(""), Some("mysql://root@127.0.0.1/main"));
        assert_eq!(c.connection_url("stats"), Some("mysql://reader@stats.local/stats"));
        assert_eq!(c.connection_url("nope"), None);
    }

    #[test]
    fn a_misspelt_key_is_refused_by_name() {
        let e = parse(r#"tabel = "x""#).unwrap_err();
        assert!(e.contains("tabel"), "{e}");
    }

    #[test]
    fn a_table_name_that_could_carry_sql_is_refused() {
        for bad in ["", "saves; DROP TABLE x", "a`b", "a-b", &"t".repeat(65)] {
            assert!(parse(&format!("table = {bad:?}")).is_err(), "{bad:?} was accepted");
        }
    }

    #[test]
    fn credentials_never_reach_a_log_line() {
        assert_eq!(redacted("mysql://root:hunter2@127.0.0.1:3306/db"), "mysql://127.0.0.1:3306/db");
        assert_eq!(redacted("mysql://root@localhost/db"), "mysql://localhost/db");
        assert_eq!(redacted("mysql://localhost/db"), "mysql://localhost/db");
        // An `@` in the path is not userinfo.
        assert_eq!(redacted("mysql://localhost/a@b"), "mysql://localhost/a@b");
    }

    #[test]
    fn the_file_beside_the_executable_wins_over_the_projects() {
        let root = std::env::temp_dir().join(format!("mysql-saves-config-{}", std::process::id()));
        let (exe, project) = (root.join("exe"), root.join("project"));
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join(FILE_NAME), r#"table = "from_project""#).unwrap();

        let (c, source) = find(Some(&exe), Some(&project)).unwrap();
        assert_eq!(c.table, "from_project");
        assert_eq!(source, Source::File(project.join(FILE_NAME)));

        std::fs::write(exe.join(FILE_NAME), r#"table = "from_exe""#).unwrap();
        assert_eq!(find(Some(&exe), Some(&project)).unwrap().0.table, "from_exe");

        std::fs::write(exe.join(FILE_NAME), "not toml =").unwrap();
        assert!(find(Some(&exe), Some(&project)).is_err(), "a broken file fell through");
        let _ = std::fs::remove_dir_all(&root);
    }
}
