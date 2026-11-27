use super::*;

/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// An anvil at `pos` made vanilla's result with a left input, for
    /// `player` (UUID) and `player_entity_id` (`AnvilUpdateEvent`, after
    /// `AnvilMenu.createResult`), whenever an input or the name changed:
    /// lend `inputs` and let listeners stage only the result and cost
    /// `edits`. Both sides: the server's menu and a server's client menu
    /// with the local player.
    fn native_anvil_update(
        &mut self,
        player: u128,
        player_entity_id: i32,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::AnvilUpdateInputs<'_>,
        edits: &mut mod_api::item::menu::AnvilUpdateEdits,
        cancel: &mut bool,
    ) {
        let _dispatch = AnvilUpdateDispatch::enter();
        crate::game::native_hooks::fire::FireHooks::fire_anvil_update(
            self,
            player,
            player_entity_id,
            pos,
            inputs,
            edits,
            cancel,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}

/// Shared body for `PlayerRef::anvil_update` once the caller has established the player's turn.
pub fn run_player_anvil_update<'g, G: crate::game::SideGame<'g>>(
    game: &mut G,
    player: mod_api::PlayerRef,
    entity: i32,
    pos: [i32; 3],
    inputs: &mod_api::item::menu::AnvilUpdateInputs<'_>,
) -> Option<mod_api::item::menu::AnvilUpdateEdits> {
    use mod_api::item::menu::{AnvilUpdateEdits, StackResult};
    let mut edits = AnvilUpdateEdits {
        output: StackResult::Keep,
        xp_cost: inputs.vanilla_xp_cost,
        material_cost: inputs.vanilla_material_cost,
    };
    let mut cancel = false;
    game.native_anvil_update(player.uuid(), entity, pos, inputs, &mut edits, &mut cancel);
    (!cancel).then_some(edits)
}

pub fn anvil_update_dispatch_running() -> bool {
    AnvilUpdateDispatch::running()
}

/// Marks an `anvil_update` dispatch on this thread while it runs, so a listener's
/// `PlayerRef::anvil_update` declines instead of running the listeners again without bound.
struct AnvilUpdateDispatch(bool);

thread_local! {
    static ANVIL_UPDATE_RUNNING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl AnvilUpdateDispatch {
    fn enter() -> Self {
        Self(ANVIL_UPDATE_RUNNING.with(|running| running.replace(true)))
    }
    fn running() -> bool {
        ANVIL_UPDATE_RUNNING.with(std::cell::Cell::get)
    }
}

impl Drop for AnvilUpdateDispatch {
    fn drop(&mut self) {
        ANVIL_UPDATE_RUNNING.with(|running| running.set(self.0));
    }
}
