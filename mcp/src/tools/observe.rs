//! Tools that look and change nothing: the editor's state, the console, the
//! scene tree, one entity's components, and a screenshot.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::ecs::reflect::ReflectComponent;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use renzora::core::console_log::{log_history, LogLevel};
use renzora::serde_json::Value;
use renzora::{CurrentProject, EditorSelection, HideInHierarchy, PlayModeState};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{arg_entity, entity_id, find_type};
use crate::{PendingShot, PendingShots, ToolReply};

/// How long a screenshot may stay pending before its caller is told it failed.
/// Shorter than the RPC timeout by design, so the answer the client gets is the
/// specific one rather than a generic timeout.
const SCREENSHOT_DEADLINE: Duration = Duration::from_secs(40);

pub(super) fn editor_state(world: &mut World) -> ToolReply {
    let mut out = String::new();

    match world.get_resource::<CurrentProject>() {
        Some(project) => {
            out.push_str(&format!(
                "project: {} at {}\nmain scene: {}\n",
                project.config.name,
                project.path.display(),
                project.config.main_scene
            ));
        }
        None => out.push_str("project: none open\n"),
    }

    let mode = match world.get_resource::<PlayModeState>() {
        Some(play) if play.is_playing() => "playing",
        Some(play) if play.is_paused() => "paused",
        Some(play) if play.is_simulating() => "simulating",
        Some(_) => "editing",
        None => "unknown",
    };
    out.push_str(&format!("mode: {mode}\n"));

    if let Some(selection) = world.get_resource::<EditorSelection>() {
        let selected = selection.get_all();
        if selected.is_empty() {
            out.push_str("selection: nothing\n");
        } else {
            let ids: Vec<String> = selected.iter().map(|e| entity_id(*e)).collect();
            out.push_str(&format!("selection: {}\n", ids.join(", ")));
        }
    }

    // The whole-ECS count, which includes editor chrome and every UI node. It is
    // a liveness signal rather than a scene statistic; `scene_tree` is the tool
    // that answers "what is in the scene".
    out.push_str(&format!(
        "entities (including editor UI): {}\n",
        world.entities().len()
    ));

    if let Some(diagnostics) = world.get_resource::<DiagnosticsStore>() {
        if let Some(frame_time) = diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
            .and_then(|d| d.average())
        {
            // Frame time inverted once, rather than Bevy's averaged FPS
            // diagnostic, so the two numbers printed here agree with each other.
            let fps = if frame_time > 0.0 {
                1000.0 / frame_time
            } else {
                0.0
            };
            out.push_str(&format!("frame: {frame_time:.2} ms ({fps:.0} fps)\n"));
        }
    }

    ToolReply::Text(out)
}

pub(super) fn read_console(args: &Value) -> ToolReply {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .clamp(1, 500) as usize;
    let level = args.get("level").and_then(Value::as_str);
    let contains = args
        .get("contains")
        .and_then(Value::as_str)
        .map(str::to_lowercase);

    // `log_history`, not the global log buffer: the buffer is a queue the Console
    // drains every frame, so a reader that is not the Console finds it empty. The
    // history is the mirror the Console keeps as it ingests, and exists because
    // this tool once reported an empty console for a whole session.
    let Ok(entries) = log_history().0.lock() else {
        return ToolReply::Failed("the console history is locked".into());
    };

    let matched: Vec<String> = entries
        .iter()
        .filter(|entry| match level {
            Some(want) => level_name(entry.level).eq_ignore_ascii_case(want),
            None => true,
        })
        .filter(|entry| match &contains {
            Some(needle) => {
                entry.message.to_lowercase().contains(needle)
                    || entry.category.to_lowercase().contains(needle)
            }
            None => true,
        })
        .map(|entry| {
            let repeat = if entry.count > 1 {
                format!(" (x{})", entry.count)
            } else {
                String::new()
            };
            format!(
                "[{}] {}: {}{}",
                level_name(entry.level),
                entry.category,
                entry.message,
                repeat
            )
        })
        .collect();

    if matched.is_empty() {
        return ToolReply::Text("the console has nothing matching that".into());
    }

    // The tail, because the newest lines are the ones worth reading and the
    // buffer is capped anyway.
    let tail = matched.len().saturating_sub(limit);
    ToolReply::Text(matched[tail..].join("\n"))
}

