//! A project's C# scripts: noticing edits, building off the main thread, and
//! what the current build contains.
//!
//! # One assembly, many files
//!
//! Every `.cs` under the scripts folder compiles into ONE assembly, because
//! that is what C# is: a script may use a helper class from another file, a
//! shared `enum`, an extension method. Building them separately would forbid
//! all of that. The cost is that one file with a compiler error stops every C#
//! script in the project, which is also how every C# engine behaves, and the
//! errors say which file to fix.
//!
//! # Generations
//!
//! Each finished build (or failed one) is a new *generation*. Loading one
//! replaces every script and drops every instance in C# at once, since the
//! types they were instances of are gone. The backend compares the generation
//! a script last ran in against the current one to answer `needs_reload`, and
//! the engine then re-runs `on_ready` on fresh instances, exactly as it does
//! for a Lua file it reloads.
//!
//! # Off the main thread
//!
//! A `dotnet build` takes seconds, so it never runs on the thread calling the
//! hooks. A hook asked for while the first build is still running returns no
//! commands (the script simply has not started yet); once the build lands, the
//! script's first hook runs its `OnReady` before anything else, so a script
//! never misses it for having been slow to compile.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock, RwLockReadGuard};
use std::time::{Duration, Instant};

use renzora::serde_json::Value;
use renzora::{ScriptValue, ScriptVariableDefinition as VarDef};

use crate::abi::ABI_VERSION;
use crate::compile::{self, Diagnostic, Failure, Host};
use crate::convert;
use crate::dotnet::Dotnet;

/// How often the scripts folder is checked for edits. The engine's own
/// hot-reload check runs at the same cadence.
const POLL: Duration = Duration::from_millis(500);

/// One script file in the current build.
#[derive(Clone, Debug)]
pub struct Script {
    /// Its index in the load report, which is how C# names it.
    pub index: i32,
    /// Why this file cannot run (no class, several classes) when it cannot.
    pub error: Option<String>,
    /// The engine names of the hooks its class overrides.
    pub hooks: Vec<String>,
    pub props: Vec<Prop>,
}

#[derive(Clone, Debug)]
pub struct Prop {
    pub name: String,
    pub display: String,
    /// The load report's type name: `float`, `int`, `bool`, `string`,
    /// `entity`, `vec2`, `vec3` or `color`.
    pub kind: String,
    pub default: ScriptValue,
    pub hint: Option<String>,
    pub tab: Option<String>,
}

impl Script {
    pub fn defines(&self, hook: &str) -> bool {
        self.hooks.iter().any(|h| h == hook)
    }

    pub fn prop(&self, name: &str) -> Option<&Prop> {
        self.props.iter().find(|p| p.name == name)
    }

    pub fn var_defs(&self) -> Vec<VarDef> {
        self.props
            .iter()
            .map(|p| VarDef {
                name: p.name.clone(),
                display_name: p.display.clone(),
                default_value: p.default.clone(),
                hint: p.hint.clone(),
                tab: p.tab.clone(),
            })
            .collect()
    }
}

/// What a loaded build contains, by normalised file path.
#[derive(Debug, Default)]
pub struct Table {
    pub scripts: HashMap<String, Script>,
}

/// Where the project's C# stands.
#[derive(Clone, Debug)]
pub enum Status {
    /// No build has finished yet (or the project has no `.cs` files).
    Pending,
    /// C# cannot run here at all: no .NET, or the runtime would not start.
    Unavailable(String),
    /// The scripts did not compile.
    Failed(Arc<Failure>),
    Loaded(Arc<Table>),
}

struct Inner {
    folder: PathBuf,
    plugin_dir: Option<PathBuf>,
    last_scan: Option<Instant>,
    /// The fingerprint of the sources most recently built (or being built).
    fingerprint: Option<u64>,
    building: bool,
    status: Status,
    generation: u64,
    /// The assembly currently loaded, so a rebuild that produced the same one
    /// (a file touched, not changed) does not reset every script.
    loaded_dll: Option<PathBuf>,
}

