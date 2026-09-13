# Mesh Draw

A level-prototyping tool: block out geometry by drawing it in the viewport instead of placing and scaling primitives.

Two draw tools and one mesh operation, in the **Edit** mode toolbar:

- **Draw Box**: click-drag a rectangle on the ground plane, release to lock it, move the cursor to extrude height, click to commit.
- **Draw Polyline**: click to drop footprint points, click the first point again (or press Enter) to close the polygon, then extrude and click to commit.
- **Join Selected**: merges the current selection of two or more meshes into one mesh entity. The sources are despawned.

Activating a draw tool clears the active transform tool, so viewport picking and gizmos stay out of the way while you draw.

**Scope:** Editor.
