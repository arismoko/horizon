impl crate::game::ClientGame<'_> {
    /// Validate and publish the calling mod's runtime entries for one language.
    pub(in crate::game) fn set_translations(
        &self,
        code: &str,
        entries: Vec<(String, String)>,
    ) -> Option<usize> {
        const MAX_ENTRIES: usize = 65_536;
        const MAX_KEY: usize = 256;
        const MAX_TEXT: usize = 32_767;
        let mod_id = mod_api::content::crash::current_native_mod()?;
        let code_ok = (1..=16).contains(&code.len())
            && code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !code_ok
            || entries.len() > MAX_ENTRIES
            || entries
                .iter()
                .any(|(key, text)| key.is_empty() || key.len() > MAX_KEY || text.len() > MAX_TEXT)
        {
            return None;
        }
        Some(sunrise_game_data::lang::set_mod_entries(
            mod_id,
            code,
            entries.into_iter().collect(),
        ))
    }
    /// Drops all of the calling mod's run-time translation entries.
    pub(in crate::game) fn clear_translations(&self) {
        if let Some(mod_id) = mod_api::content::crash::current_native_mod() {
            sunrise_game_data::lang::clear_mod_entries(mod_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_client_game;
    use sunrise_game_data::lang::{self, Language};
    use sunrise_gameplay_common::game::mods::ModGameExt1 as _;

    #[test]
    #[ignore = "daily"]
    fn disabling_native_mod_hides_runtime_translations_across_reload_and_restores_on_reenable() {
        with_client_game(|game| {
            let _publication = lang::isolate_publication(Language::of(
                "en_us",
                [("test.switch.k".into(), "pack v1".into())],
            ));
            {
                let _guard = mod_platform::content::crash::enter_native_mod(
                    mod_api::content::crash::ModCall {
                        id: "switch-owner",
                        hook: "test",
                        part: None,
                    },
                );
                assert_eq!(
                    game.set_translations(
                        "en_us",
                        vec![("test.switch.k".into(), "mod value".into())],
                    ),
                    Some(1)
                );
            }
            assert_eq!(
                game.translate("test.switch.k").as_deref(),
                Some("mod value")
            );

            game.client
                .settings
                .mods
                .disabled
                .insert("switch-owner".into());
            game.sync_native_switches();
            assert_eq!(game.translate("test.switch.k").as_deref(), Some("pack v1"));

            lang::set(Language::of(
                "en_us",
                [("test.switch.k".into(), "pack v2".into())],
            ));
            assert_eq!(game.translate("test.switch.k").as_deref(), Some("pack v2"));

            game.client.settings.mods.disabled.remove("switch-owner");
            game.sync_native_switches();
            assert_eq!(
                game.translate("test.switch.k").as_deref(),
                Some("mod value")
            );
        });
    }
}
