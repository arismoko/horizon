//! The player's travel family: `Player.travel`'s branch selection and the
//! glide branch, split out of `motion.rs` (no behaviour change). The
//! `living_travel` hook (#2414, PR 2) fires at this family's branch selection.
use super::{MotionEvent, MotionWorld, Result, fall_flying_movement};
use sunrise_core::{geometry::Vec3, position::BlockPos};
use sunrise_world::{living_air, move_gate::MoveGate};

#[derive(Clone, Copy)]
enum TravelBranch {
    Water,
    Lava,
    Glide,
    EndGlide,
    Air,
}

impl MotionWorld<'_, '_, '_> {
    /// Player.travel: swim steering, then LivingEntity.travel (fluids first,
    /// then gliding, then air), keeping 0.6 of the vertical speed while flying.
    pub(super) fn player_travel(&mut self, mut input: Vec3) -> Result<()> {
        if self.player.swimming {
            let look_y =
                sunrise_world::view_direction::view_vector(self.player.pitch, self.player.yaw).y;
            let near_surface = BlockPos::containing(Vec3::new(
                self.player.position.x,
                self.player.position.y + 1.0 - 0.1,
                self.player.position.z,
            ));
            if look_y <= 0.0
                || self.player.jumping
                || self.fluid_at(near_surface).kind() != sunrise_world::fluid::FluidKind::Empty
            {
                let blend = if look_y < -0.2 { 0.085 } else { 0.06 };
                self.player.velocity.y += (look_y - self.player.velocity.y) * blend;
            }
        }
        let branch = self.travel_branch()?;
        let mut velocity = self.player.velocity;
        if !crate::travel_gate::decide(
            self.loop_context,
            self.player.entity_id as u32,
            "minecraft:player",
            || {
                use sunrise_world::travel::TravelMode;
                match branch {
                    TravelBranch::Water => TravelMode::Water,
                    TravelBranch::Lava => TravelMode::Lava,
                    TravelBranch::Glide => TravelMode::Glide,
                    _ if self.player.flying => TravelMode::Flying,
                    _ => TravelMode::Air,
                }
            },
            &mut input,
            &mut velocity,
        ) {
            return Ok(());
        }

        self.player.set_velocity(velocity);
        // Creative flight damps the accepted vertical speed, not the speed
        // left by the branch after the move.
        let flying_y = self.player.flying.then_some(self.player.velocity.y);
        self.apply_travel_branch(branch, &mut MoveGate::new(), input)?;
        if let Some(y) = flying_y {
            self.player.set_velocity(Vec3::new(
                self.player.velocity.x,
                y * 0.6,
                self.player.velocity.z,
            ));
        }
        Ok(())
    }

    /// `LivingEntity.travel` without `Player.travel`'s swim steering, hook and
    /// flight damping (the rider's route).
    pub(super) fn living_travel(&mut self, gate: &mut MoveGate, input: Vec3) -> Result<()> {
        let branch = self.travel_branch()?;
        self.apply_travel_branch(branch, gate, input)
    }

    fn travel_branch(&mut self) -> Result<TravelBranch> {
        Ok(
            if !self.player.flying && self.player.fluids.was_touching_water {
                TravelBranch::Water
            } else if !self.player.flying && self.player.is_in_lava() {
                TravelBranch::Lava
            } else if self.player.fall_flying {
                if self.climbable()? {
                    TravelBranch::EndGlide
                } else {
                    TravelBranch::Glide
                }
            } else {
                TravelBranch::Air
            },
        )
    }

    fn apply_travel_branch(
        &mut self,
        branch: TravelBranch,
        gate: &mut MoveGate,
        input: Vec3,
    ) -> Result<()> {
        match branch {
            TravelBranch::Water | TravelBranch::Lava => {
                sunrise_world::living_fluid::travel_in_fluid(self, gate, input.into())?;
                Ok(())
            }
            TravelBranch::Glide => self.travel_fall_flying(gate),
            TravelBranch::EndGlide | TravelBranch::Air => {
                living_air::travel_in_air(self, gate, input.into())?;
                if matches!(branch, TravelBranch::EndGlide) {
                    self.player.fall_flying = false;
                }
                Ok(())
            }
        }
    }
    /// LivingEntity.travelFallFlying with updateFallFlyingMovement.
    pub(super) fn travel_fall_flying(&mut self, gate: &mut MoveGate) -> Result<()> {
        let v = self.player.velocity;
        let move_h = (v.x * v.x + v.z * v.z).sqrt();
        let gravity = sunrise_world::living_motion::effective_gravity(self)?;
        self.player.set_velocity(fall_flying_movement(
            v,
            self.player.pitch,
            self.player.yaw,
            gravity,
        ));
        let movement = self.player.velocity;
        let made = living_air::LivingAirWorld::move_self(self, gate, movement)?;
        living_air::LivingAirWorld::travel_moved(
            self,
            sunrise_world::travel::TravelMode::Glide,
            made,
        )?;
        // LivingEntity.handleFallFlyingCollisions: the fall-damage sound plays
        // on both sides (a glider's own travel is client-side, and the sound
        // must reach its own ears, #8906), only the server hurts, and it
        // compares the horizontal speed before `updateFallFlyingMovement`
        // with the delta `Entity.move` leaves (the collided axes zeroed).
        if self.player.collision.horizontal {
            let v = self.player.velocity;
            let after_h = (v.x * v.x + v.z * v.z).sqrt();
            let dmg = super::environment::fly_into_wall_damage(move_h, after_h);
            if dmg > 0.0 {
                let first = self.player.events.len();
                // getFallDamageSound((int) dmg)
                let sound = if dmg as i32 > 4 {
                    crate::landing::SceneSound::BigFall
                } else {
                    crate::landing::SceneSound::SmallFall
                };
                self.player
                    .events
                    .push(MotionEvent::Fall(crate::landing::FallEvent::Sound {
                        sound,
                        position: None,
                        volume: 1.0,
                        pitch: 1.0,
                    }));
                // Only the server hurts; the sound is shared.
                if !self.player.client_side {
                    let id = self
                        .resources
                        .health
                        .damage
                        .id("minecraft:fly_into_wall")
                        .ok_or("missing fly_into_wall damage type")?;
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
                        dmg,
                    )?;
                    if self.player.server_copy {
                        self.player.wall_hit_events = Some(first..self.player.events.len());
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::motion::{MotionWorld, PlayerMotion, dry_side};
    use crate::world::World;
    use mod_api::{
        Flow, Hooks, LivingTravelEvent, LivingTravelMovedEvent, LoopContext, TravelMode as Mode,
    };
    use std::sync::{Arc, Mutex};
    use sunrise_core::geometry::Vec3;

    /// A listener that replaces the velocity (when it names one) and answers `Flow`.
    struct Decide(Flow, Option<[f64; 3]>);

    #[mod_api::__subscriptions]
    impl Hooks for Decide {
        fn living_travel(&self, _: &dyn LoopContext, event: &mut LivingTravelEvent<'_>) -> Flow {
            if let Some(velocity) = self.1 {
                *event.velocity = velocity;
            }
            self.0
        }
    }

    /// One `player_travel` of a flying player standing at `(20.5, 4, 20.5)` with
    /// velocity `(0.2, 0.5, 0)`, under a `living_travel` listener: the
    /// position and velocity it leaves, and where it started.
    fn flying_travel(listener: Decide, replaying: bool) -> (Vec3, Vec3, Vec3) {
        let v = crate::test_fixtures::vanilla().expect("prepared resources are required");
        let (assets, binding, resources) = (&v.assets, &v.collision, &v.motion);
        let mut scene = World::test_scene_with_placements(assets, &[], 1).unwrap();
        let start = Vec3::new(20.5, 4.0, 20.5);
        let mut player = PlayerMotion::new(resources, start).unwrap();
        player.flying = true;
        player.replaying = replaying;
        player.velocity = Vec3::new(0.2, 0.5, 0.0);
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("travel", Box::new(listener))].into();
        mod_platform::testing::with_mods(mods, || {
            MotionWorld {
                side: dry_side(),
                tag_items: None,
                block_registries: None,
                loop_context: crate::test_resources::server_loop_context(),
                damage_hooks: &crate::health::DamageHooks::NONE,
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets,
                collision_binding: binding,
                resources,
                health: &resources.health,
            }
            .player_travel(Vec3::ZERO)
            .unwrap();
        });
        (player.position, player.velocity, start)
    }

    /// A flying player keeps 0.6 of the vertical speed its branch started
    /// from: a listener's replaced `0.5` -> `1.0` leaves `0.6`, not `0.3`.
    #[test]
    #[ignore = "daily"]
    fn a_flying_player_damps_the_replaced_vertical_speed() {
        let (_, velocity, _) = flying_travel(Decide(Flow::Continue, Some([0.2, 1.0, 0.0])), false);
        assert_eq!(velocity.y, 0.6);
    }

    /// Cancel skips the branch, its move and the flying damping: the flying
    /// player keeps its position and its `0.5` vertical speed.
    #[test]
    #[ignore = "daily"]
    fn a_cancelled_flying_travel_moves_and_damps_nothing() {
        let (position, velocity, start) = flying_travel(Decide(Flow::Cancel, None), false);
        assert_eq!(position, start);
        assert_eq!(velocity, Vec3::new(0.2, 0.5, 0.0));
    }

    /// A simulated (replayed) tick fires `living_travel`: its cancel holds the
    /// travel, leaving the player at its starting position and velocity.
    #[test]
    #[ignore = "daily"]
    fn a_replayed_travel_hears_a_cancel() {
        let (position, velocity, start) = flying_travel(Decide(Flow::Cancel, None), true);
        assert_eq!(position, start);
        assert_eq!(velocity, Vec3::new(0.2, 0.5, 0.0));
    }
    /// What a `living_travel_moved` listener heard: the side, mode, displacement, ground flags
    /// and jump input of one move.
    type MovedHeard = Vec<(mod_api::LoopSide, Mode, [f64; 3], bool, bool, bool)>;

    /// A `living_travel_moved` listener that records what it hears and replaces the velocity.
    struct Moved(Option<[f64; 3]>, Arc<Mutex<MovedHeard>>);

    #[mod_api::__subscriptions]
    impl Hooks for Moved {
        fn living_travel_moved(
            &self,
            context: &dyn LoopContext,
            event: &mut LivingTravelMovedEvent<'_>,
        ) {
            self.1.lock().unwrap().push((
                context.side(),
                event.mode,
                event.moved,
                event.on_ground_before,
                event.on_ground,
                event.jumping,
            ));
            if let Some(velocity) = self.0 {
                *event.velocity = velocity;
            }
        }
    }

    /// One `player_travel` of a walking (not flying) player in the air at `(20.5, 4, 20.5)` with
    /// velocity `(0.2, 0.5, 0)` under `listener`, on the loop of `side`: the velocity it leaves.
    fn air_travel(listener: Moved, side: sunrise_world::native_load_context::LoopSide) -> Vec3 {
        let v = crate::test_fixtures::vanilla().expect("prepared resources are required");
        let (assets, binding, resources) = (&v.assets, &v.collision, &v.motion);
        let mut scene = World::test_scene_with_placements(assets, &[], 1).unwrap();
        let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        player.velocity = Vec3::new(0.2, 0.5, 0.0);
        let context = crate::test_resources::loop_context(side);
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("moved", Box::new(listener))].into();
        mod_platform::testing::with_mods(mods, || {
            MotionWorld {
                side: dry_side(),
                tag_items: None,
                block_registries: None,
                loop_context: &context,
                damage_hooks: &crate::health::DamageHooks::NONE,
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets,
                collision_binding: binding,
                resources,
                health: &resources.health,
            }
            .player_travel(Vec3::ZERO)
            .unwrap();
        });
        player.velocity
    }

    /// A velocity the listener sets right after the move is what the air branch's gravity and
    /// drag act on, the same on the server's loop and on the client's own prediction: the
    /// expected values are vanilla's `travelInAir` arithmetic on the edit (gravity `0.08`, then
    /// `0.91F` horizontal and `0.98F` vertical drag), not the game's own result.
    #[test]
    #[ignore = "daily"]
    fn a_moved_velocity_edit_is_what_drag_and_gravity_act_on() {
        use sunrise_world::native_load_context::LoopSide;
        for side in [LoopSide::Server, LoopSide::Client] {
            let heard = Arc::new(Mutex::new(Vec::new()));
            let velocity = air_travel(Moved(Some([0.5, 0.0, -0.25]), Arc::clone(&heard)), side);
            assert_eq!(
                velocity,
                Vec3::new(
                    0.5 * f64::from(0.91_f32),
                    (0.0 - 0.08) * f64::from(0.98_f32),
                    -0.25 * f64::from(0.91_f32)
                )
            );
            let heard = heard.lock().unwrap();
            assert_eq!(heard.len(), 1, "once per travel branch");
            let (_, mode, moved, on_ground_before, _, jumping) = heard[0];
            assert_eq!(mode, Mode::Air);
            assert_eq!(
                moved,
                [20.5_f64 + 0.2 - 20.5, 4.0_f64 + 0.5 - 4.0, 0.0],
                "the displacement the move made"
            );
            assert!(!on_ground_before && !jumping);
        }
    }

    /// A non-finite velocity is dropped: the game's own post-move velocity goes through drag
    /// and gravity.
    #[test]
    #[ignore = "daily"]
    fn a_non_finite_moved_edit_is_dropped() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let velocity = air_travel(
            Moved(Some([f64::NAN, 0.0, 0.0]), Arc::clone(&heard)),
            sunrise_world::native_load_context::LoopSide::Server,
        );
        assert_eq!(
            velocity,
            Vec3::new(
                0.2 * f64::from(0.91_f32),
                (0.5 - 0.08) * f64::from(0.98_f32),
                0.0
            )
        );
    }
}
