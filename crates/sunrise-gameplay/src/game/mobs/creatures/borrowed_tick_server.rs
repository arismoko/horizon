use super::*;
use std::collections::hash_map::Entry;
use sunrise_gameplay_common::game::{
    combat_common::PlayerCredit, entity_memory::OffLevel, native_ticks,
};
use sunrise_gameplay_common::lan::mob_players::{GuestViews, acts_on_player};

/// Route each player event to the player selected when it was emitted.
/// Untagged events retain the previous per-tick guest binding.
pub(in crate::game) fn route_player_events(
    events: &mut Vec<MobEvent>,
    from: usize,
    seen_guest: Option<u32>,
    guest_ids: &[(u32, u32)],
    guest_events: &mut Vec<(u32, u32, MobEvent)>,
    mob: u32,
) {
    let mut k = from;
    while k < events.len() {
        let (conn, tagged) = match &events[k] {
            MobEvent::ForPlayer { conn, .. } => (*conn, true),
            _ => (seen_guest, false),
        };
        if tagged {
            let MobEvent::ForPlayer { event, .. } = events.remove(k) else {
                unreachable!();
            };
            events.insert(k, *event);
        }
        // A hoglin broadcast carries the source's player, not the receiving
        // hoglin's independently selected player.
        if let MobEvent::HoglinAttackTarget {
            target: Who::Player,
            player,
            ..
        } = &mut events[k]
        {
            *player = conn;
        }
        // ShulkerBullet stores a target for later projectile ticks. Preserve
        // the selected guest's entity ID instead of the host-player sentinel.
        if let Some(conn) = conn
            && let MobEvent::Projectile(p) = &mut events[k]
            && let ProjectileKind::ShulkerBullet { target, .. } = &mut p.kind
            && *target == Some(Who::Player)
        {
            *target = guest_ids
                .iter()
                .find(|g| g.0 == conn)
                .map(|g| Who::Mob(g.1));
        }
        if let Some(conn) = conn
            && acts_on_player(&events[k])
        {
            guest_events.push((conn, mob, events.remove(k)));
        } else {
            k += 1;
        }
    }
}

/// Commit departures in the borrowed list, not when a tick merely proposes retention.
/// In particular an interrupted pass keeps the credit of every row it gives back.
pub(in crate::game) fn retain_mob_rows(
    mobs: &mut sunrise_sim::join_seq::JoinList<Mob>,
    player_credit: &mut std::collections::HashMap<u32, PlayerCredit>,
    mut keep: impl FnMut(&Mob) -> bool,
) {
    mobs.retain(|mob| {
        let kept = keep(mob);
        if !kept {
            player_credit.remove(&mob.id);
        }
        kept
    });
}

