//! Where a JavaScript function puts what it produced.
//!
//! Every API function is a native function with no path back to the backend
//! that registered it: most are plain `fn` pointers (Boa's cheapest kind of
//! native function, and the kind that cannot capture anything). They push into
//! a thread-local instead, and the hook collects afterwards, exactly as the Lua
//! and Python plugins do. A script issuing forty commands does forty pushes and
//! one handoff.
//!
//! Thread-local rather than a `Mutex` because the interpreter that runs a hook
//! is itself thread-local (see [`crate::runtime`]): the producer and the
//! collector are always the same thread.

use std::cell::RefCell;

use renzora::core::script_bridge::DrawCmd;
use renzora::ScriptCommand;

thread_local! {
    static COMMANDS: RefCell<Vec<ScriptCommand>> = const { RefCell::new(Vec::new()) };

    /// Kept separate from [`COMMANDS`] because draws are not ECS commands:
    /// they are a per-frame list the UI vector renderer reconciles, rebuilt
    /// from scratch every frame rather than applied once.
    static DRAWS: RefCell<Vec<DrawCmd>> = const { RefCell::new(Vec::new()) };
}

pub fn push_command(cmd: ScriptCommand) {
    COMMANDS.with(|b| b.borrow_mut().push(cmd));
}

pub fn drain_commands() -> Vec<ScriptCommand> {
    COMMANDS.with(|b| std::mem::take(&mut *b.borrow_mut()))
}

pub fn push_draw(cmd: DrawCmd) {
    DRAWS.with(|b| b.borrow_mut().push(cmd));
}

pub fn drain_draws() -> Vec<DrawCmd> {
    DRAWS.with(|b| std::mem::take(&mut *b.borrow_mut()))
}