/// A project's scripts and their build.
pub struct Project {
    inner: Mutex<Inner>,
    /// Held for reading by every hook, from reading the status to the end of
    /// the call into C#, and for writing while a new build is swapped in.
    ///
    /// Without it a hook could read the old build's table, and with it a
    /// script's index, just as the worker loaded the new build into C#: the
    /// index would then name a different file's class there, and run it for a
    /// frame. With it, a swap waits for the hook in flight and the next hook
    /// waits for the swap, which is a few milliseconds once per rebuild.
    swap: RwLock<()>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            swap: RwLock::new(()),
            inner: Mutex::new(Inner {
                folder: PathBuf::new(),
                plugin_dir: None,
                last_scan: None,
                fingerprint: None,
                building: false,
                status: Status::Pending,
                generation: 0,
                loaded_dll: None,
            }),
        }
    }
}

/// The normalised form of a script path, which both sides key scripts by:
/// absolute, forward slashes, and lowercase where the file system ignores case.
pub fn norm(path: &Path) -> String {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut s = abs.to_string_lossy().replace('\\', "/");
    if let Some(rest) = s.strip_prefix("//?/") {
        s = rest.to_string();
    }
    if cfg!(any(windows, target_os = "macos")) {
        s = s.to_lowercase();
    }
    s
}

static DOTNET: OnceLock<Result<Dotnet, String>> = OnceLock::new();
static HOST: Mutex<Option<Host>> = Mutex::new(None);

impl Project {
    pub fn set_folder(&self, folder: PathBuf, plugin_dir: Option<PathBuf>) {
        if let Ok(mut inner) = self.inner.lock() {
            if inner.folder != folder {
                inner.folder = folder;
                inner.fingerprint = None;
                inner.last_scan = None;
            }
            inner.plugin_dir = plugin_dir;
        }
    }

    pub fn folder(&self) -> PathBuf {
        self.inner.lock().map(|i| i.folder.clone()).unwrap_or_default()
    }

