//! `host.transaction` (#41 §7): NeoForge 26.3's `TransactionContext` and `SnapshotJournal`, kept by
//! the game at the storage level, so a mod's handlers hold no state and write no journal.
//!
//! A capability write applies in place and keeps its storage's first before-image per depth.
//! Game-held vanilla containers defer implicit changed/comparator effects to root commit;
//! nested commits hand their snapshots and waiting work to the parent. An abort restores
//! storage quietly and discards those commit effects. Raw mod-owned storage writes and
//! restores synthesize no notice. Explicit notifications a mod issues run immediately and
//! survive abort: the journal restores storage, not observer calls or scheduled comparator
//! ticks that already happened.
//!
//! Covered here: block entities (item and data slots, saved compound, queued client update fields,
//! custom name, lock, loot roll, attachments), a hopper's cooldown and cauldron and composter
//! block states. The attachment map of every other holder (an entity, a player, a chunk,
//! the level) is kept on the first access to it through `attachment_map`.
//! A block's tentative state is stored only (`World::set_stored`): the
//! one real write, with its remesh, relight and logs, is at the root commit. A jukebox is
//! told at the root commit from its record before and after, so a disc swapped in one
//! transaction changes the song. A player's inventory (slots, armour, off hand, cursor) is kept
//! whole on its first write, from wherever the player's state lives then: the acting fields
//! or a parked guest's record. On abort, the journal follows that storage through guest turns.
//! An item a write drops from a player waits for
//! the root commit too. Item entities keep their stack, pickup delay, target and velocity on
//! first write. Extraction removes a drained item at root commit if it is still empty.
//! `ItemEntity.setItem` leaves an empty item for the tick to discard. General entity views
//! keep velocity, rotation and fall distance on first write. A guest's menu storage (ender chest,
//! crafting grid, a mod's chest menu) is kept whole on its first write through a menu slot; a
//! crafting grid changed by the transaction has its result made again at the root commit.
//! `LivingRef::set_health` keeps the player's synced float storage and unpublished update
//! events, or the mob's health and sync dirtiness. This does not journal other health,
//! damage, death or attribute operations. Mob effect writes keep their effect lists, dirty bit,
//! health and absorption, and the attribute modifier rows the effects own.
//! An entity added inside one goes at an abort, one removed is only marked until the root
//! commit, and mods hear of joins and leaves at the root commit (`transactions/entities.rs`).
use super::{BlockEntity, BlockStateId, Dim, lan_menu_data::MountClickWindow, maps::MapData};
use crate::game::{GameGameExt1 as _, resident_state::ResidentStateGameExt1 as _};
use mod_api::{
    __private::TransactionFrame, LevelRef, PlayerRef, RecipeBookTypeKey, TransactionContext,
    TransactionLifecycle, TransactionScopeError,
};
use std::collections::{BTreeMap, HashMap};
use sunrise_core::{geometry::Vec3, position::BlockPos, random::LegacyRandom};
use sunrise_gameplay_common::hoppers::HopperState;
use sunrise_items::equipment::Equipment;
use sunrise_items::items::ItemStack;
use sunrise_level::{
    block_entities::BlockEntityIdentity,
    effects::EffectInstance,
    world::{BlockEntityExtras, ModifiedChunks, World},
};
use sunrise_mod_runtime::attachments::{AttachmentQueryScopes, Attachments, Holder};
use sunrise_sim::{
    living_attributes::ModifierChanges,
    menu::QuickCraftDrag,
    mobs::{StoredEffect, finalize::EffectModifierUndo, gear::Gear},
    recipe_book::BookFlags,
};
use sunrise_world::level::flags;

#[path = "transactions/decision.rs"]
pub mod decision;
#[path = "transactions/entities.rs"]
pub mod entities;
#[allow(unused_imports)]
pub use entities::{Announcement, ArrivalRollback};
#[path = "transactions/scratch.rs"]
pub mod scratch;

/// A notice names the container that received the write, not a replacement at its position.
pub struct QuickMoveNotice {
    pub dim: Dim,
    pub holder: BlockEntityIdentity,
}

pub(in crate::game) struct QuickMoveNotices<'a, 'g, S: crate::game::SideGame<'g>> {
    pub(in crate::game) game: &'a mut S,
    pub(in crate::game) outer: Option<Vec<QuickMoveNotice>>,
    resource: std::marker::PhantomData<fn(&'g ()) -> &'g ()>,
}

impl<'a, 'g, S: crate::game::SideGame<'g>> QuickMoveNotices<'a, 'g, S> {
    #[inline(always)]
    pub(in crate::game) fn new(game: &'a mut S, outer: Option<Vec<QuickMoveNotice>>) -> Self {
        Self {
            game,
            outer,
            resource: std::marker::PhantomData,
        }
    }
}

impl<'g, S: crate::game::SideGame<'g>> Drop for QuickMoveNotices<'_, 'g, S> {
    fn drop(&mut self) {
        self.game.transactions.quick_move_notices = self.outer.take();
    }
}

pub type Pos = (i32, i32, i32);

/// A level-local save follows that level's block writes. A transaction save follows the
/// dimension-keyed owner journal (e.g. an entity's equipment), even through a different level.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum SaveChunkScope {
    Level,
    Transaction,
}

/// The last pending value at this depth for a connection, menu generation and data index.
pub type MenuDataJournal = BTreeMap<(u32, u64, usize), i32>;
pub(in crate::game) type ComposterRandoms =
    HashMap<(mod_api::DimensionKey, (i32, i32)), HashMap<Pos, sunrise_core::random::LegacyRandom>>;

/// The open transactions, innermost last, and which close is running, if any: what
/// `transaction_lifecycle` reads.
#[derive(Default)]
pub struct OpenTransactions {
    pub levels: Vec<Level>,
    pub attachment_query_scopes: AttachmentQueryScopes,
    // A live, non-ZST address distinguishes journal instances.
    pub(in crate::game) tag: Option<Box<u8>>,
    pub(in crate::game) next_frame: u64,
    /// Live scratch rows; the frame opening a loan owns its ID.
    pub scratch: HashMap<u64, scratch::Scratch>,
    pub closing: Option<TransactionLifecycle>,
    /// Implicit container notices waiting for this quick move's source settlement.
    pub quick_move_notices: Option<Vec<QuickMoveNotice>>,
    /// How many writes the journals have been asked to keep, ever: a whole-state epoch that no
    /// restore moves back. A decision that read a player's inventory or a container's slots
    /// compares it before and after its listeners ran; an equal rewrite counts as a write.
    pub writes: u64,
    /// The same count for a player's own state only (inventory, cursor, menu storage): what a
    /// decision that must stay open to other writes (a mod's move into another block) reads.
    pub player_writes: u64,
    /// The root commits telling their joins and leaves, outermost first: a listener's command in
    /// another level can commit a transaction there while this level's commit is told.
    pub committing: Vec<entities::Committing>,
    pub(in crate::game) composter_epoch: Option<mod_api::WorldEpoch>,
    /// Each composter port owns its transactional random stream, independently of Level.random.
    pub composter_randoms: ComposterRandoms,
    /// Active retained original-player attachment maps during `player_clone` dispatch,
    /// kept in sync with open transaction frames so mutations roll back on abort.
    pub(in crate::game) retained_original_attachments:
        Vec<std::sync::Arc<std::sync::Mutex<super::player_state::OriginalAttachmentSlot>>>,
    #[cfg(any(test, feature = "test-support"))]
    pub trace: Option<std::sync::Arc<std::sync::Mutex<Vec<String>>>>,
}

impl OpenTransactions {
    pub(in crate::game) fn next_frame(&mut self) -> TransactionFrame {
        let tag = self.tag.get_or_insert_with(|| Box::new(0));
        let journal = std::ptr::from_ref(&**tag) as usize;
        self.next_frame = self
            .next_frame
            .checked_add(1)
            .expect("transaction frame sequence exhausted");
        TransactionFrame::new(journal, self.next_frame, self.levels.len())
    }

    pub fn push_retained_original(
        &mut self,
        slot: std::sync::Arc<std::sync::Mutex<super::player_state::OriginalAttachmentSlot>>,
    ) {
        self.retained_original_attachments.push(slot);
    }

    pub fn pop_retained_original(&mut self) {
        self.retained_original_attachments.pop();
    }

    pub fn close_retained_originals(&self, frame: Option<TransactionFrame>, commit: bool) {
        if let Some(frame) = frame {
            for slot in &self.retained_original_attachments {
                slot.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .close_frame(frame, commit);
            }
        }
    }

    pub fn level_frame(&mut self, dim: super::Dim) -> Option<&mut Level> {
        entities::open_in(self, dim)
    }

    pub(in crate::game) fn insert_scratch(
        &mut self,
        entry: scratch::Scratch,
    ) -> mod_api::ScratchSlot {
        let slot = mod_api::ScratchSlot::fresh();
        self.scratch.insert(slot.id(), entry);
        slot
    }

    pub fn player_write(&mut self) -> Option<&mut Level> {
        self.player_writes = self.player_writes.wrapping_add(1);
        self.writes = self.writes.wrapping_add(1);
        self.last_mut()
    }

    pub fn journal_modifier_changes(&mut self, dim: Dim, id: u32, changes: ModifierChanges) {
        if let Some(level) = self.last_mut() {
            level
                .modifier_changes
                .entry((dim, id))
                .or_default()
                .merge(changes);
        }
    }

    pub fn keep_explicit_modifier_write(&mut self, dim: Dim, mob: u32, attribute: &str, id: &str) {
        for level in self.iter_mut() {
            if let Some(changes) = level.modifier_changes.get_mut(&(dim, mob)) {
                changes.keep_explicit_write(attribute, id);
            }
            for (_, kept) in level
                .effect_modifiers
                .iter_mut()
                .filter(|(owner, _)| *owner == (dim, mob))
            {
                kept.keep_explicit_write(attribute, id);
            }
        }
    }

