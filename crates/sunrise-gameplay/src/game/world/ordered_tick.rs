//! PURPOSE: ticks experience orbs, primed TNT, falling blocks and the dragon
//! in the order they joined the level, between the other kinds' batches.
//! WHY: `ServerLevel.tick` ticks every entity in join order, one kind among
//! the rest; the mobile entities used to tick as fixed batches (orbs after the
//! mobs, falling blocks and TNT after the gateways), so a TNT that joined
//! before a boat blew up after the boat moved (#6509, tick order slice 2).
//! REFS: vanilla `ServerLevel.tick` (`entityTickList.forEach`),
//! `EntityTickList`; `join_seq.rs`; `tools/test_tick_order_parity.py`.
//! LOGS: none. READS-BY: `Game::tick_world`, `tick_mobs`.
//! ENDS-WHEN: every kind ticks from the join-order walk.

use super::Result;
use sunrise_core::geometry::Vec3;
use sunrise_core::random::RandomSource as _;
use sunrise_gameplay_common::game::collision_source::Mover;
use sunrise_level::random_source;
use sunrise_sim::join_seq::{JoinSeq, Joined};

/// Progress through this level tick's ordered entity turns.
#[derive(Default, Clone)]
pub struct OrderedTicks {
    /// Joins at or above this started after the tick began; they tick next tick.
    /// The general rows' pass (`general/tick.rs`) shares the same cutoff.
    pub born: JoinSeq,
    /// The last join number ticked, if any: later ones are still to come.
    pub done: Option<JoinSeq>,
    /// Joins below this have had their spawn packet; the ones above joined
    /// since (a summoned falling block, a lit TNT) and tick after it goes out.
    pub announced: JoinSeq,
    /// For each batch of another kind, in tick order (see [`Batch`]): the
    /// lowest join number among it and every batch after it.
    suffix_min: [JoinSeq; Batch::COUNT],
    /// The orbs' lava random, made on first use.
    lava_random: Option<sunrise_core::random::LegacyRandom>,
    /// Scratch schedule reused by each ordered pass in a level tick. The
    /// entity lists can change between passes, so this only retains capacity;
    /// entries are rebuilt from the live lists every time.
    pending_buf: Vec<(JoinSeq, Pending)>,
    /// Stand turns that belong after a shulker's turn, kept in join order.
    pub(in crate::game) armor_stands: std::collections::VecDeque<JoinSeq>,
    /// The falling blocks (id, join) that ended a tick still falling with a part listening to
    /// `falling_tick`, delivered after the pass.
    pub(in crate::game) falling_ticked: Vec<(u32, JoinSeq)>,
    /// Only consecutive falling ticks without mod callbacks may defer compaction.
    pub(in crate::game) defer_landings: bool,
    /// The terminal slots awaiting compaction at the end of that run.
    pub(in crate::game) landed: sunrise_core::fast_hash::FxHashSet<JoinSeq>,
    pub(in crate::game) landing_time: sunrise_level::spans::SpanTotal,
    /// Whether the entity tick hooks are heard this tick, and which types are told alone: the
    /// server's unsimulated entities of those types are told at their place in this order
    /// (game/modding/native_ticks.rs).
    pub told: super::native_ticks::Told,
}

/// The batches of the other kinds, in the order they tick.
#[derive(Clone, Copy)]
pub enum Batch {
    ArmorStands,
    Boats,
    Carts,
    Fishing,
    Tridents,
    Rockets,
    Items,
    Mobs,
    Projectiles,
    /// The general rows tick last of the kinds (`general/tick.rs`): a slot
    /// of its own keeps every earlier batch's `ordered_before` draining the
    /// orbs, TNT and falling blocks that joined before the earliest general
    /// row, so the mobile kinds keep their join order against them.
    General,
}

impl Batch {
    const COUNT: usize = 10;
}

#[derive(Clone, Copy)]
enum Pending {
    Orb,
    Tnt,
    Falling,
    Dragon,
    /// An entity the server keeps without simulating it (a display, a marker, an interaction),
    /// of a type a mod named: only told its tick.
    Other,
}

/// The lowest join number in a batch, or none when it is empty.
pub(in crate::game::server_side) fn first_join<T: Joined>(list: &[T]) -> Option<JoinSeq> {
    list.iter().map(Joined::join_seq).min()
}

