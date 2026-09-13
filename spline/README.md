# Spline

Control-point paths with Catmull-Rom evaluation.

Almost nothing is in this plugin, and that is the point. `SplinePath` and its curve maths live in the **contract crate** (`renzora::spline`); all that remains here is registering the type for reflection.

A spline is less a feature than a shape other things read: a road builder, a camera rail, a fence generator, a patrol path, a particle track. Each of those is a plausible separate plugin, and they can only cooperate if they agree on one `SplinePath`, with one `TypeId`, one scene representation, and one answer for where `t = 1.7` falls on the curve.

**Scope:** Runtime (editor viewport and shipped game).
