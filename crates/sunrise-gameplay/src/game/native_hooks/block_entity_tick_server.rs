use super::*;
use mod_api::{
    BlockEntityAccess, BlockEntityItemError, BlockEntityRef, BlockEntityScope, BlockEntityView,
    ResidentBlockEntity, ServerHost, TransactionContext, TransactionScopeError,
    TransactionUnavailable,
};
use std::collections::HashMap;
use sunrise_core::nbt::{Compound, JavaString, Tag};
use sunrise_gameplay_common::game::{
    Dim,
    api_values::{TryIntoApi, TryIntoEngine},
    transactions::OpenTransactions,
};
use sunrise_gameplay_common::hoppers::HopperState;
use sunrise_items::items::Items;
use sunrise_level::{
    block_entities::BlockEntityLoan, chunk_map::ChunkMap, fast_hash::FxHashMap,
    world::ModifiedChunks,
};

/// Dispatch the one resident server ticker; the scope owns access to the Host.
pub(in crate::game) fn native_block_entity_tick(
    kind: mod_api::BlockEntityTypeKey,
    subject: mod_api::BlockEntityScope<'_>,
) {
    crate::game::native_hooks::fire::fire_block_entity_tick(kind, subject);
}

pub(in crate::game) fn valid_tick_subject(block: &str, entity: &EngineBlockEntity) -> bool {
    sunrise_game_data::block_entity_types::valid(block, entity.id()) == Some(true)
}

#[derive(Clone, Copy)]
pub enum Loan {
    // Reuse the ticker's validity result for this instance and state.
    Tick(Option<BlockStateId>),
    Menu(BlockStateId),
    OnLoad,
}

pub struct ResidentBlockEntityAccess<'g, 'w> {
    pub game: &'g mut crate::game::ServerGame<'w>,
    pub(in crate::game) subject: Option<BlockEntityRef>,
    pub(in crate::game) loan: Loan,
    pub(in crate::game) items_changed: bool,
}

impl<'g, 'w> ResidentBlockEntityAccess<'g, 'w> {
    pub fn new(
        game: &'g mut crate::game::ServerGame<'w>,
        subject: Option<BlockEntityRef>,
        loan: Loan,
    ) -> Self {
        Self {
            game,
            subject,
            loan,
            items_changed: false,
        }
    }

    pub(in crate::game) fn on_load(
        game: &'g mut crate::game::ServerGame<'w>,
        subject: BlockEntityRef,
    ) -> Self {
        Self::new(game, Some(subject), Loan::OnLoad)
    }
}

pub(in crate::game) struct ServerBlockEntityData<'a> {
    pub(in crate::game) dim: Dim,
    pub(in crate::game) pos: Pos,
    pub(in crate::game) loan: BlockEntityLoan<'a>,
    pub(in crate::game) slot_count: usize,
    pub(in crate::game) items: &'a Items,
    pub(in crate::game) journal: &'a mut OpenTransactions,
    pub(in crate::game) chunks: &'a ChunkMap,
    pub(in crate::game) names: &'a FxHashMap<Pos, Tag>,
    pub(in crate::game) hoppers: &'a HashMap<Pos, HopperState>,
    pub(in crate::game) menu: Option<(&'a mut ModifiedChunks, &'a mut bool)>,
    pub(in crate::game) dirty: bool,
}

impl ServerBlockEntityData<'_> {
    pub(in crate::game) fn entity(&self) -> &EngineBlockEntity {
        self.loan.entity
    }

    pub(in crate::game) fn edit(&mut self, saved: bool) -> &mut EngineBlockEntity {
        self.dirty |= saved;
        self.journal.journal_block_entity_loan(
            self.dim,
            self.pos,
            &self.loan,
            self.chunks,
            self.names.get(&self.pos),
            self.hoppers.get(&self.pos).copied(),
        );
        self.loan.entity
    }
}

impl Drop for ServerBlockEntityData<'_> {
    fn drop(&mut self) {
        if self.dirty
            && let Some((modified, _)) = &mut self.menu
        {
            let (x, _, z) = self.pos;
            self.journal
                .mark_scoped_menu_write(self.dim, (x >> 4, z >> 4), modified);
        }
    }
}

