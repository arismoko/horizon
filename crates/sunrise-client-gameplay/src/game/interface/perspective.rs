//! The camera type (`CameraType`: first person, third person behind and
//! in front, cycled by F5), the detached camera (`Camera.setup` with
//! `getMaxZoom`), and the local player drawn as a model: in third person
//! and in the inventory (`InventoryScreen.extractEntityInInventoryFollowsMouse`).
use super::Screen;
use super::mob_blocks::MobView;
#[cfg(feature = "client")]
use super::native_mob_draw::PlayerRender;
use super::stock_layer_mask::StockLayerMask;
use mod_api::Hand;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use sunrise_client::{hud::HudBatch, mobs::render::FrameDraw, mouse::Rotation};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::collider_rays::{ClipData, ColliderRays};
use sunrise_level::camera_math::{CameraEntity, CameraType, LivingCamera};
use sunrise_level::collisions::SceneCollisions;
use sunrise_sim::{
    entities::nameplate::NameplateDistances,
    mobs::{Anim, Brain, Mob, MobType},
    pose::Pose,
};
use sunrise_world::{
    collision_context::{CollisionContext, CollisionEntity, CollisionItem},
    entity_obstruction::EntityIdentity,
    world_ray::{RayBlockMode, RayFluidMode, clip_world},
};

/// `Entity.isInvisible` for the window's player: shared flag 5 as its server
/// sets it (`shared_flag`), with the Invisibility effect or by a plugin's
/// `setInvisible` alone, as pose plugins hide the real body under the pose
/// (#3788); a world in the frame has only the effect.
pub(in crate::game::client_side) fn own_invisible(
    effects: &sunrise_level::effects::PlayerEffects,
    shared_flag: bool,
) -> bool {
    shared_flag || effects.get("invisibility").is_some()
}
/// `LivingEntityRenderer.isBodyVisible` and `isInvisibleTo` for the player's
/// own body, seen by itself: an invisible player (`own_invisible`) hides it
/// unless its team sees friendly invisibles, where vanilla draws it
/// translucent.
pub(in crate::game::client_side) fn body_visible(
    invisible: bool,
    team: Option<&sunrise_game_data::scoreboard::Team>,
) -> bool {
    !sunrise_game_data::scoreboard::invisible_to(team, team, false, invisible)
}
/// The player's model type for each resources root it was asked for.
pub(in crate::game) static PLAYER_TYPES: Mutex<Vec<(PathBuf, Option<Arc<MobType>>)>> =
    Mutex::new(Vec::new());

/// The player's model type (`PlayerModel` with the default skin), read once
/// per resources root: `entity-models.json` doesn't change while the game
/// runs, and parsing it again held every world join for about 100 ms. The
/// world resources read it as the game starts (`load_world_rest`), so a join
/// finds it; a resource pack change's new root is read once.
pub(in crate::game) fn player_type(root: &Path) -> Option<Arc<MobType>> {
    let mut cache = PLAYER_TYPES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, kind)) = cache.iter().find(|(cached, _)| cached == root) {
        return kind.clone();
    }
    let models = sunrise_client::mobs_render::EntityModelDocument::load(root).ok();
    let kind = models.as_ref().and_then(load_player_type);
    cache.push((root.to_owned(), kind.clone()));
    kind
}
/// `player_type` from a document the caller already parsed (`load_core`):
/// one parse for the whole startup rather than one more beside it.
pub(in crate::game) fn player_type_with_document(
    root: &Path,
    models: &sunrise_client::mobs_render::EntityModelDocument,
) -> Option<Arc<MobType>> {
    let mut cache = PLAYER_TYPES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, kind)) = cache.iter().find(|(cached, _)| cached == root) {
        return kind.clone();
    }
    let kind = load_player_type(models);
    cache.push((root.to_owned(), kind.clone()));
    kind
}
/// `player_type` from the document: checks presence without building a `Part`.
pub(in crate::game::client_side) fn load_player_type(
    models: &sunrise_client::mobs_render::EntityModelDocument,
) -> Option<Arc<MobType>> {
    models
        .keys()
        .any(|key| key == "minecraft:player#main")
        .then(|| {
            Arc::new(MobType {
                type_key: Default::default(),
                id: "minecraft:player".into(),
                // Players render as a humanoid; the species only drives local AI.
                species: sunrise_sim::mobs::Species::Zombie,
                width: 0.6,
                height: 1.8,
                eye_height: 1.62,
                category: "misc".into(),
                max_health: 20.0,
                speed: f64::from(0.1_f32), // Player.createAttributes' MOVEMENT_SPEED
                flying_speed: 0.02,
                attack_damage: 1.0,
                attack_knockback: 0.0,
                follow_range: 0.0,
                step_height: 0.6,
                armor: 0.0,
                armor_toughness: 0.0,
                knockback_resistance: 0.0,
                jump_strength: 0.42,
                attributes: Default::default(),
                item_stack_limits: Default::default(),
                attribute_bounds: Default::default(),
                gravity: 0.08,
                safe_fall_distance: 3.0,
                fall_damage_multiplier: 1.0,
                water_efficiency: 0.0,
                water_slowdown: 0.8,
                air_speed: 0.02,
                baby_scale: 0.5,
                head_rot_speed: 10.0,
                max_head_x_rot: 40.0,
                max_head_y_rot: 75.0,
                ambient_interval: 80,
                flags: Default::default(),
                brain: Brain::Passive,
                anim: Anim::Humanoid { zombie_arms: false },
                sounds: [
                    "entity.player.hurt",
                    "entity.player.hurt",
                    "entity.player.death",
                ],
                sound_overrides: Default::default(),
                food: Arc::from([]),
                tempt: Arc::from([]),
                passenger_attachments: Vec::new(),
            })
        })
}

/// Whether a shadow pass that draws the player (`shadow_draws_player`: shadowPlayer, or a
/// native pack's shadow) takes the player's own model in first person. A shader pack's shadow
/// pass does, except for a spectator: in our probe packs' readings of Iris 1.11.7's shadow map
/// in spectator mode, the texels at the first-person camera hold the terrain below it, where
/// Sunrise drew the spectator's model (vanilla's `PlayerModel` hides a spectator's body:
/// `showBody = !state.isSpectator`). A native pack's shadow keeps it.
pub(in crate::game::client_side) fn shadow_pass_takes_own_model(
    shadow_draws_player: bool,
    shader_pack: bool,
    spectator: bool,
) -> bool {
    shadow_draws_player && !(shader_pack && spectator)
}

pub(in crate::game) struct CameraAlignment {
    pub(in crate::game) entity: CameraEntity,
    pub(in crate::game) descending: bool,
    pub(in crate::game) entity_kind: sunrise_world::collision_context::EntityContextKind,
    pub(in crate::game) interpolating_minecart: bool,
}

