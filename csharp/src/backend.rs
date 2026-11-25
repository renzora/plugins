//! The `ScriptBackend`: which scripts are live, and the hooks.
//!
//! The C# side owns the instances (one per entity and script, created on first
//! use); this side owns the bookkeeping the engine asks about: which build
//! generation each script last ran in (for `needs_reload`), what props each
//! instance was last given (so a hook sends none when nothing changed), and
//! which instances have been touched (so a hook the class does not define is
//! not called at all).
//!
//! Every hook is one [`InvokeRequest`]. Its context is a struct of plain
//! numbers built on the stack, its props are JSON only when they changed, and
//! what the script asks for comes back through the native table into the
//! thread-local buffers, which the hook drains afterwards like every other
//! language plugin.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use renzora::core::script_bridge::DrawCmd;
use renzora::script_extension::Binding;
use renzora::serde_json::Value;
use renzora::{
    FileReader, ScriptActionValue as ActionValue, ScriptBackend, ScriptCommand, ScriptContext, ScriptValue,
    ScriptVariableDefinition as VarDef, ScriptVariables,
};

use crate::abi::{hook, utf8, HookContext, InvokeRequest};
use crate::buffers::{drain_commands, drain_draws};
use crate::project::{self, norm, Project, Status};
use crate::{bridge, context, convert, frame};

const EXTENSIONS: &[&str] = &["cs"];

/// The hook's own arguments, beyond the context: see [`InvokeRequest`].
#[derive(Default)]
struct Extra<'a> {
    name: &'a str,
    args: Option<String>,
    text: &'a str,
    number: u64,
    size: (f32, f32),
}

#[derive(Default)]
pub struct CSharpBackend {
    project: Arc<Project>,
    /// Per script, the generation it last ran in. A script whose generation is
    /// behind the current one waits (runs nothing) until the engine reloads it.
    served: Mutex<HashMap<String, u64>>,
    /// Per instance, the generation and the props JSON it holds. Present means
    /// the instance exists in C#, which has run its `OnReady`.
    instances: Mutex<HashMap<(u64, String), (u64, String)>>,
    /// How many bindings the native table was last given.
    bindings: Mutex<usize>,
}

/// Where the plugin's own folder is, for its C# host sources and build output.
///
/// `RENZORA_CSHARP_PLUGIN_DIR` first, for a development checkout; then where
/// the engine says the `csharp` plugin is installed; then beside the
/// executable, which is where an exported game carries its plugins.
pub(crate) fn plugin_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("RENZORA_CSHARP_PLUGIN_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = renzora::plugin_dir("csharp") {
        return Some(dir);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.join("plugins").join("csharp");
    dir.is_dir().then_some(dir)
}

