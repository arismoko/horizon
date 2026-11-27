use mod_api::{
    CriterionContext, CriterionTest, CriterionTestError, Identifier, TriggerCriterionError,
};
use serde_json::json;
use std::sync::Arc;
use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
use sunrise_items::items::ItemStack as EngineStack;
use sunrise_level::chunk::Chunk;
use sunrise_sim::advancements as a;

fn with_isolated_guests(
    name: &str,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;

    if !run_fresh_cases(name, "SUNRISE_CRITERIA_TEST_CHILD", &["server"]) {
        with_guests(test);
    }
}

fn install(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    game.build_registries();
    game.world.chunks.insert(
        (0, 0),
        Arc::new({
            let mut chunk = Chunk::empty(0, 0);
            chunk.lit = true; // a FULL read of a hand-made column finds it ready
            chunk
        }),
    );
    let mut tree = a::Tree::default();
    for (id, trigger, conditions) in [
        (
            "default",
            "default_block_use",
            json!({"location":{"type":"minecraft:location_check","predicate":{"position":{"x":0.5,"y":64.5,"z":0.5}}}}),
        ),
        (
            "item2",
            "item_used_on_block",
            json!({"location":{"type":"minecraft:match_tool","predicate":{"items":"minecraft:stone","count":2}}}),
        ),
        (
            "position_requires_chunk",
            "default_block_use",
            json!({"location":{"type":"minecraft:location_check","offsetX":4096,"predicate":{"block":{"blocks":"minecraft:air"}}}}),
        ),
        (
            "same_column_light",
            "default_block_use",
            json!({"location":{"type":"minecraft:location_check","offsetY":1,"predicate":{"light":{"light":{"min":15}}}}}),
        ),
        (
            "event_air",
            "default_block_use",
            json!({"location":{"type":"minecraft:match_block","blocks":"minecraft:air"}}),
        ),
        (
            "event_chest",
            "default_block_use",
            json!({"location":{"type":"minecraft:match_block","blocks":"minecraft:chest","state":{"waterlogged":"false"}}}),
        ),
        (
            "event_chest_entity",
            "item_used_on_block",
            json!({"location":{"type":"minecraft:match_block","blocks":"minecraft:chest","nbt":{"id":"minecraft:chest"}}}),
        ),
        (
            "event_wrong_entity",
            "item_used_on_block",
            json!({"location":{"type":"minecraft:match_block","blocks":"minecraft:chest","nbt":{"id":"minecraft:furnace"}}}),
        ),
        (
            "default_has_entity",
            "default_block_use",
            json!({"location":{"type":"minecraft:match_block","blocks":"minecraft:chest","nbt":{"id":"minecraft:chest"}}}),
        ),
        ("default_unconditional", "default_block_use", json!({})),
        (
            "any2",
            "any_block_use",
            json!({"location":{"type":"minecraft:match_tool","predicate":{"items":"minecraft:stone","count":2}}}),
        ),
        (
            "any1",
            "any_block_use",
            json!({"location":{"type":"minecraft:match_tool","predicate":{"items":"minecraft:stone","count":1}}}),
        ),
        ("any", "any_block_use", json!({})),
        (
            "consume",
            "consume_item",
            json!({"item":{"items":"minecraft:stone","count":2}}),
        ),
    ] {
        let trigger = format!("minecraft:{trigger}");
        let index = tree.list.len();
        let id = format!("test:{id}");
        tree.by_id.insert(id.clone(), index);
        tree.by_trigger
            .entry(trigger.clone())
            .or_default()
            .push((index, 0));
        tree.list.push(a::Advancement {
            id,
            parent: None,
            display: None,
            criteria: vec![a::Criterion {
                name: "seen".into(),
                trigger,
                conditions: conditions.into(),
                player: None,
            }],
            requirements: vec![vec!["seen".into()]],
            rewards: Default::default(),
        });
    }
    game.adv.tree = Arc::new(tree);
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update_all(|g| g.advancements_loaded = true);
}
fn has(game: &sunrise_gameplay::game::ServerGame<'_>, id: &str) -> bool {
    game.server
        .lan
        .as_ref()
        .unwrap()
        .hub
        .guest(12)
        .unwrap()
        .advancements
        .has(&format!("test:{id}"), "seen")
}

