use std::sync::{Arc, atomic::AtomicI32, mpsc};
use sunrise_core::geometry::Vec3;
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::lan_actions::guest_body::initialize;
use sunrise_level::assets::Assets;
use sunrise_protocol::packets::configuration::ClientInformation;
use sunrise_sim::lan::{Joined, PlayerSender, guest::SavedBody};
use sunrise_sim::{
    lan::{guest::Guest, hub::CommittedMove},
    motion::{MotionResources, PlayerMotion},
};

#[path = "tests/fall_reset.rs"]
pub(crate) mod fall_reset;
#[path = "tests/travel.rs"]
pub(crate) mod travel;

pub(crate) fn guest(entity_id: i32) -> Guest {
    let (sender, _) = mpsc::channel();
    Guest::new(
        Joined {
            conn: entity_id as u32,
            uuid: entity_id as u128,
            name: "GuestBodyTest".into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(sender, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        entity_id,
        [0.5, 64.0, 0.5],
        2,
    )
}

#[test]
#[ignore = "daily"]
fn co_located_guests_own_distinct_streams_and_restore_the_saved_body() {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let assets = Assets::load(&root).unwrap();
    let resources = MotionResources::load(&root, &assets).unwrap();
    let mut owner = PlayerMotion::new(&resources, Vec3::new(0.5, 64.0, 0.5)).unwrap();
    let mut first = guest(2);
    let mut second = guest(3);
    let saved = SavedBody {
        fire_ticks: 43,
        air: 173,
        fall_distance: 4.5,
        velocity: [0.25, -0.5, 0.75],
        food_exhaustion: 1.25,
        food_tick_timer: 17,
        ticks_frozen: 23,
        invulnerable: true,
        invulnerable_time: 7,
    };
    first.loaded_body = Some(saved);
    initialize(&resources, &mut first);
    initialize(&resources, &mut second);
    assert!(first.loaded_body.is_none());
    assert_eq!(SavedBody::of(first.motion.as_ref().unwrap()), saved);
    let mut first_random = first.motion.as_ref().unwrap().random.clone();
    let mut second_random = second.motion.as_ref().unwrap().random.clone();
    let first_trace: Vec<_> = (0..4).map(|_| first_random.next_int()).collect();
    let second_trace: Vec<_> = (0..4).map(|_| second_random.next_int()).collect();
    let owner_trace: Vec<_> = (0..4).map(|_| owner.random.next_int()).collect();
    assert_ne!(first_trace, second_trace);
    assert_ne!(first_trace, owner_trace);
    assert_ne!(second_trace, owner_trace);
    // Advancing one guest does not borrow or advance the other body.
    first.motion.as_mut().unwrap().random.next_int();
    let unchanged: Vec<_> = (0..4)
        .map(|_| second.motion.as_mut().unwrap().random.next_int())
        .collect();
    assert_eq!(unchanged, second_trace);
}

#[test]
#[ignore = "daily"]
fn accepted_hit_moves_through_teleport_restore_but_not_respawn() {
    use sunrise_items::items::ItemStack;
    use sunrise_sim::health::{AcceptedHit, Attack, HurtSource};
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let assets = Assets::load(&root).unwrap();
    let resources = MotionResources::load(&root, &assets).unwrap();
    let source = HurtSource {
        damage_type: "minecraft:arrow",
        causing: 41,
        direct: 73,
        position: Some([-17.25, 64.5, 32.75]),
        direct_type: None,
    };
    let mut weapon = ItemStack::new("minecraft:bow", 1);
    weapon.damage = 19;
    weapon.enchant("minecraft:power", 4);
    let attack = Attack {
        attacker: 41,
        projectile: Some(73),
        projectile_weapon: Some(weapon.clone()),
        ..Default::default()
    };
    let hit = Arc::new(AcceptedHit::capture(source, &attack, None));
    for restore_all in [true, false] {
        let mut player = guest(2);
        assert!(initialize(&resources, &mut player));
        player.motion.as_mut().unwrap().health.accepted_hit = Some(hit.clone());
        let previous = player.new_body(restore_all).unwrap();
        if restore_all {
            assert!(
                previous.health.accepted_hit.is_none(),
                "the record moved, not copied"
            );
            assert!(Arc::ptr_eq(player.carried_hit.as_ref().unwrap(), &hit));
            assert!(
                player.new_body(true).is_none(),
                "a second teleport keeps the pending owner"
            );
        } else {
            assert!(
                player.carried_hit.is_none(),
                "a new living identity inherits no accepted blow"
            );
        }
        assert!(initialize(&resources, &mut player));
        assert!(
            player.carried_hit.is_none(),
            "initialization consumes the pending owner once"
        );
        let restored = player.motion.as_ref().unwrap().health.accepted_hit.as_ref();
        if restore_all {
            let restored = restored.unwrap();
            assert!(Arc::ptr_eq(restored, &hit));
            assert_eq!(restored.source(), source);
            assert_eq!(restored.weapon, Some(weapon.clone()));
            player.new_body(true);
            assert!(player.carried_hit.is_some());
            player.new_body(false);
            assert!(
                player.carried_hit.is_none(),
                "respawn clears even an unmaterialized teleport body"
            );
        } else {
            assert!(restored.is_none());
        }
    }
}

#[test]
#[ignore = "daily"]
fn entity_tick_count_moves_through_teleport_bodies_but_respawn_starts_at_zero() {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let assets = Assets::load(&root).unwrap();
    let resources = MotionResources::load(&root, &assets).unwrap();

    let mut guest = guest(2);
    assert!(initialize(&resources, &mut guest));
    guest.motion.as_mut().unwrap().health.tick_count = 123;
    guest.new_body(true);
    assert_eq!(guest.carried_tick_count, Some(123));
    assert!(initialize(&resources, &mut guest));
    assert_eq!(guest.motion.as_ref().unwrap().health.tick_count, 123);

    guest.new_body(true);
    assert_eq!(guest.carried_tick_count, Some(123));
    assert!(guest.motion.is_none());
    guest.new_body(false);
    assert_eq!(guest.carried_tick_count, None);
    assert!(initialize(&resources, &mut guest));
    assert_eq!(guest.motion.as_ref().unwrap().health.tick_count, 0);
}

#[test]
#[ignore = "daily"]
fn guest_equipment_changes_powder_snow_support_on_the_same_move() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::lan_actions::guest_body::tests::guest_equipment_changes_powder_snow_support_on_the_same_move",
        || {
            use sunrise_core::position::BlockPos;
            use sunrise_gameplay::game::native_entity_refs::tests::with_game;
            use sunrise_items::items::ItemStack;
            use sunrise_level::chunk::Chunk;
            use {
                sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
                sunrise_gameplay_common::game::server_side::native_item_places::tests::guest as online_guest,
            };

            with_game(|game| {
                open_to_lan(game);
                {
                    let (common, server) =
                        sunrise_gameplay_common::game::SideGame::common_and_state_mut(&mut *game);
                    server
                        .streaming
                        .insert_remote(&mut common.world, Chunk::empty(0, 0));
                }
                let snow = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:powder_snow")
                    .unwrap();
                game.world.set(1, 63, 1, snow).unwrap();
                let mut guest = online_guest(1);
                guest.loaded = true;
                guest.position = [1.5, 64.0, 1.5];
                guest.motion = Some(Box::new(
                    PlayerMotion::new(game.motion, Vec3::new(1.5, 64.0, 1.5)).unwrap(),
                ));
                game.server.lan.as_mut().unwrap().hub.guests.push(guest);
                for boots in [true, false] {
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(1, |guest| {
                            guest.armor[3] =
                                boots.then(|| ItemStack::new("minecraft:leather_boots", 1));
                            guest.on_ground = true;
                        })
                        .unwrap();
                    game.guest_accepted_move(
                        1,
                        CommittedMove {
                            horizontal_collision: boots,
                            from: [1.5, 64.25, 1.5],
                            jump_yaw: None,
                            movement: sunrise_sim::lan::guest::GuestMove {
                                position: [1.5, 64.0, 1.5],
                                delta: [0.0, -0.25, 0.0],
                                on_ground: true,
                                sneaking: false,
                            },
                        },
                    );
                    let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
                    let ground = &guest.motion.as_ref().unwrap().ground;
                    assert_eq!(ground.horizontal_collision, boots);
                    assert_eq!(
                        ground.supporting_block,
                        boots.then_some(BlockPos::new(1, 63, 1)),
                        "boots worn: {boots}"
                    );
                }
            });
        },
    );
}

