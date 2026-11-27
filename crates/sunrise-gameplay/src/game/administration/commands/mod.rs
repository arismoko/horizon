//! Command execution (the `execute` side of each vanilla command class):
//! entity selectors resolved against the game, vanilla feedback and errors.
pub(in crate::game) mod admin;
pub(in crate::game) mod clone_payload;
pub mod compute;
pub mod data;
pub(in crate::game) mod difficulty;
pub(in crate::game) mod display_names;
#[allow(unused_imports)]
pub(in crate::game) use data::place_summoned;
pub(in crate::game) mod effects;
pub mod execute;
pub(in crate::game) mod extra;
pub(in crate::game) mod give;
pub(in crate::game) mod item_count;
pub mod items;
pub(in crate::game) mod misc;
pub(in crate::game) mod modprofile;
pub mod place;
pub(in crate::game) mod players;
pub(in crate::game) mod position_args;
pub(in crate::game) mod pregen;
pub(in crate::game) mod profiles;
pub(in crate::game) mod raid;
pub(in crate::game) mod selector_bounds;
pub(in crate::game) mod selectors;
pub(in crate::game) mod targets;
pub mod terrain_cmds;
pub(in crate::game) mod test_locate;
pub(in crate::game) mod test_runs;
pub(in crate::game) mod text_commands;
pub mod world_cmds;

use super::Dim;
use sunrise_command::{
    Arg, Context, Reader,
    args::{BlockInput, Coordinates, ItemInput, Selector},
    vanilla::DIFFICULTIES,
};
use sunrise_core::{java_number::float_to_string, random::RandomSource};
use sunrise_game_data::gamerules::{self, Value};
use sunrise_gameplay_common::game::api_values::block_edit::block_predicate_matches_state;
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_world::registry::BlockStateId;

#[allow(unused_imports)]
pub use sunrise_gameplay_common::game::commands_common::*;

