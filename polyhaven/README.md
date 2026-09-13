# Poly Haven

Browse [Poly Haven](https://polyhaven.com)'s CC0 library inside the editor and
import an asset into the open project with one click.

Adds a **Poly Haven** panel (Add Panel → Assets). Three tabs — HDRIs, textures,
models — as a searchable thumbnail grid. Pick a resolution, click a tile, choose
where it goes, and the files land in a folder of their own under the destination
you picked.

Everything on Poly Haven is CC0, so there is nothing to attribute and no licence
to accept.

## Where things land

Clicking a tile opens a destination prompt built on ember's shared folder picker
— the same widget the marketplace install and the hierarchy's Create-asset
overlay use, **New Folder** button included. It opens on the kind's own folder
(`models/`, `textures/`, `hdris/`) when the project already has one and on the
project root when it does not — it creates nothing just by being opened. Each
asset then takes a subfolder of its own, because a model brings a `.bin` and a
`textures/` beside it and a texture brings five maps.

**Models go through the import pipeline.** The downloaded glTF is converted to a
`.glb` with its textures extracted into `textures/` and a `.material` written per
material, which is the layout the engine actually loads — the same treatment a
model dropped into the viewport or installed from the marketplace gets. The
source glTF is removed afterwards, so the asset browser shows one model rather
than two. Textures and HDRIs are images the engine already loads from disk and
are left exactly as downloaded.

## What gets downloaded

| Kind | Files |
|---|---|
| HDRI | one `.hdr` (`.exr` if a resolution publishes no `.hdr`) |
| Texture | `Diffuse`, `nor_gl`, `Rough`, `Metal`, `Displacement` as JPEG, whichever the asset publishes |
| Model | the glTF plus every buffer and texture it references, at their own relative paths |

Not every asset publishes every map or every size. A missing map is skipped; a
missing resolution falls back to the nearest published one, so asking for 8k on
an asset that stops at 4k gets you the 4k rather than an error.

`nor_gl` rather than `nor_dx` because that is the OpenGL/glTF green-channel
convention, which is what Bevy's `StandardMaterial` expects. Feeding it `nor_dx`
inverts every normal's Y and lights the surface from the wrong side.

## Notes

- **The catalogue is cached for a day.** There is no search endpoint and no
  pagination on the API — `/assets` returns the whole list or nothing — so it is
  fetched once and kept in the system temp directory. Reopening the panel is
  instant; a brand new upload shows up the next day.
- **The grid pages, 150 at a time.** It is not virtualised and the textures
  catalogue is about a thousand entries, so a page is what keeps the cost flat.
  Prev/next sit at the right of the status line with a `2 / 7` counter between
  them, and the whole pager hides itself when everything fits on one page. Paging
  resets when you search or change tab.
- **Thumbnails are throttled** to twelve concurrent downloads. Ember's
  `MarkdownImages` cache does the fetching and decoding, so they are shared with
  the rest of the editor and survive a panel close.
- **Progress is on the tile.** A bar fills across the bottom of the preview while
  a download runs, with a line under the tile saying how many files are done, and
  then `imported` or the reason it failed.
- **Downloads continue while the tab is hidden**, and the tiles are still showing
  where they got to when you come back.

## Why this is a native plugin

The C-ABI HTTP domain hands a plugin its response as a `String`, built with
`String::from_utf8_lossy`. Every file this plugin fetches is binary, so that
boundary corrupts all of them. `renzora::net::Request` returns `Vec<u8>`, and a
native plugin links the contract crate that defines it.

The second reason is that an asset browser is editor furniture: it never ships
inside a game, so nothing a C-ABI plugin buys — running in an export, on wasm, on
a console — would ever be used here.

## Building

Do **not** run `cargo build` in this directory. `plugins/` is outside the engine
workspace, so cargo would resolve a second Bevy from crates.io and the plugin
that came out would have different `TypeId`s from the engine: it would load, run,
and corrupt the World. The editor builds this against the staged SDK on startup
whenever the source is newer than `build/stamp.txt`.
