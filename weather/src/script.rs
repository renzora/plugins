//! Script control: `set_weather(name, seconds)` and `set_weather_cycle(on)`.
//!
//! Declared through `renzora::script_extension`, so every language backend
//! builds them and this plugin links no interpreter. A call acts on the first
//! [`Weather`] in the scene, switching it on if it was off. Reading the weather
//! goes through the reflection path (`get("WeatherState.rain")`), so only the
//! changes need declaring.

use bevy::prelude::*;
use renzora::script_extension::{Bind, Binding, ParamKind, ScriptExtension};
use renzora::{ScriptAction, ScriptActionValue};

use crate::data::{Weather, WeatherPreset};
use crate::systems::WeatherDriver;

pub struct WeatherScriptExtension;

impl ScriptExtension for WeatherScriptExtension {
    fn name(&self) -> &str {
        "weather"
    }

    fn bindings(&self) -> Vec<Binding> {
        vec![
            Bind::action("set_weather", "set_weather")
                .arg("name", ParamKind::Str)
                .arg("seconds", ParamKind::Float)
                .doc(
                    "Change the weather to clear, partly_cloudy, overcast, rain, storm or foggy, \
                     blending over the given seconds (a negative number uses the Weather's own \
                     transition time).",
                )
                .build(),
            Bind::action("set_weather_cycle", "set_weather_cycle")
                .arg("on", ParamKind::Bool)
                .doc("Let the weather change by itself (true) or hold where it is (false).")
                .build(),
        ]
    }
}

pub fn handle_script_actions(
    trigger: On<ScriptAction>,
    mut weathers: Query<&mut Weather>,
    mut driver: ResMut<WeatherDriver>,
) {
    let action = trigger.event();
    if action.name != "set_weather" && action.name != "set_weather_cycle" {
        return;
    }
    let Some(mut weather) = weathers.iter_mut().next() else {
        renzora::console_log::console_warn(
            "Weather",
            format!("{}: the scene has no Weather; add one to the World Environment", action.name),
        );
        return;
    };
    match action.name.as_str() {
        "set_weather" => {
            let name = match action.args.get("name") {
                Some(ScriptActionValue::String(s)) => s.as_str(),
                _ => "",
            };
            let Some(preset) = WeatherPreset::parse(name) else {
                renzora::console_log::console_warn("Weather", format!("set_weather: no weather called `{name}`"));
                return;
            };
            let seconds = match action.args.get("seconds") {
                Some(ScriptActionValue::Float(f)) => Some(*f),
                Some(ScriptActionValue::Int(i)) => Some(*i as f32),
                _ => None,
            };
            driver.pending_duration = seconds.filter(|s| s.is_finite() && *s >= 0.0);
            weather.enabled = true;
            weather.preset = preset;
        }
        "set_weather_cycle" => {
            if let Some(ScriptActionValue::Bool(on)) = action.args.get("on") {
                weather.cycle = *on;
            }
        }
        _ => {}
    }
}
