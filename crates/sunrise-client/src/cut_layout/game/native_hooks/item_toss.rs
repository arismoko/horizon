#![cfg(test)]
use sunrise_items::items::ItemStack;

#[cfg(test)]
pub(crate) mod item_toss_tests {
    use crate::game::native_hooks::item_toss::*;
    use mod_api::{Flow, Hooks, ItemTossEvent, ModEntry, ServerHost};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    /// Counts the tosses it hears. A thrown dirt block loses its pickup
    /// delay through the handle; anything else is canceled.
    struct Tosser(Arc<Mutex<u32>>);
    #[mod_api::__subscriptions]
    impl Hooks for Tosser {
        fn item_toss(&self, host: &mut dyn ServerHost, event: &mut ItemTossEvent) -> Flow {
            *self.0.lock().unwrap() += 1;
            let is_dirt = event
                .item
                .with_stack(host, |stack| {
                    mod_api::__private::item_key_id(stack.item()) == Some("minecraft:dirt")
                })
                .expect("the tossed item reads");
            if is_dirt {
                assert!(event.item.set_no_pickup_delay(host));
                Flow::Continue
            } else {
                Flow::Cancel
            }
        }
    }

    /// A toss asks `item_toss` with the item entity before it joins: what
    /// the listener changes stays, and a cancel leaves no item entity. The
    /// random scatter of a death's drops is not a toss and is never asked.
    #[test]
    #[ignore = "daily"]
    fn item_toss_hears_the_item_entity_and_a_cancel_leaves_none() {
        let heard = Arc::new(Mutex::new(0));
        let mods = vec![ModEntry::new("toss_probe", Box::new(Tosser(heard.clone())))];
        with_mods(mods.into(), || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.player = Some(
                    sunrise_sim::motion::PlayerMotion::new(
                        game.motion,
                        sunrise_core::geometry::Vec3::ZERO,
                    )
                    .unwrap(),
                );
                let eye = [0.5, 65.0, 0.5];
                let before = game.dropped.len();
                let stone = ItemStack::new("minecraft:stone", 1);
                assert!(game.throw_from_eyes(stone, eye, false, true).is_none());
                assert_eq!(*heard.lock().unwrap(), 1);
                assert_eq!(game.dropped.len(), before, "a canceled toss leaves no item");
                let dirt = ItemStack::new("minecraft:dirt", 1);
                let id = game.throw_from_eyes(dirt, eye, false, true).unwrap();
                assert_eq!(*heard.lock().unwrap(), 2);
                let thrown = game.dropped.iter().find(|d| d.id == id).unwrap();
                assert_eq!(thrown.pickup_delay, 0, "the listener's change stays");
                let dirt = ItemStack::new("minecraft:dirt", 1);
                assert!(game.throw_from_eyes(dirt, eye, true, false).is_some());
                assert_eq!(*heard.lock().unwrap(), 2);
            });
        });
    }

    /// Spawns a stone where the toss is, then cancels the toss; records the
    /// tossed id and every item join and leave.
    #[derive(Default)]
    struct SpawnSeen {
        pub tossed: Mutex<Option<u32>>,
        spawned: Mutex<Option<u32>>,
        joins: Mutex<Vec<u32>>,
        leaves: Mutex<Vec<u32>>,
        order: Mutex<Vec<&'static str>>,
        veto_join: bool,
    }
    struct SpawnThenCancel(Arc<SpawnSeen>);
    #[mod_api::__subscriptions]
    impl Hooks for SpawnThenCancel {
        fn item_toss(&self, host: &mut dyn ServerHost, event: &mut ItemTossEvent) -> Flow {
            *self.0.tossed.lock().unwrap() = Some(event.item.entity().id());
            let level = host.level("minecraft:overworld").unwrap();
            let stone = mod_api::ItemStack::new(
                mod_api::__private::item_key("minecraft:stone").unwrap(),
                1,
            )
            .unwrap();
            let spawned = level.spawn_item(host, 0.5, 65.0, 0.5, stone, 0).unwrap();
            *self.0.spawned.lock().unwrap() = spawned.map(|e| e.id());
            Flow::Cancel
        }
        fn entity_join_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::EntityJoinLevelEvent<'_>,
        ) -> Flow {
            if event.kind == "minecraft:item" {
                self.0.joins.lock().unwrap().push(event.subject.id());
                self.0.order.lock().unwrap().push("join");
                if self.0.veto_join {
                    return Flow::Cancel;
                }
            }
            Flow::Continue
        }
        fn item_spawn(&self, _: &mut dyn ServerHost, _: &mut mod_api::ItemSpawnEvent<'_>) {
            self.0.order.lock().unwrap().push("spawn");
        }
        fn entity_leave_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::EntityLeaveLevelEvent<'_>,
        ) {
            self.0.leaves.lock().unwrap().push(event.entity);
        }
    }

    /// A toss listener that spawns another item flushes the pending joins,
    /// but not the toss it is still deciding: a cancel then leaves the toss
    /// unannounced (no join, no leave), and the listener's item joins.
    #[test]
    #[ignore = "daily"]
    fn a_listener_spawn_does_not_announce_the_toss_it_then_cancels() {
        check_listener_spawn(false);
    }

    #[test]
    #[ignore = "daily"]
    fn a_deferred_join_veto_announces_no_item_spawn() {
        check_listener_spawn(true);
    }

    fn check_listener_spawn(veto_join: bool) {
        let probe = Arc::new(SpawnSeen {
            veto_join,
            ..Default::default()
        });
        let mods = vec![ModEntry::new(
            "toss_spawn_probe",
            Box::new(SpawnThenCancel(probe.clone())),
        )];
        with_mods(mods.into(), || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.player = Some(
                    sunrise_sim::motion::PlayerMotion::new(
                        game.motion,
                        sunrise_core::geometry::Vec3::ZERO,
                    )
                    .unwrap(),
                );
                let dirt = ItemStack::new("minecraft:dirt", 1);
                assert!(
                    game.throw_from_eyes(dirt, [0.5, 65.0, 0.5], false, true)
                        .is_none()
                );
                game.native_joins_flush(false, 0);
                let tossed = probe.tossed.lock().unwrap().expect("the toss was asked");
                let spawned = probe
                    .spawned
                    .lock()
                    .unwrap()
                    .expect("the listener's item spawned");
                let joins = probe.joins.lock().unwrap().clone();
                assert!(!joins.contains(&tossed), "the canceled toss never joins");
                assert!(joins.contains(&spawned), "the listener's item joins");
                assert!(!probe.leaves.lock().unwrap().contains(&tossed));
                assert!(game.dropped.iter().all(|d| d.id != tossed));
                if veto_join {
                    assert_eq!(*probe.order.lock().unwrap(), ["join"]);
                    assert!(game.dropped.iter().all(|d| d.id != spawned));
                } else {
                    assert_eq!(*probe.order.lock().unwrap(), ["join", "spawn"]);
                    assert!(game.dropped.iter().any(|d| d.id == spawned));
                }
            });
        });
    }
}
