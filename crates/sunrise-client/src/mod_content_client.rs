//! Client-only mod-content asset hooks (fluid visuals and client pack merging).
use crate::{Result, mod_content::ContentMod};
use std::path::{Path, PathBuf};

/// Remembers how the mods' fluids look (`data/<id>/sunrise/fluid/`) and publishes client models.
pub fn set_fluid_looks(mods: &[ContentMod]) {
    crate::mod_content::set_fluid_looks(mods);
    crate::mod_content::with_fluid_looks(|looks| {
        crate::fluid_models::set_world_looks(looks.values());
    });
}

/// Open passive layers for a client load, merging mod asset layers through `packs::resources`.
pub fn resolved_resources(
    base: &Path,
    prepared: PathBuf,
    jar_store: &Path,
    mods: &[ContentMod],
    cache: PathBuf,
) -> Result<sunrise_assets::resource_source::ResourceRoot> {
    crate::mod_content::resolved_resources_with(
        base,
        prepared,
        jar_store,
        mods,
        cache,
        |assets_base, layer_sources| {
            let layers: Vec<_> = layer_sources
                .into_iter()
                .map(|(id, source)| {
                    crate::packs::Pack::mod_layer(
                        id,
                        sunrise_level::pack_meta::Source::Opened(source),
                    )
                })
                .collect();
            crate::packs::resources(assets_base, &layers)
        },
    )
}

#[cfg(test)]
#[path = "mod_content_item_metadata_tests.rs"]
mod item_metadata_tests;

#[cfg(test)]
mod tests {
    use super::item_metadata_tests;
    use super::resolved_resources;
    use crate::{
        assets::Assets,
        blocks::BlockFacts,
        crafting::CraftingRemainder,
        items::Items,
        mod_content::{
            ContentMod, biome_modifiers_over_packs, resolved_server_resources,
            test_fixtures::{
                biome_base, build_fixture, content_files, folder_mod, manifest, prepared,
                relative_base, resource_root_fixture, ruby_mod_with_pistons, small_base, zinc_mod,
            },
        },
    };
    use std::{collections::BTreeMap, fs, path::Path};
    use sunrise_mod_package::{content::Content, manifest::JarRef};
    use sunrise_world::{loot::PackOverlay, registry::BlockStateId};

