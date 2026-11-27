use super::*;

impl crate::game::ClientGame<'_> {
    #[inline(always)]
    pub(in crate::game) fn client_acting_player_uuid(&self) -> Option<u128> {
        self.client.remote.as_ref().map(|r| r.profile())
    }
    #[inline(always)]
    pub(in crate::game) fn client_resolve_enchant_particles(&self) {
        self.resolve_enchant_particles();
    }
    pub(in crate::game) fn install_pot_patterns(
        &self,
        assets: std::collections::HashMap<String, String>,
    ) {
        self.client_res().block_entity_render().set_pot_patterns(
            sunrise_client_render::block_entity_render::pot_pattern_textures(
                self.common.res.item_defaults(),
                |pattern| assets.get(pattern).cloned(),
            ),
        );
    }

    #[inline(always)]
    pub(in crate::game) fn client_tags_ready(&self) -> bool {
        false
    }
    #[inline(always)]
    pub(in crate::game) fn client_world_tables(&self) -> Option<&WorldTables> {
        None
    }
    pub(in crate::game::client_side) fn defer_world_entry(
        &mut self,
        entry: PendingWorldEntry,
    ) -> Result<bool> {
        self.common.poll_world_rest()?;
        if self.common.rest_loader.is_none() {
            return Ok(false);
        }
        self.client.perf.world_opening();
        if self.client.title.0.is_none() {
            let title = self
                .new_title(std::env::current_dir()?.join("saves"))
                .with_user(|| self.common.options.player_name());
            self.show_title(title);
        }
        self.client.pending_entry.0 = Some(entry);
        log::info!("Sunrise: world entry waiting for world resources");
        Ok(true)
    }
    /// Yield one frame before validating the selected datapacks.
    pub(in crate::game::client_side) fn poll_world_creation(&mut self) -> Result<()> {
        let Some(title) = self.client.title.0.as_ref() else {
            return Ok(());
        };
        if !title.preparing() {
            return Ok(());
        }
        if !std::mem::replace(&mut self.client.creation_read_started, true) {
            return Ok(());
        }
        self.client.creation_read_started = false;
        let (dir, packs) = title.creation_packs().unwrap_or_default();
        self.common.join_world_rest()?;
        use sunrise_gameplay_common::game::integrated_link::CreationPackRead;
        let outcome = self
            .common
            .options
            .read_creation_packs
            .ok_or("world creation validation is not configured")
            .map_err(str::to_owned)
            .and_then(|read| {
                read(CreationPackRead {
                    resources: &self.common.res,
                    pack: &self.common.pack,
                    options: &self.common.options,
                    settings: self.client.settings.sim_settings(),
                    pack_dir: dir.as_deref(),
                    selected: &packs,
                    profile_resolver: self.client.profile_resolver,
                })
                .map_err(|error| error.to_string())
            });
        if let Some(title) = self.client.title.0.as_mut() {
            match outcome {
                Ok(()) => title.finish_preparing(),

                Err(e) => {
                    log::warn!("Sunrise: preparing world creation failed: {e}");

                    if !title.pack_validation_failed() {
                        title.finish_preparing();
                        title.failed(format!(
                            "{}: {e}",
                            sunrise_game_data::lang::get_or(
                                "createWorld.preparing",
                                "Preparing for world creation..."
                            )
                        ));
                    }
                }
            }
        }
        Ok(())
    }
    pub fn resume_world_entry(&mut self) -> Result<()> {
        self.poll_integrated_start()?;
        self.common.poll_world_rest()?;
        self.poll_world_creation()?;
        if self.common.rest_loader.is_some() {
            return Ok(());
        }
        let on_world_screens = self.client.title.0.as_ref().is_some_and(|title| {
            !matches!(
                title.mode,
                sunrise_client::title::Mode::Main | sunrise_client::title::Mode::Multiplayer(_)
            )
        });
        sunrise_gameplay_common::game::world_preload::preload_world_data(
            on_world_screens,
            &self.common.options.resources,
            &self.common.pack.data_changes,
            self.common.pack.assets.registry,
        );
        if std::mem::take(&mut self.client.pending_pack_reload) {
            self.reload_packs();
        }
        match self.client.pending_entry.0.take() {
            Some(PendingWorldEntry::Local {
                folder,
                name,
                seed,
                creative,
                create,
            }) => {
                self.open_world(&folder, name.as_deref(), seed, creative, create.as_deref())?;
            }
            Some(PendingWorldEntry::Server {
                address,
                name,
                list_name,
                cookies,
                lan,
                realm,
            }) => {
                self.connect_server_with_options(
                    &address,
                    &name,
                    cookies,
                    lan,
                    realm,
                    list_name.as_deref(),
                )?;
            }
            Some(PendingWorldEntry::StudioReplay(path)) => {
                let path_str = path.to_string_lossy().to_string();
                if let Err(e) = self.open_studio_replay(&path_str) {
                    log::warn!("Sunrise Studio: could not open {path_str}: {e}");
                }
            }
            None => {}
        }
        Ok(())
    }
}
