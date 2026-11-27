//! A client mod's reads of the levels the server opened for viewing (#3981, gap 10), as Immersive
//! Portals' `ClientWorldLoader.getWorld(dim)` hands out the far `ClientLevel`: on a client,
//! `host.level(dim)` also names each level the server keeps open on `sunrise:levels`, and
//! `block_state`, `brightness`, `clip`, `visit_block_collision(s)` and `has_collisions` read its world
//! (`viewed_worlds`), as do the reads built on `level_block_state`: `block`, `block_property`
//! and the both-sided fluid reads (`fluid_shape`, `fluid_height`, `fluid_height_for_camera`,
//! `fluid_can_be_replaced_with`). `get_enchant_power_bonus` uses the viewed level as context.
//! A read never writes and never loads a column: direct block reads return None for missing
//! columns, `clip` reads missing columns and out-of-height cells as air
//! (`ClientChunkCache.getChunk(..., true)`), and the collision scan skips missing columns.
//! The handle answers None after the server closes the level or a Login, Respawn or new
//! configuration phase drops every viewed world. A `LevelRef` names a level and the client
//! level's epoch, not one open of the view. A close and a later open within the same epoch make
//! the old handle read again. A Login, reconfiguration or Respawn into another level starts a
//! new epoch, so the old handle stays stale and a mod asks `host.level` again (a same-level
//! Respawn keeps the epoch). Writes, server-only reads, the other reads and the entity reads
//! (until step 9d) refuse it.
use crate::game::ClientGame;
use mod_api::{DimensionKey, LevelReadError, LevelRef};
use sunrise_level::world::World;

impl ClientGame<'_> {
    /// Resolve the constructor's level by name during Load; otherwise the installed level or
    /// a view the server opened. The constructor and installed level have distinct epochs.
    pub(in crate::game) fn client_level_by_name(&self, dimension: &str) -> Option<LevelRef> {
        let remote = self.client.remote.as_ref().filter(|r| r.logged_in)?;
        if let Some(level) = &remote.constructing_level
            && level.handle.dimension_id() == dimension
        {
            return Some(level.handle);
        }
        let key: DimensionKey = if remote.level == dimension {
            remote.level_key
        } else {
            remote.viewed.key_of(dimension)?
        };
        Some(LevelRef::new(self.common.native_lifecycle.epoch()?, key))
    }
    /// On a client, the world a handle names now: the player's own, or a viewed level's while
    /// the server keeps it open. None before login, for a stale handle and for any other level.
    pub(in crate::game) fn client_view_world(&self, level: LevelRef) -> Option<&World> {
        self.client_view_world_result(level).ok()
    }

    pub(in crate::game) fn client_view_world_result(
        &self,
        level: LevelRef,
    ) -> Result<&World, LevelReadError> {
        let remote = self
            .client
            .remote
            .as_ref()
            .filter(|r| r.logged_in)
            .ok_or(LevelReadError::Unavailable)?;
        if let Some(constructing) = remote.constructing(level) {
            return Ok(&constructing.world);
        }
        if self.common.native_lifecycle.epoch() != Some(level.epoch()) {
            return Err(LevelReadError::Stale);
        }
        if level.dimension() == remote.level_key {
            return Ok(&self.common.world);
        }
        remote
            .viewed
            .world_of(level.dimension())
            .ok_or(LevelReadError::Stale)
    }
    /// On a client, the handle the player's own level answers to now (its key and the client
    /// level's epoch): what a world view naming a level is compared with, resolved with the
    /// same rules as `client_view_world`. None before login.
    pub(super) fn client_own_level(&self) -> Option<LevelRef> {
        let remote = self.client.remote.as_ref().filter(|r| r.logged_in)?;
        Some(LevelRef::new(
            self.common.native_lifecycle.epoch()?,
            remote.level_key,
        ))
    }
    /// The frusta of the frame's camera `[fov_degrees, yaw, pitch]` and its world views, which are
    /// asked for here: each view's level handle is resolved once, first, against the player's own.
    pub(in crate::game::client_side) fn world_view_frustum(
        &mut self,
        camera: glam::DVec3,
        [fov_degrees, yaw, pitch, roll]: [f32; 4],
        view_transform: glam::Mat4,
        depth_far: f32,
    ) -> Option<sunrise_client::gfx::JoinedFrusta> {
        let own_level = self.client_own_level();
        let renderer = self.client.renderer.0.as_mut()?;
        renderer.set_world_view_own_level(own_level);
        Some(renderer.camera_frustum(
            camera,
            fov_degrees,
            yaw,
            pitch,
            roll,
            view_transform,
            depth_far,
        ))
    }
    pub(in crate::game) fn client_level_view_world(&self, level: LevelRef) -> Option<&World> {
        self.client_view_world(level)
    }

    pub(in crate::game) fn client_collision_scan_world(
        &self,
        level: LevelRef,
    ) -> Option<(&World, bool)> {
        self.client_view_world(level).map(|w| (w, true))
    }

    pub(in crate::game) fn client_level_handle_current(&self, level: LevelRef) -> bool {
        self.common.native_lifecycle.epoch() == Some(level.epoch()) && level.generation() == 0
    }

    pub(in crate::game) fn client_level_datapack_set(
        &self,
        level: LevelRef,
    ) -> Option<std::sync::Arc<mod_api::DatapackRegistrySet>> {
        let remote = self.client.remote.as_ref()?;
        if self.client_own_level() != Some(level) {
            remote.constructing(level)?;
        }
        remote.datapacks.received().cloned()
    }
}
