use crate::game::ClientGame;
use sunrise_game_data::{Result, text::Text};
use sunrise_gameplay_common::game::lan_common::LanSettings;
use sunrise_sim::lan::{self};

impl ClientGame<'_> {
    pub(in crate::game) fn client_lan_open(&mut self, settings: LanSettings) -> Result<()> {
        let server = self
            .client
            .integrated
            .as_ref()
            .ok_or("no world of this game to open")?;
        let port = server
            .ask(|reply| {
                sunrise_gameplay_common::game::owner_link::OwnerRequest::Publish(settings, reply)
            })
            .ok_or(sunrise_gameplay_common::game::integrated_link::NO_ANSWER)??;
        self.client_system_message(Text::translate(
            "menu.multiplayerOptions.publish.started.lan",
            &[Text::literal(port.to_string())],
        ));
        Ok(())
    }

    pub(in crate::game) fn client_lan_close(&mut self, announce: bool) {
        let Some(server) = &self.client.integrated else {
            self.friends_lan_closed();
            return;
        };
        let closed = server
            .ask(sunrise_gameplay_common::game::owner_link::OwnerRequest::Unpublish)
            .unwrap_or(false);
        if closed {
            self.friends_lan_closed();
            if announce {
                self.client_system_message(Text::translate(
                    "menu.multiplayerOptions.publish.stopped",
                    &[],
                ));
            }
        }
    }

    /// `sunrise-ctl lan ...`.
    pub(in crate::game::client_side) fn lan_control(
        &mut self,
        words: &[&str],
    ) -> Result<Option<serde_json::Value>> {
        use serde_json::json;
        if words[0] != "lan" {
            return Ok(None);
        }
        if let Some(server) = &self.client.integrated
            && words.get(1) == Some(&"status")
        {
            // The hub is the server thread's.
            let status = server
                .ask(sunrise_gameplay_common::game::owner_link::OwnerRequest::LanStatus)
                .ok_or(sunrise_gameplay_common::game::integrated_link::NO_ANSWER)?;
            return Ok(Some(status));
        }
        match words.get(1).copied() {
            Some("open") => {
                if self.client.title.0.is_some() {
                    return Err("open a world first".into());
                }
                let port = words.get(2).map_or(Ok(0), |p| p.parse::<u16>())?;
                self.client_lan_open(LanSettings {
                    port: if port == 0 {
                        lan::available_port()
                    } else {
                        port
                    },
                    guest_commands: words.contains(&"commands"),
                    force_game_mode: words.contains(&"force"),
                })?;
                Ok(Some(json!({ "ok": true, "port": self.lan_port() })))
            }
            Some("close") => {
                self.client_lan_close(true);
                Ok(Some(json!({ "ok": true })))
            }
            Some("status") => Ok(Some(json!({ "open": false }))),
            _ => Err("lan open|close|status|op".into()),
        }
    }
    pub(in crate::game) fn lan_port(&self) -> Option<u16> {
        self.client
            .integrated
            .as_ref()
            .and_then(|server| server.published())
    }
}
