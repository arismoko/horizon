//! The `living_jump` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A living entity jumped (`LivingEvent.LivingJumpEvent`): `entity` of `kind` (`minecraft:player`, `minecraft:cow`...), and `velocity`, its motion in blocks per tick right after the jump. It is given the loop `context` of the side that ticks the entity (its registries, settings and mod list); it fires inside the tick's own jump, after the velocity write, so it takes no `Host`. It fires only when the jump power is above vanilla's 1.0E-5 threshold, as NeoForge's event does. Like NeoForge's event it cannot be cancelled; the velocity is read-only, and a write to `event.velocity` is not applied (change the jump's height with `living_jump_power`).
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        neoforge = "LivingEvent.LivingJumpEvent",
        version = 3,
        target = "sunrise_sim::motion::native_living_jump"
    )]
    fn living_jump(entity: u32, kind: &str, velocity: [f64; 3]);
}
