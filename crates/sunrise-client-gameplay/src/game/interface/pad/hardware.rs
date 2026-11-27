//! The real controllers (the `controllers` feature): gilrs' pads, and the DualSense's own output
//! over HID: its rumble, adaptive triggers, lights and HD haptics. A build without the feature (the
//! server build) has the stub in pad.rs instead, and only `sunrise-ctl pad`'s virtual pad.
use super::{super::Screen, Connection, Pads};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use sunrise_client::controller_ui::Device;
use sunrise_client::gamepad::{self, Button, GlyphStyle, PadState, Rumble, TriggerFeel};

pub(in crate::game) mod face_buttons;
pub(in crate::game) mod hid_layout;
pub(in crate::game) mod sdl_row;
pub(in crate::game) mod trigger_effects;
pub(in crate::game) mod triggers;

/// The controllers gilrs sees and the DualSense's output.
#[derive(Default)]
pub(in crate::game) struct Devices {
    pub(in crate::game) gilrs: Option<gilrs::Gilrs>,
    pub(in crate::game) started: bool,
    /// The pads seen so far, so one already there when gilrs opens isn't news.
    pub(in crate::game) known: Vec<gilrs::GamepadId>,
    /// The pad last touched; only its input counts.
    pub(in crate::game) active: Option<gilrs::GamepadId>,
    /// Vibrations playing (an effect stops when dropped) and when they end.
    pub(in crate::game) rumbles: Vec<(gilrs::ff::Effect, Instant)>,
    /// The active pad is a DualSense.
    pub(in crate::game) dualsense: bool,
    /// Its own output over HID (rumble here; gilrs only reads it), started once this window
    /// has the pad in hand; dropped, it releases the pad.
    pub(in crate::game) output: Option<sunrise_dualsense::Link>,
    /// Its HD haptics, on USB: looked for once each time the output opens.
    pub(in crate::game) haptics: Option<sunrise_dualsense::haptics::Haptics>,
    pub(in crate::game) haptics_tried: bool,
    /// What it was last told to show.
    pub(in crate::game) shown: Option<sunrise_dualsense::device::State>,
}

/// `pad status`'s line for a controller. `uuid` is the id an SDL mapping row starts with, so a
/// report says which row a pad needs; `mapping_source` says whether one was found.
pub(in crate::game::client_side) fn controller_status(
    id: gilrs::GamepadId,
    pad: &gilrs::Gamepad,
    active: Option<gilrs::GamepadId>,
) -> Value {
    let mapping_source = match pad.mapping_source() {
        gilrs::MappingSource::SdlMappings => "sdl",
        gilrs::MappingSource::Driver => "driver",
        gilrs::MappingSource::None => "none",
    };
    let uuid: String = pad.uuid().iter().map(|b| format!("{b:02x}")).collect();
    json!({
        "name": pad.name(),
        "vendor": pad.vendor_id(),
        "product": pad.product_id(),
        "mapping_source": mapping_source,
        "uuid": uuid,
        "active": active == Some(id),
    })
}

impl Devices {
    /// The hardware part of `pad status`.
    pub(in crate::game::client_side) fn status(&self) -> Value {
        let pads: Vec<Value> = match &self.gilrs {
            Some(g) => g
                .gamepads()
                .map(|(id, p)| controller_status(id, &p, self.active))
                .collect(),
            None => Vec::new(),
        };
        json!({
            "backend": self.gilrs.is_some(),
            "controllers": pads,
            "dualsense_output": self.output.as_ref().and_then(|o| o.open()).map(|t| format!("{t:?}")),
            "trigger_effects": self.shown.map(|s| s.triggers),
        })
    }
}

pub(in crate::game::client_side) fn gilrs_button(button: Button) -> gilrs::Button {
    use gilrs::Button as G;
    match button {
        Button::South => G::South,
        Button::East => G::East,
        Button::West => G::West,
        Button::North => G::North,
        Button::LeftBumper => G::LeftTrigger,
        Button::RightBumper => G::RightTrigger,
        Button::LeftTrigger => G::LeftTrigger2,
        Button::RightTrigger => G::RightTrigger2,
        Button::Select => G::Select,
        Button::Start => G::Start,
        Button::LeftStick => G::LeftThumb,
        Button::RightStick => G::RightThumb,
        Button::DPadUp => G::DPadUp,
        Button::DPadDown => G::DPadDown,
        Button::DPadLeft => G::DPadLeft,
        Button::DPadRight => G::DPadRight,
    }
}

