//! Tools that put project content into the scene: materials, models, particle
//! effects, UI templates and the editor's own Add Entity presets.
//!
//! Every one of these ends in a component that names a FILE (`MaterialRef`,
//! `MeshInstanceData`, `HanabiEffect`, `HtmlTemplatePath`), because the file is
//! what a saved scene keeps. A handle to an asset made in memory survives until
//! the scene is reloaded and then comes back as nothing, so none of these build
//! one: they write or copy a file into the project and point the entity at it.
//!
//! That is also why [`bring_into_project`] exists. An agent usually knows a file
//! by an absolute path somewhere else (a download, the engine's own presets), and
//! a scene that references a path outside its project breaks the moment the
//! project moves. So a file from outside is copied in first, and the entity
//! points at the copy.

use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::*;
use bevy::reflect::enums::{DynamicEnum, DynamicVariant};
use bevy::reflect::structs::DynamicStruct;
use bevy::reflect::PartialReflect;
use renzora::serde_json::Value;
use renzora::{
    unique_entity_name, CurrentProject, DefaultCamera, ImportInPlaceQueue, MeshInstanceData,
    PbrAdvanced, PbrAlphaMode, PbrMaterialExtracted, SceneCamera, SpawnRegistry,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::tools::{arg_entity, arg_vec3, entity_id, find_type};
use crate::{edits, ToolReply};

// ============================================================================
// Files
// ============================================================================

/// What each `list_assets` kind means, by file extension.
const KINDS: &[(&str, &[&str])] = &[
    ("materials", &["material"]),
    (
        "models",
        &["glb", "gltf", "fbx", "obj", "stl", "ply", "dae", "usd", "usda", "usdc", "usdz", "abc"],
    ),
    ("particles", &["particle"]),
    ("ui", &["html"]),
    ("scripts", &["rs", "lua"]),
    ("scenes", &["bsn"]),
    ("textures", &["png", "jpg", "jpeg", "ktx2", "hdr", "exr", "tga", "webp"]),
    ("audio", &["ogg", "wav", "mp3", "flac"]),
];

/// Folders a project walk never descends into. `target` and `build` can hold
/// hundreds of thousands of files, and nothing in them is an asset.
const SKIP_DIRS: &[&str] = &["target", "build", ".git", ".renzora", "node_modules"];

fn project_root(world: &World) -> Result<PathBuf, String> {
    world
        .get_resource::<CurrentProject>()
        .map(|p| p.path.clone())
        .ok_or_else(|| "no project is open".into())
}

/// The engine's own asset folder (particle presets, UI templates, materials),
/// when the editor was started from a checkout.
///
/// Found the way the particle editor's Presets menu finds it: `assets/` under
/// the working directory. An installed editor has none, and then the only
/// presets are whatever the project already holds.
fn engine_assets() -> Option<PathBuf> {
    let dir = std::env::current_dir().ok()?.join("assets");
    dir.is_dir().then_some(dir)
}

/// A project-relative, forward-slashed path for `path`, which must be inside
/// `root`.
fn relative_to(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// Resolve a file the caller named into a project-relative path, copying it in
/// from outside the project when it has to.
///
/// Tried in order: a path inside the project (absolute or relative), an absolute
/// path anywhere else (copied into `subdir`), and a relative path into the
/// engine's presets (copied likewise). The copy keeps its file name and gains a
/// numeric suffix only when a DIFFERENT file already has that name; the same
/// file brought in twice reuses the first copy rather than piling up duplicates.
pub fn bring_into_project(world: &World, raw: &str, subdir: &str) -> Result<String, String> {
    let root = project_root(world)?;
    let given = PathBuf::from(raw.trim());

    let outside = if given.is_absolute() {
        if let Some(rel) = relative_to(&root, &given) {
            return if given.is_file() {
                Ok(rel)
            } else {
                Err(format!("no file at {raw}"))
            };
        }
        given
    } else {
        if root.join(&given).is_file() {
            // Already in the project, but perhaps copied before its references
            // were: fill in whatever is missing, never replacing what is there.
            copy_dependencies(&root.join(&given), &root, engine_assets().as_deref(), &mut Vec::new());
            return Ok(given.to_string_lossy().replace('\\', "/"));
        }
        let preset = engine_assets().and_then(|assets| {
            [assets.join(&given), assets.join(subdir).join(&given)]
                .into_iter()
                .find(|p| p.is_file())
        });
        match preset {
            Some(preset) => preset,
            None => {
                return Err(format!(
                    "no file called {raw} in the project or the engine presets; list_assets shows what exists"
                ))
            }
        }
    };

    if !outside.is_file() {
        return Err(format!("no file at {}", outside.display()));
    }
    let dest = copy_in(&outside, &root.join(subdir))?;
    let source_root = engine_assets().filter(|assets| outside.starts_with(assets));
    copy_dependencies(&dest, &root, source_root.as_deref(), &mut Vec::new());
    relative_to(&root, &dest).ok_or_else(|| "the copy landed outside the project".into())
}

/// Copy in every file a copied file refers to, and every file those refer to.
///
/// A file alone is often not the whole asset. A UI template mounts shared
/// components by path (`<node template="ui/components/stat_bar.html">`) and the
/// loader waits for every one of them before it builds anything, so `hud.html`
/// copied without `stat_bar.html` sat on its canvas building nothing, forever,
/// with no error. 29 of the engine's 138 templates are like that, and a material
/// can name its textures the same way.
///
/// References are found without parsing any format: every quoted string in the
/// file that names a file under `source_root` is one. That is loose on purpose.
/// It covers HTML attributes, a material's JSON and a particle's RON with one
/// rule, and a quoted string that merely looks like a path costs nothing unless
/// a file of exactly that name exists. Each lands at the SAME relative path in
/// the project, because that is the path the referring file uses. A file already
/// in the project is left alone, so a user's edited copy is never overwritten.
fn copy_dependencies(file: &Path, root: &Path, source_root: Option<&Path>, seen: &mut Vec<PathBuf>) {
    let Some(source_root) = source_root else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(file) else {
        return;
    };
    for reference in text.split('"').skip(1).step_by(2) {
        if reference.is_empty() || reference.contains('{') || reference.len() > 260 {
            continue;
        }
        let from = source_root.join(reference);
        let to = root.join(reference);
        if seen.contains(&to) || !from.is_file() {
            continue;
        }
        seen.push(to.clone());
        if !to.exists() {
            if let Some(parent) = to.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::copy(&from, &to).is_err() {
                continue;
            }
        }
        copy_dependencies(&to, root, Some(source_root), seen);
    }
}

/// Copy one file into `dir`, returning where it went.
fn copy_in(source: &Path, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let ext = source
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    let incoming = std::fs::read(source).map_err(|e| format!("could not read {}: {e}", source.display()))?;
    for n in 1.. {
        let name = if n == 1 {
            format!("{stem}{ext}")
        } else {
            format!("{stem}_{n}{ext}")
        };
        let dest = dir.join(name);
        match std::fs::read(&dest) {
            Ok(existing) if existing == incoming => return Ok(dest),
            Ok(_) => continue,
            Err(_) => {
                std::fs::write(&dest, &incoming)
                    .map_err(|e| format!("could not write {}: {e}", dest.display()))?;
                return Ok(dest);
            }
        }
    }
    unreachable!("the suffix loop only ends by returning")
}

/// List the project's files of one kind, or the engine's presets.
pub fn list_assets(world: &World, args: &Value) -> ToolReply {
    let kind = args.get("kind").and_then(Value::as_str).unwrap_or("all");
    let exts: Vec<&str> = if kind == "all" {
        KINDS.iter().flat_map(|(_, e)| e.iter().copied()).collect()
    } else {
        match KINDS.iter().find(|(k, _)| *k == kind) {
            Some((_, e)) => e.to_vec(),
            None => {
                let known: Vec<&str> = KINDS.iter().map(|(k, _)| *k).collect();
                return ToolReply::Failed(format!(
                    "no kind called {kind}; use all, {}",
                    known.join(", ")
                ));
            }
        }
    };
    let contains = args
        .get("contains")
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(200) as usize;

    let engine = args.get("source").and_then(Value::as_str) == Some("engine");
    let root = if engine {
        match engine_assets() {
            Some(root) => root,
            None => {
                return ToolReply::Failed(
                    "this editor was not started from an engine checkout, so there are no engine presets to list".into(),
                )
            }
        }
    } else {
        match project_root(world) {
            Ok(root) => root,
            Err(why) => return ToolReply::Failed(why),
        }
    };

    let mut found = Vec::new();
    walk(&root, &root, &exts, contains.as_deref(), &mut found);
    found.sort();
    let total = found.len();
    found.truncate(limit);

    let mut out = if engine {
        format!("{total} engine presets of kind {kind}; pass one to a tool by this path and it is copied into the project\n")
    } else {
        format!("{total} project files of kind {kind}\n")
    };
    for path in &found {
        out.push_str(path);
        out.push('\n');
    }
    if total > found.len() {
        out.push_str(&format!("... {} more; narrow with contains\n", total - found.len()));
    }
    ToolReply::Text(out)
}

fn walk(root: &Path, dir: &Path, exts: &[&str], contains: Option<&str>, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                walk(root, &path, exts, contains, out);
            }
            continue;
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !exts.contains(&ext.as_str()) {
            continue;
        }
        let Some(rel) = relative_to(root, &path) else {
            continue;
        };
        if contains.is_some_and(|c| !rel.to_lowercase().contains(c)) {
            continue;
        }
        out.push(rel);
    }
}

