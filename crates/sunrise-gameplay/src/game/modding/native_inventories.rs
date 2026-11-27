//! Map pictures sent to individual players.
use mod_api::{__private::game::Fields, PlayerRef};

impl crate::game::ServerGame<'_> {
    /// Sends the player `uuid` the whole picture of map `map_id`: 128 x 128
    /// colours (`MapColor` ids times 4 plus the shade, row by row), scale 0,
    /// locked, no decorations, as `ClientboundMapItemDataPacket`. A filled
    /// map the mod gives with that `minecraft:map_id` then shows it in hand
    /// and in item frames; the server keeps nothing (a map id with no saved
    /// map is never sent again by the game), so send it again after the
    /// player joins. Use ids far from the world's own (they count up from 0).
    /// False for a player not here, or `colors` not 16384 long.
    pub(in crate::game) fn player_send_map(
        &mut self,
        player: PlayerRef,
        map_id: i32,
        colors: &[u8],
    ) -> bool {
        use sunrise_protocol::packets::play::{Clientbound, MapItemData, MapPatch};
        if self.player_is_fake(player) != Some(false) || colors.len() != 128 * 128 {
            return false;
        }
        self.native_send(
            player.fields().uuid,
            Clientbound::MapItemData(MapItemData {
                map_id,
                scale: 0,
                locked: true,
                decorations: None,
                patch: Some(MapPatch {
                    width: 128,
                    height: 128,
                    start_x: 0,
                    start_y: 0,
                    colors: colors.to_vec(),
                }),
            }),
        )
    }
}
