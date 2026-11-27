//! Client-side container menus: chest/shulker titles, slots, rendering, and quick-move.
use crate::game::ClientGame;
use sunrise_game_data::text::Text;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::{BlockEntity, is_chest_block};

#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay_common::game::storage_common::{
    chest_connected_pos, is_shulker_box,
};

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// The custom name of the open container, or its default title
    /// (a double chest is named by whichever half has a name).
    pub(in crate::game) fn container_title(&self, default: String) -> String {
        self.common
            .container_parts()
            .iter()
            .find_map(|(key, _)| self.common.block_name_plain(*key))
            .unwrap_or(default)
    }
    /// ShulkerBoxSlot.mayPlace: no shulker box inside a shulker box.
    pub(in crate::game::client_side) fn refuses(&self, slot: usize, stack: &ItemStack) -> bool {
        (300..400).contains(&slot)
            && is_shulker_box(&stack.item)
            && self.common.container.is_some_and(|k| {
                is_shulker_box(
                    &self
                        .common
                        .res
                        .facts()
                        .get(self.common.world.block(k.0, k.1, k.2))
                        .name,
                )
            })
    }
}

// ---- chest menus: titles, rendering and shift-click ----

#[cfg(feature = "client")]
impl ClientGame<'_> {
    pub(in crate::game::client_side) fn container_slot(&self, mut n: usize) -> &Option<ItemStack> {
        for (pos, size) in self.common.container_parts() {
            if n < size {
                return self
                    .common
                    .world
                    .block_entities
                    .get(&pos)
                    .and_then(|e| e.slots().get(n))
                    .unwrap_or(&self.common.scratch_slot);
            }
            n -= size;
        }
        &self.common.scratch_slot
    }
    /// Container slot `n` (menu slot `300 + n`).
    pub(in crate::game::client_side) fn container_slot_mut(
        &mut self,
        mut n: usize,
    ) -> &mut Option<ItemStack> {
        let mut target = None;
        for (pos, size) in self.common.container_parts() {
            if n < size {
                target = Some((pos, n));
                break;
            }
            n -= size;
        }
        match target.and_then(|(pos, i)| {
            self.common
                .world
                .block_entities
                .get_mut(&pos)
                .and_then(|e| e.slot_mut(i))
        }) {
            Some(slot) => slot,
            None => &mut self.common.scratch_slot,
        }
    }
    pub(in crate::game) fn container_block_is_shulker(&self) -> bool {
        self.container_block()
            .as_deref()
            .is_some_and(is_shulker_box)
    }
    pub(in crate::game::client_side) fn container_block(&self) -> Option<std::sync::Arc<str>> {
        let k = self.common.container?;
        Some(
            self.common
                .res
                .facts()
                .get(self.common.world.block(k.0, k.1, k.2))
                .name
                .clone(),
        )
    }
    /// Rows of the open generic 9-wide container (`ChestMenu`), or None for
    /// the other menus (hoppers, dispensers, shulker boxes, furnaces...).
    pub(in crate::game) fn chest_rows(&self) -> Option<usize> {
        if let Some((rows, _)) = self.remote_chest_menu() {
            return Some(rows);
        }
        if !matches!(self.container_kind()?, BlockEntity::Chest(_)) {
            return None;
        }
        let generic = match self.container_block().as_deref() {
            Some("minecraft:barrel" | "minecraft:ender_chest") => true,
            Some(n) if is_chest_block(n) => true,
            _ => self.common.open_cart_kind() == Some(super::minecarts::CartKind::Chest),
        };
        generic.then(|| self.common.open_container_size() / 9)
    }
    /// The title of the open chest-like menu.
    pub(in crate::game::client_side) fn chest_title(&self, rows: usize) -> Text {
        if let Some((_, title)) = self.remote_chest_menu() {
            return title;
        }
        let (key, fallback) = match self.container_block().as_deref() {
            Some("minecraft:barrel") => ("container.barrel", "Barrel"),
            Some("minecraft:ender_chest") => ("container.enderchest", "Ender Chest"),
            _ if self.common.open_cart_kind().is_some() => {
                ("entity.minecraft.chest_minecart", "Minecart with Chest")
            }
            _ if rows > 3 => ("container.chestDouble", "Large Chest"),
            _ => ("container.chest", "Chest"),
        };
        Text::literal(self.container_title(self.translate_with_fallback(key, fallback)))
            .resolve_legacy()
    }
    /// `ContainerScreen` / `ShulkerBoxScreen`: the background with its
    /// container rows, the title and the inventory label.
    pub(in crate::game::client_side) fn draw_chest_screen(
        &self,
        batch: &mut sunrise_client::hud::HudBatch,
        ox: f32,
        oy: f32,
    ) {
        use sunrise_client::hud::rgba;
        let atlas = &self.client.gui.atlas;
        let grey = rgba(64, 64, 64, 255);
        if self
            .container_block()
            .as_deref()
            .is_some_and(is_shulker_box)
        {
            batch.sprite_region(
                atlas,
                "gui/container/shulker_box",
                [ox, oy, 176.0, 167.0],
                256.0,
                256.0,
            );
            self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
            // ShulkerBoxScreen draws the menu's title: the box's custom name
            // (BaseContainerBlockEntity.getDisplayName) or its default name.
            let title = self.remote_menu_title().unwrap_or_else(|| {
                Text::literal(self.container_title(
                    self.translate_with_fallback("container.shulkerBox", "Shulker Box"),
                ))
                .resolve_legacy()
            });
            super::screen_info::name_container(|| title.plain());
            batch.text_styled(atlas, &title, ox + 8.0, oy + 6.0, grey, false);
            let inventory = self.translate_with_fallback("container.inventory", "Inventory");
            batch.text(atlas, &inventory, ox + 8.0, oy + 73.0, grey, false);
            return;
        }
        let rows = self.chest_rows().unwrap_or(3);
        let top = rows as f32 * 18.0 + 17.0;
        batch.sprite_region(
            atlas,
            "gui/container/generic_54",
            [ox, oy, 176.0, top],
            256.0,
            256.0,
        );
        // The player half of the texture starts at v = 126.
        if let Some(mut r) = atlas.sprites.get("gui/container/generic_54").copied() {
            let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
            r.v0 += 126.0 * dv;
            r.v1 = r.v0 + 96.0 * dv;
            r.u1 = r.u0 + 176.0 * du;
            batch.rect_uv(ox, oy + top, 176.0, 96.0, r, u32::MAX);
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        let title = self.chest_title(rows);
        super::screen_info::name_container(|| title.plain());
        batch.text_styled(atlas, &title, ox + 8.0, oy + 6.0, grey, false);
        let inventory = self.translate_with_fallback("container.inventory", "Inventory");
        batch.text(
            atlas,
            &inventory,
            ox + 8.0,
            oy + 114.0 + rows as f32 * 18.0 - 94.0,
            grey,
            false,
        );
    }
}

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// `quickMoveStack` of the open chest-like, hopper, dispenser or furnace
    /// menu for a shift-click on `slot`, run by the verified menu engine
    /// (`sunrise_client::menu`); false when the menu has none.
    #[allow(dead_code)]
    pub(in crate::game) fn container_quick_move(&mut self, slot: usize) -> bool {
        self.click_open_inventory(Some(slot), 0, sunrise_sim::menu::Input::QuickMove)
    }
}
