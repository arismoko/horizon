//! `RespawnAnchorBlock` in the game: glowstone charges it (`useItemOn`), a
//! right click on a charged anchor sets the respawn point in the Nether and
//! blows it up anywhere else (`useWithoutItem`), and respawning goes to it
//! (`ServerPlayer.findRespawnAndUseSpawnBlock`, `PlayerList.respawn`),
//! using a charge.
use super::Dim;
use mod_api::client::first_person::Hand;
use sunrise_core::{
    geometry::{Aabb, Axis, Vec3, VoxelShape},
    position::BlockPos,
};
use sunrise_game_data::{Result, text::Text};
use sunrise_items::{blocks::Fluid, items::ItemStack};
use sunrise_level::collisions::{SceneCollisions, no_collision_empty};
use sunrise_sim::{
    mobs::Who,
    stand_up::{StandUpLevel, StandUpType, stand_up_fallback},
};
use sunrise_world::{level::data::state_flags, registry::BlockStateId};

/// `ServerPlayer.RespawnConfig`: a respawn point other than the world spawn
/// that the game resolves through its block (a respawn anchor).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RespawnConfig {
    pub dim: Dim,
    pub pos: [i32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub forced: bool,
}

/// A respawn point as a player's save keeps it (`respawn`): dimension,
/// block, yaw, pitch and whether it is forced.
pub(in crate::game) type SavedRespawn = (String, [i32; 3], f32, f32, bool);

impl RespawnConfig {
    /// `RespawnData.of`: the yaw wrapped to [-180, 180).
    pub fn wrapped(dim: Dim, pos: [i32; 3], yaw: f32, forced: bool) -> Self {
        Self {
            dim,
            pos,
            yaw: wrap_degrees(f64::from(yaw)) as f32,
            pitch: 0.0,
            forced,
        }
    }
    /// `SetSpawnCommand`: the forced literal, with its already parsed angles.
    pub(in crate::game) fn forced(dim: Dim, pos: [i32; 3], yaw: f32, pitch: f32) -> Self {
        Self {
            dim,
            pos,
            yaw,
            pitch,
            forced: true,
        }
    }
    /// The saved point, when this world has its level (`MinecraftServer.getLevel`). A point
    /// in a level the world lacks resolves as no point at all
    /// (`findRespawnPositionAndUseSpawnBlock` falls to `TeleportTransition.createDefault`),
    /// and the save keeps it as it was ([`Self::kept`]).
    pub(in crate::game) fn from_saved(
        (dim, pos, yaw, pitch, forced): &SavedRespawn,
    ) -> Option<Self> {
        Dim::exists(dim).then(|| Self {
            dim: Dim::from_id(dim),
            pos: *pos,
            yaw: *yaw,
            pitch: *pitch,
            forced: *forced,
        })
    }
    /// What the save keeps of a point once play is done with it: `saved` while play left
    /// unchanged the point loaded from it (one in a missing level stays as saved), else the
    /// point play set or cleared.
    pub(in crate::game::server_side) fn kept(
        saved: Option<SavedRespawn>,
        now: Option<Self>,
    ) -> Option<SavedRespawn> {
        if now == saved.as_ref().and_then(Self::from_saved) {
            saved
        } else {
            now.map(|c| c.saved())
        }
    }
    pub(in crate::game::server_side) fn saved(&self) -> SavedRespawn {
        (
            self.dim.id().to_owned(),
            self.pos,
            self.yaw,
            self.pitch,
            self.forced,
        )
    }
}

/// `RespawnAnchorBlock.RESPAWN_HORIZONTAL_OFFSETS`, then the same a block
/// below, a block above, and straight above.
pub(in crate::game) const HORIZONTAL: [[i32; 3]; 8] = [
    [0, 0, -1],
    [-1, 0, 0],
    [0, 0, 1],
    [1, 0, 0],
    [-1, 0, -1],
    [1, 0, -1],
    [-1, 0, 1],
    [1, 0, 1],
];
pub(in crate::game::server_side) fn respawn_offsets() -> impl Iterator<Item = [i32; 3]> {
    HORIZONTAL
        .into_iter()
        .chain(HORIZONTAL.into_iter().map(|[x, y, z]| [x, y - 1, z]))
        .chain(HORIZONTAL.into_iter().map(|[x, y, z]| [x, y + 1, z]))
        .chain(std::iter::once([0, 1, 0]))
}