    /// Hold the current build in place for the length of a call into C#; see
    /// [`Project::swap`].
    pub fn hold(&self) -> RwLockReadGuard<'_, ()> {
        self.swap.read().unwrap_or_else(|e| e.into_inner())
    }

    /// The current status and generation.
    pub fn status(&self) -> (Status, u64) {
        self.inner
            .lock()
            .map(|i| (i.status.clone(), i.generation))
            .unwrap_or((Status::Pending, 0))
    }

    pub fn building(&self) -> bool {
        self.inner.lock().map(|i| i.building).unwrap_or(false)
    }

    /// Look for edits (at most every [`POLL`], unless `now`) and start a build
    /// when the sources moved and none is running. A project with no `.cs`
    /// file never starts one, so a Lua-only project never starts .NET.
    pub fn poll(self: &Arc<Self>, now: bool) {
        let (folder, plugin_dir) = {
            let Ok(mut inner) = self.inner.lock() else {
                return;
            };
            if inner.building || inner.folder.as_os_str().is_empty() {
                return;
            }
            if !now && inner.last_scan.is_some_and(|t| t.elapsed() < POLL) {
                return;
            }
            inner.last_scan = Some(Instant::now());
            (inner.folder.clone(), inner.plugin_dir.clone())
        };
        // An exported game runs the build its export shipped, packed in its
        // archive (or, from an older export, beside the plugin): it has no
        // sources to look at and no SDK to compile them with. Unpacking is
        // left to the build thread; only the check runs here.
        let beside = plugin_dir.as_deref().and_then(compile::exported_build);
        let archived = beside.is_none() && crate::unpack::in_archive();
        let (files, fp) = if beside.is_some() || archived {
            // One fixed fingerprint: the shipped build never changes, so it
            // loads once.
            (Vec::new(), 1)
        } else {
            let files = compile::find_files(&folder, &["cs"]);
            if files.is_empty() {
                return;
            }
            // `csharp.toml` too, so adding a package rebuilds with no script edited.
            let mut watched = files.clone();
            watched.extend(compile::packages_file(&folder));
            let fp = compile::fingerprint(&watched);
            (files, fp)
        };
        {
            let Ok(mut inner) = self.inner.lock() else {
                return;
            };
            if inner.building || inner.fingerprint == Some(fp) {
                return;
            }
            inner.fingerprint = Some(fp);
            inner.building = true;
        }
        let me = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("csharp-build".into())
            .spawn(move || {
                let started = Instant::now();
                let exported = if archived {
                    match crate::unpack::unpack() {
                        Ok(dir) => match compile::exported_build(&dir) {
                            Some(e) => Some(e),
                            None => {
                                let why = format!("this game's C# build in {} is incomplete", dir.display());
                                me.finish(Outcome::Unavailable(why), started.elapsed(), 0);
                                return;
                            }
                        },
                        Err(e) => {
                            me.finish(Outcome::Unavailable(e), started.elapsed(), 0);
                            return;
                        }
                    }
                } else {
                    beside
                };
                match build(&folder, plugin_dir.as_deref(), &files, me.loaded_dll(), exported.as_ref()) {
                    Stage::Done(outcome) => me.finish(outcome, started.elapsed(), files.len()),
                    Stage::Load { clr, dll, compiled } => {
                        // The swap: C# and the table change together.
                        let _swap = me.swap.write().unwrap_or_else(|e| e.into_inner());
                        let rebase = exported.as_ref().map(|e| (e.built_in.as_str(), folder.as_path()));
                        let outcome = load(clr, dll, compiled, rebase);
                        me.finish(outcome, started.elapsed(), files.len());
                    }
                }
            });
        if spawned.is_err() {
            if let Ok(mut inner) = self.inner.lock() {
                inner.building = false;
                inner.fingerprint = None;
            }
        }
    }

    fn loaded_dll(&self) -> Option<PathBuf> {
        self.inner.lock().ok().and_then(|i| i.loaded_dll.clone())
    }

    fn finish(&self, outcome: Outcome, took: Duration, count: usize) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        inner.building = false;
        match outcome {
            Outcome::Unchanged => {}
            Outcome::Loaded { dll, table, compiled } => {
                if compiled {
                    bevy::log::info!(
                        "[C#] compiled {count} script file(s) in {} ms",
                        took.as_millis()
                    );
                }
                inner.loaded_dll = Some(dll);
                inner.status = Status::Loaded(Arc::new(table));
                inner.generation += 1;
            }
            Outcome::Failed(failure) => {
                for d in &failure.diagnostics {
                    bevy::log::error!("[C#] {}", d.short());
                }
                if let Some(m) = &failure.message {
                    bevy::log::error!("[C#] {m}");
                }
                inner.loaded_dll = None;
                inner.status = Status::Failed(Arc::new(failure));
                inner.generation += 1;
            }
            Outcome::Unavailable(why) => {
                bevy::log::error!("[C#] {why}");
                inner.loaded_dll = None;
                inner.status = Status::Unavailable(why);
                inner.generation += 1;
            }
        }
    }
}

enum Outcome {
    /// The build produced the assembly already loaded.
    Unchanged,
    Loaded {
        dll: PathBuf,
        table: Table,
        compiled: bool,
    },
    Failed(Failure),
    Unavailable(String),
}

/// How far [`build`] got: done (with an outcome), or ready to load.
enum Stage {
    Done(Outcome),
    Load {
        clr: &'static crate::bridge::Clr,
        dll: PathBuf,
        compiled: bool,
    },
}

