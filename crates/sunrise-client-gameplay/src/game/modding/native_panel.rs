//! The item panel beside container screens: the items a recipe viewer mod
//! lists through `panel_items`, drawn as the game's own item slots
//! in pages right of the screen's window, with a search box under them. The
//! mod says what the panel holds; the game pages it and tells the mod what
//! the player clicked, typed, and which keys went down over an item
//! (`native_panel_event`). The mouse wheel over the panel pages it, items
//! or recipes, one page per notch and stopping at both ends, and the screen
//! underneath doesn't hear that notch. An item dragged off the list and let
//! go over the screen comes to the mod as `Dropped` (`Held`). With no mod
//! showing a panel, each hook here returns after one check.
use super::{Screen, native_panel_recipes::RecipePage};
use mod_api::{PanelStyle, Stack as BrowserStack};
use sunrise_client::{
    hud::{HudBatch, rgba},
    ui::{EditBox, utf16_prefix},
};
use sunrise_gameplay_common::game::native_hooks::PanelEvent as BrowserEvent;
use sunrise_items::items::ItemStack;

/// One slot of the panel, in GUI pixels.
const SLOT: f32 = 18.0;
/// The most columns the panel takes, and the fewest it shows with.
const COLUMNS: (usize, usize) = (12, 2);
/// The gap between the window and the panel, and the screen's edges.
const MARGIN: f32 = 8.0;
const RECIPES_SETTINGS_WIDTH: f32 = 20.0;
/// The classic look's gap to the window and the screen's edges, and its
/// square buttons and search box (`PanelStyle::Classic`, JEI 1.12's
/// `IngredientListOverlay`).
const CLASSIC_PAD: f32 = 2.0;
const CLASSIC_BUTTON: f32 = 20.0;

/// Items a panel may hold.
const ITEMS: usize = 8192;
/// The search box's length, in UTF-16 code units as vanilla's `EditBox`
/// counts it (Java's `String.length()`).
const SEARCH_LENGTH: usize = 50;

/// How far, in GUI pixels, the pointer moves with the left button held on
/// one of the panel's items before the press becomes a drag. Vanilla's
/// screens have no drag threshold (`AbstractContainerScreen` drags from the
/// first move); a few pixels keep a click with a shaky hand a click.
pub(in crate::game) const DRAG_THRESHOLD: f32 = 4.0;

/// The browser's overlay selected while painting its contents.
pub enum BrowserTooltip {
    Text(String),
    Stack(ItemStack),
    Fluid(mod_api::FluidStack),
    /// An item dragged off the panel (`Held`), drawn under the pointer.
    Dragged(ItemStack),
}

/// A left press on one of the panel's items, from the press to its release:
/// a click until the pointer moves `DRAG_THRESHOLD` away, then a drag of the
/// item, drawn under the pointer, that its release drops on the screen
/// (`PanelEvent::Dropped` to the panel's owner) or, back on the panel,
/// cancels. It belongs to the menu open when it went down (`menu`): Escape,
/// focus loss, and that menu closing or being replaced cancel it. Its
/// release is the panel's whatever happened, and never reaches a screen.
#[derive(Clone, Debug)]
pub(in crate::game) struct Held {
    /// Whose panel the item is on.
    pub(in crate::game) owner: Owner,
    /// The item's index in the mod's list, as the mod sent it, and as a stack.
    pub(in crate::game) index: usize,
    pub(in crate::game) sent: BrowserStack,
    pub(in crate::game) stack: ItemStack,
    /// Where the press went down, in GUI pixels.
    pub(in crate::game) from: (f32, f32),
    /// Whether it has become a drag.
    pub(in crate::game) dragging: bool,
    /// The menu open when it went down (`ScreenState::screen_instance`).
    pub(in crate::game) menu: u64,
    /// Cancelled: it draws nothing and its release sends nothing.
    pub(in crate::game) cancelled: bool,
}

/// What the release of a `Held` press does.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::game) enum Release {
    /// It never became a drag: the item's left click.
    Click(usize),
    /// A drag let go off the panel, at this point.
    Drop(BrowserStack, (f32, f32)),
    /// Nothing: a drag let go back on the panel, a cancelled press, or one
    /// whose menu, panel or item went away under it.
    Cancel,
}

/// The game as a `Held` press's release finds it.
#[derive(Clone, Copy, Debug)]
pub(in crate::game) struct Now<'a> {
    /// The open menu (`ScreenState::screen_instance`).
    pub(in crate::game) menu: u64,
    /// Whose panel shows, if one does.
    pub(in crate::game) owner: Option<&'a Owner>,
    /// Whether the release is over the panel; None when it has no room.
    pub(in crate::game) on_panel: Option<bool>,
    /// The item at the pressed index of the shown list.
    pub(in crate::game) at_index: Option<&'a BrowserStack>,
}

impl Held {
    /// Whether it still counts on menu `menu`: not cancelled, and on the
    /// menu it went down on.
    fn live_on(&self, menu: u64) -> bool {
        !self.cancelled && self.menu == menu
    }
    /// The pointer is at `cursor` with the button still held: true once the
    /// press is a drag, which it stays until its release.
    pub(in crate::game::client_side) fn moved(&mut self, cursor: (f32, f32)) -> bool {
        let (dx, dy) = (cursor.0 - self.from.0, cursor.1 - self.from.1);
        self.dragging |= dx * dx + dy * dy >= DRAG_THRESHOLD * DRAG_THRESHOLD;
        self.dragging
    }
    /// It does nothing more; its release is still the panel's.
    fn cancel(&mut self) {
        self.cancelled = true;
        self.dragging = false;
    }
    /// The button came up at `cursor`, with the game as `now` finds it. The
    /// release point counts as a last move. A click needs the pressed item
    /// still at its index; a drop sends the item as it was pressed.
    pub(in crate::game::client_side) fn release(
        mut self,
        cursor: (f32, f32),
        now: Now<'_>,
    ) -> Release {
        let Some(on_panel) = now.on_panel else {
            return Release::Cancel;
        };
        if !self.live_on(now.menu) || now.owner != Some(&self.owner) {
            Release::Cancel
        } else if !self.moved(cursor) {
            if now.at_index == Some(&self.sent) {
                Release::Click(self.index)
            } else {
                Release::Cancel
            }
        } else if on_panel {
            Release::Cancel
        } else {
            Release::Drop(self.sent, cursor)
        }
    }
}

/// The panel's overlay, in order: an item dragged off the list over
/// everything, the settings button's label under it, then `rest` (the
/// pointed item's or fluid's tooltip).
fn browser_overlay(
    dragged: Option<ItemStack>,
    settings: Option<String>,
    rest: impl FnOnce() -> Option<BrowserTooltip>,
) -> Option<BrowserTooltip> {
    if let Some(stack) = dragged {
        Some(BrowserTooltip::Dragged(stack))
    } else if let Some(label) = settings {
        Some(BrowserTooltip::Text(label))
    } else {
        rest()
    }
}

/// Whose panel it is: a native mod's id.
#[derive(Clone, Debug, PartialEq)]
pub enum Owner {
    Native(String),
}
impl Owner {
    fn native_id(&self) -> &str {
        let Owner::Native(id) = self;
        id
    }
}

/// What a mod shows on the panel: its items, and the recipes shown in
/// their place.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::game) struct Shown {
    pub(in crate::game) owner: Owner,
    /// Which change the panel is from, so the game converts it once.
    pub(in crate::game) version: u64,
    pub(in crate::game) items: Vec<BrowserStack>,
    pub(in crate::game) recipes: Option<RecipePage>,
}

/// The panel's state in the game.
#[derive(Default)]
pub struct Panel {
    /// A native mod's panel: the last one to show something.
    native: Option<Shown>,
    /// The last native change.
    versions: u64,
    /// The items' owner and version they were made from.
    of: Option<(Owner, u64)>,
    /// The mod's items as stacks; `None` for an item the game doesn't have.
    stacks: Vec<Option<ItemStack>>,
    pub page: usize,
    pub search: Option<EditBox>,
    /// The search text the mod last heard.
    sent: String,
    /// A search text the panel's owner set (`panel_search`), cut to the box's
    /// length: it stays out of the box until a frame draws that owner's box.
    pub pending: Option<(Owner, String)>,
    /// Only while dispatching Search: an empty reply still needs an editable field.
    searching: bool,
    pub(in crate::game) recipes_settings_return: Option<Screen>,
    /// The recipes shown instead of the items.
    pub(in crate::game) recipes: Option<super::native_panel_recipes::Pages>,
    pub(in crate::game) transfer_warning: Option<(&'static str, &'static str)>,
    /// The mods that asked for another look than the recipe book's.
    styles: std::collections::BTreeMap<String, PanelStyle>,
    /// A left press on an item, until its release: a click or a drag.
    pub(in crate::game) held: Option<Held>,
}

