//! The tab strip: one chip per shell, the controls beside them, and the two
//! menus they can open.
//!
//! Rebuilt on change rather than bound cell by cell. That is the exception to
//! ember's usual "build once, then bind" rule and it is the right one here: a
//! chip is a handful of entities, the list changes only when somebody opens,
//! closes, switches, renames or re-sides a tab, and the alternative is a keyed
//! list whose row hash has to cover the title, the active flag, the rename state
//! and the close button's hover. The version counter on [`Terminal`] keeps the
//! rebuild off every other frame, and "every other frame" is nearly all of them.
//!
//! # The shape changes with the side
//!
//! On [`StripSide::Top`] the chips are a row of pills that keep their natural
//! width; on [`StripSide::Right`] they are a stacked list filling a fixed-width
//! column. That is a different tree, not a different colour, which is why
//! `set_side` bumps the version rather than restyling in place.
//!
//! The strip is always the panel root's **first** child. Putting it on the right
//! is `FlexDirection::RowReverse` on the root, not a reordering: the alternative
//! is moving children between the two layouts, and a hierarchy edit that has to
//! stay in step with a style change is a hierarchy edit that eventually does not.
//!
//! # Chips are addressed by id, never by index
//!
//! Every component here carries a tab's serial. A click is read on a later frame
//! than the one that built the chip, and closing a tab in between shifts every
//! index after it - so an index would eventually act on a tab the user never
//! pointed at. A serial at worst names a tab that is already gone.

use bevy::prelude::*;
use bevy::ui::{ComputedNode, FocusPolicy, RelativeCursorPosition, ScrollPosition, UiGlobalTransform};
use bevy::window::PrimaryWindow;

use renzora_ember::font::{glyph, ui_font, EmberFonts};
use renzora_ember::theme::{
    accent, divider, hover_bg, panel_bg, rgb, tab_active, tab_hover, text_muted, text_primary,
};
// The popup module is private; `widgets` glob-re-exports its public surface, so
// these come from the crate root rather than from `widgets::popup`.
use renzora_ember::widgets::{
    menu_item, screen_menu_est_height, screen_menu_flip, screen_menu_under, scroll_view,
    text_input, trigger_rect, EmberScroll, EmberTextInput,
};

use crate::panel::TermRoot;
use crate::state::{StripSide, Terminal};

/// Thickness of the strip: the height of a top strip, the width of a side one.
const TOP_HEIGHT: f32 = 22.0;
const SIDE_WIDTH: f32 = 132.0;
/// Height of one chip in a side strip. A top strip's chips stretch instead.
const SIDE_CHIP_HEIGHT: f32 = 20.0;
/// How narrow a top strip's chip may be squeezed before the strip overflows
/// instead.
///
/// Deliberately close to a chip's natural width, so a squeeze takes the slack
/// and stops. Set low it does technically fit more tabs, and what you get is ten
/// identical stubs with the close button pushed off each one - more tabs on
/// screen and none of them identifiable, which is not more tabs. Past this the
/// strip scrolls and the `v` lists them by name instead.
const TAB_MIN_W: f32 = 64.0;
/// How long after a click a second one still counts as a double.
const DOUBLE_CLICK_SECS: f32 = 0.4;

/// The strip's container. Its children are rebuilt wholesale by [`sync`].
#[derive(Component)]
pub(crate) struct TermStrip;

/// The scrolling part that holds the chips, so the controls beside it stay put
/// when the chips overflow.
#[derive(Component)]
pub(crate) struct TermChips;

#[derive(Component)]
pub(crate) struct TabButton {
    id: u32,
    /// Carried so [`hover`] knows what colour to fall back to. Reading it from
    /// `Terminal` would work and would mean re-reading the active tab for every
    /// chip on every pointer move, to answer a question the chip already knows.
    active: bool,
}

#[derive(Component)]
pub(crate) struct TabClose(u32);

#[derive(Component)]
pub(crate) struct NewTabButton;

/// The `v` that opens the list of every tab, shown only while they overflow.
#[derive(Component)]
pub(crate) struct OverflowButton;

/// One of the two side toggles.
#[derive(Component)]
pub(crate) struct SideButton(StripSide);

