use super::Screen;
use sunrise_client::chat::{ChatAbilities, ChatVisibility};
use sunrise_command::args::GameMode;
use sunrise_gameplay_common::game::game_mode_common::inventory_after_infinite_materials;

impl crate::game::ClientGame<'_> {
    /// `Minecraft.computeChatAbilities` from the Chat option.
    pub(in crate::game) fn chat_abilities(&self) -> ChatAbilities {
        ChatAbilities::new(ChatVisibility::from_option(
            self.client.settings.chat_visibility,
        ))
    }
    /// `MultiPlayerGameMode.setLocalMode(GameType)`: updates only the local mode and `Abilities`.
    pub fn set_local_mode(&mut self, mode: GameMode) {
        if mode != self.common.mode {
            self.common.previous_mode = Some(self.common.mode);
        }
        self.common.mode = mode;
        self.update_world_options_personal_mode(mode.id());
        if let (Some(r), Some(p)) = (self.client.remote.as_mut(), self.common.player.as_ref()) {
            r.sync_body_flight(p.flying, p.flying_speed);
        }
        self.finish_local_mode_update(mode);
    }
    /// `MultiPlayerGameMode.setLocalMode(GameType, @Nullable GameType)` for `Login` and `Respawn`.
    pub(in crate::game::client_side) fn set_local_mode_with_previous(
        &mut self,
        mode: GameMode,
        previous: Option<GameMode>,
    ) {
        self.common.mode = mode;
        self.common.previous_mode = previous;
        self.finish_local_mode_update(mode);
    }
    pub(in crate::game::client_side) fn finish_local_mode_update(&mut self, mode: GameMode) {
        let r = self
            .client
            .remote
            .as_mut()
            .expect("set_local_mode requires a remote client");
        self.common.creative = r.update_local_mode_abilities(mode.id());
        self.apply_remote_abilities();
        let local_spectator = self.common.spectator();
        if let Some(player) = self.common.player.as_mut() {
            player.local_spectator = local_spectator;
        }
    }
    /// Whether `Screen::Inventory` currently shows the player's own `InventoryScreen`
    /// rather than a container, crafting table, workstation, mount, or server/mod menu.
    pub fn is_player_inventory_screen(&self) -> bool {
        self.client.screen == Screen::Inventory
            && self.common.container.is_none()
            && self.common.station.is_none()
            && self.common.craft_size == 2
            && !self.mount_active()
            && self.client.mod_screen.0.is_none()
            && self.remote_menu_type().is_none()
    }
    /// Reconcile an open player inventory on its screen tick (`InventoryScreen.containerTick`
    /// and `CreativeModeInventoryScreen.containerTick`) against live `instabuild`, keeping
    /// the shared cursor (`carried`) stack across the switch.
    pub(in crate::game::client_side) fn tick_inventory_screen(&mut self) {
        if self.common.player.is_none() {
            return;
        }
        let next = inventory_after_infinite_materials(
            self.client.screen,
            self.is_player_inventory_screen(),
            self.instabuild(),
        );
        if next == self.client.screen {
            return;
        }
        match next {
            Screen::Creative => {
                self.set_screen(Screen::Creative);
                if self.client.screen == Screen::Creative {
                    self.refresh_creative_tabs();
                    self.client.creative_screen.0.reopen();
                    self.refresh_saved_hotbars();
                }
            }
            Screen::Inventory => {
                self.set_screen(Screen::Inventory);
            }
            _ => {}
        }
    }
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;
