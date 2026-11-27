//! A bucket's aim and target: `getPlayerPOVHitResult(..., SOURCE_ONLY)` for an empty bucket (the block outline or a source fluid, whichever is nearer) and `BucketItem.use`'s place position.
use super::*;

#[cfg(test)]
#[path = "tests/bucket_hook_moment_tests.rs"]
mod bucket_hook_moment_tests;

impl crate::game::ServerGame<'_> {
    /// The tail of `BucketItem.use` once `emptyContents` succeeded, after the
    /// criteria: `player.awardStat(Stats.ITEM_USED.get(this))`, then
    /// `ItemUtils.createFilledResult(itemStack, player,
    /// getEmptySuccessItem(itemStack, player))` — a survival bucket becomes an
    /// empty one, a creative one stays.
    pub(in crate::game::server_side) fn bucket_emptied(&mut self, item: &str) {
        self.award_stat("used", item, 1);
        if !self.common.creative {
            *self.common.held_mut() = Some(ItemStack::new("minecraft:bucket", 1));
        }
    }

    pub(in crate::game) fn server_bucket_liquid_user_entity_id(&self) -> Option<u32> {
        self.server
            .lan_acting_entity
            .or_else(|| self.common.player.as_ref().map(|p| p.entity_id as u32))
    }

    pub(in crate::game) fn server_item_pick(&self) -> Option<Hit> {
        self.item_pick()
    }

    /// `BucketItem.use`'s guard before it touches the world: `level.mayInteract`,
    /// which on a dedicated server refuses a non-operator inside the spawn
    /// protection radius (at the block the bucket's own ray met), then
    /// `player.mayUseItemAt` (adventure mode's `can_place_on`).
    pub(in crate::game::server_side) fn bucket_use_refused(&self, pos: [i32; 3]) -> bool {
        self.server
            .lan_acting_spawn_protection
            .is_some_and(|radius| {
                self.common
                    .spawn_protects(radius, self.common.dim.id(), pos)
            })
            || (!self.may_build() && !self.can_place_on_in_adventure(pos))
    }
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;
