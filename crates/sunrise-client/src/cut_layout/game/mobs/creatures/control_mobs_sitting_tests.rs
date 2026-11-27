//! The `mobs` command's `sitting` is `TamableAnimal.isInSittingPose`, the bit of
//! `DATA_FLAGS_ID` a server's client gets, not the saved order to sit only the server holds.
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;

fn reported_sitting(game: &sunrise_gameplay::game::ServerGame<'_>) -> serde_json::Value {
    game.control_mobs_at(4.0, Vec3::new(8.0, 65.0, 8.0))["near"][0]["sitting"].clone()
}

#[test]
#[ignore = "daily"]
fn the_mobs_command_reports_the_sitting_pose() {
    with_game(|game| {
        let kind = game.res.mob_types().get("minecraft:wolf").unwrap().clone();
        let mut wolf = sunrise_sim::mobs::Mob::new(5, kind, Vec3::new(8.0, 65.0, 8.0), 0.0);
        // A guest's copy: the pose bit came in the metadata; the order never does.
        wolf.extra.sitting = false;
        wolf.extra.sitting_pose = true;
        game.mobs.push(wolf);
        assert_eq!(reported_sitting(game), serde_json::json!(true));
        // Ordered to sit but not in the pose yet (in water, in the air): not drawn sitting.
        game.mobs[0].extra.sitting = true;
        game.mobs[0].extra.sitting_pose = false;
        assert_eq!(reported_sitting(game), serde_json::json!(false));
    });
}
