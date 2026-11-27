//! #792 stage 3: the general record's live load and save, through the game
//! fixture. The record codec's own tests (`general.rs`) cover the format;
//! these cover what the game does with it: publish a tree through the same
//! join boundary a fresh spawn takes, remember its identities, save one
//! record per root (never for `noSave`), and refuse a whole tree — codec
//! fault, structural fault or a UUID a live entity already owns — with zero
//! published rows.
use std::collections::{HashMap, HashSet};
use sunrise_game_data::text::Text;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay::game::server_side::persist::ChunkKey;
use sunrise_gameplay_common::game::general::{
    GeneralEntityRecord, GeneralRow, GeneralTypeEnvelope, Tag, Target, Vec3, format, limits,
    nbt_size, record_from_compound,
};
use sunrise_gameplay_common::game::test_fixtures::{collect_entity_visit, fixture_resident_column};

/// Declares saved, unsaved, fire-immune, and feature-gated general entity types.
struct Declare;

#[mod_api::__subscriptions]
impl mod_api::Hooks for Declare {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        for (path, save, features) in [
            ("robot", true, &[][..]),
            ("blaze", true, &[][..]),
            ("ghost", false, &[][..]),
            (
                "experimental",
                true,
                &[mod_api::FeatureFlag::RedstoneExperiments][..],
            ),
        ] {
            r.declare_general_entity(
                path,
                mod_api::GeneralEntityCategory::Misc,
                0.8,
                1.9,
                Some([1.0, 1.25, 0.0]),
                if save {
                    mod_api::GeneralSavePolicy::Save
                } else {
                    mod_api::GeneralSavePolicy::NoSave
                },
                true,
                false,
                path == "blaze",
                save.then_some(mod_api::PushReaction::IgnoreEntity),
                8,
                3,
                1,
                features,
            )
            .unwrap();
        }
    }
}

/// A fixture game whose frozen registry holds the two declared types. The
/// registration phase is scoped around the whole fixture: the game harness
/// re-runs the installed mods' `register`, and a general type is only
/// declared while that phase is current. The runtime initializes first: a
/// scope entered before the platform installs its instrumentation writes a
/// fallback nobody reads, and the declaration then refuses as frozen.
fn with_types(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    with_types_and(None, test);
}

fn with_types_and(
    observer: Option<mod_api::ModEntry>,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    mod_platform::__private::initialize().expect("test registration requires the mod API runtime");
    let _phase = mod_platform::__private::RegistrationGuard::open();
    let mut mods = vec![mod_api::ModEntry::new("mymod", Box::new(Declare))];
    mods.extend(observer);
    mod_platform::testing::with_mods(mods.into(), || {
        with_game(|game| resident_origin(game, test))
    });
}

/// Run `test` with chunk (0, 0) loaded: a handle resolves only for an entity in an
/// accessible section (`LevelEntityGetter`), and these fixtures' entities stand in it.
fn resident_origin(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    fixture_resident_column(&mut game.world, (0, 0));
    test(game);
}

/// One record's save tag, the format `to_tag` writes.
fn record_tag(type_key: &str, uuid: [i32; 4], payload: &[u8], riders: Vec<Tag>) -> Tag {
    let mut tag = GeneralEntityRecord {
        position: [0.5, 64.0, 0.5],
        velocity: [0.0; 3],
        rotation: [0.0, 90.0],
        owner: None,
        custom_name: None,
        custom_name_visible: false,
        base: Default::default(),
        on_ground: false,
        fall_distance: 0.0,
        uuid,
        envelope: GeneralTypeEnvelope {
            type_key: type_key.to_owned(),
            payload_version: 3,
            payload_bytes: payload.to_vec(),
        },
        passengers: Vec::new(),
    }
    .to_tag();
    if let (Tag::Compound(map), false) = (&mut tag, riders.is_empty()) {
        map.insert(format::PASSENGERS.into(), Tag::List(riders));
    }
    tag
}

