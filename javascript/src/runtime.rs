//! The interpreter, and each script's scope in it.
//!
//! # One interpreter per thread, one closure per script
//!
//! The Lua plugin gives every `(entity, script)` pair its own VM. A Boa
//! `Context` costs about half a millisecond and a few hundred kilobytes to build
//! (it sets up every builtin), and a fresh realm inside one still costs a third
//! of a millisecond; either per entity would put a hitch on every spawn.
//! JavaScript already has the right unit of isolation instead: a closure. So
//! there is one `Context`, and a script is compiled once into a *factory*, a
//! function whose body is the script's text. Calling the factory runs the
//! script's top level in a fresh function scope and hands back its hooks, so
//! every `(entity, script)` pair gets its own top-level state for about sixty
//! microseconds: `let count = 0` is per entity, exactly as in Lua.
//!
//! ```text
//! (function () {<the script, unchanged, starting on line 1>
//! ;return { on_ready: typeof on_ready === "function" ? on_ready : undefined, ... };
//! })
//! ```
//!
//! What the scripts share is the global object and the builtins, the way two
//! files loaded into one page share `window`. A script that assigns an
//! undeclared name, or patches `Array.prototype`, does it for everyone.
//!
//! "One" interpreter per THREAD, because a Boa `Context` is not `Send`. The
//! engine runs every hook from one exclusive system, which Bevy always runs on
//! the thread that owns the schedule, so in practice one interpreter does all
//! the work; another thread asking for a script's props gets its own.
//!
//! # Never dropped
//!
//! Each thread's runtime is leaked rather than kept in a `thread_local!` that
//! drops it. Boa's garbage collector keeps its own thread-local state, and the
//! order thread-locals are destroyed in at thread exit is unspecified: a
//! `Context` dropped after the collector it belongs to is a use-after-free. A
//! runtime is built once per thread that ever runs a script, which is one or
//! two threads for the life of the editor.
//!
//! # The standard streams
//!
//! Nothing here touches stdin, stdout or stderr, which a windowed game does
//! not have (the Python plugin crashed in the real game on exactly that). Boa
//! has no `console` and no `print`; its only `println!`s are behind its `trace`
//! feature and an optimizer-statistics flag, neither of which this plugin
//! turns on; oxc prints nothing. `console` is the prelude's, and writes to the
//! engine console.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use boa_engine::builtins::promise::{OperationType, Promise, PromiseState};
use boa_engine::context::HostHooks;
use boa_engine::object::builtins::JsPromise;
use boa_engine::{js_string, Context, JsError, JsObject, JsValue, Script, Source};

use crate::convert::{self, Prelude};
use crate::transpile;

/// Runs once per interpreter, before any script: the vector classes,
/// `console`, how values are shown, and the `on_draw` canvas.
const PRELUDE: &str = include_str!("js/prelude.js");

/// The hooks a script may define, in the order of [`Instance::hooks`].
/// `props` is not a hook but is found the same way.
pub const HOOKS: &[&str] = &[
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
    "props",
];

/// How many entries of an error's call stack are kept. Boa's default is 50,
/// which for a runaway recursion is fifty identical lines in the console.
const BACKTRACE_LIMIT: usize = 8;

/// One script on one entity: the functions its top level defined.
pub struct Instance {
    pub hooks: Vec<Option<JsObject>>,
    /// What it was built from, for the line map its errors are reported
    /// through.
    pub factory: std::rc::Rc<Factory>,
    /// The backend's token for this instance. When the backend's registry
    /// holds a different one (or none), the instance was evicted or reloaded
    /// and is rebuilt.
    pub token: u64,
}

impl Instance {
    pub fn hook(&self, name: &str) -> Option<&JsObject> {
        let i = HOOKS.iter().position(|h| *h == name)?;
        self.hooks.get(i)?.as_ref()
    }
}

/// A compiled script, kept per path so a hundred entities running one script
/// compile it once.
pub struct Factory {
    pub func: JsObject,
    /// For TypeScript, the printed-to-original line map (see
    /// [`transpile::Prepared::lines`]).
    pub lines: Option<Vec<u32>>,
}

struct Compiled {
    version: u64,
    factory: Result<std::rc::Rc<Factory>, String>,
}

