//! [`WeatherLook`]: one complete description of the weather, in numbers that
//! blend.
//!
//! Every preset is built *from the scene's own look* ([`WeatherBase`]) rather
//! than from fixed values. A storm darkens the sun the scene already has and
//! thickens the haze the scene already has, so the same Storm reads right over
//! a golden-hour city and over a noon desert. Only what a weather genuinely
//! decides on its own (how much of the sky is cloud, how low the deck sits) is
//! absolute.

use bevy::prelude::*;
use renzora::{Sun, WorldEnvironment};

use crate::clouds::Clouds;
use crate::data::{WeatherBase, WeatherPreset};

/// The haze a preset thickens when the scene has no fog of its own: about four
/// kilometres of visibility, the thinnest that still reads as air over a city.
const REFERENCE_EXTINCTION: Vec3 = Vec3::new(0.00011, 0.00013, 0.00017);
const REFERENCE_INSCATTERING: Vec3 = Vec3::new(0.00011, 0.00015, 0.00024);

/// The cool grey that haze, fog and dimmed sunlight drift toward under cloud.
/// Overcast light is not neutral: the sky above the deck is blue and the
/// light leaking through keeps a little of it.
const OVERCAST_GREY: Vec3 = Vec3::new(0.93, 0.97, 1.04);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherLook {
    pub clouds: Clouds,
    pub sun_illuminance: f32,
    pub sun_color: Vec3,
    pub sun_disk_intensity: f32,
    pub sun_glare: f32,
    pub fog_color: Vec3,
    pub fog_extinction: Vec3,
    pub fog_inscattering: Vec3,
    pub fog_light_color: Vec3,
    pub wind_speed: f32,
    pub wind_gust_strength: f32,
    /// The environment map's intensity: ambient light and reflections.
    pub ambient_light: f32,
    /// How hard it rains, 0 to 1.
    pub rain: f32,
    /// How hard it snows, 0 to 1.
    pub snow: f32,
    /// How often lightning strikes, 0 (never) to 1 (a full storm).
    pub lightning: f32,
}

/// The user's adjustments to every preset: how much haze, rain and snow it
/// brings, as multiples of what it is tuned to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    pub fog: f32,
    pub rain: f32,
    pub snow: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self { fog: 1.0, rain: 1.0, snow: 1.0 }
    }
}

impl Tuning {
    pub fn of(w: &crate::data::Weather) -> Self {
        Self {
            fog: w.fog_amount.clamp(0.0, 3.0),
            rain: w.rain_amount.clamp(0.0, 2.0),
            snow: w.snow_amount.clamp(0.0, 2.0),
        }
    }
}

fn v(a: [f32; 3]) -> Vec3 {
    Vec3::from_array(a)
}

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

/// `c` pulled toward a cool grey of the same brightness by `amount`.
fn toward_grey(c: Vec3, amount: f32) -> Vec3 {
    c.lerp(OVERCAST_GREY * luminance(c), amount)
}

/// How a preset differs from the scene's own look. Multipliers apply to the
/// base value; the rest are absolute.
struct Recipe {
    coverage: f32,
    density: f32,
    bottom: f32,
    top: f32,
    absorption: f32,
    shadow: Vec3,
    cloud_tint: f32,
    /// How far the cloud's sunlit colour fades toward a cool grey. Without it
    /// a storm rolling in at golden hour is a brown ceiling: the deck keeps
    /// the warm tint authored for sunset cumulus.
    cloud_grey: f32,
    brightness: f32,
    ambient: f32,
    edge: f32,
    detail: f32,
    morph: f32,
    drift: f32,
    /// Fraction of the sun that gets through.
    sun: f32,
    /// How far the sunlight's colour fades toward grey.
    sun_grey: f32,
    disk: f32,
    glare: f32,
    /// Haze thickness, as a multiple of the scene's own.
    fog: f32,
    /// How far the haze's colour fades toward grey.
    fog_grey: f32,
    wind: f32,
    wind_floor: f32,
    gust: f32,
    /// Fraction of the sky's ambient light (the environment map) that gets
    /// through. Dimming only the sun leaves a storm-dark sky over a city
    /// still lit like a clear afternoon.
    ambient_light: f32,
    rain: f32,
    snow: f32,
    lightning: f32,
}

