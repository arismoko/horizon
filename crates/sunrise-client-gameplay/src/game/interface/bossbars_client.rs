use crate::game::ClientGame;
use sunrise_client::boss_overlay;
use sunrise_protocol::packets::play::{self};

impl ClientGame<'_> {
    /// `ClientPacketListener.handleBossUpdate`.
    pub(in crate::game::client_side) fn apply_boss_event(&mut self, packet: &play::BossEvent) {
        self.client
            .boss_overlay
            .apply(packet, boss_overlay::now_millis());
    }
    #[allow(dead_code)]
    #[inline(always)]
    pub(in crate::game) fn publish_boss_bars_client(&mut self, event: &play::BossEvent, now: u128) {
        self.client.boss_overlay.apply(event, now);
    }
}
