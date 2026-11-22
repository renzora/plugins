//! The Weather inspector section.
//!
//! A module of the runtime plugin rather than a second plugin, the way `wind`
//! does it: a native plugin has no cargo features, and the inspector registry
//! is simply unread in a shipped game.

use bevy::prelude::*;
use renzora::{AppEditorExt, FieldDef, FieldType, FieldValue, InspectorEntry};

use crate::data::{Weather, WeatherPreset};

macro_rules! float_field {
    ($name:literal, $field:ident, $speed:expr, $min:expr, $max:expr) => {
        FieldDef {
            name: $name,
            field_type: FieldType::Float {
                speed: $speed,
                min: $min,
                max: $max,
            },
            get_fn: |world, entity| world.get::<Weather>(entity).map(|w| FieldValue::Float(w.$field)),
            set_fn: |world, entity, val| {
                if let (FieldValue::Float(v), Some(mut w)) = (val, world.get_mut::<Weather>(entity)) {
                    w.$field = v;
                }
            },
        }
    };
}

macro_rules! bool_field {
    ($name:literal, $field:ident) => {
        FieldDef {
            name: $name,
            field_type: FieldType::Bool,
            get_fn: |world, entity| world.get::<Weather>(entity).map(|w| FieldValue::Bool(w.$field)),
            set_fn: |world, entity, val| {
                if let (FieldValue::Bool(v), Some(mut w)) = (val, world.get_mut::<Weather>(entity)) {
                    w.$field = v;
                }
            },
        }
    };
}

fn preset_field() -> FieldDef {
    FieldDef {
        name: "Weather",
        field_type: FieldType::Enum {
            options: &WeatherPreset::LABELS,
        },
        get_fn: |world, entity| {
            world
                .get::<Weather>(entity)
                .map(|w| FieldValue::Enum(w.preset.label().to_string()))
        },
        set_fn: |world, entity, val| {
            let FieldValue::Enum(label) = val else { return };
            let Some(preset) = WeatherPreset::ALL.into_iter().find(|p| p.label() == label) else {
                return;
            };
            if let Some(mut w) = world.get_mut::<Weather>(entity) {
                w.preset = preset;
            }
        },
    }
}

fn inspector_entry() -> InspectorEntry {
    InspectorEntry {
        type_id: "weather",
        display_name: "Weather",
        icon: "cloud-rain",
        category: "rendering",
        has_fn: |world, entity| world.get::<Weather>(entity).is_some(),
        add_fn: Some(|world, entity| {
            world.entity_mut(entity).insert(Weather::default());
        }),
        remove_fn: Some(|world, entity| {
            world.entity_mut(entity).remove::<Weather>();
        }),
        is_enabled_fn: Some(|world, entity| world.get::<Weather>(entity).is_some_and(|w| w.enabled)),
        set_enabled_fn: Some(|world, entity, val| {
            if let Some(mut w) = world.get_mut::<Weather>(entity) {
                w.enabled = val;
            }
        }),
        fields: vec![
            preset_field(),
            float_field!("Transition (s)", transition_seconds, 1.0, 0.0, 1800.0),
            float_field!("Intensity", intensity, 0.01, 0.0, 2.0),
            FieldDef::toggle_group(
                bool_field!("Cycle", cycle),
                vec![
                    float_field!("Hold Min (s)", cycle_min_seconds, 5.0, 10.0, 7200.0),
                    float_field!("Hold Max (s)", cycle_max_seconds, 5.0, 10.0, 7200.0),
                ],
            ),
            FieldDef::group(
                "Effects",
                vec![bool_field!("Rain", rain_particles), bool_field!("Lightning", lightning)],
            ),
        ],
    }
}

pub fn register(app: &mut App) {
    app.register_inspector(inspector_entry());
}
