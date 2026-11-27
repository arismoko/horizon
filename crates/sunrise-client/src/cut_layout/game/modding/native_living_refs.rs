#![cfg(test)]
use mod_api::LivingRef;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::Mob;
use sunrise_sim::mobs::Who;
#[cfg(test)]
pub(crate) mod tests {

    use mod_api::AttributeModifier;
    use sunrise_gameplay_common::game::TryIntoApi;
    use sunrise_gameplay_common::game::block_props::BlockPropsGameExt2;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_living_body::NativeLivingBodyGameExt6;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;
    use sunrise_gameplay_common::game::tables::GameItems;
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt;

    use mod_api::EquipmentSlot;

    use mod_api::client::first_person::Hand;

    use sunrise_gameplay::game::commands::Target;

    use sunrise_gameplay::game::commands::items::Owner;

    use sunrise_gameplay::game::living_attributes::AttributeWrite;

    use sunrise_gameplay::game::server_side::native_living_refs::slot_name;

    use sunrise_gameplay_common::game::server_side::native_living_refs::tests::ZOMBIE_NBT;

    use sunrise_gameplay_common::game::server_side::native_living_refs::tests::ZOMBIE_UUID;

    use sunrise_gameplay::game::server_side::native_living_refs::tests::hurt_pig;

    use sunrise_gameplay::game::server_side::native_living_refs::tests::send_to_the_nether;

    use sunrise_gameplay::game::server_side::native_living_refs::tests::summon_mob;

    use super::*;

    use sunrise_core::geometry::Vec3;

