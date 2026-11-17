//! Camera Debug probe: every scene camera with its active state, projection
//! and order, the selected camera's properties, and the debug visualization
//! switches.
//!
//! The list and the properties come from [`CameraDebugState`], which
//! `update_camera_debug_state` in `state.rs` rebuilds from the 2D and 3D
//! camera queries. The panel's per-row controls become probe controls whose
//! ids carry the camera: `active:<entity bits>` is the ON/OFF switch, which
//! flips `Camera::is_active` on the entity itself, and `select:<entity bits>`
//! picks the camera whose properties are shown. The bits are resolved
//! against the snapshot's own list rather than rebuilt into an `Entity`, so a
//! stale or forged id can only miss, never name some other entity.

use bevy::prelude::*;
use renzora::probes::{ProbeDef, ProbeItem, ProbeLevel, ProbeReport};

use crate::state::{CameraDebugState, CameraProjectionType};

pub fn def() -> ProbeDef {
    ProbeDef {
        id: "camera_debug",
        title: "Camera Debug",
        icon: "video-camera",
        category: "Rendering",
        build,
        act: Some(act),
    }
}

const ACTIVE: &str = "active:";
const SELECT: &str = "select:";
const VIZ_FRUSTUM_SELECTED: &str = "viz:frustum_selected";
const VIZ_AXES: &str = "viz:axes";
const VIZ_ALL_FRUSTUMS: &str = "viz:all_frustums";

fn proj_letter(p: CameraProjectionType) -> &'static str {
    match p {
        CameraProjectionType::Perspective => "P",
        CameraProjectionType::Orthographic => "O",
    }
}

fn vec3(v: Vec3) -> String {
    format!("({:.2}, {:.2}, {:.2})", v.x, v.y, v.z)
}

fn build(world: &mut World) -> ProbeReport {
    let mut report = ProbeReport::default();
    let Some(s) = world.get_resource::<CameraDebugState>() else {
        report.item(ProbeItem::note(
            "The camera debug state is not in this world, so there is nothing to show.",
            ProbeLevel::Muted,
        ));
        return report;
    };

    report.item(ProbeItem::figure(
        renzora::lang::t("cam_debug.cameras"),
        s.scene_camera_count().to_string(),
        "cameras",
        ProbeLevel::Normal,
    ));

    // Camera list: a table for reading, then one switch and one select
    // button per camera for acting, since a table row cannot be clicked.
    report.section(renzora::lang::t("cam_debug.cameras"));
    if s.cameras.is_empty() {
        report.item(ProbeItem::note(renzora::lang::t("cam_debug.no_cameras"), ProbeLevel::Muted));
    } else {
        let rows = s
            .cameras
            .iter()
            .map(|c| {
                let selected = if s.selected_camera == Some(c.entity) { ">" } else { "" };
                let active = if c.is_active {
                    renzora::lang::t("cam_debug.on")
                } else {
                    renzora::lang::t("cam_debug.off")
                };
                vec![
                    selected.to_string(),
                    c.name.clone(),
                    active,
                    proj_letter(c.projection_type).to_string(),
                    format!("#{}", c.order),
                ]
            })
            .collect();
        report.item(ProbeItem::table(&["", "Camera", "Active", "Projection", "Order"], rows));
        for c in &s.cameras {
            let bits = c.entity.to_bits();
            report.item(ProbeItem::toggle(format!("{ACTIVE}{bits}"), c.name.clone(), c.is_active));
            if s.selected_camera != Some(c.entity) {
                report.item(ProbeItem::button(format!("{SELECT}{bits}"), format!("Select {}", c.name)));
            }
        }
    }

    // Selected camera properties, only while one is selected.
    if let Some(c) = s.selected_camera_info() {
        let projection = match c.projection_type {
            CameraProjectionType::Perspective => renzora::lang::t("comp.camera.perspective"),
            CameraProjectionType::Orthographic => renzora::lang::t("comp.camera.orthographic"),
        };
        report
            .section(renzora::lang::t("cam_debug.selected_camera"))
            .item(ProbeItem::stat("Name", c.name.clone()))
            .item(ProbeItem::stat(renzora::lang::t("cam_debug.projection"), projection))
            .item(ProbeItem::stat(
                renzora::lang::t("cam_debug.fov"),
                c.fov_degrees.map(|f| format!("{:.1}\u{b0}", f)).unwrap_or_else(|| "n/a".into()),
            ))
            .item(ProbeItem::stat(
                format!("{} / {}", renzora::lang::t("cam_debug.near"), renzora::lang::t("cam_debug.far")),
                format!("{:.2} / {:.0}", c.near, c.far),
            ))
            .item(ProbeItem::stat(renzora::lang::t("cam_debug.aspect_short"), format!("{:.3}", c.aspect_ratio)))
            .item(ProbeItem::stat(renzora::lang::t("cam_debug.position"), vec3(c.position)))
            .item(ProbeItem::stat(renzora::lang::t("cam_debug.rotation"), vec3(c.rotation_degrees)))
            .item(ProbeItem::stat(renzora::lang::t("cam_debug.forward"), vec3(c.forward)));
    }

    report
        .section(renzora::lang::t("cam_debug.debug_visualization"))
        .item(ProbeItem::toggle(
            VIZ_FRUSTUM_SELECTED,
            renzora::lang::t("cam_debug.show_frustum_selected"),
            s.show_frustum_gizmos,
        ))
        .item(ProbeItem::toggle(VIZ_AXES, renzora::lang::t("cam_debug.show_camera_axes"), s.show_camera_axes))
        .item(ProbeItem::toggle(
            VIZ_ALL_FRUSTUMS,
            renzora::lang::t("cam_debug.show_all_frustums"),
            s.show_all_frustums,
        ));
    report
}

/// The snapshot camera whose entity bits are `bits`, if it is still listed.
fn listed_camera(world: &World, bits: &str) -> Option<Entity> {
    let bits: u64 = bits.parse().ok()?;
    world
        .get_resource::<CameraDebugState>()?
        .cameras
        .iter()
        .find(|c| c.entity.to_bits() == bits)
        .map(|c| c.entity)
}

fn act(world: &mut World, id: &str, value: Option<bool>) {
    if let Some(bits) = id.strip_prefix(ACTIVE) {
        let Some(entity) = listed_camera(world, bits) else {
            return;
        };
        let Some(mut camera) = world.get_mut::<Camera>(entity) else {
            return;
        };
        // A toggle says what it wants; anything else flips, as the panel's
        // ON/OFF button did.
        let on = value.unwrap_or(!camera.is_active);
        camera.is_active = on;
        // The snapshot is refreshed on an interval; write the new state into
        // it too, so the next report does not show the old one.
        if let Some(mut state) = world.get_resource_mut::<CameraDebugState>() {
            if let Some(info) = state.cameras.iter_mut().find(|c| c.entity == entity) {
                info.is_active = on;
            }
        }
        return;
    }
    if let Some(bits) = id.strip_prefix(SELECT) {
        if let Some(entity) = listed_camera(world, bits) {
            if let Some(mut state) = world.get_resource_mut::<CameraDebugState>() {
                state.selected_camera = Some(entity);
            }
        }
        return;
    }
    let Some(mut state) = world.get_resource_mut::<CameraDebugState>() else {
        return;
    };
    let flag = match id {
        VIZ_FRUSTUM_SELECTED => &mut state.show_frustum_gizmos,
        VIZ_AXES => &mut state.show_camera_axes,
        VIZ_ALL_FRUSTUMS => &mut state.show_all_frustums,
        _ => return,
    };
    *flag = value.unwrap_or(!*flag);
}
