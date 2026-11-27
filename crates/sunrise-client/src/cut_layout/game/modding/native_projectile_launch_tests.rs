//! Owner launch boundary regression with vanilla's deterministic sequence.
#![cfg(test)]
use mod_api::Identifier;
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_gameplay::game::native_entity_refs::tests::with_resident_game;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::Mob;

#[test]
#[ignore = "daily"]
fn owner_launch_sets_owner_before_insertion_and_uses_projectile_random() {
    // Opt in only in a fresh child; never mutate the parallel parent's environment.
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::server_side::native_projectile_launch_tests::owner_launch_sets_owner_before_insertion_and_uses_projectile_random",
        "SUNRISE_TEST_SEEDS",
        &["1"],
    ) {
        return;
    }
    with_resident_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut mob = Mob::new(21, kind, Vec3::new(0.0, 64.0, 0.0), 0.0);
        mob.previous = Vec3::new(-0.2, 63.7, 0.4);
        mob.ground.on_ground = false;
        game.mobs.push(mob);
        let source = game.entity(21).unwrap();
        let owner = game.living(source).unwrap();
        game.entity_uuid(source).unwrap();
        let mut level_before = game.world.random.clone();
        let stack = ItemStack::new("minecraft:snowball", 64);
        let eye = game.entity_eye_position(source).unwrap();
        let entity_type = Identifier::parse("minecraft:snowball").unwrap();
        // This thread's next unique seed is 1234 (odd multiplier inverse),
        // independent of world RNG and any draws during resource loading.
        sunrise_level::random_source::set_thread_uniquifier(2709830126447559882).unwrap();
        let shot = game
            .spawn_projectile_from_rotation(owner, &entity_type, &stack, 0.0, 1.5, 1.0)
            .unwrap();
        assert_eq!(game.projectile_owner(shot), Some(source));
        let projectile = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id == shot.entity().id())
            .unwrap();
        assert_eq!(projectile.item.as_ref().unwrap().count, 1);
        assert_eq!(stack.count, 64);
        assert_eq!(projectile.position.y, eye[1] - 0.10000000149011612);
        // Computed with an independent implementation of the documented
        // java.util.Random algorithm, not from the game's output: seed 1234,
        // four next(32) advances for the UUID's two nextLong calls, then
        // twelve advances for the shot's six nextDouble calls. The following
        // double uses advances 17 and 18. Include the owner's inherited motion.
        assert!((projectile.velocity.x - 0.2103198363925326).abs() < 1e-12);
        assert!((projectile.velocity.y - 0.3034130508130313).abs() < 1e-12);
        assert!((projectile.velocity.z - 1.093012354329919).abs() < 1e-12);
        let mut random = projectile.random.0.clone();
        assert_eq!(random.next_double(), 0.9468595742485053);
        assert_eq!(game.world.random.next_long(), level_before.next_long());
        let count = game.creatures.projectiles.len();
        assert!(
            game.spawn_projectile_from_rotation(owner, &entity_type, &stack, 0.0, f32::NAN, 1.0)
                .is_none()
        );
        let unknown = Identifier::parse("test:unknown").unwrap();
        assert!(
            game.spawn_projectile_from_rotation(owner, &unknown, &stack, 0.0, 1.5, 1.0)
                .is_none()
        );
        assert_eq!(game.creatures.projectiles.len(), count);
        // Flame's projectile_spawned Ignite(100) runs once the snowball is in
        // the level: 100 seconds, 2000 ticks.
        let mut enchanted = stack.clone();
        enchanted.enchant("minecraft:flame", 1);
        let shot = game
            .spawn_projectile_from_rotation(owner, &entity_type, &enchanted, 0.0, 1.5, 1.0)
            .unwrap();
        let projectile = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id == shot.entity().id())
            .unwrap();
        assert_eq!(projectile.fire_ticks, 2000);
        assert_eq!(enchanted.count, 64);
        let potion = ItemStack::new("minecraft:splash_potion", 16);
        let potion_type = Identifier::parse("minecraft:splash_potion").unwrap();
        let shot = game
            .spawn_projectile_from_rotation(owner, &potion_type, &potion, 0.0, 1.5, 1.0)
            .unwrap();
        let projectile = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id == shot.entity().id())
            .unwrap();
        match &projectile.kind {
            sunrise_sim::projectiles::ProjectileKind::Potion(stack, false) => {
                assert_eq!(stack.count, 1)
            }
            _ => panic!("expected splash potion payload"),
        }
        assert_eq!(potion.count, 16);
        let wind = Identifier::parse("minecraft:wind_charge").unwrap();
        let wind_stack = ItemStack::new("minecraft:wind_charge", 1);
        let shot = game
            .spawn_projectile_from_rotation(owner, &wind, &wind_stack, 0.0, 1.5, 1.0)
            .unwrap();
        assert_eq!(game.entity_position(shot.entity()).unwrap()[1], eye[1]);
    });
}

