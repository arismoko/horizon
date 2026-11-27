//! Mounting for native mods (`native_entity_mount`: NeoForge's `EntityMountEvent`). Each place
//! the acting player or a mob gets on or off a vehicle asks `native_may_board` or
//! `native_may_leave` first, and does nothing when a mod says no.
use super::commands::Target;
use crate::game::native_hooks::mob_kind;
use sunrise_gameplay_common::game::transactions::entities::EntityRemovalGameExt as _;

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn retire_vehicle_registration(&self, target: Target) {
        match target {
            Target::Boat(id) => {
                if let Some(boat) = self.common.boats.iter().find(|boat| boat.id == id) {
                    boat.attachment_lifetime.retire();
                }
            }
            Target::Cart(id) => {
                if let Some(cart) = self.common.carts.iter().find(|cart| cart.id == id) {
                    cart.attachment_lifetime.retire();
                }
            }
            _ => {}
        }
    }

    /// Run effects before `Entity.setRemoved`. Keep the entity accessible to
    /// callbacks, but refuse another removal of it while the effects run.
    pub(in crate::game::server_side) fn with_removal_preparation(
        &mut self,
        target: Target,
        effects: impl FnOnce(&mut Self),
    ) -> Option<()> {
        if self.target_removed(&target)
            || !self
                .common
                .persist
                .preparing_vehicle_removals
                .insert(target)
        {
            return None;
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| effects(self)));
        self.common
            .persist
            .preparing_vehicle_removals
            .remove(&target);
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
        Some(())
    }

    pub(in crate::game::server_side) fn with_vehicle_removal_effects<R>(
        &mut self,
        target: Target,
        effects: impl FnOnce(&mut Self),
        remove: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        self.with_removal_preparation(target, effects)?;
        self.with_destroying_vehicle(target, remove)
    }

    /// `Entity.setRemoved` makes a vehicle invisible before notifying riders.
    /// The owning list is kept stable while callbacks run.
    pub(in crate::game::server_side) fn with_destroying_vehicle<R>(
        &mut self,
        target: Target,
        action: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        let id = self.network_entity_id(target)? as u32;
        let y = self.target_position(target)?[1];
        if self
            .common
            .persist
            .preparing_vehicle_removals
            .contains(&target)
            || !self.common.persist.destroying_vehicles.insert(target)
        {
            return None;
        }
        self.retire_vehicle_registration(target);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if !self.common.persist.notified_vehicle_removals.remove(&id) {
                self.dismount_removed_passengers(id, y);
            }
            action(self)
        }));
        self.common.persist.destroying_vehicles.remove(&target);
        match result {
            Ok(value) => Some(value),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Whether this game is a server whose mounts the mods hear: the server thread or a
    /// dedicated server, never a client.
    fn native_hears_mounts(&self) -> bool {
        self.server.serving.is_some() || self.common.options.dedicated.is_some()
    }

    /// The entity id of the acting player: a guest's while their action runs (kept with the
    /// action, as the hub is often out of `lan` then), else none (0).
    pub(in crate::game) fn native_acting_entity(&self) -> u32 {
        match self.server.lan_acting {
            Some(conn) => self.server.lan_acting_entity.unwrap_or_else(|| {
                let guests = self.lan_guest_entities();
                guests.iter().find(|g| g.0 == conn).map_or(0, |g| g.1)
            }),
            None => 0,
        }
    }

    /// The type of the vehicle with entity id `id`.
    fn native_vehicle_kind(&self, id: u32) -> String {
        if let Some(b) = self.boat_object(id) {
            b.entity_type()
        } else if let Some(c) = self.cart_object(id) {
            c.kind.id().into()
        } else if let Some(vehicle) = self.common.persist.removed_vehicles.get(&id) {
            vehicle.kind()
        } else if self.common.cushions.iter().any(|c| c.id == id) {
            "minecraft:cushion".into()
        } else if let Some(m) = self.common.mob_object(id) {
            mob_kind(m)
        } else if self.common.tnts.iter().any(|t| t.id == id) {
            "minecraft:tnt".into()
        } else if self.common.armor_stands.iter().any(|a| a.id == id) {
            self.target_type(Target::Stand(id))
        } else if self.server.others.0.iter().any(|o| o.id == id) {
            self.target_type(Target::Other(id))
        } else if let Some(row) = self.common.general_object(id) {
            super::general::general_type_catalog()
                .key(row.type_slot)
                .to_owned()
        } else if self.player_vehicle(id).is_some() {
            "minecraft:player".into()
        } else {
            String::new()
        }
    }

    /// `native_entity_mount` for `rider` (the acting player when `None`) and `vehicle`: whether
    /// the mounting or dismounting goes ahead.
    fn native_mount_allowed(&mut self, rider: Option<u32>, vehicle: u32, mounting: bool) -> bool {
        if !self.native_hears_mounts()
            || !mod_api::__private::listened(mod_api::HookId::EntityMount)
        {
            return true;
        }
        let entity = rider.unwrap_or_else(|| self.native_acting_entity());
        let notification = (entity, vehicle, mounting);
        // A callback can discard the vehicle before this edge is unlinked.
        // Its cleanup must not dispatch the identical in-flight request again.
        if !self.common.persist.mount_notifications.insert(notification) {
            return false;
        }
        let kind = match rider {
            Some(id) => self.common.mob_object(id).map_or_else(
                || {
                    self.common
                        .general_object(id)
                        .map_or_else(String::new, |row| {
                            super::general::general_type_catalog()
                                .key(row.type_slot)
                                .to_owned()
                        })
                },
                mob_kind,
            ),
            None => "minecraft:player".into(),
        };
        let vehicle_kind = self.native_vehicle_kind(vehicle);
        let mut canceled = false;
        self.native_entity_mount(mod_api::__private::EntityMountCall {
            dimension: self.common.dim.id(),
            entity,
            kind: &kind,
            vehicle,
            vehicle_kind: &vehicle_kind,
            mounting,
            canceled: &mut canceled,
        });
        self.common
            .persist
            .mount_notifications
            .remove(&notification);
        !canceled
    }

    /// The acting player is about to board `vehicle`: false when a mod canceled it.
    pub fn native_may_board(&mut self, vehicle: u32) -> bool {
        // A fake player boards nothing: every boarding path asks here first.
        if self.fake_blocks(mod_api::FakeEffect::Ride) {
            return false;
        }
        self.native_mount_allowed(None, vehicle, true)
    }

    /// The acting player is about to get off `vehicle`: false when a mod canceled it.
    pub(in crate::game) fn native_may_leave(&mut self, vehicle: u32) -> bool {
        self.native_mount_allowed(None, vehicle, false)
    }

    /// The non-player `rider` is about to board (`mounting`) or leave `vehicle`: false when a mod
    /// canceled it.
    pub(in crate::game::server_side) fn native_mob_may_ride(
        &mut self,
        rider: u32,
        vehicle: u32,
        mounting: bool,
    ) -> bool {
        self.native_mount_allowed(Some(rider), vehicle, mounting)
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
