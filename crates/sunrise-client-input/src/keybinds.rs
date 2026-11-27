//! Key mappings (Options.keyMappings): vanilla names, categories and
//! defaults, `key_<name>:key.keyboard.x` persistence, display names, and the
//! translation of rebound physical keys back to the default keys the game
//! logic listens for.
use std::{collections::HashMap, sync::LazyLock};
use winit::{event::MouseButton, keyboard::KeyCode};

pub use sunrise_game_data::key_mappings::MAPPINGS;
/// Mappings Options builds as `ToggleKeyMapping` (hold or toggle by option).
pub fn is_toggle(name: &str) -> bool {
    matches!(name, "key.sneak" | "key.sprint" | "key.use" | "key.attack")
}
/// KeyMapping.Category sort order.
pub const CATEGORIES: [&str; 8] = [
    "movement",
    "misc",
    "multiplayer",
    "gameplay",
    "inventory",
    "creative",
    "spectator",
    "debug",
];

/// InputConstants key names <-> winit physical keys.
const KEYS: &[(&str, KeyCode)] = &[
    ("key.keyboard.a", KeyCode::KeyA),
    ("key.keyboard.b", KeyCode::KeyB),
    ("key.keyboard.c", KeyCode::KeyC),
    ("key.keyboard.d", KeyCode::KeyD),
    ("key.keyboard.e", KeyCode::KeyE),
    ("key.keyboard.f", KeyCode::KeyF),
    ("key.keyboard.g", KeyCode::KeyG),
    ("key.keyboard.h", KeyCode::KeyH),
    ("key.keyboard.i", KeyCode::KeyI),
    ("key.keyboard.j", KeyCode::KeyJ),
    ("key.keyboard.k", KeyCode::KeyK),
    ("key.keyboard.l", KeyCode::KeyL),
    ("key.keyboard.m", KeyCode::KeyM),
    ("key.keyboard.n", KeyCode::KeyN),
    ("key.keyboard.o", KeyCode::KeyO),
    ("key.keyboard.p", KeyCode::KeyP),
    ("key.keyboard.q", KeyCode::KeyQ),
    ("key.keyboard.r", KeyCode::KeyR),
    ("key.keyboard.s", KeyCode::KeyS),
    ("key.keyboard.t", KeyCode::KeyT),
    ("key.keyboard.u", KeyCode::KeyU),
    ("key.keyboard.v", KeyCode::KeyV),
    ("key.keyboard.w", KeyCode::KeyW),
    ("key.keyboard.x", KeyCode::KeyX),
    ("key.keyboard.y", KeyCode::KeyY),
    ("key.keyboard.z", KeyCode::KeyZ),
    ("key.keyboard.1", KeyCode::Digit1),
    ("key.keyboard.2", KeyCode::Digit2),
    ("key.keyboard.3", KeyCode::Digit3),
    ("key.keyboard.4", KeyCode::Digit4),
    ("key.keyboard.5", KeyCode::Digit5),
    ("key.keyboard.6", KeyCode::Digit6),
    ("key.keyboard.7", KeyCode::Digit7),
    ("key.keyboard.8", KeyCode::Digit8),
    ("key.keyboard.9", KeyCode::Digit9),
    ("key.keyboard.0", KeyCode::Digit0),
    ("key.keyboard.enter", KeyCode::Enter),
    ("key.keyboard.escape", KeyCode::Escape),
    ("key.keyboard.backspace", KeyCode::Backspace),
    ("key.keyboard.tab", KeyCode::Tab),
    ("key.keyboard.space", KeyCode::Space),
    ("key.keyboard.minus", KeyCode::Minus),
    ("key.keyboard.equal", KeyCode::Equal),
    ("key.keyboard.left.bracket", KeyCode::BracketLeft),
    ("key.keyboard.right.bracket", KeyCode::BracketRight),
    ("key.keyboard.backslash", KeyCode::Backslash),
    ("key.keyboard.semicolon", KeyCode::Semicolon),
    ("key.keyboard.apostrophe", KeyCode::Quote),
    ("key.keyboard.grave.accent", KeyCode::Backquote),
    ("key.keyboard.comma", KeyCode::Comma),
    ("key.keyboard.period", KeyCode::Period),
    ("key.keyboard.slash", KeyCode::Slash),
    ("key.keyboard.caps.lock", KeyCode::CapsLock),
    ("key.keyboard.world.1", KeyCode::IntlBackslash),
    ("key.keyboard.international1", KeyCode::IntlRo),
    ("key.keyboard.international3", KeyCode::IntlYen),
    ("key.keyboard.lang1", KeyCode::Lang1),
    ("key.keyboard.lang2", KeyCode::Lang2),
    ("key.keyboard.lang3", KeyCode::Lang3),
    ("key.keyboard.lang4", KeyCode::Lang4),
    ("key.keyboard.lang5", KeyCode::Lang5),
    ("key.keyboard.help", KeyCode::Help),
    ("key.keyboard.select", KeyCode::Select),
    ("key.keyboard.again", KeyCode::Again),
    ("key.keyboard.undo", KeyCode::Undo),
    ("key.keyboard.cut", KeyCode::Cut),
    ("key.keyboard.copy", KeyCode::Copy),
    ("key.keyboard.paste", KeyCode::Paste),
    ("key.keyboard.find", KeyCode::Find),
    ("key.keyboard.f1", KeyCode::F1),
    ("key.keyboard.f2", KeyCode::F2),
    ("key.keyboard.f3", KeyCode::F3),
    ("key.keyboard.f4", KeyCode::F4),
    ("key.keyboard.f5", KeyCode::F5),
    ("key.keyboard.f6", KeyCode::F6),
    ("key.keyboard.f7", KeyCode::F7),
    ("key.keyboard.f8", KeyCode::F8),
    ("key.keyboard.f9", KeyCode::F9),
    ("key.keyboard.f10", KeyCode::F10),
    ("key.keyboard.f11", KeyCode::F11),
    ("key.keyboard.f12", KeyCode::F12),
    ("key.keyboard.f13", KeyCode::F13),
    ("key.keyboard.f14", KeyCode::F14),
    ("key.keyboard.f15", KeyCode::F15),
    ("key.keyboard.f16", KeyCode::F16),
    ("key.keyboard.f17", KeyCode::F17),
    ("key.keyboard.f18", KeyCode::F18),
    ("key.keyboard.f19", KeyCode::F19),
    ("key.keyboard.f20", KeyCode::F20),
    ("key.keyboard.f21", KeyCode::F21),
    ("key.keyboard.f22", KeyCode::F22),
    ("key.keyboard.f23", KeyCode::F23),
    ("key.keyboard.f24", KeyCode::F24),
    ("key.keyboard.print.screen", KeyCode::PrintScreen),
    ("key.keyboard.scroll.lock", KeyCode::ScrollLock),
    ("key.keyboard.pause", KeyCode::Pause),
    ("key.keyboard.insert", KeyCode::Insert),
    ("key.keyboard.home", KeyCode::Home),
    ("key.keyboard.page.up", KeyCode::PageUp),
    ("key.keyboard.delete", KeyCode::Delete),
    ("key.keyboard.end", KeyCode::End),
    ("key.keyboard.page.down", KeyCode::PageDown),
    ("key.keyboard.right", KeyCode::ArrowRight),
    ("key.keyboard.left", KeyCode::ArrowLeft),
    ("key.keyboard.down", KeyCode::ArrowDown),
    ("key.keyboard.up", KeyCode::ArrowUp),
    ("key.keyboard.num.lock", KeyCode::NumLock),
    ("key.keyboard.keypad.divide", KeyCode::NumpadDivide),
    ("key.keyboard.keypad.multiply", KeyCode::NumpadMultiply),
    ("key.keyboard.keypad.subtract", KeyCode::NumpadSubtract),
    ("key.keyboard.keypad.add", KeyCode::NumpadAdd),
    ("key.keyboard.keypad.enter", KeyCode::NumpadEnter),
    ("key.keyboard.keypad.1", KeyCode::Numpad1),
    ("key.keyboard.keypad.2", KeyCode::Numpad2),
    ("key.keyboard.keypad.3", KeyCode::Numpad3),
    ("key.keyboard.keypad.4", KeyCode::Numpad4),
    ("key.keyboard.keypad.5", KeyCode::Numpad5),
    ("key.keyboard.keypad.6", KeyCode::Numpad6),
    ("key.keyboard.keypad.7", KeyCode::Numpad7),
    ("key.keyboard.keypad.8", KeyCode::Numpad8),
    ("key.keyboard.keypad.9", KeyCode::Numpad9),
    ("key.keyboard.keypad.0", KeyCode::Numpad0),
    ("key.keyboard.keypad.period", KeyCode::NumpadDecimal),
    ("key.keyboard.keypad.equal", KeyCode::NumpadEqual),
    ("key.keyboard.keypad.comma", KeyCode::NumpadComma),
    ("key.keyboard.keypad.backspace", KeyCode::NumpadBackspace),
    (
        "key.keyboard.keypad.left.parenthesis",
        KeyCode::NumpadParenLeft,
    ),
    (
        "key.keyboard.keypad.right.parenthesis",
        KeyCode::NumpadParenRight,
    ),
    ("key.keyboard.keypad.hash", KeyCode::NumpadHash),
    (
        "key.keyboard.keypad.memory.store",
        KeyCode::NumpadMemoryStore,
    ),
    (
        "key.keyboard.keypad.memory.recall",
        KeyCode::NumpadMemoryRecall,
    ),
    (
        "key.keyboard.keypad.memory.clear",
        KeyCode::NumpadMemoryClear,
    ),
    ("key.keyboard.keypad.memory.add", KeyCode::NumpadMemoryAdd),
    (
        "key.keyboard.keypad.memory.subtract",
        KeyCode::NumpadMemorySubtract,
    ),
    ("key.keyboard.keypad.clear", KeyCode::NumpadClear),
    ("key.keyboard.keypad.clear.entry", KeyCode::NumpadClearEntry),
    ("key.keyboard.menu", KeyCode::ContextMenu),
    ("key.keyboard.left.control", KeyCode::ControlLeft),
    ("key.keyboard.left.shift", KeyCode::ShiftLeft),
    ("key.keyboard.left.alt", KeyCode::AltLeft),
    ("key.keyboard.left.win", KeyCode::SuperLeft),
    ("key.keyboard.right.control", KeyCode::ControlRight),
    ("key.keyboard.right.shift", KeyCode::ShiftRight),
    ("key.keyboard.right.alt", KeyCode::AltRight),
    ("key.keyboard.right.win", KeyCode::SuperRight),
];
pub fn key_code(name: &str) -> Option<KeyCode> {
    KEYS.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
}
pub fn key_name(code: KeyCode) -> Option<&'static str> {
    KEYS.iter().find(|(_, k)| *k == code).map(|(n, _)| *n)
}
pub fn mouse_name(button: MouseButton) -> String {
    match button {
        MouseButton::Left => "key.mouse.left".into(),
        MouseButton::Right => "key.mouse.right".into(),
        MouseButton::Middle => "key.mouse.middle".into(),
        MouseButton::Back => "key.mouse.4".into(),
        MouseButton::Forward => "key.mouse.5".into(),
        MouseButton::Other(n) => format!("key.mouse.{}", u32::from(n) + 1),
    }
}
/// `InputConstants.getKey` for a mouse button: the three named buttons, else
/// the name's number in vanilla's numbering (`InputConstants$Type`, as
/// [`crate::key_names::MOUSE`]: left 1, middle 2, right 3, then 4 and up). 6
/// and up are the inverse of [`mouse_name`] for `Other(5)` and above;
/// `Other(0)` to `Other(4)` share their names with vanilla's five numbered
/// buttons. A number no button has resolves to none.
pub fn mouse_button(name: &str) -> Option<MouseButton> {
    if let Some(button) = named_mouse_button(name) {
        return Some(button);
    }
    Some(
        match name.strip_prefix("key.mouse.")?.parse::<i32>().ok()? {
            n @ 1..=3 => crate::key_names::MOUSE
                .iter()
                .find(|(k, _)| *k == n)
                .and_then(|(_, named)| named_mouse_button(named))?,
            4 => MouseButton::Back,
            5 => MouseButton::Forward,
            n if n >= 6 => MouseButton::Other(u16::try_from(n - 1).ok()?),
            _ => return None,
        },
    )
}
/// The three buttons vanilla names (`key.mouse.left`, `middle`, `right`).
fn named_mouse_button(name: &str) -> Option<MouseButton> {
    Some(match name {
        "key.mouse.left" => MouseButton::Left,
        "key.mouse.right" => MouseButton::Right,
        "key.mouse.middle" => MouseButton::Middle,
        _ => return None,
    })
}
pub use sunrise_game_data::key_display::{
    clear_action_bindings, clear_action_resolver, display_name, replace_action_bindings,
    reset_action_bindings, set_action_binding, set_action_resolver,
};

