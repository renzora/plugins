//! The interpreter, and each script's namespace in it.
//!
//! # One interpreter per thread, one namespace per script
//!
//! The Lua plugin gives every `(entity, script)` pair its own VM, which in Lua
//! costs microseconds. A RustPython interpreter takes milliseconds and several
//! megabytes to build (it sets up every builtin type and module), so that shape
//! would put a hitch on every spawn and a VM's worth of memory on every entity.
//! Python already has the right unit of isolation instead: a module namespace.
//! So there is one interpreter, and every `(entity, script)` pair gets its own
//! globals dict in it. Each entity's script still has its own top-level state
//! (`count = 0` is per entity, exactly as in Lua); what the scripts share is the
//! interpreter's `builtins` and `sys.modules`, the way two modules of one Python
//! program do.
//!
//! "One" interpreter per THREAD, because RustPython is built here without its
//! `threading` feature (see the manifest), so its objects are `Rc` and must
//! never cross threads. The engine runs every hook from one exclusive system,
//! which Bevy always runs on the thread that owns the schedule, so in practice
//! one interpreter does all the work; another thread asking for a script's
//! props simply gets its own.
//!
//! # The native stack
//!
//! RustPython 0.4 runs each Python call as a Rust call, a few kilobytes of the
//! thread's real stack apiece, and has no check of its own against running out:
//! a script recursing too deep would overflow the stack and take the whole
//! editor down, where CPython raises `RecursionError`. So before every hook the
//! recursion limit is set from the stack actually left (see [`guard_stack`]),
//! and a runaway recursion raises like CPython's does.

use std::cell::RefCell;
use std::collections::HashMap;

use rustpython_vm::builtins::{PyBaseExceptionRef, PyCode, PyDict, PyDictRef};
use rustpython_vm::compiler::Mode;
use rustpython_vm::function::FuncArgs;
use rustpython_vm::scope::Scope;
use rustpython_vm::{AsObject, Interpreter, Py, PyObjectRef, PyRef, PyResult, Settings, VirtualMachine};

/// Runs once per interpreter, before any script: the vector types, `print`
/// into the console, the `set` shim and the `on_draw` canvas.
const PRELUDE: &str = include_str!("py/prelude.py");
/// The `random` module. See `api::math` for why it is not RustPython's.
const RANDOM: &str = include_str!("py/random.py");

/// One script on one entity.
pub struct Instance {
    pub globals: PyDictRef,
    /// The backend's token for this instance. When the backend's registry
    /// holds a different one (or none), the instance was evicted or reloaded
    /// and is rebuilt.
    pub token: u64,
}

/// A compiled script, kept per path so a hundred entities running one script
/// compile it once.
struct Compiled {
    version: u64,
    code: Result<PyRef<PyCode>, String>,
}

pub struct Runtime {
    pub interp: Interpreter,
    pub instances: HashMap<(u64, String), Instance>,
    compiled: HashMap<String, Compiled>,
    /// The `renzora` module's dict: the API, the bindings, and the natives the
    /// prelude uses.
    pub module: PyDictRef,
    /// The prelude's `set` shim, put into every script's globals.
    set_shim: PyObjectRef,
    /// The plugin's own function names, which a declared binding may override.
    pub api_names: Vec<String>,
    /// Which binding generation `builtins` currently holds.
    pub bindings_generation: Option<u64>,
    /// The backend's eviction count when this thread last pruned.
    pub eviction_epoch: u64,
}

