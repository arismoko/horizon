//! A mod menu's book uses the shared painter, but never vanilla storage ordinals.
use super::*;
use sunrise_sim::recipe_book::mod_presentation::ModPresentation;
#[path = "mod_screen/ghost.rs"]
mod ghost;
#[path = "mod_screen/layout.rs"]
mod layout;
#[path = "mod_screen/placement.rs"]
mod placement;
#[path = "mod_screen/predicate.rs"]
mod predicate;
#[path = "mod_screen/resolve.rs"]
#[cfg(feature = "client")]
mod resolve;

#[derive(Default)]
pub(in crate::game) struct ModRecipeUi {
    binding: Option<(String, String)>,
    selected: Option<String>,
    page: usize,
    presented_page: usize,
    tab_page: usize,
    presentation: Option<ModPresentation>,
    prepared: Option<sunrise_sim::recipe_book::mod_presentation::PreparedBook>,
    overlay: Option<placement::PlacementOverlay>,
    stocks: Vec<ItemStack>,
    inventory_len: usize,
    query: String,
    filtering: bool,
    refused: bool,
}

impl ModRecipeUi {
    fn category_pages(&self) -> usize {
        self.presentation
            .as_ref()
            .map_or(0, |p| p.tabs.len().saturating_sub(1).div_ceil(4))
    }
    fn visible_tabs(&self) -> Vec<usize> {
        let total = self.presentation.as_ref().map_or(0, |p| p.tabs.len());
        if total == 0 {
            return Vec::new();
        }
        let first = 1 + self.tab_page * 4;
        std::iter::once(0)
            .chain(first..total.min(first + 4))
            .collect()
    }
    fn turn_categories(&mut self, next: bool) -> bool {
        let page = if next {
            self.tab_page
                .checked_add(1)
                .filter(|p| *p < self.category_pages())
        } else {
            self.tab_page.checked_sub(1)
        };
        let Some(page) = page else { return false };
        self.tab_page = page;
        true
    }
    pub(in crate::game) fn turn_page(&mut self, next: bool) -> bool {
        let pages = self
            .presentation
            .as_ref()
            .map_or(0, |p| p.recipes.len().div_ceil(20));
        let page = if next {
            self.page.checked_add(1).filter(|p| *p < pages)
        } else {
            self.page.checked_sub(1)
        };
        let Some(page) = page else { return false };
        self.page = page;
        self.overlay = None;
        true
    }
}

fn tab_arrows(
    ui: &ModRecipeUi,
    xo: f32,
    yo: f32,
) -> impl Iterator<Item = (bool, [f32; 4], &'static str, &'static str)> + use<> {
    [
        (
            ui.tab_page > 0,
            false,
            [xo - 28.0, yo + 143.0, 12.0, 17.0],
            "page_backward",
            "gui.recipebook.previous_page",
        ),
        (
            ui.tab_page + 1 < ui.category_pages(),
            true,
            [xo - 11.0, yo + 143.0, 12.0, 17.0],
            "page_forward",
            "gui.recipebook.next_page",
        ),
    ]
    .into_iter()
    .filter_map(|(show, next, rect, sprite, key)| show.then_some((next, rect, sprite, key)))
}

fn draw_registered_book(
    layout: &Layout<'_>,
    ui: &ModRecipeUi,
    (xo, yo): (f32, f32),
    cursor: (f32, f32),
) -> (Vec<Drawn>, usize) {
    let mut calls = screen::draw(layout);
    // The tab sprites, and the category arrows painted beside them (none under an overlay).
    let first_category_arrow = calls.len();
    if ui.overlay.is_none() {
        for (_, [x, y, w, h], name, key) in tab_arrows(ui, xo, yo) {
            let hover = inside(cursor, x, y, w, h);
            let id = format!(
                "recipe_book/{name}{}",
                if hover { "_highlighted" } else { "" }
            );
            calls.push(Drawn {
                call: Call::Sprite {
                    id,
                    x: x as i32,
                    y: y as i32,
                    w: w as i32,
                    h: h as i32,
                },
                pose: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            });
            if hover {
                calls.push(Drawn {
                    call: Call::Tooltip(vec![vec![screen::Run::plain((layout.translate)(key))]]),
                    pose: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                });
            }
        }
    }
    (calls, first_category_arrow)
}

