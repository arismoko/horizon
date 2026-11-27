use super::{Hit, InteractTarget, Keys, entity::Entity};
#[allow(unused_imports)]
use std::sync::Arc;
use sunrise_core::{
    geometry::{Aabb, Vec3, trig},
    position::BlockPos,
    random::RandomSource,
};
use sunrise_game_data::scoreboard::pushable_by;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_gameplay_common::game::minecarts::CART_KEY_Y;
use sunrise_gameplay_common::game::minecarts::CartKind;
use sunrise_gameplay_common::game::minecarts::Minecart;
use sunrise_gameplay_common::game::minecarts::Rail;
use sunrise_gameplay_common::game::minecarts::SEAT;
use sunrise_gameplay_common::game::minecarts::Shape;
use sunrise_gameplay_common::game::minecarts::can_ride_cart;
use sunrise_gameplay_common::game::minecarts::cart_at;
use sunrise_gameplay_common::game::minecarts::cart_push_from;
use sunrise_gameplay_common::game::minecarts::container_keep;
use sunrise_gameplay_common::game::minecarts::furnace_fuel_after;
use sunrise_gameplay_common::game::minecarts::furnace_speed;
use sunrise_gameplay_common::game::minecarts::java_degrees;
use sunrise_gameplay_common::game::minecarts::offered_pickup;
use sunrise_gameplay_common::game::minecarts::pickup_instead_of_push;
use sunrise_gameplay_common::game::minecarts::push_along;
use sunrise_gameplay_common::game::minecarts::takes_push_from_cart;
use sunrise_gameplay_common::game::minecarts::tnt_ignites_by_collision;
use sunrise_gameplay_common::game::{
    api_values::TryIntoApi, cart_rails::dispatch_cart_rail_direction,
    native_level_signals::ReadLevel,
};
use sunrise_gameplay_common::minecart_cells::CartCells;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::entity_rules::EntityRules;
use sunrise_sim::join_seq::{JoinList, ListStamp};
use sunrise_sim::mobs::{Mob, seen_grid::SeenGrid};
use sunrise_sim::mobs::{Species, Who};
use sunrise_world::registry::BlockStateId;

#[path = "minecarts/save_lending_server.rs"]
pub(in crate::game) mod save_lending;

#[cfg(test)]
#[path = "minecarts/hit_server.rs"]
pub(in crate::game) mod hit;

#[path = "minecarts/section_steps.rs"]
pub(in crate::game) mod section_steps;

/// The carts' and mobs' boxes for one tick's cart pushes (`push_from_cart`,
/// `pickup_from_cart`), so a cart reads its neighbours instead of every cart and
/// every mob of the level. Both answer in slot order, as the scans did.
///
/// The cart grid follows each ticked cart ([`CartIndex::moved`]); it is
/// rebuilt once the list's length changed (a cart gone or added), and dropped
/// after a command, which can move or replace any cart
/// ([`CartIndex::forget_carts`]). The mob grid holds until the mob
/// list's next write ([`ListStamp`]). A native mod's hook can move either kind
/// unseen, so with mods loaded both scans stay.
#[derive(Default)]
pub struct CartIndex {
    pub carts: Option<(SeenGrid, Vec<u32>)>,
    pub(in crate::game) mobs: Option<(ListStamp, SeenGrid)>,
    pub(in crate::game) near: Vec<u32>,
}

/// The `DamageSource` a cart took. `VehicleEntity.hurtServer` attributes its
/// `ENTITY_DAMAGE` to `source.getEntity()`, and `chestVehicleDestroyed` angers
/// piglins only when `source.getDirectEntity()` is the player.
#[derive(Clone, Copy, Default)]
pub struct CartHit {
    pub(in crate::game) by: Option<Target>,
    pub(in crate::game) by_player: bool,
}

impl CartHit {
    /// `/damage`'s `DamageSource`: `by` is its direct entity, `causing` the
    /// causing entity `getEntity()` names.
    pub(in crate::game) fn command(by: Option<Target>, causing: Option<Target>) -> Self {
        Self {
            by: causing,
            by_player: matches!(by, Some(Target::Player)),
        }
    }
    /// A hit whose causing entity is `by` and whose direct entity is no player:
    /// a thrown trident, or a blast's indirect source.
    pub(in crate::game) fn attributed(by: Option<Target>) -> Self {
        Self {
            by,
            by_player: false,
        }
    }
    /// The acting player's own hit (`Player.attack`'s `playerAttack`): its
    /// direct entity is the player in place (the host, or the guest whose turn
    /// it is), whose state `anger_piglins` reads.
    pub fn player() -> Self {
        Self {
            by: Some(Target::Player),
            by_player: true,
        }
    }
    /// A projectile's hit: `getEntity()` is its owner, `getDirectEntity()` the
    /// projectile itself, so it angers no piglin.
    pub fn projectile(owner: Option<Who>, guest: Option<u32>) -> Self {
        let by = match owner {
            Some(Who::Player) => Some(guest.map_or(Target::Player, Target::Guest)),
            Some(Who::Mob(id)) => Some(Target::Mob(id)),
            None => None,
        };
        Self {
            by,
            by_player: false,
        }
    }
}

fn horizontal(c: &Minecart) -> f64 {
    sunrise_core::geometry::length(c.velocity.x, c.velocity.z)
}

