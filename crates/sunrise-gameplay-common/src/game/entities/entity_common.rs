//! Common entity helpers shared by client and server (`PlacementWorld`, spawn/placement checks,
//! body motion/restitution/item contacts, suffocation, dismount, critical check, and sound wrapper).
use super::{Hit, entity::Entity as _};
use crate::game::{
    armor_stands_common::overlaps as stand_overlaps, cushions_common::aabb as cushion_aabb,
};
use std::sync::Arc;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_gameplay_common::painting_variants::{painting_coexists, painting_facing};
use sunrise_items::items::ItemStack;
use sunrise_level::collisions::SceneCollisions;
use sunrise_sim::{
    cushion::blocks_in,
    entities::{ArmorStand, ItemEntity, Painting},
};

/// Optional resolver for entities currently lent out of their tables during an
/// inside-block sweep, returning their command target and live position.
pub type InsideSources<'a> =
    Option<&'a dyn Fn(u32) -> Option<(crate::game::commands_common::Target, [f64; 3])>>;

/// Whether an item entity can take fire damage (`!fireResistant` in vanilla's
/// `ItemEntity.fireImmune`).
pub fn item_can_burn<'r>(game: &impl crate::game::SideGame<'r>, item: &ItemEntity) -> bool {
    game.stack_can_be_hurt_by(&item.stack, "minecraft:in_fire") != Some(false)
}

pub struct PlacementWorld<'a, 'r, S: crate::game::SideGame<'r>>(
    pub &'a S,
    pub std::marker::PhantomData<fn(&'r ()) -> &'r ()>,
);

impl<'__carrier, S: crate::game::SideGame<'__carrier>> sunrise_sim::cushion::World
    for PlacementWorld<'_, '__carrier, S>
{
    fn outline_bounds(&self, p: [i32; 3]) -> Option<Aabb> {
        let boxes = self.0.outline_boxes(p);
        let mut shape = *boxes.first()?;
        for b in &boxes[1..] {
            for k in 0..3 {
                shape[k] = shape[k].min(b[k]);
                shape[k + 3] = shape[k + 3].max(b[k + 3]);
            }
        }
        Some(Aabb::new(
            Vec3::new(shape[0], shape[1], shape[2]),
            Vec3::new(shape[3], shape[4], shape[5]),
        ))
    }

    fn collision_shapes(&self, bounds: Aabb) -> Vec<sunrise_core::geometry::VoxelShape> {
        let binding = self.0.res.collision();
        let mut world = SceneCollisions {
            world: &self.0.world,
            binding,
            subject: None,
        };
        // Include neighbour blocks whose collision shape can reach outside their cell.
        blocks_in(bounds.inflate(Vec3::new(1.0, 1.0, 1.0)))
            .filter_map(|p| {
                let state = self.0.world.block(p[0], p[1], p[2]);
                binding
                    .collision_shape(&mut world, state, p)
                    .ok()
                    .map(|s| s.moved(Vec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))))
            })
            .collect()
    }

    fn suffocating(&self, p: [i32; 3]) -> bool {
        self.0
            .state_suffocating(self.0.world.block(p[0], p[1], p[2]))
    }
}

