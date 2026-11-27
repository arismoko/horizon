#![cfg(test)]
use sunrise_core::geometry::Aabb;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::piston_push::Lid;
#[cfg(test)]
mod client_lid_tests {
    use super::*;
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use std::sync::Arc;
    use sunrise_gameplay_common::game::boats_common::Boat;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::{mobs::Mob, motion::PlayerMotion};

    /// `isPassengerOfSameVehicle` on a client's own player: a shulker riding a
    /// boat spares the player riding the same boat; once the shulker rides
    /// nothing, the same lid lifts the player by the whole 0.25.
    #[test]
    #[ignore = "daily"]
    fn a_shulker_lid_spares_what_shares_its_vehicle_on_client() {
        with_client_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            for x in 0..8 {
                for z in 0..8 {
                    game.world.set(x, 199, z, stone).unwrap();
                }
            }
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:shulker")
                .unwrap()
                .clone();
            let mut shell = Mob::new(72, kind, Vec3::new(6.5, 200.0, 6.5), 0.0);
            shell.extra.mount = Some(71);
            game.boats
                .push(Boat::new(71, "oak".into(), Vec3::new(2.5, 200.0, 6.5), 0.0));
            game.mobs.push(shell);
            game.riding = Some(71);

            let around = Aabb::new(Vec3::new(1.0, 200.0, 1.0), Vec3::new(4.0, 201.0, 8.0));
            let lift = Vec3::new(0.0, 0.25, 0.0);
            let player_y = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                game.player.as_ref().unwrap().position.y
            };
            game.lid_push_local(Lid::Shulker(72), around, lift);
            assert_eq!(
                game.boats[0].position.y, 200.0,
                "the shulker's vehicle stays"
            );
            assert_eq!(player_y(game), 200.0, "a client's own player stays too");

            game.mobs[0].extra.mount = None;
            game.lid_push_local(Lid::Shulker(72), around, lift);
            assert_eq!(player_y(game), 200.25, "nothing shared: the lid lifts it");
        });
    }
}
