# C# Scripting

C# scripting for Renzora. Attach a `.cs` script to an entity and the engine runs it, with the same hooks and the same engine functions as Lua, Python and JavaScript, spelled the C# way: a script is a class deriving `Script`, a hook is an override, a prop is a field marked `[Prop]`.

```csharp
// scripts/Spinner.cs
using Renzora;

public class Spinner : Script
{
    [Prop(Hint = "degrees per second")]
    public float Speed = 90f;

    public override void OnUpdate()
    {
        Rotate(0, Speed * Delta, 0);
    }
}
```

The Assets panel's Add menu and the hierarchy's Attach submenu gain **C# Script**, with a starter.

## Requirements

C# is not compiled into the plugin the way the other languages are: a C# compiler and runtime are far too large for that, and every C# developer already has them. The plugin uses the machine's own .NET in the editor, and an exported game brings its own (see *Exporting*).

| To | You need |
|---|---|
| Write and edit C# scripts (the editor, Play) | the **.NET 10 SDK** (or newer), from <https://dotnet.microsoft.com/download/dotnet/10.0> |
| Play an exported game | nothing, with the runtime bundled (the default); otherwise the **.NET 10 runtime** |

.NET 10 is the long-term-support release, supported until November 2028.

.NET is found through `DOTNET_ROOT` when that is set, else in its default place (`C:\Program Files\dotnet` on Windows, `/usr/local/share/dotnet` on macOS, `/usr/share/dotnet` or `/usr/lib/dotnet` on Linux, or `~/.dotnet`). A newer major version runs the scripts too. Without .NET, every C# script reports what to install instead of running; the rest of the project is unaffected, and a project with no `.cs` file never starts .NET at all. Restart the editor after installing .NET.

Building needs no network unless the project uses NuGet packages (below): the plugin's own C# references none.

## NuGet packages

List the packages your scripts use in `csharp.toml` at the project root, beside `project.toml`:

```toml
[packages]
"Newtonsoft.Json" = "13.0.3"
"Humanizer.Core" = "2.14.1"
```

Then `using Newtonsoft.Json;` in any script. Saving the file rebuilds the scripts, as editing a script does. The first build after adding a package downloads it from nuget.org (or the sources your NuGet configuration names) into the machine's NuGet cache; later builds, on any project, use the cache and need no network.

- **Pin a version.** A floating version (`"13.*"`) is resolved once and then kept, since a build is reused for as long as the scripts and this list are unchanged.
- **Quotes are optional.** `Newtonsoft.Json = "13.0.3"` works too, although TOML proper would read the dots as nested tables.
- **A package that cannot be found** stops every C# script, like a compiler error, and says so: `error NU1101: Unable to find package ...`.
- **Packages with native libraries** (SQLite, image codecs) work: their library for this platform is found from the build's `.deps.json`. A package that finds files beside itself through `Assembly.Location` gets an empty string, because packages are loaded from memory so a later build can replace their files.
- **Targets `net10.0`.** A package that only supports a newer .NET does not install.

## How a script is found

Every `.cs` file under `scripts/` (subfolders included) compiles together into one assembly, so a script may use a helper class, an `enum` or an extension method from another file. A file attached to an entity runs one class from that file:

1. the class deriving `Script` that is named after the file (`Spinner` in `Spinner.cs`, case ignored), else
2. the only class deriving `Script` in the file.

A file with several script classes and none named after it is an error that names them. The plugin learns which class lives in which file from the compiler's own debug information, not by reading your source, so namespaces, nesting, `partial` classes and comments make no difference.

The starter declares a `file class NewScript : Script`. `file` makes the name private to its file, so any number of new scripts can each have a `NewScript` without colliding in the shared assembly. Rename the class (and drop `file`) when another script needs to use it.

One instance of the class is created per entity it is attached to, so instance fields are per-entity state.

## Hooks

Override the ones you need.

| Hook | When |
|---|---|
| `OnReady()` | once, when the entity's scripts start |
| `OnUpdate()` | every frame |
| `OnFixedUpdate()` | every fixed physics step; `Delta` is the step |
| `OnUi(string name, Args args, ulong entity)` | a UI markup callback fired |
| `OnDraw(Canvas g)` | every frame, to draw 2D shapes on `g` (below) |
| `OnRpc(string name, Args args, ulong from)` | a networked RPC arrived |
| `OnAnimationEvent(string name, ulong entity)` | animation playback crossed a marker |
| `OnHttp(string callback, int status, string body)` | an `HttpGet` / `HttpPost` answered |
| `OnPlayerJoined(ulong id)` / `OnPlayerLeft(ulong id)` | a peer connected or left (server only) |
| `OnSceneLoaded(string path)` / `OnSceneLoadFailed(string path, string error)` | a scene load finished |
| `OnEvent(string name, Args args)` | a broadcast event (`Emit`, or a plugin answering a `Call`) |

