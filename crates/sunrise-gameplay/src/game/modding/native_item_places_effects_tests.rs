#[cfg(test)]
pub(in crate::game) mod tests {
    use crate::game::native_entity_refs::tests::with_resident_game;
    use mod_api::HostTransaction;
    use mod_api::{ENTITY_ITEMS, ItemResource, TransactionError};
    use sunrise_gameplay_common::game::{
        api_values::TryIntoApi, native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
        transactions::TransactionsGameExt as _,
    };
    use sunrise_items::items::ItemStack;

    #[test]
    #[ignore = "daily"]
    fn stand_equipment_effects_wait_for_root_commit() {
        with_resident_game(|game| {
            let mut stand =
                sunrise_sim::entities::ArmorStand::new(40, sunrise_core::geometry::Vec3::ZERO, 0.0);
            stand.first_tick = false;
            game.armor_stands.push(stand);
            game.start_audio_capture();
            let items = game
                .entity(40)
                .unwrap()
                .capability(game, &ENTITY_ITEMS, ())
                .unwrap();
            let chest = ItemResource::of(
                &ItemStack::new("minecraft:diamond_chestplate", 1)
                    .try_into_api()
                    .unwrap(),
            );
            super::super::dispense_fixture::start_recording();
            let result = game.storage_transaction(|host, tx| {
                assert_eq!(items.insert_at(host, tx, 4, &chest, 1), Ok(1));
                Err::<(), ()>(())
            });
            assert_eq!(result, Err(()));
            assert!(game.armor_stands[0].equipment.get(4).is_none());
            assert!(
                game.server.pending_sounds.is_empty(),
                "aborted stand equip emitted sound"
            );
            assert!(super::super::dispense_fixture::stop_recording().is_empty());

            super::super::dispense_fixture::start_recording();
            let result = game.storage_transaction(|host, _| {
                host.storage_transaction(|host, tx| {
                    assert_eq!(items.insert_at(host, tx, 4, &chest, 1), Ok(1));
                    Ok::<(), ()>(())
                })
                .unwrap();
                Err::<(), ()>(())
            });
            assert_eq!(result, Err(()));
            assert!(game.armor_stands[0].equipment.get(4).is_none());
            assert!(game.server.pending_sounds.is_empty());
            assert!(super::super::dispense_fixture::stop_recording().is_empty());

            super::super::dispense_fixture::start_recording();
            let result = game.storage_transaction(|host, _| {
                // An inner abort must not contribute effects to the root.
                let aborted = host.transaction(|host, tx| {
                    assert_eq!(items.insert_at(host, tx, 4, &chest, 1), Ok(1));
                    Err::<(), ()>(())
                });
                assert_eq!(aborted, Err(TransactionError::Aborted(())));
                host.transaction(|host, tx| {
                    assert_eq!(items.insert_at(host, tx, 4, &chest, 1), Ok(1));
                    Ok::<(), ()>(())
                })
                .unwrap();
                assert!(super::super::dispense_fixture::stop_recording().is_empty());
                super::super::dispense_fixture::start_recording();
                Ok::<(), ()>(())
            });
            assert_eq!(result, Ok(()));
            assert_eq!(game.armor_stands[0].equipment.get(4).unwrap().count, 1);
            assert_eq!(game.server.pending_sounds.len(), 1);
            let events = super::super::dispense_fixture::stop_recording();
            assert_eq!(events.len(), 1, "only one event at root commit: {events:?}");
            assert!(events[0].ends_with("minecraft:equip"));
        });
    }
}