    /// Keep a resident row just before its first write, without relinquishing the loan.
    pub fn journal_block_entity_loan(
        &mut self,
        dim: super::Dim,
        pos: Pos,
        loan: &sunrise_level::block_entities::BlockEntityLoan<'_>,
        chunks: &sunrise_level::chunk_map::ChunkMap,
        name: Option<&sunrise_core::nbt::Tag>,
        hopper: Option<HopperState>,
    ) {
        self.writes = self.writes.wrapping_add(1);
        if let Some(level) = entities::open_in(self, dim) {
            level.block_entities.entry(pos).or_insert_with(|| Kept {
                entity: Some(BlockEntity::Mod(Box::new(loan.entity.clone()))),
                client_update_fields: loan.entity.snapshot_client_update_fields(),
                taken: None,
                generation: Some(loan.identity.generation),
                attachments: loan.attachments.cloned(),
                hopper,
                name: name.cloned(),
                extras: BlockEntityExtras::capture(chunks, pos),
                contents_watches: loan.contents_watches.snapshot(loan.identity),
            });
        }
    }

    pub fn mark_scoped_menu_write(
        &mut self,
        dim: Dim,
        chunk: (i32, i32),
        modified: &mut ModifiedChunks,
    ) {
        if let Some(level) = entities::open_in(self, dim) {
            level
                .save_chunks
                .insert((dim, chunk, SaveChunkScope::Level));
        } else {
            modified.insert(chunk);
        }
    }

    /// Keep the first write to each field of a live general-entity subject.
    pub fn journal_general_entity(
        &mut self,
        join: sunrise_sim::join_seq::JoinSeq,
    ) -> Option<&mut KeptGeneralEntity> {
        Some(self.last_mut()?.general_entities.entry(join).or_default())
    }

    /// The journaled-write epoch.
    pub fn writes(&self) -> u64 {
        self.writes
    }
    /// The journaled-write epoch of players' own state.
    pub fn player_writes(&self) -> u64 {
        self.player_writes
    }
}

impl std::ops::Deref for OpenTransactions {
    type Target = Vec<Level>;
    fn deref(&self) -> &Vec<Level> {
        &self.levels
    }
}

impl std::ops::DerefMut for OpenTransactions {
    fn deref_mut(&mut self) -> &mut Vec<Level> {
        &mut self.levels
    }
}

/// Puts the closing phase back when a close ends, however it ends.
pub struct ClosingPhase<'a, 'g, S: crate::game::SideGame<'g>> {
    pub game: &'a mut S,
    pub(in crate::game) outer: Option<TransactionLifecycle>,
    resource: std::marker::PhantomData<fn(&'g ()) -> &'g ()>,
}

impl<'a, 'g, S: crate::game::SideGame<'g>> ClosingPhase<'a, 'g, S> {
    #[inline(always)]
    pub fn new(game: &'a mut S, outer: Option<TransactionLifecycle>) -> Self {
        Self {
            game,
            outer,
            resource: std::marker::PhantomData,
        }
    }
}

impl<'g, S: crate::game::SideGame<'g>> Drop for ClosingPhase<'_, 'g, S> {
    fn drop(&mut self) {
        self.game.transactions.closing = self.outer;
    }
}

pub struct KeptBlock {
    pub state: BlockStateId,
    pub replay: Option<BlockReplay>,
    pub strict: bool,
}

pub type RemovedRow = (Box<BlockEntity>, u64);

pub fn take_replay_row(world: &mut World, pos: Pos) -> Option<RemovedRow> {
    let generation = world.block_entities.generation(&pos);
    world
        .block_entities
        .remove(&pos)
        .zip(generation)
        .map(|(row, generation)| (Box::new(row), generation))
}

struct PlacementData {
    name: Option<sunrise_core::nbt::Tag>,
    extras: sunrise_level::world::BlockEntityExtras,
}

#[derive(Default)]
pub struct ReplayEntity {
    pub row: Option<RemovedRow>,
    data: Option<Box<PlacementData>>,
}

impl ReplayEntity {
    pub fn take(world: &mut World, pos: Pos) -> Self {
        let name = world.block_names.get(&pos).cloned();
        let extras = world.block_entity_extras(pos);
        let data = (name.is_some() || extras.source.is_some() || extras.pending_loot.is_some())
            .then(|| Box::new(PlacementData { name, extras }));
        Self {
            row: take_replay_row(world, pos),
            data,
        }
    }

    pub fn install(self, world: &mut World, pos: Pos, placement: bool) {
        if world.block_entities.get(&pos).is_some() {
            return;
        }
        if placement {
            if let Some(data) = self.data {
                world.set_block_entity_extras(pos, data.extras);
                if let Some(name) = data.name {
                    world.block_names.insert(pos, name);
                } else {
                    world.block_names.remove(&pos);
                }
            } else {
                world.forget_block_entity_source(pos);
                world.block_names.remove(&pos);
            }
        }
        if let Some((entity, generation)) = self.row {
            let entities = &mut world.block_entities;
            entities.insert(pos, *entity);
            entities.restore_generation(pos, generation);
            entities.reattach_retired(pos, generation);
        }
    }
}

pub enum BlockEntityCreation {
    Live,
    /// A later write owns the factory result for this same resident lifetime.
    Deferred,
    /// A factory was evaluated while staging; no generation records an explicit absence.
    Staged {
        generation: Option<u64>,
        entity: ReplayEntity,
    },
}

pub struct BlockStep {
    pub pos: Pos,
    pub to: BlockStateId,
    pub replay: BlockReplay,
    // Some means staging took the resident row out (even if it was already absent).
    pub removed: Option<ReplayEntity>,
    pub placement: bool,
    pub creation: BlockEntityCreation,
}

pub struct StrictBlockStep {
    pub to: BlockStateId,
    // Before the first replacement on this cell; swapped for the new row at commit.
    pub row: Option<ReplayEntity>,
    pub created_generation: Option<u64>,
    /// A strict replacement ends the old block's connection-group lifetime.
    pub clear_connection_group: bool,
}

pub type StrictBlockSteps = indexmap::IndexMap<Pos, StrictBlockStep>;

#[derive(Clone, Copy)]
pub struct BlockReplay {
    pub state: BlockStateId,
    pub flags: i32,
    /// Replay LevelWriteHost writes through the direct host's block-entity creation path.
    pub level_host: bool,
    /// A level-host write only hid the entity from other reads. At commit
    /// the real Level.setBlock still has to run its removal side effects.
    pub replay_removal: bool,
    /// Any tentative write at this position replaced the block with one that does not keep
    /// connection groups (`!Behaviours::keeps_block_entity`), even if a later write returned
    /// it to a compatible block.
    pub clear_connection_group: bool,
}

/// Preserve requested updates, and suppress an effect only if every write
/// suppressed it. Other bits describe the final write (e.g. moved by piston).
pub(in crate::game) fn merge_block_update_flags(earlier: i32, later: i32) -> i32 {
    const REQUESTED: i32 =
        flags::UPDATE_NEIGHBORS | flags::UPDATE_CLIENTS | flags::UPDATE_IMMEDIATE;
    const SUPPRESSED: i32 = flags::UPDATE_INVISIBLE
        | flags::UPDATE_KNOWN_SHAPE
        | flags::UPDATE_SUPPRESS_DROPS
        | flags::UPDATE_SKIP_SHAPE_UPDATE_ON_WIRE
        | flags::UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS
        | flags::UPDATE_SKIP_ON_PLACE;
    ((earlier | later) & REQUESTED)
        | ((earlier & later) & SUPPRESSED)
        | (later & !(REQUESTED | SUPPRESSED))
}

/// Before-images of the fields a general-entity scope wrote, at one depth.
#[derive(Default)]
pub struct KeptGeneralEntity {
    pub velocity: Option<Vec3>,
    pub rotation: Option<[f32; 2]>,
    pub fall_distance: Option<f64>,
}

