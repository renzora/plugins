//! Spawns a ring of meshes around an entity.
//!
//! Add **Forge** to anything and it lays `count` tori in a circle of `radius`
//! around it, once. `Forged` is the marker that says it has already run, so the
//! system's `Without<Forged>` filter is what keeps it from spawning a new ring
//! every frame.
//!
//! The C-ABI version had to keep its mesh and material handles in two
//! `AtomicU64`s, because the ABI passes an asset across as an opaque integer and
//! a plugin has nowhere else to put one. A native plugin holds the real
//! `Handle<T>`s in an ordinary resource.

use bevy::prelude::*;
use renzora::{AppEditorExt, Inspectable};

/// `x` is the major radius and `y` the minor. Bevy's `Torus::new` takes
/// (inner, outer), so the two are converted rather than passed straight through.
const MAJOR: f32 = 0.45;
const MINOR: f32 = 0.18;

/// How many meshes to lay out, and where.
#[derive(Component, Clone, Debug, Reflect, Inspectable)]
#[reflect(Component)]
#[inspectable(name = "Forge", icon = "hammer", category = "tools")]
pub struct Forge {
    #[field(min = 1.0, max = 64.0)]
    pub count: i32,
    #[field(speed = 0.05, min = 0.0, max = 100.0)]
    pub radius: f32,
    #[field(speed = 0.05, min = -100.0, max = 100.0)]
    pub height: f32,
}

impl Default for Forge {
    fn default() -> Self {
        Self {
            count: 8,
            radius: 3.0,
            height: 1.0,
        }
    }
}

/// Marks a `Forge` that has already spawned its ring.
#[derive(Component, Default, Clone, Debug, Reflect)]
#[reflect(Component)]
pub struct Forged;

/// The shared torus and its material, built once at startup.
#[derive(Resource)]
struct ForgeAssets {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(ForgeAssets {
        // `Torus::new` is (inner, outer), and the authored pair is
        // (major, minor): inner = major - minor, outer = major + minor.
        mesh: meshes.add(Torus::new(MAJOR - MINOR, MAJOR + MINOR)),
        material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.9, 0.6, 0.2),
            ..default()
        }),
    });
}

fn forge(
    mut commands: Commands,
    assets: Res<ForgeAssets>,
    q: Query<(Entity, &Forge), Without<Forged>>,
) {
    for (entity, f) in &q {
        commands.entity(entity).insert(Forged);

        let n = f.count.max(1);
        for i in 0..n {
            let a = (i as f32 / n as f32) * core::f32::consts::TAU;
            commands.spawn((
                // Named, because an unnamed entity is not written to the scene
                // and would vanish on the next save.
                Name::new(format!("Forged {i}")),
                Mesh3d(assets.mesh.clone()),
                MeshMaterial3d(assets.material.clone()),
                Transform::from_xyz(a.cos() * f.radius, f.height, a.sin() * f.radius),
                ChildOf(entity),
            ));
        }
    }
}

#[derive(Default)]
pub struct ForgePlugin;

impl Plugin for ForgePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Forge>()
            .register_type::<Forged>()
            .add_systems(Startup, setup)
            // Gated on the assets resource, which `setup` inserts: the system
            // would otherwise panic on its `Res<ForgeAssets>` the first frame.
            .add_systems(Update, forge.run_if(resource_exists::<ForgeAssets>));
        app.register_inspectable::<Forge>();
    }
}

renzora::plugin!(ForgePlugin, Runtime);
