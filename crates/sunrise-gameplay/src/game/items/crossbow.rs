//! Crossbows (`CrossbowItem`): charging with the start/middle/end sounds
//! (Quick Charge), loading arrows or a held firework rocket into
//! `charged_projectiles` (Multishot loads three), and shooting critical
//! arrows at power 3.15, or rockets at 1.6 for three durability, with the
//! fanned Multishot angles and their shot pitches. `Player.getProjectile`
//! finds the ammo for bows and crossbows.
use super::view_vector;
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_gameplay_common::game::crossbow_common::{ARROW, CROSSBOW, FIREWORK_ROCKET};
#[allow(unused_imports)]
#[cfg(test)]
use sunrise_gameplay_common::game::held_items::OFFHAND_SLOT;
use sunrise_items::items::{ItemStack, Items};
use sunrise_sim::{
    mobs::Who,
    projectiles::{Projectile, ProjectileKind},
};

#[allow(unused_imports)]
pub use super::held_items::{is_arrow, projectile_slot};

/// `CrossbowItem.onUseTick`'s loading sounds for a charge now at `charge`
/// (of `getChargeDuration`) that has played `played` so far: the start and
/// middle sounds each play once the charge reaches their point, and both
/// count as unheard again while the charge is under the first one. A tick
/// that jumped the charge past a point still plays that point's sound, as
/// vanilla's two flags do. Returns the flags for the next tick and the
/// sounds to play now.
pub(in crate::game::server_side) fn charge_sounds(
    charge: f32,
    played: [bool; 2],
    sounds: [&'static str; 3],
) -> ([bool; 2], Option<&'static str>, Option<&'static str>) {
    let mut played = if charge < 0.2 { [false; 2] } else { played };
    let mut play = [None; 2];
    for (i, point) in [0.2, 0.5].into_iter().enumerate() {
        if charge >= point && !played[i] && !sounds[i].is_empty() {
            played[i] = true;
            play[i] = Some(sounds[i]);
        }
    }
    (played, play[0], play[1])
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn server_projectile_player_uuid(&mut self) -> Option<u128> {
        Some(sunrise_sim::lan::playerdata::ints_uuid(
            self.acting_player_uuid(),
        ))
    }
    pub(in crate::game::server_side) fn charging_sounds(stack: &ItemStack) -> [&'static str; 3] {
        match stack.enchantment_level("minecraft:quick_charge") {
            0 => [
                "minecraft:item.crossbow.loading_start",
                "minecraft:item.crossbow.loading_middle",
                "minecraft:item.crossbow.loading_end",
            ],
            1 => [
                "minecraft:item.crossbow.quick_charge_1",
                "",
                "minecraft:item.crossbow.loading_end",
            ],
            2 => [
                "minecraft:item.crossbow.quick_charge_2",
                "",
                "minecraft:item.crossbow.loading_end",
            ],
            _ => [
                "minecraft:item.crossbow.quick_charge_3",
                "",
                "minecraft:item.crossbow.loading_end",
            ],
        }
    }
    /// `AbstractSkeleton.performRangedAttack`'s `getProjectile`: the arrow
    /// `mob` shot becomes the replaced ammunition's (`ProjectileUtil.getMobArrow`).
    pub(in crate::game::server_side) fn mob_bow_ammo(
        &mut self,
        mob: u32,
        weapon: &ItemStack,
        arrow: &mut Projectile,
    ) {
        if !mod_api::__private::listened(mod_api::HookId::LivingGetProjectile) {
            return;
        }
        let Some(shooter) = self.common.mobs.iter().find(|m| m.id == mob) else {
            return;
        };
        let kind = shooter.kind.id.clone();
        let vanilla = Self::mob_vanilla_ammo(&shooter.extra, weapon, world_items!(self));
        let mut projectile = Some(vanilla.clone());
        self.native_living_get_projectile(None, "", Some(mob), &kind, weapon, &mut projectile);
        let Some(ammo) =
            projectile.filter(|ammo| *ammo != vanilla && is_arrow(world_items!(self), ammo))
        else {
            return;
        };
        Self::arrow_from_ammo(&ammo, arrow);
    }
    /// `Monster.getProjectile`: the projectile `weapon` supports in the off
    /// hand, then the main hand (`getHeldProjectile`), else a new arrow.
    pub(in crate::game::server_side) fn mob_vanilla_ammo(
        extra: &sunrise_sim::mobs::Extra,
        weapon: &ItemStack,
        items: &Items,
    ) -> ItemStack {
        let supported = |s: &ItemStack| {
            is_arrow(items, s) || &*weapon.item == CROSSBOW && &*s.item == FIREWORK_ROCKET
        };
        extra
            .slot_stack(sunrise_items::equipment::OFFHAND)
            .cloned()
            .into_iter()
            .chain(extra.main_hand_stack().cloned())
            .find(supported)
            .unwrap_or_else(|| ItemStack::new(ARROW, 1))
    }
    /// `ProjectileUtil.getMobArrow`: `arrow` carries `ammo`'s item and effect.
    pub(in crate::game::server_side) fn arrow_from_ammo(ammo: &ItemStack, arrow: &mut Projectile) {
        if let ProjectileKind::Arrow { effect, .. } = &mut arrow.kind {
            if &*ammo.item == "minecraft:spectral_arrow" {
                *effect = Some(("minecraft:glowing", 200));
            } else if effect.is_none()
                && let Some(tipped) = super::dispenser::tipped_effect(ammo)
            {
                // Stray.getArrow and Bogged.getArrow add their effect to the tipped arrow.
                *effect = Some(tipped);
            }
        }
        if let Some(a) = arrow.arrow.as_mut() {
            a.set_item(&sunrise_sim::entities::arrows::drawn_ammo(ammo, false));
        }
    }
    /// `CrossbowItem.onUseTick`'s `getProjectile` for a mob that finished
    /// charging `weapon` (pillagers and piglins: `Monster.getProjectile`). The
    /// replaced ammunition is what the crossbow holds; none leaves it uncharged
    /// and returns false (`tryLoadProjectiles`).
    pub(in crate::game::server_side) fn mob_crossbow_ammo(
        &mut self,
        mob: u32,
        weapon: &ItemStack,
    ) -> bool {
        let Some(shooter) = self.common.mobs.iter().find(|m| m.id == mob) else {
            return false;
        };
        let kind = shooter.kind.id.clone();
        let mut projectile = Some(Self::mob_vanilla_ammo(
            &shooter.extra,
            weapon,
            world_items!(self),
        ));
        self.native_living_get_projectile(None, "", Some(mob), &kind, weapon, &mut projectile);
        // A crossbow takes arrows and firework rockets only.
        let projectile = projectile
            .filter(|ammo| is_arrow(world_items!(self), ammo) || &*ammo.item == FIREWORK_ROCKET);
        let Some(shooter) = self.common.mobs.iter_mut().find(|m| m.id == mob) else {
            return false;
        };
        let Some(mut crossbow) = shooter.extra.main_hand_stack().cloned() else {
            return false;
        };
        let loaded = projectile.is_some();
        crossbow.data_mut().charged = projectile
            .iter()
            .map(|ammo| sunrise_sim::entities::arrows::drawn_ammo(ammo, false))
            .collect();
        crossbow.tidy();
        shooter.extra.set_main_hand_stack(Some(crossbow));
        loaded
    }
    /// `CrossbowItem.use`: shoot when charged, else start charging.
    pub fn use_crossbow(&mut self, stack: &ItemStack) -> bool {
        if &*stack.item != CROSSBOW {
            return false;
        }
        // Use is not re-triggered while the crossbow is still held charging.
        if self.common.using.is_some() {
            return true;
        }
        if stack.data().is_some_and(|d| !d.charged.is_empty()) {
            self.shoot_crossbow();
        } else if self.get_projectile(stack).is_some() {
            self.start_using_item();
        }
        true
    }
    /// `CrossbowItem.onUseTick` for the charge in progress.
    pub fn tick_crossbow(&mut self, slot: usize, ticks: u32, stack: &ItemStack) {
        // getUseDuration - the ticks left, as a mod's Start or Tick left them.
        let held = self.ticks_held(stack, ticks);
        self.common.using = Some((slot, ticks + 1));
        let charge = sunrise_gameplay_common::game::CommonGame::charge_ticks(stack).max(1) as f32;
        let now = (held + 1) as f32 / charge;
        let sounds = Self::charging_sounds(stack);
        let at = self
            .common
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        let (played, start, middle) = charge_sounds(now, self.server.crossbow_sounds, sounds);
        self.server.crossbow_sounds = played;
        if let (Some(sound), Some(at)) = (start, at) {
            self.server_play_sound(sound, at, 0.5, 1.0);
        }
        if let (Some(sound), Some(at)) = (middle, at) {
            self.server_play_sound(sound, at, 0.5, 1.0);
        }
        let charged = stack.data().is_some_and(|d| !d.charged.is_empty());
        if now >= 1.0 && !charged {
            // tryLoadProjectiles / ProjectileWeaponItem.draw.
            let Some(ammo) = self.get_projectile(stack) else {
                return;
            };
            // The projectile hook may have changed the selected ammunition or weapon.
            if let Some(i) = ammo.slot
                && self.common.stack_in(i) != Some(&ammo.stack)
            {
                return;
            }
            let (drawn, debit) = self.draw_weapon_ammo(
                stack,
                &ammo.stack,
                self.own_entity_id() as u32,
                "minecraft:player",
                self.common.creative,
            );
            if drawn.is_empty() {
                return;
            }
            let Some(bow) = self
                .common
                .stack_in_mut(slot)
                .as_mut()
                .filter(|bow| &**bow == stack)
            else {
                return;
            };
            // A pack can make the weapon its own ammo. Debit it without
            // charging it, so the destruction notice sees the original stack.
            if debit > 0 && ammo.slot == Some(slot) {
                self.consume_player_slot(slot, debit);
                return;
            }
            // Store the projectile before debiting the ammo slot: its destruction
            // listener can now move the loaded bow without losing the projectile.
            // useAmmo: the extra multishot arrows and creative's are intangible.
            bow.data_mut().charged = drawn;
            if debit > 0
                && let Some(i) = ammo.slot
            {
                self.consume_player_slot(i, debit);
            }
            let pitch = 1.0 / (self.common.world.random.next_float() * 0.5 + 1.0) + 0.2;
            if let Some(at) = at {
                self.server_play_sound(sounds[2], at, 1.0, pitch);
            }
        }
    }
    /// `CrossbowItem.performShooting`.
    pub fn shoot_crossbow(&mut self) {
        let Some(eye) = self.common.eye_position() else {
            return;
        };
        let slot = self.common.held_slot();
        let Some(loaded) = self.common.stack_in(slot).cloned() else {
            return;
        };
        // NeoForge runs this for players before removing the charged projectiles.
        let mut charge = 1;
        let mut canceled = false;
        self.native_arrow_loose(&loaded, true, &mut charge, &mut canceled);
        if canceled || charge < 0 {
            return;
        }
        let Some(bow) = self
            .common
            .stack_in_mut(slot)
            .as_mut()
            .filter(|bow| **bow == loaded)
        else {
            return;
        };
        let charged = std::mem::take(&mut bow.data_mut().charged);
        bow.tidy();
        let weapon = bow.clone();
        let count = charged.len();
        if count == 0 {
            return;
        }
        // getShootingPower: a rocket among them slows every shot.
        let power = if charged.iter().any(|s| &*s.item == FIREWORK_ROCKET) {
            1.6
        } else {
            3.15
        };
        let spread = self.entity_enchantment_value(
            &weapon,
            self.own_entity_id() as u32,
            "minecraft:player",
            "minecraft:projectile_spread",
            0.0,
        );
        let spread = if spread < 0.0 { 0.0 } else { spread };
        let step = if count == 1 {
            0.0
        } else {
            2.0 * spread / (count - 1) as f32
        };
        let offset = ((count - 1) % 2) as f32 * step / 2.0;
        let view = glam::DVec3::from_array(view_vector(self.common.yaw, self.common.pitch));
        let up = glam::DVec3::from_array(view_vector(self.common.yaw, self.common.pitch - 90.0));
        let mut direction = 1.0;
        for (index, ammo) in charged.iter().enumerate() {
            let angle = offset + direction * index.div_ceil(2) as f32 * step;
            direction = -direction;
            let dir =
                glam::DQuat::from_axis_angle(up.normalize_or_zero(), f64::from(angle.to_radians()))
                    * view;
            // createProjectile: a rocket shot at an angle from 0.15 under the
            // eyes, spread by its own random; three durability.
            let rocket = &*ammo.item == FIREWORK_ROCKET;
            if rocket {
                let at = Vec3::new(eye[0], eye[1] - f64::from(0.15f32), eye[2]);
                // Its first tick posts the shot (Projectile.tick).
                self.launch_angled_rocket(at, ammo, Some(Who::Player), |own| {
                    let mut shot = Projectile::new(ProjectileKind::Snowball, at, None);
                    shot.shoot(own, dir.to_array(), power, 1.0);
                    shot.velocity
                });
            }
            if !rocket {
                let effect = super::dispenser::tipped_effect(ammo).or_else(|| {
                    (&*ammo.item == "minecraft:spectral_arrow")
                        .then_some(("minecraft:glowing", 200))
                });
                let kind = ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: true,
                    effect,
                };
                let mut arrow = Projectile::new(
                    kind,
                    Vec3::new(eye[0], eye[1] - 0.1, eye[2]),
                    Some(Who::Player),
                );
                let pierce = self.projectile_pierce(&weapon, ammo);
                if let Some(a) = arrow.arrow.as_mut() {
                    a.set_item(ammo);
                    a.set_owner(Some(Who::Player));
                    a.fired_from_weapon = Some(weapon.clone());
                    a.crossbow_hit_sound = true;
                    a.pierce = pierce;
                }
                // Flame: `spawn_projectile` runs EnchantmentHelper.onProjectileSpawned
                // for the weapon the arrow was fired from.
                arrow.shoot_own(dir.to_array(), power, 1.0);
                self.spawn_projectile(arrow, ammo);
            }
            // Ordinary arrows post from ArrowState on their first tick.
            let pitch = if index == 0 {
                1.0
            } else {
                let high = index & 1 == 1;
                1.0 / (self.common.world.random.next_float() * 0.5 + 1.8)
                    + if high { 0.63 } else { 0.43 }
            };
            // CrossbowItem.performShooting: at the shooter's getX/Y/Z (feet).
            let feet = self
                .common
                .player
                .as_ref()
                .map_or(eye, |pl| [pl.position.x, pl.position.y, pl.position.z]);
            self.server_play_sound("minecraft:item.crossbow.shoot", feet, 1.0, pitch);
            self.damage_held(if rocket { 3 } else { 1 });
        }
        // CrossbowItem.performShooting: SHOT_CROSSBOW with the crossbow, which
        // is an empty stack (no match) when the shots broke it.
        if let Some(stack) = self.common.held().cloned() {
            self.trigger_item("minecraft:shot_crossbow", &stack);
            self.award_stat("used", &stack.item, 1);
        }
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::living_get_projectile::GameHooks as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    /// A charge that crosses a point between two ticks plays that point's
    /// sound, and one a mod's tick jumps over (`native_living_entity_use_item_tick`
    /// cutting the ticks left) still plays every point it passed, as
    /// `CrossbowItem.onUseTick`'s two flags do.
    #[test]
    fn a_jumped_charge_still_plays_the_loading_sounds_it_passed() {
        let sounds = [
            "minecraft:item.crossbow.loading_start",
            "minecraft:item.crossbow.loading_middle",
            "minecraft:item.crossbow.loading_end",
        ];
        // Each step: the charge and the sounds it plays, from the flags the
        // step before left.
        let mut played = [false; 2];
        let mut step = |charge: f32| {
            let (next, start, middle) = charge_sounds(charge, played, sounds);
            played = next;
            [start, middle]
        };
        assert_eq!(step(0.1), [None, None]);
        assert_eq!(step(0.25), [Some(sounds[0]), None]);
        assert_eq!(step(0.25), [None, None]);
        assert_eq!(step(0.55), [None, Some(sounds[1])]);
        assert_eq!(step(1.0), [None, None]);
        // A charge below the first point is a new charge: both sound again.
        assert_eq!(step(0.1), [None, None]);
        assert_eq!(step(0.6), [Some(sounds[0]), Some(sounds[1])]);
        // One tick from the start past every point, as a mod's tick makes it.
        assert_eq!(step(0.1), [None, None]);
        assert_eq!(step(0.9), [Some(sounds[0]), Some(sounds[1])]);
        // Quick Charge 1 and up have no middle sound.
        let quick = [sounds[0], "", sounds[2]];
        assert_eq!(
            charge_sounds(0.9, [false; 2], quick),
            ([true, false], Some(sounds[0]), None)
        );
    }

    #[test]
    fn held_projectiles_come_before_the_inventory() {
        let stack = |item: &str| Some(ItemStack::new(item, 1));
        // The vanilla `#minecraft:arrows` the ammo rules ask for.
        let items = Items::for_tests(std::collections::HashMap::from([(
            "minecraft:arrows".to_owned(),
            [
                "minecraft:arrow",
                "minecraft:spectral_arrow",
                "minecraft:tipped_arrow",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        )]));
        let (crossbow, rocket, arrow) = (
            stack(CROSSBOW),
            stack(FIREWORK_ROCKET),
            stack("minecraft:arrow"),
        );
        let mut inventory = vec![None; 36];
        inventory[5] = arrow.clone();
        // The weapon in hotbar slot 0, `off` in the off hand.
        let slot = |weapon, off: &Option<ItemStack>, inventory: &[Option<ItemStack>]| {
            let hands = [(OFFHAND_SLOT, off.as_ref()), (0, crossbow.as_ref())];
            projectile_slot(weapon, hands, inventory, &items)
        };
        // The off hand's rocket loads a crossbow; a bow only takes arrows.
        assert_eq!(slot(CROSSBOW, &rocket, &inventory), Some(OFFHAND_SLOT));
        assert_eq!(slot("minecraft:bow", &rocket, &inventory), Some(5));
        assert_eq!(
            slot("minecraft:bow", &arrow, &inventory),
            Some(OFFHAND_SLOT)
        );
        assert_eq!(slot(CROSSBOW, &None, &inventory), Some(5));
        // A rocket in the inventory is not ammo.
        inventory[5] = rocket.clone();
        assert_eq!(slot(CROSSBOW, &None, &inventory), None);
        // The main hand's rocket, with the crossbow in the off hand.
        let hands = [(OFFHAND_SLOT, crossbow.as_ref()), (3, rocket.as_ref())];
        assert_eq!(
            projectile_slot(CROSSBOW, hands, &inventory, &items),
            Some(3)
        );

        // Spectral and tipped arrows in off hand or inventory are recognized.
        let spectral = stack("minecraft:spectral_arrow");
        let tipped = stack("minecraft:tipped_arrow");
        inventory[7] = spectral.clone();
        assert_eq!(slot("minecraft:bow", &None, &inventory), Some(7));
        assert_eq!(
            slot("minecraft:bow", &tipped, &inventory),
            Some(OFFHAND_SLOT)
        );
        inventory[7] = None;
        assert_eq!(slot("minecraft:bow", &None, &inventory), None);
    }
}
