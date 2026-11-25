//! Moving the script's entity, its parent, and its named children.
//!
//! Every function taking a position accepts three numbers or one vector, so
//! `Call("set_position", 1, 2, 3)` and `Call("set_position", new Vector3(1, 2,
//! 3))` both work. The C# wrappers for the self and parent transforms do not
//! come through here at all (they use the native table's `transform` entry);
//! these are their by-name twins, and the child transforms.

use renzora::ScriptCommand;

use super::{push, Args, Out};

pub fn call(name: &str, a: &Args) -> Option<Out> {
    let xyz = |i| a.xyz(i);
    Some(match name {
        "set_position" => xyz(0).and_then(|[x, y, z]| push(ScriptCommand::SetPosition { x, y, z })),
        "set_rotation" => xyz(0).and_then(|[x, y, z]| push(ScriptCommand::SetRotation { x, y, z })),
        "set_scale" => xyz(0).and_then(|[x, y, z]| push(ScriptCommand::SetScale { x, y, z })),
        "set_scale_uniform" => a
            .f32(0, "scale")
            .and_then(|s| push(ScriptCommand::SetScale { x: s, y: s, z: s })),
        "translate" => xyz(0).and_then(|[x, y, z]| push(ScriptCommand::Translate { x, y, z })),
        "rotate" => xyz(0).and_then(|[x, y, z]| push(ScriptCommand::Rotate { x, y, z })),
        "look_at" => xyz(0).and_then(|[x, y, z]| push(ScriptCommand::LookAt { x, y, z })),
        "goto_camera_preset" => a
            .str(0, "name")
            .and_then(|name| push(ScriptCommand::GotoCameraPreset { name })),

        "parent_set_position" => xyz(0).and_then(|[x, y, z]| push(ScriptCommand::ParentSetPosition { x, y, z })),
        "parent_set_rotation" => xyz(0).and_then(|[x, y, z]| push(ScriptCommand::ParentSetRotation { x, y, z })),
        "parent_translate" => xyz(0).and_then(|[x, y, z]| push(ScriptCommand::ParentTranslate { x, y, z })),

        "set_child_position" => (|| {
            let name = a.str(0, "name")?;
            let [x, y, z] = a.xyz(1)?;
            push(ScriptCommand::ChildSetPosition { name, x, y, z })
        })(),
        "set_child_rotation" => (|| {
            let name = a.str(0, "name")?;
            let [x, y, z] = a.xyz(1)?;
            push(ScriptCommand::ChildSetRotation { name, x, y, z })
        })(),
        "child_translate" => (|| {
            let name = a.str(0, "name")?;
            let [x, y, z] = a.xyz(1)?;
            push(ScriptCommand::ChildTranslate { name, x, y, z })
        })(),
        _ => return None,
    })
}
