//! Removing an entity for native server mods, through an `EntityRef`
//! (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387): `Entity.discard`
//! and `Entity.remove` with a reason. Nothing dies and nothing drops loot.
//! `Entity.spawnAtLocation` drops an item at one.
//! `Entity.setRemoved` tells the world at the call: the handle and selectors stop finding it,
//! passengers get off and mods hear it leave. Inside a transaction the handle stops finding it at
//! the call and the rest waits for the root commit (`transactions/entities.rs`); an abort leaves
//! it as it was. A mob's list entry is marked at once too; the other
//! kinds stay in their lists until the level's next sweep of departed
//! entities, a few times per tick, so a hook that removes one never shifts the
//! lists the game is walking. A container vehicle spills before its logical removal;
//! only taking the row out of its list waits for the sweep.
use super::commands::Target;
use mod_api::{EntityRef, LevelWriteError, RemovalReason};
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_sim::entities::ItemEntity;

impl crate::game::ServerGame<'_> {
    /// Removes the entity with no death and no loot (`Entity.discard`): a
    /// chest boat or a minecart with a chest still drops what it holds, and
    /// a boat its lead, as vanilla's `remove` does. False for a player, a dragon part,
    /// or a stale handle.
    pub fn entity_discard(&mut self, entity: EntityRef) -> bool {
        self.entity_remove(entity, RemovalReason::Discarded)
    }
    /// Removes the entity for `reason` (`Entity.remove`): `Discarded`, as
    /// `discard`, or `Killed`, which also runs a mob's effects that act on
    /// removal (Oozing's slimes, Weaving's cobwebs, Wind Charged's burst),
    /// still with no death and no loot. False for a player, a dragon part, or a stale
    /// handle.
    pub fn entity_remove(&mut self, entity: EntityRef, reason: RemovalReason) -> bool {
        let killed = reason == RemovalReason::Killed;
        match self.native_entity_target(entity) {
            // A part leaves the level with its dragon, never alone.
            None | Some(Target::Player | Target::Guest(_) | Target::DragonPart(_)) => false,
            Some(t) if self.common.persist.preparing_vehicle_removals.contains(&t) => false,
            Some(t) if self.common.journal_removal(entity, t, killed) => true,
            Some(Target::Mob(id)) => {
                self.remove_mob_now(id, killed);
                true
            }
            Some(t) => {
                self.remove_target_now(entity, t);
                true
            }
        }
    }
    /// Kills the entity as `/kill` does (`Entity.kill(ServerLevel)` and
    /// `LivingEntity.kill(ServerLevel)` -> `hurtServer(level, damageSources().genericKill(), Float.MAX_VALUE)`):
    /// a living entity falls and drops its loot, and any other entity is removed
    /// and posts `minecraft:entity_die`. Reaches entities in parked dimensions.
    /// An open transaction stages the kill in its level; its effects wait for
    /// the root commit. A live target that refuses the kill returns `Ok(false)`.
    pub fn entity_kill(&mut self, entity: EntityRef) -> Result<bool, LevelWriteError> {
        if !self.common.is_logical_server() {
            return Err(LevelWriteError::Unavailable);
        }
        if self.common.native_lifecycle.epoch() != Some(entity.epoch())
            || self.server.held_out.contains(&entity.id())
        {
            return Err(LevelWriteError::Stale);
        }
        let sunrise_sim::join_seq::residency::table::EntityLookup::Found { location, value } =
            self.server_resident_entity(entity.id())
        else {
            return Err(LevelWriteError::Stale);
        };
        if matches!(value, Target::DragonPart(_)) {
            return Ok(false);
        }
        if location.owner.dimension != self.common.dim.key() && !self.common.transactions.is_empty()
        {
            return Err(LevelWriteError::InTransaction);
        }
        if self.common.transactions.is_empty() {
            return Ok(self.kill_target(value));
        }
        if !self.can_kill_target_in_level(value) {
            return Ok(false);
        }
        if self.journal_kill(entity) {
            Ok(true)
        } else {
            Ok(self.kill_target(value))
        }
    }
    /// `Entity.setRemoved` of `t`, which `entity` names and is no mob, player or dragon part.
    pub(in crate::game::server_side) fn remove_target_now(&mut self, entity: EntityRef, t: Target) {
        let id = entity.id();
        if matches!(t, Target::Boat(_) | Target::Cart(_)) {
            self.with_vehicle_removal_effects(
                t,
                |game| game.prepare_vehicle_removal(t, id),
                |game| {
                    // Keep the logical removal visible after the guard is released.
                    game.persist.removals.push(t);
                    game.persist.notified_vehicle_removals.insert(id);
                    game.score_destroyed_vehicle_once(id);
                    game.native_entity_left(id);
                },
            );
            return;
        }
        let position = self.target_position(t);
        // `Entity.setRemoved`: the reason first, so the hooks below see
        // it removed; then the riding links are ended and the level is told.
        let mount = self.native_vehicle_id(t);
        self.common.persist.removals.push(t);
        if let Some(vehicle) = mount {
            // The object is already leaving even if its dismount is vetoed.
            self.native_mob_may_ride(entity.id(), vehicle, false);
            self.release_removed_vehicle(vehicle);
        }
        self.retire_vehicle_registration(t);
        if matches!(
            t,
            Target::Stand(_) | Target::Tnt(_) | Target::Other(_) | Target::Cushion(_)
        ) {
            // The physical sweep uses the same destruction paths, but a
            // passenger veto here must not trigger another attempt there.
            self.common
                .persist
                .notified_vehicle_removals
                .insert(entity.id());
        }
        if let Some([_, y, _]) = position {
            self.dismount_removed_passengers(entity.id(), y);
        }
        self.native_entity_left(entity.id());
    }
    /// Drops `stack` at the entity's feet, `y_offset` above
    /// (`Entity.spawnAtLocation`), or 0.15 out along a frame or painting's
    /// horizontal facing (`HangingEntity.spawnAtLocation`), moving as a new item does, with the
    /// default pickup delay: the item entity. None for an empty stack or an
    /// unknown item, a stale handle, a player in another level, or an item a
    /// mod kept out.
    pub(in crate::game) fn entity_spawn_at_location(
        &mut self,
        entity: EntityRef,
        stack: ItemStack,
        y_offset: f32,
    ) -> Option<EntityRef> {
        if self.entity_ids_exhausted() {
            return None;
        }
        if !self.stack_spawns(&stack) {
            return None;
        }
        let target = self.native_entity_target(entity)?;
        if let Target::Guest(conn) = target
            && self.command_guest(conn)?.dimension != self.common.dim.id()
        {
            return None;
        }
        let at = self.target_position(target)?;
        let [x, y, z] = match target {
            Target::Frame(id) => {
                let frame = self
                    .common
                    .item_frames
                    .iter()
                    .find(|frame| frame.id == id)?;
                super::frames::hanging_drop_position(at, frame.step())
            }
            Target::Painting(id) => {
                let painting = self
                    .common
                    .paintings
                    .iter()
                    .find(|painting| painting.id == id)?;
                super::frames::hanging_drop_position(at, painting.step())
            }
            _ => at,
        };
        // Found by its id: the hooks the spawn runs may add entities after it.
        let id = self.server.next_entity + 1;
        self.spawn_at_location(Vec3::new(x, y, z), f64::from(y_offset), stack);
        self.common
            .dropped
            .iter()
            .rev()
            .any(|d| d.id == id)
            .then(|| EntityRef::new(entity.epoch(), id))
    }
    /// A mob leaves the level: its effects act first when it was killed.
    pub fn remove_mob_now(&mut self, id: u32, killed: bool) {
        self.remove_mob_after(id, killed, |_| {});
    }

    /// Run the caller's preparation and the killed effects while the mob still resolves.
    pub(in crate::game::server_side) fn remove_mob_after(
        &mut self,
        id: u32,
        killed: bool,
        prepare: impl FnOnce(&mut Self),
    ) {
        if !self.common.mobs.iter().any(|m| m.id == id && !m.removed) {
            return;
        }
        if self
            .with_removal_preparation(Target::Mob(id), |game| {
                prepare(game);
                if killed {
                    game.mob_removal_effects(id);
                }
            })
            .is_some()
            && self.common.mobs.iter().any(|m| m.id == id && !m.removed)
        {
            self.finish_mob_removal(id);
        }
    }

    /// `Entity.setRemoved` for a mob, also reached by the tick and the root
    /// transaction commit. A departing rider gets off before its own passengers.
    pub(in crate::game::server_side) fn finish_mob_removal(&mut self, id: u32) {
        let Some(mob) = self.mob_object_mut(id) else {
            return;
        };
        mob.removed = true;
        let position = mob.position;
        self.server.last_hurt_by_player.remove(&id);
        self.dismount_mob(id);
        self.dismount_removed_passengers(id, position.y);
        if self.has_seated_passengers(id)
            && let Some(mob) = self.common.mob_object(id)
        {
            let vehicle = sunrise_gameplay_common::game::entity_memory::RemovedVehicle::Mob {
                position,
                kind: crate::game::native_hooks::mob_kind(mob),
            };
            self.retain_removed_vehicle(id, vehicle);
        }
        self.native_entity_left(id);
    }
    /// What a killed mob's effects do on removal (Oozing's slimes, Weaving's cobwebs, Wind
    /// Charged's burst).
    pub(in crate::game::server_side) fn mob_removal_effects(&mut self, id: u32) {
        let Some(i) = self.common.mobs.iter().position(|m| m.id == id) else {
            return;
        };
        if !self.common.mobs[i].extra.effects.is_empty() {
            let m = &self.common.mobs[i];
            let names: Vec<&'static str> = m
                .extra
                .effects
                .iter()
                .map(|e| e.id.trim_start_matches("minecraft:"))
                .collect();
            let (bounds, silent, mut random) = (m.bounds(), m.silent, m.random.clone());
            self.effects_on_death(&names, bounds, silent, Some(id), Some(&mut random));
            if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                m.random = random;
            }
        }
    }

    /// Retires the item rows for removals already announced by `remove_target_now`. The item pass
    /// also sweeps these while its rows are resident, before it can tick, merge or pick them up.
    pub(in crate::game) fn native_item_removals_flush(
        &mut self,
        mut retired: impl FnMut(&ItemEntity),
    ) {
        let dropped = &mut self.common.dropped;
        self.common.persist.removals.retain(|target| {
            let Target::Item(id) = *target else {
                return true;
            };
            if let Some(i) = dropped.iter().position(|item| item.id == id) {
                retired(&dropped.remove(i));
            }
            false
        });
    }
    /// Takes out the entities mods removed since the last sweep.
    pub fn native_removals_flush(&mut self) {
        self.native_item_removals_flush(|_| {});
        if self.common.persist.removals.is_empty() {
            return;
        }
        // Keep every deferred vehicle logically removed while callbacks in
        // this sweep can inspect the remaining physical owning slots.
        let pending = std::mem::take(&mut self.common.persist.removals);
        let mut already_marked = Vec::new();
        for &target in &pending {
            match target {
                // Discard already announced the leave. Remove these owning rows
                // before any callback in the sweep can resolve them again.
                Target::Projectile(id) => self.remove_projectile(id),
                Target::Trident(id) => {
                    self.common.tridents.retain(|t| t.id != id);
                    self.forget_entity(id);
                }
                Target::Boat(_) | Target::Cart(_)
                    if !self.common.persist.destroying_vehicles.insert(target) =>
                {
                    already_marked.push(target);
                }
                _ => {}
            }
        }
        for t in pending {
            match t {
                Target::Projectile(_) | Target::Trident(_) => {}
                Target::Crystal(id) => self.common.crystals.retain(|c| c.id != id),
                Target::Dragon(id) => {
                    if self.common.dragon.as_ref().is_some_and(|d| d.id == id) {
                        self.discard_resident_dragon();
                    }
                }
                Target::Boat(id) | Target::Cart(id) => {
                    let marked = !already_marked.contains(&t);
                    if self.common.persist.notified_vehicle_removals.remove(&id) {
                        self.take_destroyed_vehicle_row(t, id);
                    } else if marked {
                        // A raw deferred entry still needs the whole removal.
                        self.common.persist.destroying_vehicles.remove(&t);
                        self.kill_vehicle_object(t);
                    }
                    self.score_destroyed_vehicle_once(id);
                    if marked {
                        self.common.persist.destroying_vehicles.remove(&t);
                    }
                    self.native_entity_left(id);
                }
                Target::Player | Target::Guest(_) | Target::Mob(_) => {}
                // Both command kill and handle removal already notified this row's
                // leave. Retire its owning slot so tracking drops the row too.
                Target::General(id) => {
                    let rows = &mut self.common.resident_entities.general.rows;
                    if let Some(index) = rows.iter().position(|row| row.id == id) {
                        rows.remove(index);
                        self.forget_entity(id);
                    }
                }
                // The remaining kinds `/kill` already removes with no death.
                t => {
                    self.kill_target_in_level(t);
                }
            }
        }
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

#[cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

#[cfg(test)]
mod tests {
    use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
    use {
        sunrise_gameplay_common::game::api_values::StackReadError,
        sunrise_gameplay_common::game::api_values::TryIntoApi,
        sunrise_gameplay_common::game::api_values::test_snapshot,
    };
    #[test]
    #[ignore = "daily"]
    fn destroying_a_cart_spills_all_slots_before_removing_its_row() {
        use super::super::{
            minecarts::{CartKind, Minecart},
            native_entity_refs::tests::with_resident_game,
        };
        use super::*;
        with_resident_game(|game| {
            let mut cart = Minecart::new(600, CartKind::Chest, Vec3::new(1.0, 70.0, 2.0));
            cart.items[0] = Some(ItemStack::new("minecraft:stone", 64));
            cart.items[5] = Some(ItemStack::new("minecraft:stone", 19));
            cart.items[26] = Some(ItemStack::new("minecraft:sand", 7));
            game.carts.push(cart);
            let entity = game.entity(600).unwrap();
            assert!(game.entity_remove(entity, RemovalReason::Killed));
            assert!(game.entity(600).is_none());
            assert_eq!(game.carts.len(), 1);
            let mut totals = std::collections::BTreeMap::new();
            for item in game.dropped.iter() {
                *totals.entry(item.stack.item.to_string()).or_insert(0) += item.stack.count;
            }
            assert_eq!(
                totals,
                std::collections::BTreeMap::from([
                    ("minecraft:stone".to_owned(), 83),
                    ("minecraft:sand".to_owned(), 7),
                ])
            );
            let count = game.dropped.len();
            game.native_removals_flush();
            assert!(game.carts.is_empty());
            assert_eq!(game.dropped.len(), count);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn selectors_stop_matching_a_discarded_entity_before_the_sweep() {
        use super::super::{
            commands::OtherEntity,
            native_entity_refs::tests::{fixture_resident_column, with_game},
        };
        use sunrise_core::geometry::Vec3;

        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            game.server.others.0.push(OtherEntity {
                size_answer: None,
                attachment_lifetime: Default::default(),
                knot_check: None,
                eye: None,
                join: Default::default(),
                display: None,
                interaction: None,
                id: 12,
                kind: "minecraft:marker".into(),
                position: Vec3::ZERO,
                yaw: 0.0,
                pitch: 0.0,
            });
            game.ui.registries.permission = 4;
            let selector = "execute if entity @e[type=minecraft:marker]";
            assert!(game.run_command(selector));
            let entity = game.entity(12).unwrap();
            assert!(entity.discard(game));
            // No sweep has run, as in the hook that discarded it.
            assert_eq!(game.server.others.0.len(), 1);
            assert!(!game.run_command(selector));
            assert!(game.level_entity(12).is_none());
            game.native_removals_flush();
            assert!(game.server.others.0.is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn removal_dismounts_and_tells_mods_once_at_the_call() {
        use super::super::{boats_common::Boat, native_entity_refs::tests::with_resident_game};
        use mod_api::{EntityLeaveLevelEvent, Hooks, ServerHost};
        use std::sync::{Arc, Mutex};
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;

        /// The entities heard leaving, and whether each read as gone by then.
        #[derive(Clone, Default)]
        struct Observer(Arc<Mutex<Vec<(u32, bool)>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn entity_leave_level(
                &self,
                host: &mut dyn ServerHost,
                event: &mut EntityLeaveLevelEvent<'_>,
            ) {
                let gone = host.entity(event.entity).is_none();
                self.0.lock().unwrap().push((event.entity, gone));
            }
        }
        let observer = Observer::default();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "removal-observer",
            Box::new(observer.clone()),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_resident_game(|game| {
                game.native_joins_skip();
                let boat_id = game.take_entity_id();
                game.boats.push(Boat::new(
                    boat_id,
                    "oak".into(),
                    Vec3::new(0.0, 64.0, 0.0),
                    0.0,
                ));
                let mob_id = game.take_entity_id();
                let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                game.mobs
                    .push(Mob::new(mob_id, cow, Vec3::new(0.0, 64.0, 0.0), 0.0));
                game.native_joins_flush(false, 0);
                let boat = game.entity(boat_id).unwrap();
                let mob = game.entity(mob_id).unwrap();
                assert!(game.entity_start_riding(mob, boat, true).is_ok());
                assert_eq!(game.entity_vehicle(mob), Some(boat));

                assert!(boat.discard(game));
                // Told, and off the vehicle, with the boat still in its list.
                assert!(game.boats.iter().any(|b| b.id == boat_id));
                assert_eq!(*observer.0.lock().unwrap(), vec![(boat_id, true)]);
                assert_eq!(game.entity_vehicle(mob), None);
                assert!(!game.mobs.iter().any(|m| m.extra.mount == Some(boat_id)));

                game.native_leaves_flush();
                assert!(!game.boats.iter().any(|b| b.id == boat_id));
                assert_eq!(*observer.0.lock().unwrap(), vec![(boat_id, true)]);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn discard_other_clears_nbt_and_saved_entity() {
        use super::super::{commands::OtherEntity, native_entity_refs::tests::with_game};
        use std::sync::Arc;
        use sunrise_core::geometry::Vec3;
        use sunrise_core::nbt::{Compound, Tag};
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::save::SaveDir;

        with_game(|game| {
            let root = std::path::PathBuf::from(format!(
                "target/discard-save-test-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            let shape = game.server.save.0.shape;
            game.server.save.0 =
                Arc::new(SaveDir::new(root.clone(), shape, Some(Default::default())));
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.server.others.0.push(OtherEntity {
                size_answer: None,
                attachment_lifetime: Default::default(),
                knot_check: None,
                eye: None,
                join: Default::default(),
                display: None,
                interaction: None,
                id: 12,
                kind: "minecraft:text_display".into(),
                position: Vec3::new(1.0, 70.0, 1.0),
                yaw: 0.0,
                pitch: 0.0,
            });
            let mut tag = Compound::new();
            tag.insert("teleport_duration".into(), Tag::Int(4));
            game.persist.raw.insert(12, Tag::Compound(tag));
            game.save_entities(true).unwrap();
            assert_eq!(
                SaveDir::new(root.clone(), shape, None)
                    .load_entities(0, 0)
                    .unwrap()
                    .len(),
                1
            );
            let entity = game.entity(12).unwrap();
            assert!(entity.discard(game));
            assert!(!entity.discard(game));
            game.native_removals_flush();
            assert!(!game.persist.raw.contains_key(&12));
            game.save_entities(true).unwrap();
            assert!(
                SaveDir::new(root.clone(), shape, None)
                    .load_entities(0, 0)
                    .unwrap()
                    .is_empty()
            );
            std::fs::remove_dir_all(root).unwrap();
        });
    }

    #[test]
    #[ignore = "daily"]
    fn discard_mob_and_refuse_players_and_stale_handles() {
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::{mobs::Mob, motion::PlayerMotion};

        with_resident_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let mob = game.entity(12).unwrap();
            assert!(mob.discard(game));
            assert!(game.mobs[0].removed);
            assert!(!mob.discard(game));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();
            assert!(!player.discard(game));
            assert!(game.player.is_some());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn spawn_at_location_drops_an_item_above_the_entity() {
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_core::geometry::Vec3;
        use sunrise_items::items::ItemStack;
        use sunrise_sim::mobs::Mob;

        with_resident_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(12, kind, Vec3::new(3.5, 70.0, -2.5), 0.0));
            let mob = game.entity(12).unwrap();
            let item = mob
                .spawn_at_location(
                    game,
                    test_snapshot(&ItemStack::new("minecraft:carrot", 2)),
                    0.5,
                )
                .unwrap()
                .unwrap();
            let dropped = game.dropped.iter().find(|d| d.id == item.id()).unwrap();
            assert_eq!(&*dropped.stack.item, "minecraft:carrot");
            assert_eq!(dropped.stack.count, 2);
            assert_eq!(dropped.position, Vec3::new(3.5, 70.5, -2.5));
            assert_eq!(dropped.pickup_delay, 10);
            // Invalid engine values are refused before an API value can exist.
            let before = game.dropped.len();
            assert_eq!(
                ItemStack::new("minecraft:carrot", 0).try_into_api(),
                Err(StackReadError::Empty)
            );
            assert_eq!(
                ItemStack::new("minecraft:no_such_item", 1).try_into_api(),
                Err(StackReadError::UnknownItem)
            );
            assert!(mob.discard(game));
            assert_eq!(
                mob.spawn_at_location(
                    game,
                    test_snapshot(&ItemStack::new("minecraft:carrot", 1)),
                    0.0
                ),
                Ok(None)
            );
            assert_eq!(game.dropped.len(), before);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn spawn_at_location_offsets_hanging_entities_out_from_the_wall() {
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_core::geometry::Vec3;
        use sunrise_items::items::ItemStack;
        use sunrise_sim::entities::{ItemFrame, Painting};

        with_resident_game(|game| {
            for (id, glow, facing) in [(12, false, 5), (13, true, 4)] {
                game.item_frames.push(ItemFrame {
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id,
                    pos: [0, 64, 0],
                    facing,
                    glow,
                    item: None,
                    rotation: 0,
                    drop_chance: 1.0,
                    fixed: false,
                    invisible: false,
                    check_interval: 0,
                    ticks: 0,
                    custom_name: None,
                    random: sunrise_core::random::LegacyRandom::new(0),
                });
            }
            game.paintings.push(Painting {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id: 14,
                pos: [0, 64, 0],
                facing: 2,
                variant: "minecraft:kebab".into(),
                asset: None,
                width: 1,
                height: 1,
                check_interval: 0,
                ticks: 0,
            });
            let offset = f64::from(0.15_f32);
            for (id, expected) in [
                (12, Vec3::new(0.03125 + offset, 65.0, 0.5)),
                (13, Vec3::new(0.96875 - offset, 65.0, 0.5)),
                (14, Vec3::new(0.5, 65.0, 0.96875 - offset)),
            ] {
                let entity = game.entity(id).unwrap();
                let item = entity
                    .spawn_at_location(
                        game,
                        test_snapshot(&ItemStack::new("minecraft:carrot", 2)),
                        0.5,
                    )
                    .unwrap()
                    .unwrap();
                let dropped = game.dropped.iter().find(|d| d.id == item.id()).unwrap();
                assert_eq!(dropped.position, expected);
                assert_eq!(dropped.stack.count, 2);
                assert_eq!(dropped.pickup_delay, 10);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn kill_command_forgets_other_entity_data() {
        use super::super::{commands::OtherEntity, native_entity_refs::tests::with_game};
        use sunrise_core::geometry::Vec3;
        use sunrise_core::nbt::{Compound, Tag};

        with_game(|game| {
            game.server.others.0.push(OtherEntity {
                size_answer: None,
                attachment_lifetime: Default::default(),
                knot_check: None,
                eye: None,
                join: Default::default(),
                display: None,
                interaction: None,
                id: 12,
                kind: "minecraft:marker".into(),
                position: Vec3::ZERO,
                yaw: 0.0,
                pitch: 0.0,
            });
            game.persist.raw.insert(12, Tag::Compound(Compound::new()));
            game.ui.registries.permission = 4;
            assert!(game.run_command("kill @e[type=minecraft:marker]"));
            assert!(game.server.others.0.is_empty());
            assert!(!game.persist.raw.contains_key(&12));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn entity_kill_kills_mobs_non_mobs_parked_entities_and_refuses_stale() {
        use super::super::native_entity_refs::tests::{fixture_resident_column, with_game};
        use super::*;
        use sunrise_sim::{entities::ExperienceOrb, mobs::Mob};

        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            // Killing a living mob in the current level kills it; a second kill
            // while already dead returns Ok(false).
            let zombie_kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let zombie_id = game.take_entity_id();
            game.mobs
                .push(Mob::new(zombie_id, zombie_kind, Vec3::ZERO, 0.0));
            let zombie = game.entity(zombie_id).unwrap();
            assert_eq!(zombie.kill(game), Ok(true));
            assert!(
                !game
                    .mobs
                    .iter()
                    .find(|m| m.id == zombie_id)
                    .unwrap()
                    .alive()
            );
            assert_eq!(zombie.kill(game), Ok(false));

            // Killing a non-mob entity (experience orb) removes it and makes
            // the handle stale afterwards.
            let orb_id = game.take_entity_id();
            game.orbs.push(ExperienceOrb::new(
                orb_id,
                Vec3::new(1.0, 70.0, 1.0),
                Vec3::ZERO,
                5,
            ));
            let orb = game.entity(orb_id).unwrap();
            assert_eq!(orb.kill(game), Ok(true));
            assert!(game.orbs.iter().all(|o| o.id != orb_id));
            assert_eq!(orb.kill(game), Err(LevelWriteError::Stale));

            // Stale handle from another epoch is refused with Stale.
            let stale_epoch = mod_api::WorldEpoch::new(
                game.native_lifecycle.epoch().unwrap().get().wrapping_add(1),
            );
            let stale = EntityRef::new(stale_epoch, zombie_id);
            assert_eq!(stale.kill(game), Err(LevelWriteError::Stale));

            // Killing an entity in a parked level routes there outside a
            // transaction and refuses with InTransaction inside one.
            let cow_kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            let cow_id = game.take_entity_id();
            game.mobs.push(Mob::new(cow_id, cow_kind, Vec3::ZERO, 0.0));
            let cow = game.entity(cow_id).unwrap();
            let nether = super::super::Dim::Nether;
            game.enter_command_level(nether).unwrap();

            game.storage_scope(|game, _| {
                assert_eq!(cow.kill(game), Err(LevelWriteError::InTransaction));
                ((), false)
            });

            assert_eq!(cow.kill(game), Ok(true));
            assert_eq!(game.dim, nether);
            assert!(
                !game
                    .server
                    .levels
                    .0
                    .parked_mobs(super::super::Dim::Overworld)
                    .unwrap()
                    .iter()
                    .find(|m| m.id == cow_id)
                    .unwrap()
                    .alive()
            );
            assert_eq!(cow.kill(game), Ok(false));

            // Client side is refused with Unavailable.
            let role = game.options.execution_role;
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            assert_eq!(cow.kill(game), Err(LevelWriteError::Unavailable));
            game.options.execution_role = role;
        });
    }
}
