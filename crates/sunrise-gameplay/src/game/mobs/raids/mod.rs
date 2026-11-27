//! Pillager patrols (`PatrolSpawner`), the raid omen, and raids (`Raids`):
//! their waves, boss bar, victory and the raiders' bookkeeping. The rules are
//! `sunrise_client::patrol` and `sunrise_client::raid`; this is the game's
//! side of them.
use super::Dim;
use crate::game::ServerGame;
use std::collections::{HashMap, HashSet};
use sunrise_core::nbt::Tag;
use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, LevelRandom, RandomSource},
};
use sunrise_gameplay::village_siege::{SiegeLevel, VillageSiege};
use sunrise_level::{
    effects::{EffectInstance, OmenTick, TickingEffect},
    random_source,
};
use sunrise_sim::mobs::villager as mobs_villager;
use sunrise_sim::{
    lan::playerdata::ints_uuid,
    mobs::{
        Mob, Species, raider,
        spawn_rules::{self, SpawnLevel},
        spawn_selection::{Heightmap, NaturalLevel},
    },
    patrol::{GameplayAttributes, Member, PatrolLevel, PatrolSpawner},
    raid::{self, Action, BarName, RaidLevel, RaidPlayer, RaiderInfo, RaiderType, Raids, Spawn},
};

/// Patrol and raid state owned by the game.
#[derive(Default)]
pub struct Raiding {
    pub(in crate::game) patrol: PatrolSpawner,
    pub siege: VillageSiege,
    /// The raids of the dimension the player is in, and that dimension.
    pub raids: Raids,
    pub(in crate::game) loaded: Option<Dim>,
    /// `ServerPlayer.raidOmenPosition` (`raid_omen_position`), read from the
    /// player data on first use.
    pub(in crate::game) omen_position: Option<[i32; 3]>,
    pub(in crate::game) omen_loaded: bool,
    /// Raids read before their first raid tick, whose randoms that tick seeds
    /// (their ids and positions in the file).
    pub(in crate::game) unseeded: Option<Vec<(i32, usize)>>,
    /// The last raider hurt each raid's bar has seen (`Raider.hurtServer`).
    pub(in crate::game) bar_hurts: HashMap<i32, u64>,
    /// Beds near each raid's centre, rescanned now and then (`HOME` points
    /// of interest for `RaiderMoveThroughVillageGoal`).
    pub(in crate::game) homes: HashMap<i32, (i64, std::sync::Arc<Vec<[i32; 3]>>)>,
}

/// A level's living raid members, in mob-list order.
pub(in crate::game::server_side) fn members(mobs: &[Mob], id: i32) -> impl Iterator<Item = &Mob> {
    mobs.iter().filter(move |m| {
        m.alive() && !m.removed && raider::state(m).is_some_and(|s| s.raid_id == Some(id))
    })
}

impl Raiding {
    pub fn raids(&self, dimension: &str) -> Option<&Raids> {
        self.loaded
            .filter(|dim| dim.id() == dimension)
            .map(|_| &self.raids)
    }
    pub fn raids_mut(&mut self, dimension: &str) -> Option<&mut Raids> {
        self.loaded
            .filter(|dim| dim.id() == dimension)
            .map(|_| &mut self.raids)
    }
    /// Exchange the level-held parts (`ServerLevel.getRaids`) with another
    /// level's (game/world/levels.rs).
    pub(in crate::game::server_side) fn swap_level(&mut self, other: &mut Self) {
        std::mem::swap(&mut self.raids, &mut other.raids);
        std::mem::swap(&mut self.loaded, &mut other.loaded);
        std::mem::swap(&mut self.unseeded, &mut other.unseeded);
        std::mem::swap(&mut self.bar_hurts, &mut other.bar_hurts);
        std::mem::swap(&mut self.homes, &mut other.homes);
    }
}

#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay::custom_spawner::{section, sections_to_village};

/// The level as a raid sees it, with this tick's village data.
pub struct Lvl<'a, 'g> {
    pub game: &'a crate::game::ServerGame<'g>,
    pub difficulty: i32,
    pub centers: &'a HashSet<(i32, i32, i32)>,
}
impl RaidLevel for Lvl<'_, '_> {
    fn difficulty(&self) -> i32 {
        self.difficulty
    }
    fn has_chunk_at(&self, p: [i32; 3]) -> bool {
        self.game.world.chunks.contains_key(&(p[0] >> 4, p[2] >> 4))
    }
    fn has_chunks_at(&self, x0: i32, z0: i32, x1: i32, z1: i32) -> bool {
        (x0 >> 4..=x1 >> 4)
            .all(|cx| (z0 >> 4..=z1 >> 4).all(|cz| self.game.world.chunks.contains_key(&(cx, cz))))
    }
    fn sections_to_village(&self, p: [i32; 3]) -> i32 {
        sections_to_village(self.centers, section(p))
    }
    fn surface_height(&self, x: i32, z: i32) -> i32 {
        self.game.surface_height(x, z)
    }
    /// `ServerLevel.isPositionEntityTicking`: a loaded chunk whose entities
    /// tick (`canPositionTick`) in entity ticking range, not merely loaded.
    fn is_position_entity_ticking(&self, p: [i32; 3]) -> bool {
        self.has_chunk_at(p)
            && self
                .game
                .server
                .tickets
                .0
                .levels()
                .entity_ticking_at(p[0], p[2])
    }
    fn ravager_can_spawn(&self, [x, y, z]: [i32; 3]) -> bool {
        let g = self.game;
        let w = g.mob_world_base(&[]);
        spawn_rules::is_spawn_position_ok("minecraft:ravager", Species::Ravager, &w, x, y, z)
            || w.block_name(x, y - 1, z) == "minecraft:snow"
                && g.res.facts().is_air(g.world.block(x, y, z))
    }
    fn players(&self) -> Vec<RaidPlayer> {
        // The players of the level in place: a server has none of its own.
        self.game
            .level_players()
            .into_iter()
            .map(|p| RaidPlayer {
                uuid: p.uuid,
                position: [p.position.x, p.position.y, p.position.z],
                block: block_of(p.position),
                alive: p.alive,
            })
            .collect()
    }
}

