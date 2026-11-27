//! Server-side piston, shulker lid, block-push-up, and forced entity move passes.
use super::move_gate::MoveWho;
use crate::game::server_side::minecarts_server::CartHit;
use sunrise_core::geometry::{
    Aabb, Axis, COLLISION_EPSILON, Direction, ShapeOp, Vec3, entity_collision_bounds,
};
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::collision_source::Mover;
use sunrise_gameplay_common::game::entity::{Entity, EntityHandle, with_entity, with_entity_mut};
use sunrise_gameplay_common::game::piston_push::{
    Body, Lid, Push, PushTarget, ShulkerPush, StepHeights, first_axis_direction, pushed_movement,
    shulker_lid,
};
use sunrise_level::collisions::{SceneCollisions, with_piston_noclip};
use sunrise_sim::entity_rules::EntityRules;
use sunrise_sim::join_seq::{JoinSeq, Joined};
use sunrise_sim::motion::MotionWorld;
use sunrise_world::{
    entity_move::MoverType, level::data::PushReaction, move_gate::MoveGate,
    piston_movement::PistonMovement, piston_push,
};

impl crate::game::ServerGame<'_> {
    /// Server arm of [`Game::side_push_piston_entities`].
    pub(in crate::game) fn server_push_piston_entities(&mut self, pushes: Vec<Push>) {
        let mut accumulators = std::collections::HashMap::new();
        let mut pricked = Vec::new();
        let index = self.common.build_piston_index();
        let steps = self.push_step_heights();
        let targets = self.piston_push_targets(&pushes, &steps);
        for push in pushes {
            let pos = push.0;
            self.push_for_piston(push, &targets, &steps, &mut accumulators, &mut pricked);
            self.common.advance_piston_collision(pos, &index);
        }
        self.hurt_cactus_pricked_vehicles(pricked);
    }

    /// `moveEntityByPiston`'s `applyEffectsFromBlocks` for a boat or a
    /// minecart: every cactus its box touches on the way from `start` to
    /// `end` pricks it (`CactusBlock.entityInside`, `Entity.checkInsideBlocks`
    /// over the box less 1e-5), even when the push moved it nowhere. The hurt
    /// waits for the end of the tick's pushes, whose targets are found first.
    pub(in crate::game) fn piston_cactus_contact(
        &self,
        target: MoveWho,
        start: Aabb,
        end: Aabb,
        pricked: &mut Vec<u32>,
    ) {
        let id = match target {
            MoveWho::Entity(EntityHandle::Boat(i)) => self.common.boats[i].id,
            MoveWho::Entity(EntityHandle::Cart(i)) => self.common.carts[i].id,
            _ => return,
        };
        let eps = f64::from(1.0e-5_f32);
        let (a, b) = (start.min_position(), end.min_position());
        let lo = Vec3::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z));
        let (a, b) = (start.max_position(), end.max_position());
        let hi = Vec3::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z));
        let facts = self.common.res.facts();
        for x in (lo.x + eps).floor() as i32..=(hi.x - eps).floor() as i32 {
            for y in (lo.y + eps).floor() as i32..=(hi.y - eps).floor() as i32 {
                for z in (lo.z + eps).floor() as i32..=(hi.z - eps).floor() as i32 {
                    if &*facts.get(self.common.world.block(x, y, z)).name == "minecraft:cactus" {
                        pricked.push(id);
                    }
                }
            }
        }
    }

    /// `Entity.hurt(cactus, 1)` for the vehicles `piston_cactus_contact`
    /// found, in the order of their pushes.
    pub(in crate::game) fn hurt_cactus_pricked_vehicles(&mut self, pricked: Vec<u32>) {
        for id in pricked {
            if let Some(i) = self.common.boats.iter().position(|b| b.id == id) {
                self.hurt_boat(i, 1.0, false);
            } else if let Some(i) = self.common.carts.iter().position(|c| c.id == id) {
                self.hurt_cart(i, 1.0, false, false, CartHit::default());
            }
        }
    }

    /// The bodies any of this tick's moving blocks can reach, in `push_targets` order.
    pub(in crate::game) fn piston_push_targets(
        &self,
        pushes: &[Push],
        steps: &StepHeights,
    ) -> Vec<PushTarget> {
        let Some(reach) = pushes
            .iter()
            .map(|(p, ..)| {
                let low = Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z));
                Aabb::new(
                    low.add_vector(Vec3::new(-2.0, -2.0, -2.0)),
                    low.add_vector(Vec3::new(3.0, 3.0, 3.0)),
                )
            })
            .reduce(|a, b| a.union_bounds(b))
        else {
            return Vec::new();
        };
        let at = pushes[0].0;
        self.push_targets()
            .filter(|&target| {
                self.piston_body(target, Some(at), steps)
                    .is_some_and(|body| body.bounds.intersects(reach))
            })
            .filter_map(|target| self.stable_push_target(target))
            .collect()
    }

    /// `moveEntityByPiston`'s `applyEffectsFromBlocks(previousPos, position)` for projectile `i`.
    pub(in crate::game) fn projectile_piston_block_effects(&mut self, i: usize, from: Vec3) {
        let p = &self.common.creatures.projectiles[i];
        let end = p.aabb();
        let offset = from.subtract(p.position);
        let start = end.move_by(offset);
        let (lo, hi) = (
            Vec3::new(
                start.min_position().x.min(end.min_position().x) + 1.0e-5,
                start.min_position().y.min(end.min_position().y) + 1.0e-5,
                start.min_position().z.min(end.min_position().z) + 1.0e-5,
            ),
            Vec3::new(
                start.max_position().x.max(end.max_position().x) - 1.0e-5,
                start.max_position().y.max(end.max_position().y) - 1.0e-5,
                start.max_position().z.max(end.max_position().z) - 1.0e-5,
            ),
        );
        let mut stuck = None;
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    let name = &self
                        .common
                        .res
                        .facts()
                        .get(self.common.world.block(x, y, z))
                        .name;
                    if let Some(s) = sunrise_sim::projectiles::stuck_speed_of(name) {
                        stuck = Some(s);
                    }
                }
            }
        }
        if let Some(stuck) = stuck {
            self.common
                .creatures
                .projectiles
                .update(i, |p| p.stuck_speed = stuck);
        }
        let stuck_arrow = self.common.creatures.projectiles[i]
            .arrow
            .as_ref()
            .is_some_and(|a| a.in_ground);
        if !stuck_arrow {
            let (world, registry, facts, collision) = (
                &self.common.world,
                self.common.pack.assets.registry,
                self.common.res.facts(),
                self.common.collision,
            );
            let tops = self.common.creatures.projectiles.update(i, |p| {
                p.bubble_surfaces.clear();
                p.bubble_columns_since(from, |at| {
                    sunrise_sim::motion::bubble_column_at(
                        world,
                        registry,
                        facts,
                        collision,
                        BlockPos::new(at[0], at[1], at[2]),
                    )
                    .map(|c| (c.drag, c.surface))
                });
                std::mem::take(&mut p.bubble_surfaces)
            });
            for [x, y, z] in tops {
                self.bubble_column_particles(BlockPos::new(x, y, z));
            }
        }
    }

    /// `moveEntityByPiston`'s `applyEffectsFromBlocks` for entity-sensing blocks.
    pub(in crate::game) fn piston_block_triggers(&mut self, start: Aabb, end: Aabb) {
        let eps = 1.0e-5;
        let (a, b) = (start.min_position(), end.min_position());
        let lo = Vec3::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z));
        let (a, b) = (start.max_position(), end.max_position());
        let hi = Vec3::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z));
        let behaviours = self.common.res.behaviours();
        let mut sensing = Vec::new();
        for x in (lo.x + eps).floor() as i32..=(hi.x - eps).floor() as i32 {
            for y in (lo.y + eps).floor() as i32..=(hi.y - eps).floor() as i32 {
                for z in (lo.z + eps).floor() as i32..=(hi.z - eps).floor() as i32 {
                    if behaviours.senses_entities(self.common.world.block(x, y, z)) {
                        sensing.push(BlockPos::new(x, y, z));
                    }
                }
            }
        }
        if sensing.is_empty() {
            return;
        }
        self.with_level(|b, level, ticks| {
            for pos in sensing {
                b.entity_inside(level, ticks, pos);
            }
        });
    }

    /// An opening lid's push on the server (`ShulkerBoxBlockEntity` / `Shulker`).
    pub fn lid_push(&mut self, lid: Lid, area: Aabb, movement: Vec3) {
        let steps = self.push_step_heights();
        let root = self.lid_root(lid);
        // The armor stands' obstacles, built at the first stand the lid meets.
        let mut stand_grid = None;
        let targets: Vec<_> = self
            .push_targets()
            .filter_map(|target| self.stable_push_target(target))
            .collect();
        targets.into_iter().for_each(|selected| {
            let Some(target) = self.resolve_push_target(selected) else {
                return;
            };
            let body = match lid {
                Lid::Box => self.piston_body(target, None, &steps),
                Lid::Shulker(_) => self.push_body(target, None, &steps),
            };
            let Some(body) = body else {
                return;
            };
            if !body.bounds.intersects(area) {
                return;
            }
            let no_physics = self.common.pushed_no_physics(target);
            if self.lid_spares(lid, root, target, no_physics) {
                return;
            }
            self.move_pushed_body(
                lid.mover(),
                target,
                body,
                movement,
                no_physics,
                &mut stand_grid,
            );
        });
        self.lid_breaks_attached(area, root);
    }

    /// The root vehicle of a shulker whose lid pushes; none for a box.
    pub(in crate::game) fn lid_root(&self, lid: Lid) -> Option<u32> {
        let id = lid.shulker()?;
        self.native_root_vehicle_id(id, super::commands::Target::Mob(id))
    }

    /// Whether a shulker's lid spares a body it meets.
    pub(in crate::game) fn lid_spares(
        &self,
        lid: Lid,
        root: Option<u32>,
        target: MoveWho,
        no_physics: bool,
    ) -> bool {
        self.common
            .lid_spares_with_root(lid, root, target, self.push_root(target), no_physics)
    }

    /// `BlockAttachedEntity.move`: a lid that moves a painting, an item frame
    /// (`ItemFrame.move`: not a fixed one) or a cushion breaks it on the server.
    pub(in crate::game) fn lid_breaks_attached(&mut self, area: Aabb, root: Option<u32>) {
        let meets = |(lo, hi): ([f64; 3], [f64; 3])| {
            Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            )
            .intersects(area)
        };
        let mut i = 0;
        while i < self.common.paintings.len() {
            if !(meets(self.common.paintings[i].bounds()) && self.break_painting_by(i, false, None))
            {
                i += 1;
            }
        }
        let mut i = 0;
        while i < self.common.item_frames.len() {
            let f = &self.common.item_frames[i];
            if f.fixed || !meets(f.bounds()) || !self.break_frame_by(i, None) {
                i += 1;
            }
        }
        let mut i = 0;
        while i < self.common.cushions.len() {
            let c = &self.common.cushions[i];
            if root == Some(c.id) || !meets(c.bounds()) || !self.break_cushion_by(i, None) {
                i += 1;
            }
        }
    }

    /// The id of the lowest vehicle under a pushed body on the server.
    pub(in crate::game) fn push_root(&self, target: MoveWho) -> Option<u32> {
        use super::commands::Target as Named;
        let vehicle_root = |v: u32| self.native_root_vehicle_id(v, Named::Mob(v));
        match target {
            MoveWho::Player => self.common.riding.and_then(vehicle_root),
            MoveWho::Guest(conn) => self.command_guest(conn)?.vehicle.and_then(vehicle_root),
            MoveWho::Entity(handle) => {
                let (id, named) =
                    with_entity!(self, handle, entity => (entity.id(), entity.selector_target()));
                match named {
                    Some(named) => self.native_root_vehicle_id(id, named),
                    None => Some(id),
                }
            }
        }
    }

    /// `Entity.move(mover, movement)` for a lid's push, collider carry/shove, or mod `move_by`.
    pub(in crate::game) fn move_pushed_body(
        &mut self,
        mover: MoverType,
        target: MoveWho,
        body: Body,
        movement: Vec3,
        no_physics: bool,
        stand_grid: &mut Option<sunrise_sim::entities::ObstacleGrid>,
    ) {
        if target == MoveWho::Player {
            let _ = self.server_move_local_player(mover, movement, false);
            return;
        }
        // Armor stands arm their gate in their own collided shove.
        if let MoveWho::Entity(EntityHandle::ArmorStand(i)) = target {
            self.shoves_armor_stand(i, mover, movement, stand_grid);
            return;
        }
        let Some(selected) = self.stable_push_target(target) else {
            return;
        };
        let mut gate = self.entity_move(target, mover, movement);
        if gate.skipped() {
            return;
        }
        let movement = gate.delta();
        let lid_move = matches!(mover, MoverType::ShulkerBox | MoverType::Shulker);
        // `Entity.move` with `noPhysics`: `setPos` the whole way.
        if no_physics {
            self.apply_push(&mut gate, target, movement);
            if let Some(target) = self.resolve_push_target(selected) {
                if lid_move && let MoveWho::Entity(EntityHandle::Boat(i)) = target {
                    self.record_boat_lid_movement(i, movement);
                }
                self.common.release_after_block_move(target);
            }
            return;
        }
        let entities = match target {
            MoveWho::Entity(EntityHandle::Boat(i)) if lid_move => {
                let area =
                    entity_collision_bounds(movement, body.bounds, body.step_height).inflate(
                        Vec3::new(COLLISION_EPSILON, COLLISION_EPSILON, COLLISION_EPSILON),
                    );
                self.entity_collisions(Mover::Boat {
                    id: self.common.boats[i].id,
                    area,
                })
            }
            _ => self.pushed_entity_colliders(target),
        };
        let collision = self.pushed_living_collision(target);
        let Ok(moved) = pushed_movement(
            &mut gate,
            SceneCollisions {
                world: &self.common.world,
                binding: self.common.collision,
                subject: collision.map(|(_, subject)| subject),
            },
            self.common.border.geometry(),
            [body.position.x, body.position.z],
            body.bounds,
            movement,
            body.step_height,
            body.on_ground,
            collision.map(|(context, _)| context),
            &entities,
        ) else {
            return;
        };
        let placed = moved.length_squared() > 1.0e-7
            || movement.length_squared() - moved.length_squared() < 1.0e-7;
        if placed {
            if lid_move && let MoveWho::Entity(EntityHandle::Boat(i)) = target {
                let boat = &self.common.boats[i];
                let fall = self
                    .common
                    .vehicle_fall_reset(boat.fall_distance, body.position, moved);
                self.common
                    .boats
                    .update(i, |boat| boat.fall_distance = fall);
            }
            self.reset_general_fall(target, body.position, moved);
            self.apply_push(&mut gate, target, moved);
        }
        let Some(target) = self.resolve_push_target(selected) else {
            return;
        };
        if lid_move
            && placed
            && let MoveWho::Entity(EntityHandle::Boat(i)) = target
        {
            self.record_boat_lid_movement(i, moved);
        }
        self.set_pushed_on_ground(target, movement, moved, None);
        if lid_move && let MoveWho::Entity(EntityHandle::Boat(i)) = target {
            let boat = &self.common.boats[i];
            if self.boat_can_simulate_movement(boat.id) {
                let fall = self.common.boat_fall(boat, moved.y);
                self.common
                    .boats
                    .update(i, |boat| boat.fall_distance = fall);
            }
        }
        if let Some(target) =
            self.finish_pushed_restitution(selected, target, body.position, movement, moved)
        {
            self.common.release_after_block_move(target);
        }
    }

    /// A mod's `Entity.move(cause, movement)` for the listed entity `handle`.
    pub fn move_listed_entity(
        &mut self,
        handle: EntityHandle,
        cause: MoverType,
        movement: Vec3,
    ) -> Vec3 {
        let target = MoveWho::Entity(handle);
        let selected = self.stable_push_target(target).expect("listed move target");
        let from = with_entity!(self, handle, entity => entity.position());
        let body = self.common.listed_body(handle, None);
        if self.common.pushed_no_physics(target) {
            let mut gate = self.entity_move(target, cause, movement);
            let movement = gate.delta();
            self.apply_push(&mut gate, target, movement);
            if let Some(target) = self.resolve_push_target(selected) {
                self.common.release_after_block_move(target);
            }
        } else if let Some(direction) = (cause == MoverType::Piston)
            .then(|| first_axis_direction(movement))
            .flatten()
        {
            let mut pricked = Vec::new();
            self.move_pushed(
                target,
                body.bounds,
                [body.position.x, body.position.z],
                body.step_height,
                body.on_ground,
                movement,
                direction,
                &mut std::collections::HashMap::new(),
                &mut pricked,
            );
            self.hurt_cactus_pricked_vehicles(pricked);
        } else {
            let stand = match handle {
                EntityHandle::ArmorStand(i) => Some(self.common.armor_stands[i].velocity),
                _ => None,
            };
            self.move_pushed_body(cause, target, body, movement, false, &mut None);
            if let Some(velocity) = stand
                && let Some(MoveWho::Entity(EntityHandle::ArmorStand(i))) =
                    self.resolve_push_target(selected)
            {
                self.common.armor_stands[i].velocity = velocity;
            }
        }
        if let Some(MoveWho::Entity(handle)) = self.resolve_push_target(selected) {
            with_entity!(self, handle, entity => entity.position()).subtract(from)
        } else {
            Vec3::ZERO
        }
    }

    pub(in crate::game) fn push_for_piston(
        &mut self,
        (pos, piston, new_progress): Push,
        targets: &[PushTarget],
        steps: &StepHeights,
        accumulators: &mut std::collections::HashMap<(u32, JoinSeq), PistonMovement>,
        pricked: &mut Vec<u32>,
    ) {
        let delta_progress = f64::from(new_progress - piston.progress);
        if delta_progress <= 0.0 {
            return;
        }
        let Some(bindings) = self.common.collision.pistons() else {
            return;
        };
        let Ok(state) = bindings.collision_related_state(piston.shape_state()) else {
            return;
        };
        let mut scene = SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: None,
        };
        let Ok(shape) =
            self.common
                .collision
                .collision_shape(&mut scene, state, [pos.x, pos.y, pos.z])
        else {
            return;
        };
        if shape.is_empty() {
            return;
        }
        let movement = piston.movement_direction();
        let boxes: Vec<Aabb> = shape.boxes().to_vec();
        let facts = self.common.res.facts();
        let slime = &*facts.get(piston.moved_state).name == "minecraft:slime_block";
        let honey = &*facts.get(piston.moved_state).name == "minecraft:honey_block";
        let sticky_top = if honey {
            let mut scene = SceneCollisions {
                world: &self.common.world,
                binding: self.common.collision,
                subject: None,
            };
            self.common
                .collision
                .collision_shape(&mut scene, piston.moved_state, [pos.x, pos.y, pos.z])
                .ok()
                .and_then(|s| s.bounds())
                .map_or(0.0, |b| b.max_position().y)
        } else {
            0.0
        };

        let search = shape.bounds().map(|bounds| {
            piston_push::movement_search_area(
                piston_push::move_by_position_and_progress(pos, bounds, piston.shape_state()),
                movement,
                delta_progress,
            )
        });
        targets.iter().copied().for_each(|selected| {
            let Some(target) = self.resolve_push_target(selected) else {
                return;
            };
            let Some(body) = self.piston_body(target, Some(pos), steps) else {
                return;
            };
            if search.is_some_and(|area| !area.intersects(body.bounds)) {
                return;
            }
            let amount = piston_push::collided_amount(
                &boxes,
                pos,
                piston.shape_state(),
                new_progress,
                movement,
                body.bounds,
            );
            if slime && search.is_some_and(|area| area.intersects(body.bounds)) {
                self.bounce(target, movement);
            }
            if amount > 0.0 {
                let step = piston_push::step_vector(movement);
                let before = body.bounds;
                self.move_pushed(
                    target,
                    before,
                    [body.position.x, body.position.z],
                    body.step_height,
                    body.on_ground,
                    step.scale(amount),
                    movement,
                    accumulators,
                    pricked,
                );
                if !piston.extending
                    && piston.source
                    && let Some(target) = self.resolve_push_target(selected)
                    && let Some(after) = self.piston_body(target, Some(pos), steps)
                    && let Some(fix) = piston_push::fix_within_piston_base(
                        pos,
                        after.bounds,
                        movement,
                        delta_progress,
                    )
                {
                    let back = piston_push::step_vector(movement.opposite());
                    self.move_pushed(
                        target,
                        after.bounds,
                        [after.position.x, after.position.z],
                        after.step_height,
                        after.on_ground,
                        back.scale(fix),
                        movement,
                        accumulators,
                        pricked,
                    );
                }
            }
        });
        // Vanilla selects honey passengers after all collided entities have moved.
        if honey && movement.normal()[1] == 0 {
            let area = piston_push::sticky_area(pos, piston.shape_state(), sticky_top);
            for &selected in targets {
                let Some(target) = self.resolve_push_target(selected) else {
                    continue;
                };
                let Some(body) = self.piston_body(target, Some(pos), steps) else {
                    continue;
                };
                if piston_push::sticky_entity_selected(
                    area,
                    body.bounds,
                    body.piston_reaction,
                    body.on_ground,
                    body.supported,
                    body.position,
                ) {
                    self.move_pushed(
                        target,
                        body.bounds,
                        [body.position.x, body.position.z],
                        body.step_height,
                        body.on_ground,
                        piston_push::step_vector(movement).scale(delta_progress),
                        movement,
                        accumulators,
                        pricked,
                    );
                }
            }
        }
    }

    pub(in crate::game) fn stable_push_target(&self, hint: MoveWho) -> Option<PushTarget> {
        let (id, join) = match hint {
            MoveWho::Player => (self.common.player.as_ref()?.entity_id as u32, None),
            MoveWho::Guest(conn) => (
                self.server.lan.as_ref()?.hub.guest(conn)?.entity_id as u32,
                None,
            ),
            MoveWho::Entity(handle) => with_entity!(self, handle, entity => {
                (entity.id(), Some(entity.join_seq()))
            }),
        };
        Some(PushTarget { hint, id, join })
    }

    pub(in crate::game) fn resolve_push_target(
        &self,
        PushTarget { hint, id, join }: PushTarget,
    ) -> Option<MoveWho> {
        match hint {
            MoveWho::Player => self
                .common
                .player
                .as_ref()
                .filter(|p| p.entity_id as u32 == id)
                .map(|_| hint),
            MoveWho::Guest(conn) => {
                let hub = &self.server.lan.as_ref()?.hub;
                hub.guest(conn)
                    .filter(|g| g.entity_id as u32 == id && hub.here(g))
                    .map(|_| hint)
            }
            MoveWho::Entity(handle) => {
                let handle = self.refresh_entity_handle(id, handle)?;
                with_entity!(self, handle, entity => {
                    (!entity.is_removed() && Some(entity.join_seq()) == join
                        && !entity.selector_target().is_some_and(|named| self.persist.removals.contains(&named)))
                        .then_some(MoveWho::Entity(handle))
                })
            }
        }
    }

    /// `Entity.getStepHeight` for each body, read once.
    pub(in crate::game) fn push_step_heights(&mut self) -> StepHeights {
        let player = self
            .common
            .player
            .as_mut()
            .and_then(|p| p.attribute("minecraft:step_height").ok())
            .map_or(0.6, |v| v as f32);
        let mut guests = std::collections::HashMap::new();
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.guests.update_all(|g| {
                let step = g
                    .motion
                    .as_mut()
                    .and_then(|m| m.attribute("minecraft:step_height").ok())
                    .map_or(0.6, |v| v as f32);
                guests.insert(g.conn, step);
            });
        }
        StepHeights { player, guests }
    }

    /// The entities a moving block can run into: the local player, LAN guests, then listed entities.
    pub(in crate::game) fn push_targets(&self) -> impl Iterator<Item = MoveWho> + use<> {
        let player = self.common.player.is_some().then_some(MoveWho::Player);
        let guests: Vec<MoveWho> = self.server.lan.as_ref().map_or_else(Vec::new, |host| {
            host.hub
                .guests
                .iter()
                .filter(|g| host.hub.here(g) && g.loaded && !g.dead)
                .map(|g| MoveWho::Guest(g.conn))
                .collect()
        });
        player
            .into_iter()
            .chain(guests)
            .chain(self.entity_handles().map(MoveWho::Entity))
    }

    /// Pistons and shulker boxes skip bodies that ignore piston reactions.
    pub(in crate::game) fn piston_body(
        &self,
        target: MoveWho,
        pos: Option<BlockPos>,
        steps: &StepHeights,
    ) -> Option<Body> {
        let body = match target {
            // PistonMovingBlockEntity collects a resident stand regardless of health.
            MoveWho::Entity(handle @ EntityHandle::ArmorStand(i)) if pos.is_some() => {
                (!self.common.armor_stands[i].is_removed())
                    .then(|| self.common.listed_body(handle, pos))
            }
            _ => self.push_body(target, pos, steps),
        }?;
        (body.piston_reaction != PushReaction::IgnoreEntity).then_some(body)
    }

    /// What the push needs from a live, non-spectating body.
    pub(in crate::game) fn push_body(
        &self,
        target: MoveWho,
        pos: Option<BlockPos>,
        steps: &StepHeights,
    ) -> Option<Body> {
        match target {
            MoveWho::Player => {
                let p = self.common.player.as_ref()?;
                if p.spectator {
                    return None;
                }
                Some(Body {
                    bounds: p.bounds(),
                    position: p.position,
                    on_ground: p.ground.on_ground,
                    step_height: steps.player,
                    piston_reaction: PushReaction::PushPull,
                    supported: pos.is_some() && p.ground.supporting_block == pos,
                })
            }
            MoveWho::Guest(conn) => {
                let host = self.server.lan.as_ref()?;
                let g = host.hub.guests.iter().find(|g| g.conn == conn)?;
                if g.dead || g.game_mode == 3 {
                    return None;
                }
                let p = g.position;
                Some(Body {
                    bounds: g.dimensions().bounds(Vec3::new(p[0], p[1], p[2])),
                    position: Vec3::new(p[0], p[1], p[2]),
                    on_ground: g.on_ground,
                    step_height: steps.guests.get(&conn).copied().unwrap_or(0.6),
                    piston_reaction: PushReaction::PushPull,
                    supported: g
                        .motion
                        .as_ref()
                        .is_some_and(|m| pos.is_some() && m.ground.supporting_block == pos),
                })
            }
            MoveWho::Entity(handle) => with_entity!(self, handle, entity => entity.is_alive())
                .then(|| self.common.listed_body(handle, pos)),
        }
    }

    /// `Entity.move(MoverType.PISTON, movement)` for one pushed entity.
    pub fn move_pushed(
        &mut self,
        target: MoveWho,
        before: Aabb,
        at: [f64; 2],
        step_height: f32,
        on_ground: bool,
        movement: Vec3,
        piston_direction: Direction,
        accumulators: &mut std::collections::HashMap<(u32, JoinSeq), PistonMovement>,
        pricked: &mut Vec<u32>,
    ) {
        let cause = MoverType::Piston;
        if target == MoveWho::Player {
            with_piston_noclip(piston_direction, || {
                let _ = self.server_move_local_player(cause, movement, false);
            });
            let after = self.common.player.as_ref().map_or(before, |p| p.bounds());
            self.piston_block_triggers(before, after);
            self.piston_cactus_contact(target, before, after, pricked);
            return;
        }
        let Some(selected) = self.stable_push_target(target) else {
            return;
        };
        let from = match target {
            MoveWho::Entity(handle) => with_entity!(self, handle, entity => entity.position()),
            _ => Vec3::ZERO,
        };
        let mut gate = self.entity_move(target, cause, movement);
        if gate.skipped() {
            return;
        }
        let movement = gate.delta();
        let Some(limited) = self.limit_piston(target, movement, accumulators) else {
            return;
        };
        if limited == Vec3::ZERO {
            with_piston_noclip(piston_direction, || {
                self.common.release_after_block_move(target);
                if let MoveWho::Entity(EntityHandle::Tnt(i)) = target {
                    let from = self.common.tnts[i].position;
                    self.tnt_piston_block_effects(i, from);
                }
                if let MoveWho::Entity(EntityHandle::Projectile(i)) = target {
                    let at = self.common.creatures.projectiles[i].position;
                    self.projectile_piston_block_effects(i, at);
                }
                self.piston_block_triggers(before, before);
                if let Some(target) = self.resolve_push_target(selected) {
                    self.piston_cactus_contact(target, before, before, pricked);
                }
            });
            return;
        }
        let projectile_from = match target {
            MoveWho::Entity(EntityHandle::Projectile(i)) => {
                self.common.creatures.projectiles.update(i, |p| {
                    if p.stuck_speed.length_squared() > 1.0e-7 {
                        p.stuck_speed = Vec3::ZERO;
                        p.velocity = Vec3::ZERO;
                    }
                    Some((i, p.position))
                })
            }
            _ => None,
        };
        let tnt = match target {
            MoveWho::Entity(EntityHandle::Tnt(i)) => {
                let t = &mut self.common.tnts[i];
                t.take_stuck();
                Some((
                    i,
                    t.position,
                    t.powder_snow_subject(),
                    t.collision_context(),
                ))
            }
            _ => None,
        };
        let entities = self.pushed_entity_colliders(target);
        with_piston_noclip(piston_direction, || {
            let collision = tnt
                .map(|(_, _, subject, context)| (context, subject))
                .or_else(|| self.pushed_living_collision(target));
            let resolved = pushed_movement(
                &mut gate,
                SceneCollisions {
                    world: &self.common.world,
                    binding: self.common.collision,
                    subject: collision.map(|(_, subject)| subject),
                },
                self.common.border.geometry(),
                at,
                before,
                limited,
                step_height,
                on_ground,
                collision.map(|(context, _)| context),
                &entities,
            )
            .unwrap_or(Vec3::ZERO);

            let moved = resolved.length_squared() > 1.0e-7
                || limited.length_squared() - resolved.length_squared() < 1.0e-7;
            if moved {
                self.reset_general_fall(target, from, resolved);
                self.apply_push(&mut gate, target, resolved);
            }
            let after = if moved {
                before.move_by(resolved)
            } else {
                before
            };
            let Some(target) = self.resolve_push_target(selected) else {
                self.piston_block_triggers(before, after);
                return;
            };
            self.set_pushed_on_ground(target, limited, resolved, tnt.map(|(_, _, s, c)| (s, c)));
            if let Some(target) =
                self.finish_pushed_restitution(selected, target, from, limited, resolved)
            {
                self.common.release_after_block_move(target);
                if let (MoveWho::Entity(EntityHandle::Tnt(i)), Some((_, from, _, _))) =
                    (target, tnt)
                {
                    self.finish_tnt_piston_move(i, from);
                }
                if let (MoveWho::Entity(EntityHandle::Projectile(i)), Some((_, from))) =
                    (target, projectile_from)
                {
                    self.projectile_piston_block_effects(i, from);
                }
            }
            self.piston_block_triggers(before, after);
            if let Some(target) = self.resolve_push_target(selected) {
                self.piston_cactus_contact(target, before, after, pricked);
            }
        });
    }

    /// `Entity.limitPistonMovement`: each axis may travel at most 0.51 blocks per game tick.
    pub(in crate::game) fn limit_piston(
        &mut self,
        target: MoveWho,
        movement: Vec3,
        accumulators: &mut std::collections::HashMap<(u32, JoinSeq), PistonMovement>,
    ) -> Option<Vec3> {
        let time = self.common.world.game_time;
        let limit = |state: &mut PistonMovement| state.limit(movement, || Ok::<i64, ()>(time)).ok();
        match target {
            MoveWho::Guest(conn) => {
                let host = self.server.lan.as_mut()?;
                limit(&mut host.hub.guests.motion_mut(conn)?.piston)
            }
            MoveWho::Entity(EntityHandle::Mob(i)) => limit(&mut self.common.mobs[i].piston),
            MoveWho::Entity(handle) => {
                let key = with_entity!(self, handle, entity => (entity.id(), entity.join_seq()));
                limit(accumulators.entry(key).or_default())
            }
            MoveWho::Player => unreachable!("local player limits its own move"),
        }
    }

    pub(in crate::game) fn apply_push(
        &mut self,
        gate: &mut MoveGate,
        target: MoveWho,
        movement: Vec3,
    ) {
        if gate.skipped() {
            return;
        }
        let id = match target {
            MoveWho::Player => {
                if let Some(p) = self.common.player.as_mut() {
                    p.position = p.position.add_vector(movement);
                }
                u32::try_from(self.own_entity_id()).ok()
            }
            MoveWho::Guest(conn) => self.server.lan.as_mut().and_then(|host| {
                host.hub.update_guest(conn, |g| {
                    for (slot, delta) in g.position.iter_mut().zip(movement.components()) {
                        *slot += delta;
                    }
                    if let Some(m) = g.motion.as_deref_mut() {
                        m.position = m.position.add_vector(movement);
                    }
                    g.entity_id as u32
                })
            }),
            MoveWho::Entity(handle) => {
                let id = with_entity_mut!(self, handle, entity => {
                    entity.move_entity(gate, movement);
                    entity.id()
                });
                Some(id)
            }
        };
        if let Some(id) = id {
            self.entity_section_moved_id(id);
            self.tell_entity_sections();
        }
    }

    /// `causeBounce`: a slime block sets the velocity along the movement axis.
    pub(in crate::game) fn bounce(&mut self, target: MoveWho, movement: Direction) {
        match target {
            MoveWho::Player => {
                if let Some(p) = self.common.player.as_mut() {
                    p.velocity = piston_push::bounce_velocity(movement, p.velocity);
                }
            }
            MoveWho::Guest(conn) => {
                if let Some(host) = self.server.lan.as_mut()
                    && let Some(motion) = host.hub.guests.motion_mut(conn)
                {
                    motion.velocity = piston_push::bounce_velocity(movement, motion.velocity);
                }
            }
            MoveWho::Entity(handle) => {
                with_entity_mut!(self, handle, entity => Entity::piston_slime_bounce(entity, movement))
            }
        }
    }

    /// Finish the collided base move shared by pistons, shulker lids and native moves.
    /// A piston's direct slime launch is not a collided move.
    pub(in crate::game) fn finish_pushed_restitution(
        &mut self,
        selected: PushTarget,
        target: MoveWho,
        from: Vec3,
        wanted: Vec3,
        moved: Vec3,
    ) -> Option<MoveWho> {
        let MoveWho::Entity(handle) = target else {
            return Some(target);
        };
        let server = self.common.is_logical_server();
        if let EntityHandle::Mob(i) = handle {
            let mob = &self.common.mobs[i];
            // The controlling player's side owns a ridden mob's restitution.
            // Even an observing copy updates support after a vertical move.
            let authoritative = if server {
                use super::commands::Target;
                let own = (self.common.riding == Some(mob.id))
                    .then(|| self.riding_player())
                    .flatten();
                let guests = self.server.lan.as_ref().into_iter().flat_map(|host| {
                    host.hub
                        .guests
                        .iter()
                        .filter(|g| host.hub.here(g) && g.vehicle == Some(mob.id))
                });
                let mut players = guests
                    .map(|g| (g.entity_id as u32, Target::Guest(g.conn)))
                    .chain(own.map(|(id, _)| (id, Target::Player)))
                    .peekable();
                let controller = if players.peek().is_some() {
                    self.first_seated_passenger(mob.id)
                        .filter(|&(_, _, is_player)| is_player)
                        .and_then(|(_, first, _)| players.find(|(id, _)| *id == first))
                } else {
                    None
                };
                !controller.is_some_and(|(_, rider)| self.native_player_controls(mob, rider))
            } else {
                false
            };
            if authoritative || wanted.y.abs() > 0.0 {
                let mut paused_hurt = None;
                let mut mobs = self.common.mobs.lift();
                let mut lent = sunrise_sim::health::LentLevel::take(&mut self.common.world);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let world = self.mob_world_base(&[]);
                    let mut level = sunrise_sim::health::DamageLevel::lent(
                        &self.common.world,
                        self.common.pack.assets.registry,
                        &mut lent,
                        Some(world_items!(self)),
                        Some(&self.common.pack.block_registries),
                        &self.common.damage_hooks,
                    )
                    .with_variants(world_variants!(self))
                    .with_reads(crate::game::entity_server::hit_reads_of!(self))
                    .with_mob_hurt_handoff(&mut paused_hurt);
                    mobs[i].update_pushed_support(&world, wanted, moved);
                    if authoritative {
                        mobs[i].restitute_movement(&world, &mut level, wanted, moved);
                    }
                }));
                lent.give_back(&mut self.common.world);
                self.common
                    .mobs
                    .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
                if let Err(payload) = result {
                    std::panic::resume_unwind(payload);
                }
                if let Some(hit) = paused_hurt
                    && let Some(index) = self.common.mob_index.find(&self.common.mobs, hit.victim)
                {
                    self.server.paused_mob_hurt.0 = Some(hit);
                    self.finish_mob_damage_at(index);
                }
            }
            return self.resolve_push_target(selected);
        }
        if let EntityHandle::General(i) = handle {
            return self.finish_general_move(selected, target, i, (wanted, moved));
        }
        // Stands retain support, but do not use nonliving restitution.
        if matches!(handle, EntityHandle::ArmorStand(_)) {
            return Some(target);
        }
        let can_simulate = match handle {
            EntityHandle::Boat(i) => self.boat_can_simulate_movement(self.common.boats[i].id),
            EntityHandle::Item(_)
            | EntityHandle::Tnt(_)
            | EntityHandle::Orb(_)
            | EntityHandle::Falling(_)
            | EntityHandle::Projectile(_)
            | EntityHandle::Rocket(_)
            | EntityHandle::Trident(_) => true,
            _ => server,
        };
        let Some((id, feet, effect)) = with_entity!(self, handle, entity => {
            (!entity.is_removed()).then(|| entity.ground_state()).flatten().map(|ground| {
                let feet = entity.position();
                (entity.id(), feet, self.body_motion(&game_side!(self)).legacy_contact(feet, ground))
            })
        }) else {
            return Some(target);
        };
        if server {
            match handle {
                EntityHandle::Tnt(i) => {
                    let t = &mut self.common.tnts[i];
                    if t.check_fall_damage(moved.y) {
                        let bounds = t.bounds();
                        self.nonliving_fall_on(id, feet, effect);
                        self.post_landing_event(feet, bounds, false);
                    }
                }
                EntityHandle::Falling(i) => self.finish_falling_piston_move(i, moved),
                EntityHandle::Cart(i) => {
                    let (distance, landing) =
                        self.cart_move_fall(&self.common.carts[i], from, moved);
                    self.common.carts.update(i, |c| c.fall_distance = distance);
                    if !self.finish_cart_landing(landing, effect, |game, power| {
                        let mut cart = game.carts[i].clone();
                        game.explode_cart(&mut cart, power)
                    }) {
                        return None;
                    }
                }
                _ => {}
            }
        }
        let MoveWho::Entity(handle) = self.resolve_push_target(selected)? else {
            return None;
        };
        let differs = sunrise_world::entity_move::different;
        if !can_simulate
            || !(wanted.y != moved.y || differs(wanted.x, moved.x) || differs(wanted.z, moved.z))
        {
            return Some(MoveWho::Entity(handle));
        }
        let drag = match handle {
            EntityHandle::Boat(_) => 1.0_f32,
            EntityHandle::Cart(_) => 0.95_f32,
            _ => 0.98_f32,
        };
        let boat_name = match handle {
            EntityHandle::Boat(i) => Some(self.common.boats[i].shared_type_name()),
            _ => None,
        };
        let kind = match handle {
            EntityHandle::Boat(_) => boat_name.as_deref()?,
            EntityHandle::Cart(i) => self.common.carts[i].kind.id(),
            EntityHandle::Projectile(i) => self.common.creatures.projectiles[i].entity_type(),
            EntityHandle::Item(_) => "minecraft:item",
            EntityHandle::Orb(_) => "minecraft:experience_orb",
            EntityHandle::Falling(_) => "minecraft:falling_block",
            EntityHandle::Tnt(_) => "minecraft:tnt",
            EntityHandle::Rocket(_) => "minecraft:firework_rocket",
            EntityHandle::Trident(_) => "minecraft:trident",
            EntityHandle::ArmorStand(_) => {
                sunrise_gameplay_common::game::armor_stands_common::TYPE_ID
            }
            EntityHandle::Mob(_) | EntityHandle::General(_) => {
                unreachable!("a mob or a general row finished its move above")
            }
        };
        let response = with_entity!(self, handle, entity => {
            self.body_motion(&game_side!(self)).restitute((id, kind),
                sunrise_sim::motion::BodyMove { wanted, moved, effect },
                entity.get_gravity(), drag, entity.velocity(), entity.body_fall_distance())
        });
        let velocity = self.server_finish_body_restitution((id, kind), feet, response);
        let MoveWho::Entity(handle) = self.resolve_push_target(selected)? else {
            return None;
        };
        with_entity_mut!(self, handle, entity => *entity.velocity_mut() = velocity);
        Some(MoveWho::Entity(handle))
    }

    /// Reset fall distance before placement so the fall check only adds the rest of the move.
    fn reset_general_fall(&mut self, target: MoveWho, from: Vec3, moved: Vec3) {
        let MoveWho::Entity(EntityHandle::General(i)) = target else {
            return;
        };
        let row = &self.resident_entities.general.rows[i];
        if row.no_physics || row.fall_distance == 0.0 || moved.length_squared() < 1.0 {
            return;
        }
        let reset = &self.res.motion().fall_reset;
        let hit = sunrise_sim::entities::fall_reset_hit(
            &self.world,
            &self.res.motion().fluids,
            |p| reset.contains(&*self.res.facts().get(self.world.block(p.x, p.y, p.z)).name),
            from,
            moved,
        );
        if hit {
            self.resident_entities.general.rows[i].fall_distance = 0.0;
        }
    }

    fn finish_general_move(
        &mut self,
        selected: PushTarget,
        target: MoveWho,
        i: usize,
        (wanted, moved): (Vec3, Vec3),
    ) -> Option<MoveWho> {
        let row = &self.common.resident_entities.general.rows[i];
        if row.no_physics {
            return Some(target);
        }
        let (id, feet, velocity, fall) = (row.id, row.position, row.velocity, row.fall_distance);
        let gravity = row.get_gravity();
        let ground = sunrise_world::ground_support::GroundState::from_ground_flag(row.on_ground);
        let kind =
            sunrise_gameplay_common::game::general::general_type_catalog().key(row.type_slot);
        let differs = sunrise_world::entity_move::different;
        let collided =
            wanted.y != moved.y || differs(wanted.x, moved.x) || differs(wanted.z, moved.z);
        let velocity = if collided {
            let response = {
                let side = game_side!(self);
                let motion = self.common.body_motion(&side);
                let effect = motion.legacy_contact(feet, &ground);
                motion.restitute(
                    (id, kind),
                    sunrise_sim::motion::BodyMove {
                        wanted,
                        moved,
                        effect,
                    },
                    gravity,
                    0.98_f32,
                    velocity,
                    fall,
                )
            };
            self.server_finish_body_restitution((id, kind), feet, response)
        } else {
            velocity
        };
        let MoveWho::Entity(handle) = self.resolve_push_target(selected)? else {
            return None;
        };
        let EntityHandle::General(i) = handle else {
            return None;
        };
        let mut velocity = velocity;
        {
            let side = game_side!(self);
            self.common
                .body_motion(&side)
                .apply_block_speed(feet, &mut velocity);
        }
        self.common.resident_entities.general.rows[i].velocity = velocity;
        Some(MoveWho::Entity(handle))
    }

    /// Server arm of [`Game::side_move_local_player`]: `Entity.move(mover, …)`
    /// with acting-guest landing handoff (`trample_farmland`, `finish_guest_damage_before_resume`).
    pub(in crate::game) fn server_move_local_player(
        &mut self,
        mover: MoverType,
        movement: Vec3,
        auto_jump: bool,
    ) -> sunrise_game_data::Result<Option<Vec3>> {
        use sunrise_sim::landing::PlayerLandingPause;

        let Some(player) = self.common.player.as_ref() else {
            return Ok(None);
        };
        let (before, id, conn, dim) = (
            player.position,
            player.entity_id,
            self.server.lan_acting,
            self.common.dim,
        );
        macro_rules! motion_phase {
            ($world:ident, $body:expr) => {{
                if self.server.lan_acting != conn || self.common.dim != dim {
                    return Ok(None);
                }
                let entities = self.entity_collisions(Mover::Player);
                let section_scope = self.player_section_scope();
                let Some(player) = self.common.player.as_mut().filter(|p| p.entity_id == id) else {
                    return Ok(None);
                };
                let side = server_game_side!(self);
                let _sections =
                    section_scope.map(|scope| scope.begin(&mut self.common.native_lifecycle));
                let mut $world = MotionWorld {
                    side: &side,
                    entities: &entities,
                    player,
                    scene: &mut self.common.world,
                    assets: &self.common.pack.assets,
                    collision_binding: self.common.collision,
                    resources: self.common.motion,
                    health: &self.common.health_resources,
                    loop_context: &self.common.loop_context,
                    damage_hooks: &self.common.damage_hooks,
                    tag_items: Some(server_world_items!(self)),
                    block_registries: Some(&self.common.pack.block_registries),
                };
                $body
            }};
        }
        let pause = motion_phase!(
            world,
            world.move_ordinary(
                &mut MoveGate::new(),
                mover,
                movement,
                auto_jump,
                conn.is_some(),
            )
        )?;
        if let Some(pause) = pause {
            let landing = match pause.landing {
                PlayerLandingPause::Trample(trample) => {
                    self.trample_farmland(id as u32, true, trample);
                    motion_phase!(world, world.finish_trample_fall(trample))?
                }
                PlayerLandingPause::Damage(landing) => Some(landing),
            };
            if let Some(landing) = landing {
                if self.finish_guest_damage_before_resume().is_err() {
                    return Ok(None);
                }
                motion_phase!(world, {
                    world.finish_guest_landing(landing)?;
                    pause.movement.finish(&mut world)
                })?;
            } else {
                motion_phase!(world, pause.movement.finish(&mut world))?;
            }
        }
        Ok(self
            .common
            .player
            .as_ref()
            .map(|player| player.position.subtract(before)))
    }

    /// `Block.pushEntitiesUp(old, new, level, pos)` before the block at `pos` changes.
    pub(in crate::game) fn push_entities_up(
        &mut self,
        pos: BlockPos,
        old: sunrise_world::registry::BlockStateId,
        new: sunrise_world::registry::BlockStateId,
        write: impl FnOnce(&mut Self) -> bool,
    ) -> bool {
        if new == old {
            return write(self);
        }
        let at = [pos.x, pos.y, pos.z];
        let shape = |state| {
            let mut scene = SceneCollisions {
                world: &self.common.world,
                binding: self.common.collision,
                subject: None,
            };
            self.common
                .collision
                .collision_shape(&mut scene, state, at)
                .ok()
        };
        let (Some(old_shape), Some(new_shape)) = (shape(old), shape(new)) else {
            return write(self);
        };
        let Ok(added) = old_shape.join_unoptimized(&new_shape, ShapeOp::ONLY_SECOND) else {
            return write(self);
        };
        let added = added.moved(Vec3::new(
            f64::from(pos.x),
            f64::from(pos.y),
            f64::from(pos.z),
        ));
        let Some(bounds) = added.bounds() else {
            return write(self);
        };
        let mut lift: Vec<_> = self
            .push_targets()
            .filter_map(|target| {
                let (body, id, join) = match target {
                    MoveWho::Player => {
                        let p = self.common.player.as_ref().filter(|p| !p.spectator)?;
                        (p.bounds(), p.entity_id as u32, None)
                    }
                    MoveWho::Guest(conn) if Some(conn) != self.server.lan_acting => {
                        let g = self
                            .server
                            .lan
                            .as_ref()?
                            .hub
                            .guest(conn)
                            .filter(|g| !g.spectator())?;
                        let [x, y, z] = g.position;
                        (
                            g.dimensions().bounds(Vec3::new(x, y, z)),
                            g.entity_id as u32,
                            None,
                        )
                    }
                    MoveWho::Guest(_) => return None,
                    MoveWho::Entity(handle) => with_entity!(self, handle, entity => {
                        if entity.is_removed() {
                            return None;
                        }
                        (Entity::bounds(entity), entity.id(), Some(entity.join_seq()))
                    }),
                };
                body.intersects(bounds).then_some((target, id, join, None))
            })
            .collect();
        for &mut (hint, id, join, ref mut moved) in &mut lift {
            let Some(target) = self.resolve_push_target(PushTarget { hint, id, join }) else {
                continue;
            };
            let body = match target {
                MoveWho::Player => self.common.player.as_ref().unwrap().bounds(),
                MoveWho::Guest(conn) => {
                    let g = self.server.lan.as_ref().unwrap().hub.guest(conn).unwrap();
                    let [x, y, z] = g.position;
                    g.dimensions().bounds(Vec3::new(x, y, z))
                }
                MoveWho::Entity(handle) => {
                    with_entity!(self, handle, entity => Entity::bounds(entity))
                }
            };
            let dy = 1.0 + added.collide(Axis::Y, body.move_by(Vec3::new(0.0, 1.0, 0.0)), -1.0);
            let (from, motion_y) = match target {
                MoveWho::Player => {
                    let p = self.common.player.as_mut().unwrap();
                    let from = p.position;
                    p.position.y += dy;
                    (from, None)
                }
                MoveWho::Guest(conn) => self
                    .server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |g| {
                        let [x, y, z] = g.position;
                        g.position[1] += dy;
                        let motion_y = g.motion.as_mut().map(|m| {
                            let y = m.position.y;
                            m.position.y += dy;
                            y
                        });
                        (Vec3::new(x, y, z), motion_y)
                    })
                    .unwrap(),
                MoveWho::Entity(handle) => with_entity_mut!(self, handle, entity => {
                    let pos = entity.position_mut();
                    let from = *pos;
                    pos.y += dy;
                    (from, None)
                }),
            };
            let to = [from.x, from.y + dy, from.z];
            *moved = Some((from, to, motion_y));
            self.entity_section_moved_id(id);
        }
        if write(self) {
            for (_, id, _, moved) in lift {
                if let Some((from, to, _)) = moved {
                    self.common
                        .entity_section_entered(id, from.components(), to);
                }
            }
            return true;
        }
        for (target, id, join, moved) in lift.into_iter().rev() {
            let Some((from, _, motion_y)) = moved else {
                continue;
            };
            let Some(target) = self.resolve_push_target(PushTarget {
                hint: target,
                id,
                join,
            }) else {
                continue;
            };
            match target {
                MoveWho::Player => self.common.player.as_mut().unwrap().position = from,
                MoveWho::Guest(conn) => {
                    self.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(conn, |g| {
                            g.position = from.components();
                            if let (Some(m), Some(y)) = (&mut g.motion, motion_y) {
                                m.position.y = y;
                            }
                        });
                }
                MoveWho::Entity(handle) => with_entity_mut!(self, handle, entity => {
                    *entity.position_mut() = from;
                }),
            }
            self.entity_section_joined(id, from.components());
        }
        false
    }

    /// `ShulkerBoxBlockEntity.moveCollidedEntities` for the server's bodies.
    pub(in crate::game) fn push_shulker_entities(&mut self, pushes: Vec<ShulkerPush>) {
        for (_, facing, lid) in pushes {
            let (movement, area) = shulker_lid(facing, lid);
            self.lid_push(Lid::Box, area, movement);
        }
    }

    /// The ground recorded after placing the body during `Entity.move`.
    pub(in crate::game) fn set_pushed_on_ground(
        &mut self,
        target: MoveWho,
        wanted: Vec3,
        moved: Vec3,
        tnt: Option<(
            sunrise_level::collisions::PowderSnowSubject<'static>,
            sunrise_world::collision_context::CollisionContext<'static>,
        )>,
    ) {
        use sunrise_world::entity_move::different;
        let MoveWho::Entity(handle) = target else {
            return;
        };
        if let EntityHandle::Mob(i) = handle {
            self.common.mobs[i].ground.horizontal_collision =
                different(wanted.x, moved.x) || different(wanted.z, moved.z);
            return;
        }
        // A general row's `Entity.move` end, for every cause: the flags, the fall check and
        // the blocked axes; a `noPhysics` row never gets here.
        if let EntityHandle::General(i) = handle {
            self.resident_entities.general.rows[i].finish_move(wanted, moved);
            return;
        }
        let authoritative = match handle {
            EntityHandle::Boat(i) => self.boat_can_simulate_movement(self.common.boats[i].id),
            _ => self.common.is_logical_server(),
        };
        if let EntityHandle::ArmorStand(i) = handle {
            let stand = &mut self.common.armor_stands[i];
            stand.ground.horizontal_collision =
                different(wanted.x, moved.x) || different(wanted.z, moved.z);
            if authoritative || wanted.y.abs() > 0.0 {
                let items = match (
                    &self.common.server_items,
                    self.server.world_tables.0.as_ref(),
                ) {
                    (Some(items), _) => &**items,
                    (None, Some(tables)) => &tables.items,
                    (None, None) => self.common.res.items(),
                };
                let _ = stand.update_retained_ground(
                    &self.common.world,
                    self.common.collision,
                    &items.entity_type_tags,
                    wanted.y < 0.0 && wanted.y != moved.y,
                    Some(moved),
                );
            }
            return;
        }
        let motion = sunrise_sim::motion::BodyMotion {
            world: &self.common.world,
            collision: self.common.collision,
            facts: self.common.res.facts(),
            blocks: Some(&self.common.res.motion().blocks),
            context: &self.common.loop_context,
            side: None,
        };
        with_entity_mut!(self.common, handle, entity => {
            let (at, bounds, id) = (entity.position(), entity.bounds(), entity.id());
            let (subject, context) = tnt.map_or_else(
                || {
                    let context = if matches!(handle, EntityHandle::Item(_)) {
                        sunrise_world::collision_context::CollisionContext::for_entity(
                            sunrise_world::collision_context::CollisionEntity {
                                identity: sunrise_world::entity_obstruction::EntityIdentity(u64::from(id)),
                                kind: sunrise_world::collision_context::EntityContextKind::Item,
                                y: at.y,
                                descending: false,
                                main_hand: None,
                            },
                            false,
                        )
                    } else {
                        sunrise_sim::motion::BodyMotion::entity_context(id, at)
                    };
                    (None, context)
                },
                |(subject, context)| (Some(subject), context),
            );
            if let Some(ground) = entity.ground_state_mut() {
                motion.record_move(
                    sunrise_sim::motion::BodyCollision {
                        footprint: (at, bounds),
                        movement: (wanted, moved),
                        authoritative,
                        context,
                        subject,
                    },
                    ground,
                );
            } else {
                entity.set_piston_on_ground(wanted.y < 0.0 && wanted.y != moved.y);
            }
        });
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::boats_common::BoatsCommonGameExt1 as _;

use sunrise_gameplay_common::game::collision_source::CollisionSourceGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;
use sunrise_gameplay_common::game::entity_sections::player::PlayerGameExt1 as _;

use sunrise_gameplay_common::game::move_gate::MoveGateGameExt1 as _;

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt2 as _;

use sunrise_gameplay_common::game::piston_push::PistonPushGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
