//! Reading a native function's arguments.
//!
//! Every API function takes the raw [`FuncArgs`] and reads it through [`Args`]
//! rather than declaring typed parameters. Two reasons: the error a script sees
//! can name the function and the parameter (`set_position(): argument 'z' must
//! be a number, not str`), and numbers coerce the way scripts expect, an `int`
//! being as good as a `float` anywhere a float is wanted. The Lua plugin got
//! the second for free from mlua; here it has to be written down once.

use rustpython_vm::function::FuncArgs;
use rustpython_vm::{AsObject, PyObjectRef, PyResult, VirtualMachine};

use crate::convert;

pub struct Args<'a> {
    args: &'a FuncArgs,
    vm: &'a VirtualMachine,
    func: &'static str,
}

impl<'a> Args<'a> {
    pub fn new(args: &'a FuncArgs, vm: &'a VirtualMachine, func: &'static str) -> Self {
        Self { args, vm, func }
    }

    pub fn vm(&self) -> &'a VirtualMachine {
        self.vm
    }

    pub fn len(&self) -> usize {
        self.args.args.len()
    }

    /// The argument at `i`, unless it is missing or `None`.
    pub fn get(&self, i: usize) -> Option<&'a PyObjectRef> {
        self.args.args.get(i).filter(|v| !self.vm.is_none(v))
    }

    /// The keyword arguments, for the functions that accept them.
    pub fn raw(&self) -> &'a FuncArgs {
        self.args
    }

    fn missing(&self, name: &str) -> rustpython_vm::builtins::PyBaseExceptionRef {
        self.vm
            .new_type_error(format!("{}() missing required argument '{name}'", self.func))
    }

    fn wrong(&self, name: &str, want: &str, got: &PyObjectRef) -> rustpython_vm::builtins::PyBaseExceptionRef {
        self.vm.new_type_error(format!(
            "{}(): argument '{name}' must be {want}, not {}",
            self.func,
            got.class().name()
        ))
    }

    pub fn f32(&self, i: usize, name: &str) -> PyResult<f32> {
        self.opt_f32(i, name)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_f32(&self, i: usize, name: &str) -> PyResult<Option<f32>> {
        match self.get(i) {
            None => Ok(None),
            Some(v) => convert::number(v, self.vm)
                .map(|n| Some(n as f32))
                .ok_or_else(|| self.wrong(name, "a number", v)),
        }
    }

    pub fn i64(&self, i: usize, name: &str) -> PyResult<i64> {
        self.opt_i64(i, name)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_i64(&self, i: usize, name: &str) -> PyResult<Option<i64>> {
        match self.get(i) {
            None => Ok(None),
            Some(v) => convert::integer(v, self.vm)
                .map(Some)
                .ok_or_else(|| self.wrong(name, "an int", v)),
        }
    }

    /// Python truthiness, so `set_visibility(0)` hides as `set_visibility(False)`.
    pub fn bool(&self, i: usize, name: &str) -> PyResult<bool> {
        self.opt_bool(i)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_bool(&self, i: usize) -> PyResult<Option<bool>> {
        match self.get(i) {
            None => Ok(None),
            Some(v) => v.clone().is_true(self.vm).map(Some),
        }
    }

    /// A string argument. Anything else is refused rather than `str()`ed: a
    /// path or a name given a number is a mistake worth hearing about.
    pub fn str(&self, i: usize, name: &str) -> PyResult<String> {
        self.opt_str(i, name)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_str(&self, i: usize, name: &str) -> PyResult<Option<String>> {
        match self.get(i) {
            None => Ok(None),
            Some(v) => convert::string(v)
                .map(Some)
                .ok_or_else(|| self.wrong(name, "a str", v)),
        }
    }

    /// Any value, as its `str()`. For log messages, which take anything.
    pub fn display(&self, i: usize) -> String {
        self.args
            .args
            .get(i)
            .map(|v| convert::display(v, self.vm))
            .unwrap_or_default()
    }

    /// Three coordinates from `start`: either three numbers, or one `vec3` (any
    /// 3-item tuple or list, or an `{x, y, z}` dict) in their place.
    ///
    /// The second form is the one Python code reaches for, since a position
    /// read back from `get("Transform.translation")` is a `vec3`; Lua had no
    /// such form because a table cannot be splatted into three arguments.
    pub fn xyz(&self, start: usize) -> PyResult<[f32; 3]> {
        self.xyz_at(start).map(|(v, _)| v)
    }

    /// [`xyz`](Self::xyz), plus how many arguments it consumed (1 or 3), for
    /// a function that takes more after the vector.
    pub fn xyz_at(&self, start: usize) -> PyResult<([f32; 3], usize)> {
        if let Some(v) = self.get(start) {
            if convert::number(v, self.vm).is_none() {
                return match convert::shape(v, self.vm) {
                    Some(convert::Shape::Vec3(v)) => Ok((v, 1)),
                    Some(convert::Shape::Vec2([x, y])) => Ok(([x, y, 0.0], 1)),
                    _ => Err(self.wrong("x", "a number or a vec3", v)),
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