pub(super) fn scene_tree(world: &mut World, args: &Value) -> ToolReply {
    let include_hidden = args
        .get("include_hidden")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let max_depth = args
        .get("max_depth")
        .and_then(Value::as_u64)
        .unwrap_or(8)
        .clamp(1, 32) as usize;

    let named: Vec<(Entity, String)> = world
        .query::<(Entity, &Name)>()
        .iter(world)
        .map(|(entity, name)| (entity, name.as_str().to_string()))
        .collect();

    let names: HashMap<Entity, String> = named.iter().cloned().collect();

    // The hierarchy panel's own rule: an entity is hidden when it or any ancestor
    // is marked, so a gizmo's children do not leak into the tree one level down.
    //
    // A bevy_ui node is excluded on top of that, because not all editor chrome
    // remembers to mark itself: `hover-tooltip` and `world_ui_pointer_2` carry a
    // name, no `HideInHierarchy`, and turned up in an otherwise empty scene as
    // though they were content. A `Node` is layout in screen space and can never
    // be a scene entity, so this needs no marker to be right. The one exception
    // is a game UI canvas, which is a `Node` and IS scene content, so it is let
    // back in by the component the game UI gives it.
    let visible: Vec<(Entity, String)> = named
        .into_iter()
        .filter(|(entity, _)| {
            include_hidden
                || !(hidden_here_or_above(world, *entity)
                    || (world.get::<Node>(*entity).is_some() && !is_ui_canvas(world, *entity)))
        })
        .collect();

    // A named entity's tree parent is its nearest NAMED ancestor, not its literal
    // `ChildOf`: an imported model puts unnamed bone and mesh entities in between,
    // and threading those through would bury the structure the user actually sees.
    let mut children: HashMap<Entity, Vec<Entity>> = HashMap::default();
    let mut roots: Vec<Entity> = Vec::new();
    for (entity, _) in &visible {
        match nearest_named_ancestor(world, *entity, &names) {
            Some(parent) if names.contains_key(&parent) => {
                children.entry(parent).or_default().push(*entity)
            }
            _ => roots.push(*entity),
        }
    }

    let visible_set: HashSet<Entity> = visible.iter().map(|(e, _)| *e).collect();
    roots.retain(|e| visible_set.contains(e));
    roots.sort_by_key(|e| names.get(e).cloned().unwrap_or_default());

    let mut out = String::new();
    for root in roots {
        write_branch(&mut out, root, 0, max_depth, &names, &children, &visible_set);
    }

    if out.is_empty() {
        return ToolReply::Text(
            "no named entities in the scene (only a named entity is saved with one)".into(),
        );
    }
    ToolReply::Text(out)
}

/// Whether an entity is a game UI canvas, checked by reflection because the
/// component lives in the game UI.
fn is_ui_canvas(world: &World, entity: Entity) -> bool {
    let registry = world.resource::<AppTypeRegistry>().read();
    let Some(reflect_component) = find_type(&registry, "UiCanvas").and_then(|r| r.data::<ReflectComponent>()) else {
        return false;
    };
    world
        .get_entity(entity)
        .is_ok_and(|e| reflect_component.contains(e))
}

fn write_branch(
    out: &mut String,
    entity: Entity,
    depth: usize,
    max_depth: usize,
    names: &HashMap<Entity, String>,
    children: &HashMap<Entity, Vec<Entity>>,
    visible: &HashSet<Entity>,
) {
    if depth >= max_depth {
        return;
    }
    let name = names
        .get(&entity)
        .map(String::as_str)
        .unwrap_or("<unnamed>");
    out.push_str(&format!(
        "{}{} [{}]\n",
        "  ".repeat(depth),
        name,
        entity_id(entity)
    ));

    if let Some(kids) = children.get(&entity) {
        let mut kids: Vec<Entity> = kids.iter().copied().filter(|k| visible.contains(k)).collect();
        kids.sort_by_key(|e| names.get(e).cloned().unwrap_or_default());
        for kid in kids {
            write_branch(out, kid, depth + 1, max_depth, names, children, visible);
        }
    }
}

fn hidden_here_or_above(world: &World, entity: Entity) -> bool {
    let mut current = Some(entity);
    while let Some(at) = current {
        if world.get::<HideInHierarchy>(at).is_some() {
            return true;
        }
        current = world.get::<ChildOf>(at).map(ChildOf::parent);
    }
    false
}

fn nearest_named_ancestor(
    world: &World,
    entity: Entity,
    names: &HashMap<Entity, String>,
) -> Option<Entity> {
    let mut current = world.get::<ChildOf>(entity).map(ChildOf::parent);
    while let Some(at) = current {
        if names.contains_key(&at) {
            return Some(at);
        }
        current = world.get::<ChildOf>(at).map(ChildOf::parent);
    }
    None
}

