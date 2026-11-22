//! The authored half: the [`Weather`] component, its presets, and the
//! [`WeatherBase`] snapshot of the look the scene had before weather took over.

use bevy::prelude::*;
use renzora::serde::{Deserialize, Serialize};

/// A named weather. Each one is a full description of the sky, the light, the
/// haze, the wind and the rain (see [`crate::look::WeatherLook::for_preset`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[serde(crate = "renzora::serde")]
pub enum WeatherPreset {
    /// The scene exactly as authored, before weather touched it.
    #[default]
    Clear,
    /// Scattered fair-weather cumulus; the sun still out.
    PartlyCloudy,
    /// A low, grey, unbroken deck. Flat light, dim sun, grey haze.
    Overcast,
    /// Overcast with rain falling and a heavier haze.
    Rain,
    /// A towering dark deck, a downpour, strong gusting wind and lightning.
    Storm,
    /// Thick ground fog under a pale sky.
    Foggy,
}

impl WeatherPreset {
    pub const ALL: [WeatherPreset; 6] = [
        WeatherPreset::Clear,
        WeatherPreset::PartlyCloudy,
        WeatherPreset::Overcast,
        WeatherPreset::Rain,
        WeatherPreset::Storm,
        WeatherPreset::Foggy,
    ];

    /// The inspector's labels, in [`ALL`](Self::ALL) order.
    pub const LABELS: [&'static str; 6] = ["Clear", "Partly Cloudy", "Overcast", "Rain", "Storm", "Foggy"];

    pub fn label(self) -> &'static str {
        Self::LABELS[self as usize]
    }

    /// Parse a label or a script-friendly name (`"partly_cloudy"`,
    /// `"PartlyCloudy"` and `"partly cloudy"` all work).
    pub fn parse(name: &str) -> Option<Self> {
        let key = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        Some(match key.as_str() {
            "clear" | "sunny" => Self::Clear,
            "partlycloudy" | "cloudy" => Self::PartlyCloudy,
            "overcast" => Self::Overcast,
            "rain" | "rainy" => Self::Rain,
            "storm" | "stormy" | "thunderstorm" => Self::Storm,
            "foggy" | "fog" | "mist" => Self::Foggy,
            _ => return None,
        })
    }

    /// Where the weather tends to go next when cycling, as (preset, weight).
    ///
    /// A chain rather than a uniform pick, because weather builds and clears
    /// through its stages: a clear sky clouds over before it rains, and a storm
    /// eases into rain before it clears. Jumping from blue sky straight to a
    /// thunderstorm reads as a bug, however slowly it blends.
    pub fn successors(self) -> &'static [(WeatherPreset, u32)] {
        use WeatherPreset::*;
        match self {
            Clear => &[(Clear, 2), (PartlyCloudy, 5), (Foggy, 1)],
            PartlyCloudy => &[(Clear, 4), (PartlyCloudy, 2), (Overcast, 3)],
            Overcast => &[(PartlyCloudy, 3), (Overcast, 1), (Rain, 3)],
            Rain => &[(Overcast, 3), (Rain, 1), (Storm, 2)],
            Storm => &[(Rain, 1)],
            Foggy => &[(Clear, 2), (PartlyCloudy, 2), (Overcast, 1)],
        }
    }
}

/// Dynamic weather. Add it to the World Environment entity (any entity works;
/// the first enabled one in the scene drives the world).
///
/// While it is enabled it owns the clouds, the sun's strength and colour, the
/// distance fog and the wind: the look the scene had when it was switched on
/// is kept in [`base`](Self::base) and is the **Clear** preset, and every
/// other preset is built from it. Switching weather off puts that look back.
/// To change the authored look itself, switch weather off, edit, and switch
/// it back on.
#[derive(Component, Clone, Debug, Reflect, Serialize, Deserialize)]
#[serde(crate = "renzora::serde", default)]
#[reflect(Component, Default, Serialize, Deserialize)]
pub struct Weather {
    pub enabled: bool,
    /// The weather to be in, or to be blending toward.
    pub preset: WeatherPreset,
    /// How long a change of preset takes to blend through, in seconds. Real
    /// weather turns over in minutes; anything under about ten reads as a
    /// switch rather than a change.
    pub transition_seconds: f32,
    /// Change the weather by itself, moving along the chain in
    /// [`WeatherPreset::successors`].
    pub cycle: bool,
    /// Shortest time a weather holds before cycling on, in seconds.
    pub cycle_min_seconds: f32,
    /// Longest time a weather holds before cycling on, in seconds.
    pub cycle_max_seconds: f32,
    /// Spawn falling rain around the camera when the weather rains.
    pub rain_particles: bool,
    /// Lightning flashes in a storm.
    pub lightning: bool,
    /// Multiplies how strongly each preset darkens, fogs and blows, 0 to 2.
    /// 1 is the presets as tuned; below that for a gentler climate.
    pub intensity: f32,
    /// The scene's own look, captured when weather was switched on. Saved with
    /// the scene, because the clouds and fog saved beside it are the
    /// *weathered* values and cannot be told apart from authored ones.
    pub base: Option<WeatherBase>,
}

