//! Real LAN merchant result takes, with native listeners and the production coordinator.
use super::native_guest_turn_tests::with_guests;
use mod_api::{
    Flow, Hooks, HostSide, HostTransaction, ItemPlace, ItemResource, ItemStackedOnOtherEvent,
    ModEntry, Mods, ServerHost, TradeWithVillagerEvent,
};
use std::cell::{Cell, RefCell};
use std::sync::{Arc, mpsc};
use sunrise_items::items::ItemStack;
use sunrise_protocol::packets::play::{Clientbound, ContainerClick};
use sunrise_sim::lan::{Out, PlayerSender};
use sunrise_sim::{
    merchant::{Session, Trader},
    mobs::finalize::{self, Attr, Op},
};

#[derive(Clone, Copy)]
pub(in crate::game) enum PlayerWrite {
    Unrelated,
    ShiftDestination { restore: bool },
}

#[derive(Clone, Copy)]
pub(in crate::game) enum EffectWrite {
    ReplaceSlowness,
    ReapplySpeed,
    Clear,
    ReplaceTransient,
}

pub(crate) const SPEED: &str = "minecraft:movement_speed";

#[derive(Clone, Default)]
pub(in crate::game) struct Script {
    pub(in crate::game) panic_on: Option<usize>,
    pub(in crate::game) answer: bool,
    pub(in crate::game) invalid: bool,
    /// An answer whose result stack is not a valid one for the game.
    pub(in crate::game) invalid_stack: bool,
    pub(in crate::game) edit_other_merchant_and_board: bool,
    pub(in crate::game) effect_write: Option<EffectWrite>,
    pub(in crate::game) player_write: Option<PlayerWrite>,
    pub(in crate::game) raise_second_price: bool,
    pub(in crate::game) refuse_stacked: bool,
    pub(in crate::game) replace_stacked_result: bool,
}
thread_local! {
    pub(in crate::game) static SCRIPT: RefCell<Script> = RefCell::new(Script::default());
    pub(in crate::game) static CALLS: Cell<usize> = const { Cell::new(0) };
    pub(in crate::game) static STACKED: Cell<usize> = const { Cell::new(0) };
    pub(in crate::game) static STAGED_WRITES: Cell<bool> = const { Cell::new(false) };
    pub(in crate::game) static USES: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    pub(in crate::game) static IN_ACTION: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}
fn script(value: Script) {
    SCRIPT.with(|s| *s.borrow_mut() = value);
    CALLS.set(0);
    STACKED.set(0);
    STAGED_WRITES.set(false);
    USES.with(|uses| uses.borrow_mut().clear());
    IN_ACTION.with(|a| a.borrow_mut().clear());
    sunrise_gameplay::game::server_side::decided_action::DECIDED.set(0);
}
pub(crate) struct Mod;
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn trade_with_villager(
        &self,
        host: &mut dyn ServerHost,
        event: &mut TradeWithVillagerEvent<'_>,
    ) {
        trade(host, event);
    }
}
/// The same trade listener, also hearing `item_stacked_on_other`: the result pickup is both.
pub(crate) struct Both;
#[mod_api::__subscriptions]
impl Hooks for Both {
    fn trade_with_villager(
        &self,
        host: &mut dyn ServerHost,
        event: &mut TradeWithVillagerEvent<'_>,
    ) {
        trade(host, event);
    }

    fn item_stacked_on_other(
        &self,
        host: HostSide,
        event: &mut ItemStackedOnOtherEvent<'_, '_>,
    ) -> Flow {
        Stacked.item_stacked_on_other(host, event)
    }
}

