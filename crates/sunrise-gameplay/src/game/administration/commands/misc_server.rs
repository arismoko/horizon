use super::{Arg, Context, R, Target, err};
use sunrise_core::random::RandomSource;
use sunrise_game_data::text::{self, Text};

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn cmd_random(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let seed = self.common.level.seed;
        if id == "random.reset" {
            let sequence = match ctx.get("sequence") {
                Some(Arg::Resource(s)) => Some(s.clone()),
                _ => None,
            };
            let int = |name: &str| match ctx.get(name) {
                Some(Arg::Int(v)) => Some(*v),
                _ => None,
            };
            let flag = |name: &str| !matches!(ctx.get(name), Some(Arg::Bool(false)));
            let name_text = |s: &str| Text::literal(s.to_owned());
            return match (sequence, int("seed")) {
                (None, None) => {
                    let n = self.server.rand_sequences.clear();
                    self.feedback(Text::translate(
                        "commands.random.reset.all.success",
                        &[Text::literal(n.to_string())],
                    ));
                    Ok(n as i32)
                }
                (None, Some(salt)) => {
                    self.server.rand_sequences.set_defaults(
                        salt,
                        flag("includeWorldSeed"),
                        flag("includeSequenceId"),
                    );
                    let n = self.server.rand_sequences.clear();
                    self.feedback(Text::translate(
                        "commands.random.reset.all.success",
                        &[Text::literal(n.to_string())],
                    ));
                    Ok(n as i32)
                }
                (Some(s), None) => {
                    self.server.rand_sequences.reset(&s, seed);
                    self.feedback(Text::translate(
                        "commands.random.reset.success",
                        &[name_text(&s)],
                    ));
                    Ok(1)
                }
                (Some(s), Some(salt)) => {
                    self.server.rand_sequences.reset_with(
                        &s,
                        seed,
                        salt,
                        flag("includeWorldSeed"),
                        flag("includeSequenceId"),
                    );
                    self.feedback(Text::translate(
                        "commands.random.reset.success",
                        &[name_text(&s)],
                    ));
                    Ok(1)
                }
            };
        }
        let Some(Arg::IntRange((lo, hi))) = ctx.get("range").cloned() else {
            return Ok(0);
        };
        let (min, max) = (lo.unwrap_or(i32::MIN), hi.unwrap_or(i32::MAX));
        let span = i64::from(max) - i64::from(min);
        if span == 0 {
            return Err(err("commands.random.error.range_too_small", &[]));
        }
        if span >= i64::from(i32::MAX) {
            return Err(err("commands.random.error.range_too_large", &[]));
        }
        let value = match ctx.get("sequence") {
            Some(Arg::Resource(s)) => {
                let s = s.clone();
                self.server
                    .rand_sequences
                    .get(&s, seed)
                    .next_int_between_inclusive(min, max)
            }
            _ => self
                .common
                .world
                .random
                .next_int_between_inclusive(min, max),
        }
        .unwrap_or(min);
        if id == "random.roll" {
            let line = Text::translate(
                "commands.random.roll",
                &[
                    self.source_display_name(),
                    Text::literal(value.to_string()),
                    Text::literal(min.to_string()),
                    Text::literal(max.to_string()),
                ],
            );
            // PlayerList.broadcastSystemMessage: every player hears the roll,
            // the source's own copy with the captured output it has (`/say`'s).
            let captured = self
                .server
                .chat_capture
                .as_ref()
                .and(self.server.lan_acting);
            if let Some(host) = &self.server.lan {
                let content = sunrise_sim::lan::data::component_nbt(&line.component());
                match captured {
                    Some(conn) => host.hub.system_message_except(content, conn),
                    None => host.hub.system_message(content),
                }
            }
            // MinecraftServer.sendSystemMessage: the server's log, unless the
            // console's captured reply below is what logs it (`console_reply`).
            if self.common.options.dedicated.is_some()
                && (self.server.lan_acting.is_some() || self.server.chat_capture.is_none())
            {
                super::super::dedicated::log("INFO", &format!("System chat: {}", line.plain()));
            }
            if self.server.lan.is_none() || self.server.chat_capture.is_some() {
                self.server_system_message(line);
            }
        } else {
            self.feedback(Text::translate(
                "commands.random.sample.success",
                &[Text::literal(value.to_string())],
            ));
        }
        Ok(value)
    }
    /// Sends disguised team chat using the world’s incoming and outgoing team chat types.
    pub(in crate::game::server_side) fn cmd_teammsg(&mut self, ctx: &Context) -> R<i32> {
        let entity = match &self.server.command_source {
            Some(s) => s.entity,
            None => Some(Target::Player),
        };
        let Some(entity) = entity else {
            return Err(err("permissions.requires.entity", &[]));
        };
        let Some(holder) = self.holder_name(entity) else {
            return Err(err("permissions.requires.entity", &[]));
        };
        let Some(team) = self.common.score.board.players_team(&holder).cloned() else {
            return Err(err("commands.teammsg.failed.noteam", &[]));
        };
        let sender = match entity {
            Target::Guest(conn) => Some(conn),
            Target::Player => self.server.lan_acting,
            _ => None,
        };
        let board = &self.common.score.board;
        let receivers: Vec<u32> = self
            .server
            .lan
            .iter()
            .flat_map(|host| host.hub.online_players())
            .filter(|g| {
                Some(g.conn) == sender
                    || board
                        .players_team(&g.name)
                        .is_some_and(|t| t.name == team.name)
            })
            .map(|g| g.conn)
            .collect();
        if receivers.is_empty() {
            return Ok(0);
        }
        let raw = match ctx.get("message") {
            Some(Arg::Str(m)) => m.clone(),
            _ => String::new(),
        };
        let message = self.resolve_chat_message(&raw);
        let name = self.source_display_name();
        let suggest = text::Style {
            hover: Some(std::sync::Arc::new(Text::translate(
                "chat.type.team.hover",
                &[],
            ))),
            click: Some(text::ClickEvent::SuggestCommand("/teammsg ".into())),
            ..text::Style::default()
        };
        let team_name = sunrise_sim::lan::data::component_nbt(
            &team
                .formatted_display_name()
                .with_style(&suggest)
                .component(),
        );
        let content = sunrise_sim::lan::data::component_nbt(&message.component());
        let name = sunrise_sim::lan::data::component_nbt(&name.component());
        if let Some(host) = &self.server.lan {
            // ChatType.bind throws for a missing type: the command fails.
            let bind = |chat_type: &str| {
                host.hub.chat_type_id(chat_type).map(|id| {
                    sunrise_protocol::packets::play::ChatTypeBound {
                        chat_type: id.network_id() as i32,
                        name: name.clone(),
                        target: Some(team_name.clone()),
                    }
                })
            };
            let (Some(incoming), Some(outgoing)) = (
                bind("team_msg_command_incoming"),
                bind("team_msg_command_outgoing"),
            ) else {
                return Err(err("command.failed", &[]));
            };
            for &conn in &receivers {
                if let Some(index) = host.hub.guests.connection_index(conn) {
                    let bound = if Some(conn) == sender {
                        outgoing.clone()
                    } else {
                        incoming.clone()
                    };
                    host.hub.send_disguised(index, content.clone(), bound);
                }
            }
        }
        Ok(receivers.len() as i32)
    }
}
