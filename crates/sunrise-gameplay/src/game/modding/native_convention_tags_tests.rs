//! Two-mod interoperability and 26.3 integrity tests for built-in `c:` convention tags.
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game, with_game_at};
use mod_api::tags::c;
use mod_api::{
    BiomeKey, BlockKey, DamageTypeKey, EnchantmentKey, EntityTypeKey, FluidKey, Hooks, Host,
    Identifier, Item, ItemKey, ItemsEvent, ModEntry, PotionKey, RegistrationContext, ReloadContext,
    StructureKey, TagMembers, TagsUpdatedEvent, WorldClockKey,
};
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::GameGameExt1 as _;
use sunrise_gameplay_common::game::tables::GameItems as _;
use sunrise_items::items::ItemStack;
use sunrise_world::tag_layers::convention;

/// Two independently built mods (`mod_a` and `mod_b`, with no dependency between them)
/// interoperate through `c:` convention tags: `mod_a` registers `mod_a:bronze_ingot` and
/// adds it to `c:ingots/bronze` and `#c:ingots`; `mod_b` reads `c::items::INGOTS` and
/// `c:ingots/bronze` through `host.tags()`, crafts a recipe referencing `#c:ingots/bronze`,
/// and exercises all ten typed `Tags` queries.
#[test]
#[ignore = "daily"]
fn two_mods_interoperate_through_c_convention_tags_and_all_ten_registries() {
    if run_fresh_cases(
        "game::server_side::native_convention_tags_tests::two_mods_interoperate_through_c_convention_tags_and_all_ten_registries",
        "SUNRISE_TEST_CONVENTION_TAGS_TWO_MODS",
        &["interop"],
    ) {
        return;
    }
    use mod_api::__private::native::BuiltMod;
    use sunrise_mod_package::{mod_list::NATIVE_DIR, runtime::Runtime};

    struct ModA;
    #[mod_api::__subscriptions]
    impl Hooks for ModA {
        fn items(&self, _: &RegistrationContext, event: &mut ItemsEvent<'_>) {
            event
                .register(Item::new("mod_a:bronze_ingot", "minecraft:iron_ingot"))
                .unwrap();
        }
    }

    #[derive(Default)]
    struct ModBSeen {
        ingots_contains_iron: bool,
        ingots_contains_bronze: bool,
        bronze_exact_match: bool,
    }

    struct ModB(Arc<Mutex<Option<ModBSeen>>>);
    #[mod_api::__subscriptions]
    impl Hooks for ModB {
        fn tags_updated(&self, context: &dyn ReloadContext, _event: &mut TagsUpdatedEvent) {
            let tags = context.tags();
            let TagMembers::Members(ingots) = tags.items(c::items::INGOTS) else {
                return;
            };
            let bronze_tag = tags
                .key(&Identifier::parse("c:ingots/bronze").unwrap())
                .unwrap()
                .typed::<ItemKey>();
            let TagMembers::Members(bronze) = tags.items(bronze_tag) else {
                return;
            };
            let iron_key = context
                .item_key(&Identifier::parse("minecraft:iron_ingot").unwrap())
                .unwrap();
            let bronze_key = context
                .item_key(&Identifier::parse("mod_a:bronze_ingot").unwrap())
                .unwrap();
            *self.0.lock().unwrap() = Some(ModBSeen {
                ingots_contains_iron: ingots.contains(&iron_key)
                    && tags.is_in_tag(iron_key, c::items::INGOTS) == Some(true),
                ingots_contains_bronze: ingots.contains(&bronze_key)
                    && tags.is_in_tag(bronze_key, c::items::INGOTS) == Some(true),
                bronze_exact_match: bronze == vec![bronze_key],
            });
        }
    }

    const BUILT: &[BuiltMod] = &[
        BuiltMod {
            id: "mod_a",
            version: "1.0.0",
            side: mod_api::Side::Both,
        },
        BuiltMod {
            id: "mod_b",
            version: "1.0.0",
            side: mod_api::Side::Both,
        },
    ];
    let dir = tempfile::tempdir().unwrap();
    for package in BUILT {
        let id = package.id;
        let path = dir.path().join(NATIVE_DIR).join(id);
        std::fs::create_dir_all(path.join("src")).unwrap();
        std::fs::write(
            path.join("sunrise-mod.toml"),
            format!(
                "id='{id}'\nname='{id}'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
                "a".repeat(40),
                "b".repeat(40),
            ),
        )
        .unwrap();
        std::fs::write(
            path.join("Cargo.toml"),
            format!("[package]\nname='{id}'\nversion='1.0.0'\n"),
        )
        .unwrap();
        std::fs::write(path.join("src/lib.rs"), "").unwrap();
    }

    // mod_a adds mod_a:bronze_ingot to c:ingots/bronze and #c:ingots/bronze to c:ingots,
    // plus a custom world_clock tag so non-empty world_clock membership is exercised too.
    let mod_a_tags = dir.path().join(NATIVE_DIR).join("mod_a/data/c/tags/item");
    std::fs::create_dir_all(mod_a_tags.join("ingots")).unwrap();
    std::fs::write(
        mod_a_tags.join("ingots/bronze.json"),
        br#"{"values":["mod_a:bronze_ingot"]}"#,
    )
    .unwrap();
    std::fs::write(
        mod_a_tags.join("ingots.json"),
        br##"{"values":["#c:ingots/bronze"]}"##,
    )
    .unwrap();
    let mod_a_clocks = dir
        .path()
        .join(NATIVE_DIR)
        .join("mod_a/data/mod_a/tags/world_clock");
    std::fs::create_dir_all(&mod_a_clocks).unwrap();
    std::fs::write(
        mod_a_clocks.join("primary.json"),
        br#"{"values":["minecraft:overworld"]}"#,
    )
    .unwrap();

    // mod_b defines a recipe consuming #c:ingots/bronze without referencing mod_a directly.
    let mod_b_recipes = dir.path().join(NATIVE_DIR).join("mod_b/data/mod_b/recipe");
    std::fs::create_dir_all(&mod_b_recipes).unwrap();
    std::fs::write(
        mod_b_recipes.join("bronze_nugget.json"),
        br##"{"type":"minecraft:crafting_shapeless","ingredients":["#c:ingots/bronze"],"result":{"id":"minecraft:gold_nugget","count":1}}"##,
    )
    .unwrap();

    let sources = sunrise_gameplay_common::game::mods::ModsState {
        native: Arc::new(Runtime::load(&dir.path().join("sunrise-client"), BUILT).unwrap()),
        ..Default::default()
    };
    let seen = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![
        ModEntry::new("mod_a", Box::new(ModA)),
        ModEntry::new("mod_b", Box::new(ModB(seen.clone()))),
    ]
    .into();

    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::test_platform::install();
        sunrise_gameplay_common::game::api_values::install_packages(sources.native.packages(), &[])
            .unwrap();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = sunrise_gameplay_common::game::mods::content_root(&base, &sources).unwrap();
        with_game_at(root.prepared().to_owned(), |game| {
            game.mods.native = sources.native.clone();
            game.pack.source = root.source().clone();
            game.pack.data_source = root.data().clone();
            game.pack.data_changes = root.data_changes().clone();
            game.options.dedicated = Some(Vec::new());
            let save = tempfile::tempdir().unwrap();
            let mut request = crate::game::world_loading::WorldRequest::new(
                "c_tags_interop",
                None,
                Some(1),
                true,
                None,
                Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
            )
            .unwrap();
            request.save_root = save.path().to_owned();
            game.load_world(request).unwrap();

            // 1. mod_b observed mod_a's bronze_ingot and vanilla iron_ingot via c::items::INGOTS.
            let observed = seen
                .lock()
                .unwrap()
                .take()
                .expect("mod_b tags_updated fired");
            assert!(observed.ingots_contains_iron);
            assert!(observed.ingots_contains_bronze);
            assert!(observed.bronze_exact_match);

            // 2. mod_b's recipe using #c:ingots/bronze matches mod_a:bronze_ingot.
            let crafted = game
                .recipes()
                .craft(
                    &game.loop_context,
                    &[Some(ItemStack::new("mod_a:bronze_ingot", 1))],
                    1,
                )
                .expect("recipe referencing #c:ingots/bronze must craft with mod_a:bronze_ingot");
            assert_eq!(&*crafted.item, "minecraft:gold_nugget");

            // 3. All ten typed Tags queries resolve with mod_api::tags::c::* constants.
            let host: &dyn Host = game;
            let tags = host.tags();

            let TagMembers::Members(ores) = tags.blocks(c::blocks::ORES) else {
                panic!("c:ores block tag must bind");
            };
            assert!(ores.contains(&host.block_key("minecraft:iron_ore").unwrap()));

            let TagMembers::Members(ingots) = tags.items(c::items::INGOTS) else {
                panic!("c:ingots item tag must bind");
            };
            assert!(
                ingots.contains(
                    &host
                        .item_key(&Identifier::parse("minecraft:iron_ingot").unwrap())
                        .unwrap()
                )
            );
            assert!(
                ingots.contains(
                    &host
                        .item_key(&Identifier::parse("mod_a:bronze_ingot").unwrap())
                        .unwrap()
                )
            );

            let TagMembers::Members(water) = tags.fluids(c::fluids::WATER) else {
                panic!("c:water fluid tag must bind");
            };
            assert!(water.contains(&host.fluid_key("minecraft:water").unwrap()));

            let TagMembers::Members(bosses) = tags.entity_types(c::entity_types::BOSSES) else {
                panic!("c:bosses entity_type tag must bind");
            };
            let boss_names: Vec<_> = bosses.iter().filter_map(|key| key.id(host)).collect();
            assert!(
                boss_names
                    .iter()
                    .any(|name| name.as_str() == "minecraft:ender_dragon")
            );
            assert!(
                boss_names
                    .iter()
                    .any(|name| name.as_str() == "minecraft:wither")
            );

            let TagMembers::Members(forests) = tags.biomes(c::biomes::IS_FOREST) else {
                panic!("c:is_forest biome tag must bind");
            };
            let forest_names: Vec<_> = forests
                .iter()
                .filter_map(|key| key.view(host).ok().map(|view| view.id()))
                .collect();
            assert!(forest_names.contains(&"minecraft:forest"));

            let TagMembers::Members(weapon_enchants) =
                tags.enchantments(c::enchantments::WEAPON_DAMAGE_ENHANCEMENTS)
            else {
                panic!("c:weapon_damage_enhancements enchantment tag must bind");
            };
            let enchant_names: Vec<_> = weapon_enchants
                .iter()
                .filter_map(|key| key.id(host))
                .collect();
            assert!(
                enchant_names
                    .iter()
                    .any(|name| name.as_str() == "minecraft:sharpness")
            );

            assert!(matches!(
                tags.structures(c::structures::HIDDEN_FROM_DISPLAYERS),
                TagMembers::Members(_)
            ));
            let village_tag = tags
                .key(&Identifier::parse("minecraft:village").unwrap())
                .unwrap()
                .typed::<StructureKey>();
            let TagMembers::Members(villages) = tags.structures(village_tag) else {
                panic!("minecraft:village structure tag must bind");
            };
            assert!(villages.contains(&StructureKey(
                Identifier::parse("minecraft:village_plains").unwrap()
            )));

            assert!(matches!(
                tags.potions(c::potions::HIDDEN_FROM_RECIPE_VIEWERS),
                TagMembers::Members(_)
            ));

            let TagMembers::Members(magic_damage) = tags.damage_types(c::damage_types::IS_MAGIC)
            else {
                panic!("neoforge:is_magic damage_type tag must bind");
            };
            assert!(magic_damage.contains(&Identifier::parse("minecraft:magic").unwrap()));
            assert_eq!(
                tags.damage_type_in_tag("minecraft:magic", c::damage_types::IS_MAGIC),
                Some(true)
            );

            let clock_tag = tags
                .key(&Identifier::parse("mod_a:primary").unwrap())
                .unwrap()
                .typed::<WorldClockKey>();
            let TagMembers::Members(primary_clocks) = tags.world_clocks(clock_tag) else {
                panic!("mod_a:primary world_clock tag must bind");
            };
            assert!(primary_clocks.contains(&WorldClockKey(
                Identifier::parse("minecraft:overworld").unwrap()
            )));
        });
    });
}

