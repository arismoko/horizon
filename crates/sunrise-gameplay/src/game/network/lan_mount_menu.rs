//! The horse inventory for a server's players (`ServerPlayer.openHorseInventory`):
//! shift + use on a tamed horse, donkey, mule, camel, llama or nautilus, or the
//! inventory key while riding one (`ServerboundPlayerCommandPacket`
//! OPEN_INVENTORY), opens `HorseInventoryMenu` over the mount with
//! `ClientboundMountScreenOpenPacket`. Clicks run on the menu engine
//! (`Kind::Mount`) with the mount's saddle, body item and chest copied in and
//! written back, and the slots go to the client after each change, as
//! `broadcastChanges` does.
use super::lan::LanHost;
use sunrise_gameplay_common::game::lan_menu_data::{GuestMenuEffects, MountClickWindow};
use sunrise_gameplay_common::game::transactions::{After, SaveChunkScope};
use sunrise_gameplay_common::mount_rules::{mount_columns, shows_body};
use sunrise_items::items::ItemStack;
use sunrise_protocol::packets::play;
use sunrise_sim::{
    menu::{Kind, Menu, QuickCraftDrag},
    mobs::nautilus,
};

/// A guest's open mount menu.
pub(in crate::game) struct GuestMount {
    pub(in crate::game) window: i32,
    pub(in crate::game) mob: u32,
    pub(in crate::game) columns: usize,
    pub(in crate::game) state: i32,
    /// Persistent quick-craft (drag) state across rebuilt mount menus.
    pub(in crate::game) drag: QuickCraftDrag,
    /// The menu's slots the client was last sent.
    pub(in crate::game) sent: Option<Vec<Option<ItemStack>>>,
}

fn live_guest(window: MountClickWindow, host: &LanHost) -> Option<usize> {
    let gi = host.hub.guests.connection_index(window.conn)?;
    let guest = &host.hub.guests[gi];
    let open = host.play.get(&window.conn)?.mount_window.as_ref()?;
    (guest.menu_generation == window.generation
        && guest.placed_in(window.dim.id())
        && open.window == window.window
        && open.mob == window.mob)
        .then_some(gi)
}

