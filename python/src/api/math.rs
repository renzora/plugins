//! `lerp`, `clamp`, the `math` module, and the generator behind `random`.
//!
//! `math` and `random` are written here rather than taken from RustPython.
//! Its `math` lives in `rustpython-stdlib`, the native half of its standard
//! library, and that crate builds a bundled SQLite from C on every desktop
//! target, unconditionally: depending on it would make a C compiler a
//! requirement for installing this plugin, which is otherwise pure Rust. (In
//! the 0.6 line it does not build from crates.io at all: its `pymath` resolves
//! a newer big-integer crate than the VM uses.) The pure-Python `random.py`
//! needs that `math` and a native `_random` besides, so it goes too. What a
//! game script uses of either is small and stable, and is reproduced below; see
//! the README for the list.
//!
//! `vec2` / `vec3` / `color` (the Lua plugin's `vec2` / `vec3` constructors)
//! are classes, defined in the prelude.

use std::cell::Cell;

use rustpython_vm::builtins::{PyDict, PyModule};
use rustpython_vm::function::FuncArgs;
use rustpython_vm::{AsObject, Py, PyObjectRef, PyRef, PyResult, VirtualMachine};

use super::add;
use super::args::Args;
use crate::convert;

pub fn register(vm: &VirtualMachine, m: &Py<PyDict>) {
    add(vm, m, "lerp", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<f64> {
        let a = Args::new(&args, vm, "lerp");
        let (x, y, t) = (a.f32(0, "a")? as f64, a.f32(1, "b")? as f64, a.f32(2, "t")? as f64);
        Ok(x + (y - x) * t)
    });
    add(vm, m, "clamp", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<f64> {
        let a = Args::new(&args, vm, "clamp");
        let (v, lo, hi) = (a.f32(0, "value")? as f64, a.f32(1, "min")? as f64, a.f32(2, "max")? as f64);
        Ok(v.max(lo).min(hi))
    });
    add(vm, m, "_rand", |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult<f64> {
        Ok(next_f64())
    });
    add(vm, m, "_seed", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<()> {
        let seed = match args.args.first().filter(|v| !vm.is_none(v)) {
            None => time_seed(),
            Some(v) => match (convert::integer(v, vm), convert::number(v, vm)) {
                (Some(n), _) if convert::string(v).is_none() => n as u64,
                (_, Some(f)) => f.to_bits(),
                // A string (or anything else) seeds from its text, so
                // `seed("level-3")` is the same sequence on every run.
                _ => fnv1a(convert::display(v, vm).as_bytes()),
            },
        };
        RNG.with(|r| r.set(splitmix(seed)));
        Ok(())
    });
}

// =============================================================================
// The generator
// =============================================================================

thread_local! {
    /// xorshift64* state. Never zero: xorshift stays at zero once there.
    static RNG: Cell<u64> = Cell::new(splitmix(time_seed()));
}

fn time_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
}

