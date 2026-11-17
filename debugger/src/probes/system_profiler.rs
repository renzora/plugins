//! The System probe: frame time against the 60 and 30 fps budgets, frame-rate
//! statistics, the scene's render totals, and the frame's GPU time broken down
//! by top-level render pass.
//!
//! It reads three snapshots and gathers nothing itself. [`DiagnosticsState`]
//! holds frame time, frame rate and the entity count; [`RenderStats`] the
//! geometry totals and GPU time, and its section is left out until that
//! resource has had a first refresh; [`SystemTimingState`] the per-pass GPU
//! timings (real `RenderDiagnosticsPlugin` timestamps, sorted slowest first)
//! with, for a pass an attribution rule covers, how many entities drive it.

use bevy::prelude::*;
use renzora::lang::t;
use renzora::probes::{ProbeDef, ProbeItem, ProbeLevel, ProbeReport};

use crate::state::{DiagnosticsState, RenderStats, SystemTimingState};

pub fn def() -> ProbeDef {
    ProbeDef {
        id: "system_profiler",
        title: "System",
        icon: "cpu",
        category: "Frame",
        build,
        act: None,
    }
}

fn build(world: &mut World) -> ProbeReport {
    let mut report = ProbeReport::default();
    let Some(d) = world.get_resource::<DiagnosticsState>() else {
        report.item(ProbeItem::note(
            "Diagnostics are not being collected in this process.",
            ProbeLevel::Muted,
        ));
        return report;
    };

    // Frame time.
    let ms = d.frame_time_ms as f32;
    let (status, status_level) = frame_status(ms);
    report
        .section(t("perf.frame_time"))
        .item(ProbeItem::figure(
            t("perf.frame_time"),
            format!("{:.2}", ms),
            "ms / frame",
            frame_time_level(ms),
        ))
        .item(ProbeItem::note(status, status_level))
        .item(ProbeItem::series(
            t("perf.frame_time"),
            d.frame_time_history.iter().copied().collect(),
            "ms",
        ));

    // FPS statistics.
    let fps = d.fps as f32;
    report
        .section(t("perf.fps_statistics"))
        .item(ProbeItem::figure("FPS", format!("{:.0}", fps), "fps", fps_level(fps)))
        .item(ProbeItem::stat(t("perf.avg"), format!("{:.0}", d.avg_fps())))
        .item(ProbeItem::stat(t("perf.min"), format!("{:.0}", d.min_fps())))
        .item(ProbeItem::stat(t("perf.max"), format!("{:.0}", d.max_fps())))
        .item(ProbeItem::stat(t("perf.entities"), d.entity_count.to_string()));

    // Render stats, only once they have been gathered: before that every
    // field is its default and the zeros would describe an empty scene.
    if let Some(r) = world.get_resource::<RenderStats>().filter(|r| r.enabled) {
        let gpu = if r.gpu_timing_available {
            format!("{:.2}ms", r.gpu_time_ms)
        } else {
            "n/a".to_string()
        };
        report
            .section(t("panel.render_stats"))
            .item(ProbeItem::stat(t("perf.instances"), r.mesh_instances.to_string()))
            .item(ProbeItem::stat(t("perf.triangles"), format_count(r.triangles)))
            .item(ProbeItem::stat(t("perf.vertices"), format_count(r.vertices)))
            .item(ProbeItem::stat(t("perf.gpu_time"), gpu));
    }

    // GPU pass breakdown. Each bar is the pass's share of the summed pass
    // time, which is the frame's GPU total because only the outermost spans
    // are kept.
    report.section(t("perf.gpu_pass_breakdown"));
    let timing = world.get_resource::<SystemTimingState>();
    let timings = timing.map(|state| state.schedule_timings.as_slice()).unwrap_or(&[]);
    if timings.is_empty() {
        report.item(ProbeItem::note(t("perf.no_timing_data"), ProbeLevel::Muted));
    } else {
        let total: f32 = timings.iter().map(|s| s.time_ms).sum();
        for s in timings {
            let ratio = if total > 0.0 { s.time_ms / total } else { 0.0 };
            report.item(ProbeItem::bar(
                s.name.clone(),
                ratio,
                format!("{:.0}%  {:.2}ms", s.percentage, s.time_ms),
                ProbeLevel::Normal,
            ));
            // Which entities drive this pass ("2 environment maps"); left out
            // when no attribution rule covers it.
            if !s.source.is_empty() {
                report.item(ProbeItem::note(s.source.clone(), ProbeLevel::Muted));
            }
        }
        report.item(ProbeItem::note(t("perf.gpu_pass_note"), ProbeLevel::Muted));
    }

    // Limitations.
    if let Some(timing) = timing {
        report
            .section(t("perf.limitations"))
            .item(ProbeItem::note(timing.limitation_note.clone(), ProbeLevel::Warn));
    }

    // External profilers: static guidance, nothing measured.
    report
        .section(t("perf.external_profilers"))
        .item(ProbeItem::note(t("perf.chrome_tracing"), ProbeLevel::Normal))
        .item(ProbeItem::note(t("perf.chrome_tracing_desc"), ProbeLevel::Muted))
        .item(ProbeItem::note("cargo run --features bevy/trace_chrome", ProbeLevel::Muted));

    report
}

/// How far the frame sits from the 60 fps budget, or from the 30 fps one once
/// it has missed 60.
fn frame_status(ms: f32) -> (String, ProbeLevel) {
    if ms <= 16.67 {
        (
            format!("Under 60fps target ({:.1}ms budget)", 16.67 - ms),
            ProbeLevel::Good,
        )
    } else if ms <= 33.33 {
        (
            format!("Between 30-60fps ({:.1}ms over 60fps target)", ms - 16.67),
            ProbeLevel::Warn,
        )
    } else {
        (
            format!("Below 30fps ({:.1}ms over target)", ms - 33.33),
            ProbeLevel::Bad,
        )
    }
}

fn frame_time_level(ms: f32) -> ProbeLevel {
    if ms <= 16.67 {
        ProbeLevel::Good
    } else if ms <= 33.33 {
        ProbeLevel::Warn
    } else {
        ProbeLevel::Bad
    }
}

fn fps_level(fps: f32) -> ProbeLevel {
    if fps >= 60.0 {
        ProbeLevel::Good
    } else if fps >= 30.0 {
        ProbeLevel::Warn
    } else {
        ProbeLevel::Bad
    }
}

fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}
