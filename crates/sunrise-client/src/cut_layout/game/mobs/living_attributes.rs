#![cfg(test)]
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_world::attributes::{Modifier, Operation};
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::server_side::living_attributes::tests::modifier;

    use sunrise_gameplay_common::game::server_side::living_attributes::tests::saved_ids;

    use sunrise_gameplay_common::game::server_side::living_attributes::tests::stack;

    use sunrise_gameplay::game::server_side::living_attributes::tests::living;

    use sunrise_gameplay::game::server_side::living_attributes::tests::read;

    use sunrise_gameplay::game::server_side::living_attributes::tests::spawn;

    use super::*;

    use crate::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;

    fn detect_test_mob_equipment(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        let items = match (
            &game.common.server_items,
            game.server.world_tables.0.as_ref(),
        ) {
            (Some(items), _) => &**items,
            (None, Some(tables)) => &tables.items,
            (None, None) => game.common.res.items(),
        };
        assert_eq!(
            sunrise_sim::mobs::finalize::detect_equipment_updates(
                &mut game.common.mobs[0],
                &game.common.motion.attributes,
                items,
                || false,
            ),
            None
        );
    }

    use sunrise_core::geometry::Vec3;

    use sunrise_items::equipment::{CHEST, HEAD, MAINHAND};

    use sunrise_items::items::ItemStack;

    use sunrise_sim::mobs::Mob;

    use mod_api::{AttributeOperation, Identifier, ModifierLifetime};

    /// A held stack with its own `attribute_modifiers` hits by them, not by
    /// its item type's defaults (#941).
    #[test]
    #[ignore = "daily"]
    fn a_changed_held_stack_hits_by_its_own_modifiers() {
        use mod_api::EquipmentSlot;
        use sunrise_core::nbt::Tag;
        use sunrise_sim::mobs::gear;
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            spawn(game);
            let zombie = living(game, 12);
            let attack = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                gear::attack_damage(
                    game.mobs.iter().find(|m| m.id == 12).unwrap(),
                    Some(game.res.items()),
                )
            };
            assert_eq!(attack(game), 3.0, "a zombie's own attack damage");
            let mut own = ItemStack::new("minecraft:diamond_sword", 1);
            own.data_mut().set_other(
                "minecraft:attribute_modifiers",
                Some(Tag::List(vec![Tag::compound([
                    ("type", Tag::String("minecraft:attack_damage".into())),
                    ("id", Tag::String("test:dull".into())),
                    ("amount", Tag::Double(1.0f64.to_bits())),
                    ("operation", Tag::String("add_value".into())),
                    ("slot", Tag::String("mainhand".into())),
                ])])),
            );
            assert!(
                game.living_set_equipment(zombie, EquipmentSlot::MainHand, Some(own)),
                "the write takes"
            );
            assert_eq!(attack(game), 4.0, "the stack's own +1, not the sword's +6");
            assert!(game.living_set_equipment(
                zombie,
                EquipmentSlot::MainHand,
                stack("minecraft:diamond_sword")
            ));
            assert_eq!(attack(game), 9.0, "the plain sword's own +6");
            assert!(game.living_set_equipment(zombie, EquipmentSlot::MainHand, None));
            assert_eq!(attack(game), 3.0);
        });
    }
    /// The mob's attribute packet is its live map's instances (#941): base
    /// and each modifier once — the equipment's own rows and `/attribute`'s
    /// writes among them — with no modifier applied twice, and no instance
    /// for an attribute nothing created.
    #[test]
    #[ignore = "daily"]
    fn the_mob_attribute_packet_is_the_live_maps_instances() {
        use mod_api::EquipmentSlot;
        use sunrise_core::nbt::Tag;
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            spawn(game);
            let zombie = living(game, 12);
            let mut own = ItemStack::new("minecraft:diamond_chestplate", 1);
            own.data_mut().set_other(
                "minecraft:attribute_modifiers",
                Some(Tag::List(vec![Tag::compound([
                    ("type", Tag::String("minecraft:armor".into())),
                    ("id", Tag::String("test:thin".into())),
                    ("amount", Tag::Double(4.0f64.to_bits())),
                    ("operation", Tag::String("add_value".into())),
                    ("slot", Tag::String("chest".into())),
                ])])),
            );
            assert!(game.living_set_equipment(zombie, EquipmentSlot::Chest, Some(own)));
            // A finalize write, whose rebuild keeps the equipment's rows.
            assert!(zombie.set_attribute_base(game, "minecraft:max_health", 25.0));
            let mut synced = Vec::new();
            let mob = game.mobs[0].clone();
            game.mob_attributes(
                &mob,
                &mut sunrise_gameplay::game::server_side::lan_attributes::SpeedByType::default(),
                &mut synced,
            );
            // Each attribute once.
            let mut ids: Vec<_> = synced.iter().map(|s| s.attribute).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), synced.len(), "{synced:?}");
            let of = |name: &str| {
                let id = game.motion.attributes.id(name).unwrap().registry_id();
                synced.iter().find(move |s| s.attribute == id)
            };
            // Armor: the zombie's base and only the stack's own row.
            let armor = of("minecraft:armor").unwrap();
            assert_eq!(armor.base, 2.0);
            assert_eq!(
                armor.modifiers,
                [sunrise_protocol::packets::play::AttributeModifier {
                    id: "test:thin".into(),
                    amount: 4.0,
                    operation: 0,
                }]
            );
            // The written max health base, as the map holds it.
            let health = of("minecraft:max_health").unwrap();
            assert_eq!(health.base, 25.0);
            assert!(health.modifiers.is_empty());
            // Movement speed: the instance every mob's first step creates.
            let speed = of("minecraft:movement_speed").unwrap();
            assert!(speed.base > 0.0, "{speed:?}");
            assert!(speed.modifiers.is_empty());
            // Nothing else: an attribute nothing created sends no instance.
            assert_eq!(synced.len(), 3, "{synced:?}");
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_base_write_preserves_covered_for_damage_and_attribute_sync() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let kind = game
                .res
                .mob_types()
                .get("minecraft:shulker")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let catalog = game.motion.attributes.clone();
            let m = &mut game.mobs[0];
            // The permanent row installed by Shulker.setRawPeekAmount(0).
            m.extra
                .live
                .edit(
                    &catalog,
                    &m.kind.id,
                    "minecraft:armor",
                    true,
                    Vec::new,
                    |value, observer| {
                        value.add_permanent(
                            Modifier::new("minecraft:covered", 20.0, Operation::AddValue).unwrap(),
                            observer,
                        )
                    },
                )
                .unwrap();
            let saved = sunrise_sim::entity_nbt::mob_to_nbt(
                &game.mobs[0],
                [0; 4],
                None,
                None,
                |_| None,
                &Default::default(),
            );
            let loaded_id = game.load_mob(saved.clone(), None).unwrap();
            let loaded = game.mobs.iter().find(|m| m.id == loaded_id).unwrap();
            assert_eq!(sunrise_sim::mobs::armor::values(loaded), (20.0, 0.0));
            game.apply_entity_nbt(Target::Mob(12), saved).unwrap();
            assert_eq!(sunrise_sim::mobs::armor::values(&game.mobs[0]), (20.0, 0.0));
            let shulker = living(game, 12);
            assert!(shulker.set_attribute_base(game, "minecraft:max_health", 40.0));
            // No handle read in between: damage must not need a read to restore the map.
            assert_eq!(sunrise_sim::mobs::armor::values(&game.mobs[0]), (20.0, 0.0));
            let mut synced = Vec::new();
            let m = game.mobs[0].clone();
            game.mob_attributes(
                &m,
                &mut sunrise_gameplay::game::server_side::lan_attributes::SpeedByType::default(),
                &mut synced,
            );
            let armor_id = catalog.id("minecraft:armor").unwrap().registry_id();
            let armor = synced.iter().find(|row| row.attribute == armor_id).unwrap();
            assert_eq!(armor.base, 0.0);
            assert_eq!(
                armor.modifiers,
                [sunrise_protocol::packets::play::AttributeModifier {
                    id: "minecraft:covered".into(),
                    amount: 20.0,
                    operation: 0,
                }]
            );
        });
    }
    /// The armor a hit reads follows the live map at once (#941): a stack a
    /// mod or `/item` changed protects by its own `attribute_modifiers`, a
    /// `/attribute` write takes effect without a tick, a horse's body piece
    /// armors it by its stack's rows, and the save keeps a written base and
    /// modifier.
    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn a_hits_armor_follows_the_live_map_at_once() {
        use AttributeOperation::*;
        use ModifierLifetime::*;
        use mod_api::EquipmentSlot;
        use sunrise_core::nbt::Tag;
        use sunrise_sim::mobs::{Damage, DamageKind, armor};
        with_game(|game| {
            spawn(game);
            let values = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                armor::values(game.mobs.iter().find(|m| m.id == 12).unwrap())
            };
            let zombie = living(game, 12);
            // The zombie's own armor is 2, the diamond chestplate's 8 and 2
            // toughness: the write through the handle collects at once.
            assert!(game.living_set_equipment(
                zombie,
                EquipmentSlot::Chest,
                stack("minecraft:diamond_chestplate")
            ));
            assert_eq!(values(game), (10.0, 2.0));
            // A chestplate with its own `attribute_modifiers` protects by
            // them, not by the diamond one's.
            let mut own = ItemStack::new("minecraft:diamond_chestplate", 1);
            own.data_mut().set_other(
                "minecraft:attribute_modifiers",
                Some(Tag::List(vec![Tag::compound([
                    ("type", Tag::String("minecraft:armor".into())),
                    ("id", Tag::String("test:thin".into())),
                    ("amount", Tag::Double(4.0f64.to_bits())),
                    ("operation", Tag::String("add_value".into())),
                    ("slot", Tag::String("chest".into())),
                ])])),
            );
            assert!(game.living_set_equipment(zombie, EquipmentSlot::Chest, Some(own)));
            assert_eq!(values(game), (6.0, 0.0));
            // A `/attribute` base write lands at once, on top of the stack's.
            assert!(zombie.set_attribute_base(game, "minecraft:armor", 5.0));
            assert_eq!(values(game), (9.0, 0.0));
            // Unequipped, the base and a permanent modifier are all that is
            // left, and the damage boundary reads the same value.
            assert!(zombie.add_attribute_modifier(
                game,
                "minecraft:armor",
                modifier("test:plated", 3.0, AddValue),
                Permanent
            ));
            assert!(game.living_set_equipment(zombie, EquipmentSlot::Chest, None));
            assert_eq!(values(game), (8.0, 0.0));
            let level = sunrise_gameplay::game::server_side::entity_server::damage_level_of!(
                game,
                &sunrise_sim::health::NO_ATTACK,
                no_push,
                None
            );
            let m = game.mobs.iter_mut().find(|m| m.id == 12).unwrap();
            m.health = 20.0;
            assert!(m.damage(level, 4.0, Damage::of(DamageKind::Cactus)));
            // CombatRules.getDamageAfterAbsorb: (8 - 4/2) of 25 absorbs.
            let expected = 4.0 * (1.0 - 6.0 / 25.0);
            assert!((f64::from(m.health) - (20.0 - expected)).abs() < 1e-3);
            // The written base and modifier survive the map's own save and
            // load; the first read builds the loaded map again, as a world
            // load's first tick or read does.
            let mut list = Vec::new();
            game.mobs[0].extra.live.save_into(&mut list);
            let mut fresh = sunrise_sim::living_attributes::LivingAttributes::default();
            fresh.load(Some(&Tag::List(list)), |_| true);
            game.mobs[0].extra.live = fresh;
            read(game, 12, "minecraft:armor");
            assert_eq!(values(game), (8.0, 0.0));
            // A horse's body piece armors it by its stack's rows.
            let horse_kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
            // The fixture dragon owns IDs 14 through 22, including its parts.
            game.mobs.push(Mob::new(40, horse_kind, Vec3::ZERO, 0.0));
            let horse = living(game, 40);
            assert!(game.living_set_equipment(
                horse,
                EquipmentSlot::Body,
                stack("minecraft:diamond_horse_armor")
            ));
            let armored = armor::values(game.mobs.iter().find(|m| m.id == 40).unwrap());
            assert!(
                armored.0 >= 1.0,
                "the body piece armors the horse: {armored:?}"
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_zombie_reads_its_defaults() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            spawn(game);
            assert_eq!(
                read(game, 12, "minecraft:max_health"),
                (Some(20.0), Some(20.0))
            );
            assert_eq!(
                read(game, 12, "minecraft:attack_damage"),
                (Some(3.0), Some(3.0))
            );
            // Not an attribute of an armor stand.
            assert_eq!(read(game, 13, "minecraft:attack_damage"), (None, None));
        });
    }
    /// A read of a mob that wears nothing builds its map once and keeps it,
    /// so the next reads allocate nothing.
    #[test]
    #[ignore = "daily"]
    fn a_read_builds_the_map_once() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            spawn(game);
            assert!(game.mobs[0].extra.live.built().is_none());
            read(game, 12, "minecraft:max_health");
            let first: *const _ = game.mobs[0].extra.live.built().unwrap();
            read(game, 12, "minecraft:attack_damage");
            read(game, 12, "minecraft:max_health");
            let again: *const _ = game.mobs[0].extra.live.built().unwrap();
            assert!(std::ptr::eq(first, again));
            // A changed base drops it; the next read builds it from the new base.
            game.mobs[0].extra.live.invalidate();
            assert!(game.mobs[0].extra.live.built().is_none());
            assert_eq!(
                read(game, 12, "minecraft:max_health"),
                (Some(20.0), Some(20.0))
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_held_sword_raises_the_attack_damage_until_it_is_put_away() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            spawn(game);
            let damage = "minecraft:attack_damage";
            game.mobs[0].set_equipment_stack(MAINHAND, stack("minecraft:diamond_sword"));
            detect_test_mob_equipment(game);
            // +6 from the sword's `base_attack_damage` modifier; the base stays.
            assert_eq!(read(game, 12, damage), (Some(9.0), Some(3.0)));
            // The same item again changes nothing (no duplicate modifier).
            detect_test_mob_equipment(game);
            assert_eq!(read(game, 12, damage), (Some(9.0), Some(3.0)));
            // Another sword takes the first one's modifier off before its own.
            game.mobs[0].set_equipment_stack(MAINHAND, stack("minecraft:netherite_sword"));
            detect_test_mob_equipment(game);
            assert_eq!(read(game, 12, damage), (Some(10.0), Some(3.0)));
            game.mobs[0].set_equipment_stack(MAINHAND, None);
            detect_test_mob_equipment(game);
            assert_eq!(read(game, 12, damage), (Some(3.0), Some(3.0)));
            // A sword on the head is in no slot its modifiers name.
            game.mobs[0].set_equipment_stack(HEAD, stack("minecraft:diamond_sword"));
            detect_test_mob_equipment(game);
            assert_eq!(read(game, 12, damage), (Some(3.0), Some(3.0)));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_stand_and_the_dragon_wear_armor_modifiers() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            spawn(game);
            let armor = "minecraft:armor";
            game.armor_stands[0]
                .equipment
                .set(HEAD, stack("minecraft:diamond_helmet"));
            game.dragon
                .as_mut()
                .unwrap()
                .equipment
                .set(CHEST, stack("minecraft:diamond_chestplate"));
            game.refresh_stand_and_dragon_equipment_attributes();
            assert_eq!(read(game, 13, armor), (Some(3.0), Some(0.0)));
            assert_eq!(read(game, 14, armor), (Some(8.0), Some(0.0)));
            game.armor_stands[0].equipment.set(HEAD, None);
            game.refresh_stand_and_dragon_equipment_attributes();
            assert_eq!(read(game, 13, armor), (Some(0.0), Some(0.0)));
            assert_eq!(read(game, 14, armor), (Some(8.0), Some(0.0)));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_stale_handle_reads_none() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            spawn(game);
            let living = game.living(game.entity(12).unwrap()).unwrap();
            assert_eq!(
                living.attribute_value(game, "minecraft:max_health"),
                Some(20.0)
            );
            game.mobs.clear();
            assert_eq!(living.attribute_value(game, "minecraft:max_health"), None);
            assert_eq!(living.attribute_base(game, "minecraft:max_health"), None);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_saved_base_and_modifier_are_read_and_saved_again() {
        use sunrise_core::nbt::Tag;
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            spawn(game);
            let saved = Tag::List(vec![Tag::compound([
                ("id", Tag::String("minecraft:max_health".into())),
                ("base", Tag::Double(40.0f64.to_bits())),
                (
                    "modifiers",
                    Tag::List(vec![Tag::compound([
                        ("id", Tag::String("test:bonus".into())),
                        ("amount", Tag::Double(0.5f64.to_bits())),
                        ("operation", Tag::String("add_multiplied_base".into())),
                    ])]),
                ),
            ])]);
            game.armor_stands[0].live.load(Some(&saved), |_| true);
            assert_eq!(
                read(game, 13, "minecraft:max_health"),
                (Some(60.0), Some(40.0))
            );
            let mut list = Vec::new();
            game.armor_stands[0].live.save_into(&mut list);
            assert_eq!(Tag::List(list), saved);
        });
    }
    /// The live map takes a held stack's own `attribute_modifiers`, and
    /// another stack of the same item with other components is a change.
    #[test]
    #[ignore = "daily"]
    fn a_stacks_own_modifiers_replace_its_items() {
        use sunrise_core::nbt::Tag;
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            spawn(game);
            let damage = "minecraft:attack_damage";
            let mut own = ItemStack::new("minecraft:diamond_sword", 1);
            own.data_mut().set_other(
                "minecraft:attribute_modifiers",
                Some(Tag::List(vec![Tag::compound([
                    ("type", Tag::String(damage.into())),
                    ("id", Tag::String("test:dull".into())),
                    ("amount", Tag::Double(1.0f64.to_bits())),
                    ("operation", Tag::String("add_value".into())),
                    ("slot", Tag::String("mainhand".into())),
                ])])),
            );
            game.mobs[0].set_equipment_stack(MAINHAND, Some(own));
            detect_test_mob_equipment(game);
            // +1 from its own list, not the sword's +6.
            assert_eq!(read(game, 12, damage), (Some(4.0), Some(3.0)));
            game.mobs[0].set_equipment_stack(MAINHAND, stack("minecraft:diamond_sword"));
            detect_test_mob_equipment(game);
            assert_eq!(read(game, 12, damage), (Some(9.0), Some(3.0)));
        });
    }
    /// A mob's gear knockback resistance reads its stacks once, keeps what it
    /// read while the same stacks are worn, and reads them again when one
    /// changes: here a chestplate with its own list.
    #[test]
    #[ignore = "daily"]
    fn gear_knockback_is_read_again_only_when_the_gear_changes() {
        use sunrise_core::nbt::Tag;
        with_game(|game| {
            spawn(game);
            let read_at = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                let m = &game.mobs[0];
                let gear = m.extra.gear_knockback.as_deref();
                (
                    gear.map(std::ptr::from_ref),
                    gear.map(|_| m.knockback_resistance()),
                )
            };
            let close = |value: Option<f64>, to: f64| value.is_some_and(|v| (v - to).abs() < 1e-6);
            game.mobs[0].set_equipment_stack(CHEST, stack("minecraft:netherite_chestplate"));
            game.refresh_gear_knockback_resistance();
            let (first, value) = read_at(game);
            assert!(first.is_some() && close(value, 0.1), "{value:?}");
            game.refresh_gear_knockback_resistance();
            assert_eq!(read_at(game).0, first, "the same gear is not read again");
            let mut own = ItemStack::new("minecraft:netherite_chestplate", 1);
            own.data_mut().set_other(
                "minecraft:attribute_modifiers",
                Some(Tag::List(vec![Tag::compound([
                    ("type", Tag::String("minecraft:knockback_resistance".into())),
                    ("id", Tag::String("test:anchor".into())),
                    ("amount", Tag::Double(0.5f64.to_bits())),
                    ("operation", Tag::String("add_value".into())),
                    ("slot", Tag::String("chest".into())),
                ])])),
            );
            game.mobs[0].set_equipment_stack(CHEST, Some(own));
            game.refresh_gear_knockback_resistance();
            let (second, value) = read_at(game);
            assert!(second != first && close(value, 0.5), "{value:?}");
            game.mobs[0].set_equipment_stack(CHEST, None);
            game.refresh_gear_knockback_resistance();
            assert_eq!(read_at(game), (None, None));
        });
    }
    /// A mob's movement speed is kept where its gameplay reads it: a mod's
    /// permanent modifier is saved, a transient one is not, both count at once.
    #[test]
    #[ignore = "daily"]
    fn a_mods_modifiers_on_a_mobs_speed_reach_its_gameplay() {
        use AttributeOperation::*;
        use ModifierLifetime::*;
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            spawn(game);
            let speed = "minecraft:movement_speed";
            let zombie = living(game, 12);
            let (_, Some(base)) = read(game, 12, speed) else {
                panic!("a zombie has a movement speed");
            };
            assert!(zombie.add_attribute_modifier(
                game,
                speed,
                modifier("test:fast", 1.0, AddMultipliedBase),
                Permanent
            ));
            // The id is taken (vanilla throws).
            assert!(!zombie.add_attribute_modifier(
                game,
                speed,
                modifier("test:fast", 2.0, AddValue),
                Transient
            ));
            assert!(zombie.add_attribute_modifier(
                game,
                speed,
                modifier("test:boost", 0.5, AddMultipliedTotal),
                Transient
            ));
            let value = base * 2.0 * 1.5;
            assert_eq!(read(game, 12, speed), (Some(value), Some(base)));
            assert_eq!(game.mobs[0].attrs.speed, value);
            let saved = sunrise_sim::entity_nbt::attribute_entries(&game.mobs[0]);
            assert_eq!(saved_ids(saved.iter().map(|(_, e)| e)), ["test:fast"]);
            // Replacing takes the new lifetime: no longer saved.
            assert!(zombie.replace_attribute_modifier(
                game,
                speed,
                modifier("test:fast", 1.0, AddMultipliedBase),
                Transient
            ));
            let saved = sunrise_sim::entity_nbt::attribute_entries(&game.mobs[0]);
            assert!(saved_ids(saved.iter().map(|(_, e)| e)).is_empty());
            assert_eq!(read(game, 12, speed), (Some(value), Some(base)));
            let boost = Identifier::parse("test:boost").unwrap();
            assert!(zombie.remove_attribute_modifier(game, speed, &boost));
            assert!(!zombie.remove_attribute_modifier(game, speed, &boost));
            assert_eq!(read(game, 12, speed), (Some(base * 2.0), Some(base)));
            assert!(zombie.set_attribute_base(game, speed, 0.5));
            assert_eq!(read(game, 12, speed), (Some(1.0), Some(0.5)));
            assert_eq!(game.mobs[0].attrs.speed, 1.0);
        });
    }
    /// An attribute only the live map keeps keeps a mod's writes when the map
    /// is built again, and saves the permanent ones.
    #[test]
    #[ignore = "daily"]
    fn a_mods_writes_outlive_a_rebuilt_map() {
        use AttributeOperation::*;
        use ModifierLifetime::*;
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            spawn(game);
            let gravity = "minecraft:gravity";
            let zombie = living(game, 12);
            assert!(zombie.set_attribute_base(game, gravity, 0.1));
            assert!(zombie.add_attribute_modifier(
                game,
                gravity,
                modifier("test:heavy", 1.0, AddMultipliedBase),
                Permanent
            ));
            assert!(zombie.add_attribute_modifier(
                game,
                gravity,
                modifier("test:float", -0.05, AddValue),
                Transient
            ));
            let both = Some((0.1 - 0.05) * 2.0);
            assert_eq!(read(game, 12, gravity), (both, Some(0.1)));
            game.mobs[0].extra.live.invalidate();
            assert_eq!(read(game, 12, gravity), (both, Some(0.1)));
            let mut list = Vec::new();
            game.mobs[0].extra.live.save_into(&mut list);
            assert_eq!(saved_ids(&list), ["test:heavy"]);
            let float = Identifier::parse("test:float").unwrap();
            assert!(zombie.remove_attribute_modifier(game, gravity, &float));
            game.mobs[0].extra.live.invalidate();
            assert_eq!(read(game, 12, gravity), (Some(0.2), Some(0.1)));
            // A change to `finalize`'s store rebuilds the map without losing them.
            sunrise_sim::mobs::finalize::set_base(
                &mut game.mobs[0],
                sunrise_sim::mobs::finalize::Attr::MaxHealth,
                30.0,
            );
            assert_eq!(read(game, 12, gravity), (Some(0.2), Some(0.1)));
            // The write is where `/attribute` reads it and the sync sends it.
            let key = game.motion.attributes.id(gravity).unwrap();
            let map = game.generic_attributes(Target::Mob(12)).unwrap();
            assert_eq!(map.value(key).unwrap(), 0.2);
            let mut synced = Vec::new();
            let zombie_mob = game.mobs[0].clone();
            game.mob_attributes(
                &zombie_mob,
                &mut sunrise_gameplay::game::server_side::lan_attributes::SpeedByType::default(),
                &mut synced,
            );
            let registry = key.registry_id();
            let sent = synced.iter().find(|s| s.attribute == registry).unwrap();
            assert_eq!(sent.base, 0.1);
            assert_eq!(
                sent.modifiers
                    .iter()
                    .map(|m| m.id.as_str())
                    .collect::<Vec<_>>(),
                ["test:heavy"]
            );
            // A mob's armor goes to its armor map, which its damage reads.
            assert!(zombie.set_attribute_base(game, "minecraft:armor", 10.0));
            assert_eq!(sunrise_sim::mobs::armor::values(&game.mobs[0]).0, 10.0);
            assert_eq!(read(game, 12, "minecraft:armor").1, Some(10.0));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_stand_the_dragon_and_a_player_take_writes() {
        use AttributeOperation::*;
        use ModifierLifetime::*;
        use sunrise_sim::motion::PlayerMotion;
        with_game(|game| {
            spawn(game);
            let health = "minecraft:max_health";
            let stand = living(game, 13);
            assert!(stand.set_attribute_base(game, health, 4.0));
            assert_eq!(read(game, 13, health), (Some(4.0), Some(4.0)));
            // Not an attribute of an armor stand; a non-finite amount.
            let damage = modifier("test:x", 1.0, AddValue);
            assert!(!stand.add_attribute_modifier(
                game,
                "minecraft:attack_damage",
                damage,
                Permanent
            ));
            assert!(!stand.add_attribute_modifier(
                game,
                health,
                modifier("test:x", f64::NAN, AddValue),
                Permanent
            ));
            // `/attribute` refuses the dragon, and nothing reads its map yet.
            let dragon = living(game, 14);
            assert!(!dragon.add_attribute_modifier(
                game,
                health,
                modifier("test:tough", 100.0, AddValue),
                Transient
            ));
            assert_eq!(read(game, 14, health), (Some(200.0), Some(200.0)));

            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            // The dragon owns its multipart IDs above 14; keep the fixture
            // player outside that range and reserve it against later spawns.
            game.server.next_entity = game.server.next_entity.max(100);
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 100, 123);
            let player = living(game, 100);
            assert!(player.add_attribute_modifier(
                game,
                health,
                modifier("test:hearty", 0.5, AddMultipliedBase),
                Permanent
            ));
            assert_eq!(read(game, 100, health), (Some(30.0), Some(20.0)));
            let hearty = Identifier::parse("test:hearty").unwrap();
            assert!(player.remove_attribute_modifier(game, health, &hearty));
            assert_eq!(read(game, 100, health), (Some(20.0), Some(20.0)));
            // A stale handle writes nothing.
            let zombie = living(game, 12);
            game.mobs.clear();
            assert!(!zombie.set_attribute_base(game, "minecraft:gravity", 1.0));
        });
    }
    /// Plates every chestplate with +0.4 knockback resistance and no armor
    /// toughness (`item_attribute_modifier`).
    struct Plated;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Plated {
        fn item_attribute_modifier(&self, event: &mut mod_api::ItemAttributeModifierEvent<'_>) {
            if !mod_api::__private::item_key_id(event.stack.item())
                .unwrap()
                .ends_with("_chestplate")
            {
                return;
            }
            let id = |text: &str| Identifier::parse(text).unwrap();
            event.modifiers.add(
                id("minecraft:knockback_resistance"),
                modifier("test:plated", 0.4, AttributeOperation::AddValue),
                mod_api::EquipmentSlotGroup::Chest,
            );
            event
                .modifiers
                .remove_all_for(&id("minecraft:armor_toughness"));
        }
    }

    /// A mod's answer to `item_attribute_modifier` is what the zombie's
    /// gameplay reads: its knockback and its live map.
    #[test]
    #[ignore = "daily"]
    fn a_mods_item_modifiers_reach_the_wearer() {
        // The stack hooks are a process-wide table: install them in a child of its own.
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::living_attributes::tests::a_mods_item_modifiers_reach_the_wearer",
            "SUNRISE_ITEM_MODIFIERS_WEARER_CHILD",
            &["plated"],
        ) {
            return;
        }
        // The game's stack snapshots, which the hook needs, as its startup installs them;
        // without this the test passes only after another test in the process installed them.
        sunrise_gameplay_common::game::api_values::install_stack_hooks();
        let mods = vec![mod_api::ModEntry::new("plated", Box::new(Plated))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                fixture_resident_column(&mut game.world, (0, 0));
                spawn(game);
                let chestplate = stack("minecraft:netherite_chestplate");
                game.mobs[0].set_equipment_stack(CHEST, chestplate);
                game.refresh_gear_knockback_resistance();
                detect_test_mob_equipment(game);
                // Netherite's own 0.1, and the mod's 0.4.
                let resistance = game.mobs[0].knockback_resistance();
                assert!((resistance - 0.5).abs() < 1e-6, "{resistance}");
                let toughness = read(game, 12, "minecraft:armor_toughness");
                assert_eq!(toughness, (Some(0.0), Some(0.0)));
                // Netherite's 8 armor still counts, on the zombie's own 2.
                assert_eq!(read(game, 12, "minecraft:armor"), (Some(10.0), Some(2.0)));
            });
        });
    }
}
