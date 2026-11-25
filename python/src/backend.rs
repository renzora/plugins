//! The `ScriptBackend`: which scripts are live, and the hooks.
//!
//! The interpreter and the scripts' namespaces are thread-local (see
//! [`crate::runtime`] for why), but `needs_reload` and `evict` may be asked from
//! any thread, so the record of which instances are live is kept here, behind a
//! lock, as a [`Registry`]. An instance in a thread's runtime is valid exactly
//! while the registry holds the same token for its key; evicting removes the
//! entry, and the runtime rebuilds (or prunes) the instance the next time it
//! looks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use renzora::script_extension::Binding;
use renzora::{
    FileReader, ScriptActionValue as ActionValue, ScriptBackend, ScriptCommand, ScriptContext,
    ScriptValue, ScriptVariableDefinition as VarDef, ScriptVariables,
};
use rustpython_vm::builtins::PyDictRef;
use rustpython_vm::{PyObjectRef, PyResult, VirtualMachine};

use crate::buffers::{drain_commands, drain_draws};
use crate::runtime::{self, Instance, Runtime};
use crate::{context, convert, frame};

/// The hooks `defines_hook` can answer for.
const KNOWN_HOOKS: &[&str] = &[
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

#[derive(Default)]
struct Live {
    token: u64,
    /// The modification time of the source this instance was built from.
    version: u64,
}

#[derive(Default)]
struct Registry {
    live: HashMap<(u64, String), Live>,
    next_token: u64,
    /// Bumped on every eviction, so a runtime knows to prune.
    epoch: u64,
}

/// The binding list the runtimes were built against, and a counter for
/// noticing it moved.
#[derive(Default)]
struct BindingCache {
    bindings: Vec<Binding>,
    generation: u64,
}

#[derive(Default)]
pub struct PythonBackend {
    registry: Mutex<Registry>,
    scripts_folder: Mutex<PathBuf>,
    file_reader: Mutex<Option<FileReader>>,
    bindings: Mutex<BindingCache>,
    /// Per script path, the hooks it defined when last built.
    defined_hooks: Mutex<HashMap<String, Vec<&'static str>>>,
}

impl PythonBackend {
    /// The script's source, through the VFS reader when the host set one (a
    /// shipped game reads out of its `.rpak`), else straight off disk.
    fn source_of(&self, path: &Path) -> Option<String> {
        if let Ok(reader) = self.file_reader.lock() {
            if let Some(reader) = reader.as_ref() {
                if let Some(text) = reader(path) {
                    return Some(text);
                }
            }
        }
        std::fs::read_to_string(path).ok()
    }

    /// Modification time, which is what an edit moves. Not a hash of the text:
    /// that would mean reading the whole file to learn nothing changed.
    /// Unreadable (an rpak entry) reads as 0, which never invalidates.
    fn version_of(path: &Path) -> u64 {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }

    /// Bring the cached binding list up to date with what the engine declares,
    /// returning its generation.
    ///
    /// Compared by count, as in the Lua plugin: extensions register while the
    /// `App` is built and never change afterwards, so the one transition to
    /// catch is "none yet" to "all of them" on the first hook after startup.
    fn sync_bindings(&self, ctx: &ScriptContext) -> u64 {
        let Ok(mut cache) = self.bindings.lock() else {
            return 0;
        };
        if let Some(extensions) = ctx.extensions() {
            if cache.bindings.len() != extensions.bindings().len() {
                cache.bindings = extensions.bindings().to_vec();
                cache.generation = cache.generation.wrapping_add(1);
            }
        }
        cache.generation
    }

    /// Put the current binding list into `rt`'s builtins, if it holds an
    /// older one. No script needs rebuilding: they look the functions up in
    /// builtins at call time.
    fn ensure_bindings(&self, rt: &mut Runtime, generation: u64) -> Result<(), String> {
        if rt.bindings_generation == Some(generation) {
            return Ok(());
        }
        let bindings = self
            .bindings
            .lock()
            .map(|c| c.bindings.clone())
            .unwrap_or_default();
        let module = rt.module.clone();
        let api_names = rt.api_names.clone();
        rt.interp
            .enter(|vm| crate::bindings::install(vm, &module, &api_names, &bindings).map_err(|e| runtime::describe(vm, &e)))?;
        rt.bindings_generation = Some(generation);
        Ok(())
    }

    /// Drop this thread's instances that the registry no longer vouches for,
    /// when anything has been evicted since the last look. Without it an
    /// entity that despawned would keep its namespace here forever.
    fn prune(&self, rt: &mut Runtime) {
        let Ok(reg) = self.registry.lock() else {
            return;
        };
        if rt.eviction_epoch == reg.epoch {
            return;
        }
        rt.instances
            .retain(|key, inst| reg.live.get(key).is_some_and(|l| l.token == inst.token));
        rt.eviction_epoch = reg.epoch;
    }

    /// Get or build the instance for this script on this entity, then run
    /// `invoke` in it with the context installed and the props in its globals.
    fn with_instance<F>(
        &self,
        path: &Path,
        ctx: &ScriptContext,
        vars: &mut ScriptVariables,
        invoke: F,
    ) -> Result<Vec<ScriptCommand>, String>
    where
        F: FnOnce(&VirtualMachine, &PyDictRef) -> PyResult<()>,
    {
        let generation = self.sync_bindings(ctx);
        let path_str = path.to_string_lossy().into_owned();
        let key = (ctx.self_entity_id, path_str.clone());
        let script_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("script")
            .to_string();

        runtime::with(|rt| {
            self.ensure_bindings(rt, generation)?;
            self.prune(rt);
            // Leftovers from a props parse or a console expression would
            // otherwise ride out with this hook's commands.
            drain_commands();
            drain_draws();

            let live_token = self
                .registry
                .lock()
                .ok()
                .and_then(|r| r.live.get(&key).map(|l| l.token));
            let valid = matches!(
                (rt.instances.get(&key), live_token),
                (Some(inst), Some(token)) if inst.token == token
            );
            let _frame = frame::enter(ctx);
            if !valid {
                self.build(rt, &key, path, ctx, &script_name)?;
            }
            let Some(globals) = rt.instances.get(&key).map(|i| i.globals.clone()) else {
                return Err("Python instance vanished".into());
            };

            let before: Vec<(String, ScriptValue)> =
                vars.iter_all().map(|(k, v)| (k.clone(), v.clone())).collect();
            let module = rt.module.clone();
            let result = rt.interp.enter(|vm| -> Result<Vec<(String, ScriptValue)>, String> {
                runtime::guard_stack(vm);
                let run = context::set_globals(vm, &globals, ctx, &before)
                    .and_then(|_| invoke(vm, &globals));
                // Flushed whether or not the hook raised, so a partial line
                // printed just before an exception still reaches the console.
                runtime::flush(vm, &module);
                run.map_err(|e| format!("{script_name}: {}", runtime::describe(vm, &e)))?;
                Ok(context::read_back(vm, &globals, &before))
            })?;
            for (name, value) in result {
                vars.set(name, value);
            }
            Ok(drain_commands())
        })
    }

    /// Compile and run a script's top level into a fresh namespace, and record
    /// it as live.
    fn build(
        &self,
        rt: &mut Runtime,
        key: &(u64, String),
        path: &Path,
        ctx: &ScriptContext,
        script_name: &str,
    ) -> Result<(), String> {
        let version = Self::version_of(path);
        let code = rt.code(&key.1, version, || self.source_of(path))?;
        let globals = rt
            .interp
            .enter(|vm| -> PyResult<PyDictRef> {
                runtime::guard_stack(vm);
                let g = rt.new_globals(vm, &key.1)?;
                // The context goes in before the top level runs, so a script
                // reading `self_entity_name` at module level gets a value
                // rather than a NameError. Props do not: an inspector value
                // must win over the top level's default, so they go in after.
                context::set_globals(vm, &g, ctx, &[])?;
                runtime::exec(vm, code, &g)?;
                Ok(g)
            })
            .map_err(|e| rt.interp.enter(|vm| format!("{script_name}: {}", runtime::describe(vm, &e))))?;

        let defined: Vec<&'static str> = rt.interp.enter(|vm| {
            KNOWN_HOOKS
                .iter()
                .copied()
                .filter(|hook| {
                    matches!(globals.get_item_opt(*hook, vm), Ok(Some(f)) if f.is_callable())
                })
                .collect()
        });
        if let Ok(mut map) = self.defined_hooks.lock() {
            map.insert(key.1.clone(), defined);
        }

        let token = {
            let mut reg = self.registry.lock().map_err(|e| e.to_string())?;
            reg.next_token = reg.next_token.wrapping_add(1);
            let token = reg.next_token;
            reg.live.insert(key.clone(), Live { token, version });
            token
        };
        rt.instances.insert(key.clone(), Instance { globals, token });
        Ok(())
    }

    /// Call `hook` with `args` (built inside the VM), if the script defines it.
    fn call_hook(
        &self,
        path: &Path,
        ctx: &ScriptContext,
        vars: &mut ScriptVariables,
        hook: &'static str,
        args: impl FnOnce(&VirtualMachine) -> PyResult<Vec<PyObjectRef>>,
    ) -> Result<Vec<ScriptCommand>, String> {
        self.with_instance(path, ctx, vars, |vm, globals| {
            // A script that does not define the hook is the common case, not
            // an error: most define two.
            let func = globals.get_item_opt(hook, vm)?;
            let Some(func) = runtime::callable(&func) else {
                return Ok(());
            };
            let args = args(vm)?;
            runtime::call_trimmed(vm, func, args).map(|_| ())
        })
        // Named the way the Lua plugin names a failure: "<file> <hook>: ...".
        .map_err(|e| match e.split_once(": ") {
            Some((file, rest)) if !file.contains('\n') => format!("{file} {hook}: {rest}"),
            _ => format!("{hook}: {e}"),
        })
    }

    /// Run a script's top level in a throwaway namespace and read `props()`.
    fn read_props(&self, path: &Path, source: String) -> Vec<VarDef> {
        let path_str = path.to_string_lossy().into_owned();
        runtime::with(|rt| {
            let code = rt.code(&path_str, Self::version_of(path), || Some(source))?;
            let props = rt.interp.enter(|vm| {
                runtime::guard_stack(vm);
                let Ok(globals) = rt.new_globals(vm, &path_str) else {
                    return Vec::new();
                };
                if runtime::exec(vm, code, &globals).is_err() {
                    return Vec::new();
                }
                context::props(vm, &globals)
            });
            // Whatever the top level printed or queued belongs to nobody.
            drain_commands();
            drain_draws();
            Ok(props)
        })
        .unwrap_or_default()
    }
}