#[test]
#[ignore = "daily"]
fn default_block_use_awards_only_default() {
    with_guests(|g| {
        install(g);
        let p = g.player(12).unwrap();
        assert_eq!(p.trigger_block_use(g, [0, 64, 0]), Ok(()));
        assert!(has(g, "default"));
        assert!(!has(g, "any"));
        assert!(!has(g, "item2"));
        assert!(!has(g, "any1"));
        assert!(!has(g, "any2"));
    });
}

#[test]
#[ignore = "daily"]
fn item_block_use_distinguishes_snapshots() {
    with_guests(|g| {
        install(g);
        let p = g.player(12).unwrap();
        let key = mod_api::__private::item_key("minecraft:stone").unwrap();
        let original = mod_api::ItemStack::new(key, 2).unwrap();
        g.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |g| {
                g.inventory[0] = Some(EngineStack::new("minecraft:dirt", 1))
            })
            .unwrap();
        assert_eq!(
            p.trigger_item_used_on_block(g, [0, 64, 0], &original),
            Ok(())
        );
        assert!(has(g, "item2"));
        assert!(!has(g, "any1"));
        assert!(!has(g, "any"));
        assert!(!has(g, "any2"));
        assert!(!has(g, "default"));
    });
}

#[test]
#[ignore = "daily"]
fn block_use_refusals_grant_nothing() {
    with_guests(|g| {
        install(g);
        let p = g.player(12).unwrap();
        let invalid = mod_api::ItemStack::new(
            mod_api::__private::item_key("minecraft:stone").unwrap(),
            999,
        )
        .unwrap();
        assert_eq!(
            p.trigger_item_used_on_block(g, [0, 64, 0], &invalid),
            Err(TriggerCriterionError::Stack(
                mod_api::StackWriteError::CountExceeded { maximum: 64 }
            ))
        );
        struct Never;
        impl CriterionTest for Never {
            fn test(&mut self, _: &dyn CriterionContext) -> Result<bool, CriterionTestError> {
                panic!("vanilla guard must precede callback");
            }
        }
        assert_eq!(
            p.trigger_criterion(
                g,
                &Identifier::parse("minecraft:any_block_use").unwrap(),
                &mut Never
            ),
            Err(TriggerCriterionError::BuiltinTrigger)
        );
        let b = g.player(13).unwrap();
        g.as_guest(12, |g| {
            g.as_guest(13, |g| {
                assert_eq!(
                    p.trigger_block_use(g, [0, 64, 0]),
                    Err(TriggerCriterionError::PlayerUnavailable)
                );
                assert_eq!(b.trigger_block_use(g, [0, 64, 0]), Ok(()));
            })
            .unwrap();
        })
        .unwrap();
        let stale = mod_api::__private::game::player_ref(p.entity(), 999);
        assert_eq!(
            stale.trigger_block_use(g, [0, 64, 0]),
            Err(TriggerCriterionError::PlayerUnavailable)
        );
        for id in ["default", "item2", "any2", "any1", "any"] {
            assert!(!has(g, id));
        }
    });
}

