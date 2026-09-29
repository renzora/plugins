//! Undoable versions of the edits this server makes.
//!
//! Every mutation here goes on the editor's own undo stack, so Ctrl+Z takes back
//! what an agent did exactly as it takes back what a person did. That is not a
//! nicety. An agent editing somebody's open scene and leaving no way to reverse
//! it is the most dangerous property this whole server could have, and for a
//! while it had it: the first version of these tools recorded nothing.
//!
//! It is possible because the undo *core* lives in the contract crate. The trait,
//! the stacks and `execute` need only `bevy`, so they sit in `renzora::undo`
//! where a plugin can reach them; `renzora_undo` keeps the editor's own concrete
//! commands, which need crates a plugin cannot link. A private copy of the trait
//! would be worse than nothing: it would push onto a stack the editor's Ctrl+Z
//! never reads, so the edit would look undoable and silently not be.
//!
//! Adding and removing a component go through [`ComponentEdit`], which holds a
//! reflected copy of the value, so even a component this plugin has never heard
//! of comes back on undo.
//!
//! What is NOT here is `despawn_entity`. Undoing a despawn means restoring every
//! component of every entity in the subtree, and doing that honestly needs the
//! scene serialiser rather than a hand-rolled capture. It is marked destructive
//! in the tool list instead, which is the true statement.

use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::*;
use bevy::reflect::{GetPath, PartialReflect};
use renzora::avian3d::prelude::{Collider, RigidBody};
use renzora::component::ScriptComponent;
use renzora::{DefaultCamera, MaterialRef, MaterialResolved, MeshColor};
use renzora::core::reflection::{get_reflected_field, set_reflected_field};
use renzora::undo::{execute, UndoCommand, UndoContext};
use renzora::PropertyValue;

/// Apply a command and push it onto the scene's undo stack.
fn apply(world: &mut World, cmd: Box<dyn UndoCommand>) {
    execute(world, UndoContext::Scene, cmd);
}

// ============================================================================
// Transform
// ============================================================================

pub struct TransformEdit {
    pub entity: Entity,
    pub before: Transform,
    pub after: Transform,
}

impl UndoCommand for TransformEdit {
    fn label(&self) -> &str {
        "move"
    }
    fn execute(&mut self, world: &mut World) {
        write_transform(world, self.entity, self.after);
    }
    fn undo(&mut self, world: &mut World) {
        write_transform(world, self.entity, self.before);
    }
}

fn write_transform(world: &mut World, entity: Entity, value: Transform) {
    if let Some(mut transform) = world.get_mut::<Transform>(entity) {
        *transform = value;
    }
}

/// Set a transform, undoably. Returns false if the entity has none.
pub fn set_transform(world: &mut World, entity: Entity, after: Transform) -> bool {
    let Some(before) = world.get::<Transform>(entity).copied() else {
        return false;
    };
    apply(
        world,
        Box::new(TransformEdit {
            entity,
            before,
            after,
        }),
    );
    true
}

// ============================================================================
// Name
// ============================================================================

pub struct RenameEdit {
    pub entity: Entity,
    pub before: Option<String>,
    pub after: String,
}

impl UndoCommand for RenameEdit {
    fn label(&self) -> &str {
        "rename"
    }
    fn execute(&mut self, world: &mut World) {
        if let Ok(mut entity_mut) = world.get_entity_mut(self.entity) {
            entity_mut.insert(Name::new(self.after.clone()));
        }
    }
    fn undo(&mut self, world: &mut World) {
        let Ok(mut entity_mut) = world.get_entity_mut(self.entity) else {
            return;
        };
        // An entity that had no name before must end up with no name again, not
        // with an empty one: the scene saver only serialises named entities, so
        // an empty `Name` would quietly change what gets written.
        match &self.before {
            Some(name) => {
                entity_mut.insert(Name::new(name.clone()));
            }
            None => {
                entity_mut.remove::<Name>();
            }
        }
    }
}

pub fn rename(world: &mut World, entity: Entity, after: String) {
    let before = world.get::<Name>(entity).map(|n| n.as_str().to_string());
    apply(
        world,
        Box::new(RenameEdit {
            entity,
            before,
            after,
        }),
    );
}

