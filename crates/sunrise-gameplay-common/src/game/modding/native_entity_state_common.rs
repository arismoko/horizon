use crate::game::commands_common::Target;
use crate::game::native_entity_base::BuiltinKind;
use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
use mod_api::{EntityReadError, EntityRef};
use std::ops::ControlFlow;
use sunrise_sim::mobs::{Mob, Species};
use sunrise_sim::motion::PlayerMotion;

const WATER_ID: &str = "minecraft:water";
const LAVA_ID: &str = "minecraft:lava";

/// `Entity.getMaxAirSupply`: a dolphin's 4800, an axolotl's 6000, else 300.
pub fn max_air(species: Option<Species>) -> i32 {
    match species {
        Some(Species::Dolphin) => 4800,
        Some(Species::Axolotl) => sunrise_sim::mobs::axolotl::MAX_AIR,
        _ => 300,
    }
}

/// What a handle names, as the flags read it.
pub enum Body<'a> {
    Player(&'a PlayerMotion),
    Mob(&'a Mob),
    Other(Target),
}

pub trait NativeEntityStateCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    fn native_target_body<'__loan>(&'__loan self, target: Target) -> Option<Body<'__loan>>
    where
        '__sg: '__loan,
    {
        Some(match target {
            Target::Player => Body::Player(self.player.as_ref()?),
            Target::Guest(conn) => Body::Player(
                crate::game::side_ops::player::SideOps::side_command_guest_motion(
                    self.side_state(),
                    conn,
                )?,
            ),
            Target::Mob(id) => Body::Mob(self.mobs.iter().find(|m| m.id == id)?),
            t => Body::Other(t),
        })
    }

    /// The armor stand an entity handle names, on either side.
    fn native_armor_stand<'__loan>(
        &'__loan self,
        entity: EntityRef,
    ) -> Option<&'__loan sunrise_sim::entities::ArmorStand>
    where
        '__sg: '__loan,
    {
        let Target::Stand(id) = self.native_entity_read_target(entity)? else {
            return None;
        };
        self.armor_stands.iter().find(|s| s.id == id)
    }

    /// `ArmorStand.getHeadPose` ... `getRightLegPose`, in `POSE_KEYS` order.
    fn entity_armor_stand_pose(&self, entity: EntityRef) -> Option<mod_api::ArmorStandPose> {
        let [head, body, left_arm, right_arm, left_leg, right_leg] =
            self.native_armor_stand(entity)?.pose;
        Some(mod_api::ArmorStandPose {
            head,
            body,
            left_arm,
            right_arm,
            left_leg,
            right_leg,
        })
    }

    /// `ArmorStand.DATA_CLIENT_FLAGS`.
    fn entity_armor_stand_flags(&self, entity: EntityRef) -> Option<mod_api::ArmorStandFlags> {
        let stand = self.native_armor_stand(entity)?;
        Some(mod_api::ArmorStandFlags {
            small: stand.small,
            show_arms: stand.show_arms,
            show_base_plate: !stand.no_base_plate,
            marker: stand.marker,
        })
    }

    /// Whether the entity touched water at its last fluid update
    /// (`Entity.isInWater`): players and mobs.
    fn entity_is_in_water(&self, entity: EntityRef) -> Option<bool> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Player(p) => Some(p.fluids.was_touching_water),
            Body::Mob(m) => Some(m.fluids.in_water),
            Body::Other(Target::General(id)) => {
                Some(self.resident_entities.general.row(id)?.fluids.in_water)
            }
            Body::Other(_) => None,
        }
    }

    /// The player's swimming flag, distinct from its pose (which also covers crawling).
    fn entity_is_swimming(&self, entity: EntityRef) -> Option<bool> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Player(p) => Some(p.swimming),
            _ => None,
        }
    }

    /// Visit the cached fluid contacts of the player or mob, highest first.
    fn entity_visit_fluid_heights(
        &self,
        entity: EntityRef,
        visit: &mut dyn FnMut(&str, f64) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        use sunrise_sim::motion::{LAVA, WATER};
        let body = self.native_target_body(self.native_entity_visit_target(entity)?);
        let (water, lava) = match body {
            Some(Body::Player(p)) => (
                (p.fluid_height(WATER), p.fluids.was_touching_water),
                (p.fluid_height(LAVA), p.is_in_lava()),
            ),
            Some(Body::Mob(m)) => (
                (m.fluids.water, m.fluids.in_water),
                (m.fluids.lava, m.fluids.in_lava),
            ),
            Some(Body::Other(Target::General(id))) => {
                let fluids = self
                    .resident_entities
                    .general
                    .row(id)
                    .ok_or(EntityReadError::Unavailable)?
                    .fluids;
                (
                    (fluids.water, fluids.in_water),
                    (fluids.lava, fluids.in_lava),
                )
            }
            _ => return Err(EntityReadError::Unavailable),
        };
        Ok(visit_touched_fluids(water, lava, visit))
    }

    /// The fluid at the entity's eyes from its last fluid update
    /// (`IEntityExtension.getFirstEyeInFluidType`): water or lava, none in air. Players and mobs.
    /// On a client this reads its own cached fluid update, without sampling the world again.
    fn entity_eye_fluid(&self, entity: EntityRef) -> Option<Option<&'static str>> {
        use sunrise_sim::motion::{LAVA, WATER};
        Some(
            match self.native_target_body(self.native_entity_read_target(entity)?)? {
                Body::Player(p) => eye_fluid(p.is_eye_in_fluid(WATER), p.is_eye_in_fluid(LAVA)),
                Body::Mob(m) => eye_fluid(m.fluids.eye_in_water, m.fluids.eye_in_lava),
                Body::Other(Target::General(id)) => {
                    let fluids = self.resident_entities.general.row(id)?.fluids;
                    eye_fluid(fluids.eye_in_water, fluids.eye_in_lava)
                }
                Body::Other(_) => return None,
            },
        )
    }

    /// Entity.isUnderWater uses eye sample AND contact; LocalPlayer's override uses the sample alone.
    fn entity_is_under_water(&self, entity: EntityRef) -> Option<bool> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Player(p) if !self.is_logical_server() => Some(p.is_under_water()),
            Body::Player(p) => Some(p.fluids.was_eye_in_water && p.fluids.was_touching_water),
            Body::Mob(m) => Some(m.was_eye_in_water && m.fluids.in_water),
            Body::Other(Target::General(id)) => {
                Some(self.resident_entities.general.row(id)?.under_water)
            }
            Body::Other(_) => None,
        }
    }

    /// `Entity.isOnFire`: remaining ticks or the client's synced flag, unless fire immune.
    fn entity_is_on_fire(&self, entity: EntityRef) -> Option<bool> {
        let (ticks, immune, kind) = match self.native_entity_read_target(entity) {
            Some(target) => self.side_entity_body_fire_facts(entity, target)?,
            None => (
                0,
                self.client_tracked_fire_immunity(entity)?,
                self.side_native_client_tracked(entity)
                    .and_then(|(id, _)| BuiltinKind::from_type_id(id)),
            ),
        };
        if kind.is_some_and(BuiltinKind::is_on_fire_overridden_false) {
            return Some(false);
        }
        let synced = !self.is_logical_server() && self.side_client_entity_on_fire(entity)?;
        Some(!immune && (ticks > 0 || synced))
    }

    /// `Entity.fireImmune`: effective immunity on the side of the host.
    fn entity_fire_immune(&self, entity: EntityRef) -> Option<bool> {
        match self.native_entity_read_target(entity) {
            Some(target) => self
                .side_entity_body_fire_facts(entity, target)
                .map(|(_, immune, _)| immune),
            None => self.client_tracked_fire_immunity(entity),
        }
    }

    fn client_tracked_fire_immunity(&self, entity: EntityRef) -> Option<bool> {
        if let Some(slot) = self.side_native_client_general_type_slot(entity) {
            return Some(crate::game::general::general_type_catalog().fire_immune(slot));
        }
        let (kind, _) = self.side_native_client_tracked(entity)?;
        if kind == "minecraft:player" {
            return self.side_client_tracked_player_fire_immunity(entity);
        }
        Some(
            BuiltinKind::from_type_id(kind).is_some_and(BuiltinKind::fire_immune)
                || self
                    .res
                    .mob_types()
                    .get(kind)
                    .is_some_and(|mob| mob.flags.fire_immune),
        )
    }

    /// The entity's air (`Entity.getAirSupply`): players and mobs.
    fn entity_air_supply(&self, entity: EntityRef) -> Option<i32> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Player(p) => Some(p.air),
            Body::Mob(m) => Some(m.air),
            Body::Other(_) => None,
        }
    }

    /// The most air the entity holds (`Entity.getMaxAirSupply`): 300, a
    /// dolphin's 4800, an axolotl's 6000.
    fn entity_max_air_supply(&self, entity: EntityRef) -> Option<i32> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Mob(m) => Some(max_air(Some(m.species()))),
            _ => Some(max_air(None)),
        }
    }

    /// Ticks the entity has lived (`Entity.tickCount`): on a server, living entities, item
    /// entities, experience orbs and minecarts; on a client, every entity of the level it
    /// shows (`side_client_entity_tick_count`). Wraps as vanilla's `int` does.
    fn entity_tick_count(&self, entity: EntityRef) -> Option<i32> {
        if !self.is_logical_server() {
            return self.side_client_entity_tick_count(entity);
        }
        Some(match self.side_native_entity_read_target(entity)? {
            Target::Player => self.player.as_ref()?.health.tick_count,
            Target::Guest(conn) => {
                crate::game::side_ops::player::SideOps::side_command_guest_tick_count(
                    self.side_state(),
                    conn,
                )?
            }
            Target::Mob(id) => self.mobs.iter().find(|m| m.id == id)?.age as i32,
            Target::Stand(id) => self.armor_stands.iter().find(|s| s.id == id)?.tick_count,
            Target::Dragon(id) => self.dragon.as_ref().filter(|d| d.id == id)?.ticks,
            Target::Item(id) => self.dropped.iter().find(|i| i.id == id)?.ticks as i32,
            Target::Orb(id) => self.orbs.iter().find(|o| o.id == id)?.ticks as i32,
            Target::Cart(id) => self.carts.iter().find(|c| c.id == id)?.tick_count,
            Target::General(id) => self.resident_entities.general.row(id)?.tick_count,
            _ => return None,
        })
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeEntityStateCommonGameExt1<'__sg> for G {}

/// Visit each fluid `(height, touched)` the entity touched, highest first;
/// water before lava at the same height (`getMaxHeightFluidType`'s order).
pub fn visit_touched_fluids(
    water: (f64, bool),
    lava: (f64, bool),
    visit: &mut dyn FnMut(&str, f64) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let mut fluids = [(WATER_ID, water), (LAVA_ID, lava)];
    if lava.0.total_cmp(&water.0).is_gt() {
        fluids.swap(0, 1);
    }
    fluids
        .into_iter()
        .filter(|(_, (_, touched))| *touched)
        .try_for_each(|(name, (height, _))| visit(name, height))
}

/// The first fluid at the eyes: water, then lava, as the trackers are kept.
pub fn eye_fluid(water: bool, lava: bool) -> Option<&'static str> {
    if water {
        Some(WATER_ID)
    } else {
        lava.then_some(LAVA_ID)
    }
}

use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;

use crate::game::side_ops::player::PlayerSideGameExt as _;

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
