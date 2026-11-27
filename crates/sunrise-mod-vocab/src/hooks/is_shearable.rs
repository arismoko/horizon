//! The `is_shearable` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether `entity` can be sheared now with `shears` (NeoForge's `IShearable.isShearable`),
    /// asked where a player's shears (`player` is who holds them) or a dispenser's (`player`
    /// none) meet it, before the `player_shear_entity` or `block_shear_entity` hook. The first
    /// part to answer wins, in dispatch order: `Some(true)` shears it through
    /// [`EntityPart::on_sheared`] in place of the species' own shearing, `Some(false)` leaves it
    /// unsheared (a player's click then does nothing), and `None` leaves it to the next part and
    /// then to the species. `shears` is a read-only view over the resident stack; call
    /// `shears.project()` only when components or an owned snapshot are needed. Both sides ask it,
    /// as `ShearsItem.interactLivingEntity` does (a client predicting a player's click answers
    /// the same question the server does); `host` says which side asks, and a dispenser's ask is
    /// the server's alone.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::is_shearable::GameHooks::native_is_shearable",
        receiver = "write",
        dispatch = "direct",
        side = "both",
        contract = "replace",
        version = 2,
        out = "shearable",
        scope = "entity",
        key = "entity",
        neoforge = "IShearable.isShearable"
    )]
    fn is_shearable(
        entity: EntityRef,
        player: Option<PlayerRef>,
        shears: ItemStackView<'_>,
        shearable: &mut Option<bool>,
    );
}
