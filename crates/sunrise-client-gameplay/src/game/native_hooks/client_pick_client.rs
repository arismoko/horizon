//! The one stored pick, refreshed before tick inputs and frame extraction.
use super::*;
use mod_api::CrosshairTarget;
use sunrise_gameplay_common::game::view_vector;
use sunrise_gameplay_common::game::{Dim, commands_common::Target};

#[derive(Default)]
pub struct ClientPick {
    pub ray: Option<([f64; 3], [f64; 3])>,
    pub(in crate::game) epoch: Option<mod_api::WorldEpoch>,
    pub(in crate::game) dimension: Option<Dim>,
    pub(in crate::game) origin: [f64; 3],
    pub(in crate::game) entity_reach: f64,
    pub(in crate::game) result: Option<CrosshairTarget>,
    pub entity_metadata: (bool, bool),
    pub rider_candidates: Vec<sunrise_gameplay_common::game::pick_ray::PickCandidate>,
    #[cfg(any(test, feature = "test-support"))]
    pub(in crate::game) test_forced_hit: Option<sunrise_gameplay_common::game::Hit>,
    pub(in crate::game) sections:
        std::cell::RefCell<crate::game::pick_entity::pick_sections::PickSectionStorage>,
    pub(in crate::game) dispatching: bool,
    /// How many picks have been taken.
    pub(in crate::game) taken: u64,
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// A window always has a decision, including before the first pick (a miss).
    /// None in the outer option means a server ray or the current pick computation.
    pub(in crate::game) fn cached_entity_pick(&self) -> Option<Option<(EntityRef, f64, [f64; 3])>> {
        self.side_uses_stored_pick()
            .then(|| match self.crosshair_target() {
                Some(CrosshairTarget::Entity { entity, point }) => {
                    Some((entity, self.pick_distance(point), point))
                }
                _ => None,
            })
    }

    pub(in crate::game) fn pick_target_live(&self, result: CrosshairTarget) -> bool {
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        match result {
            CrosshairTarget::Block { pos, state, .. } => {
                self.common
                    .world
                    .block(pos.x, pos.y, pos.z)
                    .try_into_api()
                    .ok()
                    == Some(state)
            }
            CrosshairTarget::Entity { entity, .. } => {
                self.native_client_entity_target(entity).is_some()
            }
            _ => false,
        }
    }

    pub(in crate::game) fn ask_can_rider_interact(
        &mut self,
        subject: mod_api::EntityRef,
        allowed: &mut bool,
    ) {
        let mut access = ClientPickEntityAccess {
            game: self,
            subject,
        };
        native_can_rider_interact(mod_api::__private::CanRiderInteractCall {
            subject: mod_api::EntityClientScope::new(&mut access),
            allowed,
        });
    }

    pub(in crate::game) fn native_client_pick(
        &mut self,
        call: mod_api::__private::ClientPickCall<'_>,
    ) {
        self.fire_client_pick(call);
    }
}

pub(in crate::game) fn native_can_rider_interact(
    call: mod_api::__private::CanRiderInteractCall<'_>,
) {
    crate::game::native_hooks::fire::fire_can_rider_interact(call);
}

pub(in crate::game) struct ResidentClientPickEntity {
    pub(in crate::game) kind: Option<mod_api::EntityTypeKey>,
    pub(in crate::game) position: [f64; 3],
    pub(in crate::game) rotation: [f32; 2],
    pub(in crate::game) uuid: Option<u128>,
}

impl mod_api::ResidentClientEntity for ResidentClientPickEntity {
    fn kind(&self) -> Option<mod_api::EntityTypeKey> {
        self.kind
    }
    fn position(&self) -> [f64; 3] {
        self.position
    }
    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn uuid(&self) -> Option<u128> {
        self.uuid
    }
}

pub(in crate::game) struct ClientPickEntityAccess<'g, 'w> {
    pub(in crate::game) game: &'g mut crate::game::ClientGame<'w>,
    pub(in crate::game) subject: mod_api::EntityRef,
}

impl mod_api::EntityClientAccess for ClientPickEntityAccess<'_, '_> {
    fn route_kind(&self) -> Option<mod_api::EntityTypeKey> {
        self.game.client_pick_route_kind(self.subject)
    }

