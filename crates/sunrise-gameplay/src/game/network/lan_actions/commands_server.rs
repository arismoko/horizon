use super::*;

impl crate::game::ServerGame<'_> {
    /// Commands.performCommand for a guest, with the message arguments it
    /// signed (none after an unsigned command packet). The packet's command has no chat slash, and
    /// vanilla parses it as sent: a leading `/` is part of the command (a mod's `//set` is `/set`).
    pub fn guest_command(
        &mut self,
        conn: u32,
        command: &str,
        signed: Option<Vec<(String, sunrise_sim::lan::hub::chat::ChatMessage)>>,
    ) {
        // The mods hear every command first, and may stop it.
        let mut canceled = false;
        let mut replies = Vec::new();
        if let Some((uuid, name)) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.uuid, g.name.clone()))
        {
            self.native_player_command(uuid, &name, command, &mut canceled, &mut replies);
        }
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        for line in replies {
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content: sunrise_sim::lan::data::component_nbt(&line.component()),
                    overlay: false,
                }),
            );
        }
        if canceled {
            return;
        }
        if self.guest_message_command(conn, command, signed.as_ref()) {
            return;
        }
        // ListPlayersCommand needs no permission and lists every player.
        let words: Vec<&str> = command.split(' ').collect();
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        if words[0] == "list" && (words.len() == 1 || words[1..] == ["uuids"]) {
            let content = host.hub.list_reply(words.len() == 2);
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content,
                    overlay: false,
                }),
            );
            return;
        }
        // A native mod's command, for every player its level allows,
        // whether the world allows commands or not (game/modding/native_server.rs).
        let allowed = self.common.options.dedicated.is_some() || self.common.allow_commands();
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let integrated = host.hub.config.integrated;
        if let Some(level) = self.server.native_commands.0.level(command)
            && let Some(permission) = host.hub.guest(conn).map(|g| g.permission)
            && permission >= level
        {
            host.commands.push(GuestCommand {
                conn,
                command: command.to_owned(),
                permission,
                integrated,
                quiet: false,
            });
            return;
        }
        // An integrated server runs every player's commands at their own
        // permission level (`CommandSourceStack.permissions`); a LAN world
        // follows its Allow Commands, and a dedicated server leaves it to
        // the ops list (the guest's permission level).
        if !integrated && (!host.settings.guest_commands || !allowed) {
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content: sunrise_protocol::text::translatable("command.unknown.command", &[]),
                    overlay: false,
                }),
            );
            return;
        }
        // The guest's own level (Commands.performCommand with the player's
        // source): an operator's on a dedicated server, never the host's.
        let Some(permission) = host.hub.guest(conn).map(|g| g.permission) else {
            return;
        };
        host.commands.push(GuestCommand {
            conn,
            command: command.to_owned(),
            permission,
            integrated,
            quiet: false,
        });
    }
    pub fn guest_suggestions(&mut self, conn: u32, payload: &[u8]) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let mut r = sunrise_protocol::buf::Reader::new(payload);
        let (Ok(id), Ok(text)) = (r.varint(), r.string(32500)) else {
            return;
        };
        let Some(permission) = host.hub.guest(conn).map(|g| g.permission) else {
            return;
        };
        let Some((player_uuid, player_name)) =
            host.hub.guest(conn).map(|g| (g.uuid, g.name.clone()))
        else {
            return;
        };
        let skip = usize::from(text.starts_with('/'));
        let command = &text[skip..];
        let d = self.common.ui.dispatcher.0.clone();
        // Server players and lists, not the host's registries: a dedicated server has no local player.
        let triggers = super::super::scoreboard::trigger_objectives_for(
            &self.common.score.board,
            &player_name,
        );
        let mut players = if self.common.options.dedicated.is_some() {
            Vec::new()
        } else {
            self.common.ui.registries.players.clone()
        };
        players.extend(host.hub.online_players().map(|g| g.name.clone()));
        let (ops, banned_players, banned_ips, whitelisted) = match &self.server.server_lists.0 {
            Some(lists) => {
                let l = lists.lock().unwrap();
                (
                    l.ops
                        .entries()
                        .iter()
                        .map(|e| e.user.name.clone())
                        .collect(),
                    l.bans
                        .entries()
                        .iter()
                        .map(|e| e.user.name.clone())
                        .collect(),
                    l.ip_bans.entries().iter().map(|e| e.user.clone()).collect(),
                    l.white
                        .entries()
                        .iter()
                        .map(|e| e.user.name.clone())
                        .collect(),
                )
            }
            None => Default::default(),
        };
        let reg = self.suggestion_registries();
        let level = std::mem::replace(&mut reg.permission, permission);
        let own_sender = reg
            .sender
            .replace(sunrise_game_data::registries::CommandSender {
                uuid: player_uuid,
                name: player_name.clone(),
            });
        let own_players = std::mem::replace(&mut reg.players, players);
        let own_ops = std::mem::replace(&mut reg.ops, ops);
        let own_banned = std::mem::replace(&mut reg.banned_players, banned_players);
        let own_banned_ips = std::mem::replace(&mut reg.banned_ips, banned_ips);
        let own_whitelisted = std::mem::replace(&mut reg.whitelisted, whitelisted);
        let own_triggers = std::mem::replace(&mut reg.trigger_objectives, triggers);
        let parse = d.parse(command, &self.common.ui.registries);
        // Commands.sendCommands prunes the tree a player's client completes from; the answer the
        // server sends follows the same test, so a child literal of a path the sender may not run
        // is not offered either (Bukkit hides it; brigadier alone lists every child).
        let ((start, end), mut suggestions) =
            d.client_suggestions(&parse, command.len(), &self.common.ui.registries);
        self.native_tab_complete(
            player_uuid,
            &player_name,
            command,
            [start, end],
            &mut suggestions,
        );
        let reg = self.suggestion_registries();
        reg.permission = level;
        reg.sender = own_sender;
        reg.players = own_players;
        reg.ops = own_ops;
        reg.banned_players = own_banned;
        reg.banned_ips = own_banned_ips;
        reg.whitelisted = own_whitelisted;
        reg.trigger_objectives = own_triggers;
        suggestions.retain(sunrise_command::suggestion_fits_packet);
        suggestions.truncate(sunrise_command::MAX_PROVIDER_SUGGESTIONS);
        let utf16 = |byte: usize| command[..byte.min(command.len())].encode_utf16().count() as i32;
        let mut w = ::sunrise_core::vanilla_bytes::Writer::new();
        w.varint(id);
        w.varint(utf16(start) + skip as i32);
        w.varint(utf16(end) - utf16(start));
        w.varint(suggestions.len() as i32);
        for s in &suggestions {
            w.string(&s.text);
            w.bool(s.tooltip.is_some());
            if let Some(tip) = &s.tooltip {
                w.nbt(&literal(tip));
            }
        }
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        host.hub.send_to(
            conn,
            &C::Unknown(
                sunrise_protocol::ids::play::clientbound::COMMAND_SUGGESTIONS,
                w.data,
            ),
        );
    }
    /// ServerGamePacketListenerImpl.handleCustomCommandSuggestions: the
    /// completions of the command typed so far (after an optional slash), for
    /// the nodes the tree marks `ask_server`.
    /// ServerCommandSuggestionsProvider.tryProcessRequest: answer the latest request if this
    /// tick's answer is free. One client asking a hundred times a tick costs one parse.
    pub(in crate::game::server_side) fn guest_try_suggestions(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let play = host.play.entry(conn).or_default();
        if !play.suggestion_budget {
            return;
        }
        let Some(payload) = play.suggestion.take() else {
            return;
        };
        play.suggestion_budget = false;
        self.guest_suggestions(conn, &payload);
    }
    /// One guest's command as that guest, its feedback sent to them; false
    /// when it failed (or the guest is gone).
    pub fn run_guest_command(&mut self, c: GuestCommand) -> bool {
        self.run_guest_command_told(c).is_some_and(|(ok, _)| ok)
    }
    /// `run_guest_command`, with the lines it sent the guest; None when the
    /// guest is gone.
    pub fn run_guest_command_told(
        &mut self,
        c: GuestCommand,
    ) -> Option<(bool, Vec<sunrise_game_data::text::Text>)> {
        let (ok, mut lines) = self.with_guest_swap(c.conn, self.common.dim, |game| {
            game.server.chat_capture = Some(Vec::new());
            let level = std::mem::replace(&mut game.ui.registries.permission, c.permission);
            let ok = match game.perform_native_command(c.conn, c.permission, &c.command, c.quiet) {
                Some(ok) => ok,
                None if c.quiet => game.run_command_quietly(&c.command),
                None => game.run_command(&c.command),
            };
            game.ui.registries.permission = level;
            (ok, game.server.chat_capture.take().unwrap_or_default())
        })?;
        if std::mem::take(&mut self.server.save_after_guest_turn) {
            lines.push(self.save_all_reply());
        }
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.sync_all_effects();
        }
        if !c.integrated {
            // A teleport into another level happens before its feedback is sent.
            self.run_level_teleports();
        }
        // What the command earned is announced before its feedback.
        self.announce_guest_advancements();
        for line in &lines {
            let Some(host) = self.server.lan.as_ref() else {
                return Some((ok, lines));
            };
            host.hub.send_to(
                c.conn,
                &C::SystemChat(play::SystemChat {
                    content: sunrise_sim::lan::data::component_nbt(&line.component()),
                    overlay: false,
                }),
            );
        }
        // The other operators hear of it; never the player whose command it was.
        self.send_admin_feedback();
        Some((ok, lines))
    }
    /// The guests' queued commands, run with the hub in `self.lan` so that
    /// they reach the other players too (`/give Bob`, `/tp Bob`, `/kick`).
    pub fn run_guest_commands(&mut self) {
        loop {
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            if host.commands.is_empty() {
                return;
            }
            let c = host.commands.remove(0);
            self.run_guest_command(c);
        }
    }
    /// The `broadcastToAdmins` lines queued so far: every operator
    /// (`PlayerList.isOp`) but the guest each line's command came from (kept
    /// with the line, so a block's line still pending when a guest's command
    /// drains the queue reaches that guest too). The server's log already has
    /// them (`broadcast_to_admins`). Without a server open to players they are
    /// dropped: no one else can hear them.
    pub fn send_admin_feedback(&mut self) {
        let lines = std::mem::take(&mut self.server.admin_feedback);
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        // server.toml's `admin_feedback_to_ops` off: the log only.
        if !host.hub.admin_feedback_to_ops {
            return;
        }
        for (line, source) in lines {
            let content = sunrise_sim::lan::data::component_nbt(&line.component());
            for g in host
                .hub
                .guests
                .iter()
                .filter(|g| g.placed && Some(g.conn) != source && host.hub.is_op(g.uuid))
            {
                g.send(&C::SystemChat(play::SystemChat {
                    content: content.clone(),
                    overlay: false,
                }));
            }
        }
    }
}