/// `living_breathe` for a guest's body, asked with the owning host between the air decision and
/// its application: the listener reads the helmet, edits the chestplate through a transaction,
/// triggers a criterion and answers; the game applies it to the air it decided on, or refuses
/// when the listener changed that air and rolls the transactional edit back.
pub(crate) struct Breather {
    pub(crate) meddle: bool,
    pub(crate) helmet: Arc<std::sync::Mutex<Option<String>>>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Breather {
    fn living_breathe(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LivingBreatheEvent,
    ) {
        use mod_api::HostTransaction;
        struct Granted;
        impl mod_api::CriterionTest for Granted {
            fn test(
                &mut self,
                _: &dyn mod_api::CriterionContext,
            ) -> Result<bool, mod_api::CriterionTestError> {
                Ok(true)
            }
        }
        let player = host.players().acting_player().unwrap();
        assert_eq!(event.entity.entity(), player.entity());
        let head = mod_api::ItemPlace::PlayerSlot {
            player: 123,
            slot: 103,
        };
        *self.helmet.lock().unwrap() = head
            .with_stack(host, |stack| {
                mod_api::__private::item_key_id(stack.item()).map(str::to_owned)
            })
            .flatten();
        let chest = mod_api::ItemPlace::PlayerSlot {
            player: 123,
            slot: 102,
        };
        host.transaction(|host, tx| chest.edit(host, tx, |stack| stack.set_damage(5)))
            .unwrap();
        assert_eq!(
            player.trigger_criterion(
                host,
                &mod_api::Identifier::parse("breathing:tick").unwrap(),
                &mut Granted,
            ),
            Ok(())
        );
        if self.meddle {
            assert!(event.entity.entity().set_air_supply(host, 200));
        }
        event.can_breathe = false;
        event.consume = 3;
    }
}

/// One tick of a guest in a diving helmet and a damageable chestplate: the helmet the listener
/// read, then the air and the chestplate's damage the tick left.
pub(crate) fn breathing_guest_tick(meddle: bool) -> (Option<String>, i32, i32) {
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;
    use {
        sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
        sunrise_gameplay_common::game::server_side::native_item_places::tests::guest as online_guest,
    };

    let helmet = Arc::new(std::sync::Mutex::new(None));
    let mut left = (0, 0);
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "breathing",
            Box::new(Breather {
                meddle,
                helmet: helmet.clone(),
            }),
        )]
        .into(),
        || {
            with_game(|game| {
                open_to_lan(game);
                {
                    let (common, server) =
                        sunrise_gameplay_common::game::SideGame::common_and_state_mut(&mut *game);
                    server
                        .streaming
                        .insert_remote(&mut common.world, Chunk::empty(0, 0));
                }
                let mut guest = online_guest(1);
                guest.loaded = true;
                guest.position = [1.5, 64.0, 1.5];
                guest.motion = Some(Box::new(
                    PlayerMotion::new(game.motion, Vec3::new(1.5, 64.0, 1.5)).unwrap(),
                ));
                guest.armor[0] = Some(ItemStack::new("minecraft:turtle_helmet", 1));
                guest.armor[1] = Some(ItemStack::new("minecraft:diamond_chestplate", 1));
                game.server.lan.as_mut().unwrap().hub.guests.push(guest);
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(1, |guest| {
                        guest.on_ground = true;
                        guest.moves_from_ground = false;
                        guest.moves.push(sunrise_sim::lan::guest::GuestMove {
                            position: guest.position,
                            delta: [0.0, -0.25, 0.0],
                            on_ground: true,
                            sneaking: false,
                        });
                    })
                    .unwrap();
                game.guest_server_tick(1, true);
                let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
                left = (
                    guest.motion.as_ref().unwrap().air,
                    guest.armor[1].as_ref().unwrap().damage,
                );
            });
        },
    );
    let helmet = helmet.lock().unwrap().clone();
    (helmet, left.0, i32::try_from(left.1).unwrap())
}

