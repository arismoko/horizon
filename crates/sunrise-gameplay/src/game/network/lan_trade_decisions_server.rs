use super::{
    decided_action::{DecidedTarget, Epoch},
    lan_trading::{MerchantRun, opaque_villager},
    stacked_on_other::Pickup,
};
use crate::game::lan_trade_decisions::TakePlan;
use crate::game::lan_trade_decisions::TakeReads;
use std::cell::RefCell;
use sunrise_gameplay_common::game::lan_menu_data::{
    Effect, Journal, KeptMerchantGuest, KeptMerchantMob, KeptMerchantWindow,
};
use sunrise_gameplay_common::game::transactions::decision::{Identity, Reply, Side};
use sunrise_protocol::packets::play;
use sunrise_sim::{
    lan::conn::transaction_packets::Scope,
    menu::{Input, StackedOutcome},
    merchant::Event,
};

impl crate::game::ServerGame<'_> {
    /// Install only the writes calculated by the plan.
    pub(in crate::game::server_side) fn apply_merchant_take(
        &mut self,
        reads: &TakeReads,
        mut plan: TakePlan,
    ) {
        let conn = reads.conn;
        let mob = reads.identity.target.id();
        self.journal_guest_merchant(conn);
        // The crafted stats run in fire for a take and are already consumed in that case.
        let crafted = std::mem::take(&mut plan.menu.crafted_counts);
        if !crafted.is_empty() {
            self.as_guest(conn, |g| {
                for (item, count) in crafted {
                    g.award_stat("crafted", &item, count);
                }
            });
        }
        self.register_exploration_maps(plan.made);
        let (session, _) = self.merchant_apply(
            plan.menu.merchant.take().expect("planned merchant"),
            Some(mob),
            sunrise_sim::merchant::TradeFeedback::Current,
        );
        let host = self.server.lan.as_mut().expect("validated merchant host");
        let gi = host
            .hub
            .guests
            .connection_index(conn)
            .expect("validated guest");
        host.hub.guests.update(gi, |guest| {
            for (slot, item) in plan.player_slots {
                guest.inventory[slot] = item;
            }
            if let Some(offhand) = plan.offhand {
                guest.offhand = offhand;
            }
            guest.carried = plan.menu.carried.take();
        });
        let gm = host
            .menus
            .get_mut(&conn)
            .and_then(|p| p.merchant.as_mut())
            .expect("validated merchant window");
        gm.slots = std::array::from_fn(|slot| plan.menu.slot(slot).cloned());
        gm.drag = plan.menu.quick_craft_state();
        gm.merchant.session = session;
        self.merchant_drops(conn, std::mem::take(&mut plan.menu.dropped));
    }
    pub(in crate::game) fn commit_merchant_effect(&mut self, effect: Effect) {
        match effect {
            Effect::Orb(at, value) => self.add_orb(at, value),
            Effect::Drops(conn, stacks) => {
                if let Some(gi) = self
                    .server
                    .lan
                    .as_ref()
                    .and_then(|h| h.hub.guests.iter().position(|g| g.conn == conn))
                {
                    self.drop_guest_items(gi, stacks);
                }
            }
            Effect::Reward(uuid, advancement) => {
                self.native_as_player(uuid, |game| game.commit_merchant_reward(&advancement));
            }
        }
    }
    /// Keep trade-set sequence loans and draws at their first transactional write.
    pub(in crate::game::server_side) fn journal_trade_sequences(&mut self) {
        if let Some(level) = self.common.transactions.last_mut() {
            level
                .trades
                .sequences
                .get_or_insert_with(|| self.server.rand_sequences.clone());
        }
    }
    pub fn lan_merchant_click(&mut self, conn: u32, p: &play::ContainerClick) -> bool {
        let input = match (p.slot, p.input) {
            (2, 0) => Input::Pickup,
            (2, 1) => Input::QuickMove,
            (2, 2) => Input::Swap,
            (2, 4) => Input::Throw,
            _ => return self.lan_merchant_click_plain(conn, p),
        };
        if self.menu_click_in_progress(conn) {
            return true;
        }
        if !mod_api::__private::listened(mod_api::HookId::TradeWithVillager) {
            return self.lan_merchant_click_plain(conn, p);
        }
        let Some(mob) = self.server.lan.as_ref().and_then(|host| {
            let gm = host.menus.get(&conn)?.merchant.as_ref()?;
            // A non-quick-craft click while a drag is active only resets the drag (`doClick`).
            (gm.window == p.container && !gm.drag.active())
                .then_some(gm.merchant.mob)
                .flatten()
        }) else {
            return self.lan_merchant_click_plain(conn, p);
        };
        if self
            .common
            .mobs
            .iter()
            .find(|m| m.id == mob)
            .is_some_and(|m| {
                opaque_villager(
                    &self.stations().professions,
                    m.species(),
                    &m.extra.profession,
                )
            })
        {
            return self.lan_merchant_click_plain(conn, p);
        }
        let Some(actor) = self.guest_actor(conn) else {
            self.resend_merchant_take(conn, p.container);
            return true;
        };
        let identity = Identity {
            side: Side::Server,
            world: actor.epoch(),
            actor,
            target: mod_api::EntityRef::new(actor.epoch(), mob),
        };
        let route = self
            .server
            .lan
            .as_mut()
            .unwrap()
            .menus
            .entry(conn)
            .or_default()
            .merchant
            .id();
        self.storage_scope(|game, _| {
            game.journal_guest_merchant(conn);
            let level = game.transactions.last_mut().unwrap();
            level.trades.active = Some(conn);
            level.trades.correct_on_abort.insert((conn, p.container));
            if level.packets.is_none() {
                level.packets = Some(Scope::begin());
            }
            // The coordinator contains listener unwinds; this guard also closes the click if a
            // lower menu rule panics during planning or a provisional guest turn unwinds.
            let failures = mod_api::runtime::isolated_failures();
            let accepted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                game.run_merchant_take(conn, p, input, identity, route)
            }))
            .unwrap_or(false)
                && mod_api::runtime::isolated_failures() == failures;
            ((), accepted)
        });
        true
    }
    /// Screen replacement cannot publish while a transaction owns a click. The older
    /// merchant guard protects its own guest; a scoped mod click can reach any guest's menu.
    pub(in crate::game::server_side) fn menu_click_in_progress(&self, conn: u32) -> bool {
        self.common
            .transactions
            .iter()
            .any(|level| level.menu_click_active.is_some() || level.trades.active == Some(conn))
    }
    pub(in crate::game) fn resend_merchant_take(&mut self, conn: u32, window: i32) {
        self.send_merchant_take(conn, window, true);
    }
    pub(in crate::game::server_side) fn run_merchant_take(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
        input: Input,
        identity: Identity,
        route: sunrise_gameplay_common::game::transactions::decision::MenuId,
    ) -> bool {
        let mob = identity.target.id();
        let button = i32::from(p.button);
        let mut stacked = None::<StackedOutcome>;
        if input == Input::Pickup && sunrise_gameplay_common::game::stacked_listened() {
            let mut seen = None;
            self.lan_merchant_op_with(
                conn,
                Some(p.container),
                false,
                MerchantRun::Probe,
                |menu, env| {
                    seen = menu.stacked_click(env, 2, button, input);
                },
            );
            if let Some(frozen) = seen.filter(sunrise_gameplay_common::game::stacked_listens_for) {
                let host = self.server.lan.as_ref().unwrap();
                let stamp = host.menus[&conn].merchant.stamp();
                let hub_generation = host.hub.guest(conn).expect("planned guest").menu_generation;
                // The shared pickup decision checks its journal epoch as well as these stacks.
                let (same, resolved_slot, primary) = (frozen.clone(), frozen.slot, frozen.primary);
                if self.stacked_pickup(
                    route,
                    frozen,
                    |game| {
                        let host = game.server.lan.as_ref()?;
                        let guest = host.hub.guest(conn)?;
                        let gm = host.menus.get(&conn)?.merchant.as_ref()?;
                        (game.guest_actor(conn) == Some(identity.actor)
                            && host.menus.get(&conn)?.merchant.stamp() == stamp
                            && gm.window == p.container
                            && gm.merchant.mob == Some(mob)
                            && guest.menu_generation == hub_generation
                            && gm.slots[2] == same.clicked
                            && guest.carried == same.carried)
                            .then(|| same.clone())
                    },
                    |game, carried, clicked| {
                        let mut took = false;
                        let _ = game.lan_merchant_op_with(
                            conn,
                            Some(p.container),
                            false,
                            MerchantRun::Probe,
                            |menu, env| {
                                took = menu.probe_carried_vanilla_override(
                                    env,
                                    resolved_slot,
                                    primary,
                                    carried,
                                    clicked,
                                );
                            },
                        );
                        took
                    },
                    |_, answer| stacked = answer.cloned(),
                ) == Pickup::Refused
                {
                    return false;
                }
            }
        }

        let mut previous = None;
        let mut throw_amount = 1;
        let mut edited = false;
        let mut refresh = false;
        loop {
            let Some(host) = self.server.lan.as_ref() else {
                return false;
            };
            let Some(guest) = host.hub.guest(conn) else {
                return false;
            };
            let Some(owner) = host.menus.get(&conn).map(|menus| &menus.merchant) else {
                return false;
            };
            let Some(gm) = owner.as_ref() else {
                return false;
            };
            let mut reads = TakeReads {
                conn,
                window: p.container,
                identity,
                stamp: owner.stamp(),
                hub_generation: guest.menu_generation,
                creative: guest.creative(),
                map_id: None,
            };
            if input == Input::Throw && button != 0 && previous.is_none() {
                throw_amount = gm.slots[2].as_ref().map_or(0, |stack| stack.count);
            }
            let run = stacked
                .as_ref()
                .map_or(MerchantRun::Plain, |s| MerchantRun::Decided(Some(s)));
            let mut moved = None;
            let map_id = self.peek_map_id();
            let Some((menu, made)) = self.read_merchant_menu(conn, gm, run, |menu, env| {
                moved =
                    menu.merchant_result_step(env, button, input, previous.as_ref(), throw_amount);
            }) else {
                return false;
            };
            if stacked.as_ref().is_some_and(|s| s.refused.get()) {
                log::warn!(
                    "item_stacked_on_other: the slot refused the replacement; nothing changed"
                );
                return false;
            }
            if !made.is_empty() {
                reads.map_id = Some(map_id);
            }
            let session = menu.merchant.as_ref().expect("planned merchant");
            refresh |= session
                .events
                .iter()
                .any(|event| matches!(event, Event::OffersRefreshed));
            let trade = session.events.iter().find_map(|event| match event {
                Event::Traded(index, offer) => Some((*index, offer.clone())),
                _ => None,
            });
            let player_slots = (0..36)
                .filter(|&slot| menu.player_item(slot) != guest.inventory[slot].as_ref())
                .map(|slot| (slot, menu.player_item(slot).cloned()))
                .collect();
            let offhand = (menu.player_item(40) != guest.offhand.as_ref())
                .then(|| menu.player_item(40).cloned());
            let plan = TakePlan {
                menu,
                made,
                player_slots,
                offhand,
            };
            if !reads.fresh(self) {
                return false;
            }
            let Some((index, offer)) = trade else {
                // Untaken/handled cases can still have a cursor change, but no trade hook.
                self.apply_merchant_take(&reads, plan);
                break;
            };
            let plan = RefCell::new(Some(plan));
            let mut changed = false;
            let accepted = self.decided_action_on(
                "trade_with_villager",
                DecidedTarget { route, identity },
                Epoch::Player,
                |game| reads.fresh(game),
                |game| {
                    let mut slot = plan.borrow_mut();
                    let plan = slot.as_mut().expect("frozen trade");
                    let crafted = std::mem::take(&mut plan.menu.crafted_counts);
                    if !crafted.is_empty() {
                        game.as_guest(conn, |g| {
                            for (item, count) in crafted {
                                g.award_stat("crafted", &item, count);
                            }
                        });
                    }
                    let kind = game
                        .mobs
                        .iter()
                        .find(|m| m.id == mob)
                        .map(sunrise_gameplay_common::game::native_hooks::mob_kind);
                    let Some(kind) = kind else {
                        return Reply::Failed;
                    };
                    let mut answer = (*offer).clone();
                    let Some(outcome) = game.as_guest(conn, |g| {
                        g.record_merchant_trade(Some(mob), &offer.result);
                        g.native_trade_with_villager(mob, &kind, &mut answer)
                    }) else {
                        return Reply::Failed;
                    };
                    match outcome {
                        Ok(true) => {
                            let session = plan.menu.merchant.as_mut().expect("planned session");
                            // A level-up may recalculate all prices after the Traded snapshot.
                            // Preserve it when the listener did not edit this special price.
                            if answer.special_price_diff == offer.special_price_diff {
                                answer.special_price_diff =
                                    session.offers[index].special_price_diff;
                            }
                            session.offers[index] = answer;
                            changed = true;
                            plan.menu.merchant_update_result(game.items());
                            Reply::Allow
                        }
                        Ok(false) => Reply::Allow,
                        Err(reason) => Reply::Invalid(reason),
                    }
                },
                |game| game.apply_merchant_take(&reads, plan.take().expect("one frozen take")),
            );
            if !accepted {
                return false;
            }
            edited |= changed;
            if moved.is_none() {
                break;
            }
            previous = moved;
        }
        self.send_merchant_take(conn, p.container, refresh || edited);
        true
    }
    pub(in crate::game::server_side) fn send_merchant_take(
        &mut self,
        conn: u32,
        window: i32,
        offers: bool,
    ) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(open) = host.menus.get_mut(&conn).map(|p| &mut p.merchant) else {
            return;
        };
        if open.as_ref().is_none_or(|gm| gm.window != window) {
            return;
        }
        let mut gm = open.take().expect("matching window");
        if offers {
            self.lan_send_offers(conn, &mut gm);
        }
        self.lan_send_merchant_content(conn, &mut gm);
        if let Some(host) = self.server.lan.as_mut() {
            host.menus
                .entry(conn)
                .or_default()
                .merchant
                .restore(Some(gm));
        }
    }
}