impl Default for Weather {
    fn default() -> Self {
        Self {
            enabled: true,
            preset: WeatherPreset::Clear,
            transition_seconds: 45.0,
            cycle: false,
            cycle_min_seconds: 240.0,
            cycle_max_seconds: 720.0,
            rain_particles: true,
            lightning: true,
            intensity: 1.0,
            base: None,
        }
    }
}

/// Everything weather overwrites, as the scene had it before weather began.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[serde(crate = "renzora::serde", default)]
#[reflect(Default, Serialize, Deserialize)]
pub struct WeatherBase {
    // Clouds. `has_clouds` false means the scene has no deck to drive.
    pub has_clouds: bool,
    pub cloud_coverage: f32,
    pub cloud_density: f32,
    pub cloud_bottom: f32,
    pub cloud_top: f32,
    pub cloud_absorption: f32,
    pub cloud_color: [f32; 3],
    pub cloud_shadow_color: [f32; 3],
    pub cloud_brightness: f32,
    pub cloud_ambient_brightness: f32,
    pub cloud_edge_softness: f32,
    pub cloud_detail_strength: f32,
    pub cloud_morph_speed: f32,
    pub cloud_speed: f32,
    // Sun. `has_sun` false means there is no `Sun` to dim.
    pub has_sun: bool,
    pub sun_illuminance: f32,
    pub sun_color: [f32; 3],
    pub sun_disk_intensity: f32,
    pub sun_glare: f32,
    // Haze and wind, from the World Environment.
    pub has_environment: bool,
    pub fog_enabled: bool,
    pub fog_mode: u32,
    pub fog_color: [f32; 3],
    pub fog_extinction: [f32; 3],
    pub fog_inscattering: [f32; 3],
    pub fog_light_color: [f32; 3],
    pub wind_speed: f32,
    pub wind_gust_strength: f32,
    // The environment map's strength: the ambient light and reflections the
    // sky lends the scene. `has_env_map` false means there is none to dim.
    pub has_env_map: bool,
    pub env_map_intensity: f32,
}

impl Default for WeatherBase {
    fn default() -> Self {
        Self {
            has_clouds: false,
            cloud_coverage: 0.45,
            cloud_density: 0.021,
            cloud_bottom: 1690.0,
            cloud_top: 2960.0,
            cloud_absorption: 1.0,
            cloud_color: [1.0, 0.96, 0.9],
            cloud_shadow_color: [0.55, 0.66, 0.8],
            cloud_brightness: 5.5,
            cloud_ambient_brightness: 2.5,
            cloud_edge_softness: 0.1,
            cloud_detail_strength: 0.27,
            cloud_morph_speed: 50.0,
            cloud_speed: 40.0,
            has_sun: false,
            sun_illuminance: 100_000.0,
            sun_color: [1.0, 1.0, 1.0],
            sun_disk_intensity: 1.0,
            sun_glare: 1.5,
            has_environment: false,
            fog_enabled: false,
            fog_mode: 3,
            fog_color: [0.72, 0.78, 0.9],
            fog_extinction: [0.0; 3],
            fog_inscattering: [0.0; 3],
            fog_light_color: [1.0, 0.92, 0.75],
            wind_speed: 4.0,
            wind_gust_strength: 0.45,
            has_env_map: false,
            env_map_intensity: 0.5,
        }
    }
}

/// This frame's weather, already blended between presets, for anything that
/// reacts to it: a material darkening wet asphalt, an audio bed crossfading
/// into rain, a script (read it with `get("WeatherState.rain")`).
#[derive(Resource, Clone, Copy, Debug, Default, Reflect)]
#[reflect(Resource)]
pub struct WeatherState {
    /// How much of the sky is cloud, 0 (clear) to 1 (solid overcast).
    pub cloudiness: f32,
    /// How hard it is raining, 0 (dry) to 1 (downpour).
    pub rain: f32,
    /// How wet the ground is, 0 to 1. Lags `rain` on purpose: streets take
    /// minutes to soak through and longer to dry, so a shower that has just
    /// stopped still leaves them shining.
    pub wetness: f32,
    /// This frame's lightning flash, 0 (none) to 1 (the peak of a strike).
    pub lightning: f32,
}
