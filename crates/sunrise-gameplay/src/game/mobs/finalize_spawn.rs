//! The game's own half of `Mob.finalizeSpawn`: `Game` as a
//! [`finalize::FinalizeLevel`], so the live level finalizes mobs through the
//! same [`finalize::Ctx`] world generation does (`mobs::finalize`).
//!
//! `Ctx.random` is the level's own random (`world.random`): vanilla draws
//! finalizeSpawn's rolls from `level.getRandom()` (`Mob.finalizeSpawn` the
//! follow-range triangle and the left-handed roll, `NaturalSpawner` the yaw
//! just before the call), never from the mob's own random. The game used to
//! draw them from each mob's own random in `mobs::spawning::finalize`; this
//! is that divergence removed.

use crate::game::ServerGame;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_sim::mobs::{Mob, Species, finalize, spawn_rules::Reason, spawning};
use sunrise_world::food::Difficulty;

/// `Game` seen by a finalizeSpawn `Ctx`.
pub(in crate::game) struct FinalizeGame<'a, 'g> {
    pub(in crate::game) game: &'a mut crate::game::ServerGame<'g>,
}

/// A finalizeSpawn biome lookup with its chunk not loaded: the plains' id,
/// the same default the biome facts fall back to.
pub(in crate::game::server_side) fn biome_or_plains(name: Option<String>) -> String {
    name.unwrap_or_else(|| "minecraft:plains".to_owned())
}

impl FinalizeGame<'_, '_> {
    /// The biome id at a block, when its chunk is loaded.
    fn biome_name(&self, pos: [i32; 3]) -> Option<String> {
        // `LevelReader.getBiome`: the noise cell the biome zoom picks, perhaps in a neighbouring chunk.
        let game: &crate::game::ServerGame<'_> = &*self.game;
        let biome =
            game.world
                .biome_at(game.server.streaming.biome_zoom, pos[0], pos[1], pos[2])?;
        Some(world_biomes!(game).get(biome).name.clone())
    }
}

impl finalize::FinalizeLevel for FinalizeGame<'_, '_> {
    fn mob_maps(&self) -> &sunrise_sim::neoforge_mob_maps::NeoForgeMobMaps {
        self.game.mob_maps()
    }
    fn biome(&self, pos: [i32; 3]) -> String {
        // A spawn site without its chunk (a loaded marker mob) is neutral.
        biome_or_plains(self.biome_name(pos))
    }
    fn biome_tag(&self, tag: &str, pos: [i32; 3]) -> bool {
        self.biome_name(pos).is_some_and(|b| {
            if let Some(tags) = self
                .game
                .server
                .world_tables
                .0
                .as_ref()
                .and_then(|t| t.pack_tags.get("worldgen/biome"))
            {
                return tags.get(tag).is_some_and(|members| members.contains(&b));
            }
            self.game.res.mob_data().biome_has(tag, &b)
        })
    }
    fn structures_with_piece_at(&self, pos: [i32; 3]) -> Vec<String> {
        self.game.structures_at(pos)
    }
    fn structure_tag(&self, tag: &str, structure: &str) -> bool {
        if let Some(tags) = self
            .game
            .server
            .world_tables
            .0
            .as_ref()
            .and_then(|t| t.pack_tags.get("worldgen/structure"))
        {
            return tags
                .get(tag)
                .is_some_and(|members| members.contains(structure));
        }
        self.game.structure_tag(tag).iter().any(|m| m == structure)
    }
    fn moon_phase(&self, _: [i32; 3]) -> usize {
        self.game.finalize_moon_phase()
    }
    fn spawning_monsters(&self) -> bool {
        self.game.spawning_monsters()
    }
    fn no_collision(&self, bounds: Aabb) -> bool {
        self.game
            .no_spawn_collision(bounds, |query| self.game.live_spawn_mob_collision(query))
    }
    fn free_chicken(&self, bounds: Aabb) -> Option<u32> {
        self.game.finalize_free_chicken(bounds)
    }
    fn ride_chicken(&mut self, chicken: u32, rider: u32) {
        if let Some(c) = self.game.mobs.iter_mut().find(|m| m.id == chicken) {
            c.extra.jockey = true;
            c.extra.rider = Some(rider);
        }
    }
    fn game_time(&self) -> i64 {
        self.game.finalize_game_time()
    }
    fn halloween(&self) -> bool {
        self.game.finalize_halloween()
    }
    /// `EntityType.create(level, reason)` over the live level: a new id and
    /// the type's own table entry, or None where the type cannot exist.
    fn spawn_event(&mut self, mob: &mut Mob, reason: Reason) -> finalize::SpawnEvent {
        self.game.finalize_spawn_event_result(mob, reason)
    }
    fn create(&mut self, kind: &str, _: Reason) -> Option<Mob> {
        let t = self.game.res.mob_types().get(kind)?;
        if self.game.world.health_rules.difficulty == Difficulty::Peaceful
            && !t.allowed_in_peaceful()
        {
            return None;
        }
        let t = t.clone();
        self.game.server.next_entity += 1;
        Some(Mob::new(self.game.server.next_entity, t, Vec3::ZERO, 0.0))
    }
    fn spawn_data(&self) -> finalize::SpawnData<'_> {
        self.game.finalize_spawn_data()
    }
}

