#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::boats_common::BoatStatusGameExt;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    /// A tick discards vehicles below each dimension's own floor, even when
    /// its chunk storage extends lower, and spills the container's contents.
    /// Vehicles on the line or above it survive this tick.
    #[test]
    #[ignore = "daily"]
    fn a_tick_discards_the_vehicles_under_the_world() {
        use sunrise_core::position::ChunkPos;
        use sunrise_level::chunk::Chunk;
        for min_y in [-64, 0] {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|g| {
                g.world.shape.min_y = min_y;
                g.build_registries();
                g.set_allow_commands(true);
                g.world
                    .chunks
                    .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
                g.block_ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
                assert!(g.run_command("forceload add 0 0"));
                let line = min_y - 64;
                for summon in [
                    format!("summon oak_boat 0.5 {line} 0.5"),
                    format!("summon oak_boat 4.5 {} 4.5", f64::from(line) - 0.5),
                    format!("summon minecart 6.5 {} 6.5", line + 1),
                    format!(
                        "summon chest_minecart 2.5 {} 2.5 {{Items:[{{Slot:0b,id:\"minecraft:diamond\",count:3}}]}}",
                        line - 1
                    ),
                ] {
                    assert!(g.run_command(&summon), "{summon}");
                }
                g.sync_tick_chunks();
                g.tick_boats();
                g.tick_carts();
                let boats: Vec<f64> = g.boats.iter().map(|b| b.position.x).collect();
                assert_eq!(boats, [0.5]);
                let carts: Vec<f64> = g.carts.iter().map(|c| c.position.x).collect();
                assert_eq!(carts, [6.5]);
                let spilled: u32 = g
                    .dropped
                    .iter()
                    .filter(|i| &*i.stack.item == "minecraft:diamond")
                    .map(|i| i.stack.count)
                    .sum();
                assert_eq!(spilled, 3);
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn saved_deep_end_boat_loads_then_is_discarded_and_extreme_scans_terminate() {
        use sunrise_core::geometry::Vec3;
        use sunrise_core::nbt::Tag;
        use sunrise_gameplay_common::game::boats_common::Boat;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::motion::PlayerMotion;
        for min_y in [-64, 0] {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|g| {
                g.world.shape.min_y = min_y;
                g.world.shape.min_y = 0;
                let doubles =
                    |v: &[f64]| Tag::List(v.iter().map(|d| Tag::Double(d.to_bits())).collect());
                let tag = Tag::compound([
                    (
                        "Pos",
                        doubles(&[-77.75688596047263, -2146769274.8915758, -55.6734678353711]),
                    ),
                    ("Motion", doubles(&[-2e-323, -13104.999899947774, -2e-323])),
                ]);
                g.load_boat(tag, None, "minecraft:oak_boat");
                assert_eq!(g.boats.len(), 1);
                assert_eq!(g.boats[0].position.y, -20_000_000.0);
                assert_eq!(g.boats[0].velocity, Vec3::new(-2e-323, 0.0, -2e-323));
                g.player =
                    Some(PlayerMotion::new(g.motion, Vec3::new(-77.75, 64.0, -55.67)).unwrap());
                g.world
                    .chunks
                    .insert((-5, -4), std::sync::Arc::new(Chunk::empty(-5, -4)));
                g.update_chunk_tickets();
                assert!(g.entity_ticking_area()(-78, -56));
                g.tick_boats();
                assert!(g.boats.is_empty());
                for y in [f64::from(i32::MIN) - 1.0, f64::from(i32::MAX) + 1.0] {
                    g.boats
                        .push(Boat::new(2, "oak".into(), Vec3::new(0.5, y, 0.5), 0.0));
                    assert!(g.boat_bubble_columns(0).is_empty());
                    assert_eq!(
                        g.boat_ground_friction(&g.boats[0], g.boats[0].bounds()),
                        0.0
                    );
                    g.boats.clear();
                }
            });
        }
    }
}
