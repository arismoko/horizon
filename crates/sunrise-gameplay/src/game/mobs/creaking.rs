//! The creaking heart block entity in the game: `mobs::creaking_heart`
//! ticks each heart (spawning, letting go of and bleeding for its creaking)
//! and this applies what it reports to the world, the mobs and the speakers.
use std::sync::Arc;
use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};
use sunrise_gameplay_common::entity_uuids::EntityUuids;
use sunrise_gameplay_common::game::creatures::seen;
use sunrise_world::registry::BlockStateId;
use {
    sunrise_game_data::Result, sunrise_level::chunk_map::ChunkMap,
    sunrise_level::chunk_map::ChunkMembershipDelta, sunrise_level::containers::BlockEntity,
    sunrise_level::containers::CreakingHeart, sunrise_level::particle_options::ParticleType,
    sunrise_sim::mobs::Mob, sunrise_sim::mobs::MobType, sunrise_sim::mobs::creaking_heart,
    sunrise_sim::mobs::creaking_heart::HeartEvent, sunrise_sim::mobs::creaking_heart::HeartHost,
};

pub(in crate::game) type Pos = (i32, i32, i32);

/// The game's mobs as a heart sees them, taken out of the game for the tick.
pub struct Host<'a> {
    pub(in crate::game) mobs: &'a mut sunrise_sim::join_seq::JoinList<Mob>,
    pub(in crate::game) added: Vec<Mob>,
    pub(in crate::game) uuids: &'a mut EntityUuids,
    pub(in crate::game) creaking: Option<Arc<MobType>>,
    pub(in crate::game) next_entity: &'a mut u32,
    pub(in crate::game) spawning: bool,
    /// The game's index of `mobs` by id: a heart asks for its creaking a
    /// few times a tick.
    pub(in crate::game) index: std::cell::RefCell<super::mob_index::MobIndex>,
}
impl Host<'_> {
    /// The slot of the live mob `id` in `mobs`, as a scan for the first
    /// one with that id that is not removed (ids are unique in the list).
    pub(in crate::game::server_side) fn slot(&self, id: u32) -> Option<usize> {
        self.index
            .borrow_mut()
            .find(self.mobs, id)
            .filter(|&i| !self.mobs[i].removed)
    }
}
impl HeartHost for Host<'_> {
    fn mob(&self, id: u32) -> Option<&Mob> {
        match self.slot(id) {
            Some(slot) => Some(&self.mobs[slot]),
            None => self.added.iter().find(|m| m.id == id && !m.removed),
        }
    }
    fn mob_mut(&mut self, id: u32) -> Option<&mut Mob> {
        match self.slot(id) {
            Some(slot) => Some(&mut self.mobs[slot]),
            None => self.added.iter_mut().find(|m| m.id == id && !m.removed),
        }
    }
    fn mob_for_uuid(&self, uuid: [i32; 4]) -> Option<u32> {
        self.uuids
            .iter()
            .find(|(id, u)| **u == uuid && self.mob(**id).is_some())
            .map(|(id, _)| *id)
    }
    fn uuid_for(&mut self, id: u32) -> [i32; 4] {
        self.uuids
            .get_or_insert_with(id, sunrise_sim::save::random_uuid)
    }
    fn creaking_type(&self) -> Option<Arc<MobType>> {
        self.creaking.clone()
    }
    fn create_mob(&mut self, kind: Arc<MobType>, at: Vec3) -> Mob {
        *self.next_entity += 1;
        // A new entity's random is its own (Mob::new seeds it).
        Mob::new(*self.next_entity, kind, at, 0.0)
    }
    fn add_mob(&mut self, m: Mob) {
        self.added.push(m);
    }
    fn spawning_monsters(&self) -> bool {
        self.spawning
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn heart_state_name(&self, state: BlockStateId) -> String {
        self.common
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("creaking_heart_state").cloned())
            .unwrap_or_default()
    }
    /// Runs `f` on the heart at `key` with the game's mobs and level random,
    /// then applies what the heart reported. `others`: whether `f` reads the
    /// mob world, which is then built with the other mobs (`f` gets None
    /// otherwise: the world reads every player, the teams and the weather, and
    /// most heart ticks need none of it).
    pub fn with_heart(
        &mut self,
        key: Pos,
        heart: &mut CreakingHeart,
        others: bool,
        f: impl FnOnce(
            &mut CreakingHeart,
            Option<&sunrise_sim::mobs::MobWorld>,
            &mut LegacyRandom,
            &mut Host,
            &mut Vec<HeartEvent>,
        ),
    ) -> Result<()> {
        let mut mobs = self.common.mobs.lift();
        #[cfg(any(test, feature = "test-support"))]
        crate::game::server_side::entity_server::taken_pass_tests::after_take(self);
        let mut uuids = std::mem::take(&mut self.common.persist.uuids);
        let mut next_entity = self.server.next_entity;
        let mut random = self.common.world.random.clone();
        let spawning = self.spawning_monsters();
        let creaking = self
            .common
            .res
            .mob_types()
            .get("minecraft:creaking")
            .cloned();
        let player_uuid = self.common.level.player_uuid;
        let snapshot: Vec<_> = if others {
            mobs.iter()
                .map(|m| seen(m, player_uuid, world_items!(self)))
                .collect()
        } else {
            Vec::new()
        };
        let mut events = Vec::new();
        let index = std::cell::RefCell::new(std::mem::take(&mut self.common.mob_index));
        let (added, index) = {
            let w = others.then(|| self.mob_world_base(&snapshot));
            let mut host = Host {
                mobs: &mut mobs,
                added: Vec::new(),
                uuids: &mut uuids,
                creaking,
                next_entity: &mut next_entity,
                spawning,
                index,
            };
            f(heart, w.as_ref(), &mut random, &mut host, &mut events);
            (host.added, host.index.into_inner())
        };
        // The heart's additions go after every slot the index holds.
        self.common.mob_index = index;
        mobs.extend(added);
        self.common
            .mobs
            .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
        self.common.persist.uuids = uuids;
        self.server.next_entity = next_entity;
        self.common.world.random = random;
        let mut mob_events = Vec::new();
        for e in events {
            match e {
                HeartEvent::SetBlock(at, state) => self.level_set(at, state)?,
                HeartEvent::Sound(name, at, volume, pitch) => {
                    self.server_play_sound(&format!("minecraft:{name}"), at, volume, pitch)
                }
                HeartEvent::Trail {
                    at,
                    target,
                    color,
                    duration,
                } => self.server_particle_at(
                    sunrise_sim::lan::server_effects::ServerParticle::trail(
                        target, color, duration,
                    ),
                    at,
                    [0.0; 3],
                ),
                HeartEvent::OutputChanged(at) => self.container_changed((at[0], at[1], at[2])),
                HeartEvent::BlockGameEvent(event, at, state) => {
                    self.post_block_game_event(event, at, None, Some(state))
                }
                HeartEvent::Mob(event) => mob_events.push(event),
            }
        }
        self.handle_mob_events(mob_events)?;
        let _ = key;
        Ok(())
    }
    /// After `Creaking.hurtServer` posts ENTITY_ACTION, check the live protector, let
    /// a player-credited hit reach the heart, then play the mob's hurt sound.
    pub fn creaking_hurt(&mut self, home: [i32; 3], id: u32, by_player: bool) {
        let Some(join) = self.common.mob_object(id).map(|m| m.join) else {
            return;
        };
        let [x, y, z] = home;
        let key = (x, y, z);
        // Hearts can keep a row for their removal tick. The replaced block
        // no longer supplies a protector to Creaking.hurtServer.
        let live_heart = |game: &Self| {
            &*game.res.facts().get(game.world.block(x, y, z)).name == "minecraft:creaking_heart"
        };
        if !live_heart(self) {
            return;
        }
        let Some((lease, entity)) = self.common.world.block_entities.take(&key) else {
            return;
        };
        let BlockEntity::CreakingHeart(mut heart) = entity else {
            // ENTITY_ACTION may have replaced the heart with a different block entity.
            self.common.world.block_entities.put(lease, entity);
            return;
        };
        let mut protects = false;
        let _ = self.with_heart(key, &mut heart, by_player, |h, w, lr, host, out| {
            protects = creaking_heart::protector(h, host) == Some(id);
            if protects && let Some(w) = w {
                creaking_heart::creaking_hurt(h, home, w, lr, host, out);
            }
        });
        let restored = self
            .common
            .world
            .block_entities
            .put(lease, BlockEntity::CreakingHeart(heart));
        if protects
            && restored
            && live_heart(self)
            && let Some(mob) = self
                .common
                .mobs
                .iter_mut()
                .find(|m| m.id == id && m.join == join && !m.removed)
        {
            let mut events = Vec::new();
            sunrise_sim::mobs::creaking::sway(mob, &mut events);
            let _ = self.handle_mob_events(events);
        }
    }
    /// `CreakingHeartBlock.playerWillDestroy`: the heart's creaking twitches to
    /// death and a natural heart drops experience. `onExplosionHit` does the
    /// same for the creaking, and awards the experience only when its
    /// blast's indirect source is a player (`creaking_heart_broken_by`).
    pub(in crate::game::server_side) fn creaking_heart_broken(
        &mut self,
        pos: [i32; 3],
        by_player: bool,
    ) {
        let award = by_player && !self.common.creative && !self.common.spectator();
        self.creaking_heart_broken_by(pos, award);
    }
    /// `creaking_heart_broken` once the breaker is known: `award` is
    /// `tryAwardExperience`'s player check (a player who neither builds
    /// instantly, `preventsBlockDrops`, nor spectates).
    pub(in crate::game::server_side) fn creaking_heart_broken_by(
        &mut self,
        pos: [i32; 3],
        award: bool,
    ) {
        let key = (pos[0], pos[1], pos[2]);
        if !matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::CreakingHeart(_))
        ) {
            return;
        }
        let Some((lease, BlockEntity::CreakingHeart(mut heart))) =
            self.common.world.block_entities.take(&key)
        else {
            return;
        };
        let _ = self.with_heart(key, &mut heart, false, |h, _, _, host, out| {
            creaking_heart::remove_protector(h, host, true, out);
        });
        let state = self.common.world.block(key.0, key.1, key.2);
        let natural = self
            .common
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("natural").cloned())
            .is_some_and(|v| v == "true");
        if award && natural {
            let xp = 20 + self.common.world.random.next_int_bound(5).unwrap_or(0) as u32;
            // Block.popExperience: the amount is drawn, then nothing pops
            // with `block_drops` off.
            if self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::BLOCK_DROPS)
            {
                self.spawn_orbs(
                    [
                        f64::from(pos[0]) + 0.5,
                        f64::from(pos[1]) + 0.5,
                        f64::from(pos[2]) + 0.5,
                    ],
                    xp,
                );
            }
        }
        self.common
            .world
            .block_entities
            .put(lease, BlockEntity::CreakingHeart(heart));
    }
    /// `CreakingHeartBlockEntity.serverTick` of the heart at `key`. Its slot
    /// stays while uprooted, so that a heart whose block went away is still
    /// found here and lets go of its protector.
    pub fn tick_creaking_heart(&mut self, key: Pos) -> Result<()> {
        let Some((lease, BlockEntity::CreakingHeart(mut heart))) =
            self.common.world.block_entities.take(&key)
        else {
            return Ok(());
        };
        let state = self.common.world.block(key.0, key.1, key.2);
        if sunrise_game_data::block_entity_types::factory_block(
            &self.common.res.facts().get(state).name,
        ) != "minecraft:creaking_heart"
        {
            // The block is gone: preRemoveSideEffects.
            self.common.world.block_entities.forget(&key);
            self.with_heart(key, &mut heart, false, |h, _, _, host, out| {
                creaking_heart::remove_protector(h, host, false, out);
            })?;
            return Ok(());
        }
        // getTicker: none once uprooted.
        if self.heart_state_name(state) != "uprooted" {
            let pos = [key.0, key.1, key.2];
            let others = creaking_heart::tick_updates(&heart);
            self.with_heart(key, &mut heart, others, |h, w, lr, host, out| {
                if creaking_heart::server_tick_start(h, pos, lr, host, out) {
                    debug_assert!(w.is_some(), "tick_updates said no update");
                    if let Some(w) = w {
                        creaking_heart::server_tick_update(h, pos, w, lr, host, out);
                    }
                }
            })?;
        }
        self.common
            .world
            .block_entities
            .put(lease, BlockEntity::CreakingHeart(heart));
        Ok(())
    }
    /// `ServerLevel.isSpawningMonsters`.
    pub(in crate::game::server_side) fn spawning_monsters(&self) -> bool {
        self.common.ui.difficulty != 0
            && self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_MOBS)
            && self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_MONSTERS)
    }
    /// Hearts that came with generated or loaded chunks get their block
    /// entity (`newBlockEntity`) the first time their chunk is seen lit.
    /// Only the chunks that arrived since are looked at: a server holds tens
    /// of thousands, and reading each one every tick cost milliseconds.
    pub(in crate::game) fn scan_for_hearts(&mut self) {
        if self
            .common
            .pack
            .assets
            .registry
            .default_state("minecraft:creaking_heart")
            .is_none()
        {
            return;
        }
        self.server
            .creaking
            .follow_chunks(&self.common.world.chunks);
        let mut keys: Vec<(i32, i32)> = self
            .server
            .creaking
            .unscanned
            .iter()
            .copied()
            .filter(|k| self.common.world.chunks.get(k).is_some_and(|c| c.lit))
            .collect();
        keys.sort_unstable();
        for key in keys {
            self.server.creaking.unscanned.remove(&key);
            self.server.creaking.scanned.insert(key);
            let Some(chunk) = self.common.world.chunks.get(&key).cloned() else {
                continue;
            };
            if self.server.creaking.hearts.is_empty() {
                self.server.creaking.hearts = self
                    .common
                    .res
                    .facts()
                    .states
                    .iter()
                    .map(|f| &*f.name == "minecraft:creaking_heart")
                    .collect();
            }
            let hearts = &self.server.creaking.hearts;
            let is_heart = |id: u32| hearts.get(id as usize).copied().unwrap_or(false);
            for (si, section) in chunk.sections.iter().enumerate() {
                // Only a section whose palette holds a heart state can have
                // one: the others are skipped without reading their 4,096 blocks.
                if !section.blocks.maybe_has(is_heart) {
                    continue;
                }
                for i in 0..4096 {
                    let id = section.get(i);
                    if !is_heart(id) {
                        continue;
                    }
                    let (lx, lz, ly) = ((i & 15) as i32, ((i >> 4) & 15) as i32, (i >> 8) as i32);
                    let pos = (
                        key.0 * 16 + lx,
                        self.common.world.bottom_y() + si as i32 * 16 + ly,
                        key.1 * 16 + lz,
                    );
                    self.common
                        .world
                        .block_entities
                        .get_or_insert_with(pos, || BlockEntity::CreakingHeart(Box::default()));
                }
            }
        }
    }
}

