//! Moving colliders for native mods (`EntityRef::set_collider`): the boxes an entity carries
//! that every other entity collides with, kept on the server, saved with the entity, and sent
//! to the Sunrise clients that track it.
//!
//! The server keeps a [`ColliderState`] by entity id beside the entity's attachments, and a
//! client keeps what its tracking server sent in `Remote::colliders`. A level with no colliders
//! holds empty maps, so nothing else in the game pays for them.
use super::{Dim, commands::Target};
use mod_api::{Collider, ColliderError, ColliderPose, EntityRef};
use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use sunrise_core::{geometry::Vec3, nbt::Tag};
use sunrise_gameplay_common::game::native_collider_state::{
    Placement, PlacementKey, SAVE_KEY, origin_bits,
};
use sunrise_mod_runtime::attachments::Holder;
use sunrise_protocol::collider_sync::{self, Message};
use sunrise_sim::colliders::{ColliderState, normalize_pose};
use sunrise_world::moving_colliders::History;

/// The recent frames of each level's colliders, on a server, for the move check of a player a
/// collider carries.
#[derive(Default)]
pub(in crate::game) struct Histories(
    pub(in crate::game) HashMap<Dim, std::rc::Rc<std::cell::RefCell<History>>>,
);

/// What the server sends about one entity's collider this tick.
pub(in crate::game) struct ColliderChange {
    pub(in crate::game) entity: u32,
    pub(in crate::game) update: ColliderUpdate,
}

pub(in crate::game) enum ColliderUpdate {
    Set {
        previous: Option<ColliderState>,
        state: ColliderState,
    },
    Pose(ColliderPose),
    Clear,
}

