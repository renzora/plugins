//! The editor half: the 3D Text inspector and its Add Entity preset.
//!
//! `renzora::plugin!` declares this `editor` module itself (the build finds it
//! from `src/editor.rs`) and compiles it only in the editor, so a lean game
//! build carries none of it. Not a `cfg(feature = "editor")` gate: a native plugin is
//! compiled by a bare `rustc` with no cargo features, so that gate would be
//! permanently false and the inspector and preset would vanish without a word.

use bevy::prelude::*;
use renzora::{AppEditorExt, EntityPreset, FieldDef, FieldType, FieldValue, InspectorEntry};

use crate::Text3d;

pub(crate) fn register(app: &mut App) {
    // Custom inspector: the font is an Asset field (drag-drop a `.ttf`/
    // `.otf` onto it to swap fonts), and Mode is a flat/mesh dropdown.
    app.register_inspector(InspectorEntry {
        type_id: "text3d",
        display_name: "3D Text",
        icon: "text-t",
        category: "basic",
        has_fn: |w, e| w.get::<Text3d>(e).is_some(),
        add_fn: Some(|w, e| {
            w.entity_mut(e).insert(Text3d::default());
        }),
        remove_fn: Some(|w, e| {
            w.entity_mut(e).remove::<Text3d>();
        }),
        is_enabled_fn: None,
        set_enabled_fn: None,
        fields: vec![
            renzora::string_field!("Text", Text3d, text),
            FieldDef {
                name: "Font",
                field_type: FieldType::Asset {
                    extensions: vec!["ttf".into(), "otf".into()],
                },
                get_fn: |w, e| {
                    w.get::<Text3d>(e).map(|t| {
                        FieldValue::Asset((!t.font.is_empty()).then(|| t.font.clone()))
                    })
                },
                set_fn: |w, e, v| {
                    if let (FieldValue::Asset(p), Some(mut t)) = (v, w.get_mut::<Text3d>(e)) {
                        t.font = p.unwrap_or_default();
                    }
                },
            },
            FieldDef {
                name: "Mode",
                field_type: FieldType::Enum {
                    options: &["flat", "mesh"],
                },
                get_fn: |w, e| w.get::<Text3d>(e).map(|t| FieldValue::Enum(t.mode.clone())),
                set_fn: |w, e, v| {
                    if let (FieldValue::Enum(s), Some(mut t)) = (v, w.get_mut::<Text3d>(e)) {
                        t.mode = s;
                    }
                },
            },
            renzora::float_field!("Size", Text3d, size, 1.0, 1.0, 2000.0),
            renzora::float_field!("Depth", Text3d, depth, 0.01, 0.0, 5.0),
            FieldDef {
                name: "Color",
                field_type: FieldType::Color,
                get_fn: |w, e| w.get::<Text3d>(e).map(|t| FieldValue::Color(t.color)),
                set_fn: |w, e, v| {
                    if let (FieldValue::Color(c), Some(mut t)) = (v, w.get_mut::<Text3d>(e)) {
                        t.color = c;
                    }
                },
            },
        ],
    });

    // "+ Add Entity" → "3D Text": spawn a ready-to-see text object.
    app.register_entity_preset(EntityPreset {
        id: "text3d",
        display_name: "3D Text",
        icon: "text-t",
        category: "basic",
        spawn_fn: |world| {
            world
                .spawn((
                    Name::new("3D Text"),
                    Text3d::default(),
                    Transform::default(),
                    Visibility::default(),
                ))
                .id()
        },
    });
}