/// What the creaking code remembers between ticks.
#[derive(Default)]
pub(in crate::game) struct CreakingState {
    /// Chunks already scanned for heart blocks.
    pub(in crate::game) scanned: sunrise_level::fast_hash::FxHashSet<(i32, i32)>,
    /// Resident chunks not scanned yet (not lit yet, or just arrived).
    pub(in crate::game) unscanned: sunrise_level::fast_hash::FxHashSet<(i32, i32)>,
    /// The chunk map's membership revision the two sets follow.
    pub(in crate::game) revision: Option<u64>,
    /// Which block states are creaking hearts, by state id.
    pub(in crate::game) hearts: Vec<bool>,
}

impl CreakingState {
    /// Exchange the chunks scanned for creaking hearts with another level's
    /// (game/world/levels.rs).
    pub(in crate::game::server_side) fn swap_level(&mut self, other: &mut Self) {
        std::mem::swap(&mut self.scanned, &mut other.scanned);
        std::mem::swap(&mut self.unscanned, &mut other.unscanned);
        std::mem::swap(&mut self.revision, &mut other.revision);
    }
    /// Bring `scanned` and `unscanned` up to the chunks resident now: a
    /// chunk that left is forgotten (it is scanned again if it comes back
    /// later), one that arrived waits to be scanned. From the map's
    /// membership changes, or from all its keys when they can't be replayed.
    pub(in crate::game::server_side) fn follow_chunks(&mut self, chunks: &ChunkMap) {
        let revision = chunks.membership_revision();
        if self.revision == Some(revision) {
            return;
        }
        let deltas = self.revision.and_then(|r| chunks.deltas_since(r));
        match deltas {
            Some(deltas) => {
                for (_, delta) in deltas {
                    match delta {
                        ChunkMembershipDelta::Added(key) => {
                            if chunks.contains_key(&key) && !self.scanned.contains(&key) {
                                self.unscanned.insert(key);
                            }
                        }
                        ChunkMembershipDelta::Removed(key) => {
                            if !chunks.contains_key(&key) {
                                self.scanned.remove(&key);
                                self.unscanned.remove(&key);
                            }
                        }
                        ChunkMembershipDelta::Reset => {}
                    }
                }
            }
            None => {
                self.scanned.retain(|k| chunks.contains_key(k));
                self.unscanned = chunks
                    .keys()
                    .filter(|k| !self.scanned.contains(k))
                    .copied()
                    .collect();
            }
        }
        self.revision = Some(revision);
    }
}

