//! The lightning bolt: a jagged, branching channel from the cloud base to the
//! ground.
//!
//! It plays the way lightning does. A faint **leader** steps down from the
//! cloud over [`LEADER_SECONDS`], the forks splitting off as its tip passes
//! them; when it touches the ground the **return stroke** fires, the whole
//! channel blazes, and only then does the flash light the clouds and the city
//! (see `systems::strike_light`), with its double flicker.
//!
//! It strikes somewhere in front of the camera, a few kilometres out, where a
//! bolt is actually seen: one behind the viewer would only ever be a flash.
//! The channel is a flat ribbon turned toward the camera when it strikes; the
//! strike lasts under a second, so the camera cannot move far enough to see
//! it edge on.
//!
//! Unlit and far brighter than anything around it, so bloom gives it its glow,
//! and it ignores fog: a real bolt is bright enough to cut through haze that
//! hides the buildings under it.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::look::WeatherLook;
use crate::systems::LEADER_SECONDS;

/// Peak brightness of the channel, in linear units. Above 1, so bloom gives
/// it a little glow, but a thin crack rather than a glowing tube.
const PEAK: f32 = 28.0;

/// The descending leader's brightness, as a fraction of the stroke's: barely
/// there, a hint of the path before it fires.
const LEADER_LEVEL: f32 = 0.12;

/// How long the channel itself shows after the return stroke. Far shorter
/// than the flash it lights the world with: a real bolt is gone in a blink,
/// while the sky it lit seems to linger.
const CHANNEL_SECONDS: f32 = 0.25;

/// The channel's brightness `age` seconds after the return stroke: two sharp
/// strokes down the same path, the second a little weaker.
fn channel_envelope(age: f32) -> f32 {
    let first = (-age * 35.0).exp();
    let second = if age > 0.06 { 0.75 * (-(age - 0.06) * 30.0).exp() } else { 0.0 };
    (first + second).min(1.0)
}

/// How far out a bolt strikes, in metres.
const NEAR: f32 = 1500.0;
const FAR: f32 = 5500.0;

/// The widest a bolt lands from straight ahead, in degrees, either side.
const SPREAD_DEGREES: f32 = 45.0;

/// Segments in the main channel. Enough to look jagged at any distance.
const SEGMENTS: usize = 26;

/// One straight piece of the channel.
struct Segment {
    a: Vec3,
    b: Vec3,
    width: f32,
    /// How far down the strike its top is, 0 at the cloud and 1 at the
    /// ground: the leader reaches it at that fraction of its descent.
    starts: f32,
    /// The same for its bottom.
    ends: f32,
}

/// A bolt being drawn.
#[derive(Component)]
pub struct Bolt {
    age: f32,
    brightness: f32,
    eye: Vec3,
    segments: Vec<Segment>,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    /// Whether the mesh already holds the whole channel.
    complete: bool,
}

/// Small deterministic random stream for one bolt's shape.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
    fn signed(&mut self) -> f32 {
        self.next() * 2.0 - 1.0
    }
}

fn lights_the_scene(layers: Option<&RenderLayers>) -> bool {
    layers.is_none_or(|l| l.intersects(&RenderLayers::default()))
}

/// A channel from `start`, `steps` long, each step falling `drop` metres and
/// wandering sideways up to `wander` times that.
fn channel(rng: &mut Rng, start: Vec3, steps: usize, drop: f32, wander: f32, outward: Vec2) -> Vec<Vec3> {
    let mut points = vec![start];
    let mut p = start;
    for _ in 0..steps {
        let side = Vec2::new(rng.signed(), rng.signed()) * drop * wander + outward * drop;
        p += Vec3::new(side.x, -drop * (0.6 + 0.8 * rng.next()), side.y);
        points.push(p);
    }
    points
}

