//! Every engine function a script can call, by name.
//!
//! The names, arguments and behaviour are the Lua plugin's `register_api` (and
//! so the Python and JavaScript plugins'), function for function. The C# side
//! wraps each in an idiomatic member of `Script` (`PlaySound(path)` for
//! `play_sound`), but underneath every one arrives here as a name and a JSON
//! array of arguments, through the one `call` entry of the native table. That
//! is also what `Call("play_sound", "boom.wav")` reaches, so the long tail and
//! anything added after the C# wrappers were written stay callable by name.
//!
//! A few calls are too frequent for JSON and have entries of their own in the
//! native table (`Translate`, `SetPosition`, `Log`, key and action reads; see
//! [`crate::bridge`]). Their names are answered here too, so `Call("translate",
//! 1, 0, 0)` behaves exactly like `Translate(1, 0, 0)`.
//!
//! Declared bindings (`apply_force` from physics, `mysql_query` from the MySQL
//! plugin, ...) are looked up first, as a binding registered after
//! `register_api` overrides the built-in in Lua.

use renzora::serde_json::Value;

mod draw;
mod events;
mod input;
mod media;
mod reflect;
mod save;
mod transform;
mod world;

pub use reflect::{read as read_field, split_path};

/// The result of one engine function: a JSON value, or the error the C# side
/// throws as a `ScriptException`.
pub type Out = Result<Value, String>;

/// Run the engine function or declared binding `name`.
pub fn call(name: &str, args: &[Value]) -> Out {
    if let Some(result) = crate::bindings::call(name, args) {
        return result;
    }
    let a = Args { func: name, args };
    let groups: [fn(&str, &Args) -> Option<Out>; 8] = [
        transform::call,
        input::call,
        world::call,
        media::call,
        reflect::call,
        events::call,
        save::call,
        draw::call,
    ];
    for group in groups {
        if let Some(result) = group(name, &a) {
            return result;
        }
    }
    Err(format!(
        "there is no engine function or plugin function called `{name}` (is the plugin that declares it installed?)"
    ))
}

/// Reading a call's arguments.
///
/// The point is the error a script sees, which names the function and the
/// parameter (`set_position(): argument 'z' must be a number, not string`).
/// C# already type-checks the idiomatic wrappers at compile time; this is for
/// `Call(...)`, which takes `object` and would otherwise turn a mistake into a
/// silent zero.
pub struct Args<'a> {
    pub func: &'a str,
    pub args: &'a [Value],
}

impl<'a> Args<'a> {
    /// The argument at `i`, unless it is missing or `null`.
    pub fn get(&self, i: usize) -> Option<&'a Value> {
        self.args.get(i).filter(|v| !v.is_null())
    }

    pub fn raw(&self, i: usize) -> Value {
        self.args.get(i).cloned().unwrap_or(Value::Null)
    }

    pub fn len(&self) -> usize {
        self.args.len()
    }

    pub fn missing(&self, name: &str) -> String {
        format!("{}() missing required argument '{name}'", self.func)
    }

    fn wrong(&self, name: &str, want: &str, got: &Value) -> String {
        format!(
            "{}(): argument '{name}' must be {want}, not {}",
            self.func,
            crate::convert::type_name(got)
        )
    }

    pub fn opt_f64(&self, i: usize, name: &str) -> Result<Option<f64>, String> {
        match self.get(i) {
            None => Ok(None),
            Some(v) => crate::convert::number(v)
                .map(Some)
                .ok_or_else(|| self.wrong(name, "a number", v)),
        }
    }

    pub fn f64(&self, i: usize, name: &str) -> Result<f64, String> {
        self.opt_f64(i, name)?.ok_or_else(|| self.missing(name))
    }

    pub fn f32(&self, i: usize, name: &str) -> Result<f32, String> {
        self.f64(i, name).map(|n| n as f32)
    }

    pub fn opt_f32(&self, i: usize, name: &str) -> Result<Option<f32>, String> {
        self.opt_f64(i, name).map(|n| n.map(|n| n as f32))
    }

    pub fn i64(&self, i: usize, name: &str) -> Result<i64, String> {
        match self.get(i) {
            None => Err(self.missing(name)),
            Some(v) => crate::convert::whole(v).ok_or_else(|| self.wrong(name, "a whole number", v)),
        }
    }

    pub fn bool(&self, i: usize, name: &str) -> Result<bool, String> {
        self.opt_bool(i, name)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_bool(&self, i: usize, name: &str) -> Result<Option<bool>, String> {
        match self.get(i) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(v) => Err(self.wrong(name, "a bool", v)),
        }
    }

    /// A string argument. Anything else is refused rather than stringified: a
    /// path or a name given a number is a mistake worth hearing about.
    pub fn str(&self, i: usize, name: &str) -> Result<String, String> {
        self.opt_str(i, name)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_str(&self, i: usize, name: &str) -> Result<Option<String>, String> {
        match self.get(i) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(v) => Err(self.wrong(name, "a string", v)),
        }
    }

    /// Any value, as a log line shows it.
    pub fn display(&self, i: usize) -> String {
        self.args.get(i).map(crate::convert::display).unwrap_or_default()
    }

    /// Three coordinates from `start`: three numbers, or one vector in their
    /// place.
    pub fn xyz(&self, start: usize) -> Result<[f32; 3], String> {
        self.xyz_at(start).map(|(v, _)| v)
    }

    /// [`xyz`](Self::xyz), plus how many arguments it consumed (1 or 3).
    pub fn xyz_at(&self, start: usize) -> Result<([f32; 3], usize), String> {
        if let Some(v) = self.get(start) {
            if crate::convert::number(v).is_none() {
                return match crate::convert::shape(v) {
                    Some(crate::convert::Shape::Vec3(v)) => Ok((v, 1)),
                    Some(crate::convert::Shape::Vec2([x, y])) => Ok(([x, y, 0.0], 1)),
                    _ => Err(self.wrong("x", "a number or a vector", v)),
                };
            }
        }
        Ok(([self.f32(start, "x")?, self.f32(start + 1, "y")?, self.f32(start + 2, "z")?], 3))
    }

    /// A colour at `i`: a `Color`, a `Vector3` (alpha 1), `#rrggbb` text, or
    /// `None` when absent.
    pub fn opt_color(&self, i: usize) -> Option<[f32; 4]> {
        let v = self.get(i)?;
        if let Value::String(s) = v {
            return Some(draw::parse_hex(s));
        }
        match crate::convert::shape(v)? {
            crate::convert::Shape::Color(c) => Some(c),
            crate::convert::Shape::Vec3([r, g, b]) => Some([r, g, b, 1.0]),
            crate::convert::Shape::Vec2(_) => None,
        }
    }
}

/// Push one command and return nothing, the shape of most of the API.
pub fn push(cmd: renzora::ScriptCommand) -> Out {
    crate::buffers::push_command(cmd);
    Ok(Value::Null)
}