/// What a right click with an item does to an anchor before `useWithoutItem`.
pub(in crate::game) enum AnchorUse {
    /// Glowstone charged it.
    Charged,
    /// `PASS`: the off hand's glowstone charges it instead.
    Pass,
    /// `TRY_WITH_EMPTY_HAND`, or not an anchor.
    Other,
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn anchor_with_charges(
        &self,
        state: BlockStateId,
        charges: i32,
    ) -> Option<BlockStateId> {
        let mut s = self.common.pack.assets.registry.state(state)?.clone();
        s.properties.insert("charges".into(), charges.to_string());
        self.common.pack.assets.registry.resolve(&s).ok()
    }
    /// `RespawnAnchorBlock.canSetSpawn`: the `respawn_anchor_works` attribute
    /// (only the Nether's dimension type sets it).
    pub fn anchor_works(&self) -> bool {
        self.dimension_type().respawn_anchor_works
    }
    /// `RespawnAnchorBlock.useItemOn` with `held` in the hand that acts.
    pub(in crate::game::server_side) fn use_respawn_anchor(
        &mut self,
        pos: [i32; 3],
        held: Option<&ItemStack>,
    ) -> Result<AnchorUse> {
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        if !self.common.is_anchor(state) {
            return Ok(AnchorUse::Other);
        }
        let fuel = |s: Option<&ItemStack>| s.is_some_and(|s| &*s.item == "minecraft:glowstone");
        let chargeable = self.common.anchor_charges(state) < 4;
        if fuel(held) && chargeable {
            let used = held.cloned();
            self.charge_anchor(pos, state, Some(Who::Player))?;
            // itemStack.consume(1, player); the criterion sees the stack as used.
            self.consume_held();
            if let Some(stack) = used {
                self.trigger_block("minecraft:item_used_on_block", pos, &stack);
            }
            return Ok(AnchorUse::Charged);
        }
        if self.common.hand == Hand::Main && fuel(self.common.offhand.as_ref()) && chargeable {
            return Ok(AnchorUse::Pass);
        }
        Ok(AnchorUse::Other)
    }
    /// The mods' say on a server player's respawn point about to be set to
    /// `pos` in `dim` (`ServerPlayer.setRespawnPosition`, NeoForge's
    /// `PlayerSetSpawnEvent`): true when one canceled.
    pub(in crate::game::server_side) fn spawn_point_change(
        &mut self,
        dim: Dim,
        pos: Option<[i32; 3]>,
        forced: bool,
    ) -> bool {
        let (Some(player), Some(name)) = (
            self.server.lan_acting_uuid,
            self.server.lan_acting_name.clone(),
        ) else {
            return false;
        };
        let mut cancel = false;
        let id = self.own_entity_id();
        self.native_player_set_spawn(mod_api::__private::PlayerSetSpawnCall {
            player,
            player_entity_id: id,
            name: &name,
            dimension: dim.id(),
            pos,
            forced,
            cancel: &mut cancel,
        });
        cancel
    }
    /// `ServerPlayer.setRespawnPosition` for the player in place, as
    /// `/spawnpoint` sets it: the point becomes `config`, or none for None,
    /// unless a mod cancels the change (the answer is false then). A point
    /// replaces the fallback spawn, which the world spawn then stands for.
    pub(in crate::game) fn set_respawn_point(&mut self, config: Option<RespawnConfig>) -> bool {
        let (dim, pos, forced) = config.map_or((self.common.dim, None, false), |c| {
            (c.dim, Some(c.pos), c.forced)
        });
        if self.spawn_point_change(dim, pos, forced) {
            return false;
        }
        if config.is_some() {
            self.common.spawn = None;
        }
        self.server.respawn_config = config;
        true
    }
    /// `RespawnAnchorBlock.useWithoutItem` on a charged anchor: outside the
    /// Nether it explodes; inside, it becomes the respawn point.
    pub(in crate::game::server_side) fn respawn_anchor_used(
        &mut self,
        pos: [i32; 3],
    ) -> Result<()> {
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        if !self.anchor_works() {
            return self.explode_anchor(pos, state);
        }
        let config = RespawnConfig {
            dim: self.common.dim,
            pos,
            yaw: 0.0,
            pitch: 0.0,
            forced: false,
        };
        // RespawnConfig.isSamePosition: dimension and block.
        let same = self
            .server
            .respawn_config
            .is_some_and(|c| c.dim == config.dim && c.pos == config.pos);
        if !same {
            if self.spawn_point_change(self.common.dim, Some(pos), false) {
                return Ok(());
            }
            // setRespawnPosition(config, true): the message, the point; the
            // anchor replaces a bed.
            self.server_system_message(Text::translate("block.minecraft.set_spawn", &[]));
            self.server.respawn_config = Some(config);
            if let Some([x, y, z]) = self.common.level.world_spawn {
                self.common.spawn = Some(Vec3::new(
                    f64::from(x) + 0.5,
                    f64::from(y),
                    f64::from(z) + 0.5,
                ));
            }
            let at = [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            ];
            self.server_play_sound("minecraft:block.respawn_anchor.set_spawn", at, 1.0, 1.0);
        }
        Ok(())
    }
    /// `RespawnAnchorBlock.isWaterThatWouldFlow`.
    pub(in crate::game::server_side) fn water_would_flow(&self, p: [i32; 3]) -> bool {
        let f = self
            .common
            .res
            .facts()
            .get(self.common.world.block(p[0], p[1], p[2]));
        if f.fluid != Fluid::Water {
            return false;
        }
        if f.fluid_level == 8 && !f.fluid_falling {
            return true;
        }
        if f.fluid_level < 2 {
            return false;
        }
        self.common
            .res
            .facts()
            .get(self.common.world.block(p[0], p[1] - 1, p[2]))
            .fluid
            != Fluid::Water
    }
    /// `RespawnAnchorBlock.explode`: the anchor goes, then a power 5 fiery
    /// explosion (`badRespawnPointExplosion`, `ExplosionInteraction.BLOCK`)
    /// whose centre resists like water when water would flow into it.
    pub(in crate::game::server_side) fn explode_anchor(
        &mut self,
        pos: [i32; 3],
        _state: BlockStateId,
    ) -> Result<()> {
        self.level_set(pos, self.common.world.air)?;
        let horizontal = [[0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]]
            .into_iter()
            .any(|d| self.water_would_flow([pos[0] + d[0], pos[1], pos[2] + d[2]]));
        let above = self
            .common
            .res
            .facts()
            .get(self.common.world.block(pos[0], pos[1] + 1, pos[2]))
            .fluid
            == Fluid::Water;
        let in_water = horizontal || above;
        let center = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        self.block_explosion(center, in_water.then_some(pos))
    }
    /// `BedRule.destroyOnUse` of the dimension's `bed_rule` and
    /// `straw_bed_rule`: true in the Nether and the End.
    pub(in crate::game::server_side) fn beds_destroy_on_use(&self) -> bool {
        self.dimension_type().bed_rule.destroy_on_use
    }
    /// `AbstractBedBlock.useWithoutItem` where the bed rule destroys on use
    /// (the bed's head at `head`): `BedBlock.destroyOnUse` removes both
    /// halves and explodes; `StrawBedBlock` just falls apart.
    pub(in crate::game::server_side) fn destroy_bed_on_use(
        &mut self,
        head: [i32; 3],
    ) -> Result<()> {
        let state = self.common.world.block(head[0], head[1], head[2]);
        let name = self.common.res.facts().get(state).name.clone();
        let center = [
            f64::from(head[0]) + 0.5,
            f64::from(head[1]) + 0.5,
            f64::from(head[2]) + 0.5,
        ];
        if &*name == "minecraft:straw_bed" {
            return self.destroy_straw_bed(head);
        }
        // removeBlock(head), then the foot behind it if it is still this bed.
        let facing = self
            .common
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("facing").cloned())
            .unwrap_or_default();
        let back = match facing.as_str() {
            "north" => [0, 0, 1],
            "south" => [0, 0, -1],
            "west" => [1, 0, 0],
            _ => [-1, 0, 0],
        };
        self.level_set(head, self.common.world.air)?;
        let foot = [head[0] + back[0], head[1], head[2] + back[2]];
        if self
            .common
            .res
            .facts()
            .get(self.common.world.block(foot[0], foot[1], foot[2]))
            .name
            == name
        {
            self.level_set(foot, self.common.world.air)?;
        }
        self.block_explosion(center, None)
    }
    /// `Level.explode(null, badRespawnPointExplosion(center), calculator,
    /// center, 5, true, ExplosionInteraction.BLOCK)` (beds and anchors):
    /// `water_at` is the block that resists like water.
    pub(in crate::game::server_side) fn block_explosion(
        &mut self,
        center: [f64; 3],
        water_at: Option<[i32; 3]>,
    ) -> Result<()> {
        let interaction =
            self.destroy_type(sunrise_game_data::gamerules::vanilla::BLOCK_EXPLOSION_DROP_DECAY);
        self.server_explode_as(
            center,
            5.0,
            interaction,
            true,
            "minecraft:bad_respawn_point",
            water_at.map(|p| (p, 100.0)),
        )
    }

    // ---- respawning -------------------------------------------------------

    /// This level as the stand-up search reads it (`crate::stand_up`).
    pub(in crate::game::server_side) fn stand_up_level(&self) -> StandUpLevel<'_, '_> {
        StandUpLevel {
            world: &self.common.world,
            registry: self.common.pack.assets.registry,
            facts: self.common.res.facts(),
            collision: self.common.collision,
            block_tags: &self.common.res.items().block_tags,
            border: Some(self.common.border.geometry()),
        }
    }
    /// `BlockState.getCollisionShape(level, pos)`: the shape under the empty
    /// collision context.
    pub(in crate::game::server_side) fn block_collision_shape(&self, p: [i32; 3]) -> VoxelShape {
        sunrise_gameplay_common::game::native_state_shapes::collision_shape(
            self.common.collision,
            &self.common.world,
            self.common.world.block(p[0], p[1], p[2]),
            p,
        )
    }
    /// `DismountHelper.canDismountTo(level, entity, box)`: no block collides
    /// with the box and it is inside the world border.
    pub(in crate::game::server_side) fn can_dismount_to(&self, bounds: Aabb) -> bool {
        let mut scene = SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: None,
        };
        if !no_collision_empty(&mut scene, bounds) {
            return false;
        }
        // WorldBorder.isWithinBounds(box).
        let (lo, hi) = (bounds.min(Axis::X), bounds.max(Axis::X) - 1.0e-5);
        let (lz, hz) = (bounds.min(Axis::Z), bounds.max(Axis::Z) - 1.0e-5);
        self.common.border.contains(lo, lz) && self.common.border.contains(hi, hz)
    }
    /// `DismountHelper.findSafeDismountLocation` for a standing player.
    pub(in crate::game::server_side) fn safe_dismount_location(
        &self,
        p: [i32; 3],
        check_dangerous: bool,
    ) -> Option<Vec3> {
        self.stand_up_level()
            .safe_dismount_location(StandUpType::PLAYER, p, check_dangerous)
    }
    /// `Block.isPossibleToRespawnInThis`: neither solid nor liquid; banners,
    /// signs and pressure plates override it to always.
    pub fn possible_to_respawn_in(&self, state: BlockStateId) -> bool {
        let mut answer = None;
        self.native_is_possible_to_respawn_in(state, &mut answer);
        if let Some(answer) = answer {
            return answer;
        }
        let f = self.common.res.facts().get(state);
        let name = &*f.name;
        let liquid = self.common.res.behaviours().facts(state).flags & state_flags::LIQUID != 0;
        name.ends_with("_banner")
            || f.sign_form().is_some()
            || name.ends_with("_pressure_plate")
            || !(f.legacy_solid || liquid)
    }
    /// `RespawnAnchorBlock.findStandUpPosition`: a safe spot around the
    /// anchor, else any spot.
    pub(in crate::game::server_side) fn anchor_stand_up(&self, pos: [i32; 3]) -> Option<Vec3> {
        for check in [true, false] {
            for d in respawn_offsets() {
                let p = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
                if let Some(at) = self.safe_dismount_location(p, check) {
                    return Some(at);
                }
            }
        }
        None
    }
    /// `AbstractBedBlock.findStandUpPosition` for a player beside the bed
    /// whose head is at `pos`, facing `forward`, set from `yaw`.
    pub(in crate::game::server_side) fn bed_stand_up(
        &self,
        pos: [i32; 3],
        forward: [i32; 3],
        yaw: f32,
    ) -> Option<Vec3> {
        self.stand_up_level()
            .bed_stand_up(StandUpType::PLAYER, pos, forward, yaw)
    }
    /// `LivingEntity.stopSleeping`: when the block at `head` is still a bed,
    /// stand at `AbstractBedBlock.findStandUpPosition` (else 0.1 above the
    /// block over the head), turned toward the bed's bottom centre with a
    /// level pitch.
    pub(in crate::game) fn bed_wake_pose(&self, head: [i32; 3], yaw: f32) -> Option<(Vec3, f32)> {
        let state = self.common.world.block(head[0], head[1], head[2]);
        if !self.common.is_bed(state) {
            return None;
        }
        let facing = self.common.bed_forward(state);
        let stand = self
            .bed_stand_up(head, facing, yaw)
            .unwrap_or_else(|| stand_up_fallback(head));
        Some((stand, look_at_yaw(stand, head)))
    }
    /// `BedRule.canSetSpawn` of the bed's rule attribute (`bed_rule`,
    /// `straw_bed_rule`): only the Overworld's beds set the spawn, straw
    /// beds never.
    pub fn bed_sets_spawn(&self, state: BlockStateId) -> bool {
        let straw = &*self.common.res.facts().get(state).name == "minecraft:straw_bed";
        let facts = self.dimension_type();
        let rule = if straw {
            facts.straw_bed_rule
        } else {
            facts.bed_rule
        };
        rule.can_set_spawn.test(self.dark_outside())
    }
    /// `AbstractBedBlock.getSleptInBedStatType`: the statistic sleeping in this
    /// bed counts as, which a straw bed overrides.
    pub(in crate::game::server_side) fn slept_in_bed_stat(
        &self,
        state: BlockStateId,
    ) -> &'static str {
        if &*self.common.res.facts().get(state).name == "minecraft:straw_bed" {
            "sleep_in_straw_bed"
        } else {
            "sleep_in_bed"
        }
    }
    /// `ServerPlayer.findRespawnAndUseSpawnBlock` for a config in the loaded
    /// dimension: position and angles, using an anchor's charge; a bed's
    /// stand-up spot. A forced point (`spawnpoint`) not handled by the anchor
    /// or bed rule holds if the spawn column is clear.
    pub fn find_respawn_and_use(
        &mut self,
        c: RespawnConfig,
        consume: bool,
    ) -> Result<Option<(Vec3, f32, f32)>> {
        let state = self.common.world.block(c.pos[0], c.pos[1], c.pos[2]);
        if !(self.common.is_anchor(state)
            && (c.forced || self.common.anchor_charges(state) > 0)
            && self.anchor_works())
        {
            // A bed where its bed rule lets it set the spawn.
            if self.common.is_bed(state) && self.bed_sets_spawn(state) {
                let facing = self.common.bed_forward(state);
                let stand = self.bed_stand_up(c.pos, facing, c.yaw);
                let found = stand.map(|p| (p, look_at_yaw(p, c.pos), 0.0));
                return Ok(self.part_respawn_position(state, c, found));
            }
            let [x, y, z] = c.pos;
            let free = if c.forced {
                let bottom = self.possible_to_respawn_in(state);
                let top = self.possible_to_respawn_in(self.common.world.block(x, y + 1, z));
                bottom && top
            } else {
                false
            };
            let at = Vec3::new(f64::from(x) + 0.5, f64::from(y) + 0.1, f64::from(z) + 0.5);
            return Ok(self.part_respawn_position(state, c, free.then_some((at, c.yaw, c.pitch))));
        }
        let stand = self.anchor_stand_up(c.pos);
        let stand =
            self.part_respawn_position(state, c, stand.map(|p| (p, look_at_yaw(p, c.pos), 0.0)));
        if !c.forced
            && consume
            && stand.is_some()
            && let Some(next) =
                self.anchor_with_charges(state, self.common.anchor_charges(state) - 1)
        {
            self.level_set(c.pos, next)?;
        }
        Ok(stand)
    }
    /// `IBlockStateExtension.getRespawnPosition` of the respawn point `state` of `c`: `found`
    /// (position, yaw, pitch), unless a part of the state answers; an answer with a non-finite
    /// value keeps `found`.
    pub(in crate::game::server_side) fn part_respawn_position(
        &self,
        state: BlockStateId,
        c: RespawnConfig,
        found: Option<(Vec3, f32, f32)>,
    ) -> Option<(Vec3, f32, f32)> {
        if !sunrise_gameplay_common::game::api_values::block_state::block_listens(
            mod_api::HookId::RespawnPosition,
            state,
        ) {
            return found;
        }
        let mut position = found.map(|(p, yaw, pitch)| ([p.x, p.y, p.z], yaw, pitch));
        let [x, y, z] = c.pos;
        self.native_respawn_position(
            state,
            BlockPos::new(x, y, z),
            "minecraft:player",
            c.yaw,
            &mut position,
        );
        match position {
            Some((p, yaw, pitch))
                if !(p.iter().all(|v| v.is_finite()) && yaw.is_finite() && pitch.is_finite()) =>
            {
                found
            }
            answer => answer.map(|([x, y, z], yaw, pitch)| (Vec3::new(x, y, z), yaw, pitch)),
        }
    }
}