#[test]
#[ignore = "daily"]
fn block_use_resolves_saved_event_block_without_requiring_the_location_neighbour() {
    use sunrise_level::{
        chunk::Light, chunk_streamer::ChunkStreamer as Streaming, containers::BlockEntity,
        level_loading::Status, testgen::TestTerrain,
    };
    use sunrise_sim::save::{Persistent, SaveDir};

    with_isolated_guests(
        "cut_layout::game::native_block_use_criteria_tests::block_use_resolves_saved_event_block_without_requiring_the_location_neighbour",
        |g| {
            install(g);
            let saves = tempfile::tempdir().unwrap();
            let registry = g.pack.assets.registry;
            let biomes = g.res.biomes();
            let save = Arc::new(SaveDir::new(
                saves.path().to_owned(),
                g.world.shape,
                Some(g.res.shared_items()),
            ));
            let column = (20, 22);
            let at = [column.0 * 16, 64, column.1 * 16];
            let [x, y, z] = at;
            let mut chunk = Chunk::empty(column.0, column.1);
            chunk.set(
                0,
                y,
                0,
                registry.default_state("minecraft:chest").unwrap().0,
            );
            chunk
                .block_entities
                .push(((x, y, z), BlockEntity::Chest(vec![None; 27])));
            save.save(&[Arc::new(chunk)], registry, biomes).unwrap();
            let mut streamer = Streaming::new(
                Arc::new(Persistent {
                    inner: Arc::new(TestTerrain::new(registry, biomes).unwrap()),
                    save,
                    registry: g.pack.assets.clone(),
                    biomes: biomes.clone(),
                }),
                g.pack.chunk_context.clone(),
                1,
            );
            streamer.radius = -1;
            let previous = std::mem::replace(&mut g.server.streaming, streamer);
            let p = g.player(12).unwrap();
            let key = mod_api::__private::item_key("minecraft:stone").unwrap();
            let original = mod_api::ItemStack::new(key, 2).unwrap();
            assert!(!g.world.chunks.contains_key(&column));
            assert_eq!(p.trigger_item_used_on_block(g, at, &original), Ok(()));
            assert_eq!(
                g.server
                    .streaming
                    .latest_status(&g.world, column.0, column.1),
                Some(Status::Full)
            );
            assert!(g.world.chunks.get(&column).unwrap().lit);
            assert!(has(g, "item2"));
            assert!(has(g, "event_chest_entity"));
            assert!(!has(g, "event_wrong_entity"));
            assert_eq!(p.trigger_block_use(g, at), Ok(()));
            assert!(has(g, "default_unconditional"));
            assert!(has(g, "event_chest"));
            assert!(has(g, "same_column_light"));
            assert!(!has(g, "default_has_entity"));
            assert!(!has(g, "event_air"));
            assert!(!has(g, "position_requires_chunk"));
            assert!(!g.server.streaming.commanded().contains(&column));

            // A resident column still at InitializeLight must wait for lighting before
            // same-column location_check predicates read light.
            let unlit = (24, 26);
            let mut chunk = Chunk::empty(unlit.0, unlit.1);
            let section = chunk.section_index(65).unwrap();
            Arc::make_mut(&mut chunk.sections[section]).light = Light::Uniform(0);
            g.world.chunks.insert(unlit, Arc::new(chunk));
            assert_eq!(
                g.server.streaming.latest_status(&g.world, unlit.0, unlit.1),
                Some(Status::InitializeLight)
            );
            assert_eq!(g.world.light(unlit.0 * 16, 65, unlit.1 * 16), 0);
            let p13 = g.player(13).unwrap();
            assert_eq!(
                p13.trigger_block_use(g, [unlit.0 * 16, 64, unlit.1 * 16]),
                Ok(())
            );
            assert_eq!(
                g.server.streaming.latest_status(&g.world, unlit.0, unlit.1),
                Some(Status::Full)
            );
            assert!(g.world.chunks.get(&unlit).unwrap().lit);
            assert!(
                g.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(13)
                    .unwrap()
                    .advancements
                    .has("test:same_column_light", "seen")
            );
            assert!(!g.server.streaming.commanded().contains(&unlit));
            g.server.streaming = previous;
        },
    );
}

#[test]
#[ignore = "daily"]
fn an_already_acting_guest_is_reached_in_their_own_level_and_the_outer_level_returns() {
    use sunrise_gameplay::game::commands::Target;
    use sunrise_gameplay_common::game::Dim;

    with_isolated_guests(
        "cut_layout::game::native_block_use_criteria_tests::an_already_acting_guest_is_reached_in_their_own_level_and_the_outer_level_returns",
        |g| {
            install(g);
            let outer = g.dim;
            assert_ne!(outer, Dim::Nether);
            g.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.dimension = Dim::Nether.id().to_owned())
                .unwrap();
            // A nested call: the guest's body is in place while the level is the outer one.
            g.server.lan_acting = Some(12);
            let seen = g.for_player(Target::Guest(12), |g| g.dim);
            assert_eq!(seen, Some(Dim::Nether));
            assert_eq!(g.dim, outer, "the outer level is restored");
            assert_eq!(
                g.server.lan_acting,
                Some(12),
                "no second body swap came and went"
            );
            g.server.lan_acting = None;
        },
    );
}