impl ScriptBackend for PythonBackend {
    fn name(&self) -> &str {
        "Python"
    }

    fn extensions(&self) -> &[&str] {
        &["py"]
    }

    fn set_scripts_folder(&mut self, path: PathBuf) {
        if let Ok(mut folder) = self.scripts_folder.lock() {
            *folder = path;
        }
    }

    fn set_file_reader(&mut self, reader: FileReader) {
        if let Ok(mut slot) = self.file_reader.lock() {
            *slot = Some(reader);
        }
    }

    fn get_available_scripts(&self) -> Vec<(String, PathBuf)> {
        let Ok(folder) = self.scripts_folder.lock() else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(&*folder) else {
            return Vec::new();
        };
        let mut out: Vec<(String, PathBuf)> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("py"))
            .filter_map(|p| Some((p.file_name()?.to_str()?.to_string(), p)))
            .collect();
        // Stable order, so the picker does not reshuffle between launches.
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// From the hooks recorded when the script was last built. Unknown until
    /// it has been (the first frame builds it), and never answered for "any
    /// hook": Python scripts are run by the engine's executor.
    fn defines_hook(&self, path: &Path, hook: &str) -> Option<bool> {
        if hook == renzora::ANY_HOOK {
            return None;
        }
        let map = self.defined_hooks.lock().ok()?;
        map.get(path.to_string_lossy().as_ref())
            .map(|defined| defined.contains(&hook))
    }

