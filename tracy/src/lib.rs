//! Tracy profiler bridge.
//!
//! Streams the engine's diagnostics to a running [Tracy] profiler over Tracy's
//! native protocol: one frame mark per app frame, plus every measurement the
//! engine publishes — frame time, FPS, entity count, per-render-pass GPU and CPU
//! times, process CPU and memory — as a named Tracy plot.
//!
//! [Tracy]: https://github.com/wolfpld/tracy
//!
//! ## Which Tracy
//!
//! **0.14.x**, and it is not a suggestion: Tracy refuses a connection across
//! protocol versions with "incompatible protocol version" and nothing else to go
//! on. `Cargo.toml` pins `tracy-client-sys`, which is the crate that vendors the
//! C++ client and therefore decides the protocol, so the answer is a property of
//! this plugin's release rather than of the day somebody installed it.
//!
//! Connect with the profiler GUI, or capture headlessly:
//! `tracy-capture -o out.tracy -s 10 -f`, then read the plots back with
//! `tracy-csvexport -u -p out.tracy`. Plain `tracy-csvexport` reports zones, and
//! there are none here, which is the next section.
//!
//! ## What this is not
//!
//! **A flame graph.** There are no CPU zones here and there cannot be. Bevy's
//! per-system spans are `#[cfg(feature = "trace")]` inside `bevy_ecs` and its
//! GPU-timestamp zones are `#[cfg(feature = "tracing-tracy")]` inside
//! `bevy_render`: instrumentation that was compiled out does not exist to be
//! switched on, and no plugin loaded at run time can put it back. Tracy's Flame
//! Graph and zone Statistics windows stay empty, and `tracy-csvexport -e`/`-g`
//! return nothing.
//!
//! Everything this streams is a *plot*. That is genuinely enough to find a
//! bottleneck — expand the `render/*/elapsed_gpu` rows and read the maxima — but
//! it will not tell you which system is eating the frame. For that, build the
//! engine with `cargo renzora profile`, which compiles the instrumentation in.
//! Turn this plugin OFF in such a build: Bevy frame-marks there itself, and two
//! marks per frame halves every frame time Tracy reports.
//!
//! ## Why it is a plugin rather than part of the engine
//!
//! It used to be `crates/renzora_tracy`, an rlib linked into the editor bundle,
//! which meant every editor build compiled `tracy-client`'s C sources and
//! carried the client whether or not anyone ever profiled. Here, deleting the
//! library deletes the feature: no listener socket, no C build in the engine
//! tree, nothing dormant to reason about.
//!
//! ## The dormant state, precisely
//!
//! Off, nothing is created: no Tracy client, no socket, no plot names, and the
//! per-frame system returns on its first line. On, the client starts and the
//! bridge feeds it.
//!
//! **Turning it off again stops the feeding but does not shut the client down.**
//! That is a property of `tracy-client`, not a shortcut taken here: there is no
//! `impl Drop for Client` in the crate — shutdown exists only behind its
//! `manual-lifetime` feature, whose start/stop pairing rules are delicate enough
//! that arming them for a profiler nobody has connected to would be the more
//! fragile choice. It costs an idle listener socket and nothing else, because
//! `ondemand` (see `Cargo.toml`) means the client accumulates no trace data
//! until a profiler actually attaches.

use std::collections::HashMap;

use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;
use renzora_ember::font::{ui_font, EmberFonts};
use renzora_ember::reactive::tracked::bind_2way;
use renzora_ember::reactive::Rx;
use renzora_ember::settings_sections::RegisterSettingsSection;
use renzora_ember::theme::*;
use renzora_ember::widgets::toggle_switch;
use tracy_client::{Client, PlotName};

/// A group of diagnostics the user can turn off as a unit.
///
/// Grouped rather than listed one-plot-at-a-time because the render paths are
/// per-pass and open-ended — a scene with more passes has more of them, and a
/// list of forty checkboxes that changes as you load a level is not a settings
/// panel. The categories below are stable regardless of what the scene does.
struct Category {
    /// Stable key written to the config. **Never rename one**: an old config
    /// naming a key this build does not know falls back to that category's
    /// default, silently turning a plot the user had off back on.
    key: &'static str,
    label: &'static str,
    /// What it covers, shown under the label in Settings. The old markup panel
    /// had nowhere to put this, so the labels carried it in parentheses.
    description: &'static str,
    /// Whether a path belongs to this category. Order matters — [`categorise`]
    /// takes the first match, so the specific render predicates must precede
    /// any general one.
    matches: fn(&str) -> bool,
    default: bool,
}

