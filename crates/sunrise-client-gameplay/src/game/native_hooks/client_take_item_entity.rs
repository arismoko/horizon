use super::*;

impl crate::game::ClientGame<'_> {
    /// `ClientPacketListener.handleTakeItemEntity` heard `collector` take `amount` of
    /// `collected`, before the client acts on it. Client-side.
    pub(in crate::game) fn native_client_take_item_entity(
        &self,
        collected: EntityRef,
        collector: Option<EntityRef>,
        amount: i32,
    ) {
        self.fire_client_take_item_entity(collected, collector, amount);
    }
}