pub(super) fn inspect_entity(world: &mut World, args: &Value) -> ToolReply {
    let Some(entity) = arg_entity(args, "entity") else {
        return ToolReply::Failed("inspect_entity needs an entity id".into());
    };
    if world.get_entity(entity).is_err() {
        return ToolReply::Failed(format!("{} is not a live entity", entity_id(entity)));
    }

    // (shown as, looked up by). Looked up by full type path wherever this plugin
    // chose the name, because a short one is not unique: avian and another crate
    // both define a `Rotation`, and the registry answers an ambiguous short name
    // with nothing.
    let (components, unreflected): (Vec<(String, String)>, Vec<String>) =
        match args.get("component").and_then(Value::as_str) {
            Some(one) => {
                let registry = world.resource::<AppTypeRegistry>().read();
                let full = find_type(&registry, one)
                    .map(|r| r.type_info().type_path().to_string())
                    .unwrap_or_else(|| one.to_string());
                (vec![(one.to_string(), full)], Vec::new())
            }
            None => component_names(world, entity),
        };

    let name = world
        .get::<Name>(entity)
        .map(|n| n.as_str().to_string())
        .unwrap_or_else(|| "<unnamed>".into());
    let mut out = format!("{} [{}]\n", name, entity_id(entity));

    for (shown, full) in components {
        let Some(mut rows) = describe_component(world, entity, &full) else {
            // Three different reasons land here, and each needs its own words.
            // Reporting all of them as "not reflected" told an agent a component
            // could not be read when the entity simply did not have it.
            let why = {
                let registry = world.resource::<AppTypeRegistry>().read();
                match find_type(&registry, &full).and_then(|r| r.data::<ReflectComponent>()) {
                    None => "no registered component by that name",
                    Some(rc) if world.get_entity(entity).is_ok_and(|e| !rc.contains(e)) => {
                        "not on this entity"
                    }
                    Some(_) => "not reflected",
                }
            };
            out.push_str(&format!("  {shown}: {why}\n"));
            continue;
        };
        out.push_str(&format!("  {shown}\n"));
        rows.sort();
        for (path, value) in rows {
            out.push_str(&format!("    {path} = {value}\n"));
        }
    }
    if !unreflected.is_empty() {
        out.push_str(&format!(
            "  also {} components with no reflection, so not readable here: {}\n",
            unreflected.len(),
            unreflected.join(", ")
        ));
    }

    ToolReply::Text(out)
}

/// Every component on an entity, as the names `inspect_entity` accepts back, and
/// separately the ones the type registry does not know.
///
/// From the registry's own short paths rather than the contract helper, which
/// shortens a type name by cutting at its last `::`. That is right for
/// `bevy_transform::Transform` and wrong for every generic: the cut lands inside
/// the type parameter, so `MeshMaterial3d<StandardMaterial>` came out as
/// `StandardMaterial>`, a name nothing could look up.
fn component_names(world: &World, entity: Entity) -> (Vec<(String, String)>, Vec<String>) {
    let Ok(entity_ref) = world.get_entity(entity) else {
        return (Vec::new(), Vec::new());
    };
    let registry = world.resource::<AppTypeRegistry>().read();
    let mut reflected = Vec::new();
    let mut unreflected = Vec::new();
    for id in entity_ref.archetype().components() {
        let Some(info) = world.components().get_info(*id) else {
            continue;
        };
        let registration = info
            .type_id()
            .and_then(|type_id| registry.get(type_id))
            .filter(|r| r.data::<ReflectComponent>().is_some());
        match registration {
            Some(r) => reflected.push((
                r.type_info().type_path_table().short_path().to_string(),
                r.type_info().type_path().to_string(),
            )),
            None => unreflected.push(format!("{}", info.name())),
        }
    }
    reflected.sort();
    unreflected.sort();
    (reflected, unreflected)
}

/// Every value in a component, whatever shape its reflection takes, or `None`
/// when the component cannot be read off this entity at all.
///
/// The one reader for every component. The contract crate has its own, but it
/// resolves a type by the text after the last `::`, which lands inside a generic
/// and picks an arbitrary one of two same-named types; and it reads named struct
/// fields only, so `Mass(f32)` and `RigidBody` came back empty.
fn describe_component(world: &World, entity: Entity, component: &str) -> Option<Vec<(String, String)>> {
    let registry = world.resource::<AppTypeRegistry>().read();
    let value = find_type(&registry, component)?
        .data::<ReflectComponent>()?
        .reflect(world.get_entity(entity).ok()?)?;
    let mut rows = Vec::new();
    describe_value(value.as_partial_reflect(), "", 0, &mut rows);
    Some(rows)
}

