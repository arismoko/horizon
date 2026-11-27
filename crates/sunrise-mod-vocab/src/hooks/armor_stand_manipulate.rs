//! The `armor_stand_manipulate` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player takes an item from an armor stand or puts one on it
    /// (`ArmorStand.interactAt`, after the stand's disabled-slot checks and
    /// `swapItem`'s take/put locks, before anything moves): `stand_id` is the
    /// stand's `Entity.getId` and `slot` the one the click reached
    /// (`mainhand`, `offhand`, `feet`, `legs`, `chest` or `head`). Set
    /// `cancel` and nothing changes: neither the stand nor the player's hand,
    /// and the click is spent. The player's state is swapped in for the call;
    /// `player` is the UUID the helpers take. Not told of a click that changes
    /// nothing (an empty hand on an empty slot). The listener may change
    /// the world: straight after it returns without cancelling, the game
    /// finds the stand again by `stand_id` and checks that it still exists,
    /// that the slot's stack and the held stack keep their item and count,
    /// and that the slot is not now locked; if not, the click is spent and
    /// nothing moves. Not covered, other ways a
    /// stand's equipment moves: breaking or blasting the stand (its items
    /// drop), a dispenser putting armour on one, a mod's own changes.
    /// Server-side, for every player of the world.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_armor_stand_manipulate",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (Bukkit's PlayerArmorStandManipulateEvent)",
        cancellable = true
    )]
    fn armor_stand_manipulate(
        player: u128,
        name: &str,
        stand_id: i32,
        slot: &str,
        cancel: &mut bool,
    );
}
