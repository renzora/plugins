//! Turning one row of the emulator's cell grid into the runs the panel draws.
//!
//! `bevy_ui` draws a line of text as a `Text` with `TextSpan` children, and a
//! span carries its own `TextColor`, `TextBackgroundColor` and `Underline`. That
//! is the whole set a terminal cell needs, so a row is flattened into **runs**:
//! maximal stretches of cells sharing one style, one span each.
//!
//! Runs rather than cells because the difference is not marginal. A 200x50 panel
//! is 10,000 cells and almost all of a real screen is one style for tens of
//! columns at a time; run-length encoding turns that into a few hundred spans.
//! Per-cell entities were never worth measuring.
//!
//! Each row also returns a hash of everything it draws. The panel keeps last
//! frame's hashes and rewrites only the rows whose hash moved, which is what
//! makes a full-screen program that repaints one status line cost one row.
//!
//! The hash still has to be *computed* for every row, so a frame in which
//! anything changed reads the whole grid. That is the cost worth knowing about
//! here: `vt100` exposes cells and not rows, so `Screen::cell` resolves the row
//! again for every column. It is bounded (the panel only reads when something
//! moved, and an idle terminal repaints nothing at all) and it is the price of
//! not reimplementing the emulator's row storage on this side of its API.

use bevy::prelude::*;

/// A maximal stretch of cells sharing one style.
pub struct Run {
    pub text: String,
    pub fg: Color,
    /// `Color::NONE` for the default background, so the panel's own fill shows
    /// through rather than every cell painting a quad of the same colour.
    pub bg: Color,
    pub underline: bool,
    /// The same style as raw bytes. Kept because run merging and the row hash
    /// both compare style, and doing that on a `Color` means comparing floats
    /// that went through a colour-space conversion to get there.
    style: Style,
}

/// What the panel knows about this row that the emulator does not.
#[derive(Clone, Copy, Default)]
pub struct RowOverlay {
    /// Column of the block cursor, when it is on this row and visible.
    pub cursor: Option<u16>,
    /// Whether the cursor should be drawn solid. An unfocused terminal still
    /// shows where the cursor is, but must not look like it is taking input.
    pub cursor_solid: bool,
    /// `[start, end)` columns covered by the selection on this row.
    pub selection: Option<(u16, u16)>,
}

/// The resolved appearance of one cell, before it is merged into a run.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Style {
    fg: [u8; 3],
    bg: Option<[u8; 3]>,
    /// Kept even though nothing draws a heavier weight: the panel has one mono
    /// `FontSource` and no bold face to switch to. `bold` still changes what the
    /// cell looks like, because it promotes indexed colours 0-7 to their bright
    /// counterparts (see [`resolve`]) - which is the effect a shell prompt
    /// asking for "bold red" is actually after. It stays in the style key so two
    /// cells that differ only in boldness do not merge into one run and lose
    /// that promotion.
    bold: bool,
    underline: bool,
}

/// Flatten `row` into runs, and hash what they draw.
///
/// `default_fg` / `default_bg` come from the editor theme, so a terminal in a
/// light theme is dark-on-light without the shell having to be told.
pub fn row_runs(
    screen: &vt100::Screen,
    row: u16,
    cols: u16,
    overlay: RowOverlay,
    default_fg: [u8; 3],
    default_bg: [u8; 3],
    selection_bg: [u8; 3],
) -> (Vec<Run>, u64) {
    let mut runs: Vec<Run> = Vec::new();
    let mut hash = FNV_OFFSET;
    let mut col = 0u16;

    while col < cols {
        let cell = screen.cell(row, col);
        // A wide glyph's second column is not a cell of its own: it carries no
        // contents and must not emit a space, or every CJK character would push
        // the rest of the line one column right.
        if cell.is_some_and(|c| c.is_wide_continuation()) {
            col += 1;
            continue;
        }
        let width = if cell.is_some_and(|c| c.is_wide()) { 2 } else { 1 };
        let text = match cell {
            Some(c) if c.has_contents() => c.contents().to_string(),
            // Blank cells still have to be emitted: a run of spaces is what
            // holds the columns after it in place, and a blank cell can carry a
            // background colour of its own.
            _ => " ".repeat(width as usize),
        };
        let style = cell_style(
            cell,
            col,
            overlay,
            default_fg,
            default_bg,
            selection_bg,
        );

        match runs.last_mut() {
            Some(last) if last.style == style => last.text.push_str(&text),
            _ => runs.push(Run {
                text,
                fg: rgb(style.fg),
                bg: style.bg.map(rgb).unwrap_or(Color::NONE),
                underline: style.underline,
                style,
            }),
        }
        col += width;
    }

    // Trailing default-styled blanks draw nothing, and a shell pads almost every
    // line with them. Dropping them here is what keeps an idle prompt at two or
    // three spans instead of one per style change across the full width.
    while runs
        .last()
        .is_some_and(|r| r.style.bg.is_none() && r.text.bytes().all(|b| b == b' '))
    {
        runs.pop();
    }

    for run in &runs {
        hash = fnv(hash, run.text.as_bytes());
        hash = fnv(hash, &run.style.fg);
        hash = fnv(hash, &run.style.bg.unwrap_or([0, 0, 0]));
        hash = fnv(
            hash,
            &[
                u8::from(run.style.bg.is_some()),
                u8::from(run.style.bold),
                u8::from(run.style.underline),
            ],
        );
    }
    (runs, hash)
}