#[test]
#[ignore = "daily"]
fn an_unreachable_guest_level_cannot_award_criteria_in_the_current_level() {
    use sunrise_gameplay_common::game::Dim;

    with_isolated_guests(
        "cut_layout::game::native_block_use_criteria_tests::an_unreachable_guest_level_cannot_award_criteria_in_the_current_level",
        |g| {
            install(g);
            let root = tempfile::tempdir().unwrap();
            g.options.resources = root.path().join("missing-resources");
            let here = g.dim;
            let player = g.player(12).unwrap();
            g.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.dimension = Dim::Nether.id().to_owned();
                })
                .unwrap();
            assert!(g.open_runtime_level(Dim::Nether).is_err());
            assert_eq!(
                player.trigger_block_use(g, [0, 64, 0]),
                Err(TriggerCriterionError::PlayerUnavailable)
            );
            assert!(!has(g, "default_unconditional"));
            assert_eq!(g.dim, here);
            assert!(g.server.lan_acting.is_none());
        },
    );
}

#[test]
#[ignore = "daily"]
fn custom_trigger_borrows_conditions_and_uses_the_target_players_predicate() {
    use mod_api::ItemStack;

    struct Pipe<'a> {
        stack: &'a ItemStack,
        counts: &'a mut Vec<u64>,
    }
    impl CriterionTest for Pipe<'_> {
        fn test(&mut self, criterion: &dyn CriterionContext) -> Result<bool, CriterionTestError> {
            let count = criterion.conditions()["minimum"].as_u64().unwrap();
            self.counts.push(count);
            assert!(criterion.item("missing", self.stack)?);
            criterion.item("pipe", self.stack)
        }
    }

    with_isolated_guests(
        "cut_layout::game::native_block_use_criteria_tests::custom_trigger_borrows_conditions_and_uses_the_target_players_predicate",
        |g| {
            install(g);
            let dir = tempfile::tempdir().unwrap();
            let advancements = dir.path().join("test/advancement");
            let predicates = dir.path().join("test/predicate");
            std::fs::create_dir_all(&advancements).unwrap();
            std::fs::create_dir_all(&predicates).unwrap();
            std::fs::write(
                predicates.join("dry_weather.json"),
                json!({
                    "type": "minecraft:weather_check",
                    "raining": false,
                    "condition": "minecraft:all_of"
                })
                .to_string(),
            )
            .unwrap();
            std::fs::write(
                predicates.join("rainy_weather.json"),
                json!({
                    "type": "minecraft:weather_check",
                    "raining": true,
                    "condition": "minecraft:all_of"
                })
                .to_string(),
            )
            .unwrap();
            for (id, player) in [
                ("pipe", serde_json::Value::Null),
                (
                    "pipe_burning",
                    json!({
                        "type": "minecraft:entity_properties", "entity": "this",
                        "predicate": {"minecraft:flags": {"is_on_fire": true}}
                    }),
                ),
                (
                    "pipe_conflicting",
                    json!({
                        "type": "minecraft:weather_check",
                        "raining": true,
                        "condition": "minecraft:all_of"
                    }),
                ),
                (
                    "pipe_ref_ok",
                    json!({"type": "minecraft:all_of", "terms": ["test:dry_weather"]}),
                ),
                (
                    "pipe_ref_rainy",
                    json!({"type": "minecraft:all_of", "terms": ["test:rainy_weather"]}),
                ),
                (
                    "pipe_ref_missing",
                    json!({"type": "minecraft:all_of", "terms": ["test:missing_predicate"]}),
                ),
                ("pipe_missing_terms", json!({"type": "minecraft:all_of"})),
            ] {
                std::fs::write(
                    advancements.join(format!("{id}.json")),
                    json!({"criteria":{"seen":{"trigger":"minecraft:test_pipe_connected",
                        "conditions":{"minimum":2,"pipe":{"items":"#minecraft:logs","count":{"min":2}},"player":player}}}}).to_string(),
                )
                .unwrap();
            }
            g.adv.tree = Arc::new(a::Tree::load(dir.path()));
            assert!(g.adv.tree.get("test:pipe_ref_missing").is_none());
            assert!(g.adv.tree.get("test:pipe_missing_terms").is_none());
            let p = g.player(12).unwrap();
            let trigger = Identifier::parse("minecraft:test_pipe_connected").unwrap();
            let log = mod_api::__private::item_key("minecraft:oak_log").unwrap();
            let stone = mod_api::__private::item_key("minecraft:stone").unwrap();
            let mut counts = Vec::new();
            assert_eq!(
                p.trigger_criterion(
                    g,
                    &trigger,
                    &mut Pipe {
                        stack: &ItemStack::new(log, 1).unwrap(),
                        counts: &mut counts,
                    }
                ),
                Ok(())
            );
            assert!(!has(g, "pipe"));
            assert!(!has(g, "pipe_ref_ok"));
            assert_eq!(
                p.trigger_criterion(
                    g,
                    &trigger,
                    &mut Pipe {
                        stack: &ItemStack::new(stone, 2).unwrap(),
                        counts: &mut counts,
                    }
                ),
                Ok(())
            );
            assert!(!has(g, "pipe"));
            assert!(!has(g, "pipe_ref_ok"));
            assert_eq!(
                p.trigger_criterion(
                    g,
                    &trigger,
                    &mut Pipe {
                        stack: &ItemStack::new(log, 2).unwrap(),
                        counts: &mut counts,
                    }
                ),
                Ok(())
            );
            assert!(has(g, "pipe"));
            assert!(has(g, "pipe_ref_ok"));
            assert!(!has(g, "pipe_burning"));
            assert!(!has(g, "pipe_conflicting"));
            assert!(!has(g, "pipe_ref_rainy"));
            assert_eq!(counts, [2; 15]);
        },
    );
}

