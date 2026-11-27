//! `EntityRef::move_by` and the hanging entities `LevelRef::visit_entities_in` visits.

use super::native_entity_refs::tests::{collect_game_entities_in, with_game};
use mod_api::MoveCause;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;
use sunrise_sim::entities::{ArmorStand, ItemFrame, Painting};

const STAND: u32 = 910;
const FRAME: u32 = 911;
const PAINTING: u32 = 912;
const PIG: u32 = 913;

/// A server's game with an armor stand at 0.5, 64, 0.5, an item frame at 4, 70, 4 and a painting
/// at 4, 70, 6 (the frame's and painting's boxes are a block wide, thin in depth).
fn with_scene(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.armor_stands
            .push(ArmorStand::new(STAND, Vec3::new(0.5, 64.0, 0.5), 0.0));
        game.item_frames.push(ItemFrame {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: FRAME,
            pos: [4, 70, 4],
            facing: 3,
            glow: false,
            item: None,
            rotation: 0,
            drop_chance: 1.0,
            fixed: false,
            invisible: false,
            custom_name: None,
            random: sunrise_core::random::LegacyRandom::new(0),
            check_interval: 0,
            ticks: 0,
        });
        game.paintings.push(Painting {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: PAINTING,
            pos: [4, 70, 6],
            facing: 0,
            variant: "minecraft:kebab".into(),
            asset: None,
            width: 1,
            height: 1,
            check_interval: 0,
            ticks: 0,
        });
        game.server.next_entity = game.server.next_entity.max(PIG);
        test(game);
    });
}

#[test]
#[ignore = "daily"]
fn an_area_query_lists_the_frames_and_paintings_in_the_box() {
    with_scene(|game| {
        fixture_resident_column(&mut game.world, (0, 0));
        let level = game.level("minecraft:overworld").unwrap();
        let frame = game.entity(FRAME).unwrap();
        let painting = game.entity(PAINTING).unwrap();
        let around = |game: &sunrise_gameplay::game::ServerGame<'_>, min, max| {
            collect_game_entities_in(game, level, min, max)
        };
        let both = around(game, [3.0, 69.0, 3.0], [5.0, 71.0, 7.0]).unwrap();
        assert!(both.contains(&frame) && both.contains(&painting));
        let one = around(game, [3.0, 69.0, 3.0], [5.0, 71.0, 5.0]).unwrap();
        assert!(one.contains(&frame) && !one.contains(&painting));
        let none = around(game, [10.0, 69.0, 3.0], [12.0, 71.0, 7.0]).unwrap();
        assert!(!none.contains(&frame) && !none.contains(&painting));
    });
}

#[test]
#[ignore = "daily"]
fn move_by_stops_at_a_wall_and_refuses_what_has_no_physics() {
    with_scene(|game| {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        // A wall in the stand's column once it has moved north to z = 1.5.
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        for y in 63..67 {
            game.world.set(2, y, 1, stone).unwrap();
        }
        let stand = game.entity(STAND).unwrap();
        // Open air: the whole way.
        assert_eq!(
            game.entity_move_by(stand, [0.0, 0.0, 1.0], MoveCause::SelfMovement),
            Some([0.0, 0.0, 1.0])
        );
        assert_eq!(game.armor_stands[0].position.z, 1.5);
        game.armor_stands[0].velocity = Vec3::new(0.3, 0.0, 0.1);
        // Towards the wall at x = 2: the stand's half width (0.25) stops it at x = 1.75.
        let moved = game
            .entity_move_by(stand, [3.0, 0.0, 0.0], MoveCause::SelfMovement)
            .unwrap();
        assert!((moved[0] - 1.25).abs() < 1.0e-6 && moved[1] == 0.0 && moved[2] == 0.0);
        assert!((game.armor_stands[0].position.x - 1.75).abs() < 1.0e-6);
        assert_eq!(
            game.armor_stands[0].velocity,
            Vec3::new(0.3, 0.0, 0.1),
            "a blocked move keeps the velocity"
        );
        // A marker stand has no physics: the whole delta, through the wall.
        game.armor_stands[0].marker = true;
        let moved = game
            .entity_move_by(stand, [1.0, 0.0, 0.0], MoveCause::SelfMovement)
            .unwrap();
        assert_eq!(moved, [1.0, 0.0, 0.0]);
        game.armor_stands[0].marker = false;
        // A piston's cause applies its per-tick limit, 0.51 a block on an axis: a second move in
        // the same tick (a mob keeps its own budget) goes nowhere.
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        game.mobs.push(sunrise_sim::mobs::Mob::new(
            PIG,
            kind,
            Vec3::new(8.5, 80.0, 8.5),
            0.0,
        ));
        let pig = game.entity(PIG).unwrap();
        let first = game
            .entity_move_by(pig, [1.0, 0.0, 0.0], MoveCause::Piston)
            .unwrap();
        assert!((first[0] - 0.51).abs() < 1.0e-6);
        assert_eq!(
            game.entity_move_by(pig, [1.0, 0.0, 0.0], MoveCause::Piston),
            Some([0.0; 3])
        );
        // Bad deltas, hanging entities and a stale handle.
        assert_eq!(
            game.entity_move_by(stand, [f64::NAN; 3], MoveCause::Piston),
            None
        );
        assert_eq!(
            game.entity_move_by(stand, [65.0, 0.0, 0.0], MoveCause::Piston),
            None
        );
        let frame = game.entity(FRAME).unwrap();
        assert_eq!(
            game.entity_move_by(frame, [1.0, 0.0, 0.0], MoveCause::SelfMovement),
            None
        );
        let gone = mod_api::EntityRef::new(
            mod_api::WorldEpoch::new(stand.epoch().get() + 1),
            stand.id(),
        );
        assert_eq!(
            game.entity_move_by(gone, [1.0, 0.0, 0.0], MoveCause::SelfMovement),
            None
        );
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
