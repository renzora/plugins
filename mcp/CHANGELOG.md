# Changelog

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
