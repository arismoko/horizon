//! Recording a session's packets as `.mcpr` and playing them back in a `Game`: the `/studio record`
//! and `/studio replay` commands (chat and `sunrise-ctl`), the replay's clock and its world. This is
//! platform (docs/layers.md, "Replays"); Studio's editor (`studio.rs`) builds on it.

use super::Screen;
#[cfg(feature = "client")]
use crate::game::client_side::resource_loader::PendingWorldEntry;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use sunrise_client::{
    packet_source::{PacketSource, TimestampedPacket},
    studio::{Metadata, ReplayDriver},
};
use sunrise_game_data::{Result, text::Text};
use sunrise_protocol::{buf::Reader, packets::play::Clientbound, registry::SyncedNames};

/// Decode a recorded play packet against the same registry view as live play.
fn decode_recorded_packet(
    packet: &TimestampedPacket,
    names: Option<&SyncedNames<'_>>,
) -> sunrise_protocol::Result<Clientbound> {
    let reader = Reader::new(&packet.payload);
    let reader = match names {
        Some(names) => reader.with_names(names),
        None => reader,
    };
    Clientbound::decode_reader(packet.id, reader)
}

/// The packet replay a `Game` plays, if any, and its playback clock.
#[derive(Default)]
pub(in crate::game) struct Replay {
    /// The packet replay playing, if any.
    pub(in crate::game) driver: Option<Box<dyn PacketSource>>,
    /// Camera perspective to restore after leaving replay playback.
    pub(in crate::game) previous_camera_type: Option<sunrise_level::camera_math::CameraType>,
    /// Replay opened from the title screen owns the synthetic remote session.
    pub(in crate::game) standalone: bool,
    pub(in crate::game) last_tick: Option<std::time::Instant>,
    pub(in crate::game) virtual_tick_ms: f32,
}