/// One journal, either an open frame or a level-local batch split from one at close.
#[derive(Default)]
pub struct Level {
    pub frame: Option<TransactionFrame>,
    pub parent_attachment_query_scope: u64,
    pub dimension: Option<super::Dim>,
    /// Each block entity as it was before this transaction first wrote it.
    pub block_entities: HashMap<Pos, Kept>,
    /// Rows moved out of storage when their block was tentatively replaced. Commit replays
    /// removal with the row as it was then; abort uses the first write in `block_entities`.
    /// A removal without a resident row still marks the position for abort.
    pub removed_block_entities: HashMap<Pos, Option<RemovedRow>>,
    /// A decorated pot lives in its chunk's saved block-entity tags, not the slot table.
    pub saved_pots: HashMap<Pos, Option<sunrise_core::nbt::Tag>>,
    /// Each item slot (block entity, slot) a slot-only write changed, as it was before this
    /// transaction first wrote it, for a block entity not kept whole in `block_entities`: a
    /// move through a container keeps the slot it touches, not the entity's whole saved data.
    pub slots: HashMap<(Pos, usize), KeptSlot>,
    /// Each block's original state and the combined update flags of its writes.
    pub blocks: HashMap<Pos, KeptBlock>,
    pub capture_placement: bool,
    /// Each cell's original connection group before this transaction first changed it, used for
    /// silent rollback on abort and pre-replay restoration on root commit.
    pub connection_groups: HashMap<Pos, Option<u64>>,
    /// Level.random in this frame's own dimension before its first draw.
    pub level_random: Option<LegacyRandom>,
    /// Other levels drawn from at this depth. Only those draws allocate a map entry.
    pub parked_level_randoms: HashMap<LevelRef, LegacyRandom>,
    /// Named loot streams before tentative block loot.
    pub loot_sequences: Option<sunrise_items::random_sequences::RandomSequences>,
    /// Composter-local random streams before this depth first evaluated their layers.
    pub composter_randoms: ComposterRandoms,
    /// Each quart cell before this transaction first wrote it, keyed by level.
    pub biomes: HashMap<(Dim, Pos), u16>,
    /// Each player's inventory and saved data before this transaction first wrote them.
    pub players: HashMap<JournaledPlayer, KeptPlayer>,
    /// Ender storage has one journal shared by player and menu writes.
    pub ender_items: HashMap<JournaledPlayer, Vec<Option<ItemStack>>>,
    /// Saved player extra NBT (`level.player_extra` / `guest.saved`), shared by player and
    /// persistent-data writes, including dead players.
    pub player_extra: HashMap<JournaledPlayer, Option<sunrise_core::nbt::Tag>>,
    /// A custom book's original flags, in first-write order.
    pub player_books:
        indexmap::IndexMap<(JournaledPlayer, RecipeBookTypeKey), (PlayerRef, Option<BookFlags>)>,
    /// Synced player health and its unpublished update events before set_health, by player body.
    pub player_health: HashMap<PlayerBodyKey, KeptPlayerHealth>,
    /// Mob health and synchronization dirtiness before set_health, keyed by level.
    pub mob_health: HashMap<(Dim, u32), KeptMobHealth>,
    /// Active and hidden effects before the first write to the mob at this depth, keyed by level.
    pub mob_effects: HashMap<(Dim, u32), KeptMobEffects>,
    /// Effect-owned modifier changes in write order, so rows can return to their positions.
    pub effect_modifiers: Vec<((Dim, u32), EffectModifierUndo)>,
    /// Each entity's inventory (`entity_slots`) before this transaction first wrote it, by level.
    pub entities: HashMap<(Dim, u32), Vec<Option<ItemStack>>>,
    /// Each mob's or armor stand's equipment before this transaction first
    /// wrote it (#932), told apart for the way back and keyed by level.
    pub equipment: HashMap<(Dim, u32), KeptEquipment>,
    /// Live modifier diffs, also kept for rebuilds that write no slot, keyed by level.
    pub modifier_changes: HashMap<(Dim, u32), ModifierChanges>,
    /// Each chested mob's item list's length before `entity_slots` first grew it to its slots
    /// in this transaction, keyed by level.
    pub mob_lengths: HashMap<(Dim, u32), usize>,
    /// Each item entity's changed fields before this transaction first wrote them.
    pub item_entities: HashMap<sunrise_sim::join_seq::JoinSeq, KeptItemEntity>,
    /// Each general entity subject's changed fields before the first write.
    pub general_entities: HashMap<sunrise_sim::join_seq::JoinSeq, KeptGeneralEntity>,
    /// The merge fields of resident experience orbs before the first award touched them, by level.
    pub orbs: HashMap<(Dim, sunrise_sim::join_seq::JoinSeq), (i32, u32, i32)>,
    /// Each detached station before the first click wrote it, by connection and owner.
    pub stations: HashMap<(u32, decision::MenuId), super::lan_menu_data::GuestStation>,
    /// Each guest's inventory crafting grid before the first write, by connection.
    pub menus: HashMap<u32, KeptMenus>,
    /// Drag, machine and preview state before a scoped default click.
    pub menu_clicks: HashMap<u64, KeptClick>,
    /// The first drag state written in a guest's mount window.
    pub mount_drags: HashMap<MountClickWindow, QuickCraftDrag>,
    /// Mount window identities to reconcile once the outermost transaction has settled.
    pub mount_replies: indexmap::IndexSet<MountClickWindow>,
    pub menu_click_active: Option<u32>,
    /// Each native window's slots before the first write, by menu generation.
    pub native_menu_slots: HashMap<u64, Vec<Option<ItemStack>>>,
    pub menu_data: MenuDataJournal,
    /// The scoreboard before this transaction first wrote it (`journal_scoreboard`), whole.
    pub board: Option<sunrise_game_data::scoreboard::Scoreboard>,
    /// Each entity's, player's, chunk's and the level's attachment map before this transaction
    /// first reached it (a block entity's is kept with it). Per-level holders are keyed by level;
    /// player-owned attachment maps are keyed by player body across dimensions.
    pub attachments: HashMap<AttachmentKey, KeptAttachments>,
    /// Held once this transaction reaches an attachment map, until it closes: the save
    /// admission gate (`passes_settled`) refuses a save while a map may hold tentative
    /// values, which only the root commit makes saveable.
    pub save_gate: Option<sunrise_sim::join_seq::visibility::PassLease>,
    /// Strict edits and saved pots keep their level's save gate until settlement.
    pub level_save_gate: Option<sunrise_sim::join_seq::visibility::PassLease>,
    /// Each chunk's unsaved mark and save queue entry before this transaction first reached an
    /// attachment map in it: an abort puts them back, so a chunk only a tentative write marked
    /// is clean again.
    pub chunk_marks: HashMap<(i32, i32), sunrise_level::world::ChunkMark>,
    /// Chunks whose committed storage writes must be saved, in first-write order and in the
    /// levels written.
    pub save_chunks: indexmap::IndexSet<(Dim, (i32, i32), SaveChunkScope)>,
    /// New map data and its reserved IDs. Neither enters the saved map table until root commit.
    pub new_maps: BTreeMap<i32, MapData>,
    /// What the root commit does, in order.
    pub after: Vec<After>,
    /// Internal decide/fire/apply state for menu actions.
    pub actions: decision::Journal,
    /// Merchant takes keep their session, mob, statistics and criterion progress together.
    pub trades: super::lan_menu_data::Journal,
    pub packets: Option<sunrise_sim::lan::conn::transaction_packets::Scope>,
    pub captured_packets: Vec<sunrise_sim::lan::conn::transaction_packets::Packets>,
    pub action: Option<decision::ActiveAction>,
    /// A scratch slot owned by this frame, and before-images of ancestor loans it wrote.
    pub owned_scratch: Option<u64>,
    pub scratch_kept: HashMap<u64, scratch::Scratch>,
    /// The entities it added and removed, and the announcements it holds.
    pub lifetimes: entities::EntityJournal,
    /// Each attachment map that went with an entity to another level after this transaction
    /// first reached it, by the id it arrived under, as it was then.
    pub travelled: HashMap<u32, KeptAttachments>,
    /// The index rows of the entities that left this level, kept until the root closes.
    pub departed: Vec<entities::Departure>,
    /// The entities taken for a transfer whose arrival is not adopted yet, by source id, each
    /// with the slot its arrival reads an abort's way back from.
    pub in_flight: HashMap<u32, entities::ArrivalRollback>,
}

/// A slot's stack before a transaction first wrote it, and the generation of the block entity
/// it belongs to: an abort puts it back only into that same block entity, never into one that
/// replaced it.
pub struct KeptSlot {
    pub stack: Option<ItemStack>,
    pub generation: Option<u64>,
}

/// Identity of one player body for transaction rollback: either the local host player
/// (`lan_acting.is_none()`) or one guest record in `lan.hub.guests` (a real online guest shared
/// across dimensions, or a per-level connectionless fake player), keyed by its unique `conn` id.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerBodyKey {
    Host,
    Guest(u32),
}

/// Key for `Level::attachments`: per-level holders (`Entity`, `Chunk`, `Level`) are isolated by
/// dimension, whereas player-owned attachment maps are keyed by the specific player body
/// regardless of which level is swapped in. Retained originals have their own journal.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttachmentKey {
    Level(Dim, Holder),
    Player(PlayerBodyKey),
}

pub struct KeptAttachments {
    pub contents: Attachments,
    pub indexed: bool,
}

pub struct Kept {
    pub entity: Option<BlockEntity>,
    /// The pre-write batch, stored apart from the entity clone's fresh identity.
    pub client_update_fields: Option<sunrise_core::nbt::Compound>,
    pub taken: Option<sunrise_level::block_entities::TakenLeaseSnapshot>,
    pub generation: Option<u64>,
    /// None where it had no map: an abort removes one a write made.
    pub attachments: Option<Attachments>,
    pub hopper: Option<HopperState>,
    /// The custom name Sunrise keeps beside the model (`World::block_names`).
    pub name: Option<sunrise_core::nbt::Tag>,
    /// The container lock and loot table roll kept beside the model.
    pub extras: BlockEntityExtras,
    pub contents_watches: Vec<(Pos, u64)>,
}

impl Kept {
    pub fn capture(
        world: &sunrise_level::world::World,
        hoppers: &HashMap<Pos, HopperState>,
        pos: Pos,
    ) -> Self {
        let entity = world.block_entities.get(&pos);
        Self {
            entity: entity.cloned(),
            taken: world.block_entities.taken_lease_snapshot(&pos),
            client_update_fields: match entity {
                Some(BlockEntity::Mod(entity)) => entity.snapshot_client_update_fields(),
                _ => None,
            },
            generation: world.block_entities.generation(&pos),
            attachments: world.block_entities.attachments(pos).cloned(),
            hopper: hoppers.get(&pos).copied(),
            name: world.block_names.get(&pos).cloned(),
            extras: world.block_entity_extras(pos),
            contents_watches: world
                .block_entities
                .identity(pos)
                .map_or_else(Vec::new, |observer| {
                    world.block_entities.contents_watches.snapshot(observer)
                }),
        }
    }

    pub fn capture_client(world: &sunrise_level::world::World, pos: Pos) -> Self {
        let entity = world.block_entities.get(&pos);
        Self {
            entity: entity.cloned(),
            taken: world.block_entities.taken_lease_snapshot(&pos),
            client_update_fields: match entity {
                Some(BlockEntity::Mod(entity)) => entity.snapshot_client_update_fields(),
                _ => None,
            },
            generation: world.block_entities.generation(&pos),
            attachments: world.block_entities.attachments(pos).cloned(),
            hopper: None,
            name: world.block_names.get(&pos).cloned(),
            extras: world.block_entity_extras(pos),
            contents_watches: world
                .block_entities
                .identity(pos)
                .map_or_else(Vec::new, |observer| {
                    world.block_entities.contents_watches.snapshot(observer)
                }),
        }
    }