// ============================================================================
// Materials
// ============================================================================

/// Point an entity's meshes at a `.material`, writing a new one first when the
/// caller gave colours rather than a file.
pub fn set_material(world: &mut World, args: &Value) -> ToolReply {
    let Some(entity) = arg_entity(args, "entity") else {
        return ToolReply::Failed("set_material needs an entity id".into());
    };
    if world.get_entity(entity).is_err() {
        return ToolReply::Failed(format!("{} is not a live entity", entity_id(entity)));
    }

    let (path, created) = match args.get("path").and_then(Value::as_str) {
        Some(raw) => match bring_into_project(world, raw, "materials") {
            Ok(rel) if rel.ends_with(".material") => (rel, false),
            Ok(rel) => return ToolReply::Failed(format!("{rel} is not a .material file")),
            Err(why) => return ToolReply::Failed(why),
        },
        None => match write_pbr_material(world, args) {
            Ok(rel) => (rel, true),
            Err(why) => return ToolReply::Failed(why),
        },
    };

    // The entity itself when it is a mesh, and every mesh under it too unless
    // told otherwise: an imported model is a group whose meshes are its
    // children, and "make the chair red" means all of them.
    let recursive = args
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let mut targets = Vec::new();
    let mut stack = vec![entity];
    while let Some(at) = stack.pop() {
        if world.get::<Mesh3d>(at).is_some() {
            targets.push(at);
        }
        if recursive {
            if let Some(children) = world.get::<Children>(at) {
                stack.extend(children.iter());
            }
        }
    }
    if targets.is_empty() {
        return ToolReply::Failed(format!(
            "{} has no mesh{}; a material goes on a mesh",
            entity_id(entity),
            if recursive { " and none under it" } else { "" }
        ));
    }

    let count = targets.len();
    edits::set_material(world, targets, Some(path.clone()));
    let verb = if created { "wrote" } else { "used" };
    ToolReply::Text(format!(
        "{verb} {path} and put it on {count} mesh{} under {}",
        if count == 1 { "" } else { "es" },
        entity_id(entity)
    ))
}