pub(in crate::game::client_side) fn is_dualsense(pad: &gilrs::Gamepad) -> bool {
    matches!((pad.vendor_id(), pad.product_id()),
        (Some(vendor), Some(product)) if sunrise_dualsense::is_dualsense(vendor, product))
}

/// SDL rows gilrs' bundled DB is missing (or only ships for Windows). Newer
/// than the crate's copy; `GilrsBuilder::add_mappings` prefers these. The
/// Ultimate 2's Linux/BT rows put the right stick on a2/a3 — without them
/// look never moves (#4343).
pub(in crate::game) const EXTRA_SDL_MAPPINGS: &str = concat!(
    "03000000c82d00001260000011010000,8BitDo Ultimate 2,a:b0,b:b1,back:b10,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b12,leftshoulder:b6,leftstick:b13,lefttrigger:a5,leftx:a0,lefty:a1,paddle1:b17,paddle2:b16,paddle3:b2,paddle4:b5,rightshoulder:b7,rightstick:b14,righttrigger:a4,rightx:a2,righty:a3,start:b11,x:b3,y:b4,platform:Linux,\n",
    "05000000c82d00001260000001000000,8BitDo Ultimate 2,a:b0,b:b1,back:b10,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b12,leftshoulder:b6,leftstick:b13,lefttrigger:a5,leftx:a0,lefty:a1,paddle1:b17,paddle2:b16,paddle3:b2,paddle4:b5,rightshoulder:b7,rightstick:b14,righttrigger:a4,rightx:a2,righty:a3,start:b11,x:b3,y:b4,platform:Linux,\n",
    "03000000c82d00000b31000014010000,8BitDo Ultimate 2,a:b0,b:b1,back:b6,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b8,leftshoulder:b4,leftstick:b9,lefttrigger:a2,leftx:a0,lefty:a1,rightshoulder:b5,rightstick:b10,righttrigger:a5,rightx:a3,righty:a4,start:b7,x:b2,y:b3,platform:Linux,\n",
);

/// A vibration on pad `id` through gilrs' force feedback, kept alive until its end. Platforms
/// without it (gilrs on macOS) skip it.
pub(in crate::game::client_side) fn play_pulse(
    gilrs: &mut gilrs::Gilrs,
    id: gilrs::GamepadId,
    pulse: gamepad::Pulse,
) -> Option<(gilrs::ff::Effect, Instant)> {
    if !gilrs
        .connected_gamepad(id)
        .is_some_and(|g| g.is_ff_supported())
    {
        return None;
    }
    use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Repeat, Replay, Ticks};
    let length = Ticks::from_ms(pulse.millis);
    let motor = |kind| BaseEffect {
        kind,
        scheduling: Replay {
            play_for: length,
            ..Default::default()
        },
        envelope: Default::default(),
    };
    let magnitude = |v: f32| (v.clamp(0.0, 1.0) * f32::from(u16::MAX)) as u16;
    let effect = EffectBuilder::new()
        .add_effect(motor(BaseEffectType::Strong {
            magnitude: magnitude(pulse.strong),
        }))
        .add_effect(motor(BaseEffectType::Weak {
            magnitude: magnitude(pulse.weak),
        }))
        .repeat(Repeat::For(length))
        .gamepads(&[id])
        .finish(gilrs);
    match effect.and_then(|e| e.play().map(|()| e)) {
        Ok(effect) => Some((
            effect,
            Instant::now() + Duration::from_millis(u64::from(pulse.millis) + 50),
        )),
        Err(e) => {
            log::debug!("Sunrise: controller vibration: {e}");
            None
        }
    }
}

/// The one way to open gilrs: each reader needs the extra rows.
pub(in crate::game::client_side) fn gilrs_builder() -> gilrs::GilrsBuilder {
    gilrs::GilrsBuilder::new().add_mappings(EXTRA_SDL_MAPPINGS)
}

impl Pads {
    /// Open gilrs on first use. A platform without it (or a Linux without
    /// udev) leaves controllers off; the game runs as before.
    pub(in crate::game::client_side) fn start(&mut self) {
        if self.devices.started {
            return;
        }
        self.devices.started = true;
        self.devices.gilrs = match gilrs_builder().build() {
            Ok(g) => {
                for (id, pad) in g.gamepads() {
                    log::info!("Controller connected: {}", pad.name());
                    self.devices.known.push(id);
                }
                Some(g)
            }
            Err(gilrs::Error::NotImplemented(_)) => None,
            Err(e) => {
                log::warn!("Sunrise: controllers are unavailable: {e}");
                None
            }
        };
    }