// ============================================================================
// Hierarchy
// ============================================================================

pub struct ReparentEdit {
    pub entity: Entity,
    pub before: Option<Entity>,
    pub after: Option<Entity>,
}

impl UndoCommand for ReparentEdit {
    fn label(&self) -> &str {
        "reparent"
    }
    fn execute(&mut self, world: &mut World) {
        write_parent(world, self.entity, self.after);
    }
    fn undo(&mut self, world: &mut World) {
        write_parent(world, self.entity, self.before);
    }
}

fn write_parent(world: &mut World, entity: Entity, parent: Option<Entity>) {
    let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
        return;
    };
    match parent {
        Some(parent) => {
            entity_mut.insert(ChildOf(parent));
        }
        None => {
            entity_mut.remove::<ChildOf>();
        }
    }
}

pub fn reparent(world: &mut World, entity: Entity, after: Option<Entity>) {
    let before = world.get::<ChildOf>(entity).map(ChildOf::parent);
    apply(
        world,
        Box::new(ReparentEdit {
            entity,
            before,
            after,
        }),
    );
}

// ============================================================================
// Reflected fields
// ============================================================================

pub struct FieldEdit {
    pub entity: Entity,
    pub component: String,
    pub field: String,
    pub before: Option<PropertyValue>,
    pub after: PropertyValue,
}

impl UndoCommand for FieldEdit {
    fn label(&self) -> &str {
        "change field"
    }
    fn execute(&mut self, world: &mut World) {
        write_field(world, self.entity, &self.component, &self.field, &self.after);
    }
    fn undo(&mut self, world: &mut World) {
        // No previous value means the field could not be read going in. Writing
        // something invented would be worse than leaving it: undo would "restore"
        // a value the field never held.
        if let Some(before) = &self.before {
            write_field(world, self.entity, &self.component, &self.field, before);
        }
    }
}

/// Write one reflected field, through the contract's setter where it can and
/// in place where it cannot.
///
/// The contract's setter edits a `reflect_clone` of the component and applies
/// it back, and `reflect_clone` fails for any component holding a field that
/// cannot be cloned by reflection. `WorldEnvironment` is one, which put fog out
/// of reach: `inspect_entity` showed `fog.density`, because reading never
/// clones, and every write to it was refused. The fallback writes through
/// `reflect_mut` instead, which still goes through `Mut`, so change detection
/// fires exactly as it does for the clone-and-apply path.
fn write_field(
    world: &mut World,
    entity: Entity,
    component: &str,
    field: &str,
    value: &PropertyValue,
) -> bool {
    set_reflected_field(world, entity, component, field, value)
        || write_field_in_place(world, entity, component, field, value)
}

fn write_field_in_place(
    world: &mut World,
    entity: Entity,
    component: &str,
    field: &str,
    value: &PropertyValue,
) -> bool {
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();
    let Some(reflect_component) =
        crate::tools::find_type(&registry, component).and_then(|r| r.data::<ReflectComponent>())
    else {
        return false;
    };
    let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
        return false;
    };
    let Some(mut reflected) = reflect_component.reflect_mut(&mut entity_mut) else {
        return false;
    };
    let Ok(target) = reflected.reflect_path_mut(field) else {
        return false;
    };
    apply_property(target, value)
}

/// Put a `PropertyValue` into a reflected field of whatever concrete type it
/// has, trying each type the value could reasonably mean. `try_apply` onto a
/// plain value only succeeds for the matching type, so a wrong guess changes
/// nothing and the next one is tried.
fn apply_property(target: &mut dyn PartialReflect, value: &PropertyValue) -> bool {
    let candidates: Vec<Box<dyn PartialReflect>> = match value {
        PropertyValue::Float(f) => vec![Box::new(*f), Box::new(*f as f64)],
        PropertyValue::Int(i) => vec![
            Box::new(*i as i32),
            Box::new(*i as u32),
            Box::new(*i),
            Box::new(*i as u64),
            Box::new(*i as usize),
            Box::new(*i as u8),
            Box::new(*i as f32),
            Box::new(*i as f64),
        ],
        PropertyValue::Bool(b) => vec![Box::new(*b)],
        PropertyValue::String(s) => vec![Box::new(s.clone())],
        PropertyValue::Vec3(v) => vec![
            Box::new(*v),
            Box::new(Vec3::from_array(*v)),
            Box::new(Color::linear_rgb(v[0], v[1], v[2])),
        ],
        PropertyValue::Color(c) => vec![
            Box::new(*c),
            Box::new(Vec4::from_array(*c)),
            Box::new(Color::linear_rgba(c[0], c[1], c[2], c[3])),
        ],
    };
    candidates
        .iter()
        .any(|candidate| target.try_apply(candidate.as_ref()).is_ok())
}

