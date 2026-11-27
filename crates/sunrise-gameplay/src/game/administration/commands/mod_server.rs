use super::{Dim, vehicle_teleport::TeleportMotion};
use crate::game::commands::OtherEntity;
use crate::game::commands::R;
use crate::game::commands::Target;
use crate::game::commands::err;
use crate::game::commands::format_double;
use crate::game::commands::formatted_command_failure;
use crate::game::commands::percent_f;
use crate::game::commands::target_entity_mut;
use crate::game::commands::texts;
use crate::game::commands::without_knots;
use crate::game::commands::wrap_degrees;
use mod_api::CommandFeedback;
use sunrise_command::{
    Arg, CommandError, Context,
    args::{Coordinates, GameMode, ItemInput, Selector},
};
use sunrise_core::{geometry::Vec3, java_number::float_to_string};
use sunrise_game_data::{
    gamerules::Value,
    text::{self, Text},
};
use sunrise_gameplay_common::game::transactions::entities::EntityRemovalGameExt as _;
use sunrise_items::items::ItemStack;
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_world::registry::BlockStateId;

#[path = "compute_server.rs"]
pub(in crate::game) mod compute;

#[path = "data_server.rs"]
pub mod data;

#[path = "execute_server.rs"]
pub mod execute;

#[path = "extra_server.rs"]
pub(in crate::game) mod extra;

#[path = "give_server.rs"]
pub(in crate::game) mod give;

#[path = "items_server.rs"]
pub(in crate::game) mod items;

#[path = "misc_server.rs"]
pub(in crate::game) mod misc;

#[path = "place_server.rs"]
pub(in crate::game) mod place;

#[path = "players_server.rs"]
pub(in crate::game) mod players;

#[path = "position_args_server.rs"]
pub(in crate::game) mod position_args;

#[path = "pregen_server.rs"]
pub(in crate::game) mod pregen;

#[path = "profiles_server.rs"]
pub(in crate::game) mod profiles;

#[path = "selectors_server.rs"]
pub(in crate::game) mod selectors;

#[path = "targets_server.rs"]
pub(in crate::game) mod targets;

#[path = "terrain_cmds_server.rs"]
pub(in crate::game) mod terrain_cmds;

#[path = "test_locate_server.rs"]
pub(in crate::game) mod test_locate;

#[path = "test_runs_server.rs"]
pub(in crate::game) mod test_runs;

#[path = "text_commands_server.rs"]
pub(in crate::game) mod text_commands;

#[path = "world_cmds_server.rs"]
pub mod world_cmds;

