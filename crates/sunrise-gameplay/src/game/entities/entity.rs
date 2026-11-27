use super::commands::Target;
use super::persist::ChunkKey;
use std::collections::{HashMap, HashSet};
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_core::nbt::Tag;
use sunrise_core::position::BlockPos;
use sunrise_core::random::RandomSource as _;
use sunrise_gameplay_common::game::entity::{
    Entity, EntityHandle, armor_stand_size, entity_lists, entity_lists_of, for_each_entity,
};
pub(in crate::game) use sunrise_gameplay_common::game::entity_common::InsideSources;
use sunrise_gameplay_common::game::entity_common::item_can_burn;
use sunrise_gameplay_common::game::native_entity_refs_common::{
    NativeEntityRefsCommonGameExt1 as _, NativeEntityRefsCommonGameExt5 as _,
};
use sunrise_gameplay_common::game::{
    FallingBlock, boats_common::Boat, fireworks::Rocket, minecarts::Minecart, tnt_common::PrimedTnt,
};
use sunrise_sim::entities::{ArmorStand, ExperienceOrb, ItemEntity};
use sunrise_sim::join_seq::{JoinSeq, Joined as _};
use sunrise_sim::mobs::Mob;
use sunrise_sim::projectiles::{Projectile, ProjectileKind};
use sunrise_sim::thrown_trident::ThrownTrident;

#[allow(unused_imports)]
pub(in crate::game) use super::pick_entity_server::visit_server_section_members;

/// Assign a stored owner's delayed id at its existing allocation point.
/// Kinds without a lifetime slot pass None; a bound owner must keep its id.
pub fn assign_stored_id(
    next: &mut u32,
    id: &mut u32,
    lifetime: Option<&sunrise_mod_runtime::attachments::AttachmentLifetime>,
) -> u32 {
    debug_assert!(*id != 0 || lifetime.is_none_or(|slot| !slot.is_bound()));
    super::lan_entities::assign_entity_id(next, id)
}

/// What `Entity.thunderHit` does to a kind: it sets the entity on fire and
/// calls `hurtServer(lightning_bolt, 5)`, so a kind whose `hurtServer` does
/// nothing (projectiles, rockets, TNT, falling blocks, armor stands here) stays
/// untouched. A strike runs the stages in this order.
pub(in crate::game::server_side) trait ServerEntity: Entity {
    /// The kind's serializer overlays its object-owned attachments itself (for
    /// recursive object trees); the common save walk must not encode them twice.
    const SERIALIZES_ATTACHMENTS: bool = false;
    /// Saves entity `index` of this kind into its chunk's entity list: the
    /// chunk and the NBT when it lies in `keys` and is saved with its chunk.
    /// Required, so a new kind states how it saves.
    fn save_nbt(
        game: &mut crate::game::ServerGame<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)>;
    /// Loads a saved entity whose type id is `kind` as this kind and joins it to
    /// the world, or hands `tag` back when `kind` is not one this kind loads.
    /// Required, so a new kind states how it loads: it cannot be saved and then
    /// dropped silently on the way back in.
    fn load_nbt(
        game: &mut crate::game::ServerGame<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag>;
    /// Width and height of the box a portal sweep follows, or None for a kind
    /// the game does not carry through portals (so far falling blocks; fishing
    /// hooks are off the walk, and `FishingHook.canUsePortal` is false in
    /// vanilla).
    fn portal_size(&self) -> Option<(f32, f32)> {
        None
    }
    /// Whether [`Self::portal_size`] is Some, without working the size out (a
    /// mob's depends on its age, scale and pose).
    fn sweeps_portals(&self) -> bool {
        self.portal_size().is_some()
    }
    /// `Entity.canUsePortal(false)` for a kind in `portal_size`. A kind that
    /// cannot stays tracked, so its cooldown and process run out.
    fn can_use_portal(&self, _game: &crate::game::ServerGame<'_>) -> bool {
        true
    }
    /// Whether movement applies portal block effects before Entity's base tick.
    fn portal_after_move(&self) -> bool {
        false
    }
    /// Where it stood at the start of its tick, for the kinds that store it
    /// (None: `portals.before` has it).
    fn portal_from(&self) -> Option<Vec3> {
        None
    }
    /// Whether this tick's movement can have entered a portal block: a stuck
    /// arrow or a trident in the ground that did not move enters none.
    fn portal_sweeps(&self) -> bool {
        true
    }
}

/// Whether this projectile still lives its list's tick: an arrow's state was
/// not removed. A thrown projectile leaves its list through the batch's own
/// keep flags, not a field of its own.
pub(in crate::game::server_side) fn shot_alive(shot: &Projectile) -> bool {
    !shot.destroyed && shot.arrow.as_ref().is_none_or(|a| !a.removed)
}

/// The item's view of the shared inside walk: its box, its life, the batch's
/// blocks, and the part's contact, fired mid-walk over a lent level. An item
/// has no per-block vanilla arms of its own in the walk: its fire, lava and
/// cactus contacts are its own tick's (`ItemEntity::environment_tick`), kept
/// where they were.
pub(in crate::game) struct ItemInside<'a, 'g> {
    pub(in crate::game) game: &'a mut crate::game::ServerGame<'g>,
    pub(in crate::game) item: &'a mut ItemEntity,
}

impl sunrise_sim::block_contact::InsideWalk for ItemInside<'_, '_> {
    type Effects = mod_api::InsideEffects;
    type Error = ();
    fn bounds_at(&self, at: Vec3) -> Aabb {
        let b = self.item.bounds();
        let d = at.subtract(self.item.position);
        Aabb::new(
            b.min_position().add_vector(d),
            b.max_position().add_vector(d),
        )
    }
    fn is_alive(&self) -> bool {
        self.item.health > 0
    }
    fn block(&self, at: sunrise_core::position::BlockPos) -> sunrise_world::registry::BlockStateId {
        self.game.world.block(at.x, at.y, at.z)
    }
    fn is_air(&self, state: sunrise_world::registry::BlockStateId) -> bool {
        self.game.pack.assets.registry.state(state).is_none_or(|d| {
            matches!(
                d.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        })
    }
    fn inside_shape(
        &mut self,
        state: sunrise_world::registry::BlockStateId,
        at: BlockPos,
    ) -> sunrise_sim::block_contact::InsideShape {
        self.game.common.pack.assets.registry.state(state).map_or(
            sunrise_sim::block_contact::InsideShape::Full,
            |def| {
                sunrise_sim::block_contact::cauldron_inside_shape(
                    self.game.common.collision,
                    state,
                    def,
                    at,
                )
            },
        )
    }
    fn collided_with_fluid(&self, at: BlockPos, from: Vec3, to: Vec3) -> bool {
        sunrise_sim::entities::swept_fluid_contact(
            &self.game.common.world,
            &self.game.common.res.motion().fluids,
            at,
            self.bounds_at(from),
            to.subtract(from),
        )
    }
    fn fluid_inside(
        &mut self,
        at: BlockPos,
        _: i32,
        effects: &mut mod_api::InsideEffects,
    ) -> Result<(), ()> {
        sunrise_sim::block_contact::queue_fluid_effects(
            effects,
            self.game
                .common
                .res
                .motion()
                .fluids
                .fluid(self.block(at))
                .kind(),
        );
        Ok(())
    }
    fn entity_inside(
        &mut self,
        at: sunrise_core::position::BlockPos,
        state: sunrise_world::registry::BlockStateId,
        _step: i32,
        precise: bool,
        effects: &mut mod_api::InsideEffects,
    ) -> std::result::Result<(), ()> {
        let id = self.item.id;
        self.game
            .fire_batch_inside(id as i32, "minecraft:item", state, at, precise, effects);
        let block = &*self.game.common.res.facts().get(state).name;
        sunrise_sim::block_contact::queue_block_effects(effects, block, state, at);
        if block == "minecraft:cactus" {
            self.game
                .hurt_lent_item_entity(self.item, 1.0, "minecraft:cactus", None, None, None);
        }
        Ok(())
    }
}

/// The projectile's view of the shared inside walk, the item's shape of it.
/// `builtins` is set for the kinds whose own tick applies block effects
/// ([`Projectile::applies_block_effects`]): their fire, lava, water, cauldrons
/// and powder snow queue here. An arrow's run in its own collector, and llama
/// spit takes none, so the walk gives those only the parts' contacts.
pub(in crate::game) struct ProjectileInside<'a, 'g> {
    pub(in crate::game) game: &'a mut crate::game::ServerGame<'g>,
    pub(in crate::game) shot: &'a mut Projectile,
    pub(in crate::game) builtins: bool,
}

impl sunrise_sim::block_contact::InsideWalk for ProjectileInside<'_, '_> {
    type Effects = mod_api::InsideEffects;
    type Error = ();
    fn bounds_at(&self, at: Vec3) -> Aabb {
        let b = self.shot.bounds();
        let d = at.subtract(self.shot.position);
        Aabb::new(
            b.min_position().add_vector(d),
            b.max_position().add_vector(d),
        )
    }
    fn is_alive(&self) -> bool {
        shot_alive(self.shot)
    }
    fn block(&self, at: sunrise_core::position::BlockPos) -> sunrise_world::registry::BlockStateId {
        self.game.world.block(at.x, at.y, at.z)
    }
    fn is_air(&self, state: sunrise_world::registry::BlockStateId) -> bool {
        self.game.pack.assets.registry.state(state).is_none_or(|d| {
            matches!(
                d.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        })
    }
    fn inside_shape(
        &mut self,
        state: sunrise_world::registry::BlockStateId,
        at: BlockPos,
    ) -> sunrise_sim::block_contact::InsideShape {
        self.game.common.pack.assets.registry.state(state).map_or(
            sunrise_sim::block_contact::InsideShape::Full,
            |def| {
                sunrise_sim::block_contact::cauldron_inside_shape(
                    self.game.common.collision,
                    state,
                    def,
                    at,
                )
            },
        )
    }
    fn collided_with_fluid(&self, at: BlockPos, from: Vec3, to: Vec3) -> bool {
        self.builtins
            && sunrise_sim::entities::swept_fluid_contact(
                &self.game.common.world,
                &self.game.common.res.motion().fluids,
                at,
                self.bounds_at(from),
                to.subtract(from),
            )
    }
    fn fluid_inside(
        &mut self,
        at: BlockPos,
        _: i32,
        effects: &mut mod_api::InsideEffects,
    ) -> Result<(), ()> {
        sunrise_sim::block_contact::queue_fluid_effects(
            effects,
            self.game
                .common
                .res
                .motion()
                .fluids
                .fluid(self.block(at))
                .kind(),
        );
        Ok(())
    }
    fn entity_inside(
        &mut self,
        at: sunrise_core::position::BlockPos,
        state: sunrise_world::registry::BlockStateId,
        _step: i32,
        precise: bool,
        effects: &mut mod_api::InsideEffects,
    ) -> std::result::Result<(), ()> {
        let (id, kind) = (self.shot.id, self.shot.entity_type());
        self.game
            .fire_batch_inside(id as i32, kind, state, at, precise, effects);
        if self.builtins {
            let block = &*self.game.common.res.facts().get(state).name;
            sunrise_sim::block_contact::queue_block_effects(effects, block, state, at);
        }
        Ok(())
    }
}

/// Engine backing for [`mod_api::InsideEntityView`] while flushing an item's
/// `InsideEffects` collector.
struct ItemInsideAccess<'a, 'g> {
    game: &'a mut crate::game::ServerGame<'g>,
    item: &'a mut ItemEntity,
    sources: InsideSources<'a>,
}

impl mod_api::InsideEntityAccess for ItemInsideAccess<'_, '_> {
    #[inline]
    fn id(&self) -> u32 {
        self.item.id
    }

