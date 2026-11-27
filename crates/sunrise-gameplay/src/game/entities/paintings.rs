//! Paintings: `HangingEntityItem.useOn` placement with `Painting.create`'s
//! variant choice (or the stack's `painting/variant`), `HangingEntity.survives`
//! checks every 100 ticks, breaking by attack (`dropItem`), saving, and
//! `PaintingRenderer` quads.
use super::Hit;
use std::sync::Arc;
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::{
    native_ticks::NativeTicksGameExt1 as _, transactions::entities::EntityRemovalGameExt as _,
};
use sunrise_items::items::ItemStack;
use sunrise_level::random_source;
use sunrise_sim::entities::Painting;

#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay_common::painting_variants::{
    PaintingVariants, Variant, frame_coexists, painting_coexists, painting_facing,
};
/// Whether the camera, at the origin of `corners`' space, is on the side of the quad
/// that `normal` points to.
impl crate::game::ServerGame<'_> {
    /// `HangingEntityItem.useOn` for a painting against `hit`'s face.
    pub fn place_painting(&mut self, hit: &Hit, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:painting" {
            return false;
        }
        // HangingEntityItem.mayPlace: never on a top or bottom face (FAIL).
        let Some(facing) = painting_facing(hit.face) else {
            self.server.use_failed = true;
            return true;
        };
        let pos = [
            hit.pos[0] + hit.face[0],
            hit.pos[1],
            hit.pos[2] + hit.face[2],
        ];
        self.server.next_entity += 1;
        let mut candidate = Painting {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: self.server.next_entity,
            pos,
            facing,
            variant: Arc::from(""),
            asset: None,
            width: 1,
            height: 1,
            check_interval: 0,
            ticks: 0,
        };
        // Painting.create: the largest placeable variants that survive, one at random.
        let mut fitting: Vec<Variant> = Vec::new();
        for id in world_paintings!(self).placeable.clone() {
            let Some(v) = world_paintings!(self).get(&id).cloned() else {
                continue;
            };
            (candidate.width, candidate.height) = (v.width, v.height);
            if self.common.painting_survives(&candidate) {
                fitting.push(v);
            }
        }
        // Nothing fits: CONSUME, which ItemStack.useOn still counts as a use.
        let Some(largest) = fitting.iter().map(|v| v.width * v.height).max() else {
            self.award_stat("used", &stack.item, 1);
            return true;
        };
        fitting.retain(|v| v.width * v.height >= largest);
        // Util.getRandomSafe(variants, candidate.random): the painting's own random.
        let pick = random_source::entity()
            .next_int_bound(fitting.len() as i32)
            .unwrap_or(0) as usize;
        let v = &fitting[pick];
        (candidate.variant, candidate.width, candidate.height) = (v.id.clone(), v.width, v.height);
        // EntityType.createDefaultStackConfig -> Painting.applyImplicitComponents:
        // the stack's `painting/variant` replaces the random pick, and that
        // painting has to survive where it goes (else the use is CONSUME).
        if let Some(v) = world_paintings!(self).of_stack(stack).cloned() {
            (candidate.variant, candidate.width, candidate.height) = (v.id, v.width, v.height);
            if !self.common.painting_survives(&candidate) {
                self.award_stat("used", &stack.item, 1);
                return true;
            }
        }
        // `updateCustomEntityTag`: the item's `custom_data` and its `entity_data`
        // merged over the painting, so a `variant` there replaces the pick (and
        // has to survive, else the use is CONSUME).
        let own = sunrise_sim::entity_nbt::painting_to_nbt(&candidate, [0; 4], None);
        let mut raw = sunrise_sim::block_entity_item::stack_configured_entity(
            stack,
            "minecraft:painting",
            &own,
        );
        if let Some((_, _, Some(variant))) = sunrise_sim::entity_nbt::painting_from_nbt(&raw)
            && *candidate.variant != *variant
            && let Some(v) = world_paintings!(self)
                .get(&sunrise_world::datafix::dynamic::namespaced(&variant))
                .cloned()
        {
            (candidate.variant, candidate.width, candidate.height) = (v.id, v.width, v.height);
            if !self.common.painting_survives(&candidate) {
                self.award_stat("used", &stack.item, 1);
                return true;
            }
        }
        if let sunrise_core::nbt::Tag::Compound(map) = &mut raw {
            map.remove(&"UUID".into());
        }
        if self.hanging_place_cancelled("minecraft:painting", pos) {
            return true;
        }
        self.common.persist.raw.insert(candidate.id, raw);
        let (lo, hi) = candidate.bounds();
        let at = [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ];
        self.server_play_sound("minecraft:entity.painting.place", at, 1.0, 1.0);
        // HangingEntityItem.useOn: ENTITY_PLACE at the painting, by the player, before it is
        // added; ItemStack.useOn then counts the use.
        let ctx = self.player_context(None);
        self.post_game_event("minecraft:entity_place", at, ctx);
        self.common.paintings.push(candidate);
        self.common
            .world
            .modified
            .insert((pos[0] >> 4, pos[2] >> 4));
        if !self.common.creative {
            self.consume_held();
        }
        self.award_stat("used", &stack.item, 1);
        // HangingEntityItem.useOn returns SUCCESS: the hand swings.
        self.swing_arm();
        true
    }
    /// `BlockAttachedEntity.hurtServer` -> `kill` and `dropItem`. False when a mod kept it
    /// (`native_hanging_break`): it stays at `index`.
    pub(in crate::game::server_side) fn break_painting(
        &mut self,
        index: usize,
        by_player: bool,
    ) -> bool {
        self.break_painting_by(
            index,
            by_player,
            by_player.then_some(super::commands::Target::Player),
        )
    }
    /// `BlockAttachedEntity.kill`: `ENTITY_DIE` at the painting, then `dropItem`.
    pub fn break_painting_by(
        &mut self,
        index: usize,
        by_player: bool,
        source: Option<super::commands::Target>,
    ) -> bool {
        let Some((p, at)) = self.remove_painting(index, by_player) else {
            return false;
        };
        // BlockAttachedEntity.kill, before dropItem.
        self.entity_killed_event_by(at, source);
        let creative = match source {
            Some(super::commands::Target::Player) => self.common.creative,
            Some(super::commands::Target::Guest(conn)) if self.server.lan_acting == Some(conn) => {
                self.common.creative
            }
            Some(super::commands::Target::Guest(conn)) => {
                self.command_guest(conn).is_some_and(|g| g.creative())
            }
            _ => by_player && self.common.creative,
        };
        self.drop_painting(p, at, creative);
        true
    }
    /// `BlockAttachedEntity.tick`: a painting whose `survives()` failed is
    /// `discard()`ed and drops its item, with no game event.
    pub(in crate::game::server_side) fn discard_painting(&mut self, index: usize) -> bool {
        let Some((p, at)) = self.remove_painting(index, false) else {
            return false;
        };
        self.drop_painting(p, at, false);
        true
    }
    /// `Painting.dropItem`: break sound, then item unless drops off or creative.
    pub(in crate::game) fn drop_painting(&mut self, p: Painting, at: [f64; 3], creative: bool) {
        if !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::ENTITY_DROPS)
        {
            return;
        }
        self.server_play_sound("minecraft:entity.painting.break", at, 1.0, 1.0);
        if creative {
            return;
        }
        let stack = ItemStack {
            item: Arc::from("minecraft:painting"),
            count: 1,
            damage: 0,
            extra: None,
        };
        self.spawn_item(
            stack,
            super::frames::hanging_drop_position(at, p.step()),
            [0.0; 3],
            10,
        );
    }
    /// The removal `kill` and `discard` share: the mod hook, the entity, its
    /// chunk's save mark and the centre of its box.
    pub(in crate::game::server_side) fn remove_painting(
        &mut self,
        index: usize,
        by_player: bool,
    ) -> Option<(Painting, [f64; 3])> {
        let (id, pos) = (
            self.common.paintings[index].id,
            self.common.paintings[index].pos,
        );
        if self.hanging_break_cancelled("minecraft:painting", pos, by_player) {
            return None;
        }
        // Found again by id after the listener: gone, nothing is removed.
        let index = self.common.paintings.iter().position(|p| p.id == id)?;
        let p = self.common.paintings.remove(index);
        let (lo, hi) = p.bounds();
        let at = [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ];
        self.common
            .world
            .modified
            .insert((p.pos[0] >> 4, p.pos[2] >> 4));
        Some((p, at))
    }
    /// BlockAttachedEntity.tick: check survival every 100 ticks.
    pub fn tick_paintings(&mut self) {
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut round = self.native_tick_round();
        // The painting whose tick just ended: it hears its Post before the next one's Pre.
        let mut told = None;
        // EntityTickList copy-on-iterate: while a listener may remove paintings, the round visits
        // the paintings that were there when it began, each found again by its join.
        let joins: Option<Vec<_>> = round
            .told_listening()
            .then(|| self.common.paintings.iter().map(|p| p.join).collect());
        let mut turns = joins.iter().flatten();
        let mut i = 0;
        loop {
            if let Some(id) = told.take() {
                self.native_tick_post_one(&round, id, "minecraft:painting");
            }
            if joins.is_some() {
                let Some(&join) = turns.next() else { break };
                let Some(at) = self.common.paintings.locate(join) else {
                    continue;
                };
                i = at;
            } else if i >= self.common.paintings.len() {
                break;
            }
            let (lo, hi) = self.common.paintings[i].bounds();
            let centre = [0, 2].map(|k| ((lo[k] + hi[k]) / 2.0).floor() as i32);
            if !ticking(centre[0], centre[1]) {
                i += 1;
                continue;
            }
            let id = self.common.paintings[i].id;
            // ServerLevel.tick skips an entity that `isRemoved`, here one whose removal is queued.
            let target = super::commands::Target::Painting(id);
            if self.target_removed(&target)
                || !self.native_tick_pre_one(&mut round, id, "minecraft:painting")
            {
                i += 1;
                continue;
            }
            // The Pre's listener may have removed paintings, this one included.
            if self.target_removed(&target) {
                i += 1;
                continue;
            }
            if self.common.paintings.get(i).is_none_or(|p| p.id != id) {
                let Some(now) = self.common.paintings.iter().position(|p| p.id == id) else {
                    continue;
                };
                i = now;
            }
            told = Some(id);
            // `Entity.tickCount` is raised before the rest of the tick, and is not saved.
            self.common.paintings[i].ticks = self.common.paintings[i].ticks.saturating_add(1);
            self.common.paintings[i].check_interval += 1;
            if self.common.paintings[i].check_interval >= 100 {
                self.common.paintings[i].check_interval = 0;
                let p = self.common.paintings[i].clone();
                if !self.common.painting_survives(&p) && self.discard_painting(i) {
                    continue;
                }
            }
            i += 1;
        }
        if let Some(id) = told {
            self.native_tick_post_one(&round, id, "minecraft:painting");
        }
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use sunrise_sim::entities::ItemFrame;

    #[test]
    fn hanging_entities_block_overlapping_other_types() {
        let p = Painting {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 1,
            pos: [0, 0, 0],
            facing: 0,
            variant: Arc::from("minecraft:kebab"),
            asset: None,
            width: 1,
            height: 1,
            check_interval: 0,
            ticks: 0,
        };
        let mut f = ItemFrame {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 2,
            pos: [0, 0, 0],
            facing: 3,
            glow: false,
            item: None,
            rotation: 0,
            drop_chance: 1.0,
            fixed: false,
            invisible: false,
            check_interval: 0,
            ticks: 0,
            custom_name: None,
            random: sunrise_level::random_source::entity(),
        };
        for glow in [false, true] {
            f.glow = glow;
            assert!(!frame_coexists(&f, &[], std::slice::from_ref(&p)));
            assert!(!painting_coexists(&p, &[], std::slice::from_ref(&f)));
            f.pos = [2, 0, 0];
            assert!(frame_coexists(&f, &[], std::slice::from_ref(&p)));
            assert!(painting_coexists(&p, &[], std::slice::from_ref(&f)));
            f.pos = [0, 0, 0];
        }
    }

    /// `HangingEntityItem.useOn` returns SUCCESS (the hand swings) when a
    /// painting is created on a wall. Floor, ceiling, or a failed create is
    /// CONSUME: no swing.
    fn painting_use_on_swings(face: [i32; 3], created: bool) -> bool {
        painting_facing(face).is_some() && created
    }

    #[test]
    fn placing_a_painting_on_a_wall_swings() {
        assert!(painting_use_on_swings([0, 0, 1], true));
        assert!(painting_use_on_swings([-1, 0, 0], true));
        assert!(painting_use_on_swings([0, 0, -1], true));
        assert!(painting_use_on_swings([1, 0, 0], true));
        assert!(!painting_use_on_swings([0, 1, 0], true));
        assert!(!painting_use_on_swings([0, -1, 0], true));
        assert!(!painting_use_on_swings([0, 0, 1], false));
    }
}
