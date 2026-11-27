use super::*;

impl Hub {
    /// Publish the frozen server token inventory before sending any command trees.
    pub fn set_selector_tokens(
        &mut self,
        tokens: Vec<sunrise_game_data::registries::SelectorToken>,
    ) {
        self.selector_tokens = tokens;
    }

    /// Commands.sendCommands: publish selector tokens, then the player's finished command tree.
    pub fn send_commands(&self, i: usize, packet: play::Commands) {
        let g = &self.guests[i];
        if g.brand == "sunrise" {
            g.send(&C::CustomPayload(
                sunrise_protocol::packets::configuration::CustomPayload {
                    channel: crate::lan::commands::selector_sync::CHANNEL.into(),
                    data: crate::lan::commands::selector_sync::encode(&self.selector_tokens),
                },
            ));
        }
        g.send(&C::Commands(packet));
    }
}
