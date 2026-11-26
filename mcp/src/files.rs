//! Direct access to the project's files: list, read, write, delete, make
//! folders, make scenes and copy things in from elsewhere on disk.
//!
//! The editor's own routes to these all go through a native file dialog (Import
//! Files, Import Folder, Save As), and a native dialog is modal: it blocks the
//! whole editor until a person answers it, which an agent cannot do. These tools
//! do the same work without one.
//!
//! They are also how an agent brings in what the importer will not. The import
//! window keeps only the extensions it recognises (models, images, audio, fonts,
//! `.lua`) and silently drops the rest, so a folder of `.chart`, `.ini` or `.json`
//! data, or even a `.rs` script, arrives with holes in it. `copy_into_project`
//! copies every file verbatim, so a model copied this way is the raw file;
//! `import_model` is still the route that converts one.
//!
//! Every path stays inside the open project. A path is taken project-relative,
//! or absolute when it already points inside the project, and one that climbs
//! out with `..` is refused: these tools write, and a typo must not land a file
//! in the user's home folder. Only `copy_into_project`'s source may be anywhere.
//!
//! Nothing here touches the asset browser or the script compiler directly. The
//! project watcher sees each write, so the Assets panel shows a new file a beat
//! later and a `.rs` written here compiles as if it had been saved in the code
//! editor.

use bevy::prelude::*;
use renzora::serde_json::{json, Value};
use renzora::{CurrentProject, OpenScenePathRequested};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::Sender;

use crate::ToolReply;

/// Folders a listing or a copy never descends into: build output and version
/// control, which can hold hundreds of thousands of files and no assets.
const SKIP_DIRS: &[&str] = &["target", ".git", ".renzora", "node_modules"];

/// The most `read_file` returns. A model's context is the limit that matters,
/// and a file bigger than this is not one anybody reads whole.
const READ_LIMIT: usize = 256 * 1024;

/// Ceilings on one copy, so a wrong source (a whole drive, a home folder) is
/// refused rather than filling the disk.
const COPY_FILE_LIMIT: usize = 20_000;
const COPY_BYTE_LIMIT: u64 = 4 * 1024 * 1024 * 1024;

/// What an empty scene is: the header the BSN reader expects, and nothing else.
/// The same text the Assets panel's Scene (BSN) item writes.
const EMPTY_SCENE: &str = "// renzora interim bsn v1\n";

