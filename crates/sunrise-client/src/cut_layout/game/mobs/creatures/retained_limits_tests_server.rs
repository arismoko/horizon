use crate::game::creatures::retained_limits_tests::OTHER_GUEST;
use crate::game::creatures::retained_limits_tests::ObserveMovement;
use crate::game::creatures::retained_limits_tests::PanicInMovement;
use crate::game::creatures::retained_limits_tests::TARGET_GUEST;
use crate::game::creatures::retained_limits_tests::ZOMBIE;
use mod_api::__private::{HookPanicReport, take_hook_panics};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use sunrise_core::geometry::Vec3;
use sunrise_level::chunk::Chunk;
use sunrise_sim::{
    mobs::{Mob, Who},
    motion::PlayerMotion,
};
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_guest_turn_tests::with_guests,
};

#[test]
#[ignore = "daily"]
fn same_mob_guest_attack_survives_movement_panic() {
    if run_fresh_cases(
        "cut_layout::game::server_side::creatures_server::retained_limits_tests::same_mob_guest_attack_survives_movement_panic",
        "SUNRISE_SAME_MOB_GUEST_ATTACK_CASE",
        &["movement-panic"],
    ) {
        return;
    }

    crate::entry::install_client_hooks();
    let faulty_calls = Arc::new(AtomicUsize::new(0));
    let healthy_calls = Arc::new(AtomicUsize::new(0));
    mod_platform::testing::with_mods(
        vec![
            mod_api::ModEntry::new("same-mob", Box::new(PanicInMovement(faulty_calls.clone()))),
            mod_api::ModEntry::new(
                "same-mob-observer",
                Box::new(ObserveMovement(healthy_calls.clone())),
            ),
        ]
        .into(),
        || {
            with_guests(|game| {
                game.server.world_root = PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                    .join("same-mob-guest-attack");
                game.options.dedicated = Some(Vec::new());
                game.load_server_loot();
                game.world.health_rules.difficulty = sunrise_world::food::Difficulty::Normal;
                game.world.game_time = 1000;
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                for x in 0..16 {
                    for z in 0..16 {
                        game.world.set(x, 79, z, stone).unwrap();
                    }
                }

                let far = Vec3::new(12.0, 80.0, 12.0);
                game.player = Some(PlayerMotion::new(game.motion, far).unwrap());
                let host = game.server.lan.as_mut().unwrap();
                for (conn, position) in [
                    (TARGET_GUEST, [1.5, 80.0, 0.5]),
                    (OTHER_GUEST, far.components()),
                ] {
                    host.hub
                        .update_guest(conn, |guest| {
                            guest.loaded = true;
                            guest.load_timeout = 0;
                            guest.position = position;
                        })
                        .unwrap();
                }
                let kind = game
                    .common
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mut mob = Mob::new(ZOMBIE, kind, Vec3::new(1.0, 80.0, 0.5), 0.0);
                mob.ground.on_ground = true;
                mob.ai.speed = 0.2;
                mob.ai.target = Some(Who::Player);
                game.common.mobs.push(mob);
                // Who::Player names the selected player, so bind it to the nearby guest.
                host.mob_players.insert(ZOMBIE, TARGET_GUEST);

                let health = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                    let hub = &game.server.lan.as_ref().unwrap().hub;
                    (
                        game.player.as_ref().unwrap().health.health(),
                        hub.guest(TARGET_GUEST).unwrap().health,
                        hub.guest(OTHER_GUEST).unwrap().health,
                    )
                };
                let (host_before, target_before, other_before) = health(game);
                game.tick_creature_entities().unwrap();
                let after = health(game);
                assert!(
                    after.1 < target_before,
                    "the zombie must damage its guest in the same tick"
                );
                assert_eq!(after.0, host_before, "the host was not targeted");
                assert_eq!(after.2, other_before, "the other guest was not targeted");
                assert_eq!(faulty_calls.load(Ordering::SeqCst), 1);
                assert_eq!(
                    take_hook_panics(),
                    [HookPanicReport {
                        mod_id: "same-mob",
                        hook: "living_speed"
                    }],
                );
                let healthy_after_first = healthy_calls.load(Ordering::SeqCst);
                assert!(healthy_after_first > 0, "the other listener still runs");

                // Keep the zombie's grounded speed path active. Move the guest beyond
                // melee range without unloading the chunk and clear the hurt cooldown,
                // so unchanged health cannot be explained by invulnerability.
                game.mobs[0].ground.on_ground = true;
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(TARGET_GUEST, |guest| {
                        guest.position = far.components();
                        guest.motion.as_mut().unwrap().health.invulnerable_time = 0;
                    })
                    .unwrap();
                game.tick_creature_entities().unwrap();
                assert!(healthy_calls.load(Ordering::SeqCst) > healthy_after_first);
                assert_eq!(faulty_calls.load(Ordering::SeqCst), 1);
                assert!(take_hook_panics().is_empty());
                assert_eq!(health(game), after, "the guest attack must not replay");
            });
        },
    );
}
