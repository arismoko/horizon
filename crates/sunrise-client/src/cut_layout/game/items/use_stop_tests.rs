use super::item_use::interact_vibration_tests::with_game;

#[test]
#[ignore = "daily"]
fn spectator_stops_use_at_player_feet() {
    let lines = with_game("minecraft:golden_apple", |game| {
        assert!(game.start_using_item());
        game.set_mode(sunrise_command::args::GameMode::Spectator);
        assert!(game.using.is_none());
    })
    .expect("prepared resources");
    assert_eq!(
        lines,
        [
            "G 10,64,-4 minecraft:item_interact_start",
            "G 10,64,-4 minecraft:item_interact_finish",
        ]
    );
}

#[test]
#[ignore = "daily"]
fn stopping_spear_use_is_silent() {
    let lines = with_game("minecraft:iron_spear", |game| {
        assert!(game.start_using_item());
        game.stop_active_item_use();
        assert!(game.using.is_none());
    })
    .expect("prepared resources");
    assert!(lines.is_empty(), "{lines:?}");
}

fn guest_action(item: &str, action: u8) -> Vec<String> {
    use sunrise_items::items::ItemStack;
    use sunrise_protocol::packets::play::{self, Serverbound};
    use {
        sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
        sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
    };
    with_game(item, |game| {
        game.reset_entity_residency();
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        open_to_lan(game);
        let mut player = guest(1);
        // `with_game` binds the local player to UUID 123; the guest is another profile.
        let guest_uuid = 124;
        player.uuid = guest_uuid;
        player.permission = 4;
        player.position = [21.5, 70.0, -8.25];
        player.inventory[0] = Some(ItemStack::new(item, 1));
        game.server.lan.as_mut().unwrap().hub.guests.push(player);
        game.as_guest(1, |g| assert!(g.start_using_item()));
        match action {
            0 => game.lan_guest_packet(
                1,
                Serverbound::PlayerAction(play::PlayerAction {
                    action: 7,
                    pos: [0; 3],
                    direction: 0,
                    sequence: 0,
                }),
            ),
            1 => game.lan_guest_packet(
                1,
                Serverbound::SetCarriedItem(play::SetCarriedItem { slot: 1 }),
            ),
            2 => {
                let player = game.player(guest_uuid).unwrap();
                assert!(game.player_set_selected_slot(player, 1));
            }
            3 => {
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.health = 0.0);
                game.guest_die(0, 1);
            }
            4 => {
                game.as_guest(1, |g| assert!(g.run_command("tp @s 12 65 -2")));
            }
            _ => unreachable!(),
        }
        game.as_guest(1, |g| {
            assert_eq!(g.using.is_some(), action == 4);
            if action == 0 {
                assert_eq!(g.offhand.as_ref().unwrap().item.as_ref(), item);
                assert!(g.held().is_none());
            }
        });
    })
    .expect("prepared resources")
}

#[test]
#[ignore = "daily"]
fn guest_swap_stops_at_guest_feet() {
    assert_eq!(
        guest_action("minecraft:golden_apple", 0),
        [
            "G 21,70,-9 minecraft:item_interact_start",
            "G 21,70,-9 minecraft:item_interact_finish",
        ]
    );
}

#[test]
#[ignore = "daily"]
fn guest_hotbar_change_stops_at_guest_feet() {
    assert_eq!(
        guest_action("minecraft:golden_apple", 1),
        [
            "G 21,70,-9 minecraft:item_interact_start",
            "G 21,70,-9 minecraft:item_interact_finish",
        ]
    );
}

#[test]
#[ignore = "daily"]
fn mod_hotbar_change_stops_at_guest_feet() {
    assert_eq!(
        guest_action("minecraft:golden_apple", 2),
        [
            "G 21,70,-9 minecraft:item_interact_start",
            "G 21,70,-9 minecraft:item_interact_finish",
        ]
    );
}

#[test]
#[ignore = "daily"]
fn spear_hotbar_change_posts_nothing() {
    assert!(guest_action("minecraft:iron_spear", 1).is_empty());
}

#[test]
#[ignore = "daily"]
fn guest_death_clear_posts_no_finish() {
    assert_eq!(
        guest_action("minecraft:golden_apple", 3),
        [
            "G 21,70,-9 minecraft:item_interact_start",
            "G 21,70,-9 minecraft:entity_die"
        ]
    );
}

