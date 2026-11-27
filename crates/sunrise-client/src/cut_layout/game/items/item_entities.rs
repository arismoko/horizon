#![cfg(test)]
use sunrise_core::geometry::Vec3;
#[cfg(test)]
pub(crate) mod destroyed_contents_tests {
    use super::*;
    use sunrise_core::random::RandomSource;
    use sunrise_gameplay::game::game_tests::tests::server_game;
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_items::items::ItemStack;

    /// The motion of each spilled stack, in the order the spill made them.
    fn spilled_motions(game: &sunrise_gameplay::game::ServerGame<'_>) -> Vec<[f64; 3]> {
        game.dropped
            .iter()
            .map(|item| [item.velocity.x, item.velocity.y, item.velocity.z])
            .collect()
    }

    /// `ItemUtils.onContainerDestroyed` makes each stack with
    /// `new ItemEntity(level, x, y, z, stack)`, whose motion is 0.2 up and
    /// ±0.1 sideways from the *item's* random (ItemEntity.java:61-66), and
    /// draws nothing from the level's. Every destroyed container funnels here:
    /// lava, fire and cactus in the item's environment tick, an explosion, and
    /// the damage command.
    #[test]
    #[ignore = "daily"]
    fn spilled_stacks_rise_from_their_own_randoms() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let at = Vec3::new(8.5, 70.0, 8.5);
        let stacks = ["minecraft:stone", "minecraft:dirt", "minecraft:glass"]
            .into_iter()
            .map(|id| (ItemStack::new(id, 1), at))
            .collect();
        let mut level = game.world.random.clone();
        game.spawn_destroyed_contents(stacks);
        let motions = spilled_motions(&game);
        assert_eq!(motions.len(), 3);
        for motion in &motions {
            assert_eq!(motion[1], 0.2, "the four-argument constructor rises 0.2");
            assert!((-0.1..0.1).contains(&motion[0]), "{motion:?}");
            assert!((-0.1..0.1).contains(&motion[2]), "{motion:?}");
        }
        // Three stacks, three different sideways motions: each drew its own.
        assert_ne!(motions[0], motions[1]);
        assert_ne!(motions[1], motions[2]);
        assert_ne!(motions[0], motions[2]);
        // The spill leaves the level's random alone.
        assert_eq!(game.world.random.next_double(), level.next_double());
        assert_eq!(game.world.random.next_double(), level.next_double());
    }

    /// The damage command is one of those callers: killing a shulker box item
    /// spills its contents with the same upward motion.
    #[test]
    #[ignore = "daily"]
    fn a_damaged_shulker_box_spills_its_stacks_upward() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        let feet = Vec3::new(8.5, 70.0, 8.5);
        game.player = Some(sunrise_sim::motion::PlayerMotion::new(game.motion, feet).unwrap());
        game.update_chunk_tickets();
        let boxed = sunrise_items::item_data::parse_stack(
            r#"{id:"minecraft:shulker_box",count:1,components:{"minecraft:container":[
                {slot:0,item:{id:"minecraft:stone",count:2}},
                {slot:4,item:{id:"minecraft:dirt",count:1}},
                {slot:8,item:{id:"minecraft:glass",count:3}}]}}"#,
        )
        .unwrap()
        .unwrap();
        game.spawn_item(boxed, [8.5, 70.0, 8.5], [0.0; 3], 0);
        assert_eq!(game.dropped.len(), 1);
        assert!(game.run_command("damage @e[type=minecraft:item,limit=1] 100 minecraft:generic"));
        let mut spilled: Vec<_> = game
            .dropped
            .iter()
            .map(|item| (item.stack.item.to_string(), item.stack.count))
            .collect();
        spilled.sort();
        assert_eq!(
            spilled,
            [
                ("minecraft:dirt".to_owned(), 1),
                ("minecraft:glass".to_owned(), 3),
                ("minecraft:stone".to_owned(), 2)
            ]
        );
        for item in &game.dropped {
            assert_eq!(item.velocity.y, 0.2, "{item:?}");
        }
    }
}

