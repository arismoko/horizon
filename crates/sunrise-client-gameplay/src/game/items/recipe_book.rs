//! The recipe book of the inventory, crafting table and furnace screens
//! (`RecipeBookComponent`): the green book button, the tabs by category, the
//! search box, the craftable filter, the pages of recipe buttons with the
//! alternatives overlay, ghost recipes, and placing a recipe in the grid
//! (`ServerPlaceRecipe`). Which recipes are known comes from the advancement
//! rewards; the open and filter settings are saved with the player.
#[path = "recipe_book/mod_screen.rs"]
pub(in crate::game) mod mod_screen;

use super::Screen;
use mod_api::{RecipeTransferCheck, RecipeTransferResult};
use std::{
    hash::{Hash, Hasher},
    time::{Duration, Instant},
};
use sunrise_client::{
    hud::{HudBatch, Rect, rgba},
    recipe_book_screen::{
        self as screen, ButtonView, Call, Drawn, GhostView, ItemCall, OverlayRecipe, OverlayView,
        Screen as Layout, Search, TabView,
    },
    ui::EditBox,
};
use sunrise_game_data::text::{Style, Text};
use sunrise_items::items::ItemStack;
use sunrise_level::{
    containers::{BlockEntity, FurnaceKind},
    random_source,
};
use sunrise_sim::{
    menu::place::Stacked,
    recipe_book::{Book, BookType, ClientBook, Collection, Entry, GhostSlot, GhostTarget, Tab},
};

pub(in crate::game) const WIDTH: f32 = 147.0;
pub(in crate::game) const HEIGHT: f32 = 166.0;
/// `RecipeBookComponent.TICKS_TO_SWAP_SLOT`.
pub(in crate::game) const SWAP_TICKS: f32 = 30.0;
/// Widest GUI at which the book sits next to the menu (`widthTooNarrow`).
pub(in crate::game) const NARROW: f32 = 379.0;

/// `RecipeBookComponent.isOffsetNextToMainGUI`: false in a window narrower than
/// `NARROW`, where the open book covers the menu. Escape closes only the book
/// there (`keyPressed`), not the container.
pub(in crate::game::client_side) fn escape_closes_book(width: f32, open: bool) -> bool {
    open && width < NARROW
}

/// A ghost recipe slot: the items it cycles through.
pub(in crate::game) struct Ghost {
    pub(in crate::game) slot: usize,
    pub(in crate::game) items: Vec<ItemStack>,
    pub(in crate::game) result: bool,
}
/// The alternatives overlay of a recipe button (`OverlayRecipeComponent`).
pub(in crate::game) struct Overlay {
    pub(in crate::game) x: f32,
    pub(in crate::game) y: f32,
    /// (entry index, craftable)
    pub(in crate::game) entries: Vec<(usize, bool)>,
}
/// One recipe button: the collection's recipes shown in the current view.
#[derive(Clone, Default)]
pub(in crate::game) struct Button {
    /// (entry index, craftable)
    pub(in crate::game) entries: Vec<(usize, bool)>,
}
impl Button {
    pub(in crate::game) fn craftable(&self) -> bool {
        self.entries.iter().any(|e| e.1)
    }
}
#[derive(Default)]
pub struct View {
    /// Visible tabs.
    pub tabs: Vec<Tab>,
    pub(in crate::game) buttons: Vec<Button>,
}

#[derive(Default)]
pub struct RecipeUi {
    /// Explicit viewport for renderer-less resource-backed fixtures.
    #[cfg(any(test, feature = "test-support"))]
    pub test_viewport: Option<[u32; 2]>,
    pub(in crate::game) custom: mod_screen::ModRecipeUi,
    pub(in crate::game) book: Option<Book>,
    pub(in crate::game) tab: Option<Tab>,
    pub(in crate::game) search: Option<EditBox>,
    pub(in crate::game) page: usize,
    /// Ticks, for the ghost and button cycling (`RecipeBookComponent.time`).
    pub(in crate::game) time: f32,
    pub(in crate::game) ghost: Vec<Ghost>,
    pub(in crate::game) overlay: Option<Overlay>,
    /// A press landed in the search box, so a drag selects text in it.
    pub(in crate::game) search_drag: bool,
    pub(in crate::game) view: View,
    pub(in crate::game) view_key: u64,
    /// Squeeze animations of new recipes: ticks left per button index and tab.
    pub(in crate::game) button_anim: Vec<(usize, f32)>,
    pub(in crate::game) tab_anim: Vec<(Tab, f32)>,
    /// The client's copy of the known recipes (`ClientRecipeBook`), its order deciding the buttons'.
    pub(in crate::game) client: ClientBook,
    pub(in crate::game) client_version: u64,
    /// The entries a server's packets carried, by display id: the book is
    /// made with them over its own (`Book::use_server_entries`).
    pub(in crate::game) server_entries:
        std::collections::BTreeMap<i32, sunrise_protocol::packets::recipes::RecipeBookEntry>,
    /// The display a click last placed (`lastRecipe`), which Enter and Space
    /// place again; slot clicks keep it.
    pub(in crate::game) last_clicked: Option<i32>,
    /// The server's checks of viewer transfers (`transfer_recipe_check`).
    pub checks: TransferChecks,
}

/// The recipe viewer channels the server said it answers: `sunrise:recipe_transfer`
/// and `sunrise:recipe_check` (`lan::recipes::advertise_viewer_channels`).
#[derive(Clone, Copy, Debug, Default)]
pub struct ViewerChannels {
    pub(in crate::game) transfer: bool,
    pub check: bool,
}

/// The server's answers to `sunrise:recipe_check`, for what the client last
/// saw of the open menu: its window, its state id, and the inventory and
/// grid. Any change there forgets them.
#[derive(Default)]
pub struct TransferChecks {
    pub(in crate::game) seen: Option<(i32, i32, Vec<Option<ItemStack>>)>,
    pub(in crate::game) answers: Vec<CheckAnswer>,
    /// The one request out: its tag, its recipe and when it went.
    pub asked: Option<(i32, String, Instant)>,
}

impl TransferChecks {
    /// The last answer remains usable while a refresh is pending.
    pub(in crate::game) fn lookup(
        &self,
        recipe: &str,
        now: Instant,
    ) -> (Option<&RecipeTransferCheck>, bool) {
        let answer = self.answers.iter().find(|a| a.recipe == recipe);
        let refresh_due = answer.is_none_or(|a| now >= a.refreshes);
        let waiting = self
            .asked
            .as_ref()
            .is_some_and(|(_, _, at)| now.duration_since(*at) < CHECK_WAIT);
        (answer.map(|a| &a.check), refresh_due && !waiting)
    }

    pub(in crate::game) fn ask(&mut self, recipe: &str, now: Instant) -> Option<i32> {
        static NEXT_TAG: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
        let tag = NEXT_TAG
            .try_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |tag| tag.checked_add(1),
            )
            .ok()?;
        self.asked = Some((tag, recipe.to_owned(), now));
        Some(tag)
    }

    /// Make room for a new answer and return its refresh deadline.
    pub(in crate::game) fn prepare_answer(&mut self, recipe: &str, now: Instant) -> Instant {
        self.answers.retain(|a| a.recipe != recipe);
        if self.answers.len() >= CHECK_KEPT {
            self.answers.remove(0);
        }
        now + CHECK_REFRESH
    }
}

/// One answer of `TransferChecks`.
pub(in crate::game) struct CheckAnswer {
    pub(in crate::game) recipe: String,
    /// When this answer becomes eligible to refresh.
    pub(in crate::game) refreshes: Instant,
    pub check: RecipeTransferCheck,
}

/// How long an answer stays fresh (chests beside a station change without
/// the client seeing it).
pub(in crate::game) const CHECK_REFRESH: Duration = Duration::from_secs(1);
/// A request with no answer this long is given up, and another may go.
pub(in crate::game) const CHECK_WAIT: Duration = Duration::from_millis(500);
/// The answers kept for one view of the menu.
pub(in crate::game) const CHECK_KEPT: usize = 64;

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod transfer_check_cache_tests {
    use super::*;

    #[test]
    fn a_wanted_answer_refreshes_every_second_without_a_view_change() {
        let start = Instant::now();
        let recipe = "minecraft:stick";
        let check = RecipeTransferCheck {
            result: RecipeTransferResult::Missing,
            missing_cells: vec![1],
            craftable_cells: Vec::new(),
        };
        let mut checks = TransferChecks {
            seen: Some((1, 0, Vec::new())),
            ..Default::default()
        };
        for second in 0..20 {
            // An immediate reply resets the deadline, without a new menu view.
            let received = start + Duration::from_secs(second);
            checks.asked = None;
            let refreshes = checks.prepare_answer(recipe, received);
            checks.answers.push(CheckAnswer {
                recipe: recipe.into(),
                refreshes,
                check: check.clone(),
            });
            let early = received + Duration::from_millis(999);
            let stale = received + Duration::from_secs(1);
            assert_eq!(checks.lookup(recipe, early), (Some(&check), false));
            assert_eq!(checks.lookup(recipe, stale), (Some(&check), true));
            assert!(checks.ask(recipe, stale).is_some());
            assert_eq!(checks.lookup(recipe, stale), (Some(&check), false));
        }
    }
}

impl RecipeUi {
    /// A new screen starts without a ghost recipe, overlay or search focus.
    pub fn reset_screen(&mut self) {
        self.ghost.clear();
        self.overlay = None;
        self.tab = None;
        self.page = 0;
        if let Some(search) = self.search.as_mut() {
            search.focused = false;
            search.set_value("");
        }
        self.view_key = 0;
        self.last_clicked = None;
        self.custom = mod_screen::ModRecipeUi::default();
    }
}

pub(in crate::game) fn inside(p: (f32, f32), x: f32, y: f32, w: f32, h: f32) -> bool {
    p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h
}
pub(in crate::game) fn sprite(name: &str) -> String {
    format!("recipe_book/{name}")
}