impl ResidentBlockEntity for ServerBlockEntityData<'_> {
    fn id(&self) -> &str {
        self.entity().id()
    }
    fn data_slots(&self) -> &[i32] {
        &self.entity().data
    }

    fn set_data_slot(&mut self, slot: u32, value: i32) -> bool {
        let index = slot as usize;
        if self
            .entity()
            .data
            .get(index)
            .is_none_or(|old| *old == value)
        {
            return false;
        }
        self.edit(false).data[index] = value;
        true
    }

    fn set_data_slots(&mut self, data: &[i32]) -> bool {
        if self.entity().data == data {
            return false;
        }
        let dest = &mut self.edit(false).data;
        dest.clear();
        dest.extend_from_slice(data);
        true
    }

    fn nbt(&self) -> &Compound {
        &self.entity().nbt
    }

    fn set_nbt(&mut self, key: JavaString, value: Option<Tag>) -> bool {
        let is_items = key.0.iter().copied().eq("Items".encode_utf16());
        if ["id", "x", "y", "z", sunrise_level::containers::SHAPES_KEY]
            .iter()
            .any(|reserved| key.0.iter().copied().eq(reserved.encode_utf16()))
            || (is_items && self.slot_count > 0)
            || value.as_ref().is_some_and(|tag| {
                matches!(tag, Tag::End) || sunrise_game_data::nbt_path::too_deep(tag, 1)
            })
        {
            return false;
        }
        let unchanged = if is_items && !self.entity().items.is_empty() {
            sunrise_sim::save::block_entity_encode::mod_items(self.entity(), self.items) == value
        } else {
            self.entity().nbt.get(&key) == value.as_ref()
        };
        if unchanged {
            return false;
        }
        let entity = self.edit(true);
        if is_items {
            entity.items.clear();
        }
        match value {
            Some(tag) => {
                entity.nbt.insert(key, tag);
            }
            None => {
                entity.nbt.remove(&key);
            }
        }
        true
    }

    fn item_slots(&self) -> usize {
        self.slot_count
    }

    fn item_slot(&self, slot: u32) -> Option<(&str, u32)> {
        let index = slot as usize;
        if index >= self.slot_count {
            return None;
        }
        let stack = self.entity().items.get(index)?.as_ref()?;
        Some((&stack.item, stack.count))
    }

    fn copy_item_slot(&self, slot: u32) -> Option<mod_api::ItemStack> {
        self.entity()
            .items
            .get(slot as usize)?
            .as_ref()?
            .try_into_api()
            .ok()
    }

    fn set_item_slot(
        &mut self,
        slot: u32,
        stack: Option<mod_api::ItemStack>,
    ) -> Result<bool, BlockEntityItemError> {
        let index = slot as usize;
        let slots = self.slot_count;
        if index >= slots {
            return Err(BlockEntityItemError::SlotOutOfBounds {
                slot,
                slots: slots as u32,
            });
        }
        let stack = stack
            .as_ref()
            .map(|stack| stack.try_into_engine(self.items))
            .transpose()?;
        let entity = self.entity();
        if entity.items.get(index).and_then(Option::as_ref) == stack.as_ref()
            && !entity.has_opaque_item_slot(index)
        {
            return Ok(false);
        }
        if let Some((_, changed)) = &mut self.menu {
            **changed = true;
        }
        let entity = self.edit(true);
        *entity.item_slot_mut(index) = stack;
        Ok(true)
    }

    fn set_client_update_fields(&mut self, fields: Compound) {
        if !fields.is_empty() || self.entity().has_client_update_fields() {
            self.edit(false).set_client_update_fields(fields);
        }
    }
}

impl BlockEntityAccess for ResidentBlockEntityAccess<'_, '_> {
    fn with_view(&mut self, call: &mut dyn FnMut(BlockEntityView<'_>)) {
        use mod_api::__private::game::Fields;
        let Some(subject) = self.subject else { return };
        let target = subject.fields();
        let [x, y, z] = target.position;
        if !self.game.level_in_place(target.level) || !self.game.world.loaded(x, z) {
            return;
        }
        let live_state = self.game.world.block(x, y, z);
        if matches!(self.loan, Loan::Menu(admitted) if admitted != live_state) {
            return;
        }
        let Ok(state) = live_state.try_into_api() else {
            return;
        };
        let slot_count = self
            .game
            .world
            .block_entities
            .mod_container(live_state.0)
            .map_or(0, |c| c.slots);
        let pos = (x, y, z);
        let common = &mut self.game.common;
        let Some(loan) = common
            .world
            .block_entities
            .loan_mod_entity(pos, target.generation)
        else {
            return;
        };
        if let Loan::Tick(checked) = &mut self.loan
            && *checked != Some(live_state)
        {
            if !valid_tick_subject(&common.res.facts().get(live_state).name, loan.entity) {
                return;
            }
            *checked = Some(live_state);
        }
        let mut data = ServerBlockEntityData {
            dim: common.dim,
            pos,
            loan,
            slot_count,
            items: common.res.items(),
            journal: &mut common.transactions,
            chunks: &common.world.chunks,
            names: &common.world.block_names,
            hoppers: &self.game.server.hoppers,
            menu: matches!(self.loan, Loan::Menu(_))
                .then_some((&mut common.world.modified, &mut self.items_changed)),
            dirty: false,
        };
        call(BlockEntityView::new(subject, state, &mut data));
    }

    fn host(&mut self) -> &mut dyn ServerHost {
        self.game
    }

    fn with_current_transaction_scope(
        &mut self,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn BlockEntityAccess,
            &'loan TransactionContext,
        ),
    ) -> Result<(), TransactionUnavailable> {
        let frame = self
            .game
            .current_storage_frame()
            .ok_or(TransactionUnavailable)?;
        frame.lend(|tx| callback(self, tx));
        Ok(())
    }