/// Resolve one cell's colours, applying inverse, dim, the selection and the
/// cursor in that order.
fn cell_style(
    cell: Option<&vt100::Cell>,
    col: u16,
    overlay: RowOverlay,
    default_fg: [u8; 3],
    default_bg: [u8; 3],
    selection_bg: [u8; 3],
) -> Style {
    let bold = cell.is_some_and(|c| c.bold());
    let mut fg = cell
        .map(|c| resolve(c.fgcolor(), default_fg, bold))
        .unwrap_or(default_fg);
    let mut bg = cell.and_then(|c| match c.bgcolor() {
        vt100::Color::Default => None,
        other => Some(resolve(other, default_bg, false)),
    });

    // `inverse` is the attribute a program sets for a status bar or a highlighted
    // menu entry. Swapping resolved colours (rather than the raw attributes) is
    // what makes it work when only one side was set explicitly: the default has
    // to become a real colour before it can be swapped into the other slot.
    if cell.is_some_and(|c| c.inverse()) {
        let new_bg = fg;
        fg = bg.unwrap_or(default_bg);
        bg = Some(new_bg);
    }
    if cell.is_some_and(|c| c.dim()) {
        fg = mix(fg, bg.unwrap_or(default_bg), 0.45);
    }

    if overlay
        .selection
        .is_some_and(|(start, end)| col >= start && col < end)
    {
        // The selection paints the background and leaves the text colour alone,
        // so syntax-coloured output stays readable while it is selected.
        bg = Some(selection_bg);
    }

    if overlay.cursor == Some(col) {
        if overlay.cursor_solid {
            let block = fg;
            fg = bg.unwrap_or(default_bg);
            bg = Some(block);
        } else {
            // Unfocused: a dim block. Enough to find the cursor, not enough to
            // look like the panel is taking keystrokes.
            bg = Some(mix(bg.unwrap_or(default_bg), fg, 0.35));
        }
    }

    Style { fg, bg, bold, underline: cell.is_some_and(|c| c.underline()) }
}

/// One vt100 colour as RGB.
///
/// `bold` promotes an indexed colour 0-7 to its bright counterpart. That is the
/// original meaning of the bold attribute on a hardware terminal, and shells
/// still rely on it: a prompt that asks for "bold red" is asking for index 1 +
/// bold and expects bright red, not a heavier weight of dark red.
fn resolve(color: vt100::Color, default: [u8; 3], bold: bool) -> [u8; 3] {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => [r, g, b],
        vt100::Color::Idx(i) => indexed(if bold && i < 8 { i + 8 } else { i }),
    }
}

/// The 256-colour palette.
///
/// 0-15 are a table, because those are the colours a shell prompt and `ls`
/// actually use and xterm's originals (notably `0,0,238` for blue) are close to
/// unreadable on a dark background. This is the palette VS Code's terminal uses.
/// 16-231 are the 6x6x6 cube and 232-255 the greyscale ramp, both of which are
/// defined by formula and have no room for taste.
fn indexed(i: u8) -> [u8; 3] {
    const BASE: [[u8; 3]; 16] = [
        [0, 0, 0],
        [205, 49, 49],
        [13, 188, 121],
        [229, 229, 16],
        [36, 114, 200],
        [188, 63, 188],
        [17, 168, 205],
        [229, 229, 229],
        [102, 102, 102],
        [241, 76, 76],
        [35, 209, 139],
        [245, 245, 67],
        [59, 142, 234],
        [214, 112, 214],
        [41, 184, 219],
        [255, 255, 255],
    ];
    match i {
        0..=15 => BASE[i as usize],
        16..=231 => {
            let i = i - 16;
            // Not a linear ramp: the first step is 0 and the rest start at 55,
            // which is what xterm does and what every palette generator assumes.
            let step = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            [step(i / 36), step((i / 6) % 6), step(i % 6)]
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            [v, v, v]
        }
    }
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    [lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2])]
}

fn rgb(c: [u8; 3]) -> Color {
    Color::srgb_u8(c[0], c[1], c[2])
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

fn fnv(mut hash: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
