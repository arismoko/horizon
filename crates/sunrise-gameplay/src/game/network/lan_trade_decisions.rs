//! A subscribed guest's merchant result click is one savepoint. Each take has its own
//! frozen menu plan, collect and apply; a later refusal aborts every earlier iteration.
use super::trading::MadeMaps;
use sunrise_gameplay_common::game::lan_menu_data::{
    Effect, KeptMerchantGuest, KeptMerchantMob, KeptMerchantWindow,
};
use sunrise_gameplay_common::game::transactions::{
    After,
    decision::{Identity, OwnerStamp},
};
use sunrise_items::items::ItemStack;
use sunrise_sim::{advancements::Advancement, menu::Menu};

/// The live owner's generation covers its slots, offer list, selection, and packet snapshot.
/// The action's player epoch separately covers the cursor and player-owned destinations.
pub(in crate::game) struct TakeReads {
    pub(in crate::game) conn: u32,
    pub(in crate::game) window: i32,
    pub(in crate::game) identity: Identity,
    pub(in crate::game) stamp: OwnerStamp,
    pub(in crate::game) hub_generation: u64,
    pub(in crate::game) creative: bool,
    pub(in crate::game) map_id: Option<i32>,
}

pub(in crate::game) struct TakePlan {
    pub(in crate::game) menu: Menu,
    pub(in crate::game) made: MadeMaps,
    pub(in crate::game) player_slots: Vec<(usize, Option<ItemStack>)>,
    pub(in crate::game) offhand: Option<Option<ItemStack>>,
}

impl crate::game::ServerGame<'_> {
    /// Any write to an open merchant menu, including a MerchantRef edit to another guest,
    /// belongs to the transaction that first changed it.
    pub(in crate::game) fn journal_guest_merchant(&mut self, conn: u32) {
        let Some(level) = self.common.transactions.last() else {
            return;
        };
        if level.trades.guests.contains_key(&conn) {
            return;
        }
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let Some(merchant) = host
            .menus
            .get(&conn)
            .and_then(|menus| menus.merchant.clone())
        else {
            return;
        };
        // A refresh can run during the buyer's loan; it never mutates their stats.
        let stats = host.play.get(&conn).map(|play| play.stats.clone());
        let progress = g.advancements.clone();
        let changed = g.advancements_changed.clone();
        let unlocked = g.recipes_unlocked.clone();
        let book = g.recipe_book.clone();
        self.journal_guest_click(conn);
        let level = self
            .common
            .transactions
            .last_mut()
            .expect("the action is open");
        level.trades.guests.insert(
            conn,
            Box::new(KeptMerchantGuest {
                merchant,
                stats,
                progress,
                changed,
                unlocked,
                book,
            }),
        );
    }

    /// A replacement can close a funded window from a listener's transaction.
    /// Keep the player's wire window with the trade and hold any open/close
    /// packets until the frame commits.
    pub(in crate::game) fn journal_merchant_replacement(&mut self, conn: u32) {
        let Some(level) = self.common.transactions.last_mut() else {
            return;
        };
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(menus) = host.menus.get(&conn).filter(|m| m.merchant.is_some()) else {
            return;
        };
        let Some(guest) = host.hub.guest(conn) else {
            return;
        };
        level.trades.windows.entry(conn).or_insert_with(|| {
            let window = guest.menu_before_replacement();
            let heard = menus.menu_heard.clone();
            let owned = menus.owned_menu.clone();
            KeptMerchantWindow {
                window: Box::new(window),
                heard,
                owned,
            }
        });
        level
            .packets
            .get_or_insert_with(sunrise_sim::lan::conn::transaction_packets::Scope::begin);
    }

    /// Keep the fields merchant operations own, not the mob's health, equipment or effects,
    /// which belong to their owning journals.
    pub(in crate::game) fn journal_merchant_mob(&mut self, id: u32) {
        let Some(level) = self.common.transactions.last_mut() else {
            return;
        };
        let std::collections::hash_map::Entry::Vacant(entry) = level.trades.mobs.entry(id) else {
            return;
        };
        let Some(m) = self.common.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let random = m.random.clone();
        let timer = m.ambient_timer;
        let trade = m.extra.trade.clone();
        let trading = m.extra.flag2;
        let profession = m.extra.profession.clone();
        let level = m.extra.villager_level;
        let variant = m.extra.variant;
        let villager = m.extra.villager.clone();
        entry.insert(Box::new(KeptMerchantMob {
            random,
            timer,
            trade,
            trading,
            profession,
            level,
            variant,
            villager,
        }));
    }

    pub(in crate::game) fn merchant_take_active(&self) -> bool {
        self.common.transactions.iter().any(|l| l.packets.is_some())
    }

    pub(in crate::game) fn merchant_drops(
        &mut self,
        conn: u32,
        stacks: Vec<sunrise_items::items::ItemStack>,
    ) {
        if self.merchant_take_active() {
            if !stacks.is_empty() {
                self.after_commit(After::Trade(Box::new(Effect::Drops(conn, stacks))));
            }
        } else if let Some(gi) = self
            .server
            .lan
            .as_ref()
            .and_then(|h| h.hub.guests.iter().position(|g| g.conn == conn))
        {
            self.drop_guest_items(gi, stacks);
        }
    }

    pub(in crate::game) fn merchant_orb(&mut self, at: [f64; 3], value: u32) {
        if self.merchant_take_active() {
            self.after_commit(After::Trade(Box::new(Effect::Orb(at, value))));
        } else {
            self.add_orb(at, value);
        }
    }

    // The criterion and its listener run tentatively. Rewards can spawn entities and run
    // arbitrary functions, so their publication belongs to the successful root commit.
    pub(in crate::game) fn defer_merchant_reward(&mut self, advancement: &Advancement) -> bool {
        if !self.merchant_take_active() {
            return false;
        }
        let uuid = self.native_acting_id();
        self.after_commit(After::Trade(Box::new(Effect::Reward(
            uuid,
            advancement.clone(),
        ))));
        true
    }
    pub(in crate::game) fn merchant_sound(
        &mut self,
        sound: &'static str,
        at: [f64; 3],
        pitch: f32,
    ) {
        if self.merchant_take_active() {
            self.defer_effect(
                sunrise_gameplay_common::game::transactions::DeferredEffect::Sound {
                    sound,
                    at,
                    volume: 1.0,
                    pitch,
                },
            );
        } else {
            self.server_play_sound(sound, at, 1.0, pitch);
        }
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;
