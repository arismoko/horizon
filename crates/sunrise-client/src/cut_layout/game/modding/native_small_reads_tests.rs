//! A client's `LevelRef::brightness`, `EntityRef::set_sprinting` and the server's
//! `tick_rate_is_frozen`.

use super::native_entity_refs::tests::with_game;
use mod_api::LightLayer;
use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
use sunrise_gameplay_common::game::native_entity_state_common::NativeEntityStateCommonGameExt1;

/// A client reads the light of the level it shows, only while logged in.
#[test]
#[ignore = "daily"]
fn a_client_reads_the_light_of_its_level() {
    use std::sync::Arc;
    use sunrise_level::chunk::{Chunk, Light};
    use sunrise_protocol::connection::Pipe;
    super::native_entity_refs::tests::with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "BrightnessTest").unwrap();
        game.client.remote.as_mut().unwrap().logged_in = true;
        let mut chunk = Chunk::empty(0, 0);
        Arc::make_mut(&mut chunk.sections[0]).light = Light::Uniform(0xC5);
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        let level = game.level("minecraft:overworld").unwrap();
        let lit = [3, -60, 5];
        assert_eq!(
            game.level_brightness(level, LightLayer::Block, lit),
            Some(5)
        );
        assert_eq!(game.level_brightness(level, LightLayer::Sky, lit), Some(12));
        assert_eq!(
            game.level_brightness(level, LightLayer::Sky, [16, -60, 0]),
            None
        );
        assert_eq!(level.can_see_sky(game, lit), Some(false));
        game.client.remote.as_mut().unwrap().logged_in = false;
        assert_eq!(game.level_brightness(level, LightLayer::Block, lit), None);
    });
}

/// A client's restricted level and handle read its local support and light without loading chunks.
#[test]
#[ignore = "daily"]
fn restricted_level_support_and_sky_reads() {
    use super::native_entity_refs::tests::with_client_game;
    use mod_api::{BlockFace, LevelHost, SupportType};
    use std::sync::Arc;
    use sunrise_level::chunk::{Chunk, Light};
    use sunrise_protocol::connection::Pipe;
    mod_platform::testing::with_mods(Default::default(), || {
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "SupportReadTest").unwrap();
            game.client.remote.as_mut().unwrap().logged_in = true;
            let pos = [3, -60, 5];
            let unloaded = [32, -60, 5];
            // Beyond the top light padding of the Overworld, not a block position.
            let above = [3, game.world.shape.max_y() + 16, 5];
            let mut chunk = Chunk::empty(0, 0);
            Arc::make_mut(&mut chunk.sections[0]).light = Light::Uniform(0xF0);
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(pos[0], pos[1], pos[2], stone).unwrap();
            let level = game.native_client_level().unwrap();
            assert_eq!(
                level.block_face_sturdy(game, pos, BlockFace::Up, SupportType::Full),
                Some(true)
            );
            assert_eq!(level.can_see_sky(game, pos), Some(true));
            assert_eq!(level.can_see_sky(game, above), Some(true));
            assert_eq!(level.can_see_sky(game, unloaded), Some(true));
            game.with_level(|_, level, _| {
                for support in [SupportType::Full, SupportType::Center, SupportType::Rigid] {
                    assert_eq!(
                        level.block_face_sturdy(pos, BlockFace::Up, support),
                        Some(true)
                    );
                    assert_eq!(
                        level.block_face_sturdy([4, -60, 5], BlockFace::Up, support),
                        Some(false)
                    );
                    assert_eq!(
                        level.block_face_sturdy(unloaded, BlockFace::Up, support),
                        None
                    );
                }
                assert_eq!(
                    level.block_face_sturdy(above, BlockFace::Up, SupportType::Full),
                    None
                );
                assert_eq!(level.can_see_sky(pos), Some(true));
                assert_eq!(level.can_see_sky(unloaded), Some(true));
                assert_eq!(level.can_see_sky(above), Some(true));
            });
            // A dimension type with the same range but no sky layer cannot see the sky.
            game.world.shape.sky_light = false;
            assert_eq!(level.can_see_sky(game, above), Some(false));
            game.with_level(|_, level, _| assert_eq!(level.can_see_sky(above), Some(false)));
            assert!(!game.world.loaded(unloaded[0], unloaded[2]));
            // A view without a live client level still refuses, even with a column in memory.
            game.client.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(level.can_see_sky(game, pos), None);
            assert_eq!(
                level.block_face_sturdy(game, pos, BlockFace::Up, SupportType::Rigid),
                None
            );
            game.with_level(|_, level, _| {
                assert_eq!(
                    level.block_face_sturdy(pos, BlockFace::Up, SupportType::Rigid),
                    None
                );
                assert_eq!(level.can_see_sky(pos), None);
            });
        });
    });
}

