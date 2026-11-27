//! `HoneycombItem.getWaxed` for the player's and the dispenser's honeycomb, and the axe's scrape
//! and wax-off from the world's NeoForge block maps.
use super::native_block_tables::BlockTables;
use sunrise_world::registry::BlockStateId;

impl crate::game::CommonGame<'_> {
    /// The waxed twin of `state`; see [`BlockTables::honeycomb_waxed`].
    pub fn honeycomb_waxed(&self, state: BlockStateId) -> Option<BlockStateId> {
        BlockTables::new(&self.res, &self.pack).honeycomb_waxed(state)
    }

    /// The axe's scrape or wax-off of `block` from the world's NeoForge block maps, asked
    /// before the axe's transform table: the derived `neoforge:oxidizables` inverse
    /// (`DataMapHooks.getPreviousOxidizedStage`), then the `neoforge:waxables` inverse
    /// (`getBlockUnwaxed`). None for another tool or a block the maps leave out.
    pub fn axe_mapped(
        &self,
        transformer: &str,
        block: BlockStateId,
    ) -> Option<(BlockStateId, mod_api::ToolAbility)> {
        if transformer != "minecraft:axe" {
            return None;
        }
        let behaviours = self.res.behaviours();
        let scraped = behaviours.mapped_scraped(block);
        scraped
            .map(|state| (state, mod_api::ToolAbility::AxeScrape))
            .or_else(|| {
                let unwaxed = behaviours.mapped_unwaxed(block)?;
                Some((unwaxed, mod_api::ToolAbility::AxeWaxOff))
            })
    }
}
