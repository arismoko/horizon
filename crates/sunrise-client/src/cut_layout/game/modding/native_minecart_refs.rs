#![cfg(test)]
use mod_api::EntityRef;
use mod_api::MinecartKind;
use mod_api::MinecartRef;
use sunrise_gameplay_common::game::minecarts::CartKind;
use sunrise_gameplay_common::game::minecarts::Minecart;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_resident_game;
    use super::*;
    use mod_api::WorldEpoch;
    use sunrise_core::geometry::Vec3;
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_sim::entities::ArmorStand;

    fn spawn(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        id: u32,
        kind: CartKind,
    ) -> MinecartRef {
        game.carts.push(Minecart::new(id, kind, Vec3::ZERO));
        game.minecart(game.entity(id).unwrap()).unwrap()
    }

    #[test]
    fn furnace_fuel_adds_with_java_int_arithmetic() {
        assert_eq!(
            sunrise_gameplay_common::game::minecarts::furnace_fuel_after(0),
            Some(3600)
        );
        assert_eq!(
            sunrise_gameplay_common::game::minecarts::furnace_fuel_after(28_400),
            Some(32_000)
        );
        assert_eq!(
            sunrise_gameplay_common::game::minecarts::furnace_fuel_after(28_401),
            None
        );
        assert_eq!(
            sunrise_gameplay_common::game::minecarts::furnace_fuel_after(i32::MAX),
            Some(i32::MIN + 3599)
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_furnace_carts_push_is_read_and_set_through_its_handle() {
        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let furnace = spawn(game, 50, CartKind::Furnace);
            let chest = spawn(game, 51, CartKind::Chest);
            assert_eq!(game.minecart_push(furnace), Some([0.0, 0.0, 0.0]));
            assert!(game.minecart_set_push(furnace, [0.5, 0.0, -0.5]));
            assert_eq!(game.minecart_push(furnace), Some([0.5, 0.0, -0.5]));
            assert_eq!(game.carts[0].push, Vec3::new(0.5, 0.0, -0.5));
            // Not a furnace cart, or a push that isn't finite: refused.
            assert_eq!(game.minecart_push(chest), None);
            assert!(!game.minecart_set_push(chest, [0.5, 0.0, 0.0]));
            assert!(!game.minecart_set_push(furnace, [f64::NAN, 0.0, 0.0]));
            assert_eq!(game.minecart_push(furnace), Some([0.5, 0.0, -0.5]));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_carts_display_block_and_offset_round_trip_through_its_handle() {
        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let cart = spawn(game, 60, CartKind::Chest);
            assert_eq!(cart.custom_display_block(game), Some(None));
            assert_eq!(cart.display_offset(game), Some(8));
            let lit = game
                .block_state_id("minecraft:furnace[facing=east,lit=true]")
                .unwrap()
                .try_into_api()
                .unwrap();
            assert!(cart.set_custom_display_block(game, Some(lit)));
            assert!(cart.set_display_offset(game, 3));
            assert_eq!(cart.custom_display_block(game), Some(Some(lit)));
            assert_eq!(cart.display_offset(game), Some(3));

            // Saved and loaded back as the same state and offset.
            let saved = game.carts[0].to_nbt([0; 4], None, game.res.items());
            let mut loaded = Minecart::from_nbt(&saved, 61, CartKind::Chest).unwrap();
            sunrise_gameplay_common::game::minecarts::forget_unreadable_display(&mut loaded, |t| {
                game.state_from_nbt(t).is_some()
            });
            game.carts.push(loaded);
            let loaded = game.minecart(game.entity(61).unwrap()).unwrap();
            assert_eq!(loaded.custom_display_block(game), Some(Some(lit)));
            assert_eq!(loaded.display_offset(game), Some(3));

            // A state the registry doesn't know is refused; None clears.
            let invalid = mod_api::__private::block_state_from_ordinal(lit.block(), u16::MAX);
            assert!(!cart.set_custom_display_block(game, Some(invalid)));
            assert_eq!(cart.custom_display_block(game), Some(Some(lit)));
            assert!(cart.set_custom_display_block(game, None));
            assert_eq!(cart.custom_display_block(game), Some(None));
            assert!(
                game.carts[0]
                    .to_nbt([0; 4], None, game.res.items())
                    .get("DisplayState")
                    .is_none()
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn minecart_handles_narrow_read_and_write_by_kind() {
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::native_minecart_refs::tests::minecart_handles_narrow_read_and_write_by_kind",
            "SUNRISE_MINECART_HANDLES_CASE",
            &["handles"],
        ) {
            return;
        }
        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let kinds = [
                (CartKind::Rideable, MinecartKind::Rideable),
                (CartKind::Chest, MinecartKind::Chest),
                (CartKind::Furnace, MinecartKind::Furnace),
                (CartKind::Tnt, MinecartKind::Tnt),
                (CartKind::Hopper, MinecartKind::Hopper),
                (CartKind::CommandBlock, MinecartKind::CommandBlock),
                (CartKind::Spawner, MinecartKind::Spawner),
            ];
            let carts: Vec<_> = kinds
                .iter()
                .zip(10..)
                .map(|((kind, _), id)| spawn(game, id, *kind))
                .collect();
            for (cart, (_, expected)) in carts.iter().zip(kinds) {
                assert_eq!(cart.kind(game), Some(expected));
            }
            let [rideable, _, furnace, tnt, hopper, ..] = carts[..] else {
                unreachable!()
            };
            game.armor_stands.push(ArmorStand::new(99, Vec3::ZERO, 0.0));
            assert!(game.minecart(game.entity(99).unwrap()).is_none());

            // Max speed: old behaviour, water, furnace, then the experiment's rule.
            assert_eq!(rideable.max_speed(game), Some(0.4));
            assert_eq!(furnace.max_speed(game), Some(0.2));
            game.carts.update(2, |c| c.in_water = true);
            assert_eq!(furnace.max_speed(game), Some(0.2 * 0.75));
            game.carts.update(2, |c| c.in_water = false);
            let mut features = game.level.enabled_features.to_vec();
            features.push("minecraft:minecart_improvements".into());
            game.level.enabled_features = features.into();
            game.ui.rules = sunrise_game_data::gamerules::GameRules::with_features(
                &game.level.enabled_features,
            );
            assert_eq!(rideable.max_speed(game), Some(8.0 / 20.0));
            assert_eq!(furnace.max_speed(game), Some(8.0 / 20.0 * 0.5));
            game.carts.update(0, |c| c.in_water = true);
            assert_eq!(rideable.max_speed(game), Some(8.0 / 20.0 * 0.5));

            assert_eq!(rideable.is_on_rails(game), Some(false));
            game.carts.update(0, |c| c.on_rails = true);
            assert_eq!(rideable.is_on_rails(game), Some(true));

            assert!(rideable.set_flipped(game, true));
            assert_eq!(rideable.flipped(game), Some(true));
            assert!(rideable.set_damage(game, 12.5));
            assert!(rideable.set_hurt_time(game, 7));
            assert_eq!(
                (rideable.damage(game), rideable.hurt_time(game)),
                (Some(12.5), Some(7))
            );
            let saved = game.carts[0].to_nbt([0; 4], None, game.res.items());
            assert_eq!(saved.get("FlippedRotation"), Some(&Tag::Byte(1)));

            // Furnace fuel, saved and synced as the lit flag.
            assert_eq!(furnace.fuel(game), Some(0));
            assert!(furnace.set_fuel(game, 3600));
            assert_eq!(furnace.fuel(game), Some(3600));
            assert!(!tnt.set_fuel(game, 5));
            assert_eq!(tnt.fuel(game), None);
            let saved = game.carts[2].to_nbt([0; 4], None, game.res.items());
            assert_eq!(saved.get("Fuel"), Some(&Tag::Short(3600)));
            // The field is an int; only the save casts to a short.
            assert!(furnace.set_fuel(game, 40_000));
            assert_eq!(furnace.fuel(game), Some(40_000));
            assert_eq!(
                game.carts[2]
                    .to_nbt([0; 4], None, game.res.items())
                    .get("Fuel"),
                Some(&Tag::Short(40_000_i32 as i16))
            );
            assert!(furnace.set_fuel(game, i32::MIN));
            assert_eq!(furnace.fuel(game), Some(i32::MIN));
            assert!(furnace.set_fuel(game, 3600));

            // TNT fuse.
            assert_eq!(tnt.fuse(game), Some(-1));
            game.start_audio_capture();
            assert!(tnt.prime_fuse(game));
            assert_eq!(tnt.fuse(game), Some(80));
            // MinecartTNT.primeFuse broadcasts entity event 70 for clients.
            assert!(
                game.server
                    .lan_entity_events
                    .iter()
                    .any(|e| (e.0, e.1) == (tnt.entity().id() as i32, 70))
            );
            game.stop_audio_capture();
            assert_eq!(
                game.carts[3]
                    .to_nbt([0; 4], None, game.res.items())
                    .get("fuse"),
                Some(&Tag::Int(80))
            );
            assert!(!furnace.prime_fuse(game));
            game.ui.rules.set(
                "tnt_explodes",
                sunrise_game_data::gamerules::Value::Bool(false),
            );
            game.carts.update(3, |c| c.fuse = -1);
            assert!(!tnt.prime_fuse(game));
            assert_eq!(tnt.fuse(game), Some(-1));

            // The tick counts wobble down only while positive.
            assert!(rideable.set_hurt_time(game, i32::MIN));
            assert!(rideable.set_damage(game, 0.0));
            game.tick_carts();
            assert_eq!(rideable.hurt_time(game), Some(i32::MIN));
            assert!(rideable.set_hurt_time(game, -1));
            assert!(rideable.set_damage(game, -0.5));
            game.tick_carts();
            assert_eq!(
                (rideable.hurt_time(game), rideable.damage(game)),
                (Some(-1), Some(-0.5))
            );
            assert!(rideable.set_hurt_time(game, 3));
            assert!(rideable.set_damage(game, 2.5));
            game.tick_carts();
            assert_eq!(
                (rideable.hurt_time(game), rideable.damage(game)),
                (Some(2), Some(1.5))
            );

            // Hopper.
            assert_eq!(hopper.hopper_enabled(game), Some(true));
            assert!(hopper.set_hopper_enabled(game, false));
            assert_eq!(
                game.carts[4]
                    .to_nbt([0; 4], None, game.res.items())
                    .get("Enabled"),
                Some(&Tag::Byte(0))
            );
            assert!(!rideable.set_hopper_enabled(game, false));
            assert_eq!(rideable.hopper_enabled(game), None);

            // Command block: the write resets the success count and syncs the command and output
            // for clients (`onUpdated`), and the cart then runs the new command.
            let command = carts[5];
            assert_eq!(command.command(game).as_deref(), Some(""));
            game.carts.update(5, |c| c.command.success_count = 3);
            assert!(command.set_command(game, "gamerule mob_griefing false"));
            assert_eq!(
                command.command(game).as_deref(),
                Some("gamerule mob_griefing false")
            );
            let cart = &game.carts[5];
            assert_eq!(cart.command.success_count, 0);
            assert_eq!(cart.synced_command, "gamerule mob_griefing false");
            assert!(cart.synced_output.is_none());
            let id = command.entity().id();
            assert!(
                game.ui
                    .rules
                    .bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING)
            );
            assert!(game.perform_cart_command(id));
            assert!(
                !game
                    .ui
                    .rules
                    .bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING),
                "the new command ran"
            );
            assert!(!rideable.set_command(game, "say hi"));
            assert_eq!(rideable.command(game), None);

            let stale = MinecartRef::new(EntityRef::new(
                WorldEpoch::new(rideable.entity().epoch().get() + 1),
                rideable.entity().id(),
            ));
            assert_eq!(stale.kind(game), None);
            assert!(!stale.set_flipped(game, false));
            assert_eq!(stale.max_speed(game), None);
        });
    }
}
