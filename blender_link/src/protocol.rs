//! What the add-on sends, decoded from a frame's header and payload.
//!
//! Message types (the header's `"type"`):
//!
//! | type | header | payload |
//! |---|---|---|
//! | `hello` | `file` (the .blend's name), `blender` (version) | none |
//! | `object` | `name`, `kind` (`mesh`, `light`, `empty`), `translation`, `rotation` (xyzw), `scale`, `visible` | none |
//! | `rename` | `from`, `to` | none |
//! | `mesh` | `object`, `primitives`: `[{material, vertices, indices, uv}]` | per primitive: positions, normals, uvs if `uv`, indices |
//! | `material` | `name`, factors, `textures`: `{base_color, normal}` as `{key, mime, offset, length}` (length 0: bytes sent before under `key`) | the new texture files' bytes |
//! | `light` | `object`, `light` (`point`, `spot`, `sun`, `area`), `color`, `power`, `radius`, `range`, `spot_size`, `spot_blend`, `shadows` | none |
//! | `remove` | `object` | none |
//! | `synced` | `objects`: every object the full sync sent | none |
//!
//! Everything is already in the engine's frame when it arrives: Y up, glTF
//! texture coordinates, metres. The add-on converts, because it has the
//! matrices in hand and numpy to do it in bulk.

use renzora::serde_json::Value;

pub struct Primitive {
    pub material: Option<String>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Option<Vec<[f32; 2]>>,
    pub indices: Vec<u32>,
}

pub struct MeshData {
    pub primitives: Vec<Primitive>,
}

/// An image file exactly as it sits on disk in Blender (PNG or JPEG), so the
/// editor decodes it with its own loaders and the GLB embeds it unchanged.
///
/// `key` names the image and the version of it (path and modification time),
/// so a material changed by a slider does not send its textures again: the
/// add-on sends the bytes the first time a key is seen and only the key after.
#[derive(Clone)]
pub struct Texture {
    pub key: String,
    pub bytes: std::sync::Arc<Vec<u8>>,
    pub mime: String,
}

/// A texture as a material message names it: the bytes when they are new,
/// otherwise only the key of bytes sent before.
pub struct TextureRef {
    pub key: String,
    pub fresh: Option<(Vec<u8>, String)>,
}

impl Texture {
    pub fn extension(&self) -> &'static str {
        if self.mime == "image/jpeg" {
            "jpeg"
        } else {
            "png"
        }
    }
}

#[derive(Clone)]
pub struct MaterialData {
    pub name: String,
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
    pub emissive_strength: f32,
    pub blend: bool,
    pub double_sided: bool,
    pub base_color_texture: Option<Texture>,
    pub normal_texture: Option<Texture>,
    pub normal_scale: f32,
}

#[derive(Clone, Copy)]
pub enum LightKind {
    Point,
    Spot,
    Sun,
}

#[derive(Clone, Copy)]
pub struct LightData {
    pub kind: LightKind,
    pub color: [f32; 3],
    /// Watts for point, spot and area lights; W/m² for a sun. Blender's units.
    pub power: f32,
    pub radius: f32,
    pub range: Option<f32>,
    pub spot_size: f32,
    pub spot_blend: f32,
    pub shadows: bool,
}

pub fn text<'a>(header: &'a Value, key: &str) -> Option<&'a str> {
    header.get(key).and_then(Value::as_str)
}

fn number(header: &Value, key: &str, fallback: f32) -> f32 {
    header
        .get(key)
        .and_then(Value::as_f64)
        .map_or(fallback, |v| v as f32)
}

fn array<const N: usize>(header: &Value, key: &str, fallback: [f32; N]) -> [f32; N] {
    let mut out = fallback;
    if let Some(items) = header.get(key).and_then(Value::as_array) {
        for (slot, item) in out.iter_mut().zip(items) {
            if let Some(v) = item.as_f64() {
                *slot = v as f32;
            }
        }
    }
    out
}

pub fn vec3(header: &Value, key: &str, fallback: [f32; 3]) -> [f32; 3] {
    array(header, key, fallback)
}

pub fn vec4(header: &Value, key: &str, fallback: [f32; 4]) -> [f32; 4] {
    array(header, key, fallback)
}

