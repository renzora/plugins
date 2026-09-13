//! The Poly Haven public API: what to ask for, what comes back, and which of it
//! is worth downloading.
//!
//! Four endpoints do the whole job, none of them authenticated:
//!
//! * `/types` — the three asset kinds. Hardcoded as [`Kind`]: it has not changed
//!   since the API opened, and a plugin that fetched it would have to decide what
//!   to do with a fourth kind it has no UI for anyway.
//! * `/assets?t=<kind>` — the entire catalogue for one kind as a single JSON
//!   object keyed by slug. ~500 models, ~1000 textures, ~800 HDRIs.
//! * `/files/<slug>` — every downloadable file for one asset, as a tree of
//!   `slot → resolution → format → { url, size, md5 }`.
//! * `cdn.polyhaven.com/asset_img/thumbs/<slug>.png` — the preview image. A URL
//!   convention rather than an endpoint; it is not in the API response.
//!
//! Everything is CC0, which is why there is no licence plumbing here and no
//! attribution the importer has to carry into the project.
//!
//! # Why the catalogue is fetched whole
//!
//! There is no search endpoint and no pagination — `/assets` returns everything
//! or nothing. That is fine at this size (about 1.5 MB of JSON for the largest
//! kind) as long as it happens once: see [`crate::hub::cache_path`] for the
//! on-disk cache that makes reopening the panel instant.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The API root. No key, no auth, no published rate limit.
pub const API: &str = "https://api.polyhaven.com";

/// Preview images. `?width=` is honoured by the CDN and worth passing: the
/// unsized image is ~1 MB, and a 256 px tile does not need it.
pub const THUMBS: &str = "https://cdn.polyhaven.com/asset_img/thumbs";

/// Sent on every request.
///
/// Not politeness — a requirement. The API answers `403 Forbidden` to a client
/// that sends no `User-Agent` at all (measured against Python's urllib, which
/// omits one), so a backend that happened not to set a default would fail every
/// request here with a status that looks like a permissions problem.
///
/// A literal rather than `env!("CARGO_PKG_VERSION")`: a native plugin is compiled
/// by a bare `rustc` that cargo never invoked, and the only two cargo variables
/// set for it are `CARGO_MANIFEST_DIR` and `CARGO_PKG_NAME`
/// (`renzora_native_build::rustc::env_vars`). Any other `env!` is a compile
/// error at this crate's very first line.
pub const USER_AGENT: &str = "renzora-polyhaven/0.1";

/// Which of the three catalogues an asset belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Hdris,
    Textures,
    Models,
}

impl Kind {
    /// In tab order, and in the order the panel's `[Catalogue; 3]` is indexed.
    pub const ALL: [Kind; 3] = [Kind::Hdris, Kind::Textures, Kind::Models];

    /// The `t=` query value.
    pub fn slug(self) -> &'static str {
        match self {
            Kind::Hdris => "hdris",
            Kind::Textures => "textures",
            Kind::Models => "models",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Hdris => "HDRIs",
            Kind::Textures => "Textures",
            Kind::Models => "Models",
        }
    }

    /// Phosphor icon for the tab.
    pub fn icon(self) -> &'static str {
        match self {
            Kind::Hdris => "sun",
            Kind::Textures => "grid-nine",
            Kind::Models => "cube",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Kind::Hdris => 0,
            Kind::Textures => 1,
            Kind::Models => 2,
        }
    }

    /// Where this kind's files land, under the project's `assets/`.
    pub fn folder(self) -> &'static str {
        match self {
            Kind::Hdris => "hdris",
            Kind::Textures => "textures",
            Kind::Models => "models",
        }
    }
}

/// One catalogue entry.
///
/// Deliberately tolerant: every field but `name` is optional or defaulted,
/// because the three kinds do not share a schema (a model has `polycount`, an
/// HDRI has `whitebalance`, a texture has neither) and serde's default is to
/// ignore unknown fields rather than to require known ones. A schema addition on
/// their side should not stop the panel listing anything.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Asset {
    /// The object key the entry arrived under, filled in by [`parse_catalogue`].
    /// It is the id every other endpoint takes, and it is not inside the value.
    #[serde(skip)]
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Name → credited role. Only the names are shown.
    #[serde(default)]
    pub authors: BTreeMap<String, String>,
    /// Unix seconds. Drives the default ordering, newest first.
    #[serde(default)]
    pub date_published: i64,
    /// Present on models only.
    #[serde(default)]
    pub polycount: Option<u64>,
}

