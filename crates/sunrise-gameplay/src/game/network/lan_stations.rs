//! Workstation menus for LAN guests: the screen a guest's use opens runs on
//! the host's menu engine (`sunrise_client::menu`), the one single player
//! uses, with vanilla's packets: OpenScreen with the station's `MenuType`,
//! container clicks, `ServerboundContainerButtonClickPacket`, the content and
//! data slot sync after each (`broadcastChanges`), and the inputs back into
//! the inventory when the menu closes (`AbstractContainerMenu.clearContainer`).
use crate::game::ServerGame;
use mod_api::item::menu::AnvilCraftInputs;
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::{
    api_values::TryIntoApi,
    lan_menu_data::GuestStation,
    lan_station_data::menu_of,
    stations_common::{Kind as StationKind, Station},
    transactions::After,
};
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_sim::menu::{GrindstoneTakeEdits, Kind, Menu, QuickCraftDrag};

/// A guest's open lectern: `LecternMenu` over the block entity, made on the
/// menu engine for each click (`Game::lectern_menu`).
pub(in crate::game) struct GuestLectern {
    pub(in crate::game) window: i32,
    pub(in crate::game) state: i32,
    pub(in crate::game) key: (i32, i32, i32),
    /// Persistent quick-craft (drag) state across rebuilt lectern menus.
    pub(in crate::game) drag: QuickCraftDrag,
    /// The book and page the client was last sent (`broadcastChanges`).
    pub(in crate::game) sent: Option<(Option<ItemStack>, i32)>,
}