pub(in crate::game::client_side) fn wrap(d: f32) -> f32 {
    let d = d % 360.0;
    if d >= 180.0 {
        d - 360.0
    } else if d < -180.0 {
        d + 360.0
    } else {
        d
    }
}
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `key.togglePerspective`: the next camera type, as a mod's `camera_type_cycle` leaves it.
    pub fn cycle_camera(&mut self) {
        self.apply_requested_camera_type();
        let previous = self.client.camera_type;
        let mut next = previous.cycle();
        self.native_camera_type_cycle(previous, &mut next);
        self.apply_camera_type(next);
    }
    /// `Options.setCameraType`, then `Minecraft.handleKeybinds`' check: the chain
    /// follows the first-person view.
    pub fn apply_camera_type(&mut self, camera_type: CameraType) {
        self.client.requested_camera_type.take();
        let previous = self.client.camera_type;
        self.client.camera_type = camera_type;
        if previous.is_first_person() != camera_type.is_first_person() {
            self.check_entity_post_effect(camera_type.is_first_person());
        }
    }
    /// Apply the last shared-host request through the same path as F5.
    pub(in crate::game) fn apply_requested_camera_type(&mut self) -> bool {
        let Some(camera_type) = self.client.requested_camera_type.take() else {
            return false;
        };
        let changed = self.client.camera_type != camera_type;
        self.apply_camera_type(camera_type);
        changed
    }
    /// Resolve the camera entity snapshot and collision context inputs for `Camera.alignWithEntity`.
    fn camera_alignment_source(&mut self, partial: f32) -> Option<CameraAlignment> {
        let subject = self.camera_subject()?;
        let mut entity = subject.camera;
        let minecart_position = self.remote_minecart_camera_position(&subject, partial);
        if let Some(position) = minecart_position {
            // Already evaluated at partial: do not linearly interpolate
            // between tick endpoints across a corner a second time.
            entity.position = position;
            entity.previous = position;
        }
        let descending = subject.is_local_player
            && self
                .common
                .player
                .as_ref()
                .is_some_and(|p| p.shift_key_down);
        Some(CameraAlignment {
            entity,
            descending,
            entity_kind: subject.collision_kind,
            interpolating_minecart: minecart_position.is_some(),
        })
    }

    fn align_view_with_angles(
        &self,
        source: &CameraAlignment,
        partial: f32,
        angles: (f32, f32, f32),
    ) -> sunrise_client::camera::Camera {
        let mut view = self.client.view;
        let kind = self.client.camera_type;
        let context = CollisionContext::for_entity(
            CollisionEntity {
                identity: EntityIdentity(0),
                kind: source.entity_kind,
                y: source.entity.position[1],
                descending: source.descending,
                main_hand: Some(CollisionItem::EMPTY),
            },
            false,
        );
        // `ClipContext.Block.VISUAL` with no fluids.
        let tables =
            sunrise_gameplay_common::game::native_block_tables::BlockTables::of(&self.common);
        let mut rays = ColliderRays {
            scene: SceneCollisions {
                world: &self.common.world,
                binding: self.common.collision,
                subject: None,
            },
            context,
            clip_data: Some(ClipData {
                facts: self.common.res.facts(),
                behaviours: self.common.res.behaviours(),
                tables: &tables,
                registry: self.common.pack.assets.registry,
            }),
            report_unloaded: false,
        };
        view.align_with_entity_angles(
            &source.entity,
            partial,
            kind,
            angles,
            |mut distance| {
                self.native_camera_distance(
                    partial,
                    kind.is_mirrored(),
                    &source.entity,
                    &mut distance,
                );
                distance
            },
            |from, to| {
                let hit = clip_world(
                    &mut rays,
                    Vec3::new(from[0], from[1], from[2]),
                    Vec3::new(to[0], to[1], to[2]),
                    RayBlockMode::Visual,
                    RayFluidMode::None,
                    4096,
                )
                .ok()?;
                (!hit.miss).then_some([hit.hit.location.x, hit.hit.location.y, hit.hit.location.z])
            },
        );
        view
    }

    /// `Camera.update` (`alignWithEntity` and `getMaxZoom`): the
    /// interpolated eye, or a point up to the camera distance behind (or in
    /// front of) it, pulled in where visual shapes are in the way. Returns the
    /// position, yaw and pitch to render from (without `compute_camera_angles`
    /// or `camera_position` mod hooks).
    pub fn camera_view(&mut self, partial: f32) -> Option<(glam::DVec3, f32, f32)> {
        if self.client.studio.flying() {
            return Some((
                self.client.studio.freecam.position,
                self.client.studio.freecam.yaw,
                self.client.studio.freecam.pitch,
            ));
        }
        let source = self.camera_alignment_source(partial)?;
        let view = self.align_view_with_angles(
            &source,
            partial,
            (
                source.entity.view_y_rot(partial),
                source.entity.view_x_rot(partial),
                0.0,
            ),
        );
        self.client.view = sunrise_client::camera::Camera {
            placed_position: view.position,
            ..view
        };
        Some((
            glam::DVec3::from_array(view.position),
            view.y_rot,
            view.x_rot,
        ))
    }

    /// The authoritative frame camera. NeoForge posts `ComputeCameraAngles` only on ordinary
    /// entity alignment, not in `Camera.alignWithEntity`'s interpolating minecart branch. The
    /// final `camera_position` stage sees either placed entity view, or the studio's custom view.
    /// Neither callback runs for the title panorama.
    pub fn frame_camera(&mut self, partial: f32) -> Option<(glam::DVec3, f32, f32, f32)> {
        self.apply_requested_camera_type();
        let run_hooks = !self.shows_panorama();
        let (mut view, alignment) = if self.client.studio.flying() {
            let mut view = self.client.view;
            if let Some(source) = self.camera_alignment_source(partial) {
                view.align_eye(&source.entity, partial);
            }
            let freecam = &self.client.studio.freecam;
            // Studio stores roll with the opposite sign to NeoForge's camera angles.
            view.set_rotation_with_roll(freecam.yaw, freecam.pitch, -freecam.roll);
            view.position = freecam.position.to_array();
            (view, None)
        } else {
            let source = self.camera_alignment_source(partial)?;
            let (mut yaw, mut pitch, mut roll) = (
                source.entity.view_y_rot(partial),
                source.entity.view_x_rot(partial),
                0.0,
            );
            if run_hooks && !source.interpolating_minecart {
                self.native_compute_camera_angles(partial, &mut yaw, &mut pitch, &mut roll);
                self.apply_requested_camera_type();
            }
            let angles = (yaw, pitch, roll);
            let view = self.align_view_with_angles(&source, partial, angles);
            (view, Some((source, angles)))
        };
        view.placed_position = view.position;
        if run_hooks {
            let mut position = mod_api::CameraPosition::placed_at(view.position);
            self.native_camera_position(partial, &mut position);
            if self.apply_requested_camera_type()
                && let Some((source, angles)) = alignment
            {
                view = self.align_view_with_angles(&source, partial, angles);
            }
            view.placed_position = view.position;
            for (coordinate, override_value) in view.position.iter_mut().zip(position.overrides()) {
                if let Some(value) = override_value {
                    *coordinate = value;
                }
            }
        }
        self.client.view = view;
        Some((
            glam::DVec3::from_array(view.position),
            view.y_rot,
            view.x_rot,
            view.roll,
        ))
    }
    /// `Camera.rotation()` as the quads of particles take it: the frame's camera (a
    /// bed's, a front view's, a mod's), not the player's own yaw and pitch.
    pub(in crate::game::client_side) fn particle_rotation(&self) -> glam::Quat {
        if self.client.studio.flying() {
            return self.camera_orientation();
        }
        let (yaw, pitch, roll) = self.client.posed_rotation.unwrap_or((
            self.client.view.y_rot,
            self.client.view.x_rot,
            self.client.view.roll,
        ));
        sunrise_gameplay_common::game::camera_rotation_rolled(
            CameraType::FirstPerson,
            yaw,
            pitch,
            roll,
        )
    }
    /// The player as `Camera.alignWithEntity` sees it.
    pub fn player_camera_entity(&mut self) -> Option<CameraEntity> {
        let (scale, distance) = {
            let p = self.common.player.as_mut()?;
            (
                p.attribute("minecraft:scale").unwrap_or(1.0) as f32,
                p.attribute("minecraft:camera_distance").unwrap_or(4.0),
            )
        };
        let p = self.common.player.as_ref()?;
        let sleeping = self.common.sleeping.map(|head| {
            let state = self.common.world.block(head[0], head[1], head[2]);
            let props = self
                .common
                .pack
                .assets
                .registry
                .state(state)
                .map(|s| &s.properties);
            // BedBlock.getBedOrientation as Direction.toYRot.
            props.and_then(|props| match props.get("facing").map(String::as_str) {
                Some("south") => Some(0.0),
                Some("west") => Some(90.0),
                Some("north") => Some(180.0),
                Some("east") => Some(270.0),
                _ => None,
            })
        });
        let mount = self
            .common
            .riding
            .and_then(|id| self.common.mobs.iter().find(|m| m.id == id))
            .map(Mob::camera_scale_and_distance);
        // AbstractBoat.positionRider turns a passenger by the boat's
        // deltaRotation after its yRotO was saved, and LocalPlayer.getViewYRot
        // interpolates a passenger's yaw: the boat's turn is spread over the
        // frames of a tick.
        let ride_turn = self
            .common
            .riding
            .and_then(|id| self.common.boats.iter().find(|b| b.id == id))
            .map_or_else(
                || {
                    self.common
                        .riding
                        .and_then(|id| self.common.carts.iter().find(|c| c.id == id))
                        .map_or(0.0, |c| c.passenger_yaw_delta)
                },
                |b| b.delta_rotation,
            );
        let entity = CameraEntity {
            position: [p.position.x, p.position.y, p.position.z],
            previous: [
                self.common.previous_position.x,
                self.common.previous_position.y,
                self.common.previous_position.z,
            ],
            // Entity.turn moves the old rotation with the new: the mouse
            // turns the view every frame without interpolation.
            x_rot: self.common.pitch,
            y_rot: self.common.yaw,
            x_rot_o: self.common.pitch,
            y_rot_o: self.common.yaw - ride_turn,
            eye_height: p.dimensions.eye_height,
            living: Some(LivingCamera {
                scale,
                camera_distance: distance,
                sleeping,
                dead: None,
            }),
            mount,
        };
        Some(entity)
    }

    /// Build the avatar for a newly installed local body, before any frame can read it.
    pub(in crate::game) fn ensure_player_model(&mut self) {
        if self.client.player_model.is_none()
            && let (Some(p), Some(kind)) =
                (&self.common.player, self.client_res().player_type().clone())
        {
            let mut model = Mob::new(0, kind, p.position, self.common.yaw);
            // The first position packet sets the look, not LivingEntity.yBodyRot.
            model.body_yaw = 0.0;
            model.previous_body_yaw = 0.0;
            model.head_pitch = self.common.pitch;
            model.previous_head_pitch = self.common.pitch;
            // The render state starts with the profile's paired default skin and model.
            let skin = sunrise_level::profile_lookup::default_skin(self.own_uuid());
            model.skin = Some(skin.into());
            model.slim = skin.contains("/slim/");
            self.client.player_model = Some(model);
        }
    }
    /// Keep the player's model in step with the player (`LivingEntity.tick`
    /// body turning, `WalkAnimationState`, the swing and the pose).
    pub fn tick_player_model(&mut self) {
        self.tick_player_model_with_rider_movement(None);
    }

    pub(in crate::game) fn tick_player_model_with_rider_movement(
        &mut self,
        rider_movement: Option<Vec3>,
    ) {
        let (scale, nameplate) = self.common.player.as_mut().map_or_else(
            || (1.0, NameplateDistances::default()),
            |p| {
                let scale = p.attribute("minecraft:scale").unwrap_or(1.0) as f32;
                let nameplate = NameplateDistances {
                    name_tag: p.attribute("minecraft:name_tag_distance").ok(),
                    below_name: p.attribute("minecraft:below_name_distance").ok(),
                };
                (scale, nameplate)
            },
        );
        if self.common.player.is_none() {
            self.client.player_model = None;
            return;
        }
        self.ensure_player_model();
        let p = self.common.player.as_ref().unwrap();
        let pose = p.pose;
        let crouching = pose == Pose::Crouching;
        let position = p.position;
        let (fall_flying, fly_ticks, velocity) = (p.fall_flying, p.fall_fly_ticks, p.velocity);
        let spinning = p.auto_spin_attack_ticks > 0;
        let using = self
            .common
            .using
            .map(|(_, ticks)| ticks as i32 + 1)
            .unwrap_or(0);
        let using_off_hand = self
            .common
            .using
            .is_some_and(|(slot, _)| slot == super::held_items::OFFHAND_SLOT);
        let use_animation = self
            .common
            .using
            .and_then(|(slot, _)| self.common.stack_in(slot))
            .map_or_else(Default::default, |stack| self.item_use_animation(stack));
        let charged = self.common.inventory[self.common.selected]
            .as_ref()
            .is_some_and(|s| s.data().is_some_and(|d| !d.charged.is_empty()));
        let charge_duration = self.common.inventory[self.common.selected]
            .as_ref()
            .map_or(25, |s| {
                sunrise_gameplay_common::game::CommonGame::charge_ticks(s).max(1)
            }) as i32;
        let attack = (self.common.swing.old_animation, self.common.swing.animation);
        let swinging = self.common.swing.swinging();
        // currentSwing.hand().asArm(mainArm).
        let swing_right_arm = swinging.then(|| {
            (self.common.swing.attack_hand() == mod_api::client::first_person::Hand::Main)
                != self.client.settings.main_hand_left
        });
        // LivingEntity.isVisuallySwimming and Entity.isInWater.
        let visually_swimming =
            p.pose == Pose::Swimming || (!p.fall_flying && p.pose == Pose::FallFlying);
        let in_water = p.fluids.was_touching_water;
        let in_lava = p.is_in_lava();
        let ticks_frozen = p.ticks_frozen;
        let (yaw, pitch) = (self.common.yaw, self.common.pitch);
        let main = self.common.inventory[self.common.selected].clone();
        let off = self.common.offhand.as_ref().map(|s| s.item.clone());
        let armor = self.common.armor.clone();
        let riding = self.common.riding.is_some();
        let hurt = p.health.hurt_animation_time;
        let avatar = self.client.avatar;
        let previous_position = self.common.previous_position;
        // LivingEntity.getBedOrientation: the model lies in the bed as
        // another player's does (`villager::sleep_pose`).
        let bed = self
            .common
            .sleeping
            .map_or(0, |head| self.common.bed_facing(head));
        let spectator = self.own_player_is_spectator();
        let boat_yaw = self
            .common
            .riding
            .and_then(|id| self.common.boats.iter().find(|b| b.id == id).map(|b| b.yaw));
        let blocking = boat_yaw.is_none()
            && self.common.using.is_some_and(|(slot, ticks)| {
                self.common
                    .stack_in(slot)
                    .and_then(|s| self.blocks_attacks(s))
                    .is_some_and(|c| {
                        sunrise_gameplay_common::game::held_items::block_delay_passed(&c, ticks)
                    })
            });
        // LivingEntity.DATA_ARROW_COUNT_ID and DATA_STINGER_COUNT_ID come from the server.
        let stuck = self
            .client
            .remote
            .as_ref()
            .map_or((0, 0), |r| (r.own_arrows, r.own_stingers));
        let Some(m) = self.client.player_model.as_mut() else {
            return;
        };
        m.extra.ints[1] = bed;
        m.extra.model_scale = scale;
        m.set_nameplate(nameplate);
        // The crouch flag is still last tick's here.
        let (from, at) =
            player_model_positions(previous_position, position, m.extra.crouching, crouching);
        m.previous = from;
        m.position = at;
        // The rider's own tick turns its body before the vehicle positions it
        // (Entity.rideTick). Seat displacement only updates the drawn position.
        let turn_movement = if riding {
            rider_movement.unwrap_or(Vec3::ZERO)
        } else {
            at.subtract(from)
        };
        let walked = player_walk_distance(m.previous, at);
        m.previous_yaw = m.yaw;
        m.previous_body_yaw = m.body_yaw;
        m.previous_head_yaw = m.head_yaw;
        m.previous_head_pitch = m.head_pitch;
        m.yaw = yaw;
        m.head_yaw = yaw;
        m.head_pitch = pitch;
        // AbstractBoat.positionRider follows the rider tick and directly assigns
        // the boat's yaw. A living vehicle affects only the extracted pose.
        m.body_yaw = boat_yaw.unwrap_or_else(|| {
            sunrise_sim::mobs::living::player_body_yaw(
                m.body_yaw,
                yaw,
                turn_movement.x,
                turn_movement.z,
                attack.1 > 0.0,
                blocking,
            )
        });
        if riding {
            // LivingEntity.calculateEntityAnimation: a passenger stops the
            // walk animation (WalkAnimationState.stop), so the seat moving with
            // the vehicle does not swing the legs.
            m.previous_limb_speed = 0.0;
            m.limb_speed = 0.0;
            m.limb_position = 0.0;
        } else {
            // WalkAnimationState.update(min(distance * 4, 1), 0.4).
            m.previous_limb_speed = m.limb_speed;
            m.limb_speed += ((walked * 4.0).min(1.0) - m.limb_speed) * 0.4;
            m.limb_position += m.limb_speed;
        }
        // Its idle swing holds with the world under `SUNRISE_STILL`.
        if self.common.clock.runs_normally() || !self.common.clock.still() {
            m.age = m.age.wrapping_add(1);
        }
        m.anim.attack_anim_o = attack.0;
        m.anim.attack_anim = attack.1;
        m.extra.set_main_hand_stack(main);
        m.extra.set_slot_id(sunrise_items::equipment::OFFHAND, off);
        // The worn stacks, for the elytra's glint (`WingsLayer`).
        for (slot, stack) in sunrise_items::equipment::ARMOR.into_iter().zip(armor) {
            m.extra.set_slot_stack(slot, stack);
        }
        m.extra.crouching = crouching;
        // Guests and server players take their pose from DATA_POSE, not here.
        m.extra.avatar_pose = pose;
        // What custom entity model animations read of the fluids (`is_in_water`, crawling).
        m.fluids.in_water = in_water;
        m.fluids.in_lava = in_lava;
        // Entity.DATA_TICKS_FROZEN: `isFullyFrozen` is the shiver of
        // `LivingEntityRenderer.isShaking` (#11217).
        m.ticks_frozen = ticks_frozen;
        m.extra.passenger = riding;
        m.extra.previous_velocity = m.velocity;
        m.velocity = velocity;
        m.extra.fall_flying = fall_flying;
        m.extra.fall_fly_ticks = fly_ticks;
        m.extra.auto_spin_attack = spinning;
        (m.extra.stuck.arrows, m.extra.stuck.stingers) = stuck;
        m.extra.shoulder_parrots = self
            .client
            .remote
            .as_ref()
            .map_or([None, None], |r| r.own_shoulders);
        m.extra.main_hand_left = self.client.settings.main_hand_left;
        m.extra.hidden_model_parts = !self.client.settings.file.model_customisation() & 0x7F;
        m.extra.spectator = spectator;
        m.extra.using_item = using;
        m.extra.using_off_hand = using_off_hand;
        m.extra.use_animation = use_animation;
        m.extra.charged = charged;
        m.extra.ints[5] = charge_duration;
        m.extra.elytra.tick(fall_flying, crouching, velocity);
        m.extra.swim.tick(visually_swimming, in_water);
        // The local player's ClientAvatarState, its cloak in the model's frame
        // (the model stands 0.125 lower while crouching).
        m.extra.avatar = avatar;
        if crouching {
            m.extra.avatar.cloak[1] -= 0.125;
            m.extra.avatar.cloak_o[1] -= 0.125;
        }
        m.extra.swing_right_arm = swing_right_arm;
        m.extra.swing_stab = self
            .common
            .swing
            .current
            .is_some_and(|c| c.animation.kind == 2);
        m.hurt_time = hurt;
    }

    /// `AvatarRenderer.extractRenderState` → `getArmPose`, once per frame before the frame's
    /// draws: every humanoid's held poses are selected from the swing and use state the draw
    /// then poses with.
    pub fn extract_arm_poses(&mut self) {
        self.extract_player_arm_pose();
        self.extract_remote_arm_poses();
    }
    /// The held poses of the window's own player model.
    pub fn extract_player_arm_pose(&mut self) {
        let Some(model) = self.client.player_model.as_ref() else {
            return;
        };
        let poses = self.held_arm_poses_with_stacks(
            self.own_entity_id() as u32,
            model,
            [self.common.selected, super::held_items::OFFHAND_SLOT]
                .map(|slot| self.common.stack_in(slot)),
        );
        self.client
            .player_model
            .as_mut()
            .unwrap()
            .extra
            .held_arm_pose = poses;
    }

    #[cfg(any(test, feature = "test-support"))]
    #[inline(always)]
    pub fn test_activate_studio_camera(
        &mut self,
        position: glam::DVec3,
        yaw: f32,
        pitch: f32,
        fov: f32,
        roll: f32,
    ) {
        self.client
            .studio
            .freecam
            .activate(position, yaw, pitch, fov);
        self.client.studio.freecam.roll = roll;
    }

    #[cfg(any(test, feature = "test-support"))]
    #[inline(always)]
    pub fn test_studio_camera_position(&self) -> glam::DVec3 {
        self.client.studio.freecam.position
    }
    /// `Entity.turn` for the player, by the mouse, a pad, the arrow keys or an
    /// agent's `turn`.
    /// The model's old pitch is the player's `xRotO`, which the turn moves
    /// with `xRot` (`turn_rotation`).
    pub fn turn_player(&mut self, mut xo: f64, mut yo: f64) {
        if self.client.studio.flying() {
            self.client
                .studio
                .freecam
                .turn((xo * 0.15) as f32, (yo * 0.15) as f32);
            return;
        }
        if self.common.player.is_some() && mod_api::__private::listened(mod_api::HookId::PlayerTurn)
        {
            let (orig_x, orig_y) = (xo, yo);
            self.native_player_turn(self.own_entity_id(), self.own_uuid(), &mut xo, &mut yo);
            xo = finite_turn_delta("x", orig_x, xo);
            yo = finite_turn_delta("y", orig_y, yo);
        }
        // `Entity.turn` adds the delta to the yaw and to `yRotO`: refuse a turn whose result
        // either sum cannot represent, as a whole, so the two never disagree.
        if !turn_yaw_representable(self.common.yaw, self.client.rope_rot_old.0, xo) {
            return;
        }
        let pitch_o = self
            .client
            .player_model
            .as_ref()
            .map(|m| m.previous_head_pitch);
        let rotation = turn_rotation(self.common.yaw, self.common.pitch, pitch_o, xo, yo);
        // The held lead's old look (`LocalPlayer.getRopeHoldPosition`'s
        // `yRotO`/`xRotO`) turns with the view (#10276).
        self.client.rope_rot_old = rope_old_after_turn(
            self.client.rope_rot_old,
            self.common.yaw,
            self.common.pitch,
            xo,
            yo,
        );
        self.common.yaw = rotation.y_rot;
        self.common.pitch = rotation.x_rot;
        if let Some(m) = self.client.player_model.as_mut() {
            m.previous_head_pitch = rotation.x_rot_o;
            m.head_pitch = rotation.x_rot;
        }
        // Entity.turn ends in AbstractBoat.onPassengerTurned: clampRotation
        // keeps the view within 105 degrees of the boat at every turn, not
        // only at the tick, so it never swings past and back.
        if let Some(b) = self
            .common
            .riding
            .and_then(|id| self.common.boats.iter().find(|b| b.id == id))
        {
            let (yaw, clamped) =
                sunrise_level::camera_math::clamp_boat_rotation(self.common.yaw, b.yaw);
            self.common.yaw = yaw;
            if let Some(m) = self.client.player_model.as_mut() {
                m.body_yaw = b.yaw;
            }
            // clampRotation moves `yRotO` with the view.
            self.client.rope_rot_old.0 += clamped;
        }
    }
    /// `Entity.setOldRot` after a server sets the player's rotation
    /// (`handleRotatePlayer`): the model's head snaps to it instead of easing
    /// in over the next tick.
    pub(in crate::game) fn snap_player_model_pitch(&mut self) {
        let pitch = self.common.pitch;
        if let Some(m) = self.client.player_model.as_mut() {
            m.previous_head_pitch = pitch;
            m.head_pitch = pitch;
        }
    }
    /// `LevelExtractor` submits the local player only when it holds the camera, and
    /// skips the first-person body unless it is sleeping. Studio freecam draws it too.
    pub(in crate::game::client_side) fn own_body_in_view(&self) -> bool {
        self.client.studio.flying()
            || (self.controls_camera()
                && (!self.client.camera_type.is_first_person() || self.common.sleeping.is_some()))
    }
    /// Whether the player's own model shows in the world view (its body or,
    /// while invisible, only its armour and held items).
    pub fn player_model_shown(&self) -> bool {
        (self.own_body_in_view()
            || (self.controls_camera()
                && shadow_pass_takes_own_model(
                    self.client.occlusion.0.shadow_casters.1,
                    self.client.pack_ids.is_some(),
                    self.own_player_is_spectator(),
                )))
            && self
                .common
                .player
                .as_ref()
                .is_some_and(|p| !p.health.dead_or_dying())
    }
    /// The player's own team: a client is in it by its remote profile name.
    pub(in crate::game::client_side) fn own_team(
        &self,
    ) -> Option<&sunrise_game_data::scoreboard::Team> {
        self.client
            .remote
            .as_ref()
            .and_then(|r| self.common.score.board.players_team(&r.profile_name))
    }
    /// `Entity.isInvisible` for the player itself (`own_invisible`).
    pub(in crate::game::client_side) fn own_invisible(&self) -> bool {
        own_invisible(
            &self.common.potions.effects,
            self.client.remote.as_ref().is_some_and(|r| r.own_invisible),
        )
    }
    /// Whether the player's own body is drawn (`body_visible`).
    pub(in crate::game::client_side) fn own_body_visible(&self) -> bool {
        body_visible(self.own_invisible(), self.own_team())
    }

    /// LivingEntityRenderer.solveBodyRot uses the current vehicle, even when a passenger
    /// packet arrived between ticks. These pose inputs do not change the live body angle.
    pub(in crate::game) fn extract_local_player_vehicle(&mut self) {
        if self.client.player_model.is_none() {
            return;
        }
        let vehicle_body = self.common.riding.and_then(|id| {
            if let Some(mob) = self.common.mob_object(id).filter(|mob| !mob.removed) {
                return Some((id, mob.previous_body_yaw, mob.body_yaw));
            }
            let stand = self
                .common
                .armor_stands
                .iter()
                .find(|stand| stand.id == id)?;
            Some((id, stand.previous_body_yaw, stand.body_yaw))
        });
        if let Some(model) = self.client.player_model.as_mut() {
            model.extra.mount = self.common.riding;
            model.extra.vehicle_body = vehicle_body;
        }
    }
    /// The local player's body in the ordered entity pass.
    pub(in crate::game::client_side) fn local_player_geometry(
        &self,
        camera: glam::DVec3,
        out: &mut Vec<sunrise_client_render::render_geometry::Vertex>,
    ) {
        let body_in_view = self.own_body_in_view();
        let start = out.len();
        self.mark_player(start, !body_in_view);
        self.mark_motion(start, sunrise_gameplay_common::game::MOTION_OWN_PLAYER);
        self.player_model_geometry(camera, self.own_player_alpha(), out);
        // A native pack's shadow pass alone gets the first-person model (no per-entity draws).
        if self.client.pack_ids.is_none() && !body_in_view {
            self.client
                .shadow_only_vertices
                .borrow_mut()
                .extend(out.drain(start..));
        }
    }

    /// The player's model for the entity pass in third person.
    pub fn player_model_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<sunrise_client_render::render_geometry::Vertex>,
    ) {
        if !self.player_model_shown() {
            return;
        }
        let Some(m) = self.client.player_model.as_ref() else {
            return;
        };
        let eye = self
            .common
            .player
            .as_ref()
            .map_or_else(|| m.eye_height(), |p| f64::from(p.dimensions.eye_height));
        let probe = entity_light_block(m.position, m.extra.crouching, eye);
        let light = self.common.world.light(probe[0], probe[1], probe[2]);
        let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
        let atlas = &self.client.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        let start = out.len();
        let body = self.own_body_visible();
        // The mods' pre hooks (`render_player_pre`, then `render_living_pre`), as for the other
        // players: a cancel skips the rest of this player's drawing and its post hooks, and the
        // frame's later name tag pass leaves it out too; its flames stay.
        let name = self.own_profile_name();
        let who = PlayerRender {
            name: &name,
            local: true,
        };
        let listened = self.mob_render_listened();
        let mut frame_draw = FrameDraw::default();
        if !self.mod_render_pre(
            m,
            Some(who),
            listened,
            alpha as f32,
            camera,
            packed,
            &mut frame_draw,
            out,
        ) {
            return;
        }
        // AvatarRenderer.shouldRenderLayers: a spectator draws no layers.
        let show_layers = sunrise_client::mobs::render::layers_shown(m);
        let resolved = self.entity_layers(m);
        let hidden = resolved.hidden;
        let layers = if show_layers { resolved.layers } else { None };
        frame_draw.capture_model =
            layers.is_some() || (listened && self.mob_render_post_listened(true));
        // A spectator still draws its own model in third person: the poses'
        // visibility (set by player_model_visibility) hides body parts, and
        // translucent_body marks the body OIT for the ghostly look. Only first
        // person and dead players skip it.
        if body || self.own_player_is_spectator() {
            sunrise_client::mobs::render::render_with_frame(
                m,
                alpha as f32,
                camera,
                &lookup,
                packed,
                self,
                &self.player_armor_layers(hidden),
                &mut frame_draw,
                out,
            );
        } else {
            sunrise_client::mobs::render::render_layers(
                m,
                alpha as f32,
                camera,
                &lookup,
                packed,
                self,
                &self.player_armor_layers(hidden),
                &mut frame_draw,
                out,
            );
        }
        let model = frame_draw.model();
        if show_layers {
            let chest = self.common.armor[1].as_ref().map(|s| &*s.item);
            let invisible = self.own_invisible();
            // CapeLayer and Deadmau5EarsLayer: `!state.isInvisible`, whoever sees the body.
            if !invisible && !hidden.contains(mod_api::StockLayer::Cape) {
                self.player_cape_geometry(m, chest, camera, alpha as f32, packed, out);
            }
            if !invisible && !hidden.contains(mod_api::StockLayer::Deadmau5Ears) {
                self.player_ears_geometry(m, &name, camera, alpha as f32, packed, out);
            }
            if !hidden.contains(mod_api::StockLayer::Wings) {
                self.player_elytra_geometry(m, chest, camera, alpha as f32, packed, out);
            }
            if !hidden.contains(mod_api::StockLayer::SpinAttackEffect) {
                self.spin_attack_geometry(m, camera, alpha as f32, packed, out);
            }
            if !hidden.contains(mod_api::StockLayer::ParrotOnShoulder) {
                self.shoulder_parrots_geometry(m, camera, alpha as f32, packed, out);
            }
            let view = MobView {
                camera,
                alpha,
                light: packed,
            };
            self.stuck_in_body_geometry(m, self.own_entity_id(), view, hidden, out);
            self.held_item_geometry(m, view, true, hidden, None, out);
            // CustomHeadLayer: a pumpkin, skull or other head item.
            if !hidden.contains(mod_api::StockLayer::CustomHead) {
                self.head_item_geometry(m, camera, alpha, packed, true, out);
            }
            // Mods' layers (`register_entity_layers`), over vanilla's.
            let stock_ctx = super::native_entity_layers::StockLayerContext {
                is_player: true,
                local_player: true,
                invisible,
                in_world: true,
                player_name: Some(&name),
                alpha,
                camera,
                light: packed,
                supported: resolved.supported,
                hidden,
            };
            self.mod_layer_geometry(
                m,
                Some(who),
                model,
                layers,
                &stock_ctx,
                alpha as f32,
                camera,
                packed,
                out,
            );
        }
        // The post hooks, whether or not the layers drew (a spectator draws none).
        self.mod_render_post(m, Some(who), model, alpha as f32, camera, packed, out);
        // LivingEntity.isCurrentlyGlowing: the Glowing effect.
        let vanilla = self
            .common
            .potions
            .effects
            .get("glowing")
            .map(|_| super::glowing::team_color(self.own_team()));
        let id = u32::try_from(self.own_entity_id()).unwrap_or(0);
        if let Some(color) = self.entity_outline(id, "minecraft:player", vanilla) {
            // LivingEntityRenderer.getRenderType: an invisible glowing body
            // still draws its outline.
            if !body {
                let mut silhouette = Vec::new();
                sunrise_client::mobs::render::render(
                    m,
                    alpha as f32,
                    camera,
                    &lookup,
                    packed,
                    self,
                    &mut silhouette,
                );
                self.mark_outline(&silhouette, color);
            }
            self.mark_outline(&out[start..], color);
        }
    }
    /// `extractEntityInInventoryFollowsMouse`: the player in the box
    /// `[x0, x1) x [y0, y1)`, `size` pixels per block, turned toward the
    /// cursor; drawn with a depth buffer and back-face culling.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game::client_side) fn draw_player_preview(
        &self,
        batch: &mut HudBatch,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        size: f32,
        offset_y: f32,
    ) {
        let Some(m) = self.client.player_model.as_ref() else {
            return;
        };
        if self
            .common
            .player
            .as_ref()
            .is_some_and(|p| p.health.dead_or_dying())
        {
            return;
        }
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let x_angle = ((cx - self.client.cursor.0) / 40.0).atan();
        let y_angle = ((cy - self.client.cursor.1) / 40.0).atan();
        let mut m = m.clone();
        // `extractEntityInInventory` divides the size by the entity's scale: the
        // preview is the same size whatever the player's scale.
        m.extra.model_scale = 0.0;
        // The render state: still, facing out, the head toward the cursor.
        m.previous = m.position;
        m.body_yaw = 180.0 + x_angle * 20.0;
        m.previous_body_yaw = m.body_yaw;
        m.head_yaw = m.body_yaw + x_angle * 20.0;
        m.previous_head_yaw = m.head_yaw;
        m.head_pitch = -y_angle * 20.0;
        m.previous_head_pitch = m.head_pitch;
        m.position.y += 0.125 * f64::from(u8::from(m.extra.crouching));
        m.previous = m.position;
        let camera = glam::DVec3::new(m.position.x, m.position.y, m.position.z);
        let atlas = &self.client.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        let mut verts = Vec::new();
        // A spectator preview still draws its own model: the poses'
        // visibility hides body parts, leaving the head and its layers.
        let body = self.own_body_visible() || self.own_player_is_spectator();
        let resolved = self.entity_layers(&m);
        let hidden = resolved.hidden;
        let layers = resolved.layers;
        let mut frame_draw = FrameDraw::new(layers.is_some());
        let (data, armour) = (
            self as &dyn sunrise_client::mobs::render::RenderDataSource,
            self.player_armor_layers(hidden),
        );
        let light = 240 | 240 << 8;
        // A screen preview: custom entity model animations see is_in_gui = 1.
        sunrise_client::mobs::cem_anim::in_gui(|| {
            if body {
                sunrise_client::mobs::render::render_with_frame(
                    &m,
                    1.0,
                    camera,
                    &lookup,
                    light,
                    data,
                    &armour,
                    &mut frame_draw,
                    &mut verts,
                );
            } else {
                sunrise_client::mobs::render::render_layers(
                    &m,
                    1.0,
                    camera,
                    &lookup,
                    light,
                    data,
                    &armour,
                    &mut frame_draw,
                    &mut verts,
                );
            }
        });
        let chest = self.common.armor[1].as_ref().map(|s| &*s.item);
        let invisible = self.own_invisible();
        let name = self.own_profile_name();
        if !invisible && !hidden.contains(mod_api::StockLayer::Cape) {
            self.player_cape_geometry(&m, chest, camera, 1.0, light, &mut verts);
        }
        if !invisible && !hidden.contains(mod_api::StockLayer::Deadmau5Ears) {
            self.player_ears_geometry(&m, &name, camera, 1.0, light, &mut verts);
        }
        // `WingsLayer` draws through `RenderTypes.armorCutoutNoCull`: the
        // wings' faces show from both sides (the elytra texture is empty on
        // the face that looks at the body).
        let wings_start = verts.len();
        if !hidden.contains(mod_api::StockLayer::Wings) {
            self.player_elytra_geometry(&m, chest, camera, 1.0, light, &mut verts);
        }
        let mut wings = wings_start..verts.len();
        if !hidden.contains(mod_api::StockLayer::ParrotOnShoulder) {
            self.shoulder_parrots_geometry(&m, camera, 1.0, light, &mut verts);
        }
        let view = MobView {
            camera,
            alpha: 1.0,
            light,
        };
        self.held_item_geometry(&m, view, false, hidden, None, &mut verts);
        if !hidden.contains(mod_api::StockLayer::CustomHead) {
            self.head_item_geometry(&m, camera, 1.0, light, false, &mut verts);
        }
        let who = PlayerRender {
            name: &name,
            local: true,
        };
        let stock_ctx = super::native_entity_layers::StockLayerContext {
            is_player: true,
            local_player: true,
            invisible,
            in_world: false,
            player_name: Some(&name),
            alpha: 1.0,
            camera,
            light,
            supported: resolved.supported,
            hidden,
        };
        if let Some(redrawn_wings) = self.mod_layer_geometry(
            &m,
            Some(who),
            frame_draw.model(),
            layers,
            &stock_ctx,
            1.0,
            camera,
            light,
            &mut verts,
        ) {
            wings = redrawn_wings;
        }
        let height = if m.extra.crouching { 1.5 } else { 1.8 };
        // PictureInPictureRenderer: the box centre, (size, size, -size);
        // then the translation and Rz(pi) * Rx(y_angle * 20 degrees).
        let gui = glam::Mat4::from_translation(glam::Vec3::new(cx, cy, 0.0))
            * glam::Mat4::from_scale(glam::Vec3::new(size, size, -size))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, height / 2.0 + offset_y, 0.0))
            * glam::Mat4::from_rotation_z(std::f32::consts::PI)
            * glam::Mat4::from_rotation_x((y_angle * 20.0).to_radians());
        let fire = self
            .common
            .player
            .as_ref()
            .filter(|p| p.is_on_fire() && !p.spectator)
            .map(|p| (p.dimensions.width, p.dimensions.height));
        let glints = preview_glint_quads(&verts, gui, wings.clone(), self.client.glint_frame);
        let quads = player_preview_quads_no_cull(verts, gui, fire, &atlas.sprites, wings);
        emit_preview(batch, quads, glints, [x0, y0, x1, y1]);
    }
    /// The survival inventory's player box (`InventoryScreen`).
    pub(in crate::game::client_side) fn draw_inventory_player(
        &self,
        batch: &mut HudBatch,
        ox: f32,
        oy: f32,
    ) {
        if self.client.screen == Screen::Inventory {
            self.draw_player_preview(
                batch,
                ox + 26.0,
                oy + 8.0,
                ox + 75.0,
                oy + 78.0,
                30.0,
                0.0625,
            );
        }
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `WingsLayer` for a player wearing `chest`.
    pub(in crate::game::client_side) fn player_elytra_geometry(
        &self,
        m: &Mob,
        chest: Option<&str>,
        camera: glam::DVec3,
        partial: f32,
        light: u32,
        out: &mut Vec<sunrise_client_render::render_geometry::Vertex>,
    ) {
        let stack = m
            .extra
            .slot_stack(sunrise_items::equipment::CHEST)
            .filter(|s| chest == Some(&*s.item));
        self.wings_geometry(
            chest,
            stack,
            m.cape.as_deref(),
            false,
            sunrise_client::mobs::render::base_transform(m, partial, camera),
            m.extra.elytra.angles(partial),
            m.extra.crouching,
            light,
            self.wings_overlay(stack),
            out,
        );
    }
    /// `WingsLayer` draws through `EquipmentLayerRenderer.renderLayers`: the
    /// armour glint when the worn elytra `hasFoil()`.
    pub(in crate::game::client_side) fn wings_overlay(
        &self,
        stack: Option<&sunrise_items::items::ItemStack>,
    ) -> u32 {
        let item_glint = |item: &str| {
            let props = &self.common.res.items().enchantments.props;
            props.get(item).and_then(|p| p.glint)
        };
        match stack {
            Some(s) if s.has_foil(item_glint(&s.item)) => sunrise_client::mobs::render::ARMOR_GLINT,
            _ => 0,
        }
    }
    /// `WingsLayer.submit` for any wearer: the wings of `chest`'s equipment
    /// asset on `base` (the wearer's model transform), `baby` for the baby
    /// elytra model. `cape` is a player's own cape texture, which has its
    /// elytra (`WingsLayer.getPlayerElytraTexture`). `stack` is the worn
    /// stack when known: an OptiFine CIT elytra rule for it replaces the
    /// texture, the cape's too, as OptiFine's `LayerElytra` does.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game::client_side) fn wings_geometry(
        &self,
        chest: Option<&str>,
        stack: Option<&sunrise_items::items::ItemStack>,
        cape: Option<&str>,
        baby: bool,
        base: glam::Mat4,
        angles: [f32; 3],
        crouching: bool,
        light: u32,
        overlay: u32,
        out: &mut Vec<sunrise_client_render::render_geometry::Vertex>,
    ) {
        // The worn stack's own `equippable`, for the chest slot, with an asset
        // (getEquipmentIfRenderable: anything else counts as nothing worn).
        let equippable = match stack {
            Some(s) => sunrise_sim::item_sync::equippable(s, self.items()),
            None => {
                chest.and_then(|item| sunrise_sim::item_sync::item_equippable(item, self.items()))
            }
        };
        let Some(asset) = equippable
            .filter(|e| e.slot == "chest")
            .and_then(|e| e.asset_id)
        else {
            return;
        };
        let assets = &self.client.armor_stand_assets.0;
        let Some(layer) = assets
            .equipment
            .get(&asset)
            .and_then(|k| k.get("wings"))
            .and_then(|layers| layers.first())
        else {
            return;
        };
        let sprites = &self.client.gui.atlas.sprites;
        let rule = stack.and_then(|s| self.cit_worn_rule(sunrise_client::cit::Kind::Elytra, s));
        let rect = sunrise_client::cit_armor::wings(rule, cape, |s| sprites.get(s));
        let (Some(model), Some(rect)) = (
            assets
                .player
                .get(if baby { "elytra_baby" } else { "elytra" }),
            rect.or_else(|| sprites.get(&layer.texture)),
        ) else {
            return;
        };
        let base = base * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0, 0.125));
        let pose = |part: &sunrise_client::mobs::Part| {
            let mut p = part.pose;
            if part.name == "left_wing" || part.name == "right_wing" {
                let invert = if part.name == "left_wing" { 1.0 } else { -1.0 };
                p[1] = if crouching { 3.0 } else { 0.0 };
                p[3] = angles[0];
                p[4] = invert * angles[1];
                p[5] = invert * angles[2];
            }
            p
        };
        sunrise_client::mobs_render::render_model(
            model,
            base,
            &pose,
            [rect.u0, rect.v0, rect.u1, rect.v1],
            light,
            overlay,
            out,
        );
        // The elytra texture's OptiFine emissive overlay (not a cape's), full bright.
        let glow = cape
            .and_then(|c| sprites.get(c))
            .is_none()
            .then(|| {
                let emissive = self
                    .client_res()
                    .client_mob_data()
                    .emissive
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                emissive
                    .overlay(&layer.texture)
                    .and_then(|e| sprites.get(e))
                    .copied()
            })
            .flatten();
        if let Some(g) = glow {
            sunrise_client::mobs_render::render_model(
                model,
                base,
                &pose,
                [g.u0, g.v0, g.u1, g.v1],
                sunrise_client::mobs::render::FULL_BRIGHT,
                0,
                out,
            );
        }
    }
    /// `CapeLayer` for a player wearing `chest`: no cape under an elytra's
    /// wings, and moved out over a chestplate.
    pub(in crate::game::client_side) fn player_cape_geometry(
        &self,
        m: &Mob,
        chest: Option<&str>,
        camera: glam::DVec3,
        partial: f32,
        light: u32,
        out: &mut Vec<sunrise_client_render::render_geometry::Vertex>,
    ) {
        let Some(rect) = m
            .cape
            .as_deref()
            .and_then(|c| self.client.gui.atlas.sprites.get(c))
        else {
            return;
        };
        if m.is_invisible() {
            return;
        }
        // CapeLayer.hasLayer: the worn stack's own `equippable` asset.
        let items = self.common.res.items();
        let equippable = match m
            .extra
            .slot_stack(sunrise_items::equipment::CHEST)
            .filter(|s| chest == Some(&*s.item))
        {
            Some(s) => sunrise_sim::item_sync::equippable(s, items),
            None => chest.and_then(|item| sunrise_sim::item_sync::item_equippable(item, items)),
        };
        let layers = equippable
            .filter(|e| e.slot == "chest")
            .and_then(|e| e.asset_id)
            .and_then(|asset| self.client.armor_stand_assets.0.equipment.get(&asset));
        if layers.is_some_and(|k| k.contains_key("wings")) {
            return;
        }
        let mut base = sunrise_client::mobs::render::base_transform(m, partial, camera);
        if layers.is_some_and(|k| k.contains_key("humanoid")) {
            base *= glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.053125, 0.06875));
        }
        sunrise_client::mobs::render::cape(
            m,
            partial,
            base,
            [rect.u0, rect.v0, rect.u1, rect.v1],
            light,
            self,
            out,
        );
    }
    /// `Deadmau5EarsLayer` (`AbstractClientPlayer.showExtraEars`): the ears
    /// of a player whose profile name is `deadmau5`, in the player's skin.
    pub(in crate::game::client_side) fn player_ears_geometry(
        &self,
        m: &Mob,
        name: &str,
        camera: glam::DVec3,
        partial: f32,
        light: u32,
        out: &mut Vec<sunrise_client_render::render_geometry::Vertex>,
    ) {
        if name != "deadmau5" {
            return;
        }
        let sprites = &self.client.gui.atlas.sprites;
        // The body's own fallback: a skin with no sprite draws the default.
        let Some(rect) = m.skin.as_deref().and_then(|s| sprites.get(s)).or_else(|| {
            self.client_res()
                .client_mob_models()
                .texture(&m.kind.id)
                .and_then(|t| sprites.get(t))
        }) else {
            return;
        };
        sunrise_client::mobs::render::ears(
            m,
            partial,
            sunrise_client::mobs::render::base_transform(m, partial, camera),
            [rect.u0, rect.v0, rect.u1, rect.v1],
            light,
            self,
            out,
        );
    }
    /// Decide in the render layer, fire with a ClientHost, then stage the answer
    /// on this tick's model. Stack snapshots are converted only with subscribers.
    pub fn held_arm_poses(
        &self,
        entity_id: u32,
        m: &Mob,
    ) -> [Option<sunrise_core::arm_pose::ArmPose>; 2] {
        self.held_arm_poses_with_stacks(
            entity_id,
            m,
            [
                sunrise_items::equipment::MAINHAND,
                sunrise_items::equipment::OFFHAND,
            ]
            .map(|slot| m.extra.slot_stack(slot)),
        )
    }

    pub(in crate::game::client_side) fn held_arm_poses_with_stacks(
        &self,
        entity_id: u32,
        m: &Mob,
        stacks: [Option<&sunrise_items::items::ItemStack>; 2],
    ) -> [Option<sunrise_core::arm_pose::ArmPose>; 2] {
        use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEngine};
        use sunrise_sim::mobs::anim::humanoid_arm_poses;
        let stacks =
            stacks.map(|stack| stack.filter(|s| s.count > 0 && &*s.item != "minecraft:air"));
        let mut poses = stacks.map(|stack| {
            stack
                .and_then(|s| self.items().get(&s.item))
                .and_then(|info| info.arm_pose)
        });
        if !mod_api::__private::listened(mod_api::HookId::GetArmPose) {
            return poses;
        }
        let [main_arm_pose, off_arm_pose] = humanoid_arm_poses(m, poses, m.anim.attack_anim);
        let api_stacks = stacks.map(|stack| stack.and_then(|stack| stack.try_into_api().ok()));
        let humanoid = mod_api::HumanoidRenderState {
            main_hand: api_stacks[0].as_ref(),
            off_hand: api_stacks[1].as_ref(),
            main_arm_left: m.main_arm_left(),
            is_using_item: m.extra.using_item > 0,
            use_hand: (m.extra.using_item > 0).then_some(if m.extra.using_off_hand {
                Hand::Off
            } else {
                Hand::Main
            }),
            ticks_using_item: m.extra.using_item,
            use_animation: m.extra.use_animation,
            is_swinging: m.is_swinging(),
            main_arm_pose: main_arm_pose.try_into_api().unwrap(),
            off_arm_pose: off_arm_pose.try_into_api().unwrap(),
        };
        for ((pose, stack), hand) in poses
            .iter_mut()
            .zip(api_stacks.iter().map(Option::as_ref))
            .zip(Hand::BOTH)
        {
            let Some(stack) = stack else {
                continue;
            };
            let mut answer = None;
            self.native_get_arm_pose(entity_id, hand, stack, &humanoid, &mut answer);
            if let Some(answer) = answer
                && let Ok(answer) = answer.try_into_engine(&())
            {
                *pose = Some(answer);
            }
        }
        self.transform_selected_arm_poses(m, &mut poses, humanoid);
        poses
    }
    /// `ArrowLayer` and `BeeStingerLayer` (`StuckInBodyLayer`) for a player:
    /// the arrows and bee stingers stuck in the body, placed from the entity
    /// id `id`.
    pub(in crate::game::client_side) fn stuck_in_body_geometry(
        &self,
        m: &Mob,
        id: i32,
        view: MobView,
        hidden_layers: StockLayerMask,
        out: &mut Vec<sunrise_client_render::render_geometry::Vertex>,
    ) {
        let MobView {
            camera,
            alpha,
            light,
        } = view;
        let partial = alpha as f32;
        let sprites = &self.client.gui.atlas.sprites;
        let layers = [
            (
                mod_api::StockLayer::Arrow,
                m.extra.stuck.arrows,
                false,
                self.client.arrow_model.as_ref(),
                "entity/projectiles/arrow",
            ),
            (
                mod_api::StockLayer::BeeStinger,
                m.extra.stuck.stingers,
                true,
                self.client.bee_stinger_model.as_ref(),
                "entity/bee/bee_stinger",
            ),
        ];
        for (stock, count, on_surface, model, texture) in layers {
            if hidden_layers.contains(stock) {
                continue;
            }
            let (true, Some(model), Some(rect)) = (count > 0, model, sprites.get(texture)) else {
                continue;
            };
            sunrise_client::mobs::render::stuck_in_body(
                m,
                id,
                partial,
                camera,
                self,
                count,
                on_surface,
                model,
                [rect.u0, rect.v0, rect.u1, rect.v1],
                light,
                out,
            );
        }
    }
    /// `ParrotOnShoulderLayer`: the parrots the player keeps in its
    /// `ShoulderEntityLeft` and `ShoulderEntityRight` (game/mobs/shoulders.rs).
    pub(in crate::game::client_side) fn shoulder_parrots_geometry(
        &self,
        m: &Mob,
        camera: glam::DVec3,
        partial: f32,
        light: u32,
        out: &mut Vec<sunrise_client_render::render_geometry::Vertex>,
    ) {
        let Some(parrot) = self
            .client_res()
            .client_mob_models()
            .get("minecraft:parrot")
        else {
            return;
        };
        let atlas = &self.client.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        let base = sunrise_client::mobs::render::base_transform(m, partial, camera);
        for left in [true, false] {
            if let Some(variant) = m.extra.shoulder_parrots[usize::from(!left)] {
                sunrise_client::mobs::render::shoulder_parrot(
                    m, parrot, variant, left, partial, base, &lookup, light, out,
                );
            }
        }
    }
    /// `SpinAttackEffectLayer`: Riptide's two boxes turning around a spinning
    /// player (`SpinAttackEffectModel.setupAnim`).
    pub(in crate::game::client_side) fn spin_attack_geometry(
        &self,
        m: &Mob,
        camera: glam::DVec3,
        partial: f32,
        light: u32,
        out: &mut Vec<sunrise_client_render::render_geometry::Vertex>,
    ) {
        if !m.extra.auto_spin_attack {
            return;
        }
        let (Some(model), Some(rect)) = (
            self.client_res().spin_attack().as_ref(),
            self.client
                .gui
                .atlas
                .sprites
                .get("entity/trident/trident_riptide"),
        ) else {
            return;
        };
        let age = m.age as f32 + partial;
        sunrise_client::mobs_render::render_model(
            model,
            sunrise_client::mobs::render::base_transform(m, partial, camera),
            &|part| {
                let mut p = part.pose;
                if let Some(i) = part
                    .name
                    .strip_prefix("box")
                    .and_then(|i| i.parse::<u8>().ok())
                {
                    let angle = age * -(45.0 + f32::from(i + 1) * 5.0);
                    p[4] = wrap(angle).to_radians();
                }
                p
            },
            [rect.u0, rect.v0, rect.u1, rect.v1],
            light,
            0,
            out,
        );
    }
}

