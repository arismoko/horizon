#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;
    use sunrise_gameplay_common::game::piston_carry::{
        ATTACHMENTS_KEY, Attachments, BlockPos, CARRIED_KEY, ChunkExtras, Direction, MovingPiston,
        Tag,
    };
    use sunrise_items::items::ItemStack;
    use sunrise_level::{chunk::Chunk, containers::BlockEntity};

    #[test]
    #[ignore = "daily"]
    fn a_save_mid_move_keeps_the_carried_block_entity_and_a_bad_key_is_left_out() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            let moving = registry.default_state("minecraft:moving_piston").unwrap();
            let (from, to) = ((1, 64, 1), (2, 64, 1));
            let at = BlockPos::new(to.0, to.1, to.2);
            let mut chunk = Chunk::empty(0, 0);
            chunk.set(to.0, to.1, to.2, moving.0);
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            let Tag::Compound(saved_attachments) = Tag::compound([(
                "carry_test",
                Tag::compound([("value", Tag::compound([("value", Tag::Int(7))]))]),
            )]) else {
                unreachable!()
            };
            game.world.block_entities.insert(
                from,
                BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:diamond", 3))]),
            );
            game.world
                .block_entities
                .restore_attachments(from, Attachments::from_nbt(saved_attachments.clone()));
            assert!(game.world.block_entities.carry(from, to));
            let piston = MovingPiston {
                moved_state: chest,
                direction: Direction::East,
                extending: true,
                source: false,
                progress: 0.5,
                progress_o: 0.5,
                last_ticked: 0,
                death_ticks: 0,
                id: 0,
            };

            // Saved with one key more than vanilla's tag, and only while it carries.
            let saved = game.saved_moving_piston(at, &piston).unwrap();
            let vanilla = game.moving_piston_to_nbt(at, &piston);
            let Tag::Compound(mut without) = saved.clone() else {
                unreachable!()
            };
            assert!(without.remove(&CARRIED_KEY.into()).is_some());
            assert_eq!(Tag::Compound(without), vanilla);

            // Unloaded, then loaded again with its chunk.
            game.world.block_entities.unload(|p| *p == to);
            let mut extras = ChunkExtras::default();
            extras.block_entities.push(saved);
            game.restore_moving_piston(at, piston, Some(&extras));
            assert!(game.world.block_entities.land(to));
            let diamonds = match game.world.block_entities.get(&to) {
                Some(BlockEntity::Chest(items)) => items[0].as_ref().map(|s| s.count),
                _ => None,
            };
            assert_eq!(diamonds, Some(3));
            assert_eq!(
                game.world
                    .block_entities
                    .attachments(to)
                    .map(|a| a.try_to_nbt().unwrap()),
                Some(saved_attachments)
            );
            assert_eq!(
                game.saved_moving_piston(at, &piston).unwrap(),
                vanilla,
                "nothing carried, nothing added"
            );

            // Attachments that can't be read: the block entity lands without them.
            assert!(game.world.block_entities.carry(to, to));
            let mut saved = game.saved_moving_piston(at, &piston).unwrap();
            let Some(Tag::Compound(carried)) = saved.get_mut(CARRIED_KEY) else {
                panic!("the carried block entity is saved")
            };
            carried.insert(ATTACHMENTS_KEY.into(), Tag::Int(1));
            let mut extras = ChunkExtras::default();
            extras.block_entities.push(saved);
            game.world.block_entities.unload(|p| *p == to);
            game.restore_moving_piston(at, piston, Some(&extras));
            assert!(game.world.block_entities.land(to));
            assert!(matches!(
                game.world.block_entities.get(&to),
                Some(BlockEntity::Chest(items)) if items[0].as_ref().is_some_and(|s| s.count == 3)
            ));
            assert!(game.world.block_entities.attachments(to).is_none());

            // A key that can't be read: the piston loads, its block lands bare.
            let Tag::Compound(mut bad) = vanilla.clone() else {
                unreachable!()
            };
            bad.insert(CARRIED_KEY.into(), Tag::Int(3));
            let mut extras = ChunkExtras::default();
            extras.block_entities.push(Tag::Compound(bad));
            game.world.block_entities.remove(&to);
            game.restore_moving_piston(at, piston, Some(&extras));
            assert!(game.world.block_entities.carried(&to).is_none());
            assert!(
                game.block_ticks
                    .moving_pistons
                    .iter()
                    .any(|(p, _)| *p == at)
            );
        });
    }
}
