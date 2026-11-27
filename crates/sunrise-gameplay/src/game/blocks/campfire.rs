//! Campfires' cooking: `CampfireBlock.useItemOn` (`CampfireBlockEntity.placeFood`)
//! and the block entity's `cookTick` (lit) and `cooldownTick` (unlit). The
//! items drawn on the fire are `campfire_geometry`'s; a server's clients get
//! them through the block entity's update tag (`lan::chunks::entity_tag`).
use crate::game::ServerGame;
use sunrise_core::position::BlockPos;
use sunrise_game_data::Result;
use sunrise_items::items::ItemStack;
use sunrise_level::{block_entities::Pos, containers::BlockEntity};
use sunrise_sim::mobs::Who;

/// The `campfire_cooking` recipe kind (`RecipeType.CAMPFIRE_COOKING`).
pub(in crate::game) const KIND: &str = "minecraft:campfire_cooking";

pub(in crate::game) fn is_campfire(name: &str) -> bool {
    matches!(
        sunrise_game_data::block_entity_types::factory_block(name),
        "minecraft:campfire" | "minecraft:soul_campfire"
    )
}

impl ServerGame<'_> {
    /// `CampfireBlock.useItemOn` with a held item that is a campfire recipe's
    /// input (`RecipePropertySet.CAMPFIRE_INPUT`): on a server level the food
    /// goes into the first free slot (`placeFood`), and the use is consumed
    /// whether or not there was room. A client of a server leaves the placing
    /// to the server. Whether the food went in; None when this isn't a
    /// campfire use, so the item's own use runs.
    pub(in crate::game::server_side) fn use_on_campfire(
        &mut self,
        pos: [i32; 3],
    ) -> Result<Option<bool>> {
        // `ServerPlayerGameMode.useItemOn`'s `suppressUsingBlock`: sneaking
        // with a stack in either hand never reaches the block's `useItemOn`,
        // so the food stays in the hand and the item's own use (eating)
        // runs instead.
        if self.server_acting_sneak() {
            return Ok(None);
        }
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        if !is_campfire(&self.common.res.facts().get(state).name) {
            return Ok(None);
        }
        let Some(held) = self.common.held().cloned() else {
            return Ok(None);
        };
        let Some(time) = self
            .recipes()
            .cook(&self.common.loop_context, KIND, &held)
            .map(|r| r.time as i32)
        else {
            return Ok(None);
        };
        // A client of a server predicts the placing; the server decides.
        if !self.common.is_logical_server() {
            return Ok(Some(true));
        }
        let key = (pos[0], pos[1], pos[2]);
        let placed = match self.common.world.block_entities.get_mut(&key) {
            Some(BlockEntity::Campfire(c)) => {
                let mut one = held.clone();
                c.place_food(&mut one, time)
            }
            _ => false,
        };
        if placed {
            self.native_campfire_start(pos, &held.item);
            // ItemStack.consumeAndReturn: creative keeps the item.
            self.consume_held();
            self.award_custom_stat("interact_with_campfire", 1);
            self.post_block_game_event(
                "minecraft:block_change",
                pos,
                Some(Who::Player),
                Some(state),
            );
            self.mark_block_entity_updated(key);
        }
        // `ServerPlayerGameMode.useItemOn` triggers ITEM_USED_ON_BLOCK with
        // the stack before the use once `useItemOn` consumed the click, both
        // when the food went in (SUCCESS_SERVER) and on a full campfire
        // (CONSUME).
        self.trigger_block("minecraft:item_used_on_block", pos, &held);
        Ok(Some(placed))
    }

    /// `CampfireBlockEntity.cookTick` of a lit campfire, `cooldownTick` of an
    /// unlit one. A finished item's result (the recipe's, or the item itself
    /// when no recipe takes it any more) drops at the block's corner
    /// (`Containers.dropItemStack` at its integer position), then the clients
    /// see the slot empty and a `block_change` goes out.
    pub(in crate::game) fn tick_campfire(&mut self, pos: Pos) -> Result<()> {
        let state = self.common.world.block(pos.0, pos.1, pos.2);
        let lit = self
            .common
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("lit"))
            .is_some_and(|v| v == "true");
        let Some(BlockEntity::Campfire(c)) = self.common.world.block_entities.get_mut(&pos) else {
            return Ok(());
        };
        // BlockEntity.setChanged: a timer that moved is saved with the
        // chunk (no update to the clients, who don't see the timers).
        let changed = if lit {
            c.items.iter().any(Option::is_some)
        } else {
            c.cooldown_tick()
        };
        if changed {
            self.common.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
        }
        if !lit {
            self.sleep_idle_campfire(pos, false);
            return Ok(());
        }
        let Some(BlockEntity::Campfire(c)) = self.common.world.block_entities.get_mut(&pos) else {
            return Ok(());
        };
        let done = c.cook_tick();
        for (_, item) in done {
            // The world's table, as placing the item (`self.recipes()`) asked.
            let result = world_recipes!(self)
                .cook(&self.common.loop_context, KIND, &item)
                .map_or_else(|| item.clone(), |r| r.result.clone());
            self.native_block_cook([pos.0, pos.1, pos.2], &item.item, &result.item);
            self.campfire_drop(pos, result);
            self.mark_block_entity_updated(pos);
            self.post_block_game_event(
                "minecraft:block_change",
                [pos.0, pos.1, pos.2],
                None,
                Some(state),
            );
        }
        self.sleep_idle_campfire(pos, true);
        Ok(())
    }

    pub(in crate::game::server_side) fn sleep_idle_campfire(&mut self, pos: Pos, lit: bool) {
        if matches!(self.world.block_entities.get(&pos), Some(BlockEntity::Campfire(c)) if c.idle(lit))
        {
            self.common.world.block_entities.sleep(pos);
        }
    }

    /// `Containers.dropItemStack(level, x, y, z, stack)` at the block's
    /// integer position.
    pub(in crate::game::server_side) fn campfire_drop(&mut self, pos: Pos, stack: ItemStack) {
        let at = BlockPos::new(pos.0, pos.1, pos.2);
        for piece in sunrise_world::level::removal::drop_item_stack(
            &mut self.common.world.random,
            at,
            stack.count,
        ) {
            self.spawn_item(
                ItemStack {
                    count: piece.count,
                    ..stack.clone()
                },
                piece.pos,
                piece.motion,
                0,
            );
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