pub struct Runtime {
    pub ctx: Context,
    pub instances: HashMap<(u64, String), Instance>,
    compiled: HashMap<String, Compiled>,
    /// The `renzora` object: the API, the bindings, and the natives the
    /// prelude uses.
    pub module: JsObject,
    /// The plugin's own function names, which a declared binding may override.
    pub api_names: Vec<String>,
    /// Names a prop may not take: the API and the context variables, which
    /// live on the same global object props are written to.
    pub reserved: Vec<String>,
    /// The bindings currently on the global object.
    pub installed_bindings: Vec<String>,
    /// Which binding generation the global object currently holds.
    pub bindings_generation: Option<u64>,
    /// The backend's eviction count when this thread last pruned.
    pub eviction_epoch: u64,
}

thread_local! {
    static RUNTIME: RefCell<Option<&'static RefCell<Runtime>>> = const { RefCell::new(None) };
}

/// Run `f` with this thread's interpreter, building it on first use.
///
/// `Err` when the runtime is already in use further up this thread's stack (a
/// hook that somehow re-entered the backend), rather than the panic a second
/// `borrow_mut` would be.
pub fn with<R>(f: impl FnOnce(&mut Runtime) -> Result<R, String>) -> Result<R, String> {
    let cell = RUNTIME.with(|slot| -> Result<&'static RefCell<Runtime>, String> {
        let mut slot = slot.borrow_mut();
        if let Some(cell) = *slot {
            return Ok(cell);
        }
        let cell: &'static RefCell<Runtime> = Box::leak(Box::new(RefCell::new(Runtime::new()?)));
        *slot = Some(cell);
        Ok(cell)
    })?;
    let mut rt = cell
        .try_borrow_mut()
        .map_err(|_| "the JavaScript runtime was re-entered from inside a hook".to_string())?;
    f(&mut rt)
}

impl Runtime {
    fn new() -> Result<Self, String> {
        let hooks = Rc::new(Hooks::default());
        let mut ctx = Context::builder()
            .host_hooks(hooks.clone())
            .build()
            .map_err(|e| format!("JavaScript setup failed: {e}"))?;
        ctx.insert_data(Rejections(hooks));
        ctx.runtime_limits_mut().set_backtrace_limit(BACKTRACE_LIMIT);

        let module = JsObject::with_object_proto(ctx.intrinsics());
        crate::api::register(&mut ctx, &module);
        let api_names: Vec<String> = own_names(&mut ctx, &module);
        let global = ctx.global_object();
        global
            .set(js_string!("renzora"), module.clone(), false, &mut ctx)
            .map_err(|e| format!("JavaScript setup failed: {e}"))?;
        crate::context::install(&mut ctx, &global).map_err(|e| format!("JavaScript setup failed: {e}"))?;

        // The prelude defines the classes the API hands back, so it runs
        // before anything is copied onto the global object.
        let prelude = Script::parse(
            Source::from_bytes(PRELUDE).with_path(Path::new("renzora-prelude.js")),
            None,
            &mut ctx,
        )
        .and_then(|s| s.evaluate(&mut ctx))
        .and_then(|f| {
            let f = f
                .as_callable()
                .ok_or_else(|| JsError::from_opaque(js_string!("the prelude is not a function").into()))?;
            f.call(&JsValue::undefined(), &[module.clone().into()], &mut ctx)
        })
        .map_err(|e| format!("JavaScript setup failed in the prelude: {e}"))?;
        let parts = prelude
            .as_object()
            .ok_or_else(|| "JavaScript setup failed: the prelude returned nothing".to_string())?;
        let mut part = |name: &str| -> Result<JsObject, String> {
            parts
                .get(convert::key(name), &mut ctx)
                .ok()
                .and_then(|v| v.as_object())
                .ok_or_else(|| format!("JavaScript setup failed: the prelude has no `{name}`"))
        };
        let data = Prelude {
            vec2: part("vec2")?,
            vec3: part("vec3")?,
            color: part("color")?,
            canvas: part("canvas")?,
            inspect: part("inspect")?,
        };
        ctx.insert_data(data);

        // Everything public onto the global object, so a script calls the API
        // with no import. Not writable-protected: a script may shadow or
        // reassign a name for itself, as it may any global.
        let mut public = Vec::new();
        for name in own_names(&mut ctx, &module) {
            if name.starts_with('_') {
                continue;
            }
            let value = module.get(convert::key(&name), &mut ctx).unwrap_or_default();
            global
                .set(convert::key(&name), value, false, &mut ctx)
                .map_err(|e| format!("JavaScript setup failed: {e}"))?;
            public.push(name);
        }
        let mut reserved = public;
        reserved.extend(crate::context::names());
        reserved.push("renzora".into());
        reserved.push("console".into());

        Ok(Self {
            ctx,
            instances: HashMap::new(),
            compiled: HashMap::new(),
            module,
            api_names,
            reserved,
            installed_bindings: Vec::new(),
            bindings_generation: None,
            eviction_epoch: 0,
        })
    }

