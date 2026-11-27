//! `LevelRef` placement and effects from code: `place_block`, `update_from_neighbour_shapes` and
//! `block_break_effect`.

use super::native_entity_refs::tests::with_game;
use mod_api::BlockUpdateFlags;

fn loaded(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    // The writes are `level_set_block`'s, which an authoritative server only takes.
    game.options.dedicated = Some(Vec::new());
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
    );
}

fn state(
    game: &sunrise_gameplay::game::ServerGame<'_>,
    name: &str,
) -> sunrise_world::registry::BlockStateId {
    game.pack.assets.registry.default_state(name).unwrap()
}

/// A bed placed with no player gets its head (`setPlacedBy`), and placing it again changes nothing.
#[test]
#[ignore = "daily"]
fn a_block_placed_without_a_player_runs_set_placed_by() {
    with_game(|game| {
        loaded(game);
        let level = game.level(game.dim.id()).unwrap();
        let bed = state(game, "minecraft:red_bed");
        assert_eq!(
            game.level_place_block(level, [2, 70, 2], bed, true),
            Ok(true)
        );
        assert_eq!(game.world.block(2, 70, 2), bed);
        let head = game.world.block(2, 70, 1);
        let facts = game.res.facts();
        assert_eq!(
            facts.get(head).name,
            facts.get(bed).name,
            "the head was placed"
        );
        assert_eq!(
            game.level_place_block(level, [2, 70, 2], bed, true),
            Ok(false)
        );
    });
}

