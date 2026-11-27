//! Behavioural proofs of the broadcast events around scoped interaction Parts.
#[path = "interaction_matrix_tests.rs"]
pub(crate) mod interaction_matrix_tests;
#[path = "item_fallthrough_tests.rs"]
pub(crate) mod item_fallthrough_tests;
#[path = "item_used_on_block_tests.rs"]
pub(crate) mod item_used_on_block_tests;
#[cfg(feature = "client")]
#[path = "predicted_use_tests.rs"]
pub(crate) mod predicted_use_tests;
#[path = "use_item_hand_tests.rs"]
pub(crate) mod use_item_hand_tests;
#[path = "use_item_result_tests.rs"]
pub(crate) mod use_item_result_tests;
use mod_api::InteractionResult as Answer;
use mod_api::{
    self, BlockPart, ENTITY_ITEMS, Flow, Hooks, HostSide, HostTransaction, ItemPart, ItemResource,
    LivingEntityUseItemFinishEvent, LogicalSide, Registrar, RightClickBlockEvent,
    RightClickItemEvent, TriState, UseItemEvent, UseItemOnBlockEvent, UseItemOnEvent, UseOnEvent,
    UsePhase, UseWithoutItemEvent,
};
use std::sync::{Arc, Mutex};
use sunrise_core::geometry::Vec3;
#[cfg(feature = "client")]
use sunrise_core::nbt::Tag;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
#[cfg(feature = "client")]
use sunrise_protocol::connection::Pipe;
use {
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
    sunrise_gameplay_common::game::Hit,
};
use {
    sunrise_items::items::ItemStack, sunrise_level::chunk::Chunk,
    sunrise_sim::first_person_sim::Hand, sunrise_sim::motion::PlayerMotion,
};

#[cfg(feature = "client")]
#[cfg(any())]
impl sunrise_client_gameplay::game::ClientGame<'_> {
    pub(crate) fn set_sent_sneak_for_test(&mut self) {
        self.client.remote.as_mut().unwrap().last_input |=
            sunrise_gameplay_common::game::remote::SHIFT_INPUT;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::game) enum Step {
    Phase(UsePhase, LogicalSide),
    BlockPart,
    EmptyPart(Hand, bool),
    UseOnPart(LogicalSide),
    FirstPart,
    Air(LogicalSide, Hand),
    ItemPart,
}

#[derive(Default)]
pub(in crate::game) struct Controls {
    pub(in crate::game) steps: Vec<Step>,
    /// Whether the onItemUseFirst moment is recorded in `steps`.
    pub(in crate::game) record_first: bool,
    pub(in crate::game) first_answer: Option<Answer>,
    pub(in crate::game) first_part_answer: Option<Answer>,
    pub(in crate::game) block_answer: Option<Answer>,
    pub(in crate::game) block_part_answer: Option<Answer>,
    pub(in crate::game) block_part_select_slot: Option<u8>,
    pub(in crate::game) record_empty: bool,
    pub(in crate::game) empty_part_answer: Option<Answer>,
    pub(in crate::game) item_on_answer: Option<Answer>,
    pub(in crate::game) client_use_on_answer: Option<Answer>,
    pub(in crate::game) air_answer: Option<Answer>,
    pub(in crate::game) air_main_only: bool,
    pub(in crate::game) consume_item: bool,
    /// The block phase takes one item from the hand, before its answer if it gives one.
    pub(in crate::game) take_in_block: bool,
    pub(in crate::game) expected_hand: Option<Hand>,
    pub(in crate::game) part_answer: Option<Answer>,
    pub(in crate::game) part_pass: bool,
    /// Every hand `use_item_on_block` fired for, in order, instead of asserting one.
    pub(in crate::game) hands: Option<Vec<Hand>>,
    pub(in crate::game) use_block: TriState,
    pub(in crate::game) use_item: TriState,
    /// Each `living_entity_use_item_finish`: side, hand and the item and count of the result it
    /// arrived with.
    pub(in crate::game) finishes: Vec<(LogicalSide, Hand, Option<(String, u32)>)>,
    /// What a finish listener does to the result when set: `None` clears it, `Some(n)` sets its count.
    pub(in crate::game) finish_replacement: Option<Option<u32>>,
    /// Cancel `right_click_block` with this answer.
    pub(in crate::game) click_answer: Option<Answer>,
    /// How many `right_click_block` events fired.
    pub(in crate::game) clicks: usize,
    pub(in crate::game) click_hands: Vec<Hand>,
    pub(in crate::game) use_item_on_packets: usize,
    pub(in crate::game) use_item_packets: usize,
    /// The click is the host's own, not the fixture guest's: skip the guest identity asserts.
    pub(in crate::game) host_click: bool,
    /// Each `use_item` event: side, hand, `player` and `player_entity_id`.
    pub(in crate::game) item_users: Vec<(LogicalSide, Hand, u128, i32)>,
    /// How many `set_creative_mode_slot` packets the client sent.
    pub(in crate::game) creative_slot_packets: usize,
    pub(in crate::game) expect_empty_item_phase: bool,
}

