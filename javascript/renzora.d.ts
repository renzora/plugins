// Type declarations for Renzora's JavaScript and TypeScript scripts.
//
// For your editor only: the engine never reads this file. Copy it into your
// project (next to `scripts/`, or anywhere a tsconfig.json includes) and every
// engine function, hook argument and context variable autocompletes and
// type-checks. See the plugin's README for a tsconfig.json to go with it.
//
// Everything here is a global, as it is at run time: no import is needed or
// possible. Functions other plugins declare (MySQL's `mysql_query_with`,
// navigation's `nav_set_destination`, ...) are globals too, but which ones
// exist depends on the plugins installed, so they are not listed; declare the
// ones you call yourself, or call them through `renzora`, which accepts any
// name:
//
//     declare function mysql_query_with(sql: string, params: unknown, callback: string): void;
//     renzora.nav_set_destination(vec3(1, 0, 4));

// ---------------------------------------------------------------------------
// Vectors and colours
// ---------------------------------------------------------------------------

/** Anything a vector parameter accepts: a vector, an `{x, y, z}` object, or an array. */
type Vec3Like = Vec3 | { x: number; y: number; z?: number } | [number, number, number] | number[];
type Vec2Like = Vec2 | { x: number; y: number } | [number, number] | number[];
/** A colour: a `Color`, any `{r, g, b, a?}` object, a 3 or 4 number array, or (for drawing) `"#rrggbb"` / `"#rrggbbaa"`. */
type ColorLike = Color | { r: number; g: number; b: number; a?: number } | number[];

/** A 2D vector. Arithmetic is methods (JavaScript has no operator overloading); each returns a new vector. Destructures: `const [x, y] = v`. */
declare class Vec2 implements Iterable<number> {
  constructor(x?: number, y?: number);
  constructor(from: Vec2Like);
  x: number;
  y: number;
  add(o: Vec2Like): Vec2;
  sub(o: Vec2Like): Vec2;
  mul(s: number): Vec2;
  div(s: number): Vec2;
  neg(): Vec2;
  dot(o: Vec2Like): number;
  length(): number;
  lengthSquared(): number;
  normalized(): Vec2;
  distance(o: Vec2Like): number;
  lerp(o: Vec2Like, t: number): Vec2;
  equals(o: Vec2Like): boolean;
  clone(): Vec2;
  toArray(): [number, number];
  toString(): string;
  [Symbol.iterator](): Iterator<number>;
}

/** A 3D vector, as `get("Transform.translation")` returns. Every function taking a position accepts one in place of three numbers. */
declare class Vec3 implements Iterable<number> {
  constructor(x?: number, y?: number, z?: number);
  constructor(from: Vec3Like);
  x: number;
  y: number;
  z: number;
  add(o: Vec3Like): Vec3;
  sub(o: Vec3Like): Vec3;
  mul(s: number): Vec3;
  div(s: number): Vec3;
  neg(): Vec3;
  dot(o: Vec3Like): number;
  cross(o: Vec3Like): Vec3;
  length(): number;
  lengthSquared(): number;
  normalized(): Vec3;
  distance(o: Vec3Like): number;
  lerp(o: Vec3Like, t: number): Vec3;
  equals(o: Vec3Like): boolean;
  clone(): Vec3;
  toArray(): [number, number, number];
  toString(): string;
  [Symbol.iterator](): Iterator<number>;
}

/** An RGBA colour, channels 0..1; alpha defaults to 1. */
declare class Color implements Iterable<number> {
  constructor(r?: number, g?: number, b?: number, a?: number);
  constructor(from: ColorLike);
  r: number;
  g: number;
  b: number;
  a: number;
  toArray(): [number, number, number, number];
  toString(): string;
  [Symbol.iterator](): Iterator<number>;
}

declare function vec2(x?: number, y?: number): Vec2;
declare function vec2(from: Vec2Like): Vec2;
declare function vec3(x?: number, y?: number, z?: number): Vec3;
declare function vec3(from: Vec3Like): Vec3;
declare function color(r?: number, g?: number, b?: number, a?: number): Color;
declare function color(from: ColorLike): Color;

