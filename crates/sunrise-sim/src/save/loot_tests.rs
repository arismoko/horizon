use super::*;

pub(super) fn fixture() -> (BlockRegistry, Biomes) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "sunrise-opened-loot-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let maps = root.join("assets/minecraft/textures/colormap");
    fs::create_dir_all(&maps).unwrap();
    // Original one-pixel fixtures: chunk persistence only needs biome names.
    for name in ["grass", "foliage", "dry_foliage"] {
        let file = fs::File::create(maps.join(format!("{name}.png"))).unwrap();
        let mut encoder = png::Encoder::new(file, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[0, 0, 0])
            .unwrap();
    }
    let folder = root.join("data/minecraft/worldgen/biome");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("plains.json"), "{}").unwrap();
    let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))
    .unwrap();
    fs::remove_dir_all(&root).unwrap();
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(vec![
                Tag::compound([("id", string("minecraft:air"))]),
                Tag::compound([("id", string("minecraft:chest"))]),
            ]),
        ),
        (
            "defaults",
            Tag::compound([
                ("minecraft:air", Tag::Int(0)),
                ("minecraft:chest", Tag::Int(1)),
            ]),
        ),
    ]))
    .unwrap();
    (registry, biomes)
}

#[test]
fn queued_chunk_stacks_save_against_the_loaded_item_prototypes() {
    let (registry, biomes) = fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let scratch = tempfile::tempdir().unwrap();
    fs::create_dir_all(scratch.path().join("data/minecraft/loot_table")).unwrap();
    fs::write(scratch.path().join("items.json"), r#"{"minecraft:red_banner":{"minecraft:banner_patterns":[]},"minecraft:diamond_pickaxe":{"minecraft:damage":0}}"#).unwrap();
    let defaults = sunrise_world::item_defaults::read(scratch.path()).unwrap();
    let items = Arc::new(
        crate::items::Items::catalog(
            scratch.path(),
            &sunrise_world::loot::PackOverlay::NONE,
            &defaults,
        )
        .unwrap(),
    );
    let parse = |value| crate::item_data::parse_stack(value).unwrap();
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(1, 64, 2, 1);
    chunk.block_entities.push(((1, 64, 2), crate::containers::BlockEntity::Chest(vec![
        parse(r#"{id:"minecraft:red_banner",count:1,components:{"minecraft:banner_patterns":[]}}"#),
        parse(r#"{id:"minecraft:diamond_pickaxe",count:1,components:{"minecraft:damage":0,"minecraft:custom_data":{keep:87}}}"#),
    ])));
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let root = scratch.path().join("world");
        let save = SaveDir::new(root.clone(), LevelShape::OVERWORLD, Some(items));
        save.save_later(&[Arc::new(chunk)], registry, &biomes)
            .unwrap();
        save.flush().unwrap();
        let disk = SaveDir::new(root, LevelShape::OVERWORLD, None);
        let saved = disk.raw_chunk(0, 0).unwrap();
        let Some(Tag::List(entities)) = saved.tag.get("block_entities") else {
            panic!("block entities")
        };
        let expected = sunrise_core::nbt::parse_snbt(r#"[{Slot:0b,id:"minecraft:red_banner",count:1},{Slot:1b,id:"minecraft:diamond_pickaxe",count:1,components:{"minecraft:custom_data":{keep:87}}}]"#).unwrap().0;
        assert_eq!(entities[0].get("Items"), Some(&expected));
    });
}

#[test]
fn loaded_source_stacks_are_normalised_after_overlay_without_losing_metadata() {
    let (registry, biomes) = fixture();
    let scratch = tempfile::tempdir().unwrap();
    fs::create_dir_all(scratch.path().join("data/minecraft/loot_table")).unwrap();
    fs::write(
        scratch.path().join("items.json"),
        r#"{"minecraft:red_banner":{"minecraft:banner_patterns":[]}}"#,
    )
    .unwrap();
    let defaults = sunrise_world::item_defaults::read(scratch.path()).unwrap();
    let items = crate::items::Items::catalog(
        scratch.path(),
        &sunrise_world::loot::PackOverlay::NONE,
        &defaults,
    )
    .unwrap();
    let source = sunrise_core::nbt::parse_snbt(r#"{id:"minecraft:chest",x:1,y:64,z:2,CustomName:{text:"Keep me",color:"gold"},lock:{items:"minecraft:tripwire_hook"},components:{"minecraft:custom_data":{opaque:87}},Items:[{Slot:0b,id:"minecraft:red_banner",count:1,components:{"minecraft:banner_patterns":[],"minecraft:custom_data":{keep:99},"missing_mod:state":{keep:123}}},{Slot:1b,id:"minecraft:stone",count:1},{Slot:2b,id:"minecraft:red_banner",count:150}]}"#).unwrap().0;
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(1, 64, 2, 1);
    let mut encoded = encode(&chunk, &registry, &biomes, &items);
    let Tag::Compound(root) = &mut encoded.tag else {
        panic!("chunk compound")
    };
    root.insert("block_entities".into(), Tag::List(vec![source.clone()]));
    let mut loaded = decode(&encoded, &registry, &biomes).unwrap();
    assert_eq!(
        loaded.saved.as_ref().unwrap().block_entity_sources[0].1,
        source
    );
    let crate::containers::BlockEntity::Chest(slots) = &mut loaded.block_entities[0].1 else {
        panic!("chest")
    };
    slots[1] = Some(crate::items::ItemStack::new("minecraft:gold_ingot", 2));
    let saved = encode(&loaded, &registry, &biomes, &items);
    let Some(Tag::List(entities)) = saved.tag.get("block_entities") else {
        panic!("block entities")
    };
    let saved = &entities[0];
    // The restored count-150 entry is dropped, not converted into a count-one stack.
    // The valid unchanged stack keeps even a component absent from the loaded catalog.
    let expected = sunrise_core::nbt::parse_snbt(r#"[{Slot:0b,id:"minecraft:red_banner",count:1,components:{"minecraft:custom_data":{keep:99},"missing_mod:state":{keep:123}}},{Slot:1b,id:"minecraft:gold_ingot",count:2}]"#).unwrap().0;
    assert_eq!(saved.get("Items"), Some(&expected));
    for key in ["CustomName", "lock", "components"] {
        assert_eq!(saved.get(key), source.get(key), "{key}");
    }
}

fn reload_loot_case(
    registry: &BlockRegistry,
    biomes: &Biomes,
    table: &str,
    seed: i64,
    mode: u8,
) -> Chunk {
    let pos = (1, 64, 2);
    let mut slots = vec![None; 27];

    let source = Tag::compound([
        ("id", string("minecraft:chest")),
        ("x", Tag::Int(pos.0)),
        ("y", Tag::Int(pos.1)),
        ("z", Tag::Int(pos.2)),
        ("LootTable", string(table)),
        ("LootTableSeed", Tag::Long(seed)),
        ("CustomName", string("{\"text\":\"Keep me\"}")),
    ]);
    // A loaded chest adopts its external name before the loot transition.
    let custom_name = source
        .get("CustomName")
        .and_then(crate::block_entity_nbt::component)
        .expect("fixture's admitted external name");
    let mut extras = ChunkExtras {
        block_entity_sources: vec![(pos, source)],
        pending_loot: [(
            pos,
            PendingContainerLoot {
                table: string(table),
                seed: Some(Tag::Long(seed)),
            },
        )]
        .into(),
        ..Default::default()
    };
    // Exactly the production transition used when the player opens the chest.
    if mode != 0 {
        extras.pending_loot.remove(&pos);
        slots.fill(None);
        if mode == 2 {
            slots[0] = Some(crate::items::ItemStack::new("minecraft:gold_ingot", 1));
        }
    }
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(pos.0, pos.1, pos.2, 1);
    chunk
        .block_entities
        .push((pos, crate::containers::BlockEntity::Chest(slots)));
    chunk.block_names.push((pos, custom_name));
    chunk.saved = Some(Arc::new(extras));
    let encoded = encode(&chunk, registry, biomes, &Default::default());
    let bytes = nbt::write_named(&encoded, Limits::default()).unwrap();
    let (encoded, used) = nbt::read_named(&bytes, Limits::default()).unwrap();
    assert_eq!(used, bytes.len());
    decode(&encoded, registry, biomes).unwrap()
}

#[test]
fn emptied_opened_loot_stays_empty_after_chunk_save_and_reload() {
    let (registry, biomes) = fixture();
    let loaded = reload_loot_case(&registry, &biomes, "minecraft:chests/ruined_portal", 42, 1);
    let source = &loaded.saved.as_ref().unwrap().block_entity_sources[0].1;
    assert!(
        source.get("LootTable").is_none(),
        "opened chest keeps a rerollable loot table: {source:?}"
    );
    assert!(source.get("LootTableSeed").is_none());
    assert_eq!(
        text(source.get("CustomName")).as_deref(),
        Some("{\"text\":\"Keep me\"}")
    );
    let crate::containers::BlockEntity::Chest(slots) = &loaded.block_entities[0].1 else {
        panic!("chest");
    };
    assert!(slots.iter().all(Option::is_none));
}

#[test]
fn unopened_loot_tags_round_trip_with_zero_seed_omitted() {
    let (registry, biomes) = fixture();
    let pos = (1, 64, 2);
    for (seed, expected_seed) in [
        (Some(Tag::Long(37)), Some(Tag::Long(37))),
        (None, None),
        (Some(Tag::Long(0)), None),
    ] {
        let pending = PendingContainerLoot {
            table: string("minecraft:chests/simple_dungeon"),
            seed,
        };
        let expected = |p: &PendingContainerLoot| {
            let mut map = Compound::new();
            map.insert("LootTable".into(), p.table.clone());
            if let Some(seed) = &p.seed {
                map.insert("LootTableSeed".into(), seed.clone());
            }
            nbt::write_named(
                &NamedTag {
                    name: "".into(),
                    tag: Tag::Compound(map),
                },
                Limits::default(),
            )
            .unwrap()
        };
        let bytes = expected(&PendingContainerLoot {
            table: pending.table.clone(),
            seed: expected_seed,
        });
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(1, 64, 2, 1);
        chunk
            .block_entities
            .push((pos, crate::containers::BlockEntity::Chest(vec![None; 27])));
        chunk.saved = Some(Arc::new(ChunkExtras {
            pending_loot: [(pos, pending)].into(),
            ..Default::default()
        }));
        for _ in 0..2 {
            let encoded = encode(&chunk, &registry, &biomes, &Default::default());
            let disk = nbt::write_named(&encoded, Limits::default()).unwrap();
            let (encoded, _) = nbt::read_named(&disk, Limits::default()).unwrap();
            let tag = &match encoded.tag.get("block_entities").unwrap() {
                Tag::List(tags) => tags,
                _ => panic!(),
            }[0];
            assert_eq!(
                expected(&PendingContainerLoot::from_tag(tag).unwrap()),
                bytes
            );
            assert!(tag.get("Items").is_none());
            chunk = decode(&encoded, &registry, &biomes).unwrap();
            assert!(
                chunk.block_entities[0]
                    .1
                    .slots()
                    .iter()
                    .all(Option::is_none)
            );
            let extras = chunk.saved.as_ref().unwrap();
            assert_eq!(expected(extras.pending_loot.get(&pos).unwrap()), bytes);
            assert!(extras.block_entity_sources[0].1.get("LootTable").is_none());
        }
    }
}

#[test]
fn retained_loot_and_live_items_are_written_before_chunk_metadata() {
    let (registry, biomes) = fixture();
    let pos = (1, 64, 2);
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(pos.0, pos.1, pos.2, 1);
    let mut slots = vec![None; 27];
    slots[0] = Some(crate::items::ItemStack::new("minecraft:gold_ingot", 1));
    chunk
        .block_entities
        .push((pos, crate::containers::BlockEntity::Chest(slots)));
    chunk.saved = Some(Arc::new(ChunkExtras {
        pending_loot: [(
            pos,
            PendingContainerLoot {
                table: string("minecraft:chests/simple_dungeon"),
                seed: Some(Tag::Long(37)),
            },
        )]
        .into(),
        ..Default::default()
    }));

    let limits = Limits::default();
    let encoded = encode(&chunk, &registry, &biomes, &Default::default());
    let bytes = nbt::write_named(&encoded, limits).unwrap();
    let (read, _) = nbt::read_named(&bytes, limits).unwrap();
    let Some(Tag::List(entities)) = read.tag.get("block_entities") else {
        panic!("missing block entities");
    };
    let Tag::Compound(chest) = &entities[0] else {
        panic!("missing chest compound");
    };
    // The Java spread hashes of `Items` and `z` share bucket 10 in this
    // default-sized table. Appending Items after normalization would put z first.
    let collision: Vec<_> = chest
        .keys()
        .map(|key| key.to_string().unwrap())
        .filter(|key| matches!(key.as_str(), "Items" | "z"))
        .collect();
    assert_eq!(collision, ["Items", "z"]);
    assert_eq!(entities[0].get("LootTableSeed"), Some(&Tag::Long(37)));
    assert!(matches!(entities[0].get("Items"), Some(Tag::List(items)) if items.len() == 1));
}

#[test]
#[ignore = "run by tools/test_opened_loot_parity.py"]
fn opened_loot_parity_fixture() {
    use std::io::Write;
    let (registry, biomes) = fixture();
    let input = fs::read_to_string(std::env::var("SUNRISE_OPENED_LOOT_INPUT").unwrap()).unwrap();
    let mut output =
        fs::File::create(std::env::var("SUNRISE_OPENED_LOOT_OUTPUT").unwrap()).unwrap();
    for line in input.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        let mode = fields[2].parse().unwrap();
        let chunk = reload_loot_case(
            &registry,
            &biomes,
            fields[0],
            fields[1].parse().unwrap(),
            mode,
        );
        let source = encode(&chunk, &registry, &biomes, &Default::default());
        let Tag::List(tags) = source.tag.get("block_entities").unwrap() else {
            panic!()
        };
        let source = &tags[0];
        let crate::containers::BlockEntity::Chest(slots) = &chunk.block_entities[0].1 else {
            panic!("chest");
        };
        let count = if mode == 0 {
            "unopened".into()
        } else {
            slots
                .iter()
                .flatten()
                .map(|s| s.count)
                .sum::<u32>()
                .to_string()
        };
        writeln!(
            output,
            "{} {} {count}",
            u8::from(source.get("LootTable").is_some()),
            u8::from(source.get("LootTableSeed").is_some())
        )
        .unwrap();
    }
}

#[test]
fn eager_save_edits_reach_disk_during_ticks_and_again_after_cooldown() {
    use crate::world::ModifiedChunks;
    use std::time::{Duration, Instant};
    let (registry, biomes) = fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let dir = std::env::temp_dir().join(format!("sunrise-eager-edits-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
    let mut chunk = Chunk::empty(0, 0);
    let mut dirty = ModifiedChunks::default();
    let now = Instant::now();
    for tick in [0, 1, 199, 200] {
        if tick <= 1 {
            chunk.set(1, 64, 2, if tick == 0 { 1 } else { 0 });
            dirty.insert((0, 0));
        }
        dirty.save_eagerly(
            now + Duration::from_millis(tick * 50),
            || save.active_chunk_writes() < 128,
            |_| Some(true),
            |_, _| {
                save.save_later(&[Arc::new(chunk.clone())], registry, &biomes)
                    .unwrap();
                true
            },
        );
        save.flush().unwrap(); // only a barrier: no quit/autosave submission
        let reopened = SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
        assert_eq!(
            reopened
                .load(0, 0, registry, &biomes)
                .unwrap()
                .get(1, 64, 2),
            if tick < 200 { 1 } else { 0 }
        );
    }
    drop(save);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn container_limit_survives_chunk_save_and_reload() {
    let (registry, biomes) = fixture();
    let pos = (1, 64, 2);
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(1, 64, 2, 1);
    chunk
        .block_entities
        .push((pos, crate::containers::BlockEntity::Chest(vec![None; 27])));
    chunk.saved = Some(Arc::new(ChunkExtras {
        block_entity_sources: vec![(pos, Tag::compound([("sunrise:stack_limit", Tag::Int(10))]))],
        ..Default::default()
    }));
    let encoded = encode(&chunk, &registry, &biomes, &Default::default());
    let loaded = decode(&encoded, &registry, &biomes).unwrap();
    assert_eq!(
        loaded.saved.as_ref().unwrap().block_entity_sources[0]
            .1
            .get("sunrise:stack_limit"),
        Some(&Tag::Int(10))
    );
}

#[test]
fn unmodified_container_nbt_has_no_stack_limit() {
    let (registry, biomes) = fixture();
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(1, 64, 2, 1);
    chunk.block_entities.push((
        (1, 64, 2),
        crate::containers::BlockEntity::Chest(vec![None; 27]),
    ));
    let loaded = decode(
        &encode(&chunk, &registry, &biomes, &Default::default()),
        &registry,
        &biomes,
    )
    .unwrap();
    assert_eq!(loaded.saved.as_ref().unwrap().block_entity_sources.len(), 1);
    for (_, tag) in &loaded.saved.as_ref().unwrap().block_entity_sources {
        assert!(tag.get("sunrise:stack_limit").is_none());
    }
}

#[test]
#[ignore = "daily"]
fn generated_structure_chest_stays_pending_after_save_and_reload() {
    use crate::vanillagen::{TerrainResources, VanillaTerrain};
    use sunrise_level::chunk_streamer::{ChunkGenerator, Proto};
    let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let assets = crate::assets::Assets::load(&root).unwrap();
    let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        root.clone(),
    ))
    .unwrap();
    let items = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ServerDataLoad,
    )
    .unwrap();
    // Existing generation_saves::saved_proto_block_entities_keep_their_id fixture:
    // seed 12345, cold ocean ruin centered on chunk (43, 21).
    let generator = VanillaTerrain::for_dimension(
        TerrainResources {
            data: None,
            dir: &root.join("worldgen"),
            registry: assets.registry,
            biomes: &biomes,
            item_data: &items,
        },
        12345,
        "minecraft:overworld",
    )
    .unwrap();
    let mut worker = generator.worker();
    let mut region: [Proto; 9] = std::array::from_fn(|i| {
        generator
            .proto(&mut worker, 43 + i as i32 % 3 - 1, 21 + i as i32 / 3 - 1)
            .unwrap()
    });
    generator.decorate(&mut worker, &mut region);
    let chunk = generator.finish(region.into_iter().nth(4).unwrap());
    const CHEST: (i32, i32, i32) = (692, 60, 348);
    const TABLE: &str = "minecraft:chests/underwater_ruin_small";
    const SEED: i64 = 8294534313461660897;
    let expected = PendingContainerLoot {
        table: string(TABLE),
        seed: Some(Tag::Long(SEED)),
    };
    assert_eq!(
        chunk.saved.as_ref().unwrap().pending_loot.get(&CHEST),
        Some(&expected)
    );
    assert!(
        chunk
            .block_entities
            .iter()
            .find(|(p, _)| *p == CHEST)
            .unwrap()
            .1
            .slots()
            .iter()
            .all(Option::is_none)
    );
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "generated-pending-loot-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
    save.save(&[Arc::new(chunk)], assets.registry, &biomes)
        .unwrap();
    save.flush().unwrap();
    let reopened = SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
    let reloaded = reopened.load(43, 21, assets.registry, &biomes).unwrap();
    assert_eq!(
        reloaded.saved.as_ref().unwrap().pending_loot.get(&CHEST),
        Some(&expected)
    );
    assert!(
        reloaded
            .block_entities
            .iter()
            .find(|(p, _)| *p == CHEST)
            .unwrap()
            .1
            .slots()
            .iter()
            .all(Option::is_none)
    );
    let saved = encode(&reloaded, assets.registry, &biomes, &Default::default());
    let Tag::List(entities) = saved.tag.get("block_entities").unwrap() else {
        panic!()
    };
    let chest = entities
        .iter()
        .find(|e| {
            number(e.get("x")) == Some(692)
                && number(e.get("y")) == Some(60)
                && number(e.get("z")) == Some(348)
        })
        .unwrap();
    assert_eq!(chest.get("LootTable"), Some(&string(TABLE)));
    assert_eq!(chest.get("LootTableSeed"), Some(&Tag::Long(SEED)));
    assert!(chest.get("Items").is_none());
    drop(reopened);
    drop(save);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_loot_table_that_is_not_an_identifier_is_dropped_with_its_seed() {
    // RandomizableContainer.tryLoadLootTable reads LootTable with the identifier codec: a string
    // that does not parse leaves no table, and with no table the seed is not saved either.
    let (registry, biomes) = fixture();
    let pos = (1, 64, 2);
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(pos.0, pos.1, pos.2, 1);
    chunk
        .block_entities
        .push((pos, crate::containers::BlockEntity::Chest(vec![None; 27])));
    let mut encoded = encode(&chunk, &registry, &biomes, &Default::default());
    let Tag::Compound(root) = &mut encoded.tag else {
        panic!("chunk root");
    };
    let Some(Tag::List(entities)) = root.get_mut(&"block_entities".into()) else {
        panic!("no block entities");
    };
    let Tag::Compound(chest) = &mut entities[0] else {
        panic!("chest");
    };
    chest.insert("LootTable".into(), string("Bad Name!"));
    chest.insert("LootTableSeed".into(), Tag::Long(73));
    let loaded = decode(&encoded, &registry, &biomes).unwrap();
    assert!(
        loaded
            .saved
            .as_ref()
            .is_none_or(|extras| extras.pending_loot.is_empty())
    );
    let saved = encode(&loaded, &registry, &biomes, &Default::default());
    let Some(Tag::List(entities)) = saved.tag.get("block_entities") else {
        panic!("no block entities");
    };
    assert_eq!(
        (
            entities[0].get("LootTable"),
            entities[0].get("LootTableSeed")
        ),
        (None, None)
    );
}
