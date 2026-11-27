//! Chat on the server side, as ServerGamePacketListenerImpl and PlayerList
//! handle it: chat sessions validated against the services' keys, signed
//! messages checked link by link, the acknowledged window, and messages
//! relayed with each connection's numbering and signature cache.
use super::{HostEvent, Hub, string};
use crate::lan::data::{ChatType, RegistryRef};
use crate::text::Text;
use sunrise_core::nbt::{JavaString, Tag};
use sunrise_protocol::{
    packets::play::{self, Clientbound as C},
    secure_chat::{self, Body, Chain, ChatSession, DecodeError, ProfileKey, Signature, Unpacked},
    text::{literal, translatable},
};

/// Now, in epoch milliseconds (Instant.now()).
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// StringUtil.isAllowedChatCharacter.
fn allowed(c: char) -> bool {
    c != '\u{a7}' && c >= ' ' && c != '\u{7f}'
}

/// A translatable message in red.
fn red(key: &str) -> Tag {
    let mut t = translatable(key, &[]);
    if let Tag::Compound(map) = &mut t {
        map.insert(
            JavaString::from("color"),
            Tag::String(JavaString::from("red")),
        );
    }
    t
}

/// PlayerChatMessage as the server relays it.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    pub sender: u128,
    /// The link's index in the sender's chain (0 when unsigned).
    pub index: i32,
    pub signature: Option<Signature>,
    pub content: String,
    /// Epoch milliseconds.
    pub timestamp: i64,
    pub salt: i64,
    pub last_seen: Vec<Signature>,
    /// What clients show instead of the signed text (withUnsignedContent).
    pub unsigned_content: Option<Tag>,
}

impl ChatMessage {
    /// The message a chain let through: a signed one keeps its body;
    /// PlayerChatMessage.unsigned takes the time now, no salt, nothing seen.
    pub fn new(sender: u128, unpacked: Unpacked, body: &Body, now: i64) -> Self {
        let (index, signature, timestamp, salt, last_seen) = match unpacked {
            Unpacked::Unsigned => (0, None, now, 0, Vec::new()),
            Unpacked::Signed { index, signature } => (
                index,
                Some(signature),
                body.timestamp,
                body.salt,
                body.last_seen.to_vec(),
            ),
        };
        Self {
            sender,
            index,
            signature,
            content: body.content.to_owned(),
            timestamp,
            salt,
            last_seen,
            unsigned_content: None,
        }
    }
    /// withUnsignedContent: what a mod's `native_chat_decorate` made of the
    /// message is what clients show, as unsigned content beside the signed
    /// body, which stays as the sender signed it (its content, signature,
    /// time, salt and last seen), so the signature still verifies. None
    /// leaves the message shown as signed.
    pub fn decorate(&mut self, decorated: Option<Text>) {
        self.unsigned_content =
            decorated.map(|line| crate::lan::data::component_nbt(&line.component()));
    }
    /// hasExpiredServer.
    pub fn expired(&self, now: i64) -> bool {
        now > self.timestamp + secure_chat::EXPIRES_AFTER_SERVER
    }
    /// PlayerList.verifyChatTrusted: signed and not expired.
    pub fn trusted(&self, now: i64) -> bool {
        self.signature.is_some() && !self.expired(now)
    }
}

impl Hub {
    /// MinecraftServer.enforceSecureProfile.
    pub fn enforces_secure_chat(&self) -> bool {
        self.lan.shared.config.enforces_secure_chat()
    }

    /// Component.getString in the server's language, for the log.
    pub(crate) fn english(&self, tag: &Tag) -> String {
        match &self.lan.shared.config.log {
            Some(log) => sunrise_protocol::text::plain(tag, &log.lang),
            None => sunrise_protocol::text::plain(tag, &Default::default()),
        }
    }

