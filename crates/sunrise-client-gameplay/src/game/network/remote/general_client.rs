//! A native mod's general rows as a Sunrise client sees them (#792, stage 5): the server's
//! `sunrise:general_entity` spawn adds one immediately, the vanilla packets keyed by its network
//! id move and remove it as they do any tracked entity, and its declared fields are its synced
//! attachments (`Remote::attachments`). Initial non-empty synced attachments arrive atomically
//! with the spawn inside a `BundleDelimiter` pairing bundle (`ServerEntity.addPairing` +
//! `ClientPacketListener.handleBundlePacket`, matching NeoForge `AttachmentSync`). Render
//! extraction retains the published generation, not the live synced holder. The row's behaviour
//! is the server's.
use super::{Lerp, Remote};
use indexmap::IndexMap;
use std::collections::HashMap;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::general::general_type_catalog;
use sunrise_mod_runtime::attachments::{Attachments, Holder};
use sunrise_protocol::general_entity::{self, Message};
use sunrise_sim::join_seq::JoinSeq;

/// One general row the server shows the window.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteGeneral {
    /// Admission to the level, even if the first attachment snapshot has yet to arrive.
    pub join: JoinSeq,
    /// The client's frozen catalog slot for the row's type.
    pub type_slot: usize,
    pub uuid: u128,
    pub position: [f64; 3],
    /// Where it was when the last client tick began, for a frame's partial tick.
    pub previous: [f64; 3],
    /// Yaw, then pitch, in degrees.
    pub rotation: [f32; 2],
    pub previous_rotation: [f32; 2],
    /// Client ticks since the spawn: `Entity.tickCount`, which wraps as Java's `int` does
    /// (read as `i32`).
    pub age: u32,
    /// `Entity.DATA_CUSTOM_NAME`, as `SetEntityData` entry 2 last set it.
    pub custom_name: Option<Text>,
    /// `Entity.DATA_CUSTOM_NAME_VISIBLE`, as `SetEntityData` entry 3 last set it.
    pub custom_name_visible: bool,
    /// `Entity.DATA_NO_GRAVITY`, as `SetEntityData` entry 5 last set it.
    pub no_gravity: bool,
    /// Frozen synced fields, initialized on spawn and rebuilt at each attachment sync boundary.
    pub render_attachments: mod_api::EntityRenderAttachments,
}

impl RemoteGeneral {
    pub(in crate::game) fn bounds(&self) -> Aabb {
        let [x, y, z] = self.position;
        general_type_catalog().bounds(self.type_slot, Vec3::new(x, y, z))
    }

    pub(in crate::game) fn kind(&self) -> &'static str {
        general_type_catalog().key(self.type_slot)
    }
}

/// The rows the server shows the window, by network id, and the warnings already given.
#[derive(Default)]
pub struct RemoteGenerals {
    /// Client entity lookup order: inserts append; removing one keeps the others in order.
    pub rows: IndexMap<i32, RemoteGeneral>,
    /// Type keys this client's catalog does not hold, each warned about once.
    pub(in crate::game) unknown: std::collections::HashSet<String>,
    /// Whether a spawn for an id already held was refused before, warned about once.
    pub(in crate::game) duplicate: bool,
    // Reuse the parent-before-child placement order while the window ticks.
    pub(in crate::game) riders: Vec<(usize, i32, u32, usize)>,
}

impl Remote {
    /// Discard the synced attachment mirror.
    pub(in crate::game) fn clear_attachments(&mut self) {
        self.attachments.clear();
    }
}

