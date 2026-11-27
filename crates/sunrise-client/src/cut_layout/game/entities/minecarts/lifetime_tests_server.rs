use crate::game::minecarts::lifetime_tests::CancelCartDismount;
use crate::game::minecarts::lifetime_tests::VALUE;
use crate::game::minecarts::lifetime_tests::assert_vehicle_residency_retired;
use std::sync::Arc;
use std::sync::Mutex;
use sunrise_core::geometry::Vec3;
use sunrise_core::{position::ChunkPos, random::RandomSource};
use sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood;
use sunrise_gameplay_common::game::minecarts::CartKind;
use sunrise_gameplay_common::game::minecarts::Minecart;
use sunrise_mod_runtime::attachments::{
    Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
};
use sunrise_sim::sprint::Keys;

#[test]
#[ignore = "daily"]
fn a_bound_cart_survives_the_parked_level_swap() {
    bound_cart_case(CartKind::Rideable, |game| {
        game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
            .unwrap();
        assert!(game.carts.is_empty());
        assert_eq!(
            game.server
                .levels
                .0
                .parked_mut(sunrise_gameplay_common::game::Dim::Overworld)
                .unwrap()
                .carts[0]
                .id,
            71
        );
        game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
            .unwrap();
        assert_eq!(game.carts[0].id, 71);
    });
}

pub(crate) fn bound_cart_case(
    kind: CartKind,
    action: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    super::super::native_entity_refs::tests::with_resident_game(|game| {
        game.server.tickets.0.update(None);
        game.carts
            .push(Minecart::new(71, kind, Vec3::new(0.5, 70.0, 0.5)));
        let holder = {
            let host: &dyn mod_api::ServerHost = game;
            host.entity(71).unwrap()
        };
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut values = Attachments::default();
        VALUE.set(&mut values, Holder::Entity(71), 10).unwrap();
        let original = game.carts.update(0, |cart| {
            cart.attachment_lifetime().bind(&mut values, &index, holder);
            cart.attachment_lifetime().registration().unwrap()
        });
        let key = ("test".into(), "cart_lifetime".into());
        let before = index.lock().unwrap().snapshot(&key);
        assert_eq!(before.len(), 1);
        action(game);
        let current = game
            .carts
            .update(0, |cart| cart.attachment_lifetime().registration().unwrap());
        assert!(original.same_registration(&current));
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        assert_eq!(VALUE.get(&mut values, Holder::Entity(71)), Ok(10));
        let taken = game.take_entities();
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        game.server.next_entity = game.server.next_entity.max(72);
        game.summon_with_nbt("minecraft:minecart", [8.5, 70.0, 0.5], "{}")
            .unwrap();
        let spawned = game.carts[0].id;
        assert_ne!(spawned, 71);
        let spawned_holder = game.entity(spawned).unwrap();
        let mut spawned_values = Attachments::default();
        VALUE
            .set(&mut spawned_values, Holder::Entity(spawned), 40)
            .unwrap();
        let spawned_registration = game.carts.update(0, |cart| {
            cart.attachment_lifetime()
                .bind(&mut spawned_values, &index, spawned_holder);
            cart.attachment_lifetime().registration().unwrap()
        });
        game.restore_entities(taken);
        assert_eq!(game.carts.last().unwrap().id, spawned);
        for (i, expected) in [(0, original), (game.carts.len() - 1, spawned_registration)] {
            let actual = game
                .carts
                .update(i, |cart| cart.attachment_lifetime().registration().unwrap());
            assert!(expected.same_registration(&actual));
        }
        VALUE.set(&mut values, Holder::Entity(71), 20).unwrap();
        let after = index.lock().unwrap().snapshot(&key);
        assert_eq!(after.len(), 2);
        assert_eq!(
            after
                .iter()
                .find(|row| row.holder == holder)
                .unwrap()
                .entry
                .read(&VALUE),
            Ok(&20)
        );
        assert_eq!(
            VALUE.get(&mut spawned_values, Holder::Entity(spawned)),
            Ok(40)
        );
        let live = game.entity(71).unwrap();
        assert!(game.entity_remove(live, mod_api::RemovalReason::Discarded));
        let clone = game.carts[0].clone();
        assert!(!game.store_cart_back(0, clone));
        game.native_removals_flush();
        assert!(!game.carts.iter().any(|cart| cart.id == 71));
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        VALUE.set(&mut values, Holder::Entity(71), 30).unwrap();
        let surviving = index.lock().unwrap().snapshot(&key);
        assert_eq!(surviving.len(), 1);
        assert_eq!(surviving[0].holder, spawned_holder);
        let i = game
            .carts
            .iter()
            .position(|cart| cart.id == spawned)
            .unwrap();
        drop(game.carts.remove(i));
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
        assert_eq!(
            after
                .iter()
                .find(|row| row.holder == holder)
                .unwrap()
                .entry
                .read(&VALUE),
            Ok(&20)
        );
        assert_eq!(
            after
                .iter()
                .find(|row| row.holder == spawned_holder)
                .unwrap()
                .entry
                .read(&VALUE),
            Ok(&40)
        );
    });
}

