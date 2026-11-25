//! Every engine function a script can call, grouped by what it touches.
//!
//! The names, arguments and behaviour are the Lua plugin's `register_api`
//! (and so the Python plugin's), function for function, so the languages
//! document as one API. Each submodule adds its functions to the `renzora`
//! object; the runtime then copies them onto the global object, which is what
//! lets a script call `set_position(...)` with no import, the way a Lua script
//! calls a global.
//!
//! Every function is a plain `fn` pointer (a closure that captures nothing
//! coerces to one). That is Boa's cheapest native function, and the only kind
//! it can build without `unsafe`: a capturing closure has to promise Boa's
//! garbage collector that it holds no JavaScript object, which a pointer never
//! can. The functions that need to know something, the declared bindings, are
//! in [`crate::bindings`].

use boa_engine::native_function::NativeFunctionPointer;
use boa_engine::object::FunctionObjectBuilder;
use boa_engine::{Context, JsObject, NativeFunction};

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

/// Register one native function under `name` on `m`, with `name` as its
/// JavaScript `.name` too, so a stack trace through it reads `set_position`
/// rather than an anonymous native.
pub fn add(ctx: &mut Context, m: &JsObject, name: &'static str, f: NativeFunctionPointer) {
    let func = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(f))
        .name(crate::convert::key(name))
        .build();
    if let Err(e) = m.set(crate::convert::key(name), func, false, ctx) {
        bevy::log::warn!("[JavaScript] could not register `{name}`: {e}");
    }
}

/// Register a function whose whole job is to push one command built from its
/// arguments. Most of the API is this shape, and spelling each one out as a
/// closure with its own `Ok(undefined)` would bury the command in boilerplate.
///
/// `$a` is the [`args::Args`] and `$c` the `Context`, for the arguments that
/// read a vector.
macro_rules! command {
    ($ctx:expr, $m:expr, $name:literal, |$a:ident, $c:ident| $cmd:expr) => {
        $crate::api::add($ctx, $m, $name, |_this, args, $c| {
            let _ = &$c;
            let $a = $crate::api::args::Args::new(args, $name);
            $crate::buffers::push_command($cmd);
            Ok(boa_engine::JsValue::undefined())
        })
    };
}
pub(crate) use command;

/// Every engine function, onto the `renzora` object.
pub fn register(ctx: &mut Context, m: &JsObject) {
    transform::register(ctx, m);
    input::register(ctx, m);
    save::register(ctx, m);
    media::register(ctx, m);
    world::register(ctx, m);
    reflect::register(ctx, m);
    events::register(ctx, m);
    math::register(ctx, m);
    draw::register(ctx, m);
}