    pub fn restore_client(self, world: &mut sunrise_level::world::World, removed: bool, pos: Pos) {
        let current = world.block_entities.generation(&pos);
        let replaced = current != self.generation;
        if replaced && current.is_some() {
            world.block_entities.remove(&pos);
        }
        if let (Some(BlockEntity::Mod(entity)), Some(fields)) =
            (&self.entity, self.client_update_fields)
        {
            entity.set_client_update_fields(fields);
        }
        match (self.entity, world.block_entities.get_mut(&pos)) {
            (Some(entity), Some(now)) => {
                *now = entity;
                world
                    .block_entities
                    .restore_generation(pos, self.generation.unwrap());
            }
            (Some(entity), None) if replaced || removed => {
                let generation = self.generation.unwrap();
                world.block_entities.insert(pos, entity);
                world.block_entities.restore_generation(pos, generation);
                world.block_entities.reattach_retired(pos, generation);
            }
            (None, None) if replaced && let Some(taken) = self.taken => {
                let entities = &mut world.block_entities;
                entities.restore_taken_lease(pos, taken);
                entities.reattach_retired(pos, self.generation.unwrap());
            }
            _ => {}
        }
        world
            .block_entities
            .put_back_attachments(pos, self.generation, self.attachments);
        match self.name {
            Some(name) => world.block_names.insert(pos, name),
            None => world.block_names.remove(&pos),
        };
        let chunk = (pos.0 >> 4, pos.2 >> 4);
        let mark = world.modified.mark(chunk);
        world.set_block_entity_extras(pos, self.extras);
        world.modified.restore(chunk, mark);
        if let Some(observer) = world.block_entities.identity(pos)
            && Some(observer.generation) == self.generation
        {
            world
                .block_entities
                .contents_watches
                .restore(observer, self.contents_watches);
        }
    }
}

pub struct KeptPlayerHealth {
    pub data: sunrise_world::synced_floats::SyncedFloats,
    pub events_len: usize,
    pub dead: bool,
    pub guest: Option<(u32, bool, u32)>,
}

pub struct KeptMobHealth {
    pub absorption: f32,
    pub health: f32,
    pub needs_sync: bool,
    pub death_time: i32,
    pub death_canceled: bool,
    pub processed: bool,
}

pub struct KeptMobEffects {
    pub active: Vec<StoredEffect>,
    pub chains: Vec<EffectInstance>,
    pub ambient: Vec<&'static str>,
    pub hidden: Vec<&'static str>,
    pub dirty: bool,
}

/// Storage identity, not a public player handle. Fake players in different levels
/// can share a UUID; their hub connections still distinguish their records.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum JournaledPlayer {
    Local(u128),
    Guest(u32),
}

pub enum PlayerStorageHome {
    Game,
    Guest(usize),
}

#[derive(Clone)]
pub struct KeptPlayer {
    pub inventory: Vec<Option<ItemStack>>,
    pub armor: [Option<ItemStack>; 4],
    pub offhand: Option<ItemStack>,
    pub equipment: Equipment,
    pub carried: Option<ItemStack>,
}

/// The raw slots before the first write, with a mob's separate gear overlay.
/// Projecting the slots here would bake the overlay into the resident stacks.
pub enum KeptEquipment {
    Mob {
        slots: Box<Equipment>,
        gear: Option<Box<Gear>>,
    },
    Stand(Box<Equipment>),
}

pub struct KeptClick {
    pub conn: u32,
    pub drag: Option<(u8, Vec<usize>)>,
    pub craft: Option<sunrise_sim::lan::menu::ModCraft>,
    pub slot_changed_seen: Option<Vec<Option<ItemStack>>>,
    pub owned: Option<(u64, Box<sunrise_sim::menu::Menu>)>,
    pub machine: Option<(i32, Box<sunrise_sim::menu::Menu>)>,
    pub places: Option<super::native_menu_values::MenuSlotPlaces>,
}

#[derive(Clone)]
pub struct KeptMenus {
    pub craft_grid: [Option<ItemStack>; 4],
}

/// What a write tells the world, at the root commit.
#[derive(Clone, Copy)]
pub struct RemovedBlockEntityEffects {
    pub pos: Pos,
    pub state: BlockStateId,
    pub sculk: bool,
    pub jukebox: bool,
    pub beacon: bool,
    pub gateway: bool,
    pub had_shapes: bool,
}

pub enum After {
    BlockEntityRemoved(RemovedBlockEntityEffects),
    /// Reserved raw bytes publish only when their connection incarnation is still live.
    Payload(super::payload_transport::Deferred),
    /// A validated packet carries its protocol incarnation and submitting mod for publication errors.
    Packet(
        sunrise_sim::raw_payloads::queued_writer::PreparedPacket,
        Option<&'static str>,
    ),
    Composter(super::composter_data::Effects),
    DispenseEvent(i32, Pos, i32),
    /// A mod's world pickup or placement of fluid, its block already set: what is left of it
    /// (`Game::commit_fluid_write`).
    FluidWrite(Box<super::world_fluid_data::WrittenFluid>),
    /// A bucket's sound and fluid game event (`level_play_fluid_effects`).
    FluidEffects(Box<super::world_fluid_data::FluidEffects>),
    /// A dispenser drop in the source level, with optional overflow click and smoke.
    DispenserDrop {
        pos: Pos,
        facing: sunrise_core::geometry::Direction,
        spawn: Box<super::spill_common::Spawn>,
        play_events: bool,
    },
    /// Sound and smoke for a mod's accepted dispense reply, after its behaviour's effects.
    DispenserFeedback {
        pos: Pos,
        facing: sunrise_core::geometry::Direction,
        event: i32,
    },
    /// `BlockEntity.setChanged`.
    Changed(Pos),
    /// `BlockEntity.setChanged` dirty/comparator updates without reentering `contents_changed`.
    ChangedWithoutHook(Pos),
    /// A notice for a specific resident row, not a replacement at the same position.
    ChangedHolder {
        holder: BlockEntityIdentity,
        silent: bool,
    },
    GuestMenu(u32, Box<super::lan_menu_data::GuestMenuEffects>),
    /// Publish both the connection-local state and its packets together, never on abort.
    PersonalSidebar(
        u32,
        Option<Box<sunrise_sim::lan::scoreboard::SidebarContents>>,
    ),
    /// Equipment transitions from a mount menu, with distinct sound and game event positions.
    MountEquipment {
        dim: Dim,
        id: u32,
        at: [f64; 3],
        sound: Option<(String, [f64; 3])>,
        equips: bool,
    },
    MenuClick(Box<super::lan_menu_data::MenuClickReply>),
    /// The server owns and decodes the staged open payload.
    MenuOpen(u32, i32, Box<dyn std::any::Any + Send>),
    MenuPlacesNotice(u32, u128, i32),
    FurnaceMenu(Box<super::lan_menu_data::FurnaceMenuEffects>),
    StoreCartItems(Pos),
    BackedMenuClick(u32, u128, mod_api::MenuRef, i32),
    /// A provider's answer changed here (`level_invalidate_capabilities`): the capability
    /// generation moves on.
    Capabilities(Pos),

    /// `ServerLevel.destroyBlockProgress`: breaker id, position, stage. Sent once, at the root commit.
    BreakProgress(i32, [i32; 3], i32),
    /// Replay a block write, coalesced unless the cell has placement capture or a strict edit.
    Block(BlockStep),
    /// Publish the storage and watched writes of a strict bulk edit, not ordinary updates.
    BulkBlocks(
        sunrise_level::block_edit::BulkPublication,
        Vec<super::block_edits_common::Tracked>,
        Vec<[i32; 3]>,
        StrictBlockSteps,
    ),
    /// An explicit `LevelRef::set_connection_group` write, replayed in order with block writes at
    /// the root commit so cross-position cascades (e.g. popping a supported block) clear it.
    ConnectionGroup(Pos, Option<u64>),
    /// A validated level write host effect; never published on abort.
    LevelWrite(Box<super::journal_level_effects::PendingWrite>),
    PlacementEffect(Box<sunrise_world::level::ticks::PlacementEffect>),
    WitherPattern(sunrise_sim::block_pattern::PatternMatch, Vec<[i32; 3]>),
    UpdatePlacedBlockEntity(Pos, u64),
    BlockGameEvent(
        &'static str,
        [i32; 3],
        Option<sunrise_sim::mobs::Who>,
        BlockStateId,
    ),
    DestroyBlockGameEvent {
        pos: [i32; 3],
        state: BlockStateId,
        breaker: Option<mod_api::EntityRef>,
    },
    BlockDropSpawns(Vec<super::spill_common::Spawn>, Vec<([f64; 3], u32, i32)>),
    /// Save, remesh and resend a changed biome column.
    Biome((i32, i32)),
    /// Deferred interaction effects, replayed in the same root-commit order.
    Effect(DeferredEffect),
    /// Effects of a tentative merchant take, published only at the root.
    Trade(Box<super::lan_menu_data::Effect>),
    /// `Block.popResource`.
    Drop([i32; 3], ItemStack),
    /// `Player.drop(stack, false)` by the player with this UUID: what no longer fits.
    PlayerDrop(u128, ItemStack),
    /// A respawn replaces the holder behind the player's UUID. Do not install a new
    /// attachment generation or copy tentative data until the old generation is committed.
    PlayerRespawn(u32),
    /// `/give`'s fake item and pickup sound in the receiving player's level.
    GivePickup {
        uuid: u128,
        dim: super::Dim,
        prototype: ItemStack,
    },
    InventoryTrigger(u128),
    GrindstoneEvent([i32; 3]),
    StationContent(u32, decision::MenuId, i32),
    /// What did not fit of a `/give` to the player `uuid`, thrown in the level `dim` that was in
    /// place when it was accepted; at `eye`, the player's eyes then, if they are dead or gone
    /// by the commit.
    GiveRest {
        uuid: u128,
        rest: ItemStack,
        owner: Option<[i32; 4]>,
        dim: super::Dim,
        eye: [f64; 3],
    },
    PlayerEquip(u128, String, Option<ItemStack>),
    RecipeBookSettings(
        JournaledPlayer,
        PlayerRef,
        RecipeBookTypeKey,
        Option<BookFlags>,
    ),
    /// An item split off an item entity, dropped where it is (`Block.popResource`'s delay).
    Spawn([f64; 3], ItemStack),
    /// The item entity with this id was emptied: removed, unless it holds items again.
    Emptied(u32),
    /// A component-only mob equipment write must reach the normal tracker even if the mob is idle.
    EquipmentSync(u32),
    /// `LivingEntity.onEquipItem`'s sound and equip/unequip game event, at the entity `who`
    /// is: what an equipment write through a transaction tells the world only at the root
    /// commit, so an abort stays quiet (#932).
    Equip {
        at: [f64; 3],
        sound: Option<String>,
        equips: bool,
        who: u32,
    },
}