/// The inline rename field, tagged with the tab it renames.
#[derive(Component)]
pub(crate) struct TabRenameInput(u32);

/// Records which version of the tab list the strip currently shows.
///
/// A resource rather than a field on `Terminal`, because it describes the
/// *entities* and has to reset when the panel's tree is rebuilt.
#[derive(Resource)]
pub(crate) struct StripBuilt {
    root: Entity,
    version: Option<u32>,
    /// The side the tree was built for, so [`sync`] can restyle the panel root
    /// and the strip itself only when it actually moved.
    side: StripSide,
}

// ── Build ───────────────────────────────────────────────────────────────────

pub fn build(commands: &mut Commands) -> Entity {
    let root = commands
        .spawn((
            strip_node(StripSide::Top),
            BackgroundColor(rgb(panel_bg())),
            BorderColor::all(rgb(divider())),
            // So the terminal's wheel handler can tell the strip from the grid.
            // A wheel over a column of tabs scrolls the tabs; without this it
            // scrolls the shell's history instead, which is somewhere else
            // entirely on screen.
            RelativeCursorPosition::default(),
            FocusPolicy::Pass,
            TermStrip,
            Name::new("terminal-strip"),
        ))
        .id();
    commands.insert_resource(StripBuilt { root, version: None, side: StripSide::Top });
    root
}

/// The strip's own box, which is the one thing that differs between the sides
/// beyond the chips themselves.
fn strip_node(side: StripSide) -> Node {
    if side.vertical() {
        Node {
            width: Val::Px(SIDE_WIDTH),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            flex_shrink: 0.0,
            // The divider faces the grid, so it is on the strip's *left* edge
            // when the strip is on the right.
            border: UiRect::left(Val::Px(1.0)),
            overflow: Overflow::clip(),
            ..default()
        }
    } else {
        Node {
            width: Val::Percent(100.0),
            min_height: Val::Px(TOP_HEIGHT),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Stretch,
            flex_shrink: 0.0,
            border: UiRect::bottom(Val::Px(1.0)),
            // The chips do not wrap: a strip that ran out of room would
            // otherwise grow a second line and eat the grid below it. Overflow
            // is what the `v` menu is for.
            overflow: Overflow::clip(),
            ..default()
        }
    }
}

