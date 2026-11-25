//! The live connection to the C# side: the functions it calls back into, and
//! the calls into it.
//!
//! # One runtime, one lock
//!
//! The .NET runtime is process-wide, and so is the C# side's state (the loaded
//! scripts, their instances). Hooks normally all come from the engine's one
//! scripting system, but the inspector may ask for props from another thread
//! and the background build loads a new assembly from its own, so every call
//! into C# takes [`LOCK`]. A call never re-enters (C# calls back into Rust,
//! never into C# again), so a plain mutex is enough.
//!
//! # Callbacks never unwind
//!
//! A panic unwinding out of an `extern "C"` function aborts the process, and
//! the process is the editor. Every callback body runs under `catch_unwind`
//! and reports a panic to C# as an ordinary error instead, which C# throws as
//! a `ScriptException` into the script that made the call.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use renzora::serde_json::Value;
use renzora::ScriptCommand;

use crate::abi::{self, input, op, Entries, HookContext, InvokeRequest, NativeTable, ABI_VERSION};
use crate::buffers::{self, push_command};
use crate::compile::Host;
use crate::dotnet::{Dotnet, Runtime};
use crate::frame;

static LOCK: Mutex<()> = Mutex::new(());

/// The runtime and the C# entry points, once started.
pub struct Clr {
    entries: Entries,
}

static CLR: OnceLock<Result<Clr, String>> = OnceLock::new();

/// The native table handed to `Init`. A static, because C# keeps the pointer
/// for the life of the process.
static TABLE: NativeTable = NativeTable {
    size: std::mem::size_of::<NativeTable>() as u32,
    version: ABI_VERSION,
    log: cb_log,
    transform: cb_transform,
    call: cb_call,
    input_flag: cb_input_flag,
    input_axis: cb_input_axis,
    get: cb_get,
    set_result: cb_set_result,
};

/// Start the runtime, load the host and hand it the native table, once per
/// process. A failure is remembered: a runtime that failed to start cannot be
/// started again in the same process.
pub fn clr(dotnet: &Dotnet, host: &Host) -> Result<&'static Clr, String> {
    CLR.get_or_init(|| start(dotnet, host)).as_ref().map_err(Clone::clone)
}

/// The runtime, if it has been started successfully.
pub fn running() -> Option<&'static Clr> {
    CLR.get().and_then(|r| r.as_ref().ok())
}

fn start(dotnet: &Dotnet, host: &Host) -> Result<Clr, String> {
    let runtime = Runtime::start(dotnet, &host.runtime_config)?;
    let mut ptrs = [std::ptr::null_mut(); 4];
    for (slot, name) in ptrs.iter_mut().zip(abi::ENTRY_NAMES) {
        *slot = runtime.function(&host.dll, abi::BRIDGE_TYPE, name)?;
    }
    // SAFETY: the pointers are the four `[UnmanagedCallersOnly]` methods named
    // in `ENTRY_NAMES`, whose signatures `Native.cs` declares to match.
    let entries = unsafe { Entries::from_raw(ptrs) };
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    buffers::clear_results();
    // SAFETY: `TABLE` is a static, so the pointer outlives the runtime.
    let rc = unsafe {
        (entries.init)(
            &TABLE,
            std::mem::size_of::<NativeTable>() as i32,
            std::mem::size_of::<HookContext>() as i32,
            std::mem::size_of::<InvokeRequest>() as i32,
            ABI_VERSION,
        )
    };
    if rc != 0 {
        return Err(buffers::take_result(abi::result::ERROR)
            .unwrap_or_else(|| format!("the C# host refused to start (code {rc})")));
    }
    // Leaked rather than dropped: the runtime is never shut down (see
    // `dotnet`), and `Runtime` holds nothing that needs dropping anyway.
    std::mem::forget(runtime);
    Ok(Clr { entries })
}