pub fn catalogue() -> Value {
    json!([
        {
            "name": "list_files",
            "description": "A project folder's contents, every file type included (list_assets shows only asset kinds). Use it to see what a copy or a write produced.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Project-relative folder (default the project root)." },
                    "depth": { "type": "integer", "description": "How many folder levels to show (default 1, at most 8)." },
                    "limit": { "type": "integer", "description": "Most entries to list (default 300)." }
                }
            }
        },
        {
            "name": "read_file",
            "description": "Read a text file in the project: a script, a scene, a UI template, a data file. Up to 256 KB.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Project-relative, or absolute inside the project." } },
                "required": ["path"]
            }
        },
        {
            "name": "write_file",
            "description": "Write a text file into the project, creating its folders: a .rs script (it compiles in the background, as on save; read_console reports it), a .html UI template, data, anything text. Refuses to replace an existing file unless overwrite is true. No dialog.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Project-relative, or absolute inside the project." },
                    "content": { "type": "string", "description": "The whole file." },
                    "overwrite": { "type": "boolean", "description": "Replace a file that is already there (default false)." }
                },
                "required": ["path", "content"]
            }
        },
        {
            "name": "edit_file",
            "description": "Change part of a text file in the project by exact find and replace, without sending the whole file: old must match the file exactly (whitespace included) and, unless all is true, exactly once. A .rs script recompiles as on save.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Project-relative, or absolute inside the project." },
                    "old": { "type": "string", "description": "The exact text to replace." },
                    "new": { "type": "string", "description": "What to put in its place." },
                    "all": { "type": "boolean", "description": "Replace every occurrence instead of requiring exactly one (default false)." }
                },
                "required": ["path", "old", "new"]
            }
        },
        {
            "name": "delete_file",
            "description": "Delete a file or folder in the project. Recoverable: it is moved into .renzora/deleted/<time>/ at the same path, out of the Assets panel and every listing, rather than erased. A folder with anything in it needs recursive true. The project root, project.toml and .renzora itself are refused. No dialog.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Project-relative, or absolute inside the project." },
                    "recursive": { "type": "boolean", "description": "Allow deleting a folder that is not empty, with everything in it (default false)." }
                },
                "required": ["path"]
            }
        },
        {
            "name": "create_folder",
            "description": "Make a folder in the project, and any folders above it. No dialog.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Project-relative folder path, such as songs/rock." } },
                "required": ["path"]
            }
        },
        {
            "name": "create_scene",
            "description": "Create a new empty scene file and open it in its own tab, ready to build in and save with save_scene. No Save As dialog.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Project-relative, such as scenes/level1.bsn (.bsn is added if left off)." },
                    "open": { "type": "boolean", "description": "Open it in a tab (default true)." }
                },
                "required": ["path"]
            }
        },
        {
            "name": "copy_into_project",
            "description": "Copy a file or a whole folder from anywhere on disk into the project, every file as it is, keeping the folder tree: audio, data, text, images, scripts. Unlike Import Folder it drops nothing and opens no dialog. Models arrive as raw files; run import_model on one to convert it. Existing files are kept unless overwrite is true. Answers when the copy is done.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Absolute path of the file or folder to copy." },
                    "dest": { "type": "string", "description": "Project folder to copy it into (default the project root); made if missing." },
                    "name": { "type": "string", "description": "Name for the copy inside dest (default the source's own name)." },
                    "overwrite": { "type": "boolean", "description": "Replace files that are already there (default false)." }
                },
                "required": ["source"]
            }
        }
    ])
}

pub fn is_file_tool(name: &str) -> bool {
    matches!(
        name,
        "list_files"
            | "read_file"
            | "write_file"
            | "edit_file"
            | "delete_file"
            | "create_folder"
            | "create_scene"
            | "copy_into_project"
    )
}

/// Run a file tool. `copy_into_project` answers from its own thread when the
/// copy is done; the rest answer at once.
pub fn run(world: &mut World, name: &str, args: &Value, reply: Sender<ToolReply>) {
    if name == "copy_into_project" {
        start_copy(world, args, reply);
        return;
    }
    let outcome = match name {
        "list_files" => list_files(world, args),
        "read_file" => read_file(world, args),
        "write_file" => write_file(world, args),
        "edit_file" => edit_file(world, args),
        "delete_file" => delete_file(world, args),
        "create_folder" => create_folder(world, args),
        "create_scene" => create_scene(world, args),
        other => Err(format!("no such tool: {other}")),
    };
    let _ = reply.send(match outcome {
        Ok(text) => ToolReply::Text(text),
        Err(why) => ToolReply::Failed(why),
    });
}

// ============================================================================
// Paths
// ============================================================================

fn project_root(world: &World) -> Result<PathBuf, String> {
    world
        .get_resource::<CurrentProject>()
        .map(|p| p.path.clone())
        .ok_or_else(|| "no project is open".into())
}

/// Resolve a caller's path to an absolute one inside the project.
///
/// Checked lexically, on the components, rather than by canonicalising: the
/// target of a write usually does not exist yet, and `canonicalize` fails on a
/// path that does not.
fn inside_project(root: &Path, raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    let given = PathBuf::from(trimmed);
    let relative = if given.is_absolute() {
        given
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .map_err(|_| format!("{trimmed} is outside the project ({})", root.display()))?
    } else {
        given
    };
    let mut clean = PathBuf::new();
    for part in relative.components() {
        match part {
            Component::Normal(p) => clean.push(p),
            Component::CurDir => {}
            Component::ParentDir => {
                if !clean.pop() {
                    return Err(format!("{trimmed} climbs out of the project"));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("{trimmed} is not a project path"));
            }
        }
    }
    Ok(root.join(clean))
}

