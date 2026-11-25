//! Running a script's MongoDB operations off the frame, one client per named
//! connection.
//!
//! A call is a network round trip to a server that may be slow or absent, so
//! it runs here and its answer comes back as a [`Done`] that the plugin turns
//! into a script event on a later frame.
//!
//! # One thread per operation
//!
//! Once connected, the driver puts no limit on how long an operation takes. A
//! fixed pool would let two hung operations stall every call after them, so
//! each operation gets its own thread, at most [`IN_FLIGHT`] at once. Reads also
//! carry a server-side `maxTimeMS`, and the plugin stops waiting for any answer
//! after `script::ANSWER_WITHIN`.
//!
//! # JSON in, JSON out
//!
//! Filters, updates, pipelines and documents arrive as JSON text (a script
//! argument is a number, a string or a vector, never a table) and are read as
//! MongoDB Extended JSON, so `{"_id": {"$oid": "..."}}` and
//! `{"at": {"$date": "..."}}` mean what they mean in the shell. Documents go
//! back the same way, as relaxed Extended JSON: plain numbers and strings,
//! with `$oid` / `$date` wrappers only where JSON has no type for the value.
//!
//! No Bevy, and this crate's own `serde_json` (the driver's): the engine's copy
//! is a separate crate, and the two meet only as text.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mongodb::bson::{doc, Bson, DateTime, Document};
use mongodb::options::ClientOptions;
use mongodb::sync::Client;
use serde_json::Value;

use crate::config::{self, Config};

/// How many operations may run at once. A game sends a handful at a time;
/// past this, something is calling every frame, and a new call fails at once
/// rather than piling up threads.
pub const IN_FLIGHT: usize = 32;

/// The server-side limit on a read (`maxTimeMS`), under the plugin's own
/// wait so the server's reason arrives instead of a bare timeout.
const READ_LIMIT: Duration = Duration::from_secs(25);

/// What to do.
#[derive(Debug, Clone)]
pub enum Op {
    /// `options`: `{"sort": {...}, "limit": n, "skip": n, "projection": {...}}`.
    Find { collection: String, filter: String, options: String },
    /// One document (an object) or several (an array).
    Insert { collection: String, documents: String },
    Update { collection: String, filter: String, update: String, many: bool, upsert: bool },
    Delete { collection: String, filter: String, many: bool },
    Aggregate { collection: String, pipeline: String },
    Count { collection: String, filter: String },
    /// Any database command, as `db.runCommand` takes it: the way to anything
    /// the other operations do not cover (indexes, collection stats, admin).
    Command { command: String },
    /// Store an entity tree (the engine's JSON) under `_id = key`.
    SaveTree { collection: String, key: String, tree: String },
    /// Fetch the tree stored under `_id = key`.
    LoadTree { collection: String, key: String },
}

#[derive(Debug, Clone)]
pub struct Job {
    /// Chosen by the caller, echoed in [`Done`].
    pub id: u64,
    pub connection: String,
    /// Empty means the config's `database`.
    pub database: String,
    pub op: Op,
}

/// An operation's outcome. Text, not values, so it crosses into the engine's
/// JSON without either crate's types leaking into the other.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// JSON array of the documents returned (find, aggregate, command, load).
    pub docs: String,
    pub count: i64,
    /// Documents inserted, modified or deleted.
    pub affected: i64,
    /// JSON array of the new documents' `_id`s.
    pub inserted_ids: String,
}

#[derive(Debug, Clone)]
pub struct Done {
    pub id: u64,
    pub result: Result<Outcome, String>,
}

type Clients = Arc<Mutex<HashMap<String, (String, Client)>>>;

/// Runs operations on their own threads. Cloning shares it.
#[derive(Clone)]
pub struct Workers {
    done: Arc<Mutex<Vec<Done>>>,
    config: Arc<Mutex<Config>>,
    clients: Clients,
    running: Arc<AtomicUsize>,
}

impl Workers {
    pub fn start(config: Config) -> Self {
        Self {
            done: Arc::default(),
            config: Arc::new(Mutex::new(config)),
            clients: Arc::default(),
            running: Arc::default(),
        }
    }

    pub fn set_config(&self, config: Config) {
        if let Ok(mut c) = self.config.lock() {
            *c = config;
        }
    }

    pub fn submit(&self, job: Job) {
        let id = job.id;
        if self.running.fetch_add(1, Ordering::SeqCst) >= IN_FLIGHT {
            self.running.fetch_sub(1, Ordering::SeqCst);
            let why = format!("{IN_FLIGHT} MongoDB calls are already waiting for an answer");
            return finish(&self.done, id, Err(why));
        }
        let (done, config, clients, running) =
            (self.done.clone(), self.config.clone(), self.clients.clone(), self.running.clone());
        let spawned = std::thread::Builder::new().name(format!("mongodb-query-{id}")).spawn(move || {
            let config = config.lock().map(|c| c.clone()).unwrap_or_default();
            let result = run(&job, &config, &clients);
            running.fetch_sub(1, Ordering::SeqCst);
            finish(&done, id, result);
        });
        if let Err(e) = spawned {
            self.running.fetch_sub(1, Ordering::SeqCst);
            finish(&self.done, id, Err(format!("could not start a MongoDB thread: {e}")));
        }
    }

