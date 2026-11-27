use super::test_fixtures::*;
use super::*;
use crate::{assets::Assets, blocks::BlockFacts, items::Items};
use sunrise_mod_package::manifest::JarRef;
use sunrise_world::native_load_context::DataLoadOrigin;
#[path = "mod_content_block_entity_tests.rs"]
mod block_entity_tests;
#[path = "mod_content_farmland_tests.rs"]
mod farmland_tests;
#[path = "mod_content_jar_tests.rs"]
mod jar_tests;
#[path = "mod_content_waterlogged_tests.rs"]
mod waterlogged_tests;

/// An absent original refuses publication before either the tables or the
/// decoded pack root exists.
#[test]
fn a_missing_jar_refuses_the_whole_load_before_decoded_output() {
    let scratch =
        std::env::temp_dir().join(format!("sunrise-mod-jar-missing-{}", std::process::id()));
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
    let shared = "assets/alpha/textures/block/own.png";
    let mut m = folder_mod(
        &scratch.join("alpha"),
        &[
            ("sunrise-mod.toml", &manifest("alpha", "")),
            (shared, "own"),
        ],
    );
    m.jars = vec![JarRef::new("porteda".into(), "1.0".into(), "2".repeat(128)).unwrap()];
    let root = scratch.join("root");
    let error = build_fixture(
        &base,
        &scratch.join("jars"),
        std::slice::from_ref(&m),
        &root,
    )
    .unwrap_err();
    let missing = error
        .downcast_ref::<sunrise_mod_package::manifest::MissingJar>()
        .unwrap();
    assert_eq!(missing.owners, ["alpha"]);
    assert_eq!(missing.jar, m.jars[0]);
    assert_eq!(missing.path, m.jars[0].cache_path(&scratch.join("jars")));
    assert!(!root.exists());
    assert!(!scratch.join("root-tables").exists());
    let _ = fs::remove_dir_all(&scratch);
}

use std::sync::Arc;

#[test]
fn state_light_compiles_in_final_order_and_relights_across_a_chunk_boundary() {
    use crate::{
        chunk::Chunk,
        light::{self, LightChanges, LightTables},
    };
    use std::collections::HashMap;
    use sunrise_world::{
        level::GameplayStateSource,
        registry::{BlockRegistry, BlockStateId},
    };
    let declaration = r#"{"template":"minecraft:test","item":false,"properties":{"light":3},
      "state_properties":{"mode":{"type":"enum","values":["off","dim","bright"]}},
      "outline_context_shapes":{"item":{"properties":["mode"],"values":{
        "dim":[[0,0,0,0.5,1,1]]},"default":[]}},
      "state_light":{"properties":["mode","axis"],"values":{
        "dim,x":8,"dim,y":9,"bright,x":15,"bright,y":12},"default":0}}"#;
    let (base, mods) = block_base_of(
        "state-light-propagation",
        &[declaration],
        r#"{"states":[
      ["minecraft:air",false,false,"",{"id":"minecraft:air"}],
      ["minecraft:test",false,false,"",{"id":"minecraft:test","properties":{"axis":"x"}}],
      ["minecraft:test",false,false,"",{"id":"minecraft:test","properties":{"axis":"y"}}]],
      "defaults":{"minecraft:air":0,"minecraft:test":1}}"#,
    );
    let plan = Plan::new(&base, &mods).unwrap();
    let mut facts: J = read_json(&base.join("facts.json")).unwrap();
    let mut gameplay: J = read_json(&base.join("gameplay-facts.json")).unwrap();
    let Some(J::Arr(rows)) = gameplay.at("states") else {
        unreachable!()
    };
    for (i, row) in rows.iter_mut().enumerate() {
        row.set("id", J::Num(i.into()));
        row.set("emission", J::Num(if i == 0 { 0 } else { 3 }.into()));
    }
    plan.json("facts.json", &mut facts).unwrap();
    plan.json("gameplay-facts.json", &mut gameplay).unwrap();
    fs::write(
        base.join("gameplay-facts.json"),
        serde_json::to_vec(&gameplay).unwrap(),
    )
    .unwrap();
    let registry =
        BlockRegistry::from_facts_json(serde_json::to_vec(&facts).unwrap().as_slice()).unwrap();
    let blocks: BTreeSet<_> = (0..registry.len())
        .map(|i| {
            registry
                .state(BlockStateId(i as u32))
                .unwrap()
                .name
                .as_str()
        })
        .collect();
    let blocks: Vec<_> = blocks
        .iter()
        .map(|name| serde_json::json!({"name":name,"class":["Block"]}))
        .collect();
    let behaviours = serde_json::json!({
        "states": vec![serde_json::json!([0,0]);registry.len()],"blocks":blocks
    });
    let (facts, projection) =
        BlockFacts::load_with_projection(&base, &registry, &behaviours, false).unwrap();
    assert_eq!(
        facts.get(BlockStateId(1)).emission,
        3,
        "the template is unchanged"
    );
    let tables = LightTables::new(&facts);
    let mut by_state = BTreeMap::new();
    for i in 3..registry.len() as u32 {
        let state = registry.state(BlockStateId(i)).unwrap();
        let key = (
            state.properties["axis"].as_str(),
            state.properties["mode"].as_str(),
        );
        let expected = match key {
            (_, "off") => 0,
            ("x", "dim") => 8,
            ("y", "dim") => 9,
            ("x", "bright") => 15,
            ("y", "bright") => 12,
            _ => unreachable!(),
        };
        assert_eq!(facts.get(BlockStateId(i)).emission, expected);
        assert_eq!(
            projection[i as usize].unsigned("emission"),
            u64::from(expected)
        );
        assert_eq!(tables.emission(i), expected);
        let item_outline = &plan.blocks[0].context_shapes["outline_item"][(i - 3) as usize];
        assert_eq!(
            item_outline.bounds().map(|bounds| bounds.max(Axis::X)),
            (key.1 == "dim").then_some(0.5)
        );
        by_state.insert(key, i);
    }
    let off = by_state[&("x", "off")];
    let dim = by_state[&("x", "dim")];
    let bright = by_state[&("x", "bright")];
    assert!(tables.differs(dim, bright));
    assert!(tables.differs(bright, off));
    let mut world = HashMap::new();
    for z in -1..=1 {
        for x in -1..=1 {
            let mut chunk = Chunk::empty(x, z);
            if (x, z) == (0, 0) {
                chunk.set(15, 64, 8, dim);
            }
            world.insert((x, z), Arc::new(chunk));
        }
    }
    let lit: Vec<_> = world
        .keys()
        .map(|&(x, z)| {
            let region = std::array::from_fn(|dz| {
                std::array::from_fn(|dx| {
                    world
                        .get(&(x + dx as i32 - 1, z + dz as i32 - 1))
                        .map(AsRef::as_ref)
                })
            });
            let mut chunk = (*world[&(x, z)]).clone();
            light::apply_lighting(&mut chunk, light::light_chunk(&region, &tables, false));
            chunk.lit = true;
            ((x, z), Arc::new(chunk))
        })
        .collect();
    world = lit.into_iter().collect();
    let level = |world: &HashMap<(i32, i32), Arc<Chunk>>, x: i32| {
        world[&(x >> 4, 0)].light(x & 15, 64, 8) & 15
    };
    assert_eq!((level(&world, 15), level(&world, 16)), (8, 7));
    for (state, expected) in [(bright, 15u8), (off, 0u8)] {
        Arc::make_mut(world.get_mut(&(0, 0)).unwrap()).set(15, 64, 8, state);
        let mut changed = LightChanges::default();
        light::relight(
            &mut world,
            &tables,
            false,
            &[(15, 64, 8)],
            false,
            &mut changed,
        );
        assert_eq!(
            (level(&world, 15), level(&world, 16)),
            (expected, expected.saturating_sub(1))
        );
        assert!(!changed.is_empty());
    }
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

