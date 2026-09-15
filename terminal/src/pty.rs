//! The shell process and the pseudo-terminal it talks through.
//!
//! Nothing here knows about Bevy or about drawing. The rest of the plugin gets
//! four operations - drain what the shell wrote, write what the user typed,
//! resize, and ask whether it is still alive - and this module is careful about
//! exactly two things.
//!
//! **The read has to happen on its own thread.** A pty master has no
//! non-blocking read that is portable: `read` returns when the shell has
//! something to say, which may be in a microsecond or may be never. Calling that
//! from a Bevy system would park the whole editor on the shell's output, so a
//! thread reads and appends into [`Session::pending`], and the main thread takes
//! whatever has accumulated since last frame.
//!
//! **Every handle is behind a `Mutex`.** Not for contention - the reader thread
//! touches only `pending` - but because a Bevy `Resource` must be `Sync` and
//! `portable_pty` hands out `Box<dyn MasterPty + Send>` and
//! `Box<dyn Write + Send>`. `Mutex<T>` is `Sync` whenever `T` is `Send`, which is
//! the smallest correct way to hold them.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};

/// How much unread output is kept when the main thread falls behind.
///
/// A frame's worth of a well-behaved shell is a few KB. A runaway one (`yes`, a
/// `cat` of a large binary) can produce megabytes between two frames, and the
/// reader thread cannot apply back-pressure without also blocking the shell -
/// which would hang whatever the user is running. So the buffer is capped and
/// the OLDEST bytes are dropped: what the user ends up seeing is the tail, which
/// is what a terminal shows anyway. A dropped chunk can cut an escape sequence in
/// half and briefly corrupt the styling of one screen; the next full redraw fixes
/// it, and the alternative is an unbounded allocation.
const MAX_PENDING: usize = 4 * 1024 * 1024;

/// One shell, running under a pty.
pub struct Session {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    /// Output the reader thread has collected and the main thread has not taken.
    pending: Arc<Mutex<Vec<u8>>>,
    /// Cleared by the reader thread when the shell closes its end. The pty
    /// reports the shell exiting as EOF on the master, which is the earliest and
    /// most reliable signal available - `try_wait` on the child can still say
    /// "running" for a moment after.
    alive: Arc<AtomicBool>,
    /// What was actually launched, for the "session ended" line and for the log.
    pub program: String,
}

impl Session {
    /// Launch `shell()` under a new pty sized `rows` x `cols`, starting in `cwd`.
    pub fn spawn(rows: u16, cols: u16, cwd: Option<&Path>) -> Result<Self, String> {
        let program = shell();
        let pty = portable_pty::native_pty_system()
            .openpty(PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width: 0, pixel_height: 0 })
            .map_err(|e| format!("could not open a pseudo-terminal: {e}"))?;

        let mut cmd = CommandBuilder::new(&program);
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }
        // The shell and everything it launches read `TERM` to decide what it may
        // emit. Claiming `xterm-256color` is a promise about what the emulator
        // on the other end understands, and vt100 does: 256-colour SGR, the
        // alternate screen, bracketed paste, application cursor keys. Leaving it
        // at whatever the editor inherited is how a terminal ends up with `vim`
        // refusing to start ("terminal is not fully functional") on a machine
        // launched from a desktop entry with no TERM at all.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let child = pty
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("could not start `{program}`: {e}"))?;
        // Drop the slave immediately. It is the only other holder of the pty's
        // far end, and while this process keeps it open the master never sees
        // EOF - so a shell that exits would leave the panel waiting forever for
        // output that cannot come.
        drop(pty.slave);

        let writer = pty
            .master
            .take_writer()
            .map_err(|e| format!("could not open the terminal for writing: {e}"))?;
        let mut reader = pty
            .master
            .try_clone_reader()
            .map_err(|e| format!("could not open the terminal for reading: {e}"))?;

        let pending = Arc::new(Mutex::new(Vec::new()));
        let alive = Arc::new(AtomicBool::new(true));
        {
            let pending = Arc::clone(&pending);
            let alive = Arc::clone(&alive);
            std::thread::Builder::new()
                .name("renzora-terminal-read".into())
                .spawn(move || {
                    let mut buf = [0u8; 8192];
                    loop {
                        match reader.read(&mut buf) {
                            // EOF: the shell closed its end, which is how it
                            // reports having exited.
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                let Ok(mut pending) = pending.lock() else {
                                    break;
                                };
                                pending.extend_from_slice(&buf[..n]);
                                if pending.len() > MAX_PENDING {
                                    let excess = pending.len() - MAX_PENDING;
                                    pending.drain(..excess);
                                }
                            }
                        }
                    }
                    alive.store(false, Ordering::Release);
                })
                .map_err(|e| format!("could not start the terminal reader thread: {e}"))?;
        }

        Ok(Self {
            master: Mutex::new(pty.master),
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            pending,
            alive,
            program,
        })
    }

    /// Take everything the shell has written since the last call.
    pub fn drain(&self) -> Vec<u8> {
        let Ok(mut pending) = self.pending.lock() else {
            return Vec::new();
        };
        std::mem::take(&mut *pending)
    }

    /// Send keystrokes (or a paste) to the shell.
    pub fn write(&self, bytes: &[u8]) {
        let Ok(mut writer) = self.writer.lock() else {
            return;
        };
        // A failed write means the shell is gone; `alive` will follow from the
        // reader's EOF a moment later, so there is nothing useful to report here
        // that the panel is not about to say anyway.
        let _ = writer.write_all(bytes);
        let _ = writer.flush();
    }

    /// Tell the kernel the window changed size, which is what raises `SIGWINCH`
    /// in the shell and makes a full-screen program re-flow.
    pub fn resize(&self, rows: u16, cols: u16) {
        let Ok(master) = self.master.lock() else {
            return;
        };
        let _ = master.resize(PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }
}

impl Drop for Session {
    /// Kill the shell with the panel.
    ///
    /// Closing the master would eventually hang up the far end on its own, but
    /// "eventually" is doing real work there: a program ignoring `SIGHUP` (a
    /// `nohup`'d build, a shell with `huponexit` off) would survive as an
    /// orphan with no terminal and no way to reach it. Killing is what the user
    /// asked for by closing the panel.
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Which shell to launch.
///
/// `RENZORA_TERMINAL_SHELL` wins so a user can pick `cmd.exe`, `zsh`, `nu` or a
/// wrapper script without a setting to maintain. Otherwise: PowerShell on
/// Windows, and elsewhere the login shell the user actually chose, because
/// launching `/bin/bash` at someone whose prompt, aliases and completions all
/// live in `zsh` produces a terminal that is subtly not theirs.
fn shell() -> String {
    if let Some(explicit) = std::env::var_os("RENZORA_TERMINAL_SHELL") {
        if !explicit.is_empty() {
            return explicit.to_string_lossy().into_owned();
        }
    }
    #[cfg(windows)]
    {
        "powershell.exe".to_string()
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}