fn recipe(preset: WeatherPreset) -> Option<Recipe> {
    use WeatherPreset::*;
    Some(match preset {
        Clear => return None,
        PartlyCloudy => Recipe {
            coverage: 0.56,
            density: 0.045,
            bottom: 1500.0,
            top: 3400.0,
            absorption: 1.0,
            shadow: Vec3::new(0.48, 0.6, 0.76),
            cloud_tint: 1.0,
            cloud_grey: 0.0,
            brightness: 1.0,
            ambient: 1.0,
            edge: 0.06,
            detail: 0.45,
            morph: 1.0,
            drift: 1.0,
            sun: 0.9,
            sun_grey: 0.0,
            disk: 1.0,
            glare: 1.0,
            fog: 1.15,
            fog_grey: 0.1,
            wind: 1.3,
            wind_floor: 3.0,
            gust: 0.5,
            ambient_light: 0.95,
            rain: 0.0,
            snow: 0.0,
            lightning: 0.0,
        },
        Overcast => Recipe {
            coverage: 0.88,
            density: 0.05,
            bottom: 900.0,
            top: 3400.0,
            absorption: 1.4,
            shadow: Vec3::new(0.4, 0.44, 0.5),
            cloud_tint: 0.92,
            cloud_grey: 0.5,
            brightness: 0.75,
            ambient: 0.8,
            edge: 0.18,
            detail: 0.35,
            morph: 1.0,
            drift: 1.2,
            sun: 0.3,
            sun_grey: 0.6,
            disk: 0.1,
            glare: 0.2,
            fog: 2.5,
            fog_grey: 0.6,
            wind: 1.5,
            wind_floor: 5.0,
            gust: 0.5,
            ambient_light: 0.65,
            rain: 0.0,
            snow: 0.0,
            lightning: 0.0,
        },
        // Rain and Storm keep some broken cloud (coverage under 1, density
        // well under opaque) on purpose: a solid, opaque deck has no shape at
        // all and reads as a flat ceiling. The darker masses and lighter
        // breaks between them are what make a sky read as stormy.
        Rain => Recipe {
            coverage: 0.9,
            density: 0.06,
            bottom: 750.0,
            top: 4500.0,
            absorption: 1.5,
            shadow: Vec3::new(0.3, 0.34, 0.4),
            cloud_tint: 0.85,
            cloud_grey: 0.7,
            brightness: 0.7,
            ambient: 0.6,
            edge: 0.12,
            detail: 0.5,
            morph: 1.5,
            drift: 1.6,
            sun: 0.07,
            sun_grey: 0.85,
            disk: 0.0,
            glare: 0.0,
            fog: 4.0,
            fog_grey: 0.75,
            wind: 2.0,
            wind_floor: 7.0,
            gust: 0.6,
            ambient_light: 0.45,
            rain: 0.65,
            snow: 0.0,
            lightning: 0.0,
        },
        Storm => Recipe {
            coverage: 0.9,
            density: 0.075,
            bottom: 600.0,
            top: 5500.0,
            absorption: 1.6,
            // Slate with a trace of violet: storm undersides lit by a sky that
            // is itself filtered through cloud lean blue-purple, not brown.
            shadow: Vec3::new(0.24, 0.27, 0.34),
            cloud_tint: 0.8,
            cloud_grey: 0.85,
            brightness: 0.7,
            ambient: 0.55,
            // Crisp edges and strong erosion keep the towering masses
            // readable through the gloom.
            edge: 0.08,
            detail: 0.6,
            morph: 3.0,
            drift: 2.5,
            // Low enough that buildings stop casting crisp sun shadows: under
            // a real storm deck the light is all diffuse. At 0.08 the city
            // still read as sunlit, tan with hard shadows, under a black sky.
            sun: 0.025,
            sun_grey: 0.9,
            disk: 0.0,
            glare: 0.0,
            fog: 6.0,
            fog_grey: 0.85,
            wind: 3.5,
            wind_floor: 12.0,
            gust: 0.85,
            ambient_light: 0.3,
            rain: 1.0,
            snow: 0.0,
            lightning: 1.0,
        },
        Foggy => Recipe {
            coverage: 0.6,
            density: 0.04,
            bottom: 300.0,
            top: 1500.0,
            absorption: 1.0,
            shadow: Vec3::new(0.6, 0.64, 0.7),
            cloud_tint: 1.0,
            cloud_grey: 0.4,
            brightness: 0.9,
            ambient: 1.0,
            edge: 0.3,
            detail: 0.2,
            morph: 0.5,
            drift: 0.5,
            sun: 0.45,
            sun_grey: 0.5,
            disk: 0.3,
            glare: 0.6,
            fog: 14.0,
            fog_grey: 0.9,
            wind: 0.4,
            wind_floor: 0.0,
            gust: 0.15,
            ambient_light: 0.7,
            rain: 0.0,
            snow: 0.0,
            lightning: 0.0,
        },
        // Snow cloud is bright, not dark: a thin, even, low deck that glows
        // grey-white, soft-edged and nearly featureless, with cool flat light
        // under it. Snowfall itself is what closes the distance in, so the
        // haze thickens much as it does for rain, in a cool grey-white.
        Snow => Recipe {
            coverage: 0.95,
            density: 0.05,
            bottom: 800.0,
            top: 3200.0,
            absorption: 1.1,
            shadow: Vec3::new(0.62, 0.66, 0.74),
            cloud_tint: 1.0,
            cloud_grey: 0.8,
            brightness: 0.95,
            ambient: 1.1,
            edge: 0.3,
            detail: 0.2,
            morph: 0.6,
            drift: 0.7,
            sun: 0.2,
            sun_grey: 0.85,
            disk: 0.0,
            glare: 0.1,
            fog: 5.0,
            fog_grey: 0.9,
            wind: 0.8,
            wind_floor: 2.0,
            gust: 0.3,
            // Snow light is bright and shadowless: the cloud above and the
            // white below both bounce it, so the sky's share stays high.
            ambient_light: 0.85,
            rain: 0.0,
            snow: 1.0,
            lightning: 0.0,
        },
    })
}

