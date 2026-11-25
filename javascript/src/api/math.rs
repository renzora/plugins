//! `lerp` and `clamp`, the Lua plugin's two math helpers.
//!
//! Everything else a script needs is JavaScript's own `Math` (`Math.sin`,
//! `Math.random`, `Math.hypot`, ...), which Boa implements in full, so unlike
//! the Python plugin this one carries no math or random module of its own.
//! `vec2` / `vec3` / `color` are classes, defined in the prelude.

use boa_engine::{Context, JsObject, JsValue};

use super::add;
use super::args::Args;

pub fn register(ctx: &mut Context, m: &JsObject) {
    add(ctx, m, "lerp", |_, args, _| {
        let a = Args::new(args, "lerp");
        let (x, y, t) = (a.f64(0, "a")?, a.f64(1, "b")?, a.f64(2, "t")?);
        Ok(JsValue::from(x + (y - x) * t))
    });
    add(ctx, m, "clamp", |_, args, _| {
        let a = Args::new(args, "clamp");
        let (v, lo, hi) = (a.f64(0, "value")?, a.f64(1, "min")?, a.f64(2, "max")?);
        Ok(JsValue::from(v.max(lo).min(hi)))
    });
}