#[test]
#[ignore = "daily"]
fn remote_block_use_refuses() {
    // `ClientGame` cannot be passed to `PlayerRef::trigger_block_use(&mut dyn ServerHost, ..)` at
    // compile time; exercise `ServerGame::with_criterion_player`'s remaining runtime `!is_logical_server` guard.
    with_guests(|g| {
        let p = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 1),
            12,
        );
        g.options.execution_role = sunrise_gameplay_common::execution_role::ExecutionRole::Client;
        assert_eq!(
            p.trigger_block_use(g, [0, 64, 0]),
            Err(TriggerCriterionError::NotServer)
        );
    });
}

#[test]
#[ignore = "daily"]
fn consume_item_uses_supplied_snapshot_and_refuses_invalid_inputs() {
    // `ClientGame` does not implement `ServerHost`; exercise the remaining runtime `NotServer` guard.
    with_guests(|g| {
        let p = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 7),
            12,
        );
        let key = mod_api::__private::item_key("minecraft:stone").unwrap();
        let stack = mod_api::ItemStack::new(key, 1).unwrap();
        g.options.execution_role = sunrise_gameplay_common::execution_role::ExecutionRole::Client;
        assert_eq!(
            p.trigger_consume_item(g, &stack),
            Err(TriggerCriterionError::NotServer)
        );
    });
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_guests(|g| {
            install(g);
            let p = g.player(12).unwrap();
            let key = mod_api::__private::item_key("minecraft:stone").unwrap();
            let one = mod_api::ItemStack::new(key, 1).unwrap();
            assert_eq!(p.trigger_consume_item(g, &one), Ok(()));
            assert!(!has(g, "consume"));
            let invalid = mod_api::ItemStack::new(key, 999).unwrap();
            assert_eq!(
                p.trigger_consume_item(g, &invalid),
                Err(TriggerCriterionError::Stack(
                    mod_api::StackWriteError::CountExceeded { maximum: 64 }
                ))
            );
            assert!(!has(g, "consume"));
            let two = mod_api::ItemStack::new(key, 2).unwrap();
            assert_eq!(p.trigger_consume_item(g, &two), Ok(()));
            assert!(has(g, "consume"));
            assert!(!has(g, "any"));
            g.as_guest(12, |g| {
                g.as_guest(13, |g| {
                    assert_eq!(
                        p.trigger_consume_item(g, &two),
                        Err(TriggerCriterionError::PlayerUnavailable)
                    );
                })
                .unwrap();
            })
            .unwrap();
            let stale = mod_api::__private::game::player_ref(p.entity(), 999);
            assert_eq!(
                stale.trigger_consume_item(g, &two),
                Err(TriggerCriterionError::PlayerUnavailable)
            );
        });
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