    /// The mod API keeps a written base in range; vanilla's own `setBaseValue` (`/attribute base set`) does not.
    #[test]
    #[ignore = "daily"]
    fn a_mod_base_write_stays_in_range_and_a_vanilla_one_does_not() {
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            for (id, entity) in [(55, "minecraft:pig"), (56, "minecraft:slime")] {
                let ty = game.res.mob_types().get(entity).unwrap().clone();
                game.mobs.push(Mob::new(id, ty, Vec3::ZERO, 0.0));
                let living = game.living(game.entity(id).unwrap()).unwrap();
                assert!(game.living_set_attribute_base(living, "minecraft:max_health", 2000.0));
                assert_eq!(
                    game.living_attribute_base(living, "minecraft:max_health"),
                    Some(1024.0),
                    "{entity}: the mod's write"
                );
                let write = AttributeWrite::Base(2000.0);
                assert_eq!(
                    game.write_attribute(Target::Mob(id), "minecraft:max_health", write),
                    Some(true)
                );
                assert_eq!(
                    game.living_attribute_base(living, "minecraft:max_health"),
                    Some(2000.0),
                    "{entity}: /attribute base set"
                );
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn durability_handle_and_held_path_share_rng_and_damage() {
        use mod_api::ServerHost;
        use sunrise_core::random::RandomSource;
        use sunrise_sim::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let living = game.living(game.entity(20).unwrap()).unwrap();
            let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
            stack.data_mut().enchantments = vec![("minecraft:unbreaking".into(), 3)];
            game.common.inventory[game.common.selected] = Some(stack.clone());
            let random = game.world.random.clone();
            // Before the migration, damage_held called wear and added its result.
            let old_loss = game.wear(&stack, 20);
            let old_next_random = game.world.random.next_float();
            game.world.random = random.clone();
            game.damage_held(20);
            let held_result = game.inventory[game.selected].clone();
            let next_random = game.world.random.next_float();
            assert_eq!(held_result.as_ref().unwrap().damage, old_loss);
            assert_eq!(next_random, old_next_random);
            game.world.random = random;
            game.common.inventory[game.common.selected] = Some(stack);
            let host: &mut dyn ServerHost = game;
            assert!(
                living
                    .hurt_and_break(host, EquipmentSlot::MainHand, 20)
                    .is_none()
            );
            assert_eq!(game.inventory[game.selected], held_result);
            assert_eq!(game.world.random.next_float(), next_random);

            let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
            stack.damage = 10;
            game.common.inventory[game.common.selected] = Some(stack.clone());
            let mut repair_random = game.world.random.clone();
            assert!(
                living
                    .hurt_and_break(game, EquipmentSlot::MainHand, -4)
                    .is_none()
            );
            assert_eq!(game.world.random.next_float(), repair_random.next_float());
            assert_eq!(game.inventory[game.selected].as_ref().unwrap().damage, 6);
            game.creative = true;
            assert!(
                living
                    .hurt_and_break(game, EquipmentSlot::MainHand, 100)
                    .is_none()
            );
            assert_eq!(game.inventory[game.selected].as_ref().unwrap().damage, 6);
            game.creative = false;
            let mut unbreakable = stack.clone();
            unbreakable.data_mut().set_other(
                "minecraft:unbreakable",
                Some(sunrise_core::nbt::Tag::Byte(1)),
            );
            game.common.inventory[game.common.selected] = Some(unbreakable.clone());
            assert!(
                living
                    .hurt_and_break(game, EquipmentSlot::MainHand, 100)
                    .is_none()
            );
            assert_eq!(game.inventory[game.selected], Some(unbreakable));
            let max = game.items().get(&stack.item).unwrap().max_damage;
            stack.damage = max - 1;
            game.common.inventory[game.common.selected] = Some(stack);
            let broken = living
                .hurt_and_break(game, EquipmentSlot::MainHand, 1)
                .unwrap();
            assert_eq!(broken.damage(), max);
            assert!(game.inventory[game.selected].is_none());

            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(21, kind, Vec3::ZERO, 0.0));
            let mob = game.living(game.entity(21).unwrap()).unwrap();
            let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
            sword.damage = max - 1;
            assert!(
                mob.set_equipment(
                    game,
                    EquipmentSlot::MainHand,
                    Some(sword.try_into_api().unwrap())
                )
                .unwrap()
            );
            let broken = mob
                .hurt_and_break(game, EquipmentSlot::MainHand, 1)
                .unwrap();
            assert_eq!(broken.damage(), max);
            assert!(
                mob.with_equipment(game, EquipmentSlot::MainHand, |_| ())
                    .is_none()
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn armor_hit_matches_old_draws_and_break_order() {
        use sunrise_core::random::RandomSource;
        use sunrise_sim::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let source = sunrise_sim::health::NO_ATTACK.player_source("minecraft:generic", 0);
            let mut armor: [Option<ItemStack>; 4] = std::array::from_fn(|_| None);
            for (i, name) in ["minecraft:diamond_helmet", "minecraft:diamond_chestplate"]
                .into_iter()
                .enumerate()
            {
                let mut stack = ItemStack::new(name, 1);
                stack.damage = game.items().get(name).unwrap().max_damage - 1;
                let sound = ["minecraft:entity.item.break", "minecraft:block.anvil.break"][i];
                stack.data_mut().set_other(
                    "minecraft:break_sound",
                    Some(sunrise_core::nbt::Tag::String(sound.into())),
                );
                armor[i] = Some(stack);
            }
            let mut leggings = ItemStack::new("minecraft:diamond_leggings", 1);
            leggings.data_mut().enchantments = vec![("minecraft:unbreaking".into(), 3)];
            armor[2] = Some(leggings);
            for guest in [false, true] {
                game.armor = armor.clone();
                game.server.lan_acting = guest.then_some(1);
                game.server.guest_equipment_breaks.clear();
                let random = game.world.random.clone();
                // Player.hurtArmor supplies FEET, LEGS, CHEST, HEAD;
                // LivingEntity.doHurtEquipment finishes hurtAndBreak (including
                // its effects/RNG) before the next slot's durability draw.
                let mut broken = Vec::new();
                for index in [3, 2, 1, 0] {
                    let Some(mut stack) = game.armor[index].clone() else {
                        continue;
                    };
                    let max = game.items().get(&stack.item).unwrap().max_damage;
                    let loss = game.wear(&stack, 1);
                    if loss == 0 {
                        continue;
                    }
                    stack.damage = stack.damage.saturating_add(loss).min(max);
                    if stack.damage >= max {
                        game.armor[index] = None;
                        game.player_equipment_broke(
                            sunrise_gameplay_common::game::armor_slot(index),
                            &stack,
                        );
                        broken.push(index);
                    } else {
                        game.armor[index] = Some(stack);
                    }
                }
                let expected_armor = game.armor.clone();
                let expected_events = game.server.guest_equipment_breaks.clone();
                let expected_random = game.world.random.next_float();
                assert_eq!(broken, [1, 0]);
                if guest {
                    assert_eq!(expected_events, [50, 49]);
                } else {
                    assert!(expected_events.is_empty());
                }
                game.armor = armor.clone();
                game.server.guest_equipment_breaks.clear();
                game.world.random = random;
                game.hurt_armor(&source, 4.0);
                assert_eq!(game.armor, expected_armor);
                assert_eq!(game.server.guest_equipment_breaks, expected_events);
                assert_eq!(game.world.random.next_float(), expected_random);
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn immutable_host_attribute_read_refreshes_live_cache() {
        use mod_api::Host;
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::motion::PlayerMotion;
        use sunrise_world::attributes::{Modifier, Operation};
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let living = game.living(game.entity(20).unwrap()).unwrap();
            let attributes = &mut game.player.as_mut().unwrap().attributes;
            let key = attributes.key("minecraft:max_health").unwrap();
            let instance = attributes.get_instance(Some(key.clone())).unwrap().unwrap();
            assert_eq!(attributes.instance(&instance).unwrap().value(), 20.0);
            attributes
                .edit(&instance, |a, o| {
                    a.add_transient(
                        Modifier::new("test:bonus", 5.0, Operation::AddValue).unwrap(),
                        o,
                    )
                })
                .unwrap();
            assert_eq!(attributes.cached_value(Some(&key)).unwrap(), 20.0);
            let host: &dyn Host = game;
            assert_eq!(
                living.attribute_base(host, "minecraft:max_health"),
                Some(20.0)
            );
            assert_eq!(
                game.player
                    .as_ref()
                    .unwrap()
                    .attributes
                    .cached_value(Some(&key))
                    .unwrap(),
                20.0
            );
            assert_eq!(
                living.attribute_value(host, "minecraft:max_health"),
                Some(25.0)
            );
            assert_eq!(living.max_health(host), Some(25.0));
            assert_eq!(living.attribute_value(host, "missing:attribute"), None);
            assert_eq!(
                game.player
                    .as_ref()
                    .unwrap()
                    .attributes
                    .cached_value(Some(&key))
                    .unwrap(),
                25.0
            );
        });
    }
    /// The reads a handle answers about an entity's attributes: the type's
    /// attribute ids, and one attribute's modifiers through the handle's own
    /// add, replace and remove writes.
    #[test]
    #[ignore = "daily"]
    fn a_handle_lists_its_attributes_and_reads_its_modifiers() {
        use mod_api::{
            AttributeOperation, AttributeOperation::*, EntityReadError, Identifier,
            ModifierLifetime::*,
        };
        use std::ops::ControlFlow;
        use sunrise_core::geometry::Vec3;
        use sunrise_gameplay::game::native_entity_refs::tests::fixture_resident_column;
        use sunrise_items::equipment::HEAD;
        use sunrise_sim::entities::ArmorStand;
        use sunrise_sim::mobs::Mob;
        use sunrise_sim::motion::PlayerMotion;
        let modifier = |id: &str, amount: f64, operation: AttributeOperation| {
            AttributeModifier::new(Identifier::parse(id).unwrap(), amount, operation)
        };
        super::super::native_entity_refs::tests::with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let zombie_kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie_kind, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            let stand = game.living(game.entity(13).unwrap()).unwrap();

            let read_names = |game: &dyn mod_api::Host, living: LivingRef| {
                let mut names = Vec::new();
                let result = living.visit_attributes(game, &mut |name| {
                    names.push(name.to_owned());
                    ControlFlow::Continue(())
                });
                assert_eq!(result, Ok(ControlFlow::Continue(())));
                names
            };
            let read_modifiers = |game: &dyn mod_api::Host,
                                  living: LivingRef,
                                  name: &str|
             -> Result<Vec<_>, EntityReadError> {
                let mut modifiers = Vec::new();
                assert_eq!(
                    living.visit_attribute_modifiers(game, name, &mut |m| {
                        modifiers.push((m.id.to_owned(), m.amount, m.operation));
                        ControlFlow::Continue(())
                    })?,
                    ControlFlow::Continue(())
                );
                Ok(modifiers)
            };
            // Zombie.createAttributes supports these; Attributes registers them in this order.
            let known = [
                "minecraft:armor",
                "minecraft:attack_damage",
                "minecraft:max_health",
                "minecraft:movement_speed",
            ];
            let names = read_names(game, zombie);
            let matching: Vec<_> = names
                .iter()
                .map(String::as_str)
                .filter(|id| known.contains(id))
                .collect();
            assert_eq!(matching, known);
            let mut first_name = Vec::new();
            assert_eq!(
                zombie.visit_attributes(game, &mut |id| {
                    first_name.push(id.to_owned());
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first_name, ["minecraft:air_drag_modifier".to_owned()]);
            assert!(
                !read_names(game, stand)
                    .iter()
                    .any(|id| id == "minecraft:attack_damage")
            );

            // A known attribute with no modifiers completes; an unknown or unsupported one refuses.
            let speed = "minecraft:movement_speed";
            assert_eq!(read_modifiers(game, zombie, speed), Ok(Vec::new()));
            assert_eq!(
                zombie.visit_attribute_modifiers(game, "missing:attribute", &mut |_| panic!(
                    "unknown attribute visited"
                )),
                Err(EntityReadError::Unavailable)
            );
            assert_eq!(
                stand.visit_attribute_modifiers(game, "minecraft:attack_damage", &mut |_| panic!(
                    "unsupported attribute visited"
                )),
                Err(EntityReadError::Unavailable)
            );

            // The add, replace and remove writes the reads follow.
            assert!(zombie.add_attribute_modifier(
                game,
                speed,
                modifier("test:fast", 1.0, AddMultipliedBase),
                Permanent
            ));
            assert!(zombie.add_attribute_modifier(
                game,
                speed,
                modifier("test:boost", 0.5, AddMultipliedTotal),
                Transient
            ));
            assert_eq!(
                read_modifiers(game, zombie, speed),
                Ok(vec![
                    ("test:fast".to_owned(), 1.0, AddMultipliedBase),
                    ("test:boost".to_owned(), 0.5, AddMultipliedTotal),
                ])
            );
            let mut first_modifier = Vec::new();
            assert_eq!(
                zombie.visit_attribute_modifiers(game, speed, &mut |m| {
                    first_modifier.push((m.id.to_owned(), m.amount, m.operation));
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(
                first_modifier,
                [("test:fast".to_owned(), 1.0, AddMultipliedBase)]
            );

            // Replacing puts the new one at the end, its lifetime changed.
            assert!(zombie.replace_attribute_modifier(
                game,
                speed,
                modifier("test:fast", 2.0, AddValue),
                Transient
            ));
            let boost = Identifier::parse("test:boost").unwrap();
            assert!(zombie.remove_attribute_modifier(game, speed, &boost));
            assert_eq!(
                read_modifiers(game, zombie, speed),
                Ok(vec![("test:fast".to_owned(), 2.0, AddValue)])
            );

            // A worn helmet's modifier is read off the stand's live map.
            game.armor_stands[0]
                .equipment
                .set(HEAD, Some(ItemStack::new("minecraft:diamond_helmet", 1)));
            game.refresh_stand_and_dragon_equipment_attributes();
            let armor = read_modifiers(game, stand, "minecraft:armor").unwrap();
            assert!(armor.iter().any(|m| m.1 == 3.0), "{armor:?}");

            // A player's own map answers the same read.
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.living(game.entity(20).unwrap()).unwrap();
            let health = "minecraft:max_health";
            assert_eq!(read_modifiers(game, player, health), Ok(Vec::new()));
            assert!(player.add_attribute_modifier(
                game,
                health,
                modifier("test:hearty", 0.5, AddMultipliedBase),
                Permanent
            ));
            assert_eq!(
                read_modifiers(game, player, health),
                Ok(vec![("test:hearty".to_owned(), 0.5, AddMultipliedBase)])
            );

            // A stale handle refuses before calling either visitor.
            game.mobs.clear();
            game.armor_stands.clear();
            assert_eq!(
                zombie.visit_attributes(game, &mut |_| panic!("stale attributes visited")),
                Err(EntityReadError::Stale)
            );
            assert_eq!(
                zombie.visit_attribute_modifiers(game, speed, &mut |_| panic!(
                    "stale modifier visited"
                )),
                Err(EntityReadError::Stale)
            );
            assert_eq!(
                stand.visit_attribute_modifiers(game, "minecraft:armor", &mut |_| panic!(
                    "stale modifier visited"
                )),
                Err(EntityReadError::Stale)
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn living_reads_answer_the_vanilla_defaults_and_a_gone_entity_reads_none() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::entities::ArmorStand;
        use sunrise_sim::mobs::Mob;
        use sunrise_sim::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
            let mut dragon =
                sunrise_gameplay_common::game::dragon_common::Dragon::new(Vec3::ZERO, 0.0, 0);
            // The eight dragon parts must not overlap the player at ID 20.
            dragon.id = 40;
            game.dragon.replace(dragon);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            // The vanilla attribute defaults: a player and a zombie 20, an
            // armor stand the 20 the game seeds it with, the dragon its 200.
            for (id, health, max, alive) in [
                (20u32, 20.0, 20.0, true),
                (12, 20.0, 20.0, true),
                (13, 20.0, 20.0, true),
                (40, 200.0, 200.0, true),
            ] {
                let living = game.living(game.entity(id).unwrap()).unwrap();
                assert_eq!(living.health(game), Some(health), "health of {id}");
                assert_eq!(living.max_health(game), Some(max), "max of {id}");
                assert_eq!(living.alive(game), Some(alive), "alive of {id}");
            }
            // A dead dragon: isAlive is health above zero.
            let dragon_handle = game.living(game.entity(40).unwrap()).unwrap();
            game.dragon.as_mut().unwrap().health = 0.0;
            assert_eq!(dragon_handle.health(game), Some(0.0));
            assert_eq!(dragon_handle.alive(game), Some(false));
            // The entities gone: every read answers None.
            game.player = None;
            game.server.lan_acting_uuid = None;
            game.server.lan_acting_entity = None;
            game.mobs[0].removed = true;
            game.armor_stands.clear();
            game.dragon.discard();
            for id in [20u32, 12, 13, 40] {
                let handle = game.entity(id).and_then(|er| game.living(er));
                assert!(handle.is_none(), "handle of {id} after it is gone");
            }
            assert_eq!(dragon_handle.health(game), None);
            assert_eq!(dragon_handle.max_health(game), None);
            assert_eq!(dragon_handle.alive(game), None);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn kill_credit_prefers_the_player_reference_and_never_falls_back_when_unresolved() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::{Damage, Mob};

        super::super::native_guest_turn_tests::with_resident_guests(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let wolf = game.res.mob_types().get("minecraft:wolf").unwrap().clone();
            game.mobs.push(Mob::new(100, pig, Vec3::ZERO, 0.0));
            let mut wolf = Mob::new(101, wolf, Vec3::ZERO, 0.0);
            wolf.extra.tame = true;
            wolf.extra.owner = Some(sunrise_sim::lan::playerdata::uuid_ints(12));
            game.mobs.push(wolf);
            game.mobs[0].ai.last_hurt_by = Some(Who::Mob(101));
            game.mobs[0].ai.last_hurt_by_time = game.mobs[0].age as i32;
            let living = game.living(game.entity(100).unwrap()).unwrap();
            assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(101));

            // A player hit followed by a mob hit keeps the player credit.
            game.as_guest(12, |game| game.remember_mob_player_credit(100))
                .unwrap();
            assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(12));
            // A tame wolf's hit also records its owner's reference.
            game.resolve_player_responsible(100, Damage::mob(101, [0.0; 3]));
            assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(12));
            {
                let value = "minecraft:the_nether".into();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.dimension = value);
            }
            assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(12));
            {
                let value = false;
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.placed = value);
            }
            assert_eq!(living.kill_credit(game), None);

            // Once the player reference expires, the mob can be credited.
            game.server.last_hurt_by_player.remove(&100);
            assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(101));
            // A mob that has left this level is no longer resolvable by its old ID.
            game.mobs[1].removed = true;
            assert_eq!(living.kill_credit(game), None);
            game.mobs[1].removed = false;
            game.mobs[0].age += 101;
            assert_eq!(living.kill_credit(game), None);
        });
    }
    /// `EntityReference.getEntity` looks through every level: the zombie that
    /// hurt the pig keeps the credit after it crossed into the Nether under a
    /// new id, even with another mob holding its UUID in the pig's level.
    #[test]
    #[ignore = "daily"]
    fn kill_credit_follows_a_mob_that_changed_dimension() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (pig, zombie) = hurt_pig(game);
            send_to_the_nether(game, zombie);
            let living = game.living(game.entity(pig).unwrap()).unwrap();
            let bystander = summon_mob(game, "minecraft:zombie", ZOMBIE_NBT);
            // What a mod reads from the pig's level, the Nether still parked.
            let credit = living.kill_credit(game).expect("credit kept");
            assert!(![zombie, bystander, pig].contains(&credit.id()));
            assert_eq!(living.last_attacker(game), Some(credit));
            assert_eq!(
                credit.kind(game),
                mod_api::__private::resolved_entity_type_key("minecraft:zombie")
            );
            assert_eq!(
                credit.uuid(game),
                Some(sunrise_sim::lan::playerdata::ints_uuid(ZOMBIE_UUID))
            );
            let nether = game.level("minecraft:the_nether");
            assert_eq!(credit.level(game), nether);
            // The same reads from a mod goal of the pig's own tick.
            let mut actor = game.mobs.iter().find(|m| m.id == pig).unwrap().clone();
            game.in_goal_view(&mut actor, |view| {
                assert_eq!(living.kill_credit(view), Some(credit));
                assert_eq!(
                    credit.kind(view),
                    mod_api::__private::resolved_entity_type_key("minecraft:zombie")
                );
                // `uuid` takes a `ServerHost`, which a goal view is not.
                assert_eq!(credit.level(view), nether);
            });
        });
    }
    /// A Nether mob already holds the zombie's UUID, so the zombie's copy is
    /// refused residency there: the credit names no one, not that other mob.
    #[test]
    #[ignore = "daily"]
    fn a_refused_arrival_hands_the_credit_to_no_other_mob() {
        use sunrise_gameplay_common::game::Dim;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (pig, zombie) = hurt_pig(game);
            game.enter_level(Dim::Nether).unwrap();
            summon_mob(game, "minecraft:zombie", ZOMBIE_NBT);
            game.enter_level(Dim::Overworld).unwrap();
            send_to_the_nether(game, zombie);
            let living = game.living(game.entity(pig).unwrap()).unwrap();
            assert_eq!(living.kill_credit(game), None);
            assert_eq!(living.last_attacker(game), None);
        });
    }
    /// `last_hurt_by_mob` keeps naming a credited mob that is in another
    /// loaded level: saved through the hundredth tick, not the next, and read
    /// back to the mob there.
    #[test]
    #[ignore = "daily"]
    fn a_saved_credit_names_a_mob_in_another_level_for_a_hundred_ticks() {
        use sunrise_core::nbt::Tag;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (pig, zombie) = hurt_pig(game);
            send_to_the_nether(game, zombie);
            let i = game.mobs.iter().position(|m| m.id == pig).unwrap();
            let mut saved = |since: i32| {
                let m = &mut game.mobs[i];
                m.age = (m.ai.last_hurt_by_time + since) as u32;
                let mut tag = Tag::Compound(Default::default());
                game.put_last_hurt_by_mob(&game.mobs[i], &mut tag);
                tag
            };
            let last = saved(100);
            assert_eq!(
                last.get("last_hurt_by_mob"),
                Some(&Tag::IntArray(ZOMBIE_UUID.to_vec()))
            );
            assert_eq!(
                last.get("ticks_since_last_hurt_by_mob"),
                Some(&Tag::Int(100))
            );
            assert_eq!(saved(101).get("last_hurt_by_mob"), None);
            let nether = game.level("minecraft:the_nether");
            for (since, credited) in [(100, true), (101, false)] {
                let nbt = format!(
                    "{{NoAI:1b,last_hurt_by_mob:[I;7,8,9,10],ticks_since_last_hurt_by_mob:{since}}}"
                );
                let read_back = summon_mob(game, "minecraft:pig", &nbt);
                game.resolve_last_hurt_by_refs();
                let living = game.living(game.entity(read_back).unwrap()).unwrap();
                let credit = living.kill_credit(game);
                assert_eq!(credit.is_some(), credited, "{since} ticks after the hurt");
                if let Some(credit) = credit {
                    assert_eq!(credit.level(game), nether);
                    assert_ne!(credit.id(), zombie);
                }
            }
        });
    }
    /// `living_knockback`'s kill credit for a push that leaves the pig's credit
    /// as it was: the zombie now in the Nether, and the sword it holds there.
    #[cfg(any())]
    #[test]
    #[ignore = "daily"]
    fn a_push_names_the_credited_mob_in_another_level_and_its_main_hand() {
        use sunrise_items::items::ItemStack;
        use sunrise_sim::health::NO_ATTACK;
        use sunrise_sim::mobs::{Damage, DamageKind};
        let mut credited = 0;
        let heard = super::super::native_knockback_tests::heard(|game, _| {
            let (pig, zombie) = hurt_pig(game);
            game.mobs
                .iter_mut()
                .find(|m| m.id == zombie)
                .unwrap()
                .extra
                .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_sword", 1)));
            send_to_the_nether(game, zombie);
            let living = game.living(game.entity(pig).unwrap()).unwrap();
            credited = living.kill_credit(game).unwrap().id() as i32;
            let plan =
                game.prepare_knockback_credit(pig as i32, None, game.server.combat.0.attack());
            let mut credit = plan.lend(&mut game.server.last_hurt_by_player);
            let i = game.mobs.iter().position(|m| m.id == pig).unwrap();
            let mob = &mut game.mobs[i];
            mob.knockback(
                sunrise_gameplay::game::server_side::entity_server::damage_level_of!(
                    game,
                    &NO_ATTACK,
                    &mut credit,
                    None
                ),
                0.4,
                1.0,
                0.0,
                &Damage::of(DamageKind::Generic),
                0.0,
            );
        });
        assert_eq!(
            heard.credits.last(),
            Some(&(credited, Some("minecraft:iron_sword".to_owned())))
        );
    }
    #[test]
    #[ignore = "daily"]
    fn a_mobs_last_attacker_names_a_mob_the_host_and_none_when_displaced() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;
        use sunrise_sim::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(12, pig, Vec3::ZERO, 0.0));
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(15, zombie, Vec3::ZERO, 0.0));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player.as_mut().unwrap().entity_id = 20;
            // The host's own turn: the uuid is acting, no guest connection.
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let pig_living = game.living(game.entity(12).unwrap()).unwrap();
            // A mob's hit names the mob's own id.
            game.mobs[0].ai.last_hurt_by = Some(Who::Mob(15));
            game.mobs[0].ai.last_hurt_by_time = game.mobs[0].age as i32;
            assert_eq!(pig_living.last_attacker(game).map(|a| a.id()), Some(15));
            // The host's hit, read in the host's own turn: the host's handle.
            game.mobs[0].ai.last_hurt_by = Some(Who::Player);
            game.server.last_hurt_by_player.insert(
                12,
                sunrise_gameplay_common::game::combat_common::PlayerCredit(123),
            );
            assert_eq!(pig_living.last_attacker(game).map(|a| a.id()), Some(20));
            // The same record inside a guest's turn: the host's displaced
            // body is out of reach, so the attacker reads None.
            game.server.lan_acting = Some(7);
            game.server.lan_acting_uuid = Some(456);
            assert_eq!(pig_living.last_attacker(game), None);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn set_health_can_store_zero() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            let living = game.living(game.entity(12).unwrap()).unwrap();
            assert!(living.set_health(game, 8.0));
            assert_eq!(living.health(game), Some(8.0));
            let accepted = living.set_health(game, 0.0);
            assert_eq!((accepted, living.health(game)), (true, Some(0.0)));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn set_health_journal_restores_nested_player_and_mob_writes() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            game.mobs[0].needs_sync = false;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player.as_mut().unwrap().entity_id = 20;
            super::super::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.living(game.entity(20).unwrap()).unwrap();
            let mob = game.living(game.entity(12).unwrap()).unwrap();
            let initial = (player.health(game), mob.health(game));
            let dirty = game.player.as_ref().unwrap().health.data.is_dirty();
            let events = game.player.as_ref().unwrap().events.len();
            game.storage_scope(|game, _| {
                assert!(player.set_health(game, 12.0));
                assert!(mob.set_health(game, 8.0));
                game.storage_scope(|game, _| {
                    assert!(player.set_health(game, 0.0));
                    assert!(mob.set_health(game, 0.0));
                    ((), false)
                });
                assert_eq!(
                    (player.health(game), mob.health(game)),
                    (Some(12.0), Some(8.0))
                );
                game.storage_scope(|game, _| {
                    assert!(player.set_health(game, 0.0));
                    assert!(mob.set_health(game, 0.0));
                    ((), true)
                });
                ((), false)
            });
            assert_eq!((player.health(game), mob.health(game)), initial);
            assert_eq!(game.player.as_ref().unwrap().health.data.is_dirty(), dirty);
            assert_eq!(game.player.as_ref().unwrap().events.len(), events);
            assert!(!game.mobs[0].needs_sync);
            assert!(!game.player.as_ref().unwrap().health.dead);
            assert!(!game.mobs[0].removed);
            game.mobs[0].needs_sync = true;
            game.storage_scope(|game, _| {
                assert!(mob.set_health(game, 0.0));
                ((), false)
            });
            assert_eq!(mob.health(game), initial.1);
            assert!(
                game.mobs[0].needs_sync,
                "abort preserves preexisting sync dirtiness"
            );
            game.storage_scope(|game, _| {
                assert!(player.set_health(game, 0.0));
                assert!(mob.set_health(game, 0.0));
                ((), true)
            });
            assert_eq!(
                (player.health(game), mob.health(game)),
                (Some(0.0), Some(0.0))
            );
            assert!(game.mobs[0].needs_sync);
            assert!(game.player.as_ref().unwrap().events.len() > events);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn set_health_journal_restores_zero_health_guest_outside_its_turn() {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let guest = game.player(12).unwrap();
            let living = LivingRef::new(guest.entity());
            let before = living.health(game).unwrap();
            game.storage_scope(|game, _| {
                assert!(living.set_health(game, before - 2.0));
                game.storage_scope(|game, _| {
                    assert!(living.set_health(game, 0.0));
                    assert_eq!(living.health(game), Some(0.0));
                    ((), false)
                });
                assert_eq!(living.health(game), Some(before - 2.0));
                game.storage_scope(|game, _| {
                    assert!(living.set_health(game, 0.0));
                    ((), true)
                });
                ((), false)
            });
            assert_eq!(living.health(game), Some(before));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn heal_uses_the_vanilla_signed_sum() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            let living = game.living(game.entity(12).unwrap()).unwrap();
            assert!(living.set_health(game, 10.0));
            assert!(living.heal(game, 2.0));
            assert_eq!(living.health(game), Some(12.0));
            let _ = living.heal(game, -4.0);
            assert_eq!(living.health(game), Some(8.0));
        });
    }
    #[cfg(any())]
    #[test]
    #[ignore = "daily"]
    fn absorbed_player_hit_still_remembers_the_living_attacker() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;
        use sunrise_sim::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(15, zombie, Vec3::ZERO, 0.0));
            let source = game
                .motion
                .health
                .damage
                .id("minecraft:mob_attack")
                .unwrap();
            let mut attackers = Vec::new();
            let mut health = Vec::new();
            for absorption in [0.0, 8.0] {
                game.server.combat.0 = Default::default();
                let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
                player.entity_id = 20;
                player
                    .set_modifier(
                        "minecraft:max_absorption",
                        "test:absorption",
                        8.0,
                        "add_value",
                        true,
                    )
                    .unwrap();
                player.set_health(absorption, true).unwrap();
                game.player = Some(player);
                sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
                game.note_attacker("minecraft:mob_attack", 15);
                let rules = game.world.health_rules;
                let level = sunrise_gameplay::game::server_side::entity_server::damage_level_of!(
                    game,
                    &sunrise_sim::health::NO_ATTACK,
                    no_push,
                    None
                );
                let player = game.player.as_mut().unwrap();
                assert!(
                    player
                        .hurt_caused_by(
                            level,
                            &game.motion.health,
                            rules,
                            source,
                            4.0,
                            None,
                            true,
                            15,
                            false
                        )
                        .unwrap()
                        .hurt
                );
                let events = std::mem::take(&mut player.events);
                game.record_combat(&events);
                let living = game.living(game.entity(20).unwrap()).unwrap();
                health.push(living.health(game));
                let host: &dyn mod_api::Host = game;
                attackers.push(living.last_attacker(host).map(|attacker| attacker.id()));
            }
            assert_eq!(
                health,
                vec![Some(16.0), Some(20.0)],
                "damage/absorption control"
            );
            assert_eq!(attackers, vec![Some(15), Some(15)]);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn raw_health_write_can_change_a_dead_entity_but_heal_cannot() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;
        use sunrise_sim::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(12, pig, Vec3::ZERO, 0.0));
            // Dying, mid death animation: not removed, but not alive.
            game.mobs[0].health = 0.0;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player.as_mut().unwrap().entity_id = 20;
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let mob_living = game.living(game.entity(12).unwrap()).unwrap();
            assert!(!mob_living.heal(game, 10.0));
            assert_eq!(mob_living.health(game), Some(0.0));
            assert!(mob_living.set_health(game, 10.0));
            assert_eq!(mob_living.health(game), Some(10.0));
            // A raw write also reaches a zero-health player.
            game.player
                .as_mut()
                .unwrap()
                .set_health(0.0, false)
                .unwrap();
            let player_living = game.living(game.entity(20).unwrap()).unwrap();
            assert!(!player_living.heal(game, 10.0));
            assert_eq!(player_living.health(game), Some(0.0));
            assert!(player_living.set_health(game, 10.0));
            assert_eq!(player_living.health(game), Some(10.0));
        });
    }
    /// `set_hand_item` writes the hand's equipment slot, as vanilla's
    /// `setItemInHand` does: a player's selected slot or off hand, a mob's
    /// main or off hand; a stale handle writes nothing.
    #[test]
    #[ignore = "daily"]
    fn set_hand_item_writes_the_hands_slot() {
        use mod_api::ServerHost;
        use sunrise_sim::mobs::Mob;
        use sunrise_sim::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            game.selected = 3;
            let player = game.living(game.entity(20).unwrap()).unwrap();
            let sword = ItemStack::new("minecraft:iron_sword", 1);
            let shield = ItemStack::new("minecraft:shield", 1);
            let host: &mut dyn ServerHost = game;
            assert!(
                player
                    .set_hand_item(host, Hand::Main, Some(sword.try_into_api().unwrap()))
                    .unwrap()
            );
            assert!(
                player
                    .set_hand_item(host, Hand::Off, Some(shield.try_into_api().unwrap()))
                    .unwrap()
            );
            assert_eq!(game.inventory[3], Some(sword.clone()));
            assert_eq!(game.offhand, Some(shield.clone()));
            assert_eq!(
                player.with_hand_item(game, Hand::Main, |stack| (
                    stack.item().id(game),
                    stack.count()
                )),
                Some((Some("minecraft:iron_sword"), 1))
            );
            assert_eq!(
                player.with_hand_item(game, Hand::Off, |stack| (
                    stack.item().id(game),
                    stack.count()
                )),
                Some((Some("minecraft:shield"), 1))
            );
            assert!(player.set_hand_item(game, Hand::Off, None).unwrap());
            assert_eq!(game.offhand, None);
            assert_eq!(game.inventory[3], Some(sword.clone()));

            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            assert!(
                zombie
                    .set_hand_item(game, Hand::Off, Some(sword.try_into_api().unwrap()))
                    .unwrap()
            );
            assert_eq!(
                zombie
                    .with_hand_item(game, Hand::Off, |stack| stack.project())
                    .flatten(),
                Some(sword.try_into_api().unwrap())
            );
            assert!(zombie.with_hand_item(game, Hand::Main, |_| ()).is_none());

            game.mobs.clear();
            assert!(
                !zombie
                    .set_hand_item(game, Hand::Main, Some(sword.try_into_api().unwrap()))
                    .unwrap()
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn mob_equipment_writes_keep_full_stacks() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::gear;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            let expect_slot = |game: &sunrise_gameplay::game::ServerGame<'_>,
                               living,
                               slot,
                               expected: Option<&ItemStack>| {
                assert_eq!(
                    game.living_equipment_stack(living, slot)
                        .map(|stack| stack.project())
                        .as_deref(),
                    expected,
                    "{slot:?}"
                );
            };
            for (id, kind) in [
                (12, "minecraft:zombie"),
                (13, "minecraft:horse"),
                (14, "minecraft:pillager"),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            let horse = game.living(game.entity(13).unwrap()).unwrap();
            let captain = game.living(game.entity(14).unwrap()).unwrap();
            let mut helmet = ItemStack::new("minecraft:iron_helmet", 2);
            helmet.damage = 7;
            helmet.data_mut().enchantments = vec![("minecraft:protection".into(), 3)];
            assert_eq!(
                zombie.set_equipment(
                    game,
                    EquipmentSlot::Head,
                    Some(helmet.try_into_api().unwrap())
                ),
                Err(mod_api::StackWriteError::CountExceeded { maximum: 1 })
            );
            assert!(game.living_set_equipment(zombie, EquipmentSlot::Head, Some(helmet.clone())));
            let trim: gear::Trim = ("minecraft:gold".into(), "minecraft:coast".into());
            gear::set(
                &mut game.mobs[0],
                gear::HEAD,
                vec![("minecraft:protection".into(), 4)],
            );
            gear::set_trim(&mut game.mobs[0], gear::HEAD, Some(trim.clone()));
            let mut expected = helmet.clone();
            expected.data_mut().enchantments = vec![("minecraft:protection".into(), 4)];
            expected.data_mut().trim = Some(trim);
            assert_eq!(
                zombie.with_equipment(game, EquipmentSlot::Head, |stack| {
                    (stack.count(), stack.damage(), stack.project())
                }),
                Some((2, 7, Some(expected.try_into_api().unwrap())))
            );
            assert_eq!(
                game.mobs[0]
                    .extra
                    .slot_stack(sunrise_items::equipment::HEAD),
                Some(&helmet),
                "reading overlays must not write into the resident slot"
            );
            let mut shield = ItemStack::new("minecraft:shield", 2);
            shield.damage = 11;
            assert!(game.living_set_equipment(
                zombie,
                EquipmentSlot::OffHand,
                Some(shield.clone())
            ));
            expect_slot(game, zombie, EquipmentSlot::OffHand, Some(&shield));
            assert!(game.living_set_equipment(horse, EquipmentSlot::Head, Some(helmet.clone())));
            let mut body = ItemStack::new("minecraft:diamond_horse_armor", 3);
            body.damage = 9;
            body.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("armor".into()));
            assert!(game.living_set_equipment(horse, EquipmentSlot::Body, Some(body.clone())));
            expect_slot(game, horse, EquipmentSlot::Head, Some(&helmet));
            expect_slot(game, horse, EquipmentSlot::Body, Some(&body));
            for slot in [
                EquipmentSlot::MainHand,
                EquipmentSlot::Chest,
                EquipmentSlot::Legs,
                EquipmentSlot::Feet,
            ] {
                let stack = ItemStack::new("minecraft:stone", 4);
                assert!(game.living_set_equipment(zombie, slot, Some(stack.clone())));
                expect_slot(game, zombie, slot, Some(&stack));
                assert!(game.living_set_equipment(
                    zombie,
                    slot,
                    Some(ItemStack::new("minecraft:stone", 0))
                ));
                expect_slot(game, zombie, slot, None);
            }
            let mut saddle = ItemStack::new("minecraft:saddle", 3);
            saddle.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("saddle".into()));
            assert!(game.living_set_equipment(horse, EquipmentSlot::Saddle, Some(saddle.clone())));
            expect_slot(game, horse, EquipmentSlot::Saddle, Some(&saddle));
            sunrise_sim::mobs::raider::set_patrol_leader(&mut game.mobs[2], true);
            sunrise_sim::mobs::raider::wear_banner(&mut game.mobs[2]);
            assert!(sunrise_sim::mobs::raider::is_captain(&game.mobs[2]));
            assert!(game.living_set_equipment(captain, EquipmentSlot::Head, Some(helmet.clone())));
            expect_slot(game, captain, EquipmentSlot::Head, Some(&helmet));
            assert!(!sunrise_sim::mobs::raider::is_captain(&game.mobs[2]));
            assert!(game.living_set_equipment(
                horse,
                EquipmentSlot::Body,
                Some(ItemStack::new("minecraft:air", 1))
            ));
            expect_slot(game, horse, EquipmentSlot::Body, None);
            let mut dragon =
                sunrise_gameplay_common::game::dragon_common::Dragon::new(Vec3::ZERO, 0.0, 0);
            // Reserve the dragon's eight consecutive part IDs away from the stand.
            dragon.id = 40;
            game.dragon.replace(dragon);
            let dragon = game.living(game.entity(40).unwrap()).unwrap();
            // The dragon and an armor stand store all eight slots, full
            // stacks included, and save them through the real codecs.
            let mut stand = sunrise_sim::entities::ArmorStand::new(16, Vec3::ZERO, 0.0);
            stand.id = 16;
            game.armor_stands.push(stand);
            let stand = game.living(game.entity(16).unwrap()).unwrap();
            for living in [dragon, stand] {
                for slot in EquipmentSlot::ALL {
                    let mut stack = ItemStack::new("minecraft:stone", 3);
                    stack.damage = 5 + slot as u32;
                    stack.data_mut().custom_name =
                        Some(sunrise_core::nbt::Tag::String(slot_name(slot).into()));
                    assert!(game.living_set_equipment(living, slot, Some(stack.clone())));
                    expect_slot(game, living, slot, Some(&stack));
                }
            }
            let saved = game
                .dragon
                .as_ref()
                .unwrap()
                .to_nbt([0; 4], None, &Default::default());
            let loaded =
                sunrise_gameplay_common::game::dragon_common::Dragon::from_nbt(&saved, 0).unwrap();
            assert_eq!(loaded.equipment, game.dragon.as_ref().unwrap().equipment);
            let stored = game.armor_stands.last().unwrap();
            let saved = sunrise_sim::entity_nbt::armor_stand_to_nbt(
                stored,
                [0; 4],
                None,
                &Default::default(),
            );
            let loaded = sunrise_sim::entity_nbt::armor_stand_from_nbt(&saved, 16).unwrap();
            assert_eq!(loaded.equipment, stored.equipment);
            for living in [dragon, stand] {
                assert!(game.living_set_equipment(living, EquipmentSlot::Saddle, None));
                expect_slot(game, living, EquipmentSlot::Saddle, None);
            }
            // An empty dragon saves no `equipment` key, a stale one included.
            for slot in EquipmentSlot::ALL {
                assert!(game.living_set_equipment(dragon, slot, None));
            }
            let stale =
                game.dragon
                    .as_ref()
                    .unwrap()
                    .to_nbt([0; 4], Some(&saved), &Default::default());
            assert!(stale.get("equipment").is_none());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_wolf_throws_and_reads_tamed_and_owner() {
        use sunrise_core::geometry::Vec3;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::entities::ArmorStand;

        super::super::native_entity_refs::tests::with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
            let id = game
                .spawn_mob(
                    "minecraft:wolf",
                    [2.5, 64.0, 2.5],
                    true,
                    sunrise_gameplay_common::game::creatures::SpawnSite::Finalizing(
                        sunrise_sim::mobs::spawn_rules::Reason::Command,
                    ),
                )
                .unwrap();
            let wolf = game.living(game.entity(id).unwrap()).unwrap();
            assert_eq!(game.living_is_tamed(wolf), Some(false));
            assert_eq!(game.living_owner(wolf), None);
            let m = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
            (m.extra.tame, m.extra.owner) = (true, Some([1, 2, 3, 4]));
            assert_eq!(game.living_is_tamed(wolf), Some(true));
            assert_eq!(
                game.living_owner(wolf),
                Some(0x1_0000_0002_0000_0003_0000_0004)
            );

            let bone = ItemStack::new("minecraft:bone", 1);
            assert!(
                game.living_drop(wolf, ItemStack::new("minecraft:air", 1), false, true)
                    .is_none()
            );
            let item = game.living_drop(wolf, bone.clone(), false, true).unwrap();
            assert_eq!(
                game.entity_kind(item)
                    .and_then(|kind| kind.id(game))
                    .as_deref(),
                Some("minecraft:item")
            );
            let wolf_uuid = game.uuid_for(id);
            let thrown = game.dropped.last().unwrap();
            let eye = game.mobs.iter().find(|m| m.id == id).unwrap().eye_y();
            assert_eq!(thrown.position.y, eye - f64::from(0.3f32));
            assert_eq!((thrown.pickup_delay, thrown.thrower), (40, Some(wolf_uuid)));
            // The drop swung: one tick on, that swing goes on.
            let m = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
            assert!(m.ai.swinging);
            m.ai.swing_time = 1;
            assert!(!game.living_swing(wolf, Hand::Main, true));
            let scattered = game.living_drop(wolf, bone, true, false).unwrap();
            assert_ne!(scattered, item);
            assert_eq!(game.dropped.last().unwrap().velocity.y, f64::from(0.2f32));
            assert_eq!(game.dropped.last().unwrap().thrower, None);

            game.armor_stands
                .push(ArmorStand::new(90, Vec3::new(0.5, 64.0, 0.5), 0.0));
            let stand = game.living(game.entity(90).unwrap()).unwrap();
            assert_eq!(game.living_is_tamed(stand), None);
            let apple = ItemStack::new("minecraft:apple", 1);
            assert!(game.living_drop(stand, apple, false, false).is_none());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn player_mod_drop_and_drop_key_have_equivalent_motion() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.yaw = 37.0;
            game.pitch = -21.0;
            *game.world.random = sunrise_core::random::LegacyRandom::new(12345);
            let item = ItemStack::new("minecraft:apple", 1);
            game.common.inventory[game.common.selected] = Some(item.clone());
            let player_random = game.player.as_ref().unwrap().random.clone();
            let (_, seeds) =
                sunrise_level::random_source::record_seeds(|| game.drop_held_item(true));
            let key_velocity = game.dropped.last().unwrap().velocity;
            *game.world.random = sunrise_core::random::LegacyRandom::new(12345);
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 42, 123);
            let player = game.living(game.entity(42).unwrap()).unwrap();
            game.player.as_mut().unwrap().random = player_random;
            let id = sunrise_level::random_source::replay_seeds(seeds.clone(), || {
                game.living_drop(player, item, false, true).unwrap().id()
            });
            assert!(
                seeds.complete(),
                "both drop routes allocate the same random streams"
            );
            let mod_velocity = game
                .dropped
                .iter()
                .find(|entity| entity.id == id)
                .unwrap()
                .velocity;
            assert_eq!(key_velocity, mod_velocity);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn player_drop_spread_uses_the_players_random_even_when_cancelled() {
        use mod_api::{Flow, Hooks, ItemTossEvent, ModEntry, ServerHost};
        use sunrise_core::{
            geometry::trig::{cos, sin},
            random::RandomSource,
        };
        struct Veto;
        #[mod_api::__subscriptions]
        impl Hooks for Veto {
            fn item_toss(&self, _: &mut dyn ServerHost, _: &mut ItemTossEvent) -> Flow {
                Flow::Cancel
            }
        }
        for (randomly, cancelled) in [(false, false), (true, false), (false, true)] {
            let mods: mod_api::Mods = if cancelled {
                vec![ModEntry::new("veto", Box::new(Veto))].into()
            } else {
                Vec::<ModEntry>::new().into()
            };
            mod_platform::testing::with_mods(mods, || {
                super::super::native_entity_refs::tests::with_game(|game| {
                    game.player = Some(
                        sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap(),
                    );
                    super::super::native_entity_refs::tests::bind_local_player(game, 42, 123);
                    game.yaw = 0.0;
                    game.pitch = 0.0;
                    game.player.as_mut().unwrap().random.set_seed(12345);
                    let mut expected = game.player.as_ref().unwrap().random.clone();
                    // Independent vanilla oracle at yaw=pitch=0. The item's
                    // constructor owns a separate stream, not these floats.
                    let velocity = if randomly {
                        let strength = expected.next_float() * 0.5;
                        let direction = f64::from(expected.next_float() * std::f32::consts::TAU);
                        [
                            f64::from(-sin(direction) * strength),
                            f64::from(0.2f32),
                            f64::from(cos(direction) * strength),
                        ]
                    } else {
                        let direction = f64::from(expected.next_float() * std::f32::consts::TAU);
                        let strength = f64::from(expected.next_float() * 0.02);
                        let jitter = (expected.next_float() - expected.next_float()) * 0.1;
                        [
                            sunrise_core::jmath::cos(direction) * strength,
                            f64::from(0.1 + jitter),
                            f64::from(0.3f32) + sunrise_core::jmath::sin(direction) * strength,
                        ]
                    };
                    let level = game.world.random.state();
                    let before = game.dropped.len();
                    let id = game.native_player_drop(
                        ItemStack::new("minecraft:apple", 1),
                        randomly,
                        true,
                    );
                    assert_eq!(
                        game.player.as_ref().unwrap().random.state(),
                        expected.state(),
                        "randomly={randomly} cancelled={cancelled}"
                    );
                    assert_eq!(game.world.random.state(), level, "no level random draws");
                    if cancelled {
                        assert!(id.is_none());
                        assert_eq!(game.dropped.len(), before);
                    } else {
                        let drop = game.dropped.iter().find(|d| Some(d.id) == id).unwrap();
                        assert_eq!(
                            [drop.velocity.x, drop.velocity.y, drop.velocity.z],
                            velocity
                        );
                    }
                });
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn armor_stand_drop_does_not_swing() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.armor_stands
                .push(sunrise_sim::entities::ArmorStand::new(
                    90,
                    sunrise_core::geometry::Vec3::ZERO,
                    0.0,
                ));
            let stand = game.living(game.entity(90).unwrap()).unwrap();
            let before = game.swings_started;
            assert!(
                game.living_drop(stand, ItemStack::new("minecraft:apple", 1), false, false)
                    .is_none()
            );
            assert_eq!(game.swings_started, before);
            assert!(game.dropped.is_empty());
        });
    }
    /// The living reads of #943 answer the engine's own decisions.
    #[test]
    #[ignore = "daily"]
    fn the_engines_own_portal_melee_freeze_and_air_answers() {
        use mod_api::Host;
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;
        use sunrise_sim::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            // Two zombies an arm's length apart, and one a stride away.
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(30, zombie.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0));
            game.mobs
                .push(Mob::new(31, zombie.clone(), Vec3::new(1.5, 64.0, 0.5), 0.0));
            game.mobs
                .push(Mob::new(32, zombie, Vec3::new(5.5, 64.0, 0.5), 0.0));
            let near = game.living(game.entity(30).unwrap()).unwrap();
            let beside = game.entity(31).unwrap();
            let far = game.entity(32).unwrap();

            // The walk gate's own answers: alive and travelling, until dead.
            {
                let host: &dyn Host = game;
                assert_eq!(near.can_use_portal(host), Some(true));
            }
            game.mobs[0].health = 0.0;
            {
                let host: &dyn Host = game;
                assert_eq!(near.can_use_portal(host), Some(false));
            }
            game.mobs[0].health = 20.0;

            // `LivingEntity.canUsePortal`'s sleep half: a villager asleep in
            // its bed does not travel, awake again it does.
            let villager = game
                .res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(35, villager, Vec3::new(11.5, 64.0, 0.5), 0.0));
            let sleeper = game.living(game.entity(35).unwrap()).unwrap();
            game.mobs.last_mut().unwrap().extra.ints[1] = 1;
            {
                let host: &dyn Host = game;
                assert_eq!(sleeper.can_use_portal(host), Some(false));
            }
            game.mobs.last_mut().unwrap().extra.ints[1] = 0;
            {
                let host: &dyn Host = game;
                assert_eq!(sleeper.can_use_portal(host), Some(true));
            }

            // The melee goal's own reach: beside it, yes; a stride away, no.
            // The freeze tick's decision, and a freeze-immune wearable on it.
            // A zombie's type is in the vanilla water-breathing tag.
            {
                let host: &dyn Host = game;
                assert_eq!(near.in_melee_range(host, beside), Some(true));
                assert_eq!(near.in_melee_range(host, far), Some(false));
                assert_eq!(near.can_freeze(host), Some(true));
                assert_eq!(near.can_breathe_underwater(host), Some(true));
            }
            game.mobs[0].extra.equipment.set(
                sunrise_items::equipment::FEET,
                Some(sunrise_items::items::ItemStack::new(
                    "minecraft:leather_boots",
                    1,
                )),
            );
            {
                let host: &dyn Host = game;
                assert_eq!(near.can_freeze(host), Some(false));
            }

            // A player's own answers: alive and awake, freezable, drowning.
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.living(game.entity(20).unwrap()).unwrap();
            {
                let host: &dyn Host = game;
                assert_eq!(player.can_use_portal(host), Some(true));
                assert_eq!(player.can_freeze(host), Some(true));
                assert_eq!(player.can_breathe_underwater(host), Some(false));
            }
            game.player.as_mut().unwrap().sleeping = true;
            {
                let host: &dyn Host = game;
                assert_eq!(player.can_use_portal(host), Some(false));
            }
            game.player.as_mut().unwrap().sleeping = false;
            game.player.as_mut().unwrap().freeze_immune = true;
            {
                let host: &dyn Host = game;
                assert_eq!(player.can_freeze(host), Some(false));
            }

            let squid = game.res.mob_types().get("minecraft:squid").unwrap().clone();
            game.mobs
                .push(Mob::new(33, squid, Vec3::new(7.5, 64.0, 0.5), 0.0));
            let squid = game.living(game.entity(33).unwrap()).unwrap();
            let golem_kind = game
                .res
                .mob_types()
                .get("minecraft:copper_golem")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(34, golem_kind, Vec3::new(9.5, 64.0, 0.5), 0.0));
            let golem = game.living(game.entity(34).unwrap()).unwrap();
            {
                let host: &dyn Host = game;
                // A squid and a copper golem (#can_breathe_under_water)
                // breathe water.
                assert_eq!(squid.can_breathe_underwater(host), Some(true));
                assert_eq!(golem.can_breathe_underwater(host), Some(true));
            }
            game.mobs.clear();
            {
                let host: &dyn Host = game;
                assert_eq!(near.can_use_portal(host), None);
                assert_eq!(near.in_melee_range(host, beside), None);
                assert_eq!(near.can_freeze(host), None);
                assert_eq!(near.can_breathe_underwater(host), None);
            }
        });
    }
    /// A sleeper's bed orientation is the bed block's own facing (#943):
    /// the block at the sleeping position, where it is one of `#minecraft:beds`.
    #[test]
    #[ignore = "daily"]
    fn a_sleepers_bed_orientation() {
        use mod_api::{Direction, Host};
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;
        super::super::native_entity_refs::tests::with_game(|game| {
            // A floor with a red bed on it, facing north.
            for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                let mut chunk = sunrise_level::chunk::Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
            }
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            for x in -8..8 {
                for z in -8..8 {
                    game.world.set(x, 63, z, stone).unwrap();
                }
            }
            let bed = game
                .pack
                .assets
                .registry
                .default_state("minecraft:red_bed")
                .unwrap();
            game.world.set(2, 63, 2, bed).unwrap();
            game.world.set(3, 63, 2, bed).unwrap();

            let villager = game
                .res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(30, villager, Vec3::new(0.5, 64.0, 0.5), 0.0));
            let sleeper = game.living(game.entity(30).unwrap()).unwrap();
            let host: &dyn Host = game;
            // Awake: no sleeping position, no bed.
            assert_eq!(sleeper.bed_orientation(host), None);
            // Asleep in the bed: the block's own facing, the engine's own
            // sleeping position naming the bed.
            game.mobs[0].extra.ints[1] = 1;
            sunrise_sim::mobs::villager::ext_mut(&mut game.mobs[0]).sleeping_pos = Some([2, 63, 2]);
            let host: &dyn Host = game;
            assert_eq!(sleeper.bed_orientation(host), Some(Direction::North));
            assert_eq!(sleeper.can_use_portal(host), Some(false));
            // A sleeping position that is not a bed answers none.
            sunrise_sim::mobs::villager::ext_mut(&mut game.mobs[0]).sleeping_pos = Some([0, 63, 0]);
            let host: &dyn Host = game;
            assert_eq!(sleeper.bed_orientation(host), None);
            // A player asleep in the same bed answers the same facing.
            game.player =
                Some(sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            game.sleeping = Some([2, 63, 2]);
            let player = game.living(game.entity(20).unwrap()).unwrap();
            let host: &dyn Host = game;
            assert_eq!(player.bed_orientation(host), Some(Direction::North));
            // Fox sleeping overrides LivingEntity.isSleeping. These species
            // may all travel awake; the fox sleep flag vetoes portal use.
            for name in ["minecraft:fox", "minecraft:bat", "minecraft:cat"] {
                let kind = game.res.mob_types().get(name).unwrap().clone();
                let mut mob = Mob::new(31, kind, Vec3::ZERO, 0.0);
                assert!(sunrise_sim::mobs::mob_can_use_portal(&mob));
                if name == "minecraft:fox" {
                    mob.extra.ints[0] |= sunrise_sim::mobs::fox::SLEEPING;
                    assert!(!sunrise_sim::mobs::mob_can_use_portal(&mob));
                }
            }
            assert!(!game.mob_portal_eligible(&game.mobs[0]));
            game.mobs.clear();
            let host: &dyn Host = game;
            assert_eq!(sleeper.bed_orientation(host), None);
        });
    }
    /// #943, review: a guest answers the same one portal rule over their
    /// body and vehicle (`can_use_portal_of`), not a hand-rolled copy.
    #[test]
    #[ignore = "daily"]
    fn a_guest_answers_the_one_portal_rule() {
        use mod_api::Host;
        super::super::native_guest_turn_tests::with_guests(|game| {
            // The fixture leaves the guest without a body: give one, as a
            // joined player's server record keeps.
            game.server.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                g.motion = Some(Box::new(
                    sunrise_sim::motion::PlayerMotion::new(
                        game.common.motion,
                        sunrise_core::geometry::Vec3::new(3.0, 64.0, 5.0),
                    )
                    .unwrap(),
                ))
            });
            let living = game.living(game.entity(12).unwrap()).unwrap();
            let host: &dyn Host = game;
            assert_eq!(living.can_use_portal(host), Some(true));
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| g.vehicle = Some(99));
            let host: &dyn Host = game;
            assert_eq!(living.can_use_portal(host), Some(false));
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| g.vehicle = None);
            // Asleep: the rule's sleep half answers, over their body.
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| g.sleeping = true);
            let host: &dyn Host = game;
            assert_eq!(living.can_use_portal(host), Some(false));
            // A passenger crosses with its vehicle, not as its own
            // traveller: the rule's riding half answers false.
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| g.sleeping = false);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| g.vehicle = Some(60));
            let host: &dyn Host = game;
            assert_eq!(living.can_use_portal(host), Some(false));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn armor_stand_first_tick_equipment_change_fires_after_base_tick() {
        use mod_api::{self, Hooks, LivingEquipmentChangeEvent, ModEntry, ServerHost};
        struct Reequip;
        #[mod_api::__subscriptions]
        impl Hooks for Reequip {
            fn living_equipment_change(
                &self,
                host: &mut dyn ServerHost,
                event: &mut LivingEquipmentChangeEvent<'_>,
            ) {
                if event.entity.id() == 18 && event.slot == EquipmentSlot::MainHand {
                    assert!(
                        event
                            .entity
                            .set_equipment(
                                host,
                                EquipmentSlot::Head,
                                Some(
                                    ItemStack::new("minecraft:iron_helmet", 1)
                                        .try_into_api()
                                        .unwrap()
                                ),
                            )
                            .unwrap()
                    );
                }
            }
        }
        mod_platform::testing::with_mods(
            vec![ModEntry::new("first_tick_equipment", Box::new(Reequip))].into(),
            || {
                super::super::native_entity_refs::tests::with_game(|game| {
                    let mut stand =
                        sunrise_sim::entities::ArmorStand::new(18, Vec3::new(0.5, 70.0, 0.5), 0.0);
                    stand.no_gravity = true;
                    stand.equipment[0] = Some(ItemStack::new("minecraft:stone", 1));
                    game.armor_stands.push(stand);
                    game.start_audio_capture();
                    sunrise_gameplay::game::server_side::dispense_fixture::start_recording();
                    game.tick_armor_stands().unwrap();
                    let events =
                        sunrise_gameplay::game::server_side::dispense_fixture::stop_recording();
                    assert_eq!(
                        game.armor_stands[0].equipment[5],
                        Some(ItemStack::new("minecraft:iron_helmet", 1)),
                        "real listener/handle write control",
                    );
                    assert!(!game.armor_stands[0].first_tick);
                    let sounds = game.server.pending_sounds.len();
                    let equips = events
                        .iter()
                        .filter(|e| e.ends_with("minecraft:equip"))
                        .count();
                    assert_eq!((sounds, equips), (1, 1));
                });
            },
        );
    }
    #[test]
    #[ignore = "daily"]
    fn mob_equipment_writes_emit_vanilla_equip_events() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let living = game.living(game.entity(12).unwrap()).unwrap();
            game.start_audio_capture();
            sunrise_gameplay::game::server_side::dispense_fixture::start_recording();
            let helmet = ItemStack::new("minecraft:iron_helmet", 1);
            assert!(game.living_set_equipment(living, EquipmentSlot::Head, Some(helmet.clone())));
            assert!(game.living_set_equipment(living, EquipmentSlot::Head, None));
            assert!(game.server.pending_sounds.is_empty());
            game.mobs[0].first_tick = false;
            assert!(game.living_set_equipment(living, EquipmentSlot::Head, Some(helmet.clone())));
            assert_eq!(game.server.pending_sounds.len(), 1);
            assert!(game.living_set_equipment(living, EquipmentSlot::Head, Some(helmet)));
            assert_eq!(game.server.pending_sounds.len(), 1);
            game.mobs[0].silent = true;
            assert!(game.living_set_equipment(
                living,
                EquipmentSlot::Head,
                Some(ItemStack::new("minecraft:diamond_helmet", 1))
            ));
            assert!(game.living_set_equipment(living, EquipmentSlot::Head, None));
            game.mobs[0].silent = false;
            assert!(game.living_set_equipment(
                living,
                EquipmentSlot::Chest,
                Some(ItemStack::new("minecraft:iron_helmet", 1))
            ));
            assert_eq!(game.server.pending_sounds.len(), 1);
            let events = sunrise_gameplay::game::server_side::dispense_fixture::stop_recording();
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e.ends_with("minecraft:equip"))
                    .count(),
                3
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e.ends_with("minecraft:unequip"))
                    .count(),
                1
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn stand_and_dragon_equipment_writes_run_on_equip_item() {
        use sunrise_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let mut stand = sunrise_sim::entities::ArmorStand::new(16, Vec3::ZERO, 0.0);
            stand.id = 16;
            game.armor_stands.push(stand);
            let mut dragon =
                sunrise_gameplay_common::game::dragon_common::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 17;
            game.dragon.replace(dragon);
            let stand = game.living(game.entity(16).unwrap()).unwrap();
            let dragon = game.living(game.entity(17).unwrap()).unwrap();
            game.start_audio_capture();
            sunrise_gameplay::game::server_side::dispense_fixture::start_recording();
            let helmet = ItemStack::new("minecraft:iron_helmet", 1);
            assert!(game.living_set_equipment(stand, EquipmentSlot::Head, Some(helmet.clone())));
            assert!(game.living_set_equipment(stand, EquipmentSlot::Head, None));
            assert!(game.server.pending_sounds.is_empty());
            game.tick_armor_stands().unwrap();
            game.dragon.as_mut().unwrap().ticks = 1;
            for _ in 0..2 {
                assert!(game.living_set_equipment(
                    stand,
                    EquipmentSlot::Head,
                    Some(helmet.clone())
                ));
            }
            let body = ItemStack::new("minecraft:diamond_horse_armor", 1);
            assert!(game.living_set_equipment(stand, EquipmentSlot::Body, Some(body)));
            let stone = ItemStack::new("minecraft:stone", 1);
            assert!(game.living_set_equipment(stand, EquipmentSlot::Saddle, Some(stone.clone())));
            let chest = ItemStack::new("minecraft:iron_chestplate", 1);
            assert!(game.living_set_equipment(dragon, EquipmentSlot::Chest, Some(chest)));
            assert!(game.living_set_equipment(dragon, EquipmentSlot::Saddle, Some(stone)));
            assert_eq!(game.server.pending_sounds.len(), 3);
            let events = sunrise_gameplay::game::server_side::dispense_fixture::stop_recording();
            let count = |name: &str| events.iter().filter(|e| e.ends_with(name)).count();
            assert_eq!(count("minecraft:equip"), 3);
            assert_eq!(count("minecraft:unequip"), 2);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn the_host_players_body_and_saddle_are_slots_and_saved_in_equipment() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let owner = Owner::Entity(Target::Player);
            let mut body = ItemStack::new("minecraft:diamond_horse_armor", 1);
            body.damage = 4;
            body.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("armor".into()));
            let saddle = ItemStack::new("minecraft:saddle", 1);
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.start_audio_capture();
            for (name, stack) in [("armor.body", &body), ("saddle", &saddle)] {
                let slots = game.slots_named(&owner, name);
                assert_eq!(slots.len(), 1, "{name} is one slot");
                assert!(game.slot_set(&slots[0], Some(stack.clone())));
            }
            assert_eq!(
                (game.equipment.get(6), game.equipment.get(7)),
                (Some(&body), Some(&saddle))
            );
            assert_eq!(game.server.pending_sounds.len(), 2);
            game.sync_level();
            let tag = game.level.player_tag(&Default::default());
            let equipment = tag.get("equipment").unwrap();
            assert!(equipment.get("body").is_some() && equipment.get("saddle").is_some());
            let mut read = sunrise_sim::save::Level::default();
            read.read_player(&tag);
            game.equipment = Default::default();
            let level = read;
            game.load_saved_inventory(&level);
            assert_eq!(
                (game.equipment.get(6), game.equipment.get(7)),
                (Some(&body), Some(&saddle))
            );
        });
    }

    #[test]
    fn a_worn_items_dye_override_reads_without_a_patch_projection() {
        use mod_api::{Tag, prelude::ItemsHost};
        use std::num::NonZeroU32;
        use sunrise_gameplay_common::game::native_equipment_read::with_equipment_view;
        use sunrise_sim::mobs::finalize::EquipmentRead;

        let mut host = mod_api::testing::FakeHost::new();
        let id = mod_api::Identifier::parse("armor_test:chest").unwrap();
        host.registries.resolve_items([id.clone()]).unwrap();
        let item = host.item_key(&id).unwrap();
        let metadata = (item, NonZeroU32::MIN, None);
        let probe = |stack: &ItemStack, ids: &[&str]| {
            with_equipment_view(&EquipmentRead::resident(stack), metadata, |_, read| {
                ids.iter().map(|id| read(id)).collect::<Vec<_>>()
            })
        };
        let dyed_id = ["minecraft:dyed_color", "!minecraft:dyed_color"];
        let mut stack = ItemStack::new("minecraft:leather_chestplate", 1);
        assert_eq!(probe(&stack, &dyed_id), [None, None], "absence");
        stack.data_mut().dyed_color = Some(0x12_34_56);
        assert_eq!(
            probe(&stack, &dyed_id),
            [Some(Tag::Int(0x12_34_56)), None],
            "the typed field"
        );
        let mut removed = ItemStack::new("minecraft:leather_chestplate", 1);
        removed.data_mut().set_other(
            "!minecraft:dyed_color",
            Some(Tag::Compound(Default::default())),
        );
        assert_eq!(
            probe(&removed, &dyed_id),
            [None, Some(Tag::Compound(Default::default()))],
            "the removal marker"
        );
    }
}
