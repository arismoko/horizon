//! Where a portal leads, asked through a level handle (`LevelRef::portal_destination`,
//! `LevelRef::find_or_create_portal_exit`). Both read the level in place and the parked
//! levels only: a level that is not open, or a chunk the search reads that is not loaded,
//! is `ChunksNotLoaded` (vanilla loads them), and nothing is read from the save. The search is
//! the one dimension travel runs, over a side of it (`PortalSide`).
use crate::game::ServerGame;
use mod_api::{LevelRef, PortalAngle, PortalError, PortalExit, PortalRotation, PortalShape};
use sunrise_core::{geometry::Vec3, position::BlockPos};
use sunrise_level::containers::BlockEntity;
use sunrise_world::level::portal_travel::FoundRectangle;
use {
    crate::game::nether_portal::Plan, crate::game::nether_portal::PortalSide,
    crate::game::nether_portal::end_portal_rotation, sunrise_gameplay_common::game::Dim,
    sunrise_gameplay_common::game::end_state::END_SPAWN,
};

/// How far `largest_rectangle_around` reads from a block: the portal's limit in both directions.
const RECTANGLE: i32 = 21;
/// Blocks past a portal that its collision-free arrival reads (an entity box over 4 blocks is
/// not moved).
const COLLISION: i32 = 5;

/// What a probe of a portal block found.
enum Probe {
    /// An End portal or gateway: the answer.
    Fixed(Dim, Vec3, PortalRotation),
    /// A nether portal: the plan, and the exit portal when one stands.
    Nether(Plan, Option<FoundRectangle>),
}