#[test]
#[ignore = "daily"]
fn destroyed_carts_retain_authoritative_state_and_retire_their_registration() {
    // Tick explosion, burning-arrow explosion, and ordinary vehicle damage.
    for mode in 0..3 {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "cancel_cart_explosion_dismount",
            Box::new(CancelCartDismount {
                calls: calls.clone(),
                index: index.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_resident_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.server.tickets.0.update(None);
                game.reset_entity_residency();
                let start = Vec3::new(0.5, 70.0, 0.5);
                let mut cart = Minecart::new(
                    71,
                    if mode == 2 {
                        CartKind::Rideable
                    } else {
                        CartKind::Tnt
                    },
                    start,
                );
                cart.velocity = Vec3::new(0.4, 0.0, 0.0);
                cart.no_gravity = true;
                cart.in_water = true;
                cart.tick_count = 9;
                cart.hurt_time = 3;
                cart.damage = 7.0;
                cart.fuse = 0;
                game.carts.push(cart);
                let other = Vec3::new(400.5, 70.0, 0.5);
                prepare_entity_ticking_neighborhood(game, ChunkPos::new(25, 0));
                game.carts
                    .push(Minecart::new(73, CartKind::Rideable, other));
                // Prevent the other cart's own tick from changing the hook's write.
                game.riding = Some(73);
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let mut rider = sunrise_sim::mobs::Mob::new(72, kind, start, 0.0);
                rider.extra.mount = Some(71);
                game.persist.off_level.insert(
                    72,
                    sunrise_gameplay_common::game::entity_memory::OffLevel::Mob(rider),
                );

                let holder = game.entity(71).unwrap();
                let mut values = Attachments::default();
                VALUE.set(&mut values, Holder::Entity(71), 10).unwrap();
                let registration = game.carts.update(0, |cart| {
                    cart.attachment_lifetime().bind(&mut values, &index, holder);
                    cart.attachment_lifetime().registration().unwrap()
                });
                let join = game.carts[0].join;
                let mut expected_random = game.carts[0].random.clone();
                if mode != 2 {
                    expected_random.next_double();
                }
                let next_random = expected_random.next_double();

                match mode {
                    0 => game.tick_carts(),
                    1 => game.arrow_hits_cart(0, 1.0, Vec3::ZERO, true, false, None, None),
                    _ => game.hurt_cart(
                        0,
                        5.0,
                        true,
                        false,
                        sunrise_gameplay::game::server_side::minecarts_server::CartHit::default(),
                    ),
                }
                assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
                assert!(game.entity(71).is_none());
                assert!(!game.carts.iter().any(|cart| cart.id == 71));
                assert_vehicle_residency_retired(game, 71);
                assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                assert_eq!(game.carts[0].velocity, Vec3::new(7.0, 8.0, 9.0));
                let Some(sunrise_gameplay_common::game::entity_memory::RemovedVehicle::Cart(cart)) =
                    game.persist.removed_vehicles.get(&71)
                else {
                    panic!("the canceled passenger must retain the removed cart");
                };
                assert_eq!(cart.join, join);
                assert!(
                    registration
                        .same_registration(&cart.attachment_lifetime.registration().unwrap())
                );
                assert!(
                    index
                        .lock()
                        .unwrap()
                        .snapshot(&("test".into(), "cart_lifetime".into()))
                        .is_empty()
                );
                assert_eq!(cart.random.clone().next_double(), next_random);
                if mode == 0 {
                    assert_eq!(cart.tick_count, 10);
                    assert_eq!(cart.hurt_time, 2);
                    assert_eq!(cart.damage, 6.0);
                    assert!(!cart.first_tick);
                    assert!(!cart.in_water);
                    assert_eq!(cart.previous, start);
                    assert_ne!(cart.position, start);
                    assert_ne!(cart.velocity, Vec3::new(0.4, 0.0, 0.0));
                } else {
                    assert_eq!(cart.tick_count, 9);
                    assert_eq!(cart.hurt_time, if mode == 2 { 10 } else { 3 });
                    assert_eq!(cart.damage, if mode == 2 { 57.0 } else { 7.0 });
                    assert_eq!(cart.position, start);
                }
                game.mob_object_mut(72).unwrap().extra.mount = None;
                game.release_removed_vehicle(71);
                assert!(!game.persist.removed_vehicles.contains_key(&71));
                assert_vehicle_residency_retired(game, 71);
            });
        });
    }
}