/// Rebuild the chips when anything the strip draws has moved.
#[allow(clippy::too_many_arguments)]
pub fn sync(
    mut commands: Commands,
    term: Res<Terminal>,
    built: Option<ResMut<StripBuilt>>,
    fonts: Option<Res<EmberFonts>>,
    strips: Query<Entity, With<TermStrip>>,
    hosts: Query<Entity, With<TermRoot>>,
    mut nodes: Query<&mut Node>,
) {
    let (Some(mut built), Some(fonts)) = (built, fonts) else {
        return;
    };
    // The panel's tree is despawned when its dock tab closes and rebuilt when it
    // reopens, while `StripBuilt` (a resource) survives. Comparing against the
    // live entity is how a stale record is noticed; see the same check in
    // `panel::resize` for the failure it prevents.
    let Ok(strip) = strips.single() else {
        return;
    };
    if built.root == strip && built.version == Some(term.strip_version) {
        return;
    }
    let fresh_tree = built.root != strip;
    built.root = strip;
    built.version = Some(term.strip_version);

    // Restyle the two boxes that carry the side, on a move or on a fresh tree.
    if fresh_tree || built.side != term.side {
        built.side = term.side;
        if let Ok(mut node) = nodes.get_mut(strip) {
            *node = strip_node(term.side);
        }
        // The panel stacks the strip above the grid, or beside it. This is the
        // only place the panel root's own direction is written, which is why the
        // strip owns it rather than `panel`: the side is the strip's property and
        // splitting the decision across two modules is how the two get out of
        // step.
        //
        // `RowReverse` rather than `Row`, because the strip is the root's first
        // child and reversing the axis is what puts it on the right without
        // moving anything. The probe is the root's other child and is absolutely
        // positioned, so the reversal never reaches it.
        if let Ok(host) = hosts.single() {
            if let Ok(mut node) = nodes.get_mut(host) {
                node.flex_direction = if term.side.vertical() {
                    FlexDirection::RowReverse
                } else {
                    FlexDirection::Column
                };
            }
        }
    }

    commands.entity(strip).despawn_related::<Children>();

    let vertical = term.side.vertical();
    let chips_box = commands
        .spawn((
            Node {
                // A side strip's list is wrapped in an ember scroll view, which
                // owns the growing and the scrolling; the list inside it is just
                // as tall as its chips.
                flex_grow: if vertical { 0.0 } else { 1.0 },
                flex_basis: if vertical { Val::Auto } else { Val::Px(0.0) },
                width: if vertical { Val::Percent(100.0) } else { Val::Auto },
                // These two are load-bearing, and their absence is not subtle.
                // A flex item's automatic minimum size is its *content's* size,
                // so a box holding chips that will not shrink has a floor equal
                // to all of them added up - which beats `flex_basis: 0`, grows
                // the box past the strip, and pushes the controls beside it off
                // the end. The `+`, the `v` and the layout toggles all vanish at
                // once, and because the box has grown to fit, the overflow check
                // below sees content that exactly fits its container and never
                // raises the `v` that would have got the tabs back.
                min_width: Val::Px(0.0),
                min_height: Val::Px(0.0),
                flex_direction: if vertical { FlexDirection::Column } else { FlexDirection::Row },
                align_items: AlignItems::Stretch,
                // Horizontal only. A row scrolls itself, because there is no
                // room under a 22px strip for a horizontal scrollbar and the `v`
                // menu is the answer there instead. A column hands both jobs to
                // the ember scroll view wrapped around it.
                overflow: if vertical { Overflow::visible() } else { Overflow::scroll_x() },
                ..default()
            },
            ScrollPosition::default(),
            FocusPolicy::Pass,
            TermChips,
            Name::new("terminal-chips"),
        ))
        .id();

    let active = term.active_id();
    let mut chips: Vec<Entity> = Vec::with_capacity(term.tabs().len() + 1);
    for tab in term.tabs() {
        chips.push(chip(
            &mut commands,
            &fonts,
            tab.id,
            &tab.title,
            tab.id == active,
            term.renaming == Some(tab.id),
            vertical,
        ));
    }
    // In a top strip `+` rides at the end of the list, so it is where the next
    // tab will appear. The cost is that it clips away with the chips once they
    // overflow; the overflow menu carries its own New Tab row for exactly that
    // case, and Ctrl+Shift+T never moves.
    //
    // A side strip puts it in the header band instead (see [`controls`]). Below
    // the last chip it would be a long way from the controls in a tall panel, and
    // further from them the more tabs there are - and the band is outside the
    // list that clips, so it never goes missing there.
    if !vertical {
        chips.push(control(
            &mut commands,
            "plus",
            text_muted(),
            true,
            None,
            NewTabButton,
            "terminal-new-tab",
        ));
    }
    commands.entity(chips_box).add_children(&chips);

    let controls = controls(&mut commands, term.side, term.overflowing);
    // A column gets ember's scroll view: a real, draggable, auto-hiding
    // scrollbar, which is what a vertical list of tabs wants and what a 22px
    // horizontal strip has no room for.
    let list = if vertical { scroll_view(&mut commands, chips_box) } else { chips_box };
    // On a side strip the controls read as a header above the list; on a top
    // strip they belong at the trailing end, after the tabs.
    let order = if vertical { [controls, list] } else { [list, controls] };
    commands.entity(strip).add_children(&order);
}

