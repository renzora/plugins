//! The Weather inspector section.
//!
//! Drawn natively rather than from declarative fields, because what is worth
//! showing depends on the weather: rain settings while it rains, lightning in a
//! storm, snow while it snows or while snow still lies. Settings for weather
//! that is not happening are hidden, not greyed: a Storm section offering
//! "Time to Cover" for snow is noise.
//!
//! A module of the runtime plugin rather than a second plugin, the way `wind`
//! does it: a native plugin has no cargo features, and the inspector registry
//! is simply unread in a shipped game.

use bevy::prelude::*;
use renzora::{AppEditorExt, InspectorEntry};
use renzora::ui::font::EmberFonts;
use renzora::ui::inspector::{inspector_body, inspector_row};
use renzora::ui::reactive::tracked::{bind_2way, bind_display};
use renzora::ui::reactive::Rx;
use renzora::ui::widgets::{collapsible, drag_value, dropdown, toggle_switch, DragRange};

use crate::data::{Weather, WeatherPreset, WeatherState};

fn weather<'w>(w: &Rx<'w>, entity: Entity) -> Option<&'w Weather> {
    w.get::<Weather>(entity)
}

/// Change the weather through `edit`, only if it actually changes something,
/// so an untouched control never marks the scene modified.
fn edit(world: &mut World, entity: Entity, edit: impl FnOnce(&mut Weather) -> bool) {
    if let Some(mut w) = world.get_mut::<Weather>(entity) {
        let mut copy = w.clone();
        if edit(&mut copy) {
            *w = copy;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn number(
    commands: &mut Commands,
    fonts: &EmberFonts,
    entity: Entity,
    label: &str,
    init: f32,
    step: f32,
    range: (f32, f32),
    get: fn(&Weather) -> f32,
    set: fn(&mut Weather, f32),
) -> Entity {
    let dv = drag_value(commands, &fonts.ui, "", (210, 210, 220), init, step);
    commands.entity(dv).insert(DragRange { min: range.0, max: range.1 });
    bind_2way(
        commands,
        dv,
        move |w| weather(w, entity).map(get).unwrap_or(0.0),
        move |world, v: &f32| {
            let v = v.clamp(range.0, range.1);
            edit(world, entity, |w| {
                let changed = get(w) != v;
                set(w, v);
                changed
            });
        },
    );
    inspector_row(commands, &fonts.ui, label, dv)
}

fn switch(
    commands: &mut Commands,
    fonts: &EmberFonts,
    entity: Entity,
    label: &str,
    init: bool,
    get: fn(&Weather) -> bool,
    set: fn(&mut Weather, bool),
) -> Entity {
    let sw = toggle_switch(commands, init);
    bind_2way(
        commands,
        sw,
        move |w| weather(w, entity).is_some_and(get),
        move |world, v: &bool| {
            let v = *v;
            edit(world, entity, |w| {
                let changed = get(w) != v;
                set(w, v);
                changed
            });
        },
    );
    inspector_row(commands, &fonts.ui, label, sw)
}

/// A titled, foldable group shown only while `visible` says it matters.
fn group(
    commands: &mut Commands,
    fonts: &EmberFonts,
    entity: Entity,
    icon: &str,
    title: &str,
    rows: &[Entity],
    visible: fn(&Weather, &WeatherState) -> bool,
) -> Entity {
    let (root, body) = collapsible(commands, fonts, Some(icon), title, true);
    commands.entity(body).add_children(rows);
    bind_display(commands, root, move |w| {
        let state = w.get_resource::<WeatherState>().copied().unwrap_or_default();
        weather(w, entity).is_some_and(|wt| visible(wt, &state))
    });
    root
}

fn rains(w: &Weather, _: &WeatherState) -> bool {
    matches!(w.preset, WeatherPreset::Rain | WeatherPreset::Storm)
}

fn storms(w: &Weather, _: &WeatherState) -> bool {
    w.preset == WeatherPreset::Storm
}

/// Snow settings matter while it snows, and while snow still lies after: its
/// melt time is exactly what someone switching to rain wants to adjust.
fn snows(w: &Weather, s: &WeatherState) -> bool {
    w.preset == WeatherPreset::Snow || s.snow_cover > 0.001
}

fn not_clear(w: &Weather, _: &WeatherState) -> bool {
    w.preset != WeatherPreset::Clear
}

fn cycles(w: &Weather, _: &WeatherState) -> bool {
    w.cycle
}

fn weather_ui(world: &mut World, entity: Entity) -> Entity {
    let Some(w) = world.get::<Weather>(entity).cloned() else {
        return world.spawn(Node::default()).id();
    };
    inspector_body(world, move |commands, fonts| {
        let col = commands
            .spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(3.0),
                padding: UiRect::all(Val::Px(2.0)),
                ..default()
            })
            .id();

        // The weather itself.
        let selected = WeatherPreset::ALL.iter().position(|p| *p == w.preset).unwrap_or(0);
        let dd = dropdown(commands, fonts, &WeatherPreset::LABELS, selected);
        bind_2way(
            commands,
            dd,
            move |rx| {
                weather(rx, entity)
                    .and_then(|w| WeatherPreset::ALL.iter().position(|p| *p == w.preset))
                    .unwrap_or(0)
            },
            move |world, i: &usize| {
                if let Some(&preset) = WeatherPreset::ALL.get(*i) {
                    edit(world, entity, |w| {
                        let changed = w.preset != preset;
                        w.preset = preset;
                        changed
                    });
                }
            },
        );
        let preset_row = inspector_row(commands, &fonts.ui, "Weather", dd);

        let transition = number(commands, fonts, entity, "Transition (s)", w.transition_seconds, 1.0, (0.0, 1800.0), |w| w.transition_seconds, |w, v| w.transition_seconds = v);
        let intensity = number(commands, fonts, entity, "Intensity", w.intensity, 0.01, (0.0, 2.0), |w| w.intensity, |w, v| w.intensity = v);
        let fog = number(commands, fonts, entity, "Fog", w.fog_amount, 0.01, (0.0, 3.0), |w| w.fog_amount, |w, v| w.fog_amount = v);
        bind_display(commands, fog, move |rx| weather(rx, entity).is_some_and(|w| not_clear(w, &WeatherState::default())));

        // Rain, in rain and storms.
        let rain_rows = [
            switch(commands, fonts, entity, "Show Rain", w.rain_particles, |w| w.rain_particles, |w, v| w.rain_particles = v),
            number(commands, fonts, entity, "Rain Amount", w.rain_amount, 0.01, (0.0, 2.0), |w| w.rain_amount, |w, v| w.rain_amount = v),
        ];
        let rain = group(commands, fonts, entity, "cloud-rain", "Rain", &rain_rows, rains);

        // Lightning, in storms.
        let lightning_rows = [
            switch(commands, fonts, entity, "Lightning", w.lightning, |w| w.lightning, |w, v| w.lightning = v),
            switch(commands, fonts, entity, "Bolts", w.lightning_bolts, |w| w.lightning_bolts, |w, v| w.lightning_bolts = v),
            number(commands, fonts, entity, "Frequency", w.lightning_frequency, 0.01, (0.05, 5.0), |w| w.lightning_frequency, |w, v| w.lightning_frequency = v),
            number(commands, fonts, entity, "Brightness", w.lightning_brightness, 0.01, (0.0, 3.0), |w| w.lightning_brightness, |w, v| w.lightning_brightness = v),
        ];
        let lightning = group(commands, fonts, entity, "lightning", "Lightning", &lightning_rows, storms);

        // Snow, while it snows or lies.
        let snow_rows = [
            switch(commands, fonts, entity, "Show Snow", w.snow_particles, |w| w.snow_particles, |w, v| w.snow_particles = v),
            number(commands, fonts, entity, "Snow Amount", w.snow_amount, 0.01, (0.0, 2.0), |w| w.snow_amount, |w, v| w.snow_amount = v),
            number(commands, fonts, entity, "Time to Cover (s)", w.snow_cover_seconds, 1.0, (5.0, 3600.0), |w| w.snow_cover_seconds, |w, v| w.snow_cover_seconds = v),
            number(commands, fonts, entity, "Melt Time (s)", w.snow_melt_seconds, 1.0, (5.0, 3600.0), |w| w.snow_melt_seconds, |w, v| w.snow_melt_seconds = v),
        ];
        let snow = group(commands, fonts, entity, "snowflake", "Snow", &snow_rows, snows);

        // Changing by itself.
        let cycle = switch(commands, fonts, entity, "Cycle", w.cycle, |w| w.cycle, |w, v| w.cycle = v);
        let cycle_rows = [
            number(commands, fonts, entity, "Hold Min (s)", w.cycle_min_seconds, 5.0, (10.0, 7200.0), |w| w.cycle_min_seconds, |w, v| w.cycle_min_seconds = v),
            number(commands, fonts, entity, "Hold Max (s)", w.cycle_max_seconds, 5.0, (10.0, 7200.0), |w| w.cycle_max_seconds, |w, v| w.cycle_max_seconds = v),
            switch(commands, fonts, entity, "Allow Snow", w.snow_in_cycle, |w| w.snow_in_cycle, |w, v| w.snow_in_cycle = v),
        ];
        for row in cycle_rows {
            bind_display(commands, row, move |rx| weather(rx, entity).is_some_and(|w| cycles(w, &WeatherState::default())));
        }

        commands.entity(col).add_children(&[preset_row, transition, intensity, fog, rain, lightning, snow, cycle]);
        commands.entity(col).add_children(&cycle_rows);
        col
    })
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
        // Drawn by `weather_ui`, which replaces declarative fields entirely.
        fields: vec![],
    }
}

pub(crate) fn register(app: &mut App) {
    app.register_inspector(inspector_entry());
    app.register_native_inspector_ui("weather", weather_ui);
}