#[cfg(test)]
pub(crate) mod escape_tests {
    use sunrise_core::random::RandomSource;
    use sunrise_gameplay::game::game_tests::tests::server_game;
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_items::items::ItemStack;

    /// `Entity.moveTowardsClosestSpace` uses the item's own random, never the
    /// level's random. Seeded Java values pin vanilla's float arithmetic (#13043).
    #[test]
    #[ignore = "daily"]
    fn an_item_escapes_a_block_at_its_own_randoms_speed() {
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        let feet = sunrise_core::geometry::Vec3::new(8.5, 70.0, 8.5);
        game.player = Some(sunrise_sim::motion::PlayerMotion::new(game.motion, feet).unwrap());
        crate::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
            &mut game,
            sunrise_core::position::ChunkPos::new(0, 0),
        );
        game.update_chunk_tickets();
        assert!(game.entity_ticking_area()(8, 8));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(8, 64, 8, stone).unwrap();
        let stack = ItemStack::new("minecraft:stone", 2);
        game.spawn_item(stack, [8.5, 64.4, 8.5], [0.0, 0.0, 0.0], 10);
        let item = game.dropped.last_mut().unwrap();
        item.random = sunrise_core::random::LegacyRandom::new(12345);
        let mut level = game.world.random.clone();
        game.server.ticked_entities_through = u32::MAX;
        game.tick_items();
        let item = game.dropped.last().unwrap();
        // Up is the nearest open side. Independent Java Random(12345):
        // nextFloat()*0.2F+0.1F, widened before multiplying by widened 0.98F.
        assert_eq!(item.velocity.y, f64::from_bits(0x3fc5_9ef4_570f_5500));
        assert_eq!(item.random.clone().next_float().to_bits(), 0x3f03_61b3);
        assert_eq!(game.world.random.next_float(), level.next_float());
    }
}

