//! Running a script's SQL: off the game's frame, on worker threads, one pool
//! per named connection.
//!
//! # Why threads
//!
//! A query is a network round trip, and the server may be slow or not there.
//! Run on the frame, a 2-second connect timeout is a 2-second freeze. So a
//! query is handed to a worker, the frame carries on, and the answer comes back
//! as a [`Done`] that the plugin turns into a script event on a later frame.
//!
//! # The answer's shape
//!
//! Rows travel as JSON text: an array with one object per row, column name to
//! value. A script argument holds a number, a string or a vector, not a table,
//! so JSON is how a list of rows fits into one, and both languages read it
//! (`json_parse` in Lua, `renzora::serde_json` in Rust).
//!
//! No Bevy, like `db.rs`.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use mysql::prelude::Queryable;
use mysql::{Opts, OptsBuilder, Pool, PoolConstraints, PoolOpts, TxOpts, Value};
// The engine's copy: a plugin's own would be a second serde_json with
// separate types.
use renzora::serde_json;

use crate::config::{self, Config};

/// How many queries may run at once. A game sends a handful at a time; more
/// workers would only be more idle connections on the server.
const WORKERS: usize = 2;

/// One SQL statement with its `?` parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub sql: String,
    pub params: Vec<serde_json::Value>,
}

/// Work for a worker.
#[derive(Debug, Clone)]
pub struct Job {
    /// Chosen by the caller, echoed in [`Done`].
    pub id: u64,
    pub connection: String,
    pub statements: Vec<Statement>,
    /// Run all statements in one transaction: all apply, or none do.
    pub transaction: bool,
}

/// One statement's outcome.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outcome {
    /// One JSON object per row, column name to value.
    pub rows: Vec<serde_json::Map<String, serde_json::Value>>,
    pub affected: u64,
    pub insert_id: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Done {
    pub id: u64,
    /// Every statement's outcome, in order, or why the job failed.
    pub result: Result<Vec<Outcome>, String>,
}

/// The worker pool. Cloning shares it.
#[derive(Clone)]
pub struct Workers {
    jobs: Sender<Job>,
    done: Arc<Mutex<Vec<Done>>>,
    config: Arc<Mutex<Config>>,
}

impl Workers {
    pub fn start(config: Config) -> Self {
        let (jobs, rx) = channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let done = Arc::new(Mutex::new(Vec::new()));
        let config = Arc::new(Mutex::new(config));
        let pools = Arc::new(Mutex::new(HashMap::<String, (String, Pool)>::new()));
        for i in 0..WORKERS {
            let (rx, done, config, pools) = (rx.clone(), done.clone(), config.clone(), pools.clone());
            let _ = std::thread::Builder::new()
                .name(format!("mysql-query-{i}"))
                .spawn(move || worker(rx, done, config, pools));
        }
        Self { jobs, done, config }
    }

    pub fn set_config(&self, config: Config) {
        if let Ok(mut c) = self.config.lock() {
            *c = config;
        }
    }

    pub fn submit(&self, job: Job) {
        let id = job.id;
        if self.jobs.send(job).is_err() {
            self.finish(Done { id, result: Err("the MySQL workers have stopped".into()) });
        }
    }

    fn finish(&self, done: Done) {
        if let Ok(mut d) = self.done.lock() {
            d.push(done);
        }
    }

    /// Everything finished since the last call.
    pub fn take_done(&self) -> Vec<Done> {
        self.done.lock().map(|mut d| std::mem::take(&mut *d)).unwrap_or_default()
    }
}

fn worker(
    rx: Arc<Mutex<Receiver<Job>>>,
    done: Arc<Mutex<Vec<Done>>>,
    config: Arc<Mutex<Config>>,
    pools: Arc<Mutex<HashMap<String, (String, Pool)>>>,
) {
    loop {
        // Hold the receiver only to take one job, so the other worker can take
        // the next while this one runs.
        let job = match rx.lock() {
            Ok(rx) => match rx.recv() {
                Ok(job) => job,
                Err(_) => return,
            },
            Err(_) => return,
        };
        let config = config.lock().map(|c| c.clone()).unwrap_or_default();
        let result = run(&job, &config, &pools);
        if let Ok(mut d) = done.lock() {
            d.push(Done { id: job.id, result });
        }
    }
}

