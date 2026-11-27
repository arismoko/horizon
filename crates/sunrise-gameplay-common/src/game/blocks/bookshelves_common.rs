//! Common enchanting-table bookshelf power queries (`EnchantmentMenu.slotsChanged`).
use super::tables::BlockProviderQuery;
use crate::game::native_hooks::enchant_power_bonus::GameHooks as _;
use crate::game::native_hooks::is_valid_book_shelf::GameHooks as _;
use mod_api::LevelRef;
use sunrise_core::position::BlockPos;
use sunrise_sim::stations::bookshelf_offsets;
use sunrise_world::registry::BlockStateId;

pub trait BookshelvesCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// The enchanting power around the table at `pos`, truncated as NeoForge's `(int) power`.
    fn count_bookshelves(&self, pos: [i32; 3]) -> i32 {
        let table = BlockPos::new(pos[0], pos[1], pos[2]);
        let level = self.native_current_level();
        let hooked =
            level.filter(|_| mod_api::__private::listened(mod_api::HookId::IsValidBookShelf));
        let power: f32 = bookshelf_offsets()
            .filter_map(|o| {
                let at = BlockPos::new(
                    pos[0].wrapping_add(o[0]),
                    pos[1].wrapping_add(o[1]),
                    pos[2].wrapping_add(o[2]),
                );
                let transmits = self.enchant_transmits(self.world.block(
                    pos[0].wrapping_add(o[0] / 2),
                    at.y,
                    pos[2].wrapping_add(o[2] / 2),
                ));
                let Some(hook_level) = hooked else {
                    return transmits
                        .then(|| {
                            let state = self.world.block(at.x, at.y, at.z);
                            self.enchant_power_for_table(level, state, at, table)
                        })
                        .flatten();
                };
                let state = self.world.block(at.x, at.y, at.z);
                let power = self.enchant_power_for_table(hooked, state, at, table);
                let mut valid = transmits && power.is_some_and(|power| power != 0.0);
                let offset = BlockPos::new(o[0], o[1], o[2]);
                self.native_is_valid_book_shelf(hook_level, table, offset, &mut valid);
                valid.then_some(power).flatten()
            })
            .sum();
        (power as i32).max(0)
    }

    fn enchant_transmits(&self, state: BlockStateId) -> bool {
        self.items()
            .block_tags
            .get("minecraft:enchantment_power_transmitter")
            .is_some_and(|tag| tag.contains(&*self.res.facts().get(state).name))
    }

    /// Raw provider answer. The explicit read returns it as given.
    fn enchant_power(
        &self,
        level: Option<LevelRef>,
        state: BlockStateId,
        pos: BlockPos,
        table: BlockPos,
    ) -> Option<f32> {
        let mut power = self.default_enchant_power(state);
        if !super::api_values::block_state::block_listens(mod_api::HookId::EnchantPowerBonus, state)
        {
            return Some(power);
        }
        self.with_block_provider_query(state, BlockProviderQuery::EnchantPower, || {
            self.native_enchant_power_bonus(level, state, pos, table, &mut power);
            power
        })
    }

    fn default_enchant_power(&self, state: BlockStateId) -> f32 {
        self.items()
            .default_enchant_power(&self.res.facts().get(state).name)
    }

    /// Offers and particles ignore non-finite overrides and keep the tag default.
    fn enchant_power_for_table(
        &self,
        level: Option<LevelRef>,
        state: BlockStateId,
        pos: BlockPos,
        table: BlockPos,
    ) -> Option<f32> {
        self.enchant_power(level, state, pos, table).map(|power| {
            if power.is_finite() {
                power
            } else {
                self.default_enchant_power(state)
            }
        })
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> BookshelvesCommonGameExt1<'__sg> for G {}

use crate::game::tables::GameItems as _;

use crate::game::tables::WorldTablesExt as _;

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
