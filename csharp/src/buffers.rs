//! Where the C# side's calls put what they produce, and what it hands back.
//!
//! A script's `Translate(...)` arrives as a call through the native table
//! (see [`crate::bridge`]) with no path back to the backend that is running the
//! hook. It pushes into a thread-local instead, and the hook collects afterwards,
//! exactly as the Lua, Python and JavaScript plugins do: forty commands are
//! forty pushes and one handoff.
//!
//! Thread-local rather than a `Mutex` because C# calls back on the thread that
//! called into it, which is the thread running the hook: the producer and the
//! collector are always the same thread.
//!
//! The result slots are the other direction. An entry point that has something
//! to return besides its status (an exception's text, the props a hook
//! changed, the load report) hands it over through `set_result` before
//! returning, into the slot of that kind on this thread.

use std::cell::RefCell;

use renzora::core::script_bridge::DrawCmd;
use renzora::ScriptCommand;

/// One slot per [`crate::abi::result`] kind.
const SLOTS: usize = 3;

thread_local! {
    static COMMANDS: RefCell<Vec<ScriptCommand>> = const { RefCell::new(Vec::new()) };

    /// Kept separate from [`COMMANDS`] because draws are not ECS commands:
    /// they are a per-frame list the UI vector renderer reconciles, rebuilt
    /// from scratch every frame rather than applied once.
    static DRAWS: RefCell<Vec<DrawCmd>> = const { RefCell::new(Vec::new()) };

    static RESULTS: RefCell<[Option<String>; SLOTS]> = const { RefCell::new([None, None, None]) };

    /// What the last `call` returned, kept alive until the next one so the
    /// pointer handed to C# stays valid while it copies.
    static CALL_OUT: RefCell<String> = const { RefCell::new(String::new()) };
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

pub fn set_result(kind: u32, text: String) {
    RESULTS.with(|r| {
        if let Some(slot) = r.borrow_mut().get_mut(kind as usize) {
            *slot = Some(text);
        }
    });
}

pub fn take_result(kind: u32) -> Option<String> {
    RESULTS.with(|r| r.borrow_mut().get_mut(kind as usize).and_then(Option::take))
}

/// Forget every result, before an entry point is called, so one left by an
/// earlier call is never mistaken for this one's.
pub fn clear_results() {
    RESULTS.with(|r| *r.borrow_mut() = [None, None, None]);
}

/// Keep `text` as the last call's output and return a pointer to it, valid
/// until the next [`keep_call_out`] on this thread.
pub fn keep_call_out(text: String) -> (*const u8, usize) {
    CALL_OUT.with(|c| {
        let mut c = c.borrow_mut();
        *c = text;
        (c.as_ptr(), c.len())
    })
}