impl ServerGame<'_> {
    /// A workstation a guest's use opened (`ServerPlayer.openMenu`); false
    /// when guests can't use that one yet.
    pub fn lan_open_station(&mut self, conn: u32, station: &Station) -> bool {
        let Some((menu_type, kind, _)) = menu_of(station.kind) else {
            return false;
        };
        // The hub may already have the new button map while native advancement
        // hooks are still ahead of UpdateRecipes. Do not open on an unpublished map.
        if kind == Kind::Stonecutter
            && self
                .server
                .lan
                .as_ref()
                .is_some_and(|host| host.hub.native_reload_pending)
        {
            return true;
        }
        // The mods may stop the menu; the click is spent.
        if self.guest_may_open(conn, menu_type, Some(station.pos)) != Some(true) {
            return true;
        }
        self.guest_replace_menu(conn, false, |game| {
            let title = sunrise_protocol::text::translatable(station.kind.title_key(), &[]);
            let Some(window) = game
                .server
                .lan
                .as_mut()
                .and_then(|host| host.hub.open_window(conn, menu_type, title))
            else {
                return true;
            };
            let mut menu = Box::new(Menu::new(kind));
            menu.set_registry_id(Some(menu_type));
            if kind == Kind::Stonecutter {
                menu.stone.server_buttons = game
                    .server
                    .lan
                    .as_ref()
                    .map(|host| host.hub.sync_data().stonecutter_buttons.clone());
            }
            let hooked = !mod_api::__private::mods().is_empty();
            menu.anvil_hook = kind == Kind::Anvil;
            menu.grind_hook = kind == Kind::Grindstone;
            menu.item_smithed_hook = kind == Kind::Smithing && hooked;
            if kind == Kind::Enchanting {
                // Player.enchantmentSeed, made when first needed.
                let needs_seed = game
                    .server
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guests.iter().find(|g| g.conn == conn))
                    .is_some_and(|g| g.xp_seed == 0);
                let drawn = needs_seed.then(|| game.common.world.random.next_int());
                if let Some(host) = game.server.lan.as_mut() {
                    let _ = host.hub.update_guest(conn, |g| {
                        if let Some(drawn) = drawn {
                            g.xp_seed = drawn;
                        }
                        menu.data[3] = g.xp_seed;
                    });
                }
            }
            if kind == Kind::Beacon {
                menu.data = game.beacon_data(station.pos).to_vec();
            }
            if let Some(host) = game.server.lan.as_mut() {
                *host.menus.entry(conn).or_default().station = Some(GuestStation {
                    window,
                    state: 0,
                    kind: station.kind,
                    pos: station.pos,
                    menu,
                    sent_data: Vec::new(),
                    last_sound: 0,
                });
            }
            game.lan_send_station(conn);
            game.player_menu_opening(conn);
            true
        })
    }

    /// Runs `op` on the guest's open station (window `window`, any when
    /// None) with their inventory in it, then sends what changed. False when
    /// no such window is open.
    pub fn lan_station_op(
        &mut self,
        conn: u32,
        window: Option<i32>,
        op: impl FnOnce(&mut Menu, &sunrise_sim::menu::Env),
    ) -> bool {
        self.lan_station_op_clicked(conn, window, None, op)
    }

    /// Deliver any pending `AnvilMenu.createResult` or `GrindstoneMenu.createResult` hook
    /// live after a slot write or rename, updating `menu`'s result in place.
    pub(in crate::game::server_side) fn lan_station_deliver_result(
        &mut self,
        gi: usize,
        kind: StationKind,
        pos: [i32; 3],
        menu: &mut Menu,
    ) {
        let (uuid, entity_id) = self
            .server
            .lan
            .as_ref()
            .map(|host| (host.hub.guests[gi].uuid, host.hub.guests[gi].entity_id))
            .unwrap_or((0, 0));
        if kind == StationKind::Anvil
            && menu.anvil_created()
            && mod_api::__private::listened(mod_api::HookId::AnvilUpdate)
            && let Some((inputs, mut result)) = menu.anvil_update()
        {
            self.api_anvil_update(uuid, entity_id, pos, inputs, &mut result);
            menu.anvil_updated(result);
        }
        if kind == StationKind::Grindstone && menu.grindstone_created() && menu.grind_hook {
            if mod_api::__private::listened(mod_api::HookId::GrindstonePlace) {
                let mut cancel = false;
                let edits = self.api_grindstone_place(
                    uuid,
                    entity_id,
                    pos,
                    menu.grindstone_place(),
                    &mut cancel,
                );
                menu.grindstone_placed(edits, cancel);
            } else {
                menu.grindstone_placed(Default::default(), false);
            }
        }
    }

    /// `lan_station_op` for `op` that makes `click`, which the mods decide first when it is a
    /// pickup they listen to (`guest_menu_op_decided`).
    pub(in crate::game::server_side) fn lan_station_op_clicked(
        &mut self,
        conn: u32,
        window: Option<i32>,
        click: Option<super::lan_pickup::Click>,
        op: impl FnOnce(&mut Menu, &sunrise_sim::menu::Env),
    ) -> bool {
        let Some((owner, mut gs)) = self
            .server
            .lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            .and_then(|p| {
                let owner = p.station.id();
                Some((
                    owner,
                    p.station
                        .take_if(|s| window.is_none_or(|w| s.window == w))?,
                ))
            })
        else {
            return false;
        };
        let guest = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().position(|g| g.conn == conn))
            .filter(|&gi| {
                !self
                    .server
                    .lan
                    .as_ref()
                    .is_some_and(|h| h.hub.guests[gi].spectator())
            });
        if let Some(gi) = guest {
            let smithing_before = (gs.kind == StationKind::Smithing).then(|| {
                (0..=3)
                    .map(|s| gs.menu.slot(s).cloned())
                    .collect::<Vec<_>>()
            });
            let track_results = matches!(gs.kind, StationKind::Anvil | StationKind::Grindstone);
            let pos = gs.pos;
            let kind = gs.kind;
            if gs.kind == StationKind::Grindstone && !self.common.transactions.is_empty() {
                self.journal_guest_click(conn);
                self.journal_station_click(conn, owner, &gs);
            }
            let _ = gs.menu.anvil_created();
            let _ = gs.menu.grindstone_created();
            if let Some(click) = click.filter(|_| track_results) {
                self.guest_menu_action(
                    gi,
                    &mut gs.menu,
                    Some(click),
                    None,
                    |game, menu, stacked| {
                        let mut rest =
                            game.guest_menu_op_with(gi, menu, stacked, |menu, env| {
                                menu.track_results(true);
                                menu.start_click(env, click.0, click.1, click.2)
                            })?;
                        loop {
                            game.lan_station_deliver_result(gi, kind, pos, menu);
                            game.lan_station_finish_take(conn, gi, pos, menu);
                            let Some(step) = rest.ok().flatten() else {
                                break;
                            };
                            let next = game.guest_menu_op_with(gi, menu, None, |menu, env| {
                                menu.resume_click(env, step)
                            })?;
                            rest = Ok(next);
                        }
                        menu.track_results(false);
                        Some(())
                    },
                );
            } else {
                self.guest_menu_op_decided(gi, &mut gs.menu, click, None, |menu, env| {
                    op(menu, env);
                });
                if track_results {
                    self.lan_station_deliver_result(gi, kind, pos, &mut gs.menu);
                    self.lan_station_finish_take(conn, gi, pos, &mut gs.menu);
                }
            }
            if gs.kind == StationKind::Smithing
                && smithing_before.is_some_and(|slots| {
                    slots
                        .iter()
                        .enumerate()
                        .any(|(s, b)| gs.menu.slot(s) != b.as_ref())
                })
            {
                let (uuid, name) = self
                    .server
                    .lan
                    .as_ref()
                    .map(|host| {
                        let guest = &host.hub.guests[gi];
                        (guest.uuid, guest.name.clone())
                    })
                    .unwrap_or((0, String::new()));
                let mut result = gs.menu.slot(3).cloned();
                self.native_prepare_smithing(uuid, &name, gs.pos, &mut result);
                let env = self.menu_env();
                gs.menu.set_slot(&env, 3, result);
            }
            self.lan_station_sounds(&mut gs);
            if let Some(amount) = gs.menu.ground.take() {
                self.grindstone_taken(gs.pos, amount);
            }
            // ItemSmithedEvent comes before the rest of SmithingMenu.onTake.
            let smithed = std::mem::take(&mut gs.menu.item_smithed);
            if let Some((uuid, entity_id)) = self
                .server
                .lan
                .as_ref()
                .map(|host| (host.hub.guests[gi].uuid, host.hub.guests[gi].entity_id))
            {
                for (inputs, result) in &smithed {
                    self.native_player_item_smithed(uuid, entity_id, inputs, result);
                }
            }
            if gs.kind == StationKind::Smithing && std::mem::take(&mut gs.menu.combined) {
                self.server_level_event_at(1044, pos, 0);
            }
            if let Some(button) = gs.menu.enchant_button.take() {
                self.lan_enchant(gi, &mut gs, button);
            }
            if std::mem::take(&mut gs.menu.enchant_changed) {
                self.lan_enchant_offers(gi, &mut gs);
            }
        }
        let window = gs.window;
        let grindstone = gs.kind == StationKind::Grindstone;
        if let Some(menus) = self
            .server
            .lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            && menus.station.id() == owner
            && menus.station.is_none()
        {
            *menus.station = Some(gs);
        }
        if grindstone {
            self.after_commit(After::StationContent(conn, owner, window));
        } else {
            self.lan_send_station(conn);
        }
        true
    }

    /// `LecternBlock.openScreen` for a guest: `minecraft:lectern` over the
    /// block entity, with its book and page.
    pub(in crate::game) fn lan_open_lectern(&mut self, conn: u32, key: (i32, i32, i32)) {
        if self.guest_may_open(conn, "minecraft:lectern", Some([key.0, key.1, key.2])) != Some(true)
        {
            return;
        }
        self.guest_replace_menu(conn, false, |game| {
            let title = sunrise_protocol::text::translatable("container.lectern", &[]);
            let Some(window) = game
                .server
                .lan
                .as_mut()
                .and_then(|host| host.hub.open_window(conn, "minecraft:lectern", title))
            else {
                return;
            };
            if let Some(host) = game.server.lan.as_mut() {
                host.play.entry(conn).or_default().lectern = Some(GuestLectern {
                    window,
                    state: 0,
                    key,
                    drag: QuickCraftDrag::default(),
                    sent: None,
                });
            }
            game.lan_send_lectern(conn, true);
            game.player_menu_opening(conn);
        })
    }

    /// The lectern's book and page for the client: everything (`all`), or
    /// what changed since it was last sent.
    pub(in crate::game::server_side) fn lan_send_lectern(&mut self, conn: u32, all: bool) {
        let Some((key, window, sent)) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.play.get(&conn))
            .and_then(|p| p.lectern.as_ref())
            .map(|gl| (gl.key, gl.window, &gl.sent))
        else {
            return;
        };
        let (book, page) = match self.common.world.block_entities.get(&key) {
            Some(BlockEntity::Lectern(l)) => (&l.book, l.page),
            _ => (&None, 0),
        };
        if !all
            && sent
                .as_ref()
                .is_some_and(|(old, p)| old == book && *p == page)
        {
            return;
        }
        let page_changed = sent.as_ref().is_none_or(|(_, p)| *p != page);
        let book = book.clone();
        let now = Some((book.clone(), page));
        if let Some(host) = self.server.lan.as_mut()
            && let Some(gi) = host.hub.guests.connection_index(conn)
            && let Some(gl) = host.play.get_mut(&conn).and_then(|p| p.lectern.as_mut())
        {
            gl.sent = now;
            gl.state = (gl.state + 1) & 0x7FFF;
            host.hub.send_menu_snapshot(gi, window, gl.state, &[book]);
            if all || page_changed {
                host.hub.send_to(
                    conn,
                    &C::ContainerSetData(play::ContainerSetData {
                        container: window,
                        id: 0,
                        value: page as i16,
                    }),
                );
            }
        }
    }

    /// Runs `op` on the guest's lectern menu (window `window`) with their
    /// inventory, the block entity takes the book and page back, and the
    /// client gets what changed. False when no such window is open.
    pub(in crate::game::server_side) fn lan_lectern_op(
        &mut self,
        conn: u32,
        window: i32,
        op: impl FnOnce(&mut Menu, &sunrise_sim::menu::Env),
    ) -> bool {
        self.lan_lectern_op_clicked(conn, window, None, op)
    }

    /// `lan_lectern_op` for `op` that makes `click` (see `lan_station_op_clicked`).
    pub(in crate::game::server_side) fn lan_lectern_op_clicked(
        &mut self,
        conn: u32,
        window: i32,
        click: Option<super::lan_pickup::Click>,
        op: impl FnOnce(&mut Menu, &sunrise_sim::menu::Env),
    ) -> bool {
        let Some((key, drag)) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.play.get(&conn))
            .and_then(|p| p.lectern.as_ref())
            .filter(|l| l.window == window)
            .map(|l| (l.key, l.drag.clone()))
        else {
            return false;
        };
        let guest = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().position(|g| g.conn == conn))
            .filter(|&gi| {
                !self
                    .server
                    .lan
                    .as_ref()
                    .is_some_and(|h| h.hub.guests[gi].spectator())
            });
        if let Some(gi) = guest
            && let Some(mut menu) = self.lectern_menu(key)
        {
            menu.set_quick_craft_state(drag);
            // Player.mayBuild: survival and creative.
            let may_build = self
                .server
                .lan
                .as_ref()
                .is_some_and(|host| matches!(host.hub.guests[gi].game_mode, 0 | 1));
            if self
                .guest_menu_op_decided(gi, &mut menu, click, None, |menu, env| {
                    menu.may_build = may_build;
                    op(menu, env);
                })
                .is_some()
            {
                self.lectern_menu_done(key, &menu);
                if let Some(gl) = self
                    .server
                    .lan
                    .as_mut()
                    .and_then(|host| host.play.get_mut(&conn))
                    .and_then(|p| p.lectern.as_mut())
                {
                    gl.drag = menu.quick_craft_state();
                }
            }
        }
        self.lan_send_lectern(conn, true);
        true
    }

    /// A click on the guest's lectern window; false for other windows.
    pub(in crate::game) fn lan_lectern_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        let click = super::lan_pickup::click_of(p);
        self.lan_lectern_op_clicked(conn, p.container, Some(click), |menu, env| {
            let _ = menu.click(env, click.0, click.1, click.2);
        })
    }

    /// Result-slot onTake runs before doClick checks for another result or drops the stack.
    pub(in crate::game::server_side) fn lan_station_finish_take(
        &mut self,
        conn: u32,
        gi: usize,
        pos: [i32; 3],
        menu: &mut Menu,
    ) {
        if let Some(craft) = menu.anvil_craft.take() {
            let pre = mod_api::__private::listened(mod_api::HookId::AnvilCraftPre);
            let post = mod_api::__private::listened(mod_api::HookId::AnvilCraftPost);
            let snapshot = (pre || post).then(|| craft.try_into_api().ok()).flatten();
            let inputs = snapshot
                .as_ref()
                .map(|(left, right, output)| AnvilCraftInputs {
                    left: left.as_ref(),
                    right: right.as_ref(),
                    output,
                });
            let player = (pre || post)
                .then(|| self.server.lan.as_ref()?.hub.guests.get(gi))
                .flatten()
                .map(|g| (g.uuid, g.name.clone()));
            if pre && let (Some((uuid, name)), Some(inputs)) = (&player, &inputs) {
                let mut cancel = false;
                self.native_anvil_craft_pre(*uuid, name, pos, inputs, &mut cancel);
                if cancel {
                    return;
                }
            }
            menu.record_anvil_charge();
            let charge = std::mem::take(&mut menu.level_change);
            if charge != 0 {
                let _ = self.as_guest_at(gi, conn, |g| g.give_levels(charge));
            }
            menu.anvil_take_material(&self.menu_env());
            self.lan_station_deliver_result(gi, StationKind::Anvil, pos, menu);
            menu.anvil_take_input(&self.menu_env());
            self.lan_station_deliver_result(gi, StationKind::Anvil, pos, menu);
            menu.combined = false;
            self.anvil_used(pos, menu.creative);
            if post && let (Some((uuid, name)), Some(inputs)) = (player, inputs) {
                self.native_anvil_craft_post(uuid, &name, pos, &inputs);
            }
        }
        if let Some(take) = menu.grind_take.take() {
            let edits = if mod_api::__private::listened(mod_api::HookId::GrindstoneTake)
                && let Some(guest) = self
                    .server
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guests.get(gi))
            {
                let (uuid, entity_id) = (guest.uuid, guest.entity_id);
                let mut cancel = false;
                let edits = self.api_grindstone_take(uuid, entity_id, pos, &take, &mut cancel);
                if cancel {
                    return;
                }
                edits
            } else {
                let xp = if take.xp >= 0 {
                    take.xp
                } else {
                    let amount = take.vanilla_amount(&self.menu_env());
                    self.journal_level_random();
                    sunrise_sim::stations::grindstone_xp_roll(amount, &mut self.common.world.random)
                        as i32
                };
                GrindstoneTakeEdits::initial(xp)
            };
            self.grindstone_award(pos, edits.xp.max(0) as u32);
            menu.set_slot(&self.menu_env(), 0, edits.new_top);
            self.lan_station_deliver_result(gi, StationKind::Grindstone, pos, menu);
            menu.set_slot(&self.menu_env(), 1, edits.new_bottom);
            self.lan_station_deliver_result(gi, StationKind::Grindstone, pos, menu);
            menu.grindstone_inputs_changed(&self.menu_env());
            self.lan_station_deliver_result(gi, StationKind::Grindstone, pos, menu);
        }
    }

    /// `EnchantmentMenu.slotsChanged` with a level: the costs and clues for
    /// the guest's seed and the bookshelves around the table.
    pub(in crate::game::server_side) fn lan_enchant_offers(
        &mut self,
        gi: usize,
        gs: &mut GuestStation,
    ) {
        let (seed, guest_uuid, guest_entity_id, guest_name) = self
            .server
            .lan
            .as_ref()
            .map(|host| {
                let guest = &host.hub.guests[gi];
                (
                    guest.xp_seed,
                    guest.uuid,
                    guest.entity_id,
                    guest.name.clone(),
                )
            })
            .unwrap_or((0, 0, 0, String::new()));
        let bookcases = self.count_bookshelves(gs.pos);
        let offers = if mod_api::__private::mods().is_empty() {
            sunrise_sim::stations::enchant_offers(
                &self.stations().enchantments,
                self.items(),
                seed,
                bookcases,
                gs.menu.slot(0),
            )
        } else {
            // EnchantmentLevelSetEvent for each row as getEnchantmentCost drew
            // it; the offers are then made with the levels the mods left.
            let mut drawn = [None; 3];
            sunrise_sim::stations::enchant_offers_leveled(
                &self.stations().enchantments,
                self.items(),
                seed,
                bookcases,
                gs.menu.slot(0),
                |row, cost| *drawn[row].insert(cost),
            );
            let mut levels = drawn.map(|cost| cost.unwrap_or(0));
            if let Some(item) = gs
                .menu
                .slot(0)
                .and_then(|item| TryIntoApi::try_into_api(item).ok())
            {
                for (row, original) in drawn.iter().enumerate() {
                    if let Some(original_level) = *original {
                        let offer = mod_api::item::stations::EnchantmentRow {
                            player: guest_uuid,
                            player_entity_id: guest_entity_id,
                            row: row as i32,
                            power: bookcases,
                            item: &item,
                            original_level,
                        };
                        self.native_enchantment_level_set(gs.pos, &offer, &mut levels[row]);
                    }
                }
            }
            sunrise_sim::stations::enchant_offers_leveled(
                &self.stations().enchantments,
                self.items(),
                seed,
                bookcases,
                gs.menu.slot(0),
                |row, _| levels[row],
            )
        };
        let mut costs = offers.costs;
        self.native_prepare_item_enchant(guest_uuid, &guest_name, gs.pos, &mut costs);
        let mut clues = offers.clues;
        for i in 0..3 {
            // A cost a mod changed shows the clue of the list it will give.
            if costs[i] != offers.costs[i]
                && let Some(stack) = gs.menu.slot(0)
            {
                clues[i] = sunrise_sim::stations::enchant_clue(
                    &self.stations().enchantments,
                    self.items(),
                    seed,
                    i,
                    costs[i],
                    stack,
                );
            }
        }
        for i in 0..3 {
            gs.menu.data[i] = costs[i];
            let clue = clues[i].as_ref();
            gs.menu.data[4 + i] = clue
                .and_then(|(e, _)| {
                    self.server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.registry_id("minecraft:enchantment", e))
                })
                .unwrap_or(-1);
            gs.menu.data[7 + i] = clue.map_or(-1, |(_, l)| *l as i32);
        }
        gs.menu.data[3] = seed;
    }

    /// `EnchantmentMenu.clickMenuButton`'s `access.execute`: the item gets
    /// the offer's enchantments, the guest pays its levels and lapis and
    /// gets a new seed, the table sounds, and the offers are made again.
    pub(in crate::game::server_side) fn lan_enchant(
        &mut self,
        gi: usize,
        gs: &mut GuestStation,
        button: usize,
    ) {
        let Some(item) = gs.menu.slot(0).cloned() else {
            return;
        };
        let Some((seed, creative, conn, uuid, entity_id, name)) =
            self.server.lan.as_ref().map(|host| {
                let g = &host.hub.guests[gi];
                (
                    g.xp_seed,
                    g.creative(),
                    g.conn,
                    g.uuid,
                    g.entity_id,
                    g.name.clone(),
                )
            })
        else {
            return;
        };
        let Some(enchanted) = self.table_enchant(seed, button, gs.menu.data[button], &item) else {
            return;
        };
        let enchanted_for_hook = enchanted.clone();
        let need = button as i32 + 1;
        // The enchantments the item got, for PlayerEnchantItemEvent.
        let given = mod_api::__private::listened(mod_api::HookId::PlayerEnchantItem).then(|| {
            sunrise_sim::stations::enchantment_list(
                &self.stations().enchantments,
                self.items(),
                seed,
                button,
                gs.menu.data[button],
                &item,
            )
        });
        // Player.onEnchantmentPerformed, and EnchantmentMenu's awardStat(ENCHANT_ITEM).
        self.as_guest(conn, |game| {
            game.enchantment_performed(need);
            game.award_custom_stat("enchant_item", 1);
            if let Some(given) = &given {
                game.native_player_enchant_item(uuid, entity_id, &enchanted, given);
            }
            // CriteriaTriggers.ENCHANTED_ITEM, with the levels it cost.
            game.trigger_enchanted_item(&enchanted, need);
        });
        let new_seed = self.common.world.random.next_int();
        if let Some(host) = self.server.lan.as_mut() {
            {
                let value = new_seed;
                host.hub.guests.update(gi, |guest| guest.xp_seed = value);
            }
        }
        let env = self.menu_env();
        let lapis = gs.menu.slot(1).cloned().filter(|_| !creative).map(|mut l| {
            l.count = l.count.saturating_sub(need as u32);
            l
        });
        gs.menu.set_slot(&env, 0, Some(enchanted));
        if !creative {
            gs.menu.set_slot(&env, 1, lapis.filter(|l| l.count > 0));
        }
        gs.menu.enchant_changed = false;
        self.lan_enchant_offers(gi, gs);
        let pitch = self.common.world.random.next_float() * 0.1 + 0.9;
        let at = gs.pos.map(|v| f64::from(v) + 0.5);
        self.server_play_sound("minecraft:block.enchantment_table.use", at, 1.0, pitch);
        self.native_enchant_item(uuid, &name, gs.pos, &enchanted_for_hook, need);
    }

    /// `ServerGamePacketListenerImpl.handleRenameItem`: the guest's anvil
    /// takes the name (`AnvilMenu.setItemName`).
    pub(in crate::game) fn lan_station_rename(&mut self, conn: u32, name: &str) {
        let anvil = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.menus.get(&conn))
            .and_then(|p| p.station.as_ref())
            .is_some_and(|s| s.kind == StationKind::Anvil);
        if anvil {
            self.lan_station_op(conn, None, |menu, env| {
                menu.rename(env, name);
            });
        }
    }

    /// The sounds the menu played, at the station (heard by everyone around,
    /// as `level.playSound(null, pos, ...)`), once per tick.
    pub(in crate::game::server_side) fn lan_station_sounds(&mut self, gs: &mut GuestStation) {
        let sounds = std::mem::take(&mut gs.menu.sounds);
        if sounds.is_empty() || gs.last_sound == self.common.ticks {
            return;
        }
        gs.last_sound = self.common.ticks;
        let at = gs.pos.map(|v| f64::from(v) + 0.5);
        // The menu's `lastSoundTime` lets one sound through per tick, however
        // many results a shift-click took.
        self.server_play_sound(sounds[0], at, 1.0, 1.0);
    }

    /// A click on the guest's station window; false for other windows.
    pub(in crate::game) fn lan_station_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        let click = super::lan_pickup::click_of(p);
        self.lan_station_op_clicked(conn, Some(p.container), Some(click), |menu, env| {
            let _ = menu.click(env, click.0, click.1, click.2);
        })
    }

    /// `ServerGamePacketListenerImpl.handleContainerButtonClick`:
    /// `clickMenuButton`, then `broadcastChanges` when the menu took it.
    pub(in crate::game) fn lan_station_button(&mut self, conn: u32, window: i32, id: i32) {
        // Route to the active menu type's listeners. Only a validated `Slots::Block`
        // supplies the optional position and entity backing.
        let mod_menu = self.server.lan.as_ref().and_then(|host| {
            let g = host.hub.guest(conn)?;
            let menu = g.menu.as_ref().filter(|m| m.window == window)?;
            if !mod_api::__private::menu_listens(mod_api::HookId::MenuButton, &menu.registry_id) {
                return None;
            }
            Some((
                menu.registry_id.clone(),
                g.menu_generation,
                match menu.slots {
                    sunrise_sim::lan::menu::Slots::Block(pos) => Some(pos),
                    _ => None,
                },
                !g.spectator() && menu.still_valid(&host.hub.loop_context, &self.common.world, g),
                g.carried.clone(),
                g.uuid,
                g.entity_id,
            ))
        });
        if let Some((menu, menu_generation, backing, valid, mut carried, uuid, entity_id)) =
            mod_menu
        {
            if valid {
                // The clicking player's cursor stack (`clickMenuButton`'s
                // other half). Single player's own player is a guest of
                // its own host too (`lan.rs:165`), so this one path covers
                // both single player and a LAN guest.
                let original_carried = carried.clone();
                let backing =
                    backing.filter(|&pos| self.common.mod_menu_at(pos) == Some(menu.as_ref()));
                let block = backing.map(|(x, y, z)| self.common.data_block_name([x, y, z]));
                let mut accepted = false;
                self.native_menu_button_valid(&menu, block.as_deref(), id, &mut accepted);
                if !accepted {
                    return;
                }
                let context = mod_api::item::menu::MenuButtonContext {
                    player: uuid,
                    player_entity_id: entity_id,
                    pos: backing.map(|(x, y, z)| [x, y, z]),
                    id,
                };
                // As that player: `native_acting_player` and `native_acting_id`
                // answer for the one who pressed.
                self.as_guest(conn, |game| {
                    game.mod_menu_button(&menu, context, block.as_deref(), backing, &mut carried)
                });
                if carried != original_carried
                    && let Some(host) = self.server.lan.as_mut()
                    && let Some(gi) = host.hub.guests.connection_index(conn)
                {
                    let changed = host.hub.guests.update(gi, |g| {
                        if g.menu_generation != menu_generation
                            || !g
                                .menu
                                .as_ref()
                                .is_some_and(|current| current.window == window)
                            || g.carried != original_carried
                        {
                            return false;
                        }
                        g.carried = carried;
                        true
                    });
                    if changed {
                        host.hub.sync_carried(gi, None);
                    }
                }
            }
            return;
        }
        // LecternMenu's page buttons and Take Book.
        if self.lan_lectern_op(conn, window, |menu, env| {
            menu.button(env, id);
        }) {
            return;
        }
        let identity = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.uuid, g.name.clone()));
        let mut selected = false;
        let kind = self
            .server
            .lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            .and_then(|p| p.station.as_mut())
            .filter(|s| s.window == window)
            .map(|s| {
                if s.kind == StationKind::Stonecutter
                    && let Some(sent) = s.sent_data.first_mut()
                {
                    // The guest may have predicted a candidate the real stack cannot use.
                    *sent = None;
                }
                s.kind
            });
        self.lan_station_op(conn, Some(window), |menu, env| {
            selected = menu.button(env, id);
        });
        if selected && let Some((player, name)) = identity {
            match kind {
                Some(StationKind::Loom) => {
                    self.native_player_loom_pattern_select(player, &name, id);
                }
                Some(StationKind::Stonecutter) => {
                    self.native_player_stonecutter_recipe_select(player, &name, id);
                }
                _ => {}
            }
        }
    }

    /// The whole station menu and its changed data slots for the client
    /// (`sendAllDataToRemote` / `broadcastChanges`).
    pub(in crate::game) fn lan_send_station(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let Some(gs) = host.menus.get_mut(&conn).and_then(|p| p.station.as_mut()) else {
            return;
        };
        gs.state = (gs.state + 1) & 0x7FFF;
        let own = gs.menu.slot_count() - 36;
        let mut items: Vec<Option<ItemStack>> =
            (0..own).map(|s| gs.menu.slot(s).cloned()).collect();
        let g = &host.hub.guests[gi];
        items.extend((9..36).chain(0..9).map(|i| g.inventory[i].clone()));
        let state = gs.state;
        // ContainerSetData's value is a short. An anvil cost outside it
        // would change sign (`-65535` as `+1`), so the signed cost follows
        // on `sunrise:anvil_cost` after that short.
        gs.sent_data.resize(gs.menu.data.len(), None);
        let mut data = Vec::new();
        let mut wide_cost = None;
        let anvil = gs.kind == StationKind::Anvil;
        let window = gs.window;
        for (k, &v) in gs.menu.data.iter().enumerate() {
            if gs.sent_data[k] == Some(v) {
                continue;
            }
            gs.sent_data[k] = Some(v);
            let mut value = v as i16;
            if anvil && k == 0 {
                let synced = sunrise_protocol::anvil_cost::sync(v);
                value = synced.short;
                wide_cost = synced.wide;
            }
            data.push(C::ContainerSetData(play::ContainerSetData {
                container: window,
                id: k as i16,
                value,
            }));
        }
        host.hub.send_menu_snapshot(gi, window, state, &items);
        host.hub.guests.update(gi, |g| g.inventory_told());
        for packet in data {
            host.hub.send_to(conn, &packet);
        }
        if let Some(cost) = wide_cost {
            host.hub.send_to(
                conn,
                &C::CustomPayload(sunrise_protocol::packets::configuration::CustomPayload {
                    channel: sunrise_protocol::anvil_cost::CHANNEL.into(),
                    data: sunrise_protocol::anvil_cost::encode(
                        sunrise_protocol::anvil_cost::Cost { window, cost },
                    ),
                }),
            );
        }
    }

    pub(in crate::game) fn lan_station_removed(
        &mut self,
        conn: u32,
        window: Option<i32>,
        removal: super::lan_crafting::MenuRemoval,
    ) -> bool {
        // A lectern's menu keeps nothing of its own (the book is the block entity's).
        let Some(host) = self.server.lan.as_mut() else {
            return false;
        };
        let lectern = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.lectern.take_if(|l| window.is_none_or(|w| l.window == w)));
        if let Some(lectern) = lectern {
            // AbstractContainerMenu.removed: the carried stack goes back.
            if let Some(gi) = host.hub.guests.connection_index(conn)
                && let Some(carried) = self.close_guest_menu_cursor(gi, lectern.window)
            {
                self.lan_place_back(gi, vec![carried]);
            }
            if window.is_some() {
                return true;
            }
        }
        let Some(gs) = self
            .server
            .lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            .and_then(|p| p.station.take_if(|s| window.is_none_or(|w| s.window == w)))
        else {
            return false;
        };
        let Some(host) = self.server.lan.as_mut() else {
            return true;
        };
        let Some(gi) = host.hub.guests.connection_index(conn) else {
            return true;
        };
        let carried = self.close_guest_menu_cursor(gi, gs.window);
        let inputs = station_inputs(&gs);
        if removal.drops_items() {
            self.drop_guest_items(gi, inputs);
        } else if gs.kind == StationKind::Beacon {
            // BeaconMenu.removed drops the payment (player.drop).
            self.lan_place_back(gi, carried.into_iter().collect());
            self.drop_guest_items(gi, inputs);
        } else {
            self.lan_place_back(gi, carried.into_iter().chain(inputs).collect());
        }
        true
    }

    /// `BeaconBlockEntity.dataAccess`: the pyramid's levels and the powers
    /// (`BeaconMenu.encodeEffect`: the `mob_effect` id + 1, 0 for none).
    pub(in crate::game::server_side) fn beacon_data(&self, pos: [i32; 3]) -> [i32; 3] {
        let Some(BlockEntity::Beacon(b)) = self
            .common
            .world
            .block_entities
            .get(&(pos[0], pos[1], pos[2]))
        else {
            return [0; 3];
        };
        let encode = |e: &Option<String>| {
            e.as_deref()
                .and_then(|n| {
                    self.server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.registry_id("minecraft:mob_effect", n))
                })
                .map_or(0, |id| id + 1)
        };
        [b.levels, encode(&b.primary), encode(&b.secondary)]
    }

    /// `ServerGamePacketListenerImpl.handleSetBeaconPacket`: the guest's
    /// beacon menu takes the powers (`BeaconMenu.updateEffects`), and they
    /// become the block entity's (`dataAccess.set`: the power_select sound
    /// while the beam is up).
    pub(in crate::game) fn lan_station_beacon(
        &mut self,
        conn: u32,
        primary: Option<i32>,
        secondary: Option<i32>,
    ) {
        let Some(pos) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.menus.get(&conn))
            .and_then(|p| p.station.as_ref())
            .filter(|s| s.kind == StationKind::Beacon)
            .map(|s| s.pos)
        else {
            return;
        };
        let Some((player, name)) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().find(|g| g.conn == conn))
            .map(|g| (g.uuid, g.name.clone()))
        else {
            return;
        };
        // An id the registry doesn't have fails the packet's codec.
        let mut powers: [Option<(i32, String)>; 2] = [None, None];
        for (k, id) in [primary, secondary].into_iter().enumerate() {
            if let Some(id) = id {
                let Some(name) = self
                    .server
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.registry_name("minecraft:mob_effect", id))
                else {
                    return;
                };
                powers[k] = Some((id, name.to_owned()));
            }
        }
        let data = self.beacon_data(pos);
        let mut updated = None;
        self.lan_station_op(conn, None, |menu, env| {
            menu.data.copy_from_slice(&data);
            let power = |k: usize| powers[k].as_ref().map(|(id, n)| (*id, n.as_str()));
            updated = Some(menu.beacon_update(env, power(0), power(1)));
        });
        // handleSetBeaconPacket: effects the beacon can't give, or no payment, disconnect.
        if updated == Some(false)
            && let Some(host) = self.server.lan.as_mut()
        {
            let name = host
                .hub
                .guests
                .iter()
                .find(|g| g.conn == conn)
                .map(|g| g.name.clone());
            eprintln!(
                "Player {} tried to set invalid beacon effects",
                name.unwrap_or_default()
            );
            host.hub.kick(
                conn,
                sunrise_protocol::text::translatable("multiplayer.disconnect.generic", &[]),
            );
        }
        if updated != Some(true) {
            return;
        }
        // Menu validation accepted these registry ids; convert once at the packet boundary.
        let [primary_key, secondary_key] = powers.each_ref().map(|power| {
            power.as_ref().map(|(_, id)| {
                let def = sunrise_level::brewing::effect(id).expect("validated beacon effect");
                mod_api::__private::effect_key_by_index(def.index)
                    .expect("effects resolved at load")
            })
        });
        let [primary, secondary] = powers.map(|p| p.map(|(_, n)| n));
        let mut select_sound = false;
        if let Some(BlockEntity::Beacon(b)) = self
            .common
            .world
            .block_entities
            .get_mut(&(pos[0], pos[1], pos[2]))
        {
            b.primary = primary.clone();
            b.secondary = secondary.clone();
            select_sound = !b.sections.is_empty();
        }
        self.native_player_change_beacon_effect(player, &name, pos, primary_key, secondary_key);
        if select_sound {
            self.beacon_sound(pos, "minecraft:block.beacon.power_select");
        }
        self.common
            .world
            .modified
            .insert((pos[0] >> 4, pos[2] >> 4));
    }

    /// `ServerPlayer.doTick`'s `stillValid`: a guest's station closes once its
    /// block is gone or the guest is out of reach (`isWithinBlockInteractionRange(pos, 4)`).
    pub(in crate::game) fn lan_station_tick_one(&mut self, conn: u32) {
        self.lan_lectern_tick_one(conn);
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(g) = host.hub.guest(conn).filter(|g| host.hub.here(g)) else {
            return;
        };
        let Some(gs) = host.menus.get(&conn).and_then(|p| p.station.as_ref()) else {
            return;
        };
        let (window, beacon, pos) = (gs.window, gs.kind == StationKind::Beacon, gs.pos);
        let invalid = !self.lan_station_valid(g, gs);
        if invalid {
            self.reply_menu_click(
                conn,
                sunrise_gameplay_common::game::lan_menu_data::ClickReply::Close(window),
            );
        } else if beacon {
            // The beacon's data slots follow its block entity.
            let data = self.beacon_data(pos);
            if let Some(gs) = self
                .server
                .lan
                .as_mut()
                .and_then(|host| host.menus.get_mut(&conn))
                .and_then(|p| p.station.as_mut())
                && gs.kind == StationKind::Beacon
                && gs.menu.data[..] != data[..]
            {
                gs.menu.data.copy_from_slice(&data);
                self.lan_send_station(conn);
            }
        }
    }
}