    /// Drain gilrs' events and read the active pad (opening gilrs the first time).
    pub(in crate::game::client_side) fn read(&mut self) -> PadState {
        self.start();
        let Some(gilrs) = self.devices.gilrs.as_mut() else {
            return PadState::default();
        };
        let active = self.devices.active;
        self.peaks = [[0.0; 2]; 2];
        while let Some(gilrs::Event { id, event, .. }) = gilrs.next_event() {
            use gilrs::EventType as E;
            // The furthest each stick of the active pad leant between the events, which a read
            // after them all misses (`Pads::peaks`).
            if matches!(event, E::AxisChanged(..)) && self.devices.active == Some(id) {
                keep_peaks(&mut self.peaks, sticks_of(&gilrs.gamepad(id)));
            }
            match event {
                E::Connected => {
                    log::info!("Controller connected: {}", gilrs.gamepad(id).name());
                    // Some backends report the controllers already there as they open.
                    if !self.devices.known.contains(&id) {
                        self.devices.known.push(id);
                        let pad = gilrs.gamepad(id);
                        let style = GlyphStyle::of_device(pad.vendor_id(), pad.product_id());
                        self.connections.push(Connection::Connected(style));
                    }
                    if let Some(output) = &self.devices.output {
                        output.scan();
                    }
                }
                E::Disconnected => {
                    log::info!("Controller disconnected: {}", gilrs.gamepad(id).name());
                    self.devices.known.retain(|known| *known != id);
                    self.connections.push(Connection::Disconnected {
                        active: self.devices.active == Some(id),
                    });
                    if self.devices.active == Some(id) {
                        self.devices.active = None;
                    }
                    if let Some(output) = &self.devices.output {
                        output.scan();
                    }
                }
                E::ButtonPressed(..) => self.devices.active = Some(id),
                E::AxisChanged(axis, value, _)
                    if axis_takes_over(axis, value, z_is_a_stick(&gilrs.gamepad(id))) =>
                {
                    self.devices.active = Some(id)
                }
                E::ButtonChanged(_, value, _) if value > TAKEOVER => self.devices.active = Some(id),
                _ => {}
            }
        }
        // Another pad, or none: its A going away isn't a release.
        if self.devices.active != active {
            self.menu.armed = None;
            self.radial.cancel();
            self.menu.stop_craft();
        }
        let Some(pad) = self
            .devices
            .active
            .and_then(|id| gilrs.connected_gamepad(id))
        else {
            // Before any pad is touched (the virtual pad playing), a DualSense still shows it.
            self.devices.dualsense = gilrs.gamepads().any(|(_, pad)| is_dualsense(&pad));
            self.device = gilrs.gamepads().next().map(|(_, pad)| device_of(&pad));
            return PadState::default();
        };
        self.device = Some(device_of(&pad));
        self.style = GlyphStyle::of_device(pad.vendor_id(), pad.product_id());
        self.devices.dualsense = is_dualsense(&pad);
        state_of(&pad)
    }
}

/// The furthest each stick leant: `sticks` (left, right) where they lean further than `peaks`.
pub(in crate::game::client_side) fn keep_peaks(peaks: &mut [[f32; 2]; 2], sticks: [[f32; 2]; 2]) {
    use sunrise_client_input::radial::stick_length;
    for (peak, stick) in peaks.iter_mut().zip(sticks) {
        if stick_length(stick) > stick_length(*peak) {
            *peak = stick;
        }
    }
}

/// How far a stick or trigger has to go for its pad to become the active one.
pub(in crate::game) const TAKEOVER: f32 = 0.5;

/// Whether this pad has a stick on Z, where most pads have triggers.
pub(in crate::game::client_side) fn z_is_a_stick(pad: &gilrs::Gamepad) -> bool {
    hid_layout::right_stick_on_z(pad)
        || (pad.mapping_source() != gilrs::MappingSource::SdlMappings
            && (pad.vendor_id(), pad.product_id())
                == (
                    Some(gamepad::BITDO_ULTIMATE2_VID_PID.0),
                    Some(gamepad::BITDO_ULTIMATE2_VID_PID.1),
                ))
}