    /// tryHandleChat: an illegal character disconnects; with chat hidden a
    /// message (not a command) is refused. Whether to go on.
    pub fn try_handle_chat(&mut self, i: usize, message: &str, command: bool) -> bool {
        let conn = self.guests[i].conn;
        if !message.chars().all(allowed) {
            self.kick(
                conn,
                translatable("multiplayer.disconnect.illegal_characters", &[]),
            );
            return false;
        }
        // ChatVisiblity.HIDDEN.
        if !command && self.guests[i].info.chat_visibility == 2 {
            self.guests[i].send(&C::SystemChat(play::SystemChat {
                content: red("chat.disabled.options"),
                overlay: false,
            }));
            return false;
        }
        true
    }

    /// unpackAndApplyLastSeen: the acknowledged messages, or a disconnect.
    pub fn apply_last_seen(
        &mut self,
        i: usize,
        update: &play::LastSeenUpdate,
        events: &mut Vec<HostEvent>,
    ) -> Option<Vec<Signature>> {
        let result = self.guests.update(i, |g| {
            g.last_seen
                .apply_update(update.offset, update.acknowledged, update.checksum)
        });
        match result {
            Ok(seen) => Some(seen),
            // Vanilla logs the ValidationException's stack trace after the line.
            Err(_) => {
                let g = &self.guests[i];
                events.push(HostEvent::Error(format!(
                    "Failed to validate message acknowledgements from {}",
                    g.name
                )));
                let conn = g.conn;
                self.kick(
                    conn,
                    translatable("multiplayer.disconnect.chat_validation_failed", &[]),
                );
                None
            }
        }
    }

    /// handleChatAck.
    pub(super) fn chat_ack(&mut self, i: usize, offset: i32, events: &mut Vec<HostEvent>) {
        if self
            .guests
            .update(i, |g| g.last_seen.apply_offset(offset))
            .is_err()
        {
            let g = &self.guests[i];
            events.push(HostEvent::Error(format!(
                "Failed to validate message acknowledgement offset from {}",
                g.name
            )));
            let conn = g.conn;
            self.kick(
                conn,
                translatable("multiplayer.disconnect.chat_validation_failed", &[]),
            );
        }
    }

    /// handleMessageDecodeFailure: logged, and the reason in red for the player.
    pub fn decode_failure(&mut self, i: usize, e: DecodeError, events: &mut Vec<HostEvent>) {
        let reason = red(e.key());
        events.push(HostEvent::Warn(format!(
            "Failed to update secure chat state for {}: '{}'",
            self.guests[i].name,
            self.english(&reason)
        )));
        self.guests[i].send(&C::SystemChat(play::SystemChat {
            content: reason,
            overlay: false,
        }));
    }

    /// handleChat: the acknowledgements, then the message through the
    /// sender's chain, then to everyone.
    pub(super) fn handle_chat(&mut self, i: usize, c: play::Chat, events: &mut Vec<HostEvent>) {
        let Some(last_seen) = self.apply_last_seen(i, &c.last_seen, events) else {
            return;
        };
        if !self.try_handle_chat(i, &c.message, false) {
            return;
        }
        let (enforce, now) = (self.enforces_secure_chat(), now_millis());
        let body = Body {
            content: &c.message,
            timestamp: c.timestamp,
            salt: c.salt,
            last_seen: &last_seen,
        };
        let signature = c.signature.as_deref().and_then(secure_chat::signature);
        let (uuid, name, unpacked) = self.guests.update(i, |g| {
            let uuid = g.uuid;
            (
                uuid,
                g.name.clone(),
                g.chain
                    .unpack(uuid, signature.as_ref(), &body, enforce, now),
            )
        });
        let message = match unpacked {
            Ok(unpacked) => ChatMessage::new(uuid, unpacked, &body, now),
            Err(e) => return self.decode_failure(i, e, events),
        };
        if message.signature.is_some() && message.expired(now) {
            events.push(HostEvent::Warn(format!(
                "Received expired chat: '{}'. Is the client/server system time unsynchronized?",
                message.content
            )));
        }
        // ChatType.bind throws when the registry lacks the type; the chat
        // message chain logs the failure and drops this message (no
        // broadcast, no rate-spam count), and the player stays connected.
        let Some(chat_type) = self.chat_type_id("chat") else {
            events.push(HostEvent::Error(
                "Chain link failed, continuing to next one: minecraft:chat is not in the chat_type registry"
                    .to_owned(),
            ));
            return;
        };
        let bound = play::ChatTypeBound {
            chat_type: chat_type.network_id() as i32,
            name: self.player_display_name(uuid, &name),
            target: None,
        };
        // The game owns publication: ServerChatEvent, then PlayerList's log
        // and per-player delivery, even when no mod is listening.
        events.push(HostEvent::ChatSubmitted {
            message: Box::new(message),
            bound: Box::new(bound),
            name,
        });
    }

