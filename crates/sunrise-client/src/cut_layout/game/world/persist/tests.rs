use std::collections::HashMap;

mod refused_entity_unload {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};
    use sunrise_gameplay::game::native_entity_refs::tests::{fixture_root, with_game_at};
    use sunrise_mod_runtime::attachments::{
        AttachmentCodec, AttachmentError, AttachmentType, Attachments, Holder,
    };

    const CHUNK: (i32, i32) = (100, 100);

    #[derive(Clone)]
    struct Fallible {
        value: i32,
        fail: Arc<AtomicBool>,
    }
    static FALLIBLE: AttachmentType<Fallible> = AttachmentType {
        namespace: "refusal",
        name: "fallible",
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

    fn request(root: &std::path::Path) -> sunrise_gameplay::game::world_loading::WorldRequest {
        let rules = Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla());
        let options = sunrise_sim::world_options::WorldOptions {
            world_type: sunrise_sim::save::WorldPreset::Flat,
            rules: sunrise_game_data::gamerules::GameRules::with_registry(rules.clone(), &[]),
            ..Default::default()
        };
        let mut request = sunrise_gameplay::game::world_loading::WorldRequest::new(
            "refusal",
            Some("refusal"),
            Some(1),
            true,
            Some(&options),
            rules,
        )
        .unwrap();
        request.save_root = root.to_owned();
        request
    }

    fn pump_until(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        done: impl Fn(&sunrise_gameplay::game::ServerGame<'_>) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(120);
        while !done(game) || game.server.streaming.busy() {
            assert!(Instant::now() < deadline, "streaming did not settle");
            game.server.streaming.wait(Duration::from_millis(5));
            game.server_update_world();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn an_unload_keeps_a_column_whose_entity_record_was_refused() {
        let scratch = tempfile::tempdir().unwrap();
        mod_platform::testing::with_mods(Default::default(), || {
            with_game_at(fixture_root(), |game| {
                game.load_world(request(&scratch.path().join("save")))
                    .unwrap();
                game.set_allow_commands(true);
                assert!(game.run_command("forceload add 1600 1600"));
                pump_until(game, |game| game.world.chunks.contains_key(&CHUNK));
                game.tick_world(true).unwrap();
                let target = game
                    .summon_with_nbt(
                        "minecraft:armor_stand",
                        [1604.5, 100.0, 1604.5],
                        "{NoGravity:1b}",
                    )
                    .unwrap()
                    .unwrap();
                let id = game.resident_target_id(target).unwrap();

                let fail = Arc::new(AtomicBool::new(false));
                FALLIBLE
                    .set(
                        game.entity_attachments.entry(id).or_default(),
                        Holder::Entity(id),
                        Fallible {
                            value: 7,
                            fail: fail.clone(),
                        },
                    )
                    .unwrap();
                fail.store(true, Ordering::Relaxed);

                assert!(game.run_command("forceload remove 1600 1600"));
                for _ in 0..40 {
                    game.server.streaming.wait(Duration::from_millis(5));
                    game.server_update_world();
                }
                assert!(
                    game.world.chunks.contains_key(&CHUNK),
                    "a refused entity record keeps its column"
                );
                assert!(game.entity(id).is_some(), "and the entity in it");
                game.server.save.0.flush().unwrap();
                assert!(
                    game.server
                        .save
                        .0
                        .load_entities(CHUNK.0, CHUNK.1)
                        .unwrap()
                        .is_empty(),
                    "no partial record was written"
                );

                fail.store(false, Ordering::Relaxed);
                pump_until(game, |game| !game.world.chunks.contains_key(&CHUNK));
                assert!(game.entity(id).is_none());
                game.server.save.0.flush().unwrap();
                let records = game.server.save.0.load_entities(CHUNK.0, CHUNK.1).unwrap();
                assert_eq!(records.len(), 1);
                let Some(Tag::Compound(saved)) = records[0].get(ATTACHMENTS_KEY) else {
                    panic!("the entity saved without its attachments");
                };
                let mut reloaded = Attachments::from_nbt(saved.clone());
                assert_eq!(
                    FALLIBLE
                        .get_existing(&mut reloaded, Holder::Entity(id))
                        .unwrap()
                        .map(|v| v.value),
                    Some(7)
                );
            });
        });
    }
}
use std::collections::HashSet;
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_core::position::BlockPos;
use sunrise_game_data::scoreboard::Scoreboard;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_level::chunk::Chunk;
use sunrise_mod_runtime::attachments::ATTACHMENTS_KEY;
// Test module extracted without changing its logical path.
use super::*;

use sunrise_gameplay::game::persist::{
    EntitySnapshot, ORB_LIVE, add_passengers, drop_unleashable_leash, join_saved_team, mark_fresh,
    parent_vehicle_root, read_attachments, root_vehicle_tag, take_fresh, vehicle_tree,
    with_raw_over, write_attachments,
};
use sunrise_gameplay_common::world_biomes;
#[cfg(test)]
use sunrise_sim::entity_nbt;
use sunrise_sim::save::ChunkExtras;
use sunrise_sim::save::text;

include!("position_load_tests.rs");

mod transient_entities {
    use super::*;
    use mod_api::EntityRef;
    use {
        sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
    };

    fn with_server(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        mod_platform::testing::with_mods(Default::default(), || {
            with_game(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                test(game);
            });
        });
    }

    fn display(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> EntityRef {
        let target = game
            .summon_with_nbt("minecraft:text_display", [4.0, 64.0, 4.0], "{}")
            .unwrap()
            .unwrap();
        game.entity(game.resident_target_id(target).unwrap())
            .unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn a_transient_display_is_live_and_can_resume_saving_with_its_uuid() {
        with_server(|game| {
            let display = display(game);
            let chunks = HashSet::from([(0, 0)]);
            assert_eq!(display.set_persistent(game, false), Ok(true));
            assert_eq!(display.set_persistent(game, false), Ok(true));
            assert_eq!(game.entity(display.id()), Some(display));
            let uuid = sunrise_sim::lan::playerdata::uuid_ints(game.entity_uuid(display).unwrap());
            assert_eq!(game.entity_chunks(&chunks, false), vec![((0, 0), vec![])]);
            let (capture, _) = game.snapshot_entity_chunks(&chunks, EntitySnapshot::Structure);
            assert_eq!(capture[0].1.len(), 1);
            assert_eq!(entity_nbt::uuid(&capture[0].1[0]), Some(uuid));

            assert_eq!(display.set_persistent(game, true), Ok(true));
            let restored = game.entity_chunks(&chunks, false);
            assert_eq!(restored[0].1.len(), 1);
            assert_eq!(entity_nbt::uuid(&restored[0].1[0]), Some(uuid));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn unloading_a_transient_display_erases_its_old_record() {
        with_server(|game| {
            let display = display(game);
            game.save_entities(true).unwrap();
            assert_eq!(game.server.save.0.load_entities(0, 0).unwrap().len(), 1);
            assert_eq!(display.set_persistent(game, false), Ok(true));
            assert_eq!(
                game.server.save.0.load_entities(0, 0).unwrap().len(),
                1,
                "setting the flag does not rewrite an old snapshot"
            );

            let chunk = game.world.chunks.remove(&(0, 0)).unwrap();
            let keys = HashSet::from([(0, 0)]);
            let (records, _) = game.unloading_entity_snapshots(&keys);
            game.server.save.0.save_entities_later(records).unwrap();
            game.remove_unloading_entities(&keys);
            game.server.save.0.flush().unwrap();
            assert_eq!(game.entity(display.id()), None);
            let records = game.server.save.0.load_entities(0, 0).unwrap();
            assert!(records.is_empty());
            game.world.chunks.insert((0, 0), chunk);
            game.world.queue_entity_nbt((0, 0), records);
            game.spawn_loaded_entities();
            assert!(game.server.others.0.is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_root_controls_chunk_and_player_vehicle_saves_but_not_logout_removal() {
        with_server(|game| {
            let root = game
                .summon_with_nbt(
                    "minecraft:pig",
                    [4.0, 64.0, 4.0],
                    r#"{Passengers:[{id:"minecraft:chicken",UUID:[I;0,0,0,321]}]}"#,
                )
                .unwrap()
                .unwrap();
            let root = game.entity(game.resident_target_id(root).unwrap()).unwrap();
            let rider = game.mob_passenger_ids(root.id())[0];
            let chunks = HashSet::from([(0, 0)]);
            assert_eq!(
                game.entity(rider).unwrap().set_persistent(game, false),
                Ok(true)
            );
            let saved = game.entity_chunks(&chunks, false);
            let Some(Tag::List(passengers)) = saved[0].1[0].get("Passengers") else {
                panic!("marking the rider alone must not drop it from its vehicle");
            };
            assert_eq!(entity_nbt::uuid(&passengers[0]), Some([0, 0, 0, 321]));
            let (wrapper, attach) = game.snapshot_parent_vehicle(rider, &[], false).unwrap();
            assert_eq!(attach, [0, 0, 0, 321]);
            assert_eq!(
                wrapper
                    .get("Entity")
                    .and_then(|tree| tree.get("Passengers")),
                Some(&Tag::List(passengers.clone()))
            );

            assert_eq!(root.set_persistent(game, false), Ok(true));
            assert_eq!(game.entity_chunks(&chunks, false), vec![((0, 0), vec![])]);
            assert_eq!(game.snapshot_parent_vehicle(rider, &[], false), None);
            assert!(game.entity(root.id()).is_some());
            assert_eq!(game.snapshot_parent_vehicle(rider, &[], true), None);
            assert_eq!(game.entity(root.id()), None);
            assert_eq!(game.entity(rider), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_persistence_write_refuses_players_stale_handles_clients_and_setup() {
        with_server(|game| {
            let display = display(game);
            game.player =
                Some(sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            bind_local_player(game, 100, 25);
            let player = game.entity(100).unwrap();
            assert_eq!(player.set_persistent(game, false), Ok(false));
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            assert_eq!(display.set_persistent(game, false), Ok(false));
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
            assert!(display.discard(game));
            assert_eq!(display.set_persistent(game, false), Ok(false));
        });
    }
}

#[test]
#[ignore = "daily"]
fn neutral_load_restores_only_the_saved_live_target_with_or_without_listeners() {
    use std::sync::{Arc, Mutex};

    struct Observe(Arc<Mutex<Vec<(u32, Option<u32>)>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Observe {
        fn living_change_target(
            &self,
            _host: &dyn mod_api::Host,
            event: &mut mod_api::LivingChangeTargetEvent<'_>,
        ) -> mod_api::Flow {
            self.0.lock().unwrap().push((
                event.mob.entity().id(),
                event.original_target.map(|target| target.entity().id()),
            ));
            mod_api::Flow::Continue
        }
    }

    fn check(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        observed: Option<&Mutex<Vec<(u32, Option<u32>)>>>,
    ) {
        let uuid = [274, 0, 0, 1];
        let entity_at_origin = |kind: &str| {
            let mut mob = entity(kind);
            if let Tag::Compound(ref mut tag) = mob {
                tag.insert(
                    "Pos".into(),
                    Tag::List(vec![Tag::Double(0), Tag::Double(0), Tag::Double(0)]),
                );
            }
            mob
        };
        let mut zombie = entity_at_origin("minecraft:zombie");
        if let Tag::Compound(ref mut tag) = zombie {
            tag.insert("UUID".into(), Tag::IntArray(uuid.to_vec()));
        }
        let zombie = game.load_mob(zombie, None).unwrap();
        let saved_neutral = |kind: &str, target: [i32; 4]| {
            let mut mob = entity_at_origin(kind);
            if let Tag::Compound(ref mut tag) = mob {
                // Vanilla reads the target even when the anger timer expired.
                tag.insert("angry_at".into(), Tag::IntArray(target.to_vec()));
                tag.insert("anger_end_time".into(), Tag::Long(-1));
            }
            mob
        };
        let live = game
            .load_mob(saved_neutral("minecraft:iron_golem", uuid), None)
            .unwrap();
        let absent = game
            .load_mob(saved_neutral("minecraft:iron_golem", [274, 0, 0, 2]), None)
            .unwrap();
        let enderman = game
            .load_mob(saved_neutral("minecraft:enderman", uuid), None)
            .unwrap();
        assert_eq!(
            game.mobs
                .iter()
                .find(|mob| mob.id == live)
                .unwrap()
                .ai
                .target,
            Some(sunrise_sim::mobs::Who::Mob(zombie))
        );
        assert_eq!(
            game.mobs
                .iter()
                .find(|mob| mob.id == absent)
                .unwrap()
                .ai
                .target,
            None,
            "an offline UUID must never become a different player"
        );
        let enderman_state = game.mobs.iter().find(|mob| mob.id == enderman).unwrap();
        assert_eq!(
            enderman_state.ai.target,
            Some(sunrise_sim::mobs::Who::Mob(zombie))
        );
        assert!(
            enderman_state.extra.flag,
            "Enderman.setTarget must open its mouth"
        );
        if let Some(observed) = observed {
            assert_eq!(
                *observed.lock().unwrap(),
                [
                    (live, Some(zombie)),
                    (absent, None),
                    (enderman, Some(zombie))
                ]
            );
        }
    }

    super::super::native_entity_refs::tests::with_resident_game(|game| check(game, None));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "observe",
        Box::new(Observe(observed.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            check(game, Some(&observed))
        });
    });
}

#[test]
#[ignore = "daily"]
fn resident_other_and_fangs_load_tick_and_reset() {
    super::super::native_entity_refs::tests::with_game(|game| {
        game.server.others.0.clear();
        game.fangs.list.clear();
        let tag = |kind| {
            let mut tag = entity(kind);
            if let Tag::Compound(c) = &mut tag {
                c.insert(
                    "Pos".into(),
                    Tag::List(vec![
                        Tag::Double(0.0_f64.to_bits()),
                        Tag::Double(80.0_f64.to_bits()),
                        Tag::Double(0.0_f64.to_bits()),
                    ]),
                );
                c.insert("Warmup".into(), Tag::Int(40));
            }
            tag
        };
        let id = game
            .load_other(tag("minecraft:marker"), "minecraft:marker")
            .unwrap();
        assert_eq!(game.server.others.0[0].id, id);
        let snapshot = game.server.others.0.to_vec();
        game.server.others.0.update(0, |o| o.yaw = 90.0);
        assert_eq!(snapshot[0].yaw, 0.0);
        game.load_unlisted(
            (0, 0),
            "minecraft:evoker_fangs",
            None,
            tag("minecraft:evoker_fangs"),
        )
        .unwrap();
        let survivor = game.fangs.list[0].id;
        game.insert_fangs(survivor + 1, [0.0, 80.0, 0.0], 0.0, -30, 0);
        game.fangs.list.update(1, |f| {
            f.life = 0;
            f.started = true;
        });
        game.tick_fangs();
        assert_eq!(game.fangs.list.len(), 1);
        assert_eq!(game.fangs.list[0].id, survivor);
        assert_eq!(game.fangs.list[0].warmup, 39);
        game.server.others.0.clear();
        game.fangs.list.clear();
        assert!(game.server.others.0.is_empty() && game.fangs.list.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn comparator_and_hopper_live_state_survives_chunk_reload() {
    run_comparator_and_hopper_live_state_survives_chunk_reload();
}

pub(crate) fn run_comparator_and_hopper_live_state_survives_chunk_reload() {
    use crate::game::load_resources;
    use crate::settings::Settings;
    use sunrise_gameplay_common::game::{Options, Progress};
    use sunrise_items::items::ItemStack;
    use sunrise_level::containers::BlockEntity;
    use sunrise_sim::save;
    fn cooldown(game: &sunrise_gameplay::game::ServerGame<'_>) -> i32 {
        match game
            .block_entity_nbt([3, 64, 3])
            .unwrap()
            .get("TransferCooldown")
        {
            Some(Tag::Int(value)) => *value,
            _ => -1,
        }
    }
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!("no menu handoff"),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.headless = true;
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    let comparator = BlockPos::new(1, 64, 1);
    let hopper = (3, 64, 3);
    let barrel = (3, 63, 3);
    let mut chunk = Chunk::empty(0, 0);
    for (p, name) in [
        ((1, 64, 1), "minecraft:comparator"),
        (hopper, "minecraft:hopper"),
        (barrel, "minecraft:barrel"),
        ((1, 64, 0), "minecraft:redstone_block"),
        ((1, 63, 1), "minecraft:stone"),
    ] {
        chunk.set(
            p.0,
            p.1,
            p.2,
            game.pack.assets.registry.default_state(name).unwrap().0,
        );
    }
    let raw = |id: &str, p: (i32, i32, i32), key: &str, value: i32| {
        Tag::compound([
            ("id", Tag::String(id.into())),
            ("x", Tag::Int(p.0)),
            ("y", Tag::Int(p.1)),
            ("z", Tag::Int(p.2)),
            (key, Tag::Int(value)),
            (
                "components",
                Tag::compound([(
                    "minecraft:custom_data",
                    Tag::compound([("mark", Tag::String("preserved".into()))]),
                )]),
            ),
        ])
    };
    let mut extras = ChunkExtras::default();
    extras
        .block_entities
        .push(raw("minecraft:comparator", (1, 64, 1), "OutputSignal", 3));
    extras.block_entity_sources.push((
        hopper,
        raw("minecraft:hopper", hopper, "TransferCooldown", 22),
    ));
    chunk.saved = Some(Arc::new(extras));
    game.world.chunks.insert((0, 0), Arc::new(chunk));
    game.world.block_entities.insert(
        hopper,
        BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:stone", 64)); 5]),
    );
    game.world
        .block_entities
        .insert(barrel, BlockEntity::Chest(vec![None; 27]));
    let mut trace = Vec::new();
    for (index, value) in [
        None,
        Some(Tag::Byte(-1)),
        Some(Tag::Short(23)),
        Some(Tag::Int(i32::MIN)),
        Some(Tag::Long(4_294_967_297)),
        Some(Tag::Float((-1.5f32).to_bits())),
        Some(Tag::Double(15.75f64.to_bits())),
        Some(Tag::String("wrong".into())),
    ]
    .into_iter()
    .enumerate()
    {
        let mut extra = ChunkExtras::default();
        let mut c = raw("minecraft:comparator", (1, 64, 1), "OutputSignal", 0);
        let mut h = raw("minecraft:hopper", hopper, "TransferCooldown", -1);
        for (tag, field) in [(&mut c, "OutputSignal"), (&mut h, "TransferCooldown")] {
            if let Tag::Compound(map) = tag {
                match &value {
                    Some(v) => {
                        map.insert(field.into(), v.clone());
                    }
                    None => {
                        map.remove(&field.into());
                    }
                }
            }
        }
        extra.block_entities.push(c);
        extra.block_entity_sources.push((hopper, h));
        let old = Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap())
            .saved
            .replace(Arc::new(extra));
        game.register_chunk_ticks((0, 0));
        trace.push(format!(
            "integer {index} {} {}",
            game.block_ticks
                .comparator_outputs
                .get(&comparator)
                .copied()
                .unwrap_or(0),
            cooldown(&game)
        ));
        Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).saved = old;
    }
    game.sync_tick_chunks();
    assert_eq!(
        game.block_ticks
            .comparator_outputs
            .get(&comparator)
            .copied()
            .unwrap_or(0),
        3,
        "restored comparator output"
    );
    assert_eq!(cooldown(&game), 22);
    // A real circuit update and transfer change both values since load.
    game.with_level(|b, level, ticks| b.scheduled_block_tick(level, ticks, comparator));
    assert_eq!(game.block_ticks.comparator_outputs[&comparator], 15);
    game.build_registries();
    game.set_allow_commands(true);
    assert!(game.run_command_quietly("data merge block 3 64 3 {TransferCooldown:0}"));
    for _ in 0..5 {
        game.world.game_time += 1;
        game.tick_hopper(hopper);
    }
    assert_eq!(cooldown(&game), 4);
    game.world
        .block_entities
        .insert(barrel, BlockEntity::Chest(vec![None; 27]));
    let batch = game.with_entities(game.world.chunks.values());
    let encoded = save::encode(
        &batch[0],
        game.pack.assets.registry,
        world_biomes!(game),
        &Default::default(),
    );
    // Streaming removes the resident chunk before preparing its save.
    let resident = game.world.chunks.remove(&(0, 0)).unwrap();
    let unloaded = game.with_entities([&resident]);
    assert_eq!(
        save::encode(
            &unloaded[0],
            game.pack.assets.registry,
            world_biomes!(game),
            game.res.items()
        )
        .tag,
        encoded.tag
    );
    game.world.chunks.insert((0, 0), resident);
    let Some(Tag::List(tags)) = encoded.tag.get("block_entities") else {
        panic!("missing entities")
    };
    for (id, key, expected) in [
        ("minecraft:comparator", "OutputSignal", 15),
        ("minecraft:hopper", "TransferCooldown", 4),
    ] {
        let tag = tags
            .iter()
            .find(|t| text(t.get("id")).as_deref() == Some(id))
            .unwrap();
        assert_eq!(tag.get(key), Some(&Tag::Int(expected)), "{id} live save");
        assert_eq!(
            text(
                tag.get("components")
                    .unwrap()
                    .get("minecraft:custom_data")
                    .unwrap()
                    .get("mark")
            )
            .as_deref(),
            Some("preserved")
        );
    }
    // Snapshotting must not rewrite either the source chunk or its raw NBT.
    assert_eq!(
        game.world.chunks[&(0, 0)]
            .saved
            .as_ref()
            .unwrap()
            .block_entity_sources[0]
            .1
            .get("TransferCooldown"),
        Some(&Tag::Int(22))
    );
    let restored = save::decode(&encoded, game.pack.assets.registry, world_biomes!(game)).unwrap();
    game.world.block_entities.clear();
    for (p, entity) in &restored.block_entities {
        game.world.block_entities.insert(*p, entity.clone());
    }
    game.world.chunks.insert((0, 0), Arc::new(restored));
    game.server.hoppers.clear();
    game.block_ticks = sunrise_world::level::Ticks::new(0);
    game.server.tick_chunks.clear();
    // Frozen: data must already show the saved values before any tick.
    assert_eq!(
        game.block_entity_nbt([1, 64, 1])
            .unwrap()
            .get("OutputSignal"),
        Some(&Tag::Int(15))
    );
    assert_eq!(
        game.block_entity_nbt([3, 64, 3])
            .unwrap()
            .get("TransferCooldown"),
        Some(&Tag::Int(4))
    );
    game.sync_tick_chunks();
    println!("persistence state 15 {} 0", cooldown(&game));
    trace.push("state 15 4 0".to_owned());
    for tick in 1..=6 {
        game.world.game_time += 1;
        game.tick_hopper(hopper);
        let BlockEntity::Chest(items) = game.world.block_entities.get(&barrel).unwrap() else {
            panic!("barrel")
        };
        let count: u32 = items.iter().flatten().map(|s| s.count).sum();
        assert_eq!(count, u32::from(tick >= 4), "transfer tick {tick}");
        let line = format!("tick {tick} {} {count}", cooldown(&game));
        println!("persistence {line}");
        trace.push(line);
    }
    if let Some(path) = std::env::var_os("SUNRISE_PERSISTENCE_ORACLE") {
        assert_eq!(
            trace.join("\n"),
            std::fs::read_to_string(path).unwrap().trim()
        );
    }
    game.world.chunks.clear();
    game.sync_tick_chunks();
    assert!(!game.server.hoppers.contains_key(&hopper));
    assert!(
        !game
            .block_ticks
            .comparator_outputs
            .contains_key(&comparator)
    );
}

const MOUNT: [i32; 4] = [1, 2, 3, 4];

#[test]
#[ignore = "daily"]
fn deferred_zombie_finalization_keeps_unknown_profession_through_save_and_cure() {
    super::super::native_entity_refs::tests::with_game(|game| {
        let mut tag = entity("minecraft:zombie_villager");
        if let Tag::Compound(c) = &mut tag {
            c.insert("sunrise:finalize".into(), Tag::Byte(1));
            c.insert(
                "Pos".into(),
                Tag::List(vec![
                    Tag::Double(0.0_f64.to_bits()),
                    Tag::Double(64.0_f64.to_bits()),
                    Tag::Double(0.0_f64.to_bits()),
                ]),
            );
            c.insert(
                "VillagerData".into(),
                Tag::compound([
                    ("type", Tag::String("minecraft:plains".into())),
                    ("profession", Tag::String("example:engineer".into())),
                    ("level", Tag::Int(1)),
                ]),
            );
        }
        let id = game.load_mob(tag, None).unwrap();
        let zombie = game.mobs.iter().find(|m| m.id == id).unwrap();
        assert_eq!(zombie.extra.profession.id(), "example:engineer");
        let saved = entity_nbt::mob_to_nbt(
            zombie,
            [0; 4],
            None,
            Some(game.pack.assets.registry),
            |_| None,
            &Default::default(),
        );
        let restored = entity_nbt::mob_from_nbt(
            &saved,
            game.res.mob_types(),
            id,
            Some(game.pack.assets.registry),
        )
        .unwrap();
        let mut cured = sunrise_sim::mobs::Mob::new(
            id + 1,
            game.res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone(),
            restored.position,
            0.0,
        );
        sunrise_gameplay::game::server_side::zombie_villagers::convert_to_villager(
            &restored, &mut cured, false,
        );
        assert_eq!(cured.extra.profession.id(), "example:engineer");
    });
}

#[test]
fn a_saved_team_is_joined_only_when_the_scoreboard_has_it() {
    let mut board = Scoreboard::default();
    board.add_team("red");
    let with_team = |team: &str| {
        let mut t = entity("minecraft:zombie");
        if let Tag::Compound(c) = &mut t {
            c.insert("Team".into(), Tag::String(team.into()));
        }
        t
    };
    let name = "00000001-0002-0003-0004-000000000005";
    assert!(join_saved_team(&mut board, name, &with_team("red")));
    assert_eq!(
        board.players_team(name).map(|t| t.name.as_str()),
        Some("red")
    );
    // A team the scoreboard doesn't have is ignored, and the entity
    // stays on its team.
    assert!(!join_saved_team(&mut board, name, &with_team("blue")));
    assert_eq!(
        board.players_team(name).map(|t| t.name.as_str()),
        Some("red")
    );
    assert!(!join_saved_team(
        &mut board,
        "other",
        &entity("minecraft:zombie")
    ));
    assert!(board.players_team("other").is_none());
}

#[test]
fn only_mobs_and_boats_keep_a_leash_tag() {
    let with_leash = |kind: &str| {
        let mut t = entity(kind);
        if let Tag::Compound(c) = &mut t {
            c.insert("leash".into(), Tag::IntArray(vec![6, 100, 0]));
        }
        t
    };
    let has = |t: &Tag| t.get("leash").is_some();
    for kind in [
        "minecraft:armor_stand",
        "minecraft:minecart",
        "minecraft:tnt",
        "minecraft:item",
    ] {
        let mut t = with_leash(kind);
        drop_unleashable_leash(&mut t, kind, false);
        assert!(!has(&t), "{kind}");
    }
    let mut cow = with_leash("minecraft:cow");
    drop_unleashable_leash(&mut cow, "minecraft:cow", true);
    assert!(has(&cow));
    let mut boat = with_leash("minecraft:oak_boat");
    drop_unleashable_leash(&mut boat, "minecraft:oak_boat", false);
    assert!(has(&boat));
}

fn entity(kind: &str) -> Tag {
    Tag::compound([("id", Tag::String(kind.into()))])
}
fn passengers(tag: &Tag) -> Vec<String> {
    match tag.get("Passengers") {
        Some(Tag::List(l)) => l.iter().filter_map(entity_nbt::kind).collect(),
        _ => Vec::new(),
    }
}

#[test]
fn entity_attachment_boundary_does_not_alias_equal_uuid_passengers() {
    use sunrise_mod_runtime::attachments::{
        AttachmentCodec, AttachmentError, AttachmentType, Attachments, Holder,
    };
    let key = AttachmentType {
        namespace: "test",
        name: "counter",
        default: || 0,
        codec: Some(AttachmentCodec {
            encode: |n: &i32| Ok([("count".into(), Tag::Int(*n))].into()),
            decode: |c| match c.get(&"count".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
        copy_on_death: false,
        sync_codec: None,
    };
    let root_id = [1, 2, 3, 4];
    let child_id = root_id;
    let mut data = Attachments::default();
    key.set(&mut data, Holder::Level, 29).unwrap();
    let mut maps = sunrise_gameplay_common::game::native_collider_state::EntityMaps::default();
    maps.insert(12, data);
    let mut root = Tag::compound([
        ("UUID", Tag::IntArray(root_id.to_vec())),
        (
            "Passengers",
            Tag::List(vec![Tag::compound([(
                "UUID",
                Tag::IntArray(child_id.to_vec()),
            )])]),
        ),
    ]);
    write_attachments(&mut root, 12, &maps).unwrap();
    let Some(Tag::List(children)) = root.get("Passengers") else {
        panic!("passenger missing");
    };
    assert!(children[0].get(ATTACHMENTS_KEY).is_none());
    let mut loaded = HashMap::new();
    read_attachments(&root, 12, &mut loaded, false);
    assert_eq!(key.get(loaded.get_mut(&12).unwrap(), Holder::Level), Ok(29));
}

/// `remember_uuid`'s read: a map an `entity_constructing` listener made just now takes the
/// saved values over its own (`Entity.load` after the constructor), a live map that came
/// with the entity keeps its values, and no map at all is the saved one.
#[test]
fn saved_attachments_read_over_a_constructed_map_but_not_a_live_one() {
    use sunrise_mod_runtime::attachments::{
        AttachmentCodec, AttachmentError, AttachmentType, Attachments, Holder,
    };
    fn key(name: &'static str) -> AttachmentType<i32> {
        AttachmentType {
            namespace: "test",
            name,
            default: || 0,
            codec: Some(AttachmentCodec {
                encode: |n: &i32| Ok([("count".into(), Tag::Int(*n))].into()),
                decode: |c| match c.get(&"count".into()) {
                    Some(Tag::Int(n)) => Ok(*n),
                    _ => Err(AttachmentError::Decode),
                },
            }),
            copy_on_death: false,
            sync_codec: None,
        }
    }
    let (saved, unsaved) = (key("saved"), key("unsaved"));
    let uuid = [1, 2, 3, 4];
    let mut data = Attachments::default();
    saved.set(&mut data, Holder::Level, 7).unwrap();
    let tag = Tag::compound([
        ("UUID", Tag::IntArray(uuid.to_vec())),
        (ATTACHMENTS_KEY, Tag::Compound(data.try_to_nbt().unwrap())),
    ]);
    let listener = || {
        let mut map = Attachments::default();
        saved.set(&mut map, Holder::Level, 41).unwrap();
        unsaved.set(&mut map, Holder::Level, 41).unwrap();
        HashMap::from([(12, map)])
    };

    let mut constructed = listener();
    read_attachments(&tag, 12, &mut constructed, false);
    let map = constructed.get_mut(&12).unwrap();
    assert_eq!(saved.get(map, Holder::Level), Ok(7));
    assert_eq!(unsaved.get(map, Holder::Level), Ok(41));

    let mut live = listener();
    read_attachments(&tag, 12, &mut live, true);
    assert_eq!(saved.get(live.get_mut(&12).unwrap(), Holder::Level), Ok(41));

    let mut none = HashMap::new();
    read_attachments(&tag, 12, &mut none, false);
    let map = none.get_mut(&12).unwrap();
    assert_eq!(saved.get(map, Holder::Level), Ok(7));
    assert_eq!(unsaved.get(map, Holder::Level), Ok(0));
}

/// Entities as a save before every entity held attachments wrote them (a vanilla compound
/// with its `UUID`, and `sunrise:attachments` where the mod set one), loaded and saved again:
/// each comes back on the entity with its UUID, and its attachments with it.
#[test]
#[ignore = "daily"]
fn an_older_save_loads_its_entities_attachments_back_onto_them() {
    use crate::game::load_resources;
    use crate::settings::Settings;
    use sunrise_gameplay_common::game::{Options, Progress};
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!("no menu handoff"),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    use sunrise_mod_runtime::attachments::{AttachmentCodec, AttachmentError, AttachmentType};
    static MARK: AttachmentType<i32> = AttachmentType {
        namespace: "holdproof",
        name: "mark",
        default: || 0,
        codec: Some(AttachmentCodec {
            encode: |n: &i32| Ok([("n".into(), Tag::Int(*n))].into()),
            decode: |c| match c.get(&"n".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
        copy_on_death: false,
        sync_codec: None,
    };
    let mark = |n: i32| {
        Tag::compound([(
            "holdproof",
            Tag::compound([(
                "mark",
                Tag::compound([("value", Tag::compound([("n", Tag::Int(n))]))]),
            )]),
        )])
    };
    let pos = |x: f64| Tag::List([x, 70.0, 3.0].map(|v| Tag::Double(v.to_bits())).to_vec());
    let kinds = [
        ("minecraft:zombie", vec![]),
        ("minecraft:arrow", vec![("NoGravity", Tag::Byte(1))]),
        ("minecraft:experience_orb", vec![("Value", Tag::Short(7))]),
        (
            "minecraft:falling_block",
            vec![
                (
                    "BlockState",
                    Tag::compound([("Name", Tag::String("minecraft:sand".into()))]),
                ),
                ("NoGravity", Tag::Byte(1)),
                ("Time", Tag::Int(1)),
            ],
        ),
        ("minecraft:snowball", vec![("NoGravity", Tag::Byte(1))]),
    ];
    let mut tags = Vec::new();
    let mut uuids = Vec::new();
    for (i, (kind, extra)) in kinds.iter().enumerate() {
        let n = i as i32 + 1;
        let uuid = [n, 2 * n, 3 * n, 4 * n];
        uuids.push((*kind, uuid, 1000 + n));
        let mut fields: Vec<(&str, Tag)> = vec![
            ("id", Tag::String((*kind).into())),
            ("Pos", pos(1.0 + i as f64)),
            ("UUID", Tag::IntArray(uuid.to_vec())),
            (ATTACHMENTS_KEY, mark(1000 + n)),
        ];
        fields.extend(extra.iter().cloned());
        tags.push(Tag::compound(fields));
    }
    // The level is a server's, whose attachments a mod reaches.
    game.options.dedicated = Some(Vec::new());
    game.world.queue_entity_nbt((0, 0), tags);
    game.spawn_loaded_entities();
    let saved = game.entity_chunks(&HashSet::from([(0, 0)]), false);
    let saved: Vec<&Tag> = saved.iter().flat_map(|(_, tags)| tags).collect();
    for &(kind, uuid, n) in &uuids {
        let id = game
            .persist
            .uuids
            .iter()
            .find(|(_, u)| **u == uuid)
            .map(|(id, _)| *id);
        let id = id.unwrap_or_else(|| panic!("{kind}: loaded without its UUID"));
        let map = game.attachment_map_quiet(
            mod_api::Holder::Entity(id),
            mod_api::AttachmentAccess::Existing,
        );
        let map = map.unwrap_or_else(|e| panic!("{kind}: not a holder: {e:?}"));
        assert_eq!(
            MARK.get(map, mod_api::Holder::Level),
            Ok(n),
            "{kind}: attachments not loaded"
        );
        let tag = saved.iter().find(|t| entity_nbt::uuid(t) == Some(uuid));
        let tag = tag.unwrap_or_else(|| panic!("{kind}: saved without its UUID"));
        assert_eq!(
            tag.get(ATTACHMENTS_KEY),
            Some(&mark(n)),
            "{kind}: attachments not saved back"
        );
        println!("{kind}: id {id}, UUID kept, attachments loaded and saved back");
    }
    // Pruning drops the attachments of an entity that left the level and keeps every
    // live one's, whatever list it lives in.
    let (_, zombie, _) = uuids[0];
    let zombie_id = game
        .persist
        .uuids
        .iter()
        .find(|(_, u)| **u == zombie)
        .map(|(id, _)| *id);
    game.mobs.retain(|mob| Some(mob.id) != zombie_id);
    game.prune_entity_attachments();
    for (kind, uuid, _) in uuids {
        let kept = game
            .persist
            .uuids
            .iter()
            .any(|(id, u)| *u == uuid && game.entity_attachments.contains_key(id));
        assert_eq!(kept, uuid != zombie, "{kind}: pruned wrongly");
    }
}

/// A lingering potion's cloud is an entity of its chunk: unloading the chunk writes it and
/// takes it out of the level, so loading the chunk again gives back one cloud, not two.
#[test]
#[ignore = "daily"]
fn a_lingering_cloud_leaves_with_its_chunk() {
    use crate::game::load_resources;
    use crate::settings::Settings;
    use sunrise_gameplay_common::game::{Options, Progress};
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!("no menu handoff"),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    let cloud = Tag::compound([
        ("id", Tag::String("minecraft:area_effect_cloud".into())),
        (
            "Pos",
            Tag::List(vec![
                Tag::Double(4.5f64.to_bits()),
                Tag::Double(64.0f64.to_bits()),
                Tag::Double(4.5f64.to_bits()),
            ]),
        ),
        ("UUID", Tag::IntArray(vec![5, 6, 7, 8])),
        ("Duration", Tag::Int(600)),
        (
            "potion_contents",
            Tag::compound([("potion", Tag::String("minecraft:poison".into()))]),
        ),
    ]);
    let clouds = |game: &sunrise_gameplay::game::ServerGame<'_>| {
        game.breath_clouds
            .iter()
            .filter(|c| c.potion.is_some())
            .count()
    };
    game.world.queue_entity_nbt((0, 0), vec![cloud]);
    game.spawn_loaded_entities();
    assert_eq!(clouds(&game), 1, "the cloud loads");
    let unloaded = game.entity_chunks(&HashSet::from([(0, 0)]), true);
    let written: Vec<Tag> = unloaded.into_iter().flat_map(|(_, tags)| tags).collect();
    assert_eq!(written.len(), 1, "the unload writes the cloud");
    assert_eq!(clouds(&game), 0, "and takes it out of the level");
    game.world.queue_entity_nbt((0, 0), written);
    game.spawn_loaded_entities();
    assert_eq!(clouds(&game), 1, "loading the chunk again gives one cloud");
}

#[test]
fn a_lone_rider_saves_the_horse() {
    let horse = 7;
    assert_eq!(parent_vehicle_root(horse, &[], |_| None), Some(horse));
    let tag = root_vehicle_tag(MOUNT, entity("minecraft:horse"));
    assert_eq!(tag.get("Attach"), Some(&Tag::IntArray(MOUNT.to_vec())));
    let saved = tag.get("Entity").and_then(entity_nbt::kind);
    assert_eq!(saved.as_deref(), Some("minecraft:horse"));
    assert_eq!(vehicle_tree(horse, &[]), vec![horse]);
}

#[test]
fn two_players_on_one_boat_save_nothing() {
    let boat = 3;
    assert_eq!(parent_vehicle_root(boat, &[boat], |_| None), None);
    // A player on a mob in the same boat shares its tree too.
    let pig = 4;
    let of = |id| (id == pig).then_some(boat);
    assert_eq!(parent_vehicle_root(boat, &[pig], of), None);
    assert_eq!(parent_vehicle_root(boat, &[9], of), Some(boat));
}

#[test]
fn a_mob_in_a_boat_rides_under_the_boat() {
    let (boat, pig) = (3, 4);
    let of = |id| (id == pig).then_some(boat);
    assert_eq!(parent_vehicle_root(pig, &[], of), Some(boat));
    assert_eq!(vehicle_tree(boat, &[(pig, Some(boat))]), vec![boat, pig]);
    let mut tag = entity("minecraft:oak_boat");
    add_passengers(&mut tag, vec![entity("minecraft:pig")]);
    assert_eq!(passengers(&tag), ["minecraft:pig"]);
}

#[test]
fn a_cart_saves_and_takes_its_mob() {
    let (cart, zombie, chicken) = (5, 6, 8);
    // A chicken on a zombie in the cart: the cart is the root.
    let mounts = [(zombie, Some(cart)), (chicken, Some(zombie)), (9, None)];
    let of = |id| mounts.iter().find(|m| m.0 == id).and_then(|m| m.1);
    assert_eq!(parent_vehicle_root(chicken, &[], of), Some(cart));
    assert_eq!(vehicle_tree(cart, &mounts), vec![cart, zombie, chicken]);
    let mut tag = entity("minecraft:minecart");
    add_passengers(&mut tag, vec![entity("minecraft:zombie")]);
    assert_eq!(passengers(&tag), ["minecraft:zombie"]);
    // No rider leaves no empty list.
    let mut tag = entity("minecraft:minecart");
    add_passengers(&mut tag, Vec::new());
    assert!(tag.get("Passengers").is_none());
}

/// A portal arrival or a `/summon` is told as a fresh add, and the mark
/// never reaches the entity's kept or saved compound.
#[test]
fn a_fresh_add_is_told_apart_from_a_load_and_unmarked() {
    let mut loaded = Tag::Compound(Default::default());
    assert!(!take_fresh(&mut loaded));
    let mut fresh = Tag::Compound(Default::default());
    mark_fresh(&mut fresh);
    assert!(take_fresh(&mut fresh));
    assert!(!take_fresh(&mut fresh));
    assert_eq!(fresh, Tag::Compound(Default::default()));
}

/// `NeutralMob.addPersistentAngerSaveData` names the mob the anger is at,
/// also one never saved yet (a mob gets its UUID when it is first saved).
#[test]
#[ignore = "daily"]
fn anger_at_a_mob_not_saved_yet_names_it() {
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    let types = game.res.mob_types();
    let golem = sunrise_sim::mobs::Mob::new(
        1,
        types.get("minecraft:iron_golem").unwrap().clone(),
        Vec3::ZERO,
        0.0,
    );
    let zombie = sunrise_sim::mobs::Mob::new(
        2,
        types.get("minecraft:zombie").unwrap().clone(),
        Vec3::new(3.0, 0.0, 0.0),
        0.0,
    );
    game.mobs.push(golem);
    game.mobs.push(zombie);
    game.mobs[0].extra.anger = 400;
    game.mobs[0].extra.angry_at = Some(sunrise_sim::mobs::Who::Mob(2));
    assert_eq!(game.persist.uuid_of(2), None);
    let tag = game.mob_tree_nbt(0);
    let Some(Tag::IntArray(named)) = tag.get("angry_at") else {
        panic!("no angry_at in {tag:?}");
    };
    assert_eq!(
        Some(named.as_slice()),
        game.persist.uuid_of(2).as_ref().map(|u| &u[..])
    );
}

/// `Entity.saveWithoutId` writes an experience orb's live `remainingFireTicks`
/// and `airSupply`: an orb loaded with `Fire:40s` that burned for a tick saves
/// `39s`, not the loaded `40s`, while a field it doesn't simulate
/// (`Invulnerable`) still comes from the loaded compound.
#[test]
fn an_orb_saves_its_live_fire_and_air_over_the_loaded_ones() {
    let raw =
        sunrise_core::nbt::parse_snbt("{Pos:[4.5d,100d,4.5d],Fire:40s,Air:300s,Invulnerable:1b}")
            .unwrap()
            .0;
    let mut orb = entity_nbt::orbs_from_nbt(&raw).remove(0);
    (orb.fire_ticks, orb.air) = (39, 299);
    let saved = with_raw_over(entity_nbt::orb_to_nbt(&orb, [0; 4]), Some(&raw), ORB_LIVE);
    assert_eq!(saved.get("Fire"), Some(&Tag::Short(39)));
    assert_eq!(saved.get("Air"), Some(&Tag::Short(299)));
    assert_eq!(saved.get("Invulnerable"), Some(&Tag::Byte(1)));
}

/// `Entity.saveWithoutId` writes the live `portalCooldown` that
/// `processPortalCooldown` counts down, in the chunk save and in `/data get`,
/// with the live `remainingFireTicks` and `airSupply` over the loaded ones
/// (an orb's `Air` is only stored: nothing ticks it). One real tick — the
/// orb's ordered tick, then the entity portal pass — turns a loaded
/// `Fire:40s` and `PortalCooldown:20` into 39 and 19, the chunk reload reads
/// them back, and `/data merge entity` reads `Fire` and `Air` into the live
/// orb while an unrelated key's merge leaves them.
#[test]
#[ignore = "daily"]
fn an_orb_saves_its_live_portal_cooldown() {
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::server_side::persist::tests::an_orb_saves_its_live_portal_cooldown",
        "SUNRISE_ORB_PERSISTENCE_CHILD",
        &["orb"],
    ) {
        return;
    }
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
            .unwrap();
        game.load_orb(
            sunrise_core::nbt::parse_snbt(
                "{Pos:[4.5d,100d,4.5d],Value:3s,Count:1,Fire:40s,Air:299s,PortalCooldown:20}",
            )
            .unwrap()
            .0,
            Some([9, 0, 0, 1]),
        );
        let id = game.orbs[0].id;
        // One tick: the orb's own (ordered) tick, then Entity.handlePortal's
        // pass over the entities.
        game.server.ordered.announced = sunrise_sim::join_seq::JoinSeq::MAX;
        game.server.tickets.0.update(None);
        game.begin_ordered_ticks();
        game.run_ordered_ticks(sunrise_sim::join_seq::JoinSeq::MAX, false)
            .unwrap();
        game.handle_entity_portals();
        assert_eq!(game.orbs[0].fire_ticks, 39, "one tick of Fire");
        assert_eq!(game.orbs[0].air, 299, "the stored air does not tick");
        assert_eq!(
            game.server.portals.entities.get(&id).map(|p| p.cooldown),
            Some(19),
            "the tick's portal cooldown"
        );
        // The real persistence path: the chunk save takes the orb out of the
        // level, and loading the chunk again builds it from the saved
        // compound.
        let written: Vec<Tag> = game
            .entity_chunks(&HashSet::from([(0, 0)]), true)
            .into_iter()
            .flat_map(|(_, tags)| tags)
            .collect();
        assert_eq!(written.len(), 1, "the orb leaves with its chunk");
        game.world.queue_entity_nbt((0, 0), written);
        game.spawn_loaded_entities();
        assert_eq!(game.orbs[0].fire_ticks, 39, "the reload keeps the fire");
        assert_eq!(game.orbs[0].air, 299, "and the air");
        let read = game
            .command_target_nbt(sunrise_gameplay_common::game::commands_common::Target::Orb(
                game.orbs[0].id,
            ))
            .unwrap();
        assert_eq!(read.get("PortalCooldown"), Some(&Tag::Int(19)));
        assert_eq!(read.get("Fire"), Some(&Tag::Short(39)));
        assert_eq!(read.get("Air"), Some(&Tag::Short(299)));
        // `/data merge entity`: `Fire` and `Air` are read into the live orb,
        // and a merge that does not name them leaves them alone.
        game.build_registries();
        game.set_allow_commands(true);
        let sel = "00000009-0000-0000-0000-000000000001";
        assert!(
            game.run_command_quietly(&format!("data merge entity {sel} {{Fire:40s,Air:275s}}")),
            "the merge runs"
        );
        let orb = &game.orbs[0];
        assert_eq!(
            (orb.fire_ticks, orb.air),
            (40, 275),
            "the merge reached the live orb"
        );
        assert!(
            game.run_command_quietly(&format!("data merge entity {sel} {{Invulnerable:1b}}")),
            "the unrelated merge runs"
        );
        let orb = &game.orbs[0];
        assert_eq!(
            (orb.fire_ticks, orb.air),
            (40, 275),
            "an unrelated edit preserves the live fire and air"
        );
        let read = game
            .command_target_nbt(sunrise_gameplay_common::game::commands_common::Target::Orb(
                game.orbs[0].id,
            ))
            .unwrap();
        assert_eq!(read.get("Invulnerable"), Some(&Tag::Byte(1)));
        assert_eq!(read.get("Fire"), Some(&Tag::Short(40)));
        assert_eq!(read.get("Air"), Some(&Tag::Short(275)));
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