impl Asset {
    /// `"Kirill Sannikov"`, or several joined with a comma.
    pub fn credit(&self) -> String {
        self.authors.keys().cloned().collect::<Vec<_>>().join(", ")
    }

    /// The one number worth putting on a tile, when the kind has one.
    ///
    /// Only models publish `polycount`, and for a model it is the figure that
    /// decides whether the asset belongs in the scene at all — a 200k-triangle
    /// chair is a different proposition from a 5k one, and finding that out
    /// after the download is the wrong order.
    pub fn detail(&self) -> Option<String> {
        let count = self.polycount?;
        Some(if count >= 1_000 {
            format!("{:.1}k tris", count as f64 / 1_000.0)
        } else {
            format!("{count} tris")
        })
    }

    /// Does every whitespace-separated term in `query` appear in this entry?
    ///
    /// All terms must match (narrowing as you type, which is what a search box
    /// is expected to do), but each may match the name, the slug, a tag or a
    /// category — so "wood chair" finds a chair tagged wood.
    pub fn matches(&self, terms: &[String]) -> bool {
        terms.iter().all(|term| {
            self.name.to_lowercase().contains(term)
                || self.slug.contains(term)
                || self.tags.iter().any(|t| t.to_lowercase().contains(term))
                || self
                    .categories
                    .iter()
                    .any(|c| c.to_lowercase().contains(term))
        })
    }
}

/// `/assets?t=<kind>`.
pub fn catalogue_url(kind: Kind) -> String {
    format!("{API}/assets?t={}", kind.slug())
}

/// `/files/<slug>`.
pub fn files_url(slug: &str) -> String {
    format!("{API}/files/{slug}")
}

/// The preview image for `slug`, at tile size.
pub fn thumb_url(slug: &str) -> String {
    format!("{THUMBS}/{slug}.png?width=256")
}

/// Parse `/assets` into a list ordered newest first.
///
/// The response is a JSON *object*, so it carries no order of its own — whatever
/// the server wrote is lost to any map that parses it. Sorting here rather than
/// leaving it to chance means the grid opens on the newest uploads, which is the
/// useful default for browsing a library you have already seen most of.
pub fn parse_catalogue(text: &str) -> Result<Vec<Asset>, String> {
    let map: BTreeMap<String, Asset> =
        serde_json::from_str(text).map_err(|e| format!("catalogue is not valid JSON: {e}"))?;
    let mut out: Vec<Asset> = map
        .into_iter()
        .map(|(slug, mut asset)| {
            asset.slug = slug;
            asset
        })
        .collect();
    out.sort_by(|a, b| b.date_published.cmp(&a.date_published).then(a.name.cmp(&b.name)));
    Ok(out)
}

/// One file to fetch, and where it goes relative to the asset's own folder.
#[derive(Clone, Debug)]
pub struct FilePlan {
    pub url: String,
    /// Already validated by [`safe_rel`] — never absolute, never escaping.
    pub rel: String,
}

/// The texture maps worth taking, in the order they are looked for.
///
/// Not every slot every asset offers: a texture publishes up to eleven, several
/// of which are alternates of each other (`nor_dx` vs `nor_gl` are the same map
/// for different handedness conventions, `arm` packs AO/Rough/Metal into one
/// image that `AO` + `Rough` + `Metal` also provide separately). Downloading all
/// of them would trade five files for eleven and leave the user to work out
/// which three the engine wants.
///
/// `nor_gl` because that is the OpenGL/glTF green-channel convention, which is
/// what Bevy's `StandardMaterial` expects; feeding it `nor_dx` inverts every
/// normal's Y and lights the surface from the wrong side.
const TEXTURE_SLOTS: [&str; 5] = ["Diffuse", "nor_gl", "Rough", "Metal", "Displacement"];

/// Resolutions to fall back through when the requested one is not published.
///
/// Assets do not all offer the same set — HDRIs go to 24k, some textures stop at
/// 4k, a few models publish only 2k — so a plan that insisted on the exact
/// string would fail on assets that have a perfectly good neighbouring size.
const RES_FALLBACK: [&str; 6] = ["1k", "2k", "4k", "8k", "16k", "24k"];

/// Work out which files to fetch for one asset at one resolution.
///
/// Takes the parsed `/files/<slug>` response. The three kinds are shaped
/// differently enough that there is no common walk: an HDRI is one file, a
/// texture is a handful of independent images, and a model is a glTF plus an
/// `include` map of the buffers and textures it references by relative path.
pub fn plan(kind: Kind, files: &serde_json::Value, res: &str) -> Result<Vec<FilePlan>, String> {
    match kind {
        Kind::Hdris => plan_hdri(files, res),
        Kind::Textures => plan_texture(files, res),
        Kind::Models => plan_model(files, res),
    }
}