pub(crate) struct Stacked;
#[mod_api::__subscriptions]
impl Hooks for Stacked {
    fn item_stacked_on_other(
        &self,
        _: HostSide,
        event: &mut ItemStackedOnOtherEvent<'_, '_>,
    ) -> Flow {
        STACKED.set(STACKED.get() + 1);
        let (refuse, replace) = SCRIPT.with(|s| {
            let s = s.borrow();
            (s.refuse_stacked, s.replace_stacked_result)
        });
        assert!(!refuse, "stacked listener failed");
        if replace {
            event.click.set_clicked(None);
            event.cancelled = true;
            return Flow::Cancel;
        }
        Flow::Continue
    }
}
pub(crate) fn trade(host: &mut dyn ServerHost, event: &mut TradeWithVillagerEvent<'_>) {
    {
        let call = CALLS.get() + 1;
        CALLS.set(call);
        IN_ACTION.with(|a| a.borrow_mut().push(!host.transaction_matches(None)));
        assert_eq!(event.merchant, 40);
        USES.with(|uses| uses.borrow_mut().push(event.offer.uses));
        let s = SCRIPT.with(|s| s.borrow().clone());
        if let Some(write) = s.player_write {
            let key = mod_api::__private::item_key("minecraft:dirt").unwrap();
            let dirt = ItemResource::of(&mod_api::ItemStack::new(key, 1).unwrap());
            let place = ItemPlace::PlayerSlot {
                player: 12,
                slot: match write {
                    PlayerWrite::Unrelated => 20,
                    PlayerWrite::ShiftDestination { .. } => 8,
                },
            };
            assert!(
                !host.transaction_matches(None),
                "trade listener is inside the action"
            );
            host.transaction(|host, tx| {
                assert_eq!(place.insert(host, tx, &dirt, 2), Ok(2));
                if matches!(write, PlayerWrite::ShiftDestination { restore: true }) {
                    assert_eq!(place.extract(host, tx, &dirt, 2), Ok(2));
                }
                Ok::<_, ()>(())
            })
            .unwrap();
            STAGED_WRITES.set(true);
        }
        if s.raise_second_price && call == 2 {
            event.offer.special_price_diff = 1;
        }
        if s.answer {
            event.offer.uses = 6;
            event.offer.max_uses = 19;
            event.offer.demand = -7;
            event.offer.special_price_diff = -4;
            event.offer.price_multiplier = 0.25;
            event.offer.xp = 11;
        }
        if s.invalid {
            event.offer.price_multiplier = f32::NAN;
        }
        if s.invalid_stack {
            event.offer.result.set_damage(u32::MAX);
        }
        if s.edit_other_merchant_and_board {
            let other = host.merchant(host.entity(41).unwrap()).unwrap();
            let mut offer = other.offers(host).unwrap().remove(0);
            offer.uses = 6;
            assert!(other.set_offer(host, 0, offer).unwrap());
            host.add_objective(
                "temporary",
                "dummy",
                mod_api::Text::literal("Temporary"),
                mod_api::ObjectiveRenderType::Integer,
            )
            .unwrap();
        }
        if let Some(write) = s.effect_write {
            let other = host.living(host.entity(41).unwrap()).unwrap();
            let key = |id| mod_api::__private::effect_key(id).unwrap();
            let add = |host: &mut dyn ServerHost, id| {
                let effect = mod_api::ActiveEffect::new(key(id), 200, 0, false, true, true);
                assert!(other.add_effect(host, &effect));
                assert!(other.potion_affected(host, effect.id));
            };
            match write {
                EffectWrite::ReplaceSlowness => {
                    assert!(other.remove_effect(host, key("minecraft:slowness")));
                    add(host, "minecraft:speed");
                    add(host, "minecraft:absorption");
                }
                EffectWrite::ReapplySpeed => {
                    assert!(other.remove_effect(host, key("minecraft:speed")));
                    assert!(!other.potion_affected(host, key("minecraft:speed")));
                    add(host, "minecraft:speed");
                }
                EffectWrite::Clear => {
                    host.transaction(|host, _| {
                        assert!(other.remove_all_effects(host));
                        Ok::<_, ()>(())
                    })
                    .unwrap();
                    assert!(!other.potion_affected(host, key("minecraft:speed")));
                    assert!(!other.potion_affected(host, key("minecraft:slowness")));
                }
                EffectWrite::ReplaceTransient => {
                    add(host, "minecraft:speed");
                    let explicit = mod_api::AttributeModifier::new(
                        mod_api::Identifier::parse("test:explicit").unwrap(),
                        0.0625,
                        mod_api::AttributeOperation::AddValue,
                    );
                    assert!(other.add_attribute_modifier(
                        host,
                        SPEED,
                        explicit,
                        mod_api::ModifierLifetime::Transient
                    ));
                }
            }
            STAGED_WRITES.set(true);
        }
        if s.panic_on == Some(call) {
            panic!("trade listener failed");
        }
    }
}
pub(crate) fn with_mod(test: impl FnOnce()) {
    let mods: Mods = vec![ModEntry::new("trade-observer", Box::new(Mod))].into();
    mod_platform::testing::with_mods(mods, test);
}
pub(crate) fn with_both(test: impl FnOnce()) {
    let mods: Mods = vec![ModEntry::new("trade-and-stack", Box::new(Both))].into();
    mod_platform::testing::with_mods(mods, test);
}

pub(crate) fn open(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    payment: u32,
) -> ContainerClick {
    use {
        sunrise_gameplay::game::server_side::native_merchant_refs::tests::spawn,
        sunrise_gameplay_common::game::server_side::native_merchant_refs::tests::offer,
        sunrise_gameplay_common::game::trading_common::MerchantState,
    };
    spawn(game, 40, "minecraft:villager");
    game.mobs[0].extra.profession = sunrise_sim::mobs::villager::ProfessionState::from_index(5);
    let mut offer = offer("minecraft:wheat", "minecraft:emerald");
    offer.base_cost_a.stack.count = 1;
    offer.uses = 0;
    offer.max_uses = 10;
    offer.demand = 0;
    offer.special_price_diff = 0;
    offer.price_multiplier = 0.0;
    game.mobs[0].extra.trade = Some(Box::new(Trader {
        offers: vec![offer.clone()],
        ..Trader::default()
    }));
    game.mobs[0].extra.flag2 = true;
    let mut session = Session::new(vec![offer], true);
    // Avoid a level-up changing the one-offer test while still awarding real trade XP.
    session.level = 5;
    session.profession = "minecraft:farmer".to_owned();
    game.lan_open_merchant(
        12,
        MerchantState {
            session: Box::new(session),
            scroll: 0,
            dragging: false,
            focused: None,
            mob: Some(40),
            name: None,
        },
    );
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.inventory[9] = Some(ItemStack::new("minecraft:wheat", payment));
    });
    game.lan_select_trade(12, 0);
    let window = game.server.lan.as_ref().unwrap().menus[&12]
        .merchant
        .as_ref()
        .unwrap()
        .window;
    ContainerClick {
        container: window,
        state: 0,
        slot: 2,
        button: 0,
        input: 0,
        changed: Vec::new(),
        carried: None,
    }
}
pub(crate) const WORK_TICK: i64 = 3_000;

/// A loaded composter with an adult farmer close enough to start work on its next AI tick.
pub(crate) fn prepare_work(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    save: &std::path::Path,
) {
    use sunrise_core::{geometry::Vec3, random::LegacyRandom};
    use sunrise_sim::mobs::villager_brain;

    game.server.world_root = save.to_path_buf();
    game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
        .unwrap();
    game.options.dedicated = Some(Vec::new());
    game.server.streaming.insert_remote(
        &mut game.common.world,
        sunrise_level::chunk::Chunk::empty(0, 0),
    );
    assert!(game.entity_ticking_area()(0, 0));
    let composter = game
        .pack
        .assets
        .registry
        .default_state("minecraft:composter")
        .unwrap();
    let site = [1, 80, 1];
    game.world
        .set(site[0], site[1], site[2], composter)
        .unwrap();
    game.world.game_time = WORK_TICK;
    game.world.day_time = WORK_TICK;
    *game.world.random = LegacyRandom::new(4096);
    let mob = &mut game.mobs[0];
    mob.position = Vec3::new(1.5, 80.0, 0.5);
    mob.previous = mob.position;
    mob.no_gravity = true;
    mob.extra.home = Some(site);
    villager_brain::init(mob, WORK_TICK, WORK_TICK, 40);
}

