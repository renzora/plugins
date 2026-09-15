//! Turning a Bevy key event into the bytes a shell expects.
//!
//! A terminal has no "key" concept: a program reads a byte stream and infers
//! keys from it, using conventions that are almost entirely inherited from the
//! DEC VT series. So this module is a translation table, and the two rules worth
//! stating up front are the ones that are not obvious from the table.
//!
//! **Printable text comes from `KeyboardInput::text`, not from `key_code`.**
//! `text` is what the platform produced after applying the user's layout, dead
//! keys and modifiers, so `Shift+2` is `@` on a US layout and `"` on a UK one
//! without this file knowing either exists. Deriving characters from `key_code`
//! is how a terminal ends up unable to type `#` on a French keyboard.
//!
//! **Control characters are computed, not looked up.** `Ctrl+<letter>` is the
//! letter with bit 6 cleared - `Ctrl+A` is 1, `Ctrl+C` is 3, `Ctrl+D` is 4 - which
//! is why the interrupt is on `C` and end-of-file is on `D` in the first place.
//! Anything that treats those as a special-cased list gets the handful outside
//! the alphabet (`Ctrl+[`, `Ctrl+\`, `Ctrl+?`) wrong.

use bevy::input::keyboard::{Key, KeyboardInput};

/// Modifier state at the moment of the keypress.
#[derive(Clone, Copy, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    /// The xterm modifier parameter: `1 + shift + 2*alt + 4*ctrl`, used in the
    /// `CSI 1 ; <n> <letter>` form that carries a modified arrow or Home/End.
    /// `1` means "no modifiers", and a sequence is only written in the modified
    /// form when this is greater than 1.
    fn param(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }

    fn any(self) -> bool {
        self.ctrl || self.alt || self.shift
    }
}

/// The bytes `ev` should send to the shell, or `None` for a key a terminal has
/// nothing to say about (a bare modifier, a media key, an unmapped `Fn` row).
///
/// `app_cursor` is DECCKM, the mode a full-screen program turns on to receive
/// `ESC O A` for Up instead of `ESC [ A`. Both mean the same key; readline-based
/// shells and curses programs disagree about which they want, and the terminal
/// is told which to send. Getting this wrong is the classic "arrow keys print
/// `^[[A` inside vim" bug.
pub fn encode(ev: &KeyboardInput, m: Mods, app_cursor: bool) -> Option<Vec<u8>> {
    // `Alt+<key>` is sent as ESC followed by the unmodified key. That is the
    // "meta prefix" convention every shell's readline expects (Alt+B / Alt+F to
    // move by word), and it composes with everything below rather than needing
    // its own table.
    let esc = |mut bytes: Vec<u8>| -> Option<Vec<u8>> {
        if m.alt {
            let mut out = vec![0x1b];
            out.append(&mut bytes);
            Some(out)
        } else {
            Some(bytes)
        }
    };

    match &ev.logical_key {
        Key::Enter => esc(vec![b'\r']),
        // DEL (0x7f), not BS (0x08). Unix terminals have sent DEL for the
        // backspace key since the VT220, and `stty erase` is configured to match;
        // sending 0x08 makes backspace insert a literal `^H` in most shells.
        Key::Backspace => esc(vec![0x7f]),
        Key::Tab => {
            if m.shift {
                // Back-tab, for TUIs that cycle focus backwards.
                Some(b"\x1b[Z".to_vec())
            } else {
                esc(vec![b'\t'])
            }
        }
        Key::Escape => esc(vec![0x1b]),
        Key::Space if m.ctrl => {
            // Ctrl+Space is NUL, which emacs-mode readline reads as "set mark".
            esc(vec![0])
        }

        Key::ArrowUp => Some(cursor_key(b'A', m, app_cursor)),
        Key::ArrowDown => Some(cursor_key(b'B', m, app_cursor)),
        Key::ArrowRight => Some(cursor_key(b'C', m, app_cursor)),
        Key::ArrowLeft => Some(cursor_key(b'D', m, app_cursor)),
        Key::Home => Some(cursor_key(b'H', m, app_cursor)),
        Key::End => Some(cursor_key(b'F', m, app_cursor)),

        Key::Insert => Some(tilde_key(2, m)),
        Key::Delete => Some(tilde_key(3, m)),
        Key::PageUp => Some(tilde_key(5, m)),
        Key::PageDown => Some(tilde_key(6, m)),

        // F1-F4 are `SS3` keys, F5 upwards are `CSI <n> ~`, and the numbering
        // skips 16, 22, 27, 30 and 35. That is not a mistake in this table: the
        // gaps are in the VT220 layout every terminal still emulates.
        Key::F1 => Some(func_key(b'P', 11, m)),
        Key::F2 => Some(func_key(b'Q', 12, m)),
        Key::F3 => Some(func_key(b'R', 13, m)),
        Key::F4 => Some(func_key(b'S', 14, m)),
        Key::F5 => Some(tilde_key(15, m)),
        Key::F6 => Some(tilde_key(17, m)),
        Key::F7 => Some(tilde_key(18, m)),
        Key::F8 => Some(tilde_key(19, m)),
        Key::F9 => Some(tilde_key(20, m)),
        Key::F10 => Some(tilde_key(21, m)),
        Key::F11 => Some(tilde_key(23, m)),
        Key::F12 => Some(tilde_key(24, m)),

        Key::Character(s) if m.ctrl => esc(control(s)?),

        // The ordinary case, and deliberately last: anything the platform
        // produced text for is sent as that text. `Key::Character` is checked
        // rather than `text` alone so a key that produces text incidentally
        // (some layouts emit `\r` in `text` for Enter) has already been handled
        // above by its named variant.
        Key::Character(_) | Key::Space => {
            let text = ev.text.as_ref()?;
            if text.is_empty() {
                return None;
            }
            esc(text.as_bytes().to_vec())
        }
        _ => None,
    }
}

