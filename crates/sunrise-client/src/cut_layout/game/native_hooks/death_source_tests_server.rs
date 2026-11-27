use crate::game::native_hooks::death_source_tests::bow;
use crate::game::native_hooks::death_source_tests::finish_projectile_death;
use crate::game::native_hooks::death_source_tests::mob;
use crate::game::native_hooks::death_source_tests::observed;
use crate::game::native_hooks::death_source_tests::player_arrow;
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::native_entity_refs::tests::bind_local_player;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_items::items::ItemStack;
use sunrise_sim::{mobs::Who, motion::PlayerMotion};

#[test]
#[ignore = "daily"]
fn guest_player_bow_death_keeps_the_connected_shooter_without_a_local_player() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::native_hooks_server::death_source_tests::guest_player_bow_death_keeps_the_connected_shooter_without_a_local_player",
        || {
            observed(false, false, |game, seen| {
                game.player = None;
                sunrise_gameplay::game::native_player_death::tests::add_guest(game);
                game.server.next_entity = 100;
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.position = [4.5, 80.0, 4.5];
                        guest.loaded = true;
                        guest.load_timeout = 0;
                    });
                mob(game, 41, "minecraft:zombie", Vec3::new(6.0, 80.0, 4.5));
                game.mobs[0].health = 1.0;
                let weapon = bow();
                let expected = weapon.try_into_api().unwrap();
                let arrow = game
                    .as_guest(1, |game| player_arrow(game, &weapon))
                    .unwrap();
                assert!(
                    game.player.is_none(),
                    "the impact runs in the dedicated-server context"
                );
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.inventory[guest.selected] =
                            Some(ItemStack::new("minecraft:stick", 1));
                    });
                finish_projectile_death(game, 41);
                let seen = seen.lock().unwrap();
                assert_eq!(seen.deaths.len(), 1);
                assert_eq!(seen.incoming.len(), 1);
                assert_eq!(seen.deaths[0], seen.incoming[0]);
                let hit = &seen.deaths[0];
                assert_eq!((hit.causing, hit.direct), (20, arrow as i32));
                assert_eq!(hit.weapon.as_ref(), Some(&expected));
                assert!(hit.position.is_some());
            })
        },
    );
}

pub(crate) fn player(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 80.0, 4.5)).unwrap());
    bind_local_player(game, 11, 123);
    game.set_mode_id(0);
    game.server.next_entity = 100;
}

#[test]
#[ignore = "daily"]
fn zombie_ai_melee_armor_hurt_retains_the_real_source() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::native_hooks_server::death_source_tests::zombie_ai_melee_armor_hurt_retains_the_real_source",
        || {
            observed(false, false, |game, seen| {
                player(game);
                Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).lit = true;
                game.world.day_time = 18_000; // An open-air zombie must not burn during the melee scene.
                assert!(game.run_command("difficulty normal"));
                assert!(game.run_command("forceload add 0 0"));
                game.update_chunk_tickets();
                assert!(game.server.tickets.0.levels().entity_ticking_at(5, 4));
                assert!(game.player.as_ref().unwrap().client_loaded);
                game.armor[1] = Some(ItemStack::new("minecraft:iron_chestplate", 1));
                game.apply_equipment();
                game.refresh_armor_protection();
                mob(game, 45, "minecraft:zombie", Vec3::new(5.0, 80.0, 4.5));
                game.mobs[0].no_ai = false;
                game.mobs[0].ai.speed = 0.2;
                game.mobs[0].ai.target = Some(Who::Player);
                for _ in 0..40 {
                    // MeleeAttackGoal.canUse admits a new attempt once game time reaches 20.
                    game.world.game_time += 1;
                    game.tick_creature_entities().unwrap();
                    if !seen.lock().unwrap().armor.is_empty() {
                        break;
                    }
                }
                let seen = seen.lock().unwrap();
                assert!(
                    !seen.armor.is_empty(),
                    "real zombie AI must land its melee hit"
                );
                let hit = &seen.armor[0];
                assert_eq!((hit.entity, hit.kind.as_str()), (11, "minecraft:player"));
                assert_eq!(
                    (hit.damage_type.as_str(), hit.causing, hit.direct),
                    ("minecraft:mob_attack", 45, 45)
                );
                let incoming = seen.incoming.iter().find(|hit| hit.entity == 11).unwrap();
                assert_eq!(
                    (hit.causing, hit.direct, hit.position),
                    (incoming.causing, incoming.direct, incoming.position)
                );
                assert!(game.armor[1].as_ref().unwrap().damage > 0);
            })
        },
    );
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
