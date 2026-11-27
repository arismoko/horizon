//! Trading for LAN guests: the villager screen a guest opens runs the same
//! merchant menu as the host's (`sunrise_client::menu`, `Kind::Merchant`),
//! with vanilla's packets: OpenScreen (`minecraft:merchant`), MerchantOffers,
//! the select trade packet, container clicks and the content sync after each.
use super::stacked_on_other::Pickup;
use sunrise_gameplay_common::game::{
    lan_actions::guest_within_entity_range,
    lan_menu_data::GuestMerchant,
    stations_common::{Kind as StationKind, Station},
    trading_common::MerchantState,
};
use sunrise_items::items::ItemStack;
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_sim::{
    menu::{Env, Input, Kind, Menu, QuickCraftDrag},
    merchant::{self, TradeFeedback},
};

/// A guest's UUID as gossip targets it.
fn uuid_words(uuid: u128) -> [i32; 4] {
    [
        (uuid >> 96) as i32,
        (uuid >> 64) as i32,
        (uuid >> 32) as i32,
        uuid as i32,
    ]
}

/// A villager whose profession the world's table cannot resolve (its mod is
/// gone): its trades skip the result processing and its screen closes.
pub(in crate::game) fn opaque_villager(
    professions: &sunrise_sim::mobs::villager::ResolvedProfessions,
    species: sunrise_sim::mobs::Species,
    profession: &sunrise_sim::mobs::villager::ProfessionState,
) -> bool {
    species == sunrise_sim::mobs::Species::Villager && professions.state_index(profession).is_none()
}

/// How a merchant menu op runs: ordinary, a read-only probe on a copy, or a pickup with a
/// stacked answer.
pub(in crate::game) enum MerchantRun<'a> {
    Plain,
    Probe,
    Decided(Option<&'a sunrise_sim::menu::StackedOutcome>),
}