fn chip(
    commands: &mut Commands,
    fonts: &EmberFonts,
    id: u32,
    title: &str,
    active: bool,
    renaming: bool,
    vertical: bool,
) -> Entity {
    let (bg, fg) = if active { (tab_active(), text_primary()) } else { (panel_bg(), text_muted()) };
    let mut node = Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(4.0),
        padding: UiRect::axes(Val::Px(8.0), Val::Px(0.0)),
        // A top strip's chips give ground before they clip: they shrink from
        // their natural width down to `TAB_MIN_W`, which is what lets eight tabs
        // fit a strip that comfortably held four. Only once they are all at that
        // floor does the strip genuinely overflow and the `v` appear. Clipping
        // straight from natural width would hide whole tabs while there was
        // still room to make them narrower.
        flex_shrink: 1.0,
        min_width: Val::Px(TAB_MIN_W),
        ..default()
    };
    if vertical {
        // A column stacks instead. Shrinking a chip's height only makes its
        // label unreadable, and there is no width to reclaim.
        node.flex_shrink = 0.0;
        node.min_width = Val::Auto;
        node.height = Val::Px(SIDE_CHIP_HEIGHT);
        node.width = Val::Percent(100.0);
    }
    let root = commands
        .spawn((
            node,
            BackgroundColor(rgb(bg)),
            Interaction::default(),
            // Block, or the click continues to the panel root and the strip
            // reads as part of the grid.
            FocusPolicy::Block,
            // The pointer's position inside the chip, so a right-click can open
            // its menu at the cursor.
            RelativeCursorPosition::default(),
            TabButton { id, active },
            Name::new(format!("terminal-tab-{id}")),
        ))
        .id();

    let label = if renaming {
        let input = text_input(commands, &fonts.ui, "name", title);
        // Ember's field is sized for a form row (180px minimum, 8x5 padding),
        // which is wider than most chips. Overridden rather than parameterised:
        // one caller wanting a compact field is not a reason to widen the
        // widget's API, and the shape is four numbers.
        commands.entity(input).insert((
            Node {
                min_width: Val::Px(70.0),
                padding: UiRect::axes(Val::Px(4.0), Val::Px(1.0)),
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(3.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            TabRenameInput(id),
        ));
        input
    } else {
        commands
            .spawn((
                Text::new(title.to_string()),
                ui_font(&fonts.ui, 10.0),
                TextColor(rgb(fg)),
                TextLayout::no_wrap(),
                // The label takes the room the close button leaves, so a long
                // name is clipped rather than pushing the `x` out. `min_width`
                // again: without it the label's own content is a floor, and a
                // shrinking chip cannot get below the width of its title.
                Node {
                    flex_grow: 1.0,
                    flex_basis: Val::Px(0.0),
                    min_width: Val::Px(0.0),
                    overflow: Overflow::clip(),
                    ..default()
                },
                FocusPolicy::Pass,
            ))
            .id()
    };

    // The close button is its own interactive node inside the chip, so a press
    // on the `x` never also switches to the tab being closed.
    let close = commands
        .spawn((
            Node {
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                flex_shrink: 0.0,
                ..default()
            },
            // Present and transparent, because `hover` writes into it. A node
            // with no `BackgroundColor` would not match that query at all, and
            // the button would silently never light up.
            BackgroundColor(Color::NONE),
            Interaction::default(),
            FocusPolicy::Block,
            TabClose(id),
            Name::new(format!("terminal-tab-close-{id}")),
        ))
        .id();
    let icon = glyph(commands, "x", text_muted(), 9.0);
    commands.entity(close).add_child(icon);
    commands.entity(root).add_children(&[label, close]);
    root
}

/// The overflow `v`, `+` in a side strip, and the two side toggles.
///
/// Nothing here is inside the box that clips the chips, so everything here stays
/// reachable when they do not fit. That is why the two `+` positions differ: a
/// top strip's sits at the end of the tab list, pointing at where the next tab
/// lands, while a side strip's joins this band, immediately left of the toggles.
/// Under a column of chips it would drift further from the other controls with
/// every tab opened.
fn controls(commands: &mut Commands, side: StripSide, overflowing: bool) -> Entity {
    let row = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::FlexEnd,
                flex_shrink: 0.0,
                // A side strip's controls are a header band across the column;
                // a top strip's are a cluster at the end of the row.
                width: if side.vertical() { Val::Percent(100.0) } else { Val::Auto },
                height: if side.vertical() { Val::Px(TOP_HEIGHT) } else { Val::Auto },
                ..default()
            },
            FocusPolicy::Pass,
            Name::new("terminal-controls"),
        ))
        .id();

    let mut kids = Vec::with_capacity(4);
    // Horizontal only. A side strip reaches an off-screen tab by scrolling to
    // it, which is a thing you can do without first knowing it is there; a menu
    // listing the same tabs a scrollbar already reaches is a second answer to a
    // question that has one.
    //
    // Built either way and hidden when it is not needed, rather than added and
    // removed: the strip only rebuilds on a version bump, and overflow is
    // measured every frame from layout that the rebuild itself changes.
    if !side.vertical() {
        kids.push(control(
            commands,
            "caret-down",
            text_muted(),
            overflowing,
            None,
            OverflowButton,
            "terminal-overflow",
        ));
    }
    if side.vertical() {
        kids.push(control(
            commands,
            "plus",
            text_muted(),
            true,
            None,
            NewTabButton,
            "terminal-new-tab",
        ));
    }
    for target in [StripSide::Top, StripSide::Right] {
        let on = side == target;
        let icon = if target.vertical() { "columns" } else { "rows" };
        kids.push(control(
            commands,
            icon,
            if on { accent() } else { text_muted() },
            true,
            None,
            SideButton(target),
            if target.vertical() { "terminal-side-right" } else { "terminal-side-top" },
        ));
    }
    commands.entity(row).add_children(&kids);
    row
}

