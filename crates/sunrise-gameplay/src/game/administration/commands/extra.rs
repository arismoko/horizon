//! Commands that act on the player and the hosted game rather than the world:
//! `/spectate` (SpectateCommand), `/waypoint` (WaypointCommand), `/dialog`
//! (DialogCommand), `/posteffect` (PostEffectCommand), `/kick`,
//! `/fetchprofile`, `/publish`, `/unpublish`, `/debug`, `/jfr` and
//! `/version`, with vanilla's feedback and failures.
use super::{Arg, Context, R, Target, err};
use std::{collections::HashMap, time::Instant};
use sunrise_command::CommandError;
use sunrise_core::nbt::{JavaString, Tag};
use sunrise_game_data::{
    snbt,
    text::{self, ClickEvent, InlineObject, Style, Text},
};
use sunrise_level::profile_lookup::{NameOrId, ResolvedProfile};
use sunrise_protocol::packets::play::DialogHolder;
use sunrise_sim::lan::waypoints::Icon;

/// A `/fetchprofile` lookup in flight: what it resolves, the message ids it
/// answers with (`commands.fetchprofile.<kind>.success|failure`), the
/// argument they show, and the source to answer.
pub(in crate::game) struct ProfileFetch {
    pub(in crate::game) key: NameOrId,
    pub(in crate::game) kind: &'static str,
    pub(in crate::game) argument: Text,
    pub(in crate::game) source: Option<super::execute::Source>,
}

/// `ResolvableProfile.CODEC` of a resolved profile as NBT (`id` as an int
/// array, `name`, and `properties` only when there are some, each with its
/// `signature` only when signed), and the same as the JSON a head object
/// draws from.
pub(in crate::game) fn encode_profile(profile: &ResolvedProfile) -> (Tag, serde_json::Value) {
    let ints = [
        (profile.id >> 96) as i32,
        (profile.id >> 64) as i32,
        (profile.id >> 32) as i32,
        profile.id as i32,
    ];
    let mut fields = vec![
        ("id", Tag::IntArray(ints.to_vec())),
        ("name", Tag::String(profile.name.as_str().into())),
    ];
    let mut json = serde_json::json!({ "id": ints, "name": profile.name });
    if !profile.properties.is_empty() {
        let mut list = Vec::new();
        let mut json_list = Vec::new();
        for (name, value, signature) in &profile.properties {
            let mut entry = vec![
                ("name", Tag::String(name.as_str().into())),
                ("value", Tag::String(value.as_str().into())),
            ];
            let mut j = serde_json::json!({ "name": name, "value": value });
            if let Some(signature) = signature {
                entry.push(("signature", Tag::String(signature.as_str().into())));
                j["signature"] = signature.clone().into();
            }
            list.push(Tag::compound(entry));
            json_list.push(j);
        }
        fields.push(("properties", Tag::List(list)));
        json["properties"] = json_list.into();
    }
    (Tag::compound(fields), json)
}
#[cfg(test)]
pub(in crate::game) mod fetchprofile_tests {
    use super::*;
    #[test]
    fn unsigned_profile_without_properties_omits_them() {
        let profile = ResolvedProfile {
            id: 0x0000_0001_0000_0002_0000_0003_0000_0004,
            name: "Steve".into(),
            properties: Vec::new(),
        };
        let (tag, json) = encode_profile(&profile);
        assert_eq!(snbt::to_snbt(&tag), "{id:[I;1,2,3,4],name:\"Steve\"}");
        assert!(json.get("properties").is_none());
    }
    #[test]
    fn properties_keep_the_signature_only_when_signed() {
        let profile = ResolvedProfile {
            id: 1,
            name: "Alex".into(),
            properties: vec![
                ("textures".into(), "v".into(), Some("s".into())),
                ("other".into(), "w".into(), None),
            ],
        };
        let (tag, _) = encode_profile(&profile);
        assert_eq!(
            snbt::to_snbt(&tag),
            "{id:[I;0,0,0,1],name:\"Alex\",properties:[{name:\"textures\",signature:\"s\",value:\"v\"},{name:\"other\",value:\"w\"}]}"
        );
    }
}
/// Server-side player and server state these commands keep.
#[derive(Default)]
pub struct CommandExtras {
    /// ServerPlayer.camera: the entity `/spectate` attached the view to.
    pub camera: Option<Target>,
    /// The cameras of the guests of a server, by connection
    /// (game/network/lan_camera.rs).
    pub guest_cameras: HashMap<u32, super::super::lan_camera::GuestCamera>,
    /// `/spectate` for a guest: connection and the camera's network id
    /// (None to stop), taken in the guests' tick.
    pub camera_requests: Vec<(u32, Option<i32>)>,
    /// ServerPlayer.postEffects, in insertion order.
    pub post_effects: Vec<String>,
    /// MinecraftServer's tick profiler (`/debug start`): when it started and
    /// the tick count then.
    pub profiler: Option<(Instant, u64)>,
    /// `debugCommandProfilerDelayStart`: the profiler starts with the next tick.
    pub profiler_delay: bool,
    /// `/fetchprofile name|id` lookups still running.
    pub(in crate::game) profile_fetches: Vec<ProfileFetch>,
    /// The `/jfr start` recording: when it started and the tick count then.
    pub jfr: Option<(Instant, u64, String)>,
    /// Waypoint icons changed by `/waypoint modify`.
    pub waypoint_icons: HashMap<Target, Icon>,
    /// `/datapack disable vanilla` took the built-in pack out of the selection.
    pub vanilla_disabled: bool,
    /// The waypoint manager's tracked transmitters (None until first used).
    pub waypoints: Option<Vec<Target>>,
    /// Entities whose waypoint transmit range changed since they last ticked.
    pub waypoint_dirty: Vec<Target>,
    /// Attribute maps of living entities whose attributes Sunrise does not
    /// model (an armor stand's, a mob's beyond its own store's, `finalize::Attr`).
    pub attributes: HashMap<Target, sunrise_world::attributes::entity::EntityAttributes>,
    /// The `CommandResultCallback` results of the outermost source, when a
    /// harness records them.
    pub results: Option<Vec<(bool, i32)>>,
    /// How many of `adv.announced` a command's own `broadcastChanges` had
    /// made: the harness records those with the command, and leaves what
    /// `run_command`'s check after it earns for the next tick, as vanilla
    /// does for a command that calls no `broadcastChanges` (`/loot give`).
    pub broadcast_announced: usize,
    /// The running `/pregen` task.
    pub(in crate::game) pregen: Option<super::pregen::PregenTask>,
}

