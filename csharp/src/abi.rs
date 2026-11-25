//! The boundary between this plugin and the C# side, as plain data.
//!
//! Two directions, two shapes:
//!
//! * **Rust calls C#** through four `[UnmanagedCallersOnly]` static methods of
//!   `Renzora.Bridge`, fetched once from the runtime as function pointers
//!   ([`Entries`]). A hook is one [`InvokeRequest`], pointing at one
//!   [`HookContext`].
//! * **C# calls Rust** through a [`NativeTable`] of `extern "C"` function
//!   pointers, handed to `Bridge.Init` once. The C# side keeps it in a static
//!   and calls through it from inside a hook.
//!
//! Everything here is `#[repr(C)]` and mirrored field for field by
//! `host/Native.cs`. The two must agree byte for byte, and nothing would say so
//! if they drifted: a misplaced field reads as a plausible wrong number. So
//! `Init` is told the size of every struct and refuses a mismatch, which turns
//! "the entity moves at the wrong speed" into an error naming the struct.
//!
//! Strings cross as UTF-8 pointer and length, never NUL-terminated: C# strings
//! may hold a NUL, and the length is already known on both sides. A pointer is
//! valid only for the call it was passed to; whoever receives one copies it.
//!
//! This module depends on `std` alone, so the stand-alone test harness can
//! include it with `#[path]` and speak the same protocol without linking the
//! engine.

#![allow(dead_code)]

use std::ffi::c_void;

/// Bumped whenever a struct here or in `Native.cs` changes shape. `Init`
/// refuses a host built for another version, which is what a stale host
/// assembly left in the build folder by an older plugin would be.
pub const ABI_VERSION: u32 = 1;

/// Which hook an [`InvokeRequest`] runs. The C# side switches on the same
/// numbers (`Bridge.Hook`).
pub mod hook {
    pub const READY: u32 = 0;
    pub const UPDATE: u32 = 1;
    pub const FIXED_UPDATE: u32 = 2;
    pub const UI: u32 = 3;
    pub const DRAW: u32 = 4;
    pub const RPC: u32 = 5;
    pub const ANIMATION_EVENT: u32 = 6;
    pub const HTTP: u32 = 7;
    pub const PLAYER_JOINED: u32 = 8;
    pub const PLAYER_LEFT: u32 = 9;
    pub const SCENE_LOADED: u32 = 10;
    pub const SCENE_LOAD_FAILED: u32 = 11;
    pub const EVENT: u32 = 12;

    /// The engine's name for each hook, in id order, which is what
    /// `defines_hook` is asked with and what the load report lists.
    pub const NAMES: [&str; 13] = [
        "on_ready",
        "on_update",
        "on_fixed_update",
        "on_ui",
        "on_draw",
        "on_rpc",
        "on_animation_event",
        "on_http",
        "on_player_joined",
        "on_player_left",
        "on_scene_loaded",
        "on_scene_load_failed",
        "on_event",
    ];
}

/// What `set_result` is handing back. One slot per kind, each holding the last
/// value set on the thread that set it.
pub mod result {
    /// The text of an exception or a refusal.
    pub const ERROR: u32 = 0;
    /// The props a hook changed, as a JSON object of name to value.
    pub const PROPS: u32 = 1;
    /// `Load`'s report: every script file, its class, hooks and props.
    pub const LOAD: u32 = 2;
}

/// The `op` of [`NativeTable::transform`]: the commands a script issues every
/// frame, which get a call of their own instead of a trip through JSON.
pub mod op {
    pub const SET_POSITION: u32 = 0;
    pub const SET_ROTATION: u32 = 1;
    pub const SET_SCALE: u32 = 2;
    pub const TRANSLATE: u32 = 3;
    pub const ROTATE: u32 = 4;
    pub const LOOK_AT: u32 = 5;
    pub const PARENT_SET_POSITION: u32 = 6;
    pub const PARENT_SET_ROTATION: u32 = 7;
    pub const PARENT_TRANSLATE: u32 = 8;
    pub const APPLY_FORCE: u32 = 9;
    pub const APPLY_IMPULSE: u32 = 10;
    pub const SET_VELOCITY: u32 = 11;
}

/// The `kind` of [`NativeTable::input_flag`] and [`NativeTable::input_axis`].
pub mod input {
    pub const KEY_PRESSED: u32 = 0;
    pub const KEY_JUST_PRESSED: u32 = 1;
    pub const KEY_JUST_RELEASED: u32 = 2;
    pub const ACTION_PRESSED: u32 = 3;
    pub const ACTION_JUST_PRESSED: u32 = 4;
    pub const ACTION_JUST_RELEASED: u32 = 5;

    pub const AXIS_1D: u32 = 0;
    pub const AXIS_2D: u32 = 1;
}