/// `Lighting.Entry.ENTITY_IN_UI` through the entity shader's
/// `minecraft_mix_light`: the two inventory lights, (0.2, -1, 1) and
/// (-0.2, -1, 0), with the 0.6 light power and 0.4 ambient. `normal` is the
/// face's outward normal in the preview's screen space (x right, y down, z
/// towards the viewer), where a face that looks up has a negative y.
pub(in crate::game::client_side) fn entity_in_ui_shade(normal: glam::Vec3) -> f32 {
    const LIGHT_0: glam::Vec3 = glam::Vec3::new(0.2, -1.0, 1.0);
    const LIGHT_1: glam::Vec3 = glam::Vec3::new(-0.2, -1.0, 0.0);
    let light = LIGHT_0.normalize().dot(normal).max(0.0) + LIGHT_1.normalize().dot(normal).max(0.0);
    (light * 0.6 + 0.4).min(1.0)
}

pub(in crate::game::client_side) fn player_preview_quads(
    verts: Vec<sunrise_client_render::render_geometry::Vertex>,
    gui: glam::Mat4,
    fire: Option<(f32, f32)>,
    sprites: &std::collections::HashMap<String, sunrise_client::hud::Rect>,
) -> Vec<(f32, [[f32; 3]; 4], [[f32; 2]; 4], u32)> {
    player_preview_quads_no_cull(verts, gui, fire, sprites, 0..0)
}

