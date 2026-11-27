//! Real server/client Game owners, world publication and negotiated client registry limits.
#![cfg(feature = "client")]
use mod_api::{
    DatapackField, DatapackFieldKind, DatapackRegistrySync, Hooks, ModEntry, Registrar,
    RegistrationContext,
};
use std::{ops::ControlFlow, path::Path, sync::Arc};
use sunrise_gameplay_common::game::native_registry_entries::NativeRegistryReads;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

struct RegistryMod;
#[mod_api::__subscriptions]
impl Hooks for RegistryMod {
    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        for (registry, sync) in [
            ("spell", DatapackRegistrySync::Required),
            ("secret", DatapackRegistrySync::ServerOnly),
        ] {
            registrar
                .declare_datapack_registry(
                    registry,
                    sync,
                    vec![DatapackField::required(
                        "power",
                        DatapackFieldKind::Int { min: 0, max: 100 },
                    )],
                )
                .unwrap();
        }
    }
}
fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
pub fn request(root: &Path) -> sunrise_gameplay::game::world_loading::WorldRequest {
    let mut request = sunrise_gameplay::game::world_loading::WorldRequest::new(
        "generic registries",
        Some("generic registries"),
        Some(1),
        true,
        None,
        Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
    )
    .unwrap();
    request.save_root = root.to_owned();
    request
}
fn host<'g, 'a, G: sunrise_gameplay_common::game::SideGame<'g> + mod_api::Host>(
    game: &'a G,
) -> &'a dyn mod_api::Host {
    game
}
fn id(value: &str) -> mod_api::Identifier {
    mod_api::Identifier::parse(value).unwrap()
}

