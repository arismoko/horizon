use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_command::args::GameMode;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::fireworks::Rocket;
use sunrise_gameplay_common::game::fireworks::hit_attempt_tests::Probe;
use sunrise_items::firework::FireworkExplosion;
use sunrise_sim::mobs::Who;
use sunrise_sim::{
    motion::PlayerMotion,
    projectiles::{Hit, Projectile, ProjectileKind, ThrownType},
};
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

pub(crate) fn fresh_player(game: &mut sunrise_gameplay::game::ServerGame<'_>, mode: GameMode) {
    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
    bind_local_player(game, 7, 7);
    game.server.lan_acting_name = Some("attempts".into());
    game.server.lan_acting_permission = Some(4);
    game.set_game_mode(GameMode::Survival);
    game.set_game_mode(mode);
    game.remove_player_effects(None);
}

#[test]
#[ignore = "daily"]
fn remaining_hit_callers_observe_refusals_and_zero_damage() {
    let count = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "remaining_attempts",
        Box::new(Probe(count.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let check =
                |expected, case| assert_eq!(count.swap(0, Ordering::Relaxed), expected, "{case}");
            for mode in [GameMode::Creative, GameMode::Survival] {
                fresh_player(game, mode);
                let skull = Projectile::new(
                    ProjectileKind::WitherSkull { dangerous: false },
                    Vec3::ZERO,
                    None,
                );
                game.projectile_hit(
                    &skull,
                    Hit::Entity {
                        who: Who::Player,
                        at: Vec3::ZERO,
                    },
                )
                .unwrap();
                check(2, "skull impact and its separate explosion each ask once");
                let health = game.player.as_ref().unwrap().health.health();
                if mode == GameMode::Creative {
                    assert_eq!(health, 20.0);
                } else {
                    assert!(health < 20.0);
                }
            }
            for (mode, attached, dead, loaded, expected) in [
                (GameMode::Creative, true, false, true, 1),
                (GameMode::Spectator, true, false, true, 1),
                (GameMode::Survival, true, true, true, 1),
                (GameMode::Creative, false, false, true, 1),
                (GameMode::Spectator, false, false, true, 0),
                (GameMode::Survival, false, true, true, 1),
                (GameMode::Survival, false, false, false, 1),
                (GameMode::Survival, false, false, true, 1),
            ] {
                fresh_player(game, mode);
                if dead {
                    game.player
                        .as_mut()
                        .unwrap()
                        .set_health(0.0, false)
                        .unwrap();
                }
                game.player.as_mut().unwrap().client_loaded = loaded;
                let mut rocket = Rocket::remote(900, Vec3::ZERO, Vec3::ZERO);
                rocket.attached = attached;
                std::sync::Arc::make_mut(&mut rocket.explosions).push(FireworkExplosion::default());
                game.explode_rocket(&rocket);
                check(
                    expected,
                    "rocket explosion: attached target versus spectator-filtered splash query",
                );
                let health = game.player.as_ref().unwrap().health.health();
                assert_eq!(
                    health,
                    if dead {
                        0.0
                    } else if mode == GameMode::Survival && loaded {
                        13.0
                    } else {
                        20.0
                    }
                );
            }
            fresh_player(game, GameMode::Spectator);
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut corpse = sunrise_sim::mobs::Mob::new(903, kind, Vec3::ZERO, 0.0);
            corpse.health = 0.0;
            game.mobs.push(corpse);
            let mut rocket = Rocket::remote(902, Vec3::ZERO, Vec3::ZERO);
            std::sync::Arc::make_mut(&mut rocket.explosions).push(FireworkExplosion::default());
            game.explode_rocket(&rocket);
            check(
                1,
                "resident dead mob is included in the rocket splash query",
            );
            assert_eq!(game.mobs.iter().find(|m| m.id == 903).unwrap().health, 0.0);
            game.mobs.clear();

            fresh_player(game, GameMode::Spectator);
            game.build_registries();
            game.set_allow_commands(true);
            game.server.chat_capture = Some(Vec::new());
            assert!(
                !game
                    .run_command("damage 00000000-0000-0000-0000-000000000007 4 minecraft:generic")
            );
            let feedback = game.server.chat_capture.take().unwrap();
            assert_eq!(
                count.swap(0, Ordering::Relaxed),
                1,
                "spectator damage command asks before its error: {feedback:?}"
            );
            assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);

            static ZERO: ThrownType = ThrownType {
                id: "remaining:zero",
                item: "minecraft:snowball",
                damage: 0.0,
                slot: 0,
            };
            for mode in [GameMode::Survival, GameMode::Creative] {
                for kind in [
                    ProjectileKind::Snowball,
                    ProjectileKind::Egg,
                    ProjectileKind::EnderPearl,
                    ProjectileKind::Thrown(&ZERO),
                ] {
                    fresh_player(game, mode);
                    let shot = Projectile::new(kind, Vec3::ZERO, None);
                    game.projectile_hit(
                        &shot,
                        Hit::Entity {
                            who: Who::Player,
                            at: Vec3::ZERO,
                        },
                    )
                    .unwrap();
                    check(
                        1,
                        "zero-damage projectile still enters the player predicate",
                    );
                    assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);
                }
            }
            fresh_player(game, GameMode::Survival);
            assert!(
                game.native_acting_player().is_some(),
                "fixture pearl owner has a live handle"
            );
            let mut pearl = Projectile::new(
                ProjectileKind::EnderPearl,
                Vec3::new(2.0, 1.0, 0.0),
                Some(Who::Player),
            );
            pearl.previous = Vec3::new(2.0, 1.0, 0.0);
            game.projectile_hit(
                &pearl,
                Hit::Entity {
                    who: Who::Player,
                    at: Vec3::ZERO,
                },
            )
            .unwrap();
            check(
                2,
                "zero collision and listener-zeroed teleport fall damage are separate attempts",
            );
            assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);
            assert_eq!(game.player.as_ref().unwrap().position, pearl.previous);

            // A blast's entity query includes resident dead and not-yet-loaded
            // network players, unlike the combat-target helper formerly used here.
            use sunrise_sim::lan::{NetConfig, hub::HubConfig};
            game.lan_host(
                sunrise_gameplay::game::lan::LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                NetConfig {
                    announce: false,
                    ..NetConfig::lan(0, "attempts".into(), 2)
                },
                HubConfig {
                    default_game_mode: 0,
                    guest_commands: false,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 10,
                    simulation_distance: 10,
                    dimension: "minecraft:overworld".into(),
                    allow_flight: false,
                    seed: 0,
                    max_players: 20,
                    integrated: false,
                    levels: Default::default(),
                    level_seeds: Default::default(),
                },
            )
            .unwrap();
            let (guest, _packets) =
                sunrise_gameplay_common::game::server_side::native_sound_refs::tests::guest(
                    88, 88, [0.0; 3],
                );
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
            // The egg hits above may hatch chicks (`ThrownEgg.onHit`, drawn from the level's
            // random); they would be splash targets too, so only the guest stays near the blast.
            game.mobs.clear();
            for dead in [true, false] {
                fresh_player(game, GameMode::Spectator); // excluded from splash
                let mut body = Box::new(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
                if dead {
                    body.set_health(0.0, false).unwrap();
                }
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update_connection(88, |guest| {
                        guest.motion = Some(body);
                        guest.dead = dead;
                        guest.loaded = dead;
                        guest.loading = !dead;
                    })
                    .unwrap();
                let mut rocket = Rocket::remote(901, Vec3::ZERO, Vec3::ZERO);
                std::sync::Arc::make_mut(&mut rocket.explosions).push(FireworkExplosion::default());
                game.explode_rocket(&rocket);
                check(
                    1,
                    "resident dead or unloaded guest is still a rocket damage target",
                );
            }
        })
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