/// Every built-in convention tag across all nine NeoForge convention-tagged registries binds
/// cleanly against vanilla 26.3 with zero unresolved required entries, zero cycles, and complete
/// API key resolution.
#[test]
#[ignore = "daily"]
fn all_builtin_convention_tags_bind_cleanly_against_26_3() {
    if run_fresh_cases(
        "game::server_side::native_convention_tags_tests::all_builtin_convention_tags_bind_cleanly_against_26_3",
        "SUNRISE_TEST_CONVENTION_TAGS_INTEGRITY",
        &["integrity"],
    ) {
        return;
    }
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        let save = tempfile::tempdir().unwrap();
        let mut request = crate::game::world_loading::WorldRequest::new(
            "c_tags_integrity",
            None,
            Some(1),
            true,
            None,
            Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
        )
        .unwrap();
        request.save_root = save.path().to_owned();
        game.load_world(request).unwrap();

        let host: &dyn Host = game;
        let tags = host.tags();
        let all = convention::all();
        assert_eq!(all.len(), 9, "expected all 9 convention tag registries");

        let mut total_checked = 0usize;
        for (registry, entries) in all {
            for entry in entries {
                let tag_id = &entry.id;
                let parsed = Identifier::parse(tag_id)
                    .unwrap_or_else(|e| panic!("invalid convention tag id {tag_id}: {e}"));
                let key = tags
                    .key(&parsed)
                    .unwrap_or_else(|| panic!("could not resolve TagKey for {tag_id}"));
                let ok = match registry.as_str() {
                    "block" => {
                        matches!(tags.blocks(key.typed::<BlockKey>()), TagMembers::Members(_))
                    }
                    "item" => {
                        matches!(tags.items(key.typed::<ItemKey>()), TagMembers::Members(_))
                    }
                    "fluid" => {
                        matches!(tags.fluids(key.typed::<FluidKey>()), TagMembers::Members(_))
                    }
                    "entity_type" => matches!(
                        tags.entity_types(key.typed::<EntityTypeKey>()),
                        TagMembers::Members(_)
                    ),
                    "worldgen/biome" => {
                        matches!(tags.biomes(key.typed::<BiomeKey>()), TagMembers::Members(_))
                    }
                    "enchantment" => matches!(
                        tags.enchantments(key.typed::<EnchantmentKey>()),
                        TagMembers::Members(_)
                    ),
                    "worldgen/structure" => matches!(
                        tags.structures(key.typed::<StructureKey>()),
                        TagMembers::Members(_)
                    ),
                    "potion" => {
                        matches!(
                            tags.potions(key.typed::<PotionKey>()),
                            TagMembers::Members(_)
                        )
                    }
                    "damage_type" => matches!(
                        tags.damage_types(key.typed::<DamageTypeKey>()),
                        TagMembers::Members(_)
                    ),
                    other => panic!("unexpected convention tag registry {other}"),
                };
                assert!(
                    ok,
                    "convention tag {tag_id} in registry {registry} failed to bind cleanly against 26.3"
                );
                total_checked += 1;
            }
        }
        assert_eq!(
            total_checked, 540,
            "expected all 540 NeoForge convention tags, checked {total_checked}"
        );

        // Spot-check const-built typed constants match dynamic TagKey lookup across all 9 modules.
        assert_eq!(
            c::blocks::ORES.untyped(),
            tags.key(&Identifier::parse("c:ores").unwrap()).unwrap()
        );
        assert_eq!(
            c::items::INGOTS.untyped(),
            tags.key(&Identifier::parse("c:ingots").unwrap()).unwrap()
        );
        assert_eq!(
            c::fluids::WATER.untyped(),
            tags.key(&Identifier::parse("c:water").unwrap()).unwrap()
        );
        assert_eq!(
            c::entity_types::BOSSES.untyped(),
            tags.key(&Identifier::parse("c:bosses").unwrap()).unwrap()
        );
        assert_eq!(
            c::biomes::IS_FOREST.untyped(),
            tags.key(&Identifier::parse("c:is_forest").unwrap())
                .unwrap()
        );
        assert_eq!(
            c::enchantments::WEAPON_DAMAGE_ENHANCEMENTS.untyped(),
            tags.key(&Identifier::parse("c:weapon_damage_enhancements").unwrap())
                .unwrap()
        );
        assert_eq!(
            c::structures::HIDDEN_FROM_DISPLAYERS.untyped(),
            tags.key(&Identifier::parse("c:hidden_from_displayers").unwrap())
                .unwrap()
        );
        assert_eq!(
            c::potions::HIDDEN_FROM_RECIPE_VIEWERS.untyped(),
            tags.key(&Identifier::parse("c:hidden_from_recipe_viewers").unwrap())
                .unwrap()
        );
        assert_eq!(
            c::damage_types::IS_MAGIC.untyped(),
            tags.key(&Identifier::parse("neoforge:is_magic").unwrap())
                .unwrap()
        );

        // `GameLevel` exposes biome tags through the same bound BiomeKeys snapshot.
        let level_forest_biomes =
            game.with_level(|_, level, _| mod_api::Tags::biomes(level, c::biomes::IS_FOREST));
        assert!(
            matches!(level_forest_biomes, TagMembers::Members(ref m) if !m.is_empty()),
            "expected GameLevel::biomes(c::biomes::IS_FOREST) to return Members, got {level_forest_biomes:?}"
        );
    });
}
