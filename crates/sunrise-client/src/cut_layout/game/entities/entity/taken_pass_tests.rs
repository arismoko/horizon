//! Production-path storage lifecycle and lifted-pass test support.
use std::sync::Arc;

pub use sunrise_gameplay::game::server_side::entity_server::taken_pass_tests::{
    ScratchDirectory, SlotIdentities, assert_mob_spawn_survives, in_both_dimensions, with_spawn,
};

/// Storage adapters for the same production lifecycle walk across entity kinds.
#[cfg(feature = "client")]
pub(crate) struct LifecycleFixture {
    pub seed: fn(&mut sunrise_gameplay::game::ServerGame<'_>, &[(u32, [f64; 3])]),
    pub ids: fn(&sunrise_gameplay::game::ServerGame<'_>) -> Vec<u32>,
    pub target: fn(u32) -> sunrise_gameplay_common::game::commands_common::Target,
    pub unloads_with_chunk: bool,
    pub remote: Option<RemoteLifecycle>,
}

#[cfg(feature = "client")]
pub(crate) struct RemoteLifecycle {
    pub prepare: fn(&mut sunrise_client_gameplay::game::ClientGame<'_>),
    pub apply: fn(&mut sunrise_client_gameplay::game::ClientGame<'_>, u32, u32),
}

#[cfg(feature = "client")]
impl LifecycleFixture {
    /// Adapters seed owners and read ordered identities; all exchanges and
    /// removals run through production entry points, never a substitute model.
    pub fn run(self) {
        self.run_with(|_| {});
    }

    /// Observe each seeded or transformed storage before checking its order.
    /// Slot-bearing adapters bind new owners and compare remembered registrations here.
    pub fn run_with(self, observe: impl FnMut(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        self.walk(false, observe);
    }

    /// A singleton tests selected and unselected chunks in separate passes.
    /// Remote singleton behavior is checked by its storage-specific fixture.
    pub(crate) fn run_singleton(self) {
        assert!(self.remote.is_none());
        self.walk(true, |_| {});
    }

    fn walk(
        self,
        singleton: bool,
        mut observe: impl FnMut(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let mut ids = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                observe(game);
                use sunrise_sim::join_seq::Joined;
                let mut unique = std::collections::HashSet::new();
                macro_rules! lookup {
                    ($list:expr) => {
                        for (index, entity) in $list.iter().enumerate() {
                            let join = entity.join_seq();
                            assert!(join.is_joined());
                            assert!(join < sunrise_sim::join_seq::JoinSeq::upcoming());
                            assert!(unique.insert(join), "duplicate owner join");
                            assert_eq!($list.locate(join), Some(index));
                        }
                    };
                }
                lookup!(game.server.others.0);
                lookup!(game.fangs.list);
                lookup!(game.ominous_spawners);
                lookup!(game.breath_clouds);
                if let Some(dragon) = game.dragon.as_ref() {
                    assert!(dragon.join.is_joined());
                    assert!(dragon.join < sunrise_sim::join_seq::JoinSeq::upcoming());
                    assert!(unique.insert(dragon.join));
                    assert_eq!(game.dragon.locate(dragon.join), Some(0));
                }
                (self.ids)(game)
            };
            let root = ScratchDirectory(std::env::temp_dir().join(format!(
                "sunrise-lifecycle-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            )));
            game.server.world_root = root.0.clone();
            let active = if singleton || self.unloads_with_chunk {
                vec![103]
            } else {
                vec![101, 103, 104]
            };
            if singleton {
                (self.seed)(game, &[(101, [1.0, 80.0, 1.0])]);
                game.kill_target((self.target)(101));
                assert!(ids(game).is_empty(), "singleton command removal");
                (self.seed)(game, &[(103, [33.0, 80.0, 1.0])]);
                game.entity_chunks(&[(0, 0)].into_iter().collect(), true);
                assert_eq!(ids(game), [103], "unselected singleton chunk");
                (self.seed)(game, &[(101, [1.0, 80.0, 1.0])]);
                game.entity_chunks(&[(0, 0)].into_iter().collect(), true);
                assert_eq!(
                    ids(game),
                    if self.unloads_with_chunk {
                        vec![]
                    } else {
                        vec![101]
                    },
                    "selected singleton chunk"
                );
                (self.seed)(game, &[(103, [33.0, 80.0, 1.0])]);
            } else {
                (self.seed)(
                    game,
                    &[
                        (101, [1.0, 80.0, 1.0]),
                        (102, [17.0, 80.0, 1.0]),
                        (103, [33.0, 80.0, 1.0]),
                        (104, [2.0, 80.0, 1.0]),
                    ],
                );
                assert_eq!(ids(game), [101, 102, 103, 104]);
                game.kill_target((self.target)(102));
                assert_eq!(ids(game), [101, 103, 104], "command removal");
                game.entity_chunks(&[(0, 0)].into_iter().collect(), true);
                assert_eq!(ids(game), active, "selected versus unselected chunk unload");
            }
            game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                .unwrap();
            assert!(
                ids(game).is_empty(),
                "new dimension inherited the active owners"
            );
            let parked = &[(201, [1.0, 80.0, 1.0]), (202, [33.0, 80.0, 1.0])]
                [..if singleton { 1 } else { 2 }];
            let parked_ids = if singleton { vec![201] } else { vec![201, 202] };
            (self.seed)(game, parked);
            assert_eq!(ids(game), parked_ids);
            game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
                .unwrap();
            assert_eq!(ids(game), active, "active owner restore");
            game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                .unwrap();
            assert_eq!(ids(game), parked_ids, "parked owner restore");
            game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
                .unwrap();
            assert_eq!(ids(game), active, "second active restore");
            if let Some(_remote) = self.remote {
                // Client-side remote lifecycle steps run on a `ClientGame` via
                // `run_remote_lifecycle` (see each caller).
            }
        });
    }
}

#[cfg(feature = "client")]
pub(crate) fn run_remote_lifecycle(
    seed: fn(&mut sunrise_client_gameplay::game::ClientGame<'_>, &[(u32, [f64; 3])]),
    ids: fn(&sunrise_client_gameplay::game::ClientGame<'_>) -> Vec<u32>,
    remote: RemoteLifecycle,
) {
    crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
        (remote.prepare)(game);
        seed(
            game,
            &[
                (301, [1.0, 80.0, 1.0]),
                (302, [17.0, 80.0, 1.0]),
                (303, [33.0, 80.0, 1.0]),
            ],
        );
        assert_eq!(ids(game), [301, 302, 303], "remote owner install");
        (remote.apply)(game, 302, 303);
        assert_eq!(ids(game), [301, 303], "remote update and removal");
    });
}

#[test]
fn lifecycle_scratch_is_removed_when_an_assertion_unwinds() {
    let path = std::env::temp_dir().join(format!(
        "sunrise-lifecycle-cleanup-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let result = std::panic::catch_unwind(|| {
        let _scratch = ScratchDirectory(path.clone());
        std::fs::create_dir_all(&path).unwrap();
        panic!("fixture assertion");
    });
    assert!(result.is_err());
    assert!(!path.exists());
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn lifecycle_fixture_walks_fangs_owners() {
    LifecycleFixture {
        seed: |game, entries| {
            game.fangs.list.clear();
            for &(id, at) in entries {
                game.insert_fangs(id, at, 0.0, 0, 0);
            }
        },
        ids: |game| game.fangs.list.iter().map(|f| f.id).collect(),
        target: sunrise_gameplay_common::game::commands_common::Target::Fangs,
        unloads_with_chunk: true,
        remote: None,
    }
    .run();
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn lifecycle_fixture_walks_other_owners() {
    LifecycleFixture {
        seed: |game, entries| {
            game.server.others.0.clear();
            for &(id, at) in entries {
                game.server.others.0.push(
                    sunrise_gameplay_common::game::commands_common::OtherEntity {
                        size_answer: None,
                        attachment_lifetime: Default::default(),
                        knot_check: None,
                        eye: None,
                        join: Default::default(),
                        id,
                        kind: "minecraft:marker".into(),
                        position: sunrise_core::geometry::Vec3::new(at[0], at[1], at[2]),
                        yaw: 0.0,
                        pitch: 0.0,
                        display: None,
                        interaction: None,
                    },
                );
            }
        },
        ids: |game| {
            game.server
                .others
                .0
                .iter()
                .map(|entity| entity.id)
                .collect()
        },
        target: sunrise_gameplay_common::game::commands_common::Target::Other,
        unloads_with_chunk: true,
        remote: None,
    }
    .run();
}

#[test]
#[ignore = "daily"]
fn data_merge_preserves_two_distinct_other_joins_for_every_runtime_kind() {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        for kind in [
            "marker",
            "block_display",
            "item_display",
            "text_display",
            "interaction",
        ] {
            game.server.others.0.clear();
            for (tag, x) in [("first", 1), ("second", 8)] {
                assert!(game.run_command(&format!("summon {kind} {x} 80 1 {{Tags:[\"{tag}\"]}}")));
            }
            let expected: Vec<_> = game
                .server
                .others
                .0
                .iter()
                .map(|e| (e.id, e.join))
                .collect();
            assert_eq!(expected.len(), 2, "{kind}");
            assert_ne!(expected[0].1, expected[1].1);
            assert_ne!(expected[0].1, sunrise_sim::join_seq::JoinSeq::UNJOINED);
            for tag in ["first", "second"] {
                assert!(game.run_command(&format!(
                    "data merge entity @e[tag={tag},limit=1] {{Rotation:[45.0f,10.0f]}}"
                )));
            }
            assert_eq!(
                game.server
                    .others
                    .0
                    .iter()
                    .map(|e| (e.id, e.join))
                    .collect::<Vec<_>>(),
                expected,
                "{kind}"
            );
            for (id, join) in expected {
                assert_eq!(
                    game.server.others.0[game.server.others.0.locate(join).unwrap()].id,
                    id
                );
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn mob_tick_restores_spawn_buffer_before_multiple_survivors_without_renumbering() {
    fn seed(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        ids: std::ops::RangeInclusive<u32>,
        x: f64,
    ) {
        for (index, id) in ids.enumerate() {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut mob = sunrise_sim::mobs::Mob::new(
                id,
                kind,
                sunrise_core::geometry::Vec3::new(x + index as f64 * 2.0, 80.0, 1.5),
                0.0,
            );
            mob.no_ai = true;
            game.mobs.push(mob);
        }
    }
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        seed(game, 71..=73, 1.5);
        let original: Vec<_> = game.mobs.iter().map(|m| (m.id, m.join)).collect();
        // Synthetic additions at the actual lift, then the complete production
        // tick (not a reconstructed restore or a helper replacing that tick).
        with_spawn(
            |g| seed(g, 81..=82, 9.5),
            || game.tick_creature_entities().unwrap(),
        );
        assert_eq!(
            game.mobs.iter().map(|m| m.id).collect::<Vec<_>>(),
            [81, 82, 71, 72, 73]
        );
        assert_eq!(
            game.mobs
                .iter()
                .skip(2)
                .map(|m| (m.id, m.join))
                .collect::<Vec<_>>(),
            original
        );
        assert_eq!(
            game.mobs.iter().map(|m| m.age).collect::<Vec<_>>(),
            [0, 0, 1, 1, 1]
        );
        let all: Vec<_> = game.mobs.iter().map(|m| (m.id, m.join)).collect();
        game.tick_creature_entities().unwrap();
        assert_eq!(
            game.mobs.iter().map(|m| (m.id, m.join)).collect::<Vec<_>>(),
            all
        );
        assert_eq!(
            game.mobs.iter().map(|m| m.age).collect::<Vec<_>>(),
            [1, 1, 2, 2, 2]
        );
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