pub(in crate::game) fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}
pub(in crate::game) fn tr_error(key: &str, args: &[Text]) -> CommandError {
    CommandError::new(Text::translate(key, args))
}
/// `String.format(Locale.ROOT, "%.2f", v)`.
pub(in crate::game) fn two_places(v: f64) -> String {
    super::percent_f(v, 2)
}
/// `Util.getFilenameFormattedDateTime` (`yyyy-MM-dd_HH.mm.ss`, local time).
pub(in crate::game) fn filename_date_time() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (y, mo, d, h, mi, s) = civil(secs);
    format!("{y:04}-{mo:02}-{d:02}_{h:02}.{mi:02}.{s:02}")
}
/// UTC civil date and time of a Unix timestamp.
pub(in crate::game) fn civil(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

impl crate::game::ServerGame<'_> {
    /// EntityArgument.getEntity / getPlayer.
    pub(in crate::game) fn one_entity(
        &mut self,
        ctx: &Context,
        name: &str,
        player: bool,
    ) -> R<Target> {
        Ok(self.entities(ctx, name, player)?[0])
    }

    // ---- /spectate ------------------------------------------------------

    // ---- /waypoint ------------------------------------------------------

    /// `LivingEntity.onAttributeUpdated`: change the manager's tracked set.
    pub(in crate::game::server_side) fn update_waypoint_transmitter(&mut self, target: Target) {
        let on = self.transmit_range(target) > 0.0;
        self.set_waypoint_transmitter(target, on);
    }

    /// `WaypointArgument.getWaypoint`.
    pub(in crate::game) fn waypoint_arg(&mut self, ctx: &Context) -> R<Target> {
        let t = self.one_entity(ctx, "waypoint", false)?;
        if !self.is_living(t) {
            return Err(err("argument.waypoint.invalid", &[]));
        }
        Ok(t)
    }
    pub(in crate::game) fn is_living(&self, t: Target) -> bool {
        matches!(t, Target::Player | Target::Mob(_) | Target::Stand(_))
    }
    pub(in crate::game::server_side) fn cmd_waypoint(&mut self, id: &str, ctx: &Context) -> R<i32> {
        if id == "waypoint.list" {
            let dimension = self.common.dim.id();
            let waypoints = self.waypoint_transmitters();
            if waypoints.is_empty() {
                self.feedback(Text::translate(
                    "commands.waypoint.list.empty",
                    &[Text::literal(dimension)],
                ));
                return Ok(0);
            }
            let mut names = Text::empty();
            for (i, t) in waypoints.iter().enumerate() {
                if i > 0 {
                    names.push(", ");
                }
                let pos = self
                    .target_position(*t)
                    .unwrap_or_default()
                    .map(|v| v.floor() as i32);
                let color = self.waypoint_icon(*t).color.unwrap_or(-1) as u32 & 0xFF_FFFF;
                let style = Style {
                    color: Some(color),
                    click: Some(text::ClickEvent::SuggestCommand(format!(
                        "/execute in {dimension} run tp @s {} {} {}",
                        pos[0], pos[1], pos[2]
                    ))),
                    hover: Some(std::sync::Arc::new(Text::translate(
                        "chat.coordinates.tooltip",
                        &[],
                    ))),
                    ..Style::default()
                };
                // getFeedbackDisplayName().copy().withStyle(color): the
                // waypoint colour replaces the team colour of the name.
                let name = match (
                    *t,
                    self.common
                        .score
                        .board
                        .players_team(sunrise_gameplay_common::game::chat_common::PLAYER_NAME),
                ) {
                    (Target::Player, Some(team)) => {
                        let mut n = Text::empty();
                        n.push(sunrise_game_data::scoreboard::comp_text(&team.prefix));
                        n.push(sunrise_gameplay_common::game::chat_common::PLAYER_NAME);
                        n.push(sunrise_game_data::scoreboard::comp_text(&team.suffix));
                        n
                    }
                    _ => self.target_name(*t),
                };
                names.push(name.with_style(&style));
            }
            let n = waypoints.len() as i32;
            self.feedback(Text::translate(
                "commands.waypoint.list.success",
                &[num(n), Text::literal(dimension), names],
            ));
            return Ok(n);
        }
        let t = self.waypoint_arg(ctx)?;
        let mut icon = self.waypoint_icon(t);
        let style_literal = ctx.literals.iter().any(|l| l == "style");
        let message = if style_literal {
            icon.style = if ctx.literals.iter().any(|l| l == "reset") {
                "minecraft:default".into()
            } else {
                match ctx.get("style") {
                    Some(Arg::Resource(s)) => s.clone(),
                    _ => "minecraft:default".into(),
                }
            };
            Text::translate("commands.waypoint.modify.style", &[])
        } else if ctx.literals.iter().any(|l| l == "reset") {
            icon.color = None;
            Text::translate("commands.waypoint.modify.color.reset", &[])
        } else if ctx.literals.iter().any(|l| l == "hex") {
            let rgb = match ctx.get("color") {
                Some(Arg::Int(v)) => *v,
                _ => 0,
            };
            icon.color = Some(rgb);
            let hex = format!("{:06X}", rgb as u32 & 0xFF_FFFF);
            Text::translate(
                "commands.waypoint.modify.color",
                &[Text::colored(hex, rgb as u32 & 0xFF_FFFF)],
            )
        } else {
            let name = match ctx.get("color") {
                Some(Arg::Str(s)) => s.clone(),
                _ => "white".into(),
            };
            let rgb = text::named_color(&name).unwrap_or(0xFF_FFFF);
            icon.color = Some(rgb as i32);
            Text::translate(
                "commands.waypoint.modify.color",
                &[Text::colored(name, rgb)],
            )
        };
        self.set_waypoint_icon(t, icon);
        self.feedback(message);
        Ok(0)
    }

    // ---- /dialog --------------------------------------------------------

    pub(in crate::game::server_side) fn cmd_dialog(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "targets", true)?;
        let n = targets.len() as i32;
        if id == "dialog.show" {
            let Some(Arg::Resource(dialog)) = ctx.get("dialog").cloned() else {
                return Ok(0);
            };
            self.show_dialog_to(&targets, &dialog);
        } else {
            self.clear_dialog_to(&targets);
        }
        let (single, multiple) = if id == "dialog.show" {
            (
                "commands.dialog.show.single",
                "commands.dialog.show.multiple",
            )
        } else {
            (
                "commands.dialog.clear.single",
                "commands.dialog.clear.multiple",
            )
        };
        let message = if n == 1 {
            Text::translate(single, &[self.target_name(targets[0])])
        } else {
            Text::translate(multiple, &[num(n)])
        };
        self.feedback(message);
        Ok(n)
    }

    // ---- /posteffect ----------------------------------------------------

    pub(in crate::game::server_side) fn cmd_posteffect(
        &mut self,
        id: &str,
        ctx: &Context,
    ) -> R<i32> {
        if id == "posteffect.list" {
            let t = self.one_entity(ctx, "target", true)?;
            let name = self.target_name(t);
            let effects = self.post_effects_of(t);
            if effects.is_empty() {
                self.feedback(Text::translate("commands.posteffect.list.empty", &[name]));
            } else {
                self.feedback(Text::translate(
                    "commands.posteffect.list.success",
                    &[name, num(effects.len()), Text::literal(effects.join(", "))],
                ));
            }
            return Ok(effects.len() as i32);
        }
        let targets = self.entities(ctx, "targets", true)?;
        let effect = match ctx.get("posteffect") {
            Some(Arg::Resource(s)) => s.clone(),
            _ => String::new(),
        };
        let mut changed: Vec<Target> = Vec::new();
        for t in &targets {
            let done = self.change_post_effects(*t, |list| match id {
                // ServerPlayer.addPostEffect.
                "posteffect.add" => {
                    if list.contains(&effect) {
                        false
                    } else {
                        list.push(effect.clone());
                        true
                    }
                }
                // ServerPlayer.removePostEffect.
                "posteffect.remove" => match list.iter().position(|e| *e == effect) {
                    Some(i) => {
                        list.remove(i);
                        true
                    }
                    None => false,
                },
                // ServerPlayer.clearPostEffects.
                _ => !std::mem::take(list).is_empty(),
            });
            if done {
                changed.push(*t);
            }
        }
        let key = id.trim_start_matches("posteffect.");
        if changed.is_empty() {
            return Err(err(&format!("commands.posteffect.{key}.failed"), &[]));
        }
        let n = changed.len() as i32;
        let effect_text = Text::literal(effect);
        let message = match (key, n) {
            ("clear", 1) => Text::translate(
                "commands.posteffect.clear.success.single",
                &[self.target_name(changed[0])],
            ),
            ("clear", _) => {
                Text::translate("commands.posteffect.clear.success.multiple", &[num(n)])
            }
            (_, 1) => Text::translate(
                &format!("commands.posteffect.{key}.success.single"),
                &[effect_text, self.target_name(changed[0])],
            ),
            _ => Text::translate(
                &format!("commands.posteffect.{key}.success.multiple"),
                &[effect_text, num(n)],
            ),
        };
        self.feedback(message);
        Ok(n)
    }

    // ---- /test ----------------------------------------------------------

    /// `TestCommand`: running, resetting and clearing tests is
    /// `test_runs`; `locate`, `pos` and `create` are `test_locate`.
    pub(in crate::game::server_side) fn cmd_test(&mut self, id: &str, ctx: &Context) -> R<i32> {
        match id {
            "test.run" | "test.run.found" | "test.clear" | "test.stop" => {
                self.cmd_test_runs(id, ctx)
            }
            "test.verify" => self.test_verify(ctx),
            "test.create" => self.test_create(ctx),
            "test.locate" => self.test_locate(ctx),
            _ => self.test_show_pos(ctx),
        }
    }
    /// `Entity.pick(10, 1, false)`: the first non-air block on the view ray,
    /// else the block at the ray's end.
    pub(in crate::game::server_side) fn test_pick_block(&self) -> [i32; 3] {
        use sunrise_core::{
            geometry::{Vec3, traverse_blocks},
            position::BlockPos,
        };
        let Some(p) = self.common.player.as_ref() else {
            return [0; 3];
        };
        let eye = Vec3::new(
            p.position.x,
            p.position.y + f64::from(p.dimensions.eye_height),
            p.position.z,
        );
        let (yaw, pitch) = (
            f64::from(self.common.yaw).to_radians(),
            f64::from(self.common.pitch).to_radians(),
        );
        let dir = [
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        ];
        let end = Vec3::new(
            eye.x + dir[0] * 10.0,
            eye.y + dir[1] * 10.0,
            eye.z + dir[2] * 10.0,
        );
        let hit = traverse_blocks(eye, end, 256, |b: BlockPos| {
            let state = self.common.world.block(b.x, b.y, b.z);
            (!self.common.res.facts().is_air(state)).then_some([b.x, b.y, b.z])
        });
        hit.ok().flatten().unwrap_or([
            end.x.floor() as i32,
            end.y.floor() as i32,
            end.z.floor() as i32,
        ])
    }

    // ---- /kick ----------------------------------------------------------

    /// `KickCommand.kickPlayers`: only guests of a published world can be
    /// kicked; the owner never.
    pub(in crate::game::server_side) fn cmd_kick(&mut self, ctx: &Context) -> R<i32> {
        let selector = match ctx.get("targets") {
            Some(Arg::Entity(s)) => Some(s.clone()),
            _ => None,
        };
        // EntityArgument.getPlayers runs before the published check.
        let host = self.entities(ctx, "targets", true).unwrap_or_default();
        let guests: Vec<(u32, String)> = match (&self.server.lan, &selector) {
            (Some(lan), Some(s)) => lan
                .hub
                .guests
                .iter()
                .filter(|g| {
                    (g.participates_online() || g.is_fake())
                        && lan.hub.singleplayer_owner != Some(g.uuid)
                })
                .filter(|g| match &s.player_name {
                    Some(name) => g.name.eq_ignore_ascii_case(name),
                    None => s.kind == 'a' || s.kind == 'e',
                })
                .map(|g| (g.conn, g.name.clone()))
                .collect(),
            _ => Vec::new(),
        };
        if host.is_empty() && guests.is_empty() {
            return Err(err("argument.entity.notfound.player", &[]));
        }
        if self.server.lan.is_none() || (self.serving() && self.published_port().is_none()) {
            return Err(err("commands.kick.singleplayer.failed", &[]));
        }
        let reason_raw = match ctx.get("reason") {
            Some(Arg::Str(s)) => Some(s.clone()),
            _ => None,
        };
        let reason = match &reason_raw {
            Some(raw) => self.expand_selectors(raw),
            None => Text::translate("multiplayer.disconnect.kicked", &[]),
        };
        let tag = match &reason_raw {
            Some(_) => sunrise_core::nbt::Tag::String(reason.plain().as_str().into()),
            None => sunrise_core::nbt::Tag::compound([(
                "translate",
                sunrise_core::nbt::Tag::String("multiplayer.disconnect.kicked".into()),
            )]),
        };
        let mut count = 0;
        for (conn, name) in guests {
            if let Some(lan) = self.server.lan.as_mut() {
                lan.hub.kick(conn, tag.clone());
            }
            self.feedback(Text::translate(
                "commands.kick.success",
                &[Text::literal(name), reason.clone()],
            ));
            count += 1;
        }
        if count == 0 {
            return Err(err("commands.kick.owner.failed", &[]));
        }
        Ok(count)
    }

    // ---- /fetchprofile --------------------------------------------------

    /// `FetchProfileCommand.reportResolvedProfile`: the profile as
    /// `ResolvableProfile.CODEC` encodes it (`id`, `name`, and `properties`
    /// only when there are some), four clickable actions and the head object,
    /// sent with `sendSuccess(..., false)`.
    pub(in crate::game::server_side) fn report_resolved_profile(
        &mut self,
        message_id: &str,
        argument: Text,
        profile: &ResolvedProfile,
    ) {
        let (profile_tag, json) = encode_profile(profile);
        let encoded_profile = snbt::to_snbt(&profile_tag);
        // `Component.object(new PlayerSprite(profile, true))`, encoded: `hat`
        // is left out at its default.
        let encoded_component = snbt::to_snbt(&Tag::compound([("player", profile_tag)]));
        let head = Text::styled(
            "\u{FFFC}",
            Style {
                inline_object: Some(InlineObject::Player {
                    profile: json.into(),
                    hat: true,
                    fallback: format!("[{} head]", profile.name),
                }),
                ..Style::default()
            },
        );
        let item = |key: &str, args: &[Text], click: ClickEvent| {
            let inner = Text::translate(key, args)
                .with_color(text::GREEN)
                .with_style(&Style {
                    click: Some(click),
                    ..Style::default()
                });
            Text::translate("chat.square_brackets", &[inner])
        };
        let mut clickable = item(
            "commands.fetchprofile.copy_component",
            &[],
            ClickEvent::CopyToClipboard(encoded_profile.clone()),
        );
        clickable.push(" ");
        clickable.push(item(
            "commands.fetchprofile.give_item",
            &[],
            ClickEvent::RunCommand(format!(
                "give @s minecraft:player_head[profile={encoded_profile}]"
            )),
        ));
        clickable.push(" ");
        clickable.push(item(
            "commands.fetchprofile.summon_mannequin",
            &[],
            ClickEvent::RunCommand(format!(
                "summon minecraft:mannequin ~ ~ ~ {{profile:{encoded_profile}}}"
            )),
        ));
        clickable.push(" ");
        clickable.push(item(
            "commands.fetchprofile.copy_text",
            &[head.with_color(text::WHITE)],
            ClickEvent::CopyToClipboard(encoded_component),
        ));
        self.feedback_unbroadcast(Text::translate(message_id, &[argument, clickable]));
    }

    // ---- /publish, /unpublish -------------------------------------------

    /// `PublishCommand.publish` with `IntegratedServer.publishServer`.
    pub(in crate::game::server_side) fn cmd_publish(&mut self, ctx: &Context) -> R<i32> {
        let allow = matches!(ctx.get("allowCommands"), Some(Arg::Bool(true)));
        let port = match ctx.get("port") {
            Some(Arg::Int(p)) => *p,
            // HttpUtil.getAvailablePort.
            _ => std::net::TcpListener::bind((
                if std::env::var_os("SUNRISE_TEST_LOOPBACK").is_some_and(|v| v == "1") {
                    "127.0.0.1"
                } else {
                    "0.0.0.0"
                },
                0,
            ))
            .and_then(|l| l.local_addr())
            .map_or(25564, |a| i32::from(a.port())),
        };
        let published = if self.serving() {
            self.published_port()
        } else {
            self.server.lan.as_ref().map(|l| l.settings.port)
        };
        if let Some(port) = published {
            return Err(err("commands.publish.alreadyPublished.lan", &[&port]));
        }
        let settings = super::super::lan::LanSettings {
            port: port as u16,
            guest_commands: allow,
            force_game_mode: false,
        };
        // A dedicated server is open from the start (`published` above).
        let started = self.serving() && self.publish(settings).is_ok();
        if !started {
            return Err(err("commands.publish.failed", &[]));
        }
        let port_text = port.to_string();
        let link = Text::literal(port_text.clone()).with_style(&Style {
            color: Some(text::GREEN),
            hover: Some(std::sync::Arc::new(Text::translate("chat.copy.click", &[]))),
            ..Style::default()
        });
        self.feedback(Text::translate(
            "commands.publish.started.lan",
            &[Text::translate("chat.square_brackets", &[link])],
        ));
        Ok(port)
    }
    pub(in crate::game::server_side) fn cmd_unpublish(&mut self) -> R<i32> {
        if !self.unpublish() {
            return Err(err("commands.unpublish.notPublished", &[]));
        }
        self.feedback(Text::translate("commands.unpublish.success", &[]));
        Ok(1)
    }

    // ---- /debug, /jfr, /version -----------------------------------------

    /// The tracer's `[F]`/`[C]` lines for one function, running its commands.
    pub(in crate::game::server_side) fn trace_function(
        &mut self,
        id: &str,
        lines: &[String],
        depth: usize,
        out: &mut String,
    ) {
        let indent = |d: usize| "    ".repeat(d + 1);
        out.push_str(&format!("{}[F] {id} size={}\n", indent(depth), lines.len()));
        let source = super::execute::Source {
            silent: true,
            ..self.player_source()
        };
        for line in lines {
            out.push_str(&format!("{}[C] {line}", indent(depth + 1)));
            match self.execute_line(line, source.clone()) {
                Ok(v) => out.push_str(&format!(" -> {v}\n")),
                Err(e) => {
                    out.push('\n');
                    out.push_str(&format!("{}[E] {}", indent(depth + 2), e.message.plain()));
                }
            }
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// `ServerPlayer.openDialog` for every target of `/dialog show`: a guest
    /// is sent `ClientboundShowDialogPacket`, the window's own player opens
    /// the dialog on its screen.
    pub(in crate::game) fn show_dialog_to(&mut self, targets: &[Target], dialog: &str) {
        let Some(holder) = self.dialog_holder(dialog) else {
            return;
        };
        self.send_dialog(targets, &holder);
    }
    /// `ServerPlayer.openDialog` for the dialog of a `show_dialog` click
    /// event (a sign's): a registry id or the dialog itself, opened for the
    /// player the game acts for.
    pub(in crate::game::server_side) fn show_click_dialog(
        &mut self,
        dialog: &sunrise_core::nbt::Tag,
    ) {
        let holder = match dialog {
            sunrise_core::nbt::Tag::String(id) => self.dialog_holder(&id.to_string_lossy()),
            direct => Some(DialogHolder::Direct(direct.clone())),
        };
        if let Some(holder) = holder {
            self.send_dialog(&[Target::Player], &holder);
        }
    }
    /// `ServerPlayer.openDialog`'s dialog: an entry of the `minecraft:dialog`
    /// registry, or the dialog an inline `ResourceOrIdArgument` carries.
    pub(in crate::game) fn dialog_holder(&self, dialog: &str) -> Option<DialogHolder> {
        let text = dialog.trim();
        if text.starts_with('{') {
            return sunrise_command::snbt::parse_fully(text)
                .ok()
                .map(DialogHolder::Direct);
        }
        // A registry id, written with or without its namespace.
        let id = if text.contains(':') {
            text.to_owned()
        } else {
            format!("minecraft:{text}")
        };
        let index = self
            .common
            .ui
            .registries
            .ids("minecraft:dialog")
            .iter()
            .position(|name| *name == id);
        match index {
            Some(i) => Some(DialogHolder::Registry(i as i32)),
            None => {
                log::warn!("Sunrise: no dialog {id} in the registry");
                None
            }
        }
    }
}
impl crate::game::ServerGame<'_> {
    /// `LivingEntity.readAdditionalSaveData`'s `locator_bar_icon`
    /// (`Waypoint.Icon.CODEC`: a required `style` and an optional int `color`).
    /// None when the key is missing or does not decode, which is the default icon.
    pub(in crate::game) fn waypoint_icon_from_saved(entity: Option<&Tag>) -> Option<Icon> {
        let icon = entity?.get("locator_bar_icon")?;
        let Some(Tag::String(style)) = icon.get("style") else {
            return None;
        };
        let color = match icon.get("color") {
            Some(Tag::Int(rgb)) => Some(*rgb),
            _ => None,
        };
        Some(Icon {
            style: style.to_string_lossy(),
            color,
        })
    }
    /// `LivingEntity.addAdditionalSaveData`'s `locator_bar_icon`: written only
    /// when the icon `hasData`, so any other icon takes the key out.
    pub(in crate::game) fn save_waypoint_icon(icon: &Icon, entity: &mut Option<Tag>) {
        if icon.style == "minecraft:default" && icon.color.is_none() {
            if let Some(Tag::Compound(map)) = entity {
                map.remove(&JavaString::from("locator_bar_icon"));
            }
            return;
        }
        let mut fields = vec![("style", Tag::String(icon.style.as_str().into()))];
        if let Some(rgb) = icon.color {
            fields.push(("color", Tag::Int(rgb)));
        }
        if let Tag::Compound(map) = entity.get_or_insert_with(|| Tag::Compound(Default::default()))
        {
            map.insert("locator_bar_icon".into(), Tag::compound(fields));
        }
    }
}

