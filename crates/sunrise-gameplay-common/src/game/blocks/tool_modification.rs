//! A tool acting on a block (an axe strips, a shovel flattens or douses, a hoe tills): the
//! block's parts say what the block becomes, then the global hook hears the result.

use super::Hit;
use sunrise_items::items::ItemStack;
use sunrise_world::registry::BlockStateId;

pub trait ToolModificationGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// [`Self::mod_tool_modification`] for `stack` doing `ability` to the block `hit` names,
    /// held by this player (`by_player`) or by none (a dispenser), `state` being vanilla's
    /// answer: what the block becomes, `None` when nothing does or the action was cancelled.
    /// Without a listener, `state` stands.
    fn hit_tool_modification(
        &self,
        hit: &Hit,
        stack: &ItemStack,
        ability: mod_api::ToolAbility,
        state: Option<BlockStateId>,
        by_player: bool,
    ) -> Option<BlockStateId> {
        let block = self
            .common()
            .world
            .block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if !mod_api::__private::listened(mod_api::HookId::BlockToolModification)
            && !crate::game::api_values::block_state::block_listens(
                mod_api::HookId::ToolModifiedState,
                block,
            )
        {
            return state;
        }
        let tool = mod_api::ToolUse::from(mod_api::__private::built::ToolUse {
            player: (by_player && self.common().player.is_some())
                .then(|| self.own_entity_id() as u32),
            hand: self.common().hand,
            pos: hit.pos,
            face: hit.face,
            click: hit.point,
            ability,
            simulated: false,
        });
        let mut state = state;
        let mut cancel = false;
        self.mod_tool_modification(stack, block, &tool, &mut state, &mut cancel);
        if cancel { None } else { state }
    }

    /// `FlintAndSteelItem.useOn` / `FireChargeItem.useOn`'s (and the flint and steel dispense
    /// behaviour's) `getToolModifiedState(context, FIRESTARTER_LIGHT)`: when `stack` answers
    /// `firestarter_light`, the lit state of the campfire, candle or candle cake `hit` names
    /// (`canLight`), then the block's parts and `block_tool_modification`. `None` places fire
    /// at the face instead. `by_player` as [`Self::hit_tool_modification`] takes it.
    fn fire_starter_lit(
        &self,
        hit: &Hit,
        stack: &ItemStack,
        by_player: bool,
    ) -> sunrise_game_data::Result<Option<BlockStateId>> {
        let clicked = self
            .common()
            .world
            .block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let name = self.common().res.facts().get(clicked).name.clone();
        let lights = super::item_abilities::ability::FIRESTARTER_LIGHT;
        let vanilla =
            if self.stack_can_perform(stack, lights, true) && self.can_light(clicked, &name) {
                let registry = self.common().pack.assets.registry;
                let mut state = registry
                    .state(clicked)
                    .cloned()
                    .ok_or("missing lit block state")?;
                state.properties.insert("lit".into(), "true".into());
                Some(registry.resolve(&state)?)
            } else {
                None
            };
        let light = mod_api::ToolAbility::FirestarterLight;
        Ok(self.hit_tool_modification(hit, stack, light, vanilla, by_player))
    }

    /// What the tool action `tool` makes of the block `block`: `state` starts with vanilla's
    /// answer. The block's parts answer first (`tool_modified_state`,
    /// `IBlockExtension.getToolModifiedState`), and `block_tool_modification` hears their result
    /// unless they cancelled the action.
    fn mod_tool_modification(
        &self,
        stack: &ItemStack,
        block: BlockStateId,
        tool: &mod_api::ToolUse,
        state: &mut Option<BlockStateId>,
        cancel: &mut bool,
    ) {
        let template = *state;
        let mut inert = false;
        self.native_tool_modified_state(block, stack, tool, state, cancel, &mut inert);
        if *cancel {
            return;
        }
        let part_state = *state;
        self.native_block_tool_modification(stack, tool, state, cancel);
        // Suppress only an unchanged template answer. Leaving no result (rather than
        // cancelling) lets the tool try its next ability and notify listeners of it.
        if inert && part_state == template && *state == part_state {
            *state = None;
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> ToolModificationGameExt1<'__sg> for G {}

use crate::game::block_props::BlockPropsGameExt1 as _;
use crate::game::item_abilities::ItemAbilitiesGameExt1 as _;
use crate::game::native_hooks::block_tool_modification::GameHooks as _;
use crate::game::native_hooks::tool_modified_state::GameHooks as _;
use crate::game::player_identity::PlayerIdentityGameExt1 as _;