fn registry_lists(host: &dyn mod_api::Host, registry: &str, expected: &str) -> bool {
    host.visit_registry_entries(registry, &mut |id| {
        if id == expected {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })
    .unwrap_or_else(|error| panic!("{registry}: {error:?}"))
    .is_break()
}

// Feed the same server-built snapshot to both Host implementations.
fn check_rejected_sync_tags<'g, G: sunrise_gameplay_common::game::SideGame<'g> + mod_api::Host>(
    game: &G,
) {
    for (registry, name) in [
        ("minecraft:enchantment", "minecraft:sharpness"),
        ("minecraft:entity_type", "minecraft:zombie"),
    ] {
        let entry = host(game).registry_entry(registry, name).unwrap();
        assert_eq!(
            host(game).registry_entry_in_tag(&entry, "registry_entry_test:bound"),
            Some(true)
        );
        assert_eq!(
            host(game).registry_entry_in_tag(&entry, "registry_entry_test:rejected"),
            Some(false)
        );
    }
}

#[test]
#[ignore = "daily"]
fn generic_entries_follow_real_server_tables_and_atomic_reload() {
    // The lazy wire cache is process-wide; another world must not replace this world's snapshot.
    if run_fresh_cases(
        "cut_layout::game::native_registry_entries_tests::generic_entries_follow_real_server_tables_and_atomic_reload",
        "SUNRISE_REGISTRY_ENTRIES_RELOAD_CHILD",
        &["reload"],
    ) {
        return;
    }
    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let pack = scratch.path().join("datapacks/entries");
    write(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
    );
    write(
        &scratch.path().join("datapacks.json"),
        r#"{"enabled":["vanilla","file/entries"],"disabled":[]}"#,
    );
    let fire = pack.join("data/registry_entry_test/registry_entry_test/spell/fire.json");
    let tag = pack.join("data/registry_entry_test/tags/registry_entry_test/spell/hot.json");
    write(&fire, r#"{"power":7}"#);
    write(
        &tag,
        r#"{"neoforge:conditions":[{"type":"neoforge:never"}],"values":["registry_entry_test:fire"]}"#,
    );
    write(
        &pack.join("data/registry_entry_test/registry_entry_test/secret/shadow.json"),
        r#"{"power":3}"#,
    );
    write(
        &pack.join("data/registry_entry_test/registry_entry_test/spell/conditioned.json"),
        r#"{"neoforge:conditions":[{"type":"neoforge:never"}],"power":"not an integer"}"#,
    );
    write(
        &pack.join("data/registry_entry_test/not_a_registry/fake.json"),
        "{}",
    );
    write(
        &pack.join("data/registry_entry_test/worldgen/noise_settings/broken.json"),
        "{}",
    );
    // Listing the registry decodes every unused file: a range past 32-bit heights is refused.
    write(
        &pack.join("data/registry_entry_test/worldgen/noise_settings/overflow.json"),
        r#"{"noise":{"min_y":2147483647,"height":1}}"#,
    );
    write(
        &pack.join("data/registry_entry_test/tags/worldgen/structure/towns.json"),
        r#"{"values":["minecraft:village_plains"]}"#,
    );
    write(
        &pack.join("data/registry_entry_test/tags/worldgen/structure/rejected.json"),
        r#"{"values":["minecraft:village_plains","registry_entry_test:missing"]}"#,
    );
    write(
        &pack.join("data/registry_entry_test/tags/mob_effect/rejected.json"),
        r#"{"values":["minecraft:speed","registry_entry_test:missing"]}"#,
    );
    for (kind, member) in [
        ("enchantment", "minecraft:sharpness"),
        ("entity_type", "minecraft:zombie"),
    ] {
        for (tag, values) in [
            ("bound", vec![member]),
            ("rejected", vec![member, "#registry_entry_test:missing"]),
        ] {
            write(
                &pack.join(format!("data/registry_entry_test/tags/{kind}/{tag}.json")),
                serde_json::json!({"values":values}).to_string(),
            );
        }
    }
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_entry_test", Box::new(RegistryMod))].into(),
        || {
            let mut wire = None;
            with_game(|game| {
                game.load_world(request(scratch.path())).unwrap();
                check_rejected_sync_tags(game);
                wire = game.registry_sync();
                let step = host(game)
                    .registry_entry("minecraft:game_event", "minecraft:step")
                    .unwrap();
                assert_eq!(
                    host(game).registry_entry_in_tag(&step, "minecraft:vibrations"),
                    Some(true)
                );
                assert_eq!(
                    host(game).registry_entry_in_tag(&step, "registry_entry_test:absent"),
                    Some(false)
                );
                let speed = host(game)
                    .registry_entry("minecraft:mob_effect", "minecraft:speed")
                    .unwrap();
                assert_eq!(
                    host(game).registry_entry_in_tag(&speed, "registry_entry_test:rejected"),
                    Some(false)
                );
                assert_eq!(
                    host(game).registry_entry("minecraft:game_event", "minecraft:nope"),
                    None
                );
                assert_eq!(
                    host(game).registry_entry("minecraft:nope", "minecraft:step"),
                    None
                );
                assert_eq!(
                    host(game).visit_registry_entries(
                        "registry_entry_test:not_a_registry",
                        &mut |_| panic!("unavailable registry visited")
                    ),
                    Err(mod_api::RegistryReadError::UnknownRegistry)
                );
                assert_eq!(
                    host(game).visit_registry_entries("minecraft:biome", &mut |_| panic!(
                        "unavailable registry visited"
                    )),
                    Err(mod_api::RegistryReadError::UnknownRegistry),
                    "no synthetic registry aliases"
                );
                assert_eq!(
                    host(game).registry_entry(
                        "minecraft:worldgen/noise_settings",
                        "registry_entry_test:broken"
                    ),
                    None,
                    "an unused resource is not an accepted noise-settings definition",
                );
                assert!(!registry_lists(
                    host(game),
                    "minecraft:worldgen/noise_settings",
                    "registry_entry_test:broken"
                ));
                assert!(!registry_lists(
                    host(game),
                    "minecraft:worldgen/noise_settings",
                    "registry_entry_test:overflow"
                ));
                let village = host(game)
                    .registry_entry("minecraft:worldgen/structure", "minecraft:village_plains")
                    .unwrap();
                assert_eq!(
                    host(game).registry_entry_in_tag(&village, "registry_entry_test:towns"),
                    Some(true),
                    "an accepted structure in a loaded structure tag"
                );
                assert_eq!(
                    host(game).registry_entry_in_tag(&village, "registry_entry_test:rejected"),
                    Some(false),
                    "a required missing holder rejects the whole tag"
                );
                assert_eq!(
                    host(game).registry_entry_in_tag(&village, "registry_entry_test:absent"),
                    Some(false)
                );
                let stone = host(game)
                    .registry_entry("minecraft:block", "minecraft:stone")
                    .unwrap();
                assert_eq!(
                    stone.as_block_key(host(game)),
                    game.block_key("minecraft:stone")
                );
                assert_eq!(stone.as_item_key(host(game)), None);
                for (registry, entry) in [
                    ("minecraft:damage_type", "minecraft:fall"),
                    ("minecraft:villager_type", "minecraft:plains"),
                    ("minecraft:painting_variant", "minecraft:alban"),
                    ("minecraft:instrument", "minecraft:ponder_goat_horn"),
                    ("minecraft:worldgen/noise", "minecraft:aquifer_barrier"),
                    ("minecraft:trade_set", "minecraft:armorer/level_1"),
                    (
                        "minecraft:context_int_provider",
                        "minecraft:cooking/normal_burn_time_reduction_factor",
                    ),
                ] {
                    assert!(
                        host(game).registry_entry(registry, entry).is_some(),
                        "{registry} / {entry}"
                    );
                    assert!(registry_lists(host(game), registry, entry), "{registry}");
                }
                assert_eq!(
                    {
                        let mut ids = Vec::new();
                        assert_eq!(
                            host(game).visit_registry_entries(
                                "registry_entry_test:spell",
                                &mut |id| {
                                    ids.push(id.to_owned());
                                    ControlFlow::Continue(())
                                }
                            ),
                            Ok(ControlFlow::Continue(()))
                        );
                        ids
                    },
                    ["registry_entry_test:fire"]
                );
                let held = host(game)
                    .registry_entry("registry_entry_test:spell", "registry_entry_test:fire")
                    .unwrap();
                assert_eq!(
                    host(game).registry_entry_in_tag(&held, "registry_entry_test:hot"),
                    Some(true)
                );
                assert!(
                    host(game)
                        .registry_entry("registry_entry_test:secret", "registry_entry_test:shadow")
                        .is_some()
                );
                write(&fire, "{bad json");
                assert!(!game.run_command("reload"));
                assert_eq!(
                    host(game).registry_entry_in_tag(&held, "registry_entry_test:hot"),
                    Some(true),
                    "failed candidate keeps the published entries and tags"
                );
                std::fs::remove_file(&fire).unwrap();
                write(
                    &pack.join("data/addon/registry_entry_test/spell/ice.json"),
                    r#"{"power":4}"#,
                );
                write(&tag, r#"{"replace":true,"values":["addon:ice"]}"#);
                write(
                    &pack.join("data/registry_entry_test/damage_type/late.json"),
                    r#"{"message_id":"fall","scaling":"never","exhaustion":0.0}"#,
                );
                write(
                    &pack.join("data/registry_entry_test/worldgen/noise/late.json"),
                    r#"{"base_octave":-3,"octave_count":1}"#,
                );
                assert!(game.run_command("reload"));
                assert_eq!(
                    host(game)
                        .registry_entry("registry_entry_test:spell", "registry_entry_test:fire"),
                    None
                );
                assert_eq!(
                    host(game).registry_entry_in_tag(&held, "registry_entry_test:hot"),
                    None,
                    "plain values are re-resolved after reload"
                );
                let ice = host(game)
                    .registry_entry("registry_entry_test:spell", "addon:ice")
                    .unwrap();
                assert_eq!(
                    host(game).registry_entry_in_tag(&ice, "registry_entry_test:hot"),
                    Some(true)
                );
                assert_eq!(
                    {
                        let mut ids = Vec::new();
                        assert_eq!(
                            host(game).visit_registry_entries(
                                "registry_entry_test:spell",
                                &mut |id| {
                                    ids.push(id.to_owned());
                                    ControlFlow::Continue(())
                                }
                            ),
                            Ok(ControlFlow::Continue(()))
                        );
                        ids
                    },
                    ["addon:ice"]
                );
                assert_eq!(
                    host(game).registry_entry("minecraft:damage_type", "registry_entry_test:late"),
                    None,
                    "frozen registry is not a raw reload-file inventory"
                );
                assert_eq!(
                    host(game)
                        .registry_entry("minecraft:worldgen/noise", "registry_entry_test:late"),
                    None
                );
            });
            with_client_game(|game| {
                let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(client, "Generic registries").unwrap();
                let wire = wire.unwrap();
                let mut received = wire.dynamic.clone();
                received.add_tags(&wire.tags);
                game.client.remote.as_mut().unwrap().registries = Arc::new(received);
                check_rejected_sync_tags(game);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn generic_client_entries_use_only_builtin_synced_and_negotiated_tables() {
    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_entry_test", Box::new(RegistryMod))].into(),
        || {
            with_client_game(|game| {
                use sunrise_protocol::{
                    connection::Pipe,
                    packets::configuration::{RegistryTags, UpdateTags},
                };
                assert_eq!(
                    host(game).registry_entry("minecraft:game_event", "minecraft:step"),
                    None
                );
                assert_eq!(
                    host(game).visit_registry_entries("minecraft:game_event", &mut |_| {
                        panic!("visited without a registry session")
                    }),
                    Err(mod_api::RegistryReadError::Unavailable)
                );
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "Generic registries").unwrap();
                assert_eq!(
                    host(game).registry_entry("minecraft:damage_type", "minecraft:fall"),
                    None,
                    "not synchronized yet"
                );
                assert_eq!(
                    host(game).visit_registry_entries("minecraft:worldgen/noise", &mut |_| panic!(
                        "unavailable registry visited"
                    )),
                    Err(mod_api::RegistryReadError::UnknownRegistry),
                    "prepared server-only data must not leak"
                );
                for registry in [
                    "minecraft:worldgen/feature",
                    "minecraft:worldgen/placed_feature",
                    "minecraft:worldgen/block_state_provider",
                ] {
                    assert_eq!(
                        host(game).visit_registry_entries(registry, &mut |_| panic!(
                            "unavailable registry visited"
                        )),
                        Err(mod_api::RegistryReadError::UnknownRegistry)
                    );
                    assert_eq!(host(game).registry_entry(registry, "minecraft:oak"), None);
                }
                let sync = sunrise_gameplay::game::lan::sync::sync_data(
                    &game.options.resources,
                    &game.res.core.item_data,
                    game.items(),
                    &game.pack_files(),
                )
                .unwrap();
                let mut received = sync.dynamic.clone();
                received.add_tags(&sync.tags);
                game.client.remote.as_mut().unwrap().registries = Arc::new(received);
                let step = host(game)
                    .registry_entry("minecraft:game_event", "minecraft:step")
                    .unwrap();
                assert_eq!(
                    host(game).registry_entry_in_tag(&step, "minecraft:vibrations"),
                    Some(true)
                );
                let fall = host(game)
                    .registry_entry("minecraft:damage_type", "minecraft:fall")
                    .unwrap();
                assert!(registry_lists(
                    host(game),
                    "minecraft:damage_type",
                    "minecraft:fall"
                ));
                let stone = host(game)
                    .registry_entry("minecraft:block", "minecraft:stone")
                    .unwrap();
                assert_eq!(
                    stone.as_block_key(host(game)),
                    game.block_key("minecraft:stone")
                );
                assert!(
                    host(game)
                        .registry_entry("minecraft:painting_variant", "minecraft:alban")
                        .is_some()
                );
                assert_eq!(
                    host(game).registry_entry("minecraft:game_event", "minecraft:nope"),
                    None
                );
                assert_eq!(
                    host(game)
                        .visit_registry_entries("minecraft:context_int_provider", &mut |_| panic!(
                            "unavailable registry visited"
                        )),
                    Err(mod_api::RegistryReadError::UnknownRegistry)
                );
                let declarations = mod_api::__private::datapack_registry_declarations();
                let mut candidate = mod_api::DatapackCandidate::build(
                    declarations,
                    &[
                        (
                            "data/addon/registry_entry_test/spell/remote.json".into(),
                            vec![("fixture".into(), br#"{"power":8}"#.to_vec())],
                        ),
                        (
                            "data/addon/registry_entry_test/secret/hidden.json".into(),
                            vec![("fixture".into(), br#"{"power":9}"#.to_vec())],
                        ),
                    ],
                    &mut mod_api::__private::NoVanillaValueDecoder,
                )
                .unwrap();
                candidate
                    .bind_tags(
                        "registry_entry_test:spell",
                        [("addon:hot".into(), vec!["addon:remote".into()])].into(),
                    )
                    .unwrap();
                let server_set = mod_api::DatapackRegistrySet::from_candidate(candidate);
                // A same-process server snapshot is deliberately present on the client Game too.
                game.mods.datapack_registries = Some(Arc::new(server_set.clone()));
                sunrise_sim::datapack_net::serve(Some(Arc::new(server_set)));
                let offer = sunrise_sim::datapack_net::offer()
                    .unwrap()
                    .expect("synchronized registry offer required");
                let sunrise_sim::datapack_net::Step::Send(bytes) =
                    sunrise_sim::datapack_net::server_step(true, Some(&offer))
                else {
                    panic!("expected negotiated snapshot");
                };
                let receiver = &mut game.client.remote.as_mut().unwrap().datapacks;
                for chunk in sunrise_sim::datapack_net::chunks(&bytes).unwrap() {
                    receiver.accept_chunk(&chunk).unwrap();
                }
                let remote = host(game)
                    .registry_entry("registry_entry_test:spell", "addon:remote")
                    .unwrap();
                assert_eq!(
                    host(game).registry_entry_in_tag(&remote, "addon:hot"),
                    Some(true)
                );
                assert_eq!(
                    host(game)
                        .visit_registry_entries("registry_entry_test:secret", &mut |_| panic!(
                            "unavailable registry visited"
                        )),
                    Err(mod_api::RegistryReadError::UnknownRegistry)
                );
                assert_eq!(
                    host(game).registry_entry("registry_entry_test:secret", "addon:hidden"),
                    None
                );
                let mut replacement = game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .registries
                    .as_ref()
                    .clone();
                replacement.registries.remove("minecraft:damage_type");
                replacement
                    .registries
                    .insert("test:empty".into(), Vec::new());
                replacement.registries.insert(
                    "test:protocol_order".into(),
                    vec![("test:z".into(), None), ("test:a".into(), None)],
                );
                replacement.add_tags(&UpdateTags {
                    registries: vec![RegistryTags {
                        registry: "minecraft:game_event".into(),
                        tags: vec![],
                    }],
                });
                let remote_state = game.client.remote.as_mut().unwrap();
                remote_state.registries = Arc::new(replacement);
                Arc::make_mut(&mut remote_state.builtin).registries.insert(
                    "test:sparse".into(),
                    sunrise_protocol::registry::Ids::from_names(vec![
                        String::new(),
                        "test:z".into(),
                        String::new(),
                        "test:a".into(),
                    ]),
                );
                let h = host(game);
                assert_eq!(
                    h.visit_registry_entries("test:empty", &mut |_| panic!(
                        "empty registry visited"
                    )),
                    Ok(ControlFlow::Continue(()))
                );
                for registry in ["test:protocol_order", "test:sparse"] {
                    let mut names = Vec::new();
                    assert_eq!(
                        h.visit_registry_entries(registry, &mut |id| {
                            names.push(id.to_owned());
                            ControlFlow::Continue(())
                        }),
                        Ok(ControlFlow::Continue(()))
                    );
                    assert_eq!(names, ["test:z", "test:a"]);
                }
                assert_eq!(h.registry_entry_in_tag(&fall, "minecraft:is_fall"), None);
                assert_eq!(
                    h.registry_entry_in_tag(&step, "minecraft:vibrations"),
                    Some(false)
                );
                game.client.remote.as_mut().unwrap().datapacks = Default::default();
                assert_eq!(host(game).registry_entry_in_tag(&remote, "addon:hot"), None);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn generic_worldgen_entries_use_published_decoded_owner_without_terrain_locator() {
    use sunrise_world::worldgen::{
        generator::GeneratorData, live::LiveFeatures, overrides::WorldgenOverrides,
    };
    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    with_game(|game| {
        game.load_world(request(scratch.path())).unwrap();
        let mut overrides = WorldgenOverrides::default();
        overrides.set(
            "features",
            "registry_entry_test:accepted",
            r#"{"type":"minecraft:no_op","config":{}}"#.into(),
        );
        overrides.set(
            "features",
            "registry_entry_test:rejected",
            r#"{"type":"registry_entry_test:unknown","config":{}}"#.into(),
        );
        // FeatureCatalog also uses this tag bundle for placed-holder expansion.
        overrides.set(
            "placed_features",
            "registry_entry_test:accepted",
            r#"{"feature":"registry_entry_test:accepted","placement":[]}"#.into(),
        );
        overrides.set(
            "placed_features",
            "registry_entry_test:rejected",
            r#"{"feature":"registry_entry_test:rejected","placement":[]}"#.into(),
        );
        overrides.set(
            "state_providers",
            "registry_entry_test:accepted",
            r#"{"type":"minecraft:simple","state":{"id":"minecraft:stone"}}"#.into(),
        );
        overrides.set(
            "state_providers",
            "registry_entry_test:rejected",
            r#"{"type":"registry_entry_test:unknown"}"#.into(),
        );
        overrides.set(
            "feature_tags",
            "registry_entry_test:accepted",
            r#"{"values":["registry_entry_test:accepted"]}"#.into(),
        );
        overrides.add_tag(
            "placed_feature_tags",
            "registry_entry_test:accepted",
            "fixture",
            r#"{"values":["registry_entry_test:accepted"]}"#.into(),
        );
        overrides.add_tag(
            "placed_feature_tags",
            "registry_entry_test:mixed",
            "fixture",
            r#"{"values":["registry_entry_test:accepted","registry_entry_test:rejected"]}"#.into(),
        );
        overrides.set("carvers", "registry_entry_test:accepted",
            r#"{"type":"minecraft:cave","probability":1.0,"y":{"absolute":0},"count":1,"thickness":1.0,"room_vertical_radius_multiplier":1.0,"horizontal_radius_multiplier":1.0,"vertical_radius_multiplier":1.0,"floor_level":-0.7}"#.into());
        overrides.set(
            "conditions",
            "registry_entry_test:accepted",
            r#"{"type":"minecraft:steep"}"#.into(),
        );
        let data = GeneratorData::load_with(
            &game.options.resources.join("worldgen"),
            game.pack.assets.registry,
            &overrides,
        )
        .unwrap();
        let features = Arc::new(LiveFeatures::new(&data).unwrap().unwrap());
        assert!(
            features
                .configured("registry_entry_test:accepted")
                .is_some()
        );
        assert!(
            features
                .configured("registry_entry_test:rejected")
                .is_none()
        );
        assert!(features.placed("registry_entry_test:accepted").is_some());
        for registry in [
            "minecraft:worldgen/feature",
            "minecraft:worldgen/placed_feature",
            "minecraft:worldgen/block_state_provider",
        ] {
            assert_eq!(
                features.registry_contains(registry, "registry_entry_test:accepted"),
                Some(true),
                "{registry}: fixture must pass the live owner's canonical decoder"
            );
        }
        for registry in [
            "minecraft:worldgen/carver",
            "minecraft:worldgen/material_condition",
        ] {
            assert_eq!(
                data.registry_contains(registry, "registry_entry_test:accepted"),
                Some(true),
                "{registry}: fixture must belong to the decoded generator owner"
            );
        }
        game.res.behaviours().set_live_features(features);
        game.end.set_locator(None);
        assert!(game.end.locator().is_none());
        for registry in [
            "minecraft:worldgen/feature",
            "minecraft:worldgen/placed_feature",
            "minecraft:worldgen/block_state_provider",
            "minecraft:worldgen/carver",
            "minecraft:worldgen/material_condition",
        ] {
            let accepted = host(game)
                .registry_entry(registry, "registry_entry_test:accepted")
                .unwrap_or_else(|| {
                    panic!("{registry}: decoded entry must not depend on a terrain locator")
                });
            assert!(
                registry_lists(host(game), registry, "registry_entry_test:accepted"),
                "{registry}"
            );
            assert!(
                !registry_lists(host(game), registry, "registry_entry_test:rejected"),
                "{registry}"
            );
            assert_eq!(
                host(game).registry_entry(registry, "registry_entry_test:rejected"),
                None
            );
            assert_eq!(
                host(game).registry_entry_in_tag(&accepted, "registry_entry_test:missing"),
                Some(false)
            );
            if matches!(
                registry,
                "minecraft:worldgen/feature" | "minecraft:worldgen/placed_feature"
            ) {
                assert_eq!(
                    host(game).registry_entry_in_tag(&accepted, "registry_entry_test:accepted"),
                    Some(true),
                    "{registry}: an admitted member of the registry's own tag"
                );
            }
            if registry == "minecraft:worldgen/placed_feature" {
                assert_eq!(
                    host(game).registry_entry_in_tag(&accepted, "registry_entry_test:mixed"),
                    Some(false),
                    "a required placed feature rejected by its codec cannot bind the tag"
                );
            }
        }
        // Replacing the published owner must retire its entries even while the old Arc lives.
        let mut replacement = WorldgenOverrides::default();
        replacement.set(
            "conditions",
            "registry_entry_test:replacement",
            r#"{"type":"minecraft:steep"}"#.into(),
        );
        let replacement = GeneratorData::load_with(
            &game.options.resources.join("worldgen"),
            game.pack.assets.registry,
            &replacement,
        )
        .unwrap();
        game.res
            .behaviours()
            .set_live_features(Arc::new(LiveFeatures::new(&replacement).unwrap().unwrap()));
        for registry in [
            "minecraft:worldgen/feature",
            "minecraft:worldgen/placed_feature",
            "minecraft:worldgen/block_state_provider",
            "minecraft:worldgen/carver",
            "minecraft:worldgen/material_condition",
        ] {
            assert_eq!(
                host(game).registry_entry(registry, "registry_entry_test:accepted"),
                None
            );
            assert!(
                !registry_lists(host(game), registry, "registry_entry_test:accepted"),
                "{registry}"
            );
            let retired =
                mod_api::RegistryEntry::new(id(registry), id("registry_entry_test:accepted"));
            assert_eq!(
                host(game).registry_entry_in_tag(&retired, "registry_entry_test:accepted"),
                None
            );
        }
        let replacement_entry = host(game)
            .registry_entry(
                "minecraft:worldgen/material_condition",
                "registry_entry_test:replacement",
            )
            .unwrap();
        assert!(registry_lists(
            host(game),
            "minecraft:worldgen/material_condition",
            "registry_entry_test:replacement"
        ));
        assert_eq!(
            host(game).registry_entry_in_tag(&replacement_entry, "registry_entry_test:missing"),
            Some(false)
        );
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