/// A key event: what the layout writes on its key, without modifiers, names
/// that key from now on (`key_display::learn_layout`), so an AZERTY player's
/// Q key shows A.
pub fn learn_layout(event: &winit::event::KeyEvent) {
    use winit::keyboard::{Key, PhysicalKey};
    use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
    let PhysicalKey::Code(code) = event.physical_key else {
        return;
    };
    let Some(name) = key_name(code) else {
        return;
    };
    let learn = sunrise_game_data::key_display::learn_layout;
    match event.key_without_modifiers() {
        Key::Character(text) => learn(name, &text),
        Key::Dead(Some(c)) => learn(name, c.encode_utf8(&mut [0; 4])),
        _ => {}
    }
}

/// Vanilla's name for the key the layout produced (`InputEvent.Key.getKeycode`):
/// `layout` is the event's key without modifiers. A letter or digit names its
/// own key, so AZERTY's key at QWERTY's W gives `key.keyboard.z`; a named key
/// (Enter, arrows, Shift) keeps its place on every layout, so it gets the
/// physical key's name. `None` for any other character and for a physical key
/// vanilla doesn't name. A dead key arrives as its character (winit's key
/// without modifiers), so an accent key gives `None`, not a dead-key value.
pub fn layout_key_name(
    physical: winit::keyboard::PhysicalKey,
    layout: &winit::keyboard::Key,
) -> Option<&'static str> {
    use winit::keyboard::{Key, PhysicalKey};
    match layout {
        Key::Character(text) => {
            let mut chars = text.chars();
            let c = chars.next().filter(|c| c.is_ascii_alphanumeric())?;
            if chars.next().is_some() {
                return None;
            }
            let c = c.to_ascii_lowercase();
            KEYS.iter().map(|(n, _)| *n).find(|n| {
                n.strip_prefix("key.keyboard.")
                    .is_some_and(|rest| rest.chars().eq([c]))
            })
        }
        Key::Named(_) => match physical {
            PhysicalKey::Code(code) => key_name(code),
            PhysicalKey::Unidentified(_) => None,
        },
        _ => None,
    }
}

