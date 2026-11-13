//! The enchanting power around an enchanting table (`EnchantmentMenu.slotsChanged`): the
//! bookshelves and the blocks whose parts answer `enchant_power_bonus`.
use super::Game;
use horizon_client::stations::bookshelf_offsets;
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;

impl Game<'_> {
    /// The power around the table at `pos`: the sum, over `EnchantingTableBlock.BOOKSHELF_OFFSETS`
    /// whose block between it and the table is in `#minecraft:enchantment_power_transmitter`, of
    /// each block's `getEnchantPowerBonus`, truncated as NeoForge's `(int) power`. The offers cap
    /// it at 15.
    pub(super) fn count_bookshelves(&self, pos: [i32; 3]) -> i32 {
        let tags = &self.items().block_tags;
        let facts = self.res.facts();
        let is = |state: BlockStateId, tag: &str| {
            let name = &facts.get(state).name;
            tags.get(tag).is_some_and(|t| t.contains(&**name))
        };
        let table = BlockPos::new(pos[0], pos[1], pos[2]);
        let power: f32 = bookshelf_offsets()
            .into_iter()
            .filter(|o| {
                is(
                    self.world
                        .block(pos[0] + o[0] / 2, pos[1] + o[1], pos[2] + o[2] / 2),
                    "minecraft:enchantment_power_transmitter",
                )
            })
            .map(|o| {
                let at = BlockPos::new(pos[0] + o[0], pos[1] + o[1], pos[2] + o[2]);
                let state = self.world.block(at.x, at.y, at.z);
                let own = if is(state, "minecraft:enchantment_power_provider") {
                    1.0
                } else {
                    0.0
                };
                self.enchant_power(state, at, table, own)
            })
            .sum();
        // A negative sum would make vanilla's `nextInt(bookcases + 1)` throw.
        (power as i32).max(0)
    }

    /// `IBlockStateExtension.getEnchantPowerBonus` of `state` at `pos`: `own`, unless a part of
    /// the state answers; a non-finite answer keeps `own`.
    fn enchant_power(&self, state: BlockStateId, pos: BlockPos, table: BlockPos, own: f32) -> f32 {
        if !super::api_values::block_state::block_listens(mod_api::HookId::EnchantPowerBonus, state)
        {
            return own;
        }
        let mut power = own;
        self.native_enchant_power_bonus(state, pos, table, &mut power);
        if power.is_finite() { power } else { own }
    }
}
