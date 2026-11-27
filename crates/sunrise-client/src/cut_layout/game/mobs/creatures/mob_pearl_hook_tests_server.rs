use crate::game::creatures::mob_pearl_hook_tests::PlayerPearlProbe;
use mod_api::ModEntry;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use sunrise_core::geometry::Vec3;
use sunrise_core::random::RandomSource;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_sim::mobs::Who;
use sunrise_sim::projectiles::Hit;
use sunrise_sim::projectiles::Projectile;
use sunrise_sim::projectiles::ProjectileKind;

pub(crate) fn run_player_pearl_keeps_acting_context_cancellation_and_mutable_damage() {
    use sunrise_sim::motion::PlayerMotion;
    let mode = Arc::new(AtomicU8::new(0));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "player-pearl-proof",
        Box::new(PlayerPearlProbe(Arc::clone(&mode))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let from = Vec3::new(0.0, 65.0, 0.0);
            game.player = Some(PlayerMotion::new(game.motion, from).unwrap());
            game.server.lan_acting = Some(12);
            game.server.lan_acting_uuid = Some(7);
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 2, 7);
            game.server.lan_acting_name = Some("PearlTest".into());
            game.server.lan_acting_permission = Some(4);
            game.server.lan_acting_entity = Some(2);
            sunrise_gameplay::game::native_entity_refs::tests::bind_acting_guest(game);
            game.ui.difficulty = 0;
            let pearl = Projectile::new(
                ProjectileKind::EnderPearl,
                Vec3::new(6.0, 75.0, 6.0),
                Some(Who::Player),
            );
            let hit = || Hit::Block {
                pos: [6, 75, 6],
                at: pearl.position,
                face: [0, 1, 0],
            };
            let mut expected_random = game.world.random.clone();
            let health = game.player.as_ref().unwrap().health.health();
            game.projectile_hit(&pearl, hit()).unwrap();
            assert_eq!(game.player.as_ref().unwrap().position, from);
            assert_eq!(game.player.as_ref().unwrap().health.health(), health);
            assert_eq!(game.world.random.next_long(), expected_random.next_long());
            mode.store(1, Ordering::SeqCst);
            expected_random = game.world.random.clone();
            expected_random.next_float();
            game.projectile_hit(&pearl, hit()).unwrap();
            let player = game.player.as_ref().unwrap();
            assert_eq!(player.position, Vec3::new(12.0, 80.0, -4.0));
            assert_eq!(
                player.health.health(),
                health,
                "listener suppressed player fall damage"
            );
            assert_eq!(player.fall_distance, 0.0);
            assert_eq!(game.world.random.next_long(), expected_random.next_long());
            mode.store(2, Ordering::SeqCst);
            expected_random = game.world.random.clone();
            game.projectile_hit(&pearl, hit()).unwrap();
            let player = game.player.as_ref().unwrap();
            assert_eq!(
                player.position,
                Vec3::new(3.0, 70.0, 3.0),
                "a listener's direct move is not overwritten by the pearl target"
            );
            assert_eq!(player.health.health(), health);
            assert_eq!(
                game.world.random.next_long(),
                expected_random.next_long(),
                "a stale player move makes no endermite draw"
            );
        })
    });
}