thread_local! {
    static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

/// Run `f` with this thread's interpreter, building it on first use.
///
/// `Err` when the runtime is already in use further up this thread's stack (a
/// hook that somehow re-entered the backend), rather than the panic a second
/// `borrow_mut` would be.
pub fn with<R>(f: impl FnOnce(&mut Runtime) -> Result<R, String>) -> Result<R, String> {
    RUNTIME.with(|cell| {
        let mut slot = cell
            .try_borrow_mut()
            .map_err(|_| "the Python runtime was re-entered from inside a hook".to_string())?;
        if slot.is_none() {
            *slot = Some(Runtime::new()?);
        }
        match slot.as_mut() {
            Some(rt) => f(rt),
            None => Err("the Python runtime could not be built".into()),
        }
    })
}

/// Give the process a stdin, stdout and stderr wherever it has none, pointed
/// at `NUL`.
///
/// RustPython opens descriptors 0, 1 and 2 as `sys.stdin`/`stdout`/`stderr`
/// while it builds an interpreter, and panics ("essential initialization
/// failed", `OSError 9: The handle is invalid`) when one is not open, with no
/// setting to skip it. A windowed game has none: the editor's Play reads the
/// game's output but gives it no input, and a shipped game started from
/// Explorer has no console at all. The smoke tests ran in a terminal, which is
/// why they never saw it. Nothing is lost by `NUL`: the prelude sends `print`
/// to the engine console, not to these.
///
/// Only descriptors that are not open are touched, so a game run from a
/// terminal keeps its real ones.
#[cfg(windows)]
fn ensure_std_streams() {
    use std::os::raw::c_int;
    use std::os::windows::io::IntoRawHandle;

    // The C runtime's descriptor table, which is what RustPython's `io` reads.
    extern "C" {
        fn _get_osfhandle(fd: c_int) -> isize;
        fn _open_osfhandle(handle: isize, flags: c_int) -> c_int;
        fn _dup2(from: c_int, to: c_int) -> c_int;
        fn _close(fd: c_int) -> c_int;
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        for fd in 0..3 {
            // SAFETY: querying a descriptor number has no preconditions; -1 or
            // -2 means it is not open or not attached to a stream.
            if unsafe { _get_osfhandle(fd) } >= 0 {
                continue;
            }
            let file = if fd == 0 {
                std::fs::File::open("NUL")
            } else {
                std::fs::OpenOptions::new().write(true).open("NUL")
            };
            let Ok(file) = file else { continue };
            // SAFETY: the handle is freshly opened and owned by nobody else;
            // `_open_osfhandle` takes ownership, and the temporary descriptor
            // is closed after `_dup2` has copied it into `fd`.
            unsafe {
                let tmp = _open_osfhandle(file.into_raw_handle() as isize, 0);
                if tmp >= 0 {
                    _dup2(tmp, fd);
                    _close(tmp);
                }
            }
        }
    });
}

/// Unix processes start with 0, 1 and 2 open, even when they lead nowhere.
#[cfg(not(windows))]
fn ensure_std_streams() {}

impl Runtime {
    fn new() -> Result<Self, String> {
        let mut settings = Settings::default();
        // An embedded interpreter must not take SIGINT from the editor, read
        // PYTHON* variables from whoever launched it, or go looking for a
        // `site` directory on a machine that has no Python.
        settings.install_signal_handlers = false;
        settings.ignore_environment = true;
        settings.import_site = false;
        settings.isolated = true;
        settings.user_site_directory = false;
        ensure_std_streams();
        let interp = Interpreter::with_init(settings, |vm| {
            vm.add_frozen(rustpython_pylib::FROZEN_STDLIB);
        });

        let built = interp.enter(|vm| -> PyResult<(PyDictRef, PyObjectRef, Vec<String>)> {
            let dict = vm.ctx.new_dict();
            crate::api::register(vm, &dict);
            crate::bindings::register(vm, &dict);
            let api_names: Vec<String> = (&dict)
                .into_iter()
                .filter_map(|(k, _)| crate::convert::string(&k))
                .collect();
            let modules = vm.sys_module.get_attr("modules", vm)?;
            let module = vm.new_module("renzora", dict.clone(), None);
            modules.set_item("renzora", module.into(), vm)?;
            crate::api::register_math_module(vm)?;

            // The prelude defines the types the API hands back, so it runs
            // before anything is copied into builtins.
            let prelude = run_module(vm, "_renzora_prelude", PRELUDE)?;
            for name in ["vec2", "vec3", "color", "_Canvas", "_flush"] {
                dict.set_item(name, prelude.get_item(name, vm)?, vm)?;
            }
            let set_shim = prelude.get_item("set", vm)?;

            // Everything public into builtins, so a script calls the API with
            // no import. Never `set`: see `api::reflect`.
            let builtins = vm.builtins.dict();
            for (k, v) in &dict {
                let Some(name) = crate::convert::string(&k) else {
                    continue;
                };
                if name.starts_with('_') || name == "set" {
                    continue;
                }
                builtins.set_item(name.as_str(), v, vm)?;
            }

            let random = run_module(vm, "random", RANDOM)?;
            let random_module = vm.new_module("random", random, None);
            modules.set_item("random", random_module.into(), vm)?;
            Ok((dict, set_shim, api_names))
        });
        let (module, set_shim, api_names) =
            built.map_err(|e| interp.enter(|vm| format!("Python setup failed: {}", describe(vm, &e))))?;

        Ok(Self {
            interp,
            instances: HashMap::new(),
            compiled: HashMap::new(),
            module,
            set_shim,
            api_names,
            bindings_generation: None,
            eviction_epoch: 0,
        })
    }