fn map_ghost_slots(book_type: BookType, slots: Vec<GhostSlot>) -> Vec<Ghost> {
    slots
        .into_iter()
        .map(|g| Ghost {
            slot: match (g.target, book_type) {
                (GhostTarget::Result, BookType::Crafting) => 200,
                (GhostTarget::Result, _) => 302,
                (GhostTarget::Grid(i), _) => 100 + i,
                (GhostTarget::Input, _) => 300,
                (GhostTarget::Fuel, _) => 301,
            },
            result: g.target == GhostTarget::Result,
            items: g.items,
        })
        .collect()
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// A viewer's transfer of a recipe by id, into a mod menu or a grid.
    pub(in crate::game) fn transfer_recipe_id(&mut self, recipe: &str, max: bool) {
        self.transfer_viewer_recipe_id(recipe, max);
    }

    /// The vanilla book the open screen has, if any.
    pub fn active_book(&self) -> Option<BookType> {
        if self.client.screen != Screen::Inventory || self.common.station.is_some() {
            return None;
        }
        // Explicitly assigned mod panels use their own identity and flags.
        if self.mount_active() || self.native_screen_height().is_some() {
            return None;
        }
        match self.common.container_kind() {
            // The inventory's grid, or a crafting table's (`CraftingScreen`); a server's
            // other menus have no crafting book.
            None if self.common.container.is_none()
                && self.client.remote.as_ref().is_none_or(|r| {
                    (r.menu.as_ref()).is_none_or(|(_, menu)| menu == "minecraft:crafting")
                }) =>
            {
                Some(BookType::Crafting)
            }
            Some(BlockEntity::Furnace(f)) => Some(match f.kind {
                FurnaceKind::Furnace => BookType::Furnace,
                FurnaceKind::BlastFurnace => BookType::BlastFurnace,
                FurnaceKind::Smoker => BookType::Smoker,
            }),
            _ => None,
        }
    }
    pub(in crate::game) fn book_open(&self, book: BookType) -> bool {
        self.common.level.recipe_book.open[book.index()]
    }
    pub(in crate::game) fn book_filtering(&self, book: BookType) -> bool {
        self.common.level.recipe_book.filtering[book.index()]
    }
    /// How far the open book pushes the menu to the right
    /// (`RecipeBookComponent.updateScreenPosition`).
    pub(in crate::game::client_side) fn recipe_book_shift(&self, width: f32) -> f32 {
        if self.mod_book_open() {
            return self.mod_book_shift(width);
        }
        match self.active_book() {
            Some(book) if self.book_open(book) && width >= NARROW => 77.0,
            _ => 0.0,
        }
    }
    /// `AbstractRecipeBookScreen.isHovering` is false while the book is
    /// visible in a window narrower than `NARROW`: the book covers the menu,
    /// so no slot is hovered or clicked.
    pub(in crate::game::client_side) fn book_covers_menu(&self, width: f32) -> bool {
        (self.mod_book_open() && self.mod_book_overlay(width))
            || (width < NARROW && self.active_book().is_some_and(|b| self.book_open(b)))
    }
    /// Whether typed keys go to the search box.
    pub fn recipe_search_focused(&self) -> bool {
        (self.mod_book_open() || self.active_book().is_some_and(|b| self.book_open(b)))
            && self
                .client
                .recipe_ui
                .0
                .search
                .as_ref()
                .is_some_and(|s| s.focused)
    }
    /// `keyChat` focuses the search box.
    pub fn recipe_focus_search(&mut self) -> bool {
        if (self.mod_book_open() || self.active_book().is_some_and(|b| self.book_open(b)))
            && let Some(s) = self.client.recipe_ui.0.search.as_mut()
        {
            s.focused = true;
            s.reset_blink();
            return true;
        }
        false
    }
    /// Text typed (or one backspace) into the search box.
    pub(in crate::game::client_side) fn recipe_text(
        &mut self,
        text: Option<&str>,
        backspace: bool,
    ) {
        let Some(search) = self.client.recipe_ui.0.search.as_mut() else {
            return;
        };
        if backspace {
            search.delete_chars(-1);
        } else if let Some(text) = text {
            search.insert_text(text);
        }
    }
    /// Ctrl+A/C/V/X in the search box.
    pub(in crate::game::client_side) fn recipe_shortcut(
        &mut self,
        shortcut: sunrise_client::clipboard::EditShortcut,
    ) {
        if let Some(search) = self.client.recipe_ui.0.search.as_mut() {
            search.shortcut(shortcut, &mut self.client.clipboard.0);
        }
    }
    /// A key press while the search box has focus.
    pub(in crate::game::client_side) fn recipe_key(
        &mut self,
        key: winit::keyboard::KeyCode,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
    ) -> bool {
        if let Some(shortcut) = shortcut {
            self.recipe_shortcut(shortcut);
            return true;
        }
        let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
        let Some(search) = self.client.recipe_ui.0.search.as_mut() else {
            return false;
        };
        let handled = search.key(key, ctrl, shift);
        search.reset_blink();
        handled
    }

    /// `RecipeToast.addOrUpdate` for each display of the recipes just
    /// unlocked: its crafting station and result join the toast still
    /// queued or shown, or a new one.
    pub(in crate::game) fn recipe_toast(&mut self, recipes: &[String]) {
        if recipes.is_empty() || self.common.adv.quiet {
            return;
        }
        self.ensure_book();
        let index = self.display_index();
        let book = self.client.recipe_ui.0.book.as_ref();
        // SmithingTrimDemoSlotDisplay draws from a random of its own.
        let mut random = random_source::entity();
        let mut items = Vec::new();
        for id in recipes {
            let Some(station) = index.toast_station(id) else {
                continue;
            };
            for display in index.display_ids(id) {
                // The book's displays carry their resolved results; the
                // other recipe types name theirs, and trims are demos.
                let result = book
                    .and_then(|b| {
                        b.entry_of_display(display)
                            .map(|i| b.entries[i].result.clone())
                    })
                    .or_else(|| index.result(id).map(|r| ItemStack::new(r, 1)))
                    .or_else(|| {
                        self.stations().recipes.trim_demo(
                            &self.stations().enchantments,
                            id,
                            &mut |n| {
                                sunrise_core::random::RandomSource::next_int_bound(
                                    &mut random,
                                    n as i32,
                                )
                                .unwrap_or(0) as usize
                            },
                        )
                    });
                if let Some(result) = result {
                    items.push((station.to_owned(), result));
                }
            }
        }
        if items.is_empty() {
            return;
        }
        match self.client.toast_manager.find_mut(|entry| match entry {
            super::toast_manager::Toast::Advancement(t) if t.is_recipe() => Some(t),
            _ => None,
        }) {
            Some(toast) => toast.add_recipes(items),
            None => self.add_advancement_toast(
                crate::game::client_side::advancement_toast::Toast::recipe(items),
            ),
        }
    }

    // ---- recipes and what can be crafted ----

    /// The grid size of the open crafting menu.
    pub(in crate::game) fn book_grid(&self) -> (usize, usize) {
        (self.common.craft_size, self.common.craft_size)
    }
    /// The items the book counts (`RecipeBookComponent.updateStackedContents`):
    /// the inventory and the menu's crafting slots, as the menu engine counts them.
    pub(in crate::game) fn book_available(&mut self, _book: BookType) -> Stacked {
        if self.client.remote.is_some() {
            // A server's menu: the slots it last sent, counted the same way.
            use sunrise_sim::menu::{Env, Kind, Menu};
            let table = self.common.craft_size == 3;
            let grid: Vec<_> = (0..self.common.craft_size * self.common.craft_size)
                .map(|k| self.slot_mut(100 + k).clone())
                .collect();
            let mut menu = Menu::new(if table {
                Kind::Crafting
            } else {
                Kind::Inventory
            });
            let env = Env {
                crafting_remainder: Some(self),
                player: None,
                player_entity_id: None,
                loop_context: &self.common.loop_context,
                items: self.items(),
                recipes: self.recipes(),
                brewing: &self.stations().brewing,
                banners: &self.stations().banners,
                station_recipes: self.station_recipes(),
                enchantments: &self.stations().enchantments,
                names: &self.common.pack.names,
                trade: None,
                maps: None,
                limited_crafting: self.limited_crafting(),
                can_equip: None,
                known_recipes: Some(&self.common.level.recipe_book),
                slot_rules: Some(self),
                stacked: None,
            };
            for i in 0..36 {
                menu.set_player_item(i, self.common.inventory[i].clone());
            }
            for (k, stack) in grid.into_iter().enumerate() {
                menu.set_slot(&env, 1 + k, stack);
            }
            return menu.stacked_contents(&env);
        }
        let mut out = Stacked::default();
        let stock = &mut out;
        self.menu_session(|menu, env, _| {
            *stock = menu.stacked_contents(env);
            Some(false)
        });
        out
    }
    /// Every entry of the book, in its order (`native_recipes`).
    #[cfg(test)]
    pub(in crate::game::client_side) fn recipe_book_entries(&mut self) -> Vec<Entry> {
        self.ensure_book();
        self.client
            .recipe_ui
            .0
            .book
            .as_ref()
            .map_or_else(Vec::new, |b| b.entries.clone())
    }
    /// The display ids the player's book holds, and a number that changes
    /// whenever they do (`known_recipes`).
    pub(in crate::game::client_side) fn recipe_book_displays(&mut self) -> (u64, Vec<i32>) {
        (
            self.client.recipe_ui.0.client_version,
            self.client.recipe_ui.0.client.displays(),
        )
    }
    /// The book's model is made again from `Game::recipes` when next needed:
    /// the server's recipes arrived, or a new session began.
    pub(in crate::game) fn forget_book(&mut self) {
        self.client.recipe_ui.0.book = None;
        self.client.recipe_ui.0.custom = mod_screen::ModRecipeUi::default();
        self.common.recipe_tables.books = None;
        self.client_invalidate_mod_book_placement_catalogues();
    }
    /// Clear the client's packet-owned recipe book and cached displays on disconnect.
    pub(in crate::game::client_side) fn reset_client_recipe_book(&mut self) {
        self.forget_book();
        let ui = &mut self.client.recipe_ui.0;
        ui.client.clear();
        ui.client_version += 1;
        ui.server_entries.clear();
        ui.last_clicked = None;
    }
    /// Resolves server recipe-display names from the connection's registries; a
    /// tag's items are the ones the client knows for it (the server's tags are
    /// not read).
    fn with_packet_names<R>(
        &self,
        f: impl FnOnce(&sunrise_sim::recipe_book::PacketNames<'_>) -> R,
    ) -> Option<R> {
        let r = self.client.remote.as_ref()?;
        let items = self.items();
        use sunrise_protocol::{item::Names, registry::SyncedNames};
        let names = SyncedNames {
            builtin: &r.builtin,
            dynamic: &r.registries,
        };
        let name_of = |registry: &str, id: i32| names.name(registry, id);
        let item = |id: i32| name_of("minecraft:item", id);
        let category = |id: i32| {
            name_of("minecraft:recipe_book_category", id).or_else(|| {
                sunrise_sim::recipe_book::Category::from_id(id).map(|c| c.name().to_owned())
            })
        };
        let tag = |name: &str| items.item_tag_order.get(name).cloned().unwrap_or_default();
        Some(f(&sunrise_sim::recipe_book::PacketNames {
            item: &item,
            category: &category,
            component: &|id| name_of("minecraft:data_component_type", id),
            tag: &tag,
        }))
    }
    /// Puts a server's entries over `book` (`Book::use_server_entries`).
    pub(in crate::game) fn server_entries_over<'a>(
        &self,
        book: &mut Book,
        entries: impl IntoIterator<Item = &'a sunrise_protocol::packets::recipes::RecipeBookEntry>,
    ) {
        self.with_packet_names(|names| book.use_server_entries(entries, names));
    }
    pub(in crate::game) fn ensure_book(&mut self) {
        if self.client.recipe_ui.0.book.is_none() {
            let mut book = Book::build(self.recipes(), self.items());
            book.set_display_ids(self.display_index());
            // A server numbers and describes its recipes itself.
            self.server_entries_over(&mut book, self.client.recipe_ui.0.server_entries.values());
            self.client.recipe_ui.0.book = Some(book);
        }
        self.ensure_recipe_search();
    }
    pub fn ensure_recipe_search(&mut self) {
        if self.client.recipe_ui.0.search.is_none() {
            let mut search = EditBox::new(0.0, 0.0, 81.0, 14.0);
            search.message = super::screen_info::tr("itemGroup.search", &[]);
            search.max_length = 50;
            search.focused = false;
            search.text_color = u32::MAX;
            self.client.recipe_ui.0.search = Some(search);
        }
    }
    /// Rebuilds the tab and button lists when anything they depend on changed.
    pub(in crate::game) fn refresh_book_view(&mut self, book_type: BookType) {
        self.ensure_book();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for s in self.common.inventory.iter().flatten() {
            (&s.item, s.count, s.damage).hash(&mut h);
        }
        for slot in [
            100usize, 101, 102, 103, 104, 105, 106, 107, 108, 300, 301, 302,
        ] {
            let stack =
                if slot < 109 && self.active_book() == Some(BookType::Crafting) || slot >= 300 {
                    self.slot_mut(slot).clone()
                } else {
                    None
                };
            stack.map(|s| (s.item, s.count)).hash(&mut h);
        }
        self.common.craft_size.hash(&mut h);
        self.client.recipe_ui.0.client_version.hash(&mut h);
        self.book_filtering(book_type).hash(&mut h);
        self.client
            .recipe_ui
            .0
            .tab
            .map(|t| format!("{t:?}"))
            .hash(&mut h);
        self.client
            .recipe_ui
            .0
            .search
            .as_ref()
            .map(|s| s.value.clone())
            .hash(&mut h);
        book_type.index().hash(&mut h);
        let key = h.finish();
        if key == self.client.recipe_ui.0.view_key && !self.client.recipe_ui.0.view.tabs.is_empty()
        {
            return;
        }
        let ui = std::mem::take(&mut self.client.recipe_ui.0);
        let view = self.build_view(&ui, book_type);
        self.client.recipe_ui.0 = ui;
        self.client.recipe_ui.0.view = view;
        self.client.recipe_ui.0.view_key = key;
        let pages = self.client.recipe_ui.0.view.buttons.len().div_ceil(20);
        if self.client.recipe_ui.0.page >= pages.max(1) {
            self.client.recipe_ui.0.page = 0;
        }
        self.start_new_recipe_animations(book_type);
    }
    /// `RecipeBookTabButton.startAnimation` and `RecipeButton.init`: tabs
    /// holding a recipe still to be displayed squeeze, and the buttons of the
    /// shown page squeeze once and clear their recipes' highlight.
    pub(in crate::game) fn start_new_recipe_animations(&mut self, book_type: BookType) {
        if self.common.level.recipe_book.to_be_displayed.is_empty() {
            return;
        }
        let Some(book) = self.client.recipe_ui.0.book.take() else {
            return;
        };
        let filtering = self.book_filtering(book_type);
        let (gw, gh) = self.book_grid();
        let available = self.book_available(book_type);
        let shown = |e: &Entry| {
            (book_type != BookType::Crafting || e.fits(gw, gh))
                && (!filtering || book.can_craft(e, &available))
        };
        let new = |level: &Self, e: &Entry| level.level.recipe_book.to_be_displayed.contains(&e.id);
        let mut tabs = Vec::new();
        for tab in self.client.recipe_ui.0.view.tabs.clone() {
            if tab == Tab::Search {
                continue;
            }
            let hit = self
                .client
                .recipe_ui
                .0
                .client
                .collections(&book, book_type, tab)
                .iter()
                .flat_map(|c| c.entries.iter())
                .any(|&i| shown(&book.entries[i]) && new(self, &book.entries[i]));
            if hit {
                tabs.push((tab, 15.0));
            }
        }
        self.client.recipe_ui.0.tab_anim = tabs;
        let first = self.client.recipe_ui.0.page * 20;
        let mut buttons = Vec::new();
        let mut cleared = Vec::new();
        for (n, button) in self
            .client
            .recipe_ui
            .0
            .view
            .buttons
            .iter()
            .skip(first)
            .take(20)
            .enumerate()
        {
            let ids: Vec<&String> = button
                .entries
                .iter()
                .map(|e| &book.entries[e.0].id)
                .filter(|id| self.common.level.recipe_book.to_be_displayed.contains(*id))
                .collect();
            if !ids.is_empty() {
                buttons.push((first + n, 15.0));
                cleared.extend(ids.into_iter().cloned());
            }
        }
        self.common
            .level
            .recipe_book
            .to_be_displayed
            .retain(|id| !cleared.contains(id));
        self.client.recipe_ui.0.button_anim = buttons;
        self.client.recipe_ui.0.book = Some(book);
    }
    pub(in crate::game) fn build_view(&mut self, ui: &RecipeUi, book_type: BookType) -> View {
        let Some(book) = ui.book.as_ref() else {
            return View::default();
        };
        let available = self.book_available(book_type);
        let (gw, gh) = self.book_grid();
        let selected = |e: &Entry| match book_type {
            BookType::Crafting => e.fits(gw, gh),
            _ => true,
        };
        let craftable = |e: &Entry| selected(e) && book.can_craft(e, &available);
        let filtering = self.book_filtering(book_type);
        let mut tabs = Vec::new();
        for tab in book_type.tabs() {
            let visible = tab == Tab::Search
                || ui
                    .client
                    .collections(book, book_type, tab)
                    .iter()
                    .any(|c| c.entries.iter().any(|&i| selected(&book.entries[i])));
            if visible {
                tabs.push(tab);
            }
        }
        let tab = ui.tab.filter(|t| tabs.contains(t)).unwrap_or(Tab::Search);
        let query = ui
            .search
            .as_ref()
            .map(|s| s.value.clone())
            .unwrap_or_default();
        let collections: Vec<Collection> = ui.client.collections(book, book_type, tab);
        let mut buttons = Vec::new();
        for c in collections {
            if !c.entries.iter().any(|&i| selected(&book.entries[i])) {
                continue;
            }
            if !query.is_empty()
                && !book.collection_matches(&c.entries, &query, &|stack| {
                    let mut lines = sunrise_client::item_tooltip::lines(
                        stack,
                        self.items().get(&stack.item),
                        &crate::game::native_hooks::HookedAttributeTooltips::new(self, None),
                        sunrise_client::item_tooltip::TooltipHolders::Definitions(
                            self.item_enchantments(),
                        ),
                        false,
                        &|k| self.common.pack.names.display_get(k),
                        &|attribute| self.attribute_style(attribute, false),
                    );
                    // The resolved name (a mod item's Part included) is the name line.
                    if let Some(first) = lines.first_mut() {
                        *first = self.common.stack_hover_name_text(stack).plain();
                    }
                    lines
                })
            {
                continue;
            }
            let entries: Vec<(usize, bool)> = c
                .entries
                .iter()
                .filter(|&&i| selected(&book.entries[i]))
                .map(|&i| (i, craftable(&book.entries[i])))
                .collect();
            if filtering && !entries.iter().any(|e| e.1) {
                continue;
            }
            let entries = if filtering {
                entries.into_iter().filter(|e| e.1).collect()
            } else {
                entries
            };
            buttons.push(Button { entries });
        }
        View { tabs, buttons }
    }

    // ---- geometry ----

    /// Move the vanilla recipe book origin with the window when mods reserve room.
    pub fn book_origin(&self, batch: &HudBatch) -> (f32, f32) {
        let (x, y) = self.book_origin_in_place(batch);
        (x + self.screen_room_offset(batch), y)
    }
    /// The visible left edge, including the selected tab, of the book beside
    /// the window before room moves it; none while no book is open or it covers the
    /// window of a narrow screen.
    pub fn open_book_left(&self, batch: &HudBatch) -> Option<f32> {
        if !self.mod_book_open() {
            self.active_book().filter(|b| self.book_open(*b))?;
        }
        (!self.book_covers_menu(batch.width)).then(|| self.book_origin_in_place(batch).0 - 32.0)
    }
    /// `book_origin` before the mods' room moves it.
    pub(in crate::game) fn book_origin_in_place(&self, batch: &HudBatch) -> (f32, f32) {
        let narrow = if self.native_screen_book().is_some() {
            self.mod_book_overlay(batch.width)
        } else {
            batch.width < NARROW
        };
        let offset = if narrow { 0.0 } else { 86.0 };
        (
            ((batch.width - WIDTH) / 2.0).floor() - offset,
            ((batch.height - HEIGHT) / 2.0).floor(),
        )
    }
    /// The open book's panel, `[x, y, width, height]` in GUI pixels; none
    /// while the screen has no book or it is closed.
    pub(in crate::game::client_side) fn recipe_book_rect(
        &self,
        batch: &HudBatch,
    ) -> Option<[f32; 4]> {
        if !self.mod_book_open() {
            self.active_book().filter(|b| self.book_open(*b))?;
        }
        let (x, y) = self.book_origin(batch);
        Some([x, y, WIDTH, HEIGHT])
    }
    /// The green book button's top-left corner.
    pub(in crate::game) fn book_button_pos(
        &self,
        book: BookType,
        ox: f32,
        batch: &HudBatch,
    ) -> (f32, f32) {
        let mid = (batch.height / 2.0).floor();
        match book {
            BookType::Crafting if self.common.craft_size == 2 => (ox + 104.0, mid - 22.0),
            BookType::Crafting => (ox + 5.0, mid - 49.0),
            _ => (ox + 20.0, mid - 49.0),
        }
    }
    pub fn tab_pos(&self, xo: f32, yo: f32, index: usize) -> (f32, f32) {
        (xo - 30.0, yo + 3.0 + 27.0 * index as f32)
    }
    pub fn button_pos(xo: f32, yo: f32, i: usize) -> (f32, f32) {
        (
            xo + 11.0 + 25.0 * (i % 5) as f32,
            yo + 31.0 + 25.0 * (i / 5) as f32,
        )
    }
    /// `RecipeBookComponent.slotSelectTime`.
    pub(in crate::game) fn swap_index(&self) -> usize {
        (self.client.recipe_ui.0.time / SWAP_TICKS).floor() as usize
    }
    pub(in crate::game) fn batch_for_size(&self) -> Option<HudBatch> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(size) = self.client.recipe_ui.0.test_viewport {
            return Some(HudBatch::new(size, self.client.settings.gui_scale_inputs()));
        }
        let r = self.client.renderer.0.as_ref()?;
        Some(HudBatch::new(
            [r.config.width, r.config.height],
            self.client.settings.gui_scale_inputs(),
        ))
    }

    // ---- clicks ----

    /// A click while a screen with a recipe book is open; true when the book
    /// used it.
    pub(in crate::game::client_side) fn recipe_book_click(&mut self, right: bool) -> bool {
        if self.native_screen_shown() {
            return self.mod_book_click(right);
        }
        let Some(book_type) = self.active_book() else {
            return false;
        };
        let Some(batch) = self.batch_for_size() else {
            return false;
        };
        self.ensure_book();
        let cursor = self.client.cursor;
        let (ox, _) = self.inventory_origin(&batch);
        // Clicking a crafting slot drops the ghost recipe (`slotClicked`).
        if let Some(slot) = self.slot_at(&batch, cursor.0, cursor.1)
            && self.is_book_slot(book_type, slot)
        {
            self.client.recipe_ui.0.ghost.clear();
        }
        let (bx, by) = self.book_button_pos(book_type, ox, &batch);
        if inside(cursor, bx, by, 20.0, 18.0) {
            let open = !self.book_open(book_type);
            self.common.level.recipe_book.open[book_type.index()] = open;
            // The open book moves the menu beside it: the build follows now,
            // so this click's hit tests use the new place.
            self.build_screen();
            self.send_book_settings(book_type);
            if !open {
                self.client.recipe_ui.0.overlay = None;
            }
            self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
            return true;
        }
        if !self.book_open(book_type) {
            return false;
        }
        // AbstractRecipeBookScreen.mouseClicked: in a narrow window the visible
        // book takes every click, even one it does not use itself.
        let narrow = batch.width < NARROW;
        if self.common.spectator() {
            return narrow;
        }
        self.refresh_book_view(book_type);
        let (xo, yo) = self.book_origin(&batch);
        // The alternatives overlay captures the click first.
        if let Some(overlay) = self.client.recipe_ui.0.overlay.take() {
            let maxrow = if overlay.entries.len() <= 16 { 4 } else { 5 };
            for (n, (entry, _)) in overlay.entries.iter().enumerate() {
                let x = overlay.x + 4.0 + 25.0 * (n % maxrow) as f32;
                let y = overlay.y + 5.0 + 25.0 * (n / maxrow) as f32;
                if !right && inside(cursor, x, y, 24.0, 24.0) {
                    let shift = self.shift_down();
                    if self.try_place(*entry, shift) {
                        self.remember_click(*entry);
                        self.close_narrow_book(book_type, &batch);
                    }
                    return true;
                }
            }
            return true;
        }
        let pages = self.client.recipe_ui.0.view.buttons.len().div_ceil(20);
        // Page arrows.
        if pages > 1 {
            if self.client.recipe_ui.0.page + 1 < pages
                && inside(cursor, xo + 93.0, yo + 137.0, 12.0, 17.0)
            {
                self.client.recipe_ui.0.page += 1;
                self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
                return true;
            }
            if self.client.recipe_ui.0.page > 0 && inside(cursor, xo + 38.0, yo + 137.0, 12.0, 17.0)
            {
                self.client.recipe_ui.0.page -= 1;
                self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
                return true;
            }
        }
        // Recipe buttons.
        let first = self.client.recipe_ui.0.page * 20;
        for n in 0..20 {
            let Some(button) = self.client.recipe_ui.0.view.buttons.get(first + n).cloned() else {
                break;
            };
            let (x, y) = Self::button_pos(xo, yo, n);
            if !inside(cursor, x, y, 25.0, 25.0) {
                continue;
            }
            if right {
                if button.entries.len() > 1 {
                    self.open_overlay(&button, x, y, xo, yo);
                }
            } else {
                let step = self.swap_index();
                let (entry, _) = button.entries[step % button.entries.len()];
                let shift = self.shift_down();
                if self.try_place(entry, shift) {
                    self.remember_click(entry);
                    self.close_narrow_book(book_type, &batch);
                }
            }
            return true;
        }
        // Search box.
        if let Some(search) = self.client.recipe_ui.0.search.as_mut() {
            search.x = xo + 25.0;
            search.y = yo + 13.0;
            // RecipeBookComponent's magnifierIconPlacement: from the book's
            // left edge plus eight to the search box's own left edge.
            let magnifier = inside(cursor, xo + 8.0, yo + 13.0, search.x - xo, 14.0);
            if magnifier || inside(cursor, search.x, search.y, search.width, search.height) {
                self.client.recipe_ui.0.search_drag = true;
                search.focused = true;
                search.reset_blink();
                if !magnifier {
                    let idx = search.index_at(&self.client.gui.atlas, cursor.0);
                    search.set_cursor(idx, false);
                }
                return true;
            }
            search.focused = false;
        }
        // The craftable filter.
        if inside(cursor, xo + 110.0, yo + 12.0, 26.0, 16.0) {
            let filtering = !self.book_filtering(book_type);
            self.common.level.recipe_book.filtering[book_type.index()] = filtering;
            self.send_book_settings(book_type);
            self.client.recipe_ui.0.page = 0;
            self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
            return true;
        }
        // Tabs.
        for (n, tab) in self
            .client
            .recipe_ui
            .0
            .view
            .tabs
            .clone()
            .into_iter()
            .enumerate()
        {
            let (x, y) = self.tab_pos(xo, yo, n);
            if inside(cursor, x, y, 35.0, 27.0) {
                if self.client.recipe_ui.0.tab.unwrap_or(Tab::Search) != tab {
                    self.client.recipe_ui.0.tab = Some(tab);
                    self.client.recipe_ui.0.page = 0;
                }
                self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
                return true;
            }
        }
        narrow
    }
    /// `RecipeBookComponent.mouseClicked` after a placed recipe: it becomes
    /// `lastRecipe`.
    pub(in crate::game) fn remember_click(&mut self, entry: usize) {
        self.client.recipe_ui.0.last_clicked = self
            .client
            .recipe_ui
            .0
            .book
            .as_ref()
            .and_then(|b| b.entries.get(entry))
            .map(|e| e.display_id);
    }
    /// `RecipeBookComponent.keyPressed`: Enter or Space (`isSelection`) places
    /// the recipe clicked last again, key repeats included, as many as fit
    /// with Shift. The focused search box takes the key first.
    pub(in crate::game::client_side) fn recipe_book_key(
        &mut self,
        key: winit::keyboard::KeyCode,
    ) -> bool {
        use winit::keyboard::KeyCode;
        if !matches!(key, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) {
            return false;
        }
        let Some(book_type) = self.active_book() else {
            return false;
        };
        if !self.book_open(book_type) || self.common.spectator() || self.recipe_search_focused() {
            return false;
        }
        let Some(display) = self.client.recipe_ui.0.last_clicked else {
            return false;
        };
        // The craft just taken changed what is craftable.
        self.refresh_book_view(book_type);
        let Some(entry) = self
            .client
            .recipe_ui
            .0
            .book
            .as_ref()
            .and_then(|b| b.entry_of_display(display))
        else {
            return false;
        };
        self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
        let shift = self.shift_down();
        self.try_place(entry, shift)
    }
    /// `RecipeBookComponent.mouseDragged`: while the search box is focused a
    /// drag moves its selection end to the pointer (`EditBox.onDrag`).
    pub(in crate::game::client_side) fn recipe_book_drag(&mut self) -> bool {
        if !self.client.recipe_ui.0.search_drag
            || !(self.mod_book_open() || self.active_book().is_some_and(|b| self.book_open(b)))
        {
            return false;
        }
        let x = self.client.cursor.0;
        if let Some(search) = self.client.recipe_ui.0.search.as_mut()
            && search.focused
        {
            search.drag_to(&self.client.gui.atlas, x);
            return true;
        }
        false
    }
    /// A mouse button let go: a drag no longer selects in the search box.
    pub(in crate::game::client_side) fn recipe_book_release(&mut self) -> bool {
        std::mem::take(&mut self.client.recipe_ui.0.search_drag)
    }
    /// `RecipeBookComponent.sendUpdateSettings`: the server owns the saved book.
    pub(in crate::game) fn send_book_settings(&self, book_type: BookType) {
        self.send(
            sunrise_protocol::packets::play::Serverbound::RecipeBookChangeSettings(
                sunrise_protocol::packets::recipes::RecipeBookChangeSettings {
                    book_type: book_type.index() as i32,
                    open: self.book_open(book_type),
                    filtering: self.book_filtering(book_type),
                },
            ),
        );
    }
    pub(in crate::game::client_side) fn recipe_book_flags(&self) -> Option<(bool, bool)> {
        if self.native_screen_shown() {
            return self.mod_book_flags();
        }
        let book = self.active_book()?;
        Some((self.book_open(book), self.book_filtering(book)))
    }
    /// How many displays the client's book holds.
    pub(in crate::game::client_side) fn recipe_book_known(&self) -> usize {
        self.client.recipe_ui.0.client.displays().len()
    }
    /// The next or previous page, as the arrows turn it: nothing past either end, or with a
    /// pointer button held.
    pub(in crate::game::client_side) fn pad_recipe_page(&mut self, next: bool) {
        if self.client.pad.0.menu.held != [false; 2] {
            return;
        }
        if self.native_screen_shown() {
            if self.mod_book_open() && !self.common.spectator() {
                self.client.recipe_ui.0.custom.turn_page(next);
            }
            return;
        }
        let pages = self.client.recipe_ui.0.view.buttons.len().div_ceil(20);
        let page = self.client.recipe_ui.0.page;
        let Some(to) = (if next {
            (page + 1 < pages).then_some(page + 1)
        } else {
            page.checked_sub(1)
        }) else {
            return;
        };
        self.client.recipe_ui.0.page = to;
        self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
    }
    /// The crafting book's entry of the recipe button under the pointer, in its overlay if open.
    pub(in crate::game::client_side) fn recipe_under_pointer(&self) -> Option<usize> {
        let batch = self.batch_for_size()?;
        let book_type = self.active_book()?;
        if !self.book_open(book_type) || self.common.spectator() {
            return None;
        }
        let cursor = self.client.cursor;
        if let Some(overlay) = &self.client.recipe_ui.0.overlay {
            let maxrow = if overlay.entries.len() <= 16 { 4 } else { 5 };
            return overlay.entries.iter().enumerate().find_map(|(n, e)| {
                let x = overlay.x + 4.0 + 25.0 * (n % maxrow) as f32;
                let y = overlay.y + 5.0 + 25.0 * (n / maxrow) as f32;
                inside(cursor, x, y, 24.0, 24.0).then_some(e.0)
            });
        }
        let (xo, yo) = self.book_origin(&batch);
        let first = self.client.recipe_ui.0.page * 20;
        (0..20).find_map(|n| {
            let button = self.client.recipe_ui.0.view.buttons.get(first + n)?;
            let (x, y) = Self::button_pos(xo, yo, n);
            inside(cursor, x, y, 25.0, 25.0)
                .then(|| button.entries[self.swap_index() % button.entries.len()].0)
        })
    }
    /// The recipe button under the pointer has others to show (`gui.recipebook.moreRecipes`).
    pub(in crate::game::client_side) fn recipe_has_alternatives_under_pointer(&self) -> bool {
        let Some(batch) = self.batch_for_size() else {
            return false;
        };
        if self.client.recipe_ui.0.overlay.is_some() {
            return false;
        }
        let (xo, yo) = self.book_origin(&batch);
        let first = self.client.recipe_ui.0.page * 20;
        (0..20).any(|n| {
            let (x, y) = Self::button_pos(xo, yo, n);
            inside(self.client.cursor, x, y, 25.0, 25.0)
                && self
                    .client
                    .recipe_ui
                    .0
                    .view
                    .buttons
                    .get(first + n)
                    .is_some_and(|button| button.entries.len() > 1)
        })
    }
    /// The pointer is on one of the book's own controls, a recipe or not.
    pub(in crate::game::client_side) fn recipe_book_under_pointer(&self) -> bool {
        self.recipe_book_targets().iter().any(|t| {
            let [x, y, w, h] = t.rect;
            inside(self.client.cursor, x, y, w, h)
        })
    }
    /// Whether the crafting book's entry can be crafted from what the menu holds now.
    pub(in crate::game::client_side) fn recipe_craftable(&mut self, entry_index: usize) -> bool {
        let available = self.book_available(BookType::Crafting);
        self.client.recipe_ui.0.book.as_ref().is_some_and(|book| {
            book.entries
                .get(entry_index)
                .is_some_and(|entry| book.can_craft(entry, &available))
        })
    }
    /// The crafting book's entry by what a rebuilt book keeps of it (`pad_craft::RecipeKey`).
    pub(in crate::game::client_side) fn recipe_stable_key(
        &self,
        entry_index: usize,
    ) -> Option<super::pad_craft::RecipeKey> {
        let entry = self
            .client
            .recipe_ui
            .0
            .book
            .as_ref()?
            .entries
            .get(entry_index)?;
        Some(super::pad_craft::RecipeKey {
            id: entry.id.clone(),
            display: entry.display_id,
        })
    }
    /// Where the crafting book holds the recipe now: none once the book has it no more.
    pub(in crate::game::client_side) fn recipe_entry_index(
        &self,
        key: &super::pad_craft::RecipeKey,
    ) -> Option<usize> {
        self.client
            .recipe_ui
            .0
            .book
            .as_ref()?
            .entries
            .iter()
            .position(|e| e.id == key.id && e.display_id == key.display)
    }
    /// What the crafting book's entry makes: none when the book has no such entry.
    pub(in crate::game::client_side) fn recipe_results(&self, entry_index: usize) -> &[ItemStack] {
        self.client
            .recipe_ui
            .0
            .book
            .as_ref()
            .and_then(|book| book.entries.get(entry_index))
            .map_or(&[], Entry::results)
    }
    /// Places one set of the crafting book's entry, as a click on its button places it.
    pub(in crate::game::client_side) fn recipe_place_once(&mut self, entry_index: usize) {
        self.client.recipe_ui.0.ghost.clear();
        self.client.recipe_ui.0.overlay = None;
        self.place_book_recipe(BookType::Crafting, entry_index, false);
    }
    /// The recipe book's buttons a controller can focus: the book button,
    /// recipes, page arrows, the craftable filter and the tabs (GUI pixels;
    /// the search box registers itself as a field).
    pub(in crate::game::client_side) fn recipe_book_targets(
        &self,
    ) -> Vec<sunrise_client::widgets::Target> {
        if self.native_screen_shown() {
            return self.mod_book_targets();
        }
        use sunrise_client::widgets::{Kind, Target};
        let (Some(book_type), Some(batch)) = (self.active_book(), self.batch_for_size()) else {
            return Vec::new();
        };
        let button = |rect| Target {
            rect,
            kind: Kind::Button,
        };
        let (ox, _) = self.inventory_origin(&batch);
        let (bx, by) = self.book_button_pos(book_type, ox, &batch);
        let mut out = vec![button([bx, by, 20.0, 18.0])];
        if !self.book_open(book_type) || self.common.spectator() {
            return out;
        }
        let (xo, yo) = self.book_origin(&batch);
        if self.client.recipe_ui.0.overlay.is_none() {
            // The search box is painted by hand: a field, for the pad's keyboard.
            out.push(Target {
                rect: [xo + 25.0, yo + 13.0, 81.0, 14.0],
                kind: Kind::Field,
            });
        }
        if let Some(overlay) = &self.client.recipe_ui.0.overlay {
            // The alternatives overlay takes every click while open.
            let maxrow = if overlay.entries.len() <= 16 { 4 } else { 5 };
            return (0..overlay.entries.len())
                .map(|n| {
                    let x = overlay.x + 4.0 + 25.0 * (n % maxrow) as f32;
                    let y = overlay.y + 5.0 + 25.0 * (n / maxrow) as f32;
                    button([x, y, 24.0, 24.0])
                })
                .collect();
        }
        let first = self.client.recipe_ui.0.page * 20;
        let shown = self
            .client
            .recipe_ui
            .0
            .view
            .buttons
            .len()
            .saturating_sub(first)
            .min(20);
        out.extend((0..shown).map(|n| {
            let (x, y) = Self::button_pos(xo, yo, n);
            button([x, y, 25.0, 25.0])
        }));
        let pages = self.client.recipe_ui.0.view.buttons.len().div_ceil(20);
        if pages > 1 && self.client.recipe_ui.0.page + 1 < pages {
            out.push(button([xo + 93.0, yo + 137.0, 12.0, 17.0]));
        }
        if pages > 1 && self.client.recipe_ui.0.page > 0 {
            out.push(button([xo + 38.0, yo + 137.0, 12.0, 17.0]));
        }
        out.push(button([xo + 110.0, yo + 12.0, 26.0, 16.0]));
        let selected = self.client.recipe_ui.0.tab.unwrap_or(Tab::Search);
        for (n, tab) in self.client.recipe_ui.0.view.tabs.iter().enumerate() {
            let (x, y) = self.tab_pos(xo, yo, n);
            out.push(Target {
                rect: [x, y, 35.0, 27.0],
                kind: Kind::Tab(*tab == selected),
            });
        }
        out
    }
    pub(in crate::game) fn shift_down(&self) -> bool {
        self.has_shift_down()
    }
    /// The open screen's book for `hasClickedOutside`: (visible, selected tab
    /// hovered); None when the screen has no book.
    pub(in crate::game::client_side) fn recipe_book_area(
        &self,
        batch: &HudBatch,
    ) -> Option<(bool, bool)> {
        if self.native_screen_shown() {
            return self.mod_book_area(batch);
        }
        let book = self.active_book()?;
        if !self.book_open(book) {
            return Some((false, false));
        }
        let (xo, yo) = self.book_origin(batch);
        let tabs = &self.client.recipe_ui.0.view.tabs;
        let selected = self.client.recipe_ui.0.tab.unwrap_or(Tab::Search);
        let hovered = tabs.iter().position(|t| *t == selected).is_some_and(|n| {
            let (x, y) = self.tab_pos(xo, yo, n);
            inside(self.client.cursor, x, y, 35.0, 27.0)
        });
        Some((true, hovered))
    }
    /// The grid, result and furnace slots clicks on which end a ghost recipe.
    pub(in crate::game) fn is_book_slot(&self, book: BookType, slot: usize) -> bool {
        match book {
            BookType::Crafting => (100..109).contains(&slot) || slot == 200,
            _ => (300..303).contains(&slot),
        }
    }
    /// `OverlayRecipeComponent.init`.
    pub(in crate::game) fn open_overlay(
        &mut self,
        button: &Button,
        bx: f32,
        by: f32,
        xo: f32,
        yo: f32,
    ) {
        let mut entries: Vec<(usize, bool)> =
            button.entries.iter().copied().filter(|e| e.1).collect();
        entries.extend(button.entries.iter().copied().filter(|e| !e.1));
        let (x, y) =
            screen::overlay_origin(bx as i32, by as i32, entries.len(), xo as i32, yo as i32);
        let (x, y) = (x as f32, y as f32);
        self.client.recipe_ui.0.overlay = Some(Overlay { x, y, entries });
    }

    // ---- placing a recipe ----

    /// What the book's click handler does once a recipe was placed: a book
    /// that is not offset next to the menu (`!isOffsetNextToMainGUI`, a window
    /// narrower than `NARROW`) closes with `setVisible(false)`.
    pub(in crate::game) fn close_narrow_book(&mut self, book_type: BookType, batch: &HudBatch) {
        if batch.width >= NARROW {
            return;
        }
        self.common.level.recipe_book.open[book_type.index()] = false;
        self.build_screen();
        self.send_book_settings(book_type);
        self.client.recipe_ui.0.overlay = None;
    }
    /// `RecipeBookComponent.keyPressed`: Escape closes the book, not the
    /// screen, while it covers a narrow menu; a wide window's Escape reaches
    /// the container.
    pub(in crate::game::client_side) fn recipe_book_escape(&mut self) -> bool {
        let Some(book_type) = self.active_book() else {
            return false;
        };
        let Some(batch) = self.batch_for_size() else {
            return false;
        };
        if !escape_closes_book(batch.width, self.book_open(book_type)) {
            return false;
        }
        self.close_narrow_book(book_type, &batch);
        true
    }
    /// `RecipeBookComponent.tryPlaceRecipe`: false when the click is refused.
    pub(in crate::game) fn try_place(&mut self, entry_index: usize, use_max: bool) -> bool {
        let Some(book_type) = self.active_book() else {
            return false;
        };
        if self
            .client
            .recipe_ui
            .0
            .book
            .as_ref()
            .and_then(|b| b.entries.get(entry_index))
            .is_none()
        {
            return false;
        }
        self.client.recipe_ui.0.ghost.clear();
        self.place_book_recipe(book_type, entry_index, use_max);
        true
    }
    /// Whether the open screen takes `native_place_recipe`: a crafting grid's
    /// book (the inventory's or a crafting table's), or a mod menu type's
    /// crafting grid.
    pub fn mod_recipe_screen(&self) -> bool {
        self.client.screen == Screen::Inventory
            && (self.active_book() == Some(BookType::Crafting) || self.mod_crafting_open())
            && !self.common.spectator()
    }
    /// Whether the open menu is a mod menu type with a crafting grid.
    pub(in crate::game) fn mod_crafting_open(&self) -> bool {
        self.client
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .and_then(|(_, menu)| sunrise_level::containers::mod_menu_type(menu))
            .is_some_and(|t| t.crafting.is_some())
    }
    /// `native_place_recipe`: the book's click on the recipe with this display
    /// id, refused as `handlePlaceRecipe` refuses it (no crafting screen,
    /// not in the player's book; an impossible one in `place_recipe`).
    pub(in crate::game::client_side) fn mod_place_recipe(&mut self, display_id: i32, all: bool) {
        if !self.mod_recipe_screen()
            || !self
                .client
                .recipe_ui
                .0
                .client
                .displays()
                .contains(&display_id)
        {
            return;
        }
        self.ensure_book();
        let Some(index) = self
            .client
            .recipe_ui
            .0
            .book
            .as_ref()
            .and_then(|b| b.entries.iter().position(|e| e.display_id == display_id))
        else {
            return;
        };
        self.client.recipe_ui.0.ghost.clear();
        self.place_book_recipe(BookType::Crafting, index, all);
    }
    /// Whether the open menu is a content mod's menu type.
    pub(in crate::game) fn mod_menu_open(&self) -> bool {
        self.remote_menu_type()
            .and_then(sunrise_level::containers::mod_menu_type)
            .is_some()
    }
    /// A viewer's transfer of a recipe by id (`transfer_recipe_id`): sent to the server, whose
    /// `menu_transfer` parts on a mod menu answer first, then its grid transfer. Only a server
    /// that takes viewer transfers is asked: elsewhere it shows as not into this menu.
    pub(in crate::game::client_side) fn transfer_viewer_recipe_id(
        &mut self,
        recipe: &str,
        max: bool,
    ) {
        self.client.panel.transfer_warning = None;
        let open = self.client.screen == Screen::Inventory
            && !self.common.spectator()
            && (self.mod_menu_open() || self.mod_recipe_screen());
        if !open
            || !self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.recipe_viewer.transfer)
        {
            self.show_transfer_result(RecipeTransferResult::NotIntoMenu);
            return;
        }
        let mut request = ::sunrise_core::vanilla_bytes::Writer::new();
        request.varint(self.remote_menu_id());
        request.string(recipe);
        request.u8(u8::from(max));
        self.send(sunrise_protocol::packets::play::Serverbound::CustomPayload(
            sunrise_protocol::packets::configuration::CustomPayload {
                channel: sunrise_sim::lan::recipes::TRANSFER_CHANNEL.into(),
                data: request.data,
            },
        ));
    }
    /// The display ids of `recipe_id` that the server sent and this book
    /// numbers the same way (`Book::confirmed_ids`): the entries
    /// `transfer_viewer_recipe` finds. A server numbers the recipes it sent
    /// itself, so the display index's ids are not evidence of its own.
    pub(in crate::game::client_side) fn viewer_book_ids(
        &mut self,
        recipe_id: &str,
    ) -> std::sync::Arc<[i32]> {
        self.ensure_book();
        self.client
            .recipe_ui
            .0
            .book
            .as_ref()
            .map_or_else(Default::default, |book| book.confirmed_ids(recipe_id))
    }
    /// A viewer's transfer of one set of ingredients.
    pub fn transfer_viewer_recipe(&mut self, display_id: i32) {
        self.client.panel.transfer_warning = None;
        if !self.mod_recipe_screen()
            || (self.common.creative && self.common.craft_size != 3 && !self.mod_crafting_open())
        {
            self.show_transfer_result(RecipeTransferResult::NotIntoMenu);
            return;
        }
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| !r.recipe_viewer.transfer)
        {
            // Older Sunrise and vanilla servers still accept book clicks.
            if self.recipe_place(display_id, false).is_err() {
                self.client.panel.transfer_warning = Some((
                    "sunrise.recipe_transfer.unsupported",
                    "Server requires a known recipe",
                ));
            }
            return;
        }
        self.ensure_book();
        let Some(id) = self
            .client
            .recipe_ui
            .0
            .book
            .as_ref()
            .and_then(|book| {
                book.entries
                    .iter()
                    .find(|entry| entry.display_id == display_id)
            })
            .map(|entry| entry.id.clone())
        else {
            self.show_transfer_result(RecipeTransferResult::NotIntoMenu);
            return;
        };
        if self.client.remote.is_some() {
            let mut request = ::sunrise_core::vanilla_bytes::Writer::new();
            request.varint(self.remote_menu_id());
            request.string(&id);
            self.send(sunrise_protocol::packets::play::Serverbound::CustomPayload(
                sunrise_protocol::packets::configuration::CustomPayload {
                    channel: sunrise_sim::lan::recipes::TRANSFER_CHANNEL.into(),
                    data: request.data,
                },
            ));
            return;
        }
        self.show_transfer_result(RecipeTransferResult::NotIntoMenu);
    }

    /// A payload on one of the recipe viewer's channels
    /// (`lan::recipes::viewer_channel`); with no data, the server's word that
    /// it answers that channel.
    pub(in crate::game::client_side) fn remote_recipe_viewer(
        &mut self,
        channel: &str,
        data: &[u8],
    ) {
        use sunrise_sim::lan::recipes::TRANSFER_CHANNEL;
        if data.is_empty() {
            if let Some(remote) = &mut self.client.remote {
                let channels = &mut remote.recipe_viewer;
                *(if channel == TRANSFER_CHANNEL {
                    &mut channels.transfer
                } else {
                    &mut channels.check
                }) = true;
            }
        } else if channel == TRANSFER_CHANNEL {
            self.remote_recipe_transfer(data);
        } else {
            self.remote_recipe_check(data);
        }
    }

    pub(in crate::game) fn remote_recipe_transfer(&mut self, data: &[u8]) {
        if let Ok(Some((container, result))) =
            sunrise_sim::lan::recipes::decode_transfer_reply(data)
            && container == self.remote_menu_id()
            && (self.mod_recipe_screen() || self.mod_menu_open())
            && self.client.panel.recipes.is_some()
        {
            match sunrise_sim::lan::recipes::byte_result(result) {
                Some(result) => self.show_transfer_result(result),
                None => self.client.panel.transfer_warning = None,
            }
        }
    }

    pub(in crate::game) fn show_transfer_result(&mut self, result: RecipeTransferResult) {
        if result == RecipeTransferResult::Moves {
            self.close_transferred_recipe();
        }
        self.client.panel.transfer_warning = match result {
            RecipeTransferResult::Moves => None,
            RecipeTransferResult::Missing => {
                Some(("sunrise.recipe_transfer.missing", "Missing Items"))
            }
            RecipeTransferResult::NoRoom => {
                Some(("sunrise.recipe_transfer.no_room", "Inventory full"))
            }
            RecipeTransferResult::NotIntoMenu => Some((
                "sunrise.recipe_transfer.unavailable",
                "Recipe does not fit this menu",
            )),
        };
    }

    /// `RecipesClientHost::transfer_recipe_check`: the server's latest check of a viewer
    /// transfer of `recipe` (its id) into the open menu. Asked for when
    /// there is no answer for what the client sees now (the window, its state
    /// id, the inventory and the grid), and again when the answer is one second
    /// old while it is wanted: chests a station takes from change where the client
    /// can't see them. One request out at a time. None until the answer comes,
    /// outside a screen that takes recipes, and on a server that doesn't advertise
    /// `sunrise:recipe_check`.
    pub fn transfer_recipe_check(&mut self, recipe: &str) -> Option<RecipeTransferCheck> {
        let eligible_menu = self.mod_recipe_screen()
            || (self.client.screen == Screen::Inventory
                && !self.common.spectator()
                && self.mod_menu_open());
        if !eligible_menu
            || !self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.recipe_viewer.check)
        {
            return None;
        }
        let now = Instant::now();
        let seen = (
            self.remote_menu_id(),
            self.client.remote.as_ref().map_or(0, |r| r.state_id),
            (self.common.inventory.iter().take(36))
                .chain(&self.common.craft_grid)
                .cloned()
                .collect::<Vec<_>>(),
        );
        let checks = &mut self.client.recipe_ui.0.checks;
        if checks.seen.as_ref() != Some(&seen) {
            *checks = TransferChecks {
                seen: Some(seen),
                ..Default::default()
            };
        }
        let (answer, request_due) = checks.lookup(recipe, now);
        let answer = answer.cloned();
        if request_due {
            let mut request = ::sunrise_core::vanilla_bytes::Writer::new();
            request.varint(self.remote_menu_id());
            let Some(tag) = self.client.recipe_ui.0.checks.ask(recipe, now) else {
                return answer;
            };
            request.varint(tag);
            request.string(recipe);
            self.send(sunrise_protocol::packets::play::Serverbound::CustomPayload(
                sunrise_protocol::packets::configuration::CustomPayload {
                    channel: sunrise_sim::lan::recipes::CHECK_CHANNEL.into(),
                    data: request.data,
                },
            ));
        }
        answer
    }

    /// The server's answer to `sunrise:recipe_check`. Kept only as the answer
    /// to the request out, for the view of the menu it was asked for.
    pub(in crate::game) fn remote_recipe_check(&mut self, data: &[u8]) {
        let Some((container, tag, check)) = sunrise_sim::lan::recipes::decode_check_reply(data)
        else {
            return;
        };
        let checks = &mut self.client.recipe_ui.0.checks;
        if checks.asked.as_ref().map(|(id, _, _)| *id) != Some(tag)
            || checks.seen.as_ref().map(|s| s.0) != Some(container)
        {
            return;
        }
        let (_, recipe, _) = checks.asked.take().unwrap();
        let refreshes = checks.prepare_answer(&recipe, Instant::now());
        checks.answers.push(CheckAnswer {
            recipe,
            refreshes,
            check,
        });
    }

    /// `ServerboundPlaceRecipePacket`: the server places the recipe
    /// (`ServerPlaceRecipe` on the menu engine) and answers with a ghost
    /// recipe when the items are missing.
    pub(in crate::game) fn place_book_recipe(
        &mut self,
        _book_type: BookType,
        entry_index: usize,
        use_max: bool,
    ) {
        let Some(entry) = self
            .client
            .recipe_ui
            .0
            .book
            .as_ref()
            .and_then(|b| b.entries.get(entry_index))
            .cloned()
        else {
            return;
        };
        if self.client.remote.is_none() {
            return;
        }
        // The server places it and answers with the slots, or a ghost.
        let container = self.remote_menu_id();
        self.send(sunrise_protocol::packets::play::Serverbound::PlaceRecipe(
            sunrise_protocol::packets::recipes::PlaceRecipe {
                container,
                recipe: entry.display_id,
                use_max,
            },
        ));
    }

    // ---- a server's recipe book ----

    /// ClientboundRecipeBookAddPacket (`ClientRecipeBook.add`): the
    /// displays join the book, highlighted when new, and the recipes whose
    /// entries ask for it show the toast.
    pub(in crate::game::client_side) fn remote_recipe_book_add(
        &mut self,
        p: sunrise_protocol::packets::recipes::RecipeBookAdd,
    ) {
        let ui = &mut self.client.recipe_ui.0;
        // `display_index`, borrowed by field beside the book.
        let index = match &self.common.recipe_tables.synced_index {
            Some(index) => index,
            None => &self.common.res.stations().recipe_index,
        };
        if p.replace {
            ui.client.clear();
            ui.server_entries.clear();
        }
        let mut toast = Vec::new();
        for e in &p.entries {
            ui.client.add(e.id);
            ui.server_entries.insert(e.id, e.clone());
            if e.flags & 2 != 0 {
                ui.client.highlight.insert(e.id);
            }
            if e.flags & 1 != 0
                && let Some(id) = index.recipe_of_display(e.id)
                && !toast.iter().any(|t| t == id)
            {
                toast.push(id.to_owned());
            }
        }
        ui.client_version += 1;
        // A replacing packet starts the book again from its own recipes; a
        // later one puts its entries over the book as it is.
        if p.replace {
            ui.book = None;
        } else if let Some(mut book) = ui.book.take() {
            self.server_entries_over(&mut book, p.entries.iter());
            self.client.recipe_ui.0.book = Some(book);
        }
        self.recipe_toast(&toast);
    }
    /// ClientboundRecipeBookRemovePacket.
    pub(in crate::game::client_side) fn remote_recipe_book_remove(
        &mut self,
        p: sunrise_protocol::packets::recipes::RecipeBookRemove,
    ) {
        for id in p.ids {
            self.client.recipe_ui.0.client.remove(id);
            self.client.recipe_ui.0.server_entries.remove(&id);
            if let Some(book) = self.client.recipe_ui.0.book.as_mut() {
                book.unconfirm(id);
            }
        }
        self.client.recipe_ui.0.client_version += 1;
    }
    /// ClientboundRecipeBookSettingsPacket: each book's open and filter flags.
    pub(in crate::game::client_side) fn remote_recipe_book_settings(
        &mut self,
        p: sunrise_protocol::packets::recipes::RecipeBookSettings,
    ) {
        for (i, (open, filtering)) in p.books.into_iter().enumerate() {
            self.common.level.recipe_book.open[i] = open;
            self.common.level.recipe_book.filtering[i] = filtering;
        }
        self.build_screen();
    }
    /// ClientboundPlaceGhostRecipePacket (`ClientPacketListener.handlePlaceRecipe`
    /// -> `RecipeBookComponent.showGhostRecipe`): fill directly from the server's
    /// display for the open menu.
    pub(in crate::game::client_side) fn remote_ghost_recipe(
        &mut self,
        p: sunrise_protocol::packets::recipes::PlaceGhostRecipe,
    ) {
        let open = self.remote_menu_id();
        let Some(book_type) = self.active_book() else {
            return;
        };
        if p.container != open {
            return;
        }
        let grid = self.common.craft_size;
        let fuel_empty = self.slot_mut(301).is_none();
        if let Some(slots) = self.with_packet_names(|names| {
            sunrise_sim::recipe_book::ghost_from_display(
                &p.display,
                names,
                book_type,
                grid,
                fuel_empty,
                self.items(),
                |stack| self.decode_remote_stack(stack),
            )
        }) {
            self.client.recipe_ui.0.ghost = map_ghost_slots(book_type, slots);
        }
        // The server is missing an item the client counted: nothing was placed.
        self.direct_craft_refused();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn ghost_slots(&self) -> Vec<(usize, String, bool)> {
        self.client
            .recipe_ui
            .0
            .ghost
            .iter()
            .filter_map(|g| {
                g.items
                    .first()
                    .map(|stack| (g.slot, stack.item.to_string(), g.result))
            })
            .collect()
    }

    // ---- drawing ----

    /// The book button, ghost recipe and open book, returning its deferred tooltip.
    pub(in crate::game::client_side) fn draw_recipe_book(
        &mut self,
        batch: &mut HudBatch,
    ) -> Option<Vec<(Text, u32)>> {
        // No column until this frame draws one: a closed book draws none.
        self.client.screen_state.0.drawn_book_tabs.set(None);
        if self.native_screen_shown() {
            return self.draw_mod_book(batch);
        }
        let book_type = self.active_book()?;
        self.ensure_book();
        if !self.shift_down_or_ctrl() {
            self.client.recipe_ui.0.time += self.client.frame_ticks;
        }
        let (ox, _) = self.inventory_origin(batch);
        let cursor = self.client.cursor;
        let (bx, by) = self.book_button_pos(book_type, ox, batch);
        let hover = inside(cursor, bx, by, 20.0, 18.0);
        batch.sprite(
            &self.client.gui.atlas,
            &sprite(if hover {
                "button_highlighted"
            } else {
                "button"
            }),
            bx,
            by,
        );
        self.draw_ghost(batch, book_type);
        let tooltip = if self.book_open(book_type) {
            self.refresh_book_view(book_type);
            self.draw_book_panel(batch, book_type)
        } else {
            None
        };
        self.narrate_recipe_book(batch, book_type, [bx, by, 20.0, 18.0]);
        tooltip
    }
    /// `AbstractRecipeBookScreen`'s narratables: the book button (an
    /// `ImageButton` without a message) and the `RecipeBookComponent`, which
    /// narrates its recipe buttons, search box, filter and tabs while open.
    pub(in crate::game) fn narrate_recipe_book(
        &self,
        batch: &HudBatch,
        book_type: BookType,
        button: [f32; 4],
    ) {
        use sunrise_client::widgets::{self, Narratable, key_of};
        if !widgets::narrating() {
            return;
        }
        // The button moves with the book: it keeps the focus a click gave it.
        widgets::narratable(|| {
            Narratable::button("", button, true).key(key_of(&"recipe_book/button"))
        });
        let open = self.book_open(book_type);
        let (xo, yo) = self.book_origin(batch);
        let group = key_of(&"recipe_book");
        widgets::narratable(|| Narratable::group(open, [xo, yo, WIDTH, HEIGHT]).key(group));
        if !open {
            return;
        }
        let in_book = |n: Narratable, i: usize| n.key(key_of(&(group, i))).in_group(group);
        let ui = &self.client.recipe_ui.0;
        let first = ui.page * 20;
        let shown = ui.view.buttons.iter().skip(first).take(20);
        if let Some(book) = ui.book.as_ref() {
            for (n, b) in shown.enumerate() {
                widgets::narratable(|| {
                    // RecipeButton: the result it shows now.
                    let results: Vec<Vec<ItemStack>> = b
                        .entries
                        .iter()
                        .map(|e| vec![book.entries[e.0].result.clone()])
                        .collect();
                    let stack = screen::display_stack(&results, self.swap_index());
                    let item = self.common.stack_hover_name(&stack);
                    let (x, y) = Self::button_pos(xo, yo, n);
                    in_book(
                        Narratable::recipe(item, b.entries.len() > 1, [x, y, 25.0, 25.0]),
                        n,
                    )
                });
            }
        }
        let tr = |key: &str, args: &[&str]| sunrise_client::screen_narrator::translate(key, args);
        if let Some(search) = &ui.search {
            widgets::narratable(|| {
                Narratable::edit_box(
                    tr("itemGroup.search", &[]),
                    search.value.clone(),
                    [xo + 25.0, yo + 13.0, 81.0, 14.0],
                )
                .focused(search.focused)
                .key(key_of(&(group, "search")))
                .in_group(group)
            });
        }
        widgets::narratable(|| {
            // A CycleButton of no name that hides its label: ": <value>",
            // the value's tooltip.
            let label = |filtering: bool| {
                tr(
                    if !filtering {
                        "gui.recipebook.toggleRecipes.all"
                    } else {
                        match book_type {
                            BookType::Crafting => "gui.recipebook.toggleRecipes.craftable",
                            BookType::Furnace => "gui.recipebook.toggleRecipes.smeltable",
                            BookType::BlastFurnace => "gui.recipebook.toggleRecipes.blastable",
                            BookType::Smoker => "gui.recipebook.toggleRecipes.smokable",
                        }
                    },
                    &[],
                )
            };
            let filtering = self.book_filtering(book_type);
            let full = |filtering| tr("options.generic_value", &["", &label(filtering)]);
            Narratable::cycle(
                full(filtering),
                full(!filtering),
                [xo + 110.0, yo + 12.0, 26.0, 16.0],
                true,
            )
            .hint(label(filtering))
            .key(key_of(&(group, "filter")))
            .in_group(group)
        });
        for n in 0..ui.view.tabs.len() {
            let (x, y) = self.tab_pos(xo, yo, n);
            widgets::narratable(|| {
                Narratable::button("", [x, y, 35.0, 27.0], true)
                    .key(key_of(&(group, "tab", n)))
                    .in_group(group)
            });
        }
    }
    pub(in crate::game) fn shift_down_or_ctrl(&self) -> bool {
        self.has_control_down()
    }
    /// `GhostSlots.extractRenderState`, among the container's slots.
    pub(in crate::game) fn draw_ghost(&mut self, batch: &mut HudBatch, book_type: BookType) {
        if self.client.recipe_ui.0.ghost.is_empty() {
            return;
        }
        let (ox, oy) = self.inventory_origin(batch);
        let big_result = book_type != BookType::Crafting || self.common.craft_size == 3;
        let ghosts: Vec<GhostView> = self
            .client
            .recipe_ui
            .0
            .ghost
            .iter()
            .map(|g| {
                let (sx, sy) = self.slot_position(g.slot);
                GhostView {
                    x: sx as i32,
                    y: sy as i32,
                    items: g.items.clone(),
                    result: g.result,
                }
            })
            .collect();
        let calls = screen::ghost(self.swap_index(), ox as i32, oy as i32, big_result, &ghosts);
        self.paint_book(batch, &calls);
    }
    /// `GhostSlots.extractTooltip`: the stack a ghost shows on menu slot
    /// `slot` now, cycling like its draw; only while the open book is visible
    /// (`RecipeBookComponent.extractTooltip`).
    pub(in crate::game::client_side) fn ghost_stack(&self, slot: usize) -> Option<ItemStack> {
        if self.native_screen_shown() {
            if !self.mod_book_open() {
                return None;
            }
            self.native_screen_slot(
                sunrise_gameplay_common::game::inventory_slots::container_slot_index(slot)?,
            )?;
        } else if !self.book_open(self.active_book()?) {
            return None;
        }
        let ghost = self
            .client
            .recipe_ui
            .0
            .ghost
            .iter()
            .find(|g| g.slot == slot && !g.items.is_empty())?;
        Some(ghost.items[self.swap_index() % ghost.items.len()].clone())
    }
    /// `RecipeBookComponent.extractRenderState` and `extractTooltip`, laid
    /// out by `sunrise_client::recipe_book_screen`.
    pub(in crate::game) fn draw_book_panel(
        &mut self,
        batch: &mut HudBatch,
        book_type: BookType,
    ) -> Option<Vec<(Text, u32)>> {
        let (xo, yo) = self.book_origin(batch);
        let cursor = self.client.cursor;
        let atlas = &self.client.gui.atlas;
        let search = match self.client.recipe_ui.0.search.as_mut() {
            Some(edit) => {
                // The box's place, for clicks and typing.
                edit.x = xo + 25.0;
                edit.y = yo + 13.0;
                let (display, blink_ms) = edit.view_state(atlas);
                Search {
                    value: edit.value.clone(),
                    cursor: edit.cursor,
                    highlight: edit.highlight,
                    display,
                    focused: edit.focused,
                    blink_ms,
                }
            }
            None => Search::default(),
        };
        let ui = &self.client.recipe_ui.0;
        let current = ui
            .tab
            .filter(|t| ui.view.tabs.contains(t))
            .unwrap_or(Tab::Search);
        let tabs: Vec<TabView> = ui
            .view
            .tabs
            .iter()
            .map(|tab| {
                let (a, b) = tab.icons();
                TabView {
                    icons: (a.to_owned(), b.map(str::to_owned)),
                    selected: *tab == current,
                    anim: ui
                        .tab_anim
                        .iter()
                        .find(|e| e.0 == *tab)
                        .map_or(0.0, |e| e.1),
                }
            })
            .collect();
        let pages = ui.view.buttons.len().div_ceil(20);
        let first = ui.page * 20;
        let (buttons, overlay) = match ui.book.as_ref() {
            Some(book) => {
                let buttons: Vec<ButtonView> = ui
                    .view
                    .buttons
                    .iter()
                    .enumerate()
                    .skip(first)
                    .take(20)
                    .map(|(n, b)| ButtonView {
                        has_craftable: b.craftable(),
                        entries: b
                            .entries
                            .iter()
                            .map(|e| vec![book.entries[e.0].result.clone()])
                            .collect(),
                        anim: ui
                            .button_anim
                            .iter()
                            .find(|e| e.0 == n)
                            .map_or(0.0, |e| e.1),
                    })
                    .collect();
                let overlay = ui.overlay.as_ref().map(|o| OverlayView {
                    x: o.x as i32,
                    y: o.y as i32,
                    recipes: o
                        .entries
                        .iter()
                        .map(|&(i, craftable)| OverlayRecipe {
                            craftable,
                            cells: screen::overlay_cells(book, &book.entries[i]),
                        })
                        .collect(),
                });
                (buttons, overlay)
            }
            None => (Vec::new(), None),
        };
        let translate = |key: &str| self.translate_with_fallback(key, key);
        let text_width = |s: &str| HudBatch::text_width(atlas, s) as i32;
        let layout = Layout {
            book: book_type,
            origin: (xo as i32, yo as i32),
            mouse: (cursor.0.floor() as i32, cursor.1.floor() as i32),
            index: self.swap_index(),
            filtering: self.book_filtering(book_type),
            filter_look: None,
            search: &search,
            tabs: &tabs,
            buttons: &buttons,
            page: ui.page,
            pages,
            overlay: overlay.as_ref(),
            ghost: None,
            translate: &translate,
            text_width: &text_width,
        };
        let calls = screen::draw(&layout);
        // The tabs as painted are the region `ScreenView::regions` reports (with a subscriber).
        if self.screen_view_listened() {
            self.client
                .screen_state
                .0
                .drawn_book_tabs
                .set(screen::sprite_bounds(&calls, |_, id| {
                    screen::is_tab_sprite(id)
                }));
        }
        let tooltip = self.paint_book(batch, &calls);
        // The squeeze animations run down with the frames.
        let frame = self.client.frame_ticks;
        for e in &mut self.client.recipe_ui.0.tab_anim {
            e.1 -= frame;
        }
        for e in &mut self.client.recipe_ui.0.button_anim {
            e.1 -= frame;
        }
        self.client.recipe_ui.0.tab_anim.retain(|e| e.1 > 0.0);
        self.client.recipe_ui.0.button_anim.retain(|e| e.1 > 0.0);
        tooltip
    }
    /// Paints recipe book calls; returns the first tooltip set (vanilla keeps
    /// the first one a frame asks for).
    pub(in crate::game) fn paint_book(
        &mut self,
        batch: &mut HudBatch,
        calls: &[Drawn],
    ) -> Option<Vec<(sunrise_game_data::text::Text, u32)>> {
        let mut tooltip: Option<Vec<(sunrise_game_data::text::Text, u32)>> = None;
        for d in calls {
            let start = batch.vertices.len();
            let atlas = &self.client.gui.atlas;
            match &d.call {
                Call::Blit {
                    x, y, w, h, u, v, ..
                } => {
                    if let Some(r) = atlas.sprites.get("gui/recipe_book").copied() {
                        let (w, h) = (*w as f32, *h as f32);
                        batch.rect_uv(
                            *x as f32,
                            *y as f32,
                            w,
                            h,
                            sub_rect(r, [*u, *v, w, h]),
                            u32::MAX,
                        );
                    }
                }
                Call::Sprite { id, x, y, w, h } => {
                    let name = id.as_str();
                    let native = atlas
                        .gui_rect(name)
                        .is_some_and(|r| r.w as i32 == *w && r.h as i32 == *h);
                    if native {
                        batch.sprite(atlas, name, *x as f32, *y as f32);
                    } else {
                        sunrise_client::ui::nine_slice(
                            batch,
                            atlas,
                            name,
                            *x as f32,
                            *y as f32,
                            *w as f32,
                            *h as f32,
                            u32::MAX,
                        );
                    }
                }
                Call::Fill {
                    x0,
                    y0,
                    x1,
                    y1,
                    color,
                } => batch.fill(
                    atlas,
                    *x0 as f32,
                    *y0 as f32,
                    (x1 - x0) as f32,
                    (y1 - y0) as f32,
                    argb(*color),
                ),
                Call::Highlight { x0, y0, x1, y1 } => {
                    let (a, b) = (*x0.min(x1), *x0.max(x1));
                    batch.text_highlight(
                        atlas,
                        (a as f32, *y0 as f32, (b - a) as f32, (y1 - y0) as f32),
                        true,
                    );
                }
                Call::Text {
                    runs,
                    x,
                    y,
                    color,
                    shadow,
                } => {
                    let mut x = *x as f32;
                    for r in runs {
                        let style = Style {
                            color: (r.color >= 0).then_some(r.color as u32),
                            italic: r.italic.then_some(true),
                            ..Style::default()
                        };
                        let t = Text::styled(r.text.clone(), style);
                        batch.text_styled(atlas, &t, x, *y as f32, argb(*color), *shadow);
                        x += atlas.font.width_text(&t);
                    }
                }
                Call::Item { call, stack, x, y } => match call {
                    ItemCall::Item | ItemCall::Fake => {
                        self.draw_item_model(batch, stack, *x as f32, *y as f32, 0)
                    }
                    ItemCall::Decorations => {
                        self.draw_item_decorations(batch, stack, *x as f32, *y as f32, None)
                    }
                },
                Call::Tooltip(lines) => {
                    if tooltip.is_none() {
                        tooltip = Some(
                            lines
                                .iter()
                                .map(|l| {
                                    (
                                        sunrise_game_data::text::Text::literal(
                                            l.iter().map(|r| r.text.as_str()).collect::<String>(),
                                        ),
                                        u32::MAX,
                                    )
                                })
                                .collect(),
                        );
                    }
                }
                Call::ItemTooltip { stack, more } => {
                    if tooltip.is_none() {
                        let mut lines = self.stack_tooltip(stack);
                        if *more {
                            lines.push((
                                sunrise_game_data::text::Text::literal(
                                    self.translate_with_fallback(
                                        "gui.recipebook.moreRecipes",
                                        "Right Click for More",
                                    ),
                                ),
                                u32::MAX,
                            ));
                        }
                        tooltip = Some(lines);
                    }
                }
                Call::Stratum => {}
            }
            if d.pose != screen::IDENTITY {
                let m = d.pose;
                for v in &mut batch.vertices[start..] {
                    let [x, y] = v.pos;
                    v.pos = [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
                }
            }
        }
        tooltip
    }
}

/// An ARGB colour as a HUD vertex colour.
pub(in crate::game) fn argb(c: i32) -> u32 {
    let c = c as u32;
    rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8)
}

