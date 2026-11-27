//! Living content types built through `mod_content::resource_root` and ticked
//! in a mob world: class behaviour follows the template, tags follow the type id.
use super::world::test_player;
use super::{Mob, MobEvent, MobTypes, MobWorld, Species, Who};
use mod_api::Side;
use std::{path::PathBuf, sync::Arc};
use sunrise_core::geometry::Vec3;
use sunrise_mod_package::{Package, content::Content, enabled::EnabledMods};

/// Load the mod `ruby` with these files in a world whose chunks hold `build`'s blocks.
fn fixture(
    files: &[(&str, &[u8])],
    build: impl FnOnce(&mut crate::world::World, &sunrise_world::registry::BlockRegistry),
    run: impl FnOnce(&MobWorld, &MobTypes),
) {
    let base = std::env::var_os("SUNRISE_TEST_RESOURCES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.local/sunrise-client/26.3")
        });
    assert!(
        base.join("entity-types.json").exists(),
        "prepared resources required"
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ruby.smod");
    let manifest = "id = \"ruby\"\nname = \"Ruby\"\nversion = \"1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n";
    std::fs::write(&path, Package::write_files(manifest, files).unwrap()).unwrap();
    let package = Package::read(&path).unwrap();
    let mods = [crate::mod_content::ContentMod {
        id: "ruby".into(),
        content: Content::read(&path, &package.manifest).unwrap(),
        jars: vec![],
    }];
    let inventory = EnabledMods::new([("ruby".into(), Side::Both)]);
    let root = crate::mod_content::resource_root(&base, dir.path(), &mods, dir.path(), &inventory)
        .unwrap();
    assert!(crate::mobs::world::with_resource_test_world(
        &root,
        &[],
        true,
        true,
        build,
        run
    ));
}

/// Chunks over `0..32` in x and z, a stone floor at y 63, and `fill`'s blocks.
fn arena(
    world: &mut crate::world::World,
    registry: &sunrise_world::registry::BlockRegistry,
    fill: impl Fn(i32, i32, i32) -> &'static str,
) {
    for cx in 0..2 {
        for cz in 0..2 {
            world
                .chunks
                .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
        }
    }
    for x in 0..32 {
        for z in 0..32 {
            for y in 56..=66 {
                let state = registry.default_state(fill(x, y, z)).unwrap();
                world.set(x, y, z, state).unwrap();
            }
        }
    }
}

fn mob(types: &MobTypes, id: &str, at: Vec3) -> Mob {
    let mut mob = Mob::new(1, Arc::new(types.types[id].clone()), at, 0.0);
    mob.ground.on_ground = true;
    mob
}

fn goal_names(m: &Mob, names: &[&str]) -> Vec<bool> {
    names.iter().map(|n| m.ai.goals.has(n)).collect()
}

/// A mod type on the skeleton holding a bow, with a player in sight, draws it
/// and looses a real arrow, as the skeleton does; it also has the skeleton's
/// sun and wolf goals, its class and its drawn model.
#[test]
#[ignore = "daily"]
fn a_mod_type_on_the_skeleton_draws_and_shoots_an_arrow() {
    fixture(
        &[(
            "data/ruby/sunrise/entity_type/archer.json",
            br#"{"template":"minecraft:skeleton"}"#,
        )],
        |world, registry| {
            arena(world, registry, |_, y, _| match y {
                ..=63 => "minecraft:stone",
                _ => "minecraft:air",
            })
        },
        |w, types| {
            let (archer, vanilla) = (
                &types.types["ruby:archer"],
                &types.types["minecraft:skeleton"],
            );
            assert_eq!(archer.species, Species::Skeleton);
            assert_eq!(archer.class_id(), "minecraft:skeleton");
            assert_eq!(archer.model_name(), vanilla.model_name());
            assert_eq!(
                super::species::def(archer.species).texture,
                super::species::def(vanilla.species).texture
            );
            assert_eq!(archer.sounds, vanilla.sounds);
            assert_eq!(archer.max_health, vanilla.max_health);

            let mut w = w.clone();
            w.player = Some(test_player(Vec3::new(12.5, 64.0, 0.5)));
            let mut m = mob(types, "ruby:archer", Vec3::new(0.5, 64.0, 0.5));
            m.extra.set_main_hand(Some("minecraft:bow".into()));
            let mut test_level = crate::health::TestLevel::new(0);
            let level = &mut test_level.level();
            let (mut drew, mut shot) = (false, None);
            let mut shots_at = 0;
            for tick in 0..400 {
                let mut events = Vec::new();
                m.tick(&w, level, &mut events);
                drew |= m.extra.using_item > 0;
                for e in events {
                    if let MobEvent::BowShot { mob, arrow, weapon } = e {
                        assert_eq!((mob, weapon.item.as_ref()), (1, "minecraft:bow"));
                        shot.get_or_insert(arrow);
                        shots_at = tick;
                    }
                }
                if shot.is_some() {
                    break;
                }
            }
            assert!(drew, "it draws the bow");
            let arrow = shot.unwrap_or_else(|| panic!("no arrow within 400 ticks"));
            assert!(arrow.arrow.is_some(), "the shot is an arrow entity");
            assert_eq!(arrow.owner, Some(Who::Mob(1)));
            assert!(shots_at > 0);
            assert_eq!(m.ai.target, Some(Who::Player));

            // The skeleton's sun and wolf goals, and the weapon goal that is
            // the bow attack while a bow is held.
            assert_eq!(
                goal_names(&m, &["RestrictSun", "FleeSun", "AvoidEntity", "BowAttack"]),
                [true; 4]
            );
        },
    );
}

