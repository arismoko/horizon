use crate::game::ServerGame;

impl ServerGame<'_> {
    #[inline(always)]
    pub(in crate::game) fn shield_cooldown(&self) -> u32 {
        self.server.shield_cooldown
    }
}
