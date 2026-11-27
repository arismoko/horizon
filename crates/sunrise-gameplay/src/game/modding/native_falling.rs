//! Falling blocks for native server mods (docs/modding/src/how-to/world/spawning.md,
//! "Spawning from a level"): `LevelRef::spawn_falling_block` and the canonical
//! `FallingBlockEntity` construction and admission shared with `LevelEvent::FallingBlock`.
#[cfg(test)]
use super::FallingBlock;
use sunrise_core::geometry::Vec3;
#[cfg(test)]
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::{
    Dim, native_level_refs_common::NativeLevelRefsCommonGameExt1 as _,
};
use sunrise_world::registry::BlockStateId;

impl crate::game::ServerGame<'_> {
    /// `LevelRef::spawn_falling_block`: spawn in the named open server level,
    /// leaving its block at `pos` alone. Admission does not require a loaded
    /// chunk: `ServerLevel.addFreshEntity` does not use `/summon`'s placement checks.
    pub fn level_spawn_falling_block(
        &mut self,
        level: mod_api::LevelRef,
        state: BlockStateId,
        pos: [f64; 3],
        velocity: [f64; 3],
        falling: mod_api::Falling,
    ) -> Result<Option<mod_api::EntityRef>, mod_api::LevelWriteError> {
        self.native_level_world_result(level)?;
        if !pos.iter().chain(&velocity).all(|v| v.is_finite()) {
            return Ok(None);
        }
        let dim = match level.dimension() {
            mod_api::DimensionKey::OVERWORLD => Dim::Overworld,
            mod_api::DimensionKey::NETHER => Dim::Nether,
            mod_api::DimensionKey::END => Dim::End,
            key => Dim::Extra(key),
        };
        self.with_level_in_place(dim, |game| {
            let mut damage = sunrise_sim::falling_block::Damage::falling(
                &game.common.res.facts().get(state).name,
            );
            if falling.hurts {
                damage.hurt_entities = true;
                damage.per_distance = 2.0;
                damage.max = 40;
            }
            game.spawn_falling_block_entity(
                state,
                Vec3::new(pos[0], pos[1], pos[2]),
                Vec3::new(velocity[0], velocity[1], velocity[2]),
                falling.drop_item,
                damage,
            )
            .map(|id| mod_api::EntityRef::new(level.epoch(), id))
        })
        .ok_or(mod_api::LevelWriteError::Stale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{Arc, Mutex};
    use sunrise_mod_runtime::attachments::{
        AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
    };

    static VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "falling_lifetime",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync_codec: None,
    };

    #[test]
    #[ignore = "daily"]
    fn falling_tick_preserves_its_owner_on_success_and_collision_error() {
        bound_falling_server(|game, _| {
            let at = Vec3::new(4.5, 70.0, 0.5);
            let second = FallingBlock {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id: 72,
                start_pos: BlockPos::new(4, 70, 0),
                position: at,
                previous: at,
                ..game.falling[0].clone()
            };
            game.falling.push(second);
            let expected: Vec<_> = game.falling.iter().map(|f| (f.id, f.join)).collect();
            game.tick_one_falling(0, &[]).unwrap();
            assert_eq!(
                game.falling
                    .iter()
                    .map(|f| (f.id, f.join))
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(game.falling[0].age, 1);
            assert!(game.falling[0].position.y < 70.0);
            game.tick_one_falling(1, &[]).unwrap();
            assert_eq!(
                game.falling
                    .iter()
                    .map(|f| (f.id, f.join))
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!([game.falling[0].age, game.falling[1].age], [1, 1]);
            // A corrupt resident palette makes the real collision lookup fail.
            Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).set(0, 69, 0, 65_535);
            let position = game.falling[0].position;
            assert!(game.tick_one_falling(0, &[]).is_err());
            assert_eq!(
                game.falling
                    .iter()
                    .map(|f| (f.id, f.join))
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(game.falling[0].position, position);
            assert_eq!(game.falling[0].age, 1);
        });
    }

    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn falling_client_tick_commits_state_without_detaching_its_owner() {
        bound_falling_client(|game, _| {
            game.remote_projectile_data(
                71,
                &[(
                    8,
                    10,
                    sunrise_protocol::entity_data::Value::Pos([8, 90, -4]),
                )],
            );
            assert_eq!(game.falling[0].start_pos, BlockPos::new(8, 90, -4));
            for age in 1..=3 {
                let previous = game.falling[0].position;
                game.tick_remote_projectiles(|_, at| at);
                assert_eq!(game.falling[0].age, age);
                assert_eq!(game.falling[0].previous, previous);
                assert!(game.falling[0].position.y < previous.y);
                assert!(game.falling[0].velocity.y < 0.0);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn falling_parked_swap_preserves_state_and_owner_until_discard() {
        bound_falling_server(|game, index| {
            let home = game.dim;
            game.tick_one_falling(0, &[]).unwrap();
            let position = game.falling[0].position;
            game.enter_level(super::super::Dim::Nether).unwrap();
            assert!(game.falling.is_empty());
            assert_eq!(
                index
                    .lock()
                    .unwrap()
                    .snapshot(&("test".into(), "falling_lifetime".into()))
                    .len(),
                1
            );
            game.enter_level(home).unwrap();
            assert_eq!(game.falling.len(), 1);
            assert_eq!(game.falling[0].id, 71);
            assert_eq!(game.falling[0].age, 1);
            assert_eq!(game.falling[0].position, position);
        });
    }

    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn falling_lifecycle_commands_unload_park_and_remote() {
        use crate::game::{
            commands::Target,
            server_side::entity_server::taken_pass_tests::{
                LifecycleFixture, RemoteLifecycle, SlotIdentities,
            },
        };
        let mut identities = SlotIdentities::default();
        LifecycleFixture {
            seed: |game, entries| {
                game.falling.clear();
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:sand")
                    .unwrap();
                for &(id, at) in entries {
                    let pos = Vec3::new(at[0], at[1], at[2]);
                    game.falling.push(FallingBlock {
                        size_answer: None,
                        ground: Default::default(),
                        fire_ticks: 0,
                        sync_velocity: false,
                        attachment_lifetime: Default::default(),
                        rotation: [0.0; 2],
                        join: Default::default(),
                        id,
                        state,
                        blocks_building: true,
                        start_pos: BlockPos::new(
                            at[0].floor() as i32,
                            at[1].floor() as i32,
                            at[2].floor() as i32,
                        ),
                        position: pos,
                        previous: pos,
                        velocity: Vec3::ZERO,
                        age: 0,
                        no_gravity: false,
                        portal: Default::default(),
                        drop_item: false,
                        fall_distance: 0.0,
                        stuck: Vec3::ZERO,
                        damage: Default::default(),
                        saved: None,
                    });
                }
            },
            ids: |game| game.falling.iter().map(|f| f.id).collect(),
            target: Target::Falling,
            unloads_with_chunk: true,
            remote: Some(RemoteLifecycle {
                prepare: |game| game.clear_remote_projectiles(),
                apply: |game, remove, _| game.remove_remote_projectile(remove),
            }),
        }
        .run_with(|game| {
            identities.observe(
                &mut game.falling,
                game.native_lifecycle.epoch().unwrap(),
                |f| f.id,
            );
        });
    }

    fn run_bound_falling(
        game: &mut crate::game::ServerGame<'_>,
        exercise: impl FnOnce(
            &mut crate::game::ServerGame<'_>,
            &Arc<Mutex<AttachmentIndex<mod_api::EntityRef>>>,
        ),
        clear: impl FnOnce(&mut crate::game::ServerGame<'_>),
    ) {
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:sand")
            .unwrap();
        let at = Vec3::new(0.5, 70.0, 0.5);
        game.falling.push(FallingBlock {
            size_answer: None,
            ground: Default::default(),
            fire_ticks: 0,
            sync_velocity: false,
            attachment_lifetime: Default::default(),
            rotation: [0.0; 2],
            join: Default::default(),
            id: 71,
            state,
            blocks_building: true,
            start_pos: BlockPos::new(0, 70, 0),
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            age: 0,
            no_gravity: false,
            portal: Default::default(),
            drop_item: false,
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            damage: Default::default(),
            saved: None,
        });
        let id = game.falling[0].id;
        let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), id);
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut values = Attachments::default();
        VALUE.set(&mut values, Holder::Entity(id), 10).unwrap();
        let original = game.falling.update(0, |f| {
            f.attachment_lifetime().bind(&mut values, &index, holder);
            f.attachment_lifetime().registration().unwrap()
        });
        let key = ("test".into(), "falling_lifetime".into());
        let before = index.lock().unwrap().snapshot(&key);
        assert_eq!(before.len(), 1);
        exercise(game, &index);
        let current = game
            .falling
            .update(0, |f| f.attachment_lifetime().registration().unwrap());
        assert!(original.same_registration(&current));
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        assert_eq!(VALUE.get(&mut values, Holder::Entity(id)), Ok(10));
        // Exercise the production clear before any fixture cleanup can mask it.
        clear(game);
        assert!(game.falling.is_empty());
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
    }

    #[cfg(any())]
    fn bound_falling_client(
        exercise: impl FnOnce(
            &mut crate::game::ClientGame<'_>,
            &Arc<Mutex<AttachmentIndex<mod_api::EntityRef>>>,
        ),
    ) {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            run_bound_falling(game, exercise, |game| game.clear_remote_projectiles());
        });
    }

    fn bound_falling_server(
        exercise: impl FnOnce(
            &mut crate::game::ServerGame<'_>,
            &Arc<Mutex<AttachmentIndex<mod_api::EntityRef>>>,
        ),
    ) {
        super::super::native_entity_refs::tests::with_game(|game| {
            run_bound_falling(game, exercise, |game| game.falling.clear());
        });
    }

    use sunrise_gameplay_common::game::{
        native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
        transactions::TransactionsGameExt as _,
    };

    #[test]
    #[ignore = "daily"]
    fn level_spawn_falling_block_preserves_state_flags_and_exact_velocity() {
        sunrise_gameplay_common::game::in_own_process(
            "game::server_side::native_falling::tests::level_spawn_falling_block_preserves_state_flags_and_exact_velocity",
            || {
                use mod_api::{
                    BlockState, BlockStateError, Falling, LevelRef, LevelWriteError, WorldEpoch,
                };
                use sunrise_gameplay_common::game::api_values::TryIntoApi;

                super::super::native_entity_refs::tests::with_game(|game| {
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                    let level = game.level(game.dim.id()).unwrap();
                    let sand = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:sand")
                        .unwrap();
                    let anvil = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:anvil")
                        .unwrap();
                    let air = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:air")
                        .unwrap();
                    let api_sand: BlockState = sand.try_into_api().unwrap();
                    let api_anvil: BlockState = anvil.try_into_api().unwrap();
                    let api_air: BlockState = air.try_into_api().unwrap();
                    let invalid: BlockState =
                        mod_api::__private::block_state_from_ordinal(api_sand.block(), u16::MAX);

                    // The typed handle boundary rejects unknown states without unwinding.
                    assert_eq!(
                        level.spawn_falling_block(
                            game,
                            invalid,
                            [1.5, 70.0, 2.5],
                            [0.0; 3],
                            Falling::default(),
                        ),
                        Err(LevelWriteError::InvalidState(BlockStateError::UnknownState))
                    );

                    // Live velocity is kept unclamped (past the old 3.9 clamp), and flags are preserved.
                    let mut flags = Falling::default();
                    flags.drop_item = false;
                    flags.hurts = true;
                    let thrown = level
                        .spawn_falling_block(
                            game,
                            api_sand,
                            [1.5, 70.0, 2.5],
                            [0.5, 9.0, -9.0],
                            flags,
                        )
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        game.entity_kind(thrown)
                            .and_then(|kind| kind.id(game))
                            .as_deref(),
                        Some("minecraft:falling_block")
                    );
                    assert_eq!(game.entity_position(thrown), Some([1.5, 70.0, 2.5]));
                    assert_eq!(game.entity_velocity(thrown), Some([0.5, 9.0, -9.0]));
                    let block = game.falling.iter().find(|f| f.id == thrown.id()).unwrap();
                    assert_eq!(block.state, sand);
                    assert_eq!(block.start_pos, BlockPos::new(1, 70, 2));
                    assert_eq!(block.velocity, Vec3::new(0.5, 9.0, -9.0));
                    assert!(!block.drop_item);
                    assert!(block.damage.hurt_entities);
                    assert_eq!(block.damage.per_distance, 2.0);
                    assert_eq!(block.damage.max, 40);

                    // Anvil keeps its vanilla hurt defaults even with `hurts: false`.
                    let anvil_ref = level
                        .spawn_falling_block(
                            game,
                            api_anvil,
                            [2.5, 70.0, 2.5],
                            [0.0; 3],
                            Falling::default(),
                        )
                        .unwrap()
                        .unwrap();
                    let anvil_row = game
                        .falling
                        .iter()
                        .find(|f| f.id == anvil_ref.id())
                        .unwrap();
                    assert!(anvil_row.drop_item);
                    assert!(anvil_row.damage.hurt_entities);
                    assert_eq!(anvil_row.damage.per_distance, 2.0);
                    assert_eq!(anvil_row.damage.max, 40);

                    // Air spawns and is discarded on its first tick (FallingBlockEntity.tick).
                    let air_ref = level
                        .spawn_falling_block(
                            game,
                            api_air,
                            [3.5, 70.0, 2.5],
                            [0.0; 3],
                            Falling::default(),
                        )
                        .unwrap()
                        .unwrap();
                    let air_index = game
                        .falling
                        .iter()
                        .position(|f| f.id == air_ref.id())
                        .unwrap();
                    assert_eq!(game.falling[air_index].state, air);
                    game.tick_one_falling(air_index, &[]).unwrap();
                    assert!(!game.falling.iter().any(|f| f.id == air_ref.id()));

                    // No 1,024 per-level quota.
                    for i in 0..1030 {
                        let spawned = game
                            .level_spawn_falling_block(
                                level,
                                sand,
                                [0.5 + f64::from((i % 16) as u8), 70.0, 0.5],
                                [0.0; 3],
                                Falling::default(),
                            )
                            .unwrap();
                        assert!(spawned.is_some(), "falling block {i} should spawn");
                    }
                    assert!(game.falling.len() >= 1030);

                    // Stale handles and non-finite coordinates or velocity leave no new entity.
                    let count = game.falling.len();
                    let stale = LevelRef::new(WorldEpoch::new(0), game.dim.key());
                    assert_eq!(
                        stale.spawn_falling_block(
                            game,
                            api_sand,
                            [1.5, 70.0, 1.5],
                            [0.0; 3],
                            Falling::default(),
                        ),
                        Err(LevelWriteError::Stale)
                    );
                    for (pos, vel) in [
                        ([f64::NAN, 70.0, 1.5], [0.0; 3]),
                        ([1.5, f64::INFINITY, 1.5], [0.0; 3]),
                        ([1.5, 70.0, 1.5], [0.0, f64::NAN, 0.0]),
                        ([1.5, 70.0, 1.5], [f64::INFINITY, 0.0, 0.0]),
                    ] {
                        assert_eq!(
                            level.spawn_falling_block(game, api_sand, pos, vel, Falling::default()),
                            Ok(None),
                            "pos={pos:?} vel={vel:?}"
                        );
                    }
                    assert_eq!(game.falling.len(), count);
                });
                // Client side refuses statically: ClientGame has no level_spawn_falling_block.
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn fresh_fall_does_not_require_summon_bounds_or_a_loaded_chunk() {
        sunrise_gameplay_common::game::in_own_process(
            "game::server_side::native_falling::tests::fresh_fall_does_not_require_summon_bounds_or_a_loaded_chunk",
            || {
                use super::super::native_entity_refs::tests::{
                    collect_game_all_entities, collect_game_entities_in,
                };
                use mod_api::{EntityUuid, Falling};
                use sunrise_sim::join_seq::JoinSeq;

                super::super::native_guest_turn_tests::with_guests(|game| {
                    game.server.world_root = game.server.save.0.root.clone();
                    game.enter_level(Dim::Nether).unwrap();
                    let level = game.level(game.dim.id()).unwrap();
                    let host: &dyn mod_api::Host = game;
                    let sand = host.block_state_id("minecraft:sand").unwrap();
                    assert!(game.world.chunks.is_empty());

                    let unloaded = [1000.5, 70.0, 0.5];
                    let unloaded_chunk = (1000 >> 4, 0);
                    assert!(!game.world.loaded(1000, 0));
                    let hidden = level
                        .spawn_falling_block(game, sand, unloaded, [0.0; 3], Falling::default())
                        .unwrap()
                        .unwrap();
                    // The returned handle stays valid while the section is hidden, and
                    // does not force-load the column.
                    assert_eq!(game.entity_position(hidden), Some(unloaded));
                    let hidden_uuid = EntityUuid(game.entity_uuid(hidden).unwrap());
                    assert!(!game.world.loaded(1000, 0));

                    // Hidden sections are excluded from level lookup, spatial queries,
                    // ray traces, tracking, and ticking.
                    assert_eq!(game.entity(hidden.id()), None);
                    assert_eq!(
                        game.level_entity_by_uuid(
                            game.native_current_level().unwrap(),
                            hidden_uuid
                        ),
                        None
                    );
                    assert!(
                        !collect_game_all_entities(game, level)
                            .unwrap()
                            .contains(&hidden)
                    );
                    assert!(
                        collect_game_entities_in(
                            game,
                            level,
                            [999.0, 69.0, -1.0],
                            [1002.0, 72.0, 2.0]
                        )
                        .unwrap()
                        .is_empty()
                    );
                    assert!(
                        game.level_ray_trace_entities(
                            level,
                            [998.0, 70.5, 0.5],
                            [1003.0, 70.5, 0.5],
                            0.0,
                            None,
                        )
                        .unwrap()
                        .is_empty()
                    );
                    let mut tracked = sunrise_sim::lan::entity::States::default();
                    game.lan_entities(&mut tracked, false, 0, None);
                    assert!(
                        !tracked
                            .as_slice()
                            .iter()
                            .any(|e| e.id == hidden.id() as i32)
                    );
                    game.begin_ordered_ticks();
                    game.run_ordered_ticks(JoinSeq::MAX, true).unwrap();
                    let row = game.falling.iter().find(|f| f.id == hidden.id()).unwrap();
                    assert_eq!(
                        row.age, 0,
                        "an admitted entity in a non-ticking section does not tick"
                    );

                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));

                    let above_summon_bounds = [0.5, 20_000_000.0, 0.5];
                    let high = level
                        .spawn_falling_block(
                            game,
                            sand,
                            above_summon_bounds,
                            [0.0; 3],
                            Falling::default(),
                        )
                        .unwrap()
                        .unwrap();
                    assert_eq!(game.entity_position(high), Some(above_summon_bounds));
                    assert_eq!(game.entity(high.id()), Some(high));

                    // Once the column becomes accessible, the same admitted entity is
                    // discoverable through every lookup, query, and tracking pass.
                    game.world.chunks.insert(
                        unloaded_chunk,
                        Arc::new(sunrise_level::chunk::Chunk::empty(
                            unloaded_chunk.0,
                            unloaded_chunk.1,
                        )),
                    );
                    assert_eq!(game.entity(hidden.id()), Some(hidden));
                    assert_eq!(
                        game.level_entity_by_uuid(
                            game.native_current_level().unwrap(),
                            hidden_uuid
                        ),
                        Some(hidden)
                    );
                    assert!(
                        collect_game_all_entities(game, level)
                            .unwrap()
                            .contains(&hidden)
                    );
                    assert_eq!(
                        collect_game_entities_in(
                            game,
                            level,
                            [999.0, 69.0, -1.0],
                            [1002.0, 72.0, 2.0]
                        )
                        .unwrap(),
                        vec![hidden]
                    );
                    assert_eq!(
                        game.level_ray_trace_entities(
                            level,
                            [998.0, 70.5, 0.5],
                            [1003.0, 70.5, 0.5],
                            0.0,
                            None,
                        )
                        .unwrap()
                        .len(),
                        1
                    );
                    game.lan_entities(&mut tracked, false, 0, None);
                    assert!(
                        tracked
                            .as_slice()
                            .iter()
                            .any(|e| e.id == hidden.id() as i32)
                    );
                    assert!(tracked.as_slice().iter().any(|e| e.id == high.id() as i32));
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn fresh_fall_in_unloaded_column_persists_without_clobbering_disk_entities() {
        sunrise_gameplay_common::game::in_own_process(
            "game::server_side::native_falling::tests::fresh_fall_in_unloaded_column_persists_without_clobbering_disk_entities",
            || {
                use mod_api::Falling;
                use sunrise_core::nbt::Tag;
                use sunrise_level::world::PendingEntity;
                use sunrise_sim::save::SaveDir;

                let dir = tempfile::tempdir().unwrap();
                let unloaded_chunk = (1000 >> 4, 0);
                let unloaded_pos = [1000.5, 70.0, 0.5];
                let moved_pos = [1050.5, 70.0, 0.5];
                let discarded_pos = [1100.5, 70.0, 0.5];
                let moved_chunk = (1050 >> 4, 0);
                let discarded_chunk = (1100 >> 4, 0);
                let velocity = [0.125, -0.25, 0.0625];
                let existing_uuid = [11, 22, 33, 44];
                let existing_tag = Tag::Compound(
                    [
                        ("id".into(), Tag::String("minecraft:pig".into())),
                        ("UUID".into(), Tag::IntArray(existing_uuid.to_vec())),
                        (
                            "Pos".into(),
                            Tag::List(
                                [1002.5_f64, 64.0, 2.5]
                                    .map(|n| Tag::Double(n.to_bits()))
                                    .to_vec(),
                            ),
                        ),
                    ]
                    .into(),
                );

                let mut saved_fall_uuid = None;
                let mut saved_shape = None;
                super::super::native_entity_refs::tests::with_game(|game| {
                    let shape = game.server.save.0.shape;
                    saved_shape = Some(shape);
                    game.server.save.0 = Arc::new(SaveDir::new(
                        dir.path().to_path_buf(),
                        shape,
                        Some(Default::default()),
                    ));
                    game.server
                        .save
                        .0
                        .save_entities(vec![(unloaded_chunk, vec![existing_tag.clone()])])
                        .unwrap();
                    assert!(game.world.chunks.is_empty());
                    let level = game.level(game.dim.id()).unwrap();
                    let host: &dyn mod_api::Host = game;
                    let sand = host.block_state_id("minecraft:sand").unwrap();

                    let mut flags = Falling::default();
                    flags.drop_item = false;
                    flags.hurts = true;
                    let hidden = level
                        .spawn_falling_block(game, sand, unloaded_pos, velocity, flags)
                        .unwrap()
                        .unwrap();
                    assert!(!game.world.loaded(1000, 0));

                    let discarded = level
                        .spawn_falling_block(
                            game,
                            sand,
                            discarded_pos,
                            [0.0; 3],
                            Falling::default(),
                        )
                        .unwrap()
                        .unwrap();
                    game.save_entities(true).unwrap();
                    assert_eq!(
                        game.mobs.len(),
                        1,
                        "saving admitted the prior section records"
                    );
                    saved_fall_uuid = Some(sunrise_sim::lan::playerdata::uuid_ints(
                        game.entity_uuid(hidden).unwrap(),
                    ));

                    // Hold the worker's first snapshots while both sections are still hidden.
                    let old = [unloaded_chunk, discarded_chunk].map(|key| {
                        let records = game.server.save.0.load_entities(key.0, key.1).unwrap();
                        (key, records)
                    });
                    assert_eq!(old[0].1.len(), 2);
                    assert_eq!(old[1].1.len(), 1);
                    assert!(game.entity_teleport_to(hidden, moved_pos, None));
                    // Teleport follows /tp and can replace velocity; set the expected live
                    // motion through the same handle before saving the moved entity.
                    assert!(hidden.set_velocity(game, velocity));
                    assert!(discarded.discard(game));
                    game.save_entities(false).unwrap();
                    game.server.save.0.flush().unwrap();

                    // Worker snapshots read before the second save must not re-admit
                    // the discarded block, move its neighbour back, or duplicate the pig.
                    for (key, records) in old {
                        game.world.chunks.insert(
                            key,
                            Arc::new(sunrise_level::chunk::Chunk::empty(key.0, key.1)),
                        );
                        game.world.loaded_entities.push((
                            key,
                            records
                                .into_iter()
                                .map(PendingEntity::from_disk_section)
                                .collect(),
                        ));
                    }
                    game.spawn_loaded_entities();
                    game.native_removals_flush();
                    assert_eq!(game.falling.len(), 1);
                    assert_eq!(game.entity_position(hidden), Some(moved_pos));
                    assert_eq!(game.mobs.len(), 1);
                });

                // Reopen and admit the old columns first: neither may resurrect a fall.
                let reopened = SaveDir::new(dir.path().to_path_buf(), saved_shape.unwrap(), None);
                let records = [unloaded_chunk, discarded_chunk, moved_chunk]
                    .map(|key| (key, reopened.load_entities(key.0, key.1).unwrap()));
                assert_eq!(records.each_ref().map(|(_, tags)| tags.len()), [1, 0, 1]);
                super::super::native_entity_refs::tests::with_game(|game| {
                    for (key, tags) in records {
                        game.world.chunks.insert(
                            key,
                            Arc::new(sunrise_level::chunk::Chunk::empty(key.0, key.1)),
                        );
                        game.world.queue_entity_nbt(key, tags);
                    }
                    game.spawn_loaded_entities();
                    assert_eq!(game.mobs.len(), 1);
                    let mob_id = game.mobs[0].id;
                    assert_eq!(game.uuid_for(mob_id), existing_uuid);
                    assert_eq!(game.falling.len(), 1);
                    let sand = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:sand")
                        .unwrap();
                    let restored_id = {
                        let restored = &game.falling[0];
                        assert_eq!(restored.state, sand);
                        assert_eq!(
                            [
                                restored.position.x,
                                restored.position.y,
                                restored.position.z
                            ],
                            moved_pos
                        );
                        assert_eq!(
                            [
                                restored.velocity.x,
                                restored.velocity.y,
                                restored.velocity.z
                            ],
                            velocity
                        );
                        assert!(!restored.drop_item);
                        assert!(restored.damage.hurt_entities);
                        restored.id
                    };
                    assert_eq!(game.uuid_for(restored_id), saved_fall_uuid.unwrap());
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn discarded_fresh_fall_is_not_reannounced_on_subsequent_join_flush() {
        sunrise_gameplay_common::game::in_own_process(
            "game::server_side::native_falling::tests::discarded_fresh_fall_is_not_reannounced_on_subsequent_join_flush",
            || {
                use mod_api::{
                    EntityJoinLevelEvent, EntityLeaveLevelEvent, Falling, Flow, Hooks, ServerHost,
                };

                struct Observer {
                    joins: Arc<Mutex<Vec<u32>>>,
                    leaves: Arc<Mutex<Vec<u32>>>,
                }
                #[mod_api::__subscriptions]
                impl Hooks for Observer {
                    fn entity_join_level(
                        &self,
                        _: &mut dyn ServerHost,
                        event: &mut EntityJoinLevelEvent<'_>,
                    ) -> Flow {
                        if event.kind == "minecraft:falling_block" {
                            self.joins.lock().unwrap().push(event.subject.id());
                        }
                        Flow::Continue
                    }
                    fn entity_leave_level(
                        &self,
                        _: &mut dyn ServerHost,
                        event: &mut EntityLeaveLevelEvent<'_>,
                    ) {
                        if event.kind == "minecraft:falling_block" {
                            self.leaves.lock().unwrap().push(event.entity);
                        }
                    }
                }

                let joins = Arc::new(Mutex::new(Vec::new()));
                let leaves = Arc::new(Mutex::new(Vec::new()));
                let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                    "falling-discard-no-rejoin",
                    Box::new(Observer {
                        joins: joins.clone(),
                        leaves: leaves.clone(),
                    }),
                )]
                .into();

                super::super::native_entity_refs::tests::with_game(|game| {
                    mod_platform::testing::with_mods(mods, || {
                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                        let level = game.level(game.dim.id()).unwrap();
                        let host: &dyn mod_api::Host = game;
                        let sand = host.block_state_id("minecraft:sand").unwrap();

                        let first = level
                            .spawn_falling_block(
                                game,
                                sand,
                                [1.5, 70.0, 1.5],
                                [0.0; 3],
                                Falling::default(),
                            )
                            .unwrap()
                            .unwrap();
                        assert!(first.discard(game));
                        // Deferred removal keeps the row in `self.falling` until the
                        // sweep; a join flush or another spawn before that sweep must
                        // never re-announce the discarded entity.
                        assert!(game.falling.iter().any(|f| f.id == first.id()));
                        game.native_joins_flush(false, 0);
                        let second = level
                            .spawn_falling_block(
                                game,
                                sand,
                                [2.5, 70.0, 1.5],
                                [0.0; 3],
                                Falling::default(),
                            )
                            .unwrap()
                            .unwrap();
                        game.native_joins_flush(false, 0);
                        game.native_joins_skip();
                        game.native_leaves_flush();

                        assert_eq!(*joins.lock().unwrap(), vec![first.id(), second.id()]);
                        assert_eq!(*leaves.lock().unwrap(), vec![first.id()]);
                        assert_eq!(game.falling.len(), 1);
                        assert_eq!(game.falling[0].id, second.id());
                    });
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn parked_falling_spawn_uses_the_destination_journal() {
        sunrise_gameplay_common::game::in_own_process(
            "game::server_side::native_falling::tests::parked_falling_spawn_uses_the_destination_journal",
            || {
                use mod_api::{EntityJoinLevelEvent, Falling, Flow, Hooks, ServerHost};

                struct Listener(Arc<Mutex<Vec<String>>>);
                #[mod_api::__subscriptions]
                impl Hooks for Listener {
                    fn entity_join_level(
                        &self,
                        _: &mut dyn ServerHost,
                        event: &mut EntityJoinLevelEvent<'_>,
                    ) -> Flow {
                        if event.kind == "minecraft:falling_block" {
                            self.0.lock().unwrap().push(event.dimension.to_owned());
                        }
                        Flow::Continue
                    }
                }

                let heard = Arc::new(Mutex::new(Vec::new()));
                let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                    "parked-falling-join",
                    Box::new(Listener(heard.clone())),
                )]
                .into();
                super::super::native_entity_refs::tests::with_game(|game| {
                    mod_platform::testing::with_mods(mods, || {
                        game.server.world_root = game.server.save.0.root.clone();
                        let home = game.dim;
                        let level = game.level(home.id()).unwrap();
                        let host: &dyn mod_api::Host = game;
                        let sand = host.block_state_id("minecraft:sand").unwrap();
                        let at = [1.5, 70.0, 1.5];
                        game.storage_scope(|game, _| {
                            game.enter_level(Dim::Nether).unwrap();
                            let pending = level
                                .spawn_falling_block(game, sand, at, [0.0; 3], Falling::default())
                                .unwrap()
                                .unwrap();
                            assert_eq!(game.dim, Dim::Nether);
                            assert!(game.falling.is_empty());
                            assert!(heard.lock().unwrap().is_empty());
                            game.enter_level(home).unwrap();
                            assert_eq!(game.entity_position(pending), Some(at));
                            ((), false)
                        });
                        assert!(game.falling.is_empty());

                        game.enter_level(Dim::Nether).unwrap();
                        let joined = game.storage_scope(|game, _| {
                            let joined = level
                                .spawn_falling_block(game, sand, at, [0.0; 3], Falling::default())
                                .unwrap()
                                .unwrap();
                            assert_eq!(game.dim, Dim::Nether);
                            assert!(game.falling.is_empty());
                            assert_eq!(*heard.lock().unwrap(), [home.id()]);
                            (joined, false)
                        });
                        game.enter_level(home).unwrap();
                        assert_eq!(game.entity_position(joined), Some(at));
                        assert_eq!(game.falling.len(), 1);
                    });
                });
            },
        );
    }

    /// `EntityJoinLevelEvent.getEntity`: the join listener reads the joining entity through its
    /// own handle even where `Host::entity` cannot find it (a hidden section).
    #[test]
    #[ignore = "daily"]
    fn a_join_listener_reads_a_falling_block_entering_a_hidden_column_by_its_subject() {
        sunrise_gameplay_common::game::in_own_process(
            "game::server_side::native_falling::tests::a_join_listener_reads_a_falling_block_entering_a_hidden_column_by_its_subject",
            || {
                use mod_api::{EntityJoinLevelEvent, Falling, Flow, Hooks, Host, ServerHost};

                struct Listener(Arc<Mutex<Vec<(bool, Option<[f64; 3]>)>>>);
                #[mod_api::__subscriptions]
                impl Hooks for Listener {
                    fn entity_join_level(
                        &self,
                        host: &mut dyn ServerHost,
                        event: &mut EntityJoinLevelEvent<'_>,
                    ) -> Flow {
                        if event.kind == "minecraft:falling_block" {
                            let host: &dyn Host = host;
                            self.0.lock().unwrap().push((
                                host.entity(event.subject.id()).is_some(),
                                event.subject.position(host),
                            ));
                        }
                        Flow::Continue
                    }
                }

                let heard = Arc::new(Mutex::new(Vec::new()));
                let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                    "hidden-join-subject",
                    Box::new(Listener(heard.clone())),
                )]
                .into();
                super::super::native_entity_refs::tests::with_game(|game| {
                    mod_platform::testing::with_mods(mods, || {
                        game.server.world_root = game.server.save.0.root.clone();
                        let at = [1000.5, 70.0, 0.5];
                        let level = game.level(game.dim.id()).unwrap();
                        let host: &dyn mod_api::Host = game;
                        let sand = host.block_state_id("minecraft:sand").unwrap();
                        level
                            .spawn_falling_block(game, sand, at, [0.0; 3], Falling::default())
                            .unwrap()
                            .unwrap();
                    });
                });
                assert_eq!(*heard.lock().unwrap(), [(false, Some([1000.5, 70.0, 0.5]))]);
            },
        );
    }

    /// `PersistentEntitySectionManager.storeChunkSections`: a column whose old entity record
    /// cannot be read is left untouched, and the save says it was incomplete.
    #[test]
    #[ignore = "daily"]
    fn a_save_with_an_unreadable_old_entity_record_reports_the_incomplete_column() {
        sunrise_gameplay_common::game::in_own_process(
            "game::server_side::native_falling::tests::a_save_with_an_unreadable_old_entity_record_reports_the_incomplete_column",
            || {
                use mod_api::{Falling, Host};
                super::super::native_entity_refs::tests::with_game(|game| {
                    let root = tempfile::tempdir().unwrap();
                    game.server.world_root = root.path().to_path_buf();
                    let level = game.level(game.dim.id()).unwrap();
                    let host: &dyn Host = game;
                    let sand = host.block_state_id("minecraft:sand").unwrap();
                    // Column (62, 0), outside the kept terrain; its region file's entry for it
                    // holds a record that cannot be decoded.
                    let region = game.server.save.0.root.join("entities");
                    std::fs::create_dir_all(&region).unwrap();
                    // One sector at offset 2: a length, an unknown compression type, garbage.
                    let mut file = vec![0u8; 8192 + 4096];
                    file[120..124].copy_from_slice(&[0, 0, 2, 1]);
                    file[8192..8197].copy_from_slice(&[0, 0, 0, 64, 99]);
                    std::fs::write(region.join("r.1.0.mca"), file).unwrap();
                    level
                        .spawn_falling_block(
                            game,
                            sand,
                            [1000.5, 70.0, 0.5],
                            [0.0; 3],
                            Falling::default(),
                        )
                        .unwrap()
                        .unwrap();
                    let error = game.save_entities(true).unwrap_err();
                    assert!(error.to_string().contains("1 column"), "{error}");
                    assert!(game.entity_save_complete().is_err());
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn natural_falling_join_veto_runs_before_the_next_scheduled_tick() {
        sunrise_gameplay_common::game::in_own_process(
            "game::server_side::native_falling::tests::natural_falling_join_veto_runs_before_the_next_scheduled_tick",
            || {
                use mod_api::{BlockUpdateFlags, EntityJoinLevelEvent, Flow, Hooks, ServerHost};
                use sunrise_core::ticks::TickPriority;
                use sunrise_gameplay_common::game::GameGameExt1 as _;
                use sunrise_world::level::LevelAccess;

                const FIRST: BlockPos = BlockPos::new(1, 70, 1);
                const SECOND: BlockPos = BlockPos::new(3, 70, 1);
                struct Listener(Arc<Mutex<Vec<[f64; 3]>>>);
                #[mod_api::__subscriptions]
                impl Hooks for Listener {
                    fn entity_join_level(
                        &self,
                        host: &mut dyn ServerHost,
                        event: &mut EntityJoinLevelEvent<'_>,
                    ) -> Flow {
                        if event.kind != "minecraft:falling_block" {
                            return Flow::Continue;
                        }
                        self.0.lock().unwrap().push(event.pos);
                        let level = host.level(event.dimension).unwrap();
                        let stone = host.block_state_id("minecraft:stone").unwrap();
                        assert_eq!(
                            level.set_block(
                                host,
                                [SECOND.x, SECOND.y - 1, SECOND.z],
                                stone,
                                BlockUpdateFlags::NONE,
                            ),
                            Ok(true)
                        );
                        Flow::Cancel
                    }
                }

                let joins = Arc::new(Mutex::new(Vec::new()));
                let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                    "natural-falling-veto",
                    Box::new(Listener(joins.clone())),
                )]
                .into();
                super::super::native_entity_refs::tests::with_game(|game| {
                    mod_platform::testing::with_mods(mods, || {
                        for x in -1..=1 {
                            for z in -1..=1 {
                                let mut chunk = sunrise_level::chunk::Chunk::empty(x, z);
                                chunk.lit = true;
                                game.world.chunks.insert((x, z), Arc::new(chunk));
                            }
                        }
                        game.server
                            .tickets
                            .0
                            .update(Some(sunrise_sim::ticket_levels::Sources {
                                forced: vec![(0, 0)],
                                ..Default::default()
                            }));
                        let registry = game.pack.assets.registry;
                        let sand = registry.default_state("minecraft:sand").unwrap();
                        let air = registry.default_state("minecraft:air").unwrap();
                        let stone = registry.default_state("minecraft:stone").unwrap();
                        for pos in [FIRST, SECOND] {
                            game.world.set(pos.x, pos.y, pos.z, sand).unwrap();
                        }
                        game.with_level(|b, level, ticks| {
                            ticks
                                .add_chunk(FIRST.chunk(), Default::default(), Default::default())
                                .unwrap();
                            for (pos, priority) in
                                [(FIRST, TickPriority::High), (SECOND, TickPriority::Normal)]
                            {
                                ticks.schedule_block(
                                    level.game_time(),
                                    pos,
                                    b.block(sand),
                                    0,
                                    priority,
                                );
                            }
                        });

                        game.tick_scheduled().unwrap();
                        assert_eq!(game.world.block(FIRST.x, FIRST.y, FIRST.z), air);
                        assert_eq!(game.world.block(SECOND.x, SECOND.y, SECOND.z), sand);
                        assert_eq!(game.world.block(SECOND.x, SECOND.y - 1, SECOND.z), stone);
                        assert!(game.falling.is_empty());
                        game.native_joins_flush(false, 0);
                        assert_eq!(*joins.lock().unwrap(), [[1.5, 70.0, 1.5]]);
                    });
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn toss_listener_gets_the_falling_join_veto_before_its_spawn_returns() {
        sunrise_gameplay_common::game::in_own_process(
            "game::server_side::native_falling::tests::toss_listener_gets_the_falling_join_veto_before_its_spawn_returns",
            || {
                use mod_api::{
                    EntityJoinLevelEvent, Falling, Flow, Hooks, ItemTossEvent, ServerHost,
                };

                struct Listener(Arc<Mutex<Vec<&'static str>>>);
                #[mod_api::__subscriptions]
                impl Hooks for Listener {
                    fn item_toss(&self, host: &mut dyn ServerHost, _: &mut ItemTossEvent) -> Flow {
                        let level = host.level("minecraft:overworld").unwrap();
                        let sand = host.block_state_id("minecraft:sand").unwrap();
                        assert_eq!(
                            level.spawn_falling_block(
                                host,
                                sand,
                                [1.5, 70.0, 1.5],
                                [0.0; 3],
                                Falling::default()
                            ),
                            Ok(None)
                        );
                        self.0.lock().unwrap().push("spawn returned");
                        Flow::Continue
                    }
                    fn entity_join_level(
                        &self,
                        _: &mut dyn ServerHost,
                        event: &mut EntityJoinLevelEvent<'_>,
                    ) -> Flow {
                        match event.kind {
                            "minecraft:falling_block" => {
                                self.0.lock().unwrap().push("falling join");
                                Flow::Cancel
                            }
                            "minecraft:item" => {
                                self.0.lock().unwrap().push("item join");
                                Flow::Continue
                            }
                            _ => Flow::Continue,
                        }
                    }
                }

                let order = Arc::new(Mutex::new(Vec::new()));
                let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                    "toss-falling-veto",
                    Box::new(Listener(order.clone())),
                )]
                .into();
                super::super::native_entity_refs::tests::with_game(|game| {
                    mod_platform::testing::with_mods(mods, || {
                        let item = sunrise_items::items::ItemStack::new("minecraft:dirt", 1);
                        let tossed = game.toss_item(item, [0.5, 70.0, 0.5], [0.0; 3], None);
                        assert!(tossed.is_some());
                        assert!(game.falling.is_empty());
                        game.native_joins_flush(false, 0);
                        assert_eq!(
                            *order.lock().unwrap(),
                            ["falling join", "spawn returned", "item join"]
                        );
                    });
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn level_spawn_falling_block_joins_cancel_and_roll_back_in_transactions() {
        sunrise_gameplay_common::game::in_own_process(
            "game::server_side::native_falling::tests::level_spawn_falling_block_joins_cancel_and_roll_back_in_transactions",
            || {
                use mod_api::{
                    EntityConstructingEvent, EntityJoinLevelEvent, Falling, Flow, Holder, HostSide,
                    ServerHost,
                };
                use std::sync::atomic::{AtomicBool, Ordering};

                struct Listener {
                    heard: Arc<Mutex<Vec<String>>>,
                    cancel: Arc<AtomicBool>,
                }
                #[mod_api::__subscriptions]
                impl mod_api::Hooks for Listener {
                    fn entity_constructing(
                        &self,
                        _: HostSide<'_>,
                        event: &mut EntityConstructingEvent,
                    ) {
                        if let Holder::Entity(_) = event.holder {
                            self.heard
                                .lock()
                                .unwrap()
                                .push(format!("construct {}", event.kind));
                        }
                    }
                    fn entity_join_level(
                        &self,
                        _: &mut dyn ServerHost,
                        event: &mut EntityJoinLevelEvent<'_>,
                    ) -> Flow {
                        self.heard
                            .lock()
                            .unwrap()
                            .push(format!("join {} {}", event.kind, event.loaded_from_disk));
                        if self.cancel.load(Ordering::Relaxed) {
                            Flow::Cancel
                        } else {
                            Flow::Continue
                        }
                    }
                }

                let heard = Arc::new(Mutex::new(Vec::new()));
                let cancel = Arc::new(AtomicBool::new(false));
                let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                    "falling-listener",
                    Box::new(Listener {
                        heard: heard.clone(),
                        cancel: cancel.clone(),
                    }),
                )]
                .into();

                super::super::native_entity_refs::tests::with_game(|game| {
                    mod_platform::testing::with_mods(mods, || {
                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                        let level = game.level(game.dim.id()).unwrap();
                        let sand = game
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:sand")
                            .unwrap();

                        // Normal spawn runs construct + join and keeps the falling block.
                        let kept = game
                            .level_spawn_falling_block(
                                level,
                                sand,
                                [1.5, 70.0, 1.5],
                                [0.0; 3],
                                Falling::default(),
                            )
                            .unwrap()
                            .unwrap();
                        assert!(game.falling.iter().any(|f| f.id == kept.id()));
                        game.native_joins_flush(false, 0);
                        assert_eq!(
                            *heard.lock().unwrap(),
                            [
                                "construct minecraft:falling_block",
                                "join minecraft:falling_block false",
                            ]
                        );
                        heard.lock().unwrap().clear();

                        // Cancelled join removes the falling block and returns Ok(None).
                        cancel.store(true, Ordering::Relaxed);
                        let count = game.falling.len();
                        assert_eq!(
                            game.level_spawn_falling_block(
                                level,
                                sand,
                                [2.5, 70.0, 1.5],
                                [0.0; 3],
                                Falling::default(),
                            ),
                            Ok(None)
                        );
                        assert_eq!(game.falling.len(), count);
                        assert_eq!(
                            *heard.lock().unwrap(),
                            [
                                "construct minecraft:falling_block",
                                "join minecraft:falling_block false",
                            ]
                        );
                        heard.lock().unwrap().clear();
                        cancel.store(false, Ordering::Relaxed);

                        // Aborted transaction removes the falling block and never announces join.
                        let aborted = game.storage_scope(|game, _| {
                            let aborted = game
                                .level_spawn_falling_block(
                                    level,
                                    sand,
                                    [3.5, 70.0, 1.5],
                                    [0.0; 3],
                                    Falling::default(),
                                )
                                .unwrap()
                                .unwrap();
                            assert!(game.falling.iter().any(|f| f.id == aborted.id()));
                            assert_eq!(
                                *heard.lock().unwrap(),
                                ["construct minecraft:falling_block"]
                            );
                            (aborted, false)
                        });
                        assert!(!game.falling.iter().any(|f| f.id == aborted.id()));
                        assert_eq!(
                            *heard.lock().unwrap(),
                            ["construct minecraft:falling_block"]
                        );
                        heard.lock().unwrap().clear();

                        // Committed transaction announces join at commit and keeps the falling block.
                        let committed = game.storage_scope(|game, _| {
                            let committed = game
                                .level_spawn_falling_block(
                                    level,
                                    sand,
                                    [4.5, 70.0, 1.5],
                                    [0.0; 3],
                                    Falling::default(),
                                )
                                .unwrap()
                                .unwrap();
                            assert_eq!(
                                *heard.lock().unwrap(),
                                ["construct minecraft:falling_block"]
                            );
                            (committed, true)
                        });
                        assert!(game.falling.iter().any(|f| f.id == committed.id()));
                        assert_eq!(
                            *heard.lock().unwrap(),
                            [
                                "construct minecraft:falling_block",
                                "join minecraft:falling_block false",
                            ]
                        );
                    });
                });
            },
        );
    }
}
