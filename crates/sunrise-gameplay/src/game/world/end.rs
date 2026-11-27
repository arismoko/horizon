//! The End: End portal frames filled with eyes of ender, End portal and End
//! gateway travel, the obsidian arrival platform, the exit portal and the End
//! sky. The dragon fight itself lives in game/mobs/dragon.rs: the podium is
//! inactive until the dragon dies.
use super::Dim;
use sunrise_core::{
    geometry::Vec3,
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::end_state::{Arriving, END_SPAWN, GatewayArrival, GatewayJob};
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
#[cfg(test)]
use sunrise_sim::projectiles::Projectile;
use sunrise_sim::{mobs::Who, projectiles::ProjectileKind, world_spawn::PlayerSpawnFinder};

/// `TicketType.PORTAL`'s timeout, in ticks.
pub(in crate::game) const PORTAL_TICKET: u32 = 300;

/// EnderDragonFight gateway ring radius and height.
pub(in crate::game) const GATEWAY_RADIUS: f64 = 96.0;
pub(in crate::game) const GATEWAY_Y: i32 = 75;

/// The end of `getPortalPosition`: the exit itself when the teleport is exact,
/// else the spot [`Game::resolve_gateway`] finds around it (`findExitPosition`).
pub(in crate::game::server_side) fn exit_target(
    exit: [i32; 3],
    exact: bool,
    find: impl FnOnce([i32; 3]) -> [i32; 3],
) -> [i32; 3] {
    if exact { exit } else { find(exit) }
}

pub(in crate::game::server_side) fn fallback_spawn(
    own: Option<Vec3>,
    world: Option<[i32; 3]>,
) -> Vec3 {
    let s = own.unwrap_or_else(|| {
        let [x, y, z] = world.unwrap_or([0, 64, 0]);
        Vec3::new(f64::from(x), f64::from(y), f64::from(z))
    });
    Vec3::new(s.x.floor() + 0.5, s.y, s.z.floor() + 0.5)
}

impl crate::game::ServerGame<'_> {
    /// `EyeOfEnder.tick` for the summoned and loaded eyes: they fly on by
    /// their motion and shatter on tick 81, dropping nothing.
    pub(in crate::game::server_side) fn tick_summoned_eyes(&mut self) {
        let ticking = self.entity_ticking_area();
        let mut shattered = Vec::new();
        self.server.others.0.retain_mut(|o| {
            let Some(eye) = o.eye.as_mut() else {
                return true;
            };
            // ServerLevel.tick: only entities in entity-ticking chunks tick.
            if !ticking(o.position.x.floor() as i32, o.position.z.floor() as i32) {
                return true;
            }
            // Entity.baseTick: the eye burns down; its hurt does nothing.
            if eye.fire_ticks > 0 {
                eye.fire_ticks -= 1;
            }
            eye.synced_fire = eye.fire_ticks > 0 || eye.has_visual_fire;
            let m = eye.motion;
            o.position = Vec3::new(o.position.x + m.x, o.position.y + m.y, o.position.z + m.z);
            eye.life += 1;
            if eye.life <= 80 {
                return true;
            }
            shattered.push([o.position.x, o.position.y, o.position.z]);
            false
        });
        for at in shattered {
            self.server_play_sound("minecraft:entity.ender_eye.death", at, 1.0, 1.0);
            self.server_level_event_at(2003, at.map(|v| v.floor() as i32), 0);
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// EnderEyeItem.useOn: fill an empty End portal frame, then light the
    /// portal when the ring of twelve filled frames is complete.
    pub fn use_ender_eye(&mut self, stack: &ItemStack, use_on_block: bool) -> Result<bool> {
        if &*stack.item != "minecraft:ender_eye" {
            return Ok(false);
        }
        let registry = self.common.pack.assets.registry;
        let target = self.pick().and_then(|hit| {
            let state =
                registry.state(self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))?;
            (state.name == "minecraft:end_portal_frame").then(|| (hit, state.clone()))
        });
        let Some((hit, state)) = target else {
            // EnderEyeItem.useOn passes off a frame: a guest's UseItem throws it.
            if matches!(self.server.guest_use, Some(super::GuestUse::OnBlock { .. })) {
                return Ok(false);
            }
            // EnderEyeItem.use: startUsingItem (getUseDuration 0, so the use
            // lasts until the button lets go), then the eye is thrown towards
            // the nearest stronghold if there is one (CONSUME without).
            self.start_using_item();
            self.throw_ender_eye();
            return Ok(true);
        };
        if !use_on_block {
            // EnderEyeItem.use passes on a frame; only useOn can insert an eye.
            return Ok(false);
        }
        if state.properties.get("eye").map(String::as_str) == Some("true") {
            // Both useOn and use pass on a filled frame.
            return Ok(true);
        }
        let mut filled = state;
        filled.properties.insert("eye".into(), "true".into());
        let id = registry.resolve(&filled)?;
        self.level_set(hit.pos, id)?;
        // Block.pushEntitiesUp: the eye raises the frame's top from 13/16 to a full block.
        if let Some(p) = self.common.player.as_mut() {
            let top = f64::from(hit.pos[1]) + 1.0;
            let w = f64::from(p.dimensions.width) / 2.0;
            let over = |c: f64, b: i32| c + w > f64::from(b) && c - w < f64::from(b) + 1.0;
            if over(p.position.x, hit.pos[0])
                && over(p.position.z, hit.pos[2])
                && p.position.y >= top - 0.25
                && p.position.y < top
            {
                p.position.y = top;
            }
        }
        self.award_stat("used", "minecraft:ender_eye", 1);
        self.consume_held();
        self.swing_arm();
        // EnderEyeItem.useOn emits this event for all nearby players.
        self.server_level_event_at(1503, hit.pos, 0);
        if let Some(c) = self.portal_ring_center(hit.pos) {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let p = [c[0] + dx, c[1], c[2] + dz];
                    let at = BlockPos::new(p[0], p[1], p[2]);
                    self.with_level(|b, level, ticks| {
                        b.destroy_block(level, ticks, at, true);
                    });
                    self.set_state_named(p, "minecraft:end_portal", &[])?;
                }
            }
            self.server_process_level_events()?;
            // EnderEyeItem.useOn: global level event 1038 (END_PORTAL_SPAWN),
            // heard by every player (vanilla places it at the frame's corner
            // + (1, 0, 1); only its direction matters).
            self.global_level_event(1038, c, 0);
        }
        Ok(true)
    }

    /// EndPortalFrameBlock.getOrCreatePortalShape matched around `frame`: a 5×5
    /// ring (corners free) of filled frames, all facing the centre or all
    /// facing away from it. Returns the centre of the 3×3 opening.
    pub(in crate::game::server_side) fn portal_ring_center(
        &self,
        frame: [i32; 3],
    ) -> Option<[i32; 3]> {
        let registry = self.common.pack.assets.registry;
        // (offset from the centre, facing when the frames face the centre)
        let mut ring = Vec::with_capacity(12);
        for i in -1..=1 {
            ring.push(([i, -2], "south"));
            ring.push(([i, 2], "north"));
            ring.push(([-2, i], "east"));
            ring.push(([2, i], "west"));
        }
        let opposite = |f: &str| match f {
            "south" => "north",
            "north" => "south",
            "east" => "west",
            _ => "east",
        };
        for &([ox, oz], _) in &ring {
            let c = [frame[0] - ox, frame[1], frame[2] - oz];
            for inward in [true, false] {
                let ok = ring.iter().all(|&([dx, dz], facing)| {
                    let id = self.common.world.block(c[0] + dx, c[1], c[2] + dz);
                    let Some(s) = registry.state(id) else {
                        return false;
                    };
                    let want = if inward { facing } else { opposite(facing) };
                    s.name == "minecraft:end_portal_frame"
                        && s.properties.get("eye").map(String::as_str) == Some("true")
                        && s.properties.get("facing").map(String::as_str) == Some(want)
                });
                if ok {
                    return Some(c);
                }
            }
        }
        None
    }

    /// End portals, End gateways and the exit portal, once per tick.
    pub(in crate::game::server_side) fn tick_end(&mut self) -> Result<()> {
        self.common.end.gateway_cooldown = self.common.end.gateway_cooldown.saturating_sub(1);
        if self.dimension_type().has_ender_dragon_fight {
            // The fireballs fly with the other projectiles; the breath
            // clouds tick here, once.
            self.tick_breath();
        }
        // The spikes' crystals (placed above) join before they light a fire:
        // a mod that cancels one keeps its fire out too.
        self.native_joins_flush(false, 0);
        self.tick_crystals();
        self.clear_crystal_fire();
        self.tick_eyes();
        if self.dimension_type().has_ender_dragon_fight {
            self.ensure_exit_portal()?;
        }
        Ok(())
    }

    /// Whether a `width` by `height` box at `position` is inside an End
    /// portal (its entity-inside shape), and the End gateway it touches.
    pub(in crate::game::server_side) fn inside_end_portals(
        &self,
        position: Vec3,
        width: f64,
        height: f64,
    ) -> (bool, Option<[i32; 3]>) {
        let w = width / 2.0;
        let lo = [position.x - w, position.y, position.z - w];
        let hi = [position.x + w, position.y + height, position.z + w];
        let mut portal = false;
        let mut gateway = None;
        for x in lo[0].floor() as i32..=hi[0].floor() as i32 {
            for y in lo[1].floor() as i32..=hi[1].floor() as i32 {
                for z in lo[2].floor() as i32..=hi[2].floor() as i32 {
                    let name = self.block_name([x, y, z]);
                    // EndPortalBlock entity-inside shape: y 6/16..12/16.
                    if &*name == "minecraft:end_portal"
                        && lo[1] < f64::from(y) + 0.75
                        && hi[1] > f64::from(y) + 0.375
                    {
                        portal = true;
                    } else if &*name == "minecraft:end_gateway" && self.common.dim == Dim::End {
                        gateway = Some([x, y, z]);
                    }
                }
            }
        }
        (portal, gateway)
    }

    /// EndPortalBlock.getPortalDestination for the pending enter_dimension: into
    /// the End when `entering`, else out of it to `target`, the respawn point's
    /// level or the world spawn's (which may be the End itself).
    pub(in crate::game::server_side) fn end_destination(
        &self,
        entering: bool,
        target: Dim,
    ) -> Option<Vec3> {
        if !self.server.end_portal_travel {
            return None;
        }
        if entering {
            // Vec3.atBottomCenterOf(END_SPAWN_POINT), one block lower for players.
            return Some(Vec3::new(
                f64::from(END_SPAWN[0]) + 0.5,
                f64::from(END_SPAWN[1] - 1),
                f64::from(END_SPAWN[2]) + 0.5,
            ));
        }
        // Leaving the End: the respawn position, a bed's found once it has
        // loaded (findRespawnPositionAndUseSpawnBlock, not using it up).
        if let Some(c) = self.server.respawn_config.filter(|c| c.dim == target) {
            return Some(Vec3::new(
                f64::from(c.pos[0]) + 0.5,
                f64::from(c.pos[1]),
                f64::from(c.pos[2]) + 0.5,
            ));
        }
        Some(fallback_spawn(
            self.common.spawn,
            self.common.level.world_spawn,
        ))
    }

    /// Where an End portal's traveller lands once `target` has loaded, and
    /// the yaw and pitch it faces there (None keeps its own): entering, the
    /// platform below the End's spawn point, facing west; leaving, the bed or
    /// anchor facing it, else the world spawn's column in its level, facing
    /// the world spawn's angle (`TeleportTransition.createDefault`).
    /// None while a candidate of the world spawn's search loads (`finder`
    /// keeps it, see `search_spawn`).
    pub(in crate::game::server_side) fn end_arrival(
        &mut self,
        entering: bool,
        target: Vec3,
        finder: &mut Option<PlayerSpawnFinder>,
    ) -> Result<Option<(Vec3, [Option<f32>; 2])>> {
        if entering {
            // EndPlatformFeature.createEndPlatform below the spawn point.
            let origin = [END_SPAWN[0], END_SPAWN[1] - 1, END_SPAWN[2]];
            for dz in -2..=2 {
                for dx in -2..=2 {
                    for dy in -1..3 {
                        let p = [origin[0] + dx, origin[1] + dy, origin[2] + dz];
                        let name = if dy == -1 {
                            "minecraft:obsidian"
                        } else {
                            "minecraft:air"
                        };
                        if &*self.block_name(p) != name {
                            // Portal arrivals rebuild the platform with drops enabled.
                            self.with_level(|b, level, ticks| {
                                b.destroy_block(
                                    level,
                                    ticks,
                                    BlockPos::new(p[0], p[1], p[2]),
                                    true,
                                );
                            });
                            self.set_state_named(p, name, &[])?;
                        }
                    }
                }
            }
            self.native_portal_create("minecraft:the_end", origin);
            // Direction.WEST.toYRot(); the pitch is relative (Relative.X_ROT).
            return Ok(Some((target, [Some(90.0), None])));
        }
        // ServerPlayer.findRespawnPositionAndUseSpawnBlock: where the bed or
        // anchor puts the player, facing it; else onto the spawn column.
        let mut target = target;
        if let Some(c) = self
            .server
            .respawn_config
            .filter(|c| c.dim == self.common.dim)
        {
            if let Some((at, yaw, pitch)) = self.find_respawn_and_use(c, false)? {
                return Ok(Some((at, [Some(yaw), Some(pitch)])));
            }
            target = fallback_spawn(self.common.spawn, self.common.level.world_spawn);
        }
        // TeleportTransition.createDefault: the world spawn's yaw and pitch; a bed of
        // the player's own (`spawn`) keeps the way it faced.
        let [yaw, pitch] = self.common.level.world_spawn_angle;
        let facing = if self.common.spawn.is_some() {
            [None, None]
        } else {
            [Some(yaw), Some(pitch)]
        };
        Ok(match self.end_exit_landing(target, finder) {
            Ok(at) => Some((at, facing)),
            Err(chunk) => {
                self.server.spawn_wait.push(chunk);
                None
            }
        })
    }

    /// `TeleportTransition.findAdjustedSharedSpawnPos` on the shared spawn column;
    /// elsewhere, use the personal column's surface or its saved height if it has none.
    /// Returns the next chunk to load if the shared spawn search is still pending.
    pub(in crate::game::server_side) fn end_exit_landing(
        &self,
        target: Vec3,
        finder: &mut Option<PlayerSpawnFinder>,
    ) -> std::result::Result<Vec3, (i32, i32)> {
        let (x, z) = (target.x.floor() as i32, target.z.floor() as i32);
        if let Some(world) = self
            .common
            .level
            .world_spawn
            .filter(|w| w[0] == x && w[2] == z)
        {
            return self.search_spawn(world, finder);
        }
        let y = self.common.land_column(x, z).map_or(target.y, f64::from);
        Ok(Vec3::new(target.x, y, target.z))
    }

    /// EnderDragonFight without a dragon: once the centre of the main island
    /// is loaded, place the active exit portal (EndPodiumFeature) and the
    /// first gateway (spawnNewGateway) if the portal is not there yet. Like
    /// scanState and spawnExitPortal, only while the fight has no exit portal
    /// location: a podium that is later broken stays broken.
    pub(in crate::game::server_side) fn ensure_exit_portal(&mut self) -> Result<()> {
        if !(-1..=0).all(|cz| (-1..=0).all(|cx| self.server.streaming.is_lit(cx, cz)))
            || !self.fight_ready()
        {
            return Ok(());
        }
        let won = self.server.fight.previously_killed;
        let gateway = self.first_gateway();
        if self.exit_portal().is_none() {
            // findExitPortal remembers the location of a podium that is already there.
            if let Some(level) = self.podium_level() {
                self.set_exit_portal([0, level, 0]);
            }
        }
        if self.exit_portal().is_some() {
            // The gateway may have been out of range when it was placed.
            if won
                && self.common.world.loaded(gateway[0], gateway[2])
                && &*self.block_name(gateway) != "minecraft:end_gateway"
                && &*self.block_name([gateway[0], gateway[1] + 2, gateway[2]])
                    != "minecraft:bedrock"
            {
                self.place_gateway(gateway)?;
            }
            return Ok(());
        }
        // getHeightmapPos(MOTION_BLOCKING_NO_LEAVES, podium location).below()
        let mut y = (0..256)
            .rev()
            .find(|&y| {
                let f = self
                    .common
                    .res
                    .facts()
                    .get(self.common.world.block(0, y, 0));
                (f.full_collision || f.fluid != sunrise_items::blocks::Fluid::None)
                    && !f.name.ends_with("_leaves")
            })
            .unwrap_or(0);
        while &*self.block_name([0, y, 0]) == "minecraft:bedrock" && y > 63 {
            y -= 1;
        }
        let origin = [0, y.max(1), 0];
        self.set_exit_portal(origin);
        self.place_podium(origin, won)?;
        if won && self.common.world.loaded(gateway[0], gateway[2]) {
            self.place_gateway(gateway)?;
        }
        Ok(())
    }
    /// The portal level of the podium around the centre column, read from the
    /// bedrock that EndPodiumFeature places (rim at the portal level, disc
    /// below it, the torch column) so a broken column does not hide it.
    pub(in crate::game::server_side) fn podium_level(&self) -> Option<i32> {
        let mut offsets: Vec<[i32; 3]> = (0..4).map(|dy| [0, dy, 0]).collect();
        for z in -4..=4i32 {
            for x in -4..=4i32 {
                let flat = f64::from(x * x + z * z);
                if (6.25..12.25).contains(&flat) {
                    offsets.push([x, 0, z]);
                } else if flat + 1.0 < 6.25 {
                    offsets.push([x, -1, z]);
                }
            }
        }
        (1..253)
            .map(|y| {
                let score = offsets
                    .iter()
                    .filter(|o| &*self.block_name([o[0], y + o[1], o[2]]) == "minecraft:bedrock")
                    .count();
                (score, y)
            })
            .filter(|&(score, _)| score >= 3)
            .max()
            .map(|(_, y)| y)
    }
    /// `spawnExitPortal(true)` after the dragon dies.
    pub(in crate::game::server_side) fn activate_exit_portal(&mut self) -> Result<()> {
        let origin = match self.exit_portal() {
            Some(o) => o,
            None => {
                // The rim's level, found without assuming the centre column is intact.
                [0, self.podium_level().unwrap_or(61), 0]
            }
        };
        self.place_podium(origin, true)
    }
    pub(in crate::game::server_side) fn place_gateway_at(&mut self, pos: [i32; 3]) -> Result<()> {
        self.place_gateway(pos)
    }

    /// The first gateway of the shuffled ring (EnderDragonFight constructor
    /// and spawnNewGateway).
    pub(in crate::game::server_side) fn first_gateway(&self) -> [i32; 3] {
        let mut gateways: Vec<i32> = (0..20).collect();
        let mut random = LegacyRandom::new(self.common.level.seed);
        for i in (2..=gateways.len()).rev() {
            let j = random.next_int_bound(i as i32).unwrap_or(0) as usize;
            gateways.swap(i - 1, j);
        }
        let index = f64::from(gateways[gateways.len() - 1]);
        let angle = 2.0 * (-std::f64::consts::PI + std::f64::consts::PI / 20.0 * index);
        [
            (GATEWAY_RADIUS * angle.cos()).floor() as i32,
            GATEWAY_Y,
            (GATEWAY_RADIUS * angle.sin()).floor() as i32,
        ]
    }

    /// EndPodiumFeature(active).place at `origin` (the portal level).
    pub(in crate::game::server_side) fn place_podium(
        &mut self,
        origin: [i32; 3],
        active: bool,
    ) -> Result<()> {
        for y in origin[1] - 1..=origin[1] + 32 {
            for z in origin[2] - 4..=origin[2] + 4 {
                for x in origin[0] - 4..=origin[0] + 4 {
                    let d =
                        (x - origin[0]).pow(2) + (y - origin[1]).pow(2) + (z - origin[2]).pow(2);
                    let d = f64::from(d);
                    let inside_rim = d < 2.5 * 2.5;
                    if !(inside_rim || d < 3.5 * 3.5) {
                        continue;
                    }
                    let name = if y < origin[1] {
                        if inside_rim {
                            "minecraft:bedrock"
                        } else {
                            "minecraft:end_stone"
                        }
                    } else if y > origin[1] {
                        "minecraft:air"
                    } else if !inside_rim {
                        "minecraft:bedrock"
                    } else if active {
                        "minecraft:end_portal"
                    } else {
                        "minecraft:air"
                    };
                    self.set_state_named([x, y, z], name, &[])?;
                }
            }
        }
        for dy in 0..4 {
            self.set_state_named(
                [origin[0], origin[1] + dy, origin[2]],
                "minecraft:bedrock",
                &[],
            )?;
        }
        let registry = self.common.pack.assets.registry;
        for (facing, d) in [
            ("north", [0, -1]),
            ("south", [0, 1]),
            ("west", [-1, 0]),
            ("east", [1, 0]),
        ] {
            let Some(default) = registry.default_state("minecraft:wall_torch") else {
                continue;
            };
            let mut state = registry.state(default).cloned().ok_or("wall torch")?;
            state.properties.insert("facing".into(), facing.into());
            let id = registry.resolve(&state)?;
            self.level_set([origin[0] + d[0], origin[1] + 2, origin[2] + d[1]], id)?;
        }
        Ok(())
    }

    /// EndGatewayFeature.place: the gateway block in a bedrock cross.
    pub(in crate::game::server_side) fn place_gateway(&mut self, origin: [i32; 3]) -> Result<()> {
        for y in -2i32..=2 {
            for z in -1..=1 {
                for x in -1..=1 {
                    let (same_x, same_y, same_z) = (x == 0, y == 0, z == 0);
                    let end = y.abs() == 2;
                    let name = if same_x && same_y && same_z {
                        "minecraft:end_gateway"
                    } else if same_y {
                        "minecraft:air"
                    } else if end && same_x && same_z || (same_x || same_z) && !end {
                        "minecraft:bedrock"
                    } else {
                        "minecraft:air"
                    };
                    self.set_state_named([origin[0] + x, origin[1] + y, origin[2] + z], name, &[])?;
                }
            }
        }
        self.common
            .world
            .block_entities
            .get_or_insert_with((origin[0], origin[1], origin[2]), || {
                BlockEntity::EndGateway(Default::default())
            });
        Ok(())
    }

    /// `TheEndGatewayBlockEntity.setExitPosition`: the block entity alone owns the exit.
    pub(in crate::game::server_side) fn set_gateway_exit(
        &mut self,
        gateway: [i32; 3],
        exit: [i32; 3],
        exact: bool,
    ) -> Result<()> {
        let chunk = (gateway[0] >> 4, gateway[2] >> 4);
        if !self.common.world.chunks.contains_key(&chunk) {
            return Err("gateway source chunk is not loaded".into());
        }
        // A gateway removed while an entity waits for the destination stays removed.
        if &*self.block_name(gateway) != "minecraft:end_gateway" {
            return Ok(());
        }
        if let BlockEntity::EndGateway(g) = self
            .common
            .world
            .block_entities
            .get_or_insert_with((gateway[0], gateway[1], gateway[2]), || {
                BlockEntity::EndGateway(Default::default())
            })
        {
            g.exit_portal = Some(exit);
            g.exact_teleport = exact;
            self.common.world.modified.insert(chunk);
        }
        Ok(())
    }

    pub(in crate::game::server_side) fn known_gateway_exit(
        &self,
        gateway: [i32; 3],
    ) -> Option<([i32; 3], bool)> {
        match self
            .common
            .world
            .block_entities
            .get(&(gateway[0], gateway[1], gateway[2]))
        {
            Some(BlockEntity::EndGateway(g)) => g.exit_portal.map(|e| (e, g.exact_teleport)),
            _ => None,
        }
    }

    /// Where `gateway` leads (TheEndGatewayBlockEntity.getPortalPosition):
    /// its exit, or the search for one, and the spot to load before either
    /// is resolved ([`Game::resolve_gateway`]).
    pub(in crate::game::server_side) fn gateway_job(
        &mut self,
        gateway: [i32; 3],
    ) -> (GatewayJob, Vec3) {
        let job = GatewayJob::for_gateway(
            gateway,
            self.common
                .world
                .block_entities
                .get(&(gateway[0], gateway[1], gateway[2])),
        );
        if let Some(c) = job.source_chunk()
            && !self.common.end.areas.contains(&c)
        {
            self.common.end.areas.push(c);
            self.server.streaming.extra.push((c, 1));
        }
        let destination = match job {
            GatewayJob::Known { exit, .. } => Vec3::new(
                f64::from(exit[0]) + 0.5,
                f64::from(exit[1]),
                f64::from(exit[2]) + 0.5,
            ),
            GatewayJob::Find { gateway, .. } => {
                let d = glam::DVec2::new(f64::from(gateway[0]), f64::from(gateway[2]))
                    .normalize_or_zero()
                    * 1024.0;
                Vec3::new(d.x, 75.0, d.y)
            }
        };
        (job, destination)
    }

    pub(in crate::game::server_side) fn chunk_empty(&self, x: f64, z: f64) -> Option<bool> {
        let chunk = self
            .common
            .world
            .chunks
            .get(&((x / 16.0).floor() as i32, (z / 16.0).floor() as i32))?;
        Some(chunk.sections.iter().all(|s| s.is_uniform(0)))
    }

    /// TheEndGatewayBlockEntity.findTallestBlock.
    pub(in crate::game::server_side) fn tallest_block(
        &self,
        around: [i32; 3],
        dist: i32,
        allow_bedrock: bool,
    ) -> [i32; 3] {
        self.tallest_block_in(&self.common.world, around, dist, allow_bedrock)
    }
    /// [`Self::tallest_block`] in the loaded level `world`.
    pub(in crate::game) fn tallest_block_in(
        &self,
        world: &sunrise_level::world::World,
        around: [i32; 3],
        dist: i32,
        allow_bedrock: bool,
    ) -> [i32; 3] {
        let mut tallest: Option<[i32; 3]> = None;
        for xd in -dist..=dist {
            for zd in -dist..=dist {
                if xd == 0 && zd == 0 && !allow_bedrock {
                    continue;
                }
                let floor = tallest.map_or(0, |t| t[1]);
                let mut y = 255;
                while y > floor {
                    let p = [around[0] + xd, y, around[2] + zd];
                    let f = self.common.res.facts().get(world.block(p[0], p[1], p[2]));
                    if f.full_collision && (allow_bedrock || &*f.name != "minecraft:bedrock") {
                        tallest = Some(p);
                        break;
                    }
                    y -= 1;
                }
            }
        }
        tallest.unwrap_or(around)
    }

    /// The teleport position of a gateway job, creating the exit gateway (and
    /// an island when the chunk has no landing spot) for new destinations.
    pub(in crate::game::server_side) fn resolve_gateway(
        &mut self,
        job: GatewayJob,
        target: Vec3,
    ) -> Result<Vec3> {
        let bottom_center = sunrise_sim::end_gateway::bottom_center;
        let find_exit = |game: &Self, exit: [i32; 3]| {
            let p = game.tallest_block([exit[0], exit[1] + 2, exit[2]], 5, false);
            [p[0], p[1] + 1, p[2]]
        };
        match job {
            GatewayJob::Known { exit, exact } => Ok(bottom_center(exit_target(exit, exact, |e| {
                find_exit(self, e)
            }))),
            GatewayJob::Find { gateway, exact } => {
                if !self
                    .common
                    .world
                    .chunks
                    .contains_key(&(gateway[0] >> 4, gateway[2] >> 4))
                {
                    return Err("gateway source chunk is not loaded".into());
                }
                // The player and the entities may resolve the same gateway: the
                // first one to arrive finds the exit, the others use it.
                let known = self.known_gateway_exit(gateway);
                if let Some((exit, exact)) = known {
                    return Ok(bottom_center(exit_target(exit, exact, |e| {
                        find_exit(self, e)
                    })));
                }
                // findExitPortalXZPosTentative over the loaded chunks.
                let dir = glam::DVec2::new(f64::from(gateway[0]), f64::from(gateway[2]))
                    .normalize_or_zero();
                let mut pos = glam::DVec2::new(target.x, target.z);
                let mut limit = 16;
                while self.chunk_empty(pos.x, pos.y) == Some(false) && limit > 0 {
                    limit -= 1;
                    pos -= dir * 16.0;
                }
                let mut limit = 16;
                while self.chunk_empty(pos.x, pos.y) == Some(true) && limit > 0 {
                    limit -= 1;
                    pos += dir * 16.0;
                }
                let (cx, cz) = ((pos.x / 16.0).floor() as i32, (pos.y / 16.0).floor() as i32);
                // findValidSpawnInChunk: end stone with two non-full blocks above, closest to 0,0,0.
                let mut closest: Option<([i32; 3], f64)> = None;
                for y in 30..256 {
                    for z in cz * 16..cz * 16 + 16 {
                        for x in cx * 16..cx * 16 + 16 {
                            if &*self.block_name([x, y, z]) != "minecraft:end_stone"
                                || self
                                    .common
                                    .res
                                    .facts()
                                    .get(self.common.world.block(x, y + 1, z))
                                    .full_collision
                                || self
                                    .common
                                    .res
                                    .facts()
                                    .get(self.common.world.block(x, y + 2, z))
                                    .full_collision
                            {
                                continue;
                            }
                            let d = (f64::from(x) + 0.5).powi(2)
                                + (f64::from(y) + 0.5).powi(2)
                                + (f64::from(z) + 0.5).powi(2);
                            if closest.is_none_or(|c| d < c.1) {
                                closest = Some(([x, y, z], d));
                            }
                        }
                    }
                }
                let spot = match closest {
                    Some((p, _)) => p,
                    None => {
                        let p = [
                            (pos.x + 0.5).floor() as i32,
                            75,
                            (pos.y + 0.5).floor() as i32,
                        ];
                        self.place_end_island(p)?;
                        p
                    }
                };
                let tallest = self.tallest_block(spot, 16, true);
                let exit = [tallest[0], tallest[1] + 10, tallest[2]];
                self.place_gateway(exit)?;
                self.set_gateway_exit(exit, gateway, false)?;
                // setExitPosition keeps the gateway's own exactTeleport.
                self.set_gateway_exit(gateway, exit, exact)?;
                Ok(bottom_center(exit_target(exit, exact, |e| {
                    find_exit(self, e)
                })))
            }
        }
    }

    /// EndIslandFeature with RandomSource.create(pos.asLong()).
    pub(in crate::game::server_side) fn place_end_island(
        &mut self,
        origin: [i32; 3],
    ) -> Result<()> {
        let packed = ((i64::from(origin[0]) & 0x3FF_FFFF) << 38)
            | ((i64::from(origin[2]) & 0x3FF_FFFF) << 12)
            | (i64::from(origin[1]) & 0xFFF);
        let mut random = LegacyRandom::new(packed);
        let mut size = random.next_int_bound(3).unwrap_or(0) as f32 + 4.0;
        let mut y = 0;
        while size > 0.5 {
            for x in (-size).floor() as i32..=size.ceil() as i32 {
                for z in (-size).floor() as i32..=size.ceil() as i32 {
                    if (x * x + z * z) as f32 <= (size + 1.0) * (size + 1.0) {
                        self.set_state_named(
                            [origin[0] + x, origin[1] + y, origin[2] + z],
                            "minecraft:end_stone",
                            &[],
                        )?;
                    }
                }
            }
            size -= random.next_int_bound(2).unwrap_or(0) as f32 + 0.5;
            y -= 1;
        }
        Ok(())
    }

    // ---- End gateway travel for the entities ----

    /// `EndGatewayBlock.entityInside` for the items, mobs and thrown ender
    /// pearls in the End: the gateway blocks their box swept through. A
    /// gateway that is cooling down (`TheEndGatewayBlockEntity.triggerCooldown`)
    /// takes nobody in. `entityInside` comes from the entity's own tick
    /// (`applyEffectsFromBlocks`), and `ServerLevel.tick` ticks only the
    /// entities in entity-ticking chunks: one past the simulation distance
    /// enters none (and costs nothing here, however many lie there).
    pub fn find_end_gateway_portals(&mut self) {
        if !self.common.is_logical_server() || self.common.dim != Dim::End {
            return;
        }
        let is_gateway = |game: &Self, p: BlockPos| {
            &*game.res.facts().get(game.world.block(p.x, p.y, p.z)).name == "minecraft:end_gateway"
        };
        let ticking = self.entity_ticking_area();
        let ticks = |at: Vec3| ticking(at.x.floor() as i32, at.z.floor() as i32);
        let items: Vec<(u32, Vec3, f32, f32)> = self
            .common
            .dropped
            .iter()
            .filter(|e| ticks(e.position))
            .map(|e| (e.id, e.position, 0.25, 0.25))
            .collect();
        let mobs: Vec<(u32, Vec3, f32, f32)> = self
            .common
            .mobs
            .iter()
            .filter(|m| ticks(m.position) && self.mob_portal_eligible(m))
            .map(|m| (m.id, m.position, m.width(), m.height()))
            .chain(
                self.common
                    .boats
                    .iter()
                    .filter(|b| ticks(b.position) && self.vehicle_can_use_portal(b.id))
                    .map(|b| {
                        (
                            b.id,
                            b.position,
                            sunrise_gameplay_common::game::boats_common::WIDTH as f32,
                            sunrise_gameplay_common::game::boats_common::HEIGHT as f32,
                        )
                    }),
            )
            .chain(
                self.common
                    .carts
                    .iter()
                    .filter(|c| ticks(c.position) && self.vehicle_can_use_portal(c.id))
                    .map(|c| {
                        (
                            c.id,
                            c.position,
                            super::minecarts::WIDTH as f32,
                            super::minecarts::HEIGHT as f32,
                        )
                    }),
            )
            .collect();
        let mut inside: Vec<(u32, BlockPos)> = Vec::new();
        for (id, to, width, height) in items.into_iter().chain(mobs) {
            let from = self.server.portals.before.get(&id).copied().unwrap_or(to);
            let found = sunrise_sim::end_gateway::swept_gateway(from, to, width, height, |p| {
                is_gateway(self, p)
            });
            if let Some(pos) = found {
                inside.push((id, pos));
            }
        }
        // A pearl's move this tick, from where it started it.
        let pearls: Vec<(u32, Option<Who>, Option<u32>, BlockPos)> = self
            .common
            .creatures
            .projectiles
            .iter()
            .filter(|p| ticks(p.position))
            .filter_map(|p| {
                let pos = sunrise_sim::end_gateway::pearl_gateway(p, |b| is_gateway(self, b))?;
                Some((p.id, p.owner, p.guest, pos))
            })
            .collect();
        for (id, owner, guest, pos) in pearls {
            if owner == Some(Who::Player) {
                self.pearl_owner_in_gateway(guest, pos);
            }
            inside.push((id, pos));
        }
        for (id, pos) in inside {
            // One entity at a time: the gateway cools down for 40 ticks.
            if !self.gateway_enter([pos.x, pos.y, pos.z]) {
                continue;
            }
            // An entity that just came through any portal stays out of this one.
            if self.restart_portal_cooldown(id) {
                continue;
            }
            let delay = self.dimension_changing_delay(id);
            self.common
                .end
                .gateways
                .entry(id)
                .or_default()
                .enter_with_delay(pos, delay);
        }
        // ThrowableProjectile applies the gateway's block effect before Entity.tick.
        self.handle_end_gateway_portals_phase(true);
    }

    /// `ThrownEnderpearl.onInsideBlock` in an End gateway: its owner's
    /// `ServerPlayer.onInsideBlock`, the enter_block trigger, as if they had
    /// walked in. A guest's pearl triggers theirs; one whose guest has left
    /// has no owner.
    pub(in crate::game::server_side) fn pearl_owner_in_gateway(
        &mut self,
        guest: Option<u32>,
        pos: BlockPos,
    ) {
        let state = self.common.world.block(pos.x, pos.y, pos.z);
        let Some(conn) = guest else {
            self.trigger_enter_block(state);
            return;
        };
        self.as_guest(conn, |g| g.trigger_enter_block(state));
    }

    /// `Entity.getPortalCooldown` of an ender pearl: only an End gateway gives
    /// it one (zero when it has none).
    pub(in crate::game::server_side) fn pearl_portal_cooldown(&self, id: u32) -> i32 {
        self.common
            .end
            .gateways
            .get(&id)
            .map_or(0, |t| t.cooldown())
    }

    /// `Entity.handlePortal` for the entities in an End gateway: they go
    /// through to the gateway's exit, where they wait for its chunks.
    pub fn handle_end_gateway_portals(&mut self) {
        self.handle_end_gateway_portals_phase(false);
    }

    pub fn handle_end_gateway_portals_phase(&mut self, after_move: bool) {
        // Nobody in a gateway: nothing to look up.
        if !self.common.is_logical_server()
            || self.common.dim != Dim::End
            || self.common.end.gateways.is_empty()
        {
            return;
        }
        // `handlePortal` runs in the entity's tick: one past the simulation
        // distance waits in its gateway, its state kept as it is.
        let ticking = self.entity_ticking_area();
        let ticks = |at: Vec3| ticking(at.x.floor() as i32, at.z.floor() as i32);
        let ids: Vec<(u32, bool, bool)> = self
            .common
            .dropped
            .iter()
            .map(|e| (e.id, true, ticks(e.position)))
            .chain(
                self.common
                    .mobs
                    .iter()
                    .map(|m| (m.id, self.mob_portal_eligible(m), ticks(m.position))),
            )
            .chain(
                self.common
                    .boats
                    .iter()
                    .map(|b| (b.id, self.vehicle_can_use_portal(b.id), ticks(b.position))),
            )
            .chain(
                self.common
                    .carts
                    .iter()
                    .map(|c| (c.id, self.vehicle_can_use_portal(c.id), ticks(c.position))),
            )
            .chain(
                self.common
                    .creatures
                    .projectiles
                    .iter()
                    .filter(|p| matches!(p.kind, ProjectileKind::EnderPearl))
                    .map(|p| (p.id, true, ticks(p.position))),
            )
            .collect();
        let mut present: std::collections::HashSet<u32> =
            ids.iter().map(|(id, _, _)| *id).collect();
        // A pearl waiting for its exit keeps its portal cooldown.
        present.extend(self.common.end.arrivals.iter().map(|a| match &a.traveller {
            Arriving::Pearl(p) => p.id,
            Arriving::Live(id) => *id,
        }));
        self.common
            .end
            .gateways
            .retain(|id, _| present.contains(id));
        let mut leaving: Vec<(u32, BlockPos)> = Vec::new();
        for (id, allowed, ticking) in ids {
            if !ticking {
                continue;
            }
            // Only an entity in a gateway has a state: skip the others before
            // the per-id lookups below, which walk the entity lists.
            if !self.common.end.gateways.contains_key(&id) {
                continue;
            }
            let is_pearl = self
                .common
                .creatures
                .projectiles
                .iter()
                .any(|p| p.id == id && matches!(p.kind, ProjectileKind::EnderPearl));
            if is_pearl != after_move {
                continue;
            }
            let delay = self.dimension_changing_delay(id);
            let Some(state) = self.common.end.gateways.get_mut(&id) else {
                continue;
            };
            // The End gateway has no transition time (Portal.getPortalTransitionTime).
            if let Some(entry) = state.travel_with_delay(allowed, delay) {
                leaving.push((id, entry));
            }
            if state.is_done() {
                self.common.end.gateways.remove(&id);
            }
        }
        for (id, entry) in leaving {
            self.send_through_gateway(id, entry);
        }
    }

    /// `Entity.teleportToPortalDestination` for an entity that goes through an
    /// End gateway: an ordinary same-level entity stays live while the exit loads.
    pub(in crate::game::server_side) fn send_through_gateway(
        &mut self,
        id: u32,
        gateway: BlockPos,
    ) {
        if matches!(
            self.level_entity(id as i32),
            Some(
                super::commands::Target::Item(_)
                    | super::commands::Target::Mob(_)
                    | super::commands::Target::Boat(_)
                    | super::commands::Target::Cart(_)
            )
        ) {
            let (job, target) = self.gateway_job([gateway.x, gateway.y, gateway.z]);
            let arrival = GatewayArrival {
                traveller: Arriving::Live(id),
                job,
                target,
            };
            if let Some(waiting) = self.arrive_through_gateway(arrival) {
                self.common.end.arrivals.push(waiting);
            }
            return;
        }
        if let Some(i) = self
            .common
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id == id)
        {
            let pearl = Box::new(self.common.creatures.projectiles.remove(i));
            self.native_leave_skip(id);
            let (job, target) = self.gateway_job([gateway.x, gateway.y, gateway.z]);
            let arrival = GatewayArrival {
                traveller: Arriving::Pearl(pearl),
                job,
                target,
            };
            // It is at the exit before it moves this tick, once that has loaded.
            if let Some(waiting) = self.arrive_through_gateway(arrival) {
                self.common.end.arrivals.push(waiting);
            }
        }
    }

    /// The entities that reached a gateway's exit: they land where the
    /// gateway leads, once its chunks have loaded.
    pub(in crate::game::server_side) fn resolve_end_gateway_arrivals(&mut self) -> Result<()> {
        if !self.common.is_logical_server() || self.common.dim != Dim::End {
            return Ok(());
        }
        for arrival in std::mem::take(&mut self.common.end.arrivals) {
            if let Some(waiting) = self.arrive_through_gateway(arrival) {
                self.common.end.arrivals.push(waiting);
            }
        }
        Ok(())
    }

    /// `Entity.teleport` of a gateway traveller once its exit's chunk has
    /// loaded: the ordinary entity moves live and a pearl flies on from there.
    /// The traveller comes back while it still waits.
    pub(in crate::game::server_side) fn arrive_through_gateway(
        &mut self,
        arrival: GatewayArrival,
    ) -> Option<GatewayArrival> {
        let chunk = |at: Vec3| ((at.x.floor() as i32) >> 4, (at.z.floor() as i32) >> 4);
        if !self
            .common
            .world
            .chunks
            .contains_key(&chunk(arrival.target))
            || arrival
                .job
                .source_chunk()
                .is_some_and(|c| !self.common.world.chunks.contains_key(&c))
        {
            return Some(arrival);
        }
        let at = match self.resolve_gateway(arrival.job, arrival.target) {
            Ok(at) => at,
            Err(e) => {
                // The traveller waits for the exit: the failure is retried.
                log::warn!("End gateway: {e}");
                return Some(arrival);
            }
        };
        let key = chunk(at);
        match arrival.traveller {
            Arriving::Live(id) => {
                if let Some(target) = self.level_entity(id as i32) {
                    // Entity.teleportSameDimension keeps the live entity and
                    // its motion, and moves every passenger in the same operation.
                    self.teleport_to_in_level(target, [at.x, at.y, at.z], None, [false; 3], false);
                    self.set_portal_cooldown(id);
                    self.place_portal_ticket(key);
                }
            }
            Arriving::Pearl(mut pearl) => {
                sunrise_sim::end_gateway::place_pearl(&mut pearl, at);
                // ThrownEnderpearl.teleport: placePortalTicket where it came out.
                self.common.end.tickets.retain(|(c, _)| *c != key);
                self.common.end.tickets.push((key, PORTAL_TICKET));
                self.add_projectile(*pearl);
            }
        }
        None
    }

    /// The chunks the entities waiting to arrive need, kept loaded like the
    /// areas a server keeps around its players.
    pub(in crate::game::server_side) fn keep_gateway_destinations(&mut self) {
        let areas = &self.common.end.areas;
        self.server
            .streaming
            .extra
            .retain(|&(c, r)| !(r == 1 && areas.contains(&c)));
        // The pearls' portal tickets hold End chunks, and run out there
        // (TicketType.PORTAL's timeout).
        let in_end = self.common.dim == Dim::End;
        if in_end {
            self.common.end.tickets.retain_mut(|(_, ticks)| {
                *ticks = ticks.saturating_sub(1);
                *ticks > 0
            });
        }
        let mut areas = Vec::new();
        let arrivals = self.common.end.arrivals.iter().map(|a| {
            (
                (a.target.x.floor() as i32) >> 4,
                (a.target.z.floor() as i32) >> 4,
            )
        });
        let tickets = self.common.end.tickets.iter().filter(|_| in_end);
        let sources = self
            .common
            .end
            .arrivals
            .iter()
            .filter_map(|a| a.job.source_chunk());
        for c in arrivals
            .chain(tickets.map(|&(c, _)| c))
            .chain(sources)
            .chain(self.gateway_source_chunks())
        {
            if !areas.contains(&c) {
                areas.push(c);
                self.server.streaming.extra.push((c, 1));
            }
        }
        self.common.end.areas = areas;
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::block_props::BlockPropsGameExt2 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

#[cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use sunrise_gameplay_common::game::end_state::{exact_teleport, flash_pose};
    use sunrise_level::containers::EndGateway;

    #[test]
    fn a_new_exit_keeps_the_gateways_exact_teleport() {
        let exact = BlockEntity::EndGateway(EndGateway {
            exact_teleport: true,
            ..Default::default()
        });
        let plain = BlockEntity::EndGateway(EndGateway::default());
        assert!(exact_teleport(Some(&exact)));
        assert!(!exact_teleport(Some(&plain)));
        assert!(!exact_teleport(None));
        assert!(!exact_teleport(Some(&BlockEntity::DaylightDetector)));
        let exit = [8, 100, 0];
        // Exact: the exit itself, nothing is searched for.
        assert_eq!(exit_target(exit, true, |_| panic!("searched")), exit);
        assert_eq!(
            exit_target(exit, false, |e| [e[0], e[1] + 11, e[2]]),
            [8, 111, 0]
        );
    }

    /// `EndPortalBlock.getPortalDestination` out of the End when the world spawn is set in
    /// the End itself: an exit, not an entry, so no platform is built and the player faces the
    /// world spawn's angle (`TeleportTransition.createDefault`); the way in builds it and
    /// turns the player west.
    #[test]
    #[ignore = "daily"]
    fn leaving_the_end_for_a_world_spawn_there_builds_no_platform() {
        use crate::game::native_entity_refs::tests::with_game;
        use std::sync::Arc;

        with_game(|game| {
            game.enter_level(Dim::End).unwrap();
            let platform = [END_SPAWN[0], END_SPAWN[1] - 2, END_SPAWN[2]];
            let chunk = (platform[0] >> 4, platform[2] >> 4);
            game.world.chunks.insert(
                chunk,
                Arc::new(sunrise_level::chunk::Chunk::empty(chunk.0, chunk.1)),
            );
            game.server.respawn_config = None;
            game.spawn = None;
            game.level.world_spawn = None;
            game.level.world_spawn_angle = [45.0, -10.0];
            let target = Vec3::new(9.5, 70.0, -3.5);
            let exit = game.end_arrival(false, target, &mut None).unwrap();
            assert_eq!(exit, Some((target, [Some(45.0), Some(-10.0)])));
            assert_eq!(&*game.block_name(platform), "minecraft:air");
            let entry = game.end_arrival(true, target, &mut None).unwrap();
            assert_eq!(entry, Some((target, [Some(90.0), None])));
            assert_eq!(&*game.block_name(platform), "minecraft:obsidian");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_pearl_reads_its_end_gateway_cooldown_and_refuses_the_write() {
        use crate::game::native_entity_refs::tests::with_resident_game;

        with_resident_game(|game| {
            game.add_projectile(Projectile::new(
                ProjectileKind::EnderPearl,
                Vec3::ZERO,
                None,
            ));
            let id = game.creatures.projectiles[0].id;
            let pearl = game.entity(id).unwrap();
            assert_eq!(pearl.portal_cooldown(game), Some(0));
            // The write refuses a pearl by name: its cooldown is the End
            // gateway traveller's, not the level's portal state.
            assert!(!pearl.set_portal_cooldown(game, 40));
            assert_eq!(pearl.portal_cooldown(game), Some(0));
            let traveller = game.end.gateways.entry(id).or_default();
            traveller.enter(BlockPos::new(0, 64, 0));
            assert!(traveller.travel(true).is_some());
            assert_eq!(pearl.portal_cooldown(game), Some(300));
            assert!(!pearl.set_portal_cooldown(game, 40));
            assert_eq!(pearl.portal_cooldown(game), Some(300));
        });
    }

    #[test]
    fn leaving_the_end_without_a_point_of_its_own_goes_to_the_world_spawn() {
        let world = Some([220, 63, -41]);
        assert_eq!(fallback_spawn(None, world), Vec3::new(220.5, 63.0, -40.5));
        assert_eq!(
            fallback_spawn(Some(Vec3::new(10.5, 70.0, 12.5)), world),
            Vec3::new(10.5, 70.0, 12.5)
        );
        assert_eq!(fallback_spawn(None, None), Vec3::new(0.5, 64.0, 0.5));
    }

    #[test]
    fn end_gateway_job_reads_the_block_entity_and_keeps_new_sources_loaded() {
        let gateway = [702, 100, 0];
        let exit = [8, 100, 0];
        for exact in [true, false] {
            let entity = BlockEntity::EndGateway(EndGateway {
                exit_portal: Some(exit),
                exact_teleport: exact,
                ..Default::default()
            });
            assert_eq!(
                GatewayJob::for_gateway(gateway, Some(&entity)),
                GatewayJob::Known { exit, exact },
            );
        }
        let empty = BlockEntity::EndGateway(EndGateway::default());
        for entity in [None, Some(&empty)] {
            assert_eq!(
                GatewayJob::for_gateway(gateway, entity),
                GatewayJob::Known {
                    exit: END_SPAWN,
                    exact: true
                }
            );
            let nearby = [96, 75, 0];
            let exact = BlockEntity::EndGateway(EndGateway {
                exact_teleport: true,
                ..Default::default()
            });
            assert_eq!(
                GatewayJob::for_gateway(nearby, Some(&exact)),
                GatewayJob::Find {
                    gateway: nearby,
                    exact: true
                }
            );
            assert_eq!(
                GatewayJob::for_gateway(nearby, entity),
                GatewayJob::Find {
                    gateway: nearby,
                    exact: false
                }
            );
        }
    }

    /// `endFlashPosition`'s direction: the flash sprite's centre, 100 blocks away.
    #[test]
    fn the_end_flash_is_where_its_sprite_is() {
        let at =
            |x: f32, y: f32| flash_pose(x, y).transform_point3(glam::Vec3::new(0.0, 100.0, 0.0));
        // Straight up whatever the turn, and level toward the north at y_angle 180.
        assert!(at(-90.0, 37.0).abs_diff_eq(glam::Vec3::new(0.0, 100.0, 0.0), 1e-3));
        assert!(at(0.0, 180.0).abs_diff_eq(glam::Vec3::new(0.0, 0.0, -100.0), 1e-3));
        for (x, y) in [(-60.0, -180.0), (10.0, 95.0), (-25.0, 3.0)] {
            assert!((at(x, y).length() - 100.0).abs() < 1e-3);
        }
    }
}