/// `EntityRef::is_sitting` answers each mob's own test (`Fox.isSitting`, `Panda.isSitting`,
/// `Camel.isCamelSitting`, `TamableAnimal.isInSittingPose`), and none for a kind without one or
/// a stale handle.
#[test]
#[ignore = "daily"]
fn is_sitting_reads_each_mobs_own_test() {
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::ServerGame;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::mobs::{Mob, fox, panda};
    with_game(|game| {
        let kinds = [
            "minecraft:wolf",
            "minecraft:fox",
            "minecraft:panda",
            "minecraft:camel",
            "minecraft:cow",
        ];
        for (i, kind) in kinds.iter().enumerate() {
            let kind = game.res.mob_types().get(*kind).unwrap().clone();
            game.mobs
                .push(Mob::new(40 + i as u32, kind, Vec3::ZERO, 0.0));
        }
        game.server.next_entity = game.server.next_entity.max(44);
        // An entity is found only in a loaded column.
        game.world
            .chunks
            .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
        let read =
            |game: &ServerGame<'_>, id: u32| game.entity_is_sitting(game.entity(id).unwrap());
        for id in 40..44 {
            assert_eq!(read(game, id), Some(false), "mob {id} stands");
        }
        assert_eq!(read(game, 44), None, "a cow has no sitting test");
        fn mob<'a>(game: &'a mut ServerGame<'_>, id: u32) -> &'a mut Mob {
            game.mobs.iter_mut().find(|m| m.id == id).unwrap()
        }
        mob(game, 40).extra.sitting_pose = true;
        mob(game, 41).extra.ints[0] = fox::SITTING;
        panda::set_flags(mob(game, 42), panda::FLAG_SIT);
        mob(game, 43).extra.camel_sitting = true;
        for id in 40..44 {
            assert_eq!(read(game, id), Some(true), "mob {id} sits");
        }
        // A fox's other flags are not sitting.
        mob(game, 41).extra.ints[0] = fox::CROUCHING;
        assert_eq!(read(game, 41), Some(false));
        let entity = game.entity(40).unwrap();
        let stale = mod_api::EntityRef::new(
            mod_api::WorldEpoch::new(entity.epoch().get() + 1),
            entity.id(),
        );
        assert_eq!(game.entity_is_sitting(stale), None);
    });
}