pub(crate) fn assert_isolated_breathing_tick(name: &str, meddle: bool, air: i32, damage: i32) {
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        &format!("cut_layout::game::lan_actions::guest_body::tests::{name}"),
        "SUNRISE_GUEST_BREATHING_CHILD",
        &["guest"],
    ) {
        return;
    }
    // The executable installs the breathing listener bridge process-wide.
    crate::entry::install_client_hooks();
    assert_eq!(
        breathing_guest_tick(meddle),
        (Some("minecraft:turtle_helmet".to_owned()), air, damage)
    );
}

#[test]
#[ignore = "daily"]
fn a_guest_breathes_with_what_its_listener_answers_and_wrote() {
    // Vanilla keeps the full 300 air on land; the listener took 3 and damaged the chestplate.
    assert_isolated_breathing_tick(
        "a_guest_breathes_with_what_its_listener_answers_and_wrote",
        false,
        297,
        5,
    );
}

#[test]
#[ignore = "daily"]
fn a_guest_whose_air_the_listener_changed_keeps_it_and_loses_the_transactional_writes() {
    // The decision read 300; the listener left 200, so the answer is refused rather than
    // recomputed, and the chestplate's edit is undone.
    assert_isolated_breathing_tick(
        "a_guest_whose_air_the_listener_changed_keeps_it_and_loses_the_transactional_writes",
        true,
        200,
        0,
    );
}