#[test]
#[ignore = "daily"]
fn bare_level_feature_enables_host_query_and_general_spawn() {
    use mod_api::{EntitySpawn, FeatureFlag, SpawnBody};
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::save::{Level, write_dat};

    with_types(|game| {
        game.world
            .chunks
            .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
        let level = game.level(game.dim.id()).unwrap();
        let at = [0.5, 64.0, 0.5];
        let spawn = EntitySpawn::new(SpawnBody::General);
        assert!(
            !level
                .enabled_features(game)
                .unwrap()
                .contains_flag(FeatureFlag::RedstoneExperiments)
        );
        assert!(
            level
                .spawn_entity(game, "mymod:experimental", at, &spawn)
                .is_none()
        );

        let dir = tempfile::tempdir().unwrap();
        write_dat(
            &dir.path().join("level.dat"),
            Tag::compound([(
                "Data",
                Tag::compound([
                    ("DataVersion", Tag::Int(sunrise_core::version::DATA_VERSION)),
                    (
                        "enabled_features",
                        Tag::List(vec![Tag::String("redstone_experiments".into())]),
                    ),
                ]),
            )]),
        )
        .unwrap();
        game.level.enabled_features = Level::read(dir.path()).unwrap().enabled_features;

        let flags = level.enabled_features(game).unwrap();
        assert!(flags.contains_flag(FeatureFlag::RedstoneExperiments));
        assert!(
            !flags.contains_flag(FeatureFlag::Vanilla),
            "a present list is not the default set"
        );
        assert_eq!(
            flags.iter().collect::<Vec<_>>(),
            ["minecraft:redstone_experiments"]
        );
        let entity = level
            .spawn_entity(game, "mymod:experimental", at, &spawn)
            .unwrap();
        assert_eq!(entity.position(game), Some(at));
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn bare_pack_feature_enables_host_query_and_general_spawn_on_first_open() {
    use mod_api::{EntitySpawn, FeatureFlag, SpawnBody};
    use sunrise_client::{chunk::Chunk, title::WorldOptions};

    let pack_root = tempfile::tempdir().unwrap();
    let pack = pack_root.path().join("redstone_bare");
    std::fs::create_dir_all(&pack).unwrap();
    let format = sunrise_core::version::DATA_PACK_FORMAT;
    std::fs::write(
        pack.join("pack.mcmeta"),
        serde_json::json!({
            "pack": {
                "description": "Bare redstone feature pack",
                "pack_format": format.0,
                "min_format": [format.0, format.1],
                "max_format": [format.0, format.1],
            },
            "features": {
                "enabled": ["redstone_experiments"],
            },
        })
        .to_string(),
    )
    .unwrap();
    let save_root = tempfile::tempdir().unwrap();

    with_types(|game| {
        let create = WorldOptions {
            pack_dir: Some(pack_root.path().to_path_buf()),
            data_packs: vec!["vanilla".into(), "file/redstone_bare".into()],
            ..WorldOptions::default()
        };
        let mut request = sunrise_gameplay::game::world_loading::WorldRequest::new(
            "bare_pack_feature",
            Some("bare_pack_feature"),
            Some(1),
            true,
            Some(&create),
            game.level.rule_registry.clone(),
        )
        .unwrap();
        request.save_root = save_root.path().to_owned();
        game.load_world(request).unwrap();
        game.world
            .chunks
            .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));

        let level = game.level(game.dim.id()).unwrap();
        let flags = level.enabled_features(game).unwrap();
        assert!(flags.contains_flag(FeatureFlag::RedstoneExperiments));
        assert!(flags.contains_flag(FeatureFlag::Vanilla));
        assert_eq!(
            flags.iter().collect::<Vec<_>>(),
            ["minecraft:redstone_experiments", "minecraft:vanilla"]
        );

        let at = [0.5, 64.0, 0.5];
        let entity = level
            .spawn_entity(
                game,
                "mymod:experimental",
                at,
                &EntitySpawn::new(SpawnBody::General),
            )
            .unwrap();
        assert_eq!(entity.position(game), Some(at));
    });
}

#[test]
#[ignore = "daily"]
fn feature_disabled_saved_branches_do_not_enter_summon_uuid_admission() {
    use sunrise_world::snbt_text::to_snbt;

    with_types(|game| {
        let collision = [1, 2, 3, 4];
        game.spawn_loaded_entity(
            (0, 0),
            record_tag("mymod:robot", collision, b"", vec![]),
            None,
        );
        assert!(game.uuid_admission_refused(Some(collision)));
        let child = record_tag("mymod:robot", collision, b"", vec![]);
        let disabled = |uuid| record_tag("mymod:experimental", uuid, b"", vec![child.clone()]);

        // A disabled root excludes its entire branch on a disk load too.
        game.spawn_loaded_entity((0, 0), disabled([8, 0, 0, 1]), None);
        assert_eq!(game.resident_entities.general.rows.len(), 1);

        for (index, kind) in ["mymod:robot", "minecraft:minecart"]
            .into_iter()
            .enumerate()
        {
            let root = record_tag(
                kind,
                [9, 0, 0, index as i32],
                b"",
                vec![disabled(collision), disabled([8, 0, 0, 2])],
            );
            let mount = game
                .summon_with_nbt(kind, [0.5, 64.0, 0.5], &to_snbt(&root))
                .unwrap();
            let id = match mount {
                Some(Target::General(id) | Target::Cart(id)) => id,
                other => panic!("expected the eligible mount, got {other:?}"),
            };
            assert!(game.vehicle_passenger_ids(id).is_empty());
        }
        assert_eq!(
            game.resident_entities.general.rows.len(),
            2,
            "only the live control and general mount remain"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_piston_ignored_general_row_moves_for_a_shulker_lid() {
    use sunrise_core::{
        geometry::{Aabb, Direction},
        position::BlockPos,
    };
    use sunrise_gameplay_common::game::piston_push::Lid;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::{entities::ArmorStand, mobs::Mob};
    use sunrise_world::level::pistons::MovingPiston;

    with_types(|game| {
        game.world
            .chunks
            .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
        let root = record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new());
        game.load_listed("mymod:robot", None, None, root).unwrap();
        let row = &game.resident_entities.general.rows[0];
        let (id, before) = (row.id, row.position);
        let stand = id + 1;
        let shulker = id + 2;
        game.armor_stands.push(ArmorStand::new(stand, before, 0.0));
        let kind = game
            .res
            .mob_types()
            .get("minecraft:shulker")
            .unwrap()
            .clone();
        game.mobs
            .push(Mob::new(shulker, kind, Vec3::new(8.0, 64.0, 8.0), 0.0));
        assert_eq!(
            game.entity(id).unwrap().piston_push_reaction(game),
            Ok(Some(mod_api::PushReaction::IgnoreEntity))
        );

        game.push_piston_entities(vec![(
            BlockPos::new(0, 64, 0),
            MovingPiston {
                moved_state: game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap(),
                direction: Direction::East,
                extending: true,
                source: false,
                progress: 0.0,
                progress_o: 0.0,
                last_ticked: 0,
                death_ticks: 0,
                id: 1,
            },
            0.5,
        )]);
        assert_eq!(game.resident_entities.general.rows[0].position, before);
        assert!(
            game.armor_stands[0].position.x > before.x,
            "the piston reaches the control"
        );

        let sweep = Aabb::new(Vec3::new(0.0, 64.0, 0.0), Vec3::new(1.0, 65.0, 1.0));
        let movement = Vec3::new(0.0, 0.25, 0.0);
        game.lid_push(Lid::Box, sweep, movement);
        assert_eq!(game.resident_entities.general.rows[0].position, before);
        game.lid_push(Lid::Shulker(shulker), sweep, movement);
        assert_eq!(
            game.resident_entities.general.rows[0].position,
            before.add_vector(movement)
        );
    });
}

/// The saved rows of one chunk, through the same walk the save uses.
fn saved_chunk(game: &mut sunrise_gameplay::game::ServerGame<'_>, key: ChunkKey) -> Vec<Tag> {
    let mut out: HashMap<ChunkKey, Vec<(u32, sunrise_sim::join_seq::JoinSeq, Tag)>> =
        HashMap::new();
    let mut keys = HashSet::new();
    keys.insert(key);
    let mut failed = HashMap::new();
    game.save_every_entity(&keys, &mut out, &mut failed, false);
    assert!(failed.is_empty());
    out.remove(&key)
        .unwrap_or_default()
        .into_iter()
        .map(|(_, _, tag)| tag)
        .collect()
}

/// A loaded tree publishes its rows through the same join boundary a fresh
/// spawn takes: the root and each rider are rows of one list, the riders
/// mounted, and every identity remembered. Saving the chunk writes one
/// record per root — the riders inside it — with the payload bytes it
/// loaded.
#[test]
#[ignore = "daily"]
fn general_tree_loads_joins_and_saves_one_record() {
    with_types(|game| {
        let rider = record_tag("mymod:robot", [9, 9, 9, 9], b"rider", Vec::new());
        let root = record_tag("mymod:robot", [1, 2, 3, 4], b"payload", vec![rider]);
        game.load_listed("mymod:robot", None, None, root).unwrap();
        let rows = &game.resident_entities.general.rows;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].uuid, [1, 2, 3, 4]);
        assert_eq!(rows[1].mount, Some(rows[0].id));
        assert_eq!(
            game.target_position(
                sunrise_gameplay_common::game::commands_common::Target::General(rows[0].id)
            ),
            Some([0.5, 64.0, 0.5])
        );
        assert_eq!(rows[0].payload_bytes, b"payload");
        assert_eq!(rows[0].payload_version, 3);
        assert!(rows[0].join.is_joined() && rows[1].join.is_joined());
        assert_eq!(game.persist.uuids.get(&rows[0].id), Some(&[1, 2, 3, 4]));
        assert_eq!(game.persist.uuids.get(&rows[1].id), Some(&[9, 9, 9, 9]));
        // One record per root, the rider inside it, the payload as loaded.
        let saved = saved_chunk(game, (0, 0));
        assert_eq!(saved.len(), 1);
        let Tag::List(riders) = saved[0].get(format::PASSENGERS).unwrap() else {
            panic!("the rider saves inside its root");
        };
        assert_eq!(riders.len(), 1);
        assert_eq!(
            saved[0].get(format::PAYLOAD),
            Some(&Tag::ByteArray(b"payload".to_vec()))
        );
        assert_eq!(
            saved[0].get(format::UUID),
            Some(&Tag::IntArray(vec![1, 2, 3, 4]))
        );
    });
}

#[test]
#[ignore = "daily"]
fn minecart_keeps_its_general_passenger_after_the_rider_tick() {
    with_types(|game| {
        fixture_resident_column(&mut game.world, (0, 0));
        let cart_uuid = [1, 2, 3, 4];
        let rider_uuid = [2, 3, 4, 5];
        let cart = Tag::Compound(
            [
                ("id".into(), Tag::String("minecraft:minecart".into())),
                ("UUID".into(), Tag::IntArray(cart_uuid.to_vec())),
                (
                    "Pos".into(),
                    Tag::List(
                        [0.5_f64, 64.0, 0.5]
                            .map(|n| Tag::Double(n.to_bits()))
                            .to_vec(),
                    ),
                ),
            ]
            .into(),
        );
        game.load_listed("minecraft:minecart", Some(cart_uuid), None, cart)
            .unwrap();
        game.load_listed(
            "mymod:robot",
            None,
            None,
            record_tag("mymod:robot", rider_uuid, b"rider", Vec::new()),
        )
        .unwrap();
        let cart_id = game.carts[0].id;
        let rider_id = game.resident_entities.general.rows[0].id;
        let cart = game.entity(cart_id).unwrap();
        let rider = game.entity(rider_id).unwrap();
        assert_eq!(game.entity_start_riding(rider, cart, false), Ok(()));
        assert_eq!(
            collect_entity_visit(|visit| game.entity_visit_passengers(cart, visit)),
            Ok(vec![rider])
        );

        // Keep the chunk loaded but outside simulation range so the pass refreshes
        // the adjacency without rider movement invalidating it again.
        use sunrise_sim::ticket_levels::{Sources, ViewedTicket};
        game.server.tickets.0.update(Some(Sources {
            viewed: vec![ViewedTicket {
                center: (0, 0),
                radius: 0,
                ticking: false,
            }],
            ..Default::default()
        }));
        game.begin_ordered_ticks();
        game.tick_general().unwrap();
        assert_eq!(
            collect_entity_visit(|visit| game.entity_visit_passengers(cart, visit)),
            Ok(vec![rider])
        );
        assert_eq!(game.entity_is_vehicle(cart), Some(true));
        assert_eq!(game.boarding_order(cart_id, u32::MAX, false), 1);
        let saved = saved_chunk(game, (0, 0));
        assert_eq!(saved.len(), 1);
        let Some(Tag::List(passengers)) = saved[0].get("Passengers") else {
            panic!("minecart saved without its passenger");
        };
        assert_eq!(passengers.len(), 1);
        assert_eq!(
            sunrise_sim::entity_nbt::uuid(&passengers[0]),
            Some(rider_uuid)
        );
    });
}

#[test]
#[ignore = "daily"]
fn mixed_passengers_sit_and_unload_with_their_actual_root() {
    use mod_api::StartRidingRefusal;
    use sunrise_gameplay_common::game::commands_common::Target;
    with_types(|game| {
        let root_uuid = [1, 2, 3, 4];
        let pig_uuid = [2, 3, 4, 5];
        let leaf_uuid = [3, 4, 5, 6];
        let mut root = record_tag("mymod:robot", root_uuid, b"root", Vec::new());
        if let Tag::Compound(map) = &mut root {
            map.insert(
                format::POSITION.into(),
                Tag::List(
                    [2.5_f64, 64.0, 0.5]
                        .map(|part| Tag::Double(part.to_bits()))
                        .to_vec(),
                ),
            );
            map.insert(
                format::ROTATION.into(),
                Tag::List(vec![Tag::Float(90_f32.to_bits()), Tag::Float(0)]),
            );
        }
        game.load_listed("mymod:robot", None, None, root).unwrap();
        game.load_listed(
            "mymod:robot",
            None,
            None,
            record_tag("mymod:robot", leaf_uuid, b"leaf", Vec::new()),
        )
        .unwrap();
        let root = game.resident_entities.general.rows[0].id;
        let leaf = game.resident_entities.general.rows[1].id;
        let pig = game
            .load_mob(
                Tag::Compound(
                    [
                        ("id".into(), Tag::String("minecraft:pig".into())),
                        ("UUID".into(), Tag::IntArray(pig_uuid.to_vec())),
                        (
                            "Pos".into(),
                            Tag::List(
                                [0.5_f64, 64.0, 0.5]
                                    .map(|part| Tag::Double(part.to_bits()))
                                    .to_vec(),
                            ),
                        ),
                    ]
                    .into(),
                ),
                None,
            )
            .unwrap();
        let [root_ref, pig_ref, leaf_ref] = [root, pig, leaf].map(|id| game.entity(id).unwrap());
        assert_eq!(game.entity_start_riding(pig_ref, root_ref, false), Ok(()));
        assert_eq!(game.entity_start_riding(leaf_ref, pig_ref, false), Ok(()));
        assert_eq!(
            game.entity_start_riding(root_ref, leaf_ref, true),
            Err(StartRidingRefusal::Cycle)
        );
        game.server.tickets.0.update(None);
        game.begin_ordered_ticks();
        // The mobs' pass first, as tick_mobs runs it: it names the general vehicles mobs ride.
        game.position_riders();
        game.tick_general().unwrap();
        assert_eq!(pig_ref.position(game), Some([2.5, 65.25, 1.5]));
        assert_eq!(
            leaf_ref.position(game),
            Some([2.5, 65.25 + f64::from(0.86875_f32), 1.5])
        );
        assert_eq!(leaf_ref.root_vehicle(game), Some(root_ref));

        // The leaf's and the intermediate mob's chunks do not own the tree.
        game.mob_object_mut(pig).unwrap().position.x = 18.5;
        game.resident_entities.general.rows[1].position.x = 33.5;
        // Loaded, so the moved riders stay resolvable while their own chunks are asked.
        fixture_resident_column(&mut game.world, (1, 0));
        fixture_resident_column(&mut game.world, (2, 0));
        let own = game.entity_chunks(&HashSet::from([(1, 0), (2, 0)]), true);
        assert!(own.iter().all(|(_, tags)| tags.is_empty()));
        assert!(game.entity(pig).is_some() && game.entity(leaf).is_some());
        let mut saved = game.entity_chunks(&HashSet::from([(0, 0)]), true);
        assert_eq!(saved[0].1.len(), 1);
        let saved = saved[0].1.pop().unwrap();
        let Some(Tag::List(pigs)) = saved.get("Passengers") else {
            panic!("missing nested pig")
        };
        assert_eq!(pigs.len(), 1);
        assert_eq!(sunrise_sim::entity_nbt::uuid(&pigs[0]), Some(pig_uuid));
        let Some(Tag::List(leaves)) = pigs[0].get("Passengers") else {
            panic!("missing nested general row")
        };
        assert_eq!(leaves.len(), 1);
        assert_eq!(
            leaves[0].get(format::PAYLOAD),
            Some(&Tag::ByteArray(b"leaf".to_vec()))
        );
        for id in [root, pig, leaf] {
            assert!(game.entity(id).is_none());
        }

        game.load_listed("mymod:robot", None, None, saved).unwrap();
        let [root, pig, leaf] =
            [root_uuid, pig_uuid, leaf_uuid].map(|uuid| game.vehicle_by_uuid(uuid).unwrap());
        assert_eq!(game.native_vehicle_id(Target::General(leaf)), Some(pig));
        assert_eq!(game.native_vehicle_id(Target::Mob(pig)), Some(root));
    });
}

/// `noSave`: instances load and live like any other, but produce zero
/// persisted records.
#[test]
#[ignore = "daily"]
fn no_save_type_writes_zero_records() {
    with_types(|game| {
        game.load_listed(
            "mymod:ghost",
            None,
            None,
            record_tag("mymod:ghost", [5, 6, 7, 8], b"", Vec::new()),
        )
        .unwrap();
        assert_eq!(game.resident_entities.general.rows.len(), 1);
        assert!(saved_chunk(game, (0, 0)).is_empty());
    });
}

/// A UUID a live entity of the level already owns refuses the whole tree:
/// zero rows publish, rider included, and the refusing record names it.
#[test]
#[ignore = "daily"]
fn duplicate_uuid_with_a_live_entity_refuses_the_whole_tree() {
    with_types(|game| {
        game.load_listed(
            "mymod:robot",
            None,
            None,
            record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new()),
        )
        .unwrap();
        assert_eq!(game.resident_entities.general.rows.len(), 1);
        // A different root whose rider carries the live row's UUID.
        let rider = record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new());
        let root = record_tag("mymod:robot", [8, 8, 8, 8], b"", vec![rider]);
        game.load_listed("mymod:robot", None, None, root).unwrap();
        assert_eq!(
            game.resident_entities.general.rows.len(),
            1,
            "the whole tree refuses, not only the duplicate rider"
        );
        assert_eq!(game.resident_entities.general.rows[0].uuid, [1, 2, 3, 4]);
    });
}