fn shown(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/");
    if rel.is_empty() {
        ".".into()
    } else {
        rel
    }
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn arg_bool(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn size_text(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.1} KB", b as f64 / (1u64 << 10) as f64),
        b => format!("{b} B"),
    }
}

// ============================================================================
// Looking
// ============================================================================

/// A folder's contents, every file type included, which `list_assets` (filtered
/// by asset kind) does not show.
fn list_files(world: &World, args: &Value) -> Result<String, String> {
    let root = project_root(world)?;
    let dir = inside_project(&root, arg_str(args, "path").unwrap_or(""))?;
    if !dir.is_dir() {
        return Err(format!("no folder at {}", shown(&root, &dir)));
    }
    let depth = args.get("depth").and_then(Value::as_u64).unwrap_or(1).clamp(1, 8) as usize;
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(300).clamp(1, 5000) as usize;

    let mut lines = Vec::new();
    let mut truncated = false;
    walk_listing(&dir, depth, 0, limit, &mut lines, &mut truncated);
    if lines.is_empty() {
        return Ok(format!("{} is empty", shown(&root, &dir)));
    }
    let mut out = format!("{}:\n{}", shown(&root, &dir), lines.join("\n"));
    if truncated {
        out.push_str(&format!("\n(stopped at {limit} entries; pass a larger limit or a narrower path)"));
    }
    Ok(out)
}

fn walk_listing(
    dir: &Path,
    depth: usize,
    level: usize,
    limit: usize,
    lines: &mut Vec<String>,
    truncated: &mut bool,
) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read.flatten().collect();
    // Folders first, then by name, the way the Assets panel orders them.
    entries.sort_by_key(|e| (!e.path().is_dir(), e.file_name().to_string_lossy().to_lowercase()));
    for entry in entries {
        if lines.len() >= limit {
            *truncated = true;
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let indent = "  ".repeat(level);
        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                lines.push(format!("{indent}{name}/ (skipped)"));
                continue;
            }
            lines.push(format!("{indent}{name}/"));
            if level + 1 < depth {
                walk_listing(&path, depth, level + 1, limit, lines, truncated);
            }
        } else {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            lines.push(format!("{indent}{name}  {}", size_text(size)));
        }
    }
}

fn read_file(world: &World, args: &Value) -> Result<String, String> {
    let root = project_root(world)?;
    let raw = arg_str(args, "path").ok_or("read_file needs a path")?;
    let path = inside_project(&root, raw)?;
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", shown(&root, &path)))?;
    if bytes.len() > READ_LIMIT {
        return Err(format!(
            "{} is {}, more than read_file returns ({}); it is not a file to read whole",
            shown(&root, &path),
            size_text(bytes.len() as u64),
            size_text(READ_LIMIT as u64)
        ));
    }
    String::from_utf8(bytes).map_err(|_| format!("{} is not text", shown(&root, &path)))
}

// ============================================================================
// Writing
// ============================================================================

