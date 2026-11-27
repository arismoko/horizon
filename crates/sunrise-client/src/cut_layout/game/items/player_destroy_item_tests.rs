//! Full stack and hand proofs at real consumption and durability write paths.
#![cfg(feature = "client")]
use mod_api::{Hooks, HostSide, PlayerDestroyItemEvent};
use std::sync::{Arc, Mutex};
#[cfg(test)]
use sunrise_gameplay::game::shield;
use sunrise_protocol::connection::Pipe;
use {
    crate::game::*,
    sunrise_gameplay_common::game::{
        Hand, InteractTarget, ItemStack, PlayerMotion, Screen, Vec3, in_own_process,
    },
};

#[derive(Debug)]
struct Notice {
    pub client: bool,
    player: u128,
    entity: i32,
    hand: Option<Hand>,
    original: mod_api::ItemStack,
    /// What the notified player's hand held while the listener ran.
    in_hand: Option<mod_api::ItemStack>,
    resolved: bool,
}

enum HandWrite {
    Replace(Hand, ItemStack),
    MoveBowAndReplace(ItemStack),
}

#[derive(Default)]
struct Observations {
    pub notices: Vec<Notice>,
    mining_order: Vec<&'static str>,
    hand_write: Option<HandWrite>,
    refuse_use: bool,
}

#[derive(Clone, Default)]
struct Observer(Arc<Mutex<Observations>>);

#[mod_api::__subscriptions]
impl Hooks for Observer {
    fn player_destroy_item(&self, host: HostSide<'_>, event: &mut PlayerDestroyItemEvent<'_>) {
        let client = matches!(host, HostSide::Client(_));
        let id = u32::try_from(event.player_entity_id).unwrap();
        let host_ref: &dyn mod_api::Host = match &host {
            HostSide::Server(host) => &**host,
            HostSide::Client(host) => &**host,
        };
        let living = host_ref
            .entity(id)
            .and_then(|entity| host_ref.living(entity));
        let in_hand = event.hand.zip(living).and_then(|(hand, living)| {
            living
                .with_hand_item(host_ref, hand, |stack| stack.project())
                .flatten()
        });
        let mut seen = self.0.lock().unwrap();
        seen.mining_order.push("destroy");
        seen.notices.push(Notice {
            in_hand,
            resolved: living.is_some(),
            client,
            player: event.player,
            entity: event.player_entity_id,
            hand: event.hand,
            original: event.original.clone(),
        });
        let hand_write = seen.hand_write.take();
        drop(seen);
        if let (HostSide::Server(host), Some(write)) = (host, hand_write) {
            let living = living.unwrap();
            let (hand, stack) = match write {
                HandWrite::Replace(hand, stack) => (hand, stack),
                HandWrite::MoveBowAndReplace(stack) => {
                    let bow = living
                        .with_hand_item(host, Hand::Main, |stack| stack.project())
                        .flatten()
                        .unwrap();
                    assert!(living.set_hand_item(host, Hand::Off, Some(bow)).unwrap());
                    (Hand::Main, stack)
                }
            };
            assert!(
                living
                    .set_hand_item(host, hand, Some(stack.try_into_api().unwrap()))
                    .unwrap()
            );
        }
    }

    fn right_click_item(
        &self,
        host: HostSide<'_>,
        event: &mut mod_api::RightClickItemEvent<'_>,
    ) -> mod_api::Flow {
        if !self.0.lock().unwrap().refuse_use {
            return mod_api::Flow::Continue;
        }
        let HostSide::Server(host) = host else {
            panic!("the move/refuse scenario runs on the server");
        };
        assert_eq!(event.click.hand, Hand::Main);
        let entity = host
            .entity(u32::try_from(event.click.player_entity_id).unwrap())
            .unwrap();
        let living = host.living(entity).unwrap();
        let stack = living
            .with_hand_item(host, Hand::Main, |stack| stack.project())
            .flatten()
            .unwrap();
        assert!(living.set_hand_item(host, Hand::Off, Some(stack)).unwrap());
        assert!(living.set_hand_item(host, Hand::Main, None).unwrap());
        *event.cancellation_result = mod_api::InteractionResult::Fail;
        mod_api::Flow::Cancel
    }