/// The category table. Append only; see [`Category::key`].
///
/// Everything defaults ON except shader invocation counts, which are the one
/// group that is usually noise: they are raw counters in the millions, so Tracy
/// autoscales them and they sit on the plot list crowding out the millisecond
/// timings that a frame-budget question actually turns on.
const CATEGORIES: &[Category] = &[
    Category {
        key: "frame",
        label: "Frame",
        description: "FPS, frame time, frame count",
        matches: |p| matches!(p, "fps" | "frame_time" | "frame_count"),
        default: true,
    },
    Category {
        key: "entities",
        label: "Entity count",
        description: "How many entities the world holds",
        matches: |p| p == "entity_count",
        default: true,
    },
    Category {
        key: "system",
        label: "CPU and memory",
        description: "Process and system-wide usage",
        matches: |p| p.starts_with("system/") || p.starts_with("process/"),
        default: true,
    },
    Category {
        key: "render_gpu",
        label: "Render passes, GPU time",
        description: "Per-pass GPU milliseconds. Usually where a frame goes",
        matches: |p| p.starts_with("render/") && p.ends_with("/elapsed_gpu"),
        default: true,
    },
    Category {
        key: "render_cpu",
        label: "Render passes, CPU time",
        description: "Per-pass CPU milliseconds",
        matches: |p| p.starts_with("render/") && p.ends_with("/elapsed_cpu"),
        default: true,
    },
    // The GPU pipeline-statistics family. `_primitives_out` is in here and not
    // in `other` because it is the same kind of number as the invocation
    // counters beside it — a real capture carries `clipper_invocations` and
    // `clipper_primitives_out` for every pass, and a filter that took one and
    // left the other would still leave the plot list crowded while claiming to
    // have cleared it. Matched by suffix rather than listed, since the set is
    // per-pass and grows with the scene.
    Category {
        key: "invocations",
        label: "Shader and pipeline counters",
        description: "Raw counts in the millions. Off by default: Tracy autoscales them and they crowd out the timings",
        matches: |p| p.ends_with("_invocations") || p.ends_with("_primitives_out"),
        default: false,
    },
    // The catch-all, and the reason it exists: the engine's diagnostic set is
    // open — any crate or other plugin may register its own path, and several
    // do. Without this they would match no category and a filter built from the
    // list above would silently drop them, which is indistinguishable from the
    // engine having stopped measuring. Anything unrecognised is shown.
    Category {
        key: "other",
        label: "Other diagnostics",
        description: "Anything a crate or another plugin registers",
        matches: |_| true,
        default: true,
    },
];

/// Which category a path falls into. Always succeeds — the last entry matches
/// everything.
fn categorise(path: &str) -> usize {
    CATEGORIES
        .iter()
        .position(|c| (c.matches)(path))
        .unwrap_or(CATEGORIES.len() - 1)
}

/// Everything the bridge owns.
///
/// An ordinary resource. It was a `static Mutex<Option<State>>` under the old
/// C-ABI, because the settings action handler ran on the editor's UI systems
/// with no `SystemCall` to reach a plugin resource through. A native plugin's
/// settings bindings get `&mut World`, so the lock and the `Option` both go.
#[derive(Resource)]
struct Tracy {
    /// The user's opt-in, mirrored to disk on every change.
    enabled: bool,
    /// Per-category enables, parallel to [`CATEGORIES`].
    categories: Vec<bool>,
    /// Held only while streaming. Starting it is what opens Tracy's listener.
    client: Option<Client>,
    /// Which category each path fell into, so the string matching in
    /// [`categorise`] runs once per distinct path rather than once per path per
    /// frame. At a few dozen paths either would be fine; this is a plot bridge
    /// whose own cost should not show up in what it is plotting.
    category_of: HashMap<String, usize>,
    /// Tracy's `PlotName` requires `'static` storage but diagnostic paths are
    /// dynamic strings, so each distinct path is leaked once and cached. The set
    /// is small (a few dozen) and stops growing after the first frames, so the
    /// leak is bounded — this is `new_leak`'s intended use, not an oversight.
    ///
    /// **Populated only when a path is actually plotted**, which is why it is
    /// separate from `category_of` rather than a second field in it. A category
    /// the user never turns on then leaks nothing at all — and leaking eagerly
    /// bought nothing anyway, since re-enabling a category sees every one of its
    /// paths again on the very next frame.
    plots: HashMap<String, PlotName>,
}