impl After {
    pub fn level_owned(&self) -> bool {
        matches!(
            self,
            Self::Block(_)
                | Self::BulkBlocks(..)
                | Self::ConnectionGroup(..)
                | Self::BlockEntityRemoved(_)
                | Self::DispenseEvent(..)
                | Self::Changed(_)
                | Self::ChangedWithoutHook(_)
                | Self::ChangedHolder { .. }
                | Self::Capabilities(_)
                | Self::LevelWrite(_)
                | Self::PlacementEffect(_)
                | Self::WitherPattern(..)
                | Self::UpdatePlacedBlockEntity(..)
                | Self::FluidWrite(_)
                | Self::FluidEffects(_)
                | Self::DispenserDrop { .. }
                | Self::DispenserFeedback { .. }
                | Self::BlockGameEvent(..)
                | Self::DestroyBlockGameEvent { .. }
                | Self::BlockDropSpawns(..)
                | Self::Biome(_)
                | Self::Drop(..)
                | Self::Spawn(..)
                | Self::Emptied(_)
        )
    }
}

/// Effects shared by transactional interaction contexts.
pub enum DeferredEffect {
    CustomStat {
        player: u128,
        key: &'static str,
        amount: i32,
    },
    /// `Player.awardStat` of an item statistic (`used`, `crafted`...) for `item`.
    ItemStat {
        player: u128,
        stat: &'static str,
        item: std::sync::Arc<str>,
        amount: i32,
    },
    Sound {
        sound: &'static str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
    },
    /// `PlayerRef::set_statistics` inside a transaction: the counters and their criterion
    /// scores are replaced together at the root commit.
    RestoreStatistics(
        mod_api::PlayerRef,
        Box<sunrise_gameplay_common::stats::StatValues>,
    ),
    ChangeAdvancement {
        player: mod_api::PlayerRef,
        id: String,
        criterion: Option<String>,
        grant: bool,
    },
}

impl Level {
    /// Keep the first snapshot for this stream. `None` names the frame's own level.
    pub fn keep_level_random(
        &mut self,
        owner: Option<LevelRef>,
        before: impl FnOnce() -> LegacyRandom,
    ) {
        match owner {
            Some(level) if self.dimension.map(super::Dim::key) != Some(level.dimension()) => {
                self.parked_level_randoms
                    .entry(level)
                    .or_insert_with(before);
            }
            _ => {
                self.level_random.get_or_insert_with(before);
            }
        }
    }

    pub fn keep_bulk_block(&mut self, pos: Pos, state: u32) {
        self.level_save_gate
            .get_or_insert_with(sunrise_sim::join_seq::visibility::PassLease::enter);
        self.blocks
            .entry(pos)
            .and_modify(|kept| kept.strict = true)
            .or_insert(KeptBlock {
                state: BlockStateId(state),
                replay: None,
                strict: true,
            });
    }

    pub fn keep_connection_group(&mut self, pos: Pos, group: Option<u64>) {
        self.connection_groups.entry(pos).or_insert(group);
    }

    pub fn merge_level_writes(&mut self, child: &mut Self) {
        self.level_save_gate = self.level_save_gate.take().or(child.level_save_gate.take());
        for (pos, biome) in std::mem::take(&mut child.biomes) {
            self.biomes.entry(pos).or_insert(biome);
        }
        for (pos, group) in std::mem::take(&mut child.connection_groups) {
            self.keep_connection_group(pos, group);
        }
        // Slots before whole entities: a slot the outer transaction already kept the
        // whole entity of is older there, but one kept whole only by this child is not,
        // so the child's slot entries go up beside its capture.
        for ((pos, slot), kept) in std::mem::take(&mut child.slots) {
            if !self.block_entities.contains_key(&pos) {
                self.slots.entry((pos, slot)).or_insert(kept);
            }
        }
        for (pos, kept) in std::mem::take(&mut child.block_entities) {
            self.block_entities.entry(pos).or_insert(kept);
        }
        for (pos, tag) in std::mem::take(&mut child.saved_pots) {
            self.saved_pots.entry(pos).or_insert(tag);
        }
        for (pos, removed) in std::mem::take(&mut child.removed_block_entities) {
            let first = self.removed_block_entities.entry(pos).or_default();
            if first.is_none() {
                *first = removed;
            }
        }
        for (pos, block) in std::mem::take(&mut child.blocks) {
            self.blocks
                .entry(pos)
                .and_modify(|kept| {
                    kept.strict |= block.strict;
                    if let Some(replay) = block.replay {
                        if let Some(earlier) = &mut kept.replay {
                            earlier.flags = merge_block_update_flags(earlier.flags, replay.flags);
                            earlier.level_host = replay.level_host;
                            earlier.replay_removal |= replay.replay_removal;
                            earlier.clear_connection_group |= replay.clear_connection_group;
                        } else {
                            kept.replay = Some(replay);
                        }
                    }
                })
                .or_insert(block);
        }
        for (id, kept) in std::mem::take(&mut child.item_entities) {
            self.item_entities.entry(id).or_default().merge(kept);
        }
        for (join, kept) in std::mem::take(&mut child.general_entities) {
            let first = self.general_entities.entry(join).or_default();
            first.velocity = first.velocity.or(kept.velocity);
            first.rotation = first.rotation.or(kept.rotation);
            first.fall_distance = first.fall_distance.or(kept.fall_distance);
        }
        for (chunk, mark) in std::mem::take(&mut child.chunk_marks) {
            self.chunk_marks.entry(chunk).or_insert(mark);
        }
    }

    pub fn keep_player(
        &mut self,
        owner: JournaledPlayer,
        ender_items: &[Option<ItemStack>],
        player_extra: Option<&sunrise_core::nbt::Tag>,
        capture: impl FnOnce() -> KeptPlayer,
    ) {
        self.ender_items
            .entry(owner)
            .or_insert_with(|| ender_items.to_vec());
        self.player_extra
            .entry(owner)
            .or_insert_with(|| player_extra.cloned());
        self.players.entry(owner).or_insert_with(capture);
    }

    /// Keeps chested mob `id`'s item list's length, `len`, before `entity_slots` grows it, if
    /// this transaction hasn't yet.
    pub fn keep_mob_length(&mut self, dim: Dim, id: u32, len: usize) {
        self.mob_lengths.entry((dim, id)).or_insert(len);
    }

    /// Keep a resolved equipment row at its first write in this transaction.
    /// `mob_gear` is absent for a stand; a mob's overlay may itself be absent.
    pub fn keep_equipment(
        &mut self,
        dim: Dim,
        id: u32,
        equipment: &Equipment,
        mob_gear: Option<&Option<Box<Gear>>>,
    ) {
        self.equipment.entry((dim, id)).or_insert_with(|| {
            let slots = Box::new(equipment.clone());
            match mob_gear {
                Some(gear) => KeptEquipment::Mob {
                    slots,
                    gear: gear.clone(),
                },
                None => KeptEquipment::Stand(slots),
            }
        });
    }
}

pub enum StorageSettlement {
    Committed(Option<scratch::Scratch>),
    Aborted,
    Refused,
}

/// RAII guard for one open transaction frame. Dropping it without `close` aborts this frame
/// and any nested frames still open inside it.
pub struct StorageScope<'a, 'g, S: crate::game::SideGame<'g>> {
    pub game: &'a mut S,
    pub(in crate::game) frame: Option<TransactionFrame>,
    resource: std::marker::PhantomData<fn(&'g ()) -> &'g ()>,
}

impl<'g, S: crate::game::SideGame<'g>> StorageScope<'_, 'g, S> {
    pub fn close(mut self, commit: bool) -> Option<StorageSettlement> {
        self.frame
            .take()
            .and_then(|frame| self.game.close_storage_frame(frame, commit))
    }
}

impl<'a, 'g, S: crate::game::SideGame<'g>> StorageScope<'a, 'g, S> {
    #[inline(always)]
    pub(in crate::game) fn new(game: &'a mut S, frame: Option<TransactionFrame>) -> Self {
        Self {
            game,
            frame,
            resource: std::marker::PhantomData,
        }
    }
}

impl<'g, S: crate::game::SideGame<'g>> Drop for StorageScope<'_, 'g, S> {
    fn drop(&mut self) {
        if let Some(frame) = self.frame.take() {
            let _ = self.game.close_storage_frame(frame, false);
        }
    }
}