/// One icon button. `height` is set only for a button stacked in a side strip's
/// column, where nothing else gives it one.
#[allow(clippy::too_many_arguments)]
fn control(
    commands: &mut Commands,
    icon: &str,
    color: (u8, u8, u8),
    shown: bool,
    height: Option<f32>,
    marker: impl Component,
    name: &str,
) -> Entity {
    let root = commands
        .spawn((
            Node {
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                padding: UiRect::axes(Val::Px(6.0), Val::Px(0.0)),
                flex_shrink: 0.0,
                height: height.map(Val::Px).unwrap_or(Val::Auto),
                display: if shown { Display::Flex } else { Display::None },
                ..default()
            },
            BackgroundColor(Color::NONE),
            Interaction::default(),
            FocusPolicy::Block,
            // Needed by the two that open a menu, to recover their own on-screen
            // rect; harmless on the others.
            RelativeCursorPosition::default(),
            marker,
            Name::new(name.to_string()),
        ))
        .id();
    let glyph = glyph(commands, icon, color, 10.0);
    commands.entity(root).add_child(glyph);
    root
}

// ── Overflow ────────────────────────────────────────────────────────────────

/// Decide whether the chips still fit, and show or hide the `v` accordingly.
///
/// Measured from laid-out sizes rather than predicted from the titles: a chip's
/// width is a font measurement, and guessing it from the character count is how
/// a strip ends up claiming to overflow at 80% full in one theme and clipping
/// silently in another.
///
/// This cannot oscillate, even though the chips shrink and showing the `v`
/// narrows the box they shrink in. While they still have room to give, they
/// shrink to fill it exactly and the sum equals the container, so no `v` is
/// raised. Only once every chip has bottomed out at [`TAB_MIN_W`] does the sum
/// exceed the container, and from then on it is a fixed number that taking 20px
/// away for the `v` cannot bring back under. There is no width at which hiding
/// the button would let the chips fit and showing it would stop them.
pub fn overflow(
    mut term: ResMut<Terminal>,
    boxes: Query<(&ComputedNode, &Children), With<TermChips>>,
    sizes: Query<&ComputedNode>,
    mut buttons: Query<&mut Node, With<OverflowButton>>,
) {
    // A side strip has no `v` to raise: its scrollbar already says there is more
    // and reaches it.
    if term.side.vertical() {
        term.overflowing = false;
        return;
    }
    let Ok((container, kids)) = boxes.single() else {
        return;
    };
    let available = container.size().x;
    if available <= 0.0 {
        return;
    }
    // Every child, not every chip: `+` sits in this list too, and a strip that is
    // full except for the button that no longer fits is a strip that overflows.
    let used: f32 = kids
        .iter()
        .filter_map(|kid| sizes.get(kid).ok())
        .map(|c| c.size().x)
        .sum();
    // A pixel of slack: the sum of rounded child sizes can exceed a rounded
    // parent by less than one, and a `v` that appears on an exactly-full strip
    // is a `v` that appears for no reason.
    let over = used > available + 1.0;
    if over != term.overflowing {
        term.overflowing = over;
    }
    let want = if over { Display::Flex } else { Display::None };
    for mut node in &mut buttons {
        if node.display != want {
            node.display = want;
        }
    }
}

