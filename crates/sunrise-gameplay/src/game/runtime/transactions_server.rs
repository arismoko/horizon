//! Server-side transaction commit, rollback, and journal handlers.
use super::{BlockEntity, lan_menu_data::MountClickWindow};
use mod_api::TransactionLifecycle;
use std::collections::{HashMap, HashSet};
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::transactions::{
    After, AttachmentKey, BlockReplay, ClosingPhase, DeferredEffect, JournaledPlayer, Kept,
    KeptAttachments, KeptClick, KeptEquipment, KeptMenus, KeptMobEffects, KeptMobHealth,
    KeptPlayer, KeptPlayerHealth, KeptSlot, Level, MenuDataJournal, PlayerBodyKey,
    PlayerStorageHome, Pos, QuickMoveNotice, SaveChunkScope, StorageSettlement, entities,
    take_replay_row,
};
use sunrise_gameplay_common::game::transactions::{BlockEntityCreation, ReplayEntity};
use sunrise_items::items::ItemStack;
use sunrise_level::{block_entities::BlockEntityIdentity, fast_hash::FxHashSet};
use sunrise_mod_runtime::attachments::Holder;
use sunrise_sim::{
    join_seq::{JoinSeq, Joined as _},
    mobs::Mob,
};

/// Takes the commit it pushed, and any a listener's left, off the stack when it ends, however
/// it ends.
struct Telling<'a, 'g> {
    game: &'a mut crate::game::ServerGame<'g>,
    /// The stack's length before the commit was pushed: its place.
    frame: usize,
}

impl Drop for Telling<'_, '_> {
    fn drop(&mut self) {
        self.game.transactions.committing.truncate(self.frame);
    }
}

