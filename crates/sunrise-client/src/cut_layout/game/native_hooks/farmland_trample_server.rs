#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {
    use crate::game::native_hooks::farmland_trample::tests::Choice;
    use crate::game::native_hooks::farmland_trample::tests::FIELD;
    use crate::game::native_hooks::farmland_trample::tests::Gardener;
    use crate::game::native_hooks::farmland_trample::tests::LANDING;
    use crate::game::native_hooks::farmland_trample::tests::LANDING_Y;
    use crate::game::native_hooks::farmland_trample::tests::Trace;
    use mod_api::*;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game_side;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;

    #[test]
    #[ignore = "daily"]
    fn ordinary_moves_and_short_farmland_landings_do_not_pause() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::native_hooks_server::farmland_trample::tests::ordinary_moves_and_short_farmland_landings_do_not_pause",
            || {
                use sunrise_sim::{
                    health::{DamageLevel, LentLevel},
                    mobs::{Mob, ai},
                    motion::MotionWorld,
                };

                let trace = Trace::default();
                mod_platform::testing::with_mods(
                    vec![ModEntry::new(
                        "gardener",
                        Box::new(Gardener(Choice::Allow, trace.clone())),
                    )]
                    .into(),
                    || {
                        with_game(|game| {
                            game.options.dedicated = Some(Vec::new());
                            let (common, server) =
                                sunrise_gameplay_common::game::SideGame::common_and_state_mut(game);
                            server
                                .streaming
                                .insert_remote(&mut common.world, Chunk::empty(0, 0));
                            let registry = game.pack.assets.registry;
                            let farmland = registry.default_state("minecraft:farmland").unwrap();
                            let [x, y, z] = FIELD;
                            game.world.set(x, y, z, farmland).unwrap();
                            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                            let mut lent = LentLevel::take(&mut game.world);
                            {
                                let view = game.mob_world_base(&[]);
                                let mut level = DamageLevel::lent(
                                    &game.world,
                                    registry,
                                    &mut lent,
                                    Some(sunrise_gameplay_common::game::tables::GameItems::items(
                                        &*game,
                                    )),
                                    Some(&game.pack.block_registries),
                                    &game.damage_hooks,
                                );
                                for walking in [false, true] {
                                    let (start_y, velocity) = if walking {
                                        (LANDING_Y, Vec3::new(0.125, -0.08, 0.0))
                                    } else {
                                        // Even the smallest random draw cannot pass this short fall.
                                        (64.125, Vec3::new(0.0, -0.5, 0.0))
                                    };
                                    let mut mob = Mob::new(
                                        LANDING,
                                        pig.clone(),
                                        Vec3::new(1.5, start_y, 1.5),
                                        0.0,
                                    );
                                    mob.ai.goals = Default::default();
                                    mob.ai.targets = Default::default();
                                    mob.ground.on_ground = walking;
                                    mob.velocity = velocity;
                                    let mut trample = None;
                                    let progress = ai::tick_until_consumption(
                                        &mut mob,
                                        &view,
                                        &mut level,
                                        &mut Vec::new(),
                                        &mut trample,
                                    );
                                    assert!(trample.is_none());
                                    assert!(
                                        matches!(progress, ai::TickProgress::Complete(true)),
                                        "walking={walking}: {progress:?}"
                                    );
                                    assert!(mob.ground.on_ground);
                                    assert_eq!(mob.position.y, LANDING_Y);
                                    assert_eq!(mob.fall_distance, 0.0);
                                    assert_eq!(mob.position.x > 1.5, walking);
                                }
                            }
                            lent.give_back(&mut game.world);

                            // The guest motion pass finishes a short fall by itself:
                            // `None` means no continuation and no return to the owning host.
                            let mut guest =
                                PlayerMotion::new(game.motion, Vec3::new(1.5, LANDING_Y, 1.5))
                                    .unwrap();
                            guest.fall_distance = 0.125;
                            let (player_uuid, level, session_lock) =
                                sunrise_gameplay_common::game::side_ops::world::WorldSideGameExt::side_game_side_meta(game);
                            let side = game_side!(
                                game.common,
                                (player_uuid, level, session_lock),
                                game.common.level.enabled_features.as_arc(),
                                session_lock.then(|| game.common.res.items())
                            );
                            let items = match (
                                &game.common.server_items,
                                game.server.world_tables.0.as_ref(),
                            ) {
                                (Some(items), _) => &**items,
                                (None, Some(tables)) => &tables.items,
                                (None, None) => game.common.res.items(),
                            };
                            let mut motion = MotionWorld {
                                side: &side,
                                entities: &[],
                                player: &mut guest,
                                scene: &mut game.common.world,
                                assets: &game.common.pack.assets,
                                collision_binding: game.common.collision,
                                resources: game.common.motion,
                                health: &game.common.health_resources,
                                tag_items: Some(items),
                                loop_context: &game.common.loop_context,
                                damage_hooks: &game.common.damage_hooks,
                                block_registries: Some(&game.common.pack.block_registries),
                            };
                            assert!(
                                motion
                                    .check_guest_fall_damage(
                                        -0.125,
                                        true,
                                        farmland,
                                        BlockPos::new(x, y, z)
                                    )
                                    .unwrap()
                                    .is_none()
                            );
                            assert_eq!(guest.fall_distance, 0.0);
                            assert_eq!(game.world.block(x, y, z), farmland);
                            assert!(trace.lock().unwrap().is_empty());
                        });
                    },
                );
            },
        );
    }
}
