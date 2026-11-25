# MongoDB

Connect a game to a MongoDB database: run any query or command from a script, spawn entities from documents, save entities into documents, and optionally keep the game's save slots there too. Works with a local server and with MongoDB Atlas.

> **Read before shipping a multiplayer game.** Whatever machine runs these functions holds the database password, in `mongodb.toml`, where anyone with the game can read it and then run any command they like. Run database calls on the **server** (host or dedicated) and ship `mongodb.toml` with the server only, never with the client build. A networked client is refused by default; see [Security](#security).

## Script functions

Every script language gets these, and a Rust script calls the same thing by sending the action (see below). Filters, updates, documents and pipelines are **JSON text**, read as MongoDB Extended JSON, so `{"_id": {"$oid": "..."}}` and `{"hp": {"$lt": 10}}` mean what they mean in the shell. An empty filter matches every document.

| Function | Does |
|---|---|
| `mongo_find(collection, filter, callback)` | Find matching documents. |
| `mongo_find_with(collection, filter, options, callback)` | The same with `{"sort": {...}, "limit": n, "skip": n, "projection": {...}}`. |
| `mongo_insert(collection, documents, callback)` | Insert one document (an object) or several (an array). |
| `mongo_update(collection, filter, update, callback)` | Update every match: `{"$inc": {"hp": -1}}`, or a pipeline array. |
| `mongo_delete(collection, filter, callback)` | Delete every match. |
| `mongo_aggregate(collection, pipeline, callback)` | Run an aggregation pipeline (an array of stages). |
| `mongo_count(collection, filter, callback)` | Count matches. |
| `mongo_command(command, callback)` | Any database command, as `db.runCommand` takes it: indexes, stats, anything not above. |
| `mongo_spawn(collection, filter, callback)` | Spawn one entity per matching document (see below). |
| `mongo_save_entity(collection, key, entity, callback)` | Save an entity and everything under it as one document, `_id = key`. `entity` is a name or an id. |
| `mongo_load_entity(collection, key, callback)` | Spawn the entity tree saved under `key`. |

Every call also accepts `connection` (one of the `[connections]` in `mongodb.toml`) and `database` (another database on that server). `mongo_update` and `mongo_delete` take `many = false` to touch only the first match, and `mongo_update` takes `upsert = true`. `mongo_spawn` and `mongo_load_entity` take a `parent` to spawn under. A Lua call passes these by calling `action("mongo_update", { ... })` with the extra keys.

### The answer

Work runs on background threads, so the game never waits on the server. The answer arrives a frame or more later as an event named by `callback` (`on_event(name, args)` in Lua, `ScriptHook::Event` in Rust):

| Arg | |
|---|---|
| `ok` | whether it worked |
| `error` | why not, or empty |
| `docs` | JSON array of documents, relaxed Extended JSON (`_id` arrives as `{"$oid": "..."}`) |
| `count` | documents returned, matched by an update, or counted |
| `affected` | documents inserted, modified or deleted |
| `inserted_ids` | JSON array of new `_id`s, from insert or an upsert |
| `entities` | JSON array of entity ids, from `mongo_spawn` |
| `entity` | the loaded root's id, from `mongo_load_entity` |

An empty `callback` sends no event, and a failure is only logged.

Each call runs on its own thread, so a slow one never holds up the others. A call with no answer after 30 seconds gets a failure event instead, and reads carry a 25-second server-side limit. At most 32 calls wait at once; past that, a new call fails straight away, which in practice means a script is calling every frame.

### From Lua

```lua
mongo_find_with("players", '{"level": 3}', '{"sort": {"score": -1}, "limit": 10}', "top")

function on_event(name, args)
    if name == "top" and args.ok then
        for _, p in ipairs(json_parse(args.docs)) do
            print_log(p.name .. ": " .. p.score)
        end
    end
end
```

### From Rust

```rust
use renzora::script_bridge::{ScriptAction, ScriptActionValue};

ScriptAction::new("mongo_find_with", ctx.entity())
    .arg("collection", "players")
    .arg("filter", r#"{"level": 3}"#)
    .arg("options", r#"{"sort": {"score": -1}, "limit": 10}"#)
    .arg("callback", "top")
    .send(ctx.world());

fn hooks(ctx: &mut ScriptCtx, hook: &ScriptHook) {
    if let ScriptHook::Event { name: "top", args, .. } = hook {
        if let Some(ScriptActionValue::String(docs)) = args.get("docs") {
            let docs: Vec<renzora::serde_json::Value> = renzora::serde_json::from_str(docs).unwrap_or_default();
        }
    }
}
```

Needs an engine with `ScriptHook::Event`.

## Entities from documents

`mongo_spawn` reads these fields when a document has them:

| Field | |
|---|---|
| `name` | the entity's name |
| `x`, `y`, `z` | position |
| `rx`, `ry`, `rz` | rotation in degrees |
| `scale` | uniform scale |
| `prefab` | a `.bsn` to instance, project-relative |
| `shape`, `color` | a built-in shape (`cube`, `cylinder`...) and `#rrggbb` |
| `components` | an object: `{"Health": {"0": 50}}`, any registered component by type path or short name |

The whole document stays on the entity in the `MongoDoc` component (`data`, as JSON), so a script can read the game's own fields from it later.

## Entity trees in a document

`mongo_save_entity` stores the entity, its children and every component that can be written as a nested document under `tree`, with `saved_at`. `mongo_load_entity` spawns it back as new entities. Components the engine rebuilds (physics bodies, computed bounds) or that link to other entities are left out. The root keeps its own (local) transform: save the entity that holds the placement you want, which for a prefab instance is the instance itself.

## Save slots

Unless `saves = false`, the plugin takes over the engine's save slots: `save_write`, `save_load`, `save_exists`, `save_delete` and `save_slots` go to the collection `renzora_saves`, one document per game and slot (`game`, `slot`, `data`, `saved_at`), holding the same JSON a slot file would. Remove the plugin and those calls go back to files.

When the server is down, the first save call waits out `connect_timeout_secs` (2 seconds by default) and fails with the reason. Every later call fails at once with the same reason while the plugin checks every 5 seconds in the background, and the next call after the server answers just works.

> **`save_exists` answers false when the server cannot be reached**, the same as for a slot that was never saved. A game that does "no save exists, so start fresh and save" will **overwrite the player's real save** the moment the database is back. Decide with `save_load` instead: it returns the save, nothing for a slot that is genuinely empty, or an error when the database is unreachable, and only the middle case should start fresh.

**Only one plugin can keep the save slots.** With both the MySQL and MongoDB plugins installed and `saves = true` in both, the first to load keeps them and the other logs a warning and leaves them alone. Set `saves = false` in the one that should not.

## Security

`mongodb.toml` holds the server's address, user and password, and a shipped game reads it from beside its executable, so anyone who has the file has the database.

- **Single-player:** the player has the credentials. Use a database only that game uses, with a user whose role covers only it, or a server on the player's own machine.
- **Multiplayer:** run database calls on the server. Script functions called on a networked client fail with an error saying so, unless `allow_clients = true` in `mongodb.toml`. Ship `mongodb.toml` with the server build only. Check `is_server()` in scripts that run on both, and send results to players with RPCs.
- Give the plugin a user with `readWrite` on the game's database only, not an admin role. `mongo_command` can run anything that user may.

## Setup

Run MongoDB, then either rely on the defaults (`mongodb://127.0.0.1:27017`, database `renzora`) or copy `mongodb.example.toml` to `mongodb.toml`, beside the game's executable for a shipped game, or in the project folder while developing. The file beside the executable wins. `RENZORA_MONGODB_URL` overrides `url`. Atlas works: use its `mongodb+srv://` URL.

Multi-document transactions are not offered: they need a replica set, which a plain local server is not.

**Building the plugin** needs a C compiler on Windows (the MSVC build tools Rust's installer normally sets up), for the TLS library the MongoDB driver requires.

**Scope:** Runtime, so it ships with the game. Not on the web, which has no TCP sockets.
