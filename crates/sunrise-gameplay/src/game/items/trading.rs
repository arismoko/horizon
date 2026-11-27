//! The merchant (villager trading) menu: `MerchantMenu` / `MerchantScreen`
//! layout (276x166, drawn 50 px left of the usual 176-wide origin), offer
//! buttons, payment slots, the result slot's `onTake`, villager experience and
//! level-ups with new offers from the next trade set.
use crate::game::ServerGame;
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::stations_common::{Kind, Station};
use sunrise_gameplay_common::game::trading_common::MerchantState;
use sunrise_items::{items::ItemStack, random_sequences::RandomSequences};
use sunrise_sim::merchant::{self, MerchantOffer, TradeContext, TradeFeedback};
use sunrise_sim::mobs::{
    Mob, Species,
    villager::{self, VILLAGER_TYPES},
};

/// `ExplorationMapFunction` in the game world: the nearest structure of the
/// tag through the generator's `/locate`, a new map centred on it with its
/// target decoration.
pub(in crate::game) type MadeMaps = Vec<(i32, [i32; 3], u8, String)>;

#[derive(Clone)]
pub(in crate::game) struct GameMaps<'a> {
    pub(in crate::game) terrain: Option<std::sync::Arc<sunrise_sim::vanillagen::VanillaTerrain>>,
    /// Structure tag -> its structures.
    pub(in crate::game) structure_tag: &'a dyn Fn(&str) -> Vec<String>,
    /// Maps made, for the caller to register: (map id, centre x, z, scale, target).
    pub(in crate::game) made: std::cell::RefCell<MadeMaps>,
    pub(in crate::game) next_id: std::cell::Cell<i32>,
}
impl merchant::MapMaker for GameMaps<'_> {
    fn with_staged(&self, edit: &mut dyn FnMut(&dyn merchant::MapMaker) -> bool) -> bool {
        let staged = self.clone();
        if !edit(&staged) {
            return false;
        }
        self.next_id.set(staged.next_id.get());
        self.made.replace(staged.made.into_inner());
        true
    }
    fn exploration_map(
        &self,
        stack: ItemStack,
        map: &merchant::ExplorationMap,
        origin: [f64; 3],
    ) -> ItemStack {
        let Some(terrain) = self.terrain.as_ref() else {
            return stack;
        };
        let names = (self.structure_tag)(&map.destination);
        if names.is_empty() {
            return stack;
        }
        let from = [
            origin[0].floor() as i32,
            origin[1].floor() as i32,
            origin[2].floor() as i32,
        ];
        let Some((_, at)) =
            terrain.locate_for_map(&names, from, map.search_radius, map.skip_existing_chunks)
        else {
            return stack;
        };
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.made
            .borrow_mut()
            .push((id, at, map.zoom, map.decoration.clone()));
        // The trade's own map item (woodland_mansion_map, ...) keeps its id.
        let mut out = stack;
        let d = out.data_mut();
        d.map_id = Some(id);
        d.set_other(
            "minecraft:map_decorations",
            Some(sunrise_core::nbt::Tag::compound([(
                "+",
                sunrise_core::nbt::Tag::compound([
                    (
                        "type",
                        sunrise_core::nbt::Tag::String(map.decoration.as_str().into()),
                    ),
                    (
                        "x",
                        sunrise_core::nbt::Tag::Double(f64::from(at[0]).to_bits()),
                    ),
                    (
                        "z",
                        sunrise_core::nbt::Tag::Double(f64::from(at[2]).to_bits()),
                    ),
                    (
                        "rotation",
                        sunrise_core::nbt::Tag::Float(180.0f32.to_bits()),
                    ),
                ]),
            )])),
        );
        out
    }
}

/// The world's `data/minecraft/random_sequences.dat`.
pub(in crate::game::server_side) fn random_sequences_file(
    world_root: &std::path::Path,
) -> std::path::PathBuf {
    world_root.join("data/minecraft/random_sequences.dat")
}
/// `RandomSequences` as opening the world reads it. A world with no file gets new data,
/// which `MinecraftServer`'s `savedDataStorage.computeIfAbsent(RandomSequences.TYPE)` stores
/// dirty (`SavedDataStorage.set`): its first save writes it.
pub(in crate::game::server_side) fn read_random_sequences(
    world_root: &std::path::Path,
) -> RandomSequences {
    sunrise_sim::save::read_dat(&random_sequences_file(world_root))
        .and_then(|t| t.get("data").cloned())
        .map(|d| RandomSequences::from_nbt(&d))
        .unwrap_or_else(RandomSequences::new_dirty)
}
/// `SavedDataStorage.saveAndJoin` for `RandomSequences`: written only while dirty, with
/// whatever it holds then, so the emptied map and the salt a `/random reset *` left reach
/// the file after a draw, and not without one.
pub(in crate::game::server_side) fn write_random_sequences(
    world_root: &std::path::Path,
    data: &mut RandomSequences,
) {
    let Some(data) = data.take_dirty_nbt() else {
        return;
    };
    let tag = sunrise_core::nbt::Tag::compound([
        ("data", data),
        (
            "DataVersion",
            sunrise_core::nbt::Tag::Int(sunrise_world::DATA_VERSION),
        ),
    ]);
    if let Err(e) = sunrise_sim::save::write_dat(&random_sequences_file(world_root), tag) {
        log::warn!("the villager random sequences were not written: {e}");
    }
}