impl WeatherLook {
    /// The scene exactly as authored: the Clear preset.
    pub fn from_base(b: &WeatherBase) -> Self {
        Self {
            clouds: Clouds {
                coverage: b.cloud_coverage,
                density: b.cloud_density,
                bottom_height: b.cloud_bottom,
                top_height: b.cloud_top,
                absorption: b.cloud_absorption,
                color: v(b.cloud_color),
                shadow_color: v(b.cloud_shadow_color),
                brightness: b.cloud_brightness,
                ambient_brightness: b.cloud_ambient_brightness,
                edge_softness: b.cloud_edge_softness,
                detail_strength: b.cloud_detail_strength,
                morph_speed: b.cloud_morph_speed,
                speed: b.cloud_speed,
            },
            sun_illuminance: b.sun_illuminance,
            sun_color: v(b.sun_color),
            sun_disk_intensity: b.sun_disk_intensity,
            sun_glare: b.sun_glare,
            fog_color: v(b.fog_color),
            // No authored fog is no fog: Clear must not add haze the scene
            // never had.
            fog_extinction: if b.fog_enabled { v(b.fog_extinction) } else { Vec3::ZERO },
            fog_inscattering: if b.fog_enabled { v(b.fog_inscattering) } else { Vec3::ZERO },
            fog_light_color: v(b.fog_light_color),
            wind_speed: b.wind_speed,
            wind_gust_strength: b.wind_gust_strength,
            ambient_light: b.env_map_intensity,
            rain: 0.0,
            snow: 0.0,
            lightning: 0.0,
        }
    }