#[derive(Clone, Default)]
pub(crate) struct Observer(pub(in crate::game) Arc<Mutex<Controls>>);

pub(crate) fn side(host: &HostSide<'_>) -> LogicalSide {
    match host {
        HostSide::Server(_) => LogicalSide::Server,
        HostSide::Client(_) => LogicalSide::Client,
    }
}

#[mod_api::__subscriptions]
impl Hooks for Observer {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for block in ["minecraft:lever", "minecraft:stone", "minecraft:campfire"] {
            let block_key = r.block_key(block);
            r.block_part(block_key, self.clone());
        }
        for item in ["minecraft:stick", "minecraft:shield"] {
            let item = r.item_key(item);
            r.item_part(item, self.clone());
        }
    }
    fn client_packet_send(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::ClientPacketSendEvent<'_>,
    ) -> Flow {
        match event.packet.id.path() {
            "use_item_on" => self.0.lock().unwrap().use_item_on_packets += 1,
            "use_item" => self.0.lock().unwrap().use_item_packets += 1,
            "set_creative_mode_slot" => self.0.lock().unwrap().creative_slot_packets += 1,
            _ => {}
        }
        Flow::Continue
    }
    fn right_click_block(&self, _: HostSide<'_>, event: &mut RightClickBlockEvent<'_>) -> Flow {
        let mut state = self.0.lock().unwrap();
        state.clicks += 1;
        state.click_hands.push(event.click.hand);
        if let Some(answer) = state.click_answer {
            *event.cancellation_result = answer;
            return Flow::Cancel;
        }
        *event.use_block = state.use_block;
        *event.use_item = state.use_item;
        Flow::Continue
    }
    fn living_entity_use_item_finish(
        &self,
        host: HostSide<'_>,
        event: &mut LivingEntityUseItemFinishEvent<'_>,
    ) {
        let mut state = self.0.lock().unwrap();
        state.finishes.push((
            side(&host),
            event.hand,
            event.result.as_ref().map(|r| {
                let item = mod_api::__private::item_key_id(r.item()).unwrap_or("?");
                (item.to_owned(), r.count())
            }),
        ));
        match state.finish_replacement {
            Some(None) => *event.result = None,
            Some(Some(count)) => {
                if let Some(result) = event.result.as_mut() {
                    result.set_count(count).unwrap();
                }
            }
            None => {}
        }
    }
    fn use_item_on_block(&self, host: HostSide<'_>, event: &mut UseItemOnBlockEvent<'_>) -> Flow {
        assert_eq!(side(&host), event.click.side);
        assert_eq!(event.click.pos, [0, 64, 0]);
        assert_eq!(event.click.hit.face, [0, 1, 0]);
        if event.click.side == LogicalSide::Server && !self.0.lock().unwrap().host_click {
            assert_eq!(event.click.player, 12);
            assert_eq!(event.click.player_entity_id, 12);
        }
        assert_eq!(*event.cancellation_result, Answer::TryWithEmptyHand);
        let mut state = self.0.lock().unwrap();
        if state.expect_empty_item_phase && event.phase == UsePhase::ItemAfterBlock {
            assert!(event.click.stack.is_none());
        }
        if let Some(hands) = &mut state.hands {
            hands.push(event.click.hand);
        } else {
            assert_eq!(event.click.hand, state.expected_hand.unwrap_or(Hand::Main));
        }
        if event.phase != UsePhase::ItemBeforeBlock || state.record_first {
            state.steps.push(Step::Phase(event.phase, event.click.side));
        }
        let answer = match event.phase {
            UsePhase::Block => state.block_answer,
            UsePhase::ItemAfterBlock => state.item_on_answer,
            UsePhase::ItemBeforeBlock => state.first_answer,
            _ => unreachable!("UsePhase: unhandled variant"),
        };
        if ((state.consume_item && answer.is_some() && event.phase == UsePhase::ItemAfterBlock)
            || (state.take_in_block && event.phase == UsePhase::Block))
            && let HostSide::Server(host) = host
        {
            let player = host.player(event.click.player).unwrap();
            let items = player.entity().capability(host, &ENTITY_ITEMS, ()).unwrap();
            let slot = player.selected_slot(host).unwrap();
            let resource = ItemResource::of(event.click.stack.unwrap());
            host.transaction(|host, tx| {
                assert_eq!(items.extract_at(host, tx, slot, &resource, 1), Ok(1));
                Ok::<_, ()>(())
            })
            .unwrap();
        }
        if let Some(answer) = answer {
            *event.cancellation_result = answer;
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn right_click_item(&self, host: HostSide<'_>, event: &mut RightClickItemEvent<'_>) -> Flow {
        assert_eq!(side(&host), event.click.side);
        assert_eq!(event.click.pos, [0, 64, 3]);
        if event.click.side == LogicalSide::Server && !self.0.lock().unwrap().host_click {
            assert_eq!(event.click.player, 12);
            assert_eq!(event.click.player_entity_id, 12);
        }
        assert_eq!(*event.cancellation_result, Answer::Pass);
        let mut state = self.0.lock().unwrap();
        state
            .steps
            .push(Step::Air(event.click.side, event.click.hand));
        if let Some(answer) = state.air_answer
            && (!state.air_main_only || event.click.hand == Hand::Main)
        {
            *event.cancellation_result = answer;
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}
#[mod_api::sunrise_part]
impl BlockPart for Observer {
    fn use_item_on(&self, host: HostSide<'_>, event: &mut UseItemOnEvent<'_>) -> Option<Answer> {
        assert_eq!(side(&host), event.click.side);
        let (select_slot, answer) = {
            let mut state = self.0.lock().unwrap();
            state.steps.push(Step::BlockPart);
            (state.block_part_select_slot, state.block_part_answer)
        };
        if let (Some(slot), HostSide::Client(host)) = (select_slot, host) {
            assert_eq!(host.select_hotbar_slot(slot), Ok(()));
        }
        answer
    }
    fn use_without_item(
        &self,
        host: HostSide<'_>,
        event: &mut UseWithoutItemEvent<'_>,
    ) -> Option<Answer> {
        assert_eq!(side(&host), event.click.side);
        let mut state = self.0.lock().unwrap();
        if state.record_empty {
            assert_eq!(event.click.hit.point, [0.5, 65.0, 0.5]);
            assert!(!event.click.hit.inside);
            state.steps.push(Step::EmptyPart(
                event.click.hand,
                event.click.stack.is_some(),
            ));
        }
        state.empty_part_answer
    }
}
#[mod_api::sunrise_part]
impl ItemPart for Observer {
    fn on_item_use_first(
        &self,
        host: HostSide<'_>,
        event: &mut mod_api::OnItemUseFirstEvent<'_>,
    ) -> Option<Answer> {
        assert_eq!(side(&host), event.click.side);
        let mut state = self.0.lock().unwrap();
        if state.record_first {
            state.steps.push(Step::FirstPart);
        }
        state.first_part_answer
    }
    fn use_on(&self, host: HostSide<'_>, _: &mut UseOnEvent<'_>) -> Option<Answer> {
        let mut state = self.0.lock().unwrap();
        let (side, answer) = match host {
            HostSide::Server(_) => (LogicalSide::Server, Answer::SuccessServer),
            HostSide::Client(_) => (LogicalSide::Client, state.client_use_on_answer?),
        };
        state.steps.push(Step::UseOnPart(side));
        Some(answer)
    }
    fn use_item(&self, host: HostSide<'_>, event: &mut UseItemEvent<'_>) -> Option<Answer> {
        if let Some(hand) = self.0.lock().unwrap().expected_hand {
            assert_eq!(event.hand, hand);
        }
        let mut state = self.0.lock().unwrap();
        state.item_users.push((
            side(&host),
            event.hand,
            event.player,
            event.player_entity_id,
        ));
        state.steps.push(Step::ItemPart);
        if state.part_pass {
            None
        } else {
            Some(state.part_answer.unwrap_or(Answer::SuccessServer))
        }
    }
}

#[cfg(feature = "client")]
pub(crate) fn with_interaction_client_game(
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    with_interaction_client_game_at(root, test);
}

#[cfg(feature = "client")]
pub(crate) fn with_interaction_client_game_at(
    root: std::path::PathBuf,
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    sunrise_client_gameplay::game::client_side::native_entity_refs::tests::with_game_role_client(
        root,
        ExecutionRole::Client,
        |game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "InteractionTest").unwrap();
            game.client.remote.as_mut().unwrap().logged_in = true;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 3.5)).unwrap());
            game.set_local_mode(sunrise_command::args::GameMode::Creative);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:stick", 1));
            test(game);
        },
    );
}

