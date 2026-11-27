use super::*;
use sunrise_gameplay_common::game::lan_actions::packets::creative_count_bound;
use sunrise_gameplay_common::game::lan_actions::packets::creative_count_exceeds;
use sunrise_gameplay_common::game::lan_menu_data::ClickReply;

impl crate::game::ServerGame<'_> {
    /// `ServerPlayer.sendBuildLimitMessage`: the red overlay with the limit.
    pub(in crate::game::server_side) fn build_limit_message(
        &self,
        conn: u32,
        too_high: bool,
        limit: i32,
    ) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let message = sunrise_game_data::text::Text::translate(
            if too_high {
                "build.tooHigh"
            } else {
                "build.tooLow"
            },
            &[sunrise_game_data::text::Text::literal(limit.to_string())],
        )
        .with_color(sunrise_game_data::text::RED);
        host.hub.send_to(
            conn,
            &C::SystemChat(play::SystemChat {
                content: sunrise_sim::lan::data::component_nbt(&message.component()),
                overlay: true,
            }),
        );
    }
    /// What ServerboundSetCreativeModeSlotPacket asks of a stack. Its codec
    /// (`ItemStack.validatedStreamCodec`) encodes the stack with `ItemStack.CODEC`, whose count is
    /// 1 to 99: more fails the packet and disconnects the player, as vanilla's decoder error does.
    /// A native mod can raise an item's own stack size past 99, which raises the ceiling instead.
    /// handleSetCreativeModeSlot's `validData` then ignores a stack beyond its own stack size.
    pub(in crate::game::server_side) fn creative_stack_valid(
        &mut self,
        gi: usize,
        s: &ItemStack,
    ) -> bool {
        let size = self.stack_max_size(s);
        let max = creative_count_bound(size);
        let Some(host) = self.server.lan.as_mut() else {
            return false;
        };
        if creative_count_exceeds(s.count, size) {
            let g = &host.hub.guests[gi];
            let conn = g.conn;
            let reason = format!("Invalid packet: count {} is not in 1..={max}", s.count);
            eprintln!("{} sent an invalid creative stack: {reason}", g.name);
            host.hub
                .kick(conn, sunrise_protocol::text::literal(&reason));
            return false;
        }
        s.count <= size
    }
    /// Whether the mods let a creative player set `slot` to `stack`
    /// (`native_creative_slot`), heard with the guest's state swapped in.
    pub(in crate::game::server_side) fn guest_may_set_creative_slot(
        &mut self,
        conn: u32,
        slot: i32,
        stack: Option<&ItemStack>,
    ) -> bool {
        let Some((uuid, name)) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.uuid, g.name.clone()))
        else {
            return true;
        };
        let (item, count) = stack.map_or(("minecraft:air", 0), |s| (&*s.item, s.count as i32));
        let mut cancel = false;
        let mut hook = |game: &mut Self| {
            game.native_creative_slot(uuid, &name, slot, item, count, &mut cancel)
        };
        // The guest whose turn it is is in place already.
        if self.server.lan_acting == Some(conn) {
            hook(self);
        } else {
            self.as_guest(conn, hook);
        }
        !cancel
    }
    /// `ServerGamePacketListenerImpl.tryPickItem` for the guest: a matching stack
    /// is selected, else a creative player gets the item; the held slot is sent
    /// back and the changed slots follow with the tick's inventory changes.
    pub(in crate::game::server_side) fn guest_pick_item(
        &mut self,
        conn: u32,
        from_entity: bool,
        pos: [i32; 3],
        entity_id: i32,
        stack: impl FnOnce(&mut Self) -> Option<ItemStack>,
    ) {
        let Some((uuid, name)) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.uuid, g.name.clone()))
        else {
            return;
        };
        let picked = self.as_guest(conn, |g| {
            let stack = stack(g)?;
            let infinite = g.creative;
            let mut cancel = false;
            g.native_player_pick_item(uuid, &name, from_entity, pos, entity_id, &mut cancel);
            if cancel {
                return None;
            }
            g.pick_into_inventory(&stack, infinite);
            Some(())
        });
        if picked.flatten().is_some()
            && let Some(host) = self.server.lan.as_ref()
            && let Some(g) = host.hub.guests.iter().find(|g| g.conn == conn)
        {
            host.hub.send_to(
                conn,
                &C::SetHeldSlot(play::SetHeldSlot {
                    slot: g.selected as i32,
                }),
            );
        }
    }
    /// `ServerGamePacketListenerImpl.handleTeleportToEntityPacket`: search the live levels in
    /// server order, then teleport to the first matching entity's position and rotation.
    pub(in crate::game::server_side) fn guest_spectate_teleport(&mut self, gi: usize, uuid: u128) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let guest = &host.hub.guests[gi];
        let (conn, from) = (guest.conn, Dim::from_id(&guest.dimension));
        let uuid = sunrise_sim::lan::playerdata::uuid_ints(uuid);
        let Some((dim, target)) = Dim::all().into_iter().find_map(|dim| {
            self.level_target_by_uuid(dim, uuid)
                .map(|target| (dim, target))
        }) else {
            return;
        };
        // Only enter another level to read a found entity's pose, not to probe every level.
        let back = if dim == self.common.dim {
            None
        } else if let Some(back) = self.enter_command_level(dim) {
            Some(back)
        } else {
            return;
        };
        let at = self.target_position(target);
        let [yaw, pitch] = self.target_rotation(target);
        if let Some(back) = back
            && self.enter_level(back).is_err()
        {
            return;
        }
        let Some(at) = at else {
            return;
        };
        // ServerPlayer.teleportTo resets the spectator's camera before traveling.
        self.guest_set_camera(gi, None);
        if from != dim {
            if !self.server_player_travel_allowed(
                conn,
                dim,
                crate::game::server_travel::TravelCause::Teleport,
            ) {
                return;
            }
            self.apply_direct_level_teleport(crate::game::server_travel::LevelTeleport {
                conn,
                dim,
                at,
                rotation: Some([yaw, pitch]),
                vehicle: None,
            });
            return;
        }
        self.as_guest(conn, |game| {
            game.place_player_teleport_in_level(
                Vec3::new(at[0], at[1], at[2]),
                Some([yaw, pitch]),
                [false; 3],
            )
        });
    }

    /// `ServerGamePacketListenerImpl.handleContainerClick` / `AbstractContainerMenu.clicked`
    /// for a mod menu with registered `MenuPart::menu_clicked` callbacks: fires the hook
    /// directly on the receiving menu before default click execution, with no synthetic
    /// decided-action wrapper or whole-inventory snapshot/undo. Returns `true` when this
    /// path consumed the packet, `false` when ordinary routing should run directly.
    fn lan_menu_clicked(&mut self, conn: u32, click: &play::ContainerClick) -> bool {
        if !mod_api::__private::has_part_hook(mod_api::HookId::MenuClicked) {
            return false;
        }
        let Some(guest) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
        else {
            return false;
        };
        if guest.menu.is_none() && click.container == 0 {
            return self.lan_inventory_menu_clicked(conn, click);
        }
        let Some(open) = guest.menu.as_ref().filter(|m| {
            m.window == click.container && (m.mod_type.is_some() || m.slots == Slots::Native)
        }) else {
            return false;
        };
        if guest.spectator() {
            return false;
        }
        if usize::try_from(click.slot).is_ok_and(|slot| slot >= open.slot_count()) {
            return true;
        }
        if !mod_api::__private::menu_listens(mod_api::HookId::MenuClicked, &open.registry_id) {
            return false;
        }
        let (uuid, generation) = (guest.uuid, guest.menu_generation);
        let menu = open.registry_id.clone();
        let mut skip_default = false;
        self.native_menu_clicked(mod_api::__private::MenuClickedCall {
            menu: &menu,
            uuid,
            window: click.container,
            slot: i32::from(click.slot),
            button: i32::from(click.button),
            mode: click.input,
            handled: &mut skip_default,
        });
        let Some(guest) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
        else {
            return true;
        };
        if guest.dead || guest.disconnect_pending || guest.menu_generation != generation {
            return true;
        }
        let Some(open) = guest.menu.as_ref().filter(|m| m.window == click.container) else {
            return true;
        };
        if skip_default {
            // The client predicted the unmodified click. Resend the live menu and cursor so
            // any override or component change reconciles authoritatively.
            self.reply_menu_click(conn, ClickReply::Resend(click.container));
            return true;
        }
        if guest.spectator() {
            self.reply_menu_click(conn, ClickReply::Resend(click.container));
            return true;
        }
        let slot_out_of_range =
            usize::try_from(click.slot).is_ok_and(|slot| slot >= open.slot_count());
        if !self.guest_validate_menu(conn) {
            return true;
        }
        if slot_out_of_range {
            self.reply_menu_click(conn, ClickReply::Resend(click.container));
            return true;
        }
        self.journal_guest_click(conn);
        self.journal_menu_click(conn);
        self.lan_default_container_click(conn, click);
        true
    }

    /// `AbstractContainerMenu.clicked` on the player's own `InventoryMenu` (window 0, no other
    /// menu open) for the Parts attached to `minecraft:inventory`: every mod's, in mod order, since
    /// the inventory has no owner. A handled click resends the inventory; otherwise the server
    /// rechecks that the player is live, still has no other menu open and is not a spectator,
    /// then runs the ordinary click. False when no Part listens or the slot is outside the menu,
    /// leaving the click to ordinary routing.
    fn lan_inventory_menu_clicked(&mut self, conn: u32, click: &play::ContainerClick) -> bool {
        use sunrise_sim::menu::INVENTORY_MENU;
        if !mod_api::__private::menu_listens(mod_api::HookId::MenuClicked, INVENTORY_MENU) {
            return false;
        }
        let Some(guest) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
        else {
            return false;
        };
        let slots = sunrise_sim::menu::Kind::Inventory.slot_count();
        if guest.spectator() || usize::try_from(click.slot).is_ok_and(|slot| slot >= slots) {
            return false;
        }
        let (uuid, generation) = (guest.uuid, guest.menu_generation);
        let mut skip_default = false;
        self.native_menu_clicked(mod_api::__private::MenuClickedCall {
            menu: INVENTORY_MENU,
            uuid,
            window: 0,
            slot: i32::from(click.slot),
            button: i32::from(click.button),
            mode: click.input,
            handled: &mut skip_default,
        });
        let Some(guest) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
        else {
            return true;
        };
        if guest.dead
            || guest.disconnect_pending
            || guest.menu_generation != generation
            || guest.menu.is_some()
        {
            return true;
        }
        if skip_default || guest.spectator() {
            // The client predicted the unmodified click: resend what the inventory holds.
            self.reply_menu_click(conn, ClickReply::Inventory);
            return true;
        }
        self.lan_default_container_click(conn, click);
        true
    }
    pub(in crate::game::server_side) fn item_from_network(
        &self,
        stack: &Option<sunrise_protocol::item::Stack>,
    ) -> Option<ItemStack> {
        let s = stack.as_ref()?;
        let mut out = self.server.lan.as_ref()?.hub.stack_from_network(s);
        out.count = out.count.max(1);
        Some(out)
    }
    /// Ordinary click routing. A scoped override calls this inside its action on Continue;
    /// an unsubscribed click uses it directly. False means the route refused the click.
    pub(in crate::game) fn lan_default_container_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        let Some(gi) = self.guest_index(conn) else {
            return false;
        };
        let guest = &self.server.lan.as_ref().unwrap().hub.guests[gi];
        if let Some(open) = guest.menu.as_ref().filter(|m| m.window == p.container) {
            if open.mod_type.is_some()
                && usize::try_from(p.slot).is_ok_and(|s| s >= open.slot_count())
            {
                return false;
            }
            if open.slots == Slots::Native && open.owned_slots().is_none() && !guest.spectator() {
                // Display menus do not move items unless their slots were explicitly backed.
                if let Some(applied) = self.place_menu_click(
                    guest.uuid,
                    p.container,
                    i32::from(p.slot),
                    i32::from(p.button),
                    p.input,
                ) {
                    return applied;
                }
                self.reply_predicted_menu_click(conn, p, Vec::new());
                return true;
            }
        }
        if let Some(applied) = self
            .lan_owned_menu_click(conn, p)
            .or_else(|| self.lan_mod_quick_move(conn, p, None))
        {
            return applied;
        }
        if self.lan_merchant_click(conn, p)
            || self.lan_inventory_click(conn, p)
            || self.lan_crafting_click(conn, p)
        {
            return true;
        }
        if let Some(applied) = self.lan_machine_click(conn, p) {
            return applied;
        }
        if self.lan_station_click(conn, p)
            || self.lan_lectern_click(conn, p)
            || self.lan_mount_click(conn, p)
        {
            return true;
        }
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        let slots = host.hub.guests[gi]
            .menu
            .as_ref()
            .filter(|m| m.window == p.container)
            .map(|m| m.slots);
        let before = slots.and_then(|s| s.read(&self.common.world, &host.hub.guests[gi]));
        let Some(drops) = self.lan_hub_click(conn, p) else {
            return false;
        };
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        if let Some(Slots::Block(key) | Slots::Double(key, _)) = slots
            && slots.and_then(|s| s.read(&self.common.world, &host.hub.guests[gi])) != before
        {
            self.after_commit(sunrise_gameplay_common::game::transactions::After::Changed(
                key,
            ));
        }
        if let Some(Slots::Block(key)) = slots {
            self.after_commit(
                sunrise_gameplay_common::game::transactions::After::StoreCartItems(key),
            );
        }
        if !drops.is_empty() {
            self.after_commit(
                sunrise_gameplay_common::game::transactions::After::GuestMenu(
                    conn,
                    Box::new(
                        sunrise_gameplay_common::game::lan_menu_data::GuestMenuEffects::dropped(
                            drops,
                        ),
                    ),
                ),
            );
        }
        true
    }
    /// A guest's client started or stopped flying (the hub applied it): the
    /// mods hear it with the guest's state swapped in, and when one cancels,
    /// the guest keeps what they had and is sent their abilities again, as
    /// vanilla answers what it refuses.
    pub(in crate::game) fn lan_flight_toggled(&mut self, conn: u32, flying: bool) {
        let Some((uuid, name)) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.uuid, g.name.clone()))
        else {
            return;
        };
        let mut cancel = false;
        // With the guest's state swapped in, so that the helpers reach them.
        self.as_guest(conn, |g| {
            g.native_player_toggle_flight(uuid, &name, flying, &mut cancel);
        });
        if !cancel {
            return;
        }
        if let Some(host) = self.server.lan.as_mut() {
            let _ = host.hub.update_guest(conn, |g| {
                g.abilities.flying = !flying;
                g.send(&g.abilities_packet());
            });
        }
    }
    /// A packet from a guest that the hub does not handle itself.
    pub fn lan_guest_packet(&mut self, conn: u32, packet: S) {
        // The window's ordinary reply cannot fix a predicted swap in a player slot
        // it doesn't show. Prepare the repair even if menu validation closes the window.
        const OFFHAND_BUTTON: i8 = crate::game::shield::OFFHAND_SLOT as i8;
        let swap = if let S::ContainerClick(click) = &packet
            && click.input == sunrise_sim::lan::menu::input::SWAP
            && matches!(click.button, 0..=8 | OFFHAND_BUTTON)
        {
            self.server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .and_then(|guest| guest.menu.as_ref())
                .filter(|menu| {
                    menu.window == click.container
                        && usize::try_from(click.slot).is_ok_and(|slot| slot < menu.slot_count())
                })
                .and_then(|menu| {
                    if menu.mod_type.is_some() && menu.player_slots() == 0 {
                        Some(ClickReply::Inventory)
                    } else if click.button == OFFHAND_BUTTON
                        && (menu.mod_type.is_some() || menu.slots == Slots::Native)
                    {
                        Some(ClickReply::Offhand)
                    } else {
                        None
                    }
                })
        } else {
            None
        };
        self.lan_guest_packet_inner(conn, packet);
        if let Some(reply) = swap {
            self.reply_menu_click(conn, reply);
        }
    }
    pub(in crate::game::server_side) fn lan_guest_packet_inner(&mut self, conn: u32, packet: S) {
        // A crossing the hub's move check passed; a dead player's is refused, not dropped.
        if let S::CustomPayload(p) = &packet
            && p.channel == sunrise_protocol::passage_sync::CROSS_CHANNEL
        {
            return self.guest_passage_cross(conn, &p.data);
        }
        use super::super::cauldron_prediction::{USE_CHANNEL, UseOnOutcome, use_result};
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        // An authoritative request is an ordinary UseItemOn with a receipt, not
        // permission to use an item or to bypass the vanilla loaded/dead checks.
        let (packet, receipt) = match packet {
            S::CustomPayload(p) if p.channel == USE_CHANNEL => {
                use sunrise_protocol::buf::{Codec, Reader};
                let mut input = Reader::new(&p.data);
                let decoded = (|| -> sunrise_protocol::Result<_> {
                    let receipt = input.i64()?;
                    let packet = play::UseItemOn::read(&mut input)?;
                    if input.remaining() != 0 || packet.sequence < 0 {
                        return Err(sunrise_protocol::Error::new("invalid block use request"));
                    }
                    Ok((receipt, packet))
                })();
                let (receipt, packet) = match decoded {
                    Ok(request) => request,
                    Err(error) => {
                        log::warn!("authoritative block use from guest {conn} refused: {error}");
                        return;
                    }
                };
                let guest = &host.hub.guests[gi];
                if !guest.client_loaded() {
                    host.hub
                        .send_to(conn, &use_result(receipt, UseOnOutcome::default()));
                    return;
                }
                (S::UseItemOn(packet), Some(receipt))
            }
            packet => (packet, None),
        };
        // A dead player acts no more, but the death screen still respawns and chats: its
        // commands run as anyone's (ServerGamePacketListenerImpl.handleChatCommand doesn't
        // look at the player's health).
        if host.hub.guests[gi].dead
            && !matches!(
                packet,
                S::ClientCommand(_)
                    | S::ChatCommand(_)
                    | S::ChatCommandSigned(_)
                    | S::CustomClickAction(_)
            )
        {
            if let Some(receipt) = receipt {
                host.hub
                    .send_to(conn, &use_result(receipt, UseOnOutcome::default()));
            }
            return;
        }
        // The window a packet names is the open one, as in vanilla's handlers. With no menu open
        // this is 0, so a container-0 packet asks the validity of no menu.
        let open_window = |host: &LanHost| {
            super::super::native_menus::open_menu_of(host, conn).map_or(0, |(window, _, _)| window)
        };
        let may_act_in =
            |container| container == open_window(host) && !host.hub.guests[gi].spectator();
        let validate_menu = match &packet {
            S::ContainerClick(click) => {
                // handleContainerClick ignores another window's click. A spectator's
                // click goes straight to the menu's resync path, without a validity ask.
                if click.container != open_window(host) {
                    return;
                }
                !host.hub.guests[gi].spectator()
            }
            // handleContainerButtonClick, handleContainerSlotStateChanged and handlePlaceRecipe
            // act only for the open window and a player who isn't a spectator.
            S::ContainerButtonClick(p) => may_act_in(p.container),
            S::ContainerSlotStateChanged(p) => may_act_in(p.container),
            S::PlaceRecipe(p) => may_act_in(p.container),
            S::RenameItem(_) | S::SetBeacon(_) | S::SelectTrade(_) | S::SelectBundleItem(_) => true,
            _ => false,
        };
        if validate_menu && !self.guest_validate_menu(conn) {
            return;
        }
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let spectator = host.hub.guests[gi].spectator();
        let permission = host.hub.guests[gi].permission;
        match packet {
            S::PlayerAction(a) => match a.action {
                0 => {
                    let Some(host) = self.server.lan.as_mut() else {
                        return;
                    };
                    host.hub.ack(conn, a.sequence);
                    if spectator {
                        return;
                    }
                    let context = &host.hub.loop_context;
                    if !host
                        .hub
                        .guests
                        .update(gi, |g| g.reaches_block(context, a.pos))
                    {
                        return;
                    }
                    // ServerPlayerGameMode.handleBlockBreakAction: no dig starts
                    // under spawn protection (the ack puts the block back).
                    if self.under_spawn_protection(gi, a.pos) {
                        self.spawn_protection_message(conn, a.pos);
                        return;
                    }
                    // mayInteract: a block outside the world border can't be
                    // dug (the ack puts it back on the client).
                    if !self
                        .common
                        .border
                        .contains(f64::from(a.pos[0]), f64::from(a.pos[2]))
                    {
                        return;
                    }
                    let face =
                        sunrise_gameplay_common::game::hit::direction(i32::from(a.direction));
                    // Native mods, once the dig may start: a canceled click
                    // attacks, digs and breaks nothing (the ack puts the
                    // block back on the client).
                    let canceled = self.as_guest(conn, |g| {
                        let state = g.world.block(a.pos[0], a.pos[1], a.pos[2]);
                        let mut canceled = false;
                        g.native_left_click_block(a.pos, face, state, &mut canceled);
                        canceled
                    });
                    if canceled == Some(true) {
                        return;
                    }
                    // ServerPlayerGameMode.handleBlockBreakAction: a creative START
                    // destroys the block at once (destroyAndAck "creative destroy"),
                    // so a UseItemOn sent behind it in the same tick (both buttons
                    // at once, #3006) replaces the air it left. A survival START
                    // attacks the block at once, so a tap released in the same tick
                    // still plays a note. The dig is marked begun for tick_mining.
                    let pos = a.pos;
                    enum StartDig {
                        Creative,
                        Restricted,
                        Air,
                        Accepted,
                    }
                    let start = self.as_guest(conn, |g| {
                        if g.creative {
                            let _ = g.break_block(pos);
                            return StartDig::Creative;
                        }
                        if g.block_action_restricted(pos) {
                            return StartDig::Restricted;
                        }
                        if g.res.facts().is_air(g.world.block(pos[0], pos[1], pos[2])) {
                            g.mining = Some(Mining {
                                pos,
                                progress: 0.0,
                                per_tick: 0.0,
                                ticks: 0,
                            });
                            return StartDig::Air;
                        }
                        let _ = g.attack_block(pos);
                        g.mining = Some(Mining {
                            pos,
                            progress: 0.0,
                            per_tick: 0.0,
                            ticks: 0,
                        });
                        StartDig::Accepted
                    });
                    if matches!(start, Some(StartDig::Accepted | StartDig::Air)) {
                        let Some(host) = self.server.lan.as_mut() else {
                            return;
                        };
                        let play = host.play.entry(conn).or_default();
                        play.digging = Some((pos, face));
                        play.dig_start = host.hub.ticks;
                        play.last_progress = -1;
                        // The pause between blocks is the client's destroyDelay.
                        play.destroy_delay = 0;
                        if matches!(start, Some(StartDig::Air)) {
                            play.destroy_pos = Some(pos);
                        }
                    }
                    // ServerPlayerGameMode.handleBlockBreakAction writes `destroyPos`
                    // only for a dig that takes more than one hit: a creative START
                    // and an instant one leave no block behind for a later STOP.
                    if matches!(start, Some(StartDig::Accepted)) {
                        let one_hit = self
                            .as_guest(conn, |g| {
                                let per_tick = g.destroy_progress(pos);
                                if let Some(mining) = g.mining.as_mut() {
                                    mining.per_tick = per_tick;
                                }
                                per_tick >= 1.0
                            })
                            .unwrap_or(false);
                        if !one_hit
                            && let Some(host) = self.server.lan.as_mut()
                            && let Some(play) = host.play.get_mut(&conn)
                        {
                            play.destroy_pos = Some(pos);
                        }
                        if !one_hit {
                            self.guest_dig_feedback(conn);
                        }
                    }
                }
                1 => {}
                3 => {
                    let Some(host) = self.server.lan.as_mut() else {
                        return;
                    };
                    host.hub.ack(conn, a.sequence);
                    let context = &host.hub.loop_context;
                    if !host
                        .hub
                        .guests
                        .update(gi, |g| g.reaches_block(context, a.pos))
                    {
                        // Too far (handleBlockBreakAction's "too far"): the ack has the client
                        // undo its prediction, and only the dig drops — `destroy_pos` and the
                        // ticks since its START stay, so a STOP for the same block after the
                        // guest comes back into reach counts them (#4124).
                        host.play.entry(conn).or_default().dig_ended();
                        return;
                    }
                    self.guest_stop_digging(conn, a.pos);
                }
                2 => {
                    let Some(host) = self.server.lan.as_mut() else {
                        return;
                    };
                    host.hub.ack(conn, a.sequence);
                    // ServerPlayerGameMode.abortDestroyBlock clears the dig and the crack; the
                    // clock `destroy_pos` names is left to the next START (#4124).
                    host.play.entry(conn).or_default().dig_ended();
                    host.hub
                        .send_authoritative_block(conn, &self.common.world, a.pos);
                    self.guest_crack_progress(conn, a.pos, -1);
                    let Some(host) = self.server.lan.as_ref() else {
                        return;
                    };
                    if mod_api::__private::listened(mod_api::HookId::BlockDamageAbort) {
                        let guest = &host.hub.guests[gi];
                        let (uuid, name) = (guest.uuid, guest.name.clone());
                        self.native_block_damage_abort(uuid, &name, a.pos);
                    }
                }
                4 | 5 => {
                    if !spectator {
                        self.guest_drop_held(gi, conn, a.action == 4);
                    }
                }
                // RELEASE_USE_ITEM: ServerPlayer.releaseUsingItem.
                6 => {
                    self.as_guest(conn, |g| {
                        g.release_using_item();
                    });
                }
                // SWAP_ITEM_WITH_OFFHAND: the hands swap and the item in use stops
                // (ServerGamePacketListenerImpl.handlePlayerAction), not for spectators.
                7 if !spectator => {
                    // The mods hear the swap first (LivingSwapItemsEvent.Hands):
                    // they may stop it, and the client is sent its slots again,
                    // or change what each hand gets. The
                    // stacks are copied only for a listener: `heard` holds the
                    // vanilla (to main hand, to off hand), the old off and main.
                    let (uuid, name, heard) = {
                        let Some(host) = self.server.lan.as_ref() else {
                            return;
                        };
                        let g = &host.hub.guests[gi];
                        let heard = mod_api::__private::listened(mod_api::HookId::PlayerSwapHands)
                            .then(|| (g.offhand.clone(), g.inventory[g.selected].clone()));
                        (g.uuid, g.name.clone(), heard)
                    };
                    // With the player's state swapped in, so that the helpers
                    // reach them.
                    let mut chosen = heard.clone();
                    let cancel = match chosen.as_mut() {
                        Some((to_main, to_off)) => self
                            .as_guest(conn, |g| {
                                let mut cancel = false;
                                g.native_player_swap_hands(
                                    uuid,
                                    &name,
                                    to_main,
                                    to_off,
                                    &mut cancel,
                                );
                                cancel
                            })
                            .unwrap_or(false),
                        None => false,
                    };
                    let Some(host) = self.server.lan.as_mut() else {
                        return;
                    };
                    if cancel {
                        host.hub.send_inventory(gi);
                        return;
                    }
                    let changed = chosen != heard;
                    let used =
                        host.play
                            .get(&conn)
                            .and_then(|play| play.using)
                            .and_then(|(slot, _)| {
                                let guest = &host.hub.guests[gi];
                                if slot == crate::game::shield::OFFHAND_SLOT {
                                    guest.offhand.clone()
                                } else {
                                    guest.inventory[slot].clone()
                                }
                            });
                    let (offhand, main) = host.hub.guests.update(gi, |g| {
                        let slot = g.selected;
                        std::mem::swap(&mut g.inventory[slot], &mut g.offhand);
                        if let Some((to_main, to_off)) = chosen {
                            (g.inventory[slot], g.offhand) = (to_main, to_off);
                        }
                        (g.offhand.clone(), g.inventory[slot].clone())
                    });
                    // A mod's stacks differ from vanilla's swap: both slots go to the client.
                    if changed {
                        host.hub.send_inventory(gi);
                    }
                    // setItemInHand: the off hand, then the main hand, from what each held.
                    let (old_off, old_main) = match &heard {
                        Some((old_off, old_main)) => (old_off.as_ref(), old_main.as_ref()),
                        None => (main.as_ref(), offhand.as_ref()),
                    };
                    self.as_guest(conn, |g| {
                        g.player_on_equip("offhand", old_off, offhand.as_ref());
                        g.player_on_equip("mainhand", old_main, main.as_ref());
                        match used {
                            Some(stack) => g.stop_using_item(&stack),
                            None => g.using = None,
                        }
                    });
                    if mod_api::__private::listened(mod_api::HookId::PlayerSwapWithEquipmentSlot) {
                        let Some(host) = self.server.lan.as_ref() else {
                            return;
                        };
                        let guest = &host.hub.guests[gi];
                        let (uuid, name) = (guest.uuid, guest.name.clone());
                        self.native_player_swap_with_equipment_slot(uuid, &name, "offhand");
                    }
                }
                // STAB: `PiercingWeapon.attack` of a left click with a spear.
                8 if !spectator => self.guest_jab(conn),
                _ => {}
            },
            // handleSetCarriedItem: a new slot stops a main-hand use.
            S::SetCarriedItem(p) => {
                if !(0..9).contains(&p.slot) {
                    let Some(host) = self.server.lan.as_ref() else {
                        return;
                    };
                    eprintln!(
                        "{} tried to set an invalid carried item",
                        host.hub.guests[gi].name
                    );
                    return;
                }
                let (uuid, entity_id, old_slot) = {
                    let Some(host) = self.server.lan.as_ref() else {
                        return;
                    };
                    let g = &host.hub.guests[gi];
                    (g.uuid, g.entity_id, g.selected)
                };
                let changed = old_slot != p.slot as usize;
                if changed {
                    let mut cancel = false;
                    self.native_player_switch_hotbar_slot_pre(
                        uuid,
                        entity_id,
                        old_slot as i32,
                        i32::from(p.slot),
                        &mut cancel,
                    );
                    if cancel {
                        let Some(host) = self.server.lan.as_mut() else {
                            return;
                        };
                        host.hub.send_to(
                            conn,
                            &C::SetHeldSlot(play::SetHeldSlot {
                                slot: old_slot as i32,
                            }),
                        );
                        return;
                    }
                }
                self.as_guest(conn, |g| {
                    if changed
                        && g.using
                            .is_some_and(|(slot, _)| slot != crate::game::shield::OFFHAND_SLOT)
                    {
                        g.stop_active_item_use();
                    }
                });
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                {
                    let value = p.slot as usize;
                    host.hub.guests.update(gi, |guest| guest.selected = value);
                }
                if changed {
                    self.native_player_switch_hotbar_slot_post(
                        uuid,
                        entity_id,
                        old_slot as i32,
                        i32::from(p.slot),
                    );
                }
            }
            // ServerGamePacketListenerImpl.handlePunch: the main hand swings
            // with the held item's attack animation (mining, attacking, a
            // click on nothing), seen by the players around, and the attack
            // strength starts again.
            S::Punch(_) => {
                let (player, name) = {
                    let Some(host) = self.server.lan.as_ref() else {
                        return;
                    };
                    let g = &host.hub.guests[gi];
                    (g.uuid, g.name.clone())
                };
                self.as_guest(conn, |g| {
                    g.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
                    g.reset_attack_strength();
                });
                self.native_player_attack_cooldown_reset(player, &name);
                self.native_player_arm_swing(player, &name, "mainhand");
            }
            S::UseItemOn(p) => {
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                host.hub.ack(conn, p.sequence);
                let hit = Hit::from_use_item_on(&p);
                let hand = if p.hand == 1 { Hand::Off } else { Hand::Main };
                let result = self.guest_use_item_on(conn, gi, hit, hand, receipt);
                if result.is_err()
                    && let Some(receipt) = receipt
                    && let Some(host) = self.server.lan.as_ref()
                {
                    host.hub
                        .send_to(conn, &use_result(receipt, UseOnOutcome::default()));
                }
            }
            S::UseItem(p) => {
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                host.hub.ack(conn, p.sequence);
                let now = host.hub.ticks;
                host.hub.guests.update(gi, |g| g.last_action_tick = now);
                let hand = if p.hand == 1 { Hand::Off } else { Hand::Main };
                let _ = self.guest_use_item(conn, gi, hand, Some((p.yaw, p.pitch)));
            }
            S::Attack(p) => {
                let _ = self.guest_attack(conn, gi, p.id);
            }
            S::Interact(p) => {
                self.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(gi, |g| {
                        g.sneaking = p.secondary;
                    });
                let hand = if p.hand == 1 { Hand::Off } else { Hand::Main };
                let _ = self.guest_interact(conn, gi, p.id, hand, p.location);
            }
            S::MoveVehicle(p) => self.guest_move_vehicle(conn, &p),
            S::PaddleBoat(p) => self.guest_paddle_boat(conn, [p.left, p.right]),
            // handleSetCreativeModeSlot's drop: a slot below 0 is a stack thrown out of the
            // creative screen (`handleCreativeModeItemDrop`), dropped from the player's hand.
            S::SetCreativeModeSlot(p) if p.slot < 0 => {
                let Some(host) = self.server.lan.as_ref() else {
                    return;
                };
                if !host.hub.guests[gi].creative() {
                    return;
                }
                let Some(stack) = self.item_from_network(&p.item) else {
                    return;
                };
                if !self.creative_stack_valid(gi, &stack) {
                    return;
                }
                if !self.guest_may_set_creative_slot(conn, i32::from(p.slot), Some(&stack)) {
                    return;
                }
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                let play = host.play.entry(conn).or_default();
                if play.creative_drops >= 1480 {
                    eprintln!(
                        "Player {} was dropping items too fast in creative mode, ignoring.",
                        host.hub.guests[gi].name
                    );
                    return;
                }
                play.creative_drops += 20;
                self.as_guest(conn, |g| g.player_drop(stack));
            }
            S::SetCreativeModeSlot(p) => {
                // Slot 0 is the crafting result: not a slot to set.
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                let g = &host.hub.guests[gi];
                if !g.creative() || !(1..46).contains(&p.slot) {
                    return;
                }
                let item = self.item_from_network(&p.item);
                if item
                    .as_ref()
                    .is_some_and(|s| !self.creative_stack_valid(gi, s))
                {
                    return;
                }
                if !self.guest_may_set_creative_slot(conn, i32::from(p.slot), item.as_ref()) {
                    // The creative client already holds the stack it sent: treat that as
                    // what it was told, so the next broadcast sends the real slot back.
                    let Some(host) = self.server.lan.as_mut() else {
                        return;
                    };
                    host.hub
                        .guests
                        .update(gi, |g| g.menu_sent[p.slot as usize] = item);
                    return;
                }
                let slot = p.slot as usize;
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                let old = host.hub.guests.update(gi, |g| {
                    let old = g.menu_slot(slot);
                    g.set_menu_slot(slot, item.clone());
                    // InventoryMenu.setRemoteSlot: the client already has it, so broadcastChanges
                    // sends nothing back that could undo a click made since.
                    g.menu_sent[slot] = item.clone();
                    old
                });
                // InventoryMenu's ArmorSlot and off-hand slot: setByPlayer.
                let equipment = match slot {
                    5..=8 => Some(["head", "chest", "legs", "feet"][slot - 5]),
                    45 => Some("offhand"),
                    _ => None,
                };
                if let Some(equipment) = equipment {
                    self.as_guest(conn, |g| {
                        g.player_on_equip(equipment, old.as_ref(), item.as_ref());
                    });
                }
            }
            // ServerboundPlayerCommandPacket OPEN_INVENTORY: the ridden
            // mount's inventory (`HasCustomInventoryScreen`).
            S::PlayerCommand(p) if p.action == 5 => {
                self.as_guest(conn, |g| {
                    // HasCustomInventoryScreen: a tamed mount's horse
                    // inventory, or a chest boat's chest.
                    if !g.server_open_ridden_mount_menu() {
                        g.open_ridden_chest_boat();
                    }
                });
            }
            S::PlayerCommand(p) if p.action == 6 => self.lan_start_gliding(conn),
            // STOP_SLEEPING: `stopSleepInBed(false, true)`.
            S::PlayerCommand(p) if p.action == 0 => self.wake_guest(gi, false, true),
            // START_RIDING_JUMP: the mount's handleStartJump.
            S::PlayerCommand(p) if p.action == 3 => {
                self.lan_start_riding_jump(conn, p.data);
            }
            // STOP_RIDING_JUMP: AbstractHorse and Camel handleStopJump do
            // nothing, and a nautilus' dash is not stopped this way.
            S::PlayerCommand(p) if p.action == 4 => {}
            // ServerboundResourcePackPacket in play: the server has nothing to
            // do with it (a declined required pack is handled on the connection).
            S::ResourcePack(r) => {
                let Some(host) = self.server.lan.as_ref() else {
                    return;
                };
                let guest = &host.hub.guests[gi];
                let (uuid, name) = (guest.uuid, guest.name.clone());
                self.as_guest(conn, |g| {
                    g.native_resource_pack_status(uuid, &name, r.action);
                });
            }
            // START_SPRINTING and STOP_SPRINTING: the hub has set the sprint
            // and sends the packet on when it changed it.
            S::PlayerCommand(p) if matches!(p.action, 1 | 2) => {
                let Some(host) = self.server.lan.as_ref() else {
                    return;
                };
                let guest = &host.hub.guests[gi];
                let (uuid, name) = (guest.uuid, guest.name.clone());
                self.as_guest(conn, |g| {
                    g.native_player_sprint(uuid, &name, p.action == 1);
                });
            }
            S::ContainerButtonClick(p) => {
                self.lan_station_button(conn, p.container, p.button);
            }
            S::RenameItem(p) => self.lan_station_rename(conn, &p.name),
            S::SignUpdate(p) => self.lan_sign_update(conn, p),
            S::SetCommandBlock(p) => self.lan_set_command_block(conn, p),
            S::SetCommandMinecart(p) => self.lan_set_command_minecart(conn, p),
            S::SetStructureBlock(p) => self.lan_set_structure_block(conn, p),
            S::TestInstanceBlockAction(p) => self.lan_test_instance_action(conn, p),
            S::SetTestBlock(p) => self.lan_set_test_block(conn, p),
            S::SetJigsawBlock(p) => self.lan_set_jigsaw_block(conn, p),
            S::JigsawGenerate(p) => self.lan_jigsaw_generate(conn, p),
            S::EditBook(p) => self.lan_edit_book(conn, p),
            S::ContainerSlotStateChanged(p) => {
                self.lan_crafter_slot_state(conn, p.container, p.slot, p.enabled);
            }
            S::SetBeacon(p) => self.lan_station_beacon(conn, p.primary, p.secondary),
            S::SelectTrade(p) => self.lan_select_trade(conn, p.item),
            S::SelectBundleItem(p) => {
                // ServerGamePacketListenerImpl.handleBundleItemSelectedPacket: the slot
                // is the open menu's (#4436).
                let own = self
                    .lan_crafting_table_own_slots(conn)
                    .or_else(|| self.lan_station_own_slots(conn));
                let slot = usize::try_from(p.slot)
                    .ok()
                    .and_then(|slot| bundle_inventory_slot(slot, own));
                if !spectator && let Some(slot) = slot {
                    let Some(host) = self.server.lan.as_mut() else {
                        return;
                    };
                    host.hub.select_bundle_item(
                        conn,
                        slot,
                        p.index,
                        &mut self.common.world,
                        self.common.res.items(),
                    );
                    if mod_api::__private::listened(mod_api::HookId::PlayerBundleItemSelect) {
                        let Some(host) = self.server.lan.as_ref() else {
                            return;
                        };
                        let guest = &host.hub.guests[gi];
                        let (uuid, name) = (guest.uuid, guest.name.clone());
                        self.native_player_bundle_item_select(uuid, &name, p.slot, p.index);
                    }
                }
            }
            S::ContainerClick(p) => {
                if !self.lan_menu_clicked(conn, &p) {
                    self.lan_default_container_click(conn, &p);
                }
            }
            S::ContainerClose(p) => {
                self.reply_menu_click(conn, ClickReply::ClientClose(p.container));
            }
            S::ChatCommand(c) => self.guest_chat_command(conn, &c.command),
            S::CustomPayload(p) if p.channel == sunrise_protocol::quiet::CHANNEL => {
                self.guest_quiet_commands(conn, &p.data)
            }
            S::CustomPayload(p)
                if p.channel == sunrise_sim::recipe_book::settings_sync::CHANNEL =>
            {
                self.guest_book_settings(conn, &p.data)
            }
            S::CustomPayload(p) if p.channel == sunrise_sim::lan::recipes::TRANSFER_CHANNEL => {
                self.guest_recipe_transfer(conn, &p.data);
            }
            S::CustomPayload(p) if p.channel == sunrise_sim::lan::recipes::CHECK_CHANNEL => {
                self.guest_recipe_check(conn, &p.data);
            }
            S::CustomPayload(p) if p.channel == sunrise_sim::perf_net::CHANNEL => {
                self.guest_perf(conn, &p.data)
            }
            S::CustomPayload(p) if p.channel == sunrise_sim::lod_net::CHANNEL => {
                self.guest_far_terrain(conn, &p.data)
            }
            S::ChatCommandSigned(p) => self.guest_signed_command(conn, p),
            // ServerCommonPacketListenerImpl.handleCustomClickAction: the server only logs
            // the click, so a mod's cancel leaves nothing else to skip.
            S::CustomClickAction(p) => {
                let Some(host) = self.server.lan.as_ref() else {
                    return;
                };
                let g = &host.hub.guests[gi];
                let (uuid, entity_id, name) = (g.uuid, g.entity_id, g.name.clone());
                let mut cancel = false;
                self.native_custom_click_action(
                    uuid,
                    entity_id,
                    &name,
                    &p.id,
                    p.payload.as_ref(),
                    &mut cancel,
                );
                if !cancel {
                    log::debug!(
                        "Received custom click action {} with payload {:?}",
                        p.id,
                        p.payload
                    );
                }
            }
            S::ChangeDifficulty(p) if permission >= 2 => {
                if !self.common.level.difficulty_locked && (0..=3).contains(&p.difficulty) {
                    self.common.ui.difficulty = if self.common.level.hardcore {
                        3
                    } else {
                        p.difficulty as u8
                    };
                    // The level data holds the world's difficulty: it
                    // survives the reopen, so it follows the change.
                    self.common.level.set_difficulty(self.common.ui.difficulty);
                    self.update_mob_spawning_flags();
                    self.common.sync_rules();
                    if self.common.ui.difficulty == 0 {
                        self.common.mobs.retain(|m| !m.hostile());
                    }
                    let Some(host) = self.server.lan.as_mut() else {
                        return;
                    };
                    host.hub
                        .broadcast(&C::ChangeDifficulty(play::ChangeDifficulty {
                            difficulty: i32::from(self.common.ui.difficulty),
                            locked: self.common.level.difficulty_locked,
                        }));
                }
            }
            S::LockDifficulty(p) if permission >= 2 => {
                self.common.level.difficulty_locked = p.locked;
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                host.hub
                    .broadcast(&C::ChangeDifficulty(play::ChangeDifficulty {
                        difficulty: i32::from(self.common.ui.difficulty),
                        locked: self.common.level.difficulty_locked,
                    }));
            }
            S::ClientCommand(p) if p.action == 2 && permission >= 2 => {
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                host.hub.send_to(
                    conn,
                    &C::GameRuleValues(play::GameRuleValues {
                        entries: self
                            .common
                            .ui
                            .rules
                            .available_rules()
                            .map(|r| play::GameRuleEntry {
                                name: r.name.clone(),
                                value: self
                                    .common
                                    .ui
                                    .rules
                                    .get(&r.name)
                                    .unwrap_or(r.default)
                                    .to_string(),
                            })
                            .collect(),
                    }),
                );
            }
            S::SetGameRule(p) if permission >= 2 => {
                for entry in p.entries {
                    if self.common.ui.rules.set_string(&entry.name, &entry.value) {
                        if let Some(rule) = self.common.ui.rules.rule(&entry.name).cloned()
                            && let Some(value) = self.common.ui.rules.get(&rule.name)
                        {
                            let Some(host) = self.server.lan.as_ref() else {
                                return;
                            };
                            let guest = &host.hub.guests[gi];
                            let actor = self.common.native_lifecycle.epoch().map(|epoch| {
                                mod_api::__private::game::player_ref(
                                    mod_api::EntityRef::new(epoch, guest.entity_id as u32),
                                    guest.uuid,
                                )
                            });
                            self.game_rule_changed(&rule.name, value, actor);
                        }
                        let message = sunrise_game_data::text::Text::tr(
                            "commands.gamerule.set",
                            &[&entry.name, &entry.value],
                        );
                        let content = sunrise_sim::lan::data::component_nbt(&message.component());
                        let Some(host) = self.server.lan.as_ref() else {
                            return;
                        };
                        for g in &host.hub.guests {
                            if g.permission >= 2 {
                                g.send(&C::SystemChat(play::SystemChat {
                                    content: content.clone(),
                                    overlay: false,
                                }));
                            }
                        }
                    }
                }
                self.common.sync_rules();
            }
            // handleTeleportToEntityPacket: a spectator goes to the player,
            // in whichever level they are.
            S::TeleportToEntity(p) if spectator => self.guest_spectate_teleport(gi, p.uuid),
            S::SpectatorAction(p) => self.guest_spectator_action(gi, p.target),
            S::ChangeGameMode(p) if permission >= 2 => {
                if let Some(mode) = usize::try_from(p.mode)
                    .ok()
                    .and_then(|index| sunrise_command::args::GameMode::ALL.get(index))
                {
                    let lines = self.as_guest(conn, |g| {
                        g.server.chat_capture = Some(Vec::new());
                        g.server_change_game_mode(*mode);
                        g.server
                            .chat_capture
                            .take()
                            .unwrap_or_default()
                            .iter()
                            .map(|t| sunrise_sim::lan::data::component_nbt(&t.component()))
                            .collect::<Vec<_>>()
                    });
                    for content in lines.unwrap_or_default() {
                        let Some(host) = self.server.lan.as_mut() else {
                            return;
                        };
                        host.hub.send_to(
                            conn,
                            &C::SystemChat(play::SystemChat {
                                content,
                                overlay: false,
                            }),
                        );
                    }
                    self.send_admin_feedback();
                }
            }
            S::ChangeGameMode(p) => {
                if let Some(mode) = usize::try_from(p.mode)
                    .ok()
                    .and_then(|index| sunrise_command::args::GameMode::ALL.get(index))
                {
                    let name = sunrise_game_data::text::Text::translate(
                        &format!("gameMode.{}", mode.name()),
                        &[],
                    );
                    let Some(host) = self.server.lan.as_ref() else {
                        return;
                    };
                    eprintln!(
                        "Player {} tried to change game mode to {} without required permissions",
                        host.hub.guests[gi].name,
                        name.plain()
                    );
                }
            }
            // handleEntityTagQuery: a game master's F3+I on an entity of their
            // level gets its Entity.saveWithoutId.
            S::EntityTagQuery(p) if permission >= 2 => {
                if let Some(tag) = self.level_entity_tag(p.entity) {
                    let Some(host) = self.server.lan.as_mut() else {
                        return;
                    };
                    host.hub.send_to(
                        conn,
                        &C::TagQuery(play::TagQuery {
                            transaction: p.transaction,
                            tag,
                        }),
                    );
                }
            }
            // handleBlockEntityTagQuery: BlockEntity.saveWithoutMetadata, or
            // null where there is no block entity.
            S::BlockEntityTagQuery(p) if permission >= 2 => {
                let tag = self
                    .block_entity_tag(p.pos)
                    .unwrap_or(sunrise_core::nbt::Tag::End);
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                host.hub.send_to(
                    conn,
                    &C::TagQuery(play::TagQuery {
                        transaction: p.transaction,
                        tag,
                    }),
                );
            }
            S::Unknown(id, payload)
                if id == sunrise_protocol::ids::play::serverbound::COMMAND_SUGGESTION =>
            {
                // ServerCommandSuggestionsProvider.request, then trySchedule.
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                host.play.entry(conn).or_default().suggestion = Some(payload);
                self.guest_try_suggestions(conn);
            }
            S::PlaceRecipe(p) => self.lan_place_recipe(conn, &p),
            // handleClientCommand: PERFORM_RESPAWN after the poem or on death.
            S::ClientCommand(c) if c.action == 0 => self.guest_respawn(conn),
            // ServerStatsCounter.sendStats.
            S::ClientCommand(c) if c.action == 1 => {
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                let stats = host
                    .play
                    .get_mut(&conn)
                    .map(|p| p.stats.take_dirty())
                    .unwrap_or_default();
                host.hub.send_stats(conn, &stats);
            }
            // handlePickItemFromBlock: a block within the block reach plus 1, its
            // data only with infinite materials.
            S::PickItemFromBlock(p) => {
                self.guest_pick_item(conn, false, p.pos, 0, |g| {
                    // handlePickItemFromBlock: within reach and loaded.
                    if !g.world.loaded(p.pos[0], p.pos[2]) {
                        return None;
                    }
                    let eye = g.eye_position()?;
                    let d2: f64 = (0..3)
                        .map(|a| {
                            let lo = f64::from(p.pos[a]);
                            (eye[a] - eye[a].clamp(lo, lo + 1.0)).powi(2)
                        })
                        .sum();
                    let range = g.reach() + 1.0;
                    let within = d2 < range * range;
                    if !g.within_interaction_range_of(g.server.lan_acting_uuid, p.pos, 1.0, within)
                    {
                        return None;
                    }
                    // The block's item, a part's in its place, then the block
                    // entity's data on whichever won (addBlockDataToItem).
                    let include_data = g.creative && p.include_data;
                    let mut stack = g.acting_clone_stack(p.pos, include_data)?;
                    if include_data {
                        g.add_block_data(p.pos, &mut stack);
                    }
                    Some(stack)
                });
            }
            // handlePickItemFromEntity.
            S::PickItemFromEntity(p) => {
                self.guest_pick_item(conn, true, [0; 3], p.id, |g| {
                    g.within_entity_range(p.id)
                        .then(|| g.pick_result_of(p.id))
                        .flatten()
                });
            }
            _ => {}
        }
    }
    /// `ServerLevel.getEntity(id)`'s `Entity.saveWithoutId`: the player or
    /// other entity of the level in place with that network id.
    pub(in crate::game::server_side) fn level_entity_tag(
        &mut self,
        id: i32,
    ) -> Option<sunrise_core::nbt::Tag> {
        use crate::game::commands::Target;
        let host = self.server.lan.as_ref()?;
        let player = host
            .hub
            .guests
            .iter()
            .find(|g| g.entity_id == id && host.hub.here(g))
            .map(|g| g.conn);
        let mut tag = match player {
            Some(player) => self
                .as_guest(player, |g| g.command_target_nbt(Target::Player))
                .flatten()?,
            None => {
                let target = self.level_entity(id)?;
                self.command_target_nbt(target)?
            }
        };
        if let sunrise_core::nbt::Tag::Compound(c) = &mut tag {
            c.remove(&"id".into());
        }
        Some(tag)
    }
    /// ServerPlayer.sendSpawnProtectionMessage.
    pub(in crate::game::server_side) fn spawn_protection_message(&self, conn: u32, pos: [i32; 3]) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let message = sunrise_game_data::text::Text::translate(
            "build.spawn_protection",
            &[sunrise_game_data::text::Text::literal(format!(
                "{}, {}, {}",
                pos[0], pos[1], pos[2]
            ))],
        )
        .with_color(sunrise_game_data::text::RED);
        host.hub.send_to(
            conn,
            &C::SystemChat(play::SystemChat {
                content: sunrise_sim::lan::data::component_nbt(&message.component()),
                overlay: true,
            }),
        );
    }
    pub fn spawn_protection_for(&self, host: &LanHost, uuid: u128) -> Option<i32> {
        let radius = host.hub.spawn_protection;
        (self.common.is_dedicated_server()
            && radius > 0
            && !host.hub.ops.is_empty()
            && !host.hub.ops.contains_key(&uuid))
        .then_some(radius)
    }

    /// DedicatedServer.isUnderSpawnProtection: no operators, an operator or a
    /// radius of 0 or less is never under it; else a block within the radius
    /// of the world spawn (square, in the Overworld) is.
    pub(in crate::game) fn under_spawn_protection(&self, gi: usize, pos: [i32; 3]) -> bool {
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        let g = &host.hub.guests[gi];
        self.spawn_protection_for(host, g.uuid)
            .is_some_and(|radius| self.common.spawn_protects(radius, &g.dimension, pos))
    }
    /// `ServerGamePacketListenerImpl.wasBlockPlacementAttempt`: a block item,
    /// or a bucket with something in it, that is not cooling down.
    pub(in crate::game::server_side) fn was_block_placement_attempt(&mut self, hand: Hand) -> bool {
        let Some(stack) = self.with_hand(hand, |g| g.held().cloned()) else {
            return false;
        };
        let Some(info) = self.items().get(&stack.item).cloned() else {
            return false;
        };
        (info.block.is_some() || sunrise_items::items::bucket_contents(&stack.item).is_some())
            && !self
                .common
                .cooldowns
                .is_on_cooldown(&sunrise_items::cooldowns::group(
                    &stack,
                    info.use_cooldown.as_ref(),
                ))
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::block_items::BlockItemsGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::pick_entity_common::PickEntityCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_equip::PlayerEquipGameExt1 as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;
