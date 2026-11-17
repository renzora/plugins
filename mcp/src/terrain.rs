//! Terrain tools: ground heights, placing things on the ground, sculpting,
//! generating, eroding and scattering.
//!
//! Before these, terrain work through MCP meant clicking brush buttons and
//! dragging across the viewport in screen pixels, guessing where a pixel lands
//! on a hill, and placing a tree meant guessing its height and finding it
//! buried. Everything here takes world `x`/`z` metres instead.
//!
//! The terrain itself is edited through the engine's `TerrainTools` service
//! (`renzora::terrain_tools`): this plugin cannot link the terrain crate, and
//! the service runs the same code the brushes and the Generate panel do, so an
//! edit made here is the same edit, undone the same way.

use bevy::prelude::*;
use renzora::serde_json::{json, Value};
use renzora::terrain_tools::{TerrainErode, TerrainGenerate, TerrainGraphBuild, TerrainPaint, TerrainSculpt, TerrainTools};

use crate::tools::{arg_entity, duplicate_copy, entity_id};
use crate::{edits, ToolReply};

pub fn is_terrain_tool(name: &str) -> bool {
    matches!(
        name,
        "terrain_height"
            | "place_on_ground"
            | "sculpt_terrain"
            | "generate_terrain"
            | "erode_terrain"
            | "scatter"
            | "paint_terrain"
            | "set_terrain_material"
            | "terrain_layers"
            | "build_terrain_graph"
    )
}

pub fn run(world: &mut World, name: &str, args: &Value) -> ToolReply {
    let Some(tools) = world.get_resource::<TerrainTools>().copied() else {
        return ToolReply::Failed("this editor has no terrain tools (the terrain editor plugin is not in the build)".into());
    };
    let result = match name {
        "terrain_height" => terrain_height(world, &tools, args),
        "place_on_ground" => place_on_ground(world, &tools, args),
        "sculpt_terrain" => sculpt(world, &tools, args),
        "generate_terrain" => generate(world, &tools, args),
        "erode_terrain" => erode(world, &tools, args),
        "scatter" => scatter(world, &tools, args),
        "paint_terrain" => paint(world, &tools, args),
        "set_terrain_material" => {
            let path = args.get("material").and_then(Value::as_str).unwrap_or("");
            (tools.set_material)(world, arg_entity(args, "terrain"), path)
        }
        "terrain_layers" => terrain_layers(world, &tools, args),
        "build_terrain_graph" => build_graph(world, &tools, args),
        _ => Err(format!("no such tool: {name}")),
    };
    match result {
        Ok(text) => ToolReply::Text(text),
        Err(why) => ToolReply::Failed(why),
    }
}

// ── Arguments ────────────────────────────────────────────────────────────────

fn num(args: &Value, key: &str) -> Option<f32> {
    args.get(key)?.as_f64().map(|f| f as f32)
}

fn xz(value: &Value) -> Option<Vec2> {
    let a = value.as_array()?;
    match a.len() {
        2 => Some(Vec2::new(a[0].as_f64()? as f32, a[1].as_f64()? as f32)),
        // A full position: its y is ignored.
        3 => Some(Vec2::new(a[0].as_f64()? as f32, a[2].as_f64()? as f32)),
        _ => None,
    }
}

fn points(args: &Value, key: &str) -> Result<Vec<Vec2>, String> {
    let list = args.get(key).and_then(Value::as_array).ok_or_else(|| format!("{key} must be a list of [x, z] points"))?;
    list.iter().map(|p| xz(p).ok_or_else(|| format!("{p} is not an [x, z] point"))).collect()
}

fn range(args: &Value, key: &str, default: (f32, f32)) -> (f32, f32) {
    args.get(key)
        .and_then(Value::as_array)
        .and_then(|a| Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32)))
        .unwrap_or(default)
}

// ── Lookup and placement ─────────────────────────────────────────────────────