pub(crate) fn with_interaction_server_game(
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    with_interaction_server_game_at(root, test);
}

pub(crate) fn with_interaction_server_game_at(
    root: std::path::PathBuf,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    sunrise_gameplay::game::native_entity_refs::tests::with_game_at(root, |game| {
        game.options.dedicated = Some(Vec::new());
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 3.5)).unwrap());
        game.set_mode_id(1);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let _receiver = setup_interaction_guest(game);
        game.as_guest(12, |game| {
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:stick", 1));
            test(game);
        })
        .unwrap();
    });
}

pub(crate) trait InteractionSide<'g>: sunrise_gameplay_common::game::SideGame<'g> {
    #[cfg(feature = "client")]
    const IS_CLIENT: bool;
    #[cfg(feature = "client")]
    const LOGICAL_SIDE: LogicalSide;
    #[cfg(feature = "client")]
    fn set_interaction_mode(game: &mut Self, mode: sunrise_command::args::GameMode);
    fn interaction_use_block_hand(game: &mut Self, hand: Hand) -> bool;
    #[cfg(feature = "client")]
    fn interaction_use_air_hand(game: &mut Self, hand: Hand) -> bool;
    #[cfg(feature = "client")]
    fn interaction_use_item_as_player(game: &mut Self) -> bool;
    #[cfg(feature = "client")]
    fn interaction_set_sneak(game: &mut Self, sneak: bool);
    #[cfg(feature = "client")]
    fn interaction_clear_guest_use(game: &mut Self);
    #[cfg(feature = "client")]
    fn interaction_reset_use_failed(game: &mut Self);
}

