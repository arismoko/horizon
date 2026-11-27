#![cfg(test)]
#[cfg(test)]
#[path = "entity/restore_tests.rs"]
pub(crate) mod restore_tests;

#[cfg(test)]
#[path = "entity/taken_pass_tests.rs"]
pub(crate) mod taken_pass_tests;

#[cfg(test)]
pub(crate) mod vehicle_contact_hurt_tests {
    use mod_api::{self, BlockPart, Hooks, ModEntry, Registrar, RegistrationContext};
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_core::nbt::{Compound, Tag};
    use sunrise_gameplay::game::ServerGame;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::boats_common::Boat;
    use sunrise_gameplay_common::game::commands_common::Target;
    use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart};
    use sunrise_level::chunk::Chunk;

    fn boat(id: u32) -> Boat {
        Boat::new(id, Arc::from("oak"), Vec3::new(8.5, 64.0, 8.5), 0.0)
    }

    fn cart(id: u32) -> Minecart {
        Minecart::new(id, CartKind::Rideable, Vec3::new(8.5, 64.0, 8.5))
    }

    #[derive(Clone)]
    struct HullHeat {
        damage_type: mod_api::DamageTypeKey,
        amount: f32,
    }

    #[mod_api::sunrise_part]
    impl BlockPart for HullHeat {
        fn step_on(&self, _: mod_api::LevelRandomSide<'_>, event: &mut mod_api::StepOnEvent<'_>) {
            event.write_entity(mod_api::EntityWrite::Hurt {
                damage_type: self.damage_type,
                amount: self.amount,
            });
        }
    }

    #[derive(Clone)]
    struct HullHeatBlocks {
        amount: f32,
    }

    #[mod_api::__subscriptions]
    impl Hooks for HullHeatBlocks {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let stone = r.block_key("minecraft:stone");
            let damage_type = r.damage_type_key("minecraft:cactus");
            r.block_part(
                stone,
                HullHeat {
                    damage_type,
                    amount: self.amount,
                },
            );
        }
    }

    /// A stone floor with a `step_on` Part that hurts the stepping body by
    /// `amount` (ten times the hull wear, as `VehicleEntity.hurtServer` does).
    fn with_hull_heat(amount: f32, test: impl FnOnce(&mut ServerGame<'_>)) {
        let mods: mod_api::Mods = vec![ModEntry::new(
            "hull_heat",
            Box::new(HullHeatBlocks { amount }),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            sunrise_gameplay_common::game::native_hooks::install_block_contact_hooks();
            with_game(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(8, 63, 8, stone).unwrap();
                test(game);
            });
        });
    }

    /// A boat's `step_on` contact reaches the stored hull through its real
    /// tick: the wear and hurt wobble survive the ticked copy's store-back.
    #[test]
    #[ignore = "daily"]
    fn a_surviving_contact_hurt_reaches_a_boat_through_its_tick() {
        with_hull_heat(1.0, |game| {
            let mut b = boat(1);
            b.ground.on_ground = true;
            game.boats.push(b);
            game.tick_boats();
            assert_eq!(game.boats.len(), 1);
            assert!(
                game.boats[0].damage > 0.0,
                "the contact wears the stored hull"
            );
            assert_eq!(game.boats[0].hurt_time, 10);
        });
    }

    /// A cart's `step_on` contact reaches the stored cart through its real tick.
    #[test]
    #[ignore = "daily"]
    fn a_surviving_contact_hurt_reaches_a_cart_through_its_tick() {
        with_hull_heat(1.0, |game| {
            let mut c = cart(2);
            c.ground.on_ground = true;
            game.carts.push(c);
            game.tick_carts();
            assert_eq!(game.carts.len(), 1);
            assert!(
                game.carts[0].damage > 0.0,
                "the contact wears the stored cart"
            );
            assert_eq!(game.carts[0].hurt_time, 10);
        });
    }

    /// A breaking contact hurt removes the boat and stops the rest of its
    /// tick: no later pass indexes the removed row.
    #[test]
    #[ignore = "daily"]
    fn a_breaking_contact_hurt_removes_a_boat_through_its_tick() {
        with_hull_heat(5.0, |game| {
            let mut b = boat(1);
            b.ground.on_ground = true;
            game.boats.push(b);
            game.tick_boats();
            assert!(game.boats.is_empty());
        });
    }

    /// A hit that survives the first `step_on` and breaks the boat on the
    /// second pass must stop the tick before the contacts pass indexes the
    /// removed row (the breaking-boat test above stops on the first pass).
    #[test]
    #[ignore = "daily"]
    fn a_second_pass_breaking_hurt_removes_a_boat_without_a_later_pass() {
        with_hull_heat(2.5, |game| {
            let mut b = boat(1);
            b.ground.on_ground = true;
            game.boats.push(b);
            game.tick_boats();
            assert!(game.boats.is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_breaking_contact_hurt_removes_a_cart_through_its_tick() {
        with_hull_heat(5.0, |game| {
            let mut c = cart(2);
            c.ground.on_ground = true;
            game.carts.push(c);
            game.tick_carts();
            assert!(game.carts.is_empty());
        });
    }

    /// The same contact on an `Invulnerable` hull is refused before
    /// `hurtServer`: no wear, no hurt wobble, nothing to sound.
    #[test]
    #[ignore = "daily"]
    fn an_invulnerable_vehicle_refuses_a_contact_hurt() {
        with_game(|game| {
            game.boats.push(boat(1));
            game.carts.push(cart(2));
            for id in [1, 2] {
                let mut compound = Compound::new();
                compound.insert("Invulnerable".into(), Tag::Byte(1));
                game.persist.raw.insert(id, Tag::Compound(compound));
            }
            assert!(!game.vehicle_contact_hurt(Target::Boat(1), "minecraft:cactus", 2.0));
            assert!(!game.vehicle_contact_hurt(Target::Cart(2), "minecraft:cactus", 2.0));
            assert_eq!(game.boats[0].damage, 0.0);
            assert_eq!(game.carts[0].damage, 0.0);
            assert_eq!(game.boats[0].hurt_time, 0);
            assert_eq!(game.carts[0].hurt_time, 0);
        });
    }
}
