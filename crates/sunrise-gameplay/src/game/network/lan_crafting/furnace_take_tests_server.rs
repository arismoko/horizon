//! Furnace result-take (`item_smelted`) and recipe-experience ordering on the
//! real LAN furnace menu path (`FurnaceResultSlot.checkTakeAchievements` and
//! `AbstractFurnaceBlockEntity.awardUsedRecipesAndPopExperience`).

use super::*;
use crate::game::ServerGame;
use crate::game::native_guest_turn_tests::with_guests;
use std::sync::{Arc, Mutex};
use sunrise_level::{
    chunk::Chunk,
    containers::{BlockEntity, Furnace, FurnaceKind},
};
use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
use sunrise_sim::lan::menu::input;

const FURNACE_POS: (i32, i32, i32) = (1, 64, 1);

#[derive(Debug, PartialEq, Eq)]
struct SmeltedCall {
    is_client: bool,
    uuid: u128,
    entity_id: u32,
    player_name: Option<String>,
    item: Option<String>,
    count: u32,
    projected: Option<(Option<String>, u32)>,
    amount_removed: u32,
    xp_seen: i64,
}

#[derive(Default)]
struct Recorded {
    cooking_smelts: Vec<([i32; 3], String, String)>,
    item_smelted: Vec<SmeltedCall>,
}

struct FurnaceProbe {
    recorded: Arc<Mutex<Recorded>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for FurnaceProbe {
    fn furnace_smelt(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::FurnaceSmeltEvent<'_>,
    ) {
        self.recorded.lock().unwrap().cooking_smelts.push((
            event.pos,
            event.item.to_owned(),
            event.result.to_owned(),
        ));
    }

    fn item_smelted(&self, host: mod_api::HostSide<'_>, event: &mut mod_api::ItemSmeltedEvent<'_>) {
        let (is_client, xp_seen, player_name, item, projected) = match host {
            mod_api::HostSide::Server(host) => {
                let mut xp_seen = 0;
                let level = host.level("minecraft:overworld").unwrap();
                let _ = level
                    .visit_experience_orbs(host, [0.0, 60.0, 0.0], [10.0, 70.0, 10.0], &mut |orb| {
                        xp_seen += i64::from(orb.value(host).unwrap())
                            * i64::from(orb.count(host).unwrap());
                        core::ops::ControlFlow::Continue(())
                    })
                    .unwrap();
                let projected = event
                    .stack
                    .project()
                    .map(|stack| (stack.item().id(host).map(str::to_owned), stack.count()));
                (
                    false,
                    xp_seen,
                    event.player.name(host).map(str::to_owned),
                    event.stack.item().id(host).map(str::to_owned),
                    projected,
                )
            }
            mod_api::HostSide::Client(host) => {
                let projected = event
                    .stack
                    .project()
                    .map(|stack| (stack.item().id(host).map(str::to_owned), stack.count()));
                (
                    true,
                    0,
                    None,
                    event.stack.item().id(host).map(str::to_owned),
                    projected,
                )
            }
        };
        self.recorded
            .lock()
            .unwrap()
            .item_smelted
            .push(SmeltedCall {
                is_client,
                uuid: event.player.uuid(),
                entity_id: event.player.entity().id(),
                player_name,
                item,
                count: event.stack.count(),
                projected,
                amount_removed: event.amount_removed,
                xp_seen,
            });
    }
}

fn with_probe(run: impl FnOnce(&Arc<Mutex<Recorded>>)) {
    let recorded = Arc::new(Mutex::new(Recorded::default()));
    let probe = FurnaceProbe {
        recorded: Arc::clone(&recorded),
    };
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("furnace_probe", Box::new(probe))].into(),
        || run(&recorded),
    );
}

/// Place a furnace, cook two raw iron through `tick_furnace` (so `recipes_used`
/// carries 2 * 0.7 = 1.4 experience, guaranteed >= 1 XP on every seed), and
/// open its menu for guest 12.
fn smelt_two_and_open(game: &mut ServerGame<'_>) -> i32 {
    game.common
        .world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let state = game
        .common
        .pack
        .assets
        .registry
        .default_state("minecraft:furnace")
        .unwrap();
    game.common
        .world
        .set(FURNACE_POS.0, FURNACE_POS.1, FURNACE_POS.2, state)
        .unwrap();
    game.common.world.block_entities.insert(
        FURNACE_POS,
        BlockEntity::Furnace(Furnace {
            kind: FurnaceKind::Furnace,
            items: [
                Some(ItemStack::new("minecraft:raw_iron", 2)),
                Some(ItemStack::new("minecraft:coal", 1)),
                None,
            ],
            lit_remaining: 20,
            lit_total: 200,
            cooking_timer: 199,
            cooking_total: 200,
            speed: 1.0,
            recipes_used: Default::default(),
        }),
    );

    game.tick_furnace(FURNACE_POS).unwrap();
    if let Some(BlockEntity::Furnace(furnace)) =
        game.common.world.block_entities.get_mut(&FURNACE_POS)
    {
        furnace.cooking_timer = 199;
    }
    game.tick_furnace(FURNACE_POS).unwrap();

    game.open_block_menu(12, FURNACE_POS);
    game.server.lan.as_ref().unwrap().hub.guests[0]
        .menu
        .as_ref()
        .unwrap()
        .window
}

