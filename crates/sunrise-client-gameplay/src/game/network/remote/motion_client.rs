pub(in crate::game::client_side) use super::movement::stepped_move;
use super::*;
use sunrise_gameplay_common::game::entity::Entity;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `handleMoveVehicle`'s `getInterpolation().cancel()` for the mount this
    /// client drives.
    pub(in crate::game::client_side) fn cancel_driven_mount_interpolation(&mut self, id: u32) {
        if let Some(h) = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.stepped.get_mut(&(id as i32)))
        {
            h.cancel();
        }
    }
    /// `Entity.getClientPositionAndRotation` of the mount this client drives:
    /// while an interpolation runs, where it is going.
    pub(in crate::game::client_side) fn driven_mount_heading(&self, m: &Mob) -> PosRot {
        let at = mob_pos_rot(m);
        self.client
            .remote
            .as_ref()
            .and_then(|r| r.stepped.get(&(m.id as i32)))
            .map_or(at, |h| h.heading(at))
    }
    /// `Entity.commonTick` for the mount this client drives, `mobs[i]`: an
    /// interpolation it had going when the client took it over runs on, its
    /// target carried along by the mount's own moves (where the mount fits).
    /// The mount's pose before the step: its tick saves the old pose
    /// (setOldPosAndRot) before the step in vanilla, so `drive_mount` puts this one
    /// back as the old pose once the tick has run.
    pub(in crate::game::client_side) fn interpolate_driven_mount(
        &mut self,
        i: usize,
    ) -> Option<(Vec3, f32, f32)> {
        let id = self.common.mobs[i].id as i32;
        let handler = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.stepped.get_mut(&id))?;
        let m = &mut self.common.mobs[i];
        let mut scene = sunrise_level::collisions::SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: None,
        };
        handler.follow(mob_pos_rot(m), |to| {
            let bounds = m.bounds_at(Vec3::new(to[0], to[1], to[2]));
            scene.unobstructed(bounds).unwrap_or(false)
        });
        let before = (m.position, m.yaw, m.head_pitch);
        let at = handler.interpolate()?;
        place_interpolated(m, at);
        // A step that crosses a section is filed where it happens.
        let to = m.position;
        if to != before.0 {
            let from = before.0;
            self.entity_section_step(id as u32, [from.x, from.y, from.z], [to.x, to.y, to.z]);
        }
        Some(before)
    }
    /// The mount this client drives teleported to `at` (`moveOrInterpolateTo`):
    /// it eases there over its type's update interval.
    pub(in crate::game::client_side) fn interpolate_driven_mount_to(
        &mut self,
        i: usize,
        at: PosRot,
    ) {
        let m = &mut self.common.mobs[i];
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        let handler = r.stepped.entry(m.id as i32).or_insert_with(|| {
            let interval = sunrise_sim::lan::tracking::settings(&m.kind.id).1;
            SteppedInterpolation::new(interval as i32)
        });
        let entity = mob_pos_rot(m);
        if let Start::Snap(to) = handler.interpolate_to(
            entity,
            Some(Path::Linear(at.position)),
            Some((at.yaw, at.pitch)),
        ) {
            place_interpolated(m, to);
        }
    }
    /// A move packet for entity `id`: to the end of `path` (None: a
    /// rotation alone), turned to `rotation` if it has one. `packet` says
    /// whether a living entity more than 64 blocks away is put there at once
    /// (its `snaps`).
    pub(in crate::game) fn move_entity(
        &mut self,
        id: i32,
        path: Option<Path>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
        packet: MovePacket,
    ) {
        self.move_entity_with_old(id, path, rotation, on_ground, packet, None);
    }
    /// `Entity.lerpMotion` (and `recreateFromPacket`'s motion) for the
    /// entities whose own tick moves them on a client: dropped items, experience
    /// orbs, falling blocks and primed TNT.
    pub(in crate::game::client_side) fn remote_motion(&mut self, id: i32, velocity: [f64; 3]) {
        self.remote_dragon_motion(id, velocity);
        let v = Vec3::new(velocity[0], velocity[1], velocity[2]);
        // `falling` and `tnts` are searched from the back: AddEntity calls this right after
        // appending the block or TNT, so a spawn doesn't walk the whole list.
        if let Some(m) = self
            .common
            .mob_index
            .get_mut(&mut self.common.mobs, id as u32)
        {
            // Squid.aiStep derives its body pitch and roll from network motion.
            m.velocity = v;
        } else if let Some(d) = self.common.dropped.iter_mut().find(|d| d.id == id as u32) {
            d.velocity = v;
        } else if let Some(i) = self.common.falling.iter().rposition(|f| f.id == id as u32) {
            self.common.falling.update(i, |f| f.velocity = v);
        } else if let Some(t) = self
            .common
            .tnts
            .iter_mut()
            .rev()
            .find(|t| t.id == id as u32)
        {
            t.velocity = v;
        } else if let Some(o) = self.common.orbs.iter_mut().find(|o| o.id == id as u32) {
            o.velocity = v;
        } else if let Some(i) = self.common.carts.iter().position(|c| c.id == id as u32) {
            self.common.carts.update(i, |c| c.velocity = v);
        } else if let Some(i) = self.common.boats.iter().position(|b| b.id == id as u32) {
            // Entity.lerpMotion: the boat the player drives floats on from the server's motion
            // (`tick_remote_boat_bubbles` drops it again on a boat nobody here drives).
            self.common.boats.update(i, |b| b.velocity = v);
        } else if let Some(i) = self
            .common
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id == id as u32)
        {
            self.common.creatures.projectiles.update(i, |p| {
                p.velocity = v;
                // AbstractArrow.lerpMotion: the stuck timer restarts, and an
                // arrow the server set moving leaves the block it was stuck in.
                if let Some(st) = p.arrow.as_mut() {
                    st.life = 0;
                    if st.in_ground && v.length_squared() > 0.0 {
                        st.in_ground = false;
                        p.stuck = false;
                    }
                }
            });
        } else if let Some(t) = self.common.tridents.iter_mut().find(|t| t.id == id as u32) {
            t.velocity = v;
            // AbstractArrow.lerpMotion (ThrownTrident keeps it): `life`
            // restarts, and a trident set moving leaves its block.
            t.in_ground_time = 0;
            if t.in_ground && v.length_squared() > 0.0 {
                t.in_ground = false;
            }
        } else if let Some(r) = self.common.rockets.iter_mut().find(|r| r.id == id as u32) {
            r.velocity = v;
        } else if self.remote_hook_motion(id as u32, v) {
        } else if let Some(p) = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.pending_items.get_mut(&id))
        {
            p.velocity = v;
        } else if !crate::game::client_side::potions::remote_thrown_motion(
            &mut self.common.potions,
            id as u32,
            v,
        ) {
            self.common.end.remote_eye_motion(id as u32, velocity);
        }
    }

    /// `move_entity` with an explicit old pose (`setValuesFromPositionPacket`'s snapped old pose).
    pub(in crate::game) fn move_entity_with_old(
        &mut self,
        id: i32,
        path: Option<Path>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
        packet: MovePacket,
        snap_old: Option<PosRot>,
    ) {
        // The dragon is drawn from its own state (game/mobs/dragon.rs), which
        // tick_remote_dragon eases to the server's moves through `lerps`. Its
        // copy in the mob list draws nothing: given the move, it kept the
        // dragon where it spawned (#2083).
        let dragon = self
            .common
            .dragon
            .as_ref()
            .is_some_and(|d| d.id as i32 == id);
        let driven = self.driven_mount().is_some_and(|m| m.id as i32 == id)
            || u32::try_from(id).is_ok_and(|id| self.controls_boat(id));
        let mob = self
            .common
            .mob_index
            .get_mut(&mut self.common.mobs, id as u32)
            .filter(|_| !dragon);
        let tracked = self
            .client
            .remote
            .as_ref()
            .and_then(|r| r.entities.get(&id))
            .map(|t| t.position);
        let position = path
            .as_ref()
            .map(Path::end)
            .or(tracked)
            .or_else(|| {
                mob.as_ref()
                    .map(|m| [m.position.x, m.position.y, m.position.z])
            })
            .unwrap_or([0.0; 3]);
        if self.client.remote.is_some() {
            if let Some(r) = self.client.remote.as_mut() {
                if let Some(t) = r.entities.get_mut(&id) {
                    let d = [0, 1, 2].map(|i| position[i] - t.position[i]);
                    let snap = matches!(packet, MovePacket::PositionSyncSnap)
                        || (matches!(packet, MovePacket::Teleport | MovePacket::PositionSync)
                            && d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 4096.0);
                    if path.is_some() {
                        t.position = position;
                    }
                    if let Some((yaw, pitch)) = rotation {
                        if yaw.is_finite() {
                            t.yaw = if snap { yaw } else { yaw % 360.0 };
                        }
                        if pitch.is_finite() {
                            t.pitch = (pitch % 360.0).clamp(-90.0, 90.0);
                        }
                    }
                    if snap {
                        if let Some(old) = snap_old {
                            t.previous = old.position;
                            t.previous_yaw = if old.yaw.is_finite() { old.yaw } else { t.yaw };
                            t.previous_pitch = if old.pitch.is_finite() {
                                (old.pitch % 360.0).clamp(-90.0, 90.0)
                            } else {
                                t.pitch
                            };
                        } else {
                            t.previous = t.position;
                            t.previous_yaw = t.yaw;
                            t.previous_pitch = t.pitch;
                        }
                    }
                }
                if driven {
                    // Authoritative client mounts/boats keep their own position and interpolation.
                    return;
                }
                let entity_type_tags = &self
                    .common
                    .server_items
                    .as_deref()
                    .unwrap_or_else(|| self.common.res.items())
                    .entity_type_tags;
                if let Some(m) = mob {
                    // Shulker uses NO_OP interpolation: setPos immediately, keeping old block.
                    if m.species() == sunrise_sim::mobs::Species::Shulker {
                        r.stepped.remove(&id);
                        if path.is_some() {
                            sunrise_sim::mobs::shulker::client_set_pos(
                                m,
                                position[0],
                                position[1],
                                position[2],
                            );
                        }
                        if let Some((yaw, pitch)) = rotation {
                            m.yaw += wrap_degrees(yaw - m.yaw);
                            m.head_pitch = (pitch % 360.0).clamp(-90.0, 90.0);
                        }
                        let _ = m.update_retained_ground(
                            &self.common.world,
                            self.common.collision,
                            entity_type_tags,
                            on_ground,
                            None,
                        );
                        return;
                    }
                    // LivingEntity SteppedInterpolationHandler over its type's update interval.
                    let entity = PosRot {
                        position: [m.position.x, m.position.y, m.position.z],
                        yaw: m.yaw,
                        pitch: m.head_pitch,
                    };
                    let handler = r.stepped.entry(id).or_insert_with(|| {
                        let interval = sunrise_sim::lan::tracking::settings(&m.kind.id).1;
                        SteppedInterpolation::new(interval as i32)
                    });
                    if let Some(at) =
                        stepped_move(handler, packet, entity, position, path, rotation)
                    {
                        place_interpolated(m, at);
                        if let Some(old) = snap_old {
                            m.previous =
                                Vec3::new(old.position[0], old.position[1], old.position[2]);
                            m.previous_yaw = old.yaw;
                            m.previous_head_pitch = (old.pitch % 360.0).clamp(-90.0, 90.0);
                        } else {
                            m.previous = m.position;
                            m.previous_yaw = m.yaw;
                            m.previous_head_pitch = m.head_pitch;
                        }
                    }
                    // ClientPacketListener sets onGround after moving or queuing interpolation.
                    let _ = m.update_retained_ground(
                        &self.common.world,
                        self.common.collision,
                        entity_type_tags,
                        on_ground,
                        None,
                    );
                    return;
                }
                // Armor stands use the same stepped handler in `tick_remote_entities`.
                if let Some(a) = self
                    .common
                    .armor_stands
                    .iter_mut()
                    .find(|a| a.id == id as u32)
                {
                    let entity = PosRot {
                        position: [a.position.x, a.position.y, a.position.z],
                        yaw: a.yaw,
                        pitch: a.pitch,
                    };
                    let handler = r.stepped.entry(id).or_insert_with(|| {
                        let interval =
                            sunrise_sim::lan::tracking::settings("minecraft:armor_stand").1;
                        SteppedInterpolation::new(interval as i32)
                    });
                    if let Some(at) =
                        stepped_move(handler, packet, entity, position, path, rotation)
                    {
                        a.snap(
                            Vec3::new(at.position[0], at.position[1], at.position[2]),
                            at.yaw,
                            at.pitch,
                        );
                        if let Some(old) = snap_old {
                            a.previous =
                                Vec3::new(old.position[0], old.position[1], old.position[2]);
                            a.previous_yaw = old.yaw;
                            a.previous_pitch = (old.pitch % 360.0).clamp(-90.0, 90.0);
                        }
                    }
                    let _ = a.update_retained_ground(
                        &self.common.world,
                        self.common.collision,
                        entity_type_tags,
                        on_ground,
                        None,
                    );
                    return;
                }
            }
            // Interaction keeps NO_OP interpolation: the server's position replaces its own at once.
            if self.side_move_remote_interaction(
                id,
                position,
                path.is_some(),
                rotation,
                packet,
                snap_old,
            ) {
                return;
            }
            // Projectiles keep Entity's NO_OP InterpolationHandler too: the
            // server's position replaces theirs at once and their own tick
            // flies them on (`tick_remote_projectiles`). Eased towards it
            // instead, they only moved when an update came, every 10 to 20
            // ticks.
            if let Some(i) = self
                .common
                .creatures
                .projectiles
                .iter()
                .position(|p| p.id == id as u32)
            {
                self.common.creatures.projectiles.update(i, |p| {
                    if path.is_some() {
                        p.position = Vec3::new(position[0], position[1], position[2]);
                    }
                    if let Some((yaw, pitch)) = rotation {
                        p.yaw = yaw;
                        p.pitch = pitch;
                    }
                });
                return;
            }
            if path.is_some()
                && (crate::game::client_side::potions::move_remote_thrown(
                    &mut self.common.potions,
                    id as u32,
                    Vec3::new(position[0], position[1], position[2]),
                ) || self.common.end.move_remote_eye(id as u32, position))
            {
                return;
            }
            // A fishing hook keeps a LinearInterpolationHandler: eased towards the
            // update over three ticks, on top of its own movement (`tick_remote_hooks`).
            if path.is_some() {
                if let Some(h) = self.common.hook.as_mut().filter(|h| h.id() == id as u32) {
                    h.move_to(position, on_ground);
                    return;
                }
                if self.side_move_remote_hook(id as u32, position, on_ground) {
                    return;
                }
            }
            // A thrown trident is an arrow: placed at once, flown on by its tick.
            if let Some(t) = self.common.tridents.iter_mut().find(|t| t.id == id as u32) {
                if path.is_some() {
                    t.position = Vec3::new(position[0], position[1], position[2]);
                }
                if let Some((yaw, pitch)) = rotation {
                    t.yaw = yaw;
                    t.pitch = pitch;
                }
                return;
            }
            // So does a firework rocket: its own tick flies it on.
            if path.is_some()
                && let Some(f) = self.common.rockets.iter_mut().find(|f| f.id == id as u32)
            {
                f.position = Vec3::new(position[0], position[1], position[2]);
                return;
            }
            if let Some(p) = self
                .client
                .remote
                .as_mut()
                .and_then(|r| r.pending_items.get_mut(&id))
            {
                p.position = position;
            }
            // ItemEntity, FallingBlockEntity and PrimedTnt keep Entity's
            // NO_OP InterpolationHandler: moveOrInterpolateTo puts them there
            // at once, and their own tick moves them on between the server's
            // updates (`tick_remote_entities`, `tick_remote_projectiles`).
            let at = Vec3::new(position[0], position[1], position[2]);
            if let Some(d) = self.common.dropped.iter_mut().find(|d| d.id == id as u32) {
                d.position = at;
                d.ground.on_ground = on_ground;
                return;
            }
            if let Some(i) = self.common.falling.iter().rposition(|f| f.id == id as u32) {
                self.common.falling.update(i, |f| f.position = at);
                return;
            }
            if let Some(t) = self
                .common
                .tnts
                .iter_mut()
                .rev()
                .find(|t| t.id == id as u32)
            {
                t.position = at;
                t.ground.on_ground = on_ground;
                return;
            }
            // ExperienceOrb.createInterpolationHandler is a
            // LinearInterpolationHandler: the correction is spread over 3
            // ticks (`tick_remote_entities`), while `ExperienceOrb.tick` keeps
            // moving it.
            if let Some(o) = self.common.orbs.iter_mut().find(|o| o.id == id as u32) {
                o.ground.on_ground = on_ground;
                if let Some(r) = self.client.remote.as_mut() {
                    r.lerps.insert(
                        id,
                        Lerp {
                            target: position,
                            rotation: None,
                            steps: 3,
                        },
                    );
                }
                return;
            }
            let has_cart_lerp = self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.cart_lerps.contains_key(&id));
            if has_cart_lerp {
                // A minecart on NewMinecartBehavior keeps Entity's NO_OP
                // InterpolationHandler: a teleport puts it there at once.
                if let Some(i) = self.common.carts.iter().position(|c| c.id == id as u32) {
                    self.common.carts.update(i, |c| {
                        if path.is_some() {
                            c.position = at;
                        }
                        if let Some((yaw, pitch)) = rotation {
                            c.yaw = yaw;
                            c.pitch = pitch;
                        }
                    });
                }
            } else if !self.side_has_remote_text_display(id)
                && let Some(r) = self.client.remote.as_mut()
            {
                // AbstractInterpolationHandler.interpolateTo keeps the heading rotation when absent.
                let rotation = rotation.or_else(|| r.lerps.get(&id).and_then(|l| l.rotation));
                r.lerps.insert(
                    id,
                    Lerp {
                        target: position,
                        rotation,
                        steps: 3,
                    },
                );
            }
        }
        self.turn_remote_object(id, rotation.map(|(yaw, _)| yaw), on_ground);
        match packet {
            MovePacket::PositionSync | MovePacket::PositionSyncSnap => {
                self.position_sync_remote_text_display(id, position, rotation);
            }
            MovePacket::Move | MovePacket::Teleport => {
                self.move_remote_text_display_opt(id, path.is_some().then_some(position), rotation);
            }
        }
        self.turn_remote_projectile(id, rotation);
    }

    /// `Entity.moveOrInterpolateTo` for a client mod (`EntityRef::interpolate_to`):
    /// routes to the family's own interpolation handler, falling back to a direct
    /// set for families without one. Refuses bodies the client simulates
    /// (`Entity.canSimulateMovement`), non-finite inputs, stale handles and
    /// server-side calls.
    pub(in crate::game) fn entity_interpolate_to(
        &mut self,
        entity: mod_api::EntityRef,
        position: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> bool {
        if !position.iter().all(|v| v.is_finite())
            || rotation.is_some_and(|[yaw, pitch]| !yaw.is_finite() || !pitch.is_finite())
        {
            return false;
        }
        if self.common.is_logical_server() {
            return false;
        }
        let Ok((remote, id)) = self.native_client_entity_context(entity) else {
            return false;
        };
        let entity_id = entity.id();
        if id == remote.entity_id || self.resident_dragon_part(entity_id).is_some() {
            return false;
        }
        if remote.tracked_entity(id).is_none() {
            return false;
        }
        // Vanilla `Entity.canSimulateMovement`: refuse vehicles driven by the
        // local player (`AUTHORITATIVE_SIDE`) and every `SERVER_AND_CLIENT` body.
        let simulated = self.driven_mount().is_some_and(|m| m.id == entity_id)
            || self.controls_boat(entity_id)
            || self.common.dropped.iter().any(|d| d.id == entity_id)
            || remote.pending_items.contains_key(&id)
            || self.common.tnts.iter().any(|t| t.id == entity_id)
            || self.common.falling.iter().any(|f| f.id == entity_id)
            || self.common.orbs.iter().any(|o| o.id == entity_id)
            || self
                .common
                .creatures
                .projectiles
                .iter()
                .any(|p| p.id == entity_id)
            || self.common.tridents.iter().any(|t| t.id == entity_id)
            || self.common.rockets.iter().any(|f| f.id == entity_id)
            || self
                .common
                .hook
                .as_ref()
                .is_some_and(|h| h.id() == entity_id)
            || self.client.remote_hooks.iter().any(|h| h.id() == entity_id)
            || self.common.potions.has_thrown(entity_id)
            || self
                .common
                .dragon
                .as_ref()
                .is_some_and(|d| d.id == entity_id && (d.death_time > 0 || !d.alive()));
        if simulated {
            return false;
        }
        let on_ground = self.entity_on_ground(entity).unwrap_or(false);
        self.remote_move_entity_and_file(
            id,
            Some(Path::Linear(position)),
            rotation.map(|[yaw, pitch]| (yaw, pitch)),
            on_ground,
        );
        true
    }
}

use sunrise_gameplay_common::game::side_ops::player::PlayerSideGameExt as _;

use sunrise_gameplay_common::game::dragon_common::DragonCommonGameExt1 as _;
use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;
use sunrise_gameplay_common::game::native_entity_motion_values::NativeEntityMotionValuesGameExt1 as _;