#[cfg(test)]
pub(crate) mod expire_tests {
    use mod_api::{
        Compound, EntityLeaveLevelEvent, EntityTickPostEvent, EntityTickPreEvent, HostSide,
        HostTransaction, ItemExpireEvent, ModEntry, ServerHost, Tag,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicI32, AtomicU32, Ordering},
    };
    use sunrise_core::{geometry::Vec3, position::ChunkPos};
    use sunrise_gameplay::game::ServerGame;
    use sunrise_gameplay::game::native_entity_refs::tests::{
        prepare_entity_ticking_neighborhood, with_game,
    };
    use sunrise_gameplay_common::game::Dim;
    use sunrise_gameplay_common::game::GameGameExt1 as _;
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;

    const AT: [f64; 3] = [2.5, 64.0, 2.5];

    fn expiry_level(game: &mut ServerGame<'_>) {
        game.enter_level(Dim::Overworld).unwrap();
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        assert!(game.entity_ticking_area()(2, 2));
    }

    fn drop_at_age(game: &mut ServerGame<'_>, name: &str, age: i32) -> u32 {
        game.push_item_entity(ItemStack::new(name, 1), AT, Some([0.0; 3]), 0, None);
        let item = game.dropped.last_mut().unwrap();
        item.age = age;
        let id = item.id;
        game.native_joins_flush(false, 0);
        game.server.ticked_entities_through = id;
        id
    }

    #[derive(Default)]
    struct ExpireLog {
        seen: Vec<(u32, Option<[f64; 3]>, Option<i32>)>,
        leaves: Vec<(u32, String)>,
    }

    struct ExpireListener {
        extra: Arc<AtomicI32>,
        log: Arc<Mutex<ExpireLog>>,
        record_leave: bool,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ExpireListener {
        fn item_expire(&self, host: &mut dyn ServerHost, event: &mut ItemExpireEvent) {
            let entity = event.item.entity();
            let pos = entity.position(host);
            let age = event.item.age(host);
            self.log.lock().unwrap().seen.push((entity.id(), pos, age));
            event.extra_life = event
                .extra_life
                .wrapping_add(self.extra.load(Ordering::Relaxed));
        }

        fn entity_leave_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityLeaveLevelEvent<'_>,
        ) {
            if self.record_leave {
                self.log
                    .lock()
                    .unwrap()
                    .leaves
                    .push((event.entity, event.kind.to_owned()));
            }
        }
    }

    /// NeoForge `EventHooks.onItemExpire` / `ItemEntity.tick`: listeners see the live
    /// resident item, add to `extra_life`, extend `lifespan` without rewinding `age`,
    /// clamp `lifespan` to `Short.MAX_VALUE - 1` (32766), and fire `entity_leave_level`
    /// once at the removal site when discarded.
    #[test]
    #[ignore = "daily"]
    fn item_expire_extends_lifespan_without_rewinding_age_and_caps_at_32766() {
        let log = Arc::new(Mutex::new(ExpireLog::default()));
        let extra_a = Arc::new(AtomicI32::new(60));
        let extra_b = Arc::new(AtomicI32::new(40));
        let mods: mod_api::Mods = vec![
            ModEntry::new(
                "item-expire-a",
                Box::new(ExpireListener {
                    extra: extra_a.clone(),
                    log: log.clone(),
                    record_leave: true,
                }),
            ),
            ModEntry::new(
                "item-expire-b",
                Box::new(ExpireListener {
                    extra: extra_b.clone(),
                    log: log.clone(),
                    record_leave: false,
                }),
            ),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                let id = drop_at_age(game, "minecraft:diamond", 5999);

                // First expiry at age 6000: listeners add 60 + 40 -> lifespan 6100, age stays 6000.
                game.tick_items();
                let item = game
                    .dropped
                    .iter()
                    .find(|item| item.id == id)
                    .expect("extended item survives");
                assert_eq!(item.age, 6000);
                assert_eq!(item.lifespan, 6100);
                {
                    let guard = log.lock().unwrap();
                    assert_eq!(guard.seen.len(), 2);
                    for &(seen_id, pos, age) in &guard.seen {
                        assert_eq!(seen_id, id);
                        assert!(pos.is_some(), "item entity is resident during item_expire");
                        assert_eq!(age, Some(6000));
                    }
                    assert!(guard.leaves.is_empty());
                }

                // The sum 6100 + 30000 fits a Java int but exceeds Short.MAX_VALUE - 1.
                extra_a.store(30000, Ordering::Relaxed);
                extra_b.store(0, Ordering::Relaxed);
                game.dropped.last_mut().unwrap().age = 6099;
                game.tick_items();
                let item = game
                    .dropped
                    .iter()
                    .find(|item| item.id == id)
                    .expect("clamped item survives at age 6100 < 32766");
                assert_eq!(item.age, 6100);
                assert_eq!(item.lifespan, 32766);

                // Reaching 32766 discards even when listeners still request extra life;
                // entity_leave_level fires immediately at removal and does not duplicate on flush.
                game.dropped.last_mut().unwrap().age = 32765;
                game.tick_items();
                assert!(!game.dropped.iter().any(|item| item.id == id));
                assert_eq!(
                    log.lock().unwrap().leaves.as_slice(),
                    &[(id, "minecraft:item".to_owned())]
                );
                game.native_leaves_flush();
                assert_eq!(
                    log.lock().unwrap().leaves.len(),
                    1,
                    "native_leaves_flush does not duplicate entity_leave_level"
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn item_expire_discards_after_insufficient_negative_or_wrapping_extensions() {
        let log = Arc::new(Mutex::new(ExpireLog::default()));
        let extra = Arc::new(AtomicI32::new(0));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "item-expire-discard",
            Box::new(ExpireListener {
                extra: extra.clone(),
                log: log.clone(),
                record_leave: true,
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                for (age, lifespan, extension) in [
                    (6499, 6000, 100),      // Age 6500 still exceeds the extended lifespan of 6100.
                    (5999, 6000, -500),     // Age 6000 exceeds the shortened lifespan of 5500.
                    (6099, 6100, i32::MAX), // Java int sum -2147477549 clamps to zero.
                ] {
                    extra.store(extension, Ordering::Relaxed);
                    let id = drop_at_age(game, "minecraft:stone", age);
                    game.dropped.last_mut().unwrap().lifespan = lifespan;
                    game.tick_items();
                    assert!(game.dropped.is_empty(), "extension {extension}");
                    assert_eq!(
                        std::mem::take(&mut log.lock().unwrap().leaves),
                        [(id, "minecraft:item".to_owned())]
                    );
                    game.native_leaves_flush();
                    assert!(log.lock().unwrap().leaves.is_empty());
                }
            });
        });
    }

    #[derive(Default)]
    struct LeaveObservation {
        next_drop: AtomicU32,
        heard: Mutex<Vec<(u32, Option<i32>)>>,
    }

    struct LeaveObserver(Arc<LeaveObservation>);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for LeaveObserver {
        fn entity_leave_level(
            &self,
            host: &mut dyn ServerHost,
            event: &mut EntityLeaveLevelEvent<'_>,
        ) {
            let next_age = host
                .entity(self.0.next_drop.load(Ordering::Relaxed))
                .and_then(|entity| host.item_entity(entity))
                .and_then(|item| item.age(host));
            self.0.heard.lock().unwrap().push((event.entity, next_age));
        }
    }

    #[test]
    #[ignore = "daily"]
    fn expiry_leave_runs_before_the_next_due_drop_ticks() {
        let observed = Arc::new(LeaveObservation::default());
        // With no item_expire or per-entity tick listener, only removal itself lends the list.
        let mods: mod_api::Mods = vec![ModEntry::new(
            "expiry-leave-order",
            Box::new(LeaveObserver(observed.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                let first = drop_at_age(game, "minecraft:stone", 5999);
                let next = drop_at_age(game, "minecraft:diamond", 5999);
                observed.next_drop.store(next, Ordering::Relaxed);

                game.tick_items();
                assert_eq!(
                    *observed.heard.lock().unwrap(),
                    [(first, Some(5999)), (next, None)]
                );
                assert!(game.dropped.is_empty());
                game.native_leaves_flush();
                assert_eq!(observed.heard.lock().unwrap().len(), 2);
            });
        });
    }

    #[derive(Default)]
    struct DiscardLog {
        drops: [u32; 2],
        commit: Option<bool>,
        discarded: Option<[bool; 2]>,
        expires: Vec<u32>,
        leaves: Vec<u32>,
    }

    struct DiscardOnExpiry(Arc<Mutex<DiscardLog>>);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for DiscardOnExpiry {
        fn item_expire(&self, host: &mut dyn ServerHost, event: &mut ItemExpireEvent) {
            let id = event.item.entity().id();
            let (drops, commit) = {
                let mut log = self.0.lock().unwrap();
                log.expires.push(id);
                (log.drops, log.commit)
            };
            // Without the discard both items live long enough to reach pickup.
            event.extra_life = 20;
            if id != drops[0] {
                return;
            }
            let mut discarded = [false; 2];
            let mut discard = |host: &mut dyn ServerHost| {
                discarded =
                    drops.map(|id| host.entity(id).is_some_and(|entity| entity.discard(host)));
            };
            if let Some(commit) = commit {
                let _ = host.transaction(|host, _| {
                    discard(host);
                    if commit { Ok(()) } else { Err(()) }
                });
            } else {
                discard(host);
            }
            self.0.lock().unwrap().discarded = Some(discarded);
        }

        fn entity_leave_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityLeaveLevelEvent<'_>,
        ) {
            self.0.lock().unwrap().leaves.push(event.entity);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn expiry_listener_discards_are_swept_before_more_ticks_or_pickup() {
        let log = Arc::new(Mutex::new(DiscardLog::default()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "discard-on-expiry",
            Box::new(DiscardOnExpiry(log.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(AT[0], AT[1], AT[2])).unwrap());
                game.update_chunk_tickets();
                assert!(game.entity_ticking_area()(2, 2));

                for commit in [None, Some(true), Some(false)] {
                    game.inventory.fill(None);
                    let drops = [
                        drop_at_age(game, "minecraft:apple", 5999),
                        drop_at_age(game, "minecraft:diamond", 5999),
                    ];
                    *log.lock().unwrap() = DiscardLog {
                        drops,
                        commit,
                        ..DiscardLog::default()
                    };

                    game.tick_items();
                    assert!(game.dropped.is_empty(), "{commit:?}");
                    let picked_up: Vec<&str> = game
                        .inventory
                        .iter()
                        .flatten()
                        .map(|s| s.item.as_ref())
                        .collect();
                    {
                        let log = log.lock().unwrap();
                        assert_eq!(log.discarded, Some([true, true]), "{commit:?}");
                        if commit == Some(false) {
                            assert_eq!(log.expires, drops);
                            assert_eq!(picked_up, ["minecraft:apple", "minecraft:diamond"]);
                        } else {
                            assert_eq!(log.expires, [drops[0]], "{commit:?}");
                            assert_eq!(log.leaves, drops, "{commit:?}");
                            assert!(picked_up.is_empty(), "{commit:?}");
                        }
                    }
                    game.native_leaves_flush();
                    assert_eq!(log.lock().unwrap().leaves, drops, "{commit:?}");
                }
            });
        });
    }

    #[derive(Default)]
    struct AloneDiscardLog {
        drops: [u32; 2],
        commit: Option<bool>,
        events: Vec<(&'static str, u32)>,
    }

    struct AloneDiscardListener(Arc<Mutex<AloneDiscardLog>>);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for AloneDiscardListener {
        fn entity_tick_pre(
            &self,
            _: HostSide<'_>,
            event: &mut EntityTickPreEvent<'_>,
        ) -> mod_api::Flow {
            self.0.lock().unwrap().events.push(("pre", event.entity));
            mod_api::Flow::Continue
        }

        fn entity_tick_post(&self, _: HostSide<'_>, event: &mut EntityTickPostEvent<'_>) {
            self.0.lock().unwrap().events.push(("post", event.entity));
        }

        fn item_expire(&self, host: &mut dyn ServerHost, event: &mut ItemExpireEvent) {
            let id = event.item.entity().id();
            let (drops, commit) = {
                let mut log = self.0.lock().unwrap();
                log.events.push(("expire", id));
                (log.drops, log.commit)
            };
            event.extra_life = 20;
            if id != drops[0] {
                return;
            }
            let discard = |host: &mut dyn ServerHost| {
                for id in drops {
                    assert!(host.entity(id).is_some_and(|entity| entity.discard(host)));
                }
            };
            if let Some(commit) = commit {
                let _ = host.transaction(|host, _| {
                    discard(host);
                    if commit { Ok(()) } else { Err(()) }
                });
            } else {
                discard(host);
            }
        }

        fn entity_leave_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityLeaveLevelEvent<'_>,
        ) {
            self.0.lock().unwrap().events.push(("leave", event.entity));
        }
    }

    #[test]
    #[ignore = "daily"]
    fn expiry_listener_discards_while_told_alone_still_deliver_post_once() {
        let log = Arc::new(Mutex::new(AloneDiscardLog::default()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "alone-discard-on-expiry",
            Box::new(AloneDiscardListener(log.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                for commit in [None, Some(true)] {
                    let drops = [
                        drop_at_age(game, "minecraft:apple", 5999),
                        drop_at_age(game, "minecraft:diamond", 5999),
                    ];
                    *log.lock().unwrap() = AloneDiscardLog {
                        drops,
                        commit,
                        events: Vec::new(),
                    };

                    game.tick_items();
                    assert!(game.dropped.is_empty(), "{commit:?}");
                    assert_eq!(
                        log.lock().unwrap().events,
                        [
                            ("pre", drops[0]),
                            ("expire", drops[0]),
                            ("leave", drops[0]),
                            ("leave", drops[1]),
                            ("post", drops[0]),
                        ],
                        "{commit:?}"
                    );
                    game.native_leaves_flush();
                    assert_eq!(log.lock().unwrap().events.len(), 5, "{commit:?}");
                }
            });
        });
    }

    struct LifespanEditListener(Arc<Mutex<Option<bool>>>);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for LifespanEditListener {
        fn item_expire(&self, host: &mut dyn ServerHost, event: &mut ItemExpireEvent) {
            let edited = event
                .item
                .entity()
                .merge_nbt(host, Compound::from([("Lifespan".into(), Tag::Int(12000))]));
            *self.0.lock().unwrap() = Some(edited);
            event.extra_life = 1;
        }
    }

    /// NeoForge `ItemEntity.tick` evaluates `this.lifespan` before calling
    /// `EventHooks.onItemExpire(this)`: editing `Lifespan` during dispatch does not
    /// shift the base of `lifespan + extra_life`.
    #[test]
    #[ignore = "daily"]
    fn item_expire_adds_extra_life_to_pre_dispatch_lifespan() {
        let edited = Arc::new(Mutex::new(None));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "lifespan-edit-on-expiry",
            Box::new(LifespanEditListener(edited.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                let id = drop_at_age(game, "minecraft:stone", 5999);

                game.tick_items();
                assert_eq!(*edited.lock().unwrap(), Some(true));
                let item = game
                    .dropped
                    .iter()
                    .find(|item| item.id == id)
                    .expect("item survives at age 6000 < 6001");
                assert_eq!(item.age, 6000);
                assert_eq!(item.lifespan, 6001);
            });
        });
    }

    struct BurstListener {
        expired: Arc<Mutex<Vec<(u32, Option<u32>, Option<u32>)>>>,
        left: Arc<Mutex<Vec<(u32, bool)>>>,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for BurstListener {
        fn item_expire(&self, host: &mut dyn ServerHost, event: &mut ItemExpireEvent) {
            let mut count = None;
            let mut damage = None;
            event.item.with_stack(host, |stack| {
                count = Some(stack.count());
                damage = Some(stack.damage());
            });
            self.expired
                .lock()
                .unwrap()
                .push((event.item.entity().id(), count, damage));
        }

        fn entity_leave_level(
            &self,
            host: &mut dyn ServerHost,
            event: &mut EntityLeaveLevelEvent<'_>,
        ) {
            let still_resident = host.entity(event.entity).is_some();
            self.left
                .lock()
                .unwrap()
                .push((event.entity, still_resident));
        }
    }

    /// Expiring K of N component-bearing drops in one tick lends each stack through
    /// `ItemEntityRef::with_stack`, preserves per-drop `entity_leave_level` departure
    /// order while hiding already-departed rows, and avoids O(KN) list compaction and
    /// `ItemCells` rebuilds.
    #[test]
    #[ignore = "daily"]
    fn expiry_burst_lends_stacks_preserves_order_and_reuses_cells() {
        let expired = Arc::new(Mutex::new(Vec::with_capacity(512)));
        let left = Arc::new(Mutex::new(Vec::with_capacity(512)));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "expiry-burst-listener",
            Box::new(BurstListener {
                expired: expired.clone(),
                left: left.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                let mut expected_expired = Vec::with_capacity(500);
                let mut expected_survivors = Vec::with_capacity(500);
                for idx in 0..1000usize {
                    let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
                    stack.damage = 42;
                    let data = stack.data_mut();
                    data.custom_name = Some(Tag::String("Expiring Blade".into()));
                    data.enchantments.push(("minecraft:sharpness".into(), 5));
                    let x = 2.5 + (idx % 10) as f64 * 1.1;
                    let z = 2.5 + ((idx / 10) % 10) as f64 * 1.1;
                    game.push_item_entity(stack, [x, 64.0, z], Some([0.0; 3]), 0, None);
                    let item = game.dropped.last_mut().unwrap();
                    let id = item.id;
                    if idx % 2 == 0 {
                        item.age = 5999;
                        expected_expired.push(id);
                    } else {
                        item.age = 0;
                        expected_survivors.push(id);
                    }
                    game.server.ticked_entities_through = id;
                }
                game.native_joins_flush(false, 0);

                let start = std::time::Instant::now();
                let ((), allocs) = sunrise_threads::allocation_count::measure(|| {
                    game.tick_items();
                });
                let elapsed = start.elapsed();
                eprintln!(
                    "ROW5_AFTER total_ms={:.3} us_per_expiry={:.1} allocs={allocs}",
                    elapsed.as_secs_f64() * 1000.0,
                    elapsed.as_secs_f64() * 1e6 / 500.0,
                );

                let exp = expired.lock().unwrap();
                assert_eq!(exp.len(), 500);
                for (&(id, count, damage), &want) in exp.iter().zip(&expected_expired) {
                    assert_eq!(id, want);
                    assert_eq!(count, Some(1));
                    assert_eq!(damage, Some(42));
                }
                let lft = left.lock().unwrap();
                assert_eq!(lft.len(), 500);
                for (&(id, still_resident), &want) in lft.iter().zip(&expected_expired) {
                    assert_eq!(id, want);
                    assert!(
                        !still_resident,
                        "departed item {id} must not resolve during entity_leave_level"
                    );
                }
                let survivors: Vec<u32> = game.dropped.iter().map(|item| item.id).collect();
                assert_eq!(survivors, expected_survivors);
                // Baseline before Row 5 was 404,829 allocations (O(KN) ItemCells rebuilds + stack clones).
                assert!(
                    allocs < 10_000,
                    "expiry burst allocated {allocs} times, expected O(N) without per-expiry rebuilds",
                );
            });
        });
    }

    struct CountingExpireListener {
        counts: Arc<Mutex<Vec<(u32, usize)>>>,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CountingExpireListener {
        fn item_expire(&self, host: &mut dyn ServerHost, event: &mut ItemExpireEvent) {
            let level = host.level("minecraft:overworld").unwrap();
            let mut seen_drops = 0usize;
            let _ = level.visit_entities_in(
                host,
                [0.0, 60.0, 0.0],
                [20.0, 70.0, 20.0],
                &mut |entity| {
                    if entity.kind(host).and_then(|kind| kind.id(host)).as_deref()
                        == Some("minecraft:item")
                    {
                        seen_drops += 1;
                    }
                    std::ops::ControlFlow::Continue(())
                },
            );
            self.counts
                .lock()
                .unwrap()
                .push((event.item.entity().id(), seen_drops));
        }
    }

    /// An `item_expire` listener that queries drops in the level through the host
    /// does not see an earlier drop that already expired in the same pass.
    #[test]
    #[ignore = "daily"]
    fn item_expire_host_query_excludes_earlier_expired_drops() {
        let counts = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "expire-query-listener",
            Box::new(CountingExpireListener {
                counts: counts.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                expiry_level(game);
                let mut ids = Vec::new();
                for (idx, age) in [5999, 5999, 0].into_iter().enumerate() {
                    let stack = ItemStack::new("minecraft:diamond_sword", 1);
                    let x = 2.5 + idx as f64 * 2.0;
                    game.push_item_entity(stack, [x, 64.0, 2.5], Some([0.0; 3]), 0, None);
                    let item = game.dropped.last_mut().unwrap();
                    item.age = age;
                    ids.push(item.id);
                    game.server.ticked_entities_through = item.id;
                }
                game.native_joins_flush(false, 0);

                game.tick_items();

                // First expiring drop sees all 3 live drops; second expiring drop sees only 2
                // (itself + the non-expiring survivor), never the first already-expired drop.
                assert_eq!(*counts.lock().unwrap(), vec![(ids[0], 3), (ids[1], 2)]);
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(game.dropped[0].id, ids[2]);
            });
        });
    }
}

#[cfg(test)]
#[path = "tests/thrown_items_tests.rs"]
pub(crate) mod thrown_items_tests;
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
