//! The Vignette inspector entry.
//!
//! `renzora::plugin!` declares this `editor` module itself (the build finds it
//! from `src/editor.rs`) and compiles it only in the editor, so a lean game
//! build carries none of it. See the module doc in `lib.rs` for why this is not a
//! `cfg(feature = "editor")` gate.

use bevy::post_process::effect_stack::Vignette;
use bevy::prelude::*;
use renzora::{AppEditorExt, InspectorEntry};

use crate::VignetteSettings;

fn inspector_entry() -> InspectorEntry {
    InspectorEntry {
        type_id: "vignette",
        display_name: "Vignette",
        icon: "aperture",
        category: "post_process",
        has_fn: |world, entity| world.get::<VignetteSettings>(entity).is_some(),
        add_fn: Some(|world, entity| {
            world.entity_mut(entity).insert(VignetteSettings::default());
        }),
        remove_fn: Some(|world, entity| {
            world
                .entity_mut(entity)
                .remove::<(VignetteSettings, Vignette)>();
        }),
        is_enabled_fn: Some(|world, entity| {
            world
                .get::<VignetteSettings>(entity)
                .map(|s| s.enabled)
                .unwrap_or(false)
        }),
        set_enabled_fn: Some(|world, entity, val| {
            if let Some(mut s) = world.get_mut::<VignetteSettings>(entity) {
                s.enabled = val;
            }
        }),
        fields: vec![
            renzora::float_field!("Intensity", VignetteSettings, intensity, 0.01, 0.0, 5.0),
            renzora::float_field!("Radius", VignetteSettings, radius, 0.01, 0.0, 2.0),
            renzora::float_field!("Smoothness", VignetteSettings, smoothness, 0.05, 0.0, 20.0),
            renzora::float_field!("Roundness", VignetteSettings, roundness, 0.01, 0.0, 1.0),
            renzora::vec3_color_field!("Color", VignetteSettings, color),
            renzora::float_field!(
                "Edge Compensation",
                VignetteSettings,
                edge_compensation,
                0.01,
                0.0,
                2.0
            ),
        ],
    }
}

pub(crate) fn register(app: &mut App) {
    app.register_inspector(inspector_entry());
}
