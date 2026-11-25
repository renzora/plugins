//! 3MF package in, GLB out.
//!
//! Each build item becomes one glTF node holding one mesh: the item's object
//! and every component under it, placed by their transforms, with a primitive
//! per material. 3MF stores no normals and is usually faceted CAD or a scan,
//! so normals are made here, smoothed across edges flatter than 40 degrees and
//! kept sharp across steeper ones, which renders both a printed bracket and a
//! sculpted figure the way they look in a slicer.

use crate::glb::{self, GlbMaterial, GlbMesh, GlbPrimitive};
use crate::model::{clean, Material, Package, Part, Transform};
use renzora::{ConvertedModel, ImportConvertOptions};
use std::collections::HashMap;
use std::path::Path;

/// Edges sharper than this stay sharp. Not 30: a low-poly round part (a
/// twelve-sided torus or cylinder) has exactly 30 degrees between faces, and
/// rounding put some of its edges either side of the line, shading it in
/// patches. A machined edge is 90 degrees and stays sharp either way.
const CREASE_DEGREES: f32 = 40.0;

/// Components nest; a file that nests deeper than this is cyclic or hostile.
const MAX_DEPTH: usize = 32;

/// One build item's geometry, in the 3MF frame and metres.
#[derive(Default)]
struct Gathered {
    positions: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
    /// Material per triangle, as an index into the shared list.
    materials: Vec<Option<usize>>,
}

/// Materials by their source identity, so a colour used by many objects is one
/// material in the GLB.
#[derive(Default)]
struct Materials {
    list: Vec<Material>,
    by_key: HashMap<(String, u32, u32), usize>,
    by_name: HashMap<String, usize>,
}

impl Materials {
    fn resolve(&mut self, part_path: &str, part: &Part, key: Option<(u32, u32)>) -> Option<usize> {
        let (pid, index) = key?;
        let map_key = (part_path.to_string(), pid, index);
        if let Some(i) = self.by_key.get(&map_key) {
            return Some(*i);
        }
        let material = part.groups.get(&pid)?.get(index as usize)?.clone();
        // Two materials of one name are one material: the importer writes a
        // `.material` per name.
        let i = match self.by_name.get(&material.name) {
            Some(i) => *i,
            None => {
                self.list.push(material.clone());
                self.by_name.insert(material.name.clone(), self.list.len() - 1);
                self.list.len() - 1
            }
        };
        self.by_key.insert(map_key, i);
        Some(i)
    }
}