/// `RespawnPosAngle.calculateLookAtYaw`: the yaw from the position to the
/// bottom centre of the block.
pub fn look_at_yaw(position: Vec3, block: [i32; 3]) -> f32 {
    let d = Vec3::new(
        f64::from(block[0]) + 0.5 - position.x,
        f64::from(block[1]) - position.y,
        f64::from(block[2]) + 0.5 - position.z,
    )
    .normalize();
    // Mth.atan2(z, x) * 180.0F / (float) Math.PI - 90.0: the compiled code
    // multiplies by the folded float constant widened to double.
    let degrees = sunrise_core::geometry::trig::atan2(d.z, d.x)
        * f64::from(sunrise_core::geometry::trig::RAD_TO_DEG)
        - 90.0;
    wrap_degrees(degrees) as f32
}
/// `Mth.wrapDegrees(double)`.
pub(in crate::game::server_side) fn wrap_degrees(degrees: f64) -> f64 {
    let mut d = degrees % 360.0;
    if d >= 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d
}

impl crate::game::ServerGame<'_> {
    /// `RespawnAnchorBlock.charge`: one more charge, the block change and the sound.
    pub(in crate::game::server_side) fn charge_anchor(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        who: Option<Who>,
    ) -> Result<()> {
        let Some(next) = self.anchor_with_charges(state, self.common.anchor_charges(state) + 1)
        else {
            return Ok(());
        };
        self.level_set(pos, next)?;
        self.post_block_game_event("minecraft:block_change", pos, who, Some(next));
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        self.server_play_sound("minecraft:block.respawn_anchor.charge", at, 1.0, 1.0);
        Ok(())
    }
    /// `StrawBedBlock.destroyBed`: the break sound, then the bed is air.
    pub(in crate::game::server_side) fn destroy_straw_bed(&mut self, head: [i32; 3]) -> Result<()> {
        let center = [
            f64::from(head[0]) + 0.5,
            f64::from(head[1]) + 0.5,
            f64::from(head[2]) + 0.5,
        ];
        self.server_play_sound("minecraft:block.straw_bed.break_leave", center, 1.0, 1.0);
        self.level_set(head, self.common.world.air)
    }
}