declare function lerp(a: number, b: number, t: number): number;
declare function clamp(value: number, min: number, max: number): number;

// ---------------------------------------------------------------------------
// Context variables: read-only inputs, refreshed before every hook
// ---------------------------------------------------------------------------

/** Seconds since the last frame (the fixed step in `on_fixed_update`). */
declare const delta: number;
/** Seconds since play started. */
declare const elapsed: number;
declare const position_x: number;
declare const position_y: number;
declare const position_z: number;
/** Euler angles in radians, XYZ order. */
declare const rotation_x: number;
declare const rotation_y: number;
declare const rotation_z: number;
declare const scale_x: number;
declare const scale_y: number;
declare const scale_z: number;
/** The movement input (WASD / left stick), each -1..1. */
declare const input_x: number;
declare const input_y: number;
declare const mouse_x: number;
declare const mouse_y: number;
declare const mouse_delta_x: number;
declare const mouse_delta_y: number;
declare const mouse_left: boolean;
declare const mouse_right: boolean;
declare const mouse_middle: boolean;
declare const mouse_left_just_pressed: boolean;
declare const mouse_right_just_pressed: boolean;
declare const mouse_scroll: number;
declare const camera_yaw: number;
/** The live scene exposure, EV-100. */
declare const camera_ev: number;
declare const project_width: number;
declare const project_height: number;
/** The first connected gamepad. Use the gamepad functions for others. */
declare const gamepad_left_x: number;
declare const gamepad_left_y: number;
declare const gamepad_right_x: number;
declare const gamepad_right_y: number;
declare const gamepad_left_trigger: number;
declare const gamepad_right_trigger: number;
declare const gamepad_south: boolean;
declare const gamepad_east: boolean;
declare const gamepad_west: boolean;
declare const gamepad_north: boolean;
declare const gamepad_l1: boolean;
declare const gamepad_r1: boolean;
declare const gamepad_l2: boolean;
declare const gamepad_r2: boolean;
declare const gamepad_select: boolean;
declare const gamepad_start: boolean;
declare const gamepad_l3: boolean;
declare const gamepad_r3: boolean;
declare const gamepad_dpad_up: boolean;
declare const gamepad_dpad_down: boolean;
declare const gamepad_dpad_left: boolean;
declare const gamepad_dpad_right: boolean;
/** This entity, as `Entity::to_bits()`. */
declare const self_entity_id: number;
declare const self_entity_name: string;
declare const is_colliding: boolean;
/** Names of the timers that finished since the last hook. */
declare const timers_finished: string[];
declare const self_health: number;
declare const self_max_health: number;
declare const has_parent: boolean;
declare const parent_position_x: number;
declare const parent_position_y: number;
declare const parent_position_z: number;

// ---------------------------------------------------------------------------
// Hooks (define the ones you need; all optional)
// ---------------------------------------------------------------------------

/** What `props()` returns: name to a bare default, or a spec. */
type PropValue = number | boolean | string | Vec2 | Vec3 | Color;
type PropSpec = PropValue | {
  value?: PropValue;
  default?: PropValue;
  hint?: string;
  tab?: string;
  /** `"int"` for a whole-number prop; a number is a float otherwise. */
  type?: "int" | "float";
};
/** The value an action, event or RPC argument arrives as. */
type ActionValue = number | boolean | string | Vec3;
type ActionArgs = Record<string, unknown>;

// The hook signatures, for reference (declare yours with these shapes):
//   function props(): Record<string, PropSpec>
//   function on_ready(): void
//   function on_update(): void
//   function on_fixed_update(): void
//   function on_ui(name: string, args: Record<string, ActionValue>, entity: number): void
//   function on_draw(g: Canvas): void
//   function on_rpc(name: string, args: Record<string, ActionValue>, sender: number): void
//   function on_animation_event(name: string, entity: number): void
//   function on_http(callback: string, status: number, body: string): void
//   function on_player_joined(id: number): void
//   function on_player_left(id: number): void
//   function on_scene_loaded(path: string): void
//   function on_scene_load_failed(path: string, error: string): void
//   function on_event(name: string, args: Record<string, ActionValue>): void

