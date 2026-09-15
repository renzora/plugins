//! The panel's state: a list of shells, and which one is on screen.
//!
//! One [`Tab`] is one shell with its own emulator, its own history and its own
//! selection. Nothing is shared between them except the grid size, because the
//! size comes from the panel and every tab is drawn in the same space.
//!
//! # Tabs are addressed by id, never by index
//!
//! Every place a tab is referred to from outside this module - the strip's
//! buttons, the close buttons, the active tab - uses [`Tab::id`], a serial that
//! is never reused. An index would be the obvious choice and is wrong here for a
//! specific reason: a click on tab 3's close button is read on a later frame than
//! the one that built it, and closing tab 1 in between shifts every index after
//! it. The button would then close a tab the user never pointed at. A serial
//! cannot go stale that way; at worst it names a tab that is already gone, and
//! the lookup returns `None`.

use crate::pty;

/// How many lines of history each shell keeps above the visible screen.
///
/// Per tab, and that is the cost worth knowing before raising it: ten open tabs
/// is ten of these. Worth it at this size - the first thing anyone does with a
/// long build log is scroll back through it, and a terminal that has thrown the
/// output away is a terminal you have to run the command in twice.
const SCROLLBACK: usize = 10_000;

/// One shell, with the emulator drawing it.
pub struct Tab {
    /// Never reused, so a click read a frame late cannot land on the wrong tab.
    pub id: u32,
    /// What the strip shows. The shell's own name plus the serial, because
    /// `vt100` does not track the OSC title sequence a shell would otherwise set
    /// per directory, and five tabs all called `bash` are five tabs you have to
    /// click to tell apart.
    pub title: String,
    /// True once the user has renamed this tab by hand.
    ///
    /// Without it, restarting a dead shell (Enter on the exit notice) would
    /// rename the tab back to the program, silently throwing away the name the
    /// user chose. An automatic name is a placeholder; a typed one is a
    /// decision.
    pub renamed: bool,
    /// `None` before the first launch, and again once the shell exits.
    pub session: Option<pty::Session>,
    /// Carries [`crate::reply::Replies`] so the emulator can answer a shell that
    /// asks it something. Without that the parser is write-only and PowerShell
    /// never gets past its opening cursor-position query.
    pub parser: vt100::Parser<crate::reply::Replies>,
    /// Rows scrolled up from the live bottom. `0` is the live screen.
    pub scrollback: usize,
    /// How far back this tab's history actually goes, which is what the
    /// scrollbar divides by. Refreshed by [`Tab::refresh_history`] rather than
    /// tracked incrementally: the emulator decides when a line scrolls off, and
    /// mirroring that decision here would be a second implementation of it.
    pub history: usize,
    /// Where a drag-selection started and where it is now, in visible cells.
    pub anchor: Option<(u16, u16)>,
    pub head: Option<(u16, u16)>,
    /// Set when a shell has been asked for and not yet started. Cleared by the
    /// attempt, whether or not it worked, so a failure is reported once rather
    /// than retried every frame.
    pub wanted: bool,
}

impl Tab {
    fn new(id: u32, rows: u16, cols: u16) -> Self {
        Self {
            id,
            // Renamed to the real program once it launches. Until then the strip
            // has to show something, and "Shell" is true of whatever starts.
            title: format!("Shell {id}"),
            renamed: false,
            session: None,
            // Floored, not merely non-zero: a 1x1 vt100 grid panics on its first
            // wrap. See the `MIN_ROWS` / `MIN_COLS` doc comments for why.
            parser: vt100::Parser::new_with_callbacks(
                rows.max(crate::panel::MIN_ROWS),
                cols.max(crate::panel::MIN_COLS),
                SCROLLBACK,
                crate::reply::Replies::default(),
            ),
            scrollback: 0,
            history: 0,
            anchor: None,
            head: None,
            wanted: true,
        }
    }

