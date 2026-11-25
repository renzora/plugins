# Python

Python scripting for Renzora. Attach a `.py` script to an entity and the engine runs it, with the same hooks and the same engine functions as Lua.

The interpreter is [RustPython](https://github.com/RustPython/RustPython), compiled into the plugin, so nobody has to install Python: not you, not your players. An exported game carries it like any other plugin.

```python
# scripts/spinner.py
def props():
    return {"speed": {"value": 90.0, "hint": "degrees per second"}}

def on_update():
    rotate(0, speed * delta, 0)
```

## Hooks

Define the ones you need; a script that defines none of them is valid. A hook may declare fewer parameters than it is given (`def on_rpc(name, args):` ignores the sender), as in Lua.

| Hook | When |
|---|---|
| `on_ready()` | once, when the entity's scripts start |
| `on_update()` | every frame |
| `on_fixed_update()` | every fixed physics step; `delta` is the step |
| `on_ui(name, args, entity)` | a UI markup callback fired |
| `on_draw(g)` | every frame, to draw 2D shapes on `g` (below) |
| `on_rpc(name, args, sender)` | a networked RPC arrived |
| `on_animation_event(name, entity)` | animation playback crossed a marker |
| `on_http(callback, status, body)` | an `http_get` / `http_post` answered |
| `on_player_joined(id)` / `on_player_left(id)` | a peer connected or left (server only) |
| `on_scene_loaded(path)` / `on_scene_load_failed(path, error)` | a scene load finished |
| `on_event(name, args)` | a broadcast event (`emit`, or a plugin answering) |

`args` is a `dict`.

## Variables

Before every hook the script's globals are refreshed with the frame's inputs, under the same names as in Lua: `delta`, `elapsed`, `position_x/y/z`, `rotation_x/y/z`, `scale_x/y/z`, `input_x/y`, `mouse_x/y`, `mouse_delta_x/y`, `mouse_left/right/middle`, `mouse_scroll`, `camera_yaw`, `camera_ev`, `project_width/height`, `gamepad_left_x` (and the rest of the first pad), `self_entity_id`, `self_entity_name`, `is_colliding`, `timers_finished` (a list), `self_health`, `self_max_health`, `has_parent`, `parent_position_x/y/z`.

They are inputs. Assigning `position_x = 3` changes the variable and nothing else; move an entity with `set_position` / `translate`, or through physics.

**Props** are the values the inspector edits. Declare them with `props()`, returning a dict of name to a default, or to a dict with `value` (or `default`) plus optional `hint` and `tab`. Each prop is a global in the script; the inspector's value is written in before each hook and read back after, so a hook may change it too.

Top-level code runs once per entity, so module globals are per-entity state:

```python
hits = 0

def on_event(name, args):
    global hits
    if name == "hit":
        hits += 1
```

## Engine functions

Every function the Lua plugin offers, under the same name, with the same arguments, as a builtin (no import). They are also in the `renzora` module, so `import renzora; renzora.play_sound(...)` works too. In brief:

| Area | Functions |
|---|---|
| Transform | `set_position`, `set_rotation`, `set_scale`, `set_scale_uniform`, `translate`, `rotate`, `look_at`, `goto_camera_preset`, `parent_set_position`, `parent_set_rotation`, `parent_translate`, `set_child_position`, `set_child_rotation`, `child_translate` |
| Keyboard and actions | `is_key_pressed`, `is_key_just_pressed`, `is_key_just_released`, `input_button_pressed`, `input_button_just_pressed`, `input_button_just_released`, `input_axis_1d`, `input_axis_2d` |
| Gamepads | `gamepad_count`, `gamepad_connected`, `gamepad_axis`, `gamepad_left_stick`, `gamepad_right_stick`, `gamepad_button`, `gamepad_button_just_pressed` |
| Touch | `touch_count`, `touch_id`, `touch_position`, `touch_start`, `touch_delta`, `touch_just_pressed`, `touch_just_released`, `pinch_delta`, `rotate_delta`, `tap_count`, `tap_position` |
| Saves | `save_set`, `save_get`, `save_has`, `save_remove`, `save_clear`, `save_keys`, `save_write`, `save_load`, `save_delete`, `save_exists`, `save_slots` |
| Audio | `play_sound`, `play_sound_looping`, `play_music`, `stop_music`, `stop_all_sounds`, `play_audio` |
| Physics | `apply_force`, `apply_impulse`, `set_velocity`, `set_gravity_scale` |
| Timers | `start_timer`, `stop_timer` |
| Debug | `print_log`, `draw_line` (and `print`, see below) |
| Rendering | `set_visibility`, `set_material_color` |
| Animation | `play_animation`, `stop_animation`, `pause_animation`, `resume_animation`, `set_animation_speed`, `seek_animation`, `get_animation_time`, `is_animation_playing`, `crossfade_animation`, `set_anim_param`, `set_anim_bool`, `trigger_anim`, `set_layer_weight` |
| Cursor and camera | `lock_cursor`, `unlock_cursor`, `screen_shake` |
| Entities and scenes | `spawn_entity`, `spawn_primitive`, `despawn_self`, `despawn_by_prefix`, `load_scene` |
| Environment | `set_sun_angles`, `set_fog` |
| Reflection | `set`, `set_on`, `get`, `get_on`, `get_component`, `get_component_on`, `get_components`, `get_components_on`, `has_component`, `has_component_on` |
| Events and network | `action`, `action_on`, `emit`, `rpc`, `http_get`, `http_post`, `net_is_server`, `net_is_client`, `net_is_connected`, `net_player_count` |
| JSON | `json_parse`, `json_encode` |
| Loading | `asset_progress`, `scene_load_state`, `is_loading`, `is_loaded` |
| Math | `vec2`, `vec3`, `color`, `lerp`, `clamp` |

Functions other plugins declare are Python functions too, with no extra step: the MySQL plugin's `mysql_query(sql, callback)`, navigation's `nav_set_destination`, `tr` for translations. A plugin installed after the editor started is picked up on the next frame.

### Where Python and Lua differ

Python's own conventions win wherever the two languages disagree:

- **Vectors are `vec3` tuples.** `get("Transform.translation")` returns a `vec3`, a tuple of three floats that also reads as `.x`, `.y`, `.z`, unpacks (`x, y, z = v`) and does vector arithmetic (`a + b`, `v * 2`, `v.length()`, `v.normalized()`, `a.dot(b)`, `a.cross(b)`). Likewise `vec2` and `color` (`.r .g .b .a`, alpha defaulting to 1). Anywhere a vector is accepted, a plain `(1, 2, 3)`, a list, or a `{"x": 1, "y": 2, "z": 3}` dict works too, and every function taking three coordinates also takes one vector: `set_position(v)`.
- **Several results come back as a tuple.** `ok, why = save_write("slot1")`; `x, y = input_axis_2d("move")` (a `vec2`, which unpacks the same way).
- **Indices start at 0.** `touch_position(0)` is the first finger, and `tap_position(0)` the first tap. Gamepads are by slot id as in Lua (0 is the first pad).
- **Argument dicts also take keywords.** `emit("boss_died", {"boss": "ogre"})` and `emit("boss_died", boss="ogre")` send the same event; likewise `action`, `action_on`, `rpc`.
- **Dicts and lists in an event become JSON.** An `emit`/`action`/`rpc` argument Lua could only send as `"<table>"` is sent as its JSON text, which the receiver can `json_parse`. A string parameter of a declared binding given a dict or list gets its JSON too, so `mysql_query_with(sql, [3], "scores")` needs no `json_encode`.
- **`json_encode(value, pretty=False)`** exists, as the inverse of `json_parse`.
- **`print` goes to the engine console**, stdout as info and stderr as errors. There is no terminal in the editor or a shipped game.
- **`set` is two things.** `set("Light.intensity", 2.0)` writes a component field, as in every language. `set()`, `set([1, 2])`, `isinstance(x, set)` and `set[int]` are still Python's set. (The reflection write is never put in `builtins`, so the standard library is unaffected.)

### Drawing: `on_draw(g)`

`g.width` and `g.height` are the surface in pixels. `g.line(x1, y1, x2, y2, color, thickness=2)`, `g.arc(cx, cy, r, start, end, color, thickness=2)`, `g.circle(cx, cy, r, color)`, `g.rect(x, y, w, h, color)`, `g.text(x, y, text, size, color)`, `g.triangle(x1, y1, x2, y2, x3, y3, color)`, `g.poly(points, color)`. A colour is `"#rrggbb"` / `"#rrggbbaa"` or a `color`. `points` is flat (`[x1, y1, x2, y2, ...]`) or a list of pairs.

### Answers that arrive later

```python
def on_ready():
    mysql_query_with("SELECT name, score FROM players WHERE level = ?", [3], "scores")
    http_get("https://example.com/motd.json", "motd")

def on_event(name, args):
    if name == "scores" and args["ok"]:
        for row in json_parse(args["rows"]):
            print(row["name"], row["score"])

def on_http(callback, status, body):
    if callback == "motd" and status == 200:
        print(json_parse(body)["text"])
```

## The standard library

The pure-Python standard library is compiled into the plugin, so these work with no files on disk: `json`, `collections`, `re`, `functools`, `itertools`, `operator`, `enum`, `typing`, `abc`, `contextlib`, `copy`, `heapq`, `bisect`, `string`, `textwrap`, `datetime`, `time`, `fractions`, `decimal`, `hashlib`, `uuid`, `weakref`, `io`, `os`, `pathlib`, and more.

`math` and `random` are the plugin's own (RustPython's would need a C compiler to install; see `src/api/math.rs`), with the everyday functions under the standard names: `math.sqrt`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `hypot`, `dist`, `floor`, `ceil`, `trunc`, `log`, `log2`, `log10`, `exp`, `pow`, `fabs`, `fmod`, `copysign`, `degrees`, `radians`, `isclose`, `isnan`, `isinf`, `isfinite`, `factorial`, `gcd`, the hyperbolic functions, and `pi`, `e`, `tau`, `inf`, `nan`; `random.random`, `uniform`, `randint`, `randrange`, `choice`, `choices`, `shuffle`, `sample`, `gauss`, `triangular`, `getrandbits`, `seed`.

