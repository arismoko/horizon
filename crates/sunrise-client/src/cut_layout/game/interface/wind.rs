#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_sim::projectiles::Hit;
use sunrise_sim::projectiles::Projectile;
use sunrise_sim::projectiles::ProjectileKind;
#[cfg(test)]
#[path = "tests/wind_charged_tests.rs"]
pub(crate) mod wind_charged_tests;

#[cfg(test)]
#[path = "tests/wind_dragon_tests.rs"]
pub(crate) mod wind_dragon_tests;

#[cfg(test)]
pub(crate) mod tests {

    use super::*;
    use sunrise_client_lib::join_seq::Joined;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;

    #[test]
    #[ignore = "daily"]
    fn a_wind_push_waits_for_fresh_tnt_even_after_a_later_orb_ticked() {
        use std::sync::Arc;
        use sunrise_core::position::ChunkPos;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::join_seq::JoinSeq;

        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let mut chunk = Chunk::empty(0, 0);
            chunk.set(4, 70, 8, stone.0);
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            game.server.tickets.0.update(None);

            game.server.ordered.announced = JoinSeq::upcoming();
            assert!(
                game.run_command(
                    "summon tnt 8.25 70.5 8.5 {fuse:80,Motion:[0d,0d,0d],NoGravity:1b}"
                )
            );
            assert!(game.run_command("summon experience_orb 12.5 70.5 12.5 {Value:1}"));
            let at = Vec3::new(5.0, 70.5, 8.5);
            game.add_projectile(Projectile::new(ProjectileKind::BreezeWindCharge, at, None));
            let charge = game.creatures.projectiles[0].clone();
            game.begin_ordered_ticks();
            game.run_ordered_ticks(charge.join_seq(), false).unwrap();
            assert_eq!(game.server.ordered.done, Some(game.orbs[0].join_seq()));
            assert_eq!(game.orbs[0].age, 1);
            assert_eq!(game.tnts[0].fuse, 80);

            game.wind_charge_hit(
                &charge,
                Hit::Block {
                    pos: [4, 70, 8],
                    at,
                    face: [1, 0, 0],
                },
            )
            .unwrap();
            game.run_ordered_ticks(JoinSeq::MAX, true).unwrap();
            assert_eq!(game.tnts[0].fuse, 79);
            assert_eq!(game.tnts[0].position.x, 8.25);
            game.apply_late_wind_pushes();
            // Three blocks from the centre, unobstructed: (1 - 3 / 6) due east.
            assert_eq!(game.tnts[0].velocity.x, 0.5);

            game.server.ordered.announced = JoinSeq::upcoming();
            game.begin_ordered_ticks();
            game.run_ordered_ticks(charge.join_seq(), false).unwrap();
            assert_eq!(game.tnts[0].fuse, 78);
            assert_eq!(game.tnts[0].position.x, 8.75);
        });
    }

    /// Real player and breeze wind charges post `explosion_detonate` before
    /// knockback (even when `mob_griefing` disables breeze block triggers),
    /// applying edited block and entity lists to both phases.
    #[test]
    #[ignore = "daily"]
    fn a_wind_charge_posts_detonate_and_applies_edited_blocks_and_entities() {
        use mod_api::{
            EntityRef, ExplosionDetonateEvent, ExplosionKnockbackEvent, Hooks, ServerHost,
        };
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        };
        use sunrise_core::position::ChunkPos;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::mobs::Mob;

        const FIRST: u32 = 900;
        const SPARED: u32 = 901;
        const SECOND: u32 = 902;
        const NEAR_LEVER: [i32; 3] = [5, 70, 8];
        const FAR_LEVER: [i32; 3] = [12, 70, 8];

        #[derive(Clone, Default)]
        struct EditWind {
            detonates: Arc<AtomicUsize>,
            incoming_entities: Arc<Mutex<Vec<Vec<u32>>>>,
            knockback_order: Arc<Mutex<Vec<u32>>>,
            knockback_saw_edited_blocks: Arc<Mutex<Vec<bool>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for EditWind {
            fn explosion_detonate(
                &self,
                _: &mut dyn ServerHost,
                event: &mut ExplosionDetonateEvent<'_>,
            ) {
                self.detonates.fetch_add(1, Ordering::Relaxed);
                assert!(event.blocks.contains(&NEAR_LEVER), "{:?}", event.blocks);
                assert!(!event.blocks.contains(&FAR_LEVER), "{:?}", event.blocks);
                event.blocks.retain(|p| *p != NEAR_LEVER);
                event.blocks.push(FAR_LEVER);

                let ids: Vec<u32> = event.entities.iter().map(|e| e.id()).collect();
                self.incoming_entities.lock().unwrap().push(ids);
                let epoch = event.entities[0].epoch();
                event.entities.clear();
                event.entities.push(EntityRef::new(epoch, SECOND));
                event.entities.push(EntityRef::new(epoch, FIRST));
                event.entities.push(EntityRef::new(epoch, 424242));
            }

            fn explosion_knockback(
                &self,
                _: &mut dyn ServerHost,
                event: &mut ExplosionKnockbackEvent<'_>,
            ) {
                if let Some(id) = event.entity {
                    self.knockback_order.lock().unwrap().push(id);
                    self.knockback_saw_edited_blocks.lock().unwrap().push(
                        !event.blocks.contains(&NEAR_LEVER) && event.blocks.contains(&FAR_LEVER),
                    );
                }
            }
        }

        let probe = EditWind::default();
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("edit_wind", Box::new(probe.clone()))].into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.build_registries();
                game.set_allow_commands(true);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(0, 0))
                    .unwrap();
                assert!(game.run_command("setblock 5 69 8 minecraft:stone"));
                assert!(game.run_command("setblock 12 69 8 minecraft:stone"));
                assert!(
                    game.run_command("setblock 5 70 8 minecraft:lever[face=floor,powered=false]")
                );
                assert!(
                    game.run_command("setblock 12 70 8 minecraft:lever[face=floor,powered=false]")
                );

                let zombie = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                for (id, pos) in [
                    (FIRST, Vec3::new(6.0, 70.0, 8.5)),
                    (SPARED, Vec3::new(5.25, 70.0, 9.25)),
                    (SECOND, Vec3::new(4.5, 70.0, 8.5)),
                ] {
                    game.mobs.push(Mob::new(id, zombie.clone(), pos, 0.0));
                }

                let powered = |game: &sunrise_gameplay::game::ServerGame<'_>, pos: [i32; 3]| {
                    let state = game.world.block(pos[0], pos[1], pos[2]);
                    game.pack
                        .assets
                        .registry
                        .state(state)
                        .and_then(|s| s.properties.get("powered").cloned())
                };
                let vel_of = |game: &sunrise_gameplay::game::ServerGame<'_>, id: u32| {
                    game.mobs.iter().find(|m| m.id == id).unwrap().velocity
                };

                let at = Vec3::new(5.0, 70.5, 8.5);
                game.add_projectile(Projectile::new(ProjectileKind::WindCharge, at, None));
                let player_charge = game.creatures.projectiles[0].clone();
                game.wind_charge_hit(
                    &player_charge,
                    Hit::Block {
                        pos: [4, 70, 8],
                        at,
                        face: [1, 0, 0],
                    },
                )
                .unwrap();

                assert_eq!(probe.detonates.load(Ordering::Relaxed), 1);
                assert!(
                    !probe.incoming_entities.lock().unwrap()[0].contains(&player_charge.id),
                    "direct wind charge source is excluded from Detonate entities"
                );
                assert_eq!(
                    powered(game, NEAR_LEVER).as_deref(),
                    Some("false"),
                    "spared lever stays unpowered"
                );
                assert_eq!(
                    powered(game, FAR_LEVER).as_deref(),
                    Some("true"),
                    "listener-added lever is triggered"
                );
                assert_eq!(
                    vel_of(game, SPARED),
                    Vec3::ZERO,
                    "spared entity is not pushed"
                );
                assert_ne!(vel_of(game, SECOND), Vec3::ZERO);
                assert_ne!(vel_of(game, FIRST), Vec3::ZERO);
                assert_eq!(*probe.knockback_order.lock().unwrap(), [SECOND, FIRST]);
                assert_eq!(
                    *probe.knockback_saw_edited_blocks.lock().unwrap(),
                    [true, true]
                );

                // Breeze wind charge with mob_griefing disabled still posts Detonate before
                // knockback and honours edited entity lists while skipping block triggers.
                assert!(game.ui.rules.set(
                    "mob_griefing",
                    sunrise_game_data::gamerules::Value::Bool(false),
                ));
                for m in &mut game.mobs {
                    m.velocity = Vec3::ZERO;
                }
                probe.knockback_order.lock().unwrap().clear();
                probe.knockback_saw_edited_blocks.lock().unwrap().clear();
                game.add_projectile(Projectile::new(ProjectileKind::BreezeWindCharge, at, None));
                let breeze_charge = game.creatures.projectiles.last().unwrap().clone();
                game.wind_charge_hit(
                    &breeze_charge,
                    Hit::Block {
                        pos: [4, 70, 8],
                        at,
                        face: [1, 0, 0],
                    },
                )
                .unwrap();

                assert_eq!(probe.detonates.load(Ordering::Relaxed), 2);
                assert_eq!(
                    powered(game, NEAR_LEVER).as_deref(),
                    Some("false"),
                    "breeze charge skips block triggers when mob_griefing is false"
                );
                assert_eq!(
                    powered(game, FAR_LEVER).as_deref(),
                    Some("true"),
                    "far lever unchanged when mob_griefing is false"
                );
                assert_eq!(vel_of(game, SPARED), Vec3::ZERO);
                assert_ne!(vel_of(game, SECOND), Vec3::ZERO);
                assert_ne!(vel_of(game, FIRST), Vec3::ZERO);
                assert_eq!(*probe.knockback_order.lock().unwrap(), [SECOND, FIRST]);
                assert_eq!(
                    *probe.knockback_saw_edited_blocks.lock().unwrap(),
                    [true, true]
                );
            });
        });
    }
}
