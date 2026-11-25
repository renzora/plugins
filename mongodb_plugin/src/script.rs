//! The script functions: what a game calls to use MongoDB.
//!
//! Declared through the engine's `ScriptExtension`, so every installed script
//! language gets them, and each is a `ScriptAction` underneath, which is how a
//! Rust script calls the same thing:
//!
//! ```ignore
//! ScriptAction::new("mongo_find", ctx.entity())
//!     .arg("collection", "players")
//!     .arg("filter", r#"{"level": {"$gte": 3}}"#)
//!     .arg("callback", "players")
//!     .send(ctx.world());
//! ```
//!
//! The work runs on worker threads; the answer comes back a frame or more
//! later as a game event named by `callback` (Lua's `on_event`, Rust's
//! `ScriptHook::Event`):
//!
//! | arg | |
//! |---|---|
//! | `ok`, `error` | whether it worked, and why not |
//! | `docs` | JSON array of documents (relaxed Extended JSON) |
//! | `count` | documents returned, matched (update) or counted |
//! | `affected` | documents inserted, modified or deleted |
//! | `inserted_ids` | JSON array of new `_id`s (insert, upsert) |
//! | `entities` / `entity` | spawned entity ids (`mongo_spawn` / `mongo_load_entity`) |

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use renzora::script_bridge::{GameEvent, GameEventQueue, ScriptAction, ScriptActionValue};
use renzora::script_extension::{Bind, Binding, ParamKind, ScriptExtension};
use renzora::serde_json::{self, Map, Value};

use crate::entities;
use crate::query::{Job, Op, Outcome, Workers};

#[derive(Resource)]
pub struct MongoScripts {
    pub workers: Workers,
    /// `allow_clients` from the config; see [`crate::config::Config`].
    pub allow_clients: bool,
    next_id: u64,
    pending: HashMap<u64, Pending>,
}

/// How long a script waits for an answer before it gets a failure instead.
/// The driver has no limit of its own on an operation once it is connected, so
/// without this a hung server would leave a callback that never fires.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(30);

impl MongoScripts {
    pub fn new(workers: Workers, allow_clients: bool) -> Self {
        Self { workers, allow_clients, next_id: 1, pending: HashMap::new() }
    }

    fn submit(&mut self, connection: String, database: String, op: Op, pending: Pending) {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, pending);
        self.workers.submit(Job { id, connection, database, op });
    }
}

struct Pending {
    callback: String,
    then: Then,
    started: Instant,
}

/// What to do with the result on the main thread, where the world is.
enum Then {
    Report,
    SpawnDocs { parent: Option<Entity> },
    SpawnTree { parent: Option<Entity> },
}

pub struct MongoExtension;

impl ScriptExtension for MongoExtension {
    fn name(&self) -> &str {
        "mongodb"
    }

