//! `player.sees`, `player.ack_block_changes` and `player.show_block` against real LAN guests,
//! through the public player handle, with a literal test for each refusal.

#[cfg(feature = "client")]
use super::native_entity_refs::tests::with_client_game;
use super::native_guest_turn_tests::with_guests;
use mod_api::{EntityRef, PlayerRef, PlayerWorldRefusal as Refusal, WorldEpoch};
use sunrise_protocol::packets::play::Clientbound as C;

/// The API's value for a block state of the world.
fn api_state<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(
    game: &G,
    name: &str,
) -> mod_api::BlockState {
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    game.pack
        .assets
        .registry
        .default_state(name)
        .unwrap()
        .try_into_api()
        .unwrap()
}

/// The handle of a player of an earlier open of the world.
fn stale(player: PlayerRef) -> PlayerRef {
    let entity = player.entity();
    mod_api::__private::game::player_ref(
        EntityRef::new(WorldEpoch::new(entity.epoch().get() + 1), entity.id()),
        player.uuid(),
    )
}

/// A fake player: no connection, not in the server's list.
fn fake(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> PlayerRef {
    game.server.next_entity = 100;
    let level = game.level(game.dim.id()).unwrap();
    game.fake_player(level, 1234, "machine").unwrap()
}

fn kept(game: &sunrise_gameplay::game::ServerGame<'_>) -> i32 {
    game.server
        .lan
        .as_ref()
        .unwrap()
        .hub
        .guest(13)
        .unwrap()
        .ack_up_to
        .get()
}

/// The highest sequence of the tick waits as the player's ack, in their turn or out of it.
#[test]
#[ignore = "daily"]
fn a_mod_acknowledges_block_changes() {
    with_guests(|game| {
        let b = game.player(13).unwrap();
        assert_eq!(b.ack_block_changes(game, 7), Ok(()));
        assert_eq!(b.ack_block_changes(game, 3), Ok(()));
        assert_eq!(kept(game), 7);
        game.as_guest(13, |game| assert_eq!(b.ack_block_changes(game, 9), Ok(())))
            .unwrap();
        assert_eq!(kept(game), 9);
    });
}

/// Each refusal of the acknowledgement is its own name and leaves the pending ack alone.
#[test]
#[ignore = "daily"]
fn each_block_ack_refusal_has_its_own_name() {
    with_guests(|game| {
        let b = game.player(13).unwrap();
        assert_eq!(
            b.ack_block_changes(game, -1),
            Err(Refusal::NegativeSequence { sequence: -1 })
        );
        assert_eq!(
            b.ack_block_changes(game, 0),
            Ok(()),
            "0 is the first sequence"
        );
        assert_eq!(kept(game), 0);
        assert_eq!(stale(b).ack_block_changes(game, 5), Err(Refusal::Stale));
        let machine = fake(game);
        assert_eq!(
            machine.ack_block_changes(game, 5),
            Err(Refusal::NoConnection)
        );
        assert_eq!(kept(game), 0);
    });
}

/// A block shown to one player goes to their client alone and changes nothing in the world.
#[test]
#[ignore = "daily"]
fn a_block_is_shown_to_one_player() {
    with_guests(|game| {
        let b = game.player(13).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| {
                g.sender = sunrise_sim::lan::PlayerSender::new(tx, Default::default());
            });
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let before = game.world.block(1, 70, 1);
        let shown_state = api_state(game, "minecraft:stone");
        assert_eq!(b.show_block(game, [1, 70, 1], shown_state), Ok(()));
        assert_eq!(game.world.block(1, 70, 1), before);
        let mut shown = Vec::new();
        while let Ok(sunrise_sim::lan::Out::Packet(body)) = rx.try_recv() {
            let mut r = sunrise_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            if let Ok(C::BlockUpdate(u)) = C::decode(id, r.rest()) {
                shown.push((u.pos, u.state));
            }
        }
        assert_eq!(shown, [([1, 70, 1], stone.0 as i32)]);
    });
}