impl Default for Tracy {
    fn default() -> Self {
        let (enabled, categories) = load_config();
        Self {
            enabled,
            categories,
            client: None,
            category_of: HashMap::new(),
            plots: HashMap::new(),
        }
    }
}

impl Tracy {
    /// Grow the enables to match [`CATEGORIES`], filling with each category's
    /// OWN default.
    ///
    /// A config written before a category was appended leaves the vector short,
    /// and a switch has already been drawn for it. Blanket `true` was wrong in
    /// the one direction that matters: it would switch on the counter group,
    /// whose whole reason for defaulting off is that it buries the millisecond
    /// timings, and it would do so silently, as a side effect of the user
    /// touching some unrelated switch.
    fn grow(&mut self) {
        while self.categories.len() < CATEGORIES.len() {
            self.categories.push(CATEGORIES[self.categories.len()].default);
        }
    }

    fn category_on(&self, i: usize) -> bool {
        self.categories
            .get(i)
            .copied()
            .unwrap_or_else(|| CATEGORIES[i].default)
    }
}

// ── The bridge ───────────────────────────────────────────────────────────────

/// Push this frame's measurements as plots, then close the frame on Tracy's
/// timeline.
///
/// Runs in `Last` so the diagnostics it reads are this frame's finished numbers
/// rather than a mix of this frame's and the previous one's, and so the frame
/// mark lands after everything that contributed to the frame.
fn pump(mut state: ResMut<Tracy>, diagnostics: Res<DiagnosticsStore>) {
    if !state.enabled {
        return;
    }
    // Started here rather than at init so the toggle can turn profiling ON
    // without a restart. `Client::start()` is idempotent — it returns a handle
    // to the running client if there is one — so calling it on the first enabled
    // frame costs a check thereafter.
    if state.client.is_none() {
        state.client = Some(Client::start());
    }

    // Gathered before anything is plotted. Resolving a path's category and plot
    // name needs `&mut state` (both are memoised) while `plot` needs the client
    // out of that same state, and collecting first keeps the two borrows apart
    // without cloning the client per point.
    let mut points: Vec<(PlotName, f64)> = Vec::new();
    for diagnostic in diagnostics.iter() {
        // `None` is the normal state for a diagnostic that has registered but
        // not yet been sampled. Tracy will happily accept a NaN and then draw a
        // plot with a hole in it, which reads as "the engine stopped measuring"
        // rather than "this had not started yet".
        let Some(value) = diagnostic.smoothed() else {
            continue;
        };
        if !value.is_finite() {
            continue;
        }
        let path = diagnostic.path().as_str();

        let category = match state.category_of.get(path) {
            Some(c) => *c,
            None => {
                let c = categorise(path);
                state.category_of.insert(path.to_string(), c);
                c
            }
        };
        // Checked BEFORE the plot name is created. Emitting a value is what
        // makes a row appear in Tracy, and Tracy's protocol has no message that
        // removes one — so a category left off never puts a row on the timeline
        // in the first place, which is the only point at which this decision can
        // still be made.
        if !state.category_on(category) {
            continue;
        }
        let name = match state.plots.get(path) {
            Some(name) => *name,
            None => {
                let name = PlotName::new_leak(path.to_string());
                state.plots.insert(path.to_string(), name);
                name
            }
        };
        points.push((name, value));
    }

    let Some(client) = state.client.as_ref() else {
        return;
    };
    for (name, value) in points {
        // The smoothed value: raw frame time is noisy enough that a plot of it
        // is unreadable, and Tracy does its own aggregation on top.
        client.plot(name, value);
    }
    client.frame_mark();
}

// ── Settings ─────────────────────────────────────────────────────────────────

