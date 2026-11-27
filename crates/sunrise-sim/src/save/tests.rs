use crate::mobs::spawner::SpawnDataExt;

#[test]
fn saved_preset_editor_stems_supply_shapes_and_facts_below_pack_stems() {
    use super::*;
    use sunrise_world::worldgen::overrides::{DIMENSIONS, WorldgenOverrides};
    let dir = std::env::temp_dir().join(format!(
        "sunrise-preset-shapes-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let dimensions = json_tag(&serde_json::json!({
        "minecraft:overworld": {"type":"minecraft:the_nether","generator":{"type":"minecraft:debug"}},
        "example:pocket": {
            "type":{"min_y":-128,"height":448,"logical_height":320,"has_skylight":false,"has_ceiling":true},
            "generator":{"type":"minecraft:debug"}
        }
    }));
    write_world_gen_settings_with(&dir, 1, WorldGen::default(), None, None, Some(dimensions))
        .unwrap();
    let generator = WorldGenerator::read_new(&dir).unwrap();
    let mut overrides = WorldgenOverrides::default();
    generator.merge_dimensions(&mut overrides);
    let overworld = overrides.level_height("minecraft:overworld").unwrap();
    assert_eq!((overworld.min_y, overworld.height), (0, 256));
    assert!(overworld.has_ceiling && !overworld.has_skylight);
    let pocket = overrides.level_height("example:pocket").unwrap();
    assert_eq!(
        (pocket.min_y, pocket.height, pocket.logical_height),
        (-128, 448, 320)
    );
    assert!(pocket.has_ceiling && !pocket.has_skylight);
    assert!(
        overrides
            .level_facts("minecraft:overworld")
            .unwrap()
            .unwrap()
            .fast_lava
    );
    let pack_stem = r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:debug"}}"#;
    overrides.set(DIMENSIONS, "minecraft:overworld", pack_stem.into());
    generator.merge_dimensions(&mut overrides);
    assert_eq!(
        overrides
            .entries(DIMENSIONS)
            .find(|(id, _)| *id == "minecraft:overworld")
            .unwrap()
            .1,
        pack_stem
    );
    assert_eq!(
        overrides.level_height("minecraft:overworld").unwrap().min_y,
        -64
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_ender_chest_saves_none_of_the_players_items() {
    use crate::containers::BlockEntity;
    let mut slots = vec![None; 27];
    slots[0] = Some(crate::items::ItemStack::new("minecraft:diamond", 3));
    let chest = super::block_entity_encode::encode(
        (1, 2, 3),
        &BlockEntity::Chest(slots.clone()),
        "minecraft:chest",
        &Default::default(),
    );
    assert!(
        chest.get("Items").is_some(),
        "control: a chest saves its items"
    );
    let ender = super::block_entity_encode::encode(
        (1, 2, 3),
        &BlockEntity::Chest(slots),
        "minecraft:ender_chest",
        &Default::default(),
    );
    assert_eq!(
        ender.get("id"),
        Some(&super::string("minecraft:ender_chest"))
    );
    assert!(ender.get("Items").is_none(), "{ender:?}");
}
#[test]
fn registered_rule_survives_level_save_and_absence() {
    use crate::gamerules::{Rule, RuleRegistry, Value};
    let dir = std::env::temp_dir().join(format!("sunrise-mod-rule-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rule = Rule {
        name: "example:enabled".into(),
        category: "example:machines".into(),
        default: Value::Bool(true),
        min: 0,
        max: 0,
    };
    let registry = std::sync::Arc::new(
        RuleRegistry::with_mod_rules(&["example:machines".into()], &[rule]).unwrap(),
    );
    let mut level = super::Level {
        rule_registry: registry.clone(),
        ..Default::default()
    };
    level.game_rules.insert("example:enabled".into(), 0);
    level.write(&dir, &Default::default()).unwrap();
    assert_eq!(
        super::read_saved_data(&dir, "game_rules")
            .unwrap()
            .get("example:enabled"),
        Some(&super::Tag::Byte(0))
    );
    let mut absent = super::Level::read(&dir).unwrap();
    absent.write(&dir, &Default::default()).unwrap();
    assert_eq!(
        super::read_saved_data(&dir, "game_rules")
            .unwrap()
            .get("example:enabled"),
        Some(&super::Tag::Byte(0))
    );
    let mut returned = super::Level::read_with_registry(&dir, registry).unwrap();
    assert_eq!(returned.game_rules.get("example:enabled"), Some(&0));
    returned.write(&dir, &Default::default()).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dedicated_generator_recovery_uses_a_new_seed_and_persists_pack_dimensions() {
    use super::*;
    use sunrise_world::worldgen::overrides::{DIMENSIONS, WorldgenOverrides};

    const RECOVERY_SEED: i64 = -546_172;
    let scratch = tempfile::tempdir().unwrap();
    let world = scratch.path();
    let settings = saved_data_path(world, "world_gen_settings");
    Level::default().write_without_player(world).unwrap();
    let mut overrides = WorldgenOverrides::default();
    overrides.set(
        DIMENSIONS,
        "minecraft:overworld",
        r#"{"type":"test:tall","generator":{"type":"minecraft:noise","settings":"minecraft:overworld","biome_source":{"type":"minecraft:fixed","biome":"minecraft:plains"}}}"#.to_owned(),
    );

    for case in ["missing", "unreadable", "seedless", "no-dimensions"] {
        match case {
            "missing" => fs::remove_file(&settings).unwrap(),
            "unreadable" => fs::write(&settings, b"unreadable generator settings").unwrap(),
            "seedless" | "no-dimensions" => {
                let Tag::Compound(mut invalid) =
                    read_saved_data(world, "world_gen_settings").unwrap()
                else {
                    panic!("expected saved settings")
                };
                let missing = if case == "seedless" {
                    "seed"
                } else {
                    invalid.insert("seed".into(), Tag::Long(13_579));
                    "dimensions"
                };
                invalid.remove(&missing.into());
                write_saved_data(world, "world_gen_settings", Tag::Compound(invalid)).unwrap();
            }
            _ => unreachable!(),
        }
        let before = fs::read(&settings).ok();
        let mut recovered = Level::read(world).unwrap();
        recovered
            .recover_world_generator(&overrides, || RECOVERY_SEED)
            .unwrap();
        assert_eq!(recovered.seed, RECOVERY_SEED, "{case}");
        assert!(world_gen_settings(world).is_none(), "{case}");
        assert_eq!(fs::read(&settings).ok(), before, "{case}");
        recovered.write_without_player(world).unwrap();
        let saved = read_saved_data(world, "world_gen_settings").unwrap();
        assert_eq!(saved.get("seed"), Some(&Tag::Long(RECOVERY_SEED)));
        assert_eq!(
            saved
                .get("dimensions")
                .and_then(|d| d.get("minecraft:overworld"))
                .and_then(|d| d.get("type")),
            Some(&Tag::String("test:tall".into())),
        );
    }

    write_world_gen_settings(world, 0, WorldGen::default()).unwrap();
    let mut valid = Level::read(world).unwrap();
    valid
        .recover_world_generator(&overrides, || {
            panic!("a valid saved zero must not draw a new seed")
        })
        .unwrap();
    assert_eq!(valid.seed, 0);
}

#[test]
fn a_server_falls_back_to_level_dat_old_and_restores_it() {
    let world = std::env::temp_dir().join(format!("sunrise-dat-old-{}", std::process::id()));
    let _ = fs::remove_dir_all(&world);
    fs::create_dir_all(&world).unwrap();
    let level = |name: &str| {
        Tag::compound([(
            "Data",
            Tag::compound([("LevelName", Tag::String(name.into()))]),
        )])
    };
    // Both files broken, or only level.dat_old there: nothing to fall back to.
    fs::write(world.join("level.dat"), b"cut short").unwrap();
    assert!(!fall_back_to_level_dat_old(&world));
    fs::write(world.join("level.dat_old"), b"cut short").unwrap();
    assert!(!fall_back_to_level_dat_old(&world));
    // A good level.dat_old: it takes level.dat's place and the broken file is kept.
    write_dat(&world.join("level.dat_old"), level("old")).unwrap();
    assert!(fall_back_to_level_dat_old(&world));
    assert!(level_dat_issue(&world, false).is_none());
    let kept = fs::read_dir(&world)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("level.dat_corrupted_")
        })
        .count();
    assert_eq!(kept, 1);
    // A readable level.dat is left alone.
    assert!(!fall_back_to_level_dat_old(&world));
    let _ = fs::remove_dir_all(&world);
}

#[test]
fn points_of_interest_save_as_poi_sections() {
    let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
        return;
    };
    let assets = crate::assets::Assets::load(&PathBuf::from(root)).expect("assets");
    let tables = SaveTables::new(&assets);
    let state = |name: &str, props: &[(&str, &str)]| {
        let mut s = assets
            .registry
            .state(assets.registry.default_state(name).unwrap())
            .unwrap()
            .clone();
        for (k, v) in props {
            s.properties.insert((*k).into(), (*v).into());
        }
        assets.registry.id(&s).unwrap().0
    };
    let mut chunk = Chunk::empty(2, -1);
    chunk.set(1, 70, 1, state("minecraft:red_bed", &[("part", "head")]));
    chunk.set(1, 70, 2, state("minecraft:red_bed", &[("part", "foot")]));
    chunk.set(5, 71, 5, state("minecraft:bell", &[]));
    chunk.set(6, 71, 5, state("minecraft:stone", &[]));
    chunk.set(9, 70, 9, state("minecraft:straw_bed", &[("part", "head")]));
    assert!(encode_poi(&Chunk::empty(0, 0), &tables, None).is_none());
    let saved = encode_poi(&chunk, &tables, None).expect("records");
    let Some(Tag::Compound(sections)) = saved.tag.get("Sections") else {
        panic!("no sections");
    };
    assert_eq!(sections.len(), 1);
    let section = &sections[&"4".into()];
    assert_eq!(section.get("Valid"), Some(&Tag::Byte(1)));
    let Some(Tag::List(records)) = section.get("Records") else {
        panic!("no records");
    };
    let records: Vec<_> = records
        .iter()
        .map(|r| {
            (
                text(r.get("type")).unwrap(),
                r.get("pos").cloned(),
                number(r.get("free_tickets")),
            )
        })
        .collect();
    assert_eq!(
        records,
        [
            (
                "minecraft:home".into(),
                Some(Tag::IntArray(vec![33, 70, -15])),
                Some(1)
            ),
            (
                "minecraft:meeting".into(),
                Some(Tag::IntArray(vec![37, 71, -11])),
                Some(32)
            ),
        ]
    );
}

/// A block a mod added to `minecraft:home` (`extend_poi_types`) saves as a
/// home record, as a bed head does.
#[test]
fn states_a_mod_added_to_a_poi_type_save_as_its_records() {
    let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
        return;
    };
    let assets = crate::assets::Assets::load(&PathBuf::from(root)).expect("assets");
    let hay_states: Vec<_> = (0..assets.registry.len())
        .filter_map(|i| assets.registry.state(BlockStateId(i as u32)))
        .filter(|state| state.name == "minecraft:hay_block")
        .map(|state| sunrise_world::poi::PoiExtension {
            state: state.clone(),
            poi: "minecraft:home".into(),
        })
        .collect();
    assert_eq!(hay_states.len(), 3);
    // The tables are built once per registry, so they keep the addition.
    let tables = sunrise_world::poi::with_extensions(hay_states, |refusals| {
        assert_eq!(refusals, Vec::<String>::new());
        SaveTables::new(&assets)
    });
    let hay = assets
        .registry
        .default_state("minecraft:hay_block")
        .unwrap();
    assert_eq!(tables.poi[hay.0 as usize], Some(("minecraft:home", 1)));
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    assert_eq!(tables.poi[stone.0 as usize], None);
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(3, 70, 3, hay.0);
    chunk.set(4, 70, 3, stone.0);
    let saved = encode_poi(&chunk, &tables, None).expect("records");
    let Some(Tag::Compound(sections)) = saved.tag.get("Sections") else {
        panic!("no sections");
    };
    let Some(Tag::List(records)) = sections[&"4".into()].get("Records") else {
        panic!("no records");
    };
    let records: Vec<_> = records
        .iter()
        .map(|r| (text(r.get("type")).unwrap(), r.get("pos").cloned()))
        .collect();
    assert_eq!(
        records,
        [("minecraft:home".into(), Some(Tag::IntArray(vec![3, 70, 3])))]
    );
}

#[test]
fn a_chunk_that_loses_its_last_poi_saves_empty_sections() {
    let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Arc::new(
        Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
            root.to_path_buf(),
        ))
        .expect("biomes"),
    );
    let bell = assets.registry.default_state("minecraft:bell").unwrap().0;
    let dir = std::env::temp_dir().join(format!("sunrise-poi-save-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
    let _ = save.tables.set(Arc::new(SaveTables::new(&assets)));
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(5, 71, 5, bell);
    let sections = |save: &SaveDir| match save.read_record("poi", 0, 0)?.tag.get("Sections") {
        Some(Tag::Compound(s)) => Some(s.len()),
        _ => None,
    };
    save.save_later(&[Arc::new(chunk.clone())], assets.registry, &biomes)
        .unwrap();
    save.flush().unwrap();
    assert_eq!(sections(&save), Some(1));
    chunk.set(5, 71, 5, 0);
    save.save_later(&[Arc::new(chunk)], assets.registry, &biomes)
        .unwrap();
    save.flush().unwrap();
    assert_eq!(
        sections(&SaveDir::new(dir.clone(), LevelShape::OVERWORLD, None)),
        Some(0)
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn saved_poi_records_keep_their_claims() {
    let record = |pos: [i32; 3], kind: &str, tickets: Option<i32>| {
        let mut r = vec![("pos", Tag::IntArray(pos.to_vec())), ("type", string(kind))];
        if let Some(t) = tickets {
            r.push(("free_tickets", Tag::Int(t)));
        }
        Tag::compound(r)
    };
    let poi = |records: Vec<Tag>| {
        let mut sections = Compound::new();
        sections.insert(
            "4".into(),
            Tag::compound([("Valid", Tag::Byte(1)), ("Records", Tag::List(records))]),
        );
        poi_root(sections)
    };
    let old = poi(vec![
        record([1, 70, 1], "minecraft:home", None),
        record([5, 71, 5], "minecraft:meeting", Some(30)),
    ]);
    let mut saved = poi(vec![
        record([1, 70, 1], "minecraft:home", Some(1)),
        record([5, 71, 5], "minecraft:meeting", Some(32)),
        record([9, 71, 9], "minecraft:home", Some(1)),
    ]);
    keep_claims(&mut saved, &old);
    let expected = poi(vec![
        record([1, 70, 1], "minecraft:home", None),
        record([5, 71, 5], "minecraft:meeting", Some(30)),
        record([9, 71, 9], "minecraft:home", Some(1)),
    ]);
    assert_eq!(saved.tag, expected.tag);
}

#[test]
fn level_attachments_survive_save_and_load() {
    use sunrise_mod_runtime::attachments::{
        AttachmentCodec, AttachmentError, AttachmentType, Attachments,
    };
    let dir =
        std::env::temp_dir().join(format!("sunrise-level-attachments-{}", std::process::id()));
    let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
    let key = AttachmentType {
        namespace: "test",
        name: "counter",
        default: || 0,
        codec: Some(AttachmentCodec {
            encode: |n: &i32| Ok([("count".into(), Tag::Int(*n))].into()),
            decode: |c: &Compound| match c.get(&"count".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
        copy_on_death: false,
        sync_codec: None,
    };
    let mut map = Attachments::default();
    key.set(&mut map, sunrise_mod_runtime::attachments::Holder::Level, 7)
        .unwrap();
    save.save_attachments(&map).unwrap();
    assert_eq!(
        key.get(
            &mut save.load_attachments(),
            sunrise_mod_runtime::attachments::Holder::Level
        ),
        Ok(7)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
fn loaded_chunk_uses_saved_monument_from_its_referenced_start_chunk() {
    use sunrise_core::{
        geometry::Direction,
        random::{LegacyRandom, WorldgenRandom},
    };
    use sunrise_world::worldgen::structure::{
        legacy::{LegacyPiece, random_horizontal},
        monument::MonumentBuilding,
    };
    let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").expect("resources"));
    let assets = Arc::new(crate::assets::Assets::load(&root).expect("assets"));
    let biomes = Arc::new(
        Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
            root.clone(),
        ))
        .expect("biomes"),
    );
    let item_data = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ServerDataLoad,
    )
    .expect("item data");
    let inner = Arc::new(
        crate::vanillagen::VanillaTerrain::new(
            crate::vanillagen::TerrainResources {
                data: None,
                dir: &root.join("worldgen"),
                registry: assets.registry,
                biomes: &biomes,
                item_data: &item_data,
            },
            12345,
        )
        .unwrap(),
    );
    // A moved, rotated monument that generation cannot reproduce at the owning chunk.
    // Vanilla reseeds, draws the default even with a saved orientation, then rebuilds.
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    random.set_large_feature_seed(12345, -2, 3);
    random_horizontal(&mut random);
    let top = MonumentBuilding::new(&mut random, 100, -300, Direction::East);
    let expected = Tag::compound([
        ("id", string("minecraft:monument")),
        ("ChunkX", Tag::Int(-2)),
        ("ChunkZ", Tag::Int(3)),
        ("references", Tag::Int(5)),
        (
            "Children",
            Tag::List(vec![Tag::Compound(top.save().unwrap())]),
        ),
    ]);
    let mut saved = encode(
        &Chunk::empty(-2, 3),
        assets.registry,
        &biomes,
        &Default::default(),
    );
    let Some(Tag::Compound(structures)) = saved.tag.get_mut("structures") else {
        panic!("structures")
    };
    structures.insert(
        "starts".into(),
        Tag::compound([("minecraft:monument", expected.clone())]),
    );
    let save = Arc::new(SaveDir::new(
        std::env::temp_dir().join(format!("sunrise-saved-monument-{}", std::process::id())),
        LevelShape::OVERWORLD,
        Some(Default::default()),
    ));
    save.pending
        .lock()
        .unwrap()
        .insert((Folder::Region, -2, 3), (0, Record::Tag(Arc::new(saved))));
    let generator = Persistent {
        inner: inner.clone(),
        save,
        registry: assets,
        biomes,
    };
    let mut worker = generator.worker();
    let mut referring = Chunk::empty(-1, 3);
    referring
        .structure_refs
        .push(("minecraft:monument".into(), (-2, 3)));
    generator.prepare_loaded(&mut worker, &referring);
    let starts = inner.starts_at(-2, 3);
    let restored = starts
        .iter()
        .find(|s| s.name == "minecraft:monument")
        .expect("referenced saved monument");
    assert_eq!(restored.save(), Some(expected));
}

#[test]
fn loaded_chunk_uses_saved_jigsaw_from_its_referenced_start_chunk() {
    let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = Arc::new(crate::assets::Assets::load(&root).expect("assets"));
    let biomes = Arc::new(
        Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
            root.clone(),
        ))
        .expect("biomes"),
    );
    let item_data = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ServerDataLoad,
    )
    .expect("item data");
    let inner = Arc::new(
        crate::vanillagen::VanillaTerrain::new(
            crate::vanillagen::TerrainResources {
                data: None,
                dir: &root.join("worldgen"),
                registry: assets.registry,
                biomes: &biomes,
                item_data: &item_data,
            },
            12345,
        )
        .unwrap(),
    );
    let mut original = inner
        .structure_start("minecraft:ancient_city", 2, -3)
        .unwrap();
    Arc::get_mut(&mut original).unwrap().references = 4;
    let starts = Tag::compound([
        ("minecraft:ancient_city", original.save().unwrap()),
        (
            "minecraft:mineshaft",
            Tag::compound([("id", string("INVALID"))]),
        ),
    ]);
    let mut saved = encode(
        &Chunk::empty(2, -3),
        assets.registry,
        &biomes,
        &Default::default(),
    );
    let Tag::Compound(root) = &mut saved.tag else {
        panic!("chunk root")
    };
    let Some(Tag::Compound(structures)) = root.get_mut(&"structures".into()) else {
        panic!("structures")
    };
    structures.insert("starts".into(), starts.clone());
    let loaded = decode(&saved, assets.registry, &biomes).expect("chunk");
    let Tag::Compound(expected) = &starts else {
        unreachable!()
    };
    assert_eq!(loaded.saved_structure_starts(), Some(expected));
    let again = encode(&loaded, assets.registry, &biomes, &Default::default());
    assert_eq!(
        again.tag.get("structures").and_then(|s| s.get("starts")),
        Some(&starts)
    );
    let save = Arc::new(SaveDir::new(
        std::env::temp_dir().join(format!("sunrise-saved-starts-{}", std::process::id())),
        LevelShape::OVERWORLD,
        Some(Default::default()),
    ));
    // A pending record is authoritative even before any region file exists.
    save.pending
        .lock()
        .unwrap()
        .insert((Folder::Region, 2, -3), (0, Record::Tag(Arc::new(saved))));
    let generator = Persistent {
        inner: inner.clone(),
        save,
        registry: assets,
        biomes,
    };
    let mut worker = generator.worker();
    let mut referring = Chunk::empty(3, -3);
    referring
        .structure_refs
        .push(("minecraft:ancient_city".into(), (2, -3)));
    generator.prepare_loaded(&mut worker, &referring);
    let starts = inner.starts_at(2, -3);
    let restored = starts
        .iter()
        .find(|s| s.name == original.name)
        .expect("referenced saved start");
    assert_eq!(restored.save(), original.save());
    assert_eq!(format!("{restored:?}"), format!("{original:?}"));
}

#[test]
fn chunk_attachments_survive_save_and_load() {
    use sunrise_mod_runtime::attachments::{AttachmentCodec, AttachmentError, AttachmentType};
    let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))
    .expect("biomes");
    let key = AttachmentType {
        namespace: "test",
        name: "counter",
        default: || 0,
        codec: Some(AttachmentCodec {
            encode: |n: &i32| Ok([("count".into(), Tag::Int(*n))].into()),
            decode: |c: &Compound| match c.get(&"count".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
        copy_on_death: false,
        sync_codec: None,
    };
    let mut chunk = Chunk::empty(2, 3);
    let mut extras = ChunkExtras::default();
    key.set(
        &mut extras.attachments,
        sunrise_mod_runtime::attachments::Holder::Level,
        42,
    )
    .unwrap();
    let mut block_data = sunrise_mod_runtime::attachments::Attachments::default();
    key.set(
        &mut block_data,
        sunrise_mod_runtime::attachments::Holder::Level,
        21,
    )
    .unwrap();
    let pos = (33, 64, 50);
    let chest = assets.registry.default_state("minecraft:chest").unwrap();
    chunk.set(1, 64, 2, chest.0);
    chunk
        .block_entities
        .push((pos, crate::containers::BlockEntity::Chest(vec![None; 27])));
    extras.block_entity_attachments.push((pos, block_data));
    chunk.saved = Some(Arc::new(extras));
    let saved = encode(&chunk, assets.registry, &biomes, &Default::default());
    let mut loaded = decode(&saved, assets.registry, &biomes).expect("chunk");
    assert_eq!(
        key.get(
            &mut Arc::make_mut(loaded.saved.as_mut().unwrap()).attachments,
            sunrise_mod_runtime::attachments::Holder::Level
        ),
        Ok(42)
    );
    let extras = Arc::make_mut(loaded.saved.as_mut().unwrap());
    assert_eq!(extras.block_entity_attachments[0].0, pos);
    assert_eq!(
        key.get(
            &mut extras.block_entity_attachments[0].1,
            sunrise_mod_runtime::attachments::Holder::Level
        ),
        Ok(21)
    );
}

#[test]
fn unopened_pending_containers_save_their_loot_table() {
    let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))
    .expect("biomes");
    let tables = SaveTables::new(&assets);
    let chest = assets.registry.default_state("minecraft:chest").unwrap();
    let mut slots = vec![None; 27];
    slots[3] = Some(crate::items::ItemStack::new("minecraft:bone", 2));
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(1, 64, 2, chest.0);
    chunk.block_entities.push((
        (1, 64, 2),
        crate::containers::BlockEntity::Chest(slots.clone()),
    ));
    let pending = PendingContainerLoot {
        table: string("minecraft:chests/simple_dungeon"),
        seed: Some(Tag::Long(42)),
    };
    chunk.block_entities[0].1.slots_mut().fill(None);
    chunk.saved = Some(Arc::new(ChunkExtras {
        pending_loot: [((1, 64, 2), pending)].into(),
        ..ChunkExtras::default()
    }));
    let saved_chest = |chunk: &Chunk| -> Tag {
        let saved = encode_with(
            chunk,
            assets.registry,
            &biomes,
            Some(&tables),
            LevelShape::OVERWORLD,
            &Default::default(),
        );
        let Some(Tag::List(list)) = saved.tag.get("block_entities") else {
            panic!("no block entities");
        };
        list[0].clone()
    };
    let unopened = saved_chest(&chunk);
    assert_eq!(
        text(unopened.get("LootTable")).as_deref(),
        Some("minecraft:chests/simple_dungeon")
    );
    assert_eq!(number(unopened.get("LootTableSeed")), Some(42));
    assert!(unopened.get("Items").is_none());
    Arc::make_mut(chunk.saved.as_mut().unwrap())
        .pending_loot
        .remove(&(1, 64, 2));
    // The shared unpack transition consumes metadata before inventory changes.
    if let crate::containers::BlockEntity::Chest(s) = &mut chunk.block_entities[0].1 {
        s[3] = None;
        s[4] = Some(crate::items::ItemStack::new("minecraft:bone", 1));
    }
    let opened = saved_chest(&chunk);
    assert!(opened.get("LootTable").is_none());
    assert!(opened.get("Items").is_some());
    // Loaded with its table, rolled, then emptied: no table, or it refills on load.
    let mut extras = ChunkExtras::clone(chunk.saved.as_ref().unwrap());
    extras.block_entity_sources = vec![(
        (1, 64, 2),
        Tag::compound([
            ("id", string("minecraft:chest")),
            ("x", Tag::Int(1)),
            ("y", Tag::Int(64)),
            ("z", Tag::Int(2)),
            ("LootTable", string("minecraft:chests/simple_dungeon")),
            ("LootTableSeed", Tag::Long(42)),
        ]),
    )];
    chunk.saved = Some(Arc::new(extras));
    if let crate::containers::BlockEntity::Chest(s) = &mut chunk.block_entities[0].1 {
        s.fill(None);
    }
    let emptied = saved_chest(&chunk);
    assert!(emptied.get("LootTable").is_none());
    assert!(emptied.get("LootTableSeed").is_none());
}

#[test]
fn a_tall_level_saves_and_loads_its_whole_height() {
    let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))
    .expect("biomes");
    let tables = SaveTables::new(&assets);
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    // To The Max's Overworld: -64..2032, 131 sections.
    let shape = LevelShape::new(-64, 2096, 2096, true, false).unwrap();
    let mut chunk = Chunk::with_layout(0, 0, shape.column());
    chunk.set(0, -64, 0, stone.0);
    chunk.set(0, 2000, 0, stone.0);
    let saved = encode_with(
        &chunk,
        assets.registry,
        &biomes,
        Some(&tables),
        shape,
        &Default::default(),
    );
    assert_eq!(number(saved.tag.get("yPos")), Some(-4));
    let Some(Tag::List(sections)) = saved.tag.get("sections") else {
        panic!("no sections");
    };
    assert_eq!(sections.len(), 131);
    // 12-bit heightmap entries: y 2000's top reads 2001 - -64.
    let Some(Tag::LongArray(surface)) = saved
        .tag
        .get("Heightmaps")
        .and_then(|h| h.get("WORLD_SURFACE"))
    else {
        panic!("no WORLD_SURFACE");
    };
    assert_eq!(surface[0] & 0xfff, 2065);
    let back = decode_with(
        &saved,
        assets.registry,
        &biomes,
        Some(&tables),
        shape.column(),
    )
    .expect("decodes");
    assert_eq!(back.sections.len(), 131);
    assert_eq!(back.get(0, 2000, 0), stone.0);
    assert_eq!(back.get(0, -64, 0), stone.0);
    // Read into the vanilla column, the sections above it are left out.
    let short = decode_with(
        &saved,
        assets.registry,
        &biomes,
        Some(&tables),
        ColumnLayout::VANILLA,
    )
    .expect("decodes");
    assert_eq!(short.sections.len(), SECTIONS);
    assert_eq!(short.get(0, 2000, 0), 0);
    assert_eq!(short.get(0, -64, 0), stone.0);
}

