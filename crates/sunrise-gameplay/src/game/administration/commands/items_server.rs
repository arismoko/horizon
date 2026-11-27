use super::{Arg, Context, R, Target, err};
pub(in crate::game::server_side) use crate::game::commands::items::Owner;
pub(in crate::game::server_side) use crate::game::commands::items::Slot;
use crate::game::commands::items::edit_components;
use crate::game::commands::items::list_operation;
use crate::game::commands::items::mob_slot;
use crate::game::commands::items::set_custom_data;
use crate::game::commands::items::*;
use sunrise_core::nbt::Tag;
use sunrise_game_data::text::Text;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::{furnace::FurnaceCooking, scoreboard_cmd::Tracker};

#[path = "items/slot_sources_server.rs"]
pub(in crate::game) mod slot_sources;

impl crate::game::ServerGame<'_> {
    /// One item modifier (a typed object, a list or a datapack reference)
    /// applied to a stack; `None` when the stack ends up empty.
    pub fn apply_item_modifier(
        &mut self,
        m: &serde_json::Value,
        stack: Option<ItemStack>,
        rng: &mut sunrise_core::random::LegacyRandom,
        this: Option<&sunrise_sim::advancements::EntityView>,
        origin: [f64; 3],
        depth: u32,
    ) -> Option<ItemStack> {
        use serde_json::Value;
        use sunrise_core::random::RandomSource;
        if depth > 16 {
            return stack;
        }
        let int = |v: &Value, rng: &mut sunrise_core::random::LegacyRandom| -> i32 {
            if let Some(n) = v.as_f64() {
                return n as i32;
            }
            let min = v["min"].as_f64().unwrap_or(0.) as i32;
            let max = v["max"].as_f64().unwrap_or(0.) as i32;
            match v["type"].as_str().unwrap_or("minecraft:constant") {
                "minecraft:uniform" | "uniform" => rng
                    .next_int_between_inclusive(min, max.max(min))
                    .unwrap_or(min),
                _ => v["value"].as_f64().unwrap_or(0.) as i32,
            }
        };
        match m {
            Value::Array(list) => list.iter().fold(stack, |s, f| {
                self.apply_item_modifier(f, s, rng, this, origin, depth + 1)
            }),
            Value::String(id) => match self.server.fns.0.store.item_modifiers.get(id).cloned() {
                Some(f) => self.apply_item_modifier(&f, stack, rng, this, origin, depth + 1),
                None => stack,
            },
            Value::Object(_) => {
                let mut stack = stack?;
                // 26.3 item modifiers: a single `condition`, dispatched on `type`.
                if !m["condition"].is_null() && !self.conditions_hold(&m["condition"], this, origin)
                {
                    return Some(stack);
                }
                match m["type"].as_str().unwrap_or("") {
                    "minecraft:sequence" => {
                        #[allow(clippy::manual_try_fold)]
                        return m["functions"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .fold(Some(stack), |s, f| {
                                self.apply_item_modifier(f, s, rng, this, origin, depth + 1)
                            });
                    }
                    "minecraft:exploration_map" => {
                        if stack.count > 0 {
                            let map = sunrise_world::loot::ExplorationMap::from_json(m);
                            // A failed command search keeps the input; pending loot's
                            // caller owns its separate filtered/drop policy.
                            self.materialize_exploration_map(&mut stack, &map, origin);
                        }
                    }
                    "minecraft:set_count" => {
                        let n = int(&m["count"], rng);
                        let base = if m["add"].as_bool() == Some(true) {
                            stack.count as i32
                        } else {
                            0
                        };
                        let max = self
                            .common
                            .res
                            .items()
                            .get(&stack.item)
                            .map_or(64, |i| i.max_stack);
                        stack.count = (base + n).clamp(0, max as i32) as u32;
                    }
                    "minecraft:limit_count" => {
                        let min = m["limit"]["min"].as_i64().or(m["limit"].as_i64());
                        let max = m["limit"]["max"].as_i64().or(m["limit"].as_i64());
                        if let Some(min) = min {
                            stack.count = stack.count.max(min as u32);
                        }
                        if let Some(max) = max {
                            stack.count = stack.count.min(max as u32);
                        }
                    }
                    "minecraft:set_damage" => {
                        let max = self
                            .common
                            .res
                            .items()
                            .get(&stack.item)
                            .map_or(0, |i| i.max_damage);
                        if max > 0 {
                            let f = match m["damage"].as_f64() {
                                Some(f) => f as f32,
                                None => {
                                    let lo = m["damage"]["min"].as_f64().unwrap_or(0.) as f32;
                                    let hi = m["damage"]["max"].as_f64().unwrap_or(0.) as f32;
                                    lo + rng.next_float() * (hi - lo)
                                }
                            };
                            let base = if m["add"].as_bool() == Some(true) {
                                1. - stack.damage as f32 / max as f32
                            } else {
                                0.
                            };
                            let frac = (base + f).clamp(0., 1.);
                            stack.damage = ((1. - frac) * max as f32).floor() as u32;
                        }
                    }
                    "minecraft:set_name" => {
                        if m["name"].is_null() {
                            return Some(stack);
                        }
                        if m["target"].as_str() == Some("item_name") {
                            let name = sunrise_game_data::scoreboard::json_to_tag(&m["name"]);
                            edit_components(&mut stack, |c| {
                                c.insert("minecraft:item_name".into(), name);
                            });
                        } else {
                            // SetNameFunction.run preserves the component when no entity resolver is requested.
                            stack
                                .data_mut()
                                .set_loot_name("minecraft:custom_name", &m["name"]);
                        }
                    }
                    "minecraft:set_enchantments" => {
                        let add = m["add"].as_bool() == Some(true);
                        let d = stack.extra.get_or_insert_with(Default::default);
                        let target = if &*stack.item == "minecraft:enchanted_book" {
                            &mut d.stored_enchantments
                        } else {
                            &mut d.enchantments
                        };
                        for (id, level) in m["enchantments"].as_object().into_iter().flatten() {
                            let n = int(level, rng).clamp(0, 255) as u32;
                            let id: std::sync::Arc<str> = if id.contains(':') {
                                id.as_str().into()
                            } else {
                                format!("minecraft:{id}").into()
                            };
                            match target.iter_mut().find(|(e, _)| *e == id) {
                                Some(e) => e.1 = if add { e.1 + n } else { n },
                                None => target.push((id, n)),
                            }
                            target.retain(|(_, l)| *l > 0);
                        }
                    }
                    "minecraft:set_potion" => {
                        if let Some(p) = m["id"].as_str() {
                            stack.extra.get_or_insert_with(Default::default).potion =
                                Some(p.into());
                        }
                    }
                    "minecraft:set_lore" => {
                        let lines: Vec<Tag> = m["lore"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(sunrise_game_data::scoreboard::json_to_tag)
                            .collect();
                        edit_components(&mut stack, |c| {
                            let old: Vec<Tag> = match c.get(&"minecraft:lore".into()) {
                                Some(Tag::List(l)) => l.clone(),
                                _ => Vec::new(),
                            };
                            let new = list_operation(m, old, lines);
                            c.insert("minecraft:lore".into(), Tag::List(new));
                        });
                    }
                    "minecraft:set_custom_data" => {
                        let tag = match &m["tag"] {
                            Value::String(snbt) => sunrise_core::nbt::parse_snbt(snbt)
                                .map(|(t, _)| t)
                                .unwrap_or(Tag::Compound(Default::default())),
                            v => sunrise_game_data::scoreboard::json_to_tag(v),
                        };
                        set_custom_data(&mut stack, &tag);
                    }
                    "minecraft:set_components" => {
                        let patch = m["components"].clone();
                        edit_components(&mut stack, |c| {
                            for (k, v) in patch.as_object().into_iter().flatten() {
                                let (remove, key) = match k.strip_prefix('!') {
                                    Some(rest) => (true, rest),
                                    None => (false, k.as_str()),
                                };
                                let Ok(id) = sunrise_core::identifier::Identifier::parse(key)
                                else {
                                    continue;
                                };
                                let key =
                                    sunrise_core::nbt::JavaString::from(id.to_string().as_str());
                                let marker =
                                    sunrise_core::nbt::JavaString::from(format!("!{id}").as_str());
                                c.remove(&key);
                                c.remove(&marker);
                                if remove {
                                    c.insert(marker, Tag::Compound(Default::default()));
                                } else {
                                    c.insert(key, sunrise_game_data::scoreboard::json_to_tag(v));
                                }
                            }
                        });
                    }
                    "minecraft:toggle_tooltips" => {
                        let toggles = m["toggles"].clone();
                        edit_components(&mut stack, |c| {
                            let mut hidden: Vec<String> = c
                                .get(&"minecraft:tooltip_display".into())
                                .and_then(|d| d.get("hidden_components"))
                                .and_then(|l| match l {
                                    Tag::List(l) => Some(
                                        l.iter()
                                            .filter_map(|t| match t {
                                                Tag::String(s) => Some(s.to_string_lossy()),
                                                _ => None,
                                            })
                                            .collect(),
                                    ),
                                    _ => None,
                                })
                                .unwrap_or_default();
                            let hide_tooltip = c
                                .get(&"minecraft:tooltip_display".into())
                                .and_then(|d| d.get("hide_tooltip"))
                                .cloned();
                            for (k, shown) in toggles.as_object().into_iter().flatten() {
                                let key = if k.contains(':') {
                                    k.clone()
                                } else {
                                    format!("minecraft:{k}")
                                };
                                hidden.retain(|h| *h != key);
                                if shown.as_bool() == Some(false) {
                                    hidden.push(key);
                                }
                            }
                            let mut display: Vec<(&str, Tag)> = Vec::new();
                            if let Some(h) = hide_tooltip {
                                display.push(("hide_tooltip", h));
                            }
                            if !hidden.is_empty() {
                                display.push((
                                    "hidden_components",
                                    Tag::List(
                                        hidden
                                            .iter()
                                            .map(|h| Tag::String(h.as_str().into()))
                                            .collect(),
                                    ),
                                ));
                            }
                            c.insert("minecraft:tooltip_display".into(), Tag::compound(display));
                        });
                    }
                    "minecraft:filtered" => {
                        let pass = self.loot_item_filter(&m["item_filter"], &stack);
                        let branch = if pass { &m["on_pass"] } else { &m["on_fail"] };
                        if branch.is_null() {
                            return Some(stack);
                        }
                        return self.apply_item_modifier(
                            branch,
                            Some(stack),
                            rng,
                            this,
                            origin,
                            depth + 1,
                        );
                    }
                    "minecraft:discard" => stack.count = 0,
                    _ => {}
                }
                (stack.count > 0).then_some(stack)
            }
            _ => stack,
        }
    }
    pub(in crate::game::server_side) fn block_slot_set(
        &mut self,
        p: [i32; 3],
        i: usize,
        stack: Option<ItemStack>,
    ) -> bool {
        let key = (p[0], p[1], p[2]);
        let Some(_) = self.block_container_size(p).filter(|&size| i < size) else {
            return false;
        };
        self.unpack_loot(
            sunrise_gameplay_common::game::container_loot_level::LootTarget::Block(key),
            None,
        );
        let book = stack.as_ref().is_none_or(|s| {
            self.items()
                .item_tags
                .get("minecraft:bookshelf_books")
                .is_some_and(|m| m.contains(&*s.item))
        });
        let recipes = match self.server.world_tables.0.as_ref() {
            Some(tables) => &tables.recipes,
            None => self.common.res.recipes(),
        };
        // `ChiseledBookShelfBlockEntity.setItem`: the slot written is the one
        // `updateState` takes, even when its occupancy does not change.
        let mut shelf_slot = None;
        match self.common.world.block_entities.get_mut(&key) {
            Some(BlockEntity::Furnace(f)) if i == 0 => {
                f.set_input(&self.common.loop_context, recipes, stack)
            }
            Some(BlockEntity::Furnace(f)) => {
                f.items[i] = stack;
                self.common.world.modified.insert((p[0] >> 4, p[2] >> 4));
                return true;
            }
            Some(BlockEntity::ChiseledBookshelf(_)) if !book => return true,
            Some(BlockEntity::ChiseledBookshelf(b)) => {
                // A book put in, or a stack taken out: `removeItem` only
                // updates the state when it found something.
                let had_book = b.items[i].is_some();
                b.items[i] = stack;
                if had_book || b.items[i].is_some() {
                    shelf_slot = Some(i);
                }
            }
            Some(BlockEntity::Jukebox(_)) => {
                self.jukebox_set_item(p, stack);
                return true;
            }
            Some(BlockEntity::Crafter(c)) => {
                c.disabled[i] = false;
                c.items[i] = stack;
            }
            Some(BlockEntity::Mod(m)) => {
                *m.item_slot_mut(i) = stack;
            }
            Some(entity) => entity.slots_mut()[i] = stack,
            None => return false,
        }
        if let Some(slot) = shelf_slot {
            self.chiseled_bookshelf_update_state(p, slot);
        }
        self.container_changed(key);
        true
    }
    /// `/item replace|fill|override <target> <slots> (with <item> [count] | from <source> <slots>)`.
    pub(in crate::game::server_side) fn cmd_item(&mut self, ctx: &Context) -> R<i32> {
        let mode = ctx
            .literals
            .iter()
            .find(|l| matches!(l.as_str(), "replace" | "fill" | "override"))
            .cloned()
            .unwrap_or_default();
        let targets = self.owner_of(ctx, "target", true)?;
        let Some(Arg::Str(slot_name)) = ctx.get("slots").cloned() else {
            return Ok(0);
        };
        // The items to place.
        let (items, known): (Vec<Option<ItemStack>>, Option<ItemStack>) = if ctx.has("with") {
            let stack = self.item_from_input(ctx)?;
            (vec![stack.clone()], stack)
        } else {
            let sources = self.owner_of(ctx, "source", true)?;
            let Some(Arg::Str(source_slots)) = ctx.get("sourceSlots").cloned() else {
                return Ok(0);
            };
            let mut found = Vec::new();
            for o in &sources {
                for s in self.command_slots(o, &source_slots)? {
                    found.push(self.slot_get(&s));
                }
            }
            if found.is_empty() {
                return Err(match &sources[..] {
                    // ItemCommands.getContainer: a block entity that is no Container
                    // (a sign, a lectern, a beacon) is no source either.
                    [Owner::Block(p)] if self.block_container_size(*p).is_none() => err(
                        "commands.item.source.not_a_container",
                        &[&p[0], &p[1], &p[2]],
                    ),
                    _ if source_slots.starts_with('{') || source_slots.starts_with('[') => {
                        err("commands.item.source.no_such_slot.unnamed", &[])
                    }
                    _ => err("commands.item.source.no_such_slot", &[&source_slots]),
                });
            }
            // ItemCommands.applyModifier on each source stack.
            if ctx.get("modifier").is_some() {
                let modifier = self.item_modifier_arg(ctx, "modifier")?;
                let source = self
                    .server
                    .command_source
                    .clone()
                    .unwrap_or_else(|| self.player_source());
                let view = source.entity.map(|t| self.entity_view_asked(t, &modifier));
                let mut rng = sunrise_core::random::LegacyRandom::new(0);
                found = found
                    .into_iter()
                    .map(|s| {
                        s.and_then(|s| {
                            self.apply_item_modifier(
                                &modifier,
                                Some(s),
                                &mut rng,
                                view.as_ref(),
                                source.pos,
                                0,
                            )
                        })
                    })
                    .collect();
            }
            // A source with nothing in the slot contributes air.
            (found, None)
        };
        let mut tracker = Tracker::default();
        let mut selected = 0;
        for o in &targets {
            let slots = self.command_slot_candidates(o, &slot_name)?;
            if slots.is_empty() {
                if let Owner::Block(p) = o
                    && self.block_container_size(*p).is_none()
                {
                    return Err(err(
                        "commands.item.target.not_a_container",
                        &[&p[0], &p[1], &p[2]],
                    ));
                }
                continue;
            }
            let mut changed = 0;
            let mut i = 0;
            for candidate in &slots {
                if let Slot::Contents(parent, component) = &candidate.slot {
                    changed += self.replace_contents(
                        parent,
                        *component,
                        candidate,
                        &mut selected,
                        &mut contents::Provider::new(&items, &mut i, &mode),
                    );
                    continue;
                }
                if !self.select_slot(candidate, &mut selected, false) {
                    continue;
                }
                let slot = &candidate.slot;
                // `fill` cycles through the items; `replace` stops when they
                // run out; `override` empties the rest.
                let item = match mode.as_str() {
                    "fill" => items.get(i % items.len()).cloned().flatten(),
                    "override" => items.get(i).cloned().flatten(),
                    _ => match items.get(i) {
                        Some(x) => x.clone(),
                        None => break,
                    },
                };
                i += 1;
                // ItemStack.isEmpty: air (or a count of 0) is an empty slot.
                // SlotAccess.set counts every slot it sets, the same item too.
                let item = item.filter(|s| &*s.item != "minecraft:air" && s.count > 0);
                if self.slot_set_by_command(slot, item) {
                    changed += 1;
                }
            }
            let h = self.owner_holder(o);
            tracker.track(&h, changed);
            // EntityItemAccessor: a player whose slots changed gets
            // `containerMenu.broadcastChanges` at once, so InventoryChangeTrigger
            // (and its announcement) comes before the command's message.
            if changed > 0
                && matches!(o, Owner::Entity(Target::Player))
                && self.common.player.is_some()
            {
                self.check_inventory_trigger();
                self.server.cmds.0.broadcast_announced = self.common.adv.announced.len();
            }
        }
        if selected == 0 {
            return Err(
                if slot_name.starts_with('{') || slot_name.starts_with('[') {
                    err("commands.item.target.no_such_slot.unnamed", &[])
                } else {
                    err("commands.item.target.no_such_slot", &[&slot_name])
                },
            );
        }
        let block = matches!(targets.first(), Some(Owner::Block(_)));
        let pos = match targets.first() {
            Some(Owner::Block(p)) => *p,
            _ => [0; 3],
        };
        let zero = match &known {
            Some(k) => err(
                "commands.item.target.failed.known_item",
                // The ItemStack itself is the argument: ItemStack.toString.
                &[&format!("{} {}", k.count, k.item)],
            ),
            None => err("commands.item.target.failed", &[]),
        };
        let name = known.as_ref().map(|k| self.stack_name(k));
        let (message, total) = tracker.message(
            false,
            Some(zero),
            |h, count| {
                let n = Text::literal(count.to_string());
                match (&name, block) {
                    (Some(item), true) => Text::translate(
                        "commands.item.block.replace.success.known_item",
                        &[
                            n,
                            Text::literal(pos[0].to_string()),
                            Text::literal(pos[1].to_string()),
                            Text::literal(pos[2].to_string()),
                            item.clone(),
                        ],
                    ),
                    (None, true) => Text::translate(
                        "commands.item.block.replace.success",
                        &[
                            n,
                            Text::literal(pos[0].to_string()),
                            Text::literal(pos[1].to_string()),
                            Text::literal(pos[2].to_string()),
                        ],
                    ),
                    (Some(item), false) => Text::translate(
                        "commands.item.entity.replace.success.single.known_item",
                        &[n, h.text.clone(), item.clone()],
                    ),
                    (None, false) => Text::translate(
                        "commands.item.entity.replace.success.single",
                        &[n, h.text.clone()],
                    ),
                }
            },
            |c, _| match &name {
                Some(item) => Text::translate(
                    "commands.item.entity.replace.success.multiple.known_item",
                    &[Text::literal(c.to_string()), item.clone()],
                ),
                None => Text::translate(
                    "commands.item.entity.replace.success.multiple",
                    &[Text::literal(c.to_string())],
                ),
            },
        )?;
        self.feedback(message);
        Ok(total)
    }
    /// `/item modify (entity|block) <target> <slot> <modifier>`.
    pub(in crate::game::server_side) fn cmd_item_modify(&mut self, ctx: &Context) -> R<i32> {
        let targets = self.owner_of(ctx, "target", true)?;
        let Some(Arg::Str(slot_name)) = ctx.get("slot").cloned() else {
            return Ok(0);
        };
        let modifier = self.item_modifier_arg(ctx, "modifier")?;
        let source = self
            .server
            .command_source
            .clone()
            .unwrap_or_else(|| self.player_source());
        let view = source.entity.map(|t| self.entity_view_asked(t, &modifier));
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as i64);
        let mut rng = sunrise_core::random::LegacyRandom::new(seed);
        let mut tracker = Tracker::default();
        let mut selected = 0;
        for o in &targets {
            let slots = self.command_slot_candidates(o, &slot_name)?;
            if slots.is_empty() {
                if let Owner::Block(p) = o
                    && self.block_container_size(*p).is_none()
                {
                    return Err(err(
                        "commands.item.target.not_a_container",
                        &[&p[0], &p[1], &p[2]],
                    ));
                }
                continue;
            }
            let mut changed = 0;
            for candidate in &slots {
                if let Slot::Contents(parent, component) = &candidate.slot {
                    changed += self.modify_contents(
                        parent,
                        *component,
                        candidate,
                        &mut selected,
                        |g, current| {
                            g.apply_item_modifier(
                                &modifier,
                                Some(current),
                                &mut rng,
                                view.as_ref(),
                                source.pos,
                                0,
                            )
                            .map(|mut s| {
                                s.count = s.count.min(g.stack_max_size(&s));
                                s
                            })
                        },
                    );
                    continue;
                }
                if !self.select_slot(candidate, &mut selected, true) {
                    continue;
                }
                let slot = &candidate.slot;
                // SlotSelector.NON_EMPTY_SLOTS
                let Some(current) = self.slot_get(slot) else {
                    continue;
                };
                let new = self.apply_item_modifier(
                    &modifier,
                    Some(current.clone()),
                    &mut rng,
                    view.as_ref(),
                    source.pos,
                    0,
                );
                // CountingModifier: every slot set counts, changed or not;
                // ItemStack.limitSize to the (new) max stack size.
                let new = new.map(|mut s| {
                    let max = self.stack_max_size(&s);
                    s.count = s.count.min(max);
                    s
                });
                if self.slot_set_by_command(slot, new) {
                    changed += 1;
                }
            }
            let h = self.owner_holder(o);
            tracker.track(&h, changed);
        }
        if selected == 0 {
            return Err(
                if slot_name.starts_with('{') || slot_name.starts_with('[') {
                    err("commands.item.target.no_such_slot.unnamed", &[])
                } else {
                    err("commands.item.target.no_such_slot", &[&slot_name])
                },
            );
        }
        let pos = match targets.first() {
            Some(Owner::Block(p)) => Some(*p),
            _ => None,
        };
        let (message, total) = tracker.message(
            false,
            Some(err("commands.item.target.failed", &[])),
            |h, count| {
                let n = Text::literal(count.to_string());
                match pos {
                    Some(p) => Text::translate(
                        "commands.item.block.modify.success",
                        &[
                            n,
                            Text::literal(p[0].to_string()),
                            Text::literal(p[1].to_string()),
                            Text::literal(p[2].to_string()),
                        ],
                    ),
                    None => Text::translate(
                        "commands.item.entity.modify.success.single",
                        &[n, h.text.clone()],
                    ),
                }
            },
            |c, _| {
                Text::translate(
                    "commands.item.entity.modify.success.multiple",
                    &[Text::literal(c.to_string())],
                )
            },
        )?;
        self.feedback(message);
        Ok(total)
    }
    /// `ResourceOrIdArgument.getItemModifier`: an inline modifier (SNBT) or
    /// a registered one.
    pub(in crate::game::server_side) fn item_modifier_arg(
        &mut self,
        ctx: &Context,
        name: &str,
    ) -> R<serde_json::Value> {
        let raw = match ctx.get(name) {
            Some(Arg::Resource(s) | Arg::Str(s)) => s.clone(),
            _ => return Err(err("command.failed", &[])),
        };
        if raw.starts_with('{') || raw.starts_with('[') {
            return Self::parse_inline_item_modifier(&raw);
        }
        let id = if raw.contains(':') {
            raw
        } else {
            format!("minecraft:{raw}")
        };
        self.server
            .fns
            .0
            .store
            .item_modifiers
            .get(&id)
            .cloned()
            .ok_or_else(|| {
                err(
                    "argument.resource_or_id.no_such_element",
                    &[&id, &"minecraft:item_modifier"],
                )
            })
    }
    pub fn slot_get(&mut self, s: &Slot) -> Option<ItemStack> {
        match s {
            Slot::Contents(..) => None,
            Slot::ContentItem(parent, component, index) => {
                let stack = self.slot_get(parent)?;
                component
                    .items(self.res.items(), &stack)
                    .get(*index)
                    .cloned()
                    .flatten()
            }
            Slot::Guest(conn, slot) => self
                .for_player(Target::Guest(*conn), |g| g.slot_get(slot))
                .flatten(),
            Slot::Inv(i) => self.common.inventory.get(*i).cloned().flatten(),
            Slot::MainHand => self
                .common
                .inventory
                .get(self.common.selected)
                .cloned()
                .flatten(),
            Slot::Armor(i) => self.common.armor.get(*i).cloned().flatten(),
            Slot::Offhand => self.common.offhand.clone(),
            Slot::Equipment(i) => self.common.equipment.get(*i).cloned(),
            Slot::Ender(i) => self.server.ender_items.get(*i).cloned().flatten(),
            Slot::Mob(id, key) => {
                let m = self.common.mobs.iter().find(|m| m.id == *id)?;
                m.equipment_stacks()[mob_slot(key)?].clone()
            }
            Slot::Stand(id, i) => self
                .common
                .armor_stands
                .iter()
                .find(|a| a.id == *id)?
                .equipment
                .get(*i)
                .cloned(),
            Slot::Item(id) => self
                .common
                .dropped
                .iter()
                .find(|e| e.id == *id)
                .map(|e| e.stack.clone())
                .filter(|s| &*s.item != "minecraft:air" && s.count > 0),
            Slot::Dragon(id, i) => self
                .common
                .dragon
                .as_ref()
                .filter(|d| d.id == *id)?
                .equipment
                .get(*i)
                .cloned(),
            Slot::Frame(id) => self
                .common
                .item_frames
                .iter()
                .find(|f| f.id == *id)?
                .item
                .clone(),
            Slot::Cart(id, i) => self
                .edit_cart_slots(*id, |slots| slots.get(*i).cloned().flatten())
                .flatten(),
            Slot::Block(p, i) => self.loot_inventory_item((p[0], p[1], p[2]), *i),
        }
    }
    /// The shared slot writer, with stand events suppressed for transactional replay.
    pub(in crate::game::server_side) fn slot_set_with_effects(
        &mut self,
        s: &Slot,
        stack: Option<ItemStack>,
        effects: bool,
    ) -> bool {
        if matches!(
            s,
            Slot::Inv(_) | Slot::MainHand | Slot::Armor(_) | Slot::Offhand | Slot::Equipment(_)
        ) {
            let uuid = self.native_acting_id();
            self.journal_player(uuid);
        }
        match s {
            Slot::Contents(..) => false,
            Slot::ContentItem(parent, component, index) => {
                let Some(mut outer) = self.slot_get(parent) else {
                    return false;
                };
                let mut slots = component.items(self.res.items(), &outer);
                let Some(slot) = slots.get_mut(*index) else {
                    return false;
                };
                *slot = stack;
                component.store(self.res.items(), &mut outer, slots);
                self.slot_set(parent, Some(outer))
            }
            Slot::Guest(conn, slot) => self
                .for_player(Target::Guest(*conn), |g| g.slot_set(slot, stack))
                .unwrap_or(false),
            Slot::Inv(i) if *i < self.common.inventory.len() => {
                self.stop_use_replaced(*i, stack.as_ref());
                self.common.inventory[*i] = stack;
                true
            }
            Slot::Inv(_) => false,
            // LivingEntity's equipment SlotAccess: setItemSlot, so onEquipItem.
            Slot::MainHand if self.common.selected < self.common.inventory.len() => {
                self.stop_use_replaced(self.common.selected, stack.as_ref());
                let old = std::mem::replace(
                    &mut self.common.inventory[self.common.selected],
                    stack.clone(),
                );
                self.player_on_equip("mainhand", old.as_ref(), stack.as_ref());
                true
            }
            Slot::MainHand => false,
            Slot::Armor(i) => match self.common.armor.get_mut(*i) {
                Some(slot) => {
                    let old = std::mem::replace(slot, stack.clone());
                    let name = ["head", "chest", "legs", "feet"][*i];
                    self.player_on_equip(name, old.as_ref(), stack.as_ref());
                    if self.common.transactions.is_empty() {
                        self.apply_equipment();
                    }
                    true
                }
                None => false,
            },
            Slot::Offhand => {
                self.stop_use_replaced(crate::game::shield::OFFHAND_SLOT, stack.as_ref());
                let old = std::mem::replace(&mut self.common.offhand, stack.clone());
                self.player_on_equip("offhand", old.as_ref(), stack.as_ref());
                true
            }
            // setItemSlot, so onEquipItem: the equip sound, but no game event
            // for body and saddle.
            Slot::Equipment(i) => {
                let old = self.common.equipment[*i].clone();
                self.common.equipment.set(*i, stack.clone());
                self.player_on_equip(
                    sunrise_items::equipment::SLOT_NAMES[*i],
                    old.as_ref(),
                    stack.as_ref(),
                );
                true
            }
            Slot::Ender(i) => match self.server.ender_items.get_mut(*i) {
                Some(slot) => {
                    *slot = stack.clone();
                    // An open ender chest menu shows a copy, which closing copies back
                    // over `ender_items`: the write goes into the copy as well.
                    if let Some(key) = self.server.ender_open
                        && let Some(BlockEntity::Chest(items)) =
                            self.common.world.block_entities.get_mut(&key)
                        && let Some(open) = items.get_mut(*i)
                    {
                        *open = stack;
                    }
                    true
                }
                None => false,
            },
            Slot::Mob(id, key) => {
                let Some(index) = self.common.mobs.iter().position(|m| m.id == *id) else {
                    return false;
                };
                let Some(slot) = mob_slot(key) else {
                    return false;
                };
                let m = &self.common.mobs[index];
                let old = m.equipment_stacks()[slot].clone();
                let first_tick = m.first_tick;
                let at = [m.position.x, m.position.y, m.position.z];
                let same = match (&old, &stack) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.same_item_same_components(b),
                    _ => false,
                };
                let equippable = stack.as_ref().is_some_and(|s| {
                    let data = s.data();
                    !data.is_some_and(|d| d.other("!minecraft:equippable").is_some())
                        && (data.is_some_and(|d| d.other("minecraft:equippable").is_some())
                            || self
                                .items()
                                .get(&s.item)
                                .is_some_and(|i| i.equip_slot.is_some()))
                });
                let changed = self.common.mobs[index].set_equipment_stack(slot, stack);
                if changed && !first_tick && !same && self.common.is_logical_server() {
                    self.mob_equip_event(*id, at, equippable);
                }
                changed
            }
            Slot::Stand(id, i) => {
                // LivingEntity.onEquipItem: all stand slots emit; the new
                // stack's EQUIPPABLE presence chooses equip versus unequip.
                let equippable = stack.as_ref().is_some_and(|s| {
                    let data = s.data();
                    !data.is_some_and(|d| d.other("!minecraft:equippable").is_some())
                        && (data.is_some_and(|d| d.other("minecraft:equippable").is_some())
                            || self
                                .items()
                                .get(&s.item)
                                .is_some_and(|i| i.equip_slot.is_some()))
                });
                let Some(a) = self.common.armor_stands.iter_mut().find(|a| a.id == *id) else {
                    return false;
                };
                let same = match (a.equipment.get(*i), stack.as_ref()) {
                    (None, None) => true,
                    (Some(old), Some(new)) => old.same_item_same_components(new),
                    _ => false,
                };
                let emit = !a.first_tick && !same;
                let at = a.position;
                if !a.equipment.set(*i, stack) {
                    return false;
                }
                if effects && emit && self.common.is_logical_server() {
                    self.post_game_event(
                        if equippable {
                            "minecraft:equip"
                        } else {
                            "minecraft:unequip"
                        },
                        [at.x, at.y, at.z],
                        sunrise_level::sculk::Context {
                            source: Some(sunrise_sim::mobs::Who::Mob(*id).into()),
                            living: true,
                            ..Default::default()
                        },
                    );
                }
                true
            }
            Slot::Dragon(id, i) => self
                .common
                .dragon
                .as_mut()
                .filter(|d| d.id == *id)
                .is_some_and(|d| d.equipment.set(*i, stack)),
            Slot::Item(id) => match self.common.dropped.iter_mut().find(|e| e.id == *id) {
                Some(entity) => {
                    // `ItemEntity.setItem`: the stack is the entity's.
                    match stack.filter(|s| &*s.item != "minecraft:air" && s.count > 0) {
                        Some(stack) => entity.stack = stack,
                        None => entity.stack = ItemStack::new("minecraft:air", 0),
                    }
                    true
                }
                None => false,
            },
            Slot::Frame(id) => match self.common.item_frames.iter().position(|f| f.id == *id) {
                Some(i) => {
                    self.set_frame_item(i, stack);
                    true
                }
                None => false,
            },
            Slot::Cart(id, i) => self
                .edit_cart_slots(*id, |slots| match slots.get_mut(*i) {
                    Some(slot) => {
                        *slot = stack;
                        true
                    }
                    None => false,
                })
                .unwrap_or(false),
            Slot::Block(p, i) => self.block_slot_set(*p, *i, stack),
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::player_equip::PlayerEquipGameExt1 as _;