`OnReady` always runs before any other hook of the same instance, even when the scripts were still compiling at the moment the engine started the entity.

`Args` holds names to values (`long`, `double`, `bool`, `string`, `Vector3`) with typed getters that convert between number types: `args.Float("damage")`, `args.String("boss")`, `args.Get("level", 1)`.

## Props

A field (or a property with a getter and setter) marked `[Prop]` is a prop: the inspector edits it per entity, and its initial value is the default.

```csharp
[Prop(Hint = "metres per second", Tab = "Movement")]
public float Speed = 5f;

[Prop(Label = "Start colour")]
public Color Tint = Color.Hex("#ff8800");
```

Types: `float`, `double`, `int`, `long`, `bool`, `string`, `Vector2`, `Vector3`, `Color`. The label is made from the name (`MoveSpeed` and `move_speed` both show as "Move Speed") unless `Label` says otherwise. The inspector's value is written into the field before a hook, and a value the script changes is shown back in the inspector.

## This frame

Read-only members of `Script`, refreshed before every hook: `Delta`, `Elapsed`, `EntityId`, `EntityName`, `Position`, `Rotation` (Euler degrees, the unit `SetRotation` and `Rotate` take), `Orientation` (a `Quaternion`), `Scale`, `Forward`, `Right`, `Up`, `InputMovement`, `MousePosition`, `MouseDelta`, `MouseScroll`, `MouseLeft` / `MouseRight` / `MouseMiddle`, `MouseLeftJustPressed`, `MouseRightJustPressed`, `CameraYaw`, `CameraEv`, `ProjectWidth` / `ProjectHeight`, `HasParent`, `ParentPosition`, `IsColliding`, `Health`, `MaxHealth`, `TimersFinished` (and `TimerFinished(name)`), and the first gamepad as `GamepadLeft`, `GamepadRight`, `GamepadLeftTrigger`, `GamepadRightTrigger`, `Gamepad("south")`.

Setting them does nothing (they have no setter): move an entity with `SetPosition` / `Translate`, or through physics.

## Engine functions

Every function the Lua plugin offers, as a method of `Script`, PascalCase:

| Area | Methods |
|---|---|
| Transform | `SetPosition`, `SetRotation`, `SetScale`, `SetScaleUniform`, `Translate`, `Rotate`, `LookAt`, `GotoCameraPreset`, `ParentSetPosition`, `ParentSetRotation`, `ParentTranslate`, `SetChildPosition`, `SetChildRotation`, `ChildTranslate` |
| Keyboard and actions | `IsKeyPressed`, `IsKeyJustPressed`, `IsKeyJustReleased`, `InputButtonPressed`, `InputButtonJustPressed`, `InputButtonJustReleased`, `InputAxis1D`, `InputAxis2D` |
| Gamepads | `GamepadCount`, `GamepadConnected`, `GamepadAxis`, `GamepadLeftStick`, `GamepadRightStick`, `GamepadButton`, `GamepadButtonJustPressed` |
| Touch | `TouchCount`, `TouchId`, `TouchPosition`, `TouchStart`, `TouchDelta`, `TouchJustPressed`, `TouchJustReleased`, `PinchDelta`, `RotateDelta`, `TapCount`, `TapPosition` |
| Saves | `SaveSet`, `SaveGet`, `SaveHas`, `SaveRemove`, `SaveClear`, `SaveKeys`, `SaveWrite`, `SaveLoad`, `SaveDelete`, `SaveExists`, `SaveSlots` |
| Audio | `PlaySound`, `PlaySoundLooping`, `PlayMusic`, `StopMusic`, `StopAllSounds`, `PlayAudio` |
| Physics | `ApplyForce`, `ApplyImpulse`, `SetVelocity`, `SetGravityScale` |
| Timers | `StartTimer`, `StopTimer` |
| Console and debug | `Log`, `LogWarning`, `LogError`, `PrintLog`, `DrawLine` (and `Console.WriteLine`) |
| Rendering | `SetVisibility`, `SetMaterialColor` |
| Animation | `PlayAnimation`, `StopAnimation`, `PauseAnimation`, `ResumeAnimation`, `SetAnimationSpeed`, `SeekAnimation`, `GetAnimationTime`, `IsAnimationPlaying`, `CrossfadeAnimation`, `SetAnimParam`, `SetAnimBool`, `TriggerAnim`, `SetLayerWeight` |
| Cursor and camera | `LockCursor`, `UnlockCursor`, `ScreenShake` |
| Entities and scenes | `SpawnEntity`, `SpawnPrimitive`, `DespawnSelf`, `DespawnByPrefix`, `LoadScene` |
| Environment | `SetSunAngles`, `SetFog` |
| Reflection | `Set`, `SetOn`, `Get`, `Get<T>`, `GetOn`, `GetFloat`, `GetBool`, `GetString`, `GetVector3`, `GetComponent`, `GetComponentOn`, `GetComponents`, `GetComponentsOn`, `HasComponent`, `HasComponentOn` |
| Events and network | `Action`, `ActionOn`, `Emit`, `Rpc`, `HttpGet`, `HttpPost`, `NetIsServer`, `NetIsClient`, `NetIsConnected`, `NetPlayerCount` |
| JSON | `JsonParse`, `JsonEncode` (and `System.Text.Json`) |
| Loading | `AssetProgress`, `SceneLoadState`, `IsLoading`, `IsLoaded` |
| Math | `Vector2`, `Vector3`, `Color`, `Quaternion`, `Mathf.Lerp` / `Clamp` / `MoveTowards` / `SmoothStep`, `Lerp`, `Clamp` (and `MathF`, `Random`) |
| Anything by name | `Call(name, args...)`, `Call<T>(...)`, `Tr(key)` |

