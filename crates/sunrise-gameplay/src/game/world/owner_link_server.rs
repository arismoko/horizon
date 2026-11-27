use serde_json::json;
use std::sync::Arc;
use sunrise_gameplay_common::game::blockctl::BlockctlGameExt1;
use sunrise_gameplay_common::game::{
    lan_common::LanSettings,
    owner_link::{ControlRequest, OwnerRequest, Snapshot},
};

#[cfg(feature = "test-support")]
pub type ClientHandoff<'a> = dyn FnMut(&mut crate::game::ServerGame<'_>) + 'a;

#[cfg(feature = "test-support")]
type TestTask = Box<dyn FnOnce(&mut crate::game::ServerGame<'_>, &mut ClientHandoff<'_>) + Send>;

#[cfg(feature = "test-support")]
pub fn test_owner_request(
    task: impl FnOnce(&mut crate::game::ServerGame<'_>, &mut ClientHandoff<'_>) + Send + 'static,
) -> OwnerRequest {
    let task: TestTask = Box::new(task);
    OwnerRequest::TestTask(Box::new(task))
}

impl crate::game::ServerGame<'_> {
    /// Answers one request of the window, on the server thread.
    pub fn serve_owner_request(&mut self, request: OwnerRequest) {
        self.dispatch_owner_request(
            request,
            #[cfg(feature = "test-support")]
            &mut |_| panic!("no client handoff on the server thread"),
        );
    }

    #[inline(always)]
    pub(in crate::game) fn dispatch_owner_request(
        &mut self,
        request: OwnerRequest,
        #[cfg(feature = "test-support")] handoff: &mut ClientHandoff<'_>,
    ) {
        match request {
            OwnerRequest::Publish(settings, reply) => {
                let _ = reply.send(self.publish(settings));
            }
            OwnerRequest::Unpublish(reply) => {
                let _ = reply.send(self.unpublish());
            }
            OwnerRequest::LanStatus(reply) => {
                let _ = reply.send(self.lan_status());
            }
            OwnerRequest::ServerOptions(reply) => {
                let _ = reply.send(self.server_options());
            }
            OwnerRequest::ApplyWorldOptions {
                before,
                wanted,
                done,
            } => {
                let outcome = self.apply_world_options_server(&before, &wanted);
                let _ = done.send(outcome);
            }
            OwnerRequest::LodSource(dim, reply) => {
                let _ = reply.send(self.lod_source_in(dim));
            }
            OwnerRequest::SetPregen(enabled) => self.set_pregen_commands(enabled),
            OwnerRequest::ReloadMods(reload) => self.apply_mods_reload(*reload),
            OwnerRequest::StartServerCapture(target, live, reply) => {
                let started = self.start_server_capture(target, live);
                match reply {
                    Some(reply) => {
                        let _ = reply.send(started);
                    }
                    None => {
                        if let Err(e) = started {
                            log::warn!("Sunrise: the server thread is not captured: {e}");
                        }
                    }
                }
            }
            OwnerRequest::StopServerCapture(reason, reply) => {
                self.stop_server_capture(reason, reply)
            }
            OwnerRequest::MarkServerCapture(name) => self.mark_server_capture(&name),
            OwnerRequest::Control(request) => self.serve_control_request(request),
            #[cfg(any())]
            OwnerRequest::InTest(task) => task(self),
            #[cfg(feature = "test-support")]
            OwnerRequest::TestTask(task) => {
                let task = task.downcast::<TestTask>().expect("owner test task type");
                task(self, handoff);
            }
        }
    }

    /// Answers one agent-control ask, on the server thread.
    fn serve_control_request(&mut self, request: ControlRequest) {
        match request {
            ControlRequest::Blocks { lo, hi, reply } => {
                let _ = reply.send(self.in_owner_level(|g| g.blocks_json(lo, hi)));
            }
            ControlRequest::Simulation(reply) => {
                let _ = reply.send(self.in_owner_level(|g| {
                    json!({ "game_time": g.world.game_time, "projectiles": g.projectiles_status() })
                }));
            }
            ControlRequest::HoldServer { held, done } => {
                self.server.hold_server = held;
                let _ = done.send(());
            }
            ControlRequest::ConsoleCommand { command, reply } => {
                let _ = reply.send(self.owner_console_command(&command));
            }
            ControlRequest::SetDayTime { time, done } => {
                self.common.world.day_time = time;
                let _ = done.send(());
            }
            ControlRequest::Summon {
                kind,
                at,
                baby,
                reply,
            } => {
                let id = self.in_owner_level(|g| {
                    g.summon_command_mob(&kind, Some(at), baby)
                        .map_err(|e| e.to_string())
                });
                let _ = reply.send(id);
            }
            ControlRequest::MobsAt {
                radius,
                center,
                reply,
            } => {
                let _ = reply.send(self.in_owner_level(|g| g.control_mobs_at(radius, center)));
            }
            ControlRequest::HurtLog(reply) => {
                let _ = reply.send(self.control_hurt_log());
            }
            ControlRequest::Save { all, reply } => {
                let _ = reply.send(self.control_save(all).map_err(|e| e.to_string()));
            }
            ControlRequest::BlockCommand { words, reply } => {
                let words: Vec<&str> = words.iter().map(String::as_str).collect();
                let out = self
                    .in_owner_level(|g| g.block_command_local(&words).map_err(|e| e.to_string()));
                let _ = reply.send(out);
            }
            ControlRequest::ServerSide(reply) => {
                let _ = reply.send(self.server_side());
            }
            ControlRequest::MemorySample(request) => request.collect(|| self.server_side()),
            ControlRequest::SetOp { name, level, reply } => {
                let _ = reply.send(self.lan_set_op(&name, level));
            }
            ControlRequest::WorldRoot(reply) => {
                let _ = reply.send(self.server.world_root.clone());
            }
            ControlRequest::SaveThenWait { saved, copied } => {
                let _ = saved.send(self.save_game().map_err(|e| e.to_string()));
                let _ = copied.recv();
            }
            ControlRequest::ShieldCooldown(reply) => {
                let out = self.in_owner_guest_turn(|g| g.shield_cooldown());
                let _ = reply.send(out);
            }
            ControlRequest::Sculk { words, reply } => {
                let words: Vec<&str> = words.iter().map(String::as_str).collect();
                let out = self
                    .in_owner_guest_turn(|g| g.sculk_command(&words).map_err(|e| e.to_string()))
                    .and_then(|res| res);
                let _ = reply.send(out);
            }
            ControlRequest::DragonHurt { amount, reply } => {
                let out = self.in_owner_guest_turn(|g| {
                    let _ = g.hit_special(
                        sunrise_gameplay_common::game::dragon_common::SPECIAL,
                        amount,
                        true,
                        sunrise_sim::mobs::DamageKind::PlayerAttack,
                        None,
                    );
                });
                let _ = reply.send(out);
            }
            ControlRequest::StatusExtras(reply) => {
                let out = self.in_owner_guest_turn(|g| (g.hook_status(), g.dragon_status()));
                let _ = reply.send(out);
            }
            ControlRequest::GiveLevels { levels, reply } => {
                let out = self.in_owner_guest_turn(|g| {
                    g.give_levels(levels);
                    g.experience.level
                });
                let _ = reply.send(out);
            }
            ControlRequest::StationEffect { effect, reply } => {
                let out = self.in_owner_guest_turn(|g| match effect {
                    None => {
                        let _ = g.remove_player_effects(None);
                    }
                    Some(inst) => {
                        let _ = g.apply_player_effect(inst);
                    }
                });
                if out.is_ok()
                    && let Some(host) = self.server.lan.as_mut()
                {
                    host.hub.sync_all_effects();
                }
                let _ = reply.send(out);
            }
        }
    }

    /// The command tree shows the pregen commands only while they are on.
    fn set_pregen_commands(&mut self, enabled: bool) {
        use sunrise_command::vanilla;
        if self.common.ui.dispatcher.0.find("pregen").is_some() != enabled {
            vanilla::set_sunrise_commands(enabled);
            self.common.ui.dispatcher =
                super::chat_common::SessionDispatcher(Arc::new(vanilla::session_dispatcher(
                    &self.common.level.rule_registry,
                    &self.common.level.enabled_features,
                )));
            self.restore_native_commands();
            self.send_native_command_report(true);
        }
    }

    pub(in crate::game::server_side) fn apply_world_options_server(
        &mut self,
        before: &Snapshot,
        wanted: &Snapshot,
    ) -> (bool, Option<u16>) {
        if wanted.difficulty != before.difficulty {
            let name = sunrise_sim::world_options::DIFFICULTIES[usize::from(wanted.difficulty) & 3];
            self.as_console(|g| g.run_command(&format!("difficulty {name}")));
        }
        if wanted.locked && !before.locked {
            self.common.level.difficulty_locked = true;
        }
        if wanted.default_mode != before.default_mode {
            self.set_default_game_mode(wanted.default_mode);
        }
        if wanted.personal_mode != before.personal_mode {
            self.set_owner_mode(wanted.personal_mode);
        }
        if wanted.cheats != before.cheats {
            self.set_allow_commands(wanted.cheats);
            self.owner_allow_commands(wanted.cheats);
        }
        let mut closed = false;
        let settings_changed =
            wanted.guest_commands != before.guest_commands || wanted.force != before.force;
        if wanted.lan != before.lan || (wanted.lan && wanted.port != before.port) {
            closed = self.unpublish();
            if wanted.lan {
                let _ = self.publish(LanSettings {
                    port: wanted.port,
                    guest_commands: wanted.guest_commands,
                    force_game_mode: wanted.force,
                });
            }
        } else if settings_changed {
            self.lan_apply_settings(wanted.guest_commands, wanted.force);
        }
        (closed, self.published_port())
    }
}
