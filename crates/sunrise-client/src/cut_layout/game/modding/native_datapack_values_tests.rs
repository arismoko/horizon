use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::{
    __private::VanillaValueDecoder, BlockPredicateValue, BlockStateProviderValue, DatapackField,
    DatapackFieldKind, DatapackRegistrySync, DatapackValue, DatapackValueError, DatapackVariant,
    HolderSetValue, Hooks, IntProviderValue, LegacyRandomSource, LevelRef, ModEntry, RandomSource,
    Registrar, RegistrationContext,
};
use std::sync::Arc;
use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;
use sunrise_client_gameplay::game::client_side::remote_client::{
    ingress_state::PacketError, session_state::RemoteDimensionEntry,
};
use sunrise_core::geometry::Vec3;
use sunrise_core::random::LegacyRandom;
use sunrise_gameplay::game::native_entity_refs::tests::{run_fresh_cases, with_game};
use sunrise_gameplay::game::server_side::world_loading::WorldRequest;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_gameplay_common::game::native_datapack_values::{
    DatapackValuesSide as _, GameDatapackValues as _, resolve_block_tags,
};
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_gameplay_common::game::tables::GameItems as _;
use sunrise_sim::datapack_net;
use sunrise_world::worldgen::block_feature::integer_properties;
use sunrise_world::worldgen::block_predicate::BlockPredicate;
use sunrise_world::worldgen::state_provider::{BlockStateProvider, StateProviderBindings};
use sunrise_world::worldgen::value_provider::IntProvider;

fn request(root: &std::path::Path) -> WorldRequest {
    let mut request = WorldRequest::new(
        "test",
        Some("test"),
        Some(1),
        true,
        None,
        Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
    )
    .unwrap();
    request.save_root = root.to_owned();
    request
}

struct VanillaValuesMod;
#[mod_api::__subscriptions]
impl Hooks for VanillaValuesMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.declare_datapack_registry(
            "bone_meal_rule",
            DatapackRegistrySync::Required,
            vec![
                DatapackField::required(
                    "blocks_to_grow",
                    DatapackFieldKind::IntProvider { min: 0, max: 128 },
                ),
                DatapackField::required(
                    "increase",
                    DatapackFieldKind::IntProvider { min: 0, max: 16 },
                ),
                DatapackField::required("vegetation", DatapackFieldKind::BlockStateProvider),
                DatapackField::required("ground_state", DatapackFieldKind::BlockStateProvider),
                DatapackField::required("predicate", DatapackFieldKind::BlockPredicate),
                DatapackField::required(
                    "spread_sources",
                    DatapackFieldKind::HolderSet {
                        registry: "minecraft:block".into(),
                    },
                ),
                DatapackField::required(
                    "tag_targets",
                    DatapackFieldKind::HolderSet {
                        registry: "minecraft:block".into(),
                    },
                ),
                DatapackField::required(
                    "single_target",
                    DatapackFieldKind::HolderSet {
                        registry: "minecraft:block".into(),
                    },
                ),
                DatapackField::required(
                    "variant",
                    DatapackFieldKind::Dispatch {
                        key: "type".into(),
                        variants: vec![DatapackVariant::new(
                            "ubm_test:random_tick",
                            vec![DatapackField::with_default(
                                "growth_attempts",
                                DatapackFieldKind::IntProvider { min: 1, max: 64 },
                                DatapackValue::int_provider("1"),
                            )],
                        )],
                    },
                ),
            ],
        )
        .unwrap();
    }
}

