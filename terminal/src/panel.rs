//! The dock panel: the grid of text, and everything that drives it.
//!
//! The panel is a fixed number of row entities, each a `Text` whose `TextSpan`
//! children are the styled runs of one terminal line. It is built once and then
//! mutated in place - never rebuilt - which is the same rule every ember panel
//! follows, and matters more here than usual: a program like `htop` repaints
//! the whole screen every second, and despawning a thousand entities that often
//! is visible in the frame time.
//!
//! The rows draw **one** tab, the active one. Every other tab keeps running and
//! keeps its own history (see [`crate::state`]); switching is a repaint, not a
//! rebuild, because every tab is the same size and therefore the same rows.
//!
//! Two measurements make the rest work.
//!
//! **The character advance** comes from a hidden probe: a laid-out run of twenty
//! `M`s whose width `bevy_text` reports. It cannot be assumed from the font size,
//! because the size is snapped to the theme's ladder and scaled by the user's UI
//! scale before it reaches the layout, and a terminal that thinks a cell is one
//! pixel wider than it is drifts a whole column off by the right-hand edge.
//!
//! **The grid size** comes from the panel's own `ComputedNode`. Rows and columns
//! are whatever fits, and a change is pushed both into the emulator and into the
//! kernel, which is what raises `SIGWINCH` and makes a full-screen program
//! re-flow.

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy::text::{TextBackgroundColor, TextLayoutInfo, Underline};
use bevy::ui::{ComputedNode, FocusPolicy, RelativeCursorPosition};

use renzora_ember::font::{ui_font, EmberFonts};
use renzora_ember::theme::{border, panel_bg, rgb, selection, text_muted, text_primary};
use renzora_ember::widgets::clipboard;

use crate::grid::{self, RowOverlay};
use crate::keys::{self, Mods};
use crate::pty;
use crate::state::Terminal;
use crate::strip;

/// The dock panel id. Must match on both registrations - the shell's metadata
/// and ember's content builder - or the tab has no content and the content has
/// no tab.
pub const PANEL_ID: &str = "terminal";

/// Authored font size, before the theme's UI scale and size ladder are applied.
const FONT_PX: f32 = 11.0;
/// Inset around the grid, in logical px.
const PAD: f32 = 6.0;
/// The probe string and its length. Twenty characters rather than one, so the
/// measurement averages away the sub-pixel rounding of a single advance.
const PROBE: &str = "MMMMMMMMMMMMMMMMMMMM";
const PROBE_LEN: f32 = 20.0;

/// Width of the scrollbar gutter, in logical px.
///
/// Always reserved, even with no history to scroll. The alternative - hiding the
/// gutter when it is not needed, the way the code editor does - would change the
/// usable width, and the usable width is the terminal's **column count**: the
/// first line of output would resize the shell, and a program mid-redraw would
/// be repainting for a width that no longer exists.
const SCROLLBAR_W: f32 = 8.0;
/// Floor on the thumb so a 10,000-line history still leaves something to grab.
const MIN_THUMB: f32 = 18.0;

/// Ceilings on the derived grid size.
///
/// Not defensive padding: `set_size` allocates rows x cols cells, per tab, and a
/// panel dragged to a full 4K width at a small font is a genuinely large grid.
/// The caps bound that, and are far beyond any size a person reads text at.
const MAX_COLS: u16 = 500;
const MAX_ROWS: u16 = 300;

/// Floor on the row count, and it is a **correctness** bound, not a taste one.
///
/// vt100 sets `scroll_bottom = rows - 1`, so a one-row grid has a scroll region
/// whose bottom is row 0. Its `col_wrap` then reads:
///
/// ```text
/// let mut prev_pos = self.pos;   // row 0
/// let scrolled = self.row_inc_scroll(1);  // clamps back to 0, reports 1 line
/// prev_pos.row -= scrolled;      // 0 - 1
/// self.drawing_row_mut(prev_pos.row).unwrap()
/// ```
///
/// With overflow checks off that subtraction wraps to 65535, the lookup misses,
/// and the `unwrap` (commented "we assume self.pos.row is always valid") panics
/// on the compute pool and takes the editor down. So a one-row terminal dies on
/// the first line long enough to wrap, which is essentially the first line.
///
/// Two rows is the smallest height with a non-degenerate scroll region. Nothing
/// is readable at that size either, but it does not crash, and squeezing the
/// panel is not something a person should have to be careful about.
pub(crate) const MIN_ROWS: u16 = 2;
/// Same shape of hazard on the other axis: `col_wrap` compares against
/// `self.size.cols - width`, and a double-width glyph in a one-column grid
/// underflows that too.
pub(crate) const MIN_COLS: u16 = 2;