impl crate::game::ServerGame<'_> {
    /// `Creaking.tearDown`: the crumble particles.
    pub(in crate::game) fn creaking_tear_down(&mut self, center: [f64; 3], spread: [f64; 3]) {
        let registry = self.common.pack.assets.registry;
        let crumble = |this: &mut Self, name: &str, props: &[(&str, &str)], count: i32| {
            let Some(mut state) = registry
                .default_state(name)
                .and_then(|s| registry.state(s).cloned())
            else {
                return;
            };
            for (k, v) in props {
                state.properties.insert((*k).to_owned(), (*v).to_owned());
            }
            let Ok(id) = registry.resolve(&state) else {
                return;
            };
            for _ in 0..count {
                let r = &mut this.world.random;
                let p = [
                    center[0] + r.next_gaussian() * spread[0],
                    center[1] + r.next_gaussian() * spread[1],
                    center[2] + r.next_gaussian() * spread[2],
                ];
                this.server_particle_at(
                    sunrise_sim::lan::server_effects::ServerParticle::block(
                        ParticleType::BlockCrumble,
                        id,
                    ),
                    p,
                    [0.0; 3],
                );
            }
        };
        crumble(self, "minecraft:pale_oak_wood", &[], 100);
        crumble(
            self,
            "minecraft:creaking_heart",
            &[("creaking_heart_state", "awake")],
            10,
        );
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::CreakingState;
    use std::sync::Arc;
    use sunrise_level::{chunk::Chunk, chunk_map::ChunkMap};

    fn add(chunks: &mut ChunkMap, key: (i32, i32)) {
        chunks.insert(key, Arc::new(Chunk::empty(key.0, key.1)));
    }
    fn sorted(set: &sunrise_level::fast_hash::FxHashSet<(i32, i32)>) -> Vec<(i32, i32)> {
        let mut keys: Vec<_> = set.iter().copied().collect();
        keys.sort_unstable();
        keys
    }

    /// The heart scan follows the chunks that come and go, as the scan of
    /// every resident chunk each tick did: an arrival waits, a scanned chunk
    /// that stayed is not scanned again, and one that left is forgotten.
    #[test]
    fn the_heart_scan_follows_arrivals_and_departures() {
        let mut chunks = ChunkMap::default();
        let mut state = CreakingState::default();
        add(&mut chunks, (0, 0));
        add(&mut chunks, (1, 0));
        state.follow_chunks(&chunks);
        assert_eq!(sorted(&state.unscanned), [(0, 0), (1, 0)]);
        // Scanned, as scan_for_hearts does once a chunk is lit.
        state.unscanned.remove(&(0, 0));
        state.scanned.insert((0, 0));
        add(&mut chunks, (2, 0));
        chunks.remove(&(1, 0));
        // Gone and back between two ticks: still scanned, as before.
        chunks.remove(&(0, 0));
        add(&mut chunks, (0, 0));
        state.follow_chunks(&chunks);
        assert_eq!(sorted(&state.scanned), [(0, 0)]);
        assert_eq!(sorted(&state.unscanned), [(2, 0)]);
        // Gone over a tick: forgotten, and scanned again when it returns.
        chunks.remove(&(0, 0));
        state.follow_chunks(&chunks);
        assert!(state.scanned.is_empty());
        add(&mut chunks, (0, 0));
        state.follow_chunks(&chunks);
        assert_eq!(sorted(&state.unscanned), [(0, 0), (2, 0)]);
        // A cleared map can't be replayed: rebuilt from its keys.
        chunks.clear();
        add(&mut chunks, (5, 5));
        state.follow_chunks(&chunks);
        assert_eq!(sorted(&state.unscanned), [(5, 5)]);
        assert!(state.scanned.is_empty());
    }
}
