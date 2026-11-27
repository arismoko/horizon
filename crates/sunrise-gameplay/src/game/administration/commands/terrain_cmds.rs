//! `/forceload`, `/fillbiome`, `/worldborder` and `/loot`.
use super::{Arg, Context, R, err};
use crate::game::world_border::BorderTarget;
#[allow(unused_imports)]
use sunrise_game_data::text::Text;
#[allow(unused_imports)]
use sunrise_items::items::ItemStack;
#[allow(unused_imports)]
use sunrise_sim::scoreboard_cmd::{Holder, Tracker};

// `WorldBorderCommand.setCenter` / `setSize` bounds (not the extent's bounds).
const MAX_CENTER: f64 = 29_999_984.0;
const MAX_SIZE: f64 = 59_999_968.0;

pub(in crate::game) fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}
pub(in crate::game) fn chunk_text(x: i32, z: i32) -> Text {
    Text::literal(format!("[{x}, {z}]"))
}
pub(in crate::game) fn holder(text: Text) -> Holder {
    Holder {
        name: String::new(),
        text,
        display: None,
    }
}
pub(in crate::game) fn ticks_to_seconds(ticks: i64) -> String {
    super::percent_f(ticks as f64 / 20.0, 2)
}
/// `WorldBorderCommand.ERROR_TOO_BIG`'s argument: vanilla hands the message the
/// double `5.9999968E7` (the value `setSize` compares against too), and a `%s`
/// substitution prints a double with `Double.toString`.
pub(in crate::game) fn size_limit_text() -> String {
    sunrise_core::java_number::double_to_string(MAX_SIZE)
}

