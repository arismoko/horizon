//! A new world's spawn and its first player's position: MinecraftServer.
//! setInitialSpawn walks a chunk spiral around NoiseBasedChunkGenerator.
//! getOrigin, then PlayerSpawnFinder.findSpawn picks a free column around
//! the world spawn within the respawn radius.
use crate::game::ServerGame;
use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use sunrise_game_data::Result;
pub(in crate::game) use sunrise_gameplay::initial_spawn::{Columns, bottom_center};
use sunrise_level::{
    collisions::{SceneCollisions, no_collision_or_liquid},
    random_source,
};
use sunrise_sim::world_spawn::{
    PlayerSpawnFinder, SpawnCandidate, SpawnLevel, level_respawn_pos, spawn_pos_in_chunk,
};
use sunrise_world::{
    collision_context::CollisionContext, heightmap::HeightmapType, level::flags::UPDATE_CLIENTS,
    registry::BlockStateId,
};

impl ServerGame<'_> {
    fn spawn_columns(&self) -> Columns<'_, '_> {
        Columns {
            world: &self.common.world,
            properties: &self.common.pack.assets.properties,
            collision: self.common.collision,
            // PlayerSpawnFinder.getLevelRespawnPos: a ceiling starts the walk
            // at the destination generator's spawn height.
            has_ceiling: self.dimension_type().height.has_ceiling,
            spawn_height: self.server.streaming.spawn_height(),
        }
    }
    /// ChunkGenerator.getOrigin and getSpawnHeight of the overworld.
    pub fn spawn_origin(&self) -> Result<((i32, i32), i32)> {
        if let Some(flat) = self.common.level.generator.as_ref().and_then(|g| {
            sunrise_sim::flatgen::FlatTerrain::from_settings(
                g.settings(),
                self.common.pack.assets.registry,
                world_biomes!(self),
                self.server.level_shapes.overworld,
            )
        }) {
            return Ok(((0, 0), flat.spawn_height()));
        }
        if self.common.options.test_terrain {
            return Ok(((0, 0), 64));
        }
        let origin = match self.common.end.locator() {
            Some(terrain) => terrain.origin()?,
            None => sunrise_core::position::ChunkPos::new(0, 0),
        };
        Ok(((origin.x, origin.z), 64))
    }

    /// Initialize Overworld spawn after its load event and before opening other levels.
    pub(in crate::game::server_side) fn initialize_overworld_spawn(&mut self) -> Result<()> {
        let dedicated = self.is_dedicated_server();
        if self.common.level.world_spawn.is_none() && self.server.world_debug {
            // Debug worlds skip the spawn hook and bonus chest.
            self.common.level.world_spawn = Some([0, 80, 0]);
            self.common.level.world_spawn_angle = [0.0, 0.0];
            self.common.level.spawn = Some([0.0, 80.0, 0.0]);
        }
        if self.common.level.world_spawn.is_none() {
            if dedicated {
                self.server.streaming.radius = -1;
            }
            self.try_initialize_world_spawn(true)?;
        }
        if dedicated {
            self.common.spawn =
                self.common.level.world_spawn.map(|[x, y, z]| {
                    Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5)
                });
        }
        Ok(())
    }

    pub(in crate::game::server_side) fn try_initialize_world_spawn(
        &mut self,
        wait: bool,
    ) -> Result<bool> {
        let Some([x, y, z]) = self.find_world_spawn(wait)? else {
            return Ok(false);
        };
        self.common.spawn = Some(Vec3::new(
            f64::from(x) + 0.5,
            f64::from(y),
            f64::from(z) + 0.5,
        ));
        self.common.level.spawn = Some([f64::from(x), f64::from(y), f64::from(z)]);
        // A personal spawnpoint must not replace the world's spawn.
        self.common.level.world_spawn.get_or_insert([x, y, z]);
        if !self.server.spawn_hook_cancelled
            && sunrise_sim::save::WorldGen::read(&self.server.world_root)
                .is_some_and(|g| g.bonus_chest)
        {
            self.place_bonus_chest([x, y, z])?;
        }
        Ok(true)
    }
    /// MinecraftServer.setInitialSpawn: the first chunk of the spiral with a
    /// valid respawn column gives the world spawn, else the origin chunk's
    /// centre at the spawn height. With `wait`, every tested column is loaded to FULL.
    fn find_world_spawn(&mut self, wait: bool) -> Result<Option<[i32; 3]>> {
        if !self.server.spawn_hook_fired {
            self.server.spawn_hook_fired = true;
            let mut position = self.common.level.world_spawn.unwrap_or([0, 0, 0]);
            let mut rotation = self.common.level.world_spawn_angle;
            let mut cancel = false;
            self.native_create_spawn_position(&mut position, &mut rotation, &mut cancel);
            if cancel {
                self.server.spawn_hook_cancelled = true;
                self.common.level.world_spawn = Some(position);
                self.common.level.world_spawn_angle = rotation;
                return Ok(Some(position));
            }
        }
        let ((ox, oz), height) = match self.server.spawn_origin {
            Some(origin) => origin,
            None => *self.server.spawn_origin.insert(self.spawn_origin()?),
        };
        while let Some((dx, dz)) = self.server.spawn_spiral.current() {
            if self.server.spawn_spiral.tested() {
                let (cx, cz) = (ox + dx, oz + dz);
                if wait {
                    if self.load_full_chunk(None, (cx, cz)).is_err() {
                        return Err(format!("could not load initial spawn chunk {cx},{cz}").into());
                    }
                } else if !self.server.streaming.is_lit(cx, cz) {
                    self.server.streaming.center = (cx, cz);
                    return Ok(None);
                }
                if let Some(pos) = spawn_pos_in_chunk(&self.spawn_columns(), cx, cz) {
                    return Ok(Some(pos));
                }
            }
            self.server.spawn_spiral.advance();
        }
        Ok(Some([ox * 16 + 8, height, oz * 16 + 8]))
    }
    /// MinecraftServer.setInitialSpawn's bonus chest: BonusChestFeature.place
    /// in the spawn chunk, a chest with the spawn bonus loot and a torch on
    /// each side that can hold one, on the first free column of a shuffled
    /// walk over the chunk. Skipped while the chunk is not loaded.
    pub(in crate::game::server_side) fn place_bonus_chest(
        &mut self,
        origin: [i32; 3],
    ) -> Result<()> {
        use sunrise_level::containers::BlockEntity;
        let (cx, cz) = (origin[0] >> 4, origin[2] >> 4);
        let registry = &self.common.pack.assets.registry;
        let (Some(chest), Some(torch)) = (
            registry.default_state("minecraft:chest"),
            registry.default_state("minecraft:torch"),
        ) else {
            return Ok(());
        };
        if !self.server.streaming.is_lit(cx, cz) {
            return Ok(());
        }
        // `level.getRandom()`. Util.toShuffledList of the chunk's x and z.
        let random = &mut self.common.world.random;
        let mut shuffled = |min: i32| -> [i32; 16] {
            let mut values: [i32; 16] = std::array::from_fn(|i| min + i as i32);
            for end in (2..=16usize).rev() {
                let other = random.next_int_bound(end as i32).unwrap_or(0) as usize;
                values.swap(end - 1, other);
            }
            values
        };
        let (xs, zs) = (shuffled(cx * 16), shuffled(cz * 16));
        for x in xs {
            for z in zs {
                let y = self
                    .spawn_columns()
                    .height(HeightmapType::MotionBlockingNoLeaves, x, z);
                if self.common.world.block(x, y, z) != self.common.world.air
                    && !self.collision_empty([x, y, z])
                {
                    continue;
                }
                self.set_block_clients([x, y, z], chest)?;
                let seed = self.common.world.random.next_long();
                self.common
                    .world
                    .block_entities
                    .insert((x, y, z), BlockEntity::Chest(Vec::new()));
                self.load_block_entity_nbt(
                    [x, y, z],
                    &format!(
                        "{{LootTable:\"minecraft:chests/spawn_bonus_chest\",LootTableSeed:{seed}L}}"
                    ),
                );
                for [dx, dz] in [[0, -1], [1, 0], [0, 1], [-1, 0]] {
                    let at = [x + dx, y, z + dz];
                    let survives = self.with_level(|b, level, ticks| {
                        b.can_survive(level, ticks, BlockPos::new(at[0], at[1], at[2]), torch)
                    });
                    if survives {
                        self.set_block_clients(at, torch)?;
                    }
                }
                return Ok(());
            }
        }
        Ok(())
    }
    /// `Level.setBlock(pos, state, UPDATE_CLIENTS)`: no neighbour updates.
    fn set_block_clients(&mut self, pos: [i32; 3], state: BlockStateId) -> Result<()> {
        self.with_level(|b, level, ticks| {
            b.set_block(
                level,
                ticks,
                BlockPos::new(pos[0], pos[1], pos[2]),
                state,
                UPDATE_CLIENTS,
            );
        });
        self.server_process_level_events()
    }
    /// The block at `pos` has no collision shape.
    pub(in crate::game::server_side) fn collision_empty(&self, pos: [i32; 3]) -> bool {
        let mut scene = SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: None,
        };
        self.common
            .collision
            .collision_shape_with_context(
                &mut scene,
                self.common.world.block(pos[0], pos[1], pos[2]),
                BlockPos::new(pos[0], pos[1], pos[2]),
                &CollisionContext::empty(false),
            )
            .is_ok_and(|shape| shape.is_empty())
    }
    /// PlayerSpawnFinder.findSpawn around the world spawn, as far as the loaded
    /// chunks allow: the feet of the first free column within the respawn
    /// radius, else the suggestion's own column. The finder waits in `state`
    /// (None to begin); Err gives the chunk of the candidate being waited for,
    /// which the caller must have loaded (the finder tries at most 1024
    /// columns).
    pub(in crate::game::server_side) fn search_spawn(
        &self,
        suggestion: [i32; 3],
        state: &mut Option<PlayerSpawnFinder>,
    ) -> std::result::Result<Vec3, (i32, i32)> {
        let mut finder = state
            .take()
            .unwrap_or_else(|| self.spawn_finder(suggestion));
        let suggestion = finder.suggestion;
        loop {
            let Some(candidate) = finder.next_candidate() else {
                return Ok(self.fixup_spawn_height(suggestion));
            };
            let (x, z) = match candidate {
                SpawnCandidate::Column(x, z) => (x, z),
                SpawnCandidate::Suggestion => (suggestion[0], suggestion[2]),
            };
            if !self.server.streaming.is_lit(x >> 4, z >> 4) {
                finder.retry();
                *state = Some(finder);
                return Err((x >> 4, z >> 4));
            }
            match candidate {
                SpawnCandidate::Column(x, z) => {
                    if let Some(pos) = level_respawn_pos(&self.spawn_columns(), x, z)
                        && self.player_fits(pos)
                    {
                        return Ok(bottom_center(pos));
                    }
                }
                SpawnCandidate::Suggestion => return Ok(self.fixup_spawn_height(suggestion)),
            }
        }
    }
    /// PlayerSpawnFinder.findSpawn around the world spawn: the feet of a
    /// player who has no position yet. None while a candidate's chunk loads.
    pub(in crate::game::server_side) fn find_player_spawn(
        &mut self,
        suggestion: [i32; 3],
    ) -> Result<Option<Vec3>> {
        let mut state = self.server.player_spawn.take();
        match self.search_spawn(suggestion, &mut state) {
            Ok(feet) => Ok(Some(feet)),
            Err(chunk) => {
                self.server.streaming.center = chunk;
                self.server.player_spawn = state;
                Ok(None)
            }
        }
    }
    /// A new PlayerSpawnFinder around `suggestion`: the respawn radius, cut
    /// by the world border; the suggestion itself in adventure mode.
    fn spawn_finder(&self, suggestion: [i32; 3]) -> PlayerSpawnFinder {
        if self.common.level.game_mode == 2 {
            let mut finder = PlayerSpawnFinder::new(suggestion, 0, |_| 0);
            finder.next_candidate();
            return finder;
        }
        let mut radius = self
            .common
            .ui
            .rules
            .int_at(sunrise_game_data::gamerules::vanilla::RESPAWN_RADIUS)
            .max(0);
        let to_border = self
            .common
            .border
            .distance(f64::from(suggestion[0]), f64::from(suggestion[2]))
            .floor() as i32;
        if to_border < radius {
            radius = to_border;
        }
        if to_border <= 1 {
            radius = 1;
        }
        PlayerSpawnFinder::new(suggestion, radius, |n| {
            random_source::entity().next_int_bound(n).unwrap_or(0)
        })
    }
    /// PlayerSpawnFinder.noCollisionNoLiquid: a player's box standing at the
    /// block's bottom centre touches no block or liquid.
    fn player_fits(&self, pos: [i32; 3]) -> bool {
        let feet = bottom_center(pos);
        let bounds = Aabb::new(
            Vec3::new(feet.x - 0.3, feet.y, feet.z - 0.3),
            Vec3::new(feet.x + 0.3, feet.y + 1.8, feet.z + 0.3),
        );
        let mut scene = SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: None,
        };
        no_collision_or_liquid(&mut scene, bounds)
    }
    /// PlayerSpawnFinder.fixupSpawnHeight: up out of anything solid, then
    /// down onto the ground.
    fn fixup_spawn_height(&self, pos: [i32; 3]) -> Vec3 {
        let [x, mut y, z] = pos;
        while !self.player_fits([x, y, z]) && y < self.common.world.top_y() - 1 {
            y += 1;
        }
        y -= 1;
        while self.player_fits([x, y, z]) && y > self.common.world.bottom_y() {
            y -= 1;
        }
        bottom_center([x, y + 1, z])
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
    use crate::game::world_loading::WorldRequest;
    use mod_api::{ChunkLoadEvent, Hooks, LevelLoadEvent, ModEntry, ServerHost};
    use std::sync::{Arc, Mutex};
    use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;
    use sunrise_sim::{
        save::WorldPreset,
        world_options::{FlatSettings, WorldOptions},
    };

    #[test]
    #[ignore = "daily"]
    fn integrated_creation_finishes_world_spawn_before_opening_the_nether() {
        if run_fresh_cases(
            "game::server_side::initial_spawn::tests::integrated_creation_finishes_world_spawn_before_opening_the_nether",
            "SUNRISE_INITIAL_WORLD_SPAWN_CASE",
            &["flat"],
        ) {
            return;
        }

        struct Log(Arc<Mutex<Vec<&'static str>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Log {
            fn level_load(&self, _: &mut dyn ServerHost, event: &mut LevelLoadEvent<'_>) {
                match event.dimension {
                    "minecraft:overworld" => self.0.lock().unwrap().push("overworld"),
                    "minecraft:the_nether" => self.0.lock().unwrap().push("nether"),
                    _ => {}
                }
            }
            fn chunk_load(&self, _: &mut dyn ServerHost, event: &mut ChunkLoadEvent<'_>) {
                if event.dimension == "minecraft:overworld" && event.chunk == [0, 0] {
                    self.0.lock().unwrap().push("spawn column");
                }
            }
        }
        let log = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![ModEntry::new("spawn_order", Box::new(Log(log.clone())))].into(),
            || {
                with_game(|game| {
                    let root = tempfile::tempdir().unwrap();
                    game.join_world_rest().unwrap();
                    let options = WorldOptions {
                        world_type: WorldPreset::Flat,
                        flat_settings: Some(FlatSettings {
                            structure_overrides: Vec::new(),
                            ..FlatSettings::default()
                        }),
                        structures: false,
                        ..WorldOptions::default()
                    };
                    let mut request = WorldRequest::new(
                        "spawn_order",
                        Some("Flat spawn"),
                        Some(1),
                        false,
                        Some(&options),
                        game.common.level.rule_registry.clone(),
                    )
                    .unwrap();
                    request.save_root = root.path().to_path_buf();
                    request.stem_order = Some(&[
                        "minecraft:overworld",
                        "minecraft:the_nether",
                        "minecraft:the_end",
                    ]);
                    let prepared = game.prepare_world_and_rules(request).unwrap();
                    assert!(prepared.level.world_spawn.is_none());
                    game.native_lifecycle_open_started();
                    game.finish_open_world(prepared).unwrap();

                    assert_eq!(game.common.level.world_spawn, Some([0, -60, 0]));
                    assert!(
                        game.common.player.is_none(),
                        "the owner's player is prepared separately"
                    );
                })
            },
        );
        assert_eq!(
            *log.lock().unwrap(),
            ["overworld", "spawn column", "nether"]
        );
    }
}