fn terrain_height(world: &mut World, tools: &TerrainTools, args: &Value) -> Result<String, String> {
    let mut out = String::new();
    for p in points(args, "points")? {
        match (tools.ground)(world, p) {
            Some(hit) => out.push_str(&format!(
                "[{}, {}]: height {:.2}, slope {:.1} deg, terrain {}\n",
                p.x,
                p.y,
                hit.point.y,
                hit.slope_degrees(),
                entity_id(hit.terrain)
            )),
            None => out.push_str(&format!("[{}, {}]: no terrain here\n", p.x, p.y)),
        }
    }
    Ok(out)
}

fn place_on_ground(world: &mut World, tools: &TerrainTools, args: &Value) -> Result<String, String> {
    let entities: Vec<Entity> = match args.get("entities").and_then(Value::as_array) {
        Some(list) => list.iter().filter_map(|v| arg_entity(&json!({ "e": v }), "e")).collect(),
        None => arg_entity(args, "entity").into_iter().collect(),
    };
    if entities.is_empty() {
        return Err("place_on_ground needs entity or entities".into());
    }
    let offset = num(args, "offset").unwrap_or(0.0);
    let align = args.get("align_to_slope").and_then(Value::as_bool).unwrap_or(false);
    let mut out = String::new();
    for entity in entities {
        let (Some(global), Some(local)) =
            (world.get::<GlobalTransform>(entity).copied(), world.get::<Transform>(entity).copied())
        else {
            out.push_str(&format!("{} has no transform\n", entity_id(entity)));
            continue;
        };
        let at = global.translation();
        let Some(hit) = (tools.ground)(world, at.xz()) else {
            out.push_str(&format!("{}: no terrain under it\n", entity_id(entity)));
            continue;
        };
        // Moved by the difference in world height, so a parented entity moves
        // correctly in its parent's space as long as the parent is unrotated.
        let mut t = local;
        t.translation.y += hit.point.y + offset - at.y;
        if align {
            let yaw = Quat::from_rotation_y(local.rotation.to_euler(EulerRot::YXZ).0);
            t.rotation = Quat::from_rotation_arc(Vec3::Y, hit.normal) * yaw;
        }
        edits::set_transform(world, entity, t);
        out.push_str(&format!("{} on the ground at y {:.2}\n", entity_id(entity), hit.point.y + offset));
    }
    Ok(out)
}

// ── Terrain edits ────────────────────────────────────────────────────────────

fn sculpt(world: &mut World, tools: &TerrainTools, args: &Value) -> Result<String, String> {
    let path = match args.get("path") {
        Some(_) => points(args, "path")?,
        None => vec![args.get("at").and_then(xz).ok_or("sculpt_terrain needs path or at")?],
    };
    let stroke = TerrainSculpt {
        terrain: arg_entity(args, "terrain"),
        brush: args.get("brush").and_then(Value::as_str).ok_or("sculpt_terrain needs a brush")?.to_string(),
        path,
        radius: num(args, "radius").unwrap_or(20.0),
        strength: num(args, "strength").unwrap_or(0.5),
        falloff: num(args, "falloff").unwrap_or(0.6),
        target_height: num(args, "target_height"),
    };
    let passes = args.get("passes").and_then(Value::as_u64).unwrap_or(1).clamp(1, 50);
    // Report a pass that did something: the last one alone can rightly change
    // nothing (flatten already flat) and would hide that the first ones worked.
    let mut report = String::new();
    for _ in 0..passes {
        let this = (tools.sculpt)(world, &stroke)?;
        if report.is_empty() || !this.starts_with("the stroke changed nothing") {
            report = this;
        }
    }
    Ok(if passes > 1 { format!("{report} (x{passes})") } else { report })
}

fn generate(world: &mut World, tools: &TerrainTools, args: &Value) -> Result<String, String> {
    let region = match (args.get("min").and_then(xz), args.get("max").and_then(xz)) {
        (Some(a), Some(b)) => Some((a, b)),
        _ => None,
    };
    let request = TerrainGenerate {
        terrain: arg_entity(args, "terrain"),
        noise: args.get("noise").and_then(Value::as_str).map(str::to_string),
        scale: num(args, "scale"),
        height: num(args, "height"),
        base: num(args, "base"),
        octaves: args.get("octaves").and_then(Value::as_u64).map(|o| o as u32),
        exponent: num(args, "exponent"),
        seed: args.get("seed").and_then(Value::as_u64).map(|s| s as u32),
        blend: args.get("blend").and_then(Value::as_str).map(str::to_string),
        region,
        feather: num(args, "feather"),
    };
    (tools.generate)(world, &request)
}

