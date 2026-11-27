pub(crate) use crate::game::creatures::death_weapon::tests::WeaponProbe;
pub(crate) use crate::game::creatures::death_weapon::tests::isolated;
use std::sync::{Arc, Mutex};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::TryIntoApi;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::Damage;
use sunrise_sim::mobs::DamageKind;
use sunrise_sim::mobs::Mob;
use sunrise_sim::mobs::Who;
use sunrise_sim::projectiles::Projectile;
use sunrise_sim::projectiles::ProjectileKind;

#[test]
#[ignore = "daily"]
fn a_player_credited_death_drops_experience_without_a_loot_snapshot() {
    if isolated() {
        return;
    }
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.build_registries();
        game.server.next_entity = 1000;
        game.server.server_loot.0.set(None);
        game.orbs.clear();
        let pos = [2.0, 64.0, 0.0];
        let mut mob = Mob::new(
            70,
            game.res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone(),
            Vec3::new(pos[0], pos[1], pos[2]),
            0.0,
        );
        mob.health = 0.0;
        game.mobs.push(mob.clone());
        game.drop_death_loot(
            &mut mob,
            None,
            true,
            sunrise_world::loot::LootEntity::new("minecraft:player"),
            false,
            pos,
            None,
        );
        let total: i64 = game
            .orbs
            .iter()
            .map(|orb| i64::from(orb.value) * i64::from(orb.count))
            .sum();
        assert_eq!(total, 5, "an adult unequipped zombie's experience");
    });
}

#[test]
#[ignore = "daily"]
fn death_weapon_e_fall_has_no_weapon() {
    if isolated() {
        return;
    }
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        let scratch = tempfile::tempdir().unwrap();
        game.server.world_root = scratch.path().to_path_buf();
        game.server.save.0 = Arc::new(sunrise_sim::save::SaveDir::new(
            scratch.path().join("save"),
            sunrise_sim::save::LevelShape::OVERWORLD,
            Some(Default::default()),
        ));
        game.build_registries();
        let loot = sunrise_world::loot::LootData::load(
            &game.options.resources,
            &Default::default(),
            game.res.item_defaults(),
        )
        .unwrap();
        game.server.server_loot.0.set(Some(Arc::new(loot)));
        let seen = Arc::new(Mutex::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "fall_weapon_probe",
            Box::new(WeaponProbe {
                expected: None,
                expected_source: None,
                swap: None,
                seen: seen.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let ty = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut victim = Mob::new(70, ty, Vec3::ZERO, 0.0);
            victim.health = 0.0;
            victim.last_damage = Some((Damage::fall(), 0));
            game.mob_die(&victim);
        });
        assert_eq!(*seen.lock().unwrap(), 1);
    });
}

pub(crate) fn weapon_case(item: &str, enchantment: &str, level: u32, projectile: bool, swap: bool) {
    if isolated() {
        return;
    }
    sunrise_gameplay::game::native_entity_refs::tests::with_resident_game(|game| {
        let scratch = tempfile::tempdir().unwrap();
        game.server.world_root = scratch.path().to_path_buf();
        game.server.save.0 = Arc::new(sunrise_sim::save::SaveDir::new(
            scratch.path().join("save"),
            sunrise_sim::save::LevelShape::OVERWORLD,
            Some(Default::default()),
        ));
        game.build_registries();
        game.server.next_entity = 1000;
        let loot = sunrise_world::loot::LootData::load(
            &game.options.resources,
            &Default::default(),
            game.res.item_defaults(),
        )
        .unwrap();
        game.server.server_loot.0.set(Some(Arc::new(loot)));
        let ty = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut weapon = ItemStack::new(item, 1);
        weapon.enchant(enchantment, level);
        if item == "minecraft:bow" {
            weapon
                .data_mut()
                .set_loot_name("minecraft:custom_name", &serde_json::json!("Longbow"));
        }
        let expected = weapon.try_into_api().unwrap();
        let patch = expected.components().unwrap().as_nbt();
        assert_eq!(
            patch.get(&sunrise_core::nbt::JavaString::from(
                "minecraft:enchantments"
            )),
            Some(&mod_api::Tag::Compound(
                [(enchantment.into(), mod_api::Tag::Int(level as i32))].into()
            ))
        );
        if item == "minecraft:bow" {
            assert_eq!(
                patch.get(&sunrise_core::nbt::JavaString::from(
                    "minecraft:custom_name"
                )),
                Some(&mod_api::Tag::String("Longbow".into()))
            );
        }

        let axe = ItemStack::new("minecraft:diamond_axe", 1)
            .try_into_api()
            .unwrap();
        let seen = Arc::new(Mutex::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "weapon_probe",
            Box::new(WeaponProbe {
                expected: Some(expected),
                expected_source: None,
                swap: swap.then_some(axe),
                seen: seen.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let attacker_kind = match item {
                "minecraft:bow" => "minecraft:skeleton",
                "minecraft:crossbow" => "minecraft:pillager",
                "minecraft:trident" => "minecraft:drowned",
                _ => "minecraft:zombie",
            };
            let attacker_type = game.res.mob_types().get(attacker_kind).unwrap().clone();
            let mut attacker = Mob::new(41, attacker_type, Vec3::ZERO, 0.0);
            attacker.extra.set_main_hand_stack(Some(weapon.clone()));
            game.mobs.push(attacker);
            let mut victim = Mob::new(70, ty, Vec3::new(2.0, 64.0, 0.0), 0.0);
            let mut damage = Damage::of(if projectile {
                DamageKind::Arrow
            } else {
                DamageKind::MobAttack
            });
            damage.attacker = Some(Who::Mob(41));
            victim.health = 0.0;
            victim.last_damage = Some((damage, 0));
            if projectile {
                let trident = item == "minecraft:trident";
                let kind = if trident {
                    ProjectileKind::Trident
                } else {
                    ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    }
                };
                let mut shot = Projectile::new(kind, Vec3::ZERO, Some(Who::Mob(41)));
                shot.id = 912;
                let arrow = shot.arrow.as_mut().unwrap();
                if trident {
                    arrow.pickup_item = weapon;
                } else {
                    arrow.fired_from_weapon = Some(weapon);
                }
                game.mobs
                    .iter_mut()
                    .find(|m| m.id == 41)
                    .unwrap()
                    .extra
                    .set_main_hand_stack(Some(ItemStack::new("minecraft:stick", 1)));
                // Commit through the same accepted-damage path as real projectile impacts.
                victim.health = 20.0;
                victim.last_damage = None;
                game.mobs.push(victim);
                damage.kind = game.projectile_damage_kind(&shot);
                assert!(game.damage_projectile_with_weapon(&shot, 70, 100.0, damage, None));
            } else {
                game.mob_die(&victim);
            }
        });
        assert_eq!(*seen.lock().unwrap(), 1);
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
