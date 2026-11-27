//! Client-side local player attack strength reads (`Player.getAttackStrengthScale` and
//! `Player.getCurrentItemAttackStrengthDelay`, #5691).
use crate::game::ClientGame;
use mod_api::PlayerRef;
use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
use sunrise_gameplay_common::game::native_server::{NativeServerGameExt1 as _, PlayerRead};
use sunrise_sim::attack_strength::delay;

impl ClientGame<'_> {
    /// `Player.getAttackStrengthScale(partial)` for the local player on a client (#5691).
    /// Returns `None` for a non-finite partial tick or any non-local player.
    pub(in crate::game) fn player_attack_strength_scale(
        &self,
        player: PlayerRef,
        partial: f32,
    ) -> Option<f32> {
        if !partial.is_finite() {
            return None;
        }
        match self.player_read_both(player)? {
            PlayerRead::Acting { .. } => Some(
                self.client
                    .hand_strength
                    .scale(partial, self.main_hand_attack_speed()),
            ),
            PlayerRead::Guest(_) => None,
        }
    }

    /// `Player.getCurrentItemAttackStrengthDelay()` for the local player on a client (#5691).
    pub(in crate::game) fn player_current_item_attack_strength_delay(
        &self,
        player: PlayerRef,
    ) -> Option<f32> {
        match self.player_read_both(player)? {
            PlayerRead::Acting { .. } => Some(delay(self.main_hand_attack_speed())),
            PlayerRead::Guest(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    /// A client reads its own player's attack charge and full-charge delay from the ticker and
    /// main-hand attack speed the crosshair's attack indicator reads (`hand_strength`); any other
    /// player reads None (#5691).
    #[test]
    #[ignore = "daily"]
    fn a_client_reads_its_own_players_attack_charge_and_delay() {
        use super::super::native_client_session::tests::logged_client;
        use super::super::native_entity_refs::tests::with_client_game;
        use sunrise_core::geometry::Vec3;
        use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
        use sunrise_sim::motion::PlayerMotion;

        mod_platform::testing::with_mods(Default::default(), || {
            with_client_game(|game| {
                let _server = logged_client(game);
                game.common.player =
                    Some(PlayerMotion::new(game.common.motion, Vec3::ZERO).unwrap());
                let local = game.local_player().unwrap();
                let other = mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(local.entity().epoch(), 99),
                    99,
                );
                // An empty hand: the base attack speed, 4, so a full charge takes 5 ticks.
                let delay = 5.0;
                for (ticks, scale) in [(0, 0.0), (2, 0.4), (40, 1.0)] {
                    game.client.hand_strength.ticker = ticks;
                    let host = &*game as &dyn mod_api::ClientHost;
                    assert_eq!(local.current_item_attack_strength_delay(host), Some(delay));
                    assert_eq!(local.attack_strength_scale(host, 0.0), Some(scale));
                    assert_eq!(other.current_item_attack_strength_delay(host), None);
                    assert_eq!(other.attack_strength_scale(host, 0.0), None);
                }
                let host = &*game as &dyn mod_api::ClientHost;
                assert_eq!(local.attack_strength_scale(host, 0.0), Some(1.0));
                assert_eq!(local.attack_strength_scale(host, f32::NAN), None);
            });
        });
    }
}
