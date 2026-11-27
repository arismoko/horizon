#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;
    use sunrise_gameplay_common::game::server_side::native_scoreboard::tests::guest;
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt;

    use sunrise_gameplay::game::server_side::native_scoreboard::tests::open_lan;

    use super::super::native_entity_refs::tests::with_game;

    use mod_api::prelude::ScoreboardServerHost;
    use mod_api::{
        DisplaySlot, ObjectiveError, ObjectiveReadError, ObjectiveRenderType, ScoreEntry,
        ScoreHolder, TeamColor,
    };
    use std::ops::ControlFlow;

    use sunrise_game_data::scoreboard as sb;

    #[test]
    #[ignore = "daily"]
    fn objectives_are_written_synced_saved_and_stale_once_removed() {
        with_game(|game| {
            game.server.world_root = game.server.save.0.root.clone();
            let saved_board = open_lan(game);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .push(guest(123, 1));
            let player = game.player(123).unwrap();
            let text = mod_api::Text::literal;
            assert_eq!(
                ScoreboardServerHost::visit_objectives(game, &mut |_| panic!("empty board")),
                Ok(ControlFlow::Continue(()))
            );

            // Adding refuses what the board refuses, and nothing else.
            let kills = ScoreboardServerHost::add_objective(
                game,
                "kills",
                "dummy",
                text("Kills"),
                ObjectiveRenderType::Integer,
            )
            .unwrap();
            assert_eq!(
                ScoreboardServerHost::add_objective(
                    game,
                    "kills",
                    "dummy",
                    text("x"),
                    ObjectiveRenderType::Integer
                ),
                Err(ObjectiveError::NameTaken)
            );
            assert_eq!(
                ScoreboardServerHost::add_objective(
                    game,
                    "n",
                    "nonsense",
                    text("x"),
                    ObjectiveRenderType::Integer
                ),
                Err(ObjectiveError::UnknownCriteria)
            );
            // Any string is a name: empty, wildcard and long ones included.
            for odd in ["", "*", &"x".repeat(100)] {
                assert!(
                    ScoreboardServerHost::add_objective(
                        game,
                        odd,
                        "dummy",
                        text("x"),
                        ObjectiveRenderType::Integer
                    )
                    .is_ok()
                );
            }
            assert_eq!(game.objective("kills"), Some(kills));
            let mut first = Vec::new();
            assert_eq!(
                ScoreboardServerHost::visit_objectives(game, &mut |objective| {
                    first.push(objective);
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first, [kills]);
            assert_eq!(kills.name(game), Some("kills"));
            assert_eq!(kills.criteria(game), Some("dummy"));
            assert_eq!(kills.display_name(game), Ok(text("Kills")));
            assert_eq!(
                kills.visit_slots(game, &mut |_| panic!("no display slot")),
                Ok(ControlFlow::Continue(()))
            );
            assert_eq!(
                kills.visit_scores(game, &mut |_| panic!("no score")),
                Ok(ControlFlow::Continue(()))
            );

            // A player and that player's entity are one holder, keyed by the name.
            let as_player = ScoreHolder::Player(player);
            let as_entity = ScoreHolder::Entity(player.entity());
            kills.set_score(game, as_player.clone(), 3).unwrap();
            assert_eq!(kills.add_score(game, as_entity.clone(), 2), Ok(5));
            assert_eq!(kills.score(game, as_player), Some(5));
            assert_eq!(
                kills.score(game, ScoreHolder::Name("guest1".into())),
                Some(5)
            );
            assert_eq!(game.score.board.score("guest1", "kills").unwrap().value, 5);
            kills
                .set_score(game, ScoreHolder::Name("#zero".into()), 0)
                .unwrap();
            assert_eq!(
                kills.score(game, ScoreHolder::Name("#zero".into())),
                Some(0)
            );
            let mut scores = Vec::new();
            assert_eq!(
                kills.visit_scores(game, &mut |row| {
                    scores.push((row.holder.to_owned(), row.value));
                    ControlFlow::Continue(())
                }),
                Ok(ControlFlow::Continue(()))
            );
            // The fastutil board walks bucket 26 (#zero) before bucket 14 (guest1).
            assert_eq!(
                scores,
                [(String::from("#zero"), 0), (String::from("guest1"), 5)]
            );
            assert_eq!(
                kills.visit_scores(game, &mut |row| {
                    assert_eq!(row, ScoreEntry::new("#zero", 0));
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            // The command reads what the mod wrote, and the lock starts set.
            assert_eq!(kills.score_locked(game, as_entity.clone()), Some(true));
            assert!(kills.set_score_locked(game, as_entity, false));

            // Read-only criteria refuse a write, as the command does.
            let hp = ScoreboardServerHost::add_objective(
                game,
                "hp",
                "health",
                text("HP"),
                ObjectiveRenderType::Hearts,
            )
            .unwrap();
            assert_eq!(
                hp.set_score(game, ScoreHolder::Name("x".into()), 1),
                Err(ObjectiveError::ReadOnlyCriteria)
            );
            // A holder that is not on the server is not a stale handle.
            let offline = ScoreHolder::Entity(mod_api::EntityRef::new(
                game.native_lifecycle.epoch().unwrap(),
                9_999_999,
            ));
            assert_eq!(
                kills.set_score(game, offline.clone(), 1),
                Err(ObjectiveError::UnknownHolder)
            );
            assert_eq!(
                kills.add_score(game, offline.clone(), 1),
                Err(ObjectiveError::UnknownHolder)
            );
            assert_eq!(kills.score(game, offline), None);

            // Shown in the sidebar, a guest's view has it with its score.
            let red_sidebar = DisplaySlot::TeamSidebar(TeamColor::Red);
            assert!(game.set_display_slot(red_sidebar, Some(kills)));
            assert!(game.set_display_slot(DisplaySlot::Sidebar, Some(kills)));
            assert_eq!(
                game.display_slot_objective(DisplaySlot::Sidebar),
                Some(kills)
            );
            let mut slots = Vec::new();
            assert_eq!(
                kills.visit_slots(game, &mut |slot| {
                    slots.push(slot);
                    ControlFlow::Continue(())
                }),
                Ok(ControlFlow::Continue(()))
            );
            assert_eq!(slots, [DisplaySlot::Sidebar, red_sidebar]);
            game.lan_sync_scoreboard();
            let view = sunrise_sim::lan::scoreboard::View::of(&game.score.board);
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].scoreboard,
                view
            );

            // Saved, the file holds the objective, its score and its slot, as a command's would.
            game.save_scoreboard();
            let saved = sunrise_sim::save::read_dat(&saved_board).unwrap();
            let loaded = sb::Scoreboard::unpack(saved.get("data").unwrap());
            assert_eq!(loaded.score("guest1", "kills").unwrap().value, 5);
            assert_eq!(
                loaded.display.get("sidebar").map(String::as_str),
                Some("kills")
            );

            // Reopened, the saved objective is there for a mod reading at startup, before any
            // write or tick; a handle from before the open is stale.
            game.native_lifecycle_open_started();
            game.score = Default::default();
            game.native_lifecycle_world_ready();
            let kills_again = game.objective("kills").unwrap();
            assert_ne!(kills_again, kills);
            assert_eq!(kills.name(game), None);
            assert_eq!(
                kills_again.score(game, ScoreHolder::Name("guest1".into())),
                Some(5)
            );
            assert_eq!(
                game.display_slot_objective(DisplaySlot::Sidebar),
                Some(kills_again)
            );

            // Removed, and added again under its name: the old handle stays stale.
            assert!(kills_again.remove(game));
            let renamed = ScoreboardServerHost::add_objective(
                game,
                "kills",
                "dummy",
                text("Again"),
                ObjectiveRenderType::Integer,
            )
            .unwrap();
            assert_ne!(renamed, kills_again);
            assert_eq!(kills_again.name(game), None);
            assert_eq!(kills_again.display_name(game), Err(ObjectiveError::Stale));
            assert_eq!(
                kills_again.visit_scores(game, &mut |_| panic!("stale objective")),
                Err(ObjectiveReadError::Stale)
            );
            assert_eq!(
                kills_again.visit_slots(game, &mut |_| panic!("stale objective")),
                Err(ObjectiveReadError::Stale)
            );
            assert_eq!(
                kills_again.set_score(game, ScoreHolder::Name("a".into()), 1),
                Err(ObjectiveError::Stale)
            );
            assert!(!kills_again.remove(game));
            assert_eq!(renamed.score(game, ScoreHolder::Name("a".into())), None);
            // An aborted transaction puts the board back; a committed one keeps its writes.
            let before = game.score.board.clone();
            let aborted: Result<(), ()> = game.storage_transaction(|host, _| {
                let temp = ScoreboardServerHost::add_objective(
                    host,
                    "temp",
                    "dummy",
                    text("T"),
                    ObjectiveRenderType::Integer,
                )
                .map_err(|_| ())?;
                temp.set_score(host, ScoreHolder::Name("p".into()), 4)
                    .map_err(|_| ())?;
                assert!(renamed.remove(host));
                Err(())
            });
            assert!(aborted.is_err());
            assert_eq!(game.score.board, before);
            assert!(game.objective("temp").is_none());
            assert_eq!(renamed.name(game), Some("kills"));
            let kept: Result<(), ()> = game.storage_transaction(|host, _| {
                ScoreboardServerHost::add_objective(
                    host,
                    "temp",
                    "dummy",
                    text("T"),
                    ObjectiveRenderType::Integer,
                )
                .map(|_| ())
                .map_err(|_| ())
            });
            assert!(kept.is_ok());
            assert!(game.objective("temp").is_some());

            // The command's removal makes a handle stale the same way.
            game.ui.registries.permission = 4;
            assert!(game.run_command("scoreboard objectives remove kills"));
            assert_eq!(renamed.name(game), None);
            let _ = std::fs::remove_file(&saved_board);
        });
    }
}