pub(in crate::game) fn rollback_trade_journal(
    journal: Journal,
    game: &mut crate::game::ServerGame<'_>,
) {
    for (conn, kept) in journal.guests {
        let KeptMerchantGuest {
            merchant,
            stats,
            progress,
            changed,
            unlocked,
            book,
        } = *kept;
        if let Some(host) = game.server.lan.as_mut() {
            host.menus
                .entry(conn)
                .or_default()
                .merchant
                .restore(Some(merchant));
            if let Some(stats) = stats {
                host.play.entry(conn).or_default().stats = stats;
            }
            if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) {
                host.hub.guests.update(gi, |g| {
                    g.advancements = progress;
                    g.advancements_changed = changed;
                    g.recipes_unlocked = unlocked;
                    g.recipe_book = book;
                });
            }
        }
    }
    for (conn, kept) in journal.windows {
        let KeptMerchantWindow {
            window,
            heard,
            owned,
        } = kept;
        if let Some(host) = game.server.lan.as_mut() {
            host.hub.guests.update_connection(conn, window);
            let menus = host.menus.entry(conn).or_default();
            menus.menu_heard = heard;
            menus.owned_menu = owned;
        }
    }
    for (id, kept) in journal.mobs {
        let KeptMerchantMob {
            random,
            timer,
            trade,
            trading,
            profession,
            level,
            variant,
            villager,
        } = *kept;
        if let Some(m) = game.mobs.iter_mut().find(|m| m.id == id) {
            m.random = random;
            m.ambient_timer = timer;
            m.extra.trade = trade;
            m.extra.flag2 = trading;
            m.extra.profession = profession;
            m.extra.villager_level = level;
            m.extra.variant = variant;
            m.extra.villager = villager;
        }
    }
    if let Some(sequences) = journal.sequences {
        game.server.rand_sequences = sequences;
    }
}

impl TakeReads {
    pub(in crate::game) fn fresh(&self, game: &crate::game::ServerGame<'_>) -> bool {
        let Some(host) = game.server.lan.as_ref() else {
            return false;
        };
        let Some(guest) = host.hub.guest(self.conn) else {
            return false;
        };
        let mob = self.identity.target.id();
        game.guest_actor(self.conn) == Some(self.identity.actor)
            && host.menus.get(&self.conn).is_some_and(|menus| {
                menus.merchant.stamp() == self.stamp
                    && menus
                        .merchant
                        .as_ref()
                        .is_some_and(|gm| gm.window == self.window && gm.merchant.mob == Some(mob))
            })
            && game
                .mobs
                .iter()
                .any(|m| m.id == mob && m.alive() && !m.removed)
            && guest.menu_generation == self.hub_generation
            && guest.creative() == self.creative
            && self.map_id.is_none_or(|id| game.peek_map_id() == id)
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::stacked_on_other::StackedOnOtherGameExt1 as _;