impl crate::game::ServerGame<'_> {
    // ---- /forceload ----------------------------------------------------------

    pub(in crate::game::server_side) fn column_arg(
        &self,
        ctx: &Context,
        name: &str,
    ) -> R<[i32; 2]> {
        let c = self
            .coords(ctx, name)
            .ok_or_else(|| err("argument.pos2d.incomplete", &[]))?;
        let p = self.resolve_coords(&c);
        Ok([p[0].floor() as i32, p[2].floor() as i32])
    }
    /// `ForceLoadCommand.changeForceLoad`: update and load each newly forced column in order.
    pub(in crate::game::server_side) fn cmd_forceload(
        &mut self,
        id: &str,
        ctx: &Context,
    ) -> R<i32> {
        let dim = Text::literal(self.common.dim.id());
        match id {
            "forceload.query" if ctx.get("pos").is_none() => {
                let count = self.server.streaming.commanded().len();
                if count == 0 {
                    self.command_failure(Text::translate("commands.forceload.added.none", &[dim]));
                    return Ok(0);
                }
                let list = self
                    .server
                    .streaming
                    .commanded_in_order()
                    .map(|(x, z)| format!("[{x}, {z}]"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let message = if count == 1 {
                    Text::translate(
                        "commands.forceload.list.single",
                        &[dim, Text::literal(list)],
                    )
                } else {
                    Text::translate(
                        "commands.forceload.list.multiple",
                        &[num(count), dim, Text::literal(list)],
                    )
                };
                self.feedback(message);
                Ok(count as i32)
            }
            "forceload.query" => {
                let [x, z] = self.column_arg(ctx, "pos")?;
                let key = (x >> 4, z >> 4);
                if self.server.streaming.commanded().contains(&key) {
                    self.feedback(Text::translate(
                        "commands.forceload.query.success",
                        &[chunk_text(key.0, key.1), dim],
                    ));
                    Ok(1)
                } else {
                    Err(err(
                        "commands.forceload.query.failure",
                        &[&chunk_text(key.0, key.1).plain(), &dim.plain()],
                    ))
                }
            }
            "forceload.remove.all" => {
                self.server.streaming.clear_forced();
                self.feedback(Text::translate("commands.forceload.removed.all", &[dim]));
                Ok(0)
            }
            _ => {
                let add = id == "forceload.add";
                let from = self.column_arg(ctx, "from")?;
                let to = if ctx.get("to").is_some() {
                    self.column_arg(ctx, "to")?
                } else {
                    from
                };
                let (min_x, max_x) = (from[0].min(to[0]), from[0].max(to[0]));
                let (min_z, max_z) = (from[1].min(to[1]), from[1].max(to[1]));
                if min_x < -30_000_000
                    || min_z < -30_000_000
                    || max_x >= 30_000_000
                    || max_z >= 30_000_000
                {
                    return Err(err("argument.pos.outofworld", &[]));
                }
                let (cx0, cx1, cz0, cz1) = (min_x >> 4, max_x >> 4, min_z >> 4, max_z >> 4);
                let count = i64::from(cx1 - cx0 + 1) * i64::from(cz1 - cz0 + 1);
                if count > 256 {
                    return Err(err("commands.forceload.toobig", &[&256, &count]));
                }
                let mut tracker = Tracker::default();
                for x in cx0..=cx1 {
                    for z in cz0..=cz1 {
                        let changed = self.server.streaming.set_forced_loaded(
                            &mut self.common.world,
                            x,
                            z,
                            add,
                        );
                        tracker.track(&holder(chunk_text(x, z)), i32::from(changed));
                    }
                }
                let (lo, hi) = (chunk_text(cx0, cz0), chunk_text(cx1, cz1));
                let verb = if add { "added" } else { "removed" };
                let (message, total) = tracker.message(
                    false,
                    Some(err(&format!("commands.forceload.{verb}.failure"), &[])),
                    |h, _| {
                        Text::translate(
                            &format!("commands.forceload.{verb}.single"),
                            &[h.text.clone(), dim.clone()],
                        )
                    },
                    |c, _| {
                        Text::translate(
                            &format!("commands.forceload.{verb}.multiple"),
                            &[num(c), dim.clone(), lo.clone(), hi.clone()],
                        )
                    },
                )?;
                self.feedback(message);
                Ok(total)
            }
        }
    }

    // ---- /fillbiome ----------------------------------------------------------------

    pub(in crate::game::server_side) fn cmd_fillbiome(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(biome)) = ctx.get("biome").cloned() else {
            return Ok(0);
        };
        // BlockPosArgument.getLoadedBlockPos on both corners first.
        let (a, b) = (
            self.block_pos_arg(ctx, "from")?,
            self.block_pos_arg(ctx, "to")?,
        );
        // Quantize to biome cells (`QuartPos`), then work with the box.
        let q = |v: i32| (v >> 2) << 2;
        let lo = [
            q(a[0]).min(q(b[0])),
            q(a[1]).min(q(b[1])),
            q(a[2]).min(q(b[2])),
        ];
        let hi = [
            q(a[0]).max(q(b[0])),
            q(a[1]).max(q(b[1])),
            q(a[2]).max(q(b[2])),
        ];
        let volume = i64::from(hi[0] - lo[0] + 1)
            * i64::from(hi[1] - lo[1] + 1)
            * i64::from(hi[2] - lo[2] + 1);
        let limit = self
            .common
            .ui
            .rules
            .int_at(sunrise_game_data::gamerules::vanilla::MAX_BLOCK_MODIFICATIONS);
        if volume > i64::from(limit) {
            return Err(err("commands.fillbiome.toobig", &[&limit, &volume]));
        }
        let Some(target) = world_biomes!(self)
            .list
            .iter()
            .position(|b| b.name == biome)
        else {
            return Err(err(
                "argument.resource.not_found",
                &[&biome, &"minecraft:worldgen/biome"],
            ));
        };
        let filter = match ctx.get("filter") {
            Some(Arg::Resource(f)) => Some(f.clone()),
            _ => None,
        };
        for cz in (lo[2] >> 4)..=(hi[2] >> 4) {
            for cx in (lo[0] >> 4)..=(hi[0] >> 4) {
                if !self.common.world.chunks.contains_key(&(cx, cz)) {
                    return Err(err("argument.pos.unloaded", &[]));
                }
            }
        }
        let mut changed = 0;
        let mut touched = std::collections::HashSet::new();
        for z in (lo[2]..=hi[2]).step_by(4) {
            for y in (lo[1]..=hi[1]).step_by(4) {
                for x in (lo[0]..=hi[0]).step_by(4) {
                    let key = (x >> 4, z >> 4);
                    let Some(chunk) = self.common.world.chunks.get(&key) else {
                        continue;
                    };
                    let current = chunk.biome(x, y, z);
                    let current_name = world_biomes!(self).name(current);
                    if let Some(f) = &filter
                        && !self.biome_matches(f, current_name)
                    {
                        continue;
                    }
                    if usize::from(current) == target {
                        continue;
                    }
                    let qy = ((y - self.common.world.bottom_y()) >> 2)
                        .clamp(0, self.common.world.layout().height() / 4 - 1);
                    if let Some(chunk) = self.common.world.chunks.get_mut(&key) {
                        let chunk = std::sync::Arc::make_mut(chunk);
                        chunk.set_biome_cell((x & 15) >> 2, qy, (z & 15) >> 2, target as u16);
                    }
                    touched.insert(key);
                    changed += 1;
                }
            }
        }
        if changed == 0 {
            return Err(err("commands.fillbiome.no_changes", &[]));
        }
        let columns: Vec<(i32, i32)> = ((lo[2] >> 4)..=(hi[2] >> 4))
            .flat_map(|cz| ((lo[0] >> 4)..=(hi[0] >> 4)).map(move |cx| (cx, cz)))
            .filter(|key| touched.contains(key))
            .collect();
        self.commit_biome_columns(&touched, &columns);
        self.feedback(Text::translate(
            "commands.fillbiome.success.count",
            &[
                num(changed),
                num(lo[0]),
                num(lo[1]),
                num(lo[2]),
                num(hi[0]),
                num(hi[1]),
                num(hi[2]),
            ],
        ));
        Ok(changed)
    }

    /// The tail of a biome change: the columns in `touched` are saved and re-meshed, and the
    /// players who see `columns` are sent their new biomes (`ChunkMap.resendBiomesForChunks`), in
    /// the order the columns were gathered (z, then x). Shared by `/fillbiome` and
    /// `level_set_biome`.
    pub(in crate::game) fn commit_biome_columns(
        &mut self,
        touched: &std::collections::HashSet<(i32, i32)>,
        columns: &[(i32, i32)],
    ) {
        for key in touched {
            self.common.world.modified.insert(*key);
            for sy in 0..self.common.world.layout().sections() as i32 {
                self.common.world.dirty_sections.insert((key.0, sy, key.1));
            }
        }
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.resend_biomes(&self.common.world, columns);
        }
    }
    /// Set the biome cell holding `pos` (a 4x4x4 cell, `QuartPos`) to the biome named `biome`
    /// (`minecraft:plains`), as `/fillbiome` does for one cell: the chunk is saved, re-meshed and
    /// resent. False for an unknown biome, an unloaded or out-of-height position, or a cell that
    /// already has it.
    /// Tentative reads see the new cells inside a transaction. Abort restores the cells
    /// without publishing. Saving, remeshing and resending wait for the outermost commit,
    /// once per column. Outside a transaction they run immediately.
    pub(in crate::game) fn set_biome_at(&mut self, pos: [i32; 3], biome: &str) -> bool {
        let Some(target) = world_biomes!(self)
            .list
            .iter()
            .position(|b| b.name == biome)
        else {
            return false;
        };
        let key = (pos[0] >> 4, pos[2] >> 4);
        let Some(chunk) = self.common.world.chunks.get(&key) else {
            return false;
        };
        if !self.common.world.shape.contains_y(pos[1])
            || usize::from(chunk.biome(pos[0], pos[1], pos[2])) == target
        {
            return false;
        }
        let old = chunk.biome(pos[0], pos[1], pos[2]);
        self.journal_biome(pos, old);
        let qy = ((pos[1] - self.common.world.bottom_y()) >> 2)
            .clamp(0, self.common.world.layout().height() / 4 - 1);
        if let Some(chunk) = self.common.world.chunks.get_mut(&key) {
            let chunk = std::sync::Arc::make_mut(chunk);
            chunk.set_biome_cell((pos[0] & 15) >> 2, qy, (pos[2] & 15) >> 2, target as u16);
        }
        self.after_commit(sunrise_gameplay_common::game::transactions::After::Biome(
            key,
        ));
        true
    }

    // ---- /worldborder -----------------------------------------------------------------------

    pub(in crate::game::server_side) fn cmd_worldborder(
        &mut self,
        id: &str,
        ctx: &Context,
    ) -> R<i32> {
        let float2 = |v: f64| super::percent_f(v, 2);
        let target = BorderTarget::InPlace;
        let failed = |_| err("command.failed", &[]);
        let double = match ctx.get("distance") {
            Some(Arg::Double(d)) => *d,
            _ => 0.0,
        };
        let time = match ctx.get("time") {
            Some(Arg::Int(t)) => Some(i64::from(*t)),
            _ => None,
        };
        match id {
            "worldborder.get" => {
                let size = self.common.border.size();
                self.feedback(Text::translate(
                    "commands.worldborder.get",
                    &[Text::literal(super::percent_f(size, 0))],
                ));
                Ok((size + 0.5).floor() as i32)
            }
            "worldborder.set" | "worldborder.add" => {
                let current = self.common.border.size();
                // `add` measures from the size the border has now and, given a
                // time, keeps a change in flight going for its remaining ticks
                // plus the argument (`getLerpTime() + time`); `set` replaces it.
                let (distance, ticks) = match (id, time) {
                    ("worldborder.add", Some(t)) => (
                        current + double,
                        self.common.border.remaining_ticks().wrapping_add(t),
                    ),
                    ("worldborder.add", None) => (current + double, 0),
                    (_, t) => (double, t.unwrap_or(0)),
                };
                if current == distance {
                    return Err(err("commands.worldborder.set.failed.nochange", &[]));
                }
                if distance < 1.0 {
                    return Err(err("commands.worldborder.set.failed.small", &[]));
                }
                if distance > MAX_SIZE {
                    return Err(err(
                        "commands.worldborder.set.failed.big",
                        &[&size_limit_text()],
                    ));
                }
                self.set_border_size(target, distance, ticks)
                    .map_err(failed)?;
                let shown = Text::literal(super::percent_f(distance, 1));
                if ticks > 0 {
                    let key = if distance > current {
                        "commands.worldborder.set.grow"
                    } else {
                        "commands.worldborder.set.shrink"
                    };
                    self.feedback(Text::translate(
                        key,
                        &[shown, Text::literal(ticks_to_seconds(ticks))],
                    ));
                } else {
                    self.feedback(Text::translate(
                        "commands.worldborder.set.immediate",
                        &[shown],
                    ));
                }
                Ok((distance - current) as i32)
            }
            "worldborder.center" => {
                let p = self
                    .position_arg(ctx, "pos")
                    .unwrap_or_else(|| self.source_position());
                let (x, z) = (p[0], p[2]);
                if self.common.border.center() == [x, z] {
                    return Err(err("commands.worldborder.center.failed", &[]));
                }
                if x.abs() > MAX_CENTER || z.abs() > MAX_CENTER {
                    return Err(err(
                        "commands.worldborder.set.failed.far",
                        &[&"2.9999984E7"],
                    ));
                }
                self.set_border_center(target, [x, z]).map_err(failed)?;
                self.feedback(Text::translate(
                    "commands.worldborder.center.success",
                    &[Text::literal(float2(x)), Text::literal(float2(z))],
                ));
                Ok(0)
            }
            "worldborder.damage.amount" | "worldborder.damage.buffer" => {
                let amount = id.ends_with("amount");
                let value = match ctx.get(if amount { "damagePerBlock" } else { "distance" }) {
                    Some(Arg::Float(v)) => f64::from(*v),
                    _ => 0.0,
                };
                let current = if amount {
                    self.common.border.damage_per_block
                } else {
                    self.common.border.safe_zone
                };
                if current == value {
                    return Err(err(
                        if amount {
                            "commands.worldborder.damage.amount.failed"
                        } else {
                            "commands.worldborder.damage.buffer.failed"
                        },
                        &[],
                    ));
                }
                let mut border = self.border_mut(target).map_err(failed)?;
                if amount {
                    Self::set_border_damage_per_block(&mut border, value);
                } else {
                    Self::set_border_safe_zone(&mut border, value);
                }
                self.feedback(Text::translate(
                    if amount {
                        "commands.worldborder.damage.amount.success"
                    } else {
                        "commands.worldborder.damage.buffer.success"
                    },
                    &[Text::literal(float2(value))],
                ));
                Ok(value as i32)
            }
            "worldborder.warning.distance" => {
                let d = match ctx.get("distance") {
                    Some(Arg::Int(v)) => *v,
                    _ => 0,
                };
                if self.common.border.warning_distance == d {
                    return Err(err("commands.worldborder.warning.distance.failed", &[]));
                }
                Self::set_border_warning_blocks(&mut self.border_mut(target).map_err(failed)?, d);
                self.feedback(Text::translate(
                    "commands.worldborder.warning.distance.success",
                    &[num(d)],
                ));
                Ok(d)
            }
            _ => {
                let ticks = match ctx.get("time") {
                    Some(Arg::Int(v)) => *v,
                    _ => 0,
                };
                if self.common.border.warning_time == ticks {
                    return Err(err("commands.worldborder.warning.time.failed", &[]));
                }
                Self::set_border_warning_time(&mut self.border_mut(target).map_err(failed)?, ticks);
                self.feedback(Text::translate(
                    "commands.worldborder.warning.time.success",
                    &[Text::literal(ticks_to_seconds(i64::from(ticks)))],
                ));
                Ok(ticks)
            }
        }
    }

    // ---- /loot ---------------------------------------------------------------------------------
}