pub(crate) fn tick_work(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    let worked_at = |game: &sunrise_gameplay::game::ServerGame<'_>| {
        sunrise_sim::mobs::villager::ext(game.mob_object(40).unwrap())
            .unwrap()
            .brain
            .as_ref()
            .unwrap()
            .mem
            .last_worked_at_poi
            .value()
    };
    assert_eq!(worked_at(game), None);
    game.tick_creature_entities().unwrap();
    assert_eq!(worked_at(game), Some(WORK_TICK));
}

pub(crate) fn work_screen<'a>(
    game: &'a sunrise_gameplay::game::ServerGame<'_>,
) -> &'a sunrise_gameplay_common::game::lan_menu_data::GuestMerchant {
    game.server.lan.as_ref().unwrap().menus[&12]
        .merchant
        .as_ref()
        .unwrap()
}

pub(crate) fn work_screen_mut<'a>(
    game: &'a mut sunrise_gameplay::game::ServerGame<'_>,
) -> &'a mut sunrise_gameplay_common::game::lan_menu_data::GuestMerchant {
    game.server
        .lan
        .as_mut()
        .unwrap()
        .menus
        .get_mut(&12)
        .unwrap()
        .merchant
        .as_mut()
        .unwrap()
}

pub(crate) fn reloaded_trader(game: &sunrise_gameplay::game::ServerGame<'_>) -> Box<Trader> {
    let saved = sunrise_sim::entity_nbt::mob_to_nbt(
        game.mob_object(40).unwrap(),
        [1, 2, 3, 4],
        None,
        Some(game.pack.assets.registry),
        |_| None,
        game.items(),
    );
    sunrise_sim::entity_nbt::mob_from_nbt(
        &saved,
        game.res.mob_types(),
        999,
        Some(game.pack.assets.registry),
    )
    .unwrap()
    .extra
    .trade
    .unwrap()
}

/// A new day at the workstation sends catch-up and restock in order to an already open screen.
#[test]
#[ignore = "daily"]
fn a_restock_while_a_guest_trades_reaches_the_screen_the_save_and_the_mob() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::lan_trade_decisions_tests::a_restock_while_a_guest_trades_reaches_the_screen_the_save_and_the_mob",
        || {
            with_guests(|game| {
                let save = tempfile::tempdir().unwrap();
                open(game, 2);
                prepare_work(game, save.path());
                work_screen_mut(game).merchant.session.offers[0].uses = 10;
                let trader = game.mobs[0].extra.trade.as_mut().unwrap();
                trader.offers[0].uses = 10;
                trader.last_restock = -13_000;
                trader.restocks_today = 2;

                let rx = record_packets(game);
                tick_work(game);
                let sent = packets(&rx);
                let uses: Vec<_> = sent
                    .iter()
                    .filter_map(|p| match p {
                        Clientbound::MerchantOffers(o) => Some(o.offers[0].uses),
                        _ => None,
                    })
                    .collect();
                assert_eq!(uses, [10, 0]);
                assert!(
                    sent.iter()
                        .any(|p| matches!(p, Clientbound::ContainerSetContent(_)))
                );
                let offer = &work_screen(game).merchant.session.offers[0];
                assert_eq!((offer.uses, offer.demand), (0, 10));
                let loaded = reloaded_trader(game);
                assert_eq!(
                    (
                        loaded.offers[0].uses,
                        loaded.offers[0].demand,
                        loaded.restocks_today
                    ),
                    (0, 10, 1)
                );

                game.guest_close_all(12);
                let trader = game.mobs[0].extra.trade.as_ref().unwrap();
                assert_eq!(
                    (
                        trader.offers[0].uses,
                        trader.offers[0].demand,
                        trader.restocks_today
                    ),
                    (0, 10, 1)
                );
            })
        },
    );
}

/// A guest trading with the villager loses Hero of the Village in its named Pre or Post.
pub(crate) struct RemoveHero {
    pub(crate) before_work: bool,
}

impl RemoveHero {
    fn remove(host: HostSide<'_>, entity: u32) {
        assert_eq!(entity, 40);
        let HostSide::Server(host) = host else {
            panic!("server fixture");
        };
        let guest = host.living(host.entity(12).unwrap()).unwrap();
        let hero = host.effect_key("minecraft:hero_of_the_village").unwrap();
        assert!(guest.remove_effect(host, hero));
    }
}

#[mod_api::__subscriptions]
impl Hooks for RemoveHero {
    fn entity_tick_pre(
        &self,
        host: HostSide<'_>,
        event: &mut mod_api::EntityTickPreEvent<'_>,
    ) -> mod_api::Flow {
        if self.before_work {
            Self::remove(host, event.entity);
        }
        mod_api::Flow::Continue
    }

    fn entity_tick_post(&self, host: HostSide<'_>, event: &mut mod_api::EntityTickPostEvent<'_>) {
        if !self.before_work {
            Self::remove(host, event.entity);
        }
    }
}

/// The result and the yes sound use the Hero discount seen during the work step, not after Post.
#[test]
#[ignore = "daily"]
fn a_restock_prices_the_screen_for_the_player_as_the_work_step_saw_them() {
    restock_after_hero_change(
        "cut_layout::game::server_side::lan_trade_decisions_tests::a_restock_prices_the_screen_for_the_player_as_the_work_step_saw_them",
        false,
    );
}

/// If Pre takes the discount away, four wheat no longer buys an emerald.
#[test]
#[ignore = "daily"]
fn a_restock_uses_price_changes_made_in_pre() {
    restock_after_hero_change(
        "cut_layout::game::server_side::lan_trade_decisions_tests::a_restock_uses_price_changes_made_in_pre",
        true,
    );
}

