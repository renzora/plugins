//! The inspector entry for [`LensDistortionSettings`].
//!
//! A module rather than the separate `renzora_lens_distortion_editor` crate it
//! was: that split existed so the runtime half could compile without
//! `renzora/editor`, which a plugin never needs to do.

use bevy::post_process::effect_stack::LensDistortion;
use bevy::prelude::*;
use renzora::{AppEditorExt, FieldDef, FieldType, FieldValue, InspectorEntry};

use crate::LensDistortionSettings;

/// `FieldType` has no `Vec2`, so expose each `Vec2` component as its own float
/// field (`.x` / `.y`). Used for `multiplier` and `center`.
macro_rules! vec2_axis_field {
    ($name:expr, $field:ident, $axis:ident, $speed:expr, $min:expr, $max:expr) => {
        FieldDef {
            name: $name,
            field_type: FieldType::Float { speed: $speed, min: $min, max: $max },
            get_fn: |w, e| {
                w.get::<LensDistortionSettings>(e)
                    .map(|s| FieldValue::Float(s.$field.$axis))
            },
            set_fn: |w, e, v| {
                if let (FieldValue::Float(f), Some(mut s)) =
                    (v, w.get_mut::<LensDistortionSettings>(e))
                {
                    s.$field.$axis = f;
                }
            },
        }
    };
}

fn lens_distortion_entry() -> InspectorEntry {
    InspectorEntry {
        type_id: "lens_distortion",
        display_name: "Lens Distortion",
        icon: "circle",
        category: "effects",
        has_fn: |world, entity| world.get::<LensDistortionSettings>(entity).is_some(),
        add_fn: Some(|world, entity| {
            world
                .entity_mut(entity)
                .insert(LensDistortionSettings::default());
        }),
        remove_fn: Some(|world, entity| {
            world
                .entity_mut(entity)
                .remove::<(LensDistortionSettings, LensDistortion)>();
        }),
        is_enabled_fn: Some(|world, entity| {
            world
                .get::<LensDistortionSettings>(entity)
                .map(|s| s.enabled)
                .unwrap_or(false)
        }),
        set_enabled_fn: Some(|world, entity, val| {
            if let Some(mut s) = world.get_mut::<LensDistortionSettings>(entity) {
                s.enabled = val;
            }
        }),
        fields: vec![
            // Positive = barrel, negative = pincushion.
            renzora::float_field!("Intensity", LensDistortionSettings, intensity, 0.01, -1.0, 1.0),
            renzora::float_field!("Scale", LensDistortionSettings, scale, 0.01, 0.5, 2.0),
            vec2_axis_field!("Multiplier X", multiplier, x, 0.01, 0.0, 2.0),
            vec2_axis_field!("Multiplier Y", multiplier, y, 0.01, 0.0, 2.0),
            vec2_axis_field!("Center X", center, x, 0.005, 0.0, 1.0),
            vec2_axis_field!("Center Y", center, y, 0.005, 0.0, 1.0),
            renzora::float_field!(
                "Edge Curvature",
                LensDistortionSettings,
                edge_curvature,
                0.01,
                -1.0,
                1.0
            ),
        ],
    }
}

pub fn register(app: &mut App) {
    app.register_inspector(lens_distortion_entry());
}
