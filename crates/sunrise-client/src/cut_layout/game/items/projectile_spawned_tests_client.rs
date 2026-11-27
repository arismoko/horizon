use crate::game::server_side::projectile_spawned_tests::enchanted;
use crate::game::server_side::projectile_spawned_tests::ignite;
use crate::game::server_side::projectile_spawned_tests::sound;
use crate::game::server_side::projectile_spawned_tests::with_enchantments;
use serde_json::json;
use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};
use sunrise_items::items::ItemStack;
use sunrise_sim::{
    mobs::Who,
    projectiles::{Projectile, ProjectileKind},
};

/// The sounds played since capture began, as (event, volume, pitch).
pub(crate) fn played(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> Vec<(String, f32, f32)> {
    std::mem::take(&mut game.server.pending_sounds)
        .into_iter()
        .map(|s| {
            assert_eq!(s.source, mod_api::SoundSource::Neutral as usize);
            (s.event, s.volume, s.pitch)
        })
        .collect()
}

/// Each effect's requirements are tested with the level random just before
/// it applies, and only a matching effect draws from the projectile's random:
/// a coin-flip sound draws twice when it plays and not at all when it
/// doesn't, and the next effect's draws follow on.
#[test]
#[ignore = "daily"]
fn requirements_draw_the_level_random_before_each_effect() {
    let coin = json!({"condition": "minecraft:random_chance", "chance": 0.5});
    let effects = json!([
        {"requirements": coin, "effect": sound("minecraft:entity.arrow.shoot")},
        {"effect": sound("minecraft:entity.snowball.throw")},
        // Arrows only: the projectile is the context's `this`.
        {"requirements": {"condition": "minecraft:entity_properties", "entity": "this",
            "predicate": {"type": "minecraft:arrow"}}, "effect": ignite(3.0)},
    ]);
    with_enchantments(&[("coin", effects)], |game| {
        let stack = enchanted("minecraft:snowball", 1, &[("spawn_test:coin", 1)]);
        let mut both = false;
        let mut neither = false;
        for seed in 0..32_i64 {
            let mut level = game.world.random.clone();
            // LootItemRandomChanceCondition: nextFloat() < chance.
            let flipped = level.next_float() < 0.5;
            let mut own = LegacyRandom::new(seed);
            // UniformFloat.sample: nextFloat() * (max - min) + min, volume first.
            let mut uniform = |min: f32, max: f32| own.next_float() * (max - min) + min;
            let mut expected = Vec::new();
            if flipped {
                let (v, p) = (uniform(0.5, 1.0), uniform(1.0, 2.0));
                expected.push(("minecraft:entity.arrow.shoot".to_owned(), v, p));
            }
            let (v, p) = (uniform(0.5, 1.0), uniform(1.0, 2.0));
            expected.push(("minecraft:entity.snowball.throw".to_owned(), v, p));
            let after = own.next_long();
            game.start_audio_capture();
            let shot = Projectile::new(ProjectileKind::Snowball, Vec3::new(0.5, 70.0, 0.5), None)
                .with_entity_random(LegacyRandom::new(seed));
            let id = game.spawn_projectile(shot, &stack).unwrap();
            assert_eq!(played(game), expected, "seed {seed}");
            let p = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)
                .unwrap();
            assert_eq!(p.random.0.clone().next_long(), after, "seed {seed}");
            // The entity_properties requirement draws nothing and fails.
            assert_eq!(p.fire_ticks, 0);
            assert_eq!(game.world.random.next_long(), level.next_long());
            (both, neither) = (both || flipped, neither || !flipped);
        }
        assert!(both && neither, "the coin landed both ways");
    });
}

/// The stack's effects run before the weapon's, in the stack's enchantment
/// order and each enchantment's effect order; an arrow fired from its own
/// item runs that item's effects once.
#[test]
#[ignore = "daily"]
fn stack_then_weapon_in_component_order() {
    let defs = [
        (
            "first",
            json!([{"effect": {"type": "minecraft:all_of", "effects": [
                sound("minecraft:block.note_block.bell"),
                // On a projectile: nothing, and no draws.
                {"type": "minecraft:apply_mob_effect", "to_apply": "minecraft:speed",
                    "min_duration": 1, "max_duration": 9, "min_amplifier": 0, "max_amplifier": 3},
                ignite(2.0),
            ]}}]),
        ),
        (
            "second",
            json!([{"effect": sound("minecraft:block.note_block.chime")}]),
        ),
        (
            "weapon",
            json!([
                {"effect": sound("minecraft:block.note_block.flute")},
                {"effect": ignite(5.0)},
            ]),
        ),
    ];
    with_enchantments(&defs, |game| {
        let ammo = enchanted(
            "minecraft:arrow",
            1,
            &[("spawn_test:first", 1), ("spawn_test:second", 1)],
        );
        let bow = enchanted("minecraft:bow", 1, &[("spawn_test:weapon", 1)]);
        let arrow = |weapon: &ItemStack| {
            let mut p = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::new(0.5, 70.0, 0.5),
                Some(Who::Player),
            )
            .with_entity_random(LegacyRandom::new(7));
            let a = p.arrow.as_mut().unwrap();
            a.set_item(&ammo);
            a.fired_from_weapon = Some(weapon.clone());
            p
        };
        let names = |sounds: Vec<(String, f32, f32)>| -> Vec<String> {
            sounds.into_iter().map(|(name, _, _)| name).collect()
        };
        let order: Vec<String> = ammo
            .enchantments()
            .iter()
            .map(|(id, _)| match &**id {
                "spawn_test:first" => "minecraft:block.note_block.bell",
                _ => "minecraft:block.note_block.chime",
            })
            .chain(["minecraft:block.note_block.flute"])
            .map(str::to_owned)
            .collect();
        game.start_audio_capture();
        let id = game.spawn_projectile(arrow(&bow), &ammo).unwrap();
        assert_eq!(names(played(game)), order);
        let p = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id == id)
            .unwrap();
        // igniteForTicks keeps the longer fire: the weapon's 5 s.
        assert_eq!(p.arrow.as_ref().unwrap().fire_ticks, 100);
        // The weapon is the ammo's own item: its effects run once.
        let id = game.spawn_projectile(arrow(&ammo), &ammo).unwrap();
        assert_eq!(names(played(game)).len(), 2);
        let p = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id == id)
            .unwrap();
        assert_eq!(p.arrow.as_ref().unwrap().fire_ticks, 40);
    });
}