/** The `g` that `on_draw(g)` receives. Every method draws one shape this frame. */
interface Canvas {
  readonly width: number;
  readonly height: number;
  line(x1: number, y1: number, x2: number, y2: number, color?: ColorLike | string, thickness?: number): void;
  arc(cx: number, cy: number, r: number, start: number, end: number, color?: ColorLike | string, thickness?: number): void;
  circle(cx: number, cy: number, r: number, color?: ColorLike | string): void;
  rect(x: number, y: number, w: number, h: number, color?: ColorLike | string): void;
  text(x: number, y: number, text: unknown, size: number, color?: ColorLike | string): void;
  triangle(x1: number, y1: number, x2: number, y2: number, x3: number, y3: number, color?: ColorLike | string): void;
  /** A convex polygon: flat `[x1, y1, x2, y2, ...]` or an array of points. */
  poly(points: number[] | Vec2Like[], color?: ColorLike | string): void;
}

// ---------------------------------------------------------------------------
// Transform
// ---------------------------------------------------------------------------

declare function set_position(x: number, y: number, z: number): void;
declare function set_position(v: Vec3Like): void;
/** Euler angles in degrees. */
declare function set_rotation(x: number, y: number, z: number): void;
declare function set_rotation(v: Vec3Like): void;
declare function set_scale(x: number, y: number, z: number): void;
declare function set_scale(v: Vec3Like): void;
declare function set_scale_uniform(scale: number): void;
declare function translate(x: number, y: number, z: number): void;
declare function translate(v: Vec3Like): void;
/** Turn by Euler angles in degrees. */
declare function rotate(x: number, y: number, z: number): void;
declare function rotate(v: Vec3Like): void;
declare function look_at(x: number, y: number, z: number): void;
declare function look_at(v: Vec3Like): void;
declare function goto_camera_preset(name: string): void;
declare function parent_set_position(x: number, y: number, z: number): void;
declare function parent_set_position(v: Vec3Like): void;
declare function parent_set_rotation(x: number, y: number, z: number): void;
declare function parent_set_rotation(v: Vec3Like): void;
declare function parent_translate(x: number, y: number, z: number): void;
declare function parent_translate(v: Vec3Like): void;
declare function set_child_position(name: string, x: number, y: number, z: number): void;
declare function set_child_position(name: string, v: Vec3Like): void;
declare function set_child_rotation(name: string, x: number, y: number, z: number): void;
declare function set_child_rotation(name: string, v: Vec3Like): void;
declare function child_translate(name: string, x: number, y: number, z: number): void;
declare function child_translate(name: string, v: Vec3Like): void;

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/** Key names are Bevy's `KeyCode` names: "KeyW", "Space", "ShiftLeft", ... */
declare function is_key_pressed(key: string): boolean;
declare function is_key_just_pressed(key: string): boolean;
declare function is_key_just_released(key: string): boolean;
/** Named actions from the project's input map. */
declare function input_button_pressed(name: string): boolean;
declare function input_button_just_pressed(name: string): boolean;
declare function input_button_just_released(name: string): boolean;
declare function input_axis_1d(name: string): number;
declare function input_axis_2d(name: string): Vec2;

type GamepadAxis = "left_x" | "left_y" | "right_x" | "right_y" | "left_trigger" | "right_trigger";
type GamepadButton =
  | "south" | "east" | "west" | "north" | "l1" | "r1" | "l2" | "r2"
  | "select" | "start" | "l3" | "r3" | "dpad_up" | "dpad_down" | "dpad_left" | "dpad_right";
declare function gamepad_count(): number;
/** Pads by stable slot id; 0 is the first. */
declare function gamepad_connected(pad: number): boolean;
declare function gamepad_axis(pad: number, axis: GamepadAxis): number;
declare function gamepad_left_stick(pad: number): Vec2;
declare function gamepad_right_stick(pad: number): Vec2;
declare function gamepad_button(pad: number, button: GamepadButton): boolean;
declare function gamepad_button_just_pressed(pad: number, button: GamepadButton): boolean;