fn erode(world: &mut World, tools: &TerrainTools, args: &Value) -> Result<String, String> {
    let defaults = TerrainErode::default();
    let area = match (args.get("center").and_then(xz), num(args, "radius")) {
        (Some(c), Some(r)) => Some((c, r)),
        _ => None,
    };
    let request = TerrainErode {
        terrain: arg_entity(args, "terrain"),
        droplets: args.get("droplets").and_then(Value::as_u64).map_or(defaults.droplets, |d| d.min(2_000_000) as u32),
        thermal_iterations: args
            .get("thermal_iterations")
            .and_then(Value::as_u64)
            .map_or(defaults.thermal_iterations, |t| t.min(500) as u32),
        repose_degrees: num(args, "repose_degrees").unwrap_or(defaults.repose_degrees),
        area,
        seed: args.get("seed").and_then(Value::as_u64).map_or(defaults.seed, |s| s as u32),
    };
    (tools.erode)(world, &request)
}

// ── Landscape layers and generator graphs ────────────────────────────────────

fn pair_arg(args: &Value, key: &str) -> Option<Vec2> {
    let a = args.get(key)?.as_array()?;
    Some(Vec2::new(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
}

fn paint(world: &mut World, tools: &TerrainTools, args: &Value) -> Result<String, String> {
    let path = match args.get("path") {
        Some(_) => points(args, "path")?,
        None => vec![args.get("at").and_then(xz).ok_or("paint_terrain needs path or at")?],
    };
    let stroke = TerrainPaint {
        terrain: arg_entity(args, "terrain"),
        layer: args.get("layer").and_then(Value::as_str).ok_or("paint_terrain needs a layer (see terrain_layers)")?.to_string(),
        mode: args.get("mode").and_then(Value::as_str).unwrap_or("paint").to_string(),
        path,
        radius: num(args, "radius").unwrap_or(20.0),
        strength: num(args, "strength").unwrap_or(0.5),
        target: num(args, "target"),
        falloff: num(args, "falloff").unwrap_or(0.5),
        slope_range: pair_arg(args, "slope"),
        height_range: pair_arg(args, "height"),
    };
    let passes = args.get("passes").and_then(Value::as_u64).unwrap_or(1).clamp(1, 50);
    let mut report = String::new();
    for _ in 0..passes {
        report = (tools.paint)(world, &stroke)?;
    }
    Ok(if passes > 1 { format!("{report} (x{passes})") } else { report })
}

fn terrain_layers(world: &mut World, tools: &TerrainTools, args: &Value) -> Result<String, String> {
    let layers = (tools.layers)(world, arg_entity(args, "terrain"))?;
    if layers.is_empty() {
        return Ok("the terrain's material has no layers (give it one with set_terrain_material)".into());
    }
    Ok(layers
        .iter()
        .map(|l| format!("{} ({} blend): {:.0}% painted\n", l.name, l.blend, l.coverage * 100.0))
        .collect())
}

fn build_graph(world: &mut World, tools: &TerrainTools, args: &Value) -> Result<String, String> {
    let request = TerrainGraphBuild {
        terrain: arg_entity(args, "terrain"),
        graph: args.get("graph").and_then(Value::as_str).map(str::to_string),
        seed: args.get("seed").and_then(Value::as_u64),
    };
    (tools.build_graph)(world, &request)
}

// ── Scatter ──────────────────────────────────────────────────────────────────

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 40) as f32) / (1u64 << 24) as f32
    }
}

/// Scatter, recorded as one undo step. Redo runs [`scatter_spawn`] again with
/// the same seed, which lands every copy in the same place; it must not
/// record anything itself, as it runs from inside the undo stack.
fn scatter(world: &mut World, tools: &TerrainTools, args: &Value) -> Result<String, String> {
    let (text, group) = scatter_spawn(world, tools, args)?;
    let label = format!("scatter {}", world.get::<Name>(group).map_or("scatter", |n| n.as_str()));
    let replay = args.clone();
    edits::record_spawn(world, group, label, move |world| {
        let tools = world.get_resource::<TerrainTools>().copied()?;
        scatter_spawn(world, &tools, &replay).ok().map(|(_, g)| g)
    });
    Ok(text)
}

