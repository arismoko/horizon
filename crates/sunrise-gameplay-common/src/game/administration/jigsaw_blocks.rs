//! Common jigsaw block helpers shared by client UI and server handlers.
use sunrise_level::jigsaw_block;

pub type Pos = (i32, i32, i32);

impl crate::game::CommonGame<'_> {
    /// The block's front facing, from its `orientation` (`JigsawBlock.getFrontFacing`).
    pub fn jigsaw_front(&self, pos: Pos) -> Option<String> {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let orientation = self
            .pack
            .assets
            .registry
            .state(state)?
            .properties
            .get("orientation")?;
        Some(jigsaw_block::front_of_orientation(orientation).to_owned())
    }
}