impl Panel {
    /// The item being dragged off `owner`'s panel on menu `menu`, if a drag
    /// is on.
    pub(in crate::game::client_side) fn dragged(
        &self,
        owner: &Owner,
        menu: u64,
    ) -> Option<&ItemStack> {
        (self.held.as_ref())
            .filter(|held| held.live_on(menu) && held.dragging && held.owner == *owner)
            .map(|held| &held.stack)
    }
    /// Cancels the held press (Escape, focus loss, its menu closing): true
    /// when it was a drag. Its release stays the panel's.
    pub(in crate::game::client_side) fn cancel_press(&mut self) -> bool {
        let Some(held) = self.held.as_mut().filter(|held| !held.cancelled) else {
            return false;
        };
        let dragging = held.dragging;
        held.cancel();
        dragging
    }
    /// A button went down anywhere. A left one means the held press's
    /// release never came, so it is forgotten; another cancels it.
    pub(in crate::game::client_side) fn other_press(&mut self, left: bool) {
        if left {
            self.held = None;
        } else {
            self.cancel_press();
        }
    }
    /// A mod's screen hook vetoed the held press's release: the press and
    /// its dragged item go, and nothing is sent.
    pub(in crate::game::client_side) fn release_vetoed(&mut self) {
        self.held = None;
    }
    /// The pointer moved to `cursor` with menu `menu` open: true while a
    /// press is held. A press from another menu is cancelled.
    pub(in crate::game::client_side) fn press_moved(
        &mut self,
        cursor: (f32, f32),
        menu: u64,
    ) -> bool {
        let Some(held) = self.held.as_mut() else {
            return false;
        };
        if held.live_on(menu) {
            held.moved(cursor);
        } else {
            held.cancel();
        }
        true
    }
    /// `panel_style`.
    pub(in crate::game::client_side) fn set_style(&mut self, mod_id: &str, style: PanelStyle) {
        match style {
            PanelStyle::RecipeBook => self.styles.remove(mod_id),
            _ => self.styles.insert(mod_id.to_owned(), style),
        };
    }
    /// `panel_search`, called by mod `caller` (the mod whose hook or part is
    /// running): `text`, cut to the box's length, waits for a frame that
    /// draws mod `mod_id`'s search box (`deliver_search`). The box itself
    /// doesn't change here. False, and nothing queued, unless the caller is
    /// `mod_id` and owns the panel; a later call replaces an earlier one.
    pub(in crate::game::client_side) fn set_search(
        &mut self,
        caller: Option<&str>,
        mod_id: &str,
        text: &str,
    ) -> bool {
        let Some(shown) = self.native.as_ref() else {
            return false;
        };
        if caller != Some(mod_id) || shown.owner.native_id() != mod_id {
            return false;
        }
        self.pending = Some((shown.owner.clone(), utf16_prefix(text, SEARCH_LENGTH)));
        true
    }
    /// On a frame that draws `owner`'s search box: the text `owner` queued
    /// (`set_search`) goes into the box, its cursor at the end. True when the
    /// box's text changed. A text queued by another owner is dropped, never
    /// shown; one the box already holds leaves the box, its cursor and
    /// selection, untouched.
    fn deliver_search(&mut self, owner: &Owner) -> bool {
        let Some((queued_by, text)) = self.pending.take() else {
            return false;
        };
        if queued_by != *owner {
            return false;
        }
        let search = self.search.get_or_insert_with(new_search_box);
        if utf16_prefix(&text, search.max_length) == search.value {
            return false;
        }
        search.set_value(&text);
        true
    }
    /// How `owner`'s panel looks: Sunrise's own Recipes panel is the viewer's.
    fn style(&self, owner: &Owner) -> PanelStyle {
        match owner.native_id() {
            "recipes" => PanelStyle::Viewer,
            id => self.styles.get(id).copied().unwrap_or_default(),
        }
    }
    /// Whether `owner`'s panel draws square page buttons and the viewer's
    /// search box: the viewer's look or the classic one.
    fn viewer_style(&self, owner: &Owner) -> bool {
        !matches!(self.style(owner), PanelStyle::RecipeBook)
    }
    /// The classic look's settings (`PanelStyle::Classic`), if `owner`
    /// asked for it.
    fn classic(&self, owner: &Owner) -> Option<(bool, u16)> {
        match self.style(owner) {
            PanelStyle::Classic {
                centred_search,
                window_width,
            } => Some((centred_search, window_width)),
            _ => None,
        }
    }
    /// What the panel shows: a native mod's, one `on`.
    pub(in crate::game) fn shown(&self, on: impl Fn(&str) -> bool) -> Option<&Shown> {
        self.native.as_ref().filter(|s| on(s.owner.native_id()))
    }
    /// `panel_items`.
    pub(in crate::game) fn show_items(&mut self, mod_id: &str, mut items: Vec<BrowserStack>) {
        items.truncate(ITEMS);
        if let Some(shown) = self.native_of(mod_id, !items.is_empty()) {
            shown.items = items;
            // An empty search result must leave a field to edit the query.
            self.native_changed(self.searching && !self.sent.is_empty());
        }
    }
    /// `native_panel_recipes`.
    pub(in crate::game) fn show_recipes(&mut self, mod_id: &str, page: Option<RecipePage>) {
        let filtering = mod_id == "recipes" && !self.sent.is_empty();
        if let Some(shown) = self.native_of(mod_id, page.is_some()) {
            let returning_to_search = filtering && shown.recipes.is_some() && page.is_none();
            shown.recipes = page;
            self.native_changed(returning_to_search);
        }
    }
    /// Mod `mod_id`'s panel, made when it shows something on another's;
    /// none when it clears a panel that isn't its own.
    fn native_of(&mut self, mod_id: &str, showing: bool) -> Option<&mut Shown> {
        let own = self
            .native
            .as_ref()
            .is_some_and(|s| s.owner.native_id() == mod_id);
        if !own {
            if !showing {
                return None;
            }
            self.native = Some(Shown {
                owner: Owner::Native(mod_id.to_owned()),
                version: 0,
                items: Vec::new(),
                recipes: None,
            });
        }
        self.native.as_mut()
    }
    /// A native panel changed: a new version, and no panel once it shows
    /// nothing, except while filtering the item list with a non-empty query.
    fn native_changed(&mut self, keep_search: bool) {
        self.versions += 1;
        self.transfer_warning = None;
        if let Some(shown) = self.native.as_mut() {
            shown.version = self.versions;
            if shown.items.is_empty() && shown.recipes.is_none() && !keep_search {
                self.native = None;
            }
        }
        // A queued search text is its owner's alone: gone with its panel.
        let owner = self.native.as_ref().map(|s| &s.owner);
        if self.pending.as_ref().is_some_and(|(o, _)| Some(o) != owner) {
            self.pending = None;
        }
    }
}

/// Where the panel's parts are this frame.
pub(in crate::game) struct Layout {
    /// The panel's whole area, for clicks that hit none of its parts.
    pub(in crate::game) area: [f32; 4],
    /// The grid's top left corner, its columns and rows.
    pub(in crate::game) grid: (f32, f32),
    pub(in crate::game) columns: usize,
    pub(in crate::game) rows: usize,
    /// The page arrows' row.
    pub(in crate::game) arrows_y: f32,
    pub(in crate::game) search: [f32; 4],
    settings: Option<[f32; 4]>,
    /// The classic look: its page buttons always show and wrap around.
    classic: bool,
    /// The page row's left edge and width: the grid's, or JEI's least 80
    /// pixels flush right when the grid is narrower (the classic look).
    nav: (f32, f32),
}
impl Layout {
    fn browser_arrow_rects(&self, square: bool) -> [[f32; 4]; 2] {
        if square {
            let (x, w) = self.nav;
            return [
                [x, self.arrows_y, 20.0, 20.0],
                [x + w - 20.0, self.arrows_y, 20.0, 20.0],
            ];
        }
        self.arrows().map(|(x, y)| [x, y, 12.0, 17.0])
    }

    fn per_page(&self) -> usize {
        self.columns * self.rows
    }
    pub(in crate::game::client_side) fn width(&self) -> f32 {
        self.columns as f32 * SLOT
    }
    fn slot(&self, n: usize) -> (f32, f32) {
        (
            self.grid.0 + (n % self.columns) as f32 * SLOT,
            self.grid.1 + (n / self.columns) as f32 * SLOT,
        )
    }
    /// The page arrows: backward, then forward.
    pub(in crate::game::client_side) fn arrows(&self) -> [(f32, f32); 2] {
        [
            (self.grid.0, self.arrows_y),
            (self.grid.0 + self.width() - 12.0, self.arrows_y),
        ]
    }
}

/// The classic look's parts (`PanelStyle::Classic`, JEI 1.12's
/// `IngredientListOverlay`): the items from 2 pixels right of `window` (or of
/// a window `window_width` wide centred on the screen, when that is wider) to
/// 2 from the screen's edge, flush right and centred in their height, under a
/// 20 pixel row of page buttons; the search box 20 tall at the foot of the
/// screen, under the window when `centred` and there is room below it, else
/// under the items, with the 20 pixel settings button on its right. None
/// without room for two columns or a row.
fn classic_layout(
    screen: [f32; 2],
    window: [f32; 4],
    window_width: f32,
    centred: bool,
) -> Option<Layout> {
    let [width, height] = screen;
    let window = if window_width > window[2] {
        let x = ((width - window_width) / 2.0).floor();
        [x, window[1], window_width, window[3]]
    } else {
        window
    };
    let left = window[0] + window[2] + CLASSIC_PAD;
    let right = width - CLASSIC_PAD;
    let columns = (((right - left) / SLOT).floor().max(0.0) as usize).min(COLUMNS.0);
    if columns < COLUMNS.1 {
        return None;
    }
    let grid_width = columns as f32 * SLOT;
    // JEI keeps the list 80 pixels wide at the least, the grid centred in it.
    let list_width = grid_width.max(4.0 * CLASSIC_BUTTON);
    let list_x = right - list_width;
    let x = list_x + ((list_width - grid_width) / 2.0).floor();
    let centred = centred && window[1] + window[3] + CLASSIC_BUTTON < height;
    let foot = height
        - CLASSIC_BUTTON
        - if centred {
            CLASSIC_PAD
        } else {
            2.0 * CLASSIC_PAD
        };
    let (search_x, search_width) = if centred {
        (window[0], window[2])
    } else {
        (list_x, list_width)
    };
    let search = [
        search_x,
        foot,
        search_width - CLASSIC_BUTTON + 1.0,
        CLASSIC_BUTTON,
    ];
    let settings = [
        search[0] + search[2] - 1.0,
        foot,
        CLASSIC_BUTTON,
        CLASSIC_BUTTON,
    ];
    let top = CLASSIC_PAD + CLASSIC_BUTTON;
    let bottom = if centred {
        height - CLASSIC_PAD
    } else {
        foot - CLASSIC_PAD
    };
    let rows = ((bottom - top) / SLOT).floor().max(0.0) as usize;
    let y = top + ((bottom - top - rows as f32 * SLOT) / 2.0).floor();
    (rows > 0).then_some(Layout {
        // The grid can start left of `left` when it is centred in the
        // minimum-width page row, so the area reaches back to cover it.
        area: [left.min(x), 0.0, right - left.min(x), height],
        grid: (x, y),
        columns,
        rows,
        arrows_y: CLASSIC_PAD,
        search,
        settings: Some(settings),
        classic: true,
        nav: (list_x, list_width),
    })
}