macro_rules! target_entity_mut {
    ($game:expr, $target:expr, $e:ident => $body:expr) => {{
        use super::entity::Entity as _;
        let handle = $game.entity_handles().find(|h| {
            super::entity::with_entity!($game, *h, entity => entity.selector_target() == Some($target))
        });
        if let Some(handle) = handle {
            super::entity::with_entity_mut!($game, handle, $e => $body);
        }
    }};
}
#[allow(unused_imports)]
pub(in crate::game) use target_entity_mut;

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn syntax_failure(&mut self, e: &CommandError, command: &str) {
        self.command_failure(e.message.clone());
        if let Some(context) =
            sunrise_gameplay_common::game::commands_common::format_syntax_failure_context(
                e, command,
            )
        {
            self.command_failure(context);
        }
    }
    /// `sendSuccess(message, false)`: the source is told (when it takes
    /// success feedback), the operators are not.
    pub(in crate::game) fn feedback_unbroadcast(&mut self, message: Text) {
        self.send_success(message, false);
    }
    /// A vanilla command's success line: `sendSuccess(message, true)`, except the
    /// replies vanilla sends with `broadcast` false (`admin::QUIET`).
    pub(in crate::game) fn feedback(&mut self, message: Text) {
        let broadcast = !message
            .translation_key()
            .is_some_and(|k| admin::QUIET.contains(&k));
        self.send_success(message, broadcast);
    }
    /// MinecraftServer.createCommandSourceStack's position: the corner of
    /// the respawn block (`Vec3.atLowerCornerOf`).
    pub fn server_position(&self) -> [f64; 3] {
        self.common.spawn.map_or([0.0, 64.0, 0.0], |s| {
            [s.x.floor(), s.y.floor(), s.z.floor()]
        })
    }
    pub(in crate::game) fn coords(&self, ctx: &Context, name: &str) -> Option<Coordinates> {
        match ctx.get(name) {
            Some(Arg::Coords(c)) => Some(c.clone()),
            _ => None,
        }
    }
    pub(in crate::game::server_side) fn position_arg(
        &self,
        ctx: &Context,
        name: &str,
    ) -> Option<[f64; 3]> {
        self.coords(ctx, name).map(|c| self.resolve_coords(&c))
    }
    pub(in crate::game) fn int(ctx: &Context, name: &str) -> Option<i32> {
        match ctx.get(name) {
            Some(Arg::Int(v)) => Some(*v),
            _ => None,
        }
    }
    // ---- entities -------------------------------------------------------

    /// `type=`: every positive must match, no negative may match.
    pub(in crate::game) fn selector_types_match(&self, s: &Selector, ty: &str) -> bool {
        s.types.iter().all(|(want, neg)| {
            let matches = if let Some(tag) = want.strip_prefix('#') {
                self.entity_tag_matches(tag, ty)
            } else {
                want == ty
            };
            matches != *neg
        })
    }
    /// The candidates of the level in place that pass the selector
    /// (EntitySelector.getPredicate).
    pub fn select_in_level(
        &mut self,
        candidates: Vec<Target>,
        s: &Selector,
        origin: [f64; 3],
    ) -> Vec<(Target, f64)> {
        let level = self.common.experience.level;
        // EntitySelector tests the type before any predicate. With no scores=
        // or team= (whose holders may be made as they are read), the rest of
        // the extra filter is a pure test of each candidate, so the ones of
        // another type leave first: `@e[type=marker,tag=…]` no longer reads
        // the tags of every falling block and item in the level.
        let candidates = if !s.types.is_empty() && s.scores.is_empty() && s.teams.is_empty() {
            candidates
                .into_iter()
                .filter(|&t| self.selector_types_match(s, &self.target_type(t)))
                .collect()
        } else {
            candidates
        };
        let passing = self.selector_extra_filter(&candidates, s);
        candidates
            .into_iter()
            .filter_map(|t| {
                let p = self.target_position(t)?;
                let ty = self.target_type(t);
                if !self.selector_types_match(s, &ty) {
                    return None;
                }
                if !s.include_entities
                    && !t.is_player()
                    && s.types.iter().all(|(w, n)| *n || w != &ty)
                    && s.kind != 'e'
                    && s.kind != 'n'
                {
                    return None;
                }
                for (name, neg) in &s.names {
                    let n = self.target_plain_name(t);
                    if (n == *name) == *neg {
                        return None;
                    }
                }
                if !passing.contains(&t) {
                    return None;
                }
                let player = match t {
                    // ServerPlayer.gameMode: the real mode, not `creative`.
                    Target::Player => Some((level, self.common.mode.name())),
                    Target::Guest(conn) => self.guest_level_and_mode(conn),
                    _ => None,
                };
                if let Some((lo, hi)) = s.level
                    && player.is_none_or(|(level, _)| {
                        lo.is_some_and(|l| level < l) || hi.is_some_and(|h| level > h)
                    })
                {
                    return None;
                }
                for (mode, neg) in &s.gamemodes {
                    if player.is_none_or(|(_, current)| (current == mode) == *neg) {
                        return None;
                    }
                }
                let d2 = (p[0] - origin[0]).powi(2)
                    + (p[1] - origin[1]).powi(2)
                    + (p[2] - origin[2]).powi(2);
                if let Some((lo, hi)) = s.distance
                    && (lo.is_some_and(|l| d2 < l * l) || hi.is_some_and(|h| d2 > h * h))
                {
                    return None;
                }
                // x_rotation is the pitch, y_rotation the yaw (getXRot, getYRot).
                let [yaw, pitch] = self.target_rotation(t);
                if let Some(range) = s.x_rotation
                    && !rotation_in(range, pitch)
                {
                    return None;
                }
                if let Some(range) = s.y_rotation
                    && !rotation_in(range, yaw)
                {
                    return None;
                }
                if s.delta.iter().any(Option::is_some) {
                    let d = s.delta.map(|v| v.unwrap_or(0.0));
                    if let Some(bounds) = self.selector_target_bounds(t) {
                        if !selector_bounds::selector_box(origin, d).intersects(bounds) {
                            return None;
                        }
                    } else {
                        // Unmodeled boxes retain their previous point fallback.
                        for i in 0..3 {
                            let (a, b) = (
                                origin[i].min(origin[i] + d[i]),
                                origin[i].max(origin[i] + d[i]) + 1.0,
                            );
                            if p[i] < a || p[i] >= b {
                                return None;
                            }
                        }
                    }
                }
                Some((t, d2))
            })
            .collect()
    }
    /// `EntityLookup.getEntities`: insertion order for the joined families in
    /// unbounded arbitrary queries. Spatial queries keep their existing path;
    /// their section membership order is a separate traversal.
    /// With only the `Entity::isAlive` ones when `alive_only` (`@e` and `@n`).
    pub(in crate::game) fn global_selector_candidates(&self, alive_only: bool) -> Vec<Target> {
        use super::entity::{Entity as _, entity_lists_of, with_entity};
        let original = self.level_candidates(alive_only);
        // Counts preserve every occurrence, including nonwalk fallback targets.
        // The map is never iterated to choose order and is not an order store.
        let mut remaining = std::collections::HashMap::<Target, usize>::new();
        for target in &original {
            *remaining.entry(*target).or_default() += 1;
        }
        let mut ordered = Vec::with_capacity(original.len());
        let walked = entity_lists_of!(self).walk_in_join_order(|handle, _| {
            if let Some(target) = with_entity!(self, handle, entity => entity.selector_target())
                && let Some(count) = remaining.get_mut(&target)
                && *count > 0
            {
                *count -= 1;
                ordered.push(target);
            }
        });
        // Invalid/unjoined storage retains the whole old path, never a prefix.
        if walked.is_err() {
            return original;
        }
        for target in original {
            let count = remaining.get_mut(&target).expect("an original candidate");
            if *count > 0 {
                *count -= 1;
                ordered.push(target);
            }
        }
        ordered
    }

    /// `ServerLevel.getEntityOrPart` for a part: the `EnderDragonPart` of the
    /// level in place with this network id. `level_entity` does not find it,
    /// as `Level.getEntity` does not: no selector or command reaches a part.
    pub fn level_dragon_part(&self, id: u32) -> Option<Target> {
        let part = Target::DragonPart(id);
        self.level_holds(part).then_some(part)
    }
    /// Run `f` with the level that holds the entity a target names in place
    /// (the entity ids are unique across the loaded levels) and put the
    /// level back afterwards. `f` does not run when no level holds the
    /// entity.
    pub fn in_target_level<R>(&mut self, t: Target, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        self.with_level_in_place(self.resident_target_dimension(t)?, f)
    }
    /// EntityArgument.getEntities / getPlayers / getEntity / getPlayer.
    pub(in crate::game) fn entities(
        &mut self,
        ctx: &Context,
        name: &str,
        players: bool,
    ) -> R<Vec<Target>> {
        let Some(Arg::Entity(s)) = ctx.get(name).cloned() else {
            return Ok(vec![self.implied_player()]);
        };
        self.resolve_entity_argument(&s, players)
    }
    /// Resolve an already parsed entity argument against the current command source.
    pub(in crate::game) fn resolve_entity_argument(
        &mut self,
        selector: &Selector,
        players: bool,
    ) -> R<Vec<Target>> {
        let found: Vec<Target> = self
            .select(selector)
            .into_iter()
            .filter(|t| !players || t.is_player())
            .collect();
        if found.is_empty() {
            return Err(err(
                if players {
                    "argument.entity.notfound.player"
                } else {
                    "argument.entity.notfound.entity"
                },
                &[],
            ));
        }
        Ok(found)
    }
    // ---- dispatch -------------------------------------------------------

    pub(in crate::game::server_side) fn cmd_help(&mut self, ctx: &Context) -> R<i32> {
        let d = self.common.ui.dispatcher.0.clone();
        let level = self.common.ui.registries.permission;
        match ctx.get("command") {
            None => {
                let usage = d.smart_usage(0, level);
                let n = usage.len() as i32;
                for (_, line) in usage {
                    self.feedback_unbroadcast(Text::literal(format!("/{line}")));
                }
                Ok(n)
            }
            Some(Arg::Str(command)) => {
                let parse = d.parse(command, &self.common.ui.registries);
                if parse.context.nodes.is_empty() {
                    return Err(err("commands.help.failed", &[]));
                }
                let last = parse.context.nodes.last().map(|n| n.0).unwrap_or(0);
                let usage = d.smart_usage(last, level);
                let prefix = &command[..parse.context.range.1.min(command.len())];
                for (_, line) in &usage {
                    self.feedback_unbroadcast(Text::literal(format!("/{prefix} {line}")));
                }
                if usage.is_empty() {
                    self.feedback_unbroadcast(Text::literal(format!("/{prefix}")));
                }
                Ok(usage.len().max(1) as i32)
            }
            _ => Ok(0),
        }
    }
    pub(in crate::game::server_side) fn cmd_gamemode(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::GameMode(mode)) = ctx.get("gamemode").cloned() else {
            return Ok(0);
        };
        let targets = if ctx.get("target").is_some() {
            self.entities(ctx, "target", true)?
        } else {
            vec![self.implied_player()]
        };
        let mut count = 0;
        for t in targets {
            let mode_name = Text::translate(&format!("gameMode.{}", mode.name()), &[]);
            // GameModeCommand.logGamemodeChange: the source's own entity.
            let own = self.source_entity() == Some(t);
            let told = !own
                && self
                    .common
                    .ui
                    .rules
                    .bool_at(sunrise_game_data::gamerules::vanilla::SEND_COMMAND_FEEDBACK);
            let changed = self.for_player(t, |g| {
                if !g.set_game_mode(mode) {
                    return false;
                }
                if told {
                    g.server_system_message(Text::translate(
                        "gameMode.changed",
                        std::slice::from_ref(&mode_name),
                    ));
                }
                true
            });
            if changed != Some(true) {
                continue;
            }
            count += 1;
            if own {
                self.feedback(Text::translate(
                    "commands.gamemode.success.self",
                    &[mode_name],
                ));
            } else {
                self.feedback(Text::translate(
                    "commands.gamemode.success.other",
                    &[self.target_name(t), mode_name],
                ));
            }
        }
        Ok(count)
    }
    pub fn teleport_to(
        &mut self,
        t: Target,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
        turn_riders: bool,
    ) {
        self.teleport_to_relative(t, pos, rotation, [false; 3], turn_riders);
    }
    pub fn teleport_to_in_level(
        &mut self,
        t: Target,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
        relative: [bool; 3],
        turn_riders: bool,
    ) {
        let from = self.place_teleport_filed(t, pos, rotation, relative, turn_riders);
        if let (Some(entity), Some(to)) = (self.network_entity_id(t), self.target_position(t))
            && let (true, Some(from)) = (self.common.options.dedicated.is_some(), from)
        {
            self.native_entity_teleport(self.common.dim.id(), entity as u32, from, to);
        }
    }

    /// `ServerPlayer.teleport` via `ServerGamePacketListenerImpl.teleport`: place the acting
    /// player's body and send the teleport at the end of the guest turn. Returns its old feet
    /// position, without adding command-only chunk holds or notifications.
    pub(in crate::game) fn place_player_teleport_in_level(
        &mut self,
        at: Vec3,
        rotation: Option<[f32; 2]>,
        relative: [bool; 3],
    ) -> Option<[f64; 3]> {
        self.stop_riding();
        let from = self.common.player.as_mut().map(|p| {
            let from = [p.position.x, p.position.y, p.position.z];
            p.position = at;
            p.velocity = teleport_velocity(p.velocity, relative, p.fall_flying);
            p.teleported = true;
            p.teleported_delta = relative;
            from
        });
        self.common.previous_position = at;
        if let Some([yaw, pitch]) = rotation {
            if yaw.is_finite() {
                self.common.yaw = yaw;
            }
            if !pitch.is_nan() {
                // PositionMoveRotation.calculateAbsolute clamps pitch before setXRot.
                self.common.pitch = pitch.clamp(-90.0, 90.0);
            }
        }
        from
    }

    /// `Entity.teleportSameDimension`: shared entity/passenger placement.
    /// Returns its old position for the command caller's completion notification.
    pub(in crate::game) fn place_teleport_in_level(
        &mut self,
        t: Target,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
        relative: [bool; 3],
        turn_riders: bool,
    ) -> Option<[f64; 3]> {
        use super::entity::Entity;
        let from = self.target_position(t);
        let v = Vec3::new(pos[0], pos[1], pos[2]);
        // The riders turn from the unclamped angles (calculatePassengerTransition
        // adds their offset to the transition's, each rider's own setXRot clamps).
        let requested = rotation;
        // Entity.teleportSameDimension teleports the riders before their vehicle.
        let riding = self.teleport_riders_first(t, v, relative, requested.filter(|_| turn_riders));
        // PositionMoveRotation.calculateAbsolute clamps pitch before setXRot.
        let rotation = if matches!(t, Target::Player | Target::Guest(_)) {
            requested // Player placement applies the clamp itself.
        } else {
            requested.map(|[yaw, pitch]| [yaw, pitch.clamp(-90.0, 90.0)])
        };
        match t {
            Target::General(id) => {
                if let Some(entity) = self.entity(id) {
                    self.entity_stop_riding(entity);
                }
                if let Some(row) = self.common.resident_entities.general.row_mut(id) {
                    row.position = v;
                    row.previous = v;
                    if let Some([yaw, pitch]) = rotation {
                        if yaw.is_finite() {
                            row.rotation[0] = yaw;
                        }
                        if !pitch.is_nan() {
                            row.rotation[1] = entity_pitch(pitch);
                        }
                    }
                    // Entity.teleportSetPosition ends with setOldPosAndRot.
                    row.previous_rotation = row.rotation;
                }
            }
            Target::Guest(_) => {
                self.for_player(t, |g| {
                    g.teleport_to_relative(Target::Player, pos, rotation, relative, turn_riders)
                });
            }
            Target::Player => {
                let from = self.place_player_teleport_in_level(v, rotation, relative);
                // A server's player teleported within their level is placed
                // there at once: the arrival they were waiting for elsewhere
                // in it (whose chunks lose their tickets) is abandoned.
                if let Some(conn) = self.server.lan_acting {
                    self.hold_for_chunks(conn, v);
                }
                // TeleportCommand.performTeleport grounds a player unless they're gliding.
                if let Some(p) = self.common.player.as_mut()
                    && !p.fall_flying
                {
                    p.ground.on_ground = true;
                }
                // A server player's teleport is told to the mods once their
                // move is in the hub (the put-back at the end of `guest_turn`).
                if let (Some(conn), Some(uuid), Some(name), Some(from)) = (
                    self.server.lan_acting,
                    self.server.lan_acting_uuid,
                    self.server.lan_acting_name.clone(),
                    from,
                ) {
                    self.server
                        .teleport_notes
                        .push(super::server_travel::TeleportNote {
                            conn,
                            uuid,
                            name,
                            from,
                            to: pos,
                            dimension: self.common.dim.id().to_owned(),
                        });
                }
            }
            Target::Mob(id) => {
                // Entity.teleport: stopRiding() before the entity is placed,
                // which a command's transition always does (a command's
                // transition is never asPassenger). A rider moved without this
                // is seated again by the next sync, so nothing moves.
                self.dismount_mob(id);
                if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                    m.ai.nav.stop();
                    m.velocity = teleport_velocity(m.velocity, relative, m.extra.fall_flying);
                    if !m.extra.fall_flying {
                        m.ground.on_ground = true;
                    }
                    sunrise_sim::mobs::living::teleports::snap_to(m, v);
                    m.needs_sync = true;
                    // Entity.teleportSetPosition: setYRot, setYHeadRot, setXRot
                    // (clamped to -90..90), then setOldRot.
                    if let Some([yaw, pitch]) = rotation {
                        if yaw.is_finite() {
                            m.yaw = yaw;
                            m.body_yaw = yaw;
                            m.head_yaw = yaw;
                        }
                        if !pitch.is_nan() {
                            m.head_pitch = entity_pitch(pitch);
                        }
                        m.previous_yaw = m.yaw;
                        m.previous_head_pitch = m.head_pitch;
                    }
                }
            }
            Target::Item(id) => {
                if let Some(i) = self.common.dropped.iter_mut().find(|i| i.id == id) {
                    i.position = v;
                    i.previous = v;
                    if let Some(r) = rotation {
                        Entity::set_rotation(i, r);
                    }
                }
            }
            Target::Orb(id) => {
                if let Some(o) = self.common.orbs.iter_mut().find(|o| o.id == id) {
                    o.position = v;
                    o.previous = v;
                    if let Some(r) = rotation {
                        Entity::set_rotation(o, r);
                    }
                }
            }
            Target::Projectile(id) => {
                if let Some(i) = self
                    .common
                    .creatures
                    .projectiles
                    .iter()
                    .position(|a| a.id == id)
                {
                    self.common.creatures.projectiles.update(i, |a| {
                        a.position = v;
                        a.previous = v;
                        if let Some(r) = rotation {
                            Entity::set_rotation(a, r);
                        }
                    });
                }
            }
            Target::Stand(id) => {
                if let Some(a) = self.common.armor_stands.iter_mut().find(|a| a.id == id) {
                    a.position = v;
                    a.previous = v;
                    // Entity.teleport's stopRiding(), as for a mob: a stand a
                    // server seats on a vehicle (a name tag hologram) leaves it.
                    a.vehicle = None;
                    // TeleportCommand uses the current view angles when rotation
                    // is omitted; teleportSetPosition still updates head yaw.
                    let angles = rotation.unwrap_or_else(|| a.rotation());
                    Entity::set_rotation(a, angles);
                }
            }
            Target::Boat(id) => {
                if let Some(i) = self.common.boats.iter().position(|b| b.id == id) {
                    self.common.boats.update(i, |b| {
                        b.position = v;
                        b.previous = v;
                        if let Some(r) = rotation {
                            Entity::set_rotation(b, r);
                        }
                    });
                }
            }
            Target::Cart(id) => {
                if let Some(i) = self.common.carts.iter().position(|c| c.id == id) {
                    self.common.carts.update(i, |c| {
                        c.position = v;
                        c.previous = v;
                        if let Some(r) = rotation {
                            Entity::set_rotation(c, r);
                        }
                    });
                }
            }
            Target::Fangs(id) => {
                if let Some(i) = self.common.fangs.list.iter().position(|e| e.id == id) {
                    self.common.fangs.list.update(i, |f| {
                        f.pos = [v.x, v.y, v.z];
                        if let Some([yaw, _]) = rotation.filter(|r| r[0].is_finite()) {
                            f.yaw = yaw;
                        }
                    });
                }
            }
            Target::Other(id) => {
                if let Some(i) = self.server.others.0.iter().position(|e| e.id == id) {
                    self.server.others.0.update(i, |o| {
                        o.position = v;
                        if let Some([yaw, pitch]) = rotation {
                            if yaw.is_finite() {
                                o.yaw = yaw;
                            }
                            if !pitch.is_nan() {
                                o.pitch = entity_pitch(pitch);
                            }
                        }
                    });
                }
            }
            Target::Crystal(id) => {
                if let Some(c) = self.common.crystals.iter_mut().find(|c| c.id == id) {
                    (c.position, c.fall.0) = (v, true); // performTeleport: setOnGround(true)
                    c.previous = v;
                }
            }
            Target::Dragon(id) => {
                if let Some(d) = self.common.dragon.as_mut().filter(|d| d.id == id) {
                    d.position = v;
                    d.previous = v;
                }
            }
            Target::Tnt(id) => {
                if let Some(t) = self.common.tnts.iter_mut().find(|t| t.id == id) {
                    // `PrimedTnt.teleport` includes same-level command teleports.
                    t.used_portal = true;
                    t.position = v;
                    t.previous = v;
                    if let Some(r) = rotation {
                        Entity::set_rotation(t, r);
                    }
                }
            }
            Target::Cushion(id) => {
                // BlockAttachedEntity.setPos: the block it rests on follows the position.
                if let Some(c) = self.common.cushions.iter_mut().find(|c| c.id == id) {
                    c.position = v;
                    c.pos = [v.x.floor() as i32, v.y.floor() as i32, v.z.floor() as i32];
                    if let Some(r) = rotation {
                        c.yaw = r[0];
                    }
                }
            }
            Target::Falling(id) => {
                if let Some(i) = self.common.falling.iter().position(|f| f.id == id) {
                    self.common.falling.update(i, |f| {
                        f.position = v;
                        f.previous = v;
                        if let Some(r) = rotation {
                            Entity::set_rotation(f, r);
                        }
                    });
                }
            }
            Target::Rocket(id) => {
                if let Some(r) = self.common.rockets.iter_mut().find(|r| r.id == id) {
                    r.position = v;
                    r.previous = v;
                    if let Some(rot) = rotation {
                        Entity::set_rotation(r, rot);
                    }
                }
            }
            Target::Trident(id) => {
                if let Some(t) = self.common.tridents.iter_mut().find(|t| t.id == id) {
                    t.position = v;
                    t.previous = v;
                    if let Some(r) = rotation {
                        Entity::set_rotation(t, r);
                    }
                }
            }
            Target::Spawner(id) => {
                if let Some(i) = self.common.ominous_spawners.iter().position(|e| e.id == id) {
                    self.common.ominous_spawners.update(i, |s| {
                        s.position = [v.x, v.y, v.z];
                    });
                }
            }
            Target::Cloud(id) => {
                if let Some(i) = self.common.breath_clouds.iter().position(|e| e.id == id) {
                    self.common.breath_clouds.update(i, |c| {
                        c.at = pos;
                    });
                }
            }
            Target::Bolt(id) => {
                if let Some(i) = self.common.bolts.iter().position(|e| e.id == id) {
                    self.common.bolts.update(i, |b| {
                        b.at = pos;
                        if let Some([yaw, pitch]) = rotation {
                            if yaw.is_finite() {
                                b.rotation[0] = yaw;
                            }
                            if !pitch.is_nan() {
                                b.rotation[1] = entity_pitch(pitch);
                            }
                        }
                    });
                }
            }
            // Wall-mounted entities do not move; a hook follows its own tick and a
            // dragon part its dragon's.
            Target::Frame(_) | Target::Painting(_) => self.teleport_hanging(t, v),
            Target::Hook(_) | Target::DragonPart(_) | Target::Knot(_) => {}
        }
        if !matches!(t, Target::Player | Target::Guest(_) | Target::Mob(_)) {
            target_entity_mut!(self, t, entity => {
                let velocity = entity.velocity_mut();
                *velocity = teleport_velocity(*velocity, relative, false);
            });
            if let Target::Dragon(id) = t
                && let Some(dragon) = self.common.dragon.as_mut().filter(|dragon| dragon.id == id)
            {
                dragon.teleport_motion(relative);
            }
        }
        self.announce_vehicle_teleport(t, &riding);
        from
    }

    /// `TeleportCommand.performTeleport`'s `EntityTeleportEvent.TeleportCommand`:
    /// where `t` goes in `level`, or None when a mod canceled the teleport.
    /// A target that is not a number keeps the command's. Only a server's
    /// entities with a network id are asked.
    pub(in crate::game::server_side) fn teleport_command_target(
        &mut self,
        t: Target,
        level: super::Dim,
        pos: [f64; 3],
    ) -> Option<[f64; 3]> {
        let (Some(entity), Some(from)) = (self.network_entity_id(t), self.target_position(t))
        else {
            return Some(pos);
        };
        if !self.native_authoritative() {
            return Some(pos);
        }
        let mut to = pos;
        let mut cancel = false;
        self.native_teleport_command(level.id(), entity as u32, from, &mut to, &mut cancel);
        if cancel {
            return None;
        }
        Some(if to.iter().all(|v| v.is_finite()) {
            to
        } else {
            pos
        })
    }
    /// `TimeCommand.getDefaultClock`: explicit registered clocks and dimension defaults are
    /// separate; built-in timelines and markers belong only to Overworld.
    pub(in crate::game::server_side) fn cmd_time(&mut self, id: &str, ctx: &Context) -> R<i32> {
        if id == "time.query.gametime" {
            let t = self.common.world.game_time;
            self.feedback_unbroadcast(Text::tr("commands.time.query.gametime", &[&t]));
            return Ok((t % i64::from(i32::MAX)) as i32);
        }
        // `time of <clock>` names the clock; otherwise the dimension's default clock.
        let clock = match ctx.get("clock") {
            Some(Arg::Resource(c)) => c.clone(),
            _ => match self.dimension_type().default_clock.clone() {
                Some(clock) => clock,
                None => {
                    return Err(err(
                        "commands.time.no_default_clock",
                        &[&self.common.dim.id()],
                    ));
                }
            },
        };
        let total = self.common.command_clock_time(&clock);
        match id {
            "time.set" => {
                let t = i64::from(Self::int(ctx, "time").unwrap_or(0));
                if total == t {
                    return Err(err("commands.time.set.already_at_time", &[&clock, &t]));
                }
                let mut cancel = false;
                self.native_time_skip(self.common.dim.id(), total, t, "command", &mut cancel);
                if !cancel {
                    self.common.set_command_clock_time(&clock, t);
                    self.common.command_clock_state(&clock).partial = 0.0;
                }
                self.feedback(Text::tr("commands.time.set.absolute", &[&clock, &t]));
                Ok(t as i32)
            }
            "time.set.marker" => {
                let Some(Arg::Resource(marker)) = ctx.get("timemarker").cloned() else {
                    return Ok(0);
                };
                let Some((m, target)) = self
                    .ui
                    .markers
                    .get(&(clock.clone(), marker.clone()))
                    .copied()
                    .and_then(|m| Some((m, m.resolve(total)?)))
                else {
                    return Err(err(
                        "commands.time.no_time_marker_found",
                        &[&marker, &clock],
                    ));
                };
                if m.occurs_at(total) {
                    return Err(err(
                        "commands.time.set.already_at_time_marker",
                        &[&clock, &marker],
                    ));
                }
                let mut cancel = false;
                self.native_time_skip(self.common.dim.id(), total, target, "command", &mut cancel);
                if !cancel {
                    self.common.set_command_clock_time(&clock, target);
                    self.common.command_clock_state(&clock).partial = 0.0;
                }
                self.feedback(Text::tr(
                    "commands.time.set.time_marker",
                    &[&clock, &marker],
                ));
                Ok((self.common.command_clock_time(&clock) % i64::from(i32::MAX)) as i32)
            }
            "time.add" => {
                let t = i64::from(Self::int(ctx, "time").unwrap_or(0));
                let target = (total + t).max(0);
                let mut cancel = false;
                self.native_time_skip(self.common.dim.id(), total, target, "command", &mut cancel);
                if !cancel {
                    self.common.set_command_clock_time(&clock, target);
                }
                let now = self.common.command_clock_time(&clock);
                self.feedback(Text::tr("commands.time.set.absolute", &[&clock, &now]));
                Ok((now % i64::from(i32::MAX)) as i32)
            }
            "time.pause" | "time.resume" => {
                let pause = id == "time.pause";
                if self.common.command_clock_state(&clock).paused == pause {
                    return Err(err(
                        if pause {
                            "commands.time.pause.already_paused"
                        } else {
                            "commands.time.pause.already_running"
                        },
                        &[&clock],
                    ));
                }
                self.common.command_clock_state(&clock).paused = pause;
                self.feedback(Text::tr(
                    if pause {
                        "commands.time.pause"
                    } else {
                        "commands.time.resume"
                    },
                    &[&clock],
                ));
                Ok(1)
            }
            "time.rate" => {
                let rate = match ctx.get("rate") {
                    Some(Arg::Float(r)) => *r,
                    _ => 1.0,
                };
                if self.common.command_clock_state(&clock).rate == rate {
                    return Err(err(
                        "commands.time.rate.already_same",
                        &[&clock, &float_to_string(rate)],
                    ));
                }
                self.common.command_clock_state(&clock).rate = rate;
                self.feedback(Text::tr(
                    "commands.time.rate",
                    &[&clock, &float_to_string(rate)],
                ));
                Ok(1)
            }
            "time.query.time" => {
                self.feedback_unbroadcast(Text::tr(
                    "commands.time.query.absolute",
                    &[&clock, &total],
                ));
                Ok((total % i64::from(i32::MAX)) as i32)
            }
            "time.query.timeline" | "time.query.repetition" => {
                let Some(Arg::Resource(timeline)) = ctx.get("timeline").cloned() else {
                    return Ok(0);
                };
                let Some(period) = self
                    .ui
                    .timelines
                    .get(&timeline)
                    .filter(|(c, _)| c == &clock)
                    .map(|(_, period)| *period)
                else {
                    return Err(err(
                        "commands.time.wrong_timeline_for_clock",
                        &[&timeline, &clock],
                    ));
                };
                if id == "time.query.timeline" {
                    let ticks = period.map_or(total, |p| total.rem_euclid(p));
                    self.feedback_unbroadcast(Text::tr(
                        "commands.time.query.timeline",
                        &[&timeline, &ticks],
                    ));
                    Ok(ticks as i32)
                } else {
                    let reps = period.map_or(0, |p| total / p);
                    self.feedback_unbroadcast(Text::tr(
                        "commands.time.query.timeline.repetitions",
                        &[&timeline, &reps],
                    ));
                    Ok(reps as i32)
                }
            }
            _ => Ok(0),
        }
    }
    pub(in crate::game::server_side) fn cmd_weather(&mut self, ctx: &Context) -> R<i32> {
        let duration = Self::int(ctx, "duration");
        let sample = |min: i32, max: i32, r: &mut sunrise_core::random::LegacyRandom| {
            min + r.next_int_bound(max - min + 1).unwrap_or(0)
        };
        let (key, clear, time, raining, thundering) = if ctx.has("clear") {
            (
                "commands.weather.set.clear",
                duration.unwrap_or_else(|| sample(12_000, 180_000, &mut self.common.world.random)),
                0,
                false,
                false,
            )
        } else if ctx.has("rain") {
            (
                "commands.weather.set.rain",
                0,
                duration.unwrap_or_else(|| sample(12_000, 24_000, &mut self.common.world.random)),
                true,
                false,
            )
        } else {
            (
                "commands.weather.set.thunder",
                0,
                duration.unwrap_or_else(|| sample(3_600, 15_600, &mut self.common.world.random)),
                true,
                true,
            )
        };
        if !self.set_weather_parameters(self.dim.id(), clear, time, raining, thundering) {
            return Err(err("command.failed", &[]));
        }
        self.feedback(Text::translate(key, &[]));
        Ok(duration.unwrap_or(-1))
    }
    pub fn kill_target(&mut self, t: Target) -> bool {
        // An entity in another level is killed in the level that holds it.
        self.in_target_level(t, |g| g.kill_target_in_level(t))
            .unwrap_or(false)
    }
    pub(in crate::game::server_side) fn cmd_kill(&mut self, ctx: &Context) -> R<i32> {
        let targets = if ctx.get("targets").is_some() {
            without_knots(self.entities(ctx, "targets", false)?)?
        } else {
            vec![self.implied_player()]
        };
        let single_display = (targets.len() == 1).then(|| {
            let t = targets[0];
            // The identity of an entity in another level is read there before
            // `kill_target` removes it from that level.
            match self.in_target_level(t, |g| g.capture_kill_display(t)) {
                Some(captured) => captured,
                None => self.capture_kill_display(t),
            }
        });
        for &t in &targets {
            self.kill_target(t);
        }
        let message = if let Some(captured) = single_display {
            let name = self.finish_kill_display(captured);
            Text::translate("commands.kill.success.single", &[name])
        } else {
            Text::tr("commands.kill.success.multiple", &[&targets.len()])
        };
        self.feedback(message);
        Ok(targets.len() as i32)
    }
    /// `Level.isInSpawnableBounds(BlockPos.containing(pos))`: shared by
    /// `/summon` and `/execute summon` (`SummonCommand.createEntity`).
    pub(in crate::game::server_side) fn check_summon_bounds(pos: [f64; 3]) -> R<()> {
        if !(-30_000_000.0..30_000_000.0).contains(&pos[0])
            || !(-20_000_000.0..20_000_000.0).contains(&pos[1])
            || !(-30_000_000.0..30_000_000.0).contains(&pos[2])
        {
            return Err(err("commands.summon.invalidPosition", &[]));
        }
        Ok(())
    }
    /// BlockStateArgument -> registry state.
    pub(in crate::game) fn block_state(&self, b: &BlockInput) -> R<BlockStateId> {
        let state = sunrise_world::chunk::BlockState {
            name: b.id.clone(),
            properties: b.properties.iter().cloned().collect(),
        };
        self.common
            .pack
            .assets
            .registry
            .resolve(&state)
            .map_err(|_| err("argument.block.id.invalid", &[&b.id]))
    }
    /// BlockPredicateArgument test of the block at `p`: its state, then any
    /// `{...}` against the block entity's `saveWithFullMetadata`
    /// (`NbtUtils.compareNbt(…, true)`); no block entity fails.
    pub(in crate::game::server_side) fn block_matches(&self, b: &BlockInput, p: [i32; 3]) -> bool {
        let state = self.common.world.block(p[0], p[1], p[2]);
        if !self.command_block_state_matches(b, state) {
            return false;
        }
        let Some(nbt) = &b.nbt else { return true };
        // (the argument's own grammar, `TagParser`, as the predicate was read)
        let Ok(pattern) = sunrise_command::snbt::parse_fully(nbt) else {
            return false;
        };
        sunrise_game_data::nbt_path::compare_nbt(&pattern, self.block_entity_nbt(p).as_ref())
    }
    /// The state part of `block_matches`: name or tag, then properties.
    pub(in crate::game) fn command_block_state_matches(
        &self,
        b: &BlockInput,
        state: BlockStateId,
    ) -> bool {
        block_predicate_matches_state(
            self.common.pack.assets.registry,
            [
                &self.common.res.items().block_tags,
                &self.items().block_tags,
            ],
            &b.id,
            b.tag,
            b.properties.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            state,
        )
    }
    /// `BlockInput.overwriteWithDefinedProperties`: restore explicit command
    /// properties after normalizing the state against neighbouring blocks.
    pub(in crate::game::server_side) fn block_input_state(
        &mut self,
        pos: [i32; 3],
        input: &BlockInput,
        state: BlockStateId,
        strict: bool,
    ) -> R<BlockStateId> {
        if strict {
            return Ok(state);
        }
        let updated = self.with_level(|b, level, ticks| {
            let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
            let updated = b.update_from_neighbour_shapes(level, ticks, at, state);
            if b.is_air(updated) { state } else { updated }
        });
        let registry = self.common.pack.assets.registry;
        let mut descriptor = registry
            .state(updated)
            .expect("registered block state")
            .clone();
        for (key, value) in &input.properties {
            if descriptor.properties.contains_key(key) {
                descriptor.properties.insert(key.clone(), value.clone());
            }
        }
        registry
            .resolve(&descriptor)
            .map_err(|_| err("argument.block.id.invalid", &[&input.id]))
    }
    /// Place a state like BlockInput.place: false when nothing changed.
    /// `BlockInput.place` for a block of a command. `command`: `/setblock` and `/fill` place with
    /// `UPDATE_CLIENTS | UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS`, so `Level.setBlock` notifies no neighbour,
    /// and send that notice themselves once the shape updates are done (`update_neighbours_on_block_set`):
    /// a block those shape updates pop off is gone before its neighbours are told of it.
    pub(in crate::game::server_side) fn place_state(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        strict: bool,
        command: bool,
    ) -> R<bool> {
        let previous = self.common.world.block(pos[0], pos[1], pos[2]);
        if previous == state {
            return Ok(false);
        }
        self.ensure_stateful_entity_table();
        self.sculk_prepare();
        // A different block replaces the block entity (Level.setBlock).
        let (old, new) = (
            self.common.res.facts().get(previous).name.clone(),
            self.common.res.facts().get(state).name.clone(),
        );
        // The command clears the old block entity without its side effects
        // (UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS); the new block gets a fresh one.
        let key = (pos[0], pos[1], pos[2]);
        if old != new {
            self.jukebox_removed(pos);
            self.common.world.block_entities.remove(&key);
        }
        if strict {
            self.with_level(|b, level, ticks| {
                b.set_block(
                    level,
                    ticks,
                    sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
                    state,
                    sunrise_world::level::flags::UPDATE_CLIENTS
                        | sunrise_world::level::flags::UPDATE_SKIP_ALL_SIDEEFFECTS,
                );
            });
            self.server_process_level_events()
                .map_err(|e| CommandError::new(Text::literal(e.to_string())))?;
        } else if command {
            self.level_set_with_flags(
                pos,
                state,
                sunrise_world::level::flags::UPDATE_CLIENTS
                    | sunrise_world::level::flags::UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS,
            )
            .map_err(|e| CommandError::new(Text::literal(e.to_string())))?;
        } else {
            self.level_set(pos, state)
                .map_err(|e| CommandError::new(Text::literal(e.to_string())))?;
        }
        // LevelChunk.setBlockState creates it when the block, not the state, is still
        // there after onPlace: a powered hopper turns itself `enabled=false`.
        let landed = self.common.world.block(pos[0], pos[1], pos[2]);
        let needs_entity =
            !self.common.world.block_entities.stateful_entity(landed.0) && old != new;
        if needs_entity
            && self.common.res.facts().get(landed).name == new
            && !self.sculk_place_entity(landed)
            && let Some(entity) = self.new_block_entity_for_state(landed)
        {
            self.common.world.block_entities.insert(key, entity);
        }
        Ok(true)
    }
    /// `ServerLevel.updateNeighboursOnBlockSet` (`Behaviours::update_neighbours_on_block_set`) for the
    /// block now at `pos`, which replaced `old`.
    pub(in crate::game) fn update_neighbours_on_block_set(
        &mut self,
        pos: [i32; 3],
        old: BlockStateId,
    ) -> R<()> {
        self.with_level(|b, level, ticks| {
            b.update_neighbours_on_block_set(
                level,
                ticks,
                sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
                old,
            );
        });
        self.server_process_level_events()
            .map_err(|e| CommandError::new(Text::literal(e.to_string())))
    }
    /// `Level.destroyBlock(pos, true)` through the behaviour library: the
    /// destroy event, the block's drops without a tool or breaker, then its
    /// fluid's block, the block entity spilling its contents on the way.
    pub fn destroy_block_dropping(&mut self, pos: [i32; 3]) -> R<()> {
        self.destroy_block_by(pos, None)
    }
    /// `Level.destroyBlock(pos, true, entity)`: [`Self::destroy_block_dropping`]
    /// with `entity` as the loot's `LootContextParams.THIS_ENTITY`.
    pub(in crate::game) fn destroy_block_by(
        &mut self,
        pos: [i32; 3],
        entity: Option<u32>,
    ) -> R<()> {
        let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        let entity = self.loot_entity_ref(entity);
        self.with_level(|b, level, ticks| {
            level.drop_entity = entity;
            level.drop_breaker_at = entity.map(|_| at);
            b.destroy_block(level, ticks, at, true)
        });
        self.server_process_level_events()
            .map_err(|e| CommandError::new(Text::literal(e.to_string())))
    }
    /// The block entity part of `BlockInput.place`: load the block entity
    /// from the argument's tag; true when its saved data changed.
    pub fn load_block_entity_nbt(&mut self, pos: [i32; 3], nbt: &str) -> bool {
        let Ok((tag, _)) = sunrise_core::nbt::parse_snbt(nbt) else {
            return false;
        };
        self.load_block_entity_tag(pos, tag)
    }

    /// `RandomizableContainer.unpackLootTable` for a player who opens a
    /// container: a roll the container still holds fires
    /// `player_generates_container_loot` for its table, and from then on the
    /// container saves its items, not the table.
    pub(in crate::game) fn unpack_opened_loot(&mut self, key: (i32, i32, i32), _name: &str) {
        let player = self.native_acting_player();
        self.unpack_loot(
            sunrise_gameplay_common::game::container_loot_level::LootTarget::Block(key),
            player,
        );
        if let Some((other, _)) = self.common.chest_partner(key) {
            self.unpack_loot(
                sunrise_gameplay_common::game::container_loot_level::LootTarget::Block(other),
                player,
            );
        }
    }
    /// Install a command's table without rolling. Raw NBT reads keep it pending.
    pub(in crate::game) fn take_loot_table(
        &mut self,
        key: (i32, i32, i32),
        tag: &sunrise_core::nbt::Tag,
        entity: &mut sunrise_level::containers::BlockEntity,
    ) -> bool {
        let block = self.data_block_name([key.0, key.1, key.2]);
        let pending = sunrise_sim::save::randomizable_container(entity, &block)
            .then(|| sunrise_sim::save::PendingContainerLoot::from_tag(tag))
            .flatten();
        let mut changed = false;
        self.common.world.with_chunk_extras(key, |extras| {
            changed = extras.pending_loot.get(&key) != pending.as_ref();
            extras.pending_loot.remove(&key);
            extras.pending_loot.extend(pending.map(|p| (key, p)));
        });
        changed
    }
    /// Raw NBT inspection overlays metadata without unpacking the inventory: a container's
    /// table, which a slotless mod block entity at `block` never has (its `LootTable` field is
    /// its own).
    pub fn write_pending_loot(
        world: &sunrise_level::world::World,
        key: (i32, i32, i32),
        block: &str,
        tag: &mut sunrise_core::nbt::Tag,
    ) {
        if !world
            .block_entities
            .get(&key)
            .is_some_and(|entity| sunrise_sim::save::randomizable_container(entity, block))
        {
            return;
        }
        if let sunrise_core::nbt::Tag::Compound(map) = tag {
            map.remove(&"LootTable".into());
            map.remove(&"LootTableSeed".into());
            if let Some(pending) = world
                .chunks
                .get(&(key.0 >> 4, key.2 >> 4))
                .and_then(|c| c.saved.as_ref())
                .and_then(|s| s.pending_loot.get(&key))
            {
                let items = map.get(&"Items".into()).cloned();
                pending.write_tags(map);
                if world
                    .block_entities
                    .get(&key)
                    .is_some_and(|e| e.slots().iter().any(Option::is_some))
                {
                    map.extend(items.map(|items| ("Items".into(), items)));
                }
            }
        }
    }
    /// `CloneCommands`: `from <sourceDimension>` is resolved as `begin` is
    /// read, `to <targetDimension>` as `destination` is
    /// (`DimensionArgument.getDimension`), each position checked loaded in its
    /// own level (`BlockPosArgument.getLoadedBlockPos`): an unknown source fails
    /// before an unknown target. The blocks are read in the source level and
    /// placed in the target level. The command's own level is back in place,
    /// whatever happened, before the command reports.
    pub(in crate::game::server_side) fn cmd_clone(&mut self, ctx: &Context) -> R<i32> {
        let here = self.common.dim;
        let result = self.clone_levels(ctx);
        if self.common.dim != here {
            self.enter_command_level(here);
        }
        if self.common.dim != here {
            log::error!("/clone could not return to {}", here.id());
            return Err(err("commands.clone.failed", &[]));
        }
        let count = result?;
        self.feedback(Text::tr("commands.clone.success", &[&count]));
        Ok(count)
    }
    /// `BlockPosArgument.getLoadedBlockPos` (`Level.isInWorldBounds`) in the
    /// level a dimension argument names, or, without one, the command's own.
    pub(in crate::game::server_side) fn clone_loaded(
        &mut self,
        dim: Option<Dim>,
        here: Dim,
        source_level: Dim,
        p: [i32; 3],
    ) -> R<()> {
        match dim {
            Some(dim) => self.clone_enter(dim)?,
            None => {
                self.clone_enter(here)?;
                if source_level != self.common.dim {
                    return Err(err("argument.pos.unloaded", &[]));
                }
            }
        }
        if !self
            .common
            .world
            .chunks
            .contains_key(&(p[0] >> 4, p[2] >> 4))
        {
            return Err(err("argument.pos.unloaded", &[]));
        }
        if p[1] < self.common.world.shape.min_y || p[1] >= self.common.world.shape.max_y() {
            return Err(err("argument.pos.outofworld", &[]));
        }
        Ok(())
    }
    /// `DimensionArgument.getDimension`: a built-in dimension, or
    /// `argument.dimension.invalid`; None when the argument is absent.
    pub(in crate::game::server_side) fn clone_dimension(
        &self,
        ctx: &Context,
        name: &str,
    ) -> R<Option<Dim>> {
        let Some(Arg::Resource(id)) = ctx.get(name) else {
            return Ok(None);
        };
        if !matches!(
            id.as_str(),
            "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
        ) {
            return Err(err("argument.dimension.invalid", &[id]));
        }
        Ok(Some(Dim::from_id(id)))
    }
    /// Put `dim`'s level in place for the clone, or fail it when it cannot be.
    pub(in crate::game) fn clone_enter(&mut self, dim: Dim) -> R<()> {
        if self.common.dim != dim {
            self.enter_command_level(dim);
        }
        if self.common.dim != dim {
            return Err(err("commands.clone.failed", &[]));
        }
        Ok(())
    }
    /// AdvancementCommands.perform / performCriterion.
    pub(in crate::game::server_side) fn cmd_advancement(&mut self, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "targets", true)?;
        self.load_advancements();
        let tree = self.common.adv.tree.clone();
        let grant = ctx.has("grant");
        let key = if grant { "grant" } else { "revoke" };
        let index = match ctx.get("advancement") {
            Some(Arg::Resource(id)) => Some(
                tree.find_id(id)
                    .ok_or_else(|| err("advancement.advancementNotFound", &[id]))?,
            ),
            _ => None,
        };
        if let (Some(i), Some(Arg::Str(criterion))) = (index, ctx.get("criterion")) {
            let a = &tree.list[i];
            let name = super::advancements::adv_name(a);
            if !a.criteria.iter().any(|c| &c.name == criterion) {
                return Err(CommandError::new(Text::translate(
                    "commands.advancement.criterionNotFound",
                    &[name, Text::literal(criterion.clone())],
                )));
            }
            // AdvancementCommands.performCriterion: each selected player's own turn.
            let results = self.advancement_turns(&targets, |g| {
                i32::from(if grant {
                    g.grant_advancement(i, Some(criterion))
                } else {
                    g.revoke_advancement(i, Some(criterion))
                })
            });
            let (who, many) = self.advancement_subject(&results);
            let to = if many { "many" } else { "one" };
            let args = [Text::literal(criterion.clone()), name, who];
            if results.iter().map(|r| r.1).sum::<i32>() == 0 {
                return Err(CommandError::new(Text::translate(
                    &format!("commands.advancement.{key}.criterion.to.{to}.failure"),
                    &args,
                )));
            }
            self.feedback(Text::translate(
                &format!("commands.advancement.{key}.criterion.to.{to}.success"),
                &args,
            ));
            return Ok(results.iter().map(|r| r.1).sum());
        }
        // getAdvancements: parents first (nearest up), the target, then
        // the subtree depth first.
        let list: Vec<usize> = match index {
            None => (0..tree.list.len()).collect(),
            Some(i) => {
                let mut out = Vec::new();
                if ctx.has("until") || ctx.has("through") {
                    let mut p = tree.list[i].parent.as_ref();
                    while let Some(pi) = p.and_then(|id| tree.find_id(id)) {
                        out.push(pi);
                        p = tree.list[pi].parent.as_ref();
                    }
                }
                out.push(i);
                if ctx.has("from") || ctx.has("through") {
                    let kids = tree.children();
                    fn add(kids: &[Vec<usize>], i: usize, out: &mut Vec<usize>) {
                        for &c in &kids[i] {
                            out.push(c);
                            add(kids, c, out);
                        }
                    }
                    add(&kids, i, &mut out);
                }
                out
            }
        };
        let quiet = index.is_none();
        // AdvancementCommands.perform: Action.perform for each selected player.
        let results = self.advancement_turns(&targets, |g| {
            let was_quiet = std::mem::replace(&mut g.adv.quiet, quiet);
            let mut count = 0;
            for &i in &list {
                let changed = if grant {
                    g.grant_advancement(i, None)
                } else {
                    g.revoke_advancement(i, None)
                };
                count += i32::from(changed);
            }
            g.adv.quiet = was_quiet;
            count
        });
        let total: i32 = results.iter().map(|r| r.1).sum();
        let (who, many) = self.advancement_subject(&results);
        let to = if many { "many" } else { "one" };
        let (what, first) = if list.len() == 1 {
            ("one", super::advancements::adv_name(&tree.list[list[0]]))
        } else {
            ("many", Text::literal(list.len().to_string()))
        };
        let args = [first, who];
        if total == 0 {
            return Err(CommandError::new(Text::translate(
                &format!("commands.advancement.{key}.{what}.to.{to}.failure"),
                &args,
            )));
        }
        self.feedback(Text::translate(
            &format!("commands.advancement.{key}.{what}.to.{to}.success"),
            &args,
        ));
        Ok(total)
    }
    /// Run `f` as each selected player's own turn and collect what it changed;
    /// a target without a turn counts as unchanged, like in `cmd_clear`.
    pub(in crate::game) fn advancement_turns(
        &mut self,
        targets: &[Target],
        f: impl Fn(&mut Self) -> i32,
    ) -> Vec<(Target, i32)> {
        targets
            .iter()
            .map(|&t| (t, self.for_player(t, &f).unwrap_or(0)))
            .collect()
    }
    /// The player argument of the advancement feedback: with the tracker's
    /// NON_ZERO elements on success and every element on failure, the one
    /// player's name, else the count (CommandResponseTracker.dispatch).
    pub(in crate::game) fn advancement_subject(&self, results: &[(Target, i32)]) -> (Text, bool) {
        let changed: Vec<Target> = results.iter().filter(|r| r.1 != 0).map(|r| r.0).collect();
        let picked: Vec<Target> = if changed.is_empty() {
            results.iter().map(|r| r.0).collect()
        } else {
            changed
        };
        match picked.as_slice() {
            [t] => (self.target_name(*t), false),
            _ => (Text::literal(picked.len().to_string()), true),
        }
    }
    pub(in crate::game::server_side) fn cmd_experience(&mut self, ctx: &Context) -> R<i32> {
        let levels = ctx.has("levels");
        let unit = if levels { "levels" } else { "points" };
        if ctx.has("query") {
            let target = self.entities(ctx, "target", true)?[0];
            // ExperienceCommand.queryExperience: the target's own level or points.
            let value = self
                .for_player(target, |g| {
                    let e = &g.experience;
                    if levels {
                        e.level
                    } else {
                        (e.progress * sunrise_sim::entities::Experience::needed(e.level) as f32)
                            as i32
                    }
                })
                .unwrap_or(0);
            self.feedback_unbroadcast(Text::translate(
                &format!("commands.experience.query.{unit}"),
                &[self.target_name(target), Text::literal(value.to_string())],
            ));
            return Ok(value);
        }
        let targets = self.entities(ctx, "target", true)?;
        let amount = Self::int(ctx, "amount").unwrap_or(0);
        let add = ctx.has("add");
        let op = if add { "add" } else { "set" };
        // Every target changes (a set of points can be refused for some).
        let changed: Vec<Target> = targets
            .iter()
            .copied()
            .filter(|&t| {
                self.for_player(t, |g| g.change_experience(add, levels, amount)) == Some(true)
            })
            .collect();
        if changed.is_empty() {
            return Err(err("commands.experience.set.points.invalid", &[]));
        }
        let (key, who) = match changed[..] {
            [only] => ("single", self.target_name(only)),
            _ => ("multiple", Text::literal(changed.len().to_string())),
        };
        self.feedback(Text::translate(
            &format!("commands.experience.{op}.{unit}.success.{key}"),
            &[Text::literal(amount.to_string()), who],
        ));
        Ok(changed.len() as i32)
    }
    /// ExperienceCommand's add and set on the player in place: false when
    /// the points to set do not fit the level.
    pub(in crate::game) fn change_experience(
        &mut self,
        add: bool,
        levels: bool,
        amount: i32,
    ) -> bool {
        if add {
            if levels {
                // Player.giveExperienceLevels handles the strict negative
                // boundary and emits the level-change hook itself.
                self.give_levels(amount);
                return true;
            } else {
                self.give_experience_points(amount);
            }
            return true;
        }
        if levels {
            self.set_experience_levels(amount);
        } else {
            let needed = sunrise_sim::entities::Experience::needed(self.common.experience.level);
            if amount > needed - 1 {
                return false;
            }
            self.common.experience.progress = amount as f32 / needed as f32;
        }
        true
    }
    /// `ServerPlayer.setExperienceLevels` writes an absolute level without granting experience.
    pub(in crate::game::server_side) fn set_experience_levels(&mut self, level: i32) {
        let old_level = self.common.experience.level;
        self.common.experience.level = level;
        self.native_player_level_changed(old_level);
    }

    /// `GameRules.set` as the `/gamerule` command and the management protocol call it: the rule changes, the
    /// world follows it, and a dedicated server tells the event stream (`MinecraftServer.onGameRuleChanged`
    /// tells the management protocol's clients).
    pub fn set_game_rule(
        &mut self,
        rule: &gamerules::Rule,
        value: Value,
        actor: Option<mod_api::PlayerRef>,
    ) {
        if self.common.ui.rules.set(&rule.name, value) {
            self.game_rule_changed(&rule.name, value, actor);
        }
        self.common.sync_rules();
        if self.common.is_dedicated_server() {
            let (value_type, json) = match value {
                Value::Bool(b) => ("boolean", serde_json::json!(b)),
                Value::Int(i) => ("integer", serde_json::json!(i)),
            };
            super::dedicated::events::record_server(
                "gamerule",
                serde_json::json!({
                    "key": rule.name.clone(),
                    "valueType": value_type,
                    "value": json,
                }),
            );
        }
    }
    pub(in crate::game::server_side) fn cmd_spawnpoint(&mut self, ctx: &Context) -> R<i32> {
        let targets = if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", true)?
        } else {
            vec![Target::Player]
        };
        let pos = if ctx.get("pos").is_some() {
            self.spawnable_pos_arg(ctx, "pos")?
        } else {
            let p = self.source_position();
            [
                p[0].floor() as i32,
                p[1].floor() as i32,
                p[2].floor() as i32,
            ]
        };
        let rot = match ctx.get("rotation") {
            Some(Arg::Coords(c)) => c.rotation(self.source_rotation()),
            _ => [0.0, 0.0],
        };
        let yaw = ((rot[0] + 180.0).rem_euclid(360.0)) - 180.0;
        let pitch = rot[1].clamp(-90.0, 90.0);
        // SetSpawnCommand: a forced point in the source's level, for each target.
        let level = self.common.dim;
        for &t in &targets {
            self.for_player(t, |g| {
                g.set_respawn_point(Some(super::respawn_anchor::RespawnConfig::forced(
                    level, pos, yaw, pitch,
                )));
            });
        }
        let dim = Text::literal(self.common.dim.id());
        let p = pos.map(|v| Text::literal(v.to_string()));
        let mut args = vec![
            p[0].clone(),
            p[1].clone(),
            p[2].clone(),
            Text::literal(float_to_string(yaw)),
            Text::literal(float_to_string(pitch)),
            dim,
        ];
        let key = if targets.len() == 1 {
            args.push(self.target_name(targets[0]));
            "commands.spawnpoint.success.single"
        } else {
            args.push(Text::literal(targets.len().to_string()));
            "commands.spawnpoint.success.multiple"
        };
        self.feedback(Text::translate(key, &args));
        Ok(targets.len() as i32)
    }
    /// `LevelData.setRespawnData(RespawnData.of(..))`: the world's spawn (level.dat `spawn`)
    /// moves to `pos`, its yaw wrapped and its pitch clamped, and `spawn_change` fires.
    pub(in crate::game::server_side) fn set_world_spawn(
        &mut self,
        dimension: Dim,
        pos: [i32; 3],
        [yaw, pitch]: [f32; 2],
    ) -> [f32; 2] {
        let rot = [wrap_degrees(yaw), pitch.clamp(-90.0, 90.0)];
        let previous_point = self.world_spawn_position().map(|v| v.floor() as i32);
        let old_angle = self.common.level.world_spawn_angle;
        let old_level = self.world_spawn_level();
        let old = self.common.level.world_spawn.replace(pos);
        self.common.level.world_spawn_angle = rot;
        self.common.level.world_spawn_dimension =
            (dimension != Dim::Overworld).then(|| dimension.id().to_owned());
        if previous_point != pos
            || old_angle.map(f32::to_bits) != rot.map(f32::to_bits)
            || old_level != dimension
        {
            self.lan_broadcast_world_spawn();
        }
        self.native_spawn_change(dimension.id(), pos, rot);
        // A player whose fallback is the world spawn (no spawnpoint of its
        // own; a bed or an anchor that is gone falls back to it) respawns at
        // the new one.
        let floor = |s: Vec3| [s.x.floor() as i32, s.y.floor() as i32, s.z.floor() as i32];
        if self.common.spawn.map(floor) == old {
            self.common.spawn = Some(Vec3::new(
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]),
                f64::from(pos[2]) + 0.5,
            ));
        }
        rot
    }
    pub(in crate::game) fn item_matches(&self, predicate: &ItemInput, item: &str) -> bool {
        if predicate.any {
            return true;
        }
        if predicate.tag {
            let tag = predicate.id.trim_start_matches("minecraft:");
            return self
                .common
                .res
                .items()
                .item_tags
                .get(tag)
                .or_else(|| self.items().item_tags.get(&predicate.id))
                .is_some_and(|s| {
                    s.contains(item) || s.contains(item.trim_start_matches("minecraft:"))
                });
        }
        predicate.id == item
    }
    /// `ItemPredicateArgument`: the item test and its `[...]` component
    /// tests (presence, `=` equality, `!` negation, `|` alternatives).
    pub fn stack_matches(&self, predicate: &ItemInput, stack: &ItemStack) -> bool {
        if !self.item_matches(predicate, &stack.item) {
            return false;
        }
        let Some(tests) = predicate.components.as_deref() else {
            return true;
        };
        let inner = tests
            .trim()
            .strip_prefix('[')
            .and_then(|t| t.strip_suffix(']'))
            .unwrap_or("");
        let components = match sunrise_items::item_data::component_snapshot(stack) {
            Some(sunrise_core::nbt::Tag::Compound(c)) => c,
            _ => Default::default(),
        };
        let test = |term: &str| -> bool {
            let term = term.trim();
            let (negated, term) = match term.strip_prefix('!') {
                Some(t) => (true, t.trim()),
                None => (false, term),
            };
            let split = term.find(['=', '~']).unwrap_or(term.len());
            let id = term[..split].trim();
            let id = if id.contains(':') {
                id.to_owned()
            } else {
                format!("minecraft:{id}")
            };
            let present = components.get(&id.as_str().into());
            let result = match term[split..].chars().next() {
                // Both count wrappers in `ItemPredicateArgument` use the live
                // stack count and the same integer bounds matcher.
                Some('=' | '~') if id == "minecraft:count" => {
                    let value = sunrise_core::nbt::parse_snbt(term[split + 1..].trim());
                    value.is_ok_and(|(v, _)| item_count::matches(stack.count, &v))
                }
                Some('=') => {
                    let value = sunrise_core::nbt::parse_snbt(term[split + 1..].trim())
                        .ok()
                        .map(|(t, _)| t);
                    match (present, value) {
                        (Some(have), Some(want)) => {
                            sunrise_game_data::nbt_path::compare_nbt(&want, Some(have))
                                && sunrise_game_data::nbt_path::compare_nbt(have, Some(&want))
                        }
                        _ => false,
                    }
                }
                Some('~') => {
                    // Component sub-predicates.
                    let value = sunrise_core::nbt::parse_snbt(term[split + 1..].trim())
                        .ok()
                        .map(|(t, _)| t);
                    match (id.as_str(), value) {
                        // `MinMaxBounds.Ints.matches`: DamagePredicate's damage-only
                        // Int bounds shape; effective DAMAGE presence is required.
                        // Other fields keep the entire existing predicate path.
                        ("minecraft:damage", Some(ref v))
                            if matches!(v, sunrise_core::nbt::Tag::Compound(c) if c.len() == 1)
                                && match v.get("damage") {
                                    Some(sunrise_core::nbt::Tag::Int(_)) => true,
                                    Some(sunrise_core::nbt::Tag::Compound(bounds)) => {
                                        bounds.iter().all(|(key, value)| {
                                            matches!(key.to_string_lossy().as_str(), "min" | "max")
                                                && matches!(value, sunrise_core::nbt::Tag::Int(_))
                                        })
                                    }
                                    _ => false,
                                } =>
                        {
                            let removed = stack
                                .data()
                                .is_some_and(|data| data.other("!minecraft:damage").is_some());
                            let has_damage = !removed
                                && (present.is_some()
                                    || self.items().get(&stack.item).is_some_and(|item| {
                                        item.components.iter().any(|c| c == "minecraft:damage")
                                    }));
                            let n = i64::from(stack.damage);
                            has_damage
                                && match v.get("damage").unwrap() {
                                    sunrise_core::nbt::Tag::Int(exact) => n == i64::from(*exact),
                                    bounds => {
                                        let get = |key| match bounds.get(key) {
                                            Some(sunrise_core::nbt::Tag::Int(value)) => {
                                                Some(i64::from(*value))
                                            }
                                            _ => None,
                                        };
                                        get("min").is_none_or(|min| n >= min)
                                            && get("max").is_none_or(|max| n <= max)
                                    }
                                }
                        }
                        (_, Some(v)) => present.is_some_and(|have| {
                            sunrise_game_data::nbt_path::compare_nbt(&v, Some(have))
                        }),
                        _ => false,
                    }
                }
                // Count is a pseudo-component, not a catalog component.
                _ if id == "minecraft:count" => present.is_some(),
                _ => self.stack_component_present(stack, &id),
            };
            result != negated
        };
        split_top_level(inner, ',')
            .iter()
            .filter(|t| !t.trim().is_empty())
            .all(|term| split_top_level(term, '|').iter().any(|alt| test(alt)))
    }
    /// `ItemPredicateArgument.createComponentExistencePredicate`: the stack's
    /// effective component presence, with its patch ahead of its prototype.
    pub(in crate::game) fn stack_component_present(&self, stack: &ItemStack, id: &str) -> bool {
        // Read the modeled patch before saved-default elision. A removal wins
        // even when the actual item prototype supplies the component.
        if let Some(patch) = sunrise_items::item_data::components_tag(stack.damage, stack.data()) {
            if patch.get(&format!("!{id}")).is_some() {
                return false;
            }
            if patch.get(id).is_some() {
                return true;
            }
        }
        self.items()
            .get(&stack.item)
            .is_some_and(|item| item.components.iter().any(|key| key == id))
    }
    pub(in crate::game::server_side) fn cmd_clear(&mut self, ctx: &Context) -> R<i32> {
        let targets = if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", true)?
        } else {
            vec![Target::Player]
        };
        let predicate = match ctx.get("item") {
            Some(Arg::Item(i)) => i.clone(),
            _ => ItemInput {
                id: "*".into(),
                tag: false,
                any: true,
                components: None,
            },
        };
        let max = Self::int(ctx, "maxCount").unwrap_or(-1);
        let counting = max == 0;
        // ClearInventoryCommands.clearInventory: each player on their own.
        let mut removed = 0i32;
        let mut found = Vec::new();
        for &t in &targets {
            let n = self
                .for_player(t, |g| g.clear_matching(&predicate, max))
                .unwrap_or(0);
            removed += n;
            if n != 0 {
                found.push(t);
            }
        }
        if removed == 0 {
            return Err(if targets.len() == 1 {
                CommandError::new(Text::translate(
                    "clear.failed.single",
                    &[self.target_name(targets[0])],
                ))
            } else {
                err("clear.failed.multiple", &[&targets.len()])
            });
        }
        let kind = if counting { "test" } else { "success" };
        let (key, who) = match found[..] {
            [only] => ("single", self.target_name(only)),
            _ => ("multiple", Text::literal(found.len().to_string())),
        };
        self.feedback(Text::translate(
            &format!("commands.clear.{kind}.{key}"),
            &[Text::literal(removed.to_string()), who],
        ));
        Ok(removed)
    }
    /// Inventory.clearOrCountMatchingItems for the player in place: `max`
    /// of the matching items go (all when negative, none and only counted
    /// when 0). Returns how many matched.
    pub(in crate::game) fn clear_matching(&mut self, predicate: &ItemInput, max: i32) -> i32 {
        let counting = max == 0;
        // Inventory slots 0..40: main, armor feet to head, off hand;
        // then clearOrCountMatchingItems visits the crafting grid and carried stack.
        let mut removed = 0i32;
        let mut budget = if max < 0 { i32::MAX } else { max };
        let all = |g: &Self| -> Vec<Option<ItemStack>> {
            let mut v: Vec<Option<ItemStack>> = g.inventory.clone();
            v.extend(g.armor.iter().rev().cloned());
            v.push(g.offhand.clone());
            v.extend(g.craft_grid.iter().cloned());
            v.push(g.carried.clone());
            v
        };
        let matching: Vec<bool> = all(self)
            .iter()
            .map(|s| s.as_ref().is_some_and(|s| self.stack_matches(predicate, s)))
            .collect();
        let mut slots: Vec<&mut Option<ItemStack>> = Vec::new();
        slots.extend(self.common.inventory.iter_mut());
        slots.extend(self.common.armor.iter_mut().rev());
        slots.push(&mut self.common.offhand);
        slots.extend(self.common.craft_grid.iter_mut());
        slots.push(&mut self.common.carried);
        for (slot, matches) in slots.into_iter().zip(matching) {
            if !matches {
                continue;
            }
            let Some(stack) = slot.as_mut() else { continue };
            let take = if counting {
                stack.count as i32
            } else {
                (stack.count as i32).min(budget)
            };
            removed += take;
            if !counting {
                budget -= take;
                stack.count -= take as u32;
                if stack.count == 0 {
                    *slot = None;
                }
                if budget == 0 {
                    break;
                }
            }
        }
        removed
    }
    /// `LocateCommand.locatePoi`: `PoiManager.findClosestWithType` within
    /// 256 blocks of the source, over the loaded chunks.
    pub(in crate::game::server_side) fn cmd_locate_poi(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(poi)) = ctx.get("poi").cloned() else {
            return Ok(0);
        };
        let wanted: Vec<String> = match poi.strip_prefix('#') {
            Some(tag) => self.tag_entries("point_of_interest_type", tag),
            None => vec![poi.clone()],
        };
        let from = self.source_position().map(|v| v.floor() as i32);
        const RANGE: i32 = 256;
        let mut cache: std::collections::HashMap<u32, Option<&str>> =
            std::collections::HashMap::new();
        let mut best: Option<(i64, [i32; 3], &str)> = None;
        let (cx0, cz0) = ((from[0] - RANGE) >> 4, (from[2] - RANGE) >> 4);
        let (cx1, cz1) = ((from[0] + RANGE) >> 4, (from[2] + RANGE) >> 4);
        for cx in cx0..=cx1 {
            for cz in cz0..=cz1 {
                let Some(chunk) = self.common.world.chunks.get(&(cx, cz)) else {
                    continue;
                };
                for (i, section) in chunk.sections.iter().enumerate() {
                    let mut wanted_state = |state: u32| {
                        cache
                            .entry(state)
                            .or_insert_with(|| {
                                self.poi_type(sunrise_world::registry::BlockStateId(state))
                                    .filter(|t| wanted.iter().any(|w| w == t))
                            })
                            .is_some()
                    };
                    if !section.blocks.maybe_has(&mut wanted_state) {
                        continue;
                    }
                    let base_y = self.common.world.bottom_y() + 16 * i as i32;
                    for j in 0..4096 {
                        let state = section.get(j);
                        let Some(kind) = cache.get(&state).copied().flatten() else {
                            continue;
                        };
                        let p = [
                            cx * 16 + (j & 15) as i32,
                            base_y + (j >> 8) as i32,
                            cz * 16 + ((j >> 4) & 15) as i32,
                        ];
                        let d = [p[0] - from[0], p[1] - from[1], p[2] - from[2]]
                            .map(|d| i64::from(d) * i64::from(d))
                            .iter()
                            .sum::<i64>();
                        if d <= i64::from(RANGE * RANGE) && best.is_none_or(|(b, _, _)| d < b) {
                            best = Some((d, p, kind));
                        }
                    }
                }
            }
        }
        let Some((_, p, kind)) = best else {
            return Err(CommandError::new(Text::tr(
                "commands.locate.poi.not_found",
                &[&poi],
            )));
        };
        let link = Text::teleport_link(p[0], None, p[2]);
        let (dx, dz) = (p[0] - from[0], p[2] - from[2]);
        let distance = ((dx * dx + dz * dz) as f32).sqrt().floor() as i32;
        let shown = if poi.starts_with('#') {
            format!("{poi} ({kind})")
        } else {
            poi
        };
        self.feedback(Text::translate(
            "commands.locate.poi.success",
            &[
                Text::literal(shown),
                link,
                Text::literal(distance.to_string()),
            ],
        ));
        Ok(distance)
    }
    pub(in crate::game::server_side) fn cmd_locate(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let origin = self.source_position();
        if id == "locate.biome" {
            let Some(Arg::Resource(biome)) = ctx.get("biome").cloned() else {
                return Ok(0);
            };
            // LocateCommand.locateBiome: BiomeSource.findClosestBiome3d with
            // a 6400-block radius, 32-block columns and 64-block layers.
            let from = origin.map(|v| v.floor() as i32);
            let allowed = |name: &str| self.biome_matches(&biome, name);
            let search = sunrise_world::worldgen::generator::BiomeSearch {
                allowed: &allowed,
                origin: from,
                radius: 6400,
                horizontal: 32,
                vertical: 64,
                min_y: self.common.world.bottom_y(),
                max_y: self.common.world.top_y() - 1,
            };
            let found = self
                .common
                .end
                .locator()
                .and_then(|t| t.find_closest_biome(&search));
            let Some((name, p)) = found else {
                return Err(CommandError::new(Text::tr(
                    "commands.locate.biome.not_found",
                    &[&biome],
                )));
            };
            let d2 = [p[0] - from[0], p[1] - from[1], p[2] - from[2]]
                .map(|d| f64::from(d) * f64::from(d))
                .iter()
                .sum::<f64>();
            let shown = if biome.starts_with('#') {
                format!("{biome} ({name})")
            } else {
                biome.clone()
            };
            let link = Text::teleport_link(p[0], Some(p[1]), p[2]);
            let distance = (d2 as f32).sqrt().floor() as i32;
            self.feedback(Text::translate(
                "commands.locate.biome.success",
                &[
                    Text::literal(shown),
                    link,
                    Text::literal(distance.to_string()),
                ],
            ));
            return Ok(distance);
        }
        let Some(Arg::Resource(s)) = ctx.get("structure").cloned() else {
            return Ok(0);
        };
        let names = self.structure_names(&s);
        let from = origin.map(|v| v.floor() as i32);
        let found = self
            .common
            .end
            .locator()
            .and_then(|t| t.locate(&names, from, 100));
        let Some((name, p)) = found else {
            return Err(CommandError::new(Text::tr(
                "commands.locate.structure.not_found",
                &[&s],
            )));
        };
        let link = Text::teleport_link(p[0], None, p[2]);
        let (dx, dz) = (f64::from(p[0] - from[0]), f64::from(p[2] - from[2]));
        let distance = (dx * dx + dz * dz).sqrt().floor() as i32;
        let shown = if s.starts_with('#') {
            format!("{s} ({name})")
        } else {
            name
        };
        self.feedback(Text::translate(
            "commands.locate.structure.success",
            &[
                Text::literal(shown),
                link,
                Text::literal(distance.to_string()),
            ],
        ));
        Ok(distance)
    }
    /// The structures `LocateCommand.locateStructure` searches for: an id, or
    /// a `#tag`'s members.
    pub(in crate::game) fn structure_names(&self, structure: &str) -> Vec<String> {
        match structure.strip_prefix('#') {
            Some(tag) => self.structure_tag(tag),
            None => vec![structure.to_owned()],
        }
    }
    /// The structures of a `worldgen/structure` tag, nested tags expanded.
    pub fn structure_tag(&self, tag: &str) -> Vec<String> {
        self.tag_entries("worldgen/structure", tag)
    }
    /// ListPlayersCommand: every online player in join order (a single-player
    /// owner is one of them), with the count and the max.
    pub(in crate::game::server_side) fn cmd_list(&mut self, ctx: &Context) -> R<i32> {
        let players: Vec<(u128, String)> = self
            .server
            .lan
            .iter()
            .flat_map(|h| h.hub.online_players())
            .map(|g| (g.uuid, g.name.clone()))
            .collect();
        let max = self
            .server
            .lan
            .as_ref()
            .map_or(sunrise_sim::lan::MAX_PLAYERS, |h| {
                h.hub.lan.shared.max_players()
            });
        let uuids = ctx.has("uuids");
        let mut names = Text::empty();
        for (k, (uuid, name)) in players.iter().enumerate() {
            if k > 0 {
                // ComponentUtils.formatList: gray commas.
                names.push(Text::colored(", ", text::GRAY));
            }
            if uuids {
                names.push(Text::translate(
                    "commands.list.nameAndId",
                    &[
                        Text::literal(name.as_str()),
                        Text::literal(sunrise_protocol::crypto::uuid_string(*uuid)),
                    ],
                ));
            } else {
                names.push(self.common.player_text(name, *uuid));
            }
        }
        let count = players.len();
        self.feedback_unbroadcast(Text::translate(
            "commands.list.players",
            &[
                Text::literal(count.to_string()),
                Text::literal(max.to_string()),
                names,
            ],
        ));
        Ok(count as i32)
    }
    /// MessageArgument: `@p`-style selectors in a message become names.
    pub(in crate::game) fn expand_selectors(&mut self, raw: &str) -> Text {
        // MessageArgument uses the literal before the first selector as the component root.
        let mut out: Option<Text> = None;
        let mut pending = String::new();
        let mut rest = raw;
        while let Some(at) = rest.find('@') {
            pending.push_str(&rest[..at]);
            let mut reader = Reader::new(&rest[at..]);
            match (sunrise_command::ArgType::Entity {
                single: false,
                players: false,
            })
            .parse(&mut reader, &self.common.ui.registries)
            {
                Ok(Arg::Entity(s)) if reader.cursor > 1 => {
                    let found = self.select(&s);
                    let text =
                        out.get_or_insert_with(|| Text::literal(std::mem::take(&mut pending)));
                    if !pending.is_empty() {
                        text.push(std::mem::take(&mut pending));
                    }
                    for (i, t) in found.iter().enumerate() {
                        if i > 0 {
                            text.push(", ");
                        }
                        text.push(self.target_name(*t));
                    }
                    rest = &rest[at + reader.cursor..];
                }
                _ => {
                    pending.push('@');
                    rest = &rest[at + 1..];
                }
            }
        }
        pending.push_str(rest);
        match out {
            None => Text::literal(pending),
            Some(mut text) => {
                if !pending.is_empty() {
                    text.push(pending);
                }
                text
            }
        }
    }
}

