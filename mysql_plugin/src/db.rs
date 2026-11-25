//! The MySQL side: one connection, one table, four statements.
//!
//! # One connection, opened on first use
//!
//! A game saves a few times a session, so a pool would be idle connections the
//! server has to keep. And the connection is opened by the first save call
//! rather than at startup, so a game that never saves, or an editor session in
//! which nothing is played, never connects at all, and a server that is not
//! running yet when the game starts is found once it is.
//!
//! # Reconnecting
//!
//! MySQL closes a connection that has been idle for `wait_timeout` (8 hours by
//! default), and a player who leaves the game paused overnight comes back to a
//! connection that is gone. So a call that fails on a connection that was
//! already open, for any reason other than the server refusing the statement,
//! drops it and is tried once more on a fresh one. A connection made by the
//! call itself is not retried: it just failed, and failing twice tells nobody
//! anything new.
//!
//! # A server that is down costs one stall, not one per call
//!
//! A save call holds the frame, and a load menu may ask `save_exists` for every
//! slot every frame. The first call to find the server gone waits out the
//! connect timeout; from then on every call fails at once with that reason,
//! while a background thread tries every [`RETRY_AFTER`] and hands the new
//! connection over when the server answers. Retrying on the frame instead
//! froze the game for the whole timeout every few seconds.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use mysql::prelude::Queryable;
use mysql::{Conn, Opts, OptsBuilder};

use crate::config::{self, Config};

pub const RETRY_AFTER: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq)]
pub enum DbError {
    /// No game name yet, so there is no telling whose slots to touch. The
    /// project has not loaded.
    NoGame,
    Failed(String),
}

/// Something worth a line in the log. The database does not log by itself,
/// since it is plain Rust with no Bevy; whoever builds it says where these go.
#[derive(Debug, Clone, PartialEq)]
pub enum Note {
    Connected { url: String, table: String },
    /// Reported once per outage, not once per failed call.
    Unreachable(String),
    /// The background check got through after an outage.
    Reconnected { url: String, table: String },
}

pub struct Database {
    state: Arc<Mutex<State>>,
    log: fn(Note),
}

struct State {
    config: Config,
    game: Option<String>,
    conn: Option<Conn>,
    /// Why the server is unreachable, while it is. Calls fail with this at once.
    down: Option<String>,
    /// Whether the background check is running.
    probing: bool,
}

impl Database {
    pub fn new(config: Config, log: fn(Note)) -> Self {
        Self {
            state: Arc::new(Mutex::new(State { config, game: None, conn: None, down: None, probing: false })),
            log,
        }
    }

    /// Whose slots to use. Slots are kept per game in one table, so two games
    /// on one machine can share a database without seeing each other's saves.
    pub fn set_game(&self, game: &str) {
        self.lock().game = Some(game.to_string());
    }

    /// Replace the config. The open connection, if any, is dropped when the
    /// config actually changed, so the next call connects where it now says.
    pub fn set_config(&self, config: Config) {
        let mut state = self.lock();
        if state.config != config {
            state.config = config;
            state.conn = None;
            state.down = None;
        }
    }

    pub fn write(&self, slot: &str, text: &str) -> Result<(), DbError> {
        self.run(|conn, table, game| {
            // One statement, so it is atomic on its own: the old save stays
            // until the new one is wholly in. `VALUES()` rather than MySQL 8's
            // row alias, which MariaDB does not accept.
            conn.exec_drop(
                format!(
                    "INSERT INTO `{table}` (game, slot, data) VALUES (?, ?, ?) \
                     ON DUPLICATE KEY UPDATE data = VALUES(data), saved_at = CURRENT_TIMESTAMP"
                ),
                (game, slot, text),
            )
        })
    }

    pub fn read(&self, slot: &str) -> Result<Option<String>, DbError> {
        self.run(|conn, table, game| {
            conn.exec_first::<String, _, _>(
                format!("SELECT data FROM `{table}` WHERE game = ? AND slot = ?"),
                (game, slot),
            )
        })
    }

    pub fn exists(&self, slot: &str) -> Result<bool, DbError> {
        self.run(|conn, table, game| {
            conn.exec_first::<u8, _, _>(
                format!("SELECT 1 FROM `{table}` WHERE game = ? AND slot = ?"),
                (game, slot),
            )
            .map(|row| row.is_some())
        })
    }

    pub fn delete(&self, slot: &str) -> Result<(), DbError> {
        self.run(|conn, table, game| {
            conn.exec_drop(
                format!("DELETE FROM `{table}` WHERE game = ? AND slot = ?"),
                (game, slot),
            )
        })
    }

    pub fn slots(&self) -> Result<Vec<String>, DbError> {
        self.run(|conn, table, game| {
            conn.exec::<String, _, _>(format!("SELECT slot FROM `{table}` WHERE game = ?"), (game,))
        })
    }

