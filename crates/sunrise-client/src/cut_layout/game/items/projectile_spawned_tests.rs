//! `Projectile.applyOnProjectileSpawned` through `Game::spawn_projectile` and
//! the mod host, with enchantments from a world datapack. Expected values come
//! from vanilla's formulas run on copies of the randoms involved.
#![cfg(test)]
use mod_api::Identifier;
use serde_json::{Value, json};
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::tables::GameItems;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::Mob;

/// A world whose datapack defines `spawn_test:<name>` for each
/// `(name, projectile_spawned effects)`.
pub(crate) fn with_enchantments(
    defs: &[(&str, Value)],
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    with_game(|game| {
        let root = std::env::temp_dir().join(format!(
            "sunrise-projectile-spawned-{}-{}",
            std::process::id(),
            defs.iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join("-")
        ));
        let pack = root.join("datapacks/spawned");
        let definitions = pack.join("data/spawn_test/enchantment");
        std::fs::create_dir_all(&definitions).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"projectile_spawned fixture"}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("datapacks.json"),
            r#"{"enabled":["file/spawned"],"disabled":[]}"#,
        )
        .unwrap();
        for (name, effects) in defs {
            let definition = json!({
                "description": "fixture", "supported_items": "#minecraft:enchantable/bow",
                "weight": 1, "max_level": 3,
                "min_cost": {"base": 1, "per_level_above_first": 0},
                "max_cost": {"base": 1, "per_level_above_first": 0},
                "anvil_cost": 1, "slots": ["mainhand"],
                "effects": {"minecraft:projectile_spawned": effects},
            });
            std::fs::write(
                definitions.join(format!("{name}.json")),
                serde_json::to_vec(&definition).unwrap(),
            )
            .unwrap();
        }
        game.server.world_root = root.clone();
        let stack = game.reload_datapacks(false, &[]).unwrap();
        let resources = game.options.resources.clone();
        game.server.world_tables.0 = game
            .stage_world_tables(&stack, false, &resources)
            .unwrap()
            .tables;
        for (name, _) in defs {
            assert!(
                game.stations()
                    .enchantments
                    .get(&format!("spawn_test:{name}"))
                    .is_some(),
                "{name} did not load"
            );
        }
        test(game);
        let _ = std::fs::remove_dir_all(root);
    });
}

pub(crate) fn enchanted(item: &str, count: u32, levels: &[(&str, u32)]) -> ItemStack {
    let mut stack = ItemStack::new(item, count);
    stack.data_mut().enchantments = levels
        .iter()
        .map(|(id, level)| ((*id).into(), *level))
        .collect();
    stack
}

#[cfg(feature = "client")]
pub(crate) fn sound(name: &str) -> Value {
    json!({"type": "minecraft:play_sound", "sound": name,
        "volume": {"type": "minecraft:uniform", "min_inclusive": 0.5, "max_exclusive": 1.0},
        "pitch": {"type": "minecraft:uniform", "min_inclusive": 1.0, "max_exclusive": 2.0}})
}

pub(crate) fn ignite(seconds: f32) -> Value {
    json!({"type": "minecraft:ignite", "duration": seconds})
}

/// The mod host's owner launch applies the supplied stack's effects once,
/// whatever its count, leaves the stack alone, and refuses a stack with an
/// effect the engine does not apply yet before drawing anything.
#[test]
#[ignore = "daily"]
fn owner_launch_applies_the_supplied_stacks_effects() {
    let defs = [
        ("hot", json!([{"effect": ignite(4.0)}])),
        (
            "flags",
            json!([{"requirements": {"condition":"minecraft:entity_properties", "entity":"this", "predicate":{"minecraft:flags":{"is_on_fire":true}}}, "effect": ignite(4.0)}]),
        ),
        (
            "bad_clamp",
            json!([{"effect":{"type":"minecraft:ignite", "duration":{"type":"minecraft:clamped", "value":1,"min":2,"max":0}}}]),
        ),
        (
            "boom",
            json!([{"effect": {"type": "minecraft:explode", "radius": 1,
            "block_interaction": "none", "small_particle": {"type": "minecraft:explosion"},
            "large_particle": {"type": "minecraft:explosion_emitter"},
            "sound": "minecraft:entity.generic.explode"}}]),
        ),
    ];
    with_enchantments(&defs, |game| {
        sunrise_gameplay_common::game::test_fixtures::fixture_resident_column(
            &mut game.world,
            (0, 0),
        );
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.mobs
            .push(Mob::new(21, kind, Vec3::new(0.0, 64.0, 0.0), 0.0));
        let source = game.entity(21).unwrap();
        let owner = game.living(source).unwrap();
        let snowball = Identifier::parse("minecraft:snowball").unwrap();
        let stack = enchanted("minecraft:snowball", 16, &[("spawn_test:hot", 2)]);
        let shot = game
            .spawn_projectile_from_rotation(owner, &snowball, &stack, 0.0, 1.5, 1.0)
            .unwrap();
        let p = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id == shot.entity().id())
            .unwrap();
        // Ignite(4): 80 ticks, once for the stack, not per item in it.
        assert_eq!(p.fire_ticks, 80);
        assert_eq!(p.item.as_ref().unwrap().count, 1);
        assert_eq!(stack.count, 16);
        let boom = enchanted("minecraft:snowball", 1, &[("spawn_test:boom", 1)]);
        let count = game.creatures.projectiles.len();
        let mut before = game.world.random.clone();
        assert!(
            game.spawn_projectile_from_rotation(owner, &snowball, &boom, 0.0, 1.5, 1.0)
                .is_none()
        );
        assert_eq!(game.creatures.projectiles.len(), count);
        assert_eq!(game.world.random.next_long(), before.next_long());
        for enchantment in ["spawn_test:flags", "spawn_test:bad_clamp"] {
            let invalid = enchanted("minecraft:snowball", 1, &[(enchantment, 1)]);
            let mut before = game.world.random.clone();
            assert!(
                game.spawn_projectile_from_rotation(owner, &snowball, &invalid, 0.0, 1.5, 1.0)
                    .is_none()
            );
            assert_eq!(game.creatures.projectiles.len(), count);
            assert_eq!(game.world.random.next_long(), before.next_long());
        }
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
