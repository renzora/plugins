# AI Mesh Generator

Describe a model, or give it a reference image, and get a textured GLB
imported into the open project without leaving the editor.

Adds an **AI Mesh** panel (Add Panel > Assets) and a **Settings > Plugins >
AI Mesh** section for your API keys.

## Services

| Service | Text | Image | Key |
|---|:---:|:---:|---|
| [Meshy](https://www.meshy.ai) | yes | yes | Meshy API key |
| [Tripo](https://platform.tripo3d.ai) | yes | yes | Tripo API key |
| Hunyuan3D v3 on [fal.ai](https://fal.ai) | yes | yes | fal.ai key |
| TRELLIS 2 on [fal.ai](https://fal.ai) | no | yes | the same fal.ai key |
| Hunyuan3D 2.1, local server | no | yes | none |

**Bring your own account.** There is no Renzora service in the middle: you
paste your own key, requests go straight from your editor to that service, and
each generation spends credits on your account. Each service's own terms decide
whether its output may be used commercially. Some free tiers do not allow it,
so check before you ship a generated asset.

Keys are stored in your user config folder (`%APPDATA%\renzora\ai_mesh.json`,
or `~/.config/renzora/ai_mesh.json`), never in the project, so they are never
committed and never shipped inside a game. The plugin is editor-only and does
not load in an exported game at all.

## Using it

1. Pick a service and **Text** or **Image**.
2. Type a prompt, or choose a PNG, JPEG or WebP (20 MB at most).
3. Leave **PBR materials** on for metallic, roughness and normal maps, and
   **Low poly** on for a game-budget mesh (about 10,000 triangles; Hunyuan3D's
   floor is 40,000). Off, you get each service's full-detail default, which is
   often hundreds of thousands of triangles.
4. Press **Generate**. Progress shows on the job's row; a generation usually
   takes one to three minutes, and keeps going while you work in other tabs.

The finished model lands in `models/generated/<name>/` (or `generated/<name>/`
when the project has no `models/` folder) and is run through the engine's
import pipeline, so its textures are extracted and a `.material` is written per
material, exactly as for a model dragged into the viewport. A name already in
use gets a numeric suffix rather than overwriting a model the scene may use.

## Cancelling

The **x** on a running job stops it within a fraction of a second, and nothing
is written to the project or imported. What happens at the service's end
depends on what it offers:

| Service | On cancel |
|---|---|
| Meshy | The task is deleted. A task still queued is refunded; one already generating runs to completion at Meshy and may be charged. |
| fal.ai | The request is cancelled in fal's queue. |
| Tripo | Tripo has no cancel endpoint, so the task finishes on its side and may be charged. |
| Local server | `api_server.py` has no cancel route; the GPU finishes and the result is discarded. |

A Meshy text job is two paid tasks; cancelling during the first means the
second is never started.

## Running Hunyuan3D locally

Clone [Hunyuan3D-2.1](https://github.com/Tencent-Hunyuan/Hunyuan3D-2.1),
install it per its instructions, and start `python api_server.py`. It listens
on `http://127.0.0.1:8081` by default, which is what the plugin expects; change
**Local server** in the settings if you run it elsewhere. Texturing needs the
texture pipeline installed on the server; without it you get untextured
geometry.

## Notes

- **Meshy text** runs as two tasks: a preview that builds the shape, then a
  refine that textures it. The bar's first half is the shape.
- **fal.ai** reports no percentage, only queue position and "generating", so its
  bar sits at a third until the model arrives.
- **Tripo** model links expire five minutes after a task finishes, which is why
  the download starts the moment it does.
- A job gives up after 15 minutes. This matters most for the local server,
  which keeps answering "processing" forever if its worker crashes.

## Building

Do **not** run `cargo build` in this directory. `plugins/` is outside the engine
workspace, so cargo would resolve a second Bevy from crates.io and the plugin
that came out would load and corrupt the World. The editor builds it against
the staged SDK. The first build fetches `rfd` (the file dialog) with cargo, so
it needs a network once.