/// The Tracy section on the Settings overlay's Plugins tab.
///
/// Built once per overlay open. The switches carry two-way bindings, so they
/// read live state rather than the snapshot this call could take.
fn build(commands: &mut Commands, fonts: &EmberFonts) -> Entity {
    let root = commands
        .spawn(Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(10.0),
            ..default()
        })
        .id();

    switch_row(
        commands,
        fonts,
        root,
        "Enable Tracy",
        "Streams to a running Tracy server on 127.0.0.1:8086. Takes effect immediately. Plots only: there is no flame graph without a profiling build.",
        |rx| rx.get_resource::<Tracy>().is_some_and(|t| t.enabled),
        |w, on| {
            let Some(mut t) = w.get_resource_mut::<Tracy>() else {
                return;
            };
            t.enabled = *on;
            save_config(t.enabled, &t.categories);
        },
    );

    let header = commands
        .spawn((
            Text::new("Plots".to_string()),
            ui_font(&fonts.ui, 12.5),
            TextColor(rgb(text_primary())),
            Node { margin: UiRect::top(Val::Px(6.0)), ..default() },
        ))
        .id();
    // Says what turning one off does, because the answer is not the obvious one
    // and the difference is visible on screen. Tracy's protocol has no message
    // that removes a plot — the complete set is PlotData{Int,Float,Double},
    // PlotConfig and PlotName — so a row already on the timeline stays there
    // showing a frozen line. Reconnecting drops it, because the on-demand client
    // discards plot data while nothing is attached and replays only GPU
    // contexts, lock names and thread names on connect.
    let note = commands
        .spawn((
            Text::new(
                "Turning one off stops it immediately. Rows already on Tracy's timeline stay \
                 until you reconnect, since its protocol has no way to remove a plot. Reconnect, \
                 or set these before connecting, for a clean capture."
                    .to_string(),
            ),
            ui_font(&fonts.ui, 9.5),
            TextColor(rgb(text_muted())),
        ))
        .id();
    commands.entity(root).add_children(&[header, note]);

    // The category switches stay rendered while Tracy is off rather than being
    // hidden behind it: choosing what to capture before starting a capture is
    // the normal order, and a settings panel that empties itself when you turn
    // the feature off is a worse way to say "these do nothing right now".
    for (i, cat) in CATEGORIES.iter().enumerate() {
        switch_row(
            commands,
            fonts,
            root,
            cat.label,
            cat.description,
            move |rx| rx.get_resource::<Tracy>().is_some_and(|t| t.category_on(i)),
            move |w, on| {
                let Some(mut t) = w.get_resource_mut::<Tracy>() else {
                    return;
                };
                t.grow();
                t.categories[i] = *on;
                save_config(t.enabled, &t.categories);
            },
        );
    }

    root
}

/// One labelled switch, bound both ways to whatever `get` and `set` name.
fn switch_row(
    commands: &mut Commands,
    fonts: &EmberFonts,
    parent: Entity,
    label: &str,
    description: &str,
    // Read through the `Rx` rather than off the world, so the switch subscribes
    // to what it displays: a two-way binding whose getter is untracked finds its
    // data unchanged when the user's edit lands and drops the edit.
    get: impl for<'w> Fn(&Rx<'w>) -> bool + Send + Sync + 'static,
    set: impl Fn(&mut World, &bool) + Send + Sync + 'static,
) {
    let row = commands
        .spawn(Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(10.0),
            ..default()
        })
        .id();
    let col = commands
        .spawn(Node { flex_direction: FlexDirection::Column, flex_grow: 1.0, ..default() })
        .id();
    let l = commands
        .spawn((
            Text::new(label.to_string()),
            ui_font(&fonts.ui, 11.5),
            TextColor(rgb(text_primary())),
        ))
        .id();
    let d = commands
        .spawn((
            Text::new(description.to_string()),
            ui_font(&fonts.ui, 9.5),
            TextColor(rgb(text_muted())),
        ))
        .id();
    commands.entity(col).add_children(&[l, d]);

    // `true` is a placeholder: `bind_2way` seeds the widget from `get` before it
    // is shown, so the value passed here is never what the user sees.
    let sw = toggle_switch(commands, true);
    bind_2way::<bool, _, _>(commands, sw, get, set);

    commands.entity(row).add_children(&[col, sw]);
    commands.entity(parent).add_child(row);
}

// ── Persistence ──────────────────────────────────────────────────────────────