/// Write a reflected field, undoably. Returns whether the write landed.
pub fn set_field(
    world: &mut World,
    entity: Entity,
    component: &str,
    field: &str,
    after: PropertyValue,
) -> bool {
    let before = get_reflected_field(world, entity, component, field);
    // Tried first, because a value that does not fit the field must not leave a
    // no-op sitting on the undo stack for the user to press Ctrl+Z through.
    if !write_field(world, entity, component, field, &after) {
        return false;
    }
    renzora::undo::record(
        world,
        UndoContext::Scene,
        Box::new(FieldEdit {
            entity,
            component: component.to_string(),
            field: field.to_string(),
            before,
            after,
        }),
    );
    true
}

// ============================================================================
// Scripts
// ============================================================================

/// A change to an entity's script list, held as the whole component before and
/// after.
///
/// Whole-component rather than "the one entry added": entries carry ids handed
/// out by the component's own counter, so undoing by removing index N would be
/// wrong the moment anything else reordered the list. `None` means the entity had
/// no `ScriptComponent` at all, and undo must remove it rather than leave an
/// empty one, which would still serialise into every saved scene.
pub struct ScriptsEdit {
    pub entity: Entity,
    pub before: Option<ScriptComponent>,
    pub after: Option<ScriptComponent>,
}

impl UndoCommand for ScriptsEdit {
    fn label(&self) -> &str {
        "edit scripts"
    }
    fn execute(&mut self, world: &mut World) {
        write_scripts(world, self.entity, self.after.clone());
    }
    fn undo(&mut self, world: &mut World) {
        write_scripts(world, self.entity, self.before.clone());
    }
}

fn write_scripts(world: &mut World, entity: Entity, value: Option<ScriptComponent>) {
    let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
        return;
    };
    match value {
        Some(scripts) => {
            entity_mut.insert(scripts);
        }
        None => {
            entity_mut.remove::<ScriptComponent>();
        }
    }
}

/// Replace an entity's script list, undoably.
pub fn set_scripts(world: &mut World, entity: Entity, after: Option<ScriptComponent>) {
    let before = world.get::<ScriptComponent>(entity).cloned();
    apply(
        world,
        Box::new(ScriptsEdit {
            entity,
            before,
            after,
        }),
    );
}

// ============================================================================
// Spawning
// ============================================================================

/// A spawn, undone by despawning and redone by spawning again.
///
/// The entity id is refreshed on every redo, which is what `&mut self` on the
/// trait's methods is for: a respawned entity is a new id, and a command holding
/// the old one would undo something that no longer exists.
pub struct SpawnEdit {
    pub entity: Entity,
    pub respawn: Box<dyn Fn(&mut World) -> Option<Entity> + Send + Sync>,
    pub label: String,
}

impl UndoCommand for SpawnEdit {
    fn label(&self) -> &str {
        &self.label
    }
    fn execute(&mut self, world: &mut World) {
        // Only on redo: the first spawn already happened, and `record` is what
        // puts this on the stack without calling execute.
        if world.get_entity(self.entity).is_err() {
            if let Some(entity) = (self.respawn)(world) {
                self.entity = entity;
            }
        }
    }
    fn undo(&mut self, world: &mut World) {
        if let Ok(entity_mut) = world.get_entity_mut(self.entity) {
            entity_mut.despawn();
        }
    }
}

// ============================================================================
// Whole components, by reflection
// ============================================================================

