//! Moving the script's entity, its parent, and its named children.
//!
//! Every function taking a position accepts either three numbers or one
//! `vec3`, so `set_position(*v)` and `set_position(v)` both work.

use renzora::ScriptCommand;
use rustpython_vm::builtins::PyDict;
use rustpython_vm::{Py, VirtualMachine};

use super::command;

pub fn register(vm: &VirtualMachine, m: &Py<PyDict>) {
    command!(vm, m, "set_position", |a| {
        let [x, y, z] = a.xyz(0)?;
        ScriptCommand::SetPosition { x, y, z }
    });
    command!(vm, m, "set_rotation", |a| {
        let [x, y, z] = a.xyz(0)?;
        ScriptCommand::SetRotation { x, y, z }
    });
    command!(vm, m, "set_scale", |a| {
        let [x, y, z] = a.xyz(0)?;
        ScriptCommand::SetScale { x, y, z }
    });
    command!(vm, m, "set_scale_uniform", |a| {
        let s = a.f32(0, "scale")?;
        ScriptCommand::SetScale { x: s, y: s, z: s }
    });
    command!(vm, m, "translate", |a| {
        let [x, y, z] = a.xyz(0)?;
        ScriptCommand::Translate { x, y, z }
    });
    command!(vm, m, "rotate", |a| {
        let [x, y, z] = a.xyz(0)?;
        ScriptCommand::Rotate { x, y, z }
    });
    command!(vm, m, "look_at", |a| {
        let [x, y, z] = a.xyz(0)?;
        ScriptCommand::LookAt { x, y, z }
    });
    command!(vm, m, "goto_camera_preset", |a| ScriptCommand::GotoCameraPreset {
        name: a.str(0, "name")?,
    });

    // -- Parent transform --
    command!(vm, m, "parent_set_position", |a| {
        let [x, y, z] = a.xyz(0)?;
        ScriptCommand::ParentSetPosition { x, y, z }
    });
    command!(vm, m, "parent_set_rotation", |a| {
        let [x, y, z] = a.xyz(0)?;
        ScriptCommand::ParentSetRotation { x, y, z }
    });
    command!(vm, m, "parent_translate", |a| {
        let [x, y, z] = a.xyz(0)?;
        ScriptCommand::ParentTranslate { x, y, z }
    });

    // -- Child transform, by the child's name --
    command!(vm, m, "set_child_position", |a| {
        let name = a.str(0, "name")?;
        let [x, y, z] = a.xyz(1)?;
        ScriptCommand::ChildSetPosition { name, x, y, z }
    });
    command!(vm, m, "set_child_rotation", |a| {
        let name = a.str(0, "name")?;
        let [x, y, z] = a.xyz(1)?;
        ScriptCommand::ChildSetRotation { name, x, y, z }
    });
    command!(vm, m, "child_translate", |a| {
        let name = a.str(0, "name")?;
        let [x, y, z] = a.xyz(1)?;
        ScriptCommand::ChildTranslate { name, x, y, z }
    });
}
