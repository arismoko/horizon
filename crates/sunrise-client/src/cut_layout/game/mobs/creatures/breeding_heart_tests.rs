use sunrise_gameplay_common::game::creatures::{Mob, Vec3};
/// `Turtle.TurtleBreedGoal.breed` overrides `Animal.breed`, so a bred turtle
/// never runs `finalizeSpawnChildFromBreeding`'s heart burst (entity event
/// 18); `Frog.spawnChildFromBreeding` still does.
#[test]
#[ignore = "daily"]
fn a_bred_turtle_posts_no_heart_burst_but_a_bred_frog_does() {
    let mut resources = None;
    let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
    game.start_audio_capture();
    for (kind_id, hearts) in [("minecraft:turtle", 0), ("minecraft:frog", 1)] {
        let kind = game.res.mob_types().get(kind_id).unwrap().clone();
        game.mobs
            .push(Mob::new(1, kind.clone(), Vec3::new(8.5, 64.0, 8.5), 0.0));
        game.mobs
            .push(Mob::new(2, kind, Vec3::new(9.5, 64.0, 8.5), 0.0));
        game.server.lan_entity_events.clear();
        game.breed(
            1,
            2,
            None,
            None,
            None,
            sunrise_sim::mobs::BreedExperience::Deferred,
            true,
        );
        let burst = game
            .server
            .lan_entity_events
            .iter()
            .filter(|(id, event)| *id == 1 && *event == 18)
            .count();
        assert_eq!(
            burst, hearts,
            "{kind_id} heartbeat events: {:?}",
            game.server.lan_entity_events
        );
        game.mobs.clear();
        game.orbs.clear();
    }
}