impl crate::game::ServerGame<'_> {
    /// `CommandSource.acceptsSuccess`: a player's source takes success feedback while
    /// `send_command_feedback` is on; the server console's and an RCON client's always do.
    pub(in crate::game::server_side) fn accepts_success(&self) -> bool {
        self.native_console_source()
            || self.server.console_broadcast.is_some()
            || self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::SEND_COMMAND_FEEDBACK)
    }
    /// `CommandSourceStack.getDisplayName` for the admin line: the executing
    /// entity's name, read where `execute in` put the command (`source.dimension`,
    /// in place); a command-block minecart executing as itself, and a command
    /// block's own name, in the origin's level; the server's for the console.
    pub(in crate::game::server_side) fn admin_display_name(&mut self) -> Text {
        let Some((entity, own_cart)) = self.server.command_source.as_ref().map(|source| {
            (
                source.entity,
                source.cart.is_some() && source.entity == source.cart.map(Target::Cart),
            )
        }) else {
            return if self.server.console_broadcast.is_some() || self.native_console_source() {
                self.server_name()
            } else {
                self.source_display_name()
            };
        };
        match entity {
            Some(_) if !own_cart => self.source_display_name(),
            _ => self.in_origin_level(|g| g.source_display_name()),
        }
    }
    /// `CommandSourceStack.broadcastToAdmins` when the origin informs the
    /// operators: `[name: message]` (`chat.type.admin`, gray and italic, with
    /// the source's display name: the `execute as` entity's, else the block's,
    /// minecart's or server's). Under `send_command_feedback` every other
    /// operator gets it (`PlayerList.isOp`, queued while the command runs;
    /// `send_admin_feedback` sends it). Under `log_admin_commands` the server's
    /// log gets it, unless the server's console is the source. Neither rule
    /// gates the other.
    pub(in crate::game::server_side) fn broadcast_to_admins(&mut self, message: Text) {
        if !self.should_inform_admins() {
            return;
        }
        let rules = &self.common.ui.rules;
        let ops = rules.bool_at(sunrise_game_data::gamerules::vanilla::SEND_COMMAND_FEEDBACK);
        // `source != server`: RCON is a source of its own and is logged.
        let console_source = (self.server.console_broadcast.is_some()
            || self.native_console_source())
            && !self.server.rcon_source;
        let log = !console_source
            && rules.bool_at(sunrise_game_data::gamerules::vanilla::LOG_ADMIN_COMMANDS);
        if !ops && !log {
            return;
        }
        let who = self.admin_display_name();
        let line = Text::translate("chat.type.admin", &[who, message])
            .with_color(text::GRAY)
            .italic();
        if log {
            self.log_server_system_message(&line);
        }
        if ops {
            // The player whose command it is never hears it back; a block's,
            // minecart's or console's line has no such player.
            let origin_is_player = self.server.console_broadcast.is_none()
                && self
                    .server
                    .command_source
                    .as_ref()
                    .is_none_or(|s| s.origin_player);
            let source = if origin_is_player {
                self.server.lan_acting
            } else {
                None
            };
            self.server.admin_feedback.push((line, source));
        }
    }
    pub(in crate::game::server_side) fn clone_between(
        &mut self,
        ctx: &Context,
        a: [i32; 3],
        b: [i32; 3],
        dest: [i32; 3],
        from_dim: Dim,
        to_dim: Dim,
    ) -> R<i32> {
        let min = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
        let max = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
        let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
        let dmax = [dest[0] + size[0], dest[1] + size[1], dest[2] + size[2]];
        let overlap = (0..3).all(|i| dest[i] <= max[i] && dmax[i] >= min[i]);
        let moving = ctx.has("move");
        // Mode.canOverlap: only the normal mode refuses overlapping regions.
        if !ctx.has("force") && !moving && from_dim == to_dim && overlap {
            return Err(err("commands.clone.overlap", &[]));
        }
        let area = (0..3).map(|i| i64::from(size[i] + 1)).product::<i64>();
        let limit = self
            .common
            .ui
            .rules
            .int_at(sunrise_game_data::gamerules::vanilla::MAX_BLOCK_MODIFICATIONS);
        if area > i64::from(limit) {
            return Err(err("commands.clone.toobig", &[&limit, &area]));
        }
        // CloneCommands.clone: every column of both boxes must be loaded, in each level.
        self.clone_boxes_loaded([(from_dim, a, b), (to_dim, dest, dmax)])?;
        // CloneCommands.clone: the destination level being a Debug world fails it.
        if self.server.world_debug {
            return Err(err("commands.clone.failed", &[]));
        }
        let filter = match ctx.get("filter") {
            Some(Arg::Block(f)) => Some(f.clone()),
            _ => None,
        };
        let masked = ctx.has("masked");
        self.clone_enter(from_dim)?;
        let mut copies = Vec::new();
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                for x in min[0]..=max[0] {
                    let s = self.common.world.block(x, y, z);
                    if masked && self.common.res.facts().is_air(s) {
                        continue;
                    }
                    if let Some(f) = &filter
                        && !self.block_matches(f, [x, y, z])
                    {
                        continue;
                    }
                    copies.push((
                        [x, y, z],
                        [
                            dest[0] + x - min[0],
                            dest[1] + y - min[1],
                            dest[2] + z - min[2],
                        ],
                        s,
                        self.clone_block_payload((x, y, z)),
                    ));
                }
            }
        }
        if moving {
            for (from, ..) in &copies {
                self.place_state(*from, self.common.world.air, true, false)?;
            }
        }
        self.clone_enter(to_dim)?;
        let strict = ctx.has("strict");
        // CloneCommands.clone sets every destination to a barrier first, so
        // each copied state but a barrier counts as placed.
        let barrier = self
            .common
            .pack
            .assets
            .registry
            .default_state("minecraft:barrier");
        let mut count = 0;
        for (_, to, s, payload) in copies {
            self.discard_clone_loot(to);
            self.place_state(to, s, strict, false)?;
            // CloneCommands.clone counts a block only when Level.setBlock returned true.
            if self.clone_writable(to) && Some(s) != barrier {
                count += 1;
            }
            self.apply_clone_payload(to, payload);
        }
        // LevelTicks.copyAreaFrom: the block ticks pending anywhere in the
        // source box go to the same offset in the destination, before the
        // command can fail on an empty count.
        {
            use sunrise_core::position::BlockPos;
            self.common.block_ticks.blocks.copy_area(
                sunrise_core::ticks::TickArea::new(
                    BlockPos::new(min[0], min[1], min[2]),
                    BlockPos::new(max[0], max[1], max[2]),
                ),
                BlockPos::new(dest[0] - min[0], dest[1] - min[1], dest[2] - min[2]),
            );
        }
        if count == 0 {
            return Err(err("commands.clone.failed", &[]));
        }
        Ok(count)
    }
    pub(in crate::game) fn clone_levels(&mut self, ctx: &Context) -> R<i32> {
        let here = self.common.dim;
        let source_level = self
            .server
            .command_source
            .as_ref()
            .map_or(self.common.dim, |s| s.dimension);
        // The coordinates, from the command's own source, before any level moves.
        let position = |name: &str| -> R<[i32; 3]> {
            let c = self
                .coords(ctx, name)
                .ok_or_else(|| err("argument.pos.missing.int", &[]))?;
            Ok(self.resolve_coords(&c).map(|v| v.floor() as i32))
        };
        let (a, b, dest) = (
            position("begin")?,
            position("end")?,
            position("destination")?,
        );
        let from_dim = self.clone_dimension(ctx, "sourceDimension")?;
        self.clone_loaded(from_dim, here, source_level, a)?;
        self.clone_loaded(from_dim, here, source_level, b)?;
        let to_dim = self.clone_dimension(ctx, "targetDimension")?;
        self.clone_loaded(to_dim, here, source_level, dest)?;
        self.clone_between(
            ctx,
            a,
            b,
            dest,
            from_dim.unwrap_or(here),
            to_dim.unwrap_or(here),
        )
    }
    pub(in crate::game::server_side) fn cmd_fill(&mut self, ctx: &Context) -> R<i32> {
        let a = self.block_pos_arg(ctx, "from")?;
        let b = self.block_pos_arg(ctx, "to")?;
        let Some(Arg::Block(block)) = ctx.get("block").cloned() else {
            return Ok(0);
        };
        let filter = match ctx.get("filter") {
            Some(Arg::Block(f)) => Some(f.clone()),
            _ => None,
        };
        let min = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
        let max = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
        let area = (0..3)
            .map(|i| i64::from(max[i] - min[i] + 1))
            .product::<i64>();
        let limit = self
            .common
            .ui
            .rules
            .int_at(sunrise_game_data::gamerules::vanilla::MAX_BLOCK_MODIFICATIONS);
        if area > i64::from(limit) {
            return Err(err("commands.fill.toobig", &[&limit, &area]));
        }
        // FillCommand.fillBlocks: `ServerLevel.isDebug` fails it.
        if self.server.world_debug {
            return Err(err("commands.fill.failed", &[]));
        }
        let state = self.block_state(&block)?;
        let (hollow, outline, destroy, strict, keep) = (
            ctx.has("hollow"),
            ctx.has("outline"),
            ctx.has("destroy"),
            ctx.has("strict"),
            ctx.has("keep"),
        );
        let normalize_note = !strict
            && self
                .common
                .res
                .behaviours()
                .data(state)
                .is(sunrise_world::level::classes::Class::Note);
        let mut count = 0;
        // What was placed, and what it replaced, for the notices after the last block.
        let mut updated: Vec<([i32; 3], BlockStateId)> = Vec::new();
        // Whether the filter takes a state, worked out once per state: a fill visits up to 32768 blocks of a handful
        // of states, and matching a tag is a name lookup.
        let mut matched: std::collections::HashMap<BlockStateId, bool> =
            std::collections::HashMap::new();
        // A tag can change a block entity whose block is already in place.
        let plain = !(destroy || hollow || outline) && block.nbt.is_none();
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                for x in min[0]..=max[0] {
                    let current = self.common.world.block(x, y, z);
                    if keep && !self.common.res.facts().is_air(current) {
                        continue;
                    }
                    // Already the block it would become: nothing to place (place_state would do nothing either).
                    // `BlockInput.place` can change a note's instrument even
                    // when the raw input equals the current state.
                    if plain && current == state && !normalize_note {
                        continue;
                    }
                    if let Some(f) = &filter {
                        // (a filter with NBT reads each block's own block entity)
                        let ok = if f.nbt.is_some() {
                            self.block_matches(f, [x, y, z])
                        } else {
                            match matched.get(&current) {
                                Some(&ok) => ok,
                                None => {
                                    let ok = self.command_block_state_matches(f, current);
                                    matched.insert(current, ok);
                                    ok
                                }
                            }
                        };
                        if !ok {
                            continue;
                        }
                    }
                    let edge = x == min[0]
                        || x == max[0]
                        || y == min[1]
                        || y == max[1]
                        || z == min[2]
                        || z == max[2];
                    let mut affected = false;
                    if destroy && !self.common.res.facts().is_air(current) {
                        self.destroy_block_dropping([x, y, z])?;
                        affected = true;
                    }
                    let core = (outline || hollow) && !edge;
                    let target = if core {
                        if outline {
                            if affected {
                                count += 1;
                            }
                            continue;
                        }
                        self.common.world.air
                    } else {
                        state
                    };
                    let target = if normalize_note && target == state {
                        self.block_input_state([x, y, z], &block, target, strict)?
                    } else {
                        target
                    };
                    let previous = self.common.world.block(x, y, z);
                    let placed = self.place_state([x, y, z], target, strict, true)?;
                    // BlockInput.place loads the tag into the block entity now
                    // there; a tag that changed it counts the block as placed.
                    // A hollow fill's core is plain air.
                    let loaded = !core
                        && block
                            .nbt
                            .as_deref()
                            .is_some_and(|nbt| self.load_block_entity_nbt([x, y, z], nbt));
                    if placed || loaded {
                        count += 1;
                        if !strict {
                            updated.push(([x, y, z], previous));
                        }
                    } else if affected {
                        count += 1;
                    }
                }
            }
        }
        // FillCommand.fillBlocks: every block is in before any neighbour hears of one.
        for (pos, previous) in updated {
            self.update_neighbours_on_block_set(pos, previous)?;
        }
        if count == 0 {
            return Err(err("commands.fill.failed", &[]));
        }
        self.feedback(Text::tr("commands.fill.success", &[&count]));
        Ok(count)
    }
    pub fn cmd_gamerule(&mut self, ctx: &Context) -> R<i32> {
        let Some(name) = ctx.literals.get(1).cloned() else {
            return Ok(0);
        };
        let Some(rule) = self.common.ui.rules.rule(&name).cloned() else {
            return Ok(0);
        };
        // GameRule.id(): only the default namespace is omitted in messages.
        let rule_id = rule.name.strip_prefix("minecraft:").unwrap_or(&rule.name);
        let current = self.common.ui.rules.get(&rule.name).unwrap_or(rule.default);
        let value = match ctx.get("value") {
            Some(Arg::Bool(b)) => Value::Bool(*b),
            Some(Arg::Int(i)) => Value::Int(*i),
            _ => {
                self.feedback_unbroadcast(Text::tr(
                    "commands.gamerule.query",
                    &[&rule_id, &current],
                ));
                return Ok(match current {
                    Value::Bool(b) => i32::from(b),
                    Value::Int(i) => i,
                });
            }
        };
        if current == value {
            return Err(err("commands.gamerule.not_set", &[&rule_id, &value]));
        }
        let actor = self.common.native_lifecycle.epoch().and_then(|epoch| {
            Some(mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(epoch, self.server.lan_acting_entity?),
                self.server.lan_acting_uuid?,
            ))
        });
        self.set_game_rule(&rule, value, actor);
        self.feedback(Text::tr("commands.gamerule.set", &[&rule_id, &value]));
        Ok(match value {
            Value::Bool(b) => i32::from(b),
            Value::Int(i) => i,
        })
    }
    pub(in crate::game::server_side) fn cmd_give(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Item(ItemInput { id, components, .. })) = ctx.get("item").cloned() else {
            return Ok(0);
        };
        let (damage, extra) = match components
            .as_deref()
            .map(sunrise_items::item_data::command_components)
        {
            Some(Ok(tag)) => sunrise_items::item_data::decode_components(Some(&tag)),
            Some(Err(e)) => return Err(CommandError::new(Text::literal(e))),
            None => (0, None),
        };
        let prototype = ItemStack {
            item: id.as_str().into(),
            count: 1,
            damage,
            extra,
        };
        let count = Self::int(ctx, "count").unwrap_or(1);
        let targets = self.entities(ctx, "targets", true)?;
        let max_stack = self.items().get(&id).map_or(64, |i| i.max_stack) as i32;
        // A native mod's item can stack far past vanilla's 99: saturate rather
        // than overflow on a size near i32::MAX.
        let limit = max_stack.saturating_mul(100);
        if count > limit {
            return Err(CommandError::new(Text::translate(
                "commands.give.failed.toomanyitems",
                &[
                    Text::literal(limit.to_string()),
                    self.stack_display_name(&prototype),
                ],
            )));
        }
        // An air stack is empty: nothing is given (but the message still is).
        let empty = id == "minecraft:air";
        for &t in targets.iter().filter(|_| !empty) {
            // The player's UUID, which an item they could not hold is set to.
            let conn = match t {
                Target::Guest(conn) => Some(conn),
                _ => self.server.lan_acting,
            };
            let uuid = match conn {
                Some(conn) => self
                    .command_guest(conn)
                    .map(|g| sunrise_sim::lan::playerdata::uuid_ints(g.uuid)),
                None => Some(self.common.level.ensure_player_uuid()),
            };
            self.for_player(t, |g| {
                // An acting player in another level than the command's (`execute in`) gets
                // what does not fit dropped, and the trigger's rewards, in their own.
                let theirs = conn
                    .and_then(|conn| g.command_guest(conn))
                    .map(|guest| Dim::from_id(&guest.dimension));
                let back = theirs.and_then(|dim| g.enter_command_level(dim));
                let mut remaining = count;
                while remaining > 0 {
                    let size = remaining.min(max_stack);
                    remaining -= size;
                    let rest = g.give_acting_stack(
                        ItemStack {
                            count: size as u32,
                            ..prototype.clone()
                        },
                        uuid,
                    );
                    debug_assert!(rest.is_none(), "the player's own level is in place");
                }
                // containerMenu.broadcastChanges: InventoryChangeTrigger
                // (and its announcement) comes before the command's message.
                if g.player.is_some() {
                    g.check_inventory_trigger();
                }
                if let Some(back) = back {
                    g.enter_command_level(back);
                }
            });
        }
        let name = self.stack_display_name(&prototype);
        let message = if targets.len() == 1 {
            Text::translate(
                "commands.give.success.single",
                &[
                    Text::literal(count.to_string()),
                    name,
                    self.target_name(targets[0]),
                ],
            )
        } else {
            Text::translate(
                "commands.give.success.multiple",
                &texts(&[
                    Text::literal(count.to_string()),
                    name,
                    Text::literal(targets.len().to_string()),
                ]),
            )
        };
        self.feedback(message);
        Ok(targets.len() as i32)
    }
    /// `SayCommand.register`: resolve the source name and broadcast unsigned announcements.
    pub(in crate::game::server_side) fn cmd_say(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let raw = match ctx.get(if id == "me" { "action" } else { "message" }) {
            Some(Arg::Str(s)) => s.clone(),
            _ => String::new(),
        };
        match id {
            "say" | "me" => {
                let message = self.resolve_chat_message(&raw);
                let name = self.source_display_name();
                let sender = match self.source_entity() {
                    Some(Target::Player) => self
                        .server
                        .lan_acting_uuid
                        .unwrap_or_else(|| self.own_uuid()),
                    Some(Target::Guest(conn)) => {
                        self.command_guest(conn).map_or(0, |guest| guest.uuid)
                    }
                    _ => 0,
                };
                if !self.lan_broadcast_command_chat(sender, &name, id, &message, None) {
                    return Err(err("command.failed", &[]));
                }
                Ok(1)
            }
            _ => {
                let message = self.expand_selectors(&raw);
                let name = self.source_display_name();
                let targets = self.entities(ctx, "targets", true)?;
                let count = targets.len() as i32;
                // ChatType.bind throws for a missing type before anything is sent.
                let incoming_type = match &self.server.lan {
                    Some(host) => {
                        let (Some(incoming), Some(_)) = (
                            host.hub.chat_type_id("msg_command_incoming"),
                            host.hub.chat_type_id("msg_command_outgoing"),
                        ) else {
                            return Err(err("command.failed", &[]));
                        };
                        Some(incoming)
                    }
                    None => None,
                };
                for target in targets {
                    let target_name = match target {
                        Target::Player => self.player_display_name(),
                        other => self.target_name(other),
                    };
                    let out = Text::translate(
                        "commands.message.display.outgoing",
                        &[target_name, message.clone()],
                    )
                    .with_color(text::GRAY)
                    .italic();
                    // MsgCommand.sendMessage -> CommandSourceStack.sendChatMessage:
                    // a non-player source receives the decorated outgoing line via
                    // sendSystemMessage; a player source receives it as its own message.
                    self.command_system_message(out);
                    let incoming = Text::translate(
                        "commands.message.display.incoming",
                        &[name.clone(), message.clone()],
                    )
                    .with_color(text::GRAY)
                    .italic();
                    let conn = match target {
                        Target::Guest(conn) => Some(conn),
                        Target::Player => self.server.lan_acting,
                        _ => None,
                    };
                    if let Some(conn) = conn {
                        if self.server.lan_acting == Some(conn)
                            && let Some(capture) = self.server.chat_capture.as_mut()
                        {
                            capture.push(incoming);
                        } else if let Some(host) = &self.server.lan
                            && let Some(t) = host.hub.guests.connection_index(conn)
                        {
                            // The target's copy is private chat with the incoming whisper
                            // type, shown only to players who show chat (`Hub::whisper`).
                            let bound = sunrise_protocol::packets::play::ChatTypeBound {
                                chat_type: incoming_type
                                    .expect("a hub's incoming type was checked above")
                                    .network_id() as i32,
                                name: sunrise_sim::lan::data::component_nbt(&name.component()),
                                target: None,
                            };
                            host.hub.send_disguised(
                                t,
                                sunrise_sim::lan::data::component_nbt(&message.component()),
                                bound,
                            );
                        }
                    } else if let Some(capture) = self.server.chat_capture.as_mut() {
                        capture.push(incoming);
                    }
                }
                Ok(count)
            }
        }
    }
    /// `CommandSourceStack.sendSystemMessage`: no success-feedback rule. Unlike `sendSuccess`
    /// and `sendFailure`, it addresses the executing player when there is one.
    pub(in crate::game) fn command_system_message(&mut self, message: Text) {
        if self
            .server
            .command_source
            .as_ref()
            .is_some_and(|s| s.silent)
        {
            return;
        }
        if self.native_console_source() {
            let conn = match self.source_entity() {
                Some(Target::Player) => self.server.lan_acting,
                Some(Target::Guest(conn)) => Some(conn),
                _ => {
                    self.send_command_output(CommandFeedback::System, message);
                    return;
                }
            };
            if let Some(conn) = conn {
                if let Some(host) = &self.server.lan {
                    host.hub.send_to(
                        conn,
                        &sunrise_gameplay_common::game::native_server::system_chat(&message),
                    );
                }
            } else {
                self.server_system_message(message);
            }
            return;
        }
        self.send_command_output(CommandFeedback::System, message);
    }
    /// `MessageArgument.resolveChatMessage`: only sources with entity selector permission
    /// resolve selectors. Other players' text remains literal.
    pub(in crate::game) fn resolve_chat_message(&mut self, raw: &str) -> Text {
        if self.common.ui.registries.permission >= 2 {
            self.expand_selectors(raw)
        } else {
            Text::literal(raw)
        }
    }
    pub(in crate::game::server_side) fn cmd_setblock(&mut self, ctx: &Context) -> R<i32> {
        let pos = self.block_pos_arg(ctx, "pos")?;
        let Some(Arg::Block(b)) = ctx.get("block").cloned() else {
            return Ok(0);
        };
        let state = self.block_state(&b)?;
        // SetBlockCommand.setBlock: `ServerLevel.isDebug` fails it.
        if self.server.world_debug {
            return Err(err("commands.setblock.failed", &[]));
        }
        let current = self.common.world.block(pos[0], pos[1], pos[2]);
        if ctx.has("keep") && !self.common.res.facts().is_air(current) {
            return Err(err("commands.setblock.failed", &[]));
        }
        // SetBlockCommand.setBlock skips placement when destruction already
        // leaves an air state and the requested state is also air.
        let place_needed = if ctx.has("destroy") {
            self.destroy_block_dropping(pos)?;
            let remaining = self.common.world.block(pos[0], pos[1], pos[2]);
            !(self.common.res.facts().is_air(state) && self.common.res.facts().is_air(remaining))
        } else {
            true
        };
        if place_needed {
            let state = self.block_input_state(pos, &b, state, ctx.has("strict"))?;
            // SetBlockCommand.setBlock reads the old state after a destroy.
            let old_state = self.common.world.block(pos[0], pos[1], pos[2]);
            let placed = self.place_state(pos, state, ctx.has("strict"), true)?;
            let loaded = match &b.nbt {
                Some(nbt) => self.load_block_entity_nbt(pos, nbt),
                None => false,
            };
            if !placed && !loaded {
                return Err(err("commands.setblock.failed", &[]));
            }
            if !ctx.has("strict") {
                self.update_neighbours_on_block_set(pos, old_state)?;
            }
        } else if !ctx.has("strict") {
            // ServerLevel.updateNeighboursOnBlockSet still runs when placing
            // air was unnecessary. The unchanged air has no analog output or
            // removal side effects, so only its neighbours need notification.
            let remaining = self.common.world.block(pos[0], pos[1], pos[2]);
            self.with_level(|b, level, ticks| {
                b.update_neighbors_at(
                    level,
                    ticks,
                    sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
                    b.block(remaining),
                );
            });
            self.server_process_level_events()
                .map_err(|e| CommandError::new(Text::literal(e.to_string())))?;
        }
        self.feedback(Text::tr(
            "commands.setblock.success",
            &[&pos[0], &pos[1], &pos[2]],
        ));
        Ok(1)
    }
    pub(in crate::game::server_side) fn cmd_setworldspawn(&mut self, ctx: &Context) -> R<i32> {
        // SetWorldSpawnCommand: BlockPosArgument.getSpawnablePos, in the source's level.
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
        let dimension = self
            .server
            .command_source
            .as_ref()
            .map_or(self.common.dim, |s| s.dimension);
        let [yaw, pitch] = self.set_world_spawn(dimension, pos, rot);
        let args = [
            Text::literal(pos[0].to_string()),
            Text::literal(pos[1].to_string()),
            Text::literal(pos[2].to_string()),
            Text::literal(float_to_string(yaw)),
            Text::literal(float_to_string(pitch)),
            Text::literal(dimension.id()),
        ];
        self.feedback(Text::translate("commands.setworldspawn.success", &args));
        Ok(1)
    }
    /// `SummonCommand.createEntity`: Peaceful admission uses the entity-type
    /// policy for both forms, independently of the mob category.
    pub fn cmd_summon(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(kind)) = ctx.get("entity").cloned() else {
            return Ok(0);
        };
        // ResourceArgument.getSummonableEntityType.
        if matches!(
            kind.as_str(),
            "minecraft:player" | "minecraft:fishing_bobber"
        ) {
            return Err(err("entity.not_summonable", &[&kind]));
        }
        let pos = self
            .position_arg(ctx, "pos")
            .unwrap_or(self.source_position());
        Self::check_summon_bounds(pos)?;
        let mob_type = self.common.res.mob_types().get(&kind);
        if self.difficulty_id() == 0 && mob_type.is_some_and(|t| !t.allowed_in_peaceful()) {
            return Err(err("commands.summon.failed.peaceful", &[]));
        }
        let short = kind.trim_start_matches("minecraft:").to_owned();
        if let Some(Arg::Str(nbt)) = ctx.get("nbt").cloned() {
            // With an NBT argument the entity is built from it, unfinalized.
            let before = self.spawned_mark();
            let name = match self.summon_with_nbt(&kind, pos, &nbt)? {
                Some(target) => self.target_name(target),
                // Spawned as a kind no command target stands for (primed
                // TNT, falling blocks, projectiles): its type's name.
                None if self.spawned_mark() != before => data::summon_type_name(&kind),
                None => return Err(err("commands.summon.failed", &[])),
            };
            self.feedback(Text::translate("commands.summon.success", &[name]));
            return Ok(1);
        }
        let name;
        if kind == "minecraft:ender_dragon" || mob_type.is_some() {
            let id = self
                .spawn_mob_spawned(
                    &short,
                    pos,
                    sunrise_gameplay_common::game::creatures::SpawnSite::Finalizing(
                        sunrise_sim::mobs::spawn_rules::Reason::Command,
                    ),
                )
                .ok_or_else(|| err("commands.summon.failed.uuid", &[]))?;
            name = self.target_name(Target::Mob(id));
        } else {
            match short.as_str() {
                "experience_orb" => {
                    // `EntityType`'s factory, `new ExperienceOrb(EntityType, Level)`,
                    // which only `snapTo`s: the position is kept, even in a block.
                    self.server.next_entity += 1;
                    let id = self.server.next_entity;
                    let uuid = sunrise_sim::save::insecure_uuid(
                        &mut sunrise_level::random_source::entity(),
                    );
                    self.common.persist.uuids.insert(id, uuid);
                    let at = Vec3::new(pos[0], pos[1], pos[2]);
                    let orb = sunrise_sim::entities::ExperienceOrb::new(id, at, Vec3::ZERO, 0);
                    self.common.orbs.push(orb);
                    // tryAddFreshEntityWithPassengers refused: ERROR_DUPLICATE_UUID.
                    if !self.native_joins_flush(false, id) {
                        return Err(err("commands.summon.failed.uuid", &[]));
                    }
                    name = self.target_name(Target::Orb(id));
                }
                "lightning_bolt" => {
                    let target = self
                        .summon_with_nbt(&kind, pos, "{}")?
                        .ok_or_else(|| err("commands.summon.failed", &[]))?;
                    name = self.target_name(target);
                }
                "end_crystal" => {
                    // EndCrystal's constructor takes an id from the entity
                    // counter, and the crystal is shown with its base.
                    self.server.next_entity += 1;
                    let id = self.server.next_entity;
                    let at = Vec3::new(pos[0], pos[1], pos[2]);
                    let (drawn, uuid) =
                        sunrise_gameplay_common::game::dragon_common::Crystal::own_draws();
                    self.common.persist.uuids.insert(id, uuid);
                    let time = sunrise_gameplay_common::game::dragon_common::Crystal::start_time(
                        at,
                        drawn,
                        self.common.clock.still(),
                    );
                    let crystal = sunrise_gameplay_common::game::dragon_common::Crystal::new(
                        id, at, true, time,
                    );
                    self.common.crystals.push(crystal);
                    // tryAddFreshEntityWithPassengers refused: ERROR_DUPLICATE_UUID.
                    if !self.native_joins_flush(false, id) {
                        return Err(err("commands.summon.failed.uuid", &[]));
                    }
                    name = self.target_name(Target::Crystal(id));
                }
                _ if super::minecarts::CartKind::from_id(&kind).is_some() => {
                    let cart_kind = super::minecarts::CartKind::from_id(&kind)
                        .unwrap_or(super::minecarts::CartKind::Rideable);
                    self.server.next_entity += 1;
                    let mut cart = super::minecarts::Minecart::new(
                        self.server.next_entity,
                        cart_kind,
                        Vec3::new(pos[0], pos[1], pos[2]),
                    );
                    // SummonCommand.createEntity loads even an empty compound, so a
                    // spawner minecart's BaseSpawner.load runs: SpawnPotentials becomes
                    // WeightedList.of(new SpawnData()).
                    if cart.spawner.is_some() {
                        use sunrise_sim::mobs::spawner::{Spawner, SpawnerExt};
                        let empty = sunrise_core::nbt::Tag::Compound(Default::default());
                        cart.spawner = Some(Box::new(Spawner::load(&empty)));
                    }
                    let id = cart.id;
                    let uuid = sunrise_sim::save::insecure_uuid(
                        &mut sunrise_level::random_source::entity(),
                    );
                    self.common.persist.uuids.insert(id, uuid);
                    self.common.carts.push(cart);
                    if !self.native_joins_flush(false, id) {
                        return Err(err("commands.summon.failed.uuid", &[]));
                    }
                    name = self.target_name(Target::Cart(id));
                }
                // SummonCommand.createEntity: any other type from an empty compound.
                _ => {
                    let before = self.spawned_mark();
                    name = match self.summon_with_nbt(&kind, pos, "{}")? {
                        Some(target) => self.target_name(target),
                        // Spawned as a kind no command target stands for
                        // (primed TNT, falling blocks, projectiles).
                        None if self.spawned_mark() != before => data::summon_type_name(&kind),
                        None => return Err(err("commands.summon.failed", &[])),
                    };
                }
            }
        }
        self.feedback(Text::translate("commands.summon.success", &[name]));
        Ok(1)
    }
    pub(in crate::game::server_side) fn cmd_teleport(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = if id.starts_with("teleport.targets") {
            without_knots(self.entities(ctx, "targets", false)?)?
        } else {
            vec![
                self.source_entity()
                    .ok_or_else(|| err("permissions.requires.entity", &[]))?,
            ]
        };
        if id.ends_with("destination") {
            let dest = self.entities(ctx, "destination", false)?[0];
            let (pos, level, rotation, destination_name) = self
                .in_target_level(dest, |game| {
                    (
                        game.target_position(dest).unwrap_or(game.source_position()),
                        game.target_level(dest),
                        Some(game.target_rotation(dest).map(wrap_degrees)),
                        game.target_name(dest),
                    )
                })
                .ok_or_else(|| err("argument.entity.notfound.entity", &[]))?;
            let target_name = (targets.len() == 1).then(|| {
                self.in_target_level(targets[0], |game| game.target_name(targets[0]))
                    .unwrap_or_else(Text::empty)
            });
            for &t in &targets {
                let Some(pos) = self.teleport_command_target(t, level, pos) else {
                    continue;
                };
                if !self.teleport_across(t, level, pos, rotation, true, TeleportMotion::Stop)? {
                    // TeleportCommand.teleportToEntity: the destination's angles,
                    // which the riders turn to as well.
                    self.teleport_to(t, pos, rotation, true);
                    self.tp_lands(t);
                }
            }
            let message = if targets.len() == 1 {
                Text::translate(
                    "commands.teleport.success.entity.single",
                    &[target_name.unwrap(), destination_name],
                )
            } else {
                Text::translate(
                    "commands.teleport.success.entity.multiple",
                    &[Text::literal(targets.len().to_string()), destination_name],
                )
            };
            self.feedback(message);
            return Ok(targets.len() as i32);
        }
        let pos = self
            .position_arg(ctx, "location")
            .unwrap_or(self.source_position());
        if !(-30_000_000.0..30_000_000.0).contains(&pos[0])
            || !(-30_000_000.0..30_000_000.0).contains(&pos[2])
            || !(-20_000_000.0..20_000_000.0).contains(&pos[1])
        {
            return Err(err("commands.teleport.invalidPosition", &[]));
        }
        let relative = match ctx.get("location") {
            Some(Arg::Coords(Coordinates::World(coords))) => coords.each_ref().map(|c| c.relative),
            Some(Arg::Coords(Coordinates::Local(_))) => [true; 3],
            _ => [false; 3],
        };
        let rotation = match ctx.get("rotation") {
            Some(Arg::Coords(c @ Coordinates::World(axes))) => Some((
                c.rotation(self.source_rotation()),
                [axes[0].relative, axes[1].relative],
            )),
            _ => None,
        };
        // facing: look from the new position at a location or entity.
        let facing = self.position_arg(ctx, "facingLocation").or_else(|| {
            let Some(Arg::Entity(_)) = ctx.get("facingEntity") else {
                return None;
            };
            None
        });
        let mut facing_entity = None;
        let facing = match (facing, ctx.get("facingEntity")) {
            (Some(f), _) => Some(f),
            (None, Some(_)) => {
                let e = self.entities(ctx, "facingEntity", false)?[0];
                // EntityAnchorArgument.Anchor.apply: the feet, or the eyes.
                let eyes = matches!(ctx.get("facingAnchor"), Some(Arg::Anchor(true)));
                facing_entity = Some((e, eyes));
                let eye_height = if eyes {
                    self.server_target_eye_height(e)
                } else {
                    0.0
                };
                self.target_position(e)
                    .map(|p| [p[0], p[1] + eye_height, p[2]])
            }
            _ => None,
        };
        // Entity.lookAt: from the source's anchor (the feet, unless `anchored
        // eyes`) on the teleported entity, so the eyes depend on the target.
        let anchored_eyes = self.server.command_source.as_ref().is_some_and(|s| s.eyes);
        let look_at = |this: &Self, t: Target, at: [f64; 3]| {
            let f = facing?;
            let eye = if anchored_eyes {
                at[1] + this.server_target_eye_height(t)
            } else {
                at[1]
            };
            Some(world_cmds::look_at([at[0], eye, at[2]], f))
        };
        // A server's player in another level than the source's (`execute
        // in`, the console's Overworld) moves there (ServerPlayer.teleport
        // into another level), after the command.
        let source_level = match self.server.command_source.as_ref().and_then(|s| s.level) {
            Some(level) => level,
            None if self.source_entity().is_none() => super::Dim::Overworld,
            None => self.common.dim,
        };
        let target_name = (targets.len() == 1).then(|| {
            self.in_target_level(targets[0], |game| game.target_name(targets[0]))
                .unwrap_or_else(Text::empty)
        });
        for &t in &targets {
            let Some(pos) = self.teleport_command_target(t, source_level, pos) else {
                continue;
            };
            let looked = look_at(self, t, pos);
            let turn_riders = looked.is_none();
            let rotation = looked.or_else(|| {
                let (angles, relative) = rotation?;
                let current = self
                    .in_target_level(t, |game| game.target_rotation(t))
                    .unwrap_or([0.0, 0.0]);
                // performTeleport wraps relative deltas before calculateAbsolute
                // adds them back; wrapping the resolved pitch changes its clamp.
                Some(std::array::from_fn(|axis| {
                    if relative[axis] {
                        current[axis] + wrap_degrees(angles[axis] - current[axis])
                    } else {
                        wrap_degrees(angles[axis])
                    }
                }))
            });
            if self.teleport_across(
                t,
                source_level,
                pos,
                rotation,
                turn_riders,
                TeleportMotion::Stop,
            )? {
                continue;
            }
            self.teleport_to_relative(t, pos, rotation, relative, turn_riders);
            // LookAt.perform after a successful teleportTo.
            if let Some(f) = facing {
                self.send_player_look_at(t, anchored_eyes, f, facing_entity);
            }
            self.tp_lands(t);
        }
        let [x, y, z] = pos.map(|v| Text::literal(format_double(v)));
        let message = if targets.len() == 1 {
            Text::translate(
                "commands.teleport.success.location.single",
                &[target_name.unwrap(), x, y, z],
            )
        } else {
            Text::translate(
                "commands.teleport.success.location.multiple",
                &[Text::literal(targets.len().to_string()), x, y, z],
            )
        };
        self.feedback(message);
        Ok(targets.len() as i32)
    }
    /// CommandSourceStack.sendFailure: red text.
    pub(in crate::game) fn command_failure(&mut self, message: Text) {
        if self
            .server
            .command_source
            .as_ref()
            .is_some_and(|s| s.silent)
        {
            return;
        }
        if let Some(block) = self.server.command_source.as_ref().and_then(|s| s.block) {
            self.command_block_output(block, |timestamp| {
                formatted_command_failure(message, Some(timestamp))
            });
            return;
        }
        if let Some(cart) = self.server.command_source.as_ref().and_then(|s| s.cart) {
            self.cart_command_output(cart, |timestamp| {
                formatted_command_failure(message, Some(timestamp))
            });
            return;
        }
        self.send_command_output(
            CommandFeedback::Failure,
            formatted_command_failure(message, None),
        );
    }
    pub(in crate::game::server_side) fn execute_command(
        &mut self,
        id: &str,
        ctx: &Context,
    ) -> R<i32> {
        match id {
            "native_mod_command" => self.run_native_command(ctx.clone()),
            "help" => self.cmd_help(ctx),
            "gamemode" => self.cmd_gamemode(ctx),
            "defaultgamemode" => {
                let Some(Arg::GameMode(m)) = ctx.get("gamemode") else {
                    return Ok(0);
                };
                self.set_default_game_mode(match m {
                    sunrise_command::args::GameMode::Survival => 0,
                    sunrise_command::args::GameMode::Creative => 1,
                    sunrise_command::args::GameMode::Adventure => 2,
                    sunrise_command::args::GameMode::Spectator => 3,
                });
                self.feedback(Text::translate(
                    "commands.defaultgamemode.success",
                    &[Text::translate(&format!("gameMode.{}", m.name()), &[])],
                ));
                Ok(0)
            }
            "give" => self.cmd_give(ctx),
            "teleport.location"
            | "teleport.destination"
            | "teleport.targets.location"
            | "teleport.targets.destination" => self.cmd_teleport(id, ctx),
            id if id.starts_with("time.") => self.cmd_time(id, ctx),
            id if id.starts_with("raid.") => self.cmd_raid(id, ctx),
            "weather" => self.cmd_weather(ctx),
            "kill" => self.cmd_kill(ctx),
            "summon" => self.cmd_summon(ctx),
            "effect.give" => self.cmd_effect_give(ctx),
            "effect.clear" => self.cmd_effect_clear(ctx),
            "enchant" => self.cmd_enchant(ctx),
            "tellraw" => self.cmd_tellraw(ctx),
            "title" => self.cmd_title(ctx),
            "setblock" => self.cmd_setblock(ctx),
            "fill" => self.cmd_fill(ctx),
            "clone" => self.cmd_clone(ctx),
            "experience" => self.cmd_experience(ctx),
            "advancement" => self.cmd_advancement(ctx),
            "gamerule" => self.cmd_gamerule(ctx),
            "difficulty" => self.cmd_difficulty(ctx),
            "seed" => {
                let seed = self.common.level.seed.to_string();
                let value = Text::copyable(seed);
                self.feedback_unbroadcast(Text::translate("commands.seed.success", &[value]));
                Ok(self.common.level.seed as i32)
            }
            "spawnpoint" => self.cmd_spawnpoint(ctx),
            "setworldspawn" => self.cmd_setworldspawn(ctx),
            "clear" => self.cmd_clear(ctx),
            "item" => self.cmd_item(ctx),
            "item.modify" => self.cmd_item_modify(ctx),
            "spectate" => self.cmd_spectate(ctx),
            id if id.starts_with("waypoint.") => self.cmd_waypoint(id, ctx),
            id if id.starts_with("dialog.") => self.cmd_dialog(id, ctx),
            id if id.starts_with("posteffect.") => self.cmd_posteffect(id, ctx),
            "kick" => self.cmd_kick(ctx),
            "op" | "deop" | "ban" | "pardon" => self.cmd_profiles(id, ctx),
            "ban-ip" | "pardon-ip" => self.cmd_ip_bans(id, ctx),
            id if id.starts_with("banlist") => self.cmd_banlist(id),
            id if id.starts_with("whitelist.") => self.cmd_whitelist(id, ctx),
            "setidletimeout" => self.cmd_setidle_timeout(ctx),
            "stop" => self.cmd_stop(),
            "save-all" => self.cmd_save_all(),
            "save-off" | "save-on" => self.cmd_auto_save(id == "save-on"),
            "transfer" => self.cmd_transfer(ctx),
            "fetchprofile" => self.cmd_fetchprofile(ctx),
            "publish" => self.cmd_publish(ctx),
            "unpublish" => self.cmd_unpublish(),
            id if id.starts_with("modprofile.") => self.cmd_modprofile(id),
            "debug" => self.cmd_debug(ctx),
            "jfr" => self.cmd_jfr(ctx),
            "version" => self.cmd_version(),
            id if id.starts_with("test.") => self.cmd_test(id, ctx),
            "compute" => self.cmd_compute(ctx),
            "place" => self.cmd_place(ctx),
            "place.feature" | "place.jigsaw" | "place.structure" => {
                self.cmd_place_generated(id, ctx)
            }
            "recipe.give" | "recipe.take" => self.cmd_recipe(id, ctx),
            "damage" => self.cmd_damage(ctx),
            "rotate" => self.cmd_rotate(ctx),
            "swing" => self.cmd_swing(ctx),
            "playsound" => self.cmd_playsound(ctx),
            "stopsound" => self.cmd_stopsound(ctx),
            "particle" => self.cmd_particle(ctx),
            id if id.starts_with("attribute.") => {
                let result = self.cmd_attribute(id, ctx);
                // AttributeInstance.setDirty: the entity reacts when it ticks.
                if result.is_ok()
                    && matches!(ctx.get("attribute"), Some(Arg::Resource(a)) if a == "minecraft:waypoint_transmit_range")
                    && !id.ends_with("get")
                    && let Ok(t) = self.entities(ctx, "target", false)
                {
                    // Players already have a live attribute dirty callback.
                    if !matches!(t[0], Target::Player | Target::Guest(_)) {
                        self.server.cmds.0.waypoint_dirty.push(t[0]);
                    }
                }
                result
            }
            "ride.mount" | "ride.dismount" => self.cmd_ride(id, ctx),
            "spreadplayers" => self.cmd_spreadplayers(ctx),
            id if id.starts_with("forceload.") => self.cmd_forceload(id, ctx),
            id if id.starts_with("pregen.") => self.cmd_pregen(id, ctx),
            "fillbiome" => self.cmd_fillbiome(ctx),
            id if id.starts_with("worldborder.") => self.cmd_worldborder(id, ctx),
            id if id.starts_with("loot.") => self.cmd_loot(id, ctx),
            "teammsg" => self.cmd_teammsg(ctx),
            id if id.starts_with("stopwatch.") => self.cmd_stopwatch(id, ctx),
            id if id.starts_with("tick.") => self.cmd_tick(id, ctx),
            id if id.starts_with("random.") => self.cmd_random(id, ctx),
            "function" => self.cmd_function(ctx),
            "return.value" | "return.fail" => self.cmd_return(id, ctx),
            "schedule.function" | "schedule.clear" => self.cmd_schedule(id, ctx),
            "reload" => self.cmd_reload(),
            id if id.starts_with("datapack.") => self.cmd_datapack(id, ctx),
            id if id.starts_with("bossbar.") => self.cmd_bossbar(id, ctx),
            id if id.starts_with("data.") => self.cmd_data(id, ctx),
            id if id.starts_with("tag.") => self.cmd_tag(id, ctx),
            id if id.starts_with("scoreboard.")
                || id.starts_with("team.")
                || id.starts_with("trigger") =>
            {
                self.cmd_scoreboard(id, ctx)
            }
            "locate.structure" | "locate.biome" => self.cmd_locate(id, ctx),
            "locate.poi" => self.cmd_locate_poi(ctx),
            "warden_spawn_tracker.clear" | "warden_spawn_tracker.set" => {
                let level = Self::int(ctx, "warning_level");
                self.cmd_warden_spawn_tracker(level);
                Ok(1)
            }
            "say" | "me" | "msg" => self.cmd_say(id, ctx),
            "list" => self.cmd_list(ctx),
            _ => Err(err("command.failed", &[])),
        }
    }
    /// `f` with the command's origin level in place (`source.origin_dimension`):
    /// where its command block or minecart is, whatever level `execute in` named.
    pub(in crate::game::server_side) fn in_origin_level<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let origin = self
            .server
            .command_source
            .as_ref()
            .map(|s| s.origin_dimension);
        let back = origin.and_then(|dim| self.enter_command_level(dim));
        let out = f(self);
        if let Some(back) = back {
            self.enter_command_level(back);
        }
        out
    }
    pub(in crate::game) fn can_kill_target_in_level(&self, t: Target) -> bool {
        if self.target_removed(&t) || self.common.persist.preparing_vehicle_removals.contains(&t) {
            return false;
        }
        match t {
            Target::DragonPart(_) => false,
            Target::Player => self
                .common
                .player
                .as_ref()
                .is_some_and(|p| p.health.health() > 0.0),
            Target::Guest(conn) => self.command_guest(conn).is_some_and(|g| !g.dead),
            Target::Mob(id) => self
                .common
                .mobs
                .iter()
                .any(|m| m.id == id && !m.removed && m.alive()),
            _ => self.target_position(t).is_some(),
        }
    }
    pub fn kill_target_in_level(&mut self, t: Target) -> bool {
        if !self.can_kill_target_in_level(t) {
            return false;
        }
        // Entity.kill (BlockAttachedEntity.kill; ArmorStand's generic_kill hit,
        // which bypasses its invulnerability; a vehicle is no LivingEntity):
        // removed, then ENTITY_DIE by the entity itself. Mobs post theirs as
        // they die, the dragon on its own path, and EndCrystal.kill posts it
        // after onDestroyedBy. The source is captured before the removal: an
        // armor stand is living (wardens can't target it), and a dropped item
        // keeps its stack's ItemEntity.dampensVibrations.
        let died = (!matches!(
            t,
            Target::Player
                | Target::Guest(_)
                | Target::Mob(_)
                | Target::Crystal(_)
                | Target::Dragon(_)
        ))
        .then(|| self.target_position(t))
        .flatten();
        let mut context = self
            .network_entity_id(t)
            .map(|id| self.entity_event_context(id as u32, None))
            .unwrap_or_default();
        context.living = matches!(t, Target::Stand(_));
        if let Target::Item(id) = t {
            context.dampens = self
                .common
                .dropped
                .iter()
                .find(|i| i.id == id)
                .is_some_and(|i| {
                    self.common
                        .res
                        .items()
                        .item_in_tag("minecraft:dampens_vibrations", &i.stack.item)
                });
        }
        // `Entity.kill` (and `EnderDragon.kill`, `ArmorStand.kill`,
        // `BlockAttachedEntity.kill`) removes the entity immediately with
        // `RemovalReason.KILLED`, so `ServerLevel.EntityCallbacks.onDestroyed`
        // runs `Scoreboard.entityRemoved` before `/kill` formats its reply.
        // Players are never removed from teams, and living mobs stay in the
        // level through `tickDeath`.
        let destroyed_score = (!matches!(
            t,
            Target::Player
                | Target::Guest(_)
                | Target::Mob(_)
                | Target::Knot(_)
                | Target::DragonPart(_)
        ))
        .then(|| {
            sunrise_gameplay_common::game::commands_common::target_id(t)
                .and_then(|id| self.common.persist.uuid_of(id).map(|uuid| (id, uuid)))
        })
        .flatten();
        match t {
            Target::General(id) => {
                let Some(entity) = self.entity(id) else {
                    return false;
                };
                self.remove_target_now(entity, t);
            }
            Target::Guest(_) => {
                return self
                    .for_player(t, |g| g.kill_target_in_level(Target::Player))
                    .unwrap_or(false);
            }
            Target::Player => {
                if self
                    .common
                    .player
                    .as_mut()
                    .is_none_or(|p| p.set_health(0.0, false).is_err())
                {
                    return false;
                }
                self.common.ui.death_cause = Some("death.attack.genericKill".into());
            }
            Target::Mob(id) => {
                let mut killed = None;
                if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                    // LivingEntity.kill: a generic_kill hit of Float.MAX_VALUE,
                    // which its death message tells.
                    let alive = m.alive();
                    if alive {
                        let source = sunrise_sim::mobs::Damage::of(
                            sunrise_sim::mobs::DamageKind::GenericKill,
                        );
                        m.record_damage(source, f32::MAX);
                        m.last_damage = Some((source, i64::from(m.age)));
                    }
                    m.health = 0.0;
                    killed = alive.then(|| m.clone());
                }
                // The hit runs LivingEntity.die before the command returns:
                // its ENTITY_DIE, loot and experience come now, not in the
                // next tick's deaths, which skip it (as a frog's bite does).
                let Some(m) = killed.filter(|m| !m.removed && self.server.died.insert(id)) else {
                    return false;
                };
                self.mob_die(&m);
                if self
                    .common
                    .mobs
                    .iter()
                    .any(|m| m.id == id && !m.removed && (m.alive() || m.death_canceled))
                {
                    return false;
                }
            }
            Target::Item(id) => {
                if let Some(i) = self.common.dropped.iter().position(|i| i.id == id) {
                    self.remove_dropped(i);
                }
            }
            // Entity.kill removes the orb.
            Target::Orb(id) => self.common.orbs.retain(|o| o.id != id),
            Target::Projectile(id) => self.remove_projectile(id),
            Target::Stand(id) => {
                if !self.remove_armor_stand_after(id, |_| {}) {
                    return false;
                }
            }
            Target::Frame(id) => {
                // ItemFrame.onKilled.
                if let Some(frame) = self.common.item_frames.iter().find(|f| f.id == id).cloned() {
                    self.remove_framed_map(&frame);
                }
                self.common.item_frames.retain(|f| f.id != id);
            }
            Target::Painting(id) => self.common.paintings.retain(|p| p.id != id),
            Target::Boat(_) | Target::Cart(_) => {
                if !self.kill_vehicle_object(t) {
                    return false;
                }
            }
            Target::Other(id) => {
                if self
                    .with_destroying_vehicle(t, |game| {
                        game.server.others.0.retain(|o| o.id != id);
                        game.forget_entity(id);
                    })
                    .is_none()
                {
                    return false;
                }
            }
            Target::Fangs(id) => self.common.fangs.list.retain(|f| f.id != id),
            Target::Crystal(id) => self.kill_crystal(id),
            Target::Dragon(id) => self.kill_dragon(id),
            Target::Tnt(id) => {
                if self.take_removed_tnt(id).is_none() {
                    return false;
                }
            }
            Target::Cushion(id) => {
                if self.take_removed_cushion(id).is_none() {
                    return false;
                }
            }
            Target::Falling(id) => self.common.falling.retain(|f| f.id != id),
            Target::Rocket(id) => self.common.rockets.retain(|r| r.id != id),
            Target::Trident(id) => self.common.tridents.retain(|t| t.id != id),
            Target::Spawner(id) => self.remove_ominous_spawner(id),
            Target::Cloud(id) => self.remove_breath_cloud(id),
            Target::Bolt(id) => self.remove_bolt(id),
            Target::Hook(id) => self.remove_fishing_hook(id),
            Target::Knot(id) => {
                if !self.kill_natural_knot(id) {
                    return false;
                }
            }
            // No command names a part, and `kill` on one would not stop its dragon.
            Target::DragonPart(_) => return false,
        }
        if let Some((id, uuid)) = destroyed_score {
            self.score_destroyed_entity_once(id, uuid);
        }
        if let Some(at) = died {
            self.post_game_event("minecraft:entity_die", at, context);
        }
        self.native_leaves_flush();
        true
    }
    /// The entities of the level in place a selector can find: one walk over
    /// the entity lists (a kind added there joins `@e` once it names its
    /// `Target`), then the kinds that are not on the walk. An
    /// unsorted selector lists them in that order.
    /// With only the `Entity::isAlive` ones when `alive_only`:
    /// `EntitySelectorParser` gives `@e` and `@n` that predicate, so a killed
    /// mob through its death animation, a stand or a dragon at no health is
    /// not among them.
    pub fn level_candidates(&self, alive_only: bool) -> Vec<Target> {
        use super::entity::{Entity as _, entity_lists_of, for_each_entity};
        let mut candidates = Vec::new();
        for_each_entity!(entity_lists_of!(self), entity => {
            if !alive_only || entity.is_alive() {
                candidates.extend(entity.selector_target());
            }
        });
        candidates.extend(self.common.item_frames.iter().map(|f| Target::Frame(f.id)));
        candidates.extend(self.common.paintings.iter().map(|p| Target::Painting(p.id)));
        candidates.extend(self.common.cushions.iter().map(|c| Target::Cushion(c.id)));
        candidates.extend(self.server.others.0.iter().map(|o| Target::Other(o.id)));
        candidates.extend(self.common.fangs.list.iter().map(|f| Target::Fangs(f.id)));
        // The End crystals and the dragon (EndCrystal and EnderDragon
        // are entities of their own).
        candidates.extend(self.common.crystals.iter().map(|c| Target::Crystal(c.id)));
        // EnderDragon.isAlive is its health's, through the dying flight too.
        if let Some(d) = self.common.dragon.as_ref()
            && (!alive_only || d.health > 0.0)
        {
            candidates.push(Target::Dragon(d.id));
        }
        candidates.extend(
            self.common
                .ominous_spawners
                .iter()
                .map(|s| Target::Spawner(s.id)),
        );
        candidates.extend(
            self.common
                .breath_clouds
                .iter()
                .filter(|c| c.id != 0)
                .map(|c| Target::Cloud(c.id)),
        );
        candidates.extend(
            self.common
                .bolts
                .iter()
                .filter(|b| b.id != 0)
                .map(|b| Target::Bolt(b.id)),
        );
        // The player in place keeps their hook across level swaps.
        let player_here = self
            .current_player_resident_location()
            .is_some_and(|location| location.owner.dimension == self.common.dim.key());
        candidates.extend(
            self.fishing_hooks_here()
                .filter(|(_, caster)| player_here || *caster != Target::Player)
                .map(|(hook, _)| Target::Hook(hook.id())),
        );
        candidates.extend(self.natural_knot_ids().into_iter().map(Target::Knot));
        // `isRemoved` from a mod's call on, though the sweep has not taken it out.
        candidates.retain(|t| !self.target_removed(t));
        candidates
    }
    /// `ServerLevel.getEntity`: whether the level in place still holds the
    /// entity a target names. Discard removes mobs from lookup immediately.
    pub(in crate::game) fn level_holds(&self, t: Target) -> bool {
        let Some(id) = self.resident_target_id(t) else {
            return false;
        };
        match self.server_resident_entity(id) {
            sunrise_sim::join_seq::residency::table::EntityLookup::Found { location, value }
                if location.owner.dimension == self.common.dim.key() =>
            {
                value == t
                    || matches!(t, Target::Guest(conn) if self.server.lan_acting == Some(conn) && value == Target::Player)
            }
            _ => false,
        }
    }
    /// `MinecraftServer.sendSystemMessage`: a line in the server's log, as the
    /// console's replies are logged. Kept out of an RCON reply, which carries
    /// only the source's own output.
    pub(in crate::game::server_side) fn log_server_system_message(&self, line: &Text) {
        let message = format!("System chat: {}", line.plain());
        if self.common.is_dedicated_server() {
            super::dedicated::log_outside_rcon_reply(self.server.rcon_source, &message);
        } else {
            log::info!("{message}");
        }
    }
    /// `native_player_level_change` for a level that was `from` before an experience change.
    pub(in crate::game) fn native_player_level_changed(&mut self, from: i32) {
        let to = self.common.experience.level;
        if from != to
            && mod_api::__private::listened(mod_api::HookId::PlayerLevelChange)
            && let (Some(player), Some(name)) = (
                self.server.lan_acting_uuid,
                self.server.lan_acting_name.clone(),
            )
        {
            self.native_player_level_change(player, &name, from, to);
        }
    }
    /// `ServerPlayer.lookAt`: besides turning the player, tell their client
    /// with a `ClientboundPlayerLookAtPacket` (`from` anchor, the point, and
    /// the entity with its anchor when the target is one). Only a connected
    /// player has a client to tell. The entity form is sent whenever the
    /// target is an entity with a network id (`LookAt.LookAtEntity.perform`);
    /// a player whose id is not known here (a guest's turn, with the host
    /// out of reach) falls back to the point, which a client takes the same
    /// unless the entity moves first.
    /// `Entity.getId` as the players' clients know it: a player's is the one the
    /// hub gave it, any other entity's is its own id.
    pub fn network_entity_id(&self, t: Target) -> Option<i32> {
        let host = self.server.lan.as_ref();
        match t {
            Target::Player => self.player_entity_id_by_connection(self.server.lan_acting),
            Target::Guest(conn) => host?.hub.guest(conn).map(|g| g.entity_id),
            other => targets::non_player_entity_id(other),
        }
    }
    /// Player.getDisplayName: the name (team-formatted), suggesting a
    /// `/tell`, showing the player on hover, inserting the name.
    pub(in crate::game) fn player_display_name(&self) -> Text {
        let name = self.acting_player_name();
        if let Some(uuid) = self.server.lan_acting_uuid {
            return self.common.player_text(name, uuid);
        }
        let mut displayname = self.target_name(Target::Player);
        sunrise_sim::lan::hub::native_player_name_format(
            &self.common.loop_context,
            &Text::literal(name),
            &mut displayname,
        );
        displayname
    }
    /// Runs `f` and reports the commands it ran as their source's
    /// `CommandResultCallback`s saw them, in the pair `execute store` stores:
    /// whether any succeeded (the rule of a command block, `run_as_game_master`:
    /// an `execute as @a run ...` forks and calls back once per fork) and the
    /// last callback's result. None when no command reached a callback (a
    /// syntax error, an unknown command, an `execute` forking to no target).
    pub fn recording_command_outcome<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> T,
    ) -> (T, Option<(bool, i32)>) {
        let outer = self.server.cmds.0.results.replace(Vec::new());
        let out = f(self);
        let results = std::mem::replace(&mut self.server.cmds.0.results, outer).unwrap_or_default();
        let outcome = results
            .last()
            .map(|&(_, result)| (results.iter().any(|&(success, _)| success), result));
        (out, outcome)
    }
    /// LocalCoordinates use the anchored position; world coordinates the position.
    pub(in crate::game) fn resolve_coords(&self, c: &Coordinates) -> [f64; 3] {
        let origin = match (c, &self.server.command_source) {
            (Coordinates::Local(_), Some(s)) => self.anchor_position(s),
            _ => self.source_position(),
        };
        c.position(origin, self.source_rotation())
    }
    /// Commands.performCommand for a line without its slash. A client mod's
    /// `command` hook is the caller's: this also runs on a server.
    pub fn run_command(&mut self, command: &str) -> bool {
        let d = self.common.ui.dispatcher.0.clone();
        let parse = d.parse(command, &self.common.ui.registries);
        let result = match parse.executable() {
            Err(e) => {
                self.syntax_failure(&e, command);
                return false;
            }
            Ok((ctx, _)) => {
                let ctx = ctx.clone();
                let root = parse.context.clone();
                self.server.exec.0 = Default::default();
                self.run_chain(&root, &ctx)
            }
        };
        let ok = result.is_ok();
        if let Err(e) = result {
            if e.input.is_some() {
                self.syntax_failure(&e, command);
            } else {
                self.command_failure(e.message);
            }
        }
        // InventoryChangeTrigger fires as slots change, not on the next tick.
        if self.common.player.is_some() {
            self.check_inventory_trigger();
        }
        ok
    }
    /// `run_command` with its output suppressed
    /// (`CommandSourceStack.withSuppressedOutput`): the player's own source,
    /// rights and checks, no feedback and no failure in the chat.
    pub fn run_command_quietly(&mut self, command: &str) -> bool {
        let quiet = execute::Source {
            silent: true,
            ..self.player_source()
        };
        let outer = self.server.command_source.replace(quiet);
        let ok = self.run_command(command);
        self.server.command_source = outer;
        ok
    }
    pub(in crate::game::server_side) fn send_player_look_at(
        &mut self,
        t: Target,
        from_eyes: bool,
        target: [f64; 3],
        entity: Option<(Target, bool)>,
    ) {
        let packet = C::PlayerLookAt(play::PlayerLookAt {
            from: i32::from(from_eyes),
            target,
            entity: entity.and_then(|(e, eyes)| {
                Some(play::LookAtEntity {
                    id: self.network_entity_id(e)?,
                    anchor: i32::from(eyes),
                })
            }),
        });
        match t {
            Target::Player if self.server.lan_acting.is_some() => {
                self.server.guest_packets.push(packet)
            }
            Target::Guest(conn) => {
                if let Some(host) = self.server.lan.as_ref() {
                    host.hub.send_to(conn, &packet);
                }
            }
            _ => {}
        }
    }
    /// `CommandSourceStack.sendSuccess(message, broadcast)`, the success path of
    /// vanilla's commands and of mods' (`Commands::feedback`). A silent source
    /// sends nothing. The line goes to the origin when it accepts success: a
    /// player under `send_command_feedback`, the console and RCON always, a
    /// command block or minecart that tracks output under `send_command_feedback`;
    /// never to the `execute as` entity. Independently, with `broadcast`,
    /// `broadcastToAdmins` tells the other operators and the log.
    ///
    /// Runs with the executing level in place (`source.dimension`). A block or
    /// minecart origin is read in its own level (`source.origin_dimension`),
    /// which `execute in` does not move.
    pub(in crate::game) fn send_success(&mut self, message: Text, broadcast: bool) {
        if self
            .server
            .command_source
            .as_ref()
            .is_some_and(|s| s.silent)
        {
            return;
        }
        let admins = broadcast.then(|| message.clone());
        if let Some(block) = self.server.command_source.as_ref().and_then(|s| s.block) {
            self.in_origin_level(|g| g.command_block_success(block, message));
        } else if let Some(cart) = self.server.command_source.as_ref().and_then(|s| s.cart) {
            self.in_origin_level(|g| g.cart_command_success(cart, message));
        } else if self.accepts_success() {
            self.send_command_output(CommandFeedback::Success, message);
        }
        if let Some(message) = admins {
            self.broadcast_to_admins(message);
        }
    }
    pub fn set_game_mode(&mut self, mut mode: GameMode) -> bool {
        if self.common.mode == mode {
            return false;
        }
        // A server player's change is heard by the mods first, who may keep or redirect it.
        if let Some(uuid) = self.server.lan_acting_uuid {
            let name = self.acting_player_name().to_owned();
            let mut cancel = false;
            self.native_player_game_mode_changed(
                uuid,
                &name,
                self.common.mode,
                &mut mode,
                &mut cancel,
            );
            if cancel || self.common.mode == mode {
                return false;
            }
        }
        self.set_mode(mode);

        true
    }
    /// `CommandSource.shouldInformAdmins` of the command's origin: a player's
    /// always (`ServerPlayer`); a command block's or minecart's while it tracks
    /// output and `command_block_output` is on (`BaseCommandBlock`, which reads
    /// the block itself: looked up in its own level); the console's
    /// `broadcast-console-to-ops` (`DedicatedServer`) and RCON's
    /// `broadcast-rcon-to-ops` (`RconConsoleSource`).
    pub(in crate::game::server_side) fn should_inform_admins(&mut self) -> bool {
        let origin = self
            .server
            .command_source
            .as_ref()
            .and_then(|s| s.block.map(Ok).or(s.cart.map(Err)));
        let Some(origin) = origin else {
            return self.server.console_broadcast.unwrap_or(true);
        };
        let tracks = self.in_origin_level(|g| match origin {
            Ok(pos) => g.command_block(pos).is_some_and(|c| c.track_output),
            Err(id) => g.carts.iter().any(|c| {
                c.id == id
                    && c.kind == super::minecarts::CartKind::CommandBlock
                    && c.command.track_output
            }),
        });
        tracks
            && self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::COMMAND_BLOCK_OUTPUT)
    }
    pub(in crate::game) fn source_position(&self) -> [f64; 3] {
        if let Some(s) = &self.server.command_source {
            return s.pos;
        }
        match &self.common.player {
            Some(p) => [p.position.x, p.position.y, p.position.z],
            None => self.server_position(),
        }
    }
    pub(in crate::game) fn source_rotation(&self) -> [f32; 2] {
        self.server
            .command_source
            .as_ref()
            .map_or([self.common.yaw, self.common.pitch], |s| s.rot)
    }
    /// What changes when an entity spawns, kinds that take their id only
    /// when first tracked (primed TNT, falling blocks, projectiles)
    /// included: a summon compares it before and after.
    pub(in crate::game::server_side) fn spawned_mark(&self) -> (u32, usize) {
        (
            self.server.next_entity,
            self.common.tnts.len()
                + self.common.falling.len()
                + self.common.creatures.projectiles.len()
                + self.common.rockets.len()
                + self.common.tridents.len(),
        )
    }
    /// The entries of a data pack tag of `registry`, nested tags expanded.
    pub(in crate::game) fn tag_entries(&self, registry: &str, tag: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut pending = vec![tag.to_owned()];
        let mut seen = std::collections::HashSet::new();
        while let Some(t) = pending.pop() {
            if !seen.insert(t.clone()) {
                continue;
            }
            let (ns, path) = t.split_once(':').unwrap_or(("minecraft", &t));
            let file = self
                .common
                .options
                .resources
                .join(format!("data/{ns}/tags/{registry}/{path}.json"));
            // The game's file and the world's packs' both count, entries
            // joined in resource-stack order; a pack file with `replace`
            // drops what the lower sources put in (`TagLoader.load`).
            let packed = self
                .server
                .fns
                .0
                .store
                .files
                .get(&format!("tags/{ns}:{registry}/{path}"))
                .into_iter()
                .flatten()
                .filter_map(|b| serde_json::from_slice::<serde_json::Value>(b).ok());
            let parsed = std::fs::read(file)
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .into_iter()
                .chain(packed);
            let mut entries = Vec::new();
            for v in parsed {
                if v["replace"].as_bool() == Some(true) {
                    entries.clear();
                }
                entries.extend(v["values"].as_array().into_iter().flatten().cloned());
            }
            for e in &entries {
                let id = e.as_str().or_else(|| e["id"].as_str()).unwrap_or_default();
                match id.strip_prefix('#') {
                    Some(nested) => pending.push(nested.to_owned()),
                    None if !id.is_empty() => out.push(id.to_owned()),
                    None => {}
                }
            }
        }
        out
    }
    /// `t` teleported into another level than its own: an entity crosses at
    /// once (`teleport_target_across`), a server's player is queued to move
    /// after the command (ServerPlayer.teleport). False when `t` cannot
    /// change level or is already in `level`.
    pub fn teleport_across(
        &mut self,
        t: Target,
        level: super::Dim,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
        turn_riders: bool,
        motion: TeleportMotion,
    ) -> R<bool> {
        match self.teleport_target_across(t, level, pos, rotation, turn_riders, motion) {
            Ok(Some(moved)) => return Ok(moved),
            Ok(None) => {}
            Err(error) => {
                log::error!("entity teleport into {}: {error}", level.id());
                return Err(err("command.failed", &[]));
            }
        }
        let conn = match t {
            Target::Player => self.server.lan_acting,
            Target::Guest(conn) => Some(conn),
            _ => None,
        };
        let at = self.target_level(t);
        let Some(conn) = conn.filter(|_| at != level) else {
            return Ok(false);
        };
        // A fake player stays in its level. True: the teleport is taken, so no
        // same-level teleport stands in for it.
        if Some(conn) == self.server.lan_acting
            && self.fake_blocks(mod_api::FakeEffect::ChangeDimension)
        {
            return Ok(true);
        }
        // A canceled travel keeps the player where they are.
        if !self.server_player_travel_allowed(
            conn,
            level,
            super::server_travel::TravelCause::Teleport,
        ) {
            return Ok(true);
        }
        self.server
            .level_teleports
            .push(super::server_travel::LevelTeleport {
                conn,
                dim: level,
                at: pos,
                rotation,
                vehicle: None,
            });
        Ok(true)
    }
    /// `Entity.teleportTo(level, x, y, z, relatives, yRot, xRot, ...)`, as
    /// `/tp` and `/spreadplayers` call it: its transition has no motion and no
    /// relative delta, so `teleportSetPosition` leaves the moving entity still.
    /// (An end gateway's or a vehicle's transition carries its motion.)
    pub(in crate::game) fn teleport_stops(&mut self, t: Target) {
        let _ = self.in_target_level(t, |g| {
            target_entity_mut!(g, t, e => *e.velocity_mut() = Vec3::ZERO);
            // The dragon's flight builds on the motion it carries.
            if let Target::Dragon(id) = t
                && let Some(d) = g.dragon.as_mut().filter(|d| d.id == id)
            {
                d.teleport_motion([false; 3]);
            }
        });
    }
    pub(in crate::game) fn teleport_to_relative(
        &mut self,
        t: Target,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
        relative: [bool; 3],
        turn_riders: bool,
    ) {
        // An entity in another level is teleported in the level that holds it.
        let _ = self.in_target_level(t, |g| {
            g.teleport_to_in_level(t, pos, rotation, relative, turn_riders)
        });
    }
    /// `TeleportCommand.performTeleport` after a successful `teleportTo` in
    /// the target's own level: unless it is a fall-flying living entity, its
    /// vertical motion is zeroed and it is on the ground; a mob's navigation
    /// stops (`PathfinderMob`; the others hold no path). The players' own
    /// ground is their clients'.
    pub(in crate::game::server_side) fn tp_lands(&mut self, t: Target) {
        // Player placement already handles motion and fall-flying ground state.
        if matches!(t, Target::Player | Target::Guest(_)) {
            return;
        }
        let _ = self.in_target_level(t, |g| {
            if let Target::Mob(id) = t
                && let Some(m) = g.mobs.iter_mut().find(|m| m.id == id)
            {
                m.nav_stop();
                if m.extra.fall_flying {
                    return;
                }
            }
            let common = &mut g.common;
            let server = &g.server;
            target_entity_mut!(common, t, e => {
                e.velocity_mut().y = 0.0;
                let items = match (&common.server_items, server.world_tables.0.as_ref()) {
                    (Some(items), _) => &**items,
                    (None, Some(tables)) => &tables.items,
                    (None, None) => common.res.items(),
                };
                if matches!(e.update_retained_ground(&common.world, common.collision, &items.entity_type_tags, true, None), Ok(false)) {
                    e.set_on_ground(true);
                }
            });
        });
    }
    /// `PlayerXpEvent.XpChange` for a `Player.giveExperiencePoints` of `amount`
    /// points: the amount to give, or `None` when a mod cancelled it.
    pub(in crate::game) fn xp_change(&mut self, amount: i32) -> Option<i32> {
        if !mod_api::__private::listened(mod_api::HookId::PlayerXpChange) {
            return Some(amount);
        }
        let (Some(player), Some(name)) = (
            self.server.lan_acting_uuid,
            self.server.lan_acting_name.clone(),
        ) else {
            return Some(amount);
        };
        let (mut amount, mut cancel) = (amount, false);
        let player_entity_id = self
            .server
            .lan_acting_entity
            .map_or_else(|| self.own_entity_id(), |id| id as i32);
        self.native_player_xp_change(player, player_entity_id, &name, &mut amount, &mut cancel);
        (!cancel).then_some(amount)
    }
}

