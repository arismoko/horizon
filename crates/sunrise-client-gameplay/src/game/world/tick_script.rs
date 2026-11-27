//! `SUNRISE_TICK_SCRIPT=PATH` (tools/test_wire_stream_parity.py): a server's client plays a tick-indexed
//! script, the one the vanilla client plays in `VanillaTickScript.java`, so the two clients' serverbound
//! streams can be compared tick by tick. The harness writes the scenario to PATH (JSON `{"name", "ticks",
//! "steps": {"TICK": [LINE...]}}`, LINE a sunrise-ctl line: `key w down`, `mouse left click`, `look 90 0`,
//! `click-slot 3`, `close`), then creates PATH with the extension `go`. From the next client tick the script
//! plays: its first tick sends `/wire-mark`, which marks it in the capture, then each tick's lines act
//! before the tick runs, as a player's input in the frame before it; when its ticks have run, the client
//! writes the scenario's name to PATH with the extension `done`.
use super::event_loop::EventLoopCtl;
use crate::game::ClientGame;
pub(in crate::game) use sunrise_client_gameplay::tick_script::{TickScript, parse};

impl ClientGame<'_> {
    /// Before a client tick: start the scenario the harness handed over, or play this tick's lines.
    pub(in crate::game::client_side) fn tick_script_step(&mut self, event_loop: &dyn EventLoopCtl) {
        let Some(path) = self
            .client
            .tick_script
            .path()
            .map(std::path::Path::to_path_buf)
        else {
            return;
        };
        if !self.client.tick_script.playing() {
            let go = path.with_extension("go");
            if self.client.remote.is_none() || !go.exists() {
                return;
            }
            let _ = std::fs::remove_file(&go);
            let Some(script) = std::fs::read_to_string(&path).ok().and_then(|t| parse(&t)) else {
                log::warn!("Sunrise: tick script {} is not a scenario", path.display());
                return;
            };
            self.client.tick_script.start(script);
            if self.chat_abilities().allows("/wire-mark") {
                let _ = self.send_server_command("wire-mark");
            }
        }
        let Some((lines, finished)) = self.client.tick_script.advance() else {
            return;
        };
        for line in lines {
            if !self.tick_script_click(&line) {
                self.replay_line(&line, event_loop);
            }
        }
        if let Some(name) = finished {
            self.client.tick_script.finish();
            let _ = std::fs::write(path.with_extension("done"), name);
        }
    }
}

impl ClientGame<'_> {
    /// `click-slot SLOT [right] [shift]` without a renderer: sunrise-ctl's version clicks where the
    /// slot is drawn, and `--headless` draws nothing. The vanilla driver clicks the menu itself
    /// (`MultiPlayerGameMode.handleContainerInput`), and so does this.
    fn tick_script_click(&mut self, line: &str) -> bool {
        let words: Vec<&str> = line.split_whitespace().collect();
        let (Some(&"click-slot"), Some(Ok(slot))) =
            (words.first(), words.get(1).map(|w| w.parse()))
        else {
            return false;
        };
        let input = if words.contains(&"shift") {
            sunrise_sim::menu::Input::QuickMove
        } else {
            sunrise_sim::menu::Input::Pickup
        };
        self.remote_menu_ops(&[(Some(slot), i32::from(words.contains(&"right")), input)]);
        true
    }
}
