#![cfg(test)]
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    #[cfg(feature = "client")]
    use super::*;
    #[cfg(feature = "client")]
    use mod_api::EntityRef;
    #[cfg(feature = "client")]
    use sunrise_core::geometry::Vec3;
    #[cfg(feature = "client")]
    use sunrise_sim::entities::ArmorStand;
    #[cfg(feature = "client")]
    use sunrise_sim::mobs::Mob;
    #[cfg(feature = "client")]
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay::game::native_entity_refs::tests::with_resident_game,
    };

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn piston_reaction_and_hanging_class_follow_server_and_client_entities() {
        use mod_api::PushReaction::{IgnoreEntity, PushPull};

        fn read<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(
            game: &G,
            id: u32,
        ) -> (mod_api::PushReaction, bool) {
            let entity = game.entity(id).unwrap();
            (
                entity
                    .piston_push_reaction(game.as_host())
                    .unwrap()
                    .unwrap(),
                entity.is_hanging(game.as_host()).unwrap().unwrap(),
            )
        }
        with_resident_game(|game| {
            game.build_registries();
            let pig_ty = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(10, pig_ty, Vec3::ZERO, 0.0));
            let mut stand = ArmorStand::new(11, Vec3::ZERO, 0.0);
            stand.marker = true;
            game.armor_stands.push(stand);
            assert!(game.as_console(|game| {
                game.run_command("summon minecraft:item_frame 0 64 0 {Facing:2b,Fixed:1b}")
            }));
            assert_eq!(read(game, 10), (PushPull, false));
            assert_eq!(read(game, 11), (IgnoreEntity, false));
            assert_eq!(read(game, game.item_frames[0].id), (PushPull, true));
        });
        with_client_game(|game| {
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "reaction").unwrap();
            game.client.remote.as_mut().unwrap().logged_in = true;
            use sunrise_protocol::packets::play::{self, Clientbound as C};

            for (id, key) in [
                (10, "minecraft:pig"),
                (11, "minecraft:armor_stand"),
                (12, "minecraft:item_frame"),
                (13, "minecraft:leash_knot"),
                (14, "minecraft:block_display"),
            ] {
                let kind = game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:entity_type")
                    .unwrap()
                    .id(key)
                    .unwrap();
                game.handle_packet(C::AddEntity(play::AddEntity {
                    id,
                    uuid: id as u128,
                    kind,
                    position: [0.0; 3],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                }))
                .unwrap();
            }
            assert_eq!(read(game, 11), (PushPull, false));
            game.handle_packet(C::SetEntityData(play::SetEntityData {
                id: 11,
                values: vec![(15, 0, sunrise_protocol::entity_data::Value::Byte(16))],
            }))
            .unwrap();
            assert_eq!(read(game, 10), (PushPull, false));
            assert_eq!(read(game, 11), (IgnoreEntity, false));
            assert_eq!(read(game, 12), (PushPull, true));
            assert_eq!(read(game, 13), (PushPull, false));
            assert_eq!(read(game, 14), (IgnoreEntity, false));

            let stale = EntityRef::new(mod_api::WorldEpoch::new(0), 11);
            assert_eq!(stale.piston_push_reaction(game), Ok(None));
            assert_eq!(stale.is_hanging(game), Ok(None));
        });
    }
}
