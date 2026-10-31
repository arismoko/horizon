//! Trait mods implement `Hooks` and link into the game executable.
//! The executable passes the ordered `Mods` to `entry::main_with`.
//! Hooks use `&self + Send + Sync` for shared and reentrant calls; see `docs/native-mods.md`.
//! Generate the trait and dispatch with `cargo run -p xtask -- mod-api`.
//! `Host` is hand-written for this slice; a later change will generate it from native helpers.
use std::sync::Arc;

pub use crate::items::ItemStack;
pub use crate::text::Text;
pub use horizon_world::registry::BlockStateId;

/// A native zoom factor and mouse turning scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeZoom {
    pub factor: f32,
    pub smooth: bool,
    pub turn: f32,
}

/// The ordered list of mods shared by the client and server.
pub type Mods = Arc<[Box<dyn Hooks>]>;

/// Whether a mod claims a cancellable hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Later mods and the game continue.
    Continue,
    /// Set the hook flag and skip later mods.
    Cancel,
}

/// Services supplied by the game to trait mods.
pub trait Host {
    /// Read the block with `native_block`.
    fn block(&self, pos: [i32; 3]) -> String;
    /// Set the block with `native_set_block`.
    fn set_block(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> bool;
    /// Send the acting player a line with `native_feedback`.
    fn feedback(&mut self, text: Text);
}

mod generated;
pub use generated::Hooks;
#[doc(hidden)]
pub use generated::{
    dispatch_level_tick_pre, dispatch_tooltip, dispatch_use_item_on, dispatch_zoom,
};