    #[inline]
    fn kind(&self) -> &str {
        "minecraft:item"
    }

    #[inline]
    fn is_alive(&self) -> bool {
        self.item.health > 0
    }

    #[inline]
    fn position(&self) -> [f64; 3] {
        [
            self.item.position.x,
            self.item.position.y,
            self.item.position.z,
        ]
    }

    #[inline]
    fn velocity(&self) -> [f64; 3] {
        [
            self.item.velocity.x,
            self.item.velocity.y,
            self.item.velocity.z,
        ]
    }

    #[inline]
    fn fire_ticks(&self) -> i32 {
        self.item.fire_ticks
    }

    #[inline]
    fn is_on_fire(&self) -> bool {
        self.item.fire_ticks > 0 && item_can_burn(self.game, self.item)
    }

    #[inline]
    fn fall_distance(&self) -> f64 {
        self.item.fall_distance
    }

    #[inline]
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        use sunrise_gameplay_common::game::api_values::TryIntoApi as _;
        let level = self.game.level(self.game.common.dim.id())?;
        self.game
            .level_block_state(level, pos)
            .and_then(|id| id.try_into_api().ok())
    }

    #[inline]
    fn push(&mut self, delta: [f64; 3]) -> bool {
        if !delta.iter().all(|v| v.is_finite()) {
            return false;
        }
        self.item.velocity = self
            .item
            .velocity
            .add_vector(Vec3::new(delta[0], delta[1], delta[2]));
        true
    }

    #[inline]
    fn set_velocity(&mut self, velocity: [f64; 3]) -> bool {
        if !velocity.iter().all(|v| v.is_finite()) {
            return false;
        }
        self.item.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        true
    }

    #[inline]
    fn make_stuck_in_block(&mut self, speed_multiplier: [f64; 3]) -> bool {
        if !speed_multiplier.iter().all(|v| v.is_finite()) {
            return false;
        }
        self.item.fall_distance = 0.0;
        self.item.velocity = Vec3::new(
            self.item.velocity.x * speed_multiplier[0],
            self.item.velocity.y * speed_multiplier[1],
            self.item.velocity.z * speed_multiplier[2],
        );
        true
    }

    #[inline]
    fn reset_fall_distance(&mut self) {
        self.item.fall_distance = 0.0;
    }

    #[inline]
    fn set_remaining_fire_ticks(&mut self, ticks: i32) {
        self.item.fire_ticks = ticks;
    }

    #[inline]
    fn add_effect(&mut self, _effect: mod_api::EffectKey, _duration: i32, _amplifier: u8) -> bool {
        false
    }

    #[inline]
    fn hurt_with_source(
        &mut self,
        damage_type: &str,
        amount: f32,
        direct: Option<mod_api::EntityRef>,
        causing: Option<mod_api::EntityRef>,
    ) -> bool {
        let subject_pos = self.position();
        let Some((direct, causing, from)) = self.game.resolve_inside_damage_sources(
            self.item.id,
            Target::Item(self.item.id),
            subject_pos,
            direct,
            causing,
            self.sources,
        ) else {
            return false;
        };
        self.game
            .hurt_lent_item_entity(self.item, amount, damage_type, direct, causing, from)
    }

    #[inline]
    fn set_block(&mut self, pos: [i32; 3], state: mod_api::BlockState) -> bool {
        use sunrise_gameplay_common::game::api_values::TryIntoEngine as _;
        let Some(level) = self.game.level(self.game.common.dim.id()) else {
            return false;
        };
        let Ok(id) = state.try_into_engine(&*self.game) else {
            return false;
        };
        let flags = mod_api::BlockUpdateFlags::NEIGHBORS | mod_api::BlockUpdateFlags::CLIENTS;
        self.game
            .level_set_block(level, pos, id, flags)
            .unwrap_or(false)
    }

    #[inline]
    fn destroy_block(&mut self, pos: [i32; 3], drop: bool) -> bool {
        let Some(level) = self.game.level(self.game.common.dim.id()) else {
            return false;
        };
        self.game
            .level_destroy_block(level, pos, drop, None, None, false)
            .is_some()
    }

    #[inline]
    fn apply_effect(&mut self, effect: mod_api::InsideEffect) {
        match effect {
            mod_api::InsideEffect::Extinguish => {
                self.item.fire_ticks = self.item.fire_ticks.min(0);
            }
            mod_api::InsideEffect::FireIgnite => {
                if item_can_burn(self.game, self.item) {
                    if self.item.fire_ticks < 0 {
                        self.item.fire_ticks += 1;
                    }
                    if self.item.fire_ticks >= 0 {
                        self.item.fire_ticks = self.item.fire_ticks.max(160);
                    }
                }
            }
            mod_api::InsideEffect::LavaIgnite => {
                if item_can_burn(self.game, self.item) {
                    self.item.fire_ticks = self.item.fire_ticks.max(300);
                }
            }
            mod_api::InsideEffect::Freeze | mod_api::InsideEffect::ClearFreeze => {}
        }
    }

    #[inline]
    fn apply_builtin(&mut self, action: mod_api::__private::BuiltinInsideAction) {
        use mod_api::__private::BuiltinInsideAction as Action;
        match action {
            Action::PowderSnowMelt(pos) => {
                if self.is_on_fire()
                    && self.game.can_entity_grief(Some(self.item.id))
                    && let Some(level) = self.game.level(self.game.common.dim.id())
                {
                    self.game
                        .level_destroy_block(level, pos, false, None, None, false);
                }
            }
            Action::LayeredCauldronExtinguish {
                pos: [x, y, z],
                state,
            } => {
                if self.is_on_fire() {
                    self.game.cauldron_extinguish_at(
                        BlockPos::new(x, y, z),
                        sunrise_world::registry::BlockStateId(state),
                    );
                }
            }
            Action::FireHurt(damage) => {
                self.game.hurt_lent_item_entity(
                    self.item,
                    damage,
                    "minecraft:in_fire",
                    None,
                    None,
                    None,
                );
            }
            Action::LavaHurt => {
                if item_can_burn(self.game, self.item)
                    && self.game.hurt_lent_item_entity(
                        self.item,
                        4.0,
                        "minecraft:lava",
                        None,
                        None,
                        None,
                    )
                    && (self.item.health <= 0 || self.item.ticks.is_multiple_of(10))
                {
                    let pitch = 2.0 + self.item.random.next_float() * 0.4;
                    self.game.server_play_sound(
                        "minecraft:entity.generic.burn",
                        self.item.position.components(),
                        0.4,
                        pitch,
                    );
                }
            }
        }
    }
}

/// Engine backing for [`mod_api::InsideEntityView`] while flushing a projectile's
/// `InsideEffects` collector.
struct ProjectileInsideAccess<'a, 'g> {
    game: &'a mut crate::game::ServerGame<'g>,
    shot: &'a mut Projectile,
    sources: InsideSources<'a>,
}

impl mod_api::InsideEntityAccess for ProjectileInsideAccess<'_, '_> {
    #[inline]
    fn id(&self) -> u32 {
        self.shot.id
    }

    #[inline]
    fn kind(&self) -> &str {
        self.shot.entity_type()
    }

    #[inline]
    fn is_alive(&self) -> bool {
        shot_alive(self.shot)
    }

    #[inline]
    fn position(&self) -> [f64; 3] {
        [
            self.shot.position.x,
            self.shot.position.y,
            self.shot.position.z,
        ]
    }

    #[inline]
    fn velocity(&self) -> [f64; 3] {
        [
            self.shot.velocity.x,
            self.shot.velocity.y,
            self.shot.velocity.z,
        ]
    }

    #[inline]
    fn fire_ticks(&self) -> i32 {
        self.shot.fire_ticks
    }

    #[inline]
    fn is_on_fire(&self) -> bool {
        self.shot.burning()
    }

    #[inline]
    fn fall_distance(&self) -> f64 {
        self.shot.fall_distance
    }

    #[inline]
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        use sunrise_gameplay_common::game::api_values::TryIntoApi as _;
        let level = self.game.level(self.game.common.dim.id())?;
        self.game
            .level_block_state(level, pos)
            .and_then(|id| id.try_into_api().ok())
    }

    #[inline]
    fn push(&mut self, delta: [f64; 3]) -> bool {
        if !delta.iter().all(|v| v.is_finite()) {
            return false;
        }
        self.shot.velocity = self
            .shot
            .velocity
            .add_vector(Vec3::new(delta[0], delta[1], delta[2]));
        true
    }

    #[inline]
    fn set_velocity(&mut self, velocity: [f64; 3]) -> bool {
        if !velocity.iter().all(|v| v.is_finite()) {
            return false;
        }
        self.shot.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        true
    }

    #[inline]
    fn make_stuck_in_block(&mut self, speed_multiplier: [f64; 3]) -> bool {
        if !speed_multiplier.iter().all(|v| v.is_finite()) {
            return false;
        }
        self.shot.fall_distance = 0.0;
        self.shot.velocity = Vec3::new(
            self.shot.velocity.x * speed_multiplier[0],
            self.shot.velocity.y * speed_multiplier[1],
            self.shot.velocity.z * speed_multiplier[2],
        );
        true
    }

    #[inline]
    fn reset_fall_distance(&mut self) {
        self.shot.fall_distance = 0.0;
    }

    #[inline]
    fn set_remaining_fire_ticks(&mut self, ticks: i32) {
        self.shot.fire_ticks = ticks;
    }

    #[inline]
    fn add_effect(&mut self, _effect: mod_api::EffectKey, _duration: i32, _amplifier: u8) -> bool {
        false
    }

    #[inline]
    fn hurt_with_source(
        &mut self,
        damage_type: &str,
        amount: f32,
        direct: Option<mod_api::EntityRef>,
        causing: Option<mod_api::EntityRef>,
    ) -> bool {
        if !amount.is_finite()
            || amount < 0.0
            || self
                .game
                .common
                .health_resources
                .damage
                .id(damage_type)
                .is_none()
        {
            return false;
        }
        let subject_pos = self.position();
        let Some((direct, causing, from)) = self.game.resolve_inside_damage_sources(
            self.shot.id,
            Target::Projectile(self.shot.id),
            subject_pos,
            direct,
            causing,
            self.sources,
        ) else {
            return false;
        };
        self.game
            .hurt_lent_projectile(self.shot, damage_type, direct, causing, from)
    }

    #[inline]
    fn set_block(&mut self, pos: [i32; 3], state: mod_api::BlockState) -> bool {
        use sunrise_gameplay_common::game::api_values::TryIntoEngine as _;
        let Some(level) = self.game.level(self.game.common.dim.id()) else {
            return false;
        };
        let Ok(id) = state.try_into_engine(&*self.game) else {
            return false;
        };
        let flags = mod_api::BlockUpdateFlags::NEIGHBORS | mod_api::BlockUpdateFlags::CLIENTS;
        self.game
            .level_set_block(level, pos, id, flags)
            .unwrap_or(false)
    }

    #[inline]
    fn destroy_block(&mut self, pos: [i32; 3], drop: bool) -> bool {
        let Some(level) = self.game.level(self.game.common.dim.id()) else {
            return false;
        };
        self.game
            .level_destroy_block(level, pos, drop, None, None, false)
            .is_some()
    }

    #[inline]
    fn apply_effect(&mut self, effect: mod_api::InsideEffect) {
        match effect {
            mod_api::InsideEffect::Extinguish => {
                self.shot.fire_ticks = self.shot.fire_ticks.min(0);
            }
            mod_api::InsideEffect::FireIgnite => {
                if self.shot.fire_ticks < 0 {
                    self.shot.fire_ticks += 1;
                }
                if self.shot.fire_ticks >= 0 {
                    self.shot.fire_ticks = self.shot.fire_ticks.max(160);
                }
            }
            mod_api::InsideEffect::LavaIgnite => {
                self.shot.fire_ticks = self.shot.fire_ticks.max(300);
            }
            mod_api::InsideEffect::Freeze | mod_api::InsideEffect::ClearFreeze => {}
        }
    }

