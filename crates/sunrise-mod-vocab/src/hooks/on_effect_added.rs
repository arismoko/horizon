//! The `on_effect_added` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `MobEffect.applyEffectTick`: the content mod's resolved `EffectKey` `effect` acts on
    /// `entity` in `dimension` (`minecraft:overworld`). Clear `keep` and the
    /// effect ends now, before it counts down. `entity` is a mob's
    /// `Entity.getId`, None for a player (`host.players().acting_player()` says who).
    /// It runs inside the entity's tick: a mob's world borrows the game
    /// there, so the game is read-only, and it does not hold the ticking
    /// mobs, nor the player's effects. `holder` identifies the holder; Host
    /// reads of a mob holder are unavailable while it is ticking. Queued
    /// changes apply after the hook; clearing `keep` ends the effect.
    /// `active_effects` is the holder's live active-effect snapshot at this point in the
    /// pass; `has_effect(key)` reads it, reflecting removals earlier in the pass.
    /// The current level's game rules remain readable through `host.level(dimension)`.
    /// Called only when this Part's own cadence requests it. All Parts see
    /// the same snapshots; their queued changes apply in Part order afterward
    /// (with the declaring package's — or, for a `served_by` content package,
    /// its serving native host's — Parts running first).
    /// On mobs, each queued hurt finishes its damage, death or protection, and
    /// `living_damage_post` before the next queued change or the effect countdown.
    /// Clearing `keep` skips later Parts. Server-side.
    #[hook(
        side = "server",
        scope = "effect",
        key = "effect",
        target = "mod_api::parts::effects::dispatch_mob_effect_tick",
        version = 4,
        contract = "listen",
        host = "read",
        neoforge = "none (vanilla MobEffect.applyEffectTick; NeoForge adds no event or extension for it)"
    )]
    fn apply_effect_tick(
        effect: EffectKey,
        dimension: &str,
        entity: Option<i32>,
        amplifier: i32,
        holder: LivingRef,
        health: f32,
        max_health: f32,
        food: Option<PlayerFood>,
        active_effects: &[EffectKey],
        changes: &mut Vec<EffectTickChange>,
        keep: &mut bool,
        #[engine] tick_count: i32,
        #[engine] make_call: impl FnOnce(EffectKey, i32) -> ApplyEffectTickCall<'_>,
    );
    /// `MobEffect.onEffectAdded`: called on the [`MobEffectPart`]s attached to the mod effect
    /// `effect` after `LivingEntity.addEffect` inserts an instance of `effect` onto `entity`
    /// that the entity did not already have (after attribute modifiers and the vanilla added
    /// sound, before `onEffectStarted`). Does not fire on reapplications, `forceAddEffect`,
    /// world load or client sync. The declaring package's parts run first, then dependent
    /// packages in dispatch and registration order. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::mob_effect_events::ServerGame<'_>::native_on_effect_added",
        dispatch = "flat",
        side = "server",
        scope = "effect",
        key = "effect",
        version = 2,
        contract = "listen",
        neoforge = "none (vanilla MobEffect.onEffectAdded; NeoForge adds no event or extension for it)"
    )]
    fn on_effect_added(effect: EffectKey, entity: LivingRef, amplifier: i32);
}