#[cfg(test)]
pub(in crate::game) mod tests {
    /// `WorldBorderCommand.ERROR_TOO_BIG` is built with the double
    /// `5.9999968E7`, the same value `setSize` compares `distance` against, and
    /// the `%s` substitution of a translatable renders a double with
    /// `Double.toString`: the message names `5.9999968E7`, not a rounded
    /// `5.999997E7`.
    #[test]
    fn the_too_big_message_names_the_limit_as_vanilla_prints_it() {
        assert_eq!(super::MAX_SIZE, 5.9999968E7);
        assert_eq!(super::size_limit_text(), "5.9999968E7");
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod loot_mine_component_tests {
    use super::*;
    use sunrise_level::containers::BlockEntity;

    pub fn slot(game: &crate::game::ServerGame<'_>, index: usize) -> ItemStack {
        let Some(BlockEntity::Chest(items)) = game.world.block_entities.get(&(12, 200, 4)) else {
            panic!("actual command-created chest required");
        };
        items[index]
            .clone()
            .expect("actual inventory slot must be occupied")
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod loot_mine_location_tests {
    pub const TABLE: &str = r###"{"type":"minecraft:block","pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:emerald","modifier":{"type":"minecraft:set_count","count":2}}]},{"rolls":1,"condition":{"type":"minecraft:location_check","offsetX":1,"predicate":{"block":{"blocks":"minecraft:oak_log","state":{"axis":"x"}}}},"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]}"###;
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
