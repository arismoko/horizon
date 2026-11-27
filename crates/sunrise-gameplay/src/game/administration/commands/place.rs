//! `/place template` (PlaceCommand): a saved structure template placed with
//! rotation, mirroring and integrity.
use super::{Arg, Context, R, err};
use crate::game::structure_blocks::{load_settings, template_size};
use std::sync::OnceLock;
use sunrise_core::position::BlockPos;
use sunrise_game_data::text::Text;
use {
    sunrise_core::{
        nbt,
        nbt::{Limits, Tag},
    },
    sunrise_world::{
        compression::Compression,
        worldgen::structure::{Mirror, Rotation, StateTransforms},
    },
};

pub(in crate::game) fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}

pub fn read_template_data(
    saved: &std::path::Path,
    prepared: &std::path::Path,
    resource: Option<sunrise_assets::resource_source::Resource>,
) -> Option<Tag> {
    let saved = std::iter::once_with(|| std::fs::read(saved).ok());
    let resources = std::iter::once_with(|| match resource {
        Some(resource) => resource.read().ok(),
        None => std::fs::read(prepared).ok(),
    });
    for bytes in saved.chain(resources).flatten() {
        let Ok(raw) = Compression::Gzip.decode(&bytes, 1 << 28) else {
            continue;
        };
        if let Ok((named, _)) = nbt::read_named(&raw, Limits::templates()) {
            return Some(named.tag);
        }
    }
    None
}

