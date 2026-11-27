use crate::game::creatures::projectile_batch_tests::shoot_arrow_at_wall;
use std::sync::atomic::Ordering;
use sunrise_core::geometry::Vec3;
use {
    crate::game::server_side::native_projectiles::impact_probe::Action,
    crate::game::server_side::native_projectiles::impact_probe::with_probe,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

#[test]
#[ignore = "daily"]
fn an_arrow_does_not_stick_when_the_impact_listener_clears_its_block() {
    const CASE: &str = "SUNRISE_PROJECTILE_BATCH_CASE";
    if run_fresh_cases(
        "cut_layout::game::client_side::creatures_client::projectile_batch_tests::an_arrow_does_not_stick_when_the_impact_listener_clears_its_block",
        CASE,
        &["clear", "allow"],
    ) {
        return;
    }
    let clear = std::env::var(CASE).as_deref() == Ok("clear");
    let action = if clear {
        Action::ClearBlockOnImpact
    } else {
        Action::Allow
    };
    with_probe(action, true, |game, observed| {
        game.start_audio_capture();
        shoot_arrow_at_wall(game, 2, 0.5);
        let hits = observed.hits.lock().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].at, [2.0, 64.5, 0.5]);
        assert_eq!(hits[0].projectile_pos, hits[0].at);
        let p = &game.creatures.projectiles[0];
        let arrow = p.arrow.as_ref().unwrap();
        if clear {
            assert_eq!(
                p.position.components(),
                hits[0].at,
                "refusal must not replay the flight"
            );
            assert_eq!(p.velocity, Vec3::new(3.0 * f64::from(0.99_f32), -0.05, 0.0));
            assert_eq!(arrow.shake, 0);
        } else {
            assert_eq!(p.position.x, 2.0 - f64::from(0.05_f32));
            assert_eq!(p.velocity, Vec3::ZERO);
            assert_eq!(arrow.shake, 7);
        }
        assert_eq!(arrow.in_ground, !clear);
        assert_eq!(observed.lands.load(Ordering::Relaxed), usize::from(!clear));
        assert_eq!(
            game.server
                .pending_sounds
                .iter()
                .any(|sound| { sound.event == "minecraft:entity.arrow.hit" }),
            !clear
        );
    });
}