    fn with_view(&mut self, call: &mut dyn FnMut(mod_api::EntityClientView<'_>)) {
        let Some(data) = self.game.client_pick_entity_view(self.subject) else {
            return;
        };
        call(mod_api::EntityClientView::new(self.subject, &data));
    }

    fn host(&mut self) -> &mut dyn mod_api::ClientHost {
        self.game
    }
}
impl crate::game::ClientGame<'_> {
    /// The sole mod-output geometry boundary. Occlusion is intentionally not reapplied.
    pub(crate) fn accept_client_pick(&self, result: CrosshairTarget) -> bool {
        let (point, reach) = match result {
            CrosshairTarget::Block { hit, .. } => {
                if !matches!(
                    hit.face,
                    [1, 0, 0] | [-1, 0, 0] | [0, 1, 0] | [0, -1, 0] | [0, 0, 1] | [0, 0, -1]
                ) {
                    return false;
                }
                (hit.point, self.common.block_reach)
            }
            CrosshairTarget::Entity { point, .. } => (point, self.client.client_pick.entity_reach),
            _ => return false,
        };
        point.iter().all(|v| v.is_finite())
            && self.pick_distance(point) <= reach
            && self.pick_target_live(result)
    }
    /// How many picks (a tick's or a frame's) have been taken: an agent command that turned
    /// waits for the count to move before it reads the target.
    pub(crate) fn client_picks(&self) -> u64 {
        self.client.client_pick.taken
    }
    pub fn pick_distance(&self, point: [f64; 3]) -> f64 {
        (0..3)
            .map(|i| (point[i] - self.client.client_pick.origin[i]).powi(2))
            .sum::<f64>()
            .sqrt()
    }
    pub(crate) fn publish_client_pick(&mut self, result: Option<CrosshairTarget>) {
        self.client.client_pick.result = result;
        self.client.client_pick.entity_metadata = match result {
            Some(CrosshairTarget::Entity { entity, .. }) => (
                !self.client.interactions.contains_key(&(entity.id() as i32))
                    && sunrise_gameplay_common::game::pick_entity_common::has_pick_result(
                        self.native_client_entity_target(entity),
                    ),
                self.common.dragon.as_ref().is_none_or(|d| {
                    sunrise_gameplay_common::game::dragon_common::dragon_part_index(
                        d.id,
                        entity.id(),
                    )
                    .is_none()
                }),
            ),
            _ => (false, false),
        };
    }