impl Clr {
    /// Load the scripts assembly at `dll` in place of the previous one,
    /// returning the load report (JSON).
    pub fn load(&self, dll: &Path) -> Result<String, String> {
        let path = dll.to_string_lossy();
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        buffers::clear_results();
        // SAFETY: the string outlives the call; `Load` copies it.
        let rc = unsafe { (self.entries.load)(path.as_ptr(), path.len() as i32) };
        if rc != 0 {
            return Err(buffers::take_result(abi::result::ERROR)
                .unwrap_or_else(|| format!("loading the C# scripts failed (code {rc})")));
        }
        buffers::take_result(abi::result::LOAD).ok_or_else(|| "the C# host loaded nothing".to_string())
    }

    /// Run one hook. `Ok` carries the props the hook changed, if any.
    pub fn invoke(&self, req: &InvokeRequest) -> Result<Option<String>, String> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        buffers::clear_results();
        // SAFETY: `req` and everything it points at outlive the call.
        let rc = unsafe { (self.entries.invoke)(req) };
        let props = buffers::take_result(abi::result::PROPS);
        if rc != 0 {
            return Err(buffers::take_result(abi::result::ERROR).unwrap_or_else(|| format!("the hook failed (code {rc})")));
        }
        Ok(props)
    }

    /// Drop instances: of `script` (-1 any) on `entity` (0 any).
    pub fn evict(&self, entity: u64, script: i32) {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: plain values.
        unsafe { (self.entries.evict)(entity, script) };
    }
}

// =============================================================================
// Callbacks
// =============================================================================

/// Run a callback body, turning a panic into `on_panic`.
fn guarded<R>(on_panic: R, f: impl FnOnce() -> R) -> R {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(on_panic)
}

unsafe extern "C" fn cb_log(level: u32, ptr: *const u8, len: usize) {
    guarded((), || {
        // SAFETY: C# passes a live UTF-8 buffer of `len` bytes.
        let message = unsafe { abi::read_utf8(ptr, len) };
        let level = match level {
            1 => "Warn",
            2 => "Error",
            _ => "Info",
        };
        push_command(ScriptCommand::Log {
            level: level.into(),
            message,
        });
    })
}

unsafe extern "C" fn cb_transform(op: u32, x: f32, y: f32, z: f32) {
    guarded((), || {
        let cmd = match op {
            op::SET_POSITION => ScriptCommand::SetPosition { x, y, z },
            op::SET_ROTATION => ScriptCommand::SetRotation { x, y, z },
            op::SET_SCALE => ScriptCommand::SetScale { x, y, z },
            op::TRANSLATE => ScriptCommand::Translate { x, y, z },
            op::ROTATE => ScriptCommand::Rotate { x, y, z },
            op::LOOK_AT => ScriptCommand::LookAt { x, y, z },
            op::PARENT_SET_POSITION => ScriptCommand::ParentSetPosition { x, y, z },
            op::PARENT_SET_ROTATION => ScriptCommand::ParentSetRotation { x, y, z },
            op::PARENT_TRANSLATE => ScriptCommand::ParentTranslate { x, y, z },
            op::APPLY_FORCE => ScriptCommand::ApplyForce {
                entity_id: None,
                force: [x, y, z],
            },
            op::APPLY_IMPULSE => ScriptCommand::ApplyImpulse {
                entity_id: None,
                impulse: [x, y, z],
            },
            op::SET_VELOCITY => ScriptCommand::SetVelocity {
                entity_id: None,
                velocity: [x, y, z],
            },
            _ => return,
        };
        push_command(cmd);
    })
}

/// Leave `text` for C# to copy and point `out` at it.
unsafe fn hand_out(text: String, out: *mut *const u8, out_len: *mut usize) {
    let (p, l) = buffers::keep_call_out(text);
    if !out.is_null() {
        *out = p;
    }
    if !out_len.is_null() {
        *out_len = l;
    }
}