    #[inline]
    fn apply_builtin(&mut self, action: mod_api::__private::BuiltinInsideAction) {
        use mod_api::__private::BuiltinInsideAction as Action;
        match action {
            // PowderSnowBlock.entityInside: the `mob_griefing` rule (a projectile
            // is no player) and the projectile's `mayInteract`.
            Action::PowderSnowMelt([x, y, z]) => {
                if self.shot.burning()
                    && self.game.can_entity_grief(None)
                    && self
                        .game
                        .projectile_may_interact(self.shot, BlockPos::new(x, y, z))
                    && let Some(level) = self.game.level(self.game.common.dim.id())
                {
                    self.game
                        .level_destroy_block(level, [x, y, z], false, None, None, false);
                }
            }
            Action::LayeredCauldronExtinguish {
                pos: [x, y, z],
                state,
            } => {
                let pos = BlockPos::new(x, y, z);
                if self.shot.burning() && self.game.projectile_may_interact(self.shot, pos) {
                    self.game
                        .cauldron_extinguish_at(pos, sunrise_world::registry::BlockStateId(state));
                }
            }
            Action::FireHurt(_) => {
                self.builtin_hurt("minecraft:in_fire");
            }
            Action::LavaHurt => {
                // Entity.lavaHurt: the burn plays when `hurtServer` returned true,
                // which only a shulker bullet's does (unless silent).
                if self.builtin_hurt("minecraft:lava") && !self.game.entity_silent(self.shot.id) {
                    let pitch = 2.0 + self.shot.random.0.next_float() * 0.4;
                    self.game.server_play_sound(
                        "minecraft:entity.generic.burn",
                        [
                            self.shot.position.x,
                            self.shot.position.y,
                            self.shot.position.z,
                        ],
                        0.4,
                        pitch,
                    );
                }
            }
        }
    }
}

impl ProjectileInsideAccess<'_, '_> {
    /// `hurtServer` from a block: `ShulkerBullet`'s plays its hurt, sends its
    /// crits and destroys it (true); `AbstractHurtingProjectile`'s does nothing;
    /// `Projectile`'s marks the thrown kinds hurt, whatever the damage. Only
    /// the bullet's is true.
    fn builtin_hurt(&mut self, damage_type: &str) -> bool {
        match self.shot.kind {
            ProjectileKind::ShulkerBullet { .. } => {
                if self.shot.destroyed {
                    return false;
                }
                let at = [
                    self.shot.position.x,
                    self.shot.position.y,
                    self.shot.position.z,
                ];
                let silent = self.game.entity_silent(self.shot.id);
                let ctx = sunrise_level::sculk::Context {
                    owner: self.shot.owner,
                    owner_uuid: self.shot.owner_uuid,
                    ..self.game.entity_event_context(self.shot.id, None)
                };
                self.shot.destroyed = true;
                self.game.shulker_bullet_hurt(at, silent);
                self.game
                    .post_game_event("minecraft:entity_damage", at, ctx);
                true
            }
            _ if self.shot.is_fireball()
                || matches!(
                    self.shot.kind,
                    ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
                ) =>
            {
                false
            }
            _ => self
                .game
                .hurt_lent_projectile(self.shot, damage_type, None, None, None),
        }
    }
}

/// Split a contact batch's typed hurt writes from the writes
/// `apply_contact_writes` handles. The hurt names the registered damage type
/// and is applied by `ServerGame::vehicle_contact_hurt` on the vehicle's own tick.
pub(in crate::game) fn split_contact_hurts(
    writes: Vec<mod_api::EntityWrite>,
) -> (Vec<mod_api::EntityWrite>, Vec<(&'static str, f32)>) {
    let mut rest = Vec::with_capacity(writes.len());
    let mut hurts = Vec::new();
    for write in writes {
        match write {
            mod_api::EntityWrite::Hurt {
                damage_type,
                amount,
            } => {
                if let Some(name) =
                    mod_api::__private::parts_installation::damage_type_name(damage_type)
                {
                    hurts.push((name, amount));
                }
            }
            other => rest.push(other),
        }
    }
    (rest, hurts)
}

/// A typed contact hurt a vehicle tick queued for the stored row.
#[derive(Clone, Copy, Debug)]
pub(in crate::game) struct ContactHurt {
    pub(in crate::game) target: Target,
    pub(in crate::game) damage_type: &'static str,
    pub(in crate::game) amount: f32,
}

impl crate::game::ServerGame<'_> {
    /// A contact hurt queued by a vehicle tick: [`Self::apply_vehicle_contact_hurts`]
    /// runs it on the stored row once the ticked copy is back.
    pub(in crate::game) fn queue_vehicle_contact_hurts(
        &mut self,
        target: Target,
        hurts: impl IntoIterator<Item = (&'static str, f32)>,
    ) {
        self.server
            .vehicle_contact_hurts
            .extend(hurts.into_iter().map(|(damage_type, amount)| ContactHurt {
                target,
                damage_type,
                amount,
            }));
    }

    /// Run the contact hurts a vehicle tick queued for `target`, after
    /// `store_*_back`. Returns true when a hit took the vehicle out of the
    /// level, so its tick must stop.
    pub(in crate::game) fn apply_vehicle_contact_hurts(&mut self, target: Target) -> bool {
        let mut queued = std::mem::take(&mut self.server.vehicle_contact_hurts);
        let mut gone = false;
        for hurt in queued.drain(..) {
            if hurt.target != target {
                self.server.vehicle_contact_hurts.push(hurt);
                continue;
            }
            if !gone && self.vehicle_contact_hurt(target, hurt.damage_type, hurt.amount) {
                gone = true;
            }
        }
        gone
    }

    /// `VehicleEntity.hurtServer` from a block contact's
    /// `EntityWrite::Hurt`: the registered type with no causing or direct
    /// entity, through the same invulnerability observation as `/damage`.
    /// A refused hit changes no health and plays no sound. Returns true when
    /// the vehicle is no longer in the level.
    pub fn vehicle_contact_hurt(&mut self, target: Target, damage_type: &str, amount: f32) -> bool {
        let present = |game: &crate::game::ServerGame<'_>| match target {
            Target::Boat(id) => game.common.boats.iter().any(|b| b.id == id),
            Target::Cart(id) => game.common.carts.iter().any(|c| c.id == id),
            _ => false,
        };
        let id = match target {
            Target::Boat(id) | Target::Cart(id) => id,
            _ => return false,
        };
        let registry = &self.common.health_resources.damage;
        let Some(kind) = registry.id(damage_type) else {
            return !present(self);
        };
        let ignites = registry.in_tag(kind, "minecraft:is_fire")
            || registry.in_tag(kind, "minecraft:is_explosion");
        let entity_kind = match target {
            Target::Boat(_) => self
                .common
                .boats
                .iter()
                .find(|b| b.id == id)
                .map(|b| b.entity_type()),
            Target::Cart(_) => self
                .common
                .carts
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.kind.id().to_owned()),
            _ => None,
        };
        let Some(entity_kind) = entity_kind else {
            return !present(self);
        };
        let vanilla = self.saved_refuses_damage(id, damage_type, false);
        let source = sunrise_sim::health::HurtSource {
            damage_type,
            causing: 0,
            direct: 0,
            direct_type: None,
            position: None,
        };
        if damage_level_of!(self, self.server.combat.0.attack(), no_push, None)
            .invulnerability_check(
                sunrise_sim::health::Hurting::Entity(id as i32, &entity_kind, source),
                vanilla,
            )
        {
            return !present(self);
        }
        match target {
            Target::Boat(_) => {
                if let Some(i) = self.common.boats.iter().position(|b| b.id == id) {
                    self.hurt_boat_result(i, amount, false);
                }
            }
            Target::Cart(_) => {
                if let Some(i) = self.common.carts.iter().position(|c| c.id == id) {
                    self.hurt_cart_result(
                        i,
                        amount,
                        false,
                        ignites,
                        super::minecarts_server::CartHit::default(),
                    );
                }
            }
            _ => {}
        }
        !present(self)
    }
}

impl crate::game::ServerGame<'_> {
    /// `Block.fallOn` effects for bodies whose `causeFallDamage` does nothing.
    pub(in crate::game) fn nonliving_fall_on(
        &mut self,
        id: u32,
        feet: Vec3,
        contact: sunrise_sim::motion::BodyContact,
    ) {
        // FarmlandBlock draws the level's float before checking LivingEntity.
        match &*self.common.res.facts().get(contact.state).name {
            "minecraft:farmland" => {
                self.common.world.random.next_float();
            }
            "minecraft:honey_block" => {
                let at = [feet.x, feet.y, feet.z];
                if !self.entity_silent(id) {
                    self.server_play_sound("minecraft:block.honey_block.slide", at, 1.0, 1.0);
                }
                self.lan_entity_event(id as i32, 54, at);
            }
            _ => {}
        }
    }

    /// Finish a body bounce before the caller changes the live entity's delta movement.
    pub(in crate::game) fn finish_body_restitution(
        &mut self,
        (id, kind): (u32, &str),
        feet: Vec3,
        response: sunrise_sim::motion::BodyRestitution,
    ) -> Vec3 {
        let Some(bounce) = response.bounce else {
            return response.velocity;
        };
        if !matches!(kind, "minecraft:item" | "minecraft:tnt")
            && self.common.motion.landing.has_shelf_bounce(bounce.state)
        {
            let pos = bounce.position;
            self.server_play_sound(
                "minecraft:block.shelf_mushroom.bounce",
                [
                    f64::from(pos.x) + 0.5,
                    f64::from(pos.y) + 0.5,
                    f64::from(pos.z) + 0.5,
                ],
                1.0,
                1.0,
            );
        }
        let movement = sunrise_sim::block_contact::bounce(
            &mut sunrise_sim::health::DamageLevel::of(
                &mut self.common.world,
                self.common.pack.assets.registry,
                Some(server_world_items!(self)),
                Some(&self.common.pack.block_registries),
                &self.common.damage_hooks,
            )
            .no_push(),
            sunrise_sim::block_contact::Contact {
                entity: id as i32,
                kind,
                state: bounce.state,
                position: bounce.position,
            },
            bounce.fall_distance,
            response.velocity,
        );
        let context = self.entity_event_context(id, None);
        self.post_game_event("minecraft:bounce", [feet.x, feet.y, feet.z], context);
        self.common.persist.sync_body_position.insert(id);
        movement
    }

    #[inline(always)]
    pub(in crate::game) fn server_game_side_fields(
        &self,
    ) -> (Option<u128>, Option<mod_api::LevelRef>, bool) {
        let level = self.common.native_lifecycle.epoch().and_then(|epoch| {
            let key = self.common.dim.key();
            let id = mod_api::__private::dimension_id(key)?;
            Some(mod_api::LevelRef::with_generation(
                epoch,
                key,
                self.server.levels.0.runtime.generation_of(id),
            ))
        });
        (
            self.server.lan_acting_uuid,
            level,
            self.server.session_lock.0.is_some(),
        )
    }

    pub(in crate::game) fn server_finish_body_restitution(
        &mut self,
        entity: (u32, &str),
        feet: Vec3,
        response: sunrise_sim::motion::BodyRestitution,
    ) -> Vec3 {
        self.finish_body_restitution(entity, feet, response)
    }

    pub(in crate::game) fn server_body_step_on(
        &mut self,
        entity: (u32, &str),
        footprint: (Vec3, Aabb),
        ground: &sunrise_world::ground_support::GroundState,
        velocity: &mut Vec3,
    ) -> Vec<mod_api::EntityWrite> {
        use sunrise_sim::block_contact::{StepLevel, step_listens, step_on};
        let side = game_side!(self);
        let motion = self.common.body_motion(&side);
        let Some(step) = motion.step_contact(entity, footprint, ground, velocity) else {
            return Vec::new();
        };
        if !step_listens(step.contact.state) {
            return Vec::new();
        }
        let mut level = sunrise_sim::health::DamageLevel::of(
            &mut self.common.world,
            self.common.pack.assets.registry,
            Some(server_world_items!(self)),
            Some(&self.common.pack.block_registries),
            &self.common.damage_hooks,
        );
        step_on(StepLevel::Server(&mut level), step, velocity)
    }

    pub(in crate::game) fn server_item_fall_on(&mut self, id: u32, on: BlockPos, distance: f64) {
        self.item_fall_on(id, on, distance);
    }

    pub(in crate::game) fn server_item_post_move_contacts(
        &mut self,
        item: &mut ItemEntity,
        client: bool,
        sources: InsideSources<'_>,
    ) -> bool {
        for write in self.server_body_step_on(
            (item.id, "minecraft:item"),
            (item.position, item.bounds()),
            &item.ground,
            &mut item.velocity,
        ) {
            self.apply_item_write(item, write);
            if item.health <= 0 {
                return false;
            }
        }
        if !client {
            self.sweep_item_inside(item, sources);
            if item.health <= 0 {
                return false;
            }
        }
        true
    }
}