/// The entities the panel drew itself out of.
#[derive(Resource)]
pub(crate) struct View {
    grid: Entity,
    rows: Vec<RowView>,
    /// The row height the rows were built at, so a font-scale change rebuilds
    /// them rather than leaving the text overflowing its row.
    line_h: f32,
}

struct RowView {
    root: Entity,
    text: Entity,
    /// Span entities, reused across frames. The pool only grows: emptying a
    /// span costs one component write, despawning and respawning it costs an
    /// archetype move and a hierarchy edit.
    spans: Vec<Entity>,
    /// Hash of what this row last drew. `None` forces a repaint.
    hash: Option<u64>,
}

// `pub(crate)` only because they appear in the signatures of the systems
// `lib.rs` registers, and a private type in a `pub fn`'s query is an error.
#[derive(Component)]
pub(crate) struct TermRoot;
/// The rows themselves: the node whose size decides how many rows and columns
/// there are, and the one the pointer is mapped into for selection. Not the
/// padded wrapper around it, and not the scrollbar gutter beside it.
#[derive(Component)]
pub(crate) struct TermGrid;
#[derive(Component)]
pub(crate) struct TermProbe;
#[derive(Component)]
pub(crate) struct TermScrollTrack;
#[derive(Component)]
pub(crate) struct TermScrollThumb;

// ── Build ───────────────────────────────────────────────────────────────────

pub fn build(commands: &mut Commands, fonts: &EmberFonts) -> Entity {
    let root = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                // Column here, but `strip::sync` flips it to Row when the tabs
                // move to the side. Stretch is stated rather than left to the
                // default, because it is what makes the grid fill the cross axis
                // in *both* directions once its own size stops naming one.
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(rgb(panel_bg())),
            // `Interaction` is what tells the panel it was clicked, and
            // `FocusPolicy::Block` stops the click continuing through to the
            // viewport behind the dock - which would otherwise read a click in
            // the terminal as a click in the scene and change the selection.
            Interaction::default(),
            FocusPolicy::Block,
            TermRoot,
            Name::new("terminal-root"),
        ))
        .id();

    let strip = strip::build(commands);

    // The padded area holding the rows and the scrollbar side by side. It carries
    // the padding so that neither of them has to, and so the rows' own box is
    // exactly the region the text occupies - which is what makes the row/column
    // count and the pointer-to-cell mapping fall out of one measurement.
    let body = commands
        .spawn((
            Node {
                // Sized purely by flex, in both directions: `grow: 1` with a
                // zero basis takes whatever the strip leaves along the main
                // axis, and the root's `Stretch` fills the cross axis. Naming a
                // `width: 100%` here instead works only while the root is a
                // column - the moment the tabs move to the side, that 100% is
                // the *main* axis and the body pushes the strip off the panel.
                flex_grow: 1.0,
                flex_basis: Val::Px(0.0),
                // A flex item's automatic minimum size is its content, and this
                // one's content is a stack of full-width rows. Without these it
                // refuses to shrink below them and overflows the panel.
                min_width: Val::Px(0.0),
                min_height: Val::Px(0.0),
                flex_direction: FlexDirection::Row,
                padding: UiRect::all(Val::Px(PAD)),
                overflow: Overflow::clip(),
                ..default()
            },
            FocusPolicy::Pass,
            Name::new("terminal-body"),
        ))
        .id();

    let grid = commands
        .spawn((
            Node {
                flex_grow: 1.0,
                flex_basis: Val::Px(0.0),
                min_width: Val::Px(0.0),
                min_height: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                overflow: Overflow::clip(),
                ..default()
            },
            // Drag-selection needs the pointer in the rows' own coordinates,
            // and this is the component that reports them without the panel
            // having to know where the dock put it.
            RelativeCursorPosition::default(),
            FocusPolicy::Pass,
            TermGrid,
            Name::new("terminal-grid"),
        ))
        .id();

    let track = commands
        .spawn((
            Node {
                width: Val::Px(SCROLLBAR_W),
                height: Val::Percent(100.0),
                flex_shrink: 0.0,
                ..default()
            },
            // The pointer's position along the track, which is the whole of the
            // drag: a press anywhere on it is a jump to that fraction of the
            // history, and holding is the same thing every frame.
            RelativeCursorPosition::default(),
            FocusPolicy::Block,
            TermScrollTrack,
            Name::new("terminal-scrollbar"),
        ))
        .id();
    let thumb = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(1.0),
                right: Val::Px(1.0),
                top: Val::Px(0.0),
                height: Val::Px(0.0),
                border_radius: BorderRadius::all(Val::Px(3.0)),
                // Hidden until there is history; `scrollbar_sync` sizes and shows
                // it. A full-height thumb that can never move is decoration.
                display: Display::None,
                ..default()
            },
            BackgroundColor(rgb(border())),
            FocusPolicy::Pass,
            TermScrollThumb,
            Name::new("terminal-scrollbar-thumb"),
        ))
        .id();
    commands.entity(track).add_child(thumb);
    commands.entity(body).add_children(&[grid, track]);

    // Laid out, so `bevy_text` measures it, but not drawn. `measure` reads its
    // width to derive the real character advance.
    let probe = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                ..default()
            },
            Text::new(PROBE),
            ui_font(&fonts.mono, FONT_PX),
            TextLayout::no_wrap(),
            Visibility::Hidden,
            FocusPolicy::Pass,
            TermProbe,
            Name::new("terminal-probe"),
        ))
        .id();

    commands.entity(root).add_children(&[strip, body, probe]);
    commands.insert_resource(View { grid, rows: Vec::new(), line_h: 0.0 });
    root
}

