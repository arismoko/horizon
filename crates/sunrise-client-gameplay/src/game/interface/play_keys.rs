//! A key press while a world is open (`KeyboardHandler.keyPress` and the gameplay mappings).
use super::*;
use sunrise_client::keybinds::VanillaMappingSlot;

pub(in crate::game) const GUI_KEYS: [&str; 5] = [
    "key.toggleGui",
    "key.advancements",
    "key.socialInteractions",
    "key.chat",
    "key.command",
];

impl crate::game::ClientGame<'_> {
    /// `new InventoryScreen(player)`: the player's inventory, or in creative
    /// mode the creative inventory `InventoryScreen.init` swaps in.
    pub fn open_player_inventory(&mut self) {
        if self.instabuild() {
            self.set_screen(Screen::Creative);
            self.refresh_creative_tabs();
            self.client.creative_screen.0.reopen();
            self.refresh_saved_hotbars();
        } else {
            self.set_screen(Screen::Inventory);
        }
        self.capture(false);
    }
    /// A key press while a world is open (keyboard or control socket). `key`
    /// is the vanilla default the pressed mapping translates to, `raw` the
    /// physical key as pressed (the same where the caller had no window key).
    /// The input dispatcher runs a receiving screen's key actions separately.
    pub fn key_pressed(&mut self, key: KeyCode, raw: KeyCode) -> Result<()> {
        // Scripted presses and controller presses are delivered to the same mappings
        // as window input; direct screen keys remain immediate.
        if !self.mod_mappings_screen_open() && !self.client.replay.playing() {
            let key = if key == KeyCode::Enter {
                KeyCode::KeyT
            } else {
                key
            };
            if let Some(slot) = VanillaMappingSlot::by_default_key(key)
                && Self::tick_mapping(slot.definition().0)
            {
                self.client.vanilla_mappings.click(slot);
                return Ok(());
            }
        }
        self.immediate_key_pressed(key, raw)
    }

    pub(in crate::game::client_side) fn tick_mapping(name: &str) -> bool {
        matches!(
            name,
            "key.togglePerspective"
                | "key.smoothCamera"
                | "key.toggleSpectatorShaderEffects"
                | "key.inventory"
                | "key.quickActions"
                | "key.swapOffhand"
                | "key.drop"
        ) || GUI_KEYS.contains(&name)
            || name.starts_with("key.hotbar.")
    }

    pub(in crate::game::client_side) fn immediate_key_pressed(
        &mut self,
        key: KeyCode,
        raw: KeyCode,
    ) -> Result<()> {
        self.direct_craft_stop();
        if self.client.screen == Screen::Playing && self.client.replay.playing() {
            self.studio_key(key);
            return Ok(());
        }
        // `CreativeModeInventoryScreen.keyPressed` clears `ignoreTextInput` on
        // every key, before it decides anything.
        self.client.creative_ignore_text = false;
        if self.common.sleeping.is_some()
            && self.client.screen == Screen::Playing
            && !matches!(
                key,
                KeyCode::F1 | KeyCode::F2 | KeyCode::F11 | KeyCode::KeyT | KeyCode::Slash
            )
        {
            if matches!(key, KeyCode::Escape | KeyCode::ShiftLeft) {
                self.remote_stop_sleeping();
            }
            return Ok(());
        }
        if sunrise_client::keybinds::key_name(key)
            .is_some_and(|name| self.hud_editor_key_press(name) || self.shader_pack_key_press(name))
        {
            return Ok(());
        }
        // Escape cancels a press on the item panel; one that is a drag takes
        // the Escape, and the screen stays open.
        if key == KeyCode::Escape && self.mod_browser_drag_escape() {
            return Ok(());
        }
        if key == KeyCode::Escape && self.mod_browser_back() {
            return Ok(());
        }
        match key {
            KeyCode::Escape => match self.client.screen {
                Screen::Options => {
                    self.options_key(KeyCode::Escape, None)?;
                }
                Screen::Playing => {
                    if self.client.replay.playing() && self.client.captured {
                        self.capture(false);
                        return Ok(());
                    }
                    self.client.pause_focus = None;
                    self.set_screen(Screen::Paused);
                    self.client.pause_menu = true;
                    self.capture(false);
                }
                Screen::Creative => {
                    self.pad_clear_focus();
                    self.mod_browser_closed();
                    self.common.carried = None;
                    self.set_screen_with(Screen::Playing, |g| g.close_creative_menu());
                    self.capture(true);
                }
                Screen::Paused | Screen::Inventory => {
                    // RecipeBookComponent.keyPressed: a narrow window's open
                    // book takes Escape to close itself, not the screen.
                    if self.client.screen == Screen::Inventory && self.recipe_book_escape() {
                        return Ok(());
                    }
                    // AbstractContainerScreen.onClose closes the menu (on a server, with
                    // a ServerboundContainerClosePacket); PauseScreen.onClose only
                    // swaps the screen.
                    if self.client.screen == Screen::Inventory {
                        self.close_container_client();
                    } else {
                        self.pad_clear_focus();
                    }
                    self.client.pause_focus = None;
                    self.set_screen(Screen::Playing);
                    self.client.pause_menu = true;
                    self.capture(true);
                }
                Screen::GameModeSwitcher => self.close_gamemode_switcher(false),
                Screen::Dead => {
                    if self.client.chat_ui.death_confirm.is_some() {
                        self.death_confirm_answer(false)?;
                    }
                }
                Screen::Stats | Screen::Friends => self.set_screen(Screen::Paused),
                Screen::Social => self.close_social(),
                Screen::ServerLinks => self.set_screen(Screen::Paused),
                // ConfirmScreen.keyPressed: Escape answers no; both return to
                // the Server Links screen they came from.
                Screen::ConfirmLink => self.close_confirm_link(),
                Screen::DemoIntro => self.close_demo_intro(),
                Screen::DialogWarning => self.set_screen(Screen::ServerLinks),
                Screen::Advancements => self.close_advancements(),
                Screen::WorldOptions => self.world_options_escape(),
                Screen::CommandBlock => self.close_command_block(),
                Screen::BlockEditor => self.close_block_editor(),
                Screen::Chat => self.close_chat(chat::ChatExit::Intentional),
                Screen::Win => self.close_win_screen()?,
                Screen::DebugOptions => self.close_debug_options(),
                Screen::Mod => self.close_mod_screen(),
                Screen::HudEditor => self.close_hud_editor(),
            },
            KeyCode::F11 => self.toggle_fullscreen(),
            KeyCode::KeyE
                if self.client.screen == Screen::Creative
                    && self.client.creative_screen.0.tab().search => {}
            KeyCode::KeyE => match self.client.screen {
                Screen::Creative | Screen::Inventory => self.close_inventory_key(),
                _ => {}
            },
            _ if self.client.screen == Screen::Paused && self.client.pause_menu => {
                let _ = self.pause_key(key)?;
            }
            _ if self.client.screen == Screen::Dead => {
                let _ = self.death_key(key)?;
            }
            _ if self.client.screen == Screen::Stats => self.stats_key(raw),
            _ => {}
        }
        Ok(())
    }
    /// Minecraft.handleKeybinds and Gui.handleKeybinds, in their polling order.
    /// Opening a screen releases the live mappings, including clicks not yet reached.
    pub fn handle_play_keybinds(&mut self) {
        while self.consume_mapping_click("key.togglePerspective") {
            self.cycle_camera();
        }
        while self.consume_mapping_click("key.smoothCamera") {
            self.smooth_camera_action();
        }
        for name in GUI_KEYS {
            if name == "key.command" && self.mod_mappings_screen_open() {
                break;
            }
            while self.consume_mapping_click(name) {
                match name {
                    "key.toggleGui" => self.toggle_hide_gui(),
                    "key.advancements" => self.open_advancements(false),
                    "key.socialInteractions" if self.is_multiplayer_server() => {
                        self.hide_social_tutorial();
                        self.open_social(false);
                    }
                    "key.socialInteractions" => {
                        let message = sunrise_game_data::text::Text::translate(
                            "multiplayer.socialInteractions.not_available",
                            &[],
                        );
                        self.client.titles.set_overlay(message.clone(), false);
                        self.say_system_now(&message);
                    }
                    "key.chat" => self.open_chat_method(""),
                    "key.command" => self.open_chat_method("/"),
                    _ => unreachable!("listed GUI key"),
                }
            }
        }
        while self.consume_mapping_click("key.toggleSpectatorShaderEffects") {
            self.toggle_spectator_post_effect();
        }
        self.select_queued_hotbar();
        while self.consume_mapping_click("key.inventory") {
            self.open_inventory_key_action();
        }
        while self.consume_mapping_click("key.quickActions") {
            self.open_quick_actions();
        }
        while self.consume_mapping_click("key.swapOffhand") {
            if !self.own_player_is_spectator() && self.client.remote.is_some() {
                self.remote_swap_hands();
            }
        }
        while self.consume_mapping_click("key.drop") {
            if !self.own_player_is_spectator() && self.client.remote.is_some() {
                let all = self.has_control_down() || self.input_takes_stack_drop();
                self.remote_drop(all);
            }
        }
    }

    /// The inventory key either asks the vehicle's server menu or opens the local inventory.
    pub(in crate::game::client_side) fn open_inventory_key_action(&mut self) {
        if self.in_portal_transition() {
            return;
        }
        if self.client.remote.is_some()
            && (self.riding_mount_with_menu() || self.common.riding_chest_boat())
        {
            self.remote_request_mount_inventory();
        } else if self.client.remote.is_some() || !self.common.spectator() {
            if self.client.tutorial.open_inventory() {
                self.persist_tutorial_step();
            }
            self.client.recipe_ui.0.reset_screen();
            self.common.craft_size = 2;
            self.open_player_inventory();
        }
    }
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::integrated_common::IntegratedCommonGameExt1 as _;