impl crate::game::ServerGame<'_> {
    /// `ServerPlayer.openHorseInventory` for mount `mob`: the open menu
    /// closes, the client gets the mount screen and the menu's slots.
    pub fn lan_open_mount(&mut self, conn: u32, mob: u32) {
        let Some(index) = self.common.mobs.iter().position(|m| m.id == mob) else {
            return;
        };
        let columns = mount_columns(&self.common.mobs[index]);
        if self.guest_may_open(conn, "minecraft:horse_inventory", None) != Some(true) {
            return;
        }
        self.guest_replace_menu(conn, false, |game| {
            let Some(host) = game.server.lan.as_mut() else {
                return;
            };
            let Some(window) = host.hub.open_mount_window(conn, columns as i32, mob as i32) else {
                return;
            };
            host.play.entry(conn).or_default().mount_window = Some(GuestMount {
                window,
                mob,
                columns,
                state: 0,
                drag: QuickCraftDrag::default(),
                sent: None,
            });
            game.lan_send_mount(conn, false);
            game.player_menu_opening(conn);
        })
    }

    /// `HorseInventoryMenu` over mount `index`, with the player's inventory
    /// loaded by the caller.
    pub fn mount_engine_menu(&self, index: usize, columns: usize) -> Menu {
        let env = self.menu_env();
        let mut menu = Menu::new(Kind::Mount(columns));
        menu.set_registry_id(Some("sunrise:mount_inventory"));
        menu.mount = Some(self.common.mobs[index].kind.id.to_string());
        let (saddle, body, items) = self.mount_contents(index);
        menu.load_slot(0, saddle);
        menu.load_slot(1, body.flatten());
        for (k, item) in items.into_iter().take(columns * 3).enumerate() {
            menu.set_slot(&env, 2 + k, item);
        }
        menu
    }

    /// The whole menu when its slots changed or a click requires a full correction.
    /// Cursor-only changes use the hub's carried-item broadcast.
    pub(in crate::game) fn lan_send_mount(&mut self, conn: u32, force: bool) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(gm) = host.play.get(&conn).and_then(|p| p.mount_window.as_ref()) else {
            return;
        };
        let (mob, columns) = (gm.mob, gm.columns);
        let Some(index) = self.common.mobs.iter().position(|m| m.id == mob) else {
            return;
        };
        let Some(gi) = host.hub.guests.connection_index(conn) else {
            return;
        };
        let (saddle, body, items) = self.mount_contents(index);
        let mut slots = vec![saddle, body.flatten()];
        slots.extend(items.into_iter().take(columns * 3));
        slots.extend(
            (9..36)
                .chain(0..9)
                .map(|i| host.hub.guests[gi].inventory[i].clone()),
        );
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(gm) = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.mount_window.as_mut())
        else {
            return;
        };
        if !force && gm.sent.as_ref() == Some(&slots) {
            return;
        }
        gm.state = (gm.state + 1) & 0x7FFF;
        host.hub.send_menu_snapshot(gi, gm.window, gm.state, &slots);
        gm.sent = Some(slots);
    }

    /// A click on the guest's mount window; false for other windows.
    pub(in crate::game) fn lan_mount_click(&mut self, conn: u32, p: &play::ContainerClick) -> bool {
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        let Some(open) = host
            .play
            .get(&conn)
            .and_then(|pl| pl.mount_window.as_ref())
            .filter(|m| m.window == p.container)
        else {
            return false;
        };
        let Some(gi) = host.hub.guests.connection_index(conn) else {
            return true;
        };
        let window = MountClickWindow {
            dim: self.common.dim,
            conn,
            generation: host.hub.guests[gi].menu_generation,
            window: open.window,
            mob: open.mob,
        };
        if !host.hub.guests[gi].spectator()
            && let Some(index) = self.common.mobs.iter().position(|m| m.id == window.mob)
        {
            let columns = open.columns;
            let mut menu = self.mount_engine_menu(index, columns);
            menu.set_quick_craft_state(open.drag.clone());
            let click = super::lan_pickup::click_of(p);
            let _ =
                self.guest_menu_action(gi, &mut menu, Some(click), None, |game, menu, stacked| {
                    let refuse = || {
                        if let Some(answer) = stacked {
                            answer.refused.set(true);
                        }
                        None
                    };
                    let live = game
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| live_guest(window, host));
                    let Some(gi) = live.filter(|_| game.dim == window.dim) else {
                        return refuse();
                    };
                    let Some(index) = game
                        .mobs
                        .iter()
                        .position(|m| m.id == window.mob && m.alive())
                    else {
                        return refuse();
                    };
                    // The decided route already journals this guest. A plain click may also
                    // be part of an enclosing transaction (e.g. the start of a drag).
                    if stacked.is_none() && game.has_open_transaction() {
                        game.journal_guest_click(conn);
                    }
                    let ran = game.guest_menu_edit(gi, menu, stacked, |menu, env| {
                        menu.click(env, click.0, click.1, click.2)
                    });
                    if !matches!(ran, Some(Ok(())))
                        || stacked.is_some_and(|answer| answer.refused.get())
                    {
                        return refuse();
                    }
                    let open = game
                        .server
                        .lan
                        .as_mut()
                        .unwrap()
                        .play
                        .get_mut(&conn)
                        .unwrap()
                        .mount_window
                        .as_mut()
                        .unwrap();
                    let drag = menu.quick_craft_state();
                    if open.drag != drag {
                        let before = std::mem::replace(&mut open.drag, drag);
                        game.journal_mount_drag(window, before);
                    }
                    game.lan_mount_store(index, columns, menu);
                    game.after_commit(After::GuestMenu(
                        conn,
                        Box::new(GuestMenuEffects::take(menu)),
                    ));
                    Some(())
                });
        }
        // Both accepted and refused predictions are reconciled from settled storage.
        self.settle_mount_replies([window]);
        true
    }

    /// The engine menu's mount slots back on mount `index`.
    pub fn lan_mount_store(&mut self, index: usize, columns: usize, menu: &Menu) {
        let body = shows_body(self.common.mobs[index].species()).then(|| menu.slot(1).cloned());
        let items = (0..columns * 3)
            .map(|k| menu.slot(2 + k).cloned())
            .collect();
        self.mount_store(index, menu.slot(0).cloned(), body, items);
    }

    pub(in crate::game) fn restore_mount_drag(
        &mut self,
        window: MountClickWindow,
        drag: QuickCraftDrag,
    ) {
        if let Some(host) = self.server.lan.as_mut()
            && live_guest(window, host).is_some()
        {
            host.play
                .get_mut(&window.conn)
                .unwrap()
                .mount_window
                .as_mut()
                .unwrap()
                .drag = drag;
        }
    }

    /// Resolve the same live window and read its final owner and player slots and cursor.
    pub(in crate::game) fn send_settled_mount_reply(&mut self, window: MountClickWindow) {
        let _ = self.with_effect_level(window.dim, |game| {
            if game
                .server
                .lan
                .as_ref()
                .and_then(|host| live_guest(window, host))
                .is_some()
            {
                game.lan_send_mount(window.conn, true);
            }
        });
    }

    /// `AbstractContainerMenu.removed` of the guest's mount menu (`window`;
    /// None closes it whichever it is): the carried stack goes back to the
    /// inventory. False when no such window is open.
    pub(in crate::game) fn lan_mount_removed(&mut self, conn: u32, window: Option<i32>) -> bool {
        let Some(host) = self.server.lan.as_mut() else {
            return false;
        };
        let Some(mount) = host.play.get_mut(&conn).and_then(|p| {
            p.mount_window
                .take_if(|m| window.is_none_or(|w| m.window == w))
        }) else {
            return false;
        };
        if let Some(gi) = host.hub.guests.connection_index(conn)
            && let Some(carried) = self.close_guest_menu_cursor(gi, mount.window)
        {
            self.lan_place_back(gi, vec![carried]);
        }
        true
    }

    /// Each tick: a mount menu whose mount died or is out of reach closes
    /// (`AbstractMountInventoryMenu.stillValid`: alive, within the entity
    /// interaction range plus 4 of the eyes), and the others follow the
    /// mount's and the inventory's slots. Only the level the player is in
    /// checks its menu (`ServerPlayer.tick`): another level ticking has
    /// none of its mobs (#5331).
    pub(in crate::game) fn tick_lan_mount_menu_one(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some((mob, window)) = host
            .play
            .get(&conn)
            .and_then(|p| p.mount_window.as_ref())
            .map(|m| (m.mob, m.window))
        else {
            return;
        };
        let Some(g) = host.hub.guest(conn).filter(|g| host.hub.here(g)) else {
            return;
        };
        if self.lan_mount_valid(g, mob) {
            self.lan_send_mount(conn, false);
        } else {
            self.reply_menu_click(
                conn,
                sunrise_gameplay_common::game::lan_menu_data::ClickReply::Close(window),
            );
        }
    }
    /// The slots of mount `index` as its menu shows them: the saddle, the
    /// body item (None when the slot isn't active), the chest.
    pub(in crate::game) fn mount_contents(
        &self,
        index: usize,
    ) -> (
        Option<ItemStack>,
        Option<Option<ItemStack>>,
        Vec<Option<ItemStack>>,
    ) {
        let m = &self.common.mobs[index];
        let species = m.species();
        let body = shows_body(species).then(|| m.equipment_stacks()[6].clone());
        let size = mount_columns(m) * 3;
        let mut items: Vec<_> = m.extra.items.iter().take(size).cloned().collect();
        items.resize(size, None);
        (m.saddle_item(), body, items)
    }

    /// The menu's slots back on mount `index` (`body` None: not its slot),
    /// with the equip sound of what was put on (`Equippable.equipSound`).
    pub(in crate::game) fn mount_store(
        &mut self,
        index: usize,
        saddle: Option<ItemStack>,
        body: Option<Option<ItemStack>>,
        items: Vec<Option<ItemStack>>,
    ) {
        let m = &self.common.mobs[index];
        let at = m.center();
        let saddle_put_on = saddle.is_some() && !m.extra.saddled();
        let put_on = if saddle_put_on {
            saddle.as_ref()
        } else {
            body.as_ref()
                .and_then(Option::as_ref)
                .filter(|_| m.extra.body_id().is_none())
        };
        let sound = if saddle_put_on && nautilus::is_nautilus(m.species()) {
            // AbstractNautilus.getEquipSound: its own saddle sounds.
            Some(format!("minecraft:{}", nautilus::saddle_sound(m)))
        } else {
            put_on.map(|s| {
                sunrise_sim::item_sync::equip_sound(s, self.common.res.items())
                    .unwrap_or_else(|| "minecraft:item.armor.equip_generic".into())
            })
        };
        let id = m.id;
        let pos = m.position;
        let saddle_off = m.extra.saddled() && saddle.is_none();
        let body_off = body
            .as_ref()
            .is_some_and(|b| b.is_none() && m.extra.body_id().is_some());
        let body_on = body
            .as_ref()
            .is_some_and(|b| b.is_some() && m.extra.body_id().is_none());
        self.journal_equipment(id);
        let m = &mut self.common.mobs[index];
        m.set_saddle(saddle);
        if let Some(body) = body {
            m.extra.set_body_stack(body);
        }
        if !items.is_empty() {
            self.journal_entity(id);
            let _ = self.edit_entity_slots(id, |slots, _| {
                for (slot, stack) in slots.iter_mut().zip(items) {
                    *slot = stack;
                }
            });
        }
        let chunk = (at[0].floor() as i32 >> 4, at[2].floor() as i32 >> 4);
        if self.has_open_transaction() {
            self.journal_chunk_save(chunk, SaveChunkScope::Transaction);
        } else {
            self.common.world.modified.insert(chunk);
        }
        let equips = saddle_put_on || body_on;
        if equips || saddle_off || body_off {
            self.after_commit(After::MountEquipment {
                dim: self.common.dim,
                id,
                at: [pos.x, pos.y, pos.z],
                sound: sound.map(|sound| (sound, at)),
                equips,
            });
        }
    }
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
