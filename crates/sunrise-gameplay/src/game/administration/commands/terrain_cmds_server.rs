use super::{Arg, Context, R, Target, err, items::Owner};
use crate::game::commands::terrain_cmds::holder;
use crate::game::commands::terrain_cmds::num;
use sunrise_command::CommandError;
use sunrise_core::random::RandomSource;
use sunrise_game_data::text::Text;
use sunrise_items::items::ItemStack;
use sunrise_sim::scoreboard_cmd::Tracker;

impl crate::game::ServerGame<'_> {
    /// `LootCommand.dropBlockLoot`: copy the actual block entity's components
    /// into the mine result without removing the source block or its contents.
    pub(in crate::game::server_side) fn cmd_loot(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let keep_empty = ctx.literals.iter().any(|literal| literal == "replace");
        // The drops.
        let (drops, table): (Vec<ItemStack>, Option<String>) = match id {
            "loot.loot" | "loot.fish" => {
                let Some(Arg::Resource(table)) = ctx.get("loot_table").cloned() else {
                    return Ok(0);
                };
                // An inline table (ResourceOrIdArgument) keeps its random sequence.
                if table.starts_with('{') {
                    let (tag, _) = sunrise_core::nbt::parse_snbt(&table)
                        .map_err(|e| err("argument.nbt.invalid", &[&e]))?;
                    let json = sunrise_game_data::scoreboard::tag_to_json(&tag);
                    sunrise_world::loot::validate_loot_table(&json)
                        .map_err(|e| err("argument.nbt.invalid", &[&e]))?;
                    let prepared = server_world_items!(self)
                        .prepare_inline_loot_table(&json)
                        .map_err(|e| err("argument.nbt.invalid", &[&e.to_string()]))?;
                    // LootCommand's omitted fishing tool is EMPTY, not mainhand.
                    let held = if id == "loot.fish"
                        && (ctx.get("tool").is_some() || ctx.has("mainhand") || ctx.has("offhand"))
                    {
                        self.tool_stack(ctx)?
                    } else {
                        None
                    };
                    let drops = match json["random_sequence"].as_str() {
                        Some(sequence) => {
                            let random = self
                                .server
                                .rand_sequences
                                .get(sequence, self.common.level.seed);
                            server_world_items!(self).roll_prepared_inline_loot_table_for_command(
                                &prepared,
                                held.as_ref(),
                                random,
                                keep_empty,
                            )
                        }
                        None => server_world_items!(self)
                            .roll_prepared_inline_loot_table_for_command(
                                &prepared,
                                held.as_ref(),
                                &mut self.common.world.random,
                                keep_empty,
                            ),
                    };
                    (drops, None)
                } else {
                    if !self.items().has_loot_table(&table) {
                        return Err(err("argument.resource_or_id.failed_to_parse", &[&table]));
                    }
                    let held = if id == "loot.fish" {
                        self.tool_stack(ctx)?
                    } else {
                        None
                    };
                    let drops = match server_world_items!(self).loot_sequence(&table) {
                        Some(seq) => {
                            let random =
                                self.server.rand_sequences.get(&seq, self.common.level.seed);
                            server_world_items!(self).loot_table_for_command(
                                &table,
                                false,
                                held.as_ref(),
                                random,
                                keep_empty,
                            )
                        }
                        None => server_world_items!(self).loot_table_for_command(
                            &table,
                            false,
                            held.as_ref(),
                            &mut self.common.world.random,
                            keep_empty,
                        ),
                    };
                    (drops, None)
                }
            }
            "loot.kill" => {
                let targets = self.entities(ctx, "target", false)?;
                let Some(t) = targets.first().copied() else {
                    return Ok(0);
                };
                let ty = self.target_type(t);
                // `EntityType.getDefaultLootTable`, in the type's namespace.
                let table = sunrise_sim::mobs::drops::entity_type_loot_table(&ty);
                if t == Target::Player || !world_items!(self).has_loot_table(&table) {
                    return Err(err(
                        "commands.drop.no_loot_table.entity",
                        &[&self.target_name(t).plain()],
                    ));
                }
                // LootCommand.dropKillLoot: the source's entity is the killer.
                // A connected player is the last damage player, and its own
                // stack counts; it is in place only to read that stack.
                let guest = match self.server.command_source.as_ref().map(|s| s.entity) {
                    Some(Some(Target::Guest(conn))) => self.for_player(Target::Guest(conn), |g| {
                        g.slot_get(&super::items::Slot::Inv(g.selected))
                    }),
                    _ => None,
                };
                let (by_player, held) = match guest {
                    Some(held) => (true, held),
                    None => (
                        matches!(
                            self.server.command_source.as_ref().map(|s| s.entity),
                            None | Some(Some(Target::Player))
                        ),
                        self.slot_get(&super::items::Slot::Inv(self.common.selected)),
                    ),
                };
                let drops = self.roll_kill_loot(crate::game::loot_rolls::KillRoll {
                    kind: &ty,
                    table: &table,
                    target: t,
                    killer: self
                        .server
                        .command_source
                        .as_ref()
                        .map(|s| s.entity)
                        .unwrap_or(Some(Target::Player)),
                    by_player,
                    held: held.as_ref(),
                    keep_empty,
                });
                (drops, Some(table))
            }
            _ => {
                // loot mine
                let p = self.block_pos_arg(ctx, "pos")?;
                let state = self.common.world.block(p[0], p[1], p[2]);
                let name = self.common.data_block_name(p);
                let Some(table) =
                    world_items!(self).block_loot_table(self.common.pack.assets.registry, state)
                else {
                    return Err(err("commands.drop.no_loot_table.block", &[&name]));
                };
                let tool = self.tool_stack(ctx)?;
                let mut pending = sunrise_gameplay_common::game::spill_common::Pending::default();
                let key = (p[0], p[1], p[2]);
                if !self.common.world.block_entities.contains_key(&key)
                    && let Some(saved) = self.block_entity_nbt(p)
                {
                    pending.record_saved(p, &saved);
                }
                // LootCommand: the source's entity is `this`, if it has one.
                let entity = match self.server.command_source.as_ref().map(|s| s.entity) {
                    None | Some(Some(Target::Player)) => Some(self.native_clicker().1 as u32),
                    Some(Some(Target::Guest(conn))) => self
                        .player_entity_id_by_connection(Some(conn))
                        .map(|id| id as u32),
                    Some(Some(target)) => {
                        sunrise_gameplay_common::game::commands_common::target_id(target)
                    }
                    Some(None) => None,
                };
                // The roll's location checks read the level at the block's centre.
                let roll = sunrise_items::items::BlockLootRoll {
                    held: tool.as_ref(),
                    keep_empty,
                    breaker: entity.is_some(),
                    ..Default::default()
                };
                let source = pending.source(p);
                let drops = sunrise_world::loot::copy::with_block_source(source.as_deref(), || {
                    self.roll_block_loot(p, state, entity, roll)
                });
                (drops, Some(table))
            }
        };
        let drops: Vec<ItemStack> = drops
            .into_iter()
            .filter(|d| keep_empty || d.count > 0)
            .collect();
        // Where they go.
        let mut used: Vec<ItemStack> = Vec::new();
        let literals = &ctx.literals;
        if literals.iter().any(|l| l == "give") {
            for d in &drops {
                for &t in &self.entities(ctx, "players", true)? {
                    // LootCommand.playerGive gives to every target player: a
                    // guest gets it with their state in place, as with /give.
                    let added = self.for_player(t, |g| {
                        let complete = g.creative;
                        if let Some(rest) = g.add_stack_to_inventory(d.clone()) {
                            let p = g.source_position();
                            g.spawn_item(rest, [p[0], p[1] + 1.32, p[2]], [0.0; 3], 0);
                            complete
                        } else {
                            true
                        }
                    });
                    if added == Some(true) {
                        used.push(d.clone());
                    }
                }
            }
        } else if literals.iter().any(|l| l == "spawn") {
            let at = self
                .position_arg(ctx, "targetPos")
                .unwrap_or_else(|| self.source_position());
            for d in &drops {
                let r = &mut self.common.world.random;
                let v = [
                    r.next_double() * 0.2 - 0.1,
                    0.2,
                    r.next_double() * 0.2 - 0.1,
                ];
                self.spawn_item(d.clone(), at, v, 10);
                used.push(d.clone());
            }
        } else if literals.iter().any(|l| l == "insert") {
            let p = self.block_pos_arg(ctx, "targetPos")?;
            let owner = Owner::Block(p);
            let slots = self.slots_named(&owner, "container.*");
            if slots.is_empty() {
                return Err(err(
                    "commands.item.target.not_a_container",
                    &[&p[0], &p[1], &p[2]],
                ));
            }
            for d in &drops {
                let mut left = d.clone();
                let mut placed = false;
                for s in &slots {
                    if !self.slot_accepts(s, Some(&left)) {
                        continue;
                    }
                    match self.slot_get(s) {
                        None => {
                            self.slot_set(s, Some(left.clone()));
                            placed = true;
                            left.count = 0;
                            break;
                        }
                        Some(mut cur)
                            if cur.item == left.item
                                && cur.damage == left.damage
                                && cur.extra == left.extra =>
                        {
                            let max = self.items().get(&cur.item).map_or(64, |i| i.max_stack);
                            let take = left.count.min(max.saturating_sub(cur.count));
                            if take > 0 {
                                cur.count += take;
                                left.count -= take;
                                self.slot_set(s, Some(cur));
                                placed = true;
                            }
                            if left.count == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if placed {
                    used.push(d.clone());
                }
            }
        } else {
            // replace entity|block <slot> [count]
            let block = literals.iter().any(|l| l == "block");
            let owners: Vec<Owner> = if block {
                vec![Owner::Block(self.block_pos_arg(ctx, "targetPos")?)]
            } else {
                self.entities(ctx, "entities", false)?
                    .into_iter()
                    .map(Owner::Entity)
                    .collect()
            };
            let Some(Arg::Str(slot_name)) = ctx.get("slot").cloned() else {
                return Ok(0);
            };
            let count = match ctx.get("count") {
                Some(Arg::Int(c)) => *c as usize,
                _ => drops.len(),
            };
            for o in owners {
                // `prefix.N` continues as `prefix.N+1`, ...
                let (prefix, start) = match slot_name.rsplit_once('.') {
                    Some((p, n)) if n.parse::<usize>().is_ok() => {
                        (Some(p.to_owned()), n.parse::<usize>().unwrap_or(0))
                    }
                    _ => (None, 0),
                };
                let first = self.slots_named(&o, &slot_name);
                if first.is_empty() {
                    if let Owner::Block(p) = &o {
                        return Err(err(
                            "commands.item.target.not_a_container",
                            &[&p[0], &p[1], &p[2]],
                        ));
                    }
                    return Err(err("commands.item.target.no_such_slot", &[&slot_name]));
                }
                for i in 0..count {
                    let slot = match &prefix {
                        Some(p) => self
                            .slots_named(&o, &format!("{p}.{}", start + i))
                            .first()
                            .cloned(),
                        None => first.first().cloned().filter(|_| i == 0),
                    };
                    let Some(slot) = slot else { continue };
                    let item = drops.get(i).filter(|drop| drop.count > 0).cloned();
                    if self.slot_accepts(&slot, item.as_ref())
                        && self.slot_set_by_command(&slot, item.clone())
                    {
                        used.push(item.unwrap_or(ItemStack {
                            item: "minecraft:air".into(),
                            count: 0,
                            damage: 0,
                            extra: None,
                        }));
                    }
                }
            }
        }
        let mut tracker = Tracker::default();
        for u in &used {
            // LootCommand's feedback names each drop by `getDisplayName`.
            let name = self.stack_display_name(u);
            // CommandResponseTracker.track(drop): one per stack, so the
            // command returns how many stacks it used, not how many items.
            tracker.track(&holder(name), 1);
        }
        let counts: Vec<u32> = used.iter().map(|u| u.count).collect();
        let (message, total) = tracker.message(
            false,
            None,
            |h, _| {
                let n = counts.iter().copied().find(|c| *c > 0).unwrap_or(0);
                match &table {
                    Some(t) => Text::translate(
                        "commands.drop.success.single_with_table",
                        &[num(n), h.text.clone(), Text::literal(t.clone())],
                    ),
                    None => {
                        Text::translate("commands.drop.success.single", &[num(n), h.text.clone()])
                    }
                }
            },
            |c, _| match &table {
                Some(t) => Text::translate(
                    "commands.drop.success.multiple_with_table",
                    &[num(c), Text::literal(t.clone())],
                ),
                None => Text::translate("commands.drop.success.multiple", &[num(c)]),
            },
        )?;
        self.feedback(message);
        Ok(total)
    }
    pub(in crate::game::server_side) fn tool_stack(
        &mut self,
        ctx: &Context,
    ) -> R<Option<ItemStack>> {
        if let Some(Arg::Item(p)) = ctx.get("tool") {
            let (damage, extra) = match p
                .components
                .as_deref()
                .map(sunrise_items::item_data::command_components)
            {
                Some(Ok(tag)) => sunrise_items::item_data::decode_components(Some(&tag)),
                Some(Err(e)) => return Err(CommandError::new(Text::literal(e))),
                None => (0, None),
            };
            return Ok(Some(ItemStack {
                item: p.id.as_str().into(),
                count: 1,
                damage,
                extra,
            }));
        }
        let entity = self
            .server
            .command_source
            .as_ref()
            .and_then(|s| s.entity)
            .unwrap_or(Target::Player);
        let slot = if ctx.has("offhand") {
            "weapon.offhand"
        } else {
            "weapon.mainhand"
        };
        Ok(self
            .slots_named(&Owner::Entity(entity), slot)
            .first()
            .and_then(|s| self.slot_get(s)))
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;