/// JEI's tint of a button's icon (`GuiIconButton`): light grey, darker on a
/// button that is off, pale yellow under the pointer.
fn icon_tint(on: bool, hovered: bool) -> u32 {
    match (on, hovered) {
        (false, _) => rgba(0xA0, 0xA0, 0xA0, 0xFF),
        (true, true) => rgba(0xFF, 0xFF, 0xA0, 0xFF),
        (true, false) => rgba(0xE0, 0xE0, 0xE0, 0xFF),
    }
}

pub(in crate::game::client_side) fn inside(p: (f32, f32), [x, y, w, h]: [f32; 4]) -> bool {
    p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h
}

/// A wheel notch `d` (positive is up, as winit's) with the pointer at
/// `cursor`: the page of `pages` the panel in `area` turns to from `page`,
/// one per notch, down forward and up back, stopping at both ends (it never
/// wraps, even in the classic look); `None` when the pointer is off the
/// panel, and the screen underneath hears the notch.
pub(in crate::game) fn wheel_turn(
    cursor: (f32, f32),
    area: [f32; 4],
    page: usize,
    pages: usize,
    d: f32,
) -> Option<usize> {
    if !inside(cursor, area) {
        return None;
    }
    let last = pages.saturating_sub(1);
    let page = page.min(last);
    Some(if d < 0.0 {
        (page + 1).min(last)
    } else if d > 0.0 {
        page.saturating_sub(1)
    } else {
        page
    })
}

/// The panel's search box before it is first laid out.
fn new_search_box() -> EditBox {
    let mut edit = EditBox::new(0.0, 0.0, 0.0, 0.0);
    edit.max_length = SEARCH_LENGTH;
    edit.focused = false;
    edit
}

