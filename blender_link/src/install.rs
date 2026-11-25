//! Installs the Blender add-on and turns it on, so there is nothing to do by
//! hand.
//!
//! Installing this plugin is consent to that: the listing says so, and the
//! add-on's source ships beside this file. On every editor start, for each
//! Blender 4.2+ that has been run on this machine:
//!
//! 1. The add-on is written to that version's user `scripts/addons` folder,
//!    or rewritten when this plugin carries a newer one. A running Blender
//!    with the add-on enabled notices the new file and reloads it by itself.
//! 2. If it is not yet enabled in that version's preferences, and no Blender
//!    is running, a headless Blender enables it and saves the preferences, so
//!    the next Blender start has it on. With Blender open that is not safe (it
//!    writes its own preferences back on quit and would undo the change), so
//!    the Console says to tick it in Edit > Preferences > Add-ons instead,
//!    which loads it at once, no restart.
//!
//! The source is compiled into this plugin, so what Blender runs always
//! matches the protocol this build speaks.

use renzora::core::console_log::{console_error, console_info, console_warn};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::CATEGORY;

/// The add-on, as shipped beside this file.
pub const ADDON: &str = include_str!("../addon/renzora_live_link.py");
const MODULE: &str = "renzora_live_link";
const FILE_NAME: &str = "renzora_live_link.py";

/// Run by a headless Blender: enable the add-on and save the preferences.
/// Prints the outcome on a line of its own for [`enable`] to read.
/// A raw string, because Python's indentation is significant and a `\`
/// line continuation would strip it.
const ENABLE_SCRIPT: &str = r#"import bpy, addon_utils
n = 'renzora_live_link'
if n not in bpy.context.preferences.addons:
    addon_utils.enable(n, default_set=True)
    bpy.ops.wm.save_userpref()
print('RENZORA_ENABLED' if n in bpy.context.preferences.addons else 'RENZORA_NOT_ENABLED')
"#;

/// Keeps a helper process from flashing a console window on Windows.
#[cfg(target_os = "windows")]
fn quiet(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW)
}

#[cfg(not(target_os = "windows"))]
fn quiet(cmd: &mut Command) -> &mut Command {
    cmd
}

/// Blender's per-version user folders (`.../Blender/5.1`), one per version
/// that has been run at least once on this machine.
fn version_dirs() -> Vec<(String, PathBuf)> {
    let base: Option<PathBuf> = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Blender Foundation").join("Blender"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join("Library")
                .join("Application Support")
                .join("Blender")
        })
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join("blender"))
    };
    let Some(base) = base else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };
    let mut dirs: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            supported(&name).then(|| (name, e.path()))
        })
        .collect();
    dirs.sort();
    dirs
}

/// Versions the add-on runs on: 4.2 and newer (it reads corner normals,
/// which arrived in 4.1, and targets the 4.2 extension-era API).
fn supported(version: &str) -> bool {
    let mut parts = version.split('.');
    let major = parts.next().and_then(|p| p.parse::<u32>().ok());
    let minor = parts.next().and_then(|p| p.parse::<u32>().ok());
    matches!((major, minor), (Some(m), Some(n)) if m > 4 || (m == 4 && n >= 2))
}

/// `5.1` from `Blender 5.1.2 ...` (`--version`'s first line) or from an
/// install folder named `Blender 5.1`.
fn major_minor(text: &str) -> Option<String> {
    let rest = text.trim().strip_prefix("Blender")?.trim_start();
    let mut parts = rest.split(|c: char| !c.is_ascii_digit());
    let major = parts.next().filter(|p| !p.is_empty())?;
    let minor = parts.next().filter(|p| !p.is_empty())?;
    Some(format!("{major}.{minor}"))
}

/// Blender executables on this machine, each with its `major.minor`.
fn blender_executables() -> Vec<(String, PathBuf)> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "windows") {
        for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            let Some(root) = std::env::var_os(var).map(PathBuf::from) else { continue };
            if let Ok(entries) = std::fs::read_dir(root.join("Blender Foundation")) {
                candidates.extend(entries.flatten().map(|e| e.path().join("blender.exe")));
            }
            candidates.push(root.join("Steam").join("steamapps").join("common").join("Blender").join("blender.exe"));
        }
    } else if cfg!(target_os = "macos") {
        candidates.push(PathBuf::from("/Applications/Blender.app/Contents/MacOS/Blender"));
        if let Some(home) = std::env::var_os("HOME") {
            candidates.push(PathBuf::from(home).join("Applications/Blender.app/Contents/MacOS/Blender"));
        }
    } else {
        candidates.push(PathBuf::from("/usr/bin/blender"));
        candidates.push(PathBuf::from("/snap/bin/blender"));
    }
    let exe_name = if cfg!(target_os = "windows") { "blender.exe" } else { "blender" };
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(exe_name)));
    }

    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for exe in candidates {
        if !exe.is_file() || found.iter().any(|(_, e)| *e == exe) {
            continue;
        }
        // The folder name when it carries the version (the Windows installer's
        // `Blender 5.1`), otherwise ask the executable.
        let version = exe
            .parent()
            .and_then(|d| d.file_name())
            .and_then(|n| major_minor(&n.to_string_lossy()))
            .or_else(|| {
                let out = quiet(Command::new(&exe).arg("--version")).output().ok()?;
                major_minor(String::from_utf8_lossy(&out.stdout).lines().next()?)
            });
        if let Some(version) = version {
            found.push((version, exe));
        }
    }
    found
}

