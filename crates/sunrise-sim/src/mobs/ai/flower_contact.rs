//! The flower pass of a mob's tick on the shared inside walk (#1011).
use super::*;

/// `Entity.applyEffectsFromBlocks` for the flowers that give an effect:
/// `WitherRoseBlock.entityInside` (wither for 40 ticks on a living mob that
/// can take it, not in peaceful) and `EyeblossomBlock.entityInside` (poison
/// for 25 ticks on a bee, when the open flower attracts bees). The blocks are
/// the ones the mob's box passes through along this tick's move, as
/// `Entity.checkInsideBlocks` sweeps it, so a mob falling past a flower is
/// inside it on the way.
pub(super) fn flower_contact_effects(m: &mut Mob, w: &MobWorld) {
    if !m.alive() {
        return;
    }
    let (from, to) = (m.previous, m.position);
    let mut flowers = FlowerInside { m, w };
    // The whole path in one pass, as this pass always swept it: no 16-iteration
    // budget, a fresh visited set, nothing queued.
    let _ = crate::block_contact::sweep_inside_blocks(
        &mut flowers,
        from,
        to,
        &mut crate::fast_hash::FxHashSet::default(),
        &mut NoEffects,
        1 << 22,
    );
}

/// The flower pass queues nothing: its effects are added at once.
struct NoEffects;

impl crate::block_contact::StepCollector for NoEffects {
    fn advance_step(&mut self, _: i32) {}
}

/// The flower pass's view of the shared walk: the mob's box and life, every
/// block the whole block, and the two flowers' `entityInside`.
struct FlowerInside<'a, 'w> {
    m: &'a mut Mob,
    w: &'a MobWorld<'w>,
}

impl crate::block_contact::InsideWalk for FlowerInside<'_, '_> {
    type Effects = NoEffects;
    type Error = ();
    fn bounds_at(&self, at: Vec3) -> sunrise_core::geometry::Aabb {
        self.m.bounds_at(at)
    }
    fn is_alive(&self) -> bool {
        self.m.alive()
    }
    fn block(&self, at: sunrise_core::position::BlockPos) -> sunrise_world::registry::BlockStateId {
        self.w.world.block(at.x, at.y, at.z)
    }
    fn is_air(&self, state: sunrise_world::registry::BlockStateId) -> bool {
        self.w.registry.state(state).is_none_or(|d| {
            matches!(
                d.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        })
    }
    fn inside_shape(
        &mut self,
        _state: sunrise_world::registry::BlockStateId,
        _at: sunrise_core::position::BlockPos,
    ) -> crate::block_contact::InsideShape {
        crate::block_contact::InsideShape::Full
    }
    fn entity_inside(
        &mut self,
        p: sunrise_core::position::BlockPos,
        _state: sunrise_world::registry::BlockStateId,
        _step: i32,
        _precise: bool,
        _effects: &mut NoEffects,
    ) -> std::result::Result<(), ()> {
        let (m, w) = (&mut *self.m, self.w);
        match w.block_name(p.x, p.y, p.z) {
            "minecraft:wither_rose" => {
                // World difficulty, not the regional one, and
                // isInvulnerableTo(wither) (the invulnerable flag, not the hit
                // timer); a wither and a wither skeleton refuse the effect in
                // `addEffect`.
                if w.difficulty.base != 0 && !m.invulnerable_flag {
                    m.add_effect("minecraft:wither", 40, 0);
                }
            }
            "minecraft:open_eyeblossom"
                if m.species() == Species::Bee
                    && crate::mobs::bee::attracts_bees(w, [p.x, p.y, p.z])
                    && !m.has_effect("minecraft:poison") =>
            {
                m.add_effect("minecraft:poison", 25, 0);
            }
            _ => {}
        }
        Ok(())
    }
}