/// Scroll the active chip into view when it has been squeezed off the end.
///
/// The case this exists for is pressing `+` with a full strip: the new tab is
/// created, activated, and lands past the clip, so the panel appears to have
/// done nothing at all.
///
/// Expressed as a *delta* rather than an absolute offset, and that is the whole
/// reason it is short. `UiGlobalTransform` already has the current scroll baked
/// into it, so "how far outside the box is this chip" is exactly "how far should
/// the scroll move", and the correction re-measures against the result next
/// frame. There is no content width to compute, no chip offsets to accumulate,
/// and nothing to keep in step with a rebuild.
pub fn keep_active_visible(
    term: Res<Terminal>,
    boxes: Query<(Entity, &ComputedNode, &UiGlobalTransform), With<TermChips>>,
    chips: Query<(&ComputedNode, &UiGlobalTransform, &TabButton)>,
    parents: Query<&ChildOf>,
    mut rows: Query<&mut ScrollPosition, With<TermChips>>,
    // `Without<TermChips>` because `rows` above takes `&mut ScrollPosition` and
    // this takes `&ScrollPosition`: Bevy's conflict check is static and sees an
    // alias, and panics at first run with B0001. The two never touch the same
    // entity in practice: `views` is only ever fetched for the chip strip's
    // *parent*, and a parent does not carry its child's marker — so saying so
    // in the filter costs nothing and is what makes the pair provably disjoint.
    mut views: Query<(&mut EmberScroll, &ComputedNode, &ScrollPosition), Without<TermChips>>,
) {
    let Ok((box_entity, container, container_at)) = boxes.single() else {
        return;
    };
    let active = term.active_id();
    let Some((chip, chip_at)) = chips
        .iter()
        .find(|(_, _, button)| button.id == active)
        .map(|(chip, at, _)| (chip, at))
    else {
        return;
    };

    // `ComputedNode` sizes and `UiGlobalTransform` translations are both
    // physical; one scale factor converts the pair, and since only differences
    // are used it would cancel anyway.
    let inv = container.inverse_scale_factor();
    let vertical = term.side.vertical();
    let axis = |v: Vec2| if vertical { v.y } else { v.x };

    let half_box = axis(container.size()) * inv * 0.5;
    let half_chip = axis(chip.size()) * inv * 0.5;
    let box_at = axis(container_at.translation) * inv;
    let chip_at = axis(chip_at.translation) * inv;
    let before = (chip_at - half_chip) - (box_at - half_box);
    let after = (chip_at + half_chip) - (box_at + half_box);

    if vertical {
        // The list is the content of an ember scroll view, and the view is its
        // parent. Its `ScrollPosition` must not be written directly: `EmberScroll`
        // eases toward a target and rewrites the position every frame, so a
        // direct write is undone before it is drawn. `scroll_to` moves the target.
        //
        // And the target is absolute, not a delta - so the offset is computed
        // from the chip's place in the content rather than from how far outside
        // the viewport it currently is. That makes the call idempotent, which it
        // has to be: the easing takes several frames, and during them the chip is
        // still outside the viewport and this runs again.
        let Ok(view_entity) = parents.get(box_entity).map(|p| p.parent()) else {
            return;
        };
        let Ok((mut view, _, position)) = views.get_mut(view_entity) else {
            return;
        };
        let offset = position.0.y;
        let target = if before < 0.0 {
            offset + before
        } else if after > 0.0 {
            offset + after
        } else {
            return;
        };
        view.scroll_to(target.max(0.0));
        return;
    }

    let delta = if before < 0.0 {
        before
    } else if after > 0.0 {
        after
    } else {
        return;
    };
    // Sub-pixel corrections would keep the resource marked changed forever for
    // a difference nobody can see.
    if delta.abs() < 0.5 {
        return;
    }
    if let Ok(mut scroll) = rows.single_mut() {
        scroll.0.x = (scroll.0.x + delta).max(0.0);
    }
}