/// Write a PBR `.material` from the arguments, and return its project path.
///
/// Through the engine's own `PbrMaterialExtracted` observer, the one the model
/// importer uses, so the file is exactly what an import would have written and
/// this plugin does not carry a second copy of the material graph format.
///
/// The file name is always new. The engine caches a material by path and a
/// plugin has no way to tell it a file changed, so rewriting `red.material`
/// would leave every entity using it showing the old red.
fn write_pbr_material(world: &mut World, args: &Value) -> Result<String, String> {
    let root = project_root(world)?;
    let dir = root.join("materials");

    let wanted = args.get("name").and_then(Value::as_str).unwrap_or("material");
    // The observer replaces these characters itself; doing it here first is what
    // lets this know the exact file name it will write.
    let safe: String = wanted
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    let safe = if safe.is_empty() { "material".to_string() } else { safe };
    let name = (1..)
        .map(|n| if n == 1 { safe.clone() } else { format!("{safe}_{n}") })
        .find(|n| !dir.join(format!("{n}.material")).exists())
        .unwrap_or(safe);

    let number = |key: &str, fallback: f32| {
        args.get(key)
            .and_then(Value::as_f64)
            .map(|v| v as f32)
            .unwrap_or(fallback)
    };
    let color = args
        .get("base_color")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_f64).map(|v| v as f32).collect::<Vec<_>>())
        .unwrap_or_default();
    let base_color = match color.as_slice() {
        [r, g, b] => [*r, *g, *b, 1.0],
        [r, g, b, a] => [*r, *g, *b, *a],
        [] => [0.8, 0.8, 0.8, 1.0],
        _ => return Err("base_color is three or four numbers, linear 0..1".into()),
    };
    let emissive = arg_vec3(args, "emissive").unwrap_or(Vec3::ZERO);
    let alpha_mode = if base_color[3] < 1.0 {
        PbrAlphaMode::Blend
    } else {
        PbrAlphaMode::Opaque
    };

    world.trigger(PbrMaterialExtracted {
        name: name.clone(),
        output_dir: dir.clone(),
        project_root: root.clone(),
        base_color,
        metallic: number("metallic", 0.0),
        roughness: number("roughness", 0.5),
        emissive: emissive.to_array(),
        base_color_texture: None,
        normal_texture: None,
        metallic_roughness_texture: None,
        roughness_texture: None,
        metallic_texture: None,
        emissive_texture: None,
        occlusion_texture: None,
        specular_glossiness_texture: None,
        opacity_texture: None,
        specular_texture: None,
        advanced: PbrAdvanced::default(),
        alpha_mode,
        alpha_cutoff: 0.5,
        double_sided: args
            .get("double_sided")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    });

    // The observer runs inside `trigger`, so the file is on disk now or never.
    let file = dir.join(format!("{name}.material"));
    if !file.is_file() {
        return Err(
            "the engine did not write the material; this build may have no material system (read_console may say why)".into(),
        );
    }
    relative_to(&root, &file).ok_or_else(|| "the material landed outside the project".into())
}

