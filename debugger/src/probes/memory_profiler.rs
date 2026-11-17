//! Memory probe: the process's resident memory with its peak, history and
//! trend, an estimate of what the loaded meshes, textures and materials take,
//! and the rate the process is growing or shrinking at.
//!
//! Everything comes from [`MemoryProfilerState`], which `update_memory_profiler`
//! in `state.rs` refreshes from the OS and from `Assets`. This is the Memory
//! panel's content with its colours turned into [`ProbeLevel`]s: green is
//! `Good`, yellow and orange are `Warn`, red is `Bad`, grey is `Muted`.

use bevy::prelude::*;
use renzora::probes::{ProbeDef, ProbeItem, ProbeLevel, ProbeReport};

use crate::state::{MemoryProfilerState, MemoryTrend};

pub fn def() -> ProbeDef {
    ProbeDef {
        id: "memory_profiler",
        title: "Memory",
        icon: "memory",
        category: "Memory",
        build,
        act: None,
    }
}

const MB: f64 = 1024.0 * 1024.0;

/// Bytes in the largest unit that keeps the number at least 1, with the
/// precision the panel used for each unit.
fn format_memory(bytes: u64) -> String {
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

/// [`format_memory`] split into number and unit, for a figure, which draws
/// the two at different sizes.
fn memory_figure(bytes: u64) -> (String, String) {
    let text = format_memory(bytes);
    match text.split_once(' ') {
        Some((value, unit)) => (value.to_string(), unit.to_string()),
        None => (text, String::new()),
    }
}

/// The panel's four bands for process memory: under 512 MB green, under 1 GB
/// yellow, under 2 GB orange, above that red. Yellow and orange both read as
/// a warning; the probe vocabulary has one level for them.
fn memory_level(mb: f64) -> ProbeLevel {
    if mb < 512.0 {
        ProbeLevel::Good
    } else if mb < 2048.0 {
        ProbeLevel::Warn
    } else {
        ProbeLevel::Bad
    }
}

fn trend_parts(t: MemoryTrend) -> (String, ProbeLevel) {
    match t {
        MemoryTrend::Increasing => (
            format!("\u{2191} {}", renzora::lang::t("memory.trend_increasing")),
            ProbeLevel::Bad,
        ),
        MemoryTrend::Decreasing => (
            format!("\u{2193} {}", renzora::lang::t("memory.trend_decreasing")),
            ProbeLevel::Good,
        ),
        MemoryTrend::Stable => (
            format!("\u{2194} {}", renzora::lang::t("memory.trend_stable")),
            ProbeLevel::Muted,
        ),
    }
}

/// The signed rate in B/s, KB/s or MB/s. Below 1 KB/s is noise and reads
/// muted; above that, shrinking is good, growing is a warning in KB/s and bad
/// in MB/s.
fn alloc_parts(rate: f64) -> (String, ProbeLevel) {
    let a = rate.abs();
    let (num, level) = if a < 1024.0 {
        (format!("{:.0} B/s", a), ProbeLevel::Muted)
    } else if a < MB {
        (
            format!("{:.1} KB/s", a / 1024.0),
            if rate > 0.0 { ProbeLevel::Warn } else { ProbeLevel::Good },
        )
    } else {
        (
            format!("{:.2} MB/s", a / MB),
            if rate > 0.0 { ProbeLevel::Bad } else { ProbeLevel::Good },
        )
    };
    let prefix = if rate > 0.0 {
        "+"
    } else if rate < 0.0 {
        "-"
    } else {
        ""
    };
    (format!("{}{}", prefix, num), level)
}

fn build(world: &mut World) -> ProbeReport {
    let mut report = ProbeReport::default();
    let Some(s) = world.get_resource::<MemoryProfilerState>() else {
        report.item(ProbeItem::note(
            "The memory profiler's state is not in this world, so there is nothing to show.",
            ProbeLevel::Muted,
        ));
        return report;
    };
    if !s.available {
        report.item(ProbeItem::note(renzora::lang::t("memory.unavailable"), ProbeLevel::Muted));
        return report;
    }

    // Process memory.
    let (value, unit) = memory_figure(s.process_memory);
    let (trend, trend_level) = trend_parts(s.memory_trend);
    report
        .section(renzora::lang::t("memory.process_memory"))
        .item(ProbeItem::figure(
            renzora::lang::t("memory.process_memory"),
            value,
            unit,
            memory_level(s.process_memory as f64 / MB),
        ))
        .item(ProbeItem::stat(renzora::lang::t("memory.peak"), format_memory(s.peak_memory)))
        .item(ProbeItem::series(
            renzora::lang::t("memory.process_memory"),
            s.memory_history.iter().copied().collect(),
            "MB",
        ))
        .item(ProbeItem::stat_at(renzora::lang::t("memory.trend"), trend, trend_level));

    // Asset memory. Each bar is filled against the largest of the three
    // buckets, not against the total, so the biggest one is always full.
    let a = &s.asset_memory;
    let (total_value, total_unit) =
        memory_figure(a.meshes_bytes + a.textures_bytes + a.materials_bytes);
    let max = a.meshes_bytes.max(a.textures_bytes).max(a.materials_bytes).max(1) as f32;
    report
        .section(renzora::lang::t("memory.asset_memory_estimated"))
        .item(ProbeItem::figure("Total", total_value, total_unit, ProbeLevel::Normal))
        .item(ProbeItem::bar(
            format!("{} ({})", renzora::lang::t("memory.meshes"), a.mesh_count),
            a.meshes_bytes as f32 / max,
            format_memory(a.meshes_bytes),
            ProbeLevel::Normal,
        ))
        .item(ProbeItem::bar(
            format!("{} ({})", renzora::lang::t("memory.textures"), a.texture_count),
            a.textures_bytes as f32 / max,
            format_memory(a.textures_bytes),
            ProbeLevel::Normal,
        ))
        .item(ProbeItem::bar(
            format!("{} ({})", renzora::lang::t("memory.materials"), a.material_count),
            a.materials_bytes as f32 / max,
            format_memory(a.materials_bytes),
            ProbeLevel::Normal,
        ));

    // Allocation rate, with the panel's warning past 10 MB/s of growth.
    let (rate, rate_level) = alloc_parts(s.allocation_rate);
    let high = s.allocation_rate > 10.0 * MB;
    report
        .section(renzora::lang::t("memory.allocation_rate"))
        .item(ProbeItem::stat_at(renzora::lang::t("memory.allocation_rate"), rate, rate_level));
    if high {
        report.item(ProbeItem::note(
            format!("\u{26a0} {}", renzora::lang::t("memory.high_alloc_rate")),
            ProbeLevel::Warn,
        ));
    }
    report
}