    /// Compile `source` for `path`, reusing the last compile while `version`
    /// has not moved. A syntax error is cached too, so a broken script is
    /// reported every frame without being re-parsed every frame.
    pub fn code(
        &mut self,
        path: &str,
        version: u64,
        source: impl FnOnce() -> Option<String>,
    ) -> Result<PyRef<PyCode>, String> {
        if let Some(c) = self.compiled.get(path) {
            if c.version == version {
                return c.code.clone();
            }
        }
        let source = source().ok_or_else(|| format!("could not read {path}"))?;
        let code = self.interp.enter(|vm| {
            vm.compile(&source, Mode::Exec, path.to_string())
                .map_err(|e| describe(vm, &vm.new_syntax_error(&e, Some(&source))))
        });
        self.compiled.insert(
            path.to_string(),
            Compiled {
                version,
                code: code.clone(),
            },
        );
        code
    }

    /// A fresh namespace for a script: `builtins`, a `__name__` and `__file__`
    /// like a module's, and the `set` shim.
    pub fn new_globals(&self, vm: &VirtualMachine, path: &str) -> PyResult<PyDictRef> {
        let g = vm.ctx.new_dict();
        g.set_item("__builtins__", vm.builtins.clone().into(), vm)?;
        let stem = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("script");
        g.set_item("__name__", vm.ctx.new_str(stem).into(), vm)?;
        g.set_item("__file__", vm.ctx.new_str(path).into(), vm)?;
        g.set_item("set", self.set_shim.clone(), vm)?;
        Ok(g)
    }
}

/// Run `source` as a module body in a fresh namespace and return it.
fn run_module(vm: &VirtualMachine, name: &str, source: &str) -> PyResult<PyDictRef> {
    let globals = vm.ctx.new_dict();
    globals.set_item("__name__", vm.ctx.new_str(name).into(), vm)?;
    let code = vm
        .compile(source, Mode::Exec, format!("<{name}>"))
        .map_err(|e| vm.new_syntax_error(&e, Some(source)))?;
    vm.run_code_obj(code, Scope::with_builtins(None, globals.clone(), vm))?;
    Ok(globals)
}

/// Run a compiled script body in `globals`.
pub fn exec(vm: &VirtualMachine, code: PyRef<PyCode>, globals: &PyDictRef) -> PyResult<()> {
    vm.run_code_obj(code, Scope::with_builtins(None, globals.clone(), vm))
        .map(|_| ())
}

/// Call `func` with `args`, trimmed to what it accepts.
///
/// Lua drops extra arguments silently, so a Lua `function on_event(name)` is a
/// valid handler for a hook that passes two. Python raises `TypeError`
/// instead, and a script written `def on_rpc(name, args):` (ignoring the
/// sender) would fail every call. So a plain Python function is handed only as
/// many positional arguments as it declares, unless it takes `*args`; anything
/// else is called with all of them.
pub fn call_trimmed(vm: &VirtualMachine, func: &PyObjectRef, mut args: Vec<PyObjectRef>) -> PyResult<PyObjectRef> {
    if let Some(accepts) = positional_capacity(vm, func) {
        args.truncate(accepts);
    }
    func.call(FuncArgs::from(args), vm)
}

