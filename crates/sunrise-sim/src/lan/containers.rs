//! Hub operations on what guests perceive and do beyond the basics: sounds,
//! level events, animations and block containers with their clicks.
use super::{
    hub::Hub,
    menu::{self, Menu, NativeMenuEvent, Rules, Slots, View},
};
use crate::{
    containers::{BlockEntity, ModContainerInsertRule, ModMenuType},
    items::ItemStack,
    world::World,
};
use std::collections::HashMap;
use sunrise_core::nbt::Tag;
use sunrise_core::random::RandomSource;
use sunrise_protocol::{
    item::{Hashed, Stack},
    packets::{
        configuration::CustomPayload,
        play::{self, Clientbound as C},
    },
    registry::SyncedNames,
};

/// Encode a captured level sound and its broadcast radius without a live hub.
/// Entity-bound events use the entity packet; ordinary level sounds retain
/// their fixed-point coordinates. The caller supplies the synced registry id.
pub fn captured_sound_packet(
    sound: &sunrise_game_data::sound_source::ServerSound,
    sound_id: Option<i32>,
    seed: i64,
) -> (C, f64) {
    let holder = match sound_id {
        Some(id) => (id, None),
        None => (-1, Some((sound.event.clone(), None))),
    };
    let packet = match sound.entity {
        Some(id) => C::SoundEntity(play::SoundEntity {
            sound: holder,
            source: sound.source as i32,
            id: id as i32,
            volume: sound.volume,
            pitch: sound.pitch,
            seed,
        }),
        None => C::Sound(play::Sound {
            sound: holder,
            source: sound.source as i32,
            x: (sound.position[0] * 8.0) as i32,
            y: (sound.position[1] * 8.0) as i32,
            z: (sound.position[2] * 8.0) as i32,
            volume: sound.volume,
            pitch: sound.pitch,
            seed,
        }),
    };
    (packet, captured_sound_range(sound))
}

pub(super) fn captured_sound_range(sound: &sunrise_game_data::sound_source::ServerSound) -> f64 {
    // SoundEvent.getRange multiplies as floats before the broadcast widens it.
    f64::from(if sound.volume > 1.0 {
        16.0 * sound.volume
    } else {
        16.0
    })
}

/// A container click read from the world and not yet run (`Hub::prepare_click`).
pub struct PreparedClick {
    i: usize,
    in_menu: bool,
    source: Option<Slots>,
    shulker: bool,
    drag: Option<(u8, Vec<usize>)>,

    items: Option<Vec<Option<ItemStack>>>,
    container_limits: Vec<Option<u32>>,
    pub(crate) container_insert_rules: Vec<ModContainerInsertRule>,
}

impl PreparedClick {
    /// The declaration's per-slot admission rules, for the gameplay click path.
    pub fn container_insert_rules(&self) -> &[ModContainerInsertRule] {
        &self.container_insert_rules
    }
}

/// Items detached from a closing hub menu that the game returns to the player or drops.
pub struct ClosedHubMenu {
    pub carried: Option<ItemStack>,
    pub processing_inputs: Vec<ItemStack>,
}

impl Hub {
    /// `RemoteSampleLogger.useSample`: a tick-time sample to every player who asked for
    /// `dedicated_server_tick_time` (registry id 0) and passes
    /// `ServerDebugSubscribers.hasRequiredPermissions` (`PlayerList.isOp`, any level).
    pub fn tick_time_subscribers(&self) -> Vec<u32> {
        self.guests
            .iter()
            .filter(|g| {
                g.participates_online()
                    && g.joined()
                    && !g.disconnect_pending
                    && g.debug_subscriptions.contains(&0)
                    && self.ops.contains_key(&g.uuid)
                    && g.sender.is_live()
            })
            .map(|g| g.conn)
            .collect()
    }

    /// One packet to each snapshotted connection, encoded once.
    pub fn send_to_each(&self, conns: &[u32], packet: &C) {
        let body = packet.encode();
        for g in conns.iter().filter_map(|c| self.guest(*c)) {
            g.sender.send(body.clone());
        }
    }

    pub fn send_to(&self, conn: u32, packet: &C) {
        if let Some(g) = self.guest(conn) {
            if let C::RemoveEntities(p) = packet {
                self.send_ordered_removal(g, &p.ids);
            } else {
                g.send(packet);
            }
        }
    }

    /// `ChunkMap.sendToTrackingPlayers`: send to every placed guest that
    /// tracks the entity `id`, and wrapped to the clients viewing the level that do.
    pub fn send_to_tracking(&self, id: i32, packet: &C) {
        let body = packet.encode();
        for g in self
            .guests
            .iter()
            .filter(|g| self.here(g) && g.tracker.is_tracking(id))
        {
            g.sender.send(body.clone());
        }
        self.send_view_trackers(id, packet);
    }

    /// `ServerChunkCache.sendToTrackingPlayersAndSelf`: send to every placed
    /// guest that tracks the entity `id`, and to the guest whose entity it is.
    pub fn send_to_tracking_and_self(&self, id: i32, packet: &C) {
        let body = packet.encode();
        for g in self
            .guests
            .iter()
            .filter(|g| self.here(g) && (g.entity_id == id || g.tracker.is_tracking(id)))
        {
            g.sender.send(body.clone());
        }
        self.send_view_trackers(id, packet);
    }

    /// Send to every placed guest within `range` blocks of `at`
    /// (`PlayerList.broadcast`, which leaves out a player exactly at `range`).
    pub fn broadcast_near(&self, at: [f64; 3], range: f64, packet: &C, except: Option<u32>) {
        let body = packet.encode();
        for g in self
            .guests
            .iter()
            .filter(|g| self.here(g) && Some(g.conn) != except)
        {
            if within_range(at, g.position, range) {
                g.sender.send(body.clone());
            }
        }
    }

    /// ClientboundSoundPacket for a sound event name (an inline event when
    /// the registry does not know it).
    pub fn sound_packet(
        &self,
        name: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        source: i32,
        seed: i64,
    ) -> C {
        let sound = match self.sync.id("minecraft:sound_event", name) {
            Some(id) => (id, None),
            None => (-1, Some((name.to_owned(), None))),
        };
        C::Sound(play::Sound {
            sound,
            source,
            x: (at[0] * 8.0) as i32,
            y: (at[1] * 8.0) as i32,
            z: (at[2] * 8.0) as i32,
            volume,
            pitch,
            seed,
        })
    }
    /// Replace the server sound-seed stream, for a probe that needs its draws known.
    pub fn set_sound_seeds(&self, random: sunrise_core::random::LegacyRandom) {
        *self.sound_seeds.lock().unwrap() = random;
    }
    /// Draw the next seed from the server sound source.
    pub fn next_sound_seed(&self) -> i64 {
        next_sound_seed(&mut *self.sound_seeds.lock().unwrap())
    }