impl Replay {
    /// A replay plays: the world shown is the recording's.
    pub(in crate::game) fn playing(&self) -> bool {
        self.driver.is_some()
    }
    /// How far the replay's player is between its last two ticks, while a replay plays: it
    /// moves on the replay's clock, not the game's.
    pub(in crate::game::client_side) fn player_alpha(&self) -> Option<f64> {
        self.playing()
            .then(|| (f64::from(self.virtual_tick_ms) / 50.0).clamp(0.0, 1.0))
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// How far the window's own player model is between two ticks: on the replay's clock while a
    /// replay plays, else the game's.
    pub(in crate::game::client_side) fn own_player_alpha(&self) -> f64 {
        self.client
            .replay
            .player_alpha()
            .unwrap_or_else(|| self.common.clock.camera_fraction())
    }

    /// In-game chat command handler: `/studio <record|replay> ...`.
    /// Returns true if the command was recognized and handled.
    pub(in crate::game::client_side) fn studio_command(&mut self, command: &str) -> bool {
        let Some(rest) = command.strip_prefix("studio") else {
            return false;
        };
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return false;
        }
        let words: Vec<&str> = rest.split_whitespace().collect();
        if words.is_empty() {
            self.client_system_message(Text::translate("sunrise.studio.replay.usage", &[]));
            return true;
        }

        match words[0] {
            "freecam" | "keyframe" | "path" if super::studio::EDITOR => {
                return self.studio_editor_command(&words);
            }
            "record" => match words.get(1).copied() {
                Some("start") => match self.start_studio_recording(words.get(2).copied()) {
                    Ok(v) if v["ok"].as_bool() == Some(true) => {
                        let path = v["path"].as_str().unwrap_or_default();
                        self.client_system_message(Text::tr(
                            "sunrise.studio.record.started",
                            &[&path],
                        ));
                    }
                    Ok(v) => {
                        let error = v["error"].as_str().unwrap_or_default();
                        self.client_system_message(Text::tr(
                            "sunrise.studio.record.error",
                            &[&error],
                        ));
                    }
                    Err(error) => {
                        self.client_system_message(Text::tr(
                            "sunrise.studio.record.error",
                            &[&error],
                        ));
                    }
                },
                Some("stop") => match self.stop_studio_recording() {
                    Ok(v) if v["ok"].as_bool() == Some(true) => {
                        let path = v["path"].as_str().unwrap_or_default();
                        self.client_system_message(Text::tr(
                            "sunrise.studio.record.saved",
                            &[&path],
                        ));
                    }
                    Ok(v) => {
                        let error = v["error"].as_str().unwrap_or_default();
                        self.client_system_message(Text::tr(
                            "sunrise.studio.record.error",
                            &[&error],
                        ));
                    }
                    Err(error) => {
                        self.client_system_message(Text::tr(
                            "sunrise.studio.record.error",
                            &[&error],
                        ));
                    }
                },
                Some("status") => {
                    let status = self.studio_recording_status();
                    if status["active"].as_bool() == Some(true) {
                        let elapsed =
                            (status["elapsed_ms"].as_u64().unwrap_or(0) / 1000).to_string();
                        let packets = status["packet_count"].as_u64().unwrap_or(0).to_string();
                        let size =
                            (status["bytes_written"].as_u64().unwrap_or(0) / 1024).to_string();
                        let path = status["path"].as_str().unwrap_or_default();
                        self.client_system_message(Text::tr(
                            "sunrise.studio.record.status.active",
                            &[&elapsed, &packets, &size, &path],
                        ));
                    } else if let Some(error) = status["error"].as_str() {
                        self.client_system_message(Text::tr(
                            "sunrise.studio.record.error",
                            &[&error],
                        ));
                    } else {
                        self.client_system_message(Text::translate(
                            "sunrise.studio.record.status.inactive",
                            &[],
                        ));
                    }
                }
                _ => {
                    self.client_system_message(Text::translate("sunrise.studio.record.usage", &[]))
                }
            },
            "replay" => match words.get(1).copied() {
                Some("open") => {
                    let Some(path) = words.get(2) else {
                        self.client_system_message(Text::translate(
                            "sunrise.studio.replay.open.usage",
                            &[],
                        ));
                        return true;
                    };
                    match self.open_studio_replay(path) {
                        Ok(v) if v["ok"].as_bool() == Some(true) => {
                            let duration_s = v["duration_ms"].as_f64().unwrap_or(0.0) / 1000.0;
                            let packets = v["packet_count"].as_u64().unwrap_or(0);
                            let duration = format!("{duration_s:.1}");
                            let packets = packets.to_string();
                            self.client_system_message(Text::tr(
                                "sunrise.studio.replay.opened",
                                &[path, &duration, &packets],
                            ));
                        }
                        Ok(v) => {
                            let err = v["error"].as_str().unwrap_or_default();
                            self.client_system_message(Text::tr(
                                "sunrise.studio.replay.open_error",
                                &[&err],
                            ));
                        }
                        Err(e) => {
                            let error = e.to_string();
                            self.client_system_message(Text::tr(
                                "sunrise.studio.replay.open_error",
                                &[&error],
                            ));
                        }
                    }
                }
                Some("pause") => {
                    if let Some(r) = self.client.replay.driver.as_mut() {
                        r.set_paused(true);
                        self.client_system_message(Text::translate(
                            "sunrise.studio.replay.paused",
                            &[],
                        ));
                    } else {
                        self.client_system_message(Text::translate(
                            "sunrise.studio.replay.inactive",
                            &[],
                        ));
                    }
                }
                Some("resume") => {
                    if let Some(r) = self.client.replay.driver.as_mut() {
                        r.set_paused(false);
                        self.client_system_message(Text::translate(
                            "sunrise.studio.replay.resumed",
                            &[],
                        ));
                    } else {
                        self.client_system_message(Text::translate(
                            "sunrise.studio.replay.inactive",
                            &[],
                        ));
                    }
                }
                Some("speed") => {
                    let Some(spd_str) = words.get(2) else {
                        self.client_system_message(Text::translate(
                            "sunrise.studio.replay.speed.usage",
                            &[],
                        ));
                        return true;
                    };
                    if let Ok(spd) = spd_str.parse::<f64>() {
                        let new_speed = if let Some(r) = self.client.replay.driver.as_mut() {
                            r.set_speed(spd);
                            Some(r.speed())
                        } else {
                            None
                        };
                        if let Some(speed) = new_speed {
                            let speed = format!("{speed:.2}");
                            self.client_system_message(Text::tr(
                                "sunrise.studio.replay.speed.set",
                                &[&speed],
                            ));
                        } else {
                            self.client_system_message(Text::translate(
                                "sunrise.studio.replay.inactive",
                                &[],
                            ));
                        }
                    } else {
                        self.client_system_message(Text::translate(
                            "sunrise.studio.replay.speed.invalid",
                            &[],
                        ));
                    }
                }
                Some("seek") => {
                    let Some(ms_str) = words.get(2) else {
                        self.client_system_message(Text::translate(
                            "sunrise.studio.replay.seek.usage",
                            &[],
                        ));
                        return true;
                    };
                    if let Ok(ms) = ms_str.parse::<u32>() {
                        if self.client.replay.driver.is_some() {
                            match self.seek_studio_replay(ms) {
                                Ok(()) => {
                                    let actual_ms = self
                                        .client
                                        .replay
                                        .driver
                                        .as_ref()
                                        .map_or(ms, |replay| ms.min(replay.duration_ms()));
                                    let ms_text = actual_ms.to_string();
                                    let seconds = format!("{:.1}", actual_ms as f32 / 1000.0);
                                    self.client_system_message(Text::tr(
                                        "sunrise.studio.replay.seeked",
                                        &[&ms_text, &seconds],
                                    ));
                                }
                                Err(e) => {
                                    if e.kind() == std::io::ErrorKind::Unsupported {
                                        self.client_system_message(Text::translate(
                                            "sunrise.studio.replay.seek.backward_requires_standalone",
                                            &[],
                                        ));
                                    } else {
                                        let error = e.to_string();
                                        self.client_system_message(Text::tr(
                                            "sunrise.studio.replay.seek.error",
                                            &[&error],
                                        ));
                                    }
                                }
                            }
                        } else {
                            self.client_system_message(Text::translate(
                                "sunrise.studio.replay.inactive",
                                &[],
                            ));
                        }
                    } else {
                        self.client_system_message(Text::translate(
                            "sunrise.studio.replay.seek.invalid",
                            &[],
                        ));
                    }
                }
                Some("status") => {
                    if let Some(r) = self.client.replay.driver.as_ref() {
                        let position = r.position_ms().to_string();
                        let duration = r.duration_ms().to_string();
                        let progress = format!("{:.1}", r.progress() * 100.0);
                        let speed = format!("{:.2}", r.speed());
                        let paused = r.is_paused().to_string();
                        let finished = r.is_finished().to_string();
                        self.client_system_message(Text::tr(
                            "sunrise.studio.replay.status",
                            &[&position, &duration, &progress, &speed, &paused, &finished],
                        ));
                    } else {
                        self.client_system_message(Text::translate(
                            "sunrise.studio.replay.inactive",
                            &[],
                        ));
                    }
                }
                Some("close") => {
                    if self.close_studio_replay() {
                        self.client_system_message(Text::translate(
                            "sunrise.studio.replay.closed",
                            &[],
                        ));
                    } else {
                        self.client_system_message(Text::translate(
                            "sunrise.studio.replay.inactive",
                            &[],
                        ));
                    }
                }
                _ => {
                    self.client_system_message(Text::translate("sunrise.studio.replay.usage", &[]));
                }
            },
            _ => {
                self.client_system_message(Text::translate("sunrise.studio.replay.usage", &[]));
            }
        }
        true
    }

