//! The `living_fall` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A living entity is about to handle a fall (`LivingFallEvent`). Mods may
    /// change the distance and damage multiplier, or cancel fall handling. It is given the loop `context` of the side that ticks the entity (its registries, settings and mod list); it fires inside the tick's own landing, before its damage and block effects, so it takes no `Host`.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "cancel",
        context = "loop",
        neoforge = "LivingFallEvent",
        version = 2,
        target = "sunrise_sim::landing::native_living_fall"
    )]
    fn living_fall(
        entity: u32,
        kind: &str,
        distance: &mut f64,
        multiplier: &mut f32,
        cancel: &mut bool,
    );
}