/// Open the list of every tab from the `v`.
pub fn overflow_menu(
    mut commands: Commands,
    term: Res<Terminal>,
    fonts: Option<Res<EmberFonts>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    buttons: Query<
        (&Interaction, &RelativeCursorPosition, &ComputedNode),
        (With<OverflowButton>, Changed<Interaction>),
    >,
) {
    let (Some(fonts), Ok(window)) = (fonts, windows.single()) else {
        return;
    };
    for (interaction, pointer, computed) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(cursor) = window.cursor_position() else {
            continue;
        };
        let rect = trigger_rect(cursor, pointer, computed);
        let est = screen_menu_est_height(term.tabs().len() + 1, 0);
        let menu = screen_menu_under(&mut commands, rect, window.height(), est);
        let active = term.active_id();
        let mut rows: Vec<Entity> = term
            .tabs()
            .iter()
            .map(|tab| {
                let id = tab.id;
                menu_item(
                    &mut commands,
                    &fonts,
                    // A tick on the one you are already looking at, so the list
                    // says where you are as well as where you can go.
                    if id == active { "check" } else { "terminal-window" },
                    &tab.title,
                    move |world: &mut World| {
                        if let Some(mut term) = world.get_resource_mut::<Terminal>() {
                            term.activate(id);
                        }
                    },
                )
            })
            .collect();
        // The only `+` still on screen. It lives at the end of the tab list now,
        // so once the chips overflow it has been clipped away with them, and this
        // menu is what the overflow is for.
        rows.push(menu_item(&mut commands, &fonts, "plus", "New Tab", |world: &mut World| {
            if let Some(mut term) = world.get_resource_mut::<Terminal>() {
                term.open();
            }
        }));
        commands.entity(menu).add_children(&rows);
    }
}

// ── Clicks, gestures and menus ──────────────────────────────────────────────

/// Switch, close, open, and move the strip.
///
/// `Changed<Interaction>` on every query: `Pressed` persists for as long as the
/// button is held, so without it a single click on `+` opens a tab per frame.
pub fn clicks(
    mut term: ResMut<Terminal>,
    switch: Query<(&Interaction, &TabButton), Changed<Interaction>>,
    close: Query<(&Interaction, &TabClose), Changed<Interaction>>,
    new: Query<&Interaction, (With<NewTabButton>, Changed<Interaction>)>,
    sides: Query<(&Interaction, &SideButton), Changed<Interaction>>,
) {
    // Closes are read first. A press on the `x` is a press on a node inside the
    // chip, and both are reported: handling the switch first would activate a tab
    // and then close it, leaving the strip on whichever tab the close fell back
    // to rather than on the one the user was looking at.
    let mut closed = None;
    for (interaction, target) in &close {
        if *interaction == Interaction::Pressed {
            closed = Some(target.0);
            term.close(target.0);
        }
    }
    for (interaction, target) in &switch {
        if *interaction == Interaction::Pressed && closed != Some(target.id) {
            // Deliberately does NOT end a rename in progress. Clicking another
            // tab blurs the field, and `rename_commit` reads that blur as a
            // commit - the same as clicking off any other inline field. Clearing
            // `renaming` here instead would run first and make `rename_commit`
            // return early, silently throwing the typed name away.
            term.activate(target.id);
        }
    }
    if new.iter().any(|i| *i == Interaction::Pressed) {
        term.open();
    }
    for (interaction, side) in &sides {
        if *interaction == Interaction::Pressed {
            term.set_side(side.0);
        }
    }
}