/// A sub-rectangle of a sprite given in texels.
pub(in crate::game) fn sub_rect(r: Rect, [x, y, w, h]: [f32; 4]) -> Rect {
    let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
    Rect {
        u0: r.u0 + x * du,
        v0: r.v0 + y * dv,
        u1: r.u0 + (x + w) * du,
        v1: r.v0 + (y + h) * dv,
        w: w as u32,
        h: h as u32,
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod request_generation_tests {
    use super::*;
    #[test]
    fn delayed_answers_cannot_match_inventory_changes_or_retries() {
        let mut checks = TransferChecks::default();
        let first = checks.ask("minecraft:stick", Instant::now()).unwrap();
        checks = TransferChecks::default(); // Inventory change clears the cache.
        let second = checks.ask("minecraft:stick", Instant::now()).unwrap();
        assert_ne!(first, second);
        let retry = checks.ask("minecraft:stick", Instant::now()).unwrap();
        assert_ne!(second, retry);
        assert_eq!(
            (checks.asked.as_ref()).map(|(tag, id, _)| (*tag, id.as_str())),
            Some((retry, "minecraft:stick"))
        );
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod tests {
    /// `RecipeBookComponent.keyPressed`: Escape closes a narrow window's open
    /// book (`!isOffsetNextToMainGUI`), not the container; a wide window's
    /// book leaves Escape to the container.
    #[test]
    fn escape_closes_only_a_narrow_books_menu() {
        assert!(super::escape_closes_book(super::NARROW - 1.0, true));
        assert!(!super::escape_closes_book(super::NARROW, true));
        assert!(!super::escape_closes_book(super::NARROW - 1.0, false));
    }

    #[test]
    #[ignore = "daily"]
    fn screen_room_keeps_a_mod_menus_book_toggle_painted_and_reachable() {
        use crate::game::native_hooks::book_screen::lifecycle_tests_shared::{
            BookMod, KEY, STONE, receive,
        };
        use crate::game::native_hooks::{OpenNativeMenu, OpenNativeScreen, seed_native_menu};
        use std::sync::Arc;
        use sunrise_gameplay_common::game::Screen;
        use sunrise_gameplay_common::game::tables::GameItems as _;
        use sunrise_sim::recipe_book::{
            BookFlags, placement_sync::ResourceId, settings_sync::Message,
        };

        sunrise_gameplay_common::game::in_own_process(
            "game::client_side::recipe_book::tests::screen_room_keeps_a_mod_menus_book_toggle_painted_and_reachable",
            || {
                use std::sync::atomic::{AtomicU32, Ordering};
                struct Room([Arc<AtomicU32>; 2]);
                #[mod_api::__subscriptions]
                impl mod_api::Hooks for Room {
                    fn screen_room(
                        &self,
                        _: &dyn mod_api::ClientHost,
                        event: &mut mod_api::ScreenRoomEvent<'_>,
                    ) {
                        event.left = f32::from_bits(self.0[0].load(Ordering::Relaxed));
                        event.right = f32::from_bits(self.0[1].load(Ordering::Relaxed));
                    }
                }
                let rooms = [Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(0))];
                let mods: mod_api::Mods = vec![
                    mod_api::ModEntry::new("geometry", Box::new(BookMod)),
                    mod_api::ModEntry::new("room", Box::new(Room(rooms.clone()))),
                ]
                .into();
                mod_platform::testing::with_mods(mods, || {
                    crate::game::native_entity_refs::tests::with_client_game(|game| {
                        let key = KEY.get().unwrap().clone();
                        mod_api::__private::resolve_menus(["geometry:menu"]).unwrap();
                        let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
                        game.connect_local(pipe, "Geometry").unwrap();
                        game.client.remote.as_mut().unwrap().joined = true;
                        let generation = seed_native_menu(game, 1, "geometry:menu");
                        game.client.remote.as_mut().unwrap().custom_books = true;
                        game.client.settings.gui_scale = 1;
                        game.client.screen = Screen::Inventory;
                        game.client.title.0 = None;
                        game.container = Some((i32::MIN, 0, 0));
                        let recipes = sunrise_sim::crafting::Recipes::from_sources(
                            &game.options.resources,
                            game.items(),
                            &sunrise_world::loot::PackOverlay::NONE,
                            vec![(
                                "geometry:recipe".into(),
                                serde_json::json!({"type": "geometry:crafting"}),
                            )],
                            game.res.core.item_data.context(),
                        )
                        .unwrap();
                        game.recipe_tables.install_test_recipes(recipes);
                        game.recipe_tables.books = None;
                        // (screen, menu width, [left, right]) -> the window's left edge, and the
                        // toggle's top-left corner, painted, clicked and focused there.
                        let cases = [
                            // The closed book's toggle in the right gutter, 4 right of the window.
                            ([427, 240], 176.0, [0.0, 0.0], 125.0, (305.0, 42.0)),
                            // 50 right: the toggle at 355..375 stays on the screen.
                            ([427, 240], 176.0, [100.0, 0.0], 175.0, (355.0, 42.0)),
                            // The window would fit at 249..425, the toggle not at 429..449: no move.
                            ([427, 240], 176.0, [248.0, 0.0], 125.0, (305.0, 42.0)),
                            // Right room still moves the toggle left with the window.
                            ([427, 240], 176.0, [0.0, 149.0], 51.0, (231.0, 42.0)),
                            // #6198: on a narrow screen the toggle is in the left gutter at 57; 154
                            // right would fit the window at 4..142 but put the toggle at -20..0.
                            ([301, 240], 138.0, [0.0, 0.0], 81.0, (57.0, 42.0)),
                            ([301, 240], 138.0, [0.0, 154.0], 81.0, (57.0, 42.0)),
                            // 40 right keeps it at 37..57.
                            ([301, 240], 138.0, [0.0, 40.0], 61.0, (37.0, 42.0)),
                        ];
                        for (epoch, (size, width, [left, right], x, toggle)) in (1..).zip(cases) {
                            game.client.recipe_ui.0.test_viewport = Some(size);
                            game.client.mod_screen.0 = Some(OpenNativeScreen {
                                menu: OpenNativeMenu {
                                    key: (i32::MIN, 0, 0),
                                    window: 1,
                                    generation,
                                    name: "geometry:menu".into(),
                                    title: None,
                                    block: Some(STONE.into()),
                                    pos: None,
                                    mod_type: None,
                                    data: Vec::new(),
                                },
                                slot_moves: Default::default(),
                                layout_revision: 0,
                                shown: true,
                                detached: false,
                                screen: mod_api::__private::OwnedMenuScreen::new(
                                    "geometry",
                                    0,
                                    mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                                        width,
                                        height: 166.0,
                                        slots: vec![Some(mod_api::ScreenSlot {
                                            position: [8.0, 18.0],
                                            ..Default::default()
                                        })],
                                        inventory_y: 84.0,
                                        inventory_x: 8.0,
                                        recipe_book: Some(key.clone()),
                                        ..Default::default()
                                    }),
                                ),
                            });
                            receive(game, Message::Ready);
                            receive(
                                game,
                                Message::Snapshot {
                                    epoch,
                                    revision: 0,
                                    entries: vec![(
                                        ResourceId::from_registered_book(&key),
                                        BookFlags {
                                            open: false,
                                            filtering: false,
                                        },
                                    )],
                                },
                            );
                            rooms[0].store(f32::to_bits(left), Ordering::Relaxed);
                            rooms[1].store(f32::to_bits(right), Ordering::Relaxed);
                            game.client.frames += 1;
                            game.build_screen();
                            let case = format!("{size:?} {width} [{left}, {right}]");
                            let batch = sunrise_client::hud::HudBatch::new(
                                size,
                                game.client.settings.gui_scale_inputs(),
                            );
                            assert_eq!(game.inventory_origin(&batch), (x, 37.0), "{case}");
                            let mut painted = sunrise_client::hud::HudBatch::new(
                                size,
                                game.client.settings.gui_scale_inputs(),
                            );
                            game.draw_recipe_book(&mut painted);
                            let button = &painted.vertices[..6];
                            let corner = |pick: fn(f32, f32) -> f32, i: usize| {
                                button.iter().map(|v| v.pos[i]).reduce(pick).unwrap()
                            };
                            assert_eq!(
                                [corner(f32::min, 0), corner(f32::min, 1)],
                                [toggle.0, toggle.1],
                                "painted {case}"
                            );
                            assert_eq!(
                                [corner(f32::max, 0), corner(f32::max, 1)],
                                [toggle.0 + 20.0, toggle.1 + 18.0],
                                "painted {case}"
                            );
                            assert_eq!(
                                game.recipe_book_targets()[0].rect,
                                [toggle.0, toggle.1, 20.0, 18.0],
                                "focused {case}"
                            );
                            game.client.cursor = (toggle.0 + 1.0, toggle.1 + 1.0);
                            assert!(game.recipe_book_click(false), "clicked {case}");
                        }
                    });
                });
            },
        );
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