impl crate::game::ClientGame<'_> {
    /// How mod `mod_id`'s panel draws its page buttons and search box from
    /// now on (`PanelStyle`; the recipe book's until it says). Kept for
    /// the session, whether or not its panel is showing.
    pub(in crate::game) fn panel_style(&mut self, mod_id: &str, style: PanelStyle) {
        self.client.panel.set_style(mod_id, style);
    }
    /// `panel_search`: the text in mod `mod_id`'s search box, heard as the
    /// player's typing is when its box next draws. Only the mod whose hook or
    /// part is running and owns the panel can set it.
    pub(in crate::game) fn panel_search(&mut self, mod_id: &str, text: &str) {
        let caller = mod_api::content::crash::current_native_mod();
        self.client.panel.set_search(caller, mod_id, text);
    }
    fn browser_batch(&self) -> Option<HudBatch> {
        let r = self.client.renderer.0.as_ref()?;
        Some(HudBatch::new(
            [r.config.width, r.config.height],
            self.client.settings.gui_scale_inputs(),
        ))
    }
    /// The mod showing a panel, with its items made into stacks once per
    /// change; none while no mod shows one or no container screen is up.
    fn browser_sync(&mut self) -> Option<Owner> {
        if !matches!(self.client.screen, Screen::Inventory | Screen::Creative)
            || self.client.title.0.is_some()
        {
            return None;
        }
        let Some(shown) = self.client.panel.shown(|id| {
            !self
                .common
                .mods
                .code_disabled(&self.client.settings.mods.disabled, id)
        }) else {
            self.client.panel.of = None;
            self.client.panel.stacks.clear();
            return None;
        };
        let of = (shown.owner.clone(), shown.version);
        if self.client.screen == Screen::Creative && of.0.native_id() != "recipes" {
            return None;
        }
        if self.client.panel.of.as_ref() != Some(&of) {
            let items = self
                .common
                .server_items
                .as_deref()
                .unwrap_or_else(|| self.common.res.items());
            let stacks = shown
                .items
                .iter()
                .map(|s| browser_stack(items, s))
                .collect();
            let shown = shown.recipes.clone();
            self.client.panel.stacks = stacks;
            // The same title again (the mod updating its buttons) keeps the page.
            let old = self.client.panel.recipes.take();
            self.client.panel.recipes = shown
                .as_ref()
                .map(|shown| super::native_panel_recipes::Pages::new(items, shown, old));
            self.client.panel.of = Some(of.clone());
        }
        Some(of.0)
    }
    /// What the panel shows now.
    fn panel_shown(&self) -> Option<&Shown> {
        self.client.panel.shown(|id| {
            !self
                .common
                .mods
                .code_disabled(&self.client.settings.mods.disabled, id)
        })
    }
    /// The player did `event` on `owner`'s panel.
    pub(in crate::game::client_side) fn panel_event(&mut self, owner: &Owner, event: BrowserEvent) {
        let Owner::Native(id) = owner;
        self.native_panel_event(id, &event);
    }
    /// The panel's parts: right of the window, from the top of the screen to
    /// the search box at its foot. None when the screen has no room for it.
    fn browser_layout(&self, batch: &HudBatch) -> Option<Layout> {
        let image = self.container_image(batch);
        let recipes = self
            .client
            .panel
            .of
            .as_ref()
            .is_some_and(|(owner, _)| owner.native_id() == "recipes");
        let window_right = if self.client.screen == Screen::Creative {
            Self::creative_origin(batch).0 + 198.0
        } else {
            (image.left + image.width) as f32
        };
        let classic = self
            .client
            .panel
            .of
            .as_ref()
            .and_then(|(owner, _)| self.client.panel.classic(owner));
        if let Some((centred, window_width)) = classic.filter(|_| !recipes) {
            let (left, top) = (image.left as f32, image.top as f32);
            let window = [left, top, window_right - left, image.height as f32];
            let screen = [batch.width, batch.height];
            return classic_layout(screen, window, f32::from(window_width), centred);
        }
        let left = window_right + MARGIN;
        let (width, height) = self.client.renderer.0.as_ref().filter(|_| recipes).map_or(
            (batch.width, batch.height),
            |renderer| {
                (
                    (renderer.config.width as f32 / batch.scale).floor(),
                    (renderer.config.height as f32 / batch.scale).floor(),
                )
            },
        );
        let right = width - if recipes { 6.0 } else { MARGIN };
        let columns = (((right - left) / SLOT).floor().max(0.0) as usize).min(if recipes {
            9
        } else {
            COLUMNS.0
        });
        if columns < COLUMNS.1 {
            return None;
        }
        let grid_width = columns as f32 * SLOT;
        let x = if recipes {
            right - grid_width
        } else {
            (left + (right - left - grid_width) / 2.0).floor()
        };
        let arrows_y = if recipes { 6.0 } else { MARGIN };
        let top = arrows_y + if recipes { 22.0 } else { 21.0 };
        let mut search = if recipes {
            [x, height - 26.0, grid_width, 20.0]
        } else {
            [x, batch.height - MARGIN - 16.0, grid_width, 16.0]
        };
        let settings = recipes.then(|| {
            search[2] -= RECIPES_SETTINGS_WIDTH;
            [
                search[0] + search[2],
                search[1],
                RECIPES_SETTINGS_WIDTH,
                search[3],
            ]
        });
        let rows = ((search[1] - 4.0 - top) / SLOT).floor().max(0.0) as usize;
        (rows > 0).then_some(Layout {
            area: if recipes {
                [x, 0.0, right - x, batch.height]
            } else {
                [left, 0.0, right - left, batch.height]
            },
            grid: (x, top),
            columns,
            rows: if recipes { rows.min(16) } else { rows },
            arrows_y,
            search,
            settings,
            classic: false,
            nav: (x, grid_width),
        })
    }
    fn browser_pages(&self, layout: &Layout) -> usize {
        self.client
            .panel
            .stacks
            .len()
            .div_ceil(layout.per_page())
            .max(1)
    }
    /// The stack under the pointer on the panel, an item's or a recipe's,
    /// as the mod sent it and as a stack.
    fn browser_pointed_stack(&self, layout: &Layout) -> Option<(BrowserStack, ItemStack)> {
        if self.client.panel.recipes.is_some() {
            if let Some((_, sent, stack)) = self.recipe_tab_hovered(layout) {
                return Some((sent, stack));
            }
            let (_, _, sent, stack) = self.recipe_slot_hovered(layout)?;
            return Some((sent, stack));
        }
        let (index, stack) = self.browser_hovered(layout)?;
        let items = &self.panel_shown()?.items;
        Some((items.get(index)?.clone(), stack))
    }
    /// The panel's item under the pointer: its index and stack.
    fn browser_hovered(&self, layout: &Layout) -> Option<(usize, ItemStack)> {
        if self.client.panel.recipes.is_some() {
            return None;
        }
        let first = self.client.panel.page * layout.per_page();
        (0..layout.per_page()).find_map(|n| {
            let (x, y) = layout.slot(n);
            if !inside(self.client.cursor, [x, y, SLOT, SLOT]) {
                return None;
            }
            let stack = self.client.panel.stacks.get(first + n)?.clone()?;
            Some((first + n, stack))
        })
    }
    fn browser_search(&mut self, layout: &Layout) -> &mut EditBox {
        let search = self.client.panel.search.get_or_insert_with(new_search_box);
        [search.x, search.y, search.width, search.height] = layout.search;
        search
    }

    fn draw_item_browser_arrows(
        &self,
        batch: &mut HudBatch,
        rects: [[f32; 4]; 2],
        labels: &[String; 2],
        enabled: [bool; 2],
        square: bool,
        classic: bool,
    ) {
        let atlas = &self.client.gui.atlas;
        for (i, (rect @ [x, y, w, h], on)) in rects.into_iter().zip(enabled).enumerate() {
            let hovered = inside(self.client.cursor, rect);
            if square {
                let background = match (on, hovered) {
                    (false, _) => "sunrise/jei/button_disabled_v2",
                    (true, false) => "sunrise/jei/button_enabled_v2",
                    (true, true) => "sunrise/jei/button_highlight_v2",
                };
                sunrise_client::ui::nine_slice(batch, atlas, background, x, y, w, h, u32::MAX);
                let arrow = [
                    "sunrise/jei/icons/arrow_previous_v2",
                    "sunrise/jei/icons/arrow_next_v2",
                ][i];
                // The classic look tints its icons as JEI: grey, darker when
                // off, yellow under the pointer.
                let tint = if classic {
                    icon_tint(on, hovered)
                } else {
                    u32::MAX
                };
                batch.sprite_scaled(atlas, arrow, x + 5.0, y + 6.0, 9.0, 7.0, tint);
            } else if on {
                let name = ["page_backward", "page_forward"][i];
                let suffix = if hovered { "_highlighted" } else { "" };
                batch.sprite(atlas, &format!("recipe_book/{name}{suffix}"), x, y);
            } else {
                continue;
            }
            sunrise_client::widgets::record(&labels[i], rect, on);
        }
    }

    /// The items' page arrows and grid.
    fn draw_browser_items(&mut self, batch: &mut HudBatch, layout: &Layout) {
        let classic = layout.classic;
        let pages = self.browser_pages(layout);
        self.client.panel.page = self.client.panel.page.min(pages - 1);
        let page = self.client.panel.page;
        let cursor = self.client.cursor;
        let square = self
            .client
            .panel
            .of
            .as_ref()
            .is_some_and(|(owner, _)| self.client.panel.viewer_style(owner));
        if pages > 1 || (square && !self.client.panel.stacks.is_empty()) || classic {
            let labels = [
                self.translate_with_fallback("gui.recipebook.previous_page", "Previous Page"),
                self.translate_with_fallback("gui.recipebook.next_page", "Next Page"),
            ];
            // The classic buttons wrap around, so both work with two pages.
            let shown = if classic {
                [pages > 1; 2]
            } else {
                [page > 0, page + 1 < pages]
            };
            let atlas = &self.client.gui.atlas;
            if square && !classic {
                batch.fill(
                    atlas,
                    layout.grid.0,
                    layout.arrows_y,
                    layout.width(),
                    20.0,
                    rgba(10, 14, 20, 42),
                );
            }
            self.draw_item_browser_arrows(
                batch,
                layout.browser_arrow_rects(square),
                &labels,
                shown,
                square,
                classic,
            );
            let text = format!("{}/{pages}", page + 1);
            let w = HudBatch::text_width(atlas, &text);
            let x = (layout.nav.0 + (layout.nav.1 - w) / 2.0).floor();
            batch.text(
                atlas,
                &text,
                x,
                layout.arrows_y + if square { 6.0 } else { 5.0 },
                u32::MAX,
                true,
            );
        }
        let first = page * layout.per_page();
        for n in 0..layout.per_page() {
            let Some(stack) = self.client.panel.stacks.get(first + n) else {
                break;
            };
            let Some(stack) = stack.clone() else {
                continue;
            };
            let (x, y) = layout.slot(n);
            self.draw_item(batch, &stack, x + 1.0, y + 1.0);
            let rect = [x + 1.0, y + 1.0, 16.0, 16.0];
            if inside(cursor, [x, y, SLOT, SLOT]) {
                // The classic look lights the whole slot, as JEI 1.12.
                let lit = if classic { [x, y, SLOT, SLOT] } else { rect };
                batch.fill(
                    &self.client.gui.atlas,
                    lit[0],
                    lit[1],
                    lit[2],
                    lit[3],
                    rgba(255, 255, 255, 128),
                );
            }
            let name = self.common.pack.names.display_item(&stack.item);
            sunrise_client::widgets::record_as(
                &name,
                rect,
                true,
                sunrise_client::widgets::Kind::Button,
            );
        }
    }
    /// Draw the browser's contents once, leaving its tooltip for the screen's
    /// overlay phase. The carried stack belongs to the enclosing screen.
    pub fn draw_mod_browser(&mut self, batch: &mut HudBatch) -> Option<BrowserTooltip> {
        let owner = self.browser_sync()?;
        let mut layout = self.browser_layout(batch)?;
        // A search text the owner set (`panel_search`) goes into its box only
        // now, with room for the box, and is heard through typing's own path,
        // outside the mod's call; the list it sends back is drawn this frame.
        if self.client.panel.deliver_search(&owner) {
            self.browser_searched();
            self.browser_sync()?;
            layout = self.browser_layout(batch)?;
        }
        let cursor = self.client.cursor;
        if self.client.panel.recipes.is_some() {
            self.draw_browser_recipes(batch, &layout);
        } else {
            self.draw_browser_items(batch, &layout);
        }
        let hint = self.translate_with_fallback("gui.recipebook.search_hint", "Search...");
        let skin = self
            .client
            .panel
            .of
            .as_ref()
            .is_some_and(|(owner, _)| self.client.panel.viewer_style(owner))
            .then_some("sunrise/jei/search_background_v2");
        let nothing_found = self.client.panel.stacks.is_empty();
        let search = self.browser_search(&layout);
        // The hint shows in an empty box without focus, as the recipe book's.
        search.suggestion = (search.value.is_empty() && !search.focused).then_some(hint);
        // Reset every frame, including when leaving the classic style.
        search.text_color = if layout.classic {
            if nothing_found && !search.value.is_empty() {
                rgba(255, 0, 0, 255)
            } else {
                rgba(255, 255, 255, 255)
            }
        } else {
            rgba(224, 224, 224, 255)
        };
        let mut search = search.clone();
        search.draw_with_skin(batch, &self.client.gui.atlas, None, skin);
        self.client.panel.search = Some(search);
        let mut settings_label = None;
        if let Some(rect @ [x, y, width, height]) = layout.settings {
            let hovered = inside(cursor, rect);
            let atlas = &self.client.gui.atlas;
            if layout.classic {
                // JEI's config button: its wrench on its own button.
                let sprite = if hovered {
                    "sunrise/jei/button_highlight_v2"
                } else {
                    "sunrise/jei/button_enabled_v2"
                };
                sunrise_client::ui::nine_slice(batch, atlas, sprite, x, y, width, height, u32::MAX);
                let icon = "sunrise/jei/icons/config_button";
                batch.sprite_scaled(atlas, icon, x + 2.0, y + 2.0, 16.0, 16.0, u32::MAX);
            } else {
                let sprite = if hovered {
                    "widget/button_highlighted"
                } else {
                    "widget/button"
                };
                sunrise_client::ui::nine_slice(batch, atlas, sprite, x, y, width, height, u32::MAX);
                batch.sprite_scaled(
                    atlas,
                    "sunrise/recipes/settings",
                    x + 4.0,
                    y + 2.0,
                    12.0,
                    12.0,
                    u32::MAX,
                );
            }
            let label = sunrise_game_data::lang::get("options.title");
            sunrise_client::widgets::record(&label, rect, true);
            if hovered && self.common.carried.is_none() {
                settings_label = Some(label);
            }
        }
        // The dragged item goes over everything, the settings button's label too.
        let menu = self.client.screen_state.0.screen_instance;
        let dragged = (self.client.panel.of.as_ref())
            .and_then(|(owner, _)| self.client.panel.dragged(owner, menu))
            .cloned();
        browser_overlay(dragged, settings_label, || {
            if self.common.carried.is_some() {
                None
            } else if let Some(stack) = self.browser_pointed_stack(&layout) {
                Some(BrowserTooltip::Stack(stack.1))
            } else {
                self.recipe_fluid_hovered(&layout)
                    .map(BrowserTooltip::Fluid)
            }
        })
    }

    pub(in crate::game::client_side) fn draw_mod_browser_tooltip(
        &mut self,
        batch: &mut HudBatch,
        tooltip: BrowserTooltip,
    ) {
        match tooltip {
            BrowserTooltip::Text(text) => {
                self.draw_tooltip_lines(
                    batch,
                    &[(text, u32::MAX)],
                    self.client.cursor.0,
                    self.client.cursor.1,
                );
            }
            BrowserTooltip::Stack(stack) => {
                self.draw_stack_tooltip(batch, &stack, self.client.cursor.0, self.client.cursor.1);
            }
            BrowserTooltip::Fluid(fluid) => {
                self.draw_fluid_tooltip(batch, &fluid, self.client.cursor.0, self.client.cursor.1);
            }
            BrowserTooltip::Dragged(stack) => {
                // Under the pointer, as the carried stack is drawn.
                self.draw_item(
                    batch,
                    &stack,
                    self.client.cursor.0 - 8.0,
                    self.client.cursor.1 - 8.0,
                );
            }
        }
    }

    /// A mouse press on a container screen: true when the panel took it
    /// (its search box, page arrows, items or empty space; a click there
    /// never throws the carried stack).
    pub fn mod_browser_press(&mut self, right: bool) -> bool {
        // A left press whose release never came (the pointer left the window)
        // ends here; a right press cancels the held one.
        self.client.panel.other_press(!right);
        let Some(owner) = self.browser_sync() else {
            return false;
        };
        let Some(batch) = self.browser_batch() else {
            return false;
        };
        let Some(layout) = self.browser_layout(&batch) else {
            return false;
        };
        let cursor = self.client.cursor;
        if layout.settings.is_some_and(|rect| inside(cursor, rect)) {
            if !right {
                self.list_native_mods();
                if let Some(index) = self
                    .client
                    .settings
                    .mods
                    .installed
                    .iter()
                    .position(|entry| entry.id == owner.native_id() && entry.native.is_some())
                {
                    self.mod_browser_closed();
                    self.client.panel.recipes_settings_return = Some(self.client.screen);
                    self.open_options_at(false, sunrise_client::options_ui::Page::NativeMod(index));
                }
            }
            return true;
        }
        let shift = self.has_shift_down();
        let atlas = &self.client.gui.atlas;
        let search = self.client.panel.search.get_or_insert_with(|| {
            let mut edit = EditBox::new(0.0, 0.0, 0.0, 0.0);
            edit.max_length = 50;
            edit
        });
        [search.x, search.y, search.width, search.height] = layout.search;
        if inside(cursor, layout.search) {
            search.focused = true;
            search.click(atlas, cursor.0, shift, false);
            search.reset_blink();
            return true;
        }
        search.focused = false;
        if self.client.panel.recipes.is_some() {
            return self.browser_recipes_press(&layout, &owner, right)
                || inside(cursor, layout.area);
        }
        let pages = self.browser_pages(&layout);
        let page = self.client.panel.page;
        let [back, forward] = layout.browser_arrow_rects(self.client.panel.viewer_style(&owner));
        let step = if pages > 1 && page > 0 && inside(cursor, back) {
            Some(page - 1)
        } else if pages > 1 && page + 1 < pages && inside(cursor, forward) {
            Some(page + 1)
        } else if layout.classic && pages > 1 && inside(cursor, back) {
            // The classic buttons wrap around, as JEI's.
            Some(pages - 1)
        } else if layout.classic && pages > 1 && inside(cursor, forward) {
            Some(0)
        } else {
            None
        };
        if let Some(to) = step {
            self.client.panel.page = to;
            self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
            return true;
        }
        if let Some((index, stack)) = self.browser_hovered(&layout) {
            // A left press with nothing carried waits for its release: a
            // click, or once the pointer moves away a drag of the item.
            let sent = (self.panel_shown()).and_then(|shown| shown.items.get(index).cloned());
            if !right
                && self.common.carried.is_none()
                && let Some(sent) = sent
            {
                self.client.panel.held = Some(Held {
                    owner,
                    index,
                    sent,
                    stack,
                    from: cursor,
                    dragging: false,
                    menu: self.client.screen_state.0.screen_instance,
                    cancelled: false,
                });
                return true;
            }
            let event = BrowserEvent::Clicked {
                index: index as u32,
                right,
            };
            self.panel_event(&owner, event);
            return true;
        }
        inside(cursor, layout.area)
    }

    /// The pointer moved with a press held on a panel item: it becomes a
    /// drag once it is `DRAG_THRESHOLD` away, and is cancelled once its menu
    /// is no longer the open one. True while a press is held: neither the
    /// menu nor its slots hear the move.
    pub(in crate::game::client_side) fn mod_browser_drag_move(&mut self) -> bool {
        let menu = self.client.screen_state.0.screen_instance;
        self.client.panel.press_moved(self.client.cursor, menu)
    }

    /// A left release: true when it ends a press on a panel item (`Held`),
    /// which then clicks the item, drops it on the screen
    /// (`PanelEvent::Dropped` to the panel's owner) or does nothing (`Now`:
    /// a cancelled press, its menu closed or replaced, the panel gone or
    /// changed hands, or the item no longer at its index). The release is
    /// the panel's either way: no screen hears it, no slot click, no throw,
    /// no quick craft.
    pub(in crate::game::client_side) fn mod_browser_release(&mut self) -> bool {
        if self.client.panel.held.is_none() {
            return false;
        }
        let owner = self.browser_sync();
        let on_panel = (self.browser_batch())
            .and_then(|batch| self.browser_layout(&batch))
            .map(|layout| inside(self.client.cursor, layout.area));
        let Some(held) = self.client.panel.held.take() else {
            return true;
        };
        let at_index = (self.panel_shown()).and_then(|shown| shown.items.get(held.index).cloned());
        let now = Now {
            menu: self.client.screen_state.0.screen_instance,
            owner: owner.as_ref(),
            on_panel,
            at_index: at_index.as_ref(),
        };
        let event = match held.release(self.client.cursor, now) {
            Release::Click(index) => BrowserEvent::Clicked {
                index: index as u32,
                right: false,
            },
            Release::Drop(stack, (x, y)) => BrowserEvent::Dropped { stack, x, y },
            Release::Cancel => return true,
        };
        if let Some(owner) = owner {
            self.panel_event(&owner, event);
        }
        true
    }

    /// A mod's screen hook vetoed the left release of a held press: the
    /// dragged item goes away and nothing is sent.
    pub(in crate::game::client_side) fn mod_browser_release_vetoed(&mut self) {
        self.client.panel.release_vetoed();
    }

    /// A button went down (`left` or another): a held press whose release
    /// never came is forgotten, or cancelled by another button.
    pub(in crate::game::client_side) fn mod_browser_other_press(&mut self, left: bool) {
        self.client.panel.other_press(left);
    }

    /// Escape: cancels a held press. True when that was a drag over its own
    /// menu, and the screen stays open.
    pub(in crate::game::client_side) fn mod_browser_drag_escape(&mut self) -> bool {
        let dragging = self.mod_browser_dragging();
        self.client.panel.cancel_press();
        dragging
    }

    /// The window lost the focus: a held press is cancelled.
    pub(in crate::game::client_side) fn mod_browser_focus_lost(&mut self) {
        self.client.panel.cancel_press();
    }

    /// Whether an item is being dragged off the panel over the menu it was
    /// pressed on: the screen's slots show no tooltip under it.
    pub(in crate::game::client_side) fn mod_browser_dragging(&self) -> bool {
        let menu = self.client.screen_state.0.screen_instance;
        (self.client.panel.held.as_ref()).is_some_and(|held| held.live_on(menu) && held.dragging)
    }

    /// A wheel notch `d` on a container screen (positive is up): true when
    /// the pointer is over the panel, which turns one page (`wheel_turn`;
    /// its recipe pages too) and keeps the notch from the screen
    /// underneath. With no panel shown, `browser_sync` returns at once.
    pub(in crate::game::client_side) fn mod_browser_wheel(&mut self, d: f32) -> bool {
        let Some(owner) = self.browser_sync() else {
            return false;
        };
        let Some(batch) = self.browser_batch() else {
            return false;
        };
        let Some(layout) = self.browser_layout(&batch) else {
            return false;
        };
        if self.client.panel.recipes.is_some() {
            return self.browser_recipes_wheel(&layout, &owner, d);
        }
        let pages = self.browser_pages(&layout);
        let Some(to) = wheel_turn(
            self.client.cursor,
            layout.area,
            self.client.panel.page,
            pages,
            d,
        ) else {
            return false;
        };
        self.client.panel.page = to;
        true
    }

    /// Whether keys go to the panel's search box.
    pub fn mod_browser_typing(&self) -> bool {
        matches!(self.client.screen, Screen::Inventory | Screen::Creative)
            && self.client.panel.of.is_some()
            && self.client.panel.search.as_ref().is_some_and(|s| s.focused)
    }
    /// A key press while the search box has focus.
    pub(in crate::game::client_side) fn mod_browser_key(
        &mut self,
        key: winit::keyboard::KeyCode,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
    ) -> bool {
        let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
        let Some(search) = self.client.panel.search.as_mut() else {
            return false;
        };
        let handled = if let Some(shortcut) = shortcut {
            search.shortcut(shortcut, &mut self.client.clipboard.0);
            true
        } else {
            search.key(key, ctrl, shift)
        };
        search.reset_blink();
        self.browser_searched();
        handled
    }
    /// Text typed (or one backspace) into the search box (`sunrise-ctl type`).
    pub fn mod_browser_text(&mut self, text: Option<&str>, backspace: bool) {
        let Some(search) = self.client.panel.search.as_mut() else {
            return;
        };
        if backspace {
            search.delete_chars(-1);
        } else if let Some(text) = text {
            search.insert_text(text);
        }
        self.browser_searched();
    }
    /// The mod hears the search text when it changed.
    fn browser_searched(&mut self) {
        let (Some((owner, _)), Some(search)) = (&self.client.panel.of, &self.client.panel.search)
        else {
            return;
        };
        if search.value != self.client.panel.sent {
            let (owner, value) = (owner.clone(), search.value.clone());
            // The player's edit is newer than a text the owner queued.
            if matches!(&self.client.panel.pending, Some((o, _)) if *o == owner) {
                self.client.panel.pending = None;
            }
            self.client.panel.sent.clone_from(&value);
            self.client.panel.searching = true;
            self.client.panel.page = 0;
            self.panel_event(&owner, BrowserEvent::Search(value));
            self.client.panel.searching = false;
        }
    }
    /// The container screen closed: the search box lets go of the keys.
    pub fn mod_browser_closed(&mut self) {
        self.client.panel.transfer_warning = None;
        // Closing the screen cancels a press on the panel; its release stays
        // the panel's.
        self.client.panel.cancel_press();
        if let Some(search) = self.client.panel.search.as_mut() {
            search.focused = false;
        }
    }
    pub(in crate::game::client_side) fn mod_browser_back(&mut self) -> bool {
        let Some(owner) = self.browser_sync() else {
            return false;
        };
        if owner.native_id() != "recipes" || self.client.panel.recipes.is_none() {
            return false;
        }
        self.mod_browser_closed();
        self.panel_event(&owner, BrowserEvent::Back);
        true
    }
    pub(in crate::game::client_side) fn close_transferred_recipe(&mut self) {
        if let Some(owner) = self.browser_sync()
            && owner.native_id() == "recipes"
            && self.client.panel.recipes.is_some()
        {
            self.mod_browser_closed();
            self.panel_event(&owner, BrowserEvent::Back);
        }
    }
    /// The item under the pointer on a container screen, for the mods' keys
    /// (the `Key` event): the panel's, else a slot's.
    pub(in crate::game::client_side) fn mod_browser_pointed(&mut self) -> Option<BrowserStack> {
        let batch = self.browser_batch()?;
        let stack = match self
            .browser_sync()
            .and_then(|_| self.browser_layout(&batch))
        {
            // The panel's stacks as the mod sent them.
            Some(layout) if inside(self.client.cursor, layout.area) => {
                return Some(self.browser_pointed_stack(&layout)?.0);
            }
            _ => {
                let slot = self.slot_at(&batch, self.client.cursor.0, self.client.cursor.1)?;
                self.slot_view(slot)?
            }
        };
        Some(BrowserStack {
            item: mod_api::__private::item_key(&stack.item)?,
            count: stack.count,
            components: super::native_recipes::components(&stack),
        })
    }
    /// A key `key` (`key.keyboard.r`) went down: over an item on a container
    /// screen, a native mod's panel hears it as a `Key` event.
    pub(in crate::game) fn native_panel_key(&mut self, key: &str) {
        if !matches!(self.client.screen, Screen::Inventory | Screen::Creative)
            || self.client.title.0.is_some()
            || self.mod_browser_typing()
        {
            return;
        }
        let Some(owner) = self.browser_sync() else {
            return;
        };
        if let Some(stack) = self.mod_browser_pointed() {
            let key = key.to_owned();
            self.panel_event(&owner, BrowserEvent::Key { key, stack });
        }
    }
}