    /// Compile `source` for `path`, reusing the last compile while `version`
    /// has not moved. A syntax error is cached too, so a broken script is
    /// reported every frame without being re-parsed every frame.
    pub fn factory(
        &mut self,
        path: &str,
        version: u64,
        source: impl FnOnce() -> Option<String>,
    ) -> Result<std::rc::Rc<Factory>, String> {
        if let Some(c) = self.compiled.get(path) {
            if c.version == version {
                return c.factory.clone();
            }
        }
        let name = file_name(path);
        let factory = match source() {
            None => Err(format!("could not read {path}")),
            Some(source) => self.compile(path, &name, &source).map(std::rc::Rc::new),
        };
        self.compiled.insert(
            path.to_string(),
            Compiled {
                version,
                factory: factory.clone(),
            },
        );
        factory
    }

    fn compile(&mut self, path: &str, name: &str, source: &str) -> Result<Factory, String> {
        let prepared = transpile::prepare(path, name, source).map_err(|e| format!("{name}: {e}"))?;
        let hooks: Vec<String> = HOOKS
            .iter()
            .map(|h| format!("{h}: typeof {h} === \"function\" ? {h} : undefined"))
            .collect();
        // The script starts on the wrapper's first line, so its line numbers
        // are the file's own. The newline before `;return` ends a trailing
        // `//` comment, and the `;` a final statement written without one.
        let wrapped = format!(
            "(function () {{{}\n;return {{ {} }};\n}})",
            prepared.code,
            hooks.join(", ")
        );
        let lines = prepared.lines.clone();
        let describe = |e: JsError| match &lines {
            Some(map) => transpile::map_positions(&e.to_string(), name, map),
            None => e.to_string(),
        };
        let script = match Script::parse(Source::from_bytes(&wrapped).with_path(Path::new(name)), None, &mut self.ctx) {
            Ok(script) => script,
            Err(e) => {
                // TypeScript reaching here means oxc printed something Boa
                // cannot parse, and only Boa's text says what.
                let better = if lines.is_none() {
                    transpile::syntax_error(name, &prepared.code)
                } else {
                    None
                };
                let text = better.unwrap_or_else(|| describe(e).replace(" (native)", ""));
                return Err(format!("{name}: {text}"));
            }
        };
        let func = script
            .evaluate(&mut self.ctx)
            .map_err(|e| format!("{name}: {}", describe(e)))?
            .as_callable()
            .ok_or_else(|| format!("{name}: the script did not compile to a function"))?;
        Ok(Factory { func, lines })
    }

    /// Run a script's top level in a fresh scope and collect its hooks.
    pub fn instantiate(&mut self, factory: &Factory, name: &str) -> Result<Vec<Option<JsObject>>, String> {
        let result = factory.func.call(&JsValue::undefined(), &[], &mut self.ctx);
        let result = settle(&mut self.ctx, result);
        let hooks = result.map_err(|e| format!("{name}: {}", describe(&e, name, factory)))?;
        let hooks = hooks
            .as_object()
            .ok_or_else(|| format!("{name}: the script's top level did not finish"))?;
        let mut out = Vec::with_capacity(HOOKS.len());
        for h in HOOKS {
            let f = hooks
                .get(convert::key(h), &mut self.ctx)
                .ok()
                .and_then(|v| v.as_callable());
            out.push(f);
        }
        Ok(out)
    }
}

/// Run the promise jobs a hook queued (`async` functions, `.then`
/// callbacks), so they finish inside the hook that started them and their
/// commands go out with its own. `result` is the hook's; a job's error is
/// reported only when the hook itself succeeded.
///
/// A promise still rejected with no handler once the jobs have run is logged
/// as an error, as a browser logs "Uncaught (in promise)". Without this an
/// exception thrown inside an `async` function would vanish without a trace,
/// because the hook that started it returned normally.
pub fn settle(ctx: &mut Context, result: Result<JsValue, JsError>) -> Result<JsValue, JsError> {
    let jobs = ctx.run_jobs();
    let rejected = ctx
        .get_data::<Rejections>()
        .map(|r| std::mem::take(&mut *r.0.rejected.borrow_mut()))
        .unwrap_or_default();
    for promise in rejected {
        let Ok(promise) = JsPromise::from_object(promise) else {
            continue;
        };
        if let PromiseState::Rejected(reason) = promise.state() {
            let text = convert::display(&reason, ctx);
            crate::buffers::push_command(renzora::ScriptCommand::Log {
                level: "Error".into(),
                message: format!("Uncaught (in promise) {text}"),
            });
        }
    }
    let value = result?;
    jobs?;
    Ok(value)
}