/// `%APPDATA%/renzora/tracy.json` on Windows, `~/.config/renzora/tracy.json`
/// elsewhere — the same path and format `renzora_tracy` used, so an existing
/// opt-in survives the move to a plugin.
fn config_path() -> Option<std::path::PathBuf> {
    let base = if cfg!(windows) {
        std::path::PathBuf::from(std::env::var_os("APPDATA")?)
    } else {
        std::path::PathBuf::from(std::env::var_os("HOME")?).join(".config")
    };
    Some(base.join("renzora").join("tracy.json"))
}

/// Read one `"key": true|false` out of the flat config.
///
/// Hand-parsed rather than with serde, because keeping this plugin's
/// dependencies to the engine plus `tracy-client` is the design and a handful of
/// booleans is not worth breaking it for. The file this writes is flat, one key
/// per line, so finding the key and reading the next word is the whole grammar.
fn read_flag(text: &str, key: &str, default: bool) -> bool {
    let needle = format!("\"{key}\"");
    match text
        .split_once(&needle)
        .and_then(|(_, rest)| rest.split_once(':'))
    {
        Some((_, v)) => v.trim_start().starts_with("true"),
        // Absent, not false. A config written before a category existed must
        // give that category its default rather than silently off — which for
        // the render timings would mean a profiler that came back from an
        // upgrade plotting nothing, with a settings panel insisting it was on.
        None => default,
    }
}

/// The opt-in and the per-category enables. A missing or unreadable file reads
/// as "off, with default categories" — defaulting a profiler to ON because its
/// config could not be parsed is the wrong direction to fail in.
fn load_config() -> (bool, Vec<bool>) {
    let defaults = || CATEGORIES.iter().map(|c| c.default).collect::<Vec<_>>();
    let Some(path) = config_path() else {
        return (false, defaults());
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return (false, defaults());
    };
    (
        read_flag(&text, "enabled", false),
        CATEGORIES
            .iter()
            .map(|c| read_flag(&text, c.key, c.default))
            .collect(),
    )
}

fn save_config(enabled: bool, categories: &[bool]) {
    let Some(path) = config_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut json = format!("{{\n  \"enabled\": {enabled}");
    for (i, cat) in CATEGORIES.iter().enumerate() {
        let on = categories.get(i).copied().unwrap_or(cat.default);
        json.push_str(&format!(",\n  \"{}\": {on}", cat.key));
    }
    json.push_str("\n}\n");
    let _ = std::fs::write(path, json);
}

// ── Plugin ───────────────────────────────────────────────────────────────────

pub struct TracyPlugin;

impl Plugin for TracyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tracy>();
        app.register_settings_section("tracy", "Tracy Profiler", "pulse", build);
        app.add_systems(Last, pump);
    }
}

renzora::plugin!(TracyPlugin, Editor);

#[cfg(test)]
mod tests {
    use super::*;

    /// Rebuild the config text and read every flag back. The round trip is the
    /// only thing keeping the hand-rolled parser honest against the writer.
    fn round_trip(enabled: bool, cats: &[bool]) -> (bool, Vec<bool>) {
        let mut json = format!("{{\n  \"enabled\": {enabled}");
        for (i, cat) in CATEGORIES.iter().enumerate() {
            let on = cats.get(i).copied().unwrap_or(cat.default);
            json.push_str(&format!(",\n  \"{}\": {on}", cat.key));
        }
        json.push_str("\n}\n");
        (
            read_flag(&json, "enabled", false),
            CATEGORIES
                .iter()
                .map(|c| read_flag(&json, c.key, c.default))
                .collect(),
        )
    }

    #[test]
    fn round_trips_every_flag() {
        let all_on = vec![true; CATEGORIES.len()];
        let all_off = vec![false; CATEGORIES.len()];
        assert_eq!(round_trip(true, &all_on), (true, all_on.clone()));
        assert_eq!(round_trip(false, &all_off), (false, all_off));
        // Mixed, so a writer that emitted a constant would fail here.
        let mixed: Vec<bool> = (0..CATEGORIES.len()).map(|i| i % 2 == 0).collect();
        assert_eq!(round_trip(true, &mixed), (true, mixed));
    }

    /// Anything unparseable is off. A profiler that armed itself because a
    /// config file was truncated would be a genuinely unpleasant surprise.
    #[test]
    fn garbage_reads_as_off() {
        for text in ["", "{}", "not json", "{\"enabled\":", "{\"enabled\": maybe}"] {
            assert!(!read_flag(text, "enabled", false), "{text:?} should be off");
        }
    }