/// `BlockEntity.getPosFromTag`: a block entity saved under another chunk's
/// coordinates is read into the chunk that saved it (vanilla warns and
/// keeps the low four bits), instead of landing where that chunk has no
/// block.
#[test]
fn a_block_entity_saved_under_another_chunks_coordinates_moves_here() {
    let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        root.clone(),
    ))
    .expect("biomes");
    // The destination must contain a chest: the fork validates saved block
    // entity membership before loading, independently of position repair.
    // The current saved block-state codec uses `id`, not the old `Name` key.
    let tag = sunrise_core::nbt::parse_snbt(
        r#"{xPos:0,zPos:0,yPos:-4,Status:"minecraft:full",sections:[{Y:4b,block_states:{palette:[{id:"minecraft:chest"}]}}],block_entities:[{id:"minecraft:chest",x:37,y:70,z:-3,Items:[]}]}"#,
    )
    .unwrap()
    .0;
    let chunk = decode_with(
        &NamedTag {
            name: "".into(),
            tag,
        },
        assets.registry,
        &biomes,
        None,
        ColumnLayout::VANILLA,
    )
    .expect("chunk");
    let positions: Vec<(i32, i32, i32)> = chunk.block_entities.iter().map(|(p, _)| *p).collect();
    assert_eq!(
        positions,
        [(5, 70, 13)],
        "moved into the chunk that saved it"
    );
    let order = &chunk.saved.as_ref().expect("extras").block_entity_order;
    assert_eq!(
        order,
        &[(5, 70, 13)],
        "load order follows the moved position"
    );
}

#[test]
fn nether_and_end_chunks_save_with_their_own_height() {
    let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))
    .expect("biomes");
    let tables = SaveTables::new(&assets);
    let rack = assets
        .registry
        .default_state("minecraft:netherrack")
        .unwrap();
    let mut chunk = Chunk::empty(0, 0);
    for y in 0..128 {
        chunk.set(0, y, 0, rack.0);
    }
    let saved = encode_with(
        &chunk,
        assets.registry,
        &biomes,
        Some(&tables),
        LevelShape::NETHER,
        &Default::default(),
    );
    assert_eq!(number(saved.tag.get("yPos")), Some(0));
    let Some(Tag::List(sections)) = saved.tag.get("sections") else {
        panic!("no sections");
    };
    let ys: Vec<i32> = sections
        .iter()
        .filter_map(|s| number(s.get("Y")).map(|y| y as i32))
        .collect();
    assert_eq!(ys, (0..16).collect::<Vec<_>>());
    let Some(Tag::List(post)) = saved.tag.get("PostProcessing") else {
        panic!("no PostProcessing");
    };
    assert_eq!(post.len(), 16);
    // Heightmap values count from the level's minimum: the column's top
    // block at y=127 reads 128 (vanilla), not 192 (from the Overworld's -64).
    let Some(Tag::LongArray(surface)) = saved
        .tag
        .get("Heightmaps")
        .and_then(|h| h.get("WORLD_SURFACE"))
    else {
        panic!("no WORLD_SURFACE");
    };
    assert_eq!(surface[0] & 0x1ff, 128);
    let overworld = encode_with(
        &chunk,
        assets.registry,
        &biomes,
        Some(&tables),
        LevelShape::OVERWORLD,
        &Default::default(),
    );
    assert_eq!(number(overworld.tag.get("yPos")), Some(-4));
}

#[test]
fn saved_far_surface_reads_builds_and_water_without_gameplay_chunks() {
    let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))
    .expect("biomes");
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    let water = assets.registry.default_state("minecraft:water").unwrap();
    let sand = assets.registry.default_state("minecraft:sand").unwrap();
    let dirt = assets.registry.default_state("minecraft:dirt").unwrap();
    let leaves = assets
        .registry
        .default_state("minecraft:oak_leaves")
        .unwrap();
    let end_stone = assets
        .registry
        .default_state("minecraft:end_stone")
        .unwrap();
    let chorus = assets
        .registry
        .default_state("minecraft:chorus_plant")
        .unwrap();
    let flower = assets
        .registry
        .default_state("minecraft:chorus_flower")
        .unwrap();
    let mut chunk = Chunk::empty(-2, 3);
    chunk.set(0, 109, 0, stone.0);
    chunk.set(3, 121, 5, stone.0);
    chunk.set(3, 122, 5, stone.0);
    chunk.set(3, 120, 5, dirt.0);
    chunk.set(3, 119, 5, dirt.0);
    chunk.set(3, 115, 5, stone.0);
    chunk.set(11, 125, 11, stone.0);
    chunk.set(14, 127, 14, stone.0);
    chunk.set(14, 112, 14, stone.0);
    for y in 95..98 {
        chunk.set(2, y, 6, stone.0);
    }
    chunk.lit = true;
    let mut cave_light = Box::new([0x03; 4096]);
    cave_light[crate::chunk::index(12, 125, 11)] = 0xa7;
    Arc::make_mut(&mut chunk.sections[((121 - MIN_Y) >> 4) as usize]).light =
        Light::Full(cave_light);
    chunk.set(7, 90, 9, sand.0);
    chunk.set(7, 98, 9, leaves.0);
    let forest = biomes.id("minecraft:forest");
    chunk.set_biome_cell(7 / 4, (98 - MIN_Y) / 4, 9 / 4, forest);
    chunk.set(4, 54, 4, sand.0);
    for y in 55..64 {
        chunk.set(4, y, 4, water.0);
    }
    // Chorus is thin and non-solid but still part of the far End.
    chunk.set(12, 100, 2, end_stone.0);
    chunk.set(12, 101, 2, chorus.0);
    chunk.set(12, 102, 2, flower.0);
    let mut saved = encode(&chunk, assets.registry, &biomes, &Default::default());
    put(
        &mut saved.tag,
        "block_entities",
        Tag::String("not decoded by far sampling".into()),
    );
    let dir = std::env::temp_dir().join(format!("sunrise-lod-save-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
    save.write_records("region", vec![((-2, 3), Some(saved))], true)
        .unwrap();
    let reopened = SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
    let surfaces = reopened
        .lod_surfaces(-2, 3, &assets, &biomes)
        .expect("saved summary");
    assert_eq!(surfaces[0].surface.unwrap().height, 110);
    assert_eq!(surfaces[0].surface.unwrap().state, stone);
    let narrow_build = surfaces[5 * 16 + 3].surface.unwrap();
    assert_eq!((narrow_build.height, narrow_build.state), (123, stone));
    assert!(
        surfaces[5 * 16 + 2].surface.is_none(),
        "adjacent columns stay distinct"
    );
    let strata = &surfaces[5 * 16 + 3].spans;
    assert_eq!(
        strata
            .iter()
            .map(|span| (span.min, span.max, span.state))
            .collect::<Vec<_>>(),
        vec![(115, 116, stone), (119, 121, dirt), (121, 123, stone)]
    );
    assert!(
        strata.iter().all(|span| span.light == 0x3000),
        "saved cave darkness and block light survive"
    );
    assert_eq!(
        surfaces[11 * 16 + 11].spans[0].light,
        0x70a0,
        "neighbor face light uses separate sky/block channels"
    );
    let boundaries = &surfaces[14 * 16 + 14].spans;
    assert_eq!(boundaries.len(), 2);
    assert!(
        boundaries.iter().all(|span| span.light == 0x30f0),
        "face light crosses section boundaries above and below"
    );
    let across_sections = &surfaces[6 * 16 + 2].spans;
    assert_eq!(across_sections.len(), 1);
    assert_eq!((across_sections[0].min, across_sections[0].max), (95, 98));
    let canopy = surfaces[9 * 16 + 7].surface.unwrap();
    assert_eq!(
        (canopy.height, canopy.state, canopy.biome),
        (99, leaves, forest)
    );
    let underwater = surfaces[4 * 16 + 4].surface.unwrap();
    assert_eq!((underwater.height, underwater.state), (55, sand));
    assert_eq!(underwater.water, Some((64, water)));
    let chorus_column = &surfaces[2 * 16 + 12];
    assert_eq!(
        chorus_column
            .surface
            .map(|surface| (surface.height, surface.state)),
        Some((103, flower))
    );
    assert_eq!(
        chorus_column
            .spans
            .iter()
            .map(|span| (span.min, span.max, span.state))
            .collect::<Vec<_>>(),
        vec![
            (100, 101, end_stone),
            (101, 102, chorus),
            (102, 103, flower)
        ]
    );
    assert!(surfaces[15].surface.is_none(), "saved void stays empty");
    assert!(
        reopened.regions.lock().unwrap().is_empty(),
        "far reads do not grow gameplay region cache"
    );
    let mut proto = encode(&chunk, assets.registry, &biomes, &Default::default());
    put(
        &mut proto.tag,
        "Status",
        string(ChunkStatus::Terrain.name()),
    );
    put(&mut proto.tag, "xPos", Tag::Int(-3));
    save.write_records("region", vec![((-3, 3), Some(proto))], true)
        .unwrap();
    assert!(
        reopened.lod_surfaces(-3, 3, &assets, &biomes).is_none(),
        "unfinished proto saves must not suppress complete decoration"
    );
    let nether = SaveDir::new(dir.clone(), LevelShape::NETHER, Some(Default::default()));
    let dark = nether.lod_surfaces(-2, 3, &assets, &biomes).unwrap();
    assert!(
        dark.iter()
            .flat_map(|c| &c.spans)
            .all(|s| s.light & 0xf0 == 0),
        "no invented skylight in Nether saves"
    );
    struct SavedOnly(Vec<sunrise_level::chunk_streamer::LodDetail>);
    impl sunrise_level::chunk_streamer::LodSampler for SavedOnly {
        fn sample(&mut self, _: i32, _: i32) -> Option<crate::columns::Surface> {
            panic!("saved data must win")
        }
        fn request_detail(&mut self, chunks: &[(i32, i32)], _: [i32; 2]) {
            assert!(
                chunks.is_empty(),
                "saved builds must not request speculative generation"
            );
        }
        fn drain_details(&mut self) -> Vec<sunrise_level::chunk_streamer::LodDetail> {
            std::mem::take(&mut self.0)
        }
    }
    let inner =
        Arc::new(sunrise_level::testgen::TestTerrain::new(assets.registry, &biomes).unwrap());
    let generator = Arc::new(Persistent {
        inner,
        save: Arc::new(reopened),
        registry: Arc::new(assets),
        biomes: Arc::new(biomes),
    });
    let mut sampler = SavedLodSampler {
        generator,
        inner: Box::new(SavedOnly(Vec::new())),
        cache: Default::default(),
        details: Vec::new(),
        approximate: false,
        generated: Arc::new(sunrise_level::lod_data::cache::DiskCache::new(
            dir.join("far"),
            1,
        )),
        bake: None,
    };
    use sunrise_level::chunk_streamer::LodSampler;
    sampler.request_detail(&[(-2, 3)], [-32, 48]);
    let details = sampler.drain_details();
    assert_eq!(details.len(), 1);
    assert_eq!(&details[0].1[..], &surfaces[..]);
    sampler.cache.clear();
    let generated: LodSurfaces = Arc::new(std::array::from_fn(|_| {
        sunrise_level::lod_data::Column::default()
    }));
    sampler.inner = Box::new(SavedOnly(vec![
        ((-2, 3), generated.clone()),
        ((-9, 9), generated.clone()),
    ]));
    let details = sampler.drain_details();
    assert_eq!(
        &details[0].1[..],
        &surfaces[..],
        "unrequested batch neighbours must preserve saved builds"
    );
    assert!(Arc::ptr_eq(&details[1].1, &generated));
    assert!(
        sampler.generated.get(-2, 3).is_none(),
        "saved builds are not cached as generated"
    );
    assert_eq!(sampler.generated.get(-9, 9).unwrap(), generated);
    // A later session reads the generated chunk back instead of
    // generating it again.
    sampler.request_detail(&[(-9, 9)], [-144, 144]);
    assert_eq!(sampler.drain_details()[0].1, generated);
    sampler.inner =
        sunrise_level::chunk_streamer::ChunkGenerator::lod_sampler(sampler.generator.inner.clone());
    let grid = sampler.grid(-32, 48, 2, 16);
    for z in 0..8 {
        for x in 0..8 {
            assert_eq!(
                grid[z * 16 + x],
                sunrise_level::lod_data::Column::reduced(&surfaces, x * 2, z * 2, 2),
                "saved builds must override a batched direct grid"
            );
        }
    }
    drop((save, sampler, nether));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn saved_far_surface_rejects_invalid_packed_palettes() {
    let mut states = Tag::compound([
        ("palette", Tag::List(vec![Tag::Byte(0), Tag::Byte(1)])),
        ("data", Tag::LongArray(vec![0; 256])),
    ]);
    let mut indices = [9u16; 4096];
    assert_eq!(lod_palette_indices(&states, &mut indices, 4), Some(2));
    assert_eq!(indices, [0; 4096]);
    // Two palette entries take 4 bits a block, 16 blocks a long: block 17
    // is the second of the second long, block 4095 the last of the last.
    let mut data = vec![0i64; 256];
    data[1] = 0x10;
    data[255] = 1 << 60;
    put(&mut states, "data", Tag::LongArray(data));
    assert_eq!(lod_palette_indices(&states, &mut indices, 4), Some(2));
    let set: Vec<usize> = (0..4096).filter(|&i| indices[i] != 0).collect();
    assert_eq!(set, [17, 4095]);
    assert_eq!((indices[17], indices[4095]), (1, 1));
    put(&mut states, "data", Tag::LongArray(vec![0; 255]));
    assert_eq!(lod_palette_indices(&states, &mut indices, 4), None);
    put(&mut states, "palette", Tag::List(Vec::new()));
    assert_eq!(lod_palette_indices(&states, &mut indices, 4), None);
    // One entry needs no data; the biomes' 64 cells pack from 1 bit.
    let one = Tag::compound([("palette", Tag::List(vec![Tag::Byte(0)]))]);
    let mut cells = [9u16; 64];
    assert_eq!(lod_palette_indices(&one, &mut cells, 1), Some(1));
    assert_eq!(cells, [0; 64]);
    let three = Tag::compound([
        ("palette", Tag::List(vec![Tag::Byte(0); 3])),
        // 2 bits a cell, 32 cells a long: cell 1 is 2, cell 33 is 1.
        ("data", Tag::LongArray(vec![0b1000, 0b0100])),
    ]);
    assert_eq!(lod_palette_indices(&three, &mut cells, 1), Some(3));
    assert_eq!((cells[0], cells[1], cells[33], cells[63]), (0, 2, 1, 0));
}

#[test]
#[ignore = "daily"]
fn saved_far_surface_checks_packed_indices_against_their_palettes() {
    let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        root.clone(),
    ))
    .expect("biomes");
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    let forest = biomes.id("minecraft:forest");
    let desert = biomes.id("minecraft:desert");
    assert!(forest != 0 && desert != 0 && forest != desert);
    // One section, y 0..16. Blocks: air and stone, 4 bits a block.
    // Biomes: three of them, 2 bits a 4x4x4 cell, 32 cells a long.
    let chunk = |blocks: Vec<i64>, cells: i64| NamedTag {
        name: "".into(),
        tag: Tag::compound([(
            "sections",
            Tag::List(vec![Tag::compound([
                ("Y", Tag::Byte(0)),
                (
                    "block_states",
                    Tag::compound([
                        (
                            "palette",
                            Tag::List(vec![
                                Tag::String("minecraft:air".into()),
                                Tag::String("minecraft:stone".into()),
                            ]),
                        ),
                        ("data", Tag::LongArray(blocks)),
                    ]),
                ),
                (
                    "biomes",
                    Tag::compound([
                        (
                            "palette",
                            Tag::List(vec![
                                Tag::String("minecraft:forest".into()),
                                Tag::String("minecraft:desert".into()),
                                Tag::String("minecraft:plains".into()),
                            ]),
                        ),
                        ("data", Tag::LongArray(vec![cells, 0])),
                    ]),
                ),
            ])]),
        )]),
    };
    let decode = |root: &NamedTag| {
        decode_lod_surfaces(root, &assets, &biomes, LevelShape::OVERWORLD.column(), true)
    };
    // Stone in the bottom layer (the first 256 blocks, 16 a long).
    let mut blocks = vec![0i64; 256];
    blocks[..16].fill(0x1111_1111_1111_1111);
    // The first cell (x 0..4) is desert, the second (x 4..8) names a
    // fourth biome the palette does not have, the others are forest.
    let columns = decode(&chunk(blocks.clone(), 0b1101)).expect("a valid section");
    let surface = |x: usize| columns[x].surface.unwrap();
    assert_eq!((surface(0).height, surface(0).state), (1, stone));
    assert_eq!(surface(0).biome, desert);
    assert_eq!(surface(4).biome, 0, "an index past the palette is no biome");
    assert_eq!(surface(8).biome, forest);
    // The last block names a third state the palette does not have.
    blocks[255] = 2 << 60;
    assert!(
        decode(&chunk(blocks, 0b1101)).is_none(),
        "a block index past the palette makes no far columns"
    );
}

/// #6386: far terrain looked a section's palette and data up by name for
/// each of its 4096 blocks, and its biomes again for each block it kept:
/// milliseconds for one saved chunk, most of them over the air above the
/// ground, on every far sampling thread.
#[test]
#[ignore = "daily"]
fn saved_far_surface_reads_a_section_once_not_once_a_block() {
    let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        root.clone(),
    ))
    .expect("biomes");
    let dandelion = assets
        .registry
        .default_state("minecraft:dandelion")
        .unwrap();
    // A flower in each section and air around it: nothing far terrain
    // keeps, so what is left to pay is reading the sections' blocks, as
    // loading the chunk for play does.
    let mut chunk = Chunk::empty(3, -5);
    for section in 0..chunk.sections.len() as i32 {
        chunk.set(8, MIN_Y + section * 16 + 8, 8, dandelion.0);
    }
    let saved = encode(&chunk, assets.registry, &biomes, &Default::default());
    let layout = LevelShape::OVERWORLD.column();
    let far = || decode_lod_surfaces(&saved, &assets, &biomes, layout, true);
    let columns = far().expect("far columns");
    assert!(
        columns
            .iter()
            .all(|column| column.surface.is_none() && column.spans.is_empty()),
        "a flower is no far terrain"
    );
    // The fastest of several runs of each, taken in turns, so that a busy
    // machine does not decide.
    let time = |run: &dyn Fn()| {
        let started = std::time::Instant::now();
        run();
        started.elapsed()
    };
    let (mut far_time, mut load_time) = (std::time::Duration::MAX, std::time::Duration::MAX);
    for _ in 0..25 {
        far_time = far_time.min(time(&|| {
            std::hint::black_box(far());
        }));
        load_time = load_time.min(time(&|| {
            std::hint::black_box(decode_with(&saved, assets.registry, &biomes, None, layout));
        }));
    }
    // Measured, optimized and not: 3 to 8 loads (the light layers come on
    // top of the blocks), and 100 to 300 with a lookup by name for each
    // block.
    assert!(
        far_time <= load_time * 32,
        "the far columns of a chunk took {far_time:?}, loading it {load_time:?}"
    );
}

