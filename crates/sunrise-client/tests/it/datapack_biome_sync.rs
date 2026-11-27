//! A data-pack biome is in the registry a server sends, with its data, at the
//! place the chunk encoder numbers it by, and a client rebuilds it from that
//! data.
use serde_json::Value;
use std::path::PathBuf;
use sunrise_game_data::scoreboard::json_to_tag;
use sunrise_level::{
    biome::{BiomeColors, Biomes},
    chunk::LevelShapes,
};
use sunrise_sim::lan::data::{BIOME_REGISTRY, SyncData, client_biomes};

/// Removes the folder when the test ends, passed or not.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

const BASE_BIOME: &str = r##"{"has_precipitation":true,"temperature":0.5,"downfall":0.5,"effects":{"water_color":"#0000ff"},"carvers":[],"features":[]}"##;

/// A resource folder with two biomes and 1x1 colormaps, loaded.
fn write_base(root: &std::path::Path) -> Biomes {
    let maps = root.join("assets/minecraft/textures/colormap");
    let biome_dir = root.join("data/minecraft/worldgen/biome");
    std::fs::create_dir_all(&maps).unwrap();
    std::fs::create_dir_all(&biome_dir).unwrap();
    std::fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
    std::fs::write(root.join("registries.json"), "{}").unwrap();
    std::fs::write(root.join("items.json"), "{}").unwrap();
    for name in ["grass", "foliage", "dry_foliage"] {
        let file = std::fs::File::create(maps.join(format!("{name}.png"))).unwrap();
        let mut png = png::Encoder::new(file, 1, 1);
        png.set_color(png::ColorType::Rgb);
        png.set_depth(png::BitDepth::Eight);
        let mut writer = png.write_header().unwrap();
        writer.write_image_data(&[1, 2, 3]).unwrap();
    }
    for name in ["plains", "river"] {
        std::fs::write(biome_dir.join(format!("{name}.json")), BASE_BIOME).unwrap();
    }
    Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))
    .unwrap()
}