/// A code telling physical keys apart, named or not (`InputEvent.Key.getScanCode`):
/// the platform scan code, or the native code winit reports for a key it
/// doesn't identify; 0 when there is neither. Stable within a platform only.
pub fn raw_key_code(physical: winit::keyboard::PhysicalKey) -> u32 {
    use winit::keyboard::{NativeKeyCode, PhysicalKey};
    use winit::platform::scancode::PhysicalKeyExtScancode;
    match physical {
        PhysicalKey::Unidentified(native) => match native {
            NativeKeyCode::Android(c) | NativeKeyCode::Xkb(c) => c,
            NativeKeyCode::MacOS(c) | NativeKeyCode::Windows(c) => c.into(),
            NativeKeyCode::Unidentified => 0,
        },
        PhysicalKey::Code(_) => physical.to_scancode().unwrap_or(0),
    }
}

/// An engine-side index into [`MAPPINGS`], shared by all binding sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VanillaMappingSlot(usize);

impl VanillaMappingSlot {
    /// Resolve a canonical mapping name (e.g. `key.jump`), not a key name.
    pub fn by_name(name: &str) -> Option<Self> {
        // Looked up per held-mapping read, which a server makes on every player turn.
        static BY_NAME: LazyLock<
            sunrise_core::fast_hash::FxHashMap<&'static str, VanillaMappingSlot>,
        > = LazyLock::new(|| {
            VanillaMappingSlot::all()
                .map(|slot| (slot.definition().0, slot))
                .collect()
        });
        BY_NAME.get(name).copied()
    }