unsafe extern "C" fn cb_call(
    name: *const u8,
    name_len: usize,
    args: *const u8,
    args_len: usize,
    out: *mut *const u8,
    out_len: *mut usize,
) -> i32 {
    let result = guarded(Err("the engine function panicked".to_string()), || {
        // SAFETY: C# passes live UTF-8 buffers of the given lengths.
        let (name, args) = unsafe { (abi::read_utf8(name, name_len), abi::read_utf8(args, args_len)) };
        let args: Vec<Value> = if args.is_empty() {
            Vec::new()
        } else {
            match renzora::serde_json::from_str::<Value>(&args) {
                Ok(Value::Array(list)) => list,
                Ok(other) => vec![other],
                Err(e) => return Err(format!("{name}(): the arguments did not arrive intact ({e})")),
            }
        };
        crate::api::call(&name, &args)
    });
    match result {
        Ok(v) => {
            hand_out(v.to_string(), out, out_len);
            0
        }
        Err(e) => {
            hand_out(e, out, out_len);
            1
        }
    }
}

unsafe extern "C" fn cb_input_flag(kind: u32, name: *const u8, len: usize) -> u32 {
    guarded(0, || {
        // SAFETY: C# passes a live UTF-8 buffer of `len` bytes.
        let name = unsafe { abi::read_utf8(name, len) };
        frame::with(|c| {
            let map = match kind {
                input::KEY_PRESSED => &c.keys_pressed,
                input::KEY_JUST_PRESSED => &c.keys_just_pressed,
                input::KEY_JUST_RELEASED => &c.keys_just_released,
                input::ACTION_PRESSED => &c.action_pressed,
                input::ACTION_JUST_PRESSED => &c.action_just_pressed,
                input::ACTION_JUST_RELEASED => &c.action_just_released,
                _ => return 0,
            };
            map.get(&name).copied().unwrap_or(false) as u32
        })
        .unwrap_or(0)
    })
}

unsafe extern "C" fn cb_input_axis(kind: u32, name: *const u8, len: usize, out: *mut f32) -> u32 {
    guarded(0, || {
        // SAFETY: C# passes a live UTF-8 buffer of `len` bytes.
        let name = unsafe { abi::read_utf8(name, len) };
        let value = frame::with(|c| match kind {
            input::AXIS_1D => c.action_axis_1d.get(&name).map(|v| [*v, 0.0]),
            input::AXIS_2D => c.action_axis_2d.get(&name).map(|v| [v.x, v.y]),
            _ => None,
        })
        .flatten();
        let found = value.is_some() as u32;
        let [x, y] = value.unwrap_or([0.0, 0.0]);
        if !out.is_null() {
            // SAFETY: C# passes room for two floats.
            unsafe {
                *out = x;
                if kind == input::AXIS_2D {
                    *out.add(1) = y;
                }
            }
        }
        found
    })
}

unsafe extern "C" fn cb_get(
    entity: *const u8,
    entity_len: usize,
    path: *const u8,
    path_len: usize,
    out: *mut *const u8,
    out_len: *mut usize,
) -> i32 {
    let result = guarded(Err("the read panicked".to_string()), || {
        // SAFETY: C# passes live UTF-8 buffers of the given lengths.
        let (entity, path) = unsafe { (abi::read_utf8(entity, entity_len), abi::read_utf8(path, path_len)) };
        let Some((component, field)) = crate::api::split_path(&path) else {
            return Err(format!("Get(): invalid path '{path}'; use 'Component.field'"));
        };
        let entity = (!entity.is_empty()).then_some(entity.as_str());
        Ok(crate::api::read_field(entity, &component, &field))
    });
    match result {
        Ok(v) => {
            hand_out(v.to_string(), out, out_len);
            0
        }
        Err(e) => {
            hand_out(e, out, out_len);
            1
        }
    }
}

unsafe extern "C" fn cb_set_result(kind: u32, ptr: *const u8, len: usize) {
    // SAFETY: C# passes a live UTF-8 buffer of `len` bytes.
    guarded((), || buffers::set_result(kind, unsafe { abi::read_utf8(ptr, len) }))
}
