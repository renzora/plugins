//! Turns what Blender sends into entities, and keeps them baked into the
//! project.
//!
//! ## What a linked object is in the scene
//!
//! Every object of one `.blend` sits under a root entity carrying
//! [`BlenderLinkRoot`], flat: Blender's parenting is already folded into each
//! object's transform, which keeps a re-parent in Blender from having to move
//! entities around here. Each object is an entity with a `Name`, a
//! `Transform` and [`BlenderLink`], plus, by kind:
//!
//! - **mesh**: unnamed child entities, one per material ("parts"), drawing the
//!   live geometry, and `MeshInstanceData` pointing at the object's model
//!   file: an `.rvg` (scene and virtual geometry in one file), as an import
//!   stores a model. The parts have no `Name`, so the scene never saves them;
//!   what it saves is the `MeshInstanceData`, and a scene opened without
//!   Blender (or in Play, or in an exported game) loads the model file instead.
//! - **light**: the Bevy light component itself, which saves as it is.
//! - **empty**: nothing more.
//!
//! When Blender connects to a scene that already has its objects (baked on an
//! earlier day), they are found by [`BlenderLink`] and taken over: the model
//! the engine loaded from the file is removed and live parts replace it.
//!
//! Nothing here goes on the undo stack. Blender is the source of truth for
//! these objects, and Ctrl+Z in the editor undoing a Blender edit would only
//! be overwritten by the next one.

use crate::glb;
use crate::protocol::{self, LightData, LightKind, MaterialData, MeshData};
use crate::server::{self, Incoming};
use crate::{LinkBridge, CATEGORY};
use bevy::asset::RenderAssetUsages;
use bevy::image::{
    CompressedImageFormats, Image, ImageAddressMode, ImageSampler, ImageSamplerDescriptor,
    ImageType,
};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use renzora::core::console_log::{console_info, console_warn};
use renzora::serde_json::{json, Value};
use renzora::{CurrentProject, MeshInstanceData};
use std::collections::{HashMap, HashSet};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long an object's geometry has to sit still before it is written to
/// disk. Long enough that dragging a vertex does not rewrite a file per
/// frame, short enough that a save straight after an edit finds it written.
const BAKE_AFTER: Duration = Duration::from_millis(400);

/// Marks a linked object, by its Blender object name. Saved with the scene,
/// which is how a later session finds the entity again.
#[derive(Component, Reflect, Default, Clone, Debug)]
#[reflect(Component, Default)]
pub struct BlenderLink {
    pub object: String,
}

/// Marks the entity a `.blend`'s objects sit under. Saved with the scene.
#[derive(Component, Reflect, Default, Clone, Debug)]
#[reflect(Component, Default)]
pub struct BlenderLinkRoot {
    pub file: String,
}

/// One material's worth of a linked mesh. Unnamed, so never saved.
#[derive(Component)]
pub struct LinkPart;

/// The link's state on the main thread.
#[derive(Resource, Default)]
pub struct Session {
    conn: u64,
    writer: Option<TcpStream>,
    pub file: String,
    root: Option<Entity>,
    objects: HashMap<String, Entity>,
    /// The geometry of each mesh object, kept to bake it.
    meshes: HashMap<String, MeshData>,
    materials: HashMap<String, MaterialData>,
    material_handles: HashMap<String, Handle<StandardMaterial>>,
    /// Texture bytes by the add-on's key, so a material can name them again.
    textures: HashMap<String, protocol::Texture>,
    /// Decoded images by texture key and colour space.
    images: HashMap<(String, bool), Handle<Image>>,
    /// Objects waiting to be baked, and when they last changed.
    pending: HashMap<String, Instant>,
    /// Model files being written in the background, by object.
    baking: HashMap<String, (Entity, bevy::tasks::Task<Result<String, String>>)>,
    /// Objects drawing live geometry: a model the engine loads for them from
    /// the model file is removed when it arrives.
    pub live: HashSet<String>,
    warned_no_project: bool,
}

