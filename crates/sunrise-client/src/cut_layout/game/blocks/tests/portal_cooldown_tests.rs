//! Direct production handlers over an empty in-memory world; no server or renderer.
use crate::game::{Settings, load_resources};
use std::sync::Arc;
use sunrise_core::nbt::Tag;
use sunrise_core::position::BlockPos;
use sunrise_gameplay::game::server_side::nether_portal::{arrival_from_tag, arrival_tag, doubles};
use sunrise_level::chunk::Chunk;
use sunrise_sim::nether_portal::PortalState;
use {sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress};

#[test]
#[ignore = "needs explicit SUNRISE_TEST_RESOURCES and SUNRISE_PORTAL_REFERENCE"]
fn portal_contact_and_waiting_handoff_match_vanilla_counters() {
    run_portal_contact_and_waiting_handoff_match_vanilla_counters();
}

pub(crate) fn run_portal_contact_and_waiting_handoff_match_vanilla_counters() {
    let reference: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var_os("SUNRISE_PORTAL_REFERENCE").unwrap()).unwrap(),
    )
    .unwrap();
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
    options.headless = true;
    options.game_dir = Some(std::env::current_dir().unwrap());
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
    game.build_registries();
    // Lone-traveller handling resolves entity handles, as it does in an open world.
    game.native_lifecycle
        .begin_open(Some(mod_api::Authority::IntegratedServer));
    game.native_lifecycle.world_open_ready().unwrap();
    game.options.dedicated = Some(Vec::new());
    game.world = sunrise_level::world::World::new(game.res.facts().air, 1);
    for z in -2..=2 {
        for x in -2..=2 {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
        }
    }
    let entry = BlockPos::new(4, 210, 4);
    let portal = game
        .pack
        .assets
        .registry
        .default_state("minecraft:nether_portal")
        .unwrap();
    game.world.set(entry.x, entry.y, entry.z, portal).unwrap();
    let kinds = [
        "minecraft:oak_boat",
        "minecraft:minecart",
        "minecraft:arrow",
        "minecraft:pig",
    ];
    let mut ids = Vec::new();
    for (index, kind) in kinds.iter().enumerate() {
        let uuid = [11318, 10, 0, index as i32];
        let tag = Tag::compound([
            ("id", Tag::String((*kind).into())),
            ("UUID", Tag::IntArray(uuid.to_vec())),
            ("Pos", doubles([4.5, 210.0, 4.5])),
            ("NoAI", Tag::Byte(1)),
            ("NoGravity", Tag::Byte(1)),
            ("PortalCooldown", Tag::Int(5)),
        ]);
        game.load_listed(kind, Some(uuid), None, tag).unwrap();
        ids.push(game.server.next_entity);
    }
    assert_eq!(game.boats.len(), 1);
    assert_eq!(game.carts.len(), 1);
    assert_eq!(game.creatures.projectiles.len(), 1);
    assert_eq!(game.mobs.len(), 1);
    // Direct handler calls do not refresh the headless world's chunk tickets.
    // Frozen entities retain their counters across every group/phase pass.
    game.server
        .tickets
        .0
        .update(Some(sunrise_sim::ticket_levels::Sources::default()));
    game.handle_vehicle_portals();
    game.handle_entity_portals();
    game.handle_portals(false, true);
    for id in &ids {
        assert_eq!(game.server.portals.entities.get(id).unwrap().cooldown, 5);
    }
    game.server
        .tickets
        .0
        .update(Some(sunrise_sim::ticket_levels::Sources {
            forced: vec![(0, 0)],
            ..Default::default()
        }));
    assert!(game.entity_ticking_area()(entry.x, entry.z));
    let mut runtime = Vec::new();
    for (step, ticks) in [1, 3, 1, 1, 8].into_iter().enumerate() {
        if step == 2 {
            game.common
                .world
                .set(entry.x, entry.y, entry.z, game.common.res.facts().air)
                .unwrap();
        }
        for _ in 0..ticks {
            game.handle_vehicle_portals();
            game.handle_entity_portals();
            game.find_entity_portals();
        }
        runtime.push(
            ids.iter()
                .map(|id| {
                    game.server
                        .portals
                        .entities
                        .get(id)
                        .map_or(0, |state| state.cooldown)
                })
                .collect::<Vec<_>>(),
        );
    }
    // The actual departure handler emits a waiting compound before any destination is resolved.
    let end = game
        .pack
        .assets
        .registry
        .default_state("minecraft:end_portal")
        .unwrap();
    game.world.set(entry.x, entry.y, entry.z, end).unwrap();
    for id in &ids {
        game.server
            .portals
            .entities
            .insert(*id, PortalState::default());
    }
    game.find_entity_portals();
    game.handle_vehicle_portals();
    game.handle_entity_portals();
    assert_eq!(
        game.server.portals.arrivals.len(),
        4,
        "all four production travellers departed"
    );
    let mut handoff = Vec::new();
    for kind in kinds {
        let arrival = game
            .server
            .portals
            .arrivals
            .iter()
            .find(|a| sunrise_sim::save::text(a.tag.get("id")).as_deref() == Some(kind))
            .unwrap();
        let restored = arrival_from_tag(&arrival_tag(arrival)).unwrap();
        let counter = counter(&restored.tag);
        game.load_listed(kind, None, None, restored.tag).unwrap();
        let id = game.server.next_entity;
        assert_eq!(
            game.server
                .portals
                .entities
                .get(&id)
                .map_or(0, |s| s.cooldown),
            counter
        );
        handoff.push(counter);
    }
    let report = serde_json::json!({"runtime":runtime,"handoff":handoff});
    if let Some(path) = std::env::var_os("SUNRISE_PORTAL_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    for (field, expected) in [("runtime", "contact"), ("handoff", "delays")] {
        assert_eq!(report[field], reference[expected], "{field}");
    }
}

fn counter(tag: &Tag) -> i32 {
    sunrise_sim::save::number(tag.get("PortalCooldown")).unwrap() as i32
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
