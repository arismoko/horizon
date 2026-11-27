//! A player's permission level for native server mods, through a
//! `PlayerRef` (docs/modding/src/how-to/native-mods.md): the `Player` reads the handle's game
//! mode reads don't give, and what the player may do to blocks and other
//! players (`mayUseItemAt`, `canHarmPlayer`). They return None for a stale
//! handle or a player who isn't online here.
use crate::game::ServerGame;
use mod_api::{BlockFace, GameMode, PlayerRef};
use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
use sunrise_gameplay_common::game::native_server::PlayerRead;
use sunrise_gameplay_common::game::native_world_read_common::block_face_direction;
use sunrise_items::items::ItemStack;

impl ServerGame<'_> {
    /// The player's permission level, 0 to 4 (`Player.permissions()`,
    /// `PlayerList.getProfilePermissions`): an op's level, the
    /// single-player owner's 4 with commands allowed. A displaced connected
    /// player keeps their permission level even while their body is unavailable.
    /// None for a stale or unavailable player.
    pub fn player_permission_level(&self, player: PlayerRef) -> Option<u8> {
        let read = self
            .player_handle_read(player)
            .or_else(|| self.online_player_record(player).map(PlayerRead::Guest))?;
        Some(match read {
            PlayerRead::Guest(g) => g.permission,
            // The command being executed keeps its source snapshot in `ui.registries`.
            // A live handle instead sees the same permission as the outgoing tree.
            PlayerRead::Acting { .. } => match self.server.lan_acting {
                Some(conn) => self.command_guest(conn)?.permission,
                None if self.common.allow_commands() => 4,
                None => 0,
            },
        })
    }
    /// `Player.hasInfiniteMaterials`: the `instabuild` ability, which
    /// creative mode gives.
    pub(in crate::game) fn player_has_infinite_materials(&self, player: PlayerRef) -> Option<bool> {
        self.player_is_creative(player)
    }
    /// `Player.canUseGameMasterBlocks`: creative and a permission level of
    /// at least 2, as command blocks, structure blocks and jigsaws ask.
    pub(in crate::game) fn player_can_use_game_master_blocks(
        &self,
        player: PlayerRef,
    ) -> Option<bool> {
        let creative = self.player_is_creative(player)?;
        Some(creative && self.player_permission_level(player)? >= 2)
    }
    /// `ServerPlayerGameMode.getPreviousGameModeForPlayer`: the mode the
    /// player last left (`previousPlayerGameType`, saved with the player), or
    /// `Some(None)` when they never changed mode. None for a stale or
    /// unavailable player.
    pub fn player_previous_game_mode(&self, player: PlayerRef) -> Option<Option<GameMode>> {
        Some(match self.player_handle_read(player)? {
            // The acting player's own: a guest's turn swaps theirs in.
            PlayerRead::Acting { .. } => self.common.previous_mode,
            PlayerRead::Guest(guest) => guest.previous_game_mode.and_then(GameMode::from_id),
        })
    }
    /// `Player.mayUseItemAt(pos, face, stack)`: the game's own check for an
    /// item used on a block (`Game::may_use_item_at`), with the player's
    /// `mayBuild` as the handle reads it. None for a stale or unavailable
    /// player, and for a guest in another level than the one in place, whose
    /// blocks it can't test.
    pub(in crate::game) fn player_may_use_item_at(
        &self,
        player: PlayerRef,
        pos: [i32; 3],
        face: BlockFace,
        stack: &ItemStack,
    ) -> Option<bool> {
        if let PlayerRead::Guest(guest) = self.player_handle_read(player)?
            && guest.dimension != self.common.dim.id()
        {
            return None;
        }
        let may_build = self.player_may_build(player)?;
        Some(self.may_use_item_at(may_build, pos, block_face_direction(face), stack))
    }
    /// `ServerPlayer.canHarmPlayer(other)`: the `pvp` rule and the two
    /// players' teams, as an attack, an arrow or a trident asks them
    /// (`player_may_harm`). None when either handle is stale or unavailable.
    pub(in crate::game) fn player_can_harm(
        &self,
        player: PlayerRef,
        other: PlayerRef,
    ) -> Option<bool> {
        self.player_handle_read(player)?;
        self.player_handle_read(other)?;
        let attacker = self.native_target_team(self.native_entity_target(player.entity())?)?;
        let victim = self.native_target_team(self.native_entity_target(other.entity())?)?;
        Some(self.player_may_harm(
            player.entity().id() as i32,
            other.entity().id() as i32,
            attacker,
            victim,
        ))
    }
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
