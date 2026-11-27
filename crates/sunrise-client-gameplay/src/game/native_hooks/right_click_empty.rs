use sunrise_sim::first_person_sim::Hand;

/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// The local player right-clicks empty space with an empty `hand`.
    /// Client-side only; the server does not receive this event.
    pub(in crate::game) fn native_right_click_empty(&mut self, hand: Hand) {
        self.fire_right_click_empty(hand);
    }
}
