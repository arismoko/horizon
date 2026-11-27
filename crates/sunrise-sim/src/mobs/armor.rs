//! Effective mob armor and toughness at the damage boundary.
use super::Mob;
use super::finalize::{self, Attr};
use std::sync::Arc;
use sunrise_world::attribute_map::ValueKind;
use sunrise_world::attributes::entity::{Catalog, EntityAttributes};

/// Restore saved shulker armor at admission and after invalidating writes,
/// before damage can read it without an attribute query or equipment tick.
pub fn prepare(mob: &Mob, catalog: &Arc<Catalog>) {
    if mob.species() == super::Species::Shulker {
        mob.extra
            .live
            .read(
                catalog,
                &mob.kind.id,
                "minecraft:armor",
                ValueKind::Computed,
                || {
                    crate::entity_nbt::attribute_entries(mob)
                        .into_iter()
                        .map(|(_, entry)| entry)
                        .collect()
                },
            )
            .expect("a shulker has an armor attribute");
    }
}

/// The attribute's value on the mob's own map; none without a map, or for an
/// attribute its catalog lacks.
fn effective(map: Option<&EntityAttributes>, name: &str, base: f64) -> Option<f64> {
    let map = map?;
    let key = map.catalog().id(name)?;
    if let Some(source) = map.instance(key) {
        return Some(source.value());
    }
    let definition = map.catalog().attribute(key).definition;
    Some(definition.sanitize(map.base_value(key).unwrap_or(base)))
}

/// LivingEntity.getArmorValue floors the effective double before converting
/// its integer result to float. Toughness converts its effective double directly.
///
/// The live attribute map answers first (#941): the type's base, what
/// `/attribute` and the save left, a mod's modifiers, and every worn or held
/// stack's own `attribute_modifiers`, components included — the same map a
/// `LivingRef` reads, which the tick's equipment pass keeps current. A
/// shulker's `minecraft:covered` lives on that map too. Without a map yet
/// (nothing equipped, nothing written), the type's own armor fields and the
/// `/attribute` armor map answer, as before.
pub fn values(mob: &Mob) -> (f32, f32) {
    if let Some(live) = mob.extra.live.built() {
        let toughness = live.key("minecraft:armor_toughness").and_then(|key| {
            live.value(Some(key), sunrise_world::attribute_map::ValueKind::Computed)
                .ok()
        });
        let armor = live.key("minecraft:armor").and_then(|key| {
            live.value(Some(key), sunrise_world::attribute_map::ValueKind::Computed)
                .ok()
        });
        // The computed values are already inside the catalog's bounds.
        if let (Some(armor), Some(toughness)) = (armor, toughness) {
            return (armor.floor() as i32 as f32, toughness as f32);
        }
    }
    let map = mob.extra.generic_attributes.as_deref();
    let armor_base =
        finalize::base_value_with(mob, Attr::Armor, || Some(f64::from(mob.kind.armor)));
    let armor = effective(map, "minecraft:armor", armor_base)
        .unwrap_or_else(|| mob.kind.attribute_bounds.sanitize(Attr::Armor, armor_base));
    // Without a map, the type's own toughness: its row's value, which nothing
    // writes past the bounds.
    let toughness_base = f64::from(mob.kind.armor_toughness);
    let toughness =
        effective(map, "minecraft:armor_toughness", toughness_base).unwrap_or(toughness_base);
    (armor.floor() as i32 as f32, toughness as f32)
}

