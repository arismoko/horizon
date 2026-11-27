//! Common double-chest and container-layout helpers shared by client and server.
use sunrise_gameplay_common::chests::chest_half;
#[allow(unused_imports)]
pub use sunrise_gameplay_common::chests::{chest_connected_pos, is_shulker_box};
use sunrise_level::containers::{BlockEntity, is_chest_block};

impl crate::game::CommonGame<'_> {
    /// The other half of the open double chest and whether the open one is
    /// the right half (`ChestBlock.getConnectedDirection`).
    pub fn chest_partner(&self, key: (i32, i32, i32)) -> Option<((i32, i32, i32), bool)> {
        let registry = self.pack.assets.registry;
        let state = registry.state(self.world.block(key.0, key.1, key.2))?;
        if !is_chest_block(&state.name) {
            return None;
        }
        let own = chest_half(state)?;
        let (dx, dz) = own.connected_step()?;
        let other = (key.0 + dx, key.1, key.2 + dz);
        let other_state = registry.state(self.world.block(other.0, other.1, other.2))?;
        let paired = own.pairs_with(&chest_half(other_state)?)
            && matches!(
                self.world.block_entities.get(&other),
                Some(BlockEntity::Chest(items)) if items.len() == 27
            );
        paired.then_some((other, own.kind == "right"))
    }
    /// The other half of a double chest at `key`, for neighbour updates.
    pub fn chest_other_half(&self, key: (i32, i32, i32)) -> Option<(i32, i32, i32)> {
        self.chest_partner(key).map(|p| p.0)
    }
    /// The block entities of the open container in menu order, with their
    /// sizes: a double chest is its right half then its left half.
    pub fn container_parts(&self) -> Vec<((i32, i32, i32), usize)> {
        let Some(key) = self.container else {
            return Vec::new();
        };
        let Some(entity) = self.world.block_entities.get(&key) else {
            return Vec::new();
        };
        let own = (key, entity.slots().len());
        match self.chest_partner(key) {
            Some((other, true)) => vec![own, (other, 27)],
            Some((other, false)) => vec![(other, 27), own],
            None => vec![own],
        }
    }
    /// The number of slots of the open container.
    pub fn open_container_size(&self) -> usize {
        self.container_parts().iter().map(|p| p.1).sum()
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use sunrise_level::containers::ChestHalf;

    #[test]
    fn double_chest_halves_connect_around_the_facing() {
        let half = |kind, facing| ChestHalf {
            block: "minecraft:chest",
            kind,
            facing,
        };
        // A left half facing north connects east, a right half west.
        assert_eq!(half("left", "north").connected_step(), Some((1, 0)));
        assert_eq!(half("right", "north").connected_step(), Some((-1, 0)));
        assert_eq!(half("left", "south").connected_step(), Some((-1, 0)));
        assert_eq!(half("right", "east").connected_step(), Some((0, -1)));
        assert_eq!(half("single", "east").connected_step(), None);
    }
    #[test]
    fn copper_chests_join_like_chests() {
        let half = |block, kind| ChestHalf {
            block,
            kind,
            facing: "north",
        };
        for block in [
            "minecraft:chest",
            "minecraft:trapped_chest",
            "minecraft:copper_chest",
            "minecraft:waxed_weathered_copper_chest",
        ] {
            assert!(is_chest_block(block));
            assert!(half(block, "left").pairs_with(&half(block, "right")));
        }
        // ChestBlock.combine needs the same block on both halves.
        let copper = half("minecraft:copper_chest", "left");
        assert!(!copper.pairs_with(&half("minecraft:exposed_copper_chest", "right")));
        assert!(!is_chest_block("minecraft:ender_chest"));
    }
    #[test]
    fn shulker_boxes_are_recognised_by_name() {
        assert!(is_shulker_box("minecraft:red_shulker_box"));
        assert!(is_shulker_box("minecraft:shulker_box"));
        assert!(!is_shulker_box("minecraft:chest"));
    }
}
