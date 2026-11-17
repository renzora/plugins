//! The Culling probe: how many mesh entities survive frustum and distance
//! culling, how far from the camera they sit, and the distance-culling
//! settings.
//!
//! Every value comes from [`CullingDebugState`], which
//! `update_culling_debug_state` refreshes every `update_interval` seconds by
//! walking every `Mesh3d` entity. The panel this replaces had two sliders; a
//! report has no slider, so each becomes a pair of step buttons either side of
//! the current value, clamped to the slider's own range (10 to 2000 m for the
//! maximum distance, 0.5 to 1.0 for the fade start), with the value itself shown
//! as a stat in the slider's format.

use bevy::prelude::*;
use renzora::lang::t;
use renzora::probes::{ProbeDef, ProbeItem, ProbeLevel, ProbeReport};

use crate::state::CullingDebugState;

/// The slider ranges of the old panel. A step button clamps to these, so the
/// probe can never set a value the panel could not.
const MAX_DISTANCE_RANGE: (f32, f32) = (10.0, 2000.0);
const FADE_START_RANGE: (f32, f32) = (0.5, 1.0);

/// The distance buckets `update_culling_debug_state` fills, in order.
const BUCKET_LABELS: [&str; 5] = ["0-50m", "50-100m", "100-200m", "200-500m", "500m+"];

pub fn def() -> ProbeDef {
    ProbeDef {
        id: "culling_debug",
        title: "Culling",
        icon: "scissors",
        category: "Rendering",
        build,
        act: Some(act),
    }
}

/// Frustum-visible entities that distance culling did not then remove.
fn visible(s: &CullingDebugState) -> u32 {
    s.frustum_visible.saturating_sub(s.distance_culled)
}

/// `count` as a share of `total`, with a zero total reading as an empty bar
/// rather than a division by zero.
fn frac(count: u32, total: u32) -> f32 {
    (count as f32 / total.max(1) as f32).clamp(0.0, 1.0)
}

fn build(world: &mut World) -> ProbeReport {
    let mut report = ProbeReport::default();
    let Some(s) = world.get_resource::<CullingDebugState>() else {
        report.item(ProbeItem::note(
            "Culling statistics are not being collected in this process.",
            ProbeLevel::Muted,
        ));
        return report;
    };

    let vis = visible(s);
    let total = s.total_entities;

    report.section(t("cull.visible")).item(ProbeItem::figure(
        t("cull.visible"),
        vis.to_string(),
        format!("/ {} visible", total),
        ProbeLevel::Normal,
    ));
    // The panel hid this line with no mesh entities at all, where it would only
    // ever read 0%.
    if total > 0 {
        report.item(ProbeItem::note(
            format!("{}% of mesh entities visible", vis * 100 / total.max(1)),
            ProbeLevel::Muted,
        ));
    }

    // Breakdown. The levels stand in for the panel's bar colours: green for
    // visible, amber for frustum culled and fading, red for distance culled.
    report
        .section(t("cull.breakdown"))
        .item(ProbeItem::bar(t("cull.visible"), frac(vis, total), vis.to_string(), ProbeLevel::Good))
        .item(ProbeItem::bar(
            t("cull.frustum_culled"),
            frac(s.frustum_culled, total),
            s.frustum_culled.to_string(),
            ProbeLevel::Warn,
        ))
        .item(ProbeItem::bar(
            t("cull.distance_culled"),
            frac(s.distance_culled, total),
            s.distance_culled.to_string(),
            ProbeLevel::Bad,
        ));
    if s.distance_faded > 0 {
        report.item(ProbeItem::bar(
            t("cull.fading"),
            frac(s.distance_faded, total),
            s.distance_faded.to_string(),
            ProbeLevel::Warn,
        ));
    }

    // Distribution. Each bar is relative to the fullest bucket, not to the
    // total, so the shape of the spread reads even when one bucket dominates.
    report.section(t("cull.distance_distribution"));
    let max_bucket = s.distance_buckets.iter().copied().max().unwrap_or(1).max(1);
    for (i, label) in BUCKET_LABELS.iter().enumerate() {
        let count = s.distance_buckets[i];
        report.item(ProbeItem::bar(
            *label,
            count as f32 / max_bucket as f32,
            count.to_string(),
            ProbeLevel::Normal,
        ));
    }

    report
        .section(t("common.settings"))
        .item(ProbeItem::toggle("enabled", t("cull.enable_distance_culling"), s.enabled))
        .item(ProbeItem::stat(t("cull.max_distance"), format!("{:.0}m", s.max_distance)))
        .item(ProbeItem::button("max_distance:-100", "-100 m"))
        .item(ProbeItem::button("max_distance:-10", "-10 m"))
        .item(ProbeItem::button("max_distance:+10", "+10 m"))
        .item(ProbeItem::button("max_distance:+100", "+100 m"))
        .item(ProbeItem::stat(t("cull.fade_start"), format!("{:.2}", s.fade_start_fraction)))
        .item(ProbeItem::button("fade_start:-0.05", "-0.05"))
        .item(ProbeItem::button("fade_start:+0.05", "+0.05"));

    report
}

/// The controls. `enabled` is the checkbox; every other id is
/// `<field>:<signed step>`, so the step a button applies is written once, in
/// the report that offers it.
fn act(world: &mut World, id: &str, value: Option<bool>) {
    let Some(mut s) = world.get_resource_mut::<CullingDebugState>() else {
        return;
    };
    if id == "enabled" {
        s.enabled = value.unwrap_or(!s.enabled);
        return;
    }
    let Some((field, step)) = id.split_once(':') else {
        return;
    };
    let Ok(step) = step.parse::<f32>() else {
        return;
    };
    match field {
        "max_distance" => {
            s.max_distance = (s.max_distance + step).clamp(MAX_DISTANCE_RANGE.0, MAX_DISTANCE_RANGE.1);
        }
        "fade_start" => {
            s.fade_start_fraction =
                (s.fade_start_fraction + step).clamp(FADE_START_RANGE.0, FADE_START_RANGE.1);
        }
        _ => {}
    }
}
