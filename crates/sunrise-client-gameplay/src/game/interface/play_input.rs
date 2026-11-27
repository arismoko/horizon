//! Key and mouse mappings in play: toggled sneak and sprint, keys after rebinding, keys held
//! across screens, the hotbar's wheel and bumper steps, and mouse buttons while playing.
use super::*;
use crate::game::client_side::key_codes::{gameplay_has_keyboard, glfw_modifiers, keyboard_button};

fn studio_replay_shortcut(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::Escape
            | KeyCode::KeyP
            | KeyCode::KeyF
            | KeyCode::KeyT
            | KeyCode::KeyM
            | KeyCode::Home
            | KeyCode::KeyL
            | KeyCode::KeyK
            | KeyCode::Delete
    )
}

fn studio_freecam_movement_key(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::KeyW
            | KeyCode::KeyA
            | KeyCode::KeyS
            | KeyCode::KeyD
            | KeyCode::KeyE
            | KeyCode::KeyQ
            | KeyCode::Space
            | KeyCode::ControlRight
    )
}

impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn toggle_options(&self) -> [bool; 4] {
        [
            self.client.settings.toggle_crouch,
            self.client.settings.toggle_sprint,
            self.client.settings.toggle_attack,
            self.client.settings.toggle_use,
        ]
    }
    pub(in crate::game::client_side) fn reset_toggle_keys(&mut self) {
        self.client.vanilla_mappings.reset_toggle_keys();
    }

    pub(in crate::game::client_side) fn smooth_camera_action(&mut self) {
        self.client.smooth_camera = !self.client.smooth_camera;
    }

    pub fn held_mapping(&self, action: sunrise_client::actions::Action) -> bool {
        self.mapped_key_down(action.name())
    }

    pub fn press_device_key(&mut self, key: KeyCode, source: Source) -> bool {
        let edge = self.client.keys.press(key, source);
        self.sync_device_key(key, true);
        edge
    }

    pub(in crate::game) fn release_device_key(&mut self, key: &KeyCode, source: Source) -> bool {
        let up = self.client.keys.release(key, source);
        self.sync_device_key(*key, !up);
        up
    }

    pub fn insert_device_key(&mut self, key: KeyCode) -> bool {
        let edge = self.client.keys.insert(key);
        self.sync_device_key(key, true);
        edge
    }

    pub fn remove_device_key(&mut self, key: &KeyCode) -> bool {
        let held = self.client.keys.remove(key);
        self.sync_device_key(*key, false);
        held
    }

    pub fn clear_device_keys(&mut self) {
        self.client.keys.clear();
        let options = self.toggle_options();
        self.client
            .vanilla_mappings
            .set_keys_up_on_focus_loss(options);
    }

    fn sync_device_key(&mut self, key: KeyCode, down: bool) {
        if let Some(slot) = sunrise_client::keybinds::VanillaMappingSlot::by_default_key(key) {
            self.client.vanilla_mappings.state_mut(slot).is_down = down;
        }
    }

    pub fn update_modifiers(&mut self, state: winit::keyboard::ModifiersState) {
        self.client.modifiers = state;
        if let Some(title) = &mut self.client.title.0 {
            title.set_alt_down(state.alt_key());
        }
        let released = sunrise_client_input::modifiers::reconcile(
            self.client.modifiers,
            &mut self.client.physical_keys,
            &self.client.settings.keys,
            &mut self.client.vanilla_mappings,
            [
                self.client.settings.toggle_crouch,
                self.client.settings.toggle_sprint,
            ],
            [
                self.client.pad.0.holds_action("key.sneak"),
                self.client.pad.0.holds_action("key.sprint"),
            ],
        );
        for (slot, key) in [(SNEAK, KeyCode::ShiftLeft), (SPRINT, KeyCode::ControlLeft)] {
            if released[slot] {
                self.client.keys.resume_device_input(&key);
            }
        }
    }
    /// Let a held sneak, sprint or modifier key go when the keyboard reports
    /// its modifier up. A release does not always reach the mapping: a screen
    /// reads presses only, and an unfocused window is given no release at all,
    /// so the latch would keep the player sneaking or sprinting until the key
    /// is tapped again. The modifier state is what the keyboard itself
    /// asserts, so it settles the question. A mapping that toggles, or one a
    /// pad latched, keeps its state: `set_down(false, needs_toggle)` leaves
    /// those alone, and a toggle the player asked for is meant to outlive the
    /// key. A mapping rebound away from its modifier is left alone too: the
    /// modifier says nothing about it, and so is one a controller holds.
    ///
    /// Only `physical_keys` is cleared directly here; the logical `self.keys`
    /// entries for sneak and sprint are left to the toggle loop below, because
    /// they stand for the mapping and not the physical Shift or Control.
    /// `Screen.hasShiftDown` and `hasControlDown` read the physical state
    /// (vanilla's `InputConstants.isKeyDown`), so a Shift no one is holding
    /// makes every inventory click a shift-click and every throw a whole-stack
    /// drop. Those do not care what sneak and sprint are bound to. A modifier
    /// says nothing about which side is down, so one being up means both
    /// Shifts, or both Controls, are.
    pub fn heal_hold_latches(&mut self) {
        self.update_modifiers(self.client.modifiers);
    }
    /// Whether the other device still holds the sneak or sprint mapping `source` released.
    pub(in crate::game::client_side) fn toggle_held_elsewhere(
        &self,
        slot: usize,
        key: KeyCode,
        source: Source,
    ) -> bool {
        match source {
            // A pad's toggle-sneak isn't holding it: it has no release that would end it.
            Source::Keyboard => {
                !(slot == SNEAK && self.client.settings.controller.toggle_sneak)
                    && self
                        .client
                        .pad
                        .0
                        .holds_action(["key.sneak", "key.sprint"][slot])
            }
            Source::Pad => self.client.physical_keys.iter().any(|physical| {
                self.client
                    .settings
                    .keys
                    .translate_all(*physical)
                    .contains(&Ok(key))
            }),
            Source::Agent => false,
        }
    }
    /// Apply a physical key to every mapping bound to it.
    pub fn mapped_key_event(&mut self, key: KeyCode, pressed: bool, repeat: bool) -> Result<()> {
        self.mapped_key_event_from(key, pressed, repeat, Source::Keyboard)
    }
    /// Apply a physical key edge from `source` to every mapping bound to it, once per binding
    /// (`KeyboardHandler.keyPress` -> `KeyMapping.set` / `KeyMapping.click`).
    pub(in crate::game::client_side) fn mapped_key_event_from(
        &mut self,
        key: KeyCode,
        pressed: bool,
        repeat: bool,
        source: Source,
    ) -> Result<()> {
        let action = match (pressed, repeat) {
            (false, _) => KeyAction::Release,
            (_, false) => KeyAction::Press,
            (_, true) => KeyAction::Repeat,
        };
        if let Some(name) = sunrise_client::keybinds::key_name(key) {
            let screen_open = self.mod_mappings_screen_open() || self.client.replay.playing();
            self.client.vanilla_mappings.key_event(
                &self.client.settings.keys,
                name,
                action,
                screen_open,
            );
        }
        for mapped in self.client.settings.keys.translate_all(key) {
            match mapped {
                Ok(k) => {
                    self.mapped_key_dispatch(k, pressed, repeat, key, source, true)?;
                }
                Err(button) => {
                    if let Some(pressed) = keyboard_button(
                        gameplay_has_keyboard(
                            self.client.screen,
                            self.client.title.0.is_some(),
                            self.screen_over_playing(),
                        ),
                        action,
                    ) && button != MouseButton::Middle
                    {
                        self.mouse_mapping(button, pressed, source)?;
                    }
                }
            }
        }
        Ok(())
    }
    /// Apply a physical mouse button to every mapping bound to it.
    pub fn mapped_mouse_event(
        &mut self,
        button: MouseButton,
        input: &str,
        pressed: bool,
    ) -> Result<()> {
        self.mapped_mouse_event_from(button, input, pressed, Source::Keyboard)
    }
    /// Apply a physical mouse button edge from `source` to every mapping bound to `input`,
    /// once per binding (`MouseHandler.onButton` -> `KeyMapping.set` / `KeyMapping.click`).
    pub(in crate::game::client_side) fn mapped_mouse_event_from(
        &mut self,
        button: MouseButton,
        input: &str,
        pressed: bool,
        source: Source,
    ) -> Result<()> {
        let screen_open = self.mod_mappings_screen_open();
        self.client.vanilla_mappings.mouse_event(
            &self.client.settings.keys,
            input,
            pressed,
            screen_open,
        );
        for mapped in self.client.settings.keys.translate_mouse_all(button) {
            match mapped {
                Err(MouseButton::Middle) => {}
                Err(b) => self.mouse_mapping(b, pressed, source)?,
                Ok(k) => self.mapped_key_dispatch(k, pressed, false, k, source, true)?,
            }
        }
        Ok(())
    }
    /// A key event after rebinding, as the default key of its mapping (the
    /// keyboard and controllers both end here). `raw` is the physical key
    /// the event started from (`key` where the caller started from no window
    /// key): the screens act in that keyspace (`Screen.keyPressed`), where
    /// `key`, the vanilla default the mapping translated to, may itself be
    /// another mapping's rebound key. `source` is the device the event came
    /// from: a key stays held while any device holds it.
    pub fn mapped_key(
        &mut self,
        key: KeyCode,
        pressed: bool,
        repeat: bool,
        raw: KeyCode,
        source: Source,
    ) -> Result<()> {
        self.mapped_key_dispatch(
            key,
            pressed,
            repeat,
            raw,
            source,
            source == Source::Keyboard,
        )
    }
    fn mapped_key_dispatch(
        &mut self,
        key: KeyCode,
        pressed: bool,
        repeat: bool,
        raw: KeyCode,
        source: Source,
        vanilla_queued: bool,
    ) -> Result<()> {
        if self.client.screen == Screen::Playing && self.client.replay.playing() {
            if !pressed {
                self.release_device_key(&key, source);
                return Ok(());
            }
            if studio_replay_shortcut(key) {
                if !repeat {
                    self.key_pressed(key, raw)?;
                }
                return Ok(());
            }
            if !studio_freecam_movement_key(key) || !self.client.studio.flying() {
                return Ok(());
            }
        }
        // Sneak and sprint are ToggleKeyMappings: a press (or a key
        // repeat) flips them when toggling; only releases reach them
        // behind a screen (KeyboardHandler.keyPress).
        let toggle = match key {
            KeyCode::ShiftLeft => Some((0, self.client.settings.toggle_crouch)),
            KeyCode::ControlLeft => Some((1, self.client.settings.toggle_sprint)),
            _ => None,
        };
        if let Some((slot, option)) = toggle {
            let action = match (pressed, repeat) {
                (false, _) => KeyAction::Release,
                (_, false) => KeyAction::Press,
                (_, true) => KeyAction::Repeat,
            };
            self.client.keys.resume_device_input(&key);
            // One device letting go doesn't let go for the other still holding it.
            if !pressed && self.toggle_held_elsewhere(slot, key, source) {
                return Ok(());
            }
            let screen_open = !gameplay_has_keyboard(
                self.client.screen,
                self.client.title.0.is_some(),
                self.screen_over_playing(),
            );
            self.client
                .vanilla_mappings
                .toggle_state_mut(slot)
                .key(action, screen_open, option);
            if !pressed {
                return Ok(());
            }
        } else if !pressed {
            self.release_device_key(&key, source);
            return Ok(());
        } else if !self.mod_mappings_screen_open() || self.client.replay.playing() {
            self.press_device_key(key, source);
            if let Some(slot) = sunrise_client::keybinds::VanillaMappingSlot::by_default_key(key)
                && slot.definition().2 == "movement"
            {
                self.client.vanilla_mappings.click(slot);
            }
        }
        // The window and mapped-input paths already queued their mapping clicks by resolved slot.
        // Screen keys still see the immediately delivered input; direct controller edges queue here.
        if vanilla_queued {
            self.immediate_key_pressed(key, raw)
        } else if !repeat || key == KeyCode::KeyQ {
            self.key_pressed(key, raw)
        } else {
            Ok(())
        }
    }
    /// `KeyMapping.releaseAll` when a screen opens and
    /// `restoreToggleStatesOnScreenClosed` when it closes.
    pub(in crate::game::client_side) fn track_toggle_screens(&mut self) {
        let playing = !self.mod_mappings_screen_open();
        if playing == self.client.was_playing {
            return;
        }
        self.client.was_playing = playing;
        if playing {
            let options = self.toggle_options();
            self.client
                .vanilla_mappings
                .restore_toggles_on_screen_closed(options);
        } else {
            self.release_screen_mappings();
        }
    }
    /// Immediately release gameplay mappings when a screen opens, even if it closes
    /// again before the redraw's screen tracking.
    pub(in crate::game::client_side) fn release_screen_mappings(&mut self) {
        self.client.was_playing = false;
        self.client.keys.clear();
        let options = self.toggle_options();
        self.client.vanilla_mappings.release_all(options);
    }
    /// `MouseHandler.onScroll`: the hotbar (or the spectator menu) takes the wheel only when no
    /// screen is open; a book, lectern, sign or prompt drawn over gameplay is one.
    pub(in crate::game::client_side) fn hotbar_takes_wheel(&self) -> bool {
        self.client.screen == Screen::Playing
            && !self.screen_over_playing()
            && self.common.sleeping.is_none()
            && self.common.player.is_some()
    }
    /// A mod's `MouseScrolling` cancels the step about to turn the hotbar.
    pub(in crate::game::client_side) fn hotbar_scroll_canceled(
        &mut self,
        scroll: [f64; 2],
        steps: [i32; 2],
    ) -> bool {
        let mut canceled = false;
        self.native_mouse_scrolling(
            scroll,
            steps,
            [
                self.client.physical_buttons.contains(&MouseButton::Left),
                self.client.physical_buttons.contains(&MouseButton::Middle),
                self.client.physical_buttons.contains(&MouseButton::Right),
            ],
            [self.client.cursor.0, self.client.cursor.1],
            glfw_modifiers(self.client.modifiers),
            &mut canceled,
        );
        canceled
    }
    /// What one whole wheel step does in play (`MouseHandler.onScroll`): a spectator's menu or
    /// flying speed, else the selected hotbar slot.
    pub(in crate::game::client_side) fn hotbar_wheel_step(
        &mut self,
        wheel: i32,
        vertical_steps: i32,
    ) {
        if self.own_player_is_spectator() && self.client.spectator_gui.is_menu_active() {
            self.spectator_scroll(-wheel);
        } else if self.own_player_is_spectator() {
            let speed = self.common.player.as_mut().map(|player| {
                player.flying_speed = sunrise_game_data::move_input::spectator_flying_speed(
                    player.flying_speed,
                    vertical_steps,
                );
                player.flying_speed
            });
            if let Some(speed) = speed {
                self.set_remote_flying_speed(speed);
            }
        } else {
            self.common.selected = sunrise_client::mouse::ScrollWheelHandler::next_selection(
                f64::from(wheel),
                self.common.selected as i32,
                9,
            ) as usize;
        }
    }
    /// A step of the hotbar from a controller's bumper: one wheel notch, down for the next slot,
    /// through everything the wheel goes through.
    pub(in crate::game::client_side) fn hotbar_step(&mut self, next: bool) {
        let wheel = if next { -1 } else { 1 };
        if self.hotbar_takes_wheel()
            && !self.hotbar_scroll_canceled([0.0, f64::from(wheel)], [0, wheel])
        {
            self.hotbar_wheel_step(wheel, wheel);
        }
    }
    /// A mouse mapping pressed or let go by `source`. A release waits while another device holds
    /// the mapping, so letting go of one can't end what the other still presses, as for keys
    /// (`HeldKeys`).
    pub fn mouse_mapping(
        &mut self,
        button: MouseButton,
        pressed: bool,
        source: Source,
    ) -> Result<()> {
        if self.mod_mappings_screen_open() {
            return Ok(());
        }
        if !pressed && self.mouse_mapping_held_elsewhere(button, source) {
            // The other device's hold takes over again, even after a direct setDown(false).
            // In toggle mode a release does not change the mapping.
            let toggled = match button {
                MouseButton::Left => self.client.settings.toggle_attack,
                MouseButton::Right => self.client.settings.toggle_use,
                _ => return Ok(()),
            };
            if !toggled {
                self.set_mapping_held(
                    if button == MouseButton::Left {
                        "key.attack"
                    } else {
                        "key.use"
                    },
                    true,
                );
            }
            return Ok(());
        }
        self.play_mouse(button, pressed)
    }
    /// Whether a device other than `source` holds what `button` stands for. Asks each device's own
    /// live state rather than keeping a count: a screen takes a mouse release without telling the
    /// mappings, and a count would then hold the attack for good. A script keeps no such state, so
    /// nothing waits for a script's mouse hold.
    pub(in crate::game::client_side) fn mouse_mapping_held_elsewhere(
        &self,
        button: MouseButton,
        source: Source,
    ) -> bool {
        let mapping = match button {
            MouseButton::Left => "key.attack",
            MouseButton::Right => "key.use",
            _ => return false,
        };
        match source {
            Source::Keyboard | Source::Agent => self.input_holds(mapping),
            Source::Pad => {
                let keys = &self.client.settings.keys;
                self.client
                    .physical_buttons
                    .iter()
                    .any(|held| keys.translate_mouse_all(*held).contains(&Err(button)))
                    || self
                        .client
                        .physical_keys
                        .iter()
                        .any(|held| keys.translate_all(*held).contains(&Err(button)))
            }
        }
    }
    /// A translated mouse mapping (`Left` = attack, `Right` = use, `Middle` = middle pair) while playing.
    pub fn play_mouse(&mut self, button: MouseButton, pressed: bool) -> Result<()> {
        if pressed {
            self.pvp_click(button);
        }
        if self.common.sleeping.is_some() {
            return Ok(());
        }
        let names: &[&str] = match button {
            MouseButton::Left => &["key.attack"],
            MouseButton::Right => &["key.use"],
            MouseButton::Middle => &["key.pickItem", "key.spectatorHotbar"],
            _ => return Ok(()),
        };
        for name in names {
            let slot =
                sunrise_client::keybinds::VanillaMappingSlot::by_name(name).expect("known mapping");
            let enabled = sunrise_client::keymapping::toggle_index(slot)
                .is_some_and(|i| self.toggle_options()[i]);
            self.client
                .vanilla_mappings
                .state_mut(slot)
                .button(pressed, false, enabled);
        }
        Ok(())
    }
}

