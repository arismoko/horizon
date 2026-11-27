#![cfg(test)]
use std::sync::Arc;
use std::time::Instant;
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use sunrise_client::chunk::Chunk;
    use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::world_biomes;
    use sunrise_sim::save::{LevelShape, SaveDir};

    #[test]
    #[ignore = "daily"]
    fn eager_ticks_do_not_retry_a_refused_snapshot_without_an_edit() {
        let dir = tempfile::tempdir().unwrap();
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_game(|game| {
                game.server.world_root = dir.path().to_path_buf();
                game.server.save.0 = Arc::new(SaveDir::new(
                    game.server.world_root.clone(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));
                let key = (2, 0);
                let mut chunk = Chunk::with_layout(key.0, key.1, game.world.layout());
                chunk.lit = true;
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let gold = registry.default_state("minecraft:gold_block").unwrap();
                chunk.set(1, 64, 1, stone.0);
                game.server
                    .save
                    .0
                    .save(&[Arc::new(chunk.clone())], registry, world_biomes!(game))
                    .unwrap();
                chunk.set(1, 64, 1, gold.0);
                let invalid = (0..=DISK_WRITE_NESTING_GUARD)
                    .fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
                Arc::make_mut(chunk.saved.get_or_insert_with(Default::default))
                    .root
                    .insert("test:invalid".into(), invalid);
                game.world.chunks.insert(key, Arc::new(chunk));
                game.world.modified.insert(key);

                let now = Instant::now();
                game.save_chunks_eagerly_at(now);
                game.server.save.0.flush().unwrap();
                assert_eq!(game.server.save.0.record_save_failures(), 1);

                // Advance beyond the ordinary cooldown and exercise all later save paths.
                game.save_chunks_eagerly_at(now + Duration::from_secs(11));
                game.save_game_with(false).unwrap();
                game.save_game().unwrap();
                assert_eq!(game.server.save.0.record_save_failures(), 1);
                assert_eq!(
                    game.server
                        .save
                        .0
                        .load(key.0, key.1, registry, world_biomes!(game))
                        .unwrap()
                        .block(1, 64, 1),
                    stone
                );

                let chunk = Arc::make_mut(game.world.chunks.get_mut(&key).unwrap());
                Arc::make_mut(chunk.saved.as_mut().unwrap())
                    .root
                    .remove(&"test:invalid".into());
                game.world.modified.insert(key);
                game.save_chunks_eagerly_at(now + Duration::from_secs(22));
                game.server.save.0.flush().unwrap();
                assert_eq!(
                    game.server
                        .save
                        .0
                        .load(key.0, key.1, registry, world_biomes!(game))
                        .unwrap()
                        .block(1, 64, 1),
                    gold
                );
            });
        });
    }
}