    fn bindings(&self) -> Vec<Binding> {
        use ParamKind::Str;
        let f = |name: &str, args: &[&str], doc: &str| {
            let mut b = Bind::action(name, name);
            for a in args {
                b = b.arg(a, Str);
            }
            b.doc(doc).build()
        };
        vec![
            f("mongo_find", &["collection", "filter", "callback"], "Find documents matching a JSON filter."),
            f(
                "mongo_find_with",
                &["collection", "filter", "options", "callback"],
                "Find with options: {\"sort\": {...}, \"limit\": n, \"skip\": n, \"projection\": {...}}.",
            ),
            f("mongo_insert", &["collection", "documents", "callback"], "Insert a document (object) or several (array)."),
            f("mongo_update", &["collection", "filter", "update", "callback"], "Update every matching document, e.g. {\"$inc\": {\"hp\": -1}}."),
            f("mongo_delete", &["collection", "filter", "callback"], "Delete every matching document."),
            f("mongo_aggregate", &["collection", "pipeline", "callback"], "Run an aggregation pipeline (a JSON array of stages)."),
            f("mongo_count", &["collection", "filter", "callback"], "Count matching documents."),
            f("mongo_command", &["command", "callback"], "Run any database command, as db.runCommand takes it."),
            f(
                "mongo_spawn",
                &["collection", "filter", "callback"],
                "Spawn one entity per matching document (name, x, y, z, rx, ry, rz, scale, prefab or shape and color, components).",
            ),
            f("mongo_save_entity", &["collection", "key", "entity", "callback"], "Save an entity and everything under it as one document."),
            f("mongo_load_entity", &["collection", "key", "callback"], "Spawn the entity tree saved under `key`."),
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

fn arg_bool(a: &ScriptAction, key: &str, default: bool) -> bool {
    match a.args.get(key) {
        Some(ScriptActionValue::Bool(b)) => *b,
        Some(ScriptActionValue::String(s)) => s == "true",
        Some(ScriptActionValue::Int(i)) => *i != 0,
        _ => default,
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

pub fn on_action(trigger: On<ScriptAction>, mut commands: Commands) {
    let action = trigger.event().clone();
    if !action.name.starts_with("mongo_") {
        return;
    }
    commands.queue(move |world: &mut World| handle(world, action));
}

fn handle(world: &mut World, a: ScriptAction) {
    let callback = arg_str(&a, "callback");
    // A client is the player's machine: one that can query holds the
    // credentials, so the database answers to anyone who opens the file.
    if renzora::network::is_client(world) && !world.resource::<MongoScripts>().allow_clients {
        let why = format!(
            "`{}` was called on a client; database calls run on the server \
             (set allow_clients = true in mongodb.toml to permit it)",
            a.name
        );
        return report(world, &callback, Err(why), Vec::new(), None);
    }
    let collection = arg_str(&a, "collection");
    let filter = arg_str(&a, "filter");
    let parent = arg_entity(world, &a, "parent");
    let started: Result<(Op, Then), String> = match a.name.as_str() {
        "mongo_find" | "mongo_find_with" => {
            Ok((Op::Find { collection, filter, options: arg_str(&a, "options") }, Then::Report))
        }
        "mongo_spawn" => Ok((Op::Find { collection, filter, options: arg_str(&a, "options") }, Then::SpawnDocs { parent })),
        "mongo_insert" => Ok((Op::Insert { collection, documents: arg_str(&a, "documents") }, Then::Report)),
        "mongo_update" => Ok((
            Op::Update {
                collection,
                filter,
                update: arg_str(&a, "update"),
                many: arg_bool(&a, "many", true),
                upsert: arg_bool(&a, "upsert", false),
            },
            Then::Report,
        )),
        "mongo_delete" => Ok((Op::Delete { collection, filter, many: arg_bool(&a, "many", true) }, Then::Report)),
        "mongo_aggregate" => Ok((Op::Aggregate { collection, pipeline: arg_str(&a, "pipeline") }, Then::Report)),
        "mongo_count" => Ok((Op::Count { collection, filter }, Then::Report)),
        "mongo_command" => Ok((Op::Command { command: arg_str(&a, "command") }, Then::Report)),
        "mongo_save_entity" => {
            let key = arg_str(&a, "key");
            let entity = arg_entity(world, &a, "entity").unwrap_or(a.entity);
            if key.is_empty() {
                Err("a saved entity needs a key".into())
            } else {
                entities::tree_to_json(world, entity).map(|tree| (Op::SaveTree { collection, key, tree }, Then::Report))
            }
        }
        "mongo_load_entity" => Ok((Op::LoadTree { collection, key: arg_str(&a, "key") }, Then::SpawnTree { parent })),
        other => Err(format!("`{other}` is not a MongoDB function")),
    };
    match started {
        Ok((op, then)) => world.resource_mut::<MongoScripts>().submit(
            arg_str(&a, "connection"),
            arg_str(&a, "database"),
            op,
            Pending { callback, then, started: Instant::now() },
        ),
        Err(why) => report(world, &callback, Err(why), Vec::new(), None),
    }
}

fn parse_docs(text: &str) -> Vec<Map<String, Value>> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(items)) => items
            .into_iter()
            .filter_map(|v| match v {
                Value::Object(m) => Some(m),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Turn finished operations into events, spawning first where asked.
pub fn deliver(world: &mut World) {
    // Give up on what has waited too long. Its answer, if it ever comes, finds
    // no pending entry and is dropped below.
    let overdue: Vec<(u64, String)> = world
        .resource::<MongoScripts>()
        .pending
        .iter()
        .filter(|(_, p)| p.started.elapsed() > ANSWER_WITHIN)
        .map(|(id, p)| (*id, p.callback.clone()))
        .collect();
    for (id, callback) in overdue {
        world.resource_mut::<MongoScripts>().pending.remove(&id);
        let why = format!("MongoDB did not answer within {}s", ANSWER_WITHIN.as_secs());
        report(world, &callback, Err(why), Vec::new(), None);
    }

    let done = world.resource::<MongoScripts>().workers.take_done();
    for d in done {
        let Some(p) = world.resource_mut::<MongoScripts>().pending.remove(&d.id) else { continue };
        match (d.result, p.then) {
            (Ok(outcome), Then::SpawnDocs { parent }) => {
                let (spawned, warnings) = entities::spawn_docs(world, &parse_docs(&outcome.docs), parent);
                for w in warnings {
                    warn!("[mongodb] {w}");
                }
                report(world, &p.callback, Ok(outcome), spawned, None);
            }
            (Ok(outcome), Then::SpawnTree { parent }) => {
                let tree = parse_docs(&outcome.docs).into_iter().next().map(|m| Value::Object(m).to_string());
                match tree.map(|t| entities::spawn_tree(world, &t, parent)) {
                    Some(Ok((root, warnings))) => {
                        for w in warnings {
                            warn!("[mongodb] {w}");
                        }
                        report(world, &p.callback, Ok(outcome), Vec::new(), Some(root));
                    }
                    Some(Err(e)) => report(world, &p.callback, Err(e), Vec::new(), None),
                    None => report(world, &p.callback, Err("the saved entity is empty".into()), Vec::new(), None),
                }
            }
            (result, _) => report(world, &p.callback, result, Vec::new(), None),
        }
    }
}

fn report(world: &mut World, callback: &str, result: Result<Outcome, String>, spawned: Vec<Entity>, root: Option<Entity>) {
    if callback.is_empty() {
        if let Err(e) = &result {
            warn!("[mongodb] {e}");
        }
        return;
    }
    let mut args: HashMap<String, ScriptActionValue> = HashMap::new();
    let s = |v: String| ScriptActionValue::String(v);
    match result {
        Ok(o) => {
            args.insert("ok".into(), ScriptActionValue::Bool(true));
            args.insert("error".into(), s(String::new()));
            args.insert("docs".into(), s(o.docs));
            args.insert("count".into(), ScriptActionValue::Int(o.count));
            args.insert("affected".into(), ScriptActionValue::Int(o.affected));
            args.insert("inserted_ids".into(), s(o.inserted_ids));
        }
        Err(e) => {
            args.insert("ok".into(), ScriptActionValue::Bool(false));
            args.insert("error".into(), s(e));
            args.insert("docs".into(), s("[]".into()));
            args.insert("count".into(), ScriptActionValue::Int(0));
            args.insert("affected".into(), ScriptActionValue::Int(0));
            args.insert("inserted_ids".into(), s("[]".into()));
        }
    }
    let ids: Vec<Value> = spawned.iter().map(|e| Value::from(e.to_bits())).collect();
    args.insert("entities".into(), s(Value::Array(ids).to_string()));
    if let Some(root) = root {
        args.insert("entity".into(), ScriptActionValue::Int(root.to_bits() as i64));
    }
    world
        .get_resource_or_insert_with(GameEventQueue::default)
        .pending
        .push(GameEvent { name: callback.to_string(), args, from: None });
}