impl Session {
    pub fn connected(&self) -> bool {
        self.writer.is_some()
    }

    pub fn object_count(&self) -> usize {
        self.objects.len()
    }
}

/// Drain the socket threads' queue and apply every message.
pub fn pump(world: &mut World) {
    let messages: Vec<Incoming> = {
        let Some(bridge) = world.get_resource::<LinkBridge>() else {
            return;
        };
        let Ok(rx) = bridge.rx.lock() else {
            return;
        };
        rx.try_iter().collect()
    };
    for message in messages {
        match message {
            Incoming::Connected { conn, writer } => {
                let mut session = world.resource_mut::<Session>();
                session.conn = conn;
                session.writer = Some(writer);
                session.objects.clear();
                session.live.clear();
                session.root = None;
                // A new connection starts the add-on's record of sent textures
                // afresh, so it sends every one again.
                session.textures.clear();
                session.images.clear();
            }
            Incoming::Message { conn, header, payload } => {
                if conn != world.resource::<Session>().conn {
                    continue;
                }
                if let Err(why) = handle(world, &header, &payload) {
                    console_warn(CATEGORY, format!("Blender link: {why}"));
                }
            }
            Incoming::Closed { conn, why } => {
                if conn != world.resource::<Session>().conn {
                    continue;
                }
                world.resource_mut::<Session>().writer = None;
                console_info(CATEGORY, format!("{why}; linked objects stay in the scene as baked models"));
                // Write everything now rather than waiting out the delay:
                // the geometry will not change again until Blender returns.
                bake_due(world, true);
                world.resource_mut::<Session>().live.clear();
            }
        }
    }
    bake_due(world, false);
    finish_bakes(world);
}

fn handle(world: &mut World, header: &Value, payload: &[u8]) -> Result<(), String> {
    let kind = protocol::text(header, "type").unwrap_or("");
    match kind {
        "hello" => hello(world, header),
        "object" => object(world, header),
        "rename" => rename(world, header),
        "mesh" => mesh(world, header, payload),
        "material" => material(world, header, payload),
        "light" => light(world, header),
        "remove" => {
            if let Some(name) = protocol::text(header, "object") {
                remove(world, name);
            }
            Ok(())
        }
        "synced" => synced(world, header),
        other => Err(format!("unknown message `{other}` (is the add-on newer than this plugin?)")),
    }
}

