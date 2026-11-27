#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {
    #[cfg(feature = "client")]
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    #[cfg(feature = "client")]
    use mod_api::BlockSoundCause::{Break, Dig, Fall, Place, Step};
    #[cfg(feature = "client")]
    use std::sync::Arc;
    #[cfg(feature = "client")]
    use sunrise_core::{geometry::Vec3, position::BlockPos};
    #[cfg(feature = "client")]
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
    #[cfg(feature = "client")]
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::GameGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_block_sounds::NativeBlockSoundsGameExt1 as _;
    #[cfg(feature = "client")]
    use sunrise_sim::mobs::{Mob, MobEvent, equine};
    #[cfg(feature = "client")]
    use sunrise_world::level::LevelAccess;

    // The executable installs its client callback table once per process, so the fixture runs
    // in a fresh child: this returns true there, and false in the parent once the child passed.
    #[cfg(feature = "client")]
    pub(crate) fn install_sound_fixture_in_child(test: &str) -> bool {
        const CHILD: &str = "SUNRISE_BLOCK_SOUND_TEST_CHILD";
        if run_fresh_cases(test, CHILD, &["child"]) {
            return false;
        }
        crate::entry::install_client_hooks();
        true
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_block_part_answers_the_sound_type_a_block_plays() {
        if !install_sound_fixture_in_child(
            "cut_layout::game::native_block_sounds::tests::a_block_part_answers_the_sound_type_a_block_plays",
        ) {
            return;
        }
        mod_platform::__private::part_tests::sound_type_game_thread();
        with_client_game(|client| {
            with_game(|game| {
                let registry = game.pack.assets.registry;
                let player = Some((7, "minecraft:player"));
                let creeper = Some((8, "minecraft:creeper"));
                let sniffer = Some((9, "minecraft:sniffer"));
                let sound =
                    |game: &sunrise_gameplay::game::ServerGame<'_>, block: &str, entity, cause| {
                        let state = registry.default_state(block).unwrap();
                        let sound = game.selected_block_sound([5, 64, 5], state, entity, cause);
                        (sound.event.as_str().to_owned(), sound.volume, sound.pitch)
                    };
                let stone = |event: &str| (format!("minecraft:block.stone.{event}"), 1.0, 1.0);
                let wool = |event: &str| (format!("minecraft:block.wool.{event}"), 1.0, 1.0);
                let answered = |event: &str| (format!("mymod:block.x.{event}"), 2.0, 1.5);
                // No part: the state's own.
                assert_eq!(sound(game, "minecraft:stone", creeper, Step), stone("step"));
                // A part answering every cause and entity, with the mod's own events.
                assert_eq!(
                    sound(game, "minecraft:glass", creeper, Step),
                    answered("step")
                );
                assert_eq!(
                    sound(game, "minecraft:glass", creeper, Fall),
                    answered("fall")
                );
                assert_eq!(
                    sound(game, "minecraft:glass", player, Place),
                    answered("place")
                );
                assert_eq!(
                    sound(game, "minecraft:glass", None, Break),
                    answered("break")
                );
                assert_eq!(
                    sound(game, "minecraft:glass", sniffer, Dig),
                    answered("hit")
                );
                // A part answering for a player only.
                assert_eq!(
                    sound(game, "minecraft:white_wool", player, Step),
                    answered("step")
                );
                assert_eq!(
                    sound(game, "minecraft:white_wool", creeper, Step),
                    wool("step")
                );
                assert_eq!(
                    sound(game, "minecraft:white_wool", None, Break),
                    wool("break")
                );
                // The wrapper does not replace levels the mixer will interpret, and an unused
                // event missing from the packs never hides the selected event.
                assert_eq!(
                    sound(game, "minecraft:red_wool", creeper, Step),
                    ("mymod:block.x.step".to_owned(), -3.0, 0.25)
                );
                assert_eq!(
                    sound(game, "minecraft:blue_wool", creeper, Step),
                    ("minecraft:block.amethyst_block.step".to_owned(), 2.0, 1.5)
                );
                assert_eq!(
                    sound(game, "minecraft:blue_wool", None, Break),
                    ("mymod:missing".to_owned(), 2.0, 1.5)
                );
                // The server's placement (`BlockItem.place`) asks with the acting player, and with
                // none when nothing places it (a dispenser).
                let glass = registry.default_state("minecraft:glass").unwrap();
                let client_glass = client
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:glass")
                    .unwrap();
                client.client.audio.0.capture = Some(Vec::new());
                client.play_break_sound([5, 64, 5], client_glass);
                let captured = client.client.audio.0.capture.take().unwrap();
                let [broken] = captured.as_slice() else {
                    panic!("expected one break sound");
                };
                assert_eq!(broken.event, "mymod:block.x.break");
                assert_eq!(broken.source, mod_api::SoundSource::Blocks.index());
                assert_eq!((broken.volume, broken.pitch), (1.5, 1.2));

                let white_wool = registry.default_state("minecraft:white_wool").unwrap();
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:creeper")
                    .unwrap()
                    .clone();
                let creeper_mob = Mob::new(8, kind, Vec3::ZERO, 0.0);
                let step = sunrise_sim::mobs::entity_sounds::step(
                    &creeper_mob,
                    &game.mob_world_base(&[]),
                    [5, 64, 5],
                    glass,
                )
                .unwrap();
                assert!(matches!(&step.0, std::borrow::Cow::Owned(_)));
                assert_eq!(step.0, "mymod:block.x.step");
                assert_eq!(step.1, 0.3);
                assert_eq!(step.2, 1.5);

                let mut placed = Vec::new();
                for acting in [Some(7), None] {
                    placed.extend(game.with_level(|_, level, _| {
                        let place = |state| {
                            let pos = BlockPos::new(5, 64, 5);
                            level.override_place_sound(pos, state, acting).map(|sound| {
                                (sound.event.as_str().to_owned(), sound.volume, sound.pitch)
                            })
                        };
                        [place(glass), place(white_wool)]
                    }));
                }
                let placement = Some(answered("place"));
                assert_eq!(
                    placed,
                    [
                        placement.clone(),
                        placement.clone(),
                        placement,
                        Some(wool("place")),
                    ]
                );
                // A horse stepping on stone under a snow layer plays the snow's answer, asked at the
                // stone's position (`AbstractHorse.playStepSound`): volume 2.0 × 0.15, pitch 1.5.
                let stone = registry.default_state("minecraft:stone").unwrap();
                let snow = registry.default_state("minecraft:snow").unwrap();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                game.world.set(5, 64, 5, stone).unwrap();
                game.world.set(5, 65, 5, snow).unwrap();
                let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
                let mut horse = Mob::new(12, kind, Vec3::ZERO, 0.0);
                let mut events = Vec::new();
                equine::play_step(
                    &mut horse,
                    &game.mob_world_base(&[]),
                    [5, 64, 5],
                    stone,
                    &mut events,
                );
                assert!(
                    matches!(
                        events.as_slice(),
                        [MobEvent::Sound("entity.horse.step", _, volume, pitch)]
                            if *volume == 2.0 * 0.15 && *pitch == 1.5
                    ),
                    "{events:?}"
                );
                // Oak's part edited only its place event. The horse still hears WOOD's hoof sound,
                // and a hard crop sharing the wood step event still has an ordinary hoof sound.
                let air = registry.default_state("minecraft:air").unwrap();
                let planks = registry.default_state("minecraft:oak_planks").unwrap();
                let answered = sunrise_sim::block_sounds::sound_type(
                    sunrise_sim::motion::FrictionLevel {
                        world: &game.world,
                        registry: game.pack.assets.registry,
                        context: &game.loop_context,
                        side: &*game,
                        coefficient: &|_| None,
                    },
                    game.res.facts(),
                    BlockPos::new(5, 64, 5),
                    planks,
                    Some((horse.id, "minecraft:horse")),
                    Step,
                );
                assert_eq!(answered.place.as_str(), "mymod:unused_place");
                game.world.set(5, 65, 5, air).unwrap();
                for (block, expected) in [
                    ("minecraft:oak_planks", "entity.horse.step_wood"),
                    ("minecraft:pumpkin_stem", "entity.horse.step"),
                ] {
                    let state = registry.default_state(block).unwrap();
                    game.world.set(5, 64, 5, state).unwrap();
                    events.clear();
                    equine::play_step(
                        &mut horse,
                        &game.mob_world_base(&[]),
                        [5, 64, 5],
                        state,
                        &mut events,
                    );
                    assert!(
                        matches!(events.as_slice(), [MobEvent::Sound(name, ..)] if *name == expected)
                    );
                }
                // A panic drops only the unfinished edit and disables this mod's callback. It
                // neither disables an unrelated mod nor reports the same panic a second time.
                assert_eq!(
                    sound(game, "minecraft:green_wool", creeper, Step),
                    wool("step")
                );
                assert_eq!(
                    sound(game, "minecraft:glass", creeper, Step),
                    ("minecraft:block.glass.step".to_owned(), 1.0, 1.0)
                );
                assert_eq!(
                    sound(game, "minecraft:green_wool", creeper, Step),
                    wool("step")
                );
                assert_eq!(
                    sound(game, "minecraft:black_wool", creeper, Step),
                    ("other_mod:audible".to_owned(), 2.0, 1.5)
                );
                let notices = mod_api::__private::take_hook_panics();
                let mut ours = notices
                    .iter()
                    .filter(|notice| notice.mod_id == "sound_type_test");
                let notice = ours.next().expect("sound callback panic notice");
                assert_eq!(notice.hook, "sound_type");
                assert!(ours.next().is_none());
            });
        });
    }
}
