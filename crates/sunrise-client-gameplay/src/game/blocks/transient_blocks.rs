//! `ClientboundAddTransientBlockPacket` on the window's `Game`: queueing, draining and drawing the blocks whose state lives in `sunrise_client_gameplay::transient_blocks`.
use crate::game::ClientGame;
use std::time::Instant;
pub(in crate::game) use sunrise_client_gameplay::transient_blocks::TransientBlocks;
use sunrise_client_gameplay::transient_blocks::shows_block;
use sunrise_client_render::render_geometry::Vertex;
use sunrise_world::registry::BlockStateId;

impl ClientGame<'_> {
    /// `ClientPacketListener.handleAddTransientBlockPacket`.
    pub(in crate::game::client_side) fn add_transient_block(&mut self, pos: [i32; 3], state: i32) {
        self.client
            .transient_blocks
            .queue(pos, BlockStateId(state as u32));
    }

    /// `LevelExtractor.drainTransientBlockQueue`, once a frame.
    pub(in crate::game::client_side) fn drain_transient_block_queue(&mut self) {
        self.client
            .transient_blocks
            .drain(Instant::now(), self.client.streaming.sequence());
    }

    /// `LevelRenderer.submitTransientBlocks`: each block whose time has not
    /// run out, drawn as a moving block at its position, camera-relative.
    /// One whose section mesh has been rebuilt since it came is dropped
    /// (`performTransientBlockRemovals`).
    pub(in crate::game::client_side) fn transient_block_geometry(
        &mut self,
        camera: glam::DVec3,
        out: &mut Vec<Vertex>,
    ) {
        if self.client.transient_blocks.shown.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut shown = std::mem::take(&mut self.client.transient_blocks.shown);
        shown.retain(|pos, block| {
            if now >= block.live_until {
                return false;
            }
            // performTransientBlockRemovals: the section's uploaded mesh began compiling
            // at or after the block was created, so it shows the block.
            !self.common.section_of(*pos).is_some_and(|section| {
                shows_block(
                    block.created,
                    self.client.streaming.compile_start(section),
                    self.section_upload_pending(section),
                )
            })
        });
        for (pos, block) in &shown {
            let at = glam::DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
            self.moving_block_vertices(
                block.state,
                sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
                at - camera,
                out,
            );
        }
        self.client.transient_blocks.shown = shown;
    }
}
