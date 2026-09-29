# MCP

Lets an MCP client (Claude Code, or anything else that speaks the protocol) read
and drive the editor that is currently open.

The point is the *running* editor. A project's files do not say which entities
exist after a script ran, what the console printed, or what the viewport looks
like right now, and those are the questions worth asking mid-session.

**Scope:** Editor. A shipped game has no editor to drive and no business opening
a local port that accepts commands, so this never loads in an exported game.

## Connecting

The editor prints the exact command on startup, in the Console under the `mcp`
category. It is:

```sh
claude mcp add --transport http renzora http://127.0.0.1:47800/mcp
```

Set `RENZORA_MCP_PORT` before launching the editor to bind somewhere else. The
port is fixed rather than "first one free", because the URL is written into a
client's config once and never looked at again: a port that moved when something
else held 47800 would leave that config pointing at nothing.

The status bar shows the state. `MCP 47800` means listening with nobody
attached, `MCP 12 calls` in green means a client is connected, and a red
`MCP port 47800 busy` means the bind failed and nothing is listening.

Localhost only, deliberately. These tools spawn entities, save scenes and read
the console of whatever project is open.

## Tools

| | |
|---|---|
| `editor_state` | Open project, current scene, play mode, selection, entity count, frame rate |
| `read_console` | Recent Console lines, with level and text filters |
| `scene_tree` | The scene's named entities as a tree, with their ids |
| `inspect_entity` | One entity's components and reflected field values |
| `screenshot` | A PNG of the editor window |
| `list_shapes` | Every shape `spawn_entity` can build, from the project's shape registry |
| `spawn_entity` | A named entity, optionally with a shape, colour, parent and collider |
| `spawn_light` | A light; a scene built only from shapes renders black without one |
| `duplicate_entity` | Copy an entity with its whole subtree |
| `despawn_entity` | Remove an entity and its children |
| `rename_entity` | Change the name the hierarchy shows and a saved scene keys on |
| `reparent_entity` | Move under another parent, or out to the scene root |
| `set_transform` | Move, rotate or scale |
| `set_component_field` | Write one reflected field by dotted path |
| `add_component` | Add any registered component by name and set its fields |
| `remove_component` | Remove a component |
| `make_solid` | Give colliders to an entity and every mesh under it |
| `attach_script` / `detach_script` | Attach or remove a script file |
| `list_assets` | Project files by kind, or the engine's presets with `source: engine` |
| `set_material` | Put a `.material` on an entity's meshes, or write a new one from colours |
| `import_model` | Copy a model into the project and convert it to `.glb`; answers when done |
| `spawn_model` | Place an imported `.glb` in the scene |
| `spawn_particles` | Spawn a `.particle` effect |
| `add_ui` | Show an `.html` UI template on a canvas of its own |
| `list_presets` / `spawn_preset` | Everything the Add Entity menu offers: cameras, terrain, environment, probes |
| `set_default_camera` | Choose the scene camera play mode looks through |
| `undo` | Step the scene's undo history back, or forward with `redo` |
| `set_autosave` | Pause autosave for this session, so test edits are not saved into a real scene |
| `select_entities` | Set the editor's selection |
| `set_view` | Move the viewport camera: eye and target, frame one entity, or frame all |
| `set_play_mode` | play, pause, stop or simulate |
| `save_scene` | What Ctrl+S does |
| `editor_request` | What the menus ask for: new/open/save-as scene, export, import, settings |

Entity ids are opaque strings. They are the decimal form of `Entity::to_bits`,
and strings rather than numbers because those bits pack a generation into the
high 32: an entity from a long session exceeds the 2^53 a JSON number carries
exactly, and the id would come back rounded to a *different* entity.

## What it cannot do

**A despawn cannot be undone.** Every other edit goes on the editor's own undo
stack, through the undo core in the contract crate (`edits.rs`), so Ctrl+Z
takes it back. Undoing a despawn honestly means restoring every component of a
whole subtree, which needs the scene serialiser, so `despawn_entity` is marked
destructive instead. Files a tool writes or copies into the project (materials,
imported models) stay on disk after an undo; only the scene change is reversed.

**Content is files.** A material, model, particle effect or UI template is a
file the scene points at, because that is what a saved scene keeps. A path
outside the project is copied in first, so the scene never references a file
that disappears when the project moves.

**It is only reachable from this machine, and not from a browser.** The server
binds loopback and refuses a request whose `Host` is not loopback, whose
`Origin` is anything else, or whose POST is not `application/json`. Those three
are what a web page open on the same machine cannot get past, and a local MCP
client never trips any of them.

**The screenshot is the whole window,** not the viewport alone. The viewport's
render target is not reachable from the contract crate. The window is arguably
the more useful picture anyway, since it shows the panels and the state of the
editor around the scene.

**Nothing is pushed.** The server answers requests and never speaks first, so a
`GET /mcp` asking for an event stream is answered with 405. Poll `read_console`
if you want to watch something happen.

## Shape of it

- `http.rs` owns the listener and speaks HTTP/1.1 by hand. There is no HTTP
  crate here on purpose: a plugin that declares no crates.io dependency never
  invokes cargo, so its build stays offline and takes about a second, against the
  half-minute a resolve-and-compile would cost on every engine move.
- `rpc.rs` is the MCP envelope and the tool catalogue. It turns a `tools/call`
  into a message on a channel and blocks on the reply.
- `tools.rs` drains that channel from an exclusive system, so every tool runs on
  the main thread with `&mut World` and none of them has to think about what else
  is touching the world.

Bevy's world is not `Sync` and a socket cannot wait for a frame, so the two
halves have to be different threads with a queue between them. That is the whole
design, and it is why a new tool is usually twenty lines.