/// How many positional arguments a Python function takes, or `None` when it
/// takes `*args` or is not a plain function (a builtin, a callable object).
fn positional_capacity(vm: &VirtualMachine, func: &PyObjectRef) -> Option<usize> {
    const CO_VARARGS: i64 = 0x04;
    let code = func.get_attr("__code__", vm).ok()?;
    let flags = code.get_attr("co_flags", vm).ok()?;
    if crate::convert::integer(&flags, vm)? & CO_VARARGS != 0 {
        return None;
    }
    let count = code.get_attr("co_argcount", vm).ok()?;
    let count = crate::convert::integer(&count, vm)?;
    // A bound method's `self` is already supplied.
    let bound = func.get_attr("__self__", vm).is_ok_and(|s| !vm.is_none(&s));
    usize::try_from(count - bound as i64).ok()
}

/// An exception as the text a script author needs: the traceback, then the
/// error. The engine logs it against the script.
pub fn describe(vm: &VirtualMachine, exc: &PyBaseExceptionRef) -> String {
    let mut text = String::new();
    if vm.write_exception(&mut text, exc).is_err() || text.trim().is_empty() {
        return exc.as_object().class().name().to_string();
    }
    text.trim_end().to_string()
}

/// `obj`, if it is something a hook can be called on.
pub fn callable(obj: &Option<PyObjectRef>) -> Option<&PyObjectRef> {
    obj.as_ref().filter(|f| f.is_callable())
}

/// Call the prelude's `_flush`, so a partial `print` line reaches the console.
pub fn flush(vm: &VirtualMachine, module: &Py<PyDict>) {
    if let Ok(Some(flush)) = module.get_item_opt("_flush", vm) {
        let _ = flush.call((), vm);
    }
}

// =============================================================================
// The native stack
// =============================================================================

/// What one Python-level call costs on the native stack, rounded well up.
///
/// Measured on RustPython 0.4 release builds at about 3.5 KB for a plain
/// recursive function; a call that goes through a builtin, a method lookup or
/// a comprehension on the way uses more. Erring high only lowers the limit.
const BYTES_PER_CALL: usize = 8 * 1024;
/// Kept free below the deepest Python call, for the native code it calls into
/// (string formatting, the engine's reads) and for the traceback a
/// `RecursionError` builds on its way out.
const RESERVE: usize = 128 * 1024;
/// A floor, so a hook entered with very little stack left still runs simple
/// code, and the ceiling, CPython's own default.
const MIN_LIMIT: usize = 48;
const MAX_LIMIT: usize = 1000;

/// Set the recursion limit from the stack this thread has left right now.
///
/// Called on entry to every hook, because the same interpreter is entered from
/// different depths (the frame loop, a props parse from a UI system) and the
/// stack left differs between them.
pub fn guard_stack(vm: &VirtualMachine) {
    let limit = match stack_remaining() {
        Some(left) => (left.saturating_sub(RESERVE) / BYTES_PER_CALL).clamp(MIN_LIMIT, MAX_LIMIT),
        // No way to ask on this platform: assume a 2 MB thread with half of it
        // already used, which is a cautious answer for every thread the engine
        // runs hooks on (its main thread is 8 MB on Linux and macOS).
        None => (1024 * 1024 - RESERVE) / BYTES_PER_CALL,
    };
    vm.recursion_limit.set(limit);
}

/// Bytes between here and the bottom of this thread's stack.
#[cfg(windows)]
fn stack_remaining() -> Option<usize> {
    #[link(name = "kernel32")]
    extern "system" {
        // Windows 8 and later; every Windows the engine supports.
        fn GetCurrentThreadStackLimits(low: *mut usize, high: *mut usize);
    }
    let (mut low, mut high) = (0usize, 0usize);
    // SAFETY: the function writes two `usize`-sized values through the
    // pointers it is given and has no other effect.
    unsafe { GetCurrentThreadStackLimits(&mut low, &mut high) };
    let here = &low as *const usize as usize;
    (low != 0 && here > low).then(|| here - low)
}

#[cfg(not(windows))]
fn stack_remaining() -> Option<usize> {
    None
}