/// A mod type on the drowned in a pool, with a player in the water beside it,
/// swims as the drowned does, with the drowned's water navigation and fight goals.
#[test]
#[ignore = "daily"]
fn a_mod_type_on_the_drowned_swims_and_fights_in_water() {
    fixture(
        &[(
            "data/ruby/sunrise/entity_type/diver.json",
            br#"{"template":"minecraft:drowned"}"#,
        )],
        |world, registry| {
            arena(world, registry, |x, y, z| match y {
                ..=55 => "minecraft:stone",
                56..=62 if (1..30).contains(&x) && (1..30).contains(&z) => "minecraft:water",
                56..=63 => "minecraft:stone",
                _ => "minecraft:air",
            })
        },
        |w, types| {
            let (diver, vanilla) = (
                &types.types["ruby:diver"],
                &types.types["minecraft:drowned"],
            );
            assert_eq!(diver.species, Species::Drowned);
            assert_eq!(diver.class_id(), "minecraft:drowned");
            assert_eq!(diver.model_name(), vanilla.model_name());
            assert_eq!(
                super::species::def(diver.species).texture,
                super::species::def(vanilla.species).texture
            );

            let mut w = w.clone();
            let player = Vec3::new(6.5, 58.0, 5.5);
            w.player = Some(test_player(player));
            let run = |id: &str| {
                let mut m = mob(types, id, Vec3::new(5.5, 58.0, 5.5));
                m.ground.on_ground = false;
                let mut test_level = crate::health::TestLevel::new(0);
                let level = &mut test_level.level();
                let mut swam = false;
                for _ in 0..200 {
                    let mut events = Vec::new();
                    m.tick(&w, level, &mut events);
                    swam |= m.extra.flag;
                }
                (m, swam)
            };
            let (m, swam) = run("ruby:diver");
            assert!(m.fluids.in_water, "it stays in the water");
            assert!(swam, "it swims (Drowned.setSwimming)");
            assert_eq!(m.ai.target, Some(Who::Player));
            let (vanilla_mob, vanilla_swam) = run("minecraft:drowned");
            assert_eq!(swam, vanilla_swam);
            assert_eq!(m.ai.nav.kind, vanilla_mob.ai.nav.kind, "its navigation");
            assert!(matches!(
                m.ai.nav.kind,
                super::nav::NavKind::Amphibious { .. }
            ));
            let names = [
                "DrownedGoToWater",
                "DrownedTridentAttackGoal",
                "DrownedAttackGoal",
                "DrownedGoToBeachGoal",
                "DrownedSwimUpGoal",
            ];
            assert_eq!(goal_names(&m, &names), vec![true; names.len()]);
        },
    );
}

#[test]
#[ignore = "daily"]
fn sunlight_ignition_follows_entity_type_tags() {
    fixture(
        &[
            (
                "data/ruby/sunrise/entity_type/untagged.json",
                br#"{"template":"minecraft:zombie"}"#,
            ),
            (
                "data/ruby/sunrise/entity_type/tagged.json",
                br#"{"template":"minecraft:zombie"}"#,
            ),
            (
                "data/minecraft/tags/entity_type/burn_in_daylight.json",
                br#"{"values":["ruby:tagged"]}"#,
            ),
        ],
        |world, registry| {
            arena(world, registry, |_, y, _| match y {
                ..=63 => "minecraft:stone",
                _ => "minecraft:air",
            });
        },
        |w, types| {
            let mut day = w.clone();
            day.day_time = 6000;
            day.monsters_burn = true;
            for (id, burns) in [
                ("ruby:untagged", false),
                ("ruby:tagged", true),
                ("minecraft:zombie", true),
                ("minecraft:skeleton", true),
                ("minecraft:husk", false),
            ] {
                let mut m = mob(types, id, Vec3::new(8.5, 64.0, 8.5));
                m.no_ai = true;
                m.random = sunrise_core::random::LegacyRandom::new(3);
                let mut test_level = crate::health::TestLevel::new(0);
                let level = &mut test_level.level();
                let mut events = Vec::new();
                let ignited = (0..500).any(|_| {
                    m.tick(&day, level, &mut events);
                    m.is_on_fire()
                });
                assert_eq!(ignited, burns, "daylight ignition of {id}");

                if id == "ruby:untagged" {
                    let health = m.health;
                    m.ignite_for_ticks(20);
                    assert!(m.fire_ticks > 0);
                    m.tick(&day, level, &mut events);
                    assert!(m.is_on_fire());
                    assert!(m.health < health, "ordinary fire still damages {id}");
                }
            }
        },
    );
}