    /// The entity overload of `ServerLevel.playSeededSound`. The emitter stays
    /// an entity id on the wire; clients which do not know it drop the sound.
    pub fn captured_sound(
        &self,
        sound: &sunrise_game_data::sound_source::ServerSound,
        except: Option<u32>,
    ) {
        let except = except.or_else(|| {
            sound.except_entity.and_then(|id| {
                self.guests
                    .iter()
                    .find(|g| g.entity_id as u32 == id)
                    .map(|g| g.conn)
            })
        });
        let seed = sound.seed.unwrap_or_else(|| self.next_sound_seed());
        let (packet, range) = captured_sound_packet(
            sound,
            self.sync.id("minecraft:sound_event", &sound.event),
            seed,
        );
        self.broadcast_near(sound.position, range, &packet, except);
        // Wrapped for the level's viewers too: one bound to an entity for those tracking it.
        let emitter = sound.entity.map(|id| id as i32);
        self.send_viewed_sound(
            &self.config.dimension,
            sound.position,
            range,
            emitter,
            || packet.encode(),
        );
    }

    /// `Level.playSound` at `at` for the players within range in the level in place, and wrapped
    /// for the clients that view it there (`send_viewed_sound`).
    pub fn sound(
        &self,
        name: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        source: i32,
        except: Option<u32>,
    ) {
        // Level.playSound: soundSeedGenerator.nextLong per call, not the tick.
        let seed = self.next_sound_seed();
        let packet = self.sound_packet(name, at, volume, pitch, source, seed);
        let range = f64::from(volume.max(1.0)) * 16.0;
        self.broadcast_near(at, range, &packet, except);
        self.send_viewed_sound(&self.config.dimension, at, range, None, || packet.encode());
    }