#[test]
fn state_light_rejects_unknown_unreachable_and_uncovered_state_tuples() {
    for (i, spec) in [
        r#"{"property":"missing","default":0}"#,
        r#"{"property":"axis","values":{"x":1}}"#,
        r#"{"property":"axis","values":{"x":1,"z":2},"default":0}"#,
    ]
    .iter()
    .enumerate()
    {
        let declaration = format!(r#"{{"template":"minecraft:test","state_light":{spec}}}"#);
        let (base, mods) = block_base(&format!("state-light-selection-{i}"), &[&declaration]);
        assert!(Plan::new(&base, &mods).is_err(), "{spec}");
        fs::remove_dir_all(base.parent().unwrap()).unwrap();
    }
    for (i, light) in ["", r#", "properties":{"light":4}"#].iter().enumerate() {
        let declaration = format!(r#"{{"template":"minecraft:test"{light}}}"#);
        let (base, mods) = block_base(&format!("state-light-absent-{i}"), &[&declaration]);
        let plan = Plan::new(&base, &mods).unwrap();
        let mut doc: J =
            serde_json::from_str(r#"{"states":[{"emission":6},{"emission":7}]}"#).unwrap();
        plan.json("gameplay-facts.json", &mut doc).unwrap();
        let expected = if i == 0 { vec![6, 7] } else { vec![4, 4] };
        let doc = serde_json::to_value(doc).unwrap();
        assert_eq!(doc["states"][2]["emission"], expected[0]);
        assert_eq!(doc["states"][3]["emission"], expected[1]);
        fs::remove_dir_all(base.parent().unwrap()).unwrap();
    }
}

#[test]
fn state_light_from_native_declarations_invalidates_the_content_cache() {
    let (base, mut mods) = block_base(
        "state-light-cache",
        &[r#"{"template":"minecraft:test","state_light":{"default":8}}"#],
    );
    fs::write(base.join("sunrise-resources.json"), b"{}").unwrap();
    let initial = stamp(&base, &base.join("jars"), &mods, &fixture_inventory(&mods)).unwrap();
    let declaration = &mut mods[0].content.blocks[0];
    declaration.state_light.as_mut().unwrap().default = Some(15);
    assert_ne!(
        stamp(&base, &base.join("jars"), &mods, &fixture_inventory(&mods)).unwrap(),
        initial
    );
    mods[0].content.blocks[0]
        .state_light
        .as_mut()
        .unwrap()
        .default = Some(8);
    assert_eq!(
        stamp(&base, &base.join("jars"), &mods, &fixture_inventory(&mods)).unwrap(),
        initial
    );
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

/// The prepared resources, a mod's files, and a game folder's link to them.
fn tag_files(values: &str) -> Vec<(&'static str, String)> {
    vec![
        ("sunrise-mod.toml", manifest("testmod", "")),
        (
            "data/testmod/tags/item/a.json",
            format!(r#"{{"values":[{values}]}}"#),
        ),
    ]
}

fn resources(dir: &Path) -> PathBuf {
    let base = dir.join("resources");
    small_base(&base);
    fs::write(base.join("sunrise-resources.json"), b"{}").unwrap();
    fs::write(
        base.join("facts.json"),
        r#"{"states":[["minecraft:stone"]],"defaults":{"minecraft:stone":0}}"#,
    )
    .unwrap();
    fs::write(
        base.join("gameplay-facts.json"),
        r#"{"states":[{"item":"minecraft:stone"}]}"#,
    )
    .unwrap();
    base
}

fn mod_in(dir: &Path, files: &[(&'static str, String)]) -> Vec<ContentMod> {
    let files: Vec<(&str, &str)> = files.iter().map(|(n, t)| (*n, t.as_str())).collect();
    vec![folder_mod(&dir.join("mod"), &files)]
}

#[test]
fn the_stamp_follows_a_mod_files_contents_not_its_folder_or_time() {
    let scratch = tempfile::tempdir().unwrap();
    let base = resources(scratch.path());
    let at = |dir: &str, files: &[(&'static str, String)]| {
        let mods = mod_in(&scratch.path().join(dir), files);
        stamp(&base, &base.join("jars"), &mods, &fixture_inventory(&mods)).unwrap()
    };
    let files = tag_files("");
    let first = at("one", &files);
    assert_eq!(first, at("two", &files), "the same files in another folder");
    let tag = scratch.path().join("one/mod/data/testmod/tags/item/a.json");
    fs::File::options()
        .write(true)
        .open(&tag)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
        .unwrap();
    assert_eq!(
        first,
        at("one", &files),
        "a newer file time, the same bytes"
    );
    assert_ne!(
        first,
        at("three", &tag_files(r#""minecraft:stone""#)),
        "one file's bytes differ"
    );
}

#[test]
fn an_admitted_archive_keys_resource_caches_by_bytes_not_entry_metadata() {
    use sunrise_mod_package::content::ContentSource;

    const FILE: &str = "data/testmod/custom/value.bin";
    // Equal length and CRC-32, but different bytes.
    const FIRST: &[u8] = b"\xba\x42\xc6\x9b\xb4\x3d\x52\xdd";
    const SECOND: &[u8] = b"\x63\xd6\x35\x1a\x43\xda\x3a\xd9";
    let dir = tempfile::tempdir().unwrap();
    let base = resources(dir.path());
    let cache = dir.path().join("cache");
    let jars = dir.path().join("jars");
    let path = dir.path().join("testmod.smod");
    let manifest_text = manifest("testmod", "");
    let manifest = sunrise_mod_package::Manifest::parse(&manifest_text).unwrap();
    let archive = |payload| {
        let bytes =
            sunrise_mod_package::Package::write_files(&manifest_text, &[(FILE, payload)]).unwrap();
        {
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
            let entry = zip.by_name(FILE).unwrap();
            assert_eq!((entry.crc32(), entry.size()), (0x5eab_a44f, 8));
        }
        bytes
    };
    let admit = || {
        let source = ContentSource::open(&path)
            .unwrap()
            .admit(&manifest)
            .unwrap();
        [ContentMod {
            id: "testmod".into(),
            content: Content::read_opened(&source, &manifest).unwrap(),
            jars: Vec::new(),
        }]
    };
    let build = |mods: &[ContentMod]| resource_root_fixture(&base, &jars, mods, &cache).unwrap();
    let fingerprint = |root, mods: &[ContentMod]| {
        resolved_server_resources(&base, root, &jars, mods, cache.clone())
            .unwrap()
            .mod_layers()[0]
            .fingerprint_under("data", &["custom"])
            .unwrap()
    };
    fs::write(&path, archive(FIRST)).unwrap();
    let first = admit();
    let original_root = build(&first);
    let original_fingerprint = fingerprint(original_root.clone(), &first);

    let replacement = dir.path().join("replacement.smod");
    fs::write(&replacement, archive(SECOND)).unwrap();
    fs::rename(replacement, &path).unwrap();
    assert_eq!(build(&first), original_root);
    assert_eq!(
        fingerprint(original_root.clone(), &first),
        original_fingerprint
    );

    let second = admit();
    let new_root = build(&second);
    assert_ne!(new_root, original_root);
    assert_ne!(fingerprint(new_root, &second), original_fingerprint);
}

/// Each game folder reaches the one set of prepared resources through its own link
/// (`.local/sunrise-client/26.3`), and keeps its own copy of the mods.
#[cfg(unix)]
fn game_folder(
    parent: &Path,
    name: &str,
    resources: &Path,
    files: &[(&'static str, String)],
) -> (PathBuf, Vec<ContentMod>) {
    let folder = parent.join(name);
    let link = folder.join(".local/sunrise-client/26.3");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(resources, &link).unwrap();
    (link, mod_in(&folder, files))
}

#[cfg(unix)]
#[test]
fn game_folders_linking_the_same_resources_share_a_stamp() {
    let scratch = tempfile::tempdir().unwrap();
    let real = resources(scratch.path());
    let files = tag_files("");
    let (one, mods_one) = game_folder(scratch.path(), "one", &real, &files);
    let (two, mods_two) = game_folder(scratch.path(), "two", &real, &files);
    let stamp_of = |base: &Path, mods: &[ContentMod]| {
        stamp(base, &base.join("jars"), mods, &fixture_inventory(mods)).unwrap()
    };
    assert_eq!(stamp_of(&one, &mods_one), stamp_of(&two, &mods_two));
    assert_eq!(stamp_of(&one, &mods_one), stamp_of(&real, &mods_one));
}

#[cfg(unix)]
#[test]
fn a_root_built_from_one_game_folder_is_reused_after_it_is_deleted() {
    let scratch = tempfile::tempdir().unwrap();
    let real = resources(scratch.path());
    let cache = scratch.path().join("cache");
    let files = tag_files("");
    let (one, mods_one) = game_folder(scratch.path(), "one", &real, &files);
    let first = resource_root_fixture(&one, &one.join("jars"), &mods_one, &cache).unwrap();
    let built = fs::metadata(first.join(STAMP)).unwrap().modified().unwrap();
    fs::remove_dir_all(scratch.path().join("one")).unwrap();
    let (two, mods_two) = game_folder(scratch.path(), "two", &real, &files);
    let second = resource_root_fixture(&two, &two.join("jars"), &mods_two, &cache).unwrap();
    assert_eq!(first, second, "the same root, not a rebuilt one");
    assert_eq!(
        built,
        fs::metadata(second.join(STAMP))
            .unwrap()
            .modified()
            .unwrap(),
        "its stamp wasn't written again"
    );
    assert_eq!(
        fs::read(second.join("facts.json")).unwrap(),
        fs::read(real.join("facts.json")).unwrap(),
        "its links reach the resources after the first folder is gone"
    );
    let loaded =
        resolved_server_resources(&two, second.clone(), &two.join("jars"), &mods_two, cache)
            .unwrap();
    assert!(loaded.source().read("items.json").is_ok());
}

/// A smoke test: concurrent starts on one cache end with one valid root. It doesn't
/// prove the lock is what kept them apart.
#[cfg(unix)]
#[test]
fn starts_preparing_one_root_at_once_all_get_the_same_valid_root() {
    let scratch = tempfile::tempdir().unwrap();
    let real = resources(scratch.path());
    let cache = scratch.path().join("cache");
    let files = tag_files("");
    let games: Vec<_> = ["one", "two", "three", "four"]
        .into_iter()
        .map(|name| game_folder(scratch.path(), name, &real, &files))
        .collect();
    let roots: Vec<PathBuf> = std::thread::scope(|scope| {
        let starts: Vec<_> = games
            .iter()
            .map(|(base, mods)| {
                let cache = &cache;
                scope.spawn(move || {
                    resource_root_fixture(base, &base.join("jars"), mods, cache).unwrap()
                })
            })
            .collect();
        starts.into_iter().map(|s| s.join().unwrap()).collect()
    });
    assert!(roots.iter().all(|root| root == &roots[0]));
    let loaded = resolved_server_resources(
        &games[0].0,
        roots[0].clone(),
        &games[0].0.join("jars"),
        &games[0].1,
        cache,
    )
    .unwrap();
    assert!(loaded.source().read("items.json").is_ok());
}

#[test]
#[ignore = "daily"]
fn block_aliases_keep_the_original_item_in_placement_context() {
    use std::{cell::RefCell, collections::HashMap};
    use sunrise_core::{
        geometry::{Direction, Vec3},
        position::BlockPos,
        random::LegacyRandom,
    };
    use sunrise_mod_package::content::{Block, Item};
    use sunrise_world::{
        level::{
            Behaviours, LevelAccess, LevelEvent, PlaceContext, PlaceOutcome, PlacementContext,
            Ticks,
        },
        loot::PackOverlay,
        registry::BlockStateId,
    };
    let Some(base) = std::env::var_os("SUNRISE_TEST_RESOURCES")
        .map(PathBuf::from)
        .or_else(prepared)
    else {
        return;
    };
    let scratch =
        std::env::temp_dir().join(format!("sunrise-alias-context-{}", std::process::id()));
    let mut content = Content::declared("ruby");
    content
        .declare(
            &[Block {
                id: "ruby:frame".into(),
                template: "minecraft:stone".into(),
                item: false,
                ..Default::default()
            }],
            &["empty", "filled"].map(|name| Item {
                id: format!("ruby:{name}"),
                template: "minecraft:stick".into(),
                block: Some("ruby:frame".into()),
                ..Default::default()
            }),
        )
        .unwrap();
    build_fixture(
        &base,
        Path::new(""),
        &[ContentMod {
            id: "ruby".into(),
            content,
            jars: Vec::new(),
        }],
        &scratch,
    )
    .unwrap();
    let registry = sunrise_world::level::data::load_registry(&scratch).unwrap();
    let facts = BlockFacts::load(&scratch, &registry).unwrap();
    let defaults = sunrise_world::item_defaults::read(&scratch).unwrap();
    let items = Items::load(&scratch, &registry, &facts, &PackOverlay::NONE, &defaults).unwrap();
    let b = Behaviours::load(&registry, &scratch).unwrap();
    let target = registry.default_state("ruby:frame").unwrap();
    struct Level {
        air: BlockStateId,
        blocks: HashMap<BlockPos, BlockStateId>,
        random: LegacyRandom,
        asked: RefCell<Vec<String>>,
        client: bool,
    }
    impl sunrise_world::level::BlockEntityNeighbors for Level {}
    impl LevelAccess for Level {
        fn block_state(&self, p: BlockPos) -> BlockStateId {
            self.blocks.get(&p).copied().unwrap_or(self.air)
        }
        fn set_raw(&mut self, p: BlockPos, s: BlockStateId) -> Option<BlockStateId> {
            Some(self.blocks.insert(p, s).unwrap_or(self.air))
        }
        fn is_loaded(&self, _: BlockPos) -> bool {
            true
        }
        fn random(&mut self) -> &mut LegacyRandom {
            &mut self.random
        }
        fn game_time(&self) -> i64 {
            0
        }
        fn emit(&mut self, _: LevelEvent) {}
        fn is_client_side(&self) -> bool {
            self.client
        }
        fn before_state_for_placement(
            &self,
            _: &sunrise_world::level::Ticks,
            _: Option<&sunrise_world::level::PreviewBlocks>,
            _: BlockStateId,
            c: &PlacementContext,
            _: sunrise_world::fluid::FluidState,
            _: &mut Option<Option<BlockStateId>>,
        ) {
            self.asked.borrow_mut().push(c.item.clone());
        }
    }
    for client in [false, true] {
        let mut level = Level {
            air: facts.air,
            blocks: HashMap::new(),
            random: LegacyRandom::new(1),
            asked: RefCell::default(),
            client,
        };
        for (i, name) in ["ruby:empty", "ruby:filled"].iter().enumerate() {
            assert_eq!(items.get(name).unwrap().block, Some(target));
            assert_eq!(b.item_binding(name).unwrap().block, b.block(target));
            let pos = BlockPos::new(i as i32 * 2, 70, 0);
            let ctx = PlaceContext {
                clicked_pos: pos,
                face: Direction::Up,
                hit: Vec3::new(f64::from(pos.x) + 0.5, 70.0, 0.5),
                inside: false,
                source: sunrise_world::level::PlacementSource::Player {
                    yaw: 0.0,
                    pitch: 0.0,
                    sneaking: false,
                },
                item: (*name).into(),
                can_use_game_master_blocks: false,
            };
            assert!(
                matches!(b.place_block(&mut level,&mut Ticks::default(),&ctx),PlaceOutcome::Placed{state,..} if state==target)
            );
            assert_eq!(level.asked.borrow().last().map(String::as_str), Some(*name));
        }
    }
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn block_compiles_ordinary_bindings_without_changing_components() {
    use sunrise_mod_package::content::Item;
    let (base, mut mods) = block_base(
        "place-block-bindings",
        &[r#"{"template":"minecraft:test","item":false}"#],
    );
    let item = |id: &str, target: Option<&str>| Item {
        id: id.into(),
        template: "minecraft:test".into(),
        block: target.map(str::to_owned),
        ..Default::default()
    };
    mods[0]
        .content
        .declare(
            &[],
            &[
                item("testmod:empty", Some("testmod:test")),
                item("testmod:filled", Some("testmod:test")),
                item("testmod:vanilla", Some("minecraft:test")),
                item("testmod:plain", None),
            ],
        )
        .unwrap();
    let plan = Plan::new(&base, &mods).unwrap();
    let mut behaviours:J=serde_json::from_str(r#"{"states":[{},{}],"blocks":[],"items":{"minecraft:test":{"class":["Item"],"fields":{}}}}"#).unwrap();
    plan.json("block-behaviours.json", &mut behaviours).unwrap();
    let behaviours = serde_json::to_value(behaviours).unwrap();
    for name in ["empty", "filled", "vanilla"] {
        let binding = &behaviours["items"][format!("testmod:{name}")];
        assert_eq!(binding["class"], serde_json::json!(["BlockItem", "Item"]));
        assert_eq!(
            binding["fields"]["block"],
            if name == "vanilla" {
                "minecraft:test"
            } else {
                "testmod:test"
            }
        );
    }
    assert_eq!(
        behaviours["items"]["testmod:plain"]["class"],
        serde_json::json!(["Item"])
    );
    let mut components:J=serde_json::from_str(r#"{"minecraft:test":{"minecraft:max_stack_size":16,"minecraft:item_model":"minecraft:test"}}"#).unwrap();
    plan.json("items.json", &mut components).unwrap();
    let components = serde_json::to_value(components).unwrap();
    assert_eq!(components["testmod:empty"]["minecraft:max_stack_size"], 16);
    assert_eq!(
        components["testmod:empty"]["minecraft:item_model"],
        "testmod:empty"
    );
    drop(plan);
    for target in ["testmod:missing", "testmod:plain", "minecraft:missing"] {
        mods[0].content.items[0].block = Some(target.into());
        assert!(
            Plan::new(&base, &mods)
                .err()
                .unwrap()
                .to_string()
                .contains("not a block")
        );
    }
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

#[test]
fn renames_ids_keys_and_states() {
    let mut r = Rename::new("minecraft:iron_block", "ruby:ruby_block");
    r.states = 10..12;
    r.first = 100;
    assert_eq!(
        r.string("minecraft:iron_block").as_deref(),
        Some("ruby:ruby_block")
    );
    assert_eq!(
        r.string("Block{minecraft:iron_block}").as_deref(),
        Some("Block{ruby:ruby_block}")
    );
    assert_eq!(
        r.string("block.minecraft.iron_block").as_deref(),
        Some("block.ruby.ruby_block")
    );
    assert_eq!(r.string("minecraft:iron_blocks"), None);
    assert_eq!(r.string("block.minecraft.iron_bars"), None);
    assert_eq!(
        (r.id(9), r.id(10), r.id(11), r.id(12)),
        (None, Some(100), Some(101), None)
    );
}

#[test]
fn json_keeps_key_order_and_numbers() {
    let text = r#"{"z":1,"a":[0.6,5.0,-1,{"m":null,"b":true}],"k":"v"}"#;
    let doc: J = serde_json::from_str(text).unwrap();
    assert_eq!(serde_json::to_string(&doc).unwrap(), text);
}

#[test]
fn item_components_patch_the_template_entry() {
    let mut entry: J =
        serde_json::from_str(r#"{"minecraft:max_damage": 250, "minecraft:repairable": {}}"#)
            .unwrap();
    let known: BTreeSet<String> = [
        "minecraft:max_damage",
        "minecraft:repairable",
        "minecraft:food",
    ]
    .map(str::to_owned)
    .into();
    let patch = |text: &str| -> Vec<(String, J)> {
        let J::Obj(o) = serde_json::from_str(text).unwrap() else {
            unreachable!()
        };
        o
    };
    patch_components(
        &mut entry,
        &patch(r#"{"minecraft:max_damage": 900, "!minecraft:repairable": {}, "minecraft:food": {"nutrition": 4}}"#),
        &known,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&entry).unwrap(),
        r#"{"minecraft:max_damage":900,"minecraft:food":{"nutrition":4}}"#
    );
    for typo in [r#"{"minecraft:fod": {}}"#, r#"{"!minecraft:fod": {}}"#] {
        let e = patch_components(&mut entry, &patch(typo), &known).unwrap_err();
        assert_eq!(e, "minecraft:fod is not a known component type");
    }
}

#[test]
fn creative_tabs_keep_vanilla_items_missing_from_search() {
    let tab = sunrise_mod_package::content::CreativeTab {
        id: "test:restricted".into(),
        title: "Restricted".into(),
        icon: "minecraft:barrier".into(),
        items: [
            "minecraft:barrier",
            "minecraft:command_block",
            "minecraft:written_book",
            "minecraft:knowledge_book",
        ]
        .map(str::to_owned)
        .into(),
    };
    let plan = Plan {
        blocks: Vec::new(),
        items: Vec::new(),
        vanilla_items: tab.items.iter().cloned().collect(),
        effects: Vec::new(),
        fluids: Vec::new(),
        entity_types: Vec::new(),
        block_entity_types: Vec::new(),
        block_entity_factories: BTreeMap::new(),
        menu_types: Vec::new(),
        states: 0,
        defaults: BTreeMap::new(),
        sounds: BTreeMap::new(),
        tabs: vec![&tab],
        names: BTreeMap::new(),
    };
    let mut doc: J = serde_json::from_str(
        r#"{"schema":4,"item_features":{},"potion_features":{},"tabs":[{"id":"minecraft:search","title":"Search Items","type":"SEARCH","build":[],"search_metadata":[]}]}"#,
    )
    .unwrap();
    plan.creative_tabs(&mut doc).unwrap();
    let catalog =
        sunrise_items::creative_catalog::Catalog::parse(&serde_json::to_value(doc).unwrap())
            .unwrap();
    let built = catalog.build(
        sunrise_items::creative_catalog::Parameters {
            features: &[],
            operator: false,
            holders: &Default::default(),
            items: &crate::items::Items::empty(),
        },
        None,
        |_, _, _| {},
    );
    assert_eq!(catalog.tabs()[1].id, "test:restricted");
    assert_eq!(
        built[1]
            .display
            .iter()
            .map(|stack| stack.item.as_ref())
            .collect::<Vec<_>>(),
        [
            "minecraft:barrier",
            "minecraft:command_block",
            "minecraft:written_book",
            "minecraft:knowledge_book"
        ],
    );
}

/// A block item's patch goes over its template block's item, and a type no
/// vanilla item has fails the build.
#[test]
fn block_items_patch_their_template_item() {
    let scratch = std::env::temp_dir().join(format!("sunrise-block-item-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let base = scratch.join("base");
    fs::create_dir_all(&base).unwrap();
    fs::write(
        base.join("facts.json"),
        r#"{"states": [["minecraft:iron_block"]], "defaults": {"minecraft:iron_block": 0}}"#,
    )
    .unwrap();
    fs::write(
        base.join("gameplay-facts.json"),
        r#"{"states": [{"item": "minecraft:iron_block"}]}"#,
    )
    .unwrap();
    let items = r##"{"minecraft:iron_block": {"minecraft:max_stack_size": 64,
        "minecraft:item_name": {"translate": "block.minecraft.iron_block"}},
        "minecraft:netherite_block": {"minecraft:damage_resistant": {"types": "#minecraft:is_fire"}}}"##;
    fs::write(base.join("items.json"), items).unwrap();
    let block = |dir: &str, components: &str| {
        folder_mod(
            &scratch.join(dir),
            &[
                ("sunrise-mod.toml", &manifest("ruby", "")),
                (
                    "data/ruby/sunrise/block/ruby_block.json",
                    &format!(
                        r#"{{"template": "minecraft:iron_block", "components": {components}}}"#
                    ),
                ),
            ],
        )
    };
    let mods = [block(
        "ruby",
        r##"{"minecraft:max_stack_size": 16, "minecraft:damage_resistant": {"types": "#minecraft:is_fire"}}"##,
    )];
    let mut doc: J = serde_json::from_str(items).unwrap();
    Plan::new(&base, &mods)
        .unwrap()
        .json("items.json", &mut doc)
        .unwrap();
    let doc: serde_json::Value = serde_json::to_value(&doc).unwrap();
    assert_eq!(
        doc["ruby:ruby_block"],
        serde_json::json!({"minecraft:max_stack_size": 16,
            "minecraft:item_name": {"translate": "block.ruby.ruby_block"},
            "minecraft:damage_resistant": {"types": "#minecraft:is_fire"}})
    );
    assert_eq!(doc["minecraft:iron_block"]["minecraft:max_stack_size"], 64);
    let mods = [block("typo", r#"{"minecraft:fod": {}}"#)];
    let mut doc: J = serde_json::from_str(items).unwrap();
    let e = Plan::new(&base, &mods)
        .unwrap()
        .json("items.json", &mut doc)
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "ruby:ruby_block: minecraft:fod is not a known component type"
    );
    fs::remove_dir_all(scratch).unwrap();
}

/// A block's item takes the block's name key by default (`useBlockDescriptionPrefix`, as
/// vanilla's `Items.registerBlock`), and its own `item.<namespace>.<path>` with
/// `"use_block_description_prefix": false` (#5574); a name the patch sets wins either way.
#[test]
fn a_block_item_takes_its_own_name_key_when_the_block_says_so() {
    let scratch = std::env::temp_dir().join(format!("sunrise-item-name-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let base = scratch.join("base");
    fs::create_dir_all(&base).unwrap();
    fs::write(
        base.join("facts.json"),
        r#"{"states": [["minecraft:iron_block"]], "defaults": {"minecraft:iron_block": 0}}"#,
    )
    .unwrap();
    fs::write(
        base.join("gameplay-facts.json"),
        r#"{"states": [{"item": "minecraft:iron_block"}]}"#,
    )
    .unwrap();
    let items = r#"{"minecraft:iron_block": {"minecraft:max_stack_size": 64,
        "minecraft:item_name": {"translate": "block.minecraft.iron_block"}}}"#;
    fs::write(base.join("items.json"), items).unwrap();
    let name = |dir: &str, extra: &str| {
        let mods = [folder_mod(
            &scratch.join(dir),
            &[
                ("sunrise-mod.toml", &manifest("bridges", "")),
                (
                    "data/bridges/sunrise/block/oak/log_bridge.json",
                    &format!(r#"{{"template": "minecraft:iron_block"{extra}}}"#),
                ),
            ],
        )];
        let mut doc: J = serde_json::from_str(items).unwrap();
        Plan::new(&base, &mods)
            .unwrap()
            .json("items.json", &mut doc)
            .unwrap();
        let doc: serde_json::Value = serde_json::to_value(&doc).unwrap();
        doc["bridges:oak/log_bridge"]["minecraft:item_name"].clone()
    };
    let translate = |key: &str| serde_json::json!({"translate": key});
    assert_eq!(
        name("default", ""),
        translate("block.bridges.oak.log_bridge")
    );
    assert_eq!(
        name("prefix", r#", "use_block_description_prefix": true"#),
        translate("block.bridges.oak.log_bridge")
    );
    assert_eq!(
        name("own", r#", "use_block_description_prefix": false"#),
        translate("item.bridges.oak.log_bridge")
    );
    assert_eq!(
        name(
            "patched",
            r#", "use_block_description_prefix": false,
                "components": {"minecraft:item_name": {"translate": "bridges.named"}}"#
        ),
        translate("bridges.named")
    );
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn a_mods_atlas_sources_follow_vanillas() {
    let vanilla = r#"{"sources": [{"type": "directory", "source": "block", "prefix": "block/"}]}"#;
    let more = [serde_json::json!({"type": "directory", "source": "mek_chemical"})];
    let merged: serde_json::Value = serde_json::from_slice(
        &sunrise_level::pack_meta::append_atlas_sources(Some(vanilla), &more, "a").unwrap(),
    )
    .unwrap();
    let sources: Vec<_> = merged["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["source"].as_str().unwrap())
        .collect();
    assert_eq!(sources, ["block", "mek_chemical"]);
}

/// Registry IDs and state IDs must not depend on package discovery order.
#[test]
fn allocated_ids_follow_mod_and_entry_order_after_vanilla() {
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-order-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let base = scratch.join("base");
    fs::create_dir_all(&base).unwrap();
    fs::write(
        base.join("facts.json"),
        r#"{"states": [["minecraft:iron_block"], ["minecraft:iron_block"], ["minecraft:stone"]],
            "defaults": {"minecraft:iron_block": 0, "minecraft:stone": 2}}"#,
    )
    .unwrap();
    fs::write(
        base.join("gameplay-facts.json"),
        r#"{"states": [{"item":"minecraft:iron_block"}, {"item":"minecraft:iron_block"},
            {"item":"minecraft:stone"}]}"#,
    )
    .unwrap();
    fs::write(
        base.join("items.json"),
        r#"{"minecraft:stone": {}, "minecraft:iron_block": {}}"#,
    )
    .unwrap();
    fs::write(
        base.join("registries.json"),
        r#"{"minecraft:item":{"entries":{"minecraft:stone":{"protocol_id":4},
            "minecraft:iron_block":{"protocol_id":9}}},
            "minecraft:block":{"entries":{"minecraft:stone":{"protocol_id":7},
            "minecraft:iron_block":{"protocol_id":11}}},
            "minecraft:mob_effect":{"entries":{"minecraft:speed":{"protocol_id":0},
            "minecraft:luck":{"protocol_id":25}}},
            "minecraft:fluid":{"entries":{"minecraft:empty":{"protocol_id":0},
            "minecraft:flowing_water":{"protocol_id":1},"minecraft:water":{"protocol_id":2},
            "minecraft:flowing_lava":{"protocol_id":3},"minecraft:lava":{"protocol_id":4}}},
            "minecraft:block_entity_type":{"entries":{"minecraft:chest":{"protocol_id":1}}}}"#,
    )
    .unwrap();
    fs::create_dir_all(base.join("assets/minecraft/blockstates")).unwrap();
    fs::write(
        base.join("assets/minecraft/blockstates/iron_block.json"),
        "{}",
    )
    .unwrap();
    fs::write(base.join("assets/minecraft/blockstates/stone.json"), "{}").unwrap();
    let ruby = folder_mod(
        &scratch.join("ruby"),
        &[
            ("sunrise-mod.toml", &manifest("ruby", "")),
            (
                "data/ruby/sunrise/block/zblock.json",
                r#"{"template":"minecraft:stone"}"#,
            ),
            (
                "data/ruby/sunrise/item/aitem.json",
                r#"{"template":"minecraft:stone"}"#,
            ),
            (
                "data/ruby/sunrise/mob_effect/glow.json",
                r#"{"category":"beneficial","color":16711680}"#,
            ),
            (
                "data/ruby/sunrise/fluid/honey.json",
                r#"{"sprite":"ruby:block/h"}"#,
            ),
        ],
    );
    let alpha = folder_mod(
        &scratch.join("alpha"),
        &[
            ("sunrise-mod.toml", &manifest("alpha", "")),
            (
                "data/alpha/sunrise/block/zblock.json",
                r#"{"template":"minecraft:iron_block"}"#,
            ),
            (
                "data/alpha/sunrise/item/aitem.json",
                r#"{"template":"minecraft:stone"}"#,
            ),
            (
                "data/alpha/sunrise/mob_effect/zap.json",
                r#"{"category":"harmful","color":255}"#,
            ),
            (
                "data/alpha/sunrise/mob_effect/dull.json",
                r#"{"category":"neutral","color":0}"#,
            ),
            (
                "data/alpha/sunrise/fluid/molten/tin.json",
                r#"{"sprite":"alpha:block/s"}"#,
            ),
        ],
    );
    let mut mods = vec![ruby, alpha];
    mods[0].content.block_entity_types.push(BlockEntityType {
        id: "ruby:machine".into(),
        blocks: vec!["alpha:zblock".into()],
    });
    mods[1].content.block_entity_types.push(BlockEntityType {
        id: "alpha:machine".into(),
        blocks: vec!["ruby:zblock".into()],
    });
    for index in 0..2 {
        let plan = Plan::new(&base, &mods).unwrap();
        let blocks: Vec<_> = plan
            .blocks
            .iter()
            .map(|block| {
                (
                    block.decl.id.as_str(),
                    block.rename.first,
                    block.rename.states.len(),
                )
            })
            .collect();
        assert_eq!(blocks, [("alpha:zblock", 3, 2), ("ruby:zblock", 5, 1)]);
        let items: Vec<_> = plan
            .items
            .iter()
            .map(|item| item.rename.to.as_str())
            .collect();
        assert_eq!(
            items,
            ["alpha:aitem", "alpha:zblock", "ruby:aitem", "ruby:zblock"]
        );

        // Exercise the same registry rewrite the game and server read.
        let mut registry: J = serde_json::from_str(
            r#"{"minecraft:item":{"entries":{"minecraft:stone":{"protocol_id":4},
                "minecraft:iron_block":{"protocol_id":9}}},
                "minecraft:block":{"entries":{"minecraft:stone":{"protocol_id":7},
                "minecraft:iron_block":{"protocol_id":11}}},
                "minecraft:mob_effect":{"entries":{"minecraft:speed":{"protocol_id":0},
                "minecraft:luck":{"protocol_id":25}}},
                "minecraft:fluid":{"entries":{"minecraft:empty":{"protocol_id":0},
                "minecraft:flowing_water":{"protocol_id":1},"minecraft:water":{"protocol_id":2},
                "minecraft:flowing_lava":{"protocol_id":3},"minecraft:lava":{"protocol_id":4}}},
                "minecraft:block_entity_type":{"entries":{"minecraft:chest":{"protocol_id":1}}}}"#,
        )
        .unwrap();
        plan.json("registries.json", &mut registry).unwrap();
        let registry: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&registry).unwrap()).unwrap();
        let id = |kind: &str, name: &str| {
            registry[kind]["entries"][name]["protocol_id"]
                .as_u64()
                .unwrap()
        };
        assert_eq!(id("minecraft:item", "minecraft:stone"), 4);
        assert_eq!(id("minecraft:item", "minecraft:iron_block"), 9);
        assert_eq!(id("minecraft:item", "alpha:aitem"), 10);
        assert_eq!(id("minecraft:item", "alpha:zblock"), 11);
        assert_eq!(id("minecraft:item", "ruby:aitem"), 12);
        assert_eq!(id("minecraft:item", "ruby:zblock"), 13);
        assert_eq!(id("minecraft:block", "minecraft:stone"), 7);
        assert_eq!(id("minecraft:block", "minecraft:iron_block"), 11);
        assert_eq!(id("minecraft:block", "alpha:zblock"), 12);
        assert_eq!(id("minecraft:block", "ruby:zblock"), 13);
        // Only declared block entity types get IDs, sorted independently of blocks.
        assert_eq!(id("minecraft:block_entity_type", "minecraft:chest"), 1);
        assert_eq!(id("minecraft:block_entity_type", "alpha:machine"), 2);
        assert_eq!(id("minecraft:block_entity_type", "ruby:machine"), 3);
        assert!(registry["minecraft:block_entity_type"]["entries"]["alpha:zblock"].is_null());
        assert!(registry["minecraft:block_entity_type"]["entries"]["ruby:zblock"].is_null());

        assert_eq!(id("minecraft:mob_effect", "minecraft:luck"), 25);
        assert_eq!(id("minecraft:mob_effect", "alpha:dull"), 26);
        assert_eq!(id("minecraft:mob_effect", "alpha:zap"), 27);
        assert_eq!(id("minecraft:mob_effect", "ruby:glow"), 28);
        // Each mod fluid after vanilla's, by id: its source, then its flowing form.
        assert_eq!(id("minecraft:fluid", "minecraft:lava"), 4);
        assert_eq!(id("minecraft:fluid", "alpha:molten/tin"), 5);
        assert_eq!(id("minecraft:fluid", "alpha:flowing_molten/tin"), 6);
        assert_eq!(id("minecraft:fluid", "ruby:honey"), 7);
        assert_eq!(id("minecraft:fluid", "ruby:flowing_honey"), 8);

        // `/effect` and the mob_effect arguments check ids against this table.
        let mut commands: J = serde_json::from_str(
            r#"{"blocks":[],"items":[],"effects":["minecraft:speed"],
                "by_registry":{"minecraft:mob_effect":{"ids":["minecraft:speed"]},
                "minecraft:fluid":{"ids":["minecraft:water"]}}}"#,
        )
        .unwrap();
        plan.json("command-registries.json", &mut commands).unwrap();
        let commands: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&commands).unwrap()).unwrap();
        let want = ["minecraft:speed", "alpha:dull", "alpha:zap", "ruby:glow"];
        assert_eq!(commands["effects"], serde_json::json!(want));
        assert_eq!(
            commands["by_registry"]["minecraft:mob_effect"]["ids"],
            serde_json::json!(want)
        );
        // Fluid tag files and fluid arguments check ids against this table.
        assert_eq!(
            commands["by_registry"]["minecraft:fluid"]["ids"],
            serde_json::json!([
                "minecraft:water",
                "alpha:molten/tin",
                "alpha:flowing_molten/tin",
                "ruby:honey",
                "ruby:flowing_honey"
            ])
        );
        drop(plan);

        let output = scratch.join(format!("output-{index}"));
        build_fixture(&base, Path::new(""), &mods, &output).unwrap();
        let built: serde_json::Value =
            serde_json::from_slice(&fs::read(output.join("registries.json")).unwrap()).unwrap();
        assert_eq!(
            built, registry,
            "the built root must use the tested allocation"
        );
        assert_eq!(
            fs::read_to_string(output.join(BLOCK_ENTITY_TYPES)).unwrap(),
            r#"{"alpha:zblock":"ruby:machine","ruby:zblock":"alpha:machine"}"#
        );
        mods.reverse();
    }
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn refuses_unknown_templates_and_vanilla_ids() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-refuse-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let mut m = ruby_mod(&scratch.join("ruby"));
    m.content.blocks[0].template = "minecraft:not_a_block".into();
    let e = build_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("out"),
    )
    .unwrap_err();
    assert!(e.to_string().contains("is not a block"), "{e}");
    m.content.blocks[0].template = "minecraft:iron_block".into();
    let gem = m
        .content
        .items
        .iter_mut()
        .find(|i| i.id == "ruby:ruby")
        .unwrap();
    gem.block = Some("minecraft:not_a_block".into());
    let e = build_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("out"),
    )
    .unwrap_err();
    assert!(
        e.to_string()
            .contains("block \"minecraft:not_a_block\" is not a block"),
        "{e}"
    );
    m.content
        .items
        .iter_mut()
        .find(|i| i.id == "ruby:ruby")
        .unwrap()
        .block = Some("ruby:ruby_glass".into());
    let stew = m
        .content
        .items
        .iter_mut()
        .find(|i| i.id == "ruby:ruby_stew");
    stew.unwrap().crafting_remainder = Some("minecraft:bowll".into());
    let e = build_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("out"),
    )
    .unwrap_err();
    assert!(
        e.to_string()
            .contains(r#"crafting_remainder "minecraft:bowll" is not an item"#),
        "{e}"
    );
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn refuses_more_states_than_a_world_holds() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-states-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let mut m = ruby_mod(&scratch.join("ruby"));
    // Redstone wire has 1296 states: 810 copies pass 2^20 with any game.
    let mut wire = m.content.blocks[0].clone();
    wire.template = "minecraft:redstone_wire".into();
    for i in 0..810 {
        let mut b = wire.clone();
        b.id = format!("ruby:wire_{i}");
        m.content.blocks.push(b);
    }
    let e = build_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("out"),
    )
    .unwrap_err();
    assert!(
        e.to_string()
            .contains("more than the 1048576 a world can hold"),
        "{e}"
    );
    assert!(e.to_string().contains("the mods: ruby "), "{e}");
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn builds_past_sixteen_bit_states() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-wide-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let mut m = ruby_mod(&scratch.join("ruby"));
    // 40 redstone wires: 51,840 states, past 65,536 with vanilla's 35,723.
    let mut wire = m.content.blocks[0].clone();
    wire.template = "minecraft:redstone_wire".into();
    for i in 0..40 {
        let mut b = wire.clone();
        b.id = format!("ruby:wire_{i}");
        m.content.blocks.push(b);
    }
    build_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("out"),
    )
    .unwrap();
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn no_content_keeps_the_base_root() {
    let base = Path::new("/nonexistent/base");
    assert_eq!(
        resource_root_fixture(base, Path::new(""), &[], Path::new("/nonexistent/cache")).unwrap(),
        base
    );
}

use sunrise_world::loot::PackOverlay;
use sunrise_world::registry::BlockStateId;

#[test]
#[ignore = "daily"]
fn shared_behaviour_document_matches_independent_loaders() {
    let Some(root) = prepared() else { return };
    let assets = Assets::load(&root).expect("assets");
    let doc: serde_json::Value = serde_json::from_reader(std::io::BufReader::new(
        std::fs::File::open(root.join("block-behaviours.json")).unwrap(),
    ))
    .unwrap();
    let original = BlockFacts::load(&root, assets.registry).unwrap();
    let (shared, b) = BlockFacts::load_with_behaviours(&root, assets.registry, &doc).unwrap();
    let (deferred_facts, projection) =
        BlockFacts::load_with_projection(&root, assets.registry, &doc, true).unwrap();
    let deferred = sunrise_world::level::Behaviours::load_with_gameplay(
        assets.registry,
        &root,
        &doc,
        &projection,
    )
    .unwrap();
    assert_eq!(deferred_facts.states.len(), shared.states.len());
    assert_eq!(original.states.len(), shared.states.len());
    for (a, b) in original.states.iter().zip(&shared.states) {
        assert_eq!(a.classes, b.classes);
        assert_eq!(a.legacy_solid, b.legacy_solid);
        assert_eq!(a.sturdy_full, b.sturdy_full);
        assert_eq!(a.emission, b.emission);
        assert_eq!(a.dampening, b.dampening);
        assert_eq!(a.hardness, b.hardness);
        assert_eq!(a.sounds, b.sounds);
    }
    let a = sunrise_world::level::Behaviours::load(assets.registry, &root).unwrap();
    for id in 0..assets.registry.len() {
        let state = sunrise_world::registry::BlockStateId(id as u32);
        let (a_state, b_state) = (a.facts(state), b.facts(state));
        let deferred_state = deferred.facts(state);
        assert_eq!(b_state.flags, deferred_state.flags);
        assert_eq!(b_state.emission, deferred_state.emission);
        assert_eq!(b_state.dampening, deferred_state.dampening);
        assert_eq!(b_state.hardness, deferred_state.hardness);
        assert_eq!(b_state.push, deferred_state.push);
        assert_eq!(a_state.flags, b_state.flags);
        assert_eq!(a_state.sturdy, b_state.sturdy);
        assert_eq!(a_state.weak, b_state.weak);
        assert_eq!(a_state.strong, b_state.strong);
        assert_eq!(a_state.fluid, b_state.fluid);
        assert_eq!(a_state.emission, b_state.emission);
        assert_eq!(a_state.dampening, b_state.dampening);
        assert_eq!(a_state.hardness, b_state.hardness);
        assert_eq!(a_state.push, b_state.push);
        assert_eq!(a_state.instrument, b_state.instrument);
        assert_eq!(a_state.shape, b_state.shape);
        assert_eq!(a_state.placement_sound, b_state.placement_sound);
        assert_eq!(b_state.placement_sound, deferred_state.placement_sound);
    }
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn independent_reloads_read_changed_behaviour_json_and_report_invalid_files() {
    let Some(base) = prepared() else { return };
    let scratch = std::env::temp_dir().join(format!("sunrise-json-reloads-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    // Keep the remaining resource tree intact without copying a huge prepared client.
    for entry in fs::read_dir(&base).unwrap() {
        let entry = entry.unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(entry.path(), scratch.join(entry.file_name())).unwrap();
    }
    let assets = Assets::load(&base).unwrap();
    let registry = assets.registry;
    let original = fs::read(base.join("block-behaviours.json")).unwrap();
    fs::remove_file(scratch.join("block-behaviours.json")).unwrap();
    fs::write(scratch.join("block-behaviours.json"), &original).unwrap();
    // Independent facts and behaviours can load first, followed by paired data.
    let independent_first = BlockFacts::load(&scratch, registry).unwrap();
    let independent_world_first =
        sunrise_world::level::Behaviours::load(registry, &scratch).unwrap();
    let (facts, paired_world) = BlockFacts::load_pair(&scratch, registry).unwrap();
    assert_eq!(facts.states.len(), registry.len());
    for state in [0, 1, registry.len() - 1] {
        let state = sunrise_world::registry::BlockStateId(state as u32);
        assert_eq!(
            facts.get(state).classes,
            independent_first.get(state).classes
        );
        assert_eq!(
            facts.get(state).sounds.place,
            independent_first.get(state).sounds.place
        );
        assert_eq!(
            paired_world.facts(state).flags,
            independent_world_first.facts(state).flags
        );
    }

    // Override one exported state bit at the same path and verify that a later
    // independent and paired read sees it while the original resource root does not.
    let mut changed: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let original_bits = changed["states"][0][1].as_u64().unwrap();
    changed["states"][0][1] = (original_bits ^ 1).into();
    fs::write(
        scratch.join("block-behaviours.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    let changed_pair = BlockFacts::load_pair(&scratch, registry).unwrap();
    let changed_independent = BlockFacts::load(&scratch, registry).unwrap();
    let base_facts = BlockFacts::load(&base, registry).unwrap();
    let state = sunrise_world::registry::BlockStateId(0);
    assert_ne!(
        changed_pair.0.get(state).legacy_solid,
        base_facts.get(state).legacy_solid
    );
    assert_eq!(
        changed_pair.0.get(state).legacy_solid,
        changed_independent.get(state).legacy_solid
    );
    // The world loader sees the same overridden legacy solidity flag.
    let changed_world = sunrise_world::level::Behaviours::load(registry, &scratch).unwrap();
    assert_eq!(
        changed_world.facts(state).flags & 1 != 0,
        changed_pair.0.get(state).legacy_solid
    );

    fs::write(scratch.join("block-behaviours.json"), b"{").unwrap();
    let paired_error = BlockFacts::load_pair(&scratch, registry)
        .err()
        .unwrap()
        .to_string();
    let independent_error = BlockFacts::load(&scratch, registry)
        .err()
        .unwrap()
        .to_string();
    assert_eq!(paired_error, independent_error);
    assert!(sunrise_world::level::Behaviours::load(registry, &scratch).is_err());

    fs::write(scratch.join("block-behaviours.json"), b"{}").unwrap();
    assert!(BlockFacts::load_pair(&scratch, registry).is_err());
    assert!(BlockFacts::load(&scratch, registry).is_err());
    assert!(sunrise_world::level::Behaviours::load(registry, &scratch).is_err());

    fs::write(scratch.join("block-behaviours.json"), original).unwrap();
    assert_eq!(
        BlockFacts::load_pair(&scratch, registry)
            .unwrap()
            .0
            .states
            .len(),
        registry.len()
    );
    fs::remove_file(scratch.join("gameplay-facts.json")).unwrap();
    fs::write(scratch.join("gameplay-facts.json"), b"{\"states\":[]}").unwrap();
    assert!(BlockFacts::load_pair(&scratch, registry).is_err());
    assert!(BlockFacts::load(&scratch, registry).is_err());
    assert!(sunrise_world::level::Behaviours::load(registry, &scratch).is_err());
    fs::write(scratch.join("gameplay-facts.json"), b"{").unwrap();
    assert!(BlockFacts::load_pair(&scratch, registry).is_err());
    assert!(BlockFacts::load(&scratch, registry).is_err());
    assert!(sunrise_world::level::Behaviours::load(registry, &scratch).is_err());
    fs::write(
        scratch.join("gameplay-facts.json"),
        fs::read(base.join("gameplay-facts.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        BlockFacts::load_pair(&scratch, registry)
            .unwrap()
            .0
            .states
            .len(),
        registry.len()
    );
    fs::remove_dir_all(&scratch).unwrap();
}

/// A built root is kept for later starts, and built again when a mod's
/// files change.
#[test]
#[ignore = "daily"]
fn content_root_is_reused_until_a_mod_changes() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-reuse-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let mut m = ruby_mod(&scratch.join("ruby"));
    let cache = scratch.join("cache");
    let root =
        resource_root_fixture(&base, Path::new(""), std::slice::from_ref(&m), &cache).unwrap();
    // A rebuild starts from an empty folder: this file tells them apart.
    let sentinel = root.join("sentinel");
    fs::write(&sentinel, b"").unwrap();
    assert_eq!(
        resource_root_fixture(&base, Path::new(""), std::slice::from_ref(&m), &cache).unwrap(),
        root
    );
    assert!(sentinel.exists(), "an unchanged mod rebuilt the root");
    let old_tables = fs::read(root.join("facts.json")).unwrap();
    let lang = scratch.join("ruby/assets/ruby/lang/en_us.json");
    fs::write(&lang, r#"{"block.ruby.ruby_block": "Ruby Block"}"#).unwrap();
    let file = fs::File::options().write(true).open(&lang).unwrap();
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();
    let changed =
        resource_root_fixture(&base, Path::new(""), std::slice::from_ref(&m), &cache).unwrap();
    assert_ne!(changed, root);
    assert!(sentinel.exists(), "a rebuild removed the active root");
    assert_eq!(fs::read(root.join("facts.json")).unwrap(), old_tables);
    let resources = resolved_server_resources(
        &base,
        changed.clone(),
        Path::new(""),
        std::slice::from_ref(&m),
        cache.clone(),
    )
    .unwrap();
    assert!(
        resources
            .source()
            .read_to_string("assets/ruby/lang/en_us.json")
            .unwrap()
            .contains("Ruby Block")
    );
    assert!(!changed.join("assets/ruby/lang/en_us.json").exists());
    m.content.blocks[0].template = "minecraft:missing_template".into();
    assert!(resource_root_fixture(&base, Path::new(""), std::slice::from_ref(&m), &cache).is_err());
    assert!(
        changed.exists(),
        "a refused rebuild removed the accepted root"
    );
    assert!(sentinel.exists());
    let _ = fs::remove_dir_all(&scratch);
}

/// `sunrise:add_to`: only the set, pool and processor list bundles take additions; each is
/// appended after the target's own entries, lower `sunrise:priority` first and equal priorities
/// in the order given (mod, then file order); a target nobody defines is skipped.
#[test]
fn additions_append_by_priority_then_order_and_skip_unknown_targets() {
    let pools = "worldgen/template_pools.json";
    let mut additions: Vec<Addition> = [
        r#"{"sunrise:add_to": ["mod:pool", "mod:missing"], "elements": ["a"]}"#,
        r#"{"sunrise:add_to": "mod:pool", "sunrise:priority": 5, "elements": ["b"]}"#,
        r#"{"sunrise:add_to": "mod:pool", "elements": ["c"]}"#,
        r#"{"sunrise:add_to": "mod:pool", "sunrise:priority": 5, "elements": ["d"]}"#,
        r#"{"sunrise:add_to": "mod:pool", "sunrise:priority": 1000, "elements": ["e"]}"#,
    ]
    .iter()
    .map(|text| Addition::read(pools, text).unwrap().unwrap())
    .collect();
    let mut doc: J = serde_json::from_str(
        &serde_json::json!({"mod:pool": r#"{"elements": ["own"]}"#}).to_string(),
    )
    .unwrap();
    apply_additions(&mut doc, &mut additions).unwrap();
    let Some(J::Str(text)) = doc.get("mod:pool") else {
        panic!("the pool is kept");
    };
    let pool: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        pool["elements"],
        serde_json::json!(["own", "b", "d", "a", "c", "e"])
    );
    assert!(doc.get("mod:missing").is_none());
    let feature = r#"{"sunrise:add_to": "mod:pool", "elements": ["a"]}"#;
    assert!(
        Addition::read("worldgen/features.json", feature)
            .unwrap()
            .is_none()
    );
}

/// A structure mod's worldgen reaches the worldgen loader: its structure,
/// set, pool and templates join the bundles, and its biome tag keeps only
/// what the loader knows.
#[test]
#[ignore = "daily"]
fn content_root_carries_a_structure_mod() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-worldgen-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let dir = scratch.join("huts");
    let files = [
        (
            "sunrise-mod.toml",
            "id = \"huts\"\nname = \"Huts\"\nversion = \"1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        ),
        (
            "data/huts/worldgen/structure/hut.json",
            r##"{"type": "minecraft:jigsaw", "biomes": "#huts:has_hut", "step": "surface_structures",
                "spawn_overrides": {}, "terrain_adaptation": "beard_thin", "start_pool": "huts:hut/start",
                "size": 1, "start_height": {"absolute": 0}, "project_start_to_heightmap": "WORLD_SURFACE_WG",
                "max_distance_from_center": 80}"##,
        ),
        (
            "data/huts/worldgen/template_pool/hut/start.json",
            r#"{"fallback": "minecraft:empty", "elements": [{"weight": 1, "element": {
                "element_type": "minecraft:single_pool_element", "location": "huts:hut",
                "projection": "rigid", "processors": "minecraft:empty"}}]}"#,
        ),
        (
            "data/huts/worldgen/structure_set/huts.json",
            r#"{"structures": [{"structure": "huts:hut", "weight": 1}],
                "placement": {"type": "minecraft:random_spread", "salt": 7, "spacing": 4, "separation": 2}}"#,
        ),
        (
            "data/huts/tags/worldgen/biome/has_hut.json",
            r##"{"values": ["#minecraft:is_overworld", "#c:is_plains", "othermod:glade", "minecraft:plains"]}"##,
        ),
        // Additions to vanilla's set, pool and processor list; a target nobody
        // defines is skipped. A lower priority goes first.
        (
            "data/huts/worldgen/structure_set/added/villages.json",
            r#"{"sunrise:add_to": "minecraft:villages",
                "structures": [{"structure": "huts:hut", "weight": 3}]}"#,
        ),
        (
            "data/huts/worldgen/template_pool/added/houses.json",
            r#"{"sunrise:add_to": ["othermod:houses", "minecraft:village/plains/houses"],
                "elements": [{"weight": 2, "element": {"element_type": "minecraft:single_pool_element",
                "location": "huts:hut", "projection": "rigid", "processors": "minecraft:empty"}}]}"#,
        ),
        (
            "data/huts/worldgen/template_pool/added/more_houses.json",
            r#"{"sunrise:add_to": "minecraft:village/plains/houses", "sunrise:priority": 10,
                "elements": [{"weight": 1, "element": {"element_type": "minecraft:empty_pool_element"}}]}"#,
        ),
        (
            "data/huts/worldgen/processor_list/added/mossy.json",
            r#"{"sunrise:add_to": "minecraft:mossify_10_percent",
                "processors": [{"processor_type": "minecraft:nop"}]}"#,
        ),
    ];
    for (name, text) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    // The vanilla house's bytes belong to the mod; prepared file permissions do not.
    let houses = base.join("worldgen/structure/minecraft/village/plains/houses");
    let template = fs::read(houses.join("plains_small_house_1.nbt")).unwrap();
    fs::create_dir_all(dir.join("data/huts/structure")).unwrap();
    fs::write(dir.join("data/huts/structure/hut.nbt"), template).unwrap();
    let package = sunrise_mod_package::Package::read(&dir).unwrap();
    let m = ContentMod {
        id: "huts".into(),
        content: Content::read(&dir, &package.manifest).unwrap(),
        jars: Vec::new(),
    };
    let root = resource_root_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("cache"),
    )
    .unwrap();
    let resources = resolved_server_resources(
        &base,
        root.clone(),
        Path::new(""),
        std::slice::from_ref(&m),
        scratch.join("cache"),
    )
    .unwrap();
    assert!(
        resources
            .data_changes()
            .contains("data/huts/structure/hut.nbt")
    );
    assert!(!root.join("worldgen/structure/huts/hut.nbt").exists());
    let tags: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/biome_tags.json")).unwrap()).unwrap();
    // `#c:is_plains` stays: NeoForge generates that biome tag, so the reference resolves;
    // `othermod:glade` names a biome nobody installed and goes.
    let hut: serde_json::Value = serde_json::from_str(&tags["huts:has_hut"]).unwrap();
    assert_eq!(
        hut["values"],
        serde_json::json!([
            "#minecraft:is_overworld",
            "#c:is_plains",
            "minecraft:plains"
        ])
    );
    // The additions join vanilla's entries at the end and are no set or pool
    // of their own.
    let bundle = |name: &str, base: &Path| -> BTreeMap<String, String> {
        serde_json::from_slice(&fs::read(base.join("worldgen").join(name)).unwrap()).unwrap()
    };
    for (name, id, list, added) in [
        (
            "structure_sets.json",
            "minecraft:villages",
            "structures",
            vec![serde_json::json!({"structure": "huts:hut", "weight": 3})],
        ),
        (
            "template_pools.json",
            "minecraft:village/plains/houses",
            "elements",
            vec![
                serde_json::json!({"weight": 1, "element": {"element_type": "minecraft:empty_pool_element"}}),
                serde_json::json!({"weight": 2, "element": {"element_type": "minecraft:single_pool_element",
                "location": "huts:hut", "projection": "rigid", "processors": "minecraft:empty"}}),
            ],
        ),
        (
            "processor_lists.json",
            "minecraft:mossify_10_percent",
            "processors",
            vec![serde_json::json!({"processor_type": "minecraft:nop"})],
        ),
    ] {
        let (ours, vanilla) = (bundle(name, &root), bundle(name, &base));
        let entries = |text: &str| -> Vec<serde_json::Value> {
            let v: serde_json::Value = serde_json::from_str(text).unwrap();
            v[list].as_array().unwrap().clone()
        };
        let mut expected = entries(&vanilla[id]);
        expected.extend(added);
        assert_eq!(entries(&ours[id]), expected, "{id}");
        assert!(!ours.keys().any(|k| k.starts_with("huts:added/")), "{name}");
        assert!(!ours.contains_key("othermod:houses"));
    }

    // The commands suggest the mod's structure, set and pool.
    let registries: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("command-registries.json")).unwrap()).unwrap();
    for (registry, id) in [
        ("minecraft:worldgen/structure", "huts:hut"),
        ("minecraft:worldgen/structure_set", "huts:huts"),
        ("minecraft:worldgen/template_pool", "huts:hut/start"),
    ] {
        let ids = registries["by_registry"][registry]["ids"]
            .as_array()
            .unwrap();
        assert!(ids.iter().any(|v| v == id), "{registry} lacks {id}");
        assert!(
            ids.iter()
                .any(|v| v.as_str().unwrap().starts_with("minecraft:"))
        );
    }

    let assets = Assets::load(&root).unwrap();
    let biomes = crate::biome::Biomes::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&root, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: Some(resources.data_changes()),
        dir: &root.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let terrain = crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
    let (name, at) = terrain
        .locate(&["huts:hut".to_owned()], [0, 64, 0], 16)
        .expect("the mod's structure generates");
    assert_eq!(name, "huts:hut");
    assert!(at[0].abs() < 1024 && at[2].abs() < 1024, "{at:?}");
    let _ = fs::remove_dir_all(&scratch);
}

/// `FlatLevelSource.createState`: a superflat world runs the structure sets its
/// `structure_overrides` allow (the classic flat world: strongholds and villages) on its
/// flat ground, and no other set, with the layers where the terrain would be.
#[test]
#[ignore = "daily"]
fn a_superflat_world_runs_its_structure_sets_on_flat_ground() {
    use sunrise_level::chunk_streamer::ChunkGenerator;
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(base.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&base, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: None,
        dir: &base.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let flat = sunrise_world::worldgen::generator::FlatSettings {
        layers: vec![
            ("minecraft:bedrock".into(), 1),
            ("minecraft:dirt".into(), 2),
            ("minecraft:grass_block".into(), 1),
        ],
        biome: "minecraft:plains".into(),
        structure_overrides: Some(vec![
            "minecraft:strongholds".into(),
            "minecraft:villages".into(),
        ]),
        features: false,
        lakes: false,
    };
    let terrain = crate::vanillagen::VanillaTerrain::for_flat(
        terrain_resources,
        &Default::default(),
        1234,
        &flat,
        true,
    )
    .unwrap();
    // A village and a stronghold are found; a mineshaft is not in the overrides.
    let village = terrain.locate(&["minecraft:village_plains".to_owned()], [0, 64, 0], 12);
    assert!(village.is_some(), "no plains village near the origin");
    assert!(
        terrain
            .locate(&["minecraft:stronghold".to_owned()], [0, 64, 0], 12)
            .is_some()
    );
    assert!(
        terrain
            .locate(&["minecraft:mineshaft".to_owned()], [0, 64, 0], 12)
            .is_none()
    );
    // The layers: bedrock, two dirt and grass from the level's minimum Y, air above.
    let mut worker = terrain.worker();
    let chunk = terrain.finish(terrain.proto(&mut worker, 3, 3).unwrap());
    let name = |y| {
        let id = chunk.block(3 * 16 + 5, y, 3 * 16 + 7);
        assets.registry.state(id).unwrap().name.clone()
    };
    assert_eq!(
        [-65, -64, -63, -62, -61, -60].map(name),
        [
            "minecraft:air",
            "minecraft:bedrock",
            "minecraft:dirt",
            "minecraft:dirt",
            "minecraft:grass_block",
            "minecraft:air"
        ]
        .map(String::from)
    );
}

/// `adjustGenerationSettings`: the Overworld preset decorates its plains with grass,
/// flowers and trees even with no structure set (`structure_overrides: []`, or
/// `generate_structures` off), and without `features` the same layers stay bare.
#[test]
#[ignore = "daily"]
fn a_decorated_superflat_world_without_structures_grows_its_features() {
    use sunrise_level::chunk_streamer::ChunkGenerator;
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(base.to_path_buf()),
    )
    .unwrap();
    let item_data = crate::test_resources::item_data(
        &base,
        sunrise_world::native_load_context::DataLoadOrigin::ServerDataLoad,
    )
    .unwrap();
    // Blocks above the grass (the top layer is at the level's minimum Y + 63) in the
    // chunk at 4, 4 once its 3×3 region is decorated.
    let above_ground = |features| {
        let flat = sunrise_world::worldgen::generator::FlatSettings {
            layers: vec![
                ("minecraft:bedrock".into(), 1),
                ("minecraft:stone".into(), 59),
                ("minecraft:dirt".into(), 3),
                ("minecraft:grass_block".into(), 1),
            ],
            biome: "minecraft:plains".into(),
            structure_overrides: Some(vec![]),
            features,
            lakes: false,
        };
        let terrain = crate::vanillagen::VanillaTerrain::for_flat(
            crate::vanillagen::TerrainResources {
                data: None,
                dir: &base.join("worldgen"),
                registry: assets.registry,
                biomes: &biomes,
                item_data: &item_data,
            },
            &Default::default(),
            1234,
            &flat,
            false,
        )
        .unwrap();
        let mut worker = terrain.worker();
        let mut region: [_; 9] = std::array::from_fn(|i| {
            terrain
                .proto(&mut worker, 3 + i as i32 % 3, 3 + i as i32 / 3)
                .unwrap()
        });
        terrain.decorate(&mut worker, &mut region);
        let [_, _, _, _, center, ..] = region;
        let chunk = terrain.finish(center);
        let mut found = Vec::new();
        for y in 0..16 {
            for z in 64..80 {
                for x in 64..80 {
                    let name = &assets.registry.state(chunk.block(x, y, z)).unwrap().name;
                    if name != "minecraft:air" {
                        found.push(name.clone());
                    }
                }
            }
        }
        found
    };
    let decorated = above_ground(true);
    assert!(
        decorated.iter().any(|n| n == "minecraft:short_grass"),
        "{decorated:?}"
    );
    assert_eq!(above_ground(false), Vec::<String>::new());
}

/// `createForFlat` searches the stronghold rings with seed 0, so a superflat world's
/// strongholds are in the same place on every seed; a layer that is not opaque to
/// `MOTION_BLOCKING` is `null`, left out of `buildTerrain` for the fill after the structures.
#[test]
#[ignore = "daily"]
fn a_superflat_world_keeps_its_strongholds_on_every_seed_and_leaves_null_layers_for_the_fill() {
    use sunrise_level::chunk_streamer::ChunkGenerator;
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(base.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&base, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: None,
        dir: &base.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let flat = sunrise_world::worldgen::generator::FlatSettings {
        layers: vec![
            ("minecraft:bedrock".into(), 1),
            ("minecraft:dirt".into(), 2),
            ("minecraft:snow".into(), 1),
        ],
        biome: "minecraft:plains".into(),
        structure_overrides: Some(vec!["minecraft:strongholds".into()]),
        features: false,
        lakes: false,
    };
    let make = |seed| {
        crate::vanillagen::VanillaTerrain::for_flat(
            terrain_resources,
            &Default::default(),
            seed,
            &flat,
            true,
        )
        .unwrap()
    };
    let stronghold = |seed| {
        make(seed)
            .locate(&["minecraft:stronghold".to_owned()], [0, 64, 0], 100)
            .expect("a stronghold")
    };
    assert_eq!(stronghold(1234), stronghold(-987654321));
    // `moveBelowSeaLevel(-63, ChunkGenerator.getMinY() = 0, ..)`: the sea level less 10 is far
    // below the room's height above 0, so the stronghold's top lands at `ySpan + 1` above Y 0
    // (with the level's -64 it would sit 64 blocks lower). The start's box is inflated by 12
    // for the terrain adaptation: the pieces span Y 2 to 36, the box -10 to 48.
    let (_, at) = stronghold(1234);
    let starts = make(1234).starts_at(at[0] >> 4, at[2] >> 4);
    let start = starts
        .iter()
        .find(|s| s.name == "minecraft:stronghold")
        .expect("the stronghold start");
    assert!(
        start.bounding_box.min[1] >= -12,
        "the stronghold sits below the generator's Y 0: {:?}",
        start.bounding_box
    );
    let terrain = make(1234);
    let mut worker = terrain.worker();
    let chunk = terrain.finish(terrain.proto(&mut worker, 3, 3).unwrap());
    let name = |y| {
        let id = chunk.block(3 * 16 + 5, y, 3 * 16 + 7);
        assets.registry.state(id).unwrap().name.clone()
    };
    // The snow layer (index 3, y -61) is filled later, not by the terrain.
    assert_eq!(
        [-64, -63, -62, -61].map(name),
        [
            "minecraft:bedrock",
            "minecraft:dirt",
            "minecraft:dirt",
            "minecraft:air"
        ]
        .map(String::from)
    );
}

/// `structure_overrides` is a lenient field: a list naming a structure set that does not exist
/// fails to decode and counts as absent, so every set generates.
#[test]
#[ignore = "daily"]
fn a_superflat_structure_override_naming_no_set_lets_every_set_generate() {
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(base.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&base, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: None,
        dir: &base.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let flat = sunrise_world::worldgen::generator::FlatSettings {
        layers: vec![
            ("minecraft:bedrock".into(), 1),
            ("minecraft:dirt".into(), 2),
            ("minecraft:grass_block".into(), 1),
        ],
        biome: "minecraft:plains".into(),
        structure_overrides: Some(vec!["minecraft:village".into()]),
        features: false,
        lakes: false,
    };
    let terrain = crate::vanillagen::VanillaTerrain::for_flat(
        terrain_resources,
        &Default::default(),
        1234,
        &flat,
        true,
    )
    .unwrap();
    assert!(
        terrain
            .locate(&["minecraft:stronghold".to_owned()], [0, 64, 0], 12)
            .is_some()
    );
    assert!(
        terrain
            .locate(&["minecraft:village_plains".to_owned()], [0, 64, 0], 12)
            .is_some()
    );
}

#[test]
fn declared_stew_effects_generate_food_and_search_variants_and_work_in_recipes() {
    use crate::crafting::Recipes;
    use crate::items::ItemStack;
    use serde_json::json;
    use sunrise_items::creative_catalog::{Catalog, Holders, Parameters};
    use sunrise_world::loot::PackOverlay;

    let (base, _) = block_base("stew-effects", &[]);
    let scratch = base.parent().unwrap();
    fs::create_dir_all(base.join("assets/minecraft/blockstates")).unwrap();
    fs::write(base.join("assets/minecraft/blockstates/test.json"), "{}").unwrap();
    let write = |name, value: serde_json::Value| {
        fs::write(base.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
    };
    let ids = [
        "test",
        "poppy",
        "open_eyeblossom",
        "bowl",
        "brown_mushroom",
        "red_mushroom",
        "suspicious_stew",
    ]
    .map(|name| format!("minecraft:{name}"));
    write(
        "items.json",
        ids.iter().map(|id| (id.clone(), json!({}))).collect(),
    );
    write(
        "command-registries.json",
        json!({"blocks":["minecraft:test"], "items": ids}),
    );
    write(
        "registries.json",
        json!({
            "minecraft:item": {"entries":{}}, "minecraft:block": {"entries":{}},
            "minecraft:mob_effect": {"entries": {"minecraft:luck":{"protocol_id":0}, "minecraft:speed":{"protocol_id":1}}}
        }),
    );
    // The template's Block.getItem is minecraft:test, but that item places a poppy.
    let bindings: serde_json::Map<_, _> = [
        ("minecraft:test", "minecraft:poppy"),
        ("minecraft:poppy", "minecraft:poppy"),
        ("minecraft:open_eyeblossom", "minecraft:open_eyeblossom"),
    ]
    .into_iter()
    .map(|(id, block)| {
        (
            id.to_owned(),
            json!({"class":["BlockItem"], "fields":{"block":block}}),
        )
    })
    .collect();
    write(
        "block-behaviours.json",
        json!({
            "states":[[], []], "blocks": [{"name":"minecraft:test"}], "items": bindings
        }),
    );
    fs::create_dir_all(base.join("data/minecraft/recipe")).unwrap();
    fs::create_dir_all(base.join("data/minecraft/tags/item")).unwrap();
    write(
        "data/minecraft/tags/item/mushrooms.json",
        json!({"values":["minecraft:brown_mushroom", "minecraft:red_mushroom"]}),
    );
    let row =
        |snbt: &str| json!({"kind":"stack", "visibility":"PARENT_AND_SEARCH_TABS", "snbt":snbt});
    write(
        "creative-inventory.json",
        json!({
            "schema":4, "item_features":{}, "potion_features":{}, "tabs":[
                {"id":"minecraft:food_and_drinks", "title":"Food & Drinks", "type":"CATEGORY", "build":[
                    row(r#"{id:"minecraft:rabbit_stew",count:1}"#),
                    row(r#"{id:"minecraft:suspicious_stew",count:1,components:{"minecraft:suspicious_stew_effects":[{id:"minecraft:night_vision",duration:100}]}}"#),
                    row(r#"{id:"minecraft:suspicious_stew",count:1,components:{"minecraft:suspicious_stew_effects":[{id:"minecraft:blindness",duration:220}]}}"#),
                    row(r#"{id:"minecraft:milk_bucket",count:1}"#)
                ]},
                {"id":"minecraft:search", "title":"Search", "type":"SEARCH", "build":[]}
            ]
        }),
    );
    let recipe = r##"{"type":"minecraft:crafting_shapeless",
        "ingredients":["testmod:amber", "minecraft:bowl", "#minecraft:mushrooms", "#minecraft:mushrooms"],
        "result":{"id":"minecraft:suspicious_stew", "components":{"minecraft:suspicious_stew_effects":[
            {"id":"minecraft:luck"},{"id":"minecraft:speed","duration":24}]}}}"##;
    let m = folder_mod(
        &scratch.join("good"),
        &[
            ("sunrise-mod.toml", &manifest("testmod", "")),
            (
                "data/testmod/sunrise/block/amber.json",
                r#"{"template":"minecraft:test","suspicious_stew_effects":[{"id":"luck"},{"id":"minecraft:speed","duration":24}]}"#,
            ),
            (
                "data/testmod/sunrise/block/copy.json",
                r#"{"template":"minecraft:test","suspicious_stew_effects":[{"id":"minecraft:luck","duration":null},{"id":"minecraft:speed","duration":24}]}"#,
            ),
            (
                "data/testmod/sunrise/block/reversed.json",
                r#"{"template":"minecraft:test","suspicious_stew_effects":[{"id":"minecraft:speed","duration":24},{"id":"minecraft:luck"}]}"#,
            ),
            (
                "data/testmod/sunrise/block/hidden.json",
                r#"{"template":"minecraft:test","item":false,"pick_item":"minecraft:bowl","suspicious_stew_effects":[{"id":"minecraft:luck","duration":31}]}"#,
            ),
            ("data/testmod/recipe/amber_stew.json", recipe),
        ],
    );
    let root = scratch.join("out");
    let mods = [m];
    build_fixture(&base, Path::new(""), &mods, &root).unwrap();
    let resources = resolved_server_resources(
        &base,
        root.clone(),
        Path::new(""),
        &mods,
        scratch.join("cache"),
    )
    .unwrap();
    let files = content_files(&resources);
    let overlay = PackOverlay::new(&files);
    let expected_a = [("minecraft:luck", 160), ("minecraft:speed", 24)];
    let expected_b = [("minecraft:speed", 24), ("minecraft:luck", 160)];
    let check_stew = |stack: &ItemStack, expected: &[(&str, i32)]| {
        assert_eq!(
            (&*stack.item, stack.count),
            ("minecraft:suspicious_stew", 1)
        );
        assert_eq!(
            stack
                .data()
                .unwrap()
                .stew_effects
                .iter()
                .map(|(id, ticks)| (&**id, *ticks))
                .collect::<Vec<_>>(),
            expected
        );
    };
    for (path, overlay, n) in [(&base, PackOverlay::NONE, 2), (&root, overlay, 4)] {
        let defaults = sunrise_world::item_defaults::read(path).unwrap();
        let items = Items::catalog(path, &overlay, &defaults).unwrap();
        let catalog = Catalog::parse(
            &serde_json::from_slice(&fs::read(path.join("creative-inventory.json")).unwrap())
                .unwrap(),
        )
        .unwrap();
        let params = Parameters {
            features: &[],
            operator: false,
            holders: &Holders::default(),
            items: &items,
        };
        let lists = catalog.build(params, None, |tab, _, before_edit| {
            if tab.id == "minecraft:food_and_drinks" {
                let food = &before_edit.display;
                assert_eq!(food.len(), n + 2);
                assert_eq!(
                    (&*food[0].item, &*food[n + 1].item),
                    ("minecraft:rabbit_stew", "minecraft:milk_bucket")
                );
                check_stew(&food[1], &[("minecraft:night_vision", 100)]);
                check_stew(&food[2], &[("minecraft:blindness", 220)]);
                if n == 4 {
                    check_stew(&food[3], &expected_a);
                    check_stew(&food[4], &expected_b);
                }
            }
        });
        assert_eq!(catalog.search_display(&lists), lists[0].display);
        if n == 4 {
            let recipes = Recipes::load(
                path,
                &items,
                &overlay,
                &crate::test_resources::server_load(),
            )
            .unwrap();
            let grid = [
                "testmod:amber",
                "minecraft:bowl",
                "minecraft:brown_mushroom",
                "minecraft:red_mushroom",
            ]
            .map(|id| Some(ItemStack::new(id, 1)));
            let (recipe, stack) = recipes
                .find(crate::test_resources::server_loop_context(), &grid, 2)
                .unwrap();
            assert_eq!(recipe, "testmod:amber_stew");
            check_stew(&stack, &expected_a);
        }
    }
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn a_block_with_an_unknown_stew_effect_is_refused_at_content_load() {
    let (base, mods) = block_base(
        "unknown-stew-effect",
        &[r#"{"template":"minecraft:test","suspicious_stew_effects":[{"id":"testmod:missing"}]}"#],
    );
    let scratch = base.parent().unwrap();
    fs::write(
        base.join("registries.json"),
        r#"{"minecraft:mob_effect":{"entries":{}}}"#,
    )
    .unwrap();
    let error = build_fixture(&base, Path::new(""), &mods, &scratch.join("out")).unwrap_err();
    assert!(
        error.to_string().contains(
            "testmod:test: suspicious_stew_effects: unknown mob effect \"testmod:missing\""
        )
    );
    fs::remove_dir_all(scratch).unwrap();
}

/// A mod's mob effect gets the network id after vanilla's last, on the game
/// and its server alike, and joins the ids `/effect` and its suggestions
/// check.
#[test]
#[ignore = "daily"]
fn content_root_registers_a_mod_mob_effect() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-effect-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("glow"),
        &[
            ("sunrise-mod.toml", &manifest("glow", "")),
            (
                "data/glow/sunrise/mob_effect/radiance.json",
                r#"{"category": "beneficial", "color": 16773120}"#,
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
    let report = |root: &Path| {
        sunrise_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap()
    };
    let (vanilla, ids) = (report(&base), report(&root));
    let vanilla = vanilla.get("minecraft:mob_effect").unwrap();
    let ids = ids.get("minecraft:mob_effect").unwrap();
    let count = (0..).take_while(|&i| vanilla.name(i).is_some()).count() as i32;
    assert_eq!(ids.id("glow:radiance"), Some(count));
    assert_eq!(ids.id("minecraft:speed"), vanilla.id("minecraft:speed"));
    let commands: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("command-registries.json")).unwrap()).unwrap();
    for ids in [
        &commands["effects"],
        &commands["by_registry"]["minecraft:mob_effect"]["ids"],
    ] {
        let ids = ids.as_array().unwrap();
        assert!(ids.contains(&"glow:radiance".into()) && ids.contains(&"minecraft:speed".into()));
    }
    assert!(root.join(crate::content_identity::MARKER).exists());
    fs::remove_dir_all(&scratch).unwrap();
}

/// A mod that adds only an item still moves the item network ids, so a game
/// without it must fail the content check rather than show other items.
#[test]
#[ignore = "daily"]
fn content_root_with_only_an_item_checks_its_identity() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-item-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("gem"),
        &[
            ("sunrise-mod.toml", &manifest("gem", "")),
            (
                "data/gem/sunrise/item/opal.json",
                r#"{"template": "minecraft:diamond"}"#,
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
    let ids = sunrise_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap();
    assert!(ids.get("minecraft:item").unwrap().id("gem:opal").is_some());
    assert!(root.join(crate::content_identity::MARKER).exists());
    let base_defaults = sunrise_world::item_defaults::read(&base).unwrap();
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    assert_ne!(
        crate::content_identity::identity(&root, &defaults).unwrap(),
        crate::content_identity::identity(&base, &base_defaults).unwrap()
    );
    fs::remove_dir_all(&scratch).unwrap();
}

/// a living mod type's row in `entity-types.json` is its template's with `template`,
/// attribute bases and passenger attachments; other values keep the template's text.
/// a mod's thrown entity type gets the network id after vanilla's last, the
/// commands accept it, and it changes the content identity.
#[test]
#[ignore = "daily"]
fn content_root_registers_a_mod_entity_type() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-entity-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("example"),
        &[
            ("sunrise-mod.toml", &manifest("example", "")),
            (
                "data/example/sunrise/item/gem.json",
                r#"{"template": "minecraft:snowball"}"#,
            ),
            (
                "data/example/sunrise/entity_type/bolt.json",
                r#"{"template": "minecraft:snowball", "item": "example:gem", "damage": 2}"#,
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
    let report = |root: &Path| {
        sunrise_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap()
    };
    let (vanilla, ids) = (report(&base), report(&root));
    let vanilla = vanilla.get("minecraft:entity_type").unwrap();
    let ids = ids.get("minecraft:entity_type").unwrap();
    let count = (0..).take_while(|&i| vanilla.name(i).is_some()).count() as i32;
    assert_eq!(ids.id("example:bolt"), Some(count));
    assert_eq!(
        ids.id("minecraft:snowball"),
        vanilla.id("minecraft:snowball")
    );
    let commands: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("command-registries.json")).unwrap()).unwrap();
    for ids in [
        &commands["entity_types"],
        &commands["by_registry"]["minecraft:entity_type"]["ids"],
    ] {
        assert!(ids.as_array().unwrap().contains(&"example:bolt".into()));
    }
    // its size and eyes are the snowball's, for `execute anchored eyes` and spawners.
    let types: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("entity-types.json")).unwrap()).unwrap();
    assert!(types["minecraft:snowball"].is_object());
    let mut bolt = types["example:bolt"].clone();
    assert_eq!(bolt["template"], "minecraft:snowball");
    bolt.as_object_mut().unwrap().remove("template");
    assert_eq!(bolt, types["minecraft:snowball"]);
    // the rewritten table keeps vanilla's floats to the bit (a zombie's speed).
    let speed = |root: &Path| {
        let attributes =
            crate::mobs::exact_attributes(&fs::read(root.join("entity-types.json")).unwrap());
        attributes["minecraft:zombie"]["minecraft:movement_speed"]
    };
    assert_eq!(speed(&root).to_bits(), speed(&base).to_bits());
    assert!(root.join(crate::content_identity::MARKER).exists());
    let identity = crate::content_identity::identity;
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let base_defaults = sunrise_world::item_defaults::read(&base).unwrap();
    assert_ne!(
        identity(&root, &defaults).unwrap(),
        identity(&base, &base_defaults).unwrap()
    );
    fs::remove_dir_all(&scratch).unwrap();
}

/// a mod's menu type gets the menu network id after vanilla's last, and it
/// changes the content identity.
#[test]
#[ignore = "daily"]
fn content_root_registers_a_mod_menu_type() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-menu-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("example"),
        &[
            ("sunrise-mod.toml", &manifest("example", "")),
            (
                "data/example/sunrise/menu/press.json",
                r#"{"slots": 9, "data_slots": 2, "results": [7, 8]}"#,
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
    let report = |root: &Path| {
        sunrise_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap()
    };
    let (vanilla, ids) = (report(&base), report(&root));
    let vanilla = vanilla.get("minecraft:menu").unwrap();
    let ids = ids.get("minecraft:menu").unwrap();
    let count = (0..).take_while(|&i| vanilla.name(i).is_some()).count() as i32;
    assert_eq!(ids.id("example:press"), Some(count));
    assert_eq!(ids.id("minecraft:furnace"), vanilla.id("minecraft:furnace"));
    assert!(root.join(crate::content_identity::MARKER).exists());
    let identity = crate::content_identity::identity;
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let base_defaults = sunrise_world::item_defaults::read(&base).unwrap();
    assert_ne!(
        identity(&root, &defaults).unwrap(),
        identity(&base, &base_defaults).unwrap()
    );
    fs::remove_dir_all(&scratch).unwrap();
}

/// A mod whose files sit under namespaces besides its id (Towns and Towers'
/// `kaisyn`): they join the root, and loaders read those namespaces.
#[test]
fn listed_namespaces_join_the_root() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-ns-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let toml = manifest("towers", "\"kaisyn\"");
    let m = folder_mod(
        &scratch.join("towers"),
        &[
            ("sunrise-mod.toml", &toml),
            (
                "data/kaisyn/worldgen/processor_list/mossy.json",
                r#"{"processors": []}"#,
            ),
            (
                "data/kaisyn/tags/worldgen/biome/has_tower.json",
                r#"{"values": ["minecraft:plains"]}"#,
            ),
            (
                "assets/kaisyn/lang/en_us.json",
                r#"{"block.kaisyn.x": "X"}"#,
            ),
        ],
    );
    let root = scratch.join("root");
    build_fixture(&base, Path::new(""), &[m], &root).unwrap();
    assert_eq!(namespaces(&root), ["kaisyn", "towers"]);
    assert!(tag_namespaces(&root).contains(&"kaisyn".to_owned()));
    let lists: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/processor_lists.json")).unwrap())
            .unwrap();
    assert!(lists.contains_key("kaisyn:mossy"));
    let tags: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/biome_tags.json")).unwrap()).unwrap();
    assert!(tags.contains_key("kaisyn:has_tower"));
    let _ = fs::remove_dir_all(&scratch);
}

/// Two mods owning one namespace (one's listed namespace is the other's id,
/// or both list it) are refused together, naming both.
#[test]
fn mods_sharing_a_namespace_are_refused() {
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-clash-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let towers = folder_mod(
        &scratch.join("towers"),
        &[("sunrise-mod.toml", &manifest("towers", "\"kaisyn\""))],
    );
    let kaisyn = folder_mod(
        &scratch.join("kaisyn"),
        &[("sunrise-mod.toml", &manifest("kaisyn", ""))],
    );
    let other = folder_mod(
        &scratch.join("other"),
        &[("sunrise-mod.toml", &manifest("other", "\"kaisyn\""))],
    );
    // The check runs before the base is read.
    let base = scratch.join("no-base");
    let out = scratch.join("root");
    let e = build_fixture(&base, Path::new(""), &[towers, kaisyn], &out).unwrap_err();
    assert_eq!(
        e.to_string(),
        "mods towers and kaisyn both use the namespace kaisyn; turn one of them off"
    );
    let towers = folder_mod(
        &scratch.join("towers"),
        &[("sunrise-mod.toml", &manifest("towers", "\"kaisyn\""))],
    );
    let e = build_fixture(&base, Path::new(""), &[towers, other], &out).unwrap_err();
    assert!(e.to_string().starts_with("mods towers and other"), "{e}");
    let _ = fs::remove_dir_all(&scratch);
}

#[path = "mod_content_tests_tests.rs"]
mod more;

/// A replacing mod's tag leaves vanilla's entries out of the tag file and of
/// the worldgen loader's biome tags, and the root keeps the warning for the
/// starts that reuse it.
#[test]
fn a_replacing_mod_tag_reaches_the_root() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-replace-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let file = "data/minecraft/tags/worldgen/biome/has_structure/village_plains.json";
    let adds = folder_mod(
        &scratch.join("adds"),
        &[
            ("sunrise-mod.toml", &manifest("adds", "")),
            (file, r#"{"values": ["minecraft:forest"]}"#),
        ],
    );
    let towns = folder_mod(
        &scratch.join("towns"),
        &[
            ("sunrise-mod.toml", &manifest("towns", "")),
            (file, r#"{"replace": true, "values": ["minecraft:plains"]}"#),
        ],
    );
    let late = folder_mod(
        &scratch.join("late"),
        &[
            ("sunrise-mod.toml", &manifest("late", "")),
            (file, r#"{"values": ["minecraft:savanna"]}"#),
        ],
    );
    let vanilla: serde_json::Value =
        serde_json::from_slice(&fs::read(base.join(file)).unwrap()).unwrap();
    assert!(vanilla["values"].as_array().unwrap().len() > 1, "{vanilla}");
    let root = resource_root_fixture(
        &base,
        Path::new(""),
        &[adds, towns, late],
        &scratch.join("cache"),
    )
    .unwrap();
    let expected = serde_json::json!(["minecraft:plains", "minecraft:savanna"]);
    let tag: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(file)).unwrap()).unwrap();
    assert_eq!(tag["values"], expected);
    let tags: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/biome_tags.json")).unwrap()).unwrap();
    let bundled: serde_json::Value =
        serde_json::from_str(&tags["minecraft:has_structure/village_plains"]).unwrap();
    assert_eq!(bundled["values"], expected);
    let warnings = fs::read_to_string(root.join(WARNINGS)).unwrap();
    assert!(
        warnings.starts_with("mod towns replaces data/minecraft/tags/worldgen/biome/has_structure/village_plains.json: vanilla's entries and those of adds are left out"),
        "{warnings}"
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// `StructureCheck.onStructureLoad`: the references a chunk saved for its start
/// count, so a map skips a start a vanilla map already pointed at.
#[test]
#[ignore = "daily"]
fn exploration_maps_skip_the_starts_a_chunk_saved_as_referenced() {
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(base.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&base, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: None,
        dir: &base.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let terrain = crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
    let names = ["minecraft:buried_treasure".to_owned()];
    let first = terrain
        .locate_for_map(&names, [0, 64, 0], 50, true)
        .unwrap();
    // Another terrain, on the same seed, whose chunks saved one reference for it.
    let fresh = crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
    let (name, at) = (first.0.clone(), (first.1[0] >> 4, first.1[2] >> 4));
    fresh.set_saved_references(std::sync::Arc::new(move |n, x, z| {
        i32::from(n == name && (x, z) == at)
    }));
    let next = fresh.locate_for_map(&names, [0, 64, 0], 50, true).unwrap();
    assert_ne!(next.1, first.1, "the saved reference is skipped");
    assert_eq!(
        fresh.locate_for_map(&names, [0, 64, 0], 50, false).unwrap(),
        first,
        "a map that does not skip still finds it"
    );
}

/// A skip check reads each saved chunk's record once: the session keeps every
/// answer (as vanilla's `StructureCheck` does), so a second map searching past
/// the same chunks asks the save nothing it asked before (#10255: a chest with
/// exploration maps took a growing second per open reading them again).
#[test]
#[ignore = "daily"]
fn a_saved_chunks_record_is_read_once_across_map_searches() {
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(base.clone()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&base, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain = crate::vanillagen::VanillaTerrain::new(
        crate::vanillagen::TerrainResources {
            data: None,
            dir: &base.join("worldgen"),
            registry: assets.registry,
            biomes: &biomes,
            item_data: &item_data,
        },
        1234,
    )
    .unwrap();
    let names = ["minecraft:buried_treasure".to_owned()];
    use std::sync::atomic::{AtomicUsize, Ordering};
    let reads = std::sync::Arc::new(AtomicUsize::new(0));
    let asked: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<(String, i32, i32)>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Default::default()));
    let (reads_of, asked_of) = (reads.clone(), asked.clone());
    terrain.set_saved_references(std::sync::Arc::new(move |n, x, z| {
        reads_of.fetch_add(1, Ordering::Relaxed);
        asked_of.lock().unwrap().insert((n.to_owned(), x, z));
        0
    }));
    terrain
        .locate_for_map(&names, [0, 64, 0], 50, true)
        .unwrap();
    // The same search again, now skipping the start the first pointed at:
    // it passes every chunk the first search read and may reach new ones,
    // but no chunk's record is read a second time.
    terrain
        .locate_for_map(&names, [0, 64, 0], 50, true)
        .unwrap();
    assert!(
        reads.load(Ordering::Relaxed) > 0,
        "the searches asked the save at all"
    );
    assert_eq!(
        reads.load(Ordering::Relaxed),
        asked.lock().unwrap().len(),
        "no saved chunk's record is read twice"
    );
}

/// `ChunkGenerator.findNearestMapStructure` with `createReference`: an
/// exploration map that skips known structures points at a start no map
/// pointed at before, and a reopened world remembers which.
#[test]
#[ignore = "daily"]
fn exploration_maps_skip_the_structures_maps_pointed_at() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-map-refs-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(base.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&base, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: None,
        dir: &base.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let open = || {
        let terrain = crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
        terrain.set_reference_file(scratch.join("refs.json"));
        terrain
    };
    let names = ["minecraft:buried_treasure".to_owned()];
    let terrain = open();
    let first = terrain
        .locate_for_map(&names, [0, 64, 0], 50, true)
        .unwrap();
    assert_eq!(
        terrain.locate(&names, [0, 64, 0], 50).unwrap(),
        first,
        "/locate still finds the nearest"
    );
    let second = terrain
        .locate_for_map(&names, [0, 64, 0], 50, true)
        .unwrap();
    assert_ne!(second.1, first.1, "the second map points elsewhere");
    let again = terrain
        .locate_for_map(&names, [0, 64, 0], 50, false)
        .unwrap();
    assert_eq!(
        again, first,
        "without skipping the nearest is still the first"
    );
    drop(terrain);
    let third = open().locate_for_map(&names, [0, 64, 0], 50, true).unwrap();
    assert!(
        third.1 != first.1 && third.1 != second.1,
        "a reopened world remembers {first:?} and {second:?}, found {third:?}"
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// `NoiseBasedChunkGenerator.addDebugScreenInfo`: F3's `Density` line lists
/// the noise settings' `debug_functions` with three decimals.
#[test]
#[ignore = "daily"]
fn debug_density_line_lists_the_noise_settings_functions() {
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(base.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&base, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: None,
        dir: &base.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let terrain = crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
    let line = terrain.density_debug_line([0, 70, 0]).unwrap();
    let labels: Vec<&str> = line
        .strip_prefix("Density ")
        .unwrap()
        .split(' ')
        .step_by(2)
        .collect();
    assert_eq!(
        labels,
        ["N:", "T:", "V:", "C:", "E:", "D:", "W:", "PV:", "PS:"],
        "{line}"
    );
    assert!(
        line.split(' ')
            .skip(2)
            .step_by(2)
            .all(|v| v.split('.').nth(1).is_some_and(|d| d.len() == 3)),
        "{line}"
    );
}

#[test]
#[ignore = "daily"]
fn content_shapes_drive_outline_and_collision() {
    use sunrise_core::{
        geometry::{Aabb, Axis, Vec3},
        position::BlockPos,
    };
    use sunrise_world::{collision::CachedCollisionRegistry, registry::BlockStateId};
    let Some(base) = prepared() else {
        return;
    };
    let scratch =
        std::env::temp_dir().join(format!("sunrise-content-shapes-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("mod"),
        &[
            ("sunrise-mod.toml", &manifest("shapetest", "")),
            (
                "data/shapetest/sunrise/block/rod.json",
                r#"{"template":"minecraft:oak_log","shapes":{"properties":["axis"],"values":{"x":[[0,0.25,0.25,1,0.75,0.75]],"y":[[0.25,0,0.25,0.75,1,0.75]],"z":[[0.25,0.25,0,0.75,0.75,1]]}}}"#,
            ),
            (
                "data/shapetest/sunrise/block/plain.json",
                r#"{"template":"minecraft:oak_log"}"#,
            ),
        ],
    );
    let root = resource_root_fixture(&base, Path::new(""), &[m], &scratch.join("cache")).unwrap();
    let assets = Assets::load(&root).unwrap();
    let registry = assets.registry;
    let facts = BlockFacts::load(&root, registry).unwrap();
    let (named, _) = nbt::read_named(
        &fs::read(root.join("cached-collision.nbt")).unwrap(),
        nbt::Limits::tables(),
    )
    .unwrap();
    let collision = CachedCollisionRegistry::from_nbt(&named.tag, registry).unwrap();
    for axis in ["x", "y", "z"] {
        let state = (0..registry.len())
            .map(|i| BlockStateId(i as u32))
            .find(|id| {
                let s = registry.state(*id).unwrap();
                s.name == "shapetest:rod" && s.properties.get("axis").is_some_and(|v| v == axis)
            })
            .unwrap();
        let shape = collision.cached_shape(state).unwrap().unwrap();
        let f = facts.get(state);
        assert_eq!(shape.boxes().len(), 1);
        assert_eq!(f.outline.len(), 1);
        assert_eq!(f.outline_edges.len(), 12);
        let expected = match axis {
            "x" => [0., 0.25, 0.25, 1., 0.75, 0.75],
            "y" => [0.25, 0., 0.25, 0.75, 1., 0.75],
            _ => [0.25, 0.25, 0., 0.75, 0.75, 1.],
        };
        assert_eq!(f.outline[0], expected);
        let start = Vec3::new(-1., 0.5, 0.5);
        let end = Vec3::new(2., 0.5, 0.5);
        let target = BlockPos::new(0, 0, 0);
        assert!(shape.clip(start, end, target).is_some());
        let bounds = f.outline[0];
        let outline = Aabb::new(
            Vec3::new(bounds[0], bounds[1], bounds[2]),
            Vec3::new(bounds[3], bounds[4], bounds[5]),
        );
        assert_eq!(
            shape.clip(start, end, target),
            Aabb::clip_boxes([outline], start, end, target)
        );
        let player = Aabb::new(Vec3::new(-1., 0.4, 0.4), Vec3::new(-0.2, 0.6, 0.6));
        assert!((shape.collide(Axis::X, player, 2.) - (expected[0] + 0.2)).abs() < 1e-9);
        let miss = Aabb::new(Vec3::new(-1., 0.05, 0.05), Vec3::new(-0.2, 0.15, 0.15));
        assert_eq!(shape.collide(Axis::X, miss, 2.), 2.);
    }
    let plain = registry.default_state("shapetest:plain").unwrap();
    let oak = registry.default_state("minecraft:oak_log").unwrap();
    assert_eq!(facts.get(plain).outline, facts.get(oak).outline);
    assert_eq!(
        collision.cached_shape(plain).unwrap().unwrap().boxes(),
        collision.cached_shape(oak).unwrap().unwrap().boxes()
    );
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn content_block_default_state_is_validated_and_overlaid() {
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-default-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let base = scratch.join("base");
    fs::create_dir_all(&base).unwrap();
    let facts = r#"{"states":[
      ["minecraft:piston",false,false,"",{"id":"minecraft:piston","properties":{"facing":"up","extended":"false"}}],
      ["minecraft:piston",false,false,"",{"id":"minecraft:piston","properties":{"facing":"north","extended":"false"}}],
      ["minecraft:piston",false,false,"",{"id":"minecraft:piston","properties":{"facing":"north","extended":"true"}}]],
      "defaults":{"minecraft:piston":0}}"#;
    fs::write(base.join("facts.json"), facts).unwrap();
    fs::write(base.join("gameplay-facts.json"), r#"{"states":[{"item":"minecraft:piston"},{"item":"minecraft:piston"},{"item":"minecraft:piston"}]}"#).unwrap();
    fs::write(base.join("items.json"), r#"{"minecraft:piston":{}}"#).unwrap();
    fs::write(base.join("registries.json"), r#"{"minecraft:item":{"entries":{"minecraft:piston":{"protocol_id":0}}},"minecraft:block":{"entries":{"minecraft:piston":{"protocol_id":0}}}}"#).unwrap();
    for (declaration, expected) in [
        (r#"{"template":"minecraft:piston"}"#, Some(3)),
        (
            r#"{"template":"minecraft:piston","default_state":{"facing":"north"}}"#,
            Some(4),
        ),
        (
            r#"{"template":"minecraft:piston","default_state":{"facing":"north","extended":"true"}}"#,
            Some(5),
        ),
        (
            r#"{"template":"minecraft:piston","default_state":{"facing":"sideways"}}"#,
            None,
        ),
        (
            r#"{"template":"minecraft:piston","default_state":{"direction":"north"}}"#,
            None,
        ),
    ] {
        let m = folder_mod(
            &scratch.join("mod"),
            &[
                ("sunrise-mod.toml", &manifest("testmod", "")),
                ("data/testmod/sunrise/block/test.json", declaration),
            ],
        );
        let mods = [m];
        match (Plan::new(&base, &mods), expected) {
            (Ok(plan), Some(id)) => {
                let mut doc: J = serde_json::from_str(facts).unwrap();
                plan.json("facts.json", &mut doc).unwrap();
                assert_eq!(
                    doc.get("defaults").unwrap().get("testmod:test"),
                    Some(&J::Num(id.into()))
                );
            }
            (Err(e), None) => assert!(e.to_string().contains("testmod:test"), "{e}"),
            _ => panic!("unexpected result for {declaration}"),
        }
    }
    // A malformed registry with duplicate states must not choose one silently.
    let duplicate = facts.replace(
        r#"["minecraft:piston",false,false,"",{"id":"minecraft:piston","properties":{"facing":"north","extended":"true"}}]"#,
        r#"["minecraft:piston",false,false,"",{"id":"minecraft:piston","properties":{"facing":"north","extended":"false"}}]"#,
    );
    fs::write(base.join("facts.json"), duplicate).unwrap();
    let m = folder_mod(
        &scratch.join("mod"),
        &[
            ("sunrise-mod.toml", &manifest("testmod", "")),
            (
                "data/testmod/sunrise/block/test.json",
                r#"{"template":"minecraft:piston","default_state":{"facing":"north"}}"#,
            ),
        ],
    );
    let error = match Plan::new(&base, &[m]) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("duplicate states accepted"),
    };
    assert!(error.contains("matches 2 states"), "{error}");
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn shapes_reject_unknown_or_missing_template_values() {
    let Some(base) = prepared() else { return };
    let scratch =
        std::env::temp_dir().join(format!("sunrise-shape-invalid-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    for declaration in [
        r#"{"template":"minecraft:oak_log","shapes":{"properties":["typo"],"values":{"x":[]}}}"#,
        r#"{"template":"minecraft:oak_log","shapes":{"properties":["axis"],"values":{"x":[]}}}"#,
        r#"{"template":"minecraft:oak_log","shapes":{"properties":["axis"],"values":{"x":[],"y":[],"z":[],"bad":[]}}}"#,
    ] {
        let m = folder_mod(
            &scratch.join("mod"),
            &[
                ("sunrise-mod.toml", &manifest("shapetest", "")),
                ("data/shapetest/sunrise/block/rod.json", declaration),
            ],
        );
        let mods = [m];
        let e = match Plan::new(&base, &mods) {
            Ok(_) => panic!("accepted {declaration}"),
            Err(e) => e.to_string(),
        };
        assert!(e.contains("shapetest:rod"), "{e}");
    }
    fs::remove_dir_all(scratch).unwrap();
}

/// A connecting block (template `minecraft:chorus_plant`, `connects_to` a
/// tag) joins itself and its tag's blocks on each side, needs no support, and
/// leaves vanilla's chorus plant as it was.
#[test]
#[ignore = "daily"]
fn connecting_blocks_and_vanilla_fences_use_content_tags() {
    use std::collections::HashMap;
    use sunrise_core::position::BlockPos;
    use sunrise_core::random::LegacyRandom;
    use sunrise_world::level::{Behaviours, LevelAccess, Ticks};

    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-pipe-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let dir = scratch.join("cable");
    for (name, text) in [
        (
            "sunrise-mod.toml",
            "id = \"cable\"\nname = \"Cable\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        ),
        (
            "data/cable/sunrise/block/tin_cable.json",
            r##"{"template": "minecraft:chorus_plant", "connects_to": "#cable:joins", "thickness": 4}"##,
        ),
        (
            "data/cable/sunrise/block/machine.json",
            r#"{"template": "minecraft:iron_block"}"#,
        ),
        (
            "data/cable/tags/block/joins.json",
            r#"{"values": ["cable:machine"]}"#,
        ),
        (
            "data/cable/sunrise/block/wooden_post.json",
            r#"{"template": "minecraft:end_rod"}"#,
        ),
        (
            "data/cable/sunrise/block/nether_post.json",
            r#"{"template": "minecraft:end_rod"}"#,
        ),
        (
            "data/cable/sunrise/block/untagged_post.json",
            r#"{"template": "minecraft:end_rod"}"#,
        ),
        (
            "data/minecraft/tags/block/fences.json",
            r#"{"values": ["cable:wooden_post", "cable:nether_post"]}"#,
        ),
        (
            "data/minecraft/tags/block/wooden_fences.json",
            r#"{"values": ["cable:wooden_post"]}"#,
        ),
    ] {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let package = sunrise_mod_package::Package::read(&dir).unwrap();
    let m = ContentMod {
        id: "cable".into(),
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
    let assets = Assets::load(&root).unwrap();
    let registry = assets.registry;
    let b = Behaviours::load(registry, &root).unwrap();

    struct Level {
        blocks: HashMap<BlockPos, BlockStateId>,
        air: BlockStateId,
        random: LegacyRandom,
    }
    impl sunrise_world::level::BlockEntityNeighbors for Level {}
    impl LevelAccess for Level {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.blocks.get(&pos).copied().unwrap_or(self.air)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            Some(self.blocks.insert(pos, state).unwrap_or(self.air))
        }
        fn is_loaded(&self, _pos: BlockPos) -> bool {
            true
        }
        fn random(&mut self) -> &mut LegacyRandom {
            &mut self.random
        }
        fn game_time(&self) -> i64 {
            0
        }
    }
    let air = registry.default_state("minecraft:air").unwrap();
    let mut level = Level {
        blocks: HashMap::new(),
        air,
        random: LegacyRandom::new(1),
    };
    let mut ticks = Ticks::new(0);
    ticks
        .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
        .unwrap();
    let sides = |state: BlockStateId| {
        let s = registry.state(state).unwrap();
        ["down", "up", "north", "south", "west", "east"]
            .into_iter()
            .filter(|k| s.properties.get(*k).map(String::as_str) == Some("true"))
            .collect::<Vec<_>>()
    };
    let at = |x, y, z| BlockPos::new(x, y, z);
    let machine = registry.default_state("cable:machine").unwrap();
    let cable = registry.default_state("cable:tin_cable").unwrap();
    // A cable beside a machine (in its tag) and a stone joins the machine only,
    // and floats: no chorus plant support.
    b.set_block(&mut level, &mut ticks, at(1, 64, 0), machine, 3);
    b.set_block(
        &mut level,
        &mut ticks,
        at(-1, 64, 0),
        registry.default_state("minecraft:stone").unwrap(),
        3,
    );
    let placed = b.update_from_neighbour_shapes(&mut level, &mut ticks, at(0, 64, 0), cable);
    b.set_block(&mut level, &mut ticks, at(0, 64, 0), placed, 3);
    assert_eq!(sides(level.block_state(at(0, 64, 0))), ["east"]);
    assert!(b.can_survive(&mut level, &mut ticks, at(0, 64, 0), placed));
    // A second cable above: both join, through the neighbour's shape update.
    let above = b.update_from_neighbour_shapes(&mut level, &mut ticks, at(0, 65, 0), cable);
    b.set_block(&mut level, &mut ticks, at(0, 65, 0), above, 3);
    assert_eq!(sides(level.block_state(at(0, 65, 0))), ["down"]);
    assert_eq!(sides(level.block_state(at(0, 64, 0))), ["up", "east"]);
    // The machine gone: the arm goes too.
    b.set_block(&mut level, &mut ticks, at(1, 64, 0), air, 3);
    assert_eq!(sides(level.block_state(at(0, 64, 0))), ["up"]);
    // Its shapes are 4 pixels across: the outline and the cached collision.
    let joined = level.block_state(at(0, 64, 0));
    let thin = sunrise_world::collision::pipe_shape(4.0, [false, true, false, false, false, false])
        .unwrap();
    let facts = BlockFacts::load(&root, registry).unwrap();
    let outline = &facts.get(joined).outline;
    assert_eq!(outline.len(), thin.boxes().len());
    let file =
        sunrise_world::registry::BlockTableFile::open(&root.join("cached-collision.nbt"), registry)
            .unwrap();
    let cached = sunrise_world::collision::CachedCollisionRegistry::from_table(
        file.table("collision").unwrap(),
    )
    .unwrap();
    assert!(cached.has_canonical_identities());
    let shape = cached.cached_shape(joined).unwrap().unwrap();
    assert_eq!(shape.boxes(), thin.boxes());
    assert_eq!(
        shape
            .bounds()
            .map(|b| b.min(sunrise_core::geometry::Axis::X)),
        Some(0.375)
    );
    // Vanilla's chorus plant keeps its own rules: it doesn't join the machine.
    b.set_block(&mut level, &mut ticks, at(5, 64, 0), machine, 3);
    let chorus = registry.default_state("minecraft:chorus_plant").unwrap();
    let chorus = b.update_from_neighbour_shapes(&mut level, &mut ticks, at(4, 64, 0), chorus);
    assert!(sides(chorus).is_empty());

    // The same non-sturdy template joins vanilla fences only through the matching tag.
    let oak = registry.default_state("minecraft:oak_fence").unwrap();
    let nether = registry
        .default_state("minecraft:nether_brick_fence")
        .unwrap();
    let fence_pos = at(8, 64, 0);
    let post_pos = at(9, 64, 0);
    for (id, joins_oak, joins_nether) in [
        ("cable:untagged_post", false, false),
        ("cable:wooden_post", true, false),
        ("cable:nether_post", false, true),
    ] {
        let post = registry.default_state(id).unwrap();
        assert!(!b.is_face_sturdy(post, sunrise_core::geometry::Direction::West));
        b.set_block(&mut level, &mut ticks, post_pos, post, 3);
        for (fence, joins) in [(oak, joins_oak), (nether, joins_nether)] {
            let state = b.update_from_neighbour_shapes(&mut level, &mut ticks, fence_pos, fence);
            assert_eq!(sides(state).contains(&"east"), joins, "{id}");
        }
    }
    fs::remove_dir_all(&scratch).unwrap();
}

/// An old chunk's saved jigsaw start comes back with its pieces (`StructureStart.loadStaticStart`):
/// a village this seed makes, saved as an older version would have, is loaded from its `Children`
/// and has the same pieces, boxes and positions, so the neighbours can place what crosses into
/// them. A legacy structure with no loader is the generator's own start when it has the same
/// pieces' box, and name and box alone when it has not.
#[test]
#[ignore = "daily"]
fn an_old_chunks_saved_village_comes_back_with_its_pieces() {
    use sunrise_core::position::ChunkPos;
    use {
        sunrise_core::nbt::Tag,
        sunrise_world::worldgen::blending::{OldWorld, SavedStart},
    };
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(base.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&base, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: None,
        dir: &base.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let open = || crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
    let villages: Vec<String> = ["plains", "desert", "savanna", "snowy", "taiga"]
        .iter()
        .map(|k| format!("minecraft:village_{k}"))
        .collect();
    let made = open();
    let (name, at) = made
        .locate(&villages, [0, 64, 0], 100)
        .expect("a village near the origin");
    let (cx, cz) = (at[0] >> 4, at[2] >> 4);
    let original = made
        .starts_at(cx, cz)
        .into_iter()
        .find(|s| s.name == name)
        .expect("the village start");
    assert!(original.pieces.len() > 1);
    // Saved the way a chunk keeps it: the start's tag, its Children and their union box.
    let Some(Tag::Compound(saved)) = original.save() else {
        panic!("the village saves");
    };
    let Some(Tag::List(children)) = saved.get(&"Children".into()).cloned() else {
        panic!("Children");
    };
    let bounds = original
        .pieces
        .iter()
        .map(|p| p.bounding_box())
        .fold(None::<[i32; 6]>, |u, b| {
            let [x0, y0, z0, x1, y1, z1] =
                [b.min[0], b.min[1], b.min[2], b.max[0], b.max[1], b.max[2]];
            Some(match u {
                None => [x0, y0, z0, x1, y1, z1],
                Some(u) => [
                    u[0].min(x0),
                    u[1].min(y0),
                    u[2].min(z0),
                    u[3].max(x1),
                    u[4].max(y1),
                    u[5].max(z1),
                ],
            })
        });
    struct Saved {
        chunk: (i32, i32),
        start: SavedStart,
    }
    impl OldWorld for Saved {
        fn is_old_chunk_around(&self, center: ChunkPos, range: i32) -> bool {
            (center.x - self.chunk.0).abs() <= range && (center.z - self.chunk.1).abs() <= range
        }
        fn blending(
            &self,
            _: i32,
            _: i32,
        ) -> Option<std::sync::Arc<sunrise_world::worldgen::blending::BlendingData>> {
            None
        }
        fn old_biomes(
            &self,
            _: i32,
            _: i32,
        ) -> Option<
            std::sync::Arc<
                sunrise_world::worldgen::biome_chunk::BiomeChunk<
                    sunrise_world::worldgen::biome::BiomeId,
                >,
            >,
        > {
            None
        }
        fn saved_starts(&self, x: i32, z: i32) -> Vec<SavedStart> {
            if (x, z) == self.chunk {
                vec![self.start.clone()]
            } else {
                Vec::new()
            }
        }
    }
    let loaded = open();
    loaded.set_old_world(std::sync::Arc::new(Saved {
        chunk: (cx, cz),
        // No box to compare: the pieces can only come from the Children.
        start: SavedStart {
            name: name.clone(),
            bounds: None,
            pieces: children.clone(),
        },
    }));
    let back = loaded
        .starts_at(cx, cz)
        .into_iter()
        .find(|s| s.name == name)
        .expect("the saved start");
    assert_eq!(
        back.pieces.len(),
        original.pieces.len(),
        "every piece is loaded"
    );
    for (a, b) in original.pieces.iter().zip(&back.pieces) {
        assert_eq!(a.bounding_box(), b.bounding_box());
        assert_eq!(a.type_name(), b.type_name());
        assert_eq!(a.save(), b.save(), "a piece saves as it was loaded");
    }
    assert_eq!(back.bounding_box, original.bounding_box);
    // No pieces to load and a box no generated start has: the name and the box only.
    let odd = open();
    odd.set_old_world(std::sync::Arc::new(Saved {
        chunk: (cx, cz),
        start: SavedStart {
            name: name.clone(),
            bounds: Some([0, 0, 0, 1, 1, 1]),
            pieces: Vec::new(),
        },
    }));
    let stub = odd
        .starts_at(cx, cz)
        .into_iter()
        .find(|s| s.name == name)
        .unwrap();
    assert!(stub.pieces.is_empty());
    assert_eq!(stub.bounding_box.min, [0, 0, 0]);
    // The box of the start this generator makes there: it is that start.
    let same = open();
    same.set_old_world(std::sync::Arc::new(Saved {
        chunk: (cx, cz),
        start: SavedStart {
            name: name.clone(),
            bounds,
            pieces: Vec::new(),
        },
    }));
    let rebuilt = same
        .starts_at(cx, cz)
        .into_iter()
        .find(|s| s.name == name)
        .unwrap();
    assert_eq!(rebuilt.pieces.len(), original.pieces.len());
}

/// A recipe's `result` is a stack's fields or a bare item id, as vanilla's `ItemStackTemplate`
/// codec reads it (#4904): a mod's recipes make its own items either way.
#[test]
#[ignore = "daily"]
fn mod_recipe_results_are_an_item_stack_or_a_bare_item_id() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-results-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let dir = scratch.join("ruby");
    let _ = ruby_mod(&dir);
    // Each recipe takes one item no vanilla recipe crafts alone and makes one of the mod's.
    let cases = [
        (
            "stack",
            "minecraft:diamond",
            r#"{"id": "ruby:ruby"}"#,
            "ruby:ruby",
            1,
        ),
        (
            "bare_id",
            "minecraft:flint",
            r#""ruby:ruby""#,
            "ruby:ruby",
            1,
        ),
        (
            "bare_block_item",
            "minecraft:ghast_tear",
            r#""ruby:ruby_block""#,
            "ruby:ruby_block",
            1,
        ),
        (
            "counted",
            "minecraft:ender_pearl",
            r#"{"id": "ruby:ruby_block", "count": 3}"#,
            "ruby:ruby_block",
            3,
        ),
    ];
    for (name, input, result, _, _) in cases {
        let recipe = format!(
            r#"{{"type": "minecraft:crafting_shapeless", "ingredients": ["{input}"], "result": {result}}}"#
        );
        fs::write(dir.join(format!("data/ruby/recipe/{name}.json")), recipe).unwrap();
    }
    let package = sunrise_mod_package::Package::read(&dir).unwrap();
    let m = ContentMod {
        id: "ruby".into(),
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
    let assets = Assets::load(&root).unwrap();
    let new = assets.registry;
    let facts = BlockFacts::load(&root, new).unwrap();
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let items = Items::load(&root, new, &facts, &overlay, &defaults).unwrap();
    let recipes = crate::crafting::Recipes::load(
        &root,
        &items,
        &overlay,
        &crate::test_resources::server_load(),
    )
    .unwrap();
    for (name, input, _, item, count) in cases {
        let grid = [
            Some(crate::items::ItemStack::new(input, 1)),
            None,
            None,
            None,
        ];
        let (_, made) = recipes
            .find(crate::test_resources::server_loop_context(), &grid, 2)
            .unwrap_or_else(|| panic!("{name}: no recipe"));
        assert_eq!((&*made.item, made.count), (item, count), "{name}");
    }
    fs::remove_dir_all(&scratch).unwrap();
}

/// A block that gives each of its template's states a run moves a template
/// state to the first of its own: the combination every declared property is
/// at its default for, which is the one the id cursor points at first.
#[test]
fn a_fan_out_rename_moves_a_template_state_to_its_first_state() {
    let mut r = Rename::new("minecraft:test", "testmod:test");
    r.states = 10..12;
    r.first = 100;
    r.fanout = 6;
    assert_eq!(
        (r.id(9), r.id(10), r.id(11), r.id(12)),
        (None, Some(100), Some(106), None)
    );
}

#[test]
fn tuple_shapes_follow_final_state_order_and_separate_collision() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"active":{"type":"boolean"}},
      "shapes":{"properties":["axis","active"],
        "values":{"x,true":[[0,0,0,0.5,1,1]]},"default":[[0,0,0,1,1,1]]},
      "collision_shapes":{"properties":["active","axis"],
        "values":{"true,x":[[0,0,0,0.25,1,1]]},"default":[]}}"#;
    let (base, mods) = block_base("tuple-shapes", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let block = &plan.blocks[0];
    let shapes = block.shapes.as_ref().unwrap();
    let collisions = block.collision_shapes.as_ref().unwrap();
    assert_eq!(shapes.len(), 4);
    assert_eq!(collisions.len(), 4);
    for (index, (template, combo)) in block.state_combinations().enumerate() {
        let special = template == 0
            && combo
                .iter()
                .any(|(name, value)| name == "active" && value == "true");
        assert_eq!(
            shapes[index].bounds().unwrap().max(Axis::X),
            if special { 0.5 } else { 1.0 }
        );
        assert_eq!(collisions[index].is_empty(), !special);
        if special {
            assert_eq!(collisions[index].bounds().unwrap().max(Axis::X), 0.25);
        }
    }
    let mut gameplay: J =
        serde_json::from_slice(&fs::read(base.join("gameplay-facts.json")).unwrap()).unwrap();
    plan.json("gameplay-facts.json", &mut gameplay).unwrap();
    let Some(J::Arr(rows)) = gameplay.get("states") else {
        panic!("states")
    };
    for (index, row) in rows[2..].iter().enumerate() {
        assert_eq!(row.get("full_collision"), Some(&J::Bool(false)));
        let Some(J::Arr(outline)) = row.get("outline") else {
            panic!("outline")
        };
        assert_eq!(outline.len(), shapes[index].boxes().len());
    }
    let mut tag = Tag::Compound(Compound::from([
        (
            JavaString::from("states"),
            Tag::List(vec![test_state_row("x"), test_state_row("y")]),
        ),
        (JavaString::from("shapes"), Tag::List(Vec::new())),
        (
            JavaString::from("cached_shapes"),
            Tag::IntArray(vec![-1, -1]),
        ),
    ]));
    plan.nbt(&mut tag);
    plan.declared_shapes(&mut tag, true).unwrap();
    let Some(Tag::List(entries)) = tag.get("shapes") else {
        panic!("shapes")
    };
    let Some(Tag::IntArray(mapping)) = tag.get("cached_shapes") else {
        panic!("mapping")
    };
    for (index, shape) in collisions.iter().enumerate() {
        assert_eq!(entries[mapping[2 + index] as usize], encode_shape(shape));
    }
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

/// Declared outline shapes feed gameplay lighting and both occlusion tables,
/// independently of the optional collision shape.
#[test]
fn declared_shapes_drive_lighting_and_occlusion() {
    for (label, no_occlusion) in [("occluding", false), ("nonoccluding", true)] {
        let declaration = format!(
            r#"{{"template":"minecraft:test","item":false,
          "properties":{{"occludes":{}}},
          "shapes":{{"properties":["axis"],"values":{{"x":[[0,0,0,1,1,1]],"y":[[0,0,0,0.5,1,1]]}}}},
          "collision_shapes":{{"default":[]}}}}"#,
            !no_occlusion
        );
        let (base, mods) = block_base(label, &[&declaration]);
        let plan = Plan::new(&base, &mods).unwrap();
        // A non-occluding template must not suppress an explicit true, even for a partial outline.
        let row =
            r#"{"can_occlude":false,"solid_render":false,"dampening":1,"skylight_down":false}"#;
        let mut gameplay: J =
            serde_json::from_str(&format!(r#"{{"states":[{row},{row}]}}"#)).unwrap();
        plan.json("gameplay-facts.json", &mut gameplay).unwrap();
        let Some(J::Arr(rows)) = gameplay.get("states") else {
            panic!("states")
        };
        let full = &rows[2];
        let partial = &rows[3];
        assert_eq!(full.get("solid_render"), Some(&J::Bool(!no_occlusion)));
        assert_eq!(
            full.get("dampening"),
            Some(&J::Num((if no_occlusion { 1 } else { 15 }).into()))
        );
        assert_eq!(partial.get("solid_render"), Some(&J::Bool(false)));
        assert_eq!(full.get("shape_occlusion"), Some(&J::Bool(false)));
        assert_eq!(
            partial.get("shape_occlusion"),
            Some(&J::Bool(!no_occlusion))
        );
        assert_eq!(partial.get("skylight_down"), Some(&J::Bool(true)));
        assert_eq!(partial.get("dampening"), Some(&J::Num(0.into())));
        let mut shapes = Tag::Compound(Compound::from([
            (
                JavaString::from("states"),
                Tag::List(vec![test_state_row("x"), test_state_row("y")]),
            ),
            (JavaString::from("shapes"), Tag::List(Vec::new())),
            (
                JavaString::from("cached_shapes"),
                Tag::IntArray(vec![-1, -1]),
            ),
        ]));
        plan.nbt(&mut shapes);
        plan.declared_shapes(&mut shapes, false).unwrap();
        let Some(Tag::List(entries)) = shapes.get("shapes") else {
            panic!("shapes")
        };
        let Some(Tag::IntArray(indices)) = shapes.get("cached_shapes") else {
            panic!("indices")
        };
        let expected = plan.blocks[0].shapes.as_ref().unwrap();
        for (index, shape) in expected.iter().enumerate() {
            assert_eq!(entries[indices[2 + index] as usize], encode_shape(shape));
        }
        fs::remove_dir_all(base.parent().unwrap()).unwrap();
    }
}

#[test]
fn declared_offset_occlusion_uses_origin_without_shifting_the_local_outline() {
    let declaration = r#"{"template":"minecraft:test","item":false,"properties":{"offset":"XZ"},
      "shapes":{"default":[[0,0,0,1,1,1]]}}"#;
    let (base, mods) = block_base("offset-occlusion-origin", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let row = r#"{"can_occlude":true,"solid_render":true,"dampening":15}"#;
    let mut gameplay: J = serde_json::from_str(&format!(r#"{{"states":[{row},{row}]}}"#)).unwrap();
    plan.json("gameplay-facts.json", &mut gameplay).unwrap();
    let Some(J::Arr(rows)) = gameplay.get("states") else {
        panic!("states")
    };
    assert_eq!(rows[2].get("solid_render"), Some(&J::Bool(false)));
    assert_eq!(rows[2].get("shape_occlusion"), Some(&J::Bool(true)));
    assert_eq!(rows[2].get("dampening"), Some(&J::Num(0.into())));
    assert_eq!(
        rows[2].get("outline"),
        Some(&J::Arr(vec![J::Arr(
            [0., 0., 0., 1., 1., 1.].into_iter().map(J::f64).collect()
        )]))
    );
    let mut occlusion = Tag::compound([
        (
            "states",
            Tag::List(vec![test_state_row("x"), test_state_row("y")]),
        ),
        ("shapes", Tag::List(vec![])),
        ("cached_shapes", Tag::IntArray(vec![-1, -1])),
    ]);
    plan.nbt(&mut occlusion);
    plan.declared_shapes(&mut occlusion, false).unwrap();
    let Some(Tag::List(shapes)) = occlusion.get("shapes") else {
        panic!("shapes")
    };
    let Some(Tag::IntArray(mapping)) = occlusion.get("cached_shapes") else {
        panic!("mapping")
    };
    let shifted = &shapes[mapping[2] as usize];
    let offset_axis = Tag::LongArray(vec![(-0.25f64).to_bits() as i64, 0.75f64.to_bits() as i64]);
    assert_eq!(shifted.get("x"), Some(&offset_axis));
    assert_eq!(shifted.get("z"), Some(&offset_axis));
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

#[test]
fn enabling_occlusion_uses_the_templates_published_shape() {
    let declaration =
        r#"{"template":"minecraft:test","item":false,"properties":{"occludes":true}}"#;
    let (base, mods) = block_base("enable-template-occlusion", &[declaration]);
    let tag = Tag::compound([
        ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
        (
            "states",
            Tag::List(vec![test_state_row("x"), test_state_row("y")]),
        ),
        (
            "shapes",
            Tag::List(vec![
                encode_shape(&VoxelShape::block()),
                encode_shape(&VoxelShape::empty()),
            ]),
        ),
        ("cached_shapes", Tag::IntArray(vec![0, 1])),
    ]);
    fs::write(
        base.join("occlusion.nbt"),
        nbt::write_named(
            &nbt::NamedTag {
                name: "".into(),
                tag,
            },
            Limits::tables(),
        )
        .unwrap(),
    )
    .unwrap();
    let plan = Plan::new(&base, &mods).unwrap();
    let row = r#"{"can_occlude":false,"solid_render":false,"dampening":0,"skylight_down":true}"#;
    let mut gameplay: J = serde_json::from_str(&format!(r#"{{"states":[{row},{row}]}}"#)).unwrap();
    plan.json("gameplay-facts.json", &mut gameplay).unwrap();
    let Some(J::Arr(rows)) = gameplay.get("states") else {
        panic!("states")
    };
    assert_eq!(rows[2].get("can_occlude"), Some(&J::Bool(true)));
    assert_eq!(rows[2].get("solid_render"), Some(&J::Bool(true)));
    assert_eq!(rows[2].get("dampening"), Some(&J::Num(15.into())));
    assert_eq!(rows[3].get("solid_render"), Some(&J::Bool(false)));
    assert_eq!(rows[3].get("dampening"), Some(&J::Num(0.into())));
    assert_eq!(rows[2].get("skylight_down"), Some(&J::Bool(true)));
    let mut definitions: J =
        serde_json::from_str(r#"{"minecraft:test":{"can_occlude":false}}"#).unwrap();
    plan.json("render-definitions.json", &mut definitions)
        .unwrap();
    assert_eq!(
        definitions.at("testmod:test/can_occlude"),
        Some(&mut J::Bool(true))
    );
    let mut flags = Tag::compound([("blocks", Tag::compound([("minecraft:test", Tag::Byte(0))]))]);
    plan.nbt(&mut flags);
    assert_eq!(
        flags.get("blocks").unwrap().get("testmod:test"),
        Some(&Tag::Byte(16))
    );
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

#[test]
fn declared_partial_shapes_observe_waterlogging_and_template_light_overrides() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "shapes":{"default":[[0,0,0,1,0.5,1]]}}"#;
    let facts = r#"{"states":[
      ["minecraft:test",false,false,"",{"id":"minecraft:test","properties":{"waterlogged":"false"}}],
      ["minecraft:test",false,true,"",{"id":"minecraft:test","properties":{"waterlogged":"true"}}]],
      "defaults":{"minecraft:test":0}}"#;
    let (base, mods) = block_base_of("shape-light-water", &[declaration], facts);
    let plan = Plan::new(&base, &mods).unwrap();
    let row = r#"{"can_occlude":true,"solid_render":true,"dampening":15,"skylight_down":false}"#;
    let mut gameplay: J = serde_json::from_str(&format!(r#"{{"states":[{row},{row}]}}"#)).unwrap();
    plan.json("gameplay-facts.json", &mut gameplay).unwrap();
    let Some(J::Arr(rows)) = gameplay.get("states") else {
        panic!("states")
    };
    assert_eq!(rows[2].get("skylight_down"), Some(&J::Bool(true)));
    assert_eq!(rows[2].get("dampening"), Some(&J::Num(0.into())));
    assert_eq!(rows[3].get("skylight_down"), Some(&J::Bool(false)));
    assert_eq!(rows[3].get("dampening"), Some(&J::Num(1.into())));
    drop(plan);
    fs::write(base.join("block-behaviours.json"), r#"{"blocks":[{"name":"minecraft:test","class":["TintedGlassBlock","TransparentBlock","Block"]}]}"#).unwrap();
    let plan = Plan::new(&base, &mods).unwrap();
    let mut gameplay: J = serde_json::from_str(&format!(r#"{{"states":[{row},{row}]}}"#)).unwrap();
    plan.json("gameplay-facts.json", &mut gameplay).unwrap();
    let Some(J::Arr(rows)) = gameplay.get("states") else {
        panic!("states")
    };
    assert_eq!(rows[2].get("skylight_down"), Some(&J::Bool(false)));
    assert_eq!(rows[2].get("dampening"), Some(&J::Num(15.into())));
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

/// A declared waterlogged property changes a fluid-dependent class override,
/// but not TransparentBlock's constant skylight rule.
#[test]
#[ignore = "daily"]
fn declared_waterlogged_vegetation_uses_resolved_fluid_for_skylight() {
    use crate::{
        blocks::Fluid,
        chunk::Chunk,
        light::{LightTables, apply_lighting, light_chunk},
    };
    let Some(base) = prepared() else { return };
    let scratch =
        std::env::temp_dir().join(format!("sunrise-vegetation-light-{}", std::process::id()));
    let m = folder_mod(
        &scratch.join("mod"),
        &[
            ("sunrise-mod.toml", &manifest("testmod", "")),
            (
                "data/testmod/sunrise/block/poppy.json",
                r#"{
                  "template": "minecraft:poppy",
                  "item": false,
                  "state_properties": {"waterlogged": {"type": "boolean"}},
                  "shapes": {"default": [[0.3125,0,0.3125,0.6875,0.625,0.6875]]}
                }"#,
            ),
            (
                "data/testmod/sunrise/block/glass.json",
                r#"{"template":"minecraft:glass","item":false,
                  "state_properties":{"waterlogged":{"type":"boolean"}},
                  "shapes":{"default":[[0,0,0,1,1,1]]}}"#,
            ),
        ],
    );
    let root = resource_root_fixture(&base, Path::new(""), &[m], &scratch.join("cache")).unwrap();
    let assets = Assets::load(&root).unwrap();
    let registry = assets.registry;
    let facts = BlockFacts::load(&root, registry).unwrap();
    let tables = LightTables::load(&root, registry, &facts).unwrap();
    // VegetationBlock checks fluid emptiness independently of the outline;
    // TransparentBlock always passes skylight. Values come from those rules,
    // not from the generated facts or from another run of the derivation.
    for (name, wet, down, dampening, sky) in [
        ("testmod:poppy", false, true, 0, 15),
        ("testmod:poppy", true, false, 1, 14),
        ("testmod:glass", false, true, 0, 15),
        ("testmod:glass", true, true, 0, 15),
    ] {
        let default = registry.default_state(name).unwrap();
        let mut state = registry.state(default).unwrap().clone();
        state
            .properties
            .insert("waterlogged".into(), wet.to_string());
        let id = registry.id(&state).unwrap();
        let declared = facts.get(id);
        assert_eq!(declared.waterlogged, wet, "{state:?}");
        assert_eq!(
            declared.fluid,
            if wet { Fluid::Water } else { Fluid::None },
            "{state:?}"
        );
        assert!(!declared.can_occlude, "{state:?}");
        assert_eq!(
            (declared.skylight_down, declared.dampening),
            (down, dampening),
            "{state:?}"
        );
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(8, 3, 8, id.0);
        let mut region = [[None; 3]; 3];
        region[1][1] = Some(&chunk);
        let lights = light_chunk(&region, &tables, true);
        apply_lighting(&mut chunk, lights);
        assert_eq!(chunk.light(8, 4, 8) >> 4, 15, "exposed above {state:?}");
        assert_eq!(chunk.light(8, 3, 8) >> 4, sky, "exposed cell {state:?}");
    }
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn collision_only_preserves_outline_and_shape_typos_are_errors() {
    let declaration = r#"{"template":"minecraft:test","collision_shapes":{"default":[]}}"#;
    let (base, mods) = block_base("collision-only", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    assert!(plan.blocks[0].shapes.is_none());
    assert!(
        plan.blocks[0]
            .collision_shapes
            .as_ref()
            .unwrap()
            .iter()
            .all(VoxelShape::is_empty)
    );
    let mut gameplay: J =
        serde_json::from_str(r#"{"states":[{"outline":[[0,0,0,1,1,1]]},{"outline":[]}]}"#).unwrap();
    plan.json("gameplay-facts.json", &mut gameplay).unwrap();
    let Some(J::Arr(rows)) = gameplay.get("states") else {
        panic!("states")
    };
    assert_eq!(rows[0].get("outline"), rows[2].get("outline"));
    assert_eq!(rows[1].get("outline"), rows[3].get("outline"));
    assert_eq!(rows[2].get("full_collision"), Some(&J::Bool(false)));
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
    for (index, table) in [
        r#"{"properties":["axis","typo"],"default":[]}"#,
        r#"{"properties":["axis"],"values":{"bad":[]},"default":[]}"#,
        r#"{"properties":["axis"],"values":{"x":[]}}"#,
    ]
    .iter()
    .enumerate()
    {
        let declaration = format!(r#"{{"template":"minecraft:test","collision_shapes":{table}}}"#);
        let (base, mods) = block_base(&format!("collision-typo-{index}"), &[&declaration]);
        assert!(Plan::new(&base, &mods).is_err(), "{table}");
        fs::remove_dir_all(base.parent().unwrap()).unwrap();
    }
}

/// A block's states are its template's times the combinations of the state
/// properties it declares, in a fixed order with every default first.
#[test]
fn content_block_state_properties_fan_out_its_states() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"lit":{"type":"boolean"},
        "facing":{"type":"enum","values":["north","south","east"]}}}"#;
    let (base, mods) = block_base("block-states", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let facts = fs::read_to_string(base.join("facts.json")).unwrap();
    let mut doc: J = serde_json::from_str(&facts).unwrap();
    plan.json("facts.json", &mut doc).unwrap();
    // Two template states times the six combinations, after the template's.
    let Some(J::Arr(rows)) = doc.get("states") else {
        panic!("no states")
    };
    assert_eq!(rows.len(), 2 + 12);
    // The default is the first combination of the template's default state.
    assert_eq!(
        doc.get("defaults").unwrap().get("testmod:test"),
        Some(&J::Num(2.into()))
    );
    let properties = |i: usize| -> BTreeMap<String, String> {
        let J::Arr(row) = &rows[i] else {
            panic!("row {i} is not a state")
        };
        let Some(J::Obj(state)) = row.get(4) else {
            panic!("row {i} has no state")
        };
        let Some((_, J::Obj(p))) = state.iter().find(|(k, _)| k == "properties") else {
            panic!("row {i} has no properties")
        };
        p.iter()
            .map(|(k, v)| match v {
                J::Str(s) => (k.clone(), s.clone()),
                _ => panic!("{k} is not a string"),
            })
            .collect()
    };
    // By name `facing` varies slowest, and each one's default comes first.
    assert_eq!(
        properties(2),
        BTreeMap::from([
            ("axis".to_owned(), "x".to_owned()),
            ("facing".to_owned(), "north".to_owned()),
            ("lit".to_owned(), "false".to_owned()),
        ])
    );
    // And the last is the second template state and the last combination.
    assert_eq!(
        properties(13),
        BTreeMap::from([
            ("axis".to_owned(), "y".to_owned()),
            ("facing".to_owned(), "east".to_owned()),
            ("lit".to_owned(), "true".to_owned()),
        ])
    );
    // Every state of the block is a different one: the registry indexes
    // states by their name and properties, so a repeat would be a collision.
    let states: BTreeSet<BTreeMap<String, String>> = (2..14).map(properties).collect();
    assert_eq!(states.len(), 12);
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

/// A declared property whose name sorts before the template's goes in vanilla's order
/// (`StateDefinition`): all the names sorted, the first slowest, so `alpha` varies slower than the
/// template's `axis`. A state table that read the rows in the other order stopped the game at
/// load ("registry state order differs from vanilla's state definition").
#[test]
fn content_block_state_properties_follow_vanilla_name_order() {
    let (base, mods) = block_base(
        "block-name-order",
        &[r#"{"template":"minecraft:test","item":false,
           "state_properties":{"alpha":{"type":"boolean"}}}"#],
    );
    let plan = Plan::new(&base, &mods).unwrap();
    let facts = fs::read_to_string(base.join("facts.json")).unwrap();
    let mut doc: J = serde_json::from_str(&facts).unwrap();
    plan.json("facts.json", &mut doc).unwrap();
    let Some(J::Arr(rows)) = doc.get("states") else {
        panic!("no states")
    };
    assert_eq!(rows.len(), 2 + 4);
    let properties = |i: usize| -> (String, String) {
        let J::Arr(row) = &rows[i] else {
            panic!("row {i} is not a state")
        };
        let Some(J::Obj(state)) = row.get(4) else {
            panic!("row {i} has no state")
        };
        let Some((_, J::Obj(p))) = state.iter().find(|(k, _)| k == "properties") else {
            panic!("row {i} has no properties")
        };
        let value = |name: &str| match p.iter().find(|(k, _)| k == name) {
            Some((_, J::Str(s))) => s.clone(),
            _ => panic!("row {i} has no {name}"),
        };
        (value("alpha"), value("axis"))
    };
    let own = |a: &str, b: &str| (a.to_owned(), b.to_owned());
    // `alpha` slowest, then `axis`: the first name varies slowest, as in vanilla.
    assert_eq!(
        (2..6).map(properties).collect::<Vec<_>>(),
        [
            own("false", "x"),
            own("false", "y"),
            own("true", "x"),
            own("true", "y")
        ]
    );
    // The default is the template's default state with every declared property at its default.
    assert_eq!(
        doc.get("defaults").unwrap().get("testmod:test"),
        Some(&J::Num(2.into()))
    );
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

/// `state_order` over the template's and the declared properties together.
#[test]
fn block_states_are_laid_out_in_vanilla_order() {
    let p = |name: &str, len: u32| (name.to_owned(), len);
    // Every declared name after the template's: the template's states times the combinations,
    // as before, so ids of existing blocks do not move.
    let order = state_order(&[p("axis", 2)], &[p("lit", 2)]);
    assert_eq!(order, [(0, 0), (0, 1), (1, 0), (1, 1)]);
    // No declared property, or no template property: the identity, whichever side it is on.
    assert_eq!(state_order(&[p("axis", 3)], &[]), [(0, 0), (1, 0), (2, 0)]);
    assert_eq!(state_order(&[], &[p("lit", 2)]), [(0, 0), (0, 1)]);
    assert_eq!(state_order(&[], &[]), [(0, 0)]);
    // `bites` (4 values) before `facing` (4 values): the declared one slowest.
    let order = state_order(&[p("facing", 4)], &[p("bites", 4)]);
    assert_eq!(order.len(), 16);
    assert!(
        order
            .iter()
            .enumerate()
            .all(|(j, &(s, c))| (s, c) == (j as u32 % 4, j as u32 / 4))
    );
    // Interleaved: `b` between the template's `a` and `c`. a (2), b (2), c (3), a slowest.
    let order = state_order(&[p("a", 2), p("c", 3)], &[p("b", 2)]);
    assert_eq!(order.len(), 12);
    // Template state = a * 3 + c, combo = b.
    assert_eq!(order[0], (0, 0));
    assert_eq!(order[1], (1, 0));
    assert_eq!(order[3], (0, 1));
    assert_eq!(order[6], (3, 0));
    assert_eq!(order[11], (5, 1));
    // Every pair appears once.
    let mut seen = order.clone();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 12);
}

/// The declared default comes first in the run, so its id is the block's
/// default with no arithmetic of its own.
#[test]
fn content_block_state_properties_put_the_declared_default_first() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"facing":{"type":"enum","values":["north","south","east"]}},
      "default_state":{"facing":"south"}}"#;
    let (base, mods) = block_base("block-default-first", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let facts = fs::read_to_string(base.join("facts.json")).unwrap();
    let mut doc: J = serde_json::from_str(&facts).unwrap();
    plan.json("facts.json", &mut doc).unwrap();
    let Some(J::Arr(rows)) = doc.get("states") else {
        panic!("no states")
    };
    // Two template states times three values.
    assert_eq!(rows.len(), 2 + 6);
    let facing = |i: usize| -> String {
        let J::Arr(row) = &rows[i] else {
            panic!("row {i} is not a state")
        };
        let Some(J::Obj(state)) = row.get(4) else {
            panic!("row {i} has no state")
        };
        match state
            .iter()
            .find(|(k, _)| k == "properties")
            .map(|(_, p)| p)
        {
            Some(p) => match p.get("facing") {
                Some(J::Str(s)) => s.clone(),
                _ => panic!("row {i} has no facing"),
            },
            None => panic!("row {i} has no properties"),
        }
    };
    // The declared default is the block's default, and it comes first.
    assert_eq!(
        doc.get("defaults").unwrap().get("testmod:test"),
        Some(&J::Num(2.into()))
    );
    assert_eq!(facing(2), "south");
    // The rest keep their declared order, rotated to start at the default.
    assert_eq!(
        (facing(3), facing(4)),
        ("east".to_owned(), "north".to_owned())
    );
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

/// What a declaration may not do: shadow its template's property, name a
/// value its own property does not have, or ask for more states than a world
/// holds.
#[test]
fn content_block_state_properties_are_validated() {
    // The template's `axis` is not the mod's to redefine.
    let (base, mods) = block_base(
        "block-shadow",
        &[r#"{"template":"minecraft:test","state_properties":{"axis":{"type":"boolean"}}}"#],
    );
    let error = match Plan::new(&base, &mods) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("a shadowed template property was accepted"),
    };
    assert!(error.contains("axis is already on"), "{error}");
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();

    // A code declaration is named by its ID even in a package with block JSON.
    let (base, mut mods) = block_base("block-code-shadow", &[r#"{"template":"minecraft:test"}"#]);
    let block = sunrise_mod_package::content::Block {
        id: "testmod:code".into(),
        template: "minecraft:test".into(),
        state_properties: [(
            "axis".into(),
            sunrise_mod_package::content::StateProperty::Boolean,
        )]
        .into(),
        ..Default::default()
    };
    let batches = BTreeMap::from([(
        "testmod",
        DeclaredContent {
            blocks: vec![block.validate_local().unwrap()],
            ..Default::default()
        },
    )]);
    let mut errors = Vec::new();
    accept_declared(&mut mods, &batches, &[], &BTreeSet::new(), &mut errors).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let error = match Plan::new(&base, &mods) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("a shadowed template property from code was accepted"),
    };
    assert!(error.starts_with("testmod:code:"), "{error}");
    assert!(
        error.contains("state_properties.axis") && error.contains("minecraft:test"),
        "{error}"
    );
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();

    // A default is one of the values the block declared.
    let (base, mods) = block_base(
        "block-bad-default",
        &[r#"{"template":"minecraft:test",
             "state_properties":{"lit":{"type":"boolean"}},"default_state":{"lit":"maybe"}}"#],
    );
    let error = match Plan::new(&base, &mods) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("a default outside a property's values was accepted"),
    };
    assert!(error.contains("is not one of its values"), "{error}");
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();

    // Twenty-one booleans are more combinations than a world can hold, and
    // are refused before the product is built.
    let declared: Vec<String> = (0..21)
        .map(|i| format!(r#""p{i}":{{"type":"boolean"}}"#))
        .collect();
    let declaration = format!(
        r#"{{"template":"minecraft:test","item":false,"state_properties":{{{}}}}}"#,
        declared.join(",")
    );
    let (base, mods) = block_base("block-too-many", &[&declaration]);
    let error = match Plan::new(&base, &mods) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("more states than a world holds were accepted"),
    };
    assert!(
        error.contains("more than the") && error.contains("world can hold"),
        "{error}"
    );
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

/// A `default_state` that names only properties the block itself declares
/// asks the template for nothing: `stone` has a single state and no properties
/// at all, and a block that gives it `lit` can still default `lit`.
#[test]
fn a_declared_property_defaults_on_a_template_with_none() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"lit":{"type":"boolean"}},
      "default_state":{"lit":"true"}}"#;
    let (base, mods) = block_base_of(
        "block-propertyless-default",
        &[declaration],
        r#"{"states":[["minecraft:test",true,true,"",{"id":"minecraft:test"}]],
      "defaults":{"minecraft:test":0}}"#,
    );
    let plan = Plan::new(&base, &mods).unwrap();
    let mut doc: J =
        serde_json::from_str(&fs::read_to_string(base.join("facts.json")).unwrap()).unwrap();
    plan.json("facts.json", &mut doc).unwrap();
    let Some(J::Arr(rows)) = doc.get("states") else {
        panic!("no states")
    };
    // One template state times the two combinations of `lit`.
    assert_eq!(rows.len(), 1 + 2);
    let lit = |i: usize| -> Option<String> {
        let J::Arr(row) = rows.get(i)? else {
            return None;
        };
        let J::Obj(state) = row.get(4)? else {
            return None;
        };
        let (_, properties) = state.iter().find(|(k, _)| k == "properties")?;
        match properties.get("lit") {
            Some(J::Str(s)) => Some(s.clone()),
            _ => None,
        }
    };
    // The declared default opens the block's run, and the other value follows.
    assert_eq!(lit(1).as_deref(), Some("true"));
    assert_eq!(lit(2).as_deref(), Some("false"));
    // The block's default state is the first of its run.
    assert_eq!(
        doc.get("defaults").unwrap().get("testmod:test"),
        Some(&J::Num(1.into()))
    );
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

#[test]
fn named_definitions_follow_the_block_state_fanout() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"lit":{"type":"boolean"}},
      "default_state":{"lit":"true"}}"#;
    let (base, mods) = block_base("named-state-fanout", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let mut models: J = serde_json::from_str(
        r#"{"minecraft:test":{
        "schema":{"axis":{"named":["x","y"]}},
        "states":[{"axis":"x"},{"axis":"y"}]}}"#,
    )
    .unwrap();
    plan.json("model-states.json", &mut models).unwrap();
    let added = models.get("testmod:test").unwrap();
    let expected: J = serde_json::from_str(
        r#"{"axis":{"named":["x","y"]},
        "lit":{"named":["false","true"]}}"#,
    )
    .unwrap();
    assert_eq!(added.get("schema"), Some(&expected));
    let expected: J = serde_json::from_str(
        r#"[
        {"axis":"x","lit":"true"},{"axis":"x","lit":"false"},
        {"axis":"y","lit":"true"},{"axis":"y","lit":"false"}]"#,
    )
    .unwrap();
    assert_eq!(added.get("states"), Some(&expected));

    let mut render: J = serde_json::from_str(
        r#"{"minecraft:test":{
        "default":0,"can_occlude":true,"bars_tag":false,"rule":"never",
        "states":[0,1]}}"#,
    )
    .unwrap();
    plan.json("render-definitions.json", &mut render).unwrap();
    let added = render.get("testmod:test").unwrap();
    let expected: J = serde_json::from_str("[2,3,4,5]").unwrap();
    assert_eq!(added.get("states"), Some(&expected));
    assert_eq!(added.get("default"), Some(&J::Num(2.into())));

    for (file, json) in [
        (
            "landing-definitions.json",
            r#"{"minecraft:test":{"fall":"Block","bounce":false}}"#,
        ),
        (
            "worldgen/blocks/survival-rules.json",
            r#"{"minecraft:test":"Always"}"#,
        ),
    ] {
        let mut doc: J = serde_json::from_str(json).unwrap();
        plan.json(file, &mut doc).unwrap();
        assert_eq!(doc.get("testmod:test"), doc.get("minecraft:test"), "{file}");
    }
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

/// An NBT state row for the two-state `minecraft:test` template, the shape
/// `BlockState::to_nbt` writes.
fn test_state_row(axis: &str) -> Tag {
    Tag::Compound(Compound::from([
        (
            JavaString::from("id"),
            Tag::String(JavaString::from("minecraft:test")),
        ),
        (
            JavaString::from("properties"),
            Tag::Compound(Compound::from([(
                JavaString::from("axis"),
                Tag::String(JavaString::from(axis)),
            )])),
        ),
    ]))
}

/// The NBT tables fan out like the JSON ones: `cached-collision.nbt`,
/// `occlusion.nbt` and the rest get one row per combination, so `states` and
/// `cached_shapes` stay the same length, and each row carries the properties
/// its combination declares.
#[test]
fn the_nbt_tables_fan_out_with_the_states() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"lit":{"type":"boolean"}}}"#;
    let (base, mods) = block_base("block-nbt-fanout", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let mut tag = Tag::Compound(Compound::from([
        (
            JavaString::from("states"),
            Tag::List(vec![test_state_row("x"), test_state_row("y")]),
        ),
        (JavaString::from("cached_shapes"), Tag::IntArray(vec![7, 9])),
    ]));
    plan.nbt(&mut tag);
    let Some(Tag::List(states)) = tag.get("states") else {
        panic!("no states")
    };
    let Some(Tag::IntArray(shapes)) = tag.get("cached_shapes") else {
        panic!("no cached_shapes")
    };
    // Two template states times the two combinations of `lit`, and the shapes
    // stay parallel to the states — that is what makes the file load.
    assert_eq!(states.len(), 2 + 4);
    assert_eq!(shapes.len(), states.len());
    // A combination keeps the shape of the template state it came from.
    assert_eq!(shapes, &vec![7, 9, 7, 7, 9, 9]);
    let properties = |i: usize| -> BTreeMap<String, String> {
        let Some(Tag::Compound(c)) = states.get(i) else {
            panic!("row {i} is not a compound")
        };
        let Some(Tag::Compound(p)) = c.get(&JavaString::from("properties")) else {
            panic!("row {i} has no properties")
        };
        p.iter()
            .filter_map(|(k, v)| match v {
                Tag::String(s) => Some((k.to_string_lossy(), s.to_string_lossy())),
                _ => None,
            })
            .collect()
    };
    let mut expected: Vec<String> = Vec::new();
    for axis in ["x", "y"] {
        for lit in ["false", "true"] {
            expected.push(format!("{axis}/{lit}"));
        }
    }
    let seen: Vec<String> = (2..states.len())
        .map(|i| {
            let p = properties(i);
            // The declared properties are the only ones on the row.
            assert_eq!(p.len(), 2, "row {i} is {p:?}");
            let axis = p.get("axis").cloned().unwrap_or_default();
            let lit = p.get("lit").cloned().unwrap_or_default();
            format!("{axis}/{lit}")
        })
        .collect();
    assert_eq!(seen, expected);
    for i in 2..states.len() {
        let Some(Tag::String(id)) = states[i].get("id") else {
            panic!("row {i} has no id")
        };
        assert_eq!(id.to_string_lossy(), "testmod:test");
    }
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

/// A block's shape and its count in `structure-blocks.json` cover every
/// combination too: the table's declared total and the lengths of its lists
/// grow by the fan-out, and every state of the block maps to a shape.
#[test]
fn a_shape_and_the_state_count_cover_every_combination() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"lit":{"type":"boolean"}},
      "shapes":{"properties":["axis"],"values":{"x":[[0,0,0,1,1,1]],"y":[[0,0,0,0.5,1,1]]}}}"#;
    let (base, mods) = block_base("block-shape-fanout", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    // The template's two states, no shapes yet, every vanilla state unmapped.
    // `nbt` grows the arrays to the block's four states and `collision_shapes`
    // writes into them, in the order the real path applies them.
    let mut tag = Tag::Compound(Compound::from([
        (
            JavaString::from("states"),
            Tag::List(vec![test_state_row("x"), test_state_row("y")]),
        ),
        (JavaString::from("shapes"), Tag::List(Vec::new())),
        (
            JavaString::from("cached_shapes"),
            Tag::IntArray(vec![-1, -1]),
        ),
    ]));
    plan.nbt(&mut tag);
    plan.declared_shapes(&mut tag, true).unwrap();
    let Some(Tag::List(entries)) = tag.get("shapes") else {
        panic!("no shapes")
    };
    let Some(Tag::IntArray(mapping)) = tag.get("cached_shapes") else {
        panic!("no cached_shapes")
    };
    // One shape per template state, shared by that state's combinations.
    assert_eq!(entries.len(), 2);
    assert_eq!(mapping.len(), 2 + 4);
    let added = &mapping[2..];
    assert!(added.iter().all(|id| *id >= 0), "{added:?}");
    assert_eq!((added[0], added[1]), (added[0], added[0]));
    assert_eq!((added[2], added[3]), (added[2], added[2]));
    assert_ne!(added[0], added[2]);

    let rows = "[[false,false],[false,false]]";
    let text = format!(r#"{{"rotate":{rows},"mirror":{rows},"full_block":{rows},"states":2}}"#);
    let mut doc: J = serde_json::from_str(&text).unwrap();
    plan.json("worldgen/blocks/structure-blocks.json", &mut doc)
        .unwrap();
    for key in ["rotate", "mirror", "full_block"] {
        let Some(J::Arr(list)) = doc.get(key) else {
            panic!("{key} is not a list")
        };
        assert_eq!(list.len(), 2 + 4, "{key}");
    }
    // The declared total agrees with the lists, which is what a loader checks.
    assert_eq!(doc.get("states"), Some(&J::Num(6.into())));
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

fn biome_features(root: &Path) -> BTreeMap<String, serde_json::Value> {
    let bundle: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/biomes.json")).unwrap()).unwrap();
    bundle
        .into_iter()
        .map(|(id, text)| {
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            (id, v["features"].clone())
        })
        .collect()
}

#[test]
fn biome_modifier_climate_rejects_malformed_ranges() {
    for climate in [
        r#"{"type":"sunrise:climate","temperature":{"min":"warm"}}"#,
        r#"{"type":"sunrise:climate","temperature":{"min":null,"max":1}}"#,
        r#"{"type":"sunrise:climate","temperature":null}"#,
        r#"{"type":"sunrise:climate","temperature":{"min":2,"max":1}}"#,
        r#"{"type":"sunrise:climate","temperature":{"minimum":1}}"#,
        r#"{"type":"sunrise:climate","temperature":{}}"#,
        r#"{"type":"sunrise:climate"}"#,
    ] {
        let base = biome_base("biome-invalid-climate");
        let scratch = base.parent().unwrap().to_owned();
        let modifier = format!(
            r#"{{"type":"neoforge:add_features","biomes":{climate},"features":"zinc:ore_zinc","step":"underground_ores"}}"#
        );
        let m = zinc_mod(&scratch.join("zinc"), &[("bad", &modifier)]);
        assert!(
            build_fixture(&base, Path::new(""), &[m], &scratch.join("out")).is_err(),
            "{climate}"
        );
        fs::remove_dir_all(&scratch).unwrap();
    }
}

#[test]
fn biome_modifier_climate_standalone_includes_bounds() {
    let base = biome_base("biome-climate-alone");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[(
            "warm",
            r#"{
        "type":"neoforge:add_features",
        "biomes":{"type":"sunrise:climate","temperature":{"min":2,"max":2},"downfall":{"max":0}},
        "features":"zinc:ore_zinc","step":"underground_ores"}"#,
        )],
    );
    let out = scratch.join("out");
    build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    let features = biome_features(&out);
    assert_eq!(
        features["minecraft:desert"][6],
        serde_json::json!(["zinc:ore_zinc"])
    );
    assert_eq!(
        features["minecraft:nether_wastes"][6],
        serde_json::json!(["minecraft:ore_coal", "zinc:ore_zinc"])
    );
    assert_eq!(
        features["minecraft:plains"][6],
        serde_json::json!(["minecraft:ore_coal"])
    );
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn biome_modifier_climate_combines_with_holder_sets() {
    let base = biome_base("biome-climate");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[(
            "warm",
            r##"{
        "type":"neoforge:add_features",
        "biomes":{"type":"neoforge:and","values":["#minecraft:is_overworld",
            {"type":"sunrise:climate","temperature":{"min":0.8,"max":0.8},
             "downfall":{"min":0.4}}]},
        "features":"zinc:ore_zinc","step":"underground_ores"}"##,
        )],
    );
    let out = scratch.join("out");
    build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    let features = biome_features(&out);
    assert_eq!(
        features["minecraft:plains"][6],
        serde_json::json!(["minecraft:ore_coal", "zinc:ore_zinc"])
    );
    assert_eq!(
        features["minecraft:desert"][0],
        serde_json::json!(["minecraft:fossil"])
    );
    assert_eq!(
        features["minecraft:nether_wastes"][6],
        serde_json::json!(["minecraft:ore_coal"])
    );
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn biome_modifier_climate_nested_or_not() {
    let base = biome_base("biome-climate-or-not");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[(
            "select",
            r##"{
        "type":"neoforge:add_features", "features":"zinc:ore_zinc", "step":"underground_ores",
        "biomes":{"type":"neoforge:or","values":["minecraft:plains",
            {"type":"neoforge:not","value":{"type":"sunrise:climate","temperature":{"min":1}}}]}}"##,
        )],
    );
    let out = scratch.join("out");
    build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    let features = biome_features(&out);
    assert_eq!(
        features["minecraft:plains"][6],
        serde_json::json!(["minecraft:ore_coal", "zinc:ore_zinc"])
    );
    assert_eq!(
        features["minecraft:nether_wastes"][6],
        serde_json::json!(["minecraft:ore_coal"])
    );
    assert_eq!(
        features["minecraft:desert"][0],
        serde_json::json!(["minecraft:fossil"])
    );
    fs::remove_dir_all(&scratch).unwrap();
}

/// `neoforge:add_features` puts a mod's ore at the end of its step in every
/// biome of an overworld tag, `remove_features` takes a vanilla feature out,
/// and the biomes no modifier names keep their text.
#[test]
fn biome_modifiers_place_mod_features_in_vanilla_biomes() {
    let base = biome_base("biome-modifiers");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[
            (
                "add_zinc",
                r##"{"type":"neoforge:add_features","biomes":"#minecraft:is_overworld",
                    "features":"zinc:ore_zinc","step":"underground_ores"}"##,
            ),
            // Adds before removes, whatever the file order.
            (
                "a_no_fossils",
                r#"{"type":"neoforge:remove_features","biomes":["minecraft:desert"],
                    "features":["minecraft:fossil"],"steps":["raw_generation"]}"#,
            ),
            ("nothing", r#"{"type":"neoforge:none"}"#),
        ],
    );
    let out = scratch.join("out");
    build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    let features = biome_features(&out);
    assert_eq!(
        features["minecraft:plains"],
        serde_json::json!([
            [],
            ["minecraft:lake"],
            [],
            [],
            [],
            [],
            ["minecraft:ore_coal", "zinc:ore_zinc"]
        ])
    );
    // A biome with fewer steps gets the ones up to the modifier's.
    assert_eq!(
        features["minecraft:desert"],
        serde_json::json!([[], [], [], [], [], [], ["zinc:ore_zinc"]])
    );
    let text = |root: &Path| -> BTreeMap<String, String> {
        serde_json::from_slice(&fs::read(root.join("worldgen/biomes.json")).unwrap()).unwrap()
    };
    assert_eq!(
        text(&out)["minecraft:nether_wastes"],
        text(&base)["minecraft:nether_wastes"]
    );
    let plains: serde_json::Value = serde_json::from_str(&text(&out)["minecraft:plains"]).unwrap();
    assert_eq!(plains["temperature"], serde_json::json!(0.8));

    // The generator's sorter places vanilla's features where they were and
    // the ore after the coal it follows: the order applyBiomeDecoration uses.
    use sunrise_world::worldgen::feature_sort::{FeatureIdentity, build_features_per_step};
    let mut ids: Vec<String> = Vec::new();
    let mut plan = |biomes: &[&str], features: &BTreeMap<String, serde_json::Value>| {
        let lists: Vec<Vec<Vec<FeatureIdentity>>> = biomes
            .iter()
            .map(|b| {
                features[*b]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|step| {
                        step.as_array()
                            .unwrap()
                            .iter()
                            .map(|f| {
                                let f = f.as_str().unwrap().to_owned();
                                let i = ids.iter().position(|x| *x == f).unwrap_or_else(|| {
                                    ids.push(f);
                                    ids.len() - 1
                                });
                                FeatureIdentity(i as u32)
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect();
        build_features_per_step(&lists).unwrap()
    };
    let overworld = ["minecraft:plains", "minecraft:desert"];
    let steps = plan(&overworld, &features);
    let names = |s: usize, ids: &[String]| -> Vec<String> {
        steps[s]
            .features()
            .iter()
            .map(|f| ids[f.0 as usize].clone())
            .collect()
    };
    assert_eq!(names(6, &ids), ["minecraft:ore_coal", "zinc:ore_zinc"]);
    assert_eq!(names(1, &ids), ["minecraft:lake"]);
    let _ = fs::remove_dir_all(&scratch);
}

/// 26.3's feature directory feeds the configured-feature bundle and command
/// registry, including nested paths, while other directories are not loaded.
#[test]
fn configured_features_load_from_the_26_3_path() {
    let base = biome_base("feature-paths");
    let scratch = base.parent().unwrap();
    fs::write(
        base.join("command-registries.json"),
        r#"{"blocks":{},"items":[],"by_registry":{"minecraft:worldgen/feature":{"ids":[]}}}"#,
    )
    .unwrap();
    let feature = r#"{"type":"minecraft:simple_block","config":{}}"#;
    let m = folder_mod(
        &scratch.join("zinc"),
        &[
            ("sunrise-mod.toml", &manifest("zinc", "")),
            ("data/zinc/worldgen/feature/nested/new.json", feature),
            ("data/zinc/worldgen/configured_feature/old.json", feature),
        ],
    );
    let root = scratch.join("out");
    build_fixture(&base, Path::new(""), &[m], &root).unwrap();
    let bundle: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/features.json")).unwrap()).unwrap();
    assert_eq!(
        bundle,
        BTreeMap::from([("zinc:nested/new".into(), feature.into())])
    );
    let commands: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("command-registries.json")).unwrap()).unwrap();
    let ids = commands["by_registry"]["minecraft:worldgen/feature"]["ids"]
        .as_array()
        .unwrap();
    assert_eq!(ids, &[serde_json::json!("zinc:nested/new")]);
    fs::remove_dir_all(scratch).unwrap();
}

/// A content mod's own biomes join the biome set after vanilla's, by namespace
/// then path; a `minecraft` biome file never joins it (the content fence refuses the file). Its tag keeps them, a
/// modifier on that tag reaches them, and the biome suggestions list them.
#[test]
fn a_mods_own_biomes_join_the_biome_set() {
    let base = biome_base("mod-biomes");
    let scratch = base.parent().unwrap().to_owned();
    fs::write(
        base.join("command-registries.json"),
        r#"{"blocks":{},"items":[],"by_registry":{"minecraft:worldgen/biome":{"ids":["minecraft:desert","minecraft:plains"]}}}"#,
    )
    .unwrap();
    let foo = r#"{"temperature":1.5,"downfall":0.0,"features":[[]]}"#;
    let m = folder_mod(
        &scratch.join("zinc"),
        &[
            ("sunrise-mod.toml", &manifest("zinc", "")),
            ("data/zinc/worldgen/biome/foo.json", foo),
            ("data/zinc/worldgen/biome/bar/a.json", foo),
            (
                "data/zinc/worldgen/feature/ore_zinc.json",
                r#"{"type":"minecraft:ore","config":{}}"#,
            ),
            (
                "data/zinc/worldgen/placed_feature/ore_zinc.json",
                r#"{"feature":"zinc:ore_zinc","placement":[]}"#,
            ),
            (
                "data/zinc/tags/worldgen/biome/hot.json",
                r#"{"values":["zinc:foo","zinc:missing"]}"#,
            ),
            (
                "data/zinc/neoforge/biome_modifier/hot_ore.json",
                r##"{"type":"neoforge:add_features","biomes":"#zinc:hot",
                    "features":"zinc:ore_zinc","step":"underground_ores"}"##,
            ),
        ],
    );
    let out = scratch.join("out");
    build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    let text = |root: &Path| fs::read_to_string(root.join("worldgen/biomes.json")).unwrap();
    let bundle: BTreeMap<String, String> = serde_json::from_str(&text(&out)).unwrap();
    let vanilla: BTreeMap<String, String> = serde_json::from_str(&text(&base)).unwrap();
    for (id, biome) in &vanilla {
        assert_eq!(&bundle[id], biome, "{id}");
    }
    assert_eq!(bundle.len(), vanilla.len() + 2);
    let features = biome_features(&out);
    assert_eq!(
        features["zinc:foo"],
        serde_json::json!([[], [], [], [], [], [], ["zinc:ore_zinc"]])
    );
    assert_eq!(features["zinc:bar/a"], serde_json::json!([[]]));
    // After vanilla's, then by path.
    let at = |id: &str| text(&out).find(&format!("\"{id}\":")).unwrap();
    assert!(at("minecraft:plains") < at("zinc:bar/a") && at("zinc:bar/a") < at("zinc:foo"));
    let tag: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(out.join("worldgen/biome_tags.json")).unwrap()).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&tag["zinc:hot"]).unwrap()["values"],
        serde_json::json!(["zinc:foo"])
    );
    let commands: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("command-registries.json")).unwrap()).unwrap();
    assert_eq!(
        commands["by_registry"]["minecraft:worldgen/biome"]["ids"],
        serde_json::json!([
            "minecraft:desert",
            "minecraft:plains",
            "zinc:bar/a",
            "zinc:foo"
        ])
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// A world's data pack that replaces a vanilla biome (Terralith's plains)
/// keeps the mods' modifier features, and a pack's own biome its tag puts
/// under the modifier's selector gets them: modifiers apply after
/// every pack. A biome no pack touches stays as the content root has it.
#[test]
fn biome_modifiers_apply_over_world_packs() {
    let base = biome_base("biome-modifiers-packs");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[(
            "add_zinc",
            r##"{"type":"neoforge:add_features","biomes":"#minecraft:is_overworld",
                "features":"zinc:ore_zinc","step":"underground_ores"}"##,
        )],
    );
    let out = scratch.join("out");
    build_fixture(&base, &base.join("jars"), &[m], &out).unwrap();
    let mut packs = sunrise_world::worldgen::overrides::WorldgenOverrides::default();
    let pack_plains =
        r#"{"temperature":0.5,"features":[[],[],[],[],[],[],["minecraft:ore_coal"]]}"#;
    packs.set("biomes", "minecraft:plains", pack_plains.to_owned());
    packs.set("biomes", "terra:alpine", r#"{"features":[]}"#.to_owned());
    packs.add_tag(
        "biome_tags",
        "minecraft:is_overworld",
        "file/terra",
        r#"{"values":["terra:alpine"]}"#.to_owned(),
    );
    let after = biome_modifiers_over_packs(&out, &packs, &[])
        .unwrap()
        .unwrap_or(packs);
    let features = |id: &str| -> serde_json::Value {
        let text = after.entries("biomes").find(|(i, _)| *i == id).unwrap().1;
        serde_json::from_str::<serde_json::Value>(text).unwrap()["features"].clone()
    };
    assert_eq!(
        features("minecraft:plains"),
        serde_json::json!([
            [],
            [],
            [],
            [],
            [],
            [],
            ["minecraft:ore_coal", "zinc:ore_zinc"]
        ])
    );
    assert_eq!(
        features("terra:alpine"),
        serde_json::json!([[], [], [], [], [], [], ["zinc:ore_zinc"]])
    );
    // The pack's own values stay.
    let plains = after
        .entries("biomes")
        .find(|(i, _)| *i == "minecraft:plains");
    let plains: serde_json::Value = serde_json::from_str(plains.unwrap().1).unwrap();
    assert_eq!(plains["temperature"], serde_json::json!(0.5));
    assert!(
        after
            .entries("biomes")
            .all(|(id, _)| id != "minecraft:desert")
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// Biome modifiers and structure modifiers see Sunrise's built-in `c:` and `neoforge:`
/// convention tags without a mod defining those files, and mod/pack files layer on top
/// with additive merge and `"replace": true`.
#[test]
fn biome_modifiers_resolve_builtin_convention_tags() {
    let base = biome_base("biome-modifiers-c-tags");
    let scratch = base.parent().unwrap().to_owned();

    // 1. Built-in `#c:is_plains` biome tag and `#c:bosses` entity tag resolve with no mod tag file.
    let m = zinc_mod(
        &scratch.join("zinc_default"),
        &[
            (
                "add_plains_ore",
                r##"{"type":"neoforge:add_features","biomes":"#c:is_plains",
                    "features":"zinc:ore_zinc","step":"underground_ores"}"##,
            ),
            (
                "remove_bosses",
                r##"{"type":"neoforge:remove_spawns","biomes":"#c:is_plains",
                    "entity_types":"#c:bosses"}"##,
            ),
        ],
    );
    let out = scratch.join("out_default");
    let warnings = build_fixture(&base, &base.join("jars"), &[m], &out).unwrap();
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    let features = biome_features(&out);
    assert_eq!(
        features["minecraft:plains"],
        serde_json::json!([
            [],
            ["minecraft:lake"],
            [],
            [],
            [],
            [],
            ["minecraft:ore_coal", "zinc:ore_zinc"]
        ])
    );
    assert_eq!(
        features["minecraft:desert"],
        serde_json::json!([["minecraft:fossil"]])
    );

    // 2. `biome_modifiers_over_packs` sees built-in `#c:is_plains` and layers pack additions on top.
    let mut packs = sunrise_world::worldgen::overrides::WorldgenOverrides::default();
    packs.set(
        "biomes",
        "minecraft:plains",
        r#"{"temperature":0.5,"features":[[],[],[],[],[],[],["minecraft:ore_coal"]]}"#.to_owned(),
    );
    packs.set("biomes", "terra:alpine", r#"{"features":[]}"#.to_owned());
    packs.add_tag(
        "biome_tags",
        "c:is_plains",
        "file/terra",
        r#"{"values":["terra:alpine"]}"#.to_owned(),
    );
    let after = biome_modifiers_over_packs(&out, &packs, &[])
        .unwrap()
        .unwrap();
    let pack_features = |id: &str| -> serde_json::Value {
        let text = after.entries("biomes").find(|(i, _)| *i == id).unwrap().1;
        serde_json::from_str::<serde_json::Value>(text).unwrap()["features"].clone()
    };
    assert_eq!(
        pack_features("minecraft:plains"),
        serde_json::json!([
            [],
            [],
            [],
            [],
            [],
            [],
            ["minecraft:ore_coal", "zinc:ore_zinc"]
        ])
    );
    assert_eq!(
        pack_features("terra:alpine"),
        serde_json::json!([[], [], [], [], [], [], ["zinc:ore_zinc"]])
    );

    // 3. Mod additive tag file on `c:is_plains` keeps built-in `minecraft:plains` and adds `minecraft:desert`.
    let m_add = zinc_mod(
        &scratch.join("zinc_add"),
        &[(
            "add_plains_ore",
            r##"{"type":"neoforge:add_features","biomes":"#c:is_plains",
                "features":"zinc:ore_zinc","step":"underground_ores"}"##,
        )],
    );
    let add_tag = scratch.join("zinc_add/data/c/tags/worldgen/biome/is_plains.json");
    fs::create_dir_all(add_tag.parent().unwrap()).unwrap();
    fs::write(&add_tag, r#"{"values":["minecraft:desert"]}"#).unwrap();
    let out_add = scratch.join("out_add");
    let warnings = build_fixture(&base, Path::new(""), &[m_add], &out_add).unwrap();
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    let features = biome_features(&out_add);
    assert_eq!(
        features["minecraft:plains"][6],
        serde_json::json!(["minecraft:ore_coal", "zinc:ore_zinc"])
    );
    assert_eq!(
        features["minecraft:desert"][6],
        serde_json::json!(["zinc:ore_zinc"])
    );

    // 4. Mod `"replace": true` on `c:is_plains` drops built-in `minecraft:plains`.
    let m_replace = zinc_mod(
        &scratch.join("zinc_replace"),
        &[(
            "add_plains_ore",
            r##"{"type":"neoforge:add_features","biomes":"#c:is_plains",
                "features":"zinc:ore_zinc","step":"underground_ores"}"##,
        )],
    );
    let replace_tag = scratch.join("zinc_replace/data/c/tags/worldgen/biome/is_plains.json");
    fs::create_dir_all(replace_tag.parent().unwrap()).unwrap();
    fs::write(
        &replace_tag,
        r#"{"replace":true,"values":["minecraft:desert"]}"#,
    )
    .unwrap();
    let out_replace = scratch.join("out_replace");
    let warnings = build_fixture(&base, Path::new(""), &[m_replace], &out_replace).unwrap();
    assert!(
        warnings.iter().all(|w| w.contains("replaces data/c/tags/")),
        "unexpected warnings: {warnings:?}"
    );
    let features = biome_features(&out_replace);
    assert_eq!(
        features["minecraft:plains"][6],
        serde_json::json!(["minecraft:ore_coal"])
    );
    assert_eq!(
        features["minecraft:desert"][6],
        serde_json::json!(["zinc:ore_zinc"])
    );

    // 5. Structure modifiers resolve built-in `#c:` structure and entity tags without warnings.
    let s_base = structure_modifier_base("structure-modifiers-c-tags");
    let s_scratch = s_base.parent().unwrap().to_owned();
    let sm = structure_modifier_mod(
        &s_scratch.join("smod"),
        &[(
            "remove_bosses",
            r##"{"type":"neoforge:remove_spawns","structures":"#c:hidden_from_displayers",
                "entity_types":"#c:bosses"}"##,
        )],
    );
    let s_warnings = build_fixture(&s_base, Path::new(""), &[sm], &s_scratch.join("out")).unwrap();
    assert!(
        s_warnings.is_empty(),
        "unexpected structure modifier warnings: {s_warnings:?}"
    );
    let _ = fs::remove_dir_all(&s_scratch);
    let _ = fs::remove_dir_all(&scratch);
}

/// A mod with features but no modifier leaves vanilla's biome bundle alone.
#[test]
fn biome_modifiers_none_keep_the_vanilla_biomes() {
    let base = biome_base("biome-modifiers-none");
    let scratch = base.parent().unwrap().to_owned();
    let out = scratch.join("out");
    build_fixture(
        &base,
        Path::new(""),
        &[zinc_mod(&scratch.join("zinc"), &[])],
        &out,
    )
    .unwrap();
    assert_eq!(
        fs::read(out.join("worldgen/biomes.json")).unwrap(),
        fs::read(base.join("worldgen/biomes.json")).unwrap()
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// An unknown biome, biome tag, feature, entity type, step or modifier type is
/// skipped with a warning or fails the build with the mod and file named.
#[test]
fn biome_modifiers_refuse_unknown_names() {
    let base = biome_base("biome-modifiers-errors");
    let scratch = base.parent().unwrap().to_owned();
    // A name the root doesn't have, or a type not read yet: that modifier is
    // skipped with a warning (another mod's ids, unknown tags and its own serializers
    // are common in real mods), and the mods still load.
    for (i, (modifier, warning)) in [
        (
            r##"{"type":"neoforge:add_features","biomes":"#minecraft:is_moon",
                "features":"zinc:ore_zinc","step":"underground_ores"}"##,
            "unknown biome tag #minecraft:is_moon",
        ),
        (
            r#"{"type":"neoforge:add_features","biomes":"minecraft:moon",
                "features":"zinc:ore_zinc","step":"underground_ores"}"#,
            "unknown biome minecraft:moon",
        ),
        (
            r#"{"type":"neoforge:add_features","biomes":"minecraft:plains",
                "features":"zinc:ore_tin","step":"underground_ores"}"#,
            "unknown placed feature zinc:ore_tin",
        ),
        (
            r#"{"type":"neoforge:add_spawns","biomes":"minecraft:plains",
                "spawners":{"type":"zinc:goblin","weight":10,"minCount":1,"maxCount":2}}"#,
            "unknown entity type zinc:goblin",
        ),
        (
            r##"{"type":"neoforge:remove_spawns","biomes":"minecraft:plains",
                "entity_types":"#minecraft:is_goblin"}"##,
            "unknown entity type tag #minecraft:is_goblin",
        ),
        (
            r#"{"type":"examplemod:custom","biomes":"minecraft:plains"}"#,
            "biome modifier type \"examplemod:custom\" skipped",
        ),
        (
            r#"{"type":"neoforge:add_features","biomes":"minecraft:plains",
                "features":{"type":"sunrise:climate","temperature":{"min":1}},"step":"underground_ores"}"#,
            "placed feature set type \"sunrise:climate\" is not read",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let m = zinc_mod(&scratch.join(format!("zinc{i}")), &[("bad", modifier)]);
        let warnings = build_fixture(&base, Path::new(""), &[m], &scratch.join(format!("out{i}"))).unwrap();
        assert!(
            warnings.iter().any(|w| w.contains(warning)
                && w.contains("mod zinc: data/zinc/neoforge/biome_modifier/bad.json")),
            "{warnings:?}"
        );
    }
    // A malformed modifier of a type Sunrise reads is the mod's error.
    for (i, (modifier, err)) in [
        (
            r#"{"type":"neoforge:add_features","biomes":"minecraft:plains",
                "features":"zinc:ore_zinc","step":"ores"}"#,
            "unknown generation step \"ores\"",
        ),
        (
            r#"{"type":"neoforge:add_spawns","biomes":"minecraft:plains",
                "spawners":{"type":"minecraft:zombie","weight":10,"minCount":5,"maxCount":2}}"#,
            "minCount (5) > maxCount (2)",
        ),
        (
            r#"{"type":"neoforge:add_spawn_costs","biomes":"minecraft:plains",
                "entity_types":"minecraft:zombie","spawn_cost":{"energy_budget":0.7}}"#,
            "spawn_cost: no charge",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let m = zinc_mod(&scratch.join(format!("zinc-err{i}")), &[("bad", modifier)]);
        let e = build_fixture(
            &base,
            Path::new(""),
            &[m],
            &scratch.join(format!("out-err{i}")),
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains(err), "{e}");
        assert!(
            e.contains("mod zinc: data/zinc/neoforge/biome_modifier/bad.json"),
            "{e}"
        );
    }
    let _ = fs::remove_dir_all(&scratch);
}

/// `add_spawns`, `remove_spawns`, `add_spawn_costs` and `remove_spawn_costs`
/// rewrite `minecraft:gameplay/natural_mob_spawns` in the biomes they name:
/// additions before removals regardless of file order, spawners placed under
/// their entity type's category with `minCount`/`maxCount` converted to
/// `count`, and untouched biomes left byte for byte as they were.
#[test]
fn biome_modifiers_change_mob_spawns_and_spawn_costs() {
    let base = biome_base("biome-modifiers-spawns");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[
            // Listed before `add_spawns`, but removals run after additions:
            // takes out vanilla's `minecraft:skeleton` and the added
            // `minecraft:stray` via `#minecraft:skeletons`.
            (
                "a_no_skeletons",
                r##"{"type":"neoforge:remove_spawns","biomes":"#minecraft:is_overworld",
                    "entity_types":"#minecraft:skeletons"}"##,
            ),
            (
                "a_no_skeleton_cost",
                r##"{"type":"neoforge:remove_spawn_costs","biomes":"minecraft:plains",
                    "entity_types":"#minecraft:skeletons"}"##,
            ),
            // Single spawner object (`minCount == maxCount` -> integer `count`)
            // on a biome that had no `attributes` at all (`desert`), and on `plains`.
            (
                "add_rabbits",
                r##"{"type":"neoforge:add_spawns","biomes":"#minecraft:is_overworld",
                    "spawners":{"type":"minecraft:rabbit","weight":12,"minCount":3,"maxCount":3}}"##,
            ),
            // Array of spawners (`minCount < maxCount` -> `minecraft:uniform`).
            (
                "add_monsters",
                r#"{"type":"forge:add_spawns","biomes":"plains",
                    "spawners":[
                        {"type":"stray","weight":80,"minCount":1,"maxCount":4},
                        {"type":"minecraft:zombie","weight":20,"minCount":2,"maxCount":5}
                    ]}"#,
            ),
            (
                "add_zombie_cost",
                r#"{"type":"neoforge:add_spawn_costs","biomes":["minecraft:plains","minecraft:desert"],
                    "entity_types":["minecraft:zombie","minecraft:stray"],
                    "spawn_cost":{"energy_budget":0.85,"charge":0.2}}"#,
            ),
        ],
    );
    let out = scratch.join("out");
    let warnings = build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");

    let text = |root: &Path| -> BTreeMap<String, String> {
        serde_json::from_slice(&fs::read(root.join("worldgen/biomes.json")).unwrap()).unwrap()
    };
    let out_biomes = text(&out);
    // Untouched biome keeps its exact bytes.
    assert_eq!(
        out_biomes["minecraft:nether_wastes"],
        text(&base)["minecraft:nether_wastes"]
    );

    let plains: serde_json::Value = serde_json::from_str(&out_biomes["minecraft:plains"]).unwrap();
    let plains_spawns = &plains["attributes"]["minecraft:gameplay/natural_mob_spawns"]["argument"];
    // `minecraft:skeleton` and the added `minecraft:stray` were removed by
    // `remove_spawns`, while vanilla's zombie, the second zombie spawner, and
    // the rabbit in `creature` remain.
    assert_eq!(
        plains_spawns["spawns_by_category"],
        serde_json::json!({
            "monster": [
                {"type": "minecraft:zombie", "count": 4, "weight": 95},
                {
                    "type": "minecraft:zombie",
                    "count": {"type": "minecraft:uniform", "min_inclusive": 2, "max_inclusive": 5},
                    "weight": 20
                }
            ],
            "creature": [
                {"type": "minecraft:rabbit", "count": 3, "weight": 12}
            ]
        })
    );
    // `skeleton` and `stray` costs were removed on `plains`; `zombie` cost remains.
    assert_eq!(
        plains_spawns["spawn_costs"],
        serde_json::json!({
            "minecraft:zombie": {"energy_budget": 0.85, "charge": 0.2}
        })
    );

    let desert: serde_json::Value = serde_json::from_str(&out_biomes["minecraft:desert"]).unwrap();
    let desert_spawns = &desert["attributes"]["minecraft:gameplay/natural_mob_spawns"]["argument"];
    assert_eq!(
        desert_spawns["spawns_by_category"],
        serde_json::json!({
            "creature": [
                {"type": "minecraft:rabbit", "count": 3, "weight": 12}
            ]
        })
    );
    assert_eq!(
        desert_spawns["spawn_costs"],
        serde_json::json!({
            "minecraft:zombie": {"energy_budget": 0.85, "charge": 0.2},
            "minecraft:stray": {"energy_budget": 0.85, "charge": 0.2}
        })
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// Over the real prepared resources: a mod's ore, added to
/// `#minecraft:is_overworld` by a biome modifier, generates in a fresh
/// world's chunk; without the modifier the same placed feature does not
/// (its `biome` filter only passes in biomes that list it).
#[test]
#[ignore = "daily"]
fn content_root_generates_a_biome_modifier_ore() {
    use sunrise_level::chunk_streamer::{ChunkGenerator, Proto};
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-ore-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let ore = |modifier: bool| -> usize {
        let mut files = vec![
            ("sunrise-mod.toml", manifest("zinc", "")),
            (
                "data/zinc/sunrise/block/zinc_ore.json",
                r#"{"template": "minecraft:iron_ore"}"#.to_owned(),
            ),
            (
                "data/zinc/worldgen/feature/ore_zinc.json",
                r##"{"type": "minecraft:ore", "size": 9, "discard_chance_on_air_exposure": 0.0,
                    "targets": [{"target": {"predicate_type": "minecraft:tag_match",
                        "tag": "minecraft:stone_ore_replaceables"},
                        "state": "zinc:zinc_ore"}]}"##
                    .to_owned(),
            ),
            (
                "data/zinc/worldgen/placed_feature/ore_zinc.json",
                r#"{"feature": "zinc:ore_zinc", "placement": [
                    {"type": "minecraft:count", "count": 40}, {"type": "minecraft:in_square"},
                    {"type": "minecraft:height_range", "height": {"type": "minecraft:uniform",
                        "min_inclusive": {"absolute": -32}, "max_inclusive": {"absolute": 48}}},
                    {"type": "minecraft:biome"}]}"#
                    .to_owned(),
            ),
        ];
        if modifier {
            files.push((
                "data/zinc/neoforge/biome_modifier/add_zinc.json",
                r##"{"type": "neoforge:add_features", "biomes": "#minecraft:is_overworld",
                    "features": "zinc:ore_zinc", "step": "underground_ores"}"##
                    .to_owned(),
            ));
        }
        let files: Vec<(&str, &str)> = files.iter().map(|(a, b)| (*a, b.as_str())).collect();
        let dir = scratch.join(format!("zinc-{modifier}"));
        let m = folder_mod(&dir, &files);
        let root = resource_root_fixture(&base, Path::new(""), &[m], &dir.join("cache")).unwrap();
        let assets = Assets::load(&root).unwrap();
        let zinc = assets.registry.default_state("zinc:zinc_ore").unwrap().0;
        let biomes = crate::biome::Biomes::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        )
        .unwrap();
        let item_data =
            crate::test_resources::item_data(&root, DataLoadOrigin::ServerDataLoad).unwrap();
        let terrain_resources = crate::vanillagen::TerrainResources {
            data: None,
            dir: &root.join("worldgen"),
            registry: assets.registry,
            biomes: &biomes,
            item_data: &item_data,
        };
        let terrain = crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
        let mut worker = terrain.worker();
        let mut region: [Proto; 9] = std::array::from_fn(|i| {
            terrain
                .proto(&mut worker, i as i32 % 3 - 1, i as i32 / 3 - 1)
                .unwrap()
        });
        terrain.decorate(&mut worker, &mut region);
        let [_, _, _, _, center, ..] = region;
        let chunk = terrain.finish(center);
        let mut count = 0;
        for y in chunk.min_y()..chunk.max_y() {
            for z in 0..16 {
                for x in 0..16 {
                    count += usize::from(chunk.get(x, y, z) == zinc);
                }
            }
        }
        count
    };
    assert!(ore(true) > 0, "the modifier's ore generates");
    assert_eq!(ore(false), 0, "no modifier, no ore");
    let _ = fs::remove_dir_all(&scratch);
}

