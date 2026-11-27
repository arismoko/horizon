//! Role-neutral mount helpers shared by server riding, client HUD/guide, and remote vehicles.
use sunrise_core::geometry::Vec3;
use sunrise_sim::{
    mobs::{Species, equine},
    sprint::Keys,
};

pub fn steering_item(species: Species, main: Option<&str>, offhand: Option<&str>) -> bool {
    let item = match species {
        Species::Pig => "minecraft:carrot_on_a_stick",
        Species::Strider => "minecraft:warped_fungus_on_a_stick",
        _ => return false,
    };
    main == Some(item) || offhand == Some(item)
}

#[derive(Default)]
pub struct MountState {
    /// `jumpRidingTicks`: -10 cooling down after a jump, >= 0 charging.
    pub jump_ticks: i32,
    /// `jumpRidingScale` (0..1).
    pub jump_scale: f32,
    pub(in crate::game) was_jumping: bool,
}
impl MountState {
    pub fn reset_jump(&mut self) {
        self.jump_ticks = 0;
        self.jump_scale = 0.0;
    }
}
/// `EntityAttachments.transformPoint`: `Vec3.yRot(-yRot * (float) (Math.PI / 180.0))`,
/// an attachment point turned with the vehicle.
pub(in crate::game) fn turned(a: [f64; 3], yaw: f32) -> Vec3 {
    Vec3::new(a[0], a[1], a[2]).rotate_y(-yaw * (std::f64::consts::PI / 180.0) as f32)
}

/// `Entity.getPassengerRidingPosition` for a player on `m`: the vehicle's
/// attachment point, turned with it, less the player's own vehicle attachment
/// (`Avatar.DEFAULT_VEHICLE_ATTACHMENT`, a `Vec3(0.0, 0.6, 0.0)` of doubles).
pub(in crate::game) fn seat_of(m: &sunrise_sim::mobs::Mob, attachment: [f64; 3]) -> Vec3 {
    let at = turned(attachment, m.yaw);
    Vec3::new(
        m.position.x + at.x,
        m.position.y + at.y - 0.6,
        m.position.z + at.z,
    )
}

impl crate::game::CommonGame<'_> {
    pub fn mount_index(&self) -> Option<usize> {
        let id = self.riding?;
        self.mobs.iter().position(|m| m.id == id)
    }

    /// `PlayerRideableJumping`: horses, camels and nautili (the dash).
    pub fn jumps(species: Species) -> bool {
        equine::is_equine(species) || sunrise_sim::mobs::nautilus::is_nautilus(species)
    }

    pub fn charge_mount_jump(&mut self, i: usize, keys: Keys) -> f32 {
        let saddled = self.mobs[i].extra.saddled() && Self::jumps(self.mobs[i].species());
        let mut jump = 0.0;
        if saddled {
            let st = &mut self.mount;
            if st.jump_ticks < 0 {
                st.jump_ticks += 1;
                if st.jump_ticks == 0 {
                    st.jump_scale = 0.0;
                }
            }
            if st.was_jumping && !keys.jump {
                st.jump_ticks = -10;
                let amount = (st.jump_scale * 100.0).floor() as i32;
                jump = equine::jump_pending_scale(amount);
            } else if !st.was_jumping && keys.jump {
                st.jump_ticks = 0;
                st.jump_scale = 0.0;
            } else if st.was_jumping {
                st.jump_ticks += 1;
                st.jump_scale = if st.jump_ticks < 10 {
                    st.jump_ticks as f32 * 0.1
                } else {
                    0.8 + 2.0 / (st.jump_ticks - 9) as f32 * 0.1
                };
            }
            st.was_jumping = keys.jump;
        }
        jump
    }
}

pub trait MountsCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `positionRider`: the player sits on the mount after it moves.
    fn position_rider(&mut self) {
        let Some(i) = self.mount_index() else {
            return;
        };
        let (mob_id, is_happy_ghast) = {
            let m = &self.mobs[i];
            (m.id, m.species() == Species::HappyGhast)
        };
        let seat_index = if is_happy_ghast {
            self.side_own_passenger_index(mob_id)
        } else {
            0
        };
        let m = &self.mobs[i];
        let mut at = if sunrise_sim::mobs::camel::is_camel(m.species()) {
            seat_of(m, sunrise_sim::mobs::camel::passenger_attachment(m))
        } else {
            seat_of(m, sunrise_sim::mobs::riders::type_attachment(m))
        };
        if is_happy_ghast {
            at = seat_of(
                m,
                sunrise_sim::mobs::riders::passenger_attachment_at(m, seat_index),
            );
        }
        let rear = turned(equine::rear_offset(m), m.yaw);
        at = Vec3::new(at.x + rear.x, at.y + rear.y, at.z + rear.z);
        let common = self.common_mut();
        let first = common.posed_rider != common.riding;
        common.posed_rider = common.riding;
        if let Some(p) = common.player.as_mut() {
            common.previous_position = if first { at } else { p.position };
            p.position = at;
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> MountsCommonGameExt1<'__sg> for G {}

use crate::game::side_ops::mobs_rest::MobsRestSideGameExt as _;
