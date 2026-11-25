//! The script functions: what a game calls to use MySQL.
//!
//! Declared through the engine's `ScriptExtension`, so every installed script
//! language gets them, and each is a `ScriptAction` underneath, which is how a
//! Rust script calls the same thing:
//!
//! ```ignore
//! ScriptAction::new("mysql_query", ctx.entity())
//!     .arg("sql", "SELECT name, x, y, z FROM props WHERE level = ?")
//!     .arg("params", "[3]")
//!     .arg("callback", "props")
//!     .send(ctx.world());
//! ```
//!
//! SQL runs on worker threads, so nothing here waits on the server. The answer
//! comes back a frame or more later as a game event named by `callback`, with
//! these arguments (Lua's `on_event(name, args)`, Rust's `ScriptHook::Event`):
//!
//! | arg | |
//! |---|---|
//! | `ok` | whether it worked |
//! | `error` | why not, or empty |
//! | `rows` | JSON array, one object per row |
//! | `count`, `affected`, `insert_id` | row count, rows changed, new id |
//! | `entities` | JSON array of entity ids (`mysql_spawn`) |
//! | `entity` | the spawned root's id (`mysql_load_entity`) |
//!
//! No callback means nobody is told, and a failure is only logged.

use std::collections::HashMap;

use bevy::prelude::*;
use renzora::script_bridge::{GameEvent, GameEventQueue, ScriptAction, ScriptActionValue};
use renzora::script_extension::{Bind, Binding, ParamKind, ScriptExtension};
use renzora::serde_json::Value;

use crate::entities;
use crate::query::{parse_params, parse_statements, Job, Outcome, Statement, Workers};

/// The worker pool and what each job in flight is waiting to do.
#[derive(Resource)]
pub struct MySqlScripts {
    pub workers: Workers,
    /// `allow_clients` from the config; see [`crate::config::Config`].
    pub allow_clients: bool,
    next_id: u64,
    pending: HashMap<u64, Pending>,
}

impl MySqlScripts {
    pub fn new(workers: Workers, allow_clients: bool) -> Self {
        Self { workers, allow_clients, next_id: 1, pending: HashMap::new() }
    }

    fn submit(&mut self, connection: String, statements: Vec<Statement>, transaction: bool, pending: Pending) {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, pending);
        self.workers.submit(Job { id, connection, statements, transaction });
    }
}

struct Pending {
    callback: String,
    then: Then,
}

/// What to do with the result on the main thread, where the world is.
enum Then {
    Report,
    SpawnRows { parent: Option<Entity> },
    SpawnTree { parent: Option<Entity> },
}

pub struct MySqlExtension;

impl ScriptExtension for MySqlExtension {
    fn name(&self) -> &str {
        "mysql"
    }

    fn bindings(&self) -> Vec<Binding> {
        use ParamKind::Str;
        vec![
            Bind::action("mysql_query", "mysql_query")
                .arg("sql", Str)
                .arg("callback", Str)
                .doc("Run SQL. The rows arrive as event `callback`.")
                .build(),
            Bind::action("mysql_query_with", "mysql_query")
                .arg("sql", Str)
                .arg("params", Str)
                .arg("callback", Str)
                .doc("Run SQL with ? parameters given as a JSON array, e.g. '[3, \"ana\"]'.")
                .build(),
            Bind::action("mysql_transaction", "mysql_transaction")
                .arg("statements", Str)
                .arg("callback", Str)
                .doc("Run a JSON array of statements together: all apply or none do.")
                .build(),
            Bind::action("mysql_spawn", "mysql_spawn")
                .arg("sql", Str)
                .arg("callback", Str)
                .doc("Spawn one entity per row (name, x, y, z, rx, ry, rz, scale, prefab or shape and color, components).")
                .build(),
            Bind::action("mysql_save_entity", "mysql_save_entity")
                .arg("table", Str)
                .arg("key", Str)
                .arg("entity", Str)
                .arg("callback", Str)
                .doc("Save an entity and everything under it to a row of `table`, under `key`.")
                .build(),
            Bind::action("mysql_load_entity", "mysql_load_entity")
                .arg("table", Str)
                .arg("key", Str)
                .arg("callback", Str)
                .doc("Spawn the entity tree saved under `key` in `table`.")
                .build(),
        ]
    }
}

