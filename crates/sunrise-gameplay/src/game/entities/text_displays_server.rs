#[cfg(any())]
use crate::game::client_side::text_displays::RemoteTextDisplay;
#[cfg(any())]
use crate::game::client_side::text_displays::Shows;

#[cfg(any())]
pub(in crate::game) mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn handle_teleport_keeps_display_easing() {
        use crate::game::{
            commands::OtherEntity,
            native_entity_refs::tests::{fixture_resident_column, with_game},
        };
        use sunrise_core::geometry::Vec3;
        use sunrise_core::nbt::Tag;

        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let tag = Tag::compound([("teleport_duration", Tag::Int(4))]);
            let display = game
                .read_display_nbt("minecraft:text_display", &tag)
                .map(std::sync::Arc::new);
            game.server.others.0.push(OtherEntity {
                size_answer: None,
                attachment_lifetime: Default::default(),
                knot_check: None,
                eye: None,
                join: Default::default(),
                display,
                interaction: None,
                id: 12,
                kind: "minecraft:text_display".into(),
                position: Vec3::ZERO,
                yaw: 0.0,
                pitch: 0.0,
            });
            let entity = game.entity(12).unwrap();
            assert!(entity.teleport_to(game, [8.0, 0.0, 0.0], None));
            let mut metadata = Vec::new();
            game.server.others.0[0]
                .display
                .as_ref()
                .unwrap()
                .append_metadata(None, false, |_| None, &mut metadata);
            let mut client = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
            client.metadata(&metadata);
            client.move_to(entity.position(game).unwrap(), None);
            client.tick();
            assert_eq!(client.position.x, 2.0);
            for _ in 0..3 {
                client.tick();
            }
            assert_eq!(client.position.x, 8.0);
        });
    }
}
