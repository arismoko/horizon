#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {
    use mod_api::{Hooks, HostSide, ModEntry, ModMismatchEvent};
    use std::sync::{Arc, Mutex};
    use sunrise_gameplay_common::game::execution_role;
    use sunrise_gameplay_common::game::native_hooks::mod_mismatch::GameHooks;
    use sunrise_gameplay_common::game::native_hooks::mod_mismatch::level_mods;

    #[test]
    #[ignore = "daily"]
    fn existing_lists_fire_on_client_and_dedicated_even_without_differences() {
        struct Listener(Arc<Mutex<usize>>);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn mod_mismatch(&self, _: HostSide, event: &mut ModMismatchEvent<'_>) {
                assert!(event.mismatches.is_empty());
                *self.0.lock().unwrap() += 1;
            }
        }
        let heard = Arc::new(Mutex::new(0));
        mod_platform::testing::with_mods(
            vec![ModEntry::new("listener", Box::new(Listener(heard.clone())))].into(),
            || {
                for role in [
                    execution_role::ExecutionRole::Client,
                    execution_role::ExecutionRole::DedicatedServer,
                ] {
                    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                        game.options.execution_role = role;
                        game.level.saved_mods = None;
                        game.check_mod_mismatch();
                        game.level.saved_mods = Some(vec![]);
                        game.check_mod_mismatch();
                    });
                }
            },
        );
        assert_eq!(*heard.lock().unwrap(), 2);
    }

    /// A world saved with `gone`, a mod no test loads, and its block namespace.
    fn world_with_gone_mod(name: &str) -> std::path::PathBuf {
        use sunrise_core::nbt::Tag;
        let dir = std::env::temp_dir().join(format!("sunrise-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let entry = Tag::compound([
            ("ModId", Tag::String("gone".into())),
            ("ModVersion", Tag::String("1.0.0".into())),
        ]);
        sunrise_sim::save::write_dat(
            &dir.join("level.dat"),
            Tag::compound([
                (
                    "Data",
                    Tag::compound([("LevelName", Tag::String(name.into()))]),
                ),
                (
                    "fml",
                    Tag::compound([("LoadingModList", Tag::List(vec![entry]))]),
                ),
                (
                    level_mods::COMPOUND,
                    Tag::compound([(
                        "BlockNamespaces",
                        Tag::List(vec![Tag::String("gone".into())]),
                    )]),
                ),
            ]),
        )
        .unwrap();
        dir
    }

    /// A listener that resolves the missing mod takes it off the question, so no screen shows;
    /// without one it is asked about. Asking reads the world and writes nothing.
    #[test]
    #[ignore = "daily"]
    fn a_resolved_missing_mod_is_not_asked_about() {
        struct Migrator;
        #[mod_api::__subscriptions]
        impl Hooks for Migrator {
            fn mod_mismatch(&self, _: HostSide, event: &mut ModMismatchEvent<'_>) {
                event.mark_resolved("gone");
            }
        }
        let dir = world_with_gone_mod("resolved-missing-mod");
        let before = std::fs::read(dir.join("level.dat")).unwrap();
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let missing = game.ask_missing_mods(&dir);
            assert_eq!(
                missing.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
                ["gone"]
            );
        });
        mod_platform::testing::with_mods(
            vec![ModEntry::new("migrator", Box::new(Migrator))].into(),
            || {
                sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                    assert!(game.ask_missing_mods(&dir).is_empty());
                    // The open uses this answer instead of asking the mods again.
                    let (_, resolved) = level_mods::take_answer(&dir).unwrap();
                    assert_eq!(resolved[0].resolver, "migrator");
                });
            },
        );
        assert_eq!(std::fs::read(dir.join("level.dat")).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
