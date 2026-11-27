//! #478: a mob's effect attribute modifiers reach the attribute packet as the
//! stored modifiers they are, once, and a loaded effect (which stores none)
//! sends none.
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay::game::server_side::lan_attributes::SpeedByType;
use sunrise_protocol::packets::play::{AttributeModifier, AttributeSnapshot};
use sunrise_sim::mobs::{Mob, StoredEffect};

#[test]
#[ignore = "daily"]
fn the_attribute_packet_carries_a_given_effect_modifier_once() {
    with_game(|game| {
        let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        let mut given = Mob::new(12, pig.clone(), Vec3::ZERO, 0.0);
        given.add_effect("minecraft:speed", -1, 1);
        let mut loaded = Mob::new(13, pig, Vec3::ZERO, 0.0);
        loaded.load_effect(StoredEffect::new("minecraft:speed", -1, 1));
        let speed = game
            .motion
            .attributes
            .id("minecraft:movement_speed")
            .unwrap()
            .registry_id();
        let synced = |m: &Mob| -> Vec<AttributeSnapshot> {
            let mut out = Vec::new();
            game.mob_attributes(m, &mut SpeedByType::default(), &mut out);
            out.into_iter().filter(|s| s.attribute == speed).collect()
        };
        // One movement speed instance: the pig's 0.25 and the stored
        // `minecraft:effect.speed`, 0.2F x 2, add_multiplied_total (2).
        let given = synced(&given);
        assert_eq!(given.len(), 1, "{given:?}");
        assert_eq!(given[0].base, 0.25);
        assert_eq!(
            given[0].modifiers,
            [AttributeModifier {
                id: "minecraft:effect.speed".into(),
                amount: 0.400_000_005_960_464_5,
                operation: 2,
            }]
        );
        // A loaded effect has no modifier to send.
        let loaded = synced(&loaded);
        assert_eq!(loaded.len(), 1, "{loaded:?}");
        assert_eq!(loaded[0].base, 0.25);
        assert!(loaded[0].modifiers.is_empty(), "{loaded:?}");
    });
}

/// A spawned baby zombie given Speed II: the packet lists `minecraft:baby`
/// (`setBaby` at spawn) before `minecraft:effect.speed` (`onEffectAdded`
/// later), the instance's insertion order, each once.
#[test]
#[ignore = "daily"]
fn a_babys_modifier_comes_before_a_given_effects_in_the_packet() {
    with_game(|game| {
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut baby = Mob::new(12, zombie, Vec3::ZERO, 0.0);
        sunrise_sim::mobs::finalize::set_baby(&mut baby, true);
        baby.add_effect("minecraft:speed", -1, 1);
        let speed = game
            .motion
            .attributes
            .id("minecraft:movement_speed")
            .unwrap()
            .registry_id();
        let mut out = Vec::new();
        game.mob_attributes(&baby, &mut SpeedByType::default(), &mut out);
        let rows: Vec<_> = out.iter().filter(|s| s.attribute == speed).collect();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(
            rows[0].modifiers,
            [
                AttributeModifier {
                    id: "minecraft:baby".into(),
                    amount: 0.5,
                    operation: 1,
                },
                AttributeModifier {
                    id: "minecraft:effect.speed".into(),
                    amount: 0.400_000_005_960_464_5,
                    operation: 2,
                },
            ]
        );
    });
}

/// A cat given Speed II that then dashes: `setSprinting` removes and adds
/// `minecraft:sprinting` each time the dash starts, so it comes after the
/// effect's modifier, each once.
#[test]
#[ignore = "daily"]
fn a_dash_after_a_given_effect_comes_after_it_in_the_packet() {
    with_game(|game| {
        let kind = game.res.mob_types().get("minecraft:cat").unwrap().clone();
        let mut cat = Mob::new(12, kind, Vec3::ZERO, 0.0);
        cat.add_effect("minecraft:speed", -1, 1);
        cat.extra.sprinting = true;
        let speed = game
            .motion
            .attributes
            .id("minecraft:movement_speed")
            .unwrap()
            .registry_id();
        let mut out = Vec::new();
        game.mob_attributes(&cat, &mut SpeedByType::default(), &mut out);
        let rows: Vec<_> = out.iter().filter(|s| s.attribute == speed).collect();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(
            rows[0].modifiers,
            [
                AttributeModifier {
                    id: "minecraft:effect.speed".into(),
                    amount: 0.400_000_005_960_464_5,
                    operation: 2,
                },
                AttributeModifier {
                    id: "minecraft:sprinting".into(),
                    amount: 0.300_000_011_920_928_96,
                    operation: 2,
                },
            ]
        );
    });
}