fn run(
    job: &Job,
    config: &Config,
    pools: &Mutex<HashMap<String, (String, Pool)>>,
) -> Result<Vec<Outcome>, String> {
    let url = config
        .connection_url(&job.connection)
        .ok_or_else(|| format!("no connection named `{}` in mysql.toml", job.connection))?
        .to_string();
    let pool = pool_for(&job.connection, &url, config, pools)?;
    let shown = config::redacted(&url);
    let mut conn = pool.get_conn().map_err(|e| format!("could not connect to MySQL at {shown}: {e}"))?;

    if job.transaction {
        let mut tx = conn.start_transaction(TxOpts::default()).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for s in &job.statements {
            // An error drops `tx` without a commit, which rolls it back.
            out.push(execute(&mut tx, s)?);
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(out)
    } else {
        job.statements.iter().map(|s| execute(&mut conn, s)).collect()
    }
}

/// The pool for a connection, made on first use. Remade when the URL behind
/// the name changes (a reloaded `mysql.toml`).
fn pool_for(
    name: &str,
    url: &str,
    config: &Config,
    pools: &Mutex<HashMap<String, (String, Pool)>>,
) -> Result<Pool, String> {
    let mut pools = pools.lock().map_err(|_| "the connection list is poisoned".to_string())?;
    if let Some((u, pool)) = pools.get(name) {
        if u == url {
            return Ok(pool.clone());
        }
    }
    let shown = config::redacted(url);
    let opts = Opts::from_url(url).map_err(|e| format!("{shown} is not a MySQL URL: {e}"))?;
    if config.create_database {
        ensure_database(&opts, config, &shown)?;
    }
    // Nothing opened up front, at most four: a pool's default keeps ten
    // connections open whether the game queries or not.
    let constraints = PoolConstraints::new(0, 4).unwrap_or_default();
    let builder = OptsBuilder::from_opts(opts)
        .tcp_connect_timeout(Some(config.connect_timeout))
        .read_timeout(Some(config.io_timeout))
        .write_timeout(Some(config.io_timeout))
        .pool_opts(PoolOpts::default().with_constraints(constraints));
    let pool = Pool::new(builder).map_err(|e| format!("could not connect to MySQL at {shown}: {e}"))?;
    pools.insert(name.to_string(), (url.to_string(), pool.clone()));
    Ok(pool)
}

/// Create the URL's database if it is missing, so a fresh server works on the
/// first query, as the save backend already does.
fn ensure_database(opts: &Opts, config: &Config, shown: &str) -> Result<(), String> {
    let Some(db) = opts.get_db_name().map(str::to_string) else { return Ok(()) };
    if !config::is_identifier(&db) {
        return Err(format!("the database \"{db}\" in {shown} is not a plain name"));
    }
    let builder = OptsBuilder::from_opts(opts.clone())
        .db_name(None::<String>)
        .tcp_connect_timeout(Some(config.connect_timeout));
    let mut conn = mysql::Conn::new(builder).map_err(|e| format!("could not connect to MySQL at {shown}: {e}"))?;
    conn.query_drop(format!("CREATE DATABASE IF NOT EXISTS `{db}` CHARACTER SET utf8mb4"))
        .map_err(|e| format!("could not create the database `{db}` at {shown}: {e}"))
}

fn execute(conn: &mut impl Queryable, s: &Statement) -> Result<Outcome, String> {
    let params: Vec<Value> = s.params.iter().map(to_mysql).collect();
    let params = if params.is_empty() { mysql::Params::Empty } else { mysql::Params::Positional(params) };
    let mut result = conn.exec_iter(s.sql.as_str(), params).map_err(|e| e.to_string())?;
    // Read before the rows: the counts belong to the current result set, and
    // iterating to its end moves past it, after which both read 0. That is
    // how every UPDATE first reported "0 rows changed".
    let affected = result.affected_rows();
    let insert_id = result.last_insert_id();
    let columns: Vec<String> = result.columns().as_ref().iter().map(|c| c.name_str().into_owned()).collect();
    let mut rows = Vec::new();
    for row in result.by_ref() {
        let row = row.map_err(|e| e.to_string())?;
        let mut obj = serde_json::Map::new();
        for (name, value) in columns.iter().zip(row.unwrap()) {
            obj.insert(name.clone(), to_json(value));
        }
        rows.push(obj);
    }
    Ok(Outcome { rows, affected, insert_id })
}

/// A parameter from a script: JSON in, MySQL out. A JSON array or object goes
/// as its JSON text, which is what a `JSON` column takes.
pub fn to_mysql(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::NULL,
        serde_json::Value::Bool(b) => Value::Int(*b as i64),
        serde_json::Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(i), _, _) => Value::Int(i),
            (_, Some(u), _) => Value::UInt(u),
            (_, _, Some(f)) => Value::Double(f),
            _ => Value::NULL,
        },
        serde_json::Value::String(s) => Value::Bytes(s.clone().into_bytes()),
        other => Value::Bytes(other.to_string().into_bytes()),
    }
}

