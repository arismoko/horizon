use super::*;
use sunrise_client::chat::{ChatSource, Tag as ChatTag};
use sunrise_gameplay_common::game::remote::chat::ChatTrust;
use sunrise_gameplay_common::game::remote::chat::Decoration;
use sunrise_gameplay_common::game::remote::chat::chat_type_decorate;

pub(in crate::game) trait ChatTrustClientExt: Sized {
    fn tag(&self) -> Option<ChatTag>;
}
impl ChatTrustClientExt for ChatTrust {
    /// `ChatTrustLevel.createTag`.
    fn tag(&self) -> Option<ChatTag> {
        match self {
            Self::Secure => None,
            Self::Modified(original) => Some(ChatTag::modified(original)),
            Self::NotSecure => Some(ChatTag::not_secure()),
        }
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// A line in the connection's chat, with its styles, click and hover
    /// events (ChatComponent.addMessage). The lines the Chat option hides
    /// are dropped there (its `visibleMessageFilter`).
    pub fn add_chat_line(&mut self, content: Text, source: ChatSource) -> bool {
        if !self.client.chat_ui.chat.shows(source) {
            return false;
        }
        self.push_chat_line(content, source, None, None);
        true
    }
    /// `ChatTrustLevel.evaluate` of a player message, with
    /// `ChatListener.evaluateTrustLevel`: the player's own message on their
    /// own server is secure. `decorated` is the acceptance-time chat decoration
    /// before applying the server's filter mask.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game::client_side) fn chat_trust(
        &self,
        sender: u128,
        signed: bool,
        timestamp: i64,
        signed_content: &str,
        unsigned_content: Option<&Tag>,
        decorated: &Text,
        received_ms: i64,
    ) -> ChatTrust {
        use sunrise_game_data::scoreboard::{comp_text, tag_to_json};
        if self.client.integrated.is_some()
            && self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.has_logical_player && r.profile == sender)
        {
            return ChatTrust::Secure;
        }
        // PlayerChatMessage.hasExpiredClient: MESSAGE_EXPIRES_AFTER_CLIENT is
        // the server's five minutes and two more.
        let expired = received_ms
            > timestamp.saturating_add(
                sunrise_protocol::secure_chat::EXPIRES_AFTER_SERVER + 2 * 60 * 1000,
            );
        if !signed || expired {
            return ChatTrust::NotSecure;
        }
        // ChatTrustLevel.isModified: the signed content is not in the line, or
        // the replacement uses a font other than the default.
        let font_changed = unsigned_content.is_some_and(|tag| {
            comp_text(&tag_to_json(tag)).spans.iter().any(|span| {
                span.style
                    .font
                    .as_deref()
                    .is_some_and(|font| font != "minecraft:default")
            })
        });
        if !decorated.plain().contains(signed_content) || font_changed {
            ChatTrust::Modified(signed_content.to_owned())
        } else {
            ChatTrust::Secure
        }
    }
    /// ChatType.Bound.decorate (`Decoration::Chat`) and `decorateNarration`
    /// (`Decoration::Narration`): the chat type's decoration from the
    /// server's registry, applied to `content`.
    pub(in crate::game::client_side) fn decorate_chat(
        &self,
        bound: &play::ChatTypeBound,
        content: &serde_json::Value,
        decoration: Decoration,
    ) -> sunrise_game_data::text::Text {
        use sunrise_game_data::scoreboard::comp_text;
        let Some(r) = self.client.remote.as_ref() else {
            return comp_text(content);
        };
        let chat_type = r.registries.data("minecraft:chat_type", bound.chat_type);
        comp_text(&chat_type_decorate(chat_type, decoration, bound, content))
    }
    /// `ChatListener.handleChatMessageError`: a validation failure queues a
    /// red italic error line and acknowledges the invalid signature on acceptance.
    pub(in crate::game::client_side) fn handle_chat_message_error(
        &mut self,
        sender: u128,
        chat_type: play::ChatTypeBound,
        signature: Option<Signature>,
    ) {
        self.enqueue_chat(QueuedChat {
            signature,
            sender,
            kind: QueuedChatKind::Error { chat_type },
        });
    }
    /// `ClientPacketListener.handleDeleteChat`: the packed signature names a
    /// message; one still held back by Chat Delay never shows
    /// (`ChatListener.removeFromDelayedMessageQueue`), a shown one is
    /// replaced (`ChatComponent.deleteMessage`). A signature the cache
    /// doesn't hold is an invalid packet.
    pub(in crate::game) fn handle_delete_chat(
        &mut self,
        packed: &(i32, Option<Vec<u8>>),
    ) -> Result<()> {
        let Some(r) = self.client.remote.as_mut() else {
            return Ok(());
        };
        let Some(signature) = r.signatures.unpack(packed) else {
            let reason = Text::translate("multiplayer.disconnect.invalid_packet", &[]);
            return self.leave_server(Some(reason));
        };
        r.secure_chat.ignore_pending(&signature);
        if !r.remove_queued_chat(&signature) {
            let ticks = self.visual_ticks();
            self.client
                .chat_ui
                .chat
                .delete_message(&self.client.gui.atlas, &signature, ticks);
        }
        Ok(())
    }
    /// Record a visible line, keeping a player message's signature for deletion.
    pub(in crate::game::client_side) fn push_chat_line(
        &mut self,
        content: Text,
        source: ChatSource,
        signature: Option<Signature>,
        tag: Option<ChatTag>,
    ) {
        let text = content.plain();
        let focused = self.client.screen == Screen::Chat;
        if source == ChatSource::Player {
            self.client.chat_ui.chat.add_signed(
                &self.client.gui.atlas,
                content,
                signature,
                tag,
                self.visual_ticks(),
                focused,
            );
        } else {
            // addServerSystemMessage and addClientSystemMessage both carry
            // GuiMessageTag.systemSinglePlayer: the grey bar at the line's left.
            let indicator = Some(sunrise_client::chat::SYSTEM_INDICATOR);
            self.client.chat_ui.chat.add(
                &self.client.gui.atlas,
                content,
                indicator,
                source,
                self.visual_ticks(),
                focused,
            );
        }
        if let Some(r) = self.client.remote.as_mut() {
            r.chat.push((text, Instant::now()));
            r.chat_received += 1;
            if r.chat.len() > 100 {
                r.chat.remove(0);
            }
        }
    }
    /// The server's last `n` chat lines, newest first.
    pub(in crate::game) fn remote_chat(&self, n: usize) -> Option<Vec<String>> {
        let r = self.client.remote.as_ref()?;
        Some(r.chat.iter().rev().take(n).map(|l| l.0.clone()).collect())
    }
    /// ClientPacketListener.sendChat / sendCommand, signed with a chat
    /// session (game/network/remote_chat.rs). The Chat option's Hidden and System
    /// levels keep chat out and Hidden keeps the commands out (ChatAbilities).
    pub(in crate::game) fn send_signed_chat_input(
        &mut self,
        message: &str,
    ) -> Option<sunrise_gameplay_common::game::native_server::ClientCommandOutcome> {
        if message.is_empty() || !self.chat_abilities().allows(message) {
            return None;
        }
        if let Some(command) = message.strip_prefix('/') {
            match self.send_command(command) {
                Ok(outcome) => return outcome,
                Err(error) => log::warn!("Command input could not be sent: {error}"),
            }
        } else if let Err(error) = self.send_chat(message) {
            log::warn!("Chat input could not be sent: {error}");
        }
        None
    }
    /// `ChatListener.showMessageToPlayer`, `handleDisguisedChatMessage` and
    /// `handleChatMessageError`: check the current restrictions before presentation.
    pub(in crate::game) fn show_queued_chat(&mut self, chat: QueuedChat) -> bool {
        let QueuedChat {
            signature,
            sender,
            kind,
        } = chat;
        let has_player = self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.has_logical_player);
        let source = ChatSource::Player;
        let visible = self.client.chat_ui.chat.shows(source);
        let (chat_type, mut line, narration, tag, info) = match kind {
            QueuedChatKind::Filtered => return false,
            QueuedChatKind::Player {
                chat_type,
                signed,
                timestamp,
                received_ms,
                only_show_secure,
                displayed,
                signed_content,
                unsigned_content,
                filter,
                info,
            } => {
                // `showMessageToPlayer`: the display, decorated with what receipt
                // picked, and the narration, `message.decoratedContent()`, are
                // decorated here, in the language of now; trust is evaluated with
                // the receipt time, against the line as it reads now
                // (`ChatTrustLevel.isModified`).
                let mut line = self.decorate_chat(&chat_type, &displayed, Decoration::Chat);
                let decorated = unsigned_content.as_ref().map_or_else(
                    || serde_json::Value::from(signed_content.as_str()),
                    sunrise_game_data::scoreboard::tag_to_json,
                );
                let trust = self.chat_trust(
                    sender,
                    signed,
                    timestamp,
                    &signed_content,
                    unsigned_content.as_ref(),
                    &line,
                    received_ms,
                );
                if (only_show_secure && matches!(trust, ChatTrust::NotSecure))
                    || self.social_hides(sender)
                {
                    return false;
                }
                let filtered = if filter.0 == 2 {
                    let Some(filtered) =
                        sunrise_client::chat::filtered_message(&signed_content, &filter)
                    else {
                        return false;
                    };
                    Some(filtered)
                } else {
                    None
                };
                if !has_player || !visible {
                    return false;
                }
                let narration = if let Some(filtered) = filtered {
                    line = self.decorate_chat(&chat_type, &filtered, Decoration::Chat);
                    self.decorate_chat(&chat_type, &filtered, Decoration::Narration)
                } else {
                    self.decorate_chat(&chat_type, &decorated, Decoration::Narration)
                };
                (chat_type, line, narration, trust.tag(), info)
            }
            QueuedChatKind::Disguised { chat_type, message } => {
                if !has_player || !visible {
                    return false;
                }
                let content = sunrise_game_data::scoreboard::tag_to_json(&message);
                (
                    chat_type.clone(),
                    self.decorate_chat(&chat_type, &content, Decoration::Chat),
                    self.decorate_chat(&chat_type, &content, Decoration::Narration),
                    Some(ChatTag::system()),
                    None,
                )
            }
            QueuedChatKind::Error { chat_type } => {
                // Validation errors use the player channel but no signature or mod event.
                if self.social_hides(sender) || !has_player || !visible {
                    return false;
                }
                let error = serde_json::json!({
                    "translate": "chat.validation_error",
                    "color": "red",
                    "italic": true,
                });
                let line = self.decorate_chat(&chat_type, &error, Decoration::Chat);
                self.push_chat_line(line, source, None, Some(ChatTag::chat_error()));
                self.say_system_chat_queued(&self.decorate_chat(
                    &chat_type,
                    &error,
                    Decoration::Narration,
                ));
                return true;
            }
        };
        if !self.chat_received(
            false,
            false,
            sender,
            Some(chat_type.chat_type),
            info.as_deref(),
            &mut line,
        ) {
            return false;
        }
        self.push_chat_line(line, source, signature, tag);
        self.say_chat_queued(&narration.plain());
        true
    }
}