#[test]
fn a_pack_biome_is_sent_with_its_data_and_read_back_by_a_client() {
    let root = std::env::temp_dir().join(format!("sunrise-sync-biomes-{}", std::process::id()));
    let _scratch = Scratch(root.clone());
    let base = write_base(&root);
    let mut pack: Value = serde_json::from_str(BASE_BIOME).unwrap();
    pack["effects"]["grass_color"] = "#123456".into();
    let particle = serde_json::json!([{"particle":{"type":"minecraft:ash"},"probability":1}]);
    pack["attributes"]["minecraft:visual/ambient_particles"] = particle.clone();
    let mut append = pack.clone();
    append["attributes"]["minecraft:visual/ambient_particles"] =
        serde_json::json!({"modifier":"append", "argument":particle});
    let world = base
        .with_packs([
            ("minecraft:tundra_x", &pack),
            ("minecraft:zz_append", &append),
        ])
        .unwrap();
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let items = sunrise_items::items::Items::catalog(
        &root,
        &sunrise_world::loot::PackOverlay::NONE,
        &defaults,
    )
    .unwrap();
    let sync = SyncData::load_with(
        &root,
        &sunrise_world::loot::PackOverlay::NONE,
        Some(&world),
        &defaults,
        &sunrise_sim::crafting::ResolvedSources::load(
            &root,
            &items,
            &sunrise_world::loot::PackOverlay::NONE,
            &server_load(),
        )
        .unwrap(),
    )
    .unwrap();

    // The encoder numbers a biome by its place in this list: none may be
    // missing, and the pack's is not the first.
    let registry = &sync.dynamic.registries[BIOME_REGISTRY];
    let names: Vec<_> = registry.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "minecraft:plains",
            "minecraft:river",
            "minecraft:tundra_x",
            "minecraft:zz_append"
        ]
    );
    assert!(world.list.iter().all(|b| names.contains(&b.name.as_str())));

    // A client that knows the vanilla pack still gets the pack biome's data.
    let sent = sync.registry_packets(true, &LevelShapes::VANILLA);
    use sunrise_protocol::{buf::Reader, packets::configuration::Clientbound};
    let biomes = sent
        .into_iter()
        .find(|r| r.registry == BIOME_REGISTRY)
        .unwrap();
    let bytes = Clientbound::RegistryData(biomes).encode();
    let mut reader = Reader::new(&bytes);
    let packet_id = reader.varint().unwrap();
    let Clientbound::RegistryData(biomes) = Clientbound::decode_reader(packet_id, reader).unwrap()
    else {
        panic!("expected the biome registry packet");
    };
    assert!(biomes.entries[0].data.is_none(), "a known one is not sent");
    assert!(biomes.entries[2].data.is_some(), "the pack's biome is sent");

    // The client builds its table from what arrived: the server's id for
    // the name, and the pack's colour.
    let received: Vec<_> = biomes
        .entries
        .iter()
        .map(|e| (e.id.clone(), e.data.clone()))
        .collect();
    let client = client_biomes(&base, &received)
        .unwrap()
        .expect("the pack's biome is new to the client");
    let id = client.id("minecraft:tundra_x");
    assert_eq!(id, world.id("minecraft:tundra_x"));
    assert_eq!(client.get(id).grass, 0x123456);
    assert_ne!(
        client.get(id).grass,
        client.get(client.id("minecraft:plains")).grass
    );

    let ambient_attr = sunrise_world::environment_attributes::attribute(
        sunrise_world::environment_attributes::AMBIENT_PARTICLES,
    )
    .unwrap();
    for name in ["minecraft:tundra_x", "minecraft:zz_append"] {
        let particles = match client.get(client.id(name)).attributes.get(ambient_attr) {
            Some(
                sunrise_world::environment_attributes::Entry::Override(
                    sunrise_world::environment_attributes::Value::AmbientParticles(particles),
                )
                | sunrise_world::environment_attributes::Entry::AppendAmbientParticles(particles),
            ) => particles,
            other => panic!("{name}: expected visual/ambient_particles, got {other:?}"),
        };
        assert_eq!(particles.len(), 1, "{name}");
        assert_eq!(
            *particles[0].particle,
            sunrise_world::particle_options::ParticleOptions::Simple(
                sunrise_world::particle_type::ParticleType::Ash,
            ),
            "{name}"
        );
        assert_eq!(
            particles[0].probability.to_bits(),
            1.0_f32.to_bits(),
            "{name}"
        );
    }

    // Data a client already has in its own file is nothing new, sent or not.
    let same: Vec<_> = sync
        .registry_packets(false, &LevelShapes::VANILLA)
        .into_iter()
        .find(|r| r.registry == BIOME_REGISTRY)
        .unwrap()
        .entries
        .into_iter()
        .filter(|entry| base.contains(&entry.id))
        .map(|entry| (entry.id, entry.data))
        .collect();
    assert!(client_biomes(&base, &same).unwrap().is_none());

    // Server data overrides even a locally known biome.
    let other: Value = serde_json::from_str(r##"{"temperature":1.75,"downfall":0.125,"has_precipitation":false,"effects":{"grass_color":"#654321"}}"##).unwrap();
    let other = json_to_tag(&other);
    let known: Vec<_> = received
        .iter()
        .filter(|(n, _)| base.contains(n))
        .map(|(n, _)| (n.clone(), Some(other.clone())))
        .collect();
    let replaced = client_biomes(&base, &known).unwrap().unwrap();
    let climate = replaced.properties("minecraft:plains").unwrap();
    assert_eq!(climate.temperature, 1.75);
    assert_eq!(climate.downfall, 0.125);
    assert!(!climate.has_precipitation);

    // Replacements also survive alongside newly added biomes.
    let mut mixed = known.clone();
    mixed.extend(received.iter().filter(|(n, _)| !base.contains(n)).cloned());
    let client = client_biomes(&base, &mixed).unwrap().unwrap();
    let plains = base.get(base.id("minecraft:plains")).grass;
    assert_ne!(plains, 0x654321);
    assert_eq!(client.get(client.id("minecraft:plains")).grass, 0x654321);
    assert_eq!(client.get(client.id("minecraft:tundra_x")).grass, 0x123456);
}