/// The game level as the patrol spawner reads it.
pub(in crate::game) struct PatrolCx<'a, 'g, 'h> {
    pub(in crate::game) level: super::natural_spawning::GameLevel<'a, 'g>,
    pub(in crate::game) centers: HashSet<(i32, i32, i32)>,
    pub(in crate::game) can_spawn: &'h dyn Fn(&crate::game::ServerGame<'_>, [i32; 3]) -> bool,
}
impl SpawnLevel for PatrolCx<'_, '_, '_> {
    #[inline]
    fn within_world_border(&self, x: i32, z: i32) -> bool {
        self.level.within_world_border(x, z)
    }
    #[inline]
    fn state(&self, x: i32, y: i32, z: i32) -> sunrise_world::registry::BlockStateId {
        self.level.state(x, y, z)
    }
    #[inline]
    fn facts(&self) -> &sunrise_items::blocks::BlockFacts {
        self.level.facts()
    }
    #[inline]
    fn mob_data(&self) -> &sunrise_sim::mobs::data::MobData {
        self.level.mob_data()
    }
    #[inline]
    fn block_tag(&self, tag: &str, block: &str) -> bool {
        self.level.block_tag(tag, block)
    }
    #[inline]
    fn biome_tag(&self, tag: &str, x: i32, y: i32, z: i32) -> bool {
        self.level.biome_tag(tag, x, y, z)
    }
    #[inline]
    fn sky_light(&self, x: i32, y: i32, z: i32) -> i32 {
        self.level.sky_light(x, y, z)
    }
    #[inline]
    fn block_light(&self, x: i32, y: i32, z: i32) -> i32 {
        self.level.block_light(x, y, z)
    }
    #[inline]
    fn sky_darken(&self) -> i32 {
        SpawnLevel::sky_darken(&self.level)
    }
    #[inline]
    fn thundering(&self) -> bool {
        self.level.thundering()
    }
    #[inline]
    fn sea_level(&self) -> i32 {
        self.level.sea_level()
    }
    #[inline]
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.level.world_surface(x, z)
    }
    #[inline]
    fn peaceful(&self) -> bool {
        self.level.peaceful()
    }
    #[inline]
    fn dimension_type(&self) -> &sunrise_world::dimension_type::DimensionFacts {
        self.level.dimension_type()
    }
    #[inline]
    fn surface_slime_chance(&self, x: i32, y: i32, z: i32) -> f32 {
        self.level.surface_slime_chance(x, y, z)
    }
    #[inline]
    fn world_seed(&self) -> i64 {
        self.level.world_seed()
    }
    #[inline]
    fn player_near(&self, x: f64, y: f64, z: f64, distance: f64) -> bool {
        self.level.player_near(x, y, z, distance)
    }
}
impl PatrolLevel for PatrolCx<'_, '_, '_> {
    #[inline]
    fn spawn_patrols(&self) -> bool {
        self.level
            .game
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_PATROLS)
    }
    #[inline]
    fn bright_outside(&self) -> bool {
        !self.level.game.dimension_type().has_fixed_time
            && WeatherGameExt::sky_darken(self.level.game) < 4
    }
    /// `PatrolSpawner.tick` picks from `level.players()`: the local player
    /// and every guest in the level.
    #[inline]
    fn patrol_players(&self) -> Vec<([i32; 3], bool)> {
        self.level
            .players()
            .iter()
            .map(|p| (block_of(p.position), p.spectator))
            .collect()
    }
    #[inline]
    fn sections_to_village(&self, pos: [i32; 3]) -> i32 {
        sections_to_village(&self.centers, section(pos))
    }
    #[inline]
    fn has_chunk(&self, cx: i32, cz: i32) -> bool {
        self.level.game.world.chunks.contains_key(&(cx, cz))
    }
    #[inline]
    fn can_pillager_patrol_spawn(&self, pos: [i32; 3]) -> bool {
        (self.can_spawn)(self.level.game, pos)
    }
    #[inline]
    fn effective_difficulty(&self, pos: [i32; 3]) -> f32 {
        self.level
            .game
            .local_difficulty([f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])])
            .effective
    }
    #[inline]
    fn motion_blocking_no_leaves(&self, x: i32, z: i32) -> i32 {
        self.level
            .heightmap(Heightmap::MotionBlockingNoLeaves, x, z)
    }
}