impl crate::game::CommonGame<'_> {
    /// `particles::around_self` about mob `id`.
    pub fn particles_around_mob(
        &mut self,
        _id: u32,
        _kind: &'static str,
        _burst: sunrise_level::particle_options::Burst,
    ) {
    }

    /// `BlockBehaviour.Properties.isSuffocating` of a state.
    pub fn state_suffocating(&self, state: sunrise_world::registry::BlockStateId) -> bool {
        let facts = self.res.facts().get(state);
        let name = facts.name.as_ref();
        match name {
            "minecraft:dirt_path"
            | "minecraft:farmland"
            | "minecraft:mud"
            | "minecraft:soul_sand" => true,
            "minecraft:piston" | "minecraft:sticky_piston" => self
                .pack
                .assets
                .registry
                .state(state)
                .is_some_and(|s| s.properties.get("extended").is_some_and(|p| p == "false")),
            "minecraft:mangrove_roots" | "minecraft:moving_piston" => false,
            _ if name.ends_with("_leaves")
                || name.ends_with("glass")
                || name.ends_with("copper_grate") =>
            {
                false
            }
            _ if name.ends_with("shulker_box") => true,
            _ => {
                facts.full_collision
                    && self
                        .res
                        .items()
                        .block_in_tag("minecraft:causes_suffocation", name)
            }
        }
    }

    /// `ArmorStandItem.useOn` up to its `FAIL`s (id = 0; server caller assigns its entity id).
    pub fn armor_stand_spawn(
        &self,
        hit: &Hit,
        stack: &ItemStack,
    ) -> Option<([i32; 3], ArmorStand, [i32; 4])> {
        if hit.face == [0, -1, 0] {
            return None;
        }
        let clicked = self
            .res
            .facts()
            .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]));
        let pos = if clicked.replaceable {
            hit.pos
        } else {
            std::array::from_fn(|i| hit.pos[i] + hit.face[i])
        };
        let foot = Vec3::new(
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        );
        let yaw =
            (((self.yaw - 180.0 + 180.0).rem_euclid(360.0) - 180.0 + 22.5) / 45.0).floor() * 45.0;
        let (mut stand, uuid) = ArmorStand::new_with_uuid(0, foot, yaw);
        if let Some(name) = stack.custom_name() {
            stand.custom_name = Some(sunrise_game_data::text::Text::literal(name));
        }
        let b = stand.bounds();
        let (lo, hi) = (b.min_position(), b.max_position());
        let e = 1.0e-7;
        let bounds = Aabb::new(
            Vec3::new(lo.x + e, lo.y + e, lo.z + e),
            Vec3::new(hi.x - e, hi.y - e, hi.z - e),
        );
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        if !scene.unobstructed(bounds).unwrap_or(false)
            || self
                .armor_stands
                .iter()
                .any(|o| stand_overlaps(o.bounds(), bounds))
        {
            return None;
        }
        Some((pos, stand, uuid))
    }

    /// `HangingEntity.survives`: no block collision, solid/diode wall behind, no overlapping hanging.
    pub fn painting_survives(&self, p: &Painting) -> bool {
        let (lo, hi) = p.bounds();
        let bounds = Aabb::new(
            Vec3::new(lo[0], lo[1], lo[2]),
            Vec3::new(hi[0], hi[1], hi[2]),
        );
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        if !scene.unobstructed(bounds).unwrap_or(false) {
            return false;
        }
        let s = p.step().map(f64::from);
        let (slo, shi) = (
            std::array::from_fn::<f64, 3, _>(|i| lo[i] - s[i] * 0.5 + 1.0e-7),
            std::array::from_fn::<f64, 3, _>(|i| hi[i] - s[i] * 0.5 - 1.0e-7),
        );
        for x in slo[0].floor() as i32..=shi[0].floor() as i32 {
            for y in slo[1].floor() as i32..=shi[1].floor() as i32 {
                for z in slo[2].floor() as i32..=shi[2].floor() as i32 {
                    let f = self.res.facts().get(self.world.block(x, y, z));
                    if !(f.legacy_solid || f.has_class("DiodeBlock")) {
                        return false;
                    }
                }
            }
        }
        painting_coexists(p, &self.paintings, &self.item_frames)
    }

    /// `LivingEntity.dismountVehicle` from a removed vehicle whose position was at `vehicle_y`.
    pub fn dismount_removed_vehicle(&mut self, vehicle_y: f64) {
        self.riding = None;
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let (w, h) = (
            f64::from(p.dimensions.width),
            f64::from(p.dimensions.height),
        );
        let at = self.removed_vehicle_dismount_point(p.position, w, h, vehicle_y);
        if let Some(p) = self.player.as_mut() {
            self.previous_position = p.position;
            p.position = at;
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
    }

    /// Free-position branch shared by removed vehicles and portal dismounts.
    pub fn removed_vehicle_dismount_point(
        &self,
        position: Vec3,
        w: f64,
        h: f64,
        vehicle_y: f64,
    ) -> Vec3 {
        let [min_x, min_z, max_x, max_z] = self.border.bounds();
        let border = sunrise_world::level::portal_travel::BorderBounds {
            min_x,
            min_z,
            max_x,
            max_z,
        };
        let b = self.res.behaviours();
        let mut at = Vec3::new(position.x, position.y.max(vehicle_y), position.z);
        if w <= 4.0 && h <= 4.0 {
            let center = at.add_vector(Vec3::new(0.0, h / 2.0, 0.0));
            let size = Vec3::new(w, h, w);
            let free = sunrise_world::level::portal_travel::find_free_position(
                Aabb::of_size(center, size),
                center,
                size,
                &border,
                |q| {
                    b.collision_shape(self.world.block(q.x, q.y, q.z))
                        .map(|s| s.moved(Vec3::new(f64::from(q.x), f64::from(q.y), f64::from(q.z))))
                },
            );
            if let Some(free) = free {
                at = free.add_vector(Vec3::new(0.0, -h / 2.0, 0.0));
            }
        }
        at
    }

    /// `Player.canCriticalAttack` for a living target at attack strength `scale`.
    pub fn critical_attack(&self, scale: f32) -> bool {
        let player = self.player.as_ref();
        let falling = player.is_some_and(|p| {
            !p.ground.on_ground
                && p.fall_distance > 0.0
                && !p.fluids.was_touching_water
                && !p.on_climbable
                && !p.sprinting
        });
        let restricted = self.potions.effects.get("blindness").is_some() || self.riding.is_some();
        scale > 0.9 && falling && !restricted
    }

    pub fn body_motion<'a>(
        &'a self,
        side: &'a dyn sunrise_sim::motion::SideLevel,
    ) -> sunrise_sim::motion::BodyMotion<'a> {
        sunrise_sim::motion::BodyMotion {
            world: &self.world,
            collision: self.collision,
            facts: self.res.facts(),
            blocks: Some(&self.res.motion().blocks),
            context: &self.loop_context,
            side: Some(side),
        }
    }
}