    fn block_drops(
        &self,
        _: &dyn mod_api::LevelHost,
        _: &mut mod_api::BlockDropsEvent<'_>,
    ) -> mod_api::Flow {
        self.0.lock().unwrap().mining_order.push("drops");
        mod_api::Flow::Continue
    }
}

fn named_stack(item: &str, count: u32) -> ItemStack {
    let mut stack = ItemStack::new(item, count);
    stack.data_mut().set_loot_name(
        "minecraft:custom_name",
        &serde_json::json!("Destruction snapshot"),
    );
    stack
}

fn take_notice(observer: &Observer, hand: Option<Hand>, item: &str, damage: u32) -> Notice {
    let mut seen = observer.0.lock().unwrap();
    assert_eq!(seen.notices.len(), 1, "each exhausted stack is heard once");
    let notice = seen.notices.pop().unwrap();
    assert_eq!(notice.hand, hand);
    assert_eq!(notice.original.count(), 1);
    assert_eq!(notice.original.damage(), damage);
    assert_eq!(
        mod_api::__private::item_key_id(notice.original.item()).unwrap(),
        item
    );
    assert_eq!(
        notice
            .original
            .components()
            .and_then(|components| components.as_nbt().get(&"minecraft:custom_name".into())),
        Some(&mod_api::Tag::String("Destruction snapshot".into())),
        "the snapshot keeps the literal custom name"
    );
    notice
}

#[test]
#[ignore = "daily"]
fn original_stack_and_hand_reach_used_up_and_broken_item_notices() {
    in_own_process(
        "cut_layout::game::player_destroy_item_tests::original_stack_and_hand_reach_used_up_and_broken_item_notices",
        run_isolated,
    );
}

fn refused_use_preserves_the_moved_stack(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    observer: &Observer,
) {
    let book = named_stack("minecraft:knowledge_book", 1);
    game.common.inventory[game.common.selected] = Some(book.clone());
    game.offhand = None;
    observer.0.lock().unwrap().refuse_use = true;
    assert!(!game.use_hand(None).unwrap());
    observer.0.lock().unwrap().refuse_use = false;
    assert!(game.held().is_none());
    assert_eq!(
        game.offhand.as_ref().unwrap().try_into_api().unwrap(),
        book.try_into_api().unwrap()
    );
    assert!(observer.0.lock().unwrap().notices.is_empty());
    game.offhand = None;
}

fn mining_reports_after_the_drop_hook(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    observer: &Observer,
) {
    use sunrise_core::position::ChunkPos;
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    let pos = [2, 64, 2];
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(pos[0], pos[1], pos[2], stone).unwrap();
    let mut tool = named_stack("minecraft:iron_pickaxe", 1);
    tool.damage = 249;
    game.common.inventory[game.common.selected] = Some(tool);
    observer.0.lock().unwrap().mining_order.clear();
    assert!(game.player_break_block(pos, None).unwrap());
    assert!(game.held().is_none());
    assert_ne!(game.world.block(pos[0], pos[1], pos[2]), stone);
    assert!(
        game.dropped
            .iter()
            .any(|item| &*item.stack.item == "minecraft:cobblestone")
    );
    assert_eq!(
        observer.0.lock().unwrap().mining_order,
        ["drops", "destroy"]
    );
    take_notice(observer, Some(Hand::Main), "minecraft:iron_pickaxe", 249);
}