/// A codec refusal (an envelope field of the wrong shape) publishes zero
/// rows and leaves nothing behind: the next valid record loads cleanly.
#[test]
#[ignore = "daily"]
fn codec_refusal_publishes_nothing_and_poisons_nothing() {
    with_types(|game| {
        let mut broken = record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new());
        if let Tag::Compound(map) = &mut broken {
            map.insert(format::PAYLOAD.into(), Tag::Int(1));
        }
        game.load_listed("mymod:robot", None, None, broken).unwrap();
        assert!(game.resident_entities.general.rows.is_empty());
        // The next valid record through the same entry point stands.
        game.load_listed(
            "mymod:robot",
            None,
            None,
            record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new()),
        )
        .unwrap();
        assert_eq!(game.resident_entities.general.rows.len(), 1);
    });
}

/// A structural refusal (a NaN coordinate) publishes zero rows of the tree.
#[test]
#[ignore = "daily"]
fn structural_refusal_publishes_nothing() {
    with_types(|game| {
        let mut nan = record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new());
        if let Tag::Compound(map) = &mut nan {
            map.insert(
                format::POSITION.into(),
                Tag::List(vec![
                    Tag::Double(f64::NAN.to_bits()),
                    Tag::Double(64.0f64.to_bits()),
                    Tag::Double(0.5f64.to_bits()),
                ]),
            );
        }
        game.load_listed("mymod:robot", None, None, nan).unwrap();
        assert!(game.resident_entities.general.rows.is_empty());
    });
}

/// A structurally sound tree with a type this load cannot resolve hands the
/// compound back (`Err`), keeping it for the missing mod's path instead of
/// publishing or dropping it.
#[test]
#[ignore = "daily"]
fn unknown_rider_type_hands_the_tree_back() {
    with_types(|game| {
        let rider = record_tag("gone:drone", [9, 9, 9, 9], b"", Vec::new());
        let root = record_tag("mymod:robot", [1, 2, 3, 4], b"", vec![rider]);
        let handed = root.clone();
        assert_eq!(
            game.load_listed("mymod:robot", None, None, root),
            Err(handed.clone())
        );
        assert!(game.general_quarantine_admits(&handed, "mymod:robot", Vec3::new(0.0, 0.0, 0.0)));
        assert!(game.resident_entities.general.rows.is_empty());
    });
}

/// The unload path: `unload_entities` (game.rs) runs `entity_chunks` with
/// `remove`, whose sweep (persist/unload_sweep.rs) first saves every row
/// through `save_lists!` and then removes them and forgets their ids. A
/// root and its rider leave together as one record; the resident table no
/// longer knows the old id (what an `EntityRef` resolves through), and the
/// record that comes back loads as exactly one tree of the same UUIDs,
/// under fresh ids from the never-decreasing counter.
#[test]
#[ignore = "daily"]
fn general_row_unloads_with_its_chunk_and_reloads_once() {
    use mod_api::DimensionKey;
    use std::num::NonZeroU32;
    use sunrise_gameplay::game::entity_residency::SlotAccess;
    with_types(|game| {
        game.reset_entity_residency();
        let grandchild = record_tag("mymod:robot", [8, 8, 8, 8], b"grandchild", Vec::new());
        let rider = record_tag("mymod:robot", [9, 9, 9, 9], b"rider", vec![grandchild]);
        let root = record_tag("mymod:robot", [1, 2, 3, 4], b"root", vec![rider]);
        game.load_listed("mymod:robot", None, None, root).unwrap();
        let rows = &game.resident_entities.general.rows;
        let (old_root, old_rider) = (rows[0].id, rows[1].id);
        let id = |value: u32| NonZeroU32::new(value).unwrap();
        for known in [old_root, old_rider] {
            assert!(matches!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(known), |_| true),
                SlotAccess::Done(true)
            ));
        }
        // The chunk unloads: one record leaves (the rider inside its root),
        // and both rows are gone.
        let mut chunks = game.entity_chunks(&HashSet::from([(0, 0)]), true);
        assert_eq!(chunks.len(), 1);
        let (_, mut records) = chunks.remove(0);
        assert_eq!(records.len(), 1);
        assert!(game.resident_entities.general.rows.is_empty());
        // The old ids no longer resolve: a stale handle cannot reach either
        // row again, and the counter never hands their ids back out.
        for gone in [old_root, old_rider] {
            assert!(matches!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(gone), |_| panic!(
                    "a stale id resolved"
                )),
                SlotAccess::Missing
            ));
        }
        // The same record loads back as one tree of the same UUIDs.
        game.load_listed("mymod:robot", None, None, records.remove(0))
            .unwrap();
        let rows = &game.resident_entities.general.rows;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].uuid, [8, 8, 8, 8]);
        assert_eq!(rows[2].mount, Some(rows[1].id));
        assert_eq!(rows[0].uuid, [1, 2, 3, 4]);
        assert_eq!(rows[1].uuid, [9, 9, 9, 9]);
        assert_eq!(rows[1].mount, Some(rows[0].id));
        assert_ne!(rows[0].id, old_root);
        assert_ne!(rows[1].id, old_rider);
    });
}

/// The missing-mod quarantine: a record of a namespace no loaded mod owns
/// keeps the existing unknown-kind path — held as loaded (`persist.raw`),
/// never ticked, written back from that store — byte for byte.
#[test]
#[ignore = "daily"]
fn quarantine_preserves_a_missing_mods_record_byte_for_byte() {
    with_types(|game| {
        game.server.others.0.clear();
        let mut tag = record_tag("gone:robot", [7, 7, 7, 7], b"opaque", Vec::new());
        if let Tag::Compound(fields) = &mut tag {
            // Vanilla fallback loading strips these fields; an opaque mod record must not.
            fields.insert("Team".into(), Tag::String("missing-mod-team".into()));
            fields.insert("leash".into(), Tag::Compound(Default::default()));
        }
        let loaded = tag.clone();
        game.spawn_loaded_entity((0, 0), tag, None);
        assert_eq!(game.server.others.0.len(), 1);
        assert_eq!(game.server.others.0[0].kind, "gone:robot");
        // What leaves the chunk is the very compound that arrived.
        let mut chunks = game.entity_chunks(&HashSet::from([(0, 0)]), false);
        let (_, mut records) = chunks.remove(0);
        assert_eq!(records.len(), 1);
        assert_eq!(records.remove(0), loaded);
    });
}

/// A loaded mod's namespace with an unknown type skips with a warning:
/// nothing is published, quarantined or kept for it.
#[test]
#[ignore = "daily"]
fn loaded_mods_unknown_type_skips() {
    // The package inventory is what owns a namespace: "mymod" is a loaded
    // package here, while no type of it was ever declared.
    mod_platform::__private::with_test_packages(&["mymod"], || {
        // No fixture declaration: the type is unknown precisely because no
        // loaded mod declared it.
        with_game(|game| {
            game.server.others.0.clear();
            game.spawn_loaded_entity(
                (0, 0),
                record_tag("mymod:mystery", [7, 7, 7, 7], b"", Vec::new()),
                None,
            );
            assert!(game.server.others.0.is_empty());
            let chunks = game.entity_chunks(&HashSet::from([(0, 0)]), false);
            assert!(chunks.iter().all(|(_, records)| records.is_empty()));
        });
    });
}

/// An unknown `minecraft:` kind keeps its existing path unchanged —
/// preserved whatever its size, outside the quarantine bounds.
#[test]
#[ignore = "daily"]
fn unknown_minecraft_kind_keeps_its_existing_path() {
    with_types(|game| {
        game.server.others.0.clear();
        let mut oversized = record_tag("minecraft:mystery", [7, 7, 7, 7], b"", Vec::new());
        if let Tag::Compound(map) = &mut oversized {
            map.insert(
                "Extra".into(),
                Tag::ByteArray(vec![0; limits::QUARANTINE_RECORD_LIMIT as usize + 1]),
            );
        }
        game.spawn_loaded_entity((0, 0), oversized, None);
        assert_eq!(game.server.others.0.len(), 1);
        assert_eq!(game.server.others.0[0].kind, "minecraft:mystery");
    });
}