    /// The gameplay mapping whose default is `key`, independent of its current binding.
    pub fn by_default_key(key: KeyCode) -> Option<Self> {
        static BY_KEY: LazyLock<HashMap<KeyCode, VanillaMappingSlot>> = LazyLock::new(|| {
            VanillaMappingSlot::all()
                .filter(|slot| slot.definition().2 != "debug")
                .filter_map(|slot| Some((slot.default_input()?.ok()?, slot)))
                .collect()
        });
        BY_KEY.get(&key).copied()
    }

    /// Resolve an index only if it names a row in [`MAPPINGS`].
    pub const fn from_index(index: usize) -> Option<Self> {
        if index < MAPPINGS.len() {
            Some(Self(index))
        } else {
            None
        }
    }

    /// The in-process index, not a persisted key mapping name.
    pub const fn index(self) -> usize {
        self.0
    }

    /// The canonical mapping name, default binding name, and category, in that order.
    pub const fn definition(self) -> (&'static str, &'static str, &'static str) {
        MAPPINGS[self.index()]
    }

    /// The default input for routing to gameplay (`Ok` for keyboard, `Err` for mouse).
    /// `None` means the default is unbound or unsupported.
    pub fn default_input(self) -> Option<Result<KeyCode, MouseButton>> {
        static DEFAULTS: LazyLock<[Option<Result<KeyCode, MouseButton>>; MAPPINGS.len()]> =
            LazyLock::new(|| std::array::from_fn(|index| decode_input(MAPPINGS[index].1)));
        DEFAULTS[self.index()]
    }

    fn all() -> impl Iterator<Item = Self> {
        (0..MAPPINGS.len()).map(Self)
    }
}

fn decode_input(name: &str) -> Option<Result<KeyCode, MouseButton>> {
    key_code(name)
        .map(Ok)
        .or_else(|| mouse_button(name).map(Err))
}

const UNBOUND: &str = "key.keyboard.unknown";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    name: String,
    physical: Option<Result<KeyCode, MouseButton>>,
}

/// Current bindings, indexed in [`MAPPINGS`] order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBinds {
    bindings: Box<[Binding; MAPPINGS.len()]>,
}
impl Default for KeyBinds {
    fn default() -> Self {
        Self {
            bindings: Box::new(std::array::from_fn(|index| {
                let slot = VanillaMappingSlot(index);
                Binding {
                    name: slot.definition().1.to_owned(),
                    physical: slot.default_input(),
                }
            })),
        }
    }
}

