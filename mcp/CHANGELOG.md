# Changelog

## 1.6.0

- feat: `project_load` reports how long the last project load took, stage by stage, and with `reload` reopens the project and times a fresh load. With `wait` it times the next load however it starts, such as a project opened from the launcher.
- feat: `editor_state` says whether the editor is on the launcher, the loading screen or the editor.
- feat: `memory_report` says how much memory the editor holds and which kinds of asset account for it.
- fix: a submenu opened with `move_mouse` stays open when the pointer moves into it, so its items can be clicked.
- fix: the person's own mouse is no longer held where the MCP cursor last clicked.
- fix: a click still lands on its target when the person moves their mouse while it runs.
- fix: a double click opens files and folders in the Assets panel instead of only selecting them.

These need the r1-alpha8 nightly that added `renzora::load_timeline`.

## 1.5.0

- feat: `terrain_layers` lists the paintable layers of a terrain's landscape material.
- feat: `paint_terrain` paints, erases, smooths or fills a landscape layer along a path, kept to slope and height ranges.
- feat: `set_terrain_material` gives a terrain a landscape material (Alpine Landscape by default).
- feat: `build_terrain_graph` builds a terrain generator graph into a terrain.

These need the r1-alpha8 nightly that added landscape materials and terrain graphs.

## 1.4.0

- feat: `terrain_height` gives the ground height and slope at world points.
- feat: `place_on_ground` drops entities onto the terrain, optionally tilted to its slope.
- feat: `sculpt_terrain` drags any terrain brush along a path of world points, in metres.
- feat: `generate_terrain` and `erode_terrain` run the terrain generator and hydraulic and thermal erosion.
- feat: `scatter` places copies of an entity across an area by slope and height rules, as one undo step.
- fix: `set_component_field` accepts a list index written with a dot (`layers.1.tint`) as well as brackets.
- fix: `sculpt_terrain` with `passes` reports what the passes did, not only whether the last one changed anything.
- fix: `list_assets` no longer offers the `.rvg` files the engine bakes beside a `.glb` or for a terrain, which are not models and spawned nothing.

These need the r1-alpha8 nightly that added `renzora::terrain_tools`.

## 1.3.0

- feat: `profiler_record`, `profiler_status`, `profiler_summary` and `profiler_spikes` record the editor or a running game in the companion and report where the time goes (needs the r1-alpha8 nightly with the companion profiler).
- fix: `list_assets` lists `.rvg` models, the format imports are stored in.

## 1.2.0

- feat: `spawn_model` places `.rvg` models, the format the importer now writes.
- feat: `import_model` opens the editor's import window loaded with files from outside the project, instead of converting them silently, so animation-only FBX clips import and the user sees the preview.
- feat: `import_model` takes `paths` to import several files in one window.
- feat: `game_screenshot` and `game_input` see and play the game Play is running, over the engine's live link (needs the r1-alpha8 nightly that added `renzora::GameLink`).

## 1.1.0

- feat: `key_pose` keys bones' current pose at the timeline's playhead, as K over the viewport does. Keying into bone tracks needs the r1-alpha8 nightly from 2026-10-23 or later; an older engine keys a property track on the bone instead.

## 1.0.1

- Builds on Bevy 0.20 (needs the r1-alpha8 nightly from 2026-10-19 or later).
- feat: a component's enum field can be set by its variant name.

## 1.0.0

- Initial release.
