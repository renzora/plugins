//! The Scripting Diag probe: what scripts cost last frame and on average, how
//! many entities carry them, and a per-script timing table.
//!
//! It reads the two contract-crate resources `renzora_scripting` keeps current
//! itself, [`ScriptInventory`] (rewritten only when its counts change) and
//! [`ScriptPerfStats`] (recorded around every hook call), rather than the
//! panel's `ScriptingDiagState` copy of them. That copy existed so the panel's
//! reactive bindings had one resource to watch, and it was only refreshed while
//! the panel was the active tab; a probe builds straight from the `World` a
//! few times a second, so the copy and its gated updater have nothing to add.
//!
//! A table row carries no colour, so the panel's colouring is moved into the
//! text where it said something: a script that has thrown gets a warning mark
//! before its name, one not run for more than two frames (its entity despawned,
//! or it stopped being dispatched) reads "(stale)" after it, and a last update
//! of 1 ms or more (the panel's amber) reads "(slow)" after its time, 5 ms or
//! more (its red) "(very slow)". The hook times and the error the panel printed
//! under a row are the row's last two columns.

use std::path::Path;
use std::time::Duration;

use bevy::prelude::*;
use renzora::diagnostics::script::{ScriptPerf, ScriptPerfStats, ScriptPerfTotals};
use renzora::diagnostics::ScriptInventory;
use renzora::lang::t;
use renzora::probes::{ProbeDef, ProbeItem, ProbeLevel, ProbeReport};

pub fn def() -> ProbeDef {
    ProbeDef {
        id: "scripting_diag",
        title: "Scripting Diag",
        icon: "bug",
        category: "Scripting",
        build,
        act: None,
    }
}

/// A duration in the unit that keeps it short. Zero reads as a dash: a hook
/// that has never run has no time, which is not the same as one that ran
/// instantly.
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

/// Script errors can carry a whole backtrace; 200 characters is enough to
/// recognise one.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('\u{2026}');
        out
    }
}

/// The panel's thresholds for one script's update: amber from 1 ms, red from
/// 5 ms. Tighter than the material ones because this cost is paid every frame.
fn duration_level(d: Duration) -> ProbeLevel {
    let micros = d.as_micros();
    if micros >= 5_000 {
        ProbeLevel::Bad
    } else if micros >= 1_000 {
        ProbeLevel::Warn
    } else {
        ProbeLevel::Normal
    }
}

fn row(path: &Path, perf: &ScriptPerf, current_frame: u64) -> Vec<String> {
    let stale = current_frame.saturating_sub(perf.last_seen_frame) > 2;
    let mut name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    if perf.error_count > 0 {
        name = format!("\u{26a0} {}", name);
    }
    if stale {
        name.push_str(" (stale)");
    }

    let last = match duration_level(perf.last_on_update) {
        ProbeLevel::Bad => format!("{} (very slow)", format_duration(perf.last_on_update)),
        ProbeLevel::Warn => format!("{} (slow)", format_duration(perf.last_on_update)),
        _ => format_duration(perf.last_on_update),
    };

    let mut hooks: Vec<String> = Vec::new();
    if perf.last_on_ready > Duration::ZERO {
        hooks.push(format!("ready: {}", format_duration(perf.last_on_ready)));
    }
    if perf.last_on_rpc > Duration::ZERO {
        hooks.push(format!("rpc: {}", format_duration(perf.last_on_rpc)));
    }
    if perf.last_on_ui > Duration::ZERO {
        hooks.push(format!("ui: {}", format_duration(perf.last_on_ui)));
    }

    vec![
        name,
        last,
        format_duration(perf.avg_on_update),
        format_duration(perf.max_on_update),
        format_count(perf.on_update_calls),
        hooks.join("  \u{b7}  "),
        perf.last_error
            .as_ref()
            .map(|e| format!("\u{2717} {}", truncate(e, 200)))
            .unwrap_or_default(),
    ]
}

fn build(world: &mut World) -> ProbeReport {
    let mut report = ProbeReport::default();
    let inventory = world.get_resource::<ScriptInventory>();
    let perf = world.get_resource::<ScriptPerfStats>();
    if inventory.is_none() && perf.is_none() {
        report.item(ProbeItem::note(
            "Scripting is not running in this process.",
            ProbeLevel::Muted,
        ));
        return report;
    }

    // Either alone still reports, with the other's values at their defaults,
    // exactly as the panel's updater left them.
    let inventory = inventory.cloned().unwrap_or_default();
    let totals: ScriptPerfTotals = perf.map(|p| p.totals()).unwrap_or_default();
    let scripts = perf.map(|p| p.snapshot()).unwrap_or_default();
    let frame = perf.map(|p| p.frame).unwrap_or(0);

    report
        .section("Totals")
        .item(ProbeItem::figure(
            "Update",
            format_duration(totals.total_last_update),
            "last frame",
            ProbeLevel::Normal,
        ))
        .item(ProbeItem::note(
            format!(
                "avg {} \u{b7} {} script{} \u{b7} {} calls",
                format_duration(totals.total_avg_update),
                totals.script_count,
                if totals.script_count == 1 { "" } else { "s" },
                totals.total_calls,
            ),
            ProbeLevel::Muted,
        ));
    if totals.scripts_with_errors > 0 {
        report.item(ProbeItem::note(
            format!(
                "\u{26a0} {} script{} threw errors ({} total)",
                totals.scripts_with_errors,
                if totals.scripts_with_errors == 1 { "" } else { "s" },
                totals.total_errors,
            ),
            ProbeLevel::Bad,
        ));
    }
    report
        .item(ProbeItem::stat(t("scripting.entities_with_script"), inventory.entities_with_script))
        .item(ProbeItem::stat(t("scripting.total_attachments"), inventory.total_attachments))
        .item(ProbeItem::stat(t("scripting.backends_registered"), inventory.backend_count))
        .item(ProbeItem::stat(
            t("scripting.scripts_folder"),
            inventory.scripts_folder.clone().unwrap_or_else(|| "<unset>".to_string()),
        ));

    report.section(t("scripting.per_script_timing"));
    if scripts.is_empty() {
        report.item(ProbeItem::note(t("scripting.no_scripts"), ProbeLevel::Muted));
    } else {
        let columns = [
            t("scripting.col_script"),
            t("perf.last"),
            t("perf.avg"),
            t("perf.max"),
            t("scripting.col_calls"),
            "Hooks".to_string(),
            "Error".to_string(),
        ];
        let columns: Vec<&str> = columns.iter().map(String::as_str).collect();
        let rows = scripts.iter().map(|(path, p)| row(path, p, frame)).collect();
        report.item(ProbeItem::table(&columns, rows));
    }

    report
}
