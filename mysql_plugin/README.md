# MySQL

Connect a game to a MySQL (or MariaDB) database: run any SQL from a script, spawn entities from rows, save entities into rows, and optionally keep the game's save slots there too.

> **Read before shipping a multiplayer game.** Whatever machine runs these functions holds the database password, in `mysql.toml`, where anyone with the game can read it and then run any SQL they like. Run database calls on the **server** (host or dedicated) and ship `mysql.toml` with the server only, never with the client build. A networked client is refused by default; see [Security](#security).

## Script functions

Every script language gets these, and a Rust script calls the same thing by sending the action (see below).

| Function | Does |
|---|---|
| `mysql_query(sql, callback)` | Run any SQL. |
| `mysql_query_with(sql, params, callback)` | The same, with `?` placeholders filled from `params`, a JSON array: `'[3, "ana"]'`. Always use this for values from players. |
| `mysql_transaction(statements, callback)` | A JSON array of statements, either `"SQL"` or `{ "sql": "...", "params": [...] }`, run together: all apply or none do. |
| `mysql_spawn(sql, callback)` | Spawn one entity per row (see below). |
| `mysql_save_entity(table, key, entity, callback)` | Save an entity and everything under it as one row of `table`, under `key`. The table is created if missing. `entity` is a name or an id. |
| `mysql_load_entity(table, key, callback)` | Spawn the entity tree saved under `key`. |

Every call also accepts a `connection` argument naming one of the `[connections]` in `mysql.toml`, and `mysql_spawn` / `mysql_load_entity` a `parent` to spawn under.

### The answer

SQL runs on background threads, so the game never waits on the server. The answer arrives a frame or more later as an event named by `callback` (`on_event(name, args)` in Lua, `ScriptHook::Event` in Rust):

| Arg | |
|---|---|
| `ok` | whether it worked |
| `error` | why not, or empty |
| `rows` | JSON array, one object per row: `[{"name": "ana", "score": 40}]` |
| `count` | rows returned |
| `affected` | rows changed |
| `insert_id` | the new row's id after an `INSERT` |
| `entities` | JSON array of entity ids, from `mysql_spawn` |
| `entity` | the loaded root's id, from `mysql_load_entity` |

An empty `callback` sends no event, and a failure is only logged.

### From Lua

```lua
mysql_query_with("SELECT name, score FROM players WHERE level = ? ORDER BY score DESC", "[3]", "scores")

function on_event(name, args)
    if name == "scores" and args.ok then
        for _, row in ipairs(json_parse(args.rows)) do
            print(row.name .. ": " .. row.score)
        end
    end
end
```

### From Rust

```rust
use renzora::script_bridge::{ScriptAction, ScriptActionValue};

ScriptAction::new("mysql_query", ctx.entity())
    .arg("sql", "SELECT name, score FROM players WHERE level = ?")
    .arg("params", "[3]")
    .arg("callback", "scores")
    .send(ctx.world());

fn hooks(ctx: &mut ScriptCtx, hook: &ScriptHook) {
    if let ScriptHook::Event { name: "scores", args, .. } = hook {
        if let Some(ScriptActionValue::String(rows)) = args.get("rows") {
            let rows: Vec<renzora::serde_json::Value> = renzora::serde_json::from_str(rows).unwrap_or_default();
        }
    }
}
```

Needs an engine with `ScriptHook::Event`.

## Entities from rows

`mysql_spawn` reads these columns when a row has them:

| Column | |
|---|---|
| `name` | the entity's name |
| `x`, `y`, `z` | position |
| `rx`, `ry`, `rz` | rotation in degrees |
| `scale` | uniform scale |
| `prefab` | a `.bsn` to instance, project-relative |
| `shape`, `color` | a built-in shape (`cube`, `cylinder`...) and `#rrggbb` |
| `components` | JSON: `{"Health": {"0": 50}}`, any registered component by type path or short name |

The whole row stays on the entity in the `MySqlRow` component (`data`, as JSON), so a script can read the game's own columns from it later.

## Entity trees in a row

`mysql_save_entity` writes the entity, its children and every component that can be written, as JSON, into a `LONGTEXT` column. `mysql_load_entity` spawns it back as new entities. The root keeps its own (local) transform: save the entity that holds the placement you want, which for a prefab instance is the instance itself, not the object inside it. Components that hold handles or links to other entities, and those the engine recomputes each frame, are left out; meshes and models come back from their `MeshPrimitive`, `MeshInstanceData` or prefab as usual.

## Save slots

Unless `saves = false`, the plugin also takes over the engine's save slots: `save_write`, `save_load`, `save_exists`, `save_delete` and `save_slots` go to the table `renzora_saves`, one row per game and slot, holding the same JSON a slot file would. Remove the plugin and those calls go back to files.

When the server is down, the first save call waits out `connect_timeout_secs` (2 seconds by default) and fails with the reason. Every later call fails at once with the same reason while the plugin checks every 5 seconds in the background, and the next call after the server answers just works.

> **`save_exists` answers false when the server cannot be reached**, the same as for a slot that was never saved. A game that does "no save exists, so start fresh and save" will **overwrite the player's real save** the moment the database is back. Decide with `save_load` instead: it returns the save, nothing for a slot that is genuinely empty, or an error when the database is unreachable, and only the middle case should start fresh.

**Only one plugin can keep the save slots.** With both the MySQL and MongoDB plugins installed and `saves = true` in both, the first to load keeps them and the other logs a warning and leaves them alone. Set `saves = false` in the one that should not.

## Security

`mysql.toml` holds the database's address, user and password, and a shipped game reads it from beside its executable, so anyone who has the file has the database.

- **Single-player:** the player has the credentials. Use a database only that game uses, with a user that may touch only its own tables, or a server on the player's own machine.
- **Multiplayer:** run database calls on the server. Script functions called on a networked client fail with an error saying so, unless `allow_clients = true` in `mysql.toml`. Ship `mysql.toml` with the server build only. Check `is_server()` in scripts that run on both, and send results to players with RPCs.
- Give the plugin a MySQL user with only the rights the game needs (`SELECT, INSERT, UPDATE, DELETE` on its own database), not `root`.

## Setup

Run a MySQL or MariaDB server, then either rely on the defaults (`root` with no password on `127.0.0.1:3306`, database `renzora_saves`) or copy `mysql.example.toml` to `mysql.toml`, beside the game's executable for a shipped game, or in the project folder while developing. The file beside the executable wins. `RENZORA_MYSQL_URL` overrides `url`. The database is created on first use unless `create_database = false`.

The connection is not encrypted. That is fine for a server on the same machine; for one elsewhere, keep it on a private network.

**Scope:** Runtime, so it ships with the game. Not on the web, which has no TCP sockets.
