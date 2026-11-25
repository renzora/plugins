//! A small binary glTF writer: meshes as nodes, positions, normals, indices
//! and flat-colour PBR materials. Everything the importer needs from a 3MF.

use renzora::serde_json::{json, Value};

const FLOAT: u32 = 5126;
const UNSIGNED_INT: u32 = 5125;
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;

#[derive(Default)]
pub struct GlbPrimitive {
    pub material: Option<usize>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

pub struct GlbMesh {
    pub name: String,
    pub primitives: Vec<GlbPrimitive>,
}

pub struct GlbMaterial {
    pub name: String,
    /// Linear RGB and alpha.
    pub color: [f32; 4],
}

#[derive(Default)]
struct Buffer {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Buffer {
    fn view(&mut self, bytes: &[u8], target: u32) -> usize {
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        self.views.push(json!({
            "buffer": 0,
            "byteOffset": self.bin.len(),
            "byteLength": bytes.len(),
            "target": target,
        }));
        self.bin.extend_from_slice(bytes);
        self.views.len() - 1
    }

    fn vec3(&mut self, data: &[[f32; 3]], bounds: bool) -> usize {
        let bytes: Vec<u8> = data.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
        let view = self.view(&bytes, ARRAY_BUFFER);
        let mut accessor = json!({ "bufferView": view, "componentType": FLOAT, "count": data.len(), "type": "VEC3" });
        if bounds {
            let mut min = [f32::MAX; 3];
            let mut max = [f32::MIN; 3];
            for v in data {
                for i in 0..3 {
                    min[i] = min[i].min(v[i]);
                    max[i] = max[i].max(v[i]);
                }
            }
            accessor["min"] = json!(min);
            accessor["max"] = json!(max);
        }
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    fn indices(&mut self, data: &[u32]) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|i| i.to_le_bytes()).collect();
        let view = self.view(&bytes, ELEMENT_ARRAY_BUFFER);
        self.accessors.push(json!({ "bufferView": view, "componentType": UNSIGNED_INT, "count": data.len(), "type": "SCALAR" }));
        self.accessors.len() - 1
    }
}

pub fn write(meshes: &[GlbMesh], materials: &[GlbMaterial]) -> Vec<u8> {
    let mut b = Buffer::default();
    let mut gltf_meshes = Vec::new();
    let mut nodes = Vec::new();
    for (i, mesh) in meshes.iter().enumerate() {
        let primitives: Vec<Value> = mesh
            .primitives
            .iter()
            .map(|p| {
                let mut prim = json!({
                    "attributes": { "POSITION": b.vec3(&p.positions, true), "NORMAL": b.vec3(&p.normals, false) },
                    "indices": b.indices(&p.indices),
                    "mode": 4,
                });
                if let Some(m) = p.material {
                    prim["material"] = json!(m);
                }
                prim
            })
            .collect();
        gltf_meshes.push(json!({ "name": mesh.name, "primitives": primitives }));
        nodes.push(json!({ "name": mesh.name, "mesh": i }));
    }
    let gltf_materials: Vec<Value> = materials
        .iter()
        .map(|m| {
            json!({
                "name": m.name,
                "pbrMetallicRoughness": { "baseColorFactor": m.color, "metallicFactor": 0.0, "roughnessFactor": 0.6 },
                "alphaMode": if m.color[3] < 1.0 { "BLEND" } else { "OPAQUE" },
            })
        })
        .collect();

    let mut bin = b.bin;
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let mut doc = json!({
        "asset": { "version": "2.0", "generator": "Renzora 3MF Import" },
        "scene": 0,
        "scenes": [{ "nodes": (0..nodes.len()).collect::<Vec<_>>() }],
        "nodes": nodes,
        "meshes": gltf_meshes,
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": b.views,
        "accessors": b.accessors,
    });
    if !gltf_materials.is_empty() {
        doc["materials"] = json!(gltf_materials);
    }

    let mut text = renzora::serde_json::to_vec(&doc).unwrap_or_default();
    while text.len() % 4 != 0 {
        text.push(b' ');
    }
    let total = 12 + 8 + text.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&text);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}
