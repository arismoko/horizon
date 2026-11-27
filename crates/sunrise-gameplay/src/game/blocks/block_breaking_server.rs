use super::*;
use sunrise_gameplay_common::game::item_abilities::ItemAbilitiesGameExt1 as _;

impl crate::game::ServerGame<'_> {
    pub fn break_block(&mut self, pos: [i32; 3]) -> Result<()> {
        self.carrier_break_block(pos)
    }
    /// `ServerPlayerGameMode.tick` while a player digs: level event 2020 every
    /// fourth tick since the START (the hit sound), else 2019 (the crack only),
    /// for everyone around, the digger included (`levelEvent(null, ...)`).
    pub(in crate::game) fn destroy_progress_event(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        ticks: u32,
    ) {
        let id = if ticks.is_multiple_of(4) { 2020 } else { 2019 };
        let direction = sunrise_core::geometry::Direction::ALL
            .iter()
            .position(|d| d.normal() == face)
            .unwrap_or(1);
        self.server_level_event_at(id, pos, direction as i32);
    }
    /// `BundleItem.dropContent`: `removeOneItemFromBundle` and `Player.drop`,
    /// then the drop sound and the used statistic.
    pub(in crate::game::server_side) fn drop_bundle_content(&mut self, slot: usize) {
        let items = self.items();
        let Some(mut bundle) = self.common.stack_in(slot).cloned() else {
            return;
        };
        let Some(mut contents) = sunrise_items::bundle::Contents::of(items, &bundle) else {
            return;
        };
        let Some(removed) = contents.remove_one(items) else {
            return;
        };
        contents.store(items, &mut bundle);
        let item = Arc::clone(&bundle.item);
        *self.common.stack_in_mut(slot) = Some(bundle);
        let Some(position) = self.common.player.as_ref().map(|p| p.position) else {
            return;
        };
        let at = [position.x, position.y, position.z];
        // playRemoveOneSound, then Player.drop and playDropContentsSound at
        // the player's block.
        let pitch = 0.8 + self.common.world.random.next_float() * 0.4;
        self.server_play_sound(sunrise_items::bundle::REMOVE_ONE, at, 0.8, pitch);
        self.player_drop(removed);
        let block = BlockPos::containing(position);
        let pitch = 0.8 + self.common.world.random.next_float() * 0.4;
        self.server_play_sound(
            sunrise_items::bundle::DROP_CONTENTS,
            [block.x, block.y, block.z].map(|v| f64::from(v) + 0.5),
            0.8,
            pitch,
        );
        self.award_stat("used", &item, 1);
    }
    /// `ItemStack.mineBlock` for the held stack once a survival player has
    /// mined the block `name`: an item with a `tool` component counts as used
    /// and loses its `damage_per_block` on a block that doesn't break
    /// instantly (`ShearsItem`: on any block but fire).
    pub fn mine_block_with_held(&mut self, name: &str, hardness: f32) -> Option<ItemStack> {
        let stack = self.common.inventory[self.common.selected].clone()?;
        let per_block = self.items().mining_damage(&stack)?;
        let max = self.items().mining_damage_capacity(&stack);
        let wears = if self.stack_shears(&stack, super::item_abilities::ability::SHEARS_DIG) {
            !self.items().block_in_tag("minecraft:fire", name)
        } else {
            hardness != 0.0
        };
        let broken = if wears && per_block > 0 && max > 0 {
            self.with_hand(Hand::Main, |game| {
                game.apply_held_damage(
                    per_block,
                    sunrise_core::equipment_slot::EquipmentSlot::MainHand,
                )
            })
        } else {
            None
        };
        self.award_stat("used", &stack.item, 1);
        broken
    }
    /// `ServerPlayerGameMode.destroyBlock`: whether the block was removed.
    /// A break inside another (a mod breaking more blocks from its
    /// `native_destroy_block`) keeps the outer break's own events.
    pub fn player_break_block(
        &mut self,
        pos: [i32; 3],
        collect: Option<&mut Vec<ItemStack>>,
    ) -> Result<bool> {
        self.with_player_actor(|game| game.break_block_as_player(pos, collect))
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