// ============================================================================
// Models
// ============================================================================

/// An import that has been queued and not yet answered.
pub struct PendingImport {
    /// The path pushed onto the queue, to tell when the engine has taken it.
    queued: PathBuf,
    glb: PathBuf,
    root: PathBuf,
    reply: Sender<ToolReply>,
    deadline: Instant,
}

#[derive(Resource, Default)]
pub struct PendingImports(pub Vec<PendingImport>);

/// Shorter than the RPC's own import timeout, so the caller hears this
/// module's specific answer rather than a generic one.
const IMPORT_DEADLINE: Duration = Duration::from_secs(110);

/// Copy a model into the project and queue it for the engine's importer.
///
/// Answers later, from [`collect_imports`], because the import runs in an engine
/// system on a following frame. That keeps it to one call: the caller gets the
/// `.glb` path to spawn, not a promise to go and poll for one.
pub fn start_import(world: &mut World, args: &Value, reply: Sender<ToolReply>) {
    let result = queue_import(world, args);
    match result {
        Ok(pending) => {
            let Some(mut pending_imports) = world.get_resource_mut::<PendingImports>() else {
                let _ = reply.send(ToolReply::Failed("the import tracker is missing".into()));
                return;
            };
            pending_imports.0.push(PendingImport { reply, ..pending });
        }
        Err(why) => {
            let _ = reply.send(ToolReply::Failed(why));
        }
    }
}

