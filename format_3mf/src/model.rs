//! Reading a 3MF package into meshes placed in the world.
//!
//! A 3MF is an OPC package (a zip): `_rels/.rels` names the root model part,
//! usually `3D/3dmodel.model`, an XML document of `<resources>` (materials,
//! objects with a mesh or with components) and a `<build>` of the objects to
//! make, each with a transform. Slicers (PrusaSlicer, Bambu Studio) write the
//! "production" extension, where a component names its object in another model
//! part with `p:path`, so every part is read on demand.
//!
//! Element and attribute names are matched by local name, so whatever prefix
//! a file binds the materials or production namespaces to, it reads the same.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashMap;
use std::io::{Read, Seek};

/// An affine transform in 3MF's layout: a point is a row vector, `p * M`.
#[derive(Clone, Copy, Debug)]
pub struct Transform(pub [f32; 12]);

impl Transform {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);

    fn parse(text: &str) -> Option<Self> {
        let values: Vec<f32> = text.split_whitespace().filter_map(|v| v.parse().ok()).collect();
        let m: [f32; 12] = values.try_into().ok()?;
        Some(Self(m))
    }

    pub fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let m = &self.0;
        [
            p[0] * m[0] + p[1] * m[3] + p[2] * m[6] + m[9],
            p[0] * m[1] + p[1] * m[4] + p[2] * m[7] + m[10],
            p[0] * m[2] + p[1] * m[5] + p[2] * m[8] + m[11],
        ]
    }

    /// `self` then `then`: `p * self * then`.
    pub fn then(&self, then: &Transform) -> Transform {
        let a = &self.0;
        let b = &then.0;
        let mut out = [0.0; 12];
        for row in 0..3 {
            for col in 0..3 {
                out[row * 3 + col] = (0..3).map(|k| a[row * 3 + k] * b[k * 3 + col]).sum();
            }
        }
        for col in 0..3 {
            out[9 + col] = (0..3).map(|k| a[9 + k] * b[k * 3 + col]).sum::<f32>() + b[9 + col];
        }
        Transform(out)
    }

    /// A mirroring transform turns triangles inside out; the winding has to
    /// be reversed to keep them facing outwards.
    pub fn mirrors(&self) -> bool {
        let m = &self.0;
        let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
            + m[2] * (m[3] * m[7] - m[4] * m[6]);
        det < 0.0
    }
}

/// A material slot a triangle can name: a `<base>` of a `<basematerials>` or a
/// `<color>` of a `<colorgroup>`.
#[derive(Clone, Debug)]
pub struct Material {
    pub name: String,
    /// sRGB, 0..1, with alpha.
    pub color: [f32; 4],
}

/// One `<object>`.
#[derive(Default)]
pub struct Object {
    pub name: Option<String>,
    pub kind: String,
    pub vertices: Vec<[f32; 3]>,
    /// Vertex indices, and the material key of each triangle.
    pub triangles: Vec<([u32; 3], Option<(u32, u32)>)>,
    pub components: Vec<Component>,
}

pub struct Component {
    /// The part the object lives in; `None` for the same part.
    pub path: Option<String>,
    pub object: u32,
    pub transform: Transform,
}

/// One model part, parsed.
#[derive(Default)]
pub struct Part {
    pub unit_to_metres: f32,
    pub objects: HashMap<u32, Object>,
    /// Property groups by id: each is a list of materials, indexed by a
    /// triangle's `p1` or an object's `pindex`.
    pub groups: HashMap<u32, Vec<Material>>,
    pub build: Vec<Component>,
}

fn attrs(e: &BytesStart) -> HashMap<String, String> {
    e.attributes()
        .flatten()
        .filter_map(|a| {
            let key = String::from_utf8_lossy(a.key.local_name().as_ref()).into_owned();
            let value = a.unescape_value().ok()?.into_owned();
            Some((key, value))
        })
        .collect()
}

fn unit_to_metres(unit: &str) -> f32 {
    match unit {
        "micron" => 1.0e-6,
        "centimeter" => 0.01,
        "inch" => 0.0254,
        "foot" => 0.3048,
        "meter" => 1.0,
        // The specification's default.
        _ => 0.001,
    }
}

