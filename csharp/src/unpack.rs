//! Unpacking the C# build a game carries in its archive.
//!
//! The export packs the build (the host, the scripts, their packages and, by
//! default, the .NET runtime) into the game's archive, so a single-file game
//! stays one file. .NET loads assemblies from files, though, and the runtime is
//! a native library that has to be on disk to be loaded at all, so the first
//! launch writes them out once into the player's cache folder:
//!
//! | | |
//! |---|---|
//! | Windows | `%LOCALAPPDATA%\<game>\csharp-<build>\` |
//! | macOS | `~/Library/Caches/<game>/csharp-<build>/` |
//! | Linux | `$XDG_CACHE_HOME/<game>/csharp-<build>/` (`~/.cache` by default) |
//!
//! `<build>` comes from the manifest the export wrote, which changes with any
//! file in the build, so an updated game unpacks next to the old one rather
//! than over files a running copy may hold open, and a second launch finds the
//! finished folder and unpacks nothing. A folder is written under a temporary
//! name and renamed when complete, so two copies of the game starting at once
//! cannot load a half-written build. Older builds' folders are removed.

use std::path::{Path, PathBuf};

use crate::compile;

/// The plugin's id, which its files are packed under.
const PLUGIN: &str = "csharp";
const DONE: &str = ".ok";

/// The archive key of a file of the build.
fn key(rel: &str) -> String {
    renzora::plugin_archive_path(PLUGIN, rel)
}

/// Whether this game carries a C# build in its archive. A small read, made
/// on the caller's thread; the unpacking is [`unpack`].
pub fn in_archive() -> bool {
    renzora::has_game_archive() && renzora::game_archive_read(&key(&compile::export_manifest_path())).is_some()
}

/// The per-user cache folder for this game.
fn cache_root() -> Option<PathBuf> {
    let game = std::env::current_exe()
        .ok()?
        .file_stem()?
        .to_string_lossy()
        .into_owned();
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library").join("Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
    }?;
    Some(base.join(game))
}

/// Write the build out of the archive, or find it already written. Returns
/// the folder to treat as the plugin's own (it holds `export/`).
pub fn unpack() -> Result<PathBuf, String> {
    let manifest = renzora::game_archive_read(&key(&compile::export_manifest_path()))
        .ok_or_else(|| "this game's archive holds no C# build".to_string())?;
    let build = String::from_utf8_lossy(&manifest)
        .lines()
        .find_map(|l| l.strip_prefix("build=").map(|s| s.trim().to_string()))
        .unwrap_or_else(|| compile::hex(compile::Fnv::default().bytes(&manifest).finish()));
    let root = cache_root().ok_or_else(|| "no cache folder to unpack this game's C# build into".to_string())?;
    let dir = root.join(format!("csharp-{build}"));
    if dir.join(DONE).is_file() {
        return Ok(dir);
    }

    let tmp = root.join(format!("csharp-{build}.tmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let prefix = key("");
    let keys = renzora::game_archive_paths(&prefix);
    for k in &keys {
        let rel = &k[prefix.len()..];
        let Some(dest) = safe_join(&tmp, rel) else { continue };
        let bytes = renzora::game_archive_read(k).ok_or_else(|| format!("could not read {k} from the game"))?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        std::fs::write(&dest, bytes).map_err(|e| format!("could not write {}: {e}", dest.display()))?;
    }
    std::fs::write(tmp.join(DONE), b"").map_err(|e| format!("could not finish {}: {e}", tmp.display()))?;
    let used = match std::fs::rename(&tmp, &dir) {
        Ok(()) => dir,
        // Another copy of the game finished first; use its folder.
        Err(_) if dir.join(DONE).is_file() => {
            let _ = std::fs::remove_dir_all(&tmp);
            dir
        }
        // A damaged folder is in the way: one a cleaner emptied half of while
        // the game had files open, so it lost its marker but kept the locked
        // files. Clear it and try again; when it will not clear, run from the
        // fresh copy, which is complete, rather than not at all.
        Err(_) => {
            let _ = std::fs::remove_dir_all(&dir);
            if std::fs::rename(&tmp, &dir).is_ok() {
                dir
            } else {
                bevy::log::warn!(
                    "[C#] {} is damaged and in use; running from {} instead",
                    dir.display(),
                    tmp.display()
                );
                tmp
            }
        }
    };
    bevy::log::info!("[C#] unpacked this game's C# build ({} files) into {}", keys.len(), used.display());
    remove_old(&root, &used);
    Ok(used)
}

/// `rel` under `base`, refusing anything that would leave it.
fn safe_join(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut out = base.to_path_buf();
    for part in rel.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            p if p.contains(':') || p.contains('\\') => return None,
            p => out.push(p),
        }
    }
    Some(out)
}

/// Remove earlier builds' folders, best effort: one still open in a running
/// copy of an older version stays until a later launch. A half-written
/// `.tmp-` folder goes too once it is an hour old: by then the launch that
/// wrote it has crashed or finished, and one still being written is minutes old.
fn remove_old(root: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p == keep || !p.is_dir() || !name.starts_with("csharp-") {
            continue;
        }
        if name.contains(".tmp-") {
            let stale = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() > 3600);
            if !stale {
                continue;
            }
        }
        let _ = std::fs::remove_dir_all(&p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packed_path_cannot_climb_out() {
        let base = Path::new("cache");
        assert_eq!(safe_join(base, "export/host/a.dll"), Some(base.join("export").join("host").join("a.dll")));
        assert!(safe_join(base, "export/../../evil.dll").is_none());
        assert!(safe_join(base, "C:/evil.dll").is_none());
    }
}