    /// Announces a new chat session to everyone. Vanilla runs this on the same
    /// `chatMessageChain` as chat, so a message that came first is relayed
    /// first. A message the game still has to hand to the mods
    /// (`HostEvent::ChatSubmitted`) is relayed when the game reaches it, so
    /// the announcement then waits behind it: clients would check that
    /// message against the new session and mark the sender's chain broken.
    pub(super) fn announce_chat_session(
        &mut self,
        update: play::PlayerInfoUpdate,
        events: &mut Vec<HostEvent>,
    ) {
        if events
            .iter()
            .any(|e| matches!(e, HostEvent::ChatSubmitted { .. }))
        {
            events.push(HostEvent::ChatSessionStarted(Box::new(update)));
        } else {
            self.broadcast(&C::PlayerInfoUpdate(update));
        }
    }

    /// handleChatSessionUpdate: a new profile key, validated against the
    /// services' keys, starts a new chain and is announced to everyone.
    pub(super) fn chat_session_update(
        &mut self,
        i: usize,
        p: play::ChatSessionUpdate,
        events: &mut Vec<HostEvent>,
    ) {
        let key = ProfileKey {
            expires_at: p.expires_at,
            key: p.key,
            signature: p.key_signature,
        };
        let g = &self.guests[i];
        let (conn, uuid) = (g.conn, g.uuid);
        let old = g.chain.session().map(|s| s.profile_key.expires_at);
        if g.chain.session().is_some_and(|s| s.profile_key == key) {
            return;
        }
        if old.is_some_and(|old| key.expires_at < old) {
            self.kick(
                conn,
                translatable("multiplayer.disconnect.expired_public_key", &[]),
            );
            return;
        }
        let secure = self.lan.shared.config.secure_chat.clone();
        let keys = secure.as_deref().map_or(&[][..], |s| s.keys());
        if keys.is_empty() {
            events.push(HostEvent::Warn(format!(
                "Ignoring chat session from {} due to missing Services public key",
                g.name
            )));
            return;
        }
        match ChatSession::validate(p.session, key, keys, uuid) {
            Ok(session) => {
                let data = session.data();
                self.guests.update(i, |g| g.chain = Chain::keyed(session));
                // resetPlayerChatState: INITIALIZE_CHAT to every player.
                self.announce_chat_session(
                    play::PlayerInfoUpdate {
                        actions: 2,
                        entries: vec![play::PlayerInfo {
                            uuid,
                            chat_session: Some(data),
                            ..Default::default()
                        }],
                    },
                    events,
                );
            }
            Err(reason) => {
                events.push(HostEvent::Error(
                    "Failed to validate profile key".to_owned(),
                ));
                self.kick(conn, translatable(reason, &[]));
            }
        }
    }

    /// Look up a live public-chat recipient just before deciding or sending their copy.
    pub fn chat_recipient(&self, conn: u32) -> Option<(usize, u128)> {
        let index = self.guests.connection_index(conn)?;
        let guest = &self.guests[index];
        (guest.placed
            && guest.participates_online()
            && !guest.disconnect_pending
            && guest.info.chat_visibility == 0)
            .then_some((index, guest.uuid))
    }

