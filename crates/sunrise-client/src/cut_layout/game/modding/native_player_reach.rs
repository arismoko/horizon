#![cfg(test)]

#[cfg(test)]
mod tests {
    use mod_api::{EntityReach, EntityRef, GameMode, Hand, LivingRef};
    use std::sync::Arc;
    use sunrise_core::{geometry::Vec3, nbt::Tag};
    use sunrise_gameplay::game::native_guest_turn_tests::with_resident_guests;
    use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_player_reach::NativePlayerReachGameExt1;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1;
    use sunrise_items::items::ItemStack;
    use sunrise_sim::mobs::Mob;

    /// Each reach rule on both sides of its boundary, for a survival guest with an empty hand
    /// (range 3.0, no attack range component) whose eyes are at (3, 65.62, 5). Pigs stand level
    /// with the eyes on +x; a pig's half-width is `0.9f32 / 2` = 0.449999988079071044921875, so
    /// a pig whose feet are at `3 + d + half` has its near face exactly `d` from the eyes.
    /// Interact is strict below 6.0, the entity container strict below 7.0, the attack inclusive
    /// up to 6.0. With the eyes inside the box the distance is 0 and every rule passes. A handle
    /// that names nothing, and a guest of the hub in another level, answer None. A player in
    /// survival may not fly until the `mayfly` ability is set. Creative guests use range 5.0;
    /// a main-hand attack-range component replaces the attribute with its min/max and margin.
    #[test]
    #[ignore = "daily"]
    fn reach_runs_from_the_eyes_to_the_closest_point_and_flight_follows_the_ability() {
        with_resident_guests(|game| {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let eye_y = 64.0 + 1.62;
            let half = 0.44999998807907104;
            let pig_at = |game: &mut sunrise_gameplay::game::ServerGame<'_>, id: u32, face: f64| {
                let feet = Vec3::new(3.0 + face + half, eye_y - 0.45, 5.0);
                game.mobs.push(Mob::new(id, kind.clone(), feet, 0.0));
                let pig = game.entity(id).unwrap();
                let [lo, _] = game.entity_bounding_box(pig).unwrap();
                assert_eq!(lo[0], 3.0 + face, "pig {id}'s near face");
                pig
            };
            // The fixture leaves the guest without a body, and a player's attributes live on
            // it: give one, as a joined player's record keeps.
            let body =
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(3.0, 64.0, 5.0))
                    .unwrap();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.motion = Some(Box::new(body)))
                .unwrap();
            let player = game.player(12).unwrap();
            let reach = |game: &sunrise_gameplay::game::ServerGame<'_>, pig, rule| {
                game.player_within_entity_reach(player, pig, rule)
            };
            let inside = pig_at(game, 900, -half);
            let short = pig_at(game, 901, 5.5);
            let six = pig_at(game, 902, 6.0);
            let beyond = pig_at(game, 903, 6.5);
            let seven = pig_at(game, 904, 7.0);

            assert_eq!(reach(game, short, EntityReach::Interact), Some(true));
            assert_eq!(reach(game, six, EntityReach::Interact), Some(false));
            assert_eq!(reach(game, beyond, EntityReach::Container), Some(true));
            assert_eq!(reach(game, seven, EntityReach::Container), Some(false));
            assert_eq!(reach(game, six, EntityReach::Attack), Some(true));
            assert_eq!(reach(game, beyond, EntityReach::Attack), Some(false));
            for rule in [
                EntityReach::Interact,
                EntityReach::Container,
                EntityReach::Attack,
            ] {
                assert_eq!(
                    reach(game, inside, rule),
                    Some(true),
                    "{rule:?}, eyes inside"
                );
            }
            // Creative's +2 modifier gives range 5.0 to a real guest, including when
            // read outside that guest's turn. The server adds the creative reach bonus in
            // the player's tick step, not when the mode changes, so each mode change is
            // followed by that tick step. These distances are literal box faces.
            let tick_attributes = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                assert!(game.native_as_player_handle(player, |g| g.update_player_attributes()));
            };
            assert!(game.player_set_game_mode(player, GameMode::Creative));
            tick_attributes(game);
            let living = LivingRef::new(player.entity());
            assert_eq!(
                game.living_attribute_value(living, "minecraft:entity_interaction_range"),
                Some(5.0)
            );
            let almost_eight = pig_at(game, 905, 7.5);
            let eight = pig_at(game, 906, 8.0);
            let beyond_eight = pig_at(game, 907, 8.5);
            let nine = pig_at(game, 908, 9.0);
            assert_eq!(reach(game, almost_eight, EntityReach::Interact), Some(true));
            assert_eq!(reach(game, eight, EntityReach::Interact), Some(false));
            assert_eq!(
                reach(game, beyond_eight, EntityReach::Container),
                Some(true)
            );
            assert_eq!(reach(game, nine, EntityReach::Container), Some(false));
            assert_eq!(reach(game, eight, EntityReach::Attack), Some(true));
            assert_eq!(reach(game, beyond_eight, EntityReach::Attack), Some(false));
            assert!(game.player_set_game_mode(player, GameMode::Survival));
            tick_attributes(game);

            // A held component replaces the attribute: min 5, max 6, default margin
            // 0.3. Float subtraction/addition before widening gives inclusive server
            // limits 1.6999998092651367 and 9.300000190734863. Literal expectations
            // pin both ends, inclusion, and the margin; no range helper supplies them.
            let mut weapon = ItemStack::new("minecraft:stick", 1);
            weapon.data_mut().set_other(
                "minecraft:attack_range",
                Some(Tag::Compound(
                    [
                        ("min_reach".into(), Tag::Float(5.0_f32.to_bits())),
                        ("max_reach".into(), Tag::Float(6.0_f32.to_bits())),
                    ]
                    .into_iter()
                    .collect(),
                )),
            );
            assert!(game.living_set_hand_item(living, Hand::Main, Some(weapon)));
            for (id, distance, expected) in [
                (909, 1.5, false),
                (910, 1.6999998092651367, true),
                (911, 1.75, true),
                (912, 9.25, true),
                (913, 9.300000190734863, true),
                (914, 9.5, false),
            ] {
                let pig = pig_at(game, id, distance);
                assert_eq!(
                    reach(game, pig, EntityReach::Attack),
                    Some(expected),
                    "distance {distance}"
                );
            }
            // Interact still uses the attribute, not the main hand's attack range.
            assert_eq!(reach(game, six, EntityReach::Interact), Some(false));
            assert!(game.living_set_hand_item(living, Hand::Main, None));

            // A handle that names nothing real: None.
            let stale = EntityRef::new(short.epoch(), 424242);
            assert_eq!(reach(game, stale, EntityReach::Interact), None);

            // A guest of the hub whose level is the nether: another level, None.
            use sunrise_protocol::packets::configuration::ClientInformation;
            use sunrise_sim::lan::{Joined, PlayerSender, guest::Guest};
            let (tx, _rx) = std::sync::mpsc::channel();
            use std::sync::atomic::{AtomicI32, AtomicUsize};
            let mut guest = Guest::new(
                Joined {
                    conn: 9,
                    uuid: 7777,
                    name: "NetherGuest".to_owned(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                777,
                [0.0, 64.0, 0.0],
                10,
            );
            guest.placed = true;
            guest.dimension = "minecraft:the_nether".to_owned();
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
            game.reset_entity_residency();
            let nether = game.entity(777).unwrap();
            assert_eq!(reach(game, nether, EntityReach::Interact), None);

            assert_eq!(game.player_may_fly(player), Some(false));
            let mut abilities = game.player_abilities(player).unwrap();
            abilities.may_fly = true;
            assert!(game.player_set_abilities(player, abilities));
            assert_eq!(game.player_may_fly(player), Some(true));
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn client_reach_uses_predicted_eyes_and_the_tracked_targets_box() {
        use sunrise_protocol::packets::play;
        use sunrise_sim::motion::PlayerMotion;
        use {
            crate::game::server_side::native_entity_refs::tests::with_client_game,
            sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
        };

        with_client_game(|game| {
            let _server = logged_client(game);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let player = game.local_player().unwrap();
            let me = player.entity();
            let eye_height = f64::from(game.player.as_ref().unwrap().dimensions.eye_height);
            let host = &*game as &dyn mod_api::ClientHost;
            assert_eq!(me.eye_height(host), Some(eye_height));
            assert_eq!(me.eye_position(host), Some([0.0, eye_height, 0.0]));
            assert_eq!(
                LivingRef::new(me).attribute_value(host, "minecraft:entity_interaction_range"),
                Some(3.0)
            );
            // The next read sees a predicted pose's new eye height, with no server update.
            game.player.as_mut().unwrap().dimensions.eye_height = 0.4;
            assert_eq!(
                me.eye_position(&*game as &dyn mod_api::ClientHost),
                Some([0.0, f64::from(0.4_f32), 0.0])
            );
            game.player.as_mut().unwrap().dimensions.eye_height = eye_height as f32;

            // Survival's synced range is 3; strict interact/container limits are 6 and 7,
            // and attack includes 6. Pig half-width is 0.9f32 / 2 widened to f64.
            for (id, face, interact, container, attack) in [
                (20, -0.44999998807907104, true, true, true),
                (21, 5.5, true, true, true),
                (22, 6.0, false, true, true),
                (23, 6.5, false, true, false),
                (24, 7.0, false, false, false),
            ] {
                game.spawn_entity(
                    id,
                    "minecraft:pig",
                    [face + 0.44999998807907104, eye_height - 0.45, 0.0],
                    [0.0; 3],
                );
                let pig = game.entity(id as u32).unwrap();
                let host = &*game as &dyn mod_api::ClientHost;
                assert_eq!(pig.bounding_box(host).unwrap()[0][0], face);
                for (rule, expected) in [
                    (EntityReach::Interact, interact),
                    (EntityReach::Container, container),
                    (EntityReach::Attack, attack),
                ] {
                    assert_eq!(
                        player.within_entity_reach(host, pig, rule),
                        Some(expected),
                        "face {face}, {rule:?}"
                    );
                }
            }

            // The client reads its main hand's attack-range component too: min 5, max 6,
            // margin 0.3 and server buffer 3 admit 9 but refuse a target nearer than 1.7.
            let mut weapon = ItemStack::new("minecraft:stick", 1);
            weapon.data_mut().set_other(
                "minecraft:attack_range",
                Some(Tag::Compound(
                    [
                        ("min_reach".into(), Tag::Float(5.0_f32.to_bits())),
                        ("max_reach".into(), Tag::Float(6.0_f32.to_bits())),
                    ]
                    .into_iter()
                    .collect(),
                )),
            );
            game.selected = 0;
            game.inventory[0] = Some(weapon);
            game.spawn_entity(
                25,
                "minecraft:pig",
                [9.449999988079071, eye_height - 0.45, 0.0],
                [0.0; 3],
            );
            let far = game.entity(25).unwrap();
            let inside = game.entity(20).unwrap();
            let host = &*game as &dyn mod_api::ClientHost;
            assert_eq!(
                player.within_entity_reach(host, far, EntityReach::Attack),
                Some(true)
            );
            assert_eq!(
                player.within_entity_reach(host, inside, EntityReach::Attack),
                Some(false)
            );
            assert_eq!(
                player.within_entity_reach(host, far, EntityReach::Interact),
                Some(false)
            );
            let stale = EntityRef::new(mod_api::WorldEpoch::new(0), 25);
            assert_eq!(stale.eye_position(host), None);
            assert_eq!(
                player.within_entity_reach(host, stale, EntityReach::Interact),
                None
            );
            game.handle_packet(play::Clientbound::RemoveEntities(play::RemoveEntities {
                ids: vec![25],
            }))
            .unwrap();
            assert_eq!(
                player.within_entity_reach(
                    &*game as &dyn mod_api::ClientHost,
                    far,
                    EntityReach::Attack
                ),
                None
            );
            game.client.remote.as_mut().unwrap().logged_in = false;
            let host = &*game as &dyn mod_api::ClientHost;
            assert_eq!(me.eye_height(host), None);
            assert_eq!(me.eye_position(host), None);
            assert_eq!(
                player.within_entity_reach(host, inside, EntityReach::Interact),
                None
            );
        });
    }
}
