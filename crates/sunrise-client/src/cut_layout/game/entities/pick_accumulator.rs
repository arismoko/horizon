#![cfg(test)]
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
use sunrise_gameplay_common::game::pick_ray::PickAccumulator;
use sunrise_gameplay_common::game::pick_ray::PickCandidate;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;

    /// `ProjectileUtil.getEntityHitResult` with `SulfurCube.canBePickedFromInside`: from inside its
    /// box, a cube is picked without a body item and not with one; from outside, it is picked
    /// either way.
    #[test]
    #[ignore = "daily"]
    fn a_sulfur_cube_holding_a_block_is_not_picked_from_inside() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::motion::PlayerMotion;
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            // A monster can't be summoned on peaceful.
            assert!(game.run_command("difficulty normal"));
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command(
                "summon minecraft:sulfur_cube 4.5 200 4.5 {NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b,Size:1}"
            ));
            let cube = game
                .mobs
                .iter()
                .find(|m| m.species() == sunrise_sim::mobs::Species::SulfurCube)
                .expect("the cube");
            let (id, bounds) = (cube.id, cube.bounds());
            let (min, max) = (bounds.min_position(), bounds.max_position());
            let centre = Vec3::new(
                (min.x + max.x) / 2.0,
                (min.y + max.y) / 2.0,
                (min.z + max.z) / 2.0,
            );
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap());
            let eye = game.aim().unwrap().0[1] - 64.0;
            // Yaw 0 looks along +z.
            (game.yaw, game.pitch) = (0.0, 0.0);
            let eyes_at = |game: &mut sunrise_gameplay::game::ServerGame<'_>, at: Vec3| {
                game.player = Some(
                    PlayerMotion::new(game.motion, Vec3::new(at.x, at.y - eye, at.z)).unwrap(),
                );
            };
            let picked = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.mob_hit().map(|(i, _, _)| game.mobs[i].id)
            };
            eyes_at(game, centre);
            assert_eq!(
                picked(game),
                Some(id),
                "an empty cube is picked from inside"
            );
            let cube = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
            cube.extra.set_body(Some("minecraft:oak_planks".into()));
            assert_eq!(picked(game), None, "a cube holding a block is not");
            eyes_at(game, Vec3::new(centre.x, centre.y, min.z - 1.0));
            assert_eq!(picked(game), Some(id), "from outside it still is");
        });
    }

    #[test]
    fn same_vehicle_uses_the_running_nearest_not_the_candidates_distance() {
        let mut pick = PickAccumulator::new(25.0);
        assert!(!pick.accept(
            PickCandidate {
                id: 2,
                point: [0.0; 3],
                distance_sq: 1.0,
                inside: false,
            },
            true,
            false
        ));
        assert!(pick.accept(
            PickCandidate {
                id: 1,
                point: [0.0; 3],
                distance_sq: 0.0,
                inside: true,
            },
            true,
            false
        ));
        assert!(pick.accept(
            PickCandidate {
                id: 2,
                point: [0.0; 3],
                distance_sq: 9.0,
                inside: false,
            },
            true,
            false
        ));
        assert_eq!(pick.hovered.unwrap().id, 2);
        assert_eq!(pick.nearest_sq, 0.0);
        assert!(pick.accept(
            PickCandidate {
                id: 3,
                point: [0.0; 3],
                distance_sq: 16.0,
                inside: false,
            },
            false,
            false
        ));
        assert_eq!(pick.nearest_sq, 16.0);
        assert!(!pick.accept(
            PickCandidate {
                id: 4,
                point: [0.0; 3],
                distance_sq: 20.25,
                inside: false,
            },
            false,
            false
        ));
    }
}