declare function touch_count(): number;
/** Fingers are indexed from 0, in the order they went down. `null` for no such finger. */
declare function touch_id(index: number): number | null;
declare function touch_position(index: number): Vec2;
declare function touch_start(index: number): Vec2;
declare function touch_delta(index: number): Vec2;
declare function touch_just_pressed(index: number): boolean;
declare function touch_just_released(index: number): boolean;
declare function pinch_delta(): number;
declare function rotate_delta(): number;
declare function tap_count(): number;
declare function tap_position(index: number): Vec2;

// ---------------------------------------------------------------------------
// Saves
// ---------------------------------------------------------------------------

type SaveValue = number | boolean | string | Vec2 | Vec3 | Color;
/** The outcome of a slot operation. */
interface SaveResult {
  ok: boolean;
  /** Why it failed, or `null`. */
  error: string | null;
}
declare function save_set(key: string, value: SaveValue): void;
declare function save_get<T extends SaveValue = SaveValue>(key: string, fallback?: T): T | null;
declare function save_has(key: string): boolean;
declare function save_remove(key: string): boolean;
declare function save_clear(): void;
declare function save_keys(): string[];
declare function save_write(slot: string): SaveResult;
declare function save_load(slot: string): SaveResult;
declare function save_delete(slot: string): SaveResult;
declare function save_exists(slot: string): boolean;
declare function save_slots(): { name: string; saved_at: number }[];

// ---------------------------------------------------------------------------
// Audio and animation
// ---------------------------------------------------------------------------

declare function play_sound(path: string, volume?: number, bus?: string): void;
declare function play_sound_looping(path: string, volume?: number): void;
declare function play_music(path: string, volume?: number, fade_in?: number): void;
declare function stop_music(fade_out?: number): void;
declare function stop_all_sounds(): void;
/** A one-shot from an entity's AudioPlayer; no name means this entity. */
declare function play_audio(entity?: string): void;

declare function play_animation(name: string, looping?: boolean, speed?: number): void;
declare function stop_animation(): void;
declare function pause_animation(): void;
declare function resume_animation(): void;
declare function set_animation_speed(speed: number): void;
declare function seek_animation(time: number): void;
declare function get_animation_time(): number;
declare function is_animation_playing(): boolean;
declare function crossfade_animation(name: string, duration: number, looping?: boolean): void;
declare function set_anim_param(name: string, value: number): void;
declare function set_anim_bool(name: string, value: boolean): void;
declare function trigger_anim(name: string): void;
declare function set_layer_weight(layer: string, weight: number): void;

// ---------------------------------------------------------------------------
// Physics, timers, rendering, cursor and camera
// ---------------------------------------------------------------------------

declare function apply_force(x: number, y: number, z: number): void;
declare function apply_force(v: Vec3Like): void;
declare function apply_impulse(x: number, y: number, z: number): void;
declare function apply_impulse(v: Vec3Like): void;
declare function set_velocity(x: number, y: number, z: number): void;
declare function set_velocity(v: Vec3Like): void;
declare function set_gravity_scale(scale: number): void;

declare function start_timer(name: string, duration: number, repeat?: boolean): void;
declare function stop_timer(name: string): void;

declare function set_visibility(visible: boolean): void;
declare function set_material_color(r: number, g: number, b: number, a?: number): void;
declare function set_material_color(color: ColorLike): void;

declare function lock_cursor(): void;
declare function unlock_cursor(): void;
declare function screen_shake(intensity: number, duration: number): void;

// ---------------------------------------------------------------------------
// Entities, scenes, environment
// ---------------------------------------------------------------------------

