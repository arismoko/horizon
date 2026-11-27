#![cfg(test)]
use std::time::Duration;
use std::time::Instant;
use sunrise_gameplay_common::game::saving::ClosingSave;
use sunrise_gameplay_common::game::saving::Stage;
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Arc, mpsc};
    use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_sim::save::{LevelShape, SaveDir};

    pub(crate) fn finish_server_staged_save(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        save: &mut ClosingSave,
    ) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if game.poll_server_closing_save(save).unwrap() {
                return save.quit;
            }
            assert!(
                Instant::now() < deadline,
                "the region writer never acknowledged the save"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn both_successful_save_paths_resolve_the_banner_in_the_active_language() {
        use sunrise_game_data::lang::{Language, scoped};

        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_game(|game| {
                let dir = tempfile::tempdir().unwrap();
                game.server.world_root = dir.path().to_path_buf();
                game.server.save.0 = Arc::new(SaveDir::new(
                    game.server.world_root.clone(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));
                let translated = "[test locale]";
                let language = Language::of(
                    "en_test",
                    [("commands.save.success".into(), translated.into())],
                );
                scoped(language, || {
                    game.save_game().unwrap();
                    let explicit = sunrise_game_data::lang::get("commands.save.success");
                    let mut save = ClosingSave {
                        quit: false,
                        close: false,
                        reply: None,
                        started: Instant::now(),
                        stage: Stage::Writes(Some(game.server.save.0.flush_later().unwrap())),
                    };
                    assert!(!finish_server_staged_save(game, &mut save));
                    let incremental = sunrise_game_data::lang::get("commands.save.success");
                    assert_eq!(
                        (explicit.as_str(), incremental.as_str()),
                        (translated, translated)
                    );
                });
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn closing_and_control_saves_complete_after_reporting_a_record_failure() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_game(|game| {
                for quit in [false, true] {
                    let dir = tempfile::tempdir().unwrap();
                    game.server.save.0 = Arc::new(SaveDir::new(
                        dir.path().to_path_buf(),
                        LevelShape::OVERWORLD,
                        None,
                    ));
                    let invalid = (0..=DISK_WRITE_NESTING_GUARD)
                        .fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
                    game.server
                        .save
                        .0
                        .save_entities(vec![((2, 0), vec![invalid])])
                        .unwrap();
                    assert_eq!(game.server.save.0.record_save_failures(), 1);
                    let (send, receive) = mpsc::channel();
                    let mut save = ClosingSave {
                        quit,
                        close: quit,
                        reply: (!quit).then_some(send),
                        started: Instant::now(),
                        stage: Stage::Writes(Some(game.server.save.0.flush_later().unwrap())),
                    };
                    assert_eq!(finish_server_staged_save(game, &mut save), quit);
                    if !quit {
                        let reply: serde_json::Value =
                            serde_json::from_str(&receive.try_recv().unwrap()).unwrap();
                        assert_eq!(reply, serde_json::json!({"ok": true}));
                    }
                }
            });
        });
    }
}