/// Split at a separator outside brackets, braces and quotes.
pub(in crate::game) fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut quote, mut escaped, mut start) = (0i32, None::<char>, false, 0);
    for (i, ch) in s.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '[' | '{' => depth += 1,
            ']' | '}' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(&s[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// The names `native_difficulty_changed` hears for a change from `from` to
/// `to`: the four difficulties of `options.difficulty.*`, the index kept in
/// range as the command and the options keep it.
pub(in crate::game) fn difficulty_names(from: u8, to: u8) -> (&'static str, &'static str) {
    (
        DIFFICULTIES[usize::from(from) & 3],
        DIFFICULTIES[usize::from(to) & 3],
    )
}

#[cfg(test)]
pub(in crate::game) mod difficulty_tests {
    use super::difficulty_names;

    #[test]
    fn a_difficulty_change_names_both_ends() {
        assert_eq!(difficulty_names(0, 3), ("peaceful", "hard"));
        assert_eq!(difficulty_names(2, 1), ("normal", "easy"));
        // The four difficulties are the only indexes the command and the
        // options keep; an index outside them still names one.
        assert_eq!(difficulty_names(7, 4), ("hard", "peaceful"));
    }
}

#[cfg(test)]
pub(in crate::game) mod summon_tests {
    #[test]
    fn a_summoned_entity_is_named_from_its_own_namespace() {
        use super::entity_name_key;
        assert_eq!(
            entity_name_key("minecraft:snowball"),
            "entity.minecraft.snowball"
        );
        assert_eq!(entity_name_key("ruby:bolt"), "entity.ruby.bolt");
        assert_eq!(entity_name_key("ruby:spell/bolt"), "entity.ruby.spell.bolt");
    }
}

#[cfg(test)]
pub(in crate::game) mod look_at_tests {
    use super::Target;
    use super::targets::non_player_entity_id;

    #[test]
    fn look_at_entity_form_names_any_entity_by_its_id() {
        for t in [
            Target::Mob(7),
            Target::Item(7),
            Target::Orb(7),
            Target::Projectile(7),
            Target::Boat(7),
            Target::Cart(7),
            Target::Crystal(7),
            Target::Dragon(7),
            Target::Tnt(7),
            Target::Cushion(7),
            Target::Falling(7),
            Target::Rocket(7),
            Target::Trident(7),
            Target::Spawner(7),
            Target::Cloud(7),
            Target::Bolt(7),
        ] {
            assert_eq!(non_player_entity_id(t), Some(7), "{t:?}");
        }
        // Players are the hub's to name; a dragon part's id is no entity id.
        assert_eq!(non_player_entity_id(Target::Player), None);
        assert_eq!(non_player_entity_id(Target::Guest(3)), None);
        assert_eq!(non_player_entity_id(Target::Mob(u32::MAX - 4096)), None);
    }
}

#[cfg(test)]
pub(in crate::game) mod percent_f_tests {
    use super::percent_f;

    #[test]
    fn rounds_the_shortest_decimal_half_up_like_java() {
        // WorldBorderCommand: "%.1f" distance, "%.2f" buffer, "%.0f" size.
        assert_eq!(percent_f(10.35, 1), "10.4");
        assert_eq!(percent_f(5.125, 2), "5.13");
        assert_eq!(percent_f(10.5, 0), "11");
        assert_eq!(percent_f(2.5, 0), "3");
        // TeleportCommand.formatDouble: "%f".
        assert_eq!(percent_f(0.1234565, 6), "0.123457");
        assert_eq!(percent_f(-1.005, 2), "-1.01");
        assert_eq!(percent_f(3.0, 2), "3.00");
    }
}

#[cfg(test)]
pub(in crate::game) mod query_feedback_tests {}

#[cfg(test)]
pub(in crate::game) mod forced_respawn_tests {

    #[test]
    #[ignore = "daily"]
    fn overworld_spawnpoint_is_validated_at_both_player_blocks() {
        use crate::game::{native_guest_turn_tests::with_guests, respawn_anchor::RespawnConfig};
        use sunrise_level::chunk::Chunk;

        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_guests(|game| {
                use sunrise_gameplay_common::game::{
                    chat_common::ChatCommonGameExt1 as _,
                    commands_common::CommandsCommonGameExt2 as _,
                };
                game.build_registries();
                for x in -2..=2 {
                    for z in -2..=2 {
                        game.server
                            .streaming
                            .insert_remote(&mut game.common.world, Chunk::empty(x, z));
                    }
                }
                game.level.world_spawn = Some([0, 80, 0]);
                game.level.world_spawn_angle = [-30.0, -5.0];
                let registry = game.pack.assets.registry;
                let air = registry.default_state("minecraft:air").unwrap();
                // Liquid is a block property: aquatic plants such as kelp still permit
                // a forced point, while a bubble column does not, at feet or head.
                for (block, allowed) in [
                    ("air", true),
                    ("water", false),
                    ("lava", false),
                    ("bubble_column", false),
                    ("kelp", true),
                    ("stone", false),
                ] {
                    let state = registry
                        .default_state(&format!("minecraft:{block}"))
                        .unwrap();
                    for height in [0, 1] {
                        game.world.set(5, 64, 5, air).unwrap();
                        game.world.set(5, 65, 5, air).unwrap();
                        game.world.set(5, 64 + height, 5, state).unwrap();
                        assert!(
                            game.as_console(|g| g.run_command("spawnpoint guest12 5 64 5 45 10"))
                        );
                        let config = game
                            .command_guest(12)
                            .unwrap()
                            .respawn_elsewhere
                            .as_ref()
                            .and_then(RespawnConfig::from_saved)
                            .unwrap();
                        assert!(config.forced);
                        assert_eq!(config.pos, [5, 64, 5]);
                        game.server.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                            g.dead = true;
                            g.health = 0.0;
                        });
                        game.guest_respawn(12);
                        let guest = game.command_guest(12).unwrap();
                        assert!(!guest.dead, "{block} at height {height}");
                        assert_eq!(
                            guest.respawn_elsewhere.is_some(),
                            allowed,
                            "{block} at height {height}"
                        );
                        assert_eq!(game.level.world_spawn, Some([0, 80, 0]));
                        if allowed {
                            assert_eq!(guest.position, [5.5, 64.1, 5.5]);
                            assert_eq!([guest.yaw, guest.pitch], [45.0, 10.0]);
                        } else {
                            // The missing point must not supply the fallback's facing.
                            assert_eq!([guest.yaw, guest.pitch], [-30.0, -5.0]);
                            assert_eq!(game.level.world_spawn_angle, [-30.0, -5.0]);
                        }
                    }
                }
            });
        });
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::SharedGameExt1 as _;
