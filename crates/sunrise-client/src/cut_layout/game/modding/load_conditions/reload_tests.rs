use serde_json::json;

#[test]
fn candidate_registry_folders_do_not_choose_between_two_declared_registries() {
    let root = tempfile::tempdir().unwrap();
    let keys = ["example:spell", "minecraft:example/spell"];
    let declarations = keys.map(|key| mod_api::DatapackRegistryDeclaration {
        mod_id: "example".into(),
        key: key.into(),
        fields: vec![],
        sync: mod_api::DatapackRegistrySync::ServerOnly,
    });
    let files = sunrise_level::datapack::RegistryFiles::from([(
        "data/test/example/spell/nested/added.json".into(),
        ("file/winner".into(), b"{}".to_vec()),
    )]);
    let base = sunrise_world::conditions::LoadConditions::new([]);
    let mut view = sunrise_gameplay_common::game::load_conditions::RegistryView::default();
    let candidate =
        sunrise_gameplay_common::game::load_conditions::admit_datapack_registries_for_test(
            root.path(),
            &sunrise_world::loot::PackOverlay {
                conditions: &base,
                ..Default::default()
            },
            &mut view,
            &declarations,
            &files,
            &mut mod_api::__private::NoVanillaValueDecoder,
        )
        .unwrap();
    let conditions = view.conditions(&base);
    for registry in keys {
        assert_eq!(candidate.entries(registry).unwrap(), ["test:nested/added"]);
        assert!(conditions.allows(&json!({"neoforge:conditions":[{"type":"neoforge:registered","registry":registry,"value":"test:nested/added"}]})));
    }
    assert_eq!(conditions.check_errors(), Ok(()));
}

const STAGES: [(&str, &str, i64); 3] = [
    ("z_seed", "fire", 7),
    ("m_link", "spark", 11),
    ("a_final", "light", 19),
];

