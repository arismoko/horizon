//! KeyboardInput direction and LocalPlayer input-speed arithmetic for 26.3.
//! Item-use and sneak multipliers come from the player's current state; this
//! module does not implement sprint eligibility, poses or the input lifecycle.

use std::collections::HashSet;
use winit::keyboard::{KeyCode, ModifiersState};

pub use sunrise_game_data::move_input::{keyboard_direction, modify_input};

/// `InputQuirks.isQuitShortcutDown`: checks whether the platform's window-closing
/// keyboard shortcut is held.
/// - On macOS: Command (`super_key()`) + `Q`.
/// - On Windows / Linux / other: Alt (`alt_key()`) + `F4`.
pub fn is_quit_shortcut_down(modifiers: ModifiersState, keys: &HashSet<KeyCode>) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.super_key() && keys.contains(&KeyCode::KeyQ)
    } else {
        modifiers.alt_key() && keys.contains(&KeyCode::F4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_shortcut_follows_platform_modifiers_and_keys() {
        let mut keys = HashSet::new();
        let alt = ModifiersState::ALT;
        let super_key = ModifiersState::SUPER;

        assert!(!is_quit_shortcut_down(ModifiersState::empty(), &keys));

        keys.insert(KeyCode::F4);
        if cfg!(target_os = "macos") {
            assert!(!is_quit_shortcut_down(alt, &keys));
            keys.remove(&KeyCode::F4);
            keys.insert(KeyCode::KeyQ);
            assert!(is_quit_shortcut_down(super_key, &keys));
            assert!(!is_quit_shortcut_down(alt, &keys));
        } else {
            assert!(is_quit_shortcut_down(alt, &keys));
            assert!(!is_quit_shortcut_down(super_key, &keys));
            keys.remove(&KeyCode::F4);
            keys.insert(KeyCode::KeyQ);
            assert!(!is_quit_shortcut_down(alt, &keys));
            assert!(!is_quit_shortcut_down(super_key, &keys));
        }
    }
}