fn queue_import(world: &mut World, args: &Value) -> Result<PendingImport, String> {
    let Some(raw) = args.get("path").and_then(Value::as_str) else {
        return Err("import_model needs a path".into());
    };
    let root = project_root(world)?;
    let source = PathBuf::from(raw.trim());
    let source = if source.is_absolute() { source } else { root.join(source) };
    if !source.is_file() {
        return Err(format!("no file at {}", source.display()));
    }
    let ext = source
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let models = KINDS.iter().find(|(k, _)| *k == "models").map(|(_, e)| *e).unwrap_or(&[]);
    if !models.contains(&ext.as_str()) {
        return Err(format!(".{ext} is not a model format the importer reads"));
    }

    let target = if source.starts_with(&root) {
        // Already in the project: the importer converts it where it stands. It
        // also deletes a non-GLB source afterwards, which is the engine's rule
        // for an in-project model and the reason outside files are copied.
        source
    } else {
        let stem = source
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "model".into());
        let folder = args.get("folder").and_then(Value::as_str).unwrap_or("models");
        let dir = (1..)
            .map(|n| {
                root.join(folder)
                    .join(if n == 1 { stem.clone() } else { format!("{stem}_{n}") })
            })
            .find(|d| !d.exists())
            .unwrap_or_else(|| root.join(folder).join(&stem));
        copy_model(&source, &dir)?
    };

    let glb = target.with_extension("glb");
    world
        .get_resource_or_insert_with(ImportInPlaceQueue::default)
        .0
        .push(target.clone());

    Ok(PendingImport {
        queued: target,
        glb,
        root,
        reply: std::sync::mpsc::channel().0,
        deadline: Instant::now() + IMPORT_DEADLINE,
    })
}

