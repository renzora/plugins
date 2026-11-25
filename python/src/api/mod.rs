//! Every engine function a script can call, grouped by what it touches.
//!
//! The names, arguments and behaviour are the Lua plugin's `register_api`,
//! function for function, so the two languages document as one API. Each
//! submodule adds its functions to the `renzora` module's dict; the runtime
//! then copies them into `builtins`, which is what lets a script call
//! `set_position(...)` with no import, the way a Lua script calls a global.
//!
//! Every function is a plain `fn` item or a closure that captures nothing.
//! That is load-bearing: RustPython leaks the callable of every function it
//! registers (`Box::leak` in `new_method_def`), and a zero-sized one leaks
//! nothing. The interpreter is built once per thread, so even a capturing
//! closure would leak only a little, but the declared bindings, which do vary,
//! go through one shared dispatcher for the same reason (see `bindings`).

use rustpython_vm::builtins::PyDict;
use rustpython_vm::function::IntoPyNativeFn;
use rustpython_vm::{Py, VirtualMachine};

pub mod args;
mod draw;
mod events;
mod input;
mod math;
mod media;
mod reflect;
mod save;
mod transform;
mod world;

pub use draw::canvas;
pub use math::register_math_module;

/// Register one native function under `name` in the module dict `m`.
pub fn add<F, K>(vm: &VirtualMachine, m: &Py<PyDict>, name: &'static str, f: F)
where
    F: IntoPyNativeFn<K>,
{
    let func = vm.new_function(name, f);
    if let Err(e) = m.set_item(name, func.into(), vm) {
        bevy::log::warn!("[Python] could not register `{name}`: {e:?}");
    }
}

/// Register a function whose whole job is to push one command built from its
/// arguments. Most of the API is this shape, and spelling each one out as a
/// closure with its own `Ok(())` would bury the command in boilerplate.
macro_rules! command {
    ($vm:expr, $m:expr, $name:literal, |$a:ident| $cmd:expr) => {
        $crate::api::add(
            $vm,
            $m,
            $name,
            |args: rustpython_vm::function::FuncArgs,
             vm: &rustpython_vm::VirtualMachine|
             -> rustpython_vm::PyResult<()> {
                let $a = $crate::api::args::Args::new(&args, vm, $name);
                $crate::buffers::push_command($cmd);
                Ok(())
            },
        )
    };
}
pub(crate) use command;

/// Every engine function, into the `renzora` module's dict.
pub fn register(vm: &VirtualMachine, m: &Py<PyDict>) {
    transform::register(vm, m);
    input::register(vm, m);
    save::register(vm, m);
    media::register(vm, m);
    world::register(vm, m);
    reflect::register(vm, m);
    events::register(vm, m);
    math::register(vm, m);
    draw::register(vm, m);
}