/// One `ItemEntity.tick`, with the fluid push and the bubble column decided by
/// the `Entity` defaults every kind shares: fluids, move, column, settle,
/// fluids again. Returns false when the item should be removed. A server's tick
/// runs the halves itself, with the landing's `Block.fallOn` between them.
impl crate::game::ServerGame<'_> {
    /// Tick an item owned by the caller. The two results are ordinary survival
    /// and explicit consumption by a block-contact handler. `escape_speed`:
    /// the item is inside a block and moves without physics this tick, pushed
    /// out at this speed.
    /// The item's `Entity.checkInsideBlocks`, run at its batch with the part's
    /// contact fired mid-walk: the shared sweep over the item's move, the staged
    /// fire over a lent level (as `item_fall_on` lends one), and the flush of
    /// the queued writes into this item, its own tick. Nothing runs without a
    /// listening part: one `any_inside` test is the whole cost.
    pub fn sweep_item_inside(&mut self, item: &mut ItemEntity, sources: InsideSources<'_>) {
        use mod_api::InsideEntityAccess as _;
        let fire_before = item.fire_ticks;
        let burning_before = fire_before > 0 && item_can_burn(self, item);
        let mut effects = mod_api::InsideEffects::default();
        let mut inside = ItemInside { game: self, item };
        let movement = sunrise_world::entity_move::MovementRecord {
            from: inside.item.previous,
            to: inside.item.position,
            original: None,
        };
        let mut visited = std::mem::take(&mut inside.game.server.inside_visited_scratch);
        visited.clear();
        let _ = sunrise_sim::block_contact::sweep_movements(
            &mut inside,
            &[movement],
            &mut visited,
            &mut effects,
        );
        let ItemInside { game, item } = inside;
        visited.clear();
        game.server.inside_visited_scratch = visited;
        // `applyAndClear`: the queued effects and callbacks apply in the
        // collector's order, in this item's tick, until it is destroyed.
        let mut access = ItemInsideAccess {
            game,
            item,
            sources,
        };
        effects.apply_and_clear(&mut access);
        let at = access.item.position;
        if access
            .game
            .rain_falls_on(at, access.item.bounds().max_position().y)
        {
            access.item.fire_ticks = access.item.fire_ticks.min(0);
        }
        if !access.is_on_fire() {
            if burning_before {
                let pitch =
                    1.6 + (access.item.random.next_float() - access.item.random.next_float()) * 0.4;
                access.game.server_play_sound(
                    "minecraft:entity.generic.extinguish_fire",
                    at.components(),
                    0.7,
                    pitch,
                );
            }
            if access.item.fire_ticks <= fire_before {
                access.item.fire_ticks = 0;
            }
        }
    }

    /// Apply a contact Part's write to the moving item. Items hold a multiplier
    /// for the next move (`makeStuckInBlock`), but keep no active effects.
    pub(in crate::game::server_side) fn apply_item_write(
        &mut self,
        item: &mut ItemEntity,
        write: mod_api::EntityWrite,
    ) {
        let triple = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
        match write {
            mod_api::EntityWrite::Push(v) => {
                item.velocity = item.velocity.add_vector(triple(v));
            }
            mod_api::EntityWrite::SetMovement(v) => item.velocity = triple(v),
            mod_api::EntityWrite::StuckIn(v) => {
                item.stuck = triple(v);
                item.fall_distance = 0.0;
            }
            mod_api::EntityWrite::Hurt {
                damage_type,
                amount,
            } => {
                if let Some(name) =
                    mod_api::__private::parts_installation::damage_type_name(damage_type)
                {
                    self.hurt_lent_item_entity(item, amount, name, None, None, None);
                }
            }
            mod_api::EntityWrite::ResetFallDistance => item.fall_distance = 0.0,
            mod_api::EntityWrite::SetFireTicks(ticks) => item.fire_ticks = ticks,
            mod_api::EntityWrite::AddEffect { .. } => {}
        }
    }

    /// The projectile's `Entity.checkInsideBlocks`, the same batch sweep the
    /// item's is: the shared walk over its move, the staged fire over a lent
    /// level, and the flush into this projectile's own tick.
    pub fn sweep_projectile_inside(&mut self, shot: &mut Projectile, sources: InsideSources<'_>) {
        // A client's copy takes no block effects of its own: its fire is the
        // server's flag, and the blocks are the server's.
        let builtins = shot.applies_block_effects() && self.is_logical_server();
        if !builtins
            && !sunrise_sim::block_contact::HOOKS
                .get()
                .is_some_and(|hooks| (hooks.any_inside)())
        {
            return;
        }
        let (fire_before, was_on_fire) = (shot.fire_ticks, shot.burning());
        let mut effects = mod_api::InsideEffects::default();
        let mut inside = ProjectileInside {
            game: self,
            shot,
            builtins,
        };
        let movement = sunrise_world::entity_move::MovementRecord {
            from: inside.shot.previous,
            to: inside.shot.position,
            original: None,
        };
        let mut visited = std::mem::take(&mut inside.game.server.inside_visited_scratch);
        visited.clear();
        let _ = sunrise_sim::block_contact::sweep_movements(
            &mut inside,
            &[movement],
            &mut visited,
            &mut effects,
        );
        let ProjectileInside { game, shot, .. } = inside;
        visited.clear();
        game.server.inside_visited_scratch = visited;
        let mut access = ProjectileInsideAccess {
            game,
            shot,
            sources,
        };
        effects.apply_and_clear(&mut access);
        let ProjectileInsideAccess { game, shot, .. } = access;
        if !builtins || !shot_alive(shot) {
            return;
        }
        let in_rain = game.rain_falls_on(shot.position, shot.bounds().max_position().y);
        if let Some(pitch) = shot.finish_block_effects(fire_before, was_on_fire, in_rain) {
            game.server_play_sound(
                "minecraft:entity.generic.extinguish_fire",
                [shot.position.x, shot.position.y, shot.position.z],
                0.7,
                pitch,
            );
        }
    }

    /// `Projectile.mayInteract` as NeoForge has it: a player owner's
    /// `ServerPlayer.mayInteract` (inside the world border, not under spawn
    /// protection), yes with no owner (or one that is gone), else
    /// `canEntityGrief` for the owner.
    pub(in crate::game) fn projectile_may_interact(
        &self,
        shot: &Projectile,
        pos: BlockPos,
    ) -> bool {
        let guest = match self.resolved_projectile_owner(shot.owner, shot.guest) {
            None => return true,
            Some(sunrise_sim::mobs::Who::Player) => shot.guest,
            Some(sunrise_sim::mobs::Who::Mob(id)) => match self.lan_guest_conn_by_id(id) {
                Some(conn) => Some(conn),
                None => return self.can_entity_grief(Some(id)),
            },
        };
        let protection = guest
            .zip(self.server.lan.as_ref())
            .and_then(|(conn, host)| self.spawn_protection_for(host, host.hub.guest(conn)?.uuid));
        self.common
            .border
            .geometry()
            .is_within_bounds(f64::from(pos.x), f64::from(pos.z), 0.0)
            && !protection.is_some_and(|radius| {
                self.spawn_protects(radius, self.common.dim.id(), [pos.x, pos.y, pos.z])
            })
    }

    /// Resolve optional direct/causing source handles against the live level during
    /// an inside-block callback flush, including the currently swept subject (whose row
    /// may be temporarily lent out of its table). Returns `None` if an explicitly
    /// supplied handle is stale or unresolvable so the caller reports `false` without
    /// dropping attribution or rolling back sibling operations.
    pub(in crate::game) fn resolve_inside_damage_sources(
        &self,
        subject_id: u32,
        subject_target: Target,
        subject_pos: [f64; 3],
        direct: Option<mod_api::EntityRef>,
        causing: Option<mod_api::EntityRef>,
        sources: InsideSources<'_>,
    ) -> Option<(Option<Target>, Option<Target>, Option<[f64; 3]>)> {
        let resolve = |handle: mod_api::EntityRef| {
            let id = handle.id();
            let resolved = if !self.is_logical_server()
                || self.common.native_lifecycle.epoch() != Some(handle.epoch())
            {
                None
            } else if id == subject_id {
                Some((subject_target, Some(subject_pos)))
            } else if let Some((target, pos)) = sources.and_then(|source| source(id)) {
                (!matches!(target, Target::Projectile(_)) || !self.projectile_removal_pending(id))
                    .then_some((target, Some(pos)))
            } else {
                self.native_entity_target(handle)
                    .filter(|&target| self.target_level(target) == self.common.dim)
                    .map(|target| (target, self.target_position(target)))
            };
            resolved.ok_or_else(|| {
                log::warn!(
                    "entity_inside: cannot resolve damage source {id} in the swept entity's level"
                );
            })
        };
        let direct = direct.map(resolve).transpose().ok()?;
        let causing = causing.map(resolve).transpose().ok()?;
        Some((
            direct.map(|(target, _)| target),
            causing.map(|(target, _)| target),
            direct.and_then(|(_, pos)| pos),
        ))
    }

    /// `Projectile.hurtServer`: always returns `false`, marking non-immune
    /// projectiles hurt (`sync_velocity = true`, `Entity.markHurt`).
    pub(in crate::game) fn hurt_lent_projectile(
        &mut self,
        shot: &mut Projectile,
        damage_type: &str,
        direct: Option<Target>,
        causing: Option<Target>,
        position: Option<[f64; 3]>,
    ) -> bool {
        let creative = self.creative_source_of(causing);
        let vanilla = self.saved_refuses_damage(shot.id, damage_type, creative);
        let source = sunrise_sim::health::HurtSource {
            damage_type,
            causing: causing.and_then(|t| self.network_entity_id(t)).unwrap_or(0),
            direct: direct.and_then(|t| self.network_entity_id(t)).unwrap_or(0),
            position,
            direct_type: None,
        };
        if !self.native_nonliving_invulnerable(shot.id, shot.entity_type(), source, vanilla) {
            shot.sync_velocity = true;
        }
        false
    }

    /// An `entity_inside` part's contact for a kind the batch ticks, fired
    /// mid-walk over a lent level, the whole callback staged: only an answering
    /// callback's writes join the walk, and a panic there is contained, leaving
    /// the batch's rows and the level as they were. The level is lent and given
    /// back per contact, as `item_fall_on` lends its own (#2194's carrier
    /// discipline).
    pub(in crate::game::server_side) fn fire_batch_inside(
        &mut self,
        entity: i32,
        kind: &'static str,
        state: sunrise_world::registry::BlockStateId,
        at: sunrise_core::position::BlockPos,
        precise: bool,
        effects: &mut mod_api::InsideEffects,
    ) {
        let Some(dimension) = sunrise_sim::block_contact::current_level() else {
            return;
        };
        if !sunrise_sim::block_contact::HOOKS
            .get()
            .is_some_and(|hooks| (hooks.inside_listens)(state))
        {
            return;
        }
        let mut lent = sunrise_sim::health::LentLevel::take(&mut self.common.world);
        let mut level = sunrise_sim::health::DamageLevel::lent(
            &self.common.world,
            self.common.pack.assets.registry,
            &mut lent,
            Some(server_world_items!(self)),
            Some(&self.common.pack.block_registries),
            &self.common.damage_hooks,
        )
        .with_effects(crate::game::entity_server::hit_effects_of!(self, None))
        .with_reads(crate::game::entity_server::hit_reads_of!(self));
        if let Some(hooks) = sunrise_sim::block_contact::HOOKS.get() {
            (hooks.inside_fire)(
                &mut level, dimension, entity, kind, state, at, precise, effects,
            );
        }
        lent.give_back(&mut self.common.world);
    }
}

