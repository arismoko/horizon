use std::sync::{Arc, mpsc};
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::packets::play::Clientbound as C;
use sunrise_sim::lan::{Out, PlayerSender, guest::GuestMove, hub::CommittedMove};
use {
    sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
    sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
};

fn run(distance: f64, expected: usize) {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_game(|game| {
            open_to_lan(game);
            game.server
                .streaming
                .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(1, 63, 1, stone).unwrap();
            let mut landing = guest(1);
            landing.position = [1.5, 64.0, 1.5];
            landing.loaded = true;
            landing.on_ground = true;
            let (tx, own) = mpsc::channel();
            landing.sender = PlayerSender::new(tx, Arc::default());
            let mut observer = guest(2);
            observer.position = landing.position;
            observer.loaded = true;
            let (tx, other) = mpsc::channel();
            observer.sender = PlayerSender::new(tx, Arc::default());
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .extend([landing, observer]);
            let target = match game.server.lan.as_ref().unwrap().hub.sound_packet(
                "minecraft:block.stone.fall",
                [1.5, 64.0, 1.5],
                0.5,
                0.75,
                4,
                0,
            ) {
                C::Sound(s) => s.sound,
                _ => unreachable!(),
            };
            game.guest_accepted_move(
                1,
                CommittedMove {
                    horizontal_collision: false,
                    from: [1.5, 64.0 + distance, 1.5],
                    jump_yaw: None,
                    movement: GuestMove {
                        position: [1.5, 64.0, 1.5],
                        delta: [0.0, -distance, 0.0],
                        on_ground: true,
                        sneaking: false,
                    },
                },
            );
            let sounds = |rx: mpsc::Receiver<Out>| {
                rx.try_iter()
                    .flat_map(|out| match out {
                        Out::Packet(b) => vec![b],
                        Out::Packets(b) => b,
                        _ => vec![],
                    })
                    .filter_map(|body| {
                        let mut r = sunrise_protocol::buf::Reader::new(&body);
                        let id = r.varint().unwrap();
                        match C::decode(id, r.rest()).unwrap() {
                            C::Sound(s) if s.sound == target => Some((s.source, s.volume, s.pitch)),
                            _ => None,
                        }
                    })
                    .collect::<Vec<_>>()
            };
            // Source 7 is `SoundSource.PLAYERS`.
            assert_eq!(sounds(other), vec![(7, 0.5, 0.75); expected]);
            assert!(sounds(own).is_empty(), "landing guest must be excluded");
        })
    });
}

#[test]
#[ignore = "daily"]
fn five_block_fall_sends_stone_once_except_guest() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::lan_actions::tick::fall_sound_tests::five_block_fall_sends_stone_once_except_guest",
        || {
            run(5.0, 1);
        },
    );
}
#[test]
#[ignore = "daily"]
fn three_block_fall_sends_no_stone_sound() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::lan_actions::tick::fall_sound_tests::three_block_fall_sends_no_stone_sound",
        || {
            run(3.0, 0);
        },
    );
}
