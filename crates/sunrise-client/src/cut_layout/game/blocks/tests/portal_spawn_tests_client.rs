use crate::game::server_side::nether_portal::portal_spawn_tests::game;
use crate::game::server_side::nether_portal::portal_spawn_tests::portal_on;
use crate::game::server_side::nether_portal::portal_spawn_tests::spawned;

/// On obsidian the piglin spawns, grunts (`EntityType.spawn`'s
/// `Mob.playAmbientSound`: its ambient sound at its feet, volume 1.0, at its
/// voice pitch 1.0 ± 0.2) and waits out its 300-tick portal cooldown.
#[test]
#[ignore = "daily"]
fn a_portal_over_obsidian_spawns_a_grunting_piglin() {
    let mut resources = None;
    let mut game = game(&mut resources);
    portal_on(&mut game, "obsidian");
    game.server.audio_capture_active = true;
    game.server.pending_sounds.clear();
    game.portal_spawn([0, 64, 0]);
    assert_eq!(
        spawned(&game),
        vec!["minecraft:zombified_piglin".to_owned()],
        "obsidian floor"
    );
    let id = game.mobs[0].id;
    // setPortalCooldown: Entity.getDimensionChangingDelay.
    assert_eq!(
        game.server.portals.entities.get(&id).map(|p| p.cooldown),
        Some(300),
        "the new piglin's portal cooldown"
    );
    let sounds = std::mem::take(&mut game.server.pending_sounds);
    assert_eq!(sounds.len(), 1, "{sounds:?}");
    assert_eq!(sounds[0].event, "minecraft:entity.zombified_piglin.ambient");
    assert_eq!(sounds[0].position, [0.5, 64.0, 0.5], "at the piglin's feet");
    assert_eq!(sounds[0].volume, 1.0);
    // LivingEntity.getVoicePitch of an adult: 1.0 ± 0.2. (Zombie.finalizeSpawn's
    // 5 % baby roll is not modelled yet, so a portal piglin is always grown.)
    assert!(
        (0.8..=1.2).contains(&sounds[0].pitch),
        "voice pitch {}",
        sounds[0].pitch
    );
}