/// A guest's sprinting is written to its row, so it survives the turn boundary the body is
/// rebuilt at, and its speed modifier stays with the flag; a stale handle sets nothing.
#[test]
#[ignore = "daily"]
fn a_guests_sprinting_survives_a_turn() {
    with_guests(|game: &mut sunrise_gameplay::game::ServerGame<'_>| {
        let entity = game.player(13).unwrap().entity();
        // A guest has no body until its first turn.
        assert!(game.as_guest(13, |_| ()).is_some());
        assert_eq!(game.entity_is_sprinting(entity), Some(false));
        let stored = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            let lan = game.server.lan.as_mut().unwrap();
            let flag = lan.hub.update_guest(13, |g| g.sprinting).unwrap();
            let modifier = lan.hub.guests.motion_mut(13).unwrap().has_sprint_modifier();
            (flag, modifier)
        };
        assert!(game.entity_set_sprinting(entity, true));
        assert_eq!(stored(game), (true, true));
        assert_eq!(game.entity_is_sprinting(entity), Some(true));
        // The next turn starts from the row, and ends with the row still set.
        let mut in_turn = None;
        game.as_guest(13, |game| {
            in_turn = Some(game.entity_is_sprinting(entity));
        });
        assert_eq!(in_turn, Some(Some(true)));
        assert_eq!(stored(game), (true, true));
        // Inside a turn the write goes to the row as well.
        game.as_guest(13, |game| assert!(game.entity_set_sprinting(entity, false)));
        assert_eq!(stored(game), (false, false));
        let stale = mod_api::EntityRef::new(
            mod_api::WorldEpoch::new(entity.epoch().get() + 1),
            entity.id(),
        );
        assert!(!game.entity_set_sprinting(stale, true));
    });
}

/// `EntityRef::armor_stand_pose` and `armor_stand_flags` read the stand's pose rotations and
/// client flags on a server and on a client; none for another kind or a stale handle.
#[test]
#[ignore = "daily"]
fn armor_stand_pose_and_flags_read_the_stand() {
    use super::native_entity_refs::tests::with_client_game;
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::entities::ArmorStand;
    fn check<'g>(game: &mut impl NativeEntityStateCommonGameExt1<'g>) {
        // An entity is found only in a loaded column.
        game.common_mut().world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        // A server spawns the stand here; a client has it from the server's AddEntity.
        if !game.common().armor_stands.iter().any(|s| s.id == 21) {
            game.common_mut().armor_stands.push(ArmorStand::new(
                21,
                Vec3::new(0.5, 64.0, 0.5),
                0.0,
            ));
        }
        let stand = game
            .common_mut()
            .armor_stands
            .iter_mut()
            .find(|s| s.id == 21)
            .unwrap();
        stand.pose = [
            [1.0, 2.0, 3.0],
            [4.0, 5.0, 6.0],
            [-10.0, 0.0, -10.0],
            [-15.0, 0.0, 10.0],
            [-1.0, 0.0, -1.0],
            [1.0, 0.0, 1.0],
        ];
        stand.small = true;
        stand.show_arms = true;
        stand.no_base_plate = true;
        let entity = game.entity(21).expect("the stand's handle");
        assert_eq!(
            game.entity_armor_stand_pose(entity),
            Some(mod_api::ArmorStandPose {
                head: [1.0, 2.0, 3.0],
                body: [4.0, 5.0, 6.0],
                left_arm: [-10.0, 0.0, -10.0],
                right_arm: [-15.0, 0.0, 10.0],
                left_leg: [-1.0, 0.0, -1.0],
                right_leg: [1.0, 0.0, 1.0],
            })
        );
        assert_eq!(
            game.entity_armor_stand_flags(entity),
            Some(mod_api::ArmorStandFlags {
                small: true,
                show_arms: true,
                show_base_plate: false,
                marker: false,
            })
        );
        let stale = mod_api::EntityRef::new(
            mod_api::WorldEpoch::new(entity.epoch().get() + 1),
            entity.id(),
        );
        assert_eq!(game.entity_armor_stand_pose(stale), None);
        assert_eq!(game.entity_armor_stand_flags(stale), None);
    }
    with_game(|game| {
        game.server.next_entity = game.server.next_entity.max(21);
        check(game);
        // Another kind has neither.
        let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
        game.mobs
            .push(sunrise_sim::mobs::Mob::new(22, cow, Vec3::ZERO, 0.0));
        game.server.next_entity = game.server.next_entity.max(22);
        let cow = game.entity(22).unwrap();
        assert_eq!(game.entity_armor_stand_pose(cow), None);
        assert_eq!(game.entity_armor_stand_flags(cow), None);
    });
    with_client_game(|game| {
        use sunrise_protocol::packets::play::{self, Clientbound as C};
        let _server =
            sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client(
                game,
            );
        let kind = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:armor_stand")
            .unwrap();
        game.handle_packet(C::AddEntity(play::AddEntity {
            id: 21,
            uuid: 21,
            kind,
            position: [0.5, 64.0, 0.5],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        }))
        .unwrap();
        check(game);
    });
}