/// `player_preview_quads` where the faces of the vertices in `no_cull` are
/// kept when they look away (a `NoCull` render type); their light still comes
/// from the face's own outward normal, as the vertex normal gives it.
pub(in crate::game::client_side) fn player_preview_quads_no_cull(
    mut verts: Vec<sunrise_client_render::render_geometry::Vertex>,
    gui: glam::Mat4,
    fire: Option<(f32, f32)>,
    sprites: &std::collections::HashMap<String, sunrise_client::hud::Rect>,
    no_cull: std::ops::Range<usize>,
) -> Vec<(f32, [[f32; 3]; 4], [[f32; 2]; 4], u32)> {
    let flame_start = verts.len();
    if let Some((width, height)) = fire {
        sunrise_client::fire_render::flames(
            [0.0; 3],
            width,
            height,
            glam::DVec3::ZERO,
            0.0,
            sprites,
            0xff,
            &mut verts,
        );
    }
    let mut quads: Vec<(f32, [[f32; 3]; 4], [[f32; 2]; 4], u32)> = Vec::new();
    for (index, q) in verts.as_chunks::<4>().0.iter().enumerate() {
        if matches!(
            q[0].layer & 0xffff,
            sunrise_client::gfx::GLINT_ONLY_OVERLAY | sunrise_client::gfx::ARMOR_GLINT_ONLY_OVERLAY
        ) {
            continue;
        }
        let p: [glam::Vec3; 4] =
            std::array::from_fn(|i| gui.transform_point3(glam::Vec3::from_array(q[i].pos)));
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        // The winding gives inward normals: faces with n.z >= 0 point away.
        let back = n.z >= 0.0;
        if back && !no_cull.contains(&(index * 4)) {
            continue;
        }
        let n = n.normalize_or_zero();
        let shade = if index * 4 >= flame_start {
            1.0
        } else {
            entity_in_ui_shade(-n)
        };
        let c = q[0].color;
        let ch = |s: u32| ((((c >> s) & 255) as f32 * shade) as u32) << s;
        let color = (c & 0xFF00_0000) | ch(0) | ch(8) | ch(16);
        let depth = p.iter().map(|v| v.z).sum::<f32>() / 4.0;
        let (mut pos, mut uv) = (p.map(|v| v.to_array()), std::array::from_fn(|i| q[i].uv));
        if back {
            // Seen from behind: the same quad in the opposite winding.
            pos.reverse();
            uv.reverse();
        }
        quads.push((depth, pos, uv, color));
    }
    // Keep far-to-near order for translucent faces; opaque overlap uses depth.
    quads.sort_by(|a, b| a.0.total_cmp(&b.0));
    quads
}

