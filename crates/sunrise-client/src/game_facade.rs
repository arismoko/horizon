#[cfg(feature = "client")]
pub use sunrise_client_gameplay::game::{
    BlockEntityDraw, MobRender, PlayerRender, native_block_entity_draw, native_client_services,
};
pub use sunrise_gameplay::game::HotPathsMods;
pub use sunrise_gameplay::game::hot_paths_next_block_entity;
pub use sunrise_gameplay::game::note_hot_paths_listener_call;
pub use sunrise_gameplay::game::server_side::dedicated;
pub use sunrise_gameplay_common::game::*;