pub(in crate::game) struct BorrowedMobTick<'a> {
    pub(in crate::game) mobs: &'a mut sunrise_sim::join_seq::JoinList<Mob>,
    pub(in crate::game) snapshot: &'a mut Vec<Seen>,
    pub(in crate::game) passenger_objects: &'a [Seen],
    pub(in crate::game) blocked: &'a std::collections::HashSet<u32>,
    pub(in crate::game) events: &'a mut Vec<MobEvent>,
    pub(in crate::game) restocks: &'a mut Vec<sunrise_sim::mobs::villager_brain::RestockNotice>,
    pub(in crate::game) pushes: &'a mut sunrise_sim::mobs::PendingPushes,
    pub(in crate::game) dead: &'a mut Vec<Mob>,
    pub(in crate::game) died: &'a mut sunrise_level::fast_hash::FxHashSet<u32>,
    pub(in crate::game) ticked: &'a mut sunrise_level::fast_hash::FxHashSet<u32>,
    pub(in crate::game) guests: &'a mut GuestViews,
    pub(in crate::game) spare: &'a mut Option<PlayerView>,
    pub(in crate::game) bindings: &'a mut sunrise_level::fast_hash::FxHashMap<u32, u32>,
    pub(in crate::game) guest_ids: &'a mut Vec<(u32, u32)>,
    pub(in crate::game) guest_events: &'a mut Vec<(u32, u32, MobEvent)>,
    pub(in crate::game) player_credit: &'a mut std::collections::HashMap<u32, PlayerCredit>,
    pub(in crate::game) path_writes: &'a mut Vec<((i32, i32, i32), u32, u32)>,
    pub(in crate::game) gone: &'a mut Vec<(u32, Vec3, f64, f64)>,
    pub(in crate::game) lent: sunrise_sim::health::LentLevel,
    pub(in crate::game) laps: &'a mut sunrise_level::spans::Laps,
    pub(in crate::game) uuid: Option<[i32; 4]>,
    pub(in crate::game) level_id: &'static str,
    pub(in crate::game) round: &'a mut native_ticks::TickRound,
    pub(in crate::game) stand_result: &'a mut Result<()>,
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn retained_passenger_snapshot(&self, uuid: Option<[i32; 4]>) -> Vec<Seen> {
        if self.common.persist.off_level.is_empty() {
            Vec::new()
        } else {
            self.common
                .persist
                .off_level
                .values()
                .filter_map(OffLevel::as_mob)
                .map(|m| seen(m, uuid, self.items()))
                .collect()
        }
    }

    /// The killing blow of a mob as the death hooks name it: its damage
    /// type and the causing entity's id (0 for none).
    pub fn mob_damage_source(&self, m: &Mob) -> (&'static str, u32) {
        let (damage_type, attacker) = match m.last_damage {
            Some((d, _)) => (source_damage_type(d), d.attacker),
            None => ("minecraft:generic", None),
        };
        let attacker = match attacker {
            Some(Who::Mob(id)) => id,
            Some(Who::Player) => self
                .server
                .last_hurt_by_player
                .get(&m.id)
                .and_then(|credit| self.resolve_player_credit(*credit))
                .and_then(|conn| self.player_entity_id_by_connection(conn))
                .map_or(0, |id| id as u32),
            None => 0,
        };
        (damage_type, attacker)
    }
    pub(in crate::game::server_side) fn tick_borrowed_creatures(
        &mut self,
        state: BorrowedMobTick<'_>,
    ) -> (
        sunrise_sim::health::LentLevel,
        Option<Box<dyn std::any::Any + Send>>,
    ) {
        let passenger_objects = state.passenger_objects;
        let mut blocked = std::borrow::Cow::Borrowed(state.blocked);
        let mut mobs = state.mobs.take_owned_pass();
        let mut snapshot = std::mem::take(state.snapshot);
        let mut events = std::mem::take(state.events);
        let mut dead = std::mem::take(state.dead);
        let mut died = std::mem::take(state.died);
        let mut ticked = std::mem::take(state.ticked);
        let mut guests = std::mem::take(state.guests);
        let guest_indices = |guests: &GuestViews| {
            guests
                .iter()
                .enumerate()
                .map(|(slot, (conn, _))| (*conn, slot))
                .collect()
        };
        let mut guest_slots: sunrise_level::fast_hash::FxHashMap<u32, usize> =
            guest_indices(&guests);
        let mut spare = state.spare.take();
        let mut bindings = std::mem::take(state.bindings);
        let mut guest_ids = std::mem::take(state.guest_ids);
        let mut guest_events = std::mem::take(state.guest_events);
        let mut player_credit = std::mem::take(state.player_credit);
        let mut path_writes = std::mem::take(state.path_writes);
        let mut gone = std::mem::take(state.gone);
        let mut lent = state.lent;
        let laps = state.laps;
        let uuid = state.uuid;
        let level_id = state.level_id;
        let round = state.round;
        // Each mob's position updates in its tick, in order, filed and told before the next mob
        // is examined. The small record remains separate from `Mob` because teleports update
        // the same position field as ordinary movement.
        let section_scope = round
            .sections()
            .then(|| self.entity_section_scope())
            .flatten();
        let (section_table, live_section_observer) = if round.sections() {
            self.common.native_lifecycle.section_observer.begin(
                &mut self.common.native_lifecycle.entity_sections,
                section_scope,
            )
        } else {
            (None, None)
        };
        // Flush borrowed crossings before an owning callback can record more, or at pass end.
        let entries = self.common.is_logical_server();
        let mut section_entered =
            std::mem::take(&mut self.common.native_lifecycle.entity_sections.entered);
        let mut section_moves = match section_table.as_ref() {
            Some(sections) => std::mem::take(&mut sections.borrow_mut().ticked),
            None => std::mem::take(&mut self.common.native_lifecycle.entity_sections.ticked),
        };
        let capture_sections = section_table.is_some() || entries;
        sunrise_sim::mobs::living::section_writes::begin(capture_sections);
        let mut dispatch_panic = None;
        let mut pushes = std::mem::take(state.pushes);
        let original_len = snapshot.len();
        // A restored list can put new joins first. Keep its storage and view
        // slots intact, but walk its turns in join order when interleaving stands.
        let turn_order = (!self.server.ordered.armor_stands.is_empty()
            && !mobs.is_sorted_by_key(|mob| mob.join))
        .then(|| {
            let mut order: Vec<_> = (0..original_len).collect();
            order.sort_unstable_by_key(|&slot| mobs[slot].join);
            order
        });
        let mut keep = vec![true; original_len];
        let mut grid =
            sunrise_sim::mobs::seen_grid::SeenGrid::new(snapshot.iter().map(|s| s.bounds));
        // Snapshot slots stay stable when the owning list changes order.
        let mut positions: Vec<Option<usize>> = Vec::new();
        let mut index = sunrise_level::fast_hash::FxHashMap::<u32, u32>::default();
        let mut villager_slots = Vec::new();
        let mut solid_slots = Vec::new();
        for (i, s) in snapshot.iter().enumerate() {
            index.entry(s.id).or_insert(i as u32);
            if s.may_block_mobs() {
                solid_slots.push(i as u32);
            }
            if s.species == sunrise_sim::mobs::Species::Villager {
                villager_slots.push(i as u32);
            }
        }
        let mut cursor = 0;
        // Sounds the tick's hits queued, appended to the level's queue once the level is dropped.
        let mut hit_sounds: Vec<super::NativeSound> = Vec::new();
        use sunrise_sim::mobs::{
            ai::Pause,
            player_selection::{PlayerSelection, TickPlayer},
        };
        let mut resume = None;
        // Keep the suspended move out of the ordinary tick's result.
        let mut movement = None;
        let mut resume_entity = None;
        let mut resume_air_answer = None;
        let mut resume_selection: Option<PlayerSelection> = None;
        // An invalid removed vehicle is retried once at this rider's own
        // eligible entity tick, never from the later global positioning pass.
        let mut retried_rider = None;
        let mut resume_from = 0;
        let mut resume_sampled = None;
        let mut resume_guest = None;
        // The turn of the mob whose own Pre was just told (a type a mod named): it resumes
        // there with its checks done.
        let mut told_at: Option<usize> = None;
        let mut enchantment_tick_at = None;
        // Whether a mob still in the lifted list has left the pass (`keep` false) since the list
        // was last given back: a told-alone yield then takes it out before the call.
        let mut gone_rows = false;
        let mut resume_brain = None;
        // These belong to the finished mob, not the victim its event may hurt.
        // Keep them until the mob's event tail finishes, then file its section
        // writes and issue its Post while the rows are back in the owner.
        let mut tell_post: Option<(u32, std::sync::Arc<sunrise_sim::mobs::MobType>)> = None;
        loop {
            let mut section_hint = None;
            let mut pause = None;
            let mut sting = None;
            let mut brain_from = 0;
            let mut air_answer = None;
            let mut retry_rider = None;
            let mut removing_vehicle = None;
            let mut pending_effects: Vec<MobEvent> = Vec::new();
            let mut removal = None;
            let mut stand_cut = None;
            let mut lid_push = None;
            let mut paused_hurt = None;
            // A mob told alone hears its Pre right before its tick and its Post right after:
            // stop there, restore the list, tell it while the live rows are readable, and resume.
            let mut tell_pre: Option<(u32, std::sync::Arc<sunrise_sim::mobs::MobType>, usize)> =
                None;
            let mut tick_enchantments = None;
            let mut views_dirty = false;
            {
                let ticking = self.entity_ticking_area();
                let listed = self.entity_tick_listed_area();
                let players = self.level_players();
                let nearest = super::super::natural_spawning::NearestPlayers::new(&players);
                // All but the snapshot is the same for every mob of the tick, so
                // build it once: the player view and the teams scan every UUID.
                let mut leash_boats = std::mem::take(&mut self.server.leash_boats);
                leash_boats.clear();
                leash_boats.extend(
                    self.common
                        .boats
                        .iter()
                        .map(super::super::boat_leads::boat_state),
                );
                self.server.leash_boats = leash_boats;
                let mut base = self.mob_world_base(&[]);
                // Install the shared observer only on the per-mob view below; keeping the
                // reusable base empty lets its reference fields remain copyable between rows.
                base.section_writer = None;
                base.passenger_objects = passenger_objects;
                base.asks_baby_spawn = true;
                let mut level = sunrise_sim::health::DamageLevel::lent(
                    &self.common.world,
                    self.common.pack.assets.registry,
                    &mut lent,
                    Some(world_items!(self)),
                    Some(&self.common.pack.block_registries),
                    &self.common.damage_hooks,
                )
                .with_variants(world_variants!(self))
                .with_mob_hurt_handoff(&mut paused_hurt)
                .with_mob_uuids(&self.common.persist.uuids)
                .with_player_resolver(
                    crate::game::server_side::entity_server::player_resolver!(self),
                    &mut player_credit,
                )
                .with_effects(crate::game::server_side::entity_server::hit_effects_of!(
                    self,
                    None,
                    in hit_sounds
                ))
                .with_reads(crate::game::server_side::entity_server::hit_reads_of!(self));
                let path_changes = sunrise_sim::mobs::collision_changes(&base, &path_writes);
                laps.lap("creature_prepare");
                let mut cached_difficulty: Option<((i32, i32), spawning::LocalDifficulty)> = None;
                if let Some((from, hint, post)) = resume_brain.take() {
                    brain_from = from;
                    section_hint = hint;
                    tell_post = post;
                    let before_brain = mobs.stamp();
                    sting = sunrise_sim::mobs::apply_brain_events_until_hurt(
                        &mut mobs,
                        &mut level,
                        &mut events,
                        from,
                        &base.teams,
                        true,
                    );
                    views_dirty |= mobs.stamp() != before_brain;
                }
                while sting.is_none()
                    && section_hint.is_none()
                    && tell_post.is_none()
                    && cursor < original_len
                {
                    let slot = turn_order.as_ref().map_or(cursor, |order| order[cursor]);
                    let i = if positions.is_empty() {
                        slot
                    } else if let Some(i) = positions[slot] {
                        i
                    } else {
                        keep[slot] = false;
                        snapshot[slot].alive = false;
                        snapshot[slot].removed = true;
                        let equipment_finished = matches!(
                            resume.take(),
                            Some(Pause::Equipment { .. } | Pause::RegisterBehaviors { .. })
                        );
                        movement = None;
                        resume_selection = None;
                        resume_sampled = None;
                        cursor += 1;
                        // A discard in the equipment event ended a tick that had already started.
                        let mob = &snapshot[slot];
                        if equipment_finished && round.post() {
                            tell_post = Some((mob.id, std::sync::Arc::clone(&mob.kind)));
                            break;
                        }
                        continue;
                    };
                    let m = &mut mobs[i];
                    if self
                        .server
                        .ordered
                        .armor_stands
                        .front()
                        .is_some_and(|&join| join < m.join)
                    {
                        stand_cut = Some(m.join);
                        break;
                    }
                    let retried = retried_rider.take() == Some(m.id);
                    // Checks already ran before Pre or a pause in the tick.
                    let resuming = resume.is_some();
                    let checked = resuming || told_at == Some(cursor);
                    if blocked.contains(&m.id) || resuming && resume_entity != Some((m.id, m.join))
                    {
                        cursor += 1;
                        resume = None;
                        movement = None;
                        resume_selection = None;
                        resume_sampled = None;
                        continue;
                    }
                    if m.removed && !resuming {
                        keep[slot] = false;
                        gone_rows = true;
                        snapshot[slot].alive = false;
                        // `removed` is the view's authoritative gone: a dead body
                        // that is still in the level keeps resolving (#795).
                        snapshot[slot].removed = true;
                        cursor += 1;
                        continue;
                    }
                    let (bx, bz) = (m.position.x.floor() as i32, m.position.z.floor() as i32);
                    let seen_guest = if resuming {
                        resume_guest
                    } else if guests.is_empty() {
                        None
                    } else {
                        let bound = bindings.get(&m.id).copied();
                        let conn = super::super::lan_mobs::player_for(
                            m,
                            base.player.as_ref(),
                            &guests,
                            bound,
                        );
                        match conn {
                            Some(conn) => {
                                bindings.insert(m.id, conn);
                            }
                            None => {
                                bindings.remove(&m.id);
                            }
                        }
                        conn
                    };
                    if !checked && !base.world.loaded(bx, bz) {
                        keep[slot] = !m.hostile() || m.persistent;
                        gone_rows |= !keep[slot];
                        cursor += 1;
                        let id = m.id;
                        if !keep[slot]
                            && (m.extra.mount.is_some()
                                || self.has_seated_passengers_with_lent_mobs(id, &mobs))
                        {
                            removing_vehicle = Some(id);
                            break;
                        }
                        continue;
                    }
                    // ServerLevel.tick walks the entity tick list, which holds only the
                    // entities of chunks whose own status is entity-ticking: a mob in the
                    // loaded ring outside it fires no `mob_despawn`, makes no despawn draws
                    // and doesn't tick.
                    if !checked && !listed(bx, bz) {
                        cursor += 1;
                        continue;
                    }
                    let nearest_player = nearest
                        .nearest(m.position.x, m.position.y, m.position.z)
                        .map(|p| p.position);
                    // `MobDespawnEvent`, before `checkDespawn`'s own draws; one branch
                    // with no listener. Cleared below: the tick may not reach the check.
                    // The wither overrides `checkDespawn` without `super`, so the
                    // event, patched into `Mob.checkDespawn`, never fires for it;
                    // a mannequin is no `Mob` and never checks despawning.
                    if !checked
                        && !retried
                        && mod_api::__private::listened(mod_api::HookId::MobDespawn)
                        && !matches!(
                            m.species(),
                            sunrise_sim::mobs::Species::Wither
                                | sunrise_sim::mobs::Species::Mannequin
                        )
                        && let Some(epoch) = self.common.native_lifecycle.epoch()
                    {
                        let (id, kind, at) = (
                            m.id,
                            std::sync::Arc::clone(&m.kind),
                            [m.position.x, m.position.y, m.position.z],
                        );
                        let mut result = mod_api::Despawn::default();
                        let outcome =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                // The handle resolves through the tick's own view:
                                // the game has lent its mob list to this pass (#795).
                                let world = MobWorld {
                                    others: snapshot.as_slice(),
                                    others_grid: Some(&grid),
                                    others_index: Some(&index),
                                    others_villagers: Some(&villager_slots),
                                    others_solid: Some(&solid_slots),
                                    live_mobs: None,
                                    ..base.clone()
                                };
                                self.native_mob_despawn_in_tick(
                                    m,
                                    &world,
                                    self.common.dim.id(),
                                    mod_api::MobRef::new(mod_api::EntityRef::new(epoch, id)),
                                    &kind.id,
                                    at,
                                    &mut result,
                                );
                            }));
                        m.despawn = result;
                        if let Err(payload) = outcome {
                            m.despawn = mod_api::Despawn::default();
                            dispatch_panic = Some(payload);
                            break;
                        }
                    }
                    // ServerLevel.tick: every listed entity checks despawning, but
                    // only those within the simulation distance as well tick. The
                    // despawn check reads the nearest player and the difficulty, not
                    // the player the mob sees.
                    if !checked && !ticking(bx, bz) {
                        let w = MobWorld {
                            others: &snapshot,
                            others_grid: Some(&grid),
                            others_index: Some(&index),
                            others_villagers: Some(&villager_slots),
                            others_solid: Some(&solid_slots),
                            nearest_player,
                            player: None,
                            teams: std::sync::Arc::clone(&base.teams),
                            section_writer: None,
                            ..base
                        };
                        let kept = sunrise_sim::mobs::ai::despawn_check(m, &w);
                        m.despawn = mod_api::Despawn::default();
                        if !kept {
                            died.remove(&m.id);
                            snapshot[slot].alive = false;
                            snapshot[slot].removed = true;
                        }
                        keep[slot] = kept;
                        gone_rows |= !kept;
                        cursor += 1;
                        let id = m.id;
                        if !kept
                            && (m.extra.mount.is_some()
                                || self.has_seated_passengers_with_lent_mobs(id, &mobs))
                        {
                            removing_vehicle = Some(id);
                            break;
                        }
                        continue;
                    }
                    let controlling_player_movement = sunrise_sim::mobs::equine::controlled(m)
                        .then(|| {
                            base.player
                                .as_ref()
                                .filter(|p| p.riding_vehicle == Some(m.id))
                                .map(|p| p.velocity)
                                .or_else(|| {
                                    guests
                                        .iter()
                                        .find(|(_, p)| p.riding_vehicle == Some(m.id))
                                        .map(|(_, p)| p.velocity)
                                })
                        })
                        .flatten();
                    // A mob hears its own Pre immediately before its tick.
                    if !checked && round.pre() {
                        tell_pre = Some((m.id, std::sync::Arc::clone(&m.kind), i));
                        break;
                    }
                    // A mod canceled its tick: it stays as it is.
                    if !resuming && round.canceled(m.id) {
                        m.despawn = mod_api::Despawn::default();
                        cursor += 1;
                        continue;
                    }
                    if !resuming
                        && !retried
                        && m.extra.mount.is_some_and(|vehicle| {
                            self.common.persist.removed_vehicles.contains_key(&vehicle)
                        })
                    {
                        retry_rider = Some(m.id);
                        break;
                    }
                    if !resuming
                        && enchantment_tick_at != Some(slot)
                        && entries
                        && mod_api::EquipmentSlot::ALL.into_iter().any(|slot| {
                            sunrise_sim::mobs::finalize::equipment_slot(m, slot).is_some_and(
                                |stack| {
                                    world_stations!(self)
                                        .enchantments
                                        .has_component(&stack.project(), "minecraft:tick")
                                },
                            )
                        })
                    {
                        tick_enchantments = Some((m.id, m.join));
                        break;
                    }
                    // getCurrentDifficultyAt(blockPosition()): the mob's own chunk,
                    // not the local player's (a server thread has no player).
                    let chunk_key = (bx >> 4, bz >> 4);
                    let difficulty = match cached_difficulty {
                        Some((key, difficulty)) if key == chunk_key => difficulty,
                        _ => {
                            let difficulty = self.common.local_difficulty([
                                m.position.x,
                                m.position.y,
                                m.position.z,
                            ]);
                            cached_difficulty = Some((chunk_key, difficulty));
                            difficulty
                        }
                    };
                    sunrise_sim::mobs::apply_mob_pushes(m, &mut pushes);
                    sunrise_sim::mobs::blocks_changed(m, &path_changes);
                    if !resuming {
                        // LivingEntity.aiStep -> EntityEquipment.tick, before movement.
                        super::super::inventory_tick::equipment_inventory_tick(
                            &self.common.world,
                            self.common.res.facts(),
                            level_id,
                            &mut m.extra.equipment,
                        );
                    }
                    let was_removed = m.removed;
                    let from = if resuming { resume_from } else { events.len() };
                    // The guest's view is lent to the mob for its tick, not copied:
                    // the tick only reads it (`&MobWorld`), and it goes back to the
                    // guest's slot straight after. Its identity is captured with
                    // it, once: the player a goal resolves is the one the tick
                    // sees — a guest on a server, the game's own otherwise (#1077).
                    let lent = seen_guest.and_then(|conn| guest_slots.get(&conn).copied());
                    let active_player = match (seen_guest, lent) {
                        (None, _) => base.player.take(),
                        (Some(_), Some(k)) => spare
                            .take()
                            .map(|stand_in| std::mem::replace(&mut guests[k].1, stand_in)),
                        (Some(_), None) => None,
                    };
                    let active_identity = match seen_guest {
                        None => base.player_identity,
                        Some(conn) => self.guest_identity(conn),
                    };
                    // A listener can redirect to any real player in this level. Keep
                    // immutable views during the borrow so changing selection
                    // never invalidates a view a goal already holds. No allocation
                    // is made when nobody listens for target changes and no pet
                    // needs a player other than the one it sees.
                    let selection = if resuming {
                        let mut selection = resume_selection.take();
                        if matches!(
                            resume,
                            Some(
                                Pause::AfterDamage(_)
                                    | Pause::Equipment { .. }
                                    | Pause::RegisterBehaviors { .. }
                            )
                        ) && let Some(selection) = &mut selection
                        {
                            selection.refresh_views(|player| {
                                let (view, identity) = if player.connection == seen_guest {
                                    (active_player.as_ref(), active_identity)
                                } else if let Some(conn) = player.connection {
                                    (
                                        guest_slots.get(&conn).map(|&slot| &guests[slot].1),
                                        self.guest_identity(conn),
                                    )
                                } else {
                                    (base.player.as_ref(), base.player_identity)
                                };
                                view.filter(|_| {
                                    identity.is_some_and(|(_, id)| id == player.entity_id)
                                })
                                .cloned()
                            });
                        }
                        selection
                    } else if mod_api::__private::listened(mod_api::HookId::LivingChangeTarget)
                        || sunrise_sim::mobs::tame::reads_another_player(
                            m,
                            active_player.as_ref(),
                            seen_guest.and(base.player.as_ref()).into_iter().chain(
                                guests
                                    .iter()
                                    .enumerate()
                                    .filter(|(k, _)| Some(*k) != lent)
                                    .map(|(_, (_, view))| view),
                            ),
                            |id| {
                                base.player_identity.is_some_and(|(_, own)| own == id)
                                    || guests.iter().any(|(conn, _)| {
                                        self.guest_identity(*conn)
                                            .is_some_and(|(_, guest)| guest == id)
                                    })
                            },
                        )
                    {
                        let mut players = Vec::new();
                        if let (Some(view), Some((_, entity_id))) =
                            (&active_player, active_identity)
                        {
                            players.push(TickPlayer {
                                entity_id,
                                connection: seen_guest,
                                view: view.clone(),
                            });
                        }
                        if seen_guest.is_some()
                            && let (Some(view), Some((_, entity_id))) =
                                (&base.player, base.player_identity)
                        {
                            players.push(TickPlayer {
                                entity_id,
                                connection: None,
                                view: view.clone(),
                            });
                        }
                        for (index, (conn, view)) in guests.iter().enumerate() {
                            if Some(index) == lent {
                                continue;
                            }
                            if let Some((_, entity_id)) = self.guest_identity(*conn) {
                                players.push(TickPlayer {
                                    entity_id,
                                    connection: Some(*conn),
                                    view: view.clone(),
                                });
                            }
                        }
                        PlayerSelection::new(players, active_identity.map(|(_, id)| id))
                    } else {
                        None
                    };

                    let mut w = MobWorld {
                        others: &snapshot,
                        others_grid: Some(&grid),
                        others_index: Some(&index),
                        others_villagers: Some(&villager_slots),
                        others_solid: Some(&solid_slots),
                        difficulty,
                        nearest_player,
                        player: active_player,
                        player_identity: active_identity,
                        player_selection: selection.as_ref(),
                        teams: std::sync::Arc::clone(&base.teams),
                        section_writer: None,
                        ..base
                    };
                    w.section_writer = live_section_observer.as_ref().map(|observer| {
                        let epoch = observer.borrow().epoch();
                        let residency = self.entity_residency(mod_api::EntityRef::new(epoch, m.id));
                        observer.borrow_mut().set_residency(m.id, residency);
                        std::rc::Rc::clone(observer)
                            as sunrise_sim::mobs::living::section_writes::Observer
                    });
                    w.controlling_player_movement = controlling_player_movement;
                    ticked.insert(m.id);
                    // `sunrise-report`'s cost by mob type (tick_profile.rs).
                    let sampled = if resuming {
                        sunrise_threads::work::set_entity(sunrise_threads::work::entity_id(
                            &mobs[i].kind.id,
                        ));
                        resume_sampled.take()
                    } else {
                        super::super::tick_profile::start_mob(&mobs[i].kind)
                    };
                    let run_tick = || match resume.take() {
                        Some(Pause::Breathing { air, known_speed }) => {
                            sunrise_sim::mobs::riders::resume_after_breathing(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                air,
                                known_speed,
                                resume_air_answer.take(),
                                &mut events,
                                &mut movement,
                            )
                        }
                        Some(Pause::Drink { resume, .. }) => {
                            sunrise_sim::mobs::riders::resume_after_drink(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                resume,
                                &mut events,
                                &mut movement,
                            )
                        }
                        Some(Pause::Equipment { .. }) => {
                            sunrise_sim::mobs::riders::resume_after_equipment(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                &mut events,
                                &mut movement,
                            )
                        }
                        Some(Pause::RegisterBehaviors { stage }) => {
                            sunrise_sim::mobs::riders::resume_after_register_behaviors(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                stage,
                                &mut events,
                                &mut movement,
                            )
                        }
                        Some(Pause::AfterDamage(damage)) => {
                            sunrise_sim::mobs::riders::resume_after_damage(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                damage,
                                &mut events,
                                &mut movement,
                            )
                        }
                        Some(Pause::Breeding { resume, .. }) => {
                            sunrise_sim::mobs::riders::resume_after_birth(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                resume,
                                &mut events,
                                &mut movement,
                            )
                        }
                        Some(Pause::Trample(_) | Pause::FallToPassengers(_)) => {
                            sunrise_sim::mobs::riders::resume_movement(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                &mut movement,
                                &mut events,
                            )
                        }
                        Some(Pause::InsideEffects(paused)) => {
                            sunrise_sim::mobs::riders::resume_after_inside_effects(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                paused,
                                &mut events,
                            )
                        }
                        Some(Pause::Consumption { .. }) => {
                            sunrise_sim::mobs::riders::resume_after_consumption(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                &mut events,
                                &mut movement,
                            )
                        }
                        None => sunrise_sim::mobs::riders::tick_until_consumption(
                            &mut mobs,
                            i,
                            &w,
                            &mut level,
                            &mut events,
                            &mut movement,
                        ),
                    };
                    let progress = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run_tick));
                    // Screen updates must run before the next callback. Take them out of this
                    // villager's event tail now, before any continuation remembers an index in it.
                    if mobs[i].species() == Species::Villager {
                        for event in events.extract_if(from.., |event| {
                            matches!(event, MobEvent::VillagerRestockStep(_))
                        }) {
                            let MobEvent::VillagerRestockStep(restock) = event else {
                                unreachable!()
                            };
                            state.restocks.push(restock);
                        }
                    }
                    let progress = match progress {
                        Ok(progress) => progress,
                        Err(payload) => {
                            mobs[i].despawn = mod_api::Despawn::default();
                            if seen_guest.is_none() {
                                base.player = w.player;
                            } else if let Some(k) = lent {
                                spare = w.player.map(|v| std::mem::replace(&mut guests[k].1, v));
                            }
                            sunrise_sim::mobs::take_mob_pushes(&mut events, from, &mut pushes);
                            route_player_events(
                                &mut events,
                                from,
                                seen_guest,
                                &guest_ids,
                                &mut guest_events,
                                mobs[i].id,
                            );
                            dispatch_panic = Some(payload);
                            break;
                        }
                    };
                    let kept = match progress {
                        sunrise_sim::mobs::ai::TickProgress::Complete(kept) => {
                            movement = None;
                            kept
                        }
                        sunrise_sim::mobs::ai::TickProgress::Paused(paused) => {
                            if matches!(paused, Pause::Breeding { .. }) {
                                // The birth callback sees the partner after earlier brain events.
                                sunrise_sim::mobs::apply_brain_events(
                                    &mut mobs,
                                    &mut level,
                                    &mut events,
                                    from,
                                    &base.teams,
                                );
                            }
                            // Return every player loan before ending the world borrow.
                            if seen_guest.is_none() {
                                base.player = w.player;
                            } else if let Some(k) = lent
                                && let Some(player) = w.player
                            {
                                guests[k].1 = player;
                            }
                            resume_from = from;
                            resume_sampled = sampled;
                            resume_guest = seen_guest;
                            resume_selection = selection;
                            resume_entity = Some((mobs[i].id, mobs[i].join));
                            pause = Some(paused);
                            sunrise_threads::work::clear_work();
                            break;
                        }
                    };
                    // Shulker.onPeekAmountChange shoves with the shulker's new
                    // bounds, before the next stand or mob starts its own tick.
                    if mobs[i].species() == Species::Shulker
                        && let Some(offset) = events[from..]
                            .iter()
                            .position(|event| matches!(event, MobEvent::LidPush { .. }))
                        && let MobEvent::LidPush {
                            shulker,
                            area,
                            delta,
                        } = events.remove(from + offset)
                    {
                        lid_push = Some((shulker, area, delta));
                    }
                    super::super::tick_profile::end_mob(level_id, sampled);
                    let m = &mut mobs[i];
                    // Its `xo` (`previous`) is where its tick began, unless a teleport in the
                    // tick (`snapTo`) moved it along: that teleport, then where the tick left
                    // it, are two position updates (`EntityInLevelCallback.onMove` for each).
                    if round.sections() || entries {
                        let start = section_moves.len();
                        let captured = sunrise_sim::mobs::living::section_writes::drain_for(
                            m.id,
                            &mut section_moves,
                        );
                        if kept && !m.removed {
                            if captured == 0 && m.previous != m.position {
                                // A position changed before the lifted tick (or a raw test fixture)
                                // still has the old filed section as its first write.
                                if round.sections() {
                                    section_moves.push(
                                        sunrise_sim::mobs::living::section_writes::PositionWrite::baseline(
                                            m.id, m.join, m.previous,
                                        ),
                                    );
                                }
                                section_moves.push(
                                    sunrise_sim::mobs::living::section_writes::PositionWrite::captured(
                                        m.id, m.join, m.previous, m.position,
                                    ),
                                );
                            }
                            if section_scope.is_some()
                                && section_moves[start..].iter().any(|write| !write.delivered)
                            {
                                section_hint = Some(i);
                            }
                        }
                        // One integer compare per captured write; a crossing is one push.
                        if entries {
                            for write in &section_moves[start..] {
                                if let Some(from) = write.from
                                    && sunrise_gameplay_common::game::entity_sections::crossed(
                                        [from.x, from.y, from.z],
                                        [write.to.x, write.to.y, write.to.z],
                                    )
                                {
                                    section_entered.push((write.id, write.to.components()));
                                }
                            }
                        }
                    }
                    if let Some(selection) = &selection
                        && selection.has_switched()
                        && let Some(player) = selection.selected()
                    {
                        if let Some(conn) = player.connection {
                            bindings.insert(m.id, conn);
                        } else {
                            bindings.remove(&m.id);
                        }
                    }
                    m.despawn = mod_api::Despawn::default();
                    if seen_guest.is_none() {
                        base.player = w.player;
                    } else if let Some(k) = lent {
                        spare = w.player.map(|v| std::mem::replace(&mut guests[k].1, v));
                    }
                    sunrise_sim::mobs::take_mob_pushes(&mut events, from, &mut pushes);
                    if !kept && !m.removed && !m.alive() {
                        let split = m.death_time >= 20
                            && m.extra.size > 1
                            && matches!(
                                m.species(),
                                Species::Slime | Species::MagmaCube | Species::SulfurCube
                            );
                        if m.death_time >= 20 {
                            let (width, height) = (f64::from(m.width()), f64::from(m.height()));
                            gone.push((m.id, m.position, width, height));
                        }
                        if split || !m.extra.effects.is_empty() {
                            removal = Some((m.join, split));
                        }
                    }
                    route_player_events(
                        &mut events,
                        from,
                        seen_guest,
                        &guest_ids,
                        &mut guest_events,
                        m.id,
                    );
                    // Killed during its own tick (drowning, burning, falling...).
                    if !m.alive() && !m.removed && died.insert(m.id) {
                        dead.push(m.clone());
                    }
                    if !kept {
                        died.remove(&m.id);
                    }
                    keep[slot] = kept;
                    gone_rows |= !kept;
                    // Its own Post, right after its tick, also when it died or went in it.
                    if round.post() {
                        tell_post = Some((m.id, std::sync::Arc::clone(&m.kind)));
                    }
                    // ServerLevel ticks entities one after another: the ones
                    // after this mob see it as it is now.
                    snapshot[slot] = seen(m, uuid, world_items!(self));
                    snapshot[slot].alive &= kept;
                    snapshot[slot].removed |= !kept;
                    grid.update(slot, snapshot[slot].bounds);
                    let id = m.id;
                    brain_from = from;
                    let before_brain = mobs.stamp();
                    sting = sunrise_sim::mobs::apply_brain_events_until_hurt(
                        &mut mobs,
                        &mut level,
                        &mut events,
                        from,
                        &base.teams,
                        true,
                    );
                    views_dirty |= mobs.stamp() != before_brain;
                    if sting.is_none()
                        && events[from..]
                            .iter()
                            .any(|e| matches!(e, MobEvent::EffectMob { .. }))
                    {
                        let mut idx = from;
                        while idx < events.len() {
                            if matches!(events[idx], MobEvent::EffectMob { .. }) {
                                pending_effects.push(events.remove(idx));
                            } else {
                                idx += 1;
                            }
                        }
                    }
                    cursor += 1;
                    if !kept
                        && !was_removed
                        && (mobs[i].extra.mount.is_some()
                            || self.has_seated_passengers_with_lent_mobs(id, &mobs))
                    {
                        removing_vehicle = Some(id);
                        break;
                    }
                    if sting.is_some()
                        || section_hint.is_some()
                        || tell_post.is_some()
                        || !pending_effects.is_empty()
                        || removal.is_some()
                        || lid_push.is_some()
                    {
                        break;
                    }
                }
            }
            self.server.pending_native_sounds.append(&mut hit_sounds);
            if sting.is_some() {
                resume_brain = Some((brain_from, section_hint.take(), tell_post.take()));
            }
            let stinging = sting.is_some();
            if pause.is_none()
                && !stinging
                && retry_rider.is_none()
                && removing_vehicle.is_none()
                && section_hint.is_none()
                && stand_cut.is_none()
                && lid_push.is_none()
                && tell_pre.is_none()
                && tick_enchantments.is_none()
                && tell_post.is_none()
                && pending_effects.is_empty()
                && removal.is_none()
            {
                break;
            }
            #[cfg(any(test, feature = "test-support"))]
            BORROWED_MOB_YIELDS.with(|count| count.set(count.get() + 1));
            // No MobWorld, DamageLevel or navigation loan crosses this boundary.
            // The callback sees the real owning rows and level RNG/attachments,
            // and its writes are the state the continuation will resume from.
            sunrise_sim::mobs::living::section_writes::set_enabled(false);
            lent.give_back(&mut self.common.world);
            // Earlier mobs' pushes become visible before a real pause callback
            // can read or replace another actor's velocity. A section-only
            // or a told-alone yield must not change the normal end-of-pass
            // push ordering.
            let settle = (pause.is_some() && !matches!(pause, Some(Pause::Equipment { .. })))
                || stinging
                || retry_rider.is_some()
                || removing_vehicle.is_some()
                || removal.is_some()
                || stand_cut.is_some()
                || lid_push.is_some();
            let applied_pushes = !pushes.is_empty() && settle;
            if applied_pushes {
                for m in &mut mobs {
                    sunrise_sim::mobs::apply_mob_pushes(m, &mut pushes);
                }
            }
            let same_rows = !settle && !gone_rows && pending_effects.is_empty();
            if !same_rows {
                retain_mob_rows(&mut mobs, &mut player_credit, |m| {
                    // Give the departing vehicle back for its pre-removal effects
                    // and for marking it removed before the dismount hooks.
                    removing_vehicle == Some(m.id)
                        || removal.is_some_and(|(join, _)| join == m.join)
                        || index
                            .get(&m.id)
                            .and_then(|&slot| keep.get(slot as usize))
                            .copied()
                            .unwrap_or(true)
                });
                gone_rows = false;
            }
            // Mobs that joined during the pass go first on a restore: the rows move.
            let restored_offset = self.common.mobs.len();
            let same_rows = same_rows && restored_offset == 0;
            self.common
                .mobs
                .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SpawnsFirst);
            let mut restored = Some(self.common.mobs.stamp());
            self.file_mob_section_entries(&mut section_entered);
            std::mem::swap(&mut self.server.last_hurt_by_player, &mut player_credit);
            std::mem::swap(&mut self.server.died, &mut died);
            std::mem::swap(&mut self.common.creatures.ticked, &mut ticked);
            if let Some(host) = self.server.lan.as_mut() {
                std::mem::swap(&mut host.mob_players, &mut bindings);
            }
            let breeding = matches!(pause, Some(Pause::Breeding { .. }));
            if let Some(sections) = section_table.as_ref() {
                std::mem::swap(
                    &mut self.common.native_lifecycle.entity_sections,
                    &mut sections.borrow_mut(),
                );
            }
            // A work step's restock reaches the screen before any callback can edit it.
            let callback_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                for restock in state.restocks.drain(..) {
                    self.villager_offers_open(restock);
                }
                if let Some(index) = section_hint {
                    let written = self.entity_sections_ticked_live(
                        &mut section_moves,
                        Some(restored_offset + index),
                    );
                    restored = (!written).then(|| self.common.mobs.stamp());
                }
                if !pending_effects.is_empty() {
                    let _ = self.handle_mob_events(std::mem::take(&mut pending_effects));
                }
                if removal.is_some() || breeding {
                    // Consume completed removals before calling a listener which can unwind.
                    self.emit_death_poofs(gone.drain(..));
                }
                let mut vehicle_split = false;
                if let Some((join, split)) = removal {
                    if removing_vehicle.is_some_and(|id| {
                        self.common
                            .mob_index
                            .find(&self.common.mobs, id)
                            .is_some_and(|i| self.common.mobs[i].join == join)
                    }) {
                        vehicle_split = split;
                    } else {
                        self.complete_mob_removal(join, split);
                    }
                }
                if let Some(mut sting) = sting {
                    let before_hurt = events.drain(brain_from..sting.at).collect();
                    let _ = self.handle_mob_events(before_hurt);
                    if let Some(i) = self.common.mob_index.find(&self.common.mobs, sting.target)
                        && sting.same_target(&self.common.mobs[i])
                    {
                        if let Some(hit) = paused_hurt.take() {
                            self.server.paused_mob_hurt.0 = Some(hit);
                            if let Some((i, hurt)) = self.complete_mob_hurt(i, false) {
                                sting.resolved_hit(hurt);
                                self.finish_mob_damage_at(i);
                            } else {
                                sting.resolved_hit(false);
                            }
                        } else {
                            self.finish_mob_damage_at(i);
                        }
                        self.settle_creaking_hurt(sting.target);
                    }
                    if self
                        .common
                        .mob_object(sting.fish)
                        .is_some_and(|mob| sting.same_fish(mob))
                    {
                        if let Some(ticks) = sting.poison_ticks()
                            && self
                                .mob_object(sting.target)
                                .is_some_and(|mob| sting.same_target(mob))
                        {
                            self.add_mob_effect(
                                sting.target,
                                sunrise_level::effects::EffectInstance::new("poison", ticks, 0),
                                self.entity(sting.fish),
                            );
                        }
                        if let Some(sound) = self
                            .mob_object(sting.fish)
                            .and_then(|fish| sting.sound(fish))
                        {
                            let _ = self.handle_mob_events(vec![sound]);
                        }
                    } else {
                        events.truncate(brain_from);
                        resume_brain = None;
                    }
                } else if let Some(id) = retry_rider {
                    self.dismount_mob(id);
                } else if let Some((paused, (id, join))) = pause.as_mut().zip(resume_entity)
                    && let Some(i) = self.common.mob_index.find(&self.common.mobs, id)
                    && self.common.mobs[i].join == join
                    && !self.common.mobs[i].removed
                {
                    match paused {
                        Pause::Consumption { decision, .. } => {
                            self.apply_fox_consumption(decision);
                        }
                        Pause::Drink { stack, .. } => {
                            self.finish_mob_drink(id, stack);
                        }
                        Pause::Breathing { air, .. } => {
                            air_answer = self.ask_mob_breath(air);
                        }
                        Pause::Equipment { first, .. } => {
                            self.detect_mob_equipment_updates(id, join, *first);
                        }
                        Pause::RegisterBehaviors { .. } => {
                            self.flush_resident_mob_parts(id, join);
                        }
                        Pause::Breeding { event, .. } => {
                            if let Some(event) = event.take() {
                                self.deliver_breed(*event, true);
                            }
                        }
                        Pause::AfterDamage(_) => {
                            if self.common.mobs[i].has_protected_hurt()
                                || sunrise_sim::mobs::creaking::has_pending_hurt(
                                    &self.common.mobs[i],
                                )
                            {
                                // The earlier emissions (e.g. drowning bubbles) precede this hurt.
                                let before_hurt = events.split_off(resume_from);
                                let _ = self.handle_mob_events(before_hurt);
                            }
                            if let Some(i) = self.common.mob_index.find(&self.common.mobs, id)
                                && self.common.mobs[i].join == join
                                && !self.common.mobs[i].removed
                            {
                                self.server.paused_mob_hurt.0 = paused_hurt.take();
                                self.finish_mob_damage_at(i);
                                self.settle_creaking_hurt(id);
                            }
                        }
                        Pause::Trample(trample) => {
                            self.trample_farmland(id, false, *trample);
                        }
                        Pause::FallToPassengers(fall) => {
                            self.fall_to_passengers(id, fall.distance, fall.modifier);
                        }
                        Pause::InsideEffects(inside) => {
                            let effects = std::mem::take(&mut inside.effects);
                            self.flush_mob_inside_live(inside.mob, inside.join, effects);
                        }
                    }
                }
                if let Some(id) = removing_vehicle {
                    // LivingEntity.tickDeath broadcasts event 60 before remove;
                    // cube splitting and the killed effects also precede setRemoved.
                    let animation = gone.pop_if(|death| death.0 == id);
                    let killed_effects = removal.is_some();
                    let animate = |game: &mut Self| {
                        if let Some((id, at, width, height)) = animation {
                            game.emit_death_poofs([(id, at, width, height)]);
                        }
                        if vehicle_split
                            && let Some(i) = game.common.mob_index.find(&game.common.mobs, id)
                        {
                            let mut dying = game.common.mobs[i].clone();
                            game.split_cube_children(&mut dying);
                        }
                    };
                    if self.common.mob_object(id).is_some_and(|mob| mob.removed) {
                        animate(self);
                        self.finish_mob_removal(id);
                    } else {
                        self.remove_mob_after(id, killed_effects, animate);
                    }
                    if let Some(index) = self.common.mobs.iter().position(|mob| mob.id == id) {
                        self.common.mobs.remove(index);
                    }
                }
                if let Some((shulker, area, delta)) = lid_push {
                    self.server_shulker_lid_push(shulker, area, delta);
                }
                if let Some(cut) = stand_cut {
                    *state.stand_result = self.tick_armor_stands_before(cut);
                }
                if !stinging && let Some((id, kind)) = &tell_post {
                    self.native_tick_post_one(round, *id, &kind.id);
                }
                if let Some((id, join)) = tick_enchantments
                    && self
                        .mob_object(id)
                        .is_some_and(|mob| mob.join == join && !mob.removed)
                {
                    self.tick_equipment_enchantments(crate::game::commands::Target::Mob(id), id);
                    views_dirty = true;
                }
                // Let through, it checks its sleep as a mob told with its family does.
                if let Some((id, kind, at)) = &tell_pre
                    && self.native_tick_pre_one(round, *id, &kind.id)
                    && mod_api::__private::listened(mod_api::HookId::CanContinueSleeping)
                    && let Some(i) =
                        super::super::native_ticks::place(&self.common.mobs, *at, |m| m.id == *id)
                {
                    self.sync_sleeping_mob(i);
                }
            }))
            .err();
            if let Some(sections) = section_table.as_ref() {
                std::mem::swap(
                    &mut self.common.native_lifecycle.entity_sections,
                    &mut sections.borrow_mut(),
                );
            }
            let unchanged = same_rows && !views_dirty && restored == Some(self.common.mobs.stamp());
            if tell_pre.is_some() || tick_enchantments.is_some() {
                told_at = Some(cursor);
            }
            if tick_enchantments.is_some() {
                enchantment_tick_at = Some(cursor);
            }
            tell_post = None;
            if retry_rider.is_some() || removing_vehicle.is_some() {
                blocked = std::borrow::Cow::Owned(self.mobs_below_off_level());
            }
            mobs = self.common.mobs.lift();
            std::mem::swap(&mut self.server.last_hurt_by_player, &mut player_credit);
            std::mem::swap(&mut self.server.died, &mut died);
            std::mem::swap(&mut self.common.creatures.ticked, &mut ticked);
            if let Some(host) = self.server.lan.as_mut() {
                std::mem::swap(&mut host.mob_players, &mut bindings);
            }
            if callback_panic.is_some() || state.stand_result.is_err() {
                lent = sunrise_sim::health::LentLevel::take(&mut self.common.world);
                dispatch_panic = callback_panic;
                break;
            }
            guests = self.lan_guest_views();
            guest_slots = guest_indices(&guests);
            guest_ids = self
                .server
                .lan
                .as_ref()
                .map(|host| {
                    host.hub
                        .guests
                        .iter()
                        .map(|g| (g.conn, g.entity_id as u32))
                        .collect()
                })
                .unwrap_or_default();
            spare = guests.first().map(|g| g.1.clone());
            if !unchanged || applied_pushes {
                positions.clear();
                positions.resize(snapshot.len(), None);
                for (i, m) in mobs.iter().enumerate() {
                    let current = seen(m, uuid, world_items!(self));
                    match index.entry(m.id) {
                        Entry::Occupied(entry) => {
                            let slot = *entry.get() as usize;
                            snapshot[slot] = current;
                            positions[slot] = Some(i);
                        }
                        Entry::Vacant(entry) => {
                            entry.insert(snapshot.len() as u32);
                            snapshot.push(current);
                            positions.push(Some(i));
                        }
                    }
                }
                villager_slots.clear();
                solid_slots.clear();
                for (slot, (s, position)) in snapshot.iter_mut().zip(&positions).enumerate() {
                    let present = position.is_some() && keep.get(slot).copied().unwrap_or(true);
                    s.alive &= present;
                    s.removed |= !present;
                    if s.species == Species::Villager {
                        villager_slots.push(slot as u32);
                    }
                    if s.may_block_mobs() {
                        solid_slots.push(slot as u32);
                    }
                }
                grid =
                    sunrise_sim::mobs::seen_grid::SeenGrid::new(snapshot.iter().map(|s| s.bounds));
            }
            if let Some(writes) = self.common.world.path_writes.as_mut() {
                path_writes.append(writes);
            }
            lent = sunrise_sim::health::LentLevel::take(&mut self.common.world);
            resume = pause;
            resume_air_answer = air_answer;
            retried_rider = retry_rider;
            sunrise_sim::mobs::living::section_writes::set_enabled(capture_sections);
        }
        laps.lap("creature_loop");
        if dispatch_panic.is_none() && state.stand_result.is_ok() {
            if positions.is_empty() {
                let mut flags = keep.into_iter();
                retain_mob_rows(&mut mobs, &mut player_credit, |_| {
                    flags.next().unwrap_or(true)
                });
            } else {
                retain_mob_rows(&mut mobs, &mut player_credit, |m| {
                    index
                        .get(&m.id)
                        .and_then(|&slot| keep.get(slot as usize))
                        .copied()
                        .unwrap_or(true)
                });
            }
            for m in &mut mobs {
                sunrise_sim::mobs::apply_mob_pushes(m, &mut pushes);
            }
        }
        sunrise_sim::mobs::living::section_writes::finish();
        drop(live_section_observer);
        // Keep the reusable write storage for the next pass; the live-row callback has already
        // consumed every record that belonged to this family tick.
        section_moves.clear();
        if let Some(table) = section_table {
            let mut sections = table.borrow_mut();
            sections.ticked = section_moves;
            std::mem::swap(
                &mut self.common.native_lifecycle.entity_sections,
                &mut sections,
            );
        } else {
            self.common.native_lifecycle.entity_sections.ticked = section_moves;
        }
        self.file_mob_section_entries(&mut section_entered);
        self.common.native_lifecycle.entity_sections.entered = section_entered;
        *state.pushes = pushes;
        *state.mobs = mobs;
        *state.snapshot = snapshot;
        *state.events = events;
        *state.dead = dead;
        *state.died = died;
        *state.ticked = ticked;
        *state.guests = guests;
        *state.spare = spare;
        *state.bindings = bindings;
        *state.guest_ids = guest_ids;
        *state.guest_events = guest_events;
        *state.player_credit = player_credit;
        *state.path_writes = path_writes;
        *state.gone = gone;
        (lent, dispatch_panic)
    }

    fn file_mob_section_entries(&mut self, entered: &mut Vec<(u32, [f64; 3])>) {
        let table = self.common.resident_entities.table.get_mut();
        for (id, to) in entered.drain(..) {
            table.entered_section(id, to);
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;

impl crate::game::ServerGame<'_> {
    fn mob_can_freeze(&self, m: &Mob) -> bool {
        if m.species() == Species::Skeleton {
            return false;
        }
        if m.species() == Species::SulfurCube && sunrise_sim::mobs::sulfur_cube::has_body(m) {
            return false;
        }
        if world_items!(self)
            .entity_type_tags
            .contains("minecraft:freeze_immune_entity_types", &m.kind.id)
        {
            return false;
        }
        let immune = world_items!(self)
            .item_tags
            .get("minecraft:freeze_immune_wearables");
        ![
            m.extra.body_id(),
            m.extra.head_id(),
            m.extra.slot_id(sunrise_items::equipment::CHEST),
            m.extra.slot_id(sunrise_items::equipment::LEGS),
            m.extra.slot_id(sunrise_items::equipment::FEET),
        ]
        .into_iter()
        .flatten()
        .any(|item| immune.is_some_and(|t| t.contains(item)))
    }

    pub(in crate::game) fn flush_mob_inside_live(
        &mut self,
        mob_id: u32,
        join: sunrise_sim::join_seq::JoinSeq,
        mut effects: mod_api::InsideEffects,
    ) {
        if effects.is_empty() {
            return;
        }
        let Some(slot) = self.common.mob_index.find(&self.common.mobs, mob_id) else {
            return;
        };
        let mut access = GameMobInsideAccess {
            game: self,
            mob_id,
            join,
            slot,
        };
        effects.apply_and_clear(&mut access);
    }
}

struct GameMobInsideAccess<'a, 'g> {
    game: &'a mut crate::game::ServerGame<'g>,
    mob_id: u32,
    join: sunrise_sim::join_seq::JoinSeq,
    slot: usize,
}

impl GameMobInsideAccess<'_, '_> {
    fn slot(&self) -> Option<usize> {
        native_ticks::place(&self.game.common.mobs, self.slot, |m| {
            m.id == self.mob_id && m.join == self.join && !m.removed
        })
    }

    fn mob(&self) -> Option<&Mob> {
        Some(&self.game.common.mobs[self.slot()?])
    }

    fn mob_mut(&mut self) -> Option<&mut Mob> {
        self.slot = self.slot()?;
        Some(&mut self.game.common.mobs[self.slot])
    }
}

impl mod_api::InsideEntityAccess for GameMobInsideAccess<'_, '_> {
    #[inline]
    fn id(&self) -> u32 {
        self.mob_id
    }

    #[inline]
    fn kind(&self) -> &str {
        self.mob().map_or("", |m| m.kind.id.as_str())
    }

    #[inline]
    fn is_alive(&self) -> bool {
        self.mob().is_some_and(Mob::alive)
    }

    #[inline]
    fn position(&self) -> [f64; 3] {
        self.mob()
            .map_or([0.0; 3], |m| [m.position.x, m.position.y, m.position.z])
    }

    #[inline]
    fn velocity(&self) -> [f64; 3] {
        self.mob()
            .map_or([0.0; 3], |m| [m.velocity.x, m.velocity.y, m.velocity.z])
    }

    #[inline]
    fn fire_ticks(&self) -> i32 {
        self.mob().map_or(0, |m| m.fire_ticks)
    }

    #[inline]
    fn is_on_fire(&self) -> bool {
        self.mob().is_some_and(Mob::is_on_fire)
    }

    #[inline]
    fn fall_distance(&self) -> f64 {
        self.mob().map_or(0.0, |m| m.fall_distance)
    }

    #[inline]
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        use sunrise_gameplay_common::game::api_values::TryIntoApi as _;
        let level = self.game.level(self.game.common.dim.id())?;
        self.game
            .level_block_state(level, pos)
            .and_then(|id| id.try_into_api().ok())
    }

    #[inline]
    fn push(&mut self, delta: [f64; 3]) -> bool {
        if !delta.iter().all(|v| v.is_finite()) {
            return false;
        }
        let Some(m) = self.mob_mut() else {
            return false;
        };
        m.velocity = m
            .velocity
            .add_vector(Vec3::new(delta[0], delta[1], delta[2]));
        true
    }

    #[inline]
    fn set_velocity(&mut self, velocity: [f64; 3]) -> bool {
        if !velocity.iter().all(|v| v.is_finite()) {
            return false;
        }
        let Some(m) = self.mob_mut() else {
            return false;
        };
        m.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        true
    }

    #[inline]
    fn make_stuck_in_block(&mut self, speed_multiplier: [f64; 3]) -> bool {
        if !speed_multiplier.iter().all(|v| v.is_finite()) {
            return false;
        }
        let Some(m) = self.mob_mut() else {
            return false;
        };
        m.fall_distance = 0.0;
        m.stuck_multiplier = Vec3::new(
            speed_multiplier[0],
            speed_multiplier[1],
            speed_multiplier[2],
        );
        true
    }

    #[inline]
    fn reset_fall_distance(&mut self) {
        if let Some(m) = self.mob_mut() {
            m.fall_distance = 0.0;
        }
    }

    #[inline]
    fn set_remaining_fire_ticks(&mut self, ticks: i32) {
        if let Some(m) = self.mob_mut() {
            m.fire_ticks = ticks;
        }
    }

    #[inline]
    fn add_effect(&mut self, effect: mod_api::EffectKey, duration: i32, amplifier: u8) -> bool {
        let Some(def) = mod_api::__private::effect_index(effect)
            .and_then(sunrise_game_data::potions::effect_by_index)
        else {
            return false;
        };
        let instance =
            sunrise_level::effects::EffectInstance::from_def(def, duration, i32::from(amplifier));
        self.game.add_mob_effect(self.mob_id, instance, None)
    }

    #[inline]
    fn hurt_with_source(
        &mut self,
        damage_type: &str,
        amount: f32,
        direct: Option<mod_api::EntityRef>,
        causing: Option<mod_api::EntityRef>,
    ) -> bool {
        let Some(m) = self.mob() else {
            return false;
        };
        let subject_pos = [m.position.x, m.position.y, m.position.z];
        let Some((direct, causing, from)) = self.game.resolve_inside_damage_sources(
            self.mob_id,
            crate::game::commands::Target::Mob(self.mob_id),
            subject_pos,
            direct,
            causing,
            None,
        ) else {
            return false;
        };
        self.game.hurt_target(
            crate::game::commands::Target::Mob(self.mob_id),
            amount,
            damage_type,
            direct,
            causing,
            from,
        )
    }

    #[inline]
    fn set_block(&mut self, pos: [i32; 3], state: mod_api::BlockState) -> bool {
        use sunrise_gameplay_common::game::api_values::TryIntoEngine as _;
        let Some(level) = self.game.level(self.game.common.dim.id()) else {
            return false;
        };
        let Ok(id) = state.try_into_engine(&*self.game) else {
            return false;
        };
        let flags = mod_api::BlockUpdateFlags::NEIGHBORS | mod_api::BlockUpdateFlags::CLIENTS;
        self.game
            .level_set_block(level, pos, id, flags)
            .unwrap_or(false)
    }

    #[inline]
    fn destroy_block(&mut self, pos: [i32; 3], drop: bool) -> bool {
        let Some(level) = self.game.level(self.game.common.dim.id()) else {
            return false;
        };
        self.game
            .level_destroy_block(level, pos, drop, None, None, false)
            .is_some()
    }

    #[inline]
    fn apply_effect(&mut self, effect: mod_api::InsideEffect) {
        match effect {
            mod_api::InsideEffect::Freeze => {
                let can_freeze = self.mob().is_some_and(|m| self.game.mob_can_freeze(m));
                if let Some(m) = self.mob_mut() {
                    m.in_powder_snow = true;
                    if can_freeze {
                        m.ticks_frozen = (m.ticks_frozen + 1)
                            .min(sunrise_sim::motion::PlayerMotion::TICKS_REQUIRED_TO_FREEZE);
                    }
                }
            }
            mod_api::InsideEffect::ClearFreeze => {
                if let Some(m) = self.mob_mut() {
                    m.ticks_frozen = 0;
                }
            }
            mod_api::InsideEffect::FireIgnite => {
                if let Some(m) = self.mob_mut() {
                    m.fire_ignite_contact();
                }
            }
            mod_api::InsideEffect::LavaIgnite => {
                if let Some(m) = self.mob_mut()
                    && !m.fire_immune()
                {
                    m.ignite_for_ticks(300);
                }
            }
            mod_api::InsideEffect::Extinguish => {
                if let Some(m) = self.mob_mut() {
                    m.fire_ticks = m.fire_ticks.min(0);
                }
            }
        }
    }

    #[inline]
    fn apply_builtin(&mut self, action: mod_api::__private::BuiltinInsideAction) {
        match action {
            mod_api::__private::BuiltinInsideAction::PowderSnowMelt(pos) => {
                if self.mob().is_some_and(Mob::is_on_fire)
                    && self.game.can_entity_grief(Some(self.mob_id))
                    && let Some(level) = self.game.level(self.game.common.dim.id())
                {
                    let _ = self
                        .game
                        .level_destroy_block(level, pos, false, None, None, false);
                }
            }
            mod_api::__private::BuiltinInsideAction::LayeredCauldronExtinguish { pos, state } => {
                if self.mob().is_some_and(Mob::is_on_fire) {
                    let [x, y, z] = pos;
                    self.game.cauldron_extinguish_at(
                        sunrise_core::position::BlockPos::new(x, y, z),
                        sunrise_world::registry::BlockStateId(state),
                    );
                }
            }
            mod_api::__private::BuiltinInsideAction::FireHurt(damage) => {
                self.hurt_with_source("minecraft:in_fire", damage, None, None);
            }
            mod_api::__private::BuiltinInsideAction::LavaHurt => {
                if self.mob().is_some_and(|m| !m.fire_immune())
                    && self.hurt_with_source("minecraft:lava", 4.0, None, None)
                    && let Some(m) = self.mob_mut().filter(|m| !m.silent)
                {
                    let at = m.position.components();
                    let pitch = 2.0 + m.random.next_float() * 0.4;
                    self.game
                        .server_play_sound("minecraft:entity.generic.burn", at, 0.4, pitch);
                }
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static BORROWED_MOB_YIELDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