/// #1988: a zombie given Jump Boost II and Absorption II sends each stored
/// modifier once, on its attribute's instance (base 3 and 0): 2.0 and 8.0,
/// `add_value` (0). A zombie that loaded the same effects stored none, and
/// no instance of either is sent for it. `waypoint_transmit_range` is not
/// syncable, so Invisibility's modifier is never sent.
#[test]
#[ignore = "daily"]
fn the_attribute_packet_carries_given_jump_boost_and_absorption_modifiers_once() {
    with_game(|game| {
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut given = Mob::new(12, zombie.clone(), Vec3::ZERO, 0.0);
        let mut loaded = Mob::new(13, zombie, Vec3::ZERO, 0.0);
        for (effect, amplifier) in [
            ("minecraft:jump_boost", 1),
            ("minecraft:absorption", 1),
            ("minecraft:invisibility", 0),
        ] {
            given.add_effect(effect, -1, amplifier);
            loaded.load_effect(StoredEffect::new(effect, -1, amplifier));
        }
        let id = |name: &str| game.motion.attributes.id(name).unwrap().registry_id();
        let (safe_fall, max_absorption, range) = (
            id("minecraft:safe_fall_distance"),
            id("minecraft:max_absorption"),
            id("minecraft:waypoint_transmit_range"),
        );
        let synced = |m: &Mob| -> Vec<AttributeSnapshot> {
            let mut out = Vec::new();
            game.mob_attributes(m, &mut SpeedByType::default(), &mut out);
            out
        };
        let rows = synced(&given);
        let row = |attribute: i32| {
            let found: Vec<_> = rows.iter().filter(|s| s.attribute == attribute).collect();
            assert_eq!(found.len(), 1, "{rows:?}");
            found[0].clone()
        };
        let jump = row(safe_fall);
        assert_eq!(jump.base, 3.0);
        assert_eq!(
            jump.modifiers,
            [AttributeModifier {
                id: "minecraft:effect.jump_boost".into(),
                amount: 2.0,
                operation: 0,
            }]
        );
        let absorption = row(max_absorption);
        assert_eq!(absorption.base, 0.0);
        assert_eq!(
            absorption.modifiers,
            [AttributeModifier {
                id: "minecraft:effect.absorption".into(),
                amount: 8.0,
                operation: 0,
            }]
        );
        assert!(rows.iter().all(|s| s.attribute != range), "{rows:?}");
        let rows = synced(&loaded);
        assert!(
            rows.iter()
                .all(|s| ![safe_fall, max_absorption, range].contains(&s.attribute)),
            "{rows:?}"
        );
    });
}

/// #4473: a baby zombie given a content mod's effect whose declared modifier
/// is named `minecraft:baby` replaces the baby speed bonus, as `/attribute`'s
/// `modifier remove minecraft:baby` would first: the packet sends the
/// effect's modifier alone, once, and the speed is the base plus a tenth of it.
#[test]
#[ignore = "daily"]
fn a_declared_baby_modifier_replaces_the_bonus_in_the_packet() {
    use sunrise_game_data::potions::{
        Category, ModEffect, effect, isolate_mod_effects, register_mod_effects,
    };
    use sunrise_sim::mobs::finalize::{self, Attr};
    with_game(|game| {
        let _scope = isolate_mod_effects();
        assert!(register_mod_effects([ModEffect {
            modifiers: vec![(
                "minecraft:movement_speed".into(),
                "minecraft:baby".into(),
                0.1,
                sunrise_world::attributes::Operation::AddMultipliedBase,
            )],
            ..ModEffect::new("testfx:grown", 4, Category::Neutral, "testfx")
        }]));
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut baby = Mob::new(12, zombie, Vec3::ZERO, 0.0);
        finalize::set_baby(&mut baby, true);
        let grown = effect("testfx:grown").unwrap().id;
        baby.add_effect(grown, -1, 0);
        let base = finalize::base_value(&baby, Attr::MovementSpeed);
        assert_eq!(baby.movement_speed(), base + base * 0.1);
        let speed = game
            .motion
            .attributes
            .id("minecraft:movement_speed")
            .unwrap()
            .registry_id();
        let mut out = Vec::new();
        game.mob_attributes(&baby, &mut SpeedByType::default(), &mut out);
        let rows: Vec<_> = out.iter().filter(|s| s.attribute == speed).collect();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(
            rows[0].modifiers,
            [AttributeModifier {
                id: "minecraft:baby".into(),
                amount: 0.1,
                operation: 1,
            }]
        );
    });
}

/// #4473: a transaction that gave a baby zombie an effect whose declared
/// modifier is named `minecraft:baby`, then aborted (its effect journal
/// restored): the baby speed bonus is back, in the speed and in the packet,
/// once.
#[test]
#[ignore = "daily"]
fn an_undone_declared_baby_modifier_brings_the_bonus_back_in_the_packet() {
    use sunrise_game_data::potions::{
        Category, ModEffect, effect, isolate_mod_effects, register_mod_effects,
    };
    use sunrise_sim::mobs::finalize::{self, Attr};
    with_game(|game| {
        let _scope = isolate_mod_effects();
        assert!(register_mod_effects([ModEffect {
            modifiers: vec![(
                "minecraft:movement_speed".into(),
                "minecraft:baby".into(),
                0.1,
                sunrise_world::attributes::Operation::AddMultipliedBase,
            )],
            ..ModEffect::new("testfx:grown", 4, Category::Neutral, "testfx")
        }]));
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut baby = Mob::new(12, zombie, Vec3::ZERO, 0.0);
        finalize::set_baby(&mut baby, true);
        let grown = effect("testfx:grown").unwrap().id;
        let effects = baby.extra.effects.clone();
        let undo = baby
            .effect_modifier_undo(std::iter::once(grown))
            .expect("the effect names the baby bonus");
        baby.add_effect(grown, -1, 0);
        baby.extra.effects = effects;
        undo.restore(&mut baby);
        let base = finalize::base_value(&baby, Attr::MovementSpeed);
        assert_eq!(baby.movement_speed(), base + base * 0.5);
        let speed = game
            .motion
            .attributes
            .id("minecraft:movement_speed")
            .unwrap()
            .registry_id();
        let mut out = Vec::new();
        game.mob_attributes(&baby, &mut SpeedByType::default(), &mut out);
        let rows: Vec<_> = out.iter().filter(|s| s.attribute == speed).collect();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(
            rows[0].modifiers,
            [AttributeModifier {
                id: "minecraft:baby".into(),
                amount: 0.5,
                operation: 1,
            }]
        );
    });
}
