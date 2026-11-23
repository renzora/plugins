//! The weather clock: blending between presets, cycling, lightning, and the
//! wetness that lags the rain.
//!
//! One exclusive system, because the cloud deck is reached through reflection
//! (see [`crate::clouds`]), which needs the whole `World`.

use bevy::prelude::*;
use renzora::{Sun, WorldEnvironment};

use crate::clouds::{self, Clouds};
use crate::data::{Weather, WeatherBase, WeatherPreset, WeatherState};
use crate::look::{self, Tuning, WeatherLook};

/// Seconds for a dry street to soak through in a downpour, and for a soaked
/// one to dry. Drying is the slower of the two, which is what makes the
/// streets shine after a shower has passed.
const SOAK_SECONDS: f32 = 90.0;
const DRY_SECONDS: f32 = 360.0;

/// How much faster snow melts under full rain than on a dry day: rain on snow
/// washes it away, which is what lets a storm clear a snowy street.
const RAIN_MELT_BOOST: f32 = 3.0;

pub(crate) const STRIKE_SECONDS: f32 = 0.7;

/// How long the leader takes to step down from the cloud to the ground: two
/// or three frames, a flicker rather than something to watch. Only when it
/// arrives does the return stroke, the flash that lights the world, go off.
pub(crate) const LEADER_SECONDS: f32 = 0.04;

/// The world's light from a strike `age` seconds after the leader set off:
/// nothing while it descends, then the return stroke's double flicker.
pub(crate) fn strike_light(age: f32) -> f32 {
    if age < LEADER_SECONDS {
        0.0
    } else {
        strike_envelope(age - LEADER_SECONDS)
    }
}

/// The engine's environment map settings: the sky's ambient light and
/// reflections. Reached by reflection, like the cloud deck.
const ENV_MAP_TYPE: &str = "EnvironmentMapComponentSettings";
const ENV_MAP_FIELD: &str = "intensity";

/// The weather being driven, and where it is in its blend.
///
/// A resource, not a component, because one weather drives the whole world;
/// it follows whichever entity holds the first enabled [`Weather`]. Nothing
/// here is saved: a loaded scene starts settled in its saved preset.
#[derive(Resource, Default)]
pub struct WeatherDriver {
    entity: Option<Entity>,
    base: Option<WeatherBase>,
    intensity: f32,
    /// The fog, rain and snow amounts the current look was built with.
    tuning: Tuning,
    /// The look blended from, the preset blended to, and how far along (0..1).
    from: Option<WeatherLook>,
    current: Option<WeatherLook>,
    target: WeatherPreset,
    t: f32,
    /// Seconds left before cycling on.
    hold: f32,
    /// Seconds until the next strike, and the age of the current one.
    next_strike: f32,
    strike_age: Option<f32>,
    /// Whether the settled look has been written since the blend finished.
    settled: bool,
    /// A `set_weather` script call's duration, used by the next transition
    /// only, so a script does not rewrite the authored transition time.
    pub(crate) pending_duration: Option<f32>,
    rng: u64,
}

impl WeatherDriver {
    /// xorshift64*: enough randomness for weather and lightning without
    /// pulling a crate in for it.
    pub(crate) fn random(&mut self) -> f32 {
        if self.rng == 0 {
            self.rng = 0x9E37_79B9_7F4A_7C15;
        }
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo).max(0.0) * self.random()
    }

    fn pick_successor(&mut self, from: WeatherPreset, allow_snow: bool) -> WeatherPreset {
        let options: Vec<(WeatherPreset, u32)> = from
            .successors()
            .iter()
            .copied()
            .filter(|(p, _)| allow_snow || *p != WeatherPreset::Snow)
            .collect();
        let total: u32 = options.iter().map(|(_, w)| w).sum();
        if total == 0 {
            return from;
        }
        let mut roll = (self.random() * total as f32) as u32;
        for &(p, w) in &options {
            if roll < w {
                return p;
            }
            roll -= w;
        }
        options.last().map(|(p, _)| *p).unwrap_or(from)
    }

    fn reset(&mut self) {
        *self = WeatherDriver { rng: self.rng, ..default() };
    }
}