pub(crate) fn use_block<'g, S: InteractionSide<'g>>(game: &mut S) -> bool {
    S::interaction_use_block_hand(game, Hand::Main)
}

pub(crate) fn hit() -> Hit {
    Hit {
        pos: [0, 64, 0],
        face: [0, 1, 0],
        point: [0.5, 65.0, 0.5],
        inside: false,
    }
}

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::remote_server::interaction_event_tests::setup_interaction_guest;

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_server_client_fires_use_item_finish_when_the_server_completes_the_use() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "finish_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_client_game(|game| {
            let offhand = sunrise_gameplay::game::shield::OFFHAND_SLOT;
            // (slot, creative, used stack, replacement, what the listener sees, what the slot
            // holds after), the expectations from vanilla's client finishUsingItem.
            let cases: [(
                usize,
                bool,
                (&str, u32),
                Option<Option<u32>>,
                Option<(&str, u32)>,
                Option<(&str, u32)>,
            ); 9] = [
                // A consumable loses one.
                (
                    0,
                    false,
                    ("minecraft:apple", 3),
                    None,
                    Some(("minecraft:apple", 2)),
                    Some(("minecraft:apple", 2)),
                ),
                // Nothing is used up in creative mode.
                (
                    0,
                    true,
                    ("minecraft:apple", 3),
                    None,
                    Some(("minecraft:apple", 3)),
                    Some(("minecraft:apple", 3)),
                ),
                // The last stew leaves its use remainder, the bowl, in the hand.
                (
                    0,
                    false,
                    ("minecraft:mushroom_stew", 1),
                    None,
                    Some(("minecraft:bowl", 1)),
                    Some(("minecraft:bowl", 1)),
                ),
                // A spyglass is not a consumable: it comes back as it was.
                (
                    0,
                    false,
                    ("minecraft:spyglass", 1),
                    None,
                    Some(("minecraft:spyglass", 1)),
                    Some(("minecraft:spyglass", 1)),
                ),
                // The last apple leaves nothing.
                (0, false, ("minecraft:apple", 1), None, None, None),
                // A replacement goes into the hand: survival, cleared, creative.
                (
                    0,
                    false,
                    ("minecraft:apple", 3),
                    Some(Some(7)),
                    Some(("minecraft:apple", 2)),
                    Some(("minecraft:apple", 7)),
                ),
                (
                    0,
                    false,
                    ("minecraft:apple", 3),
                    Some(None),
                    Some(("minecraft:apple", 2)),
                    None,
                ),
                (
                    0,
                    true,
                    ("minecraft:apple", 3),
                    Some(Some(7)),
                    Some(("minecraft:apple", 3)),
                    Some(("minecraft:apple", 7)),
                ),
                // The off hand, with a replacement.
                (
                    offhand,
                    false,
                    ("minecraft:apple", 2),
                    Some(Some(5)),
                    Some(("minecraft:apple", 1)),
                    Some(("minecraft:apple", 5)),
                ),
            ];
            let owned = |s: Option<(&str, u32)>| s.map(|(item, count)| (item.to_owned(), count));
            for (slot, creative, (item, count), replacement, seen, after) in cases {
                game.creative = creative;
                game.common.inventory[game.common.selected] = None;
                game.offhand = None;
                let slot = if slot == offhand {
                    offhand
                } else {
                    game.selected
                };
                *game.stack_in_mut(slot) = Some(ItemStack::new(item, count));
                game.using = Some((slot, 5));
                *observer.0.lock().unwrap() = Controls {
                    finish_replacement: replacement,
                    ..Controls::default()
                };
                game.remote_complete_using();
                let hand = if slot == offhand {
                    Hand::Off
                } else {
                    Hand::Main
                };
                assert_eq!(
                    observer.0.lock().unwrap().finishes,
                    [(LogicalSide::Client, hand, owned(seen))],
                    "{item} x{count} creative={creative} {replacement:?}"
                );
                let held = game.stack_in(slot).map(|s| (s.item.to_string(), s.count));
                assert_eq!(
                    held,
                    owned(after),
                    "{item} x{count} creative={creative} {replacement:?}"
                );
            }
            // The stack's use cooldown starts with the finish.
            game.creative = false;
            let mut cooling = ItemStack::new("minecraft:apple", 2);
            cooling.data_mut().set_other(
                "minecraft:use_cooldown",
                Some(Tag::Compound(
                    [
                        ("seconds".into(), Tag::Float(1.0_f32.to_bits())),
                        ("cooldown_group".into(), Tag::String("test:finish".into())),
                    ]
                    .into(),
                )),
            );
            game.cooldowns.remove("test:finish");
            game.common.inventory[game.common.selected] = Some(cooling);
            game.using = Some((game.selected, 5));
            *observer.0.lock().unwrap() = Controls::default();
            game.remote_complete_using();
            assert!(game.cooldowns.is_on_cooldown("test:finish"));
            // The server's inventory update replaces a mod's replacement.
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:apple", 3));
            game.using = Some((game.selected, 5));
            *observer.0.lock().unwrap() = Controls {
                finish_replacement: Some(Some(7)),
                ..Controls::default()
            };
            game.remote_complete_using();
            assert_eq!(game.held().map(|s| s.count), Some(7));
            let server = game.to_network(&Some(ItemStack::new("minecraft:apple", 2)));
            game.handle_packet(
                sunrise_protocol::packets::play::Clientbound::SetPlayerInventory(
                    sunrise_protocol::packets::play::SetPlayerInventory {
                        slot: game.selected as i32,
                        item: server,
                    },
                ),
            )
            .unwrap();
            assert_eq!(game.held().map(|s| s.count), Some(2));
        });
    });
}

