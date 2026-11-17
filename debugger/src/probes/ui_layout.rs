//! UI Layout probe: where bevy_ui's per-frame cost actually goes.
//!
//! This exists because of a concrete misdiagnosis. Opening the inspector's
//! component sections cost about 3.3 ms a frame (60 fps down to 50), and the
//! obvious suspect was reactivity. It was not: the whole reactive layer
//! measured 0.23 ms. Opening a section does not add binding work, it adds
//! *visible nodes*, and bevy_ui charges for those whether or not anything about
//! them changed: `ui_layout_system` walks the tree unconditionally, and text
//! measurement runs for every visible label before it. So the numbers worth
//! watching are how many nodes are on screen and what laying them out costs.
//!
//! ## How the timings are taken
//!
//! Bevy exposes no per-system timing without a Tracy build, so [`register`]
//! brackets the UI pipeline with three timestamps around the public sets:
//!
//! ```text
//!   A   UiSystems::Prepare .. Propagate .. Content   B   Layout   C
//!       (content: text measurement, A to B)              (taffy, B to C)
//! ```
//!
//! The split is the actionable part: text-bound means fewer or cheaper labels,
//! taffy-bound means fewer nodes. The spans cover everything scheduled in those
//! sets, not only bevy's own systems, which is the honest thing to report since
//! a plugin's layout-time system costs the frame just as much.
//!
//! The data lives in [`UiLayoutStats`], moved here from the panel along with
//! the systems that fill it. They are deliberately not gated on the probe
//! being watched: the numbers are also published into Bevy's diagnostic store,
//! which the Tracy bridge plots, and a one-frame spike is exactly what a
//! readout someone is staring at misses. The filtered node counts are
//! O(entities), so they refresh every 30th frame.

// `bevy::platform::time::Instant`, never `std`'s: std's panics on wasm.
use bevy::platform::time::Instant;

use bevy::diagnostic::{Diagnostic, DiagnosticPath, Diagnostics, RegisterDiagnostic};
use bevy::prelude::*;
use bevy::ui::UiSystems;

use renzora::probes::{ProbeDef, ProbeItem, ProbeLevel, ProbeReport};

/// Live UI-pipeline cost. Read by this probe; available to any system.
#[derive(Resource, Default)]
pub struct UiLayoutStats {
    /// `Prepare` to `Content`: propagation and text measurement, ms.
    pub content_ms: f32,
    /// `Content` to `Layout`: taffy solving the tree, ms.
    pub layout_ms: f32,
    /// Nodes in the world, refreshed every 30 frames.
    pub nodes_total: usize,
    /// Nodes whose own `display` is `None`. They still cost a tree walk, but
    /// not text measurement, which is why collapsing a section helps.
    pub nodes_hidden: usize,
    /// Nodes carrying `Text`. Measurement cost scales with these, not with
    /// the raw node count: a panel of labels is dearer than a panel of boxes.
    pub text_nodes: usize,
    /// Of those, the ones not hidden: the population text measurement
    /// actually runs over, and the number that drops when a section collapses.
    pub text_nodes_visible: usize,
    /// Recent `content + layout` per frame, oldest first.
    pub history_ms: Vec<f32>,
    /// Internal: start of the current frame's UI pipeline.
    start: Option<Instant>,
    /// Internal: the `Content`/`Layout` boundary.
    mid: Option<Instant>,
    frame: u64,
}

impl UiLayoutStats {
    pub const HISTORY_LEN: usize = 240;

    pub fn total_ms(&self) -> f32 {
        self.content_ms + self.layout_ms
    }
}

/// `Prepare` to `Content`: propagation and text measurement, ms.
pub const CONTENT_MS: DiagnosticPath = DiagnosticPath::const_new("ui/content_ms");
/// `Content` to `Layout`: taffy solving the tree, ms.
pub const LAYOUT_MS: DiagnosticPath = DiagnosticPath::const_new("ui/layout_ms");
/// Live `Node` count.
pub const NODES_TOTAL: DiagnosticPath = DiagnosticPath::const_new("ui/nodes_total");
/// Nodes at `Display::None`. Against `nodes_total` this is the share of the UI
/// tree that exists but is never drawn, mostly widget chrome (carets, selection
/// highlights, slider rails, popovers) built eagerly and hidden for its whole
/// life. It is the ceiling on what lazy chrome could remove.
pub const NODES_HIDDEN: DiagnosticPath = DiagnosticPath::const_new("ui/nodes_hidden");
/// Live `Node` + `Text` count: what text measurement scales with.
pub const TEXT_NODES: DiagnosticPath = DiagnosticPath::const_new("ui/text_nodes");
/// Of those, the ones actually visible: the population text measurement runs
/// over.
pub const TEXT_NODES_VISIBLE: DiagnosticPath =
    DiagnosticPath::const_new("ui/text_nodes_visible");

/// Install the stats resource, the timing brackets, the node census and the
/// diagnostic publishing. The probe only reads what these produce.
pub fn register(app: &mut App) {
    app.init_resource::<UiLayoutStats>();
    app.add_systems(
        PostUpdate,
        (
            mark_start.before(UiSystems::Prepare),
            // `Content` is the last set before layout, so "after Content" is
            // the boundary between measurement and solving.
            mark_mid.after(UiSystems::Content).before(UiSystems::Layout),
            mark_end.after(UiSystems::Layout),
        ),
    );
    // Counting walks every node, so it runs last and throttles itself.
    app.add_systems(Last, count_nodes);
    for path in [
        &CONTENT_MS,
        &LAYOUT_MS,
        &NODES_TOTAL,
        &NODES_HIDDEN,
        &TEXT_NODES,
        &TEXT_NODES_VISIBLE,
    ] {
        app.register_diagnostic(Diagnostic::new(path.clone()));
    }
    app.add_systems(Last, publish_ui_layout_diagnostics.after(count_nodes));
}

