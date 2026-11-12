//! The `living_drown` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0345

pub trait Hooks {
    /// An entity that can't breathe and has no air (`LivingDrownEvent`, posted
    /// from `LivingEntity.baseTick`), with vanilla's answers as the defaults: it
    /// drowns at -20 air, for 2 damage and 8 bubbles. Change `is_drowning` or
    /// `damage_amount`, or cancel to skip drowning this tick. Bubble count is
    /// read-only; changes to the event field do not affect the vanilla entity
    /// event. Asked on servers, for the entities `living_breathe` is.
    /// It is given the loop `context` of the server that ticks the entity (its
    /// registries, settings and mod list), because it fires inside the tick's
    /// own air and damage update.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        context = "loop",
        neoforge = "LivingDrownEvent",
        version = 2,
        target = "horizon_client_lib::breathing::native_living_drown"
    )]
    fn living_drown();
}