/// One glint face of a GUI model: its corners, the face's atlas UV (the mask),
/// the transformed glint UV, and whether it is the armour's glint.
pub(in crate::game) type PreviewGlint = ([[f32; 3]; 4], [[f32; 2]; 4], [[f32; 2]; 4], bool);

/// `EquipmentLayerRenderer.renderLayers`' glint on a GUI model: the faces of
/// the layers drawn with a glint render type because their stack `hasFoil`,
/// `armorCutoutNoCullGlint` and the trim's `trimmedArmorGlint`
/// (`mobs::render::ARMOR_GLINT`, which `armor_layers` gives a foil piece's
/// trim too; the model UV in `glint_uv`, scrolled here by
/// `ARMOR_ENTITY_GLINT_TEXTURING`), and the items' glint (overlays 7, 8 and
/// 33; `glint_uv` already transformed). Each face is culled and turned as
/// `player_preview_quads_no_cull` does with it.
pub(in crate::game::client_side) fn preview_glint_quads(
    verts: &[sunrise_client::mesher::Vertex],
    gui: glam::Mat4,
    no_cull: std::ops::Range<usize>,
    frame: sunrise_client::item_render::GlintFrame,
) -> Vec<PreviewGlint> {
    let armor_glint = sunrise_client::item_render::Glint {
        speed: frame.speed,
        alpha: frame.alpha,
        millis: frame.millis,
        scale: 0.16,
    };
    let mut out = Vec::new();
    for (index, q) in verts.as_chunks::<4>().0.iter().enumerate() {
        let armor = match q[0].layer & 0xffff {
            sunrise_client::mobs::render::ARMOR_GLINT
            | sunrise_client::gfx::ARMOR_GLINT_ONLY_OVERLAY => true,
            7
            | 8
            | sunrise_client::gfx::TRANSLUCENT_GLINT_ITEM_OVERLAY
            | sunrise_client::gfx::GLINT_ONLY_OVERLAY => false,
            _ => continue,
        };
        let p: [glam::Vec3; 4] =
            std::array::from_fn(|i| gui.transform_point3(glam::Vec3::from_array(q[i].pos)));
        let back = (p[1] - p[0]).cross(p[2] - p[0]).z >= 0.0;
        if back && !no_cull.contains(&(index * 4)) {
            continue;
        }
        let mut pos = p.map(|v| v.to_array());
        let mut uv: [[f32; 2]; 4] = std::array::from_fn(|i| q[i].uv);
        let mut glint_uv: [[f32; 2]; 4] = std::array::from_fn(|i| {
            if armor {
                armor_glint.uv(q[i].glint_uv)
            } else {
                q[i].glint_uv
            }
        });
        if back {
            pos.reverse();
            uv.reverse();
            glint_uv.reverse();
        }
        out.push((pos, uv, glint_uv, armor));
    }
    out
}