/// A vector or quaternion as one `[x, y, z]` row rather than one row a lane.
///
/// Recognised by shape: a struct of two to four `f32` fields named from `x`,
/// `y`, `z`, `w`. That covers `Vec2`/`Vec3`/`Vec4`/`Quat` and anything laid out
/// like them, without naming glam's types.
fn compact_vector(s: &dyn bevy::reflect::structs::Struct) -> Option<String> {
    if !(2..=4).contains(&s.field_len()) {
        return None;
    }
    let mut lanes = Vec::with_capacity(4);
    for i in 0..s.field_len() {
        let name = s.name_at(i)?;
        if !matches!(name, "x" | "y" | "z" | "w") {
            return None;
        }
        lanes.push(s.field_at(i)?.try_downcast_ref::<f32>()?.to_string());
    }
    Some(format!("[{}]", lanes.join(", ")))
}

/// Walk one reflected value into `path = value` rows.
///
/// Paths use the same dotted form `set_component_field` takes, with a tuple
/// field's index as its name (`0` for the inside of a newtype). Depth and list
/// length are capped: a mesh's vertex buffer or a deep asset graph would
/// otherwise turn one inspect into megabytes of text.
fn describe_value(
    value: &dyn bevy::reflect::PartialReflect,
    path: &str,
    depth: usize,
    out: &mut Vec<(String, String)>,
) {
    use bevy::reflect::ReflectRef;

    const MAX_DEPTH: usize = 6;
    const MAX_ITEMS: usize = 8;
    let join = |name: &str| {
        if path.is_empty() {
            name.to_string()
        } else {
            format!("{path}.{name}")
        }
    };
    let here = if path.is_empty() { "value" } else { path };
    if depth > MAX_DEPTH {
        out.push((here.to_string(), "...".into()));
        return;
    }

    match value.reflect_ref() {
        ReflectRef::Struct(s) => {
            if let Some(vector) = compact_vector(s) {
                out.push((here.to_string(), vector));
                return;
            }
            for i in 0..s.field_len() {
                if let (Some(name), Some(field)) = (s.name_at(i), s.field_at(i)) {
                    describe_value(field, &join(name), depth + 1, out);
                }
            }
        }
        ReflectRef::TupleStruct(t) => {
            for i in 0..t.field_len() {
                if let Some(field) = t.field(i) {
                    describe_value(field, &join(&i.to_string()), depth + 1, out);
                }
            }
        }
        ReflectRef::Tuple(t) => {
            for i in 0..t.field_len() {
                if let Some(field) = t.field(i) {
                    describe_value(field, &join(&i.to_string()), depth + 1, out);
                }
            }
        }
        ReflectRef::Enum(e) => {
            out.push((here.to_string(), e.variant_name().to_string()));
            for i in 0..e.field_len() {
                let Some(field) = e.field_at(i) else { continue };
                let name = e.name_at(i).map_or_else(|| i.to_string(), str::to_string);
                describe_value(field, &join(&name), depth + 1, out);
            }
        }
        ReflectRef::List(list) => {
            out.push((here.to_string(), format!("{} items", list.len())));
            for (i, item) in list.iter().take(MAX_ITEMS).enumerate() {
                describe_value(item, &join(&i.to_string()), depth + 1, out);
            }
        }
        ReflectRef::Array(array) => {
            out.push((here.to_string(), format!("{} items", array.len())));
            for (i, item) in array.iter().take(MAX_ITEMS).enumerate() {
                describe_value(item, &join(&i.to_string()), depth + 1, out);
            }
        }
        _ => {
            let mut text = format!("{value:?}");
            if text.len() > 200 {
                let cut = text.floor_char_boundary(200);
                text.truncate(cut);
                text.push_str("...");
            }
            out.push((here.to_string(), text));
        }
    }
}

fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Info => "info",
        LogLevel::Success => "success",
        LogLevel::Warning => "warning",
        LogLevel::Error => "error",
    }
}

/// Ask for a capture and park the reply channel until the PNG lands.
///
/// The whole window rather than the viewport alone: a plugin cannot reach the
/// viewport's render target (it is not in the contract crate), and the window is
/// arguably the more useful picture anyway, since it shows the panels and the
/// state of the editor around the scene.
pub(super) fn start_screenshot(world: &mut World, reply: std::sync::mpsc::Sender<ToolReply>) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let path: PathBuf = std::env::temp_dir().join(format!("renzora-mcp-{stamp}.png"));

    world
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path.clone()));

    world.resource_mut::<PendingShots>().0.push(PendingShot {
        path,
        reply,
        deadline: Instant::now() + SCREENSHOT_DEADLINE,
    });
}

/// Read a PNG only once it is whole.
///
/// The file appears the moment the encoder opens it, so existence alone is not
/// enough: a read that wins the race returns a truncated image the client cannot
/// decode. Every PNG ends with an `IEND` chunk, which makes "finished" something
/// this can check rather than guess.
pub fn read_complete_png(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 12 || &bytes[bytes.len() - 8..bytes.len() - 4] != b"IEND" {
        return None;
    }
    Some(bytes)
}