/// A mod's item as a stack: an item the game has, its components read as
/// `/give` reads them (ones it can't read are left out).
pub(in crate::game::client_side) fn browser_stack(
    items: &sunrise_items::items::Items,
    s: &BrowserStack,
) -> Option<ItemStack> {
    let item = mod_api::__private::item_key_id(s.item)?;
    items.get(item)?;
    let (damage, extra) = if s.components.is_empty() {
        (0, None)
    } else {
        sunrise_items::item_data::command_components(&s.components).map_or((0, None), |tag| {
            sunrise_items::item_data::decode_components(Some(&tag))
        })
    };
    Some(ItemStack {
        item: item.into(),
        count: s.count.clamp(1, 99),
        damage,
        extra,
    })
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::BrowserStack;
    #[cfg(test)]
    use super::{
        BrowserTooltip, DRAG_THRESHOLD, Held, Now, Owner, Panel, RecipePage, Release,
        browser_overlay,
    };
    use mod_api::FluidStack;
    use sunrise_gameplay_common::game::native_hooks::{
        NativeRecipeLayout, NativeRecipeResult, NativeRecipeView,
    };

    #[cfg(test)]
    pub fn keys() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        mod_api::__private::resolve_items([
            "minecraft:oak_log",
            "minecraft:oak_planks",
            "minecraft:stone",
            "minecraft:dirt",
            "minecraft:crafting_table",
            "recipe_pilot:pressed_fragment",
        ])
        .unwrap();
        mod_api::__private::resolve_fluids(
            &["minecraft:water", "minecraft:lava"],
            &sunrise_world::fluid::FluidKind::ALL.map(|kind| kind.name()),
        )
        .unwrap();
    }
    pub fn stack(item: &str) -> BrowserStack {
        BrowserStack {
            item: mod_api::__private::item_key(item).unwrap(),
            count: 1,
            components: String::new(),
        }
    }
    fn fluid(id: &str, amount: u32) -> FluidStack {
        FluidStack::new(mod_api::__private::fluid_key(id).unwrap(), amount)
    }
    pub fn view() -> NativeRecipeView {
        NativeRecipeView {
            layout: NativeRecipeLayout::Grid,
            width: 5,
            height: 1,
            slots: vec![vec![stack("minecraft:oak_log")]; 30],
            results: (0..6)
                .map(|i| NativeRecipeResult {
                    stacks: vec![stack("minecraft:oak_planks")],
                    chance: (i == 1).then_some(250.0),
                })
                .collect(),
            fluid_inputs: vec![fluid("minecraft:water", 1000); 10],
            fluid_outputs: vec![fluid("minecraft:lava", 250)],
            note: String::new(),
            action: Some(true),
        }
    }
    /// The menu every `held` press goes down on.
    pub const MENU: u64 = 7;
    /// Each `Search` the panel's mods heard: the mod it was for and the text.
    pub static HEARD: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());
    pub struct Listener;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Listener {
        fn panel_event(
            &self,
            _: &mut dyn mod_api::ClientHost,
            event: &mut mod_api::PanelEventEvent<'_>,
        ) {
            if let mod_api::PanelEvent::Search(text) = event.event {
                let heard = (event.mod_id.to_owned(), text.clone());
                HEARD.lock().unwrap().push(heard);
            }
        }
    }
    pub fn heard(mod_id: &str, text: &str) -> Vec<(String, String)> {
        vec![(mod_id.to_owned(), text.to_owned())]
    }

    #[cfg(test)]
    /// `panel_search` as the panel sees it: only the running mod that owns
    /// the panel queues a text, and the box changes only when that owner's
    /// box draws (`deliver_search`).
    #[test]
    fn only_the_calling_owner_queues_a_search_and_its_box_takes_it() {
        keys();
        let mut panel = Panel::default();
        let jei = Owner::Native("jei".into());
        // No panel yet: nothing to set.
        assert!(!panel.set_search(Some("jei"), "jei", "iron"));
        panel.show_items("jei", vec![stack("minecraft:stone")]);

        // The caller, not the id it names: another mod naming the owner's
        // id, no running mod, and the owner naming another id are refused.
        assert!(!panel.set_search(Some("ae2"), "jei", "iron"));
        assert!(!panel.set_search(None, "jei", "iron"));
        assert!(!panel.set_search(Some("jei"), "ae2", "iron"));
        assert_eq!(panel.pending, None);

        assert!(panel.set_search(Some("jei"), "jei", "iron"));
        // Queued, not written: the box and what the mod heard are as they were.
        assert!(panel.search.is_none());
        assert_eq!(panel.sent, "");
        // Another owner's frame never takes it, and drops it.
        assert!(!panel.deliver_search(&Owner::Native("ae2".into())));
        assert_eq!(panel.pending, None);
        assert!(panel.search.is_none());

        assert!(panel.set_search(Some("jei"), "jei", "iron"));
        assert!(panel.deliver_search(&jei), "the owner's frame takes it");
        let search = panel.search.as_ref().expect("a box made for the text");
        assert_eq!((search.value.as_str(), search.cursor), ("iron", 4));
        assert!(!panel.deliver_search(&jei), "delivered once");

        // A later call replaces an earlier one.
        panel.set_search(Some("jei"), "jei", "gold");
        panel.set_search(Some("jei"), "jei", "");
        assert!(panel.deliver_search(&jei), "clearing is a change");
        assert_eq!(panel.search.as_ref().unwrap().value, "");
    }

    #[cfg(test)]
    /// Another mod taking the panel, or the owner taking its panel away,
    /// drops a queued text: it is never shown in another owner's box.
    #[test]
    fn a_queued_search_goes_with_its_owners_panel() {
        keys();
        let mut panel = Panel::default();
        panel.show_items("jei", vec![stack("minecraft:stone")]);
        panel.set_search(Some("jei"), "jei", "iron");
        // Another mod clearing its own (empty) panel changes nothing.
        panel.show_items("ae2", Vec::new());
        assert!(panel.pending.is_some());
        panel.show_items("ae2", vec![stack("minecraft:dirt")]);
        assert_eq!(panel.pending, None);
        assert!(!panel.deliver_search(&Owner::Native("ae2".into())));
        panel.show_items("jei", vec![stack("minecraft:stone")]);
        assert!(!panel.deliver_search(&Owner::Native("jei".into())));
        assert!(panel.search.is_none());

        panel.set_search(Some("jei"), "jei", "iron");
        panel.show_items("jei", Vec::new());
        assert_eq!(panel.pending, None);
    }

    #[cfg(test)]
    /// The box counts its 50 in UTF-16 code units, as vanilla's does, and a
    /// text it already holds (once cut to that) leaves its cursor and
    /// selection where the player put them.
    #[test]
    fn a_search_the_box_already_holds_leaves_the_caret_and_selection() {
        keys();
        let mut panel = Panel::default();
        let jei = Owner::Native("jei".into());
        panel.show_items("jei", vec![stack("minecraft:stone")]);
        // 1 + 2 x 25 = 51 units: the last emoji doesn't fit.
        let long = format!("a{}", "😀".repeat(25));
        panel.set_search(Some("jei"), "jei", &long);
        assert!(panel.deliver_search(&jei));
        let cut = format!("a{}", "😀".repeat(24));
        assert_eq!(panel.search.as_ref().unwrap().value, cut);

        let search = panel.search.as_mut().unwrap();
        search.set_cursor(1, false);
        search.set_cursor(3, true);
        let caret = (search.cursor, search.highlight);
        for same in [long.as_str(), cut.as_str()] {
            panel.set_search(Some("jei"), "jei", same);
            assert!(!panel.deliver_search(&jei), "no change");
            let search = panel.search.as_ref().unwrap();
            assert_eq!((search.cursor, search.highlight), caret);
            assert_eq!(search.value, cut);
        }
    }

    #[cfg(test)]
    fn held(owner: &str, index: usize) -> Held {
        Held {
            owner: Owner::Native(owner.into()),
            index,
            sent: stack("minecraft:stone"),
            stack: sunrise_items::items::ItemStack::new("minecraft:stone", 1),
            from: (100.0, 50.0),
            dragging: false,
            menu: MENU,
            cancelled: false,
        }
    }

    #[cfg(test)]
    /// The release of a `held("jei", _)` press, with its menu still open,
    /// jei's panel shown, `at_index` at the pressed index, and the pointer
    /// `on_panel` or not.
    fn release(
        press: Held,
        to: (f32, f32),
        on_panel: bool,
        at_index: Option<&BrowserStack>,
    ) -> Release {
        let jei = Owner::Native("jei".into());
        press.release(
            to,
            Now {
                menu: MENU,
                owner: Some(&jei),
                on_panel: Some(on_panel),
                at_index,
            },
        )
    }

    #[cfg(test)]
    /// A press stays a click until the pointer is `DRAG_THRESHOLD` away, in
    /// any direction; then it is a drag, even if the pointer comes back.
    #[test]
    fn a_press_drags_only_past_the_threshold() {
        keys();
        let mut press = held("jei", 3);
        let short = DRAG_THRESHOLD - 0.5;
        for to in [(100.0, 50.0), (100.0 + short, 50.0), (100.0, 50.0 - short)] {
            assert!(!press.moved(to), "{to:?} is still a click");
        }
        // 3 right and 3 down is 4.24 away.
        assert!(!held("jei", 3).moved((102.0, 52.0)));
        assert!(held("jei", 3).moved((103.0, 53.0)));
        assert!(press.moved((100.0 - DRAG_THRESHOLD, 50.0)));
        assert!(press.moved((100.0, 50.0)), "a drag stays a drag");
    }

    #[cfg(test)]
    /// The release of a press: a click where it never moved far, a drop at
    /// the release point off the panel, nothing back on it.
    #[test]
    fn a_release_clicks_drops_or_cancels() {
        keys();
        let stone = stack("minecraft:stone");
        // A click, on the panel or (a shaky hand at its edge) just off it.
        assert_eq!(
            release(held("jei", 3), (101.0, 51.0), true, Some(&stone)),
            Release::Click(3)
        );
        assert_eq!(
            release(held("jei", 3), (102.0, 50.0), false, Some(&stone)),
            Release::Click(3)
        );
        // Let go off the panel: the item, where it landed.
        let mut dragged = held("jei", 3);
        assert!(dragged.moved((60.0, 50.0)));
        assert_eq!(
            release(dragged.clone(), (40.5, 70.25), false, Some(&stone)),
            Release::Drop(stack("minecraft:stone"), (40.5, 70.25))
        );
        // Back on the panel: nothing.
        assert_eq!(
            release(dragged, (100.0, 50.0), true, Some(&stone)),
            Release::Cancel
        );
        // A release far from the press with no move between is a drag too.
        assert_eq!(
            release(held("jei", 0), (20.0, 20.0), false, Some(&stone)),
            Release::Drop(stack("minecraft:stone"), (20.0, 20.0))
        );
    }

    #[cfg(test)]
    /// A deferred click is sent only while the shown list still has the
    /// pressed item at its index; a drag drops the item captured at the
    /// press, whatever the list holds now.
    #[test]
    fn a_deferred_click_needs_its_item_still_at_its_index() {
        keys();
        let (stone, dirt) = (stack("minecraft:stone"), stack("minecraft:dirt"));
        assert_eq!(
            release(held("jei", 3), (100.0, 50.0), true, Some(&stone)),
            Release::Click(3)
        );
        // The list changed under the press: another item at 3, or none.
        assert_eq!(
            release(held("jei", 3), (100.0, 50.0), true, Some(&dirt)),
            Release::Cancel
        );
        assert_eq!(
            release(held("jei", 3), (100.0, 50.0), true, None),
            Release::Cancel
        );
        // The same item id at another count is another stack.
        let mut more = stack("minecraft:stone");
        more.count = 2;
        assert_eq!(
            release(held("jei", 3), (100.0, 50.0), true, Some(&more)),
            Release::Cancel
        );
        // A drag keeps the stone it picked up.
        let mut dragged = held("jei", 3);
        assert!(dragged.moved((60.0, 50.0)));
        assert_eq!(
            release(dragged, (30.0, 40.0), false, Some(&dirt)),
            Release::Drop(stack("minecraft:stone"), (30.0, 40.0))
        );
    }

    #[cfg(test)]
    /// A press belongs to the menu open when it went down. Escape, focus
    /// loss, a right press, and its menu closing or being replaced cancel
    /// it: it draws nothing and its release sends nothing, but the press
    /// stays held so that release is still the panel's. A vetoed release
    /// drops it with nothing sent; a new left press forgets a press whose
    /// release never came.
    #[test]
    fn a_press_is_bound_to_its_menu() {
        keys();
        let stone = stack("minecraft:stone");
        let jei = Owner::Native("jei".into());
        let at = |menu: u64, owner: Option<&'static str>, on_panel: Option<bool>| {
            let mut press = held("jei", 3);
            assert!(press.moved((60.0, 50.0)));
            let owner = owner.map(|id| Owner::Native(id.into()));
            press.release(
                (30.0, 40.0),
                Now {
                    menu,
                    owner: owner.as_ref(),
                    on_panel,
                    at_index: Some(&stack("minecraft:stone")),
                },
            )
        };
        assert_eq!(
            at(MENU, Some("jei"), Some(false)),
            Release::Drop(stack("minecraft:stone"), (30.0, 40.0))
        );
        // Released over a replacement menu, with the panel gone, owned by
        // another mod, or with no room to show it: nothing.
        assert_eq!(at(MENU + 1, Some("jei"), Some(false)), Release::Cancel);
        assert_eq!(at(MENU, None, Some(false)), Release::Cancel);
        assert_eq!(at(MENU, Some("ae2"), Some(false)), Release::Cancel);
        assert_eq!(at(MENU, Some("jei"), None), Release::Cancel);

        // Escape and focus loss: the first cancel of a drag says so (the
        // Escape is the drag's), a second finds nothing to cancel.
        for _ in ["Escape", "focus loss"] {
            let mut panel = Panel {
                held: Some(held("jei", 3)),
                ..Panel::default()
            };
            assert!(panel.press_moved((60.0, 50.0), MENU));
            assert!(panel.dragged(&jei, MENU).is_some());
            assert!(panel.cancel_press());
            assert!(!panel.cancel_press());
            assert!(panel.dragged(&jei, MENU).is_none());
            let press = panel.held.take().expect("its release is still the panel's");
            assert_eq!(
                release(press, (30.0, 40.0), false, Some(&stone)),
                Release::Cancel
            );
        }
        // A press that is not a drag yet cancels too, and leaves Escape to
        // the screen.
        let mut panel = Panel {
            held: Some(held("jei", 3)),
            ..Panel::default()
        };
        assert!(!panel.cancel_press());
        assert_eq!(
            release(
                panel.held.take().unwrap(),
                (100.0, 50.0),
                true,
                Some(&stone)
            ),
            Release::Cancel
        );

        // Its menu replaced: the first move over the new menu cancels it.
        let mut panel = Panel {
            held: Some(held("jei", 3)),
            ..Panel::default()
        };
        assert!(panel.press_moved((60.0, 50.0), MENU));
        assert!(panel.press_moved((50.0, 50.0), MENU + 1));
        assert!(panel.dragged(&jei, MENU).is_none());
        assert!(panel.dragged(&jei, MENU + 1).is_none());
        let press = panel.held.take().unwrap();
        assert!(press.cancelled);
        assert_eq!(
            release(press, (30.0, 40.0), false, Some(&stone)),
            Release::Cancel
        );
        assert!(!panel.press_moved((50.0, 50.0), MENU), "no press, no move");

        // A right press cancels it; a vetoed release drops it outright.
        let mut panel = Panel {
            held: Some(held("jei", 3)),
            ..Panel::default()
        };
        panel.press_moved((60.0, 50.0), MENU);
        panel.other_press(false);
        assert!(panel.held.as_ref().unwrap().cancelled);
        assert!(panel.dragged(&jei, MENU).is_none());
        panel.release_vetoed();
        assert!(panel.held.is_none());

        // A left press with a press still held: that release never came.
        let mut panel = Panel {
            held: Some(held("jei", 3)),
            ..Panel::default()
        };
        panel.other_press(true);
        assert!(panel.held.is_none());
    }

    #[cfg(test)]
    /// An item dragged off the list draws over the settings button: its
    /// label shows only with no drag, and the pointed item's tooltip only
    /// with neither.
    #[test]
    fn the_dragged_item_draws_over_the_settings_label() {
        keys();
        let stone = sunrise_items::items::ItemStack::new("minecraft:stone", 1);
        let label = || Some("Options".to_owned());
        let rest = || Some(BrowserTooltip::Text("Stone".into()));
        assert!(matches!(
            browser_overlay(Some(stone.clone()), label(), rest),
            Some(BrowserTooltip::Dragged(s)) if s == stone
        ));
        assert!(matches!(
            browser_overlay(None, label(), rest),
            Some(BrowserTooltip::Text(t)) if t == "Options"
        ));
        assert!(matches!(
            browser_overlay(None, None, rest),
            Some(BrowserTooltip::Text(t)) if t == "Stone"
        ));
        assert!(browser_overlay(None, None, || None).is_none());
    }

    #[cfg(test)]
    /// Only a drag on the shown panel's own owner draws its item; a press
    /// that hasn't moved draws nothing.
    #[test]
    fn the_dragged_item_is_the_owners() {
        keys();
        let mut panel = Panel::default();
        let jei = Owner::Native("jei".into());
        assert!(panel.dragged(&jei, MENU).is_none());
        panel.held = Some(held("jei", 1));
        assert!(
            panel.dragged(&jei, MENU).is_none(),
            "a press is not a drag yet"
        );
        panel.held.as_mut().unwrap().moved((0.0, 0.0));
        assert_eq!(
            panel.dragged(&jei, MENU),
            Some(&sunrise_items::items::ItemStack::new("minecraft:stone", 1))
        );
        assert!(
            panel
                .dragged(&Owner::Native("recipes".into()), MENU)
                .is_none()
        );
    }

    #[cfg(test)]
    #[test]
    fn a_mod_chooses_the_viewers_page_buttons_and_keeps_them() {
        keys();
        let mut panel = Panel::default();
        let jei = Owner::Native("jei".into());
        assert!(
            !panel.viewer_style(&jei),
            "the recipe book's arrows by default"
        );
        assert!(panel.viewer_style(&Owner::Native("recipes".into())));
        panel.set_style("jei", super::PanelStyle::Viewer);
        panel.show_items("jei", vec![stack("minecraft:stone")]);
        assert!(panel.viewer_style(&jei));
        // Another mod's choice is its own.
        assert!(!panel.viewer_style(&Owner::Native("other".into())));
        // It outlasts the panel going away and coming back.
        panel.show_items("jei", Vec::new());
        panel.show_items("jei", vec![stack("minecraft:stone")]);
        assert!(panel.viewer_style(&jei));
        panel.set_style("jei", super::PanelStyle::RecipeBook);
        assert!(!panel.viewer_style(&jei));
    }

    #[cfg(test)]
    /// JEI 1.12's overlay: the items flush right under a 20 pixel page row,
    /// the search box at the foot of the screen with the settings button on
    /// its right, under the window when asked and there is room.
    #[test]
    fn the_classic_look_lays_the_items_out_as_jei_did() {
        let mut panel = Panel::default();
        let jei = Owner::Native("jei".into());
        let style = super::PanelStyle::Classic {
            centred_search: true,
            window_width: 198,
        };
        panel.set_style("jei", style);
        assert!(
            panel.viewer_style(&jei),
            "square buttons and the search skin"
        );
        assert_eq!(panel.classic(&jei), Some((true, 198)));
        assert_eq!(panel.classic(&Owner::Native("recipes".into())), None);

        // A 570x473 GUI with the inventory's 176x166 window at (197, 153).
        let screen = [570.0, 473.0];
        let window = [197.0, 153.0, 176.0, 166.0];
        let centred = super::classic_layout(screen, window, 0.0, true).unwrap();
        // 375 to 568 holds ten columns, flush with the right edge.
        assert_eq!((centred.columns, centred.grid.0), (10, 388.0));
        // 449 pixels under the page row hold 24 rows, centred.
        assert_eq!(
            (centred.rows, centred.grid.1, centred.arrows_y),
            (24, 30.0, 2.0)
        );
        assert_eq!(centred.search, [197.0, 451.0, 157.0, 20.0]);
        assert_eq!(centred.settings, Some([353.0, 451.0, 20.0, 20.0]));
        assert_eq!(
            centred.browser_arrow_rects(true)[1],
            [548.0, 2.0, 20.0, 20.0]
        );

        // A recipe screen 198 wide over the window: the items step right of
        // it, and the search box goes under it.
        let wide = super::classic_layout(screen, window, 198.0, true).unwrap();
        assert_eq!(wide.area[0], 386.0);
        assert_eq!(wide.search, [186.0, 451.0, 179.0, 20.0]);

        // Under the items, with the rows above it.
        let beside = super::classic_layout(screen, window, 0.0, false).unwrap();
        assert_eq!(beside.search, [388.0, 449.0, 161.0, 20.0]);
        assert_eq!(beside.settings, Some([548.0, 449.0, 20.0, 20.0]));
        assert_eq!((beside.rows, beside.grid.1), (23, 27.0));

        // A window reaching the foot keeps the search box under the items.
        let tall = super::classic_layout(screen, [197.0, 153.0, 176.0, 310.0], 0.0, true).unwrap();
        assert_eq!(tall.search[0], 388.0);
        // Three columns: the page row keeps JEI's 80 pixels, the grid centred.
        let few = super::classic_layout([300.0, 473.0], [62.0, 153.0, 176.0, 166.0], 0.0, false);
        let few = few.unwrap();
        assert_eq!(
            (few.columns, few.grid.0, few.nav),
            (3, 231.0, (218.0, 80.0))
        );
        assert_eq!(few.browser_arrow_rects(true)[1], [278.0, 2.0, 20.0, 20.0]);
        // The area covers the first column's left pixels (x 231 to 239).
        assert!(few.area[0] <= few.grid.0);
        assert!(super::inside(
            (few.grid.0 + 1.0, few.grid.1 + 1.0),
            few.area
        ));
        assert_eq!(few.search, [218.0, 449.0, 61.0, 20.0]);
        // No room for two columns: no panel.
        let narrow = [37.0, 17.0, 176.0, 166.0];
        assert!(super::classic_layout([250.0, 200.0], narrow, 0.0, true).is_none());
    }

    #[cfg(test)]
    /// A native mod's items and recipe page are what the panel holds, and
    /// its owner (who hears the panel's events) is that mod.
    #[test]
    fn native_owner_holds_the_panel() {
        keys();
        let mut panel = Panel::default();
        let items = vec![stack("minecraft:stone"), stack("minecraft:dirt")];
        panel.show_items("recipes", items.clone());
        let shown = panel.shown(|_| true).expect("a panel");
        assert_eq!(shown.owner, Owner::Native("recipes".into()));
        assert_eq!(shown.items, items);
        assert_eq!(shown.recipes, None);
        let first = shown.version;

        let tabs = vec![stack("minecraft:crafting_table")];
        let page = RecipePage::new("Oak Planks", tabs.clone(), 0, vec![view()]);
        panel.show_recipes("recipes", page);
        let shown = panel.shown(|_| true).expect("a panel");
        assert_ne!(shown.version, first, "the game converts each change");
        assert_eq!(shown.items, items);
        let page = shown.recipes.as_ref().expect("a recipe page");
        assert_eq!(
            (page.title.as_str(), &page.tabs, page.tab),
            ("Oak Planks", &tabs, 0)
        );
        // Cut as `set-recipes` cuts it: a 3 x 3 grid at most, 27 slots.
        let r = &page.recipes[0];
        assert_eq!((r.width, r.height, r.slots.len()), (3, 1, 27));
        // At most four results, a chance kept to 0..=100, at most eight fluids.
        let results = r.results.iter().map(|x| (x.stacks.len(), x.chance));
        assert_eq!(
            results.collect::<Vec<_>>(),
            [(1, None), (1, Some(100.0)), (1, None), (1, None)]
        );
        assert_eq!((r.fluid_inputs.len(), r.fluid_outputs.len()), (8, 1));

        // Another mod clearing its own (empty) panel leaves this one.
        panel.show_items("other", Vec::new());
        let owner = panel.shown(|_| true).map(|s| s.owner.clone());
        assert_eq!(owner, Some(Owner::Native("recipes".into())));
        // A mod the player turned off shows nothing.
        assert!(panel.shown(|id| id != "recipes").is_none());

        // No recipes: the items again; then no items: no panel.
        panel.show_recipes("recipes", RecipePage::new("", Vec::new(), 0, Vec::new()));
        let shown = panel.shown(|_| true).expect("a panel");
        assert_eq!((shown.items.len(), shown.recipes.is_none()), (2, true));
        panel.show_items("recipes", Vec::new());
        assert!(panel.shown(|_| true).is_none());
    }

    #[cfg(test)]
    #[test]
    fn empty_search_can_be_edited_and_an_explicit_clear_still_hides_the_panel() {
        keys();
        let mut panel = Panel::default();
        let items = vec![stack("recipe_pilot:pressed_fragment")];
        panel.show_items("recipes", items.clone());
        panel.sent = "no_such_item".into();
        panel.searching = true;
        panel.show_items("recipes", Vec::new());
        panel.searching = false;
        let empty = panel
            .shown(|_| true)
            .expect("keep the search field's owner");
        assert_eq!(empty.owner, Owner::Native("recipes".into()));
        assert!(empty.items.is_empty());
        assert!(panel.shown(|id| id != "recipes").is_none());

        panel.show_recipes(
            "recipes",
            RecipePage::new("Oak Planks", Vec::new(), 0, vec![view()]),
        );
        panel.show_recipes("recipes", None);
        let empty = panel
            .shown(|_| true)
            .expect("return to the empty search after a recipe");
        assert!(empty.items.is_empty() && empty.recipes.is_none());
        assert_eq!(panel.sent, "no_such_item");

        panel.show_items("other", Vec::new());
        panel.sent = "pressed_fragment".into();
        panel.show_items("recipes", items.clone());
        assert_eq!(panel.shown(|_| true).unwrap().items, items);

        // Outside Search, clearing the page then items still closes the viewer,
        // even though the last query was non-empty.
        panel.show_recipes("recipes", None);
        panel.show_items("recipes", Vec::new());
        assert!(panel.shown(|_| true).is_none());
        panel.show_items("other", Vec::new());
        assert!(
            panel.shown(|_| true).is_none(),
            "an empty list cannot take ownership"
        );
    }

    /// `panel_search("jei", text)` through the host, as mod `caller`'s hook calls it.
    pub fn search_as(game: &mut crate::game::ClientGame<'_>, caller: &'static str, text: &str) {
        use mod_api::content::crash::{ModCall, enter_native_mod};
        let _call = enter_native_mod(ModCall {
            id: caller,
            hook: "panel_event",
            part: None,
        });
        mod_api::prelude::RecipesClientHost::panel_search(game, "jei", text);
    }
    /// One frame of the panel on a window `size` pixels big: what the mods heard.
    pub fn frame(game: &mut crate::game::ClientGame<'_>, size: [u32; 2]) -> Vec<(String, String)> {
        let mut batch =
            sunrise_client::hud::HudBatch::new(size, game.client.settings.gui_scale_inputs());
        game.draw_mod_browser(&mut batch);
        std::mem::take(&mut *HEARD.lock().unwrap())
    }
}