#[test]
#[ignore = "daily"]
fn owner_launch_reports_explicit_guest_to_join_listeners() {
    // Opt in only in a fresh child; never mutate the parallel parent process's environment.
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::server_side::native_projectile_launch_tests::owner_launch_reports_explicit_guest_to_join_listeners",
        "SUNRISE_TEST_SEEDS",
        &["1"],
    ) {
        return;
    }
    use mod_api::{EntityJoinLevelEvent, Flow, Hooks, ModEntry, ServerHost};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    struct Observe {
        owners: Arc<Mutex<Vec<Option<u128>>>>,
        cancel: Arc<AtomicBool>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        fn entity_join_level(
            &self,
            host: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            if event.kind == "minecraft:snowball" {
                let projectile = host.projectile(event.subject).unwrap();
                let owner = projectile.owner(host).unwrap();
                self.owners.lock().unwrap().push(owner.uuid(host));
                if self.cancel.load(Ordering::Relaxed) {
                    return Flow::Cancel;
                }
            }
            Flow::Continue
        }
    }
    let observed = Arc::new(Mutex::new(Vec::new()));
    let cancel = Arc::new(AtomicBool::new(false));
    let mods = vec![ModEntry::new(
        "observe",
        Box::new(Observe {
            owners: observed.clone(),
            cancel: cancel.clone(),
        }),
    )];
    mod_platform::testing::with_mods(mods.into(), || {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            // A mod reaches the shot only in an accessible section.
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let source = game.entity(12).unwrap();
            let owner = game.living(source).unwrap();
            let previous_actor = game.server.lan_acting;
            let stack = ItemStack::new("minecraft:snowball", 1);
            let kind = Identifier::parse("minecraft:snowball").unwrap();
            let shot = game
                .spawn_projectile_from_rotation(owner, &kind, &stack, 0.0, 1.5, 1.0)
                .unwrap();
            assert_eq!(game.projectile_owner(shot), Some(source));
            assert_eq!(game.server.lan_acting, previous_actor);
            assert_eq!(*observed.lock().unwrap(), [Some(12)]);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.dimension = "minecraft:the_nether".to_owned();
                })
                .unwrap();
            let count = game.creatures.projectiles.len();
            let mut before = game.world.random.clone();
            sunrise_level::random_source::set_thread_uniquifier(2709830126447559882).unwrap();
            assert!(
                game.spawn_projectile_from_rotation(owner, &kind, &stack, 0.0, 1.5, 1.0)
                    .is_none()
            );
            assert_eq!(game.creatures.projectiles.len(), count);
            assert_eq!(game.world.random.next_long(), before.next_long());
            assert_eq!(
                sunrise_level::random_source::create().next_double(),
                0.6465821602909256
            );
            assert_eq!(observed.lock().unwrap().len(), 1);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.dimension = "minecraft:overworld".to_owned();
                })
                .unwrap();
            cancel.store(true, Ordering::Relaxed);
            assert!(
                game.spawn_projectile_from_rotation(owner, &kind, &stack, 0.0, 1.5, 1.0)
                    .is_none()
            );
            assert_eq!(game.creatures.projectiles.len(), count);
            game.native_joins_flush(false, 0);
            assert_eq!(*observed.lock().unwrap(), [Some(12), Some(12)]);
        });
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