/// The quarantine record bound: a missing-mod record one byte over
/// [`limits::QUARANTINE_RECORD_LIMIT`] skips; one inside it is kept.
#[test]
#[ignore = "daily"]
fn quarantine_record_limit_skips_one_byte_over() {
    with_types(|game| {
        game.server.others.0.clear();
        let sized = |extra: u64| {
            let mut tag = record_tag("gone:robot", [7, 7, 7, 7], b"", Vec::new());
            if let Tag::Compound(map) = &mut tag {
                map.insert("Extra".into(), Tag::ByteArray(vec![0; extra as usize]));
            }
            tag
        };
        // Size the record to exactly the bound, then one byte past it.
        let base = nbt_size(&sized(0));
        let fill = limits::QUARANTINE_RECORD_LIMIT - base;
        assert!(fill > 1, "the base record must sit below the bound");
        game.spawn_loaded_entity((0, 0), sized(fill), None);
        assert_eq!(
            game.server.others.0.len(),
            1,
            "a record at the bound is kept"
        );
        game.spawn_loaded_entity((0, 0), sized(fill + 1), None);
        assert_eq!(game.server.others.0.len(), 1, "one byte over skips");
    });
}

/// The quarantine chunk bound: the
/// [`limits::QUARANTINE_CHUNK_LIMIT`]'th record of one chunk is kept, the
/// next skips.
#[test]
#[ignore = "daily"]
fn quarantine_chunk_limit_skips_past_256() {
    with_types(|game| {
        game.server.others.0.clear();
        let at = |slot: usize| [slot as i32, 0, 0, 0];
        let tag = |slot: usize| record_tag("gone:robot", at(slot), b"", Vec::new());
        // Fill the chunk to one below the bound through the gate itself.
        for slot in 0..limits::QUARANTINE_CHUNK_LIMIT - 1 {
            game.spawn_loaded_entity((0, 0), tag(slot), None);
        }
        assert_eq!(
            game.server.others.0.len(),
            limits::QUARANTINE_CHUNK_LIMIT - 1
        );
        game.spawn_loaded_entity((0, 0), tag(250), None);
        assert_eq!(
            game.server.others.0.len(),
            limits::QUARANTINE_CHUNK_LIMIT,
            "the bound's own record is kept"
        );
        game.spawn_loaded_entity((0, 0), tag(251), None);
        assert_eq!(
            game.server.others.0.len(),
            limits::QUARANTINE_CHUNK_LIMIT,
            "the next record skips"
        );
        // A record of another chunk still quarantines.
        let mut elsewhere = tag(252);
        if let Tag::Compound(map) = &mut elsewhere {
            map.insert(
                format::POSITION.into(),
                Tag::List(
                    [1024.5f64, 64.0, 0.5]
                        .map(|v| Tag::Double(v.to_bits()))
                        .to_vec(),
                ),
            );
        }
        game.spawn_loaded_entity((64, 0), elsewhere, None);
        assert_eq!(
            game.server.others.0.len(),
            limits::QUARANTINE_CHUNK_LIMIT + 1
        );
    });
}

/// The transfer load path (`load_listed_transferred`) accepts a general
/// tree: what arrives with a transfer token is installed as the same
/// logical entity — its runtime state (attachments map) carried over to
/// the new id — instead of the old "refusing unsupported transferred
/// entity kind".
#[test]
#[ignore = "daily"]
fn transferred_general_tree_loads_as_the_same_logical_entity() {
    with_types(|game| {
        game.load_listed(
            "mymod:robot",
            None,
            None,
            record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new()),
        )
        .unwrap();
        let row = game.resident_entities.general.rows.remove(0);
        let state = game.take_transferred_entity_state(row.id);
        game.detach_entity(row.id);
        // The transferred arrival: a fresh id, the same UUID, its runtime
        // state (the attachments map, even empty) adopted, not refused.
        game.spawn_loaded_entity(
            (0, 0),
            record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new()),
            Some(state),
        );
        let rows = &game.resident_entities.general.rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].uuid, [1, 2, 3, 4]);
        assert_ne!(rows[0].id, row.id);
        assert!(game.entity_attachments.contains_key(&rows[0].id));
    });
}

#[test]
#[ignore = "daily"]
fn a_same_level_teleport_places_the_general_tree_and_unseats_a_separately_moved_rider() {
    with_types(|game| {
        let rider = record_tag("mymod:robot", [9, 9, 9, 9], b"", Vec::new());
        let root = record_tag("mymod:robot", [1, 2, 3, 4], b"", vec![rider]);
        game.load_listed("mymod:robot", None, None, root).unwrap();
        let rows = &game.resident_entities.general.rows;
        let (root, rider) = (
            game.entity(rows[0].id).unwrap(),
            game.entity(rows[1].id).unwrap(),
        );

        let to = [10.5, 64.0, 0.5];
        assert!(root.teleport_to(game, to, Some([90.0, 30.0])));
        assert_eq!(root.position(game), Some(to));
        assert_eq!(rider.position(game), Some(to));
        assert_eq!(rider.rotation(game), Some([90.0, 30.0]));

        let rider_to = [12.5, 64.0, 0.5];
        assert!(rider.teleport_to(game, rider_to, None));
        assert_eq!(rider.position(game), Some(rider_to));
        assert_eq!(rider.vehicle(game), None);
        assert_eq!(root.position(game), Some(to));
    });
}

/// A chain of general rows, each riding the one before, by direct row writes.
fn general_chain(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    poses: &[([f64; 3], [f32; 2])],
    word: i32,
) -> Vec<u32> {
    let mut ids = Vec::new();
    for (n, _) in poses.iter().enumerate() {
        let tag = record_tag("mymod:robot", [word, n as i32, 0, 0], b"", Vec::new());
        let start = game.resident_entities.general.rows.len();
        game.load_listed("mymod:robot", None, None, tag).unwrap();
        ids.push(game.resident_entities.general.rows[start].id);
    }
    for (n, (at, rotation)) in poses.iter().enumerate() {
        let row = game.resident_entities.general.row_mut(ids[n]).unwrap();
        row.position = Vec3::new(at[0], at[1], at[2]);
        row.previous = row.position;
        row.rotation = *rotation;
        row.mount = n.checked_sub(1).map(|up| ids[up]);
        row.vehicle_order = 0;
    }
    game.resident_entities.general.mount_writes += 1;
    ids
}

#[test]
#[ignore = "daily"]
fn a_general_riders_motion_follows_the_commands_delta_flags() {
    with_types(|game| {
        let ids = general_chain(
            game,
            &[
                ([0.0, 64.0, 0.0], [0.0, 0.0]),
                ([0.0, 65.0, 0.0], [0.0, 0.0]),
            ],
            3,
        );
        let velocity = Vec3::new(0.25, -0.5, 0.75);
        fn rider<'a>(
            game: &'a mut sunrise_gameplay::game::ServerGame<'_>,
            id: u32,
        ) -> &'a mut GeneralRow {
            game.resident_entities.general.row_mut(id).unwrap()
        }
        rider(game, ids[1]).velocity = velocity;
        // An absolute /tp is a transition with no motion and no delta flags.
        game.teleport_to_in_level(
            Target::General(ids[0]),
            [5.0, 64.0, 5.0],
            None,
            [false; 3],
            true,
        );
        assert_eq!(rider(game, ids[1]).velocity, Vec3::ZERO);
        // `tp ~ 70 ~`: only the axes the command made relative keep their motion.
        rider(game, ids[1]).velocity = velocity;
        game.teleport_to_in_level(
            Target::General(ids[0]),
            [5.0, 70.0, 5.0],
            None,
            [true, false, true],
            true,
        );
        assert_eq!(rider(game, ids[1]).velocity, Vec3::new(0.25, 0.0, 0.75));
    });
}

#[test]
#[ignore = "daily"]
fn a_nested_general_tree_takes_each_riders_transition_from_its_vehicle() {
    with_types(|game| {
        let ids = general_chain(
            game,
            &[
                ([26.007, 8.53, 61.898], [-103.5, -10.6]),
                ([75.6, 69.43, 30.548], [146.4, 1.8]),
                ([29.73, 5.392, 17.554], [166.7, 13.0]),
            ],
            4,
        );
        game.teleport_to_in_level(
            Target::General(ids[0]),
            [25.442, 20.88, 4.214],
            Some([82.4, -71.8]),
            [false; 3],
            true,
        );
        let row = |game: &sunrise_gameplay::game::ServerGame<'_>, n: usize| {
            game.resident_entities.general.row(ids[n]).unwrap().clone()
        };
        // Entity.calculatePassengerTransition: each edge adds the child's offset
        // from its immediate vehicle's old pose to that vehicle's transition.
        let mid_at = [
            25.442 + (75.6 - 26.007),
            20.88 + (69.43 - 8.53),
            4.214 + (30.548 - 61.898),
        ];
        let leaf_at = [
            mid_at[0] + (29.73 - 75.6),
            mid_at[1] + (5.392 - 69.43),
            mid_at[2] + (17.554 - 30.548),
        ];
        let mid = row(game, 1);
        assert_eq!([mid.position.x, mid.position.y, mid.position.z], mid_at);
        let leaf = row(game, 2);
        assert_eq!([leaf.position.x, leaf.position.y, leaf.position.z], leaf_at);
        let mid_turn = [
            82.4f32 + (146.4f32 - -103.5f32),
            -71.8f32 + (1.8f32 - -10.6f32),
        ];
        assert_eq!(mid.rotation, mid_turn);
        assert_eq!(
            leaf.rotation,
            [
                mid_turn[0] + (166.7f32 - 146.4f32),
                mid_turn[1] + (13.0f32 - 1.8f32)
            ]
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_guest_riding_a_teleported_general_vehicle_is_carried_with_its_turn() {
    with_types(|game| {
        sunrise_gameplay::game::server_side::bossbars::audience_tests::add_real_and_fake(game);
        let ids = general_chain(game, &[([0.0, 64.0, 0.0], [0.0, 0.0])], 6);
        let host = game.server.lan.as_mut().unwrap();
        let i = host.hub.guests.iter().position(|g| g.uuid == 1).unwrap();
        host.hub.guests.update(i, |g| {
            g.vehicle = Some(ids[0]);
            g.position = [0.5, 64.5, 0.25];
            g.yaw = 30.0;
        });
        game.teleport_to_in_level(
            Target::General(ids[0]),
            [10.0, 70.0, -4.0],
            Some([90.0, 0.0]),
            [false; 3],
            true,
        );
        let guest = &game.server.lan.as_ref().unwrap().hub.guests[i];
        assert_eq!(guest.position, [10.5, 70.5, -3.75]);
        assert_eq!(guest.yaw, 120.0);
        assert_eq!(guest.vehicle, Some(ids[0]));
    });
}

/// `with_types` for a fixture that also hosts guests 12 and 13.
fn with_types_and_guests(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    mod_platform::__private::initialize().expect("test registration requires the mod API runtime");
    let _phase = mod_platform::__private::RegistrationGuard::open();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("mymod", Box::new(Declare))].into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            resident_origin(game, test)
        })
    });
}