impl crate::game::ServerGame<'_> {
    /// A level tick starts: what has joined so far ticks this tick.
    pub fn begin_ordered_ticks(&mut self) {
        let _span = sunrise_level::spans::guard("ordered_ticks");
        let pending_buf = std::mem::take(&mut self.server.ordered.pending_buf);
        let landed = std::mem::take(&mut self.server.ordered.landed);
        let mut armor_stands = std::mem::take(&mut self.server.ordered.armor_stands);
        armor_stands.clear();
        let mut falling_ticked = std::mem::take(&mut self.server.ordered.falling_ticked);
        falling_ticked.clear();
        let told = self.native_tick_told();
        let firsts = [
            first_join(&self.common.armor_stands),
            first_join(&self.common.boats),
            first_join(&self.common.carts),
            self.common.hook.as_ref().map(Joined::join_seq),
            first_join(&self.common.tridents),
            first_join(&self.common.rockets),
            first_join(&self.common.dropped),
            first_join(&self.common.mobs),
            first_join(&self.common.creatures.projectiles),
            first_join(&self.common.resident_entities.general.rows),
        ];
        // A batch ticks whole, so an entity of the three kinds goes after the
        // last batch that holds anything that joined before it.
        let mut suffix_min = [JoinSeq::MAX; Batch::COUNT];
        let mut lowest = JoinSeq::MAX;
        for (slot, first) in suffix_min.iter_mut().zip(firsts).rev() {
            lowest = lowest.min(first.unwrap_or(JoinSeq::MAX));
            *slot = lowest;
        }
        self.server.ordered = OrderedTicks {
            announced: self.server.ordered.announced,
            born: JoinSeq::upcoming(),
            done: None,
            suffix_min,
            lava_random: None,
            pending_buf,
            armor_stands,
            falling_ticked,
            defer_landings: false,
            landed,
            landing_time: Default::default(),
            told,
        };
    }

    /// Ticks each ordered entity that joined before `cut` and has not ticked yet.
    pub fn run_ordered_ticks(&mut self, cut: JoinSeq, late: bool) -> Result<()> {
        let _span = sunrise_level::spans::guard("ordered_ticks");
        let mut first_error = None;
        let mut followed = None;
        let mut events = false;
        let entity_ticking = self.entity_ticking_area();
        // Who ticks in this run, sorted once: entities that join meanwhile are
        // at or above `born`, so the set cannot grow, only lose the removed.
        let (done, born, announced) = (
            self.server.ordered.done,
            self.server.ordered.born,
            self.server.ordered.announced,
        );
        // The earlier passes never tick a TNT or falling block that joined since
        // the spawn packets (`right_pass`), so the late pass offers none that
        // ticked this tick: an orb that joined after one and ticked in an
        // earlier pass must not move `done` past it.
        let wanted = |j: JoinSeq| (late || done.is_none_or(|d| j > d)) && j < cut && j < born;
        let mut pending = std::mem::take(&mut self.server.ordered.pending_buf);
        pending.clear();
        let mut offer = |j: JoinSeq, kind: Pending| {
            // Orbs are announced after their tick, always; TNT and falling
            // blocks that joined since the last packets wait for them.
            let right_pass = match kind {
                Pending::Orb | Pending::Dragon | Pending::Other => !late,
                Pending::Tnt | Pending::Falling => late == (j >= announced),
            };
            if right_pass && wanted(j) {
                pending.push((j, kind));
            }
        };
        self.common
            .orbs
            .iter()
            .for_each(|e| offer(e.join, Pending::Orb));
        self.common
            .tnts
            .iter()
            .for_each(|e| offer(e.join, Pending::Tnt));
        self.common
            .falling
            .iter()
            .for_each(|e| offer(e.join, Pending::Falling));
        if let Some(dragon) = self.common.dragon.as_ref() {
            offer(dragon.join, Pending::Dragon);
        }
        // Only while a mod hears entity ticks: otherwise none is offered.
        let told = self.server.ordered.told;
        if told.listening() {
            self.server
                .others
                .0
                .iter()
                .filter(|o| super::native_ticks_server::other_ticks(&o.kind))
                .for_each(|o| offer(o.join, Pending::Other));
        }
        pending.sort_unstable_by_key(|&(j, _)| j);
        // EntityGetter.getEntityCollisions: the boxes that stop these moves (a
        // boat, a shulker, a happy ghast held still), gathered once for the
        // run and again after an explosion, which can move or remove them.
        let mut obstacles: Option<sunrise_sim::entities::ObstacleGrid> = None;
        let mut nearby = Vec::new();
        self.server.ordered.defer_landings = mod_api::__private::mods().is_empty();
        'entity: for (join, kind) in pending.drain(..) {
            // TNT, orbs and mod tick callbacks may observe the falling list.
            if !matches!(kind, Pending::Falling) {
                self.remove_landed_falling();
            }
            self.server.ordered.done = Some(join);
            // ServerLevel.tick: only entities in entity-ticking chunks tick.
            match kind {
                Pending::Dragon => {
                    let Some(dragon) = self.common.dragon.as_ref().filter(|d| d.join == join)
                    else {
                        continue;
                    };
                    let at = dragon.position;
                    if entity_ticking(at.x.floor() as i32, at.z.floor() as i32) {
                        self.tick_dragon();
                        if let Some(dragon) = self.common.dragon.as_ref().filter(|d| d.join == join)
                        {
                            self.entity_section_step(
                                dragon.id,
                                at.components(),
                                dragon.position.components(),
                            );
                        }
                        obstacles = None;
                    }
                }
                Pending::Tnt => {
                    let Some(i) = self.common.tnts.locate(join) else {
                        continue;
                    };
                    let at = self.common.tnts[i].position;
                    if entity_ticking(at.x.floor() as i32, at.z.floor() as i32)
                        && self.entity_position_accessible([at.x, at.y, at.z])
                    {
                        let t = &self.common.tnts[i];
                        let area = t.bounds().expand_towards(t.velocity);
                        obstacles
                            .get_or_insert_with(|| {
                                sunrise_sim::entities::ObstacleGrid::new(
                                    self.entity_collisions(Mover::Passive),
                                )
                            })
                            .near(area.inflate(Vec3::new(1.0, 1.0, 1.0)), &mut nearby);
                        let count = self.common.tnts.len();
                        if let Err(e) = self.tick_one_tnt(i, &nearby) {
                            first_error.get_or_insert(e);
                        }
                        if self.common.tnts.len() < count {
                            obstacles = None;
                        }
                    }
                }
                Pending::Falling => {
                    let Some(i) = self.common.falling.locate(join) else {
                        continue;
                    };
                    let at = self.common.falling[i].position;
                    if entity_ticking(at.x.floor() as i32, at.z.floor() as i32)
                        && self.entity_position_accessible([at.x, at.y, at.z])
                    {
                        events = true;
                        let f = &self.common.falling[i];
                        let area = f.bounds().expand_towards(f.velocity);
                        obstacles
                            .get_or_insert_with(|| {
                                sunrise_sim::entities::ObstacleGrid::new(
                                    self.entity_collisions(Mover::Passive),
                                )
                            })
                            .near(area.inflate(Vec3::new(1.0, 1.0, 1.0)), &mut nearby);
                        if let Err(e) = self.tick_one_falling(i, &nearby) {
                            first_error.get_or_insert(e);
                        }
                    }
                }
                Pending::Other => {
                    let Some(i) = self.server.others.0.locate(join) else {
                        continue;
                    };
                    let at = self.server.others.0[i].position;
                    if entity_ticking(at.x.floor() as i32, at.z.floor() as i32)
                        && self.entity_position_accessible([at.x, at.y, at.z])
                    {
                        self.native_other_tick_alone(told, i);
                    }
                }
                Pending::Orb => {
                    let Some(i) = self.common.orbs.locate(join) else {
                        continue;
                    };
                    let at = self.common.orbs[i].position;
                    if !entity_ticking(at.x.floor() as i32, at.z.floor() as i32)
                        || !self.entity_position_accessible([at.x, at.y, at.z])
                    {
                        continue;
                    }
                    // ExperienceOrb.tick runs scanForMerges before it moves.
                    // An orb absorbed here is gone before its own turn's
                    // find_join finds it.
                    if self.common.is_logical_server() {
                        self.merge_orb(i);
                    }
                    let Some(i) = self.common.orbs.locate(join) else {
                        continue;
                    };
                    let followed = followed.get_or_insert_with(|| self.orb_followed());
                    let target = self.orb_target(i, followed);
                    let Some(i) = self.common.orbs.locate(join) else {
                        continue;
                    };
                    // A block of slack around the move: gravity and the pull
                    // towards a player change it before it is made.
                    let o = &self.common.orbs[i];
                    let area = o.bounds().expand_towards(o.velocity);
                    obstacles
                        .get_or_insert_with(|| {
                            sunrise_sim::entities::ObstacleGrid::new(
                                self.entity_collisions(Mover::Passive),
                            )
                        })
                        .near(area.inflate(Vec3::new(1.0, 1.0, 1.0)), &mut nearby);
                    let orb_id = self.common.orbs[i].id;
                    let vanilla = self.saved_refuses_damage(orb_id, "minecraft:lava", false);
                    let server = self.common.is_logical_server();
                    let lava_random = &mut self.server.ordered.lava_random;
                    // ExperienceOrb's lava section uses the orb tick's own random, not level random.
                    let mut next_float = || {
                        lava_random
                            .get_or_insert_with(random_source::entity)
                            .next_float()
                    };
                    let side = server_game_side!(self);
                    let motion = sunrise_sim::motion::BodyMotion {
                        world: &self.common.world,
                        collision: self.common.collision,
                        facts: self.common.res.facts(),
                        blocks: Some(&self.common.res.motion().blocks),
                        context: &self.common.loop_context,
                        side: Some(&side),
                    };
                    let tick = self.common.orbs[i].move_before_fall(
                        motion,
                        target,
                        server,
                        &mut next_float,
                        Some(self.common.border.geometry()),
                        &self.common.res.motion().fluids,
                        &nearby,
                    );
                    if server {
                        self.orb_fall(i, tick.step, tick.movement.effect);
                    }
                    let Some(i) = self.common.orbs.locate(join) else {
                        continue;
                    };
                    let response = self.common.orbs[i]
                        .move_restitution(self.common.body_motion(&game_side!(self)), tick);
                    let entity = (orb_id, "minecraft:experience_orb");
                    let mut velocity = self.finish_body_restitution(
                        entity,
                        self.common.orbs[i].position,
                        response,
                    );
                    let Some(mut i) = self.common.orbs.locate(join) else {
                        continue;
                    };
                    let orb = &self.common.orbs[i];
                    let (footprint, ground) = ((orb.position, orb.bounds()), orb.ground);
                    self.common
                        .body_motion(&game_side!(self))
                        .apply_block_speed(orb.position, &mut velocity);
                    if !tick.burnt_out {
                        let writes = self.body_step_on(entity, footprint, &ground, &mut velocity);
                        for write in writes {
                            match write {
                                mod_api::EntityWrite::Hurt {
                                    damage_type,
                                    amount,
                                } => {
                                    let Some(name) =
                                        mod_api::__private::parts_installation::damage_type_name(
                                            damage_type,
                                        )
                                    else {
                                        continue;
                                    };
                                    let Some(kind) = self.common.health_resources.damage.id(name)
                                    else {
                                        continue;
                                    };
                                    let source = sunrise_sim::health::HurtSource {
                                        damage_type: name,
                                        causing: 0,
                                        direct: 0,
                                        position: None,
                                        direct_type: None,
                                    };
                                    let vanilla = self
                                        .resident_item_or_orb_immune(orb_id, entity.1, kind, false);
                                    let invulnerable = self.native_nonliving_invulnerable(
                                        orb_id, entity.1, source, vanilla,
                                    );
                                    let Some(current) = self.common.orbs.locate(join) else {
                                        continue 'entity;
                                    };
                                    i = current;
                                    if !invulnerable && self.common.orbs[i].hurt(amount) {
                                        self.common.orbs.remove(i);
                                        continue 'entity;
                                    }
                                }
                                mod_api::EntityWrite::Push([x, y, z]) => {
                                    velocity = velocity.add_vector(Vec3::new(x, y, z))
                                }
                                mod_api::EntityWrite::SetMovement([x, y, z]) => {
                                    velocity = Vec3::new(x, y, z)
                                }
                                mod_api::EntityWrite::ResetFallDistance => {
                                    self.common.orbs[i].fall_distance = 0.0
                                }
                                mod_api::EntityWrite::SetFireTicks(ticks) => {
                                    self.common.orbs[i].fire_ticks = ticks
                                }
                                _ => {}
                            }
                        }
                    }
                    let registry = self.common.pack.assets.registry;
                    let tags = server_world_items!(self);
                    let block_registries = &self.common.pack.block_registries;
                    let hooks = &self.common.damage_hooks;
                    let side = server_game_side!(self);
                    let o = &mut self.common.orbs[i];
                    o.velocity = velocity;
                    let pending_sounds = &mut self.server.pending_sounds;
                    let capture_active = self.server.audio_capture_active;
                    let lava_random = &mut self.server.ordered.lava_random;
                    let mut next_float = || {
                        lava_random
                            .get_or_insert_with(random_source::entity)
                            .next_float()
                    };
                    let stays = o.finish_tick(
                        &mut &mut self.common.world,
                        (
                            self.common.collision,
                            self.common.res.facts(),
                            &self.common.loop_context,
                            &side,
                        ),
                        &self.common.res.motion().fluids,
                        tick,
                        server,
                        sunrise_sim::entities::OrbLavaEffects {
                            next_float: &mut next_float,
                            on_sound: &mut |pos, pitch| {
                                if capture_active {
                                    pending_sounds.push(
                                        sunrise_game_data::sound_source::ServerSound::new(
                                            "minecraft:entity.generic.burn",
                                            pos,
                                            0.4,
                                            pitch,
                                        ),
                                    );
                                }
                            },
                            invulnerable: &mut |world| {
                                let source = sunrise_sim::health::HurtSource {
                                    damage_type: "minecraft:lava",
                                    causing: 0,
                                    direct: 0,
                                    position: None,
                                    direct_type: None,
                                };
                                sunrise_sim::health::DamageLevel::of(
                                    world,
                                    registry,
                                    Some(tags),
                                    Some(block_registries),
                                    hooks,
                                )
                                .no_push()
                                .invulnerability_check(
                                    sunrise_sim::health::Hurting::Entity(
                                        orb_id as i32,
                                        "minecraft:experience_orb",
                                        source,
                                    ),
                                    vanilla,
                                )
                            },
                        },
                    );
                    // PowderSnowBlock.entityInside during the move, before an
                    // orb that is done is removed.
                    let snow = (o.previous, o.position, o.bounds(), o.velocity);
                    self.entity_powder_snow_particles(snow.0, snow.1, snow.2, snow.3, false);
                    // Its move is filed at once, also for an orb about to go.
                    let (from, to) = (snow.0, snow.1);
                    self.entity_section_step(orb_id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
                    if !stays {
                        self.common.orbs.remove(i);
                    }
                }
            }
        }
        self.server.ordered.defer_landings = false;
        self.remove_landed_falling();
        std::mem::take(&mut self.server.ordered.landing_time).record("fall_finish");
        self.server.ordered.pending_buf = pending;
        self.deliver_falling_ticks();
        if events && let Err(e) = self.server_process_level_events() {
            first_error.get_or_insert(e);
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Close a run of falling blocks before the next entity can observe the list.
    fn remove_landed_falling(&mut self) {
        if !self.server.ordered.landed.is_empty() {
            let landed = &mut self.server.ordered.landed;
            self.common
                .falling
                .retain(|block| !landed.contains(&block.join));
            landed.clear();
        }
    }

    /// Before a batch ticks, run the ordered entities that joined before
    /// everything still to tick in the batches from this one on.
    pub(in crate::game) fn ordered_before(&mut self, batch: Batch) -> Result<()> {
        let cut = self.server.ordered.suffix_min[batch as usize];
        if cut == JoinSeq::MAX {
            return Ok(());
        }
        self.run_ordered_ticks(cut, false)
    }
}

impl crate::game::ServerGame<'_> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn orb_moved(&mut self, i: usize, step: sunrise_sim::entities::OrbMove) {
        let o = &self.common.orbs[i];
        let effect = self
            .common
            .body_motion(&game_side!(self))
            .legacy_contact(o.position, &o.ground);
        self.orb_fall(i, step, effect);
    }

    /// The authoritative impact is part of the move, before the captured state's restitution.
    fn orb_fall(
        &mut self,
        i: usize,
        step: sunrise_sim::entities::OrbMove,
        effect: sunrise_sim::motion::BodyContact,
    ) {
        let o = &self.common.orbs[i];
        let mut fall = self
            .common
            .vehicle_fall_reset(o.fall_distance, step.from, step.moved);
        if !step.in_water && step.moved.y < 0.0 {
            fall -= f64::from(step.moved.y as f32);
        }
        if o.ground.on_ground {
            let (id, position, bounds) = (o.id, o.position, o.bounds());
            self.common.orbs[i].fall_distance = 0.0;
            if fall > 0.0 {
                self.nonliving_fall_on(id, position, effect);
                self.post_landing_event(position, bounds, false);
            }
        } else {
            self.common.orbs[i].fall_distance = fall;
        }
    }
}

use sunrise_gameplay_common::game::collision_source::CollisionSourceGameExt1 as _;

use sunrise_gameplay_common::game::entity_common::EntityCommonGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::first_join;
    use sunrise_sim::join_seq::{JoinList, JoinSeq, Joined as _};

    #[test]
    fn first_join_uses_join_sequence_when_storage_is_reordered() {
        struct E {
            join: JoinSeq,
        }
        impl sunrise_sim::join_seq::Joined for E {
            fn join_seq(&self) -> JoinSeq {
                self.join
            }
            fn set_join_seq(&mut self, join: JoinSeq) {
                self.join = join;
            }
        }
        let mut list: JoinList<E> = JoinList::default();
        for _ in 0..3 {
            list.push(E {
                join: JoinSeq::UNJOINED,
            });
        }
        let first = list[0].join_seq();
        list.swap(0, 2);
        assert_eq!(first_join(&list), Some(first));
    }
}