fn registered_book_tab_bounds(calls: &[Drawn], first_category_arrow: usize) -> Option<[f32; 4]> {
    screen::sprite_bounds(calls, |index, id| {
        index >= first_category_arrow || screen::is_tab_sprite(id)
    })
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    fn mod_book_panel(&self) -> Option<sunrise_sim::recipe_book::ModBookPanel<'_>> {
        let key = self.native_screen_book()?;
        let menu = self.native_screen_menu()?;
        if mod_api::RecipeBookLayout::declaring_mod(menu).is_some() {
            let view = self.common.native_net.placement.view.as_ref()?;
            if !self.native_book_placement_catalog_matches(&view.identity) {
                return None;
            }
            return view.index.panel(menu, key);
        }
        self.common
            .recipe_tables
            .books
            .as_ref()?
            .as_ref()
            .ok()?
            .panel(menu, key)
    }
    fn mod_book_layout(&self, width: f32, open: bool) -> Option<layout::BookLayout> {
        let batch = self.batch_for_size()?;
        layout::layout(
            [width, batch.height],
            [self.native_screen_width()?, self.native_screen_height()?],
            self.native_screen_toggle(),
            open,
        )
    }
    pub(in crate::game) fn mod_book_shift(&self, width: f32) -> f32 {
        self.mod_book_layout(width, true)
            .map_or(0.0, |layout| layout.shift)
    }
    pub(in crate::game) fn mod_book_overlay(&self, width: f32) -> bool {
        self.mod_book_layout(width, false)
            .is_some_and(|layout| layout.overlay)
    }
    pub(in crate::game) fn mod_book_flags(&self) -> Option<(bool, bool)> {
        self.mod_book_panel()?;
        let batch = self.batch_for_size()?;
        self.mod_book_layout(batch.width, false)?;
        self.mod_book_layout(batch.width, true)?;
        let flags = self.custom_book_settings(self.native_screen_book()?)?;
        Some((flags.open, flags.filtering))
    }
    pub(in crate::game) fn mod_book_open(&self) -> bool {
        self.mod_book_flags().is_some_and(|flags| flags.0)
    }
    fn refresh_mod_book(&mut self) {
        if self.mod_book_panel().is_some()
            && self.batch_for_size().is_some_and(|batch| {
                self.mod_book_layout(batch.width, false).is_none()
                    || self.mod_book_layout(batch.width, true).is_none()
            })
        {
            if !self.client.recipe_ui.0.custom.refused {
                if let Some(block) = self.native_screen_block() {
                    eprintln!(
                        "{}",
                        layout::refusal(
                            block,
                            self.native_screen_menu().unwrap_or_default(),
                            self.native_screen_toggle().is_some()
                        )
                    );
                }
                self.client.recipe_ui.0.custom.refused = true;
            }
            return;
        }
        let Some((open, filtering)) = self.mod_book_flags() else {
            return;
        };
        let Some(key) = self.native_screen_book() else {
            return;
        };
        let binding = (
            self.native_screen_menu().unwrap_or_default().to_owned(),
            key.as_str().to_owned(),
        );
        if self.client.recipe_ui.0.custom.binding.as_ref() != Some(&binding) {
            self.client.recipe_ui.0.reset_screen();
            self.client.recipe_ui.0.custom.binding = Some(binding);
        }
        self.ensure_recipe_search();
        if !open {
            self.client.recipe_ui.0.custom.overlay = None;
            if let Some(search) = self.client.recipe_ui.0.search.as_mut() {
                search.focused = false;
            }
            return;
        }
        let query = self
            .client
            .recipe_ui
            .0
            .search
            .as_ref()
            .map_or("", |s| s.value.as_str())
            .to_owned();
        let mut stocks: Vec<_> = self.common.inventory.iter().flatten().cloned().collect();
        let inventory_len = stocks.len();
        for slot in self.native_screen_slots().unwrap_or_default() {
            if let Some(stack) = self.slot_mut(slot).clone() {
                stocks.push(stack);
            }
        }
        let ui = &self.client.recipe_ui.0.custom;
        if ui.presentation.is_some()
            && ui.presented_page == ui.page
            && ui.query == query
            && ui.filtering == filtering
            && ui.stocks == stocks
            && ui.inventory_len == inventory_len
        {
            return;
        }
        let placement_stocks = self.mod_book_placement_stocks();
        let ui = &self.client.recipe_ui.0.custom;
        let mut stacked = Stacked::default();
        for (index, stack) in stocks.iter().enumerate() {
            let max = sunrise_sim::menu::item_max_stack(self.items(), stack);
            if index < inventory_len {
                stacked.account_simple(stack, max);
            } else {
                stacked.account_stack(stack, max);
            }
        }
        let resolver = resolve::Resolver::new(self, &stacked, &placement_stocks);
        let Some(panel) = self.mod_book_panel() else {
            return;
        };
        let prepared = ui.prepared.is_none().then(|| panel.prepare(&resolver));
        let book = ui.prepared.as_ref().or(prepared.as_ref()).unwrap();
        let mut presentation = book.presentation(
            ui.selected.as_deref(),
            &query,
            filtering,
            ui.page,
            &resolver,
        );
        let page = ui
            .page
            .min(presentation.recipes.len().div_ceil(20).saturating_sub(1));
        if page != ui.page {
            presentation =
                book.presentation(ui.selected.as_deref(), &query, filtering, page, &resolver);
        }
        let ui = &mut self.client.recipe_ui.0.custom;
        if let Some(prepared) = prepared {
            ui.prepared = Some(prepared);
        }
        ui.query = query;
        ui.filtering = filtering;
        ui.stocks = stocks;
        ui.inventory_len = inventory_len;
        ui.page = ui
            .page
            .min(presentation.recipes.len().div_ceil(20).saturating_sub(1));
        ui.presented_page = ui.page;
        ui.overlay = None;
        ui.presentation = Some(presentation);
    }
    /// A mod menu's book toggle edges before the window moves to make room.
    pub(in crate::game) fn mod_book_toggle_span(&self, batch: &HudBatch) -> Option<[f32; 2]> {
        let (open, _) = self.mod_book_flags()?;
        Some(self.mod_book_layout(batch.width, open)?.toggle_span())
    }
    fn mod_book_button(&self, batch: &HudBatch) -> (f32, f32) {
        self.mod_book_layout(batch.width, self.mod_book_open())
            .map_or((0.0, 0.0), |layout| {
                layout.shifted_toggle(self.screen_room_offset(batch))
            })
    }
    pub(in crate::game) fn draw_mod_book(
        &mut self,
        batch: &mut HudBatch,
    ) -> Option<Vec<(Text, u32)>> {
        self.refresh_mod_book();
        let (open, filtering) = self.mod_book_flags()?;
        let (bx, by) = self.mod_book_button(batch);
        batch.sprite(
            &self.client.gui.atlas,
            &sprite(if inside(self.client.cursor, bx, by, 20.0, 18.0) {
                "button_highlighted"
            } else {
                "button"
            }),
            bx,
            by,
        );
        if !self.shift_down_or_ctrl() {
            self.client.recipe_ui.0.time += self.client.frame_ticks;
        }
        self.draw_mod_book_ghosts(batch);
        if !open {
            return None;
        }
        let (xo, yo) = self.book_origin(batch);
        let search = match self.client.recipe_ui.0.search.as_mut() {
            Some(edit) => {
                edit.x = xo + 25.0;
                edit.y = yo + 13.0;
                let (display, blink_ms) = edit.view_state(&self.client.gui.atlas);
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
        let ui = &self.client.recipe_ui.0.custom;
        let presentation = ui.presentation.as_ref()?;
        let buttons: Vec<_> = presentation
            .recipes
            .iter()
            .skip(ui.page * 20)
            .take(20)
            .map(|r| r.button.clone())
            .collect();
        let tabs: Vec<_> = ui
            .visible_tabs()
            .into_iter()
            .map(|index| presentation.tabs[index].clone())
            .collect();
        let translate = |key: &str| self.translate_with_fallback(key, key);
        let text_width = |s: &str| HudBatch::text_width(&self.client.gui.atlas, s) as i32;
        let layout = Layout {
            book: BookType::Crafting,
            origin: (xo as i32, yo as i32),
            mouse: (self.client.cursor.0 as i32, self.client.cursor.1 as i32),
            index: self.swap_index(),
            filtering,
            filter_look: mod_api::RecipeBookFilterLook::registered(self.native_screen_book()?),
            search: &search,
            tabs: &tabs,
            buttons: &buttons,
            page: ui.page,
            pages: presentation.recipes.len().div_ceil(20),
            overlay: ui.overlay.as_ref().map(|overlay| &overlay.view),
            ghost: None,
            translate: &translate,
            text_width: &text_width,
        };
        let (calls, first_category_arrow) =
            draw_registered_book(&layout, ui, (xo, yo), self.client.cursor);
        // The tabs and arrows as painted are the region `ScreenView::regions` reports.
        if self.screen_view_listened() {
            self.client
                .screen_state
                .0
                .drawn_book_tabs
                .set(registered_book_tab_bounds(&calls, first_category_arrow));
        }
        self.paint_book(batch, &calls)
    }
    pub(in crate::game) fn mod_book_click(&mut self, right: bool) -> bool {
        self.refresh_mod_book();
        let Some((open, filtering)) = self.mod_book_flags() else {
            return false;
        };
        let Some(batch) = self.batch_for_size() else {
            return false;
        };
        let cursor = self.client.cursor;
        let consumed = self.mod_book_click_panel(&batch, open, filtering, right);
        if !consumed
            && self
                .slot_at(&batch, cursor.0, cursor.1)
                .is_some_and(|slot| slot >= 300)
        {
            self.client.recipe_ui.0.ghost.clear();
        }
        consumed
    }
    fn mod_book_click_panel(
        &mut self,
        batch: &HudBatch,
        open: bool,
        filtering: bool,
        right: bool,
    ) -> bool {
        let (bx, by) = self.mod_book_button(batch);
        let cursor = self.client.cursor;
        if inside(cursor, bx, by, 20.0, 18.0) {
            if let Some(book) = self.native_screen_book().cloned() {
                self.request_custom_book_settings(
                    &book,
                    mod_api::RecipeBookSettings {
                        open: !open,
                        filtering,
                    },
                );
            }
            return true;
        }
        if !open {
            return false;
        }
        let narrow = self.mod_book_overlay(batch.width);
        if self.common.spectator() {
            return narrow;
        }
        let (xo, yo) = self.book_origin(batch);
        if self.consume_mod_book_overlay(right) {
            return true;
        }
        if inside(cursor, xo + 110.0, yo + 12.0, 26.0, 16.0) {
            if let Some(book) = self.native_screen_book().cloned() {
                self.request_custom_book_settings(
                    &book,
                    mod_api::RecipeBookSettings {
                        open,
                        filtering: !filtering,
                    },
                );
            }
            return true;
        }
        if let Some(search) = self.client.recipe_ui.0.search.as_mut() {
            search.x = xo + 25.0;
            search.y = yo + 13.0;
            if inside(cursor, xo + 8.0, yo + 13.0, 98.0, 14.0) {
                self.client.recipe_ui.0.search_drag = true;
                search.focused = true;
                search.reset_blink();
                if cursor.0 >= search.x {
                    let index = search.index_at(&self.client.gui.atlas, cursor.0);
                    search.set_cursor(index, false);
                }
                return true;
            }
            search.focused = false;
        }
        let swap_index = self.swap_index();
        let placeable = self
            .native_screen_menu()
            .is_some_and(|menu| mod_api::RecipeBookLayout::declaring_mod(menu).is_some());
        let ui = &mut self.client.recipe_ui.0.custom;
        let Some(presentation) = &ui.presentation else {
            return narrow;
        };
        let pages = presentation.recipes.len().div_ceil(20);
        if ui.page + 1 < pages && inside(cursor, xo + 93.0, yo + 137.0, 12.0, 17.0) {
            return ui.turn_page(true);
        }
        if ui.page > 0 && inside(cursor, xo + 38.0, yo + 137.0, 12.0, 17.0) {
            return ui.turn_page(false);
        }
        for (next, [x, y, w, h], _, _) in tab_arrows(ui, xo, yo) {
            if inside(cursor, x, y, w, h) {
                return ui.turn_categories(next);
            }
        }
        for (n, index) in ui.visible_tabs().into_iter().enumerate() {
            let id = &presentation.tab_ids[index];
            if inside(cursor, xo - 30.0, yo + 3.0 + 27.0 * n as f32, 35.0, 27.0) {
                ui.selected = (index != 0).then(|| id.clone());
                ui.page = 0;
                ui.presentation = None;
                return true;
            }
        }
        for (n, recipe) in presentation
            .recipes
            .iter()
            .skip(ui.page * 20)
            .take(20)
            .enumerate()
        {
            let (x, y) = Self::button_pos(xo, yo, n);
            if inside(cursor, x, y, 25.0, 25.0) {
                if !right && placeable {
                    let id = recipe.recipe.clone();
                    let source = recipe.sources[swap_index % recipe.sources.len()];
                    self.request_mod_book_recipe(id, source);
                    return true;
                }
                let (x, y) = screen::overlay_origin(
                    x as i32,
                    y as i32,
                    recipe.overlay.len(),
                    xo as i32,
                    yo as i32,
                );
                ui.overlay = Some(placement::PlacementOverlay {
                    view: OverlayView {
                        x,
                        y,
                        recipes: recipe.overlay.clone(),
                    },
                    recipe: recipe.recipe.clone(),
                    sources: recipe.sources.clone(),
                });
                return true;
            }
        }
        narrow
    }
    pub(in crate::game) fn mod_book_area(&self, batch: &HudBatch) -> Option<(bool, bool)> {
        let (open, _) = self.mod_book_flags()?;
        let (xo, yo) = self.book_origin(batch);
        let ui = &self.client.recipe_ui.0.custom;
        let hovered = ui.presentation.as_ref().is_some_and(|p| {
            ui.visible_tabs().into_iter().enumerate().any(|(n, index)| {
                p.tabs[index].selected
                    && inside(
                        self.client.cursor,
                        xo - 30.0,
                        yo + 3.0 + 27.0 * n as f32,
                        35.0,
                        27.0,
                    )
            })
        });
        let arrow = tab_arrows(ui, xo, yo)
            .any(|(_, [x, y, w, h], _, _)| inside(self.client.cursor, x, y, w, h));
        Some((open, open && (hovered || arrow)))
    }
    pub(in crate::game) fn mod_book_targets(&self) -> Vec<sunrise_client::widgets::Target> {
        use sunrise_client::widgets::{Kind, Target};
        let (Some((open, _)), Some(batch)) = (self.mod_book_flags(), self.batch_for_size()) else {
            return Vec::new();
        };
        let button = |rect| Target {
            rect,
            kind: Kind::Button,
        };
        let (bx, by) = self.mod_book_button(&batch);
        let mut targets = vec![button([bx, by, 20.0, 18.0])];
        if !open || self.common.spectator() {
            return targets;
        }
        let (xo, yo) = self.book_origin(&batch);
        let ui = &self.client.recipe_ui.0.custom;
        if let Some(overlay) = &ui.overlay {
            let columns = if overlay.view.recipes.len() <= 16 {
                4
            } else {
                5
            };
            return (0..overlay.view.recipes.len())
                .map(|n| {
                    button([
                        overlay.view.x as f32 + 4.0 + 25.0 * (n % columns) as f32,
                        overlay.view.y as f32 + 5.0 + 25.0 * (n / columns) as f32,
                        24.0,
                        24.0,
                    ])
                })
                .collect();
        }
        targets.push(Target {
            rect: [xo + 25.0, yo + 13.0, 81.0, 14.0],
            kind: Kind::Field,
        });
        targets.push(button([xo + 110.0, yo + 12.0, 26.0, 16.0]));
        let Some(p) = &ui.presentation else {
            return targets;
        };
        targets.extend(tab_arrows(ui, xo, yo).map(|(_, rect, _, _)| button(rect)));
        for (n, index) in ui.visible_tabs().into_iter().enumerate() {
            let tab = &p.tabs[index];
            targets.push(Target {
                rect: [xo - 30.0, yo + 3.0 + 27.0 * n as f32, 35.0, 27.0],
                kind: Kind::Tab(tab.selected),
            });
        }
        for n in 0..p.recipes.len().saturating_sub(ui.page * 20).min(20) {
            let (x, y) = Self::button_pos(xo, yo, n);
            targets.push(button([x, y, 25.0, 25.0]));
        }
        if ui.page > 0 {
            targets.push(button([xo + 38.0, yo + 137.0, 12.0, 17.0]));
        }
        if ui.page + 1 < p.recipes.len().div_ceil(20) {
            targets.push(button([xo + 93.0, yo + 137.0, 12.0, 17.0]));
        }
        targets
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use sunrise_sim::recipe_book::mod_presentation::PresentedRecipe;

    #[test]
    fn overflow_tabs_are_reachable_without_moving_search_offscreen() {
        let tabs = (0..14)
            .map(|_| TabView {
                icons: ("minecraft:stone".into(), None),
                selected: false,
                anim: 0.0,
            })
            .collect();
        let mut ui = ModRecipeUi {
            presentation: Some(ModPresentation {
                tab_ids: (0..14).map(|i| format!("test:tab{i}")).collect(),
                tabs,
                recipes: Vec::new(),
            }),
            ..ModRecipeUi::default()
        };
        let mut reached = BTreeSet::new();
        loop {
            let indices = ui.visible_tabs();
            assert_eq!(indices[0], 0);
            assert!(indices.len() <= 5);
            reached.extend(indices);
            if !ui.turn_categories(true) {
                break;
            }
        }
        assert_eq!(reached, (0..14).collect());
        assert_eq!(ui.visible_tabs(), vec![0, 13]);
        assert!(ui.turn_categories(false));
        assert_eq!(ui.visible_tabs(), vec![0, 9, 10, 11, 12]);
    }

    #[test]
    fn the_registered_book_region_excludes_recipe_page_arrows() {
        let tab = TabView {
            icons: ("minecraft:compass".into(), None),
            selected: true,
            anim: 0.0,
        };
        let layout = Layout {
            book: BookType::Crafting,
            origin: (54, 37),
            mouse: (0, 0),
            index: 0,
            filtering: false,
            filter_look: None,
            search: &Search::default(),
            tabs: &[tab],
            buttons: &[],
            page: 0,
            pages: 2,
            overlay: None,
            ghost: None,
            translate: &str::to_owned,
            text_width: &|s| s.len() as i32,
        };
        // Search plus five categories gives two category pages. Both forward
        // arrows then use the same sprite, but only the category arrow is a tab region.
        let ui = ModRecipeUi {
            presentation: Some(ModPresentation {
                tabs: vec![layout.tabs[0].clone(); 6],
                tab_ids: (0..6).map(|i| format!("test:tab{i}")).collect(),
                recipes: Vec::new(),
            }),
            ..ModRecipeUi::default()
        };
        let (calls, first_category_arrow) =
            draw_registered_book(&layout, &ui, (54.0, 37.0), (0.0, 0.0));
        assert!(calls.iter().any(|drawn| {
            matches!(&drawn.call, Call::Sprite { id, x: 147, .. }
                if id == "recipe_book/page_forward")
        }));
        assert_eq!(
            registered_book_tab_bounds(&calls, first_category_arrow),
            Some([22.0, 40.0, 35.0, 157.0])
        );
    }

    #[test]
    fn paging_stays_within_the_current_view_and_dismisses_preview() {
        let mut ui = ModRecipeUi::default();
        assert!(!ui.turn_page(true));
        assert!(!ui.turn_page(false));
        ui.presentation = Some(ModPresentation {
            tab_ids: vec!["test:search".into()],
            tabs: vec![TabView {
                icons: ("minecraft:compass".into(), None),
                selected: true,
                anim: 0.0,
            }],
            recipes: (0..21)
                .map(|index| PresentedRecipe {
                    recipe: format!("test:r{index}"),
                    sources: vec![0],
                    button: ButtonView {
                        has_craftable: true,
                        entries: vec![vec![ItemStack::new("minecraft:stone", 1)]],
                        anim: 0.0,
                    },
                    overlay: Vec::new(),
                })
                .collect(),
        });
        ui.overlay = Some(placement::PlacementOverlay {
            view: OverlayView {
                x: 0,
                y: 0,
                recipes: Vec::new(),
            },
            recipe: "test:recipe".into(),
            sources: Vec::new(),
        });
        assert!(ui.turn_page(true));
        assert_eq!(ui.page, 1);
        assert!(ui.overlay.is_none());
        assert!(!ui.turn_page(true));
        assert_eq!(ui.page, 1);
        assert!(ui.turn_page(false));
        assert_eq!(ui.page, 0);
        assert!(!ui.turn_page(false));
    }
}