/// A guest 13 seated on a general root turned to 30 degrees, its packets read through the receiver.
fn seat_guest_on_general_root(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
) -> (u32, std::sync::mpsc::Receiver<sunrise_sim::lan::Out>) {
    let ids = general_chain(game, &[([0.0, 64.0, 0.0], [0.0, 0.0])], 7);
    let (tx, rx) = std::sync::mpsc::channel();
    let hub = &mut game.server.lan.as_mut().unwrap().hub;
    hub.update_guest(13, |g| {
        g.vehicle = Some(ids[0]);
        g.yaw = 30.0;
        g.sender = sunrise_sim::lan::PlayerSender::new(tx, Default::default());
    })
    .unwrap();
    game.guest_boarded(13, None);
    (ids[0], rx)
}

/// The packets of one guest's teleport: its own position packets and the vehicle's teleport packets.
fn guest_teleport_packets(
    rx: &std::sync::mpsc::Receiver<sunrise_sim::lan::Out>,
) -> (
    Vec<sunrise_protocol::packets::play::PlayerPosition>,
    Vec<sunrise_protocol::packets::play::TeleportEntity>,
) {
    use sunrise_protocol::packets::play::Clientbound as C;
    let (mut own, mut vehicle) = (Vec::new(), Vec::new());
    for packet in sunrise_gameplay_common::game::server_side::native_sound_refs::tests::packets(rx)
    {
        match packet {
            C::PlayerPosition(p) => own.push(p),
            C::TeleportEntity(p) => vehicle.push(p),
            _ => {}
        }
    }
    (own, vehicle)
}

#[test]
#[ignore = "daily"]
fn a_seated_guest_hears_the_general_vehicles_real_pose_and_its_own_teleport_once() {
    with_types_and_guests(|game| {
        let (root, rx) = seat_guest_on_general_root(game);
        let _ = guest_teleport_packets(&rx);
        let before = game
            .server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guest(13)
            .unwrap()
            .position;
        game.teleport_to_in_level(
            Target::General(root),
            [10.0, 70.0, -4.0],
            Some([90.0, 0.0]),
            [false; 3],
            true,
        );
        let (own, vehicle) = guest_teleport_packets(&rx);
        assert_eq!(vehicle.len(), 1);
        assert_eq!(vehicle[0].id, root as i32);
        assert_eq!(vehicle[0].position, [10.0, 70.0, -4.0]);
        assert_eq!([vehicle[0].yaw, vehicle[0].pitch], [90.0, 0.0]);
        assert_eq!(own.len(), 1);
        assert_eq!(
            own[0].position,
            [
                10.0 + before[0],
                70.0 + (before[1] - 64.0),
                -4.0 + before[2]
            ]
        );
        assert_eq!([own[0].yaw, own[0].pitch], [120.0, 0.0]);
    });
}

#[test]
#[ignore = "daily"]
fn a_seated_guest_that_runs_the_teleport_is_published_once() {
    with_types_and_guests(|game| {
        let (root, rx) = seat_guest_on_general_root(game);
        let _ = guest_teleport_packets(&rx);
        game.as_guest(13, |game| {
            game.teleport_to_in_level(
                Target::General(root),
                [10.0, 70.0, -4.0],
                Some([90.0, 0.0]),
                [false; 3],
                true,
            );
        })
        .unwrap();
        let (own, vehicle) = guest_teleport_packets(&rx);
        assert_eq!(own.len(), 1);
        assert_eq!([own[0].yaw, own[0].pitch], [120.0, 0.0]);
        assert_eq!(vehicle.len(), 1);
        assert_eq!([vehicle[0].yaw, vehicle[0].pitch], [90.0, 0.0]);
    });
}

#[test]
#[ignore = "daily"]
fn a_deep_general_chain_teleports_without_a_stack_frame_per_rider() {
    with_types(|game| {
        let poses: Vec<_> = (0..3000)
            .map(|n| ([0.0, 64.0 + n as f64 * 0.5, 0.0], [0.0, 0.0]))
            .collect();
        let ids = general_chain(game, &poses, 5);
        game.teleport_to_in_level(
            Target::General(ids[0]),
            [8.0, 64.0, 8.0],
            None,
            [false; 3],
            true,
        );
        let last = game.resident_entities.general.row(ids[2999]).unwrap();
        assert_eq!(last.position, Vec3::new(8.0, 64.0 + 2999.0 * 0.5, 8.0));
    });
}

#[test]
#[ignore = "daily"]
fn killing_or_removing_a_general_row_keeps_its_passengers_in_the_level() {
    with_types(|game| {
        for (word, by_command) in [(1, true), (2, false)] {
            let rider = record_tag("mymod:robot", [word, 1, 0, 0], b"", Vec::new());
            let root = record_tag("mymod:robot", [word, 0, 0, 0], b"", vec![rider]);
            let start = game.resident_entities.general.rows.len();
            game.load_listed("mymod:robot", None, None, root).unwrap();
            let rows = &game.resident_entities.general.rows;
            let (root, rider) = (
                game.entity(rows[start].id).unwrap(),
                game.entity(rows[start + 1].id).unwrap(),
            );

            if by_command {
                game.kill_target(Target::General(root.id()));
            } else {
                assert!(root.remove(game, mod_api::RemovalReason::Discarded));
                assert_eq!(rider.vehicle(game), None);
                game.native_removals_flush();
            }
            assert!(game.resident_entities.general.row(root.id()).is_none());
            assert!(!game.persist.uuids.contains_key(&root.id()));
            assert_eq!(root.position(game), None);
            assert_eq!(rider.position(game), Some([0.5, 64.0, 0.5]));
            assert_eq!(rider.vehicle(game), None);
        }
    });
}

