//! Monster spawners in the game: `SpawnerBlockEntity.serverTick` and
//! `clientTick` for every spawner (`mobs::spawner`), the mobs they add, a
//! spawn egg used on one (`SpawnEggItem.useOn`), their events and the mob
//! spinning inside (`SpawnerRenderer`).
use super::persist::SpawnTree;
use std::collections::HashMap;
use sunrise_core::nbt::Tag;
#[allow(unused_imports)]
use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, LevelRandom, RandomSource},
};
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::creatures::SpawnReason;
use sunrise_gameplay_common::game::native_hooks::spawner_active::GameHooks as _;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::mobs::{
    Mob, Who,
    spawner::{Spawner, SpawnerAdded, SpawnerEvent, SpawnerExt, SpawnerSpawn},
    trial_spawner::{TrialSpawnerExt, TrialState},
};

pub(in crate::game) type Pos = (i32, i32, i32);

/// What a spawner tick decided to add, created once the level is free.
pub enum Planned {
    /// A fresh mob with a provisional ID and any spawner equipment already rolled.
    Finalized(Mob),
    /// A configured tree with its root's provisional event ID.
    Loaded(SpawnTree, u32),
}

impl crate::game::ServerGame<'_> {
    /// `EntityType.byString`: the type is in the registry.
    pub(in crate::game) fn entity_type_exists(&self, id: &str) -> bool {
        self.common
            .res
            .rest
            .get()
            .is_some_and(|rest| rest.spawner_types.contains_key(id))
    }
    /// Whether `entity_type_exists` has a registry to answer from.
    pub(in crate::game::server_side) fn entity_types_loaded(&self) -> bool {
        self.common.res.rest.get().is_some()
    }
    /// The server tick of the spawner at `key`. Its client tick comes after
    /// every block entity's (`client_tick_spawners`).
    ///
    /// The caller only reaches this for a `BlockEntity::Spawner`, and changing
    /// the block removes or replaces that entity, so the block is not read
    /// again here.
    pub fn tick_spawner(&mut self, key: Pos) -> Result<()> {
        let Some((lease, BlockEntity::Spawner(mut spawner))) =
            self.common.world.block_entities.take(&key)
        else {
            return Ok(());
        };
        let result = self.server_tick_spawner(key, &mut spawner);
        self.common
            .world
            .block_entities
            .put(lease, BlockEntity::Spawner(spawner));
        result
    }
    pub(in crate::game::server_side) fn server_tick_spawner(
        &mut self,
        key: Pos,
        spawner: &mut Spawner,
    ) -> Result<()> {
        self.server_tick_spawner_of(key, spawner, None)
    }
    /// `BaseSpawner.serverTick` at `key`: a spawner block's, or the spawner minecart
    /// `cart` is at (its `broadcastEvent` is an entity event on the cart).
    pub fn server_tick_spawner_of(
        &mut self,
        key: Pos,
        spawner: &mut Spawner,
        cart: Option<u32>,
    ) -> Result<()> {
        // `BaseSpawner.serverTick` does nothing without an alive player in
        // range (`isNearPlayer`, or what a mod's `spawner_active` says) or with the rule off: no
        // level to build for it.
        let at = [key.0, key.1, key.2].map(|c| f64::from(c) + 0.5);
        let range = f64::from(spawner.required_player_range);
        let near = self.alive_player_near(at[0], at[1], at[2], range);
        let identity = |game: &Self| match cart {
            Some(id) => mod_api::Spawner::Minecart {
                entity: mod_api::EntityRef::new(game.lend_epoch(), id),
            },
            None => mod_api::Spawner::Block {
                pos: mod_api::BlockPos::new(key.0, key.1, key.2),
            },
        };
        let active = self.spawner_active(|| identity(self), near);
        if !active
            || !self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::SPAWNER_BLOCKS_WORK)
        {
            return Ok(());
        }
        // While its delay counts down, `serverTick` only takes one off it: no
        // level, random or event (a server with hundreds of dungeons near its
        // players built the level for every one of them, every tick).
        if spawner.spawn_delay > 0 {
            spawner.spawn_delay -= 1;
            return Ok(());
        }
        let pos = [key.0, key.1, key.2];
        let mut random = std::mem::replace(
            &mut self.common.world.random,
            LevelRandom::for_level(LegacyRandom::new(0)),
        );
        let mut sequences = std::mem::take(&mut self.server.rand_sequences);
        let mut events = Vec::new();
        let mut planned: Vec<Planned> = Vec::new();
        // `BaseSpawner` belongs either to a monster spawner block (`SpawnerBlockEntity`) or to a
        // spawner minecart (`MinecartSpawner`); both `mob_spawn_position_check` (`PositionCheck`)
        // and `finalize_spawn` (`FinalizeSpawnEvent`) receive that typed spawner identity.
        let source = Some(identity(self));
        let mut plan = |game: &Self| {
            let mut level = game.natural_level();
            level.spawner = source;
            level.spawner_active = Some(active);
            let mut spawn = |l: &super::natural_spawning::GameLevel,
                             rng: &mut LevelRandom,
                             r: &SpawnerSpawn| {
                l.game
                    .plan_spawner_mob(l, r, source, &mut planned, rng, &mut sequences)
            };
            spawner.server_tick(&level, &mut random, pos, &mut spawn, &mut events);
            (
                std::mem::take(&mut *level.spawned.borrow_mut()),
                std::mem::take(&mut *level.rides.borrow_mut()),
            )
        };
        let (created, rides) = if let Some(cart) = cart {
            self.with_held_entity_read(cart, plan)
        } else {
            plan(self)
        };
        self.common.world.random = random;
        self.server.rand_sequences = sequences;
        let ids = self.add_planned(planned, created, rides);
        self.apply_spawner_events(key, events, &ids, cart)
    }
    /// Resolve the provisional ids of the mobs a spawner tick planned.
    pub fn add_planned(
        &mut self,
        planned: Vec<Planned>,
        mut created: Vec<Mob>,
        rides: Vec<(u32, u32)>,
    ) -> HashMap<u32, u32> {
        let _save_lease = sunrise_sim::join_seq::visibility::PassLease::enter();
        let mut finalized: Vec<Mob> = Vec::new();
        let mut loads = Vec::new();
        for plan in planned {
            match plan {
                Planned::Finalized(mob) => finalized.push(mob),
                Planned::Loaded(tree, key) => loads.push((tree, key)),
            }
        }
        // Real ids for finalizeSpawn's provisional mobs, in spawn order, the
        // links that ride by id rewritten after them: the map the spawner's
        // events resolve their provisional ids with.
        let mut ids = self.resolve_spawn_ids([&mut finalized, &mut created], &rides);
        for mob in finalized {
            let i = self.common.mobs.len();
            self.common.mobs.push(mob);
            self.admit_new_mobs(i);
        }
        for mob in created {
            let i = self.common.mobs.len();
            self.common.mobs.push(mob);
            self.admit_new_mobs(i);
        }
        for (tree, key) in loads {
            ids.insert(key, tree.install(self));
        }
        ids
    }
    /// Decode the configured tree and assign its root a provisional ID.
    pub(super) fn load_configured_entity(
        &self,
        l: &super::natural_spawning::GameLevel,
        entity: &sunrise_core::nbt::Compound,
        position: [f64; 3],
        yaw: f32,
    ) -> Option<(SpawnTree, u32)> {
        let mut tag = entity.clone();
        // Only structure placement can request deferred vanilla finalization.
        tag.remove(&"sunrise:finalize".into());
        tag.insert(
            "Rotation".into(),
            Tag::List(vec![Tag::Float(yaw.to_bits()), Tag::Float(0f32.to_bits())]),
        );
        let mut tag = Tag::Compound(tag);
        // The placement callback applies to the whole passenger tree.
        super::commands::place_summoned(&mut tag, position);
        if !self.retain_feature_eligible_entity_tree(&mut tag) {
            return None;
        }
        let mut tree = SpawnTree::decode(self, tag)?;
        let key = l.provisional_id();
        tree.root_mob_mut()?.0.id = key;
        Some((tree, key))
    }
    /// The spawner's `loadEntityRecursive` and `tryAddFreshEntityWithPassengers`:
    /// make the accepted mobs visible to the next attempt before the tick inserts them.
    pub fn plan_spawner_mob(
        &self,
        l: &super::natural_spawning::GameLevel,
        r: &SpawnerSpawn,
        source: Option<mod_api::Spawner>,
        planned: &mut Vec<Planned>,
        random: &mut sunrise_core::random::LegacyRandom,
        sequences: &mut sunrise_items::random_sequences::RandomSequences,
    ) -> Option<SpawnerAdded> {
        let kind = self.common.res.mob_types().get(r.id)?;
        let mut plan = if r.finalize {
            // The mob a listened position check created, else a new one.
            let mob = l.take_checked(r.id, r.position, r.yaw).unwrap_or_else(|| {
                let [x, y, z] = r.position;
                Mob::new(l.provisional_id(), kind.clone(), Vec3::new(x, y, z), r.yaw)
            });
            Planned::Finalized(mob)
        } else {
            // Trial spawners skip the position check, so their tree loads here.
            let (tree, key) = match l.take_checked_tree() {
                Some(tree) => (tree, l.provisional_id()),
                None => self.load_configured_entity(l, r.entity, r.position, r.yaw)?,
            };
            Planned::Loaded(tree, key)
        };
        let (mob, tag) = match &mut plan {
            Planned::Finalized(mob) => (mob, None),
            Planned::Loaded(tree, key) => {
                let (mob, tag) = tree.root_mob_mut()?;
                mob.id = *key;
                (mob, Some(tag))
            }
        };
        let reason = if r.trial {
            SpawnReason::TrialSpawner
        } else {
            SpawnReason::Spawner
        };
        // Lend the configured NBT tags so the hook can remove them as well as add tags.
        let configured_tags = tag.as_deref().and_then(|tag| {
            mod_api::__private::listened(mod_api::HookId::FinalizeSpawn).then(|| {
                match tag.get("Tags") {
                    Some(Tag::List(tags)) => tags
                        .iter()
                        .filter_map(|tag| sunrise_sim::save::text(Some(tag)))
                        .collect::<Vec<_>>(),
                    _ => Vec::new(),
                }
            })
        });
        if let Some(given) = &configured_tags {
            mob.extra.spawn_tags.clone_from(given);
        }
        let mut group = None;
        let (decision, difficulty) = self.finalize_spawn_event(mob, reason, source, &mut group);
        // The event always runs for a spawner, even when vanilla initialization is skipped.
        if r.finalize && !decision.skip_initialization {
            let mut level = super::natural_spawning::PassLevel::new(l);
            let mut cx = sunrise_sim::mobs::finalize::Ctx {
                level: &mut level,
                random,
                difficulty,
                created: Vec::new(),
                jockey_rider: None,
            };
            sunrise_sim::mobs::finalize::finalize(&mut cx, mob, reason, group);
            if decision.veto_admission {
                l.rides.borrow_mut().retain(|&(_, rider)| rider != mob.id);
            } else {
                for m in cx.created {
                    l.record_pending(&m);
                    l.spawned.borrow_mut().push(m);
                }
            }
            // TrialSpawner.spawnMob sets persistence after finalization.
            mob.persistent |= r.trial;
            // Shulker.finalizeSpawn faces it south.
            if mob.species() == sunrise_sim::mobs::Species::Shulker {
                mob.yaw = 0.0;
                mob.previous_yaw = 0.0;
                mob.body_yaw = 0.0;
                mob.previous_body_yaw = 0.0;
                mob.head_yaw = 0.0;
                mob.previous_head_yaw = 0.0;
            }
        }
        // TrialSpawner.spawnMob sets persistence after finalization, before equipment.
        if r.trial {
            mob.persistent = true;
        }
        // The equipment roll consumes RNG before admission, including for vetoed spawns.
        if let Some(table) = r.equipment {
            self.equip_mob_from_table(mob, table, random, sequences);
        }
        // NeoForgeEventHandler.builtinMobSpawnBlocker vetoes admission on spawn_cancelled.
        if decision.veto_admission {
            return None;
        }
        let key = mob.id;
        if let Some(given) = configured_tags {
            let edited = std::mem::take(&mut mob.extra.spawn_tags);
            if edited != given
                && let Some(Tag::Compound(raw)) = tag
            {
                let key = "Tags".into();
                let original = match raw.remove(&key) {
                    Some(Tag::List(tags)) => tags,
                    _ => Vec::new(),
                };
                // Retained tags keep the original NBT strings and their UTF-16 units.
                let mut saved = HashMap::<_, std::collections::VecDeque<_>>::new();
                for (name, tag) in given.into_iter().zip(original) {
                    saved.entry(name).or_default().push_back(tag);
                }
                let tags: Vec<_> = edited
                    .into_iter()
                    .map(|name| {
                        saved
                            .get_mut(&name)
                            .and_then(|tags| tags.pop_front())
                            .unwrap_or_else(|| Tag::String(name.as_str().into()))
                    })
                    .collect();
                if !tags.is_empty() {
                    raw.insert(key, Tag::List(tags));
                }
            }
        }
        // Preview and insertion use the same tree and the same admission result.
        match &mut plan {
            Planned::Finalized(mob) => l.record_pending(mob),
            Planned::Loaded(tree, _) => {
                if !l.admit_pending_tree(tree) {
                    return None;
                }
            }
        }
        planned.push(plan);
        Some(SpawnerAdded { id: key, mob: true })
    }
    pub(super) fn decode_spawner_tree(
        &self,
        entity: &sunrise_core::nbt::Compound,
        position: [f64; 3],
        yaw: f32,
    ) -> Option<SpawnTree> {
        let mut tag = entity.clone();
        tag.insert(
            "Rotation".into(),
            Tag::List(vec![Tag::Float(yaw.to_bits()), Tag::Float(0f32.to_bits())]),
        );
        let mut tag = Tag::Compound(tag);
        // EntityType.loadEntityRecursive runs the placement callback for every member.
        super::commands::place_summoned(&mut tag, position);
        SpawnTree::decode(self, tag)
    }
    /// What a spawner's tick or a spawn egg did, shown and sent.
    pub(in crate::game::server_side) fn apply_spawner_events(
        &mut self,
        key: Pos,
        events: Vec<SpawnerEvent>,
        ids: &HashMap<u32, u32>,
        cart: Option<u32>,
    ) -> Result<()> {
        let pos = [key.0, key.1, key.2];
        for event in events {
            match event {
                // A cart has no block to update: `sendBlockUpdated` finds nothing to send.
                SpawnerEvent::BlockUpdated if cart.is_some() => {}
                SpawnerEvent::BlockUpdated => self.spawner_block_updated(key),
                // `broadcastEvent`: `level.broadcastEntityEvent(cart, id)`.
                SpawnerEvent::BlockEvent(id) if cart.is_some() => {
                    let at = pos.map(|c| f64::from(c) + 0.5);
                    self.lan_entity_event(cart.unwrap_or_default() as i32, id as i8, at);
                }
                SpawnerEvent::BlockEvent(id) => {
                    self.lan_block_event(pos, id, 0);
                    // The client copy's onEventTriggered.
                    if let Some(BlockEntity::Spawner(s)) =
                        self.common.world.block_entities.get_mut(&key)
                    {
                        s.client_event(id);
                    }
                }
                SpawnerEvent::LevelEvent(id) => self.server_level_event_at(id, pos, 0),
                SpawnerEvent::EntityPlace(mob, at) => {
                    let who = ids.get(&mob).map(|id| Who::Mob(*id));
                    self.post_block_game_event("minecraft:entity_place", at, who, None);
                }
                SpawnerEvent::SpawnAnim(mob) => {
                    if let Some(id) = ids.get(&mob) {
                        self.mob_poof(*id);
                    }
                }
            }
        }
        Ok(())
    }
    /// `sendBlockUpdated`: the clients load the new update tag (the client
    /// copy's delay follows it, its display mob is made again).
    pub(in crate::game::server_side) fn spawner_block_updated(&mut self, key: Pos) {
        self.common
            .world
            .path_type_block_updated(sunrise_core::position::BlockPos::new(key.0, key.1, key.2));
        if let Some(BlockEntity::Spawner(s)) = self.common.world.block_entities.get_mut(&key) {
            s.client_loaded();
        }
        if let Some(log) = self.common.world.block_log.as_mut() {
            log.push(key);
        }
        self.common.world.modified.insert((key.0 >> 4, key.2 >> 4));
    }
    /// `LivingEntity.makePoofParticles` (entity event 20) for a mob.
    pub(in crate::game) fn mob_poof(&mut self, id: u32) {
        let Some(m) = self.common.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let at = m.position;
        self.lan_entity_event(id as i32, 20, [at.x, at.y, at.z]);
    }
    /// `SpawnEggItem.useOn` on a spawner: the egg sets its mob, and an op's
    /// egg its effective entity `data` too.
    /// None when the clicked block has no spawner.
    pub(in crate::game) fn use_spawn_egg_on_spawner(
        &mut self,
        pos: [i32; 3],
        kind: &str,
        data: Option<&sunrise_core::nbt::Compound>,
    ) -> Option<bool> {
        let key = (pos[0], pos[1], pos[2]);
        let trial = match self.common.world.block_entities.get(&key) {
            Some(BlockEntity::Spawner(_)) => false,
            Some(BlockEntity::TrialSpawner(_)) => true,
            _ => return None,
        };
        if !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::SPAWNER_BLOCKS_WORK)
        {
            self.server_system_message(sunrise_game_data::text::Text::translate(
                "advMode.notEnabled.spawner",
                &[],
            ));
            return Some(false);
        }
        if trial {
            // TrialSpawnerBlockEntity.setEntityId and setEntityData:
            // overrideEntityToSpawn, which ends in setState(INACTIVE).
            if let Some(BlockEntity::TrialSpawner(t)) =
                self.common.world.block_entities.get_mut(&key)
            {
                match data {
                    Some(data) => t.override_entity_data(kind, data),
                    None => t.override_entity(kind),
                }
            }
            let ominous = self.trial_spawner_block(key).is_some_and(|(_, o)| o);
            let _ = self.set_trial_spawner_state(key, TrialState::Inactive, ominous);
        } else {
            let mut events = Vec::new();
            let mut random = std::mem::replace(
                &mut self.common.world.random,
                LevelRandom::for_level(LegacyRandom::new(0)),
            );
            if let Some(BlockEntity::Spawner(s)) = self.common.world.block_entities.get_mut(&key) {
                match data {
                    Some(data) => s.set_entity_data(kind, data, &mut random, &mut events),
                    None => s.set_entity_id(kind, &mut random, &mut events),
                }
            }
            self.common.world.random = random;
            let _ = self.apply_spawner_events(key, events, &HashMap::new(), None);
        }
        // sendBlockUpdated(3) and the player's block_change game event.
        self.spawner_block_updated(key);
        self.post_block_game_event("minecraft:block_change", pos, Some(Who::Player), None);
        Some(true)
    }
    /// `EntityType.createDefaultStackConfig` on an entity an item made (a
    /// vehicle, a hanging entity, an armor stand): the item's custom name and
    /// custom data (`applyComponentsFromItemStack`:
    /// `Entity.applyImplicitComponent`'s two, saved as `CustomName` and
    /// `data`), then its `minecraft:entity_data` merged over the entity's own
    /// compound and loaded (`updateCustomEntityTag`) when it names this type.
    /// An op-only type (command block and spawner minecarts) takes it from an
    /// op player only, never from a dispenser (`by_player` false).
    pub(in crate::game) fn apply_item_stack_config(
        &mut self,
        target: super::commands::Target,
        kind: &str,
        stack: &sunrise_items::items::ItemStack,
        by_player: bool,
    ) {
        let op = by_player && self.acting_player_is_op();
        let configures = stack
            .data()
            .is_some_and(|d| d.custom_name.is_some() || d.other("minecraft:custom_data").is_some())
            || sunrise_sim::block_entity_item::spawn_egg_entity_data(stack, kind, op).is_some();
        if !configures {
            return;
        }
        let Some(own) = self.command_target_nbt(target) else {
            return;
        };
        let tag = sunrise_sim::block_entity_item::stack_configured_entity_by(stack, kind, &own, op);
        // The entity's own compound, its name and data merged in: it loads back.
        let _ = self.apply_entity_nbt(target, tag);
    }
}