fn gather(
    parts: &HashMap<String, Part>,
    path: &str,
    id: u32,
    transform: &Transform,
    out: &mut Gathered,
    materials: &mut Materials,
    warnings: &mut Vec<String>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        warnings.push(format!("object {id} nests components more than {MAX_DEPTH} deep; the rest was skipped"));
        return;
    }
    let Some(part) = parts.get(path) else {
        warnings.push(format!("a component names a part that is not in the package ({path})"));
        return;
    };
    let Some(object) = part.objects.get(&id) else {
        warnings.push(format!("{path}: no object {id}"));
        return;
    };
    // Supports are printing aids, not the model.
    if matches!(object.kind.as_str(), "support" | "other") {
        return;
    }

    let unit = part.unit_to_metres;
    let base = out.positions.len() as u32;
    for v in &object.vertices {
        let p = transform.apply(*v);
        out.positions.push([p[0] * unit, p[1] * unit, p[2] * unit]);
    }
    let mirrored = transform.mirrors();
    let count = object.vertices.len() as u32;
    for (tri, key) in &object.triangles {
        if tri.iter().any(|i| *i >= count) {
            continue;
        }
        let [a, b, c] = *tri;
        out.triangles.push(if mirrored { [a + base, c + base, b + base] } else { [a + base, b + base, c + base] });
        out.materials.push(materials.resolve(path, part, *key));
    }

    for component in &object.components {
        let child_path = component.path.as_deref().map(clean).unwrap_or_else(|| path.to_string());
        let child_transform = component.transform.then(transform);
        gather(parts, &child_path, component.object, &child_transform, out, materials, warnings, depth + 1);
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1e-12 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 1.0, 0.0]
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Split one gathered item into primitives by material, with crease-smoothed
/// normals and vertices welded where position and normal agree.
fn build_mesh(name: String, g: &Gathered) -> GlbMesh {
    let face_normals: Vec<[f32; 3]> = g
        .triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| g.positions[i as usize]);
            normalize(cross(sub(b, a), sub(c, a)))
        })
        .collect();
    let mut adjacent: Vec<Vec<u32>> = vec![Vec::new(); g.positions.len()];
    for (f, t) in g.triangles.iter().enumerate() {
        for v in t {
            adjacent[*v as usize].push(f as u32);
        }
    }
    let crease = CREASE_DEGREES.to_radians().cos();

    let mut by_material: Vec<(Option<usize>, GlbPrimitive, HashMap<(u32, [i32; 3]), u32>)> = Vec::new();
    for (f, t) in g.triangles.iter().enumerate() {
        let material = g.materials[f];
        let slot = match by_material.iter().position(|(m, _, _)| *m == material) {
            Some(i) => i,
            None => {
                by_material.push((material, GlbPrimitive { material, ..Default::default() }, HashMap::new()));
                by_material.len() - 1
            }
        };
        let (_, prim, weld) = &mut by_material[slot];
        let own = face_normals[f];
        for v in t {
            let mut n = [0.0f32; 3];
            for other in &adjacent[*v as usize] {
                let fnormal = face_normals[*other as usize];
                if dot(fnormal, own) >= crease {
                    n = [n[0] + fnormal[0], n[1] + fnormal[1], n[2] + fnormal[2]];
                }
            }
            let n = normalize(n);
            let quantized = n.map(|c| (c * 1000.0).round() as i32);
            let index = *weld.entry((*v, quantized)).or_insert_with(|| {
                prim.positions.push(g.positions[*v as usize]);
                prim.normals.push(n);
                (prim.positions.len() - 1) as u32
            });
            prim.indices.push(index);
        }
    }
    GlbMesh { name, primitives: by_material.into_iter().map(|(_, p, _)| p).collect() }
}

/// sRGB to linear, as glTF's base colour factor is linear.
fn linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn convert(path: &Path, options: &ImportConvertOptions) -> Result<ConvertedModel, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("could not open {} ({e})", path.display()))?;
    let mut package = Package::open(std::io::BufReader::new(file))?;
    package.load_all()?;

    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "3MF".into());
    let mut warnings = Vec::new();
    let mut materials = Materials::default();
    let mut meshes = Vec::new();

    let root = package.root.clone();
    let build = &package.parts[&root].build;
    if build.is_empty() {
        return Err("the model's build is empty: there is nothing to make".into());
    }
    for (i, item) in build.iter().enumerate() {
        let item_path = item.path.as_deref().map(clean).unwrap_or_else(|| root.clone());
        let mut gathered = Gathered::default();
        gather(&package.parts, &item_path, item.object, &item.transform, &mut gathered, &mut materials, &mut warnings, 0);
        if gathered.triangles.is_empty() {
            continue;
        }
        // 3MF is Z up; the engine is Y up. `(x, y, z) -> (x, z, -y)` is a
        // rotation, so winding survives it. Then the user's scale.
        let s = options.scale;
        for p in &mut gathered.positions {
            *p = [p[0] * s, p[2] * s, -p[1] * s];
        }
        let name = package.parts[&item_path]
            .objects
            .get(&item.object)
            .and_then(|o| o.name.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| if build.len() == 1 { stem.clone() } else { format!("{stem} {}", i + 1) });
        meshes.push(build_mesh(name, &gathered));
    }
    if meshes.is_empty() {
        return Err("no build item has any triangles".into());
    }

    let glb_materials: Vec<GlbMaterial> = materials
        .list
        .iter()
        .map(|m| GlbMaterial {
            name: m.name.clone(),
            color: [linear(m.color[0]), linear(m.color[1]), linear(m.color[2]), m.color[3]],
        })
        .collect();
    Ok(ConvertedModel { glb: glb::write(&meshes, &glb_materials), warnings })
}
