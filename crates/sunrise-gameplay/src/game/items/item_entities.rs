//! Dropped items and experience orbs: their tick (`tick_items`, `tick_dropped_items`,
//! `tick_experience`), the stacks a destroyed container lets fall, and spawning and merging orbs.
use super::*;
use sunrise_gameplay_common::game::collision_source::Mover;
use sunrise_gameplay_common::game::creatures;
use sunrise_sim::{entities::ItemCells, join_seq::JoinList};

/// The lifted list's removal mask, pending departures and merge grid, indexed by its current
/// row order. `cells` is refilled lazily (reusing map and bucket capacities) only when a merge
/// pass actually queries it, so `give_items_back` compacts `items` before any callback runs
/// without rebuilding `ItemCells` per expiry.
pub(in crate::game::server_side) struct ItemPass {
    removed: Vec<bool>,
    cells: ItemCells,
    cells_dirty: bool,
    any_removed: bool,
    pending_left: Vec<u32>,
}

impl ItemPass {
    fn mark_removed(&mut self, index: usize, id: u32, at: Vec3, record_leave: bool) {
        if self.removed[index] {
            return;
        }
        self.removed[index] = true;
        self.any_removed = true;
        if !self.cells_dirty {
            self.cells.remove(index, at);
        }
        if record_leave {
            self.pending_left.push(id);
        }
    }
}

/// The items `give_items_back` takes again, where the item the loop was at now is (none once it
/// is gone), and the index the loop goes on from.
pub(in crate::game) struct GivenBack {
    pub(in crate::game) items: JoinList<ItemEntity>,
    pub(in crate::game) at: Option<usize>,
    pub(in crate::game) next: usize,
}

