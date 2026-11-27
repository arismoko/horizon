#![cfg(test)]

#[cfg(test)]
mod tests {
    use mod_api::ServerHost;
    use sunrise_gameplay::game::server_side::native_level_items::spill;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    #[test]
    #[ignore = "daily"]
    fn a_level_pops_spawns_and_awards_as_vanilla() {
        use mod_api::{LevelRef, WorldEpoch};
        use sunrise_core::random::RandomSource;
        use sunrise_game_data::gamerules::Value as RuleValue;
        use sunrise_items::items::ItemStack;

        super::super::native_entity_refs::tests::with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let stone = || ItemStack::new("minecraft:stone", 3);

            // Block.popResource: three level-random offsets, then the entity.
            let mut random = game.world.random.clone();
            let at = spill::pop_position(&mut random, [4, 64, -2]);
            let popped = game
                .level_pop_resource(level, [4, 64, -2], stone())
                .unwrap();
            assert_eq!(
                game.entity_kind(popped)
                    .and_then(|kind| kind.id(game))
                    .as_deref(),
                Some("minecraft:item")
            );
            assert_eq!(game.entity_position(popped), Some(at));
            let item = game.dropped.last().unwrap();
            assert_eq!((item.pickup_delay, item.stack.count), (10, 3));
            assert_eq!(item.velocity.y, 0.2);
            // The offsets are drawn even when nothing drops.
            assert!(
                game.level_pop_resource(level, [0; 3], ItemStack::new("minecraft:air", 1))
                    .is_none()
            );
            game.ui.rules.set("block_drops", RuleValue::Bool(false));
            assert!(
                game.level_pop_resource(level, [4, 64, -2], stone())
                    .is_none()
            );
            let _ = spill::pop_position(&mut random, [0; 3]);
            let _ = spill::pop_position(&mut random, [0; 3]);
            assert_eq!(random.next_long(), game.world.random.next_long());

            // new ItemEntity(level, x, y, z, stack, delta...): the pickup delay
            // is the caller's, as ItemEntity.setPickupDelay at construction.
            let spawned = game
                .level_spawn_item_with_motion(level, 1.5, 70.0, 2.5, stone(), [0.0, 0.3, 0.0], 40)
                .unwrap();
            assert_eq!(game.entity_position(spawned), Some([1.5, 70.0, 2.5]));
            let item = game.dropped.last().unwrap();
            assert_eq!((item.pickup_delay, item.velocity.y), (40, 0.3));
            assert!(
                game.level_spawn_item(level, 1.5, 70.0, 2.5, stone(), 32767)
                    .is_some()
            );
            assert_eq!(game.dropped.last().unwrap().pickup_delay, 32767);
            // Signed negative and > i16::MAX delays pass straight through in memory;
            // only NBT save narrows to (short), matching ItemEntity.setPickUpDelay.
            assert!(
                game.level_spawn_item(level, 1.5, 70.0, 2.5, stone(), -5)
                    .is_some()
            );
            assert_eq!(game.dropped.last().unwrap().pickup_delay, -5);
            assert!(
                game.level_spawn_item_with_motion(
                    level,
                    1.5,
                    70.0,
                    2.5,
                    stone(),
                    [0.0, 0.2, 0.0],
                    40_000,
                )
                .is_some()
            );
            assert_eq!(game.dropped.last().unwrap().pickup_delay, 40_000);
            assert!(
                game.level_spawn_item(level, f64::NAN, 70.0, 2.5, stone(), 0)
                    .is_none()
            );
            for empty in [
                ItemStack::new("minecraft:stone", 0),
                ItemStack::new("minecraft:nothing", 1),
            ] {
                assert!(
                    game.level_spawn_item(level, 0.0, 0.0, 0.0, empty, 0)
                        .is_none()
                );
            }

            // An item a hook spawns while the tick has the list out joins after
            // the others, which keep their numbers.
            use sunrise_sim::join_seq::Joined;
            let joins = |game: &sunrise_gameplay::game::ServerGame<'_>| -> Vec<(u32, u64)> {
                game.dropped
                    .iter()
                    .map(|d| (d.id, d.join_seq().get()))
                    .collect()
            };
            let before = joins(game);
            let out = game.dropped.lift();
            let hooked = game
                .level_spawn_item(level, 1.5, 70.0, 2.5, stone(), 0)
                .unwrap();
            assert_eq!(joins(game).len(), 1);
            game.put_back_dropped(out);
            let after = joins(game);
            assert_eq!(after[..before.len()], before[..]);
            assert_eq!(after[before.len()].0, hooked.id());
            assert!(after[before.len()].1 > before.iter().map(|j| j.1).max().unwrap());
            assert!(game.entity_kind(hooked).is_some());

