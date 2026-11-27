//! A projectile's block effects through its real tick (`Projectile.tick`:
//! move, `applyEffectsFromBlocks`, `baseTick`, then its hit).
use super::*;
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
use std::sync::Arc;
use sunrise_level::chunk::Chunk;

const CASE: &str = "SUNRISE_PROJECTILE_BLOCK_EFFECTS_CASE";

/// One projectile, still, inside `block` at the origin, ticked once; the
/// sounds it made.
fn tick_inside(game: &mut ServerGame<'_>, block: &str, mut shot: Projectile) -> Vec<String> {
    game.options.dedicated = Some(Vec::new());
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let state = game.pack.assets.registry.default_state(block).unwrap();
    game.world.set(0, 64, 0, state).unwrap();
    shot.id = 71;
    shot.velocity = Vec3::ZERO;
    game.creatures.projectiles.push(shot);
    game.start_audio_capture();
    game.tick_projectiles().unwrap();
    let sounds = std::mem::take(&mut game.server.pending_sounds);
    game.stop_audio_capture();
    sounds.into_iter().map(|sound| sound.event).collect()
}

fn at() -> Vec3 {
    Vec3::new(0.5, 64.5, 0.5)
}

#[test]
#[ignore = "daily"]
fn fire_lava_and_water_reach_a_projectile_in_its_tick() {
    if run_fresh_cases(
        "game::server_side::creatures_server::projectile_block_effects_tests::fire_lava_and_water_reach_a_projectile_in_its_tick",
        CASE,
        &["fire", "lava", "water", "lava_impact_batch"],
    ) {
        return;
    }
    if std::env::var(CASE).as_deref() == Ok("lava_impact_batch") {
        use mod_api::{
            EntityTickPostEvent, Flow, Hooks, HostBlockStates, HostSide, ProjectileImpactEvent,
            ServerHost,
        };
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        struct Probe {
            impacts: Arc<AtomicUsize>,
            snowball_id: u32,
            post_saw_fire: Arc<AtomicBool>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Probe {
            fn projectile_impact(
                &self,
                host: &mut dyn ServerHost,
                event: &mut ProjectileImpactEvent<'_>,
            ) -> Flow {
                self.impacts.fetch_add(1, Ordering::SeqCst);
                if let Some(level) = host.level(event.dimension)
                    && let Some(key) = mod_api::__private::block_key("minecraft:gold_block")
                    && let Ok(gold) = mod_api::BlockState::of(&host.block_states(), key, &[])
                {
                    let _ =
                        level.set_block(host, [4, 64, 0], gold, mod_api::BlockUpdateFlags::NONE);
                }
                Flow::Continue
            }
            fn entity_tick_post(&self, host: HostSide<'_>, event: &mut EntityTickPostEvent<'_>) {
                if let HostSide::Server(host) = host
                    && event.entity == self.snowball_id
                    && let Some(entity) = host.entity(self.snowball_id)
                    && entity.fire_ticks(host).is_some_and(|ticks| ticks > 0)
                {
                    self.post_saw_fire.store(true, Ordering::SeqCst);
                }
            }
        }

        let impacts = Arc::new(AtomicUsize::new(0));
        let post_saw_fire = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "projectile-block-effects-probe",
            Box::new(Probe {
                impacts: impacts.clone(),
                snowball_id: 72,
                post_saw_fire: post_saw_fire.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let lava = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:lava")
                    .unwrap();
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let fire = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:fire")
                    .unwrap();
                // Shulker bullet moves from lava at (0, 64, 0) into stone at (1, 64, 0).
                game.world.set(0, 64, 0, lava).unwrap();
                game.world.set(1, 64, 0, stone).unwrap();
                // Snowball sits in fire at (0, 64, 3).
                game.world.set(0, 64, 3, fire).unwrap();

                let mut bullet = Projectile::new(
                    ProjectileKind::ShulkerBullet {
                        target: None,
                        dir: None,
                        steps: 0,
                        delta: [0.0; 3],
                        seed: 0,
                    },
                    Vec3::new(0.5, 64.5, 0.5),
                    None,
                );
                bullet.id = 71;
                bullet.velocity = Vec3::new(0.8, 0.0, 0.0);
                game.creatures.projectiles.push(bullet);

                let mut snowball =
                    Projectile::new(ProjectileKind::Snowball, Vec3::new(0.5, 64.5, 3.5), None);
                snowball.id = 72;
                snowball.velocity = Vec3::ZERO;
                game.creatures.projectiles.push(snowball);

                game.tick_projectiles().unwrap();

                assert_eq!(
                    impacts.load(Ordering::SeqCst),
                    0,
                    "lava-destroyed shulker bullet must not dispatch projectile_impact"
                );
                assert_eq!(
                    game.pack
                        .assets
                        .registry
                        .state(game.world.block(4, 64, 0))
                        .unwrap()
                        .name
                        .as_str(),
                    "minecraft:air",
                    "impact listener writes must not commit for a lava-destroyed bullet"
                );
                assert!(
                    game.creatures.projectiles.iter().all(|p| p.id != 71),
                    "shulker bullet must be destroyed"
                );
                assert!(
                    post_saw_fire.load(Ordering::SeqCst),
                    "entity_tick_post must observe post-sweep fire state"
                );
            });
        });
        return;
    }
    with_game(|game| match std::env::var(CASE).as_deref() {
        // FireBlock.entityInside lights it for 160 ticks (`igniteForTicks`
        // on a count of 0), and the base tick after the walk takes one.
        Ok("fire") => {
            let shot = Projectile::new(ProjectileKind::Snowball, at(), None);
            tick_inside(game, "minecraft:fire", shot);
            assert_eq!(game.creatures.projectiles[0].fire_ticks, 159);
        }
        // Entity.lavaHurt on a shulker bullet: its hurtServer destroys it,
        // with its hurt sound, and the burn plays as hurtServer said true.
        Ok("lava") => {
            let shot = Projectile::new(
                ProjectileKind::ShulkerBullet {
                    target: None,
                    dir: None,
                    steps: 0,
                    delta: [0.0; 3],
                    seed: 0,
                },
                at(),
                None,
            );
            let sounds = tick_inside(game, "minecraft:lava", shot);
            assert!(game.creatures.projectiles.is_empty(), "the bullet is gone");
            assert!(
                sounds
                    .iter()
                    .any(|s| s == "minecraft:entity.shulker_bullet.hurt")
            );
            assert!(sounds.iter().any(|s| s == "minecraft:entity.generic.burn"));
        }
        // Water puts a burning projectile out, and the move that did plays
        // the extinguish hiss.
        Ok("water") => {
            let mut shot = Projectile::new(ProjectileKind::Snowball, at(), None);
            shot.fire_ticks = 100;
            let sounds = tick_inside(game, "minecraft:water", shot);
            assert_eq!(game.creatures.projectiles[0].fire_ticks, 0);
            assert!(
                sounds
                    .iter()
                    .any(|s| s == "minecraft:entity.generic.extinguish_fire")
            );
        }
        case => panic!("unknown case {case:?}"),
    });
}
