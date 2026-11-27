use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEngine};
use sunrise_items::items::ItemStack as EngineStack;

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn api_grindstone_place(
        &mut self,
        player: u128,
        entity: i32,
        pos: [i32; 3],
        inputs: [Option<&EngineStack>; 3],
        cancel: &mut bool,
    ) -> sunrise_sim::menu::GrindstonePlaceEdits {
        use mod_api::item::menu::{GrindstonePlaceEdits, GrindstonePlaceInputs};

        let Ok([top, bottom, vanilla_output]) = inputs.try_into_api() else {
            return Default::default();
        };
        let inputs = GrindstonePlaceInputs {
            top: top.as_ref(),
            bottom: bottom.as_ref(),
            vanilla_output: vanilla_output.as_ref(),
        };
        let mut edits = GrindstonePlaceEdits {
            output: None,
            xp: -1,
        };
        let tx_depth = self.common.transactions.len();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.native_grindstone_place(player, entity, pos, &inputs, &mut edits, cancel);
        }))
        .is_err();
        self.abort_abandoned_transactions(tx_depth);
        if panicked {
            log::error!("grindstone_place: listener panicked at {pos:?}");
            *cancel = false;
            return Default::default();
        }
        if *cancel {
            return Default::default();
        }
        let xp = edits.xp.max(-1);
        let output = match edits.output.try_into_engine(self.items()) {
            Ok(output) => output,
            Err(err) => {
                log::warn!(
                    "grindstone_place: ignoring invalid output replacement at {pos:?}: {err:?}"
                );
                None
            }
        };
        sunrise_sim::menu::GrindstonePlaceEdits { output, xp }
    }
    pub(in crate::game) fn api_grindstone_take(
        &mut self,
        player: u128,
        entity: i32,
        pos: [i32; 3],
        take: &sunrise_sim::menu::PendingGrindstoneTake,
        cancel: &mut bool,
    ) -> sunrise_sim::menu::GrindstoneTakeEdits {
        use mod_api::item::menu::{GrindstoneTakeEdits, GrindstoneTakeInputs};

        let initial_xp = if take.xp >= 0 {
            take.xp
        } else {
            let amount = take.vanilla_amount(&self.menu_env());
            self.journal_level_random();
            sunrise_sim::stations::grindstone_xp_roll(amount, &mut self.common.world.random) as i32
        };
        let Ok([top, bottom]) = [take.top.as_ref(), take.bottom.as_ref()].try_into_api() else {
            return sunrise_sim::menu::GrindstoneTakeEdits::initial(initial_xp);
        };
        let inputs = GrindstoneTakeInputs {
            top: top.as_ref(),
            bottom: bottom.as_ref(),
        };
        let mut edits = GrindstoneTakeEdits {
            xp: initial_xp,
            new_top: None,
            new_bottom: None,
        };
        let tx_depth = self.common.transactions.len();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.native_grindstone_take(player, entity, pos, &inputs, &mut edits, cancel);
        }))
        .is_err();
        self.abort_abandoned_transactions(tx_depth);
        if panicked {
            log::error!("grindstone_take: listener panicked at {pos:?}");
            *cancel = false;
            return sunrise_sim::menu::GrindstoneTakeEdits::initial(initial_xp);
        }
        if *cancel {
            return sunrise_sim::menu::GrindstoneTakeEdits::initial(initial_xp);
        }
        let new_top = match edits.new_top.try_into_engine(self.items()) {
            Ok(stack) => stack,
            Err(err) => {
                log::warn!("grindstone_take: ignoring invalid top remainder at {pos:?}: {err:?}");
                None
            }
        };
        let new_bottom = match edits.new_bottom.try_into_engine(self.items()) {
            Ok(stack) => stack,
            Err(err) => {
                log::warn!(
                    "grindstone_take: ignoring invalid bottom remainder at {pos:?}: {err:?}"
                );
                None
            }
        };
        sunrise_sim::menu::GrindstoneTakeEdits {
            xp: edits.xp,
            new_top,
            new_bottom,
        }
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn api_block_entity_update_tag(
        pos: [i32; 3],
        entity: &sunrise_level::containers::ModBlockEntity,
        tag: &mut Option<sunrise_core::nbt::Compound>,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityUpdateTag) {
            return;
        }
        let data =
            sunrise_gameplay_common::game::native_hooks::ResidentReadBlockEntity::stored(entity);
        sunrise_gameplay_common::game::native_hooks::native_block_entity_update_tag(
            pos,
            mod_api::BlockEntityDataView::new(&data),
            tag,
        );
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
