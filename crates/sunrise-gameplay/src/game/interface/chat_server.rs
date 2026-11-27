use crate::game::ServerGame;
use std::sync::Arc;
use sunrise_command::Registries;
use sunrise_game_data::text::Text;
use sunrise_gameplay::world_clock::OVERWORLD_CLOCK;
use sunrise_gameplay::world_clock::WorldClock;
use sunrise_gameplay_common::game::chat_common::SessionDispatcher;
use sunrise_gameplay_common::game::chat_common::command_export;

impl ServerGame<'_> {
    pub(in crate::game) fn server_sound_event_names(&self) -> Vec<String> {
        // Server sound events come from command-registries.json, not client audio.
        for dir in [&self.common.options.resources, &self.common.ui.vanilla_root] {
            if let Some(export) = command_export(dir, self.common.options.dedicated.is_none()) {
                return export.sounds.clone();
            }
        }
        Vec::new()
    }
    pub(in crate::game) fn server_sync_pack_tag_ids(&mut self) {
        self.sync_pack_tag_ids();
    }
    pub(in crate::game) fn server_sync_hub_clocks(&mut self) {
        if self.server.lan.is_none() {
            return;
        }
        let snap = self.hub_clock_snapshot();
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        for (name, total, rate) in snap.others {
            host.hub.set_clock_state(&name, total, rate);
        }
        host.hub.overworld_day = snap.overworld_day;
        host.hub.level_clock = snap.level_clock;
    }
    /// Commands the game issues itself (the server console, harnesses, the
    /// control socket) run at the owner's level whatever Allow Commands says.
    pub fn as_console<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let level = std::mem::replace(&mut self.common.ui.registries.permission, 4);
        let out = f(self);
        self.common.ui.registries.permission = level;
        out
    }
    /// `ServerClockManager.ServerClockInstance.loadFrom`: restore the existing
    /// Overworld state when a world opens; clear the other command-state cache.
    /// The carrier store's open-time state settles here too, before the
    /// first tick and any player: a refusal fails the open.
    pub fn load_world_state(&mut self) -> sunrise_game_data::Result<()> {
        self.common.ui.dispatcher =
            SessionDispatcher(Arc::new(sunrise_command::vanilla::session_dispatcher(
                &self.common.level.rule_registry,
                &self.common.level.enabled_features,
            )));
        self.restore_native_commands();
        self.common.ui.clock = WorldClock::default();
        self.common.ui.end_clock = WorldClock::default();
        self.common.ui.command_clocks.clear();
        // PrimaryLevelData: the difficulty comes back from level.dat's difficulty_settings.
        self.common.ui.difficulty = self.common.level.difficulty_id();
        self.common.ui.active_clock = OVERWORLD_CLOCK.to_owned();
        if let Ok(bytes) = std::fs::read(
            self.server
                .world_root
                .join("data/minecraft/world_clocks.dat"),
        ) && let Ok(raw) = sunrise_world::compression::Compression::Gzip.decode(&bytes, 1 << 20)
            && let Ok((root, _)) =
                sunrise_core::nbt::read_named(&raw, sunrise_core::nbt::Limits::default())
            && let Some(sunrise_core::nbt::Tag::Compound(clocks)) = root.tag.get("data")
        {
            use sunrise_core::nbt::Tag;
            for (name, c) in clocks {
                let mut state = WorldClock::default();
                if let Some(Tag::Float(r)) = c.get("rate") {
                    state.rate = f32::from_bits(*r);
                }
                if let Some(Tag::Float(p)) = c.get("partial_tick") {
                    state.partial = f32::from_bits(*p);
                }
                if let Some(Tag::Byte(p)) = c.get("paused") {
                    state.paused = *p != 0;
                }
                let total = match c.get("total_ticks") {
                    Some(Tag::Long(t)) => *t,
                    _ => 0,
                };
                let name = name.to_string_lossy();
                if name == OVERWORLD_CLOCK {
                    self.common.ui.clock = state;
                } else if name == "minecraft:the_end" {
                    self.common.ui.end_clock = state;
                } else {
                    self.common.ui.command_clocks.insert(name, (total, state));
                }
            }
        }
        // The level the world opens in follows its own default clock; the Overworld's total
        // (level.dat's day time, already in World.day_time) goes to the table.
        let clock = self.common.level_clock_name(self.common.dim);
        self.common.world.day_time = self.common.switch_clock(&clock);
        self.build_registries();
        self.common.sync_rules();
        self.carrier_world_opened()
    }
    /// Save the clock beside level.dat (Level.write saves the game rules: a
    /// second writer of game_rules.dat could leave it with the rules loaded
    /// when the world opened).
    pub fn save_world_state(&mut self) {
        if self.server.world_root.as_os_str().is_empty() {
            return;
        }
        use sunrise_core::nbt::Tag;
        // Every clock the world has met is saved; the fields of a clock Sunrise does not model
        // stay as saved, and (like vanilla's codec) rate, partial tick and pause are omitted
        // while they have their defaults.
        let path = self
            .server
            .world_root
            .join("data/minecraft/world_clocks.dat");
        let existing = sunrise_sim::save::read_dat(&path);
        let mut data = match existing.as_ref().and_then(|r| r.get("data")) {
            Some(d @ Tag::Compound(_)) => d.clone(),
            _ => Tag::Compound(Default::default()),
        };
        let mut clocks: Vec<(String, i64, _)> = vec![(
            OVERWORLD_CLOCK.to_owned(),
            self.command_clock_time(OVERWORLD_CLOCK),
            self.ui.clock,
        )];
        clocks.push((
            "minecraft:the_end".to_owned(),
            self.common.command_clock_time("minecraft:the_end"),
            self.common.ui.end_clock,
        ));
        for (name, (_, state)) in &self.common.ui.command_clocks {
            if name != OVERWORLD_CLOCK && name != "minecraft:the_end" {
                clocks.push((name.clone(), self.common.command_clock_time(name), *state));
            }
        }
        for (name, total, c) in clocks {
            let mut clock = match data.get(&name) {
                Some(c @ Tag::Compound(_)) => c.clone(),
                _ => Tag::Compound(Default::default()),
            };
            if let Tag::Compound(map) = &mut clock {
                map.insert("total_ticks".into(), Tag::Long(total));
                for (key, value, default) in [
                    (
                        "partial_tick",
                        Tag::Float(c.partial.to_bits()),
                        c.partial == 0.0,
                    ),
                    ("rate", Tag::Float(c.rate.to_bits()), c.rate == 1.0),
                    ("paused", Tag::Byte(i8::from(c.paused)), !c.paused),
                ] {
                    if default {
                        map.remove(&key.into());
                    } else {
                        map.insert(key.into(), value);
                    }
                }
            }
            if let Tag::Compound(map) = &mut data {
                map.insert(name.as_str().into(), clock);
            }
        }
        let root = Tag::compound([
            ("data", data),
            ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
        ]);
        if let Err(e) = sunrise_sim::save::write_dat(&path, root) {
            log::warn!("the world clocks were not written: {e}");
        }
    }
    /// A server-side system message: captured, queued to the acting player, or logged.
    pub(in crate::game) fn server_system_message(&mut self, text: Text) {
        assert!(
            self.is_logical_server(),
            "server_system_message called on Client role"
        );
        if let Some(capture) = self.server.chat_capture.as_mut() {
            capture.push(text);
            return;
        }
        if self.server.lan_acting.is_some() {
            self.server.guest_packets.push(
                sunrise_protocol::packets::play::Clientbound::SystemChat(
                    sunrise_protocol::packets::play::SystemChat {
                        content: sunrise_sim::lan::data::component_nbt(&text.component()),
                        overlay: false,
                    },
                ),
            );
            return;
        }
        if self.common.is_dedicated_server() {
            super::dedicated::log("INFO", &format!("System chat: {}", text.plain()));
        } else {
            log::info!("System chat: {}", text.plain());
        }
    }
    /// Command suggestions include the world's currently named random sequences.
    pub(in crate::game::server_side) fn suggestion_registries(&mut self) -> &mut Registries {
        self.common.ui.registries.random_sequences = self.server.rand_sequences.ids();
        &mut self.common.ui.registries
    }
    pub(in crate::game) fn server_block_entity_nbt(
        &self,
        pos: [i32; 3],
    ) -> Option<sunrise_core::nbt::Tag> {
        self.block_entity_nbt(pos)
    }
    pub(in crate::game) fn server_send_chat_command(&mut self, _command: &str) {
        debug_assert!(false, "server never sends client chat commands");
    }
    pub(in crate::game) fn server_apply_item_splash(
        &mut self,
        splash: sunrise_sim::entities::ItemSplash,
    ) {
        if let sunrise_sim::entities::ItemSplash::Sound { at, volume, pitch } = splash {
            self.server_play_sound("minecraft:entity.generic.splash", at, volume, pitch);
        }
    }
    pub(in crate::game) fn server_level_min_y(&self) -> i32 {
        self.common.world.shape.min_y
    }
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::command_registries::CommandRegistriesGameExt1 as _;