    /// Re-read how far back the history goes.
    ///
    /// `vt100` reports the *current* scrollback offset and clamps a requested one
    /// to the length it has, but exposes no length of its own. So the length is
    /// read by asking for an impossible offset and seeing what comes back, then
    /// putting the real one straight back.
    ///
    /// That is safe because `set_scrollback` assigns one integer and does nothing
    /// else - it moves no cells, allocates nothing, and cannot be observed
    /// between the two calls, which happen in the same system. The alternative,
    /// counting scrolled-off lines as output is parsed, means re-deriving when
    /// the emulator scrolls from the escape sequences it just handled.
    pub fn refresh_history(&mut self) {
        let current = self.parser.screen().scrollback();
        self.parser.screen_mut().set_scrollback(usize::MAX);
        self.history = self.parser.screen().scrollback();
        self.parser.screen_mut().set_scrollback(current);
    }

    /// The selection as `(start, end)` cell positions, top-left first.
    pub fn selection(&self) -> Option<((u16, u16), (u16, u16))> {
        let (a, b) = (self.anchor?, self.head?);
        Some(if a <= b { (a, b) } else { (b, a) })
    }
}

/// Where the tab strip sits.
///
/// Two positions rather than four. Top and Right are the two that change the
/// *shape* of the strip - a row of chips across the top, or a stacked list down
/// the side - and Bottom and Left would be the same two layouts mirrored, which
/// is a preference about which edge rather than about how tabs are shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StripSide {
    #[default]
    Top,
    Right,
}

impl StripSide {
    /// Whether chips are stacked down a column rather than laid across a row.
    pub fn vertical(self) -> bool {
        self == Self::Right
    }
}

/// Every shell the panel is running, and the one it is drawing.
#[derive(bevy::prelude::Resource)]
pub struct Terminal {
    tabs: Vec<Tab>,
    /// The id of the tab on screen. Kept valid by [`Self::close`], which is the
    /// only thing that can invalidate it.
    active: u32,
    next_id: u32,
    /// Bumped whenever the tab list, the active tab, the strip's side or the
    /// rename in progress changes, so the strip can rebuild on a change and do
    /// nothing on every other frame.
    pub strip_version: u32,
    /// Which edge the strip is on. Session-lived: a plugin has no slot in the
    /// editor's own settings file, and a terminal layout is cheap to re-pick.
    pub side: StripSide,
    /// The tab whose name is being edited, if any.
    pub renaming: Option<u32>,
    /// Whether the chips no longer fit the strip, which is what puts the
    /// overflow menu on screen. Written by the system that measures them.
    pub overflowing: bool,
    /// Whether keystrokes go to the shell. Also what decides whether the cursor
    /// is drawn solid, and whether the editor's own shortcuts are held off.
    pub focused: bool,
    pub rows: u16,
    pub cols: u16,
    /// Logical px per character cell, measured from the probe.
    pub advance: f32,
    pub line_h: f32,
    pub dragging: bool,
    /// True while the scrollbar thumb is being dragged. On `Terminal` rather than
    /// a `Local` on the drag system, because the system that *draws* the thumb
    /// has to know: a thumb that does not stay lit while you drag it reads as
    /// having been dropped.
    pub scroll_dragging: bool,
    /// Something changed that the drawn rows do not reflect yet.
    pub dirty: bool,
}

impl Default for Terminal {
    fn default() -> Self {
        let mut term = Self {
            tabs: Vec::new(),
            active: 1,
            next_id: 1,
            strip_version: 0,
            side: StripSide::Top,
            renaming: None,
            overflowing: false,
            focused: false,
            // Seeded at a conventional size. The first `resize` replaces it with
            // whatever the panel actually is, usually before the shell has
            // printed its first prompt.
            rows: 24,
            cols: 80,
            advance: 0.0,
            line_h: 0.0,
            dragging: false,
            scroll_dragging: false,
            dirty: true,
        };
        // The panel is never tabless: an empty terminal panel has no state worth
        // preserving and no obvious way back, so the first tab exists from the
        // start rather than waiting for someone to press `+`.
        term.open();
        term
    }
}

impl Terminal {
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn active_id(&self) -> u32 {
        self.active
    }