/// The server's frozen state follows `/tick freeze`; a client has none.
#[test]
#[ignore = "daily"]
fn the_tick_rate_is_frozen_on_a_server_only() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.clock.set_frozen(true);
        assert_eq!(game.tick_rate_is_frozen(), Some(true));
        game.clock.set_frozen(false);
        assert_eq!(game.tick_rate_is_frozen(), Some(false));
        game.options.execution_role = ExecutionRole::Client;
        game.options.dedicated = None;
        assert_eq!(game.tick_rate_is_frozen(), None);
    });
}

/// Menu reads follow the local client's prediction and reject every other player.
#[test]
#[ignore = "daily"]
fn a_client_reads_only_its_own_menu() {
    mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
        super::native_entity_refs::tests::with_client_game(|game| {
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "MenuReadTest").unwrap();
            game.client.remote.as_mut().unwrap().logged_in = true;
            let player = game.local_player().unwrap();
            let other = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(player.entity().epoch(), player.entity().id() + 1),
                42,
            );
            assert_eq!(game.player_container_menu(other), None);
            let stack = sunrise_items::items::ItemStack::new("minecraft:stone", 7);
            game.inventory[0] = Some(stack.clone());
            game.carried = Some(stack.clone());
            let inventory = game.player_container_menu(player).unwrap();
            assert_eq!(game.menu_slot_count(inventory), Some(46));
            assert_eq!(game.menu_slot(inventory, 36), Some(Some(stack.clone())));
            assert_eq!(game.menu_carried(inventory), Some(Some(stack.clone())));
            game.client
                .remote
                .as_mut()
                .unwrap()
                .menu
                .set(Some((1, "minecraft:generic_9x3".into())));
            let open = game.player_container_menu(player).unwrap();
            assert_eq!(game.menu_slot_count(open), Some(63));
            assert_eq!(game.menu_slot(open, 54), Some(Some(stack)));
            assert_eq!(game.menu_slot_count(inventory), None);
            game.client.remote.as_mut().unwrap().menu.set(None);
            assert_eq!(game.menu_slot_count(open), None);
            assert_eq!(
                game.menu_slot_count(game.player_container_menu(player).unwrap()),
                Some(46)
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn client_merchant_offers_follow_the_open_menu() {
    use super::native_entity_refs::tests::with_client_game;
    use mod_api::__private::game::Fields;
    use mod_api::ClientHost;
    use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;
    use sunrise_core::nbt::Tag;
    use sunrise_game_data::text::Text;
    use sunrise_protocol::{
        item::{Component, Stack, component::STORED_ENCHANTMENTS},
        packets::{
            configuration::{RegistryData, RegistryEntry},
            play,
        },
        registry::Builtin,
    };
    mod_platform::testing::with_mods(Default::default(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            let remote = game.client.remote.as_mut().unwrap();
            remote.builtin = std::sync::Arc::new(
                Builtin::parse(
                    r#"{
                "minecraft:item":{"entries":{
                    "minecraft:emerald":{"protocol_id":1},
                    "minecraft:book":{"protocol_id":2},
                    "minecraft:enchanted_book":{"protocol_id":3}
                }},
                "minecraft:data_component_type":{"entries":{
                    "minecraft:stored_enchantments":{"protocol_id":45}
                }}
            }"#,
                )
                .unwrap(),
            );
            std::sync::Arc::make_mut(&mut remote.registries).add(RegistryData {
                registry: "minecraft:enchantment".into(),
                entries: vec![RegistryEntry {
                    id: "minecraft:mending".into(),
                    data: None,
                }],
            });
            let current_menu =
                |host: &dyn ClientHost| host.local_player().unwrap().container_menu(host).unwrap();
            let inventory = current_menu(game);
            assert_eq!(inventory.merchant_offers(game), Ok(None));

            let window = 3;
            game.remote_open_merchant(window, String::new(), Text::literal("Merchant"));
            let menu = current_menu(game);
            assert_eq!(menu.merchant_offers(game), Ok(Some(vec![])));
            let player = menu.fields().player;
            let foreign = mod_api::__private::game::menu_ref(
                mod_api::__private::game::player_ref(player.entity(), 99),
                window,
                menu.fields().generation,
            );
            assert_eq!(foreign.merchant_offers(game), Ok(None));

            let discounted_book = play::NetOffer {
                cost_a: Stack::new(1, 10).into(),
                cost_b: Some(Stack::new(2, 1).into()),
                result: Stack {
                    added: vec![(STORED_ENCHANTMENTS, Component::Enchantments(vec![(0, 1)]))],
                    ..Stack::new(3, 1)
                },
                out_of_stock: false,
                uses: 2,
                max_uses: 12,
                xp: 5,
                special_price: -3,
                price_multiplier: 0.5,
                demand: 1,
            };
            let exhausted = play::NetOffer {
                result: Stack::new(2, 1),
                cost_b: None,
                out_of_stock: true,
                ..discounted_book.clone()
            };
            game.handle_packet(play::Clientbound::MerchantOffers(play::MerchantOffers {
                container: window,
                offers: vec![discounted_book, exhausted],
                level: 2,
                xp: 10,
                show_progress: true,
                can_restock: true,
            }))
            .unwrap();
            let offers = menu.merchant_offers(game).unwrap().unwrap();
            let [book, exhausted] = offers.as_slice() else {
                panic!("expected both synchronized offers");
            };
            let item = |id| mod_api::__private::item_key(id).unwrap();
            assert_eq!(book.base_cost_a.stack.item(), item("minecraft:emerald"));
            assert_eq!(book.base_cost_a.stack.count(), 10);
            let second = &book.cost_b.as_ref().unwrap().stack;
            assert_eq!((second.item(), second.count()), (item("minecraft:book"), 1));
            assert_eq!(
                (book.result.item(), book.result.count()),
                (item("minecraft:enchanted_book"), 1)
            );
            assert_eq!(
                book.result
                    .components()
                    .unwrap()
                    .as_nbt()
                    .get(&"minecraft:stored_enchantments".into()),
                Some(&Tag::compound([("minecraft:mending", Tag::Int(1))])),
            );
            assert_eq!((book.uses, book.max_uses, book.xp), (2, 12, 5));
            assert_eq!(
                (book.special_price_diff, book.demand, book.price_multiplier),
                (-3, 1, 0.5)
            );
            assert_eq!(book.cost_a_count(game), Some(12));
            assert!(!book.out_of_stock());
            assert_eq!(exhausted.result.item(), item("minecraft:book"));
            assert!(exhausted.cost_b.is_none());
            assert!(exhausted.out_of_stock());

            // The server may reuse the window id, but not the handle's generation.
            game.remote_open_merchant(window, String::new(), Text::literal("Merchant"));
            let replacement = current_menu(game);
            assert_eq!(menu.merchant_offers(game), Ok(None));
            assert_eq!(replacement.merchant_offers(game), Ok(Some(vec![])));
            game.remote_close_local();
            assert_eq!(replacement.merchant_offers(game), Ok(None));
        });
    });
}