/// `#RRGGBB` or `#RRGGBBAA` to 0..1 sRGB.
fn parse_color(text: &str) -> [f32; 4] {
    let hex = text.trim().trim_start_matches('#');
    let channel = |i: usize| {
        hex.get(i..i + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .map(|v| v as f32 / 255.0)
    };
    [
        channel(0).unwrap_or(0.8),
        channel(2).unwrap_or(0.8),
        channel(4).unwrap_or(0.8),
        channel(6).unwrap_or(1.0),
    ]
}

pub fn parse_part(xml: &str) -> Result<Part, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut part = Part { unit_to_metres: 0.001, ..Default::default() };

    let mut object: Option<(u32, Object, Option<(u32, u32)>)> = None;
    let mut group: Option<(u32, Vec<Material>)> = None;
    let mut in_build = false;

    loop {
        let event = reader.read_event().map_err(|e| format!("XML error: {e}"))?;
        let (e, empty) = match &event {
            Event::Start(e) => (Some(e.clone()), false),
            Event::Empty(e) => (Some(e.clone()), true),
            Event::End(end) => {
                match end.local_name().as_ref() {
                    b"object" => {
                        if let Some((id, obj, _)) = object.take() {
                            part.objects.insert(id, obj);
                        }
                    }
                    b"basematerials" | b"colorgroup" => {
                        if let Some((id, list)) = group.take() {
                            part.groups.insert(id, list);
                        }
                    }
                    b"build" => in_build = false,
                    _ => {}
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let Some(e) = e else { continue };
        let a = attrs(&e);
        let num = |key: &str| a.get(key).and_then(|v| v.parse::<u32>().ok());
        match e.local_name().as_ref() {
            b"model" => {
                part.unit_to_metres = unit_to_metres(a.get("unit").map(String::as_str).unwrap_or("millimeter"));
            }
            b"basematerials" | b"colorgroup" => {
                if let Some(id) = num("id") {
                    group = Some((id, Vec::new()));
                }
            }
            b"base" => {
                if let Some((id, list)) = group.as_mut() {
                    let name = a.get("name").cloned().unwrap_or_else(|| format!("Material {id}_{}", list.len()));
                    let color = a.get("displaycolor").map(|c| parse_color(c)).unwrap_or([0.8, 0.8, 0.8, 1.0]);
                    list.push(Material { name, color });
                }
            }
            b"color" => {
                if let Some((_, list)) = group.as_mut() {
                    let text = a.get("color").cloned().unwrap_or_default();
                    // Named for its colour; an opaque alpha says nothing, so it
                    // is left off (`Color 2A9D8F`, not `Color 2A9D8FFF`).
                    let hex = text.trim().trim_start_matches('#').to_ascii_uppercase();
                    let hex = hex.strip_suffix("FF").filter(|h| h.len() == 6).unwrap_or(&hex);
                    list.push(Material { name: format!("Color {hex}"), color: parse_color(&text) });
                }
            }
            b"object" => {
                let Some(id) = num("id") else { continue };
                let default = num("pid").map(|pid| (pid, num("pindex").unwrap_or(0)));
                let obj = Object {
                    name: a.get("name").cloned(),
                    kind: a.get("type").cloned().unwrap_or_else(|| "model".into()),
                    ..Default::default()
                };
                if empty {
                    part.objects.insert(id, obj);
                } else {
                    object = Some((id, obj, default));
                }
            }
            b"vertex" => {
                if let Some((_, obj, _)) = object.as_mut() {
                    let f = |k: &str| a.get(k).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
                    obj.vertices.push([f("x"), f("y"), f("z")]);
                }
            }
            b"triangle" => {
                if let Some((_, obj, default)) = object.as_mut() {
                    if let (Some(v1), Some(v2), Some(v3)) = (num("v1"), num("v2"), num("v3")) {
                        let material = match (num("pid"), num("p1")) {
                            (Some(pid), Some(p1)) => Some((pid, p1)),
                            (Some(pid), None) => Some((pid, default.map_or(0, |d| d.1))),
                            (None, Some(p1)) => default.map(|d| (d.0, p1)),
                            (None, None) => *default,
                        };
                        obj.triangles.push(([v1, v2, v3], material));
                    }
                }
            }
            b"component" => {
                if let (Some((_, obj, _)), Some(id)) = (object.as_mut(), num("objectid")) {
                    obj.components.push(Component {
                        path: a.get("path").cloned(),
                        object: id,
                        transform: a.get("transform").and_then(|t| Transform::parse(t)).unwrap_or(Transform::IDENTITY),
                    });
                }
            }
            b"build" => in_build = true,
            b"item" if in_build => {
                if let Some(id) = num("objectid") {
                    part.build.push(Component {
                        path: a.get("path").cloned(),
                        object: id,
                        transform: a.get("transform").and_then(|t| Transform::parse(t)).unwrap_or(Transform::IDENTITY),
                    });
                }
            }
            _ => {}
        }
    }
    Ok(part)
}

/// The package and every model part in it.
pub struct Package<R: Read + Seek> {
    zip: zip::ZipArchive<R>,
    pub parts: HashMap<String, Part>,
    pub root: String,
}

pub fn clean(path: &str) -> String {
    path.trim_start_matches('/').replace('\\', "/")
}

impl<R: Read + Seek> Package<R> {
    pub fn open(reader: R) -> Result<Self, String> {
        let mut zip = zip::ZipArchive::new(reader).map_err(|e| format!("not a 3MF package (zip): {e}"))?;
        // The root part, from the package relationships; the conventional name
        // when they are missing.
        let mut root = "3D/3dmodel.model".to_string();
        if let Ok(mut rels) = zip.by_name("_rels/.rels") {
            let mut text = String::new();
            if rels.read_to_string(&mut text).is_ok() {
                let mut reader = Reader::from_str(&text);
                while let Ok(event) = reader.read_event() {
                    match event {
                        Event::Empty(e) | Event::Start(e) if e.local_name().as_ref() == b"Relationship" => {
                            let a = attrs(&e);
                            if a.get("Type").is_some_and(|t| t.ends_with("/3dmodel")) {
                                if let Some(target) = a.get("Target") {
                                    root = clean(target);
                                }
                            }
                        }
                        Event::Eof => break,
                        _ => {}
                    }
                }
            }
        }
        Ok(Self { zip, parts: HashMap::new(), root })
    }

    /// Read every model part, so the object tree can be walked without going
    /// back to the archive. A slicer's package holds a handful of parts; the
    /// thumbnails and settings beside them are not read.
    pub fn load_all(&mut self) -> Result<(), String> {
        let names: Vec<String> = self
            .zip
            .file_names()
            .filter(|n| n.to_ascii_lowercase().ends_with(".model"))
            .map(str::to_string)
            .collect();
        for name in names {
            let mut text = String::new();
            self.zip
                .by_name(&name)
                .map_err(|e| format!("could not open {name} ({e})"))?
                .read_to_string(&mut text)
                .map_err(|e| format!("could not read {name} ({e})"))?;
            let part = parse_part(&text).map_err(|e| format!("{name}: {e}"))?;
            self.parts.insert(clean(&name), part);
        }
        if !self.parts.contains_key(&self.root) {
            return Err(format!("the package has no root model part ({})", self.root));
        }
        Ok(())
    }
}
