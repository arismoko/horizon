use super::*;
use sunrise_gameplay_common::game::remote::inventory::creative_sync_slot;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Find a game slot in the menu's complete, contiguous slot mapping.
    pub fn menu_slot_index(menu: &str, internal: usize) -> Option<usize> {
        (0..)
            .map_while(|slot| sunrise_gameplay_common::game::inventory_screen_slot(menu, slot))
            .position(|slot| slot == internal)
    }
    pub(in crate::game::client_side) fn open_menu_name(&self, container: i32) -> String {
        match self.client.remote.as_ref().and_then(|r| r.menu.as_ref()) {
            Some((id, name)) if *id == container && container != 0 => name.clone(),
            _ => "inventory".into(),
        }
    }
    /// The server set inventory `slot` (`shield::OFFHAND_SLOT` for the off
    /// hand), `changed` from the stack this client held there. The stack
    /// that answers a swap (`setItemInHand`) is another object, which the
    /// hand shows after the swap animation (`hand_stack_replaced`); other
    /// updates of a hand's slot count as the held stack changing in place,
    /// as this client doesn't predict every use the way vanilla's does
    /// (a thrown snowball's stack shrinks only when the server says so).
    pub(in crate::game::client_side) fn remote_hand_slot_set(
        &mut self,
        slot: usize,
        changed: bool,
    ) {
        let hand = if slot == sunrise_gameplay_common::game::held_items::OFFHAND_SLOT {
            1
        } else if slot == self.common.selected {
            0
        } else {
            return;
        };
        let answer = self
            .client
            .remote
            .as_mut()
            .is_some_and(|r| std::mem::take(&mut r.swap_answer[hand]));
        if answer && changed {
            self.hand_stack_replaced(slot);
        }
    }
    /// The open menu's window id (0: the inventory).
    pub(in crate::game) fn remote_menu_id(&self) -> i32 {
        self.client
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map_or(0, |m| m.0)
    }
    pub(in crate::game::client_side) fn set_menu_slot(
        &mut self,
        container: i32,
        slot: i32,
        item: Option<ItemStack>,
    ) {
        let menu = self.open_menu_name(container);
        if menu == "minecraft:lectern" {
            if slot == 0 {
                self.remote_lectern_book(item);
            }
            return;
        }
        if container != 0 && menu == "inventory" {
            return;
        }
        let Some(internal) = sunrise_gameplay_common::game::inventory_screen_slot(&menu, slot)
        else {
            return;
        };
        if container == 0
            && let Some(r) = self
                .client
                .remote
                .as_mut()
                .filter(|r| r.menu.as_ref().is_some())
        {
            match internal {
                100..=103 => {
                    r.inventory_craft_grid[internal - 100] = item;
                    return;
                }
                200 => {
                    r.inventory_craft_result = item;
                    return;
                }
                _ => {}
            }
        }
        if internal == 200 {
            // ClientboundContainerSetSlotPacket for the open menu's slot 0: the
            // server's result of its own recipes (a world's data packs' too).
            if container == self.remote_menu().0
                && let Some(r) = self.client.remote.as_mut()
            {
                r.craft_result = item;
            }
            return;
        }
        let slot = self.slot_mut(internal);
        let changed = !crate::game::hand::matches_for_swap(slot.as_ref(), item.as_ref());
        *slot = item;
        if internal < 36 {
            self.remote_hand_slot_set(internal, changed);
        } else if internal == 504 {
            self.remote_hand_slot_set(
                sunrise_gameplay_common::game::held_items::OFFHAND_SLOT,
                changed,
            );
        }
        if internal >= sunrise_gameplay_common::game::stations_common::INPUT
            && self.common.station.is_some()
        {
            self.remote_station_slot_changed(internal);
        }
    }
    /// `MultiPlayerGameMode.handleCreativeModeItemAdd` for inventory slot `i` (0 to 35) or the
    /// off hand (36): the stack goes out now and counts as synced. Nothing without a server.
    pub fn remote_creative_slot_add(&mut self, i: usize) {
        let stack = if i == 36 {
            self.common.offhand.clone()
        } else {
            self.common.inventory.get(i).cloned().flatten()
        };
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        if let Some(synced) = r.synced.get_mut(i) {
            *synced = stack.clone();
        }
        self.send_creative_item_add(creative_sync_slot(i), stack);
    }

    /// A headless command oracle has a real connection but no movement tick.
    /// Acknowledge loaded terrain without predicting or sending movement.
    pub fn poll_stationary_client(&mut self) -> Result<()> {
        self.poll_remote()?;
        let loaded = self.common.player.as_ref().is_some_and(|p| {
            self.client.streaming.is_lit(
                (p.position.x.floor() as i32) >> 4,
                (p.position.z.floor() as i32) >> 4,
            )
        });
        if loaded && self.client.remote.as_ref().is_some_and(|r| !r.loaded_sent) {
            self.send(S::PlayerLoaded(Empty));
            let r = self.client.remote.as_mut().unwrap();
            r.loaded_sent = true;
            r.level_load.finish();
        }
        Ok(())
    }
}
