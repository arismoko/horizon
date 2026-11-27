//! A guest's commands that carry chat: the unsigned and signed command
//! packets (ServerGamePacketListenerImpl.handleChatCommand and
//! handleSignedChatCommand), and the message commands whose text they sign
//! (SayCommand, EmoteCommands, MsgCommand), relayed by the hub as chat.
use super::commands::Target;
use sunrise_command::{Arg, ParseResults};
use sunrise_core::nbt::Tag;
use sunrise_game_data::text::{self, Text};
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_sim::lan::{
    data::component_nbt,
    hub::{HostEvent, chat::ChatMessage},
};

/// `OutgoingChatMessage`: preserve the signed body, or deliver a disguised component.
pub enum ChatDelivery<'a> {
    Player(&'a ChatMessage),
    Disguised(&'a Tag),
}

/// An owned [`ChatDelivery`], for a broadcast that waits its turn.
enum QueuedDelivery {
    Player(ChatMessage),
    Disguised(Tag),
}

/// A public chat line, identified by its sender and the text shown.
type ChatKey = (u128, String);

struct QueuedBroadcast {
    sender: u128,
    name: String,
    shown: String,
    line: Text,
    bound: play::ChatTypeBound,
    delivery: QueuedDelivery,
    /// The lines whose delivery led to this one, itself last.
    path: Vec<ChatKey>,
}

/// Broadcasts started from inside a recipient listener: delivered one after another once the
/// running broadcast finishes (no recursion), and dropped when a line repeats one of the lines
/// that led to it (a relay answering its own answer).
#[derive(Default)]
pub(in crate::game) struct ChatBroadcasts {
    active: Option<Vec<ChatKey>>,
    queue: std::collections::VecDeque<QueuedBroadcast>,
}

/// The signing context of a command: each message argument as the chain
/// let it through, by argument name (CommandSigningContext.SignedArguments).
type Signed = Vec<(String, ChatMessage)>;

impl crate::game::ServerGame<'_> {
    /// CommandDispatcher.parse with the guest's permission level.
    fn parse_as(&mut self, permission: u8, command: &str) -> ParseResults {
        let level = std::mem::replace(&mut self.common.ui.registries.permission, permission);
        let parse = self
            .common
            .ui
            .dispatcher
            .0
            .clone()
            .parse(command, &self.common.ui.registries);
        self.common.ui.registries.permission = level;
        parse
    }

    /// SignableCommand.of: the command's message arguments, by name, with
    /// their text, not past a redirect to the root (`execute ... run`).
    pub(in crate::game) fn signable_arguments(
        &mut self,
        permission: u8,
        command: &str,
    ) -> Vec<(String, String)> {
        self.parse_as(permission, command)
            .signable_arguments(&self.common.ui.dispatcher.0)
    }

    /// ServerGamePacketListenerImpl.detectRateSpam: a chat message or command adds 20 to the
    /// guest's throttler; over the threshold, a guest who is not an operator is kicked.
    pub(in crate::game::server_side) fn detect_rate_spam(&mut self, conn: u32, command: bool) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let threshold = if command {
            host.hub.command_spam_threshold
        } else {
            host.hub.chat_spam_threshold
        };
        let uuid = host.hub.guests[i].uuid;
        let play = host.play.entry(conn).or_default();
        let count = if command {
            &mut play.command_spam
        } else {
            &mut play.chat_spam
        };
        *count = count.saturating_add(20);
        // TickThrottler.isUnderThreshold: a threshold of 0 or less never kicks.
        let under = threshold <= 0 || i64::from(*count) < i64::from(threshold);
        if !under && !host.hub.ops.contains_key(&uuid) {
            host.hub.kick(
                conn,
                sunrise_protocol::text::translatable("disconnect.spam", &[]),
            );
        }
    }

    /// handleChatCommand (performUnsignedChatCommand): refused when the
    /// server enforces secure chat and the command has a message to sign.
    pub(in crate::game::server_side) fn guest_chat_command(&mut self, conn: u32, command: &str) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        if !host.hub.try_handle_chat(i, command, true) {
            return;
        }
        let permission = host.hub.guests[i].permission;
        let secure = host.hub.enforces_secure_chat();
        if secure && !self.signable_arguments(permission, command).is_empty() {
            let Some(host) = self.server.lan.as_ref() else {
                return;
            };
            let name = host.hub.guests[i].name.clone();
            self.lan_log_event(HostEvent::Error(format!(
                "Received unsigned command packet from {name}, but the command requires signable arguments: {command}"
            )));
            let failure = Text::translate("chat.disabled.invalid_command_signature", &[])
                .with_color(text::RED);
            if let Some(host) = self.server.lan.as_ref() {
                host.hub.guests[i].send(&C::SystemChat(play::SystemChat {
                    content: component_nbt(&failure.component()),
                    overlay: false,
                }));
            }
            self.detect_rate_spam(conn, true);
            return;
        }
        self.guest_command(conn, command, None);
        self.detect_rate_spam(conn, true);
    }

    /// handleSignedChatCommand: the acknowledgements, then each message
    /// argument through the sender's chain, then the command with them.
    pub(in crate::game::server_side) fn guest_signed_command(
        &mut self,
        conn: u32,
        p: play::ChatCommandSigned,
    ) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let mut events = Vec::new();
        let mut signed = None;
        let mut ran = false;
        if let Some(last_seen) = host.hub.apply_last_seen(i, &p.last_seen, &mut events)
            && host.hub.try_handle_chat(i, &p.command, true)
        {
            ran = true;
            let permission = host.hub.guests[i].permission;
            let parsed = self.signable_arguments(permission, &p.command);
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            match host
                .hub
                .signed_arguments(i, &p, &parsed, &last_seen, &mut events)
            {
                Ok(arguments) => signed = Some(arguments),
                Err(e) => host.hub.decode_failure(i, e, &mut events),
            }
        }
        for event in events {
            self.lan_log_event(event);
        }
        if let Some(signed) = signed {
            self.guest_command(conn, &p.command, Some(signed));
        }
        // The lambda counts the command even when its arguments failed to decode.
        if ran {
            self.detect_rate_spam(conn, true);
        }
    }

    /// A public chat line, once `server_chat` has left it: `chat_recipient` is asked of each
    /// receiver (`shown` is what players read), the line goes to everyone it keeps nobody from,
    /// and the log keeps it.
    pub fn lan_broadcast_chat(
        &mut self,
        (sender, name): (u128, &str),
        shown: &str,
        line: Text,
        bound: &play::ChatTypeBound,
        delivery: ChatDelivery<'_>,
    ) {
        let key = (sender, shown.to_owned());
        if let Some(active) = &self.server.chat_broadcasts.active {
            if active.contains(&key) {
                log::warn!("chat broadcast cycle refused: {shown}");
                return;
            }
            let mut path = active.clone();
            path.push(key);
            self.server
                .chat_broadcasts
                .queue
                .push_back(QueuedBroadcast {
                    sender,
                    name: name.to_owned(),
                    shown: shown.to_owned(),
                    line,
                    bound: bound.clone(),
                    delivery: match delivery {
                        ChatDelivery::Player(message) => QueuedDelivery::Player(message.clone()),
                        ChatDelivery::Disguised(content) => {
                            QueuedDelivery::Disguised(content.clone())
                        }
                    },
                    path,
                });
            return;
        }
        self.server.chat_broadcasts.active = Some(vec![key]);
        self.deliver_chat_broadcast((sender, name), shown, line, bound, delivery);
        while let Some(next) = self.server.chat_broadcasts.queue.pop_front() {
            let QueuedBroadcast {
                sender,
                name,
                shown,
                line,
                bound,
                delivery,
                path,
            } = next;
            self.server.chat_broadcasts.active = Some(path);
            let delivery = match &delivery {
                QueuedDelivery::Player(message) => ChatDelivery::Player(message),
                QueuedDelivery::Disguised(content) => ChatDelivery::Disguised(content),
            };
            self.deliver_chat_broadcast((sender, &name), &shown, line, &bound, delivery);
        }
        self.server.chat_broadcasts.active = None;
    }

    fn deliver_chat_broadcast(
        &mut self,
        (sender, name): (u128, &str),
        shown: &str,
        line: Text,
        bound: &play::ChatTypeBound,
        delivery: ChatDelivery<'_>,
    ) {
        let secure = match delivery {
            ChatDelivery::Player(message) => {
                message.trusted(sunrise_sim::lan::hub::chat::now_millis())
            }
            ChatDelivery::Disguised(_) => false,
        };
        self.record_public_chat(line, secure);
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        // Keep join order and release the hub before handing out a mutable Host.
        // Only identities are saved: eligibility is checked at the actual delivery site.
        let connections: Vec<_> = host.hub.guests.simulation_iter().map(|g| g.conn).collect();
        let listened = mod_api::__private::listened(mod_api::HookId::ChatRecipient);
        for conn in connections {
            let Some((_, recipient)) = self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.chat_recipient(conn))
            else {
                continue;
            };
            if listened {
                let mut hidden = false;
                self.native_chat_recipient(sender, name, shown, recipient, &mut hidden);
                if hidden {
                    continue;
                }
            }
            if let Some(host) = self.server.lan.as_mut()
                && let Some((index, current)) = host.hub.chat_recipient(conn)
                && current == recipient
            {
                match delivery {
                    ChatDelivery::Player(message) => host.hub.send_chat(index, message, bound),
                    ChatDelivery::Disguised(content) => {
                        host.hub
                            .send_disguised(index, content.clone(), bound.clone())
                    }
                }
            }
        }
    }

    /// `SayCommand` / `EmoteCommands`: a command source's visible message is sent as
    /// player chat, including to its source, never as the command's feedback.
    /// False, with nothing sent, when the registry lacks the chat type: `ChatType.bind`
    /// throws and the command fails (`Commands.performCommand`).
    pub(in crate::game::server_side) fn lan_broadcast_command_chat(
        &mut self,
        sender: u128,
        name: &Text,
        id: &str,
        message: &Text,
        mut signed: Option<ChatMessage>,
    ) -> bool {
        let (chat_type, key) = if id == "say" {
            ("say_command", "chat.type.announcement")
        } else {
            ("emote_command", "chat.type.emote")
        };
        let line = Text::translate(key, &[name.clone(), message.clone()]);
        let Some(host) = self.server.lan.as_ref() else {
            self.record_public_chat(line, false);
            return true;
        };
        let Some(chat_type) = host.hub.chat_type_id(chat_type) else {
            return false;
        };
        let bound = play::ChatTypeBound {
            chat_type: chat_type.network_id() as i32,
            name: component_nbt(&name.component()),
            target: None,
        };
        let content = component_nbt(&message.component());
        let delivery = match &mut signed {
            Some(signed) => {
                if *message != Text::literal(signed.content.as_str()) {
                    signed.unsigned_content = Some(content);
                }
                ChatDelivery::Player(signed)
            }
            None => ChatDelivery::Disguised(&content),
        };
        self.lan_broadcast_chat(
            (sender, &name.plain()),
            &message.plain(),
            line,
            &bound,
            delivery,
        );
        true
    }

    /// `Commands.performCommand`'s catch: a command that threw fails with
    /// `command.failed`, sent to the guest in red.
    fn guest_command_failed(&self, i: usize) {
        if let Some(host) = self.server.lan.as_ref() {
            let failure = Text::translate("command.failed", &[]).with_color(text::RED);
            host.hub.guests[i].send(&C::SystemChat(play::SystemChat {
                content: component_nbt(&failure.component()),
                overlay: false,
            }));
        }
    }

    /// SayCommand, EmoteCommands and MsgCommand for a guest: chat to the
    /// players, signed when the guest signed it (a disguised chat after an
    /// unsigned command packet). False when the command is another one, or
    /// one this can't run (selectors other than `@s` and `@a`), for the
    /// game's dispatcher to handle.
    pub fn guest_message_command(
        &mut self,
        conn: u32,
        command: &str,
        signed: Option<&Signed>,
    ) -> bool {
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return false;
        };
        let permission = host.hub.guests[i].permission;
        let parse = self.parse_as(permission, command);
        let Ok((context, id)) = parse.executable() else {
            return false;
        };
        let argument = if id == "me" { "action" } else { "message" };
        let (Some(Arg::Str(text)), true) =
            (context.get(argument), matches!(id, "say" | "me" | "msg"))
        else {
            return false;
        };
        let message = signed.and_then(|s| {
            s.iter()
                .find(|(n, _)| n == argument)
                .map(|(_, m)| m.clone())
        });
        if id != "msg" {
            // A redirect back to the root (e.g. `execute … run`) runs via the
            // dispatcher, which owns the modifiers and their resulting source(s).
            if parse.context.child.is_some() {
                return false;
            }
            // Resolve selectors as the packet's source, not the current server player.
            let Some((sender, name)) = self.command_guest(conn).and_then(|guest| {
                let host = self.server.lan.as_ref()?;
                let name = host.hub.player_display_name(guest.uuid, &guest.name);
                Some((
                    guest.uuid,
                    sunrise_game_data::scoreboard::comp_text(
                        &sunrise_game_data::scoreboard::tag_to_json(&name),
                    ),
                ))
            }) else {
                return true;
            };
            if let Some(text) =
                self.for_player(Target::Guest(conn), |game| game.resolve_chat_message(text))
            {
                if !self.lan_broadcast_command_chat(sender, &name, id, &text, message) {
                    self.guest_command_failed(i);
                }
            } else {
                log::warn!(
                    "Cannot resolve public chat command for disconnected player {sender:032x}"
                );
            }
            return true;
        }
        let Some(host) = self.server.lan.as_mut() else {
            return false;
        };
        let Some(Arg::Entity(selector)) = context.get("targets") else {
            return false;
        };
        let raw = context
            .args
            .iter()
            .rev()
            .find(|a| a.name == "targets")
            .and_then(|a| command.get(a.range.0..a.range.1));
        let guests = &host.hub.guests;
        let placed = |k: &usize| guests[*k].placed && guests[*k].participates_online();
        let targets: Vec<usize> = match (raw, &selector.player_name, selector.uuid) {
            (Some("@s"), ..) => vec![i],
            (Some("@a"), ..) => (0..guests.len()).filter(placed).collect(),
            (_, Some(name), _) => {
                let named: Vec<usize> = (0..guests.len())
                    .filter(|&k| placed(&k) && guests[k].name.eq_ignore_ascii_case(name))
                    .collect();
                if named.is_empty() {
                    // No player has that name: the one going by it as a nickname (`~` optional).
                    let bare = |s: &str| s.strip_prefix('~').unwrap_or(s).to_ascii_lowercase();
                    let wanted = bare(name);
                    let nicknamed = |k: &usize| {
                        host.hub
                            .nicknames
                            .get(&guests[*k].uuid)
                            .is_some_and(|nick| bare(nick) == wanted)
                    };
                    (0..guests.len())
                        .filter(|k| placed(k) && nicknamed(k))
                        .collect()
                } else {
                    named
                }
            }
            (_, None, Some(uuid)) => (0..guests.len())
                .filter(|&k| placed(&k) && guests[k].uuid == uuid)
                .collect(),
            _ => return false,
        };
        if targets.is_empty() {
            let failure = Text::empty()
                .append(Text::translate("argument.entity.notfound.player", &[]))
                .with_color(text::RED);
            host.hub.guests[i].send(&C::SystemChat(play::SystemChat {
                content: component_nbt(&failure.component()),
                overlay: false,
            }));
            return true;
        }
        if !host.hub.whisper(i, &targets, message, text) {
            self.guest_command_failed(i);
        }
        true
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