/// Copy the pipeline split and the node counts into the diagnostic store.
///
/// The two totals come straight from the queries because `iter().len()` is
/// answered from archetype metadata without touching an entity, so it is free
/// every frame. The two filtered counts come from [`UiLayoutStats`], refreshed
/// by [`count_nodes`] every 30th frame, because those are O(entities).
fn publish_ui_layout_diagnostics(
    mut diags: Diagnostics,
    stats: Res<UiLayoutStats>,
    nodes: Query<&Node>,
    texts: Query<&Node, With<Text>>,
) {
    diags.add_measurement(&CONTENT_MS, || stats.content_ms as f64);
    diags.add_measurement(&LAYOUT_MS, || stats.layout_ms as f64);
    diags.add_measurement(&NODES_TOTAL, || nodes.iter().len() as f64);
    diags.add_measurement(&TEXT_NODES, || texts.iter().len() as f64);
    diags.add_measurement(&NODES_HIDDEN, || stats.nodes_hidden as f64);
    diags.add_measurement(&TEXT_NODES_VISIBLE, || stats.text_nodes_visible as f64);
}

fn mark_start(mut stats: ResMut<UiLayoutStats>) {
    // Deliberately not `bypass_change_detection`: a dependency-tracked binding
    // reading this resource would never go dirty and freeze on its initial
    // zeroes. Any ember UI that still reads these numbers needs the tick.
    let s = &mut *stats;
    s.frame = s.frame.wrapping_add(1);
    s.start = Some(Instant::now());
    s.mid = None;
}

fn mark_mid(mut stats: ResMut<UiLayoutStats>) {
    stats.mid = Some(Instant::now());
}

fn mark_end(mut stats: ResMut<UiLayoutStats>) {
    let s = &mut *stats;
    let now = Instant::now();
    let (Some(start), Some(mid)) = (s.start, s.mid) else {
        return;
    };
    s.content_ms = mid.duration_since(start).as_secs_f32() * 1e3;
    s.layout_ms = now.duration_since(mid).as_secs_f32() * 1e3;

    let total = s.content_ms + s.layout_ms;
    if s.history_ms.len() >= UiLayoutStats::HISTORY_LEN {
        s.history_ms.remove(0);
    }
    s.history_ms.push(total);
}

/// Refresh the node counts every 30th frame. Two of the four are
/// `filter().count()` and so O(entities); over a few thousand nodes the
/// throttle amortises that to well under a hundred visits a frame.
fn count_nodes(
    mut stats: ResMut<UiLayoutStats>,
    nodes: Query<&Node>,
    texts: Query<&Node, With<Text>>,
) {
    if !stats.frame.is_multiple_of(30) {
        return;
    }
    let s = &mut *stats;
    s.nodes_total = nodes.iter().len();
    s.nodes_hidden = nodes.iter().filter(|n| n.display == Display::None).count();
    s.text_nodes = texts.iter().len();
    s.text_nodes_visible = texts.iter().filter(|n| n.display != Display::None).count();
}

pub fn def() -> ProbeDef {
    ProbeDef {
        id: "ui_layout",
        title: "UI Layout",
        icon: "layout",
        category: "Editor UI",
        build,
        act: None,
    }
}

/// Green under a millisecond, amber to four, red past that: the same scale
/// UI Reactivity uses, so the two are directly comparable at a glance. That
/// comparison is the point; it tells you which of the two is worth optimising.
fn cost_level(ms: f32) -> ProbeLevel {
    if ms > 4.0 {
        ProbeLevel::Bad
    } else if ms > 1.0 {
        ProbeLevel::Warn
    } else {
        ProbeLevel::Good
    }
}

fn build(world: &mut World) -> ProbeReport {
    let mut report = ProbeReport::default();
    let Some(s) = world.get_resource::<UiLayoutStats>() else {
        report.item(ProbeItem::note(
            "UiLayoutStats is not present: the UI layout timing systems are not installed.",
            ProbeLevel::Muted,
        ));
        return report;
    };

    report
        .section("Overview")
        .item(ProbeItem::figure(
            "UI layout",
            format!("{:.2}", s.total_ms()),
            "ms/frame",
            cost_level(s.total_ms()),
        ))
        .item(ProbeItem::figure("ui nodes", s.nodes_total.to_string(), "", ProbeLevel::Normal))
        .item(ProbeItem::series("Layout history", s.history_ms.clone(), "ms"));

    // Text measurement against taffy: the two halves have different fixes,
    // so the split is the actionable part.
    report
        .section("Where The Time Goes")
        .item(ProbeItem::stat("Content (text measure)", format!("{:.2} ms", s.content_ms)))
        .item(ProbeItem::stat("Layout (taffy)", format!("{:.2} ms", s.layout_ms)));

    report
        .section("Node Census")
        .item(ProbeItem::stat("Nodes total", s.nodes_total))
        .item(ProbeItem::stat("Hidden (display: none)", s.nodes_hidden))
        .item(ProbeItem::stat("Text nodes", s.text_nodes))
        // The ratio that predicts the cost: measurement scales with visible
        // text, not with the raw node count.
        .item(ProbeItem::stat("Text nodes visible", s.text_nodes_visible))
        .item(ProbeItem::note(
            "Counts refresh every 30 frames. Compare UI layout ms/frame against UI Reactivity's \
             recompute ms/frame: whichever is larger is the one worth optimising. Opening a panel \
             adds nodes, not bindings.",
            ProbeLevel::Muted,
        ));

    report
}