fn restock_after_hero_change(test: &str, before_work: bool) {
    use sunrise_level::effects::EffectInstance;
    sunrise_gameplay_common::game::in_own_process(test, || {
        let mods: Mods = vec![ModEntry::new(
            "remove-hero",
            Box::new(RemoveHero { before_work }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                let save = tempfile::tempdir().unwrap();
                open(game, 1);
                prepare_work(game, save.path());
                game.as_guest(12, |g| {
                    g.potions
                        .effects
                        .add(EffectInstance::new("hero_of_the_village", 1000, 0));
                });
                let screen = work_screen_mut(game);
                screen.slots = [Some(ItemStack::new("minecraft:wheat", 4)), None, None];
                let offer = &mut screen.merchant.session.offers[0];
                (offer.base_cost_a.stack.count, offer.uses) = (5, 10);
                let trader = game.mobs[0].extra.trade.as_mut().unwrap();
                (
                    trader.offers[0].base_cost_a.stack.count,
                    trader.offers[0].uses,
                ) = (5, 10);
                // The voice from opening the screen has finished its cooldown by work time.
                game.mobs[0].ambient_timer = 0;
                game.start_audio_capture();

                tick_work(game);

                game.as_guest(12, |g| {
                    assert_eq!(g.potions.effects.amplifier("hero_of_the_village"), None);
                });
                let screen = work_screen(game);
                assert_eq!(
                    screen.slots[2],
                    (!before_work).then(|| ItemStack::new("minecraft:emerald", 1))
                );
                assert_eq!(screen.merchant.session.hero, (!before_work).then_some(0));
                let feedback: Vec<_> = game
                    .server
                    .pending_sounds
                    .iter()
                    .map(|sound| sound.event.as_str())
                    .filter(|&event| {
                        matches!(
                            event,
                            "minecraft:entity.villager.yes" | "minecraft:entity.villager.no"
                        )
                    })
                    .collect();
                assert_eq!(
                    feedback,
                    [if before_work {
                        "minecraft:entity.villager.no"
                    } else {
                        "minecraft:entity.villager.yes"
                    }]
                );
            })
        });
    });
}

/// Edits the restocked first offer in the villager's own named `EntityTick.Post`.
pub(crate) struct EditInPost;
#[mod_api::__subscriptions]
impl Hooks for EditInPost {
    fn entity_tick_post(&self, host: HostSide<'_>, event: &mut mod_api::EntityTickPostEvent<'_>) {
        assert_eq!(event.entity, 40);
        let HostSide::Server(host) = host else {
            panic!("server fixture");
        };
        let villager = host.merchant(host.entity(40).unwrap()).unwrap();
        let mut offer = villager.offers(host).unwrap().remove(0);
        let emerald = mod_api::__private::item_key("minecraft:emerald").unwrap();
        assert_eq!(
            (offer.result.clone(), offer.uses, offer.demand),
            (mod_api::ItemStack::new(emerald, 1).unwrap(), 0, 10)
        );
        let diamond = mod_api::__private::item_key("minecraft:diamond").unwrap();
        offer.result = mod_api::ItemStack::new(diamond, 1).unwrap();
        (offer.uses, offer.demand) = (3, 17);
        assert!(villager.set_offer(host, 0, offer).unwrap());
    }
}

