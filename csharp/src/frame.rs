//! The context of the hook that is running right now.
//!
//! The per-frame values a script reads most (`Delta`, `Position`, the first
//! gamepad) travel to C# by value in the hook's [`crate::abi::HookContext`].
//! Everything else it can ask about mid-hook (a key by name, an InputMap
//! action, the second gamepad, a touch) is answered here, from the engine's
//! [`ScriptContext`], when C# calls back for it. Nothing is copied ahead of
//! time, so a script that reads one key pays for one lookup, not for the
//! sixty-odd values the context holds.
//!
//! Outside a hook (reading a script's props) there is no context, and every
//! read answers as if nothing were pressed and no time had passed.

use std::cell::Cell;
use std::marker::PhantomData;

use renzora::ScriptContext;

thread_local! {
    static CURRENT: Cell<*const ScriptContext> = const { Cell::new(std::ptr::null()) };
}

/// Installs a context for as long as it lives, and restores the previous one
/// (normally none) when dropped.
///
/// The lifetime ties the guard to the borrow it was made from, so the pointer
/// cannot outlive the context it points at: [`with`] only dereferences it while
/// a guard exists, and a guard cannot exist after its `&ScriptContext` ends.
pub struct Frame<'a> {
    previous: *const ScriptContext,
    _borrow: PhantomData<&'a ScriptContext>,
}

pub fn enter(ctx: &ScriptContext) -> Frame<'_> {
    let previous = CURRENT.with(|c| c.replace(ctx as *const ScriptContext));
    Frame {
        previous,
        _borrow: PhantomData,
    }
}

impl Drop for Frame<'_> {
    fn drop(&mut self) {
        CURRENT.with(|c| c.set(self.previous));
    }
}

/// Run `f` against the current hook's context, or return `None` outside one.
pub fn with<R>(f: impl FnOnce(&ScriptContext) -> R) -> Option<R> {
    let ptr = CURRENT.with(|c| c.get());
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the pointer is only ever non-null while a `Frame` made from a live
    // `&ScriptContext` exists on this thread (it is thread-local, and `Drop`
    // restores the previous value), so it points at a context that is still
    // borrowed by the hook currently running.
    Some(f(unsafe { &*ptr }))
}
