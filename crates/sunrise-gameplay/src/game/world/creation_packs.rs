use crate::game::ServerGame;
use sunrise_game_data::Result;
use sunrise_gameplay_common::execution_role::ExecutionRole;
use sunrise_gameplay_common::game::integrated_link::{CreationPackRead, server_thread_stack};

pub fn read_creation_packs(request: CreationPackRead<'_>) -> Result<()> {
    let result = std::thread::scope(|scope| -> Result<_> {
        std::thread::Builder::new()
            .name("world-creation".into())
            .stack_size(server_thread_stack())
            .spawn_scoped(scope, move || {
                let load = || -> Result<()> {
                    let mut options = request.options.clone();
                    options.execution_role = ExecutionRole::IntegratedServer;
                    let mut server = ServerGame::new(
                        request.resources,
                        request.pack.clone(),
                        options,
                        request.settings,
                        None,
                        request.profile_resolver,
                    )?;
                    server.validate_creation_packs(request.pack_dir, request.selected)
                };
                load().map_err(|error| error.to_string())
            })?
            .join()
            .map_err(|_| "the world creation reader panicked".into())
    })?;
    result.map_err(Into::into)
}

impl ServerGame<'_> {
    pub(in crate::game) fn validate_creation_packs(
        &mut self,
        pack_dir: Option<&std::path::Path>,
        selected: &[String],
    ) -> Result<()> {
        let resources = self.common.options.resources.clone();
        let pack = &self.common.pack;
        let stack = crate::game::functions::creation_stack(
            pack_dir,
            &resources,
            &pack.data_source,
            &pack.data_changes,
            selected,
            &pack.additional_data,
        );
        let candidate = self.stage_world_tables(&stack, false, &resources)?;
        if let Some(tables) = candidate.tables {
            tables
                .biomes
                .overrides
                .validate_fill_layers(self.common.pack.assets.registry)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{CreationPackRead, read_creation_packs};
    use crate::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::test_fixtures::offline_profiles;

    #[test]
    #[ignore = "daily"]
    fn the_creation_read_refuses_a_pack_with_an_empty_jukebox_song() {
        let folder = tempfile::tempdir().unwrap();
        let pack = folder.path().join("bad_song");
        std::fs::create_dir_all(pack.join("data/test/jukebox_song")).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"fixture"}}"#,
        )
        .unwrap();
        std::fs::write(pack.join("data/test/jukebox_song/empty.json"), "{}").unwrap();
        with_game(|game| {
            let read = |selected: &[String]| {
                read_creation_packs(CreationPackRead {
                    resources: &game.common.res,
                    pack: &game.common.pack,
                    options: &game.common.options,
                    settings: game.server.settings.clone(),
                    pack_dir: Some(folder.path()),
                    selected,
                    profile_resolver: offline_profiles,
                })
            };
            assert!(read(&[]).is_ok());
            assert!(read(&["file/bad_song".to_owned()]).is_err());
        });
    }
}
