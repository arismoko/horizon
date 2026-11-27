use super::ServerGame;
use mod_api::__private::game::*;
use mod_api::*;
use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEnginePreservingCount};
use sunrise_gameplay_common::game::item_snapshot::ItemSnapshotGameExt1 as _;

sunrise_gameplay_common::impl_shared_phase_contexts!(ServerGame<'_>);

impl Host for ServerGame<'_> {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(BackendRef::of(self))
    }
}

impl ServerHost for ServerGame<'_> {
    fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
        Some(BackendMutRef::of(self))
    }

    fn with_current_transaction_scope(
        &mut self,
        callback: &mut dyn for<'loan> FnMut(&'loan mut dyn ServerHost, &'loan TransactionContext),
    ) -> Result<(), TransactionUnavailable> {
        let frame = self.current_storage_frame().ok_or(TransactionUnavailable)?;
        frame.lend(|tx| callback(self, tx));
        Ok(())
    }

    fn with_transaction_scope(
        &mut self,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn ServerHost,
            &'loan TransactionContext,
        ) -> bool,
    ) -> Result<bool, TransactionScopeError> {
        self.scoped_storage(|game, tx| callback(game, tx).then_some(()))
            .map(|settled| settled.is_some())
    }

    fn with_scratch_stack_scope(
        &mut self,
        stack: ItemStack,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn ServerHost,
            &'loan TransactionContext,
            &'loan ItemPlace,
        ) -> bool,
    ) -> Result<Option<Option<ItemStack>>, ScratchScopeError> {
        let engine_stack = self
            .stack_boundary_items()
            .ok_or(StackWriteError::Unavailable)
            .and_then(|items| stack.try_into_engine_preserving_count(&items))
            .map_err(ScratchScopeError::Stack)?;
        let taken = self
            .scratch_scope(
                Some(engine_stack.item.clone()),
                engine_stack,
                |game, tx, place| callback(game, tx, place),
            )
            .map_err(ScratchScopeError::Transaction)?;
        Ok(taken.map(|taken| taken.stack.and_then(|stack| stack.try_into_api().ok())))
    }

    fn with_scratch_slots_scope(
        &mut self,
        stack: ItemStack,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn ServerHost,
            &'loan TransactionContext,
            &'loan ItemPlace,
        ) -> bool,
    ) -> Result<Option<(Option<ItemStack>, Option<ItemStack>)>, ScratchScopeError> {
        let engine_stack = self
            .stack_boundary_items()
            .ok_or(StackWriteError::Unavailable)
            .and_then(|items| stack.try_into_engine_preserving_count(&items))
            .map_err(ScratchScopeError::Stack)?;
        let taken = self
            .scratch_scope(None, engine_stack, |game, tx, place| {
                callback(game, tx, place)
            })
            .map_err(ScratchScopeError::Transaction)?;
        Ok(taken.map(|taken| {
            (
                taken.stack.and_then(|stack| stack.try_into_api().ok()),
                taken.overflow.and_then(|stack| stack.try_into_api().ok()),
            )
        }))
    }

    fn transaction_matches(&self, expected: Option<&TransactionContext>) -> bool {
        match (self.current_storage_frame(), expected) {
            (Some(frame), Some(tx)) => frame.matches(tx),
            (None, None) => true,
            _ => false,
        }
    }
}

impl Backend for ServerGame<'_> {}
impl BackendMut for ServerGame<'_> {}
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::transactions::scratch::ScratchGameExt1 as _;

sunrise_gameplay_common::game::native_tags::impl_tags!(
    ServerGame<'_>,
    game,
    sunrise_gameplay_common::game::tables::WorldTablesExt::native_tag_items(game),
    sunrise_gameplay_common::game::tables::WorldTablesExt::loaded_biome_keys(game)
);

include!(concat!(env!("OUT_DIR"), "/game_host_generated.rs"));

pub(crate) mod services;