    /// `preset` built over the scene's own look, scaled by `intensity` (1 is
    /// the preset as tuned, 0 is the scene unchanged).
    pub fn for_preset(preset: WeatherPreset, b: &WeatherBase, intensity: f32, tuning: Tuning) -> Self {
        let clear = Self::from_base(b);
        let Some(mut r) = recipe(preset) else {
            return clear;
        };
        // The fog amount scales what the preset adds on top of the scene's own
        // haze, never the scene's haze itself: 0 leaves the authored air.
        r.fog = 1.0 + (r.fog - 1.0) * tuning.fog;
        r.rain *= tuning.rain;
        r.snow *= tuning.snow;
        // The scene's haze, or a city's worth of air if it has none, so a
        // preset has something to thicken.
        let (ext, ins) = if b.fog_enabled && b.fog_mode == 3 {
            (v(b.fog_extinction), v(b.fog_inscattering))
        } else {
            (REFERENCE_EXTINCTION, REFERENCE_INSCATTERING)
        };
        let grey_ext = Vec3::splat(ext.element_sum() / 3.0);
        let grey_ins = OVERCAST_GREY * (ins.element_sum() / 3.0);
        let full = Self {
            clouds: Clouds {
                coverage: r.coverage,
                density: r.density.max(b.cloud_density),
                bottom_height: r.bottom,
                top_height: r.top.max(r.bottom + 500.0),
                absorption: b.cloud_absorption * r.absorption,
                color: toward_grey(clear.clouds.color, r.cloud_grey) * r.cloud_tint,
                shadow_color: r.shadow,
                brightness: b.cloud_brightness * r.brightness,
                ambient_brightness: b.cloud_ambient_brightness * r.ambient,
                edge_softness: r.edge,
                detail_strength: r.detail,
                morph_speed: b.cloud_morph_speed * r.morph,
                speed: b.cloud_speed * r.drift,
            },
            sun_illuminance: b.sun_illuminance * r.sun,
            sun_color: toward_grey(clear.sun_color, r.sun_grey),
            sun_disk_intensity: b.sun_disk_intensity * r.disk,
            sun_glare: b.sun_glare * r.glare,
            // Haze under cloud is lit by the cloud, not the sun: grey, and
            // dimmer as less light gets through.
            fog_color: toward_grey(clear.fog_color, r.fog_grey) * (1.0 - r.fog_grey * (1.0 - r.sun.sqrt())),
            fog_extinction: ext.lerp(grey_ext, r.fog_grey) * r.fog,
            fog_inscattering: ins.lerp(grey_ins, r.fog_grey) * r.fog,
            // The glow around the sun in the haze goes with the sun.
            fog_light_color: clear.fog_light_color * r.sun,
            wind_speed: (b.wind_speed * r.wind).max(r.wind_floor),
            wind_gust_strength: r.gust,
            ambient_light: b.env_map_intensity * r.ambient_light,
            rain: r.rain,
            snow: r.snow,
            lightning: r.lightning,
        };
        clear.lerp(&full, intensity).sanitized()
    }

