//! Merchant handles for native server mods: villagers and wandering traders.
use super::commands::Target;
use mod_api::{EntityRef, MerchantRef, PlayerRef};
use sunrise_core::identifier::Identifier;
use sunrise_sim::{
    merchant::{MerchantOffer, Trader},
    mobs::{
        Species,
        villager::{self, VILLAGER_TYPES},
    },
};

/// The index of a `minecraft:` registry id in a table of bare names.
fn registry_index(id: &str, names: &[&str]) -> Option<u8> {
    let id = Identifier::parse(id).ok()?;
    if id.namespace() != "minecraft" {
        return None;
    }
    names.iter().position(|n| *n == id.path()).map(|i| i as u8)
}

impl crate::game::ServerGame<'_> {
    fn native_merchant_index(&self, merchant: MerchantRef) -> Option<usize> {
        let Target::Mob(id) = self.native_entity_target(merchant.entity())? else {
            return None;
        };
        self.common.mobs.iter().position(|m| {
            m.id == id
                && !m.removed
                && matches!(m.species(), Species::Villager | Species::WanderingTrader)
        })
    }

    fn native_villager_index(&self, merchant: MerchantRef) -> Option<usize> {
        let index = self.native_merchant_index(merchant)?;
        (self.common.mobs[index].species() == Species::Villager).then_some(index)
    }

    /// Reads the offers, experience and level (1 for a trader) of the mob at
    /// `index`: from a guest's open trading screen if there is one, whose copy
    /// is newer until it closes. A trader whose offers are not rolled yet has none.
    fn merchant_read<T>(
        &self,
        index: usize,
        read: impl FnOnce(&[MerchantOffer], i32, i32) -> T,
    ) -> T {
        self.merchant_read_lent(&self.common.mobs[index], read)
    }

    /// The same read while an AI tick lends this mob out of `self.mobs`.
    pub(in crate::game) fn merchant_read_lent<T>(
        &self,
        mob: &sunrise_sim::mobs::Mob,
        read: impl FnOnce(&[MerchantOffer], i32, i32) -> T,
    ) -> T {
        if mob.species() == Species::Villager
            && self
                .stations()
                .professions
                .state_index(&mob.extra.profession)
                .is_none()
        {
            return read(
                &[],
                mob.extra.trade.as_deref().map_or(0, |t| t.xp),
                mob.extra.villager_level.max(1),
            );
        }
        if let Some(session) = self
            .lan_trade_conn(mob.id)
            .and_then(|conn| self.lan_trade_session(conn))
        {
            return read(&session.offers, session.xp, session.level);
        }
        let level = mob.extra.villager_level.max(1);
        match mob.extra.trade.as_deref() {
            Some(t) if !t.unset => read(&t.offers, t.xp, level),
            Some(t) => read(&[], t.xp, level),
            None => read(&[], 0, level),
        }
    }

    /// Changes the offers, experience and level of the mob at `index`: on a
    /// guest's open trading screen if there is one (and resent to them), else on
    /// the mob. False when a trade is open somewhere this cannot reach (the host
    /// window, or inside a hook of that trade's own click), or `edit` refused.
    fn merchant_write(
        &mut self,
        index: usize,
        edit: impl FnOnce(&mut Vec<MerchantOffer>, &mut i32, &mut i32) -> bool,
    ) -> bool {
        let mob = &self.common.mobs[index];
        if mob.species() == Species::Villager
            && self
                .stations()
                .professions
                .state_index(&mob.extra.profession)
                .is_none()
        {
            return false;
        }
        let id = mob.id;
        if let Some(conn) = self.lan_trade_conn(id) {
            let mut done = false;
            let sent = self.lan_trade_edit(conn, |session| {
                done = edit(&mut session.offers, &mut session.xp, &mut session.level);
                if done {
                    session
                        .events
                        .push(sunrise_sim::merchant::Event::OffersRefreshed);
                }
            });
            return sent && done;
        }
        if self.common.mobs[index].extra.flag2 {
            return false;
        }
        self.journal_merchant_mob(id);
        let mob = &mut self.common.mobs[index];
        let trader = mob.extra.trade.get_or_insert_with(|| {
            Box::new(Trader {
                unset: true,
                ..Trader::default()
            })
        });
        let mut level = mob.extra.villager_level.max(1);
        let done = edit(&mut trader.offers, &mut trader.xp, &mut level);
        if done && mob.species() == Species::Villager {
            mob.extra.villager_level = level.max(1);
        }
        done
    }

    /// Narrows an entity to a villager or wandering trader, or None for
    /// another kind, a stale handle, or a client.
    pub fn merchant(&self, entity: EntityRef) -> Option<MerchantRef> {
        let merchant = MerchantRef::new(entity);
        self.native_merchant_index(merchant).map(|_| merchant)
    }

    /// `AbstractVillager.getOffers`, as they are now (a copy). Offers not
    /// rolled yet read as empty: they are rolled when a player first trades,
    /// or by `add_offer`; reading never draws a random.
    pub(in crate::game) fn merchant_offers(
        &self,
        merchant: MerchantRef,
    ) -> Option<Vec<MerchantOffer>> {
        let index = self.native_merchant_index(merchant)?;
        Some(self.merchant_read(index, |offers, _, _| offers.to_vec()))
    }

    /// Replaces offer `index` (a player trading with it is sent the new
    /// offers). False for an index past the end, a stale handle, or an offer
    /// without a real item in `base_cost_a` or `result` (or `cost_b` when set).
    pub(in crate::game) fn merchant_set_offer(
        &mut self,
        merchant: MerchantRef,
        index: u32,
        offer: MerchantOffer,
    ) -> bool {
        let Some(mob) = self.native_merchant_index(merchant) else {
            return false;
        };
        self.merchant_write(mob, |offers, _, _| {
            let Some(slot) = offers.get_mut(index as usize) else {
                return false;
            };
            *slot = offer;
            true
        })
    }

    /// Appends an offer, as `getOffers().add(offer)` does. Offers not rolled
    /// yet are rolled first, as `getOffers` does for a player's first trade
    /// (this draws the trade set's world random sequence, as that does).
    /// False for a stale handle or an invalid offer (see `set_offer`).
    pub(in crate::game) fn merchant_add_offer(
        &mut self,
        merchant: MerchantRef,
        offer: MerchantOffer,
    ) -> bool {
        let Some(mob) = self.native_merchant_index(merchant) else {
            return false;
        };
        if self.lan_trade_conn(self.common.mobs[mob].id).is_none()
            && !self.common.mobs[mob].extra.flag2
        {
            self.ensure_offers(mob);
        }
        self.merchant_write(mob, |offers, _, _| {
            offers.push(offer);
            true
        })
    }

    /// Removes offer `index`, the ones after it moving up. False for an index
    /// past the end or a stale handle.
    pub(in crate::game) fn merchant_remove_offer(
        &mut self,
        merchant: MerchantRef,
        index: u32,
    ) -> bool {
        let Some(mob) = self.native_merchant_index(merchant) else {
            return false;
        };
        self.merchant_write(mob, |offers, _, _| {
            if index as usize >= offers.len() {
                return false;
            }
            offers.remove(index as usize);
            true
        })
    }

    /// `Merchant.getTradingPlayer`: the player whose trading screen is open on
    /// it. None when nobody trades, or the screen is the host window's.
    pub(in crate::game) fn merchant_trading_player(
        &self,
        merchant: MerchantRef,
    ) -> Option<PlayerRef> {
        let index = self.native_merchant_index(merchant)?;
        let conn = self.lan_trade_conn(self.common.mobs[index].id)?;
        self.player(self.lan_trade_player(conn)?)
    }

    /// `VillagerData.profession`, a registry id (`minecraft:farmer`; none is
    /// `minecraft:none`). None for a wandering trader.
    pub(in crate::game) fn merchant_profession(&self, merchant: MerchantRef) -> Option<String> {
        let mob = &self.common.mobs[self.native_villager_index(merchant)?];
        Some(mob.extra.profession.id())
    }

    /// `Villager.setVillagerData` with a new profession: a different one
    /// forgets the offers, which roll again on the next trade. Clients see the
    /// new profession through entity data. False for an unknown id, a wandering
    /// trader, or a villager with a trading screen open.
    pub(in crate::game) fn merchant_set_profession(
        &mut self,
        merchant: MerchantRef,
        profession: &str,
    ) -> bool {
        let Some(index) = self.native_villager_index(merchant) else {
            return false;
        };
        let Some(profession) = Identifier::parse(profession)
            .ok()
            .and_then(|id| self.stations().professions.resolve(&id.to_string()))
        else {
            return false;
        };
        let id = self.common.mobs[index].id;
        if self
            .stations()
            .professions
            .state_index(&self.common.mobs[index].extra.profession)
            .is_none()
            || self.common.mobs[index].extra.flag2
            || self.lan_trade_conn(id).is_some()
        {
            return false;
        }
        self.journal_merchant_mob(id);
        let table = self.stations().professions.clone();
        villager::set_profession(&mut self.common.mobs[index], &table, profession);
        true
    }

    /// `VillagerData.level`, at least 1.
    pub(in crate::game) fn merchant_level(&self, merchant: MerchantRef) -> Option<i32> {
        let index = self.native_villager_index(merchant)?;
        Some(self.merchant_read(index, |_, _, level| level.max(1)))
    }

    /// `VillagerData.withLevel`: `Math.max(1, level)`, with no upper limit in
    /// vanilla. The offers stay as they are; the next level-up adds its trades
    /// (a level past 5 can't level up). A trading guest's session follows,
    /// and gets the sequences a level-up from the new level needs.
    pub(in crate::game) fn merchant_set_level(
        &mut self,
        merchant: MerchantRef,
        level: i32,
    ) -> bool {
        let Some(index) = self.native_villager_index(merchant) else {
            return false;
        };
        let level = level.max(1);
        self.merchant_write(index, |_, _, current| {
            *current = level;
            true
        })
    }

    /// `VillagerData.type`, a registry id (`minecraft:plains`).
    pub(in crate::game) fn merchant_villager_type(&self, merchant: MerchantRef) -> Option<String> {
        let mob = &self.common.mobs[self.native_villager_index(merchant)?];
        let name = VILLAGER_TYPES[mob.extra.variant as usize % VILLAGER_TYPES.len()];
        Some(format!("minecraft:{name}"))
    }

    /// `VillagerData.withType`: also the type a trading guest's session
    /// rolls its next level-up offers with. False for an unknown id, a
    /// wandering trader, or a trade open where this can't reach.
    pub(in crate::game) fn merchant_set_villager_type(
        &mut self,
        merchant: MerchantRef,
        kind: &str,
    ) -> bool {
        let Some(index) = self.native_villager_index(merchant) else {
            return false;
        };
        let Some(kind) = registry_index(kind, &VILLAGER_TYPES) else {
            return false;
        };
        let id = self.common.mobs[index].id;
        if let Some(conn) = self.lan_trade_conn(id) {
            let variant = format!("minecraft:{}", VILLAGER_TYPES[kind as usize]);
            if !self.lan_trade_edit(conn, |session| session.variant = variant) {
                return false;
            }
        } else if self.common.mobs[index].extra.flag2 {
            return false;
        }
        self.journal_merchant_mob(id);
        self.common.mobs[index].extra.variant = kind;
        true
    }

    /// `Villager.getVillagerXp`.
    pub(in crate::game) fn merchant_xp(&self, merchant: MerchantRef) -> Option<i32> {
        let index = self.native_villager_index(merchant)?;
        Some(self.merchant_read(index, |_, xp, _| xp))
    }

    /// `Villager.setVillagerXp`: assigned as given, as vanilla does.
    pub(in crate::game) fn merchant_set_xp(&mut self, merchant: MerchantRef, xp: i32) -> bool {
        let Some(index) = self.native_villager_index(merchant) else {
            return false;
        };
        self.merchant_write(index, |_, current, _| {
            *current = xp;
            true
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    use sunrise_sim::mobs::Mob;

    pub fn spawn(game: &mut crate::game::ServerGame<'_>, id: u32, kind: &str) -> MerchantRef {
        if !game.common.world.chunks.contains_key(&(0, 0)) {
            game.common.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
        }
        let kind = game.res.mob_types().get(kind).unwrap().clone();
        game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
        let entity = game.entity(id).unwrap();
        game.merchant(entity).unwrap()
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
