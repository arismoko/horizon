use std::sync::Mutex;
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::minecarts::{
    Arc, CartKind, InteractTarget, Keys, Minecart, SEAT, Vec3,
};
use sunrise_items::items::ItemStack;
use sunrise_mod_runtime::attachments::{
    AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
};

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn lifecycle_carts_keep_registrations_through_commands_unload_park_and_remote() {
    use crate::game::server_side::entity_server::taken_pass_tests::{
        LifecycleFixture, RemoteLifecycle, SlotIdentities, run_remote_lifecycle,
    };
    use sunrise_protocol::{
        connection::Pipe,
        entity_data::{Value, serializer},
    };
    let mut identities = SlotIdentities::default();
    LifecycleFixture {
        seed: |game, entries| {
            game.carts.clear();
            for &(id, at) in entries {
                game.carts.push(Minecart::new(
                    id,
                    CartKind::Rideable,
                    Vec3::new(at[0], at[1], at[2]),
                ));
            }
        },
        ids: |game| game.carts.iter().map(|cart| cart.id).collect(),
        target: sunrise_gameplay_common::game::commands_common::Target::Cart,
        unloads_with_chunk: true,
        remote: None,
    }
    .run_with(|game| {
        let epoch = game.native_lifecycle.epoch().unwrap();
        identities.observe(&mut game.carts, epoch, |cart| cart.id)
    });
    run_remote_lifecycle(
        |game, entries| {
            game.carts.clear();
            for &(id, at) in entries {
                game.carts.push(Minecart::new(
                    id,
                    CartKind::Rideable,
                    Vec3::new(at[0], at[1], at[2]),
                ));
            }
        },
        |game| game.carts.iter().map(|cart| cart.id).collect(),
        RemoteLifecycle {
            prepare: |game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "Lifecycle").unwrap();
            },
            apply: |game, remove, update| {
                game.remote_object_data(update as i32, &[(12, serializer::INT, Value::Int(19))]);
                assert_eq!(
                    game.carts
                        .iter()
                        .find(|cart| cart.id == update)
                        .unwrap()
                        .custom_offset,
                    Some(19)
                );
                game.remove_remote_object(remove);
            },
        },
    );
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn connecting_to_a_remote_level_clears_the_existing_cart_list() {
    use sunrise_protocol::connection::Pipe;
    sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
        game.carts.push(Minecart::new(
            71,
            CartKind::Rideable,
            Vec3::new(0.5, 70.0, 0.5),
        ));
        assert_eq!(game.carts.len(), 1);
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "Cart reset").unwrap();
        // Observe the production reset before any fixture reseeding or cleanup.
        assert!(game.carts.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn exploding_a_cart_removes_only_its_owner_and_keeps_a_bound_survivor() {
    bound_cart_case(CartKind::Rideable, |game| {
        game.carts
            .push(Minecart::new(72, CartKind::Tnt, Vec3::new(40.5, 70.0, 0.5)));
        let mut doomed = game.carts[1].clone();
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut values = Attachments::default();
        VALUE.set(&mut values, Holder::Entity(72), 99).unwrap();
        game.carts.update(1, |cart| {
            cart.attachment_lifetime().bind(&mut values, &index, 72u32);
        });
        let key = ("test".into(), "cart_lifetime".into());
        let retained = index.lock().unwrap().snapshot(&key);
        assert_eq!(retained.len(), 1);
        game.explode_cart(&mut doomed, 0.0);
        assert_eq!(
            game.carts.iter().map(|cart| cart.id).collect::<Vec<_>>(),
            [71]
        );
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        VALUE.set(&mut values, Holder::Entity(72), 100).unwrap();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(retained[0].entry.read(&VALUE), Ok(&99));
        // bound_cart_case also checks the survivor's registration and value.
    });
}

pub(crate) static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "cart_lifetime",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync_codec: None,
};

