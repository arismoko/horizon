//! The `enchant_power_bonus` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The enchanting power the block `state` at `pos` gives the enchanting table at
    /// `table_pos` (`IBlockExtension.getEnchantPowerBonus`), asked for each of the table's
    /// bookshelf offsets (`EnchantingTableBlock.BOOKSHELF_OFFSETS`) whose block between it and the
    /// table is in `#minecraft:enchantment_power_transmitter`. `power` starts at the block's own
    /// answer, 1.0 in `#minecraft:enchantment_power_provider` and 0.0 for the rest: set 2.0 to
    /// count as two bookshelves, 0.5 for half of one, 0.0 to give nothing. The table adds the
    /// answers, truncates the sum to a whole number as NeoForge does and caps it at 15 for the
    /// offers; a negative sum counts as 0, where NeoForge would throw, and a non-finite answer is
    /// ignored. Asked as the offers are made, in a single-player world and on a LAN host for each
    /// guest's table, with the level read-only. Diverges from NeoForge: the table's enchant
    /// particles still read the provider tag, as they are drawn inside the particle pass where
    /// no host can be lent; they are not asked of this hook. With no part attached to the state,
    /// the game checks the hook subscription and the block key before any event conversion, so
    /// the cost is one table lookup per offset. Diverges from upstream: new hook.
    #[hook(
        side = "server",
        contract = "collect",
        out = "power",
        scope = "block",
        key = "state",
        host = "read",
        neoforge = "IBlockExtension.getEnchantPowerBonus, IBlockStateExtension.getEnchantPowerBonus"
    )]
    fn enchant_power_bonus();
}
