//! The pushes of `LivingEntity.pushEntities`: the stack a mob rides.
use super::MobWorld;

/// `Entity.getRootVehicle` of the mob `id` riding `vehicle`: the bottom of
/// its stack of vehicles (itself when it rides nothing).
pub(super) fn root_vehicle(w: &MobWorld, id: u32, vehicle: Option<u32>) -> u32 {
    let (mut root, mut next) = (id, vehicle);
    // A stack of riders is short; the bound only guards a broken loop.
    for _ in 0..16 {
        let Some(v) = next else { break };
        root = v;
        next = w.seen(v).and_then(|s| s.vehicle);
    }
    root
}
