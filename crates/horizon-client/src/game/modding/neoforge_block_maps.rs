//! NeoForge's built-in block data maps (`NeoForgeDataMaps`): `neoforge:flammable`,
//! `neoforge:oxidizables` and `neoforge:waxables`, merged from the world's
//! data packs and mods' files as a mod's own data maps are, and installed on
//! the world's behaviours for fire spread, weathering, scraping and waxing.
use super::Game;
use super::native_data_maps::{DataMapType, DataMaps};
use horizon_world::level::BlockMaps;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The three maps' ids, all on `minecraft:block`.
const MAPS: [&str; 3] = [
    "neoforge:flammable",
    "neoforge:oxidizables",
    "neoforge:waxables",
];

/// The maps of a world's tables' files; none where no pack gives a value. `rank` is a block's
/// registry id, which picks the block an inverse map gives back.
fn load(
    overlay: &horizon_world::loot::PackOverlay<'_>,
    item_tags: &HashMap<String, HashSet<String>>,
    block_tags: &HashMap<String, HashSet<String>>,
    rank: impl Fn(&str) -> u32,
) -> Option<Arc<BlockMaps>> {
    let types = MAPS.map(|id| DataMapType {
        id: id.to_owned(),
        registry: "minecraft:block".to_owned(),
    });
    let maps = DataMaps::load(&types, overlay, item_tags, block_tags);
    let values = |id| maps.values("minecraft:block", id);
    let block_maps =
        BlockMaps::from_values(values(MAPS[0]), values(MAPS[1]), values(MAPS[2]), rank);
    (!block_maps.is_empty()).then(|| Arc::new(block_maps))
}

impl Game<'_> {
    /// Install the open world's tables' block maps, replacing the last
    /// load's: none for a world without tables.
    pub(in crate::game) fn install_block_maps(&self) {
        let maps = self.world_tables.as_ref().and_then(|tables| {
            let overlay = horizon_world::loot::PackOverlay {
                conditions: &tables.conditions,
                files: &tables.files,
            };
            let table = &self.res.behaviours().table;
            let rank = |block: &str| {
                table
                    .block_by_name(block)
                    .map_or(u32::MAX, |b| u32::from(b.0))
            };
            load(
                &overlay,
                &tables.items.item_tags,
                &tables.items.block_tags,
                rank,
            )
        });
        self.res.behaviours().set_block_maps(maps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pack_files_give_the_block_maps() {
        let file = |value: serde_json::Value| vec![serde_json::to_vec(&value).unwrap()];
        let files = vec![
            (
                "data_maps/neoforge:block/flammable".to_owned(),
                file(json!({"values": {"minecraft:stone": {"ignite_odds": 5, "burn_odds": 20}}})),
            ),
            (
                "data_maps/neoforge:block/waxables".to_owned(),
                file(json!({"values": {"test:tin": {"waxed": "test:waxed_tin"}}})),
            ),
        ];
        let overlay = horizon_world::loot::PackOverlay {
            files: &files,
            ..horizon_world::loot::PackOverlay::NONE
        };
        let block_maps = load(&overlay, &HashMap::new(), &HashMap::new(), |_| 0).unwrap();
        assert_eq!(block_maps.flammable("minecraft:stone"), Some((5, 20)));
        assert_eq!(block_maps.waxed("test:tin"), Some("test:waxed_tin"));
        assert_eq!(block_maps.next_oxidation("test:tin"), None);
        let none = load(
            &horizon_world::loot::PackOverlay::NONE,
            &HashMap::new(),
            &HashMap::new(),
            |_| 0,
        );
        assert!(none.is_none());
    }
}
