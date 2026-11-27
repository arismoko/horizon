#[cfg(test)]
pub(in crate::game) mod tests {
    use crate::game::native_entity_refs::tests::with_game;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::native_entity_refs_common::{
        NativeEntityRefsCommonGameExt1 as _, NativeEntityRefsCommonGameExt6 as _,
    };
    use sunrise_sim::mobs::{self, Mob};

    /// `ZombifiedPiglin.setTarget`, `customServerAiStep`, and `alertOthers`:
    /// a host write is consumed by real mob ticks, audible to the player and
    /// propagated to another piglin. Seed selection fixes the first sound at
    /// one tick; advancing the alert timer avoids waiting four seconds.
    #[test]
    #[ignore = "daily"]
    fn piglin_target_write_drives_anger_sound_and_alert() {
        use sunrise_core::random::{LegacyRandom, RandomSource};
        with_game(|game| {
            for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                let mut chunk = sunrise_level::chunk::Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
            }
            for (id, kind, x) in [
                (12, "minecraft:zombified_piglin", 0.0),
                (13, "minecraft:zombified_piglin", 2.0),
                (14, "minecraft:iron_golem", 8.0),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                let mut mob = Mob::new(id, kind, Vec3::new(x, 100.0, 0.0), 0.0);
                mobs::ai::register(&mut mob);
                game.mobs.push(mob);
            }
            let seed = (0..10_000)
                .find(|&seed| LegacyRandom::new(seed).next_int_bound(21) == Ok(1))
                .unwrap();
            game.mobs[0].random = LegacyRandom::new(seed);
            let actor = game.mob(game.entity(12).unwrap()).unwrap();
            let other = game.mob(game.entity(13).unwrap()).unwrap();
            let target = game.living(game.entity(14).unwrap()).unwrap();
            assert!(actor.set_target(game, Some(target)));
            assert_eq!(game.mobs[0].extra.counter, 1);
            game.mobs[0].extra.counter2 = 0;
            game.start_audio_capture();
            game.tick_mobs().unwrap();
            assert_eq!(other.target(game), Some(target));
            game.tick_mobs().unwrap();
            assert!(
                game.server
                    .pending_sounds
                    .iter()
                    .any(|sound| sound.event == "minecraft:entity.zombified_piglin.angry")
            );
        });
    }
}
