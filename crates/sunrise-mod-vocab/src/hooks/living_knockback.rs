//! The `living_knockback` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `LivingKnockBackEvent`: `LivingEntity.knockback` is about to push entity
    /// `entity` of type `kind` (`minecraft:zombie`, `minecraft:player`) `strength` away from the
    /// direction (`x`, `z`), on the raw strength, before the entity's knockback resistance takes
    /// its share and its motion changes. Vanilla's direction points at the source of a hit (the
    /// attacker's side), and the entity is pushed the other way. `source` is the push's
    /// `DamageSource`, named as `native_living_incoming_damage` names a hit's. `attacker` is the
    /// attacking entity's network id, falling back to the victim's last hurt-by mob, or 0 when
    /// unknown; `attacker_main` and `attacker_off` lazily borrow what it holds as
    /// [`ItemStackView`] (a player's selected and off-hand stacks, a mob's main hand; none when
    /// unknown, empty or unrepresentable). `LivingRef::knockback` has an unattributed
    /// `minecraft:generic` source: this same attacker fallback and the victim's existing kill
    /// credit still apply. The call deals no damage and creates no credit. Vanilla's short
    /// `LivingEntity.knockback` overload delegates directly to the full overload, firing this
    /// event once per knockback operation, matching NeoForge's `CommonHooks.onLivingKnockBack`,
    /// which takes the strength as a float: `strength` is an `f32` (the engine's `f64` cast
    /// down), widened again after the edit. Change `strength`, `x` or `z`, or set `canceled`,
    /// and the push follows them: canceled, the entity's motion is left alone. A `strength` of 0
    /// or less after the hook is no push, and a direction too short to tell draws a random one
    /// after it, as in vanilla. A value a mod leaves that is not a finite number is put back as
    /// it was. It fires for `LivingRef::knockback` and for the players, mobs and armor stands
    /// the game knocks back: a hit's own push, a sprinting or Knockback hit, a sweep, a spear, a
    /// shield's push on its holder, a goat's ram and a mob's melee push. The ender dragon can
    /// also receive a default knockback when it is not sitting. The event does not fire when
    /// `SulfurCube.knockback` launches a cube carrying a block and the source has a causing
    /// entity, because it does not call the living implementation. Pushes that do not use
    /// `LivingEntity.knockback` also do not fire it: the dragon's wing push, Punch arrows,
    /// explosions, wind charges, a mace smash's area and a hoglin's throw. The level is the
    /// victim's: its blocks, random and own attachments.
    /// `kill_credit` is the pushed living entity's kill credit at this pre-motion moment:
    /// a remembered player takes precedence over its last hurting mob, even when that player
    /// cannot resolve (then 0, without falling back). `kill_credit_main` lazily borrows that
    /// entity's current main-hand stack as [`ItemStackView`], or none when unknown, empty or
    /// unrepresentable. Call [`ItemStackView::project`] only when components or an owned stack
    /// are needed; unrelated component projection failure never suppresses the event. This is
    /// not the immediate damage-source attacker. Accepted-hit memories, including a tame wolf's
    /// owner, are committed before this read. Both are additions: NeoForge's
    /// `LivingKnockBackEvent` doesn't carry them, and mods there read the pushed entity's kill
    /// credit and its holder's main hand themselves.
    #[notes = "Version 3 removes `entry` (`KnockbackEntry`) to fire one knockback moment matching NeoForge's `CommonHooks.onLivingKnockBack`, and lends `attacker_main`, `attacker_off`, and `kill_credit_main` lazily as `Option<ItemStackView<'_>>` so stack component projection failures never skip the event."]
    #[hook(
        dispatch = "api_carrier",
        side = "server",
        version = 3,
        contract = "cancel",
        out = "canceled",
        host = "level",
        neoforge = "LivingKnockBackEvent",
        target = "sunrise_gameplay_common::game::native_hooks::base::HitLevel<'_>::native_living_knockback"
    )]
    fn living_knockback(
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        attacker: i32,
        attacker_main: Option<ItemStackView<'_>>,
        attacker_off: Option<ItemStackView<'_>>,
        kill_credit: i32,
        kill_credit_main: Option<ItemStackView<'_>>,
        strength: &mut f32,
        x: &mut f64,
        z: &mut f64,
        canceled: &mut bool,
    );
}
