//! The Performance probe: frame rate, frame time and scene entity count, each
//! as a headline figure over its rolling history, plus the platform's CPU and
//! memory use when it reports them.
//!
//! Every value comes from [`DiagnosticsState`], which `update_diagnostics_state`
//! refreshes ten times a second for the status bar as well, so this probe adds
//! no gathering of its own. The thresholds are the Performance panel's: green
//! at 60 fps (16.67 ms) or better, amber down to 30 fps (33.33 ms), red below.

use bevy::prelude::*;
use renzora::lang::t;
use renzora::probes::{ProbeDef, ProbeItem, ProbeLevel, ProbeReport};

use crate::state::DiagnosticsState;

pub fn def() -> ProbeDef {
    ProbeDef {
        id: "performance",
        title: "Performance",
        icon: "gauge",
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

    let fps = d.fps as f32;
    report
        .section(t("perf.frames_per_second"))
        .item(ProbeItem::figure("FPS", format!("{:.0}", fps), "FPS", fps_level(fps)))
        .item(ProbeItem::stat(t("perf.avg"), format!("{:.0}", d.avg_fps())))
        .item(ProbeItem::stat(t("perf.min"), format!("{:.0}", d.min_fps())))
        .item(ProbeItem::stat(t("perf.max"), format!("{:.0}", d.max_fps())))
        .item(ProbeItem::series(
            t("perf.frames_per_second"),
            d.fps_history.iter().copied().collect(),
            "FPS",
        ));

    let ms = d.frame_time_ms as f32;
    report
        .section(t("perf.frame_time"))
        .item(ProbeItem::figure(
            t("perf.frame_time"),
            format!("{:.2}", ms),
            "ms",
            frame_time_level(ms),
        ))
        .item(ProbeItem::series(
            t("perf.frame_time"),
            d.frame_time_history.iter().copied().collect(),
            "ms",
        ));

    report
        .section(t("perf.entities"))
        .item(ProbeItem::figure(
            t("perf.entities"),
            d.entity_count.to_string(),
            "entities",
            ProbeLevel::Normal,
        ))
        .item(ProbeItem::series(
            t("perf.entities"),
            d.entity_count_history.iter().copied().collect(),
            "entities",
        ));

    // Only shown when the platform reports them: a zero would read as a real
    // measurement of an idle machine.
    if d.cpu_usage.is_some() || d.memory_usage.is_some() {
        report.section(t("memory.system"));
        if let Some(cpu) = d.cpu_usage {
            report.item(ProbeItem::stat(t("perf.cpu"), format!("{:.1}%", cpu)));
        }
        if let Some(mem) = d.memory_usage {
            report.item(ProbeItem::stat(t("perf.memory"), format_bytes(mem)));
        }
    }

    report
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

fn frame_time_level(ms: f32) -> ProbeLevel {
    if ms <= 16.67 {
        ProbeLevel::Good
    } else if ms <= 33.33 {
        ProbeLevel::Warn
    } else {
        ProbeLevel::Bad
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