    fn get_script_props(&self, path: &Path) -> Vec<VarDef> {
        match self.source_of(path) {
            Some(source) => self.read_props(path, source),
            None => Vec::new(),
        }
    }

    /// Whether any instance of `path` was built from older text.
    fn needs_reload(&self, path: &Path) -> bool {
        let version = Self::version_of(path);
        let key = path.to_string_lossy();
        self.registry
            .lock()
            .map(|r| {
                r.live
                    .iter()
                    .any(|((_, p), live)| *p == key && live.version != version)
            })
            .unwrap_or(false)
    }

    fn call_on_ready(
        &self,
        path: &Path,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        self.call_hook(path, ctx, vars, "on_ready", |_| Ok(Vec::new()))
    }

    fn call_on_update(
        &self,
        path: &Path,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        self.call_hook(path, ctx, vars, "on_update", |_| Ok(Vec::new()))
    }

    /// Once per fixed tick; `delta` is already the fixed step.
    fn call_on_fixed_update(
        &self,
        path: &Path,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        self.call_hook(path, ctx, vars, "on_fixed_update", |_| Ok(Vec::new()))
    }

    /// `on_rpc(name, args, sender)`.
    fn call_on_rpc(
        &self,
        path: &Path,
        rpc_name: &str,
        args: &HashMap<String, ActionValue>,
        from: u64,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        self.call_hook(path, ctx, vars, "on_rpc", |vm| {
            Ok(vec![
                convert::text(vm, rpc_name),
                convert::args_dict(vm, args)?.into(),
                convert::int(vm, from as i64),
            ])
        })
    }