/// `hdri.<res>.{hdr,exr}` — exactly one file.
///
/// `.hdr` first: Bevy decodes Radiance HDR in its default feature set, while
/// OpenEXR is behind a feature an export may not have compiled. The `.exr` is
/// higher fidelity and is taken when a resolution publishes no `.hdr`.
fn plan_hdri(files: &serde_json::Value, res: &str) -> Result<Vec<FilePlan>, String> {
    let slot = files
        .get("hdri")
        .ok_or_else(|| "no `hdri` files for this asset".to_string())?;
    let bucket = resolve_res(slot, res).ok_or_else(|| "no resolutions published".to_string())?;
    for format in ["hdr", "exr"] {
        if let Some(url) = leaf_url(bucket.get(format)) {
            let rel = filename_of(&url)?;
            return Ok(vec![FilePlan { url, rel }]);
        }
    }
    Err("no `hdr` or `exr` at any resolution".into())
}

/// The maps in [`TEXTURE_SLOTS`] that this texture actually publishes.
///
/// Missing slots are skipped rather than failed: a texture with no metalness map
/// is an ordinary dielectric, not a broken asset.
fn plan_texture(files: &serde_json::Value, res: &str) -> Result<Vec<FilePlan>, String> {
    let mut out = Vec::new();
    for slot in TEXTURE_SLOTS {
        let Some(bucket) = files.get(slot).and_then(|s| resolve_res(s, res)) else {
            continue;
        };
        // JPEG first: a 4k PNG of the same map is several times the size for a
        // difference no PBR input shows. PNG is the fallback for the few maps
        // published without one.
        for format in ["jpg", "png"] {
            if let Some(url) = leaf_url(bucket.get(format)) {
                let rel = filename_of(&url)?;
                out.push(FilePlan { url, rel });
                break;
            }
        }
    }
    if out.is_empty() {
        return Err("no downloadable maps at any resolution".into());
    }
    Ok(out)
}

/// `gltf.<res>.gltf` plus everything in its `include` map.
///
/// The `include` keys are the paths the glTF references — `ArmChair_01.bin`,
/// `textures/Armchair_01_diff_1k.jpg` — so they are also the paths the files
/// must be written to. Getting one wrong does not fail the download; it produces
/// a glTF that loads and renders untextured, which is worse.
fn plan_model(files: &serde_json::Value, res: &str) -> Result<Vec<FilePlan>, String> {
    let slot = files
        .get("gltf")
        .ok_or_else(|| "no `gltf` files for this asset".to_string())?;
    let bucket = resolve_res(slot, res).ok_or_else(|| "no resolutions published".to_string())?;
    let leaf = bucket
        .get("gltf")
        .ok_or_else(|| "resolution publishes no glTF".to_string())?;
    let url = leaf_url(Some(leaf)).ok_or_else(|| "glTF entry has no url".to_string())?;

    let mut out = vec![FilePlan {
        rel: filename_of(&url)?,
        url,
    }];
    if let Some(include) = leaf.get("include").and_then(|i| i.as_object()) {
        for (rel, entry) in include {
            let Some(url) = leaf_url(Some(entry)) else {
                continue;
            };
            // A path chosen by a remote server, about to be joined onto a
            // directory inside the user's project. `safe_rel` is what stops a
            // crafted `include` key from writing anywhere else.
            let Some(rel) = safe_rel(rel) else {
                return Err(format!("refusing unsafe path in glTF include: {rel}"));
            };
            out.push(FilePlan { url, rel });
        }
    }
    Ok(out)
}

/// The resolution bucket for `res`, or the nearest published one.
fn resolve_res<'a>(slot: &'a serde_json::Value, res: &str) -> Option<&'a serde_json::Value> {
    if let Some(exact) = slot.get(res) {
        return Some(exact);
    }
    RES_FALLBACK.iter().find_map(|r| slot.get(r))
}

/// The `url` of a `{ url, size, md5 }` leaf.
fn leaf_url(leaf: Option<&serde_json::Value>) -> Option<String> {
    leaf?.get("url")?.as_str().map(|s| s.to_string())
}

