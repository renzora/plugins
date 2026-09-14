//! Lua scripting for Renzora.
//!
//! The engine ships a scripting *system* — hooks, a command vocabulary, the
//! queue that applies commands to the world — and no interpreter. This plugin
//! supplies one. Drop it in `plugins/` and `.lua` files run; leave it out and
//! the same engine runs a game with no scripting at all, carrying none of the
//! cost.
//!
//! That is the point of the arrangement, and it generalises: a Wren or Python
//! plugin would implement the same [`Backend`] trait and claim its own
//! extensions, and neither the engine nor this crate would need to change. The
//! domain crates' script functions — `apply_force`, `nav_set_destination`,
//! `tr` — arrive as *declarations* over the boundary, so a second language gets
//! all of them without `renzora_physics` knowing it exists.
//!
//! ## Layout
//!
//! | module | what it is |
//! |---|---|
//! | [`interp`] | the interpreter, moved out of the engine largely unchanged |
//! | [`buffers`] | thread-local command/draw buffers the Lua bindings push into |
//!
//! There used to be a third, `host`, holding a call table in a thread-local so a
//! Lua `get` could read the world mid-hook. It is gone. That table existed only
//! because a C-ABI plugin could not reach the engine's own thread-locals, and a
//! native one calls [`renzora::get_handler`] directly. Its doc-comment
//! noted the names "deliberately match the engine's old `get_handler::call_*`",
//! which is what reduced deleting it to a change of import path.
//!
//! [`Backend`]: renzora::ScriptBackend

use bevy::prelude::*;
use renzora::{AppScriptBackendExt, CreateMenuItem, RegisterCreateMenuItem};

mod buffers;
mod interp;

// `mod tests` is gone. It was 395 lines of C-ABI scaffolding — `sys::ByteSink`,
// `extern "C"` callbacks, a hand-built `ScriptHostCalls` table — faking a
// boundary that no longer exists; a native backend calls the engine's
// `get_handler` thread-locals directly, so there is nothing left to fake.
//
// Not re-written in the new shape because nothing can run it: the SDK compiles a
// plugin's lib only, and `cargo test` inside a plugin directory is the forbidden
// thing that resolves a fresh Bevy from crates.io. Worth restoring from history
// once `check_plugins` grows a `--test` mode.

pub struct LuaPlugin;

impl Plugin for LuaPlugin {
    fn build(&self, app: &mut App) {
        app.add_script_backend(interp::LuaBackend::default());
        // The Assets panel's Add menu and the hierarchy's Attach submenu. Both
        // used to offer **Lua Script** unconditionally, because the entry was a
        // variant of an editor enum rather than something the interpreter
        // contributed, so an editor without this plugin would happily write a
        // `.lua` that nothing could run. Registered here, the row exists exactly
        // when the interpreter does.
        app.register_create_menu_item(
            CreateMenuItem::new("lua", "new_script", "lua", starter_lua)
                .label("assets.new.lua", "Lua Script")
                .icon("code")
                .folder("scripts")
                .attaches(),
        );
    }
}

/// Starter contents for a new `.lua` script.
///
/// Lives here, beside the hook vocabulary it demonstrates, so the two places
/// that create scripts — the Assets panel's New menu and the hierarchy's
/// right-click Attach — write the same file. The same reason
/// `renzora_blueprint::starter_blueprint_json` lives in the blueprint crate.
///
/// `boilerplate` off gives a bare comment: Lua needs no skeleton to be a valid
/// script, so "minimal" really is almost empty here — unlike Rust, which needs
/// its entry-point macro either way.
pub fn starter_lua(boilerplate: bool) -> String {
    if !boilerplate {
        return "-- New Lua script\n".to_string();
    }
    // Both hooks, and the one thing about transforms that catches everyone: the
    // `position_*` globals are read-only inputs, so moving an entity means
    // calling a function.
    concat!(
        "-- Attached to an entity. The engine calls these hooks; delete the\n",
        "-- ones you don't need.\n",
        "\n",
        "function on_ready()\n",
        "    -- Once, when the entity's scripts start.\n",
        "end\n",
        "\n",
        "function on_update()\n",
        "    -- Every frame. `delta` is seconds since the last one.\n",
        "    --\n",
        "    -- `position_x`, `rotation_y`, … are read-only inputs refreshed each\n",
        "    -- frame — assigning to them does nothing. Move an entity by calling\n",
        "    -- translate() / set_position(), or through physics.\n",
        "    local speed = 5.0\n",
        "    translate(input_x * speed * delta, 0, input_y * speed * delta)\n",
        "end\n",
    )
    .to_string()
}


renzora::plugin!(LuaPlugin);