fn hello(world: &mut World, header: &Value) -> Result<(), String> {
    let file = protocol::text(header, "file").unwrap_or("untitled.blend").to_string();
    let blender = protocol::text(header, "blender").unwrap_or("?").to_string();

    // The root for this file, if an earlier session made one.
    let mut roots = world.query::<(Entity, &BlenderLinkRoot)>();
    let found = roots
        .iter(world)
        .find(|(_, r)| r.file == file)
        .map(|(e, _)| e);
    let root = match found {
        Some(root) => root,
        None => world
            .spawn((
                Name::new(format!("Blender: {file}")),
                BlenderLinkRoot { file: file.clone() },
                Transform::default(),
                Visibility::default(),
            ))
            .id(),
    };

    // Its objects, so this session updates them rather than adding copies.
    let mut objects = HashMap::new();
    let mut linked = world.query::<(Entity, &BlenderLink, &ChildOf)>();
    for (entity, link, parent) in linked.iter(world) {
        if parent.parent() == root {
            objects.insert(link.object.clone(), entity);
        }
    }

    let project = world
        .get_resource::<CurrentProject>()
        .map(|p| p.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
        .unwrap_or_default();

    let mut session = world.resource_mut::<Session>();
    session.file = file.clone();
    session.root = Some(root);
    let found_count = objects.len();
    session.objects = objects;
    if let Some(writer) = session.writer.as_mut() {
        let _ = server::send(
            writer,
            &json!({ "type": "welcome", "engine": "renzora", "project": project }),
        );
    }
    console_info(
        CATEGORY,
        if found_count > 0 {
            format!("Blender {blender} linked {file}; updating its {found_count} object(s) already in the scene")
        } else {
            format!("Blender {blender} linked {file}")
        },
    );
    Ok(())
}

/// The entity for `name`, made under the root if it does not exist yet.
fn ensure(world: &mut World, name: &str) -> Result<Entity, String> {
    let session = world.resource::<Session>();
    if let Some(entity) = session.objects.get(name).copied() {
        if world.get_entity(entity).is_ok() {
            return Ok(entity);
        }
    }
    let Some(root) = session.root else {
        return Err("an object arrived before hello".into());
    };
    let entity = world
        .spawn((
            Name::new(name.to_string()),
            BlenderLink { object: name.to_string() },
            Transform::default(),
            Visibility::default(),
            ChildOf(root),
        ))
        .id();
    world
        .resource_mut::<Session>()
        .objects
        .insert(name.to_string(), entity);
    Ok(entity)
}

fn object(world: &mut World, header: &Value) -> Result<(), String> {
    let name = protocol::text(header, "name").ok_or("an object without a name")?;
    let entity = ensure(world, name)?;
    let t = protocol::vec3(header, "translation", [0.0; 3]);
    let r = protocol::vec4(header, "rotation", [0.0, 0.0, 0.0, 1.0]);
    let s = protocol::vec3(header, "scale", [1.0; 3]);
    let visible = header.get("visible").and_then(Value::as_bool).unwrap_or(true);
    let transform = Transform {
        translation: Vec3::from_array(t),
        rotation: Quat::from_xyzw(r[0], r[1], r[2], r[3]).normalize(),
        scale: Vec3::from_array(s),
    };
    let mut e = world.entity_mut(entity);
    // Only on a real change: a `Transform` write marks it changed, which
    // propagates and re-extracts, and Blender sends every object on a resync.
    if e.get::<Transform>() != Some(&transform) {
        e.insert(transform);
    }
    let visibility = if visible { Visibility::Inherited } else { Visibility::Hidden };
    if e.get::<Visibility>() != Some(&visibility) {
        e.insert(visibility);
    }

    // An object that changed kind in Blender (rare, but a mesh can become an
    // empty by losing its data) drops what the old kind added.
    match protocol::text(header, "kind").unwrap_or("empty") {
        "light" => {
            clear_parts(world, entity);
            world.entity_mut(entity).remove::<MeshInstanceData>();
        }
        "mesh" => {
            world
                .entity_mut(entity)
                .remove::<(PointLight, SpotLight, DirectionalLight)>();
        }
        _ => {
            clear_parts(world, entity);
            world
                .entity_mut(entity)
                .remove::<(MeshInstanceData, PointLight, SpotLight, DirectionalLight)>();
        }
    }
    Ok(())
}

fn rename(world: &mut World, header: &Value) -> Result<(), String> {
    let from = protocol::text(header, "from").ok_or("a rename without `from`")?;
    let to = protocol::text(header, "to").ok_or("a rename without `to`")?;
    let mut session = world.resource_mut::<Session>();
    let Some(entity) = session.objects.remove(from) else {
        return Ok(());
    };
    session.objects.insert(to.to_string(), entity);
    if let Some(mesh) = session.meshes.remove(from) {
        session.meshes.insert(to.to_string(), mesh);
    }
    if session.live.remove(from) {
        session.live.insert(to.to_string());
    }
    let was_pending = session.pending.remove(from).is_some();
    // The baked file is named for the object, so a rename writes a new one.
    if was_pending || session.meshes.contains_key(to) {
        session.pending.insert(to.to_string(), Instant::now());
    }
    if let Ok(mut e) = world.get_entity_mut(entity) {
        e.insert((Name::new(to.to_string()), BlenderLink { object: to.to_string() }));
    }
    Ok(())
}

/// Despawn an object's live parts, and the model the engine loaded for it
/// from its model file, leaving any child the user added themselves.
fn clear_parts(world: &mut World, entity: Entity) {
    let Some(children) = world.get::<Children>(entity) else {
        return;
    };
    let doomed: Vec<Entity> = children
        .to_vec()
        .into_iter()
        .filter(|c| {
            world.get::<LinkPart>(*c).is_some()
                || world.get::<Name>(*c).is_some_and(|n| n.as_str() == "SceneRoot")
        })
        .collect();
    for child in doomed {
        if let Ok(e) = world.get_entity_mut(child) {
            e.despawn();
        }
    }
}

fn mesh(world: &mut World, header: &Value, payload: &[u8]) -> Result<(), String> {
    let name = protocol::text(header, "object").ok_or("a mesh without an object")?.to_string();
    let data = protocol::mesh(header, payload).map_err(|e| format!("{name}: {e}"))?;
    let entity = ensure(world, &name)?;

    clear_parts(world, entity);
    let mut parts = Vec::new();
    for p in &data.primitives {
        if p.indices.is_empty() {
            continue;
        }
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, p.positions.clone());
        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, p.normals.clone());
        if let Some(uvs) = &p.uvs {
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.clone());
        }
        m.insert_indices(Indices::U32(p.indices.clone()));
        let normal_mapped = p
            .material
            .as_ref()
            .and_then(|n| world.resource::<Session>().materials.get(n))
            .is_some_and(|mat| mat.normal_texture.is_some());
        if normal_mapped && p.uvs.is_some() {
            let _ = m.generate_tangents();
        }
        let mesh = world.resource_mut::<Assets<Mesh>>().add(m);
        let material = material_handle(world, p.material.as_deref());
        parts.push((mesh, material));
    }
    for (mesh, material) in parts {
        world.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::default(),
            Visibility::default(),
            LinkPart,
            ChildOf(entity),
        ));
    }

    let mut session = world.resource_mut::<Session>();
    session.live.insert(name.clone());
    session.meshes.insert(name.clone(), data);
    session.pending.insert(name, Instant::now());
    Ok(())
}

