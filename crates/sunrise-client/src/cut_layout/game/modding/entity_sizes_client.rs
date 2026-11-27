#![cfg(test)]
#[cfg(any())]
pub(crate) mod tests {

    use sunrise_client_gameplay::game::client_side::entity_sizes_client::tests::white_outline;

    use sunrise_gameplay::game::server_side::entity_sizes::tests::add_entity;

    use sunrise_gameplay::game::server_side::entity_sizes::tests::doubled;

    use sunrise_gameplay::game::server_side::entity_sizes::tests::entity_data;

    use sunrise_gameplay::game::server_side::entity_sizes::tests::with_loopback_client;

    use mod_api::Pose;

    /// A client's copies of entities that aren't mobs ask too: a minecart's box, the
    /// crosshair's pick, its F3+B outline and the box mods read take the answer; an armor stand
    /// asks again when it turns small; a display asks from its constructor.
    #[test]
    #[ignore = "daily"]
    fn client_copies_of_other_kinds_ask_and_take_the_answer() {
        use sunrise_protocol::entity_data::{Value, serializer};
        if sunrise_gameplay_common::game::test_fixtures::run_fresh_cases(
            "cut_layout::game::client_side::entity_sizes_client::tests::client_copies_of_other_kinds_ask_and_take_the_answer",
            "SUNRISE_TEST_CLIENT_COPY_SIZE",
            &["client_copies_of_other_kinds_ask_and_take_the_answer"],
        ) {
            return;
        }
        with_loopback_client(|game, seen| {
            let at = [8.5, 70.0, 8.5];
            add_entity(game, 32, "minecraft:minecart", at);
            add_entity(game, 41, "minecraft:armor_stand", [2.5, 70.0, 2.5]);
            add_entity(game, 61, "minecraft:block_display", [4.5, 70.0, 4.5]);
            // EntityType.MINECART: 0.98 by 0.7, eyes 0.595; ARMOR_STAND: 0.5 by 1.975, eyes
            // 1.7775; BLOCK_DISPLAY: none.
            let (cart, stand) = ([0.98, 0.7, 0.595], [0.5, 1.975, 1.7775]);
            // ArmorStand.DATA_CLIENT_FLAGS small: BABY_DIMENSIONS, the type's at half scale
            // with its eyes at 0.9875.
            entity_data(game, 41, vec![(15, serializer::BYTE, Value::Byte(1))]);
            assert_eq!(
                seen.lock().unwrap().sizes,
                [
                    (32, Pose::Standing, cart, cart),
                    (41, Pose::Standing, stand, stand),
                    (61, Pose::Standing, [0.0; 3], [0.0; 3]),
                    (41, Pose::Standing, doubled(stand), [0.25, 0.9875, 0.9875]),
                ]
            );
            let [width, height, _] = doubled(cart);
            let half = f64::from(width / 2.0);
            let min = [at[0] - half, at[1], at[2] - half];
            let max = [at[0] + half, at[1] + f64::from(height), at[2] + half];
            let entity = game.entity(32).unwrap();
            assert_eq!(game.entity_bounding_box(entity), Some([min, max]));
            // Only the cart and the armor stand show an outline: keep the cart's.
            game.armor_stands.clear();
            assert_eq!(white_outline(game), (min, max));
            // Aimed over a vanilla cart (y 70.7) but into the answered box: the crosshair
            // picks the cart.
            game.side.title.0 = None;
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(5.5, 69.5, 8.5),
                )
                .unwrap(),
            );
            (game.yaw, game.pitch, game.entity_reach) = (-90.0, 0.0, 3.0);
            game.refresh_client_pick(1.0);
            assert!(matches!(
                game.crosshair_target(),
                Some(mod_api::CrosshairTarget::Entity { entity: hit, .. }) if hit == entity
            ));
        });
    }
}