/// `GuiEntityRenderer.renderToTexture`: a GUI model into `batch` in the box
/// `clip`, its faces with a depth buffer, then their glint over them in the
/// same pass.
pub(in crate::game::client_side) fn emit_preview(
    batch: &mut sunrise_client::hud::HudBatch,
    quads: Vec<(f32, [[f32; 3]; 4], [[f32; 2]; 4], u32)>,
    glints: Vec<PreviewGlint>,
    clip: [f32; 4],
) {
    let start = batch.vertices.len();
    for (_, p, uv, color) in quads {
        batch.quad_3d(p, uv, color);
    }
    let glint = batch.vertices.len();
    for (p, uv, glint_uv, armor) in glints {
        batch.quad_3d_glint(p, uv, glint_uv, armor);
    }
    batch.depth_glint_since(start, glint, clip);
}

/// Whether `Entity.turn`'s yaw delta (`xo`, 0.15 degrees a unit) leaves both the yaw and `yRotO`
/// finite: arithmetic stays plain for any turn that does, with no angular cap.
fn turn_yaw_representable(yaw: f32, old_yaw: f32, xo: f64) -> bool {
    let delta = (xo as f32) * 0.15;
    (yaw + delta).is_finite() && (old_yaw + delta).is_finite()
}

/// `Entity.turn` adds the turn to `xRotO` as well as to `xRot`, so the local
/// player's `getXRot(partialTick)` follows the mouse every frame instead of
/// easing in over a tick (#3844), while a change the tick made itself (the
/// pitch a sleeper lies at) still eases in. `pitch_o` is the model's old
/// pitch; without a model the old pitch is the pitch.
pub(in crate::game::client_side) fn turn_rotation(
    yaw: f32,
    pitch: f32,
    pitch_o: Option<f32>,
    xo: f64,
    yo: f64,
) -> Rotation {
    let mut rotation = Rotation {
        x_rot: pitch,
        y_rot: yaw,
        x_rot_o: pitch_o.unwrap_or(pitch),
        y_rot_o: yaw,
    };
    rotation.turn(xo, yo);
    rotation
}

/// `Entity.turn` adds the turn to `yRotO` and `xRotO` as well, so the look the
/// held lead's end eases from (`LocalPlayer.getRopeHoldPosition`'s
/// `Mth.lerp(partialTick * 0.5F, getYRot(), yRotO)`) follows the mouse at once;
/// only what the tick itself turned (a boat's `deltaRotation`) eases in. `old`
/// is `(yRotO, xRotO)`; the result is the pair after the turn (#10276).
pub(in crate::game::client_side) fn rope_old_after_turn(
    old: (f32, f32),
    yaw: f32,
    pitch: f32,
    xo: f64,
    yo: f64,
) -> (f32, f32) {
    let mut rotation = Rotation {
        x_rot: pitch,
        y_rot: yaw,
        x_rot_o: old.1,
        y_rot_o: old.0,
    };
    rotation.turn(xo, yo);
    (rotation.y_rot_o, rotation.x_rot_o)
}

/// `EntityRenderer.getPackedLightCoords` (`Entity.getLightProbePosition`):
/// the block containing the entity's eye position, undoing
/// `AvatarRenderer.getRenderOffset`'s 0.125 crouch drop on the model position.
pub(in crate::game::client_side) fn entity_light_block(
    position: Vec3,
    crouching: bool,
    eye_height: f64,
) -> [i32; 3] {
    let y = position.y + if crouching { 0.125 } else { 0.0 } + eye_height;
    [
        position.x.floor() as i32,
        y.floor() as i32,
        position.z.floor() as i32,
    ]
}

