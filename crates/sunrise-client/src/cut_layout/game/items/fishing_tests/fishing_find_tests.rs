//! A cast bobber is an entity of its level (#6100): selectors and `entities_in` find it, and
//! `fishing_hook` narrows it to a `FishingHookRef`, as vanilla's `FishingHook` is found.
use std::sync::Arc;
use sunrise_command::{Reader, selector};
use sunrise_core::{geometry::Vec3, random::LegacyRandom};
use sunrise_gameplay::game::{
    native_entity_refs::tests::{bind_local_player, collect_game_entities_in, with_game},
    server_side::{levels::LevelScopeTestExt as _, native_guest_turn_tests::with_guests},
};
use sunrise_gameplay_common::game::{
    Dim, chat_common::ChatCommonGameExt1 as _, commands_common::Target, fishing_common::Hook,
    in_own_process, native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
};
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::PlayerMotion;

/// The caster's bobber at (5.5, 66, 0.5) over air, after one tick.
#[test]
#[ignore = "daily"]
fn a_cast_bobber_is_found_by_selectors_and_entities_in() {
    mod_platform::testing::with_mods(Default::default(), || {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:fishing_rod", 1));
            bind_local_player(game, 20, 7);
            game.set_allow_commands(true);
            game.build_registries();
            let id = game.take_entity_id();
            let mut bobber = Hook::remote(id, Vec3::new(5.5, 66.0, 0.5), [0; 4]);
            bobber.random = LegacyRandom::new(1);
            game.install_fishing_hook(bobber);
            game.tick_fishing();
            let at = game.hook.as_ref().expect("the bobber stays out").position;
            assert!(
                game.level_candidates(false).contains(&Target::Hook(id)),
                "a selector's candidates"
            );
            assert!(game.run_command("execute if entity @e[type=minecraft:fishing_bobber]"));
            let level = game.level(game.dim.id()).unwrap();
            let found = collect_game_entities_in(
                game,
                level,
                [at.x - 1.0, at.y - 1.0, at.z - 1.0],
                [at.x + 1.0, at.y + 1.0, at.z + 1.0],
            )
            .unwrap();
            let hooks: Vec<_> = found
                .into_iter()
                .filter_map(|entity| game.fishing_hook(entity))
                .collect();
            assert_eq!(hooks.len(), 1, "entities_in finds the bobber");
            assert_eq!(hooks[0].entity().id(), id);
        });
    });
}

/// While a guest acts, their cast bobber belongs to their own level even when a selector
/// puts another level in place. A global query across both levels finds it only once.
#[test]
#[ignore = "daily"]
fn a_bobber_is_a_candidate_of_its_casters_level_only() {
    in_own_process(
        "cut_layout::game::server_side::fishing::fishing_find_tests::a_bobber_is_a_candidate_of_its_casters_level_only",
        || {
            mod_platform::testing::with_mods(Default::default(), || {
                with_guests(|game| {
                    assert_eq!(game.dim, Dim::Overworld);
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.build_registries();
                    let hook = game
                        .as_guest(12, |game| {
                            let rod = ItemStack::new("minecraft:fishing_rod", 1);
                            game.common.inventory[game.common.selected] = Some(rod.clone());
                            assert!(game.use_fishing_rod(&rod));
                            game.tick_fishing();
                            Target::Hook(game.hook.as_ref().expect("the bobber stays out").id())
                        })
                        .expect("the guest casts in the Overworld");

                    game.as_guest(12, |game| {
                        let parse = |text| {
                            selector::parse(&mut Reader::new(text), &game.ui.registries, true)
                                .unwrap()
                        };
                        let global = parse("@e[type=minecraft:fishing_bobber]");
                        let bounded = parse("@e[type=minecraft:fishing_bobber,distance=..8]");
                        // Query at the same coordinates while the turn still owns the hook.
                        let wrong_level = game
                            .with_level_in_place(Dim::Nether, |game| game.select(&bounded))
                            .expect("the Nether opens");
                        assert!(
                            wrong_level.is_empty(),
                            "the Nether is not the caster's level"
                        );
                        assert_eq!(game.select(&bounded), [hook], "bounded in the Overworld");
                        assert_eq!(game.select(&global), [hook], "once across both levels");
                    })
                    .expect("the caster takes another guest turn");
                });
            });
        },
    );
}
