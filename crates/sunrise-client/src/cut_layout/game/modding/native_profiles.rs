#![cfg(test)]
#[cfg(test)]
mod tests {
    use mod_api::{ProfileResolution, ServerHost};
    use {
        sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
        sunrise_gameplay_common::game::execution_role::ExecutionRole,
    };

    /// A name `StringUtil.isValidPlayerName` refuses answers at once with no profile, as
    /// `fetchByName` does without a request.
    #[test]
    #[ignore = "daily"]
    fn invalid_names_resolve_no_profile() {
        if run_fresh_cases(
            "cut_layout::game::server_side::native_profiles::tests::invalid_names_resolve_no_profile",
            "SUNRISE_PROFILES_CASE",
            &["server"],
        ) {
            return;
        }
        with_game(|game| {
            game.options.execution_role = ExecutionRole::IntegratedServer;
            for name in ["two words", "seventeen_letters", "ünicode"] {
                let host: &mut dyn ServerHost = game;
                assert_eq!(
                    host.profiles().resolve_name(name),
                    ProfileResolution::Done(None),
                    "{name}"
                );
            }
        });
    }
}