    pub fn lerp(&self, o: &Self, t: f32) -> Self {
        let f = |a: f32, b: f32| a + (b - a) * t;
        let c = |a: Vec3, b: Vec3| a + (b - a) * t;
        let (a, b) = (&self.clouds, &o.clouds);
        Self {
            clouds: Clouds {
                coverage: f(a.coverage, b.coverage),
                density: f(a.density, b.density),
                bottom_height: f(a.bottom_height, b.bottom_height),
                top_height: f(a.top_height, b.top_height),
                absorption: f(a.absorption, b.absorption),
                color: c(a.color, b.color),
                shadow_color: c(a.shadow_color, b.shadow_color),
                brightness: f(a.brightness, b.brightness),
                ambient_brightness: f(a.ambient_brightness, b.ambient_brightness),
                edge_softness: f(a.edge_softness, b.edge_softness),
                detail_strength: f(a.detail_strength, b.detail_strength),
                morph_speed: f(a.morph_speed, b.morph_speed),
                speed: f(a.speed, b.speed),
            },
            sun_illuminance: f(self.sun_illuminance, o.sun_illuminance),
            sun_color: c(self.sun_color, o.sun_color),
            sun_disk_intensity: f(self.sun_disk_intensity, o.sun_disk_intensity),
            sun_glare: f(self.sun_glare, o.sun_glare),
            fog_color: c(self.fog_color, o.fog_color),
            fog_extinction: c(self.fog_extinction, o.fog_extinction),
            fog_inscattering: c(self.fog_inscattering, o.fog_inscattering),
            fog_light_color: c(self.fog_light_color, o.fog_light_color),
            wind_speed: f(self.wind_speed, o.wind_speed),
            wind_gust_strength: f(self.wind_gust_strength, o.wind_gust_strength),
            ambient_light: f(self.ambient_light, o.ambient_light),
            rain: f(self.rain, o.rain),
            snow: f(self.snow, o.snow),
            lightning: f(self.lightning, o.lightning),
        }
    }

    /// Clamp into the ranges the components accept: an `intensity` above 1
    /// extrapolates past the tuned preset and must not leave them.
    fn sanitized(mut self) -> Self {
        let pos = |x: f32| x.max(0.0);
        let col = |c: Vec3| c.max(Vec3::ZERO);
        let c = &mut self.clouds;
        c.coverage = c.coverage.clamp(0.0, 1.0);
        c.density = c.density.clamp(0.0, 1.0);
        c.bottom_height = pos(c.bottom_height);
        c.top_height = c.top_height.max(c.bottom_height + 1.0);
        c.absorption = pos(c.absorption);
        c.color = col(c.color);
        c.shadow_color = col(c.shadow_color);
        c.brightness = pos(c.brightness);
        c.ambient_brightness = pos(c.ambient_brightness);
        c.edge_softness = c.edge_softness.clamp(0.001, 1.0);
        c.detail_strength = c.detail_strength.clamp(0.0, 1.0);
        c.morph_speed = pos(c.morph_speed);
        c.speed = pos(c.speed);
        self.sun_illuminance = pos(self.sun_illuminance);
        self.sun_color = col(self.sun_color);
        self.sun_disk_intensity = pos(self.sun_disk_intensity);
        self.sun_glare = pos(self.sun_glare);
        self.fog_color = col(self.fog_color);
        self.fog_extinction = col(self.fog_extinction);
        self.fog_inscattering = col(self.fog_inscattering);
        self.fog_light_color = col(self.fog_light_color);
        self.wind_speed = pos(self.wind_speed);
        self.wind_gust_strength = self.wind_gust_strength.clamp(0.0, 1.0);
        self.ambient_light = pos(self.ambient_light);
        // Up to 2: the rain and snow amounts can double a preset's own.
        self.rain = self.rain.clamp(0.0, 2.0);
        self.snow = self.snow.clamp(0.0, 2.0);
        self.lightning = self.lightning.clamp(0.0, 1.0);
        self
    }
}