/// A component added, replaced or removed, held as a reflected copy of its
/// value before and after. `None` means "not on the entity".
///
/// Reflected rather than typed because `add_component` and `remove_component`
/// reach components this plugin has never heard of; the type registry is the
/// only thing that can put one back. The type is kept by its full path, since a
/// short name that is unique today can gain a twin when a plugin loads.
pub struct ComponentEdit {
    pub entity: Entity,
    pub type_path: String,
    pub label: String,
    pub before: Option<Box<dyn Reflect>>,
    pub after: Option<Box<dyn Reflect>>,
}

impl UndoCommand for ComponentEdit {
    fn label(&self) -> &str {
        &self.label
    }
    fn execute(&mut self, world: &mut World) {
        write_component(world, self.entity, &self.type_path, self.after.as_deref());
    }
    fn undo(&mut self, world: &mut World) {
        write_component(world, self.entity, &self.type_path, self.before.as_deref());
    }
}

fn write_component(world: &mut World, entity: Entity, type_path: &str, value: Option<&dyn Reflect>) {
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();
    let Some(reflect_component) = registry
        .get_with_type_path(type_path)
        .and_then(|r| r.data::<ReflectComponent>())
    else {
        return;
    };
    let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
        return;
    };
    match value {
        Some(value) => reflect_component.insert(&mut entity_mut, value.as_partial_reflect(), &registry),
        None => reflect_component.remove(&mut entity_mut),
    }
}

/// A reflected copy of one component, or `None` if the entity has none.
///
/// `reflect_clone` first because it yields the concrete type, which inserts back
/// exactly; a type that cannot clone that way falls back to a dynamic copy,
/// which `insert` rebuilds through `FromReflect`.
pub fn snapshot_component(world: &World, entity: Entity, type_path: &str) -> Option<Box<dyn Reflect>> {
    let registry = world.resource::<AppTypeRegistry>().read();
    let reflect_component = registry
        .get_with_type_path(type_path)?
        .data::<ReflectComponent>()?;
    let value = reflect_component.reflect(world.get_entity(entity).ok()?)?;
    value.reflect_clone().ok()
}

/// Record a component change that has already been made.
pub fn record_component(
    world: &mut World,
    entity: Entity,
    type_path: &str,
    label: String,
    before: Option<Box<dyn Reflect>>,
    after: Option<Box<dyn Reflect>>,
) {
    renzora::undo::record(
        world,
        UndoContext::Scene,
        Box::new(ComponentEdit {
            entity,
            type_path: type_path.to_string(),
            label,
            before,
            after,
        }),
    );
}

// ============================================================================
// Materials
// ============================================================================

/// A material assigned to a set of meshes, with what each one had before.
pub struct MaterialEdit {
    pub before: Vec<(Entity, Option<String>)>,
    pub after: Option<String>,
}

impl UndoCommand for MaterialEdit {
    fn label(&self) -> &str {
        "set material"
    }
    fn execute(&mut self, world: &mut World) {
        for (entity, _) in &self.before {
            write_material(world, *entity, self.after.as_deref());
        }
    }
    fn undo(&mut self, world: &mut World) {
        for (entity, before) in &self.before {
            write_material(world, *entity, before.as_deref());
        }
    }
}

/// Point one mesh at a material file, or back at no file.
///
/// `MaterialResolved` comes off every time: it is the resolver's "already done"
/// marker, and while it is there a changed `MaterialRef` is never read.
///
/// Going back to no file is the awkward direction. The resolver swapped the
/// mesh's `StandardMaterial` for a material type this plugin cannot name, and
/// dropping `MaterialRef` does not swap it back, so the mesh would keep looking
/// assigned until the scene reloaded. Every `MeshMaterial3d<T>` other than the
/// standard one is removed by reflection, and a plain one is rebuilt from the
/// `MeshColor` the shape was spawned with.
fn write_material(world: &mut World, entity: Entity, value: Option<&str>) {
    let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
        return;
    };
    entity_mut.remove::<MaterialResolved>();
    if let Some(path) = value {
        entity_mut.insert(MaterialRef(path.to_string()));
        return;
    }
    entity_mut.remove::<MaterialRef>();

    let Some(color) = world.get::<MeshColor>(entity).map(|c| c.0.to_linear()) else {
        return;
    };
    let registry = world.resource::<AppTypeRegistry>().clone();
    {
        let registry = registry.read();
        let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
            return;
        };
        for registration in registry.iter() {
            let path = registration.type_info().type_path();
            if path.contains("MeshMaterial3d<") && !path.contains("StandardMaterial") {
                if let Some(reflect_component) = registration.data::<ReflectComponent>() {
                    reflect_component.remove(&mut entity_mut);
                }
            }
        }
    }
    let handle = crate::tools::material_handle(world, Vec3::new(color.red, color.green, color.blue));
    world.entity_mut(entity).insert(MeshMaterial3d(handle));
}

