//! The `trade_with_villager` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player's take from a villager or wandering trader, once per trade, after its
    /// statistic and `villager_trade` criterion. `merchant` is the trader's entity id, `kind`
    /// its type, and `offer` the post-take snapshot: the use is counted and the result is what
    /// that iteration took. `host.players().acting_player()` names the player.
    ///
    /// This non-cancellable collect can edit the live offer for subsequent takes, including
    /// later iterations of the same shift-click or throw-all. It cannot change what the current
    /// iteration paid or received. The guest's whole result click and the listeners' transaction
    /// writes commit together. A listener unwind, an invalid offer answer, changed take
    /// preconditions, or an enclosing abort leaves no partial trade. A write through the `Host`
    /// to any player's inventory, cursor or menu storage makes the take stale.
    #[notes = "NeoForge's TradeWithVillagerEvent at MerchantResultSlot.onTake. This non-cancellable collect receives the post-take offer for each trade, with the acting player swapped in. Accepted offer edits affect the next iteration and reach the client's menu snapshot on commit; observing alone keeps the usual offer-refresh timing. MerchantRef edits and menu replacements that conflict with the active click are refused."]
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_trade_with_villager",
        dispatch = "direct",
        side = "server",
        contract = "collect",
        out = "offer",
        neoforge = "TradeWithVillagerEvent",
        cancellable = false,
        version = 4
    )]
    fn trade_with_villager(merchant: u32, kind: &str, offer: &mut MerchantOffer);
}