/// The scene's look as it stands, for [`crate::data::Weather::base`].
pub fn capture(
    clouds: Option<&Clouds>,
    sun: Option<&Sun>,
    env: Option<&WorldEnvironment>,
    env_map_intensity: Option<f32>,
) -> WeatherBase {
    let mut b = WeatherBase::default();
    if let Some(i) = env_map_intensity {
        b.has_env_map = true;
        b.env_map_intensity = i;
    }
    if let Some(c) = clouds {
        b.has_clouds = true;
        b.cloud_coverage = c.coverage;
        b.cloud_density = c.density;
        b.cloud_bottom = c.bottom_height;
        b.cloud_top = c.top_height;
        b.cloud_absorption = c.absorption;
        b.cloud_color = c.color.to_array();
        b.cloud_shadow_color = c.shadow_color.to_array();
        b.cloud_brightness = c.brightness;
        b.cloud_ambient_brightness = c.ambient_brightness;
        b.cloud_edge_softness = c.edge_softness;
        b.cloud_detail_strength = c.detail_strength;
        b.cloud_morph_speed = c.morph_speed;
        b.cloud_speed = c.speed;
    }
    if let Some(s) = sun {
        b.has_sun = true;
        b.sun_illuminance = s.illuminance;
        b.sun_color = s.color.to_array();
        b.sun_disk_intensity = s.sun_disk_intensity;
        b.sun_glare = s.glare_intensity;
    }
    if let Some(e) = env {
        b.has_environment = true;
        b.fog_enabled = e.fog.enabled;
        b.fog_mode = e.fog.mode;
        b.fog_color = e.fog.color;
        b.fog_extinction = e.fog.extinction;
        b.fog_inscattering = e.fog.inscattering;
        b.fog_light_color = e.fog.directional_light_color;
        b.wind_speed = e.wind.speed;
        b.wind_gust_strength = e.wind.gust_strength;
    }
    b
}

/// The deck for `look`, with `flash` (0 to 1) of lightning glowing inside it.
pub fn clouds_for(look: &WeatherLook, flash: f32) -> Clouds {
    let mut c = look.clouds;
    // A strike lights the deck from inside: the whole cloud glows, its
    // undersides most of all.
    c.brightness *= 1.0 + flash * 5.0;
    c.ambient_brightness *= 1.0 + flash * 8.0;
    c
}

pub fn apply_sun(look: &WeatherLook, base: &WeatherBase, flash: f32, s: &mut Sun) {
    // A strike lights the ground a little: enough to read as a flash without
    // the shadows betraying that it comes from the sun.
    s.illuminance = look.sun_illuminance + flash * base.sun_illuminance * 0.35;
    s.color = look.sun_color.lerp(Vec3::new(0.85, 0.9, 1.0), flash.min(1.0));
    s.sun_disk_intensity = look.sun_disk_intensity;
    s.glare_intensity = look.sun_glare;
}

pub fn apply_environment(look: &WeatherLook, base: &WeatherBase, flash: f32, e: &mut WorldEnvironment) {
    let fog_on = base.fog_enabled || look.fog_extinction.max_element() > 1e-7;
    e.fog.enabled = fog_on;
    if fog_on {
        // Weather thickens atmospheric haze; a scene authored with a linear
        // or exponential fog is switched to it while weather runs and gets
        // its own mode back when weather stops.
        e.fog.mode = 3;
        e.fog.color = (look.fog_color * (1.0 + flash * 2.0)).to_array();
        e.fog.extinction = look.fog_extinction.to_array();
        e.fog.inscattering = look.fog_inscattering.to_array();
        e.fog.directional_light_color = look.fog_light_color.to_array();
    }
    e.wind.speed = look.wind_speed;
    e.wind.gust_strength = look.wind_gust_strength;
}

/// Put the World Environment back exactly as it was before weather.
pub fn restore_environment(base: &WeatherBase, e: &mut WorldEnvironment) {
    e.fog.enabled = base.fog_enabled;
    e.fog.mode = base.fog_mode;
    e.fog.color = base.fog_color;
    e.fog.extinction = base.fog_extinction;
    e.fog.inscattering = base.fog_inscattering;
    e.fog.directional_light_color = base.fog_light_color;
    e.wind.speed = base.wind_speed;
    e.wind.gust_strength = base.wind_gust_strength;
}