// ── Metrics and size ────────────────────────────────────────────────────────

/// Derive the character advance and row height from the probe.
pub fn measure(mut term: ResMut<Terminal>, probes: Query<&TextLayoutInfo, With<TermProbe>>) {
    let Ok(info) = probes.single() else {
        return;
    };
    if info.size.x <= 0.0 || info.size.y <= 0.0 {
        return;
    }
    // `TextLayoutInfo::size` is physical px; everything else here is logical.
    let sf = if info.scale_factor > 0.0 { info.scale_factor } else { 1.0 };
    let advance = info.size.x / PROBE_LEN / sf;
    let line_h = (info.size.y / sf).max(1.0);
    if (advance - term.advance).abs() > 0.01 || (line_h - term.line_h).abs() > 0.01 {
        term.advance = advance;
        term.line_h = line_h;
        term.dirty = true;
    }
}

/// Fit the grid to the panel, and start each tab's shell once there is a size to
/// start it at.
pub fn resize(
    mut commands: Commands,
    mut term: ResMut<Terminal>,
    // Optional, and every system here takes them the same way: these systems are
    // gated on the panel being the active tab, and on the frame it becomes
    // active they can run before ember's builder has inserted `View`. Requiring
    // it would be a panic on the frame the panel opens.
    view: Option<ResMut<View>>,
    fonts: Option<Res<EmberFonts>>,
    grids: Query<(Entity, &ComputedNode), With<TermGrid>>,
    project: Option<Res<renzora::CurrentProject>>,
) {
    let (Some(mut view), Some(fonts)) = (view, fonts) else {
        return;
    };
    if term.advance <= 0.0 || term.line_h <= 0.0 {
        return;
    }
    let Ok((grid_entity, computed)) = grids.single() else {
        return;
    };
    // No padding or scrollbar arithmetic: the rows' own box already excludes
    // both, because the padding is on the wrapper and the gutter is a sibling.
    let size = computed.size() * computed.inverse_scale_factor();
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }

    let cols = ((size.x / term.advance).floor() as u16).clamp(MIN_COLS, MAX_COLS);
    let rows = ((size.y / term.line_h).floor() as u16).clamp(MIN_ROWS, MAX_ROWS);
    let line_h = term.line_h;

    // The row entities are rebuilt only when their number or their height moved.
    // A width-only change (the common one, dragging a vertical split) keeps every
    // row entity and just repaints.
    //
    // `view.grid != grid_entity` is the third case and the least obvious: closing
    // the panel despawns its whole tree while `View` (a resource) survives, so
    // reopening it leaves the rows pointing at entities that no longer exist. The
    // grid entity from the query is always the live one, so comparing against it
    // is how a stale `View` is noticed. Without it the reopened panel is blank,
    // because every write goes to a dead entity and `try_insert` is silent about
    // it - which is exactly what you want from `try_insert` and exactly why the
    // staleness has to be caught here instead.
    let rebuild = rows as usize != view.rows.len()
        || (line_h - view.line_h).abs() > 0.01
        || view.grid != grid_entity;
    let resized = rows != term.rows || cols != term.cols;

    if rebuild {
        for row in view.rows.drain(..) {
            commands.entity(row.root).try_despawn();
        }
        view.grid = grid_entity;
        for index in 0..rows {
            view.rows.push(spawn_row(&mut commands, &fonts, grid_entity, index, line_h));
        }
        view.line_h = line_h;
    }

    if resized {
        term.rows = rows;
        term.cols = cols;
        // Every tab, not just the visible one. A background tab that kept its
        // old width would re-flow the moment it came forward, which is the shell
        // repainting a screen it drew for a size it no longer has: the last
        // command's output ends up wrapped wrongly and stays that way.
        for tab in term.iter_mut() {
            tab.parser.screen_mut().set_size(rows, cols);
            if let Some(session) = &tab.session {
                session.resize(rows, cols);
            }
            // A resize moves every cell. Nothing that survived is still valid.
            tab.anchor = None;
            tab.head = None;
            // Re-wrapping at a new width changes how many lines the history is,
            // so the scrollbar's divisor moves with it.
            tab.refresh_history();
        }
    }

    if resized || rebuild {
        for row in &mut view.rows {
            row.hash = None;
        }
        term.dirty = true;
    }

    start_pending_shells(&mut term, project.as_deref(), rows, cols);
}

