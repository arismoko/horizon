#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_world::teleport_consume::TeleportConsume;
#[cfg(test)]
pub(crate) mod teleport_hook_tests {
    use sunrise_core::random::RandomSource;
    use sunrise_gameplay::game::server_side::consume::teleport_hook_tests::ConsumptionProbe;
    use sunrise_gameplay_common::game::tables::GameItems;

    use sunrise_gameplay_common::game::api_values::test_snapshot;

    use super::*;

    use crate::game::native_entity_refs::tests::with_game;

    use mod_api::ModEntry;

    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    use sunrise_sim::motion::PlayerMotion;

    #[test]
    #[ignore = "daily"]
    fn consumption_hook_keeps_the_stack_before_use_and_cancellation_stops_attempts() {
        run_consumption_hook_keeps_the_stack_before_use_and_cancellation_stops_attempts();
    }
    pub(crate) fn run_consumption_hook_keeps_the_stack_before_use_and_cancellation_stops_attempts()
    {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let moved = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "consumed-stack-proof",
            Box::new(ConsumptionProbe(Arc::clone(&heard), Arc::clone(&moved))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let at = Vec3::new(0.0, 65.0, 0.0);
                game.player = Some(PlayerMotion::new(game.motion, at).unwrap());
                // This body belongs to the guest record installed below, never
                // to a separately admitted local player with the same runtime ID.
                game.player.as_mut().unwrap().entity_id = 2;
                game.server.lan_acting = Some(12);
                game.server.lan_acting_uuid = Some(7);
                game.server.lan_acting_name = Some("ConsumedStackTest".into());
                game.server.lan_acting_entity = Some(2);
                game.server.lan_acting_permission = Some(4);
                sunrise_gameplay::game::native_entity_refs::tests::bind_acting_guest(game);
                game.creative = false;
                let mut consumed = ItemStack::new("minecraft:chorus_fruit", 7);
                consumed.damage = 3;
                consumed.data_mut().set_loot_name(
                    "minecraft:custom_name",
                    &serde_json::json!("Captured fruit"),
                );
                let snapshot = test_snapshot(&consumed);
                game.common.inventory[game.common.selected] = Some(consumed.clone());
                let info = game.items().get(&consumed.item).unwrap().clone();
                let consumable = consumed.consumable(Some(&info)).unwrap().clone();
                game.finish_consuming(&info, &consumable, &consumed);
                assert_eq!(
                    heard.lock().unwrap().as_slice(),
                    std::slice::from_ref(&snapshot)
                );
                assert_eq!(
                    game.held().unwrap().count,
                    6,
                    "veto stops teleport, not consumption"
                );
                assert_eq!(game.player.as_ref().unwrap().position, at);

                // The source stack is explicit, even if the hand has since changed.
                game.common.inventory[game.common.selected] =
                    Some(ItemStack::new("minecraft:stone", 1));
                let mut expected_random = game.player.as_ref().unwrap().random.clone();
                for _ in 0..3 {
                    expected_random.next_double();
                }
                assert!(!game.teleport_randomly(
                    &TeleportConsume {
                        diameter: 16.0,
                        directional_particles: true,
                    },
                    &consumed
                ));
                assert_eq!(*heard.lock().unwrap(), [snapshot.clone(), snapshot]);
                assert_eq!(
                    game.player.as_mut().unwrap().random.next_long(),
                    expected_random.next_long(),
                    "veto ends the effect after the first candidate's three draws"
                );
                assert_eq!(game.player.as_ref().unwrap().position, at);

                moved.store(true, Ordering::SeqCst);
                expected_random = game.player.as_ref().unwrap().random.clone();
                for _ in 0..3 {
                    expected_random.next_double();
                }
                assert!(!game.teleport_randomly(
                    &TeleportConsume {
                        diameter: 16.0,
                        directional_particles: true,
                    },
                    &consumed
                ));
                assert_eq!(
                    game.player.as_ref().unwrap().position,
                    Vec3::new(3.0, 70.0, 3.0),
                    "a listener's direct move invalidates the consumption target"
                );
                assert_eq!(
                    game.player.as_mut().unwrap().random.next_long(),
                    expected_random.next_long()
                );
            })
        });
    }
}

