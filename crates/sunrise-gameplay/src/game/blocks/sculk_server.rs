//! Server-side sculk and game-event simulation: dispatching events to sculk
//! sensors, shriekers and wardens (`GameEventDispatcher`), the vibration
//! ticker, redstone output, resonance, shrieks, warden summoning, and catalysts.
use super::Result;
use mod_api::client::particles::PositionSource;
use std::collections::HashSet;
use sunrise_core::{
    geometry::{Aabb, Direction, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use sunrise_game_data::who::EventSource;
use sunrise_sim::join_seq::ListStamp;
use sunrise_world::registry::BlockStateId;
use {
    sunrise_level::containers::BlockEntity, sunrise_level::fast_hash::FxHashMap,
    sunrise_level::fast_hash::FxHashSet, sunrise_level::particle_options::ParticleType,
    sunrise_level::sculk, sunrise_level::sculk::Context, sunrise_level::sculk::Info,
    sunrise_level::sculk::SculkEntity, sunrise_level::sculk::SculkKind,
    sunrise_level::sculk::WardenSpawnTracker, sunrise_sim::mobs::Mob,
    sunrise_sim::mobs::MovementEmission as Steps, sunrise_sim::mobs::Species,
    sunrise_sim::mobs::Who, sunrise_sim::mobs::allay, sunrise_sim::mobs::warden,
};

#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay_common::game::side_ops::events::{
    TEST_POSTED_EVENT_ORDER, TEST_POSTED_EVENTS,
};

/// The sculk block kinds by state id, built once.
#[derive(Default)]
pub(in crate::game) struct Tables {
    pub(in crate::game) built: bool,
    /// 0 none, 1 sensor, 2 calibrated sensor, 3 shrieker, 4 catalyst.
    pub(in crate::game) kinds: Vec<u8>,
    pub(in crate::game) occludes: Vec<bool>,
    pub(in crate::game) dampens: Vec<bool>,
    pub(in crate::game) resonator: Vec<bool>,
    /// The `minecraft:climbable` tag, and the two redstone ores: read for
    /// every mob on every tick.
    pub(in crate::game) climbable: Vec<bool>,
    pub(in crate::game) redstone_ore: Vec<bool>,
}
#[derive(Default)]
pub(in crate::game) struct PlayerEvents {
    pub(in crate::game) steps: Option<Steps>,
    pub(in crate::game) last_health: Option<f32>,
    pub(in crate::game) mount: Option<(u32, [f64; 3])>,
}

#[derive(Default)]
pub struct SculkState {
    /// The player's warning level toward a warden.
    pub tracker: WardenSpawnTracker,
    pub(in crate::game) tables: Tables,
    /// The sculk block entities in the order they were added.
    pub(in crate::game) index: Index,
    pub known: HashSet<(i32, i32)>,
    /// The chunk map's membership revision `known` was taken at.
    pub known_revision: Option<u64>,
    /// The active player's movement-emission state (mobs keep their own).
    pub(in crate::game) steps: Option<Steps>,
    /// Items that have fallen since last touching the ground or water.
    pub(in crate::game) falling_items: HashSet<u32>,
    /// The vehicle each rider had last tick and where it stood.
    pub(in crate::game) mounts: FxHashMap<Who, (u32, [f64; 3])>,
    pub damaged: FxHashMap<u32, i64>,
    /// Set while the mobs' events are posted (`tick_mob_sculk_events`).
    pub(in crate::game) mob_slots: Option<MobSlots>,
    pub(in crate::game) last_health: Option<f32>,
    /// Who causes the block changes being processed (the player's actions).
    pub actor: Option<Who>,
    pub(in crate::game) ticks: u32,
    pub tracker_loaded: bool,
    /// The wardens' and allays' slots in the mob list, and the list they were
    /// read from: see `listener_slots`.
    pub(in crate::game) listeners: Vec<u32>,
    pub(in crate::game) listeners_of: Option<ListStamp>,
}

/// How far from a warden's or an allay's feet an event may be heard: their
/// listeners reach 16 blocks from the eye's block (`warden::listen`,
/// `allay::listen`), the eye stands under 3 blocks up, and flooring to blocks
/// adds under 2. Past this they do nothing with it.
pub(in crate::game) const LISTENING_MOB_REACH: f64 = 24.0;

pub(in crate::game) fn dist_sqr_f(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum()
}

/// Where each mob is in `Game::mobs` while the mobs' events are posted, for
/// the thousands of lookups by id they make, and whether any mob listens.
pub(in crate::game) struct MobSlots {
    /// The first slot of each id, as a scan would find.
    pub(in crate::game) by_id: FxHashMap<u32, usize>,
    /// The wardens and allays (the mobs that listen): slot and id.
    pub(in crate::game) listeners: Vec<(usize, u32)>,
    /// `Game::next_entity` when they were taken: unchanged, no mob joined.
    pub(in crate::game) next_entity: u32,
}

impl SculkState {
    pub(in crate::game::server_side) fn swap_player(&mut self, other: &mut PlayerEvents) {
        std::mem::swap(&mut self.steps, &mut other.steps);
        let mount = self.mounts.remove(&Who::Player);
        if let Some(guest) = other.mount.take() {
            self.mounts.insert(Who::Player, guest);
        }
        other.mount = mount;
        std::mem::swap(&mut self.last_health, &mut other.last_health);
    }

    /// Exchange the level-held parts (its sculk block entities and the
    /// chunks scanned for them) with another level's (game/world/levels.rs).
    pub(in crate::game::server_side) fn swap_level(&mut self, other: &mut Self) {
        std::mem::swap(&mut self.index, &mut other.index);
        std::mem::swap(&mut self.known, &mut other.known);
    }
}
/// An insertion-ordered set of block positions (`Level.blockEntityTickers`
/// keeps the order block entities were added in).
#[derive(Default)]
pub struct Index {
    pub(in crate::game) order: Vec<(i32, i32, i32)>,
    pub(in crate::game) set: HashSet<(i32, i32, i32)>,
    /// The positions by 32-block cell, each with its rank: ranks only grow,
    /// so sorting by them gives `order`'s order. Every mob's step posts a
    /// game event, and the listeners near it are read from at most eight
    /// cells instead of the whole level's.
    pub(in crate::game) cells: FxHashMap<(i32, i32, i32), Vec<(u64, (i32, i32, i32))>>,
    pub(in crate::game) next: u64,
}
pub(in crate::game::server_side) fn index_cell(key: (i32, i32, i32)) -> (i32, i32, i32) {
    (key.0 >> 5, key.1 >> 5, key.2 >> 5)
}
impl Index {
    pub(in crate::game::server_side) fn insert(&mut self, key: (i32, i32, i32)) {
        if self.set.insert(key) {
            self.order.push(key);
            self.cells
                .entry(index_cell(key))
                .or_default()
                .push((self.next, key));
            self.next += 1;
        }
    }
    pub(in crate::game::server_side) fn remove(&mut self, key: &(i32, i32, i32)) {
        if self.set.remove(key) {
            self.order.retain(|k| k != key);
            let cell = index_cell(*key);
            if let Some(list) = self.cells.get_mut(&cell) {
                list.retain(|(_, k)| k != key);
                if list.is_empty() {
                    self.cells.remove(&cell);
                }
            }
        }
    }
    /// The positions whose `dist_sqr` to `at` is at most `radius` squared,
    /// in `order`'s order: what filtering `iter` gives.
    pub(in crate::game) fn within(&self, at: [i32; 3], radius: i32) -> Vec<(i32, i32, i32)> {
        let limit = i64::from(radius * radius);
        let span = |v: i32| {
            let cell = |w: i64| (w >> 5).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
            cell(i64::from(v) - i64::from(radius))..=cell(i64::from(v) + i64::from(radius))
        };
        let mut found: Vec<(u64, (i32, i32, i32))> = Vec::new();
        for x in span(at[0]) {
            for y in span(at[1]) {
                for z in span(at[2]) {
                    if let Some(list) = self.cells.get(&(x, y, z)) {
                        found.extend(list.iter().filter(|(_, k)| dist_sqr(at, *k) <= limit));
                    }
                }
            }
        }
        found.sort_unstable_by_key(|(rank, _)| *rank);
        found.into_iter().map(|(_, k)| k).collect()
    }
    pub(in crate::game::server_side) fn contains(&self, key: &(i32, i32, i32)) -> bool {
        self.set.contains(key)
    }
    pub(in crate::game) fn iter(&self) -> impl Iterator<Item = &(i32, i32, i32)> {
        self.order.iter()
    }
    pub(in crate::game::server_side) fn retain(
        &mut self,
        mut keep: impl FnMut(&(i32, i32, i32)) -> bool,
    ) {
        self.order.retain(|k| keep(k));
        self.set.retain(|k| keep(k));
        self.cells.retain(|_, list| {
            list.retain(|(_, k)| keep(k));
            !list.is_empty()
        });
    }
}
pub(in crate::game) const KIND_SENSOR: u8 = 1;
pub(in crate::game) const KIND_CALIBRATED: u8 = 2;
pub(in crate::game) const KIND_SHRIEKER: u8 = 3;
pub(in crate::game) const KIND_CATALYST: u8 = 4;

pub(in crate::game::server_side) fn center(p: (i32, i32, i32)) -> [f64; 3] {
    [
        f64::from(p.0) + 0.5,
        f64::from(p.1) + 0.5,
        f64::from(p.2) + 0.5,
    ]
}
pub(in crate::game) fn block_of(p: [f64; 3]) -> [i32; 3] {
    [
        p[0].floor() as i32,
        p[1].floor() as i32,
        p[2].floor() as i32,
    ]
}
pub(in crate::game::server_side) fn dist_sqr(a: [i32; 3], b: (i32, i32, i32)) -> i64 {
    let d = |x: i32, y: i32| i64::from(x) - i64::from(y);
    d(a[0], b.0).pow(2) + d(a[1], b.1).pow(2) + d(a[2], b.2).pow(2)
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn server_post_block_game_event(
        &mut self,
        event: &'static str,
        pos: [i32; 3],
        actor: Option<Who>,
        state: Option<BlockStateId>,
    ) {
        self.post_block_game_event(event, pos, actor, state);
    }

    #[allow(dead_code)]
    pub(in crate::game) fn server_post_game_event(
        &mut self,
        event: &'static str,
        pos: [f64; 3],
        ctx: Context,
    ) {
        self.post_game_event(event, pos, ctx);
    }

    pub(in crate::game) fn server_sculk_actor(&self) -> Option<Who> {
        self.server.sculk.actor
    }

    /// `Entity.gameEvent(event, attributedTo)` at `at`, attributed to `source`
    /// (a LAN guest posts as that player), else to the entity itself.
    pub(in crate::game) fn entity_event_by(
        &mut self,
        event: &'static str,
        at: [f64; 3],
        source: Option<super::commands::Target>,
    ) {
        use super::commands::Target;
        if let Some(Target::Guest(conn)) = source {
            if self.server.lan_acting == Some(conn) {
                let ctx = self.player_context(None);
                self.post_game_event(event, at, ctx);
                return;
            }
            let posted = self
                .as_guest(conn, |g| {
                    let ctx = g.player_context(None);
                    g.post_game_event(event, at, ctx);
                })
                .is_some();
            if posted {
                return;
            }
        }
        let mut ctx = match source {
            Some(Target::Player) => self.player_context(None),
            Some(Target::Mob(id)) => self.mob_context(id, None),
            Some(target) => self
                .network_entity_id(target)
                .map(|id| self.entity_event_context(id as u32, None))
                .unwrap_or_default(),
            None => Context::default(),
        };
        // A stand is living, but Warden.canTargetEntity explicitly excludes it.
        ctx.living |= matches!(source, Some(Target::Stand(_)));
        self.post_game_event(event, at, ctx);
    }
    /// `GameEvent.Context.of(entity, state)` for a non-living entity.
    pub(in crate::game) fn entity_event_context(
        &self,
        id: u32,
        state: Option<BlockStateId>,
    ) -> Context {
        let dampens = self
            .common
            .dropped
            .iter()
            .find(|item| item.id == id)
            .is_some_and(|item| {
                world_items!(self)
                    .item_tags
                    .get("minecraft:dampens_vibrations")
                    .is_some_and(|tag| tag.contains(item.stack.item.as_ref()))
            });
        Context {
            source: Some(EventSource::Entity(id)),
            source_uuid: self.common.persist.uuid_of(id),
            affected_state: state,
            dampening_state: self.state_dampens(state),
            dampens,
            ..Context::default()
        }
    }
    /// `GameEvent.Context.of(player, state)`.
    pub(in crate::game) fn player_context(&self, state: Option<BlockStateId>) -> Context {
        let (dead, creative) = self.common.player.as_ref().map_or((true, true), |p| {
            (p.health.dead_or_dying(), self.common.creative)
        });
        Context {
            source: Some(Who::Player.into()),
            owner: None,
            affected_state: state,
            source_uuid: None,
            owner_uuid: None,
            dampening_state: self.state_dampens(state),
            spectator: self.common.spectator(),
            stepping_carefully: self.server_acting_sneak(),
            dampens: false,
            targetable: !dead && !creative && !self.common.spectator(),
            living: true,
            player: true,
        }
    }
    /// `Level.gameEvent`: tell the listeners within reach.
    pub fn post_game_event(&mut self, event: &'static str, pos: [f64; 3], ctx: Context) {
        #[cfg(any(test, feature = "test-support"))]
        TEST_POSTED_EVENTS.with(|events| {
            if let Some(events) = events.borrow_mut().as_mut() {
                events.push((event, pos, ctx.source.and_then(|source| source.actor())));
            }
        });
        #[cfg(any(test, feature = "test-support"))]
        TEST_POSTED_EVENT_ORDER.with(|order| {
            if let Some(order) = order.borrow_mut().as_mut() {
                order.push((
                    event,
                    ctx.source.and_then(|source| source.actor()),
                    self.server.lan_entity_events.len(),
                    if self.common.is_logical_server() {
                        self.server.pending_sounds.len()
                    } else {
                        0
                    },
                ));
            }
        });
        #[cfg(any(test, feature = "test-support"))]
        item_use_event_fixture::record_event(
            event,
            pos,
            ctx.source.and_then(|source| source.actor()),
        );
        self.post_game_event_with(event, pos, |_| ctx);
    }
    /// [`Self::post_game_event`] with the context worked out only when some
    /// listener may hear the event (`GameEventDispatcher.post` visits only the
    /// listeners registered in the sections around it): most of a server's
    /// mobs are near none. A subscribed mod hook still hears every event first.
    pub(in crate::game) fn post_game_event_with(
        &mut self,
        event: &'static str,
        pos: [f64; 3],
        mut make_ctx: impl FnMut(&mut Self) -> Context,
    ) {
        super::dispense_fixture::record(|| {
            format!(
                "G {},{},{} {event}",
                pos[0].floor() as i32,
                pos[1].floor() as i32,
                pos[2].floor() as i32
            )
        });
        debug_assert!(
            self.is_logical_server(),
            "post_game_event_with called on Client role"
        );
        let ctx = if mod_api::__private::listened(mod_api::HookId::VanillaGameEvent) {
            let ctx = make_ctx(self);
            let cause = event_cause(ctx.source, self.server.lan_acting_entity);
            let mut cancel = false;
            self.native_vanilla_game_event(
                self.common.dim.id(),
                event,
                pos,
                cause,
                ctx.affected_state,
                &mut cancel,
            );
            if cancel {
                return;
            }
            Some(ctx)
        } else {
            None
        };
        if !self.server.sculk.tables.built {
            return;
        }
        let at = block_of(pos);
        let radius = sculk::notification_radius(event).min(16);
        let near = self.server.sculk.index.within(at, radius);
        // The wardens and allays that may hear it, in the mobs' order, while
        // the slots taken for the mobs' events hold; else every mob is asked.
        let mobs = self.server.sculk.mob_slots.as_ref().and_then(|s| {
            let mut near = Vec::new();
            for &(i, id) in &s.listeners {
                let mob = self.common.mobs.get(i)?;
                if mob.id != id {
                    return None;
                }
                // A mod's game-event listener may have moved it since the slots were taken.
                let from = [mob.position.x, mob.position.y, mob.position.z];
                let d = dist_sqr_f(from, pos);
                if d.is_nan() || d <= LISTENING_MOB_REACH * LISTENING_MOB_REACH {
                    near.push(i);
                }
            }
            (s.next_entity == self.server.next_entity).then_some(near)
        });
        if near.is_empty() && mobs.as_ref().is_some_and(Vec::is_empty) {
            return;
        }
        let ctx = ctx.unwrap_or_else(|| make_ctx(self));
        let ctx = self.with_vibration_uuids(ctx);
        for key in near {
            self.sculk_listen(key, event, pos, &ctx);
        }
        match mobs {
            Some(mobs) => {
                for i in mobs {
                    self.mob_listen(i, event, pos, &ctx);
                }
            }
            None => {
                let stamp = self.common.mobs.stamp();
                let slots = self.listener_slots(stamp);
                for &i in &slots {
                    if (i as usize) < self.common.mobs.len() {
                        self.mob_listen(i as usize, event, pos, &ctx);
                    }
                }
                self.server.sculk.listeners = slots;
                self.server.sculk.listeners_of = Some(stamp);
            }
        }
    }
    /// A projectile's own event (`gameEvent(event)`, `Context.of(this, state)`):
    /// its owner as the projectile's owner, `state` the affected state. A
    /// guest's projectile posts as theirs, or with no owner once they have left
    /// (`Projectile.getOwner`), as its shot does.
    pub(in crate::game) fn projectile_own_event(
        &mut self,
        event: &'static str,
        owner: Option<Who>,
        guest: Option<u32>,
        at: [f64; 3],
        state: Option<BlockStateId>,
    ) {
        if let Some(conn) =
            guest.filter(|conn| owner.is_some() && self.server.lan_acting != Some(*conn))
        {
            let posted = self
                .as_guest(conn, |g| {
                    g.projectile_own_event(event, owner, None, at, state);
                })
                .is_some();
            if !posted {
                self.projectile_own_event(event, None, None, at, state);
            }
            return;
        }
        let mut ctx = Context {
            dampening_state: self.state_dampens(state),
            ..Context::default()
        };
        ctx.owner = owner;
        ctx.player = owner == Some(Who::Player) || self.rides_player(owner);
        self.post_game_event(event, at, ctx);
    }
    /// Events from entities moving, getting hurt and dying, derived from what
    /// changed this tick.
    pub(in crate::game) fn tick_entity_events(&mut self) {
        self.tick_player_sculk_events();
        self.tick_mob_sculk_events();
        if !self.server.sculk.falling_items.is_empty() {
            let present: std::collections::HashSet<u32> =
                self.common.dropped.iter().map(|item| item.id).collect();
            self.server
                .sculk
                .falling_items
                .retain(|id| present.contains(id));
        }
        // Dropped items are entities too: Entity.applyEffectsFromBlocks, from
        // ItemEntity.tick, so only for the items in entity-ticking chunks.
        let ticking = self.entity_ticking_area();
        let items: Vec<[f64; 3]> = self
            .common
            .dropped
            .iter()
            .filter(|i| {
                i.ground.on_ground
                    && ticking(i.position.x.floor() as i32, i.position.z.floor() as i32)
            })
            .map(|i| [i.position.x, i.position.y, i.position.z])
            .collect();
        for at in items {
            self.step_on_redstone_ore(at, false);
        }
    }
    pub fn tick_mob_sculk_events(&mut self) {
        let snapshot: Vec<(
            u32,
            [f64; 3],
            bool,
            bool,
            Species,
            Option<i64>,
            Option<u32>,
            u32,
            bool,
        )> = self
            .common
            .mobs
            .iter()
            .filter(|m| m.alive())
            .map(|m| {
                (
                    m.id,
                    [m.position.x, m.position.y, m.position.z],
                    m.ground.on_ground,
                    m.fluids.in_water,
                    m.species(),
                    m.last_damage.map(|(_, t)| t),
                    m.extra.mount,
                    m.age,
                    m.species() == Species::Bat && sunrise_sim::mobs::bat::resting(m),
                )
            })
            .collect();
        let mut by_id = FxHashMap::default();
        for (i, m) in self.common.mobs.iter().enumerate() {
            by_id.entry(m.id).or_insert(i);
        }
        let listeners = self
            .common
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| matches!(m.species(), Species::Warden | Species::Allay))
            .map(|(i, m)| (i, m.id))
            .collect();
        self.server.sculk.mob_slots = Some(MobSlots {
            by_id,
            listeners,
            next_entity: self.server.next_entity,
        });
        // Steps, landings, splashes, flaps and stepping on redstone ore come
        // from the mob's own tick (Entity.move, LivingEntity.aiStep's
        // applyEffectsFromBlocks), which ServerLevel.tick runs only in
        // entity-ticking chunks: for the mobs `tick_creature_entities` ticked,
        // and those standing in that area (the sculk fixture posts before the
        // mobs tick). Damage and mounting are raised by `hurt` and
        // `startRiding`, ticking or not.
        let ticking = self.entity_ticking_area();
        for (id, at, on_ground, in_water, species, damaged, vehicle, age, resting) in snapshot {
            let ticked = self.common.creatures.ticked.contains(&id)
                || ticking(at[0].floor() as i32, at[2].floor() as i32);
            if on_ground && ticked {
                self.step_on_redstone_ore(at, false);
            }
            // A mob first seen already riding raised its event when it spawned.
            let seen = self
                .mob_slot(id)
                .is_some_and(|i| self.common.mobs[i].movement_emission.is_some());
            self.mount_events(Who::Mob(id), vehicle, seen);
            let emits = !matches!(species, Species::Shulker);
            if emits && !ticked {
                // Where it is now, without a move: a mob put elsewhere while
                // it did not tick (a command) walked nowhere.
                if let Some(slot) = self.motion_steps_mut(Who::Mob(id)) {
                    let entry = slot.get_or_insert(Steps {
                        move_dist: 0.0,
                        next_step: 1.0,
                        last: at,
                        on_ground,
                        in_water,
                    });
                    entry.last = at;
                    entry.on_ground = on_ground;
                    entry.in_water = in_water;
                }
            } else if emits {
                let warden = species == Species::Warden;
                self.entity_motion_events(
                    Who::Mob(id),
                    at,
                    on_ground,
                    in_water,
                    false,
                    false,
                    warden,
                );
                // Entity.processFlappingMovement: what flaps this move flaps
                // over air only (the mob's own `MobEvent::GameEvent` is not
                // read here).
                if flaps(species, resting, on_ground, age) {
                    let below = self.common.world.block(
                        at[0].floor() as i32,
                        (at[1] - 1.0e-5).floor() as i32,
                        at[2].floor() as i32,
                    );
                    if self.common.res.facts().is_air(below) {
                        self.post_entity_event("minecraft:flap", Who::Mob(id));
                    }
                }
            }
            if let Some(t) = damaged
                && self.server.sculk.damaged.get(&id) != Some(&t)
            {
                self.server.sculk.damaged.insert(id, t);
                self.post_entity_event("minecraft:entity_damage", Who::Mob(id));
            }
        }
        self.server.sculk.mob_slots = None;
        let alive: FxHashSet<u32> = self.common.mobs.iter().map(|m| m.id).collect();
        self.server.sculk.damaged.retain(|id, _| alive.contains(id));
        self.server.sculk.mounts.retain(|who, _| match who {
            Who::Player => true,
            Who::Mob(id) => alive.contains(id),
        });
    }
    /// Entity.move emits on the authoritative server too, for each ServerPlayer.
    pub(in crate::game) fn tick_player_sculk_events(&mut self) {
        // The player.
        let player = self.common.player.as_ref().map(|p| {
            (
                [p.position.x, p.position.y, p.position.z],
                p.ground.on_ground,
                p.health.health(),
                p.flying,
            )
        });
        if let Some((at, on_ground, health, flying)) = player {
            let crouch = self.server_acting_sneak();
            if on_ground && !self.common.spectator() {
                self.step_on_redstone_ore(at, crouch);
            }
            self.mount_events(Who::Player, self.common.riding, true);
            let in_water = self.in_water_at(at);
            self.entity_motion_events(
                Who::Player,
                at,
                on_ground,
                in_water,
                crouch && on_ground,
                flying,
                false,
            );
            if let Some(last) = self.server.sculk.last_health
                && health < last
            {
                self.post_entity_event("minecraft:entity_damage", Who::Player);
            }
            self.server.sculk.last_health = Some(health);
        }
    }
    /// The block entity phase for sculk: vibration tickers and catalysts, in
    /// the order the block entities were added (`Level.tickBlockEntities`).
    pub(in crate::game::server_side) fn tick_sculk_blocks(&mut self) -> Result<()> {
        self.sculk_prepare();
        self.sculk_sync_index();
        // `ServerLevel.shouldTickBlocksAt` in a block-ticking chunk, from
        // every player's tickets (`update_chunk_tickets`), once the chunk is
        // lit: asked once per chunk, as the sensors share few chunks. Nothing
        // a sculk tick does loads, unloads or lights a chunk or moves a ticket.
        let levels = self.server.tickets.0.levels();
        let chunks = &self.common.world.chunks;
        let pending = &self.common.world.entities_pending;
        let mut ticking = FxHashMap::<(i32, i32), bool>::default();
        let mut ticks = |cx: i32, cz: i32| {
            *ticking.entry((cx, cz)).or_insert_with(|| {
                levels.block_ticking(cx, cz) && chunks.get(&(cx, cz)).is_some_and(|c| c.lit)
            })
        };
        // The ticking ones in the index's order, with whether the chunks
        // around them tick too. Most sensors and catalysts wait for a
        // vibration or a death: their tick would change nothing, so they
        // are passed over here (a vibration scheduled during these ticks is
        // chosen on the next tick at the earliest, as in vanilla).
        let entities = &self.common.world.block_entities;
        let due: Vec<((i32, i32, i32), bool)> = self
            .server
            .sculk
            .index
            .iter()
            .filter_map(|&key| {
                let (cx, cz) = (key.0 >> 4, key.2 >> 4);
                // `LevelChunk.isTicking`: its own chunk's entities loaded too.
                if !ticks(cx, cz)
                    || pending.contains(&(cx, cz))
                    || matches!(entities.get(&key), Some(BlockEntity::Sculk(e)) if e.idle())
                {
                    return None;
                }
                let adjacent = (-1..=1).all(|dx| (-1..=1).all(|dz| ticks(cx + dx, cz + dz)));
                Some((key, adjacent))
            })
            .collect();
        for (key, adjacent) in due {
            self.tick_sculk_block(key, adjacent)?;
        }
        Ok(())
    }
    /// `SpawnUtil.trySpawnMob(WARDEN, TRIGGERED, level, pos, 20, 5, 6,
    /// ON_TOP_OF_COLLIDER, false)`: every candidate spot creates a warden
    /// (`EntityType.create` draws its yaw from the level random and
    /// `finalizeSpawn` plays its agitated sound), which is discarded when
    /// `checkSpawnObstruction` fails.
    pub(in crate::game) fn try_summon_warden(
        &mut self,
        key: (i32, i32, i32),
        warning: i32,
    ) -> bool {
        if warning < 4 {
            return false;
        }
        let Some(kind) = self.common.res.mob_types().get("minecraft:warden").cloned() else {
            return false;
        };
        for _ in 0..20 {
            let r = &mut self.common.world.random;
            let dx = r.next_int_bound(11).unwrap_or(0) - 5;
            let dz = r.next_int_bound(11).unwrap_or(0) - 5;
            let Some(at) = self.spawn_position(key.0 + dx, key.1 + 6, key.2 + dz, 6) else {
                continue;
            };
            let spot = [
                f64::from(at.0) + 0.5,
                f64::from(at.1),
                f64::from(at.2) + 0.5,
            ];
            let yaw = mod_api::client::camera::wrap_degrees(
                self.common.world.random.next_float() * 360.0,
            );
            self.server.next_entity += 1;
            let mut mob = Mob::new(
                self.server.next_entity,
                kind.clone(),
                sunrise_core::geometry::Vec3::new(spot[0], spot[1], spot[2]),
                yaw,
            );
            // Warden.finalizeSpawn, then Mob.finalizeSpawn's draws from the
            // level random: the follow range bonus and left-handedness.
            let mut events = Vec::new();
            warden::finalize(&mut mob, true, &mut events);
            let _ = self.handle_mob_events(events);
            let r = &mut self.common.world.random;
            let bonus = r.triangle(0.0, 0.11485000000000001);
            mob.attrs.follow_range += mob.kind.follow_range * bonus;
            mob.update_max_visited_nodes();
            let _left_handed = r.next_float() < 0.05;
            if !self.warden_fits(&mob) {
                continue;
            }
            self.common.mobs.push(mob);
            self.admit_new_mobs(self.common.mobs.len() - 1);
            return true;
        }
        false
    }

    /// Run `f` with the player as the cause of the block changes it makes.
    pub fn with_player_actor<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let prior = self.server.sculk.actor.replace(Who::Player);
        let out = f(self);
        self.server.sculk.actor = prior;
        out
    }

    /// The `sculk` control command: listeners and their vibrations, or a
    /// test charge for the nearest catalyst.
    pub(in crate::game) fn sculk_command(&mut self, words: &[&str]) -> Result<serde_json::Value> {
        match words.get(1).copied() {
            Some("charge") => {
                let num = |i: usize| -> Result<f64> {
                    Ok(words.get(i).ok_or("missing argument")?.parse::<f64>()?)
                };
                let at = [num(2)?, num(3)?, num(4)?];
                let amount = num(5)? as u32;
                let taken = self.catalyst_takes_experience(at, |_| amount);
                Ok(serde_json::json!({ "ok": true, "taken": taken }))
            }
            Some("dig") => {
                // Let every warden's minute of calm run out.
                for m in self
                    .common
                    .mobs
                    .iter_mut()
                    .filter(|m| m.species() == Species::Warden)
                {
                    warden::expire_dig_cooldown(m);
                }
                Ok(serde_json::json!({ "ok": true }))
            }
            Some("warn") => {
                let level = words.get(2).ok_or("missing level")?.parse::<i32>()?;
                self.server.sculk.tracker.set_warning_level(level);
                Ok(serde_json::json!({ "ok": true }))
            }
            _ => Ok(self.sculk_status()),
        }
    }
    /// The `sculk` control command's listing.
    pub(in crate::game::server_side) fn sculk_status(&self) -> serde_json::Value {
        let listeners: Vec<serde_json::Value> = self
            .server
            .sculk
            .index
            .iter()
            .filter_map(|key| match self.common.world.block_entities.get(key) {
                Some(BlockEntity::Sculk(e)) => Some(serde_json::json!({
                    "pos": [key.0, key.1, key.2],
                    "kind": e.id(),
                    "current": e.vibration.current.map(|i| i.event),
                    "travel": e.vibration.travel,
                    "last_frequency": e.last_frequency,
                    "warning_level": e.warning_level,
                    "cursors": e.spreader.cursors.len(),
                })),
                _ => None,
            })
            .collect();
        serde_json::json!({
            "listeners": listeners,
            "warning_level": self.server.sculk.tracker.warning_level,
            "cooldown": self.server.sculk.tracker.cooldown_ticks,
            "darkness": 0.0,
            "chunks_known": self.server.sculk.known.len(),
        })
    }
    /// `WardenSpawnTrackerCommand`: clear the player's tracker or set its
    /// warning level.
    pub(in crate::game::server_side) fn cmd_warden_spawn_tracker(&mut self, level: Option<i32>) {
        use sunrise_game_data::text::Text;
        let key = match level {
            Some(level) => {
                self.server.sculk.tracker.set_warning_level(level);
                "commands.warden_spawn_tracker.set.success.single"
            }
            None => {
                self.server.sculk.tracker.reset();
                "commands.warden_spawn_tracker.clear.success.single"
            }
        };
        if self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::SEND_COMMAND_FEEDBACK)
        {
            self.server_system_message(Text::translate(
                key,
                &[Text::literal(super::chat_common::PLAYER_NAME)],
            ));
        }
    }
    /// `ServerPlayer`'s `warden_spawn_tracker` from the player data.
    pub(in crate::game) fn load_warden_tracker(&mut self) {
        if self.server.sculk.tracker_loaded {
            return;
        }
        self.server.sculk.tracker_loaded = true;
        if let Some(tag) = self
            .common
            .level
            .player_extra
            .as_ref()
            .and_then(|p| p.get("warden_spawn_tracker"))
        {
            self.server.sculk.tracker = WardenSpawnTracker::from_nbt(tag);
        }
    }
    /// Whether a mod canceled the `ServerPlayer.tick` of the world's owner, whose warden spawn
    /// tracker this is: looked up only when a Pre canceled a player.
    pub(in crate::game::server_side) fn owner_tick_canceled(&self) -> bool {
        !self.common.creatures.canceled_players.is_empty()
            && self
                .server
                .lan
                .as_ref()
                .and_then(|host| {
                    let uuid = host.owner.as_ref()?.uuid;
                    host.hub.guests.iter().find(|g| g.uuid == uuid)
                })
                .is_some_and(|g| self.player_tick_canceled(g.conn))
    }
    /// The tracker as saved, once it has been loaded.
    pub(in crate::game::server_side) fn warden_tracker_tag(
        &self,
    ) -> Option<sunrise_core::nbt::Tag> {
        self.server
            .sculk
            .tracker_loaded
            .then(|| self.server.sculk.tracker.to_nbt())
    }
    /// Write the tracker into the player data being saved. With a LAN owner,
    /// `store_owner` rewrites that from the owner's guest, so it puts the
    /// tracker there as well (`playerdata::with_warden`).
    pub(in crate::game::server_side) fn save_warden_tracker(&mut self) {
        let Some(tag) = self.warden_tracker_tag() else {
            return;
        };
        let extra = self
            .common
            .level
            .player_extra
            .get_or_insert_with(|| sunrise_core::nbt::Tag::Compound(Default::default()));
        if let sunrise_core::nbt::Tag::Compound(map) = extra {
            map.insert("warden_spawn_tracker".into(), tag);
        }
    }
    /// Who is causing the block changes being processed.
    pub(in crate::game) fn sculk_actor(&self) -> Option<Who> {
        self.server.sculk.actor
    }

    // ---- setup and the block entity index ----

    pub fn sculk_prepare(&mut self) {
        if self.server.sculk.tables.built {
            return;
        }
        let registry = self.common.pack.assets.registry;
        let n = registry.len();
        let tag = |name: &str| {
            self.common
                .res
                .items()
                .block_tags
                .get(name)
                .cloned()
                .unwrap_or_default()
        };
        let (wool, dampening, resonators, climbable) = (
            tag("minecraft:occludes_vibration_signals"),
            tag("minecraft:dampens_vibrations"),
            tag("minecraft:vibration_resonators"),
            tag("minecraft:climbable"),
        );
        let mut t = Tables {
            built: true,
            kinds: vec![0; n],
            occludes: vec![false; n],
            dampens: vec![false; n],
            resonator: vec![false; n],
            climbable: vec![false; n],
            redstone_ore: vec![false; n],
        };
        for i in 0..n {
            let facts = self.common.res.facts().get(BlockStateId(i as u32));
            let name: &str = &facts.name;
            t.kinds[i] = match name {
                "minecraft:sculk_sensor" => KIND_SENSOR,
                "minecraft:calibrated_sculk_sensor" => KIND_CALIBRATED,
                "minecraft:sculk_shrieker" => KIND_SHRIEKER,
                "minecraft:sculk_catalyst" => KIND_CATALYST,
                _ => 0,
            };
            t.occludes[i] = wool.contains(name);
            t.dampens[i] = dampening.contains(name);
            t.resonator[i] = resonators.contains(name);
            t.climbable[i] = climbable.contains(name);
            t.redstone_ore[i] = matches!(
                name,
                "minecraft:redstone_ore" | "minecraft:deepslate_redstone_ore"
            );
        }
        self.common.world.watched = t.kinds.iter().map(|k| *k != 0).collect();
        self.server.sculk.tables = t;
    }
    pub(in crate::game::server_side) fn kind_of_state(&self, state: u32) -> u8 {
        self.server
            .sculk
            .tables
            .kinds
            .get(state as usize)
            .copied()
            .unwrap_or(0)
    }
    pub(in crate::game::server_side) fn state_is(
        &self,
        table: &[bool],
        state: BlockStateId,
    ) -> bool {
        table.get(state.0 as usize).copied().unwrap_or(false)
    }
    pub(in crate::game::server_side) fn entity_for_kind(kind: u8) -> Option<SculkEntity> {
        Some(SculkEntity::new(match kind {
            KIND_SENSOR => SculkKind::Sensor,
            KIND_CALIBRATED => SculkKind::Calibrated,
            KIND_SHRIEKER => SculkKind::Shrieker,
            KIND_CATALYST => SculkKind::Catalyst,
            _ => return None,
        }))
    }
    /// Find the sculk blocks of chunks that just became resident, and apply
    /// the block writes recorded since the last tick.
    pub fn sculk_sync_index(&mut self) {
        // Chunks that arrived or left (every half second is plenty).
        // Nothing to do while no chunk came or went.
        let revision = self.common.world.chunks.membership_revision();
        if self.server.sculk.ticks.is_multiple_of(10)
            && self.server.sculk.known_revision != Some(revision)
        {
            self.server.sculk.known_revision = Some(revision);
            let loaded: HashSet<(i32, i32)> = self.common.world.chunks.keys().copied().collect();
            let arrived: Vec<(i32, i32)> = loaded
                .difference(&self.server.sculk.known)
                .copied()
                .collect();
            for key in arrived {
                self.sculk_scan_chunk(key);
            }
            // The blocks of every chunk that left, in one pass: a player
            // leaving unloads thousands of chunks at once.
            let gone: HashSet<(i32, i32)> = self
                .server
                .sculk
                .known
                .difference(&loaded)
                .copied()
                .collect();
            if !gone.is_empty() {
                self.server
                    .sculk
                    .index
                    .retain(|p| !gone.contains(&(p.0 >> 4, p.2 >> 4)));
            }
            self.server.sculk.known = loaded;
        }
        self.sculk_sync_writes();
    }

    /// `LevelChunk.setBlockState`: finish sculk placement before a command
    /// or `BlockItem.updateCustomBlockEntityTag` loads saved entity fields.
    pub(in crate::game) fn sculk_place_entity(&mut self, state: BlockStateId) -> bool {
        self.sculk_prepare();
        if self.kind_of_state(state.0) == 0 {
            return false;
        }
        self.sculk_sync_writes();
        true
    }

    /// `LevelChunk.removeGameEventListener`: unregister a sculk block entity's
    /// game-event listener when its block entity is removed.
    pub(super) fn sculk_unregister_listener(&mut self, key: (i32, i32, i32)) {
        self.server.sculk.index.remove(&key);
    }

    pub(in crate::game::server_side) fn sculk_sync_writes(&mut self) {
        let writes = std::mem::take(&mut self.common.world.watched_writes);
        for (key, old, new) in writes {
            let (old_kind, new_kind) = (self.kind_of_state(old), self.kind_of_state(new));
            if old_kind == new_kind {
                continue;
            }
            // SculkShriekerBlockEntity.preRemoveSideEffects.
            if old_kind == KIND_SHRIEKER
                && let Some(BlockEntity::Sculk(e)) = self.common.world.block_entities.get(&key)
                && self.shrieker_shrieking(BlockStateId(old))
            {
                let warning = e.warning_level;
                self.shrieker_respond(key, BlockStateId(old), warning);
            }
            if matches!(
                self.world.block_entities.get(&key),
                Some(BlockEntity::Sculk(_))
            ) {
                self.common.world.block_entities.remove(&key);
            }
            self.server.sculk.index.remove(&key);
            if let Some(e) = Self::entity_for_kind(new_kind) {
                self.common
                    .world
                    .block_entities
                    .insert(key, BlockEntity::Sculk(Box::new(e)));
                self.server.sculk.index.insert(key);
            }
        }
    }
    pub(in crate::game::server_side) fn sculk_scan_chunk(&mut self, key: (i32, i32)) {
        let Some(chunk) = self.common.world.chunks.get(&key).cloned() else {
            return;
        };
        let mut found = Vec::new();
        for (s, section) in chunk.sections.iter().enumerate() {
            let y0 = self.common.world.bottom_y() + s as i32 * 16;
            // Without a sculk state in its palette, no block of the section
            // is one: most sections are passed over without reading a block.
            if !section.blocks.maybe_has(|v| self.kind_of_state(v) != 0) {
                continue;
            }
            if section.blocks.uniform().is_some() {
                // A uniform section of sculk blocks does not exist.
                continue;
            }
            let mut a = [0u32; 4096];
            section.unpack(&mut a);
            for (i, v) in a.iter().enumerate() {
                if self.kind_of_state(*v) != 0 {
                    let (x, z, y) = (i & 15, (i >> 4) & 15, i >> 8);
                    found.push((
                        key.0 * 16 + x as i32,
                        y0 + y as i32,
                        key.1 * 16 + z as i32,
                        *v,
                    ));
                }
            }
        }
        for (x, y, z, state) in found {
            let pos = (x, y, z);
            if !matches!(
                self.world.block_entities.get(&pos),
                Some(BlockEntity::Sculk(_))
            ) && let Some(e) = Self::entity_for_kind(self.kind_of_state(state))
            {
                self.common
                    .world
                    .block_entities
                    .insert(pos, BlockEntity::Sculk(Box::new(e)));
            }
            self.server.sculk.index.insert(pos);
        }
    }
    pub(in crate::game::server_side) fn sculk_entity(
        &mut self,
        key: (i32, i32, i32),
    ) -> Option<&mut SculkEntity> {
        match self.common.world.block_entities.get_mut(&key) {
            Some(BlockEntity::Sculk(e)) => Some(e),
            _ => None,
        }
    }

    // ---- contexts ----

    pub(in crate::game) fn state_dampens(&self, state: Option<BlockStateId>) -> bool {
        state.is_some_and(|s| self.state_is(&self.server.sculk.tables.dampens, s))
    }
    /// `GameEvent.Context.of(mob, state)`.
    pub(in crate::game) fn mob_context(&mut self, id: u32, state: Option<BlockStateId>) -> Context {
        let mob = self.mob_slot(id).map(|i| &self.common.mobs[i]);
        self.context_of_mob(mob, id, state)
    }
    /// The slot of the first mob with this id, from the slots taken for the
    /// mobs' events while the mob is still there (mobs are only appended
    /// meanwhile), else from the mob index (as a scan finds it).
    pub(in crate::game) fn mob_slot(&mut self, id: u32) -> Option<usize> {
        if let Some(slots) = &self.server.sculk.mob_slots
            && let Some(&i) = slots.by_id.get(&id)
            && self.common.mobs.get(i).is_some_and(|m| m.id == id)
        {
            return Some(i);
        }
        self.common.mob_index.find(&self.common.mobs, id)
    }
    pub(in crate::game::server_side) fn context_of_mob(
        &self,
        mob: Option<&Mob>,
        id: u32,
        state: Option<BlockStateId>,
    ) -> Context {
        Context {
            source: Some(Who::Mob(id).into()),
            owner: None,
            affected_state: state,
            source_uuid: None,
            owner_uuid: None,
            dampening_state: self.state_dampens(state),
            spectator: false,
            stepping_carefully: false,
            dampens: mob.is_some_and(|m| m.species() == Species::Warden),
            targetable: mob.is_some_and(warden::can_be_targeted),
            living: true,
            player: mob.is_some_and(|m| m.rider.is_some()),
        }
    }
    pub(in crate::game) fn who_context(
        &mut self,
        who: Option<Who>,
        state: Option<BlockStateId>,
    ) -> Context {
        match who {
            Some(Who::Player) => self.player_context(state),
            Some(Who::Mob(id)) => self.mob_context(id, state),
            None => Context {
                affected_state: state,
                dampening_state: self.state_dampens(state),
                ..Context::default()
            },
        }
    }

    pub(in crate::game::server_side) fn event_source_context(
        &mut self,
        source: Option<EventSource>,
        state: Option<BlockStateId>,
    ) -> Context {
        match source {
            Some(EventSource::Entity(id)) => self.entity_event_context(id, state),
            _ => self.who_context(source.and_then(EventSource::actor), state),
        }
    }
    /// `LivingEntity.onEquipItem`'s game event for a mob: EQUIP when the new
    /// stack is equippable in that slot, UNEQUIP when the slot is emptied.
    pub(in crate::game) fn mob_equip_event(&mut self, id: u32, at: [f64; 3], equip: bool) {
        let event = if equip {
            "minecraft:equip"
        } else {
            "minecraft:unequip"
        };
        let ctx = self.who_context(Some(Who::Mob(id)), None);
        self.post_game_event(event, at, ctx);
    }

    // ---- posting ----

    /// The slots of the wardens and allays (the mobs that hear game events),
    /// in list order, for an event posted outside the mobs' own events. A walk
    /// over the list misses the cache once per mob, and a busy server posts
    /// hundreds of such events a tick: one walk serves until the list is next
    /// written to ([`ListStamp`]).
    pub(in crate::game) fn listener_slots(&mut self, stamp: ListStamp) -> Vec<u32> {
        let mut slots = std::mem::take(&mut self.server.sculk.listeners);
        // Taken: an event a listener posts while it hears this one finds none
        // and walks the list itself.
        if self.server.sculk.listeners_of.take() != Some(stamp) {
            slots.clear();
            slots.extend(
                self.common
                    .mobs
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| matches!(m.species(), Species::Warden | Species::Allay))
                    .map(|(i, _)| i as u32),
            );
        }
        slots
    }
    /// The context with the saved identities of its source and owner, as
    /// `VibrationInfo`'s constructor takes them from the entities when the
    /// event is heard. Only what the context already names is filled.
    pub(in crate::game) fn with_vibration_uuids(&self, mut ctx: Context) -> Context {
        if ctx.source_uuid.is_none() {
            ctx.source_uuid = match ctx.source {
                Some(EventSource::Entity(id)) => self.common.persist.uuid_of(id),
                source => self.vibration_uuid(source.and_then(EventSource::actor)),
            };
        }
        if ctx.owner_uuid.is_none() {
            ctx.owner_uuid = self.vibration_uuid(ctx.owner);
        }
        ctx
    }
    /// The saved UUID of a vibration's mob through the save layer's
    /// `save_who_uuid`. `Who::Player` stays `None`: on a server it can be the
    /// acting guest as well as the host, so it names no single entity here.
    pub(in crate::game::server_side) fn vibration_uuid(
        &self,
        who: Option<Who>,
    ) -> Option<[i32; 4]> {
        match who {
            Some(who @ Who::Mob(_)) => self.save_who_uuid(who),
            _ => None,
        }
    }
    /// The mob at `index` hears a game event, if it is a warden or an allay.
    pub(in crate::game) fn mob_listen(
        &mut self,
        index: usize,
        event: &'static str,
        pos: [f64; 3],
        ctx: &Context,
    ) {
        if self.common.mobs[index].species() == Species::Warden && self.common.mobs[index].alive() {
            self.warden_listen(index, event, pos, ctx);
        }
        if self.common.mobs[index].species() == Species::Allay && self.common.mobs[index].alive() {
            self.allay_listen(index, event, pos, ctx);
            // Allay.JukeboxListener, its own listener beside the vibrations'.
            let playing = match event {
                "minecraft:jukebox_play" => Some(true),
                "minecraft:jukebox_stop_play" => Some(false),
                _ => None,
            };
            let m = &mut self.common.mobs[index];
            if let Some(playing) = playing
                && allay::hears_jukebox(m, pos)
            {
                let at = pos.map(|v| v.floor() as i32);
                allay::set_jukebox_playing(m, at, playing);
            }
        }
    }
    /// A game event at a block (`Level.gameEvent(source, event, pos)`).
    pub(in crate::game) fn post_block_game_event(
        &mut self,
        event: &'static str,
        pos: [i32; 3],
        who: Option<Who>,
        state: Option<BlockStateId>,
    ) {
        let ctx = self.who_context(who, state);
        self.post_game_event(event, center((pos[0], pos[1], pos[2])), ctx);
    }
    /// `LivingEntity.actuallyHurt`'s damage vibration, heard as the hit lands;
    /// the end-of-tick scan then knows this damage.
    pub(in crate::game) fn mob_damage_event(&mut self, id: u32) {
        if let Some(t) = self
            .common
            .mob_index
            .get(&self.common.mobs, id)
            .and_then(|m| m.last_damage.map(|(_, t)| t))
        {
            self.server.sculk.damaged.insert(id, t);
        }
        self.post_entity_event("minecraft:entity_damage", Who::Mob(id));
    }
    /// A game event of an entity at its position.
    pub(in crate::game) fn post_entity_event(&mut self, event: &'static str, who: Who) {
        let at = match who {
            Who::Player => self
                .common
                .player
                .as_ref()
                .map(|p| [p.position.x, p.position.y, p.position.z]),
            Who::Mob(id) => self.mob_slot(id).map(|i| {
                let p = self.common.mobs[i].position;
                [p.x, p.y, p.z]
            }),
        };
        if let Some(at) = at {
            self.post_game_event_with(event, at, |g| g.who_context(Some(who), None));
        }
    }
    /// `LevelEvent::GameEvent` from the block behaviour library.
    pub(in crate::game) fn sculk_level_event(
        &mut self,
        pos: BlockPos,
        event: &'static str,
        state: Option<BlockStateId>,
    ) {
        let who = self.server.sculk.actor;
        self.post_block_game_event(event, [pos.x, pos.y, pos.z], who, state);
    }

    pub(in crate::game::server_side) fn occluded(&self, origin: [f64; 3], dest: [f64; 3]) -> bool {
        sculk::is_occluded(origin, dest, |p| {
            self.state_is(
                &self.server.sculk.tables.occludes,
                self.common.world.block(p.x, p.y, p.z),
            )
        })
    }
    /// `VibrationSystem.Listener.handleGameEvent` for a sculk block.
    pub(in crate::game) fn sculk_listen(
        &mut self,
        key: (i32, i32, i32),
        event: &'static str,
        pos: [f64; 3],
        ctx: &Context,
    ) {
        let Some(entity) = self.sculk_entity(key) else {
            self.server.sculk.index.remove(&key);
            return;
        };
        let (kind, radius, busy) = (
            entity.kind,
            entity.listener_radius(),
            entity.vibration.current.is_some(),
        );
        if kind == SculkKind::Catalyst || busy {
            return;
        }
        let source_block = block_of(pos);
        if dist_sqr(source_block, key) > i64::from(radius * radius) {
            return;
        }
        let tags = self.items().game_event_tags.clone();
        let listens = match kind {
            SculkKind::Shrieker => sculk::shrieker_can_listen(&tags, event),
            _ => sculk::is_vibration(&tags, event),
        };
        // VibrationSystem.User.isValidVibration: a player sneaking past a
        // sensor, whose step it ignores, is AVOID_VIBRATION (sensors only;
        // a shrieker cannot trigger it).
        if listens
            && matches!(kind, SculkKind::Sensor | SculkKind::Calibrated)
            && ctx.source == Some(Who::Player.into())
            && !ctx.spectator
            && ctx.stepping_carefully
            && sculk::ignored_when_sneaking(&tags, event)
        {
            self.trigger_player("minecraft:avoid_vibration");
        }
        if !sculk::valid_vibration(&tags, listens, event, ctx) {
            return;
        }
        let state = self.common.world.block(key.0, key.1, key.2);
        let can_receive = match kind {
            SculkKind::Sensor | SculkKind::Calibrated => {
                // The sensor does not hear its own placement or removal.
                let own = source_block == [key.0, key.1, key.2]
                    && matches!(event, "minecraft:block_destroy" | "minecraft:block_place");
                let frequency = self.mob_maps().vibration_frequency(event);
                let calibrated_out = kind == SculkKind::Calibrated && {
                    let back = self.calibrated_signal(key);
                    back != 0 && frequency != back
                };
                !own && !calibrated_out
                    && frequency != 0
                    && self
                        .common
                        .res
                        .behaviours()
                        .sculk_sensor_can_activate(state)
            }
            SculkKind::Shrieker => !self.shrieker_shrieking(state) && ctx.player,
            SculkKind::Catalyst => false,
        };
        if !can_receive {
            return;
        }
        let dest = center(key);
        if self.occluded(pos, dest) {
            return;
        }
        let distance = {
            let d = [pos[0] - dest[0], pos[1] - dest[1], pos[2] - dest[2]];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() as f32
        };
        let time = self.common.world.game_time;
        if let Some(entity) = self.sculk_entity(key) {
            entity.vibration.schedule(
                Info {
                    event,
                    distance,
                    pos,
                    source: ctx.source,
                    owner: ctx.owner,
                    source_uuid: ctx.source_uuid,
                    owner_uuid: ctx.owner_uuid,
                },
                time,
            );
        }
    }
    pub(in crate::game::server_side) fn calibrated_signal(&mut self, key: (i32, i32, i32)) -> i32 {
        let pos = BlockPos::new(key.0, key.1, key.2);
        let mut value = 0;
        self.with_level(|b, level, ticks| {
            value = b.calibrated_back_signal(level, ticks, pos);
        });
        value
    }
    pub(in crate::game::server_side) fn shrieker_shrieking(&self, state: BlockStateId) -> bool {
        self.state_property(state, "shrieking") == Some("true")
    }
    pub fn state_property(&self, state: BlockStateId, key: &str) -> Option<&str> {
        self.common
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get(key))
            .map(String::as_str)
    }

    /// `Warden.VibrationUser`: accept a vibration for the warden at `index`.
    pub(in crate::game::server_side) fn warden_listen(
        &mut self,
        index: usize,
        event: &'static str,
        pos: [f64; 3],
        ctx: &Context,
    ) {
        let now = self.common.world.game_time;
        let ctx = warden::heard_by(*ctx, &self.mob_teams(), self.common.mobs[index].id);
        let (occludes, world) = (&self.server.sculk.tables.occludes, &self.common.world);
        let tags = self.items().game_event_tags.clone();
        let avoided = warden::listen(
            &mut self.common.mobs[index],
            now,
            event,
            pos,
            &ctx,
            &tags,
            |a, b| {
                sculk::is_occluded(a, b, |p| {
                    let state = world.block(p.x, p.y, p.z);
                    occludes.get(state.0 as usize).copied().unwrap_or(false)
                })
            },
        );
        if avoided {
            self.trigger_player("minecraft:avoid_vibration");
        }
    }

    /// `VibrationSystem.Listener.handleGameEvent` for the allay at `index`
    /// (`Allay.VibrationUser`).
    pub(in crate::game::server_side) fn allay_listen(
        &mut self,
        index: usize,
        event: &'static str,
        pos: [f64; 3],
        ctx: &Context,
    ) {
        let now = self.common.world.game_time;
        let (occludes, world) = (&self.server.sculk.tables.occludes, &self.common.world);
        let dimension = self.common.dim.id();
        let tags = self.items().game_event_tags.clone();
        allay::listen(
            &mut self.common.mobs[index],
            now,
            event,
            pos,
            ctx,
            &tags,
            dimension,
            |a, b| {
                sculk::is_occluded(a, b, |p| {
                    let state = world.block(p.x, p.y, p.z);
                    occludes.get(state.0 as usize).copied().unwrap_or(false)
                })
            },
        );
    }

    // ---- ticking ----

    /// The entity phase of a tick for sculk: the player's warden spawn
    /// tracker (`ServerPlayer.tick`), and the events of entities standing on
    /// and moving over blocks.
    pub fn tick_sculk_entities(&mut self) {
        self.server.sculk.ticks = self.server.sculk.ticks.wrapping_add(1);
        // Its steps as spans for `sunrise-report` (nothing is read while spans are off).
        let mut laps = sunrise_level::spans::laps();
        self.sculk_prepare();
        laps.lap("sculk_prepare");
        self.load_warden_tracker();
        // ServerPlayer.tick's wardenSpawnTracker: the owner's, which a mod's Pre that canceled
        // the owner's tick stops this tick (game/network/lan_actions/tick.rs).
        if !self.owner_tick_canceled() {
            self.server.sculk.tracker.tick();
        }
        laps.lap("sculk_wardens");
        self.sculk_step_on();
        laps.lap("sculk_step_on");
        self.tick_entity_events();
        laps.lap("sculk_entity_events");
    }
    pub(in crate::game) fn tick_sculk_block(
        &mut self,
        key: (i32, i32, i32),
        adjacent: bool,
    ) -> Result<()> {
        let time = self.common.world.game_time;
        let Some(entity) = self.sculk_entity(key) else {
            self.server.sculk.index.remove(&key);
            return Ok(());
        };
        match entity.kind {
            SculkKind::Catalyst => {
                // SculkCatalystBlockEntity.serverTick.
                let mut spreader = std::mem::take(&mut entity.spreader);
                if !spreader.cursors.is_empty() {
                    let origin = BlockPos::new(key.0, key.1, key.2);
                    self.with_level(|b, level, ticks| {
                        b.sculk_update_cursors(level, ticks, &mut spreader, origin, true);
                    });
                    self.server_process_level_events()?;
                }
                if let Some(e) = self.sculk_entity(key) {
                    // Cursors added while spreading keep their place.
                    let added = std::mem::take(&mut e.spreader);
                    e.spreader = spreader;
                    for c in added.cursors {
                        e.spreader.add_cursor(c);
                    }
                }
            }
            SculkKind::Sensor | SculkKind::Calibrated | SculkKind::Shrieker => {
                let tick = entity
                    .vibration
                    .tick(time, adjacent, true, |d| d.floor() as i32);
                if let Some((info, travel)) = tick.started {
                    let source = PositionSource::Block([key.0, key.1, key.2]);
                    self.vibration_particle(source, info.pos, center(key), travel);
                }
                if let Some((info, travel)) = tick.resumed {
                    let source = PositionSource::Block([key.0, key.1, key.2]);
                    let dest = center(key);
                    let initial = info.distance.floor() as i32;
                    // `tryReloadVibrationParticle` works in doubles.
                    let alpha = if initial > 0 {
                        1.0 - f64::from(travel) / f64::from(initial)
                    } else {
                        0.0
                    };
                    let current_pos = [
                        info.pos[0] + (dest[0] - info.pos[0]) * alpha,
                        info.pos[1] + (dest[1] - info.pos[1]) * alpha,
                        info.pos[2] + (dest[2] - info.pos[2]) * alpha,
                    ];
                    self.vibration_particle(source, current_pos, dest, travel);
                }
                if let Some(info) = tick.received {
                    self.sculk_receive(key, info)?;
                }
            }
        }
        Ok(())
    }

    // ---- sensors ----

    /// `onReceiveVibration` of a sensor or a shrieker.
    pub(in crate::game::server_side) fn sculk_receive(
        &mut self,
        key: (i32, i32, i32),
        info: Info,
    ) -> Result<()> {
        let Some(entity) = self.sculk_entity(key) else {
            return Ok(());
        };
        let (kind, radius) = (entity.kind, entity.listener_radius());
        let origin = block_of(info.pos);
        let distance = (dist_sqr(origin, key) as f64).sqrt() as f32;
        let block = match kind {
            SculkKind::Sensor => "minecraft:sculk_sensor",
            SculkKind::Calibrated => "minecraft:calibrated_sculk_sensor",
            SculkKind::Shrieker => "minecraft:sculk_shrieker",
            SculkKind::Catalyst => "minecraft:sculk_catalyst",
        };
        let mut cancel = false;
        self.native_block_receive_game(
            self.common.dim.id(),
            [key.0, key.1, key.2],
            block,
            info.event,
            &mut cancel,
        );
        if cancel {
            return Ok(());
        }
        match kind {
            SculkKind::Sensor | SculkKind::Calibrated => {
                let state = self.common.world.block(key.0, key.1, key.2);
                if self
                    .common
                    .res
                    .behaviours()
                    .sculk_sensor_can_activate(state)
                {
                    let frequency = self.mob_maps().vibration_frequency(info.event);
                    if let Some(e) = self.sculk_entity(key) {
                        e.last_frequency = frequency;
                    }
                    let power = sculk::redstone_strength(distance, radius);
                    self.activate_sensor(key, info.source, power, frequency)?;
                }
            }
            SculkKind::Shrieker => {
                let by_player = info.owner.or(info.source.and_then(EventSource::actor));
                self.try_shriek(
                    key,
                    by_player == Some(Who::Player) || self.rides_player(by_player),
                )?;
            }
            SculkKind::Catalyst => {}
        }
        Ok(())
    }
    pub(in crate::game) fn rides_player(&self, who: Option<Who>) -> bool {
        matches!(who, Some(Who::Mob(id)) if self.mobs.iter().any(|m| m.id == id && m.rider.is_some()))
    }
    /// `SculkSensorBlock.activate`.
    pub(in crate::game::server_side) fn activate_sensor(
        &mut self,
        key: (i32, i32, i32),
        source: Option<EventSource>,
        power: i32,
        frequency: i32,
    ) -> Result<()> {
        let pos = BlockPos::new(key.0, key.1, key.2);
        self.with_level(|b, level, ticks| b.sculk_sensor_activate(level, ticks, pos, power));
        self.server_process_level_events()?;
        // tryResonateVibration.
        if (1..=15).contains(&frequency) {
            for direction in Direction::ALL {
                let n = direction.normal();
                let relative = (key.0 + n[0], key.1 + n[1], key.2 + n[2]);
                let state = self.common.world.block(relative.0, relative.1, relative.2);
                if self.state_is(&self.server.sculk.tables.resonator, state) {
                    let ctx = self.event_source_context(source, Some(state));
                    self.post_game_event(
                        sculk::RESONANCE[frequency as usize - 1],
                        center(relative),
                        ctx,
                    );
                    self.server_play_sound(
                        "minecraft:block.amethyst_block.resonate",
                        center(relative),
                        1.0,
                        sculk::resonance_pitch(frequency),
                    );
                }
            }
        }
        let ctx = self.event_source_context(source, None);
        self.post_game_event(sculk::TENDRILS_CLICKING, center(key), ctx);
        self.with_level(|b, level, ticks| b.sculk_sensor_click(level, ticks, pos));
        self.server_process_level_events()
    }
    /// `SculkSensorBlock.stepOn` and `SculkShriekerBlock.stepOn`: standing on
    /// them is heard (or seen) every tick.
    pub(in crate::game::server_side) fn sculk_step_on(&mut self) {
        self.sculk_step_on_player();
        let mut standing: Vec<(Who, [f64; 3])> = Vec::new();
        for m in &self.common.mobs {
            if m.alive() && m.ground.on_ground {
                standing.push((Who::Mob(m.id), [m.position.x, m.position.y, m.position.z]));
            }
        }
        self.sculk_stand_on(standing);
    }
    /// `stepOn` for the player in place: the host, or a guest swapped in
    /// (`guest_server_tick`), as a server has no player of its own.
    pub(in crate::game) fn sculk_step_on_player(&mut self) {
        if let Some(p) = self.common.player.as_ref()
            && p.ground.on_ground
        {
            let at = [p.position.x, p.position.y, p.position.z];
            self.sculk_stand_on(vec![(Who::Player, at)]);
        }
    }
    pub(in crate::game::server_side) fn sculk_stand_on(&mut self, standing: Vec<(Who, [f64; 3])>) {
        for (who, at) in standing {
            // getOnPosLegacy: 0.2 below the feet.
            let key = (
                at[0].floor() as i32,
                (at[1] - 0.2).floor() as i32,
                at[2].floor() as i32,
            );
            if !self.server.sculk.index.contains(&key) {
                continue;
            }
            let state = self.common.world.block(key.0, key.1, key.2);
            let is_warden = matches!(who, Who::Mob(id)
                if self.mobs.iter().any(|m| m.id == id && m.species() == Species::Warden));
            match self.kind_of_state(state.0) {
                KIND_SENSOR | KIND_CALIBRATED => {
                    if is_warden
                        || !self
                            .common
                            .res
                            .behaviours()
                            .sculk_sensor_can_activate(state)
                    {
                        continue;
                    }
                    let calibrated = self.kind_of_state(state.0) == KIND_CALIBRATED;
                    if calibrated {
                        let back = self.calibrated_signal(key);
                        if back != 0 && back != 1 {
                            continue;
                        }
                    }
                    let dest = center(key);
                    let d = [at[0] - dest[0], at[1] - dest[1], at[2] - dest[2]];
                    let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() as f32;
                    let time = self.common.world.game_time;
                    let source_uuid = self.vibration_uuid(Some(who));
                    if let Some(e) = self.sculk_entity(key) {
                        e.vibration.schedule(
                            Info {
                                event: "minecraft:step",
                                distance,
                                pos: at,
                                source: Some(who.into()),
                                owner: None,
                                source_uuid,
                                owner_uuid: None,
                            },
                            time,
                        );
                    }
                }
                KIND_SHRIEKER => {
                    let by_player = who == Who::Player || self.rides_player(Some(who));
                    if by_player {
                        let _ = self.try_shriek(key, true);
                    }
                }
                _ => {}
            }
        }
    }

    // ---- shriekers ----

    pub(in crate::game::server_side) fn can_respond(&self, state: BlockStateId) -> bool {
        use sunrise_world::food::Difficulty;
        self.state_property(state, "can_summon") == Some("true")
            && self.common.world.health_rules.difficulty != Difficulty::Peaceful
            && self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_WARDENS)
    }
    /// `SculkShriekerBlockEntity.tryShriek`.
    pub(in crate::game::server_side) fn try_shriek(
        &mut self,
        key: (i32, i32, i32),
        by_player: bool,
    ) -> Result<()> {
        if !by_player {
            return Ok(());
        }
        let state = self.common.world.block(key.0, key.1, key.2);
        if self.shrieker_shrieking(state) {
            return Ok(());
        }
        if let Some(e) = self.sculk_entity(key) {
            e.warning_level = 0;
        }
        if !self.can_respond(state) || self.try_to_warn(key) {
            self.shriek(key)?;
        }
        Ok(())
    }
    /// `WardenSpawnTracker.tryWarn` for the one player.
    pub(in crate::game::server_side) fn try_to_warn(&mut self, key: (i32, i32, i32)) -> bool {
        let c = center(key);
        let warden_nearby = self.common.mobs.iter().any(|m| {
            m.species() == Species::Warden
                && m.alive()
                && (m.position.x - c[0]).abs() < 24.0
                && (m.position.y + f64::from(m.height()) / 2.0 - c[1]).abs() < 24.0
                && (m.position.z - c[2]).abs() < 24.0
        });
        // The only player is the one who set it off.
        let mut trackers = [self.server.sculk.tracker];
        let level = WardenSpawnTracker::try_warn(&mut trackers, warden_nearby);
        self.server.sculk.tracker = trackers[0];
        match level {
            Some(level) => {
                if let Some(e) = self.sculk_entity(key) {
                    e.warning_level = level;
                }
                true
            }
            None => false,
        }
    }
    /// `SculkShriekerBlockEntity.shriek`.
    pub(in crate::game::server_side) fn shriek(&mut self, key: (i32, i32, i32)) -> Result<()> {
        let pos = BlockPos::new(key.0, key.1, key.2);
        self.with_level(|b, level, ticks| b.sculk_shrieker_shriek(level, ticks, pos));
        self.server_process_level_events()?;
        let ctx = self.player_context(None);
        self.post_game_event(sculk::SHRIEK, center(key), ctx);
        Ok(())
    }
    /// The shriek ended (or the block went away while shrieking).
    pub(in crate::game::server_side) fn shrieker_respond(
        &mut self,
        key: (i32, i32, i32),
        state: BlockStateId,
        warning: i32,
    ) {
        if !(self.can_respond(state) && warning > 0) {
            return;
        }
        if !self.try_summon_warden(key, warning) {
            self.warden_reply_sound(key, warning);
        }
        self.apply_darkness(center(key), 40.0);
    }
    /// A `shrieker_respond` block event.
    pub(in crate::game) fn shrieker_block_event(&mut self, pos: BlockPos, state: BlockStateId) {
        let key = (pos.x, pos.y, pos.z);
        let warning = self.sculk_entity(key).map_or(0, |e| e.warning_level);
        self.shrieker_respond(key, state, warning);
    }
    pub(in crate::game::server_side) fn warden_reply_sound(
        &mut self,
        key: (i32, i32, i32),
        warning: i32,
    ) {
        let sound = match warning {
            1 => "minecraft:entity.warden.nearby_close",
            2 => "minecraft:entity.warden.nearby_closer",
            3 => "minecraft:entity.warden.nearby_closest",
            4 => "minecraft:entity.warden.listening_angry",
            _ => return,
        };
        let r = &mut self.common.world.random;
        let mut offset = |v: i32| v + r.next_int_bound(21).unwrap_or(0) - 10;
        let at = [
            f64::from(offset(key.0)),
            f64::from(offset(key.1)),
            f64::from(offset(key.2)),
        ];
        self.server_play_sound(sound, at, 5.0, 1.0);
    }
    /// `SpawnUtil.moveToPossibleSpawnPosition` with `ON_TOP_OF_COLLIDER`.
    pub(in crate::game) fn spawn_position(
        &self,
        x: i32,
        y: i32,
        z: i32,
        range: i32,
    ) -> Option<(i32, i32, i32)> {
        use sunrise_sim::mobs::data;
        let flags = |b: BlockStateId| self.common.res.mob_data().state(b);
        let mut search = y;
        let mut above = self.common.world.block(x, search, z);
        for _ in (-range..=range).rev() {
            search -= 1;
            let current = self.common.world.block(x, search, z);
            let empty_above = flags(above).top == 0.0;
            let full_top = flags(current).flags & data::STURDY_TOP != 0;
            if empty_above && full_top {
                return Some((x, search + 1, z));
            }
            above = current;
        }
        None
    }
    /// `Warden.checkSpawnObstruction`: no liquid in its box, no living entity
    /// in the way (`isUnobstructed`), and no block collision.
    pub(in crate::game) fn warden_fits(&self, mob: &Mob) -> bool {
        let bounds = mob.bounds();
        let player_blocks = self
            .level_players()
            .iter()
            .any(|p| p.alive && !p.spectator && p.bounds.intersects(bounds));
        if player_blocks
            || self
                .common
                .mobs
                .iter()
                .any(|m| !m.removed && m.alive() && m.bounds().intersects(bounds))
        {
            return false;
        }
        let others: Vec<sunrise_sim::mobs::Seen> = Vec::new();
        let w = self.mob_world_base(&others);
        let min = bounds.min_position();
        let max = bounds.max_position();
        // Level.containsAnyLiquid: floor(min) up to ceil(max), exclusive.
        for x in min.x.floor() as i32..max.x.ceil() as i32 {
            for y in min.y.floor() as i32..max.y.ceil() as i32 {
                for z in min.z.floor() as i32..max.z.ceil() as i32 {
                    if w.fluid(x, y, z) != sunrise_items::blocks::Fluid::None {
                        return false;
                    }
                }
            }
        }
        w.no_collision(bounds)
    }

    /// `Warden.applyDarknessAround` / `MobEffectUtil.addEffectToPlayersAround`.
    pub(in crate::game) fn apply_darkness(&mut self, at: [f64; 3], radius: f64) {
        self.for_each_player(|g| g.darkness_on_player(at, radius));
    }
    pub(in crate::game::server_side) fn darkness_on_player(&mut self, at: [f64; 3], radius: f64) {
        if self.common.creative || self.common.spectator() {
            return;
        }
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        let d = [
            p.position.x - at[0],
            p.position.y - at[1],
            p.position.z - at[2],
        ];
        if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] >= radius * radius {
            return;
        }
        if let Some(active) = self.common.potions.effects.get("darkness")
            && active.duration > warden::DARKNESS_REAPPLY - 1
        {
            return;
        }
        // `new MobEffectInstance(DARKNESS, 260, 0, false, false)`: no
        // particles and no icon (#11385).
        let effect =
            sunrise_level::effects::EffectInstance::new("darkness", warden::DARKNESS_DURATION, 0)
                .with_flags(false, false);
        self.add_player_effect(effect);
    }

    // ---- catalysts ----

    /// `SculkCatalystBlockEntity.CatalystListener.handleGameEvent` for the
    /// death of a mob: the nearest catalyst within 8 blocks takes its
    /// experience. Returns whether it was consumed.
    pub(in crate::game) fn catalyst_takes_experience(
        &mut self,
        at: [f64; 3],
        experience: impl FnOnce(&mut Self) -> u32,
    ) -> bool {
        // GameEventDispatcher: catalysts are BY_DISTANCE listeners, handled
        // nearest first (exact distance to the block centre); a tie keeps the
        // order they were visited in: chunk x, chunk z, section y, then the
        // order they registered in. The first one takes the experience.
        let source = block_of(at);
        let mut candidates: Vec<(f64, (i32, i32, i32), usize, (i32, i32, i32))> = Vec::new();
        for (order, key) in self.server.sculk.index.iter().copied().enumerate() {
            if dist_sqr(source, key) > 64 {
                continue;
            }
            let is_catalyst = matches!(
                self.world.block_entities.get(&key),
                Some(BlockEntity::Sculk(e)) if e.kind == SculkKind::Catalyst
            );
            if is_catalyst {
                let c = center(key);
                let d = (at[0] - c[0]).powi(2) + (at[1] - c[1]).powi(2) + (at[2] - c[2]).powi(2);
                candidates.push((d, (key.0 >> 4, key.2 >> 4, key.1 >> 4), order, key));
            }
        }
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let nearest = candidates.first().map(|c| (c.3, ()));
        let Some((key, _)) = nearest else {
            return false;
        };
        let xp = experience(self);
        if xp > 0 {
            // sourcePosition.relative(UP, 0.5)
            let start = BlockPos::new(source[0], (at[1] + 0.5).floor() as i32, source[2]);
            if let Some(e) = self.sculk_entity(key) {
                e.spreader.add_cursors(start, xp as i32);
            }
        }
        self.catalyst_bloom(key);
        true
    }
    pub(in crate::game::server_side) fn catalyst_bloom(&mut self, key: (i32, i32, i32)) {
        let pos = BlockPos::new(key.0, key.1, key.2);
        self.with_level(|b, level, ticks| b.sculk_catalyst_bloom(level, ticks, pos));
        self.native_sculk_bloom(self.common.dim.id(), [key.0, key.1, key.2]);
        let _ = self.server_process_level_events();
        let c = center(key);
        self.server_send_particles(sunrise_sim::lan::server_effects::ServerQueuedParticle {
            dimension: self.common.dim.id(),
            particle: sunrise_sim::lan::server_effects::ServerParticle::simple(
                ParticleType::SculkSoul,
            ),
            at: [c[0], c[1] + 0.65, c[2]],
            spread: [0.2, 0.0, 0.2],
            speed: [0.0; 3],
            count: 2,
            randomization: 0,
            except: None,
        });
        let pitch = 0.6 + self.common.world.random.next_float() * 0.4;
        self.server_play_sound("minecraft:block.sculk_catalyst.bloom", c, 2.0, pitch);
    }

    /// `ServerLevel.sendParticles(new VibrationParticleOption(...))` from `VibrationSystem.Ticker`.
    pub(in crate::game::server_side) fn vibration_particle(
        &mut self,
        source: PositionSource,
        from: [f64; 3],
        _to: [f64; 3],
        ticks: i32,
    ) {
        self.server_send_particles(sunrise_sim::lan::server_effects::ServerQueuedParticle {
            dimension: self.common.dim.id(),
            particle: sunrise_sim::lan::server_effects::ServerParticle::vibration(source, ticks),
            at: from,
            spread: [0.0; 3],
            speed: [0.0; 3],
            count: 1,
            randomization: 0,
            except: None,
        });
    }
    /// A vibration travelling to a warden or allay (`MobEvent::Vibration`).
    pub(in crate::game) fn mob_vibration_particle(
        &mut self,
        source: sunrise_protocol::entity_data::PositionSource,
        from: [f64; 3],
        to: [f64; 3],
        ticks: i32,
    ) {
        let source =
            sunrise_gameplay_common::game::lan_particle_data::particle_position_source(&source);
        self.vibration_particle(source, from, to, ticks);
    }

    // ---- entity events ----

    /// `Projectile.shoot` / `onHit`: a projectile of `owner` at a position.
    pub(in crate::game) fn projectile_event(
        &mut self,
        event: &'static str,
        owner: Option<Who>,
        at: [f64; 3],
    ) {
        let mut ctx = Context::default();
        if event == "minecraft:projectile_shoot" {
            // The shooter is the source of the shot.
            ctx = self.who_context(owner, None);
        } else {
            ctx.owner = owner;
            ctx.player = owner == Some(Who::Player) || self.rides_player(owner);
        }
        self.post_game_event(event, at, ctx);
    }
    /// Retain the projectile as the landing event source and the
    /// shooter as its owner, even when the projectile is held out for ticking.
    pub(in crate::game) fn projectile_landed_event(
        &mut self,
        id: u32,
        owner: Option<Who>,
        at: [f64; 3],
    ) {
        let ctx = Context {
            source: (id != 0).then_some(EventSource::Entity(id)),
            owner,
            player: owner == Some(Who::Player) || self.rides_player(owner),
            ..Context::default()
        };
        self.post_game_event("minecraft:projectile_land", at, ctx);
    }
    /// `Projectile.onHit`: entity hits post at the ray point with no affected state;
    /// block hits post at the block center with the post-callback block state.
    pub(in crate::game) fn projectile_hit_landed_event(
        &mut self,
        id: u32,
        owner: Option<Who>,
        hit: sunrise_sim::projectiles::Hit,
    ) {
        match hit {
            sunrise_sim::projectiles::Hit::Entity { at, .. } => {
                self.projectile_landed_event(id, owner, [at.x, at.y, at.z]);
            }
            sunrise_sim::projectiles::Hit::Block { pos, .. } => {
                self.projectile_block_landed_event(id, owner, pos);
            }
            sunrise_sim::projectiles::Hit::None => {}
        }
    }
    /// `Projectile.onHit` on a block: `PROJECTILE_LAND` at block center with post-callback state.
    pub(in crate::game) fn projectile_block_landed_event(
        &mut self,
        id: u32,
        owner: Option<Who>,
        pos: [i32; 3],
    ) {
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        let ctx = Context {
            source: (id != 0).then_some(EventSource::Entity(id)),
            owner,
            player: owner == Some(Who::Player) || self.rides_player(owner),
            dampening_state: self.state_dampens(Some(state)),
            affected_state: Some(state),
            ..Context::default()
        };
        self.post_game_event("minecraft:projectile_land", at, ctx);
    }
    /// The state HIT_GROUND names for an entity of `bounds` landing at
    /// `position` (`Entity.checkFallDamage`): its supporting block's
    /// (`mainSupportingBlockPos`), else the block at `getOnPosLegacy`.
    pub(in crate::game) fn landing_state(&self, position: Vec3, bounds: Aabb) -> BlockStateId {
        let p = self.supporting_block(position, bounds).unwrap_or_else(|| {
            BlockPos::new(
                position.x.floor() as i32,
                (position.y - 0.2).floor() as i32,
                position.z.floor() as i32,
            )
        });
        self.common.world.block(p.x, p.y, p.z)
    }
    /// The player shot or threw something.
    pub(in crate::game::server_side) fn player_projectile_shot(&mut self) {
        if let Some(p) = self.common.player.as_ref() {
            let at = [
                p.position.x,
                p.position.y + f64::from(self.common.eye),
                p.position.z,
            ];
            self.projectile_event("minecraft:projectile_shoot", Some(Who::Player), at);
        }
    }

    /// `Entity.checkFallDamage` for item movement: retain a fall across an
    /// exact-height contact, without changing the saved fall-distance getter.
    pub(in crate::game::server_side) fn item_landing_event(
        &mut self,
        id: u32,
        previous: Vec3,
        position: Vec3,
        on_ground: bool,
    ) {
        let at = [position.x, position.y, position.z];
        if self.in_water_at(at) {
            self.server.sculk.falling_items.remove(&id);
            return;
        }
        if position.y < previous.y {
            self.server.sculk.falling_items.insert(id);
        }
        if on_ground && self.server.sculk.falling_items.remove(&id) {
            // EntityType.ITEM: 0.25 by 0.25.
            let bounds = Aabb::new(
                Vec3::new(position.x - 0.125, position.y, position.z - 0.125),
                Vec3::new(position.x + 0.125, position.y + 0.25, position.z + 0.125),
            );
            self.post_landing_event(position, bounds, false);
        }
    }

    /// `Entity.checkFallDamage` emits HIT_GROUND after the block's fallOn.
    pub(in crate::game) fn post_landing_event(
        &mut self,
        position: Vec3,
        bounds: Aabb,
        living: bool,
    ) {
        if self.common.options.execution_role.is_client() {
            return;
        }
        self.sculk_prepare();
        let state = self.landing_state(position, bounds);
        let mut context = self.who_context(None, Some(state));
        context.living = living;
        self.post_game_event(
            "minecraft:hit_ground",
            [position.x, position.y, position.z],
            context,
        );
    }

    /// `RedStoneOreBlock.stepOn`: standing on the ore lights it, unless the
    /// entity is stepping carefully (`attack_block` only lights an unlit ore).
    pub(in crate::game) fn step_on_redstone_ore(&mut self, at: [f64; 3], carefully: bool) {
        if carefully {
            return;
        }
        // getOnPosLegacy: 0.2 below the feet.
        let pos = [
            at[0].floor() as i32,
            (at[1] - 0.2).floor() as i32,
            at[2].floor() as i32,
        ];
        self.sculk_prepare();
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        if self.state_is(&self.server.sculk.tables.redstone_ore, state) {
            let _ = self.attack_block(pos);
        }
    }

    /// `RedStoneOreBlock.useItemOn`: a right-click lights the ore whatever the
    /// hand holds. PASS when the hand holds a block item, which goes on
    /// to be placed; SUCCESS otherwise.
    pub(in crate::game::server_side) fn use_redstone_ore(
        &mut self,
        pos: [i32; 3],
        held: Option<&sunrise_items::items::ItemStack>,
    ) -> Result<Option<mod_api::InteractionResult>> {
        use mod_api::InteractionResult as Answer;
        if !matches!(
            &*self.block_name(pos),
            "minecraft:redstone_ore" | "minecraft:deepslate_redstone_ore"
        ) {
            return Ok(None);
        }
        self.attack_block(pos)?;
        let block_item = held.is_some_and(|s| {
            self.common
                .res
                .items()
                .get(&s.item)
                .is_some_and(|i| i.block.is_some())
        });
        Ok(Some(if block_item {
            Answer::Pass
        } else {
            Answer::Success
        }))
    }

    /// Where the vehicle `id` stands: a mob, boat, minecart or cushion.
    pub(in crate::game::server_side) fn vehicle_position(&mut self, id: u32) -> Option<[f64; 3]> {
        let p = if let Some(i) = self.mob_slot(id) {
            self.common.mobs[i].position
        } else if let Some(b) = self.common.boats.iter().find(|b| b.id == id) {
            b.position
        } else if let Some(c) = self.common.carts.iter().find(|c| c.id == id) {
            c.position
        } else {
            self.common.cushions.iter().find(|c| c.id == id)?.position
        };
        Some([p.x, p.y, p.z])
    }
    /// `Entity.startRiding` and `removeVehicle`: `entity_mount` at the new
    /// vehicle and `entity_dismount` at the old one, derived from the vehicle
    /// the rider has now and the one it had last tick. `announce` is false
    /// for a rider first seen riding.
    pub(in crate::game) fn mount_events(&mut self, who: Who, vehicle: Option<u32>, announce: bool) {
        let last = self.server.sculk.mounts.get(&who).copied();
        // Riding nothing, as last tick: nothing to tell or keep.
        if vehicle.is_none() && last.is_none() {
            return;
        }
        let now = vehicle.and_then(|id| self.vehicle_position(id));
        if last.map(|l| l.0) != vehicle {
            if let Some((_, at)) = last {
                let ctx = self.who_context(Some(who), None);
                self.post_game_event("minecraft:entity_dismount", at, ctx);
            }
            if let (true, Some(at)) = (announce, now) {
                let ctx = self.who_context(Some(who), None);
                self.post_game_event("minecraft:entity_mount", at, ctx);
            }
        }
        // A vehicle that is gone keeps the spot it was last seen at.
        let kept = last.filter(|l| Some(l.0) == vehicle).map(|l| l.1);
        match (vehicle, now.or(kept)) {
            (Some(id), Some(at)) => {
                self.server.sculk.mounts.insert(who, (id, at));
            }
            _ => {
                self.server.sculk.mounts.remove(&who);
            }
        }
    }
    pub(in crate::game) fn in_water_at(&self, at: [f64; 3]) -> bool {
        let (x, y, z) = (
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        );
        self.common
            .res
            .facts()
            .get(self.common.world.block(x, y, z))
            .fluid
            == sunrise_items::blocks::Fluid::Water
    }
    /// `Entity.checkSupportingBlock`: the block under this box closest to the
    /// entity. Everything but the player works it out when it asks (Breeze ground dust too).
    pub(in crate::game) fn supporting_block(
        &self,
        position: Vec3,
        bounds: Aabb,
    ) -> Option<BlockPos> {
        let (min, max) = (bounds.min_position(), bounds.max_position());
        let foot = Aabb::new(
            Vec3::new(min.x, min.y - 1.0e-6, min.z),
            Vec3::new(max.x, min.y, max.z),
        );
        let mut scene = sunrise_level::collisions::SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: None,
        };
        sunrise_world::ground_support::find_supporting_block(
            &mut scene,
            sunrise_world::collision_context::CollisionContext::position(position.y),
            position,
            foot,
            1_000_000,
        )
        .ok()
        .flatten()
    }
    /// `Entity.mainSupportingBlockPos` of a player or mob on the ground.
    pub(in crate::game::server_side) fn main_support(&mut self, who: Who) -> Option<BlockPos> {
        match who {
            Who::Player => self.common.player.as_ref()?.ground.supporting_block,
            Who::Mob(id) => {
                let slot = self.mob_slot(id)?;
                self.common.mobs[slot].ground.supporting_block
            }
        }
    }
    pub(in crate::game) fn motion_steps_mut(&mut self, who: Who) -> Option<&mut Option<Steps>> {
        match who {
            Who::Player => Some(&mut self.server.sculk.steps),
            Who::Mob(id) => {
                let slot = self.mob_slot(id)?;
                Some(&mut self.common.mobs[slot].movement_emission)
            }
        }
    }

    /// `Entity.move`'s emission: footsteps, swimming, landing and splashes.
    pub(in crate::game) fn entity_motion_events(
        &mut self,
        who: Who,
        at: [f64; 3],
        on_ground: bool,
        in_water: bool,
        discrete: bool,
        flying: bool,
        warden: bool,
    ) {
        self.sculk_prepare();
        // Standing as it stood with no step due (a mob outside the simulation
        // distance, or one at rest): nothing below would change or be heard.
        let Some(slot) = self.motion_steps_mut(who) else {
            return;
        };
        if let Some(e) = slot.as_ref()
            && e.last == at
            && e.on_ground == on_ground
            && e.in_water == in_water
            && e.move_dist <= e.next_step
        {
            return;
        }
        let entry = slot.get_or_insert(Steps {
            move_dist: 0.0,
            next_step: 1.0,
            last: at,
            on_ground,
            in_water,
        });
        let previous = *entry;
        entry.last = at;
        entry.on_ground = on_ground;
        entry.in_water = in_water;
        let (dx, dy, dz) = (
            at[0] - previous.last[0],
            at[1] - previous.last[1],
            at[2] - previous.last[2],
        );
        // Entity.checkFallDamage: touching down after falling, with the state
        // of mainSupportingBlockPos (the block's state at getOnPosLegacy without one).
        if on_ground && (!previous.on_ground || dy < -1.0e-6) && dy < 0.0 && !in_water {
            let state = match self.main_support(who) {
                Some(support) => self.common.world.block(support.x, support.y, support.z),
                None => self.common.world.block(
                    at[0].floor() as i32,
                    (at[1] - 0.2).floor() as i32,
                    at[2].floor() as i32,
                ),
            };
            self.post_game_event_with("minecraft:hit_ground", at, |g| {
                g.who_context(Some(who), Some(state))
            });
        }
        // Entity.updateInWaterStateAndDoFluidPushing: entering water splashes.
        if in_water && !previous.in_water {
            self.post_entity_event("minecraft:splash", who);
        }
        // Player.getMovementEmission.
        if flying || discrete {
            return;
        }
        let horizontal = ((dx * dx + dz * dz).sqrt() * f64::from(0.6f32)) as f32;
        // Entity.getOnPos without a supporting block. Finding a mob's support
        // waits for a step that is due: this runs for every mob on every tick.
        let unsupported = self.common.world.block(
            at[0].floor() as i32,
            (at[1] - 1.0e-5).floor() as i32,
            at[2].floor() as i32,
        );
        let climbable = self.state_is(&self.server.sculk.tables.climbable, unsupported);
        let travelled = if climbable {
            ((dx * dx + dy * dy + dz * dz).sqrt() * f64::from(0.6f32)) as f32
        } else {
            horizontal
        };
        let Some(entry) = self.motion_steps_mut(who).and_then(Option::as_mut) else {
            return;
        };
        entry.move_dist += travelled;
        let move_dist = entry.move_dist;
        if move_dist <= entry.next_step {
            return;
        }
        // Entity.applyMovementEmissionAndPlaySound: the state at getOnPos, so a
        // carpet and not the block under it.
        let state = match on_ground.then(|| self.main_support(who)).flatten() {
            Some(support) => self.common.world.block(support.x, support.y, support.z),
            None => unsupported,
        };
        let air = self.common.res.facts().is_air(state);
        let Some(entry) = self.motion_steps_mut(who).and_then(Option::as_mut) else {
            return;
        };
        if !air {
            let steps_on_block = on_ground || climbable;
            if steps_on_block {
                entry.next_step = if warden {
                    // Warden.nextStep
                    move_dist + 0.55
                } else {
                    (move_dist as i32).wrapping_add(1) as f32
                };
                self.post_game_event_with("minecraft:step", at, |g| {
                    g.who_context(Some(who), Some(state))
                });
            } else if in_water {
                entry.next_step = (move_dist as i32).wrapping_add(1) as f32;
                self.post_entity_event("minecraft:swim", who);
            }
        }
    }
    /// `LivingEntity.die` with the catalyst's reach: a death event at the
    /// mob's position.
    pub(in crate::game) fn sculk_mob_died(&mut self, m: &Mob) {
        let ctx = self.context_of_mob(Some(m), m.id, None);
        self.post_game_event(
            "minecraft:entity_die",
            [m.position.x, m.position.y, m.position.z],
            ctx,
        );
    }
    /// `Entity.kill` for an entity that is no mob (`BlockAttachedEntity.kill`,
    /// `ArmorStand.kill`, `VehicleEntity.destroy`): removed, then
    /// `gameEvent(ENTITY_DIE, attributedTo != null ? attributedTo : this)` at
    /// the entity's position. `by_player`: the player's hit killed it, so the
    /// player is the source. Nonliving self sources retain their entity id
    /// even though they have no Who handle.
    pub(in crate::game) fn entity_killed_event(&mut self, at: [f64; 3], by_player: bool) {
        self.entity_killed_event_by(at, by_player.then_some(super::commands::Target::Player));
    }
    /// `BlockAttachedEntity.kill` / `ArmorStand.kill` with DamageSource.getEntity.
    pub(in crate::game) fn entity_killed_event_by(
        &mut self,
        at: [f64; 3],
        source: Option<super::commands::Target>,
    ) {
        self.entity_event_by("minecraft:entity_die", at, source);
    }
}

