//! Writes a linked object as a binary glTF, which is how it is saved.
//!
//! The scene stores a model as `MeshInstanceData { model_path }`, a file the
//! engine loads on scene load, in Play and in an exported game. So a linked
//! object is baked into exactly that: one `.glb` per object, in the object's
//! own space (the entity's `Transform` carries where it is), with its
//! materials and their textures embedded.
//!
//! Written here rather than by Blender's own glTF exporter because the editor
//! already holds every vertex: re-exporting from Blender on each edit would
//! cost a second trip through Python for data that has just arrived.

use crate::protocol::{MaterialData, MeshData, Texture};
use renzora::serde_json::{json, Value};
use std::collections::HashMap;

const FLOAT: u32 = 5126;
const UNSIGNED_INT: u32 = 5125;
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;

#[derive(Default)]
struct Builder {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Builder {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        let mut view = json!({
            "buffer": 0,
            "byteOffset": self.bin.len(),
            "byteLength": bytes.len(),
        });
        if let Some(target) = target {
            view["target"] = json!(target);
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(view);
        self.views.len() - 1
    }

    fn floats<const N: usize>(&mut self, data: &[[f32; N]], kind: &str, bounds: bool) -> usize {
        let mut bytes = Vec::with_capacity(data.len() * N * 4);
        for v in data {
            for c in v {
                bytes.extend_from_slice(&c.to_le_bytes());
            }
        }
        let view = self.view(&bytes, Some(ARRAY_BUFFER));
        let mut accessor = json!({
            "bufferView": view,
            "componentType": FLOAT,
            "count": data.len(),
            "type": kind,
        });
        // glTF requires them on POSITION, and a loader uses them for bounds.
        if bounds {
            let mut min = [f32::MAX; N];
            let mut max = [f32::MIN; N];
            for v in data {
                for i in 0..N {
                    min[i] = min[i].min(v[i]);
                    max[i] = max[i].max(v[i]);
                }
            }
            accessor["min"] = json!(min.to_vec());
            accessor["max"] = json!(max.to_vec());
        }
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    fn indices(&mut self, data: &[u32]) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|i| i.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(ELEMENT_ARRAY_BUFFER));
        self.accessors.push(json!({
            "bufferView": view,
            "componentType": UNSIGNED_INT,
            "count": data.len(),
            "type": "SCALAR",
        }));
        self.accessors.len() - 1
    }
}

/// The `.glb` bytes for one object.
pub fn write(name: &str, mesh: &MeshData, materials: &HashMap<String, MaterialData>) -> Vec<u8> {
    let mut b = Builder::default();
    let mut images: Vec<Value> = Vec::new();
    let mut textures: Vec<Value> = Vec::new();
    let mut gltf_materials: Vec<Value> = Vec::new();
    let mut material_index: HashMap<String, usize> = HashMap::new();
    let mut uses_emissive_strength = false;

    let mut add_texture = |b: &mut Builder, tex: &Texture| -> usize {
        let view = b.view(&tex.bytes, None);
        images.push(json!({ "bufferView": view, "mimeType": tex.mime }));
        textures.push(json!({ "source": images.len() - 1 }));
        textures.len() - 1
    };

    let mut primitives = Vec::new();
    for p in &mesh.primitives {
        if p.indices.is_empty() {
            continue;
        }
        let mut attributes = json!({
            "POSITION": b.floats(&p.positions, "VEC3", true),
            "NORMAL": b.floats(&p.normals, "VEC3", false),
        });
        if let Some(uvs) = &p.uvs {
            attributes["TEXCOORD_0"] = json!(b.floats(uvs, "VEC2", false));
        }
        let mut primitive = json!({
            "attributes": attributes,
            "indices": b.indices(&p.indices),
            "mode": 4,
        });

        if let Some(mat) = p.material.as_ref().and_then(|m| materials.get(m)) {
            let index = match material_index.get(&mat.name) {
                Some(i) => *i,
                None => {
                    let mut pbr = json!({
                        "baseColorFactor": mat.base_color,
                        "metallicFactor": mat.metallic,
                        "roughnessFactor": mat.roughness,
                    });
                    if let Some(tex) = &mat.base_color_texture {
                        pbr["baseColorTexture"] = json!({ "index": add_texture(&mut b, tex) });
                    }
                    let mut m = json!({
                        "name": mat.name,
                        "pbrMetallicRoughness": pbr,
                        "doubleSided": mat.double_sided,
                        "alphaMode": if mat.blend { "BLEND" } else { "OPAQUE" },
                    });
                    if let Some(tex) = &mat.normal_texture {
                        m["normalTexture"] =
                            json!({ "index": add_texture(&mut b, tex), "scale": mat.normal_scale });
                    }
                    if mat.emissive_strength > 0.0 && mat.emissive.iter().any(|c| *c > 0.0) {
                        m["emissiveFactor"] = json!(mat.emissive);
                        if (mat.emissive_strength - 1.0).abs() > f32::EPSILON {
                            uses_emissive_strength = true;
                            m["extensions"] = json!({
                                "KHR_materials_emissive_strength": {
                                    "emissiveStrength": mat.emissive_strength
                                }
                            });
                        }
                    }
                    gltf_materials.push(m);
                    material_index.insert(mat.name.clone(), gltf_materials.len() - 1);
                    gltf_materials.len() - 1
                }
            };
            primitive["material"] = json!(index);
        }
        primitives.push(primitive);
    }

    // The node is named apart from the object: a loaded model's node becomes
    // a child entity, and one named exactly like its parent made the editor
    // renumber the parent (`bl_ball` came back as `bl_ball_1`).
    let node = format!("{name}_mesh");
    let mut doc = json!({
        "asset": { "version": "2.0", "generator": "Renzora Blender Live Link" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "name": node, "mesh": 0 }],
        "meshes": [{ "name": node, "primitives": primitives }],
        "buffers": [{ "byteLength": 0 }],
        "bufferViews": b.views,
        "accessors": b.accessors,
    });
    if !gltf_materials.is_empty() {
        doc["materials"] = json!(gltf_materials);
    }
    if !images.is_empty() {
        doc["images"] = json!(images);
        doc["textures"] = json!(textures);
    }
    if uses_emissive_strength {
        doc["extensionsUsed"] = json!(["KHR_materials_emissive_strength"]);
    }

    let mut bin = b.bin;
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    doc["buffers"][0]["byteLength"] = json!(bin.len());

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
