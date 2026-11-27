#![cfg(feature = "client")]
use mod_api::{Flow, Hooks, HostSide, LivingEntityUseItemTickEvent, ModEntry, Mods};
use std::sync::{Arc, Mutex};
use sunrise_items::items::ItemStack;

struct Effects(Arc<Mutex<Vec<i32>>>);
#[mod_api::sunrise_part]
impl mod_api::ItemPart for Effects {
    fn on_use_tick(&self, _: HostSide, event: &mut mod_api::OnUseTickEvent<'_>) {
        self.0.lock().unwrap().push(event.remaining);
    }
}

struct Tick {
    pub answer: Arc<Mutex<Option<(bool, i32)>>>,
    heard: Arc<Mutex<Vec<i32>>>,
    effects: Arc<Mutex<Vec<i32>>>,
}
#[mod_api::__subscriptions]
impl Hooks for Tick {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let apple = r.item_key("minecraft:apple");
        r.item_part(apple, Effects(self.effects.clone()));
    }
    fn living_entity_use_item_tick(
        &self,
        _: HostSide,
        event: &mut LivingEntityUseItemTickEvent<'_>,
    ) -> Flow {
        self.heard.lock().unwrap().push(event.duration);
        if let Some((cancel, remaining)) = self.answer.lock().unwrap().take() {
            event.duration = remaining;
            if cancel {
                return Flow::Cancel;
            }
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn client_tick_cancellation_skips_one_tick_and_countdown_continues() {
    let answer = Arc::new(Mutex::new(None));
    let heard = Arc::new(Mutex::new(Vec::new()));
    let effects = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "client_tick",
        Box::new(Tick {
            answer: answer.clone(),
            heard: heard.clone(),
            effects: effects.clone(),
        }),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            for (cancel, replacement, after_first, after_second) in [
                (true, 5, -2, -3),
                (false, 0, -1, -2),
                (false, 1, 0, -1),
                (false, -7, -8, -9),
            ] {
                heard.lock().unwrap().clear();
                effects.lock().unwrap().clear();
                *answer.lock().unwrap() = Some((cancel, replacement));
                game.common.inventory[game.common.selected] =
                    Some(ItemStack::new("minecraft:apple", 2));
                game.using = Some((game.selected, 0));
                game.use_duration = 32;
                game.use_start = sunrise_gameplay_common::game::item_use_common::UseStart::of(
                    game.common.inventory[game.common.selected].clone().unwrap(),
                );
                game.set_mapping_held("key.use", true);
                game.remote_tick_using();
                assert_eq!(game.using, Some((game.selected, 1)));
                assert_eq!(
                    crate::game::item_use_common::ticks_left(game.use_duration, 1),
                    after_first
                );
                game.remote_tick_using();
                assert_eq!(game.using, Some((game.selected, 2)));
                assert_eq!(
                    crate::game::item_use_common::ticks_left(game.use_duration, 2),
                    after_second
                );
                assert_eq!(*heard.lock().unwrap(), [32, after_first]);
                let expected_effects = if cancel {
                    vec![]
                } else if replacement == 1 {
                    vec![1]
                } else {
                    vec![]
                };
                assert_eq!(*effects.lock().unwrap(), expected_effects);
                // No client-side completion or consumption.
                assert_eq!(
                    game.common.inventory[game.common.selected]
                        .as_ref()
                        .unwrap()
                        .count,
                    2
                );
            }
        });
    });
}
