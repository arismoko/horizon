use super::*;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `C::AwardStats`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_award_stats_packet(&mut self, p: play::AwardStats) -> Result<()> {
        // handleAwardStats: StatsCounter.setValue for each.
        let Some(r) = self.client.remote.as_ref() else {
            return Ok(());
        };
        let mut known = Vec::new();
        for (ty, value, count) in p.stats {
            let Some(ty) = r
                .builtin
                .get("minecraft:stat_type")
                .and_then(|t| t.name(ty))
            else {
                continue;
            };
            let values = match ty {
                "minecraft:mined" => "minecraft:block",
                "minecraft:killed" | "minecraft:killed_by" => "minecraft:entity_type",
                "minecraft:custom" => "minecraft:custom_stat",
                _ => "minecraft:item",
            };
            if let Some(key) = r.builtin.get(values).and_then(|v| v.name(value)) {
                known.push((ty.to_owned(), key.to_owned(), count));
            }
        }
        for (ty, key, count) in known {
            self.common
                .stats
                .values
                .entry(ty)
                .or_default()
                .insert(key, count);
        }
        Ok(())
    }
    /// `C::ClearTitles`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game::client_side) fn apply_clear_titles_packet(&mut self, p: play::Bool1) {
        self.client.titles.clear();
        if p.value {
            self.client.titles.reset_times();
        }
    }
    /// `C::Commands`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_commands_packet(&mut self, p: play::Commands) {
        let tree =
            sunrise_command::network::from_packet(&p, sunrise_command::vanilla::dispatcher());
        // The chat's tree follows the server's now; a mod's commands
        // go into it again (`chat_tree_for`).
        let chat_tree = self.chat_tree_for(Some(&tree));
        if let Some(r) = self.client.remote.as_mut() {
            r.chat_tree = chat_tree;
            r.server_chat_tree = Some(tree);
        }
    }
    /// `ClientPacketListener.handleContainerContent` initializes only the addressed menu.
    pub(in crate::game) fn apply_container_set_content_packet(
        &mut self,
        p: play::ContainerSetContent,
    ) {
        let open = self
            .client
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map_or(0, |m| m.0);
        if p.container == 0 || p.container == open {
            self.remote_menu_state(p.container, p.state);
            for (slot, item) in p.items.iter().enumerate() {
                let stack = self.to_stack(item);
                self.set_menu_slot(p.container, slot as i32, stack);
            }
            let carried = self.to_stack(&p.carried);
            if p.container == open {
                self.common.carried = carried;
            } else if let Some(r) = self.client.remote.as_mut() {
                r.inventory_carried = carried;
            }
            self.remote_synced();
        }
    }
    /// `C::ContainerSetData`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_container_set_data_packet(
        &mut self,
        p: play::ContainerSetData,
    ) -> Result<()> {
        if self.open_menu_name(p.container) == "minecraft:lectern" {
            self.remote_lectern_page(i32::from(p.value));
            return Ok(());
        }
        if self.remote_station_open(p.container) {
            self.remote_station_data(usize::try_from(p.id).unwrap_or(usize::MAX), p.value);
            return Ok(());
        }
        if usize::try_from(p.id)
            .is_ok_and(|index| self.native_screen_data(p.container, index, i32::from(p.value)))
        {
            return Ok(());
        }
        let v = i32::from(p.value);
        match self
            .common
            .container
            .and_then(|key| self.common.world.block_entities.get_mut(&key))
        {
            Some(BlockEntity::Furnace(f)) => match p.id {
                0 => f.lit_remaining = v,
                1 => f.lit_total = v,
                2 => f.cooking_timer = v,
                3 => f.cooking_total = v,
                _ => {}
            },
            // CrafterBlockEntity.containerData.
            Some(BlockEntity::Crafter(c)) => match usize::try_from(p.id) {
                Ok(k @ 0..9) => c.disabled[k] = v == 1,
                Ok(9) => c.triggered = v,
                _ => {}
            },
            // BrewingStandBlockEntity.dataAccess.
            Some(BlockEntity::Brewing(b)) => match p.id {
                0 => b.brew_time = v,
                1 => b.fuel = v,
                2 => b.total_brew_time = v,
                3 => b.total_fuel = v,
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }
    /// `C::ContainerSetSlot`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game::client_side) fn apply_container_set_slot_packet(
        &mut self,
        p: play::ContainerSetSlot,
    ) {
        self.remote_menu_state(p.container, p.state);
        let stack = self.to_stack(&p.item);
        self.tutorial_get_item(stack.as_ref());
        // ClientPacketListener.handleContainerSetSlot: a hotbar slot that
        // gains items squeezes for 5 ticks.
        if p.container == 0
            && (36..=45).contains(&p.slot)
            && let Some(new) = &stack
        {
            let (pop, old) = match p.slot {
                45 => (9, self.common.offhand.as_ref()),
                s => (
                    s as usize - 36,
                    self.common.inventory[s as usize - 36].as_ref(),
                ),
            };
            if old.is_none_or(|old| old.count < new.count) {
                self.client.hotbar_pop[pop] = 5;
            }
        }
        let before = self.creative_stacks();
        self.set_menu_slot(p.container, i32::from(p.slot), stack);
        self.remote_synced_since(&before);
    }
    /// `C::DisguisedChat`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_disguised_chat_packet(&mut self, p: play::DisguisedChat) {
        self.enqueue_chat(QueuedChat {
            signature: None,
            sender: 0,
            kind: QueuedChatKind::Disguised {
                chat_type: p.chat_type,
                message: p.message,
            },
        });
    }
    /// `C::Login`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game::client_side) fn apply_login_packet(&mut self, p: play::Login) {
        // A client's parts, from the tags it plays with.
        if let Err(e) = self.resolve_parts() {
            log::warn!("Sunrise: {e}");
        }
        // Configuration's tags, over the block ids the parts just bound.
        self.install_remote_tags();
        self.refresh_face_visibility_tags();
        self.refresh_render_visibility();
        self.client.test_highlights.clear();
        // handleLogin: `debugSubscriber.clear()`.
        self.client.debug_charts.clear_subscriptions();
        self.client.debug_density.borrow_mut().clear();
        self.forget_synced_recipes();
        self.common.level.hardcore = p.hardcore;
        // handleLogin: startWaitingForNewLevel with Reason.OTHER, which
        // replaces ServerReconfigScreen.
        if let Some(r) = self.client.remote.as_mut() {
            r.reconfiguring = None;
            r.clear_attachments();
            r.viewed.clear();
            // A new ClientPacketListener owns a fresh clock manager.
            r.clocks.clear();
            r.clock_game_time = 0;
            self.common.world.game_time = 0;
        }
        // handleLogin makes a new MultiPlayerGameMode (carried index 0) and a
        // new LocalPlayer (slot 0, no effects, riding nothing: the server
        // pairs a vehicle again with ClientboundSetPassengersPacket).
        self.common.selected = 0;
        self.player_score = 0;
        self.common.potions.effects.clear();
        // handleLogin creates a fresh LocalPlayer only when none exists yet.
        if self.client.remote.as_ref().is_none_or(|r| !r.logged_in) {
            self.client.post_effects.clear();
        }
        self.common.riding = None;
        if let Some(player) = self.common.player.as_mut() {
            player.entity_id = p.player_id;
        }
        self.client
            .packet_holds
            .borrow_mut()
            .clear_cauldron_continuations();
        if let Some(r) = self.client.remote.as_mut() {
            r.carried.set(Some(0));
            r.pending_cauldrons.get_mut().clear();
            // handleLogin makes a new ClientLevel: its predictions count from 0.
            r.sequence = 0;
            r.own_invisible = false;
            r.own_on_fire = false;
            r.glowing.remove(&p.player_id);
            r.online_mode = p.online_mode;
            r.levels.clone_from(&p.levels);
            r.enforces_secure_chat = p.enforces_secure_chat;
            r.entity_id = p.player_id;
            r.local_entity_silent = false;
            // handleLogin: the new LocalPlayer is the camera.
            r.camera = None;
            r.detached_camera = None;
            r.logged_in = true;
            r.view_distance = p.chunk_radius;
            r.simulation_distance = p.simulation_distance;
            r.show_death_screen = p.show_death_screen;
            r.reduced_debug_info = p.reduced_debug_info;
            r.do_limited_crafting = p.limited_crafting;
        }
        self.clear_spectated_entity_post_effect();
        self.bind_local_player_residency();
        self.refresh_debug_renderers();
        // LocalPlayer.setReducedDebugInfo always rebuilds, also on a second login.
        self.client.debug_entries.0.invalidate();
        self.client.streaming.radius =
            sunrise_gameplay_common::game::streaming_radius::streaming_radius_for(
                self.client.settings.render_distance,
                Some(p.chunk_radius),
            );

        // handleLogin: startWaitingForNewLevel(.., Reason.OTHER).
        self.enter_remote_dimension(
            &p.spawn,
            RemoteDimensionEntry::Login,
            sunrise_client::level_loading::Reason::Other,
        );
        // No ClientInformation here: the configuration phase sent it,
        // and ClientPacketListener.broadcastClientInformation sends it
        // again only when an option changes it.
        self.remote_chat_login();
        if let Some(remote) = &self.client.remote {
            self.native_client_player_logging_in(p.player_id, remote.profile);
        }
    }
    /// `C::PlayerAbilities`: mutates `Remote::abilities` in arrival order, preserving `may_build`.
    pub(in crate::game) fn apply_player_abilities_packet(&mut self, p: play::PlayerAbilities) {
        self.common.creative = p.flags & 8 != 0;
        if let Some(r) = self.client.remote.as_mut() {
            r.abilities.invulnerable = p.flags & 1 != 0;
            r.abilities.flying = p.flags & 2 != 0;
            r.abilities.may_fly = p.flags & 4 != 0;
            r.abilities.instabuild = p.flags & 8 != 0;
            r.abilities.flying_speed = p.flying_speed;
            r.abilities.walking_speed = p.walking_speed;
        }
        self.apply_remote_abilities();
    }
    /// `C::PlayerChat`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_player_chat_packet(&mut self, p: play::PlayerChat) -> Result<()> {
        // SignedMessageBody.Packed.unpack: the last seen it names must
        // be in the cache; then the message joins it
        // (MessageSignatureCache.push).
        let Some(r) = self.client.remote.as_mut() else {
            return Ok(());
        };
        // handlePlayerChat: the global index counts every chat packet; one out of
        // order disconnects.
        let expected = r.next_chat_index;
        r.next_chat_index = r.next_chat_index.wrapping_add(1);
        if p.global_index != expected {
            log::error!(
                "Missing or out-of-order chat message from server, expected index {expected} but got {}",
                p.global_index
            );
            let reason = Text::translate("multiplayer.disconnect.bad_chat_index", &[]);
            return self.leave_server(Some(reason));
        }
        let last_seen: Option<Vec<Signature>> =
            p.last_seen.iter().map(|s| r.signatures.unpack(s)).collect();
        let Some(last_seen) = last_seen else {
            log::error!(
                "Message from player with ID {:032x} referenced unrecognized signature id",
                p.sender
            );
            let reason = Text::translate("multiplayer.disconnect.invalid_packet", &[]);
            return self.leave_server(Some(reason));
        };
        let signature = p
            .signature
            .as_deref()
            .and_then(sunrise_protocol::secure_chat::signature);
        r.signatures.push(&last_seen, signature.as_ref());
        // The sender's PlayerInfo: none is an error line; else their
        // `SignedMessageValidator` decides (`updateAndValidate`), and a message it
        // takes without a signature of its own shows as unsigned.
        if !r.tab_players.contains_key(&p.sender) {
            log::error!(
                "Received player chat packet for unknown player with ID: {:032x}",
                p.sender
            );
            self.handle_chat_message_error(p.sender, p.chat_type, signature);
            return Ok(());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let incoming = secure_chat::Incoming {
            sender: p.sender,
            index: p.index,
            signature: signature.as_ref(),
            body: secure_chat::Body {
                content: &p.content,
                timestamp: p.timestamp,
                salt: p.salt,
                last_seen: &last_seen,
            },
            unsigned_content: p.unsigned_content.as_ref(),
            filter: &p.filter,
        };
        let Some(validated) = r.validate_chat(&incoming, now) else {
            self.handle_chat_message_error(p.sender, p.chat_type, signature);
            return Ok(());
        };
        let signature = signature.filter(|_| validated == secure_chat::Validated::Signed);
        if p.filter.0 == 1 {
            // `FilterMask.FULLY_FILTERED` can never display or narrate; keep
            // FIFO ordering and its deletion/ack key without text payloads.
            self.enqueue_chat(QueuedChat {
                signature,
                sender: p.sender,
                kind: QueuedChatKind::Filtered,
            });
            return Ok(());
        }
        // ChatListener.handlePlayerChatMessage: Only Show Secure Chat shows the
        // signed content, never the server's replacement, so what the line shows is
        // picked here, at receipt, with the receipt time. The line is decorated, its
        // trust evaluated and shown when the queue accepts the message
        // (`showMessageToPlayer`): a component translates when it is read
        // (`TranslatableContents.decompose`), in the language of that moment.
        let only_show_secure = self.client.settings.file.bool("onlyShowSecureChat");
        let displayed = if only_show_secure {
            serde_json::Value::from(p.content.as_str())
        } else {
            p.unsigned_content.as_ref().map_or_else(
                || serde_json::Value::from(p.content.as_str()),
                sunrise_game_data::scoreboard::tag_to_json,
            )
        };
        let signed = signature.is_some();
        let listened = mod_api::__private::listened(mod_api::HookId::ClientChatReceived);
        let info = listened.then(|| Box::new(chat_received::player_chat_info(&p, signed)));
        let received_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        self.enqueue_chat(QueuedChat {
            signature,
            sender: p.sender,
            kind: QueuedChatKind::Player {
                chat_type: p.chat_type,
                signed,
                timestamp: p.timestamp,
                received_ms,
                only_show_secure,
                displayed,
                signed_content: p.content,
                unsigned_content: p.unsigned_content,
                filter: p.filter,
                info,
            },
        });
        Ok(())
    }
    /// `C::PlayerCombatKill`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game::client_side) fn apply_player_combat_kill_packet(
        &mut self,
        p: play::PlayerCombatKill,
    ) -> Result<()> {
        // The component resolves in the client's current language, as
        // the chat line does, not in the language the session joined
        // with (`Remote.lang` is the vanilla server-language table).
        let message = remote_text(&p.message);
        let me = self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == p.id);
        if let Some(r) = self.client.remote.as_mut() {
            r.death_message = Some(message);
        }
        if me {
            self.reset_client_item_use();
        }
        // ClientPacketListener.handlePlayerCombatKill: the death
        // screen, whatever health was last sent, or `respawn()` at
        // once when it is hidden.
        if me && self.client.screen != Screen::Dead && self.common.player.is_some() {
            if self
                .client
                .remote
                .as_ref()
                .is_none_or(|r| r.show_death_screen)
            {
                self.close_dialogs();
                self.set_screen(Screen::Dead);
                self.client.chat_ui.death_time = 0;
                self.client.chat_ui.death_confirm = None;
                self.capture(false);
            } else {
                self.respawn_player()?;
            }
        }
        Ok(())
    }
    /// `C::PlayerInfoRemove`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_player_info_remove_packet(&mut self, p: play::PlayerInfoRemove) {
        if let Some(r) = self.client.remote.as_mut() {
            for u in p.uuids {
                r.players.remove(&u);
                r.tab_players.remove(&u);
                r.tab_game_modes.remove(&u);
                r.chat_sessions.remove(&u);
                r.chat_validators.remove(&u);
                r.skin_textures.remove(&u);
                r.listed.remove(&u);
                r.attachments.remove(&Holder::Player(u));
            }
        }
    }
    /// `C::PlayerInfoUpdate`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_player_info_update_packet(&mut self, p: play::PlayerInfoUpdate) {
        if let Some(r) = self.client.remote.as_mut() {
            // handlePlayerInfoUpdate: new entries first, then every
            // action on every known entry.
            // INITIALIZE_CHAT: the session (or none) each entry carries; the
            // player's validator is made again when their chat is next read.
            if p.actions & 2 != 0 {
                for e in &p.entries {
                    r.chat_sessions.insert(e.uuid, e.chat_session.clone());
                    r.chat_validators.remove(&e.uuid);
                }
            }
            if p.actions & 1 != 0 {
                for e in &p.entries {
                    if let Some(name) = &e.name {
                        r.tab_players.entry(e.uuid).or_insert_with(|| {
                            sunrise_level::profile_lookup::TabPlayer::new(e.uuid, name.clone())
                        });
                        r.tab_game_modes
                            .entry(e.uuid)
                            .or_insert_with(|| Arc::new(std::sync::atomic::AtomicI32::new(0)));
                        if let Some(t) = e.properties.iter().find(|p| p.name == "textures") {
                            // Keep the raw property until the services keys settle.
                            r.skin_textures.entry(e.uuid).or_insert_with(|| {
                                Arc::new(PlayerTextures::new(e.uuid, t.clone()))
                            });
                        }
                    }
                }
            }
        }
        for e in p.entries {
            let mode_change = e.game_mode.and_then(|mode| {
                let r = self.client.remote.as_ref()?;
                let info = r.tab_players.get(&e.uuid)?;
                (info.game_mode != mode).then_some((info.game_mode, mode))
            });
            if let Some((current, mode)) = mode_change {
                // LocalPlayer.onGameModeChanged(SPECTATOR) clears vertical motion before PlayerInfo.setGameMode.
                if mode == 3
                    && self.client.remote.as_ref().is_some_and(|r| {
                        r.profile == e.uuid
                            && (self.common.player.is_some() || r.has_logical_player || r.logged_in)
                    })
                {
                    if let Some(player) = self.common.player.as_mut() {
                        player.velocity.y = 0.0;
                    }
                    if let Some(vel) = self
                        .client
                        .remote
                        .as_mut()
                        .and_then(|r| r.respawn_velocity.as_mut())
                    {
                        vel.y = 0.0;
                    }
                }
                if mod_api::__private::listened(mod_api::HookId::ClientPlayerChangeGameType) {
                    let entity_id = self.client.remote.as_ref().and_then(|r| {
                        if e.uuid == r.profile {
                            (self.common.player.is_some() || r.has_logical_player || r.logged_in)
                                .then_some(r.entity_id)
                        } else {
                            r.player_entity(e.uuid).map(|id| id as i32)
                        }
                    });
                    self.native_client_player_change_game_type(entity_id, e.uuid, current, mode);
                }
            }
            if let Some(r) = self.client.remote.as_mut() {
                if let Some(t) = r.tab_players.get_mut(&e.uuid) {
                    if let Some(mode) = e.game_mode {
                        t.game_mode = mode;
                        r.tab_game_modes
                            .entry(e.uuid)
                            .or_insert_with(|| Arc::new(std::sync::atomic::AtomicI32::new(mode)))
                            .store(mode, std::sync::atomic::Ordering::Relaxed);
                    }
                    if let Some(latency) = e.latency {
                        t.latency = latency;
                    }
                    if let Some(name) = &e.display_name {
                        t.display_name = name
                            .as_ref()
                            .map(sunrise_game_data::scoreboard::tag_to_json);
                    }
                    if let Some(order) = e.list_order {
                        t.list_order = order;
                    }
                    if let Some(hat) = e.show_hat {
                        t.show_hat = hat;
                    }
                    match e.listed {
                        Some(true) => {
                            r.listed.insert(e.uuid);
                        }
                        Some(false) => {
                            r.listed.remove(&e.uuid);
                        }
                        None => {}
                    }
                }
                if let Some(name) = e.name {
                    r.players.insert(e.uuid, name);
                }
                // `Player.isPickable` sees `AbstractClientPlayer.gameMode` as soon as
                // `ClientPacketListener.applyPlayerInfoUpdate` publishes its new mode.
                if mode_change.is_some()
                    && let Some(id) = r.player_entity(e.uuid)
                    && let Some(index) = self.common.mob_index.find(&self.common.mobs, id)
                {
                    let spectator = r.remote_player_info_game_mode(id as i32) == Some(SPECTATOR);
                    self.common
                        .mobs
                        .update(index, |mob| mob.extra.spectator = spectator);
                }
            }
        }
    }
    /// `C::Respawn`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game::client_side) fn apply_respawn_packet(&mut self, p: play::Respawn) {
        self.reset_client_item_use();
        // The new LocalPlayer copies reduced debug info through its setter.
        self.client.debug_entries.0.invalidate();
        let old_player = self
            .client
            .remote
            .as_ref()
            .map(|r| (r.entity_id, r.profile));
        if let Some((_, uuid)) = old_player {
            self.common
                .forget_entity_initialization(Holder::Player(uuid));
        }
        if let Some(r) = self.client.remote.as_ref() {
            self.client.audio.0.remove_sound_entity(r.entity_id as u32);
        }
        // A new player, without effects (the server sends them
        // again) and with no item in use.
        self.common.potions.effects.clear();
        self.client.post_effects.clear();
        // Every synced attachment but the player's is sent again; the viewed
        // levels go or stay with the level entered (`enter_respawn_level`).
        if let Some(r) = self.client.remote.as_mut() {
            r.attachments
                .retain(|holder, _| matches!(holder, Holder::Player(_)));
        }
        self.common.using = None;
        // After a death (no KEEP_ENTITY_DATA) the new LocalPlayer
        // rides nothing: the server pairs a vehicle again with its
        // passengers. A dimension change keeps the vehicle it travels in.
        // handleRespawn: KEEP_ENTITY_DATA gives the new player the old
        // one's motion, `resetPos` none.
        let kept_velocity = if p.keep & 2 != 0 {
            self.common.player.as_ref().map(|pl| pl.velocity)
        } else {
            None
        };
        if let Some(r) = self.client.remote.as_mut() {
            r.respawn_velocity = kept_velocity;
        }
        if p.keep & 2 == 0 {
            self.reset_remote_player_riding();
            self.player_score = 0;
            // KEEP_ENTITY_DATA copies the old player's synched data
            // (its shared flags) to the new one; without it they reset.
            if let Some(r) = self.client.remote.as_mut() {
                r.own_invisible = false;
                r.own_on_fire = false;
                r.glowing.remove(&r.entity_id);
                r.local_entity_silent = false;
            }
        }
        // handleRespawn compares the levels' keys: a mod's level is
        // its own dimension even where `Dim` has no name for it.
        let dimension_changed = self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.level != p.spawn.dimension);
        let died = self.client.screen == Screen::Dead
            || self
                .common
                .player
                .as_ref()
                .is_some_and(|m| m.health.health() <= 0.0);
        let reason = sunrise_client::level_loading::reason_for_respawn(
            died,
            &p.spawn.dimension,
            self.common.dim.id(),
        );
        if dimension_changed {
            sunrise_level::spans::startup::arm("dimension.change");
        }
        // handleRespawn makes a new ClientLevel for another dimension
        // key (a proxy's server switch too): its predictions count from 0.
        self.client
            .packet_holds
            .borrow_mut()
            .clear_cauldron_continuations();
        if let Some(r) = self.client.remote.as_mut() {
            r.pending_cauldrons.get_mut().clear();
            if r.level != p.spawn.dimension {
                r.sequence = 0;
            }
        }
        self.client.respawn_attributes.respawn(p.keep & 1 != 0);
        let entry = if died {
            RemoteDimensionEntry::DeathRespawn
        } else {
            RemoteDimensionEntry::Respawn
        };
        self.enter_respawn_level(&p, entry, reason);
        // handleRespawn adds the new LocalPlayer after what the kept
        // level holds; a new level holds nothing yet.
        let before: std::collections::HashSet<u32> = self
            .common
            .mobs
            .iter()
            .map(|m| m.id)
            .chain(self.common.boats.iter().map(|b| b.id))
            .collect();
        if let Some(r) = self.client.remote.as_mut() {
            r.ticked_before_player = before;
        }
        if dimension_changed {
            // LevelExtractor.setLevel: a new level drops the highlights.
            self.client.test_highlights.clear();
            let (music, _) = self.situational_music();
            self.client
                .music
                .stop_playing(music.as_ref(), &mut self.client.audio.0);
        }
        self.set_screen(Screen::Playing);
        if let Some(r) = self.client.remote.as_mut() {
            r.death_message = None;
            r.health = (20.0, 20, 5.0);
            // handleRespawn: the new LocalPlayer is the camera.
            r.camera = None;
            r.detached_camera = None;
        }
        self.clear_spectated_entity_post_effect();
        self.capture(true);
        if let Some((old_id, uuid)) = old_player {
            self.native_client_player_clone(
                old_id,
                self.own_entity_id(),
                uuid,
                died,
                dimension_changed,
            );
        }
    }
    /// `C::SetCursorItem`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_set_cursor_item_packet(&mut self, p: play::SetCursorItem) {
        let stack = self.to_stack(&p.item);
        self.tutorial_get_item(stack.as_ref());
        self.common.carried = stack;
    }
    /// `C::SetDisplayObjective`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_set_display_objective_packet(
        &mut self,
        p: play::SetDisplayObjective,
    ) {
        // DisplaySlot ids: list, sidebar, below_name, then the team sidebars.
        let slot = match p.slot {
            0 => "list".to_owned(),
            2 => "below_name".to_owned(),
            n @ 3..=18 => format!(
                "sidebar.team.{}",
                sunrise_game_data::text::COLORS[(n - 3) as usize].1
            ),
            _ => "sidebar".to_owned(),
        };
        let board = &mut self.common.score.board;
        let objective = board.objective(&p.objective).map(|o| o.name.clone());
        board.set_display_objective(&slot, objective.as_deref());
    }
    /// `C::SetExperience`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_set_experience_packet(&mut self, p: play::SetExperience) {
        self.common.experience = Experience {
            level: p.level,
            progress: p.progress,
            total: p.total,
        }
    }
    /// `C::SetHealth`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game::client_side) fn apply_set_health_packet(&mut self, p: play::SetHealth) {
        // ClientPacketListener.handleSetHealth: LocalPlayer.hurtTo.
        let flash = self.client.remote.as_mut().is_some_and(|r| {
            r.health = (p.health, p.food, p.saturation);
            std::mem::replace(&mut r.flash_on_set_health, true)
        });
        if flash && let Some(pl) = self.common.player.as_mut() {
            pl.health.flash_health_change(p.health);
        }
        self.apply_remote_health();
        if p.health <= 0.0 {
            self.reset_client_item_use();
        }
        // The death screen opens (or the respawn is asked for) with the
        // death packet when the rule hides it.
        let shown = self
            .client
            .remote
            .as_ref()
            .is_none_or(|r| r.show_death_screen);
        if p.health <= 0.0
            && shown
            && self.client.screen != Screen::Dead
            && self.common.player.is_some()
        {
            self.close_dialogs();
            self.set_screen(Screen::Dead);
            self.client.chat_ui.death_time = 0;
            self.client.chat_ui.death_confirm = None;
            self.capture(false);
        }
    }
    /// `C::SetHeldSlot`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_set_held_slot_packet(&mut self, p: play::SetHeldSlot) {
        // ClientPacketListener.handleSetHeldSlot: a hotbar slot only, and the
        // carried index stays, so the next tick answers with SetCarriedItem.
        if (0..9).contains(&p.slot) {
            self.common.selected = p.slot as usize;
        }
    }
    /// `C::SetPlayerInventory`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_set_player_inventory_packet(
        &mut self,
        p: play::SetPlayerInventory,
    ) {
        let stack = self.to_stack(&p.item);
        self.tutorial_get_item(stack.as_ref());
        let before = self.creative_stacks();
        let held = match p.slot {
            0..=35 => self.common.inventory[p.slot as usize].as_ref(),
            40 => self.common.offhand.as_ref(),
            _ => None,
        };
        let changed = !super::super::hand::matches_for_swap(held, stack.as_ref());
        if let Ok(slot) = usize::try_from(p.slot) {
            self.remote_hand_slot_set(slot, changed);
        }
        match p.slot {
            0..=35 => self.common.inventory[p.slot as usize] = stack,
            36..=39 => self.common.armor[3 - (p.slot - 36) as usize] = stack,
            // Inventory.EQUIPMENT_SLOT_MAPPING: SLOT_OFFHAND, SLOT_BODY_ARMOR, SLOT_SADDLE.
            40 => self.common.offhand = stack,
            41 => {
                self.common
                    .equipment
                    .set(sunrise_items::equipment::BODY, stack);
            }
            42 => {
                self.common
                    .equipment
                    .set(sunrise_items::equipment::SADDLE, stack);
            }
            _ => {}
        }
        self.remote_synced_since(&before);
    }
    /// `C::SetScore`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_set_score_packet(&mut self, p: play::SetScore) {
        let board = &mut self.common.score.board;
        if board.objective(&p.objective).is_some() {
            let s = board.score_mut(&p.owner, &p.objective);
            s.value = p.score;
            s.display = p
                .display
                .as_ref()
                .map(sunrise_game_data::scoreboard::tag_to_json);
            s.number_format = p.number_format.as_ref().map(number_format);
        }
    }
    /// `C::SystemChat`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game::client_side) fn apply_system_chat_packet(
        &mut self,
        p: play::SystemChat,
    ) -> Result<()> {
        // The component as sent, translated in the client's own
        // language; mods may change or cancel it. ChatListener.handleOverlay
        // for the action bar.
        let mut text = remote_text(&p.content);
        if !self.chat_received(true, p.overlay, 0, None, None, &mut text) {
            return Ok(());
        }
        if p.overlay {
            self.say_system_queued(&text);
            self.client.titles.set_overlay(text, false);
        } else if self.add_chat_line(text.clone(), sunrise_client::chat::ChatSource::Server) {
            // handleSystemMessage: said once the chat takes it.
            self.say_system_chat_queued(&text);
        }
        Ok(())
    }
    /// `C::TabList`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game::client_side) fn apply_tab_list_packet(&mut self, p: play::TabList) {
        // handleTabListCustomisation: empty text clears it.
        let text = |t: &Tag| {
            let json = sunrise_game_data::scoreboard::tag_to_json(t);
            let text = sunrise_game_data::scoreboard::comp_text(&json);
            text.spans
                .iter()
                .any(|span| !span.text.is_empty())
                .then_some(text)
        };
        self.client.tab_list.0.header = text(&p.header);
        self.client.tab_list.0.footer = text(&p.footer);
    }
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