/// Each refusal of `show_block` is its own name and sends nothing; the edges of the packet's
/// position are taken.
#[test]
#[ignore = "daily"]
fn each_show_block_refusal_has_its_own_name() {
    with_guests(|game| {
        let b = game.player(13).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| {
                g.sender = sunrise_sim::lan::PlayerSender::new(tx, Default::default());
            });
        let stone = api_state(game, "minecraft:stone");
        for pos in [
            [1, 2048, 1],
            [1, -2049, 1],
            [1 << 25, 70, 1],
            [1, 70, -(1 << 25) - 1],
        ] {
            assert_eq!(
                b.show_block(game, pos, stone),
                Err(Refusal::PositionNotSendable { pos })
            );
        }
        for pos in [
            [(1 << 25) - 1, 2047, -(1 << 25)],
            [-(1 << 25), -2048, (1 << 25) - 1],
        ] {
            assert_eq!(
                b.show_block(game, pos, stone),
                Ok(()),
                "{pos:?} fits the packet"
            );
        }
        // A mod cannot make a state the world lacks; the game's own check names one.
        assert_eq!(
            game.player_show_block(
                b,
                [1, 70, 1],
                sunrise_world::registry::BlockStateId(u32::MAX)
            ),
            Err(Refusal::UnknownState)
        );
        assert_eq!(
            stale(b).show_block(game, [1, 70, 1], stone),
            Err(Refusal::Stale)
        );
        let machine = fake(game);
        assert_eq!(
            machine.show_block(game, [1, 70, 1], stone),
            Err(Refusal::NoConnection)
        );
        let mut sent = 0;
        while let Ok(sunrise_sim::lan::Out::Packet(body)) = rx.try_recv() {
            let mut r = sunrise_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            sent += matches!(C::decode(id, r.rest()), Ok(C::BlockUpdate(_))) as usize;
        }
        assert_eq!(sent, 2, "only the two positions at the edge went out");
    });
}

/// A player is seen unless a mod hid them below the viewer's level, or they spectate and the
/// viewer doesn't.
#[test]
#[ignore = "daily"]
fn player_sees_follows_the_trackers_rules() {
    with_guests(|game| {
        let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
        assert_eq!(a.sees(game, b), Ok(true));
        assert_eq!(a.sees(game, a), Ok(true));
        assert!(game.player_hide(b, Some(3)));
        assert_eq!(a.sees(game, b), Ok(false));
        assert_eq!(b.sees(game, a), Ok(true));
        let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
        guests.update_connection(12, |g| g.permission = 3);
        assert_eq!(a.sees(game, b), Ok(true));
        assert!(game.player_hide(b, None));
        let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
        guests.update_connection(13, |g| g.game_mode = 3);
        assert_eq!(a.sees(game, b), Ok(false), "a spectator");
        assert_eq!(b.sees(game, a), Ok(true));
    });
}

/// `sees` names a stale handle of either player and a fake player, and a client.
#[test]
#[ignore = "daily"]
fn each_sees_refusal_has_its_own_name() {
    with_guests(|game| {
        let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
        assert_eq!(a.sees(game, stale(b)), Err(Refusal::Stale));
        assert_eq!(stale(a).sees(game, b), Err(Refusal::Stale));
        let machine = fake(game);
        assert_eq!(a.sees(game, machine), Err(Refusal::NoConnection));
        assert_eq!(machine.sees(game, a), Err(Refusal::NoConnection));
    });
}

/// On a client `sees` answers `Client` (server writes are refused at compile time).
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_client_cannot_ack_show_or_ask_who_sees() {
    with_client_game(|game| {
        let ghost = mod_api::__private::game::player_ref(EntityRef::new(WorldEpoch::new(0), 1), 1);
        assert_eq!(ghost.sees(game, ghost), Err(Refusal::Client));
    });
}

