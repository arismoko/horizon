use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `Block.entityInside`: an entity's sweep crossed a block with a part for it (`precise`
    /// is `Entity.checkInsideBlocks`' flag). The part gets `&mut dyn ServerHost` once the
    /// entity's batch (mobs, projectiles, items) has ticked, not mid-sweep.
    pub(in crate::game) fn native_entity_inside(
        &mut self,
        state: BlockStateId,
        pos: BlockPos,
        entity: EntityRef,
        precise: bool,
    ) {
        self.fire_entity_inside(state, pos, entity, precise);
    }
}

#[cfg(test)]
mod entity_inside_tests {
    use super::*;
    use horizon_client::block_contact::{in_level, inside};
    use mod_api::{
        BlockPart, EntityInsideEvent, Hooks, ModEntry, Registrar, ServerHost, testing::with_mods,
    };
    use std::sync::{Arc, Mutex};

    /// Landing writes reach the level at the batch end, in the order asked, and only
    /// where the block is still the state landed on.
    #[test]
    #[ignore = "needs prepared resources"]
    fn landing_writes_apply_at_the_batch_end() {
        use horizon_client::block_contact::{landed, take_pending};
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let state = |name| game.pack.assets.registry.default_state(name).unwrap();
            let (farmland, dirt, stone, air) = (
                state("minecraft:farmland"),
                state("minecraft:dirt"),
                state("minecraft:stone"),
                state("minecraft:air"),
            );
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            game.world.set(1, 64, 1, farmland).unwrap();
            game.world.set(2, 64, 1, farmland).unwrap();
            game.world.set(3, 64, 1, stone).unwrap();
            let at = |x| BlockPos::new(x, 64, 1);
            landed(farmland, at(1), vec![LandingWrite::SetBlock(dirt)]);
            // The block changed after the landing: dropped.
            landed(stone, at(2), vec![LandingWrite::SetBlock(dirt)]);
            landed(
                stone,
                at(3),
                vec![LandingWrite::DestroyBlock { drop: false }],
            );
            // Nothing asked: nothing kept.
            landed(farmland, at(1), Vec::new());
            assert_eq!(
                game.world.block(1, 64, 1),
                farmland,
                "nothing writes mid-batch"
            );
            game.world.block_log = Some(Vec::new());
            game.drain_entity_inside();
            assert_eq!(game.world.block(1, 64, 1), dirt);
            assert_eq!(game.world.block(2, 64, 1), farmland);
            assert_eq!(game.world.block(3, 64, 1), air, "the stone is destroyed");
            // Both writes reach the clients (`setBlockAndUpdate` and `destroyBlock` are
            // flag 3); the dropped one sends nothing.
            let log = game.world.block_log.take().unwrap();
            assert!(log.contains(&(1, 64, 1)), "the set block is sent: {log:?}");
            assert!(
                log.contains(&(3, 64, 1)),
                "the destroyed block is sent: {log:?}"
            );
            assert!(
                !log.contains(&(2, 64, 1)),
                "the dropped write sends nothing"
            );
            assert!(take_pending().is_empty());
        });
    }

    /// What the part on stone heard: the entity's id, the position and `precise`.
    #[derive(Clone, Default)]
    struct Stone(Arc<Mutex<Vec<(u32, BlockPos, bool)>>>);
    #[mod_api::horizon_part]
    impl BlockPart for Stone {
        fn entity_inside(&self, _: &mut dyn ServerHost, event: &mut EntityInsideEvent) {
            self.0
                .lock()
                .unwrap()
                .push((event.entity.id(), event.pos, event.precise));
        }
    }
    struct Mod(Stone);
    #[mod_api::__subscriptions]
    impl Hooks for Mod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let block_key = r.block_key("minecraft:stone");
            r.block_part(block_key, self.0.clone());
        }
    }

    /// A contact is delivered once, in order, with its flag; one whose entity is gone
    /// (an item merged later in the batch) or whose block changed is dropped, and a
    /// block with no part is never recorded.
    #[test]
    #[ignore = "needs prepared resources"]
    fn contacts_reach_the_part_unless_stale() {
        let heard = Stone::default();
        let mods: mod_api::Mods =
            vec![ModEntry::new("inside", Box::new(Mod(heard.clone())))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        let _ = horizon_client::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
        with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let state = |name| game.pack.assets.registry.default_state(name).unwrap();
                let (stone, dirt) = (state("minecraft:stone"), state("minecraft:dirt"));
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
                );
                game.world.set(1, 64, 1, stone).unwrap();
                game.world.set(2, 64, 1, stone).unwrap();
                game.world.set(3, 64, 1, dirt).unwrap();
                let level = game.level(game.dim.id()).unwrap();
                let stack = || ItemStack::new("minecraft:stone", 1);
                let kept = game
                    .level_spawn_item(level, 1.5, 70.0, 1.5, stack(), 10)
                    .unwrap();
                let at = |x| BlockPos::new(x, 64, 1);
                in_level(Some(game.dim.id()), || {
                    inside(kept.id() as i32, stone, at(1), true);
                    inside(kept.id() as i32, stone, at(2), false);
                    // The block under a contact changed after it was noted.
                    inside(kept.id() as i32, dirt, at(2), true);
                    // No part on dirt: never recorded.
                    inside(kept.id() as i32, dirt, at(3), true);
                    // No such entity: it merged away.
                    inside(9999, stone, at(1), true);
                });
                game.drain_entity_inside();
                assert_eq!(
                    *heard.0.lock().unwrap(),
                    vec![(kept.id(), at(1), true), (kept.id(), at(2), false)]
                );
                game.drain_entity_inside();
                assert_eq!(
                    heard.0.lock().unwrap().len(),
                    2,
                    "a drain empties the queue"
                );
            });
        });
    }
}
