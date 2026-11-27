#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::mobs::Mob;
use sunrise_sim::mobs::Species;
#[cfg(test)]
pub(crate) mod tests {

    use super::*;
    use sunrise_level::sculk::Context;

    /// The wardens that hear an event are those in the list when it is
    /// posted, however the list changed since the last one.
    #[test]
    #[ignore = "daily"]
    fn wardens_hear_events_however_the_list_changed_between_them() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.sculk_prepare();
            let warden = game
                .res
                .mob_types()
                .get("minecraft:warden")
                .cloned()
                .unwrap();
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .cloned()
                .unwrap();
            let at = |x| sunrise_core::geometry::Vec3::new(x, 100.0, 0.0);
            let post = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                game.post_game_event("minecraft:explode", [0.5, 100.5, 0.5], Context::default());
                let later = game.world.game_time + 1;
                let heard = |m: &mut Mob| {
                    let warden = m.extra.warden.as_mut()?;
                    let heard = warden.vibration.selector.chosen_candidate(later);
                    warden.vibration = Default::default();
                    heard
                };
                let mut ids: Vec<u32> = game
                    .mobs
                    .iter_mut()
                    .filter_map(|m| heard(m).map(|_| m.id))
                    .collect();
                ids.sort_unstable();
                ids
            };
            assert_eq!(post(game), [0u32; 0], "nobody to hear it");
            assert_eq!(post(game), [0u32; 0]);

            game.mobs.push(Mob::new(1, zombie.clone(), at(1.0), 0.0));
            game.mobs.push(Mob::new(2, warden.clone(), at(2.0), 0.0));
            assert_eq!(post(game), [2], "one that joined since");
            assert_eq!(post(game), [2], "and again with nothing changed");

            game.mobs
                .insert(0, Mob::new(3, warden.clone(), at(3.0), 0.0));
            assert_eq!(post(game), [2, 3], "the slots moved");

            let level = game.mobs.lift();
            assert_eq!(post(game), [0u32; 0], "another level's list");
            game.mobs
                .restore(level, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
            assert_eq!(post(game), [2, 3], "this level's again");

            game.mobs.retain(|m| m.id != 3);
            assert_eq!(post(game), [2], "one that left");

            // A write that keeps every slot and makes the zombie's a listener.
            game.mobs[0] = Mob::new(1, warden, at(1.0), 0.0);
            assert_eq!(post(game), [1, 2], "one replaced in its slot");

            game.mobs[0].removed = true;
            game.mobs[0].health = 0.0;
            assert_eq!(post(game), [2], "the dead hear nothing");
        });
    }
}

#[cfg(test)]
pub(crate) mod command_damage_listener_tests {
    use super::*;
    use crate::game::{Settings, load_resources};
    use std::sync::Arc;
    use sunrise_core::position::ChunkPos;
    use sunrise_level::chunk::Chunk;
    use sunrise_level::sculk::Info;
    use {sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress};

    fn selected(
        game: &sunrise_gameplay::game::ServerGame<'_>,
        key: (i32, i32, i32),
    ) -> Option<(Info, i64)> {
        let Some(BlockEntity::Sculk(sensor)) = game.world.block_entities.get(&key) else {
            panic!("actual command-created sensor must be registered");
        };
        sensor.vibration.selector.current
    }

    #[test]
    #[ignore = "daily"]
    fn command_damage_posts_before_teleport_and_deduplicates_the_scan() {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources required"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        for x in 0..=2 {
            game.world
                .chunks
                .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(x, 0))
                .unwrap();
        }
        assert!(game.run_command("forceload add 0 0 47 15"));
        assert!(game.run_command("fill 0 199 0 47 199 15 stone"));
        assert_eq!(
            game.world.block(4, 199, 4),
            game.pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap()
        );
        let origin = (7, 200, 4);
        let destination = (39, 200, 4);
        for (index, accepted) in [true, false].into_iter().enumerate() {
            if index != 0 {
                assert!(game.run_command("setblock 7 200 4 air strict"));
                assert!(game.run_command("setblock 39 200 4 air strict"));
            }
            let uuid = format!("00000071-0000-0000-0000-{:012x}", index + 1);
            assert!(game.run_command(&format!(
                "summon pig 4.5 200 4.5 {{UUID:[I;113,0,0,{}],Health:10f,NoAI:1b,NoGravity:1b,Invulnerable:{}b,Motion:[0d,0d,0d]}}",
                index + 1, if accepted {0} else {1},
            )));
            let id = game.mobs.last().expect("actual summon must create pig").id;
            assert_eq!(game.uuid_for(id), [113, 0, 0, index as i32 + 1]);
            assert!(game.run_command("setblock 7 200 4 sculk_sensor strict"));
            assert!(game.run_command("setblock 39 200 4 sculk_sensor strict"));
            game.sculk_prepare();
            game.sculk_sync_index();
            // Establish the movement-emission snapshot before the teleport,
            // as the real server does on the mob's preceding tick.
            game.tick_mob_sculk_events();
            assert!(selected(&game, origin).is_none());
            assert!(selected(&game, destination).is_none());
            let before = game.world.game_time;
            let mob = game.mobs.iter().find(|m| m.id == id).unwrap();
            assert_eq!(mob.species(), Species::Pig);
            assert_eq!(mob.health, 10.0);
            assert!(mob.no_ai && mob.no_gravity);
            assert_eq!(mob.position, Vec3::new(4.5, 200.0, 4.5));
            assert_eq!(
                game.run_command(&format!("damage {uuid} 2 minecraft:cactus")),
                accepted
            );
            let mob = game.mobs.iter().find(|m| m.id == id).unwrap();
            assert_eq!(mob.health, if accepted { 8.0 } else { 10.0 });
            if accepted {
                let (event, tick) =
                    selected(&game, origin).expect("accepted command hit must already be heard");
                assert_eq!(event.event, "minecraft:entity_damage");
                assert_eq!(event.pos, [4.5, 200.0, 4.5]);
                assert_eq!(
                    event.source,
                    Some(sunrise_game_data::who::EventSource::Mob(id))
                );
                assert_eq!(tick, before);
                assert_eq!(
                    game.server.sculk.damaged.get(&id).copied(),
                    mob.last_damage.map(|(_, tick)| tick)
                );
            } else {
                assert!(selected(&game, origin).is_none());
                assert!(!game.server.sculk.damaged.contains_key(&id));
            }
            assert!(game.run_command(&format!("tp {uuid} 36.5 200 4.5")));
            assert_eq!(game.world.game_time, before);
            assert_eq!(
                game.mobs.iter().find(|m| m.id == id).unwrap().position,
                Vec3::new(36.5, 200.0, 4.5)
            );
            assert!(selected(&game, destination).is_none());
            if accepted {
                assert_eq!(selected(&game, origin).unwrap().0.pos, [4.5, 200.0, 4.5]);
            }
            game.world.game_time += 1;
            game.tick_mob_sculk_events();
            assert!(
                selected(&game, destination).is_none(),
                "end-of-tick scan must not post the old hit again at the moved victim"
            );
        }
    }
}
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