impl ServerGame<'_> {
    /// `RandomSequences` as `random_sequences.dat` (not on a server's guest).
    pub(in crate::game::server_side) fn save_random_sequences(&mut self) {
        if self.common.is_logical_server() {
            write_random_sequences(&self.server.world_root, &mut self.server.rand_sequences);
        }
    }
    /// `AbstractVillager.addOffersFromTradeSet` with the world's random
    /// sequence of the set, at the merchant's position.
    pub(in crate::game::server_side) fn offers_from_set(
        &mut self,
        set: &str,
        variant: &str,
        origin: [f64; 3],
    ) -> Vec<MerchantOffer> {
        let Some(sequence) = self.stations().trades.sequence(set) else {
            return Vec::new();
        };
        self.journal_trade_sequences();
        let mut random = self
            .server
            .rand_sequences
            .lend(&sequence, self.common.level.seed);
        let next_map = self.peek_map_id();
        let tag = |t: &str| self.structure_tag(t);
        let maps = GameMaps {
            terrain: self.common.end.locator(),
            structure_tag: &tag,
            made: Default::default(),
            next_id: std::cell::Cell::new(next_map),
        };
        let ctx = TradeContext {
            items: self.items(),
            enchantments: &self.stations().enchantments,
            variant,
            origin,
            maps: Some(&maps),
        };
        let offers = self
            .stations()
            .trades
            .offers_from_set(&ctx, set, &mut random);
        let made = maps.made.into_inner();
        self.server.rand_sequences.give_back(sequence, random);
        self.register_exploration_maps(made);
        offers
    }
    /// `AbstractVillager.getOffers`: generated on first use.
    pub fn ensure_offers(&mut self, index: usize) {
        let species = self.common.mobs[index].species();
        if species == Species::Villager
            && self
                .stations()
                .professions
                .state_index(&self.common.mobs[index].extra.profession)
                .is_none()
        {
            return;
        }
        let unset = self.common.mobs[index]
            .extra
            .trade
            .as_ref()
            .is_none_or(|t| t.unset);
        if unset {
            self.journal_merchant_mob(self.common.mobs[index].id);
            let m = &self.common.mobs[index];
            let variant = format!(
                "minecraft:{}",
                VILLAGER_TYPES[m.extra.variant as usize % VILLAGER_TYPES.len()]
            );
            let origin = [m.position.x, m.position.y, m.position.z];
            let offers = if species == Species::WanderingTrader {
                let mut v = Vec::new();
                for set in ["buying", "uncommon", "common"] {
                    v.extend(self.offers_from_set(
                        &format!("minecraft:wandering_trader/{set}"),
                        &variant,
                        origin,
                    ));
                }
                v
            } else {
                let profession = m.extra.profession.id();
                let level = m.extra.villager_level.max(1);
                let set = self.stations().trades.set_for(&profession, level);
                if self.stations().trades.sets.contains_key(&set) {
                    self.offers_from_set(&set, &variant, origin)
                } else {
                    Vec::new()
                }
            };
            let trade = self.common.mobs[index].extra.trade.get_or_insert_default();
            trade.offers = offers;
            trade.unset = false;
        }
    }
    /// `Villager.mobInteract` / `WanderingTrader.mobInteract`: shake the
    /// head without offers, else open the trading screen.
    pub fn interact_trader(&mut self, index: usize, item: &str) -> sunrise_game_data::Result<bool> {
        let m = &self.common.mobs[index];
        let species = m.species();
        if !matches!(species, Species::Villager | Species::WanderingTrader)
            || !m.alive()
            || item == "minecraft:villager_spawn_egg"
            || self.common.station.is_some()
            || m.extra.flag2
            || (species == Species::Villager && villager::sleeping(m))
        {
            return Ok(false);
        }
        if m.is_baby() {
            // A baby villager shakes its head; a baby wandering trader does
            // nothing of its own.
            if species == Species::WanderingTrader {
                return Ok(false);
            }
            let mut events = Vec::new();
            villager::set_unhappy(&mut self.common.mobs[index], &mut events);
            self.handle_mob_events(events)?;
            return Ok(true);
        }
        let unresolved = species == Species::Villager
            && self
                .stations()
                .professions
                .state_index(&m.extra.profession)
                .is_none();
        self.award_custom_stat("talked_to_villager", 1);
        if unresolved {
            let mut events = Vec::new();
            villager::set_unhappy(&mut self.common.mobs[index], &mut events);
            self.handle_mob_events(events)?;
            return Ok(true);
        }
        self.ensure_offers(index);
        let m = &self.common.mobs[index];
        let trade = m.extra.trade.as_deref().cloned().unwrap_or_default();
        if trade.offers.is_empty() {
            if species == Species::Villager {
                let mut events = Vec::new();
                villager::set_unhappy(&mut self.common.mobs[index], &mut events);
                self.handle_mob_events(events)?;
            }
            return Ok(true);
        }
        let villager = species == Species::Villager;
        let profession = if villager {
            m.extra.profession.id()
        } else {
            "minecraft:wandering_trader".into()
        };
        let level = if villager {
            m.extra.villager_level.max(1)
        } else {
            1
        };
        let mut session = merchant::Session::new(trade.offers, villager);
        session.xp = trade.xp;
        session.level = level;
        session.profession = profession;
        session.variant = format!(
            "minecraft:{}",
            VILLAGER_TYPES[m.extra.variant as usize % VILLAGER_TYPES.len()]
        );
        session.take_mob_clock(m);
        session.origin = [m.position.x, m.position.y, m.position.z];
        let (id, name) = (
            m.id,
            m.custom_name.as_ref().map(|plain| {
                m.custom_name_text
                    .clone()
                    .unwrap_or_else(|| Text::literal(plain))
            }),
        );
        if villager {
            session.reputation = self.villager_reputation(id);
            session.hero = self.hero_amplifier();
            for seq in merchant::Session::sequences_needed(
                &self.stations().trades,
                &session.profession,
                level,
            ) {
                let r = self
                    .server
                    .rand_sequences
                    .lend(&seq, self.common.level.seed);
                session.sequences.insert(seq, r);
            }
        }
        // startTrading: updateSpecialPrices, then the screen opens.
        session.update_special_prices();
        // setTradingPlayer: a wandering trader stands still while trading, a
        // villager's brain follows and looks at the player
        // (LookAndFollowTradingPlayerSink).
        self.common.mobs[index].extra.flag2 = true;
        self.open_merchant_state_server(MerchantState {
            session: Box::new(session),
            scroll: 0,
            dragging: false,
            focused: None,
            mob: Some(id),
            name,
        });
        Ok(true)
    }
    /// `Villager.setTradingPlayer(null)`: the session goes back to the mob
    /// (special prices stay as they were), the borrowed sequences to the world.
    pub fn merchant_closed(&mut self, state: &mut MerchantState) {
        let session = &mut state.session;
        for (id, r) in std::mem::take(&mut session.sequences) {
            self.server.rand_sequences.give_back(id, r);
        }
        let Some(id) = state.mob else { return };
        let professions = self.stations().professions.clone();
        let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        m.extra.flag2 = false;
        if session.villager && professions.state_index(&m.extra.profession).is_none() {
            return;
        }
        if session.villager {
            m.extra.villager_level = session.level.max(1);
        }
        m.extra.flag2 = false;
        session.give_mob_clock(m);
        let trade = m.extra.trade.get_or_insert_default();
        trade.xp = session.xp;
        trade.offers = session.offers.clone();
    }
    /// Opens a merchant menu without a mob (sunrise-ctl `trade`).
    #[allow(dead_code)]
    pub(in crate::game) fn open_merchant(&mut self, profession: &str, variant: &str, level: i32) {
        let profession = if profession.contains(':') {
            profession.to_owned()
        } else {
            format!("minecraft:{profession}")
        };
        let variant = format!("minecraft:{}", variant.trim_start_matches("minecraft:"));
        let level = level.clamp(1, 5);
        let origin = self
            .common
            .player
            .as_ref()
            .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z]);
        let mut offers = Vec::new();
        for l in 1..=level {
            let set = self.stations().trades.set_for(&profession, l);
            offers.extend(self.offers_from_set(&set, &variant, origin));
        }
        let mut session = merchant::Session::new(offers, true);
        session.profession = profession;
        session.variant = variant;
        session.level = level;
        session.xp = merchant::min_xp(level);
        session.origin = origin;
        self.open_merchant_state_server(MerchantState {
            session: Box::new(session),
            scroll: 0,
            dragging: false,
            focused: None,
            mob: None,
            name: None,
        });
    }
    pub(in crate::game::server_side) fn open_merchant_state_server(
        &mut self,
        state: MerchantState,
    ) {
        assert!(
            self.is_logical_server(),
            "open_merchant_state_server called on Client role"
        );
        let mut s = Station::new_kind(Kind::Merchant, [0, 0, 0]);
        s.merchant = Some(Box::new(state));
        self.common.station = Some(s);
        self.server_set_acting_use(false);
    }
    /// The trading mob's random before a menu click (it ticks meanwhile).
    pub(in crate::game) fn merchant_sync_in(session: &mut merchant::Session, mob: Option<&Mob>) {
        if let Some(m) = mob {
            session.take_mob_clock(m);
            session.origin = [m.position.x, m.position.y, m.position.z];
        }
    }
    /// After a menu click: the session back to the station, its random to
    /// the mob, and what it did (orbs, level-ups, the yes/no voice).
    #[allow(dead_code)]
    pub(in crate::game) fn merchant_sync_out(&mut self, session: Box<merchant::Session>) {
        let mob = self
            .common
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
            .and_then(|m| m.mob);
        if mob
            .and_then(|id| self.common.mobs.iter().find(|m| m.id == id))
            .is_some_and(|m| {
                m.species() == Species::Villager
                    && self
                        .stations()
                        .professions
                        .state_index(&m.extra.profession)
                        .is_none()
            })
        {
            if let Some(m) = self
                .common
                .station
                .as_mut()
                .and_then(|s| s.merchant.as_mut())
            {
                m.session = session;
            }
            return;
        }
        let (mut session, traded) = self.merchant_apply(session, mob, TradeFeedback::Current);
        self.merchant_traded(mob, &mut session.offers, &traded);
        if let Some(m) = self
            .common
            .station
            .as_mut()
            .and_then(|s| s.merchant.as_mut())
        {
            m.session = session;
        }
    }
    /// What a menu click's session did, applied to the world and its mob;
    /// with the index of the offer of each trade made, for `merchant_traded`.
    pub(in crate::game::server_side) fn merchant_apply(
        &mut self,
        session: Box<merchant::Session>,
        mob: Option<u32>,
        feedback: TradeFeedback,
    ) -> (Box<merchant::Session>, Vec<usize>) {
        let mut session = session;
        let events = std::mem::take(&mut session.events);
        let mut traded = Vec::new();
        let (villager, origin) = (session.villager, session.origin);
        if let Some(id) = mob {
            self.journal_merchant_mob(id);
        }
        let mut silent = false;
        if let Some(m) = mob.and_then(|id| self.common.mobs.iter_mut().find(|m| m.id == id)) {
            session.give_mob_clock(m);
            silent = m.silent;
            if villager {
                m.extra.villager_level = session.level.max(1);
            }
            // The mob's offers are the live ones (the brain restocks them).
            if let Some(t) = m.extra.trade.as_mut() {
                t.offers.clone_from(&session.offers);
            }
        }
        for e in events {
            match e {
                merchant::Event::OffersRefreshed => {}
                merchant::Event::Orb(v) => {
                    self.merchant_orb([origin[0], origin[1] + 0.5, origin[2]], v as u32)
                }
                merchant::Event::LevelUp(_) => {
                    if let Some(id) = mob {
                        let _ = self.add_mob_effect(
                            id,
                            sunrise_level::effects::EffectInstance::new("regeneration", 200, 0),
                            None,
                        );
                    }
                }
                merchant::Event::TradeUpdated(yes, pitch) => {
                    self.merchant_trade_feedback(villager, yes, origin, pitch, feedback, silent);
                }
                merchant::Event::Traded(index, _offer) => {
                    traded.push(index);
                }
                merchant::Event::TradeSound => {
                    let sound = if villager {
                        "minecraft:entity.villager.yes"
                    } else {
                        "minecraft:entity.wandering_trader.yes"
                    };
                    self.merchant_sound(sound, origin, 1.0);
                }
            }
        }
        (session, traded)
    }

    /// `AbstractVillager.notifyTradeUpdated`: a hurt-triggered voice must be
    /// broadcast before the hit continues to its offers and damage.
    pub(in crate::game::server_side) fn merchant_trade_feedback(
        &mut self,
        villager: bool,
        yes: bool,
        at: [f64; 3],
        pitch: f32,
        feedback: TradeFeedback,
        silent: bool,
    ) {
        let sound = match (villager, yes) {
            (true, true) => "minecraft:entity.villager.yes",
            (true, false) => "minecraft:entity.villager.no",
            (false, true) => "minecraft:entity.wandering_trader.yes",
            (false, false) => "minecraft:entity.wandering_trader.no",
        };
        if matches!(feedback, TradeFeedback::AtHurt(_)) && !self.merchant_take_active() {
            if !silent {
                self.play_immediate_level_sound(
                    sunrise_gameplay_common::game::level_sounds_common::LevelSound::new(
                        sound, at, 1.0, pitch,
                    ),
                );
            }
        } else {
            self.merchant_sound(sound, at, pitch);
        }
    }
    /// `MerchantResultSlot.onTake` for each trade made (`traded` indices into
    /// the session's live `offers`): the statistic, `AbstractVillager.notifyTrade`'s
    /// `villager_trade` trigger, then the mods' `native_trade_with_villager`
    /// for a server's player (when the trader is still in the level), which
    /// may change the live offer (Bukkit's `PlayerTradeEvent#getTrade`).
    pub(in crate::game::server_side) fn merchant_traded(
        &mut self,
        mob: Option<u32>,
        offers: &mut [MerchantOffer],
        traded: &[usize],
    ) {
        for index in traded {
            let Some(offer) = offers.get_mut(*index) else {
                continue;
            };
            self.record_merchant_trade(mob, &offer.result);
            // TradeWithVillagerEvent: on the server, in the player's turn.
            if let (Some(id), Some(_)) = (mob, self.server.lan_acting)
                && let Some(kind) = self
                    .common
                    .mobs
                    .iter()
                    .find(|m| m.id == id)
                    .map(sunrise_gameplay_common::game::native_hooks::mob_kind)
            {
                let _ = self.native_trade_with_villager(id, &kind, offer);
            }
        }
    }

    /// The completed trade's criterion and statistic, before its collect hook.
    pub(in crate::game::server_side) fn record_merchant_trade(
        &mut self,
        mob: Option<u32>,
        result: &sunrise_items::items::ItemStack,
    ) {
        self.award_custom_stat("traded_with_villager", 1);
        if let Some(id) = mob {
            self.journal_merchant_mob(id);
            self.villager_trade_pending(id);
            if let Some(villager) = self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| self.mob_entity(m))
            {
                self.trigger_trade(&villager, result);
            }
        }
    }
    #[allow(dead_code)]
    pub(in crate::game) fn merchant_scroll(&mut self, d: f32) -> bool {
        let Some(m) = self
            .common
            .station
            .as_mut()
            .and_then(|s| s.merchant.as_mut())
        else {
            return false;
        };
        if m.session.offers.len() > 7 {
            let max = m.session.offers.len() - 7;
            m.scroll = scrolled(m.scroll, max, d);
            return true;
        }
        false
    }
}