/// The shared material for a Blender material name, made on first use. A
/// material message later changes it in place, so every part using it updates.
fn material_handle(world: &mut World, name: Option<&str>) -> Handle<StandardMaterial> {
    let key = name.unwrap_or("").to_string();
    if let Some(handle) = world.resource::<Session>().material_handles.get(&key) {
        return handle.clone();
    }
    let mut standard = StandardMaterial::default();
    if let Some(data) = world.resource::<Session>().materials.get(&key).cloned() {
        fill_material(world, &mut standard, &data);
    }
    let handle = world.resource_mut::<Assets<StandardMaterial>>().add(standard);
    world
        .resource_mut::<Session>()
        .material_handles
        .insert(key, handle.clone());
    handle
}

/// The decoded image for a texture, decoded once per key and colour space: a
/// 4K PNG takes long enough to decode that doing it on every slider move in
/// Blender would stall the editor.
fn decode(world: &mut World, tex: &protocol::Texture, srgb: bool) -> Option<Handle<Image>> {
    let cache_key = (tex.key.clone(), srgb);
    if let Some(handle) = world.resource::<Session>().images.get(&cache_key) {
        return Some(handle.clone());
    }
    let handle = decode_uncached(world, tex, srgb)?;
    world.resource_mut::<Session>().images.insert(cache_key, handle.clone());
    Some(handle)
}

fn decode_uncached(world: &mut World, tex: &protocol::Texture, srgb: bool) -> Option<Handle<Image>> {
    let mut sampler = ImageSamplerDescriptor::linear();
    // glTF's default, and what a tiling texture painted in Blender expects.
    sampler.address_mode_u = ImageAddressMode::Repeat;
    sampler.address_mode_v = ImageAddressMode::Repeat;
    match Image::from_buffer(
        &tex.bytes,
        ImageType::Extension(tex.extension()),
        CompressedImageFormats::NONE,
        srgb,
        ImageSampler::Descriptor(sampler),
        RenderAssetUsages::default(),
    ) {
        Ok(image) => Some(world.resource_mut::<Assets<Image>>().add(image)),
        Err(e) => {
            console_warn(CATEGORY, format!("Blender link: a texture did not decode ({e})"));
            None
        }
    }
}

