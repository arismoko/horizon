use crate::game::ClientGame;
use sunrise_sim::first_person_sim::SwingAnimation;

impl ClientGame<'_> {
    /// A locally requested swing sends Punch only when the item's Part permits it.
    pub(in crate::game::client_side) fn remote_swing_packet(&mut self) {
        if self.try_swing_hand(self.common.hand, SwingAnimation::DEFAULT) {
            self.send(sunrise_protocol::packets::play::Serverbound::Punch(
                sunrise_protocol::packets::Empty,
            ));
        }
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;
