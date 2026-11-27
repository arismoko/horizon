use crate::game::ClientGame;
use sunrise_items::items::ItemStack;

#[path = "crafter_client.rs"]
pub(in crate::game) mod crafter;
#[path = "screen.rs"]
mod screen;

/// Which machine screen the open container uses.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::game) enum Screen {
    Hopper,
    Dispenser,
    Dropper,
}

impl ClientGame<'_> {
    pub(in crate::game) fn machine_layout_selector(&self) -> Option<bool> {
        self.machine_screen().map(|s| s == Screen::Hopper)
    }
    pub(in crate::game::client_side) fn machine_screen(&self) -> Option<Screen> {
        let key = self.common.container?;
        // A mod's screen over a hopper or dispenser menu lays out its own slots.
        if self.native_screen_inventory_y().is_some() {
            return None;
        }
        // A server's menu shows by its type, whatever block (or minecart) holds it.
        if let Some(menu) = self.remote_menu_type() {
            return match menu {
                "minecraft:hopper" => Some(Screen::Hopper),
                "minecraft:generic_3x3" => Some(Screen::Dispenser),
                _ => None,
            };
        }
        if key.1 == super::minecarts::CART_KEY_Y {
            return (self.common.open_cart_kind() == Some(super::minecarts::CartKind::Hopper))
                .then_some(Screen::Hopper);
        }
        let state = self.common.world.block(key.0, key.1, key.2);
        match &*self.common.res.facts().get(state).name {
            "minecraft:hopper" => Some(Screen::Hopper),
            "minecraft:dispenser" => Some(Screen::Dispenser),
            "minecraft:dropper" => Some(Screen::Dropper),
            _ => None,
        }
    }
    /// `imageHeight` of the open screen (HopperScreen is 133 tall).
    pub(in crate::game) fn screen_height(&self) -> f32 {
        if let Some(height) = self.native_screen_height() {
            return height;
        }
        if self
            .common
            .station
            .as_ref()
            .is_some_and(|s| s.kind == sunrise_gameplay_common::game::stations_common::Kind::Beacon)
        {
            return super::beacon::HEIGHT;
        }
        if let Some(rows) = self.chest_rows() {
            // ContainerScreen: 114 + 18 per row.
            return 114.0 + rows as f32 * 18.0;
        }
        if self.container_block_is_shulker() {
            return 167.0;
        }
        if self.machine_screen() == Some(Screen::Hopper) {
            133.0
        } else {
            166.0
        }
    }
    /// HopperMenu / DispenserMenu slot layouts; None for other screens.
    pub(in crate::game) fn machine_slot_position(&self, i: usize) -> Option<(f32, f32)> {
        let screen = self.machine_screen()?;
        let inventory_y = if screen == Screen::Hopper { 51.0 } else { 84.0 };
        Some(match i {
            0..9 => (8.0 + i as f32 * 18.0, inventory_y + 58.0),
            9..36 => (
                8.0 + ((i - 9) % 9) as f32 * 18.0,
                inventory_y + ((i - 9) / 9) as f32 * 18.0,
            ),
            300..500 if screen == Screen::Hopper => (44.0 + (i - 300) as f32 * 18.0, 20.0),
            300..500 => (
                62.0 + ((i - 300) % 3) as f32 * 18.0,
                17.0 + ((i - 300) / 3) as f32 * 18.0,
            ),
            _ => return None,
        })
    }
}