    /// ClientboundLevelEventPacket (block break particles and sounds ...); wrapped, to the
    /// clients that view the level and hold its chunk.
    pub fn level_event(&self, kind: i32, pos: [i32; 3], data: i32, except: Option<u32>) {
        let at = [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])];
        let packet = C::LevelEvent(play::LevelEvent {
            kind,
            pos,
            data,
            global: false,
        });
        self.broadcast_near(at, 64.0, &packet, except);
        self.send_viewers_of((pos[0] >> 4, pos[2] >> 4), &packet);
    }

    /// `ServerLevel.globalLevelEvent` for an event in level `level`: the packet
    /// with its global flag for every player on the server
    /// (`PlayerList.getPlayers`), each at a position of their own
    /// (`global_level_event_position`). The level is the event's, not the one in
    /// place: an event queued during one level's pass may be sent during another's.
    pub fn global_level_event(&self, level: &str, kind: i32, pos: [i32; 3], data: i32) {
        for g in self.online_players() {
            g.send(&C::LevelEvent(play::LevelEvent {
                kind,
                pos: global_level_event_position(pos, g.position, g.placed_in(level)),
                data,
                global: true,
            }));
        }
    }

    /// ClientboundBlockEventPacket (`PlayerList.broadcast` within 64 blocks); wrapped, to the
    /// clients that view the level and hold its chunk.
    pub fn block_event(&self, pos: [i32; 3], b0: i32, b1: i32, state: u32) {
        let block = self
            .encoder
            .block_ids
            .get(state as usize)
            .copied()
            .unwrap_or(0);
        let at = [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])];
        let packet = C::BlockEvent(play::BlockEvent {
            pos,
            b0: b0 as u8,
            b1: b1 as u8,
            block,
        });
        self.broadcast_near(at, 64.0, &packet, None);
        self.send_viewers_of((pos[0] >> 4, pos[2] >> 4), &packet);
    }

    /// `ServerLevel.broadcastEntityEvent`: send to the players tracking the
    /// entity and the entity itself (`sendToTrackingPlayersAndSelf`).
    pub fn entity_event(&self, id: i32, event: i8) {
        self.send_to_tracking_and_self(id, &C::EntityEvent(play::EntityEvent { id, event }));
    }

    /// `ServerPlayer.crit` (`magicCrit` when `magic`): send the victim's
    /// animation to the players tracking the attacker and the attacker
    /// themselves (`sendToTrackingPlayersAndSelf`).
    pub fn critical_hit(&self, attacker: i32, target: i32, magic: bool) {
        self.send_to_tracking_and_self(
            attacker,
            &C::Animate(play::Animate {
                id: target,
                action: if magic { 2 } else { 1 },
            }),
        );
    }

    /// `ServerLevel.broadcastDamageEvent`: the clients around show the hurt
    /// entity's flash and hear its hurt sound.
    pub fn damage_event(
        &self,
        id: i32,
        kind: &str,
        cause: Option<i32>,
        direct: Option<i32>,
        at: [f64; 3],
    ) {
        let Some(source_type) = self.sync.id("minecraft:damage_type", kind) else {
            return;
        };
        self.broadcast_near(
            at,
            128.0,
            &C::DamageEvent(play::DamageEvent {
                id,
                source_type,
                cause,
                direct,
                position: None,
            }),
            None,
        );
    }

    pub fn swing(&self, id: i32, hand: i32, at: [f64; 3], except: Option<u32>) {
        self.broadcast_near(
            at,
            128.0,
            &C::SwingAnimation(play::SwingAnimation {
                id,
                hand,
                kind: 1,
                duration: 6,
            }),
            except,
        );
    }

    pub fn take_item(&self, item: i32, collector: i32, count: i32, at: [f64; 3]) {
        self.broadcast_near(
            at,
            64.0,
            &C::TakeItemEntity(play::TakeItemEntity {
                item,
                player: collector,
                amount: count,
            }),
            None,
        );
    }

    /// Open a block container for a guest (ServerPlayer.openMenu). `extra`,
    /// for a mod menu type, goes first in a `sunrise:menu_extra` payload with
    /// the window and `pos` (`openMenu` with an extra-data writer).
    #[allow(clippy::too_many_arguments)]
    pub fn open_container(
        &mut self,
        conn: u32,
        menu_kind: &str,
        title: Tag,
        pos: (i32, i32, i32),
        slots: Slots,
        items: Vec<Option<ItemStack>>,
        extra: Option<Vec<u8>>,
    ) {
        self.open_menu_window(conn, menu_kind, title, Some(pos), slots, items, extra);
    }

    /// Open menu type `menu_kind` for a guest, of a block at `pos` or of none:
    /// its window id, None for an unknown type or guest. A [`Slots::Native`]
    /// menu's items are the guest's `native_menu`, as `open_native_menu` keeps.
    #[allow(clippy::too_many_arguments)]
    pub fn open_menu_window(
        &mut self,
        conn: u32,
        menu_kind: &str,
        title: Tag,
        pos: Option<(i32, i32, i32)>,
        slots: Slots,
        items: Vec<Option<ItemStack>>,
        extra: Option<Vec<u8>>,
    ) -> Option<i32> {
        self.open_menu_window_at(
            conn,
            menu_kind,
            title,
            pos,
            slots,
            items,
            extra,
            crate::containers::mod_menu_type(menu_kind),
        )
    }

    /// Install a window and allocate its next vanilla protocol id (`1..=100`).
    /// Stock displays have no mod menu type, even if a part listens to their clicks.
    #[allow(clippy::too_many_arguments)]
    pub fn open_menu_window_at(
        &mut self,
        conn: u32,
        menu_kind: &str,
        title: Tag,
        pos: Option<(i32, i32, i32)>,
        slots: Slots,
        items: Vec<Option<ItemStack>>,
        extra: Option<Vec<u8>>,
        mod_type: Option<&'static ModMenuType>,
    ) -> Option<i32> {
        let kind = self.registry_id("minecraft:menu", menu_kind)?;
        let i = self.guests.iter().position(|g| g.conn == conn)?;
        let encoded_extra = match extra {
            Some(data) => {
                let next_window = self.guests[i].next_window % 100 + 1;
                let message = sunrise_protocol::menu_extra::Message {
                    window: next_window,
                    pos: pos.map(|p| [p.0, p.1, p.2]),
                    data,
                };
                match sunrise_protocol::menu_extra::encode(&message) {
                    Ok(bytes) => Some(bytes),
                    Err(e) => {
                        log::error!("Sunrise: {menu_kind}'s extra data refused: {e}");
                        return None;
                    }
                }
            }
            None => None,
        };
        self.close_container_at(i);
        let window = self.guests.update(i, |g| {
            let window = g.next_menu_window();
            if slots == Slots::Native {
                g.native_menu = items.clone();
            }
            if let Some(data) = encoded_extra {
                g.send(&C::CustomPayload(CustomPayload {
                    channel: sunrise_protocol::menu_extra::CHANNEL.into(),
                    data,
                }));
            }
            g.send(&C::OpenScreen(play::OpenScreen {
                container: window,
                menu: kind,
                title,
            }));
            let mod_type = mod_type.or_else(|| crate::containers::mod_menu_type(menu_kind));
            let slot_changed_seen = mod_type.map(|t| {
                let mut seen = items.clone();
                seen.resize(t.slots, None);
                seen
            });
            g.menu = Some(Menu {
                window,
                kind,
                registry_id: menu_kind.into(),
                pos,
                slots,
                size: items.len(),
                state: 0,
                sent: Vec::new(),
                drag: None,
                sent_data: Vec::new(),
                reported_unsent_data_len: None,
                blockless_data: Vec::new(),

                mod_type,
                craft: None,
                links: Vec::new(),
                asks_still_valid: super::menu::asks_still_valid(mod_type, menu_kind),
                opening_item: None,
                opener_block: None,
                removed: false,
                slot_changed_seen,
            });
            window
        });
        self.send_menu_content(i, items);
        Some(window)
    }

    /// Opens a native mod's menu of `rows` rows (1 to 6, a chest's
    /// `generic_9xN`) showing `items` (padded to the rows' slots), over the
    /// guest's inventory: no click moves anything, each is a
    /// [`NativeMenuEvent`]. The window's id, or None for an unknown guest.
    pub fn open_native_menu(
        &mut self,
        conn: u32,
        rows: u8,
        title: Tag,
        mut items: Vec<Option<ItemStack>>,
    ) -> Option<i32> {
        let rows = rows.clamp(1, 6);
        items.resize(usize::from(rows) * 9, None);
        self.open_menu_window_at(
            conn,
            &format!("minecraft:generic_9x{rows}"),
            title,
            None,
            Slots::Native,
            items,
            None,
            None,
        )
    }

    /// Replaces what the guest's open native menu `window` shows with `items` (padded to its
    /// slots, extra ones dropped) and sends them at once, as
    /// `AbstractContainerMenu.sendAllDataToRemote`: the screen stays open.
    pub fn set_native_menu_items(
        &mut self,
        conn: u32,
        window: i32,
        mut items: Vec<Option<ItemStack>>,
        send: bool,
    ) -> Result<(), mod_api::MenuItemsRefusal> {
        use mod_api::MenuItemsRefusal;
        let i = self
            .guests
            .iter()
            .position(|g| g.conn == conn)
            .ok_or(MenuItemsRefusal::NoPlayer)?;
        let menu = self.guests[i]
            .menu
            .as_ref()
            .filter(|m| m.slots == Slots::Native && m.window == window)
            .ok_or(MenuItemsRefusal::NotOpen)?;
        if menu.owned_slots().is_some() {
            return Err(MenuItemsRefusal::OwnedSlots);
        }
        items.resize(menu.size, None);
        if send {
            self.guests.update(i, |g| g.native_menu = items.clone());
            self.send_menu_content(i, items);
        } else {
            self.guests.update(i, |g| g.native_menu = items);
        }
        Ok(())
    }

    /// Open a trading window for a guest (`Merchant.openTradingScreen`):
    /// its id, or None when the menu type is unknown.
    pub fn open_merchant_window(&mut self, conn: u32, title: Tag) -> Option<i32> {
        self.open_window(conn, "minecraft:merchant", title)
    }

    /// ServerPlayer.openMenu for a menu the game runs (merchant, crafting
    /// table): the open container closes, a new window id opens this menu.
    pub fn open_window(&mut self, conn: u32, menu: &str, title: Tag) -> Option<i32> {
        let kind = self
            .sync
            .builtin
            .get("minecraft:menu")
            .and_then(|m| m.id(menu))?;
        let i = self.guests.iter().position(|g| g.conn == conn)?;
        self.close_container_at(i);
        Some(self.guests.update(i, |g| {
            let window = g.next_menu_window();
            g.send(&C::OpenScreen(play::OpenScreen {
                container: window,
                menu: kind,
                title,
            }));
            window
        }))
    }

    /// ServerPlayer.openHorseInventory: the open container closes, a new
    /// window id opens mount `entity`'s inventory with its chest's columns
    /// (ClientboundMountScreenOpenPacket).
    pub fn open_mount_window(&mut self, conn: u32, columns: i32, entity: i32) -> Option<i32> {
        let i = self.guests.iter().position(|g| g.conn == conn)?;
        self.close_container_at(i);
        Some(self.guests.update(i, |g| {
            let window = g.next_menu_window();
            g.send(&C::MountScreenOpen(play::MountScreenOpen {
                container: window,
                columns,
                entity,
            }));
            window
        }))
    }

    /// Send the resolved guest's container slots, inventory and cursor without reopening.
    pub fn send_menu_content(&mut self, i: usize, mut items: Vec<Option<ItemStack>>) {
        let Some(menu) = &self.guests[i].menu else {
            return;
        };
        for j in 0..menu.player_slots() {
            items.push(self.guests[i].inventory[player_slot(j)].clone());
        }
        let (window, state) = (menu.window, (menu.state + 1) & 0x7FFF);
        self.send_menu_snapshot(i, window, state, &items);
        self.guests.update(i, |g| {
            let Some(menu) = g.menu.as_mut() else {
                return;
            };
            menu.state = state;
            menu.sent = items;
            if menu.player_slots() != 0 {
                g.inventory_told();
            }
        });
    }

    /// `AbstractContainerMenu.sendAllDataToRemote`: the complete slot snapshot also sets
    /// what the client knows of that menu's cursor, not any other menu's.
    pub fn send_menu_snapshot(
        &mut self,
        i: usize,
        container: i32,
        state: i32,
        items: &[Option<ItemStack>],
    ) {
        let guest = &self.guests[i];
        let carried = match (container, &guest.open_carried_sent) {
            (0, Some(_)) => guest.inactive_inventory_carried.clone(),
            (0, None) => guest.carried.clone(),
            (window, Some((open, _))) if window == *open => guest.carried.clone(),
            _ => return,
        };
        let items = items.iter().map(|s| self.stack(s)).collect();
        let packet = C::ContainerSetContent(play::ContainerSetContent {
            container,
            state,
            items,
            carried: self.stack(&carried),
        });
        self.guests.update(i, |g| {
            g.acknowledge_menu_cursor(container, carried);
            g.send(&packet);
        });
    }

    /// `AbstractContainerMenu.sendAllDataToRemote` for the guest's open
    /// container `window`: every slot, the carried stack, and committed blockless data.
    pub fn resend_menu(&mut self, conn: u32, window: i32, world: &World) {
        let Some(i) = self.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let g = &self.guests[i];
        let Some(items) = g
            .menu
            .as_ref()
            .filter(|m| window != 0 && m.window == window)
            .and_then(|m| m.slots.read(world, g))
        else {
            return;
        };
        self.send_menu_content(i, items);
        self.guests[i].send_menu_data_snapshot();
    }

    fn take_hub_menu(
        &mut self,
        i: usize,
        window: i32,
    ) -> Option<(ClosedHubMenu, Option<NativeMenuEvent>)> {
        self.guests.update(i, |g| {
            let mut menu = g.menu.take_if(|m| m.window == window)?;
            let carried = g.close_menu_cursor(menu.window);
            let processing_inputs = menu.take_processing_inputs(&mut g.native_menu);
            let closed = menu.native_removed(g.uuid, g.menu_generation, &mut g.native_menu, false);
            Some((
                ClosedHubMenu {
                    carried,
                    processing_inputs,
                },
                closed,
            ))
        })
    }

    /// `AbstractContainerMenu.removed` for a guest's active container window without yet
    /// resetting `containerMenu` to the player's inventory menu, so `PlayerContainerEvent.Close`
    /// still sees a live `MenuRef` before [`Self::finish_close_container`] clears it.
    pub fn remove_container_menu(
        &mut self,
        conn: u32,
        window: i32,
    ) -> Option<(ClosedHubMenu, Option<NativeMenuEvent>)> {
        let i = self.guests.connection_index(conn)?;
        self.guests.update(i, |g| {
            let matches = g
                .menu
                .as_ref()
                .is_some_and(|m| m.window == window && !m.removed);
            if !matches {
                return None;
            }
            let carried = g.close_menu_cursor(window);
            let uuid = g.uuid;
            let generation = g.menu_generation;
            let (processing_inputs, closed) = if let Some(menu) = g.menu.as_mut() {
                let processing_inputs = menu.take_processing_inputs(&mut g.native_menu);
                let closed = menu.native_removed(uuid, generation, &mut g.native_menu, true);
                (processing_inputs, closed)
            } else {
                (Vec::new(), None)
            };
            Some((
                ClosedHubMenu {
                    carried,
                    processing_inputs,
                },
                closed,
            ))
        })
    }

    /// Completes `ServerPlayer.doCloseContainer` (`this.containerMenu = this.inventoryMenu`)
    /// after `PlayerContainerEvent.Close`, clearing `g.menu` only if `generation` is still
    /// installed (leaving any replacement opened by a close listener intact).
    pub fn finish_close_container(&mut self, conn: u32, generation: u64) {
        let Some(i) = self.guests.connection_index(conn) else {
            return;
        };
        self.guests.update(i, |g| {
            if g.menu_generation == generation && g.menu.is_some() {
                g.menu = None;
                g.native_menu.clear();
            }
        });
    }

    fn close_container_at(&mut self, i: usize) {
        let Some(window) = self.guests[i].menu.as_ref().map(|menu| menu.window) else {
            return;
        };
        let Some((closed_menu, closed)) = self.take_hub_menu(i, window) else {
            return;
        };
        let guest = &self.guests[i];
        guest.send(&C::ContainerClose(play::ContainerClose {
            container: window,
        }));
        if let Some(item) = closed_menu.carried {
            self.native_menu_events
                .push(NativeMenuEvent::ReturnCarried {
                    conn: guest.conn,
                    item,
                });
        }
        if !closed_menu.processing_inputs.is_empty() {
            self.native_menu_events
                .push(NativeMenuEvent::ReturnProcessingInputs {
                    conn: guest.conn,
                    items: closed_menu.processing_inputs,
                    drop_instead: guest.dead,
                });
        }
        self.native_menu_events.extend(closed);
    }

    pub(super) fn close_container_for(&mut self, i: usize) {
        self.close_container_at(i);
    }

    pub fn close_container(&mut self, conn: u32) {
        if let Some(i) = self.guests.iter().position(|g| g.conn == conn) {
            self.close_container_at(i);
        }
    }

    /// Take cursor returns that a hub-side window close queued, before a guest's
    /// menu replacement, removal or save can run ahead of the queue.
    pub fn take_closed_menu_cursors(&mut self, conn: u32) -> impl Iterator<Item = ItemStack> + '_ {
        self.native_menu_events
            .extract_if(.., move |event| {
                matches!(event, NativeMenuEvent::ReturnCarried { conn: owner, .. } if *owner == conn)
            })
            .filter_map(|event| match event {
                NativeMenuEvent::ReturnCarried { item, .. } => Some(item),
                _ => None,
            })
    }

    /// Take processing-menu input returns queued by a hub-side window close, split into
    /// `(to_return, to_drop)` according to whether the player was dead or disconnecting.
    pub fn take_closed_processing_inputs(&mut self, conn: u32) -> (Vec<ItemStack>, Vec<ItemStack>) {
        let mut to_return = Vec::new();
        let mut to_drop = Vec::new();
        for event in self.native_menu_events.extract_if(.., move |event| {
            matches!(
                event,
                NativeMenuEvent::ReturnProcessingInputs { conn: owner, .. } if *owner == conn
            )
        }) {
            if let NativeMenuEvent::ReturnProcessingInputs {
                items,
                drop_instead,
                ..
            } = event
            {
                if drop_instead {
                    to_drop.extend(items);
                } else {
                    to_return.extend(items);
                }
            }
        }
        (to_return, to_drop)
    }

    /// The client closed its window. Return the closing cursor and any processing inputs to the
    /// game for removal.
    pub fn container_closed(&mut self, conn: u32, window: i32) -> Option<ClosedHubMenu> {
        let i = self.guests.connection_index(conn)?;
        let (closed_menu, closed) = self.take_hub_menu(i, window)?;
        self.native_menu_events.extend(closed);
        Some(closed_menu)
    }

    /// Project one live stack for a click claim. Reuse its admissions for the hash and
    /// the projected bytes for a correction if the client guessed wrong.
    fn stack_against_claim(
        &mut self,
        source: &Option<ItemStack>,
        predicted: &Option<Hashed>,
    ) -> (Option<Stack>, bool) {
        let Some(predicted) = predicted else {
            return (self.stack(source), source.is_none());
        };
        let projected = std::cell::RefCell::new(Vec::new());
        let wire = self.stack_projecting(source, &|effect| {
            projected.borrow_mut().push(effect.clone())
        });
        let projected = projected.into_inner();
        let agreed = wire.as_ref().is_some_and(|wire| {
            let names = SyncedNames {
                builtin: &self.sync.builtin,
                dynamic: &self.sync.dynamic,
            };
            if let Some(data) = source.as_ref().and_then(ItemStack::data) {
                predicted.matches_with_admitted(wire, &names, &|name, bytes| {
                    let matches =
                        |effect: &sunrise_world::component_value::consume_effects::Admitted| {
                            &effect.name == name && effect.wire.as_ref() == bytes
                        };
                    data.find_admitted_consume_effect(&matches)
                        .or_else(|| projected.iter().find(|effect| matches(effect)).cloned())
                })
            } else {
                predicted.matches(wire, &names)
            }
        });
        (wire, agreed)
    }

    /// `AbstractContainerMenu.synchronizeCarriedToRemote`: send only the cursor change.
    /// For a click, compare with the client's hashed prediction instead of the old snapshot.
    pub fn sync_carried(&mut self, i: usize, prediction: Option<&play::ContainerClick>) {
        let guest = &self.guests[i];
        let (window, sent) = guest
            .open_carried_sent
            .as_ref()
            .map_or((0, &guest.inventory_carried_sent), |(window, sent)| {
                (*window, sent)
            });
        match prediction {
            // SetCursorItem has no container id: it always updates the active menu.
            Some(p) if p.container != window => return,
            None if guest.carried == *sent => return,
            _ => {}
        }
        let carried = guest.carried.clone();
        let (item, agreed) = match prediction {
            Some(p) => self.stack_against_claim(&carried, &p.carried),
            None => (self.stack(&carried), false),
        };
        self.guests.update(i, |g| {
            if !agreed {
                g.send(&C::SetCursorItem(play::SetCursorItem { item }));
            }
            g.acknowledge_menu_cursor(window, carried);
        });
    }

    /// The live storage value the server presents in this menu slot.
    pub fn view_slot(&self, i: usize, world: &World, slot: usize) -> Option<ItemStack> {
        let g = &self.guests[i];
        match &g.menu {
            Some(m) if slot < m.size => m.slots.get(world, g, slot),
            Some(m) if slot < m.slot_count() => g
                .inventory
                .get(player_slot(slot - m.size))
                .cloned()
                .flatten(),
            Some(_) => None,
            None => g.menu_slot(slot),
        }
    }

    /// ServerGamePacketListenerImpl.handleContainerClick. Returns the stacks
    /// thrown out of the menu. One call for the three steps the game runs apart when it owns
    /// the click (`prepare_click`, `run_click`, `finish_click`).
    pub fn container_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
        world: &mut World,
        rules: &dyn Rules,
    ) -> Vec<ItemStack> {
        let Some(mut prepared) = self.prepare_click(conn, p, world, true) else {
            return Vec::new();
        };
        let outcome = self.run_click(&mut prepared, p, rules);
        self.finish_click(prepared, p, world, outcome)
    }

    /// The first step of a container click: what the click may do, checked against the world,
    /// and the container's slots read from it. `None` is a click that changes nothing (the
    /// guest is gone, the window is not theirs, or the menu is no longer valid), already answered
    /// where vanilla answers it. The game already checks menu validity at packet admission;
    /// callers that did not check it pass `check_validity`.
    pub fn prepare_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
        world: &World,
        check_validity: bool,
    ) -> Option<PreparedClick> {
        let i = self.guests.iter().position(|g| g.conn == conn)?;
        let in_menu = p.container != 0;
        let window_ok = !in_menu
            || self.guests[i]
                .menu
                .as_ref()
                .is_some_and(|m| m.window == p.container);
        if !window_ok {
            return None;
        }
        if self.guests[i].spectator() {
            // A spectator's click changes nothing, and sendAllDataToRemote
            // takes back what its client predicted.
            self.resend_menu(conn, p.container, world);
            return None;
        }
        let open = self.guests[i].menu.as_ref().filter(|_| in_menu);
        // Reject a target that isn't in the declared layout before forwarding it to a mod.
        if let Some(m) = open
            && m.mod_type.is_some()
            && p.slot >= 0
            && p.slot as usize >= m.slot_count()
        {
            return None;
        }
        // A native display menu has no default slot moves. The game handles native
        // listeners and handler-backed slots before entering this click engine.
        if open.is_some_and(|m| m.slots == Slots::Native) {
            self.answer_click(i, p, world, &[]);
            return None;
        }
        // A menu that is no longer valid takes no clicks.
        if check_validity
            && let Some(m) = self.guests[i].menu.as_ref().filter(|_| in_menu)
            && !m.still_valid(&self.loop_context, world, &self.guests[i])
        {
            return None;
        }
        // AbstractContainerMenu.isValidSlotIndex: -999 or below the menu's slot
        // count. A slot past the menu would reach the inventory beyond its 36 slots.
        let slots = match self.guests[i].menu.as_ref().filter(|_| in_menu) {
            Some(m) => m.slot_count(),
            None => 46,
        };
        if i32::from(p.slot) >= slots as i32 && p.slot != -999 {
            return None;
        }
        let source = self.guests[i].menu.as_ref().map(|m| m.slots);
        let shulker_box = self
            .sync
            .builtin
            .get("minecraft:menu")
            .and_then(|m| m.id("minecraft:shulker_box"));
        let shulker = in_menu
            && shulker_box.is_some()
            && self.guests[i].menu.as_ref().map(|m| m.kind) == shulker_box;
        let (drag, items, container_limits, container_insert_rules) = {
            let guest = &self.guests[i];
            let drag = if in_menu {
                guest.menu.as_ref().and_then(|m| m.drag.clone())
            } else {
                guest.drag.clone()
            };
            let items = source.and_then(|s| s.read(world, guest));
            if in_menu && items.is_none() {
                return None;
            }
            let container_limits = source
                .map(|source| source.stack_limits(world, items.as_ref().map_or(0, Vec::len)))
                .unwrap_or_default();
            let mut container_insert_rules = source
                .map(|source| {
                    source.container_insert_rules(world, items.as_ref().map_or(0, Vec::len))
                })
                .unwrap_or_default();
            if let Some(range) = guest
                .menu
                .as_ref()
                .filter(|_| in_menu)
                .and_then(Menu::linked_slot_range)
            {
                for slot in range {
                    if let Some(rule) = container_insert_rules.get_mut(slot) {
                        *rule = ModContainerInsertRule::Any;
                    }
                }
            }
            (drag, items, container_limits, container_insert_rules)
        };
        Some(PreparedClick {
            i,
            in_menu,
            source,
            shulker,
            drag,

            items,
            container_limits,
            container_insert_rules,
        })
    }

    /// The second step: the click itself, on the guest and the container's slots the first step
    /// read. It takes no world and no game, so the caller may lend the menu code anything that
    /// does not alias the hub. Returns the stacks thrown out and the slots that changed.
    pub fn run_click(
        &mut self,
        prepared: &mut PreparedClick,
        p: &play::ContainerClick,
        rules: &dyn Rules,
    ) -> (Vec<ItemStack>, Vec<usize>) {
        let loop_context = &self.loop_context;
        self.guests.update(prepared.i, |guest| {
            let mut view = View {
                guest,
                container: prepared.items.as_deref_mut(),
                shulker: prepared.shulker,
                container_limits: std::mem::take(&mut prepared.container_limits),

                loop_context,
            };
            menu::click(
                &mut view,
                &mut prepared.drag,
                i32::from(p.slot),
                i32::from(p.button),
                p.input,
                rules,
            )
        })
    }

    /// The cursor and the stack in menu slot `slot` of guest `conn`'s open menu, as a click on it
    /// finds them now: what a decision compares with the stacks it froze.
    pub fn click_stacks(
        &self,
        conn: u32,
        slot: usize,
        world: &World,
    ) -> Option<(Option<ItemStack>, Option<ItemStack>)> {
        let i = self.guests.iter().position(|g| g.conn == conn)?;
        Some((
            self.guests[i].carried.clone(),
            self.view_slot(i, world, slot),
        ))
    }

    /// The resolved pickup click `p` as `item_stacked_on_other` is asked about it, on the
    /// container the first step read: the stacks it finds.
    ///
    /// Resolves both a direct left/right `PICKUP` when no drag is active (`prepared.drag.is_none()`)
    /// and a one-slot left/right `QUICK_CRAFT` drag release (`stage == 2`, `slots.len() == 1`,
    /// `kind <= 1`, carried stack present), matching vanilla 26.3 `AbstractContainerMenu.doClick`.
    /// Returns `None` for direct pickups arriving during an active drag (which only reset the drag),
    /// multi-slot drags, middle-button drags (`kind == 2`), and other inputs.
    pub fn stacked_click(
        &mut self,
        prepared: &mut PreparedClick,
        p: &play::ContainerClick,
    ) -> Option<crate::menu::StackedClick> {
        let (slot, button) = match p.input {
            menu::input::PICKUP => {
                if prepared.drag.is_some() {
                    return None;
                }
                (i32::from(p.slot), i32::from(p.button))
            }
            menu::input::QUICK_CRAFT => {
                let stage = i32::from(p.button) & 3;
                if stage != 2 {
                    return None;
                }
                let (kind, slots) = prepared.drag.as_ref()?;
                if slots.len() != 1 || *kind > 1 {
                    return None;
                }
                self.guests[prepared.i].carried.as_ref()?;
                (slots[0] as i32, i32::from(*kind))
            }
            _ => return None,
        };
        let loop_context = &self.loop_context;
        self.guests.update(prepared.i, |guest| {
            View {
                guest,
                container: prepared.items.as_deref_mut(),
                shulker: prepared.shulker,
                container_limits: Vec::new(),

                loop_context,
            }
            .stacked_click(slot, button)
        })
    }

    /// Probe the shared vanilla carried override (`vanilla_carried_override`) on the prepared hub
    /// menu state with `(carried, clicked)` staged into the cursor and menu `slot`, restoring all
    /// state before returning whether the carried item's vanilla override handled the click.
    pub fn probe_carried_vanilla_override(
        &mut self,
        prepared: &mut PreparedClick,
        slot: usize,
        primary: bool,
        carried: Option<ItemStack>,
        clicked: Option<ItemStack>,
        rules: &dyn Rules,
    ) -> bool {
        let loop_context = &self.loop_context;
        self.guests.update(prepared.i, |guest| {
            let kept = (
                guest.inventory.clone(),
                guest.armor.clone(),
                guest.offhand.clone(),
                guest.carried.clone(),
                guest.craft_grid.clone(),
                guest.craft_result.clone(),
            );
            let items = prepared.items.clone();
            let mut view = View {
                guest,
                container: prepared.items.as_deref_mut(),
                shulker: prepared.shulker,
                container_limits: prepared.container_limits.clone(),
                loop_context,
            };
            let took = menu::probe_carried_vanilla_override(
                &mut view, slot, primary, carried, clicked, rules,
            );
            (guest.inventory, guest.armor, guest.offhand) = (kept.0, kept.1, kept.2);
            (guest.carried, guest.craft_grid, guest.craft_result) = (kept.3, kept.4, kept.5);
            prepared.items = items;
            took
        })
    }

    /// The last step: the container's slots back into the world, the unfinished drag back on the
    /// menu, and the client told what it got wrong. Returns the stacks the click threw out.
    pub fn finish_click(
        &mut self,
        prepared: PreparedClick,
        p: &play::ContainerClick,
        world: &mut World,
        (drops, changed): (Vec<ItemStack>, Vec<usize>),
    ) -> Vec<ItemStack> {
        let i = prepared.i;
        self.write_click(prepared, world);
        self.answer_click(i, p, world, &changed);
        drops
    }

    /// Keep a prepared click's slots and drag without yet replying to the guest.
    pub fn write_click(&mut self, prepared: PreparedClick, world: &mut World) {
        let PreparedClick {
            i,
            in_menu,
            source,
            drag,
            items,
            ..
        } = prepared;
        self.guests.update(i, |guest| {
            if let (Some(source), Some(items)) = (source, items) {
                source.write(world, guest, items);
            }
            if in_menu {
                if let Some(m) = guest.menu.as_mut() {
                    m.drag = drag;
                }
            } else {
                guest.drag = drag;
            }
        });
    }

    /// `AbstractContainerMenu.setSelectedBundleItemIndex` on the guest's open
    /// menu: a container's own slots, or the inventory's. A bundle's selection
    /// lives on the server's stack until the click that takes from it.
    pub fn select_bundle_item(
        &mut self,
        conn: u32,
        slot: usize,
        index: i32,
        world: &mut World,
        catalog: &crate::items::Items,
    ) {
        let _ = self.update_guest(conn, |g| {
            let open = g.menu.as_ref().map(|m| (m.size, m.slot_count(), m.slots));
            let mut items = match open {
                Some((size, _, source)) if slot < size => source.read(world, g),
                _ => None,
            };
            let stack = match open {
                Some((size, _, _)) if slot < size => items
                    .as_mut()
                    .and_then(|items| items.get_mut(slot))
                    .and_then(Option::as_mut),
                // setSelectedBundleItemIndex: a slot of the menu only.
                Some((size, total, _)) if slot < total => g
                    .inventory
                    .get_mut(player_slot(slot - size))
                    .and_then(Option::as_mut),
                Some(_) => None,
                None => match slot {
                    9..=35 => g.inventory.get_mut(slot).and_then(Option::as_mut),
                    36..=44 => g.inventory.get_mut(slot - 36).and_then(Option::as_mut),
                    45 => g.offhand.as_mut(),
                    _ => None,
                },
            };
            if let Some(stack) = stack {
                crate::bundle::toggle_selected(catalog, stack, index);
            }
            if let (Some((_, _, source)), Some(items)) = (open, items) {
                source.write(world, g, items);
            }
        });
    }

    /// Reconcile a click the game applied outside the hub's click engine.
    pub fn inventory_clicked(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
        world: &World,
        changed: &[usize],
    ) {
        if let Some(i) = self.guests.iter().position(|g| g.conn == conn) {
            self.answer_click(i, p, world, changed);
        }
    }

    /// `AbstractContainerMenu.broadcastChanges` after a click on the current state.
    /// The client's predictions replace the last sent values only for the slots it claimed.
    pub fn sync_click_prediction(
        &mut self,
        i: usize,
        p: &play::ContainerClick,
        state: &mut i32,
        items: &[Option<ItemStack>],
        slot_changed: impl Fn(usize) -> bool,
    ) -> Vec<usize> {
        let claimed: HashMap<_, _> = p.changed.iter().map(|(s, h)| (*s as usize, h)).collect();
        let mut corrected = Vec::new();
        for (slot, item) in items.iter().enumerate() {
            let net = match claimed.get(&slot) {
                Some(predicted) => {
                    let (net, agreed) = self.stack_against_claim(item, predicted);
                    if agreed {
                        continue;
                    }
                    net
                }
                None if slot_changed(slot) => self.stack(item),
                None => continue,
            };
            *state = (*state + 1) & 0x7FFF;
            self.guests[i].send(&C::ContainerSetSlot(play::ContainerSetSlot {
                container: p.container,
                state: *state,
                slot: slot as i16,
                item: net,
            }));
            corrected.push(slot);
        }
        self.sync_carried(i, Some(p));
        corrected
    }

    /// After a click: tell the client about every slot it got wrong, or that
    /// it did not claim, and the carried stack.
    fn answer_click(
        &mut self,
        i: usize,
        p: &play::ContainerClick,
        world: &World,
        changed: &[usize],
    ) {
        let in_menu = p.container != 0;
        // handleContainerClick: a click made without the latest state gets the
        // whole menu (`broadcastFullState`). Answering it slot by slot can leave
        // the client with a correction of a click it has already gone past.
        let g = &self.guests[i];
        let open = g
            .menu
            .as_ref()
            .filter(|_| in_menu)
            .map(|m| (m.size, m.state));
        let stale = match open {
            Some((_, state)) => p.state != state,
            None => !in_menu && p.state != g.menu_state,
        };
        if stale {
            match open {
                Some(_) if g.menu.as_ref().is_some_and(|m| m.slots == Slots::Native) => {
                    self.resend_menu(g.conn, p.container, world);
                }
                Some((size, _)) => {
                    let items = (0..size).map(|s| self.view_slot(i, world, s)).collect();
                    self.send_menu_content(i, items);
                }
                None => self.send_inventory(i),
            }
            return;
        }
        let total = if in_menu {
            self.guests[i].menu.as_ref().map_or(0, Menu::slot_count)
        } else {
            46
        };
        let items: Vec<_> = (0..total).map(|s| self.view_slot(i, world, s)).collect();
        let sent_changes: Vec<_> = if let Some(menu) = g.menu.as_ref().filter(|_| in_menu) {
            menu.sent
                .iter()
                .zip(&items)
                .map(|(old, now)| old != now)
                .collect()
        } else {
            Vec::new()
        };
        let mut state = open.map_or(g.menu_state, |(_, state)| state);
        let resend = self.sync_click_prediction(i, p, &mut state, &items, |slot| {
            changed.contains(&slot) || sent_changes.get(slot).copied().unwrap_or(false)
        });
        // What the client now believes equals the server's state.
        self.guests.update(i, |g| {
            if in_menu {
                // Nor is the inventory menu's sync to send what this click settled:
                // ServerPlayer.tick broadcasts only the open menu.
                let own = open.map_or(0, |(size, _)| size);
                let settled = p
                    .changed
                    .iter()
                    .map(|(slot, _)| *slot as usize)
                    .chain(changed.iter().copied())
                    .chain(resend.iter().copied());
                for slot in settled.filter(|&s| (own..total).contains(&s)) {
                    g.menu_sent[slot - own + 9] = items[slot].clone();
                }
                if let Some(m) = g.menu.as_mut() {
                    m.state = state;
                    m.sent = items;
                }
            } else {
                g.menu_state = state;
                g.menu_sent = items;
            }
        });
    }

    /// AbstractContainerMenu.broadcastChanges' data slots: a furnace's
    /// AbstractFurnaceBlockEntity.dataAccess (lit time left, lit total, cooking
    /// timer, cooking total), each sent when it changes.
    fn sync_container_data(&mut self, i: usize, world: &World) {
        self.guests.update(i, |g| {
            let Some(m) = g.menu.as_mut() else {
                return;
            };
            let Slots::Block(pos) = m.slots else {
                return;
            };
            // AbstractFurnaceBlockEntity.dataAccess, BrewingStandBlockEntity.dataAccess,
            // CrafterBlockEntity.containerData.
            let (furnace, brewing, crafter);
            let (data, declared): (&[i32], Option<usize>) = match world.block_entities.get(&pos) {
                Some(BlockEntity::Furnace(f)) => {
                    furnace = [f.lit_remaining, f.lit_total, f.cooking_timer, f.cooking_total];
                    (&furnace, None)
                }
                Some(BlockEntity::Brewing(b)) => {
                    brewing = [b.brew_time, b.fuel, b.total_brew_time, b.total_fuel];
                    (&brewing, None)
                }
                // CrafterBlockEntity.containerData: the disabled slots, then triggered.
                Some(BlockEntity::Crafter(c)) => {
                    let mut slots = [0; 10];
                    for (slot, disabled) in slots.iter_mut().zip(c.disabled) {
                        *slot = i32::from(disabled);
                    }
                    slots[9] = i32::from(c.is_triggered());
                    crafter = slots;
                    (&crafter, None)
                }
                // ContainerData supplies zeros for slots the mod has not written.
                Some(BlockEntity::Mod(e)) if m.mod_type.is_some() || !e.data.is_empty() => {
                    (&e.data, m.mod_type.map(|t| t.data_slots))
                }
                _ => return,
            };
            let count = declared.unwrap_or(data.len());
            let sync_len = count.min(i16::MAX as usize + 1);
            let window = m.window;
            let unsent = (count > sync_len).then_some(count);
            if unsent.is_some() && unsent != m.reported_unsent_data_len {
                log::warn!(
                    "Sunrise: menu {} window {window} cannot sync data slots at indices {sync_len}..{count} (ContainerSetData id max is {})",
                    m.registry_id,
                    i16::MAX,
                );
            }
            m.reported_unsent_data_len = unsent;
            m.sent_data.resize(sync_len, None);
            let mut changed = Vec::new();
            for (index, sent) in m.sent_data.iter_mut().enumerate() {
                let value = data.get(index).copied().unwrap_or_default() as i16;
                if *sent != Some(value) {
                    *sent = Some(value);
                    changed.push((index as i16, value));
                }
            }
            for (id, value) in changed {
                g.send(&C::ContainerSetData(play::ContainerSetData {
                    container: window,
                    id,
                    value,
                }));
            }
        });
    }

    /// Slot changes of open containers (AbstractContainerMenu.broadcastChanges).
    pub fn sync_containers(&mut self, world: &World) -> Vec<super::hub::HostEvent> {
        let mut events = Vec::new();
        for i in 0..self.guests.len() {
            let Some(menu) = self.guests[i]
                .menu
                .as_ref()
                .filter(|menu| !menu.removed && self.here(&self.guests[i]))
            else {
                continue;
            };
            let (total, window, slots) = (menu.slot_count(), menu.window, menu.slots);
            if !slots.valid(world) {
                let guest = &self.guests[i];
                events.push(super::hub::HostEvent::CloseInvalidContainer {
                    conn: guest.conn,
                    window,
                    generation: guest.menu_generation,
                });
                continue;
            }
            for slot in 0..total {
                let now = self.view_slot(i, world, slot);
                let sent = self.guests[i]
                    .menu
                    .as_ref()
                    .and_then(|m| m.sent.get(slot).cloned());
                if sent.as_ref() == Some(&now) {
                    continue;
                }
                let net = self.stack(&now);
                self.guests.update(i, |g| {
                    if let Some(m) = g.menu.as_mut() {
                        m.state = (m.state + 1) & 0x7FFF;
                        let state = m.state;
                        if m.sent.len() <= slot {
                            m.sent.resize(slot + 1, None);
                        }
                        m.sent[slot] = now;
                        g.send(&C::ContainerSetSlot(play::ContainerSetSlot {
                            container: window,
                            state,
                            slot: slot as i16,
                            item: net,
                        }));
                    }
                });
            }
            self.sync_carried(i, None);
            // Then the data slots, as broadcastChanges does.
            self.sync_container_data(i, world);
        }
        events
    }
}

