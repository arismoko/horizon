//! The `living_damage_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `LivingDamageEvent.Post`: the health of entity `entity` of type `kind` has
    /// just lost the hit `container` describes, with its final values (health damage, the
    /// reductions, the post-attack invulnerability ticks). `container.should_cause_side_effects()`
    /// carries `hurtServer`'s fresh-hit decision (`shouldCauseSideEffects`): false for a stronger
    /// hit inside the cooldown unless its source bypasses it, independent of the final damage
    /// or modded invulnerability ticks. It only listens, and fires where
    /// `living_damage_pre` does once the health took more than 0. It can't write the
    /// entity or the world's blocks: the level host gives the hit's random and level attachments.
    /// The container is an API-owned scalar snapshot; the engine retains sequencing and normalization.
    #[hook(
        side = "server",
        contract = "listen",
        host = "level",
        neoforge = "LivingDamageEvent.Post",
        target = "horizon_client::game::native_hooks::living_damage_post::HitLevel<'_>::native_living_damage_post"
    )]
    fn living_damage_post();
}