struct ObserveDismounts(std::sync::Arc<std::sync::Mutex<Vec<(u32, String)>>>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for ObserveDismounts {
    fn entity_mount(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::EntityMountEvent<'_>,
    ) -> mod_api::Flow {
        if !event.mounting {
            self.0
                .lock()
                .unwrap()
                .push((event.entity, event.vehicle_kind.to_owned()));
        }
        mod_api::Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn removing_a_vehicle_dismounts_general_and_mob_passengers_immediately_in_order() {
    use sunrise_gameplay::game::server_side::minecarts_server::CartHit;
    use sunrise_sim::mobs::Mob;
    use {
        sunrise_gameplay_common::game::minecarts::CartKind,
        sunrise_gameplay_common::game::minecarts::Minecart,
    };

    let dismounts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let observer =
        mod_api::ModEntry::new("dismounts", Box::new(ObserveDismounts(dismounts.clone())));
    with_types_and(Some(observer), |game| {
        let at = Vec3::new(0.5, 64.0, 0.5);
        game.options.dedicated = Some(Vec::new());
        game.build_registries();
        let mut outcomes = Vec::new();
        for (case, vehicle) in [
            "cart damage",
            "cart kill",
            "boat damage",
            "stand damage",
            "mob death",
            "mob death with only a mob rider",
        ]
        .into_iter()
        .zip(71..)
        {
            match case {
                "boat damage" => {
                    game.boats
                        .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                            vehicle,
                            "oak".into(),
                            at,
                            0.0,
                        ))
                }
                "stand damage" => game
                    .armor_stands
                    .push(sunrise_sim::entities::ArmorStand::new(vehicle, at, 0.0)),
                "mob death" | "mob death with only a mob rider" => {
                    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    game.mobs.push(Mob::new(vehicle, kind, at, 0.0));
                }
                _ => game
                    .carts
                    .push(Minecart::new(vehicle, CartKind::Rideable, at)),
            }
            let rider_id = if case == "mob death with only a mob rider" {
                // The occupancy check runs with both the vehicle and its only
                // rider in the mob tick's lent list, not in Game::mobs.
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                game.mobs.push(Mob::new(91, kind, at, 0.0));
                91
            } else {
                game.load_listed(
                    "mymod:robot",
                    None,
                    None,
                    record_tag("mymod:robot", [vehicle as i32, 0, 0, 0], b"", Vec::new()),
                )
                .unwrap();
                game.resident_entities.general.rows.last().unwrap().id
            };
            let rider = game.entity(rider_id).unwrap();
            let vehicle_ref = game.entity(vehicle).unwrap();
            game.entity_start_riding(rider, vehicle_ref, true).unwrap();
            let mut expected = vec![rider_id];
            if case == "boat damage" {
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let pig = 90;
                game.mobs.push(Mob::new(pig, kind, at, 0.0));
                game.entity_start_riding(game.entity(pig).unwrap(), vehicle_ref, true)
                    .unwrap();
                expected.push(pig);
            }
            // A previous general tick has made the passenger index current.
            // No tick is allowed between removal and checking the rider links.
            game.tick_general().unwrap();
            match case {
                "cart damage" => {
                    assert!(game.hurt_cart_result(0, 5.0, false, false, CartHit::default()))
                }
                "cart kill" => {
                    assert!(game.as_console(|g| g.run_command("kill @e[type=minecraft:minecart]")))
                }
                "boat damage" => assert!(game.hurt_boat(0, 5.0, false)),
                "stand damage" => {
                    assert!(game.break_armor_stand_by_anything(0, "minecraft:in_fire", 0))
                }
                "mob death" | "mob death with only a mob rider" => {
                    game.world.chunks.insert(
                        (0, 0),
                        std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                    );
                    let index = game.mobs.iter().position(|mob| mob.id == vehicle).unwrap();
                    game.mobs.update(index, |mob| {
                        mob.health = 0.0;
                        mob.death_time = 19;
                    });
                    game.server.died.insert(vehicle);
                    game.tick_creature_entities().unwrap();
                }
                _ => unreachable!(),
            }
            assert!(game.entity(vehicle).is_none(), "{case} removed the vehicle");
            let still_riding: Vec<_> = expected
                .iter()
                .copied()
                .filter(|&id| game.entity_is_passenger(game.entity(id).unwrap()) != Some(false))
                .collect();
            let actual: Vec<_> = dismounts
                .lock()
                .unwrap()
                .drain(..)
                .map(|(id, _)| id)
                .collect();
            outcomes.push((case, still_riding, actual, expected));
        }
        // Primed TNT is not a typed start_riding target, but the real NBT
        // loader admits its passengers. Fuse expiry must let them go too.
        assert!(game.ui.rules.set(
            "tnt_explodes",
            sunrise_game_data::gamerules::Value::Bool(false),
        ));
        game.load_tnt(
            Tag::Compound(
                [
                    ("id".into(), Tag::String("minecraft:tnt".into())),
                    (
                        "Pos".into(),
                        Tag::List(at.components().map(|n| Tag::Double(n.to_bits())).into()),
                    ),
                    ("fuse".into(), Tag::Short(1)),
                    (
                        "Passengers".into(),
                        Tag::List(vec![record_tag(
                            "mymod:robot",
                            [99, 0, 0, 0],
                            b"",
                            Vec::new(),
                        )]),
                    ),
                ]
                .into(),
            ),
            None,
        );
        let tnt = game.tnts[0].id;
        let rider = game.resident_entities.general.rows.last().unwrap().id;
        assert_eq!(game.general_row(rider).unwrap().mount, Some(tnt));
        game.tick_one_tnt(0, &[]).unwrap();
        assert!(game.tnts.is_empty());
        assert_eq!(game.general_row(rider).unwrap().mount, None);
        let actual = std::mem::take(&mut *dismounts.lock().unwrap());
        assert_eq!(actual, [(rider, "minecraft:tnt".into())]);
        for (case, still_riding, notifications, expected) in outcomes {
            assert_eq!(
                (still_riding, notifications),
                (Vec::new(), expected),
                "{case}"
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_general_entity_turns_in_place_without_dismounting() {
    with_types(|game| {
        let rider = record_tag("mymod:robot", [9, 9, 9, 9], b"", Vec::new());
        let root = record_tag("mymod:robot", [1, 2, 3, 4], b"", vec![rider]);
        game.load_listed("mymod:robot", None, None, root).unwrap();
        let rows = &game.resident_entities.general.rows;
        let (root, rider) = (
            game.entity(rows[0].id).unwrap(),
            game.entity(rows[1].id).unwrap(),
        );

        assert!(rider.set_rotation(game, 90.0, 30.0));
        assert_eq!(rider.rotation(game), Some([90.0, 30.0]));
        assert_eq!(rider.position(game), Some([0.5, 64.0, 0.5]));
        assert_eq!(rider.vehicle(game), Some(root));
        assert!(!rider.set_rotation(game, f32::NAN, 0.0));
        assert_eq!(rider.rotation(game), Some([90.0, 30.0]));
    });
}

#[test]
#[ignore = "daily"]
fn a_general_row_has_its_declared_kind_and_is_selectable_by_type() {
    with_types(|game| {
        for (key, uuid) in [("mymod:robot", [1, 2, 3, 4]), ("mymod:ghost", [4, 3, 2, 1])] {
            game.load_listed(key, None, None, record_tag(key, uuid, b"", Vec::new()))
                .unwrap();
        }
        let rows = &game.resident_entities.general.rows;
        let (robot, ghost) = (
            game.entity(rows[0].id).unwrap(),
            game.entity(rows[1].id).unwrap(),
        );
        game.build_registries();

        assert_eq!(
            robot.kind(game).unwrap().id(game).as_deref(),
            Some("mymod:robot")
        );
        assert_eq!(
            ghost.kind(game).unwrap().id(game).as_deref(),
            Some("mymod:ghost")
        );
        assert!(game.as_console(|game| game.run_command("kill @e[type=mymod:robot]")));
        assert_eq!(robot.position(game), None);
        assert_eq!(ghost.position(game), Some([0.5, 64.0, 0.5]));
    });
}

#[test]
#[ignore = "daily"]
fn a_volume_selector_finds_a_general_rows_box_without_its_origin() {
    with_types(|game| {
        game.load_listed(
            "mymod:robot",
            None,
            None,
            record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new()),
        )
        .unwrap();
        let id = game.resident_entities.general.rows[0].id;
        game.build_registries();
        // The 0.8-wide row is centered at x=0.5. The selector starts at x=0.75,
        // so the edge of the box enters its volume but the position does not.
        assert!(game.as_console(|game| {
            game.run_command("kill @e[type=mymod:robot,x=0.75,y=64,z=0,dx=0,dy=0,dz=0]")
        }));
        assert!(game.resident_entities.general.row(id).is_none());
    });
}

/// A tree travels between dimensions with its riders: the source ends with
/// zero rows and stale ids, the destination holds the same tree — same
/// UUIDs, same mount links — with fresh join identities, the root placed.
#[test]
#[ignore = "daily"]
fn general_tree_travels_to_another_dimension_with_its_riders() {
    use mod_api::DimensionKey;
    use std::num::NonZeroU32;
    use sunrise_gameplay::game::entity_residency::SlotAccess;
    with_types(|game| {
        game.reset_entity_residency();
        let grandchild = record_tag("mymod:robot", [8, 8, 8, 8], b"", Vec::new());
        let rider = record_tag("mymod:robot", [9, 9, 9, 9], b"", vec![grandchild]);
        let sibling = record_tag("mymod:robot", [7, 7, 7, 7], b"", Vec::new());
        let root = record_tag("mymod:robot", [1, 2, 3, 4], b"", vec![rider, sibling]);
        game.load_listed("mymod:robot", None, None, root).unwrap();
        let rows = &game.resident_entities.general.rows;
        let (old_root, old_rider, old_join) = (rows[0].id, rows[1].id, rows[0].join);
        {
            assert_eq!(
                game.entity(old_root).unwrap().set_persistent(game, false),
                Ok(true)
            );
        }
        assert!(
            game.teleport_general_tree_across(
                old_root,
                crate::game::Dim::Nether,
                [8.5, 100.0, 8.5],
                None,
                true,
                sunrise_gameplay::game::vehicle_teleport::TeleportMotion::Stop,
            )
            .unwrap()
        );
        // The source keeps nothing; its stale ids no longer resolve.
        assert!(game.resident_entities.general.rows.is_empty());
        let id = |value: u32| NonZeroU32::new(value).unwrap();
        for gone in [old_root, old_rider] {
            assert!(matches!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(gone), |_| panic!(
                    "a stale id resolved"
                )),
                SlotAccess::Missing
            ));
        }
        // The destination holds the same tree, freshly admitted.
        game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
            .unwrap();
        let rows = &game.resident_entities.general.rows;
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[2].uuid, [8, 8, 8, 8]);
        assert_eq!(rows[2].mount, Some(rows[1].id));
        assert_eq!(rows[3].uuid, [7, 7, 7, 7]);
        assert_eq!(rows[3].mount, Some(rows[0].id));
        assert_eq!(rows[0].uuid, [1, 2, 3, 4]);
        assert_eq!(rows[1].uuid, [9, 9, 9, 9]);
        assert_eq!(rows[1].mount, Some(rows[0].id));
        assert_eq!(rows[0].position, Vec3::new(8.5, 100.0, 8.5));
        // Every row of the record stood at the same place: each rider arrives with its vehicle.
        assert!(rows.iter().all(|row| row.position == rows[0].position));
        assert!(rows[0].join.is_joined() && rows[0].join != old_join);
        assert!(matches!(
            game.with_resident_slot(&DimensionKey::NETHER, id(rows[0].id), |_| true),
            SlotAccess::Done(true)
        ));
        assert!(
            saved_chunk(game, (0, 0)).is_empty(),
            "the live tree remains transient in its destination"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_cross_level_general_tree_applies_each_riders_transition_on_arrival() {
    use sunrise_gameplay::game::vehicle_teleport::TeleportMotion;
    with_types(|game| {
        let ids = general_chain(
            game,
            &[
                ([0.0, 64.0, 0.0], [0.0, 0.0]),
                ([1.5, 65.25, -0.5], [30.0, 10.0]),
            ],
            8,
        );
        game.resident_entities
            .general
            .row_mut(ids[1])
            .unwrap()
            .velocity = Vec3::new(0.5, 0.5, 0.5);
        assert!(
            game.teleport_general_tree_across(
                ids[0],
                crate::game::Dim::Nether,
                [8.0, 100.0, 8.0],
                Some([90.0, 20.0]),
                true,
                TeleportMotion::Stop,
            )
            .unwrap()
        );
        game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
            .unwrap();
        let rider = game.resident_entities.general.row(ids[1]).unwrap();
        assert_eq!(rider.position, Vec3::new(9.5, 101.25, 7.5));
        assert_eq!(rider.rotation, [120.0, 30.0]);
        assert_eq!(rider.velocity, Vec3::ZERO);
        assert_eq!(rider.mount, Some(ids[0]));
        let root = game.resident_entities.general.row(ids[0]).unwrap();
        assert_eq!(root.rotation, [90.0, 20.0]);
    });
}

#[test]
#[ignore = "daily"]
fn a_general_rider_targeted_across_levels_leaves_its_vehicle_first() {
    use sunrise_gameplay::game::vehicle_teleport::TeleportMotion;
    with_types(|game| {
        let ids = general_chain(
            game,
            &[
                ([0.0, 64.0, 0.0], [0.0, 0.0]),
                ([0.0, 65.0, 0.0], [0.0, 0.0]),
            ],
            9,
        );
        let moved = game
            .teleport_target_across(
                Target::General(ids[1]),
                sunrise_gameplay_common::game::Dim::Nether,
                [8.0, 100.0, 8.0],
                None,
                false,
                TeleportMotion::Stop,
            )
            .unwrap();
        assert_eq!(moved, Some(true));
        assert_eq!(game.resident_entities.general.rows.len(), 1);
        assert_eq!(game.resident_entities.general.rows[0].id, ids[0]);
        game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
            .unwrap();
        let rows = &game.resident_entities.general.rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, ids[1]);
        assert_eq!(rows[0].mount, None);
        assert_eq!(rows[0].position, Vec3::new(8.0, 100.0, 8.0));
    });
}

/// A refused transfer returns the whole tree: a destination row of one of
/// the tree's UUIDs refuses before anything moves, and the source keeps
/// every row at its place with its join identity.
#[test]
#[ignore = "daily"]
fn refused_dimension_transfer_returns_the_whole_tree() {
    with_types(|game| {
        // Seed the destination with a row of the travelling UUID.
        game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
            .unwrap();
        game.load_listed(
            "mymod:robot",
            None,
            None,
            record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new()),
        )
        .unwrap();
        game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
            .unwrap();
        let grandchild = record_tag("mymod:robot", [8, 8, 8, 8], b"", Vec::new());
        let rider = record_tag("mymod:robot", [9, 9, 9, 9], b"", vec![grandchild]);
        let sibling = record_tag("mymod:robot", [7, 7, 7, 7], b"", Vec::new());
        let root = record_tag("mymod:robot", [1, 2, 3, 4], b"", vec![rider, sibling]);
        game.load_listed("mymod:robot", None, None, root).unwrap();
        let before: Vec<_> = game
            .resident_entities
            .general
            .rows
            .iter()
            .map(|row| (row.id, row.uuid, row.join, row.position))
            .collect();
        let root_id = game.resident_entities.general.rows[0].id;
        assert!(
            game.teleport_general_tree_across(
                root_id,
                crate::game::Dim::Nether,
                [8.5, 100.0, 8.5],
                None,
                true,
                sunrise_gameplay::game::vehicle_teleport::TeleportMotion::Stop,
            )
            .is_err()
        );
        let after: Vec<_> = game
            .resident_entities
            .general
            .rows
            .iter()
            .map(|row| (row.id, row.uuid, row.join, row.position))
            .collect();
        assert_eq!(before, after, "the tree returns exactly as it stood");
    });
}

#[test]
#[ignore = "daily"]
fn no_save_rider_is_omitted_from_saved_tree() {
    with_types(|game| {
        let rider = record_tag("mymod:ghost", [9, 9, 9, 9], b"rider", Vec::new());
        let root = record_tag("mymod:robot", [1, 2, 3, 4], b"payload", vec![rider]);
        game.load_listed("mymod:robot", None, None, root).unwrap();
        let rows = &game.resident_entities.general.rows;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].uuid, [1, 2, 3, 4]);
        assert_eq!(rows[1].mount, Some(rows[0].id));
        assert_eq!(rows[0].payload_bytes, b"payload");
        assert_eq!(rows[0].payload_version, 3);
        assert!(rows[0].join.is_joined() && rows[1].join.is_joined());
        assert_eq!(game.persist.uuids.get(&rows[0].id), Some(&[1, 2, 3, 4]));
        assert_eq!(game.persist.uuids.get(&rows[1].id), Some(&[9, 9, 9, 9]));
        // One record per root, the rider inside it, the payload as loaded.
        let saved = saved_chunk(game, (0, 0));
        assert_eq!(saved.len(), 1);
        assert!(saved[0].get(format::PASSENGERS).is_none());
        assert_eq!(
            saved[0].get(format::PAYLOAD),
            Some(&Tag::ByteArray(b"payload".to_vec()))
        );
        assert_eq!(
            saved[0].get(format::UUID),
            Some(&Tag::IntArray(vec![1, 2, 3, 4]))
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_general_rider_keeps_minecart_drag_and_pickup_occupied() {
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::mobs::Mob;
    use {
        sunrise_gameplay_common::game::minecarts::CartKind,
        sunrise_gameplay_common::game::minecarts::Minecart,
    };
    with_types(|game| {
        let at = Vec3::new(8.5, 64.0, 8.5);
        let cart_id = 5000;
        let pig_id = 5001;
        game.world.chunks.insert((0, 0), Chunk::empty(0, 0).into());
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let rail = registry.default_state("minecraft:powered_rail").unwrap();
        game.world.set(8, 63, 8, stone).unwrap();
        game.world.set(8, 64, 8, rail).unwrap();
        let mut cart = Minecart::new(cart_id, CartKind::Rideable, at);
        cart.velocity = Vec3::new(0.0, 0.0, 0.25);
        game.carts.push(cart);
        game.load_listed(
            "mymod:robot",
            None,
            None,
            record_tag("mymod:robot", [2, 3, 4, 5], b"rider", Vec::new()),
        )
        .unwrap();
        let rider = game
            .entity(game.resident_entities.general.rows[0].id)
            .unwrap();
        let cart = game.entity(cart_id).unwrap();
        assert_eq!(game.entity_start_riding(rider, cart, false), Ok(()));
        // Exercise the maintained index, not the scan while it is stale after mounting.
        game.tick_general().unwrap();
        assert_eq!(cart.is_vehicle(game), Some(true));
        assert_eq!(
            collect_entity_visit(|visit| cart.visit_passengers(game, visit)),
            Ok(vec![rider])
        );
        let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        // The pig touches the moving cart, but the coincident starting positions
        // mean it applies no initial push to the cart.
        game.mobs.push(Mob::new(pig_id, pig, at, 0.0));
        game.server.tickets.0.update(None);
        game.tick_carts();
        // The unpowered rail halves the speed. The occupied cart moves at
        // three quarters of that speed, then uses 0.997 rather than 0.96 drag.
        assert_eq!(game.carts[0].position.z, at.z + 0.25 * 0.5 * 0.75);
        assert_eq!(game.carts[0].velocity.z, 0.25 * 0.5 * 0.997);
        assert_eq!(game.mob_object(pig_id).unwrap().extra.mount, None);
    });
}

#[test]
#[ignore = "daily"]
fn a_general_rider_fills_a_chest_boat_and_keeps_it_from_being_pushed() {
    use sunrise_gameplay_common::game::boats_common::Boat;
    use sunrise_sim::mobs::Mob;

    with_types(|game| {
        let at = Vec3::new(8.5, 64.0, 8.5);
        fixture_resident_column(&mut game.world, (0, 0));
        fixture_resident_column(&mut game.world, (1, 0));
        let (full, empty, pig) = (5000, 5001, 5002);
        game.boats
            .push(Boat::new(full, "oak".into(), at, 0.0).with_chest());
        game.boats.push(Boat::new(
            empty,
            "oak".into(),
            at.add_vector(Vec3::new(1.0, 0.0, 0.0)),
            0.0,
        ));
        game.load_listed(
            "mymod:robot",
            None,
            None,
            record_tag("mymod:robot", [2, 3, 4, 5], b"rider", Vec::new()),
        )
        .unwrap();
        let rider = game
            .entity(game.resident_entities.general.rows[0].id)
            .unwrap();
        let boat = game.entity(full).unwrap();
        game.entity_start_riding(rider, boat, false).unwrap();
        game.tick_general().unwrap();
        assert_eq!(boat.is_vehicle(game), Some(true));
        assert_eq!(
            collect_entity_visit(|visit| boat.visit_passengers(game, visit)),
            Ok(vec![rider])
        );

        let pig_type = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        game.mobs.push(Mob::new(
            pig,
            pig_type,
            at.add_vector(Vec3::new(8.0, 0.0, 0.0)),
            0.0,
        ));
        let pig = game.entity(pig).unwrap();
        for force in [false, true] {
            assert_eq!(
                game.entity_start_riding(pig, boat, force),
                Err(mod_api::StartRidingRefusal::Full),
            );
        }

        // Each boat sees the other. The empty one gets the impulse in both
        // directions of the contact; the boat with the general rider gets none.
        let pushed = Vec3::new(f64::from(0.05_f32), 0.0, 0.0);
        for i in [0, 1] {
            game.boats.update(1, |boat| boat.velocity = Vec3::ZERO);
            game.boat_contacts(i);
            assert_eq!(game.boats[0].velocity, Vec3::ZERO);
            assert_eq!(game.boats[1].velocity, pushed);
        }
    });
}

/// `Entity.saveWithoutId` and `Entity.load`: a general record's `CustomName` and
/// `CustomNameVisible` survive the save format, `CustomNameVisible` is written only when set,
/// and a record saved without either loads with no name and false.
#[test]
fn a_general_records_custom_name_survives_save_and_load() {
    let mut record = GeneralEntityRecord {
        position: [0.5, 64.0, 0.5],
        velocity: [0.0; 3],
        rotation: [0.0; 2],
        owner: None,
        custom_name: Some(Text::colored("Grumbles", 0xFF5555)),
        custom_name_visible: true,
        base: Default::default(),
        on_ground: false,
        fall_distance: 0.0,
        uuid: [1, 2, 3, 4],
        envelope: GeneralTypeEnvelope {
            type_key: "mymod:robot".to_owned(),
            payload_version: 3,
            payload_bytes: Vec::new(),
        },
        passengers: Vec::new(),
    };
    let tag = record.to_tag();
    assert_eq!(tag.get("CustomNameVisible"), Some(&Tag::Byte(1)));
    assert_eq!(record_from_compound(&tag).unwrap(), record);

    record.custom_name_visible = false;
    let tag = record.to_tag();
    assert_eq!(tag.get("CustomNameVisible"), None);
    assert_eq!(record_from_compound(&tag).unwrap(), record);

    // An old record never wrote either key.
    record.custom_name = None;
    let old = record.to_tag();
    assert_eq!(old.get("CustomName"), None);
    let loaded = record_from_compound(&old).unwrap();
    assert_eq!(loaded.custom_name, None);
    assert!(!loaded.custom_name_visible);
}

/// `Entity.saveWithoutId` and `Entity.load`: a general record's `NoGravity`, `OnGround` and
/// `fall_distance` survive the save format, `NoGravity` is written only when set, and a record
/// saved before they existed loads with false, false and no fall.
#[test]
fn a_general_records_no_gravity_ground_and_fall_survive_save_and_load() {
    let mut record = GeneralEntityRecord {
        position: [0.5, 64.0, 0.5],
        velocity: [0.0; 3],
        rotation: [0.0; 2],
        owner: None,
        custom_name: None,
        custom_name_visible: false,
        base: sunrise_gameplay_common::game::native_entity_base::EntityBase {
            no_gravity: true,
            ..Default::default()
        },
        on_ground: true,
        fall_distance: 2.5,
        uuid: [1, 2, 3, 4],
        envelope: GeneralTypeEnvelope {
            type_key: "mymod:robot".to_owned(),
            payload_version: 3,
            payload_bytes: Vec::new(),
        },
        passengers: Vec::new(),
    };
    let tag = record.to_tag();
    assert_eq!(tag.get("NoGravity"), Some(&Tag::Byte(1)));
    assert_eq!(tag.get("OnGround"), Some(&Tag::Byte(1)));
    assert_eq!(
        tag.get("fall_distance"),
        Some(&Tag::Double(2.5f64.to_bits()))
    );
    assert_eq!(record_from_compound(&tag).unwrap(), record);

    record.base.no_gravity = false;
    record.on_ground = false;
    let tag = record.to_tag();
    assert_eq!(tag.get("NoGravity"), None);
    assert_eq!(tag.get("OnGround"), Some(&Tag::Byte(0)));
    assert_eq!(record_from_compound(&tag).unwrap(), record);

    // An old record never wrote any of the three keys.
    record.base.no_gravity = true;
    record.on_ground = true;
    let Tag::Compound(mut old) = record.to_tag() else {
        panic!("a record is a compound");
    };
    for key in ["NoGravity", "OnGround", "fall_distance"] {
        assert!(
            old.remove(&sunrise_core::nbt::JavaString::from(key))
                .is_some()
        );
    }
    let loaded = record_from_compound(&Tag::Compound(old)).unwrap();
    assert!(!loaded.base.no_gravity);
    assert!(!loaded.on_ground);
    assert_eq!(loaded.fall_distance, 0.0);
}

/// `Entity.load`'s `getDoubleOr("fall_distance", 0.0)` reads any number and takes the default for
/// another tag, where the record's own fields would refuse: a wrong-typed fall distance loads as zero.
#[test]
fn a_general_records_fall_distance_reads_any_number_and_defaults_otherwise() {
    let record = GeneralEntityRecord {
        position: [0.5, 64.0, 0.5],
        velocity: [0.0; 3],
        rotation: [0.0; 2],
        owner: None,
        custom_name: None,
        custom_name_visible: false,
        base: Default::default(),
        on_ground: false,
        fall_distance: 2.5,
        uuid: [1, 2, 3, 4],
        envelope: GeneralTypeEnvelope {
            type_key: "mymod:robot".to_owned(),
            payload_version: 3,
            payload_bytes: Vec::new(),
        },
        passengers: Vec::new(),
    };
    let with = |value: Tag| {
        let Tag::Compound(mut map) = record.to_tag() else {
            panic!("a record is a compound");
        };
        map.insert("fall_distance".into(), value);
        record_from_compound(&Tag::Compound(map))
            .unwrap()
            .fall_distance
    };
    assert_eq!(with(Tag::Int(3)), 3.0);
    assert_eq!(with(Tag::Float(1.5f32.to_bits())), 1.5);
    assert_eq!(with(Tag::String("far".into())), 0.0);
    assert_eq!(with(Tag::List(Vec::new())), 0.0);
}

/// A live general row's name and visibility: loaded from its save, read through its handle
/// (`EntityRef::name` and the styled `EntityRef::custom_name`), matched by `@e[name=...]` (an
/// unnamed row is not), changed by a mod and saved again.
#[test]
#[ignore = "daily"]
fn a_general_rows_custom_name_is_read_selected_and_saved() {
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    with_types(|game| {
        fixture_resident_column(&mut game.world, (0, 0));
        let mut named = record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new());
        if let Tag::Compound(map) = &mut named {
            map.insert("CustomName".into(), Tag::String("Grumbles".into()));
            map.insert("CustomNameVisible".into(), Tag::Byte(1));
        }
        game.load_listed("mymod:robot", None, None, named).unwrap();
        let plain = record_tag("mymod:robot", [4, 3, 2, 1], b"", Vec::new());
        game.load_listed("mymod:robot", None, None, plain).unwrap();
        let rows = &game.resident_entities.general.rows;
        let (named_id, plain_id) = (rows[0].id, rows[1].id);
        let (named, plain) = (
            game.entity(named_id).unwrap(),
            game.entity(plain_id).unwrap(),
        );
        game.build_registries();

        assert_eq!(named.name(game).as_deref(), Some("Grumbles"));
        assert_eq!(
            named.custom_name(game).unwrap(),
            Some(mod_api::Text::literal("Grumbles"))
        );
        assert_eq!(named.custom_name_visible(game), Some(true));
        assert_ne!(plain.name(game).as_deref(), Some("Grumbles"));
        assert_eq!(plain.custom_name(game).unwrap(), None);
        assert_eq!(plain.custom_name_visible(game), Some(false));

        // A mod names the unnamed row; the save and the selector follow.
        let name: mod_api::Text = Text::colored("Rumbles", 0x55FF55).try_into_api().unwrap();
        assert!(plain.set_custom_name(game, Some(name.clone())).unwrap());
        assert!(plain.set_custom_name_visible(game, true));
        assert_eq!(plain.name(game).as_deref(), Some("Rumbles"));
        assert_eq!(plain.custom_name(game).unwrap(), Some(name));
        let saved = game.general_tree_nbt(plain_id, false).unwrap();
        let loaded = record_from_compound(&saved).unwrap();
        assert_eq!(loaded.custom_name, Some(Text::colored("Rumbles", 0x55FF55)));
        assert!(loaded.custom_name_visible);

        assert!(plain.set_custom_name(game, None).unwrap());
        assert!(plain.set_custom_name_visible(game, false));
        assert_eq!(plain.custom_name(game).unwrap(), None);
        let saved = game.general_tree_nbt(plain_id, false).unwrap();
        assert_eq!(saved.get("CustomName"), None);
        assert_eq!(saved.get("CustomNameVisible"), None);

        assert!(game.as_console(|game| game.run_command("kill @e[name=Grumbles]")));
        assert!(game.resident_entities.general.row(named_id).is_none());
        assert!(game.resident_entities.general.row(plain_id).is_some());
    });
}

/// The selectors every entity answers (`type=`, `distance=`, `@n`, a UUID) also reach a
/// general row.
#[test]
#[ignore = "daily"]
fn a_general_row_answers_distance_nearest_and_uuid_selectors() {
    with_types(|game| {
        for chunk in [(0, 0), (2, 0), (5, 0)] {
            fixture_resident_column(&mut game.world, chunk);
        }
        for (at, uuid) in [
            (0.5, [1, 2, 3, 4]),
            (40.5, [5, 6, 7, 8]),
            (80.5, [9, 10, 11, 12]),
        ] {
            let mut tag = record_tag("mymod:robot", uuid, b"", Vec::new());
            if let Tag::Compound(map) = &mut tag {
                map.insert(
                    format::POSITION.into(),
                    Tag::List(
                        [at, 64.0, 0.5]
                            .map(|p| Tag::Double(f64::to_bits(p)))
                            .to_vec(),
                    ),
                );
            }
            game.load_listed("mymod:robot", None, None, tag).unwrap();
        }
        let rows = &game.resident_entities.general.rows;
        let (near, far, last) = (rows[0].id, rows[1].id, rows[2].id);
        game.build_registries();
        // `distance=..5` from the origin reaches only the near row.
        assert!(game.as_console(|game| {
            game.run_command(
                "execute positioned 0.5 64 0.5 run kill @e[type=mymod:robot,distance=..5]",
            )
        }));
        assert!(game.resident_entities.general.row(near).is_none());
        assert!(game.resident_entities.general.row(far).is_some());
        // `@n` from beside the far row picks it.
        assert!(game.as_console(|game| {
            game.run_command("execute positioned 41 64 0.5 run kill @n[type=mymod:robot]")
        }));
        assert!(game.resident_entities.general.row(far).is_none());
        // A UUID names one row.
        assert!(
            game.as_console(|game| game.run_command("kill 00000009-0000-000a-0000-000b0000000c"))
        );
        assert!(game.resident_entities.general.row(last).is_none());
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt2 as _;

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;

use sunrise_gameplay_common::game::piston_push::PistonPushGameExt1 as _;

/// #5082: Base-state handle edits use vanilla NBT keys, and fire immunity suppresses burning.
#[test]
#[ignore = "daily"]
fn a_general_entitys_base_state_round_trips_through_its_handle_and_record() {
    with_types(|game| {
        let ids = general_chain(game, &[([0.0, 64.0, 0.0], [0.0, 0.0])], 11);
        let handle = game.entity(ids[0]).unwrap();
        assert_eq!(handle.fire_ticks(game), Some(0));
        assert!(handle.set_remaining_fire_ticks(game, 40));
        assert!(handle.set_silent(game, true));
        assert!(handle.set_glowing(game, true));
        assert!(handle.set_no_gravity(game, true));
        assert!(handle.set_invulnerable(game, true));
        assert_eq!(handle.is_on_fire(game), Some(true));
        assert_eq!(handle.fire_immune(game), Some(false));
        let Tag::Compound(saved) = game.general_row_record(ids[0], false).unwrap().to_tag() else {
            panic!("a record saves as a compound");
        };
        assert_eq!(
            saved.get(&mod_api::JavaString::from("Fire")),
            Some(&Tag::Short(40))
        );
        assert_eq!(
            saved.get(&mod_api::JavaString::from("Invulnerable")),
            Some(&Tag::Byte(1))
        );
        for key in ["Silent", "Glowing", "NoGravity"] {
            assert_eq!(
                saved.get(&mod_api::JavaString::from(key)),
                Some(&Tag::Byte(1)),
                "{key}"
            );
        }
        assert!(handle.clear_fire(game));
        assert_eq!(handle.fire_ticks(game), Some(0));

        let tag = record_tag("mymod:blaze", [11, 12, 13, 14], b"", Vec::new());
        let start = game.resident_entities.general.rows.len();
        game.load_listed("mymod:blaze", None, None, tag).unwrap();
        let blaze = game
            .entity(game.resident_entities.general.rows[start].id)
            .unwrap();
        assert!(blaze.set_remaining_fire_ticks(game, 40));
        assert_eq!(blaze.fire_immune(game), Some(true));
        assert_eq!(blaze.is_on_fire(game), Some(false));
    });
}

/// A general row tracks silence and no-gravity as `Entity.DATA_SILENT` and `DATA_NO_GRAVITY`.
#[test]
#[ignore = "daily"]
fn a_general_rows_silent_and_no_gravity_are_tracked_to_clients() {
    use sunrise_protocol::entity_data::{Value, serializer};
    with_types(|game| {
        fixture_resident_column(&mut game.world, (0, 0));
        let mut tag = record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new());
        if let Tag::Compound(map) = &mut tag {
            map.insert("Silent".into(), Tag::Byte(1));
            map.insert("NoGravity".into(), Tag::Byte(1));
        }
        game.load_listed("mymod:robot", None, None, tag).unwrap();
        let mut states = sunrise_sim::lan::entity::States::default();
        game.lan_general_states(&mut states, 0);
        let [state] = states.as_slice() else {
            panic!("one row tracked");
        };
        for index in [4, 5] {
            assert!(
                state
                    .meta
                    .contains(&(index, serializer::BOOLEAN, Value::Bool(true))),
                "{index}: {:?}",
                state.meta
            );
        }
    });
}