/// The last path segment of a URL, percent-decoded, as a safe relative path.
fn filename_of(url: &str) -> Result<String, String> {
    let last = url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("no filename in url: {url}"))?;
    let decoded = percent_decode(last.split(['?', '#']).next().unwrap_or(last));
    safe_rel(&decoded).ok_or_else(|| format!("unsafe filename in url: {url}"))
}

/// Decode `%XX` escapes. Poly Haven's extras live under paths like
/// `Color%20Charts/`, and a filename written with the escape still in it is a
/// different name from the one the glTF references.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Accept a relative path that stays inside the folder it will be joined to.
///
/// The whole reason this exists: both the filename and the glTF `include` keys
/// are strings chosen by a remote server and then used as filesystem paths. A
/// key of `../../../../.ssh/authorized_keys` is a valid JSON object key, and
/// `Path::join` would follow it out of the project without complaint.
///
/// Rejects an absolute path, a Windows drive prefix, any `..` component, and any
/// component that is empty or a bare `.` — the last two because `a//b` and `a/./b`
/// normalise to paths that look different from what was checked.
fn safe_rel(rel: &str) -> Option<String> {
    let rel = rel.replace('\\', "/");
    if rel.is_empty() || rel.starts_with('/') || rel.contains(':') {
        return None;
    }
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return None;
        }
    }
    Some(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_paths_are_refused() {
        assert!(safe_rel("textures/diff.jpg").is_some());
        assert!(safe_rel("../escape.bin").is_none());
        assert!(safe_rel("textures/../../escape.bin").is_none());
        assert!(safe_rel("/etc/passwd").is_none());
        assert!(safe_rel("C:/windows/system32").is_none());
        assert!(safe_rel("a//b").is_none());
        // A backslash is normalised first, so it cannot smuggle a component past
        // the `..` check on a platform that treats it as a separator.
        assert!(safe_rel("..\\escape").is_none());
    }

    #[test]
    fn percent_escapes_decode() {
        assert_eq!(percent_decode("Color%20Charts"), "Color Charts");
        assert_eq!(percent_decode("plain.hdr"), "plain.hdr");
        // A truncated escape is left alone rather than dropped.
        assert_eq!(percent_decode("odd%2"), "odd%2");
    }

    #[test]
    fn filename_strips_query_and_decodes() {
        let url = "https://dl.polyhaven.org/file/ph-assets/HDRIs/hdr/1k/road%201k.hdr?x=1";
        assert_eq!(filename_of(url).unwrap(), "road 1k.hdr");
    }

    #[test]
    fn a_missing_resolution_falls_back() {
        let slot = serde_json::json!({ "2k": { "hdr": { "url": "https://x/a_2k.hdr" } } });
        let files = serde_json::json!({ "hdri": slot });
        let plan = plan_hdri(&files, "16k").unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].rel, "a_2k.hdr");
    }

    #[test]
    fn a_model_takes_its_includes_at_their_own_paths() {
        let files = serde_json::json!({
            "gltf": { "1k": { "gltf": {
                "url": "https://x/Chair_1k.gltf",
                "include": {
                    "Chair.bin": { "url": "https://x/Chair.bin" },
                    "textures/diff.jpg": { "url": "https://x/diff.jpg" }
                }
            }}}
        });
        let mut plan = plan_model(&files, "1k").unwrap();
        plan.sort_by(|a, b| a.rel.cmp(&b.rel));
        let rels: Vec<&str> = plan.iter().map(|p| p.rel.as_str()).collect();
        assert_eq!(rels, ["Chair.bin", "Chair_1k.gltf", "textures/diff.jpg"]);
    }

    #[test]
    fn a_texture_skips_slots_it_does_not_publish() {
        let map = serde_json::json!({ "1k": { "jpg": { "url": "https://x/m.jpg" } } });
        let files = serde_json::json!({ "Diffuse": map.clone(), "Rough": map });
        let plan = plan_texture(&files, "1k").unwrap();
        assert_eq!(plan.len(), 2, "Metal and Displacement are absent, not fatal");
    }

    #[test]
    fn search_terms_all_have_to_match() {
        let asset = Asset {
            slug: "arm_chair_01".into(),
            name: "Arm Chair 01".into(),
            tags: vec!["wood".into(), "vintage".into()],
            categories: vec!["furniture".into()],
            ..Default::default()
        };
        assert!(asset.matches(&["chair".into()]));
        assert!(asset.matches(&["wood".into(), "chair".into()]));
        assert!(asset.matches(&["furniture".into()]));
        assert!(!asset.matches(&["chair".into(), "metal".into()]));
    }
}
