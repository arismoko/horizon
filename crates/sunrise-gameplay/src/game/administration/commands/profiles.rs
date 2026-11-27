//! `/op`, `/deop`, `/ban`, `/pardon`, `/ban-ip`, `/pardon-ip`, `/banlist`,
//! `/setidletimeout`, `/transfer` and `/whitelist` (OpCommand, DeOpCommands,
//! BanPlayerCommands, PardonCommand, BanIpCommands, PardonIpCommand,
//! BanListCommands, SetPlayerIdleTimeoutCommand, TransferCommand,
//! WhitelistCommand): commands of a dedicated server, run on its lists by an
//! operator in game (the console has its own handlers in game/interface/dedicated/mod.rs).
//! Also `/stop`, `/save-all`, `/save-off` and `/save-on` (StopCommand,
//! SaveAllCommand, SaveOffCommand, SaveOnCommand), on the switches the lists
//! share.
use super::{Arg, Context, R, err};
use mod_api::Players;
use sunrise_game_data::text::Text;
use sunrise_protocol::text::translatable;
use sunrise_server_files::user_lists::{BanEntry, NameAndId, OpEntry, WhiteEntry};

impl crate::game::ServerGame<'_> {
    /// StopCommand: the server loop (game/interface/dedicated/mod.rs) ends after this tick.
    pub(in crate::game::server_side) fn cmd_stop(&mut self) -> R<i32> {
        let lists = self.dedicated_lists()?;
        self.feedback(Text::translate("commands.stop.stopping", &[]));
        lists.lock().unwrap().stop = true;
        Ok(1)
    }

    /// Save everything; `commands.save.success`, or `commands.save.failed` in red.
    pub(in crate::game) fn save_all_reply(&mut self) -> Text {
        match self.save_game() {
            Ok(()) => Text::translate("commands.save.success", &[]),
            Err(e) => {
                log::error!("Sunrise: /save-all: {e}");
                Text::translate("commands.save.failed", &[])
                    .with_color(sunrise_game_data::text::RED)
            }
        }
    }

    /// SaveOffCommand and SaveOnCommand.
    pub(in crate::game::server_side) fn cmd_auto_save(&mut self, enabled: bool) -> R<i32> {
        let lists = self.dedicated_lists()?;
        if !super::super::dedicated::set_auto_save(&lists, enabled) {
            let already = if enabled {
                "commands.save.alreadyOn"
            } else {
                "commands.save.alreadyOff"
            };
            return Err(err(already, &[]));
        }
        let done = if enabled {
            "commands.save.enabled"
        } else {
            "commands.save.disabled"
        };
        self.feedback(Text::translate(done, &[]));
        Ok(1)
    }

    /// `GameProfileArgument.getGameProfiles`: resolve the selector or use the caller's name lookup.
    fn resolve_profiles(
        &mut self,
        argument: Option<&Arg>,
        lookup: impl FnOnce(&mut Self, &str) -> Option<NameAndId>,
    ) -> R<Vec<NameAndId>> {
        match argument {
            Some(Arg::Entity(selector)) => {
                let players = self
                    .resolve_entity_argument(selector, true)?
                    .into_iter()
                    .filter_map(|target| self.target_profile(target))
                    .collect::<Vec<_>>();
                if players.is_empty() {
                    return Err(err("argument.entity.notfound.player", &[]));
                }
                Ok(players)
            }
            Some(Arg::Str(name)) => lookup(self, name)
                .map(|profile| vec![profile])
                .ok_or_else(|| err("argument.player.unknown", &[])),
            _ => Err(err("argument.player.unknown", &[])),
        }
    }

    /// The built-in commands may contact the profile service or create an offline
    /// identity if a name is not cached.
    pub(in crate::game::server_side) fn game_profiles(
        &mut self,
        ctx: &Context,
        name: &str,
    ) -> R<Vec<NameAndId>> {
        self.resolve_profiles(ctx.get(name), Self::resolve_name)
    }

    /// Mods resolve plain names only from server state.
    pub(in crate::game) fn mod_game_profiles(&mut self, argument: &Arg) -> R<Vec<(u128, String)>> {
        Ok(self
            .resolve_profiles(Some(argument), |game, name| {
                game.profile_by_name(name).map(|profile| NameAndId {
                    id: profile.uuid,
                    name: profile.name.to_owned(),
                })
            })?
            .into_iter()
            .map(|profile| (profile.id, profile.name))
            .collect())
    }

    /// `OpCommand.opPlayers`, `DeOpCommands.deopPlayers`,
    /// `BanPlayerCommands.banPlayers` and `PardonCommand.pardonPlayers`.
    pub(in crate::game::server_side) fn cmd_profiles(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let lists = self.dedicated_lists()?;
        let players = self.game_profiles(ctx, "targets")?;
        let lit = |s: &str| Text::literal(s.to_owned());
        let mut count = 0;
        match id {
            "op" => {
                for player in &players {
                    let (known, level) = {
                        let l = lists.lock().unwrap();
                        (l.ops.contains(player), l.op_level)
                    };
                    if !known && self.permission_change(&lists, player, level) {
                        continue;
                    }
                    let added = {
                        let mut l = lists.lock().unwrap();
                        let level = l.op_level;
                        !l.ops.contains(player)
                            && l.ops.add(OpEntry {
                                user: player.clone(),
                                level,
                                bypasses_player_limit: false,
                            })
                    };
                    if added {
                        self.sync_ops(&lists);
                        count += 1;
                        self.feedback(Text::translate("commands.op.success", &[lit(&player.name)]));
                    }
                }
                if count == 0 {
                    return Err(err("commands.op.failed", &[]));
                }
            }
            "deop" => {
                for player in &players {
                    let known = lists.lock().unwrap().ops.contains(player);
                    if known && self.permission_change(&lists, player, 0) {
                        continue;
                    }
                    let removed = {
                        let mut l = lists.lock().unwrap();
                        l.ops.contains(player) && l.ops.remove(player)
                    };
                    if removed {
                        self.sync_ops(&lists);
                        count += 1;
                        self.feedback(Text::translate(
                            "commands.deop.success",
                            &[lit(&player.name)],
                        ));
                    }
                }
                if count == 0 {
                    return Err(err("commands.deop.failed", &[]));
                }
                self.kick_unlisted_players(&lists);
            }
            "ban" => {
                let reason = match ctx.get("reason") {
                    Some(Arg::Str(raw)) => {
                        let raw = raw.clone();
                        Some(self.expand_selectors(&raw).plain().to_string())
                    }
                    _ => None,
                };
                let source = self.source_text_name();
                for player in &players {
                    let banned = lists.lock().unwrap().bans.contains(player);
                    if banned {
                        continue;
                    }
                    lists.lock().unwrap().bans.add(BanEntry::new(
                        player.clone(),
                        Some(source.clone()),
                        None,
                        reason.clone(),
                    ));
                    count += 1;
                    // BanListEntry.getReasonMessage.
                    let shown = reason.as_deref().map_or_else(
                        || Text::translate("multiplayer.disconnect.banned.reason.default", &[]),
                        lit,
                    );
                    self.feedback(Text::translate(
                        "commands.ban.success",
                        &[lit(&player.name), shown],
                    ));
                    self.kick_uuid(
                        player.id,
                        translatable("multiplayer.disconnect.banned", &[]),
                    );
                }
                if count == 0 {
                    return Err(err("commands.ban.failed", &[]));
                }
            }
            _ => {
                for player in &players {
                    let removed = {
                        let mut l = lists.lock().unwrap();
                        l.bans.contains(player) && l.bans.remove(player)
                    };
                    if removed {
                        count += 1;
                        self.feedback(Text::translate(
                            "commands.pardon.success",
                            &[lit(&player.name)],
                        ));
                    }
                }
                if count == 0 {
                    return Err(err("commands.pardon.failed", &[]));
                }
            }
        }
        Ok(count)
    }

    /// `BanIpCommands.banIpOrName` and `PardonIpCommand.unban`: an address,
    /// or for `/ban-ip` an online player's.
    pub(in crate::game::server_side) fn cmd_ip_bans(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let lists = self.dedicated_lists()?;
        let lit = |s: &str| Text::literal(s.to_owned());
        let target = match ctx.get("target") {
            Some(Arg::Str(t)) => t.clone(),
            _ => return Err(err("command.unknown.argument", &[])),
        };
        let is_ip = target.parse::<std::net::IpAddr>().is_ok();
        if id == "pardon-ip" {
            if !is_ip {
                return Err(err("commands.pardonip.invalid", &[]));
            }
            let removed = {
                let mut l = lists.lock().unwrap();
                l.ip_bans.get(&target).is_some() && l.ip_bans.remove(&target)
            };
            if !removed {
                return Err(err("commands.pardonip.failed", &[]));
            }
            self.feedback(Text::translate(
                "commands.pardonip.success",
                &[lit(&target)],
            ));
            return Ok(1);
        }
        // PlayerList.getPlayerByName, then ServerPlayer.getIpAddress.
        let address = |name: &str| {
            let hub = &self.server.lan.as_ref()?.hub;
            hub.online_players()
                .find(|g| g.name.eq_ignore_ascii_case(name))
                .map(|g| g.ip_address().to_owned())
        };
        let ip = if is_ip {
            Some(target)
        } else {
            address(&target)
        };
        let Some(ip) = ip else {
            return Err(err("commands.banip.invalid", &[]));
        };
        if lists.lock().unwrap().ip_bans.get(&ip).is_some() {
            return Err(err("commands.banip.failed", &[]));
        }
        let players: Vec<(u128, String)> = self
            .server
            .lan
            .as_ref()
            .map(|h| {
                h.hub
                    .online_players()
                    .filter(|g| g.ip_address() == ip)
                    .map(|g| (g.uuid, g.name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let reason = match ctx.get("reason") {
            Some(Arg::Str(raw)) => {
                let raw = raw.clone();
                Some(self.expand_selectors(&raw).plain().to_string())
            }
            _ => None,
        };
        let source = self.source_text_name();
        lists.lock().unwrap().ip_bans.add(BanEntry::new(
            ip.clone(),
            Some(source),
            None,
            reason.clone(),
        ));
        self.feedback(Text::translate(
            "commands.banip.success",
            &[lit(&ip), Self::reason_message(reason.as_deref())],
        ));
        if !players.is_empty() {
            let names: Vec<&str> = players.iter().map(|(_, n)| n.as_str()).collect();
            self.feedback(Text::translate(
                "commands.banip.info",
                &[lit(&players.len().to_string()), lit(&names.join(", "))],
            ));
        }
        for (uuid, _) in &players {
            self.kick_uuid(*uuid, translatable("multiplayer.disconnect.ip_banned", &[]));
        }
        Ok(players.len() as i32)
    }

    /// `SetPlayerIdleTimeoutCommand.setIdleTimeout`: the minutes of idleness
    /// after which a player is kicked (`multiplayer.disconnect.idling`), 0 for
    /// never. The reply names the minutes, or says the timeout is off, and the
    /// command's result is the number of minutes.
    pub(in crate::game::server_side) fn cmd_setidle_timeout(&mut self, ctx: &Context) -> R<i32> {
        let lists = self.dedicated_lists()?;
        let Some(Arg::Int(minutes)) = ctx.get("minutes") else {
            return Ok(0);
        };
        let minutes = *minutes;
        // DedicatedServer.setPlayerIdleTimeout: the live sweep reads it, and
        // server.properties keeps it.
        self.set_player_idle_timeout(&lists, minutes);
        self.feedback(idle_timeout_reply(minutes));
        Ok(minutes)
    }

    /// `BanListCommands.showList`: the players' bans then the addresses'
    /// (`banlist`), or one of them; told to the source only.
    pub(in crate::game::server_side) fn cmd_banlist(&mut self, id: &str) -> R<i32> {
        let lists = self.dedicated_lists()?;
        let lit = |s: &str| Text::literal(s.to_owned());
        let entries: Vec<(String, String, Option<String>)> = {
            let l = lists.lock().unwrap();
            let players = l
                .bans
                .entries()
                .into_iter()
                .map(|b| (b.user.name.clone(), b.source.clone(), b.reason.clone()));
            let ips = l
                .ip_bans
                .entries()
                .into_iter()
                .map(|b| (b.user.clone(), b.source.clone(), b.reason.clone()));
            match id {
                "banlist.ips" => ips.collect(),
                "banlist.players" => players.collect(),
                _ => players.chain(ips).collect(),
            }
        };
        if entries.is_empty() {
            self.feedback_unbroadcast(Text::translate("commands.banlist.none", &[]));
            return Ok(0);
        }
        self.feedback_unbroadcast(Text::translate(
            "commands.banlist.list",
            &[lit(&entries.len().to_string())],
        ));
        for (name, source, reason) in &entries {
            self.feedback_unbroadcast(Text::translate(
                "commands.banlist.entry",
                &[
                    lit(name),
                    lit(source),
                    Self::reason_message(reason.as_deref()),
                ],
            ));
        }
        Ok(entries.len() as i32)
    }

    /// `BanListEntry.getReasonMessage`.
    pub(in crate::game::server_side) fn reason_message(reason: Option<&str>) -> Text {
        reason.map_or_else(
            || Text::translate("multiplayer.disconnect.banned.reason.default", &[]),
            |r| Text::literal(r.to_owned()),
        )
    }

    /// `WhitelistCommand`: `on`, `off`, `list`, `reload`, `add` and `remove`.
    pub(in crate::game::server_side) fn cmd_whitelist(
        &mut self,
        id: &str,
        ctx: &Context,
    ) -> R<i32> {
        let lists = self.dedicated_lists()?;
        let lit = |s: &str| Text::literal(s.to_owned());
        match id {
            "whitelist.reload" => {
                // PlayerList.reloadWhiteList.
                let _ = lists.lock().unwrap().white.load();
                self.feedback(Text::translate("commands.whitelist.reloaded", &[]));
                self.kick_unlisted_players(&lists);
                Ok(1)
            }
            "whitelist.on" => {
                if lists.lock().unwrap().use_whitelist {
                    return Err(err("commands.whitelist.alreadyOn", &[]));
                }
                self.set_using_whitelist(&lists, true);
                self.feedback(Text::translate("commands.whitelist.enabled", &[]));
                self.kick_unlisted_players(&lists);
                Ok(1)
            }
            "whitelist.off" => {
                if !lists.lock().unwrap().use_whitelist {
                    return Err(err("commands.whitelist.alreadyOff", &[]));
                }
                self.set_using_whitelist(&lists, false);
                self.feedback(Text::translate("commands.whitelist.disabled", &[]));
                Ok(1)
            }
            "whitelist.list" => {
                let names: Vec<String> = lists
                    .lock()
                    .unwrap()
                    .white
                    .entries()
                    .iter()
                    .map(|e| e.user.name.clone())
                    .collect();
                if names.is_empty() {
                    self.feedback_unbroadcast(Text::translate("commands.whitelist.none", &[]));
                } else {
                    self.feedback_unbroadcast(Text::translate(
                        "commands.whitelist.list",
                        &[lit(&names.len().to_string()), lit(&names.join(", "))],
                    ));
                }
                Ok(names.len() as i32)
            }
            _ => {
                let add = id == "whitelist.add";
                let targets = self.game_profiles(ctx, "targets")?;
                let mut count = 0;
                for target in &targets {
                    let done = {
                        let mut l = lists.lock().unwrap();
                        if add {
                            !l.white.contains(target)
                                && l.white.add(WhiteEntry {
                                    user: target.clone(),
                                })
                        } else {
                            l.white.contains(target) && l.white.remove(target)
                        }
                    };
                    if done {
                        count += 1;
                        let key = if add {
                            "commands.whitelist.add.success"
                        } else {
                            "commands.whitelist.remove.success"
                        };
                        self.feedback(Text::translate(key, &[lit(&target.name)]));
                    }
                }
                if count == 0 {
                    return Err(err(
                        if add {
                            "commands.whitelist.add.failed"
                        } else {
                            "commands.whitelist.remove.failed"
                        },
                        &[],
                    ));
                }
                if !add {
                    self.kick_unlisted_players(&lists);
                }
                Ok(count)
            }
        }
    }
}

/// `SetPlayerIdleTimeoutCommand.setIdleTimeout`'s reply: the minutes, or
/// `commands.setidletimeout.success.disabled` when the timeout is off.
pub(in crate::game) fn idle_timeout_reply(minutes: i32) -> Text {
    if minutes > 0 {
        Text::translate(
            "commands.setidletimeout.success",
            &[Text::literal(minutes.to_string())],
        )
    } else {
        Text::translate("commands.setidletimeout.success.disabled", &[])
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::idle_timeout_reply;

    /// `SetPlayerIdleTimeoutCommand.setIdleTimeout`: minutes above zero are
    /// named in `commands.setidletimeout.success`, and zero turns the timeout
    /// off with `commands.setidletimeout.success.disabled`.
    #[test]
    fn the_idle_timeout_reply_names_the_minutes_or_says_it_is_off() {
        let reply = idle_timeout_reply(5);
        assert_eq!(
            reply.translation_key(),
            Some("commands.setidletimeout.success"),
            "{reply:?}"
        );
        assert_eq!(reply.component()["with"], serde_json::json!([5]));
        let off = idle_timeout_reply(0);
        assert_eq!(
            off.translation_key(),
            Some("commands.setidletimeout.success.disabled")
        );
        // The disabled message takes no argument.
        assert!(off.component().get("with").is_none(), "{off:?}");
    }
}