/// `JukeboxBlock.setPlacedBy` reads the supplied stack's RecordItem key but does
/// not load its NBT into the jukebox; only the block state reflects that key.
#[test]
#[ignore = "daily"]
fn a_post_write_placement_reads_the_stack_without_loading_its_data() {
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_items::items::ItemStack;
    use sunrise_sim::block_entity_item::BLOCK_ENTITY_DATA;

    with_game(|game| {
        loaded(game);
        let level = game.level(game.dim.id()).unwrap();
        let pos = [2, 70, 2];
        let jukebox = state(game, "minecraft:jukebox");
        assert_eq!(
            game.level_set_block(level, pos, jukebox, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert_eq!(level.set_placed_by(game, pos, None), Ok(true));
        assert_eq!(game.world.block(pos[0], pos[1], pos[2]), jukebox);

        let mut stack = ItemStack::new("minecraft:jukebox", 1);
        let data = sunrise_command::snbt::parse_fully(
            r#"{id:"minecraft:jukebox",RecordItem:{id:"minecraft:music_disc_cat",count:1}}"#,
        )
        .unwrap();
        stack.data_mut().set_other(BLOCK_ENTITY_DATA, Some(data));
        let mut stack = stack.try_into_api().unwrap();
        stack.set_count(100).unwrap();
        stack.set_damage(12);
        assert_eq!(
            level.set_placed_by(game, [128, 70, 2], Some(&stack)),
            Ok(false)
        );
        // An invalid component on an otherwise valid item refuses before the jukebox changes.
        let mut invalid = stack.clone();
        let mut patch = invalid.components().unwrap().as_nbt().clone();
        patch.insert("minecraft:consumable".into(), mod_api::Tag::Int(1));
        invalid.set_components(patch).unwrap();
        assert_eq!(
            level.set_placed_by(game, pos, Some(&invalid)),
            Err(mod_api::LevelWriteError::InvalidStack(
                mod_api::StackWriteError::InvalidEffectPayload
            ))
        );
        assert_eq!(game.world.block(pos[0], pos[1], pos[2]), jukebox);
        assert_eq!(level.set_placed_by(game, pos, Some(&stack)), Ok(true));
        let placed = game
            .pack
            .assets
            .registry
            .state(game.world.block(pos[0], pos[1], pos[2]))
            .unwrap();
        assert_eq!(
            placed.properties.get("has_record").map(String::as_str),
            Some("true")
        );
        assert!(
            game.block_entity_nbt(pos)
                .unwrap()
                .get("RecordItem")
                .is_none()
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_code_placement_notifies_only_the_blocks_part_with_its_stack() {
    use mod_api::{EntityRef, HostTransaction, LevelRef, ServerHost, SetPlacedByEvent};
    use std::sync::{Arc, Mutex};
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_items::items::ItemStack;

    #[derive(Debug, PartialEq)]
    struct Notice {
        level: LevelRef,
        pos: [i32; 3],
        placer: Option<EntityRef>,
        stack: Option<(String, u32)>,
    }

    const POS: [i32; 3] = [2, 70, 2];
    const ABOVE: [i32; 3] = [2, 71, 2];
    const OTHER: [i32; 3] = [3, 70, 2];

    #[derive(Clone, Default)]
    struct Probe(Arc<Mutex<Vec<Notice>>>);

    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for Probe {
        fn set_placed_by(&self, host: &mut dyn ServerHost, event: &mut SetPlacedByEvent<'_>) {
            self.0.lock().unwrap().push(Notice {
                level: event.level,
                pos: event.pos,
                placer: event.placer,
                stack: event
                    .stack
                    .and_then(|stack| Some((stack.item().id(host)?.to_owned(), stack.count()))),
            });
            let [x, y, z] = event.pos;
            event
                .level
                .set_block(host, [x, y + 1, z], event.state, BlockUpdateFlags::NONE)
                .unwrap();
        }
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Probe {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let wool = r.block_key("minecraft:white_wool");
            r.block_part(wool, self.clone());
        }
    }

    let probe = Probe::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "placement_probe",
        Box::new(probe.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            loaded(game);
            let level = game.level(game.dim.id()).unwrap();
            let wool = state(game, "minecraft:white_wool");
            let stone = state(game, "minecraft:stone");
            game.level_set_block(level, POS, wool, BlockUpdateFlags::NONE)
                .unwrap();
            game.level_set_block(level, OTHER, stone, BlockUpdateFlags::NONE)
                .unwrap();
            // The caller's stack is unrelated to the placed block. Route by the block, not the stack.
            let stack = ItemStack::new("minecraft:stone", 7).try_into_api().unwrap();
            assert_eq!(level.set_placed_by(game, OTHER, Some(&stack)), Ok(true));
            assert!(probe.0.lock().unwrap().is_empty());

            let before = level.block_state(game, ABOVE);
            let aborted: Result<(), mod_api::TransactionError<()>> = (game as &mut dyn ServerHost)
                .transaction(|host, _| {
                    assert_eq!(level.set_placed_by(host, POS, Some(&stack)), Ok(true));
                    assert_eq!(
                        level.block_state(host, ABOVE),
                        Some(wool.try_into_api().unwrap())
                    );
                    Err(())
                });
            assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(())));
            assert_eq!(level.block_state(game, ABOVE), before);
            assert_eq!(
                *probe.0.lock().unwrap(),
                [Notice {
                    level,
                    pos: POS,
                    placer: None,
                    stack: Some(("minecraft:stone".into(), 7)),
                }]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_command_block_placement_callback_saves_updated_entity_data() {
    use sunrise_level::containers::BlockEntity;

    with_game(|game| {
        loaded(game);
        let level = game.level(game.dim.id()).unwrap();
        let pos = [2, 70, 2];
        let command = state(game, "minecraft:command_block");
        assert_eq!(
            game.level_set_block(level, pos, command, BlockUpdateFlags::NONE),
            Ok(true)
        );
        let key = (pos[0], pos[1], pos[2]);
        let Some(BlockEntity::CommandBlock(entity)) =
            game.common.world.block_entities.get_mut(&key)
        else {
            panic!("placement must create a command block entity");
        };
        let feedback = game
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::SEND_COMMAND_FEEDBACK);
        entity.track_output = !feedback;
        game.world.modified.clear();
        assert_eq!(level.set_placed_by(game, pos, None), Ok(true));
        let Some(BlockEntity::CommandBlock(entity)) = game.world.block_entities.get(&key) else {
            panic!("the command block entity must remain");
        };
        assert_eq!(entity.track_output, feedback);
        assert!(
            game.world.modified.remove(&(0, 0)),
            "the block entity change must be saved"
        );
        assert_eq!(level.set_placed_by(game, pos, None), Ok(true));
        assert!(
            !game.world.modified.remove(&(0, 0)),
            "no change needs no save"
        );
    });
}

#[test]
#[ignore = "daily"]
fn placement_does_not_write_a_fenced_companion() {
    with_game(|game| {
        loaded(game);
        let level = game.level(game.dim.id()).unwrap();
        let foot = [2, 70, 2];
        let head = [2, 70, 1];
        let bed = state(game, "minecraft:red_bed");
        let stone = state(game, "minecraft:stone");
        assert_eq!(
            game.level_set_block(level, head, stone, BlockUpdateFlags::NONE),
            Ok(true)
        );
        game.world.carrier_fences.lock_cell(head);
        assert_eq!(game.level_place_block(level, foot, bed, false), Ok(false));
        assert_eq!(game.world.block(2, 70, 2), game.world.air);
        assert_eq!(
            game.level_set_block(level, foot, bed, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert_eq!(level.set_placed_by(game, foot, None), Ok(false));
        assert_eq!(game.world.block(2, 70, 1), stone);
        assert_eq!(game.world.block(2, 70, 2), bed);
    });
}

#[test]
#[ignore = "daily"]
fn a_tripwire_placement_cannot_rewrite_a_fenced_hook_or_wire() {
    with_game(|game| {
        loaded(game);
        let level = game.level(game.dim.id()).unwrap();
        let hook = state(game, "minecraft:tripwire_hook");
        let east = game.with_props(hook, &[("facing", "east")]).unwrap();
        let west = game.with_props(hook, &[("facing", "west")]).unwrap();
        let wire = state(game, "minecraft:tripwire");
        let stone = state(game, "minecraft:stone");
        let (origin, between, opposite) = ([2, 70, 2], [3, 70, 2], [4, 70, 2]);
        for (pos, block) in [
            ([1, 70, 2], stone),
            ([5, 70, 2], stone),
            (between, wire),
            (opposite, west),
        ] {
            assert_eq!(
                game.level_set_block(level, pos, block, BlockUpdateFlags::NONE),
                Ok(true)
            );
        }
        for fenced in [opposite, between] {
            game.world.carrier_fences.lock_cell(fenced);
            assert_eq!(
                game.level_place_block(level, origin, east, false),
                Ok(false)
            );
            assert_eq!(
                game.world.block(origin[0], origin[1], origin[2]),
                game.world.air
            );
            game.world.carrier_fences.unlock_cell(fenced);
        }
        assert_eq!(
            game.level_set_block(level, origin, east, BlockUpdateFlags::NONE),
            Ok(true)
        );
        let saved = [origin, opposite, between].map(|[x, y, z]| game.world.block(x, y, z));
        for fenced in [opposite, between] {
            game.world.carrier_fences.lock_cell(fenced);
            assert_eq!(level.set_placed_by(game, origin, None), Ok(false));
            assert_eq!(
                [origin, opposite, between].map(|[x, y, z]| game.world.block(x, y, z)),
                saved
            );
            game.world.carrier_fences.unlock_cell(fenced);
        }
        assert_eq!(level.set_placed_by(game, origin, None), Ok(true));
        for [x, y, z] in [origin, opposite, between] {
            assert_eq!(
                game.pack
                    .assets
                    .registry
                    .state(game.world.block(x, y, z))
                    .unwrap()
                    .properties
                    .get("attached")
                    .map(String::as_str),
                Some("true")
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn placement_previews_shape_updates_before_the_tripwire_callback() {
    use super::native_guest_turn_tests::with_guests;
    use mod_api::PlayerWorldRefusal;
    use sunrise_gameplay_common::game::api_values::TryIntoApi;

    with_guests(|game| {
        loaded(game);
        let level = game.level(game.dim.id()).unwrap();
        let player = game.player(13).unwrap();
        let hook = state(game, "minecraft:tripwire_hook");
        let east = game
            .with_props(hook, &[("facing", "east"), ("attached", "false")])
            .unwrap();
        let west_attached = game
            .with_props(hook, &[("facing", "west"), ("attached", "true")])
            .unwrap();
        let west_detached = game
            .with_props(hook, &[("facing", "west"), ("attached", "false")])
            .unwrap();
        let stone = state(game, "minecraft:stone");
        let (origin, near, far) = ([2, 70, 2], [3, 70, 2], [5, 70, 2]);
        for (pos, block) in [
            ([1, 70, 2], stone),
            (origin, stone),
            (near, east),
            ([6, 70, 2], stone),
            (far, west_attached),
        ] {
            assert_eq!(
                game.level_set_block(level, pos, block, BlockUpdateFlags::NONE),
                Ok(true)
            );
        }
        // The nearer hook faces the same way, so it hides the far one until
        // the placement write replaces the nearer hook's support and drops it.
        game.world.carrier_fences.lock_cell(far);
        assert_eq!(game.level_place_block(level, origin, east, true), Ok(false));
        assert_eq!(
            player.place_block(
                game,
                level,
                origin,
                east.try_into_api().unwrap(),
                mod_api::BlockFace::Up,
                true
            ),
            Err(PlayerWorldRefusal::Fenced)
        );
        let read = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            [origin, near, far].map(|[x, y, z]| game.world.block(x, y, z))
        };
        assert_eq!(read(game), [stone, east, west_attached]);
        game.world.carrier_fences.unlock_cell(far);
        assert_eq!(game.level_place_block(level, origin, east, true), Ok(true));
        assert_eq!(read(game), [east, game.world.air, west_detached]);
    });
}

#[test]
#[ignore = "daily"]
fn a_transaction_refuses_only_a_completing_wither_placement() {
    use super::native_guest_turn_tests::with_guests;
    use mod_api::{HostTransaction, LevelWriteError, PlayerWorldRefusal, ServerHost};
    use sunrise_gameplay_common::game::api_values::TryIntoApi;

    with_guests(|game| {
        loaded(game);
        game.ui.difficulty = 2;
        let level = game.level(game.dim.id()).unwrap();
        let player = game.player(13).unwrap();
        let skull = state(game, "minecraft:wither_skeleton_skull");
        let soul = state(game, "minecraft:soul_sand");
        let air = game.world.air;
        let cells = [
            ([4, 72, 4], skull),
            ([5, 72, 4], skull),
            ([6, 72, 4], skull),
            ([4, 71, 4], soul),
            ([5, 71, 4], soul),
            ([6, 71, 4], soul),
            ([4, 70, 4], air),
            ([5, 70, 4], soul),
            ([6, 70, 4], air),
        ];
        let at = [5, 72, 4];
        let alone = [1, 72, 1];
        let before = game.mobs.len();
        (game as &mut dyn ServerHost)
            .transaction(|host, _| -> Result<(), ()> {
                assert_eq!(
                    level.place_block(host, alone, skull.try_into_api().unwrap(), false),
                    Ok(true)
                );
                assert_eq!(level.set_placed_by(host, alone, None), Ok(true));
                Ok(())
            })
            .unwrap();
        assert_eq!(game.world.block(alone[0], alone[1], alone[2]), skull);
        for (pos, state) in cells
            .into_iter()
            .filter(|(pos, state)| *pos != at && *state != air)
        {
            assert_eq!(
                game.level_set_block(level, pos, state, BlockUpdateFlags::NONE),
                Ok(true)
            );
        }
        let projected: Result<(), mod_api::TransactionError<()>> = (game as &mut dyn ServerHost)
            .transaction(|host, _| {
                assert_eq!(
                    level.place_block(host, at, skull.try_into_api().unwrap(), false),
                    Err(LevelWriteError::InTransaction)
                );
                assert_eq!(
                    player.place_block(
                        host,
                        level,
                        at,
                        skull.try_into_api().unwrap(),
                        mod_api::BlockFace::Up,
                        false
                    ),
                    Err(PlayerWorldRefusal::InTransaction)
                );
                Err(())
            });
        assert_eq!(projected, Err(mod_api::TransactionError::Aborted(())));
        assert_eq!(game.world.block(at[0], at[1], at[2]), air);
        assert_eq!(
            game.level_set_block(level, at, skull, BlockUpdateFlags::NONE),
            Ok(true)
        );
        let aborted: Result<(), mod_api::TransactionError<()>> = (game as &mut dyn ServerHost)
            .transaction(|host, _| {
                assert_eq!(
                    level.set_placed_by(host, at, None),
                    Err(LevelWriteError::InTransaction)
                );
                assert_eq!(
                    level.place_block(host, at, skull.try_into_api().unwrap(), false),
                    Ok(false)
                );
                Err(())
            });
        assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(())));
        for ([x, y, z], state) in cells {
            assert_eq!(game.world.block(x, y, z), state);
        }
        assert_eq!(game.mobs.len(), before);
        assert_eq!(level.set_placed_by(game, at, None), Ok(true));
        assert_eq!(game.mobs.len(), before + 1);
        for ([x, y, z], _) in cells {
            assert_eq!(game.world.block(x, y, z), air);
        }
    });
}

/// A fence is fitted to the fence beside it without anything being written.
#[test]
#[ignore = "daily"]
fn a_state_is_fitted_to_its_neighbours_without_a_write() {
    with_game(|game| {
        loaded(game);
        let level = game.level(game.dim.id()).unwrap();
        let fence = state(game, "minecraft:oak_fence");
        let air = game.world.block(2, 70, 2);
        assert_eq!(
            game.level_set_block(level, [3, 70, 2], fence, BlockUpdateFlags::NONE),
            Ok(true)
        );
        let fitted = game
            .level_update_from_neighbour_shapes(level, [2, 70, 2], fence)
            .unwrap();
        let sides = game.pack.assets.registry.state(fitted).unwrap();
        let side = |name: &str| sides.properties.get(name).map(String::as_str);
        assert_eq!(
            side("east"),
            Some("true"),
            "it connects to the fence at its east"
        );
        for other in ["north", "south", "west"] {
            assert_eq!(side(other), Some("false"), "nothing stands on its {other}");
        }
        assert_eq!(game.world.block(2, 70, 2), air, "nothing written");
        assert_eq!(
            game.level_update_from_neighbour_shapes(level, [9, 70, 9], fence),
            Ok(fence),
            "no neighbour, nothing to fit"
        );
    });
}

/// The break effect is level event 2001 with the state's id, and a stale handle plays nothing. The
/// event plays at once (`Level.levelEvent`), so what is observed is what reaches the guests.
#[test]
#[ignore = "daily"]
fn a_break_effect_is_played_for_a_state() {
    with_game(|game| {
        loaded(game);
        game.server.audio_capture_active = true;
        let level = game.level(game.dim.id()).unwrap();
        let stone = state(game, "minecraft:stone");
        game.server.lan_events.clear();
        assert!(game.level_block_break_effect(level, [1, 70, 1], stone));
        assert!(
            game.server
                .lan_events
                .iter()
                .any(|e| e.kind == 2001 && e.pos == [1, 70, 1] && e.data == stone.0 as i32)
        );
        let stale = mod_api::LevelRef::new(
            mod_api::WorldEpoch::new(level.epoch().get() + 1),
            level.dimension(),
        );
        game.server.lan_events.clear();
        assert!(!game.level_block_break_effect(stale, [1, 70, 1], stone));
        assert!(game.server.lan_events.is_empty());
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
