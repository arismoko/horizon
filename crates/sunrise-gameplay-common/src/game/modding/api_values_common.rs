//! Shared API value conversion hooks callable from both server and client roles.
use super::api_values::{TryIntoApi, TryIntoEngine};
use crate::game::native_hooks::anvil_update::GameHooks as _;

pub trait ApiValuesCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    fn api_anvil_update(
        &mut self,
        player: u128,
        entity: i32,
        pos: [i32; 3],
        inputs: sunrise_sim::menu::AnvilUpdateInputs<'_>,
        result: &mut sunrise_sim::stations::AnvilResult,
    ) {
        use mod_api::item::menu::{AnvilUpdateEdits, AnvilUpdateInputs, StackResult};

        let Ok(left) = inputs.left.try_into_api() else {
            return;
        };
        let Ok(right) = inputs.right.try_into_api() else {
            return;
        };
        let Ok(vanilla_output) = result.result.try_into_api() else {
            return;
        };
        let inputs = AnvilUpdateInputs {
            left: &left,
            right: right.as_ref(),
            name: inputs.name,
            vanilla_output: vanilla_output.as_ref(),
            vanilla_xp_cost: result.cost,
            vanilla_material_cost: i32::try_from(result.repair_item_count_cost).unwrap_or(i32::MAX),
        };
        let mut edits = AnvilUpdateEdits {
            output: StackResult::Keep,
            xp_cost: inputs.vanilla_xp_cost,
            material_cost: inputs.vanilla_material_cost,
        };
        let mut cancel = false;
        self.native_anvil_update(player, entity, pos, &inputs, &mut edits, &mut cancel);
        if cancel {
            *result = Default::default();
            return;
        }
        // An equal replacement keeps the original engine stack, even if a mod could not write it.
        if let StackResult::Set(output) = edits.output
            && output != vanilla_output
        {
            match output.try_into_engine(self.items()) {
                Ok(output) => {
                    result.only_renaming &= result.result == output;
                    result.result = output;
                }
                Err(err) => {
                    log::warn!(
                        "anvil_update: ignoring invalid output replacement at {pos:?}: {err:?}"
                    );
                }
            }
        }
        result.cost = edits.xp_cost.max(0);
        result.repair_item_count_cost = edits.material_cost.max(0) as u32;
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> ApiValuesCommonGameExt1<'__sg> for G {}

/// The saved block entity's payload, with its world identity and position removed.
pub(in crate::game) fn block_entity_payload(
    mut data: sunrise_core::nbt::Compound,
) -> sunrise_core::nbt::Compound {
    for key in ["id", "x", "y", "z"] {
        data.remove(&key.into());
    }
    data
}

use crate::game::tables::GameItems as _;
