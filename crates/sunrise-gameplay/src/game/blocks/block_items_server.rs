//! Server-owned block entity item helpers and placement tests.
use crate::game::ServerGame;
use sunrise_core::nbt::Tag;
#[allow(unused_imports)]
use sunrise_items::{item_data, items::ItemStack};
#[allow(unused_imports)]
use sunrise_level::containers::BlockEntity;
#[allow(unused_imports)]
use sunrise_sim::{block_entity_item, save};
use sunrise_world::registry::BlockStateId;

impl ServerGame<'_> {
    /// Server arm of [`Game::side_encode_live_block_entity`].
    pub(in crate::game) fn server_encode_live_block_entity(
        &self,
        key: (i32, i32, i32),
        entity: &BlockEntity,
        block: &str,
    ) -> Tag {
        self.encode_live_block_entity(key, entity, block, self.common.res.items())
    }

    /// `ShulkerBoxBlock.playerWillDestroy`: a Creative player's filled box is
    /// one item at the block center, with collectComponents and the default pickup delay.
    /// This direct spawn intentionally does not use the block_drops gamerule.
    pub(in crate::game) fn drop_creative_shulker(&mut self, pos: [i32; 3]) {
        let key = (pos[0], pos[1], pos[2]);
        let filled = matches!(self.world.block_entities.get(&key), Some(BlockEntity::Chest(items))
            if items.iter().flatten().any(|s| s.count > 0));
        if !filled {
            return;
        }
        let Some(saved) = self.saved_block_entity(key, false) else {
            return;
        };
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let mut stack = ItemStack::new(&self.common.res.facts().get(state).item, 1);
        let patch = block_entity_item::collected_components(&saved);
        (stack.damage, stack.extra) = item_data::decode_components(Some(&Tag::Compound(patch)));
        self.spawn_item_own(stack, pos.map(|v| f64::from(v) + 0.5), 10);
    }

    /// Ensure the selected block's entity exists before applying the original stack's data.
    pub(in crate::game) fn apply_block_item_data(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        stack: &ItemStack,
        game_master: bool,
    ) {
        let key = (pos[0], pos[1], pos[2]);
        if !self.common.world.block_entities.stateful_entity(state.0)
            && let Some(entity) = self.new_block_entity_for_state(state)
        {
            self.common.world.block_entities.insert(key, entity);
        }
        if self.common.world.block_entities.get(&key).is_some() {
            self.apply_placed_item(pos, stack, game_master);
        }
    }

    /// `BlockState.getCloneItemStack` at `pos` as the server answers the acting player: the
    /// block's own stack, or a block Part's `clone_item_stack` answer in its place. With no
    /// player acting, no Part is asked.
    pub(in crate::game) fn acting_clone_stack(
        &self,
        pos: [i32; 3],
        include_data: bool,
    ) -> Option<ItemStack> {
        let stack = self.clone_stack(pos, include_data);
        let Some(player) = self.native_acting_player() else {
            return stack;
        };
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let mut replacement = None;
        self.native_clone_item_stack(
            self.common.dim.level(),
            sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
            state,
            include_data,
            player,
            &mut replacement,
        );
        replacement.or(stack)
    }

    /// `PlayerRef::clone_item_stack`: [`Self::acting_clone_stack`] with `player` acting, in
    /// the level in place. None on a client, for a stale or dead player, another level and an
    /// unloaded position.
    pub(in crate::game) fn player_clone_item_stack(
        &mut self,
        player: mod_api::PlayerRef,
        level: mod_api::LevelRef,
        pos: [i32; 3],
        include_data: bool,
    ) -> Option<ItemStack> {
        if !self.common.is_logical_server()
            || self.native_level_read_world(level).is_none()
            || level.dimension() != self.common.dim.key()
            || !self.common.world.is_loaded_at(pos[0], pos[1], pos[2])
        {
            return None;
        }
        let mut answer = None;
        self.native_as_player_handle(player, |game| {
            answer = game.acting_clone_stack(pos, include_data);
        });
        answer
    }

    /// `BlockItem.updateCustomBlockEntityTag` and `updateBlockEntityComponents`
    /// for the block entity just placed at `pos` from `stack`; `game_master`:
    /// the placer can use game master blocks (a dispenser can't).
    pub fn apply_placed_item(&mut self, pos: [i32; 3], stack: &ItemStack, game_master: bool) {
        let key = (pos[0], pos[1], pos[2]);
        self.common.pending.clear(pos);
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        self.sculk_place_entity(state);
        if let Some(custom_data_loaded) = self.apply_placed_item_common(pos, stack, game_master) {
            if custom_data_loaded {
                self.notify_block_entity_load_additional(key);
            }
            self.side_update_spawner_display(key);
        }
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::block_items::BlockItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::SharedGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::block_entity_load_additional::BlockEntityLoadAdditionalGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::block_entity_save_additional::BlockEntitySaveAdditionalGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