fn write(path: &std::path::Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

struct HostileRng;
impl RandomSource for HostileRng {
    fn next_int(&mut self) -> i32 {
        0
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        bound
    }
    fn next_long(&mut self) -> i64 {
        0
    }
    fn next_boolean(&mut self) -> bool {
        false
    }
    fn next_float(&mut self) -> f32 {
        0.0
    }
    fn next_double(&mut self) -> f64 {
        0.0
    }
    fn next_gaussian(&mut self) -> f64 {
        0.0
    }
}

#[test]
#[ignore = "daily"]
fn datapack_vanilla_values_agree_with_worldgen_and_invalidate_on_reload() {
    if run_fresh_cases(
        "cut_layout::game::native_datapack_values_tests::datapack_vanilla_values_agree_with_worldgen_and_invalidate_on_reload",
        "SUNRISE_DATAPACK_VANILLA_VALUES_CHILD",
        &["all"],
    ) {
        return;
    }
    let _publication = datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let pack = root.join("datapacks/ubm_fixture");
    write(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"ubm vanilla values fixture","min_format":94,"max_format":94}}"#,
    );
    let tag_file = pack.join("data/ubm_test/tags/block/bonemeal_targets.json");
    write(
        &tag_file,
        r#"{"values":["minecraft:dirt","minecraft:sand"]}"#,
    );
    let uniform_json = r#"{"type":"minecraft:uniform","min_inclusive":1,"max_inclusive":2}"#;
    let weighted_int_json = r#"{"type":"minecraft:weighted_list","distribution":[{"data":0,"weight":1},{"data":1,"weight":3}]}"#;
    let cactus_provider_json = r#"{"type":"minecraft:weighted","entries":[{"data":"minecraft:cactus","weight":9},{"data":"minecraft:cactus_flower","weight":1}]}"#;
    let podzol_provider_json = r#"{"type":"minecraft:weighted","entries":[{"data":{"id":"minecraft:sweet_berry_bush","properties":{"age":"3"}},"weight":2},{"data":"minecraft:fern","weight":120}]}"#;
    let air_above_json =
        r#"{"type":"minecraft:matching_block_tag","offset":[0,1,0],"tag":"minecraft:air"}"#;
    let rule_file = pack.join("data/ubm_test/ubm_test/bone_meal_rule/cactus.json");
    let rule_v1 = format!(
        r##"{{
            "blocks_to_grow": {uniform_json},
            "increase": {weighted_int_json},
            "vegetation": {cactus_provider_json},
            "ground_state": {podzol_provider_json},
            "predicate": {air_above_json},
            "spread_sources": ["minecraft:grass_block", "minecraft:mycelium"],
            "tag_targets": "#ubm_test:bonemeal_targets",
            "single_target": "minecraft:fern",
            "variant": {{"type": "ubm_test:random_tick"}}
        }}"##
    );
    write(&rule_file, &rule_v1);
    write(
        &root.join("datapacks.json"),
        r#"{"enabled":["vanilla","file/ubm_fixture"],"disabled":[]}"#,
    );

    mod_platform::testing::with_mods(
        vec![ModEntry::new("ubm_test", Box::new(VanillaValuesMod))].into(),
        || {
            let mut captured = None;
            with_game(|game| {
                game.load_world(request(root)).unwrap();
                let level = game.level(game.common.dim.id()).unwrap();
                let stone = game
                    .common
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let air = game
                    .common
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:air")
                    .unwrap();
                if !game.common.world.chunks.contains_key(&(0, 0)) {
                    game.common
                        .world
                        .chunks
                        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                }
                game.common.world.set(0, 64, 0, stone).unwrap();
                game.common.world.set(0, 65, 0, air).unwrap();
                game.common.world.set(0, 66, 0, stone).unwrap();

                let mut blocks_to_grow = None;
                let mut increase = None;
                let mut vegetation = None;
                let mut ground_state = None;
                let mut predicate = None;
                let mut spread_sources = None;
                let mut tag_targets = None;
                let mut single_target = None;
                let mut growth_attempts = None;
                level
                    .datapack_read(
                        game,
                        "ubm_test:bone_meal_rule",
                        "ubm_test:cactus",
                        &mut |entry| {
                            blocks_to_grow = entry
                                .get("blocks_to_grow")
                                .and_then(DatapackValue::as_int_provider);
                            increase = entry
                                .get("increase")
                                .and_then(DatapackValue::as_int_provider);
                            vegetation = entry
                                .get("vegetation")
                                .and_then(DatapackValue::as_block_state_provider);
                            ground_state = entry
                                .get("ground_state")
                                .and_then(DatapackValue::as_block_state_provider);
                            predicate = entry
                                .get("predicate")
                                .and_then(DatapackValue::as_block_predicate);
                            spread_sources = entry
                                .get("spread_sources")
                                .and_then(DatapackValue::as_holder_set);
                            tag_targets = entry
                                .get("tag_targets")
                                .and_then(DatapackValue::as_holder_set);
                            single_target = entry
                                .get("single_target")
                                .and_then(DatapackValue::as_holder_set);
                            growth_attempts = entry
                                .get("variant")
                                .and_then(|v| v.get("growth_attempts"))
                                .and_then(DatapackValue::as_int_provider);
                        },
                    )
                    .unwrap();
                let blocks_to_grow = blocks_to_grow.unwrap();
                let increase = increase.unwrap();
                let vegetation = vegetation.unwrap();
                let ground_state = ground_state.unwrap();
                let predicate = predicate.unwrap();
                let spread_sources = spread_sources.unwrap();
                let tag_targets = tag_targets.unwrap();
                let single_target = single_target.unwrap();
                let growth_attempts = growth_attempts.unwrap();

                // 1. IntProvider agrees with sunrise-world's evaluator on the same JSON and seed.
                for (handle, json) in [
                    (blocks_to_grow, uniform_json),
                    (increase, weighted_int_json),
                    (growth_attempts, "1"),
                ] {
                    let direct = IntProvider::from_json(json).unwrap();
                    let mut mod_rng = LegacyRandomSource::new(0x5368_1234);
                    let mut engine_rng = LegacyRandom::new(0x5368_1234);
                    for _ in 0..32 {
                        assert_eq!(
                            handle.sample(game, &mut mod_rng).unwrap(),
                            direct.sample(&mut engine_rng).unwrap()
                        );
                    }
                }
                // Hostile RNG returning an out-of-range draw fails cleanly without panicking.
                assert_eq!(
                    blocks_to_grow.sample(game, &mut HostileRng),
                    Err(DatapackValueError::EvaluationFailed)
                );

                // 2. BlockStateProvider agrees with sunrise-world's evaluator on the same JSON and seed.
                let resolved_tags =
                    resolve_block_tags(game.common.pack.assets.registry, &game.items().block_tags);
                let bindings = StateProviderBindings::new(
                    game.common.pack.assets.registry,
                    integer_properties(game.common.pack.assets.registry),
                    &resolved_tags,
                )
                .unwrap()
                .with_bound_properties(game.common.res.behaviours().bound_physical_properties());
                let table = game.active_datapack_vanilla_values().unwrap();
                for (handle, json) in [
                    (vegetation, cactus_provider_json),
                    (ground_state, podzol_provider_json),
                ] {
                    let direct = BlockStateProvider::from_json(json, &bindings).unwrap();
                    let mut mod_rng = LegacyRandomSource::new(0x5368_5678);
                    let mut engine_rng = LegacyRandom::new(0x5368_5678);
                    for _ in 0..16 {
                        let got = handle.state(game, level, &mut mod_rng, [0, 65, 0]).unwrap();
                        let mut view = game.datapack_read_world(level, table).unwrap();
                        let expected = direct
                            .get_state(&mut view, &mut engine_rng, [0, 65, 0])
                            .unwrap()
                            .try_into_api()
                            .unwrap();
                        assert_eq!(got, expected);
                    }
                }

                // 3. BlockPredicate agrees with sunrise-world's evaluator on live world positions.
                let direct_pred =
                    BlockPredicate::from_json_with_blocks(air_above_json, &bindings).unwrap();
                for (pos, expect_true) in [([0, 64, 0], true), ([0, 65, 0], false)] {
                    let got = predicate.test(game, level, pos).unwrap();
                    let mut view = game.datapack_read_world(level, table).unwrap();
                    let expected = direct_pred.test(&mut view, pos).unwrap();
                    assert_eq!(got, expected);
                    assert_eq!(got, expect_true);
                }

                // 4. HolderSet membership for list, custom tag, and single block.
                let key = |game: &sunrise_gameplay::game::ServerGame<'_>, id: &str| {
                    <dyn mod_api::Host>::block_key(game, id).unwrap()
                };
                assert_eq!(
                    spread_sources.contains(game, key(game, "minecraft:grass_block")),
                    Ok(true)
                );
                assert_eq!(
                    spread_sources.contains(game, key(game, "minecraft:mycelium")),
                    Ok(true)
                );
                assert_eq!(
                    spread_sources.contains(game, key(game, "minecraft:stone")),
                    Ok(false)
                );
                assert_eq!(
                    tag_targets.contains(game, key(game, "minecraft:dirt")),
                    Ok(true)
                );
                assert_eq!(
                    tag_targets.contains(game, key(game, "minecraft:sand")),
                    Ok(true)
                );
                assert_eq!(
                    tag_targets.contains(game, key(game, "minecraft:gravel")),
                    Ok(false)
                );
                assert_eq!(
                    single_target.contains(game, key(game, "minecraft:fern")),
                    Ok(true)
                );
                assert_eq!(
                    single_target.contains(game, key(game, "minecraft:stone")),
                    Ok(false)
                );

                // 5. Invalid vanilla value on reload refuses with the field path and keeps gen 1 live.
                write(
                    &rule_file,
                    r##"{
                        "blocks_to_grow": {"type":"minecraft:uniform","min_inclusive":0,"max_inclusive":999},
                        "increase": 1,
                        "vegetation": {"id":"minecraft:sugar_cane","properties":{"age":"0"}},
                        "ground_state": {"id":"minecraft:fern"},
                        "predicate": {"type":"minecraft:true"},
                        "spread_sources": "minecraft:dirt",
                        "tag_targets": "#ubm_test:bonemeal_targets",
                        "single_target": "minecraft:fern",
                        "variant": {"type":"ubm_test:random_tick"}
                    }"##,
                );
                let features = game.common.level.enabled_features.clone();
                let err = game
                    .reload_datapacks(true, &features)
                    .err()
                    .expect("out-of-bounds int provider must refuse reload")
                    .to_string();
                assert!(err.contains("blocks_to_grow"), "{err}");
                assert_eq!(
                    tag_targets.contains(game, key(game, "minecraft:dirt")),
                    Ok(true)
                );

                // 6. Successful /reload swaps generation: old handles error as Stale, new handles read new data.
                write(&tag_file, r#"{"values":["minecraft:gravel"]}"#);
                write(
                    &rule_file,
                    r##"{
                        "blocks_to_grow": 5,
                        "increase": 2,
                        "vegetation": {"id":"minecraft:sugar_cane","properties":{"age":"0"}},
                        "ground_state": {"id":"minecraft:fern"},
                        "predicate": {"type":"minecraft:solid"},
                        "spread_sources": "minecraft:stone",
                        "tag_targets": "#ubm_test:bonemeal_targets",
                        "single_target": "minecraft:cactus",
                        "variant": {"type":"ubm_test:random_tick","growth_attempts":4}
                    }"##,
                );
                assert!(game.run_command("reload"));
                let mut rng = LegacyRandomSource::new(1);
                assert_eq!(
                    blocks_to_grow.sample(game, &mut rng),
                    Err(DatapackValueError::Stale)
                );
                assert_eq!(
                    vegetation.state(game, level, &mut rng, [0, 64, 0]),
                    Err(DatapackValueError::Stale)
                );
                assert_eq!(
                    predicate.test(game, level, [0, 64, 0]),
                    Err(DatapackValueError::Stale)
                );
                assert_eq!(
                    tag_targets.contains(game, key(game, "minecraft:gravel")),
                    Err(DatapackValueError::Stale)
                );

                let mut new_int = None;
                let mut new_state = None;
                let mut new_pred = None;
                let mut new_tag = None;
                level
                    .datapack_read(
                        game,
                        "ubm_test:bone_meal_rule",
                        "ubm_test:cactus",
                        &mut |entry| {
                            new_int = entry
                                .get("blocks_to_grow")
                                .and_then(DatapackValue::as_int_provider);
                            new_state = entry
                                .get("vegetation")
                                .and_then(DatapackValue::as_block_state_provider);
                            new_pred = entry
                                .get("predicate")
                                .and_then(DatapackValue::as_block_predicate);
                            new_tag = entry
                                .get("tag_targets")
                                .and_then(DatapackValue::as_holder_set);
                        },
                    )
                    .unwrap();
                assert_eq!(new_int.unwrap().sample(game, &mut rng), Ok(5));
                let sugar_cane = new_state
                    .unwrap()
                    .state(game, level, &mut rng, [0, 64, 0])
                    .unwrap();
                assert_eq!(
                    <dyn mod_api::Host>::block_state_string(game, sugar_cane).as_deref(),
                    Some("minecraft:sugar_cane[age=0]")
                );
                assert_eq!(new_pred.unwrap().test(game, level, [0, 64, 0]), Ok(true));
                assert_eq!(new_pred.unwrap().test(game, level, [0, 65, 0]), Ok(false));
                assert_eq!(
                    new_tag
                        .unwrap()
                        .contains(game, key(game, "minecraft:gravel")),
                    Ok(true)
                );
                assert_eq!(
                    new_tag.unwrap().contains(game, key(game, "minecraft:dirt")),
                    Ok(false)
                );

                // 7. /reload with native mods defers datapack snapshots until after UpdateTags
                //    (run_native_advancement_grants), so a connected client rebinds a newly
                //    added block tag referenced by a synced HolderSet cleanly.
                let offer = datapack_net::offer().unwrap().unwrap();
                let rx = sunrise_gameplay::game::server_side::lan::world_spawn_sync_tests::host_for(
                    game,
                );
                {
                    let host = game.server.lan.as_mut().unwrap();
                    host.hub.mods_loaded = true;
                    host.hub.guests.update(0, |guest| {
                        guest.brand = "sunrise".into();
                        guest.sender.datapack_offer = Some(offer.clone().into());
                    });
                }
                let drain_guest_packets = || {
                    rx.try_iter()
                        .flat_map(|out| match out {
                            sunrise_sim::lan::Out::Packet(body) => vec![body],
                            sunrise_sim::lan::Out::Packets(bodies) => bodies,
                            sunrise_sim::lan::Out::Disconnect(reason) => {
                                panic!("unexpected guest disconnect: {reason:?}")
                            }
                        })
                        .map(|body| {
                            let mut reader = sunrise_protocol::buf::Reader::new(&body);
                            let id = reader.varint().unwrap();
                            sunrise_protocol::packets::play::Clientbound::decode(id, reader.rest())
                                .unwrap()
                        })
                        .collect::<Vec<_>>()
                };
                let _ = drain_guest_packets();

                let added_tag_file = pack.join("data/ubm_test/tags/block/reload_added.json");
                write(&added_tag_file, r#"{"values":["minecraft:clay"]}"#);
                write(
                    &rule_file,
                    r##"{
                        "blocks_to_grow": 5,
                        "increase": 2,
                        "vegetation": {"id":"minecraft:sugar_cane","properties":{"age":"0"}},
                        "ground_state": {"id":"minecraft:fern"},
                        "predicate": {"type":"minecraft:solid"},
                        "spread_sources": "minecraft:stone",
                        "tag_targets": "#ubm_test:reload_added",
                        "single_target": "minecraft:cactus",
                        "variant": {"type":"ubm_test:random_tick","growth_attempts":4}
                    }"##,
                );
                let resources = game.common.options.resources.clone();
                let stack = sunrise_gameplay::game::server_side::functions::pack_stack(
                    root,
                    &resources,
                    &game.common.pack.data_source,
                    &game.common.pack.data_changes,
                    false,
                    &[],
                    &[],
                );
                let prepared = game.prepare_world_reload(&stack, &resources).unwrap();
                game.install_world_tables(prepared);
                assert!(
                    game.server.lan.as_ref().unwrap().hub.native_reload_pending,
                    "native mods must defer finish_reload_resources"
                );
                let before_finish = drain_guest_packets();
                assert!(
                    !before_finish.iter().any(|packet| matches!(
                        packet,
                        sunrise_protocol::packets::play::Clientbound::UpdateTags(_)
                    )),
                    "UpdateTags must wait for finish_reload_resources"
                );
                assert!(
                    !before_finish.iter().any(|packet| matches!(
                        packet,
                        sunrise_protocol::packets::play::Clientbound::CustomPayload(p)
                            if p.channel == datapack_net::CHANNEL
                    )),
                    "datapack snapshot must not precede UpdateTags during deferred reload"
                );

                game.run_native_advancement_grants();
                let reload_packets = drain_guest_packets();
                let tags_idx = reload_packets
                    .iter()
                    .position(|packet| {
                        matches!(
                            packet,
                            sunrise_protocol::packets::play::Clientbound::UpdateTags(_)
                        )
                    })
                    .expect("UpdateTags must be sent on finish_reload_resources");
                let snapshot_idx = reload_packets
                    .iter()
                    .position(|packet| {
                        matches!(
                            packet,
                            sunrise_protocol::packets::play::Clientbound::CustomPayload(p)
                                if p.channel == datapack_net::CHANNEL
                        )
                    })
                    .expect("datapack snapshot must be sent after finish_reload_resources");
                assert!(
                    tags_idx < snapshot_idx,
                    "UpdateTags ({tags_idx}) must arrive before datapack snapshot ({snapshot_idx})"
                );
                // 7b. UnobstructedPredicate checks non-mob blocksBuilding entities, LAN/dedicated
                //     players, and parked dimensions lazily.
                let unobstructed_file =
                    pack.join("data/ubm_test/ubm_test/bone_meal_rule/unobstructed.json");
                write(
                    &unobstructed_file,
                    r##"{
                        "blocks_to_grow": 1,
                        "increase": 1,
                        "vegetation": {"id":"minecraft:stone"},
                        "ground_state": {"id":"minecraft:stone"},
                        "predicate": {"type":"minecraft:unobstructed"},
                        "spread_sources": "minecraft:stone",
                        "tag_targets": "#ubm_test:reload_added",
                        "single_target": "minecraft:cactus",
                        "variant": {"type":"ubm_test:random_tick"}
                    }"##,
                );
                assert!(game.run_command("reload"));
                let mut unobstructed = None;
                level
                    .datapack_read(
                        game,
                        "ubm_test:bone_meal_rule",
                        "ubm_test:unobstructed",
                        &mut |entry| {
                            unobstructed = entry
                                .get("predicate")
                                .and_then(DatapackValue::as_block_predicate);
                        },
                    )
                    .unwrap();
                let unobstructed = unobstructed.unwrap();
                assert_eq!(unobstructed.test(game, level, [10, 65, 10]), Ok(true));
                // Non-marker armor stand blocks building; marker armor stand does not.
                game.common
                    .armor_stands
                    .push(sunrise_sim::entities::ArmorStand::new(
                        9001,
                        Vec3::new(10.5, 65.0, 10.5),
                        0.0,
                    ));
                assert_eq!(unobstructed.test(game, level, [10, 65, 10]), Ok(false));
                let mut marker =
                    sunrise_sim::entities::ArmorStand::new(9002, Vec3::new(12.5, 65.0, 12.5), 0.0);
                marker.marker = true;
                game.common.armor_stands.push(marker);
                assert_eq!(unobstructed.test(game, level, [12, 65, 12]), Ok(true));
                // LAN guest blocks building even when self.player is None (dedicated server).
                let saved_player = game.common.player.take();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.dimension = game.common.dim.id().to_owned();
                        guest.loaded = true;
                        guest.dead = false;
                        guest.position = [14.5, 65.0, 14.5];
                        guest.game_mode = 0;
                    });
                assert_eq!(unobstructed.test(game, level, [14, 65, 14]), Ok(false));
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.game_mode = 3;
                    });
                assert_eq!(unobstructed.test(game, level, [14, 65, 14]), Ok(true));
                game.common.player = saved_player;
                // Parked server dimension entities are checked lazily too.
                let back = game.common.dim;
                game.enter_level(Dim::Nether).unwrap();
                game.common
                    .armor_stands
                    .push(sunrise_sim::entities::ArmorStand::new(
                        9003,
                        Vec3::new(16.5, 65.0, 16.5),
                        0.0,
                    ));
                game.enter_level(back).unwrap();
                let parked_nether = LevelRef::new(level.epoch(), Dim::Nether.key());
                assert_eq!(
                    unobstructed.test(game, parked_nether, [16, 65, 16]),
                    Ok(false)
                );
                assert_eq!(
                    unobstructed.test(game, parked_nether, [18, 65, 18]),
                    Ok(true)
                );

                captured = Some((offer, reload_packets, stone, air));
            });

            let (offer, reload_packets, stone, air) = captured.unwrap();

            // 8. Client sync round-trip: replaying the reload's wire packets in order
            //    installs the newly added block tag before binding the snapshot.
            with_client_game(|game| {
                let mut rng = LegacyRandomSource::new(1);
                let key = |game: &sunrise_client_gameplay::game::ClientGame<'_>, id: &str| {
                    <dyn mod_api::Host>::block_key(game, id).unwrap()
                };
                let _server =
                    crate::game::client_side::remote_client::tests::connect_menu_test(game);
                game.drain_test_remote_join();
                {
                    let remote = game.client.remote.as_mut().unwrap();
                    remote.logged_in = true;
                    remote.entity_id = 7;
                    remote.profile = 42;
                }
                game.enter_remote_dimension(
                    &sunrise_protocol::packets::play::SpawnInfo {
                        dimension_type: 0,
                        dimension: "minecraft:overworld".into(),
                        seed: 0,
                        game_type: 1,
                        previous_game_type: None,
                        debug: false,
                        flat: true,
                        last_death: None,
                        portal_cooldown: 0,
                        sea_level: 63,
                    },
                    RemoteDimensionEntry::Login,
                    sunrise_client::level_loading::Reason::Other,
                );
                game.common
                    .world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                game.common.world.set(0, 64, 0, stone).unwrap();
                game.common.world.set(0, 65, 0, air).unwrap();
                for packet in reload_packets {
                    if matches!(
                        packet,
                        sunrise_protocol::packets::play::Clientbound::UpdateTags(_)
                            | sunrise_protocol::packets::play::Clientbound::CustomPayload(_)
                    ) {
                        game.apply_packet(packet).unwrap();
                    }
                }
                let client_level = game.level("minecraft:overworld").unwrap();
                let mut c_int = None;
                let mut c_state = None;
                let mut c_pred = None;
                let mut c_tag = None;
                client_level
                    .datapack_read(
                        game,
                        "ubm_test:bone_meal_rule",
                        "ubm_test:cactus",
                        &mut |entry| {
                            c_int = entry
                                .get("blocks_to_grow")
                                .and_then(DatapackValue::as_int_provider);
                            c_state = entry
                                .get("vegetation")
                                .and_then(DatapackValue::as_block_state_provider);
                            c_pred = entry
                                .get("predicate")
                                .and_then(DatapackValue::as_block_predicate);
                            c_tag = entry
                                .get("tag_targets")
                                .and_then(DatapackValue::as_holder_set);
                        },
                    )
                    .unwrap();
                assert_eq!(c_int.unwrap().sample(game, &mut rng), Ok(5));
                let c_sugar = c_state
                    .unwrap()
                    .state(game, client_level, &mut rng, [0, 64, 0])
                    .unwrap();
                assert_eq!(
                    <dyn mod_api::Host>::block_state_string(game, c_sugar).as_deref(),
                    Some("minecraft:sugar_cane[age=0]")
                );
                assert_eq!(
                    c_pred.unwrap().test(game, client_level, [0, 64, 0]),
                    Ok(true)
                );
                assert_eq!(
                    c_pred.unwrap().test(game, client_level, [0, 65, 0]),
                    Ok(false)
                );
                assert_eq!(
                    c_tag.unwrap().contains(game, key(game, "minecraft:clay")),
                    Ok(true)
                );
                assert_eq!(
                    c_tag.unwrap().contains(game, key(game, "minecraft:gravel")),
                    Ok(false)
                );
                // Re-binding a new client snapshot invalidates the previous client handles.
                let current_set = game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .datapacks
                    .received()
                    .unwrap()
                    .clone();
                game.install_client_datapack_set(&current_set).unwrap();
                assert_eq!(
                    c_int.unwrap().sample(game, &mut rng),
                    Err(DatapackValueError::Stale)
                );

                // 9. Atomic client binding + clean disconnect on invalid vanilla value.
                //    Read fresh handles from the current live client generation first.
                let mut live_int = None;
                let mut live_tag = None;
                client_level
                    .datapack_read(
                        game,
                        "ubm_test:bone_meal_rule",
                        "ubm_test:cactus",
                        &mut |entry| {
                            live_int = entry
                                .get("blocks_to_grow")
                                .and_then(DatapackValue::as_int_provider);
                            live_tag = entry
                                .get("tag_targets")
                                .and_then(DatapackValue::as_holder_set);
                        },
                    )
                    .unwrap();
                let live_int = live_int.unwrap();
                let live_tag = live_tag.unwrap();

                // Build a wire-valid snapshot whose HolderSet references a tag the client lacks.
                let declarations = mod_api::__private::datapack_registry_declarations();
                let bad_files = vec![(
                    "data/ubm_test/ubm_test/bone_meal_rule/cactus.json".to_owned(),
                    vec![(
                        "bad".to_owned(),
                        br##"{
                            "blocks_to_grow": 5,
                            "increase": 2,
                            "vegetation": {"id":"minecraft:sugar_cane","properties":{"age":"0"}},
                            "ground_state": {"id":"minecraft:fern"},
                            "predicate": {"type":"minecraft:solid"},
                            "spread_sources": "minecraft:stone",
                            "tag_targets": "#ubm_test:missing_client_tag",
                            "single_target": "minecraft:cactus",
                            "variant": {"type":"ubm_test:random_tick","growth_attempts":4}
                        }"##
                        .to_vec(),
                    )],
                )];
                struct AcceptAllVanilla;
                impl VanillaValueDecoder for AcceptAllVanilla {
                    fn decode_int_provider(
                        &mut self,
                        _: &str,
                        _: i32,
                        _: i32,
                    ) -> Result<IntProviderValue, String> {
                        Ok(IntProviderValue::from_raw(99, 0))
                    }
                    fn decode_block_state_provider(
                        &mut self,
                        _: &str,
                    ) -> Result<BlockStateProviderValue, String> {
                        Ok(BlockStateProviderValue::from_raw(99, 0))
                    }
                    fn decode_block_predicate(
                        &mut self,
                        _: &str,
                    ) -> Result<BlockPredicateValue, String> {
                        Ok(BlockPredicateValue::from_raw(99, 0))
                    }
                    fn decode_holder_set(
                        &mut self,
                        _: &str,
                        _: &str,
                    ) -> Result<HolderSetValue, String> {
                        Ok(HolderSetValue::from_raw(99, 0))
                    }
                }
                let bad_candidate = mod_api::DatapackCandidate::build(
                    declarations,
                    &bad_files,
                    &mut AcceptAllVanilla,
                )
                .unwrap();
                let bad_set = mod_api::DatapackRegistrySet::from_candidate(bad_candidate);
                let offered = mod_api::DatapackSnapshot::decode_offer(&offer).unwrap();
                let bad_bytes = bad_set
                    .snapshot_for(declarations, &offered)
                    .unwrap()
                    .encode()
                    .unwrap();
                let bad_chunks: Vec<Vec<u8>> = datapack_net::chunks(&bad_bytes).unwrap().collect();
                assert_eq!(bad_chunks.len(), 1);
                let bad_packet = sunrise_protocol::packets::play::Clientbound::CustomPayload(
                    sunrise_protocol::packets::configuration::CustomPayload {
                        channel: datapack_net::CHANNEL.into(),
                        data: bad_chunks[0].clone(),
                    },
                );
                // Direct apply_packet fails with PacketError and keeps the previous generation live.
                let err = game
                    .apply_packet(bad_packet.clone())
                    .expect_err("invalid vanilla value on Play reload must fail with PacketError");
                assert!(err.is::<PacketError>(), "expected PacketError, got: {err}");
                assert_eq!(live_int.sample(game, &mut rng), Ok(5));
                assert_eq!(
                    live_tag.contains(game, key(game, "minecraft:clay")),
                    Ok(true)
                );

                // Dispatching the bad Play packet through poll_remote_events disconnects cleanly.
                game.dispatch_test_remote_event(sunrise_client::net::Event::Packet(Box::new(
                    bad_packet,
                )))
                .unwrap();
                assert!(
                    game.client.remote.is_none(),
                    "invalid Play datapack vanilla value must disconnect the client"
                );

                // Event::Joined with an invalid vanilla value also disconnects cleanly.
                let _server2 = logged_client(game);
                let mut bad_receiver = datapack_net::Receiver::default();
                for chunk in &bad_chunks {
                    bad_receiver.accept_chunk(chunk).unwrap();
                }
                let joined_registries = game.client.remote.as_ref().unwrap().registries.clone();
                game.dispatch_test_remote_event(sunrise_client::net::Event::Joined {
                    registries: joined_registries,
                    datapacks: bad_receiver,
                    name: "tester".into(),
                    uuid: 1,
                })
                .unwrap();
                assert!(
                    game.client.remote.is_none(),
                    "invalid Joined datapack vanilla value must disconnect the client"
                );
                // 10. Client-side named BlockStateProvider holders come from synced
                //     remote.registries["minecraft:worldgen/block_state_provider"] (both
                //     inline NBT tags and known-pack None fallback to built-in JSON).
                let _server3 = logged_client(game);
                game.enter_remote_dimension(
                    &sunrise_protocol::packets::play::SpawnInfo {
                        dimension_type: 0,
                        dimension: "minecraft:overworld".into(),
                        seed: 0,
                        game_type: 1,
                        previous_game_type: None,
                        debug: false,
                        flat: true,
                        last_death: None,
                        portal_cooldown: 0,
                        sea_level: 63,
                    },
                    RemoteDimensionEntry::Login,
                    sunrise_client::level_loading::Reason::Other,
                );
                let synced_tag = sunrise_sim::lan::data::json_to_nbt(&serde_json::json!({
                    "type": "minecraft:weighted",
                    "entries": [
                        { "data": "minecraft:emerald_block", "weight": 1 }
                    ]
                }));
                // Pick a real built-in provider name from the prepared resources if present.
                let builtin_provider_name = std::fs::read(
                    game.common
                        .options
                        .resources
                        .join("worldgen/state_providers.json"),
                )
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .and_then(|value| value.as_object().and_then(|map| map.keys().next().cloned()))
                .map(|key| {
                    if key.contains(':') {
                        key
                    } else {
                        format!("minecraft:{key}")
                    }
                });
                {
                    let remote = game.client.remote.as_mut().unwrap();
                    let dyn_reg = Arc::make_mut(&mut remote.registries);
                    let mut provider_entries =
                        vec![("ubm_test:synced_emerald".to_owned(), Some(synced_tag))];
                    if let Some(ref builtin_name) = builtin_provider_name {
                        provider_entries.push((builtin_name.clone(), None));
                    }
                    dyn_reg.registries.insert(
                        "minecraft:worldgen/block_state_provider".into(),
                        provider_entries,
                    );
                }
                let ground_json = builtin_provider_name
                    .as_deref()
                    .map(|name| format!(r#""{name}""#))
                    .unwrap_or_else(|| r#"{"id":"minecraft:stone"}"#.to_owned());
                let named_files = vec![(
                    "data/ubm_test/ubm_test/bone_meal_rule/named.json".to_owned(),
                    vec![(
                        "synced".to_owned(),
                        format!(
                            r##"{{
                                "blocks_to_grow": 1,
                                "increase": 1,
                                "vegetation": "ubm_test:synced_emerald",
                                "ground_state": {ground_json},
                                "predicate": {{"type":"minecraft:true"}},
                                "spread_sources": "minecraft:stone",
                                "tag_targets": "minecraft:stone",
                                "single_target": "minecraft:stone",
                                "variant": {{"type":"ubm_test:random_tick"}}
                            }}"##
                        )
                        .into_bytes(),
                    )],
                )];
                let named_candidate = mod_api::DatapackCandidate::build(
                    declarations,
                    &named_files,
                    &mut AcceptAllVanilla,
                )
                .unwrap();
                let named_set = mod_api::DatapackRegistrySet::from_candidate(named_candidate);
                let named_bytes = named_set
                    .snapshot_for(declarations, &offered)
                    .unwrap()
                    .encode()
                    .unwrap();
                for chunk in datapack_net::chunks(&named_bytes).unwrap() {
                    game.apply_packet(sunrise_protocol::packets::play::Clientbound::CustomPayload(
                        sunrise_protocol::packets::configuration::CustomPayload {
                            channel: datapack_net::CHANNEL.into(),
                            data: chunk,
                        },
                    ))
                    .unwrap();
                }
                let named_level = game.level("minecraft:overworld").unwrap();
                let mut synced_veg = None;
                let mut synced_ground = None;
                named_level
                    .datapack_read(
                        game,
                        "ubm_test:bone_meal_rule",
                        "ubm_test:named",
                        &mut |entry| {
                            synced_veg = entry
                                .get("vegetation")
                                .and_then(DatapackValue::as_block_state_provider);
                            synced_ground = entry
                                .get("ground_state")
                                .and_then(DatapackValue::as_block_state_provider);
                        },
                    )
                    .unwrap();
                let emerald = synced_veg
                    .unwrap()
                    .state(game, named_level, &mut rng, [0, 64, 0])
                    .unwrap();
                assert_eq!(
                    <dyn mod_api::Host>::block_state_string(game, emerald).as_deref(),
                    Some("minecraft:emerald_block")
                );
                assert!(
                    synced_ground
                        .unwrap()
                        .state(game, named_level, &mut rng, [0, 64, 0])
                        .is_ok(),
                    "known-pack None built-in provider holder must resolve on client"
                );
            });
        },
    );
}