fn create_folder(world: &World, args: &Value) -> Result<String, String> {
    let root = project_root(world)?;
    let raw = arg_str(args, "path").ok_or("create_folder needs a path")?;
    let dir = inside_project(&root, raw)?;
    if dir.is_file() {
        return Err(format!("{} is a file", shown(&root, &dir)));
    }
    if dir.is_dir() {
        return Ok(format!("{} already exists", shown(&root, &dir)));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", shown(&root, &dir)))?;
    Ok(format!("created {}/", shown(&root, &dir)))
}

/// Write a text file. Refuses to replace an existing file unless told to, so a
/// guessed name never overwrites the user's work.
fn write_file(world: &World, args: &Value) -> Result<String, String> {
    let root = project_root(world)?;
    let raw = arg_str(args, "path").ok_or("write_file needs a path")?;
    let content = arg_str(args, "content").ok_or("write_file needs content")?;
    let path = inside_project(&root, raw)?;
    if path.is_dir() {
        return Err(format!("{} is a folder", shown(&root, &path)));
    }
    let existed = path.is_file();
    if existed && !arg_bool(args, "overwrite") {
        return Err(format!(
            "{} already exists; read_file it first, then pass overwrite true to replace it",
            shown(&root, &path)
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", shown(&root, parent)))?;
    }
    std::fs::write(&path, content).map_err(|e| format!("cannot write {}: {e}", shown(&root, &path)))?;

    let verb = if existed { "replaced" } else { "wrote" };
    let mut out = format!("{verb} {} ({})", shown(&root, &path), size_text(content.len() as u64));
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("rs")) {
        out.push_str("; it compiles in the background now, and read_console reports the result");
    }
    Ok(out)
}

/// Replace exact text in a file.
///
/// Exactly one match unless `all`, for the same reason an editor's find and
/// replace asks: a snippet that matches twice is ambiguous, and changing the
/// wrong copy of a repeated line is a bug nobody sees until it runs. The count
/// comes back so a caller replacing all can see how many it changed.
fn edit_file(world: &World, args: &Value) -> Result<String, String> {
    let root = project_root(world)?;
    let raw = arg_str(args, "path").ok_or("edit_file needs a path")?;
    let old = arg_str(args, "old").ok_or("edit_file needs old")?;
    let new = arg_str(args, "new").ok_or("edit_file needs new")?;
    if old.is_empty() {
        return Err("old is empty; use write_file to write a whole file".into());
    }
    if old == new {
        return Err("old and new are the same".into());
    }
    let path = inside_project(&root, raw)?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", shown(&root, &path)))?;
    let count = text.matches(old).count();
    let all = arg_bool(args, "all");
    match count {
        0 => {
            return Err(format!(
                "old was not found in {}; read_file it and copy the text exactly, whitespace included",
                shown(&root, &path)
            ))
        }
        n if n > 1 && !all => {
            return Err(format!(
                "old matches {n} places in {}; give more surrounding text so it matches one, or pass all true",
                shown(&root, &path)
            ))
        }
        _ => {}
    }
    let edited = if all { text.replace(old, new) } else { text.replacen(old, new, 1) };
    std::fs::write(&path, &edited).map_err(|e| format!("cannot write {}: {e}", shown(&root, &path)))?;
    let mut out = format!("replaced {count} place(s) in {}", shown(&root, &path));
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("rs")) {
        out.push_str("; it compiles in the background now, and read_console reports the result");
    }
    Ok(out)
}

/// Where a deleted file goes: `.renzora/deleted/`, under a folder per delete.
const DELETED_DIR: &str = ".renzora/deleted";

/// Delete a file or folder by moving it into [`DELETED_DIR`], at the path it
/// had in the project.
///
/// Moved rather than erased because the caller is an agent, and an agent's
/// wrong path should cost a rename to undo, not somebody's work. The editor's
/// own delete erases outright, and with a person choosing the file that is a
/// person's call; here nobody looked. `.renzora/` is already skipped by every
/// listing and by the asset browser, so a deleted file is gone from view at
/// once, and getting it back is moving it out again.
///
/// A rename within the project, so it is instant however big the folder and
/// never copies anything. A folder with anything in it needs `recursive`, so
/// a path one level too high does not take a whole tree with it unasked.
fn delete_file(world: &World, args: &Value) -> Result<String, String> {
    let root = project_root(world)?;
    let raw = arg_str(args, "path").ok_or("delete_file needs a path")?;
    let path = inside_project(&root, raw)?;
    let rel = path.strip_prefix(&root).unwrap_or(&path).to_path_buf();
    if rel.as_os_str().is_empty() {
        return Err("that is the project itself".into());
    }
    let first = rel.components().next().map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase());
    if first.as_deref() == Some(".renzora") {
        return Err(".renzora is the editor's own folder; nothing in it is deleted this way".into());
    }
    if rel.as_os_str().eq_ignore_ascii_case("project.toml") {
        return Err("project.toml is the project; deleting it would close it".into());
    }
    if !path.exists() {
        return Err(format!("nothing at {}", shown(&root, &path)));
    }
    let is_dir = path.is_dir();
    if is_dir && !arg_bool(args, "recursive") {
        let empty = std::fs::read_dir(&path).map(|mut d| d.next().is_none()).unwrap_or(false);
        if !empty {
            return Err(format!(
                "{} is a folder with things in it; pass recursive true to delete it and everything in it",
                shown(&root, &path)
            ));
        }
    }

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dest = root.join(DELETED_DIR).join(stamp.to_string()).join(&rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot make {}: {e}", shown(&root, parent)))?;
    }
    std::fs::rename(&path, &dest).map_err(|e| format!("cannot delete {}: {e}", shown(&root, &path)))?;

    let what = if is_dir { "folder" } else { "file" };
    Ok(format!(
        "deleted the {what} {}; it is kept at {} if it is wanted back",
        shown(&root, &path),
        shown(&root, &dest)
    ))
}