`Vector3` has the operators (`a + b`, `v * 2`, `-v`), `Dot`, `Cross`, `Distance`, `Lerp`, `Length`, `Normalized`, `Up` / `Forward` / ..., and converts to and from `System.Numerics.Vector3`.

Event arguments are a dictionary or an anonymous object: `Emit("boss_died", new { boss = "ogre", hp = 0 })`. A float is sent as a float and an int as an int, so the receiver gets the type you wrote.

### Functions other plugins declare

Plugins add script functions by name (navigation's `nav_set_destination`, physics' extras, the MySQL plugin's `mysql_query`, MongoDB's `mongo_find`, `tr` for translations). C# calls them with `Call`, and the answer to a query arrives as `OnEvent`, as in every language:

```csharp
public override void OnReady()
{
    Call("mysql_query_with", "SELECT name, score FROM players WHERE level = ?", new[] { 3 }, "scores");
    HttpGet("https://example.com/motd.json", "motd");
}

public override void OnEvent(string name, Args args)
{
    if (name == "scores" && args.Bool("ok"))
        foreach (var row in (List<object?>)JsonParse(args.String("rows"))!)
            Log(row);
}

public override void OnHttp(string callback, int status, string body)
{
    if (callback == "motd" && status == 200)
        Log(body);
}
```

A list or object given where a plugin function wants a string arrives as its JSON text, so `new[] { 3 }` above needs no encoding. `Call` also reaches every built-in function by its engine name (`Call("play_sound", "boom.wav")`), and returns what it returns: `null`, `bool`, `long`, `double`, `string`, `Vector2` / `Vector3` / `Color`, a `List<object?>` or a `Dictionary<string, object?>`.

### Drawing: `OnDraw(Canvas g)`

`g.Width` and `g.Height` are the surface in pixels. `g.Line(x1, y1, x2, y2, color, thickness = 2)`, `g.Arc(cx, cy, r, start, end, color, thickness = 2)`, `g.Circle(cx, cy, r, color)`, `g.Rect(x, y, w, h, color)`, `g.Text(x, y, text, size, color)`, `g.Triangle(x1, y1, x2, y2, x3, y3, color)`, `g.Poly(points, color)`. A colour is a `Color` or `"#rrggbb"` / `"#rrggbbaa"` text.

## Building and hot reload

The plugin builds with the SDK's own `dotnet build`, off the main thread:

- **Its C# host** (`host/`, the API above), once per machine, into `build/csharp/` beside the plugin.
- **Your scripts**, whenever a `.cs` file in the project or `csharp.toml` changes, through a project it generates at `<project>/.renzora/csharp/Scripts.csproj`. Open that file in Visual Studio, Rider or VS Code for completion against the Renzora API. It is rewritten on every build, so do not edit it.

The first build starts when the project opens, so it has usually finished before Play. Builds are named by a hash of the sources, and the editor and the Play window share them, so pressing Play does not compile twice, and undoing an edit costs no build at all. On a typical machine a rebuild after an edit takes one to three seconds.

When a build lands, the new code replaces the old in a fresh, unloadable assembly context: every C# script restarts on fresh instances (instance state resets), the inspector's props are applied again, and `OnReady` runs again. Until a build lands, scripts keep running the previous one.

## Errors

A compiler error is reported against the script that has it, in the compiler's own words and position:

```text
spinner.cs(12,5): error CS1002: ; expected
```

