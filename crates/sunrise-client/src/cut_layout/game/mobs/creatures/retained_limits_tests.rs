use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_core::geometry::Vec3;
use sunrise_sim::mobs::Mob;
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

pub(crate) const ZOMBIE: u32 = 71;
pub(crate) const TARGET_GUEST: u32 = 12;
pub(crate) const OTHER_GUEST: u32 = 13;

#[test]
#[ignore = "daily"]
fn pending_pushes_deliver_once_and_do_not_cross_world_reset() {
    if run_fresh_cases(
        "cut_layout::game::creatures::retained_limits_tests::pending_pushes_deliver_once_and_do_not_cross_world_reset",
        "SUNRISE_PENDING_MOB_PUSHES_CASE",
        &["delivery-and-reset"],
    ) {
        return;
    }

    with_game(|game| {
        const RECIPIENT: u32 = 72;
        const BYSTANDER: u32 = 73;
        let level = game.dim.id();
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        let mob = |id, x| Mob::new(id, kind.clone(), Vec3::new(x, 80.0, 0.5), 0.0);
        let velocity = |game: &sunrise_gameplay::game::ServerGame<'_>, id| {
            game.mobs.iter().find(|m| m.id == id).unwrap().velocity
        };

        // Only test delivery of already accepted pushes: skip ordinary mob travel in
        // the unloaded chunk so drag and new pushes cannot obscure the impulses.
        assert!(!game.world.loaded(64, 0));
        game.mobs.push(mob(BYSTANDER, 64.5));
        game.mobs.push(mob(RECIPIENT, 65.5));
        let pushes = &mut game.creatures.pending.entry(level).or_default().pushes;
        pushes.push(RECIPIENT, Vec3::new(0.25, 0.0, 0.0));
        pushes.push(RECIPIENT, Vec3::new(0.0, 0.0, -0.125));

        let expected = Vec3::new(0.25, 0.0, -0.125);
        for tick in ["delivery", "next tick"] {
            game.tick_creature_entities().unwrap();
            assert_eq!(
                velocity(game, RECIPIENT),
                expected,
                "{tick}: lost or replayed push"
            );
            assert_eq!(
                velocity(game, BYSTANDER),
                Vec3::ZERO,
                "{tick}: wrong recipient"
            );
        }

        game.creatures
            .pending
            .entry(level)
            .or_default()
            .pushes
            .push(RECIPIENT, Vec3::new(-0.125, 0.0, 0.0));
        game.clear_entities();
        game.mobs.push(mob(RECIPIENT, 66.5));
        game.tick_creature_entities().unwrap();
        assert_eq!(
            velocity(game, RECIPIENT),
            Vec3::ZERO,
            "a queued push crossed the world reset to a reused ID"
        );
    });
}

pub(crate) struct PanicInMovement(pub(crate) Arc<AtomicUsize>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for PanicInMovement {
    fn living_speed(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut mod_api::LivingSpeedEvent<'_>,
    ) {
        if event.entity == ZOMBIE {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("zombie speed listener failed");
        }
    }
}

pub(crate) struct ObserveMovement(pub(crate) Arc<AtomicUsize>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for ObserveMovement {
    fn living_speed(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut mod_api::LivingSpeedEvent<'_>,
    ) {
        if event.entity == ZOMBIE {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}
