//! Save slots in a MongoDB collection: one document per game and slot,
//! `{ game, slot, data, saved_at }`, unique on `(game, slot)`.
//!
//! `data` is the same text a slot file holds, so the engine's version check
//! and corruption refusal still apply, and a slot copied out of `data` is a
//! valid `<slot>.json`.
//!
//! # A server that is down costs one stall, not one per call
//!
//! A save call holds the frame, and a load menu may ask `save_exists` for every
//! slot every frame. The first call to find the server gone waits out the
//! connect timeout; from then on every call fails at once with that reason,
//! while a background thread checks every [`RETRY_AFTER`] and puts the
//! connection back when the server answers. Waiting on the frame for each
//! retry instead froze the game for the whole timeout every few seconds.
//!
//! Only an unreachable server counts as down. A refusal (a duplicate key, a
//! permission) is that call's answer and changes nothing for the next one.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use mongodb::bson::{doc, DateTime, Document};
use mongodb::error::ErrorKind;
use mongodb::options::IndexOptions;
use mongodb::sync::Collection;
use mongodb::IndexModel;

use crate::config::{self, Config};

pub const RETRY_AFTER: Duration = Duration::from_secs(5);

/// The server-side limit on a save read, so a slow server cannot hold the
/// frame for long once connected.
const READ_LIMIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq)]
pub enum DbError {
    /// No game name yet: the project has not loaded.
    NoGame,
    Failed(String),
}

pub struct Database {
    state: Arc<Mutex<State>>,
    log: fn(&str),
}

struct State {
    config: Config,
    game: Option<String>,
    collection: Option<Collection<Document>>,
    /// Why the server is unreachable, while it is.
    down: Option<String>,
    /// Whether the background check is running.
    probing: bool,
}

impl Database {
    pub fn new(config: Config, log: fn(&str)) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                config,
                game: None,
                collection: None,
                down: None,
                probing: false,
            })),
            log,
        }
    }

    pub fn set_game(&self, game: &str) {
        lock(&self.state).game = Some(game.to_string());
    }

    /// Replace the config. A changed one is tried afresh on the next call.
    pub fn set_config(&self, config: Config) {
        let mut s = lock(&self.state);
        if s.config != config {
            s.config = config;
            s.collection = None;
            s.down = None;
        }
    }

    pub fn write(&self, slot: &str, text: &str) -> Result<(), DbError> {
        self.run(|c, game| {
            c.replace_one(
                doc! { "game": game, "slot": slot },
                doc! { "game": game, "slot": slot, "data": text, "saved_at": DateTime::now() },
            )
            .upsert(true)
            .run()
            .map(|_| ())
        })
    }

    pub fn read(&self, slot: &str) -> Result<Option<String>, DbError> {
        self.run(|c, game| {
            c.find_one(doc! { "game": game, "slot": slot })
                .max_time(READ_LIMIT)
                .run()
                .map(|d| d.and_then(|d| d.get_str("data").ok().map(str::to_string)))
        })
    }

    pub fn exists(&self, slot: &str) -> Result<bool, DbError> {
        self.run(|c, game| {
            c.count_documents(doc! { "game": game, "slot": slot }).limit(1).max_time(READ_LIMIT).run().map(|n| n > 0)
        })
    }

    pub fn delete(&self, slot: &str) -> Result<(), DbError> {
        self.run(|c, game| c.delete_one(doc! { "game": game, "slot": slot }).run().map(|_| ()))
    }

    pub fn slots(&self) -> Result<Vec<String>, DbError> {
        self.run(|c, game| {
            c.distinct("slot", doc! { "game": game })
                .max_time(READ_LIMIT)
                .run()
                .map(|v| v.into_iter().filter_map(|b| b.as_str().map(str::to_string)).collect())
        })
    }

    fn run<T>(
        &self,
        f: impl Fn(&Collection<Document>, &str) -> mongodb::error::Result<T>,
    ) -> Result<T, DbError> {
        let mut s = lock(&self.state);
        let game = s.game.clone().ok_or(DbError::NoGame)?;
        let collection = self.collection(&mut s)?;
        f(&collection, &game).map_err(|e| {
            let why = e.to_string();
            if unreachable(&e) {
                let down = format!("MongoDB at {} stopped answering: {why}", config::redacted(&s.config.url));
                self.went_down(&mut s, down);
            }
            DbError::Failed(why)
        })
    }

    fn collection(&self, s: &mut State) -> Result<Collection<Document>, DbError> {
        if let Some(why) = &s.down {
            return Err(DbError::Failed(why.clone()));
        }
        if let Some(c) = &s.collection {
            return Ok(c.clone());
        }
        match open(&s.config) {
            Ok(c) => {
                (self.log)(&format!("saves in {}", place(&s.config)));
                s.collection = Some(c.clone());
                Ok(c)
            }
            Err(why) => {
                self.went_down(s, why.clone());
                Err(DbError::Failed(why))
            }
        }
    }

    /// Mark the server down, say so once, and start the background check.
    fn went_down(&self, s: &mut State, why: String) {
        s.collection = None;
        if s.down.is_none() {
            (self.log)(&format!(
                "{why}. Saving and loading fail at once until it answers; checking every {}s",
                RETRY_AFTER.as_secs()
            ));
        }
        s.down = Some(why);
        if s.probing {
            return;
        }
        s.probing = true;
        let (state, log) = (self.state.clone(), self.log);
        let started = std::thread::Builder::new().name("mongodb-saves-reconnect".into()).spawn(move || probe(state, log));
        if started.is_err() {
            // No thread, so no recovery in the background: let the next call
            // try for itself instead of failing until the game restarts.
            s.probing = false;
            s.down = None;
        }
    }
}

/// Check every [`RETRY_AFTER`] until the server answers, connecting without
/// holding the lock so save calls keep failing fast meanwhile.
fn probe(state: Arc<Mutex<State>>, log: fn(&str)) {
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
        let Ok(c) = open(&config) else { continue };
        let mut s = lock(&state);
        // A config that changed meanwhile was cleared by `set_config`; its
        // first call connects for itself.
        if s.down.is_some() && s.config == config {
            log(&format!("MongoDB answers again; saves in {}", place(&config)));
            s.collection = Some(c);
            s.down = None;
        }
        if s.down.is_none() {
            s.probing = false;
            return;
        }
    }
}

/// A client and the slot collection, with its index. Creating the index is
/// also the first round trip, so a server that is not there fails here.
fn open(config: &Config) -> Result<Collection<Document>, String> {
    let client = crate::query::connect(&config.url, config)?;
    let c = client.database(&config.database).collection::<Document>(&config.collection);
    let index = IndexModel::builder()
        .keys(doc! { "game": 1, "slot": 1 })
        .options(IndexOptions::builder().unique(true).build())
        .build();
    c.create_index(index)
        .run()
        .map_err(|e| format!("could not reach MongoDB at {}: {e}", config::redacted(&config.url)))?;
    Ok(c)
}

/// Whether an error means the server cannot be reached, as opposed to the
/// server refusing this one call.
fn unreachable(e: &mongodb::error::Error) -> bool {
    matches!(
        *e.kind,
        ErrorKind::ServerSelection { .. } | ErrorKind::Io(_) | ErrorKind::ConnectionPoolCleared { .. }
    )
}

fn place(config: &Config) -> String {
    format!("{} / {}.{}", config::redacted(&config.url), config.database, config.collection)
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}