impl crate::game::ServerGame<'_> {
    /// Development oracle: observe the actual open guest merchant, never a synthetic menu.
    pub(in crate::game::server_side) fn harness_merchant(
        &self,
        host: &super::lan::LanHost,
        conn: u32,
    ) -> Option<(i32, i32, serde_json::Value)> {
        use serde_json::json;
        let gm = host.menus.get(&conn)?.merchant.as_ref()?;
        let guest = host
            .hub
            .guests
            .iter()
            .find(|g| g.conn == conn && g.placed)?;
        let id = gm.merchant.mob?;
        self.common
            .mobs
            .iter()
            .find(|m| m.id == id && m.alive() && !m.removed)?;
        let stack = |s: &Option<ItemStack>| s.as_ref().map(|s| json!([s.item.as_ref(), s.count]));
        let session = &gm.merchant.session;
        Some((
            gm.window,
            gm.state,
            json!({"window":gm.window,"merchant":self.persist.uuids.get(&id)?,
            "player":uuid_words(guest.uuid),"slots":gm.slots.iter().map(stack).collect::<Vec<_>>(),
            "carried":stack(&guest.carried),"xp":session.xp,
            "sent_offers":gm.sent.iter().map(|o|json!({"uses":o.uses,
                "special":o.special_price_diff,"demand":o.demand,"cost":o.cost_a_count(self.items())})).collect::<Vec<_>>(),
            "offers":session.offers.iter().map(|o|json!({"uses":o.uses,
                "special":o.special_price_diff,"demand":o.demand,"cost":o.cost_a_count(self.items())})).collect::<Vec<_>>()}),
        ))
    }

    /// A trading screen the guest's interaction opened (it opened as a
    /// station while the guest acted): handed to the guest.
    pub fn lan_open_merchant(&mut self, conn: u32, mut state: MerchantState) {
        let Some(uuid) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().find(|g| g.conn == conn))
            .map(|g| uuid_words(g.uuid))
        else {
            return;
        };
        if let Some(id) = state.mob
            && state.session.villager
        {
            // startTrading with the guest's reputation. The session already
            // captured their Hero effect while they acted as the player.
            state.session.reputation = self.villager_reputation_of(id, uuid);
            state.session.update_special_prices();
        }
        let title = self.merchant_display_name(&state);
        if self.guest_may_open(conn, "minecraft:merchant", None) != Some(true) {
            self.merchant_closed(&mut state);
            return;
        }
        self.guest_replace_menu(conn, false, |game| {
            let Some(window) = game
                .server
                .lan
                .as_mut()
                .and_then(|host| host.hub.open_merchant_window(conn, title))
            else {
                game.merchant_closed(&mut state);
                return;
            };
            let mut gm = GuestMerchant {
                window,
                state: 0,
                slots: [None, None, None],
                drag: QuickCraftDrag::default(),
                merchant: Box::new(state),
                sent: Vec::new(),
                sent_level: 0,
                sent_xp: 0,
                sent_result: None,
            };
            game.lan_send_offers(conn, &mut gm);
            game.lan_send_merchant_content(conn, &mut gm);
            if let Some(host) = game.server.lan.as_mut() {
                host.menus
                    .entry(conn)
                    .or_default()
                    .merchant
                    .restore(Some(gm));
            }
            game.player_menu_opening(conn);
        })
    }
    /// `Entity.getDisplayName` of the merchant, the screen's title: its name
    /// (custom, else the profession's) with the show_entity hover event and
    /// its UUID as insertion.
    fn merchant_display_name(&mut self, state: &MerchantState) -> sunrise_core::nbt::Tag {
        use sunrise_core::nbt::Tag;
        let s = &state.session;
        let (kind, key) = if s.villager {
            let p = s.profession.trim_start_matches("minecraft:");
            (
                "minecraft:villager",
                format!("entity.minecraft.villager.{p}"),
            )
        } else {
            (
                "minecraft:wandering_trader",
                "entity.minecraft.wandering_trader".to_owned(),
            )
        };
        let name = match &state.name {
            Some(n) => sunrise_game_data::scoreboard::json_to_tag(&n.component()),
            None => Tag::compound([("translate", Tag::String(key.as_str().into()))]),
        };
        let uuid = state.mob.map_or([0; 4], |id| self.mob_uuid(id));
        let w = uuid.map(|v| v as u32);
        let text = format!(
            "{:08x}-{:04x}-{:04x}-{:04x}-{:04x}{:08x}",
            w[0],
            w[1] >> 16,
            w[1] & 0xffff,
            w[2] >> 16,
            w[2] & 0xffff,
            w[3]
        );
        let mut out = match name.clone() {
            Tag::Compound(c) => c,
            other => {
                let mut c = sunrise_core::nbt::Compound::new();
                c.insert("text".into(), other);
                c
            }
        };
        out.insert(
            "hover_event".into(),
            Tag::compound([
                ("action", Tag::String("show_entity".into())),
                ("id", Tag::String(kind.into())),
                ("uuid", Tag::IntArray(uuid.to_vec())),
                ("name", name),
            ]),
        );
        out.insert("insertion".into(), Tag::String(text.as_str().into()));
        Tag::Compound(out)
    }
    pub(in crate::game::server_side) fn lan_send_offers(
        &mut self,
        conn: u32,
        gm: &mut GuestMerchant,
    ) {
        if let Some(host) = self.server.lan.as_mut() {
            Self::send_merchant_offers(&mut host.hub, conn, gm);
        }
    }

    /// `ServerPlayer.sendMerchantOffers`: send the live offers, and keep the
    /// packet's snapshot for clicks on this particular window.
    fn send_merchant_offers(
        hub: &mut sunrise_sim::lan::hub::Hub,
        conn: u32,
        gm: &mut GuestMerchant,
    ) {
        let s = &gm.merchant.session;
        let offers: Vec<play::NetOffer> = s
            .offers
            .iter()
            .filter_map(|o| {
                let cost = |st: &ItemStack, hub: &mut sunrise_sim::lan::hub::Hub| {
                    hub.stack(&Some(st.clone())).map(play::NetCost::from)
                };
                Some(play::NetOffer {
                    cost_a: cost(&o.base_cost_a.stack, hub)?,
                    result: hub.stack(&Some(o.result.clone()))?,
                    cost_b: match &o.cost_b {
                        Some(b) => Some(cost(&b.stack, hub)?),
                        None => None,
                    },
                    out_of_stock: o.out_of_stock(),
                    uses: o.uses,
                    max_uses: o.max_uses,
                    xp: o.xp,
                    special_price: o.special_price_diff,
                    price_multiplier: o.price_multiplier,
                    demand: o.demand,
                })
            })
            .collect();
        hub.send_to(
            conn,
            &C::MerchantOffers(play::MerchantOffers {
                container: gm.window,
                offers,
                level: s.level,
                xp: s.xp,
                show_progress: s.villager,
                can_restock: s.villager,
            }),
        );
        gm.sent = s.offers.clone();
        gm.sent_level = s.level;
        gm.sent_xp = s.xp;
    }
    pub(in crate::game::server_side) fn lan_send_merchant_content(
        &mut self,
        conn: u32,
        gm: &mut GuestMerchant,
    ) {
        if let Some(host) = self.server.lan.as_mut() {
            let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
                return;
            };
            let mut items: Vec<Option<ItemStack>> = gm.slots.to_vec();
            let g = &host.hub.guests[gi];
            items.extend((9..36).chain(0..9).map(|i| g.inventory[i].clone()));
            gm.state = (gm.state + 1) & 0x7FFF;
            gm.sent_result.clone_from(&gm.slots[2]);
            host.hub.send_menu_snapshot(gi, gm.window, gm.state, &items);
        }
    }
    /// Runs `op` on the guest's merchant menu (their inventory copied in and
    /// back), then syncs the client. False when the guest has no such window.
    pub(in crate::game::server_side) fn lan_merchant_op(
        &mut self,
        conn: u32,
        window: Option<i32>,
        refresh_offers: bool,
        op: impl FnOnce(&mut Menu, &Env),
    ) -> bool {
        self.lan_merchant_op_with(conn, window, refresh_offers, MerchantRun::Plain, op)
    }

    /// `lan_merchant_op`, run as a decision asks (see `MerchantRun`).
    pub(in crate::game::server_side) fn lan_merchant_op_with(
        &mut self,
        conn: u32,
        window: Option<i32>,
        refresh_offers: bool,
        run: MerchantRun<'_>,
        op: impl FnOnce(&mut Menu, &Env),
    ) -> bool {
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        let Some(gi) = host.hub.guests.connection_index(conn) else {
            return false;
        };
        // Clicks and closes need the player's own inventory, not the body swapped out
        // into their guest entry. Price refreshes below only touch the merchant slots.
        if self.server.lan_guest_swaps.contains(&conn) {
            return false;
        }
        let Some(gm) = host.menus.get(&conn).and_then(|p| p.merchant.as_ref()) else {
            return false;
        };
        if window.is_some_and(|window| window != gm.window) {
            return false;
        }
        let probe = matches!(run, MerchantRun::Probe);
        let refused = match &run {
            MerchantRun::Decided(Some(stacked)) => Some(&stacked.refused),
            _ => None,
        };
        let Some((mut menu, made)) = self.read_merchant_menu(conn, gm, run, op) else {
            return false;
        };
        if probe || refused.is_some_and(std::cell::Cell::get) {
            return true;
        }
        self.journal_guest_merchant(conn);
        let host = self.server.lan.as_mut().expect("open merchant");
        let mut gm = host.menus.get_mut(&conn).unwrap().merchant.take().unwrap();
        let session = menu.merchant.take().expect("attached");
        let dropped = std::mem::take(&mut menu.dropped);
        let crafted_counts = std::mem::take(&mut menu.crafted_counts);
        gm.slots = [
            menu.slot(0).cloned(),
            menu.slot(1).cloned(),
            menu.slot(2).cloned(),
        ];
        gm.drag = menu.quick_craft_state();
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.guests.update(gi, |g| {
                for i in 0..36 {
                    g.inventory[i] = menu.player_item(i).cloned();
                }
                g.offhand = menu.player_item(40).cloned();
                g.carried = menu.carried.take();
            });
        }
        self.register_exploration_maps(made);
        if !crafted_counts.is_empty() {
            self.as_guest(conn, |g| {
                for (item, count) in crafted_counts {
                    g.award_stat("crafted", &item, count);
                }
            });
        }
        let opaque = gm
            .merchant
            .mob
            .and_then(|id| self.common.mobs.iter().find(|m| m.id == id))
            .is_some_and(|m| {
                opaque_villager(
                    &self.stations().professions,
                    m.species(),
                    &m.extra.profession,
                )
            });
        let offers_refreshed = session
            .events
            .iter()
            .any(|event| matches!(event, merchant::Event::OffersRefreshed));
        let (mut session, traded) = if opaque {
            (session, Vec::new())
        } else {
            self.merchant_apply(session, gm.merchant.mob, TradeFeedback::Current)
        };
        if !traded.is_empty() {
            // The guest's own statistic and advancements.
            let mob = gm.merchant.mob;
            self.as_guest(conn, |g| {
                g.merchant_traded(mob, &mut session.offers, &traded)
            });
        }
        gm.merchant.session = session;
        self.merchant_drops(conn, dropped);
        if refresh_offers || offers_refreshed {
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
        true
    }
    /// Execute only on a detached menu. The caller decides if and when to publish it.
    pub(in crate::game::server_side) fn read_merchant_menu(
        &self,
        conn: u32,
        gm: &GuestMerchant,
        run: MerchantRun<'_>,
        op: impl FnOnce(&mut Menu, &Env),
    ) -> Option<(Menu, super::trading::MadeMaps)> {
        let mut session = gm.merchant.session.clone();
        Self::merchant_sync_in(
            &mut session,
            self.common
                .mobs
                .iter()
                .find(|m| Some(m.id) == gm.merchant.mob),
        );
        let next_map = self.peek_map_id();
        let tag = |t: &str| self.structure_tag(t);
        let maps = super::trading::GameMaps {
            terrain: self.common.end.locator(),
            structure_tag: &tag,
            made: Default::default(),
            next_id: std::cell::Cell::new(next_map),
        };
        let trade_env = merchant::TradeEnv {
            trades: &self.stations().trades,
            enchantments: &self.stations().enchantments,
            // A level-up offer can locate and record a structure even on a detached menu.
            maps: if matches!(run, MerchantRun::Probe) {
                None
            } else {
                Some(&maps)
            },
        };
        let host = self.server.lan.as_ref()?;
        let g = host.hub.guest(conn)?;
        let (player_uuid, player_entity_id) = (g.uuid, g.entity_id);
        let env = Env {
            crafting_remainder: Some(self),
            player: Some(player_uuid),
            player_entity_id: Some(player_entity_id),
            loop_context: &self.common.loop_context,
            items: self.items(),
            recipes: self.recipes(),
            brewing: &self.stations().brewing,
            banners: &self.stations().banners,
            station_recipes: self.station_recipes(),
            enchantments: &self.stations().enchantments,
            names: &self.common.pack.names,
            trade: Some(&trade_env),
            maps: None,
            limited_crafting: host.hub.limited_crafting,
            can_equip: None,
            known_recipes: Some(&g.recipe_book),
            slot_rules: Some(self),
            stacked: None,
        };
        let mut menu = Menu::new(Kind::Merchant);
        menu.set_registry_id(Some("minecraft:merchant"));
        for i in 0..36 {
            menu.set_player_item(i, g.inventory[i].clone());
        }
        for (i, stack) in g.armor.iter().enumerate() {
            menu.set_player_item(39 - i, stack.clone());
        }
        menu.set_player_item(40, g.offhand.clone());
        menu.selected = g.selected;
        menu.creative = g.creative();
        menu.carried = g.carried.clone();
        for (i, s) in gm.slots.iter().enumerate() {
            menu.set_slot(&env, i, s.clone());
        }
        menu.merchant = Some(session);
        menu.set_quick_craft_state(gm.drag.clone());
        let env = match &run {
            MerchantRun::Decided(stacked) => Env {
                stacked: *stacked,
                ..env
            },
            _ => env,
        };
        op(&mut menu, &env);
        Some((menu, maps.made.take()))
    }

    /// Close an open merchant whose profession became unresolved, returning inputs.
    pub(in crate::game::server_side) fn close_opaque_merchant(&mut self, id: u32) {
        if self
            .common
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
            .is_some_and(|m| m.mob == Some(id))
        {
            self.close_container_server();
        }
        if let Some(conn) = self.lan_trade_conn(id) {
            let window = self
                .server
                .lan
                .as_ref()
                .and_then(|h| h.menus.get(&conn))
                .and_then(|p| p.merchant.as_ref())
                .map(|m| m.window);
            if let Some(window) = window {
                self.reply_menu_click(
                    conn,
                    sunrise_gameplay_common::game::lan_menu_data::ClickReply::Close(window),
                );
            }
        }
    }

    /// A container click on the guest's trading window.
    pub(in crate::game::server_side) fn lan_merchant_click_plain(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        let input = match p.input {
            0 => Input::Pickup,
            1 => Input::QuickMove,
            2 => Input::Swap,
            3 => Input::Clone,
            4 => Input::Throw,
            5 => Input::QuickCraft,
            _ => Input::PickupAll,
        };
        let (slot, button) = (i32::from(p.slot), i32::from(p.button));
        let op = move |menu: &mut Menu, env: &Env| {
            let _ = menu.click(env, slot, button, input);
        };
        if !sunrise_gameplay_common::game::stacked_listened()
            || !matches!(input, Input::Pickup | Input::QuickCraft)
        {
            return self.lan_merchant_op(conn, Some(p.container), false, op);
        }
        // Freeze what the click finds as the guest's trade state makes it, without a write.
        let mut found = None;
        self.lan_merchant_op_with(
            conn,
            Some(p.container),
            false,
            MerchantRun::Probe,
            |menu, env| {
                found = menu.stacked_click(env, slot, button, input);
            },
        );
        let Some(frozen) = found.filter(sunrise_gameplay_common::game::stacked_listens_for) else {
            return self.lan_merchant_op(conn, Some(p.container), false, op);
        };
        let route = match self.server.lan.as_mut() {
            Some(host) => host.menus.entry(conn).or_default().hub.id(),
            None => return true,
        };
        let gi = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().position(|g| g.conn == conn));
        let (carried, same, resolved_slot, primary) = (
            frozen.carried.clone(),
            frozen.clone(),
            frozen.slot,
            frozen.primary,
        );
        let mut handled = true;
        let pickup = self.stacked_pickup(
            route,
            frozen,
            move |game| {
                let now = game
                    .server
                    .lan
                    .as_ref()?
                    .hub
                    .guests
                    .get(gi?)?
                    .carried
                    .clone();
                (now == carried).then(|| same.clone())
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
            |game, answer| {
                game.journal_guest_click(conn);
                handled = game.lan_merchant_op_with(
                    conn,
                    Some(p.container),
                    false,
                    MerchantRun::Decided(answer),
                    op,
                );
            },
        );
        if pickup == Pickup::Refused {
            self.resend_merchant_take(conn, p.container);
            return true;
        }
        handled
    }
    /// ServerboundSelectTradePacket: the selection hint, then tryMoveItems.
    pub fn lan_select_trade(&mut self, conn: u32, index: i32) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let Some((uuid, name)) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().find(|g| g.conn == conn))
            .map(|guest| (guest.uuid, guest.name.clone()))
        else {
            return;
        };
        if self.lan_merchant_op(conn, None, false, |menu, env| {
            menu.merchant_select(env, index)
        }) {
            let villager = self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.menus.get(&conn))
                .and_then(|p| p.merchant.as_ref())
                .and_then(|m| m.merchant.mob);
            self.native_trade_select(uuid, &name, villager, index);
        }
    }
    /// The guest closed the trading window (or it is closed for them): MerchantMenu.removed restores the villager's state.
    pub(in crate::game) fn lan_merchant_removed(
        &mut self,
        conn: u32,
        window: Option<i32>,
        removal: super::lan_crafting::MenuRemoval,
    ) -> bool {
        let Some(gi) = self.guest_index(conn) else {
            return false;
        };
        self.journal_guest_merchant(conn);
        let Some(mut gm) = self
            .server
            .lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            .and_then(|menu| {
                menu.merchant
                    .take_if(|gm| window.is_none_or(|window| gm.window == window))
            })
        else {
            return false;
        };
        // `MerchantMenu.removed` takes only the cursor and payment slots.
        // Save their destination through the same loan used to return them.
        self.journal_guest_inventory(conn);
        self.journal_trade_sequences();
        if let Some(id) = gm.merchant.mob {
            self.journal_merchant_mob(id);
        }
        let carried = self.close_guest_menu_cursor(gi, gm.window);
        let [a, b, _] = gm.slots;
        let mob = gm
            .merchant
            .mob
            .and_then(|id| self.common.mobs.iter().find(|m| m.id == id));
        Self::merchant_sync_in(&mut gm.merchant.session, mob);
        self.merchant_closed(&mut gm.merchant);
        if removal == super::lan_crafting::MenuRemoval::Live {
            self.lan_place_back(
                gi,
                carried
                    .into_iter()
                    .chain([a, b].into_iter().flatten())
                    .collect(),
            );
        } else {
            self.lan_place_back(gi, carried.into_iter().collect());
            self.merchant_drops(conn, [a, b].into_iter().flatten().collect());
        }
        true
    }
    /// `Merchant.stillValid` for a trader: it is alive and the guest is within
    /// the entity interaction range plus 4 of its box.
    pub(in crate::game::server_side) fn lan_merchant_valid(
        &self,
        g: &sunrise_sim::lan::guest::Guest,
        id: u32,
    ) -> bool {
        self.common
            .mobs
            .iter()
            .find(|m| m.id == id)
            .is_some_and(|m| {
                m.alive()
                    && !opaque_villager(
                        &self.stations().professions,
                        m.species(),
                        &m.extra.profession,
                    )
                    && guest_within_entity_range(g, &m.bounds(), 4.0)
            })
    }

    /// `ServerPlayer.tick`: broadcast menu changes before checking `stillValid` for one guest.
    pub(in crate::game) fn lan_merchant_tick_one(&mut self, conn: u32) {
        let Some((window, sync_result, close)) = self.server.lan.as_ref().and_then(|host| {
            let gm = host.menus.get(&conn)?.merchant.as_ref()?;
            let g = host.hub.guest(conn).filter(|g| host.hub.here(g))?;
            let sync_result = gm.slots[2] != gm.sent_result;
            let close = gm
                .merchant
                .mob
                .is_some_and(|id| !self.lan_merchant_valid(g, id));
            (sync_result || close).then_some((gm.window, sync_result, close))
        }) else {
            return;
        };
        if sync_result
            && let Some(host) = self.server.lan.as_mut()
            && let Some(gm) = host
                .menus
                .get_mut(&conn)
                .and_then(|menu| menu.merchant.as_mut())
            && gm.window == window
            && gm.slots[2] != gm.sent_result
        {
            gm.state = (gm.state + 1) & 0x7FFF;
            gm.sent_result.clone_from(&gm.slots[2]);
            let item = host.hub.stack(&gm.sent_result);
            host.hub.send_to(
                conn,
                &C::ContainerSetSlot(play::ContainerSetSlot {
                    container: window,
                    state: gm.state,
                    slot: 2,
                    item,
                }),
            );
        }
        if close {
            self.reply_menu_click(
                conn,
                sunrise_gameplay_common::game::lan_menu_data::ClickReply::Close(window),
            );
        }
    }
    /// `ServerPlayer.tick`: broadcast menu changes before checking `stillValid`.
    #[cfg(test)]
    pub(in crate::game) fn lan_merchant_tick(&mut self) {
        let conns: Vec<u32> = self
            .server
            .lan
            .as_ref()
            .map(|host| {
                host.menus
                    .iter()
                    .filter(|(_, p)| p.merchant.is_some())
                    .map(|(c, _)| *c)
                    .collect()
            })
            .unwrap_or_default();
        for conn in conns {
            if !self.player_tick_canceled(conn) {
                self.lan_merchant_tick_one(conn);
            }
        }
    }
    /// `Villager.updateSpecialPrices` notifies the current buyer's menu only if
    /// offers exist. `MerchantContainer.updateSellItem` only makes a sound with payment.
    pub(in crate::game::server_side) fn villager_trade_will_notify(
        &self,
        id: u32,
        uuid: [i32; 4],
    ) -> bool {
        if self.common.level.player_uuid == Some(uuid)
            && let Some(station) = &self.common.station
            && station
                .merchant
                .as_ref()
                .is_some_and(|m| m.mob == Some(id) && !m.session.offers.is_empty())
            && station.inputs.iter().any(Option::is_some)
        {
            return true;
        }
        let Some(host) = &self.server.lan else {
            return false;
        };
        let Some(guest) = host
            .hub
            .holder_guest(
                sunrise_sim::lan::playerdata::ints_uuid(uuid),
                self.common.dim.id(),
            )
            .filter(|guest| host.hub.here(guest))
        else {
            return false;
        };
        host.menus
            .get(&guest.conn)
            .and_then(|menu| menu.merchant.as_ref())
            .is_some_and(|m| {
                m.merchant.mob == Some(id)
                    && !m.merchant.session.offers.is_empty()
                    && m.slots[..2].iter().any(Option::is_some)
            })
    }

    /// Refresh open guest prices after the actual villager reputation event.
    pub(in crate::game) fn lan_merchant_reputation(
        &mut self,
        id: u32,
        only: Option<[i32; 4]>,
        feedback: TradeFeedback,
    ) {
        let conns: Vec<u32> = self
            .server
            .lan
            .as_ref()
            .map(|host| {
                host.menus
                    .iter()
                    .filter(|(_, p)| {
                        p.merchant.as_ref().is_some_and(|m| {
                            m.merchant.session.villager
                                && m.merchant.mob == Some(id)
                                && !m.merchant.session.offers.is_empty()
                        })
                    })
                    .map(|(c, _)| *c)
                    .collect()
            })
            .unwrap_or_default();
        for conn in conns {
            let Some(uuid) = self
                .server
                .lan
                .as_ref()
                .and_then(|host| {
                    host.hub
                        .guests
                        .iter()
                        .find(|g| g.conn == conn && host.hub.here(g))
                })
                .map(|g| uuid_words(g.uuid))
            else {
                continue;
            };
            if only.is_some_and(|only| only != uuid) {
                continue;
            }
            let reputation = self.villager_reputation_of(id, uuid);
            self.journal_guest_merchant(conn);
            self.journal_merchant_mob(id);
            let items = server_world_items!(self);
            let host = self.server.lan.as_mut().expect("the buyer's host");
            let Some(gm) = host
                .menus
                .get_mut(&conn)
                .and_then(|menu| menu.merchant.as_mut())
            else {
                continue;
            };
            let mob = self.common.mobs.iter_mut().find(|m| m.id == id);
            let session = &mut gm.merchant.session;
            Self::merchant_sync_in(session, mob.as_deref());
            session.reputation = reputation;
            session.update_special_prices();
            let [a, b, result] = &mut gm.slots;
            let voice = session.update_sell_item(items, a.as_ref(), b.as_ref(), result, feedback);
            let silent = if let Some(mob) = mob {
                mob.random.clone_from(&session.random);
                mob.ambient_timer = session.ambient_sound_time;
                mob.silent
            } else {
                false
            };
            let (window, origin) = (gm.window, session.origin);
            if let Some((yes, pitch)) = voice {
                // The window is live while the sound hook runs, so a replacement
                // can close it and return the actual payment slots.
                self.merchant_trade_feedback(true, yes, origin, pitch, feedback, silent);
            }
            // A replacement can have opened a different menu in the hook. Do
            // not send the old offers or touch the replacement's result slot.
            if let Some(host) = self.server.lan.as_mut()
                && let Some(gm) = host
                    .menus
                    .get_mut(&conn)
                    .and_then(|menu| menu.merchant.as_mut())
                && gm.window == window
            {
                Self::send_merchant_offers(&mut host.hub, conn, gm);
            }
        }
    }
    /// Takes a station a guest's action opened: a trading screen or a
    /// workstation menu goes to the guest (game/network/lan_stations.rs).
    pub(in crate::game::server_side) fn lan_station_opened(&mut self, conn: u32, station: Station) {
        let mut station = station;
        if station.kind == StationKind::Merchant
            && let Some(m) = station.merchant.take()
        {
            self.lan_open_merchant(conn, *m);
            return;
        }
        self.lan_open_station(conn, &station);
    }
}