impl crate::game::ClientGame<'_> {
    /// Asks the mods about each gameplay mapping a physical input (a key's or a mouse button's
    /// name) is bound to, before the game acts on it: true when one cancels a press. Only while
    /// playing, and nothing is asked with no mod listening.
    pub fn key_mapping_canceled(&mut self, input: Option<&str>, pressed: bool) -> bool {
        let Some(input) = input else {
            return false;
        };
        if !mod_api::__private::listened(mod_api::HookId::KeyMappingPre)
            || !gameplay_has_keyboard(
                self.client.screen,
                self.client.title.0.is_some(),
                self.dialog_open(),
            )
        {
            return false;
        }
        let names: Vec<&'static str> = self
            .client
            .settings
            .keys
            .translated_mappings(input)
            .collect();
        let mut canceled = false;
        for name in names {
            let mut this = false;
            self.native_key_mapping_pre(name, pressed, &mut this);
            canceled |= this;
        }
        canceled && pressed
    }
    /// `key_mapping_canceled` for a key event, and what a cancel leaves of the key's hold: a first
    /// press that is cancelled never happened, so `restore_held_keys` must not hold it (nor its
    /// repeats); a cancelled repeat only drops its own action and keeps the hold an accepted
    /// press made.
    pub fn key_press_canceled(&mut self, key: KeyCode, pressed: bool, repeat: bool) -> bool {
        if !repeat {
            self.client.rejected_keys.remove(&key);
        }
        let name = sunrise_client::keybinds::key_name(key);
        if !self.key_mapping_canceled(name, pressed) {
            return false;
        }
        if !repeat {
            self.client.rejected_keys.insert(key);
        }
        if self.client.rejected_keys.contains(&key) {
            self.client.physical_keys.remove(&key);
        }
        true
    }
}