fn click_result(window: i32, button: i8, click_input: i32) -> Serverbound {
    Serverbound::ContainerClick(ContainerClick {
        container: window,
        state: 0,
        slot: 2,
        button,
        input: click_input,
        changed: Vec::new(),
        carried: None,
    })
}

fn recipes_used_empty(game: &ServerGame<'_>) -> bool {
    let Some(BlockEntity::Furnace(furnace)) = game.common.world.block_entities.get(&FURNACE_POS)
    else {
        panic!("furnace missing");
    };
    furnace.recipes_used.is_empty()
}

fn total_orb_xp(game: &ServerGame<'_>) -> i64 {
    game.common
        .orbs
        .iter()
        .map(|orb| i64::from(orb.value) * i64::from(orb.count))
        .sum()
}

#[test]
#[ignore = "daily"]
fn pickup_awards_xp_before_item_smelted_and_keeps_cooking_separate() {
    with_probe(|recorded| {
        with_guests(|game| {
            let window = smelt_two_and_open(game);

            // Cooking ticks fired furnace_smelt twice and never item_smelted.
            {
                let seen = recorded.lock().unwrap();
                assert_eq!(seen.cooking_smelts.len(), 2);
                assert_eq!(
                    seen.cooking_smelts[0],
                    (
                        [FURNACE_POS.0, FURNACE_POS.1, FURNACE_POS.2],
                        "minecraft:raw_iron".to_owned(),
                        "minecraft:iron_ingot".to_owned(),
                    )
                );
                assert!(seen.item_smelted.is_empty());
            }
            assert!(!recipes_used_empty(game));
            let used: Vec<String> = match game.common.world.block_entities.get(&FURNACE_POS) {
                Some(BlockEntity::Furnace(furnace)) => {
                    furnace.recipes_used.keys().cloned().collect()
                }
                _ => unreachable!(),
            };
            assert!(!used.is_empty());

            game.lan_guest_packet(12, click_result(window, 0, input::PICKUP));
            // The extracting guest learns what the furnace used (`ServerPlayer.awardRecipes`).
            let learned = game
                .as_guest(12, |g| {
                    used.iter().all(|id| g.common.level.recipe_book.knows(id))
                })
                .unwrap();
            assert!(learned, "{used:?}");

            let seen = recorded.lock().unwrap();
            assert_eq!(seen.item_smelted.len(), 1);
            let call = &seen.item_smelted[0];
            assert!(!call.is_client);
            assert_eq!(call.uuid, 12);
            assert_eq!(call.entity_id, 12);
            assert_eq!(call.player_name.as_deref(), Some("guest12"));
            assert_eq!(call.item.as_deref(), Some("minecraft:iron_ingot"));
            assert_eq!(call.count, 2);
            assert_eq!(
                call.projected,
                Some((Some("minecraft:iron_ingot".to_owned()), 2))
            );
            assert_eq!(call.amount_removed, 2);
            assert!(
                call.xp_seen >= 1,
                "experience orbs must already be spawned before item_smelted runs"
            );
            assert!(recipes_used_empty(game));
        });
    });
}

#[test]
#[ignore = "daily"]
fn quick_move_fires_item_smelted_once_and_swap_awards_xp_without_event() {
    with_probe(|recorded| {
        with_guests(|game| {
            // Shift-click fires item_smelted once (never a second 0-removed call).
            let window = smelt_two_and_open(game);
            game.lan_guest_packet(12, click_result(window, 0, input::QUICK_MOVE));
            {
                let mut seen = recorded.lock().unwrap();
                assert_eq!(seen.item_smelted.len(), 1);
                assert_eq!(seen.item_smelted[0].amount_removed, 2);
                assert_eq!(seen.item_smelted[0].count, 2);
                assert!(seen.item_smelted[0].xp_seen >= 1);
                seen.item_smelted.clear();
            }
            assert!(recipes_used_empty(game));

            // Hotbar swap has removeCount == 0: awards XP/recipes without firing item_smelted.
            let xp_before = total_orb_xp(game);
            let window = smelt_two_and_open(game);
            game.lan_guest_packet(12, click_result(window, 1, input::SWAP));

            assert!(recorded.lock().unwrap().item_smelted.is_empty());
            assert!(recipes_used_empty(game));
            assert!(
                total_orb_xp(game) > xp_before,
                "hotbar swap must still spawn furnace experience orbs"
            );
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].inventory[1],
                Some(ItemStack::new("minecraft:iron_ingot", 2))
            );
        });
    });
}