impl crate::game::ServerGame<'_> {
    /// Abort any storage transactions left open above `depth` (for example when a
    /// an engine decision unwinds before it can close its savepoint).
    pub fn abort_abandoned_transactions(&mut self, depth: usize) {
        while self.common.transactions.len() > depth {
            let _ = self.close_storage_transaction(false);
        }
    }

    /// Closes the innermost transaction: `commit` hands its changes to the one around it, or
    /// at the root makes them final and tells the world; otherwise every storage it wrote goes
    /// back to how it was. Nothing happens with none open.
    ///
    /// Always switches into the top frame's recorded dimension for the entire close (retirement,
    /// commit or rollback) and restores the caller's active dimension afterwards, so an abandoned
    /// or cross-level frame never commits or restores its level-local snapshots into another level.
    pub(in crate::game) fn server_tx_close_storage_transaction(
        &mut self,
        commit: bool,
    ) -> Option<StorageSettlement> {
        let owner = self
            .common
            .transactions
            .last()?
            .dimension
            .unwrap_or(self.common.dim);
        if let Some(settlement) = self.with_effect_level(owner, |game| {
            game.close_storage_transaction_in_place(commit)
        }) {
            settlement
        } else {
            log::error!(
                "Sunrise: could not switch to {} to close transaction; dropping level-local writes",
                owner.id()
            );
            self.drop_unreachable_transaction_frame();
            Some(if commit {
                StorageSettlement::Refused
            } else {
                StorageSettlement::Aborted
            })
        }
    }
    /// Closes the innermost transaction with its owning level already in place.
    pub(in crate::game::server_side) fn close_storage_transaction_in_place(
        &mut self,
        commit: bool,
    ) -> Option<StorageSettlement> {
        let mut level = self.common.transactions.pop()?;
        let refused = commit
            && level.packets.is_some()
            && sunrise_sim::lan::conn::transaction_packets::Scope::refused();
        let commit = commit && !refused;
        // Expire the loan before replay can reenter mods. Its closed row must never join
        // another journal, even if the current frame's commit is nested.
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
        // Kept and put back, so a transaction a root-commit listener opens and closes doesn't
        // end the root's phase early. The guard puts it back on a panic too, so a listener
        // that panics leaves the phase as it was.
        let outer = self.common.transactions.closing.replace(closing);
        // Each world's retired rows stay bound through its own root commit, including
        // transactions reentered by that commit's listeners.
        let retire = !self.has_level_transaction()
            && !entities::is_committing(&self.common.transactions.committing, self.common.dim);
        let guard = ClosingPhase::new(self, outer);
        #[cfg(any(test, feature = "test-support"))]
        guard
            .game
            .trace_storage_attempt(if commit { "commit" } else { "abort" }.into());
        guard.game.close_level(level, commit);
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
    /// Defensive fallback if a frame's recorded dimension is no longer parked: pop the frame so
    /// abandon loops always terminate and discard its level-local snapshots rather than restoring
    /// them into the wrong active level.
    pub(in crate::game::server_side) fn drop_unreachable_transaction_frame(&mut self) {
        let Some(mut level) = self.common.transactions.pop() else {
            return;
        };
        let _ = level.owned_scratch.take().and_then(|id| {
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
        let guard = ClosingPhase::new(self, outer);
        let mut merchant_corrections = std::mem::take(&mut level.trades.refused_clicks);
        merchant_corrections.append(&mut level.trades.correct_on_abort);
        let mount_replies = std::mem::take(&mut level.mount_replies);
        let departed = std::mem::take(&mut level.departed);
        guard
            .game
            .transactions
            .close_retained_originals(level.frame, false);
        // Only roll back journals that don't rely on the owning level being in place.
        level.blocks.clear();
        level.connection_groups.clear();
        level.block_entities.clear();
        level.removed_block_entities.clear();
        level.saved_pots.clear();
        level.slots.clear();
        level.item_entities.clear();
        level.general_entities.clear();
        level.level_random = None;
        level.chunk_marks.clear();
        level.biomes.clear();
        level.after.clear();
        let _ = std::mem::take(&mut level.lifetimes);
        guard.game.server_tx_roll_back(level);
        guard.game.close_departures(departed);
        if let Some(outer) = guard.game.transactions.last_mut() {
            outer.trades.refused_clicks.extend(merchant_corrections);
        } else {
            for (conn, window) in merchant_corrections {
                guard.game.resend_merchant_take(conn, window);
            }
        }
        guard.game.settle_mount_replies(mount_replies);
    }
    /// What closing `level` does, with the closing phase already set.
    pub(in crate::game) fn close_level(&mut self, mut level: Level, commit: bool) {
        if let Some(scope) = level.packets.take()
            && commit
            && let Some(packets) = scope.finish()
        {
            level.captured_packets.push(packets);
        }
        let mut merchant_corrections = std::mem::take(&mut level.trades.refused_clicks);
        if !commit {
            merchant_corrections.append(&mut level.trades.correct_on_abort);
        }
        let mount_replies = std::mem::take(&mut level.mount_replies);
        let departed = std::mem::take(&mut level.departed);
        debug_assert_eq!(
            level.dimension,
            Some(self.dim),
            "transaction closed in a different level"
        );
        // A nested operation can briefly put another dimension in place. Its inline snapshot
        // belongs to that stream, not to the parent's in-place slot.
        let child_random_owner = if commit
            && level.level_random.is_some()
            && self
                .common
                .transactions
                .last()
                .is_some_and(|outer| outer.dimension != level.dimension)
        {
            self.common
                .native_lifecycle
                .epoch()
                .map(|epoch| self.level_ref(epoch, self.common.dim.key()))
        } else {
            None
        };
        let lifetimes = std::mem::take(&mut level.lifetimes);
        self.common
            .transactions
            .close_retained_originals(level.frame, commit);
        let mut independent = None;
        if commit && let Some(outer) = self.common.transactions.last_mut() {
            if outer.dimension == level.dimension {
                outer.merge_level_writes(&mut level);
            } else {
                let mut local = Level {
                    block_entities: std::mem::take(&mut level.block_entities),
                    saved_pots: std::mem::take(&mut level.saved_pots),
                    removed_block_entities: std::mem::take(&mut level.removed_block_entities),
                    slots: std::mem::take(&mut level.slots),
                    blocks: std::mem::take(&mut level.blocks),
                    connection_groups: std::mem::take(&mut level.connection_groups),
                    item_entities: std::mem::take(&mut level.item_entities),
                    general_entities: std::mem::take(&mut level.general_entities),
                    chunk_marks: std::mem::take(&mut level.chunk_marks),
                    level_save_gate: level.level_save_gate.take(),
                    biomes: std::mem::take(&mut level.biomes),
                    after: level
                        .after
                        .extract_if(.., |after| after.level_owned())
                        .collect(),
                    ..Level::default()
                };
                level.save_chunks.retain(|&(dim, chunk, scope)| {
                    if dim != self.common.dim || scope == SaveChunkScope::Transaction {
                        return true;
                    }
                    local.save_chunks.insert((dim, chunk, scope));
                    false
                });
                if let Some(owner) =
                    entities::open_in(&mut self.common.transactions, self.common.dim)
                {
                    owner.merge_level_writes(&mut local);
                    owner.save_chunks.extend(local.save_chunks);
                    owner.after.extend(local.after);
                } else {
                    independent = Some(local);
                }
            }
        }
        match (commit, self.common.transactions.levels.last_mut()) {
            (true, Some(outer)) => {
                for (id, kept) in level.scratch_kept {
                    if self.common.transactions.scratch.contains_key(&id) {
                        outer.scratch_kept.entry(id).or_insert(kept);
                    }
                }
                outer.actions.merge(level.actions);
                outer.trades.merge(level.trades);
                if let Some(before) = level.level_random {
                    if outer.dimension == level.dimension {
                        outer.keep_level_random(None, || before);
                    } else if let Some(owner) = child_random_owner {
                        outer.keep_level_random(Some(owner), || before);
                    }
                }
                for (owner, before) in level.parked_level_randoms {
                    outer.keep_level_random(Some(owner), || before);
                }
                if outer.loot_sequences.is_none() {
                    outer.loot_sequences = level.loot_sequences;
                }
                for (chunk, randoms) in level.composter_randoms {
                    let kept = outer.composter_randoms.entry(chunk).or_default();
                    for (pos, random) in randoms {
                        kept.entry(pos).or_insert(random);
                    }
                }
                for (owner, kept) in level.players {
                    outer.players.entry(owner).or_insert(kept);
                }
                for (uuid, kept) in level.player_health {
                    outer.player_health.entry(uuid).or_insert(kept);
                }
                for (id, kept) in level.mob_health {
                    outer.mob_health.entry(id).or_insert(kept);
                }
                for (id, kept) in level.mob_effects {
                    outer.mob_effects.entry(id).or_insert(kept);
                }
                outer.effect_modifiers.extend(level.effect_modifiers);
                for (id, kept) in level.entities {
                    outer.entities.entry(id).or_insert(kept);
                }
                for (id, kept) in level.equipment {
                    outer.equipment.entry(id).or_insert(kept);
                }
                for (id, changes) in level.modifier_changes {
                    outer.modifier_changes.entry(id).or_default().merge(changes);
                }
                if outer.board.is_none() {
                    outer.board = level.board;
                }
                for (join, kept) in level.orbs {
                    outer.orbs.entry(join).or_insert(kept);
                }
                for (key, kept) in level.stations {
                    outer.stations.entry(key).or_insert(kept);
                }
                for (id, len) in level.mob_lengths {
                    outer.mob_lengths.entry(id).or_insert(len);
                }
                for (owner, kept) in level.ender_items {
                    outer.ender_items.entry(owner).or_insert(kept);
                }
                for (owner, kept) in level.player_extra {
                    outer.player_extra.entry(owner).or_insert(kept);
                }
                for (owner, kept) in level.player_books {
                    outer.player_books.entry(owner).or_insert(kept);
                }
                for (conn, kept) in level.menus {
                    outer.menus.entry(conn).or_insert(kept);
                }
                for (generation, kept) in level.menu_clicks {
                    outer.menu_clicks.entry(generation).or_insert(kept);
                }
                for (window, kept) in level.mount_drags {
                    outer.mount_drags.entry(window).or_insert(kept);
                }
                for (generation, kept) in level.native_menu_slots {
                    outer.native_menu_slots.entry(generation).or_insert(kept);
                }
                outer.menu_data.extend(level.menu_data);
                outer.captured_packets.extend(level.captured_packets);
                for (holder, kept) in level.attachments {
                    outer.attachments.entry(holder).or_insert(kept);
                }
                for (id, kept) in level.travelled {
                    outer.travelled.entry(id).or_insert(kept);
                }
                for (id, slot) in level.in_flight {
                    outer.in_flight.entry(id).or_insert(slot);
                }
                outer.save_gate = outer.save_gate.take().or(level.save_gate);
                outer.save_chunks.extend(level.save_chunks);
                outer.new_maps.append(&mut level.new_maps);
                outer.after.extend(level.after);
                // The nearest enclosing transaction of this level takes its entities back; a
                // parent in another level can't, and with none in this level they are told now.
                match (level.dimension)
                    .and_then(|dim| entities::open_in(&mut self.common.transactions, dim))
                {
                    Some(enclosing) => enclosing.lifetimes.merge(lifetimes),
                    None => self.commit_entity_journal(lifetimes, |game| {
                        if let Some(local) = independent {
                            game.server_tx_root_commit(local);
                        }
                    }),
                }
            }
            (true, None) => {
                // The commit is in place before the effects run: a join one causes (a placed
                // item's leftover spawning) never announces what the transaction added and removed.
                self.commit_entity_journal(lifetimes, |game| game.server_tx_root_commit(level));
            }
            (false, _) => {
                // The entities it added go first, so nothing is put back into one.
                let arrivals = self.roll_back_entity_journal(lifetimes);
                let dimension = level.dimension;
                self.server_tx_roll_back(level);
                if let Some(enclosing) =
                    dimension.and_then(|dim| entities::open_in(&mut self.common.transactions, dim))
                {
                    enclosing.lifetimes.keep_arrivals(arrivals);
                }
            }
        }
        self.common.close_departures(departed);
        // Rejected client predictions need the final restored menu, whether the enclosing
        // transaction commits or aborts. Do not stage a correction using tentative state.
        if let Some(outer) = self.common.transactions.last_mut() {
            outer.trades.refused_clicks.extend(merchant_corrections);
        } else {
            for (conn, window) in merchant_corrections {
                self.resend_merchant_take(conn, window);
            }
        }
        self.settle_mount_replies(mount_replies);
    }

    /// An unseeded pot loot table may draw from either the level or a named stream.
    pub(in crate::game) fn journal_block_loot_random(&mut self) {
        self.journal_level_random();
        if let Some(level) = entities::open_in(&mut self.common.transactions, self.common.dim) {
            level
                .loot_sequences
                .get_or_insert_with(|| self.server.rand_sequences.clone());
        }
    }

    /// Keeps the block entity at `pos` as it is, with its attachments and a hopper's
    /// cooldown, if a transaction is open and hasn't yet.
    pub(in crate::game) fn server_tx_journal_block_entity(&mut self, pos: Pos) {
        self.common.transactions.writes = self.common.transactions.writes.wrapping_add(1);
        let Some(level) = entities::open_in(&mut self.common.transactions, self.common.dim) else {
            return;
        };
        level
            .block_entities
            .entry(pos)
            .or_insert_with(|| Kept::capture(&self.common.world, &self.server.hoppers, pos));
    }

    /// Keeps `holder`'s attachment map as it is, if a transaction is open and hasn't yet: a
    /// block entity's with the rest of it, any other's alone.
    pub(in crate::game) fn journal_attachments(&mut self, holder: Holder) {
        let dim = self.common.dim;
        let level = if matches!(holder, Holder::BlockEntity(_)) {
            entities::open_in(&mut self.common.transactions, dim)
        } else {
            self.common.transactions.last_mut()
        };
        if let Some(level) = level {
            level
                .save_gate
                .get_or_insert_with(sunrise_sim::join_seq::visibility::PassLease::enter);
        }
        // The chunk that a block entity's or chunk's map marks unsaved (`world_attachments`).
        let chunk = match holder {
            Holder::BlockEntity([x, _, z]) => Some((x >> 4, z >> 4)),
            Holder::Chunk(chunk) => Some((chunk[0], chunk[1])),
            _ => None,
        };
        if let Some(chunk) = chunk
            && let Some(level) = entities::open_in(&mut self.common.transactions, dim)
        {
            let modified = &self.common.world.modified;
            level
                .chunk_marks
                .entry(chunk)
                .or_insert_with(|| modified.mark(chunk));
        }
        if let Holder::BlockEntity([x, y, z]) = holder {
            return self.journal_block_entity((x, y, z));
        }
        let key = match holder {
            Holder::Player(uuid) => {
                let Some(body) = self.attachment_player_body(uuid) else {
                    return;
                };
                AttachmentKey::Player(body)
            }
            other => AttachmentKey::Level(dim, other),
        };
        let Some(level) = self.common.transactions.last() else {
            return;
        };
        if level.attachments.contains_key(&key) {
            return;
        }
        let entity_bound = if let Holder::Entity(id) = holder {
            std::num::NonZeroU32::new(id).and_then(|id| {
                match self.with_resident_slot(&dim.key(), id, |slot| slot.is_bound()) {
                    super::entity_residency::SlotAccess::Done(bound) => Some(bound),
                    _ => None,
                }
            })
        } else {
            None
        };
        let Ok(kept) = self
            .attachment_slot(holder, mod_api::AttachmentAccess::Existing)
            .map(|map| KeptAttachments {
                contents: map.clone(),
                indexed: entity_bound.unwrap_or_else(|| map.has_index_binding()),
            })
        else {
            return;
        };
        if let Some(level) = self.common.transactions.last_mut() {
            level.attachments.insert(key, kept);
        }
    }

    /// Keeps the inventory of `uuid`, whose state is acting in `Game` now, on its first
    /// write in the open frame.
    pub(in crate::game) fn server_tx_journal_player(&mut self, uuid: u128) {
        let Some(level) = self.common.transactions.player_write() else {
            return;
        };
        let owner = self
            .server
            .lan_acting
            .map_or(JournaledPlayer::Local(uuid), JournaledPlayer::Guest);
        level.keep_player(
            owner,
            &self.server.ender_items,
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

    /// The same journal for a player resolved to their own parked record at `index`.
    pub(in crate::game::server_side) fn journal_parked_player(&mut self, index: usize) {
        let Some(level) = self.common.transactions.player_write() else {
            return;
        };
        let guest = &self.server.lan.as_ref().expect("resolved guest").hub.guests[index];
        level.keep_player(
            JournaledPlayer::Guest(guest.conn),
            &guest.ender_items,
            guest.saved.as_ref(),
            || KeptPlayer {
                inventory: guest.inventory.clone(),
                armor: guest.armor.clone(),
                offhand: guest.offhand.clone(),
                equipment: guest.equipment.clone(),
                carried: guest.carried.clone(),
            },
        );
    }

    /// Save the guest's inventory and current cursor wherever their turn lent
    /// them. A suspended guest cannot be re-entered to take this snapshot.
    pub(in crate::game::server_side) fn journal_guest_inventory(&mut self, conn: u32) {
        let owner = JournaledPlayer::Guest(conn);
        if self
            .common
            .transactions
            .last()
            .is_none_or(|level| level.players.contains_key(&owner))
        {
            return;
        }
        let Some(home) = self.player_storage_home(owner) else {
            return;
        };
        let kept = match home {
            PlayerStorageHome::Game => KeptPlayer {
                inventory: self.common.inventory.clone(),
                armor: self.common.armor.clone(),
                offhand: self.common.offhand.clone(),
                equipment: self.common.equipment.clone(),
                carried: self.common.carried.clone(),
            },
            PlayerStorageHome::Guest(index) => {
                let g = &self.server.lan.as_ref().expect("resolved guest").hub.guests[index];
                KeptPlayer {
                    inventory: g.inventory.clone(),
                    armor: g.armor.clone(),
                    offhand: g.offhand.clone(),
                    equipment: g.equipment.clone(),
                    carried: g.carried.clone(),
                }
            }
        };
        self.common
            .transactions
            .player_write()
            .expect("open transaction")
            .players
            .insert(owner, kept);
    }

    /// Follow the actual storage, even when public handles cannot resolve the player.
    /// Each outstanding swap holds the immediately preceding actor in the inner guest's row.
    pub(in crate::game) fn player_storage_home(
        &self,
        owner: JournaledPlayer,
    ) -> Option<PlayerStorageHome> {
        let parked_in = match owner {
            JournaledPlayer::Local(uuid) => {
                let local = self
                    .common
                    .level
                    .player_uuid
                    .map(sunrise_sim::lan::playerdata::ints_uuid);
                if local != Some(uuid) {
                    return None;
                }
                self.server.lan_guest_swaps.first().copied()
            }
            JournaledPlayer::Guest(conn) => self
                .server
                .lan_guest_swaps
                .iter()
                .position(|&swapped| swapped == conn)
                .map_or(Some(conn), |i| {
                    self.server.lan_guest_swaps.get(i + 1).copied()
                }),
        };
        match parked_in {
            Some(conn) => self.guest_index(conn).map(PlayerStorageHome::Guest),
            None => Some(PlayerStorageHome::Game),
        }
    }
    /// Keeps the saved player extra NBT (`level.player_extra`) of `uuid`, whose state is
    /// acting in `Game` now, on its first write in the open frame.
    pub(in crate::game::server_side) fn journal_player_extra(&mut self, uuid: u128) {
        let Some(level) = self.common.transactions.player_write() else {
            return;
        };
        let owner = self
            .server
            .lan_acting
            .map_or(JournaledPlayer::Local(uuid), JournaledPlayer::Guest);
        level
            .player_extra
            .entry(owner)
            .or_insert_with(|| self.common.level.player_extra.clone());
    }
    /// The same journal for a player resolved to their own parked record at `index`.
    pub(in crate::game::server_side) fn journal_parked_player_extra(&mut self, index: usize) {
        let Some(level) = self.common.transactions.player_write() else {
            return;
        };
        let guest = &self.server.lan.as_ref().expect("resolved guest").hub.guests[index];
        level
            .player_extra
            .entry(JournaledPlayer::Guest(guest.conn))
            .or_insert_with(|| guest.saved.clone());
    }
    pub(in crate::game) fn journal_player_book(
        &mut self,
        player: mod_api::PlayerRef,
        book: &mod_api::RecipeBookTypeKey,
    ) -> bool {
        let Some(level) = self.common.transactions.player_write() else {
            return false;
        };
        let owner = self.server.lan_acting.map_or(
            JournaledPlayer::Local(player.uuid()),
            JournaledPlayer::Guest,
        );
        level
            .player_books
            .entry((owner, book.clone()))
            .or_insert_with(|| {
                (
                    player,
                    self.common.level.recipe_book.custom.stored(book.as_str()),
                )
            });
        true
    }

    /// Called with the target player's body swapped in, before set_health changes it.
    pub(in crate::game) fn journal_player_health(&mut self) {
        let body = self.acting_player_body();
        let Some(level) = self.common.transactions.last_mut() else {
            return;
        };
        let Some(player) = self.common.player.as_ref() else {
            return;
        };
        let std::collections::hash_map::Entry::Vacant(entry) = level.player_health.entry(body)
        else {
            return;
        };
        let guest = self.server.lan_acting.and_then(|conn| {
            let guest = self.server.lan.as_ref()?.hub.guest(conn)?;
            Some((conn, guest.death_canceled, guest.death_time))
        });
        entry.insert(KeptPlayerHealth {
            data: player.health.data.clone(),
            events_len: player.events.len(),
            dead: player.health.dead,
            guest,
        });
    }
    pub(in crate::game) fn journal_mob_health(&mut self, id: u32) {
        let dim = self.common.dim;
        let Some(level) = self.common.transactions.last_mut() else {
            return;
        };
        if let Some(mob) = self.common.mobs.iter().find(|m| m.id == id && !m.removed) {
            level.mob_health.entry((dim, id)).or_insert(KeptMobHealth {
                health: mob.health,
                absorption: mob.absorption,
                needs_sync: mob.needs_sync,
                death_time: mob.death_time,
                death_canceled: mob.death_canceled,
                processed: self.server.died.contains(&id),
            });
        }
    }
    /// Writes one mob effect, or any active effect when `effect` is `None`. In a transaction,
    /// keep the effect lists and use the same health journal as `set_health` and `set_absorption`.
    pub(in crate::game::server_side) fn mutate_mob_effects<R>(
        &mut self,
        id: u32,
        effect: Option<&'static str>,
        mutate: impl FnOnce(&mut Mob) -> R,
    ) -> Option<R> {
        let dim = self.common.dim;
        self.journal_mob_health(id);
        let mob = self
            .common
            .mobs
            .iter_mut()
            .find(|mob| mob.id == id && !mob.removed)?;
        let invalidation = if !self.common.transactions.is_empty() {
            self.common.transactions.writes = self.common.transactions.writes.wrapping_add(1);
            let level = self
                .common
                .transactions
                .last_mut()
                .expect("open transaction");
            level
                .mob_effects
                .entry((dim, id))
                .or_insert_with(|| KeptMobEffects {
                    active: mob.extra.effects.clone(),
                    chains: mob.extra.effect_chains.clone(),
                    ambient: mob.extra.ambient_effects.clone(),
                    hidden: mob.extra.hidden_effects.clone(),
                    dirty: mob.effects_dirty,
                });
            let modifiers = match effect {
                Some(effect) => mob.effect_modifier_undo(std::iter::once(effect)),
                None => mob.effect_modifier_undo(mob.extra.effects.iter().map(|effect| effect.id)),
            };
            if let Some(modifiers) = modifiers {
                level.effect_modifiers.push(((dim, id), modifiers));
                super::living_attributes::modifier_invalidation_undo(
                    &self.common.motion.attributes,
                    mob,
                )
            } else {
                None
            }
        } else {
            None
        };
        let result = mutate(mob);
        // The ordered effect journal owns the seed; only the discarded live
        // equipment rows and detection state go to the general modifier journal.
        if let Some(changes) = invalidation
            && mob.extra.live.is_invalidated()
        {
            self.common
                .transactions
                .journal_modifier_changes(dim, id, changes);
        }
        Some(result)
    }

    /// Keeps `player`'s menu storage as it is, if a transaction is open and hasn't yet.
    pub(in crate::game) fn server_tx_journal_menus(&mut self, player: mod_api::PlayerRef) {
        self.common.transactions.player_writes =
            self.common.transactions.player_writes.wrapping_add(1);
        self.common.transactions.writes = self.common.transactions.writes.wrapping_add(1);
        let Some(level) = self.common.transactions.last() else {
            return;
        };
        let Some(g) = self.native_menu_player(player) else {
            return;
        };
        let conn = g.conn;
        let grid = (!level.menus.contains_key(&conn)).then(|| KeptMenus {
            craft_grid: g.craft_grid.clone(),
        });
        let native = (g
            .menu
            .as_ref()
            .is_some_and(|menu| menu.slots == sunrise_sim::lan::menu::Slots::Native)
            && !level.native_menu_slots.contains_key(&g.menu_generation))
        .then(|| (g.menu_generation, g.native_menu.clone()));
        if grid.is_none() && native.is_none() {
            return;
        }
        let owner = JournaledPlayer::Guest(conn);
        let ender = (!level.ender_items.contains_key(&owner)).then(|| {
            if self.server.lan_acting == Some(conn) {
                self.server.ender_items.clone()
            } else {
                g.ender_items.clone()
            }
        });
        if let Some(level) = self.common.transactions.last_mut() {
            if let Some(grid) = grid {
                level.menus.insert(conn, grid);
            }
            if let Some((generation, slots)) = native {
                level.native_menu_slots.insert(generation, slots);
            }
            if let Some(ender) = ender {
                level.ender_items.insert(owner, ender);
            }
        }
    }

    pub(in crate::game) fn journal_menu_click(&mut self, conn: u32) {
        let Some(level) = self.common.transactions.last_mut() else {
            return;
        };
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(guest) = host.hub.guest(conn) else {
            return;
        };
        let Some(menu) = guest.menu.as_ref() else {
            return;
        };
        level
            .menu_clicks
            .entry(guest.menu_generation)
            .or_insert_with(|| {
                let owner = host.menus.get(&conn);
                KeptClick {
                    conn,
                    drag: menu.drag.clone(),
                    craft: menu.craft.clone(),
                    slot_changed_seen: menu.slot_changed_seen.clone(),
                    owned: owner.and_then(|owner| owner.owned_menu.clone()),
                    machine: owner.and_then(|owner| owner.crafting.machine.as_ref().cloned()),
                    places: owner.and_then(|owner| owner.slot_places.clone()),
                }
            });
    }

    /// On both commit and abort, defer a window's reply if any storage remains tentative.
    pub(in crate::game) fn settle_mount_replies(
        &mut self,
        replies: impl IntoIterator<Item = MountClickWindow>,
    ) {
        if let Some(level) = self.common.transactions.last_mut() {
            level.mount_replies.extend(replies);
        } else {
            for window in replies {
                self.send_settled_mount_reply(window);
            }
        }
    }

    /// Release accepted writes after source settlement, in their original levels and only to
    /// the same storage rows. Don't enqueue the notices in an enclosing quick move again.
    pub(in crate::game::server_side) fn publish_quick_move_notices(
        &mut self,
        notices: Vec<QuickMoveNotice>,
    ) {
        for QuickMoveNotice { dim, holder } in notices {
            let back = self.enter_command_level(dim);
            if self.common.dim == dim {
                self.publish_holder_changed(holder, false);
            }
            if let Some(back) = back {
                self.enter_command_level(back);
            }
        }
    }

    pub(in crate::game) fn publish_holder_changed(
        &mut self,
        holder: BlockEntityIdentity,
        silent: bool,
    ) {
        let [x, y, z] = holder.position;
        let pos = (x, y, z);
        if self.common.world.block_entities.identity(pos) == Some(holder) {
            if silent {
                self.container_changed_without_hook(pos);
            } else {
                self.container_changed(pos);
            }
        }
    }

    pub(in crate::game) fn server_tx_container_change_effect(
        &self,
        pos: Pos,
        holder: Option<BlockEntityIdentity>,
    ) -> After {
        let silent = !self.server.contents_changing.is_empty()
            && self.native_level_here().is_some_and(|level| {
                self.contents_notice_active(level, BlockPos::new(pos.0, pos.1, pos.2))
            });
        let holder = holder.or_else(|| {
            // Other writes to a dispenser's slots share its identity-bound notice, so they
            // coalesce with the dispense rather than leaving a positional notice behind.
            let dispenser = &*self
                .common
                .res
                .facts()
                .get(self.common.world.block(pos.0, pos.1, pos.2))
                .name
                == "minecraft:dispenser";
            if dispenser || (!silent && self.common.transactions.quick_move_notices.is_some()) {
                self.common.world.block_entities.identity(pos)
            } else {
                None
            }
        });
        if let Some(holder) = holder {
            After::ChangedHolder { holder, silent }
        } else if silent {
            After::ChangedWithoutHook(pos)
        } else {
            After::Changed(pos)
        }
    }

    pub(in crate::game) fn commit_menu_data(&mut self, journal: MenuDataJournal) {
        let Some(host) = &mut self.server.lan else {
            return;
        };
        for ((conn, generation, index), value) in journal {
            host.hub.update_guest(conn, |guest| {
                if guest.menu_generation == generation {
                    guest.set_menu_data(index, value);
                }
            });
        }
    }

    /// What the mod block entity at `pos` held before a committed transaction that kept
    /// `block_entities` and `slots`: its kept entity, or the live items with the kept slots put
    /// back. `None` when the transaction kept nothing there.
    pub(in crate::game) fn committed_block_items_before(
        &self,
        block_entities: &HashMap<Pos, Kept>,
        slots: &HashMap<(Pos, usize), KeptSlot>,
        pos: Pos,
    ) -> Option<Vec<Option<ItemStack>>> {
        #[cfg(any(test, feature = "test-support"))]
        super::slot_changes::probe::snapshot();
        if let Some(kept) = block_entities.get(&pos) {
            return match kept.entity.as_ref() {
                Some(BlockEntity::Mod(entity)) => Some(entity.items.clone()),
                _ => None,
            };
        }
        let Some(BlockEntity::Mod(entity)) = self.common.world.block_entities.get(&pos) else {
            return None;
        };
        let mut before = entity.items.clone();
        let mut patched = false;
        for (&(slot_pos, slot), kept) in slots {
            if slot_pos == pos {
                if before.len() <= slot {
                    before.resize(slot + 1, None);
                }
                before[slot] = kept.stack.clone();
                patched = true;
            }
        }
        patched.then_some(before)
    }
    pub(in crate::game) fn server_tx_root_commit(&mut self, mut level: Level) {
        for after in &level.after {
            if let After::MenuOpen(conn, identity, _) = after {
                self.reserve_menu_publication(*conn, *identity);
            }
        }
        // Storage is final. Publish save marks before any callback can unwind or start a save.
        for (dim, chunk, _) in level.save_chunks {
            let world = if dim == self.common.dim {
                Some(&mut self.common.world)
            } else {
                self.server
                    .levels
                    .0
                    .parked_mut(dim)
                    .map(|level| &mut level.world)
            };
            if let Some(world) = world {
                world.modified.insert(chunk);
            }
        }
        // Captured packets and deferred listeners can refer to a new map.
        for (id, data) in level.new_maps {
            self.publish_map(id, data);
        }
        // Publish captured opens and resends before the final menu deltas, so a
        // snapshot taken before commit cannot overwrite a newer value.
        for packets in level.captured_packets {
            packets.publish();
        }
        self.commit_menu_data(level.menu_data);
        // A strict cell is rewound to its storage before its first edit when an ordinary write
        // shares it, or when its connection group is rewound below: replay then shows each
        // callback the block and group together. For a rewound cell, work back from its final
        // staged row to the row before its first edit. A strict step takes the row it installed
        // from the next retirement, or from storage if it survived. Ordinary removal then
        // observes its own row, never a row from a later strict edit. Other cells stay in final
        // staged storage.
        let mut ordered = indexmap::IndexMap::<Pos, bool>::new();
        for after in &level.after {
            if let After::Block(step) = after
                && (step.placement || level.blocks[&step.pos].strict)
            {
                *ordered.entry(step.pos).or_default() |= step.placement;
            }
        }
        for &pos in level.connection_groups.keys() {
            if level.blocks.get(&pos).is_some_and(|kept| kept.strict) {
                ordered.entry(pos).or_default();
            }
        }

        let mut clean_groups = Vec::new();
        for (pos @ (x, y, z), was) in level.connection_groups {
            self.common.world.set_connection_group_stored(x, y, z, was);
            if self
                .common
                .world
                .connection_group_log
                .as_ref()
                .is_some_and(|log| !log.contains(&pos))
            {
                clean_groups.push((pos, was));
            }
        }

        let mut rows = HashMap::<Pos, (ReplayEntity, bool)>::new();
        for after in level.after.iter_mut().rev() {
            match after {
                After::Block(step) if ordered.contains_key(&step.pos) => {
                    use std::collections::hash_map::Entry;
                    let (row, later_factory) = match rows.entry(step.pos) {
                        Entry::Occupied(entry) => entry.into_mut(),
                        Entry::Vacant(entry)
                            if matches!(step.creation, BlockEntityCreation::Staged { .. })
                                || step.removed.is_some() =>
                        {
                            entry.insert((
                                ReplayEntity::take(&mut self.common.world, step.pos),
                                false,
                            ))
                        }
                        Entry::Vacant(_) => continue,
                    };
                    if let BlockEntityCreation::Staged { generation, entity } = &mut step.creation {
                        if generation.is_some()
                            && row.row.as_ref().map(|(_, generation)| *generation) == *generation
                        {
                            *entity = std::mem::take(row);
                        }
                        *later_factory = true;
                    } else if *later_factory {
                        step.creation = BlockEntityCreation::Deferred;
                    }
                    if let Some(removed) = &mut step.removed {
                        *row = std::mem::take(removed);
                    }
                    if step.removed.is_some() || step.replay.clear_connection_group {
                        *later_factory = false;
                    }
                }
                After::BulkBlocks(_, _, _, writes) if !ordered.is_empty() => {
                    for (&pos, step) in writes {
                        if ordered.contains_key(&pos)
                            && let Some(removed) = &mut step.row
                        {
                            let (row, later_factory) = rows.entry(pos).or_insert_with(|| {
                                (ReplayEntity::take(&mut self.common.world, pos), false)
                            });
                            std::mem::swap(removed, row);
                            *later_factory = false;
                        }
                    }
                }
                _ => {}
            }
        }
        for (&pos, &placement) in &ordered {
            let (x, y, z) = pos;
            self.common
                .world
                .set_stored(x, y, z, level.blocks[&pos].state);
            if let Some((row, _)) = rows.remove(&pos) {
                row.install(&mut self.common.world, pos, placement);
            }
        }
        for after in &mut level.after {
            if let After::Block(step) = after
                && !ordered.contains_key(&step.pos)
                && let Some(ReplayEntity { row: Some(row), .. }) = step.removed.take()
            {
                let first = level.removed_block_entities.entry(step.pos).or_default();
                if first.is_none() {
                    *first = Some(row);
                }
            }
        }
        let mut told = std::collections::HashSet::new();
        let mut told_silent = FxHashSet::default();
        let mut told_holders = FxHashSet::default();
        let mut explicit_group_since_block = std::collections::HashSet::new();
        let mut biome_columns = std::collections::HashSet::new();
        let mut menu_lifecycle = Vec::new();
        for after in level.after {
            match after {
                After::MenuOpen(..) => menu_lifecycle.push(after),
                After::MenuClick(ref reply) if reply.closes_menu() => menu_lifecycle.push(after),

                After::Changed(pos) if !told.insert(Ok(pos)) => {}
                After::ChangedWithoutHook(pos) if !told_silent.insert(pos) => {}
                After::ChangedHolder { holder, silent }
                    if !told_holders.insert((holder, silent)) => {}
                After::ConnectionGroup(pos @ (x, y, z), group) => {
                    self.common.world.set_connection_group(x, y, z, group, true);
                    explicit_group_since_block.insert(pos);
                }
                After::Block(step) if ordered.contains_key(&step.pos) => {
                    let (x, y, z) = step.pos;
                    explicit_group_since_block.remove(&step.pos);
                    if step.removed.is_some() && !step.replay.replay_removal {
                        self.common.world.block_entities.remove(&step.pos);
                    }
                    self.server_set_block_live_with_entity(
                        [x, y, z],
                        step.to,
                        step.replay.flags,
                        step.replay.level_host,
                        step.creation,
                    );
                    let _ = self.server_process_level_events();
                }
                After::BulkBlocks(publication, tracked, removed, steps) => {
                    for (pos, step) in steps {
                        if let Some(&placement) = ordered.get(&pos) {
                            let (x, y, z) = pos;
                            self.common.world.set_stored(x, y, z, step.to);
                            if let Some(row) = step.row {
                                self.common.world.block_entities.remove(&pos);
                                row.install(&mut self.common.world, pos, placement);
                            }
                        }
                        if step.clear_connection_group {
                            explicit_group_since_block.remove(&pos);
                            self.common.world.invalidate_replaced_group(pos);
                        }
                    }
                    self.publish_bulk_edit(publication, tracked, removed);
                }
                After::Block(step) => {
                    let pos = step.pos;
                    let clears_group = step.replay.clear_connection_group;
                    if !told.insert(Err(pos)) {
                        if clears_group && explicit_group_since_block.remove(&pos) {
                            self.common.world.invalidate_replaced_group(pos);
                        }
                        continue;
                    }
                    explicit_group_since_block.remove(&pos);
                    let now = self.common.world.block(pos.0, pos.1, pos.2);
                    let Some(BlockReplay {
                        state: was,
                        flags,
                        level_host,
                        replay_removal,
                        clear_connection_group,
                    }) = level.blocks[&pos].replay
                    else {
                        continue;
                    };

                    let absent_after_writes = self.common.world.block_entities.get(&pos).is_none()
                        && level.removed_block_entities.contains_key(&pos);
                    let b = self.common.res.behaviours();
                    let net_keeps_group = b.keeps_block_entity(was, now);
                    let surviving = if now != was && !net_keeps_group {
                        take_replay_row(&mut self.common.world, pos)
                    } else {
                        None
                    };
                    if (replay_removal || b.block(now) == b.block(was))
                        && let Some((entity, generation)) =
                            level.removed_block_entities.remove(&pos).flatten()
                    {
                        self.restore_removed_block_entity(pos, *entity, generation);
                    }
                    if now != was {
                        if clear_connection_group && net_keeps_group {
                            self.common.world.invalidate_replaced_group(pos);
                        }

                        self.common.world.set_stored(pos.0, pos.1, pos.2, was);
                        let pos = [pos.0, pos.1, pos.2];
                        if level_host {
                            self.set_block_live(pos, now, flags, true);
                            let _ = self.server_process_level_events();
                        } else {
                            let _ = self.level_set_with_flags(pos, now, flags);
                        }
                        if self.common.world.block(pos[0], pos[1], pos[2]) == now {
                            let key = (pos[0], pos[1], pos[2]);
                            if let Some((entity, generation)) = surviving {
                                self.common.world.block_entities.remove(&key);
                                self.restore_removed_block_entity(key, *entity, generation);
                            } else if absent_after_writes {
                                self.common.world.block_entities.remove(&key);
                            }
                        }
                    } else if clear_connection_group {
                        self.common.world.invalidate_replaced_group(pos);
                    }
                }
                After::Biome(key) if !biome_columns.insert(key) => {}
                after => self.server_tx_run_after(after),
            }
        }
        for (pos @ (x, y, z), was) in clean_groups {
            if self.common.world.connection_group(x, y, z) == was
                && let Some(log) = self.common.world.connection_group_log.as_mut()
            {
                log.remove(&pos);
            }
        }
        for ((owner, book), (player, before)) in level.player_books {
            self.server_tx_run_after(After::RecipeBookSettings(owner, player, book, before));
        }
        // A crafting grid the transaction changed has its result made again, once for each
        // player however many writes it took (`InventoryMenu.slotsChanged`).
        for (conn, kept) in &level.menus {
            let mut guests = self
                .server
                .lan
                .iter()
                .flat_map(|host| host.hub.guests.iter());
            let changed =
                guests.any(|g| g.placed && g.conn == *conn && g.craft_grid != kept.craft_grid);
            if changed {
                self.guest_craft_result(*conn);
            }
        }
        // `menu_slot_changed` for the open mod menus whose storage this transaction changed: one
        // batch per menu, owned menus by ascending generation, then block-backed menus by
        // ascending position. A touched block costs one lookup in the open-menu index unless a
        // menu is open on it; only then is anything collected or copied.
        if mod_api::__private::has_part_hook(mod_api::HookId::MenuSlotChanged) {
            let owned = std::mem::take(&mut level.native_menu_slots);
            let mut owned_generations: Vec<_> = owned.into_iter().collect();
            owned_generations.sort_by_key(|(generation, _)| *generation);
            for (generation, before) in owned_generations {
                self.notify_owned_menu_slot_changed(generation, &before);
            }
            let mut viewed = Vec::new();
            if let Some(host) = self.server.lan.as_ref() {
                let touched = level.block_entities.keys().copied();
                for pos in touched.chain(level.slots.keys().map(|&(pos, _)| pos)) {
                    if super::slot_changes::block_menu_viewed(&host.hub.guests, pos)
                        && !viewed.contains(&pos)
                    {
                        viewed.push(pos);
                    }
                }
            }
            viewed.sort_unstable();
            for pos in viewed {
                let (kept, slots) = (&level.block_entities, &level.slots);
                self.notify_block_menu_slot_changed(pos, |game| {
                    game.committed_block_items_before(kept, slots, pos)
                });
            }
        }
        for (pos, kept) in level.block_entities {
            let record = |entity: Option<&BlockEntity>| match entity {
                Some(BlockEntity::Jukebox(j)) => j.record().cloned(),
                _ => None,
            };
            let was = record(kept.entity.as_ref());
            let now = record(self.common.world.block_entities.get(&pos));
            if was == now {
                continue;
            }
            // The song of the record it had stops; the one it has now starts.
            let pos = [pos.0, pos.1, pos.2];
            if was.is_some() {
                self.jukebox_stopped(pos);
            }
            if now.is_some() {
                self.jukebox_inserted(pos);
            }
        }
        for after in menu_lifecycle {
            self.server_tx_run_after(after);
        }
    }
    pub(in crate::game) fn server_tx_roll_back(&mut self, mut level: Level) {
        self.common.roll_back_in_flight(&mut level);
        self.common
            .scratch_roll_back(std::mem::take(&mut level.scratch_kept));
        // Discard deferred effects before undoing tentative action storage.
        level.after.clear();
        // Restore slots and undo only their recorded modifier edits. Explicit
        // attribute writes survive an abort, and no item query runs here.
        let equipment = std::mem::take(&mut level.equipment);
        for ((dim, id), kept) in equipment {
            let _ = self.with_effect_level(dim, |game| match kept {
                KeptEquipment::Mob { slots, gear } => {
                    if let Some(mob) = game.mobs.iter_mut().find(|m| m.id == id && !m.removed) {
                        mob.extra.equipment = *slots;
                        mob.extra.gear = gear;
                    }
                }
                KeptEquipment::Stand(slots) => {
                    if let Some(stand) = game.armor_stands.iter_mut().find(|a| a.id == id) {
                        stand.equipment = *slots;
                    }
                }
            });
        }

        for ((dim, id), kept) in level.mob_effects {
            let _ = self.with_effect_level(dim, |game| {
                if let Some(mob) = game.mobs.iter_mut().find(|mob| mob.id == id) {
                    mob.extra.effects = kept.active;
                    mob.extra.effect_chains = kept.chains;
                    mob.extra.ambient_effects = kept.ambient;
                    mob.extra.hidden_effects = kept.hidden;
                    mob.effects_dirty = kept.dirty;
                }
            });
        }
        for ((dim, id), kept) in level.effect_modifiers.into_iter().rev() {
            let _ = self.with_effect_level(dim, |game| {
                if let Some(mob) = game.mobs.iter_mut().find(|mob| mob.id == id) {
                    kept.restore(mob);
                }
            });
        }
        for ((dim, id), changes) in std::mem::take(&mut level.modifier_changes) {
            let _ = self.with_effect_level(dim, |game| {
                let common = &mut game.common;
                if let Some(mob) = common.mobs.iter_mut().find(|m| m.id == id && !m.removed) {
                    super::living_attributes::undo_mob_modifiers(
                        &common.motion.attributes,
                        mob,
                        changes,
                    );
                }
            });
        }

        for ((x, y, z), tag) in level.saved_pots {
            sunrise_gameplay_common::game::spill_common::set_saved_entity(
                &mut self.common.world,
                [x, y, z],
                tag,
            );
        }
        // Block entities whose published shapes the abort changed back.
        let mut reshaped = Vec::new();
        for (pos, kept) in level.block_entities {
            let current = self.common.world.block_entities.generation(&pos);
            let replaced = current != kept.generation;
            if replaced && current.is_some() {
                self.common.world.block_entities.remove(&pos);
            }
            if let (Some(BlockEntity::Mod(entity)), Some(fields)) =
                (&kept.entity, kept.client_update_fields)
            {
                entity.set_client_update_fields(fields);
            }
            match (kept.entity, self.common.world.block_entities.get_mut(&pos)) {
                (Some(entity), Some(now)) => {
                    fn shapes(
                        entity: &BlockEntity,
                    ) -> Option<&sunrise_level::containers::PublishedShapes> {
                        match entity {
                            BlockEntity::Mod(entity) => entity.shapes(),
                            _ => None,
                        }
                    }
                    if shapes(now) != shapes(&entity) {
                        reshaped.push(pos);
                    }
                    *now = entity;
                    self.common
                        .world
                        .block_entities
                        .restore_generation(pos, kept.generation.unwrap());
                }
                // One this transaction removed comes back.
                (Some(entity), None)
                    if replaced || level.removed_block_entities.contains_key(&pos) =>
                {
                    if matches!(&entity, BlockEntity::Mod(entity) if entity.shapes().is_some()) {
                        reshaped.push(pos);
                    }
                    self.restore_removed_block_entity(pos, entity, kept.generation.unwrap());
                }
                (None, None) if replaced && let Some(taken) = kept.taken => {
                    let entities = &mut self.common.world.block_entities;
                    entities.restore_taken_lease(pos, taken);
                    entities.reattach_retired(pos, kept.generation.unwrap());
                }
                _ => {}
            }
            self.common.world.block_entities.put_back_attachments(
                pos,
                kept.generation,
                kept.attachments,
            );
            match kept.name {
                Some(name) => self.common.world.block_names.insert(pos, name),
                None => self.common.world.block_names.remove(&pos),
            };
            // Restoring saved metadata is not a new write. The ordinary storage
            // setter marks its chunk unsaved, including when the extras are empty.
            let chunk = (pos.0 >> 4, pos.2 >> 4);
            let mark = self.common.world.modified.mark(chunk);
            self.common.world.set_block_entity_extras(pos, kept.extras);
            self.common.world.modified.restore(chunk, mark);
            if let Some(observer) = self.common.world.block_entities.identity(pos)
                && Some(observer.generation) == kept.generation
            {
                self.common
                    .world
                    .block_entities
                    .contents_watches
                    .restore(observer, kept.contents_watches);
            }
            match kept.hopper {
                Some(hopper) => self.server.hoppers.insert(pos, hopper),
                None => self.server.hoppers.remove(&pos),
            };
        }
        // After the whole entities: a slot kept here was kept before any whole capture of its
        // entity at this depth, so its stack is the older one. Only into the block entity it
        // was taken from: one a write replaced (a `set_block` in the transaction) is left as is.
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
                #[cfg(any(test, feature = "test-support"))]
                if let Some(trace) = &self.common.transactions.trace {
                    trace
                        .lock()
                        .unwrap()
                        .push(format!("restore:{},{},{}:{slot}", pos.0, pos.1, pos.2));
                }
            }
        }
        for ((dim, (qx, qy, qz)), biome) in level.biomes {
            let _ = self.with_effect_level(dim, |game| {
                let bottom_quart = game.world.bottom_y() >> 2;
                if let Some(chunk) = game.world.chunks.get_mut(&(qx >> 2, qz >> 2)) {
                    std::sync::Arc::make_mut(chunk).set_biome_cell(
                        qx & 3,
                        qy - bottom_quart,
                        qz & 3,
                        biome,
                    );
                }
            });
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
        if let Some(random) = level.level_random {
            *self.common.world.random = random;
        }
        if let Some(sequences) = level.loot_sequences {
            self.server.rand_sequences = sequences;
        }
        for (owner, before) in level.parked_level_randoms {
            if let Some(random) = self.level_random_mut(owner) {
                *random = before;
            }
        }
        // The shapes are back: the neighbours that read them (a fluid beside it) are told, as
        // the publish told them.
        for pos in reshaped {
            let at = BlockPos::new(pos.0, pos.1, pos.2);
            let state = self.common.world.block(pos.0, pos.1, pos.2);
            self.with_level(|b, level, ticks| {
                let block = b.block(state);
                b.update_neighbors_at(level, ticks, at, block);
            });
        }
        for (chunk, randoms) in level.composter_randoms {
            self.common
                .transactions
                .composter_randoms
                .entry(chunk)
                .or_default()
                .extend(randoms);
        }
        for ((dim, id), kept) in level.entities {
            let _ = self.with_effect_level(dim, |game| {
                game.edit_entity_slots(id, |slots, _| {
                    for (slot, kept) in slots.iter_mut().zip(kept) {
                        *slot = kept;
                    }
                });
            });
        }
        for ((dim, id), len) in level.mob_lengths {
            let _ = self.with_effect_level(dim, |game| {
                if let Some(m) = game.mobs.iter_mut().find(|m| m.id == id) {
                    m.extra.items.truncate(len);
                }
            });
        }
        for (join, kept) in level.item_entities {
            if let Some(index) = self.common.dropped.locate(join) {
                kept.restore(&mut self.common.dropped[index]);
            }
        }
        let general = &mut self.common.resident_entities.general.rows;
        for (join, kept) in level.general_entities {
            if let Some(index) = general.locate(join) {
                let row = &mut general[index];
                if let Some(velocity) = kept.velocity {
                    row.velocity = velocity;
                }
                if let Some(rotation) = kept.rotation {
                    row.rotation = rotation;
                }
                if let Some(fall_distance) = kept.fall_distance {
                    row.fall_distance = fall_distance;
                }
            }
        }
        for ((dim, join), kept) in level.orbs {
            let _ = self.with_effect_level(dim, |game| {
                if let Some(i) = game.orbs.locate(join) {
                    let orb = &mut game.orbs[i];
                    (orb.value, orb.count, orb.age) = kept;
                }
            });
        }
        for (window, drag) in level.mount_drags {
            self.restore_mount_drag(window, drag);
        }
        if let Some(host) = self.server.lan.as_mut() {
            for ((conn, owner), kept) in level.stations {
                if let Some(menus) = host.menus.get_mut(&conn)
                    && menus.station.id() == owner
                    && menus
                        .station
                        .as_ref()
                        .is_some_and(|s| s.window == kept.window)
                {
                    menus.station.restore(Some(kept));
                }
            }
            for (generation, kept) in level.menu_clicks {
                if let Some(i) = host.hub.guests.connection_index(kept.conn)
                    && host.hub.guests[i].menu_generation == generation
                    && host.hub.guests[i].menu.is_some()
                {
                    host.hub.guests.update(i, |g| {
                        let menu = g.menu.as_mut().unwrap();
                        menu.drag = kept.drag;
                        menu.craft = kept.craft;
                        menu.slot_changed_seen = kept.slot_changed_seen;
                    });
                    let owner = host.menus.entry(kept.conn).or_default();
                    owner.owned_menu = kept.owned;
                    owner.crafting.machine.restore(kept.machine);
                    owner.slot_places = kept.places;
                }
            }
            // A removed window's snapshot is now held by its pending close notice, not
            // by the player. Never restore that window's stacks into a replacement.
            use sunrise_sim::lan::menu::NativeMenuEvent;
            if !level.native_menu_slots.is_empty() {
                for event in &mut host.hub.native_menu_events {
                    if let NativeMenuEvent::Closed {
                        generation,
                        owned_slots,
                        ..
                    } = event
                        && !owned_slots.is_empty()
                        && let Some(before) = level.native_menu_slots.remove(generation)
                    {
                        *owned_slots = before;
                    }
                }
            }
            host.hub.guests.update_all(|g| {
                if let Some(kept) = level.menus.get(&g.conn).filter(|_| g.placed) {
                    g.craft_grid.clone_from(&kept.craft_grid);
                }
                if g.placed
                    && g.menu.is_some()
                    && let Some(before) = level.native_menu_slots.remove(&g.menu_generation)
                {
                    g.native_menu = before;
                }
            });
        }
        for (key, kept) in level.attachments {
            match key {
                AttachmentKey::Level(dim, holder) => {
                    let _ = self.with_effect_level(dim, |game| {
                        if let Ok(map) =
                            game.attachment_slot(holder, mod_api::AttachmentAccess::Existing)
                        {
                            map.replace_contents(kept.contents);
                            if !kept.indexed {
                                map.unbind_index();
                            }
                        }
                        if !kept.indexed
                            && let Holder::Entity(id) = holder
                            && let Some(id) = std::num::NonZeroU32::new(id)
                        {
                            game.with_resident_slot(&dim.key(), id, |slot| {
                                *slot = Default::default();
                            });
                        }
                    });
                }
                AttachmentKey::Player(body) => {
                    self.restore_player_attachments(body, kept);
                }
            }
        }
        self.common
            .restore_travelled(std::mem::take(&mut level.travelled));
        // After the maps: putting a chunk's map back marks it again.
        for (chunk, mark) in level.chunk_marks {
            self.common.world.modified.restore(chunk, mark);
        }
        for ((dim, id), kept) in level.mob_health {
            let _ = self.with_effect_level(dim, |game| {
                if let Some(mob) = game.mobs.iter_mut().find(|m| m.id == id) {
                    mob.health = kept.health;
                    mob.absorption = kept.absorption;
                    mob.needs_sync = kept.needs_sync;
                    mob.death_time = kept.death_time;
                    mob.death_canceled = kept.death_canceled;
                    if kept.processed {
                        game.server.died.insert(id);
                    } else {
                        game.server.died.remove(&id);
                    }
                }
            });
        }
        for (body, kept) in level.player_health {
            // Restoration must reach that exact player body even when tentatively set to zero
            // health or when another level holds a fake player sharing the same UUID.
            let target = if self.acting_player_body() == body {
                Some(sunrise_gameplay_common::game::commands_common::Target::Player)
            } else if let PlayerBodyKey::Guest(conn) = body {
                Some(sunrise_gameplay_common::game::commands_common::Target::Guest(conn))
            } else {
                None
            };
            if let Some(target) = target {
                self.for_player(target, |g| {
                    if let Some(player) = g.player.as_mut() {
                        player.health.data = kept.data;
                        player.events.truncate(kept.events_len);
                        player.health.dead = kept.dead;
                    }
                    if let Some((conn, canceled, time)) = kept.guest
                        && let Some(host) = g.server.lan.as_mut()
                        && let Some(gi) = host.hub.guests.connection_index(conn)
                    {
                        host.hub.guests.update(gi, |guest| {
                            guest.death_canceled = canceled;
                            guest.death_time = time;
                        });
                    }
                });
            }
        }
        crate::game::server_side::decision_engine_server::rollback_in(level.actions, self);
        crate::game::server_side::lan_trade_decisions_server::rollback_trade_journal(
            level.trades,
            self,
        );
        if let Some(saved) = level.board {
            self.common.score.board.restore(saved);
            self.sync_score_registries();
        }
        for (owner, kept) in level.ender_items {
            match self.player_storage_home(owner) {
                Some(PlayerStorageHome::Game) => self.server.ender_items = kept,
                Some(PlayerStorageHome::Guest(index)) => {
                    let host = self.server.lan.as_mut().expect("resolved guest");
                    host.hub.guests.update(index, |g| g.ender_items = kept);
                }
                None => {}
            }
        }
        for (owner, kept) in level.player_extra {
            match self.player_storage_home(owner) {
                Some(PlayerStorageHome::Game) => self.common.level.player_extra = kept,
                Some(PlayerStorageHome::Guest(index)) => {
                    let host = self.server.lan.as_mut().expect("resolved guest");
                    host.hub.guests.update(index, |g| g.saved = kept);
                }
                None => {}
            }
        }
        for ((owner, book), (_, kept)) in level.player_books {
            let key = book.as_str().to_owned();
            match self.player_storage_home(owner) {
                Some(PlayerStorageHome::Game) => {
                    self.common.level.recipe_book.custom.restore(key, kept);
                }
                Some(PlayerStorageHome::Guest(index)) => {
                    self.server
                        .lan
                        .as_mut()
                        .expect("resolved guest")
                        .hub
                        .guests
                        .update(index, |g| {
                            g.recipe_book.custom.restore(key, kept);
                        });
                }
                None => {}
            }
        }
        for (owner, kept) in level.players {
            match self.player_storage_home(owner) {
                Some(PlayerStorageHome::Game) => {
                    self.common.inventory = kept.inventory;
                    self.common.armor = kept.armor;
                    self.common.offhand = kept.offhand;
                    self.common.equipment = kept.equipment;
                    self.common.carried = kept.carried;
                }
                Some(PlayerStorageHome::Guest(index)) => {
                    let host = self.server.lan.as_mut().expect("resolved guest");
                    host.hub.guests.update(index, |g| {
                        g.inventory = kept.inventory;
                        g.armor = kept.armor;
                        g.offhand = kept.offhand;
                        g.equipment = kept.equipment;
                        g.carried = kept.carried;
                    });
                }
                None => {}
            }
        }
    }

    /// Identity of the player body currently swapped into `Game`.
    pub(in crate::game) fn acting_player_body(&self) -> PlayerBodyKey {
        if let Some(conn) = self.server.lan_acting {
            return PlayerBodyKey::Guest(conn);
        }
        if let Some(uuid) = self.server.lan_acting_uuid
            && let Some(conn) = self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.holder_guest(uuid, self.common.dim.id()))
                .map(|guest| guest.conn)
        {
            return PlayerBodyKey::Guest(conn);
        }
        PlayerBodyKey::Host
    }

    /// Resolves which player body `attachment_slot(Holder::Player(uuid), _)` addresses now:
    /// the acting body when its UUID matches, else the exact guest `conn` resolved in `self.dim`
    /// (online real player across dimensions, or the per-level fake player in `self.dim`).
    pub(in crate::game) fn attachment_player_body(&self, uuid: u128) -> Option<PlayerBodyKey> {
        if self.server.lan_acting_uuid == Some(uuid) {
            return Some(self.acting_player_body());
        }
        self.attachment_player_connection(uuid)
            .map(PlayerBodyKey::Guest)
    }

    /// Restore a player body's attachment map directly wherever that exact body's state lives
    /// now (acting body in `player_state` or parked guest `conn` in `lan.hub.guests`), without
    /// looking up by UUID across unrelated per-level fake players.
    pub(in crate::game) fn restore_player_attachments(
        &mut self,
        body: PlayerBodyKey,
        kept: KeptAttachments,
    ) {
        let map = if self.acting_player_body() == body {
            Some(&mut self.common.player_state.attachments)
        } else if let PlayerBodyKey::Guest(conn) = body
            && !self.server.lan_guest_swaps.contains(&conn)
        {
            self.server
                .lan
                .as_mut()
                .and_then(|host| host.hub.guests.attachments_mut(conn))
        } else {
            None
        };
        if let Some(map) = map {
            map.replace_contents(kept.contents);
            if !kept.indexed {
                map.unbind_index();
            }
        }
    }

    pub(in crate::game) fn server_tx_run_after(&mut self, after: After) {
        match after {
            After::BlockEntityRemoved(effects) => self.apply_removed_block_entity_effects(effects),
            After::DispenseEvent(id, (x, y, z), data) => {
                self.server_level_event_at(id, [x, y, z], data)
            }
            After::Payload(payload) => payload.publish(),
            After::Packet(..) => {
                debug_assert!(!self.is_logical_server(), "After::Packet is client-only");
            }
            // Only reached through a root commit, which replays blocks and connection groups in order.
            After::Block(_) | After::BulkBlocks(..) | After::ConnectionGroup(..) => {}
            After::RecipeBookSettings(owner, player, book, before) => {
                let key = book.as_str();
                let flags = match self.player_storage_home(owner) {
                    Some(PlayerStorageHome::Game) => self.common.level.recipe_book.custom.get(key),
                    Some(PlayerStorageHome::Guest(index)) => {
                        self.server.lan.as_ref().expect("resolved guest").hub.guests[index]
                            .recipe_book
                            .custom
                            .get(key)
                    }
                    None => return,
                };
                if flags != before.unwrap_or_default() {
                    self.publish_player_book_settings(player, book, flags);
                }
            }
            After::PlayerRespawn(conn) => self.commit_player_respawn(conn),
            After::Trade(effect) => self.commit_merchant_effect(*effect),
            After::Composter(effects) => self.commit_composter_effects(effects),
            After::FluidWrite(written) => self.commit_fluid_write(*written),
            After::FluidEffects(effects) => self.commit_fluid_effects(*effects),
            After::DispenserDrop {
                pos,
                facing,
                spawn,
                play_events,
            } => {
                self.commit_dispensed(pos, facing, *spawn, play_events);
            }
            After::DispenserFeedback { pos, facing, event } => {
                self.commit_dispense_feedback(pos, facing, event);
            }
            After::LevelWrite(write) => self.commit_level_write(*write),
            After::PlacementEffect(effect) => self.publish_placement_effect(*effect),
            After::WitherPattern(found, cells) => {
                let _ = self.finish_wither_pattern(found, &cells);
            }
            After::UpdatePlacedBlockEntity(pos, generation) => {
                if self.common.world.block_entities.generation(&pos) == Some(generation) {
                    self.block_entity_updated(pos);
                }
            }
            After::BlockGameEvent(id, pos, who, state) => {
                self.post_block_game_event(id, pos, who, Some(state));
            }
            After::DestroyBlockGameEvent {
                pos,
                state,
                breaker,
            } => {
                self.post_destroy_block_game_event(pos, state, breaker);
            }
            After::BlockDropSpawns(items, orbs) => self.publish_block_spawns(items, orbs),
            After::Changed(pos) => self.container_changed(pos),
            After::ChangedWithoutHook(pos) => self.container_changed_without_hook(pos),
            After::ChangedHolder { holder, silent } => {
                if !silent && let Some(queue) = &mut self.common.transactions.quick_move_notices {
                    queue.push(QuickMoveNotice {
                        dim: self.common.dim,
                        holder,
                    });
                } else {
                    self.publish_holder_changed(holder, silent);
                }
            }
            After::MenuOpen(conn, identity, open) => {
                let open = open
                    .downcast::<crate::game::server_side::native_menus::PendingMenuOpen>()
                    .unwrap_or_else(|_| panic!("invalid staged menu open payload"));
                self.publish_menu_open(conn, identity, open);
            }
            After::GuestMenu(conn, effects) => self.publish_guest_menu_effects(conn, *effects),
            After::PersonalSidebar(conn, contents) => {
                if contents.is_some() {
                    self.load_scoreboard();
                }
                let board = &self.common.score.board;
                if let Some(host) = &mut self.server.lan {
                    host.hub.update_guest(conn, |guest| {
                        guest.set_personal_sidebar(board, contents.map(|contents| *contents));
                    });
                }
            }
            After::MountEquipment {
                dim,
                id,
                at,
                sound,
                equips,
            } => {
                let _ = self.with_effect_level(dim, |game| {
                    if let Some((sound, center)) = sound {
                        assert!(
                            game.is_logical_server(),
                            "After::MountEquipment is server-only"
                        );
                        game.server_play_sound(&sound, center, 1.0, 1.0);
                    }
                    game.mob_equip_event(id, at, equips);
                });
            }
            After::MenuClick(reply) => self.publish_menu_click(*reply),
            After::MenuPlacesNotice(conn, uuid, window) => {
                self.queue_menu_places_notice(conn, uuid, window);
            }
            After::FurnaceMenu(effects) => self.publish_furnace_menu_effects(*effects),
            After::StoreCartItems(pos) => self.common.store_cart_items(pos),
            After::BackedMenuClick(conn, uuid, menu, window) => {
                self.publish_backed_menu_click(conn, uuid, menu, window);
            }
            After::Capabilities(pos) => self.common.world.capability_generations.invalidate(pos),

            After::BreakProgress(breaker, pos, progress) => {
                self.server_destroy_block_progress(breaker, pos, progress);
            }
            After::Biome(key) => {
                self.commit_biome_columns(&std::collections::HashSet::from([key]), &[key]);
            }
            After::Effect(effect) => match effect {
                DeferredEffect::CustomStat {
                    player,
                    key,
                    amount,
                } => {
                    self.native_as_player(player, |g| g.award_custom_stat(key, amount));
                }
                DeferredEffect::ItemStat {
                    player,
                    stat,
                    item,
                    amount,
                } => {
                    self.native_as_player(player, |g| g.award_stat(stat, &item, amount));
                }
                DeferredEffect::Sound {
                    sound,
                    at,
                    volume,
                    pitch,
                } => {
                    assert!(
                        self.is_logical_server(),
                        "DeferredEffect::Sound is server-only"
                    );
                    self.server_play_sound(sound, at, volume, pitch);
                }
                DeferredEffect::RestoreStatistics(player, values) => {
                    self.restore_player_statistics(player, *values);
                }
                DeferredEffect::ChangeAdvancement {
                    player,
                    id,
                    criterion,
                    grant,
                } => {
                    self.apply_deferred_advancement_change(
                        player,
                        &id,
                        criterion.as_deref(),
                        grant,
                    );
                }
            },
            After::Drop(pos, stack) => {
                self.spawn_item(stack, pos.map(|p| f64::from(p) + 0.5), [0.0; 3], 10);
            }
            After::GivePickup {
                uuid,
                dim,
                prototype,
            } => {
                self.native_as_player(uuid, |g| {
                    let back = g.enter_command_level(dim);
                    if g.dim == dim {
                        g.give_pickup(prototype);
                    }
                    if let Some(back) = back {
                        g.enter_command_level(back);
                    }
                });
            }
            After::InventoryTrigger(uuid) => {
                self.native_as_player(uuid, |g| g.check_inventory_trigger());
            }
            After::GrindstoneEvent(pos) => self.server_level_event_at(1042, pos, 0),
            After::StationContent(conn, owner, window) => {
                if self
                    .server
                    .lan
                    .as_ref()
                    .and_then(|host| host.menus.get(&conn))
                    .is_some_and(|menus| {
                        menus.station.id() == owner
                            && menus.station.as_ref().is_some_and(|s| s.window == window)
                    })
                {
                    self.lan_send_station(conn);
                }
            }
            After::GiveRest {
                uuid,
                rest,
                owner,
                dim,
                eye,
            } => {
                let mut rest = Some(rest);
                self.native_as_player(uuid, |g| {
                    let back = g.enter_command_level(dim);
                    if let Some(rest) = rest.take() {
                        g.give_remainder(rest, owner);
                    }
                    if let Some(back) = back {
                        g.enter_command_level(back);
                    }
                });
                if let Some(rest) = rest {
                    let back = self.enter_command_level(dim);
                    self.give_remainder_unheld(rest, owner, eye);
                    if let Some(back) = back {
                        self.enter_command_level(back);
                    }
                }
            }
            After::PlayerEquip(uuid, slot, stack) => {
                self.native_as_player(uuid, |g| {
                    g.player_equipped(&slot, stack.as_ref());
                    g.apply_equipment();
                });
            }
            After::PlayerDrop(uuid, stack) => {
                self.native_as_player(uuid, |g| {
                    g.drop_plain(stack);
                });
            }
            After::Spawn(at, stack) => self.spawn_item(stack, at, [0.0; 3], 10),
            After::Emptied(id) => {
                let emptied = self
                    .common
                    .dropped
                    .iter()
                    .position(|e| e.id == id && e.stack.count == 0);
                if let Some(i) = emptied {
                    self.remove_dropped(i);
                }
            }
            After::EquipmentSync(id) => {
                if let Some(mob) = self
                    .common
                    .mobs
                    .iter_mut()
                    .find(|m| m.id == id && !m.removed)
                {
                    mob.needs_sync = true;
                }
            }
            After::Equip {
                at,
                sound,
                equips,
                who,
            } => {
                let ctx = self.who_context(Some(sunrise_sim::mobs::Who::Mob(who)), None);
                self.equip_item_effects(at, sound, equips, ctx);
            }
        }
    }

    /// What `journal` left for the root commit: the joins it held, then its removals, in order,
    /// then the leaves it held. An entity added and removed in it is never announced, so it never
    /// leaves. What the listeners ask for meanwhile waits in the commit's queue and is told after
    /// it, in the order it was asked for; a transaction a listener commits tells through the
    /// running commit too.
    /// `effects` is what the root transaction's other deferred effects do (`root_commit`): it runs
    /// with the commit in place, so a join it causes waits behind the commit's own batches.
    pub(in crate::game) fn commit_entity_journal(
        &mut self,
        journal: entities::EntityJournal,
        effects: impl FnOnce(&mut Self),
    ) {
        let removed: Vec<u32> = (journal.removals.iter())
            .map(|removal| match removal {
                entities::Removal::Mob { id, .. } => *id,
                entities::Removal::Other(entity) | entities::Removal::Kill(entity) => entity.id(),
            })
            .collect();
        let dimension = self.common.dim;
        let steps = entities::Step::of(journal.announcements, journal.removals);
        if let Some(commit) =
            entities::committing_in(&mut self.common.transactions.committing, dimension)
        {
            commit.removed.extend(removed);
            commit.queue.extend(steps);
            effects(self);
            return;
        }
        let commit = entities::Committing {
            dimension,
            removed,
            queue: steps,
            removing: None,
        };
        let frame = self.common.transactions.committing.len();
        self.common.transactions.committing.push(commit);
        let telling = Telling { game: self, frame };
        effects(telling.game);
        telling.game.tell_commit(frame);
    }

    /// Entity `id` is leaving this level with `map` inside a transaction: its index rows stay
    /// until the root closes, so a snapshot inside the action sees no gap.
    pub(in crate::game) fn defer_departure(
        &mut self,
        id: u32,
        map: Option<&mut sunrise_mod_runtime::attachments::Attachments>,
    ) {
        use crate::game::entity_residency::SlotAccess;
        let lifetime = std::num::NonZeroU32::new(id)
            .map(|id| self.with_resident_slot(&self.common.dim.key(), id, std::mem::take))
            .and_then(|access| match access {
                SlotAccess::Done(lifetime) => Some(lifetime),
                _ => None,
            })
            .unwrap_or_default();
        let registration = map.and_then(|map| map.take_index_binding());
        if let Some(level) = self.common.transactions.last_mut() {
            level.departed.push(entities::Departure {
                lifetime,
                registration,
            });
        }
    }
    /// The step `next` of the queue of the commit at `frame`, with the entities never to join. The
    /// removals of a step are taken when `run`, so they run once.
    fn commit_step(
        &mut self,
        frame: usize,
        next: usize,
        run: bool,
    ) -> Option<(entities::Step, Vec<u32>)> {
        let commit = self.common.transactions.committing.get_mut(frame)?;
        let step = match commit.queue.get_mut(next)? {
            entities::Step::Tell(announcement) => entities::Step::Tell(*announcement),
            entities::Step::Remove(removals) if run => {
                entities::Step::Remove(std::mem::take(removals))
            }
            entities::Step::Remove(_) => entities::Step::Remove(Vec::new()),
        };
        Some((step, commit.removed.clone()))
    }
    /// Publish an item spawned by `preRemoveSideEffects` before returning to the block
    /// write. At commit, tell the already-queued batches first, excluding pending removes.
    pub(in crate::game) fn flush_removal_join(&mut self, watch: u32) -> bool {
        let Some(frame) = self
            .common
            .transactions
            .committing
            .iter()
            .rposition(|commit| commit.dimension == self.common.dim)
        else {
            return self.native_joins_flush(false, watch);
        };
        self.tell_commit_joins(frame);
        let removed = self.common.transactions.committing[frame].removed.clone();
        self.flush_joins(false, watch, &removed, Some(JoinSeq::latest()))
    }

    fn tell_commit_joins(&mut self, frame: usize) -> usize {
        let mut next = 0;
        while let Some((step, removed)) = self.commit_step(frame, next, false) {
            next += 1;
            if let entities::Step::Tell(announcement) = step
                && announcement.is_join()
            {
                self.tell_announcement(announcement, &removed);
            }
        }
        next
    }

    /// Tells every join first, as the queue grows, then the remaining steps in order.
    fn tell_commit(&mut self, frame: usize) {
        let joined = self.tell_commit_joins(frame);
        let mut next = 0;
        while let Some((step, removed)) = self.commit_step(frame, next, true) {
            match step {
                entities::Step::Tell(announcement) if next < joined && announcement.is_join() => {}
                entities::Step::Tell(announcement) => {
                    self.tell_announcement(announcement, &removed)
                }
                entities::Step::Remove(removals) => self.remove_committed(removals),
            }
            next += 1;
        }
    }
    fn tell_announcement(&mut self, announcement: entities::Announcement, removed: &[u32]) {
        match announcement {
            entities::Announcement::Joins {
                loaded_from_disk,
                through,
            } => {
                self.flush_joins(loaded_from_disk, 0, removed, Some(through));
            }
            entities::Announcement::FreshJoin { handle, id } => {
                if !removed.contains(&id) {
                    self.admit_fresh_join(handle, id);
                }
            }
            entities::Announcement::Left(id) => self.entity_left_now(id),
            entities::Announcement::Leaves => {
                self.native_removals_flush();
                self.leaves_now();
            }
        }
    }

    /// Marks `id` as the entity being removed in the commit of the level in place; the mark it
    /// replaced.
    fn set_removing(&mut self, id: Option<u32>) -> Option<u32> {
        let dimension = self.common.dim;
        let commit = entities::committing_in(&mut self.common.transactions.committing, dimension)?;
        std::mem::replace(&mut commit.removing, id)
    }
    /// Runs the removals a commit holds, as a removal outside a transaction does.
    fn remove_committed(&mut self, removals: Vec<entities::Removal>) {
        for removal in removals {
            match removal {
                entities::Removal::Mob { id, killed } => {
                    if killed {
                        self.mob_removal_effects(id);
                    }
                    self.finish_mob_removal(id);
                }
                entities::Removal::Other(entity) | entities::Removal::Kill(entity) => {
                    let outer = self.set_removing(Some(entity.id()));
                    if let Some(target) = self.native_entity_target(entity) {
                        if matches!(removal, entities::Removal::Kill(_)) {
                            self.kill_target_in_level(target);
                        } else {
                            self.remove_target_now(entity, target);
                        }
                    }
                    self.set_removing(outer);
                }
            }
        }
    }
    /// Undoes `journal`'s entity changes, after the rest of its transaction rolled back: the
    /// removed mobs back, the entities it added out. Its arrivals come back.
    pub(in crate::game) fn roll_back_entity_journal(
        &mut self,
        journal: entities::EntityJournal,
    ) -> HashSet<u32> {
        for (id, was) in journal.mob_removed {
            for mob in self.common.mobs.iter_mut().filter(|m| m.id == id) {
                mob.removed = was;
            }
        }
        self.common.potions.thrown.truncate(journal.potions);
        // One load when nothing joined anywhere since the transaction opened.
        if JoinSeq::latest() >= journal.mark {
            self.discard_joined_since(journal.mark, &journal.arrivals);
        }
        // After the discard: forgetting an aborted entity reads nothing keyed by position.
        for kept in journal.anon {
            self.common.persist.put_back_anon(kept);
        }
        journal.arrivals
    }
    /// Takes every entity that joined at `mark` or later out of its list, but the `arrivals`,
    /// with no hook (as a join a mod cancels), and forgets what was kept of it.
    fn discard_joined_since(&mut self, mark: JoinSeq, arrivals: &HashSet<u32>) {
        use super::entity::{Entity as _, with_entity};
        let added = |id: u32, join: JoinSeq| join >= mark && !arrivals.contains(&id);
        let mut gone = Vec::new();
        for handle in self.entity_handles_rev() {
            let (id, join) = with_entity!(self, handle, e => (e.id(), e.join_seq()));
            if added(id, join) {
                self.discard_entity(handle);
                gone.push(id);
            }
        }
        macro_rules! apart {
            ($($list:expr),* $(,)?) => {$(
                $list.retain(|e| {
                    let out = added(e.id, e.join_seq());
                    if out {
                        gone.push(e.id);
                    }
                    !out
                });
            )*};
        }
        apart!(
            self.crystals,
            self.cushions,
            self.paintings,
            self.item_frames,
            self.server.others.0,
            self.breath_clouds,
            self.bolts,
            self.fangs.list,
            self.ominous_spawners,
            self.resident_entities.knots,
        );
        if let Some(id) = (self.common.dragon.as_ref())
            .filter(|dragon| added(dragon.id, dragon.join_seq()))
            .map(|dragon| dragon.id)
        {
            self.discard_resident_dragon();
            gone.push(id);
        }
        if let Some(id) = (self.common.hook.as_ref())
            .filter(|hook| added(hook.id(), hook.join_seq()))
            .map(|hook| hook.id())
        {
            self.common.hook.discard();
            gone.push(id);
        }
        (self.common.native_lifecycle.pending_item_spawns).retain(|spawn| !gone.contains(&spawn.0));
        for id in gone.into_iter().filter(|&id| id != 0) {
            self.forget_entity(id);
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl crate::game::ServerGame<'_> {
    pub fn test_ender_items(&self) -> &[Option<sunrise_items::items::ItemStack>] {
        &self.server.ender_items
    }
    pub fn test_lan_guest_swaps(&self) -> &[u32] {
        &self.server.lan_guest_swaps
    }
    pub fn test_clear_hopper(&mut self, pos: (i32, i32, i32)) {
        self.server.hoppers.remove(&pos);
    }
    pub fn test_attachment_slot(
        &mut self,
        holder: mod_api::Holder,
        access: mod_api::AttachmentAccess,
    ) -> std::result::Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        self.attachment_slot(holder, access)
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::player_equip::PlayerEquipGameExt2 as _;

use sunrise_gameplay_common::game::resident_state::ResidentStateGameExt1 as _;