declare function spawn_entity(name: string): void;
/** `kind` is any shape id: "cube", "sphere", "wall", ... Without a colour the shape keeps its own tint. */
declare function spawn_primitive(name: string, kind: string, x: number, y: number, z: number, r?: number, g?: number, b?: number): void;
declare function spawn_primitive(name: string, kind: string, position: Vec3Like, color?: ColorLike): void;
declare function despawn_self(): void;
declare function despawn_by_prefix(prefix: string): void;
declare function load_scene(path: string): void;
declare function set_sun_angles(azimuth: number, elevation: number): void;
declare function set_fog(enabled: boolean, start: number, end: number): void;

// ---------------------------------------------------------------------------
// Reflection: any reflected component field, by path
// ---------------------------------------------------------------------------

type FieldValue = number | boolean | string | Vec3 | Color;
/** `set("Light.intensity", 2)`, on this entity. */
declare function set(path: string, value: FieldValue | Vec2Like | Vec3Like | ColorLike): void;
declare function set_on(entity: string, path: string, value: FieldValue | Vec2Like | Vec3Like | ColorLike): void;
/** `null` when the entity has no such field. */
declare function get<T extends FieldValue = FieldValue>(path: string): T | null;
declare function get_on<T extends FieldValue = FieldValue>(entity: string, path: string): T | null;
declare function get_component(component: string): Record<string, FieldValue> | null;
declare function get_component_on(entity: string, component: string): Record<string, FieldValue> | null;
declare function get_components(): string[];
declare function get_components_on(entity: string): string[];
declare function has_component(component: string): boolean;
declare function has_component_on(entity: string, component: string): boolean;

interface AssetProgress {
  state: "idle" | "loading" | "done";
  total_files: number;
  loaded_files: number;
  total_bytes: number;
  loaded_bytes: number;
  fraction: number;
  elapsed_secs: number;
  current_path: string | null;
}
interface SceneLoadState {
  phase: "idle" | "loading" | "ready" | "failed";
  progress: number;
  current_path: string | null;
}
declare function asset_progress(): AssetProgress | null;
declare function scene_load_state(): SceneLoadState | null;
declare function is_loading(): boolean;
declare function is_loaded(): boolean;

// ---------------------------------------------------------------------------
// Events, network, HTTP, JSON, logging
// ---------------------------------------------------------------------------

/** Nested objects and arrays in `args` travel as JSON text. */
declare function action(name: string, args?: ActionArgs): void;
declare function action_on(entity: string, name: string, args?: ActionArgs): void;
/** Every script's `on_event(name, args)` fires next frame. */
declare function emit(name: string, args?: ActionArgs): void;
/** A networked RPC; peers receive it in `on_rpc(name, args, sender)`. */
declare function rpc(name: string, args?: ActionArgs): void;
/** The answer arrives as `on_http(callback, status, body)`. */
declare function http_get(url: string, callback?: string): void;
/** An object or array `body` is sent as JSON. */
declare function http_post(url: string, body?: unknown, callback?: string): void;
declare function net_is_server(): boolean;
declare function net_is_client(): boolean;
declare function net_is_connected(): boolean;
declare function net_player_count(): number;

/** `null` when the text is not JSON (where `JSON.parse` throws). */
declare function json_parse(text: string): any;
declare function json_encode(value: unknown, pretty?: boolean): string;

declare function print_log(message: unknown): void;
/** A line in the world for `duration` seconds (0: this frame). */
declare function draw_line(sx: number, sy: number, sz: number, ex: number, ey: number, ez: number, duration?: number): void;
declare function draw_line(start: Vec3Like, end: Vec3Like, duration?: number): void;

/** `console.log` and friends write to the engine console: log/info/debug as info, warn as a warning, error as an error. */
interface Console {
  log(...data: unknown[]): void;
  info(...data: unknown[]): void;
  debug(...data: unknown[]): void;
  warn(...data: unknown[]): void;
  error(...data: unknown[]): void;
  trace(...data: unknown[]): void;
  assert(condition: unknown, ...data: unknown[]): void;
  count(label?: string): void;
  countReset(label?: string): void;
  time(label?: string): void;
  timeEnd(label?: string): void;
}
declare var console: Console;

/** Every engine function and every declared binding, by name. */
declare const renzora: {
  [name: string]: any;
};