/// Whether any Blender is running right now.
fn blender_running() -> bool {
    if cfg!(target_os = "windows") {
        quiet(Command::new("tasklist").args(["/FI", "IMAGENAME eq blender.exe", "/NH"]))
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("blender.exe"))
    } else {
        let name = if cfg!(target_os = "macos") { "Blender" } else { "blender" };
        Command::new("pgrep")
            .args(["-x", name])
            .output()
            .is_ok_and(|o| o.status.success())
    }
}

/// Whether the add-on is enabled in a version's saved preferences. The
/// preferences file stores each enabled add-on's module name as plain text;
/// a false "no" (a compressed file, say) only costs a headless Blender run
/// that finds it already on.
fn enabled_in(version_dir: &Path) -> bool {
    std::fs::read(version_dir.join("config").join("userpref.blend"))
        .is_ok_and(|bytes| bytes.windows(MODULE.len()).any(|w| w == MODULE.as_bytes()))
}

/// Write the add-on if it is missing or different. `Ok(true)` when written.
fn write_addon(version_dir: &Path) -> Result<bool, String> {
    let dir = version_dir.join("scripts").join("addons");
    let path = dir.join(FILE_NAME);
    if std::fs::read_to_string(&path).is_ok_and(|existing| existing == ADDON) {
        return Ok(false);
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Through a temporary file: a running Blender polls this file to reload
    // itself, and must never read half of it.
    let tmp = dir.join(format!("{FILE_NAME}.tmp"));
    std::fs::write(&tmp, ADDON).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(true)
}

/// An earlier build of this plugin put the add-on in `scripts/startup`. Two
/// copies would both try to load, so that one goes.
fn remove_startup_copy(version_dir: &Path) {
    let old = version_dir.join("scripts").join("startup").join(FILE_NAME);
    if std::fs::read_to_string(&old).is_ok_and(|text| text.contains("Renzora Live Link")) {
        let _ = std::fs::remove_file(old);
    }
}

/// Enable the add-on in one version's preferences with a headless Blender.
fn enable(exe: &Path) -> Result<(), String> {
    let out = quiet(
        Command::new(exe)
            .args(["--background", "--python-expr", ENABLE_SCRIPT]),
    )
    .output()
    .map_err(|e| format!("could not start {} ({e})", exe.display()))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if stdout.lines().any(|l| l.trim() == "RENZORA_ENABLED") {
        Ok(())
    } else {
        let tail: String = stdout.lines().rev().take(5).collect::<Vec<_>>().join(" | ");
        Err(format!("Blender did not enable it ({tail})"))
    }
}

/// The whole pass, logged to the Console. Run off the main thread: a
/// headless Blender takes seconds to start.
pub fn run() {
    let versions = version_dirs();
    if versions.is_empty() {
        console_info(
            CATEGORY,
            "no Blender 4.2 or newer has been run on this machine, so the Blender add-on was not installed; start Blender once, then restart the editor",
        );
        return;
    }

    let mut written = Vec::new();
    let mut to_enable = Vec::new();
    for (version, dir) in &versions {
        remove_startup_copy(dir);
        match write_addon(dir) {
            Ok(true) => written.push(version.clone()),
            Ok(false) => {}
            Err(why) => {
                console_error(CATEGORY, format!("could not install the Blender add-on for Blender {version}: {why}"));
                continue;
            }
        }
        if !enabled_in(dir) {
            to_enable.push(version.clone());
        }
    }

    let updated: Vec<&String> = written.iter().filter(|v| !to_enable.contains(v)).collect();
    if !updated.is_empty() {
        console_info(
            CATEGORY,
            format!(
                "Blender add-on updated for Blender {}; an open Blender reloads it by itself",
                join(&updated)
            ),
        );
    }
    if to_enable.is_empty() {
        return;
    }

    let tick = "tick Renzora Live Link in Blender's Edit > Preferences > Add-ons (press Refresh if it is not listed); it loads at once";
    if blender_running() {
        console_info(
            CATEGORY,
            format!("Blender add-on installed for Blender {}. Blender is open, so {tick}", join(&to_enable.iter().collect::<Vec<_>>())),
        );
        return;
    }

    let executables = blender_executables();
    for version in to_enable {
        let Some((_, exe)) = executables.iter().find(|(v, _)| *v == version) else {
            console_info(
                CATEGORY,
                format!("Blender add-on installed for Blender {version}, but that Blender's program was not found to enable it; {tick}"),
            );
            continue;
        };
        match enable(exe) {
            Ok(()) => console_info(
                CATEGORY,
                format!("Blender add-on installed and enabled for Blender {version}: open the Renzora tab in the 3D Viewport sidebar (N)"),
            ),
            Err(why) => console_warn(
                CATEGORY,
                format!("Blender add-on installed for Blender {version} but not enabled ({why}); {tick}"),
            ),
        }
    }
}

fn join(versions: &[&String]) -> String {
    versions.iter().map(|v| v.as_str()).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::{major_minor, supported};

    #[test]
    fn versions_from_4_2_up() {
        assert!(supported("4.2"));
        assert!(supported("5.1"));
        assert!(!supported("4.1"));
        assert!(!supported("3.6"));
        assert!(!supported("config"));
    }

    #[test]
    fn version_from_folder_or_banner() {
        assert_eq!(major_minor("Blender 5.1").as_deref(), Some("5.1"));
        assert_eq!(major_minor("Blender 4.2.3 LTS").as_deref(), Some("4.2"));
        assert_eq!(major_minor("Blender Foundation"), None);
    }
}