/// Bits of [`HookContext::flags`].
pub mod flag {
    pub const HAS_PARENT: u32 = 1;
    pub const COLLIDING: u32 = 2;
    pub const NET_SERVER: u32 = 4;
    pub const NET_CONNECTED: u32 = 8;
}

/// The per-frame inputs a hook reads, by value.
///
/// Every field a script reads on most frames is here, so `Delta`, `Position`
/// and `InputMovement` are a field load on the C# side rather than a call back
/// into Rust. What is rarely read and costly to copy (the key maps, gamepads
/// beyond the first, touch) stays behind a call.
///
/// The 8-byte fields come first so neither side inserts padding between the
/// rest, which keeps the C# mirror a plain list of fields in the same order.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HookContext {
    pub entity_id: u64,
    pub elapsed: f64,
    /// The entity's `Name`, UTF-8.
    pub name_ptr: *const u8,
    pub name_len: usize,
    /// Timers that finished since the last frame, joined with `\n` (a timer
    /// name never holds one). Empty when none did, which is nearly always.
    pub timers_ptr: *const u8,
    pub timers_len: usize,
    pub delta: f32,
    pub position: [f32; 3],
    /// `x, y, z, w`.
    pub rotation: [f32; 4],
    /// Degrees, `XYZ` order: the unit the rotation commands take.
    pub euler: [f32; 3],
    pub scale: [f32; 3],
    pub input: [f32; 2],
    pub mouse: [f32; 2],
    pub mouse_delta: [f32; 2],
    pub mouse_scroll: f32,
    pub camera_yaw: f32,
    pub camera_ev: f32,
    pub project_size: [f32; 2],
    /// The first connected gamepad, as the `gamepad_*` variables are.
    pub pad_left: [f32; 2],
    pub pad_right: [f32; 2],
    pub pad_triggers: [f32; 2],
    /// Bit `i` is button `i` of `GAMEPAD_BUTTON_NAMES`.
    pub pad_buttons: u32,
    /// Bits 0 to 4 held, bits 8 to 12 pressed this frame (left, right,
    /// middle, back, forward).
    pub mouse_buttons: u32,
    /// [`flag`] bits.
    pub flags: u32,
    pub net_player_count: i32,
    pub gamepad_count: u32,
    pub parent_position: [f32; 3],
    pub health: f32,
    pub max_health: f32,
}

impl HookContext {
    /// A context with nothing pressed and no time passed, for calls made
    /// outside any hook (reading a script's props).
    pub fn idle() -> Self {
        Self {
            entity_id: 0,
            elapsed: 0.0,
            name_ptr: std::ptr::null(),
            name_len: 0,
            timers_ptr: std::ptr::null(),
            timers_len: 0,
            delta: 0.0,
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            euler: [0.0; 3],
            scale: [1.0; 3],
            input: [0.0; 2],
            mouse: [0.0; 2],
            mouse_delta: [0.0; 2],
            mouse_scroll: 0.0,
            camera_yaw: 0.0,
            camera_ev: 0.0,
            project_size: [1920.0, 1080.0],
            pad_left: [0.0; 2],
            pad_right: [0.0; 2],
            pad_triggers: [0.0; 2],
            pad_buttons: 0,
            mouse_buttons: 0,
            flags: 0,
            net_player_count: 0,
            gamepad_count: 0,
            parent_position: [0.0; 3],
            health: 0.0,
            max_health: 0.0,
        }
    }
}

/// One hook call.
///
/// The string fields carry the hook's own arguments, which differ by hook:
/// `name` is the event, RPC, UI callback, animation marker or HTTP callback
/// name, or the scene path; `args` is a JSON object of the event's arguments;
/// `text` is an HTTP body or a scene-load error; `number` is the RPC sender,
/// the UI or animation entity, the player id or the HTTP status.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct InvokeRequest {
    pub entity: u64,
    pub number: u64,
    pub ctx: *const HookContext,
    /// The inspector's values as a JSON object, or empty when they are what
    /// this instance already holds (the usual case, and the reason a hook
    /// costs no JSON at all on most frames).
    pub props_ptr: *const u8,
    pub props_len: usize,
    pub name_ptr: *const u8,
    pub name_len: usize,
    pub args_ptr: *const u8,
    pub args_len: usize,
    pub text_ptr: *const u8,
    pub text_len: usize,
    /// The script's index in the last `Load` report.
    pub script: i32,
    /// A [`hook`] id.
    pub hook: u32,
    /// The `on_draw` surface, in pixels.
    pub width: f32,
    pub height: f32,
}