    /// Reset playback tick accumulator and align positions on seek.
    pub(in crate::game::client_side) fn reset_studio_playback_ticks(&mut self) {
        self.client.replay.virtual_tick_ms = 0.0;
        if let Some(player) = &self.common.player {
            self.common.previous_position = player.position;
            if let Some(m) = self.client.player_model.as_mut() {
                m.previous = player.position;
                m.position = player.position;
                m.limb_speed = 0.0;
                m.previous_limb_speed = 0.0;
            }
        }
    }

    /// Reconstruct replay state at a target timestamp. Standalone replays can
    /// safely reset their synthetic remote; an in-world replay can only seek
    /// forward because its pre-replay world must remain intact.
    pub(in crate::game::client_side) fn seek_studio_replay(
        &mut self,
        target_ms: u32,
    ) -> std::io::Result<()> {
        let current_ms = self
            .client
            .replay
            .driver
            .as_ref()
            .map_or(0, |driver| driver.position_ms());
        let packets = if self.client.replay.standalone {
            let packets = self
                .client
                .replay
                .driver
                .as_mut()
                .expect("replay checked by caller")
                .seek_and_collect(target_ms)?;
            self.init_remote_for_replay()
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            packets
        } else {
            if target_ms < current_ms {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "backward seek requires a standalone replay",
                ));
            }
            self.client
                .replay
                .driver
                .as_mut()
                .expect("replay checked by caller")
                .seek_forward(target_ms)?
        };

        self.dispatch_studio_replay_packets(packets);
        self.reset_studio_playback_ticks();
        self.client.replay.last_tick = Some(std::time::Instant::now());
        Ok(())
    }

    fn dispatch_studio_replay_packets(&mut self, packets: Vec<TimestampedPacket>) {
        for pkt in packets {
            let names = self.client.remote.as_ref().map(|remote| SyncedNames {
                builtin: &remote.builtin,
                dynamic: &remote.registries,
            });
            match decode_recorded_packet(&pkt, names.as_ref()) {
                Ok(Clientbound::CustomPayload(payload))
                    if payload.channel == sunrise_protocol::attachment_sync::CHANNEL =>
                {
                    match sunrise_protocol::attachment_sync::decode_staged(payload) {
                        Ok(staged) => self.apply_staged_attachment_sync(staged),
                        Err(error) => log::debug!("Studio replay skipped attachment sync: {error}"),
                    }
                }
                Ok(packet) => {
                    let _ = self.handle_packet(packet);
                }
                Err(e) => {
                    let name = sunrise_protocol::ids::name(
                        sunrise_protocol::State::Play,
                        sunrise_protocol::Direction::Clientbound,
                        pkt.id,
                    )
                    .unwrap_or("?");
                    log::debug!(
                        "Studio replay skipped packet {name} (id {:#x}): {e}",
                        pkt.id
                    );
                }
            }
        }
    }

    /// Advance active replay playback by elapsed frame time and dispatch ready packets.
    pub(in crate::game::client_side) fn advance_studio_replay(&mut self) -> Result<()> {
        let Some(driver) = self.client.replay.driver.as_mut() else {
            self.client.replay.last_tick = None;
            return Ok(());
        };

        let now = std::time::Instant::now();
        let delta = match self.client.replay.last_tick {
            Some(last) => now
                .saturating_duration_since(last)
                .min(Duration::from_millis(250)),
            None => Duration::from_millis(16),
        };
        self.client.replay.last_tick = Some(now);

        let ready_packets = match driver.advance(delta) {
            Ok(pkts) => pkts,
            Err(e) => {
                log::warn!("Studio replay advance error: {e}");
                return Ok(());
            }
        };

        self.dispatch_studio_replay_packets(ready_packets);
        let paused = self
            .client
            .replay
            .driver
            .as_ref()
            .is_some_and(|replay| replay.is_paused());
        if self.client.player_model.is_none() {
            self.tick_player_model();
        }
        if !paused {
            let speed = self
                .client
                .replay
                .driver
                .as_ref()
                .map_or(1.0, |r| r.speed() as f32)
                .max(0.1);
            self.client.replay.virtual_tick_ms += delta.as_secs_f32() * 1000.0 * speed;
            while self.client.replay.virtual_tick_ms >= 50.0 {
                self.client.replay.virtual_tick_ms -= 50.0;
                if let Some(player) = &self.common.player {
                    let current_pos = player.position;
                    let (dx, dz) = (
                        current_pos.x - self.common.previous_position.x,
                        current_pos.z - self.common.previous_position.z,
                    );
                    let walked = (dx * dx + dz * dz).sqrt() as f32;
                    if let Some(m) = self.client.player_model.as_mut() {
                        m.previous = self.common.previous_position;
                        m.position = current_pos;
                        m.previous_limb_speed = m.limb_speed;
                        let target_speed = (walked * 4.0).min(1.0);
                        m.limb_speed += (target_speed - m.limb_speed) * 0.4;
                        if m.limb_speed < 0.001 {
                            m.limb_speed = 0.0;
                        }
                        m.limb_position += m.limb_speed;
                        m.previous_yaw = m.yaw;
                        m.previous_head_yaw = m.head_yaw;
                        m.previous_head_pitch = m.head_pitch;
                        m.previous_body_yaw = m.body_yaw;
                        m.yaw = self.common.yaw;
                        m.head_yaw = self.common.yaw;
                        m.head_pitch = self.common.pitch;
                        let mut target = m.body_yaw;
                        if dx * dx + dz * dz > 0.0025 {
                            let walk = (dz.atan2(dx) as f32).to_degrees() - 90.0;
                            let facing =
                                ((self.common.yaw - walk + 180.0).rem_euclid(360.0) - 180.0).abs();
                            target = if 95.0 < facing && facing < 265.0 {
                                walk - 180.0
                            } else {
                                walk
                            };
                        }
                        let diff = (target - m.body_yaw + 180.0).rem_euclid(360.0) - 180.0;
                        m.body_yaw += diff * 0.3;
                        let body_diff =
                            (self.common.yaw - m.body_yaw + 180.0).rem_euclid(360.0) - 180.0;
                        if body_diff.abs() > 50.0 {
                            m.body_yaw += body_diff - body_diff.signum() * 50.0;
                        }
                        m.age = m.age.wrapping_add(1);
                    }
                    self.common.previous_position = current_pos;
                }
            }
        } else if let Some(m) = self.client.player_model.as_mut() {
            m.previous_limb_speed = m.limb_speed;
        }
        self.studio_replay_advanced();
        Ok(())
    }

    /// Control socket command handler for `studio`.
    pub(in crate::game::client_side) fn studio_control(
        &mut self,
        words: &[&str],
    ) -> Result<Option<Value>> {
        if words.first().copied() != Some("studio") {
            return Ok(None);
        }
        match words.get(1).copied() {
            Some("record") => match words.get(2).copied() {
                Some("start") => {
                    let name = words.get(3).copied();
                    let res = self.start_studio_recording(name)?;
                    Ok(Some(res))
                }
                Some("stop") => {
                    let res = self.stop_studio_recording()?;
                    Ok(Some(res))
                }
                Some("status") => {
                    let res = self.studio_recording_status();
                    Ok(Some(res))
                }
                _ => Ok(Some(json!({ "error": "unknown studio record subcommand" }))),
            },
            Some("replay") => match words.get(2).copied() {
                Some("open") => {
                    let path = words.get(3).ok_or("missing path for studio replay open")?;
                    let res = self.open_studio_replay(path)?;
                    Ok(Some(res))
                }
                Some("pause") => {
                    if let Some(r) = self.client.replay.driver.as_mut() {
                        r.set_paused(true);
                        Ok(Some(json!({ "ok": true, "paused": true })))
                    } else {
                        Ok(Some(json!({ "error": "no active replay" })))
                    }
                }
                Some("resume") => {
                    if let Some(r) = self.client.replay.driver.as_mut() {
                        r.set_paused(false);
                        Ok(Some(json!({ "ok": true, "paused": false })))
                    } else {
                        Ok(Some(json!({ "error": "no active replay" })))
                    }
                }
                Some("speed") => {
                    let spd_str = words.get(3).ok_or("missing speed value")?;
                    let spd: f64 = spd_str.parse().map_err(|_| "invalid speed")?;
                    if let Some(r) = self.client.replay.driver.as_mut() {
                        r.set_speed(spd);
                        Ok(Some(json!({ "ok": true, "speed": r.speed() })))
                    } else {
                        Ok(Some(json!({ "error": "no active replay" })))
                    }
                }
                Some("seek") => {
                    let ms_str = words.get(3).ok_or("missing seek timestamp")?;
                    let ms: u32 = ms_str.parse().map_err(|_| "invalid timestamp")?;
                    let pos = if let Some(r) = self.client.replay.driver.as_mut() {
                        r.seek(ms).map_err(|e| format!("{e}"))?;
                        Some(r.position_ms())
                    } else {
                        None
                    };
                    if let Some(pos) = pos {
                        self.reset_studio_playback_ticks();
                        Ok(Some(json!({ "ok": true, "position_ms": pos })))
                    } else {
                        Ok(Some(json!({ "error": "no active replay" })))
                    }
                }
                Some("status") => {
                    if let Some(r) = self.client.replay.driver.as_ref() {
                        Ok(Some(json!({
                            "ok": true,
                            "active": true,
                            "position_ms": r.position_ms(),
                            "duration_ms": r.duration_ms(),
                            "progress": r.progress(),
                            "speed": r.speed(),
                            "paused": r.is_paused(),
                            "finished": r.is_finished(),
                        })))
                    } else {
                        Ok(Some(json!({ "ok": true, "active": false })))
                    }
                }
                Some("close") => {
                    let closed = self.close_studio_replay();
                    Ok(Some(json!({ "ok": true, "closed": closed })))
                }
                _ => Ok(Some(json!({ "error": "unknown studio replay subcommand" }))),
            },
            _ => Ok(Some(json!({ "error": "unknown studio command" }))),
        }
    }

    pub(in crate::game::client_side) fn start_studio_recording(
        &mut self,
        name: Option<&str>,
    ) -> Result<Value> {
        let Some(r) = self.client.remote.as_ref() else {
            return Ok(json!({ "ok": false, "error": "no active world session" }));
        };
        if r.session.is_recording() {
            return Ok(json!({ "ok": false, "error": "recording already active" }));
        }
        let _ = std::fs::create_dir_all("recordings");
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let filename = match name {
            Some(n) if !n.is_empty() => format!("{}_{}.mcpr", n, ts),
            _ => format!("recording_{}.mcpr", ts),
        };
        let path = PathBuf::from("recordings").join(filename);
        let server_address = if r.session.address.is_empty() {
            "local".to_owned()
        } else {
            r.session.address.clone()
        };
        let is_singleplayer = server_address == "local";
        let meta = Metadata::new(
            server_address,
            is_singleplayer,
            sunrise_core::release::VERSION.into(),
        );
        match sunrise_client::studio::StudioRecorder::start(path.clone(), meta) {
            Ok(recorder) => {
                r.session.start_recording(recorder, |recorder| {
                    self.dump_world_snapshot_to_recorder(recorder);
                });
                Ok(json!({ "ok": true, "path": path.display().to_string() }))
            }
            Err(e) => Ok(json!({ "ok": false, "error": e.to_string() })),
        }
    }

    pub(in crate::game::client_side) fn stop_studio_recording(&mut self) -> Result<Value> {
        let Some(r) = self.client.remote.as_ref() else {
            return Ok(json!({ "ok": false, "error": "no active world session" }));
        };
        match r.session.stop_recording() {
            Some(Ok(path)) => Ok(json!({ "ok": true, "path": path.display().to_string() })),
            Some(Err(e)) => Ok(json!({ "ok": false, "error": e.to_string() })),
            None => Ok(json!({ "ok": false, "error": "no active recording" })),
        }
    }

    pub(in crate::game::client_side) fn studio_recording_status(&self) -> Value {
        if let Some(r) = self.client.remote.as_ref()
            && let Some(status) = r.session.recorder_status()
        {
            json!({
                "ok": true,
                "active": status.active,
                "path": status.path.display().to_string(),
                "elapsed_ms": status.elapsed.as_millis(),
                "packet_count": status.packet_count,
                "bytes_written": status.bytes_written,
            })
        } else {
            json!({ "ok": true, "active": false })
        }
    }

    pub(in crate::game::client_side) fn open_studio_replay(&mut self, path: &str) -> Result<Value> {
        if let Err(error) = self.save_studio_camera_track() {
            return Ok(json!({ "ok": false, "error": error.to_string() }));
        }
        // The recordings browser can be opened while the title screen is
        // available but the slower world tables are still loading. Replay
        // setup builds the normal client registries, which require `rest`.
        if self.client.remote.is_none() && self.common.res.rest.get().is_none() {
            let pending = PendingWorldEntry::StudioReplay(PathBuf::from(path));
            if self.defer_world_entry(pending)? {
                return Ok(json!({ "ok": true, "pending": true }));
            }
            self.common.join_world_rest()?;
        }

        // Validate and index the archive before replacing the title menu or active world. A
        // temp-file, metadata, or protocol error must leave the current screen usable.
        let driver = match ReplayDriver::open(Path::new(path)) {
            Ok(driver) => driver,
            Err(error) => {
                log::error!("Failed to open studio replay '{path}': {error}");
                return Ok(json!({ "ok": false, "error": error.to_string() }));
            }
        };

        let standalone = self.client.replay.driver.is_none() && self.client.remote.is_none();
        if self.client.remote.is_none()
            && let Err(error) = self.init_remote_for_replay()
        {
            log::error!("Failed to initialize studio replay session: {error}");
            return Ok(json!({ "ok": false, "error": error.to_string() }));
        }

        let duration_ms = driver.duration_ms();
        let packets = driver.packet_count();
        self.studio_replay_opened(&driver);
        if self.client.replay.driver.is_none() {
            self.client.replay.previous_camera_type = Some(self.client.camera_type);
            self.client.replay.standalone = standalone;
        }
        self.client.replay.driver = Some(Box::new(driver));
        self.client.camera_type = sunrise_level::camera_math::CameraType::FirstPerson;
        self.client.title.0 = None;
        self.client.screen = Screen::Playing;
        self.commit_screen_transition();
        self.capture(false);
        // Apply timestamp-zero login, player pose, and chunk packets before the first rendered
        // frame so playback opens at the recorded player's viewpoint.
        let _ = self.advance_studio_replay();
        self.reset_studio_playback_ticks();
        log::info!("Studio: Opened replay '{path}' ({duration_ms} ms, {packets} packets)");
        Ok(json!({
            "ok": true,
            "duration_ms": duration_ms,
            "packet_count": packets,
        }))
    }
    pub(in crate::game::client_side) fn close_studio_replay(&mut self) -> bool {
        self.studio_replay_closing();
        let closed = self.client.replay.driver.take().is_some();
        if !closed {
            return false;
        }

        // A standalone replay owns a synthetic Remote connection. Treat that
        // connection as replay state even if the saved origin flag was lost;
        // sending it through leave_server() opens the Multiplayer screen.
        let standalone = self.client.replay.standalone
            || self
                .client
                .remote
                .as_ref()
                .is_some_and(|remote| remote.brand == "sunrise-studio");
        self.client.replay.standalone = false;
        if let Some(camera_type) = self.client.replay.previous_camera_type.take() {
            self.client.camera_type = camera_type;
        }
        self.client.replay.last_tick = None;
        self.studio_replay_closed();
        self.client.replay.virtual_tick_ms = 0.0;
        self.capture(false);
        if standalone {
            self.reset_client_item_use();
            if let Some(mut remote) = self.client.remote.take() {
                remote.session.disconnect();
            }
            self.common.using = None;
            if let Err(error) = self.finish_close_world() {
                log::error!("Failed to close standalone Studio replay cleanly: {error}");
            }
            self.reset_studio_hand_state();
            // A title-screen replay has no previous world perspective to
            // restore. Start the next world in first person with visible hands.
            self.client.camera_type = sunrise_level::camera_math::CameraType::FirstPerson;
            if let Some(title) = self.client.title.0.as_mut() {
                title.activate_mode(sunrise_client::title::Mode::Main);
            }
        } else if self.client.integrated.is_none() {
            if let Err(error) = self.close_world() {
                log::error!("Failed to close Studio replay world cleanly: {error}");
            }
            if let Some(title) = self.client.title.0.as_mut() {
                title.activate_mode(sunrise_client::title::Mode::Main);
            }
        }
        closed
    }
    /// Dump current player pose, dimension login, center chunk, and resident loaded chunks
    /// into the recording stream at timestamp 0ms so replays accurately reconstruct the world.
    pub(in crate::game::client_side) fn dump_world_snapshot_to_recorder(
        &self,
        recorder: &sunrise_client::studio::StudioRecorder,
    ) {
        let Some(r) = self.client.remote.as_ref() else {
            return;
        };
        use sunrise_protocol::packets::play::Clientbound;
        let record_cb = |packet: Clientbound| {
            let encoded = match packet.try_encode() {
                Ok(encoded) => encoded,
                Err(error) => {
                    if let Clientbound::LevelChunkWithLight(chunk) = &packet {
                        log::warn!(
                            "Studio Recorder: could not encode snapshot chunk ({}, {}): {error}",
                            chunk.x,
                            chunk.z
                        );
                    } else {
                        log::warn!(
                            "Studio Recorder: could not encode snapshot {}: {error}",
                            packet.name()
                        );
                    }
                    return false;
                }
            };
            let mut reader = sunrise_protocol::buf::Reader::new(&encoded);
            let Ok(id) = reader.varint() else {
                return false;
            };
            recorder.record_packet_at(0, id, reader.rest());
            true
        };

        // 1. C::Login packet baseline
        let login = sunrise_protocol::packets::play::Clientbound::Login(
            sunrise_protocol::packets::play::Login {
                player_id: r.entity_id,
                hardcore: self.common.level.hardcore,
                levels: vec!["minecraft:overworld".into()],
                max_players: 20,
                chunk_radius: self.client.settings.render_distance,
                simulation_distance: self.client.settings.simulation_distance,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn: sunrise_protocol::packets::play::SpawnInfo {
                    dimension_type: 0,
                    dimension: "minecraft:overworld".into(),
                    seed: 0,
                    game_type: 0,
                    previous_game_type: None,
                    debug: false,
                    flat: false,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                },
                online_mode: false,
                enforces_secure_chat: false,
            },
        );
        record_cb(login);

        // 2. Center chunk and player position
        let (pos, yaw, pitch) = if let Some(p) = &self.common.player {
            (
                glam::DVec3::new(p.position.x, p.position.y, p.position.z),
                self.common.yaw,
                self.common.pitch,
            )
        } else {
            (glam::DVec3::ZERO, self.common.yaw, self.common.pitch)
        };
        let chunk_x = (pos.x.floor() as i32) >> 4;
        let chunk_z = (pos.z.floor() as i32) >> 4;

        let center_chunk = sunrise_protocol::packets::play::Clientbound::SetChunkCacheCenter(
            sunrise_protocol::packets::play::SetChunkCacheCenter {
                x: chunk_x,
                z: chunk_z,
            },
        );
        record_cb(center_chunk);

        let player_pos = sunrise_protocol::packets::play::Clientbound::PlayerPosition(
            sunrise_protocol::packets::play::PlayerPosition {
                teleport: 0,
                position: [pos.x, pos.y, pos.z],
                delta: [0.0, 0.0, 0.0],
                yaw,
                pitch,
                relatives: 0,
            },
        );
        record_cb(player_pos);

        // 3. Encode resident loaded chunks into LevelChunkWithLight packets
        let biome_names: Vec<String> = world_biomes!(self)
            .list
            .iter()
            .map(|b| b.name.clone())
            .collect();
        let registry: Vec<String> = r
            .registries
            .registries
            .get("minecraft:worldgen/biome")
            .map(|v| v.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_else(|| biome_names.clone());

        let encoder = sunrise_sim::lan::chunks::ChunkEncoder::with_signs(
            self.common.res.facts().states.iter().map(|s| {
                (
                    s.name.clone(),
                    s.fluid_level > 0 || s.waterlogged,
                    s.sign_form(),
                )
            }),
            &r.builtin,
            &biome_names,
            &registry,
            self.common.res.shared_items(),
        );

        let mut count = 0;
        for chunk in self.common.world.chunks.values() {
            let level_chunk = encoder.encode(chunk, &self.common.world.block_entities);
            count += usize::from(record_cb(Clientbound::LevelChunkWithLight(level_chunk)));
        }
        log::info!(
            "Studio Recorder: Dumped initial world snapshot ({count} chunks) at timestamp 0ms"
        );
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::nbt::Tag;
    use sunrise_protocol::{
        packets::play,
        registry::{Builtin, Dynamic, Ids},
    };

    #[test]
    fn recording_decodes_item_hover_using_the_replay_session_registry() {
        let title = Clientbound::SetTitleText(play::Text {
            text: Tag::compound([
                ("text", Tag::String("Recorded item".into())),
                (
                    "hover_event",
                    Tag::compound([
                        ("action", Tag::String("show_item".into())),
                        ("id", Tag::String("minecraft:stone".into())),
                    ]),
                ),
            ]),
        });
        let encoded = title.encode();
        let mut reader = Reader::new(&encoded);
        let packet = TimestampedPacket {
            timestamp_ms: 0,
            id: reader.varint().unwrap(),
            payload: reader.rest().to_vec(),
        };
        let builtin = Builtin {
            registries: [(
                "minecraft:item".into(),
                Ids::from_names(vec!["minecraft:stone".into()]),
            )]
            .into(),
        };
        let names = SyncedNames {
            builtin: &builtin,
            dynamic: &Dynamic::default(),
        };
        assert!(decode_recorded_packet(&packet, None).is_err());
        assert_eq!(
            decode_recorded_packet(&packet, Some(&names)).unwrap(),
            title
        );
    }
}
