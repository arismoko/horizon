#![cfg(test)]
#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::super::tests::{
        AIR, STONE, bound_slot, client, column, data, open, run, run_at, spawn, swap, viewed,
    };

    use sunrise_core::nbt::Tag;
    use sunrise_protocol::{
        chunk::{BlockEntityInfo, ChunkData, LightData, Paletted, Section, Strategy},
        packets::play::{self, Clientbound as C},
    };

    const OVERWORLD: &str = "minecraft:overworld";
    const NETHER: &str = "minecraft:the_nether";

    fn respawn(dimension: &str, keep: i8) -> C {
        C::Respawn(play::Respawn {
            spawn: spawn(dimension),
            keep,
        })
    }

    fn block(x: i32, y: i32, z: i32, state: sunrise_world::registry::BlockStateId) -> C {
        C::BlockUpdate(play::BlockUpdate {
            pos: [x, y, z],
            state: state.0 as i32,
        })
    }

    /// The player's columns, sorted.
    fn own(game: &sunrise_client_gameplay::game::ClientGame<'_>) -> Vec<(i32, i32)> {
        let mut keys: Vec<_> = game.world.chunks.keys().copied().collect();
        keys.sort_unstable();
        keys
    }

    fn loading(game: &sunrise_client_gameplay::game::ClientGame<'_>) -> bool {
        game.client.remote.as_ref().unwrap().level_load.is_active()
    }

    /// Travels both ways between two runtime levels of one dimension type that the Login did not
    /// name: each `swap` binds the level left to the slot of the level entered with a new epoch,
    /// both worlds keep their columns, the next `run` of the slot reaches the level left, and a
    /// late `run` of the level entered, at its old epoch, is dropped.
    #[test]
    #[ignore = "daily"]
    fn travels_between_two_runtime_levels_of_one_type_keep_both_worlds() {
        const A: &str = "test:instance/a";
        const B: &str = "test:instance/b";
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(respawn(A, 3)).unwrap();
        game.handle_packet(column(0, 0)).unwrap();
        game.handle_packet(open(1, B)).unwrap();
        game.handle_packet(run(1, &[column(5, 5)])).unwrap();
        let b_open = bound_slot(1);
        game.handle_packet(respawn(B, 3)).unwrap();
        game.handle_packet(swap(1)).unwrap();
        assert_eq!(own(&game), vec![(5, 5)], "B's columns, now the player's");
        assert_eq!(
            game.client
                .remote
                .as_ref()
                .unwrap()
                .viewed
                .get(1)
                .unwrap()
                .level,
            A
        );
        // The slot's next run reaches A; one of B's late, at B's epoch, is dropped.
        game.handle_packet(run(1, &[column(1, 0)])).unwrap();
        game.handle_packet(run_at(b_open, &[column(9, 9)])).unwrap();
        let a = viewed(&game, 1).unwrap();
        assert!(a.chunks.contains_key(&(0, 0)) && a.chunks.contains_key(&(1, 0)));
        assert!(!a.chunks.contains_key(&(9, 9)));
        // And back into A: B is kept in the slot, with its columns.
        game.handle_packet(respawn(A, 3)).unwrap();
        game.handle_packet(swap(1)).unwrap();
        assert_eq!(own(&game), vec![(0, 0), (1, 0)]);
        assert_eq!(
            game.client
                .remote
                .as_ref()
                .unwrap()
                .viewed
                .get(1)
                .unwrap()
                .level,
            B
        );
        game.handle_packet(run(1, &[column(6, 6)])).unwrap();
        let b = viewed(&game, 1).unwrap();
        assert!(b.chunks.contains_key(&(5, 5)) && b.chunks.contains_key(&(6, 6)));
    }

    /// A travel the client does not take as a swap, here into a viewed level whose dimension type
    /// changed since its `open`: it enters afresh, and the server's `swap` is refused with a fixed
    /// reason, so the two sides never disagree in silence.
    #[test]
    #[ignore = "daily"]
    fn a_swap_the_client_did_not_make_is_refused() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(1, NETHER)).unwrap();
        game.handle_packet(run(1, &[column(5, 5)])).unwrap();
        // The Respawn names the Nether's own type (1), of another shape than its `open`'s (0).
        let respawn = C::Respawn(play::Respawn {
            spawn: play::SpawnInfo {
                dimension_type: 1,
                ..spawn(NETHER)
            },
            keep: 3,
        });
        game.handle_packet(respawn).unwrap();
        assert!(loading(&game), "entered afresh");
        let refused = game.route_viewed_levels(data(&swap(1))).unwrap_err();
        assert_eq!(
            refused.to_string(),
            "sunrise:levels: a swap the client did not make"
        );
    }

    /// A level the server opened after Login (a runtime level) is in the client's table of levels
    /// while the player is in it, and dropped once it leaves; a level the Login named stays. The
    /// same id entered again (a level deleted and opened again) starts with no column of the old
    /// one.
    #[test]
    #[ignore = "daily"]
    fn a_level_opened_after_login_is_forgotten_once_left() {
        use sunrise_gameplay_common::game::Dim;
        const RUNTIME: &str = "test:instance/1";
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(respawn(RUNTIME, 3)).unwrap();
        assert!(Dim::exists(RUNTIME));
        game.handle_packet(column(0, 0)).unwrap();
        assert_eq!(own(&game), vec![(0, 0)]);
        game.handle_packet(respawn("test:void", 3)).unwrap();
        assert!(!Dim::exists(RUNTIME), "left, and not named at Login");
        assert!(Dim::exists("test:void"));
        game.handle_packet(respawn(OVERWORLD, 3)).unwrap();
        assert!(Dim::exists("test:void"), "named at Login");
        game.handle_packet(respawn(RUNTIME, 3)).unwrap();
        assert!(Dim::exists(RUNTIME));
        assert!(own(&game).is_empty(), "nothing of the level before");
    }

    /// A column of stone at `x, z` with a chest at (3, 70, 5) in it, which its packet lists.
    fn column_with_a_chest(
        game: &sunrise_client_gameplay::game::ClientGame<'_>,
        x: i32,
        z: i32,
    ) -> C {
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        let section = |states: &[i32]| Section {
            non_empty: 4096,
            fluids: 0,
            states: Paletted::from_values(states, Strategy::Blocks, 16),
            biomes: Paletted::from_values(&[1; 64], Strategy::Biomes, 2),
        };
        let mut sections = vec![section(&[STONE.0 as i32; 4096]); 24];
        // Section 8 holds y 64 to 79; (3, 6, 5) in it is index 6 * 256 + 5 * 16 + 3.
        let mut states = [STONE.0 as i32; 4096];
        states[6 * 256 + 5 * 16 + 3] = chest.0 as i32;
        sections[8] = section(&states);
        C::LevelChunkWithLight(play::LevelChunkWithLight {
            x,
            z,
            chunk: ChunkData {
                heightmaps: Vec::new(),
                buffer: ChunkData::from_sections(&sections),
                block_entities: vec![BlockEntityInfo {
                    packed_xz: (3 << 4) | 5,
                    y: 70,
                    kind: 1,
                    tag: Tag::End,
                }],
            },
            light: LightData {
                sky_mask: Vec::new(),
                block_mask: Vec::new(),
                empty_sky_mask: Vec::new(),
                empty_block_mask: Vec::new(),
                sky: Vec::new(),
                block: Vec::new(),
            },
        })
    }

    /// A travel into the viewed Nether swaps the worlds: the Nether's columns, with the block
    /// changed while it was viewed, are the player's and queued to light and mesh; the
    /// Overworld's are its viewed world, which wrapped packets of level 0 now change; no loading
    /// screen starts.
    #[test]
    #[ignore = "daily"]
    fn a_travel_into_a_viewed_level_swaps_the_worlds_with_no_loading_screen() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(column(0, 0)).unwrap();
        game.handle_packet(column(1, 0)).unwrap();
        game.handle_packet(open(1, NETHER)).unwrap();
        game.handle_packet(run(1, &[column(5, 5), column(6, 5)]))
            .unwrap();
        game.handle_packet(run(1, &[block(83, 70, 85, AIR)]))
            .unwrap();
        game.handle_packet(respawn(NETHER, 3)).unwrap();
        game.handle_packet(swap(1)).unwrap();
        let r = game.client.remote.as_ref().unwrap();
        assert_eq!(r.level, NETHER);
        assert!(!loading(&game), "no loading screen");
        assert_eq!(own(&game), vec![(5, 5), (6, 5)]);
        assert!(game.client.streaming.is_lit(5, 5) && game.client.streaming.is_lit(6, 5));
        assert_eq!(game.world.block(83, 70, 85), AIR);
        assert_eq!(game.world.block(83, 71, 85), STONE);
        // The `swap` bound the Overworld to the Nether's slot, 1.
        assert_eq!(r.viewed.len(), 1);
        let overworld = r.viewed.get(1).expect("the Overworld, viewed");
        assert_eq!(overworld.level, OVERWORLD);
        let mut kept: Vec<_> = overworld.world.chunks.keys().copied().collect();
        kept.sort_unstable();
        assert_eq!(kept, vec![(0, 0), (1, 0)]);
        // The slot's packets go to its viewed world, plain ones to the Nether.
        game.handle_packet(run(1, &[block(3, 70, 5, AIR)])).unwrap();
        game.handle_packet(block(84, 70, 85, AIR)).unwrap();
        assert_eq!(viewed(&game, 1).unwrap().block(3, 70, 5), AIR);
        assert_eq!(game.world.block(84, 70, 85), AIR);
        assert_eq!(viewed(&game, 1).unwrap().block(3, 71, 5), STONE);
    }

    /// Block entities go with their level both ways: a Nether chest its chunk packet listed is the
    /// player's once it travels there, and the Overworld chest the player's world had is kept
    /// in its viewed world and is the player's again on the way back.
    #[test]
    #[ignore = "daily"]
    fn block_entities_travel_with_their_level_both_ways() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let overworld_chest = column_with_a_chest(&game, 0, 0);
        let nether_chest = column_with_a_chest(&game, 5, 5);
        game.handle_packet(overworld_chest).unwrap();
        assert!(game.world.block_entities.contains_key(&(3, 70, 5)));
        game.handle_packet(open(1, NETHER)).unwrap();
        game.handle_packet(run(1, &[nether_chest])).unwrap();
        game.handle_packet(respawn(NETHER, 3)).unwrap();
        game.handle_packet(swap(1)).unwrap();
        assert!(game.world.block_entities.contains_key(&(83, 70, 85)));
        assert!(!game.world.block_entities.contains_key(&(3, 70, 5)));
        game.handle_packet(respawn(OVERWORLD, 3)).unwrap();
        game.handle_packet(swap(1)).unwrap();
        assert_eq!(own(&game), vec![(0, 0)]);
        assert!(game.world.block_entities.contains_key(&(3, 70, 5)));
        assert!(!game.world.block_entities.contains_key(&(83, 70, 85)));
        let nether = game.client.remote.as_ref().unwrap().viewed.get(1).unwrap();
        assert!(nether.world.block_entities.contains_key(&(83, 70, 85)));
    }

    /// The client's server names the chest's block entity type 0: the chest's state.
    fn with_chests(
        game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    ) -> sunrise_world::registry::BlockStateId {
        std::sync::Arc::make_mut(&mut game.client.remote.as_mut().unwrap().builtin)
            .registries
            .insert(
                "minecraft:block_entity_type".into(),
                sunrise_protocol::registry::Ids::from_names(vec!["minecraft:chest".into()]),
            );
        (game.pack.assets.registry)
            .default_state("minecraft:chest")
            .unwrap()
    }

    /// A chest's data at `pos`: 7 diamonds in slot 3.
    fn diamonds(pos: [i32; 3]) -> C {
        let items =
            sunrise_core::nbt::parse_snbt(r#"{Items:[{Slot:3b,id:"minecraft:diamond",count:7}]}"#);
        C::BlockEntityData(play::BlockEntityData {
            pos,
            kind: 0,
            tag: items.unwrap().0,
        })
    }

    /// The diamonds of the chest at `pos` in the player's world, if it has them.
    fn diamonds_at(
        game: &sunrise_client_gameplay::game::ClientGame<'_>,
        pos: (i32, i32, i32),
    ) -> Option<u32> {
        let chest = game.world.block_entities.get(&pos).expect("a chest");
        let item = chest.slots()[3].as_ref()?;
        assert_eq!(item.item.as_ref(), "minecraft:diamond");
        Some(item.count)
    }

    /// A chest placed in a viewed column after its chunk packet keeps the data sent for it: an
    /// empty position, then the chest's block update and data, then the travel, and the chest in
    /// the new world holds its diamonds. Data for a position whose block has no block entity is
    /// not kept meanwhile.
    #[test]
    #[ignore = "daily"]
    fn a_chest_placed_in_a_viewed_column_keeps_its_data_through_the_travel() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let chest = with_chests(&mut game);
        game.handle_packet(open(1, NETHER)).unwrap();
        game.handle_packet(run(1, &[column(5, 5)])).unwrap();
        game.handle_packet(run(1, &[block(83, 70, 85, AIR)]))
            .unwrap();
        game.handle_packet(run(
            1,
            &[
                block(83, 70, 85, chest),
                diamonds([83, 70, 85]),
                diamonds([84, 70, 85]),
            ],
        ))
        .unwrap();
        let nether = game.client.remote.as_ref().unwrap().viewed.get(1).unwrap();
        let kept: Vec<_> = nether.block_entities[&(5, 5)]
            .iter()
            .map(|info| (info.packed_xz, info.y))
            .collect();
        assert_eq!(
            kept,
            vec![((3 << 4) | 5, 70)],
            "the chest's, not the stone's"
        );
        game.handle_packet(respawn(NETHER, 3)).unwrap();
        game.handle_packet(swap(1)).unwrap();
        assert_eq!(diamonds_at(&game, (83, 70, 85)), Some(7));
    }

    /// Another block replacing a viewed block entity takes its data with it: a chest with data,
    /// then air, then a chest again with none, is an empty chest after the travel. A chest's own
    /// state change (another facing) keeps the data.
    #[test]
    #[ignore = "daily"]
    fn a_block_replacing_a_viewed_block_entity_takes_its_data() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let chest = with_chests(&mut game);
        let turned = sunrise_world::registry::BlockStateId(chest.0 + 1);
        assert_eq!(&*game.res.facts().get(turned).name, "minecraft:chest");
        game.handle_packet(open(1, NETHER)).unwrap();
        game.handle_packet(run(1, &[column(5, 5)])).unwrap();
        game.handle_packet(run(
            1,
            &[
                block(83, 70, 85, chest),
                diamonds([83, 70, 85]),
                block(83, 70, 85, AIR),
                block(83, 70, 85, chest),
                block(86, 70, 85, chest),
                diamonds([86, 70, 85]),
                block(86, 70, 85, turned),
            ],
        ))
        .unwrap();
        game.handle_packet(respawn(NETHER, 3)).unwrap();
        game.handle_packet(swap(1)).unwrap();
        assert_eq!(
            diamonds_at(&game, (83, 70, 85)),
            None,
            "the new chest is empty"
        );
        assert_eq!(
            diamonds_at(&game, (86, 70, 85)),
            Some(7),
            "the turned chest keeps them"
        );
    }

    /// Removing both placed block entities clears both retained stores before travel.
    #[test]
    #[ignore = "daily"]
    fn removed_block_entities_at_two_positions_do_not_return_on_travel() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let chest = with_chests(&mut game);
        game.handle_packet(open(1, NETHER)).unwrap();
        game.handle_packet(run(1, &[column(5, 5)])).unwrap();
        for [x, y, z] in [[83, 70, 85], [86, 70, 85]] {
            game.handle_packet(run(1, &[block(x, y, z, chest), diamonds([x, y, z])]))
                .unwrap();
        }
        // Instantiate live block entities, park their world, then retain fresh packet data too.
        game.handle_packet(respawn(NETHER, 3)).unwrap();
        game.handle_packet(swap(1)).unwrap();
        game.handle_packet(respawn(OVERWORLD, 3)).unwrap();
        game.handle_packet(swap(1)).unwrap();
        game.handle_packet(run(1, &[diamonds([83, 70, 85]), diamonds([86, 70, 85])]))
            .unwrap();
        {
            let nether = game.client.remote.as_ref().unwrap().viewed.get(1).unwrap();
            assert_eq!(nether.block_entities[&(5, 5)].len(), 2);
            assert_eq!(nether.world.block_entities.len(), 2);
        }
        game.handle_packet(run(1, &[block(83, 70, 85, AIR)]))
            .unwrap();
        {
            let nether = game.client.remote.as_ref().unwrap().viewed.get(1).unwrap();
            assert_eq!(nether.block_entities[&(5, 5)].len(), 1);
            assert!(nether.world.block_entities.contains_key(&(86, 70, 85)));
            assert!(!nether.world.block_entities.contains_key(&(83, 70, 85)));
        }
        game.handle_packet(run(1, &[block(86, 70, 85, AIR)]))
            .unwrap();
        {
            let nether = game.client.remote.as_ref().unwrap().viewed.get(1).unwrap();
            assert!(nether.block_entities.values().all(Vec::is_empty));
            assert!(nether.world.block_entities.is_empty());
        }
        game.handle_packet(respawn(NETHER, 3)).unwrap();
        game.handle_packet(swap(1)).unwrap();
        for pos in [(83, 70, 85), (86, 70, 85)] {
            assert!(!game.world.block_entities.contains_key(&pos));
            assert_eq!(game.world.block(pos.0, pos.1, pos.2), AIR);
        }
    }

    /// A travel into a level the client does not view enters it afresh: every viewed world goes,
    /// with the player's, and the loading screen waits for the server.
    #[test]
    #[ignore = "daily"]
    fn a_travel_into_a_level_not_viewed_reloads() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(column(0, 0)).unwrap();
        game.handle_packet(open(1, NETHER)).unwrap();
        game.handle_packet(run(1, &[column(5, 5)])).unwrap();
        game.handle_packet(respawn("minecraft:the_end", 3)).unwrap();
        assert_eq!(game.client.remote.as_ref().unwrap().viewed.len(), 0);
        assert!(own(&game).is_empty());
        assert!(loading(&game));
    }

    /// A death's Respawn (keep 0) and an End exit's (`PlayerList.respawn`, keep 1) into the
    /// viewed Nether reload it: every viewed world goes, the Nether's columns too, and the
    /// loading screen waits for the server.
    #[test]
    #[ignore = "daily"]
    fn a_death_or_an_end_exit_into_a_viewed_level_reloads() {
        let mut resources = None;
        for keep in [0, 1] {
            let mut game = client(&mut resources);
            game.handle_packet(column(0, 0)).unwrap();
            game.handle_packet(open(1, NETHER)).unwrap();
            game.handle_packet(run(1, &[column(5, 5)])).unwrap();
            game.handle_packet(respawn(NETHER, keep)).unwrap();
            let r = game.client.remote.as_ref().unwrap();
            assert_eq!(r.level, NETHER, "keep {keep}");
            assert_eq!(r.viewed.len(), 0, "keep {keep}");
            assert!(own(&game).is_empty(), "keep {keep}");
            assert!(loading(&game), "keep {keep}");
        }
    }
}
