#![cfg(test)]
#[cfg(test)]
mod tests {
    #[cfg(feature = "client")]
    use mod_api::{FluidDefinition, FluidStateError};
    use mod_api::{FluidTypeProperties, HostBlockStates};
    use sunrise_gameplay_common::game::native_block_states::{
        Arc, BlockRegistry, BlockStateId, FluidState, ItemStack, Registries, StateConstraint,
        StateMatchError, block_registries, description_id, match_state_properties, parse,
        serialize,
    };

    /// Air, a stair with two properties (its default `facing=north,half=bottom`)
    /// and wheat with an integer `age`, as `parse` reads them.
    fn blocks() -> (Arc<Registries>, &'static BlockRegistry) {
        let facts = br#"{"defaults":{"minecraft:air":0,"minecraft:oak_stairs":1,"minecraft:wheat":5},"states":[
            ["air",true,false,"","minecraft:air"],
            ["s",false,false,"",{"id":"minecraft:oak_stairs","properties":{"facing":"north","half":"bottom"}}],
            ["s",false,false,"",{"id":"minecraft:oak_stairs","properties":{"facing":"north","half":"top"}}],
            ["s",false,false,"",{"id":"minecraft:oak_stairs","properties":{"facing":"east","half":"bottom"}}],
            ["s",false,false,"",{"id":"minecraft:oak_stairs","properties":{"facing":"east","half":"top"}}],
            ["w",false,false,"",{"id":"minecraft:wheat","properties":{"age":"0"}}],
            ["w",false,false,"",{"id":"minecraft:wheat","properties":{"age":"1"}}]
        ]}"#;
        let registry = Box::leak(Box::new(
            BlockRegistry::from_facts_json(&facts[..]).unwrap(),
        ));
        (block_registries(registry), registry)
    }

    #[test]
    #[ignore = "daily"]
    fn item_and_block_registry_facts_use_the_loaded_tables() {
        use mod_api::Host;
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let host: &dyn Host = game;
            let remainder = host
                .item_crafting_remainder(
                    &ItemStack::new("minecraft:water_bucket", 1)
                        .try_into_api()
                        .unwrap(),
                )
                .unwrap();
            assert_eq!(
                remainder.item(),
                mod_api::__private::item_key("minecraft:bucket").unwrap()
            );
            assert_eq!(remainder.count(), 1);
            assert!(
                host.item_crafting_remainder(
                    &ItemStack::new("minecraft:stone", 1).try_into_api().unwrap()
                )
                .is_none()
            );
            assert_eq!(
                host.block_item(mod_platform::__private::part_tests::block_key(
                    "minecraft:stone"
                ))
                .and_then(mod_api::__private::item_key_id),
                Some("minecraft:stone")
            );
            assert_eq!(
                host.block_item(mod_platform::__private::part_tests::block_key(
                    "minecraft:water"
                ))
                .and_then(mod_api::__private::item_key_id),
                Some("minecraft:air")
            );
            let item = |id| mod_api::__private::item_key(id).unwrap();
            let block = mod_platform::__private::part_tests::block_key;
            assert_eq!(
                host.item_block(item("minecraft:oak_planks")),
                Some(block("minecraft:oak_planks"))
            );
            // `Block.byItem(WHEAT_SEEDS)` is `Blocks.WHEAT`, whose item id is
            // the seeds' even though the names differ.
            assert_eq!(
                host.item_block(item("minecraft:wheat_seeds")),
                Some(block("minecraft:wheat"))
            );
            // A standing/wall item places its standing block; the wall variant
            // is the placement's, not the item's (`Block.byItem(TORCH)`).
            assert_eq!(
                host.item_block(item("minecraft:torch")),
                Some(block("minecraft:torch"))
            );
            assert_eq!(host.item_block(item("minecraft:stick")), None);
        });
    }

    /// A mod item maps back to the content block it places: the item's
    /// binding is the block the content root registered for it.
    #[test]
    #[ignore = "daily"]
    fn item_block_maps_a_mod_items_back_to_its_block() {
        use mod_api::{self, Hooks, Host};
        use std::sync::Arc;
        use {
            sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
            sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
        };

        const CHILD: &str = "SUNRISE_ITEM_BLOCK_MOD_CHILD";
        if run_fresh_cases(
            "cut_layout::game::native_block_states::tests::item_block_maps_a_mod_items_back_to_its_block",
            CHILD,
            &["game"],
        ) {
            return;
        }
        const CRATE: &str = "item_test:crate";
        struct TestMod;
        #[mod_api::__subscriptions]
        impl Hooks for TestMod {
            fn blocks(
                &self,
                _context: &mod_api::RegistrationContext,
                event: &mut mod_api::BlocksEvent<'_>,
            ) {
                event
                    .register(mod_api::Block::new(CRATE, "minecraft:stone"))
                    .unwrap();
            }
        }
        let mods: mod_api::Mods =
            Arc::from([mod_api::ModEntry::new("item_test", Box::new(TestMod))]);
        mod_platform::__private::install(mods.clone());
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        with_game_at(root, |game| {
            let host: &dyn Host = game;
            assert_eq!(
                host.item_block(mod_api::__private::item_key(CRATE).unwrap()),
                mod_api::__private::block_key(CRATE)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn host_key_iteration_matches_the_26_3_registries_in_registry_order() {
        use mod_api::Host;
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        // The content root publishes the built-in entity-type and menu orders, as startup does.
        let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        sunrise_gameplay::game::native_entity_refs::tests::with_game_at(root.clone(), |game| {
            let host: &dyn Host = game;
            let blocks = host.block_keys();
            assert_eq!(blocks.len(), 1286);
            assert_eq!(blocks[0].id(host), Some("minecraft:air"));
            assert_eq!(
                blocks.last().unwrap().id(host),
                Some("minecraft:firefly_bush")
            );

            let items = host.item_keys();
            assert_eq!(items.len(), 1658);
            assert_eq!(items[0].id(host), Some("minecraft:air"));
            assert_eq!(
                items.last().unwrap().id(host),
                Some("minecraft:ominous_bottle")
            );

            let entities = host.entity_type_keys();
            assert_eq!(entities.len(), 161);
            assert_eq!(
                entities[0].id(host).as_deref(),
                Some("minecraft:acacia_boat")
            );
            assert_eq!(
                entities.last().unwrap().id(host).as_deref(),
                Some("minecraft:fishing_bobber")
            );

            let menus = host.menu_keys();
            assert_eq!(menus.len(), 25);
            assert_eq!(menus[0].id(host).as_deref(), Some("minecraft:generic_9x1"));
            assert_eq!(
                menus.last().unwrap().id(host).as_deref(),
                Some("minecraft:stonecutter")
            );

            // Enchantments load with the item tables, which this harness stops short of;
            // read them the way a world open does.
            let items = sunrise_items::items::Items::load(
                &root,
                game.pack.assets.registry,
                game.res.facts(),
                &sunrise_world::loot::PackOverlay::NONE,
                game.res.item_defaults(),
            )
            .unwrap();
            mod_api::__private::resolve_enchantments(
                items.enchantments.list.iter().map(|entry| &*entry.id),
            )
            .unwrap();
            let enchantments = host.enchantment_keys();
            assert_eq!(enchantments.len(), 43);
            assert_eq!(
                enchantments[0].id(host).as_deref(),
                Some("minecraft:aqua_affinity")
            );
            assert_eq!(
                enchantments.last().unwrap().id(host).as_deref(),
                Some("minecraft:wind_burst")
            );

            // A second read is the same table in the same order.
            assert_eq!(host.block_keys(), blocks);
            assert_eq!(host.enchantment_keys(), enchantments);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn public_host_resolves_literal_block_and_effect_names() {
        mod_platform::testing::with_mods(std::sync::Arc::from([]), || {
            let root =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::with_game_at(root, |game| {
                let host: &dyn mod_api::Host = game;
                assert_eq!(
                    host.block_key("minecraft:stone").unwrap().id(host),
                    Some("minecraft:stone")
                );
                assert_eq!(
                    host.effect_key("minecraft:speed").unwrap().id(host),
                    Some("minecraft:speed")
                );
                assert_eq!(host.block_key("test:missing_block"), None);
                assert_eq!(host.effect_key("test:missing_effect"), None);
            });
        });
    }

    #[test]
    fn description_ids_write_a_path_separator_as_a_dot() {
        assert_eq!(
            description_id("fluid_type", "ruby:molten/iron").as_deref(),
            Some("fluid_type.ruby.molten.iron")
        );
        assert_eq!(description_id("block", "no_namespace"), None);
    }

    #[test]
    #[ignore = "daily"]
    fn fluid_type_properties_answer_each_fluid_family() {
        use mod_api::{Host, HostFluidStates};
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let host: &dyn Host = game;
            let service = host.fluid_states();
            let read = |id| {
                service
                    .type_properties(host.fluid_key(id).unwrap())
                    .unwrap()
            };
            let water = FluidTypeProperties {
                description_id: "block.minecraft.water".into(),
                light_level: 0,
                fall_distance_modifier: 0.0,
                can_push_entity: true,
                can_swim: true,
                can_drown: true,
                can_extinguish: true,
                can_hydrate: true,
                supports_boating: true,
                explosion_resistance: 100.0,
                randomly_ticks: false,
                drip_particle: Some("minecraft:dripping_water".into()),
                temperature: 300,
                density: 1000,
                viscosity: 1000,
            };
            let lava = FluidTypeProperties {
                description_id: "block.minecraft.lava".into(),
                light_level: 15,
                fall_distance_modifier: 0.5,
                can_push_entity: true,
                can_swim: false,
                can_drown: false,
                can_extinguish: false,
                can_hydrate: false,
                supports_boating: false,
                explosion_resistance: 100.0,
                randomly_ticks: true,
                drip_particle: Some("minecraft:dripping_lava".into()),
                temperature: 1300,
                density: 3000,
                viscosity: 6000,
            };
            // NeoForgeMod.EMPTY_TYPE states zeros; a tank-only fluid with no file keeps
            // `FluidType.Properties`' defaults.
            let inert = |description_id: &str, [temperature, density, viscosity]: [i32; 3]| {
                FluidTypeProperties {
                    description_id: description_id.into(),
                    light_level: 0,
                    fall_distance_modifier: 1.0,
                    can_push_entity: false,
                    explosion_resistance: 0.0,
                    randomly_ticks: false,
                    drip_particle: None,
                    temperature,
                    density,
                    viscosity,
                    ..lava.clone()
                }
            };
            assert_eq!(read("minecraft:water"), water);
            assert_eq!(read("minecraft:flowing_water"), water);
            assert_eq!(read("minecraft:lava"), lava);
            assert_eq!(read("minecraft:flowing_lava"), lava);
            assert_eq!(
                read("minecraft:empty"),
                inert("block.minecraft.air", [0; 3])
            );
            assert_eq!(
                read("sunrise:definition_test"),
                inert("fluid_type.sunrise.definition_test", [300, 1000, 1000])
            );
            assert_eq!(host.fluid_key("minecraft:no_such_fluid"), None);
        });
    }

    /// A mod's fluid file through the real content loader: the numbers it states, its light
    /// level among them, for its source and flowing forms alike, `FluidType.Properties`' defaults
    /// for keys it leaves out, and the bucket it names, which must be a live item that keeps the
    /// fluid. Loading a mod replaces process-wide fluid tables, so this runs in its own process.
    #[test]
    #[ignore = "daily"]
    fn a_mod_fluid_file_states_its_temperature_density_and_viscosity() {
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::native_block_states::tests::a_mod_fluid_file_states_its_temperature_density_and_viscosity",
            "SUNRISE_FLUID_MEASURES_CHILD",
            &["loaded"],
        ) {
            return;
        }
        use mod_api::{FluidTypeProperties, Host, HostFluidStates};
        let dir = tempfile::tempdir().unwrap();
        let fluids = dir.path().join("data/measured/sunrise/fluid");
        std::fs::create_dir_all(&fluids).unwrap();
        std::fs::write(
            dir.path().join("sunrise-mod.toml"),
            "id = \"measured\"\nname = \"Measured\"\nversion = \"1\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n",
        )
        .unwrap();
        std::fs::write(
            fluids.join("brine.json"),
            br#"{"temperature":-40,"density":1200,"viscosity":0,"light_level":10,"bucket_item":"measured:brine_pail","world":{}}"#,
        )
        .unwrap();
        std::fs::write(fluids.join("plain.json"), br#"{"world":{}}"#).unwrap();
        for (name, body) in [
            // A file alone registers no tank-only fluid.
            (
                "essence",
                &br#"{"bucket_item":"minecraft:honey_bottle"}"#[..],
            ),
            (
                "salt",
                br#"{"bucket_item":"minecraft:water_bucket","world":{"bucket":false}}"#,
            ),
            (
                "shared",
                br#"{"bucket_item":"measured:plain_bucket","world":{"bucket":false}}"#,
            ),
            (
                "lost",
                br#"{"bucket_item":"measured:no_such_jar","world":{"bucket":false}}"#,
            ),
            (
                "cod",
                br#"{"bucket_item":"minecraft:cod_bucket","world":{"bucket":false}}"#,
            ),
            (
                "hollow",
                br#"{"bucket_item":"minecraft:bucket","world":{"bucket":false}}"#,
            ),
        ] {
            std::fs::write(fluids.join(format!("{name}.json")), body).unwrap();
        }
        let mut sources = sunrise_gameplay_common::game::mods::ModsState {
            dev: vec![dir.path().to_path_buf()],
            ..Default::default()
        };
        sources.select(
            &std::env::current_dir().unwrap(),
            &["measured".to_owned()].into(),
        );
        let vanilla = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = sunrise_gameplay_common::game::mods::content_root(&vanilla, &sources).unwrap();
        sunrise_gameplay::game::native_entity_refs::tests::with_game_at(
            root.prepared().to_path_buf(),
            |game| {
                let host: &dyn Host = game;
                let read = |id: &str| -> FluidTypeProperties {
                    host.fluid_states()
                        .type_properties(host.fluid_key(id).unwrap())
                        .unwrap()
                };
                let measures = |id: &str| {
                    let properties = read(id);
                    [
                        properties.temperature,
                        properties.density,
                        properties.viscosity,
                    ]
                };
                assert_eq!(measures("measured:brine"), [-40, 1200, 0]);
                assert_eq!(measures("measured:flowing_brine"), [-40, 1200, 0]);
                assert_eq!(measures("measured:plain"), [300, 1000, 1000]);
                assert_eq!(measures("measured:flowing_plain"), [300, 1000, 1000]);
                // `FluidType.getLightLevel` is the file's, though the block emits none.
                assert_eq!(read("measured:flowing_brine").light_level, 10);
                assert_eq!(read("measured:plain").light_level, 0);
                // `Fluid.getBucket`: the named bucket and the default one, each holding its
                // fluid back; none for an unregistered fluid, a missing item or the game's
                // bucket of another fluid.
                let states = host.block_states();
                let bucket =
                    |fluid: &str| host.fluid_key(fluid).and_then(|key| key.bucket(&states));
                let item = |id: &str| mod_api::__private::item_key(id);
                assert_eq!(
                    bucket("measured:flowing_brine"),
                    item("measured:brine_pail")
                );
                assert!(item("measured:brine_bucket").is_none());
                assert_eq!(bucket("measured:plain"), item("measured:plain_bucket"));
                for (bucket, fluid) in [
                    ("measured:brine_pail", "measured:brine"),
                    ("measured:plain_bucket", "measured:plain"),
                    ("minecraft:water_bucket", "minecraft:water"),
                ] {
                    assert_eq!(
                        item(bucket).unwrap().bucket_fluid(&states),
                        host.fluid_key(fluid)
                    );
                }
                assert_eq!(
                    item("minecraft:bucket").unwrap().bucket_fluid(&states),
                    None
                );
                for fluid in [
                    "measured:essence",
                    "measured:salt",
                    "measured:shared",
                    "measured:lost",
                    "measured:cod",
                    "measured:hollow",
                ] {
                    assert_eq!(bucket(fluid), None, "{fluid}");
                }
                let essence = host.fluid_key("measured:essence");
                assert_eq!(essence.and_then(|key| key.pickup_sound(&states)), None);
                assert_eq!(essence.and_then(|key| key.empty_sound(&states)), None);
            },
        );
    }

    /// `FluidState.createLegacyBlock`: a source is its block at level 0, flowing water of amount
    /// 3 is level 5, falling adds 8, the empty fluid is air; states no fluid can be in have none.
    #[test]
    #[ignore = "daily"]
    fn legacy_blocks_follow_the_liquid_levels() {
        use mod_api::{BlockMetadata, Host, HostBlockStates, HostFluidStates};
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let host: &dyn Host = game;
            let service = host.fluid_states();
            let metadata = host.block_states();
            let state = |id: &str, amount, falling, source| {
                mod_api::__private::fluid_state(
                    host.fluid_key(id).unwrap(),
                    amount,
                    falling,
                    source,
                )
            };
            let block = |state: FluidState| {
                let state = service.legacy_block(state)?;
                Some((
                    metadata.name(state.block()).unwrap(),
                    state.property(&metadata, "level").unwrap(),
                ))
            };
            assert_eq!(
                block(state("minecraft:water", 8, false, true)),
                Some(("minecraft:water", Some("0")))
            );
            assert_eq!(
                block(state("minecraft:flowing_water", 3, false, false)),
                Some(("minecraft:water", Some("5")))
            );
            assert_eq!(
                block(state("minecraft:flowing_lava", 3, true, false)),
                Some(("minecraft:lava", Some("13")))
            );
            assert_eq!(
                block(state("minecraft:flowing_water", 1, false, false)),
                Some(("minecraft:water", Some("7")))
            );
            assert_eq!(
                block(state("minecraft:empty", 0, false, false)),
                Some(("minecraft:air", None))
            );
            for refused in [
                state("minecraft:flowing_water", 9, false, false),
                state("minecraft:flowing_water", 0, false, false),
                state("minecraft:water", 3, false, true),
                state("minecraft:empty", 8, false, false),
                state("sunrise:definition_test", 8, false, true),
                state("minecraft:flowing_water", u8::MAX, true, false),
            ] {
                assert_eq!(service.legacy_block(refused), None, "{refused:?}");
            }
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn fluid_definitions_and_constraints_use_the_loaded_registry() {
        use mod_api::{Host, HostFluidStates};
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let host: &dyn Host = game;
            let water = host.fluid_key("minecraft:water").unwrap();
            let flowing = host.fluid_key("minecraft:flowing_water").unwrap();
            let setup: &dyn mod_api::ClientSetupContext = game;
            let reload: &dyn mod_api::ReloadContext = game;
            for context in [setup as &dyn mod_api::ReloadContext, reload] {
                let water = context.fluid_key("minecraft:water").unwrap();
                let fluids = context.fluid_states();
                assert!(matches!(
                    fluids.definition(water),
                    Some(FluidDefinition::World { .. })
                ));
                assert_eq!(fluids.validate(water, &[]), Ok(()));
                let stone = context.block_state_id("minecraft:stone").unwrap();
                assert_eq!(context.block_states().destroy_speed(stone), Some(1.5));

                let item = |id| {
                    let key = context
                        .item_key(&mod_api::Identifier::parse(id).unwrap())
                        .unwrap();
                    mod_api::ItemResource::of(&mod_api::ItemStack::new(key, 1).unwrap())
                };
                let sword = item("minecraft:iron_sword");
                assert!(sword.damageable(context));
                assert!(!sword.foil(context));
                assert!(sword.repairable(context, &item("minecraft:iron_ingot")));
                assert!(!sword.repairable(context, &item("minecraft:stick")));
            }
            let service = host.fluid_states();
            let tank = host.fluid_key("sunrise:definition_test").unwrap();
            assert_eq!(service.definition(tank), Some(FluidDefinition::TankOnly));
            assert_eq!(
                service.validate(tank, &[]),
                Err(FluidStateError::NoWorldState)
            );
            const SOURCE: &[(&str, &[&str])] = &[("falling", &["true", "false"])];
            const FLOWING: &[(&str, &[&str])] = &[
                ("falling", &["true", "false"]),
                ("level", &["1", "2", "3", "4", "5", "6", "7", "8"]),
            ];
            for (id, amount, source, expected) in [
                ("minecraft:water", 8, true, SOURCE),
                ("minecraft:flowing_water", 1, false, FLOWING),
                ("minecraft:lava", 8, true, SOURCE),
                ("minecraft:flowing_lava", 1, false, FLOWING),
                ("minecraft:empty", 0, false, &[]),
            ] {
                let key = host.fluid_key(id).unwrap();
                let FluidDefinition::World {
                    default,
                    properties,
                } = host.fluid_states().definition(key).unwrap()
                else {
                    panic!("world definition")
                };
                assert_eq!(
                    (
                        default.fluid,
                        default.amount,
                        default.source,
                        default.falling
                    ),
                    (key, amount, source, amount != 0)
                );
                assert_eq!(properties.is_empty(), id == "minecraft:empty");
                assert_eq!(properties.len(), expected.len());
                for (property, &(name, values)) in properties.iter().zip(expected) {
                    assert_eq!(property.name, name);
                    assert_eq!(property.values, values);
                }
                assert_eq!(service.validate(key, &[]), Ok(()));
            }
            assert_eq!(
                service.validate(
                    flowing,
                    &[
                        StateConstraint::new("unknown", "true"),
                        StateConstraint::new("level", "1"),
                        StateConstraint::new("level", "1")
                    ]
                ),
                Err(FluidStateError::UnknownProperty(0))
            );
            let FluidDefinition::World { default, .. } = service.definition(water).unwrap() else {
                panic!("world fluid")
            };
            assert!(default.is_source());
            assert_eq!(
                service.validate(water, &[StateConstraint::new("level", "8")]),
                Err(FluidStateError::UnknownProperty(0))
            );
            assert_eq!(
                service.validate(flowing, &[StateConstraint::new("level", "9")]),
                Err(FluidStateError::InvalidValue(0))
            );
            assert_eq!(
                service.validate(
                    flowing,
                    &[
                        StateConstraint::new("level", "8"),
                        StateConstraint::new("level", "8")
                    ]
                ),
                Err(FluidStateError::Duplicate(1))
            );
            assert!(default.matches(&[StateConstraint::new("falling", "true")]));
            assert!(!default.matches(&[StateConstraint::new("unknown", "true")]));
            let FluidDefinition::World {
                default: flowing_default,
                ..
            } = service.definition(flowing).unwrap()
            else {
                panic!("flowing fluid")
            };
            assert!(flowing_default.matches(&[StateConstraint::new("level", "1")]));
            assert!(!flowing_default.matches(&[StateConstraint::new("level", "8")]));
        });
    }

    #[test]
    fn block_property_constraints_validate_and_match_partial_states() {
        let (reg, registry) = blocks();
        let target = registry.state(BlockStateId(1)).unwrap();
        let definition = &reg.blocks["minecraft:oak_stairs"];
        let check = |id, constraints: &[StateConstraint]| {
            let state = registry.state(BlockStateId(id)).unwrap();
            match_state_properties(
                definition,
                target.name == state.name,
                |name, value| {
                    state
                        .properties
                        .get(name)
                        .is_some_and(|actual| actual == value)
                },
                constraints,
            )
        };
        let east = StateConstraint::new("facing", "east");
        assert_eq!(check(4, std::slice::from_ref(&east)), Ok(true));
        assert_eq!(check(1, std::slice::from_ref(&east)), Ok(false));
        assert_eq!(check(0, std::slice::from_ref(&east)), Ok(false));
        assert_eq!(
            check(4, &[StateConstraint::new("shape", "straight")]),
            Err(StateMatchError::UnknownProperty(0))
        );
        assert_eq!(
            check(4, &[StateConstraint::new("facing", "up")]),
            Err(StateMatchError::InvalidValue(0))
        );
        assert_eq!(check(4, &[]), Ok(true));
        assert_eq!(definition.len(), 2);
        assert!(
            definition
                .iter()
                .any(|(name, values)| name == "half" && values.contains(&"top".to_owned()))
        );
    }

    #[test]
    fn a_state_name_reads_as_setblock_reads_it_and_round_trips() {
        let (reg, registry) = blocks();
        let id = |name: &str| parse(&reg, registry, name);
        assert_eq!(id("minecraft:air"), Some(BlockStateId(0)));
        assert_eq!(id("oak_stairs"), Some(BlockStateId(1)));
        assert_eq!(
            id("oak_stairs[half=top,facing=east]"),
            Some(BlockStateId(4))
        );
        assert_eq!(
            id("minecraft:oak_stairs[facing=east]"),
            Some(BlockStateId(3))
        );
        // IntegerProperty.getValue: Integer.parseInt.
        assert_eq!(id("wheat[age=01]"), Some(BlockStateId(6)));
        for state in 0..7 {
            let text = serialize(registry.state(BlockStateId(state)).unwrap());
            assert_eq!(id(&text), Some(BlockStateId(state)), "{text}");
        }
        assert_eq!(
            serialize(registry.state(BlockStateId(4)).unwrap()),
            "minecraft:oak_stairs[facing=east,half=top]"
        );
        assert_eq!(
            serialize(registry.state(BlockStateId(0)).unwrap()),
            "minecraft:air"
        );
    }

    #[test]
    fn a_state_name_that_does_not_read_has_no_id() {
        let (reg, registry) = blocks();
        for name in [
            "minecraft:nothing",
            "oak_stairs[shape=straight]",
            "oak_stairs[facing=up]",
            "oak_stairs[facing=east,facing=north]",
            "wheat[age=2]",
            "oak_stairs{}",
            "oak_stairs extra",
            "#minecraft:stairs",
            "",
        ] {
            assert_eq!(parse(&reg, registry, name), None, "{name:?}");
        }
    }

    #[test]
    fn a_block_key_resolves_its_default_state() {
        use mod_platform::__private::part_tests;
        let (_, registry) = blocks();
        mod_platform::__private::install(Default::default());
        crate::game::api_values::register_game_keys([
            "minecraft:oak_stairs".to_owned(),
            "minecraft:wheat".to_owned(),
        ]);
        sunrise_gameplay_common::game::api_values::resolve_block_definitions(registry).unwrap();
        assert_eq!(
            sunrise_gameplay_common::game::api_values::default_state(part_tests::block_key(
                "oak_stairs"
            ))
            .unwrap(),
            BlockStateId(1)
        );
        assert_eq!(
            sunrise_gameplay_common::game::api_values::default_state(part_tests::block_key(
                "minecraft:wheat"
            ))
            .unwrap(),
            BlockStateId(5)
        );
        // A valid key has no mapping on a different owning thread. Raw unknown slots are
        // refused by the generated host boundary, not revalidated by typed operations.
        let key = part_tests::block_key("minecraft:wheat");
        assert!(
            std::thread::spawn(
                move || sunrise_gameplay_common::game::api_values::default_state(key)
            )
            .join()
            .unwrap()
            .is_none()
        );
    }
}

#[cfg(test)]
mod facts_tests {
    use mod_api::HostBlockStates;
    use sunrise_gameplay_common::game::api_values::TryIntoApi;

    #[test]
    #[ignore = "daily"]
    fn weathering_and_wax_reads_keep_shared_properties() {
        use mod_api::{WeatheringAge::*, prelude::BlockStatesHost};

        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let stairs = |name| {
                let id = format!(
                    "minecraft:{name}[facing=east,half=top,shape=inner_left,waterlogged=true]"
                );
                BlockStatesHost::block_state_id(game, &id).unwrap()
            };
            let fresh = stairs("cut_copper_stairs");
            let exposed = stairs("exposed_cut_copper_stairs");
            let weathered = stairs("weathered_cut_copper_stairs");
            let oxidized = stairs("oxidized_cut_copper_stairs");
            let waxed = stairs("waxed_exposed_cut_copper_stairs");
            let facts = game.block_states();
            assert_eq!(facts.weathering_age(fresh), Some(Unaffected));
            assert_eq!(facts.weathering_age(exposed), Some(Exposed));
            assert_eq!(facts.weathering_age(weathered), Some(Weathered));
            assert_eq!(facts.weathering_age(oxidized), Some(Oxidized));
            assert_eq!(facts.weathering_age(waxed), None);
            assert_eq!(facts.next_weathering_stage(waxed), None);
            assert_eq!(facts.previous_weathering_stage(waxed), None);
            assert_eq!(facts.previous_weathering_stage(fresh), None);
            assert_eq!(facts.next_weathering_stage(oxidized), None);
            assert_eq!(facts.next_weathering_stage(fresh), Some(exposed));
            assert_eq!(facts.previous_weathering_stage(oxidized), Some(weathered));
            assert_eq!(facts.waxed(exposed), Some(waxed));
            assert_eq!(facts.unwaxed(waxed), Some(exposed));
            assert_eq!(facts.unwaxed(exposed), None);

            let invalid = mod_api::__private::block_state_from_ordinal(exposed.block(), u16::MAX);
            assert_eq!(facts.weathering_age(invalid), None);
            assert_eq!(facts.next_weathering_stage(invalid), None);
            assert_eq!(facts.waxed(invalid), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_state_facts_read_vanillas_tables_through_the_host() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let state = |name: &str| {
                game.pack
                    .assets
                    .registry
                    .default_state(name)
                    .unwrap()
                    .try_into_api()
                    .unwrap()
            };
            let (stone, obsidian, bedrock) = (
                state("minecraft:stone"),
                state("minecraft:obsidian"),
                state("minecraft:bedrock"),
            );
            let (air, grass, chest) = (
                state("minecraft:air"),
                state("minecraft:short_grass"),
                state("minecraft:chest"),
            );
            let stairs = state("minecraft:oak_stairs");
            let facts = game.block_states();
            // destroyTime: stone 1.5, obsidian 50, bedrock -1 (unbreakable).
            assert_eq!(facts.destroy_speed(stone), Some(1.5));
            assert_eq!(facts.destroy_speed(obsidian), Some(50.0));
            assert_eq!(facts.destroy_speed(bedrock), Some(-1.0));
            // canBeReplaced: air and grass, not stone.
            assert_eq!(facts.replaceable(air), Some(true));
            assert_eq!(facts.replaceable(grass), Some(true));
            assert_eq!(facts.replaceable(stone), Some(false));
            assert_eq!(facts.has_block_entity(chest), Some(true));
            assert_eq!(facts.has_block_entity(stone), Some(false));
            // MapColor.STONE is id 11 and air has none.
            assert_eq!(facts.map_color(stone), Some(11));
            assert_eq!(facts.map_color(air), Some(0));
            // getLightEmission: a torch 14, glowstone 15, stone 0.
            assert_eq!(facts.light_emission(state("minecraft:torch")), Some(14));
            assert_eq!(facts.light_emission(state("minecraft:glowstone")), Some(15));
            assert_eq!(facts.light_emission(stone), Some(0));
            // getFriction: 0.6 by default, ice 0.98, slime 0.8.
            assert_eq!(facts.friction(stone), Some(0.6));
            assert_eq!(facts.friction(state("minecraft:ice")), Some(0.98));
            assert_eq!(facts.friction(state("minecraft:slime_block")), Some(0.8));
            // getSpeedFactor and getJumpFactor: 1.0 by default, soul sand 0.4 speed, honey 0.4
            // speed and 0.5 jump.
            let (soul_sand, honey) = (state("minecraft:soul_sand"), state("minecraft:honey_block"));
            assert_eq!(facts.speed_factor(stone), Some(1.0));
            assert_eq!(facts.jump_factor(stone), Some(1.0));
            assert_eq!(facts.speed_factor(soul_sand), Some(0.4));
            assert_eq!(facts.jump_factor(soul_sand), Some(1.0));
            assert_eq!(facts.speed_factor(honey), Some(0.4));
            assert_eq!(facts.jump_factor(honey), Some(0.5));
            // isSolidRender: a full occluding cube only.
            assert_eq!(facts.is_solid_render(stone), Some(true));
            assert_eq!(facts.is_solid_render(state("minecraft:glass")), Some(false));
            assert_eq!(
                facts.is_solid_render(state("minecraft:stone_slab")),
                Some(false)
            );
            // PoiTypes.forState: a lectern is a librarian's job site, a bell a meeting point;
            // stone is in no POI type.
            let poi = |name: &str| facts.poi_type(state(name)).map(|id| id.to_string());
            assert_eq!(
                poi("minecraft:lectern").as_deref(),
                Some("minecraft:librarian")
            );
            assert_eq!(poi("minecraft:bell").as_deref(), Some("minecraft:meeting"));
            assert_eq!(poi("minecraft:stone"), None);
            // propagatesSkylightDown: glass's own answer and air, not a full cube.
            assert_eq!(
                facts.propagates_skylight_down(state("minecraft:glass")),
                Some(true)
            );
            assert_eq!(facts.propagates_skylight_down(air), Some(true));
            assert_eq!(facts.propagates_skylight_down(stone), Some(false));
            // isAir: the three airs by their own flag, nothing else.
            assert_eq!(facts.is_air(air), Some(true));
            assert_eq!(facts.is_air(state("minecraft:cave_air")), Some(true));
            assert_eq!(facts.is_air(stone), Some(false));
            assert_eq!(facts.is_air(grass), Some(false));
            assert_eq!(facts.is_redstone_conductor(stone), Some(true));
            assert_eq!(
                facts.is_redstone_conductor(state("minecraft:glass")),
                Some(false)
            );
            assert_eq!(
                facts.is_redstone_conductor(state("minecraft:moving_piston")),
                Some(false)
            );
            assert_eq!(
                facts.is_redstone_conductor(state("minecraft:red_shulker_box")),
                None
            );
            // SoundType.STONE: its break event, at volume and pitch 1.
            let sound = facts.sound(stone).unwrap();
            assert_eq!(sound.break_sound.as_str(), "minecraft:block.stone.break");
            assert_eq!((sound.volume, sound.pitch), (1.0, 1.0));
            let properties = game.block_states().definition(stairs.block()).unwrap();
            let expected: [(&str, &[&str]); 4] = [
                ("facing", &["north", "south", "west", "east"]),
                ("half", &["top", "bottom"]),
                (
                    "shape",
                    &[
                        "straight",
                        "inner_left",
                        "inner_right",
                        "outer_left",
                        "outer_right",
                    ],
                ),
                ("waterlogged", &["true", "false"]),
            ];
            assert_eq!(properties.len(), expected.len());
            for (property, (name, values)) in properties.iter().zip(expected) {
                assert_eq!(property.name, name);
                assert_eq!(property.values, values);
            }
            assert!(facts.definition(stone.block()).unwrap().is_empty());
            // Oak stairs: facing 4 × half 2 × shape 5 × waterlogged 2 = 80 states, its default
            // among them; stone has one.
            let range = facts
                .states_of(mod_platform::__private::part_tests::block_key(
                    "minecraft:oak_stairs",
                ))
                .unwrap();
            assert_eq!(range.len(), 80);
            assert!(range.contains(stairs));
            let key = mod_platform::__private::part_tests::block_key("minecraft:oak_stairs");
            assert!(range.iter().all(|state| state.block() == key));
            assert_eq!(
                range
                    .iter()
                    .map(mod_api::BlockState::ordinal)
                    .collect::<Vec<_>>(),
                (0..80).collect::<Vec<u16>>()
            );
            assert_eq!(
                facts
                    .states_of(mod_platform::__private::part_tests::block_key(
                        "minecraft:stone"
                    ))
                    .map(|r| (r.first(), r.len())),
                Some((stone, 1))
            );
            // A forged local ordinal refuses at the generated Game crossing.
            let invalid = mod_api::__private::block_state_from_ordinal(stone.block(), u16::MAX);
            assert_eq!(facts.destroy_speed(invalid), None);
            assert_eq!(facts.sound(invalid), None);
        });
    }
}

#[cfg(test)]
mod crafting_remainder_tests {
    use {
        crate::game::native_block_states::*,
        sunrise_gameplay_common::game::native_block_states::ItemStack,
    };
    #[cfg(feature = "client")]
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    #[cfg(feature = "client")]
    struct Worn;
    #[cfg(feature = "client")]
    #[mod_api::sunrise_part]
    impl mod_api::ItemPart for Worn {
        fn crafting_remainder(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::CraftingRemainderEvent<'_>,
        ) {
            CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            *event.remainder = if event.stack.damage() == 0 {
                let mut stack = event.stack.project().expect("the input stack projects");
                stack.set_count(65).unwrap();
                Some(stack)
            } else if event.stack.damage() >= 4 {
                None
            } else {
                let mut stack = event.stack.project().expect("the input stack projects");
                stack.set_count(1).unwrap();
                stack.set_damage(event.stack.damage() + 1);
                Some(stack)
            };
        }
    }
    #[cfg(feature = "client")]
    struct Empty;
    #[cfg(feature = "client")]
    #[mod_api::sunrise_part]
    impl mod_api::ItemPart for Empty {
        fn crafting_remainder(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::CraftingRemainderEvent<'_>,
        ) {
            CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            *event.remainder = None;
        }
    }
    #[cfg(feature = "client")]
    struct DefaultPart;
    #[cfg(feature = "client")]
    #[mod_api::sunrise_part]
    impl mod_api::ItemPart for DefaultPart {
        fn inventory_tick(
            &self,
            _: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::InventoryTickEvent<'_>,
        ) {
        }
    }
    #[cfg(feature = "client")]
    struct RemainderMod;
    #[cfg(feature = "client")]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for RemainderMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let tool = r.item_key("minecraft:wooden_hoe");
            r.item_part(tool, Worn);
            let milk = r.item_key("minecraft:milk_bucket");
            r.item_part(milk, DefaultPart);
            let water = r.item_key("minecraft:water_bucket");
            r.item_part(water, Empty);
        }
    }
    #[test]
    #[ignore = "daily"]
    fn crafting_remainder_without_parts_matches_data() {
        let mods: mod_api::Mods = Vec::new().into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                assert!(!mod_api::__private::has_part_hook(
                    mod_api::HookId::CraftingRemainder
                ));
                let grid = [
                    Some(ItemStack::new("minecraft:milk_bucket", 1)),
                    None,
                    Some(ItemStack::new("minecraft:stone", 1)),
                    None,
                ];
                let remaining =
                    game.recipes()
                        .remainders(&game.loop_context, "missing", &grid, 2, &|s| {
                            game.crafting_remainder(s)
                        });
                assert_eq!(
                    remaining,
                    vec![
                        Some(ItemStack::new("minecraft:bucket", 1)),
                        None,
                        None,
                        None
                    ]
                );
            });
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn crafting_remainder_part_override_and_default() {
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::native_block_states::crafting_remainder_tests::crafting_remainder_part_override_and_default",
            "SUNRISE_FURNACE_CRAFTING_REMAINDER_CASE",
            &["override"],
        ) {
            return;
        }
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("remainder", Box::new(RemainderMod))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            let mut client_fuel = None;
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                sunrise_gameplay::game::native_entity_refs::tests::resolve_parts(game).unwrap();
                let mut tool = ItemStack {
                    damage: 2,
                    ..ItemStack::new("minecraft:wooden_hoe", 1)
                };
                tool.data_mut()
                    .set_other("minecraft:max_damage", Some(sunrise_core::nbt::Tag::Int(5)));
                tool = game.crafting_remainder(&tool).unwrap();
                assert_eq!((tool.damage, tool.count), (3, 1));
                tool = game.crafting_remainder(&tool).unwrap();
                assert_eq!((tool.damage, tool.count), (4, 1));
                assert!(game.crafting_remainder(&tool).is_none());
                let calls = CALLS.load(std::sync::atomic::Ordering::Relaxed);
                for id in ["minecraft:milk_bucket", "minecraft:lava_bucket"] {
                    let remainder = game.crafting_remainder(&ItemStack::new(id, 1)).unwrap();
                    assert_eq!((&*remainder.item, remainder.count), ("minecraft:bucket", 1));
                }
                assert_eq!(
                    CALLS.load(std::sync::atomic::Ordering::Relaxed),
                    calls,
                    "items without this subscription call no Part"
                );
                let water = ItemStack::new("minecraft:water_bucket", 1);
                assert!(
                    game.crafting_remainder(&water).is_none(),
                    "None suppresses the data remainder"
                );
                let bucket = game.item_crafting_remainder(&water).unwrap();
                assert_eq!((&*bucket.item, bucket.count), ("minecraft:bucket", 1));
                assert!(game.item_crafting_remainder(&tool).is_none());
                assert!(
                    game.crafting_remainder(&ItemStack::new("minecraft:wooden_hoe", 1))
                        .is_none(),
                    "oversized Part output is refused"
                );
                let fuel = ItemStack {
                    damage: 2,
                    ..ItemStack::new("minecraft:wooden_hoe", 1)
                };
                let furnace = sunrise_level::containers::Furnace {
                    kind: sunrise_level::containers::FurnaceKind::Furnace,
                    items: [
                        Some(ItemStack::new("minecraft:iron_ore", 1)),
                        Some(fuel.clone()),
                        None,
                    ],
                    lit_remaining: 0,
                    lit_total: 0,
                    cooking_timer: 0,
                    cooking_total: 0,
                    speed: 1.0,
                    recipes_used: Default::default(),
                };
                use sunrise_level::containers::BlockEntity;
                let pos = (8, 64, 8);
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:furnace")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                game.world
                    .block_entities
                    .insert(pos, BlockEntity::Furnace(furnace));
                game.tick_furnace(pos).unwrap();
                let Some(BlockEntity::Furnace(furnace)) = game.world.block_entities.get(&pos)
                else {
                    panic!("no furnace");
                };
                let grid = [Some(fuel), None, None, None];
                let crafted =
                    game.recipes()
                        .remainders(&game.loop_context, "missing", &grid, 2, &|s| {
                            game.crafting_remainder(s)
                        });
                assert_eq!(furnace.items[1], crafted[0]);
                assert_eq!(
                    furnace.items[1].as_ref().map(|s| (s.damage, s.count)),
                    Some((3, 1))
                );
                let role = game.options.execution_role;
                game.options.execution_role =
                    sunrise_gameplay_common::game::execution_role::ExecutionRole::DedicatedServer;
                let remainder = game.crafting_remainder(grid[0].as_ref().unwrap()).unwrap();
                assert_eq!((remainder.damage, remainder.count), (3, 1));
                game.options.execution_role = role;
                client_fuel = grid[0].clone();
            });
            let client_fuel = client_fuel.unwrap();
            crate::game::server_side::native_entity_refs::tests::with_client_game(|client| {
                sunrise_gameplay::game::native_entity_refs::tests::resolve_parts(client).unwrap();
                assert!(mod_api::__private::item_listens(
                    mod_api::HookId::CraftingRemainder,
                    &client_fuel.item
                ));
                let calls = CALLS.load(std::sync::atomic::Ordering::Relaxed);
                assert!(client.crafting_remainder(&client_fuel).is_none());
                assert_eq!(CALLS.load(std::sync::atomic::Ordering::Relaxed), calls);
            });
        });
    }
}
use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::native_block_states::NativeBlockStatesGameExt1 as _;