#[cfg(test)]
pub(in crate::game) mod waypoint_icon_save_tests {
    use super::*;
    use sunrise_sim::save::Level;

    fn icon(style: &str, color: Option<i32>) -> Icon {
        Icon {
            style: style.into(),
            color,
        }
    }

    #[test]
    fn an_icon_set_by_waypoint_modify_survives_a_save_and_a_load() {
        let mut level = Level::default();
        crate::game::ServerGame::save_waypoint_icon(
            &icon("minecraft:bowtie", Some(0x00FF_0000)),
            &mut level.player_extra,
        );
        // The player's file: `locator_bar_icon` is `{style, color}`.
        let saved = level.player_tag(&Default::default());
        assert_eq!(
            saved.get("locator_bar_icon"),
            Some(&Tag::compound([
                ("style", Tag::String("minecraft:bowtie".into())),
                ("color", Tag::Int(0x00FF_0000)),
            ]))
        );
        // Loaded again: the same icon.
        assert_eq!(
            crate::game::ServerGame::waypoint_icon_from_saved(Some(&saved)),
            Some(icon("minecraft:bowtie", Some(0x00FF_0000)))
        );
    }

    #[test]
    fn a_style_alone_or_a_colour_alone_is_data_and_is_saved() {
        let mut only_style = None;
        crate::game::ServerGame::save_waypoint_icon(
            &icon("minecraft:bowtie", None),
            &mut only_style,
        );
        let tag = only_style.unwrap();
        assert_eq!(
            tag.get("locator_bar_icon"),
            Some(&Tag::compound([(
                "style",
                Tag::String("minecraft:bowtie".into())
            )]))
        );
        let mut only_color = None;
        crate::game::ServerGame::save_waypoint_icon(
            &icon("minecraft:default", Some(7)),
            &mut only_color,
        );
        assert_eq!(
            crate::game::ServerGame::waypoint_icon_from_saved(only_color.as_ref()),
            Some(icon("minecraft:default", Some(7)))
        );
    }