/// `Bee.isFlapping` (flying, every other tick) and `Bat.isFlapping`
/// (awake, every tenth tick). Bird distance thresholds run in mob movement.
pub(in crate::game) fn flaps(species: Species, resting: bool, on_ground: bool, age: u32) -> bool {
    match species {
        Species::Bee => !on_ground && age.is_multiple_of(2),
        Species::Bat => !resting && age.is_multiple_of(10),
        _ => false,
    }
}

pub(in crate::game) fn event_cause(
    source: Option<EventSource>,
    acting_player: Option<u32>,
) -> Option<u32> {
    match source {
        Some(EventSource::Player) => acting_player,
        Some(EventSource::Mob(id) | EventSource::Entity(id)) => Some(id),
        None => None,
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    #[test]
    fn game_event_cause_uses_acting_guest() {
        assert_eq!(event_cause(Some(Who::Player.into()), Some(42)), Some(42));
        assert_eq!(event_cause(Some(Who::Player.into()), None), None);
        assert_eq!(event_cause(Some(Who::Mob(7).into()), Some(42)), Some(7));
        assert_eq!(
            event_cause(Some(EventSource::Entity(99)), Some(42)),
            Some(99)
        );
        assert_eq!(event_cause(None, Some(42)), None);
    }

    #[test]
    #[ignore = "daily"]
    fn dropped_wool_dampens_its_own_vibrations() {
        use crate::game::native_entity_refs::tests::with_game;
        use std::sync::Arc;
        use sunrise_items::items::ItemStack;
        use sunrise_level::chunk::Chunk;

        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.sculk_prepare();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let sensor = game
                .pack
                .assets
                .registry
                .default_state("minecraft:sculk_sensor")
                .unwrap();
            game.world.set(1, 64, 1, sensor).unwrap();
            game.sculk_sync_index();
            game.spawn_item(
                ItemStack::new("minecraft:white_wool", 1),
                [3.5, 64.0, 1.5],
                [0.0; 3],
                0,
            );
            let id = game.dropped[0].id;
            let entity = game.entity(id).unwrap();
            assert!(game.entity_game_event(entity, "minecraft:step"));
            assert!(
                game.sculk_entity((1, 64, 1))
                    .unwrap()
                    .vibration
                    .selector
                    .current
                    .is_none()
            );
            game.dropped[0].stack = ItemStack::new("minecraft:stone", 1);
            assert!(game.entity_game_event(entity, "minecraft:step"));
            let (info, _) = game
                .sculk_entity((1, 64, 1))
                .unwrap()
                .vibration
                .selector
                .current
                .unwrap();
            assert_eq!(info.source, Some(EventSource::Entity(id)));
            game.sculk_entity((1, 64, 1))
                .unwrap()
                .vibration
                .selector
                .current = None;
            game.projectile_landed_event(123, Some(Who::Player), [3.5, 64.0, 1.5]);
            let (info, _) = game
                .sculk_entity((1, 64, 1))
                .unwrap()
                .vibration
                .selector
                .current
                .unwrap();
            assert_eq!(info.source, Some(EventSource::Entity(123)));
            assert_eq!(info.owner, Some(Who::Player));
        });
    }

    /// `vanilla_game_event` hears `GameEvent.Context.affectedState`: the state a context
    /// was made with reaches the listener, and a context made with none reads as None.
    #[test]
    #[ignore = "daily"]
    fn vanilla_game_event_carries_the_affected_state() {
        use crate::game::native_entity_refs::tests::with_game;
        use mod_api::{Hooks, ModEntry, VanillaGameEventEvent};
        use mod_platform::testing::with_mods;
        use std::sync::{Arc, Mutex};

        struct Heard(Arc<Mutex<Vec<Option<BlockStateId>>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Heard {
            fn vanilla_game_event(
                &self,
                _host: &mut dyn mod_api::ServerHost,
                event: &mut VanillaGameEventEvent<'_>,
            ) -> mod_api::Flow {
                self.0
                    .lock()
                    .unwrap()
                    .push(event.affected_state.map(|state| {
                        sunrise_gameplay_common::game::api_values::TryIntoEngine::try_into_engine(
                            &state,
                            &(),
                        )
                        .unwrap()
                    }));
                mod_api::Flow::Continue
            }
        }

        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new("heard", Box::new(Heard(heard.clone())))].into();
        with_mods(mods, || {
            with_game(|game| {
                let state = BlockStateId(3);
                let placed = game.player_context(Some(state));
                game.post_game_event("minecraft:block_place", [0.5, 64.0, 0.5], placed);
                let stepped = game.who_context(None, None);
                game.post_game_event("minecraft:step", [0.5, 64.0, 0.5], stepped);
            });
        });
        assert_eq!(*heard.lock().unwrap(), vec![Some(BlockStateId(3)), None]);
    }

    /// Retaining keeps the insertion order and the membership in step.
    #[test]
    fn the_index_retains_in_order() {
        let mut index = Index::default();
        for key in [(5, 0, 0), (40, 1, 0), (6, 2, 17), (-3, 3, 0), (41, 4, 0)] {
            index.insert(key);
        }
        index.retain(|p| p.0 >> 4 != 2 && p.2 >> 4 != 1);
        assert_eq!(index.order, [(5, 0, 0), (-3, 3, 0)]);
        assert!(index.contains(&(5, 0, 0)) && !index.contains(&(41, 4, 0)));
        assert_eq!(index.set.len(), 2);
        index.insert((41, 4, 0));
        assert_eq!(index.iter().last(), Some(&(41, 4, 0)));
    }

    /// The listeners near a point come out as filtering the whole index in
    /// order would give them, through inserts, removals and retains.
    #[test]
    fn listeners_within_a_radius_are_the_filtered_index_in_order() {
        let mut index = Index::default();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |n: i32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % (2 * n as u64 + 1)) as i32 - n
        };
        for step in 0..3000 {
            let key = (next(70), next(40), next(70));
            match step % 7 {
                0..=3 => index.insert(key),
                4..=5 => {
                    let picked = index
                        .order
                        .get(next(1000).unsigned_abs() as usize % index.order.len().max(1))
                        .copied();
                    if let Some(k) = picked {
                        index.remove(&k);
                    }
                }
                _ if step % 500 == 6 => index.retain(|p| (p.0 ^ p.2) & 3 != 0),
                _ => {}
            }
            let at = [next(80), next(50), next(80)];
            let radius = next(16).abs();
            let expected: Vec<_> = index
                .iter()
                .copied()
                .filter(|p| dist_sqr(at, *p) <= i64::from(radius * radius))
                .collect();
            assert_eq!(
                index.within(at, radius),
                expected,
                "step {step} at {at:?} r {radius}"
            );
        }
        assert!(index.cells.values().map(Vec::len).sum::<usize>() == index.order.len());
    }

    /// A bat flaps every tenth tick while it is not resting, whether it is on
    /// the ground or not; a resting bat never flaps and nothing else does.
    #[test]
    fn a_bat_flaps_every_tenth_tick_unless_it_is_resting() {
        assert!(flaps(Species::Bat, false, false, 0));
        assert!(flaps(Species::Bat, false, true, 10));
        assert!(!flaps(Species::Bat, false, false, 11));
        assert!(!flaps(Species::Bat, true, false, 20), "resting");
        assert!(flaps(Species::Bee, false, false, 4));
        assert!(!flaps(Species::Bee, false, true, 4), "a bee on the ground");
        assert!(!flaps(Species::Bat, false, false, 5));
        assert!(!flaps(Species::Cow, false, false, 0));
    }

    #[test]
    fn centres_and_blocks_agree() {
        assert_eq!(center((1, 2, 3)), [1.5, 2.5, 3.5]);
        assert_eq!(block_of([1.5, 2.5, -0.5]), [1, 2, -1]);
        assert_eq!(dist_sqr([0, 0, 0], (3, 4, 0)), 25);
    }
}

#[cfg(test)]
impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn test_sculk_mut(&mut self) -> &mut SculkState {
        &mut self.server.sculk
    }
}

use sunrise_gameplay_common::game::tables::{GameItems as _, WorldTablesExt as _};

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

#[cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(any(test, feature = "test-support"))]
pub mod item_use_event_fixture {
    type RecordedEvent = (&'static str, [f64; 3], Option<sunrise_sim::mobs::Who>);
    thread_local! {
        pub static EVENTS: std::cell::RefCell<Vec<RecordedEvent>> = const { std::cell::RefCell::new(Vec::new()) };
        pub static RECORDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    pub fn record_event(
        event: &'static str,
        pos: [f64; 3],
        source: Option<sunrise_sim::mobs::Who>,
    ) {
        if RECORDING.get() {
            EVENTS.with(|events| events.borrow_mut().push((event, pos, source)));
        }
    }
}
