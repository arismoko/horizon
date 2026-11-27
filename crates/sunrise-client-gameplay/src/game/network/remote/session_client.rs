use super::level_format::{LevelEnvironments, LevelFormat, pack_environment, same_stacks};
pub(in crate::game) use super::opening::RemoteOpening;
use super::*;
use sunrise_level::environment::Environment;
use sunrise_protocol::packets::configuration::Transfer;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn client_refresh_infiniburn(&mut self) {
        self.common.world.infiniburn = Some(self.resolve_level_infiniburn(self.dimension_type()));
    }
    /// Close the transport before finishing any pack feedback, but keep the pack manager's
    /// prompt state until that feedback has been processed.
    pub(in crate::game::client_side) fn close_server_session(
        &mut self,
        reason: Option<Text>,
        before_pack_cleanup: impl FnOnce(&mut crate::game::server_packs::ServerPackState),
    ) -> Result<()> {
        self.client_logging_out();
        self.finish_leave_server_with(reason, before_pack_cleanup)
    }

    /// Tear down the client's server session without firing `client_logging_out`:
    /// used after `client_logging_out` has already run (`leave_server`, integrated
    /// server close), and for aborted or failed `ConnectScreen` / login-handshake
    /// attempts that never reached `ClientCommonPacketListenerImpl` or
    /// `Minecraft.disconnect`.
    pub(in crate::game) fn finish_leave_server(&mut self, reason: Option<Text>) -> Result<()> {
        self.finish_leave_server_with(reason, |_| {})
    }

    fn finish_leave_server_with(
        &mut self,
        reason: Option<Text>,
        before_pack_cleanup: impl FnOnce(&mut crate::game::server_packs::ServerPackState),
    ) -> Result<()> {
        self.finish_perf();
        // The one boundary every network session passes when it closes here: the player leaving,
        // or this client ending it with a reason (a raw disconnect closed it already, with the
        // connection's or the server's).
        match &reason {
            None => self.connection_left(sunrise_hooks::connection::Left::Disconnected),
            Some(reason) => self.connection_ended(
                &reason.plain(),
                reason.translation_key(),
                sunrise_hooks::connection::By::Client,
            ),
        }
        self.clear_waypoints();
        self.hud_disconnected();
        self.reset_server_debug_lines();
        // Vanilla's Hud also ticks on the title and connect screens, so the
        // action bar has run out by the next world; Sunrise's ticks in one.
        self.client.titles.overlay_time = 0;
        // GameRenderer.resetData and narrator.clear (Minecraft.disconnect).
        self.client.item_activation.reset();
        // Kept mod block entity geometry belongs to the world leaving.
        self.client.kept_block_entities.get_mut().clear();
        self.client.debug_density.borrow_mut().clear();
        self.client.mod_view_distances.take();
        self.clear_narrator();
        // A world is leaving either way: its icon attempt goes with it.
        self.client.world_icon = None;
        // Minecraft.disconnect: the client level goes.
        if self.client.remote.is_some() {
            self.forget_all_remote_columns();
        }
        if let Some(old) = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.heard_level.take())
        {
            self.native_client_level_unload(&old);
        }
        self.client.packet_holds.borrow_mut().clear();
        self.client.native_client_commands.pending_commands.clear();
        self.client.native_client_commands.pending_chats.clear();
        if let Some(mut r) = self.client.remote.take() {
            r.session.disconnect();
            self.clear_spectated_entity_post_effect();
            self.reset_creative_contents();
            self.refresh_creative_search_tags();
        }
        // Minecraft.disconnect -> updateLevelInEngines(null), after logout and level
        // unload: stop even the sounds a listener started during teardown.
        self.client.ambience.clear(&mut self.client.audio.0);
        self.client
            .music
            .stop_playing(None, &mut self.client.audio.0);
        self.client.audio.0.leave_level();
        self.client.jukebox_sounds.clear();
        self.client.mod_sounds.clear();
        self.update_window_title();
        self.client.gui.atlas.clear_object_skins();
        before_pack_cleanup(&mut self.client.server_packs.0);
        self.server_packs_disconnect();
        self.client.code_of_conduct = None;
        self.reset_tutorial();
        self.close_dialogs();
        // Gui.setScreen -> KeyMapping.releaseAll: the title or disconnect
        // screen releases every held key. Its release goes to that screen,
        // so a key kept here would still be down in the next world (#3246).
        self.clear_device_keys();
        self.set_mapping_held("key.attack", false);
        self.set_mapping_held("key.use", false);
        // The shared world is closing: its invite code ends with it.
        self.friends_lan_closed();
        // Minecraft.disconnect waits for the integrated server before showing the title.
        let had_integrated = self.client.integrated.is_some();
        let stop_result = self
            .client
            .integrated
            .take()
            .map(|server| server.stop())
            .transpose();
        // Keep the server's registry snapshot until the thread has joined, even on error.
        sunrise_gameplay_common::game::lan_common::sync::forget_sync_data();
        mod_api::content::crash::clear_detail("Server");
        self.common.clock = Default::default();
        self.common.adv = Default::default();
        self.common.stats = Default::default();
        self.common.score.board = Default::default();
        self.common.server_items = None;
        self.common.recipe_tables.clear_session();
        self.reset_client_recipe_book();
        self.refresh_recipe_property_context();
        self.common.world_biomes = None;
        self.common.world_biomes_base = None;
        self.client.streaming = MeshStreamer::new(Arc::new(NoTerrain), self.worker_context(), 1);
        self.client.streaming.paused = true;
        self.reset_client_world(0);
        self.remove_local_player_body();
        self.client.post_effects.clear();
        self.clear_spectated_entity_post_effect();
        // The vehicle went with the level: an id kept here would seat the
        // next login on an entity that is not there.
        self.common.riding = None;
        // Out of the server's bed: InBedChatScreen goes with the level.
        self.common.sleeping = None;
        self.common.sleep_timer = 0;
        self.client_clear_entities();
        self.common.score.board = Default::default();
        self.client.tab_list.0 = Default::default();
        self.common.border = Default::default();
        self.set_screen(Screen::Playing);
        self.common.container = None;
        self.clear_lod();
        self.client.pending_uploads.0.clear();
        if let Some(r) = self.client.renderer.0.as_mut() {
            r.clear_sections(self.common.world.layout());
        }
        let saves = std::env::current_dir().unwrap_or_default().join("saves");
        let mut title = self
            .new_title(saves.clone())
            .with_user(|| self.common.options.player_name());
        title.last_server = self.client.settings.file.string("lastServer");
        title.skip_multiplayer_warning = self.client.settings.file.bool("skipMultiplayerWarning");
        if let Err(err) = stop_result {
            // Always restore the title screen before reporting an integrated-server stop failure.
            title.failed(err.to_string());
            self.show_title(title);
            self.capture(false);
            return Err(err);
        }
        if had_integrated && reason.is_none() {
            self.show_title(title);
            self.capture(false);
            return Ok(());
        }
        title.mode = sunrise_client::title::Mode::Multiplayer(match reason {
            Some(reason) => sunrise_client::multiplayer::Screen::Disconnected {
                reason,
                retry: None,
                lines: std::cell::Cell::new(1),
            },
            None => sunrise_client::multiplayer::Screen::list(&saves),
        });
        self.show_title(title);
        self.offer_retry();
        self.capture(false);
        Ok(())
    }
    pub fn connect_remote_session(
        &mut self,
        address: &str,
        name: &str,
        local: Option<sunrise_protocol::connection::Pipe>,
        cookies: Option<sunrise_client_lib::net::Cookies>,
        lan: bool,
        realm: bool,
        list_name: Option<&str>,
    ) -> Result<()> {
        let opening =
            self.open_remote_session(address, name, local, cookies, lan, true, list_name)?;
        self.install_remote_session(opening, lan, realm)
    }
    /// Switch to a (possibly new) dimension: a fresh client level.
    pub fn enter_remote_dimension(
        &mut self,
        spawn: &play::SpawnInfo,
        entry: RemoteDimensionEntry,
        reason: sunrise_client_render::level_loading::Reason,
    ) {
        // The replacement player starts with its inventory menu, even if the
        // server switched levels without closing the previous container.
        self.remote_close_local();
        self.common.carried = None;
        self.common.craft_grid.fill(None);
        self.set_screen(Screen::Playing);
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        r.craft_result = None;
        r.state_id = 0;
        // handleRespawn keeps the ClientLevel only for the same level key:
        // two levels sharing a dimension type are still two levels.
        let same = matches!(
            entry,
            RemoteDimensionEntry::Respawn | RemoteDimensionEntry::DeathRespawn
        ) && r.heard_level.as_deref() == Some(spawn.dimension.as_str());
        // An open LevelLoadingScreen keeps its presentation, even if it is vanilla.
        // New screens for login and death do not receive transition keys.
        let transition_from = if matches!(entry, RemoteDimensionEntry::Respawn)
            && !r.level_load.is_active()
            && r.heard_level.is_some()
        {
            Some(r.level_key)
        } else {
            None
        };
        let adjust_mode = if matches!(
            entry,
            RemoteDimensionEntry::Respawn | RemoteDimensionEntry::DeathRespawn
        ) {
            self.common.mode
        } else {
            sunrise_command::args::GameMode::Survival
        };
        let mode = sunrise_gameplay_common::game_modes::mode_from_id(spawn.game_type);
        let previous = spawn
            .previous_game_type
            .map(sunrise_gameplay_common::game_modes::mode_from_id);
        let body_flight = self
            .common
            .player
            .as_ref()
            .map(|p| (p.flying, p.flying_speed));
        let r = self.client.remote.as_mut().unwrap();
        let new_logical_player =
            !matches!(entry, RemoteDimensionEntry::Login) || !r.has_logical_player;
        // A Sunrise server's bulk edit sends whole columns with the light
        // from before its relight; any other server's packet light is final.
        let relight_resent = r.quiet_commands;
        let format: LevelFormat = r.level_format(spawn);
        let level_key = mod_api::__private::dimension_key(&spawn.dimension);
        let replacement = if same {
            None
        } else {
            // ClientLevel's constructor posts Load before Minecraft.setLevel unloads
            // the old level. Give the new level its own handle without retiring the
            // old one, which must still read its existing chunks during Unload.
            let epoch = sunrise_gameplay_common::game::native_lifecycle_common::fresh_epoch();
            let handle = mod_api::LevelRef::new(epoch, level_key);
            let mut world =
                sunrise_gameplay_common::game::native_hooks::block_entity_on_load::world_for_mods(
                    self.common.res.facts().air,
                    spawn.seed,
                );
            world.shape = format.shape;
            world.game_time = self.common.world.game_time;
            r.constructing_level = Some(ConstructingClientLevel {
                handle,
                world,
                facts: format.facts.clone(),
            });
            self.native_client_level_load(&spawn.dimension);
            let level = self
                .client
                .remote
                .as_mut()
                .unwrap()
                .constructing_level
                .take()
                .unwrap();
            self.unload_client_level();
            self.forget_all_remote_columns();
            Some(level)
        };
        // Keep the retiring level's metadata in place until its Unload has returned.
        self.track_remote_levels(&spawn.dimension);
        let r = self.client.remote.as_mut().unwrap();
        if !same {
            r.collider_shove_start = None;
            // A same-level respawn keeps the existing ClientLevel's seed and sea level.
            r.hashed_seed = spawn.seed;
            r.sea_level = spawn.sea_level;
        }
        let dimension = format.dimension;
        r.chunk_format = format.chunk_format;
        r.level_facts = format.facts;
        r.synced_type = format.synced_type;
        r.dimension = dimension;
        r.resolve_clocks();
        self.common.world.day_time = r.clock_time(r.default_clock);
        r.level = spawn.dimension.clone();
        r.level_key = level_key;
        if new_logical_player {
            self.client.drop_spam_throttler = Default::default();
            r.abilities = sunrise_sim::lan::abilities::PlayerAbilities::default();
            r.own_player_info = std::cell::OnceCell::new();
            // The new LocalPlayer, admitted now, counts its ticks from 0.
            r.own_tick_count = 0;
        } else if let Some((flying, flying_speed)) = body_flight {
            r.sync_body_flight(flying, flying_speed);
        }
        r.has_logical_player = true;
        // handleLogin/handleRespawn run adjustPlayer(newPlayer) before setLocalMode.
        r.abilities.update_for_mode(adjust_mode.id());
        r.flat = spawn.flat;
        r.loaded_sent = false;
        // A replacement LevelLoadingScreen starts its bar from nothing (the
        // first one carries on from the server's start).
        if self.common.player.is_some() {
            self.client.loading_progress = 0.0;
        }
        // startWaitingForNewLevel: `LevelLoadTracker.startClientLoad` and the
        // `LevelLoadingScreen`.
        let transition_factory = transition_from.and_then(|from| {
            sunrise_client_render::dimension_transition::select(r.level_key, from)
        });
        r.level_load.start_client_load(client_millis(), reason);
        // A replacement LocalPlayer has a fresh ambient mood and sendPosition baseline.
        self.client.ambience.moodiness = 0.0;
        r.position_sender = Default::default();
        r.predictions.clear();
        self.set_local_mode_with_previous(mode, previous);
        self.common.dim = Dim::from_id(&spawn.dimension);
        self.build_level_environments();
        self.sync_timeline_registries();
        // ClientPacketListener.handleLogin / handleRespawn: the server's
        // Player.getLastDeathLocation, for the recovery compass.
        self.common.level.last_death = spawn
            .last_death
            .as_ref()
            .map(|d| (d.dimension.clone(), d.pos));
        if let Some(mut level) = replacement {
            self.common.clock.reset_client_level_tick_state();
            let mut streaming = MeshStreamer::new(
                Arc::new(NoTerrain),
                self.worker_context(),
                self.common.res.threads(),
            );
            streaming.remote = true;
            streaming.radius = self.client.streaming.radius;
            // SpawnInfo already carries the hashed seed the client's BiomeManager holds.
            streaming.biome_zoom = sunrise_core::biome::BiomeZoom::new(spawn.seed);
            self.client.streaming = streaming;
            level.world.day_time = self.common.world.day_time;
            self.install_client_world(level.world);
            if let Some(remote) = self.client.remote.as_ref() {
                self.common
                    .world
                    .chunks
                    .track_connection_groups(remote.group_count.clone());
            }
            // The border and weather belong to this ClientLevel, not the connection.
            self.common.border = Default::default();
            // A same-level respawn keeps the level; a replacement retires the
            // old handles and adopts the epoch already given to Load listeners.
            self.native_lifecycle_open_at(level.handle.epoch());
            self.native_lifecycle_world_ready();
            self.common.mobs.clear();
            self.clear_remote_objects();
            self.clear_remote_projectiles();
            self.common.dropped.clear();
            self.clear_client_level_particles();
            if let Some(r) = self.client.remote.as_mut() {
                r.heard_level = Some(spawn.dimension.clone());
                r.entities.clear();
                r.synced_attributes.clear();
                r.passenger_vehicles.clear();
                r.lerps.clear();
                r.general.clear();
                r.cart_lerps.clear();
                r.stepped.clear();
                r.head_lerps.clear();
                r.pending_items.clear();
                // The entity ids are the old world's; the new dimension's
                // player entities re-map to their profiles on AddEntity.
                r.level_players.clear();
                r.level_uuids.clear();
                r.level_entity_uuids.clear();
                r.level_player_infos.clear();
                r.level_player_names.clear();
                r.level_player_textures.clear();
                // A new ClientLevel: a new chunk storage, centred on 0, 0.
                r.chunk_center = (0, 0);
                r.awaiting_viewed_chunk_center = false;
            }
            self.clear_lod();
            self.client.pending_uploads.0.clear();
            if let Some(renderer) = self.client.renderer.0.as_mut() {
                renderer.clear_sections(self.common.world.layout());
            }
            // Minecraft.setLevel -> updateLevelInEngines, after the old-level unload.
            self.stop_sounds_for_new_level();
        }
        self.client.audio.0.set_world(&spawn.dimension);
        self.client.streaming.relight_resent = relight_resent;
        let has_sky = self
            .client
            .remote
            .as_ref()
            .is_none_or(|r| r.dimension.has_skylight);
        self.client.streaming.sky_light = has_sky;
        self.client.streaming.nether_shading = self.nether_lighting();
        let player = self.common.player.as_ref();
        self.client
            .particles
            .0
            .detach_tracking_target(TrackingTarget::LocalPlayer, || {
                player.map(player_tracking_body)
            });
        if new_logical_player {
            self.drop_remote_player();
            // handleLogin / handleRespawn: ClientLevel.addEntity admits the local player
            // now, before the position packet creates its motion body.
            self.common.resident_entities.local_player.renew();
        } else {
            self.retain_remote_player_on_login();
        }
        // handleLogin / handleRespawn: LocalPlayer.setPortalCooldown, so a
        // player who arrives standing in a portal gets no overlay from it.
        self.common.portal_player.cooldown = spawn.portal_cooldown;
        if let Some(profile) = self.client.remote.as_ref().map(|remote| remote.profile) {
            self.initialize_entity(Holder::Player(profile), "minecraft:player");
        }
        self.finish_remote_level();
        if let Some(registration) = transition_factory
            && registration.factory_owner.is_enabled()
            && let Ok(presentation) = registration.factory_owner.call(
                "dimension transition screen",
                "dimension_transition_screen_factory",
                || {
                    sunrise_client_render::dimension_transition::ActiveTransitionScreen::new(
                        registration.draw_owner.clone(),
                        registration.factory.create(self, reason.into()),
                    )
                },
            )
            && let Some(r) = self.client.remote.as_mut()
        {
            r.level_load.set_transition_screen(presentation);
        }
    }
    pub(in crate::game::client_side) fn finish_remote_level(&mut self) {
        self.client_refresh_infiniburn();
        self.apply_settings();
    }
    /// A joined client's trust keys, fetched off the game thread. Minecraft makes its
    /// services once per run; a new set ends the skin verdicts made with the old one.
    fn fresh_services_keys(&mut self) -> Arc<std::sync::Mutex<ServicesKeys>> {
        self.client.skin_downloads.0.borrow_mut().forget_unpacked();
        ServicesKeys::fetching()
    }
    /// Prepare a client session and remote state to receive packets from a studio replay.
    /// A replayed or viewed world checks its players' signed properties as a joined
    /// client does.
    pub fn init_remote_for_replay(&mut self) -> Result<()> {
        // Resolve fallible resources before mutating state or clearing the title screen.
        let root = self.common.options.resources.clone();
        let builtin = builtin_registries(&root).ok_or("Resources are incomplete")?;
        let professions = self.client_professions()?;
        // Minecraft builds its `Services` once per client: a replay that opens a new
        // session on every seek keeps the keys the session it replaces is fetching or
        // has fetched.
        let services_keys = match &self.client.remote {
            Some(remote) => remote.services_keys.clone(),
            None => self.fresh_services_keys(),
        };
        self.reset_client_item_use();
        self.client.realm_session = false;
        self.client.lan_session = false;
        self.reset_tutorial();
        self.client.perf.world_opening();
        self.common.world_biomes = None;
        self.common.world_biomes_base = None;
        let lang = Arc::new(sunrise_sim::lang_load::load_language(&root));
        let info = self
            .client
            .settings
            .client_information(client_view_distance(self.client.settings.render_distance));
        let auth = self.auth();
        let name = auth.as_ref().map_or("ReplayPlayer", |a| a.name.as_str());
        let user = auth.as_ref().map_or_else(
            || {
                (
                    name.to_owned(),
                    sunrise_protocol::crypto::offline_uuid(name),
                )
            },
            |a| (a.name.clone(), a.uuid),
        );
        self.server_packs_connect(None, None, user);
        let session = Session::replay(name);
        let player_type = super::super::perspective::player_type(&root);
        let mut streaming = MeshStreamer::new(
            Arc::new(NoTerrain),
            self.worker_context(),
            self.common.res.threads(),
        );
        streaming.remote = true;
        streaming.radius = self.client.settings.render_distance;
        self.client.streaming = streaming;
        self.reset_client_world(0);
        let group_count = sunrise_level::chunk_map::ConnectionGroupCount::default();
        self.common
            .world
            .chunks
            .track_connection_groups(group_count.clone());
        self.common.inventory = vec![None; 36];
        self.common.offhand = None;
        self.common.armor = [None, None, None, None];
        self.common.equipment = Default::default();
        self.common.craft_grid = vec![None; 9];
        self.common.craft_size = 2;
        self.remove_local_player_body();
        self.common.spawn = None;
        self.common.level.world_spawn_dimension = None;
        self.common.mobs.clear();
        self.clear_remote_objects();
        self.clear_remote_projectiles();
        self.common.creatures.projectiles.clear();
        self.common.dropped.clear();
        self.clear_client_level_particles();
        self.common.orbs.clear();
        self.common.falling.clear();
        self.set_client_mining(None);
        self.common.container = None;
        self.common.carried = None;
        self.common.using = None;

        self.common.experience = Experience {
            level: 0,
            progress: 0.0,
            total: 0,
        };
        self.common.ui.registries = Default::default();
        self.build_registries();
        self.common.ui.registries.permission = 0;
        self.client.chat_ui.chat.clear();
        self.set_screen(Screen::Playing);
        self.client.title.0 = None;
        self.common.dim = Dim::Overworld;
        self.clear_lod();
        self.client.pending_uploads.0.clear();
        if let Some(r) = self.client.renderer.0.as_mut() {
            r.clear_sections(self.common.world.layout());
        }
        self.apply_settings();
        self.clear_waypoints();
        self.client.boss_overlay.reset();

        self.client.native_client_commands.pending_commands.clear();
        self.client.native_client_commands.pending_chats.clear();
        let chat_tree = self.chat_tree_for(None);

        self.common.ui.registries.selector_tokens.clear();
        self.client.remote = Some(Remote {
            selector_metadata_supported: true,
            professions,
            display_address: Remote::display_address_for(&session.address),
            session,
            brand: "sunrise-studio".into(),
            datapack_vanilla_values: None,
            particle_packet_random: sunrise_level::random_source::create(),
            recipe_viewer: Default::default(),
            features: ["minecraft:vanilla".to_string()].into(),
            quiet_commands: false,
            custom_books: false,
            attachments: HashMap::new(),
            colliders: HashMap::new(),
            collider_shove_start: None,
            carry_age: Default::default(),
            far_terrain: None,
            builtin,
            status: "Playing Studio Replay".into(),
            lang,
            registries: Arc::new(Dynamic::default()),
            variants: Arc::new(sunrise_sim::mobs::variants::VariantTables::default()),
            tags: None,
            enchantments: None,
            joined: false,
            reconfiguring: None,
            logged_in: true,
            has_logical_player: false,
            logged_out: false,
            local_entity_silent: false,
            entity_id: 0,
            camera: None,
            detached_camera: None,
            dimension: DimensionType::vanilla("minecraft:overworld"),
            level: "minecraft:overworld".into(),
            level_key: mod_api::DimensionKey::OVERWORLD,
            level_facts: None,
            level_environment: None,
            level_pack_environment: None,
            synced_type: None,
            level_environments: Default::default(),
            facts_cache: Default::default(),
            added_levels: Default::default(),
            viewed: Default::default(),
            hashed_seed: 0,
            sea_level: 63,
            chunk_format: Arc::new(ChunkFormat::new(
                &DimensionType::vanilla("minecraft:overworld"),
                Vec::new(),
                7,
            )),
            decoder: ChunkDecoder::new(),
            view_distance: self.client.settings.render_distance,
            simulation_distance: self.client.settings.simulation_distance,
            sequence: 0,
            last_teleport_sequence: -1,
            position_sender: Default::default(),
            last_input: 0,
            was_sprinting: false,
            was_shared_flags: 0,
            was_living_flags: 0,
            last_recorded_pose: None,
            carried: std::cell::Cell::new(None),
            abilities: sunrise_sim::lan::abilities::PlayerAbilities::default(),
            respawn_velocity: None,
            loaded_sent: false,
            heard_level: None,
            datapacks: Default::default(),
            constructing_level: None,
            level_load: sunrise_client_render::level_loading::LevelLoadTracker::new(0),
            sent_info: info,
            group_count,
            predictions: HashMap::new(),
            menu: ClientMenu::default(),
            ticked_before_player: std::collections::HashSet::new(),
            craft_result: None,
            menu_title: None,
            menu_title_snapshot: None,
            state_id: 0,
            inventory_state_id: 0,
            inventory_carried: None,
            inventory_craft_grid: Default::default(),
            inventory_craft_result: None,
            use_target: None,
            cauldron_entries: None,
            cauldron_receipt: 0,
            pending_cauldrons: Default::default(),
            menu_extra: None,
            entities: HashMap::new(),
            tick_fluids: Vec::new(),
            tick_rain: Vec::new(),
            tick_standings: Vec::new(),
            tick_water_fx: Vec::new(),
            passenger_vehicles: HashMap::new(),
            lerps: HashMap::new(),
            general: Default::default(),
            cart_lerps: HashMap::new(),
            stepped: HashMap::new(),
            head_lerps: HashMap::new(),
            ghast_dismount: None,
            first_passenger: false,
            own_tick_count: 0,
            passenger_index: 0,
            pending_items: HashMap::new(),
            health: (20.0, 20, 5.0),
            flash_on_set_health: false,
            game_mode: 0,
            chat: Vec::new(),
            chat_received: 0,
            chat_queue: VecDeque::new(),
            signatures: Default::default(),
            chat_previous: None,
            next_chat_index: 0,
            enforces_secure_chat: false,
            services_keys,
            chat_sessions: HashMap::new(),
            chat_validators: HashMap::new(),
            joining_narrated: None,
            chat_tree,
            server_chat_tree: None,
            secure_chat: Default::default(),
            custom_completions: Default::default(),
            players: HashMap::new(),
            tab_players: HashMap::new(),
            tab_game_modes: HashMap::new(),
            own_player_info: std::cell::OnceCell::new(),
            skin_textures: HashMap::new(),
            own_profile: OwnProfile::Idle,
            skins: HashMap::new(),
            skins_revision: 0,
            published_skins: None,
            listed: Default::default(),
            online_mode: false,
            levels: Vec::new(),
            level_players: HashMap::new(),
            level_uuids: HashMap::new(),
            level_entity_uuids: HashMap::new(),
            level_player_infos: HashMap::new(),
            level_player_names: HashMap::new(),
            level_player_textures: HashMap::new(),
            glowing: Default::default(),
            own_invisible: false,
            own_on_fire: false,
            own_arrows: 0,
            own_stingers: 0,
            own_shoulders: [None; 2],
            profile: 0,
            profile_name: String::new(),
            destroying: None,
            destroying_item: None,
            is_destroying: false,
            interaction_swing: [true; 2],
            destroy_delay: 0,
            swap_answer: [false; 2],
            stop_mining: false,
            flat: false,
            death_message: None,
            show_death_screen: true,
            reduced_debug_info: false,
            do_limited_crafting: false,
            player_type,
            synced: vec![None; CREATIVE_SYNCED],
            decode_errors: 0,
            clocks: HashMap::new(),
            default_clock: None,
            overworld_clock: None,
            clock_game_time: 0,
            chunk_center: (0, 0),
            awaiting_viewed_chunk_center: false,
            chunk_light: RemoteLight::default(),
            backlog: VecDeque::new(),
            backlog_since: None,
            server_links: Vec::new(),
            debug_query: Default::default(),
            synced_attributes: HashMap::new(),
        });
        self.set_local_mode_with_previous(sunrise_command::args::GameMode::Survival, None);
        self.common.clock = sunrise_sim::clock::FixedStep::client();
        self.common.adv = Default::default();
        self.common.stats = Default::default();
        Ok(())
    }
    pub(in crate::game::client_side) fn install_remote_session(
        &mut self,
        opening: RemoteOpening,
        lan: bool,
        realm: bool,
    ) -> Result<()> {
        // Resolve fallible resources before mutating state or hiding the title screen.
        let professions = self.client_professions()?;
        let RemoteOpening {
            session,
            builtin,
            lang,
            info,
            user,
            local,
            list_name,
        } = opening;
        self.reset_client_item_use();
        self.client.realm_session = realm;
        self.client.lan_session = lan;
        self.reset_tutorial();
        self.client.perf.world_opening();
        if !local {
            // A server's own biomes come over the connection, not from the last local world.
            self.common.world_biomes = None;
            self.common.world_biomes_base = None;
            mod_api::content::crash::set_detail(
                "Server",
                sunrise_client_lib::net::shown_address(&session.address),
            );
        }
        self.server_packs_connect(
            (!local).then_some(&session.address),
            list_name.as_deref(),
            user,
        );
        let root = self.common.options.resources.clone();
        let player_type = super::super::perspective::player_type(&root);
        let mut streaming = MeshStreamer::new(
            Arc::new(NoTerrain),
            self.worker_context(),
            self.common.res.threads(),
        );
        streaming.remote = true;
        streaming.radius = self.client.settings.render_distance;
        self.client.streaming = streaming;
        self.reset_client_world(0);
        let group_count = sunrise_level::chunk_map::ConnectionGroupCount::default();
        self.common
            .world
            .chunks
            .track_connection_groups(group_count.clone());
        self.common.inventory = vec![None; 36];
        self.common.offhand = None;
        self.common.armor = [None, None, None, None];
        self.common.equipment = Default::default();
        self.common.craft_grid = vec![None; 9];
        self.common.craft_size = 2;
        self.remove_local_player_body();
        self.common.spawn = None;
        self.common.level.world_spawn_dimension = None;
        self.common.mobs.clear();
        self.clear_remote_objects();
        self.clear_remote_projectiles();
        self.common.creatures.projectiles.clear();
        self.common.dropped.clear();
        self.clear_client_level_particles();
        self.common.falling.clear();
        self.set_client_mining(None);
        self.common.container = None;
        self.common.carried = None;
        self.common.using = None;
        self.common.clicks.miss_time = 0;
        // A new LocalPlayer has no effects: the server sends what it has
        // (ClientboundUpdateMobEffectPacket on placement).
        self.common.potions = Default::default();
        self.common.experience = Experience {
            level: 0,
            progress: 0.0,
            total: 0,
        };
        // The connected client never opens a server world (single player also
        // joins here), so load the registries its argument parsers and local
        // suggestions need before receiving the server's command tree.
        self.common.ui.registries = Default::default();
        self.build_registries();
        self.common.ui.registries.permission = 0;
        self.client.chat_ui.chat.clear();
        self.set_screen(Screen::Playing);
        self.hide_title();
        self.common.dim = Dim::Overworld;
        self.clear_lod();
        self.client.pending_uploads.0.clear();
        if let Some(r) = self.client.renderer.0.as_mut() {
            r.clear_sections(self.common.world.layout());
        }
        self.apply_settings();
        self.clear_waypoints();
        self.client.boss_overlay.reset();
        // The chat's tree until the server sends its own, or forever when it
        // sends none: the game's own, plus the commands native mods own.
        self.client.native_client_commands.pending_commands.clear();
        self.client.native_client_commands.pending_chats.clear();
        let chat_tree = self.chat_tree_for(None);
        let services_keys = self.fresh_services_keys();
        self.common.ui.registries.selector_tokens.clear();
        self.client.remote = Some(Remote {
            selector_metadata_supported: true,
            professions,
            display_address: Remote::display_address_for(&session.address),
            session,
            brand: "unknown".into(),
            datapack_vanilla_values: None,
            particle_packet_random: sunrise_level::random_source::create(),
            recipe_viewer: Default::default(),
            features: ["minecraft:vanilla".to_string()].into(),
            quiet_commands: false,
            custom_books: false,
            logged_out: false,
            attachments: HashMap::new(),
            colliders: HashMap::new(),
            collider_shove_start: None,
            carry_age: Default::default(),
            far_terrain: None,
            builtin,
            status: lang
                .get("connect.connecting")
                .cloned()
                .unwrap_or_else(|| "Connecting to the server...".into()),
            lang,
            registries: Arc::new(Dynamic::default()),
            variants: Arc::new(sunrise_sim::mobs::variants::VariantTables::default()),
            tags: None,
            enchantments: None,
            joined: false,
            reconfiguring: None,
            logged_in: false,
            has_logical_player: false,
            entity_id: 0,
            local_entity_silent: false,
            camera: None,
            detached_camera: None,
            dimension: DimensionType::vanilla("minecraft:overworld"),
            level: "minecraft:overworld".into(),
            level_key: mod_api::DimensionKey::OVERWORLD,
            level_facts: None,
            level_environment: None,
            level_pack_environment: None,
            synced_type: None,
            level_environments: Default::default(),
            facts_cache: Default::default(),
            added_levels: Default::default(),
            viewed: Default::default(),
            hashed_seed: 0,
            sea_level: 63,
            chunk_format: Arc::new(ChunkFormat::new(
                &DimensionType::vanilla("minecraft:overworld"),
                Vec::new(),
                7,
            )),
            decoder: ChunkDecoder::new(),
            view_distance: self.client.settings.render_distance,
            simulation_distance: self.client.settings.simulation_distance,
            sequence: 0,
            last_teleport_sequence: -1,
            position_sender: Default::default(),
            last_input: 0,
            was_sprinting: false,
            carried: std::cell::Cell::new(None),
            was_shared_flags: 0,
            was_living_flags: 0,
            last_recorded_pose: None,
            abilities: sunrise_sim::lan::abilities::PlayerAbilities::default(),
            respawn_velocity: None,
            loaded_sent: false,
            heard_level: None,
            datapacks: Default::default(),
            constructing_level: None,
            level_load: sunrise_client_render::level_loading::LevelLoadTracker::new(0),
            sent_info: info,
            group_count,
            predictions: HashMap::new(),
            menu: ClientMenu::default(),
            ticked_before_player: std::collections::HashSet::new(),
            craft_result: None,
            menu_title: None,
            menu_title_snapshot: None,
            state_id: 0,
            inventory_state_id: 0,
            inventory_carried: None,
            inventory_craft_grid: Default::default(),
            inventory_craft_result: None,
            use_target: None,
            cauldron_entries: None,
            cauldron_receipt: 0,
            pending_cauldrons: Default::default(),
            menu_extra: None,
            entities: HashMap::new(),
            tick_fluids: Vec::new(),
            tick_rain: Vec::new(),
            tick_standings: Vec::new(),
            tick_water_fx: Vec::new(),
            passenger_vehicles: HashMap::new(),
            lerps: HashMap::new(),
            general: Default::default(),
            cart_lerps: HashMap::new(),
            stepped: HashMap::new(),
            head_lerps: HashMap::new(),
            ghast_dismount: None,
            first_passenger: false,
            own_tick_count: 0,
            passenger_index: 0,
            pending_items: HashMap::new(),
            health: (20.0, 20, 5.0),
            flash_on_set_health: false,
            game_mode: 0,
            chat: Vec::new(),
            chat_received: 0,
            chat_queue: VecDeque::new(),
            signatures: Default::default(),
            chat_previous: None,
            next_chat_index: 0,
            enforces_secure_chat: false,
            services_keys,
            chat_sessions: HashMap::new(),
            chat_validators: HashMap::new(),
            joining_narrated: None,
            chat_tree,
            server_chat_tree: None,
            secure_chat: Default::default(),
            custom_completions: Default::default(),
            players: HashMap::new(),
            tab_players: HashMap::new(),
            tab_game_modes: HashMap::new(),
            own_player_info: std::cell::OnceCell::new(),
            skin_textures: HashMap::new(),
            own_profile: OwnProfile::Idle,
            skins: HashMap::new(),
            skins_revision: 0,
            published_skins: None,
            listed: Default::default(),
            online_mode: false,
            levels: Vec::new(),
            level_players: HashMap::new(),
            level_uuids: HashMap::new(),
            level_entity_uuids: HashMap::new(),
            level_player_infos: HashMap::new(),
            level_player_names: HashMap::new(),
            level_player_textures: HashMap::new(),
            glowing: Default::default(),
            own_invisible: false,
            own_on_fire: false,
            own_arrows: 0,
            own_stingers: 0,
            own_shoulders: [None; 2],
            profile: 0,
            profile_name: String::new(),
            destroying: None,
            destroying_item: None,
            is_destroying: false,
            interaction_swing: [true; 2],
            destroy_delay: 0,
            swap_answer: [false; 2],
            stop_mining: false,
            flat: false,
            death_message: None,
            show_death_screen: true,
            reduced_debug_info: false,
            do_limited_crafting: false,
            player_type,
            synced: vec![None; CREATIVE_SYNCED],
            decode_errors: 0,
            clocks: HashMap::new(),
            default_clock: None,
            overworld_clock: None,
            clock_game_time: 0,
            chunk_center: (0, 0),
            awaiting_viewed_chunk_center: false,
            chunk_light: RemoteLight::default(),
            backlog: VecDeque::new(),
            backlog_since: None,
            server_links: Vec::new(),
            debug_query: Default::default(),
            synced_attributes: HashMap::new(),
        });
        self.set_local_mode_with_previous(sunrise_command::args::GameMode::Survival, None);
        // The level's TickRateManager follows the server's ticking packets.
        self.common.clock = sunrise_sim::clock::FixedStep::client();
        // The player's advancements and statistics are the server's.
        self.common.adv = Default::default();
        self.common.stats = Default::default();
        Ok(())
    }
    pub(in crate::game::client_side) fn install_remote_transfer(
        &mut self,
        opening: RemoteOpening,
    ) -> Result<()> {
        self.connection_left(sunrise_hooks::connection::Left::Transferred);
        self.leave_server(None)?;
        self.connection_starts(&opening.session.address, &opening.session.name, false);
        self.install_remote_session(opening, false, false)
    }
    /// Disconnect and return to the multiplayer screen (DisconnectedScreen).
    pub fn leave_server(&mut self, reason: Option<Text>) -> Result<()> {
        if reason.is_none()
            && self
                .client
                .integrated
                .as_ref()
                .is_some_and(|s| !s.stopped())
        {
            return self.begin_closing_save(false);
        }
        self.close_server_session(reason, |_| {})
    }
    pub(in crate::game::client_side) fn open_remote_session(
        &mut self,
        address: &str,
        name: &str,
        local: Option<sunrise_protocol::connection::Pipe>,
        cookies: Option<sunrise_client_lib::net::Cookies>,
        lan: bool,
        announce: bool,
        list_name: Option<&str>,
    ) -> Result<RemoteOpening> {
        let root = &self.common.options.resources;
        let item_data = Arc::clone(&self.common.res.core.item_data);
        let builtin = builtin_registries(root).ok_or("Resources are incomplete: registries.json is missing or invalid. Click \"Prepare again\" in the launcher (or run sunrise-prepare), then try again.")?;
        let lang = Arc::new(sunrise_sim::lang_load::load_language(root));
        let info = self
            .client
            .settings
            .client_information(client_view_distance(self.client.settings.render_distance));
        // A signed-in account always plays under its own name.
        let auth = self.auth();
        let name = auth.as_ref().map_or(name, |a| a.name.as_str());
        let user = (
            name.to_owned(),
            auth.as_ref()
                .map_or_else(|| sunrise_protocol::crypto::offline_uuid(name), |a| a.uuid),
        );
        let is_local = local.is_some();
        if announce && !is_local {
            self.connection_starts(address, name, lan);
        }
        let session = match (local, cookies) {
            (Some(pipe), _) => Session::connect_local(
                pipe,
                name,
                auth.clone(),
                info.clone(),
                lang.clone(),
                item_data,
                Arc::clone(&self.client.client_reports.0),
            ),
            (None, Some(cookies)) => Session::connect_transferred(
                address,
                name,
                auth.clone(),
                info.clone(),
                lang.clone(),
                cookies,
                item_data,
                Arc::clone(&self.client.client_reports.0),
            ),
            (None, None) => {
                let connect = if lan {
                    Session::connect_lan
                } else {
                    Session::connect
                };
                connect(
                    address,
                    name,
                    auth.clone(),
                    info.clone(),
                    lang.clone(),
                    item_data,
                    Arc::clone(&self.client.client_reports.0),
                )
            }
        };
        Ok(RemoteOpening {
            session,
            builtin,
            lang,
            info,
            user,
            local: is_local,
            list_name: list_name.map(str::to_owned),
        })
    }
    /// Refuse invalid server data with the normal connection-error screen.
    pub(in crate::game) fn refuse_remote_input(
        &mut self,
        error: impl std::fmt::Display,
    ) -> Result<()> {
        log::warn!("Sunrise: {error}");
        self.leave_server(Some(Text::translate(
            "disconnect.genericReason",
            &[Text::literal(error.to_string())],
        )))
    }
    /// `ClientPacketListener.handleLogin` when `Minecraft.player != null`: retain the live player, run `resetPos`, rebind residency, and reset input.
    pub(in crate::game::client_side) fn retain_remote_player_on_login(&mut self) {
        self.side_reset_client_item_use();
        self.common.hook.discard();
        if let Some(r) = self.client.remote.as_mut() {
            r.respawn_velocity = None;
        }
        let retained_max_health = self.common.player.as_mut().map(|player| {
            let _ = player.set_pose(sunrise_sim::pose::Pose::Standing);
            player.velocity = Vec3::ZERO;
            player.pitch = 0.0;
            let max_health = player
                .attribute("minecraft:max_health")
                .ok()
                .map(|v| v as f32);
            if let Some(max) = max_health {
                let _ = player.set_health(max, false);
            }
            max_health
        });
        if let Some(max_health) = retained_max_health {
            self.common.pitch = 0.0;
            if let Some(max) = max_health
                && let Some(r) = self.client.remote.as_mut()
            {
                r.health.0 = max;
            }
            self.client.chat_ui.death_time = 0;
            self.input_player_replaced();
        }
        self.bind_local_player_residency();
    }
    /// `handleTransfer`: close the old connection and start the new one now.
    /// Keep the old listener installed for the remaining handlers of this bundle.
    pub(in crate::game::client_side) fn start_remote_transfer(
        &mut self,
        transfer: Transfer,
    ) -> Result<Option<RemoteOpening>> {
        let Some(remote) = self.client.remote.as_ref() else {
            return Ok(None);
        };
        remote.session.close_after_pending();
        if remote.session.address == "local" {
            return Ok(None);
        }
        let address = format!("{}:{}", transfer.host, transfer.port);
        let name = remote.session.name.clone();
        let cookies = remote.session.cookies.clone();
        self.open_remote_session(&address, &name, None, Some(cookies), false, false, None)
            .map(Some)
    }
    /// The environment attributes of the level the player is in, from its synchronized dimension
    /// type, as `EnvironmentAttributeSystem.addDimensionLayer` takes `level.dimensionType()`'s
    /// attributes (and `addDynamicLayers` its timelines): the type's attributes and timelines over
    /// the client's own environment of that type, for gameplay, and over the packs' Polytone stack
    /// of that type, whose modifiers then apply over them, for the frame. A built-in level is
    /// included, so a data pack's Overworld reaches the frame; the base is the client's
    /// environment of the same type, so the Nether and the End keep their own file's layers (no
    /// weather) instead of the Overworld's. Only the three vanilla type IDs have a pack stack: a
    /// mod's type is drawn without the packs' layers. Run again when the packs change.
    ///
    /// Building one reads every biome file, so both are kept by type name with the JSON and the
    /// packs' stacks they were built from (`LevelEnvironments`): entering a level of an unchanged
    /// type again (a respawn, a travel back) reuses them as kept, without cloning the JSON, and a
    /// pack reload, which makes new stacks, builds only the frame's again, in the same entry.
    pub(in crate::game) fn build_level_environments(&mut self) {
        let stacks = self.client.client_pack.environments.as_ref();
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        let Some((name, json)) = &r.synced_type else {
            r.level_environment = None;
            r.level_pack_environment = None;
            return;
        };
        if r.level_environments
            .get(name)
            .is_some_and(|e| e.json == *json)
        {
            let stale = r
                .level_environments
                .get(name)
                .is_some_and(|e| !same_stacks(e.stacks.as_ref(), stacks));
            let rebuilt =
                stale.then(|| pack_environment(name, json, stacks, &r.synced_timelines()));
            if let Some(e) = r.level_environments.get_mut(name) {
                if let Some(pack) = rebuilt {
                    e.pack = pack;
                    e.stacks = stacks.map(Arc::downgrade);
                }
                r.level_environment = e.own.clone();
                r.level_pack_environment = e.pack.clone();
            }
            return;
        }
        let timelines = r.synced_timelines();
        let own = Environment::with_type(self.common.res.environment_of(name), json, &timelines)
            .map(Arc::new);
        let pack = pack_environment(name, json, stacks, &timelines);
        r.level_environments.insert(
            name.clone(),
            LevelEnvironments {
                json: json.clone(),
                own: own.clone(),
                stacks: stacks.map(Arc::downgrade),
                pack: pack.clone(),
            },
        );
        r.level_environment = own;
        r.level_pack_environment = pack;
    }
}

use sunrise_gameplay_common::game::command_registries::CommandRegistriesGameExt1 as _;
use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;

use sunrise_gameplay_common::game::side_ops::player::PlayerSideGameExt as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::entity_initialization_common::EntityInitializationCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::build_creative_mode_tab_contents::GameHooks as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
