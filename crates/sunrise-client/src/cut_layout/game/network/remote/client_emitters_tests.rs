use sunrise_level::particle_options::ParticleType::{Crit, EnchantedHit};
use sunrise_protocol::entity_data::{Value, serializer};
use {
    crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::logged_in,
    crate::game::server_side::native_entity_refs::tests::with_client_game,
};
use {sunrise_gameplay_common::game::remote::C, sunrise_gameplay_common::game::remote::play};

pub(in crate::game) fn with_isolated_client(name: &str, body: impl FnOnce()) {
    let (_, module) = module_path!().split_once("::").unwrap();
    let test = format!("{module}::{name}");
    sunrise_gameplay_common::game::in_own_process(&test, body);
}

pub(in crate::game) fn registry_id(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    registry: &str,
    name: &str,
) -> i32 {
    game.client
        .remote
        .as_ref()
        .unwrap()
        .builtin
        .get(registry)
        .unwrap()
        .id(name)
        .unwrap()
}

pub(in crate::game) fn add_entity(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    uuid: u128,
    kind: &str,
    position: [f64; 3],
) -> C {
    C::AddEntity(play::AddEntity {
        id,
        uuid,
        kind: registry_id(game, "minecraft:entity_type", kind),
        position,
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    })
}

pub(in crate::game) fn deliver(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    packet: C,
) {
    let bytes = packet.encode();
    let mut reader = sunrise_protocol::buf::Reader::new(&bytes);
    game.handle_packet(C::decode(reader.varint().unwrap(), reader.rest()).unwrap())
        .unwrap();
}