#[test]
#[ignore = "daily"]
fn an_offer_edit_after_the_work_step_survives_the_restock() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::lan_trade_decisions_tests::an_offer_edit_after_the_work_step_survives_the_restock",
        || {
            let mods: Mods = vec![ModEntry::new("edit-in-post", Box::new(EditInPost))].into();
            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    let save = tempfile::tempdir().unwrap();
                    open(game, 1);
                    prepare_work(game, save.path());
                    work_screen_mut(game).merchant.session.offers[0].uses = 10;
                    game.mobs[0].extra.trade.as_mut().unwrap().offers[0].uses = 10;

                    tick_work(game);

                    let got = |o: &sunrise_sim::merchant::MerchantOffer| {
                        (o.result.clone(), o.uses, o.demand)
                    };
                    let want = (ItemStack::new("minecraft:diamond", 1), 3, 17);
                    assert_eq!(got(&work_screen(game).merchant.session.offers[0]), want);
                    assert_eq!(got(&reloaded_trader(game).offers[0]), want);
                })
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_silent_trade_edit_keeps_the_mob_clock_and_rng_when_the_threshold_wraps() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::lan_trade_decisions_tests::a_silent_trade_edit_keeps_the_mob_clock_and_rng_when_the_threshold_wraps",
        || {
            with_guests(|game| {
                open(game, 1);
                let mob = &mut game.mobs[0];
                Arc::make_mut(&mut mob.kind).ambient_interval = -i32::MAX;
                mob.ambient_timer = 17;
                let random = <(u64, Option<f64>)>::from(&mob.random);
                let rx = record_packets(game);

                assert!(game.lan_trade_edit(12, |session| {
                    session.offers[0].result = ItemStack::new("minecraft:diamond", 1);
                }));

                let result = work_screen(game).slots[2].as_ref().unwrap();
                assert_eq!(result.item.as_ref(), "minecraft:diamond");
                assert_eq!(game.mobs[0].ambient_timer, 17);
                assert_eq!(<(u64, Option<f64>)>::from(&game.mobs[0].random), random);
                assert!(!packets(&rx).iter().any(|packet| matches!(
                    packet,
                    Clientbound::Sound(_) | Clientbound::SoundEntity(_)
                )));
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn an_unused_offer_stays_unchanged_when_the_villager_starts_work() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::lan_trade_decisions_tests::an_unused_offer_stays_unchanged_when_the_villager_starts_work",
        || {
            with_guests(|game| {
                let save = tempfile::tempdir().unwrap();
                open(game, 1);
                prepare_work(game, save.path());
                let offer = work_screen(game).merchant.session.offers[0].clone();
                let result = work_screen(game).slots[2].clone();
                let trader = game.mobs[0].extra.trade.as_mut().unwrap();
                // Signed int from RestocksToday: no Java subtraction on this same-day check.
                trader.restocks_today = i32::MIN;
                let rx = record_packets(game);

                tick_work(game);

                assert!(
                    !packets(&rx)
                        .iter()
                        .any(|p| matches!(p, Clientbound::MerchantOffers(_)))
                );
                let screen = work_screen(game);
                assert_eq!(screen.merchant.session.offers[0], offer);
                assert_eq!(screen.slots[2], result);
                let trader = game.mobs[0].extra.trade.as_ref().unwrap();
                assert_eq!((trader.last_restock, trader.restocks_today), (0, i32::MIN));
            })
        },
    );
}

pub(crate) struct FishBreath(pub(crate) Arc<std::sync::atomic::AtomicBool>);
#[mod_api::__subscriptions]
impl Hooks for FishBreath {
    fn living_breathe(&self, host: &mut dyn ServerHost, event: &mut mod_api::LivingBreatheEvent) {
        if event.entity.entity().id() == 41 {
            let merchant = host.merchant(host.entity(40).unwrap()).unwrap();
            assert_eq!(merchant.offers(host).unwrap()[0].uses, 0);
            self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

#[test]
#[ignore = "daily"]
fn an_earlier_restock_does_not_skip_a_paused_mobs_first_action() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::mobs::Mob;

    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::lan_trade_decisions_tests::an_earlier_restock_does_not_skip_a_paused_mobs_first_action",
        || {
            crate::entry::install_client_hooks();
            let breathed = Arc::new(AtomicBool::new(false));
            let mods: Mods = vec![ModEntry::new(
                "pufferfish-breath",
                Box::new(FishBreath(Arc::clone(&breathed))),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    let save = tempfile::tempdir().unwrap();
                    open(game, 1);
                    prepare_work(game, save.path());
                    work_screen_mut(game).merchant.session.offers[0].uses = 10;
                    game.mobs[0].extra.trade.as_mut().unwrap().offers[0].uses = 10;
                    let make = |id, kind, x| {
                        let mut mob = Mob::new(
                            id,
                            game.res.mob_types().get(kind).unwrap().clone(),
                            Vec3::new(x, 80.0, 1.5),
                            0.0,
                        );
                        mob.no_ai = true;
                        mob.no_gravity = true;
                        mob
                    };
                    let mut fish = make(41, "minecraft:pufferfish", 7.5);
                    fish.extra.size = 2;
                    // Outside normal body collision so a push cannot precede the sting,
                    // but inside Pufferfish.aiStep's 0.3-block inflated hitbox.
                    let pig = make(42, "minecraft:pig", 8.45);
                    assert!(!fish.bounds().intersects(pig.bounds()));
                    assert!(
                        fish.bounds()
                            .inflate(Vec3::new(0.3, 0.3, 0.3))
                            .intersects(pig.bounds())
                    );
                    let health = pig.health;
                    game.mobs.push(fish);
                    game.mobs.push(pig);

                    tick_work(game);

                    assert!(breathed.load(Ordering::Relaxed));
                    let pig = game.mob_object(42).unwrap();
                    assert_eq!(pig.health, health - 3.0);
                    assert!(pig.has_effect("minecraft:poison"));
                })
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn refusal_restores_crafted_score_and_another_merchants_offers() {
    use mod_api::{ObjectiveRenderType, prelude::ScoreboardServerHost};
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            use {
                sunrise_gameplay::game::server_side::native_merchant_refs::tests::spawn,
                sunrise_gameplay_common::game::server_side::native_merchant_refs::tests::offer,
            };
            spawn(game, 41, "minecraft:villager");
            game.mobs[1].extra.trade = Some(Box::new(Trader {
                offers: vec![offer("minecraft:stick", "minecraft:emerald")],
                ..Trader::default()
            }));
            ScoreboardServerHost::add_objective(
                game,
                "crafted",
                "minecraft.crafted:minecraft.emerald",
                mod_api::Text::literal("Crafted"),
                ObjectiveRenderType::Integer,
            )
            .unwrap();
            let player = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(12)
                .unwrap()
                .name
                .clone();
            script(Script {
                edit_other_merchant_and_board: true,
                panic_on: Some(1),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            unchanged(game, 2);
            assert_eq!(CALLS.get(), 1);
            assert!(game.score.board.objective("temporary").is_none());
            assert!(game.score.board.score(&player, "crafted").is_none());
            let other = game.mobs[1].extra.trade.as_ref().unwrap();
            assert_eq!(other.offers[0].uses, 1);
        })
    });
}

#[test]
#[ignore = "daily"]
fn an_invalid_offer_restores_another_merchants_effects_and_modifiers() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            sunrise_gameplay::game::server_side::native_merchant_refs::tests::spawn(
                game,
                41,
                "minecraft:villager",
            );
            let mob = &mut game.mobs[1];
            mob.add_effect("minecraft:slowness", 300, 0);
            mob.effects_dirty = false;
            let speed = mob.movement_speed().to_bits();
            script(Script {
                invalid: true,
                effect_write: Some(EffectWrite::ReplaceSlowness),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 1);
            assert!(
                STAGED_WRITES.get(),
                "effect writes must succeed before refusing the trade"
            );
            unchanged(game, 2);
            let other = &game.mobs[1];
            assert_eq!(other.extra.effects.len(), 1);
            assert_eq!(other.extra.effects[0].id, "minecraft:slowness");
            assert_eq!(other.extra.effects[0].duration, 300);
            assert_eq!(other.movement_speed().to_bits(), speed);
            assert_eq!(other.absorption, 0.0);
            assert!(!other.effects_dirty);
            assert!(game.transactions.is_empty());
        });
    });
}

/// Observe the order from gameplay's store and independently through the mod API's live map.
pub(crate) fn speed_modifier_ids(
    game: &sunrise_gameplay::game::ServerGame<'_>,
) -> [Vec<String>; 2] {
    let attrs = game.mobs[1].extra.attributes.as_deref().unwrap();
    let legacy = attrs
        .modifiers
        .iter()
        .chain(&attrs.transient)
        .filter(|row| row.attr == Attr::MovementSpeed)
        .map(|row| row.id.to_string())
        .collect();
    let other = game.living(game.entity(41).unwrap()).unwrap();
    let mut live = Vec::new();
    assert_eq!(
        other.visit_attribute_modifiers(game, SPEED, &mut |row| {
            live.push(row.id.to_owned());
            std::ops::ControlFlow::Continue(())
        }),
        Ok(std::ops::ControlFlow::Continue(()))
    );
    [legacy, live]
}

