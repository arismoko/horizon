//! The `on_sheared` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Shears `entity` (NeoForge's `IShearable.onSheared`): called on the part whose
    /// [`EntityPart::is_shearable`] said `Some(true)`, once the shear hooks let it through.
    /// Change the entity and play its sound here, and append drops to `event.drops`: the game
    /// spawns each as `IShearable.spawnShearedDrop` does (a snow golem's at its eye height, a
    /// bogged's and a mooshroom's at the top of its box, a mooshroom's with no pickup delay, any
    /// other one block above its feet with a small random push drawn only once the item joined), posts the shear game event and damages the shears. `shears` is a read-only
    /// view over the resident stack; call `shears.project()` only when components or an owned
    /// snapshot are needed. Both sides run it
    /// (`ShearsItem.interactLivingEntity` calls `onSheared` on both): a client predicting the click
    /// changes its copy of the entity and plays its sound, and the `drops` it fills are discarded
    /// there, since only the server spawns drops, damages the shears and shears for a dispenser
    /// (`host` says which side runs).
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::on_sheared::GameHooks::native_on_sheared",
        receiver = "write",
        dispatch = "direct",
        side = "both",
        contract = "collect",
        out = "drops",
        version = 2,
        scope = "entity",
        key = "entity",
        neoforge = "IShearable.onSheared"
    )]
    fn on_sheared(
        entity: EntityRef,
        player: Option<PlayerRef>,
        shears: ItemStackView<'_>,
        drops: &mut Vec<ItemStack>,
    );
}