/// One client tick of the rows: `setOldPosAndRot`, then a step of the non-living
/// `InterpolationHandler` (`Entity.moveOrInterpolateTo`, 3 steps) toward the server's last move.
pub(in crate::game) fn tick_remote_general(
    rows: &mut IndexMap<i32, RemoteGeneral>,
    lerps: &mut HashMap<i32, Lerp>,
) {
    for (id, row) in rows.iter_mut() {
        row.previous = row.position;
        row.previous_rotation = row.rotation;
        row.age = row.age.wrapping_add(1);
        let Some(l) = lerps.get_mut(id).filter(|l| l.steps > 0) else {
            continue;
        };
        let n = f64::from(l.steps);
        for k in 0..3 {
            row.position[k] += (l.target[k] - row.position[k]) / n;
        }
        if let Some((yaw, pitch)) = l.rotation {
            row.rotation[0] =
                crate::game::remote_objects::lerp_yaw_step(row.rotation[0], yaw, l.steps);
            row.rotation[1] += (pitch - row.rotation[1]) / l.steps as f32;
        }
        l.steps -= 1;
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// The server removed general row `id` from the window.
    pub(in crate::game::client_side) fn forget_remote_general(r: &mut Remote, id: i32) {
        r.general.rows.shift_remove(&id);
    }
    /// A `sunrise:general_entity` payload the server sent the window.
    pub(in crate::game) fn remote_general_entity(&mut self, data: &[u8]) {
        let message = match general_entity::decode(data) {
            Ok(message) => message,
            Err(e) => return log::debug!("Sunrise: a bad general entity spawn: {e}"),
        };
        let Message::Spawn {
            id,
            uuid,
            type_key,
            position,
            rotation,
            velocity: _,
        } = message;
        // Both sides froze the same declarations (general types come from both-side
        // packages): a key this client lacks comes from a server that does not match it.
        let Some(type_slot) = general_type_catalog().slot(&type_key) else {
            if let Some(r) = self.client.remote.as_mut()
                && r.general.unknown.insert(type_key.clone())
            {
                log::warn!(
                    "Sunrise: the server sent general entity type {type_key}, not installed here"
                );
            }
            return;
        };
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        if r.general.rows.contains_key(&id) {
            if !std::mem::replace(&mut r.general.duplicate, true) {
                log::warn!("Sunrise: the server sent general entity {id} twice; the first stays");
            }
            return;
        }
        let render_attachments = match u32::try_from(id)
            .ok()
            .and_then(|id| r.attachments.get(&Holder::Entity(id)))
        {
            Some(existing) => mod_api::__private::game::capture_entity_render_attachments(existing),
            None => {
                mod_api::__private::game::capture_entity_render_attachments(&Attachments::default())
            }
        };
        let row = RemoteGeneral::new(type_slot, uuid, position, rotation, render_attachments);
        self.common
            .persist
            .uuids
            .insert(id as u32, sunrise_sim::lan::playerdata::uuid_ints(uuid));
        // Tracked: the id-keyed move, position sync, teleport and removal handlers apply.
        // Snapshot any old emitter body before publishing this new row.
        self.spawn_entity(
            id,
            &type_key,
            position,
            [rotation[0], rotation[1], rotation[0]],
        );
        if let Some(r) = self.client.remote.as_mut() {
            r.general.rows.insert(id, row);
            r.lerps.remove(&id);
        }
        self.pick_section_sync_id(id as u32);
    }

    /// `ClientLevel.tickPassenger` for the general rows in an applied passenger
    /// list. Seat a nested general vehicle before its own general passengers.
    pub fn position_remote_general_passengers(&mut self) {
        let Some(remote) = self.client.remote.as_mut() else {
            return;
        };
        if remote.general.rows.is_empty() || remote.passenger_vehicles.is_empty() {
            return;
        }
        let mut riders = std::mem::take(&mut remote.general.riders);
        riders.clear();
        riders.extend(remote.passenger_vehicles.iter().filter_map(|(&id, seat)| {
            let id = id as i32;
            if !remote.general.rows.contains_key(&id) {
                return None;
            }
            let depth = std::iter::successors(Some(seat.vehicle), |&v| remote.passenger_vehicle(v))
                .take(remote.passenger_vehicles.len() + 1)
                .count();
            Some((depth, id, seat.vehicle, seat.index))
        }));
        riders.sort_unstable();
        for &(_, id, vehicle, _) in &riders {
            if let Some(at) = self.general_passenger_position(vehicle, id as u32)
                && let Some(remote) = self.client.remote.as_mut()
                && let Some(row) = remote.general.rows.get_mut(&id)
            {
                row.position = [at.x, at.y, at.z];
            }
        }
        if let Some(remote) = self.client.remote.as_mut() {
            remote.general.riders = riders;
        }
    }
}

impl RemoteGeneral {
    pub(in crate::game) fn new(
        type_slot: usize,
        uuid: u128,
        position: [f64; 3],
        rotation: [f32; 2],
        render_attachments: mod_api::EntityRenderAttachments,
    ) -> Self {
        Self {
            join: JoinSeq::next(),
            type_slot,
            uuid,
            position,
            previous: position,
            rotation,
            previous_rotation: rotation,
            age: 0,
            custom_name: None,
            custom_name_visible: false,
            no_gravity: false,
            render_attachments,
        }
    }
    pub(in crate::game::client_side) fn publish_attachments(&mut self, attachments: &Attachments) {
        self.render_attachments =
            mod_api::__private::game::capture_entity_render_attachments(attachments);
    }
}

impl RemoteGenerals {
    /// Publish after the complete message has been applied.
    pub fn attachments_arrived(&mut self, id: i32, attachments: &Attachments) {
        if let Some(row) = self.rows.get_mut(&id) {
            row.publish_attachments(attachments);
        }
    }
    /// Forget every row: a world change or a respawn into another dimension.
    pub fn clear(&mut self) {
        self.rows.clear();
    }
}

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;
