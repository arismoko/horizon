#![cfg(test)]
use mod_api::EntityRef;
use mod_api::ItemEntityRef;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_items::items::ItemStack;
use sunrise_sim::lan::playerdata::uuid_ints;

use sunrise_gameplay_common::game::native_item_entity_refs::NativeItemEntityRefsGameExt1 as _;

#[cfg(test)]
pub(crate) mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use mod_api::{HostTransaction, WorldEpoch};
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::api_values::test_snapshot;
    use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;
    use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;
    use sunrise_sim::{
        entities::{ArmorStand, ItemEntity},
        mobs::Mob,
    };

    #[test]
    #[ignore = "daily"]
    fn item_handles_preserve_stacks_and_distinguish_target_from_thrower() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            game.options.dedicated = Some(Vec::new());
            let mut stack = ItemStack {
                item: "minecraft:diamond_pickaxe".into(),
                count: 1,
                damage: 17,
                extra: None,
            };
            stack.data_mut().custom_name =
                Some(sunrise_core::nbt::Tag::String("Named pick".into()));
            game.spawn_item(stack.clone(), [0.0; 3], [0.0; 3], 0);
            let entity = game.entity(game.dropped[0].id).unwrap();
            let item = game.item_entity(entity).unwrap();
            assert_eq!(item.entity(), entity);
            assert_eq!(
                item.with_stack(game, |read| (
                    read.item().id(game),
                    read.count(),
                    read.damage()
                )),
                Some((Some("minecraft:diamond_pickaxe"), 1, 17))
            );
            let mut changed = stack;
            changed.damage = 21;
            let changed = test_snapshot(&changed);
            assert_eq!(item.set_stack(game, Some(changed.clone())), Ok(true));
            assert_eq!(
                item.with_stack(game, |stack| stack.project()).flatten(),
                Some(changed.clone())
            );
            assert!(item.set_pickup_delay(game, -7));
            assert_eq!(item.pickup_delay(game), Some(-7));
            assert!(item.set_default_pickup_delay(game));
            assert_eq!(item.pickup_delay(game), Some(10));
            assert!(item.set_never_pickup(game));
            assert_eq!(
                game.dropped[0].pickup_delay,
                ItemEntity::INFINITE_PICKUP_DELAY
            );
            assert_eq!(
                item.pickup_delay(game),
                Some(ItemEntity::INFINITE_PICKUP_DELAY)
            );
            assert!(item.set_no_pickup_delay(game));
            assert_eq!(item.pickup_delay(game), Some(0));
            game.dropped[0].age = ItemEntity::INFINITE_LIFETIME;
            assert_eq!(item.age(game), Some(ItemEntity::INFINITE_LIFETIME));
            let target = 0x12345678_9abcdef0_10203040_50607080_u128;
            assert!(item.set_target(game, Some(target)));
            assert_eq!(item.target(game), Some(target));
            assert_eq!(item.owner(game), None);
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(99, kind, Vec3::ZERO, 0.0));
            let thrower = game.entity(99).unwrap();
            game.dropped[0].thrower = Some(uuid_ints(game.entity_uuid(thrower).unwrap()));
            assert_eq!(item.owner(game), Some(thrower));
            assert_eq!(item.target(game), Some(target));
            assert!(item.set_target(game, None));
            assert_eq!(item.target(game), None);
            game.armor_stands
                .push(ArmorStand::new(100, Vec3::ZERO, 0.0));
            assert!(game.item_entity(game.entity(100).unwrap()).is_none());
            let stale = ItemEntityRef::new(EntityRef::new(
                WorldEpoch::new(entity.epoch().get() + 1),
                entity.id(),
            ));
            assert_eq!(stale.age(game), None);
            assert!(stale.with_stack(game, |_| ()).is_none());
            assert!(!stale.set_target(game, Some(target)));
            let mut empty = item
                .with_stack(game, |stack| stack.project())
                .flatten()
                .unwrap();
            assert_eq!(empty.set_count(0), Err(mod_api::StackError::Empty));
            // An empty stack stays resident until the next item tick.
            let air = test_snapshot(&ItemStack::new("minecraft:air", 1));
            assert_eq!(item.set_stack(game, Some(air)), Ok(true));
            assert_eq!(game.dropped.len(), 1);
            assert_eq!(game.dropped[0].stack.count, 0);
            assert_eq!(&*game.dropped[0].stack.item, "minecraft:air");
            assert!(item.with_stack(game, |_| ()).is_none());
            // Aborting the transaction restores the prior stack.
            assert_eq!(item.set_stack(game, Some(changed.clone())), Ok(true));
            assert_eq!(item.with_stack(game, |stack| stack.damage()), Some(21));
            let host: &mut dyn mod_api::ServerHost = game;
            let _ = host.transaction(|host, _tx| {
                assert_eq!(item.set_stack(host, None), Ok(true));
                assert!(item.with_stack(host, |_| ()).is_none());
                Err::<(), ()>(())
            });
            assert_eq!(item.with_stack(game, |stack| stack.damage()), Some(21));
            let taken = game.dropped.lift();
            assert!(item.with_stack(game, |_| ()).is_none());
            assert!(!item.set_no_pickup_delay(game));
            game.put_back_dropped(taken);
            // Committing an empty stack leaves the item resident until its next tick discards it.
            assert_eq!(item.set_stack(game, None), Ok(true));
            assert_eq!(game.dropped.len(), 1);
            game.server.ticked_entities_through = u32::MAX;
            let mut round = game.native_tick_round();
            game.tick_dropped_items(&mut round);
            assert!(
                game.dropped.is_empty(),
                "normal ItemEntity.tick discards an item entity holding an empty stack"
            );
        });
    }
}