/// Inventory index of the n-th player slot of a container menu (27 main, then the hotbar).
fn player_slot(n: usize) -> usize {
    if n < 27 { 9 + n } else { n - 27 }
}

/// `Level.soundSeedGenerator.nextLong` for `ClientboundSoundPacket.seed`.
fn next_sound_seed(g: &mut impl RandomSource) -> i64 {
    g.next_long()
}

/// `PlayerList.broadcast`'s distance test: the summed squared axis distances
/// against the squared radius, and a player exactly at the radius is out.
pub fn within_range(at: [f64; 3], pos: [f64; 3], range: f64) -> bool {
    let d: f64 = (0..3).map(|k| (pos[k] - at[k]).powi(2)).sum();
    d < range * range
}

/// The position `ServerLevel.globalLevelEvent` sends a player whose feet are
/// at `player` for an event at `pos` (its lambda): `pos` for a player in the
/// event's level (`same_level`) whose squared distance to the block's centre is
/// below `Mth.square(32)`, the block 32 blocks from a further one towards that
/// centre, and the block a player in another level stands in. The client
/// places the sound from it (`LevelEventHandler.globalLevelEvent`).
fn global_level_event_position(pos: [i32; 3], player: [f64; 3], same_level: bool) -> [i32; 3] {
    use sunrise_core::{geometry::Vec3, position::BlockPos};
    let feet = Vec3::new(player[0], player[1], player[2]);
    let at = if same_level {
        let center = Vec3::new(
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        );
        // Entity.distanceToSqr against the int square widened (`i2d`, `dcmpg`, `ifge`).
        if feet.distance_squared(center) < f64::from(32 * 32) {
            center
        } else {
            feet.add_vector(center.subtract(feet).normalize().scale(32.0))
        }
    } else {
        feet
    };
    let block = BlockPos::containing(at);
    [block.x, block.y, block.z]
}

#[cfg(test)]
mod tests {
    use super::{next_sound_seed, within_range};
    use sunrise_core::random::LegacyRandom;

    #[test]
    fn two_sounds_in_one_tick_get_different_seeds() {
        let mut g = LegacyRandom::new(0);
        let a = next_sound_seed(&mut g);
        let b = next_sound_seed(&mut g);
        assert_ne!(a, b);
        assert_ne!(a, 0);
        assert_ne!(b, 0);
    }

    #[test]
    fn the_tick_is_not_the_sound_seed() {
        let ticks = 12i64;
        let mut g = LegacyRandom::new(0);
        assert_ne!(next_sound_seed(&mut g), ticks);
    }

    /// `PlayerList.broadcast` compares the squared distances with `ifge`, so a
    /// player exactly on the radius gets no packet while one a hair inside does.
    #[test]
    fn a_guest_exactly_on_the_radius_hears_nothing() {
        let at = [10.0, 64.0, -3.0];
        let at_radius = [at[0] + 16.0, at[1], at[2]];
        assert!(!within_range(at, at_radius, 16.0));
        assert!(within_range(at, [at[0] + 15.99, at[1], at[2]], 16.0));
        assert!(within_range(at, at, 16.0));
    }
}
