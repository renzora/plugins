//! The canvas `OnDraw(Canvas g)` draws on.
//!
//! `g.Width` / `g.Height` are the draw surface in pixels; every method records
//! one [`DrawCmd`] into the draw buffer, which `call_on_draw` returns. The
//! methods are the Lua plugin's, with the same argument order. The C# `Canvas`
//! calls these by their underscored names, which keeps them out of the way of
//! the script-facing ones.
//!
//! A colour is a `"#RRGGBB"` / `"#RRGGBBAA"` string as in Lua, or a `Color`
//! with channels in 0..1.

use renzora::core::script_bridge::DrawCmd;
use renzora::serde_json::Value;

use super::{Args, Out};
use crate::buffers::push_draw;
use crate::convert::{self, Shape};

/// Parse a `#RRGGBB` / `#RRGGBBAA` hex string to `[r, g, b, a]` in 0..1.
/// Anything unparseable falls back to opaque white, so a typo is visible
/// rather than invisible.
pub fn parse_hex(s: &str) -> [f32; 4] {
    let s = s.trim().trim_start_matches('#');
    let ch = |i: usize| {
        s.get(i..i + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .map(|v| v as f32 / 255.0)
            .unwrap_or(1.0)
    };
    match s.len() {
        6 => [ch(0), ch(2), ch(4), 1.0],
        8 => [ch(0), ch(2), ch(4), ch(6)],
        _ => [1.0, 1.0, 1.0, 1.0],
    }
}

fn color(a: &Args, i: usize) -> [f32; 4] {
    a.opt_color(i).unwrap_or([1.0, 1.0, 1.0, 1.0])
}

fn draw(cmd: DrawCmd) -> Out {
    push_draw(cmd);
    Ok(Value::Null)
}

pub fn call(name: &str, a: &Args) -> Option<Out> {
    Some(match name {
        "_draw_line" => (|| {
            draw(DrawCmd::Line {
                x1: a.f32(0, "x1")?,
                y1: a.f32(1, "y1")?,
                x2: a.f32(2, "x2")?,
                y2: a.f32(3, "y2")?,
                color: color(a, 4),
                thickness: a.opt_f32(5, "thickness")?.unwrap_or(2.0),
            })
        })(),
        "_draw_arc" => (|| {
            draw(DrawCmd::Arc {
                cx: a.f32(0, "cx")?,
                cy: a.f32(1, "cy")?,
                r: a.f32(2, "r")?,
                start: a.f32(3, "start")?,
                end: a.f32(4, "end")?,
                color: color(a, 5),
                thickness: a.opt_f32(6, "thickness")?.unwrap_or(2.0),
            })
        })(),
        "_draw_circle" => (|| {
            draw(DrawCmd::Circle {
                cx: a.f32(0, "cx")?,
                cy: a.f32(1, "cy")?,
                r: a.f32(2, "r")?,
                color: color(a, 3),
            })
        })(),
        "_draw_rect" => (|| {
            draw(DrawCmd::Rect {
                x: a.f32(0, "x")?,
                y: a.f32(1, "y")?,
                w: a.f32(2, "w")?,
                h: a.f32(3, "h")?,
                color: color(a, 4),
            })
        })(),
        "_draw_text" => (|| {
            draw(DrawCmd::Text {
                x: a.f32(0, "x")?,
                y: a.f32(1, "y")?,
                text: a.display(2),
                size: a.f32(3, "size")?,
                color: color(a, 4),
            })
        })(),
        "_draw_triangle" => (|| {
            draw(DrawCmd::Triangle {
                x1: a.f32(0, "x1")?,
                y1: a.f32(1, "y1")?,
                x2: a.f32(2, "x2")?,
                y2: a.f32(3, "y2")?,
                x3: a.f32(4, "x3")?,
                y3: a.f32(5, "y3")?,
                color: color(a, 6),
            })
        })(),
        // poly(points, color): a convex polygon, fanned from its first vertex
        // into filled triangles. `points` is a list of `Vector2`, or flat
        // numbers `[x1, y1, x2, y2, ...]` as in Lua.
        "_draw_poly" => (|| {
            let Some(Value::Array(list)) = a.get(0) else {
                return Err("poly(): points must be a list".to_string());
            };
            let mut pts: Vec<(f32, f32)> = Vec::with_capacity(list.len());
            let mut flat: Vec<f32> = Vec::new();
            for item in list {
                if let Some(n) = convert::number(item) {
                    flat.push(n as f32);
                } else if let Some(Shape::Vec2([x, y]) | Shape::Vec3([x, y, _])) = convert::shape(item) {
                    pts.push((x, y));
                }
            }
            pts.extend(flat.chunks_exact(2).map(|c| (c[0], c[1])));
            let col = color(a, 1);
            if pts.len() >= 3 {
                let (x0, y0) = pts[0];
                for w in pts[1..].windows(2) {
                    push_draw(DrawCmd::Triangle {
                        x1: x0,
                        y1: y0,
                        x2: w[0].0,
                        y2: w[0].1,
                        x3: w[1].0,
                        y3: w[1].1,
                        color: col,
                    });
                }
            }
            Ok(Value::Null)
        })(),
        _ => return None,
    })
}
