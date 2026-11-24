//! The Night Stars inspector.
//!
//! Was a separate `Editor`-scope crate so the lean runtime half carried no
//! editor contract; as one `Runtime` native plugin it is a module again.

use bevy::prelude::*;
use renzora::{AppEditorExt, InspectorEntry, NightStarsData};

fn inspector_entry() -> InspectorEntry {
    InspectorEntry {
        type_id: "night_stars",
        display_name: "Night Stars",
        icon: "moon-stars",
        category: "rendering",
        has_fn: |world, entity| world.get::<NightStarsData>(entity).is_some(),
        add_fn: Some(|world, entity| {
            world.entity_mut(entity).insert(NightStarsData::default());
        }),
        remove_fn: Some(|world, entity| {
            world.entity_mut(entity).remove::<NightStarsData>();
        }),
        is_enabled_fn: Some(|world, entity| {
            world
                .get::<NightStarsData>(entity)
                .map(|s| s.enabled)
                .unwrap_or(false)
        }),
        set_enabled_fn: Some(|world, entity, val| {
            if let Some(mut s) = world.get_mut::<NightStarsData>(entity) {
                s.enabled = val;
            }
        }),
        fields: vec![
            renzora::float_field!("Density", NightStarsData, density, 0.01, 0.0, 1.0),
            renzora::float_field!("Brightness", NightStarsData, brightness, 0.05, 0.0, 10.0),
            renzora::float_field!("Star Size", NightStarsData, star_size, 0.05, 0.2, 5.0),
            renzora::float_field!("Twinkle Speed", NightStarsData, twinkle_speed, 0.05, 0.0, 10.0),
            renzora::float_field!("Twinkle Amount", NightStarsData, twinkle_amount, 0.01, 0.0, 1.0),
            renzora::float_field!("Horizon Fade", NightStarsData, horizon_fade, 0.01, 0.0, 1.0),
            renzora::tuple_color_field!("Color", NightStarsData, color),
        ],
    }
}

/// Called by `renzora::plugin!`, which declares this `editor` module itself,
/// after the plugin's `build`. Not a
/// `cfg(feature = "editor")` gate: a native plugin compiles with no cargo
/// features, so that gate would be permanently false.
pub(crate) fn register(app: &mut App) {
    app.register_inspector(inspector_entry());
}
