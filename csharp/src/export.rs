//! Shipping a project's C# with an exported game.
//!
//! A game has no .NET SDK and no script sources, so it cannot build anything:
//! it runs the build the editor made. When an export starts, the engine asks
//! this plugin what it ships ([`stage`], declared in `lib.rs`), and the answer
//! is that build: the C# host, the scripts assembly with its packages, and a
//! manifest naming the folder they were compiled in and the build. The
//! exporter packs them into the game's archive, so a single-file game stays one
//! file; the game unpacks them once into the player's cache folder (see
//! `unpack`) and `project` loads them from there instead of compiling.
//!
//! The build is the editor's current one when there is one, so this is a hash
//! check and some file names. When the scripts changed since the last build
//! (or never built), it builds them here, on the editor's thread: an export is
//! the one moment worth a few seconds' wait for an up-to-date game.
//!
//! # The runtime
//!
//! Windows does not come with the .NET the scripts run on, so by default the
//! export bundles it ([`BUNDLE_RUNTIME`], a switch in the Export window): this
//! machine's .NET runtime, about 76 MB before compression, in the build's
//! `export/dotnet/`, which the game uses whatever the player has. Switched off, the game uses the player's .NET
//! and asks them to install it when there is none (see `prompt`).
//!
//! A runtime can only be bundled for the platform this editor runs on, since
//! it is copied from this machine's install; an export for another platform
//! falls back to asking, and the log says so.

use renzora::{PluginExportFiles, PluginExportOption, PluginExportTarget};

use crate::abi::ABI_VERSION;
use crate::compile;
use crate::dotnet::{Dotnet, MIN_MAJOR, RUNTIME_DOWNLOAD};

/// The Export window switch that bundles the runtime.
pub const BUNDLE_RUNTIME: &str = "bundle_runtime";

/// The switches this plugin adds to the Export window.
pub fn options() -> Vec<PluginExportOption> {
    vec![PluginExportOption {
        id: BUNDLE_RUNTIME.into(),
        label: "Bundle the .NET runtime".into(),
        help: format!(
            "Ships .NET {MIN_MAJOR} with the game (about 70 MB), so players install nothing. \
             Off, players without it are asked to install it when the game starts."
        ),
        default: true,
    }]
}

/// The platform folder name of the machine this runs on, as the export names
/// platforms.
fn this_platform() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x64",
        ("windows", "aarch64") => "windows-arm64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", "x86_64") => "macos-x64",
        ("macos", "aarch64") => "macos-arm64",
        _ => return None,
    })
}

/// The export's question: what does C# need in this game?
pub fn stage(target: &PluginExportTarget) -> Result<PluginExportFiles, String> {
    let mut out = PluginExportFiles::default();
    let files = compile::find_files(target.project_root, &["cs"]);
    if files.is_empty() {
        // Ticked by hand in a project with no C#: nothing to ship.
        return Ok(out);
    }
    if !target.platform.starts_with("windows")
        && !target.platform.starts_with("linux")
        && !target.platform.starts_with("macos")
    {
        return Err(format!(
            "C# scripts cannot run on {} (the .NET runtime this plugin hosts is desktop only); \
             untick C# Scripting in the Plugins tab to export without them",
            target.platform
        ));
    }
    let dotnet = Dotnet::locate()?;
    let plugin_dir = crate::backend::plugin_dir()
        .ok_or_else(|| "the C# Scripting plugin could not find its own folder; reinstall the plugin".to_string())?;
    let host = compile::ensure_host(&dotnet, &plugin_dir, &ABI_VERSION.to_le_bytes())?;
    let built = compile::ensure_scripts(&dotnet, &host, target.project_root, &files).map_err(|f| {
        let mut lines: Vec<String> = f.diagnostics.iter().map(|d| d.short()).collect();
        lines.extend(f.message);
        format!("the C# scripts do not build, so the game would run without them:\n{}", lines.join("\n"))
    })?;

    // Everything goes into the game's archive, so a single-file export stays
    // one file; the game unpacks it on first launch (see `unpack`).
    let mut shipped = compile::export_layout(&host, &built.dll);
    let count = shipped.len();
    out.notes.push(format!("packing the C# build of {} script file(s) ({count} files)", files.len()));

    let bundle = target.option(BUNDLE_RUNTIME) && this_platform() == Some(target.platform);
    if !target.option(BUNDLE_RUNTIME) {
        out.notes.push(format!(
            "the .NET runtime is not bundled: players need .NET {MIN_MAJOR} and are asked to install it \
             ({RUNTIME_DOWNLOAD})"
        ));
    } else if this_platform() != Some(target.platform) {
        out.notes.push(format!(
            "WARN: the .NET runtime can only be bundled for {} (this machine's platform), not {}: \
             players need .NET {MIN_MAJOR} and are asked to install it",
            this_platform().unwrap_or("this machine"),
            target.platform
        ));
    }
    if bundle {
        let runtime = dotnet.runtime_files()?;
        let bytes: u64 = runtime.iter().filter_map(|(_, p)| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
        out.notes.push(format!(
            "bundling the .NET runtime ({} files, {} MB before compression): players install nothing",
            runtime.len(),
            bytes / (1024 * 1024)
        ));
        shipped.extend(runtime.into_iter().map(|(rel, src)| (compile::export_runtime_path(&rel), src)));
    }
    let build = compile::build_id(&shipped);
    for (path, source) in shipped {
        out.archive_copy(path, source);
    }
    out.archive_bytes(compile::export_manifest_path(), compile::export_manifest(target.project_root, build));
    Ok(out)
}
