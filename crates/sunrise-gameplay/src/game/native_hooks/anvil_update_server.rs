use sunrise_gameplay_common::game::native_hooks::anvil_update::{
    anvil_update_dispatch_running, run_player_anvil_update,
};

impl crate::game::ServerGame<'_> {
    /// `PlayerRef::anvil_update`: a mod's own anvil menu runs the `anvil_update` listeners
    /// (`CommonHooks.onAnvilUpdate`) for `player`, from `Keep` and the menu's costs. None when
    /// a listener canceled, for a stale player, and from inside an `anvil_update` listener.
    pub(in crate::game) fn player_anvil_update(
        &mut self,
        player: mod_api::PlayerRef,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::AnvilUpdateInputs<'_>,
    ) -> Option<mod_api::item::menu::AnvilUpdateEdits> {
        if anvil_update_dispatch_running() {
            return None;
        }
        let entity = i32::try_from(player.entity().id()).ok()?;
        let mut answer = None;
        self.native_as_player_handle(player, |game| {
            answer = run_player_anvil_update(game, player, entity, pos, inputs);
        });
        answer
    }
}
