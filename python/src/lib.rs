//! Python scripting for Renzora, on RustPython.
//!
//! The engine ships a scripting *system* and no interpreter; this plugin
//! supplies one, beside (not instead of) Lua. A project can have `.lua` and
//! `.py` entities side by side, because backends are routed by file extension.
//!
//! RustPython rather than CPython so that nobody has to install Python: the
//! interpreter is compiled into this plugin, which is the only way an exported
//! game can carry it. The price is speed (several times slower than CPython)
//! and the C extension ecosystem (no numpy). See the README.
//!
//! The script-facing API is the Lua plugin's, function for function and under
//! the same names, so a script ports between the two by changing syntax only.
//! Where the two languages differ, Python's own conventions win: indices start
//! at 0, several results come back as a tuple, vectors are `vec3` tuples.
//!
//! ## Layout
//!
//! | module | what it is |
//! |---|---|
//! | [`backend`] | the `ScriptBackend`: which scripts are live, and the hooks |
//! | [`runtime`] | the interpreter itself, one per thread, and each script's globals |
//! | [`api`] | every engine function a script can call, one module per subject |
//! | [`bindings`] | functions other plugins declare through `ScriptExtension` |
//! | [`context`] | the per-call variables (`delta`, `position_x`, ...) and props |
//! | [`convert`] | Python values to and from the engine's value types |
//! | [`frame`] | the context of the hook currently running, for the input reads |
//! | [`buffers`] | where API calls put the commands and draws they produce |

use bevy::prelude::*;
use renzora::{AppCodeSyntaxExt, AppScriptBackendExt, CodeSyntax, CreateMenuItem, RegisterCreateMenuItem};

mod api;
mod backend;
mod bindings;
mod buffers;
mod context;
mod convert;
mod frame;
mod runtime;

pub struct PythonPlugin;

impl Plugin for PythonPlugin {
    fn build(&self, app: &mut App) {
        app.add_script_backend(backend::PythonBackend::default());
        // A project with these files needs this plugin in its exported game.
        // Nothing else to ship: a game reads its scripts from the archive.
        renzora::register_plugin_export(renzora::PluginExport {
            plugin: "python".into(),
            script_extensions: vec!["py".into()],
            ..Default::default()
        });
        // Colouring for the code editor, which knows a language a plugin brings
        // only from the plugin's description of it. Triple-quoted strings span
        // lines, and capitalised names colour as types (classes, by PEP 8).
        app.add_code_syntax(CodeSyntax {
            keywords: &[
                "and", "as", "assert", "async", "await", "break", "class", "continue", "def",
                "del", "elif", "else", "except", "finally", "for", "from", "global", "if",
                "import", "in", "is", "lambda", "match", "case", "nonlocal", "not", "or",
                "pass", "raise", "return", "try", "while", "with", "yield", "True", "False",
                "None", "self",
            ],
            types: &[
                "int", "float", "str", "bool", "bytes", "list", "dict", "tuple", "set",
                "frozenset", "object",
            ],
            caps_are_types: true,
            line_comment: Some("#"),
            multiline_strings: &["\"\"\"", "'''"],
            ..CodeSyntax::new("Python", &["py"])
        });
        // The Assets panel's Add menu and the hierarchy's Attach submenu, as
        // the Lua plugin does: registered here, the row exists exactly when an
        // interpreter that can run the file does.
        app.register_create_menu_item(
            CreateMenuItem::new("python", "new_script", "py", starter_python)
                .label("assets.new.python", "Python Script")
                .icon("code")
                .folder("scripts")
                .attaches(),
        );
    }
}

/// Starter contents for a new `.py` script.
///
/// The Python twin of the Lua plugin's `starter_lua`, line for line, so the
/// two starters teach the same two things: which hooks exist, and that the
/// `position_*` variables are read-only inputs rather than handles.
pub fn starter_python(boilerplate: bool) -> String {
    if !boilerplate {
        return "# New Python script\n".to_string();
    }
    concat!(
        "# Attached to an entity. The engine calls these hooks; delete the\n",
        "# ones you don't need.\n",
        "\n",
        "def on_ready():\n",
        "    # Once, when the entity's scripts start.\n",
        "    pass\n",
        "\n",
        "def on_update():\n",
        "    # Every frame. `delta` is seconds since the last one.\n",
        "    #\n",
        "    # `position_x`, `rotation_y`, ... are read-only inputs refreshed each\n",
        "    # frame: assigning to them does nothing. Move an entity by calling\n",
        "    # translate() / set_position(), or through physics.\n",
        "    speed = 5.0\n",
        "    translate(input_x * speed * delta, 0, input_y * speed * delta)\n",
    )
    .to_string()
}

// Runtime, not Editor: the interpreter is what runs a game's `.py` scripts, so
// it has to be present in Play and in every exported game, not only in the
// editor's own preview.
renzora::plugin!(PythonPlugin, Runtime);