    /// `on_ui(name, args, entity_bits)`.
    fn call_on_ui(
        &self,
        path: &Path,
        name: &str,
        args: &HashMap<String, ActionValue>,
        entity_bits: u64,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        self.call_hook(path, ctx, vars, "on_ui", |vm| {
            Ok(vec![
                convert::text(vm, name),
                convert::args_dict(vm, args)?.into(),
                convert::int(vm, entity_bits as i64),
            ])
        })
    }

    /// The one hook whose output is not commands: `on_draw(g)` feeds the 2D
    /// vector renderer, so the draws are returned and any commands the script
    /// also issued are dropped, as in Lua (the trait has nowhere for both).
    fn call_on_draw(
        &self,
        path: &Path,
        width: f32,
        height: f32,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<renzora::DrawCmd>, String> {
        let mut draws = Vec::new();
        self.with_instance(path, ctx, vars, |vm, globals| {
            let func = globals.get_item_opt("on_draw", vm)?;
            if let Some(func) = runtime::callable(&func) {
                let g = crate::api::canvas(vm, width, height)?;
                runtime::call_trimmed(vm, func, vec![g])?;
            }
            draws = drain_draws();
            Ok(())
        })?;
        Ok(draws)
    }

    /// `on_animation_event(name, entity_bits)`.
    fn call_on_animation_event(
        &self,
        path: &Path,
        name: &str,
        entity_bits: u64,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        self.call_hook(path, ctx, vars, "on_animation_event", |vm| {
            Ok(vec![convert::text(vm, name), convert::int(vm, entity_bits as i64)])
        })
    }

    /// `on_http(callback, status, body)`.
    fn call_on_http(
        &self,
        path: &Path,
        callback: &str,
        status: u16,
        body: &str,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        self.call_hook(path, ctx, vars, "on_http", |vm| {
            Ok(vec![
                convert::text(vm, callback),
                convert::int(vm, status as i64),
                convert::text(vm, body),
            ])
        })
    }

    /// Joined and left are separate functions, which is why the flag picks the
    /// name rather than becoming an argument.
    fn call_on_player_event(
        &self,
        path: &Path,
        id: u64,
        joined: bool,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        let hook = if joined { "on_player_joined" } else { "on_player_left" };
        self.call_hook(path, ctx, vars, hook, |vm| Ok(vec![convert::int(vm, id as i64)]))
    }

    /// `on_scene_loaded(path)`, or `on_scene_load_failed(path, error)`.
    fn call_on_scene_event(
        &self,
        path: &Path,
        scene: &str,
        error: Option<&str>,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        match error {
            None => self.call_hook(path, ctx, vars, "on_scene_loaded", |vm| {
                Ok(vec![convert::text(vm, scene)])
            }),
            Some(err) => self.call_hook(path, ctx, vars, "on_scene_load_failed", |vm| {
                Ok(vec![convert::text(vm, scene), convert::text(vm, err)])
            }),
        }
    }

    /// `on_event(name, args)`.
    fn call_on_event(
        &self,
        path: &Path,
        name: &str,
        args: &HashMap<String, ActionValue>,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        self.call_hook(path, ctx, vars, "on_event", |vm| {
            Ok(vec![convert::text(vm, name), convert::args_dict(vm, args)?.into()])
        })
    }

    /// The console: an expression's value, or `None` after a statement.
    fn eval_expression(&self, expr: &str) -> Result<String, String> {
        let generation = self.bindings.lock().map(|c| c.generation).unwrap_or(0);
        let out = runtime::with(|rt| {
            self.ensure_bindings(rt, generation)?;
            rt.interp.enter(|vm| -> Result<String, String> {
                runtime::guard_stack(vm);
                let globals = rt
                    .new_globals(vm, "<console>")
                    .map_err(|e| runtime::describe(vm, &e))?;
                let scope = || rustpython_vm::scope::Scope::with_builtins(None, globals.clone(), vm);
                let path = "<console>".to_string();
                let value = match vm.compile(expr, rustpython_vm::compiler::Mode::Eval, path.clone()) {
                    Ok(code) => vm.run_code_obj(code, scope()),
                    // Not an expression; try it as statements.
                    Err(_) => match vm.compile(expr, rustpython_vm::compiler::Mode::Exec, path) {
                        Ok(code) => vm.run_code_obj(code, scope()),
                        Err(e) => return Err(runtime::describe(vm, &vm.new_syntax_error(&e, Some(expr)))),
                    },
                }
                .map_err(|e| runtime::describe(vm, &e))?;
                runtime::flush(vm, &rt.module);
                Ok(match convert::string(&value) {
                    Some(s) => s,
                    None => value
                        .repr(vm)
                        .map(|r| r.as_str().to_owned())
                        .unwrap_or_else(|_| convert::display(&value, vm)),
                })
            })
        });
        // A `print` in the expression is the answer the user asked for, so
        // its lines are kept and logged rather than dropped.
        for cmd in drain_commands() {
            if let ScriptCommand::Log { message, .. } = cmd {
                bevy::log::info!(target: "script", "{message}");
            }
        }
        drain_draws();
        out
    }

    /// Drop every instance of `path`, so the next hook compiles it again.
    fn reload(&self, path: &Path) -> Result<(), String> {
        self.evict(path, 0);
        Ok(())
    }

    /// An empty `path` means any script, a zero `entity` any entity, so a
    /// despawn sends `("", bits)` and a detached script `(path, bits)`.
    fn evict(&self, path: &Path, entity: u64) {
        let path = path.to_string_lossy();
        if let Ok(mut reg) = self.registry.lock() {
            let before = reg.live.len();
            reg.live.retain(|(eid, p), _| {
                let path_matches = path.is_empty() || p.as_str() == path;
                let entity_matches = entity == 0 || *eid == entity;
                !(path_matches && entity_matches)
            });
            if reg.live.len() != before || entity == 0 {
                reg.epoch = reg.epoch.wrapping_add(1);
            }
        }
    }
}
