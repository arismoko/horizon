use super::*;

impl crate::game::ServerGame<'_> {
    /// `ContainerEntity.isChestVehicleStillValid`: the chest minecart or chest
    /// boat is still there and within the guest's entity interaction range
    /// plus 4 of its box.
    pub(in crate::game::server_side) fn chest_vehicle_still_valid(
        &self,
        id: i32,
        g: &sunrise_sim::lan::guest::Guest,
    ) -> bool {
        let bounds = self
            .common
            .carts
            .iter()
            .find(|c| c.id as i32 == id)
            .map(|c| c.bounds())
            .or_else(|| {
                self.common
                    .boats
                    .iter()
                    .find(|b| b.id as i32 == id)
                    .map(|b| b.bounds())
            });
        bounds.is_some_and(|b| guest_within_entity_range(g, &b, 4.0))
    }
    /// `stillValid` of the guest's open container: its blocks are still there
    /// and in reach, or its vehicle exists and is in reach. True with none open.
    pub(in crate::game::server_side) fn container_menu_valid(
        &self,
        g: &sunrise_sim::lan::guest::Guest,
    ) -> bool {
        let Some(host) = self.server.lan.as_ref() else {
            return true;
        };
        g.menu.as_ref().is_none_or(|m| {
            m.slots.valid(&self.common.world)
                && m.still_valid(&host.hub.loop_context, &self.common.world, g)
                && !matches!(m.slots, Slots::Block((_, y, id))
                    if y == sunrise_gameplay_common::game::minecarts::CART_KEY_Y
                        && !self.chest_vehicle_still_valid(id, g))
        })
    }
    /// `Player.remove(CHANGED_DIMENSION)` removes both menus without dropping
    /// a cursor that still fits. Overflow belongs in the level the guest left.
    /// Return the retired nonzero window in case the trip fails before sending a respawn.
    pub(in crate::game::server_side) fn guest_close_menus_for_travel(
        &mut self,
        conn: u32,
    ) -> (Option<i32>, super::super::lan_menus::MenuOpenHold) {
        let hold = self.hold_menu_opens(conn);
        let retired = self.guest_remove_menus(conn, MenuRemoval::ChangedDimension, false);
        (retired, hold)
    }
    /// `ContainerOpenersCounter.incrementOpeners` / `ContainerOpenersCounter.decrementOpeners`:
    /// the first opener and the last closer each trigger a sound and game event.
    pub(in crate::game) fn guest_container_opener(
        &mut self,
        conn: u32,
        pos: (i32, i32, i32),
        open: bool,
    ) {
        let acting = self.server.lan_acting == Some(conn);
        let spectator = if acting {
            self.common.spectator()
        } else {
            let Some(guest) = self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
            else {
                return;
            };
            guest.spectator()
        };
        if spectator || !self.container_openers_changed(pos, open) {
            return;
        }
        let state = self.common.world.block(pos.0, pos.1, pos.2);
        let name = &self.common.res.facts().get(state).name;
        let shulker = sunrise_gameplay_common::chests::is_shulker_box(name);
        // Resolve the sound before the game event; its listener can replace the block.
        let sound = self.resolve_container_sound(pos, name, open);
        // ContainerOpenersCounter calls onOpen/onClose before the game event;
        // ShulkerBoxBlockEntity emits its game event before playing its sound.
        if !shulker && let Some((sound, at)) = sound {
            self.play_container_sound(sound, at);
        }
        let event = if open {
            "minecraft:container_open"
        } else {
            "minecraft:container_close"
        };
        let post = |game: &mut Self| {
            game.post_block_game_event(
                event,
                [pos.0, pos.1, pos.2],
                Some(sunrise_sim::mobs::Who::Player),
                None,
            );
        };
        if acting {
            post(self);
        } else {
            self.as_guest(conn, post);
        }
        if shulker && let Some((sound, at)) = sound {
            self.play_container_sound(sound, at);
        }
    }
    pub fn guest_disconnected(&mut self, conn: u32) {
        let hold = self.hold_menu_opens(conn);
        self.guest_remove_menus(conn, MenuRemoval::Disconnected, false);
        self.resume_menu_opens(hold);
    }
    /// Whether the mods let the guest open the menu `kind` (`native_container_opened`),
    /// heard with the guest's state swapped in. Asked before the menu that
    /// was open closes. None if the guest's turn cannot be entered.
    pub(in crate::game) fn guest_may_open(
        &mut self,
        conn: u32,
        kind: &str,
        pos: Option<[i32; 3]>,
    ) -> Option<bool> {
        let guest = self.server.lan.as_ref()?.hub.guest(conn)?;
        if !guest.participates_in_player_simulation() || guest.disconnect_pending {
            return None;
        }
        let (uuid, name) = (guest.uuid, guest.name.clone());
        let hook = |game: &mut Self| {
            let mut cancel = false;
            game.native_container_opened(uuid, &name, kind, pos, &mut cancel);
            !cancel
        };
        // The guest whose turn it is is in place already.
        if self.server.lan_acting == Some(conn) {
            Some(hook(self))
        } else {
            self.as_guest(conn, hook)
        }
    }
    /// `AbstractContainerMenu.stillValid` of whatever menu the guest has open
    /// (a container or vehicle, a trade, the crafting table, a station, a
    /// lectern, a mount's inventory). True when none is open.
    pub(in crate::game::server_side) fn guest_menu_still_valid(&self, conn: u32) -> bool {
        let Some(host) = self.server.lan.as_ref() else {
            return true;
        };
        let Some(g) = host.hub.guest(conn) else {
            return true;
        };
        if !self.container_menu_valid(g) {
            return false;
        }
        let p = host.play.get(&conn);
        let menus = host.menus.get(&conn);
        menus
            .and_then(|m| m.crafting.table.as_ref())
            .is_none_or(|t| self.lan_table_valid(g, t.3))
            && menus
                .and_then(|m| m.station.as_ref())
                .is_none_or(|s| self.lan_station_valid(g, s))
            && p.and_then(|p| p.lectern.as_ref())
                .is_none_or(|l| self.lan_lectern_valid(g, l.key))
            && menus
                .and_then(|m| m.merchant.as_ref())
                .and_then(|m| m.merchant.mob)
                .is_none_or(|id| self.lan_merchant_valid(g, id))
            && p.and_then(|p| p.mount_window.as_ref())
                .is_none_or(|m| self.lan_mount_valid(g, m.mob))
    }
    /// The check before a menu click: a menu that is no longer valid closes
    /// (returning what it held, as `ContainerClose` does) and the client's
    /// window with it, so the click lands on nothing
    /// (`ServerGamePacketListenerImpl.handleContainerClick` and the others
    /// ask `stillValid` first).
    pub(in crate::game::server_side) fn guest_validate_menu(&mut self, conn: u32) -> bool {
        if self.guest_menu_still_valid(conn) {
            return true;
        }
        self.guest_close_menus_and_tell(conn);
        false
    }
    /// `ServerPlayer.tick`'s `stillValid`: a guest's block container closes
    /// once its block is removed or out of reach. Death removes every kind
    /// of open menu through the same cleanup and synchronous close boundary.
    pub(in crate::game) fn lan_container_tick_one(&mut self, conn: u32) {
        let Some(dead) = self.server.lan.as_ref().and_then(|host| {
            let g = host.hub.guest(conn).filter(|g| host.hub.here(g))?;
            let close = (g.dead && super::super::native_menus::open_menu_of(host, conn).is_some())
                || !self.container_menu_valid(g);
            close.then_some(g.dead)
        }) else {
            return;
        };
        self.guest_close_menus(
            conn,
            if dead {
                MenuRemoval::Killed
            } else {
                MenuRemoval::Live
            },
        );
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn lan_containers_tick(&mut self) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let conns: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g) && !self.player_tick_canceled(g.conn))
            .map(|g| g.conn)
            .collect();
        for conn in conns {
            self.lan_container_tick_one(conn);
        }
    }
}
