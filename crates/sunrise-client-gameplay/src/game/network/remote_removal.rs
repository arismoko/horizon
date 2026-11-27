//! Removing entities a server dropped (`RemoveEntities`), out of remote.rs,
//! which may not grow.
use crate::game::ClientGame;

impl ClientGame<'_> {
    pub fn remove_entity(&mut self, id: i32) {
        self.remove_entities(&[id]);
    }

    /// `ClientboundRemoveEntitiesPacket`: a packet's ids go together, the big
    /// entity lists filtered once (#13007), each id's own cleanup in order.
    pub(in crate::game::client_side) fn remove_entities(&mut self, ids: &[i32]) {
        let removed: sunrise_level::fast_hash::FxHashSet<u32> =
            ids.iter().map(|&id| id as u32).collect();
        let mut vehicles = sunrise_level::fast_hash::FxHashSet::default();
        let mut include = |id| {
            if removed.contains(&id) {
                vehicles.insert(id);
            }
        };
        if let Some(r) = &self.client.remote {
            for seat in r.passenger_vehicles.values() {
                include(seat.vehicle);
            }
        }
        // Shared/server riding paths can also set these links without a
        // SetPassengers entry. Preserve their cleanup, scanning once per packet.
        for m in &self.common.mobs {
            if let Some(id) = m.extra.mount {
                include(id);
            }
            if m.extra.rider.is_some() {
                include(m.id);
            }
        }
        for id in self
            .client
            .text_displays
            .0
            .values()
            .filter_map(|d| d.vehicle)
            .chain(self.common.armor_stands.iter().filter_map(|a| a.vehicle))
            .chain(self.client.interactions.values().filter_map(|i| i.vehicle))
            .chain(
                self.common
                    .boats
                    .iter()
                    .filter(|b| b.paddle_controller)
                    .map(|b| b.id),
            )
            .chain(self.common.riding)
        {
            include(id);
        }
        // Retain removed camera entity snapshot until SetCamera replaces it (vanilla 26.3).
        self.detach_camera_if_matching(|cam_id| removed.contains(&(cam_id as u32)));
        // Snapshot before removing passenger keys: a removed passenger can
        // precede its vehicle, whose other seat state still needs clearing.
        for &id in ids {
            self.detach_remote_tracking_entity(id);
            self.client.audio.0.remove_sound_entity(id as u32);
            // Entity.setRemoved detaches every passenger, even when no empty
            // SetPassengers packet preceded the vehicle's removal. Otherwise
            // mobs and other players keep their passenger pose after it is gone.
            if vehicles.contains(&(id as u32)) {
                self.remote_clear_passengers(id);
            }
            // Before the crystals go: the dragon's beam index follows them.
            self.remove_remote_dragon(id);
            self.common.crystals.retain(|c| c.id != id as u32);
            self.remove_remote_object(id as u32);
            self.remove_remote_small_projectiles(id as u32);
            self.common.persist.uuids.remove(&(id as u32));
            if let Some(r) = self.client.remote.as_mut() {
                Self::forget_remote_general(r, id);
                if let Some(old_uuid) = r.level_entity_uuids.remove(&id)
                    && r.level_uuids.get(&old_uuid) == Some(&id)
                {
                    r.level_uuids.remove(&old_uuid);
                }
                r.glowing.remove(&id);
                r.entities.remove(&id);
                r.synced_attributes.remove(&id);
                r.passenger_vehicles.remove(&(id as u32));
                r.lerps.remove(&id);
                r.cart_lerps.remove(&id);
                r.stepped.remove(&id);
                r.head_lerps.remove(&id);
                r.pending_items.remove(&id);
            }
        }
        let keep = |id: u32| {
            if ids.len() > 1 {
                !removed.contains(&id)
            } else {
                ids.first().is_none_or(|&removed| id != removed as u32)
            }
        };
        self.common.mobs.retain(|m| keep(m.id));
        self.common.orbs.retain(|o| keep(o.id));
        self.common.dropped.retain(|d| keep(d.id));
        self.common.falling.retain(|f| keep(f.id));
        self.common.tnts.retain(|t| keep(t.id));
        if let Some(r) = self.client.remote.as_mut() {
            r.passenger_vehicles.retain(|_, seat| keep(seat.vehicle));
        }
    }
}
