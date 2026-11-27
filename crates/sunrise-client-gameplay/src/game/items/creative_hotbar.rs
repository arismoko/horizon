//! The creative toolbars, `hotbar.nbt` beside `options.txt`: hold the save or
//! load key and press a hotbar key (`Minecraft.handleKeybinds`).
use crate::game::ClientGame;
use sunrise_client::keybinds;
use sunrise_core::nbt::Tag;
use sunrise_game_data::text::Text;
use sunrise_items::items::ItemStack;
use sunrise_sim::hotbar;

impl ClientGame<'_> {
    /// Whether the live mapping is held, regardless of the device's current state.
    pub fn mapped_key_down(&self, mapping: &str) -> bool {
        keybinds::VanillaMappingSlot::by_name(mapping)
            .is_some_and(|slot| self.client.vanilla_mappings.is_down(slot))
    }

    /// `CreativeModeInventoryScreen.selectTab` on the HOTBAR tab: each saved
    /// row's nine stacks, or for an empty row a locked paper in that row's own
    /// slot, named after the keys that save it (`inventory.hotbarInfo`).
    pub(in crate::game::client_side) fn refresh_saved_hotbars(&mut self) {
        if !self.client.creative_screen.0.tab().hotbar {
            return;
        }
        let path = sunrise_gameplay_common::game::options_path().with_file_name("hotbar.nbt");
        let known = |id: &str| self.common.res.items().get(id).is_some();
        let rows = hotbar::load(&path, &known);
        let key =
            |mapping: &str| keybinds::display_name(self.client.settings.keys.get(mapping)).plain();
        let empty = || ItemStack::new("minecraft:air", 0);
        let mut items = Vec::with_capacity(hotbar::GROUPS * hotbar::SIZE);
        for (i, row) in rows.iter().enumerate().take(hotbar::GROUPS) {
            if row.iter().any(Option::is_some) {
                items.extend(row.iter().map(|s| s.clone().unwrap_or_else(empty)));
                continue;
            }
            for j in 0..hotbar::SIZE {
                if j != i {
                    items.push(empty());
                    continue;
                }
                let with = [
                    key("key.saveToolbarActivator"),
                    key(&format!("key.hotbar.{}", i + 1)),
                ];
                let mut paper = ItemStack::new("minecraft:paper", 1);
                let data = paper.data_mut();
                data.set_other(
                    "minecraft:creative_slot_lock",
                    Some(Tag::compound(Vec::<(&str, Tag)>::new())),
                );
                data.set_other(
                    "minecraft:item_name",
                    Some(Tag::compound([
                        ("translate", Tag::String("inventory.hotbarInfo".into())),
                        (
                            "with",
                            Tag::List(
                                with.iter()
                                    .map(|s| Tag::String(s.as_str().into()))
                                    .collect(),
                            ),
                        ),
                    ])),
                );
                items.push(paper);
            }
        }
        self.client.creative_screen.0.set_saved_hotbars(items);
    }
    /// `CreativeModeInventoryScreen.handleHotbarLoadOrSave` for the hotbar key
    /// `index`: load puts the saved row in the hotbar and, on a server, sends
    /// each of its nine slots at once (`handleCreativeModeItemAdd`, changed or
    /// not), so a drop later in the tick drops what the server now holds; save
    /// stores the hotbar and says so on the action bar.
    pub(in crate::game::client_side) fn hotbar_load_or_save(&mut self, index: usize, load: bool) {
        let path = sunrise_gameplay_common::game::options_path().with_file_name("hotbar.nbt");
        let known = |id: &str| self.items().get(id).is_some();
        let mut rows = hotbar::load(&path, &known);
        if load {
            for (slot, stack) in rows[index].iter().enumerate() {
                self.common.inventory[slot] = stack.clone();
                self.remote_creative_slot_add(slot);
            }
            return;
        }
        rows[index] = self.common.inventory[..hotbar::SIZE].to_vec();
        if let Err(e) = hotbar::save(&path, &rows, self.items()) {
            // saveHotbar sets the message inside its try, so a failed save says nothing.
            log::warn!("the creative hotbar was not saved: {e}");
            return;
        }
        let key = |mapping: &str| keybinds::display_name(self.client.settings.keys.get(mapping));
        let message = Text::translate(
            "inventory.hotbarSaved",
            &[
                key("key.loadToolbarActivator"),
                key(&format!("key.hotbar.{}", index + 1)),
            ],
        );
        // handleHotbarLoadOrSave: the overlay, said at once.
        self.say_system_now(&message);
        self.client.titles.set_overlay(message, false);
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;
