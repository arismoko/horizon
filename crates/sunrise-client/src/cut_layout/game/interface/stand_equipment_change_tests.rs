//! `LivingEntity.collectEquipmentChanges` on an armor stand walks every
//! `EquipmentSlot`: a body or saddle a mod wrote is told to
//! `living_equipment_change` like the six hand and armor slots.
use std::sync::{Arc, Mutex};

use mod_api::{self, EquipmentSlot, Hooks, LivingEquipmentChangeEvent, ServerHost};
use sunrise_core::geometry::Vec3;
use sunrise_items::equipment::{BODY, HEAD, MAINHAND, SADDLE};
use sunrise_items::items::ItemStack;
use sunrise_sim::entities::ArmorStand;

use sunrise_gameplay::game::native_entity_refs::tests::{fixture_resident_column, with_game};

use sunrise_gameplay_common::game::api_values::TryIntoApi;
type Heard = Vec<(
    u32,
    EquipmentSlot,
    Option<mod_api::ItemStack>,
    Option<mod_api::ItemStack>,
)>;

struct Counter(Arc<Mutex<Heard>>);
#[mod_api::__subscriptions]
impl Hooks for Counter {
    fn living_equipment_change(
        &self,
        _: &mut dyn ServerHost,
        event: &mut LivingEquipmentChangeEvent<'_>,
    ) {
        self.0.lock().unwrap().push((
            event.entity.id(),
            event.slot,
            event.from.as_ref().and_then(|stack| stack.project()),
            event.to.as_ref().and_then(|stack| stack.project()),
        ));
    }
}

#[test]
#[ignore = "daily"]
fn a_stands_body_and_saddle_writes_are_told_once_each() {
    let heard = Arc::new(Mutex::new(Heard::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "counter",
        Box::new(Counter(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            game.armor_stands.push(ArmorStand::new(16, Vec3::ZERO, 0.0));
            let body = ItemStack::new("minecraft:diamond_horse_armor", 1);
            let saddle = ItemStack::new("minecraft:saddle", 1);
            let helmet = ItemStack::new("minecraft:iron_helmet", 1);
            let stand = &mut game.armor_stands[0];
            stand.equipment[BODY] = Some(body.clone());
            stand.equipment[SADDLE] = Some(saddle.clone());
            stand.equipment[HEAD] = Some(helmet.clone());
            game.armor_stand_location_effects(0, false);
            // Moving or landing later copies nothing unreported.
            game.armor_stand_location_effects(0, true);
            game.armor_stands[0].equipment[BODY] = None;
            game.armor_stand_location_effects(0, false);
            let heard = heard.lock().unwrap();
            assert_eq!(
                *heard,
                vec![
                    (
                        16,
                        EquipmentSlot::Head,
                        None,
                        Some(helmet.try_into_api().unwrap())
                    ),
                    (
                        16,
                        EquipmentSlot::Body,
                        None,
                        Some(body.try_into_api().unwrap())
                    ),
                    (
                        16,
                        EquipmentSlot::Saddle,
                        None,
                        Some(saddle.try_into_api().unwrap())
                    ),
                    (
                        16,
                        EquipmentSlot::Body,
                        Some(body.try_into_api().unwrap()),
                        None
                    ),
                ]
            );
        })
    });
}

struct Rewriter {
    heard: Arc<Mutex<Heard>>,
    restore_head: ItemStack,
    equip_chest: ItemStack,
}
#[mod_api::__subscriptions]
impl Hooks for Rewriter {
    fn living_equipment_change(
        &self,
        host: &mut dyn ServerHost,
        event: &mut LivingEquipmentChangeEvent<'_>,
    ) {
        self.heard.lock().unwrap().push((
            event.entity.id(),
            event.slot,
            event.from.as_ref().and_then(|stack| stack.project()),
            event.to.as_ref().and_then(|stack| stack.project()),
        ));
        if event.slot == EquipmentSlot::MainHand {
            // with_game resolves the fixture's item keys before dispatching this hook.
            let restore_head = self.restore_head.try_into_api().unwrap();
            let equip_chest = self.equip_chest.try_into_api().unwrap();
            // Undo the pending Head change back to what enchantment_equipment holds
            // and write a previously untouched Chest slot in the same pass.
            event
                .entity
                .set_equipment(host, EquipmentSlot::Head, Some(restore_head))
                .unwrap();
            event
                .entity
                .set_equipment(host, EquipmentSlot::Chest, Some(equip_chest))
                .unwrap();
        }
    }
}

#[test]
#[ignore = "daily"]
fn listener_edits_to_later_slots_take_effect_in_the_same_pass() {
    let heard = Arc::new(Mutex::new(Heard::new()));
    let sword = ItemStack::new("minecraft:iron_sword", 1);
    let old_helmet = ItemStack::new("minecraft:iron_helmet", 1);
    let new_helmet = ItemStack::new("minecraft:diamond_helmet", 1);
    let chestplate = ItemStack::new("minecraft:iron_chestplate", 1);
    let rewriter = Rewriter {
        heard: heard.clone(),
        restore_head: old_helmet.clone(),
        equip_chest: chestplate.clone(),
    };
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("rewriter", Box::new(rewriter))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            game.armor_stands.push(ArmorStand::new(17, Vec3::ZERO, 0.0));
            game.armor_stands[0].equipment[HEAD] = Some(old_helmet.clone());
            game.armor_stand_location_effects(0, false);
            heard.lock().unwrap().clear();

            // Change MainHand and Head, leaving Chest untouched. When MainHand
            // fires, Rewriter restores Head back to `old_helmet` and equips
            // `chestplate` in Chest.
            let stand = &mut game.armor_stands[0];
            stand.equipment[MAINHAND] = Some(sword.clone());
            stand.equipment[HEAD] = Some(new_helmet.clone());
            game.armor_stand_location_effects(0, false);

            // Only MainHand and Chest fire; Head is skipped instead of firing
            // a bogus identical `(old_helmet, old_helmet)` transition.
            assert_eq!(
                *heard.lock().unwrap(),
                vec![
                    (
                        17,
                        EquipmentSlot::MainHand,
                        None,
                        Some(sword.try_into_api().unwrap())
                    ),
                    (
                        17,
                        EquipmentSlot::Chest,
                        None,
                        Some(chestplate.try_into_api().unwrap())
                    ),
                ]
            );

            // A follow-up pass sees everything synchronized and emits nothing.
            heard.lock().unwrap().clear();
            game.armor_stand_location_effects(0, false);
            assert!(heard.lock().unwrap().is_empty());
        })
    });
}