impl crate::game::ServerGame<'_> {
    pub fn state_transforms(&self) -> Option<&'static StateTransforms> {
        static TRANSFORMS: OnceLock<Option<StateTransforms>> = OnceLock::new();
        TRANSFORMS
            .get_or_init(|| {
                let json = std::fs::read_to_string(
                    self.common
                        .options
                        .resources
                        .join("worldgen/blocks/structure-blocks.json"),
                )
                .ok()?;
                StateTransforms::from_json(&json, self.common.pack.assets.registry.len()).ok()
            })
            .as_ref()
    }
    /// `BlockPosArgument.getLoadedBlockPos(pos)`, or the source's block.
    pub(in crate::game::server_side) fn place_pos(&self, ctx: &Context, name: &str) -> R<[i32; 3]> {
        if ctx.get(name).is_some() {
            return self.block_pos_arg(ctx, name);
        }
        Ok(self.source_position().map(|v| v.floor() as i32))
    }
    /// `PlaceCommand.placeFeature`, `placeJigsaw` and `placeStructure`.
    pub(in crate::game::server_side) fn cmd_place_generated(
        &mut self,
        id: &str,
        ctx: &Context,
    ) -> R<i32> {
        match id {
            "place.feature" => {
                let raw = match ctx.get("feature") {
                    Some(Arg::Resource(s) | Arg::Str(s)) => s.clone(),
                    _ => return Ok(0),
                };
                let pos = self.place_pos(ctx, "pos")?;
                let (cx, cz) = (pos[0] >> 4, pos[2] >> 4);
                self.place_check_loaded([cx - 1, cz - 1], [cx + 1, cz + 1])?;
                // ResourceOrHolderArgument: an inline feature value goes
                // through the same codec and `Feature.place`; a value that
                // does not decode keeps the plain failure.
                if raw.starts_with('{') {
                    let (tag, _) = sunrise_core::nbt::parse_snbt(&raw)
                        .map_err(|_| err("command.failed", &[]))?;
                    let json = sunrise_game_data::scoreboard::tag_to_json(&tag).to_string();
                    let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
                    let placed = self
                        .with_level(|b, level, ticks| {
                            b.place_inline_feature(level, ticks, &json, at)
                        })
                        .ok_or_else(|| err("command.failed", &[]))?;
                    self.server_process_level_events()
                        .map_err(|_| err("command.failed", &[]))?;
                    if !placed {
                        return Err(err("commands.place.feature.failed", &[]));
                    }
                    self.feedback(Text::translate(
                        "commands.place.feature.success.inline",
                        &[num(pos[0]), num(pos[1]), num(pos[2])],
                    ));
                    return Ok(1);
                }
                let name = if raw.contains(':') {
                    raw
                } else {
                    format!("minecraft:{raw}")
                };
                let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
                let placed = self
                    .with_level(|b, level, ticks| b.place_feature(level, ticks, &name, at))
                    .ok_or_else(|| err("command.failed", &[]))?;
                self.server_process_level_events()
                    .map_err(|_| err("command.failed", &[]))?;
                if !placed {
                    return Err(err("commands.place.feature.failed", &[]));
                }
                self.feedback(Text::translate(
                    "commands.place.feature.success",
                    &[Text::literal(name), num(pos[0]), num(pos[1]), num(pos[2])],
                ));
                Ok(1)
            }
            "place.jigsaw" => {
                let Some(Arg::Resource(pool)) = ctx.get("pool").cloned() else {
                    return Ok(0);
                };
                if !self
                    .common
                    .ui
                    .registries
                    .ids("minecraft:worldgen/template_pool")
                    .contains(&pool)
                {
                    return Err(err("commands.place.jigsaw.invalid", &[&pool]));
                }
                let target = match ctx.get("target") {
                    Some(Arg::Resource(s) | Arg::Str(s)) => s.clone(),
                    _ => return Ok(0),
                };
                let max_depth = match ctx.get("max_depth") {
                    Some(Arg::Int(n)) => *n,
                    _ => return Ok(0),
                };
                let pos = self.place_pos(ctx, "position")?;
                self.place_check_loaded([pos[0] >> 4, pos[2] >> 4], [pos[0] >> 4, pos[2] >> 4])?;
                let terrain = self
                    .common
                    .end
                    .locator()
                    .ok_or_else(|| err("command.failed", &[]))?;
                let query = sunrise_world::worldgen::generator::JigsawQuery {
                    pool: &pool,
                    target: &target,
                    max_depth,
                    position: pos,
                };
                let (placed, entities) = self.with_level(|b, level, ticks| {
                    b.place_jigsaw(level, ticks, pos, false, || terrain.jigsaw_pieces(&query))
                });
                for entity in entities {
                    self.spawn_entity_from_nbt(Tag::Compound(entity));
                }
                self.server_process_level_events()
                    .map_err(|_| err("command.failed", &[]))?;
                if !placed {
                    return Err(err("commands.place.jigsaw.failed", &[]));
                }
                self.feedback(Text::translate(
                    "commands.place.jigsaw.success",
                    &[num(pos[0]), num(pos[1]), num(pos[2])],
                ));
                Ok(1)
            }
            _ => {
                let Some(Arg::Resource(structure)) = ctx.get("structure").cloned() else {
                    return Ok(0);
                };
                if !self
                    .common
                    .ui
                    .registries
                    .ids("minecraft:worldgen/structure")
                    .contains(&structure)
                {
                    return Err(err("commands.place.structure.invalid", &[&structure]));
                }
                let pos = self.place_pos(ctx, "pos")?;
                // PlaceCommand.placeStructure: Structure.generate for the
                // position's chunk with its `biome -> true` predicate, then
                // placeInChunk for every chunk of the start's box.
                let terrain = self
                    .common
                    .end
                    .locator()
                    .ok_or_else(|| err("command.failed", &[]))?;
                let start = terrain
                    .structure_start(&structure, pos[0] >> 4, pos[2] >> 4)
                    .ok_or_else(|| err("commands.place.structure.failed", &[]))?;
                let (min, max) = (start.bounding_box.min, start.bounding_box.max);
                self.place_check_loaded([min[0] >> 4, min[2] >> 4], [max[0] >> 4, max[2] >> 4])?;
                let (placed, entities) =
                    self.with_level(|b, level, ticks| b.place_structure(level, ticks, &start));
                for entity in entities {
                    self.spawn_entity_from_nbt(Tag::Compound(entity));
                }
                if !placed {
                    return Err(err("commands.place.structure.failed", &[]));
                }
                self.feedback(Text::translate(
                    "commands.place.structure.success",
                    &[
                        Text::literal(structure),
                        num(pos[0]),
                        num(pos[1]),
                        num(pos[2]),
                    ],
                ));
                Ok(1)
            }
        }
    }
    pub(in crate::game::server_side) fn cmd_place(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(id)) = ctx.get("template").cloned() else {
            return Ok(0);
        };
        let pos = self.place_pos(ctx, "pos")?;
        let rotation = match ctx.get("rotation") {
            Some(Arg::Str(r)) => Rotation::from_name(r).unwrap_or(Rotation::None),
            _ => Rotation::None,
        };
        let mirror = match ctx.get("mirror") {
            Some(Arg::Str(m)) => match m.as_str() {
                "left_right" => Mirror::LeftRight,
                "front_back" => Mirror::FrontBack,
                _ => Mirror::None,
            },
            _ => Mirror::None,
        };
        let integrity = match ctx.get("integrity") {
            Some(Arg::Float(f)) => *f,
            _ => 1.0,
        };
        let seed = match ctx.get("seed") {
            Some(Arg::Int(s)) => *s,
            _ => 0,
        };
        let strict = ctx.has("strict");
        let entry = self
            .template_entry(&id)
            .ok_or_else(|| err("commands.place.template.invalid", &[&id]))?;
        let size = template_size(&entry.tag);
        let end = [pos[0] + size[0], pos[2] + size[2]];
        self.place_check_loaded([pos[0] >> 4, pos[2] >> 4], [end[0] >> 4, end[1] >> 4])?;
        // `placeInWorld(level, pos, pos, settings, createRandom(seed), flags)`.
        let settings = load_settings(mirror, rotation, integrity, i64::from(seed), strict);
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        if !self.place_template_entry(&entry, at, settings, mirror, rotation) {
            return Err(err("commands.place.template.failed", &[]));
        }
        self.feedback(Text::translate(
            "commands.place.template.success",
            &[Text::literal(id), num(pos[0]), num(pos[1]), num(pos[2])],
        ));
        Ok(1)
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;