/// The promises rejected with nobody listening, collected for [`settle`].
///
/// Boa asks the host about every rejection (the spec's
/// `HostPromiseRejectionTracker`): `Reject` when a promise is rejected with no
/// handler, `Handle` when a handler is attached later. What is left after the
/// jobs have run is what nobody will ever handle.
#[derive(Default)]
struct Hooks {
    rejected: RefCell<Vec<JsObject>>,
}

impl HostHooks for Hooks {
    fn promise_rejection_tracker(&self, promise: &JsObject<Promise>, operation: OperationType, _context: &mut Context) {
        let promise = promise.clone().upcast();
        let mut rejected = self.rejected.borrow_mut();
        match operation {
            OperationType::Reject => rejected.push(promise),
            OperationType::Handle => rejected.retain(|p| !JsObject::equals(p, &promise)),
        }
    }
}

/// The `Hooks` the `Context` was built with, kept where [`settle`] can reach
/// them: `Context::host_hooks` hands them back only as `dyn HostHooks`.
struct Rejections(Rc<Hooks>);

// =============================================================================
// The native stack
// =============================================================================

/// What one JavaScript call can cost on the native stack, rounded well up.
///
/// Boa runs a JavaScript-to-JavaScript call inside its own loop, but a call
/// that goes through a native function (an `Array.prototype.map` callback, a
/// getter, a `sort` comparator) recurses on the thread's real stack. Measured
/// on a release build: 512 such calls fit in 512 KB and not in 256 KB, so
/// about 1 KB each; erring high only lowers the limit.
const BYTES_PER_CALL: usize = 2 * 1024;
/// Kept free below the deepest call, for the engine code a native calls into
/// and for the error a `RuntimeLimitError` builds on its way out.
const RESERVE: usize = 128 * 1024;
/// A floor, so a hook entered with very little stack left still runs simple
/// code, and the ceiling, Boa's own default.
const MIN_LIMIT: usize = 64;
const MAX_LIMIT: usize = 512;

/// Set Boa's recursion limit from the stack this thread has left right now.
///
/// Without it a script recursing through a callback overflows the stack and
/// takes the whole editor down, where Boa would have thrown a
/// `RuntimeLimitError`: Boa's default limit of 512 calls assumes a thread with
/// more stack than the engine's main thread may have left by the time it runs
/// scripts. Called on entry to every hook, because the interpreter is entered
/// from different depths (the frame loop, a props parse from a UI system).
pub fn guard_stack(ctx: &mut Context) {
    let limit = match stack_remaining() {
        Some(left) => (left.saturating_sub(RESERVE) / BYTES_PER_CALL).clamp(MIN_LIMIT, MAX_LIMIT),
        // No way to ask on this platform, where every thread the engine runs
        // hooks on has at least the 2 MB a Rust thread starts with (its main
        // thread is 8 MB on Linux and macOS): Boa's default is safe there.
        None => MAX_LIMIT,
    };
    ctx.runtime_limits_mut().set_recursion_limit(limit);
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

/// An error as the text a script author needs: the error, then where it was
/// thrown, with TypeScript positions mapped back to the file the author wrote.
pub fn describe(e: &JsError, name: &str, factory: &Factory) -> String {
    let text = e.to_string();
    match &factory.lines {
        Some(map) => transpile::map_positions(&text, name, map),
        None => text,
    }
}

/// The file name errors are reported under: `player.ts`, not the full path,
/// which the console would wrap and the author already knows.
pub fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("script")
        .to_string()
}

/// The string-keyed own property names of `obj`.
fn own_names(ctx: &mut Context, obj: &JsObject) -> Vec<String> {
    obj.own_property_keys(ctx)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|k| match k {
            boa_engine::property::PropertyKey::String(s) => Some(s.to_std_string_lossy()),
            _ => None,
        })
        .collect()
}