/// A column value: MySQL in, JSON out. Text stays text; bytes that are not
/// UTF-8 (a `BLOB` of an image) are replaced lossily rather than failing the
/// row. Dates and times become the text MySQL itself would print.
pub fn to_json(v: Value) -> serde_json::Value {
    match v {
        Value::NULL => serde_json::Value::Null,
        Value::Int(i) => i.into(),
        Value::UInt(u) => u.into(),
        Value::Float(f) => serde_json::Number::from_f64(f as f64).map(Into::into).unwrap_or(serde_json::Value::Null),
        Value::Double(f) => serde_json::Number::from_f64(f).map(Into::into).unwrap_or(serde_json::Value::Null),
        Value::Bytes(b) => String::from_utf8_lossy(&b).into_owned().into(),
        Value::Date(y, mo, d, h, mi, s, us) => {
            let mut t = format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}");
            if us > 0 {
                t.push_str(&format!(".{us:06}"));
            }
            t.into()
        }
        Value::Time(neg, d, h, mi, s, us) => {
            let hours = d * 24 + h as u32;
            let mut t = format!("{}{hours:02}:{mi:02}:{s:02}", if neg { "-" } else { "" });
            if us > 0 {
                t.push_str(&format!(".{us:06}"));
            }
            t.into()
        }
    }
}

/// Parse a script's parameter list: a JSON array, or nothing.
pub fn parse_params(text: &str) -> Result<Vec<serde_json::Value>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Array(items)) => Ok(items),
        Ok(single) => Ok(vec![single]),
        Err(e) => Err(format!("parameters are not JSON: {e}")),
    }
}

/// Parse a transaction: a JSON array of `{ "sql": "...", "params": [...] }`
/// or of plain SQL strings.
pub fn parse_statements(text: &str) -> Result<Vec<Statement>, String> {
    let items: Vec<serde_json::Value> =
        serde_json::from_str(text).map_err(|e| format!("statements are not a JSON array: {e}"))?;
    items
        .into_iter()
        .map(|item| match item {
            serde_json::Value::String(sql) => Ok(Statement { sql, params: Vec::new() }),
            serde_json::Value::Object(mut o) => {
                let sql = match o.remove("sql") {
                    Some(serde_json::Value::String(s)) => s,
                    _ => return Err("a statement needs \"sql\"".to_string()),
                };
                let params = match o.remove("params") {
                    Some(serde_json::Value::Array(p)) => p,
                    None | Some(serde_json::Value::Null) => Vec::new(),
                    Some(other) => vec![other],
                };
                Ok(Statement { sql, params })
            }
            _ => Err("a statement is a string or { \"sql\": ..., \"params\": [...] }".to_string()),
        })
        .collect()
}
