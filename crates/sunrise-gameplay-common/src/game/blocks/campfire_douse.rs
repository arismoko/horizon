//! `CampfireBlock.useItemOn`: an item that can `shovel_douse` (vanilla's
//! `#minecraft:douses_campfires`, the shovels) put on a lit campfire puts it out.
use super::Hit;
use crate::game::item_abilities::ItemAbilitiesGameExt1 as _;
use crate::game::tool_modification::ToolModificationGameExt1 as _;
use sunrise_items::items::ItemStack;
use sunrise_world::chunk::BlockState;

pub trait CampfireDouseGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// The lit campfire at `pos` that `held` puts out, when the item can `shovel_douse`
    /// (`#minecraft:douses_campfires`, `CampfireBlock.useItemOn`).
    fn campfire_to_douse(&self, pos: [i32; 3], held: Option<&ItemStack>) -> Option<BlockState> {
        held.filter(|s| {
            let douses = self
                .common()
                .res
                .items()
                .item_tags
                .get("minecraft:douses_campfires")
                .is_some_and(|members| members.contains(&*s.item));
            self.stack_can_perform(s, super::item_abilities::ability::SHOVEL_DOUSE, douses)
        })?;
        self.common()
            .pack
            .assets
            .registry
            .state(self.common().world.block(pos[0], pos[1], pos[2]))
            .filter(|s| matches!(&*s.name, "minecraft:campfire" | "minecraft:soul_campfire"))
            .filter(|s| s.properties.get("lit").map(String::as_str) == Some("true"))
            .cloned()
    }

    fn modified_campfire(
        &self,
        hit: &Hit,
        stack: &ItemStack,
        unlit: sunrise_world::registry::BlockStateId,
    ) -> Option<sunrise_world::registry::BlockStateId> {
        self.hit_tool_modification(
            hit,
            stack,
            mod_api::ToolAbility::ShovelDouse,
            Some(unlit),
            true,
        )
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> CampfireDouseGameExt1<'__sg> for G {}
