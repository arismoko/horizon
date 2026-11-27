use super::{Arg, Context, R, Target, err};
use sunrise_game_data::text::Text;
use sunrise_server_files::user_lists::NameAndId;

impl crate::game::ServerGame<'_> {
    /// SaveAllCommand. Every save waits for the chunks to be written, so
    /// `flush` changes nothing.
    pub(in crate::game::server_side) fn cmd_save_all(&mut self) -> R<i32> {
        self.dedicated_lists()?;
        self.feedback(Text::translate("commands.save.saving", &[]));
        sunrise_gameplay_common::game::CommonGame::admit_entity_save()
            .map_err(|_| err("commands.save.failed", &[]))?;
        if self.server.lan_acting.is_some() {
            // The acting guest's record holds the game's state until their
            // turn ends: saving now would write it to their file.
            // shortcut: the other operators aren't told of a guest's save, add it if anyone asks.
            self.server.save_after_guest_turn = true;
            return Ok(1);
        }
        self.save_game().map_err(|e| {
            log::error!("Sunrise: /save-all: {e}");
            err("commands.save.failed", &[])
        })?;
        self.feedback(Text::translate("commands.save.success", &[]));
        Ok(1)
    }
    /// `TransferCommand.register` and `TransferCommand.transfer`: each target's
    /// client is handed to `hostname:port` (`ClientboundTransferPacket`), and
    /// the source is told who went — the player's name for one, the count for
    /// several. Without a players argument the source's own player is the
    /// target, as `getPlayerOrException` asks.
    pub(in crate::game::server_side) fn cmd_transfer(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Str(hostname)) = ctx.get("hostname").cloned() else {
            return Ok(0);
        };
        // `IntegerArgumentType.getInteger(c, "port")`, else 25565.
        let port = match ctx.get("port") {
            Some(Arg::Int(port)) => *port,
            _ => 25565,
        };
        let targets = if ctx.get("players").is_some() {
            self.entities(ctx, "players", true)?
        } else if self.server.lan_acting.is_none() {
            // CommandSourceStack.getPlayerOrException: a console or a command
            // block has no player to send.
            return Err(err("permissions.requires.player", &[]));
        } else {
            vec![Target::Player]
        };
        let players: Vec<(u32, String)> = targets
            .iter()
            .filter_map(|target| {
                let conn = match *target {
                    Target::Player => self.server.lan_acting?,
                    Target::Guest(conn) => conn,
                    _ => return None,
                };
                Some((conn, self.command_guest(conn)?.name.clone()))
            })
            .collect();
        if players.is_empty() {
            return Err(err("commands.transfer.error.no_players", &[]));
        }
        if let Some(host) = self.server.lan.as_mut() {
            for (conn, _) in &players {
                host.hub.transfer(*conn, &hostname, port);
            }
        }
        // CommandResponseTracker.messages: one player by display name, several
        // by count, both with the hostname and the port.
        let lit = |s: &str| Text::literal(s.to_owned());
        let (key, first) = match players.len() {
            1 => ("commands.transfer.success.single", lit(&players[0].1)),
            n => ("commands.transfer.success.multiple", lit(&n.to_string())),
        };
        self.feedback(Text::translate(
            key,
            &[first, lit(&hostname), lit(&port.to_string())],
        ));
        Ok(players.len() as i32)
    }
    /// Commands.CommandSelection.DEDICATED: on an integrated server these do
    /// not exist.
    pub(in crate::game::server_side) fn dedicated_lists(
        &self,
    ) -> R<std::sync::Arc<std::sync::Mutex<super::super::dedicated::Lists>>> {
        match (
            &self.server.server_lists.0,
            self.common.is_dedicated_server(),
        ) {
            (Some(lists), true) => Ok(lists.clone()),
            _ => Err(err("command.unknown.command", &[])),
        }
    }
    /// `CommandSourceStack.getTextName`.
    pub(in crate::game::server_side) fn source_text_name(&self) -> String {
        if self.server.console_broadcast.is_some() {
            "Server".to_owned()
        } else {
            self.acting_player_name().to_owned()
        }
    }
    /// `NameAndId` of a player target.
    pub(in crate::game::server_side) fn target_profile(&self, t: Target) -> Option<NameAndId> {
        match t {
            Target::Player => Some(NameAndId {
                id: self.server.lan_acting_uuid?,
                name: self.acting_player_name().to_owned(),
            }),
            Target::Guest(conn) => self.command_guest(conn).map(|g| NameAndId {
                id: g.uuid,
                name: g.name.clone(),
            }),
            _ => None,
        }
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