pub(crate) struct CancelCartDismount {
    pub(crate) calls: Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) index: Arc<Mutex<AttachmentIndex<mod_api::EntityRef>>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for CancelCartDismount {
    fn entity_mount(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::EntityMountEvent<'_>,
    ) -> mod_api::Flow {
        if !event.mounting && event.entity == 72 {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // Destruction is visible before the callback, but the hook can
            // still mutate other resident entities and thus the cart list.
            assert!(host.entity(event.vehicle).is_none());
            assert!(
                self.index
                    .lock()
                    .unwrap()
                    .snapshot(&("test".into(), "cart_lifetime".into()))
                    .is_empty()
            );
            assert!(host.entity(73).unwrap().set_velocity(host, [7.0, 8.0, 9.0]));
            return mod_api::Flow::Cancel;
        }
        mod_api::Flow::Continue
    }
}

pub(crate) struct TeleportPlayerOnCartDismount(pub(crate) Arc<std::sync::atomic::AtomicUsize>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for TeleportPlayerOnCartDismount {
    fn entity_mount(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::EntityMountEvent<'_>,
    ) -> mod_api::Flow {
        if !event.mounting && event.vehicle == 71 {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let player = host.entity(70).unwrap();
            assert!(player.teleport_to(host, [30.5, 210.0, 40.5], None));
            return mod_api::Flow::Cancel;
        }
        mod_api::Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn exploding_driven_cart_does_not_reseat_a_player_moved_by_its_dismount_hook() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "teleport_cart_rider",
        Box::new(TeleportPlayerOnCartDismount(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(0.5, 200.0, 0.5))
                    .unwrap(),
            );
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 70, 70);
            let mut cart = Minecart::new(71, CartKind::Tnt, Vec3::new(0.5, 200.0, 0.5));
            cart.no_gravity = true;
            cart.fuse = 0;
            game.carts.push(cart);
            game.riding = Some(71);
            assert!(game.tick_cart_riding(Keys::default()));
            assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
            assert_eq!(game.riding, Some(71));
            assert_eq!(
                game.player.as_ref().unwrap().position,
                Vec3::new(30.5, 210.0, 40.5)
            );
            assert!(game.persist.removed_vehicles.contains_key(&71));
        });
    });
}

