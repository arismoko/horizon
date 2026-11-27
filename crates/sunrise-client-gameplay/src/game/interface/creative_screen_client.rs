use super::*;
use sunrise_items::creative_catalog::{item_group_key, mod_item_group_key};

impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn creative_origin(batch: &HudBatch) -> (f32, f32) {
        batch.centered_origin(Creative::WIDTH, Creative::HEIGHT)
    }

    pub(in crate::game) fn click_creative(&mut self, right: bool) -> bool {
        if self.mod_browser_press(right) {
            return true;
        }
        let Some(size) = self
            .client
            .renderer
            .0
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return false;
        };
        let batch = HudBatch::new(size, self.client.settings.gui_scale_inputs());
        let (ox, oy) = Self::creative_origin(&batch);
        let (cx, cy) = self.client.cursor;
        // NeoForge's page buttons, when mods add tabs.
        if let Some(step) =
            Self::creative_page_button(ox, oy, (cx, cy), self.client.creative_screen.0.pages())
        {
            // Page buttons are outside the inventory panel; their release must
            // not throw a carried stack, including when the button is disabled.
            self.client.inv_clicks.skip_release();
            let page = self.client.creative_screen.0.page as i64 + step;
            if (0..self.client.creative_screen.0.pages() as i64).contains(&page) {
                self.play_audio_button_click();
                self.client.creative_screen.0.set_page(page as u32);
                self.refresh_saved_hotbars();
            }
            return true;
        }
        // Only the clicked tab's index is needed: cloning the tabs here copied
        // every tab's items on each click, saved hotbars' stacks included.
        let creative = &self.client.creative_screen.0;
        let clicked = creative.tabs.iter().position(|t| {
            let (tx, ty) = Creative::tab_position(t);
            creative.on_page(t)
                && cx >= ox + tx
                && cx < ox + tx + 26.0
                && cy >= oy + ty
                && cy < oy + ty + 32.0
        });
        if let Some(i) = clicked {
            select_creative_tab(
                &mut self.client.creative_screen.0,
                &mut self.client.inv_clicks,
                i,
            );
            self.build_screen();
            self.refresh_saved_hotbars();
            self.client.inv_drag = None;
            self.client.last_inv_click = None;
            return true;
        }
        if self.creative_inventory_open() {
            if Self::creative_trash_hovered(&batch, self.client.cursor) {
                if self.has_shift_down() {
                    self.common.inventory.fill(None);
                    self.common.armor.fill(None);
                    self.common.offhand = None;
                    self.common.equipment = Default::default();
                    self.common.craft_grid[..4].fill(None);
                    self.apply_equipment();
                    if self.client.remote.is_some() {
                        self.send_creative_slots();
                    }
                } else {
                    self.common.carried = None;
                }
                self.client.inv_drag = None;
                self.client.inv_clicks = Default::default();
                return true;
            } else if self.slot_at(&batch, cx, cy).is_some() || self.clicked_outside(&batch) {
                return self.inventory_press(right);
            }
            return false;
        }
        if !right && self.creative_search_click(ox, oy) {
            return true;
        }
        // Holding the scrollbar scrolls as the mouse moves (mouseDragged).
        if !right
            && self.client.creative_screen.0.press_scrollbar(
                cx - ox,
                cy - oy,
                &self.common.pack.names,
            )
        {
            return true;
        }
        if self.slot_at(&batch, cx, cy).is_some() || self.clicked_outside(&batch) {
            return self.inventory_press(right);
        }
        false
    }
    /// The stack at a resolved display slot on the active item tab, accounting for scroll.
    pub fn creative_display_stack(&self, slot: usize) -> Option<ItemStack> {
        let shown = self
            .client
            .creative_screen
            .0
            .display_window(&self.common.pack.names);
        shown.stack(slot).cloned()
    }
    pub fn creative_inventory_open(&self) -> bool {
        self.client.screen == Screen::Creative && self.client.creative_screen.0.tab().inventory
    }
    /// NeoForge's creative page buttons, 20 square, 50 above the screen at
    /// its left and right edges: -1 or 1 for the one at `cursor`, when there
    /// is more than one page.
    pub(in crate::game::client_side) fn creative_page_button(
        ox: f32,
        oy: f32,
        cursor: (f32, f32),
        pages: u32,
    ) -> Option<i64> {
        if pages <= 1 {
            return None;
        }
        let y = oy - 50.0;
        let inside =
            |x: f32| cursor.0 >= x && cursor.0 < x + 20.0 && cursor.1 >= y && cursor.1 < y + 20.0;
        if inside(ox) {
            Some(-1)
        } else if inside(ox + Creative::WIDTH - 20.0) {
            Some(1)
        } else {
            None
        }
    }
    /// A click on the search tab's box (`EditBox.onClick`).
    pub(in crate::game::client_side) fn creative_search_click(&mut self, ox: f32, oy: f32) -> bool {
        if !self.client.creative_screen.0.tab().search {
            return false;
        }
        let (x, y, w, h) = (ox + 82.0, oy + 6.0, 80.0, 9.0);
        let (mx, my) = self.client.cursor;
        if mx < x || my < y || mx >= x + w || my >= y + h {
            return false;
        }
        let shift = self.has_shift_down();
        let search = &mut self.client.creative_screen.0.search;
        search.x = x;
        search.y = y;
        search.width = w;
        search.click(&self.client.gui.atlas, mx, shift, false);
        true
    }
    /// An item tab of the creative screen (its ItemPickerMenu).
    pub fn creative_tab_open(&self) -> bool {
        self.client.screen == Screen::Creative && !self.client.creative_screen.0.tab().inventory
    }
    /// A creative tab's title: `itemGroup.*` for vanilla's; a mod tab's
    /// `itemGroup.<mod>.<name>`, else its `title` as a lang key (a mod's own
    /// `itemGroup.<mod>`), else `title` as English text.
    pub(in crate::game::client_side) fn creative_tab_title(
        &self,
        tab: &sunrise_client::creative::Tab,
    ) -> String {
        if let Some((name, _, translate)) = &tab.name {
            return if *translate {
                self.common.translate(name).unwrap_or_else(|| name.clone())
            } else {
                name.clone()
            };
        }
        match item_group_key(&tab.id) {
            Some(key) => self.translate_with_fallback(key, &tab.title),
            None => {
                let own = self.translate_with_fallback(&tab.title, &tab.title);
                self.translate_with_fallback(&mod_item_group_key(&tab.id), &own)
            }
        }
    }
    /// `CreativeModeInventoryScreen.getTooltipFromContainerItem`: the lines
    /// that follow the item's name, none for a category tab's own grid slots
    /// (`grid`): the names of the tabs containing the stack, in blue, then on
    /// the search tab's grid the tags a `#` search shows, in dark purple.
    pub(in crate::game::client_side) fn creative_tooltip_tabs(
        &self,
        stack: &ItemStack,
        grid: bool,
    ) -> Vec<(sunrise_game_data::text::Text, u32)> {
        let screen = &self.client.creative_screen.0;
        let tab = screen.tab();
        let category = !tab.search && !tab.inventory;
        if !sunrise_client::creative::tooltip_lists_tabs(category, grid) {
            return Vec::new();
        }
        let blue = rgba(85, 85, 255, 255);
        let dark_purple = rgba(170, 0, 170, 255);
        let mut lines: Vec<(sunrise_game_data::text::Text, u32)> = screen
            .tabs_containing(stack)
            .into_iter()
            .map(|i| {
                let title = self.creative_tab_title(&screen.tabs[i]);
                (sunrise_game_data::text::Text::literal(title), blue)
            })
            .collect();
        if tab.search && grid {
            for tag in screen.search_tag_lines(stack) {
                lines.push((sunrise_game_data::text::Text::literal(tag), dark_purple));
            }
        }
        lines
    }
    pub(in crate::game::client_side) fn creative_trash_hovered(
        batch: &HudBatch,
        cursor: (f32, f32),
    ) -> bool {
        let (ox, oy) = Self::creative_origin(batch);
        let (x, y) = Creative::TRASH_POSITION;
        cursor.0 >= ox + x
            && cursor.0 < ox + x + 16.0
            && cursor.1 >= oy + y
            && cursor.1 < oy + y + 16.0
    }
    /// `CreativeModeInventoryScreen.extractRenderState`: item slots share the
    /// container screen's hover target for their highlight and tooltip. False if no tab was drawn.
    pub(in crate::game) fn draw_creative(&mut self, batch: &mut HudBatch) -> bool {
        // CreativeModeInventoryScreen: `CommonComponents.EMPTY` for a title.
        screen_info::begin_container("creative");
        let (ox, oy) = Self::creative_origin(batch);
        let selected = self.client.creative_screen.0.selected_index();
        // Only what is drawn of the tabs: their stacks stay where they are.
        let tabs = self.client.creative_screen.0.tab_headers();
        let Some(tab) = tabs.get(selected.min(tabs.len().saturating_sub(1))) else {
            return false;
        };
        let tab_sprite = |t: &sunrise_client::creative::Tab, on: bool| {
            format!(
                "container/creative_inventory/tab_{}_{}_{}",
                if t.top { "top" } else { "bottom" },
                if on { "selected" } else { "unselected" },
                t.column + 1
            )
        };
        self.render_transparent_background("creative", batch);
        // Unselected tabs behind the panel, then the panel, then the selected tab.
        for (i, t) in tabs.iter().enumerate() {
            if i != selected && self.client.creative_screen.0.on_page(t) {
                let (tx, ty) = Creative::tab_position(t);
                batch.sprite(
                    &self.client.gui.atlas,
                    &tab_sprite(t, false),
                    ox + tx,
                    oy + ty,
                );
                let mouse = (self.client.cursor.0 - ox, self.client.cursor.1 - oy);
                if Creative::tab_wants_hand(t, false, mouse) {
                    sunrise_client::widgets::request_cursor(
                        sunrise_client::widgets::Cursor::PointingHand,
                    );
                }
            }
        }
        let background = if tab.search {
            "gui/container/creative_inventory/tab_item_search"
        } else if tab.inventory {
            "gui/container/creative_inventory/tab_inventory"
        } else {
            "gui/container/creative_inventory/tab_items"
        };
        if let Some(mut r) = self.client.gui.atlas.sprites.get(background).copied() {
            r.u1 = r.u0 + (r.u1 - r.u0) * Creative::WIDTH / r.w as f32;
            r.v1 = r.v0 + (r.v1 - r.v0) * Creative::HEIGHT / r.h as f32;
            batch.rect_uv(ox, oy, Creative::WIDTH, Creative::HEIGHT, r, u32::MAX);
        }
        let (tx, ty) = Creative::tab_position(tab);
        batch.sprite(
            &self.client.gui.atlas,
            &tab_sprite(tab, true),
            ox + tx,
            oy + ty,
        );
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        for t in tabs
            .iter()
            .filter(|t| self.client.creative_screen.0.on_page(t))
        {
            let (tx, ty) = Creative::tab_position(t);
            let stack = ItemStack {
                item: Arc::from(t.icon.as_str()),
                count: 1,
                damage: 0,
                extra: None,
            };
            self.draw_item(
                batch,
                &stack,
                ox + tx + 5.0,
                oy + ty + if t.top { 9.0 } else { 7.0 },
            );
        }
        let atlas = &self.client.gui.atlas;
        if !tab.inventory {
            let title = self.creative_tab_title(tab);
            batch.text(
                atlas,
                &title,
                ox + 8.0,
                oy + 6.0,
                tab.name
                    .as_ref()
                    .and_then(|n| n.1)
                    .map_or(rgba(64, 64, 64, 255), |c| {
                        rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, 255)
                    }),
                false,
            );
        }
        if tab.search {
            // CreativeModeInventoryScreen: an unbordered EditBox of 50 at (82, 6).
            let search = &mut self.client.creative_screen.0.search;
            search.x = ox + 82.0;
            search.y = oy + 6.0;
            search.width = 80.0;
            search.bordered = false;
            search.focused = true;
            search.text_color = u32::MAX;
            search.max_length = clipboard::CREATIVE_SEARCH_CHARS;
            if sunrise_client::widgets::narrating() {
                search.message = screen_info::tr("itemGroup.search", &[]);
            }
            search.draw(batch, atlas, None);
        }
        // The hovered stack, and whether it is a grid slot (`CustomCreativeSlot`).
        let mut hovered: Option<(ItemStack, bool)> = None;
        // Use the same padded slot bounds as clicks and the other menus, so
        // highlights and tooltips remain visible between adjacent items.
        let hovered_slot = self.slot_at(batch, self.client.cursor.0, self.client.cursor.1);
        if tab.inventory {
            self.draw_player_preview(
                batch,
                ox + 73.0,
                oy + 6.0,
                ox + 105.0,
                oy + 49.0,
                20.0,
                0.0625,
            );
            let trash = Self::creative_trash_hovered(batch, self.client.cursor);
            let (tx, ty) = Creative::TRASH_POSITION;
            if trash {
                self.draw_slot_highlight(batch, false, ox + tx, oy + ty);
            }
            self.draw_hovered_slot_highlight(batch, false, ox, oy);
            self.draw_inventory_slots(batch, ox, oy);
            if trash {
                self.draw_slot_highlight(batch, true, ox + tx, oy + ty);
            }
            self.draw_hovered_slot_highlight(batch, true, ox, oy);
        } else {
            let shown = self
                .client
                .creative_screen
                .0
                .display_window(&self.common.pack.names);
            for (slot, (x, y)) in Creative::display_slots() {
                let Some(stack) = shown.stack(slot) else {
                    continue;
                };
                let (x, y) = (ox + x, oy + y);
                let over = hovered_slot == Some(slot);
                if over {
                    self.draw_slot_highlight(batch, false, x, y);
                }
                self.draw_item(batch, stack, x, y);
                if over {
                    self.draw_slot_highlight(batch, true, x, y);
                    hovered = Some((stack.clone(), true));
                }
            }
            let drag = self.drag_view();
            for c in 0..9 {
                let (x, y) = (ox + 9.0 + c as f32 * 18.0, oy + 112.0);
                let over = hovered_slot == Some(c);
                if over {
                    self.draw_slot_highlight(batch, false, x, y);
                }
                self.draw_slot_item(batch, drag.as_ref(), c, &self.common.inventory[c], x, y);
                if over {
                    self.draw_slot_highlight(batch, true, x, y);
                    hovered = self.common.inventory[c].clone().map(|s| (s, false));
                }
            }
            let scroller = if shown.can_scroll() {
                "container/creative_inventory/scroller"
            } else {
                "container/creative_inventory/scroller_disabled"
            };
            batch.sprite(
                &self.client.gui.atlas,
                scroller,
                ox + Creative::SCROLL_X,
                oy + Creative::SCROLL_Y + self.client.creative_screen.0.scroller_offset(),
            );
        }
        let effect_tip = self.draw_inventory_effects(batch, (ox, oy), Creative::WIDTH as i32);
        self.draw_creative_pages(batch, ox, oy);
        let browser_tip = self.draw_mod_browser(batch);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        self.draw_carried(batch);
        if tab.inventory {
            if Self::creative_trash_hovered(batch, self.client.cursor) {
                // `CreativeModeInventoryScreen`: the destroy slot's tooltip is
                // `inventory.binSlot`, not an English literal.
                let destroy = self.translate_with_fallback("inventory.binSlot", "Destroy Item");
                self.tooltip(batch, &destroy, self.client.cursor.0, self.client.cursor.1);
            } else if let Some(slot) =
                self.slot_at(batch, self.client.cursor.0, self.client.cursor.1)
                && let Some(stack) = self.slot_view(slot)
                && self.tooltip_with_carried(&stack)
            {
                let tabs = self.creative_tooltip_tabs(&stack, false);
                self.draw_stack_tooltip_with(
                    batch,
                    &stack,
                    false,
                    self.client.cursor.0,
                    self.client.cursor.1,
                    tabs,
                );
            }
        } else if let Some((item, grid)) = hovered
            && self.tooltip_with_carried(&item)
        {
            let tabs = self.creative_tooltip_tabs(&item, grid);
            // A `CustomCreativeSlot`: TooltipFlag.asCreative.
            self.draw_stack_tooltip_with(
                batch,
                &item,
                grid,
                self.client.cursor.0,
                self.client.cursor.1,
                tabs,
            );
        }
        if let Some(lines) = effect_tip {
            self.draw_tooltip_lines(batch, &lines, self.client.cursor.0, self.client.cursor.1);
        }
        // CreativeModeInventoryScreen.checkTabHovering: a tab says its name.
        if let Some(i) = self
            .client
            .creative_screen
            .0
            .hovered_tab((ox, oy), self.client.cursor)
            && let Some(t) = tabs.get(i)
        {
            let title = self.creative_tab_title(t);
            self.tooltip(batch, &title, self.client.cursor.0, self.client.cursor.1);
        }
        if let Some(tooltip) = browser_tip {
            self.draw_mod_browser_tooltip(batch, tooltip);
        }
        true
    }
    /// The page buttons and "page / pages" between them.
    pub(in crate::game::client_side) fn draw_creative_pages(
        &self,
        batch: &mut HudBatch,
        ox: f32,
        oy: f32,
    ) {
        let pages = self.client.creative_screen.0.pages();
        if pages <= 1 {
            return;
        }
        let atlas = &self.client.gui.atlas;
        let y = oy - 50.0;
        for (x, label, step) in [(ox, "<", -1), (ox + Creative::WIDTH - 20.0, ">", 1)] {
            let hover = Self::creative_page_button(ox, oy, self.client.cursor, pages) == Some(step);
            let page = self.client.creative_screen.0.page as i64 + step;
            let active = (0..pages as i64).contains(&page);
            let sprite = match (active, hover) {
                (false, _) => "widget/button_disabled",
                (true, true) => "widget/button_highlighted",
                (true, false) => "widget/button",
            };
            sunrise_client::ui::nine_slice(batch, atlas, sprite, x, y, 20.0, 20.0, u32::MAX);
            let color = if active {
                u32::MAX
            } else {
                rgba(160, 160, 160, 255)
            };
            let w = HudBatch::text_width(atlas, label);
            batch.text(atlas, label, x + (20.0 - w) / 2.0, y + 6.0, color, true);
        }
        let text = format!("{} / {}", self.client.creative_screen.0.page + 1, pages);
        let w = HudBatch::text_width(atlas, &text);
        batch.text(
            atlas,
            &text,
            ox + (Creative::WIDTH - w) / 2.0,
            oy - 44.0,
            u32::MAX,
            true,
        );
    }
    /// Vanilla tooltip: dark box with a purple gradient border, laid out and
    /// drawn like an item tooltip (4 px of padding on every side).
    pub(in crate::game) fn tooltip(&self, batch: &mut HudBatch, text: &str, x: f32, y: f32) {
        self.draw_tooltip_lines(
            batch,
            &[(sunrise_game_data::text::Text::literal(text), u32::MAX)],
            x,
            y,
        );
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
