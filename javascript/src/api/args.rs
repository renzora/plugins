//! Reading a native function's arguments.
//!
//! Every API function takes Boa's raw argument slice and reads it through
//! [`Args`] rather than converting each value by hand. The point is the error a
//! script sees, which names the function and the parameter
//! (`set_position(): argument 'z' must be a number, not string`) where a bare
//! `ToNumber` would have quietly turned a typo into `NaN` and moved the entity
//! to nowhere. JavaScript's own functions coerce; the engine's refuse, as they
//! do in the Lua and Python plugins.

use boa_engine::{Context, JsError, JsNativeError, JsResult, JsValue};

use crate::convert;

pub struct Args<'a> {
    args: &'a [JsValue],
    func: &'static str,
}

impl<'a> Args<'a> {
    pub fn new(args: &'a [JsValue], func: &'static str) -> Self {
        Self { args, func }
    }

    pub fn len(&self) -> usize {
        self.args.len()
    }

    /// The argument at `i`, unless it is missing, `undefined` or `null`.
    pub fn get(&self, i: usize) -> Option<&'a JsValue> {
        self.args.get(i).filter(|v| !v.is_null_or_undefined())
    }

    /// The argument at `i` as given, `undefined` when missing.
    pub fn raw(&self, i: usize) -> JsValue {
        self.args.get(i).cloned().unwrap_or_default()
    }

    pub fn missing(&self, name: &str) -> JsError {
        JsNativeError::typ()
            .with_message(format!("{}() missing required argument '{name}'", self.func))
            .into()
    }

    pub fn wrong(&self, name: &str, want: &str, got: &JsValue) -> JsError {
        JsNativeError::typ()
            .with_message(format!(
                "{}(): argument '{name}' must be {want}, not {}",
                self.func,
                type_name(got)
            ))
            .into()
    }

    pub fn f64(&self, i: usize, name: &str) -> JsResult<f64> {
        self.opt_f64(i, name)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_f64(&self, i: usize, name: &str) -> JsResult<Option<f64>> {
        match self.get(i) {
            None => Ok(None),
            Some(v) => convert::number(v)
                .map(Some)
                .ok_or_else(|| self.wrong(name, "a number", v)),
        }
    }

    pub fn f32(&self, i: usize, name: &str) -> JsResult<f32> {
        self.f64(i, name).map(|n| n as f32)
    }

    pub fn opt_f32(&self, i: usize, name: &str) -> JsResult<Option<f32>> {
        self.opt_f64(i, name).map(|n| n.map(|n| n as f32))
    }

    pub fn i64(&self, i: usize, name: &str) -> JsResult<i64> {
        self.opt_i64(i, name)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_i64(&self, i: usize, name: &str) -> JsResult<Option<i64>> {
        match self.get(i) {
            None => Ok(None),
            Some(v) => convert::integer(v)
                .map(Some)
                .ok_or_else(|| self.wrong(name, "a finite number", v)),
        }
    }

    /// JavaScript truthiness, so `set_visibility(0)` hides as
    /// `set_visibility(false)` does.
    pub fn bool(&self, i: usize, name: &str) -> JsResult<bool> {
        self.opt_bool(i).ok_or_else(|| self.missing(name))
    }

    pub fn opt_bool(&self, i: usize) -> Option<bool> {
        self.get(i).map(JsValue::to_boolean)
    }

    /// A string argument. Anything else is refused rather than stringified: a
    /// path or a name given a number is a mistake worth hearing about.
    pub fn str(&self, i: usize, name: &str) -> JsResult<String> {
        self.opt_str(i, name)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_str(&self, i: usize, name: &str) -> JsResult<Option<String>> {
        match self.get(i) {
            None => Ok(None),
            Some(v) => convert::string(v)
                .map(Some)
                .ok_or_else(|| self.wrong(name, "a string", v)),
        }
    }

    /// Any value, as `console.log` would show it. For log messages, which take
    /// anything.
    pub fn display(&self, i: usize, ctx: &mut Context) -> String {
        match self.args.get(i) {
            Some(v) => convert::display(v, ctx),
            None => String::new(),
        }
    }

    /// Three coordinates from `start`: either three numbers, or one vector (a
    /// `Vec3`, any `{x, y, z}` object, or a 3-number array) in their place.
    ///
    /// The second form is the one JavaScript code reaches for, since a
    /// position read back from `get("Transform.translation")` is a `Vec3`; Lua
    /// had no such form because a table cannot be spread into three arguments.
    pub fn xyz(&self, start: usize, ctx: &mut Context) -> JsResult<[f32; 3]> {
        self.xyz_at(start, ctx).map(|(v, _)| v)
    }

    /// [`xyz`](Self::xyz), plus how many arguments it consumed (1 or 3), for
    /// a function that takes more after the vector.
    pub fn xyz_at(&self, start: usize, ctx: &mut Context) -> JsResult<([f32; 3], usize)> {
        if let Some(v) = self.get(start) {
            if convert::number(v).is_none() {
                return match convert::shape(v, ctx) {
                    Some(convert::Shape::Vec3(v)) => Ok((v, 1)),
                    Some(convert::Shape::Vec2([x, y])) => Ok(([x, y, 0.0], 1)),
                    _ => Err(self.wrong("x", "a number or a vector", v)),
                };
            }
        }
        Ok((
            [
                self.f32(start, "x")?,
                self.f32(start + 1, "y")?,
                self.f32(start + 2, "z")?,
            ],
            3,
        ))
    }
}

/// A value's type as a script author would name it: `typeof`, except that an
/// array says `array` and `null` says `null` rather than `object`.
pub fn type_name(v: &JsValue) -> &'static str {
    if v.is_null() {
        return "null";
    }
    if v.as_object().is_some_and(|o| o.is_array()) {
        return "array";
    }
    v.type_of()
}
