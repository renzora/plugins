//! The `g` canvas `on_draw(g)` receives.
//!
//! `g.width` / `g.height` are the draw surface in pixels; every method records
//! one [`DrawCmd`] into the draw buffer. The methods are the Lua plugin's,
//! with the same argument order: `g.line(x1, y1, x2, y2, color, thickness=2)`,
//! `g.arc(cx, cy, r, start, end, color, thickness=2)`, `g.circle(cx, cy, r,
//! color)`, `g.rect(x, y, w, h, color)`, `g.text(x, y, text, size, color)`,
//! `g.triangle(x1, y1, x2, y2, x3, y3, color)`, `g.poly(points, color)`.
//!
//! A colour is a `"#RRGGBB"` / `"#RRGGBBAA"` string as in Lua, or a `color`
//! (any 3 or 4 number tuple) with channels in 0..1.
//!
//! The canvas class itself is in the prelude; these are the natives its
//! methods are bound to.

use renzora::core::script_bridge::DrawCmd;
use rustpython_vm::builtins::PyDict;
use rustpython_vm::function::FuncArgs;
use rustpython_vm::{Py, PyResult, VirtualMachine};

use super::add;
use super::args::Args;
use crate::buffers::push_draw;
use crate::convert::{self, Shape};

/// Parse a `#RRGGBB` / `#RRGGBBAA` hex string to `[r, g, b, a]` in 0..1.
/// Anything unparseable falls back to opaque white, so a typo is visible
/// rather than invisible.
fn parse_hex(s: &str) -> [f32; 4] {
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
    let Some(v) = a.get(i) else {
        return [1.0, 1.0, 1.0, 1.0];
    };
    if let Some(s) = convert::string(v) {
        return parse_hex(&s);
    }
    match convert::shape(v, a.vm()) {
        Some(Shape::Color(c)) => c,
        Some(Shape::Vec3([r, g, b])) => [r, g, b, 1.0],
        _ => [1.0, 1.0, 1.0, 1.0],
    }
}

/// `on_draw`'s `g`, as an instance of the prelude's canvas class.
pub fn canvas(vm: &VirtualMachine, width: f32, height: f32) -> PyResult {
    let class = vm.import("renzora", 0)?.get_attr("_Canvas", vm)?;
    class.call(
        (convert::float(vm, width as f64), convert::float(vm, height as f64)),
        vm,
    )
}

pub fn register(vm: &VirtualMachine, m: &Py<PyDict>) {
    add(vm, m, "_draw_line", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<()> {
        let a = Args::new(&args, vm, "line");
        push_draw(DrawCmd::Line {
            x1: a.f32(0, "x1")?,
            y1: a.f32(1, "y1")?,
            x2: a.f32(2, "x2")?,
            y2: a.f32(3, "y2")?,
            color: color(&a, 4),
            thickness: a.opt_f32(5, "thickness")?.unwrap_or(2.0),
        });
        Ok(())
    });
    add(vm, m, "_draw_arc", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<()> {
        let a = Args::new(&args, vm, "arc");
        push_draw(DrawCmd::Arc {
            cx: a.f32(0, "cx")?,
            cy: a.f32(1, "cy")?,
            r: a.f32(2, "r")?,
            start: a.f32(3, "start")?,
            end: a.f32(4, "end")?,
            color: color(&a, 5),
            thickness: a.opt_f32(6, "thickness")?.unwrap_or(2.0),
        });
        Ok(())
    });
    add(vm, m, "_draw_circle", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<()> {
        let a = Args::new(&args, vm, "circle");
        push_draw(DrawCmd::Circle {
            cx: a.f32(0, "cx")?,
            cy: a.f32(1, "cy")?,
            r: a.f32(2, "r")?,
            color: color(&a, 3),
        });
        Ok(())
    });
    add(vm, m, "_draw_rect", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<()> {
        let a = Args::new(&args, vm, "rect");
        push_draw(DrawCmd::Rect {
            x: a.f32(0, "x")?,
            y: a.f32(1, "y")?,
            w: a.f32(2, "w")?,
            h: a.f32(3, "h")?,
            color: color(&a, 4),
        });
        Ok(())
    });
    add(vm, m, "_draw_text", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<()> {
        let a = Args::new(&args, vm, "text");
        push_draw(DrawCmd::Text {
            x: a.f32(0, "x")?,
            y: a.f32(1, "y")?,
            text: a.display(2),
            size: a.f32(3, "size")?,
            color: color(&a, 4),
        });
        Ok(())
    });
    add(vm, m, "_draw_triangle", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<()> {
        let a = Args::new(&args, vm, "triangle");
        push_draw(DrawCmd::Triangle {
            x1: a.f32(0, "x1")?,
            y1: a.f32(1, "y1")?,
            x2: a.f32(2, "x2")?,
            y2: a.f32(3, "y2")?,
            x3: a.f32(4, "x3")?,
            y3: a.f32(5, "y3")?,
            color: color(&a, 6),
        });
        Ok(())
    });
    // poly(points, color): a convex polygon, fanned from its first vertex into
    // filled triangles. `points` is flat, `[x1, y1, x2, y2, ...]` as in Lua, or
    // a list of pairs, `[(x1, y1), (x2, y2), ...]`, which is how Python code
    // usually holds points.
    add(vm, m, "_draw_poly", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<()> {
        let a = Args::new(&args, vm, "poly");
        let Some(list) = a.get(0).and_then(|v| convert::items(v)) else {
            return Err(vm.new_type_error("poly(): points must be a list".to_owned()));
        };
        let mut pts: Vec<(f32, f32)> = Vec::with_capacity(list.len());
        let mut flat: Vec<f32> = Vec::new();
        for item in &list {
            if let Some(n) = convert::number(item, vm) {
                flat.push(n as f32);
            } else if let Some(Shape::Vec2([x, y])) = convert::shape(item, vm) {
                pts.push((x, y));
            }
        }
        pts.extend(flat.chunks_exact(2).map(|c| (c[0], c[1])));
        let col = color(&a, 1);
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
        Ok(())
    });
}
