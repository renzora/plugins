//! Moving the script's entity, its parent, and its named children.
//!
//! Every function taking a position accepts either three numbers or one
//! vector, so `set_position(1, 2, 3)` and `set_position(vec3(1, 2, 3))` (or
//! `set_position({x: 1, y: 2, z: 3})`) all work.

use boa_engine::{Context, JsObject};
use renzora::ScriptCommand;

use super::command;

pub fn register(ctx: &mut Context, m: &JsObject) {
    command!(ctx, m, "set_position", |a, c| {
        let [x, y, z] = a.xyz(0, c)?;
        ScriptCommand::SetPosition { x, y, z }
    });
    command!(ctx, m, "set_rotation", |a, c| {
        let [x, y, z] = a.xyz(0, c)?;
        ScriptCommand::SetRotation { x, y, z }
    });
    command!(ctx, m, "set_scale", |a, c| {
        let [x, y, z] = a.xyz(0, c)?;
        ScriptCommand::SetScale { x, y, z }
    });
    command!(ctx, m, "set_scale_uniform", |a, c| {
        let s = a.f32(0, "scale")?;
        ScriptCommand::SetScale { x: s, y: s, z: s }
    });
    command!(ctx, m, "translate", |a, c| {
        let [x, y, z] = a.xyz(0, c)?;
        ScriptCommand::Translate { x, y, z }
    });
    command!(ctx, m, "rotate", |a, c| {
        let [x, y, z] = a.xyz(0, c)?;
        ScriptCommand::Rotate { x, y, z }
    });
    command!(ctx, m, "look_at", |a, c| {
        let [x, y, z] = a.xyz(0, c)?;
        ScriptCommand::LookAt { x, y, z }
    });
    command!(ctx, m, "goto_camera_preset", |a, c| ScriptCommand::GotoCameraPreset {
        name: a.str(0, "name")?,
    });

    // -- Parent transform --
    command!(ctx, m, "parent_set_position", |a, c| {
        let [x, y, z] = a.xyz(0, c)?;
        ScriptCommand::ParentSetPosition { x, y, z }
    });
    command!(ctx, m, "parent_set_rotation", |a, c| {
        let [x, y, z] = a.xyz(0, c)?;
        ScriptCommand::ParentSetRotation { x, y, z }
    });
    command!(ctx, m, "parent_translate", |a, c| {
        let [x, y, z] = a.xyz(0, c)?;
        ScriptCommand::ParentTranslate { x, y, z }
    });

    // -- Child transform, by the child's name --
    command!(ctx, m, "set_child_position", |a, c| {
        let name = a.str(0, "name")?;
        let [x, y, z] = a.xyz(1, c)?;
        ScriptCommand::ChildSetPosition { name, x, y, z }
    });
    command!(ctx, m, "set_child_rotation", |a, c| {
        let name = a.str(0, "name")?;
        let [x, y, z] = a.xyz(1, c)?;
        ScriptCommand::ChildSetRotation { name, x, y, z }
    });
    command!(ctx, m, "child_translate", |a, c| {
        let name = a.str(0, "name")?;
        let [x, y, z] = a.xyz(1, c)?;
        ScriptCommand::ChildTranslate { name, x, y, z }
    });
}
