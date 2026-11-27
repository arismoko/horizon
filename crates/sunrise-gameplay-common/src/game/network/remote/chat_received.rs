//! `ClientChatReceivedEvent` for the lines the server sends: the queued player
//! message and the hook that may replace or cancel a line.
use super::*;

/// Receipt-time context for `ChatListener.handleMessage`. Its presentation,
/// including trust evaluation, decoration, the mod hook, and checks against
/// current social/visibility options, waits until the queue accepts it.
pub struct QueuedChat {
    /// The valid player signature (also the deletion key), or the invalid
    /// signature an error only acknowledges. Disguised chat has neither.
    pub signature: Option<Signature>,
    /// The player's UUID, 0 for disguised chat (`Util.NIL_UUID`).
    pub sender: u128,
    pub kind: QueuedChatKind,
}

pub enum QueuedChatKind {
    Player {
        chat_type: play::ChatTypeBound,
        signed: bool,
        timestamp: i64,
        received_ms: i64,
        only_show_secure: bool,
        /// What the line shows, as `handlePlayerChatMessage` picks it at receipt
        /// (`displayedMessage.decoratedContent()`: the signed content under Only
        /// Show Secure Chat, else the unsigned content or the signed content). It
        /// translates when the line is decorated to be shown.
        displayed: serde_json::Value,
        /// `FilterMask.applyWithFormatting` and `ChatTrustLevel.isModified` use the signed text.
        signed_content: String,
        unsigned_content: Option<Tag>,
        filter: (i32, Vec<i64>),
        /// Saved only while a mod listens to `ClientChatReceivedEvent`.
        info: Option<Box<mod_api::PlayerChatInfo>>,
    },
    /// `FilterMask.FULLY_FILTERED`: stays in FIFO queue order with its deletion
    /// key and acknowledges as not shown on acceptance, without retaining text.
    Filtered,
    Disguised {
        chat_type: play::ChatTypeBound,
        message: Tag,
    },
    Error {
        chat_type: play::ChatTypeBound,
    },
}

/// `PlayerChatMessage` of a `ClientboundPlayerChatPacket`, for
/// `ClientChatReceivedEvent.Player`; `signed` when its signature validated.
pub fn player_chat_info(p: &play::PlayerChat, signed: bool) -> mod_api::PlayerChatInfo {
    use sunrise_game_data::scoreboard::{comp_text, tag_to_json};
    let filter = match p.filter.0 {
        1 => mod_api::ChatFilter::FullyFiltered,
        2 => mod_api::ChatFilter::Partial(p.filter.1.iter().map(|&w| w as u64).collect()),
        _ => mod_api::ChatFilter::PassThrough,
    };
    mod_api::PlayerChatInfo {
        body: p.content.clone(),
        signed,
        timestamp_ms: p.timestamp,
        salt: p.salt,
        // A line the API's text form cannot hold is left out.
        unsigned_content: p
            .unsigned_content
            .as_ref()
            .and_then(|t| sunrise_sim::api_text::to_api(&comp_text(&tag_to_json(t))).ok()),
        filter,
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    /// The packet to `PlayerChatInfo` conversion alone (the fake signature is
    /// never validated; `signed` is passed in): the body is what the player
    /// typed, the unsigned content the server's line, and a partial filter
    /// keeps its bitset words.
    #[test]
    fn a_signed_relayed_message_keeps_its_body_and_the_servers_line() {
        let p = play::PlayerChat {
            global_index: 0,
            sender: 7,
            index: 0,
            signature: Some(vec![1; 256]),
            content: "hello".into(),
            timestamp: 1_700_000_000_000,
            salt: -5,
            last_seen: Vec::new(),
            unsigned_content: Some(Tag::String("[mod] hello".into())),
            filter: (2, vec![-1, 3]),
            chat_type: play::ChatTypeBound {
                chat_type: 0,
                name: Tag::String("Steve".into()),
                target: None,
            },
        };
        let info = player_chat_info(&p, true);
        assert!(info.signed);
        assert_eq!(info.body, "hello");
        assert_eq!((info.timestamp_ms, info.salt), (1_700_000_000_000, -5));
        let unsigned = info.unsigned_content.expect("the server's line");
        let mod_api::text::TextContent::Literal(text) = &unsigned.content else {
            panic!("expected the server's literal line, got {unsigned:?}");
        };
        assert_eq!(text, "[mod] hello");
        assert_eq!(info.filter, mod_api::ChatFilter::Partial(vec![u64::MAX, 3]));
        let passed = play::PlayerChat {
            filter: (0, Vec::new()),
            ..p
        };
        assert_eq!(
            player_chat_info(&passed, false).filter,
            mod_api::ChatFilter::PassThrough
        );
    }
}