/// Double-click to rename, right-click for the chip's menu.
///
/// Both live here rather than in [`clicks`] because neither is a plain press:
/// `Interaction` reports the left button only, so the right-click is read from
/// `ButtonInput` against whichever chip is hovered, and the double-click needs
/// the timing that a single `Pressed` cannot carry.
pub fn gestures(
    mut commands: Commands,
    mut term: ResMut<Terminal>,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    fonts: Option<Res<EmberFonts>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    chips: Query<(&Interaction, &TabButton)>,
    pressed: Query<(&Interaction, &TabButton), Changed<Interaction>>,
    mut last_click: Local<Option<(u32, f32)>>,
) {
    let now = time.elapsed_secs();
    for (interaction, chip) in &pressed {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let double = last_click
            .filter(|(id, at)| *id == chip.id && now - at < DOUBLE_CLICK_SECS)
            .is_some();
        if double {
            term.set_renaming(Some(chip.id));
            // Cleared, so a third click is a fresh single rather than a second
            // double: without it, click-click-click renames twice.
            *last_click = None;
        } else {
            *last_click = Some((chip.id, now));
        }
    }

    if !mouse.just_pressed(MouseButton::Right) {
        return;
    }
    let Some(chip) = chips
        .iter()
        .find(|(interaction, _)| **interaction != Interaction::None)
        .map(|(_, chip)| chip.id)
    else {
        return;
    };
    let (Some(fonts), Ok(window)) = (fonts, windows.single()) else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let menu = screen_menu_flip(&mut commands, cursor.x, cursor.y, window.height());
    let rows = [
        menu_item(&mut commands, &fonts, "pencil-simple", "Rename", move |world: &mut World| {
            if let Some(mut term) = world.get_resource_mut::<Terminal>() {
                term.set_renaming(Some(chip));
            }
        }),
        menu_item(&mut commands, &fonts, "plus", "New Tab", |world: &mut World| {
            if let Some(mut term) = world.get_resource_mut::<Terminal>() {
                term.open();
            }
        }),
        menu_item(&mut commands, &fonts, "x", "Close", move |world: &mut World| {
            if let Some(mut term) = world.get_resource_mut::<Terminal>() {
                term.close(chip);
            }
        }),
        menu_item(&mut commands, &fonts, "broom", "Close Others", move |world: &mut World| {
            if let Some(mut term) = world.get_resource_mut::<Terminal>() {
                term.close_others(chip);
            }
        }),
    ];
    commands.entity(menu).add_children(&rows);
}

// ── Rename ──────────────────────────────────────────────────────────────────

/// Focus the rename field the frame it appears.
///
/// It is spawned by [`sync`] rather than by a click on the field itself, so
/// `text_input`'s own focus-on-press never runs for it. `select_all` makes the
/// first keystroke replace the name, which is what every OS rename does.
pub fn focus_rename(mut inputs: Query<&mut EmberTextInput, Added<TabRenameInput>>) {
    for mut input in &mut inputs {
        input.focused = true;
        input.select_all = true;
    }
}

/// Commit (Enter, or clicking away) or cancel (Escape) the rename in progress.
pub fn rename_commit(
    mut term: ResMut<Terminal>,
    keys: Res<ButtonInput<KeyCode>>,
    inputs: Query<(&EmberTextInput, &TabRenameInput)>,
    mut had_focus: Local<bool>,
) {
    let Some(id) = term.renaming else {
        *had_focus = false;
        return;
    };
    if keys.just_pressed(KeyCode::Escape) {
        term.set_renaming(None);
        *had_focus = false;
        return;
    }
    // Wait for the field to spawn; `sync` builds it a frame after the rename
    // begins, and cancelling in the meantime would make double-click do nothing.
    let Some((input, _)) = inputs.iter().find(|(_, target)| target.0 == id) else {
        return;
    };
    if input.focused {
        *had_focus = true;
    }
    let enter = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter);
    // Only a field that has actually held focus can lose it. Without that, the
    // frame between spawning and focusing reads as a blur and the rename ends
    // before the user has typed anything.
    let blurred = *had_focus && !input.focused;
    if !enter && !blurred {
        return;
    }
    let name = input.value.replace('\n', "");
    *had_focus = false;
    term.set_renaming(None);
    term.rename(id, &name);
}

// ── Hover ───────────────────────────────────────────────────────────────────

/// Light the chip or control under the pointer.
///
/// Its own system rather than an ember widget because a chip is a bespoke
/// multi-part control, and its halves have to highlight independently: hovering
/// the `x` should show that the `x` is what will be hit, not that the tab is.
pub fn hover(
    mut tabs: Query<(&Interaction, &TabButton, &mut BackgroundColor), Changed<Interaction>>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (
            Changed<Interaction>,
            Without<TabButton>,
            Or<(With<TabClose>, With<NewTabButton>, With<OverflowButton>, With<SideButton>)>,
        ),
    >,
) {
    for (interaction, tab, mut bg) in &mut tabs {
        let base = if tab.active { tab_active() } else { panel_bg() };
        bg.0 = rgb(if *interaction == Interaction::None { base } else { tab_hover() });
    }
    for (interaction, mut bg) in &mut buttons {
        // Transparent when idle: these sit inside a chip or a strip whose colour
        // already says what it is, and painting them their own background would
        // draw a second box around the icon.
        bg.0 = if *interaction == Interaction::None { Color::NONE } else { rgb(hover_bg()) };
    }
}