#[test]
#[ignore = "daily"]
fn disabled_tnt_rule_discards_primed_carts_but_keeps_unprimed_ticks() {
    for ridden in [false, true] {
        for primed in [false, true] {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.server.tickets.0.update(None);
                game.reset_entity_residency();
                assert!(game.ui.rules.set(
                    "tnt_explodes",
                    sunrise_game_data::gamerules::Value::Bool(false),
                ));
                let start = Vec3::new(0.5, 64.25, 0.5);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                game.world.set(0, 63, 0, stone).unwrap();
                let mut cart = Minecart::new(71, CartKind::Tnt, start);
                cart.no_gravity = true;
                cart.velocity = Vec3::new(0.4, -1.0, 0.0);
                cart.tick_count = 9;
                cart.fuse = if primed { 0 } else { -1 };
                // MinecartTNT.causeFallDamage attempts a blast on this landing.
                cart.fall_distance = 10.0;
                game.carts.push(cart);
                if ridden {
                    game.player =
                        Some(sunrise_sim::motion::PlayerMotion::new(game.motion, start).unwrap());
                    super::super::native_entity_refs::tests::bind_local_player(game, 70, 70);
                    game.riding = Some(71);
                    assert!(game.tick_cart_riding(Keys::default()));
                } else {
                    game.tick_carts();
                }
                if primed {
                    // Vanilla discards an already-primed cart even though no blast runs.
                    assert!(game.carts.is_empty());
                    assert_vehicle_residency_retired(game, 71);
                    if ridden {
                        assert_eq!(game.riding, None);
                    }
                } else {
                    assert_eq!(game.carts.len(), 1);
                    assert_eq!(game.carts[0].tick_count, 10);
                    assert_ne!(game.carts[0].position, start);
                    assert_eq!(game.carts[0].fall_distance.to_bits(), 0.0_f64.to_bits());
                    if ridden {
                        assert_eq!(game.riding, Some(71));
                        assert!(game.tick_cart_riding(Keys::default()));
                    } else {
                        game.tick_carts();
                    }
                    assert_eq!(game.carts[0].tick_count, 11);
                    // A burning arrow cannot explode this unprimed cart, but
                    // MinecartTNT.hurtServer still forwards its ordinary damage.
                    let before = game.carts[0].damage;
                    game.arrow_hits_cart(0, 1.0, Vec3::ZERO, true, false, None, None);
                    assert_eq!(game.carts[0].damage, before + 10.0);
                }
            });
        }
    }
}

#[test]
#[ignore = "daily"]
fn terminal_vehicle_damage_and_native_removal_retire_residency() {
    for cart in [false, true] {
        for mode in 0..3 {
            super::super::native_entity_refs::tests::with_resident_game(|game| {
                game.reset_entity_residency();
                game.server.tickets.0.update(None);
                game.summon_with_nbt(
                    if cart {
                        "minecraft:minecart"
                    } else {
                        "minecraft:oak_boat"
                    },
                    [0.5, 70.0, 0.5],
                    "{}",
                )
                .unwrap();
                let id = if cart {
                    game.carts[0].id
                } else {
                    game.boats[0].id
                };
                assert!(matches!(
                    game.with_resident_slot(
                        &mod_api::DimensionKey::OVERWORLD,
                        std::num::NonZeroU32::new(id).unwrap(),
                        |_| (),
                    ),
                    sunrise_gameplay::game::entity_residency::SlotAccess::Done(()),
                ));
                if mode == 0 {
                    if cart {
                        game.hurt_cart(
                            0,
                            5.0,
                            true,
                            false,
                            sunrise_gameplay::game::server_side::minecarts_server::CartHit::default(
                            ),
                        );
                    } else {
                        assert!(game.hurt_boat(0, 5.0, true));
                    }
                } else {
                    let entity = game.entity(id).unwrap();
                    assert!(game.entity_remove(
                        entity,
                        if mode == 1 {
                            mod_api::RemovalReason::Discarded
                        } else {
                            mod_api::RemovalReason::Killed
                        }
                    ));
                    game.native_removals_flush();
                }
                assert_vehicle_residency_retired(game, id);
            });
        }
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