use sunrise_gameplay_common::game::weather::WeatherGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    #[test]
    fn waking_up_next_to_a_bed_looks_toward_the_bed_head() {
        let head = [10, 64, 5];
        // Standing one block south of the bed head (z + 1) looks north (180 deg).
        let south = Vec3::new(10.5, 64.0, 6.5);
        assert!((look_at_yaw(south, head).abs() - 180.0).abs() < 1.0e-4);
        // Standing one block east of the bed head (x + 1) looks west (90 deg).
        let east = Vec3::new(11.5, 64.0, 5.5);
        assert!((look_at_yaw(east, head) - 90.0).abs() < 1.0e-4);
    }

    /// `findRespawnPositionAndUseSpawnBlock`: a point in a level the world lacks resolves as no
    /// point (the world spawn's transition, no missing-block message), and the save keeps it
    /// until play sets or clears one.
    #[test]
    fn a_point_in_a_missing_level_resolves_as_none_and_stays_saved() {
        let gone: SavedRespawn = ("example:gone".into(), [4, 70, 4], 90.0, 0.0, false);
        assert_eq!(RespawnConfig::from_saved(&gone), None);
        assert_eq!(
            RespawnConfig::kept(Some(gone.clone()), None),
            Some(gone.clone())
        );
        let anchor = RespawnConfig::wrapped(Dim::Nether, [1, 40, 1], 0.0, false);
        assert_eq!(
            RespawnConfig::kept(Some(gone), Some(anchor)),
            Some(anchor.saved())
        );
        let nether = anchor.saved();
        assert_eq!(RespawnConfig::from_saved(&nether), Some(anchor));
        assert_eq!(
            RespawnConfig::kept(Some(nether.clone()), Some(anchor)),
            Some(nether.clone())
        );
        assert_eq!(
            RespawnConfig::kept(Some(nether), None),
            None,
            "play cleared it"
        );
    }
}