/// `hide` and `set_nickname` read back through `hidden` and `nickname`: the player's own value,
/// shown for another, and shown again after being cleared.
#[test]
#[ignore = "daily"]
fn hide_level_and_nickname_read_back() {
    with_guests(|game| {
        let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
        assert!(a.hide(game, Some(2)));
        assert_eq!(a.hidden(game), Ok(Some(2)));
        assert_eq!(b.hidden(game), Ok(None));
        assert!(a.hide(game, None));
        assert_eq!(a.hidden(game), Ok(None));

        assert!(a.set_nickname(game, Some("Cap")));
        assert_eq!(a.nickname(game), Ok(Some("Cap")));
        assert_eq!(b.nickname(game), Ok(None));
        assert!(a.set_nickname(game, None));
        assert_eq!(a.nickname(game), Ok(None));

        // A vanish is kept by UUID after the player leaves; a nickname needs them online.
        assert!(a.hide(game, Some(3)));
        assert!(a.set_nickname(game, Some("Cap")));
        game.server.lan.as_mut().unwrap().hub.guests.clear();
        assert_eq!(a.hidden(game), Ok(Some(3)));
        assert_eq!(a.nickname(game), Err(Refusal::Stale));
    });
}

/// An unpublished single-player world runs a hub for its owner, but nobody else can join it
/// yet: a hide level set there reads back as stored, before and after the world is published,
/// as `hide` stores it, and so does a nickname.
#[test]
#[ignore = "daily"]
fn an_unpublished_single_player_world_reads_the_stored_hide_level() {
    with_guests(|game| {
        let owner = game.player(12).unwrap();
        game.server.serving =
            Some(sunrise_gameplay::game::server_side::integrated::Serving::fixture(0));
        assert!(owner.hide(game, Some(2)));
        assert!(owner.set_nickname(game, Some("Cap")));
        assert_eq!(owner.hidden(game), Ok(Some(2)));
        assert_eq!(owner.nickname(game), Ok(Some("Cap")));
        game.server.serving =
            Some(sunrise_gameplay::game::server_side::integrated::Serving::fixture(25565));
        assert_eq!(owner.hidden(game), Ok(Some(2)));
        assert_eq!(owner.nickname(game), Ok(Some("Cap")));
    });
}