/// `MerchantScreen.mouseScrolled`: `Mth.clamp((int)(scrollOff - scrollY), 0, max)`,
/// so a fractional wheel step that truncates to zero does not move the list.
#[allow(dead_code)]
pub(in crate::game::server_side) fn scrolled(scroll: usize, max: usize, d: f32) -> usize {
    ((scroll as f64 - f64::from(d)) as i32).clamp(0, max as i32) as usize
}

#[cfg(test)]
#[path = "tests/random_sequences_save_tests.rs"]
pub(in crate::game) mod random_sequences_save_tests;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    /// `MerchantScreen.mouseScrolled`: the offset is `(int)(scrollOff - scrollY)`
    /// clamped, so the list moves by as many rows as the event's delta and a
    /// fractional delta still moves one row (the truncation is after the
    /// subtraction). Sunrise moved exactly one row per event.
    #[test]
    fn the_offers_scroll_by_the_wheel_delta() {
        assert_eq!(scrolled(3, 5, 1.0), 2, "one notch down");
        assert_eq!(scrolled(3, 5, -1.0), 4, "one notch up");
        assert_eq!(scrolled(3, 5, 0.25), 2, "a fraction still moves a row");
        assert_eq!(scrolled(3, 5, 2.5), 0, "a two-and-a-half notch event");
        assert_eq!(scrolled(0, 5, 1.0), 0, "clamped at the top");
        assert_eq!(scrolled(5, 5, -1.0), 5, "clamped at the bottom");
    }
}

#[cfg(test)]
pub(in crate::game) mod select_on_a_server_tests {}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;
