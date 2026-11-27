//! The `living_drown` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entity that can't breathe and has no air (`LivingDrownEvent`, posted
    /// from `LivingEntity.baseTick`), with vanilla's answers as the defaults: it
    /// drowns at -20 air, for 2 damage and 8 bubbles. Change `is_drowning`,
    /// `damage_amount` or `bubble_count`, or cancel to skip drowning this tick.
    /// Eight bubbles are the vanilla entity event; another positive count is
    /// sent as that many bubble particles, and none sends nothing. Asked on
    /// servers, for the entities `living_breathe` is; a water animal,
    /// axolotl or nautilus shows no bubbles, as in vanilla.
    /// It is given the loop `context` of the server that ticks the entity (its
    /// registries, settings and mod list), because it fires inside the tick's
    /// own air and damage update.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        context = "loop",
        neoforge = "LivingDrownEvent",
        version = 3,
        target = "sunrise_sim::breathing::native_living_drown"
    )]
    fn living_drown(
        entity: u32,
        kind: &str,
        is_drowning: &mut bool,
        damage_amount: &mut f32,
        bubble_count: &mut i32,
        cancel: &mut bool,
    );
}
