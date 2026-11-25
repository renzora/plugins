# 3MF Import

Adds **3MF** (3D Manufacturing Format) to the importer: the format PrusaSlicer,
Cura, Bambu Studio, OrcaSlicer and Windows 3D Builder save, and that Blender
reads through its official "3MF format" extension. Drop a `.3mf` on the editor,
or pick it in **Import > Files**, and it imports like any other model: preview,
an editable `.material` per material, and the stored `.rvg`.

## What it reads

| 3MF | In the editor |
|---|---|
| Objects with a mesh | one mesh per build item, with a primitive per material |
| Components, nested | merged into their build item, each placed by its transform |
| Build items | one model node each, at its build-plate position |
| Mirrored transforms | geometry flipped with its faces kept outward |
| `unit` (micron, millimeter, centimeter, inch, foot, meter) | metres |
| `<basematerials>` | a material per base, named and coloured as the file says |
| `<m:colorgroup>` | a material per colour (`Color 2A9D8F`) |
| Per-triangle and per-object properties | the triangle's material |
| The production extension (`p:path`) | objects in other model parts of the package, as slicers save them |

3MF is Z up; models arrive Y up. 3MF stores no normals: they are made on
import, smooth across edges flatter than 40 degrees and sharp across steeper
ones. The Import window's **Scale** applies on top of the file's units.

## Not read

- Textures (`<m:texture2dgroup>`), and the material extension's PBR display
  properties: materials come in as flat colours.
- Support objects (`type="support"` or `"other"`): printing aids, not the
  model.
- Slicer settings, thumbnails and G-code stored in the package.
- Per-vertex colour blends within a triangle: a triangle takes the colour of
  its first corner.

## How it plugs in

The whole integration is one call, `renzora::register_import_format`, in
`src/lib.rs`. The converter turns the package into a GLB; the importer does
everything after that. See *Teaching the importer a file format* in the
engine's native plugin docs.
