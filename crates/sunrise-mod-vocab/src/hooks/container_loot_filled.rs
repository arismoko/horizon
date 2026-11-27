//! The container loot fill hook.

pub trait Hooks {
    /// Sunrise addition: no NeoForge counterpart. Fired once per container fill,
    /// each half of a double chest separately, on the rolled list in roll order
    /// before shuffle, splitting and slot placement. Add, replace or remove drops;
    /// empty stacks are dropped. Stacks over their item's maximum are trimmed at
    /// placement, after the hook; overflow that cannot fit is discarded.
    /// The opener is named only when a player's open starts the fill. Other
    /// accesses (including an `ItemPlace::EntitySlot` read, even during a player
    /// action) give None. The origin is the block's centre or the container entity's
    /// position. Already filled containers and block or mob drops do not fire this
    /// hook. Listeners run in dispatch order, each seeing the previous list, after
    /// loot_table_load's changes.
    /// The level Host provides reads only; only the drop list is writable.
    /// Non-negative damage within the signed integer range is accepted even above
    /// an item's maximum. The drop list checks each edit that supplies a stack. A refusal
    /// returns an error and warns without undoing other edits. A panic undoes only the
    /// panicking listener's edits. Listeners use their own randomness.
    /// With no listener, the roll and placement random draws remain unchanged.
    #[notes = r#"## Container loot

- **`container_loot_filled(context, drops)`**: Sunrise addition. Each block or chest vehicle fill exposes its rolled list before shuffle, splitting and placement. Each double-chest half fires separately. Replace, remove or add stacks; empty stacks disappear, oversized counts are trimmed to the effective maximum, and excess stacks that cannot fit are discarded. Context names the table, dimension, block position or entity id and block centre or entity position. Only a player open that initiates the fill names an opener; other accesses, including an ItemPlace::EntitySlot read during a player action, give None. Filled containers and block or mob drops do not fire it. The read-only level Host permits reads and only the list is writable. No API snapshots are built without listeners, and the roll and placement draws are unchanged. Codec-valid damage above max_damage is kept. The `HookList` validates each edit that supplies a stack. A refused edit warns and leaves other edits intact."#]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        version = 3,
        contract = "collect",
        out = "drops",
        stack_count = "trim",
        target = "sunrise_gameplay_common::game::native_hooks::container_loot_filled::GameLevel<'_>::native_container_loot_filled"
    )]
    fn container_loot_filled(context: &ContainerLootFilled<'_>, drops: &mut Vec<ItemStack>);
}
