//! What the emulator has to say back to the shell.
//!
//! A terminal is not only a screen. Some escape sequences are *questions*, and a
//! program that asks one stops dead until the answer arrives. PowerShell is the
//! case that makes this unavoidable: PSReadLine opens by sending `ESC[6n`
//! ("where is the cursor?") and writes nothing at all until it is told, not even
//! the banner. A panel that never answers shows an empty grid with a cursor in
//! the corner, which reads as a terminal that failed to start rather than one
//! that is waiting.
//!
//! `vt100` draws the screen and stops there. It owns no writer, so a query it
//! cannot answer from the screen alone arrives at
//! [`Callbacks::unhandled_csi`], and that is the right place to answer from:
//! the callback runs with the screen in exactly the state the question was
//! asked about. Answering later, from the panel, would report wherever the rest
//! of the same chunk has since moved the cursor to.
//!
//! Only the status reports are answered. A Device Attributes query (`ESC[c`)
//! goes deliberately unanswered, because the reply is a *claim about what this
//! emulator supports* and an invented one talks programs into escape sequences
//! the grid cannot draw. A cursor position is a fact about the screen, and the
//! screen is here.

use vt100::{Callbacks, Screen};

/// Replies owed to the shell, waiting to be written back into the pty.
///
/// A `Vec` rather than an immediate write because the callback has no reach to
/// the pty and should not: `process` is called from the panel's pump, which
/// holds the session already and writes whatever accumulated as soon as the
/// chunk is parsed.
#[derive(Default)]
pub struct Replies {
    pending: Vec<u8>,
}

impl Replies {
    /// Take everything the last `process` produced.
    pub fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending)
    }
}

impl Callbacks for Replies {
    fn unhandled_csi(
        &mut self,
        screen: &mut Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        if c != 'n' {
            return;
        }
        let ps = params.first().and_then(|p| p.first()).copied();
        match (i1, ps) {
            // DSR 5: "are you there?". `0n` is the healthy answer, and a program
            // that asks is usually deciding whether it is talking to a terminal
            // at all.
            (None, Some(5)) => self.pending.extend_from_slice(b"\x1b[0n"),
            // DSR 6: the cursor position, 1-based on the wire while the emulator
            // counts rows and columns from zero.
            (None, Some(6)) => {
                let (row, col) = screen.cursor_position();
                self.pending.extend_from_slice(report(row, col, false).as_bytes());
            }
            // DECXCPR, the same question with the page number added to the
            // answer. There is one page, so it is always 1. Answered because a
            // program that asks this form and gets the plain `R` back cannot
            // parse it and waits out its timeout instead.
            (Some(b'?'), Some(6)) => {
                let (row, col) = screen.cursor_position();
                self.pending.extend_from_slice(report(row, col, true).as_bytes());
            }
            _ => {}
        }
    }
}

/// A cursor position report, in the plain (`CPR`) or extended (`DECXCPR`) form.
fn report(row: u16, col: u16, extended: bool) -> String {
    if extended {
        format!("\x1b[?{};{};1R", row + 1, col + 1)
    } else {
        format!("\x1b[{};{}R", row + 1, col + 1)
    }
}
