#![cfg(test)]
//! A golden apple's `ItemPart` answers `can_continue_using`
//! (`IItemExtension.canContinueUsing`): the use goes on, or stops, whatever
//! `ItemStack.isSameItem` says.
use mod_api::{CanContinueUsingEvent, Hooks, Host, ItemPart, Registrar, RegistrationContext};
use std::sync::Mutex;
use sunrise_gameplay::game::ServerGame;
use sunrise_gameplay_common::game::{
    chat_common::ChatCommonGameExt1 as _, held_items::HeldItemsGameExt1 as _,
    player_identity::PlayerIdentityGameExt1 as _,
};
use sunrise_items::items::ItemStack;

/// What the part answers: `None` leaves vanilla's answer.
static ANSWER: Mutex<Option<bool>> = Mutex::new(None);
/// What the part was asked: `(held is the use's item, entity, vanilla's answer)`.
static ASKED: Mutex<Vec<(bool, Option<i32>, bool)>> = Mutex::new(Vec::new());

#[derive(Clone)]
struct Apple;
#[mod_api::sunrise_part]
impl ItemPart for Apple {
    fn can_continue_using(&self, _: &dyn Host, event: &mut CanContinueUsingEvent<'_>) {
        ASKED.lock().unwrap().push((
            event.held.item() == event.stack.item(),
            event.entity,
            event.can_continue,
        ));
        if let Some(answer) = *ANSWER.lock().unwrap() {
            event.can_continue = answer;
        }
    }
}
struct Parts;
#[mod_api::__subscriptions]
impl Hooks for Parts {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.item_key("minecraft:golden_apple");
        r.item_part(key, Apple);
    }
}

/// Starts eating a golden apple with the part on, ticks the use five times,
/// then runs `body`.
fn eating(body: impl FnOnce(&mut ServerGame<'_>)) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("apple", Box::new(Parts))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        super::item_use::interact_vibration_tests::with_game("minecraft:golden_apple", |game| {
            game.creative = false;
            game.build_registries();
            game.server_set_acting_use(true);
            assert!(game.start_using_item());
            for _ in 0..5 {
                game.tick_using_server();
            }
            assert_eq!(game.using, Some((game.selected, 5)));
            body(game);
        })
        .expect("prepared resources");
    });
}

#[test]
#[ignore = "daily"]
fn a_part_keeps_or_stops_a_use_whatever_the_hand_holds() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::continue_using_tests::a_part_keeps_or_stops_a_use_whatever_the_hand_holds",
        || {
            let asked = || ASKED.lock().unwrap().drain(..).collect::<Vec<_>>();
            eating(|game| {
                // Asked each tick with the same apple: vanilla's `true`.
                assert_eq!(asked().len(), 5);
                // A bow in the apple's place goes on when the part says so,
                // and the use's stack is the bow from then on.
                *ANSWER.lock().unwrap() = Some(true);
                *game.held_mut() = Some(ItemStack::new("minecraft:bow", 1));
                game.tick_using_server();
                assert_eq!(game.using.map(|u| u.1), Some(6));
                assert_eq!(asked(), [(false, Some(game.own_entity_id()), false)]);
                // The bow has no part: it is not asked again, and the bow goes on.
                game.tick_using_server();
                assert_eq!(game.using.map(|u| u.1), Some(7));
                assert!(asked().is_empty());
            });
            *ANSWER.lock().unwrap() = None;
            eating(|game| {
                asked();
                *ANSWER.lock().unwrap() = Some(false);
                // The same apple, refused: the use stops without eating it.
                game.tick_using_server();
                assert!(game.using.is_none());
                assert_eq!(game.held().map(|s| s.count), Some(1));
            });
            *ANSWER.lock().unwrap() = None;
        },
    );
}