macro_rules! define_server_entity_handles {
    (() $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        /// Allocation-free storage-order walk with stable IDs and verified index hints.
        /// The next ID is captured before the caller can remove the current entity.
        pub(in crate::game) struct EntityIdCursor {
            before: JoinSeq,
            $($field: (Option<(u32, JoinSeq)>, usize, usize),)*
        }

        /// Every list moved out of `Game`, for [`for_each_entity_mut`]: the
        /// world stays readable while the entities change.
        pub struct TakenEntities {
            $(pub(in crate::game) $field: sunrise_sim::join_seq::JoinList<$ty, { sunrise_gameplay_common::game::entity::direct_entity_edit!($kind) }>,)*
        }


        impl $crate::game::ServerGame<'_> {
            /// Moves every list out; give it back with `restore_entities`.
            pub fn take_entities(&mut self) -> TakenEntities {
                TakenEntities {
                    $($field: self.common$(.$list)+.lift(),)*
                }
            }
            pub fn restore_entities(&mut self, taken: TakenEntities) {
                $({
                    let expected = self.common$(.$list)+.len() + taken.$field.len();
                    self.common$(.$list)+.restore(taken.$field, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
                    debug_assert_eq!(self.common$(.$list)+.len(), expected);
                })*
            }

            pub(in crate::game::server_side) fn entity_id_cursor(&self) -> EntityIdCursor {
                EntityIdCursor {
                    before: JoinSeq::upcoming(),
                    $($field: (self.common$(.$list)+.first().map(|e| (e.id(), e.join_seq())), 0, self.common$(.$list)+.len()),)*
                }
            }

            /// Verify a stable identity before returning any index-bearing handle.
            /// Removed next identities resume at the next surviving original join.
            /// The captured join ceiling excludes entities spawned by callbacks.
            pub(in crate::game::server_side) fn next_entity_id(&self, cursor: &mut EntityIdCursor) -> Option<(u32, EntityHandle)> {
                $({
                    let (next, hint, remaining) = &mut cursor.$field;
                    if *remaining > 0 {
                        if let Some((id, join)) = *next {
                            let index = if self.common$(.$list)+.get(*hint).is_some_and(|e| e.id() == id && e.join_seq() == join) {
                                Some(*hint)
                            } else {
                                self.common$(.$list)+.iter().position(|e| e.id() == id && e.join_seq() == join).or_else(|| {
                                    self.common$(.$list)+.iter().position(|e| e.join_seq() > join && e.join_seq() < cursor.before)
                                })
                            };
                            if let Some(i) = index {
                                *remaining -= 1;
                                *hint = i + 1;
                                *next = self.common$(.$list)+.get(i + 1).filter(|e| e.join_seq() < cursor.before).map(|e| (e.id(), e.join_seq()));
                                return Some((self.common$(.$list)+[i].id(), EntityHandle::$kind(i)));
                            }
                        }
                        *remaining = 0;
                    }
                })*
                None
            }

            /// Resolve one stable ID, checking its cheap index hint before searching.
            pub(in crate::game) fn refresh_entity_handle(&self, id: u32, hint: EntityHandle) -> Option<EntityHandle> {
                match hint {
                    $(EntityHandle::$kind(i) => {
                        if self.common$(.$list)+.get(i).is_some_and(|e| e.id() == id) { Some(hint) }
                        else { self.common$(.$list)+.iter().position(|e| e.id() == id).map(EntityHandle::$kind) }
                    })*
                }
            }

            /// Takes the entity out of its list (`Entity.discard` before it ever ticked).
            pub(in crate::game::server_side) fn discard_entity(&mut self, handle: EntityHandle) {
                match handle {
                    $(EntityHandle::$kind(i) => {
                        self.common$(.$list)+.remove(i);
                    })*
                }
            }

            /// [`Self::entity_handles`], last of each list first, so the
            /// caller can remove the entity it is on.
            pub(in crate::game::server_side) fn entity_handles_rev(&self) -> impl Iterator<Item = EntityHandle> + use<> {
                std::iter::empty()
                    $(.chain((0..self.common$(.$list)+.len()).rev().map(EntityHandle::$kind)))*
            }
        }

    };
}
pub(crate) use define_server_entity_handles;
entity_lists!(crate::game::entity_server::define_server_entity_handles,);

/// What a hit lends its listeners besides the level ([`sunrise_client::health::HitEffects`]):
/// the level sound queue and sound-seed stream, and the entity the site lends (`$lend`: an
/// `Option<`[`HitLend`](sunrise_client::health::HitLend)`>`, `None` where nothing is lent). Only field borrows of
/// `$game`, so it composes with the borrows of [`damage_level_of`].
macro_rules! hit_effects_of {
    // A site whose `$game` is borrowed whole queues into its own `$sounds`, which it appends to
    // `creatures.lan_sounds` once the level is dropped (the seeds are still drawn at the call).
    ($game:ident, $lend:expr, in $sounds:expr) => {
        sunrise_sim::health::HitEffects::new(
            &mut $sounds,
            $game.server.lan.as_ref().map(|lan| &lan.hub),
            $game.common.dim.id(),
            $game.common.native_lifecycle.epoch(),
            $lend,
        )
    };
    ($game:ident, $lend:expr) => {
        sunrise_sim::health::HitEffects::new(
            &mut $game.server.pending_native_sounds,
            $game.server.lan.as_ref().map(|lan| &lan.hub),
            $game.common.dim.id(),
            $game.common.native_lifecycle.epoch(),
            $lend,
        )
    };
}
/// What a hit's listeners need from `$game` to name and read the living entities the hit lends
/// ([`HitReads`](sunrise_client::health::HitReads)), as field-level borrows; none before the
/// world has an epoch. It asks nothing: the views it backs exist only inside a callback the
/// hit dispatches, behind that callback's own subscription check.
macro_rules! hit_reads_of {
    ($game:expr) => {
        $game
            .common
            .native_lifecycle
            .epoch()
            .map(|epoch| sunrise_sim::health::HitReads {
                epoch,
                living_types: $game.common.res.living_type_keys(),
                attributes: &$game.common.motion.attributes,
                player_effects: Some(&$game.common.potions.effects),
                player_equipment: acting_equipment!($game.common),
                player_uuid: $game.server.lan_acting_uuid,
                player_attachments: Some(&$game.common.player_state.attachments),
                entity_attachments: Some(&*$game.common.entity_attachments),
            })
    };
}
#[allow(unused_imports)]
pub(in crate::game) use hit_reads_of;
/// A hit's one decision about its causing entity `$causing` (an entity id, 0 for none), taken
/// before the site borrows the game's fields for the hit: whether a `HitHost` hook listens,
/// the causer's place in `$game.mobs` when it is a mob, and otherwise, when it is not the player
/// in place either, the parked guest it may be (see [`Game::native_parked_causer`]). Expands to
/// `(i32, bool, Option<usize>, Option<u32>)`: the causing id with the three; without a
/// listener it looks nothing up.
macro_rules! causer_decision {
    ($game:ident, $causing:expr) => {{
        let causing: i32 = $causing;
        let listened = causing != 0 && sunrise_sim::health::hit_host_listened();
        let mob = listened
            .then(|| {
                $game
                    .common
                    .mobs
                    .iter()
                    .position(|m| m.id as i32 == causing)
            })
            .flatten();
        let in_place = $game
            .common
            .player
            .as_ref()
            .is_some_and(|p| p.entity_id == causing);
        let parked = (listened && mob.is_none() && !in_place)
            .then(|| $game.native_parked_causer(causing))
            .flatten();
        (causing, listened, mob, parked)
    }};
}
#[allow(unused_imports)]
pub(in crate::game) use causer_decision;
/// The causing entity `$causing` a hit on the mob at `$index` of `$game.mobs` lends its
/// listeners, beside that victim, as `$decision` ([`causer_decision`]) decided: another mob of
/// the list (split with `get_disjoint_mut`), the player in place, or a parked guest's record
/// (read-only, as its `LivingRef` reads answer from it). A mob that caused its own hit is not
/// lent again: the listener reads it as the victim. Expands to `(&mut Mob, Option<HitLend>)`.
macro_rules! victim_and_causer {
    ($game:ident, $index:expr, $causing:expr, $decision:expr) => {{
        let (index, causing): (usize, i32) = ($index, $causing);
        let (_, listened, mob, parked): (i32, bool, Option<usize>, Option<u32>) = $decision;
        match mob {
            Some(other) if other != index => {
                let [victim, causer] = $game
                    .common
                    .mobs
                    .get_disjoint_mut([index, other])
                    .expect("the victim and its causer are two mobs");
                (victim, sunrise_sim::health::HitLend::mob(causer))
            }
            Some(_) => (&mut $game.common.mobs[index], None),
            None => (
                &mut $game.common.mobs[index],
                if listened {
                    sunrise_sim::health::HitLend::attacking_player(
                        $game.common.player.as_mut(),
                        causing,
                        &$game.common.potions.effects,
                        acting_equipment!($game.common),
                    )
                    .or_else(|| {
                        sunrise_sim::health::HitLend::parked_guest(
                            $game.server.lan.as_ref().map(|lan| &lan.hub),
                            parked?,
                        )
                    })
                } else {
                    None
                },
            ),
        }
    }};
}
#[allow(unused_imports)]
pub(in crate::game) use victim_and_causer;
/// The causing entity a hit on the player in place lends its listeners, as `$decision`
/// ([`causer_decision`]) decided: a mob of `$game.mobs`, or a parked guest's record. The player
/// causing its own hit is not lent again: the listener reads it as the victim.
macro_rules! player_victim_causer {
    ($game:ident, $decision:expr) => {{
        let (causing, _, mob, parked): (i32, bool, Option<usize>, Option<u32>) = $decision;
        match mob {
            // The decision may be older than this level (the player's hurt builds three): the
            // row must still be the causer.
            Some(index) => $game
                .common
                .mobs
                .get_mut(index)
                .filter(|m| m.id as i32 == causing)
                .and_then(sunrise_sim::health::HitLend::mob),
            None => parked.and_then(|conn| {
                sunrise_sim::health::HitLend::parked_guest(
                    $game.server.lan.as_ref().map(|lan| &lan.hub),
                    conn,
                )
            }),
        }
    }};
}
#[allow(unused_imports)]
pub(in crate::game) use player_victim_causer;
/// The level `$game` ticks, lent to a hit's damage hooks with its block registry and merged
/// tag tables, so a mob or the player beside them can take the hit. `$attack` is the
/// [`Attack`](sunrise_client::health::Attack) the site took first. Every site names what it
/// lends, in one place: its credit (a per-victim plan's loan, `prepare_knockback_credit`, or
/// `no_push` for a site that never makes a living push: a damage type tagged `no_knockback`, or
/// a check that pushes nothing), and the entity whose own random its listeners may draw
/// (`$lend`: the attacker the site holds a live borrow of, built with `HitLend`, or `None` where
/// it cannot lend one).
macro_rules! damage_level_of {
    ($game:ident, $attack:expr, no_push, $lend:expr $(, $equipment:expr)?) => {
        &mut sunrise_sim::health::DamageLevel::of(
            &mut $game.common.world,
            $game.common.pack.assets.registry,
            Some(server_world_items!($game)),
            Some(&$game.common.pack.block_registries),
            &$game.common.damage_hooks,
        )
        .attacked_by($attack)
        .no_push()
        .with_variants(world_variants!($game))
        .with_mob_hurt_handoff(&mut $game.server.paused_mob_hurt.0)
        .with_effects(crate::game::entity_server::hit_effects_of!($game, $lend))
        .with_reads(crate::game::entity_server::hit_reads_of!($game))
        $(.with_player_equipment($equipment))?
    };
    ($game:ident, $attack:expr, $credit:expr, $lend:expr) => {
        &mut sunrise_sim::health::DamageLevel::of(
            &mut $game.common.world,
            $game.common.pack.assets.registry,
            Some(server_world_items!($game)),
            Some(&$game.common.pack.block_registries),
            &$game.common.damage_hooks,
        )
        .attacked_by($attack)
        .with_knockback_credit($credit)
        .with_variants(world_variants!($game))
        .with_mob_hurt_handoff(&mut $game.server.paused_mob_hurt.0)
        .with_effects(crate::game::entity_server::hit_effects_of!($game, $lend))
        .with_reads(crate::game::entity_server::hit_reads_of!($game))
    };
}
#[allow(unused_imports)]
pub(in crate::game) use damage_level_of;
/// `$game`'s [`PlayerResolver`](sunrise_client::health::PlayerResolver),
/// borrowing only the fields it reads (`Game::acting_uuid`'s).
macro_rules! player_resolver {
    ($game:ident) => {
        sunrise_sim::health::PlayerResolver {
            in_place: $game.common.player.is_some().then(|| {
                $game
                    .server
                    .lan_acting_uuid
                    .map(sunrise_sim::lan::playerdata::uuid_ints)
                    .or($game.common.level.player_uuid)
                    .unwrap_or_default()
            }),
            hub: $game.server.lan.as_ref().map(|lan| &lan.hub),
        }
    };
}
#[allow(unused_imports)]
pub(in crate::game) use hit_effects_of;
#[allow(unused_imports)]
pub(in crate::game) use player_resolver;