fn fill_material(world: &mut World, m: &mut StandardMaterial, d: &MaterialData) {
    let [r, g, b, a] = d.base_color;
    m.base_color = Color::linear_rgba(r, g, b, a);
    m.metallic = d.metallic;
    m.perceptual_roughness = d.roughness;
    let [er, eg, eb] = d.emissive;
    // As the engine's glTF loader reads the baked file: factor times strength.
    m.emissive = LinearRgba::rgb(er, eg, eb) * d.emissive_strength;
    m.alpha_mode = if d.blend { AlphaMode::Blend } else { AlphaMode::Opaque };
    m.double_sided = d.double_sided;
    m.cull_mode = if d.double_sided { None } else { Some(bevy::render::render_resource::Face::Back) };
    m.base_color_texture = d.base_color_texture.as_ref().and_then(|t| decode(world, t, true));
    m.normal_map_texture = d.normal_texture.as_ref().and_then(|t| decode(world, t, false));
}

/// The texture a reference names: the fresh bytes (remembered for next time)
/// or bytes this session was sent before under the same key.
fn resolve(world: &mut World, tex: Option<protocol::TextureRef>) -> Option<protocol::Texture> {
    let tex = tex?;
    let mut session = world.resource_mut::<Session>();
    match tex.fresh {
        Some((bytes, mime)) => {
            let texture = protocol::Texture { key: tex.key.clone(), bytes: std::sync::Arc::new(bytes), mime };
            // New bytes under this key: whatever was decoded from the old ones
            // is stale.
            session.images.retain(|(key, _), _| *key != tex.key);
            session.textures.insert(tex.key, texture.clone());
            Some(texture)
        }
        None => {
            let known = session.textures.get(&tex.key).cloned();
            if known.is_none() {
                console_warn(CATEGORY, format!("Blender link: a material named texture `{}` before sending it", tex.key));
            }
            known
        }
    }
}

fn material(world: &mut World, header: &Value, payload: &[u8]) -> Result<(), String> {
    let message = protocol::material(header, payload).ok_or("a material without a name")?;
    let mut data = message.data;
    data.base_color_texture = resolve(world, message.base_color);
    data.normal_texture = resolve(world, message.normal);
    let name = data.name.clone();
    let existing = world.resource::<Session>().material_handles.get(&name).cloned();
    world
        .resource_mut::<Session>()
        .materials
        .insert(name.clone(), data.clone());
    if let Some(handle) = existing {
        let mut fresh = StandardMaterial::default();
        fill_material(world, &mut fresh, &data);
        if let Some(mut m) = world.resource_mut::<Assets<StandardMaterial>>().get_mut(&handle) {
            *m = fresh;
        }
    }
    // Every object drawing with it is baked again, since the model file carries
    // its materials.
    let mut session = world.resource_mut::<Session>();
    let users: Vec<String> = session
        .meshes
        .iter()
        .filter(|(_, mesh)| mesh.primitives.iter().any(|p| p.material.as_deref() == Some(&name)))
        .map(|(object, _)| object.clone())
        .collect();
    let now = Instant::now();
    for object in users {
        session.pending.insert(object, now);
    }
    Ok(())
}