    fn with_transaction_scope(
        &mut self,
        call: &mut dyn for<'loan> FnMut(
            &'loan mut dyn BlockEntityAccess,
            &'loan TransactionContext,
        ) -> bool,
    ) -> Result<bool, TransactionScopeError> {
        let (subject, loan) = (self.subject, self.loan);
        let committed = self.game.scoped_storage(|game, tx| {
            let mut child = ResidentBlockEntityAccess::new(game, subject, loan);
            call(&mut child, tx).then_some(child.items_changed)
        })?;
        if let Some(changed) = committed {
            self.items_changed |= changed;
        }
        Ok(committed.is_some())
    }
}

impl crate::game::ServerGame<'_> {
    /// The resident mod block entity and admitted block state behind a menu callback.
    pub fn resident_block_entity_target(&self, pos: Pos) -> Option<(BlockEntityRef, BlockStateId)> {
        let level = self.level(self.common.dim.id())?;
        let handle = self.block_entity(level, [pos.0, pos.1, pos.2])?;
        matches!(
            self.world.block_entities.get(&pos),
            Some(BlockEntity::Mod(_))
        )
        .then(|| (handle, self.common.world.block(pos.0, pos.1, pos.2)))
    }
    /// `LevelChunk.BoundTickingBlockEntity.tick` for a resident mod block entity.
    /// Returns whether the ticker was dispatched, so skipped rows have no after-slot.
    pub(crate) fn tick_mod_block_entity_loaded(&mut self, pos: Pos) -> bool {
        use sunrise_level::block_entities::Ticking;

        if !self.common.world.block_entities.has_states() {
            self.bind_server_ticking_states();
        }
        let Some(BlockEntity::Mod(entity)) = self.common.world.block_entities.get(&pos) else {
            return false;
        };
        let Some(kind) = entity.tick_kind() else {
            return false;
        };
        let Some(generation) = self.common.world.block_entities.generation(&pos) else {
            return false;
        };
        let live_state = self.common.world.block(pos.0, pos.1, pos.2);
        // A typed ticker is only bound to a registered type with a numeric id.
        let numeric = entity.numeric_id().expect("registered ticker type");
        match self
            .common
            .world
            .block_entities
            .mod_ticking(live_state.0, numeric)
        {
            Ticking::Yes => {}
            Ticking::Idle => return false,
            Ticking::No => {
                if let Some(id) = self
                    .common
                    .world
                    .block_entities
                    .record_invalid_mod_tick(pos, generation)
                {
                    let block = &self.common.res.facts().get(live_state).name;
                    log::warn!("Block entity {id} at {pos:?} cannot tick on {block}");
                }
                return false;
            }
        }
        let Some(level) = self.level(self.common.dim.id()) else {
            return false;
        };
        let subject =
            mod_api::__private::game::block_entity_ref(level, [pos.0, pos.1, pos.2], generation);
        let mut access =
            ResidentBlockEntityAccess::new(self, Some(subject), Loan::Tick(Some(live_state)));
        native_block_entity_tick(kind, BlockEntityScope::new(&mut access));
        self.common
            .world
            .block_entities
            .clear_ticker_warning(pos, generation);
        true
    }
}

impl<'g, 'w> ResidentBlockEntityAccess<'g, 'w> {
    pub fn blockless(game: &'g mut crate::game::ServerGame<'w>) -> Self {
        Self::new(game, None, Loan::OnLoad)
    }
    pub fn for_handle(
        game: &'g mut crate::game::ServerGame<'w>,
        subject: BlockEntityRef,
        state: BlockStateId,
    ) -> Self {
        Self::new(game, Some(subject), Loan::Menu(state))
    }
    pub fn items_changed(&self) -> bool {
        self.items_changed
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
