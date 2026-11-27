//! Trial spawners in the game: `TrialSpawner.tickServer` for every trial
//! spawner (`mobs::trial_spawner`), the block's state as it changes, the mobs
//! it adds and the rewards it ejects.
use super::{natural_spawning::GameLevel, spawner::Planned};
use std::collections::HashMap;
use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, LevelRandom, RandomSource},
};
use sunrise_game_data::Result;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::mobs::{
    Mob, Who,
    spawner::SpawnerSpawn,
    trial_spawner::{
        TRIAL_OMEN_TICKS_PER_LEVEL, TrialEvent, TrialPlayer, TrialSpawner, TrialSpawnerExt,
        TrialSpawnerLevel, TrialState, Uuid,
    },
};

type Pos = (i32, i32, i32);

impl TrialSpawnerLevel for GameLevel<'_, '_> {
    fn game_time(&self) -> i64 {
        self.game.world.game_time
    }
    fn mob_spawning(&self) -> bool {
        self.game
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_MOBS)
    }
    fn players(&self) -> Vec<TrialPlayer> {
        let guests = self.game.lan_present().into_iter().map(|g| TrialPlayer {
            uuid: g.uuid,
            block: g.position.map(|v| v.floor() as i32),
            eye: g.eye,
            creative: g.creative,
            spectator: g.spectator,
            position: g.position,
            height: g.height,
            alive: g.alive,
            omen: g.omen,
        });
        let (Some(p), Some(view)) = (self.game.player.as_ref(), self.game.player_view()) else {
            return guests.collect();
        };
        let host = TrialPlayer {
            uuid: self.game.acting_uuid(),
            block: [p.position.x, p.position.y, p.position.z].map(|v| v.floor() as i32),
            eye: view.eye,
            creative: self.game.creative,
            spectator: self.game.spectator(),
            position: [p.position.x, p.position.y, p.position.z],
            height: p.dimensions.height,
            alive: self.game.level.health > 0.0,
            omen: super::lan_mobs::omen_of(&self.game.potions.effects),
        };
        std::iter::once(host).chain(guests).collect()
    }
    fn visual_hit(&self, from: [f64; 3], to: [f64; 3]) -> Option<[i32; 3]> {
        let v = |p: [f64; 3]| Vec3::new(p[0], p[1], p[2]);
        self.w.visual_hit(v(from), v(to))
    }
    fn entity_block(&self, uuid: Uuid) -> Option<[i32; 3]> {
        let (id, _) = self.game.persist.uuids.iter().find(|(_, u)| **u == uuid)?;
        let m = self.game.mobs.iter().find(|m| m.id == *id)?;
        (!m.removed && m.alive())
            .then(|| [m.position.x, m.position.y, m.position.z].map(|v| v.floor() as i32))
    }
    fn entity_box(&self, uuid: Uuid) -> Option<([f64; 3], f32)> {
        let (id, _) = self.game.persist.uuids.iter().find(|(_, u)| **u == uuid)?;
        let m = self.game.mobs.iter().find(|m| m.id == *id)?;
        let b = m.bounds();
        (!m.removed && m.alive()).then(|| {
            (
                [m.position.x, m.position.y, m.position.z],
                (b.max_position().y - b.min_position().y) as f32,
            )
        })
    }
    fn no_block_collision(&self, block: [i32; 3]) -> bool {
        self.w
            .block_collision_boxes(block[0], block[1], block[2])
            .is_empty()
    }
    fn loot_items(&self, table: &str, seed: i64) -> Vec<ItemStack> {
        let Some(loot) = self.game.server.server_loot.0.get() else {
            return Vec::new();
        };
        loot.roll_split(table, &mut LegacyRandom::new(seed))
            .iter()
            .map(sunrise_sim::structure_blocks::loot_item_with_components)
            .collect()
    }
    fn seed(&self) -> i64 {
        self.game.level.seed
    }
}