/// A guest's latency is what its keep-alives measured; a fake player has none.
#[test]
#[ignore = "daily"]
fn latency_reads_what_the_keep_alives_measured() {
    use std::sync::atomic::Ordering;
    with_guests(|game| {
        let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
        assert_eq!((a.latency(game), b.latency(game)), (Ok(0), Ok(0)));
        let guests = &game.server.lan.as_ref().unwrap().hub.guests;
        for (conn, ms) in [(12, 45), (13, 120)] {
            guests
                .iter()
                .find(|g| g.conn == conn)
                .unwrap()
                .latency
                .store(ms, Ordering::Relaxed);
        }
        assert_eq!((a.latency(game), b.latency(game)), (Ok(45), Ok(120)));
        let machine = fake(game);
        assert_eq!(machine.latency(game), Ok(0));
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn player_support_uses_the_footprint_then_the_exact_feet_offset_on_both_sides() {
    use mod_api::BlockPos;
    use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;
    use sunrise_core::geometry::Vec3;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;
    use sunrise_world::entity_move::MoverType::SelfMovement;

    fn setup<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(game: &mut G) {
        let slab = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone_slab")
            .unwrap();
        game.world.set(0, 64, 0, slab).unwrap();
    }
    fn land<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(game: &mut G) {
        // The player's centre is over air. Only the edge overlaps the slab.
        game.player.as_mut().unwrap().position = Vec3::new(1.125, 65.0, 0.5);
        game.move_local_player(SelfMovement, Vec3::new(0.0, -1.0, 0.0), false)
            .unwrap();
    }

    with_guests(|game| {
        game.server
            .streaming
            .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
        setup(game);
        let player = game.player(12).unwrap();
        assert!(game.command_guest(12).unwrap().motion.is_none());
        assert_eq!(player.supporting_block(game), Ok(None));
        assert_eq!(player.on_block(game), Ok(Some(BlockPos::new(3, 63, 5))));
        assert_eq!(player.entity().on_block(game), Some([3, 63, 5]));
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |guest| guest.position[1] = 64.1)
            .unwrap();
        assert_eq!(player.on_block(game), Ok(Some(BlockPos::new(3, 64, 5))));
        assert_eq!(player.entity().on_block(game), Some([3, 64, 5]));

        game.as_guest(12, land).unwrap();
        let slab = BlockPos::new(0, 64, 0);
        // Changing the world without moving the guest must not replace the
        // support retained by the guest's last movement with a fresh scan.
        let air = game
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        game.world.set(0, 64, 0, air).unwrap();
        assert_eq!(player.supporting_block(game), Ok(Some(slab)));
        assert_eq!(player.on_block(game), Ok(Some(slab)));
        assert_eq!(player.entity().on_block(game), Some([0, 64, 0]));

        for (dy, block_y) in [(0.5, 64), (0.1, 65)] {
            game.as_guest(12, |game| {
                game.move_local_player(SelfMovement, Vec3::new(0.0, dy, 0.0), false)
                    .unwrap();
            })
            .unwrap();
            assert_eq!(player.supporting_block(game), Ok(None));
            assert_eq!(
                player.on_block(game),
                Ok(Some(BlockPos::new(1, block_y, 0)))
            );
            assert_eq!(player.entity().on_block(game), Some([1, block_y, 0]));
        }
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |guest| guest.position[1] = 66.1)
            .unwrap();
        assert_eq!(player.on_block(game), Ok(Some(BlockPos::new(1, 66, 0))));
        assert_eq!(player.entity().on_block(game), Some([1, 66, 0]));
        assert_eq!(stale(player).supporting_block(game), Ok(None));
        assert_eq!(stale(player).on_block(game), Ok(None));
    });

    with_client_game(|game| {
        let _server = logged_client(game);
        let player = game.local_player().unwrap();
        let mut body = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
        body.entity_id = player.entity().id() as i32;
        game.player = Some(body);
        game.client
            .streaming
            .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
        setup(game);
        land(game);

        let slab = BlockPos::new(0, 64, 0);
        assert_eq!(player.supporting_block(game), Ok(Some(slab)));
        assert_eq!(player.on_block(game), Ok(Some(slab)));
        for (dy, block_y) in [(0.5, 64), (0.1, 65)] {
            game.move_local_player(SelfMovement, Vec3::new(0.0, dy, 0.0), false)
                .unwrap();
            assert_eq!(player.supporting_block(game), Ok(None));
            assert_eq!(
                player.on_block(game),
                Ok(Some(BlockPos::new(1, block_y, 0)))
            );
        }

        let other = mod_api::__private::game::player_ref(
            EntityRef::new(player.entity().epoch(), player.entity().id() + 1),
            player.uuid() + 1,
        );
        for invalid in [stale(player), other] {
            assert_eq!(invalid.supporting_block(game), Ok(None));
            assert_eq!(invalid.on_block(game), Ok(None));
        }
    });
}

/// A stale handle is refused as stale, for all three.
#[test]
#[ignore = "daily"]
fn a_stale_handle_is_refused_latency_hide_level_and_nickname() {
    with_guests(|game| {
        let old = stale(game.player(12).unwrap());
        assert_eq!(old.latency(game), Err(Refusal::Stale));
        assert_eq!(old.hidden(game), Err(Refusal::Stale));
        assert_eq!(old.nickname(game), Err(Refusal::Stale));
    });
}

/// A client has no server list to read: all three name the client.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_client_is_refused_latency_hide_level_and_nickname() {
    with_client_game(|game| {
        let player = mod_api::__private::game::player_ref(EntityRef::new(WorldEpoch::new(0), 1), 1);
        assert_eq!(player.latency(game), Err(Refusal::Client));
        assert_eq!(player.hidden(game), Err(Refusal::Client));
        assert_eq!(player.nickname(game), Err(Refusal::Client));
    });
}

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::piston_push::PistonPushGameExt1 as _;

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