/// The input source (docs/layers.md, "Controllers, replays and UI pages"): what
/// gameplay asks of a device beyond the keyboard and mouse, and tells it. Gameplay calls these,
/// never the controller modules; today `pad` answers.
impl crate::game::ClientGame<'_> {
    /// The analog move vector (`[left, forward]`), after direct mapping writes.
    pub(in crate::game::client_side) fn input_move(&self) -> Option<[f32; 2]> {
        self.pad_move()
            .map(|stick| self.apply_movement_writes(stick))
    }

    fn apply_movement_writes(&self, [left, forward]: [f32; 2]) -> [f32; 2] {
        let axis = |value: f32, positive, negative| {
            let component = |key, value| match self.client.keys.override_for(&key) {
                Some(true) => 1.0,
                Some(false) => 0.0,
                None => value,
            };
            component(positive, value.max(0.0)) - component(negative, (-value).max(0.0))
        };
        [
            axis(left, KeyCode::KeyA, KeyCode::KeyD),
            axis(forward, KeyCode::KeyW, KeyCode::KeyS),
        ]
    }

    /// KeyboardInput.tick: classify the original stick first; a direct write changes only
    /// its own flag and analog component, never a different direction's classification.
    pub fn sample_movement_keys(&self, stick: Option<[f32; 2]>) -> sunrise_sim::sprint::Keys {
        use sunrise_sim::sprint::Keys;
        let mut keys = Keys {
            forward: self.mapped_key_down("key.forward"),
            backward: self.mapped_key_down("key.back"),
            left: self.mapped_key_down("key.left"),
            right: self.mapped_key_down("key.right"),
            jump: self.mapped_key_down("key.jump"),
            shift: self.mapped_key_down("key.sneak"),
            sprint: self.mapped_key_down("key.sprint"),
            ..Keys::default()
        }
        .with_stick(stick);
        for (down, key) in [
            (&mut keys.forward, KeyCode::KeyW),
            (&mut keys.backward, KeyCode::KeyS),
            (&mut keys.left, KeyCode::KeyA),
            (&mut keys.right, KeyCode::KeyD),
        ] {
            if let Some(value) = self.client.keys.override_for(&key) {
                *down = value;
            }
        }
        keys.analog = stick.map(|stick| self.apply_movement_writes(stick));
        keys
    }

    /// Whether a device holds the key mapping named `mapping` (`key.use`).
    pub(in crate::game::client_side) fn input_holds(&self, mapping: &str) -> bool {
        self.client.pad.0.holds_action(mapping)
    }

    /// Turns the view by what the devices give this frame (`Entity.turn`).
    pub(in crate::game::client_side) fn input_look(&mut self, seconds: f32) {
        self.pad_look(seconds);
    }

    /// The keyboard or mouse acted in play (a key or button pressed): it has control again, though
    /// only moving the mouse takes a pad out of use.
    pub(in crate::game::client_side) fn input_keyboard_used(&mut self) {
        self.client.pad.0.keyboard_took_over = true;
    }

    /// Whether the pointer event being handled is one a device made (a pad's menu cursor
    /// clicking), not the mouse's.
    pub(in crate::game::client_side) fn input_made_pointer_event(&self) -> bool {
        self.client.pad.0.menu.synthetic
    }

    /// A new player entity took over (spawn, respawn, a server joined, a dimension change).
    pub(in crate::game::client_side) fn input_player_replaced(&mut self) {
        self.pad_player_replaced();
    }

    /// Whether the drop about to happen is a device's whole-stack drop (it has no Control key to
    /// hold). Asking answers once.
    pub(in crate::game::client_side) fn input_takes_stack_drop(&mut self) -> bool {
        std::mem::take(&mut self.client.pad.0.drop_stack)
    }

    /// The HUD starts a frame: what a device drew on the last one is gone.
    pub(in crate::game::client_side) fn input_hud_begin(&mut self) {
        self.client.pad.0.guide_shown.clear();
    }

    /// Something happened to the player that a device may let them feel.
    pub(in crate::game::client_side) fn input_feedback(
        &mut self,
        event: sunrise_client::input_source::Feedback,
    ) {
        self.pad_rumble(event.into());
    }

    /// Whether `key` is held, a movement key by the analog move vector too: as the keys it holds
    /// (`eight_direction_keys`), whatever moves the player. For readers of the keys that aren't
    /// the movement itself.
    pub fn movement_key_held(&self, key: KeyCode) -> bool {
        if let Some(down) = self.client.keys.override_for(&key) {
            return down;
        }
        if sunrise_client::keybinds::VanillaMappingSlot::by_default_key(key)
            .is_some_and(|slot| self.client.vanilla_mappings.is_down(slot))
        {
            return true;
        }
        let Some(stick) = self.pad_move() else {
            return false;
        };
        let [forward, backward, left, right] = sunrise_client::gamepad::eight_direction_keys(stick);
        match key {
            KeyCode::KeyW => forward,
            KeyCode::KeyS => backward,
            KeyCode::KeyA => left,
            KeyCode::KeyD => right,
            _ => false,
        }
    }
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::{studio_freecam_movement_key, studio_replay_shortcut};
    use winit::keyboard::KeyCode;

    #[test]
    fn replay_shortcuts_are_distinct_from_freecam_movement() {
        for key in [
            KeyCode::Escape,
            KeyCode::KeyP,
            KeyCode::KeyF,
            KeyCode::KeyT,
            KeyCode::KeyM,
            KeyCode::Home,
            KeyCode::KeyL,
            KeyCode::KeyK,
            KeyCode::Delete,
        ] {
            assert!(studio_replay_shortcut(key));
        }
        for key in [
            KeyCode::KeyW,
            KeyCode::KeyA,
            KeyCode::KeyS,
            KeyCode::KeyD,
            KeyCode::KeyE,
            KeyCode::KeyQ,
        ] {
            assert!(studio_freecam_movement_key(key));
            assert!(!studio_replay_shortcut(key));
        }
        assert!(!studio_freecam_movement_key(KeyCode::KeyI));
    }
}