impl crate::game::ServerGame<'_> {
    /// The server tick of the trial spawner at `key`. Its client tick comes
    /// after every block entity's.
    pub(in crate::game) fn tick_trial_spawner(&mut self, key: Pos) -> Result<()> {
        let Some((current, ominous)) = self.trial_spawner_block(key) else {
            return Ok(());
        };
        let Some((lease, BlockEntity::TrialSpawner(mut spawner))) =
            self.common.world.block_entities.take(&key)
        else {
            return Ok(());
        };
        let result = self.server_tick_trial_spawner(key, &mut spawner, current, ominous);
        self.common
            .world
            .block_entities
            .put(lease, BlockEntity::TrialSpawner(spawner));
        result
    }
    /// What the LAN encoder builds update tags from beyond the block entity:
    /// the mods' hook (`api_block_entity_update_tag`), and the block
    /// states with `trial_spawner_state=active`, the only ones whose tag
    /// holds `next_mob_spawns_at` (`TrialSpawnerStateData.getUpdateTag`).
    pub(in crate::game::server_side) fn lan_update_tag_sources(
        &self,
        encoder: &mut sunrise_sim::lan::chunks::ChunkEncoder,
    ) {
        encoder.mod_update_tag = Some(|pos, entity| {
            let mut tag = None;
            crate::game::ServerGame::api_block_entity_update_tag(pos, entity, &mut tag);
            tag
        });
        let registry = self.common.pack.assets.registry;
        encoder.trial_active = (0..registry.len() as u32)
            .filter(|&i| {
                registry
                    .state(sunrise_world::registry::BlockStateId(i))
                    .is_some_and(|b| {
                        b.name == "minecraft:trial_spawner"
                            && b.properties
                                .get("trial_spawner_state")
                                .is_some_and(|s| s == TrialState::Active.name())
                    })
            })
            .collect();
    }
    /// The trial spawner block at `key`: its state and ominous flag.
    pub(in crate::game) fn trial_spawner_block(&self, key: Pos) -> Option<(TrialState, bool)> {
        let block = self
            .common
            .pack
            .assets
            .registry
            .state(self.common.world.block(key.0, key.1, key.2))?;
        if sunrise_game_data::block_entity_types::factory_block(&block.name)
            != "minecraft:trial_spawner"
        {
            return None;
        }
        let state = block
            .properties
            .get("trial_spawner_state")
            .and_then(|s| TrialState::from_name(s))
            .unwrap_or_default();
        Some((
            state,
            block.properties.get("ominous").is_some_and(|v| v == "true"),
        ))
    }
    pub fn server_tick_trial_spawner(
        &mut self,
        key: Pos,
        spawner: &mut TrialSpawner,
        current: TrialState,
        ominous: bool,
    ) -> Result<()> {
        let pos = [key.0, key.1, key.2];
        let mut random = std::mem::replace(
            &mut self.common.world.random,
            LevelRandom::for_level(LegacyRandom::new(0)),
        );
        let sequences = std::cell::RefCell::new(std::mem::take(&mut self.server.rand_sequences));
        let mut events = Vec::new();
        let mut planned: Vec<Planned> = Vec::new();
        let source = Some(mod_api::Spawner::Trial {
            pos: mod_api::BlockPos::new(key.0, key.1, key.2),
        });
        let mut uuids: Vec<(u32, Uuid)> = Vec::new();
        let mut ejected: Vec<(ItemStack, [f64; 3])> = Vec::new();
        let extras: (Vec<Mob>, Vec<(u32, u32)>);
        let next = {
            let level = self.natural_level();
            let loot = self.server.server_loot.0.get();
            let seed = self.common.level.seed;
            let mut spawn = |l: &GameLevel, rng: &mut LevelRandom, r: &SpawnerSpawn| {
                let mut sequences = sequences.borrow_mut();
                let added =
                    l.game
                        .plan_spawner_mob(l, r, source, &mut planned, rng, &mut sequences)?;
                let uuid = match planned.last() {
                    Some(Planned::Loaded(tree, _)) => tree.root_uuid(),
                    _ => sunrise_sim::save::random_uuid(),
                };
                uuids.push((added.id, uuid));
                Some(uuid)
            };
            // TrialSpawner.ejectReward: the table's items, each thrown up out
            // of the spawner (DefaultDispenseItemBehavior.spawnItem, accuracy 2).
            let mut eject = |table: &str, random: &mut LevelRandom| {
                let Some(loot) = &loot else {
                    return false;
                };
                let drops = match loot.random_sequence(table) {
                    Some(s) => {
                        let mut sequences = sequences.borrow_mut();
                        loot.roll_split(table, sequences.get(s, seed))
                    }
                    None => loot.roll_split(table, random),
                };
                for stack in &drops {
                    // The power, which only scales the direction's x and z (0 going up).
                    random.next_double();
                    let spread = 0.017_227_5 * 2.0;
                    let v = [
                        random.triangle(0.0, spread),
                        random.triangle(0.2, spread),
                        random.triangle(0.0, spread),
                    ];
                    ejected.push((
                        sunrise_sim::structure_blocks::loot_item_with_components(stack),
                        v,
                    ));
                }
                !drops.is_empty()
            };
            let state = spawner.server_tick(
                &level,
                &mut random,
                pos,
                current,
                ominous,
                &mut spawn,
                &mut eject,
                &mut events,
            );
            extras = (
                std::mem::take(&mut *level.spawned.borrow_mut()),
                std::mem::take(&mut *level.rides.borrow_mut()),
            );
            state
        };
        self.common.world.random = random;
        self.server.rand_sequences = sequences.into_inner();
        let (created, rides) = extras;
        let ids = self.add_planned(planned, created, rides);
        let mut by_uuid = HashMap::new();
        for (provisional, uuid) in uuids {
            if let Some(id) = ids.get(&provisional) {
                self.common.persist.uuids.insert(*id, uuid);
                by_uuid.insert(uuid, *id);
            }
        }
        let center = pos.map(|v| f64::from(v) + 0.5);
        // Vec3.atBottomCenterOf(pos).relative(UP, 1.2), lowered by spawnItem.
        let above = [center[0], center[1] + 0.7 - 0.125, center[2]];
        for (stack, v) in ejected {
            self.spawn_item_thrown(stack, above, v);
        }
        if next != current || ominous != spawner.is_ominous {
            self.set_trial_spawner_state(key, next, spawner.is_ominous)?;
        }
        for event in events {
            match event {
                TrialEvent::BlockUpdated => self.mark_block_entity_updated(key),
                TrialEvent::LevelEvent(id, at, data) => self.server_level_event_at(id, at, data),
                TrialEvent::Sound(name) => {
                    self.server_play_sound(&format!("minecraft:{name}"), center, 1.0, 1.0)
                }
                // Written with the state above.
                TrialEvent::OminousSet(_) => {}
                TrialEvent::EntityPlace(uuid, at) => {
                    let who = by_uuid.get(&uuid).map(|id| Who::Mob(*id));
                    self.post_block_game_event("minecraft:entity_place", at, who, None);
                }
                TrialEvent::TrialOmen(uuid) => self.bad_omen_to_trial_omen(uuid),
                TrialEvent::Discard(uuid) => {
                    if let Some((&id, _)) =
                        self.common.persist.uuids.iter().find(|(_, u)| **u == uuid)
                        && let Some(m) = self
                            .common
                            .mobs
                            .iter_mut()
                            .find(|m| m.id == id && !m.removed)
                    {
                        let at = m.position;
                        let drops = sunrise_sim::mobs::drops::drop_preserved_equipment(m);
                        for stack in drops {
                            self.spawn_at_location(at, 0.0, stack);
                        }
                        self.remove_mob_now(id, false);
                    }
                }
                TrialEvent::SoundAt(name, at, pitch) => self.server_play_sound(
                    &format!("minecraft:{name}"),
                    at.map(|v| f64::from(v) + 0.5),
                    1.0,
                    pitch,
                ),
                TrialEvent::ItemSpawner {
                    item,
                    position,
                    delay,
                } => self.add_ominous_item_spawner(item, position, delay),
            }
        }
        Ok(())
    }
    /// `setState` (and `removeOminous`): the block's state and ominous flag.
    pub(in crate::game::server_side) fn set_trial_spawner_state(
        &mut self,
        key: Pos,
        state: TrialState,
        ominous: bool,
    ) -> Result<()> {
        let current = self.common.world.block(key.0, key.1, key.2);
        let Some(mut block) = self.common.pack.assets.registry.state(current).cloned() else {
            return Ok(());
        };
        block
            .properties
            .insert("trial_spawner_state".into(), state.name().into());
        block
            .properties
            .insert("ominous".into(), ominous.to_string());
        let id = self.common.pack.assets.registry.resolve(&block)?;
        self.level_set([key.0, key.1, key.2], id)
    }
    /// `transformBadOmenIntoTrialOmen`: Bad Omen becomes Trial Omen, 15
    /// minutes per level, with Trial Omen's sound when it is new. `uuid` is
    /// the player found: the one in the game's frame, else a guest.
    fn bad_omen_to_trial_omen(&mut self, uuid: Uuid) {
        if self.acting_uuid() == uuid {
            self.transform_player_bad_omen();
            return;
        }
        let acting = self.server.lan_acting;
        let conn = self.server.lan.as_ref().and_then(|host| {
            host.hub.guests.iter().find_map(|g| {
                (sunrise_sim::lan::playerdata::uuid_ints(g.uuid) == uuid && Some(g.conn) != acting)
                    .then_some(g.conn)
            })
        });
        if let Some(conn) = conn {
            self.as_guest(conn, |g| g.transform_player_bad_omen());
        }
    }
    /// `TrialSpawnerStateData.transformBadOmenIntoTrialOmen` for the acting
    /// player: `removeEffect(BAD_OMEN)` then `addEffect(TRIAL_OMEN)` through
    /// the standard effect hooks and added-sound path.
    fn transform_player_bad_omen(&mut self) {
        let Some(amplifier) = self.common.potions.effects.amplifier("bad_omen") else {
            return;
        };
        if let Some(def) = sunrise_game_data::potions::effect("bad_omen") {
            self.remove_player_effects(Some(def.index));
        }
        self.add_player_effect(sunrise_level::effects::EffectInstance::new(
            "trial_omen",
            TRIAL_OMEN_TICKS_PER_LEVEL * (amplifier + 1),
            0,
        ));
    }
    /// `sendBlockUpdated` of a block entity: the clients load its new update tag.
    pub fn mark_block_entity_updated(&mut self, key: Pos) {
        self.common
            .world
            .path_type_block_updated(sunrise_core::position::BlockPos::new(key.0, key.1, key.2));
        if let Some(log) = self.common.world.block_log.as_mut() {
            log.push(key);
        }
        self.common.world.modified.insert((key.0 >> 4, key.2 >> 4));
    }
}

