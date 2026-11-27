//! The `dispense_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `DispenserBlock.getDispenseMethod`: choose a behaviour for any item,
    /// including vanilla items, on the authoritative server (not droppers).
    /// The block the dispenser faces is asked first (`dispense_method`), and
    /// its answer keeps this part from being asked. Leave answer None for
    /// vanilla. Otherwise answer `(remainder, event)`:
    /// a remainder of None empties the slot, `Some(stack)` replaces it (the hook
    /// owns consumption: no extra item is removed afterwards). `event` is the
    /// level event the dispenser plays with its smoke: 1000 the click, 1001 the
    /// fail sound (`OptionalDispenseItemBehavior` without success), or 0 for no sound or smoke.
    /// In an open transaction in the dispenser's level, sound and smoke follow the behaviour's
    /// published effects at the outermost commit and are discarded on abort.
    /// The remainder is detached until the dispenser writes its slot. An invalid remainder is
    /// reported without writing or notifying the slot, and no other behaviour runs. To drop one
    /// normally, resolve `host.level(dimension)`, call `level.dispense_default(host, pos, facing, stack)`,
    /// and answer `(remainder, 1000)` on `Ok(remainder)`.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_dispense_item",
        dispatch = "flat",
        side = "server",
        contract = "replace",
        out = "answer",
        scope = "item",
        key = "stack",
        neoforge = "none (vanilla DispenserBlock.getDispenseMethod; NeoForge adds no event or extension for it)",
        version = 4
    )]
    fn dispense_item(
        dimension: &str,
        pos: [i32; 3],
        facing: Direction,
        stack: &ItemStack,
        answer: &mut Option<mod_api::Dispensed>,
    );
}
