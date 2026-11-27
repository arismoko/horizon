#[cfg(test)]
mod tests {
    use mod_api::{Identifier, TextureAtlasRegistration};

    /// Registers the fixture atlases only on the first reload.
    struct Maps(std::sync::Arc<std::sync::Mutex<(u32, Vec<Option<mod_api::AtlasRegion>>)>>);
    fn maps() -> TextureAtlasRegistration {
        registration("maps_mod:maps", "maps_mod:maps")
    }
    /// An atlas whose name holds `presets`, which the menu's own picture folder also has.
    fn presets() -> TextureAtlasRegistration {
        registration("maps_mod:presets", "maps_mod:presets")
    }
    fn player(atlas: &TextureAtlasRegistration) -> Identifier {
        atlas.sprite_name(&Identifier::parse("maps_mod:player").unwrap())
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Maps {
        fn register_texture_atlases(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterTextureAtlasesEvent<'_>,
        ) {
            let mut kept = self.0.lock().unwrap();
            kept.0 += 1;
            if kept.0 == 2 {
                event.register(maps());
                event.register(presets());
            }
        }

        fn texture_atlas_stitched(
            &self,
            host: &mut dyn mod_api::ClientHost,
            event: &mut mod_api::TextureAtlasStitchedEvent,
        ) {
            if event.atlas == mod_api::TextureAtlas::Gui {
                self.0
                    .lock()
                    .unwrap()
                    .1
                    .push(host.gui_sprite(&player(&maps())));
            }
        }
    }

    fn registration(atlas: &str, definition: &str) -> TextureAtlasRegistration {
        TextureAtlasRegistration {
            atlas: Identifier::parse(atlas).unwrap(),
            definition: Identifier::parse(definition).unwrap(),
        }
    }

    fn fixture_png(width: u32, height: u32, channel: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&vec![channel; (width * height * 4) as usize])
            .unwrap();
        bytes
    }

    /// The registered atlas's sprite is in the GUI lookup at its own size after a reload that
    /// registers it, drawable by its name, and gone after a reload that no longer registers it.
    #[test]
    #[ignore = "daily"]
    fn a_mod_atlas_sprite_is_stitched_into_the_gui_atlas_until_a_reload_drops_it() {
        if crate::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::client_side::client_reload::texture_atlases::tests::a_mod_atlas_sprite_is_stitched_into_the_gui_atlas_until_a_reload_drops_it",
            "SUNRISE_ATLAS_RELOAD_CHILD",
            &["reload"],
        ) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mod_dir = dir.path().join("maps_mod");
        std::fs::create_dir_all(mod_dir.join("assets/maps_mod/atlases")).unwrap();
        std::fs::create_dir_all(mod_dir.join("assets/maps_mod/textures/markers")).unwrap();
        std::fs::write(
            mod_dir.join("sunrise-mod.toml"),
            "id = \"maps_mod\"\nname = \"Maps mod\"\nversion = \"1\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n",
        )
        .unwrap();
        let def = r#"{"sources":[{"type":"single","resource":"maps_mod:markers/player","sprite":"maps_mod:player"}]}"#;
        std::fs::write(mod_dir.join("assets/maps_mod/atlases/maps.json"), def).unwrap();
        std::fs::write(mod_dir.join("assets/maps_mod/atlases/presets.json"), def).unwrap();
        std::fs::write(
            mod_dir.join("assets/maps_mod/textures/markers/player.png"),
            fixture_png(8, 8, 200),
        )
        .unwrap();

        let kept = std::sync::Arc::default();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "maps_mod",
            Box::new(Maps(std::sync::Arc::clone(&kept))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                game.common.mods.dev.push(mod_dir.clone());
                let reload = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                    game.spawn_pack_reload(
                        sunrise_client_gameplay::game::client_side::packs::ReloadKind::Selection,
                    )
                    .unwrap();
                    let started = std::time::Instant::now();
                    while game.client.pack_reload.0.is_some() {
                        assert!(
                            started.elapsed().as_secs() < 300,
                            "the reload never finished"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        game.drive_pack_reload();
                    }
                    assert_eq!(game.client.chat_ui.options.resource_error, None);
                };
                reload(game);
                let map_sprite = player(&maps());
                let preset_sprite = player(&presets());
                assert_eq!(
                    game.gui_sprite_pixels(&map_sprite.to_string()),
                    Some([8.0, 8.0])
                );
                assert_eq!(
                    game.gui_sprite_pixels(&preset_sprite.to_string()),
                    Some([8.0, 8.0]),
                    "a sprite under an atlas named presets is stitched too"
                );
                {
                    let kept = kept.lock().unwrap();
                    assert_eq!(kept.0, 2);
                    assert_eq!(
                        kept.1.last().copied().flatten().map(|r| r.size),
                        Some([8, 8])
                    );
                }
                reload(game);
                let kept = kept.lock().unwrap();
                assert_eq!(kept.0, 3);
                assert_eq!(
                    kept.1.last().copied(),
                    Some(None),
                    "looked up in the new atlas"
                );
                assert_eq!(game.gui_sprite_pixels(&map_sprite.to_string()), None);
                assert_eq!(game.gui_sprite_pixels(&preset_sprite.to_string()), None);
            });
        });
    }
}
