use super::{Arg, Context, R, Target, err};
use crate::game::commands::extra::ProfileFetch;
use crate::game::commands::extra::filename_date_time;
use crate::game::commands::extra::num;
use crate::game::commands::extra::tr_error;
use crate::game::commands::extra::two_places;
use std::time::Instant;
use sunrise_game_data::text::{Style, Text};
use sunrise_level::profile_lookup::{NameOrId, Resolve, ResolvedProfile};
use sunrise_protocol::packets::{
    Empty,
    play::{self, Clientbound as C, DialogHolder},
};
use sunrise_sim::lan::waypoints::Icon;

impl crate::game::ServerGame<'_> {
    /// Changes a player target's `postEffects`; whether `change` did, which
    /// sets `postEffectsDirty` for the packet.
    pub(in crate::game) fn change_post_effects(
        &mut self,
        t: Target,
        change: impl FnOnce(&mut Vec<String>) -> bool,
    ) -> bool {
        let conn = match t {
            Target::Guest(conn) => Some(conn),
            Target::Player => self.server.lan_acting,
            _ => return false,
        };
        match conn {
            Some(conn) => self
                .server
                .lan
                .as_mut()
                .and_then(|h| {
                    h.hub.update_guest(conn, |g| {
                        let changed = change(&mut g.post_effects);
                        g.post_effects_dirty |= changed;
                        changed
                    })
                })
                .unwrap_or(false),
            None => change(&mut self.server.cmds.0.post_effects),
        }
    }
    /// `ServerPlayer.closeDialog` for every target of `/dialog clear`.
    pub(in crate::game) fn clear_dialog_to(&mut self, targets: &[Target]) {
        assert!(
            self.is_logical_server(),
            "clear_dialog_to called on Client role"
        );
        let packet = C::ClearDialog(Empty);
        for &t in targets {
            match t {
                Target::Player if self.server.lan_acting.is_some() || self.server.lan.is_none() => {
                    self.server.guest_packets.push(packet.clone());
                }
                Target::Player => {}
                Target::Guest(conn) => {
                    if let Some(host) = self.server.lan.as_ref() {
                        host.hub.send_to(conn, &packet);
                    }
                }
                _ => {}
            }
        }
    }
    pub(in crate::game::server_side) fn cmd_debug(&mut self, ctx: &Context) -> R<i32> {
        if ctx.literals.iter().any(|l| l == "function") {
            return self.cmd_debug_function(ctx);
        }
        // MinecraftServer.isTimeProfilerRunning / startTimeProfiler /
        // stopTimeProfiler: a stop before the delayed start took effect
        // reports empty results and leaves the start pending.
        let running = self.server.cmds.0.profiler_delay || self.server.cmds.0.profiler.is_some();
        if ctx.literals.iter().any(|l| l == "start") {
            if running {
                return Err(err("commands.debug.alreadyRunning", &[]));
            }
            self.server.cmds.0.profiler_delay = true;
            self.feedback(Text::translate("commands.debug.started", &[]));
            return Ok(0);
        }
        if !running {
            return Err(err("commands.debug.notRunning", &[]));
        }
        let (seconds, ticks) = match self.server.cmds.0.profiler.take() {
            Some((started, ticks)) => (
                started.elapsed().as_nanos() as f64 / 1.0e9,
                (self.common.ticks - ticks) as f64,
            ),
            None => (0.0, 0.0),
        };
        let tps = ticks / seconds;
        self.feedback(Text::translate(
            "commands.debug.stopped",
            &[
                Text::literal(two_places(seconds)),
                num(ticks as i64),
                Text::literal(two_places(tps)),
            ],
        ));
        Ok(tps as i32)
    }
    /// `DebugCommand.TraceCustomExecutor`: run the functions with a tracer
    /// writing to `debug/debug-trace-<time>.txt`.
    pub(in crate::game::server_side) fn cmd_debug_function(&mut self, ctx: &Context) -> R<i32> {
        if !self.server.exec.0.functions.is_empty() {
            return Err(err("commands.debug.function.noRecursion", &[]));
        }
        let Some(Arg::Resource(id)) = ctx.get("name").cloned() else {
            return Ok(0);
        };
        let functions = self.functions_named(&id)?;
        let output = format!("debug-trace-{}.txt", filename_date_time());
        let dir = std::env::current_dir().unwrap_or_default().join("debug");
        let mut trace = String::new();
        let mut total = 0;
        let mut traced: Vec<String> = Vec::new();
        for f in &functions {
            let lines = match f.instantiate(None) {
                Ok(lines) => lines,
                Err((key, args)) => {
                    let args: Vec<&dyn std::fmt::Display> =
                        args.iter().map(|s| s as &dyn std::fmt::Display).collect();
                    let reason = Text::tr(&key, &args);
                    self.command_failure(Text::translate(
                        "commands.function.instantiationFailure",
                        &[Text::literal(f.id.clone()), reason],
                    ));
                    continue;
                }
            };
            total += lines.len() as i32;
            traced.push(f.id.clone());
            trace.push_str(&f.id);
            trace.push('\n');
            self.trace_function(&f.id, &lines, 0, &mut trace);
        }
        let written = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join(&output), trace))
            .is_ok();
        if !written {
            self.command_failure(Text::translate("commands.debug.function.traceFailed", &[]));
        }
        let message = if traced.len() == 1 {
            Text::translate(
                "commands.debug.function.success.single",
                &[
                    num(total),
                    Text::literal(traced[0].clone()),
                    Text::literal(output),
                ],
            )
        } else {
            Text::translate(
                "commands.debug.function.success.multiple",
                &[num(total), num(traced.len()), Text::literal(output)],
            )
        };
        self.feedback(message);
        Ok(total)
    }
    /// `FetchProfileCommand.resolveName` and `resolveId`: the lookup runs on
    /// background threads (`Util.nonCriticalIoPool`, here the cached profile
    /// resolver's), and the answer is reported by a later tick;
    /// `printForEntity` reports the player's own profile at once.
    pub(in crate::game::server_side) fn cmd_fetchprofile(&mut self, ctx: &Context) -> R<i32> {
        if ctx.literals.iter().any(|l| l == "name") {
            let name = match ctx.get("name") {
                Some(Arg::Str(s)) => s.clone(),
                _ => String::new(),
            };
            self.server.cmds.0.profile_fetches.push(ProfileFetch {
                key: NameOrId::Name(name.clone()),
                kind: "name",
                argument: Text::literal(name),
                source: self.server.command_source.clone(),
            });
            return Ok(1);
        }
        if ctx.literals.iter().any(|l| l == "id") {
            let id = match ctx.get("id") {
                Some(Arg::Str(s)) => sunrise_core::text::parse_java_uuid(s),
                _ => None,
            }
            .unwrap_or_default();
            // `Component.translationArg(id)`: the UUID's string form.
            self.server.cmds.0.profile_fetches.push(ProfileFetch {
                key: NameOrId::Id(id),
                kind: "id",
                argument: Text::literal(sunrise_protocol::crypto::uuid_string(id)),
                source: self.server.command_source.clone(),
            });
            return Ok(1);
        }
        let t = self.one_entity(ctx, "entity", false)?;
        if t != Target::Player {
            return Err(tr_error(
                "commands.fetchprofile.no_profile",
                &[self.target_name(t)],
            ));
        }
        // The player in place: a guest's own profile while their command runs.
        let id = sunrise_sim::lan::playerdata::ints_uuid(self.acting_uuid());
        let name = self.acting_player_name().to_owned();
        self.report_resolved_profile(
            "commands.fetchprofile.entity.success",
            Text::literal(&name),
            &ResolvedProfile {
                id,
                name,
                properties: Vec::new(),
            },
        );
        Ok(1)
    }
    /// `/jfr start|stop` (JfrCommand). Sunrise has no JVM flight recorder:
    /// the recording is its own tick timing, dumped as JSON.
    pub(in crate::game::server_side) fn cmd_jfr(&mut self, ctx: &Context) -> R<i32> {
        if ctx.literals.iter().any(|l| l == "start") {
            if self.server.cmds.0.jfr.is_some() {
                return Err(err("commands.jfr.start.failed", &[]));
            }
            let stamp = filename_date_time();
            self.server.cmds.0.jfr = Some((Instant::now(), self.common.ticks, stamp));
            self.feedback(Text::translate("commands.jfr.started", &[]));
            return Ok(1);
        }
        let Some((started, ticks, stamp)) = self.server.cmds.0.jfr.take() else {
            return Err(tr_error(
                "commands.jfr.dump.failed",
                &[Text::literal("Not currently profiling")],
            ));
        };
        let relative = format!("debug/client-{stamp}.json");
        let path = std::env::current_dir().unwrap_or_default().join(&relative);
        let report = serde_json::json!({
            "environment": "client",
            "seconds": started.elapsed().as_secs_f64(),
            "ticks": self.ticks - ticks,
        });
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, report.to_string()));
        if let Err(e) = written {
            return Err(tr_error(
                "commands.jfr.dump.failed",
                &[Text::literal(e.to_string())],
            ));
        }
        let file = Text::literal(relative).underlined().with_style(&Style {
            hover: Some(std::sync::Arc::new(Text::translate("chat.copy.click", &[]))),
            ..Style::default()
        });
        self.feedback(Text::translate("commands.jfr.stopped", &[file]));
        Ok(1)
    }
    /// `SpectateCommand.spectate`.
    pub(in crate::game::server_side) fn cmd_spectate(&mut self, ctx: &Context) -> R<i32> {
        let target = if ctx.get("target").is_some() {
            Some(self.one_entity(ctx, "target", false)?)
        } else {
            None
        };
        let player = if ctx.get("player").is_some() {
            self.one_entity(ctx, "player", true)?
        } else {
            self.source_player()?
        };
        if target == Some(player) {
            return Err(err("commands.spectate.self", &[]));
        }
        if !self.common.spectator() {
            return Err(tr_error(
                "commands.spectate.not_spectator",
                &[self.target_name(player)],
            ));
        }
        // EntityType.clientTrackingRange: only a marker's is zero.
        if let Some(t) = target
            && self.target_type(t) == "minecraft:marker"
        {
            return Err(tr_error(
                "commands.spectate.cannot_spectate",
                &[self.target_name(t)],
            ));
        }
        // A guest's camera is the hub's: the view moves in the guests' tick.
        let guest = match player {
            Target::Player => self.server.lan_acting,
            Target::Guest(conn) => Some(conn),
            _ => None,
        };
        match guest {
            Some(conn) => {
                let camera = target.and_then(|t| self.network_entity_id(t));
                self.server.cmds.0.camera_requests.push((conn, camera));
            }
            None => self.set_camera(target),
        }
        match target {
            Some(t) => {
                let name = self.target_name(t);
                self.feedback(Text::translate(
                    "commands.spectate.success.started",
                    &[name],
                ));
            }
            None => self.feedback(Text::translate("commands.spectate.success.stopped", &[])),
        }
        Ok(1)
    }
    /// `VersionCommand`.
    pub(in crate::game::server_side) fn cmd_version(&mut self) -> R<i32> {
        if !self
            .server
            .command_source
            .as_ref()
            .is_some_and(|s| s.silent)
        {
            self.command_system_message(Text::translate("commands.version.header", &[]));
            for line in self.common.version_lines() {
                self.command_system_message(line);
            }
        }
        Ok(1)
    }
    /// `ServerPlayer.postEffects` of a player target: a connected player's
    /// (told to their client at the end of their tick), or the host's own.
    pub(in crate::game) fn post_effects_of(&self, t: Target) -> Vec<String> {
        let conn = match t {
            Target::Guest(conn) => Some(conn),
            Target::Player => self.server.lan_acting,
            _ => return Vec::new(),
        };
        match conn {
            Some(conn) => self
                .command_guest(conn)
                .map(|g| g.post_effects.clone())
                .unwrap_or_default(),
            None => self.server.cmds.0.post_effects.clone(),
        }
    }
    /// The answers of the profile lookups that have come in
    /// (`server.execute(() -> result.ifPresentOrElse(...))`).
    /// Whether a `/fetchprofile` lookup is still in flight (the command
    /// harness waits for them, as vanilla's oracle resolves at once).
    pub(in crate::game::server_side) fn profile_fetches_pending(&mut self) -> bool {
        self.server.cmds.0.profile_fetches.iter().any(|f| {
            matches!(
                self.server.profile_resolver.lookup(&f.key),
                Resolve::Pending
            )
        })
    }
    /// `ServerPlayer.openDialog` for each target: the packet to a guest, the
    /// acting player's packet queue otherwise.
    pub(in crate::game) fn send_dialog(&mut self, targets: &[Target], holder: &DialogHolder) {
        assert!(
            self.is_logical_server(),
            "send_dialog called on Client role"
        );
        let packet = C::ShowDialog(play::ShowDialog {
            dialog: holder.clone(),
        });
        for &t in targets {
            match t {
                Target::Player if self.server.lan_acting.is_some() || self.server.lan.is_none() => {
                    self.server.guest_packets.push(packet.clone());
                }
                Target::Player => {}
                Target::Guest(conn) => {
                    if let Some(host) = self.server.lan.as_ref() {
                        host.hub.send_to(conn, &packet);
                    }
                }
                _ => {}
            }
        }
    }
    /// `ServerPlayer.setCamera`: a new camera moves the player to it.
    pub(in crate::game::server_side) fn set_camera(&mut self, target: Option<Target>) {
        let target = target.filter(|t| *t != Target::Player);
        if self.server.cmds.0.camera == target {
            return;
        }
        self.server.cmds.0.camera = target;
        if let Some(t) = target
            && let Some(p) = self.target_position(t)
            && let Some(player) = self.common.player.as_mut()
        {
            player.position = sunrise_core::geometry::Vec3::new(p[0], p[1], p[2]);
            self.common.previous_position = player.position;
        }
    }
    /// `WaypointCommand.mutateIcon`: untrack, change, track again.
    pub(in crate::game) fn set_waypoint_icon(&mut self, t: Target, icon: Icon) {
        if t == Target::Player {
            Self::save_waypoint_icon(&icon, &mut self.common.level.player_extra);
        }
        let t = self.waypoint_target(t);
        self.server.cmds.0.waypoint_icons.insert(t, icon);
        self.waypoint_transmitters();
        if let Some(tracked) = self.server.cmds.0.waypoints.as_mut() {
            tracked.retain(|x| *x != t);
            tracked.push(t);
        }
    }
    pub(in crate::game) fn set_waypoint_transmitter(&mut self, target: Target, on: bool) {
        let tracked = self.server.cmds.0.waypoints.get_or_insert_default();
        tracked.retain(|t| *t != target);
        if on {
            tracked.push(target);
        }
        if let Target::Guest(conn) = target {
            self.lan_set_waypoint_transmitting(conn, on);
        }
    }
    /// CommandSourceStack.getPlayerOrException.
    pub(in crate::game::server_side) fn source_player(&self) -> R<Target> {
        match self.server.command_source.as_ref().map(|s| s.entity) {
            None | Some(Some(Target::Player)) => Ok(Target::Player),
            _ => Err(err("permissions.requires.player", &[])),
        }
    }
    /// The camera part of `ServerPlayer.tick`: follow a living camera, drop a
    /// removed one, and stop when the player sneaks or leaves spectator mode.
    pub(in crate::game::server_side) fn tick_camera(&mut self) {
        let Some(t) = self.server.cmds.0.camera else {
            return;
        };
        if !self.common.spectator() {
            self.set_camera(None);
            return;
        }
        let sneaking = self.server_acting_sneak();
        match self.target_position(t) {
            Some(p) if !sneaking => {
                let rot = self.target_rotation(t);
                if let Some(player) = self.common.player.as_mut() {
                    player.position = sunrise_core::geometry::Vec3::new(p[0], p[1], p[2]);
                }
                self.common.yaw = rot[0];
                self.common.pitch = rot[1];
            }
            _ => self.set_camera(None),
        }
    }
    /// Per-tick server work of these commands: the delayed tick profiler
    /// start, queued messages and the spectator camera.
    pub(in crate::game::server_side) fn tick_command_extras(&mut self) {
        if self.server.cmds.0.profiler_delay {
            self.server.cmds.0.profiler_delay = false;
            self.server.cmds.0.profiler = Some((Instant::now(), self.common.ticks));
        }
        self.tick_profile_fetches();
        self.tick_camera();
        self.tick_waypoints();
    }
    pub(in crate::game::server_side) fn tick_profile_fetches(&mut self) {
        let fetches = std::mem::take(&mut self.server.cmds.0.profile_fetches);
        for fetch in fetches {
            let state = self.server.profile_resolver.lookup(&fetch.key);
            match state {
                Resolve::Pending => self.server.cmds.0.profile_fetches.push(fetch),
                Resolve::Done(result) => {
                    let outer = std::mem::replace(&mut self.server.command_source, fetch.source);
                    match result {
                        Some(profile) => self.report_resolved_profile(
                            &format!("commands.fetchprofile.{}.success", fetch.kind),
                            fetch.argument,
                            &profile,
                        ),
                        None => self.command_failure(Text::translate(
                            &format!("commands.fetchprofile.{}.failure", fetch.kind),
                            &[fetch.argument],
                        )),
                    }
                    self.server.command_source = outer;
                }
            }
        }
    }
    /// The waypoint part of a living entity's tick: dirty transmit ranges
    /// track or untrack it.
    pub fn tick_waypoints(&mut self) {
        let dirty = std::mem::take(&mut self.server.cmds.0.waypoint_dirty);
        if dirty.is_empty() {
            return;
        }
        self.waypoint_transmitters();
        for t in dirty {
            self.update_waypoint_transmitter(t);
        }
    }
    /// A living entity's `waypoint_transmit_range` (`isTransmittingWaypoint`).
    pub(in crate::game) fn transmit_range(&mut self, t: Target) -> f64 {
        if matches!(t, Target::Guest(_)) {
            return self
                .for_player(t, |g| g.transmit_range(Target::Player))
                .unwrap_or(0.0);
        }
        if t == Target::Player {
            return self
                .common
                .player
                .as_mut()
                .and_then(|p| {
                    let k = p.attributes.key("minecraft:waypoint_transmit_range")?;
                    p.attributes
                        .value(Some(k), sunrise_world::attribute_map::ValueKind::Computed)
                        .ok()
                })
                .unwrap_or(0.0);
        }
        // A mob's is in its own store, an Invisibility effect's stored
        // modifier among the others (`Mob::set_effect_modifiers`).
        if let Target::Mob(id) = t {
            return self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)
                .map_or(0.0, |m| {
                    sunrise_sim::mobs::finalize::value(
                        m,
                        sunrise_sim::mobs::finalize::Attr::WaypointTransmitRange,
                    )
                });
        }
        let catalog = self.common.motion.attributes.clone();
        let Some(key) = catalog.id("minecraft:waypoint_transmit_range") else {
            return 0.0;
        };
        match self.server.cmds.0.attributes.get_mut(&t) {
            Some(map) => map.value(key).unwrap_or(0.0),
            None => 0.0,
        }
    }
    pub fn waypoint_icon(&self, t: Target) -> Icon {
        self.server
            .cmds
            .0
            .waypoint_icons
            .get(&self.waypoint_target(t))
            .cloned()
            // The player's own icon comes back from their saved `locator_bar_icon`.
            .or_else(|| {
                (t == Target::Player)
                    .then(|| {
                        Self::waypoint_icon_from_saved(self.common.level.player_extra.as_ref())
                    })
                    .flatten()
            })
            .unwrap_or_default()
    }
    /// A guest's body is only `Player` during its turn; retained command state
    /// must use its connection so the next server tick still finds that body.
    pub(in crate::game::server_side) fn waypoint_target(&self, t: Target) -> Target {
        match t {
            Target::Player => self.server.lan_acting.map_or(t, Target::Guest),
            _ => t,
        }
    }
    /// `ServerWaypointManager.transmitters()`: the tracked set. The player
    /// joins tracked when transmitting; a changed range takes effect when the
    /// entity next ticks (`LivingEntity.onAttributeUpdated`), and
    /// `/waypoint modify` re-tracks its waypoint whatever its range.
    pub(in crate::game) fn waypoint_transmitters(&mut self) -> Vec<Target> {
        if self.server.cmds.0.waypoints.is_none() {
            let tracked = if self.server.lan.is_none() && self.transmit_range(Target::Player) > 0.0
            {
                vec![self.waypoint_target(Target::Player)]
            } else {
                Vec::new()
            };
            self.server.cmds.0.waypoints = Some(tracked);
        }
        let tracked = self.server.cmds.0.waypoints.clone().unwrap_or_default();
        let alive: Vec<Target> = tracked
            .into_iter()
            .filter(|t| self.target_position(*t).is_some())
            .collect();
        self.server.cmds.0.waypoints = Some(alive.clone());
        alive
            .into_iter()
            .filter(|t| self.target_level(*t) == self.common.dim)
            .map(|t| match t {
                Target::Guest(conn) if self.server.lan_acting == Some(conn) => Target::Player,
                _ => t,
            })
            .collect()
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