/// Whether an axis this far along means someone is using its pad. Z and axes gilrs has no name for
/// can be triggers, which rest at -1, so only their pulled end counts: touching an idle pad's
/// trigger mustn't take over from the pad in use.
pub(in crate::game::client_side) fn axis_takes_over(
    axis: gilrs::Axis,
    value: f32,
    z_is_a_stick: bool,
) -> bool {
    use gilrs::Axis as A;
    match axis {
        A::LeftZ | A::RightZ if z_is_a_stick => value.abs() > TAKEOVER,
        A::LeftZ | A::RightZ | A::Unknown => value > TAKEOVER,
        _ => value.abs() > TAKEOVER,
    }
}

/// What the controller settings say about a pad.
pub(in crate::game::client_side) fn device_of(pad: &gilrs::Gamepad) -> Device {
    Device {
        name: pad.name().to_owned(),
        vendor: pad.vendor_id(),
        product: pad.product_id(),
    }
}

/// A pad's buttons, sticks and triggers now.
pub(in crate::game::client_side) fn state_of(pad: &gilrs::Gamepad) -> PadState {
    let mut state = PadState::default();
    let crossed = face_buttons::crossed(pad);
    for button in Button::ALL {
        if !matches!(button, Button::LeftTrigger | Button::RightTrigger) {
            let named = face_buttons::named(gilrs_button(button), crossed);
            state.set(button, pad.is_pressed(named));
        }
    }
    [state.left, state.right] = sticks_of(pad);
    state.triggers = triggers::triggers(pad);
    state
}

/// A pad's sticks now (left, right).
pub(in crate::game::client_side) fn sticks_of(pad: &gilrs::Gamepad) -> [[f32; 2]; 2] {
    let axis = |a| pad.value(a);
    use gilrs::Axis as A;
    let sdl_mapped = pad.mapping_source() == gilrs::MappingSource::SdlMappings;
    let (left, right) = gamepad::stick_axes(
        [axis(A::LeftStickX), axis(A::LeftStickY)],
        [axis(A::RightStickX), axis(A::RightStickY)],
        axis(A::LeftZ),
        pad.vendor_id(),
        pad.product_id(),
        sdl_mapped,
    );
    let (left, right) = sdl_row::sticks(pad, left, right);
    let right = if hid_layout::right_stick_on_z(pad) {
        hid_layout::right_stick([axis(A::LeftZ), axis(A::RightZ)])
    } else {
        right
    };
    [left, right]
}

