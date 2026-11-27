/// The local player's own state reads answer on a client from what the
/// server sent it (`LocalPlayer`); another player reads None there.
#[test]
#[ignore = "daily"]
fn client_player_reads_answer_from_the_local_players_copy() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::{EntityRef, GameMode};
    use sunrise_client::level_loading::Reason;
    use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::{
        connection::Pipe,
        packets::play::{self, Clientbound as C, SpawnInfo},
    };
    use sunrise_sim::motion::PlayerMotion;

    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "PlayerReadTest").unwrap();
        let spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        let remote = game.client.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        remote.profile = 42;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 65.0, 0.5)).unwrap());
        let player = game.local_player().unwrap();

        // Before ADD_PLAYER, the entity's PlayerInfo is unresolved (`AbstractClientPlayer.gameMode()`
        // is null), while the controller mode and abilities reflect Login (Survival).
        assert_eq!(game.mode_id(), 0);
        assert_eq!(player.game_mode(game), None);
        assert_eq!(player.is_creative(game), Some(false));
        assert_eq!(player.is_spectator(game), Some(false));
        assert_eq!(player.may_build(game), Some(true));
        assert_eq!(player.abilities(game).map(|a| a.may_fly), Some(false));

        // CHANGE_GAME_MODE updates controller mode and abilities without creating PlayerInfo.
        game.handle_packet(C::GameEvent(play::GameEvent {
            event: 3,
            param: 2.0,
        }))
        .unwrap();
        assert_eq!(game.mode_id(), 2);
        assert_eq!(player.game_mode(game), None);
        assert_eq!(player.is_creative(game), Some(false));
        assert_eq!(player.is_spectator(game), Some(false));
        assert_eq!(player.may_build(game), Some(false));
        // The parameter rounds (Mth.floor(param + 0.5F)): 0.6 is creative, 1.6 adventure again.
        for (param, mode_id) in [(0.6, 1), (1.6, 2)] {
            game.handle_packet(C::GameEvent(play::GameEvent { event: 3, param }))
                .unwrap();
            assert_eq!(game.mode_id(), mode_id, "{param}");
        }
        game.handle_packet(C::PlayerAbilities(play::PlayerAbilities {
            flags: 4 | 2,
            flying_speed: 0.1,
            walking_speed: 0.2,
        }))
        .unwrap();
        let abilities = player.abilities(game).unwrap();
        assert!(abilities.may_fly && abilities.flying && !abilities.invulnerable);
        assert!(!abilities.may_build);
        assert_eq!((abilities.fly_speed, abilities.walk_speed), (0.1, 0.2));

        // An invalid game mode falls back to Survival in the controller and its remote snapshot.
        game.handle_packet(C::GameEvent(play::GameEvent {
            event: 3,
            param: 4.0,
        }))
        .unwrap();
        assert_eq!(game.mode_id(), 0);
        assert_eq!(game.client.remote.as_ref().unwrap().game_mode, 0);

        // Once ADD_PLAYER arrives, PlayerRef mode queries follow the entity's resolved PlayerInfo,
        // independent of controller mode (CHANGE_GAME_MODE) and abilities.
        game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: 0x01 | 0x04 | 0x08,
            entries: vec![play::PlayerInfo {
                uuid: 42,
                name: Some("PlayerReadTest".into()),
                game_mode: Some(2),
                listed: Some(true),
                ..play::PlayerInfo::default()
            }],
        }))
        .unwrap();
        game.handle_packet(C::GameEvent(play::GameEvent {
            event: 3,
            param: 3.0,
        }))
        .unwrap();
        assert_eq!(game.mode_id(), 3);
        assert_eq!(player.game_mode(game), Some(GameMode::Adventure));
        assert_eq!(player.is_creative(game), Some(false));
        assert_eq!(player.is_spectator(game), Some(false));
        assert_eq!(player.abilities(game).map(|a| a.may_fly), Some(true));

        for (info_id, expected, creative, spectator) in [
            (1, GameMode::Creative, true, false),
            (3, GameMode::Spectator, false, true),
            (0, GameMode::Survival, false, false),
        ] {
            game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                actions: 0x04,
                entries: vec![play::PlayerInfo {
                    uuid: 42,
                    game_mode: Some(info_id),
                    ..play::PlayerInfo::default()
                }],
            }))
            .unwrap();
            assert_eq!(player.game_mode(game), Some(expected));
            assert_eq!(player.is_creative(game), Some(creative));
            assert_eq!(player.is_spectator(game), Some(spectator));
        }

        // Food from SetHealth; the client keeps no exhaustion.
        game.handle_packet(C::SetHealth(play::SetHealth {
            health: 20.0,
            food: 5,
            saturation: 1.5,
        }))
        .unwrap();
        let food = player.food(game).unwrap();
        assert_eq!(
            (food.level, food.saturation, food.exhaustion),
            (5, 1.5, 0.0)
        );
        assert_eq!(player.needs_food(game), Some(true));
        assert_eq!(player.has_enough_food(game), Some(false));

        // The hotbar slot and the experience the server set.
        game.handle_packet(C::SetHeldSlot(play::SetHeldSlot { slot: 4 }))
            .unwrap();
        assert_eq!(player.selected_slot(game), Some(4));
        game.handle_packet(C::SetExperience(play::SetExperience {
            progress: 0.25,
            level: 3,
            total: 30,
        }))
        .unwrap();
        let xp = player.experience(game).unwrap();
        assert_eq!((xp.level, xp.progress, xp.total), (3, 0.25, 30));
        assert_eq!(player.xp_needed_for_next_level(game), Some(13));

        // The main inventory retains typed stacks and their damage, in slot order.
        let mut stack = sunrise_items::items::ItemStack::new("minecraft:diamond_pickaxe", 1);
        stack.damage = 7;
        game.inventory.fill(None);
        game.inventory[3] = Some(stack);
        game.inventory[8] = Some(sunrise_items::items::ItemStack::new("minecraft:stone", 12));
        player
            .with_inventory(game, |view| {
                let mut slots = view.iter();
                let (slot, pickaxe) = slots.next().unwrap();
                let expected = mod_api::__private::item_key("minecraft:diamond_pickaxe").unwrap();
                assert_eq!(
                    (slot, pickaxe.item(), pickaxe.count(), pickaxe.damage()),
                    (3, expected, 1, 7)
                );
                let (slot, stone) = slots.next().unwrap();
                let expected = mod_api::__private::item_key("minecraft:stone").unwrap();
                assert_eq!((slot, stone.item(), stone.count()), (8, expected, 12));
                assert!(slots.next().is_none());
            })
            .unwrap();

        // Another player: the client isn't sent their state.
        let epoch = player.entity().epoch();
        let other = mod_api::__private::game::player_ref(EntityRef::new(epoch, 8), 43);
        assert_eq!(other.game_mode(game), None);
        assert_eq!(other.food(game), None);
        assert_eq!(other.selected_slot(game), None);
        assert_eq!(other.with_inventory(game, |_| ()), None);
    });
}

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