#[test]
#[ignore = "daily"]
fn client_player_state_reads_its_own_copy_only() {
    use super::native_entity_refs::tests::with_client_game;
    use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;
    use sunrise_core::geometry::Vec3;
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::{
        motion::{MotionWorld, PlayerMotion},
        pose::Pose,
    };
    mod_platform::testing::with_mods(Default::default(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            game.client
                .streaming
                .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
            game.common.player =
                Some(PlayerMotion::new(game.common.motion, Vec3::new(8.5, 64.0, 8.5)).unwrap());
            let local = game.local_player().unwrap();
            let entity = local.entity();
            let living = mod_api::LivingRef::new(entity);
            let other = mod_api::EntityRef::new(entity.epoch(), 99);
            let other_living = mod_api::LivingRef::new(other);
            let other_player = mod_api::__private::game::player_ref(other, 99);
            let air = game.common.world.air;
            let lava = game
                .pack
                .assets
                .registry
                .default_state("minecraft:lava")
                .unwrap();
            game.common.inventory[5] = Some(ItemStack::new("minecraft:stone", 3));
            for flag in [true, false] {
                game.common
                    .world
                    .set(8, 64, 8, if flag { lava } else { air })
                    .unwrap();
                {
                    let body = game.common.player.as_mut().unwrap();
                    body.fluids.first_tick = false;
                }
                {
                    let level = game.native_client_level();
                    let remote = game.client.remote.as_ref().unwrap();
                    let side = sunrise_gameplay_common::game_side!(
                        game.common,
                        (None, level, false),
                        &remote.features,
                        remote.tags.as_ref()
                    );
                    let mut world = MotionWorld {
                        side: &side,
                        entities: &[],
                        player: game.common.player.as_mut().unwrap(),
                        scene: &mut game.common.world,
                        assets: &game.common.pack.assets,
                        collision_binding: game.common.collision,
                        resources: game.common.motion,
                        health: &game.common.health_resources,
                        tag_items: None,
                        loop_context: &game.common.loop_context,
                        damage_hooks: &game.common.damage_hooks,
                        block_registries: None,
                    };
                    world.fluid_base_tick().unwrap();
                }
                let body = game.common.player.as_mut().unwrap();
                body.ground.on_ground = flag;
                body.fluids.was_touching_water = flag;
                body.swimming = flag;
                body.sprinting = flag;
                body.pose = if flag {
                    Pose::Crouching
                } else {
                    Pose::Standing
                };
                body.local_crouching = flag;
                body.no_jump_delay = if flag { 10 } else { 0 };
                body.air = if flag { 87 } else { 12 };
                let host = &*game as &dyn mod_api::ClientHost;
                assert_eq!(entity.on_ground(host), Some(flag));
                assert_eq!(entity.is_in_water(host), Some(flag));
                assert_eq!(entity.is_in_lava(host), Some(flag));
                assert_eq!(entity.is_swimming(host), Some(flag));
                assert_eq!(entity.is_sprinting(host), Some(flag));
                assert_eq!(
                    entity.pose(host),
                    Some(if flag { "crouching" } else { "standing" })
                );
                assert_eq!(entity.is_crouching(host), Some(flag));
                assert_eq!(living.no_jump_delay(host), Some(if flag { 10 } else { 0 }));
                assert_eq!(entity.air_supply(host), Some(if flag { 87 } else { 12 }));
                assert_eq!(entity.max_air_supply(host), Some(300));
                assert_eq!(other.on_ground(host), None);
                assert_eq!(other.is_in_water(host), None);
                assert_eq!(other.is_in_lava(host), None);
                assert_eq!(other.is_swimming(host), None);
                assert_eq!(other.is_sprinting(host), None);
                assert_eq!(other.pose(host), None);
                assert_eq!(other.is_crouching(host), None);
                assert_eq!(other_living.no_jump_delay(host), None);
                assert_eq!(other.air_supply(host), None);
                assert_eq!(other.max_air_supply(host), None);
                local
                    .with_inventory(host, |view| {
                        let mut slots = view.iter();
                        let (slot, stack) = slots.next().unwrap();
                        assert_eq!((slot, stack.count()), (5, 3));
                        assert!(slots.next().is_none());
                    })
                    .unwrap();
                assert_eq!(other_player.with_inventory(host, |_| ()), None);
                assert_eq!(local.with_ender_chest(host, |_| ()), None);
            }
            let body = game.common.player.as_mut().unwrap();
            body.pose = Pose::Crouching;
            body.local_crouching = false;
            let host = &*game as &dyn mod_api::ClientHost;
            assert_eq!(entity.pose(host), Some("crouching"));
            assert_eq!(entity.is_crouching(host), Some(false));
            let body = game.common.player.as_mut().unwrap();
            body.pose = Pose::Standing;
            body.local_crouching = true;
            let host = &*game as &dyn mod_api::ClientHost;
            assert_eq!(entity.pose(host), Some("standing"));
            assert_eq!(entity.is_crouching(host), Some(true));
        });
    });
}