/// The mesh of every segment the leader has reached by `reveal` (0 to 1 down
/// the strike), the one it is in the middle of cut at its tip.
fn build_mesh(segments: &[Segment], reveal: f32, eye: Vec3) -> Mesh {
    let (mut positions, mut normals, mut indices) = (Vec::new(), Vec::new(), Vec::<u32>::new());
    for s in segments.iter().filter(|s| s.starts <= reveal) {
        let span = (s.ends - s.starts).max(1e-4);
        let b = s.a.lerp(s.b, ((reveal - s.starts) / span).clamp(0.0, 1.0));
        let to_eye = (eye - s.a).normalize_or_zero();
        let side = (b - s.a).cross(to_eye).normalize_or_zero() * s.width * 0.5;
        let base = positions.len() as u32;
        for p in [s.a - side, s.a + side, b - side, b + side] {
            positions.push(p.to_array());
            normals.push(to_eye.to_array());
        }
        indices.extend_from_slice(&[base, base + 2, base + 1, base + 1, base + 2, base + 3]);
    }
    let uvs = vec![[0.0f32, 0.0]; positions.len()];
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Strike a bolt in front of the active camera, out of the cloud base `look`
/// has, at `brightness` times the usual. `seed` in 0..1 picks its shape.
pub fn strike(world: &mut World, look: &WeatherLook, brightness: f32, seed: f32) {
    let mut cameras = world.query_filtered::<(&GlobalTransform, &Camera, Option<&RenderLayers>), With<Camera3d>>();
    let Some(camera) = cameras
        .iter(world)
        .find(|(_, c, l)| c.is_active && lights_the_scene(*l))
        .map(|(t, _, _)| *t)
    else {
        return;
    };
    let eye = camera.translation();
    let forward = camera.forward().as_vec3();
    let flat = Vec2::new(forward.x, forward.z).normalize_or(Vec2::Y);

    let mut rng = Rng((seed * u32::MAX as f32) as u32 | 1);
    let angle = rng.signed() * SPREAD_DEGREES.to_radians();
    let heading = Vec2::from_angle(angle).rotate(flat);
    let distance = NEAR + (FAR - NEAR) * rng.next();
    let ground_xz = Vec2::new(eye.x, eye.z) + heading * distance;
    let ground_y = eye.y.min(0.0) - 5.0;
    let top_y = look.clouds.bottom_height.max(eye.y + 200.0);
    let start = Vec3::new(ground_xz.x, top_y, ground_xz.y);
    let height = (top_y - ground_y).max(1.0);
    let depth = |p: Vec3| ((top_y - p.y) / height).clamp(0.0, 1.0);

    let drop = height / SEGMENTS as f32;
    let main = channel(&mut rng, start, SEGMENTS, drop, 0.45, Vec2::ZERO);
    // A couple of pixels across wherever it lands: a crack, not a rope.
    let width = distance * 0.0015;

    let mut segments = Vec::new();
    let mut add = |points: &[Vec3], width: f32| {
        for pair in points.windows(2) {
            segments.push(Segment {
                a: pair[0],
                b: pair[1],
                width,
                starts: depth(pair[0]),
                ends: depth(pair[1]),
            });
        }
    };
    add(&main, width);
    // Forks off the upper two thirds of the channel, thinner and shorter,
    // splaying away from it.
    let forks = 2 + (rng.next() * 3.0) as usize;
    for _ in 0..forks {
        let from = 2 + (rng.next() * (SEGMENTS as f32 * 0.6)) as usize;
        let outward = Vec2::new(rng.signed(), rng.signed()).normalize_or(Vec2::X) * 0.35;
        let steps = 4 + (rng.next() * 7.0) as usize;
        let fork = channel(&mut rng, main[from], steps, drop * 0.8, 0.5, outward);
        add(&fork, width * 0.45);
    }

    let mesh = world.resource_mut::<Assets<Mesh>>().add(build_mesh(&segments, 0.0, eye));
    let material = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
        base_color: bolt_colour(LEADER_LEVEL * brightness),
        unlit: true,
        fog_enabled: false,
        // Seen from either side: the ribbon is turned to the camera, but which
        // way its triangles wind depends on the channel's direction, and a
        // back-face cull hid every bolt that wound away.
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    world.spawn((
        Bolt {
            age: 0.0,
            brightness,
            eye,
            segments,
            mesh: mesh.clone(),
            material: material.clone(),
            complete: false,
        },
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        Visibility::default(),
        NotShadowCaster,
        renzora::HideInHierarchy,
    ));
}

fn bolt_colour(level: f32) -> Color {
    let k = PEAK * level.max(0.0);
    Color::linear_rgb(0.8 * k, 0.86 * k, k)
}

/// Grow each bolt down from the cloud, flicker it with its return stroke, and
/// take it away when the strike ends.
pub fn fade_bolts(
    mut commands: Commands,
    time: Res<Time>,
    mut bolts: Query<(Entity, &mut Bolt)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, mut bolt) in &mut bolts {
        bolt.age += time.delta_secs();
        if bolt.age > LEADER_SECONDS + CHANNEL_SECONDS {
            commands.entity(entity).despawn();
            continue;
        }
        let descending = bolt.age < LEADER_SECONDS;
        if !bolt.complete {
            let reveal = if descending { bolt.age / LEADER_SECONDS } else { 1.0 };
            let rebuilt = build_mesh(&bolt.segments, reveal, bolt.eye);
            if let Some(mut mesh) = meshes.get_mut(&bolt.mesh) {
                *mesh = rebuilt;
            }
            bolt.complete = !descending;
        }
        let level = if descending {
            LEADER_LEVEL
        } else {
            channel_envelope(bolt.age - LEADER_SECONDS)
        };
        if let Some(mut m) = materials.get_mut(&bolt.material) {
            m.base_color = bolt_colour(level * bolt.brightness);
        }
    }
}