/// A lightning strike's brightness `age` seconds in: a bright leader, then a
/// weaker return stroke, which is the double flicker that makes a flash read
/// as lightning rather than a lamp.
pub(crate) fn strike_envelope(age: f32) -> f32 {
    if age < 0.0 {
        return 0.0;
    }
    let first = (-age * 18.0).exp();
    let second = if age > 0.16 { 0.7 * (-(age - 0.16) * 12.0).exp() } else { 0.0 };
    (first + second).min(1.0)
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Wetness heads toward `to`: soaking at a pace set by how hard it rains,
/// drying at a steady, slower one.
fn soak(w: f32, to: f32, rain: f32, dt: f32) -> f32 {
    if to > w {
        (w + dt * rain.max(0.2) / SOAK_SECONDS).min(to)
    } else {
        (w - dt / DRY_SECONDS).max(to)
    }
}

/// Snow cover heads toward full while snow falls, at a pace set by how hard,
/// and melts once it stops: over `melt_seconds` on a dry day, several times
/// faster under rain.
fn settle(cover: f32, snow: f32, rain: f32, dt: f32, cover_seconds: f32, melt_seconds: f32) -> f32 {
    if snow > 0.05 {
        (cover + dt * snow / cover_seconds.max(1.0)).min(1.0)
    } else {
        let melt = (1.0 + rain.min(1.0) * RAIN_MELT_BOOST) / melt_seconds.max(1.0);
        (cover - dt * melt).max(0.0)
    }
}

fn first_sun(world: &mut World) -> Option<Sun> {
    world.query::<&Sun>().iter(world).next().cloned()
}

fn first_environment(world: &mut World) -> Option<WorldEnvironment> {
    world.query::<&WorldEnvironment>().iter(world).next().cloned()
}

/// Write `look` into the scene, with `flash` of lightning on top.
fn write_look(world: &mut World, look: &WeatherLook, base: &WeatherBase, flash: f32, deck: Option<Entity>) {
    if let (Some(entity), true) = (deck, base.has_clouds) {
        clouds::write(world, entity, &look::clouds_for(look, flash));
    }
    if base.has_sun {
        if let Some(mut s) = world.query::<&mut Sun>().iter_mut(world).next() {
            look::apply_sun(look, base, flash, &mut s);
        }
    }
    if base.has_environment {
        if let Some(mut e) = world.query::<&mut WorldEnvironment>().iter_mut(world).next() {
            look::apply_environment(look, base, flash, &mut e);
        }
    }
    if base.has_env_map {
        if let Some((entity, _)) = clouds::read_scalar(world, ENV_MAP_TYPE, ENV_MAP_FIELD) {
            // A strike lights the whole city from the sky, not just the cloud.
            let ambient = look.ambient_light * (1.0 + flash * 6.0);
            clouds::write_scalar(world, entity, ENV_MAP_TYPE, ENV_MAP_FIELD, ambient);
        }
    }
}

/// Put the scene back exactly as `base` describes it.
pub fn restore(world: &mut World, base: &WeatherBase) {
    let clear = WeatherLook::from_base(base);
    if base.has_clouds {
        if let Some((entity, _)) = clouds::read(world) {
            clouds::write(world, entity, &clear.clouds);
        }
    }
    if base.has_sun {
        if let Some(mut s) = world.query::<&mut Sun>().iter_mut(world).next() {
            look::apply_sun(&clear, base, 0.0, &mut s);
        }
    }
    if base.has_environment {
        if let Some(mut e) = world.query::<&mut WorldEnvironment>().iter_mut(world).next() {
            look::restore_environment(base, &mut e);
        }
    }
    if base.has_env_map {
        if let Some((entity, _)) = clouds::read_scalar(world, ENV_MAP_TYPE, ENV_MAP_FIELD) {
            clouds::write_scalar(world, entity, ENV_MAP_TYPE, ENV_MAP_FIELD, base.env_map_intensity);
        }
    }
}

pub fn drive_weather(world: &mut World) {
    world.resource_scope(|world, mut driver: Mut<WeatherDriver>| drive(world, &mut driver));
}

fn drive(world: &mut World, driver: &mut WeatherDriver) {
    let dt = world.resource::<Time>().delta_secs();

    // Weather switched off: put the scene back and forget the snapshot, so
    // switching it on again captures whatever the scene looks like then.
    let mut weathers = world.query::<(Entity, &mut Weather)>();
    let switched_off: Vec<(Entity, WeatherBase)> = weathers
        .iter_mut(world)
        .filter(|(_, w)| !w.enabled && w.base.is_some())
        .filter_map(|(e, mut w)| w.base.take().map(|b| (e, b)))
        .collect();
    for (entity, base) in switched_off {
        restore(world, &base);
        if driver.entity == Some(entity) {
            driver.reset();
        }
    }

    let active = weathers.iter(world).find(|(_, w)| w.enabled).map(|(e, w)| (e, w.clone()));
    let Some((entity, weather)) = active else {
        driver.entity = None;
        let mut state = world.resource_mut::<WeatherState>();
        let wetness = soak(state.wetness, 0.0, 0.0, dt);
        // With weather off nothing is falling, so the snow simply melts.
        let snow_cover = settle(state.snow_cover, 0.0, 0.0, dt, 1.0, 120.0);
        *state = WeatherState { wetness, snow_cover, ..default() };
        return;
    };

    let deck = clouds::read(world);
    let base = match weather.base.clone() {
        Some(base) => base,
        None => {
            let sun = first_sun(world);
            let env = first_environment(world);
            let env_map = clouds::read_scalar(world, ENV_MAP_TYPE, ENV_MAP_FIELD).map(|(_, i)| i);
            let base = look::capture(
                deck.as_ref().map(|(_, c): &(Entity, Clouds)| c),
                sun.as_ref(),
                env.as_ref(),
                env_map,
            );
            if let Some(mut w) = world.get_mut::<Weather>(entity) {
                w.base = Some(base.clone());
            }
            base
        }
    };
    let intensity = weather.intensity.clamp(0.0, 2.0);
    let tuning = Tuning::of(&weather);
    // A tuning slider applies at once, without a blend: the user is looking
    // at the result while dragging.
    if driver.tuning != tuning {
        driver.tuning = tuning;
        driver.settled = false;
    }

    if driver.rng == 0 {
        driver.rng = (world.resource::<Time>().elapsed().as_nanos() as u64) ^ (entity.to_bits() << 17) | 1;
    }

    // A new weather entity, a loaded scene, or a fresh snapshot: start settled
    // in the chosen preset rather than blending in from nothing.
    let fresh = driver.entity != Some(entity) || driver.base.as_ref() != Some(&base) || driver.current.is_none();
    if fresh {
        let look = WeatherLook::for_preset(weather.preset, &base, intensity, tuning);
        driver.reset();
        driver.tuning = tuning;
        driver.entity = Some(entity);
        driver.base = Some(base.clone());
        driver.intensity = intensity;
        driver.from = Some(look);
        driver.current = Some(look);
        driver.target = weather.preset;
        driver.t = 1.0;
        driver.hold = driver.range(weather.cycle_min_seconds, weather.cycle_max_seconds);
        driver.next_strike = driver.range(2.0, 10.0);
    }

    // A change of preset (the inspector, a script, or the cycle below) or of
    // intensity blends from wherever the weather is right now, so changing
    // your mind halfway through a transition does not snap.
    if weather.preset != driver.target || (intensity - driver.intensity).abs() > 1e-4 {
        driver.from = driver.current;
        driver.target = weather.preset;
        driver.intensity = intensity;
        driver.t = 0.0;
        driver.settled = false;
    }

    if driver.t < 1.0 {
        let seconds = driver.pending_duration.unwrap_or(weather.transition_seconds);
        driver.t = if seconds <= 0.0 { 1.0 } else { (driver.t + dt / seconds).min(1.0) };
        if driver.t >= 1.0 {
            driver.pending_duration = None;
        }
    }
    let target_look = WeatherLook::for_preset(driver.target, &base, intensity, tuning);
    let current = match driver.from {
        // Settled: the target itself, so a tuning change shows at once rather
        // than staying with the look the blend started from.
        Some(_) if driver.t >= 1.0 => target_look,
        Some(from) => from.lerp(&target_look, smoothstep(driver.t)),
        None => target_look,
    };
    driver.current = Some(current);

    // Cycling: once settled, hold for a while, then move along the chain.
    if weather.cycle && driver.t >= 1.0 {
        driver.hold -= dt;
        if driver.hold <= 0.0 {
            let next = driver.pick_successor(weather.preset, weather.snow_in_cycle);
            driver.hold = driver.range(weather.cycle_min_seconds, weather.cycle_max_seconds);
            if next != weather.preset {
                if let Some(mut w) = world.get_mut::<Weather>(entity) {
                    w.preset = next;
                }
            }
        }
    }

    // Lightning, more often the stormier it is.
    let mut flash = 0.0;
    let mut struck = false;
    let frequency = weather.lightning_frequency.clamp(0.05, 5.0);
    if weather.lightning && current.lightning > 0.05 {
        if let Some(age) = driver.strike_age.as_mut() {
            *age += dt;
            flash = strike_light(*age);
            if *age > LEADER_SECONDS + STRIKE_SECONDS {
                driver.strike_age = None;
                flash = 0.0;
            }
        } else {
            driver.next_strike -= dt;
            if driver.next_strike <= 0.0 {
                driver.strike_age = Some(0.0);
                struck = true;
                driver.next_strike = driver.range(4.0, 16.0) / (current.lightning * frequency);
            }
        }
    } else {
        driver.strike_age = None;
    }
    let flashing = driver.strike_age.is_some();
    let flash_light = flash * weather.lightning_brightness.clamp(0.0, 3.0);
    if struck && weather.lightning_bolts {
        let seed = driver.random();
        crate::bolt::strike(world, &current, weather.lightning_brightness.clamp(0.0, 3.0), seed);
    }

    // Write only while something is changing. A settled weather leaves the
    // components alone, so it does not fight a user dragging a cloud slider
    // and does not mark the scene changed every frame.
    if driver.t < 1.0 || flashing || !driver.settled {
        write_look(world, &current, &base, flash_light, deck.map(|(e, _)| e));
        driver.settled = driver.t >= 1.0 && !flashing;
    }

    let mut state = world.resource_mut::<WeatherState>();
    let soak_to = if current.rain > 0.05 { 1.0 } else { 0.0 };
    let wetness = soak(state.wetness, soak_to, current.rain, dt);
    let snow_cover = settle(
        state.snow_cover,
        current.snow,
        current.rain,
        dt,
        weather.snow_cover_seconds,
        weather.snow_melt_seconds,
    );
    *state = WeatherState {
        cloudiness: if base.has_clouds { current.clouds.coverage } else { 0.0 },
        rain: current.rain,
        wetness,
        snow: current.snow,
        snow_cover,
        lightning: flash,
    };
}

/// Weather removed from its entity: put the scene back as it was.
///
/// The snapshot is read here, while the component still exists; the restore
/// itself needs the whole `World` (the deck is reflected), so it runs as a
/// command.
pub fn restore_on_remove(trigger: On<Remove<Weather>>, weathers: Query<&Weather>, mut commands: Commands) {
    let entity = trigger.entity;
    let base = weathers.get(entity).ok().filter(|w| w.enabled).and_then(|w| w.base.clone());
    commands.queue(move |world: &mut World| {
        // Only when the component was taken off a living entity. A despawn
        // (closing or switching scenes) also removes it, and by the time this
        // runs the next scene may be loading: restoring then would write the
        // old scene's look into the new one's sun.
        if let (Some(base), true) = (base, world.get_entity(entity).is_ok()) {
            restore(world, &base);
        }
        if let Some(mut driver) = world.get_resource_mut::<WeatherDriver>() {
            if driver.entity == Some(entity) {
                driver.reset();
            }
        }
    });
}
