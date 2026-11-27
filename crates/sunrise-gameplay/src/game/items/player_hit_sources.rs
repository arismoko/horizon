//! Player hit-source adapters.
use crate::game::ServerGame;
use sunrise_gameplay_common::game::creatures::KnownCause;

impl ServerGame<'_> {
    /// Damage whose causing mob is known (`cause`, or none): not guessed
    /// from the mob nearest to `from`.
    pub fn mob_hits_player_caused(
        &mut self,
        amount: f32,
        from: [f64; 3],
        kind: &str,
        cause: Option<u32>,
    ) {
        let known = cause.map_or(KnownCause::None, KnownCause::Entity);
        self.hurt_player_from(amount, Some(from), kind, known, false);
    }
    pub fn mob_hits_player(&mut self, amount: f32, from: [f64; 3], damage_type: &str) -> bool {
        self.hurt_player_from(amount, Some(from), damage_type, KnownCause::Infer, false)
    }
    /// [`mob_hits_player`](Self::mob_hits_player) whose hit was a creative
    /// player's, which passes the victim's invulnerability
    /// (`DamageSource.isCreativePlayer`, #916).
    pub fn mob_hits_player_from(
        &mut self,
        amount: f32,
        from: [f64; 3],
        damage_type: &str,
        by_creative_player: bool,
    ) -> bool {
        self.hurt_player_from(
            amount,
            Some(from),
            damage_type,
            KnownCause::Infer,
            by_creative_player,
        )
    }
}
