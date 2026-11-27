use super::*;

/// One `use_item` event as `(side, hand, player, player_entity_id)`.
pub(crate) type ItemUser = (LogicalSide, Hand, u128, i32);

pub(crate) fn item_users(observer: &Observer) -> Vec<ItemUser> {
    std::mem::take(&mut observer.0.lock().unwrap().item_users)
}

pub(crate) fn item_observer() -> (Observer, mod_api::Mods) {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "item_users",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    (observer, mods)
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn the_item_event_names_this_client_on_a_predicted_use() {
    let (observer, mods) = item_observer();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_client_game(|game| {
            let remote = game.client.remote.as_mut().unwrap();
            remote.profile = 0xC0FF_EE00_0000_0000_0000_0000_0000_002A;
            remote.entity_id = 42;
            game.offhand = Some(ItemStack::new("minecraft:stick", 1));
            for hand in [Hand::Main, Hand::Off] {
                game.remote_use_item(hand);
                assert_eq!(
                    item_users(&observer),
                    [(
                        LogicalSide::Client,
                        hand,
                        0xC0FF_EE00_0000_0000_0000_0000_0000_002A,
                        42
                    )],
                    "{hand:?}"
                );
            }
        });
    });
}