/// Assign `after` to every entity, undoably.
pub fn set_material(world: &mut World, entities: Vec<Entity>, after: Option<String>) {
    let before = entities
        .into_iter()
        .map(|e| (e, world.get::<MaterialRef>(e).map(|m| m.0.clone())))
        .collect();
    apply(world, Box::new(MaterialEdit { before, after }));
}

// ============================================================================
// Default camera
// ============================================================================

/// Which scene camera play mode looks through. Undo gives the marker back to
/// whichever cameras held it, including none.
pub struct DefaultCameraEdit {
    pub before: Vec<Entity>,
    pub after: Entity,
}

impl UndoCommand for DefaultCameraEdit {
    fn label(&self) -> &str {
        "set default camera"
    }
    fn execute(&mut self, world: &mut World) {
        write_default_cameras(world, &[self.after]);
    }
    fn undo(&mut self, world: &mut World) {
        let before = self.before.clone();
        write_default_cameras(world, &before);
    }
}

/// Leave `DefaultCamera` on exactly `cameras`, and on no other entity.
fn write_default_cameras(world: &mut World, cameras: &[Entity]) {
    let mut holders = world.query_filtered::<Entity, With<DefaultCamera>>();
    let holders: Vec<Entity> = holders.iter(world).collect();
    for entity in holders {
        if !cameras.contains(&entity) {
            world.entity_mut(entity).remove::<DefaultCamera>();
        }
    }
    for &entity in cameras {
        if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
            entity_mut.insert(DefaultCamera);
        }
    }
}

pub fn set_default_camera(world: &mut World, before: Vec<Entity>, after: Entity) {
    apply(world, Box::new(DefaultCameraEdit { before, after }));
}

// ============================================================================
// Colliders
// ============================================================================

/// Colliders given to a set of meshes by `make_solid`.
///
/// Each entry keeps the body the entity had going in. An entity that already
/// had a `RigidBody` keeps it on redo, and undo leaves it exactly as it was,
/// rather than stripping a dynamic prop of the body it was placed with.
pub struct SolidEdit {
    pub entries: Vec<(Entity, Collider, Option<RigidBody>)>,
}

impl UndoCommand for SolidEdit {
    fn label(&self) -> &str {
        "make solid"
    }
    fn execute(&mut self, world: &mut World) {
        for (entity, collider, body) in &self.entries {
            if let Ok(mut entity_mut) = world.get_entity_mut(*entity) {
                entity_mut.insert((collider.clone(), body.unwrap_or(RigidBody::Static)));
            }
        }
    }
    fn undo(&mut self, world: &mut World) {
        for (entity, _, body) in &self.entries {
            if let Ok(mut entity_mut) = world.get_entity_mut(*entity) {
                entity_mut.remove::<Collider>();
                if body.is_none() {
                    entity_mut.remove::<RigidBody>();
                }
            }
        }
    }
}

/// Give each entity its collider, undoably.
pub fn make_solid(world: &mut World, targets: Vec<(Entity, Collider)>) {
    let entries = targets
        .into_iter()
        .map(|(entity, collider)| {
            let body = world.get::<RigidBody>(entity).copied();
            (entity, collider, body)
        })
        .collect();
    apply(world, Box::new(SolidEdit { entries }));
}

/// Record an already-spawned entity so Ctrl+Z removes it again.
pub fn record_spawn(
    world: &mut World,
    entity: Entity,
    label: impl Into<String>,
    respawn: impl Fn(&mut World) -> Option<Entity> + Send + Sync + 'static,
) {
    renzora::undo::record(
        world,
        UndoContext::Scene,
        Box::new(SpawnEdit {
            entity,
            respawn: Box::new(respawn),
            label: label.into(),
        }),
    );
}