macro_rules! walk_lists_mut {
    (($lists:expr, $entity:ident, $body:block)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $($lists.$field.for_each_mut(|$entity| $body);)*
    };
}
#[allow(unused_imports)]
pub(crate) use walk_lists_mut;

/// [`for_each_entity`] over [`TakenEntities`], with the entity mutable.
macro_rules! for_each_entity_mut {
    ($lists:expr, $entity:ident => $body:block) => {
        sunrise_gameplay_common::game::entity::entity_lists!(
            $crate::game::entity_server::walk_lists_mut,
            $lists,
            $entity,
            $body
        )
    };
}
#[allow(unused_imports)]
pub(in crate::game) use for_each_entity_mut;

macro_rules! save_lists {
    (($game:expr, $keys:expr, $out:expr, $failed:expr, $include_transient:expr)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $(for i in 0..$game$(.$list)+.len() {
            let id = $game$(.$list)+[i].id();
            if ($include_transient || !$game.persist.transient.contains(&id)) {
                $game.persist.attachment_encode_error = None;
                if let Some((key, mut tag)) = <$ty as $crate::game::entity_server::ServerEntity>::save_nbt($game, i, $keys) {
                    if !<$ty as $crate::game::entity_server::ServerEntity>::SERIALIZES_ATTACHMENTS {
                        $game.write_entity_attachments(&mut tag, id);
                    }
                    if let Some(error) = $game.persist.attachment_encode_error.take() {
                        $failed.entry(key).or_insert(error);
                    } else {
                        $out.entry(key).or_default().push((id, $game$(.$list)+[i].join_seq(), tag));
                    }
                } else {
                    $game.persist.attachment_encode_error = None;
                }
            }
        })*
    };
}
#[allow(unused_imports)]
pub(crate) use save_lists;

/// Saves every entity of every list in `entity_lists!` into `$out` by chunk,
/// each with its id and join number (`$out` maps a chunk to `(id, JoinSeq, Tag)`),
/// each through its own `ServerEntity::save_nbt`: one loop per list, static dispatch,
/// nothing allocated beyond the tags. A kind added to the lists cannot be left
/// out, since `save_nbt` has no default.
macro_rules! save_entities_of_every_kind {
    ($game:expr, $keys:expr, $out:expr, $failed:expr, $include_transient:expr) => {
        sunrise_gameplay_common::game::entity::entity_lists!(
            $crate::game::entity_server::save_lists,
            $game,
            $keys,
            $out,
            $failed,
            $include_transient
        )
    };
}
#[allow(unused_imports)]
pub(in crate::game) use save_entities_of_every_kind;

/// A matched kind consumes only its own explicit token; failed kind probes never do.
macro_rules! load_with_transfer {
    ($game:expr, $state:expr, $ordinary:ident, $transfer:ident, $($arg:expr),+ $(,)?) => {{
        if let Some(state) = $state.take() {
            $game.$transfer($($arg),+, Some(state));
        } else {
            $game.$ordinary($($arg),+);
        }
    }};
}
#[allow(unused_imports)]
pub(in crate::game) use load_with_transfer;

macro_rules! load_lists {
    (($game:expr, $kind:expr, $uuid:expr, $player:expr, $tag:expr, $transferred:expr)
     $($kind_name:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {{
        let mut tag = $tag;
        $(match <$ty as $crate::game::entity_server::ServerEntity>::load_nbt($game, $kind, $uuid, $player, tag, $transferred) {
            Ok(()) => return Ok(()),
            Err(back) => tag = back,
        })*
        Err(tag)
    }};
}
#[allow(unused_imports)]
pub(crate) use load_lists;

/// Loads a saved entity as the first kind in `entity_lists!` that claims its
/// type id, through that kind's own `ServerEntity::load_nbt`: one test per kind,
/// static dispatch, nothing allocated (the compound moves through). Evaluates
/// to `Err(tag)` when no kind claims it, and `return`s `Ok` from the caller
/// otherwise. A kind added to the lists cannot be left out, since `load_nbt`
/// has no default.
macro_rules! load_entity_of_any_kind {
    ($game:expr, $kind:expr, $uuid:expr, $player:expr, $tag:expr, $transferred:expr) => {
        sunrise_gameplay_common::game::entity::entity_lists!(
            $crate::game::entity_server::load_lists,
            $game,
            $kind,
            $uuid,
            $player,
            $tag,
            $transferred
        )
    };
}
#[allow(unused_imports)]
pub(in crate::game) use load_entity_of_any_kind;

/// The type ids the minecart kind loads.
pub(in crate::game::server_side) fn loads_as_cart(kind: &str) -> bool {
    super::minecarts::CartKind::from_id(kind).is_some()
}
/// The type id the primed TNT kind loads.
pub(in crate::game::server_side) fn loads_as_tnt(kind: &str) -> bool {
    kind == "minecraft:tnt"
}

impl crate::game::ServerGame<'_> {
    /// The type ids the mob kind loads.
    pub(in crate::game::server_side) fn loads_as_mob(&self, kind: &str) -> bool {
        self.common.res.mob_types().contains_key(kind)
    }
    /// The type ids the boat kind loads.
    pub(in crate::game::server_side) fn loads_as_boat(&self, kind: &str) -> bool {
        sunrise_gameplay_common::game::boats_common::registered_boat(
            kind,
            &self.common.res.items().component_registry,
        )
        .is_some()
    }
    /// Whether `load_listed` loads `kind` as a vehicle that takes its mob
    /// riders (`take_mob_passengers`): a mob, boat, minecart or primed TNT.
    pub(in crate::game::server_side) fn listed_kind_takes_riders(&self, kind: &str) -> bool {
        self.loads_as_mob(kind)
            || self.loads_as_boat(kind)
            || loads_as_cart(kind)
            || loads_as_tnt(kind)
    }
    /// Loads a saved entity as the first kind in `entity_lists!` that claims
    /// its type id: `Err` hands the compound back when none does.
    pub fn load_listed(
        &mut self,
        kind: &str,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        tag: Tag,
    ) -> Result<(), Tag> {
        load_entity_of_any_kind!(self, kind, uuid, player_uuid, tag, &mut None)
    }
    /// Restore a live portal object with its explicitly captured runtime ownership.
    /// This dispatch covers only reconstructible portal kinds; ordinary disk loads
    /// continue through the canonical kind walk without transfer state.
    pub fn load_listed_transferred(
        &mut self,
        kind: &str,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        tag: Tag,
        state: super::entity_initialization::TransferredEntityState,
    ) -> Result<(), Tag> {
        let mut state = Some(state);
        load_entity_of_any_kind!(self, kind, uuid, player_uuid, tag, &mut state)
            .or_else(|tag| {
                self.load_unlisted_transferred(
                    Self::chunk_of(sunrise_sim::entity_nbt::position(&tag).unwrap_or_default()),
                    kind,
                    uuid,
                    tag,
                    &mut state,
                )
            })
            .or_else(|tag| {
                let saved = tag.clone();
                self.load_other_transferred(tag, kind, state.take())
                    .map(|_| ())
                    .ok_or(saved)
            })
    }

    /// The ids of the entities that are not on the walk of `entity_lists!`.
    pub fn entity_ids_off_walk(&self) -> impl Iterator<Item = u32> + '_ {
        self.common
            .cushions
            .iter()
            .map(|e| e.id)
            .chain(self.common.item_frames.iter().map(|e| e.id))
            .chain(self.common.paintings.iter().map(|e| e.id))
            .chain(self.server.others.0.iter().map(|e| e.id))
            .chain(self.common.breath_clouds.iter().map(|e| e.id))
            .chain(self.common.bolts.iter().map(|e| e.id))
            .chain(self.common.fangs.list.iter().map(|e| e.id))
            .chain(self.common.crystals.iter().map(|e| e.id))
            .chain(self.common.dragon.iter().map(|e| e.id))
            .chain(
                self.common
                    .dragon
                    .iter()
                    .filter(|dragon| dragon.id != 0)
                    .flat_map(|dragon| {
                        (0..dragon.parts.len()).map(move |offset| {
                            sunrise_gameplay_common::game::dragon_common::dragon_part_id(
                                dragon.id, offset,
                            )
                        })
                    }),
            )
            .chain(self.common.ominous_spawners.iter().map(|e| e.id))
            .chain(self.common.resident_entities.knots.iter().map(|e| e.id))
            // Cast hooks live with their players, not in the saveable lists.
            // Keep both their identities and attachment holders until they leave.
            .chain(self.fishing_hooks_here().map(|(hook, _)| hook.id()))
            .chain(self.natural_knot_id_iter())
    }
    /// The ids of every entity in the level that is not removed: one walk.
    pub(in crate::game::server_side) fn live_entity_ids(&self) -> HashSet<u32> {
        let mut alive: HashSet<u32> = self.entity_ids_off_walk().collect();
        for_each_entity!(entity_lists_of!(self), entity => {
            if !entity.is_removed() {
                alive.insert(entity.id());
            }
        });
        alive
    }
    /// Saves every entity of every kind that lies in `keys` into `out`, by chunk.
    pub fn save_every_entity(
        &mut self,
        keys: &HashSet<ChunkKey>,
        out: &mut HashMap<ChunkKey, Vec<(u32, JoinSeq, Tag)>>,
        failed: &mut HashMap<ChunkKey, sunrise_mod_runtime::attachments::AttachmentEncodeError>,
        include_transient: bool,
    ) {
        save_entities_of_every_kind!(self, keys, out, failed, include_transient);
    }
}

