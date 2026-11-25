//! JavaScript and TypeScript scripting for Renzora, on Boa.
//!
//! The engine ships a scripting *system* and no interpreter; this plugin
//! supplies one, beside (not instead of) Lua and Python. A project can have
//! `.lua`, `.py`, `.js` and `.ts` entities side by side, because backends are
//! routed by file extension.
//!
//! Boa rather than V8 or QuickJS so that installing the plugin needs no C
//! compiler: a native plugin is compiled on the machine that installs it, and
//! Boa is Rust all the way down. The price is speed (an interpreter with no JIT)
//! and a few corners of the language (see the README). TypeScript goes through
//! oxc first, which strips the types and nothing else.
//!
//! The script-facing API is the Lua plugin's, function for function and under
//! the same names, so a script ports between the languages by changing syntax
//! only. Where they differ, JavaScript's own conventions win: indices start at
//! 0, vectors are `{x, y, z}` objects, several results come back as an object,
//! `console.log` is how you print.
//!
//! ## Layout
//!
//! | module | what it is |
//! |---|---|
//! | [`backend`] | the `ScriptBackend`: which scripts are live, and the hooks |
//! | [`runtime`] | the interpreter itself, one per thread, and each script's scope |
//! | [`transpile`] | TypeScript to JavaScript, and `export` removal |
//! | [`api`] | every engine function a script can call, one module per subject |
//! | [`bindings`] | functions other plugins declare through `ScriptExtension` |
//! | [`context`] | the per-call variables (`delta`, `position_x`, ...) and props |
//! | [`convert`] | JavaScript values to and from the engine's value types |
//! | [`frame`] | the context of the hook currently running, for the input reads |
//! | [`buffers`] | where API calls put the commands and draws they produce |
//! | [`syntax`] | how the code editor colours `.js` and `.ts` |

use bevy::prelude::*;
use renzora::{AppCodeSyntaxExt, AppScriptBackendExt, CreateMenuItem, RegisterCreateMenuItem};

mod api;
mod backend;
mod bindings;
mod buffers;
mod context;
mod convert;
mod frame;
mod runtime;
mod syntax;
mod transpile;

pub struct JavaScriptPlugin;

impl Plugin for JavaScriptPlugin {
    fn build(&self, app: &mut App) {
        app.add_script_backend(backend::JavaScriptBackend::default());
        // Colouring for the code editor, which knows no language a plugin
        // brings until the plugin describes it.
        app.add_code_syntax(syntax::JAVASCRIPT);
        app.add_code_syntax(syntax::TYPESCRIPT);
        // The Assets panel's Add menu and the hierarchy's Attach submenu, as
        // the Lua and Python plugins do: registered here, the rows exist
        // exactly when an interpreter that can run the files does.
        app.register_create_menu_item(
            CreateMenuItem::new("javascript", "new_script", "js", starter_javascript)
                .label("assets.new.javascript", "JavaScript Script")
                .icon("code")
                .folder("scripts")
                .attaches(),
        );
        app.register_create_menu_item(
            CreateMenuItem::new("typescript", "new_script", "ts", starter_typescript)
                .label("assets.new.typescript", "TypeScript Script")
                .icon("code")
                .folder("scripts")
                .attaches(),
        );
    }
}

/// Starter contents for a new `.js` script.
///
/// The JavaScript twin of the Lua plugin's `starter_lua`, line for line, so
/// the starters teach the same two things: which hooks exist, and that the
/// `position_*` variables are read-only inputs rather than handles.
pub fn starter_javascript(boilerplate: bool) -> String {
    if !boilerplate {
        return "// New JavaScript script\n".to_string();
    }
    concat!(
        "// Attached to an entity. The engine calls these hooks; delete the\n",
        "// ones you don't need.\n",
        "\n",
        "function on_ready() {\n",
        "  // Once, when the entity's scripts start.\n",
        "}\n",
        "\n",
        "function on_update() {\n",
        "  // Every frame. `delta` is seconds since the last one.\n",
        "  //\n",
        "  // `position_x`, `rotation_y`, ... are read-only inputs refreshed each\n",
        "  // frame: assigning to them does nothing. Move an entity by calling\n",
        "  // translate() / set_position(), or through physics.\n",
        "  const speed = 5.0;\n",
        "  translate(input_x * speed * delta, 0, input_y * speed * delta);\n",
        "}\n",
    )
    .to_string()
}

/// Starter contents for a new `.ts` script: the JavaScript starter with types,
/// and the `export {}` that keeps its names out of every other script's way in
/// the editor (see `transpile`).
pub fn starter_typescript(boilerplate: bool) -> String {
    if !boilerplate {
        return "// New TypeScript script\nexport {};\n".to_string();
    }
    concat!(
        "// Attached to an entity. The engine calls these hooks; delete the\n",
        "// ones you don't need. For autocomplete, put the JavaScript plugin's\n",
        "// renzora.d.ts in the project (see the plugin's README).\n",
        "export {};\n",
        "\n",
        "function on_ready(): void {\n",
        "  // Once, when the entity's scripts start.\n",
        "}\n",
        "\n",
        "function on_update(): void {\n",
        "  // Every frame. `delta` is seconds since the last one.\n",
        "  //\n",
        "  // `position_x`, `rotation_y`, ... are read-only inputs refreshed each\n",
        "  // frame: assigning to them does nothing. Move an entity by calling\n",
        "  // translate() / set_position(), or through physics.\n",
        "  const speed: number = 5.0;\n",
        "  translate(input_x * speed * delta, 0, input_y * speed * delta);\n",
        "}\n",
    )
    .to_string()
}

// Runtime, not Editor: the interpreter is what runs a game's `.js` and `.ts`
// scripts, so it has to be present in Play and in every exported game, not
// only in the editor's own preview.
renzora::plugin!(JavaScriptPlugin, Runtime);