pub trait TransactionsGameExt<'g>: crate::game::SideGame<'g> {
    /// Actual transaction/restore events for storage-attempt regression traces.
    #[cfg(any(test, feature = "test-support"))]
    fn trace_storage_attempt(&self, entry: String) {
        if let Some(trace) = &self.transactions.trace {
            trace.lock().unwrap().push(entry);
        }
    }
    /// Pending payloads in the innermost transaction (before publication).
    #[cfg(test)]
    fn pending_transaction_payloads(&self) -> usize {
        self.transactions
            .last()
            .unwrap()
            .after
            .iter()
            .filter(|after| matches!(after, After::Payload(_)))
            .count()
    }
    /// Pending item-stat effects in the innermost transaction (before publication).
    #[cfg(any(test, feature = "test-support"))]
    fn pending_transaction_item_stats(&self) -> usize {
        self.transactions
            .last()
            .unwrap()
            .after
            .iter()
            .filter(|after| matches!(after, After::Effect(DeferredEffect::ItemStat { .. })))
            .count()
    }

    /// Opens a transaction and gives back its frame identity.
    fn open_storage_transaction(&mut self) -> TransactionFrame {
        mod_api::__private::game::CallbackOwner::call_host(|| {
            let frame = self.transactions.next_frame();
            if !self.world.block_entities.defers_retirement() {
                self.world.block_entities.defer_retirement();
            }
            let lifetimes = self.entity_journal_open();
            let common = self.common_mut();
            let parent_attachment_query_scope = common.transactions.attachment_query_scopes.enter();
            common.transactions.push(Level {
                frame: Some(frame),
                parent_attachment_query_scope,
                dimension: Some(common.dim),
                packets: sunrise_sim::lan::conn::transaction_packets::Scope::nested(),
                lifetimes,
                ..Level::default()
            });
            for slot in &common.transactions.retained_original_attachments {
                slot.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .open_frame(frame);
            }
            #[cfg(any(test, feature = "test-support"))]
            self.trace_storage_attempt("open".into());
            frame
        })
    }
    /// Closes `frame`, first aborting every transaction opened inside it and left open, so a
    /// scope closing its own is never stranded behind an abandoned child.
    fn close_storage_frame(
        &mut self,
        frame: TransactionFrame,
        commit: bool,
    ) -> Option<StorageSettlement> {
        mod_api::__private::game::CallbackOwner::close_host_transaction(commit, |commit| {
            let at = self
                .transactions
                .iter()
                .rposition(|level| level.frame == Some(frame))?;
            if self.transactions.len() > at + 1 {
                log::warn!("Sunrise: aborted a transaction left open inside the one being closed");
            }
            while self.transactions.len() > at + 1 {
                let _ = self.close_storage_transaction(false);
            }
            self.close_storage_transaction(commit)
        })
    }
    /// An engine batch opens a frame only when it has a callback to journal.
    fn storage_scope_guard(&mut self, open: bool) -> StorageScope<'_, 'g, Self> {
        let frame = open.then(|| self.open_storage_transaction());
        StorageScope::new(self, frame)
    }

    /// The trident tick opens a frame at contact, then closes it after applying the
    /// decision. The same guard aborts an open frame if dispatch or apply unwinds.
    fn with_late_storage_scope<R>(
        &mut self,
        f: impl FnOnce(&mut Self, &mut Option<TransactionFrame>) -> R,
    ) -> R {
        let mut scope = self.storage_scope_guard(false);
        f(&mut *scope.game, &mut scope.frame)
    }

    /// Runs `f` inside a new (or nested) storage transaction, committing iff `f` returns
    /// `(out, true)` and aborting on `false` or unwind.
    fn storage_scope<R>(&mut self, f: impl FnOnce(&mut Self, TransactionFrame) -> (R, bool)) -> R {
        self.run_storage_scope(f).0
    }

    /// Also returns the frame's own detached scratch row when it commits.
    fn run_storage_scope<R>(
        &mut self,
        f: impl FnOnce(&mut Self, TransactionFrame) -> (R, bool),
    ) -> (R, StorageSettlement) {
        let frame = self.open_storage_transaction();
        let guard = StorageScope::new(self, Some(frame));
        let (out, commit) = f(&mut *guard.game, frame);
        let settled = guard.close(commit).unwrap_or(if commit {
            StorageSettlement::Refused
        } else {
            StorageSettlement::Aborted
        });
        (out, settled)
    }
    /// Runs `f` inside a new (or nested) storage transaction, lending a scoped
    /// [`TransactionContext`]; commits on `Ok` and aborts on `Err` or unwind.
    fn storage_transaction<T, E>(
        &mut self,
        f: impl for<'loan> FnOnce(&'loan mut Self, &'loan TransactionContext) -> Result<T, E>,
    ) -> Result<T, E> {
        self.storage_scope(|game, frame| {
            let outcome = frame.lend(|tx| f(game, tx));
            let commit = outcome.is_ok();
            (outcome, commit)
        })
    }
    /// Runs `f` inside a new (or nested) storage transaction and reports whether a requested
    /// commit was refused and rolled back.
    fn storage_transaction_settled<T, E>(
        &mut self,
        f: impl for<'loan> FnOnce(&'loan mut Self, &'loan TransactionContext) -> Result<T, E>,
    ) -> Result<T, mod_api::TransactionError<E>> {
        let (outcome, settled) = self.run_storage_scope(|game, frame| {
            let outcome = frame.lend(|tx| f(game, tx));
            let commit = outcome.is_ok();
            (outcome, commit)
        });
        match (outcome, settled) {
            (Ok(value), StorageSettlement::Committed(_)) => Ok(value),
            (Ok(_), StorageSettlement::Refused | StorageSettlement::Aborted) => {
                Err(mod_api::TransactionError::CommitRefused)
            }
            (Err(error), _) => Err(mod_api::TransactionError::Aborted(error)),
        }
    }
    /// Runs an API scope and reports actual settlement, not just the callback's intent.
    fn scoped_storage<T>(
        &mut self,
        f: impl for<'loan> FnOnce(&'loan mut Self, &'loan TransactionContext) -> Option<T>,
    ) -> Result<Option<T>, TransactionScopeError> {
        let (outcome, settled) = self.run_storage_scope(|game, frame| {
            let outcome = frame.lend(|tx| f(game, tx));
            let commit = outcome.is_some();
            (outcome, commit)
        });
        match settled {
            StorageSettlement::Committed(_) => Ok(outcome),
            StorageSettlement::Aborted => Ok(None),
            StorageSettlement::Refused => Err(TransactionScopeError::CommitRefused),
        }
    }
    /// Whether at least one storage transaction is currently open.
    fn has_open_transaction(&self) -> bool {
        !self.transactions.is_empty()
    }
    /// The innermost open storage transaction's frame identity, if any.
    fn current_storage_frame(&self) -> Option<TransactionFrame> {
        self.transactions.last().and_then(|level| level.frame)
    }
    /// Whether `tx` is the live loan for this game's innermost open transaction.
    fn transaction_context_is_current(&self, tx: &TransactionContext) -> bool {
        self.current_storage_frame()
            .is_some_and(|frame| frame.matches(tx))
    }
    /// `transaction_lifecycle`: the close running, else whether one is open.
    fn transaction_lifecycle(&self) -> TransactionLifecycle {
        match self.transactions.closing {
            Some(closing) => closing,
            None if self.transactions.is_empty() => TransactionLifecycle::None,
            None => TransactionLifecycle::Open,
        }
    }
    fn current_transaction_is_here(&self) -> bool {
        self.transactions
            .last()
            .is_some_and(|frame| frame.dimension == Some(self.dim))
    }
    fn has_level_transaction(&self) -> bool {
        self.transactions
            .iter()
            .any(|level| level.dimension == Some(self.dim))
    }
    /// Closes the innermost transaction, delegating to the active role's handler.
    fn close_storage_transaction(&mut self, commit: bool) -> Option<StorageSettlement> {
        self.side_tx_close_storage_transaction(commit)
    }
    /// Aborts abandoned frames before the next operation can join them. Call only when
    /// there is no caller-owned frame; the tick start also uses this as a backstop.
    fn abort_open_transactions(&mut self) {
        while !self.transactions.is_empty() {
            let _ = self.close_storage_transaction(false);
        }
        // If called during a root close, the enclosing close still owns retirement.
        if self.transactions.closing.is_none() {
            self.world.block_entities.publish_retirement();
        }
    }

    /// `ComposterWrapper.transactionalRandom`: snapshot only the position's private owner,
    /// including its initial state on a first access that later aborts.
    fn take_composter_random(&mut self, pos: [i32; 3]) -> sunrise_core::random::LegacyRandom {
        let epoch = self.native_lifecycle.epoch();
        if self.transactions.composter_epoch != epoch {
            self.clear_composter_randoms();
            self.transactions.composter_epoch = epoch;
        }
        let chunk = (self.dim.key(), (pos[0] >> 4, pos[2] >> 4));
        let key = (pos[0], pos[1], pos[2]);
        let random = self
            .transactions
            .composter_randoms
            .entry(chunk)
            .or_default()
            .remove(&key)
            .unwrap_or_else(|| (*self.world.random).clone());
        if let Some(level) = self.transactions.last_mut() {
            level
                .composter_randoms
                .entry(chunk)
                .or_default()
                .entry(key)
                .or_insert_with(|| random.clone());
        }
        random
    }

    fn put_composter_random(&mut self, pos: [i32; 3], random: sunrise_core::random::LegacyRandom) {
        let common = self.common_mut();
        common
            .transactions
            .composter_randoms
            .entry((common.dim.key(), (pos[0] >> 4, pos[2] >> 4)))
            .or_default()
            .insert((pos[0], pos[1], pos[2]), random);
    }

    /// Retire only an unloading chunk's cached owners and their pending savepoints.
    fn forget_composter_chunk(&mut self, dimension: mod_api::DimensionKey, chunk: [i32; 2]) {
        let key = (dimension, (chunk[0], chunk[1]));
        self.transactions.composter_randoms.remove(&key);
        for level in &mut self.transactions.levels {
            level.composter_randoms.remove(&key);
        }
    }