/// Reads fixed-size little-endian arrays off the front of a payload.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| "the mesh payload is shorter than its header says".to_string())?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn floats<const N: usize>(&mut self, count: usize) -> Result<Vec<[f32; N]>, String> {
        let raw = self.take(count * N * 4)?;
        Ok(raw
            .chunks_exact(N * 4)
            .map(|chunk| {
                let mut v = [0f32; N];
                for (slot, b) in v.iter_mut().zip(chunk.chunks_exact(4)) {
                    *slot = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                }
                v
            })
            .collect())
    }

    fn indices(&mut self, count: usize) -> Result<Vec<u32>, String> {
        let raw = self.take(count * 4)?;
        Ok(raw
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect())
    }
}

pub fn mesh(header: &Value, payload: &[u8]) -> Result<MeshData, String> {
    let mut cursor = Cursor { bytes: payload, at: 0 };
    let mut primitives = Vec::new();
    for p in header
        .get("primitives")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let vertices = p.get("vertices").and_then(Value::as_u64).unwrap_or(0) as usize;
        let index_count = p.get("indices").and_then(Value::as_u64).unwrap_or(0) as usize;
        let has_uv = p.get("uv").and_then(Value::as_bool).unwrap_or(false);
        let positions = cursor.floats::<3>(vertices)?;
        let normals = cursor.floats::<3>(vertices)?;
        let uvs = if has_uv { Some(cursor.floats::<2>(vertices)?) } else { None };
        let indices = cursor.indices(index_count)?;
        if let Some(bad) = indices.iter().find(|i| **i as usize >= vertices) {
            return Err(format!("an index ({bad}) points past the {vertices} vertices sent"));
        }
        primitives.push(Primitive {
            material: text(p, "material").map(str::to_string),
            positions,
            normals,
            uvs,
            indices,
        });
    }
    Ok(MeshData { primitives })
}

fn texture(header: &Value, slot: &str, payload: &[u8]) -> Option<TextureRef> {
    let t = header.get("textures")?.get(slot)?;
    let key = text(t, "key")?.to_string();
    let offset = t.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let length = t.get("length").and_then(Value::as_u64).unwrap_or(0) as usize;
    let fresh = if length > 0 {
        let bytes = payload.get(offset..offset.checked_add(length)?)?.to_vec();
        let mime = text(t, "mime").unwrap_or("image/png").to_string();
        Some((bytes, mime))
    } else {
        None
    };
    Some(TextureRef { key, fresh })
}

/// A material message, with its textures still as references for the caller
/// to resolve against the bytes it has already been sent.
pub struct MaterialMessage {
    pub data: MaterialData,
    pub base_color: Option<TextureRef>,
    pub normal: Option<TextureRef>,
}

pub fn material(header: &Value, payload: &[u8]) -> Option<MaterialMessage> {
    let data = MaterialData {
        name: text(header, "name")?.to_string(),
        base_color: vec4(header, "base_color", [0.8, 0.8, 0.8, 1.0]),
        metallic: number(header, "metallic", 0.0),
        roughness: number(header, "roughness", 0.5),
        emissive: vec3(header, "emissive", [0.0; 3]),
        emissive_strength: number(header, "emissive_strength", 0.0),
        blend: header.get("blend").and_then(Value::as_bool).unwrap_or(false),
        double_sided: header
            .get("double_sided")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        base_color_texture: None,
        normal_texture: None,
        normal_scale: number(header, "normal_scale", 1.0),
    };
    Some(MaterialMessage {
        data,
        base_color: texture(header, "base_color", payload),
        normal: texture(header, "normal", payload),
    })
}

pub fn light(header: &Value) -> LightData {
    let kind = match text(header, "light") {
        Some("spot") => LightKind::Spot,
        Some("sun") => LightKind::Sun,
        // Bevy has no area light; a point light of the same power is the
        // nearest thing, and says so in the README.
        _ => LightKind::Point,
    };
    LightData {
        kind,
        color: vec3(header, "color", [1.0; 3]),
        power: number(header, "power", 1000.0),
        radius: number(header, "radius", 0.1),
        range: header.get("range").and_then(Value::as_f64).map(|v| v as f32),
        spot_size: number(header, "spot_size", 45f32.to_radians()),
        spot_blend: number(header, "spot_blend", 0.15),
        shadows: header.get("shadows").and_then(Value::as_bool).unwrap_or(true),
    }
}