impl ServerEntity for Mob {
    const SERIALIZES_ATTACHMENTS: bool = true;
    fn save_nbt(
        game: &mut crate::game::ServerGame<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_mob(index, keys)
    }
    fn load_nbt(
        game: &mut crate::game::ServerGame<'_>,
        kind: &str,
        _uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !game.loads_as_mob(kind) {
            return Err(tag);
        }
        load_with_transfer!(game, transferred, load_mob, load_mob_transferred, tag, None);
        Ok(())
    }
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((self.width(), self.height()))
    }
    fn sweeps_portals(&self) -> bool {
        true
    }
    fn can_use_portal(&self, game: &crate::game::ServerGame<'_>) -> bool {
        game.mob_portal_eligible(self)
    }
}

impl ServerEntity for ItemEntity {
    fn save_nbt(
        game: &mut crate::game::ServerGame<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_item(index, keys)
    }
    fn load_nbt(
        game: &mut crate::game::ServerGame<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:item") {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_item,
            load_item_transferred,
            tag,
            uuid
        );
        Ok(())
    }
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.25, 0.25))
    }
}

impl ServerEntity for ExperienceOrb {
    fn save_nbt(
        game: &mut crate::game::ServerGame<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_orb(index, keys)
    }
    fn load_nbt(
        game: &mut crate::game::ServerGame<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:experience_orb") {
            return Err(tag);
        }
        load_with_transfer!(game, transferred, load_orb, load_orb_transferred, tag, uuid);
        Ok(())
    }
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.5, 0.5))
    }
}

impl ServerEntity for Boat {
    fn save_nbt(
        game: &mut crate::game::ServerGame<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_boat(index, keys)
    }
    fn load_nbt(
        game: &mut crate::game::ServerGame<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !game.loads_as_boat(kind) {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_boat,
            load_boat_transferred,
            tag,
            uuid,
            kind
        );
        Ok(())
    }
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((
            super::boats_common::WIDTH as f32,
            super::boats_common::HEIGHT as f32,
        ))
    }
    fn can_use_portal(&self, game: &crate::game::ServerGame<'_>) -> bool {
        game.vehicle_can_use_portal(self.id)
    }
}

impl ServerEntity for Minecart {
    fn save_nbt(
        game: &mut crate::game::ServerGame<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_cart(index, keys)
    }
    fn load_nbt(
        game: &mut crate::game::ServerGame<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !loads_as_cart(kind) {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_cart,
            load_cart_transferred,
            tag,
            uuid,
            kind
        );
        Ok(())
    }
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((
            super::minecarts::WIDTH as f32,
            super::minecarts::HEIGHT as f32,
        ))
    }
    fn can_use_portal(&self, game: &crate::game::ServerGame<'_>) -> bool {
        game.vehicle_can_use_portal(self.id)
    }
}

impl ServerEntity for ArmorStand {
    fn save_nbt(
        game: &mut crate::game::ServerGame<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_armor_stand(index, keys)
    }
    fn load_nbt(
        game: &mut crate::game::ServerGame<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:armor_stand") {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_armor_stand,
            load_armor_stand_transferred,
            tag,
            uuid
        );
        Ok(())
    }
    /// `ArmorStand.getDefaultDimensions`: the box a portal sweep follows, and
    /// the size its exit is placed by. `ArmorStand.readAdditionalSaveData`
    /// makes a stand without `hasPhysics` (a marker or a `NoGravity` one)
    /// `noPhysics`, so `Entity.isAffectedByBlocks` never runs its sweep and it
    /// enters no portal; a marker's box is `MARKER_DIMENSIONS`, 0 by 0, too.
    fn portal_size(&self) -> Option<(f32, f32)> {
        if self.marker || self.no_gravity {
            return None;
        }
        let (width, height) = armor_stand_size(self);
        Some((width as f32, height as f32))
    }
}

impl ServerEntity for PrimedTnt {
    fn save_nbt(
        game: &mut crate::game::ServerGame<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_tnt(index, keys)
    }
    fn load_nbt(
        game: &mut crate::game::ServerGame<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !loads_as_tnt(kind) {
            return Err(tag);
        }
        load_with_transfer!(game, transferred, load_tnt, load_tnt_transferred, tag, uuid);
        Ok(())
    }
    /// `PrimedTnt` is 0.98 across.
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.98, 0.98))
    }
    fn can_use_portal(&self, game: &crate::game::ServerGame<'_>) -> bool {
        game.tnt_can_use_portal(self.id)
    }
}

impl ServerEntity for FallingBlock {
    fn save_nbt(
        game: &mut crate::game::ServerGame<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_falling(index, keys)
    }
    fn load_nbt(
        game: &mut crate::game::ServerGame<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:falling_block") {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_falling,
            load_falling_transferred,
            tag,
            uuid
        );
        Ok(())
    }
}

impl ServerEntity for Projectile {
    fn portal_after_move(&self) -> bool {
        !matches!(
            self.kind,
            sunrise_sim::projectiles::ProjectileKind::LlamaSpit
                | sunrise_sim::projectiles::ProjectileKind::ShulkerBullet { .. }
        )
    }
    fn save_nbt(
        game: &mut crate::game::ServerGame<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_projectile(index, keys)
    }
    fn load_nbt(
        game: &mut crate::game::ServerGame<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        match kind {
            "minecraft:arrow" | "minecraft:spectral_arrow" => {
                load_with_transfer!(
                    game,
                    transferred,
                    load_arrow,
                    load_arrow_transferred,
                    tag,
                    uuid,
                    player_uuid
                )
            }
            "minecraft:llama_spit" | "minecraft:shulker_bullet" => {
                load_with_transfer!(
                    game,
                    transferred,
                    load_spit_or_bullet,
                    load_spit_or_bullet_transferred,
                    tag,
                    uuid,
                    player_uuid
                )
            }
            "minecraft:fireball"
            | "minecraft:small_fireball"
            | "minecraft:dragon_fireball"
            | "minecraft:wind_charge"
            | "minecraft:breeze_wind_charge"
            | "minecraft:wither_skull" => load_with_transfer!(
                game,
                transferred,
                load_fireball,
                load_fireball_transferred,
                tag,
                uuid,
                player_uuid
            ),
            id if sunrise_sim::entity_nbt::throwable_kind(id).is_some() => {
                load_with_transfer!(
                    game,
                    transferred,
                    load_throwable,
                    load_throwable_transferred,
                    tag,
                    uuid,
                    player_uuid
                )
            }
            _ => return Err(tag),
        }
        Ok(())
    }
    /// Arrows and the thrown items and fireballs a chunk saves.
    fn portal_size(&self) -> Option<(f32, f32)> {
        (super::nether_portal::is_arrow(self) || super::nether_portal::is_portal_projectile(self))
            .then(|| super::nether_portal::projectile_box(self))
    }
    fn portal_from(&self) -> Option<Vec3> {
        Some(self.previous)
    }
    fn portal_sweeps(&self) -> bool {
        !super::nether_portal::is_arrow(self) || !self.stuck || self.previous != self.position
    }
}

impl ServerEntity for Rocket {
    fn save_nbt(
        game: &mut crate::game::ServerGame<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_rocket(index, keys)
    }
    fn load_nbt(
        game: &mut crate::game::ServerGame<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:firework_rocket") {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_rocket,
            load_rocket_transferred,
            tag,
            uuid,
            player_uuid
        );
        Ok(())
    }
    /// A rocket that carries its owner stays.
    fn portal_size(&self) -> Option<(f32, f32)> {
        (self.attached_to.is_none() && !self.attached).then_some((0.25, 0.25))
    }
    fn portal_from(&self) -> Option<Vec3> {
        Some(self.previous)
    }
}

impl ServerEntity for ThrownTrident {
    fn portal_after_move(&self) -> bool {
        true
    }
    fn save_nbt(
        game: &mut crate::game::ServerGame<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_trident(index, keys)
    }
    fn load_nbt(
        game: &mut crate::game::ServerGame<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:trident") {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_trident,
            load_trident_transferred,
            tag,
            uuid
        );
        Ok(())
    }
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.5, 0.5))
    }
    fn portal_from(&self) -> Option<Vec3> {
        Some(self.previous)
    }
    fn portal_sweeps(&self) -> bool {
        !self.in_ground || self.previous != self.position
    }
}

#[cfg(test)]
pub(in crate::game) mod no_gravity_stand_tests {
    use super::Entity as _;
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::entities::ArmorStand;

    #[test]
    fn a_no_gravity_stand_is_not_swept_for_the_blocks_it_is_in() {
        let mut stand = ArmorStand::new(1, Vec3::new(0.5, 100.0, 0.5), 0.0);
        // A resting stand presses a plate (measured beside the real server)...
        assert!(stand.sweeps_blocks_inside() && stand.triggers_blocks());
        // ...a NoGravity one does not, nor does a marker.
        stand.no_gravity = true;
        assert!(!stand.sweeps_blocks_inside());
        stand.no_gravity = false;
        stand.marker = true;
        assert!(!stand.triggers_blocks());
    }
}

#[cfg(test)]
pub(in crate::game) mod load_claim_tests {
    /// `Mob` is first in `entity_lists!` and claims every id in `mob_types`;
    /// the old load tried every other kind first. Both agree only while no
    /// mob species shares an id with another kind, which this checks.
    #[test]
    fn no_mob_species_is_an_id_another_kind_loads() {
        let others = [
            "minecraft:item",
            "minecraft:experience_orb",
            "minecraft:arrow",
            "minecraft:spectral_arrow",
            "minecraft:llama_spit",
            "minecraft:shulker_bullet",
            "minecraft:fireball",
            "minecraft:small_fireball",
            "minecraft:wind_charge",
            "minecraft:breeze_wind_charge",
            "minecraft:falling_block",
            "minecraft:trident",
            "minecraft:firework_rocket",
            "minecraft:tnt",
            "minecraft:armor_stand",
            "minecraft:ominous_item_spawner",
            "minecraft:ender_dragon",
            "minecraft:end_crystal",
            "minecraft:item_frame",
            "minecraft:glow_item_frame",
            "minecraft:cushion",
            "minecraft:painting",
        ];
        for def in sunrise_sim::mobs::species::ALL {
            let id = format!("minecraft:{}", def.name);
            assert!(!others.contains(&id.as_str()), "{id}");
            assert!(
                sunrise_sim::entity_nbt::throwable_kind(&id).is_none(),
                "{id}"
            );
            assert!(
                sunrise_gameplay_common::game::boats_common::any_boat_wood(&id).is_none(),
                "{id}"
            );
            assert!(
                super::super::minecarts::CartKind::from_id(&id).is_none(),
                "{id}"
            );
        }
    }
}

#[cfg(test)]
pub(in crate::game) mod trident_selector_tests {
    use super::Entity as _;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::commands_common::Target;
    use sunrise_sim::thrown_trident::ThrownTrident;