All scripts build together, so while any file has an error, no C# script runs; the others report which file is holding them up. An exception thrown in a hook is reported with its type, message and the lines of your scripts it passed through:

```text
spinner.cs on_update: NullReferenceException: Object reference not set to an instance of an object.
  at Spinner.OnUpdate (spinner.cs:21)
```

An engine function given the wrong kind of argument through `Call` throws a `ScriptException` naming the function and the parameter.

## Exporting

A project with `.cs` files has C# Scripting ticked in the Export window's Plugins tab, and its scripts ship inside the game: when the export starts, the plugin hands it the current build of the scripts (with their NuGet packages), its C# host and, by default, the .NET runtime, and the export packs them into the game's archive. A lean single-binary export stays a single file.

.NET can only load assemblies from files, so the game writes the build out once, on its first launch, into the player's cache folder, and loads it from there:

| | |
|---|---|
| Windows | `%LOCALAPPDATA%\<game>\csharp-<build>\` |
| macOS | `~/Library/Caches/<game>/csharp-<build>/` |
| Linux | `$XDG_CACHE_HOME/<game>/csharp-<build>/` (`~/.cache` by default) |

Later launches find it and unpack nothing. An updated game has a different `<build>` and unpacks beside the old one, which is then removed.

- **Scripts that do not build stop the export**, with the compiler's errors, rather than shipping a game whose logic is missing.
- **A build out of date is rebuilt** when the export starts, which is the one time an export waits on `dotnet build`.
- **Web and mobile exports** refuse C# with a message; untick C# Scripting to export without it.

### The .NET runtime

Windows does not come with the .NET these scripts run on, so the export bundles it. **Bundle the .NET runtime**, under *csharp* in the Plugins tab, is on by default: the export packs this machine's .NET 10 runtime (about 76 MB before compression: `hostfxr` and `Microsoft.NETCore.App`, with Microsoft's licence and notices; .NET is MIT licensed) with the build, and the game runs on that whatever the player has installed.

Switched off, the game is that much smaller and uses the player's .NET. A player without .NET 10 (or newer) is asked when the game starts, in the system's own dialog, and **Yes** opens Microsoft's download page; the game keeps running meanwhile, with its C# doing nothing until .NET is installed.

A runtime can only be bundled for the platform the editor runs on, since it is copied from this machine. Exporting for another desktop platform with the switch on logs a warning and falls back to asking.

## Limitations

- **The console does not run C#.** Evaluating C# expressions needs Roslyn's scripting API, which is not part of .NET itself. Use `Log` in a script.
- **Engine calls happen on the hook's thread.** `async` code, `Task.Run` and threads are C#, and work, but an engine function called from anywhere but the hook itself throws, because there is nothing to deliver it to. Logging (`Log`, `Console.WriteLine`) from another thread is kept and printed with the next hook. An exception left unhandled on a thread you start ends the process, as it does in any .NET program.
- **One assembly.** A compile error anywhere stops every C# script until it is fixed, and a rebuild restarts every C# script, not only the one edited.
- **An exported game writes its C# to the player's cache on first launch.** .NET loads assemblies from files, so the build packed in the game is unpacked once (see *Exporting*); that first launch takes a moment longer.
- **Not on the web or mobile.** The plugin hosts the desktop .NET runtime, which a browser or a phone does not have.
- **Entity ids are `ulong`.** `EntityId` and the `entity` arguments are the engine's `Entity::to_bits()`.

## Files

| | |
|---|---|
| `src/backend.rs` | the `ScriptBackend`: generations, instances, hooks, reload and eviction |
| `src/project.rs` | noticing edits, building off the main thread, the loaded build's table |
| `src/compile.rs` | `dotnet build` of the host and the scripts, the generated project, error parsing |
| `src/dotnet.rs` | finding .NET and starting its runtime through `hostfxr` |
| `src/bridge.rs` / `src/abi.rs` | the calls into C# and the callbacks out of it, and the structs both sides share |
| `src/api/` | every engine function, by name, one module per subject |
| `src/bindings.rs` | functions declared by other plugins |
| `src/context.rs`, `src/convert.rs` | the frame's values and JSON conversions |
| `host/Bridge.cs`, `host/Native.cs` | the C# entry points and the native table |
| `host/Loader.cs` | load contexts, which class each file runs, props |
| `host/Script*.cs` | the `Script` base class: hooks, the frame's values, the API |
| `host/Types.cs`, `host/Args.cs`, `host/Json.cs` | `Vector3` and friends, `Args`, `[Prop]`, `Canvas`, the JSON encoding |

**Scope:** Runtime, so it runs in Play. Desktop only.
