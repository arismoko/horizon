use super::*;
use sunrise_gameplay_common::game::GameGameExt3 as _;

impl crate::game::ServerGame<'_> {
    /// `Player.aiStep` touching arrows in the guest's box grown by (1, 0.5, 1):
    /// `AbstractArrow.playerTouch` for a guest, as `touch_arrows` does for the
    /// host's player. The guest's own client plays the pickup sound
    /// (`handleTakeItemEntity`).
    pub fn guest_arrows(&mut self, gi: usize) {
        use sunrise_sim::entities::arrows::Touch;
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let (conn, id, at, creative, unable, dims, player, name) = {
            let g = &host.hub.guests[gi];
            (
                g.conn,
                g.entity_id,
                g.position,
                g.creative(),
                g.spectator() || g.dead || g.health <= 0.0,
                g.dimensions(),
                g.uuid,
                g.name.clone(),
            )
        };
        if unable || self.common.creatures.projectiles.is_empty() {
            return;
        }
        let w = f64::from(dims.width) / 2.0;
        let reach = Aabb::new(
            Vec3::new(at[0] - w, at[1], at[2] - w),
            Vec3::new(at[0] + w, at[1] + f64::from(dims.height), at[2] + w),
        )
        .inflate(Vec3::new(1.0, 0.5, 1.0));
        let mut projectiles = self.common.creatures.projectiles.lift();
        #[cfg(any(test, feature = "test-support"))]
        crate::game::server_side::entity_server::taken_pass_tests::after_take(self);
        let mut picked = Vec::new();
        projectiles.retain(|shot| {
            let Some(a) = shot.arrow.as_ref().filter(|a| !a.removed) else {
                return true;
            };
            let p = shot.position;
            let bounds = Aabb::new(
                Vec3::new(p.x - 0.25, p.y, p.z - 0.25),
                Vec3::new(p.x + 0.25, p.y + 0.5, p.z + 0.25),
            );
            if !bounds.intersects(reach) {
                return true;
            }
            let taken = match a.player_touch(creative) {
                Touch::Ignore => false,
                Touch::Take => true,
                // Inventory.add: a creative player takes what does not fit.
                Touch::Give(stack) => {
                    self.as_guest(conn, |g| g.add_stack_to_inventory(stack))
                        .flatten()
                        .is_none()
                        || creative
                }
            };
            if taken {
                picked.push(shot.clone());
            }
            !taken
        });
        self.common.creatures.projectiles.restore(
            projectiles,
            sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst,
        );
        for shot in picked {
            if let Some(arrow) = shot.arrow.as_ref() {
                self.native_player_pickup_arrow(player, &name, shot.id, &arrow.pickup_item.item);
            }
            if let Some(host) = self.server.lan.as_ref() {
                host.hub.take_item(shot.id as i32, id, 1, at);
            }
        }
    }
    /// `Player.tick`'s takeXpDelay and `Player.aiStep`'s touch of experience orbs.
    pub fn guest_orbs(&mut self, gi: usize) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let g = &host.hub.guests[gi];
        let own = g.bounds();
        let vehicle = g.vehicle;
        let (conn, id, at, player, name) =
            (g.conn, g.entity_id, g.position, g.uuid, g.name.clone());
        let unable = g.spectator() || g.dead || g.health <= 0.0;
        let play = host.play.entry(conn).or_default();
        let cooldown = if play.xp_delay > 0 {
            play.xp_delay -= 1;
            Some(play.xp_delay)
        } else {
            None
        };
        if let Some(cooldown) = cooldown {
            self.native_player_exp_cooldown_change(player, &name, cooldown);
        }
        let delayed = self
            .server
            .lan
            .as_ref()
            .is_none_or(|host| host.play.get(&conn).is_some_and(|play| play.xp_delay > 0));
        if unable || delayed {
            return;
        }
        let area = match vehicle.and_then(|v| self.guest_target_box(gi, v as i32)) {
            Some(vehicle) => own.union_bounds(vehicle).inflate(Vec3::new(1.0, 0.0, 1.0)),
            None => own.inflate(Vec3::new(1.0, 0.5, 1.0)),
        };
        let (min, max) = (
            [
                area.min_position().x,
                area.min_position().y,
                area.min_position().z,
            ],
            [
                area.max_position().x,
                area.max_position().y,
                area.max_position().z,
            ],
        );
        let touching: Vec<usize> = self
            .common
            .orbs
            .iter()
            .enumerate()
            .filter(|(_, o)| {
                o.position.x + 0.25 > min[0]
                    && o.position.x - 0.25 < max[0]
                    && o.position.y + 0.5 > min[1]
                    && o.position.y < max[1]
                    && o.position.z + 0.25 > min[2]
                    && o.position.z - 0.25 < max[2]
            })
            .map(|(i, _)| i)
            .collect();
        if touching.is_empty() {
            return;
        }
        let pick = self
            .common
            .world
            .random
            .next_int_bound(touching.len() as i32)
            .unwrap_or(0) as usize;
        let index = touching[pick];
        let join = self.common.orbs[index].join;
        let orb_id = self.common.orbs[index].id;
        // Swapping the guest in costs: only when a mod hears the pickup.
        let listened = (self.server.serving.is_some() || self.common.options.dedicated.is_some())
            && mod_api::__private::listened(mod_api::HookId::PickupXp);
        if listened && let Some(epoch) = self.common.native_lifecycle.epoch() {
            let orb = mod_api::ExperienceOrbRef::new(mod_api::EntityRef::new(epoch, orb_id));
            let mut canceled = false;
            let dimension = self.common.dim.id();
            let called = self.as_guest(conn, |game| {
                let player = game.native_acting_id();
                game.native_pickup_xp(player, dimension, orb, &mut canceled);
            });
            if called.is_none() || canceled {
                return;
            }
        }
        // A callback may have committed a discard: the row stays until removals flush.
        let live = |game: &Self| {
            !game.server.held_out.contains(&orb_id)
                && game.resident_source_target_visible(
                    game.common.dim.key(),
                    crate::game::commands::Target::Orb(orb_id),
                )
        };
        if !live(self) {
            return;
        }
        let Some(index) = self.common.orbs.locate(join) else {
            return;
        };
        let (orb_value, orb_pos) = {
            let orb = &self.common.orbs[index];
            (orb.value, orb.position)
        };
        // ExperienceOrb.playerTouch: delay, take, repair/award positive value,
        // then decrement count and discard at zero.
        if let Some(host) = self.server.lan.as_mut() {
            host.play.entry(conn).or_default().xp_delay = 2;
        }
        self.native_player_exp_cooldown_change(player, &name, 2);
        if let Some(host) = self.server.lan.as_ref() {
            host.hub.take_item(orb_id as i32, id, 1, at);
        }
        let mut random = sunrise_level::random_source::create();
        let pitch = (random.next_float() - random.next_float()) * 0.35 + 0.9;
        self.server_play_sound(
            "minecraft:entity.experience_orb.pickup",
            [orb_pos.x, orb_pos.y, orb_pos.z],
            0.1,
            pitch,
        );
        self.as_guest(conn, |game| {
            let rest = game.mending(orb_value);
            if rest > 0 {
                game.give_experience_points(rest);
            }
        });
        if !live(self) {
            return;
        }
        if let Some(index) = self.common.orbs.locate(join) {
            if self.common.orbs[index].count <= 1 {
                self.remove_orb(index);
            } else {
                self.common.orbs[index].count -= 1;
            }
        }
    }
    /// Dropped items touching a guest go into their inventory
    /// (`ItemEntity.playerTouch`, then `ServerPlayer.onItemPickup`).
    pub fn guest_pickups(&mut self, gi: usize) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let (conn, mode, position, id, uuid) = {
            let g = &host.hub.guests[gi];
            (
                g.conn,
                g.game_mode,
                g.position,
                g.entity_id,
                sunrise_sim::lan::playerdata::uuid_ints(g.uuid),
            )
        };
        if mode == SPECTATOR || self.common.dropped.is_empty() {
            return;
        }
        // The test below takes items whose position is less than 1.5 from
        // the player's on x and z: their boxes reach these columns.
        if let Some(columns) = self.server.pickup_columns.as_mut()
            && !columns.may_reach_area(
                &self.common.dropped,
                (
                    (position[0] - 1.5).floor() as i32,
                    (position[0] + 1.5).floor() as i32,
                ),
                (
                    (position[2] - 1.5).floor() as i32,
                    (position[2] + 1.5).floor() as i32,
                ),
            )
        {
            return;
        }
        let listens = mod_api::__private::listened(mod_api::HookId::ItemPickedUp);
        let near: Vec<_> = self
            .common
            .dropped
            .iter()
            .filter(|d| {
                (listens
                    || Self::pickup_decision(
                        d.pickup_delay,
                        d.target,
                        uuid,
                        mod_api::TriState::Default,
                    ))
                    && (d.position.x - position[0]).abs() < 1.5
                    && (d.position.z - position[2]).abs() < 1.5
                    && d.position.y > position[1] - 0.6
                    && d.position.y < position[1] + 2.4
            })
            .map(|d| d.id)
            .collect();
        let mut taken = Vec::new();
        for item_id in near.into_iter().rev() {
            self.native_guest_pickup(conn, item_id, uuid, &mut taken);
        }
        // Entity.take: the clients play the pickup sound themselves
        // (handleTakeItemEntity).
        if let Some(host) = self.server.lan.as_ref() {
            for (item, count) in taken {
                host.hub.take_item(item as i32, id, count as i32, position);
            }
        }
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;
