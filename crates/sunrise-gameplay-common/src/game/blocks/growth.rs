#[cfg(any())]
#[cfg(feature = "client")]
pub mod tests {

    use mod_api::{self, BonemealEvent, BonemealSource, Flow, Hooks, HostSide};

    use std::sync::{Mutex, MutexGuard, PoisonError};

    use sunrise_core::{geometry::Direction, position::BlockPos};

    use sunrise_items::items::ItemStack;

    /// A dispenser at (0, 64, 1) facing young wheat, with `meal` listening:
    /// what the slot holds after one dispense, and the wheat's age.
    pub fn dispense(meal: Meal, target: &str) -> (Option<ItemStack>, i32) {
        heard().clear();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("meal", Box::new(meal))].into();
        let mut out = None;
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let registry = game.pack.assets.registry;
                let farmland = registry.default_state("minecraft:farmland").unwrap();
                game.world.set(0, 63, 0, farmland).unwrap();
                let block = registry.default_state(target).unwrap();
                game.world.set(CROP.x, CROP.y, CROP.z, block).unwrap();
                let left = game
                    .dispense_item(
                        (0, 64, 1),
                        Direction::North,
                        ItemStack::new("minecraft:bone_meal", 3),
                    )
                    .unwrap()
                    .continue_value()
                    .unwrap();
                let b = game.res.behaviours();
                let state = game.world.block(CROP.x, CROP.y, CROP.z);
                out = Some((left, b.table.int(state, b.props.age)));
            });
        });
        out.unwrap()
    }
}