    /// A key the file does not mention takes its DEFAULT, not `false`. This is
    /// the upgrade path: a config written before a category existed must not
    /// silently turn that category off, or a profiler that came back from an
    /// upgrade would plot nothing while its panel insisted everything was on.
    #[test]
    fn a_missing_category_takes_its_default() {
        let old = "{\n  \"enabled\": true\n}\n";
        for cat in CATEGORIES {
            assert_eq!(
                read_flag(old, cat.key, cat.default),
                cat.default,
                "{} should fall back to its default",
                cat.key
            );
        }
    }

    /// Categorisation is first-match, so the specific render predicates must win
    /// over the catch-all — and each path must land where the label claims.
    #[test]
    fn paths_land_in_the_right_category() {
        let key = |p: &str| CATEGORIES[categorise(p)].key;
        assert_eq!(key("fps"), "frame");
        assert_eq!(key("frame_time"), "frame");
        assert_eq!(key("entity_count"), "entities");
        assert_eq!(key("system/cpu_usage"), "system");
        assert_eq!(key("process/mem_usage"), "system");
        assert_eq!(key("render/main_opaque_pass_3d/elapsed_gpu"), "render_gpu");
        assert_eq!(key("render/main_opaque_pass_3d/elapsed_cpu"), "render_cpu");
        // Every pipeline-statistics suffix a real capture actually carries,
        // taken from one: the first version of this matched only
        // `_invocations` and let five `_primitives_out` rows through into
        // `other`, where they were shown despite the group being off.
        for p in [
            "render/main_opaque_pass_3d/vertex_shader_invocations",
            "render/ui/fragment_shader_invocations",
            "render/taa/clipper_invocations",
            "render/bin_unpacking/compute_shader_invocations",
            "render/ui/clipper_primitives_out",
            "render/early prepass/clipper_primitives_out",
        ] {
            assert_eq!(key(p), "invocations", "{p} escaped the counter group");
        }
    }

    /// An unknown path is SHOWN, not dropped. The engine's diagnostic set is
    /// open — any crate may register a path — and a filter that silently
    /// swallowed them would be indistinguishable from the engine having stopped
    /// measuring.
    #[test]
    fn an_unknown_path_falls_through_to_other() {
        assert_eq!(CATEGORIES[categorise("my_plugin/widgets_drawn")].key, "other");
        assert!(CATEGORIES.last().unwrap().default, "`other` must default on");
    }

    /// Growing a short config must use each category's own default. Filling with
    /// a blanket `true` would switch the counter group on behind the user's
    /// back, as a side effect of touching some unrelated switch.
    ///
    /// Exercises [`Tracy::grow`] itself, which is what the settings bindings
    /// call. The old version of this test reimplemented the loop and so could
    /// only ever agree with itself.
    #[test]
    fn growing_a_short_config_uses_defaults() {
        let mut t = Tracy {
            enabled: true,
            categories: vec![true],
            client: None,
            category_of: HashMap::new(),
            plots: HashMap::new(),
        };
        t.grow();
        assert_eq!(t.categories.len(), CATEGORIES.len());
        for (i, cat) in CATEGORIES.iter().enumerate().skip(1) {
            assert_eq!(t.categories[i], cat.default, "{} grew to the wrong value", cat.key);
        }
        assert!(!t.category_on(CATEGORIES.iter().position(|c| c.key == "invocations").unwrap()));
    }

    /// A category read past the end of a short config takes its default rather
    /// than panicking or reading as off. The settings switches call this on
    /// every rebuild, before anything has grown the vector.
    #[test]
    fn a_category_past_the_end_reads_its_default() {
        let t = Tracy {
            enabled: false,
            categories: Vec::new(),
            client: None,
            category_of: HashMap::new(),
            plots: HashMap::new(),
        };
        for (i, cat) in CATEGORIES.iter().enumerate() {
            assert_eq!(t.category_on(i), cat.default, "{} read wrong", cat.key);
        }
    }

    /// Category keys are the config's field names, so a duplicate would make one
    /// silently shadow the other on load.
    #[test]
    fn category_keys_are_unique() {
        for (i, a) in CATEGORIES.iter().enumerate() {
            for b in &CATEGORIES[i + 1..] {
                assert_ne!(a.key, b.key, "duplicate category key");
            }
        }
    }
}
