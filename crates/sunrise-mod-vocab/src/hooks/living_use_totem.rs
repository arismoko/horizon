//! The `living_use_totem` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `LivingUseTotemEvent` at `LivingEntity.checkTotemDeathProtection`.
    /// Fires for each eligible hand's actual `death_protection` stack in main-hand,
    /// then off-hand order, until a candidate is accepted. Cancel to leave that
    /// stack alone and try the next hand. Damage that bypasses invulnerability
    /// never checks either hand. Accepted protection resolves before damage Post.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "hit",
        neoforge = "LivingUseTotemEvent",
        cancellable = true,
        version = 4,
        target = "sunrise_gameplay_common::game::native_hooks::living_use_totem::HitLevel<'_>::native_living_use_totem"
    )]
    fn living_use_totem(
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        hand: Hand,
        stack: &ItemStack,
        cancel: &mut bool,
    );
}