impl ServerGame<'_> {
    /// The guests' lecterns each tick: `stillValid` (the block entity, its
    /// book, the reach) closes them, and `broadcastChanges` sends a book or
    /// page another player changed.
    fn lan_lectern_tick_one(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some((window, key)) = host
            .play
            .get(&conn)
            .and_then(|p| p.lectern.as_ref())
            .map(|l| (l.window, l.key))
        else {
            return;
        };
        let Some(g) = host.hub.guest(conn).filter(|g| host.hub.here(g)) else {
            return;
        };
        if self.lan_lectern_valid(g, key) {
            self.lan_send_lectern(conn, false);
        } else {
            self.reply_menu_click(
                conn,
                sunrise_gameplay_common::game::lan_menu_data::ClickReply::Close(window),
            );
        }
    }
}

/// What `clearContainer` gives back from a station's menu.
pub(in crate::game) fn station_inputs(gs: &GuestStation) -> Vec<ItemStack> {
    menu_of(gs.kind)
        .map(|(_, _, inputs)| inputs.filter_map(|s| gs.menu.slot(s).cloned()).collect())
        .unwrap_or_default()
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::api_values_common::ApiValuesCommonGameExt1 as _;

use sunrise_gameplay_common::game::bookshelves_common::BookshelvesCommonGameExt1 as _;