/// A script's log line, written to the same two places the engine writes one:
/// the Console panel and the tracing log a game without one prints.
fn print_log(level: &str, message: String) {
    use renzora::core::console_log::{console_error, console_info, console_warn};
    match level.to_ascii_lowercase().as_str() {
        "warn" => {
            bevy::log::warn!(target: "script", "{message}");
            console_warn("Script", message);
        }
        "error" => {
            bevy::log::error!(target: "script", "{message}");
            console_error("Script", message);
        }
        _ => {
            bevy::log::info!(target: "script", "{message}");
            console_info("Script", message);
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("script")
        .to_string()
}

impl CSharpBackend {
    /// Bring the binding table up to date with what the engine declares.
    ///
    /// Compared by count, as in the Lua and JavaScript plugins: extensions
    /// register while the `App` is built and never change afterwards, so the
    /// one transition to catch is "none yet" to "all of them".
    fn sync_bindings(&self, ctx: &ScriptContext) {
        let Some(extensions) = ctx.extensions() else {
            return;
        };
        let list: &[Binding] = extensions.bindings();
        if let Ok(mut count) = self.bindings.lock() {
            if *count != list.len() {
                crate::bindings::install(list);
                *count = list.len();
            }
        }
    }

    /// Run one hook. `Ok` carries the commands (and draws) it produced.
    fn run(
        &self,
        path: &Path,
        ctx: &ScriptContext,
        vars: &mut ScriptVariables,
        hook_id: u32,
        extra: Extra<'_>,
    ) -> Result<(Vec<ScriptCommand>, Vec<DrawCmd>), String> {
        self.sync_bindings(ctx);
        self.project.poll(false);
        // The build stays the one read below until the call into C# returns.
        let _hold = self.project.hold();
        let hook_name = hook::NAMES[hook_id as usize];
        let file = file_name(path);
        let (status, generation) = self.project.status();
        let table = match status {
            // Not built yet: the script has not started, which is not an error.
            Status::Pending => return Ok(Default::default()),
            Status::Unavailable(why) => return Err(format!("{file}: {why}")),
            Status::Failed(f) => return Err(project::failure_for(&f, path)),
            Status::Loaded(t) => t,
        };
        let key = norm(path);
        let Some(script) = table.scripts.get(&key) else {
            if self.project.building() {
                // A file added since the last build, which is being built now.
                return Ok(Default::default());
            }
            return Err(format!(
                "{file}: no class deriving Renzora.Script was found in this file (declare `public class {} : Script`)",
                path.file_stem().and_then(|s| s.to_str()).unwrap_or("MyScript")
            ));
        };
        if let Some(e) = &script.error {
            return Err(format!("{file}: {e}"));
        }

        // Wait for the engine's reload when this script last ran in an older
        // build: it re-runs `on_ready` on a fresh instance, and running hooks
        // on one in between would run `OnReady` twice.
        {
            let mut served = self.served.lock().map_err(|e| e.to_string())?;
            match served.get(&key) {
                Some(g) if *g != generation => return Ok(Default::default()),
                Some(_) => {}
                None => {
                    served.insert(key.clone(), generation);
                }
            }
        }

        let instance = (ctx.self_entity_id, key);
        let before: Vec<(String, ScriptValue)> = vars.iter_all().map(|(k, v)| (k.clone(), v.clone())).collect();
        let props_now = convert::props_json(&before);
        let (touched, props_unchanged) = {
            let map = self.instances.lock().map_err(|e| e.to_string())?;
            match map.get(&instance) {
                Some((g, last)) if *g == generation => (true, *last == props_now),
                _ => (false, false),
            }
        };
        // A hook the class does not override does nothing, so it is not
        // called, unless the instance does not exist yet: creating it runs a
        // deferred `OnReady` (see `project`), and that must not wait for a hook
        // the script happens to define.
        if touched && !script.defines(hook_name) {
            return Ok(Default::default());
        }

        let Some(clr) = bridge::running() else {
            return Ok(Default::default());
        };
        let mut timers = String::new();
        let hook_ctx: HookContext = context::fill(ctx, &mut timers);
        let props_sent = if props_unchanged { "" } else { props_now.as_str() };
        let args = extra.args.unwrap_or_default();
        let (props_ptr, props_len) = utf8(props_sent);
        let (name_ptr, name_len) = utf8(extra.name);
        let (args_ptr, args_len) = utf8(&args);
        let (text_ptr, text_len) = utf8(extra.text);
        let req = InvokeRequest {
            entity: ctx.self_entity_id,
            number: extra.number,
            ctx: &hook_ctx,
            props_ptr,
            props_len,
            name_ptr,
            name_len,
            args_ptr,
            args_len,
            text_ptr,
            text_len,
            script: script.index,
            hook: hook_id,
            width: extra.size.0,
            height: extra.size.1,
        };

        let _frame = frame::enter(ctx);
        // Leftovers from a call made outside any hook would otherwise ride out
        // with this one's commands.
        drain_commands();
        drain_draws();
        let result = clr.invoke(&req);
        // The props are read back whether or not the hook threw, so a value
        // set just before an exception is kept, as in the other plugins.
        let changed = match &result {
            Ok(p) => p.clone(),
            Err(_) => None,
        };
        if let Some(changed) = changed.as_deref().and_then(convert::parse) {
            if let Value::Object(map) = changed {
                for (name, value) in map {
                    let kind = script.prop(&name).map(|p| p.kind.as_str()).unwrap_or("");
                    if let Some(sv) = convert::to_prop_value(&value, kind) {
                        vars.set(name, sv);
                    }
                }
            }
        }
        let after: Vec<(String, ScriptValue)> = vars.iter_all().map(|(k, v)| (k.clone(), v.clone())).collect();
        if let Ok(mut map) = self.instances.lock() {
            map.insert(instance, (generation, convert::props_json(&after)));
        }
        let commands = drain_commands();
        let draws = drain_draws();
        match result {
            Ok(_) => Ok((commands, draws)),
            Err(e) => {
                // A hook that throws returns no commands, and the engine drops
                // them, so lines it logged before the exception are printed
                // here. They are usually what explains it: a script that logged
                // three lines and then failed showed only the failure.
                for cmd in commands {
                    if let ScriptCommand::Log { level, message } = cmd {
                        print_log(&level, message);
                    }
                }
                // Named the way the Lua plugin names a failure: "<file> <hook>: ...".
                Err(format!("{file} {hook_name}: {e}"))
            }
        }
    }

    fn commands(
        &self,
        path: &Path,
        ctx: &ScriptContext,
        vars: &mut ScriptVariables,
        hook_id: u32,
        extra: Extra<'_>,
    ) -> Result<Vec<ScriptCommand>, String> {
        self.run(path, ctx, vars, hook_id, extra).map(|(c, _)| c)
    }

    /// The script at `path` in the current build, if it is loaded.
    fn script(&self, path: &Path) -> Option<project::Script> {
        match self.project.status().0 {
            Status::Loaded(t) => t.scripts.get(&norm(path)).cloned(),
            _ => None,
        }
    }
}

impl ScriptBackend for CSharpBackend {
    fn name(&self) -> &str {
        "C#"
    }

    fn extensions(&self) -> &[&str] {
        EXTENSIONS
    }

    /// Also starts the first build, off the main thread, when the project has
    /// any `.cs` file: by the time Play is pressed it has usually finished.
    fn set_scripts_folder(&mut self, path: PathBuf) {
        self.project.set_folder(path, plugin_dir());
        self.project.poll(true);
    }

    /// Kept for the trait's sake. C# is compiled from the files on disk by
    /// `dotnet`, which cannot read an `.rpak`; see the README's limitations.
    fn set_file_reader(&mut self, _reader: FileReader) {}

    fn get_available_scripts(&self) -> Vec<(String, PathBuf)> {
        let folder = self.project.folder();
        let mut out: Vec<(String, PathBuf)> = crate::compile::find_files(&folder, EXTENSIONS)
            .into_iter()
            .filter_map(|p| {
                let rel = p.strip_prefix(&folder).ok()?.to_string_lossy().replace('\\', "/");
                Some((rel, p))
            })
            .collect();
        // Stable order, so the picker does not reshuffle between launches.
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// From the current build. Unknown until there is one, and never answered
    /// for "any hook": the engine's executor runs C# scripts.
    fn defines_hook(&self, path: &Path, hook: &str) -> Option<bool> {
        if hook == renzora::ANY_HOOK {
            return None;
        }
        self.script(path).map(|s| s.defines(hook))
    }

    /// The `[Prop]` fields of the script's class, from the current build.
    /// Empty until the first build lands; the inspector shows them from then.
    fn get_script_props(&self, path: &Path) -> Vec<VarDef> {
        self.project.poll(false);
        self.script(path).map(|s| s.var_defs()).unwrap_or_default()
    }

    /// Whether `path` last ran in an older build than the current one.
    fn needs_reload(&self, path: &Path) -> bool {
        self.project.poll(false);
        let generation = self.project.status().1;
        self.served
            .lock()
            .map(|s| s.get(&norm(path)).is_some_and(|g| *g != generation))
            .unwrap_or(false)
    }

    fn call_on_ready(&self, path: &Path, ctx: &mut ScriptContext, vars: &mut ScriptVariables) -> Result<Vec<ScriptCommand>, String> {
        self.commands(path, ctx, vars, hook::READY, Extra::default())
    }

    fn call_on_update(&self, path: &Path, ctx: &mut ScriptContext, vars: &mut ScriptVariables) -> Result<Vec<ScriptCommand>, String> {
        self.commands(path, ctx, vars, hook::UPDATE, Extra::default())
    }

    /// Once per fixed tick; `Delta` is already the fixed step.
    fn call_on_fixed_update(
        &self,
        path: &Path,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        self.commands(path, ctx, vars, hook::FIXED_UPDATE, Extra::default())
    }

    /// `OnRpc(name, args, from)`.
    fn call_on_rpc(
        &self,
        path: &Path,
        rpc_name: &str,
        args: &HashMap<String, ActionValue>,
        from: u64,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        let extra = Extra {
            name: rpc_name,
            args: Some(convert::args_json(args)),
            number: from,
            ..Default::default()
        };
        self.commands(path, ctx, vars, hook::RPC, extra)
    }

    /// `OnUi(name, args, entity)`.
    fn call_on_ui(
        &self,
        path: &Path,
        name: &str,
        args: &HashMap<String, ActionValue>,
        entity_bits: u64,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        let extra = Extra {
            name,
            args: Some(convert::args_json(args)),
            number: entity_bits,
            ..Default::default()
        };
        self.commands(path, ctx, vars, hook::UI, extra)
    }

    /// The one hook whose output is not commands: `OnDraw(g)` feeds the 2D
    /// vector renderer, so the draws are returned and any commands the script
    /// also issued are dropped, as in every other plugin (the trait has
    /// nowhere for both).
    fn call_on_draw(
        &self,
        path: &Path,
        width: f32,
        height: f32,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<DrawCmd>, String> {
        let extra = Extra {
            size: (width, height),
            ..Default::default()
        };
        self.run(path, ctx, vars, hook::DRAW, extra).map(|(_, d)| d)
    }

    /// `OnAnimationEvent(name, entity)`.
    fn call_on_animation_event(
        &self,
        path: &Path,
        name: &str,
        entity_bits: u64,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        let extra = Extra {
            name,
            number: entity_bits,
            ..Default::default()
        };
        self.commands(path, ctx, vars, hook::ANIMATION_EVENT, extra)
    }

    /// `OnHttp(callback, status, body)`.
    fn call_on_http(
        &self,
        path: &Path,
        callback: &str,
        status: u16,
        body: &str,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        let extra = Extra {
            name: callback,
            text: body,
            number: status as u64,
            ..Default::default()
        };
        self.commands(path, ctx, vars, hook::HTTP, extra)
    }

    /// Joined and left are separate methods, which is why the flag picks the
    /// hook rather than becoming an argument.
    fn call_on_player_event(
        &self,
        path: &Path,
        id: u64,
        joined: bool,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        let hook_id = if joined { hook::PLAYER_JOINED } else { hook::PLAYER_LEFT };
        let extra = Extra {
            number: id,
            ..Default::default()
        };
        self.commands(path, ctx, vars, hook_id, extra)
    }

    /// `OnSceneLoaded(path)`, or `OnSceneLoadFailed(path, error)`.
    fn call_on_scene_event(
        &self,
        path: &Path,
        scene: &str,
        error: Option<&str>,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        let (hook_id, text) = match error {
            None => (hook::SCENE_LOADED, ""),
            Some(e) => (hook::SCENE_LOAD_FAILED, e),
        };
        let extra = Extra {
            name: scene,
            text,
            ..Default::default()
        };
        self.commands(path, ctx, vars, hook_id, extra)
    }

    /// `OnEvent(name, args)`, including a plugin's answer to a `Call`.
    fn call_on_event(
        &self,
        path: &Path,
        name: &str,
        args: &HashMap<String, ActionValue>,
        ctx: &mut ScriptContext,
        vars: &mut ScriptVariables,
    ) -> Result<Vec<ScriptCommand>, String> {
        let extra = Extra {
            name,
            args: Some(convert::args_json(args)),
            ..Default::default()
        };
        self.commands(path, ctx, vars, hook::EVENT, extra)
    }

    /// C# has no expression evaluator in the runtime itself (that is Roslyn's
    /// scripting API, a NuGet package of its own), so the console cannot run
    /// C#. Said plainly rather than pretending.
    fn eval_expression(&self, _expr: &str) -> Result<String, String> {
        Err("the console does not evaluate C#; use Log(...) in a script, or a Lua or JavaScript console line".into())
    }

    /// The engine's half of a hot reload: from now on `path` runs in the
    /// current build, on fresh instances.
    fn reload(&self, path: &Path) -> Result<(), String> {
        let key = norm(path);
        let _hold = self.project.hold();
        let generation = self.project.status().1;
        if let Ok(mut served) = self.served.lock() {
            served.insert(key.clone(), generation);
        }
        if let Ok(mut map) = self.instances.lock() {
            map.retain(|(_, p), _| *p != key);
        }
        if let (Some(clr), Some(script)) = (bridge::running(), self.script(path)) {
            clr.evict(0, script.index);
        }
        Ok(())
    }

    /// An empty `path` means any script, a zero `entity` any entity, so a
    /// despawn sends `("", bits)` and a detached script `(path, bits)`.
    fn evict(&self, path: &Path, entity: u64) {
        let key = (!path.as_os_str().is_empty()).then(|| norm(path));
        if let Ok(mut map) = self.instances.lock() {
            map.retain(|(e, p), _| {
                let path_matches = key.as_ref().is_none_or(|k| k == p);
                let entity_matches = entity == 0 || *e == entity;
                !(path_matches && entity_matches)
            });
        }
        let Some(clr) = bridge::running() else {
            return;
        };
        let _hold = self.project.hold();
        match &key {
            None => clr.evict(entity, -1),
            Some(_) => {
                if let Some(script) = self.script(path) {
                    clr.evict(entity, script.index);
                }
            }
        }
    }
}
