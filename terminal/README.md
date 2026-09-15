# Terminal

A real shell in a dock panel. Not a log view that echoes commands: an operating-system pseudo-terminal with a shell on the far end, so `vim`, `htop`, `git add -p`, a dev server and `claude` all run full-screen inside the panel and behave exactly as they do in a standalone terminal.

**Open it:** the **Terminal** panel, under the "Tools" group in the panel list.

**Tabs:** `+` or Ctrl+Shift+T opens another shell, Ctrl+Shift+W closes one, Ctrl+Tab cycles, double-click a tab to rename it, and the strip can sit across the top or down the right. Each tab keeps 10,000 lines of its own scrollback, and a tab left in the background keeps running: a build started in one is still going when you come back to it.

**Shell:** PowerShell on Windows, `$SHELL` elsewhere. Set `RENZORA_TERMINAL_SHELL` to pick another one. It starts in the open project's directory.

**Copy and paste** are Ctrl+Shift+C and Ctrl+Shift+V, so plain Ctrl+C still sends SIGINT to whatever is running.

**Files:** `pty.rs` is the shell and its pseudo-terminal, `reply.rs` answers the status queries a shell blocks on, `keys.rs` turns a keystroke into the bytes a shell expects, `grid.rs` turns emulator cells into styled text runs, and `panel.rs` drives the lot.

**Scope:** Editor. It never loads in an exported game, and handing a player a shell in the game's own process is not a feature anyone asked for.
