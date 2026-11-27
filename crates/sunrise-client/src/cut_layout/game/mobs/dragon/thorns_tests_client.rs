use crate::game::server_side::dragon::thorns_tests::thorny;
use sunrise_gameplay_common::game::dragon_common::Dragon;
#[cfg(test)]
use sunrise_gameplay_common::game::{
    Instant, Options, PathBuf, PlayerMotion, Progress, Vec3, World,
};
use sunrise_sim::mobs::{Damage, DamageKind, Mob, Who, finalize as f};
use {crate::game::*, sunrise_gameplay_common::game::execution_role};
use {
    sunrise_gameplay_common::game::dragon_common::FightWho,
    sunrise_gameplay_common::game::dragon_common::SPECIAL,
};

pub(crate) fn check_answer(g: &sunrise_gameplay::game::ServerGame<'_>, slot: usize, answers: bool) {
    let health = g.mobs[0].health;
    assert_eq!(health < 100.0, answers, "attacker health: {health}");
    assert_eq!(
        f::item_stack(&g.mobs[1], slot).unwrap().damage,
        if answers { 2 } else { 0 }
    );
    assert_eq!(f::item_stack(&g.mobs[0], f::MAINHAND).unwrap().damage, 0);
    let sounds: Vec<_> = g
        .server
        .pending_sounds
        .iter()
        .filter(|s| s.event == "minecraft:enchant.thorns.hit")
        .collect();
    assert_eq!(sounds.len(), usize::from(answers));
    if answers {
        assert!((95.0..=99.0).contains(&health));
        let source = g.mobs[0].last_damage.unwrap().0;
        assert_eq!(source.kind, DamageKind::Thorns);
        assert_eq!(source.attacker, Some(Who::Mob(42)));
        assert_eq!(sounds[0].position, g.mobs[0].center());
    }
}

#[test]
#[ignore = "daily"]
fn dragon_takes_player_thorns_through_its_body() {
    with_game(|g| {
        for wing in [false, true] {
            g.player = Some(PlayerMotion::new(g.motion, Vec3::new(30.0, 64.0, 0.5)).unwrap());
            g.player.as_mut().unwrap().entity_id = 77;
            g.armor[1] = Some(thorny());
            let mut d = Dragon::new(Vec3::new(0.0, 70.0, 0.0), 0.0, 1);
            d.id = SPECIAL;
            g.dragon.replace(d);
            g.server.pending_sounds.clear();
            if wing {
                g.dragon_wing_knock_back(Vec3::ZERO, false);
            } else {
                g.dragon_hurts_player("minecraft:mob_attack", 10.0);
            }
            let d = g.dragon.as_ref().unwrap();
            // DamageEntity rolls [1,5); the body's cut is damage/4 + min(damage,1).
            assert_eq!(d.health, 198.019_12);
            assert_eq!(
                d.last_source,
                Some(("minecraft:thorns", Some(77), Some(77)))
            );
            assert_eq!(d.hurt_by_player, Some((FightWho::Host, 0)));
            // Player.hurtArmor adds floor(max(1, damage / 4)) before
            // Thorns' two durability: 10-point head hit = 4, 5-point wing = 3.
            assert_eq!(
                g.armor[1].as_ref().unwrap().damage,
                if wing { 3 } else { 4 }
            );
            assert_eq!(
                g.server
                    .pending_sounds
                    .iter()
                    .filter(|s| s.event == "minecraft:enchant.thorns.hit")
                    .count(),
                1
            );
        }
        // Head contacts run effects through a refused player hit; breath does not.
        g.dragon.as_mut().unwrap().invulnerable = 0;
        g.player.as_mut().unwrap().health.advance_clock();
        let health = g.player.as_ref().unwrap().health.health();
        g.dragon_hurts_player("minecraft:mob_attack", 1.0);
        assert_eq!(g.player.as_ref().unwrap().health.health(), health);
        // The refused head hit adds only Thorns wear to the wing's 3;
        // the refused breath hit adds neither ordinary armor wear nor Thorns.
        assert_eq!(g.armor[1].as_ref().unwrap().damage, 5);
        let before = g.dragon.as_ref().unwrap().health;
        g.dragon_hurts_player("minecraft:indirect_magic", 1.0);
        assert_eq!(g.dragon.as_ref().unwrap().health, before);
        assert_eq!(g.armor[1].as_ref().unwrap().damage, 5);

        // Move past knockBack's two-tick recent-attacker exclusion, but stay
        // inside the ten-tick damage cooldown so this is an actual refused hurt.
        for _ in 0..3 {
            g.player.as_mut().unwrap().health.advance_clock();
            g.world.game_time += 1;
        }
        // A refused wing still retaliates, but neither wears ordinary armor
        // nor retains its nonzero push or a new velocity-sync marker.
        let d = g.dragon.as_mut().unwrap();
        d.invulnerable = 0;
        d.last_source = None;
        let before = d.health;
        g.server.pending_sounds.clear();
        let p = g.player.as_mut().unwrap();
        p.health.sync_velocity = false;
        let velocity = Vec3::new(0.25, -0.125, 0.75);
        p.velocity = velocity;
        g.dragon_wing_knock_back(Vec3::new(0.5, 0.2, -0.25), false);
        let p = g.player.as_ref().unwrap();
        assert_eq!(p.health.health(), health);
        assert_eq!(p.velocity, velocity);
        assert!(!p.health.sync_velocity);
        assert_eq!(g.armor[1].as_ref().unwrap().damage, 7);
        let d = g.dragon.as_ref().unwrap();
        assert!(d.health < before);
        assert_eq!(
            d.last_source,
            Some(("minecraft:thorns", Some(77), Some(77)))
        );
        // Attribution stores the dragon's age, not the world's game time.
        assert_eq!(d.hurt_by_player, Some((FightWho::Host, 0)));
        assert_eq!(
            g.server
                .pending_sounds
                .iter()
                .filter(|s| s.event == "minecraft:enchant.thorns.hit")
                .count(),
            1
        );
    });
}

pub(crate) fn pair(g: &mut sunrise_gameplay::game::ServerGame<'_>, slot: usize) -> Damage {
    g.mobs.clear();
    g.server.pending_sounds.clear();
    let kind = g.res.mob_types().get("minecraft:husk").unwrap().clone();
    for (id, x) in [(41, 10.5), (42, 0.5)] {
        let mut m = Mob::new(id, kind.clone(), Vec3::new(x, 64.0, 0.5), 0.0);
        m.health = 100.0;
        g.mobs.push(m);
    }
    g.mobs[0].random.set_seed(1);
    f::set_item_stack(&mut g.mobs[0], f::MAINHAND, thorny()); // Retaliation must not recurse.
    f::set_item_stack(&mut g.mobs[1], slot, thorny());
    Damage::mob(41, [10.5, 64.0, 0.5])
}

pub(crate) fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        Instant::now(),
        false,
        execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let mut g = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    g.options.dedicated = Some(Vec::new());
    g.world = World::new(g.res.facts().air, 1);
    g.player = Some(PlayerMotion::new(g.motion, Vec3::new(30.0, 64.0, 0.5)).unwrap());
    g.player.as_mut().unwrap().entity_id = 77;
    g.start_audio_capture();
    test(&mut g);
}