/// Find .NET, build the host and the scripts, and start the runtime: every
/// step before the swap, none of which touches what hooks are running.
fn build(
    folder: &Path,
    plugin_dir: Option<&Path>,
    files: &[PathBuf],
    loaded: Option<PathBuf>,
    exported: Option<&compile::Exported>,
) -> Stage {
    // A game runs the runtime its export bundled, whatever the machine has;
    // without one, the player's own.
    let bundled = exported.and_then(|e| e.dotnet_root.as_deref()).and_then(Dotnet::at);
    let dotnet = match bundled {
        Some(d) => d,
        None => match DOTNET.get_or_init(Dotnet::locate) {
            Ok(d) => d.clone(),
            Err(e) if exported.is_some() => {
                // A player, not a developer: ask them to install it, and say
                // so in words for them rather than the SDK instructions.
                crate::prompt::missing_runtime();
                return Stage::Done(Outcome::Unavailable(format!(
                    "this game's C# scripts need the Microsoft .NET 10 Runtime, which is not installed ({}); \
                     install it from {}",
                    e.split(';').next().unwrap_or(e),
                    crate::dotnet::RUNTIME_DOWNLOAD
                )));
            }
            Err(e) => return Stage::Done(Outcome::Unavailable(e.clone())),
        },
    };
    if let Some(ex) = exported {
        if loaded.as_deref() == Some(ex.scripts.as_path()) {
            return Stage::Done(Outcome::Unchanged);
        }
        return match crate::bridge::clr(&dotnet, &ex.host) {
            Ok(clr) => Stage::Load {
                clr,
                dll: ex.scripts.clone(),
                compiled: false,
            },
            // The player has .NET, but too old a one: the runtime would not
            // start for want of .NET 10. Asked the same way as having none,
            // since the fix is the same install. Not for a bundled runtime,
            // which always has its framework.
            Err(e) if ex.dotnet_root.is_none() && e.contains(crate::dotnet::INSTALL_HINT) => {
                crate::prompt::missing_runtime();
                Stage::Done(Outcome::Unavailable(format!(
                    "this game's C# scripts need the Microsoft .NET 10 Runtime, and only an older one is installed; \
                     install it from {}",
                    crate::dotnet::RUNTIME_DOWNLOAD
                )))
            }
            Err(e) => Stage::Done(Outcome::Unavailable(e)),
        };
    }
    let Some(plugin_dir) = plugin_dir else {
        return Stage::Done(Outcome::Unavailable(
            "the C# Scripting plugin could not find its own folder (its C# host lives there); reinstall the plugin"
                .into(),
        ));
    };
    let host = {
        let cached = HOST.lock().ok().and_then(|h| h.clone());
        match cached {
            Some(h) => h,
            None => match compile::ensure_host(&dotnet, plugin_dir, &ABI_VERSION.to_le_bytes()) {
                Ok(h) => {
                    if let Ok(mut slot) = HOST.lock() {
                        *slot = Some(h.clone());
                    }
                    h
                }
                Err(e) => return Stage::Done(Outcome::Unavailable(e)),
            },
        }
    };
    let built = match compile::ensure_scripts(&dotnet, &host, folder, files) {
        Ok(b) => b,
        Err(f) => return Stage::Done(Outcome::Failed(f)),
    };
    for w in &built.warnings {
        bevy::log::warn!("[C#] {}", w.short());
    }
    if loaded.as_deref() == Some(built.dll.as_path()) {
        return Stage::Done(Outcome::Unchanged);
    }
    match crate::bridge::clr(&dotnet, &host) {
        Ok(clr) => Stage::Load {
            clr,
            dll: built.dll,
            compiled: built.compiled,
        },
        Err(e) => Stage::Done(Outcome::Unavailable(e)),
    }
}