/// `Ctrl+<char>`: clear bit 6 of the uppercased ASCII character.
///
/// Only defined for `@A-Z[\]^_` and `?`; `Ctrl+5` has no control character and
/// returns `None` rather than inventing one, so the keypress does nothing
/// instead of sending a byte the shell will act on.
fn control(s: &str) -> Option<Vec<u8>> {
    let c = s.chars().next()?.to_ascii_uppercase();
    match c {
        '@'..='_' => Some(vec![(c as u8) & 0x1f]),
        // Ctrl+? is DEL, the one control character above the letters.
        '?' => Some(vec![0x7f]),
        _ => None,
    }
}

/// An arrow / Home / End key.
///
/// Unmodified it is `ESC [ <letter>`, or `ESC O <letter>` in application-cursor
/// mode. With a modifier it is always `ESC [ 1 ; <n> <letter>` - the application
/// form has no place to put the parameter, so xterm switches back to CSI, and
/// programs expect that.
fn cursor_key(letter: u8, m: Mods, app_cursor: bool) -> Vec<u8> {
    if m.any() {
        format!("\x1b[1;{}{}", m.param(), letter as char).into_bytes()
    } else if app_cursor {
        vec![0x1b, b'O', letter]
    } else {
        vec![0x1b, b'[', letter]
    }
}

/// A `CSI <n> ~` key (Insert, Delete, PageUp, PageDown, F5 and above).
fn tilde_key(n: u8, m: Mods) -> Vec<u8> {
    if m.any() {
        format!("\x1b[{n};{}~", m.param()).into_bytes()
    } else {
        format!("\x1b[{n}~").into_bytes()
    }
}

/// F1-F4: `SS3 <letter>` unmodified, `CSI <n> ; <mod> ~` once a modifier is held.
fn func_key(letter: u8, n: u8, m: Mods) -> Vec<u8> {
    if m.any() {
        format!("\x1b[{n};{}~", m.param()).into_bytes()
    } else {
        vec![0x1b, b'O', letter]
    }
}