    fn clear_composter_randoms(&mut self) {
        self.transactions.composter_epoch = None;
        self.transactions.composter_randoms.clear();
        for level in &mut self.transactions.levels {
            level.composter_randoms.clear();
        }
    }
    /// Keeps a pot's saved item and loot metadata until its level commits.
    fn journal_saved_pot(&mut self, pos: Pos) {
        let common = self.common_mut();
        common.transactions.writes = common.transactions.writes.wrapping_add(1);
        if let Some(level) = entities::open_in(&mut common.transactions, common.dim) {
            level.saved_pots.entry(pos).or_insert_with(|| {
                super::spill_common::saved_entity(&common.world, [pos.0, pos.1, pos.2])
            });
            level
                .level_save_gate
                .get_or_insert_with(sunrise_sim::join_seq::visibility::PassLease::enter);
        }
    }

    /// Keeps the block entity at `pos` as it is if a transaction is open.
    fn journal_block_entity(&mut self, pos: Pos) {
        self.side_tx_journal_block_entity(pos);
    }
    /// Puts back a removed block entity and its generation where none stands there now.
    fn restore_removed_block_entity(&mut self, pos: Pos, entity: BlockEntity, generation: u64) {
        let entities = &mut self.world.block_entities;
        if entities.get(&pos).is_none() {
            entities.insert(pos, entity);
            entities.restore_generation(pos, generation);
            entities.reattach_retired(pos, generation);
        }
    }

    /// Records an explicit `BlockEntityRef::remove` inside an open transaction: captures the
    /// original row in `block_entities`, marks `removed_block_entities` with a `None` sentinel
    /// without overwriting any earlier `Some(row)` queued for `preRemoveSideEffects` replay,
    /// and queues the deferred `setRemoved` effects to run at root commit.
    /// Returns `true` if a transaction is open in the current dimension (effects deferred),
    /// or `false` if outside a transaction (caller runs effects immediately).
    fn journal_explicit_remove_block_entity(
        &mut self,
        pos: Pos,
        effects: RemovedBlockEntityEffects,
    ) -> bool {
        self.journal_block_entity(pos);
        let common = self.common_mut();
        let Some(level) = entities::open_in(&mut common.transactions, common.dim) else {
            return false;
        };
        level.removed_block_entities.entry(pos).or_insert(None);
        level.after.push(After::BlockEntityRemoved(effects));
        true
    }
    /// Hides the block entity from tentative reads. The first-write journal can undo all edits;
    /// the removed row itself still contains the edits that matter to removal at commit.
    fn journal_remove_block_entity(&mut self, pos: Pos) {
        self.journal_block_entity(pos);
        let common = self.common_mut();
        let Some(level) = entities::open_in(&mut common.transactions, common.dim) else {
            common.world.block_entities.remove(&pos);
            return;
        };
        if let Some(After::Block(step)) = level.after.last_mut()
            && step.pos == pos
        {
            step.removed = Some(ReplayEntity::take(&mut common.world, pos));
        } else {
            let removed = take_replay_row(&mut common.world, pos);
            let first = level.removed_block_entities.entry(pos).or_default();
            if first.is_none() {
                *first = removed;
            }
        }
    }
    /// How many block entities the open transactions keep whole, and how many single slots:
    /// what a test reads to tell a slot journal from a whole capture without timing it.
    #[cfg(any(test, feature = "test-support"))]
    fn journal_kept(&self) -> (usize, usize) {
        self.transactions
            .iter()
            .fold((0, 0), |(whole, slots), level| {
                (
                    whole + level.block_entities.len(),
                    slots + level.slots.len(),
                )
            })
    }
    /// Keeps item slot `slot` of the block entity at `pos` as it is, if a transaction is open
    /// and keeps neither it nor the whole entity yet. Only for a write that changes that slot
    /// and nothing else of the entity (no cooldown, cooking or saved data beside it), so the
    /// journal holds one stack however large the entity's saved data is. A slot past the end of
    /// the entity's slot list (a mod's list shorter than its container) is a write that grows
    /// the list: the entity is kept whole instead, length and all.
    fn journal_block_entity_slot(&mut self, pos: Pos, slot: usize) {
        let common = self.common_mut();
        common.transactions.writes = common.transactions.writes.wrapping_add(1);
        let entity = common.world.block_entities.get(&pos);
        // Claiming an opaque row changes the saved compound as well as the slot.
        if entity.is_none_or(|entity| slot >= entity.slots().len())
            || matches!(entity, Some(BlockEntity::Mod(m)) if m.has_opaque_item_slot(slot))
        {
            return self.journal_block_entity(pos);
        }
        let Some(level) = entities::open_in(&mut common.transactions, common.dim) else {
            return;
        };
        if level.block_entities.contains_key(&pos) {
            return;
        }
        level.slots.entry((pos, slot)).or_insert_with(|| KeptSlot {
            stack: entity.and_then(|entity| entity.slots()[slot].clone()),
            generation: common.world.block_entities.generation(&pos),
        });
    }
    /// Keeps the inventory of `uuid` on its first write in the open frame.
    fn journal_player(&mut self, uuid: u128) {
        self.side_tx_journal_player(uuid);
    }
    /// Keeps `player`'s menu storage as it is if a transaction is open.
    fn journal_menus(&mut self, player: mod_api::PlayerRef) {
        self.side_tx_journal_menus(player);
    }

    fn journal_mount_drag(&mut self, window: MountClickWindow, before: QuickCraftDrag) {
        if let Some(level) = self.transactions.last_mut() {
            level.mount_drags.entry(window).or_insert(before);
        }
    }

    /// Keeps entity `id`'s inventory as it is, if a transaction is open and hasn't yet.
    fn journal_entity(&mut self, id: u32) {
        self.side_journal_entity(id);
    }
    /// Keeps the scoreboard as it is, if a transaction is open and hasn't yet: a copy of the
    /// whole board, taken on the transaction's first scoreboard write and never otherwise, so a
    /// transaction that writes none pays nothing. An abort puts it back.
    fn journal_scoreboard(&mut self) {
        let common = self.common_mut();
        let Some(level) = common.transactions.last_mut() else {
            return;
        };
        if level.board.is_none() {
            level.board = Some(common.score.board.clone());
        }
    }
    /// Keeps the slots at their first write. Equipment collection records any
    /// actual modifier edits separately; a quiet edit does not run it.
    fn journal_equipment(&mut self, id: u32) {
        let common = self.common_mut();
        let dim = common.dim;
        let Some(level) = common.transactions.last_mut() else {
            return;
        };
        if level.equipment.contains_key(&(dim, id)) {
            return;
        }
        if let Some(mob) = common.mobs.iter().find(|m| m.id == id && !m.removed) {
            level.keep_equipment(dim, id, &mob.extra.equipment, Some(&mob.extra.gear));
        } else if let Some(stand) = common.armor_stands.iter().find(|a| a.id == id) {
            level.keep_equipment(dim, id, &stand.equipment, None);
        }
    }
    /// `drop_plain` by the player whose state is in `Game` now, `uuid`: at once, or at the root
    /// commit inside a transaction.
    fn drop_after_commit(&mut self, uuid: u128, stack: ItemStack) {
        match self.transactions.last_mut() {
            Some(level) => level.after.push(After::PlayerDrop(uuid, stack)),
            None => {
                self.drop_plain(stack);
            }
        }
    }
    /// Set or clear a connection group at `pos`. Inside an open Host transaction, writes
    /// storage quietly, records the prior group so abort restores it silently, and defers
    /// remesh + client sync to root commit. Returns `false` if unloaded, out of height, or
    /// unchanged.
    fn journal_set_connection_group(&mut self, pos: [i32; 3], group: Option<u64>) -> bool {
        let common = self.common_mut();
        let dim = common.dim;
        let Some(level) = entities::open_in(&mut common.transactions, dim) else {
            return common
                .world
                .set_connection_group(pos[0], pos[1], pos[2], group, true);
        };
        let key = (pos[0], pos[1], pos[2]);
        let old = common.world.connection_group(pos[0], pos[1], pos[2]);
        let changed = common
            .world
            .set_connection_group_stored(pos[0], pos[1], pos[2], group);
        if changed {
            level.keep_connection_group(key, old);
            level.after.push(After::ConnectionGroup(key, group));
        }
        changed
    }
    /// Sets the block at `pos` to `state`. In a transaction, stored only, keeping the old
    /// state, and set with its updates at the root commit; otherwise at once
    /// (`Level.setBlock`).
    fn journal_set_block(&mut self, pos: [i32; 3], state: BlockStateId) -> bool {
        self.journal_set_block_flags(pos, state, sunrise_world::level::flags::UPDATE_ALL, false)
    }
    /// The root commit replays one write per position with the combined update flags.
    fn journal_set_block_flags(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
        level_host: bool,
    ) -> bool {
        let placement = self.captures_placement();
        let common = self.common_mut();
        let Some(level) = entities::open_in(&mut common.transactions, common.dim) else {
            return self.level_set_with_flags(pos, state, flags).is_ok();
        };
        let key = (pos[0], pos[1], pos[2]);
        let old = common.world.block(pos[0], pos[1], pos[2]);
        let clears_group = !common.res.behaviours().keeps_block_entity(old, state);
        if !common.world.set_stored(pos[0], pos[1], pos[2], state) {
            return false;
        }
        // Mirror live `chunk_set_block_state`: replacing the block with one that does not keep
        // connection groups clears any tentative group at `pos`, while compatible state changes
        // preserve it.
        let old_group = clears_group
            .then(|| common.world.connection_group(pos[0], pos[1], pos[2]))
            .flatten();
        if old_group.is_some() {
            let _ = common
                .world
                .set_connection_group_stored(pos[0], pos[1], pos[2], None);
        }
        let block = level.blocks.entry(key).or_insert(KeptBlock {
            state: old,
            replay: None,
            strict: false,
        });
        let step = BlockReplay {
            state: old,
            flags,
            level_host,
            replay_removal: false,
            clear_connection_group: clears_group,
        };
        let replay = block.replay.get_or_insert(step);
        replay.flags = merge_block_update_flags(replay.flags, flags);
        replay.level_host = level_host;
        replay.clear_connection_group |= clears_group;
        if old_group.is_some() {
            level.keep_connection_group(key, old_group);
        }
        level.after.push(After::Block(BlockStep {
            pos: key,
            to: state,
            replay: step,
            removed: None,
            placement,
            creation: BlockEntityCreation::Live,
        }));
        true
    }
    /// Hide a replaced block entity from staged reads, without dropping its
    /// contents until the real write. Fluid pickup/pour instead rolls its own
    /// drops against the old entity before journalling the removal.
    fn journal_block_replacement(&mut self, pos: Pos) {
        let placement = self.captures_placement()
            || self
                .world
                .block_entities
                .generation(&pos)
                .is_some_and(|generation| self.is_placement_entity(pos, generation));
        self.journal_remove_block_entity(pos);
        if placement {
            self.world.block_names.remove(&pos);
            self.world.forget_block_entity_source(pos);
            self.side_tx_invalidate_placement_hopper(pos);
        }
        let common = self.common_mut();
        let Some(level) = entities::open_in(&mut common.transactions, common.dim) else {
            return;
        };
        if let Some(After::Block(step)) = level.after.last_mut() {
            step.replay.replay_removal = true;
        }
        if let Some(replay) = level
            .blocks
            .get_mut(&pos)
            .and_then(|block| block.replay.as_mut())
        {
            replay.replay_removal = true;
        }
    }

