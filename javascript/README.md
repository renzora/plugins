# JavaScript

JavaScript and TypeScript scripting for Renzora. Attach a `.js` or `.ts` script to an entity and the engine runs it, with the same hooks and the same engine functions as Lua and Python.

The interpreter is [Boa](https://github.com/boa-dev/boa), and TypeScript goes through [oxc](https://github.com/oxc-project/oxc) on the way in; both are compiled into the plugin, so nobody has to install Node, Deno or `tsc`: not you, not your players. An exported game carries them like any other plugin.

```js
// scripts/spinner.js
function props() {
  return { speed: { value: 90, hint: "degrees per second" } };
}

function on_update() {
  rotate(0, speed * delta, 0);
}
```

```ts
// scripts/spinner.ts
export {};
declare let speed: number;

function props() {
  return { speed: { value: 90, hint: "degrees per second" } };
}

function on_update(): void {
  rotate(0, speed * delta, 0);
}
```

The Assets panel's Add menu and the hierarchy's Attach submenu gain **JavaScript Script** and **TypeScript Script**, each with a starter.

## Hooks

Define the ones you need; a script that defines none of them is valid. A hook may declare fewer parameters than it is given (`function on_rpc(name, args)` ignores the sender), as in every JavaScript function.

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

`args` is a plain object. A hook is any function the script's top level defines under that name: `function on_update() {}`, `const on_update = () => {}`, or `export function on_update() {}`.

`async` hooks work, and so does any promise a hook starts: the plugin runs the promise jobs before the hook returns, so `await` on something already settled (or on another `async` function) finishes in the same frame, and its commands go out with the hook's. There is no event loop beyond that, so nothing waits on time: use `start_timer` and `timers_finished`. A promise rejected with no handler is logged as an error, `Uncaught (in promise) ...`, as a browser logs it.

## Variables

Before every hook the frame's inputs are readable as globals, under the same names as in Lua: `delta`, `elapsed`, `position_x/y/z`, `rotation_x/y/z` (radians), `scale_x/y/z`, `input_x/y`, `mouse_x/y`, `mouse_delta_x/y`, `mouse_left/right/middle`, `mouse_left_just_pressed`, `mouse_right_just_pressed`, `mouse_scroll`, `camera_yaw`, `camera_ev`, `project_width/height`, `gamepad_left_x` (and the rest of the first pad, `gamepad_south` and the other buttons), `self_entity_id`, `self_entity_name`, `is_colliding`, `timers_finished` (an array), `self_health`, `self_max_health`, `has_parent`, `parent_position_x/y/z`.

They are read-only inputs, read from the engine at the moment you read them. Assigning `position_x = 3` does nothing (and throws a `TypeError` in strict code, such as a class body); move an entity with `set_position` / `translate`, or through physics. A script can declare its own variable of the same name (`let delta = 0`), which shadows the global as any declaration does.

**Props** are the values the inspector edits. Declare them with `props()`, returning an object of name to a default, or to an object with `value` (or `default`) plus optional `hint`, `tab` and `type`. Each prop is a global for the length of a hook: the inspector's value is written in before it and read back after, so a hook may change it too (`speed *= 2`). A number is a float prop; write `{value: 3, type: "int"}` for a whole number, because JavaScript writes `3` and `3.0` the same. In TypeScript, tell the compiler a prop exists with `declare let speed: number;` (it compiles to nothing).

Top-level code runs once per entity, so top-level variables are per-entity state:

```js
let hits = 0;

function on_event(name, args) {
  if (name === "hit") hits++;
}
```

## Engine functions

Every function the Lua plugin offers, under the same name, with the same arguments, as a global (no import). They are also properties of the `renzora` object, so `renzora.play_sound(...)` works too. In brief:

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
| Debug | `print_log`, `draw_line` (and `console`, see below) |
| Rendering | `set_visibility`, `set_material_color` |
| Animation | `play_animation`, `stop_animation`, `pause_animation`, `resume_animation`, `set_animation_speed`, `seek_animation`, `get_animation_time`, `is_animation_playing`, `crossfade_animation`, `set_anim_param`, `set_anim_bool`, `trigger_anim`, `set_layer_weight` |
| Cursor and camera | `lock_cursor`, `unlock_cursor`, `screen_shake` |
| Entities and scenes | `spawn_entity`, `spawn_primitive`, `despawn_self`, `despawn_by_prefix`, `load_scene` |
| Environment | `set_sun_angles`, `set_fog` |
| Reflection | `set`, `set_on`, `get`, `get_on`, `get_component`, `get_component_on`, `get_components`, `get_components_on`, `has_component`, `has_component_on` |
| Events and network | `action`, `action_on`, `emit`, `rpc`, `http_get`, `http_post`, `net_is_server`, `net_is_client`, `net_is_connected`, `net_player_count` |
| JSON | `json_parse`, `json_encode` (and JavaScript's own `JSON`) |
| Loading | `asset_progress`, `scene_load_state`, `is_loading`, `is_loaded` |
| Math | `vec2`, `vec3`, `color`, `lerp`, `clamp` (and JavaScript's own `Math`) |

An engine function checks its arguments and throws a `TypeError` naming the function and the parameter (`set_position(): argument 'z' must be a number, not string`) rather than turning a typo into `NaN`.

Functions other plugins declare are JavaScript functions too, with no extra step: the MySQL plugin's `mysql_query_with(sql, params, callback)`, MongoDB's `mongo_find_with`, navigation's `nav_set_destination`, `tr` for translations. A plugin installed after the editor started is picked up on the next frame. A declared function named like one of JavaScript's own globals (`parseInt`, `JSON`, ...) is not allowed to hide it: call it as `renzora.parseInt(...)`, and the console says so.

### Where JavaScript and Lua differ

JavaScript's own conventions win wherever the two languages disagree:

- **Vectors are objects.** `get("Transform.translation")` returns a `Vec3`: an object with `x`, `y` and `z`, which also destructures (`const [x, y, z] = v`). JavaScript has no operator overloading, so `a + b` cannot add vectors; the arithmetic is methods that return a new vector: `a.add(b)`, `a.sub(b)`, `v.mul(2)`, `v.div(2)`, `v.neg()`, `a.dot(b)`, `a.cross(b)`, `v.length()`, `v.normalized()`, `a.distance(b)`, `a.lerp(b, t)`, `a.equals(b)`. Likewise `Vec2`, and `Color` (`.r .g .b .a`, alpha defaulting to 1). `vec3(x, y, z)` makes one (no `new` needed). Anywhere a vector is accepted, a plain `{x: 1, y: 2, z: 3}` or `[1, 2, 3]` works too, and every function taking three coordinates also takes one vector: `set_position(v)`.
- **Several results come back as one object or vector.** `const { ok, error } = save_write("slot1")`; `const { x, y } = input_axis_2d("move")` (a `Vec2`, which also destructures as `[x, y]`). `ok` is a boolean and `error` the reason, or `null`.
- **Indices start at 0.** `touch_position(0)` is the first finger, and `tap_position(0)` the first tap. Gamepads are by slot id as in Lua (0 is the first pad).
- **"Nothing" is `null`.** Where Lua returns `nil` (`get` of a missing field, `touch_id` of a missing finger, `json_parse` of text that is not JSON), these return `null`. `null` or `undefined` sent in an event arrives elsewhere as Lua's `"nil"`.
- **Objects and arrays in an event become JSON.** An `emit` / `action` / `rpc` argument Lua could only send as `"<table>"` is sent as its JSON text, which the receiver can `json_parse`. A string parameter of a declared binding given an object or array gets its JSON too, so `mysql_query_with(sql, [3], "scores")` and `mongo_find_with("players", {level: {$gt: 3}}, "found")` need no `json_encode`.
- **Numbers are one type.** A whole number is sent to the engine as an integer and any other as a float, as Lua and Python send `5` and `5.5`; a float field takes an integer without complaint, so `set("Light.intensity", 2)` works.
- **`json_encode(value, pretty = false)`** exists, as the inverse of `json_parse`. `JSON.parse` and `JSON.stringify` are there too; `json_parse` returns `null` on bad text where `JSON.parse` throws.
- **`console.log` goes to the engine console**: `log`, `info` and `debug` as info, `warn` as a warning, `error` as an error, plus `assert`, `trace`, `count` and `time`. Values print the way Node prints them (`{ a: 1 }`, `[1, 2]`, `Vec3(1, 2, 3)`), and `%s` / `%d` / `%o` substitutions work. There is no terminal in the editor or a shipped game, and the plugin never touches the process's standard streams.
- **The console runs JavaScript.** An expression typed at the editor's console is evaluated in the global scope, and a `var` or function declared there stays for the next line, as in a browser.

### Drawing: `on_draw(g)`

`g.width` and `g.height` are the surface in pixels. `g.line(x1, y1, x2, y2, color, thickness = 2)`, `g.arc(cx, cy, r, start, end, color, thickness = 2)`, `g.circle(cx, cy, r, color)`, `g.rect(x, y, w, h, color)`, `g.text(x, y, text, size, color)`, `g.triangle(x1, y1, x2, y2, x3, y3, color)`, `g.poly(points, color)`. A colour is `"#rrggbb"` / `"#rrggbbaa"`, a `Color`, or `[r, g, b, a]`. `points` is flat (`[x1, y1, x2, y2, ...]`) or an array of points (`[[x, y], ...]` or `[{x, y}, ...]`).

### Answers that arrive later

```js
function on_ready() {
  mysql_query_with("SELECT name, score FROM players WHERE level = ?", [3], "scores");
  http_get("https://example.com/motd.json", "motd");
}

function on_event(name, args) {
  if (name === "scores" && args.ok) {
    for (const row of json_parse(args.rows)) console.log(row.name, row.score);
  }
}

function on_http(callback, status, body) {
  if (callback === "motd" && status === 200) console.log(json_parse(body).text);
}
```

## TypeScript

A `.ts` script is TypeScript: types, interfaces, generics, `enum`, `namespace`, parameter properties, `as`, `!`, `satisfies` and the rest. The plugin strips the types and runs the JavaScript that is left, exactly as `tsc` would emit it; it does **not** type-check. That is your editor's job, and the plugin ships what it needs to do it: `renzora.d.ts`, the declarations of every engine function, hook argument and context variable.

To use it, copy `renzora.d.ts` from this plugin's folder into your project and add a `tsconfig.json` beside it:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2023"],
    "strict": true,
    "noEmit": true,
    "types": []
  },
  "include": ["renzora.d.ts", "scripts/**/*.ts"]
}
```

`"types": []` and the `lib` without `DOM` keep browser and Node globals (`window`, `setTimeout`, `require`) out of autocomplete, since none of them exist here.

Two habits make the editor's checking match what runs:

- **Start each script with `export {};`** (the TypeScript starter does). Without an `import` or `export`, TypeScript treats a file as a global script, and two scripts that both declare `function on_update` or `const speed` collide in the editor although they never meet at run time. The plugin accepts `export` anywhere and ignores it: `export function on_update()` is just `function on_update()`.
- **Declare props and declared bindings** the compiler cannot know about: `declare let speed: number;` for a prop, `declare function mysql_query_with(sql: string, params: unknown, callback: string): void;` for a plugin's function. Or call the latter as `renzora.mysql_query_with(...)`, which the declarations type loosely.

An error in a `.ts` script names the line you wrote, not a line of the generated JavaScript.

`import` is refused with an error in both languages: a script is not a module, there is nothing to import, and every engine function is already a global. `import type` and other type-only imports are fine.

## Limitations

Boa is not V8. Know these before choosing JavaScript over Lua:

- **Slower than a browser.** Boa interprets; there is no JIT. A typical `on_update` costs about 2 microseconds per entity, and building a script's instance for a new entity about 40 (measured on a desktop build): fine for game logic on a few hundred entities, not for a per-frame loop over tens of thousands of items.
- **Not every newer feature.** Boa passes most of the ECMAScript test suite, including classes and private fields, `async`/`await`, generators, `BigInt`, `Proxy`, `Map`/`Set` and the new `Set` methods, iterator helpers, typed arrays, `Object.groupBy`, RegExp named groups and lookbehind. Missing: `Intl` (not compiled in: its data would add megabytes to every exported game), `Temporal`, `structuredClone`, `Array.fromAsync`, decorators (`@dec`, in JavaScript or TypeScript, either flavour: a syntax error), and `using` declarations (they run, but never call the disposer: do not rely on them).
- **No browser or Node APIs.** No `window`, `document`, `fetch`, `setTimeout`, `setInterval`, `require` or `process`. Networking goes through the engine (`http_get`, `rpc`), timers through `start_timer`, which also work in an exported game.
- **The first line of an error is Boa's.** An uncaught `Error` reports, after its message, the position of the call that led to the throw; the `at` lines under it are exact.
- **Scripts share a global object.** Each entity's script has its own top-level scope, but the global object and the builtins are shared, as two `<script>` files on one page share `window`. Assigning an undeclared name, or patching `Array.prototype`, does it for every script.
- **Recursion is limited.** A runaway recursion throws `RuntimeLimitError` after a few hundred calls (fewer when the engine's thread has little stack left), rather than overflowing the stack. Write deep searches as loops.
- **Entity ids are numbers.** `self_entity_id` and the `entity` arguments are JavaScript numbers, exact for every id a scene will ever hold (up to 2^53).
- **Not on the web.** Not yet tried on `wasm32`; a web export runs no JavaScript.

## Files

| | |
|---|---|
| `src/backend.rs` | the `ScriptBackend`: hooks, live instances, reload and eviction |
| `src/runtime.rs` | the interpreter (one per thread), each script's scope, promises, the stack guard |
| `src/transpile.rs` | TypeScript to JavaScript, `export` removal, syntax error positions |
| `src/api/` | every engine function, one module per subject |
| `src/bindings.rs` | functions declared by other plugins |
| `src/context.rs` | the per-hook variables and `props()` |
| `src/convert.rs` | JavaScript values to and from the engine's |
| `src/js/prelude.js` | `Vec2` / `Vec3` / `Color`, `console`, how values print, the canvas |
| `renzora.d.ts` | TypeScript declarations for your editor |

**Scope:** Runtime, so it ships with the game. Not on the web.
