use crate::game::ClientGame;
use serde_json::{Value, json};
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::integrated_link::NO_ANSWER;
use sunrise_gameplay_common::game::owner_link::{ControlRequest, OwnerRequest};

impl ClientGame<'_> {
    /// Block commands; None when `words[0]` is not one of them.
    pub(in crate::game) fn block_command(&mut self, words: &[&str]) -> Result<Option<Value>> {
        let ours = matches!(
            words.first(),
            Some(&("setblock" | "fill" | "getblock" | "container"))
        );
        // Single player on its server thread: the world is the server's, so
        // the command runs there, in the owner's level, and the client hears
        // of it like any change.
        if ours && let Some(server) = &self.client.integrated {
            let owned: Vec<String> = words.iter().map(|w| (*w).to_owned()).collect();
            return server
                .ask(|reply| {
                    OwnerRequest::Control(ControlRequest::BlockCommand {
                        words: owned,
                        reply,
                    })
                })
                .ok_or(NO_ANSWER)?
                .map_err(Into::into);
        }
        // Another server's world changes only through its own commands, which
        // it checks as the player's; `getblock` and `container` read what this
        // client has of it.
        if self.client.remote.is_some() && matches!(words.first(), Some(&("setblock" | "fill"))) {
            let command = words.join(" ");
            if self.chat_abilities().allows(&format!("/{command}")) {
                let _ = self.send_server_command(&command);
            }
            return Ok(Some(json!({ "ok": true, "sent": true })));
        }
        self.block_command_local(words)
    }
}

use sunrise_gameplay_common::game::blockctl::BlockctlGameExt1 as _;