#[test]
fn save_batch_preserves_order_coordinates_and_ownership() {
    use crate::containers::BlockEntity;
    let mut entities = HashMap::new();
    let mut names = HashMap::new();
    for x in [i32::MIN, -17, -16, -1, 0, 15, 16, i32::MAX] {
        for z in [-17, -16, -1, 0, 15, 16] {
            let p = (x, 64, z);
            entities.insert(p, BlockEntity::Chest(vec![None; 27]));
            names.insert(p, string(&format!("container {x},{z}")));
        }
    }
    // Names without a modelled entity must also survive the snapshot.
    names.insert((-2, 80, -3), string("orphan name"));
    let mut source = Chunk::empty(-1, -1);
    source
        .block_entities
        .push(((99, 0, 99), BlockEntity::Chest(vec![])));
    source.block_names.push(((99, 0, 99), string("stale")));
    source.saved = Some(Arc::new(ChunkExtras::default()));
    let source = Arc::new(source);
    let batch = vec![
        source.clone(),
        Arc::new(Chunk::empty(1, 1)),
        Arc::new(Chunk::empty(-2, 0)),
        source.clone(), // Preserve input order and repeated coordinates.
        Arc::new(Chunk::empty(i32::MIN >> 4, -2)),
        Arc::new(Chunk::empty(i32::MAX >> 4, 1)),
        Arc::new(Chunk::empty(100, 100)),
    ];
    let encode = |entries: &[((i32, i32, i32), BlockEntity)]| {
        entries
            .iter()
            .map(|(p, e)| {
                block_entity_encode::encode(*p, e, "minecraft:chest", &Default::default())
            })
            .collect::<Vec<_>>()
    };
    for input in [&batch[..0], &batch[..1], &batch[..]] {
        let actual = chunks_with_block_entities(input, &entities, &names);
        assert_eq!(actual.len(), input.len());
        for (c, original) in actual.iter().zip(input) {
            let expected: Vec<_> = entities
                .iter()
                .filter(|(p, _)| (p.0 >> 4, p.2 >> 4) == (c.x, c.z))
                .map(|(p, e)| (*p, e.clone()))
                .collect();
            let expected_names: Vec<_> = names
                .iter()
                .filter(|(p, _)| (p.0 >> 4, p.2 >> 4) == (c.x, c.z))
                .map(|(p, n)| (*p, n.clone()))
                .collect();
            assert_eq!((c.x, c.z), (original.x, original.z));
            assert_eq!(encode(&c.block_entities), encode(&expected));
            assert_eq!(c.block_names, expected_names);
            assert!(Arc::ptr_eq(&c.sections[0], &original.sections[0]));
            if let Some(saved) = &original.saved {
                assert!(Arc::ptr_eq(c.saved.as_ref().unwrap(), saved));
            }
        }
    }
    let snapshot = chunks_with_block_entities(&batch, &entities, &names);
    entities.clear();
    names.clear();
    assert!(!snapshot[0].block_entities.is_empty());
    assert!(!snapshot[0].block_names.is_empty());
    assert_eq!(source.block_names[0].1, string("stale"));
    assert_eq!(source.block_entities[0].0, (99, 0, 99));
}

#[test]
fn work_shared_out_is_all_done_in_order_when_no_helper_can_start() {
    use super::share_out;
    let squares = |i: usize| i * i;
    let want: Vec<usize> = (0..40).map(squares).collect();
    // Every helper refused (a container at its thread limit): this thread does it all.
    assert_eq!(share_out(40, 4, squares, || false), want);
    // Some refused, some started.
    let mut asked = 0;
    let some = || {
        asked += 1;
        asked % 2 == 0
    };
    assert_eq!(share_out(40, 4, squares, some), want);
    assert_eq!(share_out(40, 4, squares, || true), want);
    assert_eq!(share_out(0, 4, squares, || true), Vec::<usize>::new());
}

/// `RegionFile.getVersion` writes a chunk with the version
/// `region-file-compression` selected: `compress_records` asks
/// `Compression::selected()` for every record it encodes, so a server told
/// to write lz4, gzip or uncompressed regions does
/// (`RegionFileVersion.configure`).
#[test]
fn a_chunk_is_written_with_the_selected_region_version() {
    use super::{Record, compress_records};
    let tag = Arc::new(NamedTag {
        name: "".into(),
        tag: Tag::compound([("xPos", Tag::Int(3))]),
    });
    let records = [((3, 4), Record::Tag(Arc::clone(&tag)))];
    let refs: Vec<_> = records.iter().map(|(k, r)| (*k, r)).collect();
    for name in ["none", "lz4", "gzip", "deflate"] {
        assert!(Compression::configure(name), "{name}");
        let written = compress_records(&refs, 99)
            .pop()
            .expect("one record")
            .expect("an encoded record")
            .expect("a chunk, not a deletion")
            .record;
        assert_eq!(
            written.compression,
            Compression::selected(),
            "written with {name}"
        );
        // The bytes are that codec's, and read back as the chunk.
        assert_eq!(written.nbt(Limits::default()).unwrap(), *tag);
    }
    // Leave the default behind for the rest of this process's tests.
    assert!(Compression::configure("deflate"));
}

#[test]
fn records_compressed_on_helpers_match_one_thread_in_order() {
    use super::{Record, compress_records};
    let records: Vec<((i32, i32), Record)> = (0..53i32)
        .map(|i| {
            let record = if i % 7 == 3 {
                Record::Delete
            } else {
                let payload: Vec<i64> = (0..i * 97 % 2000).map(|v| i64::from(v * i)).collect();
                Record::Tag(Arc::new(NamedTag {
                    name: "".into(),
                    tag: Tag::compound([("xPos", Tag::Int(i)), ("Data", Tag::LongArray(payload))]),
                }))
            };
            ((i, -i), record)
        })
        .collect();
    let refs: Vec<_> = records.iter().map(|(k, r)| (*k, r)).collect();
    let compressed = compress_records(&refs, 1234);
    assert_eq!(compressed.len(), records.len());
    for ((_, record), got) in records.iter().zip(compressed) {
        let want = record
            .tag()
            .map(|t| ChunkRecord::from_nbt(&t, Compression::selected(), 1234, Limits::default()))
            .transpose()
            .unwrap();
        let got = got.unwrap().map(|r| r.record);
        assert_eq!(
            got.as_ref().map(|r| (&r.compressed, r.timestamp)),
            want.as_ref().map(|r| (&r.compressed, r.timestamp))
        );
    }
}

