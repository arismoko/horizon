//! `entity_constructing` (`EntityEvent.EntityConstructing`): vanilla's `Entity.<init>`, once per
//! entity on the side that makes it. Sunrise has no one constructor, so the hook is told where
//! each kind of entity starts: a spawned mob (`spawn_mob`), an entity read from disk or a
//! summon's NBT (`remember_uuid`, before its saved attachments are read), every other new
//! entity in the join pass just before `entity_join_level`, a LAN guest's player as it is
//! loaded or respawned, and on a client of a server the entities and players the server adds.
//! The first attachment access of an entity not yet told tells it first, so no mod reads an
//! attachment before the listeners have set it.

use crate::game::ServerGame;
use mod_api::Holder;
use std::collections::HashSet;
use sunrise_gameplay_common::game::entity_initialization_common::Construction;

impl ServerGame<'_> {
    pub(in crate::game) fn server_bind_constructed_entity_attachments(&mut self, id: u32) {
        if self.native_authoritative()
            && self
                .common
                .entity_attachments
                .get(&id)
                .is_some_and(|map| map.has_index_entries())
            && let Err(error) = self.bind_resident_entity_attachments(id)
        {
            log::warn!("constructor attachment binding failed for entity {id}: {error:?}");
        }
    }

    /// Forget destroyed objects in this level before its UUID metadata is pruned.
    pub fn prune_entity_initializations(&mut self, retained: &HashSet<u32>) {
        self.common
            .entity_initialization
            .initialized
            .retain(|holder| match holder {
                Holder::Entity(id) => {
                    !self.common.persist.uuids.contains_key(id) || retained.contains(id)
                }
                _ => true,
            });
    }

    /// A LAN guest's player, its file just read for connection `conn`.
    pub fn initialize_server_player(&mut self, uuid: u128, conn: u32) {
        if self.common.entity_initialization.players.insert(uuid, conn) != Some(conn) {
            self.common
                .forget_entity_initialization(Holder::Player(uuid));
        }
        self.initialize_construction(
            Construction::ServerPlayer { uuid, conn },
            "minecraft:player",
        );
    }

    /// `PlayerList.respawn` constructs another `ServerPlayer` before `restoreFrom`,
    /// even though the replacement uses the same connection and UUID.
    pub(in crate::game::server_side) fn reinitialize_server_player(
        &mut self,
        uuid: u128,
        conn: u32,
    ) {
        self.common
            .forget_entity_initialization(Holder::Player(uuid));
        self.initialize_construction(
            Construction::ServerPlayer { uuid, conn },
            "minecraft:player",
        );
    }
}

/// Explicit ownership passed alongside one travelling object's NBT, never encoded in NBT.
/// Not Clone: a transfer cannot install the same runtime state twice.
#[derive(Debug)]
pub struct TransferredEntityState {
    pub(in crate::game) attachments: Option<sunrise_mod_runtime::attachments::Attachments>,
    pub(in crate::game) initialized: bool,
    pub(in crate::game) accepted_hit: Option<std::sync::Arc<sunrise_sim::health::AcceptedHit>>,
    pub(in crate::game) died: bool,
    pub(in crate::game) transient: bool,
    /// The source id whose attachment map a transaction open at the take kept: the
    /// arrival moves that keeping to the new id.
    pub(in crate::game) journaled: Option<u32>,
    /// Filled by an abort that comes before the arrival: the contents the arrival puts back.
    pub(in crate::game) rollback:
        Option<sunrise_gameplay_common::game::transactions::ArrivalRollback>,
    /// The id the entity had before it travelled, which a kill credit may still name.
    pub(in crate::game) source: u32,
}

impl TransferredEntityState {
    /// Move the single accepted-hit owner onto a reconstructed living body.
    pub fn restore_mob(&mut self, mob: &mut sunrise_sim::mobs::Mob) {
        if mob.health > 0.0 {
            self.died = false;
        }
        if let Some(hit) = self.accepted_hit.take() {
            mob.last_damage = hit.committed;
            mob.accepted_hit = Some(hit);
        }
    }
}

impl ServerGame<'_> {
    /// Consume the source object's state before forgetting its persistence metadata.
    /// Retiring the publisher immediately makes retained query write tokens stale.
    /// Inside a transaction the take is journaled: the map's writes in the action roll back
    /// wherever the map is, and the source level's index keeps its rows until the root closes
    /// (`defer_departure`). Travel itself is not undone, so the map stays with the entity.
    pub fn take_transferred_entity_state(&mut self, id: u32) -> TransferredEntityState {
        let journaled = (!self.common.transactions.is_empty()).then(|| {
            self.journal_attachments(Holder::Entity(id));
            id
        });
        let rollback = journaled.map(|id| self.common.journal_in_flight(id));
        let mut attachments = self.common.entity_attachments.remove(&id);
        if journaled.is_some() {
            self.defer_departure(id, attachments.as_mut());
        } else if let Some(map) = &mut attachments {
            map.unbind_index();
        }
        let accepted_hit = self
            .mob_object_mut(id)
            .and_then(|mob| mob.accepted_hit.take());
        let died = self.server.died.remove(&id);
        TransferredEntityState {
            attachments,
            accepted_hit,
            died,
            transient: self.common.persist.transient.remove(&id),
            journaled,
            rollback,
            source: id,
            initialized: self
                .common
                .entity_initialization
                .initialized
                .remove(&Holder::Entity(id)),
        }
    }

    /// Install before `remember_uuid`: the destination is the same logical entity,
    /// so neither construction nor the saved overlay may replace its runtime values.
    pub fn adopt_transferred_entity_state(&mut self, id: u32, state: TransferredEntityState) {
        self.common.journal_arrival(id);
        let level = self.common.dim;
        self.common.entity_attachments.move_collider(id, level);
        if let Some(source) = state.journaled {
            self.common.journal_travel(source, id);
        }
        let now = self.common.world.game_time;
        self.server.levels.0.note_travel(state.source, id, now);
        if state.died {
            self.server.died.insert(id);
        }
        if state.transient {
            self.common.persist.transient.insert(id);
        }
        // Even an empty map marks this as runtime state for `remember_uuid`.
        let mut map = state.attachments.unwrap_or_default();
        // 🤖 Agent: an action that took this entity and was rolled back before it arrived left
        // the contents it first found here: the arrival carries those, not the action's writes.
        if let Some(kept) = state.rollback.and_then(|slot| {
            slot.lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
        }) {
            map.replace_contents(kept);
        }
        self.common.entity_attachments.insert(id, map);
        if state.initialized {
            self.common
                .entity_initialization
                .initialized
                .insert(Holder::Entity(id));
        }
    }
}

use sunrise_gameplay_common::game::entity_initialization_common::EntityInitializationCommonGameExt1 as _;
