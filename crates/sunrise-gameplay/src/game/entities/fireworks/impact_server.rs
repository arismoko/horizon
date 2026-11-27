use super::*;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static ROCKET_IMPACT_PREPARES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl crate::game::ServerGame<'_> {
    /// Let listeners inspect and remove the live rocket, preserving its identity
    /// across the detached tick. Refuse a stale hit after writes.
    pub(in crate::game::server_side) fn rocket_impact(
        &mut self,
        index: usize,
        r: Rocket,
        hit: ProjectileHit,
    ) -> Option<(Rocket, bool)> {
        if matches!(hit, ProjectileHit::None) || !self.native_hears_impacts() {
            return Some((r, false));
        }
        #[cfg(any(test, feature = "test-support"))]
        ROCKET_IMPACT_PREPARES.with(|count| count.set(count.get() + 1));
        let (id, join, position, velocity) = (r.id, r.join, r.position, r.velocity);
        let owner = self.rocket_owner(&r);
        let target = self.native_impact_target(hit);
        // The resident owner keeps its slot during the tick: publish the advanced
        // state for the listeners, then read it back as the next tick clone.
        if !self.store_rocket_back(index, r) {
            return None;
        }
        let canceled =
            self.native_separate_projectile_impact(id, "minecraft:firework_rocket", owner, hit);
        let at = self.common.rockets.locate(join)?;
        if self
            .common
            .persist
            .removals
            .contains(&super::super::commands::Target::Rocket(id))
        {
            drop(self.common.rockets.extract(join)?);
            self.common.rockets.retire_extracted(id, join);
            return None;
        }
        let r = self.common.rockets[at].clone();
        let changed = r.position != position
            || r.velocity != velocity
            || target != self.native_impact_target(hit);
        Some((r, canceled || changed))
    }
}