## Limitations

RustPython is not CPython. Know these before choosing Python over Lua:

- **Slower.** Several times slower than CPython, and slower than Lua. Fine for game logic on a few hundred entities; not for a per-frame loop over thousands of items.
- **No C extensions.** `numpy`, `pygame` and anything else built on CPython's C API cannot load, and `pip` has nowhere to install to.
- **A subset of the standard library.** Modules whose native half the plugin does not carry fail to import: `dataclasses` and `inspect`, `statistics`, `struct`, `base64`, `csv`, `array`, `pickle`, `zlib`, `socket`. Networking goes through the engine (`http_get`, `rpc`), which also works in an exported game.
- **Python 3.12-era syntax, without `match`.** RustPython 0.4's parser does not implement structural pattern matching; use `if`/`elif`. Everything else (f-strings, walrus, `async`, type hints) parses.
- **Recursion is limited by the real stack.** RustPython runs each Python call on the thread's native stack, so the recursion limit is set before every hook from the stack actually left (on Windows; a fixed cautious limit elsewhere). Expect `RecursionError` after roughly a hundred nested calls rather than CPython's thousand. Write deep searches as loops.
- **Scripts share an interpreter.** Each entity's script has its own globals, but `sys.modules` and `builtins` are shared, as two modules of one Python program share them. Do not assign into `builtins`.
- **No reference-cycle collector.** RustPython 0.4 frees by reference counting only, so a structure that refers to itself (a node whose child points back at it) is never freed. Break such links when done, or prefer ids over references.
- **Scripts can reach the filesystem** with `open()` and `os`, as Lua's `io` can. Use `save_*` for game data: it works the same in an exported game, where the project folder does not exist.
- **Not on the web.** Not yet tried on `wasm32`; a web export runs no Python.

## Files

| | |
|---|---|
| `src/backend.rs` | the `ScriptBackend`: hooks, live instances, reload and eviction |
| `src/runtime.rs` | the interpreter (one per thread) and each script's namespace |
| `src/api/` | every engine function, one module per subject |
| `src/bindings.rs` | functions declared by other plugins |
| `src/context.rs` | the per-hook variables and `props()` |
| `src/convert.rs` | Python values to and from the engine's |
| `src/py/prelude.py` | `vec2` / `vec3` / `color`, `print` routing, `set`, the canvas |
| `src/py/random.py` | the `random` module |

**Scope:** Runtime, so it ships with the game. Not on the web.
