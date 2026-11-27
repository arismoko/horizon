use crate::game::ClientGame;
use mod_api::TransactionLifecycle;
use sunrise_gameplay_common::game::transactions::{
    After, ClosingPhase, JournaledPlayer, Kept, KeptPlayer, Level, Pos, StorageSettlement, entities,
};
use sunrise_level::block_entities::BlockEntityIdentity;

impl ClientGame<'_> {
    pub(in crate::game) fn client_tx_close_storage_transaction(
        &mut self,
        commit: bool,
    ) -> Option<StorageSettlement> {
        let mut level = self.common.transactions.pop()?;
        let refused = commit
            && level.packets.is_some()
            && sunrise_sim::lan::conn::transaction_packets::Scope::refused();
        let commit = commit && !refused;
        let owned = level.owned_scratch.take().and_then(|id| {
            level.scratch_kept.remove(&id);
            self.common.transactions.scratch.remove(&id)
        });
        self.common
            .transactions
            .attachment_query_scopes
            .leave(level.parent_attachment_query_scope);
        let closing = if self.common.transactions.is_empty() {
            TransactionLifecycle::RootClosing
        } else {
            TransactionLifecycle::Closing
        };
        let outer = self.common.transactions.closing.replace(closing);
        let retire = !self.has_level_transaction()
            && !entities::is_committing(&self.common.transactions.committing, self.common.dim);
        let guard = ClosingPhase::new(self, outer);
        #[cfg(any(test, feature = "test-support"))]
        guard
            .game
            .trace_storage_attempt(if commit { "commit" } else { "abort" }.into());
        if let Some(scope) = level.packets.take()
            && commit
            && let Some(packets) = scope.finish()
        {
            level.captured_packets.push(packets);
        }
        guard
            .game
            .transactions
            .close_retained_originals(level.frame, commit);
        let transactions = &mut guard.game.common.transactions;
        match (commit, transactions.levels.last_mut()) {
            (true, Some(outer)) => {
                outer.merge_level_writes(&mut level);
                for (id, kept) in level.scratch_kept {
                    if transactions.scratch.contains_key(&id) {
                        outer.scratch_kept.entry(id).or_insert(kept);
                    }
                }
                for (owner, kept) in level.players {
                    outer.players.entry(owner).or_insert(kept);
                }
                for (holder, kept) in level.attachments {
                    outer.attachments.entry(holder).or_insert(kept);
                }
                outer.captured_packets.extend(level.captured_packets);
                outer.after.extend(level.after);
            }
            (true, None) => guard.game.client_tx_root_commit(level),
            (false, _) => guard.game.client_tx_roll_back(level),
        }
        if retire {
            guard.game.world.block_entities.publish_retirement();
        }
        Some(if commit {
            StorageSettlement::Committed(owned)
        } else if refused {
            StorageSettlement::Refused
        } else {
            StorageSettlement::Aborted
        })
    }

    pub(in crate::game::client_side) fn client_tx_root_commit(&mut self, level: Level) {
        for packets in level.captured_packets {
            packets.publish();
        }
        for after in level.after {
            self.client_tx_run_after(after);
        }
    }

    pub(in crate::game::client_side) fn client_tx_roll_back(&mut self, mut level: Level) {
        self.common
            .scratch_roll_back(std::mem::take(&mut level.scratch_kept));
        level.after.clear();
        level.captured_packets.clear();
        let removed = std::mem::take(&mut level.removed_block_entities);
        for (pos, kept) in level.block_entities {
            let was_removed = removed.contains_key(&pos);
            kept.restore_client(&mut self.common.world, was_removed, pos);
        }
        for ((pos, slot), kept) in level.slots {
            if self.common.world.block_entities.generation(&pos) != kept.generation {
                continue;
            }
            if let Some(target) = self
                .common
                .world
                .block_entities
                .get_mut(&pos)
                .and_then(|entity| entity.slots_mut().get_mut(slot))
            {
                *target = kept.stack;
            }
        }
        for (pos, block) in level.blocks {
            self.common
                .world
                .set_stored(pos.0, pos.1, pos.2, block.state);
        }
        for ((x, y, z), group) in level.connection_groups {
            self.common
                .world
                .set_connection_group_stored(x, y, z, group);
        }
        for (owner, kept) in level.players {
            if matches!(owner, JournaledPlayer::Local(_)) {
                self.common.inventory = kept.inventory;
                self.common.armor = kept.armor;
                self.common.offhand = kept.offhand;
                self.common.equipment = kept.equipment;
                self.common.carried = kept.carried;
            }
        }
    }

    pub(in crate::game) fn client_tx_run_after(&mut self, after: After) {
        match after {
            After::Payload(payload) => payload.publish(),
            After::Packet(packet, owner) => self.publish_mod_packet(packet, owner),
            After::Block(_)
            | After::BulkBlocks(..)
            | After::ConnectionGroup(..)
            | After::Changed(_)
            | After::ChangedWithoutHook(_)
            | After::ChangedHolder { .. } => {}
            other => {
                debug_assert!(
                    false,
                    "server-only After variant replayed on Client role: {:?}",
                    std::mem::discriminant(&other)
                );
            }
        }
    }

    pub(in crate::game) fn client_tx_container_change_effect(
        &self,
        pos: (i32, i32, i32),
        holder: Option<BlockEntityIdentity>,
    ) -> After {
        match holder {
            Some(holder) => After::ChangedHolder {
                holder,
                silent: false,
            },
            None => After::Changed(pos),
        }
    }

    pub(in crate::game) fn client_tx_journal_player(&mut self, uuid: u128) {
        let Some(level) = self.common.transactions.player_write() else {
            return;
        };
        level.keep_player(
            JournaledPlayer::Local(uuid),
            &[],
            self.common.level.player_extra.as_ref(),
            || KeptPlayer {
                inventory: self.common.inventory.clone(),
                armor: self.common.armor.clone(),
                offhand: self.common.offhand.clone(),
                equipment: self.common.equipment.clone(),
                carried: self.common.carried.clone(),
            },
        );
    }

    pub(in crate::game) fn client_tx_journal_block_entity(&mut self, pos: Pos) {
        self.common.transactions.writes = self.common.transactions.writes.wrapping_add(1);
        let Some(level) = entities::open_in(&mut self.common.transactions, self.common.dim) else {
            return;
        };
        level
            .block_entities
            .entry(pos)
            .or_insert_with(|| Kept::capture_client(&self.common.world, pos));
    }

    pub(in crate::game) fn client_tx_journal_menus(&mut self, _player: mod_api::PlayerRef) {
        self.common.transactions.player_writes =
            self.common.transactions.player_writes.wrapping_add(1);
        self.common.transactions.writes = self.common.transactions.writes.wrapping_add(1);
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
