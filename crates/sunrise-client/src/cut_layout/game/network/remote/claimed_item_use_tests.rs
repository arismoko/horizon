#![cfg(feature = "client")]
use mod_api::{ClientHost, ClientItemUseBehavior, Hooks, ItemKey, Registrar, RegistrationContext};
use std::sync::{Arc, Mutex};
use sunrise_client::interaction_input::Button;
use sunrise_core::nbt::{JavaString, Tag};
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use {
    crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::logged_in,
    crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::sent,
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay_common::game::Hit, sunrise_gameplay_common::game::Screen,
};
use {sunrise_gameplay_common::game::remote::S, sunrise_gameplay_common::game::remote::play};

pub(crate) const OWNER: &str = "client_use_fixture";
pub(crate) const SHIELD: &str = "minecraft:shield";
pub(crate) const STICK: &str = "minecraft:stick";
pub(crate) const CUSTOM_DATA: &str = "minecraft:custom_data";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Start,
    Release(u32),
    Stop(u32),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Call {
    pub(in crate::game) phase: Phase,
    pub(in crate::game) registered_for: &'static str,
    pub(in crate::game) item: ItemKey,
    pub(in crate::game) slot: u32,
    pub(in crate::game) count: u32,
    pub(in crate::game) marker: Option<Tag>,
}

#[derive(Clone, Default)]
pub(crate) struct Calls(pub(in crate::game) Arc<Mutex<Vec<Call>>>);

impl Calls {
    pub(crate) fn take(&self) -> Vec<Call> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

#[mod_api::__subscriptions]
impl Hooks for Calls {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for registered_for in [SHIELD, STICK] {
            let item = r.item_key(registered_for);
            r.register_client_item_use(
                item,
                Arc::new(RecordUse {
                    registered_for,
                    calls: self.clone(),
                }),
            );
        }
    }
}

pub(crate) struct RecordUse {
    pub(crate) registered_for: &'static str,
    pub(crate) calls: Calls,
}

impl RecordUse {
    pub(crate) fn record(&self, phase: Phase, stack: &mod_api::ItemStack, slot: u32) {
        let call = mod_api::content::crash::current_native_call().unwrap();
        assert_eq!(call.id, OWNER);
        assert_eq!(
            call.hook,
            match phase {
                Phase::Start => "use_item",
                Phase::Release(_) => "release_using",
                Phase::Stop(_) => "on_stop_using",
            }
        );
        let marker = stack
            .components()
            .and_then(|components| components.as_nbt().get(&JavaString::from(CUSTOM_DATA)))
            .and_then(|data| data.get("marker"))
            .cloned();
        self.calls.0.lock().unwrap().push(Call {
            phase,
            registered_for: self.registered_for,
            item: stack.item(),
            slot,
            count: stack.count(),
            marker,
        });
    }
}

impl ClientItemUseBehavior for RecordUse {
    fn use_item(&self, _: &mut dyn ClientHost, stack: &mod_api::ItemStack, slot: u32) {
        self.record(Phase::Start, stack, slot);
    }

    fn release_using(
        &self,
        _: &mut dyn ClientHost,
        stack: &mod_api::ItemStack,
        slot: u32,
        ticks_held: u32,
    ) {
        self.record(Phase::Release(ticks_held), stack, slot);
    }

    fn on_stop_using(
        &self,
        _: &mut dyn ClientHost,
        stack: &mod_api::ItemStack,
        slot: u32,
        ticks_held: u32,
    ) {
        self.record(Phase::Stop(ticks_held), stack, slot);
    }
}

pub(crate) fn with_owner(test: impl FnOnce(&Calls)) {
    let calls = Calls::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(OWNER, Box::new(calls.clone()))].into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || test(&calls));
}

#[test]
#[ignore = "daily"]
fn interception_does_not_send_item_use_or_release_packets() {
    with_owner(|calls| {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            game.client.screen = Screen::Playing;
            game.client.remote.as_mut().unwrap().loaded_sent = true;

            // An owner for the off-hand kind does not suppress normal off-hand use or release.
            game.offhand = Some(ItemStack::new(SHIELD, 1));
            game.set_mapping_held("key.use", true);
            game.queue_mapping_click(Button::Use.mapping());
            game.remote_handle_input();
            assert!(game.using.is_some());
            assert!(
                sent(&mut reader)
                    .iter()
                    .any(|packet| matches!(packet, S::UseItem(play::UseItem { hand: 1, .. })))
            );
            game.set_mapping_held("key.use", false);
            game.remote_handle_input();
            assert!(game.using.is_none());
            assert!(sent(&mut reader).iter().any(|packet| matches!(
                packet,
                S::PlayerAction(play::PlayerAction { action: 6, .. })
            )));
            assert!(calls.take().is_empty());

            // Now target a block with the registered kind in the main hand. Two presses in one
            // input drain both reach start-use handling, but only the first can capture it.
            let previous_use = game.use_start.item.clone();
            game.common.inventory[game.common.selected] = game.offhand.take();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world.set(0, 64, 0, stone).unwrap();
            game.set_test_forced_hit(Some(Hit {
                pos: [0, 64, 0],
                face: [0, 1, 0],
                point: [0.5, 65.0, 0.5],
                inside: false,
            }));
            game.set_mapping_held("key.use", true);
            game.queue_mapping_click(Button::Use.mapping());
            game.queue_mapping_click(Button::Use.mapping());
            game.remote_handle_input();
            assert!(game.test_has_client_item_use());
            assert!(game.using.is_none());

            game.use_delay = 0;
            game.remote_handle_input();
            game.remote_tick_using();
            assert!(game.using.is_none());
            assert_eq!(game.use_start.item, previous_use);

            game.set_mapping_held("key.use", false);
            game.remote_handle_input();
            assert!(!game.test_has_client_item_use());
            assert!(game.using.is_none());
            assert_eq!(
                calls
                    .take()
                    .into_iter()
                    .map(|call| call.phase)
                    .collect::<Vec<_>>(),
                [Phase::Start, Phase::Release(1), Phase::Stop(1)]
            );
            let packets = sent(&mut reader);
            assert!(
                !packets.iter().any(|packet| matches!(
                    packet,
                    S::UseItem(_)
                        | S::UseItemOn(_)
                        | S::Interact(_)
                        | S::PlayerAction(play::PlayerAction { action: 6, .. })
                )),
                "interception must never send use or release packets: {packets:?}"
            );
        });
    });
}

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