    /// ServerPlayer.sendChatMessage (players who show chat) and
    /// sendPlayerChatMessage: numbered for the connection, the last seen
    /// packed against its cache; a signed message joins the cache and the
    /// messages the player must acknowledge.
    pub fn send_chat(&mut self, k: usize, message: &ChatMessage, bound: &play::ChatTypeBound) {
        let overflow = self.guests.update(k, |g| {
            if g.info.chat_visibility != 0 {
                return false;
            }
            let last_seen = message
                .last_seen
                .iter()
                .map(|s| g.signatures.pack(s))
                .collect();
            g.send(&C::PlayerChat(play::PlayerChat {
                global_index: g.chat_index,
                sender: message.sender,
                index: message.index,
                signature: message.signature.map(|s| s.to_vec()),
                content: message.content.clone(),
                timestamp: message.timestamp,
                salt: message.salt,
                last_seen,
                unsigned_content: message.unsigned_content.clone(),
                filter: (0, Vec::new()),
                chat_type: bound.clone(),
            }));
            g.chat_index += 1;
            if let Some(signature) = &message.signature {
                g.signatures.push(&message.last_seen, Some(signature));
                g.last_seen.add_pending(signature);
                if g.last_seen.tracked_count() > 4096 {
                    return true;
                }
            }
            false
        });
        if overflow {
            self.kick(
                self.guests[k].conn,
                translatable("multiplayer.disconnect.too_many_pending_chats", &[]),
            );
        }
    }

    /// sendChatMessage of a system message: a disguised chat, to players
    /// who show chat.
    pub fn send_disguised(&self, k: usize, message: Tag, chat_type: play::ChatTypeBound) {
        let g = &self.guests[k];
        if g.info.chat_visibility == 0 {
            g.send(&C::DisguisedChat(play::DisguisedChat {
                message,
                chat_type,
            }));
        }
    }

    /// `PlayerList.broadcastSystemMessage` to everyone but `except`, whose
    /// copy travels with its command's captured output (as `/say`'s).
    pub fn system_message_except(&self, content: Tag, except: u32) {
        self.broadcast_except(
            except,
            &C::SystemChat(play::SystemChat {
                content,
                overlay: false,
            }),
        );
    }

    /// collectSignedArguments, or collectUnsignedArguments when the client
    /// signed none: each of the command's message arguments (`parsed`, by
    /// name) through the sender's chain.
    pub fn signed_arguments(
        &mut self,
        i: usize,
        p: &play::ChatCommandSigned,
        parsed: &[(String, String)],
        last_seen: &[Signature],
        events: &mut Vec<HostEvent>,
    ) -> Result<Vec<(String, ChatMessage)>, DecodeError> {
        let (enforce, now) = (self.enforces_secure_chat(), now_millis());
        self.guests.update(i, |g| {
        let uuid = g.uuid;
        let mut out = Vec::new();
        if p.arguments.is_empty() {
            for (name, value) in parsed {
                // SignedMessageBody.unsigned.
                let body = Body {
                    content: value,
                    timestamp: now,
                    salt: 0,
                    last_seen: &[],
                };
                let unpacked = g.chain.unpack(uuid, None, &body, enforce, now)?;
                out.push((name.clone(), ChatMessage::new(uuid, unpacked, &body, now)));
            }
            return Ok(out);
        }
        let mismatch = |events: &mut Vec<HostEvent>| {
            let names = |l: &mut dyn Iterator<Item = &String>| {
                l.map(String::as_str).collect::<Vec<_>>().join(", ")
            };
            events.push(HostEvent::Error(format!(
                "Signed command mismatch between server and client ('{}'): got [{}] from client, but expected [{}]",
                p.command,
                names(&mut p.arguments.iter().map(|a| &a.0)),
                names(&mut parsed.iter().map(|a| &a.0)),
            )));
            DecodeError::InvalidCommandSignature
        };
        for (name, signature) in &p.arguments {
            let Some((_, value)) = parsed.iter().find(|(n, _)| n == name) else {
                g.chain.set_broken();
                return Err(mismatch(events));
            };
            let body = Body {
                content: value,
                timestamp: p.timestamp,
                salt: p.salt,
                last_seen,
            };
            let signature = secure_chat::signature(signature);
            let unpacked = g
                .chain
                .unpack(uuid, signature.as_ref(), &body, enforce, now)?;
            out.push((name.clone(), ChatMessage::new(uuid, unpacked, &body, now)));
        }
        if parsed.iter().any(|(n, _)| !out.iter().any(|(o, _)| o == n)) {
            return Err(mismatch(events));
        }
        Ok(out)
        })
    }

