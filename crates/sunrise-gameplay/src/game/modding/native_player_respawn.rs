//! A server mod's respawn of a dead player (`PlayerRef::respawn`): the Respawn button, pressed
//! for them; and where a player would respawn (`PlayerRef::respawn_target`), without it.
use super::{Dim, respawn_anchor::RespawnConfig};
use mod_api::{PlayerRef, RespawnRefusal, RespawnTarget, RespawnTargetUnavailable};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::native_server::PlayerRead;

impl crate::game::ServerGame<'_> {
    /// The Respawn button for a dead player (`handleClientCommand`'s `PERFORM_RESPAWN`), outside
    /// every player's hook. Each refusal is a variant of [`RespawnRefusal`], checked in its order.
    pub(in crate::game) fn player_respawn(
        &mut self,
        player: PlayerRef,
    ) -> Result<(), RespawnRefusal> {
        if !self.common.is_logical_server() {
            return Err(RespawnRefusal::Client);
        }
        if self.server.lan_acting.is_some() || !self.common.player_state.cloning_players.is_empty()
        {
            return Err(RespawnRefusal::InPlayerHook);
        }
        let (conn, has_health, won_game) = match self.player_handle_read(player) {
            None => {
                let guest = self
                    .online_player_record(player)
                    .filter(|g| g.respawning)
                    .ok_or(RespawnRefusal::Stale)?;
                (guest.conn, guest.health > 0.0, guest.won_game)
            }
            // The world's own player has no connection: it respawns through its own screen.
            Some(PlayerRead::Acting { .. }) => return Err(RespawnRefusal::NoConnection),
            Some(PlayerRead::Guest(guest)) if guest.is_fake() => {
                return Err(RespawnRefusal::NoConnection);
            }
            Some(PlayerRead::Guest(guest)) => (guest.conn, guest.health > 0.0, guest.won_game),
        };
        // `handleClientCommand`: the credits button respawns even a living player.
        if has_health && !won_game {
            return Err(RespawnRefusal::Alive);
        }
        self.guest_respawn(conn);
        Ok(())
    }

    /// `ServerPlayer.findRespawnPositionAndUseSpawnBlock(false, DO_NOTHING)`, as `EndPortalBlock`
    /// asks it: the bed or charged anchor of the player's point, in that level, with no charge
    /// used up; else the world spawn's column, `missing_respawn_block` when the point gave
    /// nothing (`TeleportTransition.missingRespawnBlock`), not when there was no point or its
    /// level is gone (`TeleportTransition.createDefault`). Each level it reads is put in place
    /// for the read, and the caller's back before it returns.
    pub(in crate::game) fn player_respawn_target(
        &mut self,
        player: PlayerRef,
    ) -> Result<RespawnTarget, RespawnTargetUnavailable> {
        let saved = match self
            .player_handle_read(player)
            .ok_or(RespawnTargetUnavailable::Stale)?
        {
            PlayerRead::Acting { .. } => self.server.respawn_config.map(|c| c.saved()),
            PlayerRead::Guest(guest) => guest.respawn_elsewhere.clone(),
        };
        let config = saved.as_ref().and_then(RespawnConfig::from_saved);
        let back = self.common.dim;
        let found = self.in_level_for_read(back, config.map(|c| c.dim), |game| {
            let c = config.expect("a level only with a point");
            Ok(game.respawn_point_target(c))
        });
        let missing = match found {
            Some(Ok(Some(target))) => return Ok(target),
            Some(Err(refusal)) => return Err(refusal),
            Some(Ok(None)) => true,
            None => false,
        };
        let spawn = self.world_spawn_level();
        self.in_level_for_read(back, Some(spawn), |game| {
            game.world_spawn_target(spawn, missing)
        })
        .expect("a level is named")
    }

    /// `read` with `dim` in place, then the level `back` again; None for no level. Refused with
    /// `InPlayerHook` when `dim` is not in place and a player's state is swapped in, and with
    /// `Stale` when a level cannot be put in place.
    fn in_level_for_read<T>(
        &mut self,
        back: Dim,
        dim: Option<Dim>,
        read: impl FnOnce(&mut Self) -> Result<T, RespawnTargetUnavailable>,
    ) -> Option<Result<T, RespawnTargetUnavailable>> {
        let dim = dim?;
        if dim != self.common.dim
            && (self.server.lan_acting.is_some()
                || !self.common.player_state.cloning_players.is_empty())
        {
            return Some(Err(RespawnTargetUnavailable::InPlayerHook));
        }
        if let Err(e) = self.enter_level(dim) {
            log::error!("could not open {} for a respawn target: {e}", dim.id());
            return Some(Err(RespawnTargetUnavailable::Stale));
        }
        let answer = read(self);
        if let Err(e) = self.enter_level(back) {
            log::error!("could not reopen {} after a respawn target: {e}", back.id());
        }
        Some(answer)
    }

    /// `ServerPlayer.findRespawnAndUseSpawnBlock(level, config, false)` with the point's level in
    /// place: `Level.getBlockState` loads its column to FULL first (not outside the level's
    /// height, where it reads air without loading).
    fn respawn_point_target(&mut self, c: RespawnConfig) -> Option<RespawnTarget> {
        let [x, y, z] = c.pos;
        if self.common.world.shape.contains_y(y)
            && let Err(e) = self.load_full_chunk(None, (x >> 4, z >> 4))
        {
            log::warn!(
                "the respawn point's chunk at {:?} did not load: {e:?}",
                c.pos
            );
        }
        let found = self.find_respawn_and_use(c, false).unwrap_or_else(|e| {
            log::error!("reading the respawn point at {:?} failed: {e}", c.pos);
            None
        });
        let (at, yaw, pitch) = found?;
        Some(self.respawn_target(c.dim, at, [yaw, pitch], false))
    }

    /// `TeleportTransition.missingRespawnBlock` or `createDefault` with the world spawn's level
    /// in place: `findAdjustedSharedSpawnPos` (the spawn search around the world spawn, which
    /// holds a `SPAWN_SEARCH` ticket on the chunk it waits for) and the world spawn's angle.
    fn world_spawn_target(
        &mut self,
        spawn: Dim,
        missing: bool,
    ) -> Result<RespawnTarget, RespawnTargetUnavailable> {
        let [x, y, z] = self.world_spawn_position();
        let at = if self.common.level.world_spawn.is_some() {
            match self.search_spawn([x, y, z].map(|v| v.floor() as i32), &mut None) {
                Ok(at) => at,
                Err(chunk) => {
                    self.server
                        .tickets
                        .0
                        .add_spawn(chunk, sunrise_sim::ticket_levels::SpawnTicketKind::Search);
                    return Err(RespawnTargetUnavailable::Loading);
                }
            }
        } else {
            Vec3::new(x, y, z)
        };
        let angle = self.common.level.world_spawn_angle;
        Ok(self.respawn_target(spawn, at, angle, missing))
    }

    fn respawn_target(
        &self,
        dim: Dim,
        at: Vec3,
        [yaw, pitch]: [f32; 2],
        missing_respawn_block: bool,
    ) -> RespawnTarget {
        RespawnTarget::from(mod_api::__private::built::RespawnTarget {
            level: self
                .level(dim.id())
                .expect("the level was in place for the read"),
            position: [at.x, at.y, at.z],
            yaw,
            pitch,
            missing_respawn_block,
        })
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
