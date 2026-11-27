//! The `living_incoming_damage` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `LivingIncomingDamageEvent`: `LivingEntity.hurtServer`, past its guards (an
    /// invulnerable or dead entity, fire on a fire-resistant one), is about to hurt entity
    /// `entity` of type `kind` (`minecraft:zombie`, `minecraft:player`) by `source`: its damage
    /// type (`minecraft:fall`), causing entity (`DamageSource.getEntity`: the shooter of an
    /// arrow), direct entity (`getDirectEntity`: the projectile's entity id when the
    /// damage type names one, on a player, mob or ender dragon; 0 for `magic()` and any
    /// other source with no direct entity, even during that projectile's impact), and
    /// where it comes from (`getSourcePosition`). `victim` is where the entity stands and
    /// looks, and `weapon` the direct entity's weapon (`getWeaponItem`) when it is known.
    /// `causing_weapon` is the causing living entity's current main-hand weapon
    /// (`getEntity().getWeaponItem`), read at hit time, not a projectile's saved
    /// weapon; None for an absent, unknown or non-living cause, or once the shooter
    /// is gone. Both weapon views lend the hit-time state. A melee hit can lend the
    /// same view for both fields; unavailable weapons do not suppress the event.
    /// `causing_weapon` is an addition: NeoForge's `LivingIncomingDamageEvent` doesn't
    /// carry it, and mods there read the source entity's main hand themselves.
    /// `container` is a live mutable borrow of the hit's damage container before any
    /// reduction; engine sequencing and normalization remain game-owned. Change its
    /// `new_damage`, the invulnerability ticks a fresh hit leaves (vanilla's 20), or
    /// install an owned reduction modifier closure with `add_reduction_modifier` (run in
    /// registration order when that reduction stage executes, receiving a read-only view
    /// of the live container and the preceding reduction value). Set `canceled` and nothing
    /// of `hurtServer` happens (no damage, hurt sound, knockback, cooldown, statistics or
    /// criteria); a canceled hit on the ender dragon still counts as landed (`EnderDragon.hurt`
    /// returns true whatever `reallyHurt` did). It fires on every damage route, for players,
    /// mobs and the ender dragon. The host is the hit's (`HitHost`): its blocks, random and own
    /// attachments, level sounds, and the own random of the entities the hit lends; the hurt entity
    /// is in the middle of its hit.
    #[hook(
        dispatch = "api_carrier",
        side = "server",
        contract = "cancel",
        out = "canceled",
        host = "hit",
        neoforge = "LivingIncomingDamageEvent",
        version = 6,
        target = "sunrise_gameplay_common::game::native_hooks::base::HitLevel<'_>::native_living_incoming_damage"
    )]
    fn living_incoming_damage(
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        victim: &Victim,
        weapon: Option<&ItemStackView<'_>>,
        causing_weapon: Option<&ItemStackView<'_>>,
        container: mod_api::DamageContainer<'_>,
        canceled: &mut bool,
    );
}