    /// MsgCommand.sendMessage to each target: the outgoing line for the
    /// sender, the incoming one for the target. Nothing is logged. False,
    /// with nothing sent, when the registry lacks one of the two chat types:
    /// `ChatType.bind` throws before the first message goes out, and the
    /// command fails.
    pub fn whisper(
        &mut self,
        i: usize,
        targets: &[usize],
        message: Option<ChatMessage>,
        text: &str,
    ) -> bool {
        let (uuid, name) = (self.guests[i].uuid, self.guests[i].name.clone());
        let (Some(outgoing), Some(incoming)) = (
            self.chat_type_id("msg_command_outgoing"),
            self.chat_type_id("msg_command_incoming"),
        ) else {
            return false;
        };
        for &t in targets {
            let target = self.player_display_name(self.guests[t].uuid, &self.guests[t].name);
            let out = play::ChatTypeBound {
                chat_type: outgoing.network_id() as i32,
                name: self.player_display_name(uuid, &name),
                target: Some(target),
            };
            let inc = play::ChatTypeBound {
                chat_type: incoming.network_id() as i32,
                name: self.player_display_name(uuid, &name),
                target: None,
            };
            match &message {
                Some(m) => {
                    self.send_chat(i, m, &out);
                    self.send_chat(t, m, &inc);
                }
                None => {
                    self.send_disguised(i, literal(text), out);
                    self.send_disguised(t, literal(text), inc);
                }
            }
        }
        true
    }

    /// `ChatType.bind`'s lookup: a `ChatType` key's entry in the world's registry, by the
    /// network id the clients were sent (`SyncData::id`, the order of the sync), so a pack's
    /// replacement of the entry is the one they decorate with. None when the registry lacks the
    /// key, where `getOrThrow` throws: the vanilla pack is not required (`/datapack disable
    /// vanilla`) and a pack.mcmeta filter can hide a built-in chat type. Each caller does what
    /// the exception does to it in vanilla.
    pub fn chat_type_id(&self, name: &str) -> Option<RegistryRef<ChatType>> {
        let id = self
            .sync
            .id("minecraft:chat_type", &format!("minecraft:{name}"))?;
        Some(RegistryRef::new(id as u32))
    }
}

/// A yellow translation of player names (`multiplayer.player.joined`).
pub(super) fn yellow_names(key: &str, names: Vec<Tag>) -> Tag {
    Tag::compound([
        ("translate", string(key)),
        ("with", Tag::List(names)),
        ("color", string("yellow")),
    ])
}

