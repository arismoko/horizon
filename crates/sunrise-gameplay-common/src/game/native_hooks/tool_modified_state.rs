use super::*;

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// The state the block `state` becomes under the tool action `tool` of the
    /// held `stack` (`IBlockExtension.getToolModifiedState`): `final_state`
    /// starts with vanilla's answer and the block's parts may allow, replace
    /// or, with `cancel`, deny it. Asked before the global
    /// `block_tool_modification`.
    #[inline(always)]
    fn native_tool_modified_state(
        &self,
        state: BlockStateId,
        stack: &ItemStack,
        tool: &ToolUse,
        final_state: &mut Option<BlockStateId>,
        cancel: &mut bool,
        inert: &mut bool,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_tool_modified_state(
            self,
            state,
            stack,
            tool,
            final_state,
            cancel,
            inert,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
