use super::*;

/// The hook point. It does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// Ask `living_breathe` about the air update `step` decided for the player in
    /// `living`, then check with `unchanged` that what the decision read still
    /// stands. The listener's writes and that check are one transaction: a
    /// refusal (`None`) rolls the listener's transactional writes back and
    /// leaves the air as it is, and the caller applies nothing.
    pub(crate) fn ask_breath(
        &mut self,
        living: mod_api::LivingRef,
        vanilla: sunrise_sim::breathing::Breath,
        unchanged: impl FnOnce(&Self) -> bool,
    ) -> Option<sunrise_sim::breathing::Breath> {
        let mut breath = vanilla;
        let failures = mod_api::runtime::isolated_failures();
        let answered: Result<(), ()> = self.storage_transaction(|game, _| {
            game.native_living_breathe(
                living,
                &mut breath.can_breathe,
                &mut breath.consume,
                &mut breath.refill,
            );
            if mod_api::runtime::isolated_failures() == failures && unchanged(game) {
                Ok(())
            } else {
                Err(())
            }
        });
        answered.ok().map(|()| breath)
    }
    /// A paused mob's breathing, as `ask_player_breath` does for a player.
    pub(crate) fn ask_mob_breath(
        &mut self,
        pause: &sunrise_sim::mobs::ai::AirPause,
    ) -> Option<sunrise_sim::breathing::Breath> {
        let living = self
            .entity(pause.mob)
            .and_then(|entity| self.living(entity))?;
        self.ask_breath(living, pause.step.vanilla, |game| {
            game.mobs.iter().any(|m| pause.still_valid(m))
        })
    }
    /// The acting (guest) player's breathing, the answer to apply to the air
    /// `step` decided on, or `None` when the player is gone or their air changed.
    pub fn ask_player_breath(
        &mut self,
        step: sunrise_sim::breathing::AirStep,
    ) -> Option<sunrise_sim::breathing::Breath> {
        let id = self.native_acting_entity();
        let living = self.entity(id).and_then(|entity| self.living(entity))?;
        self.ask_breath(living, step.vanilla, move |game| {
            game.player
                .as_ref()
                .is_some_and(|player| player.air == step.air && player.entity_id as u32 == id)
        })
    }
    /// A stand's breathing, as `ask_player_breath` does for a player.
    pub(in crate::game) fn ask_stand_breath(
        &mut self,
        id: u32,
        step: sunrise_sim::breathing::AirStep,
    ) -> Option<sunrise_sim::breathing::Breath> {
        let living = self.entity(id).and_then(|entity| self.living(entity))?;
        self.ask_breath(living, step.vanilla, move |game| {
            game.armor_stands
                .iter()
                .any(|a| a.id == id && a.health > 0.0 && a.air == step.air)
        })
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