/// `ClientLevel.tickRateManager()`: the rate, freeze and steps the ticking packets set
/// (`handleTickingState` / `handleTickingStep`), from `Login` on, before any player is placed;
/// none before a client level exists.
#[test]
#[ignore = "daily"]
fn tick_rate_reads_the_clients_tick_rate_manager() {
    use sunrise_protocol::{
        connection::Pipe,
        packets::play::{self, Clientbound as C},
    };
    mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
        super::native_entity_refs::tests::with_client_game(|game| {
            let read = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                mod_api::prelude::ClientViewClientHost::tick_rate(game)
            };
            assert!(game.native_client_level().is_none());
            assert_eq!(read(game), None);
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "TickRateTest").unwrap();
            assert_eq!(read(game), None, "connected, but no level before Login");
            let spawn = play::SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 0,
                game_type: 0,
                previous_game_type: None,
                debug: false,
                flat: false,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            };
            game.handle_packet(C::Login(play::Login {
                player_id: 7,
                hardcore: false,
                levels: vec![spawn.dimension.clone()],
                max_players: 1,
                chunk_radius: 2,
                simulation_distance: 2,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn,
                online_mode: false,
                enforces_secure_chat: false,
            }))
            .unwrap();
            game.player = None;
            let before = read(game).expect("Login makes the client level");
            assert_eq!(before.tick_rate, 20.0);
            game.handle_packet(C::TickingState(play::TickingState {
                rate: 5.0,
                frozen: true,
            }))
            .unwrap();
            game.handle_packet(C::TickingStep(play::TickingStep { steps: 7 }))
                .unwrap();
            assert_eq!(
                read(game),
                Some(mod_api::ClientTickRate {
                    tick_rate: 5.0,
                    frozen: true,
                    frozen_ticks_to_run: 7,
                }),
                "read before the player is placed"
            );
        });
    });
}

