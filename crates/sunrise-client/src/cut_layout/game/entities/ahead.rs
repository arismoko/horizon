#![cfg(test)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use sunrise_core::{geometry::Vec3, random::RandomSource};
    use sunrise_gameplay::game::game_tests::tests::server_game;
    use sunrise_gameplay::game::server_side::ahead::{AHEAD, TAKEN};
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::collision_source::CollisionSourceGameExt1;
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;

    /// One item tick with the moves worked out ahead leaves every item, and the level's random,
    /// as the tick that moves each item at its turn: items escaping a block (their own random),
    /// floating in water, falling, and merging into a later item before its turn (whose move
    /// is then made at its turn).
    #[test]
    #[ignore = "daily"]
    fn moves_worked_out_ahead_match_those_made_at_each_turn() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let feet = sunrise_core::geometry::Vec3::new(8.5, 70.0, 8.5);
        game.player = Some(sunrise_sim::motion::PlayerMotion::new(game.motion, feet).unwrap());
        // `ChunkMap.prepareEntityTickingChunk`: the chunks within 2 are loaded too.
        for x in -2..=2 {
            for z in -2..=2 {
                game.world
                    .chunks
                    .insert((x, z), Arc::new(Chunk::empty(x, z)));
            }
        }
        game.update_chunk_tickets();
        assert!(game.entity_ticking_area()(8, 8));
        game.border.set_center([8.0, 8.0]).unwrap();
        game.border.set_size(16.0).unwrap();
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let water = registry.default_state("minecraft:water").unwrap();
        let mut escaping = Vec::new();
        for x in 0..16 {
            for z in 0..16 {
                let (fx, fz) = (f64::from(x) + 0.5, f64::from(z) + 0.5);
                let stack = |count| ItemStack::new("minecraft:stone", count);
                match (x + z) % 5 {
                    0 => {
                        game.world.set(x, 65, z, stone).unwrap();
                        game.spawn_item(stack(1), [fx, 65.3, fz], [0.0; 3], 10);
                        escaping.push(game.dropped.last().unwrap().id);
                    }
                    1 => {
                        game.world.set(x, 65, z, water).unwrap();
                        game.spawn_item(stack(1), [fx, 65.5, fz], [0.0; 3], 10);
                    }
                    2 => {
                        game.spawn_item(stack(1), [fx, 66.0, fz], [0.0; 3], 0);
                        game.spawn_item(stack(5), [fx, 66.0, fz], [0.0; 3], 0);
                    }
                    _ => game.spawn_item(stack(1), [fx, 67.0, fz], [0.1, 0.2, -0.1], 10),
                }
            }
        }
        // Real solid-entity boxes, not a stubbed obstacle query. One item is
        // caught inside the boat; another falls onto its top this tick.
        game.server.next_entity += 1;
        let boat = sunrise_gameplay_common::game::boats_common::Boat::new(
            game.server.next_entity,
            Arc::from("oak"),
            Vec3::new(8.5, 80.0, 8.5),
            0.0,
        );
        let boat_top = boat.bounds().max_position().y;
        game.boats.push(boat);
        game.spawn_item(
            ItemStack::new("minecraft:iron_ingot", 1),
            [8.5, 80.2, 8.5],
            [0.0; 3],
            10,
        );
        let caught = game.dropped.last().unwrap().id;
        escaping.push(caught);
        game.spawn_item(
            ItemStack::new("minecraft:gold_ingot", 1),
            [8.5, 81.0, 8.5],
            [0.0, -1.0, 0.0],
            10,
        );
        let landing = game.dropped.last().unwrap().id;
        // The border cuts the box even though the centre is still inside it.
        game.spawn_item(
            ItemStack::new("minecraft:diamond", 1),
            [15.99, 80.0, 8.5],
            [0.0; 3],
            10,
        );
        let border = game.dropped.last().unwrap().id;
        escaping.push(border);
        let boxes =
            game.entity_collisions(sunrise_gameplay_common::game::collision_source::Mover::Passive);
        for id in [caught, border] {
            let item = game.dropped.iter().find(|i| i.id == id).unwrap();
            assert!(item.inside_block(
                &game.world,
                game.collision,
                &boxes,
                Some(game.border.geometry())
            ));
            assert!(!item.inside_block(&game.world, game.collision, &[], None));
        }
        for item in game.dropped.iter_mut() {
            item.ticks = 39;
        }
        game.server.ticked_entities_through = u32::MAX;
        let (items, random) = (game.dropped.clone(), game.world.random.clone());
        // Each merge-written field independently invalidates a prepared move.
        // Rejecting it must not overwrite the merge or advance the item's RNG.
        AHEAD.with(|a| a.set(Some(true)));
        let obstacles = sunrise_sim::entities::ObstacleGrid::new(boxes);
        for changed in 0..3 {
            let mut moves = sunrise_gameplay::game::server_side::ahead::Moves::work_out(
                &game,
                &items,
                &obstacles,
                &sunrise_gameplay_common::game::native_ticks::TickRound::default(),
                |_| true,
            );
            let mut item = items[0].clone();
            match changed {
                0 => item.stack.count += 1,
                1 => item.age += 1,
                _ => item.pickup_delay += 1,
            }
            let before = format!("{item:?}");
            assert!(
                moves
                    .take(0, &mut item, game.world.revision, |_| {
                        panic!("invalidated move was verified")
                    })
                    .is_none()
            );
            assert_eq!(format!("{item:?}"), before);
        }
        let mut runs = Vec::new();
        // First use the real environment switch, then serial/ahead/serial
        // overrides. This also checks switch-off behavior above the threshold.
        for ahead in [None, Some(false), Some(true), Some(false)] {
            game.dropped = items.clone();
            game.world.random = random.clone();
            AHEAD.with(|a| a.set(ahead));
            TAKEN.with(|t| t.set((0, 0)));
            game.tick_items();
            // Independently advance just the escaping items' starting randoms.
            // The helper copy must consume exactly one float and return that
            // state at the turn; ordinary moves must consume none of it.
            for item in game.dropped.iter() {
                let before = items.iter().find(|i| i.id == item.id).unwrap();
                let mut expected = before.random.clone();
                if escaping.contains(&item.id) {
                    expected.next_float();
                }
                assert_eq!(
                    item.random.clone().next_int(),
                    expected.next_int(),
                    "item {}",
                    item.id
                );
            }
            let landed = game.dropped.iter().find(|i| i.id == landing).unwrap();
            assert_eq!(landed.position.y, boat_top);
            assert!(landed.ground.on_ground);
            assert_eq!(format!("{:?}", game.world.random), format!("{random:?}"));
            let left: Vec<String> = game.dropped.iter().map(|i| format!("{i:?}")).collect();
            runs.push((
                left,
                format!("{:?}", game.world.random),
                TAKEN.with(|t| t.get()),
            ));
        }
        AHEAD.with(|a| a.set(None));
        assert!(runs[0].0.len() < items.len(), "some items merged");
        assert_eq!(runs[0].0, runs[1].0);
        assert_eq!(runs[0].1, runs[1].1);
        assert_eq!(runs[0].0, runs[2].0);
        assert_eq!(runs[0].1, runs[2].1);
        assert_eq!(runs[0].0, runs[3].0);
        assert_eq!(runs[0].1, runs[3].1);
        assert_eq!(runs[1].2, (0, 0));
        assert_eq!(runs[3].2, (0, 0));
        if sunrise_sim::parallel::lanes() > 1 {
            assert_eq!(runs[0].2, runs[2].2);
        } else {
            assert_eq!(runs[0].2, (0, 0));
        }
        // The path under test ran: most moves were put in place, and a merge refused some.
        let (taken, refused) = runs[2].2;
        assert!(
            taken >= 200 && refused > 0,
            "{taken} put in place, {refused} refused"
        );
        eprintln!(
            "item work-ahead proof: {} inputs, {} escaping; {taken} taken, {refused} invalidated",
            items.len(),
            escaping.len()
        );
    }
}