pub(in crate::game::server_side) fn block_of(p: Vec3) -> [i32; 3] {
    [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
}

impl crate::game::ServerGame<'_> {
    /// `Heightmap.Types.WORLD_SURFACE`: the first free block above the
    /// highest non-air one.
    pub(in crate::game::server_side) fn surface_height(&self, x: i32, z: i32) -> i32 {
        let top = if self.common.dim == Dim::Nether {
            127
        } else {
            319
        };
        (self.common.world.bottom_y()..=top)
            .rev()
            .find(|&y| {
                !self
                    .common
                    .res
                    .facts()
                    .is_air(self.common.world.block(x, y, z))
            })
            .map_or(self.common.world.bottom_y(), |y| y + 1)
    }
    /// Every claimed village point of interest (job sites, beds, bells).
    pub(in crate::game::server_side) fn village_pois(&self) -> Vec<[i32; 3]> {
        let mut out: Vec<[i32; 3]> = Vec::new();
        for m in &self.common.mobs {
            if m.species() != Species::Villager || !m.alive() {
                continue;
            }
            for p in [m.extra.home, m.extra.bed, m.extra.meeting]
                .into_iter()
                .flatten()
            {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        out
    }
    pub fn difficulty_id(&self) -> i32 {
        use sunrise_world::food::Difficulty;
        match self.common.world.health_rules.difficulty {
            Difficulty::Peaceful => 0,
            Difficulty::Easy => 1,
            Difficulty::Normal => 2,
            Difficulty::Hard => 3,
        }
    }
    pub(in crate::game::server_side) fn player_block(&self) -> Option<[i32; 3]> {
        self.common.player.as_ref().map(|p| block_of(p.position))
    }
    /// `dimensions/<dimension>/data/minecraft/raids.dat`: each dimension
    /// keeps its own raids.
    pub(in crate::game::server_side) fn raids_path(&self, dim: Dim) -> std::path::PathBuf {
        sunrise_sim::save::dimension_dir(&self.server.world_root, dim.id())
            .join("data/minecraft/raids.dat")
    }
    /// The raids of the player's dimension, seeded (the previous dimension's
    /// saved first).
    pub(in crate::game::server_side) fn load_raids(&mut self) {
        self.read_raids();
        if let Some(order) = self.server.raiding.0.unseeded.take() {
            let seed = random_source::unique_seed();
            self.server.raiding.0.raids.seed_loaded(seed, &order);
        }
    }
    /// Reads the raids of the player's dimension from its file (the previous
    /// dimension's saved first), as a level opens (`ServerLevel`'s
    /// constructor). Their own randoms are seeded by `load_raids` on the first
    /// raid tick without drawing from the level's stream.
    pub fn read_raids(&mut self) {
        if self.server.raiding.0.loaded == Some(self.common.dim) {
            return;
        }
        if self.server.raiding.0.loaded.is_some() {
            self.save_raids();
        }
        self.server.raiding.0.loaded = Some(self.common.dim);
        self.server.raiding.0.bar_hurts.clear();
        self.server.raiding.0.homes.clear();
        let (raids, order) = match sunrise_sim::save::read_dat(&self.raids_path(self.common.dim))
            .and_then(|tag| tag.get("data").cloned())
        {
            Some(data) => Raids::from_nbt_unseeded(&data),
            None => (Raids::default(), Vec::new()),
        };
        self.server.raiding.0.raids = raids;
        self.server.raiding.0.unseeded = Some(order);
    }
    /// `Raids` as its dimension's `raids.dat`, when it changed.
    pub fn save_raids(&mut self) {
        let Some(dim) = self.server.raiding.0.loaded else {
            return;
        };
        if !self.server.raiding.0.raids.dirty || !self.common.is_logical_server() {
            return;
        }
        let tag = sunrise_core::nbt::Tag::compound([
            ("data", self.server.raiding.0.raids.to_nbt()),
            (
                "DataVersion",
                sunrise_core::nbt::Tag::Int(sunrise_world::DATA_VERSION),
            ),
        ]);
        if sunrise_sim::save::write_dat(&self.raids_path(dim), tag).is_ok() {
            self.server.raiding.0.raids.dirty = false;
        }
        self.save_raid_omen_position();
    }
    /// `ServerPlayer.raidOmenPosition` from the player data.
    pub(in crate::game::server_side) fn load_raid_omen_position(&mut self) {
        if self.server.raiding.0.omen_loaded {
            return;
        }
        self.server.raiding.0.omen_loaded = true;
        self.server.raiding.0.omen_position = match self
            .common
            .level
            .player_extra
            .as_ref()
            .and_then(|p| p.get("raid_omen_position"))
        {
            Some(sunrise_core::nbt::Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
            _ => None,
        };
    }
    /// Writes `raid_omen_position` into the player data being saved.
    pub(in crate::game::server_side) fn save_raid_omen_position(&mut self) {
        if !self.server.raiding.0.omen_loaded {
            return;
        }
        let extra = self
            .common
            .level
            .player_extra
            .get_or_insert_with(|| sunrise_core::nbt::Tag::Compound(Default::default()));
        if let sunrise_core::nbt::Tag::Compound(map) = extra {
            match self.server.raiding.0.omen_position {
                Some(p) => {
                    map.insert(
                        "raid_omen_position".into(),
                        sunrise_core::nbt::Tag::IntArray(p.to_vec()),
                    );
                }
                None => {
                    map.remove(&"raid_omen_position".into());
                }
            }
        }
    }
    /// The world's attributes (`WorldTables::gameplay`); a game with no world has none.
    pub(in crate::game::server_side) fn gameplay_attributes(
        &self,
    ) -> std::sync::Arc<GameplayAttributes> {
        self.server
            .world_tables
            .0
            .as_ref()
            .map(|tables| tables.gameplay.clone())
            .unwrap_or_default()
    }
    /// A boolean gameplay attribute at a position (dimension, biome, then
    /// the timelines at the overworld clock).
    pub(in crate::game::server_side) fn gameplay_attribute(
        &mut self,
        key: &str,
        at: [i32; 3],
    ) -> bool {
        let attributes = self.gameplay_attributes();
        let biome = self
            .common
            .world
            .chunks
            .get(&(at[0] >> 4, at[2] >> 4))
            .map(|c| {
                self.common
                    .res
                    .biomes()
                    .get(c.biome(at[0], at[1], at[2]))
                    .name
                    .clone()
            })
            .unwrap_or_default();
        attributes.value(
            key,
            &self.dimension_type().id,
            &biome,
            self.common.world.day_time,
        )
    }

    /// `BadOmenMobEffect` and `RaidOmenMobEffect.applyEffectTick` for the
    /// player's effect tick: Bad Omen inside a village becomes Raid Omen,
    /// whose last tick starts or extends a raid where it began.
    pub(in crate::game) fn omen_tick(&mut self, e: TickingEffect) -> OmenTick {
        if self.common.spectator() || self.common.player.is_none() {
            return OmenTick::Continue;
        }
        let Some(pos) = self.player_block() else {
            return OmenTick::Continue;
        };
        self.load_raids();
        self.load_raid_omen_position();
        match e.id {
            "bad_omen" => {
                if self.difficulty_id() != 0
                    && sections_to_village(&self.village_centers(), section(pos)) <= 1
                    && self
                        .server
                        .raiding
                        .0
                        .raids
                        .nearby(pos)
                        .is_none_or(|(_, r)| r.omen_level < raid::MAX_OMEN_LEVEL)
                {
                    self.server.raiding.0.omen_position = Some(pos);
                    return OmenTick::Add(EffectInstance::new("raid_omen", 600, e.amplifier));
                }
                OmenTick::Continue
            }
            "raid_omen" => match self.server.raiding.0.omen_position.take() {
                Some(at) => {
                    self.create_or_extend_raid(at, Some(e.amplifier));
                    OmenTick::Remove
                }
                None => OmenTick::Continue,
            },
            _ => OmenTick::Continue,
        }
    }
    /// `Raids.createOrExtendRaid`: not for spectators, with the `raids` rule
    /// and where raids may start; the raid centres on the occupied village
    /// points of interest within 64 blocks.
    pub fn create_or_extend_raid(&mut self, at: [i32; 3], amplifier: Option<i32>) -> Option<i32> {
        if self.common.spectator()
            || !self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::RAIDS)
            || !self.gameplay_attribute(sunrise_sim::patrol::CAN_START_RAID, at)
        {
            return None;
        }
        self.load_raids();
        let center = raid::raid_center(at, &self.village_pois());
        // The new raid's own random: `RandomSource.create()`, not the level's.
        let seed = random_source::unique_seed();
        let difficulty = self.difficulty_id();
        let (id, triggered) = self
            .server
            .raiding
            .0
            .raids
            .create_or_extend(center, difficulty, amplifier, seed);
        if triggered {
            self.award_custom_stat("raid_trigger", 1);
            self.trigger_player("minecraft:voluntary_exile");
            if let Some(level) = self.level(self.common.dim.id()) {
                self.native_raid_state_change(
                    mod_api::RaidRef::new(level, id),
                    center,
                    "triggered",
                );
            }
        }
        Some(id)
    }
    /// The raids of the level in place, once they are read (`read_raids`).
    pub(in crate::game::server_side) fn raids_in_place(&self) -> Option<&Raids> {
        self.server.raiding.0.raids(self.common.dim.id())
    }
    /// `ServerLevel.getRaidAt`.
    pub(in crate::game::server_side) fn raiding_nearby(&self, at: [i32; 3]) -> Option<i32> {
        self.server.raiding.0.raids.nearby(at).map(|(id, _)| id)
    }
    /// `/raid start`: a raid in the local village at the given omen level.
    pub fn raid_command_create(&mut self, at: [i32; 3], omen_level: i32) -> bool {
        self.load_raids();
        let Some(id) = self.create_or_extend_raid(at, None) else {
            return false;
        };
        if let Some(r) = self.server.raiding.0.raids.get_mut(id) {
            r.omen_level = omen_level;
        }
        self.server.raiding.0.raids.dirty = true;
        true
    }
    pub fn raid_command_stop(&mut self, id: i32) {
        if let Some(center) = self.server.raiding.0.raids.stop(id)
            && let Some(level) = self.level(self.common.dim.id())
        {
            self.native_raid_state_change(mod_api::RaidRef::new(level, id), center, "stopped");
        }
    }
    pub(in crate::game::server_side) fn raid_set_omen(&mut self, id: i32, level: i32) -> i32 {
        self.server
            .raiding
            .0
            .raids
            .set_omen_level(id, level)
            .unwrap_or(0)
    }
    pub(in crate::game::server_side) fn raid_glow(&mut self, id: i32) {
        let targets: Vec<u32> = self
            .common
            .mobs
            .iter()
            .filter(|m| raider::state(m).is_some_and(|s| s.raid_id == Some(id)))
            .map(|m| m.id)
            .collect();
        for mob_id in targets {
            self.add_mob_effect(
                mob_id,
                sunrise_level::effects::EffectInstance::new("glowing", 1000, 1),
                None,
            );
        }
    }
    /// The `/raid check` summary line.
    pub(in crate::game::server_side) fn raid_status_text(&self, id: i32) -> String {
        let Some(r) = self.server.raiding.0.raids.get(id) else {
            return String::new();
        };
        let members: Vec<&Mob> = self.raid_members(id).collect();
        let health: f32 = members.iter().fold(0.0, |a, m| a + m.health);
        // Java's string concatenation of the floats: `20.0`, not `20`.
        format!(
            "Num groups spawned: {} Raid omen level: {} Num mobs: {} Raid health: {} / {}",
            r.groups_spawned,
            r.omen_level,
            members.len(),
            sunrise_core::java_number::float_to_string(health),
            sunrise_core::java_number::float_to_string(r.total_health)
        )
    }
    /// A raid's members (`groupRaiderMap`): the living raiders it counts.
    pub fn raid_members(&self, id: i32) -> impl Iterator<Item = &Mob> {
        members(&self.common.mobs, id)
    }
    /// `getHealthOfLivingRaiders`, leaving out one raider.
    pub(in crate::game::server_side) fn remaining_raider_health(
        &self,
        id: i32,
        except: Option<u32>,
    ) -> f32 {
        self.raid_members(id)
            .filter(|m| Some(m.id) != except)
            .fold(0.0, |a, m| a + m.health)
    }
    /// The bar update the last hurt raider of a raid asked for
    /// (`Raider.hurtServer` runs `updateBossbar` before the damage).
    pub(in crate::game::server_side) fn flush_raid_bar(&mut self, id: i32) {
        let seen = self
            .server
            .raiding
            .0
            .bar_hurts
            .get(&id)
            .copied()
            .unwrap_or(0);
        let last = self
            .raid_members(id)
            .filter_map(|m| raider::state(m).map(|s| (s.hurt_seq, m.id, s.pre_hurt_health)))
            .filter(|h| h.0 > seen)
            .max_by_key(|h| h.0);
        let Some((seq, mob, before)) = last else {
            return;
        };
        let living = self.remaining_raider_health(id, Some(mob)) + before;
        if let Some(r) = self.server.raiding.0.raids.get_mut(id) {
            r.update_bossbar(living);
        }
        self.server.raiding.0.bar_hurts.insert(id, seq);
    }

    /// The members of every raid, for `Raids.tick`.
    pub(in crate::game::server_side) fn raider_infos(&self) -> HashMap<i32, Vec<RaiderInfo>> {
        let mut out: HashMap<i32, Vec<RaiderInfo>> = HashMap::new();
        for m in &self.common.mobs {
            if !m.alive() || m.removed {
                continue;
            }
            let Some(s) = raider::state(m) else { continue };
            let Some(id) = s.raid_id else { continue };
            let position = block_of(m.position);
            out.entry(id).or_default().push(RaiderInfo {
                id: m.id,
                wave: s.wave,
                health: m.health,
                position,
                tick_count: m.age as i32,
                no_action_time: m.ai.no_action_time,
                ticks_outside: s.ticks_outside_raid,
                removed: false,
                other_dimension: false,
                in_level: true,
                patrol_leader: s.patrol_leader,
            });
        }
        out
    }
    /// Raiders loaded with a `RaidId` join their raid
    /// (`readAdditionalSaveData`: `addWaveMob` without counting health, and
    /// the wave's leader).
    pub(in crate::game::server_side) fn register_loaded_raiders(&mut self) {
        let pending: Vec<(u32, i32, i32, f32, bool)> = self
            .common
            .mobs
            .iter()
            .filter(|m| m.alive() && !m.removed)
            .filter_map(|m| {
                let s = raider::state(m)?;
                (!s.registered).then_some(())?;
                Some((m.id, s.raid_id?, s.wave, m.health, s.patrol_leader))
            })
            .collect();
        for (mob, id, wave, health, leader) in pending {
            let living = self.remaining_raider_health(id, None);
            let known = match self.server.raiding.0.raids.get_mut(id) {
                Some(r) => {
                    r.add_wave_mob(health, false, living);
                    if leader {
                        r.set_leader(wave, mob);
                    }
                    self.server.raiding.0.raids.dirty = true;
                    true
                }
                None => false,
            };
            if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) {
                let s = raider::state_mut(m);
                s.registered = true;
                if !known {
                    s.raid_id = None;
                }
            }
        }
    }
    /// `ServerLevel.tick` for the raids and their raiders.
    pub fn tick_raids(&mut self) {
        if !self.common.is_logical_server() {
            return;
        }
        self.load_raids();
        self.register_loaded_raiders();
        for id in self.server.raiding.0.raids.ids() {
            self.flush_raid_bar(id);
        }
        let centers = self.village_centers();
        let infos = self.raider_infos();
        let old_states: HashMap<_, _> = self
            .server
            .raiding
            .0
            .raids
            .ids()
            .into_iter()
            .filter_map(|id| {
                self.server
                    .raiding
                    .0
                    .raids
                    .get(id)
                    .map(|r| (id, (r.status, r.center)))
            })
            .collect();
        let mut raids = std::mem::take(&mut self.server.raiding.0.raids);
        let actions = {
            let lvl = Lvl {
                game: self,
                difficulty: self.difficulty_id(),
                centers: &centers,
            };
            let members = |id: i32| infos.get(&id).cloned().unwrap_or_default();
            raids.tick(
                &lvl,
                self.common
                    .ui
                    .rules
                    .bool_at(sunrise_game_data::gamerules::vanilla::RAIDS),
                &members,
            )
        };
        self.server.raiding.0.raids = raids;
        for (id, (old, _)) in old_states {
            if let Some(r) = self.server.raiding.0.raids.get(id)
                && r.status != old
                && let Some(level) = self.level(self.common.dim.id())
            {
                self.native_raid_state_change(
                    mod_api::RaidRef::new(level, id),
                    r.center,
                    r.status.name(),
                );
            }
        }
        for (id, action) in actions {
            self.apply_raid_action(id, action);
        }
        self.sync_raiders(&centers);
        let infos = self.raider_infos();
        self.sync_villager_raids(&infos);
    }
    pub fn apply_raid_action(&mut self, id: i32, action: Action) {
        match action {
            Action::Spawn { pos, wave, spawns } => self.spawn_wave(id, pos, wave, &spawns),
            Action::Horn { pos, seed } => self.raid_horn(id, pos, seed),
            Action::Remove { id: mob } => {
                if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) {
                    let s = raider::state_mut(m);
                    s.raid_id = None;
                    s.active_raid = false;
                }
            }
            Action::SetOutside { id: mob, ticks } => {
                if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) {
                    raider::state_mut(m).ticks_outside_raid = ticks;
                }
            }
            Action::Victory { heroes, amplifier } => self.for_each_raid_hero(&heroes, |g| {
                let mut effect =
                    EffectInstance::new("hero_of_the_village", raid::HERO_DURATION, amplifier);
                effect.visible = false;
                g.add_player_effect(effect);
                g.award_custom_stat("raid_win", 1);
                g.trigger_player("minecraft:hero_of_the_village");
            }),
        }
    }

    /// Raid.tick resolves and rewards each living hero in the set's traversal
    /// order, not in the level's player order.
    pub(in crate::game::server_side) fn for_each_raid_hero(
        &mut self,
        heroes: &[[i32; 4]],
        mut reward: impl FnMut(&mut Self),
    ) {
        for &hero in heroes {
            let uuid = ints_uuid(hero);
            let acting_uuid = self
                .server
                .lan_acting_uuid
                .or_else(|| self.common.level.player_uuid.map(ints_uuid));
            if let Some(p) = self.common.player.as_ref()
                && acting_uuid == Some(uuid)
            {
                if !self.server.lan_acting_fake
                    && !self.common.spectator()
                    && !p.health.dead_or_dying()
                {
                    reward(self);
                }
                continue;
            }
            if let Some((i, conn)) = self.server.lan.as_ref().and_then(|host| {
                host.hub.guests.iter().enumerate().find_map(|(i, g)| {
                    (g.uuid == uuid
                        && host.hub.here(g)
                        && g.loaded
                        && !g.dead
                        && g.health > 0.0
                        && !g.spectator())
                    .then_some((i, g.conn))
                })
            }) {
                let _ = self.as_guest_at(i, conn, &mut reward);
            }
        }
    }
    /// `Raid.playSound`: the horn, 13 blocks from each player toward the wave,
    /// for everyone within 64 blocks or in the raid. One seed for every packet
    /// (`SoundSource.NEUTRAL`, volume 64, pitch 1).
    pub(in crate::game::server_side) fn raid_horn(&mut self, id: i32, pos: [i32; 3], seed: i64) {
        let in_raid = self
            .server
            .raiding
            .0
            .raids
            .get(id)
            .is_some_and(|r| r.bar.player);
        let (rx, rz) = (f64::from(pos[0]) + 0.5, f64::from(pos[2]) + 0.5);
        // The raid tracks one participant (`bar.player`): the first player in
        // the level. Everyone else is heard by distance alone.
        let local_first = self.common.player.is_some();
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let dim = host.hub.config.dimension.clone();
        let spotted: Vec<(u32, [f64; 3], bool)> = host
            .hub
            .guests
            .iter()
            .map(|g| (g.conn, g.position, g.placed && g.dimension == dim))
            .collect();
        let neutral = sunrise_level::sim_settings::SOUND_SOURCES
            .iter()
            .position(|s| *s == "neutral")
            .unwrap_or(0) as i32;
        let mut first_guest = !local_first;
        for (conn, position, here) in spotted {
            if !here {
                continue;
            }
            let participating = std::mem::replace(&mut first_guest, false) && in_raid;
            let Some(at) = raid::horn_at(position, rx, rz, participating) else {
                continue;
            };
            let packet =
                host.hub
                    .sound_packet("minecraft:event.raid.horn", at, 64.0, 1.0, neutral, seed);
            host.hub.send_to(conn, &packet);
        }
    }
    /// `Raid.spawnGroup`: a wave of raiders at `pos`, the leader wearing the
    /// banner, ravagers carrying their riders.
    pub(in crate::game::server_side) fn spawn_wave(
        &mut self,
        raid_id: i32,
        pos: [i32; 3],
        wave: i32,
        spawns: &[Spawn],
    ) {
        let odds = self
            .server
            .raiding
            .0
            .raids
            .get(raid_id)
            .map_or(0.0, raid::Raid::enchant_odds);
        for sp in spawns {
            let Some(id) = self.spawn_raider(sp.kind, pos, raid_id, wave, sp.leader, odds) else {
                continue;
            };
            if sp.leader
                && let Some(r) = self.server.raiding.0.raids.get_mut(raid_id)
            {
                r.set_leader(wave, id);
            }
            if let Some(kind) = sp.rider
                && let Some(rider) = self.spawn_raider(kind, pos, raid_id, wave, false, odds)
            {
                self.mount_mob(rider, id);
            }
        }
        self.native_raid_spawn_wave(raid_id, pos, wave, spawns.len());
    }
    /// `Raid.joinRaid` for a new raider: placed at the block's centre one
    /// above, finalized for an event, its raid buffs applied.
    pub(in crate::game::server_side) fn spawn_raider(
        &mut self,
        kind: RaiderType,
        pos: [i32; 3],
        raid_id: i32,
        wave: i32,
        leader: bool,
        odds: f32,
    ) -> Option<u32> {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 1.0,
            f64::from(pos[2]) + 0.5,
        ];
        let id = self.spawn_event_mob(kind.entity(), at, leader, raider::Reason::Event)?;
        let m = self.common.mobs.iter_mut().find(|m| m.id == id)?;
        let s = raider::state_mut(m);
        s.raid_id = Some(raid_id);
        s.wave = wave;
        s.can_join_raid = true;
        s.ticks_outside_raid = 0;
        s.active_raid = true;
        s.registered = true;
        m.ground.on_ground = true;
        self.apply_raid_buffs(id, kind, wave, odds);
        Some(id)
    }
    /// `EntityType.create` then `finalizeSpawn` for a patrol or a raid: the
    /// mob keeps the default rotation; a leader is set before finalizing so
    /// it wears the banner.
    pub(in crate::game::server_side) fn spawn_event_mob(
        &mut self,
        kind: &str,
        at: [f64; 3],
        leader: bool,
        reason: raider::Reason,
    ) -> Option<u32> {
        let t = self.common.res.mob_types().get(kind).cloned()?;
        self.server.next_entity += 1;
        let mut mob = Mob::new(
            self.server.next_entity,
            t,
            Vec3::new(at[0], at[1], at[2]),
            0.0,
        );
        if leader {
            raider::set_patrol_leader(&mut mob, true);
            if reason == raider::Reason::Patrol {
                // findPatrolTarget runs before the mob is placed: the target
                // is around the world origin.
                raider::find_patrol_target_from(&mut mob, [0, 0, 0]);
            }
        }
        let spawn_reason = if reason == raider::Reason::Patrol {
            sunrise_sim::mobs::spawn_rules::Reason::Patrol
        } else {
            sunrise_sim::mobs::spawn_rules::Reason::Event
        };
        if !self.finalize_with(&mut mob, spawn_reason, None) {
            return None;
        }
        let id = mob.id;
        self.common.mobs.push(mob);
        self.admit_new_mobs(self.common.mobs.len() - 1);
        Some(id)
    }

    /// Per-tick raid bookkeeping of every raider (and the ravagers they
    /// ride): the raid's state for the goals, joining and recruiting,
    /// the banner of a fallen captain, idle time.
    pub(in crate::game::server_side) fn sync_raiders(
        &mut self,
        centers: &HashSet<(i32, i32, i32)>,
    ) {
        let now = self.common.world.game_time;
        let golems: HashSet<u32> = self
            .common
            .mobs
            .iter()
            .filter(|m| m.species() == Species::IronGolem)
            .map(|m| m.id)
            .collect();
        // Living leaders and carried riders.
        let mut leaders: HashSet<(i32, i32)> = HashSet::new();
        let mut ridden: HashSet<u32> = HashSet::new();
        let mut positions: Vec<(u32, Vec3, bool)> = Vec::new();
        for m in &self.common.mobs {
            let Some(s) = raider::state(m) else { continue };
            if !m.alive() {
                continue;
            }
            if let (Some(id), true) = (s.raid_id, s.patrol_leader) {
                leaders.insert((id, s.wave));
            }
            if let Some(v) = m.extra.mount {
                ridden.insert(v);
            }
            positions.push((m.id, m.position, s.active_raid));
        }
        let banners: Vec<(usize, u32, Vec3)> = self
            .common
            .dropped
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                &*e.stack.item == raider::BANNER
                    && e.stack.data().is_some_and(|d| d.ominous_banner)
                    && e.pickup_delay == 0
            })
            .map(|(i, e)| (i, e.id, e.position))
            .collect();
        let mut taken: Vec<usize> = Vec::new();
        let mut collectors: Vec<(usize, u32)> = Vec::new();
        let mut recruit: Vec<(i32, Vec3)> = Vec::new();
        let mut joins: Vec<(u32, i32, f32)> = Vec::new();
        let mut new_leaders: Vec<(i32, i32, u32)> = Vec::new();
        let homes = self.raid_homes();
        for m in &mut self.common.mobs {
            let species = m.species();
            if !raider::is_raider(species) {
                continue;
            }
            let alive = m.alive();
            let position = m.position;
            let block = block_of(position);
            let in_village = sections_to_village(centers, section(block)) <= 1;
            let has_rider = ridden.contains(&m.id);
            let target_golem =
                matches!(m.ai.target, Some(sunrise_sim::mobs::Who::Mob(t)) if golems.contains(&t));
            let target_player = m.ai.target == Some(sunrise_sim::mobs::Who::Player);
            let (health, range) = (m.health, m.attrs.follow_range);
            let raids = &mut self.server.raiding.0.raids;
            let wears_banner = m.extra.head_id() == Some(raider::BANNER);
            let s = m.extra.raider.get_or_insert_with(Box::default);
            s.rider = has_rider;
            s.in_village = in_village;
            // Vindicator.customServerAiStep: `ServerLevel.isRaided`.
            s.raided = species == Species::Vindicator && raids.nearby(block).is_some();
            if s.raid_id.is_some_and(|id| raids.get(id).is_none()) {
                s.raid_id = None;
                s.ticks_outside_raid = 0;
            }
            match s.raid_id.and_then(|id| raids.get(id)) {
                Some(r) => {
                    s.active_raid = r.active;
                    s.raid_over = r.is_over();
                    s.raid_loss = r.is_loss();
                    s.raid_center = r.center;
                    s.homes = s.raid_id.and_then(|id| homes.get(&id).cloned());
                }
                None => {
                    s.active_raid = false;
                    s.raid_over = false;
                    s.raid_loss = false;
                    s.homes = None;
                    s.banner_at = None;
                }
            }
            if !alive {
                continue;
            }
            // Raider.aiStep: a raider fighting a player or golem in a raid is
            // never idle. The per-tick growth (+1 serverAiStep, +2
            // Raider.updateNoActionTime) is added by the mob's own step.
            if s.raid_id.is_some() && (target_player || target_golem) {
                m.ai.no_action_time = 0;
            }
            // Raider.aiStep: join the raid the raider stands in.
            if s.raid_id.is_none()
                && s.can_join_raid
                && now % 20 == 0
                && m.ai.no_action_time <= 2400
                && let Some((id, _)) = raids.nearby(block)
            {
                joins.push((m.id, id, health));
            }
            // PathfindToRaidGoal.tick: recruitNearby, only while the goal runs.
            if std::mem::take(&mut s.recruit_due)
                && s.active_raid
                && let Some(id) = s.raid_id
            {
                recruit.push((id, position));
            }
            // ObtainRaidLeaderBannerGoal.
            s.banner_at = None;
            if let Some(id) = s.raid_id
                && s.active_raid
                && !s.raid_over
                && raider::can_be_leader(species)
                && !wears_banner
                && raids.get(id).is_some_and(|r| r.leader(s.wave).is_none())
                && !leaders.contains(&(id, s.wave))
            {
                let in_reach = |(i, _, p): &&(usize, u32, Vec3)| {
                    !taken.contains(i)
                        && (p.x - position.x).abs() <= range
                        && (p.z - position.z).abs() <= range
                        && (p.y - position.y).abs() <= 8.0
                };
                let by_distance = |a: &&(usize, u32, Vec3), b: &&(usize, u32, Vec3)| {
                    a.2.distance_squared(position)
                        .total_cmp(&b.2.distance_squared(position))
                };
                // The unreachable cache only picks the navigation target;
                // a banner next to the raider is still picked up.
                let near = banners
                    .iter()
                    .filter(in_reach)
                    .filter(|(_, item, _)| {
                        !s.unreachable_banners
                            .iter()
                            .any(|&(b, until)| b == *item && now < until)
                    })
                    .min_by(by_distance);
                let nearest = banners.iter().filter(in_reach).min_by(by_distance);
                let adjacent =
                    nearest.filter(|(_, _, p)| p.distance_squared(position) < 1.414 * 1.414);
                if let Some((i, item, p)) = adjacent.or(near) {
                    s.banner_at = Some([p.x, p.y, p.z]);
                    s.banner_item = *item;
                    if p.distance_squared(position) < 1.414 * 1.414 {
                        // pickUpItem: the banner becomes the wave's leader.
                        taken.push(*i);
                        collectors.push((*i, m.id));
                        leaders.insert((id, s.wave));
                        new_leaders.push((id, s.wave, m.id));
                        s.banner_at = None;
                        s.patrol_leader = true;
                        s.patrolling = true;
                        // wear_banner: head id + ominous stack for SetEquipment (#4567).
                        raider::wear_banner(m);
                    }
                }
            }
        }
        for (i, mob) in collectors {
            let item = self.common.dropped[i].clone();
            self.server_mob_take_item(&item, mob);
        }
        taken.sort_unstable();
        for i in taken.into_iter().rev() {
            self.remove_dropped(i);
        }
        for (id, wave, mob) in new_leaders {
            if let Some(r) = self.server.raiding.0.raids.get_mut(id) {
                r.set_leader(wave, mob);
            }
        }
        // PathfindToRaidGoal.recruitNearby: raiders nearby in no active raid
        // join this one at its current wave.
        for (id, at) in recruit {
            for (mid, p, active) in &positions {
                if !active
                    && (p.x - at.x).abs() <= 16.0
                    && (p.y - at.y).abs() <= 16.0
                    && (p.z - at.z).abs() <= 16.0
                    && let Some(m) = self.common.mobs.iter().find(|m| m.id == *mid)
                    && raider::state(m).is_some_and(|s| s.can_join_raid && s.raid_id != Some(id))
                    && m.alive()
                    && m.ai.no_action_time <= 2400
                {
                    joins.push((*mid, id, m.health));
                }
            }
        }
        for (mob, id, health) in joins {
            self.join_raid(mob, id, health);
        }
    }
    /// `Raid.joinRaid` for a raider already in the world: it counts toward
    /// the bar and takes the raid's current wave.
    pub(in crate::game::server_side) fn join_raid(&mut self, mob: u32, id: i32, health: f32) {
        let Some(wave) = self
            .server
            .raiding
            .0
            .raids
            .get(id)
            .map(|r| r.groups_spawned)
        else {
            return;
        };
        let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) else {
            return;
        };
        let s = raider::state_mut(m);
        if s.raid_id == Some(id) {
            return;
        }
        s.raid_id = Some(id);
        s.wave = wave;
        s.can_join_raid = true;
        s.ticks_outside_raid = 0;
        s.registered = true;
        let living = self.remaining_raider_health(id, None);
        if let Some(r) = self.server.raiding.0.raids.get_mut(id) {
            r.add_wave_mob(health, true, living);
        }
        self.server.raiding.0.raids.dirty = true;
    }
    /// Beds within 64 blocks of each raid's centre (the `HOME` points of
    /// interest), rescanned every five seconds.
    pub(in crate::game::server_side) fn raid_homes(
        &mut self,
    ) -> HashMap<i32, std::sync::Arc<Vec<[i32; 3]>>> {
        let now = self.common.world.game_time;
        let ids: Vec<(i32, [i32; 3])> = self
            .server
            .raiding
            .0
            .raids
            .map
            .iter()
            .filter(|(_, r)| r.active && !r.is_over() && r.started)
            .map(|(id, r)| (*id, r.center))
            .collect();
        self.server
            .raiding
            .0
            .homes
            .retain(|id, _| ids.iter().any(|(i, _)| i == id));
        for (id, c) in ids {
            if self
                .server
                .raiding
                .0
                .homes
                .get(&id)
                .is_some_and(|(at, _)| now - at < 100)
            {
                continue;
            }
            let mut beds = Vec::new();
            let mut known: HashMap<u32, bool> = HashMap::new();
            for y in c[1] - 32..=c[1] + 32 {
                for z in c[2] - 64..=c[2] + 64 {
                    for x in c[0] - 64..=c[0] + 64 {
                        let state = self.common.world.block(x, y, z);
                        if self.common.res.facts().is_air(state) {
                            continue;
                        }
                        let bed =
                            *known.entry(state.0).or_insert_with(|| {
                                self.common.res.facts().get(state).name.ends_with("_bed")
                                    && self.common.pack.assets.registry.state(state).is_some_and(
                                        |s| {
                                            s.properties.get("part").map(String::as_str)
                                                == Some("head")
                                        },
                                    )
                            });
                        if bed {
                            beds.push([x, y, z]);
                        }
                    }
                }
            }
            self.server
                .raiding
                .0
                .homes
                .insert(id, (now, std::sync::Arc::new(beds)));
        }
        self.server
            .raiding
            .0
            .homes
            .iter()
            .map(|(id, (_, v))| (*id, v.clone()))
            .collect()
    }
    /// `Raider.die`: the wave loses its leader, the killing player becomes
    /// a hero of the village, and the raider leaves the raid (its health
    /// stays in the bar's total).
    pub(in crate::game::server_side) fn raider_died(
        &mut self,
        m: &Mob,
        by_player: bool,
        killer: Option<u32>,
    ) {
        let Some(s) = raider::state(m) else {
            return;
        };
        let Some(id) = s.raid_id else {
            return;
        };
        let (leader, wave) = (s.patrol_leader, s.wave);
        self.flush_raid_bar(id);
        let living = self.remaining_raider_health(id, Some(m.id));
        let uuid = self.player_uuid_of(killer);
        if let Some(r) = self.server.raiding.0.raids.get_mut(id) {
            if leader {
                r.remove_leader(wave);
            }
            if by_player && let Some(uuid) = uuid {
                r.add_hero(uuid);
            }
            r.remove_from_raid(m.health, false, living);
            self.server.raiding.0.raids.dirty = true;
        }
        if let Some(dead) = self.common.mobs.iter_mut().find(|x| x.id == m.id) {
            raider::state_mut(dead).raid_id = None;
        }
    }

    /// What `getRaidAt` tells each villager where it stands (its brain's
    /// `SetRaidStatus`, `ResetRaidStatus`, `ReactToBell` and raid packages
    /// read it).
    pub(in crate::game::server_side) fn sync_villager_raids(
        &mut self,
        infos: &HashMap<i32, Vec<RaiderInfo>>,
    ) {
        let raids = &self.server.raiding.0.raids;
        let mut ongoing = Vec::new();
        for m in &mut self.common.mobs {
            if m.species() != Species::Villager {
                continue;
            }
            let at = block_of(m.position);
            let (status, wave) = match raids.nearby(at) {
                Some((id, r)) => {
                    let alive = infos.get(&id).map_or(0, Vec::len);
                    let wave = r.has_first_wave_spawned() && !r.is_between_waves(alive);
                    let status = if r.is_victory() {
                        mobs_villager::RAID_VICTORY
                    } else if r.is_loss() {
                        mobs_villager::RAID_LOSS
                    } else if wave {
                        mobs_villager::RAID
                    } else {
                        mobs_villager::PRE_RAID
                    };
                    (status, wave)
                }
                None => (mobs_villager::RAID_NONE, false),
            };
            let ext = mobs_villager::ext_mut(m);
            ext.raid = status;
            ext.raid_wave = wave;
            if !m.no_ai && matches!(status, mobs_villager::PRE_RAID | mobs_villager::RAID) {
                ongoing.push(m.id);
            }
        }
        // Villager.customServerAiStep: a 1 in 100 chance each tick, in an
        // ongoing raid, of entity event 42 (`handleEntityEvent` splashes).
        for id in ongoing {
            if self.server.effect_rng.next_int_bound(100) == Ok(0) {
                self.mob_raid_splash(id);
            }
        }
    }
    /// `OminousBottleAmplifier.onConsume`: Bad Omen for 100 minutes at the
    /// bottle's amplifier. The consumable list owns the dispose sound.
    pub(in crate::game::server_side) fn drink_ominous_bottle(
        &mut self,
        stack: &sunrise_items::items::ItemStack,
    ) {
        let amplifier = stack.data().and_then(|d| d.ominous_amplifier).unwrap_or(0);
        let mut effect =
            sunrise_level::effects::EffectInstance::new("bad_omen", 120_000, i32::from(amplifier));
        effect.visible = false;
        self.add_player_effect(effect);
    }
    /// Entity event 42 for villager `id`: the guests get the byte, and this
    /// game draws `AbstractVillager.addParticlesAroundSelf(SPLASH)` itself.
    pub(in crate::game::server_side) fn mob_raid_splash(&mut self, id: u32) {
        let Some(m) = self.common.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let at = [m.position.x, m.position.y, m.position.z];
        self.lan_entity_event(id as i32, 42, at);
        self.common.particles_around_mob(
            id,
            "splash",
            sunrise_level::particle_options::Burst::VILLAGER,
        );
    }

    /// The raid bars a player `uuid` is in (`Raid.getPlayers`): red, notched
    /// in tens. The set is the one `updatePlayers` cached on the last 20-tick
    /// beat, so a viewer joins or leaves it up to 19 ticks late, alive or not
    /// (`validPlayer`'s `isAlive` is only read on a beat). The name is the
    /// translate component the client draws.
    pub fn raid_bars_at(&self, uuid: u128) -> Vec<(i32, Tag, f32)> {
        if self.common.dim != Dim::Overworld {
            return Vec::new();
        }
        self.server
            .raiding
            .0
            .raids
            .ids()
            .into_iter()
            .filter_map(|id| self.server.raiding.0.raids.get(id).map(|raid| (id, raid)))
            .filter(|(_, raid)| {
                raid.bar.players.contains(&uuid) && raid.bar.visible && !raid.is_stopped()
            })
            .map(|(id, raid)| (id, raid_name_tag(raid.bar.name), raid.bar.progress))
            .collect()
    }
    /// A firework a villager launches to celebrate (`CelebrateVillagersSurvivedRaid`).
    pub(in crate::game::server_side) fn villager_firework(
        &mut self,
        at: [f64; 3],
        color: usize,
        flight: u8,
    ) {
        let mut stack = sunrise_items::items::ItemStack::new("minecraft:firework_rocket", 1);
        stack.data_mut().fireworks = Some((
            flight,
            vec![sunrise_items::firework::FireworkExplosion {
                shape: sunrise_items::firework::FireworkShape::Burst,
                colors: vec![sunrise_items::firework::DYE_FIREWORK_COLORS[color & 15].1],
                fade: Vec::new(),
                trail: false,
                twinkle: false,
            }],
        ));
        let rocket = self.new_rocket(
            Vec3::new(at[0], at[1], at[2]),
            &stack,
            false,
            &mut random_source::entity(),
        );
        self.common.rockets.push(rocket);
    }
    /// `BehaviorUtils.throwItem`: a hero's gift tossed toward the player.
    pub(in crate::game::server_side) fn throw_gift(
        &mut self,
        table: &str,
        from: [f64; 3],
        to: [f64; 3],
    ) {
        let drops =
            self.common
                .res
                .items()
                .loot_table(table, false, None, &mut self.common.world.random);
        let d = Vec3::new(to[0] - from[0], to[1] - from[1], to[2] - from[2]).normalize();
        for stack in drops {
            self.spawn_item(stack, from, [d.x * 0.3, d.y * 0.3, d.z * 0.3], 10);
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// Sections holding a claimed village point of interest (a job site,
    /// bed or bell a villager has taken): `PoiManager.isVillageCenter`.
    pub fn village_centers(&self) -> HashSet<(i32, i32, i32)> {
        let mut out = HashSet::new();
        let world = self.mob_world_base(&[]);
        for m in &self.common.mobs {
            if m.species() != Species::Villager || !m.alive() {
                continue;
            }
            // A remembered HOME is no longer an occupied village POI once its
            // loaded bed head disappears. An unavailable chunk remains unknown.
            let bed = m.extra.bed.filter(|&[x, y, z]| {
                !self.common.world.loaded(x, z)
                    || sunrise_sim::mobs::villager_brain::poi_at(&world, x, y, z)
                        == Some(sunrise_world::poi::PoiIdentity::Vanilla(
                            sunrise_world::poi::PoiType::Home,
                        ))
            });
            for p in [m.extra.home, bed, m.extra.meeting].into_iter().flatten() {
                out.insert(section(p));
            }
        }
        out
    }
    /// `PatrolSpawner.tick` (`sunrise_client::patrol`) on the game level:
    /// the pillagers it accepts are created after the pass, finalized for a
    /// patrol.
    pub fn tick_patrol_spawner(&mut self) {
        if self.common.dim != Dim::Overworld {
            return;
        }
        let attributes = self.gameplay_attributes();
        let biome_ok = |game: &crate::game::ServerGame<'_>, at: [i32; 3]| {
            let biome = game
                .world
                .chunks
                .get(&(at[0] >> 4, at[2] >> 4))
                .map(|c| {
                    game.res
                        .biomes()
                        .get(c.biome(at[0], at[1], at[2]))
                        .name
                        .clone()
                })
                .unwrap_or_default();
            attributes.value(
                sunrise_sim::patrol::CAN_PILLAGER_PATROL_SPAWN,
                game.dim.id(),
                &biome,
                game.world.day_time,
            )
        };
        let enemies = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_MOBS)
            && self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_MONSTERS);
        let mut random = std::mem::replace(
            &mut self.common.world.random,
            LevelRandom::for_level(LegacyRandom::new(0)),
        );
        let mut members: Vec<Member> = Vec::new();
        let mut spawner = std::mem::take(&mut self.server.raiding.0.patrol);
        {
            let level = PatrolCx {
                level: self.natural_level(),
                centers: self.village_centers(),
                can_spawn: &biome_ok,
            };
            let mut spawn = |_: &PatrolCx, m: Member, _: &mut sunrise_core::random::LevelRandom| {
                members.push(m);
                true
            };
            if let Some(pillager) = self.common.res.mob_types().get("minecraft:pillager") {
                spawner.tick(&level, enemies, pillager, &mut random, &mut spawn);
            }
        }
        self.server.raiding.0.patrol = spawner;
        self.common.world.random = random;
        for m in members {
            let at = [
                f64::from(m.pos[0]),
                f64::from(m.pos[1]),
                f64::from(m.pos[2]),
            ];
            self.spawn_event_mob("minecraft:pillager", at, m.leader, raider::Reason::Patrol);
        }
    }
    /// `VillageSiege.tick`, run by `VillageSiege` in `sunrise-gameplay`.
    pub fn tick_village_siege(&mut self) {
        let mut siege = std::mem::take(&mut self.server.raiding.0.siege);
        siege.tick(self);
        self.server.raiding.0.siege = siege;
    }
    /// `ServerLevel.isVillage`: within one section of a village.
    pub fn is_village(centers: &HashSet<(i32, i32, i32)>, pos: [i32; 3]) -> bool {
        sunrise_gameplay::village_siege::is_village(centers, pos)
    }
    pub fn level_is_village(
        &self,
        level: mod_api::LevelRef,
        pos: [i32; 3],
    ) -> Result<bool, mod_api::LevelReadError> {
        let world = self.native_level_world_result(level)?;
        let [x, y, z] = pos;
        if level.dimension() != self.common.dim.key()
            || !world.loaded(x, z)
            || !world.shape.contains_y(y)
        {
            return Err(mod_api::LevelReadError::Unavailable);
        }
        Ok(Self::is_village(&self.village_centers(), pos))
    }
}

