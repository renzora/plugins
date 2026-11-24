//! 3D text: render a string as glyph geometry in world space.
//!
//! The engine has no real 3D-text entity — only the stroke-font debug gizmo. A
//! [`Text3d`] entity fills that gap in two modes:
//!
//! * **flat** — rasterize the font through Bevy's atlas, pack it into a signed
//!   distance field, and emit one textured quad per glyph. Cheap and crisp at
//!   any distance, but a flat card.
//! * **mesh** — read the glyph outlines from the font file with `ttf-parser` and
//!   triangulate the fill (holes and all) with `lyon`, optionally extruded. Real
//!   3D letters that catch light.
//!
//! # What changed when this became a native plugin
//!
//! There used to be two of these and neither was whole. `crates/renzora_text3d`
//! was statically linked and had both modes; `plugins/text3d` was a C-ABI
//! cdylib that had **only mesh mode**, with its own `Text3d` component and a
//! fixed cap on how many could exist at once. The C-ABI boundary is why: flat
//! mode needs `SdfTextMaterial` — a real Bevy `Material` — and a function table
//! cannot carry one.
//!
//! A native plugin can, because it links the real Bevy. So this is the union of
//! the two, and both predecessors are gone. Its one requirement was that
//! `SdfTextMaterial` be a single type shared with `renzora::ui`'s world-space
//! UI emitter, which is why the glyph machinery moved into the contract crate as
//! [`renzora::text_mesh`] — see that module's doc.

use bevy::prelude::*;

pub mod outline;
pub mod systems;

// Re-exported so callers can reach the builder through this plugin, as they
// could when it was a crate.
pub use renzora::text_mesh::build_text_mesh;

/// Embedded fallback font (Noto Sans), used when a `Text3d` leaves its `font`
/// field empty — so BOTH flat and mesh modes render out of the box. Mesh mode
/// especially needs real outline bytes, which the OS-family fallback can't give.
/// Under `src/` rather than beside the manifest because the native-plugin
/// stager mirrors only `Cargo.toml` and `src/` into the build directory — a font
/// at the plugin root would compile here and vanish there.
pub(crate) const DEFAULT_FONT: &[u8] = include_bytes!("embedded/NotoSans-Regular.ttf");

/// A `Handle` to [`DEFAULT_FONT`] registered as a `Font` asset, so flat mode can
/// reference the same default font mesh mode reads bytes from.
#[derive(Resource)]
pub(crate) struct DefaultFont(pub Handle<bevy::text::Font>);

fn register_default_font(mut commands: Commands, mut fonts: ResMut<Assets<bevy::text::Font>>) {
    let handle = fonts.add(bevy::text::Font::from_bytes(DEFAULT_FONT.to_vec()));
    commands.insert_resource(DefaultFont(handle));
}

/// A string rendered as a 3D quad mesh at this entity's `Transform`.
///
/// `font` is an asset-relative path to a `.ttf`/`.otf` (e.g. `"fonts/Roboto.ttf"`),
/// or empty for the OS sans-serif. `size` is the rasterization size in pixels;
/// the resulting mesh is scaled to world units ([`renzora::text_mesh::WORLD_UNITS_PER_PX`]) and
/// centred on the origin, so the entity's `Transform` scale/rotation place it.
///
/// Reflect (not serde) backs scene save/load, matching the other distribution
/// plugins; the `Inspectable` derive (editor-only) renders its fields — `color`
/// is `[f32; 3]` so it maps to the inspector's colour picker.
#[derive(Component, Reflect, Clone)]
#[reflect(Component, Default)]
pub struct Text3d {
    /// The text to display.
    pub text: String,
    /// Asset-relative font path, or empty for the OS sans-serif (flat mode only —
    /// mesh mode needs a real font file to read outlines from).
    pub font: String,
    /// `"flat"` = crisp SDF-textured quads (cheap, great for UI, but a flat card);
    /// `"mesh"` = true triangulated glyph geometry, extrudable and lit.
    pub mode: String,
    /// Glyph rasterization / em size in pixels.
    pub size: f32,
    /// Extrusion depth in world units — mesh mode only; 0 = a flat filled outline.
    pub depth: f32,
    /// sRGB text colour, components 0..1.
    pub color: [f32; 3],
}

impl Default for Text3d {
    fn default() -> Self {
        Self {
            text: "3D Text".into(),
            font: String::new(),
            // Flat by default so a fresh entity renders immediately (mesh mode
            // needs a font dropped on it first).
            mode: "flat".into(),
            size: 100.0,
            depth: 0.1,
            color: [1.0, 1.0, 1.0],
        }
    }
}

#[derive(Default)]
pub struct Text3dPlugin;

impl Plugin for Text3dPlugin {
    fn build(&self, app: &mut App) {
        info!("[text3d] native plugin");

        renzora::text_mesh::ensure_sdf_material(app);
        app.add_systems(Startup, register_default_font);

        // Rebuild the quad mesh whenever the text/font/size/colour changes (and
        // retry while a custom font is still loading); tear it down on removal.
        app.add_systems(
            Update,
            (systems::rebuild_text3d, systems::cleanup_text3d),
        );

        // Always registered — scene save/load and reflection need the type.
        app.register_type::<Text3d>();
    }
}

// `Runtime`, explicitly. `plugin!` defaults to `Editor` where `add!` defaulted
// to `Runtime`, and 3D text is scene content a shipped game renders. The
// inspector and preset live in `src/editor.rs`: `plugin!` declares the `editor`
// module itself (the build finds it from `src/editor.rs`), compiling it in the
// editor and leaving it out of a lean game build.
renzora::plugin!(Text3dPlugin, Runtime);
