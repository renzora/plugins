//! The Render Stats probe: total GPU time for the frame with its history, and
//! the scene's geometry totals (mesh instances, triangles, vertices).
//!
//! Everything comes from [`RenderStats`], filled by `update_render_stats`.
//! GPU time is the sum of the top-level render-pass timestamps, so on a backend
//! without timestamp queries it reads "n/a" rather than a guess derived from
//! frame time. The geometry counts are taken from the scene's `Mesh3d`
//! entities, not from what the renderer actually drew: culling, batching and
//! instancing are not reflected in them.

use bevy::prelude::*;
use renzora::lang::t;
use renzora::probes::{ProbeDef, ProbeItem, ProbeLevel, ProbeReport};

use crate::state::RenderStats;

pub fn def() -> ProbeDef {
    ProbeDef {
        id: "render_stats",
        title: "Render Stats",
        icon: "chart-bar",
        category: "Rendering",
        build,
        act: None,
    }
}

fn build(world: &mut World) -> ProbeReport {
    let mut report = ProbeReport::default();
    // `enabled` turns true on the first refresh; before it the numbers are
    // the defaults, and zeros would read as an empty scene.
    let Some(r) = world.get_resource::<RenderStats>().filter(|r| r.enabled) else {
        report.item(ProbeItem::note(t("render.collecting"), ProbeLevel::Muted));
        return report;
    };

    let (value, level) = if r.gpu_timing_available {
        (format!("{:.2}", r.gpu_time_ms), gpu_time_level(r.gpu_time_ms))
    } else {
        ("n/a".to_string(), ProbeLevel::Muted)
    };
    report
        .section(t("render.gpu_timing"))
        .item(ProbeItem::figure(t("render.gpu_timing"), value, "ms GPU", level))
        .item(ProbeItem::series(
            t("render.gpu_timing"),
            r.gpu_time_history.clone(),
            "ms",
        ));

    report
        .section(t("render.scene_geometry"))
        .item(ProbeItem::stat(t("render.mesh_instances"), format_number(r.mesh_instances)))
        .item(ProbeItem::stat(t("perf.triangles"), format_number(r.triangles)))
        .item(ProbeItem::stat(t("perf.vertices"), format_number(r.vertices)));

    report
}

/// GPU time is judged against half the 60 fps budget: the CPU needs the rest
/// of the frame, so a GPU that alone takes 16 ms already misses the target.
fn gpu_time_level(ms: f32) -> ProbeLevel {
    if ms <= 8.0 {
        ProbeLevel::Good
    } else if ms <= 16.67 {
        ProbeLevel::Warn
    } else {
        ProbeLevel::Bad
    }
}

fn format_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}