    /// `@e`, `/kill` and `/data` reach a thrown trident like any other entity
    /// (`EntityTypeTest.forClass(Entity.class)`: every entity in the level).
    #[test]
    fn a_thrown_trident_is_a_selector_target() {
        let t = ThrownTrident::remote(9, Vec3::new(0.5, 100.0, 0.5), 0.0, 0.0);
        assert_eq!(t.selector_target(), Some(Target::Trident(9)));
    }
}

#[cfg(test)]
pub(in crate::game) mod join_order_tests {
    use sunrise_gameplay_common::game::server_side::entity_server::join_order_tests::{at, mixed};

    use sunrise_gameplay_common::game::entity::{EntityHandle, JoinOrderError};
    use sunrise_sim::entities::ArmorStand;
    use sunrise_sim::join_seq::JoinList;

    /// The walk visits entities in the order they joined, not list by list.
    #[test]
    fn mixed_kinds_are_visited_in_join_order() {
        let (lists, joined) = mixed();
        assert_eq!(lists.walk().unwrap(), joined);
        // Walked list by list they would come in another order.
        assert_ne!(joined[0], EntityHandle::ArmorStand(0));
    }

    /// An entity taken out and put back (a tick takes its list) keeps its place.
    #[test]
    fn an_entity_put_back_keeps_its_join_number() {
        let (mut lists, joined) = mixed();
        let taken = lists.tridents.lift();
        let before: Vec<_> = taken
            .iter()
            .map(sunrise_sim::join_seq::Joined::join_seq)
            .collect();
        let mut back = JoinList::default();
        for t in taken {
            back.push(t);
        }
        lists.tridents = back;
        let after: Vec<_> = lists
            .tridents
            .iter()
            .map(sunrise_sim::join_seq::Joined::join_seq)
            .collect();
        assert_eq!(before, after);
        assert_eq!(lists.walk().unwrap(), joined);
    }

    /// The walk is checked: a list out of join order, or an entity that never
    /// went through the list's `push`, is reported and not walked past.
    #[test]
    fn the_walk_reports_a_list_out_of_order_and_an_entity_that_never_joined() {
        let (mut lists, _) = mixed();
        lists.tridents.swap(0, 1);
        assert!(matches!(lists.walk(), Err(JoinOrderError::OutOfOrder(_))));
        let (mut lists, _) = mixed();
        lists.stands.push(ArmorStand::new(9, at(), 0.0));
        lists.stands[2].join = sunrise_sim::join_seq::JoinSeq::UNJOINED;
        assert_eq!(
            lists.walk(),
            Err(JoinOrderError::Unjoined(EntityHandle::ArmorStand(2)))
        );
    }
}

#[cfg(test)]
pub(in crate::game) mod set_rotation_tests {
    use super::Entity as _;
    use sunrise_sim::entities::ExperienceOrb;

    /// `Entity.teleportSetPosition`: yaw as given, pitch clamped to -90..90, on
    /// every kind (the ones with no field of their own used to drop it).
    #[test]
    fn set_rotation_keeps_yaw_and_clamps_pitch_on_kinds_that_had_none() {
        let mut orb = ExperienceOrb {
            size_answer: None,
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            join: Default::default(),
            no_gravity: false,
            id: 0,
            position: sunrise_core::geometry::Vec3::ZERO,
            previous: sunrise_core::geometry::Vec3::ZERO,
            velocity: sunrise_core::geometry::Vec3::ZERO,
            value: 1,
            age: 0,
            ground: Default::default(),
            count: 1,
            ticks: 0,
            health: 5,
            air: 300,
            rotation: [0.0, 0.0],
            following: None,
            fire_ticks: 0,
        };
        orb.set_rotation([90.0, 10.0]);
        assert_eq!(orb.rotation(), [90.0, 10.0]);
        orb.set_rotation([0.0, 200.0]);
        assert_eq!(orb.rotation(), [0.0, 90.0]);
    }
}

#[cfg(test)]
pub(in crate::game) mod scoped_edit_tests {
    use super::*;
    use sunrise_gameplay_common::game::entity::entity_access;

    #[test]
    fn scoped_walk_skips_only_one_entity_and_keeps_later_kinds() {
        let mut lists = TakenEntities {
            mobs: Default::default(),
            dropped: Default::default(),
            boats: Default::default(),
            carts: Default::default(),
            armor_stands: Default::default(),
            tnts: Default::default(),
            falling: Default::default(),
            orbs: Default::default(),
            projectiles: Default::default(),
            rockets: Default::default(),
            tridents: Default::default(),
            general: Default::default(),
        };
        lists
            .boats
            .push(Boat::new(1, "oak".into(), Vec3::ZERO, 0.0));
        lists
            .boats
            .push(Boat::new(2, "oak".into(), Vec3::ZERO, 0.0));
        lists.carts.push(Minecart::new(
            3,
            super::super::minecarts::CartKind::Rideable,
            Vec3::ZERO,
        ));
        let mut visited = Vec::new();
        for_each_entity_mut!(lists, entity => {
            if entity.id() == 1 {
                return;
            }
            visited.push(entity.id());
            entity.velocity_mut().x = 4.0;
        });
        assert_eq!(visited, [2, 3]);
        assert_eq!(lists.boats[0].velocity.x, 0.0);
        assert_eq!(lists.boats[1].velocity.x, 4.0);
        assert_eq!(lists.carts[0].velocity.x, 4.0);
        let previous = entity_access!(exclusive, lists.boats, 1, boat, {
            let previous = boat.velocity.x;
            boat.velocity.x = 7.0;
            previous
        });
        assert_eq!(previous, 4.0);
        assert_eq!(lists.boats[1].velocity.x, 7.0);
    }
}

use sunrise_gameplay_common::game::block_props::BlockPropsGameExt2 as _;

#[cfg(any(test, feature = "test-support"))]
#[path = "entity/taken_pass_tests.rs"]
pub mod taken_pass_tests;

#[cfg(test)]
mod vehicle_contact_hurt_tests {
    use super::*;
    use crate::game::{ServerGame, native_entity_refs::tests::with_game};
    use mod_api::{self, BlockPart, Hooks, ModEntry, Registrar, RegistrationContext};
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_core::nbt::{Compound, Tag};
    use sunrise_level::chunk::Chunk;

    fn boat(id: u32) -> Boat {
        Boat::new(id, Arc::from("oak"), Vec3::new(8.5, 64.0, 8.5), 0.0)
    }

    fn cart(id: u32) -> super::super::minecarts::Minecart {
        super::super::minecarts::Minecart::new(
            id,
            super::super::minecarts::CartKind::Rideable,
            Vec3::new(8.5, 64.0, 8.5),
        )
    }

    #[derive(Clone)]
    struct HullHeat {
        damage_type: mod_api::DamageTypeKey,
        amount: f32,
    }

    #[mod_api::sunrise_part]
    impl BlockPart for HullHeat {
        fn step_on(&self, _: mod_api::LevelRandomSide<'_>, event: &mut mod_api::StepOnEvent<'_>) {
            event.write_entity(mod_api::EntityWrite::Hurt {
                damage_type: self.damage_type,
                amount: self.amount,
            });
        }
    }

    #[derive(Clone)]
    struct HullHeatBlocks {
        amount: f32,
    }

    #[mod_api::__subscriptions]
    impl Hooks for HullHeatBlocks {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let stone = r.block_key("minecraft:stone");
            let damage_type = r.damage_type_key("minecraft:cactus");
            r.block_part(
                stone,
                HullHeat {
                    damage_type,
                    amount: self.amount,
                },
            );
        }
    }

    /// A stone floor with a `step_on` Part that hurts the stepping body by
    /// `amount` (ten times the hull wear, as `VehicleEntity.hurtServer` does).
    fn with_hull_heat(amount: f32, test: impl FnOnce(&mut ServerGame<'_>)) {
        let mods: mod_api::Mods = vec![ModEntry::new(
            "hull_heat",
            Box::new(HullHeatBlocks { amount }),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            sunrise_gameplay_common::game::native_hooks::bounce_restitution::install_block_contact_hooks();
            with_game(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(8, 63, 8, stone).unwrap();
                test(game);
            });
        });
    }

    /// A boat's `step_on` contact reaches the stored hull through its real
    /// tick: the wear and hurt wobble survive the ticked copy's store-back.
    #[test]
    #[ignore = "daily"]
    fn a_surviving_contact_hurt_reaches_a_boat_through_its_tick() {
        with_hull_heat(1.0, |game| {
            let mut b = boat(1);
            b.ground.on_ground = true;
            game.boats.push(b);
            game.tick_boats();
            assert_eq!(game.boats.len(), 1);
            assert!(
                game.boats[0].damage > 0.0,
                "the contact wears the stored hull"
            );
            assert_eq!(game.boats[0].hurt_time, 10);
        });
    }

    /// A cart's `step_on` contact reaches the stored cart through its real tick.
    #[test]
    #[ignore = "daily"]
    fn a_surviving_contact_hurt_reaches_a_cart_through_its_tick() {
        with_hull_heat(1.0, |game| {
            let mut c = cart(2);
            c.ground.on_ground = true;
            game.carts.push(c);
            game.tick_carts();
            assert_eq!(game.carts.len(), 1);
            assert!(
                game.carts[0].damage > 0.0,
                "the contact wears the stored cart"
            );
            assert_eq!(game.carts[0].hurt_time, 10);
        });
    }

    /// A breaking contact hurt removes the boat and stops the rest of its
    /// tick: no later pass indexes the removed row.
    #[test]
    #[ignore = "daily"]
    fn a_breaking_contact_hurt_removes_a_boat_through_its_tick() {
        with_hull_heat(5.0, |game| {
            let mut b = boat(1);
            b.ground.on_ground = true;
            game.boats.push(b);
            game.tick_boats();
            assert!(game.boats.is_empty());
        });
    }

    /// A hit that survives the first `step_on` and breaks the boat on the
    /// second pass must stop the tick before the contacts pass indexes the
    /// removed row (the breaking-boat test above stops on the first pass).
    #[test]
    #[ignore = "daily"]
    fn a_second_pass_breaking_hurt_removes_a_boat_without_a_later_pass() {
        with_hull_heat(2.5, |game| {
            let mut b = boat(1);
            b.ground.on_ground = true;
            game.boats.push(b);
            game.tick_boats();
            assert!(game.boats.is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_breaking_contact_hurt_removes_a_cart_through_its_tick() {
        with_hull_heat(5.0, |game| {
            let mut c = cart(2);
            c.ground.on_ground = true;
            game.carts.push(c);
            game.tick_carts();
            assert!(game.carts.is_empty());
        });
    }

    /// The same contact on an `Invulnerable` hull is refused before
    /// `hurtServer`: no wear, no hurt wobble, nothing to sound.
    #[test]
    #[ignore = "daily"]
    fn an_invulnerable_vehicle_refuses_a_contact_hurt() {
        with_game(|game| {
            game.boats.push(boat(1));
            game.carts.push(cart(2));
            for id in [1, 2] {
                let mut compound = Compound::new();
                compound.insert("Invulnerable".into(), Tag::Byte(1));
                game.persist.raw.insert(id, Tag::Compound(compound));
            }
            assert!(!game.vehicle_contact_hurt(Target::Boat(1), "minecraft:cactus", 2.0));
            assert!(!game.vehicle_contact_hurt(Target::Cart(2), "minecraft:cactus", 2.0));
            assert_eq!(game.boats[0].damage, 0.0);
            assert_eq!(game.carts[0].damage, 0.0);
            assert_eq!(game.boats[0].hurt_time, 0);
            assert_eq!(game.carts[0].hurt_time, 0);
        });
    }
}
