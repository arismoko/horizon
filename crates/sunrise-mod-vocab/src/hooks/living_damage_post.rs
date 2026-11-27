//! The `living_damage_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `LivingDamageEvent.Post`: entity `entity` of type `kind` has been hurt by the
    /// hit `container` (a read-only live [`DamageContainerView`]) describes, with its final
    /// values (health damage, the reductions, the post-attack invulnerability ticks). It fires
    /// once for every hit that passed `living_incoming_damage` and the invulnerability cooldown,
    /// at the end of `hurtServer`: after the hurt sounds and after that hit's death processing
    /// (even if vetoed) or totem protection, before the hurt caller resumes. The next hazard,
    /// poison application or rider's hit sees the preceding hit already completed. It is not
    /// delayed to the next tick or the end of a player's turn. A queued mob effect's hurt
    /// finishes before the next action; a dolphin's dry-out hurt finishes before the land hop.
    /// A bee's drowning hurt finishes before it checks stinger expiry; an expiry hurt finishes
    /// before nectar and anger bookkeeping and movement controls.
    /// A server player's landing hurt finishes before the landing's `HIT_GROUND` and fall reset,
    /// and before the next accepted move. A mob landing in its own tick finishes its hurt before
    /// the fall reset, movement restitution, travel's collision hurt or a later goal. This
    /// includes a `BreathAir` move from either selector and a dolphin following a rider.
    /// A pufferfish sting on a mob finishes the hit before applying poison or starting the
    /// next sting, and before the fish's own `entity_tick_post`.
    /// A projectile hit finishes before its enchantment effects, levitation or a follow-up
    /// explosion. Each mount or passenger's hurt finishes before the fall routine continues
    /// to the next hit: a horse is hurt before its riders.
    /// `container.inflicted()` is the amount before absorption;
    /// `container.health_damage()` is the amount applied to health. A hit that took no
    /// health (a shield, absorption, armor or a `living_damage_pre` listener took it all) fires
    /// it too, with `container.health_damage()` 0. `container.should_cause_side_effects()`
    /// carries `hurtServer`'s fresh-hit decision (`shouldCauseSideEffects`): false for a stronger
    /// hit inside the cooldown unless its source bypasses it, independent of the final damage
    /// or modded invulnerability ticks. It only listens. It can't write the
    /// entity or the world's blocks: the host (`HitHost`) gives the hit's random and level
    /// attachments, level sounds, and the own random of the entities the hit lends.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        host = "hit",
        neoforge = "LivingDamageEvent.Post",
        version = 5,
        target = "sunrise_gameplay_common::game::native_hooks::living_damage_post::HitLevel<'_>::native_living_damage_post"
    )]
    fn living_damage_post(
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        container: mod_api::DamageContainerView<'_>,
    );
}