/// A client reads a tracked player's and a tracked mob's pose from the `DATA_POSE` the server
/// sent, and has no jump cooldown for either.
#[test]
#[ignore = "daily"]
fn client_tracked_entities_read_their_synced_pose() {
    use super::native_entity_refs::tests::with_client_game;
    use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;
    use sunrise_protocol::packets::play::{self, Clientbound as C};
    fn add_entity(
        game: &sunrise_client_gameplay::game::ClientGame<'_>,
        id: i32,
        uuid: u128,
        kind: &str,
    ) -> C {
        let kind = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id(kind)
            .unwrap();
        C::AddEntity(play::AddEntity {
            id,
            uuid,
            kind,
            position: [2.5, 64.0, 2.5],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        })
    }
    mod_platform::testing::with_mods(Default::default(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            let player_uuid = 0x5e4d_0001_u128;
            game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                actions: 0x01 | 0x04 | 0x08,
                entries: vec![play::PlayerInfo {
                    uuid: player_uuid,
                    name: Some("Tracked".into()),
                    game_mode: Some(0),
                    listed: Some(true),
                    ..play::PlayerInfo::default()
                }],
            }))
            .unwrap();
            let pkt = add_entity(game, 301, player_uuid, "minecraft:player");
            game.handle_packet(pkt).unwrap();
            let pkt = add_entity(game, 302, 0x5e4d_0002, "minecraft:zombie");
            game.handle_packet(pkt).unwrap();
            // As SetEntityData's DATA_POSE leaves them: a player's in `synced_pose`, a mob's in
            // `pose` (crouching is 5, swimming 3).
            for mob in game.mobs.iter_mut() {
                match mob.id {
                    301 => mob.extra.synced_pose = 5,
                    302 => mob.extra.pose = 3,
                    _ => {}
                }
            }
            let player = game.entity(301).expect("the tracked player");
            let zombie = game.entity(302).expect("the tracked zombie");
            let host = &*game as &dyn mod_api::ClientHost;
            assert_eq!(player.pose(host), Some("crouching"));
            assert_eq!(player.is_crouching(host), Some(true));
            assert_eq!(zombie.pose(host), Some("swimming"));
            assert_eq!(zombie.is_crouching(host), Some(false));
            for entity in [player, zombie] {
                assert_eq!(mod_api::LivingRef::new(entity).no_jump_delay(host), None);
            }
        });
    });
}

