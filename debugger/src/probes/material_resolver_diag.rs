//! The Material Diag probe: how many materials the resolver is holding, how
//! often its cache is hit, what compiling has cost, a per-material timing table
//! and the most recent compile failures.
//!
//! The values come from two contract-crate resources that `renzora_shader`
//! keeps current as it works: [`MaterialCacheCounts`] (published in place of
//! the resolver's own `MaterialCache`, which a plugin cannot name) and
//! [`MaterialPerfStats`]. Neither needs a gathering system here, so watching
//! this probe costs only the report.
//!
//! A table row carries no colour, so the panel's colouring is moved into the
//! text where it said something: a material that has failed to compile gets a
//! warning mark before its name, and a last compile of 20 ms or more (the
//! panel's amber) reads "(slow)" after its time, 100 ms or more (its red)
//! "(very slow)". The error
//! the panel printed under a row is the row's last column.

use std::time::Duration;

use bevy::prelude::*;
use renzora::diagnostics::material::{MaterialPerf, MaterialPerfStats, MAX_RECENT_FAILURES};
use renzora::diagnostics::MaterialCacheCounts;
use renzora::lang::t;
use renzora::probes::{ProbeDef, ProbeItem, ProbeLevel, ProbeReport};

pub fn def() -> ProbeDef {
    ProbeDef {
        id: "material_resolver_diag",
        title: "Material Diag",
        icon: "palette",
        category: "Rendering",
        build,
        act: None,
    }
}

/// A duration in the unit that keeps it short. Zero reads as a dash: a
/// material that has never compiled has no time, which is not the same as one
/// that compiled instantly.
fn format_duration(d: Duration) -> String {
    let us = d.as_micros();
    if us == 0 {
        "-".to_string()
    } else if us < 1_000 {
        format!("{} \u{b5}s", us)
    } else if us < 1_000_000 {
        format!("{:.2} ms", us as f64 / 1_000.0)
    } else {
        format!("{:.2} s", us as f64 / 1_000_000.0)
    }
}

fn format_count(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    }
}

/// Shader errors can run to pages; 200 characters is enough to recognise one.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('\u{2026}');
        out
    }
}

/// The panel's thresholds for a compile time: amber from 20 ms, red from 100 ms.
fn duration_level(d: Duration) -> ProbeLevel {
    let ms = d.as_micros() as f64 / 1_000.0;
    if ms >= 100.0 {
        ProbeLevel::Bad
    } else if ms >= 20.0 {
        ProbeLevel::Warn
    } else {
        ProbeLevel::Normal
    }
}

/// `parent/file`: enough to tell materials apart without the project root
/// that every path shares.
fn short_path(path: &str) -> String {
    let p = std::path::Path::new(path);
    if let (Some(parent), Some(name)) = (p.parent().and_then(|p| p.file_name()), p.file_name()) {
        format!("{}/{}", parent.to_string_lossy(), name.to_string_lossy())
    } else {
        path.to_string()
    }
}

fn row(path: &str, p: &MaterialPerf) -> Vec<String> {
    let name = if p.fail_count > 0 {
        format!("\u{26a0} {}", short_path(path))
    } else {
        short_path(path)
    };
    let last = match duration_level(p.last_compile) {
        ProbeLevel::Bad => format!("{} (very slow)", format_duration(p.last_compile)),
        ProbeLevel::Warn => format!("{} (slow)", format_duration(p.last_compile)),
        _ => format_duration(p.last_compile),
    };
    vec![
        name,
        p.kind.label().to_string(),
        last,
        format_duration(p.max_compile),
        format_count(p.compile_count),
        format_count(p.cache_hits),
        p.last_error
            .as_ref()
            .map(|e| format!("\u{2717} {}", truncate(e, 200)))
            .unwrap_or_default(),
    ]
}

fn build(world: &mut World) -> ProbeReport {
    let mut report = ProbeReport::default();
    let (Some(cache), Some(perf)) = (
        world.get_resource::<MaterialCacheCounts>(),
        world.get_resource::<MaterialPerfStats>(),
    ) else {
        report.item(ProbeItem::note(t("material.resources_absent"), ProbeLevel::Muted));
        return report;
    };

    report
        .section("Cache")
        .item(ProbeItem::figure(
            "Materials",
            cache.total().to_string(),
            "materials cached",
            ProbeLevel::Normal,
        ))
        .item(ProbeItem::note(
            format!(
                "{} std \u{b7} {} graph \u{b7} {} code  \u{b7}  {} master-meta entries",
                cache.standard, cache.graph, cache.code, cache.master_meta,
            ),
            ProbeLevel::Muted,
        ))
        .item(ProbeItem::stat(t("material.cache_hits"), format_count(perf.total_cache_hits)))
        .item(ProbeItem::stat(t("material.compiles_ran"), format_count(perf.total_compiles)))
        .item(ProbeItem::stat(
            t("material.total_compile_time"),
            format_duration(perf.total_compile_time),
        ));
    if perf.total_failures > 0 {
        report.item(ProbeItem::note(
            format!(
                "\u{26a0} {} compile failure{}",
                perf.total_failures,
                if perf.total_failures == 1 { "" } else { "s" }
            ),
            ProbeLevel::Bad,
        ));
    }

    report.section(t("material.perf_table"));
    let snap = perf.snapshot();
    if snap.is_empty() {
        report.item(ProbeItem::note(t("material.no_materials"), ProbeLevel::Muted));
    } else {
        let columns = [
            t("material.col_material"),
            t("material.col_kind"),
            t("perf.last"),
            t("perf.max"),
            t("material.col_compiles"),
            t("material.col_hits"),
            "Error".to_string(),
        ];
        let columns: Vec<&str> = columns.iter().map(String::as_str).collect();
        let rows = snap.iter().map(|(path, p)| row(path, p)).collect();
        report.item(ProbeItem::table(&columns, rows));
    }

    if !perf.recent_failures.is_empty() {
        report
            .section(format!(
                "Recent failures ({} of {} kept)",
                perf.recent_failures.len(),
                MAX_RECENT_FAILURES
            ))
            .item(ProbeItem::table(
                &["Material", "Error"],
                perf.recent_failures
                    .iter()
                    .map(|(path, err)| vec![short_path(path), truncate(err, 200)])
                    .collect(),
            ));
    }

    report
}