impl ServerGame<'_> {
    fn portal_error(error: mod_api::LevelReadError) -> PortalError {
        match error {
            mod_api::LevelReadError::Stale => PortalError::LevelMissing,
            _ => PortalError::Unavailable,
        }
    }

    /// Every chunk holding a block within `reach` blocks of column (`x`, `z`) is loaded in `side`.
    fn blocks_loaded(side: &PortalSide<'_>, x: i32, z: i32, reach: i32) -> bool {
        // `x` and `z` are a mod's: a sum past the ends of `i32` stops at them, in a chunk that
        // is not loaded.
        let (min_x, max_x) = (x.saturating_sub(reach) >> 4, x.saturating_add(reach) >> 4);
        let (min_z, max_z) = (z.saturating_sub(reach) >> 4, z.saturating_add(reach) >> 4);
        (min_z..=max_z)
            .all(|cz| (min_x..=max_x).all(|cx| side.world.chunks.contains_key(&(cx, cz))))
    }

    /// Every chunk the search of `radius` around `at` reads is loaded in `side`.
    fn portal_chunks_loaded(side: &PortalSide<'_>, at: BlockPos, radius: i32) -> bool {
        PortalSide::search_chunks(at.x >> 4, at.z >> 4, radius)
            .all(|chunk| side.world.chunks.contains_key(&chunk))
    }

    /// The portal block at `pos` of `level` and what it leads to, found without changing
    /// anything.
    fn probe_portal(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        size: [f64; 2],
    ) -> Result<Probe, PortalError> {
        // The size is validated once, here: a width or height a float cannot hold, or a
        // negative one, would make the placement arithmetic non-finite.
        if !size
            .iter()
            .all(|v| v.is_finite() && (0.0..=f64::from(f32::MAX)).contains(v))
        {
            return Err(PortalError::InvalidSize);
        }
        self.native_level_world_result(level)
            .map_err(Self::portal_error)?;
        let source = self
            .portal_side(Dim::from_id(level.dimension_id()))
            .ok_or(PortalError::LevelMissing)?;
        let [x, y, z] = pos;
        // `largest_rectangle_around` reads up to `RECTANGLE` blocks each way around the entry.
        if !Self::blocks_loaded(&source, x, z, RECTANGLE) {
            return Err(PortalError::ChunksNotLoaded);
        }
        let state = source.world.block(x, y, z);
        let name = &*self.common.res.facts().get(state).name;
        if self.common.res.behaviours().portal_axis(state).is_some() {
            let entry = BlockPos::new(x, y, z);
            let at = Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5);
            let plan =
                self.portal_plan_from(&source, entry, at, size[0] as f32, size[1] as f32, true);
            if !self.may_enter(plan.dim) {
                return Err(PortalError::EnteringNotAllowed);
            }
            let target = self
                .portal_side(plan.dim)
                .filter(|target| {
                    Self::portal_chunks_loaded(
                        target,
                        plan.approximate,
                        PortalSide::search_radius(plan.dim == Dim::Nether),
                    )
                })
                .ok_or(PortalError::ChunksNotLoaded)?;
            let found = self.find_portal_in(&target, None, &plan);
            // The exit rectangle and the collision-free placement read around the portal found.
            if let Some((at, _)) = found
                && !Self::blocks_loaded(&target, at.x, at.z, RECTANGLE + COLLISION)
            {
                return Err(PortalError::ChunksNotLoaded);
            }
            return Ok(Probe::Nether(plan, found.map(|(_, rectangle)| rectangle)));
        }
        match name {
            "minecraft:end_portal" => self.probe_end_portal(&source),
            "minecraft:end_gateway" => self.probe_end_gateway(&source, pos),
            _ => Err(PortalError::NotAPortal),
        }
    }

    /// `EndPortalBlock.getPortalDestination` for an entity other than a player, without the
    /// platform: the End's spawn point, or out of the End the world spawn in its level.
    fn probe_end_portal(&self, source: &PortalSide<'_>) -> Result<Probe, PortalError> {
        if source.dim != Dim::End {
            let [x, y, z] = END_SPAWN;
            self.portal_side(Dim::End)
                .ok_or(PortalError::ChunksNotLoaded)?;
            // Facing west, whatever the entity faced.
            let [yaw, pitch] =
                end_portal_rotation(true, [0.0; 2], self.common.level.world_spawn_angle);
            return Ok(Probe::Fixed(
                Dim::End,
                Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5),
                PortalRotation::new(PortalAngle::Set(yaw), PortalAngle::Turn(pitch)),
            ));
        }
        let [x, _, z] = self.common.level.world_spawn.unwrap_or_else(|| {
            self.common.spawn.map_or([0, 64, 0], |s| {
                [s.x.floor() as i32, s.y.floor() as i32, s.z.floor() as i32]
            })
        });
        let level = self.world_spawn_level();
        let side = self
            .portal_side(level)
            .filter(|side| side.world.is_loaded_at(x, 64, z))
            .ok_or(PortalError::ChunksNotLoaded)?;
        let y = self.common.motion_top_in(side.world, level, x, z);
        // Turned by the world spawn's angle.
        let [yaw, pitch] =
            end_portal_rotation(false, [0.0; 2], self.common.level.world_spawn_angle);
        Ok(Probe::Fixed(
            level,
            Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5),
            PortalRotation::new(PortalAngle::Turn(yaw), PortalAngle::Turn(pitch)),
        ))
    }

    /// `TheEndGatewayBlockEntity.getPortalPosition` of a gateway that already has its exit.
    fn probe_end_gateway(
        &self,
        source: &PortalSide<'_>,
        pos: [i32; 3],
    ) -> Result<Probe, PortalError> {
        let Some(BlockEntity::EndGateway(gateway)) =
            source.world.block_entities.get(&(pos[0], pos[1], pos[2]))
        else {
            return Err(PortalError::NotAPortal);
        };
        let exit = gateway.exit_portal.ok_or(PortalError::NoExit)?;
        let target = if gateway.exact_teleport {
            exit
        } else {
            let (cx, cz) = (exit[0] >> 4, exit[2] >> 4);
            let loaded = (cz - 1..=cz + 1)
                .all(|z| (cx - 1..=cx + 1).all(|x| source.world.chunks.contains_key(&(x, z))));
            if !loaded {
                return Err(PortalError::ChunksNotLoaded);
            }
            let p = self.tallest_block_in(
                source.world,
                [exit[0], exit[1].saturating_add(2), exit[2]],
                5,
                false,
            );
            [p[0], p[1].saturating_add(1), p[2]]
        };
        Ok(Probe::Fixed(
            source.dim,
            sunrise_sim::end_gateway::bottom_center(target),
            PortalRotation::new(PortalAngle::Turn(0.0), PortalAngle::Turn(0.0)),
        ))
    }

    /// The answer for the destination `exit` of `plan` in the loaded level of `plan.dim`.
    fn nether_exit(
        &self,
        level: LevelRef,
        plan: &Plan,
        exit: FoundRectangle,
    ) -> Result<PortalExit, PortalError> {
        let side = self.portal_side(plan.dim).ok_or(PortalError::Unavailable)?;
        let (at, yaw_turn) = self.arrival_in(&side, plan, exit);
        let corner = exit.min_corner;
        let x_axis = self
            .common
            .res
            .behaviours()
            .portal_axis(side.world.block(corner.x, corner.y, corner.z))
            .unwrap_or(true);
        let shape = PortalShape::from(mod_api::__private::built::PortalShape {
            x_axis,
            bottom_left: [corner.x, corner.y, corner.z],
            width: exit.axis1_size,
            height: exit.axis2_size,
        });
        Ok(PortalExit::new(
            self.level_ref(level.epoch(), plan.dim.key()),
            [at.x, at.y, at.z],
            PortalRotation::new(PortalAngle::Turn(yaw_turn), PortalAngle::Turn(0.0)),
            Some(shape),
        ))
    }

    fn fixed_exit(
        &self,
        level: LevelRef,
        dim: Dim,
        at: Vec3,
        rotation: PortalRotation,
    ) -> PortalExit {
        PortalExit::new(
            self.level_ref(level.epoch(), dim.key()),
            [at.x, at.y, at.z],
            rotation,
            None,
        )
    }

    /// Where an entity of `size` arrives through the portal at `portal_pos`, changing nothing
    /// (`getPortalDestination`). Vanilla places the exit from the two portals' axes
    /// alone.
    pub fn level_portal_destination(
        &self,
        level: LevelRef,
        portal_pos: [i32; 3],
        size: [f64; 2],
    ) -> Result<PortalExit, PortalError> {
        match self.probe_portal(level, portal_pos, size)? {
            Probe::Fixed(dim, at, rotation) => Ok(self.fixed_exit(level, dim, at, rotation)),
            Probe::Nether(plan, Some(exit)) => self.nether_exit(level, &plan, exit),
            Probe::Nether(_, None) => Err(PortalError::NoExit),
        }
    }

    /// [`Self::level_portal_destination`], building the exit portal when none stands
    /// (`PortalForcer.createPortal`) and telling `portal_create` as travel does.
    pub fn level_find_or_create_portal_exit(
        &mut self,
        level: LevelRef,
        portal_pos: [i32; 3],
        size: [f64; 2],
    ) -> Result<PortalExit, PortalError> {
        if !self.common.transactions.is_empty() {
            return Err(PortalError::InTransaction);
        }
        let plan = match self.probe_portal(level, portal_pos, size)? {
            Probe::Nether(plan, None) => plan,
            _ => return self.level_portal_destination(level, portal_pos, size),
        };
        // Building works on the level in place: the destination goes in place for it, and the
        // level that was in place comes back.
        let source = self.common.dim;
        self.enter_level(plan.dim)
            .map_err(|_| PortalError::LevelMissing)?;
        let built = self.create_exit_portal(&plan);
        let answer = built.map(|exit| self.nether_exit(level, &plan, exit));
        // The source level was parked by the swap above, so entering it again only swaps back:
        // opening a level is the one way `enter_level` fails.
        let back = self.enter_level(source);
        debug_assert!(back.is_ok(), "the parked source level re-enters");
        answer.unwrap_or(Err(PortalError::NoExit))
    }
}

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
