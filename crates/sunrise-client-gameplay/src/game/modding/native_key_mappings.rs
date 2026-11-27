//! The keys mods register (`Registrar::register_key_mapping`) as plain `KeyMapping`s: held while
//! their binding is down and counting its presses for `consume_click`. Keys reach them on one
//! route with `input_key`, from the window and the control socket alike; mouse buttons from the
//! window and the control socket's `mouse`.
use super::key_codes::{gameplay_has_keyboard, glfw_modifiers};
use crate::game::ClientGame;
use mod_api::{ClientModifiers, KeyConflictContext, MappingError, ModKeyMappingRef, ModKeyRefusal};
use sunrise_client::keymapping::{KeyAction, MappingContext, ModBinding, ModKeySlot};
use sunrise_game_data::text::Text;
use sunrise_mod_package::mod_list::{ALT, CONTROL, SHIFT};
use winit::{event::MouseButton, keyboard::KeyCode};

fn mod_key_slot(mapping: ModKeyMappingRef) -> ModKeySlot {
    let (generation, index) = mod_api::__private::mod_key_mapping_parts(mapping);
    ModKeySlot::from_parts(generation, index)
}

impl ClientGame<'_> {
    /// Binds the mods' mappings to the player's keys (`sunrise-mods.txt`) or their declared
    /// defaults: when the settings apply, at start and after Controls changes one.
    pub(in crate::game::client_side) fn bind_mod_mappings(&mut self) {
        let mappings = super::mods::collect_key_mappings(
            self.common.options.execution_role,
            mod_api::__private::key_mappings(),
        );
        let settings = &self.client.settings;
        self.client
            .mod_keys
            .bind(mappings.iter().map(|(owner, mapping)| {
                let (input, modifiers) = settings.mods.binding(
                    owner,
                    &mapping.id,
                    (&mapping.key, mapping.modifiers.bits),
                );
                ModBinding {
                    owner,
                    id: &mapping.id,
                    input,
                    modifiers,
                    context: match mapping.context {
                        KeyConflictContext::InGame => MappingContext::InGame,
                        KeyConflictContext::Gui => MappingContext::Gui,
                        _ => MappingContext::Universal,
                    },
                }
            }));
    }
    pub fn mod_key_mapping(
        &self,
        owner: &str,
        id: &str,
    ) -> Result<ModKeyMappingRef, ModKeyRefusal> {
        if !self.common.options.execution_role.is_client() {
            return Err(ModKeyRefusal::Unavailable);
        }
        let slot = self
            .client
            .mod_keys
            .resolve(owner, id)
            .ok_or(ModKeyRefusal::NotRegistered)?;
        let (generation, index) = slot.parts();
        Ok(mod_api::__private::mod_key_mapping_ref(generation, index))
    }

    pub(in crate::game) fn mod_key_mapping_binding(
        &self,
        mapping: ModKeyMappingRef,
    ) -> Result<(&str, ClientModifiers), ModKeyRefusal> {
        let row = self
            .client
            .mod_keys
            .get(mod_key_slot(mapping))
            .ok_or(ModKeyRefusal::Stale)?;
        let (key, modifiers) = row.binding();
        Ok((key, ClientModifiers::new(modifiers)))
    }

    pub(in crate::game) fn mod_key_mapping_translated_key_message(
        &self,
        mapping: ModKeyMappingRef,
    ) -> Result<Text, MappingError> {
        let row = self
            .client
            .mod_keys
            .get(mod_key_slot(mapping))
            .ok_or(MappingError::Stale)?;
        let (key, modifiers) = row.binding();
        Ok(sunrise_client::options_ui::binding_text(key, modifiers))
    }

    /// Binds the mapping to `key` with `modifiers`, as Controls does (`OptionsUi::bind_key`):
    /// the mappings are bound again and the settings saved.
    pub(in crate::game) fn mod_key_mapping_set_binding(
        &mut self,
        mapping: ModKeyMappingRef,
        key: &str,
        modifiers: ClientModifiers,
    ) -> Result<(), ModKeyRefusal> {
        let (owner, id) = self
            .client
            .mod_keys
            .get(mod_key_slot(mapping))
            .ok_or(ModKeyRefusal::Stale)?
            .identity();
        let known = key == "key.keyboard.unknown"
            || sunrise_client::keybinds::key_code(key).is_some()
            || sunrise_client::keybinds::mouse_button(key).is_some();
        if !known {
            return Err(ModKeyRefusal::UnknownKey);
        }
        if modifiers.bits & !(SHIFT | CONTROL | ALT) != 0 {
            return Err(ModKeyRefusal::UnknownModifiers);
        }
        // A modifier key bound on its own is the key, not a modifier held with itself.
        let own = match key {
            "key.keyboard.left.shift" | "key.keyboard.right.shift" => SHIFT,
            "key.keyboard.left.control" | "key.keyboard.right.control" => CONTROL,
            "key.keyboard.left.alt" | "key.keyboard.right.alt" => ALT,
            _ => 0,
        };
        self.client
            .settings
            .mods
            .set_binding(owner, id, key, modifiers.bits & !own);
        self.mod_keys_changed();
        Ok(())
    }
    /// Gives the mapping its declared default back, as Controls' Reset does.
    pub(in crate::game) fn mod_key_mapping_reset_binding(
        &mut self,
        mapping: ModKeyMappingRef,
    ) -> Result<(), ModKeyRefusal> {
        let (owner, id) = self
            .client
            .mod_keys
            .get(mod_key_slot(mapping))
            .ok_or(ModKeyRefusal::Stale)?
            .identity();
        self.client.settings.mods.reset_binding(owner, id);
        self.mod_keys_changed();
        Ok(())
    }
    /// After a mod changed a binding: the mappings answer to it from the next input, and the
    /// settings are saved as Controls saves them.
    fn mod_keys_changed(&mut self) {
        self.bind_mod_mappings();
        self.client
            .settings
            .save(&sunrise_gameplay_common::game::options_path());
    }
    /// One key event, the window's after it handled it or the control socket's: the mods'
    /// mappings bound to it hear it (`KeyMapping.set`, `KeyMapping.click`), then `input_key`
    /// fires. `key` is vanilla's name for it; the rest is what `native_input_key` takes.
    pub fn key_input(
        &mut self,
        key: Option<&'static str>,
        layout_key: Option<&str>,
        raw_code: u32,
        action: mod_api::KeyAction,
        modifiers: u32,
    ) {
        if let Some(name) = key {
            let action = match action {
                mod_api::KeyAction::Release => KeyAction::Release,
                mod_api::KeyAction::Repeat => KeyAction::Repeat,
                _ => KeyAction::Press,
            };
            self.mod_mappings_input(name, action, false);
        }
        let key = key.unwrap_or("key.keyboard.unknown");
        self.native_input_key(key, layout_key, raw_code, action, modifiers);
    }
    /// The control socket's `key KEY down|up|tap|repeat`, through `key_input` as a window key
    /// goes: a tap is a press, then a release. A script types on a QWERTY layout.
    pub(in crate::game::client_side) fn socket_key_input(&mut self, key: KeyCode, action: &str) {
        let name = sunrise_client::keybinds::key_name(key);
        let layout_key = match key {
            KeyCode::Backquote
            | KeyCode::Minus
            | KeyCode::Equal
            | KeyCode::BracketLeft
            | KeyCode::BracketRight
            | KeyCode::Backslash
            | KeyCode::IntlBackslash
            | KeyCode::Semicolon
            | KeyCode::Quote
            | KeyCode::Comma
            | KeyCode::Period
            | KeyCode::Slash => None,
            _ => name,
        };
        let raw_code =
            sunrise_client::keybinds::raw_key_code(winit::keyboard::PhysicalKey::Code(key));
        let actions: &[mod_api::KeyAction] = match action {
            "down" => &[mod_api::KeyAction::Press],
            "repeat" => &[mod_api::KeyAction::Repeat],
            "up" => &[mod_api::KeyAction::Release],
            _ => &[mod_api::KeyAction::Press, mod_api::KeyAction::Release],
        };
        for action in actions {
            let modifiers = glfw_modifiers(self.client.modifiers);
            self.key_input(name, layout_key, raw_code, *action, modifiers);
        }
    }
    /// The same as a key's for a mouse button (`MouseHandler.onButton`).
    pub(in crate::game::client_side) fn mod_mappings_button(
        &mut self,
        button: MouseButton,
        pressed: bool,
    ) {
        let action = if pressed {
            KeyAction::Press
        } else {
            KeyAction::Release
        };
        let screen_open = self.mod_mappings_screen_open();
        let held = glfw_modifiers(self.client.modifiers);
        self.client
            .mod_keys
            .button(button, action, screen_open, held);
    }
    fn mod_mappings_input(&mut self, input: &str, action: KeyAction, mouse: bool) {
        let screen_open = self.mod_mappings_screen_open();
        let held = glfw_modifiers(self.client.modifiers);
        self.client
            .mod_keys
            .input(input, action, mouse, screen_open, held);
    }
    /// A screen or overlay is open: the mappings hear no presses and are released.
    pub fn mod_mappings_screen_open(&self) -> bool {
        !gameplay_has_keyboard(
            self.client.screen,
            self.client.title.0.is_some(),
            self.screen_over_playing(),
        )
    }
    /// `KeyMapping.isDown`: held, while its conflict context and modifiers are active.
    pub(in crate::game) fn mod_key_mapping_is_down(
        &self,
        mapping: ModKeyMappingRef,
    ) -> Result<bool, ModKeyRefusal> {
        let screen_open = self.mod_mappings_screen_open();
        self.client
            .mod_keys
            .is_down(
                mod_key_slot(mapping),
                glfw_modifiers(self.client.modifiers),
                screen_open,
            )
            .ok_or(ModKeyRefusal::Stale)
    }
    /// `KeyMapping.consumeClick`: takes one pending click.
    pub(in crate::game) fn mod_key_mapping_consume_click(
        &mut self,
        mapping: ModKeyMappingRef,
    ) -> Result<bool, ModKeyRefusal> {
        self.client
            .mod_keys
            .consume_click(mod_key_slot(mapping))
            .ok_or(ModKeyRefusal::Stale)
    }
}
