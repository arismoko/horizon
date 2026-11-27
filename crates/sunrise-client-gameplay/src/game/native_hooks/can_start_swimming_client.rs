use super::*;
use mod_api::SwimmingCheck;

impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn client_ask_swim_eligibility(
        &self,
        check: SwimmingCheck,
    ) -> Option<bool> {
        let player = self.local_player()?;
        if Some(player.entity().id()) != self.common.player.as_ref().map(|p| p.entity_id as u32) {
            return None;
        }
        let mut answer = None;
        self.native_can_start_swimming(player, LogicalSide::Client, check, &mut answer);
        answer
    }
}

use sunrise_gameplay_common::game::native_hooks::can_start_swimming::GameHooks as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