/// Copies of `source` placed on the ground where the rules allow, under one
/// new group entity. Returns the summary and the group.
fn scatter_spawn(world: &mut World, tools: &TerrainTools, args: &Value) -> Result<(String, Entity), String> {
    let source = arg_entity(args, "source").ok_or("scatter needs a source entity to copy")?;
    if world.get_entity(source).is_err() {
        return Err(format!("{} is not a live entity", entity_id(source)));
    }
    let count = args.get("count").and_then(Value::as_u64).unwrap_or(20).clamp(1, 2000) as usize;
    let (lo, hi) = match (args.get("min").and_then(xz), args.get("max").and_then(xz), args.get("center").and_then(xz)) {
        (Some(a), Some(b), _) => (a.min(b), a.max(b)),
        (_, _, Some(c)) => {
            let r = num(args, "radius").unwrap_or(50.0);
            (c - Vec2::splat(r), c + Vec2::splat(r))
        }
        _ => return Err("scatter needs an area: min and max, or center and radius".into()),
    };
    let circle = args.get("center").and_then(xz).zip(num(args, "radius"));
    let slope = range(args, "slope", (0.0, 30.0));
    let height = range(args, "height", (f32::MIN, f32::MAX));
    let scale = range(args, "scale", (0.8, 1.25));
    let spacing = num(args, "min_spacing").unwrap_or(0.0);
    let sink = num(args, "sink").unwrap_or(0.0);
    let align = args.get("align_to_slope").and_then(Value::as_bool).unwrap_or(false);
    let random_yaw = args.get("random_yaw").and_then(Value::as_bool).unwrap_or(true);
    let seed = args.get("seed").and_then(Value::as_u64).unwrap_or(1);
    let base_scale = world.get::<Transform>(source).map_or(Vec3::ONE, |t| t.scale);
    let source_name = world.get::<Name>(source).map_or("scatter".to_string(), |n| n.as_str().to_string());
    let group_name = args.get("name").and_then(Value::as_str).map_or(format!("{source_name}_scatter"), str::to_string);

    // Find the spots first, so a scatter that finds nowhere it may go spawns
    // nothing rather than an empty group.
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut spots: Vec<(Vec3, Vec3, f32, f32)> = Vec::new();
    let mut attempts = 0;
    while spots.len() < count && attempts < count * 40 {
        attempts += 1;
        let p = Vec2::new(lo.x + (hi.x - lo.x) * rng.next(), lo.y + (hi.y - lo.y) * rng.next());
        if circle.is_some_and(|(c, r)| p.distance(c) > r) {
            continue;
        }
        let Some(hit) = (tools.ground)(world, p) else { continue };
        let s = hit.slope_degrees();
        if s < slope.0 || s > slope.1 || hit.point.y < height.0 || hit.point.y > height.1 {
            continue;
        }
        if spacing > 0.0 && spots.iter().any(|(q, ..)| q.xz().distance(p) < spacing) {
            continue;
        }
        let yaw = if random_yaw { rng.next() * std::f32::consts::TAU } else { 0.0 };
        let size = scale.0 + (scale.1 - scale.0) * rng.next();
        spots.push((hit.point, hit.normal, yaw, size));
    }
    if spots.is_empty() {
        return Err(format!(
            "found no ground in that area matching slope {:?} and height {:?} after {attempts} tries",
            slope, height
        ));
    }

    let group = world
        .spawn((Name::new(group_name.clone()), Transform::IDENTITY, Visibility::default()))
        .id();
    for (point, normal, yaw, size) in &spots {
        let copy = duplicate_copy(world, source, None);
        let up = if align { Quat::from_rotation_arc(Vec3::Y, *normal) } else { Quat::IDENTITY };
        let rotation = up * Quat::from_rotation_y(*yaw) * world.get::<Transform>(source).map_or(Quat::IDENTITY, |t| t.rotation);
        let transform = Transform {
            translation: *point - normal * sink,
            rotation,
            scale: base_scale * *size,
        };
        world.entity_mut(copy).insert((transform, ChildOf(group)));
    }

    let missing = count - spots.len();
    Ok((
        format!(
            "scattered {} copies of {} under {} [{}]{}",
            spots.len(),
            entity_id(source),
            group_name,
            entity_id(group),
            if missing > 0 { format!("; {missing} fewer than asked, the rest of the area does not match the rules") } else { String::new() }
        ),
        group,
    ))
}

