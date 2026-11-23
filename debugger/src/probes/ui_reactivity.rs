//! UI Reactivity probe: a live profile of ember's reactive layer.
//!
//! Everything here is read straight from [`ReactiveStats`], which ember's
//! reactive drivers update every frame on their own: how many bindings and
//! keyed lists exist, what recomputing them costs, how many produced a new
//! value, and the top-N tables ember rebuilds every 30 frames (the most
//! expensive bindings, the bindings whose values keep changing, and every
//! keyed list's snapshot cost). The probe adds no gathering of its own, so
//! watching it costs the editor one resource read a few times a second.
//!
//! Unlike the panel it replaces, the probe does not observe itself: the panel's
//! own bindings sat in the same registry and nudged the totals up while it was
//! open, and a report built from the `World` registers none.

use bevy::prelude::*;

use renzora::lang::t;
use renzora::probes::{ProbeDef, ProbeItem, ProbeLevel, ProbeReport};
use renzora::ui::reactive::{BindingReport, ReactiveStats};

/// How many keyed lists the panel listed; the report keeps the same cap so a
/// UI with hundreds of lists does not make the report itself heavy.
const LIST_ROWS: usize = 8;

pub fn def() -> ProbeDef {
    ProbeDef {
        id: "ui_reactivity",
        title: "UI Reactivity",
        icon: "lightning",
        category: "Editor UI",
        build,
        act: None,
    }
}

/// Recompute cost colouring: green under a millisecond, amber to four, red
/// past that. UI Layout uses the same scale so the two compare at a glance.
fn cost_level(ms: f32) -> ProbeLevel {
    if ms > 4.0 {
        ProbeLevel::Bad
    } else if ms > 1.0 {
        ProbeLevel::Warn
    } else {
        ProbeLevel::Good
    }
}

/// Share of the walked bindings the dependency gate skipped, in percent.
/// `None` with no bindings at all, where a percentage means nothing.
fn skipped_pct(s: &ReactiveStats) -> Option<f32> {
    (s.bindings_total != 0)
        .then(|| 100.0 * s.skipped_this_frame as f32 / s.bindings_total as f32)
}

/// One row of a top-N binding table: `label, kind, metric`.
fn binding_rows(reports: &[BindingReport], metric: impl Fn(&BindingReport) -> String) -> Vec<Vec<String>> {
    reports
        .iter()
        .take(ReactiveStats::TOP_N)
        .map(|r| vec![r.label.clone(), r.kind.to_string(), metric(r)])
        .collect()
}

fn build(world: &mut World) -> ProbeReport {
    let mut report = ProbeReport::default();
    let Some(s) = world.get_resource::<ReactiveStats>() else {
        report.item(ProbeItem::note(
            "ReactiveStats is not present: ember's reactive layer is not running in this process.",
            ProbeLevel::Muted,
        ));
        return report;
    };

    let recompute_ms = (s.reactions_us + s.lists_us) / 1000.0;
    // The headline for dependency tracking: of the bindings eligible to run
    // this frame, how many were skipped because nothing they read had moved.
    // 0% means nothing on screen is migrated to `tracked::` yet: the machinery
    // is inert, not broken.
    let pct = skipped_pct(s);
    let pct_level = match pct {
        Some(p) if p > 60.0 => ProbeLevel::Good,
        Some(p) if p > 15.0 => ProbeLevel::Warn,
        _ => ProbeLevel::Bad,
    };

    report
        .section("Overview")
        .item(ProbeItem::figure("bindings", s.bindings_total.to_string(), "", ProbeLevel::Normal))
        .item(ProbeItem::figure(
            "recompute",
            format!("{recompute_ms:.2}"),
            "ms/frame",
            cost_level(recompute_ms),
        ))
        .item(ProbeItem::figure(
            "skipped (tracked)",
            pct.map(|p| format!("{p:.0}")).unwrap_or_else(|| "n/a".to_string()),
            "%",
            pct_level,
        ))
        .item(ProbeItem::series(
            "Recompute history",
            s.history_us.iter().map(|us| us / 1000.0).collect(),
            "ms",
        ));

    report
        .section(t("reactivity.this_frame"))
        .item(ProbeItem::stat(t("reactivity.bindings_run"), s.bindings_total))
        .item(ProbeItem::stat(
            t("reactivity.skipped"),
            format!("{} / {}", s.skipped_this_frame, s.bindings_total),
        ))
        // Parked bindings are alive and restorable but out of the walk
        // entirely: collapse a big section and watch this take the load.
        .item(ProbeItem::stat(t("reactivity.parked"), s.parked_total))
        .item(ProbeItem::stat(t("reactivity.new_values"), s.changed_this_frame))
        .item(ProbeItem::stat(t("reactivity.changes_per_sec"), format!("{:.0}", s.changes_per_sec)))
        .item(ProbeItem::stat(t("reactivity.bindings_time"), format!("{:.2} ms", s.reactions_us / 1000.0)))
        .item(ProbeItem::stat(t("reactivity.keyed_lists"), s.lists_total))
        .item(ProbeItem::stat(t("reactivity.lists_time"), format!("{:.2} ms", s.lists_us / 1000.0)))
        .item(ProbeItem::stat(t("reactivity.rows_rebuilt"), s.rows_rebuilt_this_frame));

    let cost_rows = binding_rows(&s.top_cost, |r| format!("{:.1}", r.cost_ema_us));
    report.section(t("reactivity.top_cost"));
    if cost_rows.is_empty() {
        report.item(ProbeItem::note("No bindings ranked yet.", ProbeLevel::Muted));
    } else {
        report.item(ProbeItem::table(&["Binding", "Kind", "us/frame"], cost_rows));
    }

    // Churn: bindings whose computed value keeps coming back different. They
    // defeat value-diffing and force a UI write every frame.
    let churn_rows = binding_rows(&s.top_churn, |r| format!("{:.0}/s", r.change_rate));
    report.section(t("reactivity.top_churn"));
    if churn_rows.is_empty() {
        report.item(ProbeItem::note("No bindings ranked yet.", ProbeLevel::Muted));
    } else {
        report.item(ProbeItem::table(&["Binding", "Kind", "Changes"], churn_rows));
    }

    let list_rows: Vec<Vec<String>> = s
        .list_reports
        .iter()
        .take(LIST_ROWS)
        .map(|r| {
            vec![
                r.label.clone(),
                format!("{} rows", r.rows),
                format!("{:.1}", r.cost_ema_us),
                format!("{} rebuilt", r.rows_rebuilt),
            ]
        })
        .collect();
    report.section(t("reactivity.keyed_lists_section"));
    if list_rows.is_empty() {
        report.item(ProbeItem::note("No keyed lists.", ProbeLevel::Muted));
    } else {
        report.item(ProbeItem::table(&["List", "Rows", "us/frame", "Rebuilt"], list_rows));
    }

    report
}
