//! Pure common block-state, bed, respawn-anchor, and portal-eligibility helpers.

use sunrise_core::geometry::{Vec3, VoxelShape};
use sunrise_items::items::ItemStack;
use sunrise_sim::{
    bed_geometry::{SleepOn, pos_to_bed, shape_top, slept_on},
    motion::PlayerMotion,
    stand_up::bed_forward,
};
use sunrise_world::registry::BlockStateId;

impl crate::game::CommonGame<'_> {
    pub fn prop(&self, state: BlockStateId, key: &str) -> Option<String> {
        self.pack
            .assets
            .registry
            .state(state)?
            .properties
            .get(key)
            .cloned()
    }

    /// The same block with other properties (`withProperty`).
    pub fn with_props(&self, state: BlockStateId, props: &[(&str, &str)]) -> Option<BlockStateId> {
        let mut s = self.pack.assets.registry.state(state)?.clone();
        for (k, v) in props {
            s.properties.insert((*k).to_owned(), (*v).to_owned());
        }
        self.pack.assets.registry.resolve(&s).ok()
    }

    pub fn is_bed(&self, state: BlockStateId) -> bool {
        self.res.facts().get(state).name.ends_with("_bed")
    }

    /// The bed's `facing`, as a step.
    pub fn bed_forward(&self, state: BlockStateId) -> [i32; 3] {
        let facing = self.pack.assets.registry.state(state);
        bed_forward(
            facing
                .and_then(|s| s.properties.get("facing"))
                .map(String::as_str),
        )
    }

    /// The bed's `part`: head or foot.
    pub(in crate::game) fn bed_part(&self, state: BlockStateId) -> Option<&str> {
        self.pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("part"))
            .map(String::as_str)
    }

    pub fn is_anchor(&self, state: BlockStateId) -> bool {
        &*self.res.facts().get(state).name == "minecraft:respawn_anchor"
    }

    pub fn anchor_charges(&self, state: BlockStateId) -> i32 {
        self.pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("charges"))
            .and_then(|c| c.parse().ok())
            .unwrap_or(0)
    }

    /// `RespawnAnchorBlock.useItemOn`'s test: `held` is glowstone and the anchor can be charged.
    pub fn anchor_takes(&self, pos: [i32; 3], held: Option<&ItemStack>) -> bool {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        held.is_some_and(|s| &*s.item == "minecraft:glowstone")
            && self.is_anchor(state)
            && self.anchor_charges(state) < 4
    }

    /// `LivingEntity.canUsePortal` over a player's body and its transport.
    pub fn can_use_portal_of(&self, p: &PlayerMotion, riding: bool, sleeping: bool) -> bool {
        !p.health.dead_or_dying() && !riding && !sleeping
    }

    /// [`can_use_portal_of`](Self::can_use_portal_of) for the player of this game.
    pub fn player_can_use_portal(&self) -> bool {
        self.player
            .as_ref()
            .is_some_and(|p| self.can_use_portal_of(p, self.riding.is_some(), p.sleeping))
    }
}

pub trait BlockPropsGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `CampfireBlock.canLight || CandleBlock.canLight || CandleCakeBlock.canLight`.
    fn can_light(&self, state: BlockStateId, name: &str) -> bool {
        let lit = self.prop(state, "lit");
        let waterlogged = self.prop(state, "waterlogged");
        let dry_unlit = lit.as_deref() == Some("false") && waterlogged.as_deref() == Some("false");
        ((self.in_block_tag("minecraft:campfires", name)
            || self.in_block_tag("minecraft:candles", name))
            && dry_unlit)
            || (self.in_block_tag("minecraft:candle_cakes", name)
                && lit.as_deref() == Some("false"))
    }

    /// `LivingEntity.startSleeping`'s height: the top of the shape the player lies on.
    fn sleep_height(&self, head: [i32; 3]) -> Option<f64> {
        let state = self.world.block(head[0], head[1], head[2]);
        if !self.is_bed(state) {
            return None;
        }
        let name = &self.res.facts().get(state).name;
        let part = self.bed_part(state);
        let step = self.bed_forward(state);
        let foot_at = [head[0] - step[0], head[1], head[2] - step[2]];
        let foot_state = self.world.block(foot_at[0], foot_at[1], foot_at[2]);
        let foot_name = &self.res.facts().get(foot_state).name;
        let foot_part = self.bed_part(foot_state);
        let on = slept_on(name, part, Some((foot_name, foot_part)))?;
        let at = match on {
            SleepOn::Head => head,
            SleepOn::Foot => foot_at,
        };
        shape_top(
            self.outline_boxes_iter(
                &self.world,
                self.world.block(at[0], at[1], at[2]),
                crate::game::Side::shulkers(self.side_state()),
                at,
                true,
            ),
            f64::from(at[1]),
        )
    }

    /// Where the sleeper lies (`setPosToBed`), when the bed gives it a shape.
    fn sleeping_position(&self, head: [i32; 3]) -> Option<Vec3> {
        self.sleep_height(head)
            .map(|height| pos_to_bed(head, height))
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> BlockPropsGameExt1<'__sg> for G {}

impl crate::game::CommonGame<'_> {
    /// Whether a vehicle can take the server players riding it through a
    /// portal: only a server moves players between its levels. Elsewhere a
    /// ridden vehicle stays, rather than leave its rider behind.
    pub fn carries_riders(&self) -> bool {
        self.options.dedicated.is_some()
    }
}

pub trait BlockPropsGameExt2<'__sg>: crate::game::SideGame<'__sg> {
    /// `canUsePortal(false)` of a mob: what the mob itself decides
    /// (`mobs::mob_can_use_portal`), riding nothing, not asleep, and not
    /// carrying the players this side does.
    fn mob_portal_eligible(&self, m: &sunrise_sim::mobs::Mob) -> bool {
        sunrise_sim::mobs::mob_can_use_portal(m)
            && m.extra.mount.is_none()
            && self.riding != Some(m.id)
            && (!self.guest_rides(m.id) || self.carries_riders())
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> BlockPropsGameExt2<'__sg> for G {}

impl crate::game::CommonGame<'_> {
    /// The level in place (`minecraft:overworld`), for a hook that isn't told
    /// which it runs in.
    pub fn dimension(&self) -> &'static str {
        self.dim.id()
    }
}

impl crate::game::CommonGame<'_> {
    pub fn block_collision_shape_in(
        &self,
        world: &sunrise_level::world::World,
        p: [i32; 3],
    ) -> VoxelShape {
        crate::game::native_state_shapes::collision_shape(
            self.collision,
            world,
            world.block(p[0], p[1], p[2]),
            p,
        )
    }
}

use crate::game::side_ops::lan::LanSideGameExt as _;

use crate::game::tables::WorldTablesExt as _;

use crate::game::native_state_shapes::NativeStateShapesGameExt1 as _;
