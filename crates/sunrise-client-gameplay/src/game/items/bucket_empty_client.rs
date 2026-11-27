use crate::game::ClientGame;
use sunrise_items::items::ItemStack;
use sunrise_world::level::BucketContent;

impl ClientGame<'_> {
    /// `bucket_contents` for the pour a server's client predicts: none for an
    /// item whose `empty_contents` Part answers on the server, whose result
    /// stands.
    pub(in crate::game::client_side) fn predicted_bucket_contents<'s>(
        &self,
        stack: &'s ItemStack,
    ) -> Option<(BucketContent, &'s str, Option<&'s str>)> {
        if mod_api::__private::has_part_hook(mod_api::HookId::EmptyContents)
            && mod_api::__private::item_listens(mod_api::HookId::EmptyContents, &stack.item)
        {
            return None;
        }
        sunrise_items::items::bucket_contents(&stack.item)
    }
}