/// A mob's `getMaxAbsorption`: its `max_absorption` attribute, an
/// Absorption effect's stored modifier among the others
/// ([`Mob::set_effect_modifiers`]).
pub fn max_absorption(mob: &Mob) -> f32 {
    finalize::value(mob, Attr::MaxAbsorption) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::{Damage, DamageKind, Species};
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_world::attributes::{Modifier, Operation, entity::Catalog};

    /// A pig whose live attribute map holds `armor` and `toughness` bases, as
    /// `/attribute` writes and the save leave them, with its catalog for edits.
    fn pig(armor: f64, toughness: f64) -> (Mob, Arc<Catalog>) {
        let bits = |v: f64| format!("{:016x}", v.to_bits());
        let catalog = Arc::new(Catalog::from_json(&serde_json::json!({
            "data_version": sunrise_world::DATA_VERSION,
            "attributes": [
                {"id":"minecraft:armor", "default":bits(0.0), "min":bits(0.0), "max":bits(30.0), "syncable":true},
                {"id":"minecraft:armor_toughness", "default":bits(0.0), "min":bits(0.0), "max":bits(20.0), "syncable":true}
            ],
            "entities": {"minecraft:pig":{"minecraft:armor":bits(0.0), "minecraft:armor_toughness":bits(0.0)}}
        })).unwrap());
        let mut live = crate::living_attributes::LivingAttributes::default();
        for (name, base) in [
            ("minecraft:armor", armor),
            ("minecraft:armor_toughness", toughness),
        ] {
            live.edit(
                &catalog,
                "minecraft:pig",
                name,
                true,
                Vec::new,
                |value, observer| value.set_base_value(base, observer),
            )
            .unwrap();
        }
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = "minecraft:pig".into();
        ty.species = Species::Pig;
        let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
        mob.extra.live = live;
        mob.attrs.max_health = 1000.0;
        mob.health = 1000.0;
        (mob, catalog)
    }

    #[test]
    fn effective_mob_armor_controls_damage_and_fractional_armor_floors() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        for (armor, toughness, source, expected) in [
            (20.0, 8.0, DamageKind::Cactus, 999039),
            (0.0, 8.0, DamageKind::Cactus, 996000),
            (0.5, 8.0, DamageKind::Cactus, 996000),
            (20.0, 8.0, DamageKind::SonicBoom, 996000),
        ] {
            let (mut mob, _) = pig(armor, toughness);
            assert!(mob.damage(level, 4.0, Damage::of(source)));
            assert_eq!((f64::from(mob.health) * 1000.0) as i32, expected);
        }
    }

    /// A worn stack's modifiers are the live map's transient rows, whatever
    /// changed them: a mod's, `/item`'s or `set_equipment`'s (#941).
    fn wear(mob: &mut Mob, catalog: &Arc<Catalog>, amount: f64, operation: Operation) {
        mob.extra
            .live
            .edit(
                catalog,
                "minecraft:pig",
                "minecraft:armor",
                true,
                Vec::new,
                |value, observer| {
                    value.add_transient(
                        Modifier::new("minecraft:armor.helmet", amount, operation).unwrap(),
                        observer,
                    )
                },
            )
            .unwrap();
    }

    #[test]
    fn equipment_is_counted_once_before_effective_limits() {
        let (mut mob, catalog) = pig(20.0, 8.0);
        wear(&mut mob, &catalog, 3.0, Operation::AddValue);
        assert_eq!(values(&mob), (23.0, 8.0));
        let (mut mob, catalog) = pig(10.0, 8.0);
        wear(&mut mob, &catalog, 0.5, Operation::AddMultipliedTotal);
        assert_eq!(values(&mob), (15.0, 8.0));
        let (mob, _) = pig(100.0, 100.0);
        assert_eq!(values(&mob), (30.0, 20.0));
    }

    /// A mob that never built a map answers from its type's armor fields, and
    /// one whose type lacks the attributes from the range's floor.
    #[test]
    fn a_mob_without_a_map_answers_its_types_fields() {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.armor = 7.0;
        ty.armor_toughness = 2.0;
        let mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
        assert_eq!(values(&mob), (7.0, 2.0));
        assert!(mob.extra.live.built().is_none());
    }
    /// Absorption's stored modifier is one more `ADD_VALUE` modifier on the
    /// mob's `max_absorption`: under the multiplying operations and the range
    /// clamp, `getMaxAbsorption` is what the pinned 26.3 server answered (36,
    /// 28, 60, 2042 and 2048), and the base stays as it was.
    #[test]
    fn absorption_is_an_add_value_modifier_under_every_operation() {
        use finalize::Op;
        let bits = |v: f64| format!("{:016x}", v.to_bits());
        let catalog = Catalog::from_json(&serde_json::json!({
            "data_version": sunrise_world::DATA_VERSION,
            "attributes": [
                {"id":"minecraft:max_absorption", "default":bits(0.0), "min":bits(0.0), "max":bits(2048.0), "syncable":true}
            ],
            "entities": {"minecraft:pig":{"minecraft:max_absorption":bits(0.0)}}
        }))
        .unwrap();
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.attributes = Arc::new([("minecraft:max_absorption".to_owned(), 0.0)].into());
        ty.attribute_bounds = finalize::AttrBounds::of(&Arc::new(catalog.clone()));
        // base, other modifiers, Absorption's amplifier, the Java value
        let cases: [(f64, &[(f64, Op)], i32, f32); 5] = [
            (20.0, &[(0.5, Op::AddMultipliedBase)], 0, 36.0),
            (10.0, &[(1.0, Op::AddMultipliedTotal)], 0, 28.0),
            (
                10.0,
                &[
                    (2.0, Op::AddValue),
                    (0.5, Op::AddMultipliedBase),
                    (1.0, Op::AddMultipliedTotal),
                ],
                1,
                60.0,
            ),
            (2048.0, &[(-10.0, Op::AddValue)], 0, 2042.0),
            (2046.0, &[], 0, 2048.0),
        ];
        for (base, mods, amplifier, java) in cases {
            let mut mob = Mob::new(1, kind.clone(), Vec3::ZERO, 0.0);
            finalize::set_base(&mut mob, Attr::MaxAbsorption, base);
            for (n, (amount, op)) in mods.iter().enumerate() {
                finalize::add_modifier(
                    &mut mob,
                    Attr::MaxAbsorption,
                    format!("h203:m{n}"),
                    *amount,
                    *op,
                );
            }
            mob.add_effect("minecraft:absorption", -1, amplifier);
            assert_eq!(max_absorption(&mob), java, "base {base}");
            assert_eq!(
                finalize::base_value(&mob, Attr::MaxAbsorption),
                base,
                "the stored base is unchanged"
            );
        }
    }

    fn magma_cube(size: i32) -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = "minecraft:magma_cube".into();
        ty.species = Species::MagmaCube;
        let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
        crate::mobs::cube::set_size(&mut mob, size);
        mob
    }

    /// MagmaCube.setSize writes `size * 3` into `Attributes.ARMOR`, and
    /// `LivingEntity.getArmorValue` reads that base back (no armor map needed).
    #[test]
    fn magma_cube_armor_is_its_size_times_three_and_softens_hits() {
        for size in [2, 4] {
            assert_eq!(
                values(&magma_cube(size)),
                ((size * 3) as f32, 0.0),
                "size {size}"
            );
        }
        let mut small = magma_cube(1);
        let mut big = magma_cube(4);
        let hit = Damage::of(DamageKind::Cactus);
        let small_before = small.health;
        let big_before = big.health;
        assert!(small.damage(&mut crate::health::TestLevel::new(0).level(), 0.5, hit));
        assert!(big.damage(&mut crate::health::TestLevel::new(0).level(), 0.5, hit));
        assert!(
            big_before - big.health < small_before - small.health,
            "size 4 lost {} and size 1 lost {}",
            big_before - big.health,
            small_before - small.health
        );
    }
}