/// Spread a seed over all 64 bits, so seeds 1 and 2 start far apart, and keep
/// it off zero.
fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    if z == 0 {
        0x2545_F491_4F6C_DD1D
    } else {
        z
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

/// A float in `[0, 1)` from the top 53 bits, which is every value a double can
/// space evenly in that range.
fn next_f64() -> f64 {
    RNG.with(|r| {
        let mut x = r.get();
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        r.set(x);
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    })
}

// =============================================================================
// The `math` module
// =============================================================================

fn num(a: &Args, i: usize, name: &str) -> PyResult<f64> {
    a.f64_arg(i, name)
}

fn domain(vm: &VirtualMachine) -> rustpython_vm::builtins::PyBaseExceptionRef {
    vm.new_value_error("math domain error".to_owned())
}

/// A float that Python code expects as an `int` (`floor`, `ceil`, `trunc`).
fn to_int(vm: &VirtualMachine, f: f64) -> PyResult {
    if f.is_nan() {
        return Err(vm.new_value_error("cannot convert float NaN to integer".to_owned()));
    }
    if f.is_infinite() {
        return Err(vm.new_overflow_error("cannot convert float infinity to integer".to_owned()));
    }
    if f.abs() < 9.0e15 {
        return Ok(convert::int(vm, f as i64));
    }
    let big = rustpython_vm::builtins::try_f64_to_bigint(f, vm)?;
    Ok(vm.ctx.new_int(big).into())
}

/// A float result. A plain `fn` because the functions below must capture
/// nothing (see `api` for why).
fn f(vm: &VirtualMachine, v: f64) -> PyResult {
    Ok(convert::float(vm, v))
}

/// One-argument float functions. A macro so each line reads as the function
/// it registers.
macro_rules! unary {
    ($vm:expr, $m:expr, $name:literal, |$x:ident, $v:ident| $body:expr) => {
        add($vm, $m, $name, |args: FuncArgs, $v: &VirtualMachine| -> PyResult {
            let $x = num(&Args::new(&args, $v, $name), 0, "x")?;
            $body
        });
    };
}

/// Build `math` and put it in `sys.modules`, so `import math` finds it.
pub fn register_math_module(vm: &VirtualMachine) -> PyResult<PyRef<PyModule>> {
    let dict = vm.ctx.new_dict();
    let m: &Py<PyDict> = &dict;

    m.set_item("pi", convert::float(vm, std::f64::consts::PI), vm)?;
    m.set_item("e", convert::float(vm, std::f64::consts::E), vm)?;
    m.set_item("tau", convert::float(vm, std::f64::consts::TAU), vm)?;
    m.set_item("inf", convert::float(vm, f64::INFINITY), vm)?;
    m.set_item("nan", convert::float(vm, f64::NAN), vm)?;

    unary!(vm, m, "sqrt", |x, vm| if x < 0.0 { Err(domain(vm)) } else { f(vm, x.sqrt()) });
    unary!(vm, m, "cbrt", |x, vm| f(vm, x.cbrt()));
    unary!(vm, m, "exp", |x, vm| f(vm, x.exp()));
    unary!(vm, m, "exp2", |x, vm| f(vm, x.exp2()));
    unary!(vm, m, "expm1", |x, vm| f(vm, x.exp_m1()));
    unary!(vm, m, "log10", |x, vm| if x <= 0.0 { Err(domain(vm)) } else { f(vm, x.log10()) });
    unary!(vm, m, "log2", |x, vm| if x <= 0.0 { Err(domain(vm)) } else { f(vm, x.log2()) });
    unary!(vm, m, "log1p", |x, vm| if x <= -1.0 { Err(domain(vm)) } else { f(vm, x.ln_1p()) });
    unary!(vm, m, "sin", |x, vm| f(vm, x.sin()));
    unary!(vm, m, "cos", |x, vm| f(vm, x.cos()));
    unary!(vm, m, "tan", |x, vm| f(vm, x.tan()));
    unary!(vm, m, "asin", |x, vm| if x.abs() > 1.0 { Err(domain(vm)) } else { f(vm, x.asin()) });
    unary!(vm, m, "acos", |x, vm| if x.abs() > 1.0 { Err(domain(vm)) } else { f(vm, x.acos()) });
    unary!(vm, m, "atan", |x, vm| f(vm, x.atan()));
    unary!(vm, m, "sinh", |x, vm| f(vm, x.sinh()));
    unary!(vm, m, "cosh", |x, vm| f(vm, x.cosh()));
    unary!(vm, m, "tanh", |x, vm| f(vm, x.tanh()));
    unary!(vm, m, "asinh", |x, vm| f(vm, x.asinh()));
    unary!(vm, m, "acosh", |x, vm| if x < 1.0 { Err(domain(vm)) } else { f(vm, x.acosh()) });
    unary!(vm, m, "atanh", |x, vm| if x.abs() >= 1.0 { Err(domain(vm)) } else { f(vm, x.atanh()) });
    unary!(vm, m, "fabs", |x, vm| f(vm, x.abs()));
    unary!(vm, m, "degrees", |x, vm| f(vm, x.to_degrees()));
    unary!(vm, m, "radians", |x, vm| f(vm, x.to_radians()));
    unary!(vm, m, "floor", |x, vm| to_int(vm, x.floor()));
    unary!(vm, m, "ceil", |x, vm| to_int(vm, x.ceil()));
    unary!(vm, m, "trunc", |x, vm| to_int(vm, x.trunc()));
    unary!(vm, m, "isnan", |x, vm| Ok(convert::boolean(vm, x.is_nan())));
    unary!(vm, m, "isinf", |x, vm| Ok(convert::boolean(vm, x.is_infinite())));
    unary!(vm, m, "isfinite", |x, vm| Ok(convert::boolean(vm, x.is_finite())));

    // log(x, base=e)
    add(vm, m, "log", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let a = Args::new(&args, vm, "log");
        let x = num(&a, 0, "x")?;
        if x <= 0.0 {
            return Err(domain(vm));
        }
        Ok(convert::float(
            vm,
            match a.get(1) {
                None => x.ln(),
                Some(_) => {
                    let base = num(&a, 1, "base")?;
                    if base <= 0.0 || base == 1.0 {
                        return Err(domain(vm));
                    }
                    x.ln() / base.ln()
                }
            },
        ))
    });
    add(vm, m, "atan2", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<f64> {
        let a = Args::new(&args, vm, "atan2");
        Ok(num(&a, 0, "y")?.atan2(num(&a, 1, "x")?))
    });
    add(vm, m, "pow", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<f64> {
        let a = Args::new(&args, vm, "pow");
        Ok(num(&a, 0, "x")?.powf(num(&a, 1, "y")?))
    });
    add(vm, m, "fmod", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let a = Args::new(&args, vm, "fmod");
        let (x, y) = (num(&a, 0, "x")?, num(&a, 1, "y")?);
        if y == 0.0 {
            return Err(domain(vm));
        }
        Ok(convert::float(vm, x % y))
    });
    add(vm, m, "copysign", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<f64> {
        let a = Args::new(&args, vm, "copysign");
        Ok(num(&a, 0, "x")?.copysign(num(&a, 1, "y")?))
    });
    // hypot(*coordinates), as in Python 3.8+.
    add(vm, m, "hypot", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<f64> {
        let a = Args::new(&args, vm, "hypot");
        let mut sum = 0.0;
        for i in 0..a.len() {
            let v = num(&a, i, "x")?;
            sum += v * v;
        }
        Ok(sum.sqrt())
    });
    // dist(p, q): the distance between two points of equal length.
    add(vm, m, "dist", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<f64> {
        let point = |i: usize| -> PyResult<Vec<f64>> {
            let Some(obj) = args.args.get(i) else {
                return Err(vm.new_type_error("dist() takes two points".to_owned()));
            };
            convert::items(obj)
                .ok_or_else(|| vm.new_type_error("dist(): points must be tuples or lists".to_owned()))?
                .iter()
                .map(|v| {
                    convert::number(v, vm)
                        .ok_or_else(|| vm.new_type_error("dist(): coordinates must be numbers".to_owned()))
                })
                .collect()
        };
        let (p, q) = (point(0)?, point(1)?);
        if p.len() != q.len() {
            return Err(vm.new_value_error(
                "both points must have the same number of dimensions".to_owned(),
            ));
        }
        Ok(p.iter().zip(&q).map(|(a, b)| (a - b) * (a - b)).sum::<f64>().sqrt())
    });
    // isclose(a, b, rel_tol=1e-09, abs_tol=0.0)
    add(vm, m, "isclose", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let a = Args::new(&args, vm, "isclose");
        let (x, y) = (num(&a, 0, "a")?, num(&a, 1, "b")?);
        let kw = |k: &str, d: f64| {
            args.kwargs
                .get(k)
                .and_then(|v| convert::number(v, vm))
                .unwrap_or(d)
        };
        let (rel, abs) = (kw("rel_tol", 1e-9), kw("abs_tol", 0.0));
        if x == y {
            return Ok(true);
        }
        if x.is_infinite() || y.is_infinite() {
            return Ok(false);
        }
        let diff = (x - y).abs();
        Ok(diff <= (rel * y.abs()).max(rel * x.abs()) || diff <= abs)
    });
    add(vm, m, "factorial", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let n = Args::new(&args, vm, "factorial").i64(0, "n")?;
        if n < 0 {
            return Err(vm.new_value_error("factorial() not defined for negative values".to_owned()));
        }
        // 20! is the last that fits an i64; past it, a float would silently
        // lose digits, so the product is built as a Python int instead.
        let mut acc: PyObjectRef = convert::int(vm, 1);
        for i in 2..=n {
            acc = vm._mul(&acc, &convert::int(vm, i))?;
        }
        Ok(acc)
    });
    add(vm, m, "gcd", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<i64> {
        let a = Args::new(&args, vm, "gcd");
        let mut g: i64 = 0;
        for i in 0..a.len() {
            let mut x = a.i64(i, "x")?.abs();
            let mut y = g;
            while y != 0 {
                (x, y) = (y, x % y);
            }
            g = x;
        }
        Ok(g)
    });

    let module = vm.new_module("math", dict, None);
    vm.sys_module
        .get_attr("modules", vm)?
        .set_item("math", module.clone().into(), vm)?;
    Ok(module)
}

impl Args<'_> {
    /// A number at full precision, for `math`, where rounding through `f32`
    /// would make `math.pi` and `math.sqrt(2)` disagree with every reference.
    fn f64_arg(&self, i: usize, name: &str) -> PyResult<f64> {
        match self.get(i) {
            None => Err(self
                .vm()
                .new_type_error(format!("missing required argument '{name}'"))),
            Some(v) => convert::number(v, self.vm()).ok_or_else(|| {
                self.vm().new_type_error(format!(
                    "argument '{name}' must be a number, not {}",
                    v.class().name()
                ))
            }),
        }
    }
}
