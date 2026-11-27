use crate::game::ClientGame;
use sunrise_core::nbt::Tag;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::{block_entity_item, save};

impl ClientGame<'_> {
    /// Client arm of [`Game::side_encode_live_block_entity`].
    pub(in crate::game) fn client_encode_live_block_entity(
        &self,
        key: (i32, i32, i32),
        entity: &BlockEntity,
        block: &str,
    ) -> Tag {
        save::block_entity_encode::encode(key, entity, block, self.common.res.items())
    }

    /// `BlockItem.updateBlockEntityComponents` on a client's level, for the
    /// block it predicts at `pos`: the block entity takes the stack's
    /// components but not its `block_entity_data` (`updateCustomBlockEntityTag`
    /// returns on a client), so a copied sign's editor opens with its text (#8102).
    pub(in crate::game::client_side) fn apply_predicted_item(
        &mut self,
        pos: [i32; 3],
        stack: &ItemStack,
    ) {
        let mut stack = stack.clone();
        if stack.data().is_some() {
            stack
                .data_mut()
                .set_other(block_entity_item::BLOCK_ENTITY_DATA, None);
        }
        if self.apply_placed_item_common(pos, &stack, false).is_some() {
            self.side_update_spawner_display((pos[0], pos[1], pos[2]));
        }
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::block_items::BlockItemsGameExt1 as _;
