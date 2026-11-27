use crate::game::client_side::remote_client::Remote;
use mod_api::{EntityReadError, EntityRef};
use std::ops::ControlFlow;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_gameplay_common::game::entity_type_rows::entity_type_rows;
use sunrise_gameplay_common::game::remote::passengers::passenger_root;
use sunrise_gameplay_common::game::transactions::entities::EntityRemovalGameExt as _;

impl crate::game::ClientGame<'_> {
    /// Resolve the client's packet-tracked riding state.
    fn riding_read(&self, entity: EntityRef) -> Result<&Remote, EntityReadError> {
        let (remote, id) = self.native_client_entity_context(entity)?;
        if id == remote.entity_id
            || remote.tracked_entity(id).is_some()
            || self.common.dragon.as_ref().is_some_and(|d| {
                d.id != 0
                    && id != 0
                    && sunrise_gameplay_common::game::dragon_common::dragon_part_index(
                        d.id,
                        entity.id(),
                    )
                    .is_some()
                    && !self.common.persist.removals.contains(&Target::Dragon(d.id))
                    && !self.target_removed(&Target::DragonPart(entity.id()))
            })
        {
            return Ok(remote);
        }
        Err(EntityReadError::Stale)
    }

    /// What the entity rides (`Entity.getVehicle`), if its vehicle has a handle.
    pub(in crate::game) fn entity_vehicle(&self, entity: EntityRef) -> Option<EntityRef> {
        let remote = self.riding_read(entity).ok()?;
        self.entity(remote.passenger_vehicle(entity.id())?)
    }

    /// Whether the entity rides another object, even one without a handle.
    pub(in crate::game) fn entity_is_passenger(&self, entity: EntityRef) -> Option<bool> {
        let remote = self.riding_read(entity).ok()?;
        let vehicle = remote.passenger_vehicle(entity.id());
        Some(vehicle.is_some())
    }

    /// Whether the HUD draws this entity's hearts while the local player rides it
    /// (`Entity.showVehicleHealth`, which is `instanceof LivingEntity`): on a client, the
    /// local player is one and a tracked entity answers from its type's row.
    pub(in crate::game) fn entity_shows_vehicle_health(&self, entity: EntityRef) -> Option<bool> {
        match self.native_client_entity_target(entity) {
            Some(Target::Player) => Some(true),
            Some(Target::DragonPart(_)) => Some(false),
            _ => {
                let (kind, _) = self.native_client_tracked(entity)?;
                let rows = entity_type_rows(&self.common.options.resources);
                Some(rows.is_living(kind))
            }
        }
    }

    /// The lowest vehicle under the entity (`Entity.getRootVehicle`): the
    /// entity itself when it rides nothing. Off-level intermediaries are traversed;
    /// an off-level root or a cycle returns no handle, never a descendant substitute.
    pub(in crate::game) fn entity_root_vehicle(&self, entity: EntityRef) -> Option<EntityRef> {
        let remote = self.riding_read(entity).ok()?;
        let root = passenger_root(&remote.passenger_vehicles, entity.id()).ok()?;
        self.entity(root)
    }

    /// `Entity.getPassengers`: resident handles in packet order on the client.
    pub(in crate::game) fn entity_visit_passengers(
        &self,
        entity: EntityRef,
        visit: &mut dyn FnMut(EntityRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        let remote = self.riding_read(entity)?;
        let mut after = None;
        let result = std::iter::from_fn(|| {
            let (id, index) = remote.next_passenger_seat(entity.id(), after)?;
            after = Some(index);
            Some(id)
        })
        .filter_map(|id| self.entity(id))
        .try_for_each(visit);
        Ok(result)
    }

    /// Whether any object rides the entity, even one without a handle.
    pub(in crate::game) fn entity_is_vehicle(&self, entity: EntityRef) -> Option<bool> {
        let remote = self.riding_read(entity).ok()?;
        Some(remote.has_passengers(entity.id()))
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