fn light(world: &mut World, header: &Value) -> Result<(), String> {
    let name = protocol::text(header, "object").ok_or("a light without an object")?;
    let entity = ensure(world, name)?;
    let LightData { kind, color, power, radius, range, spot_size, spot_blend, shadows } =
        protocol::light(header);
    let color = Color::linear_rgb(color[0], color[1], color[2]);
    // Blender's watts to lumens the way its own glTF exporter converts them
    // (683 lm/W over the sphere's 4π), and a sun's W/m² to lux at 683.
    let lumens = power * 683.0 / (4.0 * std::f32::consts::PI);
    let range = range.unwrap_or(40.0);
    let mut e = world.entity_mut(entity);
    match kind {
        LightKind::Point => {
            e.remove::<(SpotLight, DirectionalLight)>().insert(PointLight {
                color,
                intensity: lumens,
                range,
                radius,
                shadow_maps_enabled: shadows,
                ..default()
            });
        }
        LightKind::Spot => {
            let outer = (spot_size * 0.5).clamp(0.0, std::f32::consts::FRAC_PI_2);
            e.remove::<(PointLight, DirectionalLight)>().insert(SpotLight {
                color,
                intensity: lumens,
                range,
                radius,
                outer_angle: outer,
                inner_angle: outer * (1.0 - spot_blend.clamp(0.0, 1.0)),
                shadow_maps_enabled: shadows,
                ..default()
            });
        }
        LightKind::Sun => {
            e.remove::<(PointLight, SpotLight)>().insert(DirectionalLight {
                color,
                illuminance: power * 683.0,
                shadow_maps_enabled: shadows,
                ..default()
            });
        }
    }
    Ok(())
}

fn remove(world: &mut World, name: &str) {
    let mut session = world.resource_mut::<Session>();
    let entity = session.objects.remove(name);
    session.meshes.remove(name);
    session.pending.remove(name);
    session.live.remove(name);
    if let Some(entity) = entity {
        if let Ok(e) = world.get_entity_mut(entity) {
            e.despawn();
        }
    }
}

/// After a full sync: anything linked from this file that Blender no longer
/// has was deleted while the two were apart.
fn synced(world: &mut World, header: &Value) -> Result<(), String> {
    let present: HashSet<String> = header
        .get("objects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let gone: Vec<String> = world
        .resource::<Session>()
        .objects
        .keys()
        .filter(|k| !present.contains(*k))
        .cloned()
        .collect();
    for name in &gone {
        remove(world, name);
    }
    let count = world.resource::<Session>().objects.len();
    console_info(
        CATEGORY,
        if gone.is_empty() {
            format!("Blender link synced {count} object(s)")
        } else {
            format!("Blender link synced {count} object(s); removed {} deleted in Blender", gone.len())
        },
    );
    Ok(())
}

/// Keep a file name to what every filesystem accepts.
fn file_safe(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim_matches('.').to_string();
    if trimmed.is_empty() {
        "object".into()
    } else {
        trimmed
    }
}

/// Write each object whose geometry or materials settled (or all of them,
/// when `now` is set) and point its `MeshInstanceData` at the file.
fn bake_due(world: &mut World, now: bool) {
    let due: Vec<String> = {
        let session = world.resource::<Session>();
        if session.pending.is_empty() {
            return;
        }
        let at = Instant::now();
        session
            .pending
            .iter()
            .filter(|(_, changed)| now || at.duration_since(**changed) >= BAKE_AFTER)
            .map(|(name, _)| name.clone())
            .collect()
    };
    if due.is_empty() {
        return;
    }
    let Some(root) = world.get_resource::<CurrentProject>().map(|p| p.path.clone()) else {
        let mut session = world.resource_mut::<Session>();
        if !session.warned_no_project {
            session.warned_no_project = true;
            console_warn(CATEGORY, "Blender link: no project is open, so linked meshes are not being saved");
        }
        return;
    };
    for name in due {
        // One bake per object at a time: a newer change waits in `pending`
        // until the one writing its file has finished, so two never write
        // the same file at once.
        if world.resource::<Session>().baking.contains_key(&name) {
            continue;
        }
        world.resource_mut::<Session>().pending.remove(&name);
        start_bake(world, &root, &name);
    }
}

/// Start writing one object's model file in the background.
///
/// Stored as an `.rvg`, the engine's model file (the scene and its virtual
/// geometry in one file), exactly as an import stores a model. Building one
/// can take seconds for a dense mesh, hence the background. When the builder
/// is not available or refuses the geometry, the object is kept as a `.glb`,
/// which loads as a model just the same, as an import does.
fn start_bake(world: &mut World, project: &std::path::Path, name: &str) {
    let session = world.resource::<Session>();
    let Some(entity) = session.objects.get(name).copied() else {
        return;
    };
    let Some(mesh) = session.meshes.get(name) else {
        return;
    };
    if mesh.primitives.iter().all(|p| p.indices.is_empty()) {
        // Nothing to draw: no file, and no reference to one, or the engine
        // would load an empty model in Play.
        world.entity_mut(entity).remove::<MeshInstanceData>();
        return;
    }
    let glb = glb::write(name, mesh, &session.materials);
    let stem = file_safe(session.file.trim_end_matches(".blend"));
    let base = format!("blender/{stem}/{}", file_safe(name));
    let project = project.to_path_buf();
    let task = bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
        let (bytes, extension) = match renzora::build_model_file(&glb) {
            Some(Ok(rvg)) => (rvg, "rvg"),
            _ => (glb, "glb"),
        };
        write_model(&project, &base, &bytes, extension)
    });
    world
        .resource_mut::<Session>()
        .baking
        .insert(name.to_string(), (entity, task));
}