impl crate::game::ClientGame<'_> {
    /// Controllers are turned off: dropping the DualSense's output frees its triggers.
    pub(in crate::game::client_side) fn pad_output_off(&mut self) {
        self.client.pad.0.devices.output = None;
        self.client.pad.0.devices.shown = None;
    }

    /// After each poll: the DualSense's output opens once this window has the pad in hand, then
    /// shows what the game is doing; vibrations that are over stop.
    pub(in crate::game::client_side) fn pad_output_frame(&mut self) {
        if self.client.pad.0.devices.dualsense
            && self.client.pad.0.in_use
            && self.pad_has_window()
            && self.client.pad.0.devices.output.is_none()
        {
            self.client.pad.0.devices.output = Some(sunrise_dualsense::Link::start());
        }
        self.pad_haptics();
        self.pad_show();
        let now = Instant::now();
        self.client
            .pad
            .0
            .devices
            .rumbles
            .retain(|(_, end)| *end > now);
    }

    /// The DualSense's HID output, while it is the active pad, open, and this window has focus
    /// (another game on the same pad keeps it otherwise).
    pub(in crate::game::client_side) fn pad_output(&self) -> Option<&sunrise_dualsense::Link> {
        let output = self.client.pad.0.devices.output.as_ref()?;
        (self.client.pad.0.devices.dualsense && self.pad_has_window() && output.open().is_some())
            .then_some(output)
    }

    /// What the DualSense shows: while playing, its triggers feel what their actions do; in
    /// menus, paused, unfocused or with another pad active, they are free.
    pub(in crate::game::client_side) fn pad_show(&mut self) {
        if self.client.pad.0.devices.output.is_none() {
            return;
        }
        let playing = self.client.pad.0.devices.dualsense
            && self.pad_has_window()
            && self.client.title.0.is_none()
            && self.client.screen == Screen::Playing
            && !self.screen_over_playing()
            && !self.common.pause;
        let (using, attacking) = if playing {
            self.trigger_feels()
        } else {
            (TriggerFeel::Free, TriggerFeel::Free)
        };
        // The lightbar follows the player; menus keep the last colour.
        let in_world = self.client.title.0.is_none();
        let lightbar = match self.common.player.as_mut().filter(|_| in_world) {
            Some(p) => {
                let max = p.attribute("minecraft:max_health").unwrap_or(20.0) as f32;
                let health = p.health.health() / max.max(1.0);
                self.client
                    .settings
                    .controller
                    .lightbar
                    .colour(health, self.common.dim.id())
            }
            None => self.client.pad.0.devices.shown.and_then(|s| s.lightbar),
        };
        let lights = self.client.settings.controller.lightbar != gamepad::Lightbar::Off;
        let state = sunrise_dualsense::device::State {
            triggers: trigger_effects::effects(&self.client.settings.controller, using, attacking),
            lightbar: lightbar.filter(|_| lights),
            player_leds: lights.then_some(sunrise_dualsense::PLAYER_1),
        };
        if self.client.pad.0.devices.shown != Some(state)
            && let Some(output) = &self.client.pad.0.devices.output
        {
            output.set(state);
            self.client.pad.0.devices.shown = Some(state);
        }
    }

    /// What `key.use` and `key.attack` are doing, for the triggers bound to them.
    pub(in crate::game::client_side) fn trigger_feels(&self) -> (TriggerFeel, TriggerFeel) {
        let using = match self.common.using {
            Some((slot, ticks)) => match self.common.stack_in(slot) {
                Some(s) if &*s.item == "minecraft:bow" => {
                    TriggerFeel::Draw(gamepad::bow_power(ticks))
                }
                // TridentItem.THROW_THRESHOLD_TIME.
                Some(s) if &*s.item == "minecraft:trident" => {
                    TriggerFeel::Draw(ticks as f32 / 10.0)
                }
                Some(s) if &*s.item == "minecraft:crossbow" => TriggerFeel::Draw(
                    ticks as f32
                        / sunrise_gameplay_common::game::CommonGame::charge_ticks(s).max(1) as f32,
                ),
                Some(s) if self.blocks_attacks(s).is_some() => TriggerFeel::Brace,
                _ => TriggerFeel::Free,
            },
            None => self.idle_use_feel(),
        };
        // Creative breaks at once; blocks that can't be broken have a negative hardness.
        let attacking = match &self.common.mining {
            Some(m) if !self.common.creative => {
                let state = self.common.world.block(m.pos[0], m.pos[1], m.pos[2]);
                TriggerFeel::Texture(self.common.res.facts().get(state).hardness)
            }
            _ => TriggerFeel::Free,
        };
        (using, attacking)
    }

    /// The use trigger while nothing is in use: a loaded crossbow clicks, an item that boosts (a
    /// rocket while gliding, the stick of the mount being ridden) catches, a shield weighs, and
    /// locks while on cooldown after an axe disable.
    pub(in crate::game::client_side) fn idle_use_feel(&self) -> TriggerFeel {
        let gliding = self.common.player.as_ref().is_some_and(|p| p.fall_flying);
        let mount = self
            .common
            .riding
            .and_then(|id| self.common.mobs.iter().find(|m| m.id == id));
        let mount = mount.map_or("", |m| m.kind.id.as_str());
        let feel = |stack: Option<&sunrise_items::items::ItemStack>| match stack
            .map(|s| (s, &*s.item))
        {
            Some((s, "minecraft:crossbow")) if s.data().is_some_and(|d| !d.charged.is_empty()) => {
                TriggerFeel::Click
            }
            Some((_, "minecraft:firework_rocket")) if gliding => TriggerFeel::Boost,
            Some((_, "minecraft:carrot_on_a_stick")) if mount == "minecraft:pig" => {
                TriggerFeel::Boost
            }
            Some((_, "minecraft:warped_fungus_on_a_stick")) if mount == "minecraft:strider" => {
                TriggerFeel::Boost
            }
            Some((s, _)) if self.blocks_attacks(s).is_some() => {
                let default = self
                    .items()
                    .get(&s.item)
                    .and_then(|i| i.use_cooldown.as_ref());
                let group = sunrise_items::cooldowns::group(s, default);
                if self.common.cooldowns.is_on_cooldown(&group) {
                    TriggerFeel::Locked
                } else {
                    TriggerFeel::Brace
                }
            }
            _ => TriggerFeel::Free,
        };
        // Use tries the main hand first.
        match feel(self.common.stack_in(self.common.selected)) {
            TriggerFeel::Free => feel(self.common.offhand.as_ref()),
            main => main,
        }
    }

    /// Open the pad's haptics once its output is open over USB (its audio device is there
    /// only then); close them when it goes. Opt-in with `SUNRISE_DUALSENSE_HAPTICS=1` until
    /// they are proven on a pad (docs/controllers.md); never for games without sound (tests).
    pub(in crate::game::client_side) fn pad_haptics(&mut self) {
        let usb = self
            .client
            .pad
            .0
            .devices
            .output
            .as_ref()
            .and_then(|o| o.open())
            == Some(sunrise_dualsense::Transport::Usb);
        if !usb {
            self.client.pad.0.devices.haptics = None;
            self.client.pad.0.devices.haptics_tried = false;
        } else if !self.client.pad.0.devices.haptics_tried {
            self.client.pad.0.devices.haptics_tried = true;
            if std::env::var_os("SUNRISE_DUALSENSE_HAPTICS").is_some_and(|v| v == "1")
                && std::env::var_os("SUNRISE_NO_AUDIO").is_none()
            {
                self.client.pad.0.devices.haptics = sunrise_dualsense::haptics::Haptics::open();
            }
        }
    }

    /// Release the DualSense's output (the game is quitting).
    pub(in crate::game::client_side) fn pad_close_output(&mut self) {
        self.client.pad.0.devices.haptics = None;
        self.client.pad.0.devices.output = None;
    }

    /// Play a vibration on the active pad: a DualSense through its own report, else gilrs' force
    /// feedback, which platforms without it (gilrs on macOS) skip.
    pub(in crate::game::client_side) fn pad_vibrate(
        &mut self,
        rumble: Rumble,
        pulse: gamepad::Pulse,
    ) {
        let level = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        if let Some(haptics) = self
            .client
            .pad
            .0
            .devices
            .haptics
            .as_ref()
            .filter(|_| self.pad_output().is_some())
        {
            let steps = rumble.shape().into_iter().map(|step| {
                (
                    step.strong,
                    step.weak,
                    Duration::from_millis(u64::from(step.millis)),
                )
            });
            haptics.play(&steps.collect::<Vec<_>>());
            return;
        }
        if let Some(output) = self.pad_output() {
            let steps = rumble.shape().into_iter().map(|step| {
                let length = Duration::from_millis(u64::from(step.millis));
                ([level(step.strong), level(step.weak)], length)
            });
            output.rumble(steps.collect());
            return;
        }
        let (Some(gilrs), Some(id)) = (
            self.client.pad.0.devices.gilrs.as_mut(),
            self.client.pad.0.devices.active,
        ) else {
            return;
        };
        if let Some(playing) = play_pulse(gilrs, id, pulse) {
            self.client.pad.0.devices.rumbles.push(playing);
        }
    }

    pub(in crate::game::client_side) fn pad_devices_status(&self) -> Value {
        self.client.pad.0.devices.status()
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod takeover_tests {
    use super::*;
    use gilrs::Axis as A;

    #[test]
    fn an_idle_pads_resting_trigger_doesnt_take_over() {
        for axis in [A::LeftZ, A::RightZ, A::Unknown] {
            assert!(!axis_takes_over(axis, -0.99, false), "{axis:?}");
            assert!(axis_takes_over(axis, 0.9, false), "{axis:?}");
        }
    }

    #[test]
    fn a_stick_takes_over_in_either_direction() {
        for axis in [A::LeftStickX, A::LeftStickY, A::RightStickX, A::RightStickY] {
            assert!(axis_takes_over(axis, -0.9, false), "{axis:?}");
            assert!(axis_takes_over(axis, 0.9, false), "{axis:?}");
            assert!(!axis_takes_over(axis, 0.2, false), "{axis:?}");
        }
    }

    #[test]
    fn a_right_stick_on_z_takes_over_in_either_direction() {
        // An Xbox pad over Bluetooth pushed left, or up.
        assert!(axis_takes_over(A::LeftZ, -0.9, true));
        assert!(axis_takes_over(A::RightZ, -0.9, true));
        assert!(
            !axis_takes_over(A::Unknown, -0.99, true),
            "its triggers rest here"
        );
    }
}