/// The functions C# calls back into Rust with.
///
/// `extern "C"` on this side and `delegate* unmanaged[Cdecl]` on the other,
/// which name the same convention on every 64-bit target the plugin runs on.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NativeTable {
    /// `size_of::<NativeTable>()`, checked by `Init`.
    pub size: u32,
    pub version: u32,
    /// A console line: `level` 0 info, 1 warning, 2 error.
    pub log: unsafe extern "C" fn(level: u32, ptr: *const u8, len: usize),
    /// A transform or physics command with three numbers; see [`op`].
    pub transform: unsafe extern "C" fn(op: u32, x: f32, y: f32, z: f32),
    /// Any engine function or declared binding by name, with its arguments as
    /// a JSON array. The result (JSON, or an error's text when the return is
    /// not 0) is left in a thread-local buffer whose pointer and length are
    /// written to `out` / `out_len`, valid until the next call on this thread.
    pub call: unsafe extern "C" fn(
        name: *const u8,
        name_len: usize,
        args: *const u8,
        args_len: usize,
        out: *mut *const u8,
        out_len: *mut usize,
    ) -> i32,
    /// A key or InputMap action's state; see [`input`]. 1 or 0.
    pub input_flag: unsafe extern "C" fn(kind: u32, name: *const u8, len: usize) -> u32,
    /// An InputMap axis, into `out[0]` (1D) or `out[0..2]` (2D). Returns 1
    /// when the axis exists.
    pub input_axis: unsafe extern "C" fn(kind: u32, name: *const u8, len: usize, out: *mut f32) -> u32,
    /// A reflected field, `Component.field`, on this entity (`entity_len` 0)
    /// or a named one. JSON out, as [`Self::call`]; `null` when absent.
    pub get: unsafe extern "C" fn(
        entity: *const u8,
        entity_len: usize,
        path: *const u8,
        path_len: usize,
        out: *mut *const u8,
        out_len: *mut usize,
    ) -> i32,
    /// Hand a result back from inside an entry point; see [`result`].
    pub set_result: unsafe extern "C" fn(kind: u32, ptr: *const u8, len: usize),
}

/// `Bridge.Init(table, tableSize, contextSize, requestSize, version)`: 0 when
/// the host accepts the table, else an error left with `set_result`.
pub type InitFn = unsafe extern "C" fn(table: *const NativeTable, table_size: i32, ctx_size: i32, req_size: i32, version: u32) -> i32;
/// `Bridge.Load(path, len)`: load a compiled scripts assembly in place of the
/// last one. 0 with a [`result::LOAD`] report, else an error.
pub type LoadFn = unsafe extern "C" fn(path: *const u8, len: i32) -> i32;
/// `Bridge.Invoke(request)`: 0 ok (maybe with [`result::PROPS`]), else an error.
pub type InvokeFn = unsafe extern "C" fn(req: *const InvokeRequest) -> i32;
/// `Bridge.Evict(entity, script)`: drop instances. 0 means any entity, -1 any
/// script.
pub type EvictFn = unsafe extern "C" fn(entity: u64, script: i32);

/// The C# entry points, fetched once.
#[derive(Clone, Copy)]
pub struct Entries {
    pub init: InitFn,
    pub load: LoadFn,
    pub invoke: InvokeFn,
    pub evict: EvictFn,
}

// SAFETY: function pointers into the runtime, which is process-global and never
// unloaded. Every call through them is serialised by the backend's lock.
unsafe impl Send for Entries {}
unsafe impl Sync for Entries {}

/// The entry points' names on `Renzora.Bridge`, for fetching.
pub const BRIDGE_TYPE: &str = "Renzora.Bridge, Renzora.Scripting";
pub const ENTRY_NAMES: [&str; 4] = ["Init", "Load", "Invoke", "Evict"];

impl Entries {
    /// Build from the four pointers fetched in [`ENTRY_NAMES`] order.
    ///
    /// # Safety
    /// Each pointer must be the `[UnmanagedCallersOnly]` method of that name,
    /// whose signature `Native.cs` declares to match the type above.
    pub unsafe fn from_raw(ptrs: [*mut c_void; 4]) -> Self {
        Self {
            init: std::mem::transmute::<*mut c_void, InitFn>(ptrs[0]),
            load: std::mem::transmute::<*mut c_void, LoadFn>(ptrs[1]),
            invoke: std::mem::transmute::<*mut c_void, InvokeFn>(ptrs[2]),
            evict: std::mem::transmute::<*mut c_void, EvictFn>(ptrs[3]),
        }
    }
}

/// A Rust string as a pointer and length for one call.
pub fn utf8(s: &str) -> (*const u8, usize) {
    if s.is_empty() {
        (std::ptr::null(), 0)
    } else {
        (s.as_ptr(), s.len())
    }
}

/// Copy a pointer and length received from C# into a `String`. Invalid UTF-8
/// (C# never sends it, but a bug might) becomes U+FFFD rather than a panic.
///
/// # Safety
/// `ptr` must be valid for `len` bytes, or `len` must be 0.
pub unsafe fn read_utf8(ptr: *const u8, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    String::from_utf8_lossy(std::slice::from_raw_parts(ptr, len)).into_owned()
}