impl crate::game::ServerGame<'_> {
    pub fn tick_items(&mut self) {
        let mut round = self.item_update_round();
        self.tick_dropped_items(&mut round);
        sunrise_threads::work::clear_work();
        self.drain_entity_inside();
    }
    /// An item that joined during this tick's entity pass: it first ticks on
    /// the next one.
    pub fn joined_during_entity_pass(&self, id: u32) -> bool {
        id > self.server.ticked_entities_through || self.server.contact_drops.contains(&id)
    }
    pub fn tick_dropped_items(&mut self, round: &mut native_ticks::TickRound) {
        // Item update hooks can discard a drop before the pass starts.
        self.native_item_removals_flush(|_| {});
        let mut items = self.common.dropped.lift();
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut pass = ItemPass {
            removed: vec![false; items.len()],
            cells: ItemCells::default(),
            cells_dirty: true,
            any_removed: false,
            pending_left: Vec::new(),
        };
        // A LAN or server guest only shows the host's items.
        let merging = self.common.is_logical_server();
        // `Entity.checkBelowWorld`: below `Level.getMinY() - 64`.
        let void = f64::from(self.common.world.shape.min_y - 64);
        let mut obstacles =
            sunrise_sim::entities::ObstacleGrid::new(self.entity_collisions(Mover::Passive));
        let mut nearby = Vec::new();
        let mut bubble_tops = Vec::new();
        let mut destroyed = Vec::new();
        let mut moves = ahead::Moves::work_out(self, &items, &obstacles, round, |item| {
            item.stack.count > 0
                && &*item.stack.item != "minecraft:air"
                && !self.joined_during_entity_pass(item.id)
                && ticking(
                    item.position.x.floor() as i32,
                    item.position.z.floor() as i32,
                )
        });
        let item_type = sunrise_threads::work::entity_id(native_ticks::ITEM);
        // When entity tick hooks are subscribed, each item hears its Pre right before its own
        // tick (followed by `on_entity_item_update`) and its Post right after it, with the list
        // given back for the call.
        let told = round.told_listening();
        let tell_pre = round.pre();
        let tell_post = round.post();
        let update_hook =
            merging && mod_api::__private::has_part_hook(mod_api::HookId::OnEntityItemUpdate);
        let leave_hook = merging && mod_api::__private::listened(mod_api::HookId::EntityLeaveLevel);
        let mut next = 0;
        'items: while next < items.len() {
            let mut i = next;
            next += 1;
            sunrise_threads::work::set_entity(item_type);
            if pass.removed[i] {
                continue;
            }
            // `addFreshEntity` during the entity pass: the item ticks from the
            // next tick on (`EntityTickList`).
            if merging && self.joined_during_entity_pass(items[i].id) {
                continue;
            }
            if !ticking(
                items[i].position.x.floor() as i32,
                items[i].position.z.floor() as i32,
            ) {
                continue;
            }
            if tell_pre || (told && update_hook) {
                let (id, join) = (items[i].id, items[i].join);
                let given = self.give_items_back(items, &mut pass, i, round, |game, round| {
                    if game
                        .common
                        .dropped
                        .locate(join)
                        .is_some_and(|k| game.common.dropped[k].id == id)
                        && (!tell_pre || game.native_tick_pre_one(round, id, native_ticks::ITEM))
                    {
                        if tell_pre {
                            game.native_item_removals_flush(|_| {});
                        }
                        if update_hook
                            && let Some(k) = game
                                .common
                                .dropped
                                .locate(join)
                                .filter(|&k| game.common.dropped[k].id == id)
                        {
                            if game.item_update_halts(k) {
                                round.halt(id);
                            }
                            if tell_post {
                                game.native_item_removals_flush(|_| {});
                                if !game
                                    .common
                                    .dropped
                                    .locate(join)
                                    .is_some_and(|k| game.common.dropped[k].id == id)
                                {
                                    game.native_tick_post_one(round, id, native_ticks::ITEM);
                                }
                            }
                        }
                    }
                });
                (items, next) = (given.items, given.next);
                let Some(at) = given.at else {
                    continue;
                };
                i = at;
            }
            if round.canceled(items[i].id) {
                continue;
            }
            let mut expiry_discarded = false;
            let mut env_destroyed = None;
            'tick: {
                if round.halted(items[i].id) {
                    break 'tick;
                }
                // `ItemEntity.tick`: after `onEntityItemUpdate`, an empty stack discards itself.
                if items[i].stack.count == 0 || &*items[i].stack.item == "minecraft:air" {
                    let (id, at) = (items[i].id, items[i].position);
                    pass.mark_removed(i, id, at, merging && !self.common.persist.live.is_empty());
                    break 'tick;
                }
                self.tick_saved_invulnerable_time(items[i].id);
                let (no_physics, alive, discarded, entered) = items
                    .with_one_and_rest(i, |item, before, after| {
                        let sources = |id| {
                            before
                                .iter()
                                .chain(after)
                                .zip(pass.removed[..i].iter().chain(&pass.removed[i + 1..]))
                                .find(|(p, removed)| p.id == id && !**removed && p.health > 0)
                                .map(|(p, _)| {
                                    (
                                        sunrise_gameplay_common::game::commands_common::Target::Item(
                                            id,
                                        ),
                                        p.position.components(),
                                    )
                                })
                        };
                        let revision = self.common.world.revision;
                        let mut at_turn = |item: &mut ItemEntity| {
                            // `getEntityCollisions(entity, bounds.expandTowards(movement))`: the boxes around
                            // where the item can reach this tick.
                            obstacles.near(ahead::reach(item), &mut nearby);
                            // ItemEntity.tick's noPhysics (servers only): out of a block, a solid entity
                            // or the border, placed or closed on it, towards the closest open side (#1252).
                            let no_physics = merging
                                && item.inside_block(
                                    &self.common.world,
                                    self.common.collision,
                                    &nearby,
                                    Some(self.common.border.geometry()),
                                );
                            // `moveTowardsClosestSpace` draws the speed from the item's own random.
                            let escape_speed =
                                no_physics.then(|| item.random.next_float() * 0.2 + 0.1);
                            let mut entered = Vec::new();
                            let (alive, discarded) = self.tick_item_with_block_contacts(
                                item,
                                escape_speed,
                                &nearby,
                                false,
                                Some(&sources),
                                &mut |splash| entered.push(splash),
                            );
                            (no_physics, alive, discarded, entered)
                        };
                        match moves.take(i, item, revision, &mut at_turn) {
                            Some(moved) => moved,
                            None => at_turn(item),
                        }
                    })
                    .expect("the item tick has a current row");
                let item = &mut items[i];
                // Entity.doWaterSplashEffect: the server plays the sound (broadcast
                // to clients). Its addParticle is empty.
                for splash in entered {
                    if let sunrise_sim::entities::ItemSplash::Sound { at, volume, pitch } = splash {
                        self.server_play_sound(
                            "minecraft:entity.generic.splash",
                            at,
                            volume,
                            pitch,
                        );
                    }
                }
                if discarded {
                    if merging
                        && item.health <= 0
                        && mod_api::__private::item_listens(
                            mod_api::HookId::ItemDestroyed,
                            &item.stack.item,
                        )
                    {
                        let damage_type = self
                            .server
                            .item_destroyed_damage
                            .take_if(|(destroyed_id, _)| *destroyed_id == item.id)
                            .map(|(_, ty)| ty)
                            .unwrap_or_else(|| "minecraft:generic".to_owned());
                        if !pass.cells_dirty {
                            pass.cells.moved(i, item.previous, item.position);
                        }
                        env_destroyed = Some((item.id, item.position, damage_type));
                    } else {
                        self.server
                            .item_destroyed_damage
                            .take_if(|(destroyed_id, _)| *destroyed_id == item.id);
                        let (id, at) = (item.id, item.previous);
                        pass.mark_removed(
                            i,
                            id,
                            at,
                            merging && !self.common.persist.live.is_empty(),
                        );
                    }
                    break 'tick;
                }
                if merging && !no_physics {
                    self.item_landing_event(
                        item.id,
                        item.previous,
                        item.position,
                        item.ground.on_ground,
                    );
                    self.entity_powder_snow_particles(
                        item.previous,
                        item.position,
                        item.bounds(),
                        item.velocity,
                        false,
                    );
                }
                if !pass.cells_dirty {
                    pass.cells.moved(i, item.previous, item.position);
                }
                // Entity.handleOnAboveBubbleColumn: the server sends the particles of
                // the surface of a bubble column the item is at the top of.
                if merging {
                    bubble_tops.extend(sunrise_sim::motion::bubble_column_surfaces(
                        &self.common.world,
                        self.common.pack.assets.registry,
                        self.common.res.facts(),
                        self.common.collision,
                        item.bounds(),
                    ));
                }
                // ItemEntity.tick: merge every 2 ticks while moving between
                // blocks, every 40 otherwise, before the lifetime check.
                let block = |v: Vec3| (v.x.floor(), v.y.floor(), v.z.floor());
                let rate = if block(item.previous) == block(item.position) {
                    40
                } else {
                    2
                };
                if merging
                    && (alive || item.age <= ItemEntity::LIFETIME && item.position.y >= void)
                    && item.ticks.is_multiple_of(rate)
                {
                    if pass.cells_dirty {
                        pass.cells.rebuild_live(&items, &pass.removed);
                        pass.cells_dirty = false;
                    }
                    let max_stack = |stack: &ItemStack| self.stack_max_size(stack).max(1);
                    let record_leave = merging && !self.common.persist.live.is_empty();
                    // A merge may take this item or a neighbour out of the pass.
                    sunrise_sim::entities::merge_with_neighbours_recording(
                        &mut items,
                        &mut pass.removed,
                        &mut pass.cells,
                        i,
                        true,
                        &max_stack,
                        &mut |emptied_id| {
                            pass.any_removed = true;
                            if record_leave {
                                pass.pending_left.push(emptied_id);
                            }
                        },
                    );
                }
                let mut alive =
                    alive || (items[i].age < items[i].lifespan && items[i].position.y >= void);
                if merging
                    && !pass.removed[i]
                    && items[i].age >= items[i].lifespan
                    && items[i].position.y >= void
                {
                    // NeoForge `ItemEntity.tick` evaluates the left-hand `this.lifespan`
                    // before calling `EventHooks.onItemExpire(this)`.
                    let lifespan = items[i].lifespan;
                    let mut extra_life = 0;
                    if mod_api::__private::listened(mod_api::HookId::ItemExpire) {
                        let (id, join) = (items[i].id, items[i].join);
                        let given =
                            self.give_items_back(items, &mut pass, i, round, |game, round| {
                                if let Some(epoch) = game.common.native_lifecycle.epoch()
                                    && game
                                        .common
                                        .dropped
                                        .locate(join)
                                        .is_some_and(|k| game.common.dropped[k].id == id)
                                {
                                    let item = mod_api::ItemEntityRef::new(
                                        mod_api::EntityRef::new(epoch, id),
                                    );
                                    game.native_item_expire(item, &mut extra_life);
                                }
                                if tell_post {
                                    game.native_item_removals_flush(|_| {});
                                    if !game
                                        .common
                                        .dropped
                                        .locate(join)
                                        .is_some_and(|k| game.common.dropped[k].id == id)
                                    {
                                        game.native_tick_post_one(round, id, native_ticks::ITEM);
                                    }
                                }
                            });
                        (items, next) = (given.items, given.next);
                        let Some(at) = given.at else {
                            continue 'items;
                        };
                        i = at;
                    }
                    let item = &mut items[i];
                    // NeoForge `ItemEntity.tick`: Java int addition, then clamp to
                    // `0..=Short.MAX_VALUE - 1`, without rewinding `age`.
                    item.lifespan = lifespan
                        .wrapping_add(extra_life)
                        .clamp(0, ItemEntity::MAX_LIFESPAN);
                    alive =
                        item.age < item.lifespan && item.position.y >= void && item.stack.count > 0;
                    expiry_discarded = !alive;
                }
                if !alive || pass.removed[i] {
                    let (id, at) = (items[i].id, items[i].position);
                    pass.mark_removed(i, id, at, merging && !self.common.persist.live.is_empty());
                    break 'tick;
                }
                let item = &mut items[i];
                let from = item.previous;
                let to = item.position;
                if round.sections() {
                    let live = self.entity_section_step_visible(
                        item.id,
                        [from.x, from.y, from.z],
                        [to.x, to.y, to.z],
                    );
                    if !live {
                        let id = item.id;
                        pass.mark_removed(
                            i,
                            id,
                            to,
                            merging && !self.common.persist.live.is_empty(),
                        );
                        break 'tick;
                    }
                } else {
                    self.common.entity_section_entered(
                        item.id,
                        [from.x, from.y, from.z],
                        [to.x, to.y, to.z],
                    );
                }
            }
            if let Some((id, at, damage_type)) = env_destroyed {
                let mut drops = Vec::new();
                let given = self.give_items_back(items, &mut pass, i, round, |game, round| {
                    drops = if let Some(k) =
                        native_ticks::place(&game.common.dropped, i, |item| item.id == id)
                    {
                        game.destroy_item_entity_drops(k, &damage_type)
                    } else {
                        Vec::new()
                    };
                    if tell_post && !game.common.dropped.iter().any(|item| item.id == id) {
                        game.native_tick_post_one(round, id, native_ticks::ITEM);
                    }
                });
                (items, next) = (given.items, given.next);
                for stack in drops {
                    destroyed.push((stack, at));
                }
                if let Some(at_idx) = given.at {
                    i = at_idx;
                    let (id, at) = (items[i].id, items[i].position);
                    pass.mark_removed(i, id, at, merging && !self.common.persist.live.is_empty());
                } else {
                    continue;
                }
            }
            // `Entity.discard` calls `ServerLevel.EntityCallbacks.onTrackingEnd` within
            // this turn. Restore surviving drops for the leave hook even without a Post.
            if tell_post || (expiry_discarded && leave_hook && !self.common.persist.live.is_empty())
            {
                let id = items[i].id;
                let given = self.give_items_back(items, &mut pass, i, round, |game, round| {
                    if tell_post {
                        game.native_tick_post_one(round, id, native_ticks::ITEM);
                    }
                });
                (items, next) = (given.items, given.next);
            }
        }
        for pos in bubble_tops {
            self.bubble_column_particles(pos);
        }
        if pass.any_removed {
            let mut k = 0;
            items.retain(|_| {
                let keep = !pass.removed[k];
                k += 1;
                keep
            });
        }
        if !pass.pending_left.is_empty() {
            self.common.put_back_dropped(items);
            for id in pass.pending_left {
                self.native_entity_left(id);
            }
            self.native_item_removals_flush(|_| {});
            items = self.common.dropped.lift();
        }
        let Some(p) = self.common.player.as_ref() else {
            self.common.put_back_dropped(items);
            self.spawn_destroyed_contents(destroyed);
            return;
        };
        if p.health.dead_or_dying() || self.common.spectator() {
            self.common.put_back_dropped(items);
            self.spawn_destroyed_contents(destroyed);
            return;
        }
        let w = f64::from(p.dimensions.width) / 2.0;
        let (min, max) = (
            [
                p.position.x - w - 1.0,
                p.position.y - 0.5,
                p.position.z - w - 1.0,
            ],
            [
                p.position.x + w + 1.0,
                p.position.y + f64::from(p.dimensions.height) + 0.5,
                p.position.z + w + 1.0,
            ],
        );
        let feet = [p.position.x, p.position.y, p.position.z];
        let own = self.common.level.ensure_player_uuid();
        self.native_player_pickups(&mut items, (min, max), feet, own);
        self.common.put_back_dropped(items);
        self.spawn_destroyed_contents(destroyed);
    }
    /// Gives the lifted items back to the level for a hook that reads them live (a named item's
    /// own Pre or Post, at item `i`), then takes them again. The items gone this pass so far
    /// leave the list first, so the hook sees the level as it stands. When none left, none
    /// joined during the pass and the hook wrote no item (the list's write stamp didn't move),
    /// the list comes back as it went and `pass.removed` stays as it is; otherwise it is reset
    /// and `pass.cells` marked dirty for the list taken back. The loop goes on right after
    /// item `i`, or, when it is gone, after where it stood in the list's join order.
    pub(in crate::game::server_side) fn give_items_back(
        &mut self,
        mut items: JoinList<ItemEntity>,
        pass: &mut ItemPass,
        i: usize,
        round: &mut native_ticks::TickRound,
        call: impl FnOnce(&mut Self, &mut native_ticks::TickRound),
    ) -> GivenBack {
        let (id, join) = (items[i].id, items[i].join);
        let same_rows = self.common.dropped.is_empty();
        let compacted = pass.any_removed;
        if compacted {
            let mut k = 0;
            items.retain(|_| {
                let keep = !pass.removed[k];
                k += 1;
                keep
            });
            pass.any_removed = false;
            pass.cells_dirty = true;
        }
        let mut left = std::mem::take(&mut pass.pending_left);
        self.common.put_back_dropped(items);
        let restored = self.common.dropped.stamp();
        if !left.is_empty() {
            for &left_id in &left {
                self.native_entity_left(left_id);
            }
            self.native_item_removals_flush(|_| {});
            left.clear();
        }
        pass.pending_left = left;
        call(self, round);
        self.native_item_removals_flush(|_| {});
        let unchanged = same_rows && !compacted && self.common.dropped.stamp() == restored;
        let items = self.common.dropped.lift();
        if unchanged {
            return GivenBack {
                items,
                at: Some(i),
                next: i + 1,
            };
        }
        pass.removed.clear();
        pass.removed.resize(items.len(), false);
        pass.cells_dirty = true;
        let at = items.locate(join).filter(|&k| items[k].id == id);
        let next = match at {
            Some(at) => at + 1,
            None => items.partition_point(|item| item.join <= join),
        };
        GivenBack { items, at, next }
    }
    /// `ItemUtils.onContainerDestroyed`: spilled stacks use their own constructor
    /// motion (0.2 up and ±0.1 sideways), leaving the level random untouched.
    pub fn spawn_destroyed_contents(&mut self, destroyed: Vec<(ItemStack, Vec3)>) {
        for (stack, at) in destroyed {
            self.spawn_item_own(stack, [at.x, at.y, at.z], 0);
        }
    }
    /// `ExperienceOrb.award`: orbs of the largest sizes; each draws the id
    /// `tryMergeToExisting` compares from the level random, then moves with
    /// its own random.
    pub fn spawn_orbs(&mut self, at: [f64; 3], value: u32) {
        self.spawn_orbs_toward(at, [0.0; 3], value);
    }
    /// `ExperienceOrb.awardWithDirection`: `award` whose fresh orbs start
    /// `roughly` that way (`new ExperienceOrb(level, pos, roughly, value)`).
    pub(in crate::game::server_side) fn spawn_orbs_toward(
        &mut self,
        at: [f64; 3],
        roughly: [f64; 3],
        mut value: u32,
    ) {
        while value > 0 {
            let v = ExperienceOrb::split(value);
            value -= v;
            let merge_id = self.common.world.random.next_int_bound(40).unwrap_or(0);
            if !self.merge_new_orb(at, v, merge_id) {
                self.add_orb_toward(at, roughly, v);
            }
        }
    }
    /// `ExperienceOrb.tryMergeToExisting`: an orb of the same value within a
    /// 1 block box of `at`, whose id is a multiple of 40 from `merge_id`, takes
    /// the new one as one more in its count (and restarts its age); otherwise
    /// a fresh orb spawns.
    pub fn award_orb(&mut self, at: [f64; 3], value: u32, merge_id: i32) {
        if !self.merge_new_orb(at, value, merge_id) {
            self.add_orb(at, value);
        }
    }
    /// `tryMergeToExisting`: true when an orb took the new one.
    pub(in crate::game::server_side) fn merge_new_orb(
        &mut self,
        at: [f64; 3],
        value: u32,
        merge_id: i32,
    ) -> bool {
        let centre = Vec3::new(at[0], at[1], at[2]);
        let half = Vec3::new(0.5, 0.5, 0.5);
        let area = Aabb::new(centre.subtract(half), centre.add_vector(half));
        // A server's orbs are its own; a LAN or server guest only shows them.
        if self.common.is_logical_server() {
            for i in 0..self.common.orbs.len() {
                let o = &self.common.orbs[i];
                if !o.bounds().intersects(area) {
                    continue;
                }
                let vanilla = o.can_merge(merge_id as u32, value as i32);
                let (join, id, count, age) = (o.join, o.id, o.count, o.age);
                let orb_value = o.value;
                self.journal_orb_merge(join, (orb_value, count, age));
                if !self.orb_merges(id, orb_value, count, 0, value as i32, 1, vanilla) {
                    self.restore_orb(join, orb_value, count, age);
                    continue;
                }
                let Some(i) = self.common.orbs.locate(join) else {
                    continue;
                };
                let o = &mut self.common.orbs[i];
                // A mod's write in the merge hook is the merged orb; else one more in its count.
                if (o.value, o.count) == (orb_value, count) {
                    if count >= sunrise_sim::orb_limits::MAX_COUNT {
                        continue;
                    }
                    o.count += 1;
                }
                o.age = 0;
                return true;
            }
        }
        false
    }
    /// Puts back what a merge hook may have written to the orb `join`: a refused merge
    /// leaves both orbs as they were.
    pub(in crate::game::server_side) fn restore_orb(
        &mut self,
        join: sunrise_sim::join_seq::JoinSeq,
        value: i32,
        count: u32,
        age: i32,
    ) {
        if let Some(i) = self.common.orbs.locate(join) {
            let orb = &mut self.common.orbs[i];
            (orb.value, orb.count, orb.age) = (value, count, age);
        }
    }
    /// Whether orb `into` takes orb `from` (0: a spawning orb of `from_value`),
    /// given vanilla's `canMerge` answer: the mods may flip it, then cancel the merge.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game::server_side) fn orb_merges(
        &mut self,
        into: u32,
        into_value: i32,
        into_count: u32,
        from: u32,
        from_value: i32,
        from_count: u32,
        vanilla: bool,
    ) -> bool {
        let dimension = self.common.dim.id();
        let mut can_merge = vanilla;
        self.native_experience_orb_can_merge(
            dimension,
            into,
            from,
            into_value,
            from_value,
            &mut can_merge,
        );
        if !can_merge {
            return false;
        }
        let mut canceled = false;
        self.native_experience_orb_merge(mod_api::__private::ExperienceOrbMergeCall {
            dimension,
            into,
            from,
            into_value,
            into_count,
            from_value,
            from_count,
            canceled: &mut canceled,
        });
        !canceled
    }
    /// `ExperienceOrb.tick`'s scan before movement, once every twenty ticks.
    /// Hooks can change the resident list, so retain join identities, not indices.
    pub fn merge_orb(&mut self, i: usize) {
        if !self.common.orbs[i].ticks.is_multiple_of(20) {
            return;
        }
        let into_join = self.common.orbs[i].join;
        let area = self.common.orbs[i]
            .bounds()
            .inflate(Vec3::new(0.5, 0.5, 0.5));
        let candidates: Vec<_> = self
            .common
            .orbs
            .iter()
            .filter(|o| o.join != into_join && o.bounds().intersects(area))
            .map(|o| o.join)
            .collect();
        for from_join in candidates {
            let Some(i) = self.common.orbs.locate(into_join) else {
                return;
            };
            let Some(j) = self.common.orbs.locate(from_join) else {
                continue;
            };
            let (a, b) = (&self.common.orbs[i], &self.common.orbs[j]);
            let (into, into_value, into_count, into_age) = (a.id, a.value, a.count, a.age);
            let (from, from_value, from_count) = (b.id, b.value, b.count);
            let vanilla = b.can_merge(a.id, a.value);
            if !self.orb_merges(
                into, into_value, into_count, from, from_value, from_count, vanilla,
            ) {
                self.restore_orb(into_join, into_value, into_count, into_age);
                continue;
            }
            let Some(i) = self.common.orbs.locate(into_join) else {
                return;
            };
            let written =
                (self.common.orbs[i].value, self.common.orbs[i].count) != (into_value, into_count);
            let fresh = self.common.orbs.locate(from_join).filter(|&j| {
                let (a, b) = (&self.common.orbs[i], &self.common.orbs[j]);
                (b.value, b.count) == (from_value, from_count)
                    && b.bounds()
                        .intersects(a.bounds().inflate(Vec3::new(0.5, 0.5, 0.5)))
            });
            // A mod's write in the merge hook is the merged orb; else `into` adds the
            // source's count. A stale source, or a count past what a save holds, refuses
            // the merge and puts a written `into` back.
            let count = if written {
                Some(self.common.orbs[i].count)
            } else {
                into_count
                    .checked_add(from_count)
                    .filter(|&c| c <= sunrise_sim::orb_limits::MAX_COUNT)
            };
            let (Some(j), Some(count)) = (fresh, count) else {
                self.restore_orb(into_join, into_value, into_count, into_age);
                continue;
            };
            let age = self.common.orbs[j].age;
            self.common.orbs[i].count = count;
            self.common.orbs[i].age = self.common.orbs[i].age.min(age);
            self.common.orbs.remove(j);
        }
    }
    /// `Entity.unstuckIfPossible`, which `ExperienceOrb`'s constructor runs on an
    /// orb whose box is in a block: the free spot nearest its box's centre. An
    /// orb in open air keeps its position.
    pub(in crate::game::server_side) fn unstuck_orb(&self, at: [f64; 3]) -> [f64; 3] {
        let b = self.common.res.behaviours();
        let border = self.border_bounds();
        ExperienceOrb::unstuck(Vec3::new(at[0], at[1], at[2]), &border, |p| {
            b.collision_shape(self.common.world.block(p.x, p.y, p.z))
                .map(|s| s.moved(Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z))))
        })
        .map_or(at, |p| [p.x, p.y, p.z])
    }
    /// `new ExperienceOrb(level, pos, Vec3.ZERO, value)`: yaw and motion from
    /// the orb's own random.
    pub fn add_orb(&mut self, at: [f64; 3], value: u32) {
        self.add_orb_toward(at, [0.0; 3], value);
    }
    /// `new ExperienceOrb(level, pos, roughly, value)`: a random motion that
    /// points against `roughly` is turned round, and the orb starts half its
    /// size (0.25) along `roughly`, unstuck if it collides there.
    pub(in crate::game::server_side) fn add_orb_toward(
        &mut self,
        at: [f64; 3],
        roughly: [f64; 3],
        value: u32,
    ) {
        let len_sqr = roughly.iter().map(|v| v * v).sum::<f64>();
        // Vec3.normalize: a vector shorter than 1e-5 has no direction.
        let len = len_sqr.sqrt();
        let unit = if len < 1.0e-5 {
            [0.0; 3]
        } else {
            roughly.map(|v| v / len)
        };
        let at = self.unstuck_orb([0, 1, 2].map(|a| at[a] + unit[a] * 0.25));
        let mut r = random_source::entity();
        let _yaw = r.next_float();
        let p = Vec3::new(at[0], at[1], at[2]);
        let mut velocity = Vec3::new(
            (r.next_double() * 0.2 - 0.1) * 2.0,
            r.next_double() * 0.2 * 2.0,
            (r.next_double() * 0.2 - 0.1) * 2.0,
        );
        let dot = roughly[0] * velocity.x + roughly[1] * velocity.y + roughly[2] * velocity.z;
        if len_sqr > 0.0 && dot < 0.0 {
            velocity = Vec3::new(-velocity.x, -velocity.y, -velocity.z);
        }
        self.server.next_entity += 1;
        self.common.orbs.push(ExperienceOrb {
            size_answer: None,
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: self.server.next_entity,
            position: p,
            previous: p,
            velocity,
            // `getExperienceValue` sizes: 1 to 2477.
            value: value as i32,
            age: 0,
            ground: Default::default(),
            count: 1,
            ticks: 0,
            health: 5,
            air: 300,
            following: None,
            fire_ticks: 0,
        });
        // ServerLevel.addFreshEntity.
        self.native_joins_flush(false, 0);
    }
    /// The orbs' tick, but for the movement: that is `run_ordered_ticks`'s,
    /// in join order with the other kinds.
    pub fn tick_experience(&mut self) {
        if self.server.xp_delay > 0 {
            self.server.xp_delay -= 1;
        }
        let spectator = self.common.spectator();
        let player = self
            .common
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying() && !spectator);
        // Player.aiStep: standing, the pickup area is the player's box
        // inflated by (1, 0.5, 1); riding, the union with the vehicle's box
        // inflated by (1, 0, 1).
        let bounds = player.map(|p| {
            let w = f64::from(p.dimensions.width) / 2.0;
            let own = Aabb::new(
                Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
                Vec3::new(
                    p.position.x + w,
                    p.position.y + f64::from(p.dimensions.height),
                    p.position.z + w,
                ),
            );
            let area = match self.riding_box() {
                Some(vehicle) => own.union_bounds(vehicle).inflate(Vec3::new(1.0, 0.0, 1.0)),
                None => own.inflate(Vec3::new(1.0, 0.5, 1.0)),
            };
            (
                [
                    area.min_position().x,
                    area.min_position().y,
                    area.min_position().z,
                ],
                [
                    area.max_position().x,
                    area.max_position().y,
                    area.max_position().z,
                ],
            )
        });
        let mut orbs = self.common.orbs.lift();
        #[cfg(any(test, feature = "test-support"))]
        crate::game::server_side::entity_server::taken_pass_tests::after_take(self);
        let mut gained = 0i32;
        let mut taken = Vec::new();
        if let Some((min, max)) = bounds {
            orbs.retain_mut(|o| {
                let touching = o.position.x + 0.25 > min[0]
                    && o.position.x - 0.25 < max[0]
                    && o.position.y + 0.5 > min[1]
                    && o.position.y < max[1]
                    && o.position.z + 0.25 > min[2]
                    && o.position.z - 0.25 < max[2];
                if touching && self.server.xp_delay == 0 {
                    // ExperienceOrb.playerTouch: one orb every 2 ticks; a
                    // merged orb gives its value once per count.
                    self.server.xp_delay = 2;
                    gained = gained.saturating_add(o.value);
                    taken.push(o.clone());
                    o.count -= 1;
                    return o.count > 0;
                }
                true
            });
        }
        self.common
            .orbs
            .restore(orbs, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
        // ExperienceOrb.playerTouch repairs with the signed value, whatever it is.
        if !taken.is_empty() {
            gained = self.mending(gained);
        }
        for o in taken {
            // ClientPacketListener.handleTakeItemEntity: every orb taken sounds
            // at the orb, whether mending took its points or not.
            let mut r = random_source::create();
            let pitch = (r.next_float() - r.next_float()) * 0.35 + 0.9;
            let at = [o.position.x, o.position.y, o.position.z];
            self.server_play_sound("minecraft:entity.experience_orb.pickup", at, 0.1, pitch);
        }
        if gained > 0 {
            self.give_experience_points(gained);
        }
    }

    /// `ServerLevel.addFreshEntity` of the item `id` that `push_item_entity`
    /// made, with the stack it holds now.
    /// Whether the item entity is in the level after its join: false when a join listener kept
    /// it out (or no such item entity was made).
    pub(in crate::game) fn join_item_entity(&mut self, id: u32, position: [f64; 3]) -> bool {
        self.announce_item_entity(id, position, false)
    }

    pub(in crate::game) fn join_removal_item(&mut self, id: u32, position: [f64; 3]) {
        self.announce_item_entity(id, position, true);
    }

    pub(in crate::game) fn announce_item_entity(
        &mut self,
        id: u32,
        position: [f64; 3],
        in_removal: bool,
    ) -> bool {
        let Some((item, count)) = self
            .common
            .dropped
            .iter()
            .rev()
            .find(|d| d.id == id)
            .map(|d| (d.stack.item.clone(), d.stack.count as i32))
        else {
            return false;
        };
        if !mod_api::__private::mods().is_empty() {
            self.common.native_lifecycle.pending_item_spawns.push((
                id,
                item.to_string(),
                count,
                position,
            ));
            if in_removal {
                self.flush_removal_join(id)
            } else {
                self.native_joins_flush(false, id)
            }
        } else if self.native_joins_flush(false, id) {
            self.native_item_spawn(id, &item, count, position);
            true
        } else {
            false
        }
    }

    /// `Player.dropEquipment` for the player who died: unless keep_inventory
    /// is on, `destroyVanishingCursedItems`, then `Inventory.dropAll` from
    /// `at` (the eyes less 0.3): slots 0..36, armor feet to head (36..39),
    /// the off hand (40), each thrown in a random direction. The mods hear
    /// the drops (`native_living_drops`) before they join the level.
    pub fn drop_inventory(&mut self, at: [f64; 3], dying: &creatures::DyingDrops<'_>) {
        let mut stacks: Vec<ItemStack> = Vec::new();
        if !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::KEEP_INVENTORY)
        {
            stacks.extend(self.common.inventory.iter_mut().filter_map(Option::take));
            stacks.extend(self.common.armor.iter_mut().rev().filter_map(Option::take));
            stacks.extend(self.common.offhand.take());
            stacks.extend(self.common.equipment.take_all());
            stacks.retain(|s| s.enchantment_level("minecraft:vanishing_curse") == 0);
            self.apply_equipment();
        }
        let stacks = stacks.into_iter().map(|stack| (stack, at, 0)).collect();
        let mut random = random_source::entity();
        self.drop_death_items(dying, stacks, 40, |game| {
            Some(sunrise_sim::entities::drop_motion(
                &mut random,
                game.yaw,
                game.pitch,
                true,
            ))
        });
    }

    /// Item entity `i` leaves the level (`Entity.discard`); mods hear it.
    pub fn remove_dropped(&mut self, i: usize) -> ItemEntity {
        let e = self.common.dropped.remove(i);
        self.native_entity_left(e.id);
        e
    }

    /// Experience orb `i` leaves the level (`Entity.discard`); mods hear it.
    pub(in crate::game) fn remove_orb(&mut self, i: usize) -> ExperienceOrb {
        let o = self.common.orbs.remove(i);
        self.native_entity_left(o.id);
        o
    }

    /// The item entity `spawn_item` just added, unless a native mod kept it
    /// out of the level (`native_entity_join_level`).
    pub(in crate::game) fn just_spawned_item(&mut self) -> Option<&mut ItemEntity> {
        let id = self.server.next_entity;
        self.common.dropped.last_mut().filter(|d| d.id == id)
    }

    /// `new ItemEntity(level, x, y, z, stack, dx, dy, dz)`.
    pub fn spawn_item(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: [f64; 3],
        pickup_delay: i32,
    ) {
        self.add_item_entity(stack, position, Some(velocity), pickup_delay);
    }

    /// `new ItemEntity(level, x, y, z, stack)`: the item leaves with its own
    /// random's motion, which this returns.
    pub fn spawn_item_own(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        pickup_delay: i32,
    ) -> [f64; 3] {
        self.add_item_entity(stack, position, None, pickup_delay)
    }

    /// The dispenser constructs default motion before replacing it with the throw.
    pub(in crate::game) fn spawn_item_thrown(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: [f64; 3],
    ) {
        let random = sunrise_gameplay_common::game::spill_common::prepare_item_random(None);
        self.add_item_entity_prepared(stack, position, Some(velocity), 0, Some(random));
    }

    pub fn add_item_entity(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: Option<[f64; 3]>,
        pickup_delay: i32,
    ) -> [f64; 3] {
        self.add_item_entity_prepared(stack, position, velocity, pickup_delay, None)
    }

    /// Block.popResource
    pub(in crate::game) fn pop_resource(&mut self, pos: [i32; 3], stack: ItemStack) {
        let block_drops = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::BLOCK_DROPS);
        if let Some(at) = sunrise_gameplay_common::game::spill_common::admitted_pop_position(
            &mut self.common.world.random,
            pos,
            block_drops,
        ) {
            self.spawn_item_own(stack, at, 10);
        }
    }

    /// `Containers.dropItemStack`: a crafting remainder leaves from inside the
    /// block, split by the level random, and can be picked up at once. Unlike
    /// `Block.popResource` it has no `block_drops` test.
    pub(in crate::game::server_side) fn drop_item_stack_containers(
        &mut self,
        pos: [i32; 3],
        stack: ItemStack,
    ) {
        let count = stack.count;
        let at = [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])];
        for piece in sunrise_world::level::removal::drop_item_stack_at(
            &mut self.common.world.random,
            at,
            count,
        ) {
            self.spawn_item_thrown(
                ItemStack {
                    count: piece.count,
                    ..stack.clone()
                },
                piece.pos,
                piece.motion,
            );
        }
    }

    /// `Block.popResourceFromFace`: the loot leaves the clicked face with the
    /// face's own motion instead of the item's random one.
    pub(in crate::game::server_side) fn pop_resource_from_face(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        stack: ItemStack,
    ) {
        let (p, v) = sunrise_gameplay_common::game::spill_common::pop_position_from_face(
            &mut self.common.world.random,
            pos,
            face,
        );
        // The private overload checks the gamerule after the offsets are drawn.
        if !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::BLOCK_DROPS)
        {
            return;
        }
        self.spawn_item(stack, p, v, 10);
    }

    /// Player.drop: thrown from the eyes along the view direction. True when
    /// the selected stack's `on_dropped_by_player` Part kept it in the slot
    /// (`ServerPlayer.drop(boolean)`, as NeoForge asks it).
    pub fn drop_held_item(&mut self, all: bool) -> bool {
        if self.common.eye_position().is_none() {
            return false;
        }
        let spectator = self.common.spectator();
        if !spectator && self.native_dropped_by_player_event(all) {
            return true;
        }
        let Some(thrown) = sunrise_items::items::take_held_drop(
            &mut self.common.inventory[self.common.selected],
            all,
            spectator,
        ) else {
            return false;
        };
        self.player_drop(thrown);
        false
    }

    /// A guest's drop key. A drop the held item's Part refused sends the
    /// inventory back, undoing the removal the client predicted.
    pub fn guest_drop_held(&mut self, gi: usize, conn: u32, all: bool) {
        let kept = self.as_guest(conn, |g| g.drop_held_item(all)) == Some(true);
        if let Some(host) = self.server.lan.as_mut() {
            if kept {
                host.hub.send_inventory(gi);
            } else {
                host.hub.guests.update(gi, |g| g.told_selected());
            }
        }
    }

    /// `Player.createItemStackToDrop`: an item entity thrown from the eyes
    /// along the view direction. `thrown_from_hand` (`drop(stack, true)`, the
    /// drop key) names the player as the thrower; `drop(stack, false)`, for
    /// what no longer fits in the inventory, does not.
    pub fn throw_from_eyes(
        &mut self,
        thrown: ItemStack,
        eye: [f64; 3],
        randomly: bool,
        thrown_from_hand: bool,
    ) -> Option<u32> {
        let v = self.player_drop_motion(randomly);
        let at = [eye[0], eye[1] - f64::from(0.3f32), eye[2]];
        let uuid = thrown_from_hand.then(|| self.acting_player_uuid());
        // Only `drop(.., false)` tosses ask `item_toss`: a death's scatter (`randomly`) does not.
        let id = if randomly {
            self.spawn_item(thrown, at, v, 40);
            self.server.next_entity
        } else {
            self.toss_item(thrown, at, v, uuid)?
        };
        let item = self.common.dropped.iter_mut().rev().find(|d| d.id == id)?;
        item.thrower = uuid;
        Some(item.id)
    }

    /// `LivingEntity.createItemStackToDrop` draws the spread from the player's own random.
    pub(in crate::game) fn player_drop_motion(&mut self, randomly: bool) -> [f64; 3] {
        match self.common.player.as_mut() {
            Some(player) => sunrise_sim::entities::drop_motion(
                &mut player.random,
                self.common.yaw,
                self.common.pitch,
                randomly,
            ),
            None => sunrise_sim::entities::drop_motion(
                &mut random_source::entity(),
                self.common.yaw,
                self.common.pitch,
                randomly,
            ),
        }
    }

    /// `FurnaceResultSlot.checkTakeAchievements`: the player took from the output of the
    /// furnace at `pos`, which pops the experience of what it made at them
    /// (`awardUsedRecipesAndPopExperience`) and forgets it.
    #[must_use = "the used recipes are awarded to the extracting player"]
    pub(in crate::game::server_side) fn award_furnace_recipes(
        &mut self,
        pos: (i32, i32, i32),
        at: [f64; 3],
    ) -> Vec<String> {
        let (used, recipes) = match self.common.world.block_entities.get_mut(&pos) {
            Some(BlockEntity::Furnace(f)) => {
                let recipes_table = match self.server.world_tables.0.as_ref() {
                    Some(tables) => &tables.recipes,
                    None => self.common.res.recipes(),
                };
                let used = f.recipe_experience(recipes_table);
                let mut recipes: Vec<String> = f.recipes_used.keys().cloned().collect();
                recipes.sort();
                f.recipes_used.clear();
                (used, recipes)
            }
            _ => return Vec::new(),
        };
        for (count, experience) in used {
            let xp = sunrise_level::containers::create_experience(
                count,
                experience,
                &mut self.common.world.random,
            );
            self.spawn_orbs(at, xp);
        }
        recipes
    }
}

use sunrise_gameplay_common::game::collision_source::CollisionSourceGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt1 as _;

use sunrise_gameplay_common::game::entity_common::EntityCommonGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;
