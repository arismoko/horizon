//! The `living_heal` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A living entity is about to heal (NeoForge's `LivingHealEvent`, posted at the start of
    /// `LivingEntity.heal`, which in vanilla 26.3 adds the amount through `setHealth` when
    /// `getHealth` is above 0). Mods may change the amount or cancel.
    /// `entity` identifies the affected entity, and `healed` lends it read-only
    /// ([`LivingView`], `LivingHealEvent.getEntity`): its health before the heal, its effects,
    /// attributes and held stacks as they are at this call, read through the same functions as
    /// the [`LivingRef`](crate::LivingRef) reads. It is given the loop `context` of the side that ticks the entity (its registries, settings and mod list), because it fires inside the tick's own health update, where a deferred call would reorder vanilla's effects; there is no host, so `healed` reads no attachments. The game fires it only for an entity of an open world, which the view names.
    #[notes = r#"## Heal and movement views

**`living_heal` version 3; `living_jump_power`, `living_speed` and `living_flying_speed` version 3:** each takes a read-only `&LivingView` of the entity it is about, after `entity` (and `kind`): `healed` for `living_heal` (`LivingHealEvent.getEntity`), `living` for the three motion values. The view is the one a hit lends at the damage hooks (`HitHost::victim`): `health`, `absorption`, `max_health`, `effect`, `effects`, `potion_affected`, `attribute_value`, `with_hand_item`, `no_ai`, `kind`, `entity`, `id` and `holder`, each answering from the entity's live state at the call through the function its `LivingRef` read uses, so an effect added, refreshed or removed earlier in the same tick is what the listener reads, with no one-tick cache. A player's view reads the effects and stacks of the side that ticks it: the server's, or the client's own copy in its prediction. These hooks take a loop context, not a host, so the view's `holder()` only names the entity; nothing reads or writes its attachments. **Migration:** add the parameter to the `Hooks` method (`healed: &LivingView<'_>` after `entity`; `living: &LivingView<'_>` after `kind`); nothing else changes. Tick-time effect: the game lends the body it already holds; with no listener nothing more is built, and with one it assembles the view's reads (the world's epoch, the attribute catalog, the player's effects and stacks) once per call."#]
    #[hook(
        side = "both",
        contract = "cancel",
        out = "cancel",
        context = "loop",
        neoforge = "LivingHealEvent",
        version = 3,
        target = "sunrise_sim::hunger::native_living_heal"
    )]
    fn living_heal(entity: u32, healed: &LivingView<'_>, amount: &mut f32, cancel: &mut bool);
}
