//! Player liquid travel and environmental damage over the shared fluid state.
use super::*;
use crate::breathing::{AirStep, Breath};
use sunrise_core::random::RandomSource;
use sunrise_world::{
    fluid::FluidKind,
    living_fluid::{LivingFluidWorld, TravelFluidTag},
};
impl MotionWorld<'_, '_, '_> {
    pub(super) fn environment_hurt(&mut self, source: &str, damage: f32) -> Result<()> {
        let id = self
            .health
            .damage
            .id(source)
            .ok_or("missing environment damage type")?;
        self.environment_hurt_type(id, damage)
    }

    pub(super) fn environment_hurt_type(
        &mut self,
        id: sunrise_world::damage_source::DamageTypeId,
        damage: f32,
    ) -> Result<()> {
        let rules = self.scene.health_rules;
        self.player.hurt(
            &mut crate::health::DamageLevel::of(
                self.scene,
                self.assets.registry,
                self.tag_items,
                self.block_registries,
                self.damage_hooks,
            )
            .with_player_equipment(self.side.player_equipment())
            .with_reads(self.side.hit_reads()),
            self.health,
            rules,
            id,
            damage,
        )?;
        Ok(())
    }
    pub(super) fn hurt_from_remaining_fire(&mut self) -> Result<()> {
        if self.player.spectator {
            self.player.fire_ticks = 0;
        } else if self.player.mod_fire_immune {
            self.player.fire_ticks = self.player.fire_ticks.min(0);
        }
        if self.player.invulnerable && self.player.fire_ticks > 1 {
            self.player.fire_ticks = 1;
        }
        if self.player.fire_ticks > 0
            && self.player.fire_ticks % 20 == 0
            && !self.player.is_in_lava()
        {
            self.environment_hurt("minecraft:on_fire", 1.0)?;
        }
        Ok(())
    }
    /// `LivingEntity.baseTick`'s air supply and drowning, then lava, with
    /// vanilla's breathing: a server's game stops between [`Self::decide_air`]
    /// and [`Self::apply_air`] to fire `living_breathe` with its owning host.
    pub fn tick_environment_damage(&mut self) -> Result<()> {
        if let Some(step) = self.decide_air()? {
            self.apply_air(step, step.vanilla)?;
        }
        self.tick_lava_contact()
    }
    /// What `LivingEntity.baseTick`'s air supply does to the player this tick,
    /// before the mods are asked. Air is synced entity data: only the server
    /// runs LivingEntity's air block, so a client's copy decides nothing.
    pub fn decide_air(&mut self) -> Result<Option<AirStep>> {
        if self.player.client_side {
            return Ok(None);
        }
        let eye = BlockPos::containing(self.player.position.add_vector(Vec3::new(
            0.0,
            f64::from(self.player.dimensions.eye_height),
            0.0,
        )));
        let bubble_column = self
            .assets
            .registry
            .state(self.block(eye))
            .is_some_and(|state| state.name == "minecraft:bubble_column");
        let submerged = self.player.is_eye_in_fluid(WATER) && !bubble_column;
        let drowns = submerged
            && !self.player.status.water_breathing
            && !self.player.status.breath_of_the_nautilus
            && !self.player.invulnerable
            && !self.player.spectator;
        let vanilla = if drowns {
            // decreaseAirSupply's getAttribute: the instance exists from now
            // on, even unmodified.
            let key = self.player.attributes.key("minecraft:oxygen_bonus");
            self.player
                .attributes
                .get_instance(key)
                .map_err(|e| format!("oxygen bonus: {e:?}"))?;
            let oxygen = self
                .player
                .attribute("minecraft:oxygen_bonus")?
                .max(f64::from(self.player.respiration));
            let sparing = oxygen > 0.0 && self.scene.random.next_double() >= 1.0 / (oxygen + 1.0);
            Breath {
                can_breathe: false,
                consume: i32::from(!sparing),
                refill: 0,
            }
        } else {
            // The nautilus' breath keeps a submerged player's air as it is.
            let held = submerged
                && self.player.status.breath_of_the_nautilus
                && !self.player.status.water_breathing;
            Breath {
                can_breathe: true,
                consume: 1,
                refill: if held { 0 } else { 4 },
            }
        };
        // Entity.getMaxAirSupply for a player.
        Ok(Some(AirStep::generic(self.player.air, 300, vanilla)))
    }
    /// Apply `breath` (vanilla's, or the mods' answer to `living_breathe`) to
    /// the air `step` decided on, then LivingDrownEvent. A player whose air is no
    /// longer what was decided on keeps it: the decision is never recomputed.
    pub fn apply_air(&mut self, step: AirStep, breath: Breath) -> Result<()> {
        if self.player.air != step.air {
            return Ok(());
        }
        let entity = self.player.entity_id as u32;
        if let Some(drowning) = step.finish(
            self.loop_context,
            entity,
            "minecraft:player",
            &mut self.player.air,
            breath,
        ) {
            self.player
                .events
                .push(MotionEvent::Drown(drowning.bubbles));
            if let Some(damage) = drowning.damage {
                self.player.events.push(MotionEvent::Health(
                    crate::health::HealthEvent::DrownDamage(damage),
                ));
                self.environment_hurt("minecraft:drown", damage)?;
            }
        }
        Ok(())
    }
    /// `LivingEntity.baseTick`'s lava contact, after the air.
    pub fn tick_lava_contact(&mut self) -> Result<()> {
        if self.player.is_in_lava() {
            self.lava_ignite_and_hurt()?;
        }
        Ok(())
    }
    /// `Entity.lavaIgnite` and `Entity.lavaHurt`: both check `fireImmune`.
    pub(super) fn lava_ignite_and_hurt(&mut self) -> Result<()> {
        if self.player.mod_fire_immune {
            return Ok(());
        }
        self.player.ignite_for_ticks(300)?;
        self.environment_hurt("minecraft:lava", 4.0)
    }
    /// Entity.checkBelowWorld: the void damages even a creative player.
    pub fn check_below_world(&mut self) -> Result<()> {
        if self.player.position.y < self.scene.below_world_y() {
            let id = self
                .health
                .damage
                .id("minecraft:out_of_world")
                .ok_or("missing environment damage type")?;
            let rules = self.scene.health_rules;
            self.player.hurt(
                &mut crate::health::DamageLevel::of(
                    self.scene,
                    self.assets.registry,
                    self.tag_items,
                    self.block_registries,
                    self.damage_hooks,
                )
                .with_player_equipment(self.side.player_equipment())
                .with_reads(self.side.hit_reads()),
                self.health,
                rules,
                id,
                4.0,
            )?;
        }
        Ok(())
    }
    /// LivingEntity.baseTick's eye-in-wall hurt, after the owning server's shape check.
    pub fn suffocation_hurt(&mut self) -> Result<()> {
        self.environment_hurt("minecraft:in_wall", 1.0)
    }

    /// Reset the player's fire-contact delay (`-getFireImmuneTicks()`) when
    /// not touching fire (`Entity.baseTick`). Ordinary fire ignition and hurt
    /// run inside `BaseFireBlock.entityInside`'s collector steps.
    pub fn reset_unlit_fire(&mut self) {
        if self.player.spectator {
            self.player.fire_ticks = 0;
            return;
        }
        if fire_contact(self.scene, self.assets.registry, self.player.bounds()) <= 0.0
            && self.player.fire_ticks <= 0
        {
            self.player.fire_ticks = -20;
        }
    }
}
pub(crate) fn fire_contact(
    world: &World,
    registry: &sunrise_world::registry::BlockRegistry,
    bounds: Aabb,
) -> f32 {
    let b = bounds.deflate(Vec3::new(1e-5, 1e-5, 1e-5));
    let mut damage = 0.0f32;
    crate::block_contact::box_cells(b, |p| {
        damage = damage.max(
            match registry
                .state(world.block(p.x, p.y, p.z))
                .map(|s| s.name.as_str())
            {
                Some("minecraft:fire") => 1.0,
                Some("minecraft:soul_fire") => 2.0,
                _ => 0.0,
            },
        );
        true
    });
    damage
}
/// `CactusBlock.entityInside`: the box (deflated by `1.0E-5`, as in
/// `Entity.checkInsideBlocks`) touches a cactus; its inside shape is the
/// whole block.
pub(crate) fn cactus_contact(
    world: &World,
    registry: &sunrise_world::registry::BlockRegistry,
    bounds: Aabb,
) -> bool {
    let b = bounds.deflate(Vec3::new(1e-5, 1e-5, 1e-5));
    // Stopped early: a cactus cell was found.
    !crate::block_contact::box_cells(b, |p| {
        !registry
            .state(world.block(p.x, p.y, p.z))
            .is_some_and(|s| s.name == "minecraft:cactus")
    })
}
impl LivingFluidWorld for MotionWorld<'_, '_, '_> {
    fn in_water(&mut self) -> Result<bool> {
        Ok(self.player.fluids.was_touching_water)
    }
    fn sprinting(&mut self) -> Result<bool> {
        Ok(self.player.sprinting)
    }
    fn water_slow_down(&mut self) -> Result<f32> {
        Ok(0.8)
    }
    fn water_movement_efficiency(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:water_movement_efficiency")
    }
    fn dolphins_grace(&mut self) -> Result<bool> {
        Ok(self.player.status.dolphins_grace)
    }
    fn fluid_height(&mut self, tag: TravelFluidTag) -> Result<f64> {
        Ok(self.player.fluid_height(if tag == TravelFluidTag::Lava {
            LAVA
        } else {
            WATER
        }))
    }
    fn fluid_jump_threshold(&mut self) -> Result<f64> {
        Ok(self.player.fluid_jump_threshold())
    }
    fn is_free(&mut self, v: Vec3) -> Result<bool> {
        let b = self.player.bounds().move_by(v);
        Ok(self.no_player_collision(b)? && !self.has_liquid(b)?)
    }
    fn is_vehicle(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn can_float_while_ridden(&mut self) -> Result<bool> {
        Ok(false)
    }
}
impl PlayerMotion {
    /// `Entity.getFluidJumpThreshold`: 0.4, or 0 under eyes lower than that.
    pub fn fluid_jump_threshold(&self) -> f64 {
        sunrise_world::swimming::fluid_jump_threshold(self.dimensions.eye_height)
    }
}
impl MotionWorld<'_, '_, '_> {
    /// `Player.getBlockSpeedFactor` (#16176): soul sand and honey slow the
    /// player's move only while it neither flies nor glides.
    pub(super) fn player_block_speed_factor(&mut self) -> Result<f32> {
        let efficiency = self.player.attribute("minecraft:movement_efficiency")? as f32;
        let flying = self.player.flying || self.player.fall_flying;
        let location = self.location();
        let context = self.loop_context;
        Ok(movement_support::player_block_speed_factor(
            &mut RegistryMovementWorld {
                properties: &self.resources.blocks,
                world: self,
                edits: &ModBlockFactors(context),
            },
            location,
            flying,
            efficiency,
        )?)
    }
    /// `LivingEntity.aiStep`'s jump phase: rise in water or lava, else jump
    /// from the ground (or from shallow fluid); nothing while flying.
    pub(super) fn jump_phase(&mut self) -> Result<()> {
        if !self.player.jumping || self.player.flying {
            self.player.no_jump_delay = 0;
            return Ok(());
        }
        let in_lava = self.player.is_in_lava();
        let height = self.player.fluid_height(if in_lava { LAVA } else { WATER });
        let in_water = self.player.fluids.was_touching_water && height > 0.0;
        let threshold = self.player.fluid_jump_threshold();
        let on_ground = self.player.ground.on_ground;
        if in_water && !(on_ground && height <= threshold) {
            // jumpInLiquid(WATER)
            self.player.velocity.y += f64::from(0.04f32);
        } else if in_lava && !(on_ground && self.player.fluid_height(LAVA) <= threshold) {
            // jumpInLiquid(LAVA)
            self.player.velocity.y += f64::from(0.04f32);
        } else if (on_ground || in_water && height <= threshold) && self.player.no_jump_delay == 0 {
            self.jump_from_ground()?;
            self.player.no_jump_delay = 10;
        }
        Ok(())
    }
    /// `BubbleColumnBlock.entityInside` with `Player.onAboveBubbleColumn` /
    /// `onInsideBubbleColumn`: pushed up (or dragged down), faster at the
    /// surface, never while flying.
    pub(super) fn bubble_column_inside(
        &mut self,
        state: BlockStateId,
        p: BlockPos,
        precise: bool,
    ) -> Result<()> {
        if !precise {
            return Ok(());
        }
        let drag = self
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("drag"))
            .is_some_and(|d| d == "true");
        let above = self.block(p.offset(0, 1, 0));
        // stateAbove.getCollisionShape(level, pos): the shape is asked at the
        // column's own position.
        let binding = self.collision_binding;
        let context = CollisionContext::empty(false);
        let empty = binding
            .collision_shape_with_context(&mut self.collisions(), above, p, &context)?
            .is_empty()
            && self.resources.fluids.fluid(above).kind() == FluidKind::Empty;
        if self.player.flying {
            return Ok(());
        }
        let v = self.player.velocity;
        let y = match (empty, drag) {
            (true, true) => (-0.9_f64).max(v.y - 0.03),
            (true, false) => 1.8_f64.min(v.y + 0.1),
            (false, true) => (-0.3_f64).max(v.y - 0.03),
            (false, false) => 0.7_f64.min(v.y + 0.06),
        };
        self.player.velocity = Vec3::new(v.x, y, v.z);
        if empty {
            // Entity.sendBubbleColumnParticles: two splashes and two bubbles
            // over the column's top, from the level random.
            let random = &mut self.scene.random;
            for bubble in [false, true, false, true] {
                let x = f64::from(p.x) + random.next_double();
                let z = f64::from(p.z) + random.next_double();
                self.player.events.push(MotionEvent::BubbleColumnParticle {
                    bubble,
                    position: Vec3::new(x, f64::from(p.y + 1), z),
                });
            }
        } else {
            self.player.fall_distance = 0.0;
        }
        Ok(())
    }
    fn has_liquid(&self, b: Aabb) -> Result<bool> {
        for x in b.min_position().x.floor() as i32..=b.max_position().x.floor() as i32 {
            for y in b.min_position().y.floor() as i32..=b.max_position().y.floor() as i32 {
                for z in b.min_position().z.floor() as i32..=b.max_position().z.floor() as i32 {
                    if self.fluid_at(BlockPos::new(x, y, z)).kind() != FluidKind::Empty {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
}

pub(crate) fn ignite_contact(
    ticks: i32,
    player: bool,
    random: &mut impl RandomSource,
    multiplier: f64,
) -> i32 {
    let ticks = if ticks < 0 {
        ticks + 1
    } else if player {
        ticks.wrapping_add(1 + random.next_int_bound(2).unwrap_or(0))
    } else {
        ticks
    };
    if ticks >= 0 {
        ticks.max((160.0 * multiplier).ceil() as i32)
    } else {
        ticks
    }
}

/// `LivingEntity.handleFallFlyingCollisions`: a glide a block stopped takes
/// `(before - after) * 10 - 3`, and nothing when that is not positive.
pub(super) fn fly_into_wall_damage(before: f64, after: f64) -> f32 {
    ((before - after) * 10.0 - 3.0) as f32
}

#[cfg(test)]
mod fire_ignition_tests {
    use super::*;
    #[test]
    fn fire_hurts_before_twenty_tick_ignition_and_scales_duration() {
        let mut r = sunrise_core::random::LegacyRandom::new(3);
        let mut ticks = -20;
        for _ in 0..19 {
            ticks = ignite_contact(ticks, true, &mut r, 0.4);
            assert!(ticks < 0);
        }
        assert_eq!(ignite_contact(ticks, true, &mut r, 0.4), 64);
    }
}

#[cfg(test)]
mod below_world_tests {
    use super::*;
    use crate::chunk::LevelShape;

    /// The player's health after the server hazard steps at `y` in a level of `shape`.
    fn health_after_hazards(shape: LevelShape, y: f64) -> f32 {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("SUNRISE_TEST_RESOURCES"),
        );
        let assets = Assets::load(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene_with_placements(&assets, &[], 1).unwrap();
        scene.shape = shape;
        let mut player = PlayerMotion::new(&resources, Vec3::new(0.5, y, 0.5)).unwrap();
        MotionWorld {
            side: crate::motion::dry_side(),
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets: &assets,
            collision_binding: &binding,
            resources: &resources,
            health: &resources.health,
        }
        .check_below_world()
        .unwrap();
        player.health.health()
    }

    /// `Entity.checkBelowWorld` hurts below `Level.getMinY() - 64`: y = -64 in the End and the
    /// Nether (min Y 0), y = -128 in the Overworld, whatever the chunks' widened column holds.
    #[test]
    #[ignore = "daily"]
    fn the_void_starts_64_blocks_below_each_levels_floor() {
        assert_eq!(
            health_after_hazards(LevelShape::END, -64.5),
            16.0,
            "End: below -64"
        );
        assert_eq!(
            health_after_hazards(LevelShape::NETHER, -64.5),
            16.0,
            "Nether: below -64"
        );
        assert_eq!(
            health_after_hazards(LevelShape::END, -63.5),
            20.0,
            "End: above -64"
        );
        assert_eq!(
            health_after_hazards(LevelShape::OVERWORLD, -64.5),
            20.0,
            "Overworld: above -128"
        );
        assert_eq!(
            health_after_hazards(LevelShape::OVERWORLD, -128.5),
            16.0,
            "Overworld: below -128"
        );
    }
}
