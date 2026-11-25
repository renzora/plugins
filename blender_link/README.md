# Blender Live Link

Work in Blender and the Renzora editor follows. Objects, their meshes with
every modifier applied, materials and lights update in the editor as you
change them, and they are saved into the project as ordinary models. The
scene therefore still works with Blender closed, in Play, and in an exported
game.

## Setup

1. Install this plugin in the editor. **Installing it also installs and
   enables its Blender add-on**: on every editor start, for each Blender 4.2
   or newer that has been run on this machine, it writes
   `addon/renzora_live_link.py` (the add-on's full source, in this folder) to
   that version's user `scripts/addons` folder and enables it in Blender's
   preferences. The Console says what it did.
   - With Blender closed, a headless Blender enables the add-on and saves the
     preferences, so it is on the next time Blender starts.
   - With Blender open, the preferences are left alone (Blender would write
     its own back on quit). Tick **Renzora Live Link** in **Edit >
     Preferences > Add-ons** instead (press Refresh if it is not listed). It
     loads at once, with no restart.
   - When a plugin update brings a newer add-on, a running Blender notices
     the new file and reloads it by itself, reconnecting if it was linked.
2. In Blender's 3D Viewport press **N**, open the **Renzora** tab and press
   **Connect**.

To remove the add-on, uninstall this plugin, then disable and remove
**Renzora Live Link** in Blender's **Edit > Preferences > Add-ons**.

The editor's status bar shows `Blender <file> (<objects>)` while linked.

## What is linked

| Blender | In the editor |
|---|---|
| Mesh, curve, surface, text and metaball objects | the evaluated mesh (modifiers, geometry nodes, armature pose) with one part per material |
| Principled BSDF | base colour, metallic, roughness, alpha, emission, base colour texture, normal map (PNG or JPEG) |
| Point, spot and sun lights | `PointLight`, `SpotLight`, `DirectionalLight`; an area light becomes a point light |
| Empties, cameras and anything else | an entity with the object's transform |
| Renames and deletions | the entity is renamed or removed |

Every object of a `.blend` sits under one entity named `Blender: <file>`. The
hierarchy there is flat: Blender's parenting is already part of each object's
transform.

Transforms are checked 30 times a second. A mesh is sent when Blender
reports its geometry changed, at most 10 times a second per object, so
dragging a vertex in Edit Mode streams.

## Saving

Each mesh object is written to `blender/<file>/<object>.glb` in the project,
400 ms after its geometry or materials stop changing (and at once when
Blender disconnects). The entity's `MeshInstanceData` points at that file, so
saving the scene saves the object like any imported model. Lights save as
their components.

Opening the scene later loads the `.glb` files. Connecting Blender again
finds the objects by name and takes them over, so nothing is duplicated.

## Limits

- **One way.** Blender is the source of truth: moving a linked object in the
  editor lasts only until the next change to it in Blender.
- **Units.** Light power is converted as Blender's own glTF exporter does
  (watts × 683 / 4π to lumens, a sun's W/m² × 683 to lux). The result can
  read brighter or darker than Cycles or Eevee.
- **Textures** reach the editor only as PNG or JPEG files on disk or packed
  in the `.blend`. A texture painted in Blender and not yet saved is skipped
  (the Blender console says which). Other texture inputs (roughness, metallic
  maps) are not linked yet.
- **Not linked:** collection instances, particle systems, shape key
  animation, and cameras' lens settings.
- A deleted object's `.glb` is left in the project, because a saved scene may
  still point at it. Delete the files in `blender/` yourself once nothing uses
  them.
- The link listens on loopback only. Set `RENZORA_BLENDER_PORT` before
  starting the editor to use another port, and set the same port in the
  add-on.