/// Load a build into C# and read what it holds. Called with the swap lock
/// held for writing.
fn load(clr: &crate::bridge::Clr, dll: PathBuf, compiled: bool, rebase: Option<(&str, &Path)>) -> Outcome {
    let report = match clr.load(&dll) {
        Ok(r) => r,
        Err(e) => {
            return Outcome::Failed(Failure {
                diagnostics: Vec::new(),
                message: Some(e),
            })
        }
    };
    match parse_report(&report, rebase) {
        Some(table) => Outcome::Loaded { dll, table, compiled },
        None => Outcome::Failed(Failure {
            diagnostics: Vec::new(),
            message: Some("the C# host sent a load report this plugin cannot read".into()),
        }),
    }
}

/// Read `Load`'s report:
///
/// ```json
/// {"scripts": [{"file": "c:/p/scripts/spinner.cs", "class": "Spinner",
///   "error": null, "hooks": ["on_update"],
///   "props": [{"name": "Speed", "display": "Speed", "type": "float",
///              "value": 90.0, "hint": "degrees per second", "tab": null}]}]}
/// ```
///
/// `rebase` is `(the folder the build was compiled in, this game's folder)`
/// for an exported build: the report names each script by its path on the
/// machine that built it, and the engine asks by its path here.
pub fn parse_report(text: &str, rebase: Option<(&str, &Path)>) -> Option<Table> {
    let root = convert::parse(text)?;
    let list = root.get("scripts")?.as_array()?;
    let mut table = Table::default();
    for (i, s) in list.iter().enumerate() {
        let file = s.get("file")?.as_str()?;
        let hooks = s
            .get("hooks")
            .and_then(Value::as_array)
            .map(|h| h.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let props = s
            .get("props")
            .and_then(Value::as_array)
            .map(|ps| {
                ps.iter()
                    .filter_map(|p| {
                        let kind = p.get("type")?.as_str()?.to_string();
                        Some(Prop {
                            name: p.get("name")?.as_str()?.to_string(),
                            display: p.get("display")?.as_str()?.to_string(),
                            default: convert::to_prop_value(p.get("value")?, &kind)?,
                            hint: p.get("hint").and_then(Value::as_str).map(str::to_string),
                            tab: p.get("tab").and_then(Value::as_str).map(str::to_string),
                            kind,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let key = match rebase {
            Some((from, to)) => rebase_path(file, from, to),
            None => norm(Path::new(file)),
        };
        table.scripts.insert(
            key,
            Script {
                index: i as i32,
                error: s.get("error").and_then(Value::as_str).map(str::to_string),
                hooks,
                props,
            },
        );
    }
    Some(table)
}

/// `file`, compiled under `from`, as the same file under `to`, normalised. A
/// file outside `from` keeps its own path (it cannot be asked for anyway).
fn rebase_path(file: &str, from: &str, to: &Path) -> String {
    let file_n = norm(Path::new(file));
    let from_n = norm(Path::new(from));
    match file_n.strip_prefix(&from_n) {
        Some(rest) if !from_n.is_empty() => norm(&to.join(rest.trim_start_matches('/'))),
        _ => file_n,
    }
}

/// What a script at `path` reports when the build failed: its own errors when
/// it has some, else a pointer at the file that does.
pub fn failure_for(failure: &Failure, path: &Path) -> String {
    let key = norm(path);
    let own: Vec<&Diagnostic> = failure
        .diagnostics
        .iter()
        .filter(|d| d.file.as_deref().is_some_and(|f| norm(f) == key))
        .collect();
    if !own.is_empty() {
        return own.iter().map(|d| d.short()).collect::<Vec<_>>().join("\n");
    }
    if let Some(m) = &failure.message {
        return m.clone();
    }
    let first: Vec<String> = failure.diagnostics.iter().take(3).map(Diagnostic::short).collect();
    let more = failure.diagnostics.len().saturating_sub(first.len());
    let tail = if more > 0 { format!(" (and {more} more)") } else { String::new() };
    format!(
        "not run: the project's C# scripts do not compile, and all of them build together: {}{tail}",
        first.join("; ")
    )
}