    pub fn take_done(&self) -> Vec<Done> {
        self.done.lock().map(|mut d| std::mem::take(&mut *d)).unwrap_or_default()
    }
}

fn finish(done: &Mutex<Vec<Done>>, id: u64, result: Result<Outcome, String>) {
    if let Ok(mut d) = done.lock() {
        d.push(Done { id, result });
    }
}

/// A client for `url`, configured to give up after the config's timeout
/// instead of the driver's 30 seconds.
pub fn connect(url: &str, config: &Config) -> Result<Client, String> {
    let shown = config::redacted(url);
    let mut opts = ClientOptions::parse(url)
        .run()
        .map_err(|e| format!("{shown} is not a MongoDB URL: {e}"))?;
    opts.server_selection_timeout = Some(config.connect_timeout);
    opts.connect_timeout = Some(config.connect_timeout);
    opts.app_name = Some("renzora".into());
    Client::with_options(opts).map_err(|e| format!("could not set up a client for {shown}: {e}"))
}

fn client_for(name: &str, config: &Config, clients: &Clients) -> Result<Client, String> {
    let url = config
        .connection_url(name)
        .ok_or_else(|| format!("no connection named `{name}` in mongodb.toml"))?
        .to_string();
    let mut clients = clients.lock().map_err(|_| "the client list is poisoned".to_string())?;
    if let Some((u, c)) = clients.get(name) {
        if *u == url {
            return Ok(c.clone());
        }
    }
    let client = connect(&url, config)?;
    clients.insert(name.to_string(), (url, client.clone()));
    Ok(client)
}

fn run(job: &Job, config: &Config, clients: &Clients) -> Result<Outcome, String> {
    let client = client_for(&job.connection, config, clients)?;
    let db_name = if job.database.is_empty() { config.database.as_str() } else { job.database.as_str() };
    let db = client.database(db_name);
    let fail = |e: mongodb::error::Error| e.to_string();
    let coll = |name: &str| -> Result<mongodb::sync::Collection<Document>, String> {
        if name.trim().is_empty() {
            return Err("no collection was named".into());
        }
        Ok(db.collection::<Document>(name))
    };

    match &job.op {
        Op::Find { collection, filter, options } => {
            let opts = document(options)?;
            // Bound first: the builder borrows the collection until `run`.
            let c = coll(collection)?;
            let mut find = c.find(document(filter)?).max_time(READ_LIMIT);
            if let Ok(sort) = opts.get_document("sort") {
                find = find.sort(sort.clone());
            }
            if let Ok(projection) = opts.get_document("projection") {
                find = find.projection(projection.clone());
            }
            if let Some(limit) = number(&opts, "limit") {
                find = find.limit(limit);
            }
            if let Some(skip) = number(&opts, "skip") {
                find = find.skip(skip.max(0) as u64);
            }
            let docs: Vec<Document> = find.run().map_err(fail)?.collect::<Result<_, _>>().map_err(fail)?;
            Ok(listed(docs))
        }
        Op::Insert { collection, documents } => {
            let docs = documents_of(documents)?;
            if docs.is_empty() {
                return Err("no documents to insert".into());
            }
            let result = coll(collection)?.insert_many(docs).run().map_err(fail)?;
            let mut ids: Vec<(usize, Bson)> = result.inserted_ids.into_iter().collect();
            ids.sort_by_key(|(i, _)| *i);
            let ids: Vec<Value> = ids.into_iter().map(|(_, id)| id.into_relaxed_extjson()).collect();
            Ok(Outcome {
                docs: "[]".into(),
                count: 0,
                affected: ids.len() as i64,
                inserted_ids: Value::Array(ids).to_string(),
            })
        }
        Op::Update { collection, filter, update, many, upsert } => {
            let c = coll(collection)?;
            let (filter, update) = (document(filter)?, update_of(update)?);
            let result = match (many, update) {
                (true, Update::Doc(u)) => c.update_many(filter, u).upsert(*upsert).run(),
                (true, Update::Pipeline(p)) => c.update_many(filter, p).upsert(*upsert).run(),
                (false, Update::Doc(u)) => c.update_one(filter, u).upsert(*upsert).run(),
                (false, Update::Pipeline(p)) => c.update_one(filter, p).upsert(*upsert).run(),
            }
            .map_err(fail)?;
            let ids: Vec<Value> = result.upserted_id.into_iter().map(Bson::into_relaxed_extjson).collect();
            Ok(Outcome {
                docs: "[]".into(),
                count: result.matched_count as i64,
                affected: result.modified_count as i64 + ids.len() as i64,
                inserted_ids: Value::Array(ids).to_string(),
            })
        }
        Op::Delete { collection, filter, many } => {
            let c = coll(collection)?;
            let filter = document(filter)?;
            let result = if *many { c.delete_many(filter).run() } else { c.delete_one(filter).run() }.map_err(fail)?;
            Ok(Outcome { docs: "[]".into(), affected: result.deleted_count as i64, inserted_ids: "[]".into(), ..Default::default() })
        }
        Op::Aggregate { collection, pipeline } => {
            let stages = documents_of(pipeline)?;
            let docs: Vec<Document> =
                coll(collection)?.aggregate(stages).max_time(READ_LIMIT).run().map_err(fail)?.collect::<Result<_, _>>().map_err(fail)?;
            Ok(listed(docs))
        }
        Op::Count { collection, filter } => {
            let n = coll(collection)?.count_documents(document(filter)?).max_time(READ_LIMIT).run().map_err(fail)?;
            Ok(Outcome { docs: "[]".into(), count: n as i64, inserted_ids: "[]".into(), ..Default::default() })
        }
        Op::Command { command } => {
            let reply = db.run_command(document(command)?).run().map_err(fail)?;
            // A command that returns a cursor (`find`, `aggregate`) has its
            // documents in `cursor.firstBatch`; list those, not the envelope.
            let batch = reply
                .get_document("cursor")
                .ok()
                .and_then(|c| c.get_array("firstBatch").ok())
                .map(|a| a.iter().filter_map(|b| b.as_document().cloned()).collect::<Vec<_>>());
            Ok(listed(batch.unwrap_or_else(|| vec![reply])))
        }
        Op::SaveTree { collection, key, tree } => {
            let tree = document(tree)?;
            let entry = doc! { "_id": key.as_str(), "tree": tree, "saved_at": DateTime::now() };
            coll(collection)?
                .replace_one(doc! { "_id": key.as_str() }, entry)
                .upsert(true)
                .run()
                .map_err(fail)?;
            Ok(Outcome { docs: "[]".into(), affected: 1, inserted_ids: "[]".into(), ..Default::default() })
        }
        Op::LoadTree { collection, key } => {
            let found = coll(collection)?.find_one(doc! { "_id": key.as_str() }).run().map_err(fail)?;
            let tree = found
                .and_then(|d| d.get_document("tree").ok().cloned())
                .ok_or_else(|| format!("no entity is saved under \"{key}\""))?;
            Ok(listed(vec![tree]))
        }
    }
}

