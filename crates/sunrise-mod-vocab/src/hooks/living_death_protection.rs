//! The `living_death_protection` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A death protection held somewhere other than the hands, asked in
    /// `LivingEntity.checkTotemDeathProtection` once neither hand protected the dying entity:
    /// no hand had a `death_protection` stack, or `living_use_totem` refused each. Answer by
    /// setting `protector` to a stack carrying `minecraft:death_protection`, such as a totem worn
    /// in an accessory slot, and the entity survives as a hand's totem would make it: 1 health,
    /// that stack's `death_effects`, entity event 35 (the totem animation and sound), and for a
    /// player the `used` statistic and the `used_totem` trigger with that stack.
    ///
    /// The game takes nothing anywhere for an answer: the listener takes the item out of the
    /// place it found it in itself. Listeners run in order and each sees the answer so far,
    /// so answer only while `protector` is None. A stack without `death_protection` is no answer.
    /// Damage that bypasses invulnerability never asks. Beyond NeoForge: `LivingUseTotemEvent`
    /// only refuses a hand's stack; accessory mods inject into `checkTotemDeathProtection`.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "collect",
        out = "protector",
        host = "hit",
        neoforge = "none (vanilla LivingEntity.checkTotemDeathProtection; accessory mods inject there)",
        target = "sunrise_gameplay_common::game::native_hooks::living_death_protection::HitLevel<'_>::native_living_death_protection"
    )]
    fn living_death_protection(
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        protector: &mut Option<ItemStack>,
    );
}