impl SiegeLevel for ServerGame<'_> {
    fn sky_darken(&self) -> i32 {
        WeatherGameExt::sky_darken(self)
    }
    fn surface_height(&self, x: i32, z: i32) -> i32 {
        crate::game::ServerGame::surface_height(self, x, z)
    }
    fn biome_tag(&self, tag: &str, pos: [i32; 3]) -> bool {
        self.natural_level().biome_tag(tag, pos[0], pos[1], pos[2])
    }
    fn check_zombie_spawn_rules(&mut self, [x, y, z]: [i32; 3]) -> bool {
        let mut random = std::mem::replace(
            &mut self.common.world.random,
            LevelRandom::for_level(LegacyRandom::new(0)),
        );
        let ok = spawn_rules::monster_rules(
            Species::Zombie,
            &self.mob_world_base(&[]),
            spawn_rules::Reason::Event,
            &mut random,
            x,
            y,
            z,
        );
        self.common.world.random = random;
        ok
    }
    fn village_siege_cancelled(&mut self, player: u128, attempted: [f64; 3]) -> bool {
        let mut canceled = false;
        self.native_village_siege(self.common.dim.id(), player, attempted, &mut canceled);
        canceled
    }
}

/// `Raid.raidEvent`'s name: the raid title, the remaining-raiders line, or
/// the victory and defeat lines.
pub(in crate::game::server_side) fn raid_name_tag(name: BarName) -> Tag {
    let key = |k: &str| Tag::compound([("translate", Tag::String(k.into()))]);
    match name {
        BarName::Raid => key("event.minecraft.raid"),
        BarName::Victory => key("event.minecraft.raid.victory.full"),
        BarName::Defeat => key("event.minecraft.raid.defeat.full"),
        BarName::Remaining(n) => Tag::compound([
            ("translate", Tag::String("event.minecraft.raid".into())),
            (
                "extra",
                Tag::List(vec![
                    Tag::String(" - ".into()),
                    Tag::compound([
                        (
                            "translate",
                            Tag::String("event.minecraft.raid.raiders_remaining".into()),
                        ),
                        (
                            "with",
                            Tag::List(vec![Tag::String(n.to_string().as_str().into())]),
                        ),
                    ]),
                ]),
            ),
        ]),
    }
}