fn listed(docs: Vec<Document>) -> Outcome {
    let count = docs.len() as i64;
    let values: Vec<Value> = docs.into_iter().map(|d| Bson::Document(d).into_relaxed_extjson()).collect();
    Outcome { docs: Value::Array(values).to_string(), count, affected: 0, inserted_ids: "[]".into() }
}

fn number(d: &Document, key: &str) -> Option<i64> {
    match d.get(key)? {
        Bson::Int32(n) => Some(*n as i64),
        Bson::Int64(n) => Some(*n),
        Bson::Double(f) => Some(*f as i64),
        _ => None,
    }
}

/// Extended JSON text to BSON. Empty text is nothing, which callers read as
/// an empty filter: every document.
fn bson_of(text: &str) -> Result<Option<Bson>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    Bson::try_from(value).map(Some).map_err(|e| format!("not Extended JSON: {e}"))
}

/// One document: a JSON object, or empty for `{}`.
pub fn document(text: &str) -> Result<Document, String> {
    match bson_of(text)? {
        None => Ok(Document::new()),
        Some(Bson::Document(d)) => Ok(d),
        Some(_) => Err(format!("expected a JSON object, got `{}`", text.trim())),
    }
}

/// One document or an array of them.
fn documents_of(text: &str) -> Result<Vec<Document>, String> {
    match bson_of(text)? {
        None => Ok(Vec::new()),
        Some(Bson::Document(d)) => Ok(vec![d]),
        Some(Bson::Array(items)) => items
            .into_iter()
            .map(|b| match b {
                Bson::Document(d) => Ok(d),
                other => Err(format!("expected objects in the array, got {other}")),
            })
            .collect(),
        Some(other) => Err(format!("expected an object or an array of objects, got {other}")),
    }
}

/// An update is a document of operators (`{"$set": ...}`) or an aggregation
/// pipeline (an array of stages); the driver takes either.
enum Update {
    Doc(Document),
    Pipeline(Vec<Document>),
}

fn update_of(text: &str) -> Result<Update, String> {
    match bson_of(text)? {
        Some(Bson::Document(d)) => Ok(Update::Doc(d)),
        Some(Bson::Array(_)) => documents_of(text).map(Update::Pipeline),
        _ => Err("an update is an object like {\"$set\": {...}} or a pipeline array".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_json_means_what_it_does_in_the_shell() {
        let d = document(r#"{"_id": {"$oid": "65a1f0c2e4b0a1b2c3d4e5f6"}, "hp": {"$gt": 3}}"#).unwrap();
        assert!(matches!(d.get("_id"), Some(Bson::ObjectId(_))));
        assert!(document("").unwrap().is_empty());
        assert!(document("[1]").is_err());
    }

    #[test]
    fn one_or_many_documents() {
        assert_eq!(documents_of(r#"{"a": 1}"#).unwrap().len(), 1);
        assert_eq!(documents_of(r#"[{"a": 1}, {"a": 2}]"#).unwrap().len(), 2);
        assert!(documents_of("[1, 2]").is_err());
    }
}