// ── Catalogue ────────────────────────────────────────────────────────────────

pub fn catalogue() -> Value {
    let point = json!({ "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 3, "description": "[x, z] in world metres ([x, y, z] also works; y is ignored)." });
    let points = json!({ "type": "array", "items": point, "description": "World [x, z] points." });
    let pair = |d: &str| json!({ "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2, "description": d });
    json!([
        {
            "name": "terrain_height",
            "description": "The ground height and slope at world [x, z] points, on whichever terrain is there. Use it to plan placement and strokes.",
            "inputSchema": { "type": "object", "properties": { "points": points }, "required": ["points"] }
        },
        {
            "name": "place_on_ground",
            "description": "Drop entities onto the terrain under them, keeping x and z. Undoable.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": { "type": "string", "description": "One entity." },
                    "entities": { "type": "array", "items": { "type": "string" }, "description": "Several entities." },
                    "offset": { "type": "number", "description": "Metres above the ground (negative sinks it in, which hides a model's flat base on a slope)." },
                    "align_to_slope": { "type": "boolean", "description": "Tilt to the ground's slope, keeping its heading (default false)." }
                }
            }
        },
        {
            "name": "sculpt_terrain",
            "description": "Drag a terrain brush along a path of world [x, z] points, exactly as the Sculpt tab's brushes do. One undo step. Brushes: raise, lower, sculpt, noise (strength in metres at the stroke's centre; negative strength reverses raise or sculpt), and smooth, flatten, set_height, ramp, terrace, relax, pinch, retop, cliff, erase, erosion (thermal weathering) and hydro (rain carving gullies) with strength 0..1 per pass.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "brush": { "type": "string" },
                    "path": points,
                    "at": point,
                    "radius": { "type": "number", "description": "Brush radius in metres (default 20)." },
                    "strength": { "type": "number", "description": "Metres for raise/lower/sculpt/noise; 0..1 for the rest (default 0.5)." },
                    "falloff": { "type": "number", "description": "0..1, how much of the radius fades out (default 0.6)." },
                    "target_height": { "type": "number", "description": "World height for flatten, set_height and ramp (default the ground at the first point)." },
                    "passes": { "type": "integer", "description": "Repeat the stroke this many times (default 1)." },
                    "terrain": { "type": "string", "description": "Terrain entity (default the one under the path)." }
                },
                "required": ["brush"]
            }
        },
        {
            "name": "generate_terrain",
            "description": "Fill the terrain, or a rectangle of it, with generated landforms, as the Generate panel does. One undo step. Leave a parameter out to keep the panel's.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "noise": { "type": "string", "description": "hybrid (ridges with rolling flanks), ridged, fbm, billow or warped." },
                    "scale": { "type": "number", "description": "Metres per feature: 60 gives hills, 300 a mountain range." },
                    "height": { "type": "number", "description": "Peak-to-floor metres, capped by the terrain's height range." },
                    "base": { "type": "number", "description": "World height of the floor." },
                    "octaves": { "type": "integer" },
                    "exponent": { "type": "number", "description": "Above 1 sharpens peaks and widens valleys (default 1.6)." },
                    "seed": { "type": "integer" },
                    "blend": { "type": "string", "description": "replace (default), add, subtract, max or min." },
                    "min": point,
                    "max": point,
                    "feather": { "type": "number", "description": "0..1 edge blend of the rectangle." },
                    "terrain": { "type": "string" }
                }
            }
        },
        {
            "name": "erode_terrain",
            "description": "Weather the terrain: simulated rain carves branching gullies and fills valleys with sediment, then loose slopes settle to their angle of repose. Run it after generating to turn smooth noise into believable ground. One undo step.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "droplets": { "type": "integer", "description": "Raindrops (default 60000; 200000 is heavy weathering on 512 m)." },
                    "thermal_iterations": { "type": "integer", "description": "Slope settling passes (default 8)." },
                    "repose_degrees": { "type": "number", "description": "Slope loose ground rests at (default 38)." },
                    "center": point,
                    "radius": { "type": "number", "description": "With center: erode only this disc." },
                    "seed": { "type": "integer" },
                    "terrain": { "type": "string" }
                }
            }
        },
        {
            "name": "scatter",
            "description": "Place copies of an entity (a tree, rock, prop or model) on the terrain across an area, only where the slope and height rules allow, with random yaw and size. The copies go under one new group entity, one undo step. Same seed, same layout.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "The entity to copy. Hide or move it after if it should not stay." },
                    "count": { "type": "integer", "description": "How many (default 20, at most 2000)." },
                    "min": point,
                    "max": point,
                    "center": point,
                    "radius": { "type": "number", "description": "With center: a disc instead of a rectangle." },
                    "slope": pair("[min, max] degrees allowed (default [0, 30])."),
                    "height": pair("[min, max] world heights allowed."),
                    "scale": pair("[min, max] size multiplier (default [0.8, 1.25])."),
                    "min_spacing": { "type": "number", "description": "Metres between copies." },
                    "sink": { "type": "number", "description": "Metres to push each copy into the ground." },
                    "align_to_slope": { "type": "boolean", "description": "Tilt copies to the ground (rocks yes, trees no; default false)." },
                    "random_yaw": { "type": "boolean", "description": "Default true." },
                    "seed": { "type": "integer" },
                    "name": { "type": "string", "description": "Name of the group (default <source>_scatter)." }
                },
                "required": ["source"]
            }
        },
        {
            "name": "terrain_layers",
            "description": "The paintable layers a terrain's landscape material offers (grass, rock, snow...), with how much of the terrain each covers. Use the names with paint_terrain.",
            "inputSchema": {
                "type": "object",
                "properties": { "terrain": { "type": "string" } }
            }
        },
        {
            "name": "paint_terrain",
            "description": "Paint a landscape layer along a path of world [x, z] points: the layer rises and the others fall in proportion. Modes: paint, erase, smooth, fill. Slope and height ranges keep the paint to matching ground (rock on steep slopes, snow up high). One undo step.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "layer": { "type": "string", "description": "Layer name, from terrain_layers." },
                    "mode": { "type": "string", "description": "paint (default), erase, smooth or fill." },
                    "path": points,
                    "at": point,
                    "radius": { "type": "number", "description": "Brush radius in metres (default 20)." },
                    "strength": { "type": "number", "description": "0..1 per pass (default 0.5)." },
                    "target": { "type": "number", "description": "0..1 weight paint and fill bring the layer to (default 1)." },
                    "falloff": { "type": "number", "description": "0..1 soft edge (default 0.5)." },
                    "slope": pair("[min, max] degrees the paint is kept to."),
                    "height": pair("[min, max] world heights the paint is kept to."),
                    "passes": { "type": "integer", "description": "Repeat the stroke (default 1, at most 50)." },
                    "terrain": { "type": "string" }
                },
                "required": ["layer"]
            }
        },
        {
            "name": "set_terrain_material",
            "description": "Give a terrain a landscape .material (a node-graph material with Layer Blend nodes). An empty or missing path is Alpine Landscape, the engine default, copied into the project if needed. One undo step.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "material": { "type": "string", "description": "Project-relative .material path." },
                    "terrain": { "type": "string" }
                }
            }
        },
        {
            "name": "build_terrain_graph",
            "description": "Build a terrain generator graph (.tgraph) into a terrain: replaces the generated ground and keeps hand sculpting on top. With no graph, rebuilds the one the terrain already uses. Not undoable: rebuild with the previous graph or seed to go back.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "graph": { "type": "string", "description": "Project-relative .tgraph path; stored on the terrain." },
                    "seed": { "type": "integer", "description": "Stored on the terrain too." },
                    "terrain": { "type": "string" }
                }
            }
        }
    ])
}
