//! Draft API for statically linked trait mods.
use std::sync::Arc;

pub use crate::game::native_hooks::NativeZoom;
pub use crate::items::ItemStack;
pub use crate::text::Text;
pub use horizon_world::registry::BlockStateId;

/// The ordered list of mods shared by the client and server.
pub type Mods = Arc<[Box<dyn Hooks>]>;

/// Whether a mod claims a cancellable hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Cancel,
}

/// Services supplied by the game to trait mods.
pub trait Host {
    fn block(&self, pos: [i32; 3]) -> String;
    fn set_block(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> bool;
    fn feedback(&mut self, text: Text);
    fn chat(&mut self, text: &str);
}

mod generated;
pub use generated::Hooks;
pub(crate) use generated::*;