/// `biomes` may be one id or a list of ids as well as a tag (NeoForge's
/// HolderSet forms); an id may leave out `minecraft:`.
#[test]
fn biome_modifiers_take_one_biome_or_a_list() {
    let base = biome_base("biome-modifiers-lists");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[
            (
                "list",
                r#"{"type":"neoforge:add_features","biomes":["minecraft:plains","nether_wastes"],
                    "features":"zinc:ore_zinc","step":"underground_ores"}"#,
            ),
            (
                "one",
                r#"{"type":"neoforge:add_features","biomes":"minecraft:desert",
                    "features":["minecraft:lake"],"step":"lakes"}"#,
            ),
        ],
    );
    let out = scratch.join("out");
    build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    let features = biome_features(&out);
    assert_eq!(
        features["minecraft:plains"][6],
        serde_json::json!(["minecraft:ore_coal", "zinc:ore_zinc"])
    );
    assert_eq!(
        features["minecraft:nether_wastes"][6],
        serde_json::json!(["minecraft:ore_coal", "zinc:ore_zinc"])
    );
    assert_eq!(
        features["minecraft:desert"],
        serde_json::json!([["minecraft:fossil"], ["minecraft:lake"]])
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// Blocks and items a native mod declares in code join the content plan:
/// inside the content mod of their namespace, or as one of their own, with
/// ids after vanilla's as files' are.
#[test]
#[ignore = "daily"]
fn blocks_and_items_declared_in_code_get_ids() {
    use sunrise_mod_package::content::{Block, Item};
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-declared-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let known = known_components(&base).unwrap();
    let block = |id: &str| Block {
        id: id.into(),
        template: "minecraft:oak_log".into(),
        item: true,
        ..Block::default()
    };
    let wrench = Item {
        id: "gears:wrench".into(),
        template: "minecraft:stick".into(),
        max_stack_size: Some(1),
        ..Item::default()
    };
    let mut mods = vec![ruby_mod(&scratch.join("ruby"))];
    let batches = BTreeMap::from([
        (
            "ruby",
            DeclaredContent {
                blocks: vec![block("ruby:ruby_log").validate_local().unwrap()],
                ..Default::default()
            },
        ),
        (
            "gears",
            DeclaredContent {
                blocks: vec![block("gears:shaft").validate_local().unwrap()],
                items: vec![wrench.validate_local().unwrap()],
                ..Default::default()
            },
        ),
    ]);
    let mut errors = Vec::new();
    accept_declared(&mut mods, &batches, &[], &known, &mut errors).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(mods.len(), 2);
    assert!(
        mods[0]
            .content
            .blocks
            .iter()
            .any(|b| b.id == "ruby:ruby_log")
    );
    assert_eq!(mods[1].id, "gears");
    // A namespace another mod owns, and a clash with a file's block, are refused whole.
    let colliding = BTreeMap::from([(
        "ruby",
        DeclaredContent {
            blocks: [block("ruby:ruby_block"), block("ruby:ruby_post")]
                .into_iter()
                .map(|b| b.validate_local().unwrap())
                .collect(),
            ..Default::default()
        },
    )]);
    accept_declared(&mut mods, &colliding, &[], &known, &mut errors).unwrap();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        !mods[0]
            .content
            .blocks
            .iter()
            .any(|b| b.id == "ruby:ruby_post")
    );

    let root = resource_root_fixture(&base, Path::new(""), &mods, &scratch.join("cache")).unwrap();
    let vanilla = Assets::load(&base).unwrap();
    let assets = Assets::load(&root).unwrap();
    let n = vanilla.registry.len() as u32;
    let log_states = (0..n)
        .filter(|&s| vanilla.registry.state(BlockStateId(s)).unwrap().name == "minecraft:oak_log")
        .count() as u32;
    // Sorted by mod, then block: gears:shaft, then ruby's ruby_block, ruby_door, ruby_glass, ruby_log.
    let shaft = (n..assets.registry.len() as u32)
        .find(|&s| assets.registry.state(BlockStateId(s)).unwrap().name == "gears:shaft")
        .unwrap();
    assert_eq!(shaft, n);
    assert!(assets.registry.default_state("ruby:ruby_log").is_some());
    assert_eq!(
        assets.registry.len() as u32,
        n + log_states + 1 + 64 + 1 + log_states
    );
    let ids = sunrise_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap();
    for item in ["gears:shaft", "gears:wrench", "ruby:ruby_log"] {
        assert!(
            ids.get("minecraft:item").unwrap().id(item).is_some(),
            "{item}"
        );
    }
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let items = Items::load(
        &root,
        assets.registry,
        &facts,
        &PackOverlay::NONE,
        &defaults,
    )
    .unwrap();
    assert_eq!(items.items["gears:wrench"].max_stack, 1);
    assert!(items.items["gears:shaft"].block.is_some());
    // The same declarations reuse the root; others build a new one.
    assert_eq!(
        resource_root_fixture(&base, Path::new(""), &mods, &scratch.join("cache")).unwrap(),
        root
    );
    // Changed declarations build a root of their own: the old one stays as
    // it was, for a switch back to that set.
    let stamp = fs::read(root.join(STAMP)).unwrap();
    mods[1].content.items[0].max_stack_size = Some(2);
    let changed =
        resource_root_fixture(&base, Path::new(""), &mods, &scratch.join("cache")).unwrap();
    assert_ne!(changed, root);
    assert_eq!(fs::read(root.join(STAMP)).unwrap(), stamp);
    assert_ne!(fs::read(changed.join(STAMP)).unwrap(), stamp);
    let changed_defaults = sunrise_world::item_defaults::read(&changed).unwrap();
    let changed_items = Items::load(
        &changed,
        assets.registry,
        &facts,
        &PackOverlay::NONE,
        &changed_defaults,
    )
    .unwrap();
    assert_eq!(changed_items.items["gears:wrench"].max_stack, 2);
    assert_eq!(items.items["gears:wrench"].max_stack, 1);
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
#[ignore = "daily"]
fn item_references_resolve_against_other_packages_registered_content() {
    use sunrise_mod_package::{Manifest, content::Item};

    let base = prepared().expect("daily test requires prepared resources");
    let scratch = std::env::temp_dir().join(format!(
        "sunrise-cross-item-references-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&scratch);
    let addon = scratch.join("addon");
    let declarations = addon.join("data/addon/sunrise/item");
    fs::create_dir_all(&declarations).unwrap();
    fs::write(
        declarations.join("planter.json"),
        r#"{"template":"minecraft:stick","block":"provider:crop","crafting_remainder":"provider:handle"}"#,
    ).unwrap();
    let manifest = Manifest::parse(
        r#"
        id = "addon"
        name = "Planter"
        version = "1.0.0"
        api = "0.2"
        side = "both"
        capabilities = ["registries"]
    "#,
    )
    .unwrap();
    let mut addon = Content::read(&addon, &manifest).unwrap();
    addon
        .declare(
            &[],
            &[Item {
                id: "addon:tool".into(),
                template: "minecraft:stick".into(),
                block: Some("provider:crop".into()),
                crafting_remainder: Some("provider:handle".into()),
                ..Item::default()
            }],
        )
        .unwrap();
    let mut provider = Content::declared("provider");
    provider
        .declare(
            &[Block {
                id: "provider:crop".into(),
                template: "minecraft:glass".into(),
                item: false,
                ..Block::default()
            }],
            &[Item {
                id: "provider:handle".into(),
                template: "minecraft:bowl".into(),
                ..Item::default()
            }],
        )
        .unwrap();
    let mut mods = [
        ContentMod {
            id: "addon".into(),
            content: addon,
            jars: vec![],
        },
        ContentMod {
            id: "provider".into(),
            content: provider,
            jars: vec![],
        },
    ];
    let root = scratch.join("out");
    build_fixture(&base, Path::new(""), &mods, &root).unwrap();
    let assets = Assets::load(&root).unwrap();
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let items = Items::load(
        &root,
        assets.registry,
        &facts,
        &PackOverlay::NONE,
        &defaults,
    )
    .unwrap();
    let crop = assets.registry.default_state("provider:crop").unwrap();
    for id in ["addon:planter", "addon:tool"] {
        let item = &items.items[id];
        assert_eq!(item.block, Some(crop));
        assert_eq!(item.crafting_remainder.as_deref(), Some("provider:handle"));
    }

    mods[1].content.items.clear();
    let error = build_fixture(&base, Path::new(""), &mods, &scratch.join("no-item")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "items.json: addon:planter: crafting_remainder \"provider:handle\" is not an item"
    );
    mods[0].content.items[0].crafting_remainder = Some("provider:crop".into());
    let error = build_fixture(&base, Path::new(""), &mods, &scratch.join("itemless")).unwrap_err();
    assert_eq!(
        error.to_string(),
        r#"items.json: addon:planter: crafting_remainder "provider:crop" is not an item"#,
    );
    mods[1].content.blocks.clear();
    let error = build_fixture(&base, Path::new(""), &mods, &scratch.join("no-block")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "addon:planter: block \"provider:crop\" is not a block"
    );
    fs::remove_dir_all(&scratch).unwrap();
}

/// A fence gate of a mod is a vanilla `FenceGateBlock` through its template, with
/// its wood's sounds unless the file names its own: `open_sound` and
/// `close_sound` override every site or just the named sites, retaining the
/// template wood for each missing site.
#[test]
#[ignore = "daily"]
fn a_content_fence_gate_plays_its_own_sounds_or_its_woods() {
    use sunrise_world::level::Behaviours;

    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-gates-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("gates"),
        &[
            ("sunrise-mod.toml", &manifest("gates", "")),
            (
                "data/gates/sunrise/block/oak.json",
                r#"{"template": "minecraft:oak_fence_gate"}"#,
            ),
            (
                "data/gates/sunrise/block/rope.json",
                r#"{"template": "minecraft:oak_fence_gate", "open_sound": "gates:rope.open", "close_sound": "minecraft:block.chain.break"}"#,
            ),
            (
                "data/gates/sunrise/block/half.json",
                r#"{"template": "minecraft:birch_fence_gate", "open_sound": "gates:half.open"}"#,
            ),
            (
                "data/gates/sunrise/block/sites.json",
                r#"{"template":"minecraft:oak_fence_gate","open_sound":{"use":"gates:use.open","redstone":"gates:wire.open"},"close_sound":{"explosion":"gates:blast.close"}}"#,
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
    let registry = Assets::load(&root).unwrap().registry;
    let behaviours = Behaviours::load(registry, &root).unwrap();
    let sounds = |name: &str| {
        let state = registry.default_state(name).unwrap();
        let data = behaviours.data(state);
        (
            std::array::from_fn::<_, 3, _>(|i| data.gate_open[i].as_deref().map(str::to_owned)),
            std::array::from_fn::<_, 3, _>(|i| data.gate_close[i].as_deref().map(str::to_owned)),
        )
    };
    let gate = |open: Option<&str>, close: Option<&str>| {
        (
            std::array::from_fn::<_, 3, _>(|_| open.map(str::to_owned)),
            std::array::from_fn::<_, 3, _>(|_| close.map(str::to_owned)),
        )
    };
    assert_eq!(
        sounds("gates:oak"),
        gate(
            Some("minecraft:block.fence_gate.open"),
            Some("minecraft:block.fence_gate.close")
        )
    );
    assert_eq!(
        sounds("gates:rope"),
        gate(Some("gates:rope.open"), Some("minecraft:block.chain.break"))
    );
    assert_eq!(
        sounds("gates:half"),
        gate(
            Some("gates:half.open"),
            Some("minecraft:block.fence_gate.close")
        )
    );
    assert_eq!(
        sounds("gates:sites"),
        (
            [
                "gates:use.open",
                "minecraft:block.fence_gate.open",
                "gates:wire.open"
            ]
            .map(|s| Some(s.to_owned())),
            [
                "minecraft:block.fence_gate.close",
                "gates:blast.close",
                "minecraft:block.fence_gate.close"
            ]
            .map(|s| Some(s.to_owned())),
        )
    );
    assert_eq!(
        sounds("minecraft:oak_fence_gate"),
        sounds("gates:oak"),
        "the vanilla gate keeps its sounds"
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// A block's item is one value in its gameplay row: its own id by default, the
/// `block_item` it names (a mod's own item or a vanilla one), or none with
/// `item` false. A `block_item` that is no item refuses the load. Pick-block's item
/// is that value unless the block declares a `pick_item` of its own.
#[test]
#[ignore = "daily"]
fn a_block_yields_the_item_it_names_or_its_own_or_none() {
    use sunrise_world::level::Behaviours;

    let Some(base) = prepared() else {
        return;
    };
    let scratch =
        std::env::temp_dir().join(format!("sunrise-mod-blockitem-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("pick"),
        &[
            ("sunrise-mod.toml", &manifest("pick", "")),
            (
                "data/pick/sunrise/block/plain.json",
                r#"{"template": "minecraft:stone"}"#,
            ),
            (
                "data/pick/sunrise/block/pillar.json",
                r#"{"template": "minecraft:stone", "item": false, "block_item": "pick:rod"}"#,
            ),
            (
                "data/pick/sunrise/block/gem_ore.json",
                r#"{"template": "minecraft:stone", "item": false, "block_item": "minecraft:diamond"}"#,
            ),
            (
                "data/pick/sunrise/block/ghost.json",
                r#"{"template": "minecraft:stone", "item": false}"#,
            ),
            (
                "data/pick/sunrise/block/crop.json",
                r#"{"template": "minecraft:stone", "item": false, "pick_item": "minecraft:wheat_seeds"}"#,
            ),
            (
                "data/pick/sunrise/block/gilded.json",
                r#"{"template": "minecraft:stone", "pick_item": "pick:rod"}"#,
            ),
            (
                "data/pick/sunrise/item/rod.json",
                r#"{"template": "minecraft:stick"}"#,
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
    let registry = Assets::load(&root).unwrap().registry;
    let facts = BlockFacts::load(&root, registry).unwrap();
    let behaviours = Behaviours::load(registry, &root).unwrap();
    let item = |name: &str| {
        let state = registry.default_state(name).unwrap();
        facts.get(state).item.to_string()
    };
    assert_eq!(item("pick:plain"), "pick:plain");
    assert_eq!(item("pick:pillar"), "pick:rod");
    assert_eq!(item("pick:gem_ore"), "minecraft:diamond");
    assert_eq!(item("pick:ghost"), "minecraft:air");
    let pick = |name: &str| {
        let state = registry.default_state(name).unwrap();
        facts.get(state).pick_item.to_string()
    };
    // Without a `pick_item`, pick-block takes the block's item.
    assert_eq!(pick("pick:plain"), "pick:plain");
    assert_eq!(pick("pick:pillar"), "pick:rod");
    assert_eq!(pick("pick:ghost"), "minecraft:air");
    // With one, it takes that, and `Block.asItem` is unchanged.
    assert_eq!(pick("pick:crop"), "minecraft:wheat_seeds");
    assert_eq!(item("pick:crop"), "minecraft:air");
    assert_eq!(pick("pick:gilded"), "pick:rod");
    assert_eq!(item("pick:gilded"), "pick:gilded");
    let id = |name: &str| behaviours.table.block_by_name(name).unwrap();
    assert_eq!(&*behaviours.block_data(id("pick:pillar")).item, "pick:rod");
    assert_eq!(
        &*behaviours.block_data(id("pick:ghost")).item,
        "minecraft:air"
    );

    let missing = folder_mod(
        &scratch.join("lost"),
        &[
            ("sunrise-mod.toml", &manifest("lost", "")),
            (
                "data/lost/sunrise/block/pillar.json",
                r#"{"template": "minecraft:stone", "item": false, "block_item": "lost:rod"}"#,
            ),
        ],
    );
    let e = resource_root_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&missing),
        &scratch.join("cache2"),
    )
    .err()
    .unwrap()
    .to_string();
    assert!(e.contains("lost:pillar") && e.contains("lost:rod"), "{e}");
    let missing = folder_mod(
        &scratch.join("gone"),
        &[
            ("sunrise-mod.toml", &manifest("gone", "")),
            (
                "data/gone/sunrise/block/crop.json",
                r#"{"template": "minecraft:stone", "item": false, "pick_item": "gone:seed"}"#,
            ),
        ],
    );
    let e = resource_root_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&missing),
        &scratch.join("cache3"),
    )
    .err()
    .unwrap()
    .to_string();
    assert!(e.contains("gone:crop") && e.contains("gone:seed"), "{e}");
    let _ = fs::remove_dir_all(&scratch);
}

/// `WorldGenRegion.setBlock`: a feature that places a mod's sign leaves it its
/// block entity, which goes by the template's class and not the block's name.
#[test]
#[ignore = "daily"]
fn a_feature_gives_a_mods_sign_its_block_entity() {
    use crate::containers::BlockEntity;
    use sunrise_level::chunk_streamer::{ChunkGenerator, Proto};

    let Some(base) = prepared() else {
        return;
    };
    let scratch =
        std::env::temp_dir().join(format!("sunrise-mod-sign-feature-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("post"),
        &[
            ("sunrise-mod.toml", &manifest("post", "")),
            (
                "data/post/sunrise/block/notice.json",
                r#"{"template": "minecraft:oak_sign"}"#,
            ),
            (
                "data/post/worldgen/feature/notice.json",
                r#"{"type": "minecraft:ore", "size": 9, "discard_chance_on_air_exposure": 0.0,
                    "targets": [{"target": {"predicate_type": "minecraft:tag_match",
                        "tag": "minecraft:stone_ore_replaceables"},
                        "state": "post:notice"}]}"#,
            ),
            (
                "data/post/worldgen/placed_feature/notice.json",
                r#"{"feature": "post:notice", "placement": [
                    {"type": "minecraft:count", "count": 40}, {"type": "minecraft:in_square"},
                    {"type": "minecraft:height_range", "height": {"type": "minecraft:uniform",
                        "min_inclusive": {"absolute": -32}, "max_inclusive": {"absolute": 48}}},
                    {"type": "minecraft:biome"}]}"#,
            ),
            (
                "data/post/neoforge/biome_modifier/add_notice.json",
                r##"{"type": "neoforge:add_features", "biomes": "#minecraft:is_overworld",
                    "features": "post:notice", "step": "underground_ores"}"##,
            ),
        ],
    );
    let root = resource_root_fixture(&base, Path::new(""), &[m], &scratch.join("cache")).unwrap();
    let assets = Assets::load(&root).unwrap();
    let registry = assets.registry;
    let facts = BlockFacts::load(&root, registry).unwrap();
    let biomes = crate::biome::Biomes::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&root, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: None,
        dir: &root.join("worldgen"),
        registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let notice = registry.default_state("post:notice").unwrap().0;
    let generate = |signs: bool| -> (usize, usize) {
        let mut terrain = crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
        if signs {
            terrain.add_mod_signs(&facts);
        }
        let mut worker = terrain.worker();
        let mut region: [Proto; 9] = std::array::from_fn(|i| {
            terrain
                .proto(&mut worker, i as i32 % 3 - 1, i as i32 / 3 - 1)
                .unwrap()
        });
        terrain.decorate(&mut worker, &mut region);
        let [_, _, _, _, center, ..] = region;
        let chunk = terrain.finish(center);
        let mut placed = 0;
        for y in chunk.min_y()..chunk.max_y() {
            for z in 0..16 {
                for x in 0..16 {
                    placed += usize::from(chunk.get(x, y, z) == notice);
                }
            }
        }
        let entities = chunk
            .block_entities
            .iter()
            .filter(|(_, e)| matches!(e, BlockEntity::Sign(s) if !s.hanging))
            .count();
        (placed, entities)
    };
    let (placed, entities) = generate(false);
    assert!(placed > 0, "the feature places the sign");
    assert_eq!(entities, 0, "by its name alone it has none");
    assert_eq!(generate(true), (placed, placed));
    let _ = fs::remove_dir_all(&scratch);
}

/// Every folder (`None`) and file (`Some(bytes)`) under `root`, by relative path.
fn tree(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                out.insert(rel, None);
                walk(root, &path, out);
            } else {
                out.insert(rel, Some(fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// Writes into a file where it is (same data), not by replacing it: what a link shares and a copy does not.
fn edit_in_place(path: &Path, bytes: &[u8]) {
    use std::io::Write;
    let mut file = fs::File::options()
        .write(true)
        .truncate(true)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
}

/// The overlay looks like the base, and shares its files (a hard link on Windows, a symlink elsewhere) rather than
/// holding copies of them.
#[test]
fn the_overlay_shares_the_base_files() {
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-share-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let (base, out) = (scratch.join("base"), scratch.join("root"));
    small_base(&base);
    Overlay::new(&base, &out).unwrap();
    assert_eq!(tree(&out), tree(&base));
    edit_in_place(
        &base.join("assets/minecraft/lang/en_us.json"),
        b"{\"a\": 2}",
    );
    assert_eq!(
        fs::read(out.join("assets/minecraft/lang/en_us.json")).unwrap(),
        b"{\"a\": 2}",
        "the overlay copied the base file instead of linking it"
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// Writing a file of the overlay replaces the name, so the base file it was linked to keeps its bytes and the other
/// files of the overlay still follow the base.
#[test]
fn writing_into_the_overlay_leaves_the_base_alone() {
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-write-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let (base, out) = (scratch.join("base"), scratch.join("root"));
    small_base(&base);
    let overlay = Overlay::new(&base, &out).unwrap();
    let before = tree(&base);
    overlay
        .write(
            "data/minecraft/tags/block/logs.json",
            b"{\"values\": [\"mod:log\"]}",
        )
        .unwrap();
    overlay.write("items.json", b"{\"mod\": 1}").unwrap();
    overlay.write("assets/mod/new.json", b"{}").unwrap();
    assert_eq!(tree(&base), before, "a write reached the base");
    assert_eq!(
        fs::read(out.join("data/minecraft/tags/block/logs.json")).unwrap(),
        b"{\"values\": [\"mod:log\"]}"
    );
    assert_eq!(fs::read(out.join("items.json")).unwrap(), b"{\"mod\": 1}");
    edit_in_place(
        &base.join("assets/minecraft/lang/en_us.json"),
        b"{\"a\": 3}",
    );
    assert_eq!(
        fs::read(out.join("assets/minecraft/lang/en_us.json")).unwrap(),
        b"{\"a\": 3}"
    );
    // A rebuild starts by removing the overlay: that removes names, not the base's data.
    fs::remove_dir_all(&out).unwrap();
    assert_eq!(fs::read(base.join("items.json")).unwrap(), b"{}");
    let _ = fs::remove_dir_all(&scratch);
}

/// Where a hard link cannot be made (another volume, a filesystem without links) a file is copied, as it always was
/// on Windows; where it can, the file is linked.
#[test]
fn a_file_is_linked_or_else_copied() {
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-linkfile-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    let target = scratch.join("base.json");
    fs::write(&target, b"original").unwrap();

    let linked = scratch.join("linked.json");
    link_file(&target, &linked, |from, to| fs::hard_link(from, to)).unwrap();
    edit_in_place(&target, b"edited");
    assert_eq!(fs::read(&linked).unwrap(), b"edited", "not a link");

    let copied = scratch.join("copied.json");
    fs::write(&target, b"original").unwrap();
    link_file(&target, &copied, |_, _| {
        Err(std::io::Error::other("no links here"))
    })
    .unwrap();
    edit_in_place(&target, b"edited");
    assert_eq!(fs::read(&copied).unwrap(), b"original", "not a copy");

    // A target that is not there is an error whichever way the file is made.
    let missing = link_file(
        &scratch.join("missing.json"),
        &scratch.join("x.json"),
        |from, to| fs::hard_link(from, to),
    );
    assert!(missing.is_err());
    let _ = fs::remove_dir_all(&scratch);
}

/// A symlink in the base is copied through, as before; linked, the overlay's file would be a symlink too.
#[cfg(unix)]
#[test]
fn a_symlink_file_is_copied_through() {
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-symfile-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    fs::write(scratch.join("real.json"), b"real").unwrap();
    std::os::unix::fs::symlink("real.json", scratch.join("alias.json")).unwrap();
    let at = scratch.join("copy.json");
    link_file(&scratch.join("alias.json"), &at, |from, to| {
        fs::hard_link(from, to)
    })
    .unwrap();
    assert!(!fs::symlink_metadata(&at).unwrap().file_type().is_symlink());
    assert_eq!(fs::read(&at).unwrap(), b"real");
    let _ = fs::remove_dir_all(&scratch);
}

/// A declared sign text colour is the index into the dye table the signs draw with.
#[test]
fn sign_text_colors_are_the_signs_dye_ids() {
    let names: Vec<_> = sunrise_level::signs_data::DYE_TEXT
        .iter()
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(names, sunrise_mod_package::content::DYE_COLORS);
}

/// A content mod of recipes, a tag and a name, for vanilla's items: what
/// switches live.
fn glint_mod(dir: &Path) -> ContentMod {
    folder_mod(
        dir,
        &[
            ("sunrise-mod.toml", &manifest("glint", "")),
            (
                "data/glint/recipe/diamond_from_dirt.json",
                r#"{"type": "minecraft:crafting_shapeless", "category": "misc",
                    "ingredients": ["minecraft:dirt", "minecraft:dirt"],
                    "result": {"id": "minecraft:diamond"}}"#,
            ),
            (
                "data/c/tags/item/gems/glint.json",
                r#"{"values": ["minecraft:diamond"]}"#,
            ),
            (
                "assets/glint/lang/en_us.json",
                r#"{"glint.hello": "Hello"}"#,
            ),
        ],
    )
}

#[test]
fn only_assets_and_data_switch_live() {
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-live-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    assert_eq!(
        live_switch(&glint_mod(&scratch.join("glint")).content),
        Ok(())
    );
    let ruby = ruby_mod(&scratch.join("ruby"));
    assert_eq!(
        live_switch(&ruby.content),
        Err("sunrise.mods.switch.registries")
    );
    let effect = folder_mod(
        &scratch.join("fx"),
        &[
            ("sunrise-mod.toml", &manifest("fx", "")),
            (
                "data/fx/sunrise/mob_effect/glow.json",
                r#"{"category": "harmful", "color": 1}"#,
            ),
        ],
    );
    assert_eq!(
        live_switch(&effect.content),
        Err("sunrise.mods.switch.registries")
    );
    for (id, file, text) in [
        (
            "ores",
            "data/ores/neoforge/biome_modifier/add.json",
            r#"{"type": "neoforge:none"}"#,
        ),
        (
            "village",
            "data/minecraft/tags/worldgen/biome/has_structure/village_plains.json",
            r#"{"values": []}"#,
        ),
        (
            "feat",
            "data/feat/worldgen/placed_feature/rock.json",
            r#"{"feature": "minecraft:ore_iron", "placement": []}"#,
        ),
    ] {
        let m = folder_mod(
            &scratch.join(id),
            &[("sunrise-mod.toml", &manifest(id, "")), (file, text)],
        );
        assert_eq!(
            live_switch(&m.content),
            Err("sunrise.mods.switch.worldgen"),
            "{file}"
        );
    }
    fs::remove_dir_all(&scratch).unwrap();
}

/// On, off and on again: each root has what its set of mods has, a switch
/// never changes a root a game was given, and a set seen before reuses its
/// root without building it again.
#[test]
#[ignore = "daily"]
fn a_data_mod_loads_unloads_and_loads_again() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-reload-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let cache = scratch.join("cache");
    let glint = glint_mod(&scratch.join("glint"));
    let dirt = || Some(crate::items::ItemStack::new("minecraft:dirt", 1));
    let grid = [dirt(), dirt(), None, None];
    // What a world reads from a root: the item tags and the recipes.
    let tables = |root: &Path, mods: &[ContentMod]| {
        let resources = resolved_server_resources(
            &base,
            root.to_path_buf(),
            Path::new(""),
            mods,
            cache.clone(),
        )
        .unwrap();
        let files = content_files(&resources);
        let overlay = PackOverlay::new(&files);
        let assets = Assets::load(root).unwrap();
        let facts = BlockFacts::load(root, assets.registry).unwrap();
        let defaults = sunrise_world::item_defaults::read(root).unwrap();
        let items = Items::load(root, assets.registry, &facts, &overlay, &defaults).unwrap();
        let recipes = crate::crafting::Recipes::load(
            root,
            &items,
            &overlay,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let crafted = recipes
            .find(crate::test_resources::server_loop_context(), &grid, 2)
            .map(|(_, out)| out.item.to_string());
        let tagged = items.item_tag_order.get("c:gems/glint").cloned();
        (
            crafted,
            tagged,
            resources.source().contains("assets/glint/lang/en_us.json"),
        )
    };
    let loaded = (
        Some("minecraft:diamond".to_owned()),
        Some(vec!["minecraft:diamond".to_owned()]),
        true,
    );

    let started = std::time::Instant::now();
    let on =
        resource_root_fixture(&base, Path::new(""), std::slice::from_ref(&glint), &cache).unwrap();
    eprintln!("content root built in {} ms", started.elapsed().as_millis());
    assert_ne!(on, base);
    assert_eq!(tables(&on, std::slice::from_ref(&glint)), loaded);

    // Off: the base root, without any of it.
    let off = resource_root_fixture(&base, Path::new(""), &[], &cache).unwrap();
    assert_eq!(off, base);
    assert_eq!(tables(&off, &[]), (None, None, false));
    assert!(!on.join("assets/glint/lang/en_us.json").exists());
    assert!(
        resolved_server_resources(
            &base,
            on.clone(),
            Path::new(""),
            std::slice::from_ref(&glint),
            cache.clone()
        )
        .unwrap()
        .source()
        .contains("assets/glint/lang/en_us.json")
    );

    // On again: the same root, not built again.
    let sentinel = on.join("sentinel");
    fs::write(&sentinel, b"").unwrap();
    let started = std::time::Instant::now();
    let again =
        resource_root_fixture(&base, Path::new(""), std::slice::from_ref(&glint), &cache).unwrap();
    eprintln!(
        "content root reused in {} ms",
        started.elapsed().as_millis()
    );
    assert_eq!(again, on);
    assert!(sentinel.exists());
    assert_eq!(tables(&again, std::slice::from_ref(&glint)), loaded);

    // Another set of mods builds its own root beside it and leaves this one as it was.
    let ruby = ruby_mod(&scratch.join("ruby"));
    let package = sunrise_mod_package::Package::read(&scratch.join("glint")).unwrap();
    let glint_again = ContentMod {
        id: "glint".into(),
        jars: Vec::new(),
        content: Content::read(&scratch.join("glint"), &package.manifest).unwrap(),
    };
    let both_mods = [glint_again, ruby];
    let both = resource_root_fixture(&base, Path::new(""), &both_mods, &cache).unwrap();
    assert_ne!(both, on);
    assert!(sentinel.exists());
    assert_eq!(tables(&both, &both_mods).0, loaded.0);
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn roots_unused_for_a_day_are_removed() {
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-prune-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let roots = scratch.join("mod-content");
    let root = |name: &str, age_s: u64| {
        let dir = roots.join(name);
        fs::create_dir_all(&dir).unwrap();
        let stamp = dir.join(STAMP);
        fs::write(&stamp, b"x").unwrap();
        let file = fs::File::options().write(true).open(&stamp).unwrap();
        file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(age_s))
            .unwrap();
        dir
    };
    let current = root("00000000000000aa", 3 * 86400);
    let recent = root("00000000000000bb", 60);
    let stale = root("00000000000000cc", 2 * 86400);
    // Built two days ago but reused a minute ago: its `.used` beside it keeps it (#12403).
    let reused = root("00000000000000dd", 2 * 86400);
    fs::write(used_path(&reused), b"").unwrap();
    fs::write(used_path(&stale), b"").unwrap();
    let file = fs::File::options()
        .write(true)
        .open(used_path(&stale))
        .unwrap();
    file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 86400))
        .unwrap();
    drop(file);
    // Not a root's name: never touched while the folder has no root of the old layout.
    let other = roots.join("notes");
    fs::create_dir_all(&other).unwrap();
    prune_roots(&roots, &current, KEEP_ROOTS);
    assert!(current.exists() && recent.exists() && other.exists() && reused.exists());
    assert!(!stale.exists() && !used_path(&stale).exists());
    // An older game's single root, built straight in the folder, goes.
    fs::write(roots.join(STAMP), b"old").unwrap();
    fs::create_dir_all(roots.join("assets/minecraft")).unwrap();
    prune_roots(&roots, &current, KEEP_ROOTS);
    assert!(!roots.join(STAMP).exists() && !roots.join("assets").exists());
    assert!(current.exists() && recent.exists());
    fs::remove_dir_all(&scratch).unwrap();
}

#[path = "mod_content_load_conditions_tests.rs"]
mod registered_conditions_tests;

fn structure_modifier_base(tag: &str) -> PathBuf {
    let base = biome_base(tag);
    let entries = BTreeMap::from([
        (
            "minecraft:hut",
            r#"{"spawn_overrides":{"monster":{"bounding_box":"full","spawns":[{"type":"minecraft:skeleton","count":1,"weight":10}]}}}"#,
        ),
        ("minecraft:untouched", r#"{ "spawn_overrides": {} }"#),
        ("zinc:nested/tower", r#"{"spawn_overrides":{}}"#),
    ]);
    fs::write(
        base.join("worldgen/structures.json"),
        serde_json::to_vec(&entries).unwrap(),
    )
    .unwrap();
    for (id, text) in entries {
        let (ns, path) = id.split_once(':').unwrap();
        let file = base.join(format!("data/{ns}/worldgen/structure/{path}.json"));
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }
    base
}

fn structure_modifier_mod(dir: &Path, modifiers: &[(&str, &str)]) -> ContentMod {
    let mut files = vec![("sunrise-mod.toml".to_owned(), manifest("zinc", ""))];
    for (name, text) in modifiers {
        files.push((
            format!("data/zinc/neoforge/structure_modifier/{name}.json"),
            text.to_string(),
        ));
    }
    files.push((
        "data/zinc/tags/worldgen/structure/targets.json".into(),
        r#"{"values":["minecraft:hut","zinc:nested/tower"]}"#.into(),
    ));
    folder_mod(
        dir,
        &files
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect::<Vec<_>>(),
    )
}

#[test]
fn structure_modifiers_apply_phases_and_reload_without_stale_spawns() {
    use crate::mobs::spawn_selection::StructureSpawns;
    let base = structure_modifier_base("structure-modifiers-reload");
    let scratch = base.parent().unwrap();
    let m = structure_modifier_mod(
        &scratch.join("zinc"),
        &[
            (
                "a_remove",
                r##"{"type":"neoforge:remove_spawns","structures":"#zinc:targets","entity_types":"#minecraft:skeletons"}"##,
            ),
            (
                "b_add",
                r##"{"type":"neoforge:add_spawns","structures":"#zinc:targets","spawners":[{"type":"stray","weight":30,"minCount":1,"maxCount":2},{"type":"zombie","weight":20,"minCount":2,"maxCount":5}]}"##,
            ),
            (
                "c_add",
                r#"{"type":"neoforge:add_spawns","structures":["minecraft:hut","zinc:nested/tower"],"spawners":{"type":"rabbit","weight":12,"minCount":3,"maxCount":3}}"#,
            ),
            (
                "d_add",
                r#"{"type":"neoforge:add_spawns","structures":["minecraft:hut","zinc:nested/tower"],"spawners":{"type":"zombie","weight":7,"minCount":1,"maxCount":1}}"#,
            ),
        ],
    );
    let out = scratch.join("out");
    let warnings = build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let loaded = StructureSpawns::load(&out);
    for id in ["minecraft:hut", "zinc:nested/tower"] {
        let monster = loaded.get(id, "monster").unwrap();
        assert_eq!(monster.piece, id == "zinc:nested/tower");
        assert_eq!(
            monster.spawns,
            vec![
                crate::biome::SpawnEntry {
                    entity: "minecraft:zombie".into(),
                    weight: 20,
                    min: 2,
                    max: 5,
                    constant: false
                },
                crate::biome::SpawnEntry {
                    entity: "minecraft:zombie".into(),
                    weight: 7,
                    min: 1,
                    max: 1,
                    constant: true
                }
            ]
        );
        assert_eq!(
            loaded.get(id, "creature").unwrap().spawns,
            vec![crate::biome::SpawnEntry {
                entity: "minecraft:rabbit".into(),
                weight: 12,
                min: 3,
                max: 3,
                constant: true
            }]
        );
    }
    assert!(loaded.get("minecraft:untouched", "monster").is_none());
    let bundle: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(out.join("worldgen/structures.json")).unwrap()).unwrap();
    let canonical: serde_json::Value = serde_json::from_str(&bundle["zinc:nested/tower"]).unwrap();
    let raw: serde_json::Value = serde_json::from_slice(
        &fs::read(out.join("data/zinc/worldgen/structure/nested/tower.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(canonical, raw);
    assert_eq!(
        fs::read(out.join("data/minecraft/worldgen/structure/untouched.json")).unwrap(),
        fs::read(base.join("data/minecraft/worldgen/structure/untouched.json")).unwrap()
    );
    // Keep the mod enabled while removing its monster modifiers. Rebuilding
    // the same destination must start from vanilla, not the previous overlay.
    let remaining = structure_modifier_mod(
        &scratch.join("remaining"),
        &[(
            "rabbits",
            r#"{"type":"neoforge:add_spawns","structures":"minecraft:hut","spawners":{"type":"rabbit","weight":8,"minCount":1,"maxCount":1}}"#,
        )],
    );
    build_fixture(&base, Path::new(""), &[remaining], &out).unwrap();
    let partial = StructureSpawns::load(&out);
    let monsters = &partial.get("minecraft:hut", "monster").unwrap().spawns;
    assert_eq!(monsters.len(), 1);
    assert_eq!(monsters[0].entity, "minecraft:skeleton");
    assert!(partial.get("zinc:nested/tower", "monster").is_none());
    let rabbits = &partial.get("minecraft:hut", "creature").unwrap().spawns;
    assert_eq!(rabbits.len(), 1);
    assert_eq!(rabbits[0].weight, 8);
    // Finally remove the remaining modifier as well.
    build_fixture(&base, Path::new(""), &[], &out).unwrap();
    let reloaded = StructureSpawns::load(&out);
    assert!(reloaded.get("zinc:nested/tower", "monster").is_none());
    assert!(reloaded.get("minecraft:hut", "creature").is_none());
    assert_eq!(
        reloaded.get("minecraft:hut", "monster").unwrap().spawns[0].entity,
        "minecraft:skeleton"
    );
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn structure_modifiers_remove_keeps_empty_override_and_clear_removes_it() {
    use crate::mobs::spawn_selection::StructureSpawns;
    let base = structure_modifier_base("structure-modifiers-clear");
    let scratch = base.parent().unwrap();
    let out = scratch.join("out");
    let m = structure_modifier_mod(
        &scratch.join("remove"),
        &[(
            "remove",
            r#"{"type":"neoforge:remove_spawns","structures":"minecraft:hut","entity_types":"skeleton"}"#,
        )],
    );
    build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    assert!(
        StructureSpawns::load(&out)
            .get("minecraft:hut", "monster")
            .unwrap()
            .spawns
            .is_empty()
    );
    for (name, categories) in [
        ("one", r#", "categories":"monster""#),
        ("list", r#", "categories":["monster","creature"]"#),
        ("all", ""),
    ] {
        let text = format!(
            r#"{{"type":"neoforge:clear_spawns","structures":"minecraft:hut"{categories}}}"#
        );
        let m = structure_modifier_mod(
            &scratch.join(name),
            &[
                ("clear", &text),
                (
                    "rabbit",
                    r#"{"type":"neoforge:add_spawns","structures":"minecraft:hut","spawners":{"type":"rabbit","weight":4,"minCount":1,"maxCount":1}}"#,
                ),
            ],
        );
        build_fixture(&base, Path::new(""), &[m], &out).unwrap();
        assert!(
            StructureSpawns::load(&out)
                .get("minecraft:hut", "monster")
                .is_none()
        );
        assert_eq!(
            StructureSpawns::load(&out)
                .get("minecraft:hut", "creature")
                .is_some(),
            name == "one"
        );
    }
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn structure_modifiers_unknown_targets_and_types_warn_and_preserve_base() {
    let base = structure_modifier_base("structure-modifiers-unknown");
    let scratch = base.parent().unwrap();
    let out = scratch.join("out");
    let m = structure_modifier_mod(
        &scratch.join("zinc"),
        &[
            ("none", r#"{"type":"neoforge:none"}"#),
            ("custom", r#"{"type":"zinc:custom"}"#),
            (
                "missing",
                r#"{"type":"neoforge:remove_spawns","structures":"zinc:missing","entity_types":"skeleton"}"#,
            ),
        ],
    );
    let warnings = build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("unknown structure zinc:missing"))
    );
    assert_eq!(
        fs::read(out.join("worldgen/structures.json")).unwrap(),
        fs::read(base.join("worldgen/structures.json")).unwrap()
    );
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn structure_modifiers_follow_mod_order_and_use_overlaid_definitions() {
    use crate::mobs::spawn_selection::StructureSpawns;
    let base = structure_modifier_base("structure-modifiers-overlay");
    let scratch = base.parent().unwrap();
    let first = folder_mod(
        &scratch.join("first"),
        &[
            ("sunrise-mod.toml", &manifest("zinc", "")),
            (
                "data/zinc/worldgen/structure/nested/tower.json",
                r#"{"spawn_overrides":{"monster":{"bounding_box":"piece","spawns":[{"type":"minecraft:skeleton","count":2,"weight":11}]}}}"#,
            ),
            (
                "data/zinc/neoforge/structure_modifier/add.json",
                r#"{"type":"neoforge:add_spawns","structures":"zinc:nested/tower","spawners":{"type":"zombie","weight":5,"minCount":1,"maxCount":1}}"#,
            ),
        ],
    );
    let second = folder_mod(
        &scratch.join("second"),
        &[
            ("sunrise-mod.toml", &manifest("post", "")),
            (
                "data/post/neoforge/structure_modifier/add.json",
                r#"{"type":"neoforge:add_spawns","structures":"zinc:nested/tower","spawners":{"type":"stray","weight":9,"minCount":1,"maxCount":1}}"#,
            ),
        ],
    );
    let out = scratch.join("out");
    let warnings = build_fixture(&base, Path::new(""), &[first, second], &out).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let loaded = StructureSpawns::load(&out);
    let monster = loaded.get("zinc:nested/tower", "monster").unwrap();
    assert!(monster.piece);
    assert_eq!(
        monster
            .spawns
            .iter()
            .map(|s| (s.entity.as_str(), s.weight, s.min))
            .collect::<Vec<_>>(),
        vec![
            ("minecraft:skeleton", 11, 2),
            ("minecraft:zombie", 5, 1),
            ("minecraft:stray", 9, 1)
        ]
    );
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn structure_modifiers_reject_malformed_supported_data() {
    for text in [
        r#"{"type":"neoforge:add_spawns","structures":"minecraft:hut","spawners":{"type":"zombie","weight":1,"minCount":4,"maxCount":1}}"#,
        r#"{"type":"neoforge:add_spawns","structures":"minecraft:hut"}"#,
        r#"{"type":"neoforge:clear_spawns","structures":"minecraft:hut","categories":"bogus"}"#,
        r#"{"type":"neoforge:remove_spawns","entity_types":"zombie"}"#,
    ] {
        let error = structures::Modifier::read(
            "zinc",
            "data/zinc/neoforge/structure_modifier/bad.json",
            text,
            &mut Vec::new(),
        )
        .err()
        .expect("malformed modifier must fail");
        assert!(
            error
                .to_string()
                .contains("mod zinc: data/zinc/neoforge/structure_modifier/bad.json"),
            "{error}"
        );
    }
}

#[test]
fn structure_modifiers_and_biome_modifier_reject_out_of_range_spawns() {
    for spawner in [
        r#"{"type":"zombie","weight":1,"minCount":1,"maxCount":2147483648}"#,
        r#"{"type":"zombie","weight":2147483648,"minCount":1,"maxCount":1}"#,
        r#"{"type":"zombie","weight":1,"minCount":4294967297,"maxCount":4294967297}"#,
    ] {
        let structure = format!(
            r#"{{"type":"neoforge:add_spawns","structures":"minecraft:hut","spawners":{spawner}}}"#
        );
        assert!(
            structures::Modifier::read("zinc", "bad.json", &structure, &mut Vec::new()).is_err()
        );
        let biome = format!(
            r#"{{"type":"neoforge:add_spawns","biomes":"minecraft:plains","spawners":{spawner}}}"#
        );
        assert!(
            biomes::Modifier::read("zinc", "bad.json", &biome, &|_| false, &mut Vec::new())
                .is_err()
        );
    }
}

#[test]
fn structure_modifiers_boundary_count_samples_through_reader() {
    use crate::mobs::spawn_selection::{StructureSpawns, sample_count};
    use sunrise_core::random::LegacyRandom;

    let base = structure_modifier_base("structure-modifiers-boundary");
    let scratch = base.parent().unwrap();
    let m = structure_modifier_mod(
        &scratch.join("zinc"),
        &[(
            "boundary",
            r#"{"type":"neoforge:add_spawns","structures":"minecraft:hut","spawners":{"type":"zombie","weight":2147483647,"minCount":1,"maxCount":2147483647}}"#,
        )],
    );
    let out = scratch.join("out");
    assert!(
        build_fixture(&base, Path::new(""), &[m], &out)
            .unwrap()
            .is_empty()
    );
    let loaded = StructureSpawns::load(&out);
    let entry = loaded
        .get("minecraft:hut", "monster")
        .unwrap()
        .spawns
        .iter()
        .find(|entry| entry.weight == 2147483647)
        .unwrap();
    assert_eq!((entry.min, entry.max), (1, 2147483647));
    let mut random = LegacyRandom::new(1);
    assert_eq!(sample_count(entry, &mut random), 1569548986);
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn structure_modifiers_evaluate_conditions_before_required_fields() {
    let base = structure_modifier_base("structure-modifiers-conditions");
    let scratch = base.parent().unwrap();
    let m = structure_modifier_mod(
        &scratch.join("zinc"),
        &[
            (
                "disabled",
                r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"absent"}],"type":"neoforge:add_spawns"}"#,
            ),
            (
                "enabled",
                r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"zinc"}],"type":"neoforge:add_spawns","structures":"minecraft:hut","spawners":{"type":"rabbit","weight":3,"minCount":1,"maxCount":1}}"#,
            ),
        ],
    );
    let out = scratch.join("out");
    assert!(
        build_fixture(&base, Path::new(""), &[m], &out)
            .unwrap()
            .is_empty()
    );
    let loaded = crate::mobs::spawn_selection::StructureSpawns::load(&out);
    assert_eq!(
        loaded.get("minecraft:hut", "creature").unwrap().spawns[0].entity,
        "minecraft:rabbit"
    );
    fs::remove_dir_all(scratch).unwrap();
}

/// Exercise the production resource-root cache and the spawn consumer against
/// prepared vanilla data, including switching back to the unmodified root.
#[test]
#[ignore = "daily"]
fn structure_modifiers_prepared_root_reloads_spawn_overrides() {
    use crate::mobs::spawn_selection::StructureSpawns;
    let Some(base) = prepared() else {
        return;
    };
    let _assets =
        crate::assets::Assets::load(&base).expect("prepared resources must match this build");
    let scratch = std::env::temp_dir().join(format!(
        "sunrise-structure-modifier-prepared-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("zinc"),
        &[
            ("sunrise-mod.toml", &manifest("zinc", "")),
            (
                "data/zinc/neoforge/structure_modifier/rabbits.json",
                r#"{"type":"neoforge:add_spawns","structures":"minecraft:desert_pyramid","spawners":{"type":"minecraft:rabbit","weight":17,"minCount":2,"maxCount":2}}"#,
            ),
        ],
    );
    let vanilla = StructureSpawns::load(&base);
    assert!(
        vanilla
            .get("minecraft:desert_pyramid", "creature")
            .is_none()
    );
    let cache = scratch.join("cache");
    let root = resource_root_fixture(&base, Path::new(""), &[m], &cache).unwrap();
    let loaded = StructureSpawns::load(&root);
    let rabbits = loaded.get("minecraft:desert_pyramid", "creature").unwrap();
    assert!(rabbits.piece);
    assert_eq!(
        rabbits.spawns,
        vec![crate::biome::SpawnEntry {
            entity: "minecraft:rabbit".into(),
            weight: 17,
            min: 2,
            max: 2,
            constant: true,
        }]
    );
    let restored = resource_root_fixture(&base, Path::new(""), &[], &cache).unwrap();
    assert!(
        StructureSpawns::load(&restored)
            .get("minecraft:desert_pyramid", "creature")
            .is_none()
    );
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn structure_modifiers_bundle_spawns_without_modifier_preserve_individual_files() {
    use crate::mobs::spawn_selection::StructureSpawns;
    let base = structure_modifier_base("structure-modifiers-bundle-only");
    let out = base.parent().unwrap().join("out");
    let m = folder_mod(
        &base.parent().unwrap().join("zinc"),
        &[
            ("sunrise-mod.toml", &manifest("zinc", "")),
            (
                "data/zinc/worldgen/structure/tower.json",
                r#"{"spawn_overrides":{"monster":{"bounding_box":"piece","spawns":[{"type":"minecraft:zombie","weight":17,"count":{"type":"minecraft:uniform","min_inclusive":2,"max_inclusive":4}}]}}}"#,
            ),
        ],
    );
    assert!(
        build_fixture(&base, Path::new(""), &[m], &out)
            .unwrap()
            .is_empty()
    );
    let loaded = StructureSpawns::load(&out);
    let tower = loaded.get("zinc:tower", "monster").unwrap();
    assert!(tower.piece);
    assert_eq!(
        tower.spawns,
        vec![crate::biome::SpawnEntry {
            entity: "minecraft:zombie".into(),
            weight: 17,
            min: 2,
            max: 4,
            constant: false,
        }]
    );
    assert_eq!(
        loaded.get("minecraft:hut", "monster").unwrap().spawns[0].weight,
        10
    );
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

/// The legacy solid bit of declared blocks in the generated behaviours table, through `Plan::new`
/// and `Plan::json` over a hand-made base: the final flag integers, not only the helper's answer.
#[test]
fn declared_solid_reaches_the_behaviour_table() {
    // Templates, one state each but `axis` (two): `plain` is not solid, `keeps` is, and the two
    // forcing ones carry the opposite bit to what they force. Every row has the marker bit 64.
    let facts = r#"{"states":[
      ["minecraft:plain",false,false,"",{"id":"minecraft:plain"}],
      ["minecraft:forced_on",false,false,"",{"id":"minecraft:forced_on"}],
      ["minecraft:forced_off",false,false,"",{"id":"minecraft:forced_off"}],
      ["minecraft:keeps",false,false,"",{"id":"minecraft:keeps"}],
      ["minecraft:axis",false,false,"",{"id":"minecraft:axis","properties":{"axis":"x"}}],
      ["minecraft:axis",false,false,"",{"id":"minecraft:axis","properties":{"axis":"y"}}]],
      "defaults":{"minecraft:plain":0,"minecraft:forced_on":1,"minecraft:forced_off":2,
        "minecraft:keeps":3,"minecraft:axis":4}}"#;
    let (base, _) = block_base_of("declared-solid", &[], facts);
    let template_flags = [64, 64, 64 | 1, 64 | 1, 64, 64 | 1];
    let row = |name: &str, flags: i32| format!(r#"["{name}",{flags},0,0,false,{{"id":"{name}"}}]"#);
    let rows: Vec<String> = ["plain", "forced_on", "forced_off", "keeps", "axis", "axis"]
        .iter()
        .zip(template_flags)
        .map(|(name, flags)| row(&format!("minecraft:{name}"), flags))
        .collect();
    fs::write(
        base.join("block-behaviours.json"),
        format!(
            r#"{{"blocks":[],"items":{{}},"states":[{}]}}"#,
            rows.join(",")
        ),
    )
    .unwrap();
    // Block definition flags: bit 0 forces solid on, bit 1 forces it off.
    let blocks = Compound::from(
        [
            ("minecraft:plain", 0),
            ("minecraft:forced_on", 1),
            ("minecraft:forced_off", 2),
            ("minecraft:keeps", 0),
            ("minecraft:axis", 0),
        ]
        .map(|(name, flags)| (JavaString::from(name), Tag::Byte(flags))),
    );
    fs::create_dir_all(base.join("worldgen/blocks")).unwrap();
    fs::write(
        base.join("worldgen/blocks/block-properties.nbt"),
        nbt::write_named(
            &nbt::NamedTag {
                name: JavaString::from(""),
                tag: Tag::Compound(Compound::from([(
                    JavaString::from("blocks"),
                    Tag::Compound(blocks),
                )])),
            },
            Limits::tables(),
        )
        .unwrap(),
    )
    .unwrap();
    let full = "[[0,0,0,1,1,1]]";
    let thin = "[[0,0,0,1,0.1,1]]";
    let declare = |template: &str, shapes: &str, collision: &str| {
        format!(
            r#"{{"template":"minecraft:{template}","item":false{shapes}{collision}}}"#,
            shapes = if shapes.is_empty() {
                String::new()
            } else {
                format!(r#","shapes":{shapes}"#)
            },
            collision = if collision.is_empty() {
                String::new()
            } else {
                format!(r#","collision_shapes":{collision}"#)
            },
        )
    };
    let default = |boxes: &str| format!(r#"{{"default":{boxes}}}"#);
    let files = [
        // Collision wins over a full outline.
        (
            "collision_wins",
            declare("plain", &default(full), &default(thin)),
        ),
        ("outline_only", declare("plain", &default(full), "")),
        ("no_shape", declare("keeps", "", "")),
        ("force_on", declare("forced_on", &default(thin), "")),
        ("force_off", declare("forced_off", &default(full), "")),
        ("cleared", declare("keeps", &default(thin), "")),
        (
            "per_state",
            declare(
                "axis",
                &format!(r#"{{"properties":["axis"],"values":{{"x":{full}}},"default":{thin}}}"#),
                "",
            ),
        ),
    ];
    let manifest = manifest("testmod", "");
    let paths: Vec<String> = files
        .iter()
        .map(|(name, _)| format!("data/testmod/sunrise/block/{name}.json"))
        .collect();
    let mut entries: Vec<(&str, &str)> = vec![("sunrise-mod.toml", &manifest)];
    entries.extend(
        paths
            .iter()
            .zip(&files)
            .map(|(path, (_, body))| (path.as_str(), body.as_str())),
    );
    let mods = vec![folder_mod(&base.parent().unwrap().join("mod"), &entries)];
    let plan = Plan::new(&base, &mods).unwrap();
    let mut doc: J = read_json(&base.join("block-behaviours.json")).unwrap();
    plan.json("block-behaviours.json", &mut doc).unwrap();
    let Some(J::Arr(table)) = doc.get("states") else {
        panic!("states")
    };
    let flags = |index: usize| match &table[index] {
        J::Arr(row) => match &row[1] {
            J::Num(n) => n.as_i64().unwrap(),
            other => panic!("flags {other:?}"),
        },
        other => panic!("row {other:?}"),
    };
    // Vanilla rows are as they were.
    for (index, expected) in template_flags.into_iter().enumerate() {
        assert_eq!(flags(index), i64::from(expected), "template row {index}");
    }
    let first = |id: &str| {
        plan.blocks
            .iter()
            .find(|b| b.decl.id == id)
            .unwrap_or_else(|| panic!("{id}"))
            .rename
            .first as usize
    };
    // The marker bit stays in every declared row; only bit 0 is recomputed.
    for (id, solid) in [
        ("testmod:collision_wins", 0),
        ("testmod:outline_only", 1),
        ("testmod:no_shape", 1),
        ("testmod:force_on", 1),
        ("testmod:force_off", 0),
        ("testmod:cleared", 0),
    ] {
        assert_eq!(flags(first(id)), 64 | solid, "{id}");
    }
    // One row per state, in state order: x is the full shape, y the thin one.
    let axis = first("testmod:per_state");
    assert_eq!((flags(axis), flags(axis + 1)), (64 | 1, 64));
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

#[test]
fn a_declared_shape_decides_the_legacy_solid_bit() {
    let boxed =
        |top: f64| VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, top, 1.0))).unwrap();
    // The other flag bits stay as the template has them.
    let flags = 0b10;
    // A full cube and a slab (average size 0.83) are solid, a flat carpet-thin box is not, and a
    // box a block and a half tall is.
    assert_eq!(with_solid_bit(flags, None, &boxed(1.0)), 0b11);
    assert_eq!(with_solid_bit(flags, None, &boxed(0.5)), 0b11);
    assert_eq!(with_solid_bit(flags | 1, None, &boxed(0.1)), 0b10);
    assert_eq!(with_solid_bit(flags, None, &boxed(1.5)), 0b11);
    // A thin tall pole is solid by height alone; a thin short one is not.
    let pole = |top: f64| {
        VoxelShape::from_box(Aabb::new(
            Vec3::new(0.4, 0.0, 0.4),
            Vec3::new(0.6, top, 0.6),
        ))
        .unwrap()
    };
    assert_eq!(with_solid_bit(flags, None, &pole(1.0)), 0b11);
    assert_eq!(with_solid_bit(flags, None, &pole(0.5)), 0b10);
    // A template that forces solidity keeps its answer whatever the shape.
    assert_eq!(with_solid_bit(flags, Some(true), &boxed(0.1)), 0b11);
    assert_eq!(with_solid_bit(flags | 1, Some(false), &boxed(1.0)), 0b10);
}

#[test]
fn context_only_property_outline_moves_without_moving_the_fallback() {
    use sunrise_world::{
        collision::{CachedCollisionRegistry, ShapeChannel},
        collision_context::EntityContextKind,
        registry::{BlockRegistry, BlockStateId},
    };
    let declaration = r#"{"template":"minecraft:test","item":false,"properties":{"offset":"XZ"},
      "outline_context_shapes":{"living":{"properties":["axis"],
        "values":{"x":[[0.25,0,0.25,0.75,1,0.75]]},"default":[]}}}"#;
    let (base, mods) = block_base("offset-context-variant", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let mut facts: J = read_json(&base.join("facts.json")).unwrap();
    plan.json("facts.json", &mut facts).unwrap();
    let registry =
        BlockRegistry::from_facts_json(serde_json::to_vec(&facts).unwrap().as_slice()).unwrap();
    let mut tag = Tag::compound([
        ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
        (
            "states",
            Tag::List(vec![test_state_row("x"), test_state_row("y")]),
        ),
        (
            "shapes",
            Tag::List(vec![encode_shape(&VoxelShape::block())]),
        ),
        ("cached_shapes", Tag::IntArray(vec![0, 0])),
    ]);
    plan.nbt(&mut tag);
    plan.declared_shapes(&mut tag, true).unwrap();
    let cached = CachedCollisionRegistry::from_nbt(&tag, &registry).unwrap();
    for (id, axis) in [(2, "x"), (3, "y")] {
        let state = BlockStateId(id);
        assert_eq!(registry.state(state).unwrap().properties["axis"], axis);
        let outline = cached
            .context_outline_shape(state, EntityContextKind::Living)
            .unwrap();
        let pos = [1, 64, 2];
        let outline = cached.positioned_shape(state, ShapeChannel::ContextOutline, pos, outline);
        if axis == "x" {
            assert_eq!(outline.min(Axis::X), 0.13333334028720856);
            assert_eq!(outline.min(Axis::Z), 0.06666667014360428);
        } else {
            assert!(outline.is_empty());
        }
        assert!(
            cached
                .context_outline_shape(state, EntityContextKind::Other)
                .is_none()
        );
        let fallback = cached.cached_shape(state).unwrap().unwrap();
        assert_eq!(
            fallback.bounds(),
            Some(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0)))
        );
    }
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

#[test]
fn declared_offsets_resolve_modes_and_publish_only_declared_shape_channels() {
    for (label, properties, expected) in [
        ("none", r#""offset":"NONE""#, None),
        (
            "xz",
            r#""offset":"XZ","max_vertical_offset":0.2"#,
            Some((0.25f32, 0.0f32)),
        ),
        ("xyz", r#""offset":"XYZ""#, Some((0.25, 0.2))),
        (
            "clamped",
            r#""offset":"XYZ","max_horizontal_offset":0.125,"max_vertical_offset":0.1"#,
            Some((0.125, 0.1)),
        ),
        ("inherited", "", Some((0.25, 0.0))),
        (
            "inherited-xz",
            r#""max_horizontal_offset":0.125,"max_vertical_offset":0.2"#,
            Some((0.125, 0.0)),
        ),
    ] {
        let declaration = format!(
            r#"{{"template":"minecraft:test","item":false,"properties":{{{properties}}},
          "shapes":{{"default":[[0,0,0,0.5,1,0.5]]}},
          "interaction_shapes":{{"default":[[0,0,0,1,1,1]]}}}}"#
        );
        let (base, mods) = block_base(&format!("offset-{label}"), &[&declaration]);
        let row = r#"{"offset":true,"outline_offset":true}"#;
        fs::write(
            base.join("gameplay-facts.json"),
            format!(r#"{{"states":[{row},{row}]}}"#),
        )
        .unwrap();
        let plan = Plan::new(&base, &mods).unwrap();
        assert_eq!(plan.blocks[0].offset_limits, vec![expected; 2]);
        let mut gameplay: J =
            serde_json::from_str(&format!(r#"{{"states":[{row},{row}]}}"#)).unwrap();
        plan.json("gameplay-facts.json", &mut gameplay).unwrap();
        let Some(J::Arr(rows)) = gameplay.get("states") else {
            panic!("states")
        };
        for row in &rows[2..] {
            assert_eq!(row.get("offset"), Some(&J::Bool(expected.is_some())));
            assert_eq!(
                row.get("outline_offset"),
                Some(&J::Bool(expected.is_some()))
            );
            if let Some((h, v)) = expected {
                assert_eq!(
                    row.get("offset_limits"),
                    Some(&J::Arr(vec![J::f64(h as f64), J::f64(v as f64)]))
                );
            }
        }
        let mut tag = Tag::compound([
            (
                "states",
                Tag::List(vec![test_state_row("x"), test_state_row("y")]),
            ),
            ("shapes", Tag::List(vec![])),
            ("cached_shapes", Tag::IntArray(vec![-1, -1])),
        ]);
        plan.nbt(&mut tag);
        plan.declared_shapes(&mut tag, true).unwrap();
        if expected.is_some() {
            let Some(Tag::List(offsets)) = tag.get("shape_offsets") else {
                panic!("offsets")
            };
            assert_eq!(offsets.len(), 2);
            // Default collision plus interaction; no contextual channels were declared.
            assert_eq!(offsets[0].get("channels"), Some(&Tag::Byte(9)));
        } else {
            assert!(tag.get("shape_offsets").is_none());
        }
        fs::remove_dir_all(base.parent().unwrap()).unwrap();
    }
}

#[test]
fn plain_block_properties_replace_template_facts_and_face_rule() {
    for value in [true, false] {
        let skip = if value {
            r#"[{"neighbor":"self"}]"#
        } else {
            "[]"
        };
        let declaration = format!(
            r#"{{"template":"minecraft:test","item":false,"properties":{{"instrument":"CUSTOM_HEAD","ignited_by_lava":{value},"replaceable":{value},"skip_rendering":{skip}}}}}"#
        );
        let (base, mods) = block_base(&format!("plain-properties-{value}"), &[&declaration]);
        let plan = Plan::new(&base, &mods).unwrap();
        let row = format!(
            r#"{{"ignited_by_lava":{},"replaceable":{}}}"#,
            !value, !value
        );
        let mut gameplay: J =
            serde_json::from_str(&format!(r#"{{"states":[{row},{row}]}}"#)).unwrap();
        plan.json("gameplay-facts.json", &mut gameplay).unwrap();
        let Some(J::Arr(rows)) = gameplay.get("states") else {
            panic!("states")
        };
        for row in &rows[2..] {
            assert_eq!(row.get("ignited_by_lava"), Some(&J::Bool(value)));
            assert_eq!(row.get("replaceable"), Some(&J::Bool(value)));
        }
        let template_flags = if value { 16 } else { 16 | 4 };
        let mut properties = Tag::Compound(Compound::from([(
            "blocks".into(),
            Tag::Compound(Compound::from([(
                "minecraft:test".into(),
                Tag::Byte(template_flags),
            )])),
        )]));
        plan.nbt(&mut properties);
        assert_eq!(
            properties.get("blocks").unwrap().get("testmod:test"),
            Some(&Tag::Byte(if value { 16 | 4 } else { 16 }))
        );
        let mut definitions: J =
            serde_json::from_str(r#"{"minecraft:test":{"rule":"bars"}}"#).unwrap();
        plan.json("render-definitions.json", &mut definitions)
            .unwrap();
        let expected_rules = if value {
            J::Arr(vec![J::Obj(vec![(
                "neighbor".into(),
                J::Str("self".into()),
            )])])
        } else {
            J::Arr(Vec::new())
        };
        assert_eq!(
            definitions.at("testmod:test/skip_rendering"),
            Some(&mut expected_rules.clone())
        );
        let mut behaviour: J = serde_json::from_str(
            r#"{"states":[[0,0,0,0,0,0,0,0,"HARP"],[0,0,0,0,0,0,0,0,"HARP"]],"blocks":[],"items":{}}"#,
        )
        .unwrap();
        plan.json("block-behaviours.json", &mut behaviour).unwrap();
        let Some(J::Arr(rows)) = behaviour.get("states") else {
            panic!("states")
        };
        for row in &rows[2..] {
            let J::Arr(row) = row else { panic!("row") };
            assert_eq!(row[8], J::Str("CUSTOM_HEAD".into()));
        }
        fs::remove_dir_all(base.parent().unwrap()).unwrap();
    }
}

#[test]
fn accept_declared_refuses_host_duplicates_of_served_content_and_keeps_served_remainders() {
    use sunrise_mod_package::content::{Block, Content, Item};
    let block = |id: &str, item: bool| Block {
        id: id.into(),
        template: "minecraft:stone".into(),
        item,
        ..Block::default()
    };
    let item = |id: &str, remainder: Option<&str>| Item {
        id: id.into(),
        template: "minecraft:stick".into(),
        crafting_remainder: remainder.map(str::to_owned),
        ..Item::default()
    };
    let mut runes = Content::declared("runes");
    runes.blocks.push(block("runes:altar", true));
    runes.blocks.push(block("runes:crop", false));
    runes.items.push(item("runes:stone", None));
    runes
        .items
        .push(item("runes:soup", Some("runes:extra_item")));
    runes
        .items
        .push(item("runes:ritual", Some("runes:extra_block")));
    let mut plain = Content::declared("plain");
    plain.items.push(item("plain:spoon", None));

    let host_manifest = sunrise_mod_package::Manifest::parse(&format!(
        "id='horizon_java'\nname='Horizon'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nserves_content=true\n",
        "a".repeat(40),
    ))
    .unwrap();
    let runes_manifest = sunrise_mod_package::Manifest::parse(
        "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\nserved_by='horizon_java'\n",
    )
    .unwrap();
    let plain_manifest = sunrise_mod_package::Manifest::parse(
        "id='plain'\nname='Plain'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n",
    )
    .unwrap();
    let manifests = [&host_manifest, &runes_manifest, &plain_manifest];
    let known = BTreeSet::new();

    // Non-colliding host declarations in a served namespace succeed and coexist without clash.
    // Both directions of cross-layer crafting_remainder work (served -> host item & block item,
    // and host -> served item & block item).
    let mut mods = vec![
        ContentMod {
            id: "runes".into(),
            content: runes,
            jars: Vec::new(),
        },
        ContentMod {
            id: "plain".into(),
            content: plain,
            jars: Vec::new(),
        },
    ];
    let valid = BTreeMap::from([(
        "horizon_java",
        DeclaredContent {
            blocks: vec![block("runes:extra_block", true).validate_local().unwrap()],
            items: vec![
                item("runes:extra_item", None).validate_local().unwrap(),
                // Host item using a served content item as its crafting_remainder.
                item("runes:filled_stone", Some("runes:stone"))
                    .validate_local()
                    .unwrap(),
                // Host item using a served content item-bearing block as its crafting_remainder.
                item("horizon_java:altar_kit", Some("runes:altar"))
                    .validate_local()
                    .unwrap(),
                // Same ID as a block with `item: false` is allowed.
                item("runes:crop", None).validate_local().unwrap(),
            ],
            ..Default::default()
        },
    )]);
    let mut errors = Vec::new();
    accept_declared(&mut mods, &valid, &manifests, &known, &mut errors).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(mods.len(), 3);
    assert_eq!(mods[2].id, "horizon_java");
    // Served and host items with valid cross-layer remainders (both items and block items) survive.
    let base = tempfile::tempdir().unwrap();
    std::fs::write(base.path().join("items.json"), "{}").unwrap();
    assert!(
        drop_dangling_remainders(base.path(), &mut mods)
            .unwrap()
            .is_empty()
    );
    assert_eq!(mods.len(), 3);
    // Both directions retain the exact item keys for the combined registry lookup.
    for (layer, id, remainder) in [
        (0, "runes:soup", "runes:extra_item"),
        (0, "runes:ritual", "runes:extra_block"),
        (2, "runes:filled_stone", "runes:stone"),
        (2, "horizon_java:altar_kit", "runes:altar"),
    ] {
        let item = mods[layer]
            .content
            .items
            .iter()
            .find(|item| item.id == id)
            .unwrap();
        assert_eq!(item.crafting_remainder.as_deref(), Some(remainder));
    }

    // Host remainders in unserved namespaces and unresolved keys are both deferred to the
    // registry. The syntactically valid references do not make the declaration batch invalid.
    for (label, bad_item) in [
        (
            "unserved package remainder",
            item("horizon_java:stolen_rem", Some("plain:spoon")),
        ),
        (
            "missing remainder",
            item("runes:missing_rem", Some("runes:nonexistent")),
        ),
    ] {
        let mut errors = Vec::new();
        let id = bad_item.id.clone();
        let remainder = bad_item.crafting_remainder.clone();
        let batches = BTreeMap::from([(
            "horizon_java",
            DeclaredContent {
                items: vec![bad_item.validate_local().unwrap()],
                ..Default::default()
            },
        )]);
        accept_declared(&mut mods, &batches, &manifests, &known, &mut errors).unwrap();
        assert!(errors.is_empty(), "{label}: {errors:?}");
        let item = mods[2]
            .content
            .items
            .iter()
            .find(|item| item.id == id)
            .unwrap();
        assert_eq!(item.crafting_remainder, remainder, "{label}");
    }

    // An itemless block never reserves an item ID in either direction across layers:
    // (1) code itemless block + file item with the same ID, and
    // (2) file itemless block + code item with the same ID.
    let mut itemless_cross_runes = Content::declared("runes");
    itemless_cross_runes
        .blocks
        .push(block("runes:no_item", false));
    itemless_cross_runes.items.push(item("runes:stone", None));
    let mut itemless_cross_mods = vec![ContentMod {
        id: "runes".into(),
        content: itemless_cross_runes,
        jars: Vec::new(),
    }];
    let mut itemless_cross_errors = Vec::new();
    let itemless_cross_batches = BTreeMap::from([(
        "horizon_java",
        DeclaredContent {
            blocks: vec![block("runes:stone", false).validate_local().unwrap()],
            items: vec![item("runes:no_item", None).validate_local().unwrap()],
            ..Default::default()
        },
    )]);
    accept_declared(
        &mut itemless_cross_mods,
        &itemless_cross_batches,
        &manifests,
        &known,
        &mut itemless_cross_errors,
    )
    .unwrap();
    assert!(
        itemless_cross_errors.is_empty(),
        "{itemless_cross_errors:?}"
    );
    assert!(
        itemless_cross_mods[1]
            .content
            .blocks
            .iter()
            .any(|b| b.id == "runes:stone" && !b.item)
    );
    assert!(
        itemless_cross_mods[1]
            .content
            .items
            .iter()
            .any(|i| i.id == "runes:no_item")
    );

    // Every true block/item collision against a served package refuses atomically.
    let cases = [
        (
            "duplicate block",
            DeclaredContent {
                blocks: [block("runes:altar", true), block("runes:atomic_skip", true)]
                    .into_iter()
                    .map(|b| b.validate_local().unwrap())
                    .collect(),
                ..Default::default()
            },
            "runes:altar",
        ),
        (
            "item-bearing block colliding with served item",
            DeclaredContent {
                blocks: [block("runes:stone", true), block("runes:atomic_skip", true)]
                    .into_iter()
                    .map(|b| b.validate_local().unwrap())
                    .collect(),
                ..Default::default()
            },
            "runes:stone",
        ),
        (
            "item colliding with served block item",
            DeclaredContent {
                blocks: vec![block("runes:atomic_skip", true).validate_local().unwrap()],
                items: vec![item("runes:altar", None).validate_local().unwrap()],
                ..Default::default()
            },
            "runes:altar",
        ),
        (
            "duplicate item",
            DeclaredContent {
                blocks: vec![block("runes:atomic_skip", true).validate_local().unwrap()],
                items: vec![item("runes:stone", None).validate_local().unwrap()],
                ..Default::default()
            },
            "runes:stone",
        ),
    ];
    for (label, batch, conflicting_id) in cases {
        let mut errors = Vec::new();
        let batches = BTreeMap::from([("horizon_java", batch)]);
        accept_declared(&mut mods, &batches, &manifests, &known, &mut errors).unwrap();
        assert_eq!(errors.len(), 1, "{label}: {errors:?}");
        assert!(
            matches!(
                &errors[0].error,
                mod_api::DeclarationError::Rejected { id, reason }
                    if id == conflicting_id && reason.contains("runes")
            ),
            "{label}: {:?}",
            errors[0]
        );
        assert!(
            !mods[2]
                .content
                .blocks
                .iter()
                .any(|b| b.id == "runes:atomic_skip"),
            "{label}: batch must be refused atomically"
        );
    }
}

#[test]
fn spawn_rule_sets_the_valid_spawn_flags_of_every_declared_state() {
    use crate::mobs::data::{STURDY_TOP, VALID_SPAWN, VALID_SPAWN_ICE, VALID_SPAWN_LEAVES};
    let all = u32::from(VALID_SPAWN | VALID_SPAWN_LEAVES | VALID_SPAWN_ICE);
    let leaves = u32::from(VALID_SPAWN_LEAVES);
    let sturdy = u32::from(STURDY_TOP);
    // (properties, template flags, expected spawn bits): a template that refuses everything
    // still takes `always`, a light-15 block refuses under the default rule, and an unset
    // rule keeps the template's flags.
    let cases = [
        (r#""spawn_rule":"default","light":15"#, all | sturdy, 0),
        (r#""spawn_rule":"default","light":13"#, sturdy, all),
        (r#""spawn_rule":"default","light":13"#, all, 0),
        (r#""spawn_rule":"default""#, all, 0),
        (r#""spawn_rule":"always","light":15"#, sturdy, all),
        (r#""spawn_rule":"never""#, all | sturdy, 0),
        (r#""spawn_rule":"ocelot_or_parrot""#, all | sturdy, leaves),
        (r#""light":15"#, all | sturdy, all),
    ];
    for (i, (properties, template_flags, expected)) in cases.iter().enumerate() {
        let declaration = format!(
            r#"{{"template":"minecraft:test","item":false,"properties":{{{properties}}}}}"#
        );
        let (base, mods) = block_base(&format!("spawn-rule-{i}"), &[&declaration]);
        let plan = Plan::new(&base, &mods).unwrap();
        let row = format!(r#"["OPEN",{template_flags},1.0]"#);
        let mut doc: J = serde_json::from_str(&format!(r#"{{"states":[{row},{row}]}}"#)).unwrap();
        plan.json("mob-data.json", &mut doc).unwrap();
        let doc = serde_json::to_value(doc).unwrap();
        for state in [2, 3] {
            let flags = doc["states"][state][1].as_u64().unwrap() as u32;
            assert_eq!(flags & all, *expected, "{properties}: {flags}");
            assert_eq!(flags & !all, template_flags & !all, "{properties}");
        }
        fs::remove_dir_all(base.parent().unwrap()).unwrap();
    }
}
