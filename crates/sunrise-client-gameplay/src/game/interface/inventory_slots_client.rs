use super::*;
use sunrise_gameplay_common::game::inventory_slots::MOD_MENU_SLOT_BASE;
#[cfg(test)]
use sunrise_gameplay_common::game::inventory_slots::SLOT_POSITIONS;
use sunrise_gameplay_common::game::inventory_slots::container_slot_index;

impl crate::game::ClientGame<'_> {
    pub fn craft_result(&self) -> Option<ItemStack> {
        // A server's result slot is the server's (`CraftingMenu.slotChangedCraftingGrid`).
        if let Some(r) = &self.client.remote {
            return r.craft_result.clone();
        }
        self.craft_recipe().map(|(_, result)| result)
    }
    /// The quick craft drag in progress, as the menu engine previews it.
    pub(in crate::game) fn drag_view(&mut self) -> Option<DragView> {
        let (ty, slots) = self.client.inv_drag.clone()?;
        let mut view = None;
        self.menu_session(|menu, env, to_menu| {
            let menu_slots: Option<Vec<usize>> = slots.iter().map(|&g| to_menu(g)).collect();
            let shown = match menu.quick_craft_preview(env, ty, &menu_slots?) {
                Some((shown, left)) => {
                    let slots = slots.iter().copied().zip(shown).collect();
                    DragView {
                        slots,
                        left: Some(left),
                    }
                }
                // A single dragged slot is hidden.
                None => DragView {
                    slots: slots.iter().map(|&g| (g, None)).collect(),
                    left: None,
                },
            };
            view = Some(shown);
            Some(false)
        });
        view
    }
    /// `extractCarriedItem`: the carried stack at the cursor, showing what a
    /// quick craft drag over several slots leaves in hand.
    pub fn draw_carried(&mut self, batch: &mut HudBatch) {
        let Some(mut stack) = self.common.carried.clone() else {
            return;
        };
        if let Some(left) = self.drag_view().and_then(|d| d.left) {
            if left == 0 {
                return;
            }
            stack.count = left;
        }
        self.draw_item(
            batch,
            &stack,
            self.client.cursor.0 - 8.0,
            self.client.cursor.1 - 8.0,
        );
    }
    /// The highlight of the slot under the cursor, for the screens whose
    /// slots all sit in `slot_at`'s grid. `Slot.isHighlightable` is false
    /// for the crafter's result (`NonInteractiveResultSlot`).
    pub(in crate::game) fn draw_hovered_slot_highlight(
        &self,
        batch: &mut HudBatch,
        front: bool,
        ox: f32,
        oy: f32,
    ) {
        let Some(slot) = self.slot_at(batch, self.client.cursor.0, self.client.cursor.1) else {
            return;
        };
        if slot == 309 && matches!(self.container_kind(), Some(BlockEntity::Crafter(_))) {
            return;
        }
        let (sx, sy) = self.slot_position(slot);
        self.draw_slot_highlight(batch, front, ox + sx, oy + sy);
    }
    /// True when the inventory painter ran, not a station without its side regions.
    pub(in crate::game) fn draw_inventory(&mut self, batch: &mut HudBatch) -> bool {
        screen_info::begin_container("container");
        if self.draw_station(batch) {
            return false;
        }
        let (ox, oy) = self.inventory_origin(batch);
        {
            let atlas = &self.client.gui.atlas;
            self.render_transparent_background("inventory", batch);
            let part = |batch: &mut HudBatch,
                        name: &str,
                        sx: f32,
                        sy: f32,
                        w: f32,
                        h: f32,
                        dx: f32,
                        dy: f32| {
                if let Some(mut r) = atlas.gui_rect(name).copied() {
                    let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
                    r.u0 += sx * du;
                    r.v0 += sy * dv;
                    r.u1 = r.u0 + w * du;
                    r.v1 = r.v0 + h * dv;
                    batch.rect_uv(dx, dy, w, h, r, u32::MAX);
                }
            };
            let grey = rgba(64, 64, 64, 255);
            match self
                .common
                .container
                .and_then(|k| self.common.world.block_entities.get(&k))
            {
                Some(_) if self.draw_native_screen(batch, ox, oy) => {}
                Some(BlockEntity::Chest(_)) if self.draw_machine_screen(batch, ox, oy) => {}
                Some(BlockEntity::Chest(_)) => self.draw_chest_screen(batch, ox, oy),
                Some(BlockEntity::Furnace(f)) => {
                    let (tex, key, fallback) = match f.kind {
                        sunrise_level::containers::FurnaceKind::Furnace => {
                            ("gui/container/furnace", "container.furnace", "Furnace")
                        }
                        sunrise_level::containers::FurnaceKind::BlastFurnace => (
                            "gui/container/blast_furnace",
                            "container.blast_furnace",
                            "Blast Furnace",
                        ),
                        sunrise_level::containers::FurnaceKind::Smoker => {
                            ("gui/container/smoker", "container.smoker", "Smoker")
                        }
                    };
                    part(batch, tex, 0.0, 0.0, 176.0, 166.0, ox, oy);
                    self.draw_screen_phase(
                        batch,
                        sunrise_client::screen_draw::RenderPhase::Background,
                    );
                    // A server's furnace menu has the title it was given.
                    let title = self.remote_menu_title().unwrap_or_else(|| {
                        sunrise_game_data::text::Text::literal(
                            self.container_title(self.translate_with_fallback(key, fallback)),
                        )
                        .resolve_legacy()
                    });
                    screen_info::name_container(|| title.plain());
                    let tw = atlas.font.width_text(&title);
                    batch.text_styled(
                        atlas,
                        &title,
                        ox + (176.0 - tw) / 2.0,
                        oy + 6.0,
                        grey,
                        false,
                    );
                    let inventory =
                        self.translate_with_fallback("container.inventory", "Inventory");
                    batch.text(atlas, &inventory, ox + 8.0, oy + 72.0, grey, false);
                    if f.lit_remaining > 0 {
                        // AbstractFurnaceScreen: flame height ceil(lit * 13) + 1.
                        let total = if f.lit_total == 0 { 200 } else { f.lit_total };
                        let h = ((f.lit_remaining as f32 / total as f32) * 13.0).ceil() + 1.0;
                        part(
                            batch,
                            "container/furnace/lit_progress",
                            0.0,
                            14.0 - h,
                            14.0,
                            h,
                            ox + 56.0,
                            oy + 36.0 + 14.0 - h,
                        );
                    }
                    if f.cooking_timer > 0 && f.cooking_total > 0 {
                        let w = ((f.cooking_timer as f32 / f.cooking_total as f32) * 24.0).ceil();
                        part(
                            batch,
                            "container/furnace/burn_progress",
                            0.0,
                            0.0,
                            w,
                            16.0,
                            ox + 79.0,
                            oy + 34.0,
                        );
                    }
                }
                Some(BlockEntity::Brewing(_)) => {}
                Some(BlockEntity::Crafter(_)) => self.draw_crafter_screen(batch, ox, oy),
                Some(
                    BlockEntity::Sign(_)
                    | BlockEntity::Jukebox(_)
                    | BlockEntity::Beacon(_)
                    | BlockEntity::Lectern(_)
                    | BlockEntity::Sculk(_)
                    | BlockEntity::Banner(_)
                    | BlockEntity::CopperGolemStatue(_)
                    | BlockEntity::CreakingHeart(_)
                    | BlockEntity::Spawner(_)
                    | BlockEntity::TrialSpawner(_)
                    | BlockEntity::Vault(_)
                    | BlockEntity::Beehive(_)
                    | BlockEntity::CommandBlock(_)
                    | BlockEntity::EndGateway(_)
                    | BlockEntity::Skull(_)
                    | BlockEntity::DaylightDetector
                    | BlockEntity::Bell
                    | BlockEntity::Conduit(_)
                    | BlockEntity::EndPortal
                    | BlockEntity::EnchantingTable
                    | BlockEntity::Campfire(_)
                    | BlockEntity::ChiseledBookshelf(_)
                    | BlockEntity::Brushable(_)
                    | BlockEntity::Shelf(_)
                    | BlockEntity::Mod(_)
                    | BlockEntity::PotentSulfur(_)
                    | BlockEntity::StructureBlock(_)
                    | BlockEntity::JigsawBlock(_)
                    | BlockEntity::TestBlock(_)
                    | BlockEntity::TestInstanceBlock(_),
                ) => self
                    .draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background),
                None if self.mount_active() => self.draw_mount_background(batch, ox, oy),
                None => {
                    let texture = if self.common.craft_size == 3 {
                        "gui/container/crafting_table"
                    } else {
                        "gui/container/inventory"
                    };
                    part(batch, texture, 0.0, 0.0, 176.0, 166.0, ox, oy);
                    self.draw_screen_phase(
                        batch,
                        sunrise_client::screen_draw::RenderPhase::Background,
                    );
                    let title = self.translate_with_fallback("container.crafting", "Crafting");
                    screen_info::name_container(|| title.clone());
                    let tx = if self.common.craft_size == 3 {
                        29.0
                    } else {
                        97.0
                    };
                    batch.text(atlas, &title, ox + tx, oy + 6.0, grey, false);
                    if self.common.craft_size == 3 {
                        let inventory =
                            self.translate_with_fallback("container.inventory", "Inventory");
                        batch.text(atlas, &inventory, ox + 8.0, oy + 72.0, grey, false);
                    }
                }
            }
        }
        if self.common.container.is_none() && self.common.craft_size != 3 && !self.mount_active() {
            self.draw_inventory_player(batch, ox, oy);
        }
        self.draw_hovered_slot_highlight(batch, false, ox, oy);
        self.draw_inventory_slots(batch, ox, oy);
        self.draw_hovered_slot_highlight(batch, true, ox, oy);
        self.draw_sort_buttons(batch, ox, oy);
        let book_tip = self.draw_recipe_book(batch);
        let effect_tip = if self.common.container.is_none()
            && self.common.craft_size == 2
            && !self.mount_active()
        {
            self.draw_inventory_effects(batch, (ox, oy), 176)
        } else {
            None
        };
        let browser_tip = self.draw_mod_browser(batch);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        if let Some(lines) = book_tip {
            self.draw_tooltip_lines(batch, &lines, self.client.cursor.0, self.client.cursor.1);
        }
        self.draw_carried(batch);
        self.draw_slot_tooltip(batch);
        self.draw_native_screen_tooltip(batch);
        if let Some(lines) = effect_tip {
            self.draw_tooltip_lines(batch, &lines, self.client.cursor.0, self.client.cursor.1);
        }
        if let Some(tooltip) = browser_tip {
            self.draw_mod_browser_tooltip(batch, tooltip);
        }
        true
    }
    pub(in crate::game) fn draw_inventory_slots(&mut self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let drag = self.drag_view();
        for i in self.slots() {
            let (sx, sy) = self.slot_position(i);
            if i == 504 && self.common.offhand.is_none() {
                batch.sprite(
                    &self.client.gui.atlas,
                    "container/slot/shield",
                    ox + sx,
                    oy + sy,
                );
            }
            if (500..504).contains(&i) && self.common.armor[i - 500].is_none() {
                let icon = ["helmet", "chestplate", "leggings", "boots"][i - 500];
                batch.sprite(
                    &self.client.gui.atlas,
                    &format!("container/slot/{icon}"),
                    ox + sx,
                    oy + sy,
                );
            }
            let stack = self.slot_view(i);
            self.draw_slot_item(batch, drag.as_ref(), i, &stack, ox + sx, oy + sy);
        }
    }
    pub(in crate::game) fn draw_item(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        x: f32,
        y: f32,
    ) {
        // `GuiGraphics.renderItem`: the resolved item model (tints, foil and
        // special renderers included), then the decorations.
        self.draw_item_model(batch, stack, x, y, 0);
        self.draw_item_decorations(batch, stack, x, y, None);
    }
    /// `GuiGraphicsExtractor.itemDecorations`: the count (or `count_text`)
    /// and damage bar.
    pub(in crate::game) fn draw_item_decorations(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        x: f32,
        y: f32,
        count_text: Option<&str>,
    ) {
        self.draw_vanilla_item_decorations(batch, stack, x, y, count_text);
        let hook = mod_api::HookId::ItemDecorations;
        if mod_api::__private::has_part_hook(hook) || mod_api::__private::listened(hook) {
            // The ordinary inventory, hotbar and tooltip paths draw in screen GUI units.
            self.draw_item_decorator(batch, stack, x, y);
        }
    }

    /// Draws the standard suffix before a scoped caller lends its canvas to a decorator.
    pub(in crate::game) fn draw_vanilla_item_decorations(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        x: f32,
        y: f32,
        count_text: Option<&str>,
    ) {
        // GuiGraphicsExtractor.itemDecorations, laid out at the origin and
        // moved to (x, y): the damage bar, the cooldown shade and the count.
        let items = self.items();
        let info = items.get(&stack.item);
        let cooldown =
            sunrise_items::cooldowns::group(stack, info.and_then(|i| i.use_cooldown.as_ref()));
        let partial = self.common.clock.fraction() as f32;
        let mut bar = sunrise_items::properties::stack_bar(items, stack, info)
            .map(|bar| (bar.width, bar.color));
        // Client-only: a mod may replace or hide the bar vanilla just chose.
        self.native_item_bar(stack, &mut bar);
        let decorations = hud_overlays::ItemDecorations {
            count: stack.count,
            count_text,
            bar,
            cooldown: self.common.cooldowns.percent(&cooldown, partial),
        };
        hud_overlays::paint_item_decorations(batch, &self.client.gui.atlas, [x, y], &decorations);
    }
    /// `AbstractContainerScreen.extractSlotHighlightBack` and
    /// `AbstractContainerScreen.extractSlotHighlightFront`: the 24 x 24
    /// `container/slot_highlight_back` sprite behind the slot at `x`, `y` (its
    /// 16 x 16 item box), the `_front` one over its item.
    pub(in crate::game) fn draw_slot_highlight(
        &self,
        batch: &mut HudBatch,
        front: bool,
        x: f32,
        y: f32,
    ) {
        let sprite = if front {
            "container/slot_highlight_front"
        } else {
            "container/slot_highlight_back"
        };
        sunrise_client::ui::nine_slice(
            batch,
            &self.client.gui.atlas,
            sprite,
            x - 4.0,
            y - 4.0,
            24.0,
            24.0,
            u32::MAX,
        );
    }
    /// `extractSlot` of slot `slot` holding `stack` at (x, y): during a quick
    /// craft drag a dragged slot shows what it would hold over a white
    /// highlight (its count yellow when the slot caps it), and hides while
    /// it is the drag's only slot.
    pub(in crate::game) fn draw_slot_item(
        &self,
        batch: &mut HudBatch,
        drag: Option<&DragView>,
        slot: usize,
        stack: &Option<ItemStack>,
        x: f32,
        y: f32,
    ) {
        let carried;
        let (stack, capped) = match drag.and_then(|d| d.slots.get(&slot)) {
            Some(None) => return,
            Some(Some((count, capped))) => {
                batch.fill(
                    &self.client.gui.atlas,
                    x,
                    y,
                    16.0,
                    16.0,
                    rgba(255, 255, 255, 128),
                );
                carried = self
                    .common
                    .carried
                    .clone()
                    .map(|c| ItemStack { count: *count, ..c });
                (carried.as_ref(), *capped)
            }
            None => (stack.as_ref(), false),
        };
        if let Some(stack) = stack {
            self.draw_item_model(batch, stack, x, y, 0);
            let mut count = [0; 13];
            let text = if capped {
                Some(hud_overlays::item_count_text(stack.count, true, &mut count))
            } else {
                None
            };
            self.draw_item_decorations(batch, stack, x, y, text);
        } else if let Some(icon) =
            container_slot_index(slot).and_then(|slot| self.native_screen_slot_icon(slot))
            && let Some(rect) = self.client.gui.atlas.gui_sprite_or_missing(icon)
        {
            batch.rect_uv(x, y, 16.0, 16.0, rect, 0xFFFF_FFFF);
        }
    }
    /// The slot a pointer over every one of `hits` (in `slots` order) takes:
    /// the first. A mod screen's slots overlap by menu index, the container
    /// first, then the main inventory, then the hotbar, as the menu lists
    /// them, so the lowest index wins.
    pub(in crate::game) fn first_hovered(
        &self,
        mut hits: impl Iterator<Item = usize>,
    ) -> Option<usize> {
        if !self.native_screen_shown() {
            return hits.next();
        }
        hits.min_by_key(|&slot| {
            if let Some(idx) = container_slot_index(slot) {
                idx
            } else if (9..36).contains(&slot) {
                MOD_MENU_SLOT_BASE + slot
            } else {
                MOD_MENU_SLOT_BASE + 1000 + slot
            }
        })
    }
    /// The open container screen's player-inventory origin (`getXOrigin`/
    /// `getYOrigin`): the build's, or the frame's own arithmetic with no
    /// build yet.
    pub fn inventory_origin(&self, batch: &HudBatch) -> (f32, f32) {
        match self.built_container(batch) {
            Some(layout) => layout.origin,
            None => self.inventory_origin_unbuilt(batch),
        }
    }
    /// Recompute the origin when no build holds this size, including the book shift and signed mod offset.
    pub(in crate::game) fn inventory_origin_unbuilt(&self, batch: &HudBatch) -> (f32, f32) {
        if self.client.screen == Screen::Creative {
            return Self::creative_origin(batch);
        }
        let width = self.native_screen_width().unwrap_or(176.0);
        let (x, y) = batch.centered_origin(width, self.screen_height());
        let x = x + self.recipe_book_shift(batch.width) + self.screen_room_offset(batch);
        (x, y)
    }
    /// Whether a quick craft drag takes `slot`
    /// (`shouldAddSlotToQuickCraft`); screens the menu engine doesn't
    /// model take every slot.
    pub(in crate::game) fn quick_craft_accepts(
        &mut self,
        slot: usize,
        ty: i32,
        dragged: usize,
    ) -> bool {
        let mut accepts = true;
        self.menu_session(|menu, env, to_menu| {
            accepts = to_menu(slot).is_some_and(|s| menu.quick_craft_accepts(env, s, ty, dragged));
            Some(false)
        });
        accepts
    }
    /// The slot under the cursor (`AbstractContainerScreen.getHoveredSlot`),
    /// from the build's slot boxes rather than a fresh grid, so the hit test
    /// is the one the frame draws with. None while the recipe book covers the
    /// menu in a narrow window (`AbstractRecipeBookScreen.isHovering`).
    pub fn slot_at(&self, batch: &HudBatch, x: f32, y: f32) -> Option<usize> {
        if self.book_covers_menu(batch.width) {
            return None;
        }
        match self.built_container(batch) {
            Some(layout) => self.first_hovered(
                layout
                    .slots
                    .iter()
                    .filter(|(_, r)| {
                        // AbstractContainerScreen.isHovering: the 16x16 slot
                        // grows by one pixel on every side, so the gaps
                        // between slots still hover one.
                        x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
                    })
                    .map(|&(slot, _)| slot),
            ),
            None => self.slot_at_unbuilt(batch, x, y),
        }
    }
    /// `slot_at` from the game's state alone, without a build.
    pub(in crate::game) fn slot_at_unbuilt(
        &self,
        batch: &HudBatch,
        x: f32,
        y: f32,
    ) -> Option<usize> {
        let (ox, oy) = self.inventory_origin_unbuilt(batch);
        let hits = self.slots().into_iter().filter(|&i| {
            let (sx, sy) = self.slot_position(i);
            // AbstractContainerScreen.isHovering: the 16x16 slot grows by one
            // pixel on every side, so the gaps between slots still hover one.
            x >= ox + sx - 1.0 && x < ox + sx + 17.0 && y >= oy + sy - 1.0 && y < oy + sy + 17.0
        });
        self.first_hovered(hits)
    }
    /// How many slots `slots` lists, counted without building the list: the
    /// screen render hook asks once a frame and must not allocate. Same
    /// branches in the same order, so the two stay in step.
    pub fn slot_count(&self) -> usize {
        if self.creative_inventory_open() {
            return Creative::inventory_slots().count();
        }
        if self.creative_tab_open() {
            return Creative::tab_slots().count();
        }
        if let Some(n) = self
            .station_slot_count()
            .or_else(|| self.mount_slot_count())
        {
            return n;
        }
        let inventory = 36;
        if let Some(screen) = self.shown_native_screen() {
            return screen.visible_slots().count();
        }
        let container = match self.common.container_kind() {
            Some(BlockEntity::Chest(_)) => self.common.open_container_size(),
            Some(BlockEntity::Furnace(_)) => 3,
            Some(BlockEntity::Brewing(_)) => 5,
            Some(BlockEntity::Crafter(_)) => 10,
            Some(
                BlockEntity::Sign(_)
                | BlockEntity::Jukebox(_)
                | BlockEntity::Beacon(_)
                | BlockEntity::Lectern(_)
                | BlockEntity::Sculk(_)
                | BlockEntity::Banner(_)
                | BlockEntity::CopperGolemStatue(_)
                | BlockEntity::CreakingHeart(_)
                | BlockEntity::Spawner(_)
                | BlockEntity::TrialSpawner(_)
                | BlockEntity::Vault(_)
                | BlockEntity::Beehive(_)
                | BlockEntity::CommandBlock(_)
                | BlockEntity::EndGateway(_)
                | BlockEntity::Skull(_)
                | BlockEntity::DaylightDetector
                | BlockEntity::Bell
                | BlockEntity::Conduit(_)
                | BlockEntity::EndPortal
                | BlockEntity::EnchantingTable
                | BlockEntity::Campfire(_)
                | BlockEntity::ChiseledBookshelf(_)
                | BlockEntity::Brushable(_)
                | BlockEntity::Shelf(_)
                | BlockEntity::Mod(_)
                | BlockEntity::PotentSulfur(_)
                | BlockEntity::StructureBlock(_)
                | BlockEntity::JigsawBlock(_)
                | BlockEntity::TestBlock(_)
                | BlockEntity::TestInstanceBlock(_),
            ) => 0,
            None => {
                self.common.craft_size * self.common.craft_size
                    + 1
                    + if self.common.craft_size == 2 { 5 } else { 0 }
            }
        };
        inventory + container
    }
    pub fn slot_mut(&mut self, i: usize) -> &mut Option<ItemStack> {
        if i >= MOD_MENU_SLOT_BASE && !self.is_mount_chest_slot(i) {
            return self.container_slot_mut(i - MOD_MENU_SLOT_BASE);
        }
        if i >= sunrise_gameplay_common::mount_rules::SADDLE {
            return self.mount_slot_mut(i);
        }
        if i >= stations::INPUT {
            return self.station_slot_mut(i);
        }
        if (500..504).contains(&i) {
            return &mut self.common.armor[i - 500];
        }
        if i == 504 {
            return &mut self.common.offhand;
        }
        if i >= 300 {
            return self.container_slot_mut(i - 300);
        }
        if (100..109).contains(&i) {
            &mut self.common.craft_grid[i - 100]
        } else {
            &mut self.common.inventory[i]
        }
    }
    /// InventoryMenu / CraftingMenu slot layouts.
    pub fn slot_position(&self, i: usize) -> (f32, f32) {
        #[cfg(test)]
        SLOT_POSITIONS.with(|count| count.set(count.get() + 1));
        if self.creative_inventory_open()
            && let Some(position) = Creative::inventory_slot_position(i)
        {
            return position;
        }
        if self.creative_tab_open()
            && let Some(position) = Creative::tab_slot_position(i)
        {
            return position;
        }
        if let Some(p) = self
            .station_slot_position(i)
            .or_else(|| {
                sunrise_gameplay_common::game::CommonGame::stock_machine_slot(
                    self.machine_slot_position(i),
                    self.native_screen_shown(),
                )
            })
            .or_else(|| self.crafter_slot_position(i))
            .or_else(|| self.mount_slot_position(i))
        {
            return p;
        }
        let table = self.common.craft_size == 3;
        // ContainerScreen (3 rows) shifts the player inventory down by one pixel.
        // ChestMenu: the inventory sits `18 * (rows - 3) + 1` lower than the
        // 3-row layout of the other menus.
        let shift = match (self.chest_rows(), self.native_screen_inventory_y()) {
            (_, Some(y)) => y - 84.0,
            (Some(rows), None) => 1.0 + (rows as f32 - 3.0) * 18.0,
            (None, None) => 0.0,
        };
        let inventory_x = self.native_screen_inventory_x().unwrap_or(8.0);
        match i {
            // A mod screen's committed move of a player slot, else the grid below.
            0..36 if let Some(p) = self.native_screen_player_slot(i) => p,
            0..9 => (inventory_x + i as f32 * 18.0, 142.0 + shift),
            9..36 => (
                inventory_x + ((i - 9) % 9) as f32 * 18.0,
                84.0 + shift + ((i - 9) / 9) as f32 * 18.0,
            ),
            500..504 => (8.0, 8.0 + (i - 500) as f32 * 18.0),
            // InventoryMenu: the shield slot.
            504 => (77.0, 62.0),
            i if let Some(idx) = container_slot_index(i)
                && let Some(p) = self.native_screen_slot(idx) =>
            {
                p
            }
            i if let Some(idx) = container_slot_index(i) => match self.common.container_kind() {
                Some(BlockEntity::Furnace(_)) => {
                    [(56.0, 17.0), (56.0, 53.0), (116.0, 35.0)][idx.min(2)]
                }
                _ => (
                    8.0 + (idx % 9) as f32 * 18.0,
                    18.0 + (idx / 9) as f32 * 18.0,
                ),
            },
            200 => {
                if table {
                    (124.0, 35.0)
                } else {
                    (154.0, 28.0)
                }
            }
            _ => {
                let g = i - 100;
                let (r, c) = (g / self.common.craft_size, g % self.common.craft_size);
                if table {
                    (30.0 + c as f32 * 18.0, 17.0 + r as f32 * 18.0)
                } else {
                    (98.0 + c as f32 * 18.0, 18.0 + r as f32 * 18.0)
                }
            }
        }
    }
    /// What a slot shows: the crafting result slots are computed.
    pub fn slot_view(&self, i: usize) -> Option<ItemStack> {
        match i {
            200 => self.craft_result(),
            309 if matches!(self.container_kind(), Some(BlockEntity::Crafter(_))) => {
                self.crafter_result()
            }
            i if i >= MOD_MENU_SLOT_BASE && !self.is_mount_chest_slot(i) => {
                self.container_slot(i - MOD_MENU_SLOT_BASE).clone()
            }
            i if i >= sunrise_gameplay_common::mount_rules::SADDLE => self.mount_slot(i).clone(),
            i if i >= stations::INPUT => self.station_slot(i).clone(),
            500..=503 => self.common.armor[i - 500].clone(),
            504 => self.common.offhand.clone(),
            i if i >= 300 => self.container_slot(i - 300).clone(),
            100..=108 => self.common.craft_grid[i - 100].clone(),
            _ => self.common.inventory[i].clone(),
        }
    }
    /// 300.. are the open container's slots.
    pub fn slots(&self) -> Vec<usize> {
        if self.creative_inventory_open() {
            return Creative::inventory_slots().collect();
        }
        if self.creative_tab_open() {
            return Creative::tab_slots().collect();
        }
        if let Some(v) = self.station_slots().or_else(|| self.mount_slots()) {
            return v;
        }
        if let Some(screen) = self.shown_native_screen() {
            return screen.visible_slots().collect();
        }
        let mut v: Vec<usize> = (0..36).collect();
        match self.common.container_kind() {
            Some(BlockEntity::Chest(_)) => {
                v.extend((0..self.common.open_container_size()).map(|i| 300 + i))
            }
            Some(BlockEntity::Furnace(_)) => v.extend(300..303),
            Some(BlockEntity::Brewing(_)) => v.extend(300..305),
            Some(BlockEntity::Crafter(_)) => v.extend(300..310),
            Some(
                BlockEntity::Sign(_)
                | BlockEntity::Jukebox(_)
                | BlockEntity::Beacon(_)
                | BlockEntity::Lectern(_)
                | BlockEntity::Sculk(_)
                | BlockEntity::Banner(_)
                | BlockEntity::CopperGolemStatue(_)
                | BlockEntity::CreakingHeart(_)
                | BlockEntity::Spawner(_)
                | BlockEntity::TrialSpawner(_)
                | BlockEntity::Vault(_)
                | BlockEntity::Beehive(_)
                | BlockEntity::CommandBlock(_)
                | BlockEntity::EndGateway(_)
                | BlockEntity::Skull(_)
                | BlockEntity::DaylightDetector
                | BlockEntity::Bell
                | BlockEntity::Conduit(_)
                | BlockEntity::EndPortal
                | BlockEntity::EnchantingTable
                | BlockEntity::Campfire(_)
                | BlockEntity::ChiseledBookshelf(_)
                | BlockEntity::Brushable(_)
                | BlockEntity::Shelf(_)
                | BlockEntity::Mod(_)
                | BlockEntity::PotentSulfur(_)
                | BlockEntity::StructureBlock(_)
                | BlockEntity::JigsawBlock(_)
                | BlockEntity::TestBlock(_)
                | BlockEntity::TestInstanceBlock(_),
            ) => {}
            None => {
                v.extend((0..self.common.craft_size * self.common.craft_size).map(|i| 100 + i));
                v.push(200);
                if self.common.craft_size == 2 {
                    v.extend(500..505);
                }
            }
        }
        v
    }
}