#[test]
fn a_data_less_biome_the_game_lacks_keeps_its_own_key_and_renders_as_plains() {
    let root =
        std::env::temp_dir().join(format!("sunrise-unresolved-biomes-{}", std::process::id()));
    let _scratch = Scratch(root.clone());
    let base = write_base(&root);
    let registry = vec![
        ("minecraft:plains".to_owned(), None),
        ("minecraft:river".to_owned(), None),
        ("gone_mod:highland".to_owned(), None),
    ];
    let client = client_biomes(&base, &registry).unwrap().unwrap();
    let id = client.id("gone_mod:highland");
    // Not plains' slot: the stored biome reads back as the server's key.
    assert_ne!(id, client.id("minecraft:plains"));
    assert_eq!(client.name(id), "gone_mod:highland");
    assert_eq!(
        client.registered_keys()[usize::from(id)].to_string(),
        "gone_mod:highland"
    );
    assert!(client.is_unresolved("gone_mod:highland"));
    assert!(!client.is_unresolved("minecraft:river"));
    // It renders as plains.
    let plains = client.get(client.id("minecraft:plains"));
    assert_eq!(client.get(id).grass, plains.grass);
    assert_eq!(client.get(id).temperature, plains.temperature);
    // Only names the game defines: no table at all.
    assert!(client_biomes(&base, &registry[..2]).unwrap().is_none());
}

#[test]
fn wire_load_without_a_world_table_uses_only_prepared_vanilla_biomes() {
    use sunrise_world::loot::PackOverlay;
    let root =
        std::env::temp_dir().join(format!("sunrise-base-wire-biomes-{}", std::process::id()));
    let _scratch = Scratch(root.clone());
    write_base(&root);
    let mut invalid: Value = serde_json::from_str(BASE_BIOME).unwrap();
    invalid["carvers"] = serde_json::json!({});
    let wrapped = serde_json::json!({"neoforge:conditions":[{"type":"neoforge:always"}], "neoforge:value":invalid});
    std::fs::create_dir_all(root.join("worldgen")).unwrap();
    std::fs::write(
        root.join("worldgen/biomes.json"),
        serde_json::json!({"demo:prepared":wrapped.to_string()}).to_string(),
    )
    .unwrap();
    let files = [(
        "worldgen/biome/demo:overlay".into(),
        vec![wrapped.to_string().into_bytes()],
    )];
    let overlay = PackOverlay::new(&files);
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let items = sunrise_items::items::Items::catalog(&root, &overlay, &defaults).unwrap();
    let sources =
        sunrise_sim::crafting::ResolvedSources::load(&root, &items, &overlay, &server_load())
            .unwrap();
    let wire = SyncData::load(&root, &overlay, &defaults, &sources).unwrap();
    let network_biomes = wire
        .registry_packets(false, &LevelShapes::VANILLA)
        .into_iter()
        .find(|registry| registry.registry == BIOME_REGISTRY)
        .unwrap();
    assert_eq!(
        network_biomes
            .entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["minecraft:plains", "minecraft:river"]
    );
}

#[test]
fn a_server_replacement_of_a_known_biome_keeps_its_id_and_changes_its_data() {
    let root = std::env::temp_dir().join(format!("sunrise-replace-biomes-{}", std::process::id()));
    let _scratch = Scratch(root.clone());
    let base = write_base(&root);
    let replaced: Value = serde_json::from_str(
        r##"{"effects":{"water_color":"#112233"},"attributes":{"minecraft:visual/ambient_particles":[{"particle":{"type":"minecraft:ash"},"probability":0.25}]}}"##,
    )
    .unwrap();
    let received = vec![
        ("minecraft:plains".to_owned(), Some(json_to_tag(&replaced))),
        ("minecraft:river".to_owned(), None),
    ];
    let client = client_biomes(&base, &received)
        .unwrap()
        .expect("plains differs from the client's file");
    assert_eq!(client.id("minecraft:plains"), base.id("minecraft:plains"));
    let plains = client.get(client.id("minecraft:plains"));
    assert_eq!(plains.water, 0x112233);
    let ambient_attr = sunrise_world::environment_attributes::attribute(
        sunrise_world::environment_attributes::AMBIENT_PARTICLES,
    )
    .unwrap();
    let Some(sunrise_world::environment_attributes::Entry::Override(
        sunrise_world::environment_attributes::Value::AmbientParticles(particles),
    )) = plains.attributes.get(ambient_attr)
    else {
        panic!("replaced plains must carry visual/ambient_particles");
    };
    assert_eq!(particles.len(), 1);
    assert_eq!(
        *particles[0].particle,
        sunrise_world::particle_options::ParticleOptions::Simple(
            sunrise_world::particle_type::ParticleType::Ash,
        )
    );
    assert_eq!(particles[0].probability, 0.25);
    // The tables that read the biome's file see the replacement; river is untouched.
    assert!(client.pack_file("minecraft:plains").is_some());
    assert!(client.pack_file("minecraft:river").is_none());
}