/// Launch a shell for every tab that has asked for one and does not have one.
///
/// Driven from `resize` rather than from wherever a tab is opened, because a tab
/// cannot be started until the grid size is known: a shell launched at the
/// default 80x24 and resized a frame later has already drawn its prompt at the
/// wrong width.
fn start_pending_shells(
    term: &mut Terminal,
    project: Option<&renzora::CurrentProject>,
    rows: u16,
    cols: u16,
) {
    // The project directory, when there is one: a terminal in a game editor is
    // overwhelmingly used to run something against the project, and the editor's
    // own working directory is wherever the launcher happened to start it.
    let cwd = project.map(|p| p.path.clone());
    let mut started = false;
    for tab in term.iter_mut() {
        if !tab.wanted || tab.session.is_some() {
            continue;
        }
        tab.wanted = false;
        match pty::Session::spawn(rows, cols, cwd.as_deref()) {
            Ok(session) => {
                info!("[terminal] started {}", session.program);
                // Now that there is a real program, name the tab after it. The
                // basename alone: a `$SHELL` of `/usr/bin/zsh` in a 60px chip
                // would otherwise show the path and none of the name.
                tab.title = format!("{} {}", basename(&session.program), tab.id);
                tab.session = Some(session);
            }
            Err(err) => {
                // Reported through the emulator rather than as a panel state of
                // its own: it is the same red text in the same grid, and the
                // panel needs no second way to say something.
                tab.parser.process(format!("\r\n\x1b[31m{err}\x1b[0m\r\n").as_bytes());
                error!("[terminal] {err}");
            }
        }
        started = true;
    }
    if started {
        term.strip_version += 1;
        term.dirty = true;
    }
}

fn basename(program: &str) -> &str {
    program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .trim_end_matches(".exe")
}

fn spawn_row(
    commands: &mut Commands,
    fonts: &EmberFonts,
    grid: Entity,
    index: u16,
    line_h: f32,
) -> RowView {
    let root = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(line_h),
                flex_shrink: 0.0,
                overflow: Overflow::clip(),
                ..default()
            },
            FocusPolicy::Pass,
            Name::new(format!("terminal-row-{index}")),
        ))
        .id();
    let text = commands
        .spawn((
            Text::new(""),
            ui_font(&fonts.mono, FONT_PX),
            TextColor(rgb(text_primary())),
            TextLayout::no_wrap(),
            FocusPolicy::Pass,
        ))
        .id();
    commands.entity(root).add_child(text);
    commands.entity(grid).add_child(root);
    RowView { root, text, spans: Vec::new(), hash: None }
}

// ── The shells' output ──────────────────────────────────────────────────────

