//! C# scripting for Renzora, on .NET 10: the machine's in the editor, the
//! machine's or a bundled one in a game.
//!
//! The engine ships a scripting *system* and no interpreter; this plugin
//! supplies one, beside (not instead of) Lua, Python and JavaScript. A project
//! can have `.lua`, `.py`, `.js` and `.cs` entities side by side, because
//! backends are routed by file extension.
//!
//! Unlike the other language plugins, the language does not live inside the
//! plugin. C# needs a compiler and a runtime that are far too large to compile
//! into a plugin from source, and every C# developer already has both: the
//! .NET SDK. So this plugin *hosts* the installed runtime (through `hostfxr`,
//! Microsoft's documented native-hosting entry) and *drives* the installed
//! compiler (`dotnet build`), and ships only the glue: this crate, and the C#
//! host assembly in `host/`, as source.
//!
//! The script-facing API is the Lua plugin's, function for function, with C#
//! spelling: `translate(x, y, z)` is `Translate(x, y, z)`, `delta` is `Delta`,
//! a hook is an override (`public override void OnUpdate()`), and a prop is a
//! field marked `[Prop]`. Anything without an idiomatic wrapper, including
//! every function another plugin declares, is `Call("name", args...)`.
//!
//! ## Layout
//!
//! | module | what it is |
//! |---|---|
//! | [`backend`] | the `ScriptBackend`: generations, instances, the hooks |
//! | [`project`] | noticing edits, building off the main thread, the loaded build |
//! | [`compile`] | `dotnet build` of the host and of a project's scripts |
//! | [`dotnet`] | finding .NET and starting its runtime through `hostfxr` |
//! | [`bridge`] | the calls into C#, and the callbacks C# calls back with |
//! | [`abi`] | the structs both sides agree on, byte for byte |
//! | [`api`] | every engine function a script can call, by name |
//! | [`bindings`] | functions other plugins declare through `ScriptExtension` |
//! | [`context`] | the engine's context, flattened for C# |
//! | [`convert`] | JSON to and from the engine's value types |
//! | [`frame`] | the context of the hook currently running, for the input reads |
//! | [`buffers`] | where callbacks put the commands and draws they produce |
//! | [`syntax`] | how the code editor colours `.cs` |
//!
//! The C# half is `host/`: `Bridge.cs` (the entry points), `Loader.cs` (load
//! contexts, which class is which file, props), `Script*.cs` (the API scripts
//! derive from).

use bevy::prelude::*;
use renzora::{AppCodeSyntaxExt, AppScriptBackendExt, CreateMenuItem, RegisterCreateMenuItem};

mod abi;
mod api;
mod backend;
mod bindings;
mod bridge;
mod buffers;
mod compile;
mod context;
mod convert;
mod dotnet;
mod export;
mod frame;
mod project;
mod prompt;
mod unpack;
mod syntax;

pub struct CSharpPlugin;

impl Plugin for CSharpPlugin {
    fn build(&self, app: &mut App) {
        app.add_script_backend(backend::CSharpBackend::default());
        // Tell the exporter about C#: a project with `.cs` files needs this
        // plugin in its game, and the game needs the build shipped beside it
        // (see `export`). Declared here rather than known to the engine, which
        // names no language a plugin brings.
        renzora::register_plugin_export(renzora::PluginExport {
            plugin: "csharp".into(),
            script_extensions: vec!["cs".into()],
            stage: Some(export::stage),
            options: export::options(),
        });
        // Colouring for the code editor, which knows no language a plugin
        // brings until the plugin describes it.
        app.add_code_syntax(syntax::CSHARP);
        // The Assets panel's Add menu and the hierarchy's Attach submenu, as
        // the other language plugins do: registered here, the rows exist
        // exactly when something that can run the files does.
        app.register_create_menu_item(
            CreateMenuItem::new("csharp", "new_script", "cs", starter_csharp)
                .label("assets.new.csharp", "C# Script")
                .icon("code")
                .folder("scripts")
                .attaches(),
        );
    }
}

/// Starter contents for a new `.cs` script.
///
/// The C# twin of the other plugins' starters: which hooks exist, that the
/// `Position`-style members are read-only inputs rather than handles, and how a
/// prop is declared.
///
/// The class is `file`-scoped because the starter cannot know the name the new
/// file will be given, and two scripts created from it would otherwise both
/// declare `NewScript` in the one assembly every script compiles into, which
/// is a compiler error. A `file class` is private to its file, so any number of
/// them coexist; the plugin finds a file's class from the file, not the name.
pub fn starter_csharp(boilerplate: bool) -> String {
    if !boilerplate {
        return concat!(
            "using Renzora;\n",
            "\n",
            "file class NewScript : Script\n",
            "{\n",
            "}\n",
        )
        .to_string();
    }
    concat!(
        "using Renzora;\n",
        "\n",
        "// Attached to an entity. The engine calls these overrides; delete the\n",
        "// ones you don't need. `file` keeps the class name private to this file,\n",
        "// so any number of scripts may call theirs NewScript; rename it and drop\n",
        "// `file` when another script needs to use this class.\n",
        "file class NewScript : Script\n",
        "{\n",
        "    // A prop: edited in the inspector, and this value is its default.\n",
        "    [Prop(Hint = \"metres per second\")]\n",
        "    public float Speed = 5.0f;\n",
        "\n",
        "    public override void OnReady()\n",
        "    {\n",
        "        // Once, when the entity's scripts start.\n",
        "        Log($\"{EntityName} is ready\");\n",
        "    }\n",
        "\n",
        "    public override void OnUpdate()\n",
        "    {\n",
        "        // Every frame. `Delta` is seconds since the last one.\n",
        "        //\n",
        "        // `Position`, `Rotation`, ... are read-only inputs refreshed each\n",
        "        // frame: move the entity with Translate() / SetPosition(), or\n",
        "        // through physics.\n",
        "        Translate(InputMovement.X * Speed * Delta, 0, InputMovement.Y * Speed * Delta);\n",
        "    }\n",
        "}\n",
    )
    .to_string()
}

// Runtime, not Editor: this is what runs a game's `.cs` scripts, so it has to
// be present in Play and in a shipped game, not only in the editor's preview.
renzora::plugin!(CSharpPlugin, Runtime);