    /// Run `f` on a live connection, connecting or reconnecting as the module
    /// docs describe.
    fn run<T>(&self, f: impl Fn(&mut Conn, &str, &str) -> mysql::Result<T>) -> Result<T, DbError> {
        let mut state = self.lock();
        let game = state.game.clone().ok_or(DbError::NoGame)?;
        let table = state.config.table.clone();

        let reused = state.conn.is_some();
        let conn = self.connection(&mut state)?;
        match f(conn, &table, &game) {
            Ok(value) => Ok(value),
            // The server read the statement and said no. The connection is
            // fine, and running it again would get the same answer.
            Err(e @ mysql::Error::MySqlError(_)) => Err(DbError::Failed(e.to_string())),
            Err(e) => {
                state.conn = None;
                if !reused {
                    return Err(DbError::Failed(e.to_string()));
                }
                let conn = self.connection(&mut state)?;
                f(conn, &table, &game).map_err(|e| {
                    state.conn = None;
                    DbError::Failed(e.to_string())
                })
            }
        }
    }

    /// The open connection, or a new one.
    fn connection<'s>(&self, state: &'s mut State) -> Result<&'s mut Conn, DbError> {
        if state.conn.is_none() {
            if let Some(why) = &state.down {
                return Err(DbError::Failed(why.clone()));
            }
            match connect(&state.config) {
                Ok(conn) => {
                    state.conn = Some(conn);
                    (self.log)(Note::Connected {
                        url: config::redacted(&state.config.url),
                        table: state.config.table.clone(),
                    });
                }
                Err(why) => {
                    self.went_down(state, why.clone());
                    return Err(DbError::Failed(why));
                }
            }
        }
        state.conn.as_mut().ok_or_else(|| DbError::Failed("no connection".into()))
    }

    /// Mark the server down, say so once, and start the background check.
    fn went_down(&self, state: &mut State, why: String) {
        if state.down.is_none() {
            (self.log)(Note::Unreachable(why.clone()));
        }
        state.down = Some(why);
        if state.probing {
            return;
        }
        state.probing = true;
        let (shared, log) = (self.state.clone(), self.log);
        let started = std::thread::Builder::new().name("mysql-saves-reconnect".into()).spawn(move || probe(shared, log));
        if started.is_err() {
            // No thread, so no recovery in the background: let the next call
            // try for itself instead of failing until the game restarts.
            state.probing = false;
            state.down = None;
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Try every [`RETRY_AFTER`] until the server answers, connecting without
/// holding the lock so save calls keep failing fast meanwhile.
fn probe(state: Arc<Mutex<State>>, log: fn(Note)) {
    loop {
        std::thread::sleep(RETRY_AFTER);
        let config = {
            let mut s = lock(&state);
            if s.down.is_none() {
                s.probing = false;
                return;
            }
            s.config.clone()
        };
        let Ok(conn) = connect(&config) else { continue };
        let mut s = lock(&state);
        // A config that changed meanwhile was cleared by `set_config`; its
        // first call connects for itself.
        if s.down.is_some() && s.config == config {
            log(Note::Reconnected { url: config::redacted(&config.url), table: config.table.clone() });
            s.conn = Some(conn);
            s.down = None;
        }
        if s.down.is_none() {
            s.probing = false;
            return;
        }
    }
}

/// Open a connection and make sure the database and the table are there.
///
/// Errors are sentences that name the server (without its password), since
/// they reach a script's `save_write` result and the log as they are.
fn connect(config: &Config) -> Result<Conn, String> {
    let shown = config::redacted(&config.url);
    let opts = Opts::from_url(&config.url).map_err(|e| format!("{shown} is not a MySQL URL: {e}"))?;
    let database = opts
        .get_db_name()
        .map(str::to_string)
        .ok_or_else(|| format!("{shown} names no database; add one, as in mysql://root@127.0.0.1:3306/my_game"))?;
    if !config::is_identifier(&database) {
        return Err(format!(
            "the database \"{database}\" in {shown} is not a plain name; use letters, digits or _"
        ));
    }

    let builder = OptsBuilder::from_opts(opts)
        .tcp_connect_timeout(Some(config.connect_timeout))
        .read_timeout(Some(config.io_timeout))
        .write_timeout(Some(config.io_timeout));

    let unreachable = |e: mysql::Error| format!("could not connect to MySQL at {shown}: {e}");
    let mut conn = if config.create_database {
        // Connect with no database first: naming one that does not exist yet
        // fails the handshake, which is the very case this is for.
        let mut conn = Conn::new(builder.db_name(None::<String>)).map_err(unreachable)?;
        conn.query_drop(format!(
            "CREATE DATABASE IF NOT EXISTS `{database}` CHARACTER SET utf8mb4"
        ))
        .map_err(|e| format!("could not create the database `{database}` at {shown}: {e}"))?;
        conn.query_drop(format!("USE `{database}`"))
            .map_err(|e| format!("could not open the database `{database}` at {shown}: {e}"))?;
        conn
    } else {
        Conn::new(builder).map_err(unreachable)?
    };

    // `slot` is ASCII and compared exactly, since slot names are already
    // limited to ASCII letters, digits, `-` and `_`, and a case-insensitive
    // collation would make `Slot1` overwrite `slot1`. 191 characters of game
    // name keeps the key inside the index limit of an old utf8mb4 server.
    conn.query_drop(format!(
        "CREATE TABLE IF NOT EXISTS `{table}` (\
            game VARCHAR(191) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL, \
            slot VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, \
            data LONGTEXT CHARACTER SET utf8mb4 NOT NULL, \
            saved_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP, \
            PRIMARY KEY (game, slot)\
        ) CHARACTER SET utf8mb4",
        table = config.table
    ))
    .map_err(|e| format!("could not create the table `{}` at {shown}: {e}", config.table))?;
    Ok(conn)
}