/// Feed whatever each shell wrote into its emulator.
///
/// Every tab, and registered to run even while the panel is hidden. Both are
/// deliberate: a build or a `claude` session left running in a background tab has
/// to still be there when you come back to it, and a pty that nobody reads fills
/// its kernel buffer and blocks the program writing to it. Draining only the
/// visible tab would not background a command, it would suspend it.
pub fn pump(mut term: ResMut<Terminal>) {
    let mut dirty = false;
    let mut ended = false;
    for tab in term.iter_mut() {
        let Some((bytes, alive)) = tab.session.as_ref().map(|s| (s.drain(), s.is_alive())) else {
            continue;
        };
        if !bytes.is_empty() {
            tab.parser.process(&bytes);
            // Anything in that chunk that was a question rather than output is
            // now sitting in the emulator's reply buffer, and the shell is
            // blocked until it is written back. Immediately, not next frame: a
            // shell that has stopped to wait produces nothing further, so a
            // reply deferred is a reply the panel has no later reason to send.
            let answer = tab.parser.callbacks_mut().take();
            if !answer.is_empty() {
                if let Some(session) = &tab.session {
                    session.write(&answer);
                }
            }
            // Output is the only thing that pushes lines into the history, so
            // this is where its length can have moved.
            tab.refresh_history();
            dirty = true;
        }
        if !alive {
            // One last drain: the reader thread appends and only then reports
            // EOF, so the final chunk can land between the two reads above.
            // Without this a command's last line of output is lost exactly when
            // it matters - the error that killed the shell.
            if let Some(session) = tab.session.take() {
                let tail = session.drain();
                if !tail.is_empty() {
                    tab.parser.process(&tail);
                }
            }
            tab.parser
                .process(b"\r\n\x1b[90m[process exited - press Enter for a new shell]\x1b[0m\r\n");
            dirty = true;
            ended = true;
        }
    }
    term.dirty |= dirty;
    if ended {
        // The strip shows the program's name, and there is no longer one.
        term.strip_version += 1;
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn render(
    mut commands: Commands,
    mut term: ResMut<Terminal>,
    view: Option<ResMut<View>>,
    fonts: Option<Res<EmberFonts>>,
) {
    let (Some(mut view), Some(fonts)) = (view, fonts) else {
        return;
    };
    if !term.dirty || view.rows.is_empty() {
        return;
    }

    let default_fg = triple(text_primary());
    let default_bg = triple(panel_bg());
    let selection_bg = triple(selection());
    let focused = term.focused;
    let cols = term.cols;

    {
        let Some(tab) = term.active() else {
            return;
        };
        let sel = tab.selection();
        let scrolled = tab.scrollback > 0;
        let screen = tab.parser.screen();
        let (cursor_row, cursor_col) = screen.cursor_position();
        // No cursor while scrolled back: it belongs to the live screen, and
        // drawing it over history claims the user can type there.
        let show_cursor = !screen.hide_cursor() && !scrolled;

        for (index, row) in view.rows.iter_mut().enumerate() {
            let index = index as u16;
            let overlay = RowOverlay {
                cursor: (show_cursor && cursor_row == index).then_some(cursor_col),
                cursor_solid: focused,
                selection: sel.and_then(|(start, end)| row_selection(start, end, index, cols)),
            };
            let (runs, hash) =
                grid::row_runs(screen, index, cols, overlay, default_fg, default_bg, selection_bg);
            if row.hash == Some(hash) {
                continue;
            }
            row.hash = Some(hash);

            for (i, run) in runs.iter().enumerate() {
                let span = match row.spans.get(i) {
                    Some(span) => *span,
                    None => {
                        let span = commands
                            .spawn((
                                TextSpan::new(String::new()),
                                ui_font(&fonts.mono, FONT_PX),
                                TextColor(run.fg),
                            ))
                            .id();
                        commands.entity(row.text).add_child(span);
                        row.spans.push(span);
                        span
                    }
                };
                let mut span = commands.entity(span);
                span.try_insert((
                    TextSpan::new(run.text.clone()),
                    TextColor(run.fg),
                    TextBackgroundColor(run.bg),
                ));
                if run.underline {
                    span.try_insert(Underline);
                } else {
                    span.remove::<Underline>();
                }
            }
            // Spans past the end of this row's runs are emptied rather than
            // despawned, so a screen that alternates between a wide line and a
            // narrow one does no hierarchy work at all.
            for span in row.spans.iter().skip(runs.len()) {
                commands
                    .entity(*span)
                    .try_insert((TextSpan::new(String::new()), TextBackgroundColor(Color::NONE)))
                    .remove::<Underline>();
            }
        }
    }

    term.dirty = false;
}

/// The `[start, end)` columns of `row` covered by a selection running from
/// `start` to `end`, or `None` when the row is outside it.
fn row_selection(start: (u16, u16), end: (u16, u16), row: u16, cols: u16) -> Option<(u16, u16)> {
    if row < start.0 || row > end.0 {
        return None;
    }
    // A selection is by line, not a rectangle: a middle row is selected edge to
    // edge, and only the first and last are cut by the drag's own columns. That
    // is what makes dragging over a wrapped command select the command.
    let from = if row == start.0 { start.1 } else { 0 };
    // Inclusive of the cell under the pointer, which is what makes selecting a
    // single character possible at all.
    let to = if row == end.0 { (end.1 + 1).min(cols) } else { cols };
    (from < to).then_some((from, to))
}

fn triple((r, g, b): (u8, u8, u8)) -> [u8; 3] {
    [r, g, b]
}

// ── Focus ───────────────────────────────────────────────────────────────────

/// Take keyboard focus on a click inside the panel, and give it up on a click
/// anywhere else.
pub fn focus(
    mut term: ResMut<Terminal>,
    mouse: Res<ButtonInput<MouseButton>>,
    roots: Query<&Interaction, With<TermRoot>>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    // Anywhere in the panel, the strip included: clicking a tab should leave you
    // able to type into it without a second click on the grid.
    let inside = roots.single().is_ok_and(|i| *i != Interaction::None);
    if term.focused != inside {
        term.focused = inside;
        // The cursor changes appearance with focus, so the row it is on has to
        // be repainted.
        term.dirty = true;
    }
}

/// Hold the editor's own keyboard shortcuts off while the terminal has focus.
///
/// Raised every frame rather than latched: `plugin_wants_keyboard` is cleared by
/// the editor once it has been read, so a plugin that stops running - or a panel
/// that is no longer the active tab, which stops these systems - releases the
/// keyboard without having to remember to.
pub fn claim_keyboard(term: Res<Terminal>, focus: Option<ResMut<renzora::InputFocusState>>) {
    if !term.focused {
        return;
    }
    if let Some(mut focus) = focus {
        focus.plugin_wants_keyboard = true;
    }
}

// ── Input ───────────────────────────────────────────────────────────────────

pub fn keyboard(
    mut term: ResMut<Terminal>,
    mut events: MessageReader<KeyboardInput>,
    held: Res<ButtonInput<KeyCode>>,
) {
    // A rename in progress owns the keyboard: the chip is showing an ember text
    // field, which reads the same `KeyboardInput` stream from its own cursor. Not
    // gating here does not split the keys between them - it delivers every one to
    // both, so typing a tab's new name also types it into the shell.
    if !term.focused || term.renaming.is_some() {
        // The events still have to be drained, or they arrive in a burst the
        // moment the panel is focused.
        events.clear();
        return;
    }
    let mods = Mods {
        ctrl: held.pressed(KeyCode::ControlLeft) || held.pressed(KeyCode::ControlRight),
        alt: held.pressed(KeyCode::AltLeft) || held.pressed(KeyCode::AltRight),
        shift: held.pressed(KeyCode::ShiftLeft) || held.pressed(KeyCode::ShiftRight),
    };

    for event in events.read() {
        if event.state != ButtonState::Pressed {
            continue;
        }

        // Ctrl+Shift is the terminal's own modifier, for the same reason it is
        // in every other terminal: plain Ctrl belongs to the program running
        // inside, and a terminal where Ctrl+C copies is one you cannot stop a
        // runaway command in.
        if mods.ctrl && mods.shift {
            // Tab is a named key, not a character, so it is matched separately.
            if event.logical_key == Key::Tab {
                term.cycle(false);
                continue;
            }
            if let Key::Character(c) = &event.logical_key {
                match c.to_ascii_lowercase().as_str() {
                    "c" => {
                        copy_selection(&term);
                        continue;
                    }
                    "v" => {
                        paste(&mut term);
                        continue;
                    }
                    "t" => {
                        term.open();
                        continue;
                    }
                    "w" => {
                        let active = term.active_id();
                        term.close(active);
                        continue;
                    }
                    _ => {}
                }
            }
        }
        if mods.ctrl && !mods.shift && event.logical_key == Key::Tab {
            term.cycle(true);
            continue;
        }

        // With no shell running the tab is showing the exit notice, and the only
        // key that means anything is the one that starts another.
        if term.active().is_some_and(|tab| tab.session.is_none()) {
            if event.logical_key == Key::Enter {
                if let Some(tab) = term.active_mut() {
                    tab.wanted = true;
                }
            }
            continue;
        }

        let app_cursor = term
            .active()
            .is_some_and(|tab| tab.parser.screen().application_cursor());
        if let Some(bytes) = keys::encode(event, mods, app_cursor) {
            term.send(&bytes);
            // Typing replaces the selection's meaning: what is highlighted is no
            // longer what the next command will act on.
            term.clear_selection();
        }
    }
}

fn copy_selection(term: &Terminal) {
    let Some(tab) = term.active() else {
        return;
    };
    let Some((start, end)) = tab.selection() else {
        return;
    };
    // `contents_between` is inclusive of the start cell and exclusive of the
    // end, and the drag's end cell is the one under the pointer - so it is
    // pushed one column right to include it.
    let text = tab
        .parser
        .screen()
        .contents_between(start.0, start.1, end.0, (end.1 + 1).min(term.cols));
    if !text.is_empty() {
        clipboard::set_text(&text);
    }
}

fn paste(term: &mut Terminal) {
    let Some(text) = clipboard::get_text() else {
        return;
    };
    if text.is_empty() {
        return;
    }
    // Bracketed paste, when the program asked for it: it wraps the text in
    // markers so an editor can tell pasted text from typed text and skip
    // auto-indent. Without the check, the markers would be typed literally into
    // a shell that never enabled the mode.
    let bracketed = term
        .active()
        .is_some_and(|tab| tab.parser.screen().bracketed_paste());
    let mut bytes = Vec::with_capacity(text.len() + 12);
    if bracketed {
        bytes.extend_from_slice(b"\x1b[200~");
    }
    // A pasted newline is a carriage return on the wire, the same as pressing
    // Enter. Sending `\n` instead runs the line in some shells and does nothing
    // in others.
    bytes.extend(text.bytes().map(|b| if b == b'\n' { b'\r' } else { b }));
    if bracketed {
        bytes.extend_from_slice(b"\x1b[201~");
    }
    term.send(&bytes);
}

/// Scroll back through the active tab's history with the wheel.
pub fn wheel(
    mut term: ResMut<Terminal>,
    mut events: MessageReader<MouseWheel>,
    roots: Query<&Interaction, With<TermRoot>>,
    strips: Query<&RelativeCursorPosition, With<crate::strip::TermStrip>>,
) {
    let hovered = roots.single().is_ok_and(|i| *i != Interaction::None);
    // The strip is inside the panel, so the root reports it as hovered too. The
    // wheel there belongs to the tab list, not to the shell's history: a side
    // strip's scroll view reads the same events, and scrolling both at once
    // moves two things the user was not looking at.
    let over_strip = strips.single().is_ok_and(|pointer| pointer.cursor_over);
    if !hovered || over_strip {
        events.clear();
        return;
    }
    let line_h = term.line_h.max(1.0);
    let mut delta = 0.0f32;
    for event in events.read() {
        delta += match event.unit {
            MouseScrollUnit::Line => event.y * 3.0,
            MouseScrollUnit::Pixel => event.y / line_h,
        };
    }
    if delta.abs() < 0.5 {
        return;
    }
    let mut moved = false;
    if let Some(tab) = term.active_mut() {
        let wanted = (tab.scrollback as f32 + delta).max(0.0) as usize;
        tab.parser.screen_mut().set_scrollback(wanted);
        // Read the value back rather than keeping the request: the emulator
        // clamps to how much history there actually is, and a local copy that
        // drifted above it would need several wheel notches before scrolling
        // appeared to respond.
        let actual = tab.parser.screen().scrollback();
        if actual != tab.scrollback {
            tab.scrollback = actual;
            // Cell positions are relative to the visible screen, so a selection
            // made before the scroll would now cover different text.
            tab.anchor = None;
            tab.head = None;
            moved = true;
        }
    }
    term.dirty |= moved;
}

// ── Scrollbar ───────────────────────────────────────────────────────────────

/// Thumb height for a track of `track_h` showing `visible` of `total` lines.
///
/// Shared by the system that draws the thumb and the one that drags it, because
/// a drag maps the pointer to the thumb's *centre* and would otherwise be
/// half a thumb out at both ends of a long history - the point at which a
/// scrollbar stops being able to reach the top of the buffer.
fn thumb_height(track_h: f32, visible: usize, total: usize) -> f32 {
    let ratio = visible as f32 / total.max(1) as f32;
    (track_h * ratio).clamp(MIN_THUMB.min(track_h), track_h)
}

/// Size and place the thumb from the active tab's position in its history.
pub fn scrollbar_sync(
    term: Res<Terminal>,
    tracks: Query<(&ComputedNode, &RelativeCursorPosition), With<TermScrollTrack>>,
    mut thumbs: Query<(&mut Node, &mut BackgroundColor), With<TermScrollThumb>>,
) {
    let (Ok((track, pointer)), Ok((mut node, mut color))) = (tracks.single(), thumbs.single_mut())
    else {
        return;
    };
    let Some(tab) = term.active() else {
        return;
    };
    let history = tab.history;
    // Nothing has scrolled off yet, so there is nowhere to go and nothing to say.
    if history == 0 {
        if node.display != Display::None {
            node.display = Display::None;
        }
        return;
    }

    let track_h = track.size().y * track.inverse_scale_factor();
    if track_h <= 0.0 {
        return;
    }
    let visible = term.rows as usize;
    let thumb_h = thumb_height(track_h, visible, history + visible);
    // `scrollback` counts *backwards*: 0 is the live screen at the bottom of the
    // history, `history` is as far back as it goes. The thumb runs the other way,
    // so the fraction is inverted before it becomes a distance from the top.
    let from_top = 1.0 - (tab.scrollback as f32 / history as f32);
    let top = (track_h - thumb_h) * from_top.clamp(0.0, 1.0);

    if node.display != Display::Flex {
        node.display = Display::Flex;
    }
    node.height = Val::Px(thumb_h);
    node.top = Val::Px(top);
    let lit = term.scroll_dragging || pointer.cursor_over;
    color.0 = rgb(if lit { text_muted() } else { border() });
}

/// Press or drag anywhere on the track to jump through the history.
///
/// The whole track is the control, not just the thumb. A terminal scrollbar is
/// thin by necessity, and requiring the grab to land on the thumb makes the
/// difference between an 8px target and a 2px one at the far end of a long
/// buffer.
pub fn scrollbar_drag(
    mut term: ResMut<Terminal>,
    mouse: Res<ButtonInput<MouseButton>>,
    tracks: Query<(&ComputedNode, &RelativeCursorPosition), With<TermScrollTrack>>,
) {
    let Ok((track, pointer)) = tracks.single() else {
        return;
    };
    if mouse.just_pressed(MouseButton::Left) && pointer.cursor_over {
        term.scroll_dragging = true;
    }
    if !mouse.pressed(MouseButton::Left) {
        term.scroll_dragging = false;
        return;
    }
    if !term.scroll_dragging {
        return;
    }
    let (Some(normalized), Some(tab)) = (pointer.normalized, term.active()) else {
        return;
    };
    let history = tab.history;
    if history == 0 {
        return;
    }
    let track_h = track.size().y * track.inverse_scale_factor();
    let thumb_h = thumb_height(track_h, term.rows as usize, history + term.rows as usize);
    let free = (track_h - thumb_h).max(1.0);
    // The pointer holds the *centre* of the thumb, so the travel it maps onto
    // starts half a thumb down the track and ends half a thumb short of the end.
    let local_y = (normalized.y + 0.5) * track_h - thumb_h * 0.5;
    let from_top = (local_y / free).clamp(0.0, 1.0);
    let wanted = ((1.0 - from_top) * history as f32).round() as usize;

    let mut moved = false;
    if let Some(tab) = term.active_mut() {
        tab.parser.screen_mut().set_scrollback(wanted);
        let actual = tab.parser.screen().scrollback();
        if actual != tab.scrollback {
            tab.scrollback = actual;
            // Cell positions are relative to the visible screen, so a selection
            // made before the scroll would now cover different text.
            tab.anchor = None;
            tab.head = None;
            moved = true;
        }
    }
    term.dirty |= moved;
}

/// Click-drag to select text in the active tab.
pub fn select(
    mut term: ResMut<Terminal>,
    mouse: Res<ButtonInput<MouseButton>>,
    grids: Query<(&RelativeCursorPosition, &ComputedNode), With<TermGrid>>,
) {
    let Ok((pointer, computed)) = grids.single() else {
        return;
    };
    let cell = pointer.normalized.map(|normalized| {
        let size = computed.size() * computed.inverse_scale_factor();
        let local = Vec2::new((normalized.x + 0.5) * size.x, (normalized.y + 0.5) * size.y);
        let col = (local.x / term.advance.max(1.0)).floor();
        let row = (local.y / term.line_h.max(1.0)).floor();
        (
            (row.max(0.0) as u16).min(term.rows.saturating_sub(1)),
            (col.max(0.0) as u16).min(term.cols.saturating_sub(1)),
        )
    });

    let pressed = mouse.pressed(MouseButton::Left);
    let just = mouse.just_pressed(MouseButton::Left);
    let dragging = term.dragging;
    let mut dirty = false;

    if just && pointer.cursor_over {
        if let (Some(cell), Some(tab)) = (cell, term.active_mut()) {
            tab.anchor = Some(cell);
            tab.head = Some(cell);
            dirty = true;
        }
        term.dragging = true;
    } else if dragging && pressed {
        if let (Some(cell), Some(tab)) = (cell, term.active_mut()) {
            if tab.head != Some(cell) {
                tab.head = Some(cell);
                dirty = true;
            }
        }
    } else if dragging {
        term.dragging = false;
        // A click that never moved is a click, not an empty selection. Left as
        // one, the highlight on a single cell would sit there until the next
        // drag.
        if let Some(tab) = term.active_mut() {
            if tab.anchor == tab.head {
                tab.anchor = None;
                tab.head = None;
                dirty = true;
            }
        }
    }
    term.dirty |= dirty;
}
