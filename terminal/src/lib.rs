//! **Terminal** - a real shell in a dock panel.
//!
//! Not a log view that echoes commands: an operating-system pseudo-terminal with
//! a shell on the far end, so `vim`, `htop`, `git add -p`, a dev server and
//! `claude` all run full-screen inside the panel and behave exactly as they do
//! in a standalone terminal.
//!
//! The panel is deliberately the *only* thing this plugin registers. There is no
//! command vocabulary, no allow-list, no "run this for me" API - a terminal that
//! filters what may be typed into it is not a terminal, and the editor already
//! has a Console for the engine's own log and its slash commands. The two are
//! different panels on purpose.
//!
//! # Why this is a native plugin
//!
//! A C-ABI plugin links no Bevy and talks through a fixed function table, which
//! is right for a post-process effect and wrong here: the panel is a `bevy_ui`
//! tree built from ember's widgets, themed from ember's palette, and it needs
//! `InputFocusState` from the contract crate to hold the editor's shortcuts off
//! while the user is typing. All three are shared-image state that only a native
//! plugin can reach.
//!
//! Scope is `Editor`. A shipped game has no dock to put a panel in, and handing
//! a player a shell in the game's own process is not a feature anyone asked for.
//!
//! # Layout
//!
//! - [`pty`] - the shell process and its pseudo-terminal. Knows nothing of Bevy.
//! - [`keys`] - a Bevy key event to the bytes a shell expects.
//! - [`reply`] - the answers the emulator owes a shell that asks it a question.
//! - [`grid`] - one row of emulator cells to the styled runs the panel draws.
//! - [`state`] - the open shells, and which one is on screen.
//! - [`strip`] - the tab strip.
//! - [`panel`] - the dock panel, and everything that drives it.

use bevy::prelude::*;
use renzora::core::RenzoraShellExt;
use renzora_ember::panel::RegisterPanelContent;

mod grid;
mod keys;
mod panel;
mod pty;
mod reply;
mod state;
mod strip;

pub struct TerminalPlugin;

impl Plugin for TerminalPlugin {
    fn build(&self, app: &mut App) {
        info!("[editor] TerminalPlugin");
        app.init_resource::<state::Terminal>();
        // The tab's title, icon and category. A plugin owns its own entry: left
        // in the shell's table, disabling this plugin would leave "Terminal"
        // listed in Add Panel with nothing behind it.
        app.register_shell_panel(panel::PANEL_ID, "Terminal", "terminal-window", "Tools");
        // `scroll: false` - the grid is exactly as tall as the panel and scrolls
        // through history itself. An ember scroll view around it would fight the
        // emulator for what "scrolled to the bottom" means.
        app.register_panel_content(panel::PANEL_ID, false, panel::build)
            .systems(
                Update,
                (
                    // Chained because each step depends on the one before within
                    // the same frame: the metrics decide the grid size, the grid
                    // size decides which cell the pointer is over, and the input
                    // decides what `render` has to repaint. Left unordered, a
                    // keystroke would land a frame after the click that focused
                    // the panel.
                    panel::measure,
                    panel::resize,
                    panel::focus,
                    strip::clicks,
                    strip::gestures,
                    strip::overflow_menu,
                    strip::hover,
                    strip::focus_rename,
                    strip::rename_commit,
                    // Before `select`, so a press that lands on the scrollbar is
                    // claimed as a scroll rather than starting a text selection
                    // in the row behind it.
                    panel::scrollbar_drag,
                    panel::select,
                    panel::wheel,
                    panel::keyboard,
                    panel::claim_keyboard,
                    // After every system that can open, close, switch, rename or
                    // re-side a tab: the strip rebuilds from the tab list, so
                    // running it earlier would show the list as it was before
                    // this frame's click.
                    strip::sync,
                    // And after the rebuild, because it measures what the rebuild
                    // laid out. A frame late either way - `ComputedNode` is
                    // written in `PostUpdate` - which is invisible for a control
                    // that appears when a strip fills up.
                    strip::overflow,
                    // After the rebuild too, and for the same reason: it reads
                    // where layout actually put the active chip.
                    strip::keep_active_visible,
                    // Last, so the thumb reflects this frame's scrolling rather
                    // than last frame's.
                    panel::scrollbar_sync,
                    panel::render,
                )
                    .chain(),
            )
            // The one thing that must not pause with the tab. A pty nobody reads
            // fills its kernel buffer and blocks the program writing into it, so
            // backgrounding the tab would suspend a running build rather than
            // leave it running - and the output has to be there when the tab
            // comes back.
            .always(Update, panel::pump.before(panel::render));
    }
}

renzora::plugin!(TerminalPlugin, Editor);