/// A server reads a mob's jump cooldown, `LivingEntity.noJumpDelay`, from its own body.
#[test]
#[ignore = "daily"]
fn a_server_reads_a_mobs_jump_cooldown() {
    use std::sync::Arc;
    use sunrise_core::nbt::{Compound, Tag};
    use sunrise_level::chunk::Chunk;
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(sunrise_gameplay_common::game::Dim::Overworld);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.server.streaming.forced.insert((0, 0));
        game.native_joins_skip();
        let nbt: Compound = [("NoAI".into(), Tag::Byte(1))].into_iter().collect();
        let level = game.level("minecraft:overworld").expect("the overworld");
        let entity = game
            .level_summon(level, "minecraft:pig", [1.5, 70.0, 1.5], nbt)
            .expect("a level write")
            .expect("a pig");
        let id = entity.id();
        for delay in [7, 0] {
            game.mobs
                .iter_mut()
                .find(|m| m.id == id)
                .unwrap()
                .ai
                .no_jump_delay = delay;
            let host = &*game as &dyn mod_api::Host;
            assert_eq!(
                mod_api::LivingRef::new(entity).no_jump_delay(host),
                Some(delay)
            );
        }
    });
}

/// The HUD clock is a client clock: pause stops it, server freeze does not.
#[test]
#[ignore = "daily"]
fn gui_ticks_follow_unpaused_client_ticks() {
    mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
        super::native_entity_refs::tests::with_client_game(|game| {
            assert_eq!(game.gui_ticks(), 0);
            game.client_tick_steps(2, None).unwrap();
            assert_eq!(game.gui_ticks(), 2);
            game.pause = true;
            game.client_tick_steps(3, None).unwrap();
            assert_eq!(game.gui_ticks(), 2);
            game.pause = false;
            game.clock.set_frozen(true);
            game.client_tick_steps(1, None).unwrap();
            assert_eq!(game.gui_ticks(), 3);
            game.client.gui_ticks = i32::MAX;
            game.client_tick_steps(1, None).unwrap();
            assert_eq!(game.gui_ticks(), i32::MIN);
        });
    });
}

#[test]
#[ignore = "daily"]
fn client_abilities_follow_predicted_flying_between_packets() {
    use super::native_entity_refs::tests::with_client_game;
    use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::packets::play;
    use sunrise_sim::motion::PlayerMotion;
    mod_platform::testing::with_mods(Default::default(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.handle_packet(play::Clientbound::PlayerAbilities(play::PlayerAbilities {
                flags: 4,
                flying_speed: 0.1,
                walking_speed: 0.2,
            }))
            .unwrap();
            let local = game.local_player().unwrap();
            for flying in [true, false] {
                game.player.as_mut().unwrap().flying = flying;
                let abilities = local.abilities(&*game as &dyn mod_api::ClientHost).unwrap();
                assert_eq!(abilities.flying, flying);
                assert!(abilities.may_fly);
                assert!(!abilities.invulnerable);
                assert!(!abilities.instabuild);
                assert_eq!((abilities.fly_speed, abilities.walk_speed), (0.1, 0.2));
            }
        });
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_menu_refs_common::NativeMenuRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