pub trait EntityCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `HangingEntityItem.useOn` check: true once a surviving painting variant fits.
    fn painting_fits(&self, hit: &Hit, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:painting" {
            return false;
        }
        let Some(facing) = painting_facing(hit.face) else {
            return false;
        };
        let pos = [
            hit.pos[0] + hit.face[0],
            hit.pos[1],
            hit.pos[2] + hit.face[2],
        ];
        let mut candidate = Painting {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 0,
            pos,
            facing,
            variant: Arc::from(""),
            asset: None,
            width: 1,
            height: 1,
            check_interval: 0,
            ticks: 0,
        };
        // Not world_paintings!: its trait import is unused where SideGame is a supertrait.
        let paintings = match crate::game::Side::world_tables(self.side_state()) {
            Some(t) => &t.painting_variants,
            None => &self.painting_variants,
        };
        let mut any = false;
        for id in &paintings.placeable {
            let Some(v) = paintings.get(id) else {
                continue;
            };
            (candidate.width, candidate.height) = (v.width, v.height);
            if self.painting_survives(&candidate) {
                any = true;
                break;
            }
        }
        if !any {
            return false;
        }
        if let Some(v) = paintings.of_stack(stack).cloned() {
            (candidate.variant, candidate.width, candidate.height) = (v.id, v.width, v.height);
            return self.painting_survives(&candidate);
        }
        true
    }

    /// Play an entity-bound sound following the acting player on the active side.
    fn play_player_bound_sound(
        &mut self,
        event: &str,
        source: usize,
        volume: f32,
        pitch: f32,
        except_actor: bool,
    ) {
        self.side_play_player_bound_sound(event, source, volume, pitch, except_actor);
    }

    /// `CushionItem.useOn` up to its `FAIL`s.
    fn cushion_spawn(&self, hit: &Hit) -> Option<(Vec3, Aabb)> {
        let mut adjusted = *hit;
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if self.items().block_in_tag(
            "minecraft:cushion_uses_collision_shape",
            &self.res.facts().get(state).name,
        ) && let Some(eye) = self.eye_position()
        {
            let start = Vec3::new(eye[0], eye[1], eye[2]);
            let point = Vec3::new(hit.point[0], hit.point[1], hit.point[2]);
            let end = point.add_vector(point.subtract(start).normalize().scale(0.001));
            let binding = self.res.collision();
            let mut world = SceneCollisions {
                world: &self.world,
                binding,
                subject: None,
            };
            if let Ok(shape) = binding.collision_shape(&mut world, state, hit.pos)
                && let Some(result) = shape.clip(
                    start,
                    end,
                    sunrise_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
                )
            {
                adjusted.face = result.direction.normal();
                adjusted.point = [result.location.x, result.location.y, result.location.z];
            }
        }
        let hit = &adjusted;
        if hit.face != [0, 1, 0] {
            return None;
        }
        let clicked = self
            .res
            .facts()
            .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]));
        let pos = if clicked.replaceable {
            hit.pos
        } else {
            std::array::from_fn(|i| hit.pos[i] + hit.face[i])
        };
        let y = hit.point[1];
        let position = Vec3::new(f64::from(pos[0]) + 0.5, y, f64::from(pos[2]) + 0.5);
        let bounds = cushion_aabb((
            [position.x - 0.5, position.y, position.z - 0.5],
            [position.x + 0.5, position.y + 0.25, position.z + 0.5],
        ));
        sunrise_sim::cushion::can_place(&PlacementWorld(self, std::marker::PhantomData), bounds)
            .then_some((position, bounds))
    }

    /// Step on the retained block before the simulating body's inside scan.
    fn body_step_on(
        &mut self,
        entity: (u32, &str),
        footprint: (Vec3, Aabb),
        ground: &sunrise_world::ground_support::GroundState,
        velocity: &mut Vec3,
    ) -> Vec<mod_api::EntityWrite> {
        self.side_body_step_on(entity, footprint, ground, velocity)
    }

    fn tick_item_with_block_contacts(
        &mut self,
        item: &mut ItemEntity,
        escape_speed: Option<f32>,
        entity_boxes: &[Aabb],
        client: bool,
        sources: InsideSources<'_>,
        splash: &mut impl FnMut(sunrise_sim::entities::ItemSplash),
    ) -> (bool, bool) {
        let no_physics = escape_speed.is_some();
        let before = item.velocity;
        let heights =
            item.push_by_fluid(&self.world, &self.res.motion().fluids, &self.loop_context);
        item.water_entry(heights.0 > 0.0, item.ticks == 0, before, splash);
        let in_lava = item.ticks > 0 && heights.1 > 0.0;
        item.lava_fall(in_lava);
        if client || item.fire_ticks > 0 && !item_can_burn(self, item) {
            item.fire_ticks = item.fire_ticks.min(0);
        } else if item.fire_ticks > 0 {
            if item.fire_ticks % 20 == 0 && !in_lava {
                self.side_hurt_lent_item_entity(item, 1.0, "minecraft:on_fire");
            }
            item.fire_ticks -= 1;
            if item.health <= 0 {
                item.previous = item.position;
                return (false, true);
            }
        }
        let mut step = item.tick_move_before_block_effects(
            &self.world,
            self.res.facts(),
            self.collision,
            no_physics,
            escape_speed,
            heights,
            entity_boxes,
            Some(self.border.geometry()),
            Some(&self.res.motion().blocks),
            &self.loop_context,
        );
        item.check_fall(&mut step, |fall, from, moved| {
            self.vehicle_fall_reset(fall, from, moved)
        });
        if !client && let Some((on, distance)) = step.landed() {
            self.side_item_fall_on(item.id, on, distance);
        }
        if let Some(response) = item.move_restitution(
            self.body_motion(&game_side!(self, self.side_game_side_fields())),
            step,
        ) {
            item.velocity = self.side_finish_body_restitution(
                (item.id, "minecraft:item"),
                item.position,
                response,
            );
        }
        if let Some(pos) = step.vertical_contact {
            let state = self.world.block(pos.x, pos.y, pos.z);
            let position = item.position;
            let mut discarded = false;
            if crate::game::api_values::block_state::block_listens(
                mod_api::HookId::ItemAfterVerticalCollision,
                state,
            ) {
                let mut stack = Some(item.stack.clone());
                self.native_item_after_vertical_collision(
                    crate::game::native_hooks::ItemAfterVerticalCollisionCall {
                        dimension: self.dim.id(),
                        client,
                        entity: item.id,
                        state,
                        block_pos: [pos.x, pos.y, pos.z],
                        item_pos: [position.x, position.y, position.z],
                        stack: &mut stack,
                        discard: &mut discarded,
                    },
                );
                match stack {
                    Some(stack) => item.stack = stack,
                    None => discarded = true,
                }
            }
            if discarded || item.stack.count == 0 {
                return (false, true);
            }
        }
        if !no_physics {
            item.apply_move_speed(
                self.body_motion(&game_side!(self, self.side_game_side_fields())),
                step,
            );
            if !self.side_item_post_move_contacts(item, client, sources) {
                return (false, true);
            }
        }
        item.finish_move_block_effects(&self.world, self.res.facts(), step);
        item.push_by_bubble_column(
            &self.world,
            self.pack.assets.registry,
            self.res.facts(),
            self.collision,
        );
        let side = game_side!(self, self.side_game_side_fields());
        let alive = item.tick_settle(
            &self.world,
            self.pack.assets.registry,
            self.res.facts(),
            step,
            &self.loop_context,
            &side,
        );
        let before = item.velocity;
        let (water, _) =
            item.push_by_fluid(&self.world, &self.res.motion().fluids, &self.loop_context);
        item.water_entry(water > 0.0, false, before, splash);
        (alive, false)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> EntityCommonGameExt1<'__sg> for G {}

use crate::game::side_ops::mobs_rest::MobsRestSideGameExt as _;

use crate::game::tables::GameItems as _;

use crate::game::GameGameExt1 as _;

use crate::game::native_hooks::item_after_vertical_collision::GameHooks as _;

use crate::game::native_components::NativeComponentsGameExt1 as _;
