#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    /// Records each entity tick notification.
    struct Displays {
        heard: Heard,
    }

    /// A listener's calls, in order: the phase and the entities told.
    type HeardEntries = Vec<(&'static str, u32)>;
    type Heard = Arc<std::sync::Mutex<HeardEntries>>;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Displays {
        fn entity_tick_pre(
            &self,
            _: mod_api::HostSide<'_>,
            event: &mut mod_api::EntityTickPreEvent<'_>,
        ) -> mod_api::Flow {
            self.heard.lock().unwrap().push(("pre", event.entity));
            mod_api::Flow::Continue
        }
        fn entity_tick_post(
            &self,
            _: mod_api::HostSide<'_>,
            event: &mut mod_api::EntityTickPostEvent<'_>,
        ) {
            self.heard.lock().unwrap().push(("post", event.entity));
        }
    }

    use std::sync::Arc;

    use sunrise_core::position::ChunkPos;

    use sunrise_level::chunk::Chunk;

    #[test]
    #[ignore = "daily"]
    fn dispatch_ticks_both_owners_of_each_located_kind() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            for x in [4, 12] {
                assert!(game.run_command(&format!("summon tnt {x} 70 4 {{fuse:80}}")));
                assert!(game.run_command(&format!(
                    "summon falling_block {x} 70 8 {{BlockState:{{Name:\"minecraft:sand\"}},Time:1}}"
                )));
                assert!(game.run_command(&format!("summon experience_orb {x} 70 12 {{Value:1}}")));
            }
            // Existing residents whose spawn packets were already announced.
            game.server.ordered.announced = sunrise_sim::join_seq::JoinSeq::MAX;
            game.server.tickets.0.update(None);
            game.begin_ordered_ticks();
            game.run_ordered_ticks(sunrise_sim::join_seq::JoinSeq::MAX, false)
                .unwrap();
            assert_eq!(
                game.tnts.iter().map(|t| t.fuse).collect::<Vec<_>>(),
                [79, 79]
            );
            assert_eq!(
                game.falling.iter().map(|f| f.age).collect::<Vec<_>>(),
                [2, 2]
            );
            assert_eq!(game.orbs.iter().map(|o| o.age).collect::<Vec<_>>(), [1, 1]);
        });
    }
    /// Two text displays, which the server keeps without simulating them, through a level's
    /// entity phase: their ids and what the listener heard.
    fn display_ticks() -> (Vec<u32>, HeardEntries) {
        let heard = Heard::default();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "display-ticks",
            Box::new(Displays {
                heard: heard.clone(),
            }),
        )]
        .into();
        let mut ids = Vec::new();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.build_registries();
                game.set_allow_commands(true);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(0, 0))
                    .unwrap();
                for x in [4, 12] {
                    assert!(
                        game.run_command(&format!("summon text_display {x} 70 4 {{text:\"x\"}}"))
                    );
                }
                ids = game.server.others.0.iter().map(|o| o.id).collect();
                game.server.tickets.0.update(None);
                game.begin_ordered_ticks();
                game.run_ordered_ticks(sunrise_sim::join_seq::JoinSeq::MAX, false)
                    .unwrap();
            });
        });
        let heard = std::mem::take(&mut *heard.lock().unwrap());
        (ids, heard)
    }
    #[test]
    #[ignore = "daily"]
    fn displays_are_bracketed_one_by_one_in_join_order() {
        let (ids, heard) = display_ticks();
        assert_eq!(ids.len(), 2);
        assert_eq!(
            heard,
            [
                ("pre", ids[0]),
                ("post", ids[0]),
                ("pre", ids[1]),
                ("post", ids[1]),
            ]
        );
    }
}