impl CartIndex {
    /// The slots of the carts whose box may meet `area`, for the cart in slot
    /// `i` (just moved); false: scan them all.
    pub fn carts_near(
        &mut self,
        carts: &[Minecart],
        i: usize,
        area: Aabb,
        out: &mut Vec<u32>,
    ) -> bool {
        if !Self::usable() {
            return false;
        }
        let fresh = |ids: &[u32]| ids.len() == carts.len() && ids[i] == carts[i].id;
        if !self.carts.as_ref().is_some_and(|(_, ids)| fresh(ids)) {
            self.carts = Some((
                SeenGrid::new(carts.iter().map(Minecart::bounds)),
                carts.iter().map(|c| c.id).collect(),
            ));
        }
        let Some((grid, ids)) = &mut self.carts else {
            return false;
        };
        grid.update(i, carts[i].bounds());
        if !grid.candidates(area, out) {
            return false;
        }
        if out.iter().any(|&j| ids[j as usize] != carts[j as usize].id) {
            // A cart was replaced in place: start over next time.
            self.carts = None;
            return false;
        }
        true
    }
    pub fn forget_carts(&mut self) {
        self.carts = None;
    }
    /// The slots of the mobs whose box may meet `area` into `out`;
    /// false means the caller must walk them all.
    pub fn mobs_near(&mut self, mobs: &JoinList<Mob>, area: Aabb, out: &mut Vec<u32>) -> bool {
        if !Self::usable() {
            return false;
        }
        if !self
            .mobs
            .as_ref()
            .is_some_and(|(stamp, _)| *stamp == mobs.stamp())
        {
            self.mobs = Some((mobs.stamp(), SeenGrid::new(mobs.iter().map(Mob::bounds))));
        }
        self.mobs
            .as_ref()
            .is_some_and(|(_, grid)| grid.candidates(area, out))
    }
    /// The cart in slot `i` has these bounds now.
    pub fn moved(&mut self, i: usize, bounds: Aabb) {
        if let Some((grid, ids)) = &mut self.carts
            && i < ids.len()
        {
            grid.update(i, bounds);
        }
    }
    pub(in crate::game::server_side) fn usable() -> bool {
        mod_api::__private::mods().is_empty()
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn server_cart_rail_direction(
        &self,
        cart: Option<u32>,
        state: BlockStateId,
        pos: sunrise_core::position::BlockPos,
        text: &str,
        vanilla: Shape,
    ) -> Shape {
        let (Some(level), Some(epoch), Ok(api_state)) = (
            self.level(self.common.dim.id()),
            self.common.native_lifecycle.epoch(),
            state.try_into_api(),
        ) else {
            return vanilla;
        };
        let read = ReadLevel::new(self, level, &self.common.world);
        let side = mod_api::LevelSide::Server(&read);
        let cart = cart.map(|id| mod_api::EntityRef::new(epoch, id));
        dispatch_cart_rail_direction(side, self.common.dim.level(), api_state, pos, cart, text)
    }

    /// Closing the cart's container open under `key` copies the items back.
    pub fn close_cart_container(&mut self, key: (i32, i32, i32)) {
        if key.1 != CART_KEY_Y {
            return;
        }
        self.common.store_cart_items(key);
        let guest_viewers = self.server.lan.as_ref().is_some_and(|host| {
            sunrise_gameplay_common::game::CommonGame::vehicle_has_other_viewers(
                None,
                &host.hub.guests,
                None,
                key,
            )
        });
        if !guest_viewers {
            self.common.world.block_entities.remove(&key);
        }
    }

    /// `activateMinecart` on an activator rail.
    pub fn activate_cart(&mut self, c: &mut Minecart, powered: bool) {
        match c.kind {
            CartKind::Rideable if powered => {
                // Minecart.activateMinecart: ejectPassengers.
                if self.common.riding == Some(c.id)
                    && self.native_may_leave(c.id)
                    && self.common.riding == Some(c.id)
                {
                    self.common.riding = None;
                }
                for (_, rider, player) in self.seated_passengers(c.id) {
                    if !player
                        && self
                            .common
                            .mob_object(rider)
                            .is_some_and(|mob| !mob.removed && mob.extra.mount == Some(c.id))
                    {
                        self.dismount_mob_from_cart(rider, c);
                    }
                }
                if c.hurt_time == 0 {
                    c.hurt_dir = -c.hurt_dir;
                    c.hurt_time = 10;
                    c.damage = 50.0;
                }
            }
            CartKind::Tnt if powered && c.fuse < 0 => self.prime_cart(c),
            CartKind::Hopper => c.enabled = !powered,
            // MinecartCommandBlock.activateMinecart: the command runs at most
            // every ACTIVATION_DELAY ticks.
            CartKind::CommandBlock if powered && c.tick_count - c.last_activated >= 4 => {
                c.command_due = true;
                c.last_activated = c.tick_count;
            }
            _ => {}
        }
    }
    /// `NewMinecartBehavior.adjustToRails`: snap only on the first tick or
    /// when entering rails; curves project onto the segment between exits.
    pub(in crate::game::server_side) fn adjust_cart_to_rails(
        c: &mut Minecart,
        pos: [i32; 3],
        shape: Shape,
    ) {
        let (e0, e1) = shape.exits();
        let exit =
            |e: [i32; 3]| Vec3::new(f64::from(e[0]), f64::from(e[1]), f64::from(e[2])).scale(0.5);
        let old = c.position;
        let (exit0, exit1) = (exit(e0), exit(e1));
        let center = Vec3::new(
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        );
        let mut out = Vec3::new(exit0.x, 0.0, exit0.z);
        let mut into = Vec3::new(exit1.x, 0.0, exit1.z);
        if c.velocity.length() > f64::from(1.0e-5_f32) && c.velocity.dot(out) < c.velocity.dot(into)
            || Self::is_descending(into, shape)
        {
            std::mem::swap(&mut out, &mut into);
        }
        let mut yaw = 180.0 - java_degrees(out.z, out.x) as f32;
        if exit0.x != exit1.x && exit0.z != exit1.z {
            let along = exit1.subtract(exit0);
            let from = c.position.subtract(center).subtract(exit0);
            let projected = along.scale(along.dot(from) / along.dot(along));
            c.position = center.add_vector(exit0).add_vector(projected);
            yaw = 180.0 - java_degrees(projected.z, projected.x) as f32;
        } else {
            if exit0.z != exit1.z {
                c.position.x = center.x;
            }
            if exit0.x != exit1.x {
                c.position.z = center.z;
            }
            c.position.y = center.y;
        }
        let pitch = if shape.is_slope() {
            c.position.y += center.add_vector(into).distance(c.position) + 0.1;
            if c.flipped { 45.0 } else { -45.0 }
        } else {
            c.position.y += 0.1;
            0.0
        };
        Self::set_cart_rotation(c, yaw + if c.flipped { 180.0 } else { 0.0 }, pitch);
        let distance = old.distance(c.position);
        if distance > 0.0 {
            Self::record_cart_step(c, if c.first_tick { 0.0 } else { distance as f32 });
        }
    }
    /// An arrow hit cart `i` (`MinecartTNT.hurtServer`, then
    /// `VehicleEntity.hurtServer`): a burning arrow sets off a TNT minecart.
    /// `owner` is `AbstractArrow.getOwner`, the hit's `DamageSource.getEntity`;
    pub fn arrow_hits_cart(
        &mut self,
        i: usize,
        damage: f32,
        arrow_velocity: Vec3,
        burning: bool,
        creative: bool,
        owner: Option<Who>,
        guest: Option<u32>,
    ) {
        if i >= self.common.carts.len() {
            return;
        }
        if burning && self.common.carts[i].kind == CartKind::Tnt {
            let mut c = self.common.carts[i].clone();
            if self.explode_cart(&mut c, arrow_velocity.length_squared()) {
                return;
            }
        }
        let hit = CartHit::projectile(owner, guest);
        self.hurt_cart(i, damage, creative, false, hit);
    }
    /// `applyEffectsFromBlocks` for a cart: a bubble column lifts or drags it
    /// and resets the fall distance.
    pub(in crate::game::server_side) fn cart_block_effects(&mut self, c: &mut Minecart) {
        let writes = self.body_step_on(
            (c.id, c.kind.id()),
            (c.position, c.bounds()),
            &c.ground,
            &mut c.velocity,
        );
        let (writes, hurts) = crate::game::entity_server::split_contact_hurts(writes);
        super::entity::apply_contact_writes(
            writes,
            &mut c.velocity,
            Some(&mut c.fall_distance),
            Some(&mut c.stuck),
            None,
        );
        self.queue_vehicle_contact_hurts(super::commands::Target::Cart(c.id), hurts);
        if c.push_by_bubble_column(
            &self.common.world,
            self.common.pack.assets.registry,
            self.common.res.facts(),
            self.common.collision,
        ) {
            c.fall_distance = 0.0;
        }
        // PowderSnowBlock's FREEZE contact is counted here too.
        // WebBlock.entityInside: makeStuckInBlock(0.25, 0.05, 0.25) for every
        // cobweb the box (deflated by 1.0E-5) is in; it resets the fall distance.
        let b = c.bounds().deflate(Vec3::new(1e-5, 1e-5, 1e-5));
        let (lo, hi) = (b.min_position(), b.max_position());
        let facts = self.common.res.facts();
        let mut froze = false;
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    let name = &*facts.get(self.common.world.block(x, y, z)).name;
                    if name == "minecraft:powder_snow" {
                        froze = true;
                    }
                    if name == "minecraft:cobweb" {
                        c.fall_distance = 0.0;
                        c.stuck = Vec3::new(0.25, f64::from(0.05_f32), 0.25);
                    }
                }
            }
        }
        if froze {
            // InsideBlockEffectType.FREEZE: once per applyEffectsFromBlocks.
            c.ticks_frozen = (c.ticks_frozen + 1)
                .min(sunrise_sim::motion::PlayerMotion::TICKS_REQUIRED_TO_FREEZE);
        }
    }
    /// `Entity.checkInsideBlocks` for a recorded move of the cart from `from`
    /// to `to`: the entity-sensing blocks (plates, buttons, tripwires, detector
    /// rails) on the path run `entityInside` now, with the cart where the tick
    /// left it. `slot` is where the cart is stored.
    pub(in crate::game::server_side) fn cart_inside_blocks(
        &mut self,
        c: &mut Minecart,
        slot: usize,
        from: Vec3,
        to: Vec3,
    ) {
        let shrink = f64::from(1.0e-5f32);
        let bounds = c.bounds_at(to).deflate(Vec3::new(shrink, shrink, shrink));
        let mut hits = Vec::new();
        let behaviours = self.common.res.behaviours();
        let world = &self.common.world;
        let _ = sunrise_core::geometry::sweep_intersected_blocks(
            from,
            to,
            bounds,
            1_000_000,
            |p, _| {
                if behaviours.senses_entities(world.block(p.x, p.y, p.z)) {
                    hits.push(p);
                }
                true
            },
        );
        if hits.is_empty() {
            return;
        }
        let Some(i) = cart_at(&self.common.carts, c.id, slot) else {
            return;
        };
        // The stored cart is the one a plate counts: it takes the moved cart
        // for the walk and hands back what the walk left.
        self.common
            .carts
            .update(i, |stored| std::mem::swap(stored, c));
        self.with_level(|b, level, ticks| {
            for p in hits {
                b.entity_inside(level, ticks, p);
            }
        });
        self.common
            .carts
            .update(i, |stored| std::mem::swap(stored, c));
        let _ = self.server_process_level_events();
    }
    /// `lavaHurt` twice a tick in lava: from `LavaFluid.entityInside` in the
    /// behaviour's `applyEffectsFromBlocks`, then from `AbstractMinecart.tick`'s
    /// own `isInLava`. The second breaks the cart, or primes a TNT minecart
    /// (`MinecartTNT.destroy` for a fire source). A cart has no `baseTick`,
    /// so the fire `lavaIgnite` lights never burns it.
    pub(in crate::game::server_side) fn cart_lava_tick(&mut self, id: u32, slot: usize) {
        for _ in 0..2 {
            let Some(i) = cart_at(&self.common.carts, id, slot) else {
                return;
            };
            if !self.in_lava(self.common.carts[i].bounds()) {
                return;
            }
            let p = self.common.carts[i].position;
            let kind = self.common.carts[i].kind.id();
            let vanilla = self.saved_refuses_damage(id, "minecraft:lava", false);
            if !self.refuses_lava_hurt(id, kind, vanilla) {
                self.hurt_cart(i, 4.0, false, true, CartHit::default());
            }
            self.lava_hurt_sound([p.x, p.y, p.z]);
        }
    }
    /// `AbstractMinecart.getMaxSpeed`: the behavior's, which a furnace
    /// minecart cuts (`MinecartFurnace.getMaxSpeed`).
    pub(in crate::game::server_side) fn cart_max_speed(&self, c: &Minecart) -> f64 {
        if self.minecart_improvements() {
            furnace_speed(c, self.max_speed_new(c))
        } else {
            self.max_speed(c)
        }
    }
    /// `comeOffTrack`.
    pub(in crate::game::server_side) fn come_off_track(&mut self, c: &mut Minecart) -> bool {
        let max = self.cart_max_speed(c);
        c.velocity.x = c.velocity.x.clamp(-max, max);
        c.velocity.z = c.velocity.z.clamp(-max, max);
        if c.ground.on_ground {
            c.velocity = Vec3::new(c.velocity.x * 0.5, c.velocity.y * 0.5, c.velocity.z * 0.5);
        }
        let v = c.velocity;
        if !self.move_cart(c, v) {
            return false;
        }
        if !c.ground.on_ground {
            // scale(getAirDrag()): 0.95F, widened.
            let drag = f64::from(0.95_f32);
            c.velocity = Vec3::new(
                c.velocity.x * drag,
                c.velocity.y * drag,
                c.velocity.z * drag,
            );
        }
        true
    }
    /// `AbstractMinecart.isRedstoneConductor`: the block state's own flag
    /// (soul sand conducts without a full collision shape).
    pub(in crate::game::server_side) fn conductor(&self, x: i32, y: i32, z: i32) -> bool {
        self.common
            .res
            .behaviours()
            .facts(self.common.world.block(x, y, z))
            .flags
            & sunrise_world::level::data::state_flags::CONDUCTOR
            != 0
    }
    /// `NewMinecartBehavior.moveAlongTrack`: each step has its own rotation
    /// and interpolation weight, including the final off-rail move.
    pub(in crate::game::server_side) fn finish_cart_step(
        &self,
        c: &mut Minecart,
        old: Vec3,
        initial: Vec3,
    ) {
        let delta = old.vector_to(c.position);
        let length = delta.length();
        if length > f64::from(1.0e-5_f32) {
            if delta.horizontal_distance_squared() > f64::from(1.0e-5_f32) {
                let yaw = 180.0 - java_degrees(delta.z, delta.x) as f32
                    + if c.flipped { 180.0 } else { 0.0 };
                let pitch = if c.ground.on_ground && !c.on_rails {
                    0.0
                } else {
                    (90.0 - java_degrees(delta.horizontal_distance(), delta.y) as f32)
                        * if c.flipped { -1.0 } else { 1.0 }
                };
                Self::set_cart_rotation(c, yaw, pitch);
            } else if !c.on_rails {
                c.pitch = if c.ground.on_ground {
                    0.0
                } else {
                    c.pitch + 0.2 * ((-c.pitch + 180.0).rem_euclid(360.0) - 180.0)
                };
            }
            Self::record_cart_step(c, length.min(self.max_speed_new(c)) as f32);
        } else if initial.horizontal_distance_squared() > 0.0 {
            Self::record_cart_step(c, 1.0);
        }
    }
    /// `interact`: ride, open or fuel the cart under the crosshair, or the one
    /// a guest's interact packet named. A TNT cart has no interact of its own
    /// (`Entity.interact` passes), so nothing here lights it.
    pub fn interact_cart(&mut self, target: Option<InteractTarget>) -> bool {
        let Some((i, d)) = (match target {
            Some(t) => sunrise_gameplay_common::game::item_actions::named_entity(
                &self.common.carts,
                t.id,
                |c| c.id,
            )
            .map(|i| (i, 0.0)),
            None => self.cart_hit(),
        }) else {
            return false;
        };
        // Only a client's own pick is stopped by a block: the range check passed.
        if target.is_none() && !self.before_block(d) {
            return false;
        }
        let sneaking = self.server_acting_sneak();
        let held = self.common.held().cloned();
        match self.common.carts[i].kind {
            CartKind::Rideable => {
                let id = self.common.carts[i].id;
                if sneaking || self.common.riding.is_some() || self.has_seated_passengers(id) {
                    return false;
                }
                self.start_riding_target(
                    super::commands::Target::Player,
                    super::commands::Target::Cart(id),
                    false,
                );
            }
            // AbstractMinecartContainer.interact: no sneak check.
            CartKind::Chest | CartKind::Hopper => self.open_cart_container(i),
            CartKind::Furnace => {
                let fuel = held
                    .as_ref()
                    .is_some_and(|s| matches!(&*s.item, "minecraft:coal" | "minecraft:charcoal"));
                // Java int arithmetic: a mod can set any fuel.
                let fueled = self.common.carts.update(i, |c| {
                    if !fuel {
                        return false;
                    }
                    let Some(next) = furnace_fuel_after(c.fuel) else {
                        return false;
                    };
                    c.fuel = next;
                    if c.fuel > 0 {
                        let p = self
                            .common
                            .player
                            .as_ref()
                            .map_or(c.position, |p| p.position);
                        c.push = Vec3::new(c.position.x - p.x, 0.0, c.position.z - p.z);
                    }
                    true
                });
                if fueled && !self.common.creative {
                    self.consume_held();
                }
            }
            // MinecartTNT has no interact of its own (`Entity.interact` passes).
            CartKind::Tnt => return false,
            // MinecartCommandBlock.interact: a game master gets SUCCESS (the
            // client opens the editor, `open_cart_command_block`), anyone else
            // PASSes to the held item. SUCCESS swings on the client only.
            CartKind::CommandBlock => return self.common.can_use_game_master_blocks(),
            // MinecartSpawner has no interact of its own (`Entity.interact` passes).
            CartKind::Spawner => return false,
        }
        self.swing_arm();
        true
    }
    /// `NewMinecartBehavior.isDecending`: the movement goes down the
    /// shape's rise.
    pub(in crate::game) fn is_descending(movement: Vec3, shape: Shape) -> bool {
        match shape {
            Shape::AscendingEast => movement.x < 0.0,
            Shape::AscendingWest => movement.x > 0.0,
            Shape::AscendingNorth => movement.z > 0.0,
            Shape::AscendingSouth => movement.z < 0.0,
            _ => false,
        }
    }
    pub(in crate::game) fn max_speed(&self, c: &Minecart) -> f64 {
        let base = if c.in_water { 0.2 } else { 0.4 };
        furnace_speed(c, base)
    }
    /// `NewMinecartBehavior.getMaxSpeed`: the game rule over water's half,
    /// every cart alike (`cart_max_speed` cuts a furnace minecart's).
    pub(in crate::game::server_side) fn max_speed_new(&self, c: &Minecart) -> f64 {
        f64::from(
            self.common
                .ui
                .rules
                .int_at(sunrise_game_data::gamerules::vanilla::MAX_MINECART_SPEED),
        ) * if c.in_water { 0.5 } else { 1.0 }
            / 20.0
    }
    /// `OldMinecartBehavior.moveAlongTrack`.
    pub(in crate::game::server_side) fn move_along_track(
        &mut self,
        c: &mut Minecart,
        pos: [i32; 3],
        rail: &Rail,
        intent: [f64; 2],
        slot: usize,
    ) -> bool {
        c.fall_distance = 0.0;
        let ridden = self.has_seated_passengers(c.id);
        let (x, z) = (c.position.x, c.position.z);
        let old = self.rail_pos(Some(c.id), x, c.position.y, z);
        let mut y = f64::from(pos[1]);
        let (mut power, mut halt) = (false, false);
        if &*rail.name == "minecraft:powered_rail" {
            power = rail.powered;
            halt = !power;
        }
        let slide = if c.in_water {
            0.0078125 * 0.2
        } else {
            0.0078125
        };
        match rail.shape {
            Shape::AscendingEast => c.velocity.x -= slide,
            Shape::AscendingWest => c.velocity.x += slide,
            Shape::AscendingNorth => c.velocity.z += slide,
            Shape::AscendingSouth => c.velocity.z -= slide,
            _ => {}
        }
        if rail.shape.is_slope() {
            y += 1.0;
        }
        let (e0, e1) = rail.shape.exits();
        let (mut xd, mut zd) = (f64::from(e1[0] - e0[0]), f64::from(e1[2] - e0[2]));
        let length = sunrise_core::geometry::length(xd, zd);
        if c.velocity.x * xd + c.velocity.z * zd < 0.0 {
            xd = -xd;
            zd = -zd;
        }
        let pow = horizontal(c).min(2.0);
        c.velocity = Vec3::new(pow * xd / length, c.velocity.y, pow * zd / length);
        if ridden
            && (intent[0] != 0.0 || intent[1] != 0.0)
            && c.velocity.x.powi(2) + c.velocity.z.powi(2) < 0.01
        {
            c.velocity.x += intent[0] * 0.001;
            c.velocity.z += intent[1] * 0.001;
            halt = false;
        }
        if halt {
            if horizontal(c) < 0.03 {
                c.velocity = Vec3::ZERO;
            } else {
                c.velocity = Vec3::new(c.velocity.x * 0.5, 0.0, c.velocity.z * 0.5);
            }
        }
        let (fx, fz) = (f64::from(pos[0]), f64::from(pos[2]));
        let x0 = fx + 0.5 + f64::from(e0[0]) * 0.5;
        let z0 = fz + 0.5 + f64::from(e0[2]) * 0.5;
        let x1 = fx + 0.5 + f64::from(e1[0]) * 0.5;
        let z1 = fz + 0.5 + f64::from(e1[2]) * 0.5;
        let (xd, zd) = (x1 - x0, z1 - z0);
        let progress = if xd == 0.0 {
            z - fz
        } else if zd == 0.0 {
            x - fx
        } else {
            ((x - x0) * xd + (z - z0) * zd) * 2.0
        };
        c.position = Vec3::new(x0 + xd * progress, y, z0 + zd * progress);
        let scale = if ridden { 0.75 } else { 1.0 };
        let max = self.max_speed(c);
        let delta = Vec3::new(
            (scale * c.velocity.x).clamp(-max, max),
            0.0,
            (scale * c.velocity.z).clamp(-max, max),
        );
        if !self.move_cart(c, delta) {
            return false;
        }
        // 🤖 Agent: `AbstractMinecart.move` runs applyEffectsFromBlocks at
        // once, the cart still at the rail's top height (below it is lowered).
        let here = c.position;
        self.cart_inside_blocks(c, slot, here, here);
        let at = |e: [i32; 3], c: &Minecart| {
            c.position.x.floor() as i32 - pos[0] == e[0]
                && c.position.z.floor() as i32 - pos[2] == e[2]
        };
        if e0[1] != 0 && at(e0, c) {
            c.position.y += f64::from(e0[1]);
        } else if e1[1] != 0 && at(e1, c) {
            c.position.y += f64::from(e1[1]);
        }
        c.velocity = self.natural_slowdown(c, ridden, 0.96);
        if let (Some(new), Some(old)) = (
            self.rail_pos(Some(c.id), c.position.x, c.position.y, c.position.z),
            old,
        ) {
            let speed = (old[1] - new[1]) * 0.05;
            let h = horizontal(c);
            if h > 0.0 {
                c.velocity.x *= (h + speed) / h;
                c.velocity.z *= (h + speed) / h;
            }
            c.position.y = new[1];
        }
        let (xn, zn) = (c.position.x.floor() as i32, c.position.z.floor() as i32);
        if xn != pos[0] || zn != pos[2] {
            let h = horizontal(c);
            c.velocity = Vec3::new(
                h * f64::from(xn - pos[0]),
                c.velocity.y,
                h * f64::from(zn - pos[2]),
            );
        }
        if power {
            let h = horizontal(c);
            if h > 0.01 {
                c.velocity.x += c.velocity.x / h * 0.06;
                c.velocity.z += c.velocity.z / h * 0.06;
            } else if rail.shape == Shape::EastWest {
                if self.conductor(pos[0] - 1, pos[1], pos[2]) {
                    c.velocity.x = 0.02;
                } else if self.conductor(pos[0] + 1, pos[1], pos[2]) {
                    c.velocity.x = -0.02;
                }
            } else if rail.shape == Shape::NorthSouth {
                if self.conductor(pos[0], pos[1], pos[2] - 1) {
                    c.velocity.z = 0.02;
                } else if self.conductor(pos[0], pos[1], pos[2] + 1) {
                    c.velocity.z = -0.02;
                }
            }
        }
        true
    }
    /// `NewMinecartBehavior.moveAlongTrack`: the iteration keeps moving
    /// while distance is left, over as many rails as it takes, the speed
    /// recomputed per rail (slope, halt, boost).
    pub(in crate::game::server_side) fn move_along_track_new(
        &mut self,
        c: &mut Minecart,
        intent: [f64; 2],
    ) -> bool {
        // `TrackIteration`: the slope, halt and boost speeds each apply once
        // per tick, and the iteration keeps moving while distance is left.
        let mut first = true;
        let mut movement_left = 0.0;
        let mut slope_done = false;
        let mut halted = false;
        let mut boosted = false;
        let mut step_start = c.previous;
        while first || movement_left > f64::from(1.0e-5_f32) {
            let initial = c.velocity;
            let pos = self.rail_block(c.position, true);
            let Some(rail) = self.rail(Some(c.id), pos[0], pos[1], pos[2]) else {
                c.on_rails = false;
                if !self.come_off_track(c) {
                    return false;
                }
                self.finish_cart_step(c, step_start, initial);
                if first || c.position.distance(step_start) > f64::from(1.0e-5_f32) {
                    self.cart_block_effects(c);
                    self.cart_block_effects(c);
                }
                return true;
            };
            if !c.on_rails {
                c.on_rails = true;
                Self::adjust_cart_to_rails(c, pos, rail.shape);
            }
            c.fall_distance = 0.0;
            step_start = c.position;
            self.note_cart_pass(c, pos);
            if &*rail.name == "minecraft:activator_rail" {
                self.activate_cart(c, rail.powered);
            }
            let ridden = self.has_seated_passengers(c.id);
            let initial_h = c.velocity.horizontal_distance();
            let mut v = Vec3::new(c.velocity.x, 0.0, c.velocity.z);
            // calculateSlopeSpeed (once per tick): the slide grows with the
            // speed, so a descending cart keeps its pace.
            if !slope_done {
                let slide =
                    0.0078125_f64.max(initial_h * 0.02) * if c.in_water { 0.2 } else { 1.0 };
                let before = v.horizontal_distance_squared();
                match rail.shape {
                    Shape::AscendingEast => v.x -= slide,
                    Shape::AscendingWest => v.x += slide,
                    Shape::AscendingNorth => v.z += slide,
                    Shape::AscendingSouth => v.z -= slide,
                    _ => {}
                }
                slope_done = v.horizontal_distance_squared() != before;
                if !slope_done {
                    v = Vec3::new(c.velocity.x, 0.0, c.velocity.z);
                }
            }
            // calculatePlayerInputSpeed (first iteration).
            if first
                && ridden
                && (intent[0] != 0.0 || intent[1] != 0.0)
                && v.x * v.x + v.z * v.z < 0.01
            {
                let input = Vec3::new(intent[0], 0.0, intent[1])
                    .normalize()
                    .scale(0.001);
                let pushed = v.add_vector(input);
                if pushed.horizontal_distance_squared() != v.horizontal_distance_squared() {
                    v = pushed;
                    halted = true;
                }
            }
            // calculateHaltTrackSpeed (until the input halted).
            if !halted && &*rail.name == "minecraft:powered_rail" && !rail.powered {
                let before = sunrise_core::geometry::length(v.x, v.z);
                if before < 0.03 {
                    v = Vec3::ZERO;
                } else {
                    v = Vec3::new(v.x * 0.5, v.y, v.z * 0.5);
                }
                halted = sunrise_core::geometry::length(v.x, v.z) != before;
            }
            // applyNaturalSlowdown and the speed cap (first iteration).
            if first {
                c.velocity = v;
                v = self.natural_slowdown(c, ridden, 0.975);
                if v.length_squared() > 0.0 {
                    let speed = v.length().min(self.cart_max_speed(c));
                    v = v.normalize().scale(speed);
                }
            }
            // calculateBoostTrackSpeed (once per tick).
            if !boosted && &*rail.name == "minecraft:powered_rail" && rail.powered {
                let before = v;
                let len = v.length();
                if len > 0.01 {
                    v = v.normalize().scale(len + 0.06);
                } else {
                    let dir = self.redstone_direction(pos, rail.shape);
                    if dir != Vec3::ZERO {
                        v = dir.scale(len + 0.2);
                    }
                }
                boosted = v.horizontal_distance_squared() != before.horizontal_distance_squared();
                if !boosted {
                    v = before;
                }
            }
            c.velocity = v;
            let new_h = v.horizontal_distance();
            movement_left = if first {
                new_h
            } else {
                movement_left + (new_h - initial_h)
            };
            let Some(left) = self.new_step_along_track(c, pos, &rail, movement_left) else {
                return false;
            };
            movement_left = left;
            self.finish_cart_step(c, step_start, initial);
            // NewMinecartBehavior.moveAlongTrack applies block effects twice,
            // after recording the step, including a stationary first step.
            if first || c.position.distance(step_start) > f64::from(1.0e-5_f32) {
                self.cart_block_effects(c);
                self.cart_block_effects(c);
            }
            first = false;
        }
        true
    }
    /// `applyNaturalSlowdown` (with `MinecartFurnace`'s push); `factor` is
    /// `getSlowdownFactor`: 0.997 riding, the empty cart's own by behavior.
    pub fn natural_slowdown(&self, c: &mut Minecart, ridden: bool, factor: f64) -> Vec3 {
        let mut v = c.velocity;
        let water = c.in_water;
        if c.kind == CartKind::Furnace {
            if c.push.length_squared() > 1.0e-7 {
                c.push = push_along(c.push, v);
                v = Vec3::new(v.x * 0.8 + c.push.x, 0.0, v.z * 0.8 + c.push.z);
                if water {
                    v = Vec3::new(v.x * 0.1, 0.0, v.z * 0.1);
                }
            } else {
                v = Vec3::new(v.x * 0.98, 0.0, v.z * 0.98);
            }
        }
        if matches!(c.kind, CartKind::Chest | CartKind::Hopper) {
            // `AbstractMinecartContainer.applyNaturalSlowdown` replaces the
            // base one whole: the keep factor leans on how empty it is, and
            // riders and the behavior's own factor do not enter.
            let signal = Minecart::container_signal(&c.items, self.items());
            let keep = container_keep(signal, c.loot.is_some(), water);
            return Vec3::new(v.x * keep, 0.0, v.z * keep);
        }
        let f = if ridden { 0.997 } else { factor };
        let mut out = Vec3::new(v.x * f, 0.0, v.z * f);
        if water {
            // scale(0.95F): the float widened.
            let drag = f64::from(0.95_f32);
            out = Vec3::new(out.x * drag, 0.0, out.z * drag);
        }
        out
    }
    /// `NewMinecartBehavior.stepAlongTrack`: toward the rail's exit, the
    /// hill's Y projected, a V of opposing slopes resting a slow cart.
    pub(in crate::game::server_side) fn new_step_along_track(
        &mut self,
        c: &mut Minecart,
        pos: [i32; 3],
        rail: &Rail,
        mut movement_left: f64,
    ) -> Option<f64> {
        if movement_left < f64::from(1.0e-5_f32) {
            return Some(0.0);
        }
        let old = c.position;
        let (e0, e1) = rail.shape.exits();
        let movement = Vec3::new(c.velocity.x, 0.0, c.velocity.z);
        if movement.length() < f64::from(1.0e-5_f32) {
            c.velocity = Vec3::ZERO;
            return Some(0.0);
        }
        let in_hill = e0[1] != e1[1];
        let mut out_dir = Vec3::new(f64::from(e0[0]) * 0.5, 0.0, f64::from(e0[2]) * 0.5);
        let in_dir = Vec3::new(f64::from(e1[0]) * 0.5, 0.0, f64::from(e1[2]) * 0.5);
        if movement.dot(out_dir) < movement.dot(in_dir) {
            out_dir = in_dir;
        }
        let out_len = sunrise_core::geometry::length(out_dir.x, out_dir.z);
        let mut out_pos = Vec3::new(
            f64::from(pos[0]) + 0.5 + out_dir.x + out_dir.x / out_len * f64::from(1.0e-5_f32),
            f64::from(pos[1]) + 0.1,
            f64::from(pos[2]) + 0.5 + out_dir.z + out_dir.z / out_len * f64::from(1.0e-5_f32),
        );
        if in_hill && !Self::is_descending(c.velocity, rail.shape) {
            out_pos.y += 1.0;
        }
        let towards_out = c.position.vector_to(out_pos).normalize();
        // The movement turns onto the exit's direction, at the same speed
        // (`this.setDeltaMovement(movement)` at stepAlongTrack's end).
        let h_speed = sunrise_core::geometry::length(movement.x, movement.z);
        let th = sunrise_core::geometry::length(towards_out.x, towards_out.z);
        let movement = towards_out.scale(h_speed / th);
        let mut new_pos = if in_hill {
            old.add_vector(
                movement
                    .normalize()
                    .scale(movement_left * f64::from(std::f32::consts::SQRT_2)),
            )
        } else {
            old.add_vector(movement.normalize().scale(movement_left))
        };
        if out_pos.distance(old) <= new_pos.distance(old) {
            movement_left = out_pos.vector_to(new_pos).horizontal_distance();
            new_pos = out_pos;
        } else {
            movement_left = 0.0;
        }
        if !self.move_cart(c, old.vector_to(new_pos)) {
            return None;
        }
        if in_hill {
            let new_block = [
                new_pos.x.floor() as i32,
                new_pos.y.floor() as i32,
                new_pos.z.floor() as i32,
            ];
            if let Some(new_rail) = self.rail(Some(c.id), new_block[0], new_block[1], new_block[2])
            {
                // restAtVShape: opposing slopes rest a slow cart.
                if c.velocity.length_squared() < 0.005
                    && new_rail.shape.is_slope()
                    && Self::is_descending(c.velocity, rail.shape)
                    && !Self::is_descending(c.velocity, new_rail.shape)
                {
                    c.velocity = Vec3::ZERO;
                    return Some(0.0);
                }
            }
            let h_dist =
                sunrise_core::geometry::length(out_pos.x - c.position.x, out_pos.z - c.position.z);
            let project = out_pos.y
                + if Self::is_descending(movement, rail.shape) {
                    h_dist
                } else {
                    -h_dist
                };
            if c.position.y < project {
                c.position.y = project;
            }
        }
        // The cart did not move but the step would have: it is stuck.
        if c.position.distance(old) < f64::from(1.0e-5_f32)
            && new_pos.distance(old) > f64::from(1.0e-5_f32)
        {
            c.velocity = Vec3::ZERO;
            return Some(0.0);
        }
        c.velocity = movement;
        Some(movement_left)
    }
    /// The cart passes over the rail at `pos`: `minecart_pass`, delivered after its batch.
    pub(in crate::game::server_side) fn note_cart_pass(&self, c: &Minecart, pos: [i32; 3]) {
        sunrise_sim::block_contact::minecart_pass(
            c.id as i32,
            self.common.world.block(pos[0], pos[1], pos[2]),
            BlockPos::new(pos[0], pos[1], pos[2]),
        );
    }
    /// `AbstractMinecartContainer.interact`: the menu opens over the cart's
    /// own slots, kept under a placeholder key until it closes
    /// (`store_cart_items`/`close_cart_container`).
    pub fn open_cart_container(&mut self, i: usize) {
        self.unpack_cart_loot(i);
        let c = self.common.carts[i].clone();
        let key = (i32::MAX - 15, CART_KEY_Y, c.id as i32);
        // `startOpen` shares one container, it does not reset it: a stand-in
        // another player's menu is still editing over stays as it is.
        if !self.common.world.block_entities.contains_key(&key) {
            self.common
                .world
                .block_entities
                .insert(key, BlockEntity::Chest(c.items.clone()));
        }
        self.common.container = Some(key);
        self.server_set_acting_use(false);
        if c.kind == CartKind::Chest {
            // `MinecartChest.interact` (which overrides
            // `AbstractMinecartContainer.interact`): a successful open fires
            // CONTAINER_OPEN with the player and angers the piglins that see
            // it. `MinecartHopper` does not override it, so a hopper minecart
            // fires neither. Vanilla plays no open sound for a chest minecart.
            let at = [c.position.x, c.position.y, c.position.z];
            let ctx = self.player_context(None);
            self.post_game_event("minecraft:container_open", at, ctx);
            self.anger_piglins(true);
        }
    }
    /// `OldMinecartBehavior.pushAndPickupEntities` for living entities in cart
    /// `i`'s box (0.2 wider on each side): an eligible mob boards a moving
    /// rideable cart (`startRiding`); others get `Entity.push`.
    pub fn pickup_from_cart(&mut self, i: usize, index: &mut CartIndex) {
        let v = self.common.carts[i].velocity;
        if self.common.carts[i].kind != CartKind::Rideable || v.x * v.x + v.z * v.z < 0.01 {
            return;
        }
        let reach = f64::from(0.2f32);
        let id = self.common.carts[i].id;
        let pos = self.common.carts[i].position;
        let hitbox = self.common.carts[i]
            .bounds()
            .inflate(Vec3::new(reach, 0.0, reach));
        let mut near = std::mem::take(&mut index.near);
        let mut occupied = self.has_seated_passengers(id);
        if !index.mobs_near(&self.common.mobs, hitbox, &mut near) {
            near.clear();
            near.extend(0..self.common.mobs.len() as u32);
        }
        // The box first: it turns away nearly every mob of the level.
        near.retain(|&j| {
            let m = &self.common.mobs[j as usize];
            !m.removed && m.extra.mount != Some(id) && hitbox.intersects(m.bounds())
        });
        let candidates: Vec<u32> = if near.is_empty() {
            Vec::new()
        } else {
            let (teams, own) = (self.mob_teams(), self.team_with_owner(id, || None));
            near.iter()
                .map(|&j| &self.common.mobs[j as usize])
                .filter(|m| {
                    offered_pickup(m.pushable(), m.species(), m.alive())
                        && pushable_by(own, teams.of(Who::Mob(m.id)), true)
                })
                .map(|m| m.id)
                .collect()
        };
        index.near = near;
        for mob in candidates {
            // By id: a mod's mount hook may have removed mobs since the list
            // was made, which moves the others' indices.
            let Some(j) = self.common.mobs.iter().position(|m| m.id == mob) else {
                continue;
            };
            let species = self.common.mobs[j].species();
            let player = &*self.common.mobs[j].kind.id == "minecraft:player";
            let cooldown = self.common.mobs[j].extra.boarding_cooldown;
            let passenger = self.common.mobs[j].extra.mount.is_some();
            if pickup_instead_of_push(species, passenger, occupied) && !player {
                // Entity.startRiding: a mod may cancel it (EntityMountEvent).
                if can_ride_cart(species, cooldown)
                    && self.native_mob_may_ride(mob, id, true)
                    && let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob)
                {
                    sunrise_sim::mobs::shulker::on_start_riding(m);
                    m.extra.mount = Some(id);
                    m.extra.passenger = true;
                    m.ai.nav.stop();
                    m.needs_sync = true;
                    occupied = true;
                    self.seat_mob(mob);
                    self.break_leash(mob, true, false);
                    self.note_started_riding(id);
                    self.vehicle_picked_up();
                }
                continue;
            }
            if !takes_push_from_cart(species) {
                continue;
            }
            let other = self.common.mobs[j].position;
            let Some(v) = sunrise_gameplay_common::game::boats_common::push_impulse(pos, other)
            else {
                continue;
            };
            if !occupied {
                self.common
                    .carts
                    .update(i, |c| c.velocity = c.velocity.subtract(v));
            }
            let m = &mut self.common.mobs[j];
            if m.pushable() {
                m.velocity = m.velocity.add_vector(v);
                m.needs_sync = true;
            }
        }
    }
    /// `MinecartItem.useOn`: a cart on the clicked rail.
    pub(in crate::game::server_side) fn place_minecart(
        &mut self,
        hit: &Hit,
        stack: &ItemStack,
    ) -> bool {
        let Some(kind) = CartKind::from_id(&stack.item) else {
            return false;
        };
        let Some(rail) = self.rail(None, hit.pos[0], hit.pos[1], hit.pos[2]) else {
            return false;
        };
        let offset = if rail.shape.is_slope() { 0.5 } else { 0.0 };
        let at = Vec3::new(
            f64::from(hit.pos[0]) + 0.5,
            f64::from(hit.pos[1]) + 0.0625 + offset,
            f64::from(hit.pos[2]) + 0.5,
        );
        self.server.next_entity += 1;
        let cart = Minecart::new(self.server.next_entity, kind, at);
        self.common
            .world
            .modified
            .insert((hit.pos[0] >> 4, hit.pos[2] >> 4));
        self.common.carts.push(cart);
        let id = self.server.next_entity;
        self.apply_item_stack_config(super::commands::Target::Cart(id), kind.id(), stack, true);
        if self.server.serving.is_some() || self.common.options.dedicated.is_some() {
            let cart = self.common.carts.last().unwrap();
            self.native_vehicle_create(
                cart.kind.id(),
                [cart.position.x, cart.position.y, cart.position.z],
            );
        }
        // MinecartItem.useOn: ENTITY_PLACE at the rail, by the player, with the block under it.
        let below = self
            .common
            .world
            .block(hit.pos[0], hit.pos[1] - 1, hit.pos[2]);
        self.post_block_game_event(
            "minecraft:entity_place",
            hit.pos,
            self.sculk_actor(),
            Some(below),
        );
        if !self.common.creative {
            self.consume_held();
        }
        // ItemStack.useOn counts the use.
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        true
    }
    /// `AbstractMinecart.push` from each player walking into a cart
    /// (`LivingEntity.pushEntities`): the window's player and every server
    /// player (single player's own on its server thread, LAN guests). A
    /// player in a vehicle pushes nothing here. The quarter push the player
    /// gets back only moves the server's copy of it (`Entity.push` marks
    /// `needsSync`, which `ServerEntity` sends to the other players only),
    /// so no player feels it.
    pub(in crate::game::server_side) fn push_carts(&mut self) {
        if self.common.carts.is_empty() {
            return;
        }
        // (feet, box)
        let mut pushers: Vec<(Vec3, Aabb)> = Vec::new();
        if self.common.riding.is_none()
            && let Some(p) = self.common.player.as_ref()
        {
            pushers.push((p.position, p.bounds()));
        }
        if let Some(host) = self.server.lan.as_ref() {
            for g in host.hub.guests.iter().filter(|g| {
                host.hub.here(g) && g.loaded && !g.dead && !g.spectator() && g.vehicle.is_none()
            }) {
                let [x, y, z] = g.position;
                pushers.push((Vec3::new(x, y, z), g.bounds()));
            }
        }
        let cells = sunrise_sim::mobs::seen_grid::SeenGrid::new(
            self.common.carts.iter().map(Minecart::bounds),
        );
        let mut near = Vec::new();
        for (feet, bb) in pushers {
            if !cells.candidates(bb, &mut near) {
                near.extend(0..self.common.carts.len() as u32);
            }
            for &k in &near {
                let k = k as usize;
                self.common.carts.update(k, |c| {
                    if !bb.intersects(c.bounds()) {
                        return;
                    }
                    let (mut xa, mut za) = (feet.x - c.position.x, feet.z - c.position.z);
                    let dd = xa * xa + za * za;
                    if dd < f64::from(1.0e-4f32) {
                        return;
                    }
                    let d = dd.sqrt();
                    let pow = (1.0 / d).min(1.0);
                    xa = xa / d * pow * f64::from(0.1f32) * 0.5;
                    za = za / d * pow * f64::from(0.1f32) * 0.5;
                    c.velocity.x -= xa;
                    c.velocity.z -= za;
                });
            }
        }
        self.push_carts_from_mobs(&cells);
    }
    /// `AbstractMinecart.push(mob)` from each mob walking into a cart (`LivingEntity.pushEntities`
    /// -> `doPush`): the cart gets the push and the mob a quarter of it back (`Entity.push`, which
    /// needs the mob to be pushable and to carry no one). Bats push nothing (`Bat.pushEntities`),
    /// nor does a mob whose box reaches no cart: its box is worked out only if it may.
    pub(in crate::game::server_side) fn push_carts_from_mobs(&mut self, cells: &SeenGrid) {
        let (teams, mut near) = (self.mob_teams(), Vec::new());
        // Every mob, each asked which carts it is near: a mob far from all of them is turned
        // away by a bitmap of the carts' coarse cells, without a lookup or a grid of the mobs.
        let reach = self.vehicle_mob_reach();
        let coarse = reach.map(|_| CartCells::of(self.common.carts.iter().map(Minecart::bounds)));
        // (mob index, feet, box)
        let pushers: Vec<(usize, Vec3, Aabb)> = (0..self.common.mobs.len() as u32)
            .into_iter()
            .map(|j| (j as usize, &self.common.mobs[j as usize]))
            .filter(|(_, m)| {
                // The test that turns most mobs away comes first: it reads only the position.
                reach.is_none_or(|r| {
                    coarse
                        .as_ref()
                        .is_none_or(|c| c.may_meet(r.around(m.position)))
                }) && !m.removed
                    && m.alive()
                    && m.species() != Species::Bat
                    && reach.is_none_or(|r| self.mob_may_meet_cart(r, m, cells, &mut near))
                    && pushable_by(teams.of(Who::Mob(m.id)), None, true)
            })
            .map(|(j, m)| (j, m.position, m.bounds()))
            .collect();
        drop(teams);
        for (j, feet, bb) in pushers {
            if !cells.candidates(bb, &mut near) {
                near.extend(0..self.common.carts.len() as u32);
            }
            for &k in &near {
                let k = k as usize;
                self.common.carts.update(k, |c| {
                    // hasPassenger(mob): a mob in this cart never pushes it.
                    if self.common.mobs[j].extra.mount == Some(c.id) || !bb.intersects(c.bounds()) {
                        return;
                    }
                    let Some((xa, za)) = cart_push_from(feet, c.position) else {
                        return;
                    };
                    c.velocity.x -= xa;
                    c.velocity.z -= za;
                    let m = &mut self.common.mobs[j];
                    if m.pushable() && m.rider.is_none() && m.extra.rider.is_none() {
                        m.velocity.x += xa / 4.0;
                        m.velocity.z += za / 4.0;
                        m.needs_sync = true;
                    }
                });
            }
        }
    }
    /// `OldMinecartBehavior.pushAndPickupEntities` for carts, at the end of
    /// cart `i`'s own tick: each other cart in its box (0.2 wider on each
    /// side) gets `push(cart i)`. Running it per cart, after that cart moved
    /// and before the next one does, is what splits stacked carts apart.
    pub fn push_from_cart(&mut self, i: usize, index: &mut CartIndex) {
        let reach = f64::from(0.2f32);
        let hitbox = self.common.carts[i]
            .bounds()
            .inflate(Vec3::new(reach, 0.0, reach));
        let mut near = std::mem::take(&mut index.near);
        if !index.carts_near(&self.common.carts, i, hitbox, &mut near) {
            near.clear();
            near.extend(0..self.common.carts.len() as u32);
        }
        let allowed = self.cart_push_allowed(&self.common.carts, i, &near);
        for (k, &j) in near.iter().enumerate() {
            let j = j as usize;
            if !allowed[k] || j == i || !hitbox.intersects(self.common.carts[j].bounds()) {
                continue;
            }
            // AbstractMinecart.push(entity): `this` is cart j, `entity` cart i.
            let (this, other) = (&self.common.carts[j], &self.common.carts[i]);
            let (mut xa, mut za) = (
                other.position.x - this.position.x,
                other.position.z - this.position.z,
            );
            let dd = xa * xa + za * za;
            if dd < f64::from(1.0e-4f32) {
                continue;
            }
            let d = dd.sqrt();
            xa /= d;
            za /= d;
            let pow = (1.0 / d).min(1.0);
            xa = xa * pow * f64::from(0.1f32) * 0.5;
            za = za * pow * f64::from(0.1f32) * 0.5;
            // pushOtherMinecart (old movement): only along this cart's facing.
            let dir = glam::DVec2::new(
                other.position.x - this.position.x,
                other.position.z - this.position.z,
            )
            .normalize_or_zero();
            let r = f64::from(this.yaw * 0.017453292f32);
            let facing = glam::DVec2::new(f64::from(trig::cos(r)), f64::from(trig::sin(r)))
                .normalize_or_zero();
            if dir.dot(facing).abs() < f64::from(0.8f32) {
                continue;
            }
            let (mt, mo) = (this.velocity, other.velocity);
            let (ft, fo) = (
                this.kind == CartKind::Furnace,
                other.kind == CartKind::Furnace,
            );
            let (nt, no) = if fo && !ft {
                (
                    Vec3::new(mt.x * 0.2 + (mo.x - xa), mt.y, mt.z * 0.2 + (mo.z - za)),
                    Vec3::new(mo.x * 0.95, mo.y, mo.z * 0.95),
                )
            } else if !fo && ft {
                (
                    Vec3::new(mt.x * 0.95, mt.y, mt.z * 0.95),
                    Vec3::new(mo.x * 0.2 + (mt.x + xa), mo.y, mo.z * 0.2 + (mt.z + za)),
                )
            } else {
                let (xdd, zdd) = ((mo.x + mt.x) / 2.0, (mo.z + mt.z) / 2.0);
                (
                    Vec3::new(mt.x * 0.2 + (xdd - xa), mt.y, mt.z * 0.2 + (zdd - za)),
                    Vec3::new(mo.x * 0.2 + (xdd + xa), mo.y, mo.z * 0.2 + (zdd + za)),
                )
            };
            self.common.carts.update(j, |c| c.velocity = nt);
            self.common.carts.update(i, |c| c.velocity = no);
        }
        index.near = near;
        self.pickup_from_cart(i, index);
    }
    /// Runs `push_from_cart` for the cart being ticked (`c` is a copy of the
    /// stored one, kept at `slot`): the stored cart is replaced by `c` for the
    /// push and `c` takes back what the push changed. The rolled loot stays
    /// with `c`: the stored cart is without it for the whole tick
    /// (`tick_carts_now`), and a mineshaft's chest cart carries its whole roll
    /// as NBT.
    pub(in crate::game::server_side) fn push_from_ticked_cart(
        &mut self,
        c: &mut Minecart,
        slot: usize,
        index: &mut CartIndex,
    ) {
        let Some(i) = cart_at(&self.common.carts, c.id, slot) else {
            return;
        };
        let loot = c.loot.take();
        if !self.common.store_cart_back(i, c.clone()) {
            return;
        }
        self.push_from_cart(i, index);
        if let Some(i) = self.common.carts.iter().position(|x| x.id == c.id) {
            *c = self.common.carts[i].clone();
        }
        c.loot = loot;
    }
    /// `NewMinecartBehavior.MinecartStep`: wire rotations use ROTATION_BYTE.
    pub(in crate::game::server_side) fn record_cart_step(c: &mut Minecart, weight: f32) {
        use sunrise_protocol::buf::pack_degrees;
        c.lerp_steps
            .push(sunrise_protocol::packets::play::MinecartStep {
                position: c.position.components(),
                movement: c.velocity.components(),
                yaw: pack_degrees(c.yaw),
                pitch: pack_degrees(c.pitch),
                weight,
            });
    }
    /// `AbstractMinecart.getRedstoneDirection` (over the old port's
    /// conductor proxy): which way a powered rail pushes a resting cart.
    pub fn redstone_direction(&self, pos: [i32; 3], shape: Shape) -> Vec3 {
        let (dx, dz) = match shape {
            Shape::NorthSouth => (0, 1),
            Shape::EastWest => (1, 0),
            _ => return Vec3::ZERO,
        };
        // The push points away from the signal's side (getRedstoneDirection).
        if self.conductor(pos[0] - dx, pos[1], pos[2] - dz) {
            Vec3::new(f64::from(dx), 0.0, f64::from(dz))
        } else if self.conductor(pos[0] + dx, pos[1], pos[2] + dz) {
            Vec3::new(f64::from(-dx), 0.0, f64::from(-dz))
        } else {
            Vec3::ZERO
        }
    }
    /// `NewMinecartBehavior.setRotation`: preserve the cart's facing when
    /// motion reverses, including the pitch sign on a slope.
    pub(in crate::game::server_side) fn set_cart_rotation(
        c: &mut Minecart,
        mut yaw: f32,
        mut pitch: f32,
    ) {
        if (175.0..=185.0).contains(&f64::from((yaw - c.yaw).abs())) {
            c.flipped = !c.flipped;
            yaw -= 180.0;
            pitch = -pitch;
        }
        c.yaw = yaw % 360.0;
        c.pitch = pitch.clamp(-45.0, 45.0) % 360.0;
    }
    /// `AbstractMinecart.tick` for one cart; returns false once it is gone.
    /// `slot` is where the cart is stored (where to look first).
    pub fn tick_cart(
        &mut self,
        c: &mut Minecart,
        intent: [f64; 2],
        slot: usize,
        index: &mut CartIndex,
    ) -> bool {
        c.tick_count += 1;
        c.previous = c.position;
        c.previous_yaw = c.yaw;
        c.previous_pitch = c.pitch;
        if c.hurt_time > 0 {
            c.hurt_time -= 1;
        }
        if c.damage > 0.0 {
            c.damage -= 1.0;
        }
        let improved = self.minecart_improvements();
        if improved && c.first_tick {
            let pos = self.rail_block(c.position, true);
            if let Some(rail) = self.rail(Some(c.id), pos[0], pos[1], pos[2]) {
                c.on_rails = true;
                Self::adjust_cart_to_rails(c, pos, rail.shape);
            } else {
                c.on_rails = false;
            }
        }
        c.velocity.y -= c.get_gravity();
        if improved {
            if !self.move_along_track_new(c, intent) {
                return false;
            }
        } else {
            let pos = self.rail_block(c.position, false);
            let rail = self.rail(Some(c.id), pos[0], pos[1], pos[2]);
            c.on_rails = rail.is_some();
            if let Some(rail) = rail {
                if !self.move_along_track(c, pos, &rail, intent, slot) {
                    return false;
                }
                self.note_cart_pass(c, pos);
                if &*rail.name == "minecraft:activator_rail" {
                    self.activate_cart(c, rail.powered);
                }
            } else if !self.come_off_track(c) {
                return false;
            }
        }
        c.first_tick = false;
        if !self.minecart_improvements() {
            c.pitch = 0.0;
            let (xd, zd) = (c.previous.x - c.position.x, c.previous.z - c.position.z);
            if xd * xd + zd * zd > 0.001 {
                c.yaw = sunrise_core::geometry::trig::atan2_degrees_double(zd, xd);
                if c.flipped {
                    c.yaw += 180.0;
                }
            }
            let diff = (c.yaw - c.previous_yaw + 180.0).rem_euclid(360.0) - 180.0;
            if !(-170.0..170.0).contains(&diff) {
                c.yaw += 180.0;
                c.flipped = !c.flipped;
            }
            c.yaw %= 360.0;
            c.pitch %= 360.0;
        }
        if !self.minecart_improvements() {
            self.cart_block_effects(c);
            let here = c.position;
            self.cart_inside_blocks(c, slot, here, here);
        }
        self.push_from_ticked_cart(c, slot, index);
        c.in_water =
            c.push_by_fluid(
                &self.common.world,
                &self.common.res.motion().fluids,
                &self.common.loop_context,
            )
            .0 > 0.0;
        if c.in_water {
            c.fall_distance = 0.0;
        }
        self.entity_powder_snow_particles(c.previous, c.position, c.bounds(), c.velocity, false);
        match c.kind {
            CartKind::Furnace => {
                if c.fuel > 0 {
                    c.fuel -= 1;
                }
                if c.fuel <= 0 {
                    c.push = Vec3::ZERO;
                }
                if c.fuel > 0 {
                    // Clients emit LargeSmoke locally; keep c.random aligned with vanilla.
                    let _ = c.random.next_int_bound(4);
                }
            }
            CartKind::Tnt => {
                if c.fuse > 0 {
                    // Clients emit fuse Smoke locally after entity event 70.
                    c.fuse -= 1;
                } else if c.fuse == 0 {
                    let speed = c.velocity.x * c.velocity.x + c.velocity.z * c.velocity.z;
                    if self.explode_cart(c, speed) {
                        return false;
                    }
                }
                let speed = c.velocity.horizontal_distance_squared();
                if tnt_ignites_by_collision(c.ground.horizontal_collision, speed)
                    && self.explode_cart(c, speed)
                {
                    return false;
                }
            }
            CartKind::Spawner => {
                if c.spawner.as_ref().is_some_and(|s| s.spawn_delay <= 0)
                    && (mod_api::__private::listened(mod_api::HookId::MobSpawnPositionCheck)
                        || mod_api::__private::listened(mod_api::HookId::FinalizeSpawn))
                    && let Some(i) = cart_at(&self.carts, c.id, slot)
                {
                    // push_from_ticked_cart stored the moved cart. Bring its stored
                    // fluid state up to date for the shared spawner hooks; mutable
                    // hooks cannot touch it until the tick finishes.
                    self.carts.update(i, |stored| {
                        stored.velocity = c.velocity;
                        stored.in_water = c.in_water;
                        stored.fall_distance = c.fall_distance;
                    });
                }
                if let Some(mut spawner) = c.spawner.take() {
                    let key = (
                        c.position.x.floor() as i32,
                        c.position.y.floor() as i32,
                        c.position.z.floor() as i32,
                    );
                    let _ = self.server_tick_spawner_of(key, &mut spawner, Some(c.id));
                    c.spawner = Some(spawner);
                }
            }
            _ => {}
        }
        true
    }
    /// The rider's tick: the cart moves with the rider's input as its push.
    pub fn tick_cart_riding(&mut self, keys: Keys) -> bool {
        let Some(i) = self
            .common
            .carts
            .iter()
            .position(|c| Some(c.id) == self.common.riding)
        else {
            return false;
        };
        if keys.shift {
            self.dismount_cart(i);
            return true;
        }
        // ServerPlayer.getLastClientMoveIntent: getInputVector((left, 0, forward), 1, yRot).
        let left = f64::from(i8::from(keys.left) - i8::from(keys.right));
        let forward = f64::from(i8::from(keys.forward) - i8::from(keys.backward));
        let input = sunrise_core::geometry::Vec3::new(left, 0.0, forward);
        let v = sunrise_world::living_air::input_vector(input, 1.0, self.common.yaw);
        let intent = [v.x, v.z];
        let id = self.common.carts[i].id;
        let result =
            self.with_entity_held_out(id, |game| section_steps::tick_riding(game, id, i, intent));
        // A contact hurt lands on the stored row, not the ticked copy; a
        // breaking hit removes the cart and releases its seat.
        if self.apply_vehicle_contact_hurts(super::commands::Target::Cart(id)) {
            if self.common.riding == Some(id) && !self.persist.removed_vehicles.contains_key(&id) {
                self.common.riding = None;
            }
            return true;
        }
        let Some((_, alive, cart_step, player_step)) = result else {
            return true;
        };
        section_steps::tell(self, id, cart_step);
        section_steps::tell_player(self, player_step);
        // AbstractMinecart.activateMinecart: a powered activator rail ejects
        // the rider, and getDismountLocationForPassenger places it beside the
        // cart instead of leaving it at the seat.
        if alive
            && self.common.riding.is_none()
            && let Some(i) = self.common.carts.iter().position(|cart| cart.id == id)
        {
            self.dismount_cart(i);
        }
        true
    }
    /// Every cart but the ridden one (which ticks with its rider).
    pub fn tick_carts(&mut self) {
        self.vehicles_ticking(true);
        self.tick_carts_now();
        self.vehicles_ticking(false);
    }
    pub fn tick_carts_now(&mut self) {
        self.push_carts();
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut round = self.native_tick_round();
        // A server player's minecart moves with its rider (lan_vehicles.rs).
        let ridden = self.guest_vehicles();
        // By id: an exploding cart can destroy or prime others mid-loop.
        let ids: Vec<u32> = self.common.carts.iter().map(|c| c.id).collect();
        let mut index = CartIndex::default();
        let mut hint = 0;
        for &id in &ids {
            let Some(i) = cart_at(&self.common.carts, id, hint) else {
                continue;
            };
            hint = i + 1;
            let p = self.common.carts[i].position;
            if Some(id) == self.common.riding
                || ridden.contains(&id)
                || !ticking(p.x.floor() as i32, p.z.floor() as i32)
            {
                continue;
            }
            let kind = self.common.carts[i].kind.id();
            sunrise_threads::work::set_entity(sunrise_threads::work::entity_id(kind));
            if !self.native_tick_pre_one(&mut round, id, kind) {
                continue;
            }
            let Some(i) = self.cart_in_world(id, i, &round, kind) else {
                continue;
            };
            // The ticked copy takes the loot roll instead of copying it: only
            // opening the cart, a command or a save reads it, never a tick, and
            // a mineshaft's chest cart carries its whole roll as NBT. The
            // stored cart keeps its slots for whatever reads them meanwhile.
            let (due, section_step) =
                self.with_entity_held_out(id, |game| section_steps::tick(game, id, i, &mut index));
            // A contact hurt lands on the stored row, not the ticked copy.
            if self.apply_vehicle_contact_hurts(super::commands::Target::Cart(id)) {
                continue;
            }
            section_steps::tell(self, id, section_step);
            // A hook may have removed the cart during its tick: the cart's tick stops here.
            if cart_at(&self.common.carts, id, i).is_none() {
                continue;
            }
            if due {
                self.perform_cart_command(id);
                index.forget_carts();
            }
            // After the ticked cart is stored: a mod reads and changes the cart as it is now.
            self.native_tick_post_one(&round, id, kind);
        }
        sunrise_threads::work::clear_work();
        // On the server for every cart, the ridden ones too.
        let mut hint = 0;
        for id in ids {
            if round.canceled(id) {
                continue;
            }
            if let Some(i) = cart_at(&self.common.carts, id, hint) {
                hint = i + 1;
                let p = self.common.carts[i].position;
                if ticking(p.x.floor() as i32, p.z.floor() as i32) {
                    self.cart_lava_tick(id, i);
                }
            }
        }
    }
    pub(in crate::game::server_side) fn unpack_cart_loot(&mut self, i: usize) {
        self.unpack_loot(
            sunrise_gameplay_common::game::container_loot_level::LootTarget::Cart(
                self.common.carts[i].id,
            ),
            self.native_acting_player(),
        );
    }
}

#[cfg(any(test, feature = "test-support"))]
#[path = "minecarts/minecart_tnt_tests_server.rs"]
pub mod minecart_tnt_tests;

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::cart_rails::CartRailsGameExt1 as _;

use sunrise_gameplay_common::game::entity_common::EntityCommonGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

use sunrise_gameplay_common::game::scoreboard_common::CommonGameExt1 as _;

use sunrise_gameplay_common::game::minecarts::MinecartRailGameExt as _;