    fn client_target_kind_name(&self, target: Target) -> Option<&str> {
        Some(match target {
            Target::Player | Target::Guest(_) => {
                sunrise_gameplay_common::game::native_living_body::PLAYER
            }
            Target::Mob(id) => sunrise_gameplay_common::game::native_living_body::body_kind_name(
                sunrise_sim::health::LivingBody::Mob(self.common.mobs.iter().find(|m| m.id == id)?),
            ),
            Target::Item(_) => "minecraft:item",
            Target::Orb(_) => "minecraft:experience_orb",
            Target::Projectile(id) => self
                .common
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)?
                .entity_type(),
            Target::Stand(_) => sunrise_gameplay_common::game::native_living_body::ARMOR_STAND,
            Target::General(id) => self
                .client
                .remote
                .as_ref()?
                .general
                .rows
                .get(&(id as i32))?
                .kind(),
            Target::Frame(id) => {
                if self.common.item_frames.iter().any(|f| f.id == id && f.glow) {
                    "minecraft:glow_item_frame"
                } else {
                    "minecraft:item_frame"
                }
            }
            Target::Painting(_) => "minecraft:painting",
            Target::Boat(id) => self
                .common
                .boats
                .iter()
                .find(|b| b.id == id)?
                .entity_type_name(),
            Target::Cart(id) => self.common.carts.iter().find(|c| c.id == id)?.kind.id(),
            Target::Other(id) => {
                if self.client.interactions.contains_key(&(id as i32)) {
                    "minecraft:interaction"
                } else {
                    self.client.remote.as_ref()?.tracked(id as i32)?.0
                }
            }
            Target::Crystal(_) => "minecraft:end_crystal",
            Target::Dragon(_) | Target::DragonPart(_) => {
                sunrise_gameplay_common::game::native_living_body::ENDER_DRAGON
            }
            Target::Tnt(_) => "minecraft:tnt",
            Target::Cushion(_) => "minecraft:cushion",
            Target::Falling(_) => "minecraft:falling_block",
            Target::Rocket(_) => "minecraft:firework_rocket",
            Target::Trident(_) => "minecraft:trident",
            Target::Fangs(_) => "minecraft:evoker_fangs",
            Target::Spawner(_) => "minecraft:ominous_item_spawner",
            Target::Cloud(_) => "minecraft:area_effect_cloud",
            Target::Bolt(_) => "minecraft:lightning_bolt",
            Target::Hook(_) => "minecraft:fishing_bobber",
            Target::Knot(_) => "minecraft:leash_knot",
        })
    }

    fn client_target_kind_key(&self, target: Target) -> Option<mod_api::EntityTypeKey> {
        let keys = self.common.res.living_type_keys();
        match target {
            Target::Player | Target::Guest(_) => keys.player,
            Target::Mob(id) => self.common.mobs.iter().find(|m| m.id == id)?.kind.type_key,
            Target::Stand(_) => keys.armor_stand,
            Target::Dragon(_) | Target::DragonPart(_) => keys.ender_dragon,
            target => {
                mod_api::__private::resolved_entity_type_key(self.client_target_kind_name(target)?)
            }
        }
    }

    fn client_target_position(&self, target: Target) -> Option<[f64; 3]> {
        match target {
            Target::Player => self
                .common
                .player
                .as_ref()
                .map(|p| [p.position.x, p.position.y, p.position.z]),
            Target::Guest(_) => None,
            Target::Mob(id) => self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| [m.position.x, m.position.y, m.position.z]),
            Target::Item(id) => self
                .common
                .dropped
                .iter()
                .find(|i| i.id == id)
                .map(|i| [i.position.x, i.position.y, i.position.z]),
            Target::Orb(id) => self
                .common
                .orbs
                .iter()
                .find(|o| o.id == id)
                .map(|o| [o.position.x, o.position.y, o.position.z]),
            Target::Projectile(id) => self
                .common
                .creatures
                .projectiles
                .iter()
                .find(|a| a.id == id)
                .map(|a| [a.position.x, a.position.y, a.position.z]),
            Target::Stand(id) => self
                .common
                .armor_stands
                .iter()
                .find(|a| a.id == id)
                .map(|a| [a.position.x, a.position.y, a.position.z]),
            Target::Frame(id) => self
                .common
                .item_frames
                .iter()
                .find(|f| f.id == id)
                .map(|f| {
                    let (lo, hi) = f.bounds();
                    [
                        (lo[0] + hi[0]) / 2.0,
                        (lo[1] + hi[1]) / 2.0,
                        (lo[2] + hi[2]) / 2.0,
                    ]
                }),
            Target::Painting(id) => self.common.paintings.iter().find(|p| p.id == id).map(|p| {
                let (lo, hi) = p.bounds();
                [
                    (lo[0] + hi[0]) / 2.0,
                    (lo[1] + hi[1]) / 2.0,
                    (lo[2] + hi[2]) / 2.0,
                ]
            }),
            Target::Boat(id) => self
                .common
                .boats
                .iter()
                .find(|b| b.id == id)
                .map(|b| [b.position.x, b.position.y, b.position.z]),
            Target::Cart(id) => self
                .common
                .carts
                .iter()
                .find(|c| c.id == id)
                .map(|c| [c.position.x, c.position.y, c.position.z]),
            Target::Other(id) => self
                .client
                .interactions
                .get(&(id as i32))
                .map(|e| [e.position.x, e.position.y, e.position.z])
                .or_else(|| {
                    self.client
                        .remote
                        .as_ref()?
                        .tracked(id as i32)
                        .map(|(_, p)| p)
                }),
            Target::Crystal(id) => self
                .common
                .crystals
                .iter()
                .find(|c| c.id == id)
                .map(|c| [c.position.x, c.position.y, c.position.z]),
            Target::Dragon(id) => self
                .common
                .dragon
                .as_ref()
                .filter(|d| d.id == id)
                .map(|d| [d.position.x, d.position.y, d.position.z]),
            Target::Cushion(id) => self
                .common
                .cushions
                .iter()
                .find(|c| c.id == id)
                .map(|c| [c.position.x, c.position.y, c.position.z]),
            Target::DragonPart(id) => self.common.dragon.as_ref().and_then(|d| {
                let i = sunrise_gameplay_common::game::dragon_common::dragon_part_index(d.id, id)?;
                let p = d.parts[i].0;
                Some([p.x, p.y, p.z])
            }),
            Target::Tnt(id) => self
                .common
                .tnts
                .iter()
                .find(|t| t.id == id)
                .map(|t| [t.position.x, t.position.y, t.position.z]),
            Target::Falling(id) => self
                .common
                .falling
                .iter()
                .find(|f| f.id == id)
                .map(|f| [f.position.x, f.position.y, f.position.z]),
            Target::Rocket(id) => self
                .common
                .rockets
                .iter()
                .find(|r| r.id == id)
                .map(|r| [r.position.x, r.position.y, r.position.z]),
            Target::Cloud(id) => self
                .common
                .breath_clouds
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.at),
            Target::Bolt(id) => self.common.bolts.iter().find(|b| b.id == id).map(|b| b.at),
            Target::Hook(id) => self
                .client
                .remote_hooks
                .iter()
                .find(|h| h.id == id)
                .map(|h| [h.position.x, h.position.y, h.position.z])
                .or_else(|| {
                    self.client
                        .remote
                        .as_ref()?
                        .tracked(id as i32)
                        .map(|(_, p)| p)
                }),
            Target::Trident(id) => self
                .common
                .tridents
                .iter()
                .find(|t| t.id == id)
                .map(|t| [t.position.x, t.position.y, t.position.z]),
            Target::Fangs(id) => self
                .common
                .fangs
                .list
                .iter()
                .find(|f| f.id == id)
                .map(|f| f.pos),
            Target::Spawner(id) => self
                .common
                .ominous_spawners
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.position),
            Target::Knot(id) => self
                .client
                .remote
                .as_ref()?
                .tracked(id as i32)
                .map(|(_, p)| p),
            Target::General(id) => self
                .client
                .remote
                .as_ref()?
                .general
                .rows
                .get(&(id as i32))
                .map(|row| row.position),
        }
    }

    fn client_target_rotation(&self, target: Target) -> [f32; 2] {
        use sunrise_sim::entity_nbt::loaded_rotation;
        let saved_rotation = |id| {
            self.common
                .persist
                .raw
                .get(&id)
                .map_or([0.0; 2], loaded_rotation)
        };
        let tracked_rotation = |id: u32| {
            self.client
                .remote
                .as_ref()
                .and_then(|r| r.entities.get(&(id as i32)))
                .map_or([0.0; 2], |t| [t.yaw, t.pitch])
        };
        match target {
            Target::Player => [self.common.yaw, self.common.pitch],
            Target::Guest(_) => [0.0; 2],
            Target::Mob(id) => self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or([0.0; 2], |a| [a.yaw, a.head_pitch]),
            Target::Item(id) => self
                .common
                .dropped
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], |a| a.rotation),
            Target::Orb(id) => self
                .common
                .orbs
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], |a| a.rotation),
            Target::Projectile(id) => self
                .common
                .creatures
                .projectiles
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], |a| [a.yaw, a.pitch]),
            Target::Stand(id) => self
                .common
                .armor_stands
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], |a| [a.yaw, a.pitch]),
            Target::Frame(id) => self
                .common
                .item_frames
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], sunrise_sim::entities::ItemFrame::entity_rotation),
            Target::Painting(id) => self
                .common
                .paintings
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], |a| {
                    [f32::from(a.facing) * 90.0, saved_rotation(id)[1]]
                }),
            Target::Boat(id) => self
                .common
                .boats
                .iter()
                .find(|b| b.id == id)
                .map_or([0.0; 2], |a| [a.yaw, a.pitch]),
            Target::Cart(id) => self
                .common
                .carts
                .iter()
                .find(|c| c.id == id)
                .map_or([0.0; 2], |a| [a.yaw, a.pitch]),
            Target::Fangs(id) => self
                .common
                .fangs
                .list
                .iter()
                .find(|f| f.id == id)
                .map_or([0.0; 2], |f| [f.yaw, saved_rotation(id)[1]]),
            Target::Crystal(id) => self
                .common
                .crystals
                .iter()
                .find(|c| c.id == id)
                .map_or([0.0; 2], |c| c.rotation),
            Target::Cloud(id) => saved_rotation(id),
            Target::Bolt(id) => self
                .common
                .bolts
                .iter()
                .find(|b| b.id == id)
                .map_or([0.0; 2], |b| b.rotation),
            Target::Dragon(id) => self
                .common
                .dragon
                .as_ref()
                .filter(|a| a.id == id)
                .map_or([0.0; 2], |a| [a.yaw, a.pitch]),
            Target::Tnt(id) => self
                .common
                .tnts
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], |a| a.rotation),
            Target::Cushion(id) => self
                .common
                .cushions
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], |a| [a.yaw, saved_rotation(id)[1]]),
            Target::Falling(id) => self
                .common
                .falling
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], |a| a.rotation),
            Target::Rocket(id) => self
                .common
                .rockets
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], |a| a.rotation),
            Target::Trident(id) => self
                .common
                .tridents
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], |a| [a.yaw, a.pitch]),
            Target::Spawner(id) => self
                .common
                .ominous_spawners
                .iter()
                .find(|a| a.id == id)
                .and_then(|a| a.raw.as_deref())
                .map_or([0.0; 2], sunrise_sim::entity_nbt::rotation),
            Target::Hook(id) => tracked_rotation(id),
            Target::Other(id) => self
                .client
                .interactions
                .get(&(id as i32))
                .map_or_else(|| tracked_rotation(id), |e| [e.yaw, e.pitch]),
            Target::General(id) => self
                .client
                .remote
                .as_ref()
                .and_then(|r| r.general.rows.get(&(id as i32)))
                .map_or([0.0; 2], |row| row.rotation),
            Target::DragonPart(_) | Target::Knot(_) => [0.0; 2],
        }
    }

    pub(in crate::game) fn client_pick_route_kind(
        &self,
        subject: mod_api::EntityRef,
    ) -> Option<mod_api::EntityTypeKey> {
        if let Some(target) = self.native_client_entity_target(subject) {
            return self.client_target_kind_key(target);
        }
        if let Ok((remote, id)) = self.native_client_entity_context(subject)
            && let Some(row) = remote.general.rows.get(&id)
        {
            return mod_api::__private::resolved_entity_type_key(row.kind());
        }
        mod_api::__private::resolved_entity_type_key(self.native_client_tracked(subject)?.0)
    }

    pub(in crate::game) fn client_pick_entity_view(
        &self,
        subject: mod_api::EntityRef,
    ) -> Option<ResidentClientPickEntity> {
        let (kind, position, rotation) =
            if let Some(target) = self.native_client_entity_target(subject) {
                let position = self.client_target_position(target)?;
                (
                    self.client_target_kind_key(target),
                    position,
                    self.client_target_rotation(target),
                )
            } else if let Ok((remote, id)) = self.native_client_entity_context(subject)
                && let Some(row) = remote.general.rows.get(&id)
            {
                (
                    mod_api::__private::resolved_entity_type_key(row.kind()),
                    row.position,
                    row.rotation,
                )
            } else {
                let (name, position) = self.native_client_tracked(subject)?;
                let rotation = self
                    .client
                    .remote
                    .as_ref()
                    .and_then(|r| {
                        let id = i32::try_from(subject.id()).ok()?;
                        r.entities.get(&id).map(|t| [t.yaw, t.pitch])
                    })
                    .unwrap_or([0.0, 0.0]);
                (
                    mod_api::__private::resolved_entity_type_key(name),
                    position,
                    rotation,
                )
            };
        let uuid = self.client.remote.as_ref().and_then(|r| {
            let id = i32::try_from(subject.id()).ok()?;
            if r.logged_in && id == r.entity_id {
                return Some(r.profile());
            }
            r.level_entity_uuids
                .get(&id)
                .copied()
                .or_else(|| r.general.rows.get(&id).map(|row| row.uuid))
                .or_else(|| r.level_player(id).map(|(uuid, _)| uuid))
                .or_else(|| {
                    self.common
                        .persist
                        .uuids
                        .get(&subject.id())
                        .copied()
                        .map(sunrise_sim::lan::playerdata::ints_uuid)
                })
        });
        Some(ResidentClientPickEntity {
            kind,
            position,
            rotation,
            uuid,
        })
    }

    pub(crate) fn client_entity_interaction_kind(&self, id: i32) -> Option<String> {
        let entity = EntityRef::new(
            self.common.native_lifecycle.epoch()?,
            u32::try_from(id).ok()?,
        );
        if let Some(target) = self.native_client_entity_target(entity) {
            return match target {
                Target::Mob(uid) => self
                    .common
                    .mobs
                    .iter()
                    .find(|m| m.id == uid)
                    .map(|m| sunrise_command::args::normalize(&m.kind.id)),
                Target::Boat(uid) => self
                    .common
                    .boats
                    .iter()
                    .find(|b| b.id == uid)
                    .map(|b| b.entity_type()),
                other => self.client_target_kind_name(other).map(str::to_owned),
            };
        }
        if let Ok((remote, id)) = self.native_client_entity_context(entity)
            && let Some(row) = remote.general.rows.get(&id)
        {
            return Some(row.kind().to_owned());
        }
        self.native_client_tracked(entity)
            .map(|(kind, _)| kind.to_owned())
    }
    pub fn refresh_client_pick(&mut self, partial_tick: f32) {
        if !self.common.native_window()
            || self.client.title.0.is_some()
            || self.common.player.is_none()
            || self.common.native_lifecycle.epoch().is_none()
        {
            return;
        }
        let Some(camera) = self
            .spectated_camera_snapshot()
            .or_else(|| self.player_camera_entity())
        else {
            return;
        };
        let origin = std::array::from_fn(|i| {
            camera.previous[i]
                + f64::from(partial_tick) * (camera.position[i] - camera.previous[i])
                + if i == 1 {
                    f64::from(camera.eye_height)
                } else {
                    0.0
                }
        });
        let direction = view_vector(
            camera.view_y_rot(partial_tick),
            camera.view_x_rot(partial_tick),
        );
        self.client.client_pick.ray = Some((origin, direction));
        self.client.client_pick.origin = origin;
        self.client.client_pick.epoch = self.common.native_lifecycle.epoch();
        self.client.client_pick.dimension = Some(self.common.dim);
        let from = sunrise_core::geometry::Vec3::new(origin[0], origin[1], origin[2]);
        let max_distance = self.common.block_reach.max(self.common.entity_reach);
        let block_hit = self.pick_unfiltered();
        // LocalPlayer.pick: on a block hit, blockDistanceSq shortens both the ray and search bound;
        // on a block miss, search uses nominal maxDistance^2 while final comparison uses to.distanceToSqr(from).
        let (entity_max_distance, search_max_distance_sq, block_distance_sq) = match block_hit {
            Some(hit) => {
                let d2 = from.distance_squared(sunrise_core::geometry::Vec3::new(
                    hit.point[0],
                    hit.point[1],
                    hit.point[2],
                ));
                (d2.sqrt(), d2, d2)
            }
            None => {
                let to = sunrise_core::geometry::Vec3::new(
                    from.x + direction[0] * max_distance,
                    from.y + direction[1] * max_distance,
                    from.z + direction[2] * max_distance,
                );
                (
                    max_distance,
                    max_distance * max_distance,
                    to.distance_squared(from),
                )
            }
        };
        let block_candidate = block_hit
            .filter(|hit| {
                from.distance_squared(sunrise_core::geometry::Vec3::new(
                    hit.point[0],
                    hit.point[1],
                    hit.point[2],
                )) < self.common.block_reach * self.common.block_reach
            })
            .and_then(|hit| self.crosshair_block_target(hit));
        let camera_id = self.pick_camera_id();
        let attack_result = self
            .common
            .active_attack_range()
            .map(|range| self.attack_range_pick_result(&range));
        let mut entity_in_range = None;
        let original = match attack_result {
            Some(
                sunrise_gameplay_common::game::pick_entity_common::AttackRangePickResult::Entity(
                    hit,
                ),
            ) => self
                .common
                .native_lifecycle
                .epoch()
                .map(|epoch| CrosshairTarget::Entity {
                    entity: EntityRef::new(epoch, hit.id),
                    point: hit.point,
                }),
            Some(
                sunrise_gameplay_common::game::pick_entity_common::AttackRangePickResult::Block(
                    hit,
                ),
            ) => self.crosshair_block_target(hit),
            None
            | Some(
                sunrise_gameplay_common::game::pick_entity_common::AttackRangePickResult::Miss,
            ) => {
                let raw_entity_hit =
                    self.pick_entity_candidate_within(entity_max_distance, search_max_distance_sq);
                // Refuse the pick if a rider listener changed camera, epoch, or dimension.
                if camera_id != self.pick_camera_id()
                    || self.client.client_pick.epoch != self.common.native_lifecycle.epoch()
                    || self.client.client_pick.dimension != Some(self.common.dim)
                {
                    self.client.client_pick.ray = None;
                    self.publish_client_pick(None);
                    return;
                }
                entity_in_range = raw_entity_hit.filter(|target| match target {
                    CrosshairTarget::Entity { point, .. } => {
                        from.distance_squared(sunrise_core::geometry::Vec3::new(
                            point[0], point[1], point[2],
                        )) < self.common.entity_reach * self.common.entity_reach
                    }
                    _ => false,
                });
                match raw_entity_hit {
                    Some(CrosshairTarget::Entity { point, .. })
                        if from.distance_squared(sunrise_core::geometry::Vec3::new(
                            point[0], point[1], point[2],
                        )) < block_distance_sq =>
                    {
                        entity_in_range
                    }
                    _ => block_candidate,
                }
            }
        };
        self.publish_client_pick(original);
        self.client.client_pick.taken += 1;
        if !mod_api::__private::listened(mod_api::HookId::ClientPick) {
            self.client.client_pick.ray = None;
            return;
        }
        // Supply block and entity candidates independently only when a ClientPick listener is active.
        let entity_candidate = if matches!(
            attack_result,
            Some(
                sunrise_gameplay_common::game::pick_entity_common::AttackRangePickResult::Entity(_)
            )
        ) {
            original
        } else if entity_in_range.is_some() {
            entity_in_range
        } else if block_hit.is_some()
            || matches!(
                attack_result,
                Some(
                    sunrise_gameplay_common::game::pick_entity_common::AttackRangePickResult::Block(
                        _
                    )
                )
            )
        {
            let independent = self.pick_entity_candidate_within(
                self.common.entity_reach,
                self.common.entity_reach * self.common.entity_reach,
            );
            if camera_id != self.pick_camera_id()
                || self.client.client_pick.epoch != self.common.native_lifecycle.epoch()
                || self.client.client_pick.dimension != Some(self.common.dim)
            {
                self.client.client_pick.ray = None;
                self.publish_client_pick(None);
                return;
            }
            independent.filter(|target| match target {
                CrosshairTarget::Entity { point, .. } => {
                    from.distance_squared(sunrise_core::geometry::Vec3::new(
                        point[0], point[1], point[2],
                    )) < self.common.entity_reach * self.common.entity_reach
                }
                _ => false,
            })
        } else {
            None
        };
        self.client.client_pick.ray = None;
        // Capture the same forward-movement envelope as the vanilla item ray,
        // before listeners can change the player or held item.
        self.client.client_pick.entity_reach = self.client_pick_entity_reach(direction);
        self.storage_scope(|game, _| {
            game.client.client_pick.dispatching = true;
            let mut result = original;
            game.native_client_pick(mod_api::__private::ClientPickCall {
                partial_tick,
                origin,
                original,
                block_candidate,
                entity_candidate,
                result: &mut result,
            });
            game.client.client_pick.dispatching = false;
            let valid_context = game.client.client_pick.epoch == game.native_lifecycle.epoch()
                && game.client.client_pick.dimension == Some(game.dim);
            let result = result.filter(|&result| {
                valid_context && (Some(result) == original || game.accept_client_pick(result))
            });
            game.publish_client_pick(result);
            ((), true)
        });
    }
    /// Consumers read the stored hit as-is until the next pick. In particular,
    /// a block broken earlier in a tick is still that tick's use target.
    pub fn stored_client_pick(&self) -> Option<CrosshairTarget> {
        if self.client.client_pick.dispatching {
            return self.client.client_pick.result;
        }
        (self.client.client_pick.epoch == self.common.native_lifecycle.epoch()
            && self.client.client_pick.dimension == Some(self.common.dim))
        .then_some(())?;
        self.client.client_pick.result
    }
    pub(in crate::game) fn uses_stored_pick(&self) -> bool {
        self.common.native_window() && self.client.client_pick.ray.is_none()
    }
}

use sunrise_gameplay_common::game::side_ops::player::PlayerSideGameExt as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
