//! Bursts of key presses and clicks on a server's client: attack, drop and swap clicks drain
//! completely in the next tick's `handleKeybinds` order (`while (key.consumeClick())`), with
//! no fixed per-tick or queue-length cap. Each hotbar key keeps its own counter and consumes one
//! click per selection drain (`if (keyHotbarSlots[i].consumeClick())`).

/// How many of each packet `packets` holds: punches, player actions (drops and swaps) and slot
/// changes, in that order.
pub(crate) fn counts(packets: &[String]) -> [usize; 3] {
    [
        "minecraft:punch",
        "minecraft:player_action",
        "minecraft:set_carried_item",
    ]
    .map(|id| packets.iter().filter(|p| *p == id).count())
}