fn arg_str(a: &ScriptAction, key: &str) -> String {
    match a.args.get(key) {
        Some(ScriptActionValue::String(s)) => s.clone(),
        Some(ScriptActionValue::Int(i)) => i.to_string(),
        Some(ScriptActionValue::Float(f)) => f.to_string(),
        Some(ScriptActionValue::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// An entity named in an argument: its id as a number, or its name.
fn arg_entity(world: &mut World, a: &ScriptAction, key: &str) -> Option<Entity> {
    match a.args.get(key) {
        Some(ScriptActionValue::Int(bits)) => Entity::try_from_bits(*bits as u64),
        Some(ScriptActionValue::String(s)) if !s.is_empty() => {
            if let Ok(bits) = s.parse::<u64>() {
                return Entity::try_from_bits(bits);
            }
            let mut q = world.query::<(Entity, &Name)>();
            q.iter(world).find(|(_, n)| n.as_str() == s).map(|(e, _)| e)
        }
        _ => None,
    }
}

/// `ScriptAction` observer: start whatever the action asks for.
pub fn on_action(trigger: On<ScriptAction>, mut commands: Commands) {
    let action = trigger.event().clone();
    if !action.name.starts_with("mysql_") {
        return;
    }
    commands.queue(move |world: &mut World| handle(world, action));
}

fn handle(world: &mut World, a: ScriptAction) {
    let callback = arg_str(&a, "callback");
    let connection = arg_str(&a, "connection");
    let fail = |world: &mut World, why: String| report(world, &callback, Err(why), Vec::new(), None);

    // A client is the player's machine: one that can query holds the
    // credentials, so the database answers to anyone who opens the file.
    if renzora::network::is_client(world) && !world.resource::<MySqlScripts>().allow_clients {
        return fail(
            world,
            format!(
                "`{}` was called on a client; database calls run on the server \
                 (set allow_clients = true in mysql.toml to permit it)",
                a.name
            ),
        );
    }

    let started: Result<(Vec<Statement>, bool, Then), String> = match a.name.as_str() {
        "mysql_query" | "mysql_execute" | "mysql_spawn" => parse_params(&arg_str(&a, "params")).map(|params| {
            let then = if a.name == "mysql_spawn" {
                let parent = arg_entity(world, &a, "parent");
                Then::SpawnRows { parent }
            } else {
                Then::Report
            };
            (vec![Statement { sql: arg_str(&a, "sql"), params }], false, then)
        }),
        "mysql_transaction" => parse_statements(&arg_str(&a, "statements")).map(|s| (s, true, Then::Report)),
        "mysql_save_entity" => save_entity(world, &a),
        "mysql_load_entity" => table_name(&arg_str(&a, "table")).map(|table| {
            let parent = arg_entity(world, &a, "parent");
            let sql = format!("SELECT data FROM `{table}` WHERE `key` = ?");
            let key = Value::String(arg_str(&a, "key"));
            (vec![Statement { sql, params: vec![key] }], false, Then::SpawnTree { parent })
        }),
        other => Err(format!("`{other}` is not a MySQL function")),
    };
    match started {
        Ok((statements, _, _)) if statements.iter().any(|s| s.sql.trim().is_empty()) => {
            fail(world, "no SQL was given".into())
        }
        Ok((statements, transaction, then)) => {
            world
                .resource_mut::<MySqlScripts>()
                .submit(connection, statements, transaction, Pending { callback: callback.clone(), then });
        }
        Err(why) => fail(world, why),
    }
}

/// A table name, checked: it goes into the SQL text, where a parameter cannot.
fn table_name(t: &str) -> Result<String, String> {
    if crate::config::is_identifier(t) {
        Ok(t.to_string())
    } else {
        Err(format!("table \"{t}\" is not a plain name; use letters, digits or _"))
    }
}

fn save_entity(world: &mut World, a: &ScriptAction) -> Result<(Vec<Statement>, bool, Then), String> {
    let table = table_name(&arg_str(a, "table"))?;
    let key = arg_str(a, "key");
    if key.is_empty() {
        return Err("a saved entity needs a key".into());
    }
    let entity = arg_entity(world, a, "entity")
        .or_else(|| a.target_entity.as_deref().and_then(|t| {
            let mut q = world.query::<(Entity, &Name)>();
            q.iter(world).find(|(_, n)| n.as_str() == t).map(|(e, _)| e)
        }))
        .unwrap_or(a.entity);
    let json = entities::tree_to_json(world, entity)?;
    // The table on first use, then the row: one transaction, so a failed
    // write leaves the previous save in place.
    let create = format!(
        "CREATE TABLE IF NOT EXISTS `{table}` (\
            `key` VARCHAR(191) NOT NULL PRIMARY KEY, \
            data LONGTEXT CHARACTER SET utf8mb4 NOT NULL, \
            saved_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP\
        ) CHARACTER SET utf8mb4"
    );
    let upsert = format!(
        "INSERT INTO `{table}` (`key`, data) VALUES (?, ?) ON DUPLICATE KEY UPDATE data = VALUES(data)"
    );
    Ok((
        vec![
            Statement { sql: create, params: Vec::new() },
            Statement { sql: upsert, params: vec![Value::String(key), Value::String(json)] },
        ],
        true,
        Then::Report,
    ))
}

/// Turn finished jobs into events, spawning first where the job asked.
pub fn deliver(world: &mut World) {
    let done = world.resource::<MySqlScripts>().workers.take_done();
    for d in done {
        let Some(p) = world.resource_mut::<MySqlScripts>().pending.remove(&d.id) else { continue };
        match (d.result, p.then) {
            (Ok(outcomes), Then::SpawnRows { parent }) => {
                let rows = outcomes.last().map(|o| o.rows.clone()).unwrap_or_default();
                let (spawned, warnings) = entities::spawn_rows(world, &rows, parent);
                for w in warnings {
                    warn!("[mysql] {w}");
                }
                report(world, &p.callback, Ok(outcomes), spawned, None);
            }
            (Ok(outcomes), Then::SpawnTree { parent }) => {
                let data = outcomes
                    .last()
                    .and_then(|o| o.rows.first())
                    .and_then(|r| r.get("data"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                match data.map(|d| entities::spawn_tree(world, &d, parent)) {
                    Some(Ok((root, warnings))) => {
                        for w in warnings {
                            warn!("[mysql] {w}");
                        }
                        report(world, &p.callback, Ok(outcomes), Vec::new(), Some(root));
                    }
                    Some(Err(e)) => report(world, &p.callback, Err(e), Vec::new(), None),
                    None => report(world, &p.callback, Err("no entity is saved under that key".into()), Vec::new(), None),
                }
            }
            (result, _) => report(world, &p.callback, result, Vec::new(), None),
        }
    }
}

fn report(world: &mut World, callback: &str, result: Result<Vec<Outcome>, String>, spawned: Vec<Entity>, root: Option<Entity>) {
    if callback.is_empty() {
        if let Err(e) = &result {
            warn!("[mysql] {e}");
        }
        return;
    }
    let mut args: HashMap<String, ScriptActionValue> = HashMap::new();
    match &result {
        Ok(outcomes) => {
            let last = outcomes.last().cloned().unwrap_or_default();
            args.insert("ok".into(), ScriptActionValue::Bool(true));
            args.insert("error".into(), ScriptActionValue::String(String::new()));
            let rows: Vec<Value> = last.rows.iter().cloned().map(Value::Object).collect();
            args.insert("rows".into(), ScriptActionValue::String(Value::Array(rows).to_string()));
            args.insert("count".into(), ScriptActionValue::Int(last.rows.len() as i64));
            args.insert("affected".into(), ScriptActionValue::Int(outcomes.iter().map(|o| o.affected as i64).sum()));
            args.insert("insert_id".into(), ScriptActionValue::Int(last.insert_id.unwrap_or(0) as i64));
        }
        Err(e) => {
            args.insert("ok".into(), ScriptActionValue::Bool(false));
            args.insert("error".into(), ScriptActionValue::String(e.clone()));
            args.insert("rows".into(), ScriptActionValue::String("[]".into()));
            args.insert("count".into(), ScriptActionValue::Int(0));
        }
    }
    let ids: Vec<Value> = spawned.iter().map(|e| Value::from(e.to_bits())).collect();
    args.insert("entities".into(), ScriptActionValue::String(Value::Array(ids).to_string()));
    if let Some(root) = root {
        args.insert("entity".into(), ScriptActionValue::Int(root.to_bits() as i64));
    }
    world
        .get_resource_or_insert_with(GameEventQueue::default)
        .pending
        .push(GameEvent { name: callback.to_string(), args, from: None });
}
