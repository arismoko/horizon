//! `PlayerRef::respawn_target`: where a player would respawn, as
//! `ServerPlayer.findRespawnPositionAndUseSpawnBlock(false, DO_NOTHING)` finds it, with no respawn.
use mod_api::{RespawnTarget, RespawnTargetUnavailable, ServerHost};
use {
    crate::game::server_side::server_travel::credits_respawn_tests::server_with_a_nether_anchor,
    sunrise_gameplay::game::server_side::server_travel::credits_respawn_tests::charge_anchor,
    sunrise_gameplay::game::server_side::server_travel::credits_respawn_tests::light_spawn_chunks,
    sunrise_gameplay_common::game::Dim, sunrise_gameplay_common::game::player_state::CloneScope,
    sunrise_gameplay_common::game::server_side::server_travel::credits_respawn_tests::ANCHOR,
};

/// Guest 2's answer, asked through its handle from the Overworld, which stays in place.
fn respawn_target(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
) -> Result<RespawnTarget, RespawnTargetUnavailable> {
    let player = game.player(game.command_guest(2).unwrap().uuid).unwrap();
    let host: &mut dyn ServerHost = game;
    let answer = player.respawn_target(host);
    assert_eq!(
        game.dim,
        Dim::Overworld,
        "the caller's level is back in place"
    );
    let guest = game.command_guest(2).unwrap();
    assert_eq!(guest.dimension, Dim::Overworld.id(), "the player stays");
    assert_eq!(guest.position, [0.5, 100.0, 0.5], "the player stays");
    answer
}

/// The world spawn's 5x5 chunks lit in the Overworld, which is put back in place.
fn light_the_world_spawn(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    game.enter_level(Dim::Overworld).unwrap();
    light_spawn_chunks(game);
}

/// A charged anchor in the Nether gives the spot on its top, facing it (straight down gives a
/// yaw of -90, `RespawnPosAngle.calculateLookAtYaw`'s `atan2(0, 0) - 90`), in the Nether, and
/// keeps its charge: `findRespawnAndUseSpawnBlock(.., false)` uses none of it.
#[test]
#[ignore = "daily"]
fn a_charged_anchor_gives_its_position_and_keeps_the_charge() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let mut resources = None;
        let (_scratch, mut game, _rx) = server_with_a_nether_anchor(&mut resources);
        game.enter_level(Dim::Nether).unwrap();
        light_spawn_chunks(&mut game);
        let charged = charge_anchor(&mut game);
        light_the_world_spawn(&mut game);
        let target = respawn_target(&mut game).unwrap();
        assert_eq!(target.level, game.level(Dim::Nether.id()).unwrap());
        assert_eq!(target.position, [10.5, 101.0, 10.5]);
        assert_eq!((target.yaw, target.pitch), (-90.0, 0.0));
        assert!(!target.missing_respawn_block);
        game.enter_level(Dim::Nether).unwrap();
        assert_eq!(
            game.world.block(ANCHOR[0], ANCHOR[1], ANCHOR[2]),
            charged,
            "the anchor keeps its charge"
        );
    });
}

/// A point in the Overworld whose bed is gone gives the world spawn's column, with
/// `missing_respawn_block` (`TeleportTransition.missingRespawnBlock`) and the world spawn's
/// angle.
#[test]
#[ignore = "daily"]
fn a_broken_bed_gives_the_world_spawn_with_missing_respawn_block() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let mut resources = None;
        let (_scratch, mut game, _rx) = server_with_a_nether_anchor(&mut resources);
        game.level.world_spawn_angle = [45.0, 10.0];
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.respawn_elsewhere = Some((Dim::Overworld.id().into(), [5, 100, 5], 0.0, 0.0, false));
        });
        light_the_world_spawn(&mut game);
        let target = respawn_target(&mut game).unwrap();
        assert_eq!(target.level, game.level(Dim::Overworld.id()).unwrap());
        assert_eq!([target.position[0], target.position[2]], [0.5, 0.5]);
        assert_eq!((target.yaw, target.pitch), (45.0, 10.0));
        assert!(target.missing_respawn_block);
    });
}

/// No respawn point gives the world spawn's column (`TeleportTransition.createDefault`),
/// without `missing_respawn_block`; while the spawn search waits for a chunk, the answer is
/// `Loading`.
#[test]
#[ignore = "daily"]
fn no_respawn_point_gives_the_world_spawn() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let mut resources = None;
        let (_scratch, mut game, _rx) = server_with_a_nether_anchor(&mut resources);
        game.level.world_spawn_angle = [45.0, 10.0];
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.respawn_elsewhere = None);
        assert_eq!(
            respawn_target(&mut game),
            Err(RespawnTargetUnavailable::Loading),
            "the world spawn's chunks are not loaded yet"
        );
        light_the_world_spawn(&mut game);
        let target = respawn_target(&mut game).unwrap();
        assert_eq!(target.level, game.level(Dim::Overworld.id()).unwrap());
        assert_eq!([target.position[0], target.position[2]], [0.5, 0.5]);
        assert_eq!((target.yaw, target.pitch), (45.0, 10.0));
        assert!(!target.missing_respawn_block);
    });
}

/// While a player's state is swapped in (a lifecycle event lending guest 2's body), a point in a
/// level not in place is refused with `InPlayerHook`: putting that level in place would move the
/// swapped-in player's level under it.
#[test]
#[ignore = "daily"]
fn a_point_in_another_level_is_refused_while_a_player_is_swapped_in() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let mut resources = None;
        let (_scratch, mut game, _rx) = server_with_a_nether_anchor(&mut resources);
        let answer = game
            .with_cloning_player(2, CloneScope::LendBody, |game| respawn_target(game))
            .unwrap();
        assert_eq!(answer, Err(RespawnTargetUnavailable::InPlayerHook));
    });
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
