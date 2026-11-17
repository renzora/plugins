//! ECS Stats probe: how many entities the whole `World` holds (the editor's
//! own UI included, which is the point of this view), its history, and the
//! archetypes, component types and resources behind that number.
//!
//! The data is [`EcsStatsState`], which `update_ecs_stats` in `state.rs`
//! fills from an archetype scan. The panel kept each list behind a
//! collapsible header so a closed list built no rows; a probe has no rows to
//! build, so all three are always in the report, with the same caps the
//! panel used (20 archetypes, 30 component types, 50 resources).

use bevy::prelude::*;
use renzora::probes::{ProbeDef, ProbeItem, ProbeLevel, ProbeReport};

use crate::state::EcsStatsState;

pub fn def() -> ProbeDef {
    ProbeDef {
        id: "ecs_stats",
        title: "ECS Stats",
        icon: "list-numbers",
        category: "World",
        build,
        act: None,
    }
}

/// The last path segment of a type path: `bevy_transform::Transform` reads
/// as `Transform`. Generic arguments keep their own paths, as in the panel.
fn short_name(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

fn build(world: &mut World) -> ProbeReport {
    let mut report = ProbeReport::default();
    let Some(s) = world.get_resource::<EcsStatsState>() else {
        report.item(ProbeItem::note(
            "The ECS stats state is not in this world, so there is nothing to show.",
            ProbeLevel::Muted,
        ));
        return report;
    };

    report
        .section(renzora::lang::t("ecs.entities"))
        .item(ProbeItem::figure(
            renzora::lang::t("ecs.entities"),
            s.entity_count.to_string(),
            "entities",
            ProbeLevel::Normal,
        ))
        .item(ProbeItem::stat(renzora::lang::t("ecs.archetypes"), s.archetype_count))
        .item(ProbeItem::stat(renzora::lang::t("ecs.component_types"), s.component_stats.len()))
        .item(ProbeItem::series(
            renzora::lang::t("ecs.entities"),
            s.entity_count_history.iter().copied().collect(),
            "",
        ));

    // Archetypes, largest first. More than three components collapse to the
    // first three and a count, so a row stays one line.
    report.section(format!("{} ({})", renzora::lang::t("ecs.archetypes"), s.archetype_count));
    if s.top_archetypes.is_empty() {
        report.item(ProbeItem::note(renzora::lang::t("ecs.no_archetypes"), ProbeLevel::Muted));
    } else {
        let rows = s
            .top_archetypes
            .iter()
            .take(20)
            .map(|a| {
                let detail = if a.components.len() > 3 {
                    format!(
                        "{}, ... +{}",
                        a.components.iter().take(3).map(|c| short_name(c)).collect::<Vec<_>>().join(", "),
                        a.components.len() - 3
                    )
                } else {
                    a.components.iter().map(|c| short_name(c)).collect::<Vec<_>>().join(", ")
                };
                vec![a.entity_count.to_string(), detail]
            })
            .collect();
        report.item(ProbeItem::table(&["Entities", "Components"], rows));
    }

    // Component types, by instance count.
    let total = s.component_stats.len();
    report.section(format!("{} ({})", renzora::lang::t("ecs.component_types"), total));
    if s.component_stats.is_empty() {
        report.item(ProbeItem::note(renzora::lang::t("ecs.no_components"), ProbeLevel::Muted));
    } else {
        let rows = s
            .component_stats
            .iter()
            .take(30)
            .map(|c| vec![c.instance_count.to_string(), short_name(&c.name).to_string()])
            .collect();
        report.item(ProbeItem::table(&["Instances", "Component"], rows));
        let more = total.saturating_sub(30);
        if more > 0 {
            report.item(ProbeItem::note(format!("... and {} more", more), ProbeLevel::Muted));
        }
    }

    // Resources.
    report.section(format!("{} ({})", renzora::lang::t("ecs.resources"), s.resources.len()));
    if s.resources.is_empty() {
        report.item(ProbeItem::note(
            renzora::lang::t("ecs.resources_not_enabled"),
            ProbeLevel::Muted,
        ));
    } else {
        let rows = s
            .resources
            .iter()
            .take(50)
            .map(|r| vec![short_name(r).to_string()])
            .collect();
        report.item(ProbeItem::table(&["Resource"], rows));
    }
    report
}