/// Finishing a use sends no creative packet in vanilla (`LivingEntity.completeUsingItem`); its
/// inventory-slot edits go out through `CreativeInventoryListener`, which
/// `CreativeModeInventoryScreen` listens with only while it is open. (Hotbar loads and creative
/// drops send their own packets; they are not this path.) So a hand a finish listener replaced
/// in creative mode is not sent as `SetCreativeModeSlot` by either of this client's senders.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_creative_hand_replaced_by_a_finish_listener_is_not_sent_as_a_creative_slot() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "finish_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_client_game(|game| {
            let offhand = sunrise_gameplay::game::shield::OFFHAND_SLOT;
            let packets = |observer: &Observer| observer.0.lock().unwrap().creative_slot_packets;
            // Finish the use of `slot`, a listener setting the result's count to `count`.
            let finish = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                          slot: usize,
                          count: u32| {
                observer.0.lock().unwrap().finish_replacement = Some(Some(count));
                game.using = Some((slot, 5));
                game.remote_complete_using();
            };
            // Both senders: the per-tick scan and the one a creative click ends with.
            let sync = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                game.remote_send_state(0);
                game.send_creative_slots();
            };
            game.creative = true;
            game.selected = 2;
            game.inventory[2] = Some(ItemStack::new("minecraft:apple", 3));
            game.offhand = Some(ItemStack::new("minecraft:apple", 2));
            game.remote_synced();
            // The main hand.
            finish(game, 2, 7);
            assert_eq!(game.inventory[2].as_ref().map(|s| s.count), Some(7));
            sync(game);
            assert_eq!(packets(&observer), 0);
            // The off hand.
            finish(game, offhand, 9);
            assert_eq!(game.offhand.as_ref().map(|s| s.count), Some(9));
            sync(game);
            assert_eq!(packets(&observer), 0);
            // The selection moves on before the sync: the slot the use had is still synced.
            finish(game, 2, 4);
            game.selected = 5;
            sync(game);
            assert_eq!(game.inventory[2].as_ref().map(|s| s.count), Some(4));
            assert_eq!(packets(&observer), 0);
            // Two finishes before a sync.
            game.selected = 2;
            finish(game, 2, 6);
            finish(game, 2, 8);
            sync(game);
            assert_eq!(packets(&observer), 0);
            // An edit away and back to the synced value before a sync is no change.
            game.inventory[2] = Some(ItemStack::new("minecraft:apple", 5));
            game.inventory[2] = Some(ItemStack::new("minecraft:apple", 8));
            sync(game);
            assert_eq!(packets(&observer), 0);
            // A creative edit of the hand afterwards is still sent: one packet.
            game.inventory[2] = Some(ItemStack::new("minecraft:apple", 5));
            sync(game);
            assert_eq!(packets(&observer), 1);
        });
    });
}

#[path = "interaction_key_tests.rs"]
pub(crate) mod interaction_key_tests;
#[cfg(feature = "client")]
#[path = "mapping_press_tests.rs"]
pub(crate) mod mapping_press_tests;

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_server_client_keeps_a_bundle_use_until_the_server_completes_it() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "finish_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_client_game(|game| {
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:bundle", 1));
            assert!(game.start_using_item());
            // 200 client ticks: the use is still on, waiting for the server.
            for _ in 0..200 {
                game.remote_tick_using();
            }
            assert_eq!(game.using, Some((game.selected, 200)));
            game.remote_tick_using();
            assert_eq!(game.using, Some((game.selected, 201)));
            // Entity event 9 arrives late: the client-side finish fires once.
            game.remote_complete_using();
            assert_eq!(game.using, None);
            assert_eq!(
                observer.0.lock().unwrap().finishes,
                [(
                    LogicalSide::Client,
                    Hand::Main,
                    Some(("minecraft:bundle".to_owned(), 1))
                )]
            );
        });
    });
}

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
