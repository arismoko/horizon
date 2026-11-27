//! Block callbacks shared by thrown projectiles and fishing bobbers.
use sunrise_core::{geometry::Vec3, position::BlockPos};
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::creatures::ProjectileHitter;
use sunrise_sim::mobs::Who;

impl crate::game::ServerGame<'_> {
    /// The common block-state callbacks before projectile-specific breaking.
    pub(in crate::game) fn projectile_block_impact(
        &mut self,
        projectile: &ProjectileHitter,
        pos: [i32; 3],
        at: Vec3,
        face: [i32; 3],
    ) -> Result<()> {
        self.bell_projectile_hit(projectile, pos, at, face);
        self.amethyst_projectile_hit(pos);
        if self.with_level(|blocks, level, ticks| {
            blocks.big_dripleaf_projectile_hit(level, ticks, BlockPos::new(pos[0], pos[1], pos[2]))
        }) {
            self.server_process_level_events()?;
        }
        Ok(())
    }

    /// `TargetBlock.onProjectileHit`: a target block takes its power from
    /// where the projectile landed and resets after 20 ticks for an arrow, 8
    /// for any other projectile. A player's shot counts as a target hit.
    pub(in crate::game) fn target_block_hit(
        &mut self,
        p: &ProjectileHitter,
        pos: [i32; 3],
        at: Vec3,
        face: [i32; 3],
    ) {
        let block = BlockPos::new(pos[0], pos[1], pos[2]);
        let arrow = p.arrow;
        let location = [at.x, at.y, at.z];
        let hit = self.with_level(|b, level, ticks| {
            b.target_projectile_hit(level, ticks, block, location, face, arrow)
        });
        if let Some(strength) = hit {
            let _ = self.server_process_level_events();
            self.native_target_hit(self.common.dim.id(), pos, strength);
            if p.owner == Some(Who::Player)
                && (p.guest.is_none() || p.guest == self.server.lan_acting)
            {
                self.award_custom_stat("target_hit", 1);
                let projectile = sunrise_sim::advancements::EntityView {
                    kind: p.kind.to_owned(),
                    position: [p.position.x, p.position.y, p.position.z],
                    ..Default::default()
                };
                self.trigger_target_hit(&projectile, strength);
            }
        }
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;