struct Registry;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Registry {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        for (registry, _, _) in STAGES {
            r.declare_datapack_registry(
                registry,
                mod_api::DatapackRegistrySync::Required,
                vec![mod_api::DatapackField::required(
                    "power",
                    mod_api::DatapackFieldKind::Int { min: 0, max: 100 },
                )],
            )
            .unwrap();
        }
    }
}
fn write(path: &std::path::Path, bytes: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
#[test]
#[ignore = "daily"]
fn reload_conditions_read_candidate_registries_and_bound_tags() {
    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let pack = root.join("datapacks/base");
    write(
        &pack.join("pack.mcmeta"),
        br#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
    );
    write(
        &root.join("datapacks.json"),
        br#"{"enabled":["vanilla","file/base"],"disabled":[]}"#,
    );
    let flag_tag = pack.join("data/reload_conditions/tags/item/flag.json");
    let nonempty_flag = br#"{"values":["minecraft:stick","minecraft:flint"]}"#;
    write(&flag_tag, nonempty_flag);
    write(
        &pack.join("data/reload_conditions/tags/reload_conditions/z_seed/hot.json"),
        br#"{"values":["reload_conditions:fire"]}"#,
    );
    let stage_conditions = [
        json!({"type":"neoforge:tag_empty","tag":"reload_conditions:flag"}),
        json!({"type":"neoforge:not","value":{"type":"neoforge:tag_empty","registry":"reload_conditions:z_seed","tag":"reload_conditions:hot"}}),
        json!({"type":"neoforge:registered","registry":"reload_conditions:m_link","value":"reload_conditions:spark"}),
    ];
    for ((registry, id, power), condition) in STAGES.into_iter().zip(stage_conditions) {
        write(
            &pack.join(format!(
                "data/reload_conditions/reload_conditions/{registry}/{id}.json"
            )),
            json!({"power":power,"neoforge:conditions":[condition]}).to_string(),
        );
    }
    let empty_tag = pack.join("data/reload_conditions/tags/item/empty.json");
    write(&empty_tag, br#"{"values":[]}"#);
    for (path, bytes, _) in sunrise_gameplay_common::game::native_tags::PACK_BINDING_CASES {
        write(
            &pack.join(format!("data/binding/tags/item/{path}.json")),
            bytes,
        );
    }
    for (name, condition) in [
        (
            "registered",
            json!({"type":"neoforge:registered","registry":"reload_conditions:a_final","value":"reload_conditions:light"}),
        ),
        (
            "empty",
            json!({"type":"neoforge:tag_empty","registry":"minecraft:item","tag":"reload_conditions:flag"}),
        ),
        (
            "absent",
            json!({"type":"neoforge:not","value":{"type":"neoforge:registered","registry":"reload_conditions:absent","value":"reload_conditions:fire"}}),
        ),
    ] {
        write(&pack.join(format!("data/reload_conditions/recipe/{name}.json")), serde_json::to_vec(&json!({
            "type":"minecraft:crafting_shapeless","ingredients":["minecraft:flint"],"result":{"id":"minecraft:stick"},"neoforge:conditions":[condition]
        })).unwrap());
    }
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "reload_conditions",
            Box::new(Registry),
        )]
        .into(),
        || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let mut request = sunrise_gameplay::game::world_loading::WorldRequest::new(
                    "conditions",
                    Some("test"),
                    Some(1),
                    true,
                    None,
                    std::sync::Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
                )
                .unwrap();
                request.save_root = root.to_owned();
                game.load_world(request).unwrap();
                let item_tags = |game: &sunrise_gameplay::game::ServerGame<'_>,
                                 tag: mod_api::TagKey| {
                    mod_api::Tags::items(game, tag.typed())
                };
                let key = |path: &str| {
                    mod_api::__private::tag_key(
                        &mod_api::Identifier::parse(&format!("reload_conditions:{path}")).unwrap(),
                    )
                    .unwrap()
                };
                for (path, _, is_bound) in
                    sunrise_gameplay_common::game::native_tags::PACK_BINDING_CASES
                {
                    let tag = mod_api::__private::tag_key(
                        &mod_api::Identifier::parse(&format!("binding:{path}")).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(
                        item_tags(game, tag),
                        if is_bound {
                            mod_api::TagMembers::Members(vec![])
                        } else {
                            mod_api::TagMembers::Missing
                        }
                    );
                }
                let empty = key("empty");
                let flag = key("flag");
                assert_eq!(item_tags(game, empty), mod_api::TagMembers::Members(vec![]));
                let flag_members = mod_api::TagMembers::Members(vec![
                    mod_api::__private::item_key("minecraft:stick").unwrap(),
                    mod_api::__private::item_key("minecraft:flint").unwrap(),
                ]);
                let answers = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                    ["registered", "empty", "absent"].map(|name| {
                        game.recipes()
                            .recipe(&format!("reload_conditions:{name}"))
                            .is_some()
                    })
                };
                let check_stages = |game: &sunrise_gameplay::game::ServerGame<'_>,
                                    admitted: bool| {
                    let level = game.level(game.dim.id()).unwrap();
                    for (registry, entry, power) in STAGES {
                        let registry = format!("reload_conditions:{registry}");
                        let entry = format!("reload_conditions:{entry}");
                        let mut ids = Vec::new();
                        assert_eq!(
                            level.visit_datapack_entries(game, &registry, &mut |id| {
                                ids.push(id.to_owned());
                                std::ops::ControlFlow::Continue(())
                            }),
                            Ok(std::ops::ControlFlow::Continue(()))
                        );
                        assert_eq!(
                            ids,
                            if admitted {
                                vec![entry.clone()]
                            } else {
                                Vec::new()
                            }
                        );
                        let mut value = None;
                        let read = level.datapack_read(game, &registry, &entry, &mut |decoded| {
                            value = decoded.get("power").cloned();
                        });
                        if admitted {
                            assert_eq!(read, Ok(()));
                            assert_eq!(value, Some(mod_api::DatapackValue::Int(power)));
                        } else {
                            assert_eq!(read, Err(mod_api::DatapackReadError::MissingEntry));
                            assert_eq!(value, None);
                        }
                    }
                };
                assert_eq!(item_tags(game, flag), flag_members);
                check_stages(game, false);
                assert_eq!(answers(game), [false, false, true]);

                write(&flag_tag, br#"{"values":[]}"#);
                assert!(game.run_command("reload"));
                assert_eq!(item_tags(game, flag), mod_api::TagMembers::Members(vec![]));
                check_stages(game, true);
                assert_eq!(answers(game), [true, true, true]);

                std::fs::remove_file(empty_tag).unwrap();
                write(&flag_tag, nonempty_flag);
                assert!(game.run_command("reload"));
                assert_eq!(item_tags(game, flag), flag_members);
                check_stages(game, false);
                assert_eq!(answers(game), [false, false, true]);
                assert_eq!(item_tags(game, empty), mod_api::TagMembers::Missing);
                assert_eq!(
                    game.mods
                        .load_condition_candidate
                        .as_ref()
                        .unwrap()
                        .check_errors(),
                    Ok(())
                );
            });
        },
    );
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
