//! The client's input layer, below gameplay and screens: key bindings and their
//! names, toggle keys, mouse settings, controllers, the clipboard and window
//! options. sunrise-client re-exports each module under its old path
//! (`sunrise_client::keybinds`, ...).
#![forbid(unsafe_code)]
pub mod actions;
pub mod clipboard;
pub mod gamepad;
pub mod gameplay_guide;
pub mod held;
pub mod input;
pub mod input_source;
pub mod interaction_input;
pub use sunrise_game_data::key_names;
pub mod keybinds;
pub mod keymapping;
pub mod modifiers;
pub mod mouse;
pub mod radial;
pub mod snap;
pub mod window;