#[test]
fn older_worlds_are_upgraded_on_opening() {
    let dir = std::env::temp_dir().join(format!("sunrise-dv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let level = |v: i32| Tag::compound([("Data", Tag::compound([("DataVersion", Tag::Int(v))]))]);
    let version = |dir: &std::path::Path| {
        super::read_dat(&dir.join("level.dat"))
            .and_then(|r| super::number(r.get("Data")?.get("DataVersion")))
    };
    // 1.12.2 and 1.16.5: upgraded too, every server fix is ported. Data from before 1.13
    // needs the data fixers' tables of the prepared resources: without them (a checkout
    // nothing was prepared in, CI's verify) only 1.16.5 is upgraded here.
    let olds: &[i32] = if sunrise_world::datafix::tables_available() {
        &[1343, 2586]
    } else {
        &[2586]
    };
    for &old in olds {
        super::write_dat(&dir.join("level.dat"), level(old)).unwrap();
        super::upgrade_older_world(&dir).unwrap();
        assert_eq!(version(&dir), Some(i64::from(super::DATA_VERSION)));
    }
    // 1.21.1: upgraded to the current version.
    super::write_dat(&dir.join("level.dat"), level(3955)).unwrap();
    super::upgrade_older_world(&dir).unwrap();
    assert_eq!(version(&dir), Some(i64::from(super::DATA_VERSION)));
    super::upgrade_older_world(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn region_files_kept_open_stay_under_vanillas_cache_size() {
    let open_fds = || std::fs::read_dir("/dev/fd").map_or(0, |d| d.count());
    let dir = std::env::temp_dir().join(format!("sunrise-save-fds-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let save = super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
    let before = open_fds();
    let storage = save.region_storage(Folder::Region);
    let held = storage.open_cached((0, 0), true).unwrap().unwrap();
    for x in 1..=600 {
        storage.open_cached((x, 0), true).unwrap().unwrap();
    }
    // Other tests open files at the same time: a margin, far below 600.
    let grown = open_fds().saturating_sub(before);
    assert!(
        grown <= super::MAX_OPEN_REGIONS + 32,
        "{grown} descriptors more"
    );
    let cache = save.regions.lock().unwrap();
    assert!(
        cache.open() <= super::MAX_OPEN_REGIONS,
        "{} open",
        cache.open()
    );
    // The one still held is never closed under its holder.
    assert!(Arc::ptr_eq(
        &cache.regions[&(0, 0)].0.clone().unwrap(),
        &held
    ));
    drop(cache);
    // What was written to regions since closed reads back from the disk.
    let entity = |x: i32| Tag::compound([("x", Tag::Int(x))]);
    save.save_entities_later((0..300).map(|i| ((i * 32, 0), vec![entity(i)])).collect())
        .unwrap();
    save.flush().unwrap();
    assert!(save.entity_regions.lock().unwrap().open() <= super::MAX_OPEN_REGIONS);
    for i in [0, 1, 150, 299] {
        assert_eq!(save.load_entities(i * 32, 0).unwrap(), vec![entity(i)]);
    }
    drop(held);
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn write_completion_polls_without_waiting_and_keeps_latest_snapshot() {
    let dir = std::env::temp_dir().join(format!("sunrise-save-poll-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let save = super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
    let lock = super::SessionLock::acquire(&dir).unwrap();
    let region = save
        .region_storage(Folder::Entities)
        .open_cached((0, 0), true)
        .unwrap()
        .unwrap();
    let chunk_region = save
        .region_storage(Folder::Region)
        .open_cached((0, 0), true)
        .unwrap()
        .unwrap();
    // Hold the actual region locks, rather than relying on disk speed or sleeps. The
    // writer takes the Region folder before Entities in a pass, so both are held.
    let blocked = (region.lock().unwrap(), chunk_region.lock().unwrap());
    let entity = |version| Tag::compound([("version", Tag::Int(version))]);
    save.save_entities_later(vec![((1, 2), vec![entity(1)])])
        .unwrap();
    save.save_entities_later(vec![((1, 2), vec![entity(2)])])
        .unwrap();
    assert!(!save.writes_finished().unwrap());
    assert_eq!(save.active_chunk_writes(), 0); // entity batches do not count
    save.write_records("region", vec![((0, 0), None), ((1, 0), None)], false)
        .unwrap();
    // The writer is blocked above: two chunk records in ONE queued batch.
    assert_eq!(save.active_chunk_writes(), 2);
    assert_eq!(save.load_entities(1, 2).unwrap(), vec![entity(2)]);
    assert!(super::SessionLock::acquire(&dir).is_err());
    drop(blocked);
    save.flush().unwrap();
    assert!(save.writes_finished().unwrap());
    assert_eq!(save.active_chunk_writes(), 0);
    let reopened =
        super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
    assert_eq!(reopened.load_entities(1, 2).unwrap(), vec![entity(2)]);
    save.save_entities_later(vec![((1, 2), vec![])]).unwrap();
    save.flush().unwrap();
    assert!(
        super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD, None)
            .load_entities(1, 2)
            .unwrap()
            .is_empty()
    );
    drop((reopened, save, lock));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_failed_entity_read_does_not_clear_the_stored_record() {
    let dir = tempfile::tempdir().unwrap();
    let save = super::SaveDir::new(dir.path().to_owned(), LevelShape::OVERWORLD, None);
    let entity = Tag::compound([("id", Tag::String("minecraft:pig".into()))]);
    // Put matching records in two different regions.
    save.save_entities(vec![
        ((0, 0), vec![entity.clone()]),
        ((32, 0), vec![entity.clone()]),
    ])
    .unwrap();
    let region_b = save.region_path("entities", 1, 0);
    let saved = std::fs::read(&region_b).unwrap();
    let mut broken = saved.clone();
    let sector = u32::from_be_bytes(saved[..4].try_into().unwrap()) >> 8;
    // Fail decompression without changing the slot or its record length.
    broken[sector as usize * 4096 + 5] = 0;
    std::fs::write(&region_b, &broken).unwrap();
    assert_eq!(save.load_entities(0, 0).unwrap(), vec![entity.clone()]);
    assert!(save.load_entities(32, 0).is_err());
    assert!(save.load_entities(64, 0).unwrap().is_empty());
    std::fs::write(&region_b, &saved).unwrap();

    // Leaving and returning to the chunk does not restart the failed entity load.
    assert!(save.load_entities(32, 0).is_err());
    // The same save pass can clear a healthy chunk and must leave B alone.
    save.save_entities(vec![((0, 0), vec![]), ((32, 0), vec![])])
        .unwrap();
    drop(save);
    let reopened = super::SaveDir::new(dir.path().to_owned(), LevelShape::OVERWORLD, None);
    assert!(reopened.load_entities(0, 0).unwrap().is_empty());
    assert_eq!(reopened.load_entities(32, 0).unwrap(), vec![entity]);
}

#[test]
fn a_save_pass_resumes_at_the_write_watermark_instead_of_flushing() {
    let dir = std::env::temp_dir().join(format!("sunrise-save-watermark-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let save = std::sync::Arc::new(super::SaveDir::new(
        dir.clone(),
        super::LevelShape::OVERWORLD,
        Some(Default::default()),
    ));
    let lock = super::SessionLock::acquire(&dir).unwrap();
    // Hold the region lock so the writer stops there deterministically, rather
    // than relying on disk speed or sleeps: it keeps the lock while it writes.
    let region = save
        .region_storage(Folder::Region)
        .open_cached((0, 0), true)
        .unwrap()
        .unwrap();
    let blocked = region.lock().unwrap();
    let record = |x: i32| ((x, 0), None);
    save.write_records("region", (0..4).map(record).collect(), false)
        .unwrap();
    assert_eq!(save.active_chunk_writes(), 4);
    assert!(!save.writes_finished().unwrap());
    // The four queued records are at the watermark: a save pass resumes at
    // once and leaves them to the writer. On its own thread so that a pass
    // that waited like `flush` fails the test instead of hanging it.
    let (resumed, resume_result) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
    let resuming = {
        let save = save.clone();
        std::thread::spawn(move || {
            resumed
                .send(
                    save.wait_for_chunk_writes_below(4)
                        .map_err(|e| e.to_string()),
                )
                .unwrap()
        })
    };
    resume_result
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap();
    resuming.join().unwrap();
    assert_eq!(save.active_chunk_writes(), 4);
    assert!(!save.writes_finished().unwrap());
    // One below it, on that same queue, the wait does hold the caller, as a
    // flush does: only releasing the lock below can wake either.
    let (waited, wait_result) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
    let waiting = {
        let save = save.clone();
        std::thread::spawn(move || {
            waited
                .send(
                    save.wait_for_chunk_writes_below(3)
                        .map_err(|e| e.to_string()),
                )
                .unwrap()
        })
    };
    let (flushed, flush_result) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
    let flushing = {
        let save = save.clone();
        std::thread::spawn(move || {
            flushed
                .send(save.flush().map_err(|e| e.to_string()))
                .unwrap()
        })
    };
    let quiet = |rx: &std::sync::mpsc::Receiver<std::result::Result<(), String>>| {
        matches!(
            rx.recv_timeout(std::time::Duration::from_millis(200)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        )
    };
    assert!(quiet(&wait_result), "the wait returned below its watermark");
    assert!(quiet(&flush_result), "flush returned while blocked");
    drop(blocked);
    wait_result
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap();
    flush_result
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap();
    flushing.join().unwrap();
    waiting.join().unwrap();
    assert!(save.writes_finished().unwrap());
    assert_eq!(save.active_chunk_writes(), 0);
    drop((save, lock));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_async_io_failure_retires_the_pending_snapshot_without_retrying_it() {
    let dir = tempfile::tempdir().unwrap();
    // A file where a directory is needed causes a real writer I/O error.
    let blocked = dir.path().join("entities");
    std::fs::write(&blocked, b"blocked").unwrap();
    let save = super::SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD, None);
    let entity = Tag::compound([("version", Tag::Int(3))]);
    save.save_entities_later(vec![((0, 0), vec![entity])])
        .unwrap();
    save.flush().unwrap();
    assert!(save.writes_finished().unwrap());
    assert_eq!(save.record_save_failures(), 1);

    // Fix the directory without submitting another snapshot. A fresh read
    // does not see the unpersisted entity and a barrier does not retry it.
    std::fs::remove_file(&blocked).unwrap();
    save.flush().unwrap();
    assert!(save.load_entities(0, 0).unwrap().is_empty());
    assert!(!blocked.exists());
    assert_eq!(save.record_save_failures(), 1);
}

#[test]
fn session_lock_is_exclusive() {
    let dir = std::env::temp_dir().join(format!("sunrise-lock-{}", std::process::id()));
    let first = super::SessionLock::acquire(&dir).expect("first lock");
    assert!(super::SessionLock::acquire(&dir).is_err());
    drop(first);
    // Windows refuses a read through another handle while the file is
    // locked, so the bytes are read once the lock is released.
    assert_eq!(
        std::fs::read(dir.join("session.lock")).unwrap(),
        [0x26, 0x03]
    );
    assert!(super::SessionLock::acquire(&dir).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}
/// #749: a copy of the lock's descriptor (a child process spawned on
/// another thread holds one until it execs) must not keep a released
/// world locked.
#[test]
fn session_lock_releases_while_a_descriptor_copy_lives() {
    let dir = std::env::temp_dir().join(format!("sunrise-lock-copy-{}", std::process::id()));
    let lock = super::SessionLock::acquire(&dir).expect("first lock");
    let copy = lock.0.try_clone().unwrap();
    drop(lock);
    assert!(super::SessionLock::acquire(&dir).is_ok());
    drop(copy);
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn crafter_block_entity_round_trips() {
    let mut crafter = crate::containers::Crafter::default();
    crafter.items[2] = Some(crate::items::ItemStack::new("minecraft:stick", 5));
    crafter.disabled[4] = true;
    crafter.triggered = 1;
    crafter.crafting_ticks = 3;
    let tag = block_entity_encode::encode(
        (7, 8, 9),
        &crate::containers::BlockEntity::Crafter(crafter),
        "minecraft:crafter",
        &Default::default(),
    );
    let (pos, back) = decode_block_entity(&tag).expect("crafter");
    assert_eq!(pos, (7, 8, 9));
    let crate::containers::BlockEntity::Crafter(back) = back else {
        panic!("not a crafter");
    };
    assert_eq!(back.items[2].as_ref().map(|s| s.count), Some(5));
    assert!(back.disabled[4] && !back.disabled[3]);
    assert!(back.is_triggered());
    assert_eq!(back.crafting_ticks, 3);
    assert_eq!(back.redstone_signal(), 2);
}
/// `triggered` is an int vanilla saves as it is and calls triggered only at 1, so other
/// values come back unchanged and untriggered.
#[test]
fn crafter_triggered_keeps_the_whole_int() {
    for value in [0, 1, 5, -1, 65537] {
        let crafter = crate::containers::Crafter {
            triggered: value,
            ..Default::default()
        };
        let tag = block_entity_encode::encode(
            (0, 0, 0),
            &crate::containers::BlockEntity::Crafter(crafter),
            "minecraft:crafter",
            &Default::default(),
        );
        let (_, back) = decode_block_entity(&tag).expect("crafter");
        let crate::containers::BlockEntity::Crafter(back) = back else {
            panic!("not a crafter");
        };
        assert_eq!(back.triggered, value);
        assert_eq!(back.is_triggered(), value == 1, "triggered {value}");
    }
}
#[test]
fn crafter_triggered_double_load_floors_and_saturates() {
    for (value, expected) in [(-0.5_f64, -1), (1.5, 1), (4294967297.0, i32::MAX)] {
        let tag = Tag::compound([
            ("id", string("minecraft:crafter")),
            ("x", Tag::Int(0)),
            ("y", Tag::Int(0)),
            ("z", Tag::Int(0)),
            ("triggered", Tag::Double(value.to_bits())),
        ]);
        let (_, back) = decode_block_entity(&tag).expect("crafter");
        let crate::containers::BlockEntity::Crafter(back) = back else {
            panic!("not a crafter");
        };
        assert_eq!(back.triggered, expected, "triggered {value}d");
        assert_eq!(back.is_triggered(), expected == 1, "triggered {value}d");
    }
}
#[test]
fn campfire_block_entity_round_trips() {
    let mut c = crate::containers::Campfire::default();
    c.items[1] = Some(crate::items::ItemStack::new("minecraft:beef", 1));
    c.progress[1] = 120;
    c.total[1] = 600;
    let tag = block_entity_encode::encode(
        (3, 64, -2),
        &crate::containers::BlockEntity::Campfire(Box::new(c.clone())),
        "minecraft:soul_campfire",
        &Default::default(),
    );
    // One block entity type for both campfires.
    assert_eq!(text(tag.get("id")).as_deref(), Some("minecraft:campfire"));
    let (pos, back) = decode_block_entity(&tag).expect("campfire");
    assert_eq!(pos, (3, 64, -2));
    let crate::containers::BlockEntity::Campfire(back) = back else {
        panic!("not a campfire");
    };
    assert_eq!(*back, c);
    // The update tag a server's clients get: the items alone.
    let campfire = crate::containers::BlockEntity::Campfire(Box::new(c));
    let update =
        crate::lan::chunks::entity_tag(&campfire, false, &Default::default()).expect("update tag");
    assert!(update.get("Items").is_some() && update.get("CookingTimes").is_none());
}
/// `LecternBlockEntity.loadAdditional`: `Page` is clamped to
/// `0..getPageCount(book) - 1`, so a stored page outside the book loads as
/// the first or last page the block holds. A bookless lectern counts no
/// pages and loads page -1.
#[test]
fn a_lecterns_page_loads_within_its_book() {
    let page_of = |tag: &str| {
        let tag = sunrise_core::nbt::parse_snbt(tag).unwrap().0;
        let (_, entity) = decode_block_entity(&tag).expect("lectern");
        let crate::containers::BlockEntity::Lectern(l) = entity else {
            panic!("not a lectern");
        };
        l.page
    };
    let book = |pages: &str| {
        format!(
            r#"id:"minecraft:written_book",count:1,components:{{
                "minecraft:written_book_content":{{title:"t",author:"a",pages:[{pages}]}}
            }}"#
        )
    };
    // Two pages: a stored 7 is the last page, a stored -3 the first.
    assert_eq!(
        page_of(&format!(
            r#"{{id:"minecraft:lectern",x:1,y:64,z:2,Page:7,Book:{{{}}}}}"#,
            book(r#""one","two""#)
        )),
        1
    );
    assert_eq!(
        page_of(&format!(
            r#"{{id:"minecraft:lectern",x:1,y:64,z:2,Page:-3,Book:{{{}}}}}"#,
            book(r#""one","two""#)
        )),
        0
    );
    // No book counts no pages, so any page loads as -1, as vanilla's.
    assert_eq!(
        page_of(r#"{id:"minecraft:lectern",x:1,y:64,z:2,Page:4}"#),
        -1
    );
    // A page already inside the book is kept, and saved back as it is.
    let tag = format!(
        r#"{{id:"minecraft:lectern",x:1,y:64,z:2,Page:1,Book:{{{}}}}}"#,
        book(r#""one","two","three""#)
    );
    let tag = sunrise_core::nbt::parse_snbt(&tag).unwrap().0;
    let (pos, entity) = decode_block_entity(&tag).expect("lectern");
    let crate::containers::BlockEntity::Lectern(l) = entity else {
        panic!("not a lectern");
    };
    assert_eq!(l.page, 1);
    let saved = block_entity_encode::encode(
        pos,
        &crate::containers::BlockEntity::Lectern(l),
        "minecraft:lectern",
        &Default::default(),
    );
    assert_eq!(number(saved.get("Page")), Some(1));
}
#[test]
fn a_chest_loot_table_takes_precedence_over_saved_items() {
    let mut saved = sunrise_core::nbt::parse_snbt(
        r#"{id:"minecraft:chest",x:1,y:2,z:3,Items:[{Slot:3b,id:"minecraft:diamond",count:7}],LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:42L}"#,
    )
    .unwrap()
    .0;
    let (_, chest) = decode_block_entity(&saved).expect("chest");
    assert!(
        chest.slots().iter().all(Option::is_none),
        "ChestBlockEntity.loadAdditional ignores Items when a loot table loads"
    );
    // The command path keeps an empty inventory and the pending table/seed
    // for data get and saving. No loot resources are needed to keep that state.
    let pending = PendingContainerLoot::from_tag(&saved).unwrap();
    let stored = block_entity_encode::data(
        (1, 2, 3),
        &chest,
        "minecraft:chest",
        block_entity_encode::DataFields {
            pending: Some(&pending),
            ..Default::default()
        },
        &Default::default(),
    );
    assert_eq!(stored.get("LootTable"), saved.get("LootTable"));
    assert_eq!(stored.get("LootTableSeed"), saved.get("LootTableSeed"));
    assert!(stored.get("Items").is_none());
    let Tag::Compound(fields) = &mut saved else {
        panic!("compound");
    };
    fields.remove(&"LootTable".into());
    let (_, chest) = decode_block_entity(&saved).expect("chest without table");
    let item = chest.slots()[3].as_ref().expect("saved item");
    assert_eq!(item.item.as_ref(), "minecraft:diamond");
    assert_eq!(item.count, 7);
}

/// `tryLoadLootTable` in every `RandomizableContainer`'s `loadAdditional`
/// (barrel, shulker boxes, hopper, dispenser, dropper):
/// a valid table leaves the saved `Items` unread, as for the chest.
#[test]
fn every_randomizable_container_lets_its_loot_table_win_over_saved_items() {
    for (id, size) in [
        ("minecraft:barrel", 27),
        ("minecraft:shulker_box", 27),
        ("minecraft:white_shulker_box", 27),
        ("minecraft:hopper", crate::containers::HOPPER_SLOTS),
        ("minecraft:dispenser", 9),
        ("minecraft:dropper", 9),
    ] {
        let mut saved = sunrise_core::nbt::parse_snbt(&format!(
            r#"{{id:"{id}",x:1,y:2,z:3,Items:[{{Slot:3b,id:"minecraft:diamond",count:7}}],LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:42L}}"#
        ))
        .unwrap()
        .0;
        let (_, container) = decode_block_entity(&saved).expect(id);
        assert_eq!(container.slots().len(), size, "{id}");
        assert!(
            container.slots().iter().all(Option::is_none),
            "{id}: loadAdditional ignores Items when a loot table loads"
        );
        let pending = PendingContainerLoot::from_tag(&saved).unwrap();
        let stored = block_entity_encode::data(
            (1, 2, 3),
            &container,
            id,
            block_entity_encode::DataFields {
                pending: Some(&pending),
                ..Default::default()
            },
            &Default::default(),
        );
        assert_eq!(stored.get("LootTable"), saved.get("LootTable"), "{id}");
        assert_eq!(
            stored.get("LootTableSeed"),
            saved.get("LootTableSeed"),
            "{id}"
        );
        assert!(stored.get("Items").is_none(), "{id}");
        let Tag::Compound(fields) = &mut saved else {
            panic!("compound");
        };
        fields.remove(&"LootTable".into());
        let (_, container) = decode_block_entity(&saved).expect(id);
        let item = container.slots()[3].as_ref().expect("saved item");
        assert_eq!(item.item.as_ref(), "minecraft:diamond", "{id}");
        assert_eq!(item.count, 7, "{id}");
    }
}

#[test]
fn invalid_container_loot_tables_leave_saved_items_intact() {
    for id in [
        "minecraft:chest",
        "minecraft:trapped_chest",
        "minecraft:barrel",
        "minecraft:shulker_box",
        "minecraft:hopper",
        "minecraft:dispenser",
        "minecraft:dropper",
    ] {
        for invalid in ["INVALID TABLE", "bad:table/with spaces", "bad@id"] {
            let saved = sunrise_core::nbt::parse_snbt(&format!(
                r#"{{id:"{id}",x:1,y:2,z:3,Items:[{{Slot:0b,id:"minecraft:diamond",count:3}}],LootTable:"{invalid}"}}"#
            ))
            .unwrap()
            .0;
            let (_, container) = decode_block_entity(&saved).expect(id);
            let item = container.slots()[0]
                .as_ref()
                .expect("saved item with invalid table");
            assert_eq!(item.item.as_ref(), "minecraft:diamond");
            assert_eq!(item.count, 3);
        }
    }
}

/// `LockCode`: a chest's `lock` is not in Sunrise's model, so `data get`
/// takes it from the tag the block entity was loaded (or set) with.
#[test]
fn a_locked_chest_keeps_its_lock_through_decode_and_encode() {
    let saved = sunrise_core::nbt::parse_snbt(
        r#"{id:"minecraft:chest",x:1,y:2,z:3,lock:{items:"minecraft:gold_ingot"}}"#,
    )
    .unwrap()
    .0;
    let (pos, chest) = decode_block_entity(&saved).expect("chest");
    assert_eq!(chest.slots().len(), 27);
    assert_eq!(pos, (1, 2, 3));
    // The encoded entity has no field to carry the lock...
    let plain = block_entity_encode::encode(pos, &chest, "minecraft:chest", &Default::default());
    assert!(plain.get("lock").is_none());
    // ...so `data get` puts back the one it was loaded with.
    let data = block_entity_encode::data(
        pos,
        &chest,
        "minecraft:chest",
        block_entity_encode::DataFields {
            lock: saved.get("lock"),
            ..Default::default()
        },
        &Default::default(),
    );
    let lock = sunrise_core::nbt::parse_snbt(r#"{items:"minecraft:gold_ingot"}"#)
        .unwrap()
        .0;
    assert_eq!(data.get("lock"), Some(&lock));
}
#[test]
fn spawner_block_entity_round_trips() {
    // A dungeon spawner as vanilla saves it (`id` is mob_spawner, not the block).
    let saved = sunrise_core::nbt::parse_snbt(
        r#"{id:"minecraft:mob_spawner",x:67,y:-52,z:-64,Delay:20s,SpawnData:{entity:{id:"minecraft:zombie"}},SpawnPotentials:[]}"#,
    )
    .unwrap()
    .0;
    let (pos, back) = decode_block_entity(&saved).expect("spawner");
    assert_eq!(pos, (67, -52, -64));
    let crate::containers::BlockEntity::Spawner(spawner) = &back else {
        panic!("not a spawner");
    };
    let next = spawner.next_data().and_then(|d| d.id());
    assert_eq!(next.as_deref(), Some("minecraft:zombie"));
    let tag = block_entity_encode::encode(pos, &back, "minecraft:spawner", &Default::default());
    let (_, again) = decode_block_entity(&tag).expect("spawner again");
    let crate::containers::BlockEntity::Spawner(again) = again else {
        panic!("not a spawner");
    };
    assert_eq!(again.update_tag(), spawner.update_tag());
}
#[test]
#[ignore = "daily"]
fn native_saved_ids_must_match_the_declared_block_membership() {
    // The production registry is immutable. Exercise its real readers in a fresh process.
    const CHILD: &str = "SUNRISE_TEST_NATIVE_SAVED_MEMBERSHIP";
    if std::env::var_os(CHILD).is_none() {
        for mode in ["accepted", "revoked"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--include-ignored",
                    "--exact",
                    "save::tests::native_saved_ids_must_match_the_declared_block_membership",
                    "--nocapture",
                ])
                .env(CHILD, mode)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "test=save::tests::native_saved_ids_must_match_the_declared_block_membership exit_code={:?} category=fixture-child-failed",
                output.status.code()
            );
        }
        return;
    }
    use crate::block_entity_types::{self, BlockEntityTypes};
    let mut types = BlockEntityTypes::new(
        [
            ("test:press".into(), "test:machine".into()),
            ("test:mill".into(), "test:machine".into()),
            ("test:board".into(), "test:other".into()),
        ]
        .into(),
        vec![
            "test:machine".into(),
            "test:other".into(),
            "minecraft:chest".into(),
        ],
        ["test:machine".into(), "test:other".into()],
    );
    // Seed the same canonical pairs as production when resources are available.
    // The explicit shared-representation cases below also run without resources.
    let mut vanilla_blocks: Vec<String> = [
        "chest",
        "trapped_chest",
        "barrel",
        "dropper",
        "dispenser",
        "hopper",
        "shulker_box",
    ]
    .into_iter()
    .map(|name| format!("minecraft:{name}"))
    .collect();
    for color in [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ] {
        vanilla_blocks.push(format!("minecraft:{color}_shulker_box"));
    }
    if let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") {
        let path = sunrise_protocol::registry::Builtin::find(&PathBuf::from(root))
            .expect("registries.json");
        let builtin = sunrise_protocol::registry::Builtin::load(&path).unwrap();
        vanilla_blocks.extend(
            builtin
                .get("minecraft:block")
                .unwrap()
                .names
                .iter()
                .cloned(),
        );
    }
    for block in vanilla_blocks {
        if let Some(kind) = crate::lan::chunks::block_entity_type(&block) {
            types.template(block.clone(), block, kind.into());
        }
    }
    types.template(
        "minecraft:trapped_chest".into(),
        "minecraft:trapped_chest".into(),
        "minecraft:trapped_chest".into(),
    );
    let accepted = std::env::var(CHILD).unwrap() == "accepted";
    if accepted {
        types
            .add_valid("minecraft:trapped_chest", "minecraft:chest")
            .unwrap();
    }
    block_entity_types::install(types).unwrap();
    let tag = |id: &str| {
        sunrise_core::nbt::parse_snbt(&format!(r#"{{id:"{id}",x:1,y:64,z:2,energy:900L}}"#,))
            .unwrap()
            .0
    };
    for (block, kind) in block_entity_types::current().unwrap().memberships() {
        if block.starts_with("minecraft:") {
            let saved = tag(kind);
            // Some vanilla types have no client representation yet. Membership
            // must not discard any entity that the decoder can represent.
            assert_eq!(
                decode_block_entity_for_block(&saved, block).is_some(),
                decode_block_entity(&saved).is_some(),
                "{kind} on {block}"
            );
        }
    }
    assert_eq!(
        decode_block_entity_for_block(&tag("minecraft:chest"), "minecraft:trapped_chest").is_some(),
        accepted
    );
    assert!(decode_block_entity_for_block(&tag("minecraft:banner"), "minecraft:stone").is_none());
    assert!(
        decode_block_entity_for_block(&tag("minecraft:banner"), "minecraft:trapped_chest")
            .is_none()
    );
    for block in [
        "trapped_chest",
        "barrel",
        "dropper",
        "dispenser",
        "hopper",
        "shulker_box",
        "red_shulker_box",
    ] {
        let block = format!("minecraft:{block}");
        let kind = crate::lan::chunks::block_entity_type(&block).unwrap();
        assert!(
            decode_block_entity_for_block(&tag(kind), &block).is_some(),
            "{kind} on {block}"
        );
    }
    for block in ["test:press", "test:mill"] {
        let (_, entity) = decode_block_entity_for_block(&tag("test:machine"), block).unwrap();
        assert!(entity.fits(block));
        assert_eq!(entity.id(), "test:machine");
        let crate::containers::BlockEntity::Mod(m) = entity else {
            panic!("not native");
        };
        assert_eq!(m.nbt.get(&"energy".into()), Some(&Tag::Long(900)));
        for id in [
            "test:press",
            "test:other",
            "test:unknown",
            "minecraft:banner",
        ] {
            assert!(
                decode_block_entity_for_block(&tag(id), block).is_none(),
                "{id}"
            );
        }
    }
    for block in [
        "test:casing",
        "test:board",
        "other:press",
        "minecraft:stone",
    ] {
        assert!(decode_block_entity_for_block(&tag("test:machine"), block).is_none());
    }
    assert!(
        decode_block_entity_for_block(&tag("minecraft:banner"), "minecraft:white_banner").is_some()
    );
}

#[test]
fn queued_client_update_fields_are_not_saved_or_taken_by_a_save() {
    let saved = sunrise_core::nbt::parse_snbt(r#"{id:"test:machine",x:5,y:-60,z:12,progress:7}"#)
        .unwrap()
        .0;
    let (pos, entity) = decode_block_entity(&saved).unwrap();
    let crate::containers::BlockEntity::Mod(machine) = &entity else {
        panic!("not a mod's");
    };
    let batch = Compound::from([("effect".into(), Tag::Int(23))]);
    machine.set_client_update_fields(batch.clone());
    assert_eq!(
        block_entity_encode::encode(pos, &entity, "test:machine", &Default::default()),
        saved
    );
    assert_eq!(machine.take_client_update_fields(), Some(batch));
}

#[test]
fn published_shapes_are_saved_with_the_block_entity_and_come_back_on_load() {
    use crate::containers::{BlockEntity, PublishedShapes, SHAPES_KEY};
    let saved = sunrise_core::nbt::parse_snbt(r#"{id:"test:bus",x:5,y:-60,z:12,progress:7}"#)
        .unwrap()
        .0;
    let (pos, entity) = decode_block_entity(&saved).unwrap();
    let BlockEntity::Mod(mut bus) = entity else {
        panic!("not a mod's");
    };
    assert!(bus.shapes().is_none());
    let shapes = PublishedShapes::new(mod_api::BlockEntityShapes {
        outline: vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]],
        collision: vec![[-0.25, 0.0, 0.0, 1.0, 1.5, 1.0]],
    })
    .unwrap();
    bus.set_shapes(Some(std::sync::Arc::new(shapes)));
    let saved =
        block_entity_encode::encode(pos, &BlockEntity::Mod(bus), "test:bus", &Default::default());
    // Kept apart from the mod's own fields.
    let (_, back) = decode_block_entity(&saved).unwrap();
    let BlockEntity::Mod(back) = back else {
        panic!("not a mod's");
    };
    assert_eq!(back.nbt.len(), 1);
    assert!(!back.nbt.contains_key(&SHAPES_KEY.into()));
    let loaded = back.shapes().expect("restored on load");
    assert_eq!(loaded.boxes().collision, [[-0.25, 0.0, 0.0, 1.0, 1.5, 1.0]]);
    // Saved data that is over a limit loads without them.
    let Tag::Compound(mut bad) = saved else {
        unreachable!()
    };
    bad.insert(
        SHAPES_KEY.into(),
        Tag::Compound(Compound::from([
            ("outline".into(), Tag::LongArray(vec![0; 6 * 65])),
            ("collision".into(), Tag::LongArray(vec![])),
        ])),
    );
    let (_, bad) = decode_block_entity(&Tag::Compound(bad)).unwrap();
    let BlockEntity::Mod(bad) = bad else {
        panic!("not a mod's");
    };
    assert!(bad.shapes().is_none());
}

#[test]
fn mod_block_entity_items_claim_only_the_slots_written() {
    use crate::{
        containers::{BlockEntity, ModBlockEntity},
        items::ItemStack,
    };
    let parse = |s| sunrise_core::nbt::parse_snbt(s).unwrap().0;
    let saved = parse(
        r#"{id:"test:machine",x:0,y:64,z:0,Items:[{Slot:0b,raw:"superseded"},{Slot:1b,raw:"retained"},{raw:"no slot"}]}"#,
    );
    let (pos, BlockEntity::Mod(mut data)) = decode_block_entity(&saved).unwrap() else {
        panic!("expected mod block entity")
    };
    let encode = |data: &ModBlockEntity| {
        block_entity_encode::encode(
            pos,
            &BlockEntity::Mod(Box::new(data.clone())),
            "test:machine",
            &Default::default(),
        )
    };
    data.items.resize(2, None);
    *data.item_slot_mut(0) = Some(ItemStack::new("minecraft:stone", 2));
    let encoded = encode(&data);
    assert_eq!(
        encoded.get("Items"),
        Some(&parse(
            r#"[{Slot:0b,id:"minecraft:stone",count:2},{Slot:1b,raw:"retained"},{raw:"no slot"}]"#
        ))
    );
    let (_, BlockEntity::Mod(reloaded)) = decode_block_entity(&encoded).unwrap() else {
        panic!("expected mod block entity")
    };
    assert_eq!(reloaded.items[0], data.items[0]);
    assert_eq!(encode(&reloaded), encoded);
    *data.item_slot_mut(0) = None;
    // A tick reply replacing the mutable fields, including shrinking the
    // modeled vector to zero, cannot restore the old saved row for slot 0.
    let mut snapshot = (**data).clone();
    snapshot.items.clear();
    let revision = data.revision().changes() + 1;
    data.apply_snapshot(snapshot, revision);
    assert_eq!(
        encode(&data).get("Items"),
        Some(&parse(r#"[{Slot:1b,raw:"retained"},{raw:"no slot"}]"#))
    );
    data.items.resize(2, None);
    *data.item_slot_mut(1) = None;
    assert_eq!(
        encode(&data).get("Items"),
        Some(&parse(r#"[{raw:"no slot"}]"#))
    );
    let (_, BlockEntity::Mod(mixed)) = decode_block_entity(&parse(
        r#"{id:"test:machine",x:0,y:64,z:0,Items:[{Slot:0b,raw:"opaque"},{Slot:1b,id:"minecraft:stone",count:2}]}"#,
    ))
    .unwrap() else {
        panic!("expected mod block entity")
    };
    assert_eq!(
        mixed.items,
        [None, Some(ItemStack::new("minecraft:stone", 2))]
    );
    data.nbt.insert("Items".into(), parse("{energy:5}"));
    assert_eq!(encode(&data).get("Items"), Some(&parse("{energy:5}")));
    data.nbt.remove(&"Items".into());
    assert_eq!(encode(&data).get("Items"), None);
}

#[test]
fn mod_block_entity_round_trips_the_first_and_last_byte_indexed_slots() {
    use crate::{
        containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields},
        items::ItemStack,
    };

    let mut items = vec![None; 256];
    items[0] = Some(ItemStack::new("minecraft:stone", 2));
    items[255] = Some(ItemStack::new("minecraft:dirt", 3));
    let entity = BlockEntity::Mod(Box::new(ModBlockEntity::new(
        "test:machine".into(),
        ModBlockEntityFields {
            items: items.clone(),
            ..Default::default()
        },
    )));
    let saved =
        block_entity_encode::encode((0, 64, 0), &entity, "test:machine", &Default::default());
    let expected_rows = sunrise_core::nbt::parse_snbt(
        r#"[{Slot:0b,id:"minecraft:stone",count:2},{Slot:-1b,id:"minecraft:dirt",count:3}]"#,
    )
    .unwrap()
    .0;
    assert_eq!(saved.get("Items"), Some(&expected_rows));
    let (_, BlockEntity::Mod(reloaded)) = decode_block_entity(&saved).unwrap() else {
        panic!("expected mod block entity")
    };
    assert_eq!(reloaded.items, items);
}

/// A holder's capacity is its block's declaration, not the length of what it stores: a write
/// reaches its slot and no further, so a first write into a holder declared at the Java-int
/// maximum allocates the slots up to it, not the holder.
#[test]
fn a_slot_write_grows_a_mod_holder_only_to_that_slot() {
    let saved = sunrise_core::nbt::parse_snbt(r#"{id:"gtmod:vault",x:0,y:0,z:0}"#)
        .unwrap()
        .0;
    let (_, mut back) = decode_block_entity(&saved).expect("mod block entity");
    let crate::containers::BlockEntity::Mod(m) = &mut back else {
        panic!("not a mod's");
    };
    assert!(m.items.is_empty());
    *m.item_slot_mut(2) = Some(crate::items::ItemStack::new("minecraft:stone", 1));
    assert_eq!(m.items.len(), 3);
    assert!(m.items[0].is_none() && m.items[1].is_none());
    // Slots past the last written one read as empty without being allocated.
    assert!(m.items.get(i32::MAX as usize - 1).is_none());
}

#[test]
fn mod_block_entity_round_trips() {
    let saved = sunrise_core::nbt::parse_snbt(
        r#"{id:"gtmod:macerator",x:5,y:-60,z:12,energy:1200L,progress:7,Items:[{Slot:0b,id:"minecraft:iron_ore",count:3}]}"#,
    )
    .unwrap()
    .0;
    let (pos, back) = decode_block_entity(&saved).expect("mod block entity");
    assert_eq!(pos, (5, -60, 12));
    let crate::containers::BlockEntity::Mod(m) = &back else {
        panic!("not a mod's");
    };
    assert_eq!(m.id(), "gtmod:macerator");
    // The mod sees its own fields only.
    for key in ["id", "x", "y", "z"] {
        assert!(!m.nbt.contains_key(&key.into()), "{key}");
    }
    assert_eq!(m.nbt.get(&"energy".into()), Some(&Tag::Long(1200)));
    // `Items` are its slots, not the mod's compound.
    assert!(!m.nbt.contains_key(&"Items".into()));
    assert_eq!(m.items.len(), 1);
    assert_eq!(m.items[0].as_ref().map(|s| s.count), Some(3));
    assert_eq!(
        block_entity_encode::encode(pos, &back, "gtmod:macerator", &Default::default()),
        saved
    );
    // `Items` that are not an item list stay the mod's, as saved.
    for items in [
        r#"Items:{energy:5}"#,
        r#"Items:[1,2,3]"#,
        r#"Items:[{Slot:0b,id:"minecraft:air",count:1}]"#,
        r#"Items:[{Slot:0b,id:"minecraft:stone",count:1,note:"mine"}]"#,
    ] {
        let saved =
            sunrise_core::nbt::parse_snbt(&format!(r#"{{id:"gtmod:cable",x:0,y:0,z:0,{items}}}"#))
                .unwrap()
                .0;
        let (pos, back) = decode_block_entity(&saved).expect("mod block entity");
        let crate::containers::BlockEntity::Mod(m) = &back else {
            panic!("not a mod's");
        };
        assert!(m.items.is_empty(), "{items}");
        assert_eq!(
            block_entity_encode::encode(pos, &back, "gtmod:cable", &Default::default()),
            saved,
            "{items}"
        );
    }
    // Entries load by their own `Slot`, in any list order (`ContainerHelper.loadAllItems`).
    let saved = sunrise_core::nbt::parse_snbt(
        r#"{id:"gtmod:macerator",x:5,y:-60,z:12,Items:[{Slot:2b,id:"minecraft:stone",count:2},{Slot:0b,id:"minecraft:iron_ore",count:3}]}"#,
    )
    .unwrap()
    .0;
    let (pos, back) = decode_block_entity(&saved).expect("mod block entity");
    let crate::containers::BlockEntity::Mod(m) = &back else {
        panic!("not a mod's");
    };
    assert!(!m.nbt.contains_key(&"Items".into()));
    assert_eq!(m.items.len(), 3);
    assert_eq!(
        m.items[0].as_ref().map(|s| (&*s.item, s.count)),
        Some(("minecraft:iron_ore", 3))
    );
    assert!(m.items[1].is_none());
    assert_eq!(
        m.items[2].as_ref().map(|s| (&*s.item, s.count)),
        Some(("minecraft:stone", 2))
    );
    // Saved again, they are written in slot order.
    let reordered = sunrise_core::nbt::parse_snbt(
        r#"{id:"gtmod:macerator",x:5,y:-60,z:12,Items:[{Slot:0b,id:"minecraft:iron_ore",count:3},{Slot:2b,id:"minecraft:stone",count:2}]}"#,
    )
    .unwrap()
    .0;
    assert_eq!(
        block_entity_encode::encode(pos, &back, "gtmod:macerator", &Default::default()),
        reordered
    );
    // A lossless slot is modeled even beside an opaque row. Duplicate
    // slot indices stay opaque because the engine cannot model both stacks.
    let duplicate =
        r#"Items:[{Slot:1b,id:"minecraft:stone",count:1},{Slot:1b,id:"minecraft:dirt",count:1}]"#;
    let modeled = [Some(crate::items::ItemStack::new("minecraft:stone", 1))];
    let parse = |items| {
        sunrise_core::nbt::parse_snbt(&format!(r#"{{id:"gtmod:cable",x:0,y:0,z:0,{items}}}"#))
            .unwrap()
            .0
    };
    for (input, output, slots) in [
        (
            r#"Items:[{Slot:1b,id:"minecraft:stone",count:1,note:"mine"},{Slot:0b,id:"minecraft:stone",count:1}]"#,
            r#"Items:[{Slot:0b,id:"minecraft:stone",count:1},{Slot:1b,id:"minecraft:stone",count:1,note:"mine"}]"#,
            modeled.as_slice(),
        ),
        (duplicate, duplicate, &[]),
    ] {
        let (pos, back) = decode_block_entity(&parse(input)).expect("mod block entity");
        let crate::containers::BlockEntity::Mod(m) = &back else {
            panic!("not a mod's");
        };
        assert_eq!(m.items.as_slice(), slots, "{input}");
        assert_eq!(
            block_entity_encode::encode(pos, &back, "gtmod:cable", &Default::default()),
            parse(output),
            "{input}"
        );
    }
    // Vanilla's types are never a mod's, modelled or not.
    let ender = sunrise_core::nbt::parse_snbt(r#"{id:"minecraft:ender_chest",x:0,y:0,z:0}"#)
        .unwrap()
        .0;
    assert!(!matches!(
        decode_block_entity(&ender),
        Some((_, crate::containers::BlockEntity::Mod(_)))
    ));
}

#[test]
fn mod_block_entity_item_slots_round_trip_as_unsigned_bytes() {
    use crate::{
        containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields},
        items::ItemStack,
    };
    let parse = |s| sunrise_core::nbt::parse_snbt(s).unwrap().0;
    let pos = (4, 64, -2);
    let stone = ItemStack::new("minecraft:stone", 3);
    let iron = ItemStack::new("minecraft:iron_ingot", 4);
    let dirt = ItemStack::new("minecraft:dirt", 7);

    // A container holds at most 256 slots (`Registrar::block_container`): its rows carry
    // vanilla's unsigned-byte slot, 255 as -1b.
    let mut items = vec![None; 256];
    items[0] = Some(stone.clone());
    items[255] = Some(iron.clone());
    let entity = ModBlockEntity::new(
        "test:vault".into(),
        ModBlockEntityFields {
            items,
            ..Default::default()
        },
    );
    let encoded = block_entity_encode::encode(
        pos,
        &BlockEntity::Mod(Box::new(entity)),
        "test:vault",
        &Default::default(),
    );
    assert_eq!(
        encoded.get("Items"),
        Some(&parse(
            r#"[{Slot:0b,id:"minecraft:stone",count:3},{Slot:-1b,id:"minecraft:iron_ingot",count:4}]"#
        ))
    );

    // Reloading with the declared shape restores slots 0 and 255.
    let reloaded =
        decode_mod_block_entity("test:vault".into(), &encoded, 256).expect("declared mod vault");
    assert_eq!(reloaded.items.len(), 256);
    assert_eq!(reloaded.items[0].as_ref(), Some(&stone));
    assert_eq!(reloaded.items[255].as_ref(), Some(&iron));
    assert!(reloaded.nbt.get(&"Items".into()).is_none());
    assert_eq!(
        block_entity_encode::encode(
            pos,
            &BlockEntity::Mod(Box::new(reloaded)),
            "test:vault",
            &Default::default(),
        ),
        encoded
    );

    // Rows outside the declared shape, slots past a byte, Int slots that are not the byte
    // form, duplicates and slotless entries all remain opaque verbatim.
    let mixed = parse(
        r#"{id:"test:vault",x:4,y:64,z:-2,Items:[{Slot:0b,id:"minecraft:stone",count:3},{Slot:256,id:"minecraft:dirt",count:7},{Slot:-1,id:"minecraft:coal",count:1},{Slot:1,id:"minecraft:sand",count:1},{Slot:5b,id:"minecraft:dirt",count:1},{Slot:5b,id:"minecraft:dirt",count:2},{Slot:9b,id:"minecraft:dirt",count:1},{raw:"no slot"}]}"#,
    );
    let bounded = decode_mod_block_entity("test:vault".into(), &mixed, 9).expect("bounded vault");
    assert_eq!(bounded.items[0].as_ref(), Some(&stone));
    assert!(bounded.items.iter().skip(1).all(Option::is_none));
    assert_eq!(
        bounded.nbt.get(&"Items".into()),
        Some(&parse(
            r#"[{Slot:256,id:"minecraft:dirt",count:7},{Slot:-1,id:"minecraft:coal",count:1},{Slot:1,id:"minecraft:sand",count:1},{Slot:5b,id:"minecraft:dirt",count:1},{Slot:5b,id:"minecraft:dirt",count:2},{Slot:9b,id:"minecraft:dirt",count:1},{raw:"no slot"}]"#
        ))
    );

    // Slotless declared shape keeps Items verbatim in nbt.
    let slotless = decode_mod_block_entity("test:vault".into(), &encoded, 0).expect("slotless");
    assert!(slotless.items.is_empty());
    assert_eq!(slotless.nbt.get(&"Items".into()), encoded.get("Items"));

    // Opaque slot queries and claims: writing slot 0 takes over its opaque row.
    let opaque = parse(
        r#"{id:"test:vault",x:4,y:64,z:-2,Items:[{Slot:0b,raw:"low"},{Slot:256,raw:"high"}]}"#,
    );
    let mut claimed =
        decode_mod_block_entity("test:vault".into(), &opaque, 256).expect("opaque rows");
    assert!(claimed.has_opaque_item_slot(0));
    claimed.items.resize(256, None);
    *claimed.item_slot_mut(0) = Some(dirt.clone());
    assert!(!claimed.has_opaque_item_slot(0));
    assert_eq!(
        block_entity_encode::encode(
            pos,
            &BlockEntity::Mod(Box::new(claimed)),
            "test:vault",
            &Default::default(),
        )
        .get("Items"),
        Some(&parse(
            r#"[{Slot:0b,id:"minecraft:dirt",count:7},{Slot:256,raw:"high"}]"#
        ))
    );

    // Vanilla containers stay byte-exact.
    let mut chest_slots = vec![None; 27];
    chest_slots[0] = Some(stone);
    chest_slots[1] = Some(dirt);
    let chest = block_entity_encode::encode(
        pos,
        &BlockEntity::Chest(chest_slots),
        "minecraft:chest",
        &Default::default(),
    );
    assert_eq!(
        chest.get("Items"),
        Some(&parse(
            r#"[{Slot:0b,id:"minecraft:stone",count:3},{Slot:1b,id:"minecraft:dirt",count:7}]"#
        ))
    );
}

/// `CopperGolemStatueBlockEntity.createStatue`: the golem's styled custom
/// name lives in the statue's components, through every weathering stage,
/// a save and its item.
#[test]
fn copper_golem_statue_keeps_the_golem_name() {
    use crate::containers::BlockEntity;
    let name = Tag::compound([
        ("text", string("Bob")),
        ("color", string("gold")),
        ("bold", Tag::Byte(1)),
    ]);
    let components: Compound = [("minecraft:custom_name".into(), name.clone())].into();
    for block in [
        "minecraft:copper_golem_statue",
        "minecraft:exposed_copper_golem_statue",
        "minecraft:weathered_copper_golem_statue",
        "minecraft:oxidized_copper_golem_statue",
        "minecraft:waxed_oxidized_copper_golem_statue",
    ] {
        // Scraping and waxing swap the block, so every stage keeps the entity.
        let entity = BlockEntity::for_block(block).expect(block);
        assert!(entity.fits(block), "{block}");
        let BlockEntity::CopperGolemStatue(_) = entity else {
            panic!("{block}: not a statue");
        };
    }
    let entity = BlockEntity::CopperGolemStatue(components.clone());
    let tag = block_entity_encode::encode(
        (1, 2, 3),
        &entity,
        "minecraft:oxidized_copper_golem_statue",
        &Default::default(),
    );
    assert_eq!(
        tag.get("id"),
        Some(&string("minecraft:copper_golem_statue"))
    );
    let (pos, back) = decode_block_entity(&tag).expect("statue");
    assert_eq!(pos, (1, 2, 3));
    let BlockEntity::CopperGolemStatue(back) = back else {
        panic!("not a statue");
    };
    assert_eq!(back, components);
    // The statue item carries the name (`getItem`: collectComponents).
    let saved = crate::block_entity_nbt::canonical(&tag).expect("canonical statue");
    let patch = crate::block_entity_item::pick(&saved);
    assert_eq!(patch.get(&"minecraft:custom_name".into()), Some(&name));
    // Placing that item gives the entity the name back.
    let fresh = crate::block_entity_nbt::canonical(&block_entity_encode::encode(
        (1, 2, 3),
        &BlockEntity::CopperGolemStatue(Compound::new()),
        "minecraft:copper_golem_statue",
        &Default::default(),
    ))
    .expect("canonical statue");
    let placed = crate::block_entity_item::place(&fresh, &patch, false);
    let (_, placed) = decode_block_entity(&placed).expect("placed statue");
    assert!(matches!(placed, BlockEntity::CopperGolemStatue(c) if c == components));
}
#[test]
fn banner_block_entity_round_trips() {
    let banner = crate::containers::Banner {
        layers: vec![crate::banner::BannerLayer::new("minecraft:creeper", "lime")],
        name: Some("Flag".into()),
        components: [("minecraft:rarity".into(), string("uncommon"))].into(),
    };
    let tag = block_entity_encode::encode(
        (1, 2, 3),
        &crate::containers::BlockEntity::Banner(banner.clone()),
        "minecraft:red_banner",
        &Default::default(),
    );
    let (pos, back) = decode_block_entity(&tag).expect("banner");
    assert_eq!(pos, (1, 2, 3));
    let crate::containers::BlockEntity::Banner(back) = back else {
        panic!("not a banner");
    };
    assert_eq!(back.layers, banner.layers);
    assert_eq!(back.name, banner.name);
    assert_eq!(back.components, banner.components);
}
use super::*;
use crate::chunk::{MIN_Y, SECTIONS};

/// `TheEndGatewayBlockEntity.loadAdditional` reads `ExactTeleport` with
/// `getBooleanOr`: a number is true when its byte value is not 0, so an
/// int 256 is false and 257 true.
#[test]
fn an_end_gateways_exact_teleport_is_read_as_a_byte() {
    let exact =
        |saved: Tag| decode_end_gateway(&Tag::compound([("ExactTeleport", saved)])).exact_teleport;
    assert!(exact(Tag::Byte(1)));
    assert!(!exact(Tag::Byte(0)));
    assert!(!exact(Tag::Int(256)));
    assert!(exact(Tag::Int(257)));
    assert!(!decode_end_gateway(&Tag::compound([("Age", Tag::Long(1))])).exact_teleport);
}

#[test]
fn a_mods_fluid_ticks_are_saved_by_name() {
    use sunrise_world::fluid::{FluidKind, ModFluidRules, ModFluidType, register_mod_fluid};
    let fluid = register_mod_fluid(ModFluidType {
        id: "savetest:oil".into(),
        flowing_id: "savetest:flowing_oil".into(),
        block_id: "savetest:oil".into(),
        bucket: None,
        rules: ModFluidRules {
            tick_delay: 5,
            drop_off: 1,
            slope_find_distance: 4,
            converts_to_source: false,
        },
    });
    let fluid = fluid.unwrap();
    let tick = |kind: FluidKind| SavedTick {
        kind: sunrise_core::ticks::TickTypeId(kind.tick_id()),
        pos: BlockPos::new(1, 64, 2),
        delay: 3,
        priority: sunrise_core::ticks::TickPriority::Normal,
    };
    let ticks = [
        tick(FluidKind::ModFlowing(fluid)),
        tick(FluidKind::Mod(fluid)),
        tick(FluidKind::FlowingWater),
        tick(FluidKind::Lava),
    ];
    let tag = tick_list(&ticks, fluid_name);
    let names: Vec<_> = match &tag {
        Tag::List(list) => list.iter().filter_map(|t| text(t.get("i"))).collect(),
        _ => Vec::new(),
    };
    assert_eq!(
        names,
        [
            "savetest:flowing_oil",
            "savetest:oil",
            "minecraft:flowing_water",
            "minecraft:lava"
        ]
    );
    let back = read_ticks(Some(&tag), (0, 0), fluid_id);
    assert_eq!(
        back.iter().map(|t| t.kind).collect::<Vec<_>>(),
        ticks.iter().map(|t| t.kind).collect::<Vec<_>>()
    );
}

#[test]
fn end_gateway_block_entity_round_trips() {
    use crate::containers::{BlockEntity, EndGateway};
    let plain = BlockEntity::for_block("minecraft:end_gateway").expect("gateway");
    assert!(matches!(&plain, BlockEntity::EndGateway(g) if *g == EndGateway::default()));
    for exit in [None, Some([8, 100, 0])] {
        for exact in [false, true] {
            let gateway = EndGateway {
                age: 201,
                exit_portal: exit,
                exact_teleport: exact,
            };
            let tag = block_entity_encode::encode(
                (702, 100, 0),
                &BlockEntity::EndGateway(gateway.clone()),
                "minecraft:end_gateway",
                &Default::default(),
            );
            assert_eq!(
                text(tag.get("id")).as_deref(),
                Some("minecraft:end_gateway")
            );
            assert_eq!(tag.get("Age"), Some(&Tag::Long(201)));
            assert_eq!(
                tag.get("exit_portal"),
                exit.map(|p| Tag::IntArray(p.to_vec())).as_ref()
            );
            assert_eq!(
                tag.get("ExactTeleport"),
                exact.then_some(Tag::Byte(1)).as_ref()
            );
            let (pos, back) = decode_block_entity(&tag).expect("gateway");
            assert_eq!(pos, (702, 100, 0));
            assert!(matches!(back, BlockEntity::EndGateway(g) if g == gateway));
        }
    }
}

#[test]
fn end_gateway_exit_requires_spawnable_bounds() {
    for (exit, valid) in [
        (vec![-30_000_000, -20_000_000, -30_000_000], true),
        (vec![29_999_999, 19_999_999, 29_999_999], true),
        (vec![30_000_000, 0, 0], false),
        (vec![0, 0, 30_000_000], false),
        (vec![-30_000_001, 0, 0], false),
        (vec![0, 0, -30_000_001], false),
        (vec![0, 20_000_000, 0], false),
        (vec![0, -20_000_001, 0], false),
        (vec![i32::MIN, 0, 0], false),
        (vec![0, 100], false),
    ] {
        let tag = Tag::compound([
            ("id", string("minecraft:end_gateway")),
            ("x", Tag::Int(702)),
            ("y", Tag::Int(100)),
            ("z", Tag::Int(0)),
            ("exit_portal", Tag::IntArray(exit)),
        ]);
        let (_, crate::containers::BlockEntity::EndGateway(g)) = decode_block_entity(&tag).unwrap()
        else {
            panic!("gateway");
        };
        assert_eq!(g.exit_portal.is_some(), valid);
        let canonical = crate::block_entity_nbt::canonical(&tag).expect("gateway NBT");
        assert_eq!(canonical.get("exit_portal").is_some(), valid);
    }
    assert_eq!(
        decode_end_gateway(&Tag::Compound(Compound::new())),
        crate::containers::EndGateway::default()
    );
}

#[test]
fn skull_block_entity_keeps_its_profile() {
    // A placed `player_head` as vanilla saves it: the owning player's
    // `profile` survives load, encode and `canonical`.
    let saved = sunrise_core::nbt::parse_snbt(
        r#"{id:"minecraft:skull",x:10,y:64,z:20,profile:{name:"Steve",id:[I;1,2,3,4],properties:[{name:"textures",value:"abc"}]},note_block_sound:"minecraft:entity.player.hurt",custom_name:"Trophy"}"#,
    )
    .unwrap()
    .0;
    let (pos, back) = decode_block_entity(&saved).expect("skull");
    assert_eq!(pos, (10, 64, 20));
    let crate::containers::BlockEntity::Skull(skull) = &back else {
        panic!("not a skull");
    };
    assert_eq!(skull.profile, saved.get("profile").cloned());
    assert_eq!(
        skull.note_block_sound.as_deref(),
        Some("minecraft:entity.player.hurt")
    );
    assert_eq!(back.id(), "minecraft:skull");
    let tag = block_entity_encode::encode(pos, &back, "minecraft:player_head", &Default::default());
    assert_eq!(
        crate::block_entity_nbt::id(&tag).as_deref(),
        Some("minecraft:skull")
    );
    assert_eq!(tag.get("profile"), saved.get("profile"));
    let (_, again) = decode_block_entity(&tag).expect("skull again");
    let crate::containers::BlockEntity::Skull(again) = again else {
        panic!("not a skull");
    };
    assert_eq!(again.profile, skull.profile);
    // A profile-less head (skeleton, creeper) round-trips too.
    let plain = block_entity_encode::encode(
        pos,
        &crate::containers::BlockEntity::Skull(crate::containers::Skull::default()),
        "minecraft:creeper_head",
        &Default::default(),
    );
    let (_, decoded) = decode_block_entity(&plain).expect("plain skull");
    assert!(matches!(decoded, crate::containers::BlockEntity::Skull(_)));
    // The heads and skulls have one; the piston's head is no skull.
    use crate::containers::BlockEntity;
    assert!(matches!(
        BlockEntity::for_block("minecraft:player_wall_head"),
        Some(BlockEntity::Skull(_))
    ));
    assert!(BlockEntity::for_block("minecraft:piston_head").is_none());
}

#[test]
fn uuid_text_round_trips() {
    let u = [-158327916, 1572358581, -2125825222, -487948715];
    assert_eq!(uuid_from_text(&uuid_text(u)), Some(u));
    assert_eq!(
        uuid_text([0, 1, 2, 3]),
        "00000000-0000-0001-0000-000200000003"
    );
    let r = random_uuid();
    assert_eq!((r[1] >> 12) & 0xF, 4, "version 4");
}
#[test]
fn random_uuids_made_at_once_differ() {
    let made: std::collections::HashSet<[i32; 4]> = (0..10_000).map(|_| random_uuid()).collect();
    assert_eq!(made.len(), 10_000);
}
#[test]
fn creaking_heart_keeps_its_protector_uuid() {
    use crate::containers::{BlockEntity, CreakingHeart};
    let heart = CreakingHeart {
        creaking: Some((Some(7), [1, 2, 3, 4])),
        ..Default::default()
    };
    let tag = block_entity_encode::encode(
        (3, 70, -5),
        &BlockEntity::CreakingHeart(Box::new(heart)),
        "minecraft:creaking_heart",
        &Default::default(),
    );
    assert_eq!(
        tag.get("creaking"),
        Some(&Tag::IntArray(vec![1, 2, 3, 4])),
        "vanilla stores the protector's UUID under `creaking`"
    );
    let (pos, entity) = decode_block_entity(&tag).expect("heart decodes");
    assert_eq!(pos, (3, 70, -5));
    let BlockEntity::CreakingHeart(h) = entity else {
        panic!("a creaking heart");
    };
    // The entity id is resolved again once the creaking is loaded.
    assert_eq!(h.creaking, Some((None, [1, 2, 3, 4])));
    let lonely = block_entity_encode::encode(
        (0, 0, 0),
        &BlockEntity::CreakingHeart(Box::default()),
        "minecraft:creaking_heart",
        &Default::default(),
    );
    assert!(lonely.get("creaking").is_none());
}
#[test]
fn light_nibbles_use_vanilla_order() {
    let mut a = Box::new([0u8; 4096]);
    a[0] = 0xF3; // sky 15, block 3 at x=0
    a[1] = 0x52; // sky 5, block 2 at x=1
    let light = Light::Full(a);
    assert_eq!(nibbles(&light, false)[0], 0x23);
    assert_eq!(nibbles(&light, true)[0], 0x5F);
}
/// The independent reference: parse the saved layers and let
/// `crate::light::light_from_layers` expand and repack them.
fn reference(tag: &Tag, chunk: &mut Chunk, sky_light: bool) {
    if tag.get("isLightOn") != Some(&Tag::Byte(1)) {
        return;
    }
    let Some(Tag::List(sections)) = tag.get("sections") else {
        return;
    };
    let mut layers: Vec<(Option<&[u8]>, Option<&[u8]>)> = vec![(None, None); SECTIONS + 2];
    for section in sections {
        let Some(y) = number(section.get("Y")) else {
            continue;
        };
        let si = y as i32 - (MIN_Y >> 4) + 1;
        if !(0..SECTIONS as i32 + 2).contains(&si) {
            continue;
        }
        let layer = |name: &str| match section.get(name) {
            Some(Tag::ByteArray(a)) if a.len() == 2048 => Some(&a[..]),
            _ => None,
        };
        layers[si as usize] = (layer("SkyLight"), layer("BlockLight"));
    }
    crate::light::light_from_layers(chunk, &layers, sky_light);
}

#[test]
fn stored_light_loads_with_vanilla_defaults() {
    let mut sky = vec![0u8; 2048];
    sky[0] = 0x5A; // x=0 -> 10, x=1 -> 5
    let section = |y: i8, entries: Vec<(&str, Tag)>| {
        let mut all = vec![("Y", Tag::Byte(y))];
        all.extend(entries);
        Tag::compound(all)
    };
    let tag = Tag::compound([
        ("isLightOn", Tag::Byte(1)),
        (
            "sections",
            Tag::List(vec![
                section(
                    -4,
                    vec![
                        ("SkyLight", Tag::ByteArray(sky)),
                        ("BlockLight", Tag::ByteArray(vec![0x21; 2048])),
                    ],
                ),
                // No sky layer: repeats the bottom layer of the section above.
                section(-3, vec![]),
            ]),
        ),
    ]);
    let mut chunk = Chunk::empty(0, 0);
    load_light(&tag, &mut chunk, true);
    assert!(chunk.lit && chunk.light_loaded);
    let at = |c: &Chunk, si: usize, i: usize| c.sections[si].light(i);
    assert_eq!(at(&chunk, 0, 0), 0xA1, "stored sky 10, block 1");
    assert_eq!(at(&chunk, 0, 1), 0x52, "stored sky 5, block 2");
    // Section -3 has no layers; above it (-2..) sky defaults to 15.
    assert_eq!(at(&chunk, 1, 0), 0xF0);
    assert_eq!(at(&chunk, SECTIONS - 1, 5), 0xF0);
    let mut dark = Chunk::empty(0, 0);
    load_light(&tag, &mut dark, false);
    assert_eq!(at(&dark, 1, 0), 0x00, "no skylight dimension");
    let mut unlit = Chunk::empty(0, 0);
    load_light(
        &Tag::compound([("isLightOn", Tag::Byte(0))]),
        &mut unlit,
        true,
    );
    assert!(!unlit.lit && !unlit.light_loaded);
}
#[test]
#[ignore = "daily"]
fn both_light_pads_survive_full_chunk_save_and_reload() {
    let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let assets = crate::assets::Assets::load(&root).unwrap();
    let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        root,
    ))
    .unwrap();
    let mut original = Chunk::empty(0, 0);
    let mut layers = vec![(None, None); SECTIONS + 2];
    layers[0] = (None, Some(vec![0x55; 2048]));
    layers[SECTIONS + 1] = (Some(vec![0x27; 2048]), Some(vec![0; 2048]));
    crate::light::light_from_layers(&mut original, &layers, true);
    let saved = encode(&original, assets.registry, &biomes, &Default::default());
    let Some(Tag::List(sections)) = saved.tag.get("sections") else {
        panic!("missing saved sections");
    };
    assert_eq!(sections.len(), SECTIONS + 2);
    for section in [sections.first().unwrap(), sections.last().unwrap()] {
        assert!(section.get("block_states").is_none());
        assert!(section.get("biomes").is_none());
    }
    let mut loaded = decode(&saved, assets.registry, &biomes).unwrap();
    load_light(&saved.tag, &mut loaded, true);
    assert_eq!(loaded.light(0, MIN_Y - 1, 0), 0x75);
    assert_eq!(loaded.light(1, MIN_Y - 1, 0), 0x25);
    assert_eq!(loaded.light(0, 70, 0), 0x70);
    assert_eq!(loaded.light(1, 70, 0), 0x20);
    assert_eq!(loaded.light(0, loaded.max_y(), 0), 0x70);
    assert_eq!(loaded.light(1, loaded.max_y(), 0), 0x20);
    assert_eq!(loaded.stored_light_layers(0), Some([false, true]));
    assert_eq!(loaded.stored_light_layers(SECTIONS + 1), Some([true, true]));
}

#[test]
#[ignore = "daily"]
fn saved_light_matches_reference_for_layers_and_metadata() {
    let section = |y: Tag, sky: Option<Tag>, block: Option<Tag>| {
        let mut entries = vec![("Y", y)];
        if let Some(v) = sky {
            entries.push(("SkyLight", v));
        }
        if let Some(v) = block {
            entries.push(("BlockLight", v));
        }
        Tag::compound(entries)
    };
    let root = |sections| {
        Tag::compound([
            ("isLightOn", Tag::Byte(1)),
            ("sections", Tag::List(sections)),
        ])
    };
    let check = |tag: &Tag| {
        for sky_light in [false, true] {
            let mut expected = Chunk::empty(-3, 5);
            let s = Arc::make_mut(&mut expected.sections[0]);
            s.light = Light::Full(Box::new([0xab; 4096]));
            s.blocks.set(0, 1);
            s.blocks.set(0, 0);
            s.biomes.set(0, 1);
            s.biomes.set(0, 0);
            let mut actual = expected.clone();
            reference(tag, &mut expected, sky_light);
            load_light(tag, &mut actual, sky_light);
            assert_eq!(
                (actual.lit, actual.light_loaded),
                (expected.lit, expected.light_loaded)
            );
            for (a, b) in actual.sections.iter().zip(&expected.sections) {
                assert_eq!(
                    std::mem::discriminant(&a.light),
                    std::mem::discriminant(&b.light)
                );
                assert_eq!(
                    format!("{:?}", a.blocks.to_wire()),
                    format!("{:?}", b.blocks.to_wire())
                );
                assert_eq!(
                    format!("{:?}", a.biomes.to_wire()),
                    format!("{:?}", b.biomes.to_wire())
                );
                for i in 0..4096 {
                    assert_eq!(a.light(i), b.light(i), "index {i}, sky {sky_light}");
                }
            }
        }
    };
    for tag in [
        Tag::compound(Vec::<(&str, Tag)>::new()),
        Tag::compound([("isLightOn", Tag::Byte(0))]),
        Tag::compound([("isLightOn", Tag::Byte(2))]),
        Tag::compound([("isLightOn", Tag::Int(1)), ("sections", Tag::List(vec![]))]),
        Tag::compound([("isLightOn", Tag::Byte(1)), ("sections", Tag::Int(0))]),
        root(vec![]),
    ] {
        check(&tag);
    }
    // Cover every repeated byte, including 0xF0/0x0F: a uniform byte
    // array need not hold uniform nibbles.
    for byte in 0..=255u8 {
        check(&root(vec![section(
            Tag::Byte(19),
            Some(Tag::ByteArray(vec![byte; 2048])),
            Some(Tag::ByteArray(vec![255 - byte; 2048])),
        )]));
    }
    // Every pair of uniform sky/block nibble values, with missing
    // sections below inheriting the stored bottom row.
    for sky in 0..16u8 {
        for block in 0..16u8 {
            check(&root(vec![section(
                Tag::Byte(1),
                Some(Tag::ByteArray(vec![sky * 17; 2048])),
                Some(Tag::ByteArray(vec![block * 17; 2048])),
            )]));
        }
    }
    for length in [0, 1, 2047, 2048, 2049] {
        for invalid in [
            Tag::Int(1),
            Tag::List(vec![]),
            Tag::ByteArray(vec![0xf0; length]),
        ] {
            check(&root(vec![section(
                Tag::Byte(-4),
                Some(invalid.clone()),
                Some(invalid),
            )]));
        }
    }
    let varied: Vec<u8> = (0..2048)
        .map(|i| (i as u8).wrapping_mul(73).wrapping_add(31))
        .collect();
    check(&root(vec![
        section(Tag::Byte(-5), Some(Tag::ByteArray(varied.clone())), None),
        section(Tag::Byte(20), Some(Tag::ByteArray(varied.clone())), None),
        section(
            Tag::String("19".into()),
            Some(Tag::ByteArray(varied.clone())),
            None,
        ),
        section(Tag::Byte(19), Some(Tag::ByteArray(varied.clone())), None),
        section(Tag::Byte(17), None, Some(Tag::ByteArray(varied.clone()))),
        section(Tag::Byte(-4), Some(Tag::ByteArray(varied.clone())), None),
        // Last duplicate wins, even when its arrays have wrong types.
        section(
            Tag::Byte(-4),
            Some(Tag::Int(15)),
            Some(Tag::ByteArray(vec![7; 2047])),
        ),
        section(Tag::Byte(0), None, Some(Tag::ByteArray(varied))),
    ]));
}
/// A saved sky layer is the resident `Light::Sky` layer byte for byte,
/// so a section whose block light is uniformly zero can adopt it.
#[test]
fn a_varying_sky_layer_is_adopted_without_expansion() {
    let saved: Vec<u8> = (0..2048).map(|i| (i as u8).wrapping_mul(29)).collect();
    let tag = Tag::compound([
        ("isLightOn", Tag::Byte(1)),
        (
            "sections",
            Tag::List(vec![Tag::compound([
                ("Y", Tag::Byte(3)),
                ("SkyLight", Tag::ByteArray(saved.clone())),
            ])]),
        ),
    ]);
    let mut chunk = Chunk::empty(0, 0);
    let mut expected = chunk.clone();
    load_light(&tag, &mut chunk, true);
    reference(&tag, &mut expected, true);
    for i in 0..4096 {
        assert_eq!(chunk.sections[7].light(i), expected.sections[7].light(i));
    }
    let Light::Sky(layer, 0) = &chunk.sections[7].light else {
        panic!("varying sky with no block light is Light::Sky(_, 0)");
    };
    assert_eq!(
        &layer[..],
        &saved[..],
        "resident layer equals the saved one"
    );
}
/// #2571: the difficulty is a property of the world: the level data
/// holds it, so a world that was set to hard is hard when it is opened
/// again, and a world that was downloaded keeps the difficulty it was
/// made with.
#[test]
fn the_level_data_holds_the_difficulty() {
    let dir = std::env::temp_dir().join(format!("sunrise-difficulty-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let mut level = Level {
        name: "Test".into(),
        ..Level::default()
    };
    // The world's difficulty is the level data's.
    level.set_difficulty(3);
    assert_eq!(level.difficulty, "hard");
    assert_eq!(level.difficulty_id(), 3);
    // It survives the save, so the world comes back with it.
    level.write(&dir, &Default::default()).unwrap();
    let back = Level::read(&dir).unwrap();
    assert_eq!(back.difficulty, "hard");
    assert_eq!(back.difficulty_id(), 3);
    // A world whose difficulty is not one of ours is normal.
    let other = Level {
        difficulty: "extreme".into(),
        ..Level::default()
    };
    assert_eq!(other.difficulty_id(), 2);
    let _ = fs::remove_dir_all(&dir);
}

/// Dedicated world saves must not overwrite separately managed player data.
#[test]
fn dedicated_world_save_preserves_player_file() {
    let dir = std::env::temp_dir().join(format!(
        "sunrise-dedicated-player-save-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);

    let mut level = Level {
        name: "Test".into(),
        position: Some([12.5, 70.0, -8.25]),
        inventory: vec![(0, "minecraft:diamond".into(), 7, 0)],
        ..Level::default()
    };

    // Create a real player file using the existing singleplayer writer.
    level.write(&dir, &Default::default()).unwrap();
    let uuid = level.player_uuid.unwrap();
    let player_file = dir
        .join("players")
        .join("data")
        .join(format!("{}.dat", uuid_text(uuid)));

    let before = fs::read(&player_file).unwrap();

    // Simulate an incomplete world-level player state.
    level.position = None;
    level.inventory.clear();
    level.day_time = 12345;

    // Dedicated saves must update metadata without touching player data.
    level.write_without_player(&dir).unwrap();

    let after = fs::read(&player_file).unwrap();
    assert_eq!(before, after, "dedicated save overwrote player data");

    // The world metadata must still be persisted.
    let reopened = Level::read(&dir).unwrap();
    assert_eq!(reopened.day_time, 12345);

    let _ = fs::remove_dir_all(&dir);
}

/// A crash save that excludes the single-player owner writes `level.dat` without a new
/// player file and keeps a legacy `Data.Player`, the owner's only copy before a normal save.
#[test]
fn keeping_the_owner_writes_no_player_file_and_keeps_legacy_data_player() {
    let dir = std::env::temp_dir().join(format!("sunrise-keep-owner-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let mut level = Level {
        name: "Test".into(),
        ..Level::default()
    };
    level.write_without_player(&dir).unwrap();
    let legacy = Tag::compound([("Health", Tag::Float(7.0_f32.to_bits()))]);
    let mut root = read_dat(&dir.join("level.dat")).unwrap();
    let Tag::Compound(root_map) = &mut root else {
        panic!("level.dat is a compound");
    };
    let Some(Tag::Compound(data)) = root_map.get_mut(&JavaString::from("Data")) else {
        panic!("level.dat has Data");
    };
    data.insert(JavaString::from("Player"), legacy.clone());
    write_dat(&dir.join("level.dat"), root).unwrap();

    level.position = Some([1.0, 2.0, 3.0]);
    level.day_time = 777;
    level.write_keeping_player(&dir).unwrap();

    let root = read_dat(&dir.join("level.dat")).unwrap();
    assert_eq!(
        root.get("Data").and_then(|d| d.get("Player")),
        Some(&legacy)
    );
    assert!(!dir.join("players").join("data").exists());
    assert_eq!(Level::read(&dir).unwrap().day_time, 777);
    let _ = fs::remove_dir_all(&dir);
}

/// A keep-owner write leaves `singleplayer_uuid` as it was on disk, absent included:
/// naming the offline `Player` would make a reopen load another account's file.
#[test]
fn keeping_the_owner_keeps_the_on_disk_singleplayer_uuid() {
    let dir = std::env::temp_dir().join(format!("sunrise-keep-uuid-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let mut level = Level {
        name: "Test".into(),
        ..Level::default()
    };
    let set_uuid = |uuid: Option<Tag>| {
        let mut root = read_dat(&dir.join("level.dat")).unwrap();
        let Tag::Compound(root_map) = &mut root else {
            panic!("level.dat is a compound");
        };
        let Some(Tag::Compound(data)) = root_map.get_mut(&JavaString::from("Data")) else {
            panic!("level.dat has Data");
        };
        match uuid {
            Some(tag) => data.insert(JavaString::from("singleplayer_uuid"), tag),
            None => data.remove(&JavaString::from("singleplayer_uuid")),
        };
        write_dat(&dir.join("level.dat"), root).unwrap();
    };
    let on_disk = || {
        read_dat(&dir.join("level.dat"))
            .unwrap()
            .get("Data")
            .and_then(|d| d.get("singleplayer_uuid"))
            .cloned()
    };
    level.write_without_player(&dir).unwrap();

    set_uuid(None);
    level.write_keeping_player(&dir).unwrap();
    assert_eq!(on_disk(), None, "a level.dat without a uuid keeps none");

    let owner = Tag::IntArray(vec![1, 2, 3, 4]);
    set_uuid(Some(owner.clone()));
    level.write_keeping_player(&dir).unwrap();
    assert_eq!(on_disk(), Some(owner), "a uuid stays the same");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_ends_clock_keeps_its_own_time() {
    let dir = std::env::temp_dir().join(format!("sunrise-end-clock-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let mut level = Level {
        name: "Test".into(),
        time: 5000,
        day_time: 9000,
        ..Level::default()
    };
    // A world without one starts the End's clock at its game time.
    level.write(&dir, &Default::default()).unwrap();
    assert_eq!(Level::read(&dir).unwrap().end_clock, Some(5000));
    // Then it keeps its own count, apart from the game time and the day.
    level.time = 8000;
    level.end_clock = Some(321);
    level.write(&dir, &Default::default()).unwrap();
    let back = Level::read(&dir).unwrap();
    assert_eq!(
        (back.time, back.day_time, back.end_clock),
        (8000, 9000, Some(321))
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn level_round_trips_through_the_vanilla_files() {
    let dir = std::env::temp_dir().join(format!("sunrise-level-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let mut level = Level {
        name: "Test".into(),
        seed: -42,
        time: 1234,
        day_time: 25000,
        creative: true,
        position: Some([1.5, 70.0, -3.25]),
        rotation: [90.0, 10.0],
        health: 17.0,
        absorption: 5.0,
        food: 15,
        saturation: 2.5,
        inventory: vec![
            (0, "minecraft:stone".into(), 12, 0),
            (5, "minecraft:iron_sword".into(), 1, 3),
            (103, "minecraft:iron_helmet".into(), 1, 0),
        ],
        xp_level: 3,
        xp_progress: 0.5,
        xp_total: 30,
        player_score: -14,
        dimension: "minecraft:the_nether".into(),
        world_spawn: Some([8, 64, 8]),
        world_spawn_angle: [45.0, -10.0],
        // The world spawn's level is the world's, not the player's.
        world_spawn_dimension: Some("minecraft:the_end".into()),
        spawn: Some([20.0, 70.0, 20.0]),
        difficulty_locked: true,
        flying: true,
        ..Level::default()
    };
    level.weather.raining = true;
    level.weather.rain_time = 500;
    level
        .game_rules
        .insert("minecraft:keep_inventory".into(), 1);
    level.write(&dir, &Default::default()).unwrap();
    let saved = read_dat(&dir.join("level.dat")).unwrap();
    let spawn = saved.get("Data").and_then(|d| d.get("spawn"));
    assert_eq!(
        text(spawn.and_then(|s| s.get("dimension"))).as_deref(),
        Some("minecraft:the_end"),
        "level.dat's spawn.dimension"
    );
    let back = Level::read(&dir).unwrap();
    assert_eq!(back.name, "Test");
    assert_eq!(back.seed, -42);
    assert_eq!((back.time, back.day_time), (1234, 25000));
    assert!(back.creative);
    assert!(back.flying);
    assert_eq!(back.position, Some([1.5, 70.0, -3.25]));
    assert_eq!(back.rotation, [90.0, 10.0]);
    assert_eq!((back.health, back.food, back.saturation), (17.0, 15, 2.5));
    assert_eq!(back.absorption, 5.0);
    let mut inv = back.inventory.clone();
    inv.sort();
    assert_eq!(
        inv,
        vec![
            (0, "minecraft:stone".into(), 12, 0),
            (5, "minecraft:iron_sword".into(), 1, 3),
            (103, "minecraft:iron_helmet".into(), 1, 0)
        ]
    );
    assert_eq!(
        (back.xp_level, back.xp_progress, back.xp_total),
        (3, 0.5, 30)
    );
    assert_eq!(back.player_score, -14);
    assert_eq!(back.dimension, "minecraft:the_nether");
    assert_eq!(back.world_spawn, Some([8, 64, 8]));
    assert_eq!(back.world_spawn_angle, [45.0, -10.0]);
    assert_eq!(
        back.world_spawn_dimension.as_deref(),
        Some("minecraft:the_end")
    );
    assert_eq!(back.spawn, Some([8.0, 64.0, 8.0]));
    assert_eq!(
        back.respawn_elsewhere,
        Some(("minecraft:overworld".into(), [20, 70, 20], 0.0, 0.0, true))
    );
    assert!(back.difficulty_locked);
    assert!(back.weather.raining);
    assert_eq!(back.weather.rain_time, 500);
    assert_eq!(back.game_rule("keep_inventory"), 1);
    assert_eq!(back.game_rule("random_tick_speed"), 3);
    // Unknown player data survives another save.
    let mut extra = back.player_extra.clone().unwrap();
    put(
        &mut extra,
        "ShoulderEntityLeft",
        Tag::compound([("id", string("minecraft:parrot"))]),
    );
    let mut ender = vec![None; 27];
    ender[3] = Some(crate::items::ItemStack::new("minecraft:diamond", 2));
    let mut again = Level {
        player_extra: Some(extra),
        ender_items: ender,
        // The live amount, not the one the loaded compound held (#1307).
        absorption: 2.0,
        ..back
    };
    again.write(&dir, &Default::default()).unwrap();
    let third = Level::read(&dir).unwrap();
    assert_eq!(third.absorption, 2.0);
    assert!(matches!(
        third
            .player_extra
            .as_ref()
            .and_then(|p| p.get("ShoulderEntityLeft")),
        Some(Tag::Compound(_))
    ));
    // EnderItems round-trip by slot.
    assert_eq!(
        third.ender_items[3].as_ref().map(|s| (&*s.item, s.count)),
        Some(("minecraft:diamond", 2))
    );
    // A dead player's empty inventory replaces the previously loaded items.
    let mut dead = third;
    dead.health = 0.0;
    dead.inventory.clear();
    dead.inventory_components.clear();
    dead.write(&dir, &Default::default()).unwrap();
    let dead = Level::read(&dir).unwrap();
    assert_eq!(dead.health, 0.0);
    assert!(dead.inventory.is_empty());
    assert!(dead.inventory_components.is_empty());
    assert!(dead.player_extra.unwrap().get("equipment").is_none());
    assert!(dir.join("data/minecraft/world_gen_settings.dat").exists());
    let _ = fs::remove_dir_all(&dir);
}

fn eff(
    id: &str,
    duration: i32,
    amplifier: i32,
    ambient: bool,
    particles: bool,
    icon: bool,
) -> crate::effects::SavedEffect {
    crate::effects::SavedEffect {
        ambient,
        particles,
        icon,
        ..crate::effects::SavedEffect::new(id, duration, amplifier)
    }
}

#[test]
fn a_player_keeps_the_effects_hidden_under_a_stronger_one() {
    // MobEffectInstance.Details.CODEC nests `hidden_effect` (no id of its
    // own): Poison II for 6 s over Poison I for 30 s over Poison I
    // forever, as `/effect give` leaves them.
    let tag = Tag::compound([(
        "active_effects",
        Tag::List(vec![Tag::compound([
            ("id", string("minecraft:poison")),
            ("duration", Tag::Int(120)),
            ("amplifier", Tag::Byte(1)),
            ("show_icon", Tag::Byte(1)),
            (
                "hidden_effect",
                Tag::compound([
                    ("duration", Tag::Int(600)),
                    ("ambient", Tag::Byte(1)),
                    ("show_icon", Tag::Byte(1)),
                    (
                        "hidden_effect",
                        Tag::compound([
                            ("duration", Tag::Int(-1)),
                            ("show_particles", Tag::Byte(0)),
                            ("show_icon", Tag::Byte(0)),
                        ]),
                    ),
                ]),
            ),
        ])]),
    )]);
    let mut level = Level::default();
    level.read_player(&tag);
    let expected = crate::effects::SavedEffect {
        hidden: Some(Box::new(crate::effects::SavedEffect {
            hidden: Some(Box::new(eff(
                "minecraft:poison",
                -1,
                0,
                false,
                false,
                false,
            ))),
            ..eff("minecraft:poison", 600, 0, true, true, true)
        })),
        ..eff("minecraft:poison", 120, 1, false, true, true)
    };
    assert_eq!(level.active_effects, [expected]);
    // Written back, the chain is the same compound.
    assert_eq!(
        level.player_tag(&Default::default()).get("active_effects"),
        tag.get("active_effects")
    );
}

#[test]
fn a_saved_effect_without_show_icon_takes_show_particles() {
    // MobEffectInstance.Details.CODEC: `show_icon` defaults to
    // `show_particles`; an explicit one is kept apart from it.
    let effect = |extra: Vec<(&'static str, Tag)>| {
        let mut e = vec![
            ("id", string("minecraft:darkness")),
            ("duration", Tag::Int(260)),
        ];
        e.extend(extra);
        Tag::compound(e)
    };
    let mut level = Level::default();
    level.read_player(&Tag::compound([(
        "active_effects",
        Tag::List(vec![
            effect(vec![("show_particles", Tag::Byte(0))]),
            effect(vec![]),
            effect(vec![("show_icon", Tag::Byte(0))]),
        ]),
    )]));
    let flags: Vec<_> = level
        .active_effects
        .iter()
        .map(|e| (e.particles, e.icon))
        .collect();
    assert_eq!(flags, [(false, false), (true, true), (true, false)]);
}

#[test]
fn effects_that_ended_are_not_saved_again() {
    // A player loaded with an effect that has since run out: the loaded
    // compound still holds it, but the save writes only what is active.
    let mut level = Level {
        active_effects: vec![eff("minecraft:speed", 200, 1, false, true, true)],
        ..Level::default()
    };
    let loaded = level.player_tag(&Default::default());
    assert!(loaded.get("active_effects").is_some());
    level.player_extra = Some(loaded);
    level.active_effects.clear();
    assert!(
        level
            .player_tag(&Default::default())
            .get("active_effects")
            .is_none()
    );
}

#[test]
fn effect_amplifier_default_is_omitted_from_saved_players() {
    let effects = vec![
        eff("minecraft:luck", -1, 0, false, false, false),
        eff("minecraft:speed", 200, 1, true, false, true),
        eff("minecraft:regeneration", 40, 128, false, true, false),
        eff("minecraft:strength", -1, 255, false, true, true),
    ];
    for raw in [
        None,
        Some(Tag::compound([
            ("H100Keep", Tag::Int(7)),
            (
                "active_effects",
                Tag::List(vec![Tag::compound([
                    ("id", string("minecraft:luck")),
                    ("amplifier", Tag::Byte(42)),
                ])]),
            ),
        ])),
    ] {
        let level = Level {
            active_effects: effects.clone(),
            player_extra: raw.clone(),
            ..Level::default()
        };
        let saved = level.player_tag(&Default::default());
        let Some(Tag::List(entries)) = saved.get("active_effects") else {
            panic!("modeled live effects must be saved even without raw input")
        };
        assert_eq!(entries.len(), 4);
        assert_eq!(
            entries
                .iter()
                .map(|e| e.get("amplifier").cloned())
                .collect::<Vec<_>>(),
            vec![
                None,
                Some(Tag::Byte(1)),
                Some(Tag::Byte(-128)),
                Some(Tag::Byte(-1))
            ],
        );
        for (entry, e) in entries.iter().zip(&effects) {
            assert_eq!(entry.get("id"), Some(&string(&e.id)));
            assert_eq!(entry.get("duration"), Some(&Tag::Int(e.duration)));
            // MobEffectInstance.Details.MAP_CODEC leaves out ambient false
            // and show_particles true.
            let ambient_tag = e.ambient.then_some(Tag::Byte(1));
            assert_eq!(entry.get("ambient"), ambient_tag.as_ref());
            let particles_tag = (!e.particles).then_some(Tag::Byte(0));
            assert_eq!(entry.get("show_particles"), particles_tag.as_ref());
            assert_eq!(entry.get("show_icon"), Some(&Tag::Byte(i8::from(e.icon))));
        }
        assert_eq!(
            level.active_effects, effects,
            "saving must not mutate live state"
        );
        assert_eq!(
            saved.get("H100Keep"),
            raw.as_ref().and_then(|t| t.get("H100Keep"))
        );
    }
}

#[test]
fn effect_amplifier_255_reloads_as_an_unsigned_byte() {
    // `MobEffectInstance.MAX_AMPLIFIER` is 255. `Details.MAP_CODEC` writes
    // it with `ExtraCodecs.UNSIGNED_BYTE`: `255.byteValue()` is -1 and
    // `128.byteValue()` is -128; `UnsignedBytes.toInt` restores both.
    let effects = vec![
        eff("minecraft:strength", -1, 255, false, true, true),
        eff("minecraft:regeneration", -1, 128, false, true, true),
        eff("minecraft:speed", 40, 1, false, true, true),
    ];
    let level = Level {
        active_effects: effects.clone(),
        ..Level::default()
    };
    let tag = level.player_tag(&Default::default());
    let Tag::List(saved) = tag.get("active_effects").expect("active_effects") else {
        panic!("active_effects list");
    };
    assert_eq!(
        saved
            .iter()
            .map(|e| e.get("amplifier").cloned())
            .collect::<Vec<_>>(),
        vec![
            Some(Tag::Byte(-1)),
            Some(Tag::Byte(-128)),
            Some(Tag::Byte(1)),
        ]
    );
    let mut loaded = Level::default();
    loaded.read_player(&tag);
    assert_eq!(loaded.active_effects, effects);
}

#[test]
fn forced_overworld_spawnpoint_reloads_as_a_configuration() {
    // A personal point coinciding with the world spawn must still keep
    // its forced flag and yaw, rather than turning into a world fallback.
    for pos in [[5, 64, 5], [0, 80, 0]] {
        let point = ("minecraft:overworld".to_owned(), pos, 37.0, 0.0, true);
        let level = Level {
            respawn_elsewhere: Some(point.clone()),
            ..Level::default()
        };
        let mut loaded = Level {
            spawn: Some([0.0, 80.0, 0.0]),
            world_spawn: Some([0, 80, 0]),
            ..Level::default()
        };
        loaded.read_player(&level.player_tag(&Default::default()));
        assert_eq!(loaded.respawn_elsewhere, Some(point));
        assert_eq!(loaded.spawn, Some([0.0, 80.0, 0.0]));
        assert_eq!(
            loaded.player_tag(&Default::default()).get("respawn"),
            level.player_tag(&Default::default()).get("respawn")
        );
    }
}

#[test]
fn creative_flight_status_persists_in_save_and_resets_for_survival() {
    let mut level = Level {
        creative: true,
        flying: true,
        ..Level::default()
    };
    let tag = level.player_tag(&Default::default());
    let flying_byte = tag
        .get("abilities")
        .and_then(|a| number(a.get("flying")))
        .unwrap_or(0);
    assert_eq!(flying_byte, 1);

    let mut loaded = Level::default();
    loaded.read_player(&tag);
    assert!(loaded.flying);

    // Survival mode resets flying ability to 0 in save.
    level.creative = false;
    level.game_mode = 0;
    let tag_survival = level.player_tag(&Default::default());
    let flying_survival = tag_survival
        .get("abilities")
        .and_then(|a| number(a.get("flying")))
        .unwrap_or(0);
    assert_eq!(flying_survival, 0);
}

#[test]
fn unconsumed_root_vehicle_keeps_the_complete_vanilla_tree() {
    let entity = Tag::compound([
        ("id", Tag::String("minecraft:horse".into())),
        ("UUID", uuid_tag([5, 6, 7, 8])),
        (
            "Passengers",
            Tag::List(vec![Tag::compound([
                ("id", Tag::String("minecraft:pig".into())),
                ("UUID", uuid_tag([1, 2, 3, 4])),
            ])]),
        ),
    ]);
    let vehicle = Tag::compound([("Attach", uuid_tag([1, 2, 3, 4])), ("Entity", entity)]);
    let raw = Tag::compound([("RootVehicle", vehicle.clone())]);
    let mut level = Level::default();
    level.read_player(&raw);
    level.player_extra = Some(raw);
    assert_eq!(level.root_vehicle, Some([1, 2, 3, 4]));
    assert_eq!(
        level.player_tag(&Default::default()).get("RootVehicle"),
        Some(&vehicle)
    );
    // Even saving before the vehicle is restored must not destroy it.
    level.root_vehicle = None;
    assert_eq!(
        level.player_tag(&Default::default()).get("RootVehicle"),
        Some(&vehicle)
    );
}

/// `RecipesUsed` is what a furnace owes in experience, so a save that drops it pays the
/// player nothing after a reload.
#[test]
fn a_furnaces_recipes_used_survive_a_save() {
    use crate::containers::BlockEntity;
    let Some(BlockEntity::Furnace(mut f)) = BlockEntity::for_block("minecraft:furnace") else {
        panic!("a furnace");
    };
    f.recipes_used
        .insert("minecraft:cooked_porkchop".to_owned(), 3);
    let tag = block_entity_encode::encode(
        (1, 64, 2),
        &BlockEntity::Furnace(f),
        "minecraft:furnace",
        &Default::default(),
    );
    let Some((_, BlockEntity::Furnace(back))) = decode_block_entity(&tag) else {
        panic!("a furnace back");
    };
    assert_eq!(back.recipes_used.get("minecraft:cooked_porkchop"), Some(&3));
}
/// The world's save (Level.write) writes the rules as they are now, and a
/// world opened again reads them back (#74).
#[test]
fn level_save_keeps_changed_rules() {
    use crate::gamerules::{GameRules, Value};
    let dir = std::env::temp_dir().join(format!("sunrise-gamerules-level-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut level = super::Level::default();
    level.write(&dir, &Default::default()).unwrap();
    // Rules changed while playing, then saved and loaded again.
    let mut rules = GameRules::load(&dir);
    assert_eq!(rules, GameRules::default());
    assert!(rules.set("advance_time", Value::Bool(false)));
    assert!(rules.set("advance_weather", Value::Bool(false)));
    assert!(rules.set("random_tick_speed", Value::Int(0)));
    level.game_rules = rules.by_name();
    level.write(&dir, &Default::default()).unwrap();
    assert_eq!(GameRules::load(&dir), rules);
    let read = super::Level::read(&dir).unwrap();
    assert_eq!(read.game_rules, rules.by_name());
    assert_eq!(read.game_rule("advance_time"), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn level_save_rule_files_round_trip_without_byte_changes() {
    for absent_mod in [false, true] {
        let dir = std::env::temp_dir().join(format!(
            "sunrise-rule-file-roundtrip-{}-{absent_mod}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut level = super::Level::default();
        level.write(&dir, &Default::default()).unwrap();
        if absent_mod {
            let mut data = super::read_saved_data(&dir, "game_rules").unwrap();
            super::put(&mut data, "absent:rule", super::Tag::Long(42));
            super::write_saved_data(&dir, "game_rules", data).unwrap();
        }
        let mut before_level = super::read_dat(&dir.join("level.dat")).unwrap();
        let path = super::saved_data_path(&dir, "game_rules");
        let before_rules = std::fs::read(&path).unwrap();
        super::Level::read(&dir)
            .unwrap()
            .write(&dir, &Default::default())
            .unwrap();
        let mut after_level = super::read_dat(&dir.join("level.dat")).unwrap();
        for level in [&mut before_level, &mut after_level] {
            if let super::Tag::Compound(root) = level
                && let Some(super::Tag::Compound(data)) = root.get_mut(&"Data".into())
            {
                data.remove(&"LastPlayed".into());
            }
        }
        assert_eq!(after_level, before_level);
        assert_eq!(std::fs::read(path).unwrap(), before_rules);
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn level_read_retains_the_session_registry() {
    let dir = std::env::temp_dir().join(format!("sunrise-level-registry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let first = std::sync::Arc::new(crate::gamerules::RuleRegistry::vanilla());
    let second = std::sync::Arc::new(crate::gamerules::RuleRegistry::vanilla());
    let mut level = super::Level {
        rule_registry: first.clone(),
        ..Default::default()
    };
    level.write(&dir, &Default::default()).unwrap();
    let loaded = super::Level::read_with_registry(&dir, first.clone()).unwrap();
    assert!(std::sync::Arc::ptr_eq(&loaded.rule_registry, &first));
    let next = super::Level::read_with_registry(&dir, second.clone()).unwrap();
    assert!(!std::sync::Arc::ptr_eq(
        &loaded.rule_registry,
        &next.rule_registry
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn level_saved_rules_have_vanilla_keys_types_and_defaults() {
    let dir = std::env::temp_dir().join(format!("sunrise-rule-defaults-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut level = super::Level::default();
    level.write(&dir, &Default::default()).unwrap();
    let super::Tag::Compound(rules) = super::read_saved_data(&dir, "game_rules").unwrap() else {
        panic!("rule compound")
    };
    assert_eq!(rules.len(), 58);
    assert!(
        rules
            .keys()
            .all(|key| key.to_string().unwrap().starts_with("minecraft:"))
    );
    assert_eq!(
        rules.get(&"minecraft:random_tick_speed".into()),
        Some(&super::Tag::Int(3))
    );
    assert_eq!(
        rules.get(&"minecraft:keep_inventory".into()),
        Some(&super::Tag::Byte(0))
    );
    assert_eq!(
        rules.get(&"minecraft:max_snow_accumulation_height".into()),
        Some(&super::Tag::Int(1))
    );
    assert!(!rules.contains_key(&"minecraft:max_minecart_speed".into()));
    level.enabled_features = [crate::gamerules::MINECART_IMPROVEMENTS.to_owned()].into();
    level.game_rules =
        crate::gamerules::GameRules::with_features(&level.enabled_features).by_name();
    level.write(&dir, &Default::default()).unwrap();
    assert_eq!(
        super::read_saved_data(&dir, "game_rules")
            .unwrap()
            .get("minecraft:max_minecart_speed"),
        Some(&super::Tag::Int(8))
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn legacy_bare_saved_rules_load_under_canonical_keys() {
    let dir = std::env::temp_dir().join(format!("sunrise-rule-legacy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    super::Level::default()
        .write(&dir, &Default::default())
        .unwrap();
    super::write_saved_data(
        &dir,
        "game_rules",
        super::Tag::compound([
            ("keep_inventory", super::Tag::Byte(1)),
            ("random_tick_speed", super::Tag::Int(7)),
        ]),
    )
    .unwrap();
    let level = super::Level::read(&dir).unwrap();
    assert_eq!(level.game_rules.get("minecraft:keep_inventory"), Some(&1));
    assert_eq!(
        level.game_rules.get("minecraft:random_tick_speed"),
        Some(&7)
    );
    assert!(!level.game_rules.contains_key("keep_inventory"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn level_save_preserves_absent_mod_rules() {
    let dir =
        std::env::temp_dir().join(format!("sunrise-gamerules-unknown-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut level = super::Level::default();
    level.write(&dir, &Default::default()).unwrap();
    super::write_saved_data(
        &dir,
        "game_rules",
        super::Tag::compound([("absent:rule", super::Tag::Long(42))]),
    )
    .unwrap();
    level.game_rules.insert("minecraft:advance_time".into(), 0);
    level.write(&dir, &Default::default()).unwrap();
    assert_eq!(
        super::read_saved_data(&dir, "game_rules")
            .unwrap()
            .get("absent:rule"),
        Some(&super::Tag::Long(42))
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A save written before the key was namespaced holds an id twice, once bare: vanilla
/// refuses such a furnace, so a load merges them under the full id.
#[test]
fn a_furnaces_recipes_used_merges_bare_and_full_ids() {
    use crate::containers::BlockEntity;
    let Some(BlockEntity::Furnace(mut f)) = BlockEntity::for_block("minecraft:furnace") else {
        panic!("a furnace");
    };
    f.recipes_used
        .insert("iron_ingot_from_smelting_iron_ore".to_owned(), 2);
    f.recipes_used
        .insert("minecraft:iron_ingot_from_smelting_iron_ore".to_owned(), 3);
    let tag = block_entity_encode::encode(
        (1, 64, 2),
        &BlockEntity::Furnace(f),
        "minecraft:furnace",
        &Default::default(),
    );
    let Some((_, BlockEntity::Furnace(back))) = decode_block_entity(&tag) else {
        panic!("a furnace back");
    };
    assert_eq!(back.recipes_used.len(), 1);
    assert_eq!(
        back.recipes_used
            .get("minecraft:iron_ingot_from_smelting_iron_ore"),
        Some(&5)
    );
}

/// A world with an experiment writes and reads its full feature set.
#[test]
fn experimental_feature_flags_round_trip() {
    let dir = gen_world("enabled-features");
    let mut level = Level {
        name: "Features".into(),
        enabled_features: [
            "minecraft:vanilla".into(),
            "minecraft:minecart_improvements".into(),
        ]
        .into(),
        ..Level::default()
    };
    level.write(&dir, &Default::default()).unwrap();
    let back = Level::read(&dir).unwrap();
    assert_eq!(back.enabled_features, level.enabled_features);
    let raw = read_level_dat(&dir).unwrap();
    assert!(matches!(
        raw.get("Data").unwrap().get("enabled_features"),
        Some(Tag::List(_)),
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn omitted_feature_flags_decode_as_vanilla_and_stay_omitted() {
    let dir = gen_world("omitted-feature-flags");
    let output = gen_world("resaved-feature-flags");
    let mut level = Level::default();
    assert_eq!(*level.enabled_features, ["minecraft:vanilla"]);
    level.write(&dir, &Default::default()).unwrap();
    let raw = read_level_dat(&dir).unwrap();
    assert!(raw.get("Data").unwrap().get("enabled_features").is_none());

    let mut read = Level::read(&dir).unwrap();
    assert_eq!(*read.enabled_features, ["minecraft:vanilla"]);
    // The encoder must drop even a tag retained from an older file.
    let mut experimental = Level {
        enabled_features: [
            "minecraft:vanilla".into(),
            "minecraft:minecart_improvements".into(),
        ]
        .into(),
        ..Level::default()
    };
    experimental.write(&output, &Default::default()).unwrap();
    read.write(&output, &Default::default()).unwrap();
    let raw = read_level_dat(&output).unwrap();
    assert!(raw.get("Data").unwrap().get("enabled_features").is_none());
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(output);
}

/// `WorldDataConfiguration.MAP_CODEC` preserves a valid empty set, but defaults an
/// invalid tag instead of interpreting a failed list decode as an empty set.
#[test]
fn explicit_empty_and_malformed_feature_flags_have_distinct_save_outcomes() {
    for (case, input, expected_flags, expected_tag) in [
        ("empty", Tag::List(vec![]), &[][..], Some(Tag::List(vec![]))),
        (
            "non-string",
            Tag::List(vec![Tag::Int(7)]),
            &["minecraft:vanilla"][..],
            None,
        ),
        ("non-list", Tag::Int(7), &["minecraft:vanilla"][..], None),
    ] {
        let dir = gen_world(&format!("feature-flags-{case}"));
        write_dat(
            &dir.join("level.dat"),
            Tag::compound([(
                "Data",
                Tag::compound([
                    ("DataVersion", Tag::Int(DATA_VERSION)),
                    ("enabled_features", input),
                ]),
            )]),
        )
        .unwrap();
        let mut level = Level::read(&dir).unwrap();
        assert_eq!(&*level.enabled_features, expected_flags, "{case}");
        level.write_without_player(&dir).unwrap();
        let raw = read_level_dat(&dir).unwrap();
        assert_eq!(
            raw.get("Data").unwrap().get("enabled_features"),
            expected_tag.as_ref(),
            "{case}"
        );
        let _ = fs::remove_dir_all(dir);
    }
}

fn gen_world(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sunrise-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// `WorldOpenFlows.openWorldCheckWorldStemCompatibility`: every world a preset makes is
/// stable, and a world with a changed dimension, a custom one or a customized generator
/// is asked about.
#[test]
fn world_stem_compatibility_flags_unstable_and_customized_worlds() {
    for preset in [
        WorldPreset::Normal,
        WorldPreset::Flat,
        WorldPreset::LargeBiomes,
        WorldPreset::Amplified,
        WorldPreset::SingleBiomeSurface,
        WorldPreset::Debug,
    ] {
        let dir = gen_world("stem-stable");
        write_world_gen_settings(
            &dir,
            7,
            WorldGen {
                preset,
                structures: true,
                bonus_chest: false,
            },
        )
        .unwrap();
        assert!(!unstable(&dir), "{preset:?}");
        assert!(!is_old_customized_world(&dir));
        let _ = fs::remove_dir_all(dir);
    }
    // Without world_gen_settings there is nothing to ask about.
    let dir = gen_world("stem-none");
    assert!(!unstable(&dir) && !is_old_customized_world(&dir));
    let _ = fs::remove_dir_all(dir);

    let dir = gen_world("stem-edit");
    write_world_gen_settings(&dir, 7, WorldGen::default()).unwrap();
    let settings = world_gen_settings(&dir).unwrap();
    let edit = |change: &dyn Fn(&mut Tag)| {
        let mut tag = settings.clone();
        change(&mut tag);
        write_saved_data(&dir, "world_gen_settings", tag).unwrap();
    };
    fn dimension<'a>(tag: &'a mut Tag, key: &str) -> &'a mut Tag {
        tag.get_mut("dimensions").unwrap().get_mut(key).unwrap()
    }
    // A custom biome source in the Nether is not the preset.
    edit(&|tag| {
        let nether = dimension(tag, "minecraft:the_nether");
        let generator = nether.get_mut("generator").unwrap();
        *generator.get_mut("biome_source").unwrap() = Tag::compound([
            ("type", string("minecraft:fixed")),
            ("biome", string("minecraft:plains")),
        ]);
    });
    assert!(unstable(&dir));
    // The End on another dimension type.
    edit(&|tag| {
        let end = dimension(tag, "minecraft:the_end");
        *end.get_mut("type").unwrap() = string("minecraft:overworld");
    });
    assert!(unstable(&dir));
    // Another noise settings in the Overworld is fine (amplified), in the Nether it is not.
    edit(&|tag| {
        let nether = dimension(tag, "minecraft:the_nether");
        *nether
            .get_mut("generator")
            .unwrap()
            .get_mut("settings")
            .unwrap() = string("minecraft:overworld");
    });
    assert!(unstable(&dir));
    // A fourth dimension, and a missing builtin one.
    edit(&|tag| {
        if let Some(Tag::Compound(dimensions)) = tag.get_mut("dimensions") {
            let end = dimensions.get(&"minecraft:the_end".into()).unwrap().clone();
            dimensions.insert("mypack:moon".into(), end);
        }
    });
    assert!(unstable(&dir));
    edit(&|tag| {
        if let Some(Tag::Compound(dimensions)) = tag.get_mut("dimensions") {
            dimensions.remove(&"minecraft:the_end".into());
        }
    });
    assert!(unstable(&dir));
    // legacy_custom_options marks an old customized world.
    edit(&|tag| {
        if let Tag::Compound(map) = tag {
            map.insert("legacy_custom_options".into(), string("{\"seaLevel\":63}"));
        }
    });
    assert!(is_old_customized_world(&dir));
    let _ = fs::remove_dir_all(dir);
}

/// `WorldOpenFlows.loadWorldStem`: the enabled data packs' level stems are the base of
/// `WorldDimensions.bake`, replacing the saved one of their id.
#[test]
fn world_stem_compatibility_reads_data_pack_dimensions() {
    let dir = gen_world("stem-datapack");
    write_world_gen_settings(&dir, 7, WorldGen::default()).unwrap();
    assert!(!unstable(&dir));
    let write = |pack: &str, file: &str, text: &str| {
        let path = dir.join("datapacks").join(pack).join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write("p", "pack.mcmeta", r#"{"pack":{"description":"fixture"}}"#);
    // A pack's dimension of another id is not vanilla-like.
    write(
        "p",
        "data/mypack/dimension/moon.json",
        r#"{"type":"minecraft:the_end","generator":{"type":"minecraft:noise","settings":"minecraft:end","biome_source":{"type":"minecraft:the_end"}}}"#,
    );
    assert!(unstable(&dir));
    // Disabled, it counts for nothing.
    fs::write(dir.join("datapacks.json"), r#"{"disabled":["file/p"]}"#).unwrap();
    assert!(!unstable(&dir));
    fs::remove_file(dir.join("datapacks.json")).unwrap();
    fs::remove_file(dir.join("datapacks/p/data/mypack/dimension/moon.json")).unwrap();
    assert!(!unstable(&dir));
    // A pack's Nether with another biome source replaces the saved vanilla one.
    write(
        "p",
        "data/minecraft/dimension/the_nether.json",
        r#"{"type":"minecraft:the_nether","generator":{"type":"minecraft:noise","settings":"minecraft:nether","biome_source":{"type":"minecraft:fixed","biome":"minecraft:plains"}}}"#,
    );
    assert!(unstable(&dir));
    // And a vanilla-like one stays stable.
    write(
        "p",
        "data/minecraft/dimension/the_nether.json",
        r#"{"type":"minecraft:the_nether","generator":{"type":"minecraft:noise","settings":"minecraft:nether","biome_source":{"type":"minecraft:multi_noise","preset":"minecraft:nether"}}}"#,
    );
    assert!(!unstable(&dir));
    let _ = fs::remove_dir_all(dir);
}

/// A world that still needs its file fixes keeps its generator settings in level.dat; they
/// are read as the fixes would leave them, so the question is asked on the first open.
#[test]
fn world_gen_settings_of_a_world_needing_file_fixes_are_read_in_memory() {
    let dir = gen_world("stem-prefix");
    write_world_gen_settings(&dir, 7, WorldGen::default()).unwrap();
    let settings = world_gen_settings(&dir).unwrap();
    let customized = {
        let mut tag = settings.clone();
        if let Tag::Compound(map) = &mut tag {
            map.insert("legacy_custom_options".into(), string("{\"seaLevel\":63}"));
        }
        tag
    };
    // 26.3 layout moved back to the old one: settings in level.dat, no saved data file.
    fs::remove_file(saved_data_path(&dir, "world_gen_settings")).unwrap();
    write_dat(
        &dir.join("level.dat"),
        Tag::compound([(
            "Data",
            Tag::compound([
                ("DataVersion", Tag::Int(4700)),
                ("WorldGenSettings", customized),
            ]),
        )]),
    )
    .unwrap();
    assert!(needs_file_fixes(&dir));
    let read = world_gen_settings(&dir).expect("settings");
    assert!(read.get("dimensions").is_some());
    assert!(!unstable(&dir));
    let _ = fs::remove_dir_all(dir);
}

/// Settings that fail validation in the old layout do not hide valid saved data.
#[test]
fn invalid_projected_settings_fall_through_to_saved_data() {
    let dir = gen_world("stem-prefix");
    write_world_gen_settings(&dir, 7, WorldGen::default()).unwrap();
    write_dat(
        &dir.join("level.dat"),
        Tag::compound([(
            "Data",
            Tag::compound([
                ("DataVersion", Tag::Int(4700)),
                ("WorldGenSettings", Tag::compound([("seed", Tag::Long(5))])),
            ]),
        )]),
    )
    .unwrap();
    assert!(needs_file_fixes(&dir));
    assert_eq!(
        number(world_gen_settings(&dir).unwrap().get("seed")),
        Some(7)
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn low_disk_space_is_under_64_mib_usable() {
    assert_eq!(LOW_DISK_SPACE, 64 * 1024 * 1024);
    let dir = std::env::temp_dir();
    assert!(estimate_disk_space(&dir) > 0);
    assert_eq!(
        low_disk_space(&dir),
        estimate_disk_space(&dir) < LOW_DISK_SPACE
    );
    // A path that does not exist cannot be asked: vanilla reads that as plenty.
    #[cfg(unix)]
    assert_eq!(
        estimate_disk_space(&dir.join("sunrise-no-such-folder/x")),
        u64::MAX
    );
}

#[test]
fn mod_attachments_save_in_sunrises_namespace_only_when_some() {
    use sunrise_mod_runtime::attachments::Attachments;
    let root = std::env::temp_dir().join(format!("sunrise-attach-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let save = SaveDir::new(
        root.clone(),
        LevelShape::OVERWORLD,
        Some(Default::default()),
    );
    let file = root.join("data/sunrise/attachments.dat");
    save.save_attachments(&Attachments::default()).unwrap();
    assert!(
        !root.join("data").exists(),
        "a level without mod data writes nothing"
    );
    let data = Compound::from_iter([("example:count".into(), Tag::Int(3))]);
    save.save_attachments(&Attachments::from_nbt(data.clone()))
        .unwrap();
    assert!(file.is_file());
    assert_eq!(save.load_attachments().try_to_nbt().unwrap(), data);
    fs::write(&file, b"not nbt").unwrap();
    assert!(save.load_attachments().try_to_nbt().unwrap().is_empty());
    assert_eq!(
        fs::read(root.join("data/sunrise/attachments.dat.corrupt")).unwrap(),
        b"not nbt"
    );
    assert!(!file.exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn attachment_encode_failure_refuses_record_and_keeps_original_saved_value() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use sunrise_mod_runtime::attachments::{
        AttachmentCodec, AttachmentEncodeError, AttachmentError, AttachmentType, Attachments,
        Holder,
    };

    #[derive(Clone)]
    struct Fallible {
        value: i32,
        fail: Arc<AtomicBool>,
    }
    fn encode_fallible(v: &Fallible) -> std::result::Result<Compound, AttachmentError> {
        if v.fail.load(Ordering::Relaxed) {
            return Err(AttachmentError::Encode);
        }
        Ok([("value".into(), Tag::Int(v.value))].into())
    }
    fn decode_fallible(c: &Compound) -> std::result::Result<Fallible, AttachmentError> {
        match c.get(&"value".into()) {
            Some(Tag::Int(value)) => Ok(Fallible {
                value: *value,
                fail: Arc::new(AtomicBool::new(false)),
            }),
            _ => Err(AttachmentError::Decode),
        }
    }
    static FALLIBLE: AttachmentType<Fallible> = AttachmentType {
        namespace: "example_mod",
        name: "persisted",
        default: || Fallible {
            value: 0,
            fail: Arc::new(AtomicBool::new(false)),
        },
        codec: Some(AttachmentCodec {
            encode: encode_fallible,
            decode: decode_fallible,
        }),
        copy_on_death: false,
        sync_codec: None,
    };

    let dir = tempfile::tempdir().unwrap();
    let save = SaveDir::new(
        dir.path().to_path_buf(),
        LevelShape::OVERWORLD,
        Some(Default::default()),
    );
    let fail = Arc::new(AtomicBool::new(false));
    let mut attachments = Attachments::default();
    FALLIBLE
        .set(
            &mut attachments,
            Holder::Level,
            Fallible {
                value: 42,
                fail: fail.clone(),
            },
        )
        .unwrap();
    // First save accepts and persists 42 on disk.
    save.save_attachments(&attachments).unwrap();

    // Next encode fails: level save refuses the record and disk keeps 42.
    fail.store(true, Ordering::Relaxed);
    let err = save.save_attachments(&attachments).unwrap_err();
    let encode_err = err.downcast_ref::<AttachmentEncodeError>().unwrap();
    assert_eq!(encode_err.namespace, "example_mod");
    assert_eq!(encode_err.name, "persisted");
    let mut reloaded = save.load_attachments();
    assert_eq!(
        FALLIBLE
            .get_existing(&mut reloaded, Holder::Level)
            .unwrap()
            .map(|v| v.value),
        Some(42)
    );

    // Same contract for a chunk record: save accepted value, fail next encode, reload keeps 42.
    let (reg, biomes) = loot_tests::fixture();
    let reg = &*Box::leak(Box::new(reg));
    let biomes = Arc::new(biomes);
    let fail_chunk = Arc::new(AtomicBool::new(false));
    let mut chunk_attachments = Attachments::default();
    FALLIBLE
        .set(
            &mut chunk_attachments,
            Holder::Chunk([0, 0]),
            Fallible {
                value: 42,
                fail: fail_chunk.clone(),
            },
        )
        .unwrap();
    let mut chunk = Chunk::empty(0, 0);
    chunk.saved = Some(Arc::new(ChunkExtras {
        attachments: chunk_attachments,
        ..Default::default()
    }));
    let chunk = Arc::new(chunk);
    save.save_later(std::slice::from_ref(&chunk), reg, &biomes)
        .unwrap();
    save.flush().unwrap();
    assert_eq!(save.record_save_failures(), 0);

    fail_chunk.store(true, Ordering::Relaxed);
    save.save_later(std::slice::from_ref(&chunk), reg, &biomes)
        .unwrap();
    save.flush().unwrap();
    assert_eq!(save.record_save_failures(), 1);

    let reloaded_chunk = save.load(0, 0, reg, &biomes).unwrap();
    let mut reloaded_chunk_attachments = reloaded_chunk.saved.as_ref().unwrap().attachments.clone();
    assert_eq!(
        FALLIBLE
            .get_existing(&mut reloaded_chunk_attachments, Holder::Chunk([0, 0]))
            .unwrap()
            .map(|v| v.value),
        Some(42)
    );
}

/// A block caught mid-move carries its block entity's attachments with no live block entity
/// to write them. If they cannot encode, the whole chunk record is refused and the old record
/// stays, rather than the moving block being saved without them.
#[test]
fn a_moving_blocks_attachment_encode_failure_refuses_its_chunk_record() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use sunrise_mod_runtime::attachments::{
        AttachmentCodec, AttachmentError, AttachmentType, Attachments, Holder,
    };

    #[derive(Clone)]
    struct Fallible {
        value: i32,
        fail: Arc<AtomicBool>,
    }
    static FALLIBLE: AttachmentType<Fallible> = AttachmentType {
        namespace: "example_mod",
        name: "carried",
        default: || Fallible {
            value: 0,
            fail: Arc::new(AtomicBool::new(false)),
        },
        codec: Some(AttachmentCodec {
            encode: |v: &Fallible| {
                if v.fail.load(Ordering::Relaxed) {
                    return Err(AttachmentError::Encode);
                }
                Ok([("value".into(), Tag::Int(v.value))].into())
            },
            decode: |c| match c.get(&"value".into()) {
                Some(Tag::Int(value)) => Ok(Fallible {
                    value: *value,
                    fail: Arc::new(AtomicBool::new(false)),
                }),
                _ => Err(AttachmentError::Decode),
            },
        }),
        copy_on_death: false,
        sync_codec: None,
    };
    let value = |value: i32, holder: Holder, fail: &Arc<AtomicBool>| {
        let mut attachments = Attachments::default();
        FALLIBLE
            .set(
                &mut attachments,
                holder,
                Fallible {
                    value,
                    fail: fail.clone(),
                },
            )
            .unwrap();
        attachments
    };
    let dir = tempfile::tempdir().unwrap();
    let save = SaveDir::new(
        dir.path().to_path_buf(),
        LevelShape::OVERWORLD,
        Some(Default::default()),
    );
    let (reg, biomes) = loot_tests::fixture();
    let reg = &*Box::leak(Box::new(reg));
    let biomes = Arc::new(biomes);
    let never = Arc::new(AtomicBool::new(false));
    let chunk_value = |chunk: &Chunk| {
        let mut attachments = chunk.saved.as_ref().unwrap().attachments.clone();
        FALLIBLE
            .get_existing(&mut attachments, Holder::Chunk([0, 0]))
            .unwrap()
            .map(|v| v.value)
    };
    let saved = |chunk_value: i32, carried_fails: &Arc<AtomicBool>| {
        let mut chunk = Chunk::empty(0, 0);
        chunk.saved = Some(Arc::new(ChunkExtras {
            attachments: value(chunk_value, Holder::Chunk([0, 0]), &never),
            block_entity_attachments: vec![(
                (1, 64, 1),
                value(5, Holder::BlockEntity([1, 64, 1]), carried_fails),
            )],
            ..Default::default()
        }));
        Arc::new(chunk)
    };

    // Setting a value encodes it once, so the codec starts out working and fails only from
    // the save that should be refused.
    let carried_fails = Arc::new(AtomicBool::new(false));
    save.save_later(&[saved(42, &carried_fails)], reg, &biomes)
        .unwrap();
    save.flush().unwrap();
    assert_eq!(save.record_save_failures(), 0);
    assert_eq!(
        chunk_value(&save.load(0, 0, reg, &biomes).unwrap()),
        Some(42)
    );

    let refused = saved(9, &carried_fails);
    carried_fails.store(true, Ordering::Relaxed);
    save.save_later(&[refused], reg, &biomes).unwrap();
    save.flush().unwrap();
    assert_eq!(save.record_save_failures(), 1);
    assert_eq!(
        chunk_value(&save.load(0, 0, reg, &biomes).unwrap()),
        Some(42),
        "the refused record left the old one on disk"
    );
}

/// A level id never names a folder outside the world's `dimensions` folder, and no two ids
/// share a folder.
#[test]
fn a_level_folder_stays_inside_the_world() {
    let world = Path::new("/w");
    assert_eq!(
        dimension_dir(world, "minecraft:the_nether"),
        Path::new("/w/dimensions/minecraft/the_nether")
    );
    assert_eq!(
        dimension_dir(world, "test:a/b"),
        Path::new("/w/dimensions/test/a/b")
    );
    let mut seen = std::collections::HashSet::new();
    for id in [
        "test:../minecraft/overworld",
        "test:a/../../b",
        "test:/abs",
        "..:x",
        "test:a//b",
        "test:Pocket",
    ] {
        let dir = dimension_dir(world, id);
        assert!(
            dir.starts_with("/w/dimensions/%invalid")
                && dir
                    .components()
                    .all(|c| !matches!(c, std::path::Component::ParentDir)),
            "{id}: {}",
            dir.display()
        );
        assert!(seen.insert(dir), "{id} shares a folder");
    }
}

#[test]
fn forced_chunks_survive_dimension_restarts_and_removals() {
    let root = std::env::temp_dir().join(format!("sunrise-forced-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let dimensions = [
        "minecraft:overworld",
        "minecraft:the_nether",
        "minecraft:the_end",
    ];
    for (i, dim) in dimensions.iter().enumerate() {
        let save = SaveDir::new(
            dimension_dir(&root, dim),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        assert!(save.load_forced_chunks().is_empty());
        let forced = HashSet::from([(i as i32, -17), (-23, i as i32), (-1, -1)]);
        save.save_chunk_tickets(&forced, &[]).unwrap();
    }
    for (i, dim) in dimensions.iter().enumerate() {
        let path = dimension_dir(&root, dim);
        let save = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        let mut forced = save.load_forced_chunks();
        assert_eq!(
            forced,
            HashSet::from([(i as i32, -17), (-23, i as i32), (-1, -1)])
        );
        let mut data = read_saved_data(&path, "chunk_tickets").unwrap();
        let other = Tag::compound([
            ("type", Tag::String("example:loader".into())),
            ("level", Tag::Int(30)),
            ("chunk_pos", Tag::IntArray(vec![7, -8])),
            ("ticks_left", Tag::Long(42)),
        ]);
        if let Some(Tag::List(tickets)) = data.get_mut("tickets") {
            assert!(
                tickets
                    .iter()
                    .all(|t| t.get("level") == Some(&Tag::Int(31)))
            );
            tickets.push(other.clone());
        }
        write_saved_data(&path, "chunk_tickets", data).unwrap();
        forced.remove(&(-1, -1));
        save.save_chunk_tickets(&forced, &[]).unwrap();
        drop(save);
        let reopened = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        assert_eq!(reopened.load_forced_chunks(), forced);
        reopened.save_chunk_tickets(&HashSet::new(), &[]).unwrap();
        assert!(
            SaveDir::new(path.clone(), LevelShape::OVERWORLD, None)
                .load_forced_chunks()
                .is_empty()
        );
        assert_eq!(
            read_saved_data(&path, "chunk_tickets")
                .unwrap()
                .get("tickets"),
            Some(&Tag::List(vec![other]))
        );
    }
    fs::remove_dir_all(root).unwrap();
}

/// The Debug preset (`worldgen/world_preset/debug_all_block_states.json`): its id names it,
/// the saved overworld generator is `minecraft:debug` and reads back as it, and the create
/// screen's button never cycles to it.
#[test]
fn the_debug_preset_is_saved_and_read_back() {
    assert_eq!(
        WorldPreset::of_id("minecraft:debug_all_block_states"),
        Some(WorldPreset::Debug)
    );
    assert!(!WorldPreset::ALL.contains(&WorldPreset::Debug));
    let dir = gen_world("debug-preset");
    write_world_gen_settings(
        &dir,
        7,
        WorldGen {
            preset: WorldPreset::Debug,
            structures: true,
            bonus_chest: false,
        },
    )
    .unwrap();
    let settings = world_gen_settings(&dir).unwrap();
    let generator = settings
        .get("dimensions")
        .and_then(|d| d.get("minecraft:overworld"))
        .and_then(|o| o.get("generator"))
        .unwrap();
    assert_eq!(
        text(generator.get("type")).as_deref(),
        Some("minecraft:debug")
    );
    assert_eq!(WorldGen::read(&dir).unwrap().preset, WorldPreset::Debug);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn world_preset_next_and_prev_cycle_bidirectionally() {
    let first = WorldPreset::Normal;
    assert_eq!(first.next(), WorldPreset::Flat);
    assert_eq!(first.prev(), WorldPreset::SingleBiomeSurface);
    assert_eq!(WorldPreset::SingleBiomeSurface.next(), WorldPreset::Normal);
    for preset in WorldPreset::ALL {
        assert_eq!(preset.next().prev(), preset);
        assert_eq!(preset.prev().next(), preset);
    }
}
/// A region record of the dimension folder `save` at chunk (0, 0), written
/// as an older version left it.
fn write_old_record(save: &SaveDir, folder: &str, tag: Tag) {
    let record = Record::Tag(Arc::new(NamedTag {
        name: "".into(),
        tag,
    }));
    let result = super::write_region_records(
        save.region_storage(Folder::of(folder)),
        &[((0, 0), &record)],
        &fs::File::sync_data,
    );
    assert!(result.failed_records.is_empty() && result.sync_error.is_none());
}

#[test]
fn older_records_of_a_current_world_upgrade_as_they_are_read() {
    let world = std::env::temp_dir().join(format!("sunrise-lazy-dv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    let save = SaveDir::new(
        super::dimension_dir(&world, "minecraft:overworld"),
        LevelShape::OVERWORLD,
        Some(Default::default()),
    );
    // A 1.20.1 chunk (DataVersion 3465): 1.20.3 renamed its grass to short_grass.
    let section = Tag::compound([
        ("Y", Tag::Byte(-4)),
        (
            "block_states",
            Tag::compound([(
                "palette",
                Tag::List(vec![Tag::compound([("Name", string("minecraft:grass"))])]),
            )]),
        ),
        (
            "biomes",
            Tag::compound([("palette", Tag::List(vec![string("minecraft:plains")]))]),
        ),
    ]);
    write_old_record(
        &save,
        "region",
        Tag::compound([
            ("DataVersion", Tag::Int(3465)),
            ("xPos", Tag::Int(0)),
            ("zPos", Tag::Int(0)),
            ("yPos", Tag::Int(-4)),
            ("Status", string("minecraft:full")),
            ("sections", Tag::List(vec![section])),
        ]),
    );
    let chunk = save.raw_chunk(0, 0).expect("the chunk reads");
    assert_eq!(
        number(chunk.tag.get("DataVersion")),
        Some(i64::from(DATA_VERSION))
    );
    let Some(Tag::List(sections)) = chunk.tag.get("sections") else {
        panic!("no sections: {:?}", chunk.tag);
    };
    let Some(Tag::List(palette)) = sections[0]
        .get("block_states")
        .and_then(|b| b.get("palette"))
    else {
        panic!("no palette");
    };
    assert_eq!(
        text(palette[0].get("id")).as_deref(),
        Some("minecraft:short_grass")
    );
    // Decoded, it keeps its blocks: read raw, the 26.x decoder finds no `id` in a
    // 1.20 palette (`Name`) and the whole column is air.
    if let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") {
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
            root.to_path_buf(),
        ))
        .expect("biomes");
        let grass = assets
            .registry
            .default_state("minecraft:short_grass")
            .unwrap();
        let loaded = save
            .load(0, 0, assets.registry, &biomes)
            .expect("the chunk loads");
        assert_eq!(loaded.get(3, -60, 7), grass.0);
    }
    // A 1.20.1 item entity: 1.20.5 made its stack `count` (an int) from `Count`.
    write_old_record(
        &save,
        "entities",
        Tag::compound([
            ("DataVersion", Tag::Int(3465)),
            ("Position", Tag::IntArray(vec![0, 0])),
            (
                "Entities",
                Tag::List(vec![Tag::compound([
                    ("id", string("minecraft:item")),
                    (
                        "Item",
                        Tag::compound([("id", string("minecraft:stone")), ("Count", Tag::Byte(3))]),
                    ),
                ])]),
            ),
        ]),
    );
    let entities = save.load_entities(0, 0).unwrap();
    let item = entities[0].get("Item").expect("the item stack");
    assert_eq!(item.get("count"), Some(&Tag::Int(3)));
    assert!(item.get("Count").is_none());
    let _ = std::fs::remove_dir_all(&world);
}

#[test]
fn old_chunks_are_found_in_the_saved_records_and_old_poi_upgrade() {
    let world = std::env::temp_dir().join(format!("sunrise-lazy-old-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    let nether = SaveDir::new(
        super::dimension_dir(&world, "minecraft:the_nether"),
        LevelShape::NETHER,
        Some(Default::default()),
    );
    // A 1.20.1 Nether chunk: upgraded it has no `blending_data` (only Overworld
    // chunks blend) and the current DataVersion, so only the saved record says
    // it is older than blending (4997), as `IOWorker.isOldChunk` reads it.
    write_old_record(
        &nether,
        "region",
        Tag::compound([
            ("DataVersion", Tag::Int(3465)),
            ("xPos", Tag::Int(0)),
            ("zPos", Tag::Int(0)),
            ("yPos", Tag::Int(0)),
            ("Status", string("minecraft:full")),
            ("sections", Tag::List(Vec::new())),
        ]),
    );
    let upgraded = nether.raw_chunk(0, 0).expect("the chunk reads");
    assert!(upgraded.tag.get("blending_data").is_none());
    assert_ne!(nether.old_chunk_bits(0, 0)[0] & 1, 0);
    // One chunk's record tells the same, without the region's other chunks; a chunk this
    // version saved, and one not saved, are not old.
    let current = Record::Tag(Arc::new(NamedTag {
        name: "".into(),
        tag: Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("xPos", Tag::Int(1)),
            ("zPos", Tag::Int(0)),
            ("yPos", Tag::Int(0)),
            ("Status", string("minecraft:full")),
            ("sections", Tag::List(Vec::new())),
        ]),
    }));
    let result = super::write_region_records(
        nether.region_storage(Folder::Region),
        &[((1, 0), &current)],
        &fs::File::sync_data,
    );
    assert!(result.failed_records.is_empty() && result.sync_error.is_none());
    assert!(nether.is_old_chunk(0, 0));
    assert!(!nether.is_old_chunk(1, 0));
    assert!(!nether.is_old_chunk(2, 0));
    assert_eq!(nether.old_chunk_bits(0, 0)[0] & 0b110, 0);
    // A POI record of before 1.14.4 (no DataVersion, read as 1945): its
    // sections at the root, which 1946 moved under `Sections`.
    let overworld = SaveDir::new(
        super::dimension_dir(&world, "minecraft:overworld"),
        LevelShape::OVERWORLD,
        Some(Default::default()),
    );
    write_old_record(
        &overworld,
        "poi",
        Tag::compound([(
            "0",
            Tag::compound([("Valid", Tag::Byte(1)), ("Records", Tag::List(Vec::new()))]),
        )]),
    );
    let poi = overworld.read_record("poi", 0, 0).expect("the POI reads");
    assert!(poi.tag.get("Sections").and_then(|s| s.get("0")).is_some());
    assert_eq!(
        number(poi.tag.get("DataVersion")),
        Some(i64::from(DATA_VERSION))
    );
    let _ = std::fs::remove_dir_all(&world);
}

#[test]
fn a_chunk_from_before_1_18_is_brought_to_the_current_layout() {
    let world = std::env::temp_dir().join(format!("sunrise-lazy-117-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    let save = SaveDir::new(
        super::dimension_dir(&world, "minecraft:overworld"),
        LevelShape::OVERWORLD,
        Some(Default::default()),
    );
    // A 1.17.1 chunk (DataVersion 2730), its fields under `Level`: section 0 all stone.
    let section = Tag::compound([
        ("Y", Tag::Byte(0)),
        (
            "Palette",
            Tag::List(vec![
                Tag::compound([("Name", string("minecraft:air"))]),
                Tag::compound([("Name", string("minecraft:stone"))]),
            ]),
        ),
        (
            "BlockStates",
            Tag::LongArray(vec![0x1111_1111_1111_1111; 256]),
        ),
    ]);
    write_old_record(
        &save,
        "region",
        Tag::compound([
            ("DataVersion", Tag::Int(2730)),
            (
                "Level",
                Tag::compound([
                    ("xPos", Tag::Int(0)),
                    ("zPos", Tag::Int(0)),
                    ("Status", string("full")),
                    ("Biomes", Tag::IntArray(vec![1; 1024])),
                    ("Sections", Tag::List(vec![section])),
                ]),
            ),
        ]),
    );
    let chunk = save.raw_chunk(0, 0).expect("the chunk reads");
    assert!(
        chunk.tag.get("Level").is_none(),
        "still wrapped: {:?}",
        chunk.tag
    );
    assert_eq!(number(chunk.tag.get("xPos")), Some(0));
    // 1.18's height fix: an Overworld chunk older than the new heights blends with them.
    assert!(chunk.tag.get("blending_data").is_some());
    let _ = std::fs::remove_dir_all(&world);
}

/// A chunk last saved by 1.16.5 (DataVersion 2586) keeps its entities in its own
/// `Level.Entities`: the upgrade moves them to `entities` and the load adds them
/// (`SerializableChunkData.postLoadChunk`, `ServerLevel.addLegacyChunkEntities`), so its
/// minecart is in the world (#14424).
#[test]
#[ignore = "daily"]
fn a_chunk_from_before_1_17_loads_the_entities_it_keeps() {
    let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        root.clone(),
    ))
    .expect("biomes");
    let world = std::env::temp_dir().join(format!("sunrise-lazy-116-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    let save = SaveDir::new(
        super::dimension_dir(&world, "minecraft:overworld"),
        LevelShape::OVERWORLD,
        Some(Default::default()),
    );
    let section = Tag::compound([
        ("Y", Tag::Byte(0)),
        (
            "Palette",
            Tag::List(vec![
                Tag::compound([("Name", string("minecraft:air"))]),
                Tag::compound([("Name", string("minecraft:stone"))]),
            ]),
        ),
        (
            "BlockStates",
            Tag::LongArray(vec![0x1111_1111_1111_1111; 256]),
        ),
    ]);
    let minecart = Tag::compound([
        ("id", string("minecraft:minecart")),
        (
            "Pos",
            Tag::List([3.5f64, 16.0, 7.5].map(|v| Tag::Double(v.to_bits())).into()),
        ),
        ("UUID", Tag::IntArray(vec![1, 2, 3, 4])),
    ]);
    write_old_record(
        &save,
        "region",
        Tag::compound([
            ("DataVersion", Tag::Int(2586)),
            (
                "Level",
                Tag::compound([
                    ("xPos", Tag::Int(0)),
                    ("zPos", Tag::Int(0)),
                    ("Status", string("full")),
                    ("Biomes", Tag::IntArray(vec![1; 1024])),
                    ("Sections", Tag::List(vec![section])),
                    ("Entities", Tag::List(vec![minecart])),
                ]),
            ),
        ]),
    );
    let id = |e: &Tag| text(e.get("id"));
    let chunk = save
        .load(0, 0, assets.registry, &biomes)
        .expect("the chunk loads");
    let ids: Vec<_> = chunk.entities.iter().map(id).collect();
    assert_eq!(ids, [Some("minecraft:minecart".to_owned())]);
    // There is no `entities/` record for it: these are all it has.
    assert!(save.load_entities(0, 0).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&world);
}

/// `world_gen_unstable` over the view of the packs the world selects.
fn unstable(world: &Path) -> bool {
    world_gen_unstable(world, &world_data_packs(world))
}