/// Write `<base>.<extension>` under the project, and remove the same object's
/// file in the other format, so the project never holds an old `.glb` and a
/// new `.rvg` (or the reverse) for one object. Returns the path written,
/// relative to the project.
fn write_model(project: &std::path::Path, base: &str, bytes: &[u8], extension: &str) -> Result<String, String> {
    let rel = format!("{base}.{extension}");
    let path: PathBuf = project.join(&rel);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // The other format goes first. An old `.glb` left in place would have
    // the engine build an `.rvg` of virtual geometry for it, under the very
    // name this object's `.rvg` model has.
    let other = if extension == "rvg" { "glb" } else { "rvg" };
    let _ = std::fs::remove_file(project.join(format!("{base}.{other}")));
    // Through a temporary file, so Play or a scene load reading it mid-write
    // sees the old model or the new one, never half of one.
    let tmp = path.with_extension(format!("{extension}.tmp"));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(rel)
}

/// Point each object whose model file finished writing at it.
fn finish_bakes(world: &mut World) {
    let finished: Vec<(String, Entity, Result<String, String>)> = {
        let mut session = world.resource_mut::<Session>();
        if session.baking.is_empty() {
            return;
        }
        let done: Vec<String> = session
            .baking
            .iter()
            .filter(|(_, (_, task))| task.is_finished())
            .map(|(name, _)| name.clone())
            .collect();
        done.into_iter()
            .filter_map(|name| {
                let (entity, task) = session.baking.remove(&name)?;
                let result = bevy::tasks::block_on(task);
                Some((name, entity, result))
            })
            .collect()
    };
    for (name, entity, result) in finished {
        let rel = match result {
            Ok(rel) => rel,
            Err(why) => {
                console_warn(CATEGORY, format!("Blender link: could not save {name} ({why})"));
                continue;
            }
        };
        let wanted = Some(rel);
        let Ok(mut e) = world.get_entity_mut(entity) else {
            continue;
        };
        if e.get::<MeshInstanceData>().map(|m| &m.model_path) != Some(&wanted) {
            // The parts are already its children, so the engine's model
            // loader, which only fills an instance with none, leaves it alone.
            e.insert(MeshInstanceData { model_path: wanted });
        }
    }
}

/// Remove a model the engine loaded from its file for an object that is
/// drawing live geometry: a scene loaded with linked objects starts loading
/// their files before Blender connects, and the load can land after the live
/// parts have.
pub fn drop_loaded_models(
    mut commands: Commands,
    session: Res<Session>,
    added: Query<(Entity, &Name, &ChildOf), Added<Name>>,
    links: Query<&BlenderLink>,
) {
    if session.live.is_empty() {
        return;
    }
    for (entity, name, parent) in &added {
        if name.as_str() != "SceneRoot" {
            continue;
        }
        if links
            .get(parent.parent())
            .is_ok_and(|l| session.live.contains(&l.object))
        {
            commands.entity(entity).try_despawn();
        }
    }
}
