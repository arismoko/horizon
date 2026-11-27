#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_sim::save::level_mods;

    /// Without the switch the server refuses, naming the switch, and changes nothing in the
    /// world; with it the world opens.
    #[test]
    #[ignore = "daily"]
    fn check_missing_mods_refuses_unless_allowed_and_writes_nothing() {
        use sunrise_core::nbt::Tag;
        let dir = std::env::temp_dir().join(format!(
            "sunrise-dedicated-missing-mod-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let gone = Tag::compound([
            ("ModId", Tag::String("gone".into())),
            ("ModVersion", Tag::String("1.0.0".into())),
        ]);
        sunrise_sim::save::write_dat(
            &dir.join("level.dat"),
            Tag::compound([(
                "fml",
                Tag::compound([("LoadingModList", Tag::List(vec![gone]))]),
            )]),
        )
        .unwrap();
        let before = std::fs::read(dir.join("level.dat")).unwrap();
        sunrise_gameplay::game::native_entity_refs::tests::with_game_role(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap()),
            sunrise_gameplay_common::game::execution_role::ExecutionRole::DedicatedServer,
            |game| {
                let refused = game
                    .check_missing_mods(&dir, false)
                    .unwrap_err()
                    .to_string();
                assert!(refused.contains(level_mods::ALLOW_FLAG), "{refused}");
                assert!(game.check_missing_mods(&dir, true).is_ok());
            },
        );
        assert_eq!(std::fs::read(dir.join("level.dat")).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