fn continuations_preserve_listener_hand_writes(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    observer: &Observer,
) {
    // A broken steering stick must not install its fishing rod over the new stack.
    let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
    game.mobs
        .push(sunrise_sim::mobs::Mob::new(21, pig, Vec3::ZERO, 0.0));
    game.riding = Some(21);
    let mut stick = named_stack("minecraft:carrot_on_a_stick", 1);
    stick.damage = 24;
    game.common.inventory[game.common.selected] = Some(stick);
    observer.0.lock().unwrap().hand_write = Some(HandWrite::Replace(
        Hand::Main,
        ItemStack::new("minecraft:apple", 1),
    ));
    assert!(game.use_steering_stick("minecraft:carrot_on_a_stick"));
    take_notice(
        observer,
        Some(Hand::Main),
        "minecraft:carrot_on_a_stick",
        24,
    );
    assert_eq!(&*game.held().unwrap().item, "minecraft:apple");
    game.riding = None;

    // The same applies to a powder snow bucket's pending empty bucket.
    let powder = named_stack("minecraft:powder_snow_bucket", 1);
    game.offhand = Some(powder.clone());
    observer.0.lock().unwrap().hand_write = Some(HandWrite::Replace(
        Hand::Off,
        ItemStack::new("minecraft:stick", 1),
    ));
    game.with_hand(Hand::Off, |game| game.consume_placed(&powder));
    take_notice(observer, Some(Hand::Off), "minecraft:powder_snow_bucket", 0);
    assert_eq!(&*game.offhand.as_ref().unwrap().item, "minecraft:stick");

    // The notice may move the loaded bow and install a different one. The
    // original bow must keep its projectile; the replacement gets none.
    let bow = ItemStack::new("minecraft:crossbow", 1);
    let mut replacement = bow.clone();
    replacement.damage = 2;
    let slot = game.selected;
    game.inventory[slot] = Some(bow.clone());
    game.offhand = Some(named_stack("minecraft:arrow", 1));
    observer.0.lock().unwrap().hand_write = Some(HandWrite::MoveBowAndReplace(replacement));
    game.tick_crossbow(
        slot,
        sunrise_gameplay_common::game::CommonGame::charge_ticks(&bow),
        &bow,
    );
    let notice = take_notice(observer, Some(Hand::Off), "minecraft:arrow", 0);
    assert!(
        notice.in_hand.is_none(),
        "the ammo was debited before the move"
    );
    let moved = game.offhand.as_ref().unwrap();
    assert_eq!(&*moved.item, "minecraft:crossbow");
    assert_eq!(moved.damage, 0);
    let [arrow] = moved.data().unwrap().charged.as_slice() else {
        panic!("the moved bow must keep its one arrow");
    };
    assert_eq!(&*arrow.item, "minecraft:arrow");
    assert_eq!(arrow.count, 1);
    assert_eq!(arrow.custom_name().as_deref(), Some("Destruction snapshot"));
    let kept = game.stack_in(slot).unwrap();
    assert_eq!(&*kept.item, "minecraft:crossbow");
    assert_eq!(kept.damage, 2);
    assert!(kept.data().is_none_or(|data| data.charged.is_empty()));

    // A data pack can tag the crossbow itself as ammunition. The original
    // must be the uncharged stack, not a crossbow that already loaded itself.
    let items = game.items().with_tags(
        None,
        Some(std::collections::HashMap::from([(
            "minecraft:arrows".into(),
            vec!["minecraft:crossbow".into()],
        )])),
    );
    let previous_items = game.server_items.replace(Arc::new(items));
    let bow = named_stack("minecraft:crossbow", 1);
    game.inventory[slot] = Some(bow.clone());
    game.offhand = None;
    game.using = None;
    game.tick_crossbow(
        slot,
        sunrise_gameplay_common::game::CommonGame::charge_ticks(&bow),
        &bow,
    );
    assert!(game.stack_in(slot).is_none());
    let notice = take_notice(observer, Some(Hand::Main), "minecraft:crossbow", 0);
    assert_eq!(notice.original, bow.try_into_api().unwrap());
    game.server_items = previous_items;
    game.using = None;
}