#[test]
#[ignore = "daily"]
fn axe_disabled_shield_posts_finish() {
    use sunrise_sim::health::HurtSource;
    use sunrise_sim::health::{BlockOutcome, NO_ATTACK};
    let lines = with_game("minecraft:shield", |game| {
        assert!(game.start_using_item());
        game.server.combat.0.player_attacker = Some(([11.5, 64.0, -3.25], 5.0));
        game.after_block(
            BlockOutcome {
                blocking_item_used: true,
                hurt: false,
                blocked: 4.0,
                shield_damage: 1,
                fully_blocked: true,
                sound: false,
                knockback: false,
                canceled: false,
                dealt: 0.0,
            },
            (None, false),
            HurtSource {
                damage_type: "minecraft:player_attack",
                causing: 1,
                direct: 1,
                position: None,
                direct_type: None,
            },
            &NO_ATTACK,
        );
        assert!(game.using.is_none());
        assert!(game.server.shield_cooldown > 0);
    })
    .expect("prepared resources");
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.ends_with("minecraft:item_interact_finish"))
            .cloned()
            .collect::<Vec<_>>(),
        ["G 10,64,-4 minecraft:item_interact_finish"]
    );
}

#[test]
#[ignore = "daily"]
fn same_dimension_tp_keeps_use() {
    assert_eq!(
        guest_action("minecraft:golden_apple", 4),
        ["G 21,70,-9 minecraft:item_interact_start"]
    );
}

#[test]
#[ignore = "daily"]
fn swap_with_emptied_used_slot_clears_silently() {
    use sunrise_items::items::ItemStack;
    use sunrise_protocol::packets::play::{PlayerAction, Serverbound};
    use {
        sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
        sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
    };
    let lines = with_game("minecraft:golden_apple", |game| {
        open_to_lan(game);
        let mut player = guest(1);
        player.inventory[0] = Some(ItemStack::new("minecraft:golden_apple", 1));
        game.server.lan.as_mut().unwrap().hub.guests.push(player);
        game.as_guest(1, |g| {
            assert!(g.start_using_item());
            *g.held_mut() = None;
        });
        // Only observe the action, after the use-start event.
        sunrise_gameplay::game::server_side::dispense_fixture::stop_recording();
        sunrise_gameplay::game::server_side::dispense_fixture::start_recording();
        game.lan_guest_packet(
            1,
            Serverbound::PlayerAction(PlayerAction {
                action: 7,
                pos: [0; 3],
                direction: 0,
                sequence: 0,
            }),
        );
        game.as_guest(1, |g| assert!(g.using.is_none())).unwrap();
    })
    .unwrap();
    assert!(lines.is_empty(), "{lines:?}");
}

#[test]
#[ignore = "daily"]
fn nether_portal_stops_use_in_target_level() {
    use std::sync::{Arc, Mutex};
    use sunrise_core::{geometry::Vec3, position::BlockPos};
    use sunrise_gameplay_common::game::Dim;
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;
    use {
        sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
        sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
    };
    struct Heard(Arc<Mutex<Vec<(String, [f64; 3])>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Heard {
        fn vanilla_game_event(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::VanillaGameEventEvent<'_>,
        ) -> mod_api::Flow {
            if event.event == "minecraft:item_interact_finish" {
                self.0
                    .lock()
                    .unwrap()
                    .push((event.dimension.to_owned(), event.pos));
            }
            mod_api::Flow::Continue
        }
    }
    let heard = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "portal_finish",
            Box::new(Heard(heard.clone())),
        )]
        .into(),
        || {
            with_game("minecraft:golden_apple", |game| {
                let scratch = tempfile::tempdir().unwrap();
                game.server.world_root = scratch.path().to_path_buf();
                game.reset_entity_residency();
                game.native_lifecycle
                    .begin_open(Some(mod_api::Authority::IntegratedServer));
                open_to_lan(game);
                // Reuse enter_level's normal second-level preparation before the trip.
                game.enter_level(Dim::Nether).unwrap();
                game.enter_level(Dim::Overworld).unwrap();
                let mut chunk = Chunk::empty(0, 0);
                chunk.lit = true;
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                let portal = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:nether_portal")
                    .unwrap();
                game.world.set(8, 70, 8, portal).unwrap();
                let mut player = guest(1);
                player.position = [8.5, 70.0, 8.5];
                player.inventory[0] = Some(ItemStack::new("minecraft:golden_apple", 1));
                game.server.lan.as_mut().unwrap().hub.guests.push(player);
                game.as_guest(1, |g| assert!(g.start_using_item())).unwrap();
                let plan = game.portal_plan(
                    BlockPos::new(8, 70, 8),
                    Vec3::new(8.5, 70.0, 8.5),
                    0.6,
                    1.8,
                    false,
                );
                game.start_server_trip(1, Dim::Nether, Some(plan));
                let player = &game.server.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(player.dimension, "minecraft:the_nether");
                assert_eq!(player.position, [1.5, 70.0, 1.5]);
                game.enter_level(Dim::Nether).unwrap();
                game.as_guest(1, |g| assert!(g.using.is_none())).unwrap();
                game.enter_level(Dim::Overworld).unwrap();
            })
            .unwrap();
        },
    );
    assert_eq!(
        *heard.lock().unwrap(),
        [("minecraft:the_nether".to_owned(), [1.5, 70.0, 1.5])]
    );
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
