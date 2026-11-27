//! The removal in a player's break (`ServerPlayerGameMode.destroyBlock`): the block's parts
//! (`on_destroyed_by_player`, `IBlockExtension.onDestroyedByPlayer`) may take it over, and with
//! none answering `playerWillDestroy` and `Level.removeBlock` are vanilla's.
use super::{item_abilities::ability, storage_common};
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::native_hooks::player_destroyed_block::GameHooks as _;
use sunrise_items::{blocks::StateFacts, items::ItemStack};
use sunrise_world::registry::BlockStateId;

/// What a player's removal of a block left for the break around it.
pub(in crate::game) struct PlayerRemoval {
    /// Whether the block was removed.
    pub removed: bool,
    /// The state the removal saw (vanilla's `adjustedState`: a cracked pot).
    pub state: BlockStateId,
    /// The one harvest answer, asked before the tool wore and the block went.
    pub harvest: bool,
    /// The main-hand stack as it was before the tool wore: what removal and loot see, whatever a
    /// listener does to the selected slot afterwards.
    pub tool: Option<ItemStack>,
    /// The stack the wear used up, if it broke.
    pub broken_tool: Option<ItemStack>,
}

impl crate::game::ServerGame<'_> {
    /// Remove the block `state` at `pos` for the player breaking it: `ServerPlayerGameMode.destroyBlock`
    /// from `playerWillDestroy` on (NeoForge's patch of it). `filled_hive` is a beehive with bees or
    /// honey.
    ///
    /// The order is NeoForge's: `playerWillDestroy` (pot cracking, piglin anger, a creaking heart's
    /// protector, tripwire disarming, a creative shulker box's or beehive's drop), then the main-hand
    /// stack is copied, the harvest answer is asked once (`hasCorrectToolForDrops`, with
    /// `can_harvest_block` and `player_harvest_check`) of the adjusted state, the tool wears
    /// (`ItemStack.mineBlock`), and the removal (`on_destroyed_by_player`, which may take it over, or
    /// `Level.removeBlock`) runs with that answer and that copy. Only a survival player's break asks
    /// and wears; a creative break harvests nothing.
    pub(in crate::game) fn player_destroy_block_removal(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        facts: &StateFacts,
        filled_hive: bool,
    ) -> PlayerRemoval {
        let creative = self.common.creative;
        let block_entity = self.removed_block_entity_data(pos, state);
        let state = self.player_will_destroy(pos, state, facts, filled_hive);
        let tool = self.common.held().cloned();
        let (harvest, broken_tool) = if creative {
            (false, None)
        } else {
            let harvest = self.acting_can_harvest(pos, state);
            (
                harvest,
                self.mine_block_with_held(&facts.name, facts.hardness),
            )
        };
        let mut destroyed = None;
        if sunrise_gameplay_common::game::api_values::block_state::block_listens(
            mod_api::HookId::OnDestroyedByPlayer,
            state,
        ) {
            let player = self.native_acting_id();
            self.native_on_destroyed_by_player(
                player,
                pos,
                state,
                tool.as_ref(),
                harvest,
                &mut destroyed,
            );
        }
        let removed = match destroyed {
            Some(removed) => {
                if removed {
                    self.invalidate_active_player_breaks(pos);
                    self.native_player_destroyed_block(pos, state, block_entity);
                }
                removed
            }
            None => self.remove_block_as_player(pos, creative, harvest),
        };
        PlayerRemoval {
            removed,
            state,
            harvest,
            tool,
            broken_tool,
        }
    }

    /// The block side effects of `Block.playerWillDestroy` that run before the vanilla
    /// behaviour library's removal (which adds the particles, the game event and the block
    /// entity's own effects); returns the adjusted state.
    fn player_will_destroy(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        facts: &StateFacts,
        filled_hive: bool,
    ) -> BlockStateId {
        let creative = self.common.creative;
        let state = self.crack_mined_pot(pos, state);
        // Block.playerWillDestroy: mining their gold angers all nearby piglins.
        if self.guarded_by_piglins(&facts.name) {
            self.anger_piglins(false);
        }
        self.creaking_heart_broken(pos, true);
        // TripWireBlock.playerWillDestroy: shears disarm the string first (flags
        // 260), so taking it down does not trip the hook.
        if &*facts.name == "minecraft:tripwire"
            && self.held_shears(ability::SHEARS_DISARM)
            && let Some(disarmed) = self.common.with_props(state, &[("disarmed", "true")])
        {
            let at = BlockPos::new(pos[0], pos[1], pos[2]);
            self.with_level(|b, level, ticks| b.set_block(level, ticks, at, disarmed, 260));
            self.post_block_game_event("minecraft:shear", pos, self.sculk_actor(), None);
        }
        if creative && storage_common::is_shulker_box(&facts.name) {
            self.drop_creative_shulker(pos);
        }
        // BeehiveBlock.playerWillDestroy: a creative break still drops the hive
        // with its bees and honey level.
        if creative
            && filled_hive
            && self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::BLOCK_DROPS)
            && let Some(stack) = self.picked_stack(pos, true)
        {
            self.spawn_item_own(stack, pos.map(f64::from), 10);
        }
        state
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::block_items::BlockItemsGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;