#[test]
fn a_biome_the_mods_modifiers_changed_is_sent_to_a_client_that_knows_the_vanilla_pack() {
    let root = std::env::temp_dir().join(format!("sunrise-modified-biomes-{}", std::process::id()));
    let _scratch = Scratch(root.clone());
    let base = write_base(&root);
    // What `write_state` saves: the modifiers changed plains, and the root holds the result.
    std::fs::write(
        root.join("biome-modifiers.json"),
        r#"{"unmodified":{"minecraft:plains":"{}"}}"#,
    )
    .unwrap();
    let mut plains: Value = serde_json::from_str(BASE_BIOME).unwrap();
    plains["effects"]["grass_color"] = "#112233".into();
    std::fs::write(
        root.join("data/minecraft/worldgen/biome/plains.json"),
        plains.to_string(),
    )
    .unwrap();
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let items = sunrise_items::items::Items::catalog(
        &root,
        &sunrise_world::loot::PackOverlay::NONE,
        &defaults,
    )
    .unwrap();
    let sync = SyncData::load_with(
        &root,
        &sunrise_world::loot::PackOverlay::NONE,
        Some(
            &Biomes::load_for_server(&sunrise_assets::resource_source::ResourceRoot::directory(
                root.clone(),
            ))
            .unwrap(),
        ),
        &defaults,
        &sunrise_sim::crafting::ResolvedSources::load(
            &root,
            &items,
            &sunrise_world::loot::PackOverlay::NONE,
            &server_load(),
        )
        .unwrap(),
    )
    .unwrap();
    let sent = sync.registry_packets(true, &LevelShapes::VANILLA);
    let biomes = sent.iter().find(|r| r.registry == BIOME_REGISTRY).unwrap();
    let entry = |name: &str| biomes.entries.iter().find(|e| e.id == name).unwrap();
    assert!(
        entry("minecraft:plains").data.is_some(),
        "the changed one is sent"
    );
    assert!(
        entry("minecraft:river").data.is_none(),
        "an unchanged one is not"
    );
    // The client takes the server's colour over its own file's.
    let received: Vec<_> = biomes
        .entries
        .iter()
        .map(|e| (e.id.clone(), e.data.clone()))
        .collect();
    let client = client_biomes(&base, &received)
        .unwrap()
        .expect("plains differs");
    assert_eq!(client.get(client.id("minecraft:plains")).grass, 0x112233);
}

#[test]
fn recolouring_twice_starts_from_the_biome_files_each_time() {
    let root = std::env::temp_dir().join(format!("sunrise-recolor-biomes-{}", std::process::id()));
    let _scratch = Scratch(root.clone());
    let base = write_base(&root);
    let grass = |b: &Biomes| b.get(b.id("minecraft:plains")).grass;
    let darker = |_: &str, c: &mut BiomeColors| c.grass -= 0x10;
    let once = base.recolored_from(&base, darker).expect("colours changed");
    assert_eq!(grass(&once), grass(&base) - 0x10);
    // The same colours again, asked from the base: nothing to rebuild.
    assert!(base.recolored_from(&once, darker).is_none());
    // Other colours replace the first ones rather than add to them.
    let lighter = |_: &str, c: &mut BiomeColors| c.grass += 0x10;
    let next = base.recolored_from(&once, lighter).unwrap();
    assert_eq!(grass(&next), grass(&base) + 0x10);
}

fn server_load() -> sunrise_world::native_load_context::LoadContext {
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
    LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    )
}
