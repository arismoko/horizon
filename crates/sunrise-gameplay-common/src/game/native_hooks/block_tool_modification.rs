use super::*;

/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// A tool is about to transform a block. Cancel or replace the final state;
    /// `None` suppresses the transformation. The block's own `tool_modified_state` part answers
    /// first, and a block whose part cancelled the action never reaches this hook.
    #[inline(never)]
    fn native_block_tool_modification(
        &self,
        stack: &ItemStack,
        tool: &ToolUse,
        final_state: &mut Option<BlockStateId>,
        cancel: &mut bool,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_block_tool_modification(
            self,
            stack,
            tool,
            final_state,
            cancel,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