#[cfg(any(test, feature = "test-support"))]
#[path = "enchant_tests_server.rs"]
pub mod enchant_tests;

#[cfg(any(test, feature = "test-support"))]
#[path = "clone_writes_tests_server.rs"]
pub mod clone_writes_tests;

#[cfg(test)]
pub(in crate::game) mod forced_respawn_tests {
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

    #[test]
    #[ignore = "daily"]
    fn setworldspawn_reports_normalized_rotation() {
        use crate::game::native_entity_refs::tests::with_game;
        use std::sync::Arc;
        use sunrise_core::geometry::Vec3;
        use sunrise_game_data::text::Text;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::motion::PlayerMotion;

        with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(5.5, 64.0, 5.5)).unwrap());
            game.build_registries();
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.server.chat_capture = Some(Vec::new());

            assert!(game.run_command("setworldspawn 5 64 5 270 120"));
            assert_eq!(game.level.world_spawn_angle, [-90.0, 90.0]);
            let expected = Text::translate(
                "commands.setworldspawn.success",
                &["5", "64", "5", "-90.0", "90.0", "minecraft:overworld"].map(Text::literal),
            );
            let lines = game.server.chat_capture.take().unwrap();
            assert_eq!(lines.len(), 1);
            assert_eq!(lines[0].plain(), expected.plain());
        });
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(test)]
mod persistent_component_tests {
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
    #[test]
    #[ignore = "daily"]
    fn native_persistent_components_are_available_to_give() {
        sunrise_protocol::item::custom::install(1000, vec!["example:marker".into()]).unwrap();
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.build_registries();
            game.build_registries();
            game.ui.registries.permission = 4;
            assert_eq!(
                game.ui
                    .registries
                    .persistent_components
                    .iter()
                    .filter(|id| *id == "example:marker")
                    .count(),
                1
            );
            let mut suggestions =
                sunrise_command::SuggestionsBuilder::new("minecraft:apple[example:", 0);
            sunrise_command::item::suggest_stack(&mut suggestions, &game.ui.registries);
            assert!(
                suggestions
                    .out
                    .iter()
                    .any(|entry| entry.text == "example:marker=")
            );
            assert!(game.run_command("give @s minecraft:apple[example:marker={probe:1}]"));
            let item = game
                .inventory
                .iter()
                .flatten()
                .find(|item| &*item.item == "minecraft:apple")
                .unwrap();
            assert_eq!(
                item.data().and_then(|data| data.other("example:marker")),
                Some(&sunrise_core::nbt::Tag::compound([(
                    "probe",
                    sunrise_core::nbt::Tag::Int(1)
                )]))
            );
            assert!(!game.run_command("give @s minecraft:apple[minecraft:creative_slot_lock={}]"));
        });
    }
}