impl crate::game::ServerGame<'_> {
    /// The guest whose trading screen is open on mob `mob`.
    pub(in crate::game) fn lan_trade_conn(&self, mob: u32) -> Option<u32> {
        self.lan_trade_screen(mob).map(|(conn, _)| conn)
    }
    /// The live trading screen and its guest, even during a borrowed guest turn.
    pub(in crate::game::server_side) fn lan_trade_screen(
        &self,
        mob: u32,
    ) -> Option<(u32, &GuestMerchant)> {
        let host = self.server.lan.as_ref()?;
        host.menus.iter().find_map(|(&conn, state)| {
            let screen = state.merchant.as_ref()?;
            (screen.merchant.mob == Some(mob)).then_some((conn, screen))
        })
    }
    /// The live session of a guest's open trading screen, whose offers,
    /// experience and level are newer than the mob's until the screen closes.
    pub fn lan_trade_session(&self, conn: u32) -> Option<&merchant::Session> {
        let gm = self
            .server
            .lan
            .as_ref()?
            .menus
            .get(&conn)?
            .merchant
            .as_ref()?;
        Some(&gm.merchant.session)
    }
    /// The guest's player UUID.
    pub(in crate::game) fn lan_trade_player(&self, conn: u32) -> Option<u128> {
        Some(self.server.lan.as_ref()?.hub.guest(conn)?.uuid)
    }
    /// Changes the session of a guest's open trading screen and sends the
    /// client its offers and result slot again. The merchant's own random is
    /// not drawn: the update sound is held back.
    pub fn lan_trade_edit(&mut self, conn: u32, edit: impl FnOnce(&mut merchant::Session)) -> bool {
        if self.menu_click_in_progress(conn) {
            return false;
        }
        let done = self.lan_merchant_op(conn, None, false, |menu, env| {
            let Some(session) = menu.merchant.as_mut() else {
                return;
            };
            edit(session);
            session.selection_hint = session
                .selection_hint
                .min(session.offers.len().saturating_sub(1));
            // Suppress feedback even if the declared interval wraps the reply threshold.
            let sound_time = std::mem::replace(&mut session.ambient_sound_time, i32::MIN);
            menu.merchant_update(env);
            if let Some(session) = menu.merchant.as_mut() {
                session.ambient_sound_time = sound_time;
            }
        });
        if done {
            self.lan_trade_sequences(conn);
        }
        done
    }
    /// Lends the open session the world sequences of the levels above its
    /// level, and gives back the ones it no longer needs, so a level-up after
    /// the level was changed still rolls its trade set.
    fn lan_trade_sequences(&mut self, conn: u32) {
        let Some(session) = self.lan_trade_session(conn) else {
            return;
        };
        if !session.villager {
            return;
        }
        let needed = merchant::Session::sequences_needed(
            &self.stations().trades,
            &session.profession,
            session.level,
        );
        let stale: Vec<String> = session
            .sequences
            .keys()
            .filter(|id| !needed.contains(id))
            .cloned()
            .collect();
        if stale.is_empty() && needed.iter().all(|id| session.sequences.contains_key(id)) {
            return;
        }
        self.journal_trade_sequences();
        let Some(gm) = self
            .server
            .lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            .and_then(|p| p.merchant.as_mut())
        else {
            return;
        };
        let session = &mut gm.merchant.session;
        for id in stale {
            if let Some(random) = session.sequences.remove(&id) {
                self.server.rand_sequences.give_back(id, random);
            }
        }
        for id in needed {
            if let std::collections::btree_map::Entry::Vacant(slot) = session.sequences.entry(id) {
                let random = self
                    .server
                    .rand_sequences
                    .lend(slot.key(), self.common.level.seed);
                slot.insert(random);
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use sunrise_gameplay_common::game::server_side::native_merchant_refs::tests::offer;

    use super::*;
    use crate::game::lan::LanSettings;
    use crate::game::server_side::native_merchant_refs::tests::spawn;
    use std::sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize},
    };
    use sunrise_protocol::packets::configuration::ClientInformation;
    use sunrise_sim::{
        lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
        merchant::{MerchantOffer, Trader},
    };

    /// A hub with one guest (conn 12) trading with a farmer villager (mob 40)
    /// at `level`; the guest's channel comes back with the villager.
    pub fn open_trade(
        game: &mut crate::game::ServerGame<'_>,
        level: i32,
    ) -> (
        mod_api::MerchantRef,
        std::sync::mpsc::Receiver<sunrise_sim::lan::Out>,
        MerchantOffer,
    ) {
        let settings = LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        };
        let net = NetConfig {
            listen: false,
            announce: false,
            lod_distance: 0,
            ..NetConfig::lan(0, String::new(), 2)
        };
        let config = HubConfig {
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: game.dim.id().to_owned(),
            allow_flight: true,
            seed: 0,
            max_players: 2,
            integrated: true,
            levels: game.server.level_shapes.clone(),
            level_seeds: game.server.levels.0.runtime.seeds(),
        };
        game.lan_host(settings, net, config).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 12,
                uuid: 12,
                name: "guest12".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            12,
            [3.0, 64.0, 5.0],
            10,
        );
        guest.placed = true;
        let window = guest.next_menu_window();
        game.server.lan.as_mut().unwrap().hub.guests.push(guest);

        let villager = spawn(game, 40, "minecraft:villager");
        game.mobs[0].extra.profession = sunrise_sim::mobs::villager::ProfessionState::from_index(5);
        let profession = "minecraft:farmer".to_owned();
        let first = offer("minecraft:wheat", "minecraft:emerald");
        game.mobs[0].extra.trade = Some(Box::new(Trader {
            offers: vec![first.clone()],
            ..Trader::default()
        }));
        game.mobs[0].extra.flag2 = true;
        let mut session = merchant::Session::new(vec![first.clone()], true);
        session.level = level;
        session.profession = profession;
        session.xp = 0;
        session.variant = "minecraft:plains".into();
        for id in
            merchant::Session::sequences_needed(&game.stations().trades, &session.profession, level)
        {
            let random = game.server.rand_sequences.lend(&id, game.level.seed);
            session.sequences.insert(id, random);
        }
        let gm = GuestMerchant {
            window,
            state: 0,
            slots: [None, None, None],
            drag: QuickCraftDrag::default(),
            merchant: Box::new(MerchantState {
                session: Box::new(session),
                scroll: 0,
                dragging: false,
                focused: None,
                mob: Some(40),
                name: None,
            }),
            sent: vec![first.clone()],
            sent_level: level,
            sent_xp: 0,
            sent_result: None,
        };
        game.server
            .lan
            .as_mut()
            .unwrap()
            .menus
            .entry(12)
            .or_default()
            .merchant
            .restore(Some(gm));
        (villager, rx, first)
    }

    #[cfg(test)]
    #[test]
    fn a_registered_profession_trades_normally_and_an_unregistered_one_stays_opaque() {
        use sunrise_sim::mobs::{
            Species,
            villager::{ProfessionState, ResolvedProfessions},
        };
        let id = |s| mod_api::Identifier::parse(s).unwrap();
        let engineer = mod_api::ProfessionDefinition {
            workstations: vec![],
            key: mod_api::ProfessionKey(id("test:engineer")),
            held_poi: vec![mod_api::PoiKey(id("test:assembler"))],
            acquirable_poi: vec![mod_api::PoiKey(id("test:assembler"))],
            requested_items: vec![],
            secondary_blocks: vec![],
            work_sound: None,
            trades: vec![(1, mod_api::TradeSetKey(id("test:engineer")))],
        };
        let table =
            ResolvedProfessions::prepare(vec![("test".into(), engineer)], |_, _| true).unwrap();
        let registered = table
            .state(table.resolve("test:engineer").unwrap())
            .unwrap();
        assert!(matches!(registered, ProfessionState::Registered { .. }));
        assert!(
            !opaque_villager(&table, Species::Villager, &registered),
            "a registered mod profession gets the normal trade processing"
        );
        let missing = ProfessionState::Unresolved("gone:smith".into());
        assert!(opaque_villager(&table, Species::Villager, &missing));
        assert!(!opaque_villager(
            &table,
            Species::Villager,
            &ProfessionState::from_index(5)
        ));
        assert!(!opaque_villager(&table, Species::WanderingTrader, &missing));
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::stacked_on_other::StackedOnOtherGameExt1 as _;