impl KeyBinds {
    /// Lend the current binding name, including unbound and unsupported names.
    pub fn binding_at(&self, slot: VanillaMappingSlot) -> &str {
        &self.bindings[slot.index()].name
    }
    /// Read the decoded binding (`Ok` for keyboard, `Err` for mouse), or `None`
    /// if it is unbound or unsupported.
    pub fn physical_at(&self, slot: VanillaMappingSlot) -> Option<Result<KeyCode, MouseButton>> {
        self.bindings[slot.index()].physical
    }
    pub fn get(&self, name: &str) -> &str {
        VanillaMappingSlot::by_name(name).map_or(UNBOUND, |slot| self.binding_at(slot))
    }
    pub fn default_of(name: &str) -> &'static str {
        VanillaMappingSlot::by_name(name).map_or(UNBOUND, |slot| slot.definition().1)
    }
    pub fn is_default(&self, name: &str) -> bool {
        VanillaMappingSlot::by_name(name)
            .is_none_or(|slot| self.binding_at(slot) == slot.definition().1)
    }
    /// `KeyBindsScreen.extractRenderState`: Reset All is active only while a
    /// mapping differs from its default (`KeyMapping.isDefault`, the bound key
    /// against `defaultKey`).
    pub fn changed_from_defaults(&self) -> bool {
        VanillaMappingSlot::all().any(|slot| self.binding_at(slot) != slot.definition().1)
    }
    pub fn set(&mut self, name: &str, key: &str) {
        if let Some(slot) = VanillaMappingSlot::by_name(name) {
            self.bindings[slot.index()] = Binding {
                name: key.to_owned(),
                physical: decode_input(key),
            };
        }
    }
    /// Publish these bindings as the ones `keybind` text components show
    /// (`KeybindResolver`). Only the active set calls this: on load and after
    /// the options change it.
    pub fn sync_action_bindings(&self) {
        replace_action_bindings(
            VanillaMappingSlot::all().map(|slot| (slot.definition().0, self.binding_at(slot))),
        );
    }
    /// The bindings `options.txt` holds: vanilla's mappings only.
    pub fn saved(&self) -> Vec<(&'static str, String)> {
        VanillaMappingSlot::all()
            .map(|slot| (slot.definition().0, self.binding_at(slot).to_owned()))
            .collect()
    }
    pub fn reset_all(&mut self) {
        *self = Self::default();
    }
    /// Mappings sharing `name`'s key (KeyMapping.same), for conflict marks.
    pub fn conflicts(&self, name: &str) -> Vec<&'static str> {
        let Some(current) = VanillaMappingSlot::by_name(name) else {
            return Vec::new();
        };
        let key = self.binding_at(current);
        if key == UNBOUND {
            return Vec::new();
        }
        let moved = key != current.definition().1;
        VanillaMappingSlot::all()
            .filter(|&slot| {
                slot != current
                    && self.binding_at(slot) == key
                    && (moved || key != slot.definition().1)
            })
            .map(|slot| slot.definition().0)
            .collect()
    }
    /// Every slot bound to `key`, in [`MAPPINGS`] order.
    pub fn slots_for<'a>(&'a self, key: &'a str) -> impl Iterator<Item = VanillaMappingSlot> + 'a {
        VanillaMappingSlot::all().filter(move |&slot| self.binding_at(slot) == key)
    }
    /// Every mapping a physical input triggers, in list order: like
    /// `KeyMapping.set`/`click`, one key drives all the mappings bound to it.
    pub fn mappings_for<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'static str> + 'a {
        self.slots_for(key).map(|slot| slot.definition().0)
    }
    /// The mappings bound to `key` that [`KeyBinds::translate_all`] follows:
    /// all but the debug keys. Those act only with F3 held
    /// (`KeyboardHandler.handleDebugKeys`, `debug_keys.rs` reads the physical
    /// key and the binding itself); their defaults (A, S, D, T, L, 1 to 4...)
    /// would otherwise turn the key back into its old gameplay meaning.
    pub fn translated_mappings<'a>(
        &'a self,
        key: &'a str,
    ) -> impl Iterator<Item = &'static str> + 'a {
        self.translated_slots(key).map(|slot| slot.definition().0)
    }
    fn translated_slots<'a>(
        &'a self,
        key: &'a str,
    ) -> impl Iterator<Item = VanillaMappingSlot> + 'a {
        VanillaMappingSlot::all()
            .filter(move |&slot| slot.definition().2 != "debug" && self.binding_at(slot) == key)
    }
    /// Whether a mapping (other than a debug key) whose default is `key` was
    /// bound elsewhere.
    fn default_moved(&self, key: &str) -> bool {
        VanillaMappingSlot::all().any(|slot| {
            let (_, default, category) = slot.definition();
            default == key && category != "debug" && self.binding_at(slot) != default
        })
    }
    /// Translate a physical key into the default key of the mapping it is
    /// bound to, so game code written against vanilla defaults follows the
    /// player's bindings. `None` means the key now does nothing; mouse
    /// mappings bound to keys come back as `Err(button)`. The first of
    /// [`KeyBinds::translate_all`].
    pub fn translate(&self, key: KeyCode) -> Option<Result<KeyCode, MouseButton>> {
        self.translate_all(key).into_iter().next()
    }
    /// The default keys of every mapping `key` is bound to (empty when it
    /// now does nothing), so a key bound to two mappings (forward and
    /// sprint) drives both, as `KeyMapping.set` does.
    pub fn translate_all(&self, key: KeyCode) -> Vec<Result<KeyCode, MouseButton>> {
        let Some(name) = key_name(key) else {
            return vec![Ok(key)];
        };
        self.translate_input(name, Ok(key))
    }
    fn translate_input(
        &self,
        name: &str,
        input: Result<KeyCode, MouseButton>,
    ) -> Vec<Result<KeyCode, MouseButton>> {
        // Compare the original names: decoding e.g. `key.mouse.1` would also
        // match `key.mouse.left`, which the name-based controls don't do.
        let mut bound = Vec::new();
        for default in self
            .translated_slots(name)
            .filter_map(VanillaMappingSlot::default_input)
        {
            // A shared default (Pick Block and spectator hotbar) is one press.
            if !bound.contains(&default) {
                bound.push(default);
            }
        }
        // An unbound input whose default meaning moved elsewhere is inert.
        if bound.is_empty() && !self.default_moved(name) {
            bound.push(input);
        }
        bound
    }
    /// `Minecraft.handleKeybinds`: key.pickItem and key.spectatorHotbar share the middle button by
    /// default and are separate mappings, so a press of what `input` is bound to acts only as the
    /// one this player can use: the spectator hotbar for a spectator, pick block otherwise. (Pick
    /// block does nothing a spectator would see.)
    pub fn middle_acts(&self, input: &str, spectator: bool) -> bool {
        let mapping = if spectator {
            "key.spectatorHotbar"
        } else {
            "key.pickItem"
        };
        self.get(mapping) == input
    }
    /// Translate a mouse button the same way (keys bound to mouse buttons).
    pub fn translate_mouse(&self, button: MouseButton) -> Option<Result<KeyCode, MouseButton>> {
        self.translate_mouse_all(button).into_iter().next()
    }
    /// [`KeyBinds::translate_all`] for a mouse button.
    pub fn translate_mouse_all(&self, button: MouseButton) -> Vec<Result<KeyCode, MouseButton>> {
        self.translate_input(&mouse_name(button), Err(button))
    }
    /// `AbstractContainerScreen.checkHotbarMouseClicked`: off hand, then hotbar.
    pub fn container_mouse_key(&self, button: MouseButton, creative: bool) -> Option<KeyCode> {
        if matches!(button, MouseButton::Left | MouseButton::Right) {
            return None;
        }
        let name = mouse_name(button);
        if creative && self.get("key.pickItem") == name {
            return None;
        }
        self.container_key(&name)
    }
    /// `AbstractContainerScreen.checkHotbarKeyPressed`: the off hand and the
    /// hotbar slots, as the controls bind them now. Answers the key the menu
    /// acts on (`KeyF` for the off hand, `DigitN` for a slot), so a rebound key
    /// swaps instead of reaching the screen.
    pub fn container_hotbar_key(&self, key: KeyCode) -> Option<KeyCode> {
        self.container_key(key_name(key)?)
    }
    fn container_key(&self, name: &str) -> Option<KeyCode> {
        let offhand = VanillaMappingSlot::by_name("key.swapOffhand")
            .filter(|&slot| self.binding_at(slot) == name);
        let slot = offhand.or_else(|| {
            VanillaMappingSlot::all().find(|&slot| {
                slot.definition().0.starts_with("key.hotbar.") && self.binding_at(slot) == name
            })
        })?;
        slot.default_input()?.ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mappings_sharing_the_middle_button_act_as_themselves() {
        let middle = "key.mouse.middle";
        let z = "key.keyboard.z";
        let mut keys = KeyBinds::default();
        assert!(keys.middle_acts(middle, false) && keys.middle_acts(middle, true));
        // The spectator hotbar on Z: the middle button only picks, and Z never picks.
        keys.set("key.spectatorHotbar", z);
        assert!(keys.middle_acts(middle, false));
        assert!(!keys.middle_acts(middle, true));
        assert!(keys.middle_acts(z, true));
        assert!(!keys.middle_acts(z, false));
        // Pick block on Z instead: the other way round.
        keys.reset_all();
        keys.set("key.pickItem", z);
        assert!(keys.middle_acts(z, false));
        assert!(!keys.middle_acts(z, true));
        assert!(keys.middle_acts(middle, true));
        assert!(!keys.middle_acts(middle, false));
    }
    /// #1646: the name `mouse_name` writes for each named button, and for
    /// `Other(5)` up to the last button winit can name, reads back.
    #[test]
    fn every_saved_mouse_button_name_reads_back() {
        use MouseButton::*;
        for button in [
            Left,
            Right,
            Middle,
            Back,
            Forward,
            Other(5),
            Other(6),
            Other(99),
            Other(u16::MAX),
        ] {
            assert_eq!(
                mouse_button(&mouse_name(button)),
                Some(button),
                "{button:?}"
            );
        }
        assert_eq!(mouse_name(Other(5)), "key.mouse.6");
        assert_eq!(mouse_name(Other(u16::MAX)), "key.mouse.65536");
    }
    /// `InputConstants.getKey`: a name outside the named ones reads its number
    /// with `Integer.parseInt` as the button, in vanilla's numbering
    /// (`InputConstants$Type`: left 1, middle 2, right 3), and a number no
    /// button has never resolves.
    #[test]
    fn a_numbered_mouse_name_reads_as_vanillas_button() {
        use MouseButton::*;
        for (name, button) in [
            ("key.mouse.1", Left),
            ("key.mouse.2", Middle),
            ("key.mouse.3", Right),
            ("key.mouse.4", Back),
            ("key.mouse.5", Forward),
            ("key.mouse.+7", Other(6)),
            ("key.mouse.007", Other(6)),
        ] {
            assert_eq!(mouse_button(name), Some(button), "{name}");
        }
        for name in [
            "key.mouse.0",
            "key.mouse.-6",
            "key.mouse.65537",
            "key.mouse.2147483648",
            "key.mouse.-2147483648",
            "key.mouse.six",
            "key.mouse.",
            "key.mouse.6 ",
            "key.mouse6",
            "key.keyboard.6",
        ] {
            assert_eq!(mouse_button(name), None, "{name}");
        }
    }
    #[test]
    fn container_mouse_hotkeys_follow_slot_bindings_and_pick_item_priority() {
        for button in [MouseButton::Back, MouseButton::Forward, MouseButton::Middle] {
            for (mapping, default, _) in MAPPINGS.iter().filter(|m| m.0.starts_with("key.hotbar."))
            {
                let mut keys = KeyBinds::default();
                keys.set(mapping, &mouse_name(button));
                assert_eq!(keys.container_mouse_key(button, false), key_code(default));
                assert_eq!(
                    keys.container_mouse_key(button, true),
                    if button == MouseButton::Middle {
                        None
                    } else {
                        key_code(default)
                    }
                );
            }
        }
        let mut keys = KeyBinds::default();
        keys.set("key.hotbar.1", "key.mouse.4");
        keys.set("key.swapOffhand", "key.mouse.4");
        assert_eq!(
            keys.container_mouse_key(MouseButton::Back, false),
            Some(KeyCode::KeyF)
        );
        keys.set("key.hotbar.2", "key.mouse.left");
        assert_eq!(keys.container_mouse_key(MouseButton::Left, false), None);
        assert_eq!(keys.container_mouse_key(MouseButton::Right, false), None);
        assert_eq!(keys.container_mouse_key(MouseButton::Forward, false), None);
    }

    #[test]
    fn container_hotkeys_follow_the_bound_keys() {
        let mut keys = KeyBinds::default();
        assert_eq!(
            keys.container_hotbar_key(KeyCode::KeyF),
            Some(KeyCode::KeyF)
        );
        assert_eq!(
            keys.container_hotbar_key(KeyCode::Digit3),
            Some(KeyCode::Digit3)
        );
        assert_eq!(keys.container_hotbar_key(KeyCode::Tab), None);
        // The report's key: bound to the off hand, the menu takes it first.
        keys.set("key.swapOffhand", "key.keyboard.tab");
        assert_eq!(keys.container_hotbar_key(KeyCode::Tab), Some(KeyCode::KeyF));
        assert_eq!(keys.container_hotbar_key(KeyCode::KeyF), None);
        keys.set("key.hotbar.3", "key.keyboard.t");
        assert_eq!(
            keys.container_hotbar_key(KeyCode::KeyT),
            Some(KeyCode::Digit3)
        );
    }

    #[test]
    fn rebinding_translates_to_defaults() {
        let mut k = KeyBinds::default();
        assert_eq!(k.translate(KeyCode::KeyW), Some(Ok(KeyCode::KeyW)));
        // Forward on the up arrow, inventory on W.
        k.set("key.forward", "key.keyboard.up");
        k.set("key.inventory", "key.keyboard.w");
        assert_eq!(k.translate(KeyCode::ArrowUp), Some(Ok(KeyCode::KeyW)));
        assert_eq!(k.translate(KeyCode::KeyW), Some(Ok(KeyCode::KeyE)));
        // E's default meaning moved and nothing else uses it.
        assert_eq!(k.translate(KeyCode::KeyE), None);
        // Attack on a key.
        k.set("key.attack", "key.keyboard.r");
        assert_eq!(k.translate(KeyCode::KeyR), Some(Err(MouseButton::Left)));
        assert_eq!(k.translate_mouse(MouseButton::Left), None);
        assert_eq!(k.translate(KeyCode::Escape), Some(Ok(KeyCode::Escape)));
        assert!(k.conflicts("key.forward").is_empty());
        k.set("key.jump", "key.keyboard.up");
        assert_eq!(k.conflicts("key.forward"), ["key.jump"]);
    }
    #[test]
    fn a_moved_key_keeps_no_default_action() {
        // #3158: the debug keys (F3+A, F3+1...) share these keys' defaults,
        // and must not hand them back their gameplay meaning.
        let mut k = KeyBinds::default();
        k.set("key.hotbar.1", "key.keyboard.q");
        assert_eq!(k.translate_all(KeyCode::Digit1), []);
        assert_eq!(
            k.translate_all(KeyCode::KeyQ),
            [Ok(KeyCode::KeyQ), Ok(KeyCode::Digit1)]
        );
        k.set("key.inventory", "key.keyboard.a");
        k.set("key.left", "key.keyboard.e");
        assert_eq!(k.translate_all(KeyCode::KeyA), [Ok(KeyCode::KeyE)]);
        assert_eq!(k.translate_all(KeyCode::KeyE), [Ok(KeyCode::KeyA)]);
        k.set("key.back", "key.keyboard.down");
        assert_eq!(k.translate_all(KeyCode::KeyS), []);
        // Keys left alone still act, and a moved debug key changes nothing.
        k.set("key.debug.fpsCharts", "key.keyboard.9");
        assert_eq!(k.translate_all(KeyCode::Digit2), [Ok(KeyCode::Digit2)]);
        assert_eq!(k.translate_all(KeyCode::Digit9), [Ok(KeyCode::Digit9)]);
        assert_eq!(k.translate_all(KeyCode::F3), [Ok(KeyCode::F3)]);
    }

    /// `KeyBindsScreen.extractRenderState`: Reset All is active only while a
    /// binding differs from its default.
    #[test]
    fn reset_all_is_active_only_after_a_binding_moves() {
        let mut keys = KeyBinds::default();
        assert!(!keys.changed_from_defaults());
        keys.set("key.attack", "key.mouse.right");
        assert!(keys.changed_from_defaults());
        // Setting a mapping back to its own default leaves nothing to reset.
        keys.set("key.attack", KeyBinds::default_of("key.attack"));
        assert!(!keys.changed_from_defaults());
        keys.set("key.jump", "key.keyboard.z");
        keys.reset_all();
        assert!(!keys.changed_from_defaults());
    }
    #[test]
    fn display_names() {
        assert_eq!(display_name("key.keyboard.w").plain(), "W");
        assert_eq!(display_name("key.keyboard.slash").plain(), "/");
    }
    #[test]
    fn only_a_synced_set_changes_keybind_display() {
        let mut k = KeyBinds::default();
        k.set("key.forward", "key.keyboard.up");
        k.sync_action_bindings();
        assert_eq!(display_name("key.forward").plain(), "key.keyboard.up");
        let mut inactive = KeyBinds::default();
        inactive.set("key.forward", "key.keyboard.x");
        assert_eq!(display_name("key.forward").plain(), "key.keyboard.up");
        clear_action_bindings();
    }

    #[test]
    fn a_key_event_names_the_layout_key_and_tells_unnamed_keys_apart() {
        use winit::keyboard::{Key, NamedKey, NativeKeyCode, PhysicalKey};
        // AZERTY: the key at QWERTY's W writes z.
        let w = PhysicalKey::Code(KeyCode::KeyW);
        assert_eq!(key_name(KeyCode::KeyW), Some("key.keyboard.w"));
        let z = layout_key_name(w, &Key::Character("Z".into()));
        assert_eq!(z, Some("key.keyboard.z"));
        let enter = PhysicalKey::Code(KeyCode::Enter);
        let named = layout_key_name(enter, &Key::Named(NamedKey::Enter));
        assert_eq!(named, Some("key.keyboard.enter"));
        assert_eq!(layout_key_name(w, &Key::Character("é".into())), None);
        assert_eq!(layout_key_name(w, &Key::Dead(Some('^'))), None);
        // Two keys vanilla doesn't name share a name but not a code.
        let a = PhysicalKey::Unidentified(NativeKeyCode::Windows(0x6a));
        let b = PhysicalKey::Unidentified(NativeKeyCode::Windows(0x6b));
        assert_eq!((raw_key_code(a), raw_key_code(b)), (0x6a, 0x6b));
    }
}