#[cfg(test)]
pub(crate) mod fox_consumption_hook_tests {
    use super::*;
    use mod_api::{Flow, Hand, Hooks, ItemConsumptionTeleportEvent, ModEntry, ServerHost};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    };
    use sunrise_core::random::RandomSource;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::api_values::test_snapshot;
    use sunrise_sim::mobs::{Mob, fox};

    struct Probe {
        mode: Arc<AtomicU8>,
        heard: Arc<Mutex<Vec<(u32, mod_api::ItemStack)>>>,
        spawned: Arc<Mutex<Vec<u32>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn item_consumption_teleport(
            &self,
            host: &mut dyn ServerHost,
            event: &mut ItemConsumptionTeleportEvent<'_>,
        ) -> Flow {
            self.heard
                .lock()
                .unwrap()
                .push((event.consumer.entity().id(), event.consumed.clone()));
            assert_eq!(event.consumer.entity().position(host), Some(event.from));
            match self.mode.load(Ordering::SeqCst) {
                0 => Flow::Cancel,
                1 => {
                    *event.target = [4.5, 70.0, 4.5];
                    Flow::Continue
                }
                2 => {
                    assert!(
                        event
                            .consumer
                            .set_hand_item(
                                host,
                                Hand::Main,
                                Some(test_snapshot(&ItemStack::new("minecraft:stone", 1)))
                            )
                            .unwrap()
                    );
                    Flow::Continue
                }
                3 => {
                    assert!(
                        event
                            .consumer
                            .entity()
                            .teleport_to(host, [3.5, 64.0, 3.5], None)
                    );
                    Flow::Continue
                }
                4 => {
                    assert!(event.consumer.entity().discard(host));
                    Flow::Continue
                }
                5 => {
                    match event.consumer.entity().id() {
                        200 => {
                            assert_eq!(
                                event.from,
                                [8.5, 64.0, 8.5],
                                "the hook precedes this fox's movement"
                            );
                            // Full owning-host access, before this fox moves and
                            // before any later mob's AI/consumption step.
                            let next = host.entity(201).unwrap();
                            assert_eq!(next.position(host), Some([11.5, 64.0, 8.5]));
                            let next_living = host.living(next).unwrap();
                            assert_eq!(
                                next_living.with_hand_item(host, Hand::Main, |stack| stack.count()),
                                Some(7)
                            );
                            assert!(
                                next_living
                                    .set_hand_item(
                                        host,
                                        Hand::Main,
                                        Some(test_snapshot(&ItemStack::new("minecraft:stone", 1)))
                                    )
                                    .unwrap()
                            );
                            assert!(next.teleport_to(host, [12.5, 64.0, 8.5], None));
                            assert!(host.entity(202).unwrap().discard(host));
                            let level = host.level("minecraft:overworld").unwrap();
                            let spawned = level
                                .summon(host, "minecraft:cow", [2.5, 64.0, 2.5], Default::default())
                                .unwrap()
                                .unwrap();
                            self.spawned.lock().unwrap().push(spawned.id());
                            *event.target = [4.5, 70.0, 4.5];
                            Flow::Continue
                        }
                        204 => {
                            // The earlier fox resumed before this one, and the
                            // listener's unrelated actor writes survived it.
                            let first = host.living(host.entity(200).unwrap()).unwrap();
                            assert_eq!(
                                first.with_hand_item(host, Hand::Main, |stack| stack.count()),
                                Some(6)
                            );
                            let next = host.living(host.entity(201).unwrap()).unwrap();
                            assert_eq!(
                                next.with_hand_item(host, Hand::Main, |stack| stack.project())
                                    .flatten(),
                                Some(test_snapshot(&ItemStack::new("minecraft:stone", 1)))
                            );
                            assert!(host.entity(202).is_none());
                            Flow::Cancel
                        }
                        other => panic!("unexpected consumption by {other}"),
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn fox_consumption_adapter_uses_full_host_and_preserves_listener_writes() {
        run_fox_consumption_adapter_uses_full_host_and_preserves_listener_writes();
    }

    pub(crate) fn run_fox_consumption_adapter_uses_full_host_and_preserves_listener_writes() {
        let mode = Arc::new(AtomicU8::new(0));
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "fox-consumption-proof",
            Box::new(Probe {
                mode: Arc::clone(&mode),
                heard: Arc::clone(&heard),
                spawned: Arc::new(Mutex::new(Vec::new())),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                for x in 0..16 {
                    for z in 0..16 {
                        game.world.set(x, 63, z, stone).unwrap();
                    }
                }
                let ty = game.res.mob_types().get("minecraft:fox").unwrap().clone();
                for action in 0..5 {
                    mode.store(action, Ordering::SeqCst);
                    let id = 60 + u32::from(action);
                    let from = Vec3::new(8.5, 64.0, 8.5);
                    let mut actor = Mob::new(id, ty.clone(), from, 0.0);
                    actor.ground.on_ground = true;
                    actor.fall_distance = 9.0;
                    actor.extra.ints[1] = 600;
                    let mut stack = ItemStack::new("minecraft:chorus_fruit", 7);
                    stack
                        .data_mut()
                        .set_loot_name("minecraft:custom_name", &serde_json::json!("Fox fruit"));
                    actor.extra.set_main_hand_stack(Some(stack.clone()));
                    let mut events = Vec::new();
                    let decision = fox::ai_step_until_consumption(
                        &mut actor,
                        &game.mob_world_base(&[]),
                        &mut events,
                    )
                    .expect("pause before effect draws and mouth replacement");
                    assert!(events.is_empty());
                    let mut expected_random = actor.random.clone();
                    // Independent Java Random state-advance budget before the
                    // teleport: emitParticlesAndSounds spends 1 boolean plus
                    // 3 floats for sound, 16 crumbs spend 4 floats each, and
                    // FoodProperties' sound triangle spends 2 floats: 70
                    // advances total. Discarded nextFloat advances the same
                    // 48-bit LCG state once, without copying emission logic.
                    for _ in 0..70 {
                        expected_random.next_float();
                    }
                    // The first teleport attempt consumes three doubles
                    // before any listener can cancel or replace the actor.
                    for _ in 0..3 {
                        expected_random.next_double();
                    }
                    game.mobs.push(actor);
                    let applied = game.apply_fox_consumption(&decision);
                    assert_eq!(applied, action <= 1);
                    assert_eq!(
                        heard.lock().unwrap().last(),
                        Some(&(id, test_snapshot(&stack)))
                    );
                    let actor = game.mobs.iter_mut().find(|m| m.id == id && !m.removed);
                    match action {
                        0 => {
                            let m = actor.unwrap();
                            assert_eq!(m.position, from);
                            assert_eq!(m.extra.main_hand_stack().unwrap().count, 6);
                            assert_eq!(m.extra.ints[1], 0);
                            assert_eq!(m.random.next_long(), expected_random.next_long());
                        }
                        1 => {
                            let m = actor.unwrap();
                            assert_eq!(m.position, Vec3::new(4.5, 64.0, 4.5));
                            assert_eq!(m.extra.main_hand_stack().unwrap().count, 6);
                            assert_eq!(m.fall_distance, 0.0);
                            assert_eq!(m.random.next_long(), expected_random.next_long());
                        }
                        2 => {
                            let m = actor.unwrap();
                            assert_eq!(m.position, from);
                            assert_eq!(
                                &*m.extra.main_hand_stack().unwrap().item,
                                "minecraft:stone"
                            );
                            assert_eq!(m.random.next_long(), expected_random.next_long());
                        }
                        3 => {
                            let m = actor.unwrap();
                            assert_eq!(m.position, Vec3::new(3.5, 64.0, 3.5));
                            assert_eq!(m.extra.main_hand_stack(), Some(&stack));
                            assert_eq!(m.random.next_long(), expected_random.next_long());
                        }
                        4 => assert!(actor.is_none()),
                        _ => unreachable!(),
                    }
                }
                assert_eq!(
                    heard.lock().unwrap().len(),
                    5,
                    "one callback per first attempt"
                );
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn fox_tick_pauses_before_movement_and_next_actor_and_preserves_full_host_writes() {
        run_fox_tick_pauses_before_movement_and_next_actor_and_preserves_full_host_writes();
    }

    pub(crate) fn run_fox_tick_pauses_before_movement_and_next_actor_and_preserves_full_host_writes()
     {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let spawned = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "fox-tick-consumption-proof",
            Box::new(Probe {
                mode: Arc::new(AtomicU8::new(5)),
                heard: Arc::clone(&heard),
                spawned: Arc::clone(&spawned),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                // Legacy summon checks the actual server owner as well as
                // the opened native lifecycle, like the production server does.
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                for x in 0..16 {
                    for z in 0..16 {
                        game.world.set(x, 63, z, stone).unwrap();
                    }
                }
                game.server.next_entity = 300;
                let ty = game.res.mob_types().get("minecraft:fox").unwrap().clone();
                for (id, x) in [(200, 8.5), (201, 11.5), (202, 13.5), (204, 6.5)] {
                    let mut actor = Mob::new(id, ty.clone(), Vec3::new(x, 64.0, 8.5), 0.0);
                    actor.persistent = true;
                    actor.ground.on_ground = true;
                    actor.extra.ints[1] = 600;
                    actor
                        .extra
                        .set_main_hand_stack(Some(ItemStack::new("minecraft:chorus_fruit", 7)));
                    if id == 200 {
                        actor.velocity = Vec3::new(0.2, 0.0, 0.0);
                    }
                    game.mobs.push(actor);
                }
                game.tick_creature_entities().unwrap();
                let heard = heard.lock().unwrap();
                assert_eq!(
                    heard.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
                    [200, 204]
                );
                assert!(heard.iter().all(|(_, stack)| stack.count() == 7));
                let first = game.mobs.iter().find(|m| m.id == 200).unwrap();
                assert_eq!(first.age, 1, "base tick must not repeat after the callback");
                assert_eq!(
                    first.previous,
                    Vec3::new(4.5, 64.0, 4.5),
                    "the normal teleport resets interpolation before resumed movement"
                );
                assert_eq!(first.extra.main_hand_stack().unwrap().count, 6);
                assert!(
                    first.position.x < 6.0,
                    "movement resumes at the redirected landing"
                );
                let next = game.mobs.iter().find(|m| m.id == 201).unwrap();
                assert_eq!(next.age, 1);
                assert_eq!(
                    &*next.extra.main_hand_stack().unwrap().item,
                    "minecraft:stone"
                );
                assert_eq!(next.previous, Vec3::new(12.5, 64.0, 8.5));
                assert!(!game.mobs.iter().any(|m| m.id == 202));
                let last = game.mobs.iter().find(|m| m.id == 204).unwrap();
                assert_eq!(last.age, 1);
                assert_eq!(last.extra.main_hand_stack().unwrap().count, 6);
                let spawned = spawned.lock().unwrap();
                assert_eq!(spawned.len(), 1);
                let cow = game.mobs.iter().find(|m| m.id == spawned[0]).unwrap();
                assert_eq!(
                    cow.age, 0,
                    "a listener's new entity waits until the next batch"
                );
            })
        });
    }
}