impl crate::game::ServerGame<'_> {
    /// Read-only command-harness observation of the existing raid state and inputs.
    pub(in crate::game::server_side) fn raid_fixture_state(
        &self,
        op: &serde_json::Value,
    ) -> sunrise_game_data::Result<serde_json::Value> {
        use serde_json::json;
        let id = op["id"].as_i64().ok_or("raid observation: missing id")? as i32;
        let mut state = json!({"id":id,"present":self.server.raiding.0.raids.get(id).is_some()});
        if let Some(r) = self.server.raiding.0.raids.get(id) {
            state["status"] = json!(r.status.name());
            state["started"] = json!(r.started);
            state["active"] = json!(r.active);
            state["groups_spawned"] = json!(r.groups_spawned);
            state["center"] = json!(r.center);
            // Observe the production answer; never substitute a corrected POI test here.
            state["village"] = json!(Self::is_village(&self.village_centers(), r.center));
            if op["members"].as_bool() == Some(true) {
                state["member_present"] = json!(self.raid_members(id).next().is_some());
            }
        }
        let observe = |m: &Mob| -> sunrise_game_data::Result<serde_json::Value> {
            let uuid = self
                .common
                .persist
                .uuids
                .get(&m.id)
                .copied()
                .ok_or("raid observation: actor UUID is not registered")?;
            let tag = sunrise_sim::entity_nbt::mob_to_nbt(
                m,
                uuid,
                self.common.persist.raw.get(&m.id),
                Some(self.common.pack.assets.registry),
                |other| self.common.persist.uuid_of(other),
                self.common.res.items(),
            );
            Ok(json!({"uuid":uuid,"type":m.kind.id,"health":m.health,
                "pos":[m.position.x,m.position.y,m.position.z],"noai":m.no_ai,
                "home":m.extra.bed.map(|p| p.to_vec()).unwrap_or_default(),
                "raid_id":raider::state(m).and_then(|s|s.raid_id).unwrap_or(-1),
                "nbt":sunrise_game_data::snbt::to_snbt(&tag)}))
        };
        let actors = self
            .raid_members(id)
            .map(observe)
            .collect::<sunrise_game_data::Result<Vec<_>>>()?;
        let mut villagers = Vec::new();
        for uuid in op["villagers"]
            .as_array()
            .ok_or("raid observation: missing villagers")?
        {
            let uuid = u128::from_str_radix(
                &uuid
                    .as_str()
                    .ok_or("raid observation: invalid UUID")?
                    .replace('-', ""),
                16,
            )?;
            let Some(super::commands::Target::Mob(id)) = self.target_by_uuid(uuid) else {
                return Err("raid observation: villager UUID missing/wrong type".into());
            };
            let m = self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id && m.species() == Species::Villager)
                .ok_or("raid observation: actor is not a villager")?;
            villagers.push(observe(m)?);
        }
        let pos: [i32; 3] =
            std::array::from_fn(|i| op["poi"][i].as_i64().unwrap_or(i64::MIN) as i32);
        let loaded = self.common.world.loaded(pos[0], pos[2]);
        let live_poi = if loaded {
            let w = self.mob_world_base(&[]);
            sunrise_sim::mobs::villager_brain::poi_at(&w, pos[0], pos[1], pos[2])
                .and_then(|poi| w.professions.pois.name(poi))
        } else {
            None
        };
        eprintln!(
            "raid input raw: {}",
            json!({"label":op["label"],"id":id,"loaded":loaded,
            "live_poi":live_poi,"claimed_pois":self.village_pois(),
            "raid_nbt":sunrise_game_data::snbt::to_snbt(&self.server.raiding.0.raids.to_nbt()),
            "actors":actors,"villagers":villagers})
        );
        Ok(state)
    }
}

use sunrise_gameplay_common::game::weather::WeatherGameExt;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