    fn journal_stage_block_entity(&mut self, pos: Pos, entity: Option<BlockEntity>) {
        if let Some(entity) = entity {
            self.journal_block_entity(pos);
            self.world.block_names.remove(&pos);
            self.world.forget_block_entity_source_unmodified(pos);
            self.world.block_entities.insert(pos, entity);
        }
        let common = self.common_mut();
        let generation = common.world.block_entities.generation(&pos);
        let level = entities::open_in(&mut common.transactions, common.dim).unwrap();
        let Some(After::Block(step)) = level.after.last_mut() else {
            unreachable!("factory staging follows the journaled block write");
        };
        debug_assert_eq!(step.pos, pos);
        step.creation = BlockEntityCreation::Staged {
            generation,
            entity: ReplayEntity::default(),
        };
    }

    fn begin_placement_capture(&mut self) {
        let common = self.common_mut();
        entities::open_in(&mut common.transactions, common.dim)
            .unwrap()
            .capture_placement = true;
    }

    fn captures_placement(&self) -> bool {
        self.transactions
            .iter()
            .any(|frame| frame.dimension == Some(self.dim) && frame.capture_placement)
    }

    /// Snapshot each cell in first-write order, with its state as the listener will see it.
    fn placement_footprint(&mut self) -> Vec<(BlockPos, BlockStateId)> {
        let common = self.common_mut();
        let level = entities::open_in(&mut common.transactions, common.dim).unwrap();
        level.capture_placement = false;
        let mut seen = sunrise_level::fast_hash::FxHashSet::default();
        let mut writes = Vec::new();
        let mut keep = |(x, y, z)| {
            if seen.insert((x, y, z)) {
                writes.push((BlockPos::new(x, y, z), common.world.block(x, y, z)));
            }
        };
        for after in &level.after {
            match after {
                After::Block(step) => keep(step.pos),
                After::BulkBlocks(_, _, _, steps) => steps.keys().copied().for_each(&mut keep),
                _ => {}
            }
        }
        writes
    }

    fn is_placement_entity(&self, pos: Pos, generation: u64) -> bool {
        self.has_journaled_block_entity_creation(pos, generation, true)
    }

    fn has_journaled_block_entity_creation(
        &self,
        pos: Pos,
        generation: u64,
        placement_only: bool,
    ) -> bool {
        self.transactions.iter().any(|frame| {
            frame.dimension == Some(self.dim)
                && frame.blocks.contains_key(&pos)
                && frame.after.iter().rev().any(|after| match after {
                    After::Block(step) => {
                        step.pos == pos
                            && (!placement_only || step.placement)
                            && matches!(step.creation, BlockEntityCreation::Staged {
                            generation: Some(created), ..
                        } if created == generation)
                    }
                    After::BulkBlocks(_, _, _, steps) if !placement_only => steps
                        .get(&pos)
                        .is_some_and(|step| step.created_generation == Some(generation)),
                    _ => false,
                })
        })
    }

    /// A journaled creation is still unpublished until the enclosing transaction commits.
    fn defer_unpublished_block_entity_update(&mut self, pos: Pos) -> bool {
        if let Some(generation) = self.world.block_entities.generation(&pos)
            && self.has_journaled_block_entity_creation(pos, generation, false)
        {
            self.after_commit(After::UpdatePlacedBlockEntity(pos, generation));
            return true;
        }
        false
    }

    fn journal_set_level_random(&mut self, random: LegacyRandom) {
        self.journal_level_random();
        *self.world.random = random;
    }
    fn journal_level_random(&mut self) {
        self.journal_active_level_random(LegacyRandom::clone);
    }

    /// Keep an engine or restricted-writer draw from the stream currently in place.
    fn journal_active_level_random(&mut self, before: impl FnOnce(&LegacyRandom) -> LegacyRandom) {
        let owner = match self.transactions.last() {
            None => return,
            Some(frame) if frame.dimension == Some(self.dim) => None,
            Some(_) => {
                let Some(epoch) = self.native_lifecycle.epoch() else {
                    return;
                };
                let Some(level_ref) = self.side_active_level_ref(epoch) else {
                    return;
                };
                Some(level_ref)
            }
        };
        let common = self.common_mut();
        if let Some(frame) = common.transactions.last_mut() {
            frame.keep_level_random(owner, || before(&common.world.random));
        }
    }
    fn journal_orb_merge(&mut self, join: sunrise_sim::join_seq::JoinSeq, fields: (i32, u32, i32)) {
        let dim = self.dim;
        if let Some(level) = self.transactions.last_mut() {
            level.orbs.entry((dim, join)).or_insert(fields);
        }
    }
    fn journal_station_click(
        &mut self,
        conn: u32,
        owner: decision::MenuId,
        station: &super::lan_menu_data::GuestStation,
    ) {
        if let Some(level) = self.transactions.last_mut() {
            level
                .stations
                .entry((conn, owner))
                .or_insert_with(|| station.clone());
        }
    }
    /// Keep the original cell in the transaction for the level in place.
    fn journal_biome(&mut self, pos: [i32; 3], old: u16) {
        let dim = self.dim;
        if let Some(level) = entities::open_in(&mut self.transactions, dim) {
            level
                .biomes
                .entry((dim, (pos[0] >> 2, pos[1] >> 2, pos[2] >> 2)))
                .or_insert(old);
        }
    }
    /// Mark a written chunk unsaved at commit, without issuing any notice.
    fn journal_chunk_save(&mut self, chunk: (i32, i32), scope: SaveChunkScope) {
        let dim = self.dim;
        let level = match scope {
            SaveChunkScope::Level => entities::open_in(&mut self.transactions, dim),
            SaveChunkScope::Transaction => self.transactions.last_mut(),
        };
        if let Some(level) = level {
            level.save_chunks.insert((dim, chunk, scope));
        }
    }
    /// Queue a reusable interaction effect for the outermost successful commit.
    fn defer_effect(&mut self, effect: DeferredEffect) {
        self.after_commit(After::Effect(effect));
    }
    /// Collect committed implicit `setChanged` notices while the callback runs without opening
    /// a transaction. Direct calls to `BlockEntityRef::set_changed` remain immediate.
    fn collect_quick_move_notices<R>(
        &mut self,
        dispatch: impl FnOnce(&mut Self) -> R,
    ) -> (R, Vec<QuickMoveNotice>) {
        let outer = self.transactions.quick_move_notices.replace(Vec::new());
        let scope = QuickMoveNotices::new(self, outer);
        let result = dispatch(&mut *scope.game);
        let notices = scope
            .game
            .transactions
            .quick_move_notices
            .take()
            .expect("active quick-move notice scope");
        (result, notices)
    }

    /// Journal a container notice already bound to the row that received the write.
    fn after_holder_changed(&mut self, holder: BlockEntityIdentity) {
        let [x, y, z] = holder.position;
        self.after_commit(self.side_tx_container_change_effect((x, y, z), Some(holder)));
    }

    /// Runs `after` at once, or at the root commit inside a transaction.
    fn after_commit(&mut self, after: After) {
        let after = match after {
            After::Changed(pos) => self.side_tx_container_change_effect(pos, None),
            after => after,
        };
        let common = self.common_mut();
        let frame = if after.level_owned() {
            entities::open_in(&mut common.transactions, common.dim)
        } else {
            common.transactions.last_mut()
        };
        match frame {
            Some(level) => level.after.push(after),
            None => self.side_tx_run_after(after),
        }
    }
    fn pending_menu_data(&self, key: (u32, u64, usize)) -> Option<i32> {
        self.transactions
            .iter()
            .rev()
            .find_map(|level| level.menu_data.get(&key).copied())
    }

    fn journal_menu_data(&mut self, key: (u32, u64, usize), value: i32) -> bool {
        let Some(level) = self.transactions.last_mut() else {
            return false;
        };
        level.menu_data.insert(key, value);
        true
    }
}
impl<'g, T: crate::game::SideGame<'g>> TransactionsGameExt<'g> for T {}

include!("transactions/item_entity.rs");

use crate::game::side_ops::events::EventsSideGameExt as _;

use crate::game::side_ops::world::WorldSideGameExt as _;
