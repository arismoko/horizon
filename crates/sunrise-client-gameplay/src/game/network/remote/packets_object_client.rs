use super::*;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `C::AddEntity`: admit the entity before replacing an old ID.
    pub(in crate::game::client_side) fn apply_add_entity_packet(&mut self, p: play::AddEntity) {
        let kind = self
            .client
            .remote
            .as_ref()
            .and_then(|r| {
                r.builtin
                    .get("minecraft:entity_type")?
                    .name(p.kind)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        // `createEntityFromPacket`: an unlisted PlayerInfo still supplies its profile.
        if kind == "minecraft:player"
            && !self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.tab_players.contains_key(&p.uuid))
        {
            log::warn!(
                "Server attempted to add a player before PlayerInfo ({:032x})",
                p.uuid
            );
            return;
        }
        // ClientLevel.addEntity removes any existing entity with the same id before inserting.
        self.detach_camera_if_matching(|cam_id| cam_id == p.id);
        self.client.audio.0.remove_sound_entity(p.id as u32);
        let rotation = [p.yaw, p.pitch, p.head_yaw].map(sunrise_protocol::buf::unpack_degrees);
        // ClientLevel.addEntity discards the old entity before inserting the new one.
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.general.rows.contains_key(&p.id))
        {
            self.apply_remove_entities_packet(play::RemoveEntities { ids: vec![p.id] });
        }
        if let Some(r) = self.client.remote.as_mut() {
            // ClientLevel.addEntity: an id it holds already is removed
            // and added again, after the player.
            r.ticked_before_player.remove(&(p.id as u32));
            // A new entity looks its PlayerInfo up afresh.
            r.level_player_textures.remove(&p.id);
            r.level_player_infos.remove(&p.id);
            if let Some(old_uuid) = r.level_entity_uuids.insert(p.id, p.uuid)
                && old_uuid != p.uuid
                && r.level_uuids.get(&old_uuid) == Some(&p.id)
            {
                r.level_uuids.remove(&old_uuid);
            }
            if let Some(old_id) = r.level_uuids.insert(p.uuid, p.id)
                && old_id != p.id
                && r.level_entity_uuids.get(&old_id) == Some(&p.uuid)
            {
                r.level_entity_uuids.remove(&old_id);
            }
            if kind == "minecraft:player" {
                r.level_players.insert(p.id, p.uuid);
                r.level_player_infos
                    .insert(p.id, std::cell::OnceCell::new());
                // createEntityFromPacket: the player takes the profile of its list entry.
                match r.players.get(&p.uuid) {
                    Some(name) => r.level_player_names.insert(p.id, name.clone()),
                    None => r.level_player_names.remove(&p.id),
                };
            } else {
                r.level_players.remove(&p.id);
                r.level_player_names.remove(&p.id);
            }
        }
        // Entity.getTeam: a mob is on a team by its UUID, a player by
        // its profile name (`Game::scoreboard_name`).
        self.common
            .persist
            .uuids
            .insert(p.id as u32, sunrise_sim::lan::playerdata::uuid_ints(p.uuid));
        let holder = if kind == "minecraft:player" {
            Holder::Player(p.uuid)
        } else {
            Holder::Entity(p.id as u32)
        };
        // The constructor's `EntityEvent.Size` comes before `EntityConstructing`.
        let constructed = self.native_constructing_size(p.id as u32, &kind);
        self.initialize_entity(holder, &kind);
        let replace = self.tracks_remote_entity(p.id);
        self.spawn_entity(p.id, &kind, p.position, rotation);
        if let Some(r) = self.client.remote.as_mut() {
            r.cart_lerps.remove(&p.id);
            r.lerps.remove(&p.id);
        }
        if !self.spawn_remote_object(p.id, &kind, p.position, rotation, p.data) {
            self.spawn_added_projectile(&p, &kind, (rotation[0], rotation[1]), replace);
        }
        // Entity.recreateFromPacket: objects must exist before applying motion.
        self.remote_motion(p.id, p.velocity);
        if self.minecart_improvements()
            && self.common.carts.iter().any(|c| c.id == p.id as u32)
            && let Some(r) = self.client.remote.as_mut()
        {
            r.cart_lerps.insert(p.id, CartLerp::new());
        }
        self.move_remote_text_display(p.id, p.position, Some((rotation[0], rotation[1])));
        self.move_remote_interaction(p.id, p.position);
        self.pick_section_sync_id(p.id as u32);
        self.entity_section_joined(p.id as u32, p.position);
        self.native_constructed_size(p.id as u32, constructed);
    }
    /// `C::EntityPositionSync`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_entity_position_sync_packet(
        &mut self,
        p: play::EntityPositionSync,
    ) {
        let path = match p.path {
            play::PositionPath::Linear(at) => Path::Linear(at),
            play::PositionPath::Stepped(steps) => Path::Stepped(steps),
        };
        // handleEntityPositionSync: the position codec's base is set.
        if let Some(t) = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.entities.get_mut(&p.id))
        {
            t.codec_base = path.end();
        }
        self.remote_position_sync_and_file(p.id, path, p.yaw, p.pitch, p.on_ground);
    }
    /// `C::HurtAnimation`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_hurt_animation_packet(&mut self, p: play::HurtAnimation) {
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == p.id)
        {
            // Player.animateHurt: the tilt turns around the yaw.
            if let Some(pl) = self.common.player.as_mut() {
                pl.health.animate_hurt_dir(p.yaw);
            }
        } else if let Some(m) = self
            .common
            .mob_index
            .get_mut(&mut self.common.mobs, p.id as u32)
        {
            m.hurt_time = 10;
            if &*m.kind.id == "minecraft:player" {
                m.extra.hurt_dir = p.yaw;
            }
        } else {
            self.remote_dragon_hurt(p.id);
        }
    }
    /// `C::MoveVehicle`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_move_vehicle_packet(&mut self, p: play::MoveVehicle) {
        // handleMoveVehicle: the vehicle this client moves goes where the server says,
        // unless it is within 1e-5 already, and is reported back at once.
        if let Some(at) = self.driven_client_position() {
            let to = Vec3::new(p.position[0], p.position[1], p.position[2]);
            if to.subtract(at).length() > f64::from(1.0e-5_f32) {
                self.remote_move_vehicle_and_file(p.position, (p.yaw, p.pitch));
            }
            if let Some(report) = self.remote_vehicle_packets().into_iter().next() {
                self.send(report);
            }
        }
    }
    /// `C::ProjectilePower`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_projectile_power_packet(&mut self, p: play::ProjectilePower) {
        // ClientPacketListener.handleProjectilePowerPacket: the power a
        // hurting projectile accelerates by, sent bundled with its motion.
        super::super::remote_projectiles::remote_projectile_power(
            &mut self.common.creatures.projectiles,
            p.id,
            p.power,
        );
    }
    /// `C::RemoveEntities`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_remove_entities_packet(&mut self, p: play::RemoveEntities) {
        for &id in &p.ids {
            self.forget_remote_entity_construction(id);
            self.forget_client_collider(id);
            if let Some(r) = self.client.remote.as_mut() {
                if let Some(uuid) = r.level_players.remove(&id) {
                    r.attachments.remove(&Holder::Player(uuid));
                }
                r.level_player_names.remove(&id);
                r.level_player_textures.remove(&id);
                r.level_player_infos.remove(&id);
                r.attachments.remove(&Holder::Entity(id as u32));
            }
            self.pick_section_remove(id as u32);
        }
        self.remove_entities(&p.ids);
    }
    /// `C::RotateHead`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_rotate_head_packet(&mut self, p: play::RotateHead) {
        // ClientPacketListener.handleRotateMob: lerpHeadTo(yHeadRot, 3).
        if let Some(r) = self.client.remote.as_mut() {
            let yaw = sunrise_protocol::buf::unpack_degrees(p.head_yaw);
            r.head_lerps.insert(p.id, (yaw, 3));
        }
    }
    /// `C::SetEntityLink`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game::client_side) fn apply_set_entity_link_packet(
        &mut self,
        p: play::SetEntityLink,
    ) {
        // ClientPacketListener.handleEntityLinkPacket: who holds a mob's
        // lead (this player, another player, a mob or a fence knot), for
        // the rope.
        let me = self.client.remote.as_ref().map(|r| r.entity_id);
        use sunrise_sim::mobs::leash::Holder;
        let holder = match p.dest {
            0 => None,
            d if Some(d) == me => Some(Holder::Player),
            d => match self.client.remote_knots.get(&(d as u32)) {
                Some(&block) => Some(Holder::Knot(block)),
                // Not a known knot yet: kept by id until the knot
                // arrives (game/network/remote_objects.rs).
                None if self.common.boats.iter().any(|b| b.id == d as u32) => {
                    Some(Holder::Boat(d as u32))
                }
                None => Some(Holder::Mob(d as u32)),
            },
        };
        if let Some(m) = self
            .common
            .mob_index
            .get_mut(&mut self.common.mobs, p.source as u32)
        {
            m.extra.leash = holder;
        } else if let Some(i) = self
            .common
            .boats
            .iter()
            .position(|b| b.id == p.source as u32)
        {
            self.common.boats.update(i, |b| {
                // A leashed boat or raft (AbstractBoat is Leashable).
                b.leash = holder;
            });
        }
    }
    /// `C::SetEntityMotion`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_set_entity_motion_packet(
        &mut self,
        p: play::SetEntityMotion,
    ) -> Result<()> {
        // ClientPacketListener.handleSetEntityMotion: knockback on this player.
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == p.id)
        {
            if let Some(pl) = self.common.player.as_mut() {
                pl.velocity = Vec3::new(p.velocity[0], p.velocity[1], p.velocity[2]);
            }
            return Ok(());
        }
        // The same packet for another entity: Entity.lerpMotion.
        self.remote_motion(p.id, p.velocity);
        Ok(())
    }
    /// `C::SwingAnimation`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_swing_animation_packet(&mut self, p: play::SwingAnimation) {
        // ClientPacketListener.handleSwingAnimation: the entity swings
        // (LivingEntity.swing, which a swing in its first half ignores).
        let hand = if p.hand == 1 { Hand::Off } else { Hand::Main };
        let animation = sunrise_sim::first_person_sim::SwingAnimation {
            kind: p.kind.clamp(0, 255) as u8,
            duration: p.duration,
        };
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == p.id)
        {
            self.swing_hand(hand, animation);
        } else if let Some(m) = self
            .common
            .mob_index
            .get_mut(&mut self.common.mobs, p.id as u32)
        {
            m.swing_hand_modified(hand == Hand::Off, animation);
        }
    }
    /// `C::TakeItemEntity`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game::client_side) fn apply_take_item_entity_packet(
        &mut self,
        p: play::TakeItemEntity,
    ) {
        // A client mod hears the pickup while what was taken is still whole (#5595): only of an
        // entity this client tracks, as vanilla acts only on one (`if (from != null)`).
        if mod_api::__private::listened(mod_api::HookId::ClientTakeItemEntity)
            && let Some(collected) = u32::try_from(p.item).ok().and_then(|id| self.entity(id))
        {
            let collector = u32::try_from(p.player).ok().and_then(|id| self.entity(id));
            self.native_client_take_item_entity(collected, collector, p.amount);
        }
        // ClientPacketListener.handleTakeItemEntity: the pickup sound at
        // the orb, item or arrow, whoever collects it (the server sends
        // none); an entity this client doesn't track sounds at this player.
        let orb_at = self
            .common
            .orbs
            .iter()
            .find(|o| o.id == p.item as u32)
            .map(|o| o.position);
        let orb = orb_at.is_some();
        let item = self.common.dropped.iter().find(|d| d.id == p.item as u32);
        let shot = self
            .common
            .creatures
            .projectiles
            .iter()
            .find(|s| s.id == p.item as u32);
        let shot = shot.map(|s| s.position).or_else(|| {
            self.common
                .tridents
                .iter()
                .find(|t| t.id == p.item as u32)
                .map(|t| t.position)
        });
        let at = match (item, shot) {
            (Some(d), _) => Some([d.position.x, d.position.y, d.position.z]),
            (None, Some(s)) => Some([s.x, s.y, s.z]),
            (None, None)
                if self
                    .client
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.entity_id == p.player) =>
            {
                self.common.eye_position()
            }
            (None, None) => None,
        };
        // `ClientPacketListener.random`, the listener's own stream: a
        // pickup pitch is never a draw of the level's.
        if let Some(o) = orb_at {
            let mut r = sunrise_level::random_source::create();
            let pitch = (r.next_float() - r.next_float()) * 0.35 + 0.9;
            self.play_audio(
                "minecraft:entity.experience_orb.pickup",
                Some([o.x, o.y, o.z]),
                0.1,
                pitch,
            );
        } else if let Some(at) = at {
            let mut r = sunrise_level::random_source::create();
            let pitch = ((r.next_float() - r.next_float()) * 0.7 + 1.0) * 2.0;
            self.play_audio("minecraft:entity.item.pickup", Some(at), 0.2, pitch);
        }
        // ItemPickupParticle toward the collector (this player, another one or a mob).
        let collector = if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == p.player)
        {
            super::super::pickups::Collector::Player
        } else {
            super::super::pickups::Collector::Mob(p.player as u32)
        };
        if let Some(item) = self
            .common
            .dropped
            .iter()
            .find(|d| d.id == p.item as u32)
            .cloned()
        {
            self.add_pickup(
                super::super::pickups::Taken::Item(Box::new(item)),
                collector,
            );
        } else if let Some(orb) = self
            .common
            .orbs
            .iter()
            .find(|o| o.id == p.item as u32)
            .cloned()
        {
            self.add_pickup(super::super::pickups::Taken::Orb(Box::new(orb)), collector);
        } else if let Some(shot) = self
            .common
            .creatures
            .projectiles
            .iter()
            .find(|s| s.id == p.item as u32)
            .cloned()
        {
            // An arrow flies to its collector too, then goes.
            self.add_pickup(
                super::super::pickups::Taken::Arrow(Box::new(shot)),
                collector,
            );
        } else if let Some(i) = self
            .common
            .tridents
            .iter()
            .position(|t| t.id == p.item as u32)
        {
            // So does a trident: a server's client keeps them apart.
            let trident = self.common.tridents.remove(i);
            self.add_pickup(
                super::super::pickups::Taken::Trident(Box::new(trident)),
                collector,
            );
        }
        // An orb stays: the server may still hold merged ones in it. An item shrinks by the
        // amount taken and goes only when its stack is empty, so a survivor stays in view.
        if !orb {
            let remaining = self
                .common
                .dropped
                .iter_mut()
                .find(|d| d.id == p.item as u32)
                .map(|item| {
                    if item.stack.count > 0 {
                        item.stack.count = item.stack.count.saturating_sub(p.amount.max(0) as u32);
                    }
                    item.stack.count
                });
            if remaining.is_none_or(|count| count == 0) {
                self.remove_entity(p.item);
            }
        }
    }
    /// `C::TeleportEntity`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game::client_side) fn apply_teleport_entity_packet(
        &mut self,
        p: play::TeleportEntity,
    ) -> Result<()> {
        // handleTeleportEntity: setValuesFromPositionPacket counts a relative part
        // from where the entity is on this client (PositionMoveRotation.of).
        if self.driven_vehicle(p.id as u32).is_some() {
            use play::relative as rel;
            let Some((at, yaw, pitch, velocity)) = self.driven_vehicle(p.id as u32) else {
                return Ok(());
            };
            let pick = |bit: i32, current: f64, change: f64| {
                if p.relatives & bit != 0 {
                    current + change
                } else {
                    change
                }
            };
            let position = [
                pick(rel::X, at.x, p.position[0]),
                pick(rel::Y, at.y, p.position[1]),
                pick(rel::Z, at.z, p.position[2]),
            ];
            let turn = |bit: i32, current: f32, change: f32| {
                if p.relatives & bit != 0 {
                    current + change
                } else {
                    change
                }
            };
            let rotation = (
                turn(rel::Y_ROT, yaw, p.yaw),
                turn(rel::X_ROT, pitch, p.pitch),
            );
            let delta = Vec3::new(
                pick(rel::DELTA_X, velocity.x, p.delta[0]),
                pick(rel::DELTA_Y, velocity.y, p.delta[1]),
                pick(rel::DELTA_Z, velocity.z, p.delta[2]),
            );
            // The tracked endpoint stays the server's latest, for the relative
            // teleports that come once nobody here drives it.
            if let Some(t) = self
                .client
                .remote
                .as_mut()
                .and_then(|r| r.entities.get_mut(&p.id))
            {
                t.position = position;
            }
            self.teleport_driven_vehicle(position, rotation, delta, p.on_ground);
            return Ok(());
        }
        if self.client.text_displays.0.contains_key(&p.id) {
            self.teleport_remote_text_display(p.id, p.position, p.yaw, p.pitch, p.relatives);
            self.entity_section_moved_id(p.id as u32);
            return Ok(());
        }
        let (base, now_yaw, now_pitch, old_pos, old_yaw, old_pitch, now_vel) =
            if let Some(m) = self.common.mob_index.get(&self.common.mobs, p.id as u32) {
                let at = self
                    .client
                    .remote
                    .as_ref()
                    .and_then(|r| r.stepped.get(&p.id))
                    .map_or_else(|| mob_pos_rot(m), |h| h.heading(mob_pos_rot(m)));
                (
                    at.position,
                    at.yaw,
                    at.pitch,
                    [m.previous.x, m.previous.y, m.previous.z],
                    m.previous_yaw,
                    m.previous_head_pitch,
                    [m.velocity.x, m.velocity.y, m.velocity.z],
                )
            } else if let Some(i) = self.client.interactions.get(&p.id) {
                (
                    [i.position.x, i.position.y, i.position.z],
                    i.yaw,
                    i.pitch,
                    [i.previous.x, i.previous.y, i.previous.z],
                    i.previous_yaw,
                    i.previous_pitch,
                    [0.0; 3],
                )
            } else if let Some(a) = self
                .common
                .armor_stands
                .iter()
                .find(|a| a.id == p.id as u32)
            {
                let cur = PosRot {
                    position: [a.position.x, a.position.y, a.position.z],
                    yaw: a.yaw,
                    pitch: a.pitch,
                };
                let at = self
                    .client
                    .remote
                    .as_ref()
                    .and_then(|r| r.stepped.get(&p.id))
                    .map_or(cur, |h| h.heading(cur));
                (
                    at.position,
                    at.yaw,
                    at.pitch,
                    [a.previous.x, a.previous.y, a.previous.z],
                    a.previous_yaw,
                    a.previous_pitch,
                    [a.velocity.x, a.velocity.y, a.velocity.z],
                )
            } else if let Some(b) = self.common.boats.iter().find(|b| b.id == p.id as u32) {
                (
                    [b.position.x, b.position.y, b.position.z],
                    b.yaw,
                    b.pitch,
                    [b.previous.x, b.previous.y, b.previous.z],
                    b.previous_yaw,
                    b.pitch,
                    [b.velocity.x, b.velocity.y, b.velocity.z],
                )
            } else if let Some(c) = self.common.carts.iter().find(|c| c.id == p.id as u32) {
                (
                    [c.position.x, c.position.y, c.position.z],
                    c.yaw,
                    c.pitch,
                    [c.previous.x, c.previous.y, c.previous.z],
                    c.previous_yaw,
                    c.previous_pitch,
                    [c.velocity.x, c.velocity.y, c.velocity.z],
                )
            } else if let Some(t) = self
                .client
                .remote
                .as_ref()
                .and_then(|r| r.entities.get(&p.id))
            {
                (
                    t.position,
                    t.yaw,
                    t.pitch,
                    t.previous,
                    t.previous_yaw,
                    t.previous_pitch,
                    [0.0; 3],
                )
            } else {
                ([0.0; 3], 0.0, 0.0, [0.0; 3], 0.0, 0.0, [0.0; 3])
            };
        let resolve = |origin: [f64; 3], origin_yaw: f32, origin_pitch: f32| {
            let coord = |bit: i32, i: usize| {
                if p.relatives & bit != 0 {
                    origin[i] + p.position[i]
                } else {
                    p.position[i]
                }
            };
            let pos = [
                coord(play::relative::X, 0),
                coord(play::relative::Y, 1),
                coord(play::relative::Z, 2),
            ];
            let yaw = if p.relatives & play::relative::Y_ROT != 0 {
                origin_yaw + p.yaw
            } else {
                p.yaw
            };
            let pitch = (if p.relatives & play::relative::X_ROT != 0 {
                origin_pitch + p.pitch
            } else {
                p.pitch
            })
            .clamp(-90.0, 90.0);
            PosRot {
                position: pos,
                yaw,
                pitch,
            }
        };
        let target = resolve(base, now_yaw, now_pitch);
        let snap_old = resolve(old_pos, old_yaw, old_pitch);
        let vel = |bit: i32, i: usize| {
            if p.relatives & bit != 0 {
                now_vel[i] + p.delta[i]
            } else {
                p.delta[i]
            }
        };
        let delta = [
            vel(play::relative::DELTA_X, 0),
            vel(play::relative::DELTA_Y, 1),
            vel(play::relative::DELTA_Z, 2),
        ];
        // setValuesFromPositionPacket: interpolated unless too far; old pose resolved separately on snap.
        self.move_entity_with_old(
            p.id,
            Some(Path::Linear(target.position)),
            Some((target.yaw, target.pitch)),
            p.on_ground,
            MovePacket::Teleport,
            Some(snap_old),
        );
        self.remote_motion(p.id, delta);
        self.entity_section_moved_id(p.id as u32);
        Ok(())
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::entity_initialization_common::EntityInitializationCommonGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use crate::game::client_side::remote_client::minecarts::CartLerpClientExt as _;

use sunrise_gameplay_common::game::minecarts::MinecartRailGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
