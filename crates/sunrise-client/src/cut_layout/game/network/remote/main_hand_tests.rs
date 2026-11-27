use sunrise_core::geometry::Vec3;
use sunrise_protocol::{buf::Reader, connection::Pipe, packets::play::Clientbound};
use sunrise_sim::{
    lan::entity::{Entities, EntityState, Viewer},
    mobs::Mob,
};
use {
    crate::game::remote::tests::spawn_menu_test,
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay_common::game::lan_meta_fixture::meta_hub,
    sunrise_gameplay_common::game::lan_mob_metadata::mob_meta,
};

#[test]
#[ignore = "daily"]
fn a_client_receives_a_mannequins_main_hand() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::tests::main_hand_tests::a_client_receives_a_mannequins_main_hand",
        || {
            with_client_game(|game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "MannequinArmTest").unwrap();
                spawn_menu_test(game);

                let kind = "minecraft:mannequin";
                let network_kind = game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:entity_type")
                    .unwrap()
                    .id(kind)
                    .unwrap();
                let mut mannequin = Mob::new(
                    30,
                    game.res.mob_types().get(kind).unwrap().clone(),
                    Vec3::new(0.5, 64.0, 0.5),
                    0.0,
                );
                let hub = meta_hub("MannequinArmTest");
                let mut tracker = Default::default();
                let mut entities = Entities::default();
                let chunks = [(0, 0)].into_iter().collect();
                for left in [true, false] {
                    mannequin.extra.main_hand_left = left;
                    let mut state = EntityState {
                        id: 30,
                        uuid: 30,
                        kind: kind.into(),
                        position: [0.5, 64.0, 0.5],
                        ..Default::default()
                    };
                    mob_meta(
                        &mannequin,
                        &hub,
                        None,
                        0,
                        &mut state.meta,
                        &mut state.defaults,
                    );

                    let mut out = Vec::new();
                    entities.tick(
                        &[&state],
                        &Default::default(),
                        &mut [Viewer {
                            entity_id: 1,
                            spectator: false,
                            permission: 0,
                            sunrise: true,
                            general_entity: false,
                            attachment_sender: None,
                            moved: false,
                            position: state.position,
                            view_distance: 8,
                            chunks: &chunks,
                            sources: None,
                            far: &[],
                            tracker: &mut tracker,
                            out: &mut out,
                        }],
                        None,
                        |_| Some(network_kind),
                    );

                    for body in out {
                        let mut reader = Reader::new(&body);
                        let id = reader.varint().unwrap();
                        let packet = Clientbound::decode(id, reader.rest()).unwrap();
                        game.apply_packet(packet).unwrap();
                    }
                    let seen = game.mobs.iter().find(|mob| mob.id == 30).unwrap();
                    assert_eq!(seen.main_arm_left(), left);
                }
            });
        },
    );
}
