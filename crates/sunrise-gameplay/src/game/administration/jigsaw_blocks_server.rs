use crate::game::ServerGame;
use sunrise_gameplay_common::game::jigsaw_blocks::Pos;
use sunrise_level::{
    containers::BlockEntity,
    jigsaw_block::{JigsawBlock, JointType},
};
use sunrise_protocol::packets::play;
use {sunrise_core::nbt::Tag, sunrise_world::worldgen::generator::JigsawQuery};

impl ServerGame<'_> {
    pub(in crate::game) fn jigsaw_block_mut(&mut self, pos: Pos) -> Option<&mut JigsawBlock> {
        match self.common.world.block_entities.get_mut(&pos)? {
            BlockEntity::JigsawBlock(b) => Some(b),
            _ => None,
        }
    }

    /// `save::decode_block_entity` for data loaded into the block at `pos`
    /// (`/setblock`, `/data`): a jigsaw without `joint` takes the default its
    /// facing gives (`JigsawBlockEntity.loadAdditional`), which the decode
    /// alone cannot know.
    pub(in crate::game) fn decode_block_entity_at(
        &self,
        pos: Pos,
        tag: &Tag,
    ) -> Option<(Pos, BlockEntity)> {
        let (key, mut entity) = sunrise_sim::save::decode_block_entity_replacing(
            tag,
            &self.common.data_block_name([pos.0, pos.1, pos.2]),
            self.common.world.block_entities.get(&pos),
        )?;
        if let BlockEntity::JigsawBlock(b) = &mut entity
            && tag.get("joint").is_none()
            && let Some(front) = self.common.jigsaw_front(pos)
        {
            b.joint = JointType::default_for_front(&front);
        }
        Some((key, entity))
    }

    pub fn apply_set_jigsaw_block(&mut self, p: &play::SetJigsawBlock) {
        let pos = (p.pos[0], p.pos[1], p.pos[2]);
        let Some(b) = self.jigsaw_block_mut(pos) else {
            return;
        };
        b.name = p.name.clone();
        b.target = p.target.clone();
        b.pool = p.pool.clone();
        b.final_state = p.final_state.clone();
        // JointType.byName(name, ALIGNED).
        b.joint = JointType::of_serialized_name(&p.joint).unwrap_or(JointType::Aligned);
        b.placement_priority = p.placement_priority;
        b.selection_priority = p.selection_priority;
        self.mark_block_entity_updated(pos);
    }

    /// `handleSetJigsawBlock` for a guest: only a creative player with game
    /// master commands edits the block.
    pub(in crate::game::server_side) fn lan_set_jigsaw_block(
        &mut self,
        conn: u32,
        p: play::SetJigsawBlock,
    ) {
        let Some(g) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
        else {
            return;
        };
        let allowed = g.creative() && g.permission >= 2;
        self.as_guest(conn, |game| {
            if allowed {
                game.apply_set_jigsaw_block(&p);
            }
        });
    }

    /// `handleJigsawGenerate` for a guest.
    pub(in crate::game) fn lan_jigsaw_generate(&mut self, conn: u32, p: play::JigsawGenerate) {
        let Some(g) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
        else {
            return;
        };
        let allowed = g.creative() && g.permission >= 2;
        self.as_guest(conn, |game| {
            if allowed {
                game.generate_jigsaw_block(&p);
            }
        });
    }

    /// `JigsawBlockEntity.generate`: the pool is built from the block's
    /// front neighbour, `levels` deep.
    pub(in crate::game) fn generate_jigsaw_block(&mut self, p: &play::JigsawGenerate) {
        let pos = (p.pos[0], p.pos[1], p.pos[2]);
        let Some(BlockEntity::JigsawBlock(b)) = self.common.world.block_entities.get(&pos) else {
            return;
        };
        let (pool, target) = (b.pool.clone(), b.target.clone());
        let Some(front) = self.common.jigsaw_front(pos) else {
            return;
        };
        let step = sunrise_gameplay_common::game::facing_offset(&front);
        let position = [pos.0 + step[0], pos.1 + step[1], pos.2 + step[2]];
        // registry.getOrThrow(pool): an unknown pool throws, ending the packet.
        if !self
            .common
            .ui
            .registries
            .ids("minecraft:worldgen/template_pool")
            .contains(&pool)
        {
            return;
        }
        let Some(terrain) = self.common.end.locator() else {
            return;
        };
        let query = JigsawQuery {
            pool: &pool,
            target: &target,
            max_depth: p.levels,
            position,
        };
        let keep = p.keep_jigsaws;
        let (_, entities) = self.with_level(|b, level, ticks| {
            b.place_jigsaw(level, ticks, position, keep, || {
                terrain.jigsaw_pieces(&query)
            })
        });
        for entity in entities {
            self.spawn_entity_from_nbt(Tag::Compound(entity));
        }
        let _ = self.server_process_level_events();
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;