impl ServerGame<'_> {
    /// `DimensionType.moonPhase` at the level's day time.
    pub(in crate::game::server_side) fn finalize_moon_phase(&self) -> usize {
        (self.common.world.day_time.div_euclid(24000).rem_euclid(8)) as usize
    }
    /// The level's game time (`Level.getDayTime`).
    pub(in crate::game::server_side) fn finalize_game_time(&self) -> i64 {
        self.common.world.day_time
    }
    /// The first free chicken a jockey can take: alive, unhindered, inside
    /// the bounds.
    pub(in crate::game::server_side) fn finalize_free_chicken(&self, bounds: Aabb) -> Option<u32> {
        self.common
            .mobs
            .iter()
            .find(|c| {
                c.species() == Species::Chicken
                    && c.alive()
                    && c.extra.rider.is_none()
                    && c.extra.mount.is_none()
                    && c.bounds().intersects(bounds)
            })
            .map(|c| c.id)
    }
    /// `Fallable.isHalloween` at the level's clock.
    pub(in crate::game::server_side) fn finalize_halloween(&self) -> bool {
        finalize::is_halloween()
    }

    /// The active variant tables: the server's frozen world tables (or prepared defaults).
    pub fn world_variants(&self) -> &sunrise_sim::mobs::variants::VariantTables {
        self.server
            .world_tables
            .0
            .as_ref()
            .map_or_else(|| self.common.res.variants(), |t| &t.variants)
    }
    /// The tables a finalizeSpawn `Ctx` borrows: the variant registries, the
    /// enchantments and items its gear rolls need, and `EntityTypes.CAMEL_HUSK`'s
    /// dimensions if the table has the type.
    pub(in crate::game::server_side) fn finalize_spawn_data(&self) -> finalize::SpawnData<'_> {
        let game = self;
        finalize::SpawnData {
            variants: game.world_variants(),
            enchantments: &world_stations!(game).enchantments,
            items: world_items!(game),
            camel_husk_size: game
                .res
                .mob_types()
                .get("minecraft:camel_husk")
                .map(|t| (t.width as f32, t.height as f32))
                .unwrap_or((1.7, 2.375)),
        }
    }
    /// The mod event half of a finalizeSpawn, mapped to the `Ctx`'s outcome:
    /// the level decides, and the game carries it out.
    pub(in crate::game::server_side) fn finalize_spawn_event_result(
        &self,
        mob: &mut Mob,
        reason: Reason,
    ) -> finalize::SpawnEvent {
        // A mob finalization makes starts without group data, and the
        // finalization that follows reads none.
        self.finalize_spawn_event(mob, reason, None, &mut None).0
    }
    /// The game's own finalization of a mob, with no event: the same
    /// `finalize::Ctx` world generation runs, over the live level, with the
    /// level's random (`world.random`). Mobs the call made (jockeys and
    /// mounts) join the level beside it.
    pub fn finalize_plain(
        &mut self,
        mob: &mut Mob,
        difficulty: &spawning::LocalDifficulty,
        reason: Reason,
    ) {
        self.finalize_grouped(mob, difficulty, reason, None);
    }
    /// [`Game::finalize_plain`] with the pack's group data, as a listener of
    /// `finalize_spawn` left it.
    pub(in crate::game) fn finalize_grouped(
        &mut self,
        mob: &mut Mob,
        difficulty: &spawning::LocalDifficulty,
        reason: Reason,
        group: Option<finalize::Group>,
    ) {
        let mut random = self.common.world.random.clone();
        let mut level = FinalizeGame { game: self };
        let mut cx = finalize::Ctx {
            level: &mut level,
            random: &mut random,
            difficulty: *difficulty,
            created: Vec::new(),
            jockey_rider: None,
        };
        finalize::finalize(&mut cx, mob, reason, group);
        let _save_lease = sunrise_sim::join_seq::visibility::PassLease::enter();
        let created = std::mem::take(&mut cx.created);
        let game = level.game;
        game.world.random = random;
        for m in created {
            game.mobs.push(m);
            game.admit_new_mobs(game.mobs.len() - 1);
        }
    }
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
