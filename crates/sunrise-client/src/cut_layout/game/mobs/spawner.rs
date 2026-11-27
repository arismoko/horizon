#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_level::containers::BlockEntity;
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sunrise_core::random::RandomSource;
    use sunrise_sim::mobs::spawner::Spawner;

    /// Keeps the spawner block at `self.0` going with no player near, and the one at `self.1`
    /// idle with one beside it; logs what it is asked.
    struct Rule(
        [i32; 3],
        [i32; 3],
        std::sync::Arc<std::sync::Mutex<Vec<(String, [i32; 3], bool)>>>,
    );
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Rule {
        fn spawner_active(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::SpawnerActiveEvent<'_>,
        ) {
            let mod_api::Spawner::Block { pos } = event.spawner else {
                return;
            };
            let pos = [pos.x, pos.y, pos.z];
            self.2
                .lock()
                .unwrap()
                .push((event.dimension.to_owned(), pos, event.active));
            if pos == self.0 {
                event.active = true;
            } else if pos == self.1 {
                event.active = false;
            }
        }
    }

    /// `spawner_active` (#6040): a mod keeps a spawner with no player in range ticking and one
    /// with a player beside it idle, each asked once a tick from vanilla's `isNearPlayer`
    /// answer, and the saved `RequiredPlayerRange` is left as it was. At delay 0 the forced-active
    /// far spawner runs its spawn (`BaseSpawner.serverTick` reads the answer the tick settled on,
    /// not `isNearPlayer` again) and the hook is not asked a second time.
    #[test]
    #[ignore = "daily"]
    fn a_mod_decides_whether_a_spawner_is_active() {
        let heard = std::sync::Arc::default();
        let (far, near) = ((8, 64, 120), (8, 64, 12));
        let mods = vec![mod_api::ModEntry::new(
            "rule",
            Box::new(Rule(
                [8, 64, 120],
                [8, 64, 12],
                std::sync::Arc::clone(&heard),
            )),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.player = Some(
                    sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5))
                        .unwrap(),
                );
                for key in [far, near] {
                    let spawner = Spawner {
                        spawn_delay: 3,
                        ..Spawner::default()
                    };
                    game.world
                        .block_entities
                        .insert(key, BlockEntity::Spawner(Box::new(spawner)));
                }
                let state = |game: &sunrise_gameplay::game::ServerGame<'_>, key| match game
                    .world
                    .block_entities
                    .get(&key)
                {
                    Some(BlockEntity::Spawner(s)) => (s.spawn_delay, s.required_player_range),
                    _ => panic!("the spawner is gone"),
                };
                game.tick_spawner(far).unwrap();
                game.tick_spawner(near).unwrap();
                assert_eq!(
                    state(game, far),
                    (2, 16),
                    "no player near, but the mod keeps it going"
                );
                assert_eq!(
                    state(game, near),
                    (3, 16),
                    "a player beside it, but the mod idles it"
                );
                let dimension = game.dim.id().to_owned();
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        (dimension.clone(), [8, 64, 120], false),
                        (dimension.clone(), [8, 64, 12], true)
                    ]
                );
                // At delay 0 the far spawner the mod forces active runs its spawn, which
                // with nothing to spawn draws its next delay: the spawn reads the tick's
                // answer, since asking the level again would find no player and stop it.
                game.world.chunks.insert(
                    (0, 7),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 7)),
                );
                heard.lock().unwrap().clear();
                game.world.block_entities.insert(
                    far,
                    BlockEntity::Spawner(Box::new(Spawner {
                        spawn_delay: 0,
                        ..Spawner::default()
                    })),
                );
                game.tick_spawner(far).unwrap();
                let (delay, range) = state(game, far);
                assert!((200..800).contains(&delay), "{delay}");
                assert_eq!(range, 16);
                assert_eq!(
                    *heard.lock().unwrap(),
                    [(dimension, [8, 64, 120], false)],
                    "asked once for the tick, not again by the spawn"
                );
            });
        });
    }

    /// `BaseSpawner.serverTick` with a player in range: a running delay only
    /// goes down by one a tick, drawing nothing from the level random and
    /// adding no mob. At 0 the spawn runs: with nothing to spawn it calls
    /// `delay`, which draws the next delay in [MinSpawnDelay, MaxSpawnDelay).
    #[test]
    #[ignore = "daily"]
    fn a_running_delay_counts_down_without_drawing() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5))
                    .unwrap(),
            );
            let key = (8, 64, 12);
            let spawner = Spawner {
                spawn_delay: 3,
                ..Spawner::default()
            };
            game.world
                .block_entities
                .insert(key, BlockEntity::Spawner(Box::new(spawner)));
            let delay = |game: &sunrise_gameplay::game::ServerGame<'_>| match game
                .world
                .block_entities
                .get(&key)
            {
                Some(BlockEntity::Spawner(s)) => s.spawn_delay,
                _ => panic!("the spawner is gone"),
            };
            for expected in [2, 1, 0] {
                let mut before = game.world.random.clone();
                game.tick_spawner(key).unwrap();
                assert_eq!(delay(game), expected);
                assert_eq!(
                    game.world.random.next_long(),
                    before.next_long(),
                    "no draw at delay {expected}"
                );
                assert!(game.mobs.is_empty());
            }
            let mut before = game.world.random.clone();
            game.tick_spawner(key).unwrap();
            assert!((200..800).contains(&delay(game)), "{}", delay(game));
            assert_ne!(game.world.random.next_long(), before.next_long());
        });
    }
}
