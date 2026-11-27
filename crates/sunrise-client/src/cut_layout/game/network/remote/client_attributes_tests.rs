/// After `ClientboundUpdateAttributes`, the local player's `LivingRef`
/// reads its attribute value, base, modifiers and attribute list from the
/// client's own copy of the synced instances. A mob the server tracks
/// keeps no attribute state of its own on the client, so its reads are
/// `None` there.
#[test]
#[ignore = "daily"]
fn update_attributes_answers_the_local_players_living_reads() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::{AttributeOperation, EntityReadError};
    use std::ops::ControlFlow;
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::{
        buf::Reader,
        connection::Pipe,
        packets::play::{self, Clientbound as C},
    };
    use sunrise_sim::{mobs::Mob, motion::PlayerMotion};

    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "attributes-client").unwrap();
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.player.as_mut().unwrap().entity_id = 42;
        game.mobs.push(Mob::new(60, zombie, Vec3::ZERO, 0.0));
        let remote = game.client.remote.as_mut().unwrap();
        remote.entity_id = 42;
        remote.logged_in = true;
        remote.entities.insert(
            60,
            sunrise_client_gameplay::game::client_side::remote_client::session_state::Tracked {
                kind: "minecraft:zombie".into(),
                position: [0.0; 3],
                previous: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                previous_yaw: 0.0,
                previous_pitch: 0.0,
                codec_base: [0.0; 3],
                on_fire: false,
                mod_fire_immune: false,
                silent: false,
                vehicle: None,
                tick_count: 0,
                size_answer: None,
            },
        );

        let health = "minecraft:max_health";
        let living = game.living(game.entity(42).unwrap()).unwrap();
        // Before the server says anything, the client's own map holds the
        // type's defaults, as a vanilla client's does before its sync.
        assert_eq!(living.attribute_value(game, health), Some(20.0));
        assert_eq!(
            living.visit_attribute_modifiers(game, health, &mut |_| panic!(
                "default modifier visited"
            )),
            Ok(ControlFlow::Continue(()))
        );

        let registry_id = game.motion.attributes.id(health).unwrap().registry_id();
        let packet = C::UpdateAttributes(play::UpdateAttributes {
            id: 42,
            values: vec![play::AttributeSnapshot {
                attribute: registry_id,
                base: 30.0,
                modifiers: vec![play::AttributeModifier {
                    id: "test:bonus".into(),
                    amount: 5.0,
                    operation: 0,
                }],
            }],
        });
        let body = packet.encode();
        let mut reader = Reader::new(&body);
        let id = reader.varint().unwrap();
        game.handle_packet(C::decode(id, reader.rest()).unwrap())
            .unwrap();

        assert_eq!(living.attribute_value(game, health), Some(35.0));
        assert_eq!(living.attribute_base(game, health), Some(30.0));
        let mut modifiers = Vec::new();
        assert_eq!(
            living.visit_attribute_modifiers(game, health, &mut |m| {
                modifiers.push((m.id.to_owned(), m.amount, m.operation));
                ControlFlow::Continue(())
            }),
            Ok(ControlFlow::Continue(()))
        );
        assert_eq!(
            modifiers,
            [("test:bonus".to_owned(), 5.0, AttributeOperation::AddValue)]
        );
        assert_eq!(
            living.visit_attributes(game, &mut |name| {
                if name == health {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            }),
            Ok(ControlFlow::Break(()))
        );

        // The mob's attributes are the server's business: no client state,
        // no read.
        let mob = mod_api::LivingRef::new(game.entity(60).unwrap());
        assert_eq!(mob.attribute_value(game, health), None);
        assert_eq!(mob.attribute_base(game, health), None);
        assert_eq!(
            mob.visit_attribute_modifiers(game, health, &mut |_| panic!(
                "client mob modifier visited"
            )),
            Err(EntityReadError::Unavailable)
        );
        assert_eq!(
            mob.visit_attributes(game, &mut |_| panic!("client mob attribute visited")),
            Err(EntityReadError::Unavailable)
        );
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
