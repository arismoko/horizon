impl crate::game::ServerGame<'_> {
    /// `ServerPlayer.die` → `Entity.awardKillScore` on the entity the dying
    /// guest's combat record credits (`LivingEntity.getKillCredit`): the
    /// guest's `entity_killed_player`, with that entity and the death's
    /// `killing_blow`. Nothing credited, nothing fires.
    pub(in crate::game) fn guest_killed_player_trigger(
        &mut self,
        conn: u32,
        at: [f64; 3],
        blow: Option<(String, Option<u32>)>,
    ) {
        const TRIGGER: &str = "minecraft:entity_killed_player";
        let now = self.common.world.game_time;
        let Some(id) = self
            .guest_combat_record(conn)
            .and_then(|c| c.kill_credit(now))
        else {
            return;
        };
        let Some(killer) = self.guest_entity_view(id, TRIGGER) else {
            return;
        };
        let blow = self.guest_killing_blow(blow, at, TRIGGER);
        self.with_dying_guest(conn, |g| {
            g.trigger_killed_as(TRIGGER, &killer, blow.as_ref());
        });
    }

    /// The dying guest's last blow (`LivingEntity.getLastDamageSource`), its
    /// damage type and causing entity as the combat record held them, as a
    /// damage source: the projectile as the direct entity.
    pub(in crate::game) fn guest_killing_blow(
        &self,
        blow: Option<(String, Option<u32>)>,
        at: [f64; 3],
        trigger: &str,
    ) -> Option<sunrise_sim::advancements::DamageView> {
        let (damage, attacker) = blow?;
        // The damage's causing entity: the player in place, another guest, or
        // a mob (`DamageSource.getEntity`).
        let source = attacker.and_then(|id| self.guest_entity_view(id, trigger));
        Some(sunrise_sim::advancements::recorded_blow(damage, source, at))
    }
}