fn run_isolated() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "destroy_observer",
        Box::new(observer.clone()),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        const PLAYER: u128 = 0x0000_0001_0000_0002_0000_0003_0000_0004;
        native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            native_entity_refs::tests::bind_local_player(game, 71, PLAYER);
            game.set_mode_id(0);
            game.selected = 0;
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            // Both hands, partial consumption, and creative preservation.
            for hand in Hand::BOTH {
                game.with_hand(hand, |game| {
                    *game.held_mut() = Some(named_stack("minecraft:apple", 2));
                    game.consume_held();
                    assert!(observer.0.lock().unwrap().notices.is_empty());
                    game.creative = true;
                    game.consume_held();
                    assert_eq!(game.held().unwrap().count, 1);
                    assert!(observer.0.lock().unwrap().notices.is_empty());
                    game.creative = false;
                    game.consume_held();
                    assert!(game.held().is_none());
                    let notice = take_notice(&observer, Some(hand), "minecraft:apple", 0);
                    assert!(!notice.client);
                    assert_eq!(notice.entity, 71);
                    assert_eq!(notice.player, PLAYER);
                });
            }
            // Actual Item.use: a knowledge book is exhausted even when it has no recipes.
            game.offhand = Some(named_stack("minecraft:knowledge_book", 1));
            assert!(!game.with_hand(Hand::Off, |g| g.use_hand(None)).unwrap());
            assert!(game.offhand.is_none());
            take_notice(&observer, Some(Hand::Off), "minecraft:knowledge_book", 0);

            // Completed use must report the consumed potion, not its glass bottle remainder.
            let potion = named_stack("minecraft:potion", 1);
            game.offhand = Some(potion.clone());
            let info = game.items().get(&potion.item).unwrap().clone();
            game.with_hand(Hand::Off, |g| {
                g.complete_using_item(shield::OFFHAND_SLOT, &potion, &info)
            });
            assert_eq!(
                &*game.offhand.as_ref().unwrap().item,
                "minecraft:glass_bottle"
            );
            let drunk = take_notice(&observer, Some(Hand::Off), "minecraft:potion", 0);
            assert!(
                drunk.resolved,
                "the listener resolves the player's living handle"
            );
            assert!(
                drunk.in_hand.is_none(),
                "the potion is gone and the bottle is not installed at dispatch"
            );

            refused_use_preserves_the_moved_stack(game, &observer);
            mining_reports_after_the_drop_hook(game, &observer);
            continuations_preserve_listener_hand_writes(game, &observer);

            // The same notice identifies a swapped-in guest rather than the local host.
            game.server.lan_acting_uuid = Some(0x1111_2222_3333_4444_5555_6666_7777_8888);
            game.server.lan_acting_entity = Some(123);
            game.offhand = Some(named_stack("minecraft:apple", 1));
            game.with_hand(Hand::Off, |game| game.consume_held());
            let guest = take_notice(&observer, Some(Hand::Off), "minecraft:apple", 0);
            assert_eq!(guest.player, 0x1111_2222_3333_4444_5555_6666_7777_8888);
            assert_eq!(guest.entity, 123);
            game.server.lan_acting_uuid = None;
            game.server.lan_acting_entity = None;

            // Native entity interaction: the last wheat is eaten by an adult cow.
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.clear();
            game.mobs.push(sunrise_sim::mobs::Mob::new(
                42,
                kind,
                Vec3::new(0.5, 64.0, 2.5),
                0.0,
            ));
            game.offhand = Some(named_stack("minecraft:wheat", 1));
            assert!(
                game.with_hand(Hand::Off, |g| g.interact_entity(Some(InteractTarget {
                    id: 42,
                    location: [0.0, 1.0, 0.0],
                })))
                .unwrap()
            );
            assert!(game.offhand.is_none());
            take_notice(&observer, Some(Hand::Off), "minecraft:wheat", 0);

            // Immediate durability reports pre-wear damage.
            let mut tool = named_stack("minecraft:iron_pickaxe", 1);
            tool.damage = 249;
            game.offhand = Some(tool);
            game.with_hand(Hand::Off, |g| g.damage_held(1));
            assert!(game.offhand.is_none());
            take_notice(&observer, Some(Hand::Off), "minecraft:iron_pickaxe", 249);

            // Armour has no hand; handle-driven wear must keep the same original snapshot.
            let mut helmet = named_stack("minecraft:iron_helmet", 1);
            helmet.damage = 164;
            game.armor[0] = Some(helmet);
            game.damage_player_equipment(mod_api::EquipmentSlot::Head, 1);
            assert!(game.armor[0].is_none());
            take_notice(&observer, None, "minecraft:iron_helmet", 164);

            // Equipping into an empty armour slot moves the stack: nothing is used up.
            game.offhand = Some(named_stack("minecraft:iron_helmet", 1));
            game.with_hand(Hand::Off, |g| g.use_hand(None)).unwrap();
            assert!(game.armor[0].is_some() && game.offhand.is_none());
            assert!(observer.0.lock().unwrap().notices.is_empty());
        });
        native_entity_refs::tests::with_client_game(|game| {
            let (local, _peer) = Pipe::pair();
            game.connect_local(local, "DestroyTest").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 71;
            remote.profile = PLAYER;
            let mut player = PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap();
            player.entity_id = 71;
            game.player = Some(player);
            game.set_local_mode(sunrise_command::args::GameMode::Survival);
            game.selected = 0;
            game.client.screen = Screen::Playing;
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            // Both hands, partial consumption, and creative preservation.
            for hand in Hand::BOTH {
                game.with_hand(hand, |game| {
                    *game.held_mut() = Some(named_stack("minecraft:apple", 2));
                    game.consume_held();
                    assert!(observer.0.lock().unwrap().notices.is_empty());
                    game.creative = true;
                    game.consume_held();
                    assert_eq!(game.held().unwrap().count, 1);
                    assert!(observer.0.lock().unwrap().notices.is_empty());
                    game.creative = false;
                    game.consume_held();
                    assert!(game.held().is_none());
                    let notice = take_notice(&observer, Some(hand), "minecraft:apple", 0);
                    assert!(notice.client);
                    assert_eq!(notice.entity, 71);
                });
            }
            // Actual Item.use: a knowledge book is exhausted even when it has no recipes.
            game.offhand = Some(named_stack("minecraft:knowledge_book", 1));
            assert!(!game.remote_use_item(Hand::Off));
            assert!(game.offhand.is_none());
            take_notice(&observer, Some(Hand::Off), "minecraft:knowledge_book", 0);

            // Completed use must report the consumed potion, not its glass bottle remainder.
            let potion = named_stack("minecraft:potion", 1);
            game.offhand = Some(potion.clone());
            let info = game.items().get(&potion.item).unwrap().clone();
            game.remote_use_item_finished(shield::OFFHAND_SLOT, &potion, &info);
            assert_eq!(
                &*game.offhand.as_ref().unwrap().item,
                "minecraft:glass_bottle"
            );
            let drunk = take_notice(&observer, Some(Hand::Off), "minecraft:potion", 0);
            assert!(
                drunk.resolved,
                "the listener resolves the player's living handle"
            );
            assert!(
                drunk.in_hand.is_none(),
                "the potion is gone and the bottle is not installed at dispatch"
            );

            // Equipment status can arrive with a stale stack in its slot. It
            // drives effects only, not a notice claiming an exact original.
            let stale = named_stack("minecraft:iron_pickaxe", 1);
            game.offhand = Some(stale.clone());
            use sunrise_protocol::packets::play::{Clientbound, EntityEvent};
            game.apply_packet(Clientbound::EntityEvent(EntityEvent { id: 71, event: 48 }))
                .unwrap();
            assert_eq!(game.offhand.as_ref(), Some(&stale));
            assert!(observer.0.lock().unwrap().notices.is_empty());
        });
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