    /// A Java jar's assets are a pack layer under the mod's own files, in mod
    /// order under the later mods. Passive assets stay in the jar, and the files
    /// every pack stacks (atlases here) merge across vanilla, the jars and the
    /// mods in that same order.
    #[test]
    fn a_mods_jars_layer_under_its_files_in_mod_order() {
        use std::io::Write as _;
        let scratch = std::env::temp_dir().join(format!("sunrise-mod-jars-{}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        let base = scratch.join("base");
        small_base(&base);
        // `build` reads the base's registry tables even for a plan with no
        // blocks: a minimal facts pair satisfies it without the full export.
        fs::write(
            base.join("facts.json"),
            r#"{"states": [["minecraft:stone"]], "defaults": {"minecraft:stone": 0}}"#,
        )
        .unwrap();
        fs::write(
            base.join("gameplay-facts.json"),
            r#"{"states": [{"item": "minecraft:stone"}]}"#,
        )
        .unwrap();
        let vanilla_path = "assets/minecraft/textures/block/vanilla_path.png";
        fs::create_dir_all(base.join(vanilla_path).parent().unwrap()).unwrap();
        fs::write(base.join(vanilla_path), b"vanilla").unwrap();
        fs::create_dir_all(base.join("assets/minecraft/atlases")).unwrap();
        fs::write(
            base.join("assets/minecraft/atlases/blocks.json"),
            r#"{"sources": [{"type": "directory", "source": "block", "prefix": "block/"}]}"#,
        )
        .unwrap();
        let atlas = |source: &str| {
            format!(
                "{{\"sources\": [{{\"type\": \"directory\", \"source\": \"{source}\", \"prefix\": \"\"}}]}}"
            )
        };
        // A jar in the store as the launcher names it: `<sha512>.jar`, with
        // entries in any namespace (a jar is not fenced as a mod's files are).
        let jar = |name: &str, entries: &[(&str, String)]| {
            let sha512 = format!("{}{}", "1".repeat(127), name.chars().last().unwrap());
            let file = scratch.join(format!("jars/{sha512}.jar"));
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            let mut zip = zip::ZipWriter::new(fs::File::create(&file).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            for (entry, text) in entries
                .iter()
                .chain(std::iter::once(&("Example.class", "dummy".to_owned())))
            {
                zip.start_file(*entry, options).unwrap();
                zip.write_all(text.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
            JarRef::new(name.into(), "1.0".into(), sha512).unwrap()
        };
        let ported = |dir: &str, id: &str, files: &[(&str, String)], jars: Vec<JarRef>| {
            let mut m = folder_mod(
                &scratch.join(dir),
                &files
                    .iter()
                    .map(|(name, text)| (*name, text.as_str()))
                    .chain(std::iter::once((
                        "sunrise-mod.toml",
                        manifest(id, "").as_str(),
                    )))
                    .collect::<Vec<_>>(),
            );
            m.jars = jars;
            m
        };
        fn own<'a>(path: &'a str, text: &str) -> (&'a str, String) {
            (path, text.to_owned())
        }
        let atlas_file = |source: &str| ("assets/minecraft/atlases/blocks.json", atlas(source));
        let jar_a = jar(
            "porteda",
            &[
                own("assets/alpha/own_and_jar.png", "jar-a"),
                own("assets/alpha/own_only.png", "jar-a"),
                own(vanilla_path, "jar-a-mc"),
                atlas_file("jar_a"),
            ],
        );
        let jar_b = jar(
            "portedb",
            &[
                own("assets/alpha/own_and_jar.png", "jar-b"),
                own("assets/beta/only_jar.png", "jar-b-only"),
                atlas_file("jar_b"),
            ],
        );
        let alpha = ported(
            "alpha",
            "alpha",
            &[
                own("assets/alpha/own_and_jar.png", "mod-a"),
                own("assets/alpha/own_only.png", "mod-a"),
                atlas_file("mod_a"),
            ],
            vec![jar_a.clone()],
        );
        let beta = ported(
            "beta",
            "beta",
            &[own("assets/beta/own.png", "mod-b"), atlas_file("mod_b")],
            vec![jar_b.clone()],
        );
        let root = scratch.join("root");
        let mods = [alpha, beta];
        let warnings = build_fixture(&base, &scratch.join("jars"), &mods, &root).unwrap();
        let resources = resolved_resources(
            &base,
            root.clone(),
            &scratch.join("jars"),
            &mods,
            scratch.join("cache"),
        )
        .unwrap();
        assert!(!root.join("assets/alpha").exists());
        assert!(!root.join("assets/beta").exists());
        let mut listed = warnings.clone();
        listed.sort();
        assert_eq!(
            listed,
            [
                "mod alpha: jar porteda also provides assets/alpha/own_and_jar.png; the mod's own file wins",
                "mod alpha: jar porteda also provides assets/alpha/own_only.png; the mod's own file wins",
            ]
        );
        // A mod's own files win over its jars; a later mod's jar wins over an
        // earlier mod's own file; a jar wins over vanilla.
        assert_eq!(
            resources
                .source()
                .read("assets/alpha/own_only.png")
                .unwrap(),
            b"mod-a"
        );
        assert_eq!(
            resources
                .source()
                .read("assets/alpha/own_and_jar.png")
                .unwrap(),
            b"jar-b"
        );
        assert_eq!(resources.source().read(vanilla_path).unwrap(), b"jar-a-mc");
        assert_eq!(
            resources.source().read("assets/beta/only_jar.png").unwrap(),
            b"jar-b-only"
        );
        // The stacked files merge across vanilla, the jars and the mods in the
        // same order, and no root Java bytecode leaves the jar.
        let sources: Vec<_> = serde_json::from_slice::<serde_json::Value>(
            &resources
                .source()
                .read("assets/minecraft/atlases/blocks.json")
                .unwrap(),
        )
        .unwrap()["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["source"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(sources, ["block", "jar_a", "mod_a", "jar_b", "mod_b"]);
        assert!(!root.join("Example.class").exists());
        // A mod without its own files still layers its jar, under an earlier
        // mod's own files.
        let root = scratch.join("root2");
        let mods = [
            ported(
                "alpha2",
                "alpha",
                &[own("assets/alpha/own.png", "mod-a")],
                vec![jar_a],
            ),
            ported("beta-empty", "beta", &[], vec![jar_b]),
        ];
        build_fixture(&base, &scratch.join("jars"), &mods, &root).unwrap();
        let resources = resolved_resources(
            &base,
            root.clone(),
            &scratch.join("jars"),
            &mods,
            scratch.join("cache"),
        )
        .unwrap();
        assert!(resources.source().contains("assets/alpha/own.png"));
        assert!(!root.join("assets/alpha").exists());
        assert_eq!(
            resources.source().read("assets/beta/only_jar.png").unwrap(),
            b"jar-b-only"
        );
        let _ = fs::remove_dir_all(&scratch);
    }

    /// A mod's `assets/minecraft/atlases/blocks.json` adds its sources after vanilla's.
    #[test]
    fn a_content_root_appends_a_mods_atlas_sources() {
        let Some(base) = prepared() else {
            return;
        };
        let scratch =
            std::env::temp_dir().join(format!("sunrise-mod-atlas-{}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        let m = folder_mod(
            &scratch.join("gauge"),
            &[
                ("sunrise-mod.toml", &manifest("gauge", "")),
                (
                    "assets/minecraft/atlases/blocks.json",
                    r#"{"sources": [{"type": "minecraft:directory", "source": "gauge_liquid", "prefix": "gauge_liquid/"}]}"#,
                ),
            ],
        );
        let root = scratch.join("root");
        build_fixture(&base, Path::new(""), std::slice::from_ref(&m), &root).unwrap();
        let resources = resolved_resources(
            &base,
            root.clone(),
            Path::new(""),
            std::slice::from_ref(&m),
            scratch.join("cache"),
        )
        .unwrap();
        let sources =
            |source: &sunrise_assets::resource_source::ResourceSource| -> Vec<serde_json::Value> {
                let text = source.read("assets/minecraft/atlases/blocks.json").unwrap();
                let doc: serde_json::Value = serde_json::from_slice(&text).unwrap();
                doc["sources"].as_array().unwrap().clone()
            };
        let (vanilla, built) = (
            sources(&sunrise_assets::resource_source::ResourceSource::directory(
                base.clone(),
            )),
            sources(resources.source()),
        );
        assert_eq!(built.len(), vanilla.len() + 1);
        assert_eq!(built[..vanilla.len()], vanilla[..]);
        assert_eq!(built[vanilla.len()]["source"], "gauge_liquid");
        let _ = fs::remove_dir_all(&scratch);
    }

    /// A mod's creative tab comes after vanilla's, on page 1 in the first top
    /// slot, with only the items the game has; its new items join the search tab
    /// under their English names.
    #[test]
    #[ignore = "daily"]
    fn creative_tabs_come_on_their_own_page() {
        let Some(base) = prepared() else {
            return;
        };
        let scratch = std::env::temp_dir().join(format!("sunrise-mod-tabs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        let dir = scratch.join("gems");
        for (name, text) in [
            (
                "sunrise-mod.toml",
                "id = \"gems\"\nname = \"Gems\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
            ),
            (
                "data/gems/sunrise/item/ruby.json",
                r#"{"template": "minecraft:diamond"}"#,
            ),
            (
                "data/gems/sunrise/block/ruby_block.json",
                r#"{"template": "minecraft:diamond_block"}"#,
            ),
            (
                "data/gems/sunrise/creative_tab/gems.json",
                r#"{"title": "Gems", "icon": "gems:ruby", "items": ["gems:ruby", "minecraft:diamond", "gems:missing"]}"#,
            ),
            (
                "assets/gems/lang/en_us.json",
                r#"{"item.gems.ruby": "Ruby"}"#,
            ),
        ] {
            let path = dir.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let package = sunrise_mod_package::Package::read(&dir).unwrap();
        let m = ContentMod {
            id: "gems".into(),
            content: Content::read(&dir, &package.manifest).unwrap(),
            jars: Vec::new(),
        };
        let relative = relative_base(&base);
        let root = resource_root_fixture(
            &relative,
            Path::new(""),
            std::slice::from_ref(&m),
            &scratch.join("cache"),
        )
        .unwrap();
        let mut creative = crate::creative::Creative::load(&root).unwrap();
        let lists = creative.catalog().build(
            sunrise_items::creative_catalog::Parameters {
                operator: false,
                features: &["minecraft:vanilla".into()],
                holders: &Default::default(),
                items: &crate::items::Items::empty(),
            },
            None,
            |_, _, _| {},
        );
        creative.publish(
            lists,
            Default::default(),
            Default::default(),
            &crate::item_names::Names::default(),
        );
        assert_eq!(creative.pages(), 2);
        let i = creative
            .tabs
            .iter()
            .position(|t| t.id == "gems:gems")
            .unwrap();
        let tab = &creative.tabs[i];
        assert_eq!(
            (tab.page, tab.top, tab.column, tab.title.as_str()),
            (1, true, 0, "Gems")
        );
        let items: Vec<&str> = tab.items.iter().map(|s| &*s.item).collect();
        assert_eq!(items, ["gems:ruby", "minecraft:diamond"]);
        assert!(
            creative
                .search_items()
                .iter()
                .any(|s| &*s.item == "gems:ruby")
        );
        // A block's item is in search once.
        let blocks = creative
            .search_items()
            .iter()
            .filter(|s| &*s.item == "gems:ruby_block");
        assert_eq!(blocks.count(), 1);
        // Search, on every page, stays selected over a page turn.
        let search = creative.tabs.iter().position(|t| t.search).unwrap();
        creative.select(search);
        creative.set_page(1);
        assert_eq!((creative.selected_index(), creative.page), (search, 1));
        creative.select(0);
        // Page 1 shows the mod's tab and the right-hand ones, not vanilla's.
        creative.set_page(1);
        assert_eq!(creative.selected_index(), i);
        let shown: Vec<&str> = creative
            .tabs
            .iter()
            .filter(|t| creative.on_page(t))
            .map(|t| t.id.as_str())
            .collect();
        assert!(
            shown.contains(&"gems:gems") && shown.contains(&"minecraft:search"),
            "{shown:?}"
        );
        assert!(!shown.contains(&"minecraft:building_blocks"), "{shown:?}");
        // Selecting a vanilla tab goes back to its page.
        creative.select(0);
        assert_eq!(creative.page, 0);
        fs::remove_dir_all(&scratch).unwrap();
    }

    /// A sign's four forms come from its template, whatever the blocks are named,
    /// its wood from its declaration, and its item places the wall form it names
    /// and is that form's item too. A `road_sign` of stone is no sign.
    #[test]
    #[ignore = "daily"]
    fn signs_take_their_form_wood_and_wall_block_from_their_declaration() {
        use sunrise_world::level::{Behaviours, ItemKind};

        let Some(base) = prepared() else {
            return;
        };
        let scratch =
            std::env::temp_dir().join(format!("sunrise-mod-signs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        let m = folder_mod(
            &scratch.join("signs"),
            &[
                ("sunrise-mod.toml", &manifest("signs", "")),
                (
                    "data/signs/sunrise/block/notice.json",
                    r#"{"template": "minecraft:oak_sign", "wood_type": "signs:elm", "wall_block": "signs:board"}"#,
                ),
                (
                    "data/signs/sunrise/block/board.json",
                    r#"{"template": "minecraft:oak_wall_sign", "wood_type": "signs:elm", "item": false}"#,
                ),
                (
                    "data/signs/sunrise/block/lantern.json",
                    r#"{"template": "minecraft:spruce_hanging_sign", "wall_block": "signs:bracket"}"#,
                ),
                (
                    "data/signs/sunrise/block/bracket.json",
                    r#"{"template": "minecraft:spruce_wall_hanging_sign", "wood_type": "cherry", "item": false}"#,
                ),
                (
                    "data/signs/sunrise/block/road_sign.json",
                    r#"{"template": "minecraft:stone"}"#,
                ),
            ],
        );
        let root = resource_root_fixture(
            &base,
            Path::new(""),
            std::slice::from_ref(&m),
            &scratch.join("cache"),
        )
        .unwrap();
        let assets = Assets::load(&root).unwrap();
        let registry = assets.registry;
        let facts = BlockFacts::load(&root, registry).unwrap();
        let behaviours = Behaviours::load(registry, &root).unwrap();
        let block = |name: &str| facts.get(registry.default_state(name).unwrap());
        let form = |hanging, wall| Some(crate::blocks::SignForm { hanging, wall });
        assert_eq!(block("signs:notice").sign_form(), form(false, false));
        assert_eq!(block("signs:board").sign_form(), form(false, true));
        assert_eq!(block("signs:lantern").sign_form(), form(true, false));
        assert_eq!(block("signs:bracket").sign_form(), form(true, true));
        assert_eq!(block("signs:road_sign").sign_form(), None);
        assert_eq!(
            block("minecraft:oak_wall_sign").sign_form(),
            form(false, true)
        );
        // The declared wood, or the template's.
        assert_eq!(facts.wood_type("signs:notice"), Some("signs:elm"));
        assert_eq!(facts.wood_type("signs:bracket"), Some("cherry"));
        assert_eq!(facts.wood_type("signs:lantern"), Some("spruce"));
        assert_eq!(
            facts.wood_type("minecraft:dark_oak_wall_sign"),
            Some("dark_oak")
        );
        assert_eq!(facts.wood_type("signs:road_sign"), None);
        // Each item places its sign and the wall form it names, which it is the item of.
        let id = |name: &str| behaviours.table.block_by_name(name);
        let notice = behaviours.item_binding("signs:notice").unwrap();
        assert_eq!(notice.kind, ItemKind::StandingAndWall);
        assert_eq!(
            (Some(notice.block), notice.wall_block),
            (id("signs:notice"), id("signs:board"))
        );
        let lantern = behaviours.item_binding("signs:lantern").unwrap();
        assert_eq!(lantern.kind, ItemKind::HangingSign);
        assert_eq!(
            (Some(lantern.block), lantern.wall_block),
            (id("signs:lantern"), id("signs:bracket"))
        );
        assert_eq!(&*block("signs:board").item, "signs:notice");
        assert_eq!(&*block("signs:bracket").item, "signs:lantern");
        let mut renderer = crate::block_entity_render::BlockEntityRenderer::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            registry,
        )
        .unwrap();
        renderer.add_mod_signs(&facts, registry);
        for name in [
            "signs:notice",
            "signs:board",
            "signs:lantern",
            "signs:bracket",
        ] {
            assert!(
                renderer.renders(registry.default_state(name).unwrap().0),
                "{name}"
            );
        }
        assert!(!renderer.renders(registry.default_state("signs:road_sign").unwrap().0));
        // Vanilla's pair stays as it was.
        let oak = behaviours.item_binding("minecraft:oak_sign").unwrap();
        assert_eq!(oak.wall_block, id("minecraft:oak_wall_sign"));
        let _ = fs::remove_dir_all(&scratch);
    }

    /// `sunrise:modify_biome` writes ambient particles, ambient sounds and music as the vanilla
    /// attribute values (the shapes of `warped_forest.json`), each replacing the biome's value.
    #[test]
    fn modify_biome_sets_ambient_particles_sounds_and_music() {
        let base = biome_base("biome-modifiers-modify-effects");
        let scratch = base.parent().unwrap().to_owned();
        fs::write(
            base.join("registries.json"),
            r#"{"minecraft:item":{"entries":{"minecraft:test":{"protocol_id":0}}},"minecraft:block":{"entries":{"minecraft:test":{"protocol_id":0}}},"minecraft:sound_event":{"entries":{"minecraft:ambient.warped_forest.loop":{"protocol_id":0},"minecraft:ambient.warped_forest.additions":{"protocol_id":1},"minecraft:ambient.warped_forest.mood":{"protocol_id":2}}}}"#,
        )
        .unwrap();
        let m = zinc_mod(
            &scratch.join("zinc"),
            &[(
                "dunes",
                r#"{"type":"sunrise:modify_biome","biomes":"minecraft:plains","attributes":{
                    "minecraft:visual/ambient_particles":{"argument":[{"particle":{"type":"minecraft:ash"},"probability":0.25}]},
                    "minecraft:audio/ambient_sounds":{"loop":"minecraft:ambient.warped_forest.loop",
                        "additions":{"sound":"minecraft:ambient.warped_forest.additions","tick_chance":1.5},
                        "mood":{"sound":"minecraft:ambient.warped_forest.mood","tick_delay":6000,"block_search_extent":8,"offset":2.0}},
                    "minecraft:audio/background_music":{"default":{"sound":{"sound_id":"zinc:music.dunes"},"min_delay":25000,"max_delay":24000}},
                    "minecraft:audio/music_volume":0.5}}"#,
            )],
        );
        let out = scratch.join("out");
        let warnings = build_fixture(&base, &base.join("jars"), &[m], &out).unwrap();
        assert!(
            warnings.iter().all(|w| !w.contains("skipped")),
            "{warnings:?}"
        );
        let after: BTreeMap<String, String> =
            serde_json::from_slice(&fs::read(out.join("worldgen/biomes.json")).unwrap()).unwrap();
        let plains: serde_json::Value = serde_json::from_str(&after["minecraft:plains"]).unwrap();
        let attributes = &plains["attributes"];
        assert_eq!(
            attributes["minecraft:visual/ambient_particles"],
            serde_json::json!([{"particle": {"type": "minecraft:ash"}, "probability": 0.25}])
        );
        assert_eq!(
            attributes["minecraft:audio/ambient_sounds"],
            serde_json::json!({
                "loop": "minecraft:ambient.warped_forest.loop",
                "additions": {"sound": "minecraft:ambient.warped_forest.additions", "tick_chance": 1.5},
                "mood": {"sound": "minecraft:ambient.warped_forest.mood", "tick_delay": 6000,
                    "block_search_extent": 8, "offset": 2.0},
            })
        );
        assert_eq!(
            attributes["minecraft:audio/background_music"],
            serde_json::json!({"default": {"sound": {"sound_id": "zinc:music.dunes"}, "min_delay": 25000, "max_delay": 24000}})
        );
        assert_eq!(attributes["minecraft:audio/music_volume"], 0.5);

        // The modified biome flows through playback selection with its custom sound and volume.
        let builtin = std::sync::Arc::new(
            sunrise_protocol::registry::Builtin::load(&base.join("registries.json")).unwrap(),
        );
        let music_tables = crate::music::MusicTables::load(&out, builtin.clone());
        let (bg, vol) = music_tables.at("minecraft:overworld", "minecraft:plains", Some(&plains));
        let selected = crate::music::Situation {
            screen_music: None,
            in_world: true,
            in_end: false,
            boss_music: false,
            background: &bg,
            probe_volume: vol,
            creative: false,
            underwater: false,
        };
        assert_eq!(
            selected.music().map(|m| m.sound),
            Some("zinc:music.dunes".to_owned())
        );
        assert_eq!(selected.volume(), 0.5);

        // Typed ModifyBiome declared in code emits inline sound events that load and play identically.
        let typed = mod_api::BiomeModifier {
            biomes: vec!["minecraft:desert".into()],
            operation: mod_api::BiomeModifierOperation::ModifyBiome {
                temperature: None,
                downfall: None,
                has_precipitation: None,
                temperature_modifier: None,
                sky_color: None,
                fog_color: None,
                water_color: None,
                water_fog_color: None,
                foliage_color: None,
                dry_foliage_color: None,
                grass_color: None,
                ambient_particles: None,
                ambient_loop: None,
                ambient_additions: None,
                ambient_mood: None,
                background_music: Some(mod_api::BiomeMusic {
                    sound: "zinc:music.oasis".into(),
                    min_delay: 100,
                    max_delay: 200,
                }),
                music_volume: Some(0.25),
            },
        };
        let declared = [[
            "zinc".to_owned(),
            "data/zinc/neoforge/biome_modifier/oasis.json".to_owned(),
            typed.json(),
        ]];
        let packs = sunrise_world::worldgen::overrides::WorldgenOverrides::default();
        let overridden = biome_modifiers_over_packs(&out, &packs, &declared)
            .unwrap()
            .expect("typed modifier produces biome overrides");
        let desert_text = overridden
            .entries("biomes")
            .find_map(|(id, text)| (id == "minecraft:desert").then_some(text))
            .expect("desert override present");
        let desert: serde_json::Value = serde_json::from_str(desert_text).unwrap();
        let (desert_bg, desert_vol) =
            music_tables.at("minecraft:overworld", "minecraft:desert", Some(&desert));
        assert_eq!(
            desert_bg.default.map(|m| m.sound),
            Some("zinc:music.oasis".to_owned())
        );
        assert_eq!(desert_vol, 0.25);

        fs::remove_dir_all(scratch).unwrap();
    }

    /// The overlay over the real prepared resources: every loader reads it, the
    /// new blocks behave like their templates with the overrides, and vanilla's
    /// states are where they were.
    #[test]
    #[ignore = "daily"]
    fn content_root_loads_with_every_table() {
        let Some(base) = prepared() else {
            return;
        };
        let scratch =
            std::env::temp_dir().join(format!("sunrise-mod-content-{}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        let m = ruby_mod_with_pistons(&scratch.join("ruby"), true);
        // The base as a relative path (cargo runs tests in the crate's folder):
        // the overlay's links still resolve.
        let relative = relative_base(&base);
        let root = resource_root_fixture(
            &relative,
            Path::new(""),
            std::slice::from_ref(&m),
            &scratch.join("cache"),
        )
        .unwrap();
        assert_ne!(root, base);
        let resources = resolved_server_resources(
            &base,
            root.clone(),
            Path::new(""),
            std::slice::from_ref(&m),
            scratch.join("cache"),
        )
        .unwrap();

        let files = content_files(&resources);
        let overlay = PackOverlay::new(&files);
        let vanilla = Assets::load(&base).unwrap();
        let assets = Assets::load(&root).unwrap();
        let (old, new) = (vanilla.registry, assets.registry);
        let n = old.len() as u32;
        let iron = old.default_state("minecraft:iron_block").unwrap();
        let door = old.default_state("minecraft:oak_door").unwrap();
        let door_first = (0..n)
            .map(BlockStateId)
            .find(|&s| old.state(s).unwrap().name == "minecraft:oak_door")
            .unwrap();
        // Vanilla keeps its ids; blocks come after, sorted: ruby_block, ruby_door, ruby_glass.
        assert_eq!(new.len(), old.len() + 1 + 64 + 1 + 2);
        assert_eq!(new.default_state("minecraft:iron_block"), Some(iron));
        let ruby = new.default_state("ruby:ruby_block").unwrap();
        assert_eq!(ruby, BlockStateId(n));
        let ruby_door = new.default_state("ruby:ruby_door").unwrap();
        assert_eq!(ruby_door, BlockStateId(n + 1 + (door.0 - door_first.0)));
        assert_eq!(
            new.default_state("ruby:ruby_glass"),
            Some(BlockStateId(n + 65))
        );
        // Network ids: the new items and blocks follow vanilla's in the registries report, which
        // the game and its server both read (a missing one travels as air).
        let report = |root: &Path| {
            sunrise_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap()
        };
        let (vanilla_ids, ids) = (report(&base), report(&root));
        let item = |b: &sunrise_protocol::registry::Builtin, name: &str| {
            b.get("minecraft:item").and_then(|r| r.id(name))
        };
        let vanilla_items = (0..)
            .take_while(|&i| vanilla_ids.get("minecraft:item").unwrap().name(i).is_some())
            .count() as i32;
        assert_eq!(
            item(&ids, "minecraft:stone"),
            item(&vanilla_ids, "minecraft:stone")
        );
        let ruby_item = item(&ids, "ruby:ruby_block").unwrap();
        assert!(ruby_item >= vanilla_items, "{ruby_item} < {vanilla_items}");
        assert_eq!(
            ids.get("minecraft:item").unwrap().name(ruby_item),
            Some("ruby:ruby_block")
        );
        assert!(item(&ids, "ruby:ruby_glass").is_none(), "no item declared");
        assert!(
            ids.get("minecraft:block")
                .unwrap()
                .id("ruby:ruby_glass")
                .is_some()
        );

        let old_facts = BlockFacts::load(&base, old).unwrap();
        let facts = BlockFacts::load(&root, new).unwrap();
        let shared_json: serde_json::Value = serde_json::from_reader(std::io::BufReader::new(
            fs::File::open(root.join("block-behaviours.json")).unwrap(),
        ))
        .unwrap();
        let (shared_facts, shared_behaviours) =
            BlockFacts::load_with_behaviours(&root, new, &shared_json).unwrap();
        let independent_behaviours = sunrise_world::level::Behaviours::load(new, &root).unwrap();
        let (deferred_facts, gameplay) =
            BlockFacts::load_with_projection(&root, new, &shared_json, true).unwrap();
        let deferred_behaviours = sunrise_world::level::Behaviours::load_with_gameplay(
            new,
            &root,
            &shared_json,
            &gameplay,
        )
        .unwrap();
        assert_eq!(shared_facts.states.len(), facts.states.len());
        // `random_ticks` makes the block tick randomly although its template (iron) never does; the
        // sections' counts read the same flag (`random_tick_candidate`).
        assert!(independent_behaviours.is_randomly_ticking(ruby));
        assert!(independent_behaviours.random_tick_candidate(ruby));
        assert!(!independent_behaviours.is_randomly_ticking(iron));
        assert!(deferred_behaviours.is_randomly_ticking(ruby));
        for state in (n + 1)..(n + 65) {
            assert_eq!(
                independent_behaviours.facts(BlockStateId(state)).push,
                sunrise_world::level::PushReaction::Immoveable
            );
        }
        assert_eq!(
            independent_behaviours.facts(ruby).push,
            independent_behaviours.facts(iron).push
        );
        // `occludes: false` (noOcclusion): the iron-template block occludes nothing, iron still does.
        assert!(!facts.get(ruby).can_occlude && !facts.get(ruby).solid_render);
        // Its light dampening is a non-solid full block's, as leaves' and spawners' are (iron's is 15).
        assert_eq!(
            (facts.get(ruby).dampening, facts.get(iron).dampening),
            (1, 15)
        );
        assert!(facts.get(iron).can_occlude && facts.get(iron).solid_render);
        // `destroy_overlay: false` hides the crack on the declared block only.
        assert!(!facts.get(ruby).destroy_overlay && facts.get(iron).destroy_overlay);
        let definitions: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("render-definitions.json")).unwrap())
                .unwrap();
        assert_eq!(
            definitions["ruby:ruby_block"]["can_occlude"], false,
            "faces beside it are drawn"
        );
        assert_eq!(definitions["minecraft:iron_block"]["can_occlude"], true);
        // `render_shape` leaves the glass's model out of the chunk mesh; the block keeps the template's.
        assert_eq!(
            facts.get(BlockStateId(n + 65)).render_shape,
            crate::blocks::RenderShape::Animated
        );
        assert_eq!(
            facts.get(ruby).render_shape,
            crate::blocks::RenderShape::Model
        );
        for state in [ruby, ruby_door, BlockStateId(n + 65)] {
            assert_eq!(shared_facts.get(state).hardness, facts.get(state).hardness);
            assert_eq!(
                deferred_facts.get(state).hardness,
                facts.get(state).hardness
            );
            assert_eq!(deferred_facts.get(state).classes, facts.get(state).classes);
            assert_eq!(
                deferred_behaviours.facts(state).flags,
                independent_behaviours.facts(state).flags
            );
            assert_eq!(
                deferred_behaviours.facts(state).placement_sound,
                independent_behaviours.facts(state).placement_sound
            );
            assert_eq!(shared_facts.get(state).emission, facts.get(state).emission);
            assert_eq!(shared_facts.get(state).classes, facts.get(state).classes);
            assert_eq!(
                shared_behaviours.facts(state).flags,
                independent_behaviours.facts(state).flags,
            );
            assert_eq!(
                shared_behaviours.facts(state).placement_sound,
                independent_behaviours.facts(state).placement_sound,
            );
        }

        let (a, b) = (old_facts.get(iron), facts.get(ruby));
        assert_eq!((b.hardness, b.emission, b.friction), (4.0, 3, 0.8));
        assert_eq!(b.explosion_resistance, a.explosion_resistance);
        assert_eq!(b.outline, a.outline);
        assert_eq!(b.classes, a.classes);
        assert_eq!(&*b.item, "ruby:ruby_block");
        assert_eq!(b.map_color, 28);
        let amethyst = old_facts.get(old.default_state("minecraft:amethyst_block").unwrap());
        assert_eq!(b.sounds.break_sound, amethyst.sounds.break_sound);
        for i in 0..64 {
            let (v, m) = (
                old_facts.get(BlockStateId(door_first.0 + i)),
                facts.get(BlockStateId(n + 1 + i)),
            );
            assert_eq!(
                (v.outline.clone(), v.hardness, v.classes.clone()),
                (m.outline.clone(), m.hardness, m.classes.clone())
            );
            assert_eq!(&*m.name, "ruby:ruby_door");
        }
        let glass = facts.get(BlockStateId(n + 65));
        assert_eq!(&*glass.item, "minecraft:air");
        // Vanilla's own rows are untouched.
        assert_eq!(facts.get(iron).hardness, a.hardness);
        assert_eq!(&*facts.get(iron).name, "minecraft:iron_block");

        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = Items::load(&root, new, &facts, &overlay, &defaults).unwrap();
        item_metadata_tests::assert_metadata(&root, &items, &defaults, ruby, n);
        let binding = independent_behaviours.item_binding("ruby:ruby").unwrap();
        assert_eq!(
            binding.block,
            independent_behaviours.block(BlockStateId(n + 65))
        );
        // Exercise the same binding as server placement, including its emitted sound.
        {
            use sunrise_core::{
                geometry::{Direction, Vec3},
                position::BlockPos,
                random::LegacyRandom,
            };
            use sunrise_world::level::{
                LevelAccess, LevelEvent, PlaceContext, PlaceOutcome, Ticks,
            };
            struct TestLevel {
                blocks: std::collections::HashMap<BlockPos, BlockStateId>,
                air: BlockStateId,
                random: LegacyRandom,
                events: Vec<LevelEvent>,
                time: i64,
            }
            impl sunrise_world::level::BlockEntityNeighbors for TestLevel {}
            impl LevelAccess for TestLevel {
                fn block_state(&self, pos: BlockPos) -> BlockStateId {
                    *self.blocks.get(&pos).unwrap_or(&self.air)
                }
                fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
                    Some(self.blocks.insert(pos, state).unwrap_or(self.air))
                }
                fn is_loaded(&self, _: BlockPos) -> bool {
                    true
                }
                fn random(&mut self) -> &mut LegacyRandom {
                    &mut self.random
                }
                fn game_time(&self) -> i64 {
                    self.time
                }
                fn emit(&mut self, event: LevelEvent) {
                    self.events.push(event);
                }
            }
            let floor = BlockPos::new(0, 0, 0);
            let wheat = new.default_state("minecraft:wheat").unwrap();
            let mut level = TestLevel {
                blocks: [(floor, iron)].into_iter().collect(),
                air: new.default_state("minecraft:air").unwrap(),
                random: LegacyRandom::new(1),
                events: Vec::new(),
                time: 0,
            };
            let ctx = PlaceContext {
                clicked_pos: floor,
                face: Direction::Up,
                hit: Vec3::new(0.5, 1.0, 0.5),
                inside: false,
                source: sunrise_world::level::PlacementSource::Player {
                    yaw: 0.0,
                    pitch: 0.0,
                    sneaking: false,
                },
                item: "ruby:ruby_sword".into(),
                can_use_game_master_blocks: false,
            };
            let denied = independent_behaviours.place_block(&mut level, &mut Ticks::new(1), &ctx);
            assert!(matches!(denied, PlaceOutcome::Failed));
            assert!(level.events.is_empty());
            level
                .blocks
                .insert(floor, new.default_state("minecraft:farmland").unwrap());
            let placed = independent_behaviours.place_block(&mut level, &mut Ticks::new(1), &ctx);
            assert!(matches!(placed, PlaceOutcome::Placed { state, .. } if state == wheat));
            assert!(level.events.iter().any(|e| matches!(e, LevelEvent::Sound { sound, .. } if &**sound == "minecraft:item.crop.plant")));

            // Exercise the real piston caller with both stone-based content blocks.
            let piston = new
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:piston".into(),
                    properties: [
                        ("facing".into(), "east".into()),
                        ("extended".into(), "false".into()),
                    ]
                    .into_iter()
                    .collect(),
                })
                .unwrap();
            let power = new.default_state("minecraft:redstone_block").unwrap();
            for (name, moves) in [("ruby:z_stone", true), ("ruby:z_stuck", false)] {
                let block = new.default_state(name).unwrap();
                let base = BlockPos::new(0, 1, 0);
                let front = BlockPos::new(1, 1, 0);
                let destination = BlockPos::new(2, 1, 0);
                let mut level = TestLevel {
                    blocks: Default::default(),
                    air: new.default_state("minecraft:air").unwrap(),
                    random: LegacyRandom::new(1),
                    events: Vec::new(),
                    time: 0,
                };
                let mut ticks = Ticks::new(0);
                for x in -4..4 {
                    for z in -4..4 {
                        ticks
                            .add_empty_chunk(sunrise_core::position::ChunkPos::new(x, z))
                            .unwrap();
                    }
                }
                for (pos, state) in [
                    (base, piston),
                    (front, block),
                    (BlockPos::new(-1, 1, 0), power),
                ] {
                    independent_behaviours.set_block(
                        &mut level,
                        &mut ticks,
                        pos,
                        state,
                        sunrise_world::level::flags::UPDATE_ALL,
                    );
                }
                for _ in 0..4 {
                    level.time += 1;
                    independent_behaviours.tick_scheduled(&mut level, &mut ticks);
                    independent_behaviours.run_block_events(&mut level, &mut ticks);
                    independent_behaviours.tick_block_entities(&mut level, &mut ticks);
                }
                assert_eq!(
                    new.state(level.block_state(base)).unwrap().properties["extended"],
                    if moves { "true" } else { "false" }
                );
                assert_eq!(
                    level.block_state(if moves { destination } else { front }),
                    block
                );
            }
        }
        let sword = &items.items["ruby:ruby_sword"];
        assert_eq!(sword.block, new.default_state("minecraft:wheat"));
        assert_eq!(items.items["minecraft:iron_block"].block, Some(iron));
        let iron_sword = &items.items["minecraft:iron_sword"];
        assert_eq!((sword.max_damage, iron_sword.max_damage), (900, 250));
        assert_eq!(sword.modifiers.len(), 1);
        assert_eq!(sword.modifiers[0].amount, 7.0);
        assert_eq!(sword.tool.len(), iron_sword.tool.len());
        // The stew leaves its bowl in the grid; vanilla's stew leaves none.
        let stew = &items.items["ruby:ruby_stew"];
        assert_eq!(stew.crafting_remainder.as_deref(), Some("minecraft:bowl"));
        assert_eq!(
            items.items["minecraft:mushroom_stew"].crafting_remainder,
            None
        );
        let recipes = crate::crafting::Recipes::load(
            &root,
            &items,
            &overlay,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let grid = [
            Some(crate::items::ItemStack::new("ruby:ruby_stew", 1)),
            None,
            None,
            None,
        ];
        let (id, result) = recipes
            .find(crate::test_resources::server_loop_context(), &grid, 2)
            .unwrap();
        assert_eq!(&*result.item, "ruby:ruby");
        let rest = recipes.remainders(
            crate::test_resources::server_loop_context(),
            id,
            &grid,
            2,
            &|s| items.crafting_remainder(s),
        );
        assert_eq!(rest[0].as_ref().map(|s| &*s.item), Some("minecraft:bowl"));
        assert!(!items.items.contains_key("ruby:ruby_glass"));
        let pickaxe = &items.block_tags["minecraft:mineable/pickaxe"];
        assert!(pickaxe.contains("ruby:ruby_block"));
        assert!(!pickaxe.contains("ruby:left_out") && !pickaxe.contains("othermod:thing"));
        assert!(pickaxe.contains("minecraft:stone"));
        // Convention tags load, with the ids nobody registers dropped.
        assert_eq!(items.item_tag_order["c:gems/ruby"], ["ruby:ruby"]);

        crate::collisions::load(&root, new).unwrap();
        crate::models::load_visibility(&root, new).unwrap();
        crate::light::LightTables::load(&root, new, &facts).unwrap();
        crate::motion::MotionResources::load(&root, &assets).unwrap();
        sunrise_world::worldgen::generator::GeneratorData::load(&root.join("worldgen"), new)
            .unwrap();
        assert!(resources.source().contains("assets/ruby/lang/en_us.json"));
        // Blocks without a blockstate of their own take their template's.
        assert_eq!(
            resources
                .source()
                .read("assets/ruby/blockstates/ruby_door.json")
                .unwrap(),
            fs::read(base.join("assets/minecraft/blockstates/oak_door.json")).unwrap()
        );

        // The base root was only read.
        assert!(
            fs::symlink_metadata(base.join("facts.json"))
                .unwrap()
                .is_file()
        );
        fs::remove_dir_all(&scratch).unwrap();
    }

    /// Load the actual generated tables as the renderer and world generator do.
    #[test]
    #[ignore = "daily"]
    fn declared_shapes_reach_render_and_worldgen_occlusion() {
        use crate::{
            chunk::Chunk,
            light::{LightTables, apply_lighting, light_chunk},
        };
        use sunrise_core::geometry::Direction;
        use sunrise_world::{
            block_properties::{BlockPropertiesRegistry, BlockProperty},
            collision::CachedCollisionRegistry,
            registry::BlockTableFile,
        };
        let Some(base) = prepared() else { return };
        let scratch =
            std::env::temp_dir().join(format!("sunrise-shape-occlusion-{}", std::process::id()));
        let manifest = manifest("testmod", "");
        let m = folder_mod(
            &scratch.join("mod"),
            &[
                ("sunrise-mod.toml", &manifest),
                (
                    "data/testmod/sunrise/block/full.json",
                    r#"{"template":"minecraft:stone","item":false,"shapes":{"default":[[0,0,0,1,1,1]]}}"#,
                ),
                (
                    "data/testmod/sunrise/block/partial.json",
                    r#"{"template":"minecraft:stone","item":false,"shapes":{"default":[[0,0,0,1,0.5,1]]}}"#,
                ),
                (
                    "data/testmod/sunrise/block/open.json",
                    r#"{"template":"minecraft:stone","item":false,"properties":{"occludes":false},"shapes":{"default":[[0,0,0,1,1,1]]}}"#,
                ),
                (
                    "data/testmod/sunrise/block/partial_open.json",
                    r#"{"template":"minecraft:stone","item":false,"properties":{"occludes":false},"shapes":{"default":[[0,0,0,1,0.5,1]]}}"#,
                ),
                (
                    "data/testmod/sunrise/block/bubble.json",
                    r#"{"template":"minecraft:bubble_column","item":false,"shapes":{"default":[]}}"#,
                ),
                (
                    "data/testmod/sunrise/block/kelp.json",
                    r#"{"template":"minecraft:kelp","item":false,"shapes":{"default":[[0,0,0,1,0.5625,1]]}}"#,
                ),
            ],
        );
        let root =
            resource_root_fixture(&base, Path::new(""), &[m], &scratch.join("cache")).unwrap();
        let assets = Assets::load(&root).unwrap();
        let registry = assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let full = registry.default_state("testmod:full").unwrap();
        let partial = registry.default_state("testmod:partial").unwrap();
        let open = registry.default_state("testmod:open").unwrap();
        let facts = BlockFacts::load(&root, registry).unwrap();
        assert_eq!(
            (facts.get(full).solid_render, facts.get(full).dampening),
            (true, 15)
        );
        assert_eq!(
            (
                facts.get(partial).solid_render,
                facts.get(partial).dampening
            ),
            (false, 0)
        );
        assert_eq!(
            (facts.get(open).solid_render, facts.get(open).dampening),
            (false, 1)
        );
        // Compare against the actual vanilla exports, including the lower slab's
        // partial outline and leaves' full, non-occluding shape.
        let slab = registry.default_state("minecraft:stone_slab").unwrap();
        let leaves = registry.default_state("minecraft:oak_leaves").unwrap();
        for (content, vanilla) in [(full, stone), (partial, slab), (open, leaves)] {
            assert_eq!(
                (
                    facts.get(content).solid_render,
                    facts.get(content).dampening,
                    facts.get(content).skylight_down
                ),
                (
                    facts.get(vanilla).solid_render,
                    facts.get(vanilla).dampening,
                    facts.get(vanilla).skylight_down
                ),
            );
        }
        // Intrinsic water is present without a waterlogged property. Identical
        // declared outlines must retain the template's exported light facts.
        for (content, template) in [
            ("testmod:bubble", "minecraft:bubble_column"),
            ("testmod:kelp", "minecraft:kelp"),
        ] {
            let vanilla = facts.get(registry.default_state(template).unwrap());
            let declared = facts.get(registry.default_state(content).unwrap());
            assert_eq!(declared.outline, vanilla.outline);
            for state in [vanilla, declared] {
                assert!(!state.waterlogged);
                assert_eq!((state.skylight_down, state.dampening), (false, 1));
            }
        }
        let tables = LightTables::load(&root, registry, &facts).unwrap();
        let partial_open = registry.default_state("testmod:partial_open").unwrap();
        for (roof, below) in [(partial, 0), (slab, 0), (partial_open, 15)] {
            // A sealed room with a bottom-half roof: its open top admits sky,
            // but only occluding shapes stop it at the covered bottom face.
            let mut chunk = Chunk::empty(0, 0);
            for x in 4..=12 {
                for z in 4..=12 {
                    chunk.set(x, -1, z, stone.0);
                    chunk.set(x, 3, z, roof.0);
                    if x == 4 || x == 12 || z == 4 || z == 12 {
                        for y in 0..3 {
                            chunk.set(x, y, z, stone.0);
                        }
                    }
                }
            }
            let mut region = [[None; 3]; 3];
            region[1][1] = Some(&chunk);
            let lights = light_chunk(&region, &tables, true);
            apply_lighting(&mut chunk, lights);
            assert_eq!(chunk.light(8, 3, 8) >> 4, 15, "open roof top {roof:?}");
            assert_eq!(chunk.light(8, 2, 8) >> 4, below, "roof bottom {roof:?}");
        }
        let mut visibility = crate::models::load_visibility(&root, registry).unwrap();
        for (content, vanilla) in [(full, stone), (partial, slab), (open, leaves)] {
            assert_eq!(
                visibility
                    .should_render_face(stone, content, Direction::East, false)
                    .unwrap(),
                visibility
                    .should_render_face(stone, vanilla, Direction::East, false)
                    .unwrap(),
            );
        }
        assert!(
            !visibility
                .should_render_face(stone, full, Direction::East, false)
                .unwrap()
        );
        assert!(
            visibility
                .should_render_face(stone, partial, Direction::East, false)
                .unwrap()
        );
        assert!(
            visibility
                .should_render_face(stone, open, Direction::East, false)
                .unwrap()
        );
        let collisions =
            BlockTableFile::open(&root.join("worldgen/blocks/cached-collision.nbt"), registry)
                .unwrap();
        let collisions =
            CachedCollisionRegistry::from_table(collisions.table("collision").unwrap()).unwrap();
        let properties =
            BlockTableFile::open(&root.join("worldgen/blocks/block-properties.nbt"), registry)
                .unwrap();
        let properties = BlockPropertiesRegistry::from_table(
            properties.table("properties").unwrap(),
            &collisions,
        )
        .unwrap();
        assert!(
            properties
                .property(full, BlockProperty::SolidRender)
                .unwrap()
        );
        assert!(
            !properties
                .property(partial, BlockProperty::SolidRender)
                .unwrap()
        );
        assert!(
            !properties
                .property(open, BlockProperty::SolidRender)
                .unwrap()
        );
        assert!(!properties.bind_can_occlude().contains(open));
        fs::remove_dir_all(scratch).unwrap();
    }

    /// Face rules declared in a content block's data reach the visibility table the mesher culls
    /// with, over the prepared game: `skip_rendering`, `hides_neighbor_face` and
    /// `supports_external_face_hiding` decide the face between two neighbours.
    #[test]
    #[ignore = "daily"]
    fn content_block_face_rules_reach_the_mesher_visibility() {
        use sunrise_core::geometry::Direction;
        let Some(base) = prepared() else {
            return;
        };
        let scratch =
            std::env::temp_dir().join(format!("sunrise-face-rules-{}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        let manifest_text = manifest("testmod", "");
        let m = folder_mod(
            &scratch.join("mod"),
            &[
                ("sunrise-mod.toml", manifest_text.as_str()),
                (
                    "data/testmod/sunrise/block/self_skip.json",
                    r#"{"template":"minecraft:glass","item":false,"properties":{"occludes":false,"skip_rendering":[{"neighbor":"self"}]}}"#,
                ),
                (
                    "data/testmod/sunrise/block/no_skip.json",
                    r#"{"template":"minecraft:glass","item":false,"properties":{"occludes":false,"skip_rendering":[]}}"#,
                ),
                (
                    "data/testmod/sunrise/block/hider.json",
                    r#"{"template":"minecraft:glass","item":false,"properties":{"occludes":false,"skip_rendering":[],"hides_neighbor_face":[{"neighbor":"testmod:no_skip"},{"neighbor":"testmod:opt_out"}]}}"#,
                ),
                (
                    "data/testmod/sunrise/block/opt_out.json",
                    r#"{"template":"minecraft:glass","item":false,"properties":{"occludes":false,"skip_rendering":[],"supports_external_face_hiding":false}}"#,
                ),
            ],
        );
        let root =
            resource_root_fixture(&base, Path::new(""), &[m], &scratch.join("cache")).unwrap();
        let assets = Assets::load(&root).unwrap();
        let registry = assets.registry;
        let mut visibility = crate::models::load_visibility(&root, registry).unwrap();
        let state = |id: &str| BlockStateId(registry.default_state(id).unwrap().0);
        let (self_skip, no_skip, hider, opt_out) = (
            state("testmod:self_skip"),
            state("testmod:no_skip"),
            state("testmod:hider"),
            state("testmod:opt_out"),
        );
        let mut shown = |a, b| {
            visibility
                .should_render_face(a, b, Direction::East, true)
                .unwrap()
        };
        assert!(
            !shown(self_skip, self_skip),
            "skip_rendering self hides the shared face"
        );
        assert!(
            shown(no_skip, no_skip),
            "an empty skip_rendering drops glass's same-block skip"
        );
        assert!(
            !shown(no_skip, hider),
            "hides_neighbor_face hides the neighbour's face"
        );
        assert!(
            shown(opt_out, hider),
            "supports_external_face_hiding: false keeps the face"
        );
        let _ = fs::remove_dir_all(&scratch);
    }
}