    #[test]
    fn a_player_with_no_icon_saves_no_key_and_a_reset_takes_it_out() {
        let mut level = Level::default();
        assert!(
            level
                .player_tag(&Default::default())
                .get("locator_bar_icon")
                .is_none()
        );
        // The default icon on a player with no file writes nothing, not even a file.
        crate::game::ServerGame::save_waypoint_icon(&Icon::default(), &mut level.player_extra);
        assert!(level.player_extra.is_none());
        crate::game::ServerGame::save_waypoint_icon(
            &icon("minecraft:default", Some(1)),
            &mut level.player_extra,
        );
        assert!(
            level
                .player_tag(&Default::default())
                .get("locator_bar_icon")
                .is_some()
        );
        // `/waypoint modify ... reset` leaves the default icon: the key goes.
        crate::game::ServerGame::save_waypoint_icon(&Icon::default(), &mut level.player_extra);
        assert!(
            level
                .player_tag(&Default::default())
                .get("locator_bar_icon")
                .is_none()
        );
    }

    #[test]
    fn a_save_without_the_key_or_with_a_broken_one_loads_the_default_icon() {
        assert_eq!(
            crate::game::ServerGame::waypoint_icon_from_saved(None),
            None
        );
        let none = Tag::compound([("Health", Tag::Float(20f32.to_bits()))]);
        assert_eq!(
            crate::game::ServerGame::waypoint_icon_from_saved(Some(&none)),
            None
        );
        // `style` is required by the codec.
        let no_style =
            Tag::compound([("locator_bar_icon", Tag::compound([("color", Tag::Int(5))]))]);
        assert_eq!(
            crate::game::ServerGame::waypoint_icon_from_saved(Some(&no_style)),
            None
        );
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