/// The player's model's old and new positions for a tick: the player's old
/// (`Entity.xo`, `yo`, `zo`) and new positions, each 0.125 lower while
/// crouching. The model is the player itself in vanilla, so
/// `EntityRenderer.extractRenderState` lerps it from the same old position
/// `Camera.setup` does, however far it moved: at Levitation 255 (12.75 blocks
/// a tick) the body keeps up with the camera. A teleport or respawn resets
/// the old position, and the model jumps with it.
pub(in crate::game::client_side) fn player_model_positions(
    previous_position: Vec3,
    position: Vec3,
    was_crouching: bool,
    crouching: bool,
) -> (Vec3, Vec3) {
    let drop =
        |p: Vec3, crouching: bool| Vec3::new(p.x, p.y - if crouching { 0.125 } else { 0.0 }, p.z);
    (
        drop(previous_position, was_crouching),
        drop(position, crouching),
    )
}

/// The distance `LivingEntity.calculateEntityAnimation` walks the player's
/// model for a move from `from` to `to`. `omnidirectionalAirMover()` is false
/// for every player (`Entity`'s, not overridden), so only the horizontal
/// distance counts, flying and gliding included: rising or sinking straight
/// up or down leaves the legs and arms still.
pub(in crate::game::client_side) fn player_walk_distance(from: Vec3, to: Vec3) -> f32 {
    let (dx, dz) = (to.x - from.x, to.z - from.z);
    (dx * dx + dz * dz).sqrt() as f32
}