impl crate::game::ServerGame<'_> {
    /// The entity id a handle names when it may carry a collider: a server entity that is
    /// not a player.
    pub(in crate::game::server_side) fn collider_holder(
        &self,
        entity: EntityRef,
    ) -> Result<u32, ColliderError> {
        if !self.common.is_logical_server() {
            return Err(ColliderError::StaleEntity);
        }
        match self.native_entity_target(entity) {
            None | Some(Target::Player | Target::Guest(_)) => Err(ColliderError::StaleEntity),
            Some(_) => Ok(entity.id()),
        }
    }

    pub(in crate::game) fn entity_collider(&self, entity: EntityRef) -> Option<Collider> {
        let id = self.collider_holder(entity).ok()?;
        self.common
            .entity_attachments
            .collider(id)
            .map(ColliderState::to_api)
    }

    pub(in crate::game) fn entity_set_collider(
        &mut self,
        entity: EntityRef,
        collider: Option<Collider>,
    ) -> Result<(), ColliderError> {
        let id = self.collider_holder(entity)?;
        let Some(collider) = collider else {
            if self.common.entity_attachments.remove_collider(id) {
                self.server.collider_changes.push(ColliderChange {
                    entity: id,
                    update: ColliderUpdate::Clear,
                });
            }
            return Ok(());
        };
        let state = ColliderState::from_api(&collider)?;
        let previous = self.common.entity_attachments.collider(id).cloned();
        let level = self.common.dim;
        self.common
            .entity_attachments
            .insert_collider(id, level, state.clone());
        self.server.collider_changes.push(ColliderChange {
            entity: id,
            update: ColliderUpdate::Set { previous, state },
        });
        Ok(())
    }

    pub(in crate::game) fn entity_set_collider_pose(
        &mut self,
        entity: EntityRef,
        pose: ColliderPose,
    ) -> Result<(), ColliderError> {
        let id = self.collider_holder(entity)?;
        let pose = normalize_pose(pose)?;
        let Some(entry) = self.common.entity_attachments.collider_mut(id) else {
            return Err(ColliderError::NoCollider);
        };
        if entry.pose() == pose {
            return Ok(());
        }
        entry.set_pose(pose);
        self.server.collider_changes.push(ColliderChange {
            entity: id,
            update: ColliderUpdate::Pose(pose),
        });
        Ok(())
    }

    /// Reads the collider a saved entity carried. A malformed one is dropped with a log line.
    pub fn load_saved_collider(&mut self, id: u32, tag: &Tag) {
        let Some(saved) = tag.get(SAVE_KEY) else {
            return;
        };
        if let Some(state) = ColliderState::from_nbt(saved) {
            self.common
                .entity_attachments
                .insert_collider(id, self.common.dim, state);
        } else {
            log::warn!("Sunrise: dropped a malformed saved collider of entity {id}");
        }
    }

    /// Where the collider entity `id` is, in the server level in place.
    pub fn collider_origin(&self, id: u32) -> Option<Vec3> {
        let at = self.target_position(self.level_entity(id as i32)?)?;
        Some(Vec3::new(at[0], at[1], at[2]))
    }

    /// Every entity in the vehicle stack the entity `id` is in on the server.
    pub fn vehicle_stack(&self, id: u32) -> Vec<u32> {
        let Some(epoch) = self.common.native_lifecycle.epoch() else {
            return vec![id];
        };
        let entity = EntityRef::new(epoch, id);
        let root = self.entity_root_vehicle(entity).unwrap_or(entity);
        let mut stack = vec![root.id()];
        let _ = self.entity_visit_indirect_passengers(root, &mut |rider| {
            stack.push(rider.id());
            ControlFlow::Continue(())
        });
        if !stack.contains(&id) {
            stack.push(id);
        }
        stack
    }

    /// The frame the collider of entity `id` has now on the server.
    pub(in crate::game) fn collider_now_frame(
        &self,
        id: u32,
    ) -> Option<sunrise_core::geometry::oriented::Frame> {
        let state = self.common.entity_attachments.collider(id)?;
        Some(state.frame(self.collider_origin(id)?))
    }

    /// Calls `f` with each collider of the server level in place.
    pub(in crate::game::server_side) fn each_collider(
        &self,
        mut f: impl FnMut(u32, &ColliderState) -> bool,
    ) -> bool {
        for id in self.common.entity_attachments.colliders_in(self.common.dim) {
            if let Some(entry) = self.common.entity_attachments.collider(id)
                && !f(id, entry)
            {
                return false;
            }
        }
        true
    }

    /// Whether the entity `id` is in no vehicle stack but its own.
    pub(in crate::game::server_side) fn is_alone(&self, id: u32) -> bool {
        let Some(epoch) = self.common.native_lifecycle.epoch() else {
            return true;
        };
        let entity = EntityRef::new(epoch, id);
        self.entity_is_passenger(entity) == Some(false)
            && self.entity_is_vehicle(entity) == Some(false)
    }

    /// Whether `kept` is still the vehicle stack of the entity `id`.
    pub(in crate::game::server_side) fn stack_is(&self, id: u32, kept: &[u32]) -> bool {
        if kept == [id] {
            self.is_alone(id)
        } else {
            let mut now = self.vehicle_stack(id);
            let mut kept = kept.to_vec();
            now.sort_unstable();
            kept.sort_unstable();
            now == kept
        }
    }

    /// The colliders of the server level in place, where they stand now.
    pub fn placed_moving_colliders(&mut self) -> sunrise_world::moving_colliders::Prepared {
        use sunrise_world::moving_colliders::{Placed, Prepared};
        let level = self.common.dim;
        let mut any = false;
        self.each_collider(|_, _| {
            any = true;
            false
        });
        if !any {
            self.common.collider_placements.0.remove(&level);
            return Prepared::empty();
        }
        if let Some(kept) = self.common.collider_placements.0.get(&level) {
            let mut at = 0;
            let same = self.each_collider(|id, state| {
                let key = kept.keys.get(at);
                at += 1;
                let Some(origin) = self.collider_origin(id) else {
                    return false;
                };
                key.is_some_and(|key| {
                    key.id == id
                        && key.serial == state.serial()
                        && key.origin == origin_bits(origin)
                        && self.stack_is(id, &key.stack)
                })
            });
            if same && at == kept.keys.len() {
                return kept.prepared.clone();
            }
        }
        let mut keys = Vec::new();
        let mut placed = Vec::new();
        self.each_collider(|id, state| {
            if let Some(origin) = self.collider_origin(id) {
                let stack = self.vehicle_stack(id);
                keys.push(PlacementKey {
                    id,
                    serial: state.serial(),
                    origin: origin_bits(origin),
                    stack: stack.clone(),
                });
                placed.push(Placed {
                    id,
                    collider: state.oriented(origin),
                    exempt: stack,
                });
            }
            true
        });
        let prepared = Prepared::new(placed);
        self.common.collider_placements.0.insert(
            level,
            Placement {
                keys,
                prepared: prepared.clone(),
            },
        );
        prepared
    }

    /// The history the level keeps of its colliders, if it has any.
    #[cfg(any(test, feature = "test-support"))]
    pub fn level_collider_history(
        &self,
    ) -> Option<std::rc::Rc<std::cell::RefCell<sunrise_world::moving_colliders::History>>> {
        self.server
            .collider_histories
            .0
            .get(&self.common.dim)
            .cloned()
    }

    /// The server level in place, including the world lifetime and runtime level generation.
    pub fn collider_level(&self) -> Option<mod_api::LevelRef> {
        Some(self.level_ref(self.common.native_lifecycle.epoch()?, self.common.dim.key()))
    }

    /// Runs `f` with this server level's moving colliders and history.
    pub fn in_this_levels_colliders<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        use sunrise_world::moving_colliders::{HistoryScope, Scope};

        let level = self.collider_level();
        if level.is_some_and(Scope::covers) {
            return f(self);
        }
        let placed = self.placed_moving_colliders();
        let history = self
            .server
            .collider_histories
            .0
            .get(&self.common.dim)
            .cloned();
        let _history = HistoryScope::replace(history);
        let _scope = Scope::enter_level(level, placed);
        f(self)
    }

    /// Records the level's colliders as its tick begins into the level's history and answers it.
    pub(in crate::game::server_side) fn collider_history(
        &mut self,
        placed: &sunrise_world::moving_colliders::Prepared,
    ) -> Option<std::rc::Rc<std::cell::RefCell<sunrise_world::moving_colliders::History>>> {
        let level = self.common.dim;
        if placed.is_empty() {
            self.server.collider_histories.0.remove(&level);
            return None;
        }
        let history = self
            .server
            .collider_histories
            .0
            .entry(level)
            .or_default()
            .clone();
        history.borrow_mut().record(placed);
        Some(history)
    }

    /// Sends what changed this tick to the clients that track each entity and have the
    /// channel, except those whose whole collider `fresh` is about to send.
    pub(in crate::game) fn flush_collider_sync(&mut self, fresh: &HashSet<(u128, u32)>) {
        let changes = std::mem::take(&mut self.server.collider_changes);
        for change in changes {
            let senders = self.server.lan.as_ref().map_or_else(Vec::new, |host| {
                host.hub
                    .guests
                    .iter()
                    .filter(|g| {
                        host.hub.here(g)
                            && g.placed
                            && g.negotiated_collider_sync()
                            && g.tracker.is_tracking(change.entity as i32)
                            && !fresh.contains(&(g.uuid, change.entity))
                    })
                    .map(|g| g.sender.clone())
                    .collect()
            });
            if senders.is_empty() {
                continue;
            }
            let entity = change.entity;
            match change.update {
                ColliderUpdate::Set { previous, state } => {
                    let messages = match &previous {
                        Some(old) => collider_sync::changes(entity, old.wire(), state.wire()),
                        None => state.messages(entity),
                    };
                    for message in &messages {
                        self.send_collider_sync(&senders, message);
                    }
                    if previous.is_some_and(|old| old.pose() != state.pose()) {
                        self.send_collider_sync(
                            &senders,
                            &Message::Pose {
                                entity,
                                rotation: state.rotation(),
                                pivot: state.pivot(),
                            },
                        );
                    }
                }
                ColliderUpdate::Pose(pose) => {
                    self.send_collider_sync(
                        &senders,
                        &Message::Pose {
                            entity,
                            rotation: pose.rotation,
                            pivot: pose.pivot,
                        },
                    );
                }
                ColliderUpdate::Clear => {
                    self.send_collider_sync(&senders, &Message::Clear { entity })
                }
            }
        }
    }

    /// The whole collider of the entity `id` to one player who started tracking it.
    pub(in crate::game) fn send_collider_snapshot(
        &mut self,
        sender: &sunrise_sim::lan::conn::PlayerSender,
        id: u32,
    ) {
        let Some(entry) = self.common.entity_attachments.collider(id) else {
            return;
        };
        let messages = entry.messages(id);
        for message in &messages {
            self.send_collider_sync(std::slice::from_ref(sender), message);
        }
    }

    pub(in crate::game) fn send_collider_sync(
        &mut self,
        senders: &[sunrise_sim::lan::conn::PlayerSender],
        message: &Message,
    ) {
        use mod_api::{PayloadBytes, PayloadId, PayloadSendError};
        use sunrise_sim::raw_payloads::Endpoint;
        let body = collider_sync::encode(message);
        for sender in senders {
            let sent = self.queue_current_payload(
                Endpoint::server(sender.clone()),
                &PayloadId(collider_sync::CHANNEL.into()),
                PayloadBytes(body.clone()),
            );
            match sent {
                Ok(_) => {}
                // A peer without the channel, or between epochs, is sent nothing.
                Err(
                    PayloadSendError::UnsupportedPeer(_)
                    | PayloadSendError::NotNegotiated
                    | PayloadSendError::WrongProtocol(_)
                    | PayloadSendError::StaleEpoch
                    | PayloadSendError::Disconnected,
                ) => {}
                // The replica would miss the change: close, as a full queue always did.
                Err(PayloadSendError::Backpressure) => sender.close(),
                Err(error) => log::warn!("Sunrise: collider sync not sent: {error:?}"),
            }
        }
    }

    /// The holders a flush sends whole collider snapshots for: the entities players started
    /// to track. Marks the pairs so the change flush skips them.
    pub(in crate::game) fn fresh_collider_pairs(
        snapshots: &[(u128, Holder)],
    ) -> HashSet<(u128, u32)> {
        snapshots
            .iter()
            .filter_map(|(player, holder)| match holder {
                Holder::Entity(id) => Some((*player, *id)),
                _ => None,
            })
            .collect()
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::payload_transport::PayloadTransportGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use sunrise_gameplay_common::game::{
        native_collider_state::stack_in_graph, remote::passengers::PassengerSeat,
    };

    /// What a client works out from the riding its server told: the root, every rider at any
    /// depth, siblings included.
    #[test]
    fn a_stack_in_a_riding_graph_holds_root_and_every_rider() {
        // 1 rides 2 rides 3; 10 and 11 ride 12; 20 and 21 ride each other (a cycle).
        let graph = [(1, 2), (2, 3), (10, 12), (11, 12), (20, 21), (21, 20)]
            .into_iter()
            .map(|(id, vehicle)| (id, PassengerSeat { vehicle, index: 0 }))
            .collect();
        let sorted = |id| {
            let mut stack = stack_in_graph(&graph, id);
            stack.sort_unstable();
            stack
        };
        for member in [1, 2, 3] {
            assert_eq!(sorted(member), vec![1, 2, 3], "from {member}");
        }
        for member in [10, 11, 12] {
            assert_eq!(
                sorted(member),
                vec![10, 11, 12],
                "a sibling sees its siblings"
            );
        }
        assert_eq!(sorted(99), vec![99], "alone");
        // A cycle ends, and still holds the entity asked about.
        assert!(sorted(20).contains(&20));
    }

    /// The client's exemption, from its riding graph, as the collision query applies it: a
    /// collider on one sibling is not met by the other sibling or the vehicle, and is met by a
    /// stranger.
    #[test]
    fn a_client_exempts_the_stack_its_graph_gives_and_meets_a_stranger() {
        use std::sync::Arc;
        use sunrise_core::geometry::{
            Aabb,
            oriented::{ColliderBoxes, Frame, OrientedCollider},
        };
        use sunrise_world::entity_obstruction::EntityIdentity;
        use sunrise_world::moving_colliders::{Placed, Prepared, in_level, near};
        // 10 and 11 ride 12, and 10 carries the collider; 30 is a stranger.
        let graph = [(10, 12), (11, 12)]
            .into_iter()
            .enumerate()
            .map(|(index, (id, vehicle))| (id, PassengerSeat { vehicle, index }))
            .collect();
        let boxes = ColliderBoxes::new(vec![Aabb::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 2.0),
        )]);
        let placed = Prepared::new(vec![Placed {
            id: 10,
            collider: OrientedCollider::new(Frame::translation(Vec3::ZERO), Arc::new(boxes)),
            exempt: stack_in_graph(&graph, 10),
        }]);
        let meets = |id: u32| {
            let mut found = Vec::new();
            in_level(placed.clone(), || {
                near(
                    EntityIdentity(u64::from(id)),
                    Aabb::new(Vec3::new(0.5, 0.5, 0.5), Vec3::new(1.0, 1.5, 1.0)),
                    &mut found,
                );
            });
            !found.is_empty()
        };
        assert!(
            !meets(10) && !meets(11) && !meets(12),
            "the stack is exempt"
        );
        assert!(meets(30), "a stranger meets it");
    }
}