/// Swap Bad Omen for Trial Omen on `effects`; true when Trial Omen is new.
#[cfg(test)]
fn transform_bad_omen(effects: &mut sunrise_level::effects::PlayerEffects) -> bool {
    let Some(amplifier) = effects.amplifier("bad_omen") else {
        return false;
    };
    effects.list.retain(|e| e.id != "bad_omen");
    let new = effects.get("trial_omen").is_none();
    effects.add(sunrise_level::effects::EffectInstance::new(
        "trial_omen",
        TRIAL_OMEN_TICKS_PER_LEVEL * (amplifier + 1),
        0,
    ));
    new
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

#[cfg(test)]
mod omen_tests {
    use super::*;
    use sunrise_level::effects::{EffectInstance, PlayerEffects};
    use sunrise_sim::mobs::trial_spawner::Omen;

    #[test]
    fn bad_omen_becomes_trial_omen_per_level() {
        let omen_of = super::super::lan_mobs::omen_of;
        let mut e = PlayerEffects::default();
        assert_eq!(omen_of(&e), None);
        e.add(EffectInstance::new("bad_omen", 100, 1));
        assert_eq!(omen_of(&e), Some(Omen::Bad));
        assert!(transform_bad_omen(&mut e));
        assert!(e.get("bad_omen").is_none());
        assert_eq!(
            e.get("trial_omen").unwrap().duration,
            TRIAL_OMEN_TICKS_PER_LEVEL * 2
        );
        assert_eq!(omen_of(&e), Some(Omen::Trial));
        assert!(!transform_bad_omen(&mut e));
    }

    #[cfg(any())]
    #[test]
    #[ignore = "daily"]
    fn ominous_item_target_uses_the_reloaded_player_table() {
        use std::sync::Arc;
        use sunrise_core::nbt::parse_snbt;
        use sunrise_level::chunk::Chunk;
        use {
            crate::game::native_entity_refs::tests::with_game,
            crate::game::native_living_flags::tests::guest_server,
            sunrise_gameplay_common::game::server_side::native_sound_refs::tests::guest,
        };

        with_game(|game| {
            guest_server(game, false, false);
            game.world
                .chunks
                .insert((-1, 0), Arc::new(Chunk::empty(-1, 0)));
            game.load_server_loot();
            game.level.seed = 0;
            game.world.game_time = 1;
            let (mut left, _left_rx) = guest(16, 16, [-1.5, 0.0, 0.5]);
            let (mut right, _right_rx) = guest(1, 1, [2.5, 0.0, 0.5]);
            for player in [&mut left, &mut right] {
                player.loaded = true;
                player.dimension = game.dim.id().to_owned();
            }
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .extend([left, right]);

            let ids = (32..45)
                .map(|id| format!("[I;0,0,0,{id}]"))
                .collect::<Vec<_>>();
            let warmup = parse_snbt(&format!("{{registered_players:[{}]}}", ids.join(",")))
                .unwrap()
                .0;
            let previous = TrialSpawner::load(&warmup, None);
            let data = parse_snbt(
                r#"{registered_players:[[I;0,0,0,1],[I;0,0,0,16]],spawn_data:{entity:{id:"minecraft:zombie"}},next_mob_spawns_at:100L}"#,
            ).unwrap().0;
            // Vanilla's fresh and grown-table positions for seed 13 (trial_spawner parity).
            for (previous, expected) in
                [(None, [-1.5, 2.5, 0.5]), (Some(&previous), [2.5, 2.5, 0.5])]
            {
                let mut spawner = TrialSpawner::load(&data, previous);
                let count = game.ominous_spawners.len();
                game.world.random = LevelRandom::for_level(LegacyRandom::new(13));
                game.server_tick_trial_spawner((0, 0, 0), &mut spawner, TrialState::Active, true)
                    .unwrap();
                assert_eq!(game.ominous_spawners.len(), count + 1);
                assert_eq!(game.ominous_spawners.last().unwrap().position, expected);
            }
        });
    }
}
