//! The `item_consumption_teleport` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// On an authoritative server, a cauldron at `pos` is about to change
    /// fill level from `from_level` to `to_level` for `reason`. Set `cancel`
    /// to leave it unchanged. For `extinguishing` (a burning entity in a water
    /// or powder snow cauldron), the game checks after the hook returns without
    /// cancelling that the block at `pos` still has the block id and properties it
    /// judged; if not, the block is not changed.
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's CauldronLevelChangeEvent)",
        cancellable = true,
        target = "sunrise_gameplay_common::game::native_hooks::base::GameLevel<'_>::native_cauldron_level_changed"
    )]
    fn cauldron_level_changed(
        dimension: LevelDimension,
        player: Option<u128>,
        pos: BlockPos,
        from_level: i32,
        to_level: i32,
        reason: &str,
        cancel: &mut bool,
    );
    /// A server player or fox that ate chorus fruit (or another item with the
    /// `teleport_randomly` consume effect) is about to try a random spot
    /// (NeoForge's `EntityTeleportEvent.ItemConsumption`, vanilla's
    /// `TeleportRandomlyConsumeEffect.apply`): once per attempt (up to 16),
    /// after they stopped riding. `from` is their feet, `target` the spot
    /// drawn; a mod may move it (its y kept inside the level's height, a
    /// value that is not a number ignored), and the game still looks down
    /// from it for ground it may land on. Canceling ends the effect: no teleport and no
    /// more attempts. `consumer` names the living actor through its handle.
    /// `consumed` is the stack captured before consumption, including
    /// its count and components; it is not reread from the hand after listener writes.
    /// A player's state is swapped in (`host.players().acting_player()`). A fox's
    /// callback has the full server host before its living AI/movement step and
    /// before the next mob ticks. Listener writes survive; changing the consumer's
    /// item, position or eligibility refuses the captured effect instead of
    /// overwriting that write. Cancellation still finishes consumption.
    /// A consumed stack the API can't carry (a saved item whose id isn't registered) isn't
    /// heard: the listeners aren't called for that consumption and the teleport goes on as
    /// vanilla's.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_item_consumption_teleport",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        version = 2,
        out = "cancel",
        neoforge = "EntityTeleportEvent.ItemConsumption",
        cancellable = true
    )]
    fn item_consumption_teleport(
        consumer: mod_api::LivingRef,
        consumed: &mod_api::ItemStack,
        from: [f64; 3],
        target: &mut [f64; 3],
        cancel: &mut bool,
    );
}
