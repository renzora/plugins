# Tracy

Streams the editor's live measurements to the [Tracy profiler](https://github.com/wolfpld/tracy): frame time, FPS, and whatever else the engine and your other plugins are measuring, plus one frame mark per frame so Tracy's timeline lines up with real frames.

Use it to answer "where is my frame going" and "did that change help", without adding any instrumentation to your own code.

## You need Tracy 0.14.x

Get it from the [Tracy releases page](https://github.com/wolfpld/tracy/releases) (`windows-0.14.1.zip`, `linux-0.14.1.zip`, `macos-0.14.1.zip`).

The version matters. Tracy refuses a connection across protocol versions, and the only thing it tells you is *"The client you are trying to connect to uses incompatible protocol version."* If you see that, you have the wrong Tracy, not a broken plugin.

## Turn it on

**Settings → Plugins → Tracy Profiler → Enable Tracy.**

It takes effect immediately, no restart. Until you switch it on the plugin does nothing at all: no profiler client, no network listener, nothing buffered.

Then start `tracy-profiler`, and the editor appears in its client list on `127.0.0.1:8086`. Click it to connect.

## What you get

Every measurement arrives as a named **plot** row on Tracy's timeline. Expand a row to see it over time and read its maximum, which is usually the interesting number: a stall shows up in the max and hides in the mean.

The switches under **Plots** control which groups are sent:

| Group | What it covers |
|---|---|
| Frame | FPS, frame time, frame count |
| Entity count | How many entities the world holds |
| CPU and memory | Process and system-wide usage |
| Render passes, GPU time | Per-pass GPU milliseconds, usually where a frame goes |
| Render passes, CPU time | Per-pass CPU milliseconds |
| Shader and pipeline counters | Raw counts in the millions. Off by default, because Tracy autoscales them and they crowd out the millisecond timings |
| Other diagnostics | Anything a crate or another plugin registers |

**Some groups will be greyed out**, with a line saying why. That is not a fault: those measurements are taken by something else, and if nothing is taking them there is nothing to plot. Entity count and CPU/memory come from the **Debugger** plugin, so enable that too if you want them. The render-pass timings only exist in a profiling build (below).

Set the switches **before** you connect. Turning one off stops new values at once, but a row already drawn on Tracy's timeline stays there showing a frozen line, because Tracy's protocol has no message that removes a plot. Reconnecting clears it.

## What you do not get

**No flame graph, and no per-system breakdown.** Tracy's Flame Graph and Statistics windows will be empty.

That is not a limitation of this plugin. Those need instrumentation compiled into the engine itself, and a normal build does not have it, so there is nothing for a plugin loaded at run time to switch on.

If you need to know *which system* is eating the frame, build the engine with profiling turned on:

```sh
cargo renzora profile
```

**Turn this plugin off in such a build.** That build marks frames itself, and two marks per frame halves every frame time Tracy reports.

## Capturing without the GUI

Useful for recording a run to look at later, or on a machine with no display:

```sh
tracy-capture -o out.tracy -s 10 -f      # connect and record 10 seconds
tracy-csvexport -u -p out.tracy          # the plot data, as CSV
```

`-u -p` is the part worth remembering. Plain `tracy-csvexport` reports zones, and there are none here, so without it you get an empty file that looks like nothing was recorded.

## Where the settings live

`%APPDATA%\renzora\tracy.json` on Windows, `~/.config/renzora/tracy.json` elsewhere. Plain JSON, one flag per group, safe to edit or delete. A missing or unreadable file reads as off.

**Scope:** Editor.
