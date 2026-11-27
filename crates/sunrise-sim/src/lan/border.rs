//! The world border a server's players see: all of it when they enter a
//! level (`PlayerList.sendLevelInfo`), then each change as
//! `BorderChangeListener` broadcasts it: a new centre, a new size, a size
//! change in progress, a new warning time or distance.
use crate::worldborder::WorldBorder;
use sunrise_protocol::packets::play;

/// `ClientboundInitializeBorderPacket`.
pub fn initialize(b: &WorldBorder) -> play::InitializeBorder {
    let [center_x, center_z] = b.center();
    play::InitializeBorder {
        center_x,
        center_z,
        old_size: b.size(),
        new_size: b.target_size(),
        lerp_time: b.remaining_ticks(),
        absolute_max_size: b.absolute_max_size(),
        warning_blocks: b.warning_distance,
        warning_time: b.warning_time,
    }
}