/// Write an empty scene file and, unless told not to, open it in a tab.
fn create_scene(world: &mut World, args: &Value) -> Result<String, String> {
    let root = project_root(world)?;
    let raw = arg_str(args, "path").ok_or("create_scene needs a path, such as scenes/level.bsn")?;
    let mut path = inside_project(&root, raw)?;
    if path.extension().is_none() {
        path.set_extension("bsn");
    }
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("bsn")) {
        return Err("a scene file ends in .bsn".into());
    }
    if path.exists() {
        return Err(format!(
            "{} already exists; open it with editor_request open_scene_path",
            shown(&root, &path)
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", shown(&root, parent)))?;
    }
    std::fs::write(&path, EMPTY_SCENE).map_err(|e| format!("cannot write {}: {e}", shown(&root, &path)))?;

    let open = args.get("open").and_then(Value::as_bool).unwrap_or(true);
    if open {
        world.insert_resource(OpenScenePathRequested(path.clone()));
        Ok(format!("created {} and opened it in a new tab", shown(&root, &path)))
    } else {
        Ok(format!("created {}", shown(&root, &path)))
    }
}

// ============================================================================
// Copying in
// ============================================================================

/// Copy a file or a whole folder from anywhere on disk into the project, every
/// file as it is.
///
/// Planned on the main thread (paths checked, the tree walked and sized) and
/// copied on a thread of its own, which answers the caller itself: a folder of
/// songs is hundreds of megabytes, and copying that inside a frame would freeze
/// the editor for as long as it took.
fn start_copy(world: &mut World, args: &Value, reply: Sender<ToolReply>) {
    let plan = match plan_copy(world, args) {
        Ok(plan) => plan,
        Err(why) => {
            let _ = reply.send(ToolReply::Failed(why));
            return;
        }
    };
    std::thread::Builder::new()
        .name("mcp-copy".into())
        .spawn(move || {
            let _ = reply.send(plan.run());
        })
        .map(|_| ())
        .unwrap_or_else(|e| warn!("[mcp] could not start the copy thread: {e}"));
}

struct CopyPlan {
    root: PathBuf,
    /// Where the source's top lands: the copied folder itself, or the file.
    dest: PathBuf,
    /// (from, to) for every file.
    files: Vec<(PathBuf, PathBuf)>,
    bytes: u64,
    overwrite: bool,
}

fn plan_copy(world: &World, args: &Value) -> Result<CopyPlan, String> {
    let root = project_root(world)?;
    let raw = arg_str(args, "source").ok_or("copy_into_project needs a source")?;
    let source = PathBuf::from(raw.trim());
    if !source.is_absolute() {
        return Err("source must be an absolute path".into());
    }
    if !source.exists() {
        return Err(format!("nothing at {}", source.display()));
    }
    if source.starts_with(&root) {
        return Err("that is already inside the project".into());
    }
    if root.starts_with(&source) {
        return Err("that folder holds the project itself, so copying it would copy forever".into());
    }

    let folder = inside_project(&root, arg_str(args, "dest").unwrap_or(""))?;
    if folder.is_file() {
        return Err(format!("{} is a file, not a folder", shown(&root, &folder)));
    }
    let name = source
        .file_name()
        .ok_or_else(|| format!("{} has no name to copy it under", source.display()))?;
    let dest = match arg_str(args, "name") {
        Some(rename) if !rename.trim().is_empty() => {
            let rename = rename.trim();
            if rename.contains(['/', '\\']) || rename == ".." {
                return Err("name is a single file or folder name".into());
            }
            folder.join(rename)
        }
        _ => folder.join(name),
    };

    let mut files = Vec::new();
    let mut bytes = 0u64;
    if source.is_file() {
        bytes = source.metadata().map(|m| m.len()).unwrap_or(0);
        files.push((source.clone(), dest.clone()));
    } else {
        collect_tree(&source, &dest, &mut files, &mut bytes)?;
    }
    if files.is_empty() {
        return Err(format!("{} has no files in it", source.display()));
    }

    Ok(CopyPlan {
        root,
        dest,
        files,
        bytes,
        overwrite: arg_bool(args, "overwrite"),
    })
}

fn collect_tree(
    from: &Path,
    to: &Path,
    files: &mut Vec<(PathBuf, PathBuf)>,
    bytes: &mut u64,
) -> Result<(), String> {
    let read = std::fs::read_dir(from).map_err(|e| format!("cannot read {}: {e}", from.display()))?;
    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        // Not followed: a link can point back up the tree and walk forever.
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if SKIP_DIRS.iter().any(|s| name.eq_ignore_ascii_case(s)) {
                continue;
            }
            collect_tree(&path, &to.join(&name), files, bytes)?;
        } else if kind.is_file() {
            *bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            files.push((path, to.join(&name)));
        }
        if files.len() > COPY_FILE_LIMIT || *bytes > COPY_BYTE_LIMIT {
            return Err(format!(
                "{} is too big to copy in one go (over {COPY_FILE_LIMIT} files or {}); copy a narrower folder",
                from.display(),
                size_text(COPY_BYTE_LIMIT)
            ));
        }
    }
    Ok(())
}

impl CopyPlan {
    fn run(self) -> ToolReply {
        let mut copied = 0usize;
        let mut kept = Vec::new();
        for (from, to) in &self.files {
            if to.exists() && !self.overwrite {
                kept.push(shown(&self.root, to));
                continue;
            }
            if let Some(parent) = to.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return ToolReply::Failed(format!(
                        "copied {copied} file(s), then could not create {}: {e}",
                        shown(&self.root, parent)
                    ));
                }
            }
            if let Err(e) = std::fs::copy(from, to) {
                return ToolReply::Failed(format!(
                    "copied {copied} file(s), then could not copy {}: {e}",
                    from.display()
                ));
            }
            copied += 1;
        }

        let mut out = format!(
            "copied {copied} file(s), {}, into {}",
            size_text(self.bytes),
            shown(&self.root, &self.dest)
        );
        if !kept.is_empty() {
            let sample: Vec<_> = kept.iter().take(5).cloned().collect();
            out.push_str(&format!(
                "; left {} existing file(s) as they were (pass overwrite true to replace): {}{}",
                kept.len(),
                sample.join(", "),
                if kept.len() > 5 { ", ..." } else { "" }
            ));
        }
        ToolReply::Text(out)
    }
}