pub(crate) struct ToggleActivatorDismount {
    pub(crate) cancel: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) calls: Arc<std::sync::atomic::AtomicUsize>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for ToggleActivatorDismount {
    fn entity_mount(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::EntityMountEvent<'_>,
    ) -> mod_api::Flow {
        if !event.mounting && event.vehicle == 71 {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return mod_api::Flow::Cancel;
            }
        }
        mod_api::Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn activator_respects_canceled_player_dismount() {
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "cancel_activator_dismount",
        Box::new(ToggleActivatorDismount {
            cancel: cancel.clone(),
            calls: calls.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.player =
                Some(sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 70, 70);
            let mut cart = Minecart::new(71, CartKind::Rideable, Vec3::ZERO);
            game.carts.push(cart.clone());
            game.riding = Some(71);
            game.activate_cart(&mut cart, true);
            assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
            assert_eq!(game.riding, Some(71));
            cancel.store(false, std::sync::atomic::Ordering::Relaxed);
            game.activate_cart(&mut cart, true);
            assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
            assert_eq!(game.riding, None);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_bound_cart_commits_the_normal_server_tick() {
    bound_cart_case(CartKind::Furnace, |game| {
        game.carts.update(0, |cart| cart.fuel = 6);
        game.tick_carts();
        assert_eq!(game.carts[0].fuel, 5);
        assert_eq!(game.carts[0].tick_count, 1);
    });
}

#[test]
#[ignore = "daily"]
fn a_bound_cart_commits_the_ridden_tick() {
    bound_cart_case(CartKind::Rideable, |game| {
        game.carts.update(0, |cart| cart.in_water = true);
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(0.5, 70.0 + SEAT, 0.5))
                .unwrap(),
        );
        game.riding = Some(71);
        assert!(game.tick_cart_riding(Keys::default()));
        assert!(!game.carts[0].in_water);
        assert_eq!(game.carts[0].tick_count, 1);
    });
}

#[test]
#[ignore = "daily"]
fn a_bound_ticked_cart_publishes_its_moved_position_before_pushing() {
    bound_cart_case(CartKind::Rideable, |game| {
        game.carts.update(0, |cart| cart.velocity.x = 0.4);
        game.carts.push(Minecart::new(
            72,
            CartKind::Rideable,
            Vec3::new(1.75, 70.0, 0.5),
        ));
        // Keep the other cart's own tick from masking the first cart's push.
        game.riding = Some(72);
        game.tick_carts();
        assert!(game.carts[0].position.x > 0.5);
        assert!(game.carts[1].velocity.x > 0.0);
    });
}

#[test]
#[ignore = "daily"]
fn a_bound_tnt_cart_passes_interaction_without_priming() {
    bound_cart_case(CartKind::Tnt, |game| {
        game.common.inventory[game.common.selected] =
            Some(ItemStack::new("minecraft:fire_charge", 1));
        // MinecartTNT inherits Entity.interact (PASS), even with a fire charge.
        assert!(!game.interact_cart(Some(InteractTarget {
            id: 71,
            location: [0.5, 70.0, 0.5],
        })));
        assert_eq!(game.carts[0].fuse, -1);
        assert_eq!(
            game.inventory[game.selected],
            Some(ItemStack::new("minecraft:fire_charge", 1))
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_bound_cart_commits_valid_nbt_and_refuses_an_invalid_load() {
    bound_cart_case(CartKind::Furnace, |game| {
        let target = sunrise_gameplay_common::game::commands_common::Target::Cart(71);
        game.carts.update(0, |cart| cart.fuel = 6);
        let uuid = game.uuid_for(71);
        assert!(
            game.apply_entity_nbt(target, Tag::Compound(Default::default()))
                .is_err()
        );
        assert_eq!(game.carts[0].fuel, 6);
        let (tag, _) = sunrise_core::nbt::parse_snbt(
            "{Pos:[0.5d,70.0d,0.5d],Fuel:13s,UUID:[I;9,8,7,6],DisplayState:{Name:\"test:missing\"}}",
        ).unwrap();
        game.apply_entity_nbt(target, tag).unwrap();
        assert_eq!(game.carts[0].fuel, 13);
        assert_eq!(game.uuid_for(71), uuid);
        assert!(game.carts[0].custom_display.is_none());
    });
}

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::minecarts_server::lifetime_tests::bound_cart_case;

#[test]
#[ignore = "daily"]
fn a_bound_tnt_cart_keeps_its_join_when_damage_primes_it() {
    bound_cart_case(CartKind::Tnt, |game| {
        game.carts.push(Minecart::new(
            72,
            CartKind::Rideable,
            Vec3::new(4.5, 70.0, 0.5),
        ));
        let expected: Vec<_> = game.carts.iter().map(|c| (c.id, c.join)).collect();
        let join = game.carts[0].join;
        game.hurt_cart(
            0,
            5.0,
            false,
            true,
            sunrise_gameplay::game::server_side::minecarts_server::CartHit::default(),
        );
        assert_eq!(
            game.carts
                .iter()
                .map(|c| (c.id, c.join))
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(game.carts[0].join, join);
        assert_eq!(game.carts.locate(join), Some(0));
        assert!((0..=38).contains(&game.carts[0].fuse));
        assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
    });
}

// A public entity lookup also hides a temporarily extracted row. Check the
// residency journal itself so a terminal path cannot leave a permanent Taken
// record that poisons later level queries (#2855).
pub(crate) fn assert_vehicle_residency_retired(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    id: u32,
) {
    use sunrise_gameplay::game::entity_residency::SlotAccess;
    assert_eq!(
        game.with_resident_slot(
            &mod_api::DimensionKey::OVERWORLD,
            std::num::NonZeroU32::new(id).unwrap(),
            |_| panic!("removed vehicle still occupies a resident slot"),
        ),
        SlotAccess::<()>::Missing,
    );
    assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
}