            // ExperienceOrb.award: 100 is 73, 17, 7 and 3, far apart from other orbs.
            assert!(game.level_award_experience(level, 100.5, 70.0, 0.5, 100));
            let values: Vec<u32> = game.orbs.iter().map(|o| o.points()).collect();
            assert_eq!(values, [73, 17, 7, 3]);
            assert!(!game.level_award_experience(level, 100.5, 70.0, 0.5, 0));

            let stale = LevelRef::new(WorldEpoch::new(0), game.dim.key());
            assert!(
                game.level_spawn_item(stale, 0.0, 0.0, 0.0, stone(), 0)
                    .is_none()
            );
            assert!(game.level_pop_resource(stale, [0; 3], stone()).is_none());
            assert!(!game.level_award_experience(stale, 0.0, 0.0, 0.0, 5));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_orb_awarded_in_a_merge_hook_is_merged_by_a_later_scan() {
        use mod_api::{self, ExperienceOrbMergeEvent, Flow, Hooks};
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::entities::ExperienceOrb;

        // Each scanForMerges queries the level afresh (ExperienceOrb.java:183),
        // so B's scan finds the orb a hook awarded during A's.
        struct Award;
        #[mod_api::__subscriptions]
        impl Hooks for Award {
            fn experience_orb_merge(
                &self,
                host: &mut dyn ServerHost,
                event: &mut ExperienceOrbMergeEvent<'_>,
            ) -> Flow {
                if event.from == 80 {
                    let level = host.level(event.dimension).unwrap();
                    assert!(level.award_experience(host, 101.3, 70.0, 0.5, 3));
                }
                Flow::Continue
            }
        }
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("level-item-awards", Box::new(Award))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let orb = |id, x, value, ticks| {
                    let p = Vec3::new(x, 70.0, 0.5);
                    ExperienceOrb {
                        size_answer: None,
                        attachment_lifetime: Default::default(),
                        fall_distance: 0.0,
                        rotation: [0.0, 0.0],
                        join: Default::default(),
                        id,
                        position: p,
                        previous: p,
                        velocity: Vec3::ZERO,
                        value,
                        age: 0,
                        ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
                        count: 1,
                        fire_ticks: 0,
                        ticks,
                        health: 5,
                        air: 300,
                        no_gravity: true,
                        following: None,
                    }
                };
                // A takes X; B, far off, ticks after A. The award is id 160.
                // The per-orb scan runs before tick increments ticks, so 0 scans now.
                game.orbs.push(orb(40, 0.5, 3, 0));
                game.orbs.push(orb(80, 0.5, 3, 1));
                game.orbs.push(orb(120, 100.5, 3, 0));
                game.server.next_entity = 159;
                // Only initial residents take turns; each scan sees hook-awarded residents.
                let ticking: Vec<_> = game.orbs.iter().map(|orb| orb.join).collect();
                for join in ticking {
                    if let Some(index) = game.orbs.locate(join) {
                        game.merge_orb(index);
                    }
                }
                let left: Vec<(u32, u32)> = game.orbs.iter().map(|o| (o.id, o.count)).collect();
                assert_eq!(left, [(40, 2), (120, 2)]);
            });
        });
    }
}
