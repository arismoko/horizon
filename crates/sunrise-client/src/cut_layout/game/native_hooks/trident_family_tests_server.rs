use crate::game::native_hooks::trident_family_tests::Probe;
use crate::game::native_hooks::trident_family_tests::Seen;
use mod_api::ModEntry;
use std::sync::{Arc, Mutex};
use sunrise_core::{geometry::Vec3, position::ChunkPos};
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::PlayerMotion;
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

pub(crate) fn fixture(
    run: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &Arc<Mutex<Seen>>),
) {
    crate::entry::install_client_hooks();
    let seen = Arc::new(Mutex::new(Seen::default()));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "trident-family",
            Box::new(Probe(seen.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                game.build_registries();
                game.load_server_loot();
                game.set_allow_commands(true);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(0, 0))
                    .unwrap();
                assert!(game.run_command("fill 0 79 0 15 79 15 stone"));
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 80.0, 4.5)).unwrap());
                bind_local_player(game, 11, 123);
                game.set_mode_id(0);
                game.server.next_entity = 100;
                run(game, &seen);
            })
        },
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
