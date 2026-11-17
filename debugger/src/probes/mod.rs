//! The debug readouts, as probes the companion shows.
//!
//! These were dock panels in the editor's own window. A panel that changes
//! every frame makes Bevy lay out the editor's whole UI again every frame:
//! with the Debug workspace open, layout took 30 ms of a 50 ms frame. As
//! probes they build a plain report a few times a second, only while the
//! companion's Debug screen shows them, and the companion draws it.
//!
//! One module per probe, each with a `def()`; the two that own systems which
//! must keep running whether or not anyone watches (UI layout timing feeds
//! the diagnostic store, render toggles enforce their switches) also have a
//! `register`.

use bevy::prelude::*;
use renzora::probes::RegisterProbe;

mod camera_debug;
mod culling_debug;
mod ecs_stats;
mod material_resolver_diag;
mod memory_profiler;
mod performance;
mod render_stats;
mod render_toggles;
mod scripting_diag;
mod system_profiler;
mod ui_layout;
mod ui_reactivity;

pub fn register(app: &mut App) {
    ui_layout::register(app);
    render_toggles::register(app);
    app.register_probe(performance::def())
        .register_probe(system_profiler::def())
        .register_probe(render_stats::def())
        .register_probe(memory_profiler::def())
        .register_probe(ecs_stats::def())
        .register_probe(camera_debug::def())
        .register_probe(culling_debug::def())
        .register_probe(material_resolver_diag::def())
        .register_probe(scripting_diag::def())
        .register_probe(ui_reactivity::def())
        .register_probe(ui_layout::def())
        .register_probe(render_toggles::def());
}
