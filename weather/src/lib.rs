//! Dynamic weather: a sky that changes.
//!
//! A [`Weather`] component on the World Environment picks a preset (Clear,
//! Partly Cloudy, Overcast, Rain, Storm, Foggy, Snow) and blends the whole
//! world into it over a transition time: the cloud deck's coverage, density,
//! height and colour, the sun's strength and colour, the sky's ambient light,
//! the haze, and the wind. It can cycle through weathers by itself, along a
//! chain that builds and clears the way real weather does. Rain and snow fall
//! around the camera, and a storm brings lightning in the clouds.
//!
//! Weather does not hold a look of its own. It snapshots the scene's look when
//! it is switched on ([`WeatherBase`], saved with the scene) and builds every
//! preset from that, so a storm over a golden-hour city is a golden-hour storm.
//! Switching it off, or removing it, puts the snapshot back.
//!
//! The clouds and the falling rain and snow belong to engine crates a native
//! plugin cannot link, so they are driven through reflection ([`clouds`],
//! [`precipitation`]). The evaluated result is published as the
//! [`WeatherState`] resource (rain, wetness, snow, snow cover, lightning), for
//! materials, audio and scripts to react to. Lying snow is drawn by the plugin
//! itself, into the deferred G-buffer ([`snow_cover`]), so every material
//! carries it without being changed.

use bevy::prelude::*;

pub mod bolt;
pub mod clouds;
pub mod data;
pub mod look;
pub mod precipitation;
pub mod script;
pub mod snow_cover;
pub mod systems;

// The inspector section lives in `src/editor.rs`. `renzora::plugin!` declares
// the `editor` module itself (the build finds it from `src/editor.rs`),
// compiling it in the editor and leaving it out of a lean game build.

pub use data::{Weather, WeatherBase, WeatherPreset, WeatherState};
pub use look::WeatherLook;
pub use precipitation::Precipitation;
pub use systems::WeatherDriver;

#[derive(Default)]
pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        info!("[runtime] WeatherPlugin");
        // Registered everywhere, a dedicated server included, so a scene that
        // carries weather deserializes the same on every build.
        app.register_type::<Weather>()
            .register_type::<WeatherBase>()
            .register_type::<WeatherPreset>()
            .register_type::<WeatherState>()
            .init_resource::<WeatherState>()
            .init_resource::<WeatherDriver>()
            .add_observer(script::handle_script_actions);

        // Weather is what the world looks like; a server has nothing to show.
        if app.world().contains_resource::<renzora::DedicatedServer>() {
            return;
        }
        bevy::asset::embedded_asset!(app, "rain.particle");
        bevy::asset::embedded_asset!(app, "snow.particle");
        snow_cover::build(app);
        app.add_observer(systems::restore_on_remove)
            .add_systems(
                Update,
                (systems::drive_weather, precipitation::follow_precipitation, bolt::fade_bolts).chain(),
            );
    }

    fn finish(&self, app: &mut App) {
        // `renzora::script_extension`, not `renzora_scripting`: a plugin
        // reaches `bevy`, `renzora` and `renzora::ui` and nothing else.
        let mut extensions = app
            .world_mut()
            .get_resource_or_insert_with(renzora::script_extension::ScriptExtensions::default);
        extensions.register(script::WeatherScriptExtension);
    }
}

renzora::plugin!(WeatherPlugin, Runtime);