/// Copy a model, and for the formats that keep their data in sibling files, the
/// folder it sits in.
///
/// A `.gltf` names its `.bin` and textures by relative path, and an `.obj` its
/// `.mtl`, so copying the file alone would import geometry with no buffers or no
/// materials. The whole folder goes, but only when it is small enough to be the
/// model's own: a `.gltf` sitting loose in Downloads must not drag the rest of
/// Downloads with it.
fn copy_model(source: &Path, dir: &Path) -> Result<PathBuf, String> {
    const MAX_SIBLINGS: usize = 400;
    let ext = source
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let parent = source.parent().unwrap_or(Path::new("."));

    if matches!(ext.as_str(), "gltf" | "obj") {
        let mut count = 0;
        count_files(parent, &mut count, MAX_SIBLINGS + 1);
        if count <= MAX_SIBLINGS {
            copy_tree(parent, dir)?;
            return Ok(dir.join(source.file_name().unwrap_or_default()));
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let dest = dir.join(source.file_name().unwrap_or_default());
    std::fs::copy(source, &dest).map_err(|e| format!("could not copy the model: {e}"))?;
    Ok(dest)
}

fn count_files(dir: &Path, count: &mut usize, stop_at: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if *count >= stop_at {
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            count_files(&path, count, stop_at);
        } else {
            *count += 1;
        }
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("could not create {}: {e}", to.display()))?;
    let entries = std::fs::read_dir(from).map_err(|e| format!("could not read {}: {e}", from.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let dest = to.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &dest)?;
        } else {
            std::fs::copy(&path, &dest).map_err(|e| format!("could not copy {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Answer each import once the engine has taken it off the queue.
///
/// The engine's drain runs the whole import inside one exclusive system, so
/// "no longer queued" means "finished", and the `.glb` on disk is the verdict.
pub fn collect_imports(world: &mut World) {
    if world
        .get_resource::<PendingImports>()
        .is_none_or(|p| p.0.is_empty())
    {
        return;
    }
    let queued: Vec<PathBuf> = world
        .get_resource::<ImportInPlaceQueue>()
        .map(|q| q.0.clone())
        .unwrap_or_default();
    let now = Instant::now();

    let Some(mut pending) = world.get_resource_mut::<PendingImports>() else {
        return;
    };
    pending.0.retain(|import| {
        if queued.contains(&import.queued) {
            if now >= import.deadline {
                let _ = import.reply.send(ToolReply::Failed(
                    "the import was never picked up; this build may have no importer".into(),
                ));
                return false;
            }
            return true;
        }
        let reply = match relative_to(&import.root, &import.glb) {
            Some(rel) if is_glb(&import.glb) => ToolReply::Text(format!(
                "imported as {rel}; spawn it with spawn_model. Its materials are in the materials folder beside it"
            )),
            Some(rel) if import.glb.is_file() => ToolReply::Failed(format!(
                "the importer could not convert this model. It left the original bytes at {rel} under a .glb name, which will not load; delete it. The engine logs the reason to its log file, not the Console"
            )),
            _ => ToolReply::Failed(
                "the importer ran but wrote no .glb; read_console may have its error".into(),
            ),
        };
        let _ = import.reply.send(reply);
        false
    });
}

/// Whether a file really is a binary glTF, by its magic bytes.
///
/// The extension cannot be trusted. When conversion fails the engine falls back
/// to copying the source under the `.glb` name, so an FBX that could not be read
/// arrives as `Running.glb` with `Kaydara FBX Binary` inside it. Reporting that
/// as imported sent the caller off to spawn a model that loads as nothing.
fn is_glb(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && &magic == b"glTF"
}

/// Place an imported model in the scene.
pub fn spawn_model(world: &mut World, args: &Value) -> ToolReply {
    let entity = match spawn_model_from_args(world, args) {
        Ok(entity) => entity,
        Err(why) => return ToolReply::Failed(why),
    };
    let again = args.clone();
    edits::record_spawn(world, entity, "spawn model", move |world| {
        spawn_model_from_args(world, &again).ok()
    });
    let name = world
        .get::<Name>(entity)
        .map(|n| n.as_str().to_string())
        .unwrap_or_default();
    ToolReply::Text(format!("spawned {name} [{}]", entity_id(entity)))
}

fn spawn_model_from_args(world: &mut World, args: &Value) -> Result<Entity, String> {
    let Some(raw) = args.get("path").and_then(Value::as_str) else {
        return Err("spawn_model needs a path".into());
    };
    let rel = bring_into_project(world, raw, "models")?;
    let lower = rel.to_lowercase();
    if !(lower.ends_with(".glb") || lower.ends_with(".gltf")) {
        return Err(format!(
            "{rel} is not a .glb; import_model converts it first and answers with the path to spawn"
        ));
    }
    if lower.ends_with(".glb") && !is_glb(&project_root(world)?.join(&rel)) {
        return Err(format!(
            "{rel} is not really a GLB (a failed import leaves the source bytes under that name); import the original again or delete it"
        ));
    }
    let parent = checked_parent(world, args)?;

    let stem = Path::new(&rel)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "model".into());
    let entity = world
        .spawn((
            placed(args),
            Visibility::default(),
            // A bare instance with no children: the engine's rehydrate step sees
            // it added, loads the file and builds the hierarchy under it, the
            // same path a saved scene takes on load.
            MeshInstanceData {
                model_path: Some(rel),
            },
        ))
        .id();
    name_entity(world, entity, args, &stem);
    if let Some(parent) = parent {
        world.entity_mut(entity).insert(ChildOf(parent));
    }
    Ok(entity)
}

// ============================================================================
// Particles
// ============================================================================

/// Spawn an entity playing a `.particle` effect.
pub fn spawn_particles(world: &mut World, args: &Value) -> ToolReply {
    let entity = match spawn_particles_from_args(world, args) {
        Ok(entity) => entity,
        Err(why) => return ToolReply::Failed(why),
    };
    let again = args.clone();
    edits::record_spawn(world, entity, "spawn particles", move |world| {
        spawn_particles_from_args(world, &again).ok()
    });
    let name = world
        .get::<Name>(entity)
        .map(|n| n.as_str().to_string())
        .unwrap_or_default();
    ToolReply::Text(format!("spawned {name} [{}]", entity_id(entity)))
}

fn spawn_particles_from_args(world: &mut World, args: &Value) -> Result<Entity, String> {
    let Some(raw) = args.get("path").and_then(Value::as_str) else {
        return Err("spawn_particles needs the path of a .particle file".into());
    };
    let rel = bring_into_project(world, raw, "particles")?;
    if !rel.to_lowercase().ends_with(".particle") {
        return Err(format!("{rel} is not a .particle file"));
    }
    let parent = checked_parent(world, args)?;

    // `HanabiEffect` lives in a crate a plugin cannot link, and it has no
    // reflected `Default`, so it is built here as a dynamic value with every
    // field and turned into the real type by `FromReflect` on insert. The one
    // field left out, `variable_overrides`, is `#[reflect(ignore)]` and takes
    // its default.
    let number = |key: &str, fallback: f32| {
        args.get(key)
            .and_then(Value::as_f64)
            .map(|v| v as f32)
            .unwrap_or(fallback)
    };
    let mut source = DynamicStruct::default();
    source.insert("path", rel.clone());
    let mut effect = DynamicStruct::default();
    effect.insert("source", DynamicEnum::new("Asset", DynamicVariant::Struct(source)));
    effect.insert("playing", true);
    effect.insert("rate_multiplier", number("rate", 1.0));
    // `size` is the effect's own scale. Particles ignore the entity's transform
    // scale, so the vec3 `scale` argument (which `placed` reads) resizes nothing
    // visible; a bare number under `scale` is read as a size too, because that is
    // what a caller passing one meant.
    let size = args
        .get("size")
        .or_else(|| args.get("scale").filter(|v| v.is_number()))
        .and_then(Value::as_f64)
        .map_or(1.0, |v| v as f32);
    effect.insert("scale_multiplier", size);
    effect.insert("color_tint", [1.0f32, 1.0, 1.0, 1.0]);
    effect.insert("time_scale", number("time_scale", 1.0));

    let stem = Path::new(&rel)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "particles".into());
    let entity = world.spawn((placed(args), Visibility::default())).id();

    if let Err(why) = insert_dynamic(world, entity, "HanabiEffect", &effect) {
        world.entity_mut(entity).despawn();
        return Err(why);
    }
    name_entity(world, entity, args, &stem);
    if let Some(parent) = parent {
        world.entity_mut(entity).insert(ChildOf(parent));
    }
    Ok(entity)
}

/// Insert a component this plugin can only name, from a dynamic value.
fn insert_dynamic(
    world: &mut World,
    entity: Entity,
    component: &str,
    value: &dyn PartialReflect,
) -> Result<(), String> {
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();
    let Some(reflect_component) = find_type(&registry, component).and_then(|r| r.data::<ReflectComponent>()) else {
        return Err(format!(
            "this build has no {component} component; the plugin that provides it is not loaded"
        ));
    };
    let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
        return Err(format!("{} is not a live entity", entity_id(entity)));
    };
    reflect_component.insert(&mut entity_mut, value, &registry);
    Ok(())
}

// ============================================================================
// Game UI
// ============================================================================

/// Put an `.html` UI template on screen, on a canvas of its own.
pub fn add_ui(world: &mut World, args: &Value) -> ToolReply {
    let entity = match add_ui_from_args(world, args) {
        Ok(entity) => entity,
        Err(why) => return ToolReply::Failed(why),
    };
    let again = args.clone();
    edits::record_spawn(world, entity, "add ui", move |world| {
        add_ui_from_args(world, &again).ok()
    });
    let name = world
        .get::<Name>(entity)
        .map(|n| n.as_str().to_string())
        .unwrap_or_default();
    ToolReply::Text(format!("added UI canvas {name} [{}]", entity_id(entity)))
}

fn add_ui_from_args(world: &mut World, args: &Value) -> Result<Entity, String> {
    let Some(raw) = args.get("template").and_then(Value::as_str) else {
        return Err("add_ui needs a template, the path of an .html file".into());
    };
    let rel = bring_into_project(world, raw, "ui")?;
    if !rel.to_lowercase().ends_with(".html") {
        return Err(format!("{rel} is not an .html template"));
    }
    let root = project_root(world)?;
    // The engine's own helper, which the viewport's template drop uses: it knows
    // the canvas's layout node and the naming rule, and this should not keep a
    // second copy of either.
    let entity = renzora_ember::game_ui::spawn::spawn_ui_canvas_with_template(world, &root.join(&rel));
    if let Some(name) = args.get("name").and_then(Value::as_str) {
        let unique = unique_entity_name(world, name, entity);
        world.entity_mut(entity).insert(Name::new(unique));
    }
    Ok(entity)
}

// ============================================================================
// Presets and cameras
// ============================================================================

/// Everything the editor's Add Entity menu offers.
pub fn list_presets(world: &World) -> ToolReply {
    let Some(registry) = world.get_resource::<SpawnRegistry>() else {
        return ToolReply::Failed("this build has no spawn presets".into());
    };
    let mut rows: Vec<(&str, &str, &str)> = registry
        .iter()
        .map(|p| (p.category, p.id, p.display_name))
        .collect();
    rows.sort();
    let mut out = String::from("Presets (category: id, name), for spawn_preset:\n");
    for (category, id, name) in rows {
        out.push_str(&format!("  {category}: {id} ({name})\n"));
    }
    ToolReply::Text(out)
}

/// Spawn one of the editor's Add Entity presets.
///
/// This is the lever that reaches everything the other tools do not name: a
/// camera, terrain, a world environment, a reflection probe, a UI canvas, and
/// any preset a plugin registers after this was written. Each preset is the
/// function the menu itself calls, so what comes out is exactly what a person
/// clicking the menu would get.
pub fn spawn_preset(world: &mut World, args: &Value) -> ToolReply {
    let entity = match spawn_preset_from_args(world, args) {
        Ok(entity) => entity,
        Err(why) => return ToolReply::Failed(why),
    };
    let again = args.clone();
    edits::record_spawn(world, entity, "spawn preset", move |world| {
        spawn_preset_from_args(world, &again).ok()
    });
    let name = world
        .get::<Name>(entity)
        .map(|n| n.as_str().to_string())
        .unwrap_or_default();
    ToolReply::Text(format!("spawned {name} [{}]", entity_id(entity)))
}

fn spawn_preset_from_args(world: &mut World, args: &Value) -> Result<Entity, String> {
    let Some(id) = args.get("id").and_then(Value::as_str) else {
        return Err("spawn_preset needs an id; list_presets shows them".into());
    };
    let spawn = world
        .get_resource::<SpawnRegistry>()
        .and_then(|r| r.iter().find(|p| p.id == id).map(|p| p.spawn_fn))
        .ok_or_else(|| format!("no preset called {id}; list_presets shows them"))?;
    let parent = checked_parent(world, args)?;

    let entity = spawn(world);

    if let Some(position) = arg_vec3(args, "position") {
        if let Some(mut transform) = world.get_mut::<Transform>(entity) {
            transform.translation = position;
        }
    }
    if let Some(name) = args.get("name").and_then(Value::as_str) {
        let unique = unique_entity_name(world, name, entity);
        world.entity_mut(entity).insert(Name::new(unique));
    }
    if let Some(parent) = parent {
        world.entity_mut(entity).insert(ChildOf(parent));
    }
    Ok(entity)
}

/// Make a scene camera the one play mode looks through.
pub fn set_default_camera(world: &mut World, args: &Value) -> ToolReply {
    let Some(entity) = arg_entity(args, "entity") else {
        return ToolReply::Failed("set_default_camera needs an entity id".into());
    };
    if world.get::<SceneCamera>(entity).is_none() {
        return ToolReply::Failed(format!(
            "{} is not a scene camera; spawn_preset camera_3d makes one",
            entity_id(entity)
        ));
    }
    let mut others = world.query_filtered::<Entity, With<DefaultCamera>>();
    let before: Vec<Entity> = others.iter(world).collect();
    edits::set_default_camera(world, before, entity);
    ToolReply::Text(format!("{} is now the default camera", entity_id(entity)))
}

// ============================================================================
// Helpers
// ============================================================================

/// The `parent` argument, checked before anything is spawned so a bad one
/// leaves nothing behind.
fn checked_parent(world: &World, args: &Value) -> Result<Option<Entity>, String> {
    match arg_entity(args, "parent") {
        Some(parent) if world.get_entity(parent).is_err() => {
            Err(format!("the parent {} does not exist", entity_id(parent)))
        }
        other => Ok(other),
    }
}

/// A transform from the `position`, `rotation` (Euler degrees) and `scale`
/// arguments.
fn placed(args: &Value) -> Transform {
    let mut transform = Transform::from_translation(arg_vec3(args, "position").unwrap_or(Vec3::ZERO));
    if let Some(euler) = arg_vec3(args, "rotation") {
        transform.rotation = Quat::from_euler(
            EulerRot::XYZ,
            euler.x.to_radians(),
            euler.y.to_radians(),
            euler.z.to_radians(),
        );
    }
    if let Some(scale) = arg_vec3(args, "scale") {
        transform.scale = scale;
    }
    transform
}

/// Name an entity from the `name` argument or a fallback, unique in the scene.
fn name_entity(world: &mut World, entity: Entity, args: &Value, fallback: &str) {
    let wanted = args.get("name").and_then(Value::as_str).unwrap_or(fallback);
    let unique = unique_entity_name(world, wanted, entity);
    world.entity_mut(entity).insert(Name::new(unique));
}