/// Keeps a listener-edited `Entity.turn` axis delta when vanilla's `(delta as f32) * 0.15`
/// cast produces a finite turn; logs and reverts that axis otherwise.
fn finite_turn_delta(axis: &'static str, original: f64, proposed: f64) -> f64 {
    if ((proposed as f32) * 0.15).is_finite() {
        proposed
    } else {
        log::warn!("player_turn: refused non-finite {axis} delta ({proposed}); keeping {original}");
        original
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod preview_tests {
    use super::*;

    #[test]
    fn finite_turn_delta_keeps_finite_values_and_reverts_non_finite_ones() {
        assert_eq!(finite_turn_delta("x", 10.0, 2.5), 2.5);
        assert_eq!(finite_turn_delta("y", -4.0, 0.0), 0.0);
        assert_eq!(finite_turn_delta("x", 10.0, -3600.0), -3600.0);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300, -1e300] {
            assert_eq!(finite_turn_delta("x", 10.0, bad), 10.0, "{bad}");
        }
    }
    #[test]
    fn a_spectators_own_model_stays_out_of_a_shader_packs_shadow_pass() {
        // First person, a shader pack whose shadow pass draws the player: a spectator's head
        // isn't in Iris's map (#8464); the other game modes keep the model, as before.
        assert!(!shadow_pass_takes_own_model(true, true, true));
        assert!(shadow_pass_takes_own_model(true, true, false));
        // A native pack's shadow keeps the first-person model, a spectator's too.
        assert!(shadow_pass_takes_own_model(true, false, true));
        // No shadow pass that draws the player: nothing to take.
        assert!(!shadow_pass_takes_own_model(false, true, false));
    }
    #[test]
    fn the_player_model_rises_with_the_camera_at_levitation_255() {
        // LivingEntity.travel at Levitation 255: 0.05 * 255 blocks a tick.
        let from = Vec3::new(0.5, 80.0, 0.5);
        let to = Vec3::new(0.5, 92.75, 0.5);
        // The camera lerps from the old position: so does the body.
        assert_eq!(
            player_model_positions(from, to, false, false),
            (from, to),
            "the body jumps instead of rising (#3616)"
        );
        // Any distance, crouching on both ends.
        let far = Vec3::new(40.5, 80.0, 0.5);
        assert_eq!(
            player_model_positions(from, far, true, true),
            (Vec3::new(0.5, 79.875, 0.5), Vec3::new(40.5, 79.875, 0.5)),
        );
        // A teleport resets the old position to the new one: no sweep.
        assert_eq!(player_model_positions(to, to, false, false), (to, to));
    }
    #[test]
    fn flying_straight_up_does_not_walk_the_player_model() {
        let from = Vec3::new(0.5, 80.0, 0.5);
        // Creative flight straight up (#3302) and straight down.
        assert_eq!(player_walk_distance(from, Vec3::new(0.5, 80.375, 0.5)), 0.0);
        assert_eq!(player_walk_distance(from, Vec3::new(0.5, 78.5, 0.5)), 0.0);
        // Moving sideways while rising walks only the horizontal part.
        let d = player_walk_distance(from, Vec3::new(0.8, 81.0, 0.9));
        assert!((d - 0.5).abs() < 1e-6, "{d}");
    }
    /// #3844: `getXRot(partialTick)` after `Entity.turn`, as the model's
    /// renderer lerps it, with turns before and after the tick.
    #[test]
    fn own_head_pitch_follows_the_mouse_between_ticks() {
        // (pitch, the model's previous head pitch, its head pitch), as
        // `turn_player` and `tick_player_model` move them.
        let turn = |(pitch, previous, _): (f32, f32, f32), degrees: f32| {
            let r = turn_rotation(0.0, pitch, Some(previous), 0.0, f64::from(degrees / 0.15));
            (r.x_rot, r.x_rot_o, r.x_rot)
        };
        let tick = |(pitch, _, head): (f32, f32, f32)| (pitch, head, pitch);
        let x_rot =
            |(_, previous, head): (f32, f32, f32), alpha: f32| previous + (head - previous) * alpha;
        let at = |state, pitch: f32| {
            for alpha in [0.0, 0.5, 1.0] {
                let x = x_rot(state, alpha);
                assert!((x - pitch).abs() < 1e-3, "partial tick {alpha}: {x}");
            }
        };
        // The arrow keys turn before the frame's ticks: the tick then copies
        // ends that are already in step.
        let still = (10.0, 10.0, 10.0);
        let state = tick(turn(still, 30.0));
        at(state, 40.0);
        // The mouse turns after the tick: the head is there at once.
        let state = turn(state, -30.0);
        at(state, 10.0);
        // The tick laid the head down from 30 to 0: the turn moves both
        // ends, and the tick's change still eases in.
        let state = turn((0.0, 30.0, 0.0), 10.0);
        assert!((x_rot(state, 0.5) - 25.0).abs() < 1e-3, "{state:?}");
        // xRotO is clamped as xRot is.
        let r = turn_rotation(0.0, 60.0, Some(80.0), 0.0, 200.0);
        assert_eq!((r.x_rot_o, r.x_rot), (90.0, 90.0));
        // Without a model the old pitch is the pitch.
        let r = turn_rotation(0.0, 10.0, None, 0.0, 100.0);
        assert_eq!(r.x_rot_o, r.x_rot);
    }
    /// #10276: the lead held in first person eases from `yRotO`/`xRotO`, which
    /// `Entity.turn` moves with the view: a mouse turn leaves no gap for the
    /// hand to lag by, while a boat's turn (not a `turn`) still eases in.
    #[test]
    fn the_held_leads_old_look_turns_with_the_mouse() {
        // A tick's snapshot, then a 30 degree mouse turn right and 15 down.
        let (yaw, pitch) = (40.0_f32, 10.0_f32);
        let old = (yaw, pitch);
        let r = turn_rotation(yaw, pitch, None, 200.0, 100.0);
        let old = rope_old_after_turn(old, yaw, pitch, 200.0, 100.0);
        assert_eq!(old, (r.y_rot, r.x_rot), "the old look moved by the turn");
        // The boat turned the view by 5 after the snapshot: that part eases.
        let old = rope_old_after_turn((yaw, pitch), yaw + 5.0, pitch, 200.0, 0.0);
        assert!((old.0 - (yaw + 30.0)).abs() < 1e-4, "{old:?}");
        // xRotO is clamped as xRot is.
        let old = rope_old_after_turn((0.0, 85.0), 0.0, 85.0, 0.0, 100.0);
        assert_eq!(old.1, 90.0);
    }
    /// Individually finite turns can overflow the old yaw while the current yaw still takes
    /// them: such a turn is refused whole, and every turn that fits is plain arithmetic.
    #[test]
    fn a_turn_that_overflows_the_old_yaw_is_refused_whole() {
        let big = f64::from(f32::MAX);
        assert!(turn_yaw_representable(0.0, 0.0, big));
        // The current yaw takes a second turn of the same size; the old yaw, already moved by
        // one, would overflow.
        let yaw = f32::MAX * 0.5;
        let old = f32::MAX * 0.9;
        assert!(!turn_yaw_representable(yaw, old, big));
        assert!(turn_yaw_representable(yaw, old, 100.0));
        assert!(turn_yaw_representable(yaw, old, -big));
    }
    #[test]
    fn preview_keeps_crossing_faces_depth_and_clips_without_dropping_faces() {
        use sunrise_client::{
            hud::{Blend, HudBatch},
            mesher::Vertex,
        };
        let face = |z: [f32; 4]| {
            [[-2.0, -1.0], [-2.0, 1.0], [2.0, 1.0], [2.0, -1.0]]
                .into_iter()
                .zip(z)
                .map(|([x, y], z)| Vertex {
                    pos: [x, y, z],
                    uv: [0.0; 2],
                    layer: 0,
                    color: u32::MAX,
                    light: 0,
                    normal: [0; 4],

                    glint_uv: [0.0, 0.0],
                })
        };
        // A shield and arm can intersect: neither average depth orders both ends.
        let verts = face([1.0, 1.0, -1.0, -1.0]).chain(face([0.0; 4])).collect();
        let quads = player_preview_quads(verts, glam::Mat4::IDENTITY, None, &Default::default());
        assert_eq!(quads.len(), 2);
        let mut batch = HudBatch::scratch([320, 240]);
        for (_, p, uv, color) in quads {
            batch.quad_3d(p, uv, color);
        }
        // Its centre is outside this scissor; the visible part must still be drawn.
        batch.depth_since(0, [1.0, 0.0, 2.0, 1.0]);
        assert_eq!(batch.vertices.len(), 12);
        let shield = &batch.vertices[..6];
        let arm = &batch.vertices[6..];
        assert!(shield[0].depth < arm[0].depth);
        assert!(shield[2].depth > arm[2].depth);
        assert_eq!(batch.runs(), vec![(0..12, Blend::Depth([1, 0, 1, 1]))]);
    }

    #[test]
    fn no_cull_faces_stay_in_the_preview_when_they_look_away() {
        use sunrise_client_render::render_geometry::Vertex;
        // The wings' `armorCutoutNoCull`: a face turned from the viewer is
        // dropped by the preview's culling unless its vertices are no-cull.
        let face = |z: f32| {
            [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]].map(|[x, y]| Vertex {
                pos: [x, y, z],
                uv: [x, y],
                layer: 0,
                color: u32::MAX,
                light: 0,
                normal: [0; 4],
                glint_uv: [0.0, 0.0],
            })
        };
        // Wound one way a face looks at the viewer, the other way it looks away.
        let mut away = face(0.0);
        away.reverse();
        let verts: Vec<_> = face(0.0).into_iter().chain(away).collect();
        let sprites = Default::default();
        let gui = glam::Mat4::IDENTITY;
        let culled = player_preview_quads(verts.clone(), gui, None, &sprites);
        assert_eq!(culled.len(), 1);
        let kept = player_preview_quads_no_cull(verts.clone(), gui, None, &sprites, 4..8);
        assert_eq!(kept.len(), 2);
        // The first face is outside the range and still culled when it looks away.
        let first_away = player_preview_quads_no_cull(verts, gui, None, &sprites, 0..0);
        assert_eq!(first_away.len(), 1);
    }

    /// #12666: enchanted armour glints on the inventory's player as in the
    /// world: `armorCutoutNoCullGlint` layers and foil items get a glint face.
    #[test]
    fn the_preview_glints_its_enchanted_armour_and_items() {
        use sunrise_client::{item_render::GlintFrame, mesher::Vertex, mobs::render::ARMOR_GLINT};
        let face = |layer: u32, glint_uv: [f32; 2]| {
            [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]].map(|[x, y]| Vertex {
                pos: [x, y, 0.0],
                uv: [0.5 + x / 4.0, 0.5 + y / 4.0],
                layer,
                color: u32::MAX,
                light: 0,
                normal: [0; 4],
                glint_uv,
            })
        };
        let mut away = face(ARMOR_GLINT, [0.25, 0.75]);
        away.reverse();
        let verts: Vec<_> = [
            face(0, [0.0; 2]),
            face(ARMOR_GLINT, [0.25, 0.75]),
            face(7, [3.0, 4.0]),
            // The upper bits carry the HDR tier, not the overlay.
            face(ARMOR_GLINT | 1 << 16, [0.25, 0.75]),
            away,
        ]
        .into_iter()
        .flatten()
        .collect();
        let frame = GlintFrame {
            millis: 1_234,
            speed: 0.5,
            alpha: 0.75,
            sprite: true,
        };
        let gui = glam::Mat4::IDENTITY;
        let glints = preview_glint_quads(&verts, gui, 0..0, frame);
        // The plain face has none; the armour face looking away is culled.
        assert_eq!(glints.len(), 3);
        let armor = sunrise_client::item_render::Glint {
            speed: 0.5,
            alpha: 0.75,
            millis: 1_234,
            scale: 0.16,
        };
        let (pos, uv, glint_uv, is_armor) = glints[0];
        assert!(is_armor);
        assert_eq!(pos, std::array::from_fn(|i| verts[4 + i].pos));
        assert_eq!(uv[0], verts[4].uv);
        // ARMOR_ENTITY_GLINT_TEXTURING scrolls the model UV.
        assert_eq!(glint_uv[0], armor.uv([0.25, 0.75]));
        // The item's glint UV is already transformed.
        assert_eq!((glints[1].2[0], glints[1].3), ([3.0, 4.0], false));
        assert!(glints[2].3);
        // A no-cull (wings) face that looks away keeps its glint, turned.
        let kept = preview_glint_quads(&verts, gui, 16..20, frame);
        assert_eq!(kept.len(), 4);
        assert_eq!(kept[3].0[0], verts[16 + 3].pos);
    }

    #[test]
    fn the_inventory_preview_is_lit_as_entity_in_ui() {
        // A face looking at the viewer, up, down, to the right and to the left.
        let shade = |x, y, z| entity_in_ui_shade(glam::Vec3::new(x, y, z));
        assert!((shade(0.0, 0.0, 1.0) - 0.8195).abs() < 1e-3);
        assert_eq!(shade(0.0, -1.0, 0.0), 1.0);
        assert!((shade(0.0, 1.0, 0.0) - 0.4).abs() < 1e-6);
        assert!((shade(1.0, 0.0, 0.0) - 0.4839).abs() < 1e-3);
        assert!((shade(-1.0, 0.0, 0.0) - 0.5176).abs() < 1e-3);
        // Away from the viewer: only the ambient light.
        assert!((shade(0.0, 0.0, -1.0) - 0.4).abs() < 1e-6);
    }

    #[test]
    #[ignore = "daily"]
    fn player_render_transform_uses_vanilla_avatar_scale() {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("SUNRISE_TEST_RESOURCES"),
        );
        let mut m = Mob::new(1, player_type(&root).unwrap(), Vec3::ZERO, 0.0);
        for (id, expected) in [("minecraft:player", 0.9375), ("minecraft:zombie", 1.0)] {
            Arc::make_mut(&mut m.kind).id = id.into();
            let transform =
                sunrise_client::mobs::render::base_transform_at(&m, 1.0, glam::Mat4::IDENTITY);
            for axis in [glam::Vec3::X, glam::Vec3::Y, glam::Vec3::Z] {
                assert!((transform.transform_vector3(axis).length() - expected).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn an_invisible_player_hides_its_own_body() {
        use sunrise_game_data::scoreboard::Scoreboard;
        use sunrise_level::effects::EffectInstance;
        // The effect as a server's UpdateMobEffect or a drunk potion adds it.
        let id = sunrise_level::brewing::effect("minecraft:invisibility")
            .unwrap()
            .id;
        let mut effects = sunrise_level::effects::PlayerEffects::default();
        assert!(body_visible(own_invisible(&effects, false), None));
        effects.add(EffectInstance::new(id, 3600, 0));
        let invisible = own_invisible(&effects, false);
        assert!(
            !body_visible(invisible, None),
            "no team: the body is hidden"
        );
        // Entity.isInvisibleTo: a team that sees friendly invisibles sees it.
        let mut board = Scoreboard::default();
        let team = board.add_team("blue");
        assert!(body_visible(invisible, Some(team)));
        team.see_friendly_invisibles = false;
        assert!(!body_visible(invisible, Some(team)));
    }

    /// #3788: a pose plugin's `Player.setInvisible(true)` sets only the
    /// shared flag (no effect); `Entity.isInvisible` reads that flag, so the
    /// real body under the pose is not drawn.
    #[test]
    fn the_servers_invisible_flag_hides_the_own_body_without_the_effect() {
        let effects = sunrise_level::effects::PlayerEffects::default();
        assert!(own_invisible(&effects, true));
        assert!(!body_visible(own_invisible(&effects, true), None));
        let mut board = sunrise_game_data::scoreboard::Scoreboard::default();
        let team = board.add_team("blue");
        assert!(body_visible(own_invisible(&effects, true), Some(team)));
    }

    /// LivingEntityRenderer.submit: an invisible body is not drawn, its
    /// HumanoidArmorLayer is.
    #[test]
    fn an_invisible_body_keeps_its_armour() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client/26.3");
        let Some(kind) = player_type(&root) else {
            return;
        };
        let m = Mob::new(1, kind.clone(), Vec3::ZERO, 0.0);
        let models = sunrise_client::mobs_render::ClientMobModels::load(&root).unwrap();
        let armour = [(
            sunrise_client::mobs::render::WornModel::Vanilla(
                models.model(&kind.id).expect("missing player visual"),
            ),
            [0.5, 0.5, 0.75, 0.75],
            u32::MAX,
            0,
            false,
        )];
        let atlas = |_: &str| Some([0.0, 0.0, 0.25, 0.25]);
        let data = sunrise_client::mobs_render::ClientMobData::default();
        let source = sunrise_client::mobs_render::ClientMobRender {
            models: &models,
            data: &data,
        };
        let draw = |body: bool| {
            let mut out = Vec::new();
            let at = glam::DVec3::ZERO;
            if body {
                sunrise_client::mobs::render::render_with(
                    &m, 1.0, at, &atlas, 0xF0F0, &source, &armour, &mut out,
                );
            } else {
                sunrise_client::mobs::render::render_layers(
                    &m,
                    1.0,
                    at,
                    &atlas,
                    0xF0F0,
                    &source,
                    &armour,
                    &mut FrameDraw::default(),
                    &mut out,
                );
            }
            out
        };
        let (shown, hidden) = (draw(true), draw(false));
        assert!(shown.iter().any(|v| v.uv[0] <= 0.25), "the body draws");
        assert!(!hidden.is_empty(), "the armour still draws");
        assert!(hidden.iter().all(|v| v.uv[0] >= 0.5), "only the armour");
        assert_eq!(hidden.len() * 2, shown.len());
        // AvatarRenderer.shouldRenderLayers: a spectator has no armour layer.
        let mut ghost = m.clone();
        ghost.extra.spectator = true;
        assert!(!sunrise_client::mobs::render::layers_shown(&ghost));
        let mut with_armour = Vec::new();
        let mut without = Vec::new();
        for (armour, out) in [(&armour[..], &mut with_armour), (&[][..], &mut without)] {
            sunrise_client::mobs::render::render_layers(
                &ghost,
                1.0,
                glam::DVec3::ZERO,
                &atlas,
                0xF0F0,
                &source,
                armour,
                &mut FrameDraw::default(),
                out,
            );
        }
        assert!(without.is_empty() && with_armour.is_empty(), "no armour");
    }

    #[test]
    fn burning_inventory_preview_keeps_visible_fullbright_flames() {
        let rect = |u0| sunrise_client::hud::Rect {
            u0,
            v0: 0.0,
            u1: u0 + 0.1,
            v1: 0.1,
            w: 16,
            h: 16,
        };
        let sprites = std::collections::HashMap::from([
            ("block/fire_0".into(), rect(0.0)),
            ("block/fire_1".into(), rect(0.5)),
        ]);
        for pitch in [-30.0f32, 0.0, 30.0] {
            let gui = glam::Mat4::from_scale(glam::Vec3::new(30.0, 30.0, -30.0))
                * glam::Mat4::from_rotation_z(std::f32::consts::PI)
                * glam::Mat4::from_rotation_x(pitch.to_radians());
            assert!(player_preview_quads(Vec::new(), gui, None, &sprites).is_empty());
            let quads = player_preview_quads(Vec::new(), gui, Some((0.6, 1.8)), &sprites);
            assert!(
                !quads.is_empty(),
                "inventory projection must not cull all flames"
            );
            assert!(quads.iter().all(|q| q.3 == 0xffff_ffff));
            assert!(quads.iter().any(|q| q.2[0][0] <= 0.1));
            assert!(quads.iter().any(|q| q.2[0][0] >= 0.5));
            assert!(quads.windows(2).all(|q| q[0].0 <= q[1].0));
        }
    }

    #[test]
    fn sleeping_player_light_probe_stays_below_a_two_block_ceiling() {
        // In a 2-block-high room (bed at y = 64, air at 65, solid ceiling at 66),
        // setPosToBed places the sleeper at y = 64 + 0.5625 + 0.125 = 64.6875.
        let in_bed = Vec3::new(10.5, 64.0 + 0.5625 + 0.125, -4.5);
        let sleeping_eye = f64::from(Pose::Sleeping.dimensions(1.0).eye_height);
        assert_eq!(
            entity_light_block(in_bed, false, sleeping_eye),
            [10, 64, -5]
        );
        // Using the standing eye height (1.62) while in bed probed y = 66,
        // inside the ceiling block where light is 0 (#5121).
        let standing_eye = f64::from(Pose::Standing.dimensions(1.0).eye_height);
        assert_eq!(
            entity_light_block(in_bed, false, standing_eye),
            [10, 66, -5]
        );
        // A crouching player on a 7-layer snow block (feet y = 65.75, model y
        // dropped by 0.125, crouch eye 1.27) still probes at real y + eye = 67.02.
        let (_, crouch_model) = player_model_positions(
            Vec3::new(10.5, 65.75, -4.5),
            Vec3::new(10.5, 65.75, -4.5),
            true,
            true,
        );
        let crouch_eye = f64::from(Pose::Crouching.dimensions(1.0).eye_height);
        assert_eq!(
            entity_light_block(crouch_model, true, crouch_eye),
            [10, 67, -5]
        );
    }

    /// `LivingEntityRenderer.solveBodyRot` worked by hand: the head turns at
    /// most 85 degrees from the vehicle's body, and past 50 the body follows a
    /// fifth of the turn.
    #[test]
    fn a_living_vehicles_rider_faces_its_body_within_85_degrees() {
        let body = |head, vehicle| {
            (sunrise_sim::mobs::riders::solve_body_rot(head, vehicle) * 1000.0).round() / 1000.0
        };
        assert_eq!(body(60.0, 40.0), 40.0);
        assert_eq!(body(100.0, 40.0), 52.0);
        assert_eq!(body(140.0, 40.0), 72.0);
        assert_eq!(body(-60.0, 40.0), 8.0);
        // Across the wrap: head at 170, vehicle at -170 (20 degrees apart).
        assert_eq!(body(170.0, -170.0), 190.0);
    }
}
