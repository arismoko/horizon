//! Real skull, rocket, command and zero-damage projectile entries.
use mod_api::{
    EnderPearlTeleportEvent, EntityInvulnerabilityCheckEvent, Flow, Hooks, LevelSide, ServerHost,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub struct Probe(pub Arc<AtomicUsize>);
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn entity_invulnerability_check(
        &self,
        _: LevelSide<'_>,
        _: &mut EntityInvulnerabilityCheckEvent<'_>,
    ) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
    fn ender_pearl_teleport(
        &self,
        _: &mut dyn ServerHost,
        event: &mut EnderPearlTeleportEvent<'_>,
    ) -> Flow {
        event.damage = 0.0;
        Flow::Continue
    }
}
