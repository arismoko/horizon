use std::sync::atomic::Ordering;
use sunrise_core::geometry::Vec3;
use sunrise_sim::thrown_trident::ThrownTrident;
use {
    crate::game::server_side::native_projectiles::impact_probe::Action,
    crate::game::server_side::native_projectiles::impact_probe::with_probe,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

pub(crate) fn aim_trident_at_wall(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
) -> mod_api::EntityRef {
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(2, 64, 0, stone).unwrap();
    let mut t = ThrownTrident::remote(71, Vec3::new(0.5, 64.5, 0.5), 0.0, 0.0);
    t.velocity = Vec3::new(4.0, 0.0, 0.0);
    t.owner = 7;
    t.left_owner = true;
    t.has_been_shot = false;
    game.tridents.push(t);
    game.native_joins_skip();
    game.entity(71).unwrap()
}

#[test]
#[ignore = "daily"]
fn an_already_discarded_trident_is_not_ticked_before_sweep() {
    const CASE: &str = "SUNRISE_TRIDENT_DISCARD_BEFORE_TICK_CASE";
    if run_fresh_cases(
        "cut_layout::game::server_side::trident::impact_tests::an_already_discarded_trident_is_not_ticked_before_sweep",
        CASE,
        &["committed", "journaled"],
    ) {
        return;
    }
    let journaled = std::env::var(CASE).as_deref() == Ok("journaled");
    with_probe(Action::Allow, true, |game, observed| {
        let handle = aim_trident_at_wall(game);
        let scope = game.storage_scope_guard(journaled);
        let actor = &mut *scope.game;
        assert!(actor.entity_discard(handle));
        assert!(actor.entity(71).is_none());
        let (age, at) = (actor.tridents[0].age, actor.tridents[0].position);
        actor.tick_tridents();
        assert_eq!(
            (actor.tridents[0].age, actor.tridents[0].position),
            (age, at)
        );
        assert!(observed.hits.lock().unwrap().is_empty());
        assert_eq!(observed.lands.load(Ordering::Relaxed), 0);
        scope.close(true);
        game.native_removals_flush();
        assert!(game.tridents.is_empty());
        assert_eq!(*observed.leaves.lock().unwrap(), [71]);
    });
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