    pub fn active(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == self.active)
    }

    pub fn active_mut(&mut self) -> Option<&mut Tab> {
        let active = self.active;
        self.tabs.iter_mut().find(|t| t.id == active)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Tab> {
        self.tabs.iter_mut()
    }

    /// Open a new tab and switch to it. Returns its id.
    pub fn open(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab::new(id, self.rows, self.cols));
        self.activate(id);
        id
    }

    pub fn activate(&mut self, id: u32) {
        if self.active == id && self.tabs.iter().any(|t| t.id == id) {
            return;
        }
        if self.tabs.iter().any(|t| t.id == id) {
            self.active = id;
            self.strip_version += 1;
            self.dirty = true;
        }
    }

    /// Close a tab, killing its shell.
    ///
    /// Closing the last one opens a fresh tab in its place rather than leaving
    /// the panel empty. That is the same decision as seeding the first tab in
    /// `Default`, pointed at the other end: a terminal panel showing no terminal
    /// is a dead panel, and "close the only tab" reads perfectly well as "give me
    /// a clean shell".
    pub fn close(&mut self, id: u32) {
        let Some(index) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        // Dropping the `Tab` drops its `Session`, whose `Drop` kills the shell.
        self.tabs.remove(index);
        if self.renaming == Some(id) {
            // The field being edited belongs to a tab that no longer exists. Left
            // set, the strip would keep drawing a rename box on whichever tab
            // inherited nothing.
            self.renaming = None;
        }
        if self.tabs.is_empty() {
            self.open();
            return;
        }
        if self.active == id {
            // The neighbour to the left, or the new first tab. Stepping right
            // would land on a tab that has just shifted into the closed one's
            // place, which reads as the panel closing nothing at all.
            self.active = self.tabs[index.saturating_sub(1).min(self.tabs.len() - 1)].id;
        }
        self.strip_version += 1;
        self.dirty = true;
    }

    /// Move the strip to another edge.
    pub fn set_side(&mut self, side: StripSide) {
        if self.side == side {
            return;
        }
        self.side = side;
        // The chips are shaped differently on each edge (a row of pills, or a
        // stacked list filling the column's width), so this is a rebuild rather
        // than a restyle.
        self.strip_version += 1;
    }

    /// Begin renaming a tab, or stop.
    pub fn set_renaming(&mut self, id: Option<u32>) {
        if self.renaming == id {
            return;
        }
        self.renaming = id;
        // The chip swaps its label for a text field and back.
        self.strip_version += 1;
    }

    /// Apply a typed name. An empty one is a cancel, not a nameless tab.
    pub fn rename(&mut self, id: u32, title: &str) {
        let title = title.trim();
        if title.is_empty() {
            return;
        }
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            tab.title = title.to_string();
            tab.renamed = true;
            self.strip_version += 1;
        }
    }

    /// Close every tab but one.
    pub fn close_others(&mut self, keep: u32) {
        let doomed: Vec<u32> = self.tabs.iter().map(|t| t.id).filter(|id| *id != keep).collect();
        for id in doomed {
            self.close(id);
        }
        self.activate(keep);
    }

    /// Move to the next or previous tab, wrapping.
    pub fn cycle(&mut self, forward: bool) {
        if self.tabs.len() < 2 {
            return;
        }
        let Some(index) = self.tabs.iter().position(|t| t.id == self.active) else {
            return;
        };
        let len = self.tabs.len();
        let next = if forward { (index + 1) % len } else { (index + len - 1) % len };
        self.activate(self.tabs[next].id);
    }

    /// Send bytes to the active shell and return it to the live screen.
    ///
    /// Jumping to the bottom on input is the behaviour every terminal has: you
    /// scrolled up to read something, you start typing, and you expect to see
    /// what you are typing rather than the history you were looking at.
    pub fn send(&mut self, bytes: &[u8]) {
        let mut moved = false;
        if let Some(tab) = self.active_mut() {
            let Some(session) = &tab.session else {
                return;
            };
            session.write(bytes);
            if tab.scrollback != 0 {
                tab.scrollback = 0;
                tab.parser.screen_mut().set_scrollback(0);
                moved = true;
            }
        }
        self.dirty |= moved;
    }

    pub fn clear_selection(&mut self) {
        let mut cleared = false;
        if let Some(tab) = self.active_mut() {
            if tab.anchor.is_some() {
                tab.anchor = None;
                tab.head = None;
                cleared = true;
            }
        }
        self.dirty |= cleared;
    }
}
