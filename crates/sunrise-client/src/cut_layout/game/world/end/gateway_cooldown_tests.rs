//! Production gateway counters and direct phases over a silent, empty Game.
use sunrise_core::{geometry::Vec3, position::BlockPos};
use sunrise_sim::{
    end_gateway::Traveller,
    projectiles::{Projectile, ProjectileKind},
};
use {
    sunrise_gameplay_common::game::boats_common::Boat,
    sunrise_gameplay_common::game::minecarts::CartKind,
    sunrise_gameplay_common::game::minecarts::Minecart,
};

#[test]
#[ignore = "needs explicit SUNRISE_TEST_RESOURCES and SUNRISE_PORTAL_REFERENCE"]
fn gateway_counters_match_actual_vanilla_methods() {
    run_gateway_counters_match_actual_vanilla_methods();
}

pub(crate) fn run_gateway_counters_match_actual_vanilla_methods() {
    let reference: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var_os("SUNRISE_PORTAL_REFERENCE").unwrap()).unwrap(),
    )
    .unwrap();
    let boat = Boat::new(1, "oak".into(), Vec3::ZERO, 0.0);
    let cart = Minecart::new(2, CartKind::Rideable, Vec3::ZERO);
    let arrow = Projectile::new(
        ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        },
        Vec3::ZERO,
        None,
    );
    let mut delays = [0; 4];
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.boats.push(boat);
        game.carts.push(cart);
        let mut arrow = arrow;
        arrow.id = 3;
        game.creatures.projectiles.push(arrow);
        delays = [
            game.dimension_changing_delay(1),
            game.dimension_changing_delay(2),
            game.dimension_changing_delay(3),
            game.dimension_changing_delay(999),
        ];
    });
    assert_eq!(serde_json::json!(delays), reference["delays"]);
    let mut travellers: [Traveller; 4] = std::array::from_fn(|_| Traveller::default());
    let entry = BlockPos::new(0, 100, 0);
    for (traveller, delay) in travellers.iter_mut().zip(delays) {
        traveller.enter_with_delay(entry, delay);
        assert_eq!(traveller.travel_with_delay(true, delay), Some(entry));
        assert_eq!(traveller.cooldown(), delay);
        traveller.set_cooldown(5);
    }
    let mut rows = Vec::new();
    for (step, ticks) in [1, 3, 1, 1, 8].into_iter().enumerate() {
        for _ in 0..ticks {
            for (i, (traveller, delay)) in travellers.iter_mut().zip(delays).enumerate() {
                if step < 2 && i == 2 {
                    traveller.enter_with_delay(entry, delay);
                }
                traveller.travel_with_delay(true, delay);
                if step < 2 && i != 2 {
                    traveller.enter_with_delay(entry, delay);
                }
            }
        }
        rows.push(
            travellers
                .iter()
                .map(Traveller::cooldown)
                .collect::<Vec<_>>(),
        );
    }
    let phases = game_gateway_phases();
    let report = serde_json::json!({"contact":rows,"phases":phases});
    assert_eq!(report["contact"], reference["contact"]);
    if let Some(path) = std::env::var_os("SUNRISE_PORTAL_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}

// Exercise the real dispatch with cooldown-only states, so no teleport destination is resolved.
fn game_gateway_phases() -> [[i32; 2]; 4] {
    use std::sync::Arc;
    use sunrise_core::nbt::Tag;
    use sunrise_level::chunk::Chunk;
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
    game.options.dedicated = Some(Vec::new());
    game.world = sunrise_level::world::World::new(game.res.facts().air, 1);
    for z in -2..=2 {
        for x in -2..=2 {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
        }
    }
    game.dim = sunrise_gameplay_common::game::Dim::End;
    let mut ids = Vec::new();
    for (index, kind) in ["minecraft:oak_boat", "minecraft:ender_pearl"]
        .into_iter()
        .enumerate()
    {
        let uuid = [11318, 20, 0, index as i32];
        let tag = Tag::compound([
            ("id", Tag::String(kind.into())),
            ("UUID", Tag::IntArray(uuid.to_vec())),
            (
                "Pos",
                Tag::List(vec![
                    Tag::Double(4.5f64.to_bits()),
                    Tag::Double(210.0f64.to_bits()),
                    Tag::Double(4.5f64.to_bits()),
                ]),
            ),
            ("NoGravity", Tag::Byte(1)),
        ]);
        game.load_listed(kind, Some(uuid), None, tag).unwrap();
        ids.push(game.server.next_entity);
    }
    assert_eq!(game.boats.len(), 1);
    assert_eq!(game.creatures.projectiles.len(), 1);
    assert!(matches!(
        game.creatures.projectiles[0].kind,
        ProjectileKind::EnderPearl
    ));
    game.server
        .tickets
        .0
        .update(Some(sunrise_sim::ticket_levels::Sources {
            forced: vec![(0, 0)],
            ..Default::default()
        }));
    assert!(game.entity_ticking_area()(4, 4));
    for &id in &ids {
        let mut state = Traveller::default();
        state.set_cooldown(5);
        game.end.gateways.insert(id, state);
    }
    let counters = |game: &sunrise_gameplay::game::ServerGame<'_>| {
        [
            game.end.gateways[&ids[0]].cooldown(),
            game.end.gateways[&ids[1]].cooldown(),
        ]
    };
    let mut rows = [[0; 2]; 4];
    for tick in 0..2 {
        game.handle_end_gateway_portals();
        rows[tick * 2] = counters(&game);
        assert_eq!(rows[tick * 2], [4 - tick as i32, 5 - tick as i32]);
        game.handle_end_gateway_portals_phase(true);
        rows[tick * 2 + 1] = counters(&game);
        assert_eq!(rows[tick * 2 + 1], [4 - tick as i32, 4 - tick as i32]);
    }
    assert!(game.end.arrivals.is_empty());
    assert_eq!(game.creatures.projectiles.len(), 1);
    rows
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