#[test]
#[ignore = "daily"]
fn a_refused_trade_restores_modifier_order_after_reapplying_the_same_effect() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            sunrise_gameplay::game::server_side::native_merchant_refs::tests::spawn(
                game,
                41,
                "minecraft:villager",
            );
            let mob = &mut game.mobs[1];
            mob.add_effect("minecraft:speed", 300, 0);
            finalize::add_modifier(
                mob,
                Attr::MovementSpeed,
                "test:survivor",
                0.25,
                Op::AddMultipliedTotal,
            );
            let other = game.living(game.entity(41).unwrap()).unwrap();
            assert!(game.living_set_equipment(
                other,
                mod_api::EquipmentSlot::Head,
                Some(ItemStack::new("minecraft:diamond_helmet", 1)),
            ));
            assert_eq!(other.attribute_value(game, "minecraft:armor"), Some(3.0));
            let expected = ["minecraft:effect.speed", "test:survivor"];
            for ids in speed_modifier_ids(game) {
                assert_eq!(ids, expected);
            }
            script(Script {
                invalid: true,
                effect_write: Some(EffectWrite::ReapplySpeed),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert!(STAGED_WRITES.get(), "Speed was removed and re-added");
            unchanged(game, 2);
            for ids in speed_modifier_ids(game) {
                assert_eq!(ids, expected);
            }
            assert_eq!(other.attribute_value(game, "minecraft:armor"), Some(3.0));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_refused_trade_restores_modifier_order_after_a_nested_effect_clear() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            sunrise_gameplay::game::server_side::native_merchant_refs::tests::spawn(
                game,
                41,
                "minecraft:villager",
            );
            let mob = &mut game.mobs[1];
            mob.add_effect("minecraft:speed", 300, 0);
            mob.add_effect("minecraft:slowness", 400, 0);
            finalize::add_modifier(
                mob,
                Attr::MovementSpeed,
                "test:survivor",
                0.25,
                Op::AddMultipliedTotal,
            );
            let expected = [
                "minecraft:effect.speed",
                "minecraft:effect.slowness",
                "test:survivor",
            ];
            for ids in speed_modifier_ids(game) {
                assert_eq!(ids, expected);
            }
            script(Script {
                invalid: true,
                effect_write: Some(EffectWrite::Clear),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert!(
                STAGED_WRITES.get(),
                "the nested clear committed inside the refused trade"
            );
            unchanged(game, 2);
            for ids in speed_modifier_ids(game) {
                assert_eq!(ids, expected);
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_refused_trade_restores_a_replaced_transient_in_place() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            sunrise_gameplay::game::server_side::native_merchant_refs::tests::spawn(
                game,
                41,
                "minecraft:villager",
            );
            let mob = &mut game.mobs[1];
            finalize::add_transient_modifier(
                mob,
                Attr::MovementSpeed,
                "minecraft:effect.speed",
                0.125,
                Op::AddValue,
            );
            finalize::add_transient_modifier(
                mob,
                Attr::MovementSpeed,
                "test:survivor",
                0.25,
                Op::AddValue,
            );
            for ids in speed_modifier_ids(game) {
                assert_eq!(ids, ["minecraft:effect.speed", "test:survivor"]);
            }
            script(Script {
                invalid: true,
                effect_write: Some(EffectWrite::ReplaceTransient),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert!(
                STAGED_WRITES.get(),
                "the colliding effect and the explicit write must succeed"
            );
            unchanged(game, 2);
            for ids in speed_modifier_ids(game) {
                assert_eq!(
                    ids,
                    ["minecraft:effect.speed", "test:survivor", "test:explicit"]
                );
            }
            assert_eq!(
                finalize::transient_modifier_amount(
                    &game.mobs[1],
                    Attr::MovementSpeed,
                    "minecraft:effect.speed"
                ),
                Some(0.125)
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn exploration_trade_maps_share_nested_reservations() {
    use sunrise_gameplay_common::game::maps::MapData;
    with_guests(|game| {
        let start = game.peek_map_id();
        let make = || MapData::fresh(0.0, 0.0, 0, "minecraft:overworld");
        for commit in [false, true] {
            let trade_id = game.storage_scope(|game, _| {
                assert_eq!(game.new_map_id(make()), start);
                let trade_id = game.storage_scope(|game, _| {
                    let trade_id = game.peek_map_id();
                    assert_eq!(trade_id, start + 1);
                    game.register_exploration_maps(vec![(
                        trade_id,
                        [640, 64, -640],
                        2,
                        String::new(),
                    )]);
                    (trade_id, true)
                });
                assert_eq!(game.new_map_id(make()), start + 2);
                assert!(!game.maps.data.contains_key(&trade_id));
                (trade_id, commit)
            });
            assert_eq!(game.peek_map_id(), if commit { start + 3 } else { start });
            if commit {
                assert!(game.maps.data[&trade_id].unlimited);
                assert!(!game.maps.data[&(start + 2)].unlimited);
            } else {
                assert!(!game.maps.data.contains_key(&trade_id));
            }
        }
    });
}

pub(crate) fn record_packets(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
) -> mpsc::Receiver<Out> {
    let (tx, rx) = mpsc::channel();
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .update_guest(12, |guest| {
            guest.sender = PlayerSender::new(tx, Arc::default());
        })
        .unwrap();
    rx
}

pub(crate) fn packets(rx: &mpsc::Receiver<Out>) -> Vec<Clientbound> {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            _ => Vec::new(),
        })
        .map(|body| {
            let mut r = sunrise_protocol::buf::Reader::new(&body);
            Clientbound::decode(r.varint().unwrap(), r.rest()).unwrap()
        })
        .collect()
}

pub(crate) fn corrected(rx: &mpsc::Receiver<Out>, window: i32, payment: u32) {
    let received = packets(rx);
    let [
        Clientbound::MerchantOffers(offers),
        Clientbound::ContainerSetContent(content),
    ] = received.as_slice()
    else {
        panic!("expected just the authoritative offers and contents");
    };
    assert_eq!((offers.container, content.container), (window, window));
    assert_eq!(offers.offers[0].uses, 0);
    assert_eq!(content.items[0].as_ref().unwrap().count as u32, payment);
    assert_eq!(content.items[2].as_ref(), Some(&offers.offers[0].result));
    assert!(content.carried.is_none());
}

pub(crate) fn count(stack: Option<&ItemStack>) -> u32 {
    stack.map_or(0, |s| s.count)
}
pub(crate) fn result(game: &sunrise_gameplay::game::ServerGame<'_>) -> u32 {
    let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
    guest
        .inventory
        .iter()
        .flatten()
        .filter(|s| s.item.as_ref() == "minecraft:emerald")
        .map(|s| s.count)
        .sum::<u32>()
        + count(
            guest
                .carried
                .as_ref()
                .filter(|s| s.item.as_ref() == "minecraft:emerald"),
        )
}
pub(crate) fn dirt(game: &sunrise_gameplay::game::ServerGame<'_>) -> u32 {
    count(game.server.lan.as_ref().unwrap().hub.guests[0].inventory[20].as_ref())
}
pub(crate) fn stat(game: &sunrise_gameplay::game::ServerGame<'_>) -> i32 {
    game.server.lan.as_ref().unwrap().play[&12]
        .stats
        .values
        .get("minecraft:custom")
        .and_then(|v| v.get("minecraft:traded_with_villager"))
        .copied()
        .unwrap_or(0)
}
pub(crate) fn unchanged(game: &mut sunrise_gameplay::game::ServerGame<'_>, payment: u32) {
    assert_eq!(result(game), 0);
    assert_eq!(dirt(game), 0);
    assert_eq!(stat(game), 0);
    let session = game.lan_trade_session(12).unwrap();
    assert_eq!(session.offers[0].uses, 0);
    assert_eq!(session.xp, 0);
    assert!(game.orbs.is_empty());
    let host = game.server.lan.as_ref().unwrap();
    let slots = &host.menus[&12].merchant.as_ref().unwrap().slots;
    assert_eq!(count(slots[0].as_ref()), payment);
    assert_eq!(slots[2], Some(ItemStack::new("minecraft:emerald", 1)));
}

#[test]
#[ignore = "daily"]
fn accepted_trade_keeps_the_collect_answer() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            let rx = record_packets(game);
            script(Script {
                answer: true,
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 1);
            assert_eq!(
                sunrise_gameplay::game::server_side::decided_action::DECIDED.get(),
                1
            );
            assert_eq!(IN_ACTION.with(|a| a.borrow().clone()), [true]);
            assert_eq!(result(game), 1);
            assert_eq!(dirt(game), 0);
            assert_eq!(stat(game), 1);
            let session = game.lan_trade_session(12).unwrap();
            assert_eq!(session.xp, 7);
            let offer = &session.offers[0];
            assert_eq!(
                (
                    offer.uses,
                    offer.max_uses,
                    offer.demand,
                    offer.special_price_diff,
                    offer.xp
                ),
                (6, 19, -7, -4, 11)
            );
            assert_eq!(offer.price_multiplier, 0.25);
            let received = packets(&rx);
            let [
                Clientbound::MerchantOffers(offers),
                Clientbound::ContainerSetContent(content),
            ] = received.as_slice()
            else {
                panic!("an edited offer must refresh before the new contents");
            };
            assert_eq!(
                (offers.container, content.container),
                (click.container, click.container)
            );
            assert_eq!(offers.offers[0].uses, 6);
            assert_eq!(content.carried.as_ref().unwrap().count, 1);
            assert!(!game.orbs.is_empty());
            assert!(game.transactions.is_empty());
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_committed_take_publishes_success_packets() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            let rx = record_packets(game);
            script(Script {
                answer: true,
                ..Script::default()
            });
            game.storage_scope(|game, _| {
                assert!(game.lan_merchant_click(12, &click));
                assert!(packets(&rx).is_empty());
                ((), true)
            });
            assert_eq!((result(game), stat(game)), (1, 1));
            assert_eq!(game.lan_trade_session(12).unwrap().offers[0].uses, 6);
            let received = packets(&rx);
            let [
                Clientbound::MerchantOffers(offers),
                Clientbound::ContainerSetContent(content),
            ] = received.as_slice()
            else {
                panic!("expected committed offers and content packets");
            };
            assert_eq!(
                (offers.container, content.container),
                (click.container, click.container)
            );
            assert_eq!(offers.offers[0].uses, 6);
            assert_eq!(content.carried.as_ref().unwrap().count, 1);
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_panicking_trade_listener_undoes_the_take_and_the_next_trade_works() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            script(Script {
                panic_on: Some(1),
                player_write: Some(PlayerWrite::Unrelated),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 1);
            unchanged(game, 2);
            assert!(game.transactions.is_empty());
            script(Script::default());
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 0, "the panicking listener stays disabled");
            assert_eq!((result(game), stat(game)), (1, 1));
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_second_trade_panic_undoes_the_entire_shift_click() {
    with_mod(|| {
        with_guests(|game| {
            let mut click = open(game, 3);
            click.input = 1;
            script(Script {
                panic_on: Some(2),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 2);
            assert_eq!(
                sunrise_gameplay::game::server_side::decided_action::DECIDED.get(),
                2
            );
            unchanged(game, 3);
            assert!(game.transactions.is_empty());
            script(Script::default());
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 0, "the panicking listener stays disabled");
            assert_eq!((result(game), stat(game)), (3, 3));
        })
    });
}

#[test]
#[ignore = "daily"]
fn an_outer_abort_undoes_an_accepted_trade() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            script(Script::default());
            let rx = record_packets(game);
            game.storage_scope(|game, _| {
                assert!(game.lan_merchant_click(12, &click));
                assert_eq!((result(game), dirt(game), stat(game)), (1, 0, 1));
                assert!(game.orbs.is_empty(), "orb creation waits for root commit");
                assert!(packets(&rx).is_empty());
                ((), false)
            });
            corrected(&rx, click.container, 2);
            unchanged(game, 2);
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_non_finite_collect_answer_refuses_the_whole_take() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            script(Script {
                invalid: true,
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 1);
            unchanged(game, 2);
        })
    });
}

#[test]
#[ignore = "daily"]
fn no_subscriber_uses_the_ordinary_trade_without_a_decision() {
    mod_platform::testing::with_mods(Mods::default(), || {
        with_guests(|game| {
            let mut click = open(game, 3);
            click.input = 1;
            script(Script::default());
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(
                sunrise_gameplay::game::server_side::decided_action::DECIDED.get(),
                0
            );
            assert_eq!(CALLS.get(), 0);
            assert_eq!((result(game), stat(game)), (3, 3));
            assert_eq!(game.lan_trade_session(12).unwrap().offers[0].uses, 3);
            assert_eq!(game.lan_trade_session(12).unwrap().xp, 21);
            assert!(game.transactions.is_empty());
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_result_pickup_heard_as_trade_and_stacked_event_is_one_accepted_action() {
    with_both(|| {
        with_guests(|game| {
            let click = open(game, 2);
            script(Script {
                answer: true,
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!((CALLS.get(), STACKED.get()), (1, 1));
            assert_eq!((result(game), dirt(game), stat(game)), (1, 0, 1));
            assert!(game.transactions.is_empty());
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_trade_refused_inside_the_stacked_pickup_undoes_the_whole_take() {
    for (trade_subscribed, entry) in [
        (true, ModEntry::new("trade-and-stack", Box::new(Both))),
        (false, ModEntry::new("stacked-only", Box::new(Stacked))),
    ] {
        mod_platform::testing::with_mods(vec![entry].into(), || {
            with_guests(|game| {
                let click = open(game, 2);
                let rx = record_packets(game);
                if trade_subscribed {
                    script(Script {
                        panic_on: Some(1),
                        ..Script::default()
                    });
                    assert!(game.lan_merchant_click(12, &click));
                    assert_eq!((CALLS.get(), STACKED.get()), (1, 1));
                    unchanged(game, 2);
                    corrected(&rx, click.container, 2);
                }
                // Even without a trade listener, an attempted result replacement or a failed
                // pickup leaves the take untouched and corrects the client's result slot. Run
                // the panicking case last because a panicking callback is retired for the session.
                for script_value in [
                    Script {
                        replace_stacked_result: true,
                        ..Script::default()
                    },
                    Script {
                        refuse_stacked: true,
                        ..Script::default()
                    },
                ] {
                    script(script_value);
                    assert!(game.lan_merchant_click(12, &click));
                    assert_eq!((CALLS.get(), STACKED.get()), (0, 1));
                    unchanged(game, 2);
                    corrected(&rx, click.container, 2);
                }
                assert!(game.transactions.is_empty());
            })
        });
    }
}

#[test]
#[ignore = "daily"]
fn an_invalid_result_stack_refuses_the_whole_take() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            script(Script {
                invalid_stack: true,
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 1);
            unchanged(game, 2);
            assert!(game.transactions.is_empty());
        })
    });
}

#[test]
#[ignore = "daily"]
fn shift_click_reads_each_use_and_applies_the_second_price_before_the_next_trade() {
    with_mod(|| {
        with_guests(|game| {
            let mut click = open(game, 3);
            click.input = 1;
            script(Script {
                raise_second_price: true,
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(USES.with(|uses| uses.borrow().clone()), [1, 2]);
            assert_eq!((result(game), stat(game)), (2, 2));
            let gm = game.server.lan.as_ref().unwrap().menus[&12]
                .merchant
                .as_ref()
                .unwrap();
            assert_eq!(gm.slots[0].as_ref().unwrap().count, 1);
            assert!(gm.slots[2].is_none());
        })
    });
}

#[test]
#[ignore = "daily"]
fn player_writes_refuse_the_frozen_take_without_recomputing() {
    with_mod(|| {
        with_guests(|game| {
            let mut click = open(game, 3);
            let rx = record_packets(game);
            for (input, player_write) in [
                (1, PlayerWrite::ShiftDestination { restore: false }),
                (1, PlayerWrite::ShiftDestination { restore: true }),
                (0, PlayerWrite::Unrelated),
            ] {
                click.input = input;
                script(Script {
                    player_write: Some(player_write),
                    ..Script::default()
                });
                assert!(game.lan_merchant_click(12, &click));
                assert_eq!(CALLS.get(), 1);
                assert!(
                    STAGED_WRITES.get(),
                    "the scripted slot writes must succeed before refusal"
                );
                unchanged(game, 3);
                assert!(
                    game.server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(12)
                        .unwrap()
                        .inventory[8]
                        .is_none()
                );
                corrected(&rx, click.container, 3);
            }
        })
    });
}

#[test]
#[ignore = "daily"]
fn observing_a_trade_keeps_the_plain_packet_sequence() {
    let trace = |mods: Mods| {
        let mut bodies = None;
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                let click = open(game, 2);
                let rx = record_packets(game);
                script(Script::default());
                assert!(game.lan_merchant_click(12, &click));
                let trace = packets(&rx);
                assert!(matches!(
                    trace.as_slice(),
                    [Clientbound::ContainerSetContent(_)]
                ));
                bodies = Some(trace.into_iter().map(|p| p.encode()).collect::<Vec<_>>());
            })
        });
        bodies.unwrap()
    };
    let observed: Mods = vec![ModEntry::new("trade-observer", Box::new(Mod))].into();
    assert_eq!(trace(Mods::default()), trace(observed));
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