pub(in crate::game) fn animate(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    action: u8,
) {
    deliver(game, C::Animate(play::Animate { id, action }));
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::client_emitters_tests::capture_requests;

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::client_emitters_tests::assert_bursts;

#[test]
#[ignore = "daily"]
fn hit_emitters_follow_vehicles_and_finish_at_the_removed_body() {
    with_isolated_client(
        "hit_emitters_follow_vehicles_and_finish_at_the_removed_body",
        || {
            with_client_game(|game| {
                let _reader = logged_in(game);
                let mut random = capture_requests(game);
                let (boat, cart) = (51, 52);
                animate(game, boat, 1);
                assert_bursts(game, &mut random, &[]);

                let boat_at = [4.0, 65.0, 0.0];
                let cart_at = [24.0, 65.0, 0.0];
                deliver(
                    game,
                    add_entity(game, boat, 151, "minecraft:oak_boat", boat_at),
                );
                deliver(
                    game,
                    add_entity(game, cart, 152, "minecraft:minecart", cart_at),
                );
                let boat_body = |x| (Crit, [x, 65.0, 0.0], 1.375, 0.5625);
                let cart_body = |x| (EnchantedHit, [x, 65.0, 0.0], 0.98, 0.7);
                animate(game, boat, 1);
                animate(game, cart, 2);
                assert_bursts(game, &mut random, &[boat_body(4.0), cart_body(24.0)]);

                game.boats.update(0, |body| body.position.x = 8.0);
                game.carts.update(0, |body| body.position.x = 28.0);
                game.tick_particle_engine();
                assert_bursts(game, &mut random, &[boat_body(8.0), cart_body(28.0)]);

                // Move the boat after the last emitter tick. Keep that final body,
                // not its previous sample and not the replacement with its id.
                game.boats.update(0, |body| body.position.x = 12.0);
                deliver(
                    game,
                    C::RemoveEntities(play::RemoveEntities { ids: vec![boat] }),
                );
                deliver(
                    game,
                    add_entity(game, boat, 153, "minecraft:oak_boat", [16.0, 65.0, 0.0]),
                );
                game.tick_particle_engine();
                assert_bursts(game, &mut random, &[boat_body(12.0), cart_body(28.0)]);
                game.tick_particle_engine();
                assert_bursts(game, &mut random, &[]);

                let stand_at = [5.0, 65.0, 0.0];
                let crystal_at = [21.0, 65.0, 0.0];
                deliver(
                    game,
                    add_entity(game, 53, 53, "minecraft:armor_stand", stand_at),
                );
                deliver(
                    game,
                    add_entity(game, 54, 54, "minecraft:end_crystal", crystal_at),
                );
                animate(game, 53, 1);
                animate(game, 54, 2);
                assert_bursts(
                    game,
                    &mut random,
                    &[
                        (Crit, stand_at, 0.5, 1.975),
                        (EnchantedHit, crystal_at, 2.0, 2.0),
                    ],
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn hit_emitters_use_cloud_radius_and_the_live_thrown_potion() {
    with_isolated_client(
        "hit_emitters_use_cloud_radius_and_the_live_thrown_potion",
        || {
            with_client_game(|game| {
                let _reader = logged_in(game);
                let mut random = capture_requests(game);
                let (cloud, potion) = (61, 62);
                let cloud_at = [4.0, 65.0, 0.0];
                let potion_at = [8.0, 65.0, 4.0];
                deliver(
                    game,
                    add_entity(game, cloud, 161, "minecraft:area_effect_cloud", cloud_at),
                );
                deliver(
                    game,
                    add_entity(game, potion, 162, "minecraft:splash_potion", potion_at),
                );
                let radius = |r| {
                    C::SetEntityData(play::SetEntityData {
                        id: cloud,
                        values: vec![(8, serializer::FLOAT, Value::Float(r))],
                    })
                };
                deliver(game, radius(1.25));
                game.step_remote_thrown();
                // Read the independently ticked projectile, not its packet position
                // or the resolver under test. Gravity moves it even at zero motion.
                let position = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                    let at = game.potions.thrown_entities()[0].1;
                    [at.x, at.y, at.z]
                };
                let first = position(game);
                assert_ne!(first, potion_at);
                animate(game, cloud, 1);
                animate(game, potion, 2);
                assert_bursts(
                    game,
                    &mut random,
                    &[
                        (Crit, cloud_at, 2.5, 0.5),
                        (EnchantedHit, first, 0.25, 0.25),
                    ],
                );

                deliver(game, radius(2.0));
                game.step_remote_thrown();
                let second = position(game);
                assert_ne!(first, second);
                game.tick_particle_engine();
                assert_bursts(
                    game,
                    &mut random,
                    &[
                        (Crit, cloud_at, 4.0, 0.5),
                        (EnchantedHit, second, 0.25, 0.25),
                    ],
                );

                deliver(game, radius(2.75));
                game.step_remote_thrown();
                let last = position(game);
                assert_ne!(second, last);
                deliver(
                    game,
                    C::RemoveEntities(play::RemoveEntities {
                        ids: vec![cloud, potion],
                    }),
                );
                game.tick_particle_engine();
                assert_bursts(
                    game,
                    &mut random,
                    &[(Crit, cloud_at, 5.5, 0.5), (EnchantedHit, last, 0.25, 0.25)],
                );
                game.tick_particle_engine();
                assert_bursts(game, &mut random, &[]);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn replacing_a_general_entity_keeps_its_emitters_separate() {
    with_isolated_client(
        "replacing_a_general_entity_keeps_its_emitters_separate",
        || {
            use sunrise_protocol::{general_entity, packets::configuration::CustomPayload};
            struct Declare;
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for Declare {
                fn register(
                    &self,
                    r: &mut dyn mod_api::Registrar,
                    _: &mod_api::RegistrationContext,
                ) {
                    r.declare_general_entity(
                        "target",
                        mod_api::GeneralEntityCategory::Misc,
                        0.5,
                        1.25,
                        None,
                        mod_api::GeneralSavePolicy::Save,
                        true,
                        true,
                        false,
                        None,
                        8,
                        1,
                        1,
                        &[],
                    )
                    .unwrap();
                }
            }
            mod_platform::__private::initialize().unwrap();
            let _phase = mod_platform::__private::RegistrationGuard::open();
            let declare = || vec![mod_api::ModEntry::new("emitter_test", Box::new(Declare))];
            mod_platform::testing::with_mods(declare().into(), || {
                mod_platform::__private::part_tests::install_parts_fixture(&declare()).unwrap();
                with_client_game(|game| {
                    let _reader = logged_in(game);
                    let mut random = capture_requests(game);
                    let id = 71;
                    // At this x, rounding collapses `bounds.max.x - bounds.min.x`
                    // to zero. The z offsets still have the declared width.
                    let old = [4_503_599_627_370_497.0, 65.0, 4.0];
                    deliver(
                        game,
                        C::CustomPayload(CustomPayload {
                            channel: general_entity::CHANNEL.into(),
                            data: general_entity::encode(&general_entity::Message::Spawn {
                                id,
                                uuid: 171,
                                type_key: "emitter_test:target".into(),
                                position: old,
                                rotation: [0.0; 2],
                                velocity: [0.0; 3],
                            })
                            .unwrap(),
                        }),
                    );
                    animate(game, id, 1);
                    let old_body = (Crit, old, 0.5, 1.25);
                    assert_bursts(game, &mut random, &[old_body]);

                    let new = [12.0, 65.0, 0.0];
                    deliver(game, add_entity(game, id, 172, "minecraft:minecart", new));
                    assert!(
                        !game
                            .client
                            .remote
                            .as_ref()
                            .unwrap()
                            .general
                            .rows
                            .contains_key(&id)
                    );
                    animate(game, id, 2);
                    let new_body = (EnchantedHit, new, 0.98, 0.7);
                    assert_bursts(game, &mut random, &[new_body]);
                    game.tick_particle_engine();
                    assert_bursts(game, &mut random, &[old_body, new_body]);
                    game.tick_particle_engine();
                    assert_bursts(game, &mut random, &[old_body, new_body]);
                    game.tick_particle_engine();
                    assert_bursts(game, &mut random, &[]);
                });
            });
        },
    );
}