impl Hub {
    /// PlayerList.broadcastSystemMessage.
    pub fn system_message(&self, content: Tag) {
        self.broadcast(&C::SystemChat(play::SystemChat {
            content,
            overlay: false,
        }));
    }
    /// A system message for the guests `hears` picks by name
    /// (`PlayerList.broadcastSystemToTeam` and its kin).
    pub fn system_message_to(&self, content: Tag, hears: impl Fn(&str) -> bool) {
        let body = C::SystemChat(play::SystemChat {
            content,
            overlay: false,
        })
        .encode();
        for g in self.guests.iter().filter(|g| g.placed && hears(&g.name)) {
            g.sender.send(body.clone());
        }
    }
    /// `ServerPlayer.sendSystemMessage` to the guest with `uuid`, in any dimension.
    pub fn system_message_to_player(&self, uuid: u128, content: Tag) {
        let body = C::SystemChat(play::SystemChat {
            content,
            overlay: false,
        })
        .encode();
        for g in self.guests.iter().filter(|g| g.placed && g.uuid == uuid) {
            g.sender.send(body.clone());
        }
    }
    /// `ServerPlayer.sendOverlayMessage` to every guest (the action bar).
    pub fn overlay_message(&self, content: Tag) {
        self.broadcast(&C::SystemChat(play::SystemChat {
            content,
            overlay: true,
        }));
    }
    /// A chat message from the host (a disguised chat: the host has no connection to sign with).
    pub fn chat(&self, sender: &str, text: &str) {
        // ChatType.bind throws without the type: nothing is sent.
        let Some(chat_type) = self.chat_type_id("chat") else {
            return;
        };
        self.broadcast(&C::DisguisedChat(play::DisguisedChat {
            message: literal(text),
            chat_type: play::ChatTypeBound {
                chat_type: chat_type.network_id() as i32,
                name: literal(sender),
                target: None,
            },
        }));
    }
    /// ListPlayersCommand's reply: the players in join order (the host of a
    /// LAN world first), as display names (Player.getDisplayName, with the
    /// team's colour and affixes) or names with ids, joined by
    /// gray commas (ComponentUtils.formatList).
    pub fn list_reply(&self, uuids: bool) -> Tag {
        let players: Vec<(u128, &str)> = self
            .online_players()
            .map(|g| (g.uuid, g.name.as_str()))
            .collect();
        let format = |(uuid, name): (u128, &str)| {
            if uuids {
                Tag::compound([
                    ("translate", string("commands.list.nameAndId")),
                    (
                        "with",
                        Tag::List(vec![
                            string(name),
                            string(&sunrise_protocol::crypto::uuid_string(uuid)),
                        ]),
                    ),
                ])
            } else {
                self.player_display_name(uuid, name)
            }
        };
        let names = match players.len() {
            0 => string(""),
            1 => format(players[0]),
            _ => {
                let mut extra = Vec::new();
                for (k, p) in players.iter().enumerate() {
                    if k > 0 {
                        extra.push(Tag::compound([
                            ("text", string(", ")),
                            ("color", string("gray")),
                        ]));
                    }
                    extra.push(format(*p));
                }
                Tag::compound([("text", string("")), ("extra", Tag::List(extra))])
            }
        };
        Tag::compound([
            ("translate", string("commands.list.players")),
            (
                "with",
                Tag::List(vec![
                    Tag::Int(players.len() as i32),
                    Tag::Int(self.lan.shared.max_players() as i32),
                    names,
                ]),
            ),
        ])
    }

    /// Disconnect a guest with a reason (ServerGamePacketListenerImpl.disconnect), once
    /// ([`Guest::begin_disconnect`]): a guest already on the way out keeps its first reason.
    pub fn kick(&mut self, conn: u32, reason: Tag) {
        let text = self.english(&reason);
        let started = self.update_guest(conn, |g| {
            g.begin_disconnect(text.clone(), reason)
                .then(|| (g.uuid, g.name.clone()))
        });
        if let Some((player, name)) = started.flatten() {
            self.pending_kicks.push(HostEvent::Kicked {
                conn,
                player,
                name,
                reason: text,
            });
        }
    }

    /// `TransferCommand.transfer`: the guest's client is handed to another
    /// server (`ClientboundTransferPacket`, the play state's `Transfer`); the
    /// client closes the connection and logs in there. Nothing disconnects the
    /// player here: as in vanilla, the close comes from the client.
    pub fn transfer(&self, conn: u32, host: &str, port: i32) {
        self.send_to(
            conn,
            &C::Transfer(sunrise_protocol::packets::configuration::Transfer {
                host: host.to_owned(),
                port,
            }),
        );
    }
}

#[cfg(test)]
mod decorate_tests {
    use super::*;

    /// A decorated message keeps the body its sender signed and only adds the
    /// line clients show; an undecorated one is shown as signed.
    #[test]
    fn decorating_keeps_the_signed_body() {
        let seen = [[7u8; secure_chat::SIGNATURE_BYTES]];
        let body = Body {
            content: "hello",
            timestamp: 1_000,
            salt: 42,
            last_seen: &seen,
        };
        let signed = Unpacked::Signed {
            index: 3,
            signature: [9u8; secure_chat::SIGNATURE_BYTES],
        };
        let mut message = ChatMessage::new(1, signed, &body, 2_000);
        let before = message.clone();
        message.decorate(None);
        assert_eq!(message, before);
        let line = Text::literal("[VIP] hello");
        message.decorate(Some(line.clone()));
        assert_eq!(
            message.unsigned_content,
            Some(crate::lan::data::component_nbt(&line.component()))
        );
        assert_eq!(
            ChatMessage {
                unsigned_content: None,
                ..message
            },
            before
        );
    }
}
