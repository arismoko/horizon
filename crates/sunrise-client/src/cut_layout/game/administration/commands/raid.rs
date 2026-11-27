#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_command::vanilla;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_sim::motion::PlayerMotion;

    /// Runs a line of the dev tree (`/raid` is registered only there).
    fn run_dev(game: &mut sunrise_gameplay::game::ServerGame<'_>, line: &str) -> i32 {
        let d = vanilla::build_with(true);
        let parse = d.parse(line, &game.ui.registries);
        let (ctx, _) = parse.executable().expect(line);
        let (ctx, root) = (ctx.clone(), parse.context.clone());
        game.run_chain(&root, &ctx).expect(line)
    }

    #[test]
    #[ignore = "daily"]
    fn raid_check_finds_a_raid_counting_down_and_the_horn_is_local_only() {
        with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            // RaidCommand needs the owner's permission level (3).
            game.ui.registries.permission = 4;
            game.server.chat_capture = Some(Vec::new());
            let (id, _) = game
                .server
                .raiding
                .0
                .raids
                .create_or_extend([0, 64, 0], 2, None, 3);
            assert!(!game.server.raiding.0.raids.get(id).unwrap().started);

            assert_eq!(run_dev(game, "raid check"), 1);
            let lines: Vec<String> = game
                .server
                .chat_capture
                .take()
                .unwrap()
                .iter()
                .map(|t| t.plain())
                .collect();
            assert_eq!(
                lines,
                [
                    "Found a started raid!",
                    "Num groups spawned: 0 Raid omen level: 0 Num mobs: 0 Raid health: 0.0 / 0.0",
                ]
            );

            game.start_audio_capture();
            run_dev(game, "raid sound global");
            assert!(game.server.pending_sounds.is_empty());
            run_dev(game, "raid sound local");
            assert_eq!(game.server.pending_sounds.len(), 1);
            let sound = &game.server.pending_sounds[0];
            assert_eq!(sound.event, "minecraft:event.raid.horn");
            assert_eq!(sound.position, [5.5, 64.0, 0.5]);
            assert_eq!(sound.volume, 2.0);
            assert_eq!(sound.pitch, 1.0);
            assert!(sound.seed.is_some());
        });
    }
}
