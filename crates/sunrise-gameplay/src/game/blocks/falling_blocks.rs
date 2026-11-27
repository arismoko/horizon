//! Falling block entities (`FallingBlockEntity.tick`): one block's tick (`tick_one_falling`), its move
//! (`fall_step`) and the item entity it leaves as a drop (`drop_falling_block`, `Entity.spawnAtLocation`).
use super::*;

impl crate::game::ServerGame<'_> {
    /// The end of a piston's `Entity.move(MoverType.PISTON, …)` for falling
    /// block `i` after a step of `moved`: `checkFallDamage` adds the drop and,
    /// on the ground, runs fallOn and posts HIT_GROUND after a fall, then
    /// forgets it, so its own next tick does not land again.
    pub(in crate::game::server_side) fn finish_falling_piston_move(
        &mut self,
        i: usize,
        moved: Vec3,
    ) {
        let mut f = self.common.falling[i].clone();
        if moved.y < 0.0 {
            f.fall_distance -= f64::from(moved.y as f32);
        }
        if f.ground.on_ground {
            self.falling_block_impact(&mut f);
            if f.fall_distance > 0.0 {
                let bounds = f.bounds();
                self.post_landing_event(f.position, bounds, false);
            }
            f.fall_distance = 0.0;
        }
        self.common.falling.update(i, |block| *block = f);
    }
    /// FallingBlockEntity.tick: fall with 0.04 gravity and 0.98F drag, then land.
    /// One falling block's `tick`: places or drops its block in its own tick,
    /// so a block falling onto one that just landed lands on top of it. `i` is
    /// its place in the list; the caller has checked it is ticking.
    /// `entities` are the boxes that stop its move.
    pub fn tick_one_falling(&mut self, i: usize, entities: &[Aabb]) -> Result<()> {
        let entity_drops = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::ENTITY_DROPS);
        // Like boats, tick an unbound state clone while the resident owner keeps
        // its slot. Extracting and reinserting every airborne block shifts the
        // whole list tail twice per entity, making a sand-filled cobweb O(n^2).
        let mut f = self.common.falling[i].clone();
        let from = f.position;
        let (id, join) = (f.id, f.join);
        self.with_entity_held_out(id, |game| {
            let step = game.fall_step(&mut f, entity_drops, entities)?;
            // Its move is filed at once, also for one that lands or drops.
            let to = f.position;
            game.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
            if matches!(step, FallStep::Falling) {
                let state = f.state;
                if !game
                    .persist
                    .removals
                    .contains(&super::commands::Target::Falling(id))
                    && game.falling.store_back(i, f)
                {
                    game.note_falling_tick(id, join, state);
                }
                return Ok(());
            }
            if game.server.ordered.defer_landings {
                if let Some(mut kept) = game.finish_falling_step(step, f) {
                    // Entity.scale(getAirDrag()) ends the tick of a block that stays.
                    kept.velocity = sunrise_sim::entity_rules::scale_air_drag(kept.velocity);
                    let state = kept.state;
                    if !game
                        .persist
                        .removals
                        .contains(&super::commands::Target::Falling(id))
                        && game.falling.store_back(i, kept)
                    {
                        game.note_falling_tick(id, join, state);
                    }
                    return Ok(());
                }
                // Only other falling blocks run before compaction, and their
                // collision search excludes falling blocks. Finish this entity
                // now so subsequent falls see its placed block.
                game.server.ordered.landed.insert(join);
                return Ok(());
            }
            // Only terminal owners leave storage. Carry the original lifetime
            // into the final state until placement, drops or travel finish.
            let Some(mut owner) = game.falling.extract(join) else {
                return Ok(());
            };
            sunrise_sim::join_seq::store_back(&mut owner, f);
            if let Some(mut kept) = game.finish_falling_step(step, owner)
                && !game
                    .persist
                    .removals
                    .contains(&super::commands::Target::Falling(id))
            {
                // Entity.scale(getAirDrag()) ends the tick of a block that stays.
                kept.velocity = sunrise_sim::entity_rules::scale_air_drag(kept.velocity);
                let state = kept.state;
                let falling = &mut game.common.falling;
                falling.put_back(i.min(falling.len()), kept);
                game.note_falling_tick(id, join, state);
                return Ok(());
            }
            game.falling.retire_extracted(id, join);
            Ok(())
        })
    }
    /// The landing work of a falling block's tick: nothing for one gone, its item for one
    /// that dropped, or the landing decision for one on the ground (returning the block when
    /// it stays).
    pub(in crate::game::server_side) fn finish_falling_step(
        &mut self,
        step: FallStep,
        owner: FallingBlock,
    ) -> Option<FallingBlock> {
        match step {
            FallStep::Gone => None,
            FallStep::Dropped(at) => {
                self.drop_falling_block(owner.state, at);
                None
            }
            FallStep::Landed(pos) => {
                let sample = sunrise_level::spans::SpanTotal::start();
                let kept = self.finish_falling_block(owner, pos);
                self.server.ordered.landing_time.add(sample);
                kept
            }
            FallStep::Falling => unreachable!(),
        }
    }
    /// One `FallingBlockEntity.tick` move of `f`.
    pub(in crate::game) fn fall_step(
        &mut self,
        f: &mut FallingBlock,
        entity_drops: bool,
        entities: &[Aabb],
    ) -> Result<FallStep> {
        if self.common.res.facts().is_air(f.state) {
            return Ok(FallStep::Gone);
        }
        f.previous = f.position;
        f.age = f.age.wrapping_add(1);
        f.velocity.y -= sunrise_sim::entity_rules::EntityRules::get_gravity(f);
        let wanted = f.velocity;
        // The falling block's Entity.move goes through the move gate (#2414),
        // armed before the stuck multiplier of the move itself; PR 1a records
        // the movement unchanged.
        let mut gate = sunrise_sim::move_gate::armed(
            &self.common.loop_context,
            f.id,
            "minecraft:falling_block",
            MoverType::SelfMovement,
            wanted,
        );
        let mut wanted = gate.delta();
        // Entity.move: a cobweb it was in scales this move, which then
        // starts from rest.
        if f.stuck.length_squared() > 1.0e-7 {
            wanted = wanted.multiply(f.stuck);
            f.stuck = Vec3::ZERO;
            f.velocity = Vec3::ZERO;
        }
        let scene = collisions::SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: Some(collisions::PowderSnowSubject {
                fall_distance: f.fall_distance,
                falling_block: true,
                ..Default::default()
            }),
        };
        // Entity.isDescending: only sneaking entities descend through
        // scaffolding. Falling blocks do not sneak, so descending is always
        // false (Entity.isDescending returns isShiftKeyDown, fixes #7553).
        let context = CollisionContext::for_entity(
            CollisionEntity {
                identity: EntityIdentity(u64::from(f.id)),
                kind: sunrise_world::collision_context::EntityContextKind::Other,
                y: f.position.y,
                descending: false,
                main_hand: None,
            },
            false,
        );
        // On an error the caller keeps the block as it was before this tick.
        // Entity.collide: the world border's walls and the entities that
        // `canBeCollidedWith` it (a boat, a shulker) stop it too.
        let mut collisions = collisions::EntityCollisions {
            blocks: scene,
            entities,
            border: Some((self.common.border.geometry(), [f.position.x, f.position.z])),
        };
        let movement = sunrise_sim::move_gate::entities_resolve_with_context(
            &mut gate,
            &mut collisions,
            f.bounds(),
            wanted,
            0.0,
            false,
            context,
        )?;
        let next = Vec3::new(
            f.position.x + movement.x,
            f.position.y + movement.y,
            f.position.z + movement.z,
        );
        if movement.y < 0.0 {
            f.fall_distance -= f64::from(movement.y as f32);
        }
        let from = f.position;
        f.position = next;
        let body = sunrise_sim::motion::BodyMotion {
            world: &self.common.world,
            collision: self.common.collision,
            facts: self.common.res.facts(),
            blocks: Some(&self.common.res.motion().blocks),
            context: &self.common.loop_context,
            side: None,
        };
        if !gate.skipped() {
            body.record_move(
                sunrise_sim::motion::BodyCollision {
                    footprint: (f.position, f.bounds()),
                    movement: (wanted, movement),
                    authoritative: true,
                    context,
                    subject: Some(collisions::PowderSnowSubject {
                        fall_distance: f.fall_distance,
                        falling_block: true,
                        ..Default::default()
                    }),
                },
                &mut f.ground,
            );
        }
        let effect = body.legacy_contact(f.position, &f.ground);
        if f.ground.on_ground {
            self.falling_block_impact(f);
            if f.fall_distance > 0.0 {
                self.post_landing_event(f.position, f.bounds(), false);
            }
            f.fall_distance = 0.0;
        }
        let entity = (f.id, "minecraft:falling_block");
        let response = self.common.body_motion(&game_side!(self)).restitute(
            entity,
            sunrise_sim::motion::BodyMove {
                wanted,
                moved: movement,
                effect,
            },
            sunrise_sim::entity_rules::EntityRules::get_gravity(f),
            0.98_f32,
            f.velocity,
            f.fall_distance,
        );
        f.velocity = self.finish_body_restitution(entity, f.position, response);
        let side = game_side!(self);
        let body = self.common.body_motion(&side);
        body.apply_block_speed(f.position, &mut f.velocity);
        let writes =
            self.body_step_on(entity, (f.position, f.bounds()), &f.ground, &mut f.velocity);
        super::entity::apply_contact_writes(
            writes,
            &mut f.velocity,
            Some(&mut f.fall_distance),
            Some(&mut f.stuck),
            Some(&mut f.fire_ticks),
        );
        // applyEffectsFromBlocks: WebBlock.entityInside makes it stuck
        // (Entity.makeStuckInBlock), BubbleColumnBlock.entityInside lifts or
        // drags it. Entity.move records the move unless it was blocked short
        // of a tiny one.
        let moved = movement.length_squared();
        let recorded = moved > 1.0e-7 || wanted.length_squared() - moved < 1.0e-7;
        let world = &self.common.world;
        let facts = self.common.res.facts();
        let (registry, collision) = (self.common.pack.assets.registry, self.common.collision);
        let mut tops = Vec::new();
        let stuck = sunrise_sim::falling_block::apply_effects_from_blocks(
            from,
            f.position,
            recorded.then_some(wanted),
            &mut f.velocity,
            &mut f.fall_distance,
            &mut tops,
            |p| {
                use sunrise_sim::falling_block::Inside;
                if &*facts.get(world.block(p.x, p.y, p.z)).name == "minecraft:cobweb" {
                    Inside::Cobweb
                } else if let Some(column) =
                    sunrise_sim::motion::bubble_column_at(world, registry, facts, collision, p)
                {
                    Inside::BubbleColumn(column)
                } else {
                    Inside::Nothing
                }
            },
        );
        if let Some(multiplier) = stuck {
            f.stuck = multiplier;
        }
        // The same walk's fluids: LavaFluid.entityInside's LAVA_IGNITE, then
        // its lavaHurt, whose `FallingBlockEntity.hurtServer` harms nothing but
        // marks the block hurt (`markHurt`, below); then WaterFluid's EXTINGUISH
        // (`clearFire`). A block not on fire that nothing ignited is put at
        // `-getFireImmuneTicks()`, 0.
        let fluids = &self.common.res.motion().fluids;
        let touches = |family| {
            sunrise_sim::entities::fluid_contacts(
                from,
                f.position,
                recorded.then_some(wanted),
                sunrise_sim::falling_block::bounds,
                |p| sunrise_sim::entities::fluid_height(world, fluids, p, family),
            ) > 0
        };
        let previous_fire = f.fire_ticks;
        let mut fire = previous_fire;
        let in_lava = touches(sunrise_world::fluid::FluidKind::Lava);
        if in_lava {
            fire = fire.max(300);
        }
        if fire > 0 && touches(sunrise_world::fluid::FluidKind::Water) {
            fire = 0;
        }
        if fire <= 0 && fire <= previous_fire {
            fire = 0;
        }
        f.fire_ticks = fire;
        // lavaHurt runs on the server alone; `isInvulnerableToBase` reads the
        // saved `Invulnerable`.
        if in_lava {
            let vanilla = self.saved_refuses_damage(f.id, "minecraft:lava", false);
            if !self.refuses_lava_hurt(f.id, "minecraft:falling_block", vanilla) {
                f.sync_velocity = true;
            }
        }
        for pos in tops {
            self.bubble_column_particles(pos);
        }
        self.entity_powder_snow_particles(from, f.position, f.bounds(), f.velocity, false);
        // FallingBlockEntity.tick handles its portal after the move's block
        // effects, before placing/dropping on the floor under an End portal.
        let teleported = self.falling_through_portal(f);
        // FallingBlockEntity.forceTickAfterTeleportToDuplicate: crossing
        // either way through the End deliberately keeps the source's final
        // placement/drop branch, even though the source entity was removed.
        if let Some(target) = teleported
            && self.common.dim != Dim::End
            && target != Dim::End
        {
            return Ok(FallStep::Gone);
        }
        let y = next.y.floor() as i32;
        let outside = y <= self.common.world.bottom_y() || y >= self.common.world.top_y();
        if !f.ground.on_ground && (f.age > 100 && outside || f.age > 600) {
            return Ok(if f.drop_item && entity_drops {
                FallStep::Dropped(next)
            } else {
                FallStep::Gone
            });
        }
        if f.ground.on_ground {
            let pos = [
                f.position.x.floor() as i32,
                f.position.y.floor() as i32,
                f.position.z.floor() as i32,
            ];
            // On the ground, before the landing decision.
            f.velocity = f.velocity.multiply(Vec3::new(0.7, -0.5, 0.7));
            let at = self.common.world.block(pos[0], pos[1], pos[2]);
            if &*self.common.res.facts().get(at).name != "minecraft:moving_piston" {
                return Ok(FallStep::Landed(pos));
            }
        }
        // Entity.scale(getAirDrag()): 0.98F, widened before it scales motion.
        f.velocity = sunrise_sim::entity_rules::scale_air_drag(f.velocity);
        Ok(if teleported.is_some() {
            FallStep::Gone
        } else {
            FallStep::Falling
        })
    }
    /// `Entity.spawnAtLocation`: an item entity where the block was.
    pub(in crate::game::server_side) fn drop_falling_block(
        &mut self,
        state: BlockStateId,
        at: Vec3,
    ) {
        // Entity.spawnAtLocation: an item entity where the block was.
        let item = self.common.res.facts().get(state).item.clone();
        if &*item != "minecraft:air" {
            // new ItemEntity(level, x, y, z, stack): the motion is from the
            // item's own random, not the level's.
            let stack = ItemStack {
                item,
                count: 1,
                damage: 0,
                extra: None,
            };
            self.spawn_item_own(stack, [at.x, at.y, at.z], 10);
        }
    }
}

use sunrise_gameplay_common::game::entity_common::EntityCommonGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;
