//! The `entity_interact` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player right-clicks the entity `entity` (`Entity.getId`) of type
    /// `kind` (`minecraft:cow`) with `hand` (`Hand::Main` or `Hand::Off`), holding `stack` (none in an
    /// empty hand): where `Player.interactOn` starts, before the entity's own
    /// `interact` and the item's `interactLivingEntity`. Both logical sides
    /// answer independently, once per hand, main hand before off hand.
    /// A consuming client result ends the loop; only Success swings there.
    /// SuccessServer and Consume consume without a client swing. Fail,
    /// Pass and TryWithEmptyHand fall through on this entity path.
    /// A server-only answer leaves the client's off-hand prediction unchanged.
    /// ClientHost permits reads and local effects, not authoritative writes.
    /// A client without this mod predicts vanilla here; the server corrects it.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::GameHooks::native_entity_interact",
        dispatch = "flat",
        side = "both",
        version = 2,
        contract = "replace",
        out = "result",
        neoforge = "PlayerInteractEvent.EntityInteract"
    )]
    fn entity_interact(
        hand: Hand,
        entity: i32,
        kind: &str,
        stack: Option<&ItemStack>,
        result: &mut Option<mod_api::InteractionResult>,
    );
}
