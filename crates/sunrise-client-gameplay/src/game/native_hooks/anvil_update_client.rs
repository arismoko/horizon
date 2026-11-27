use sunrise_gameplay_common::game::native_hooks::anvil_update::{
    anvil_update_dispatch_running, run_player_anvil_update,
};
use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

impl crate::game::ClientGame<'_> {
    /// `PlayerRef::anvil_update`: a mod's own anvil menu runs the `anvil_update` listeners
    /// (`CommonHooks.onAnvilUpdate`) for the local `player`, from `Keep` and the menu's costs.
    /// None when a listener canceled, for another player, and from inside an `anvil_update` listener.
    pub(in crate::game) fn player_anvil_update(
        &mut self,
        player: mod_api::PlayerRef,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::AnvilUpdateInputs<'_>,
    ) -> Option<mod_api::item::menu::AnvilUpdateEdits> {
        if anvil_update_dispatch_running() || self.local_player() != Some(player) {
            return None;
        }
        let entity = i32::try_from(player.entity().id()).ok()?;
        run_player_anvil_update(self, player, entity, pos, inputs)
    }
}
