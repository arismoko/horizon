use sunrise_gameplay_common::stats::Stats;
use {
    crate::game::server_side::game_server::reset_world_progress,
    sunrise_gameplay_common::game::advancements_common::AdvState,
};

#[test]
fn world_entry_invalidates_loaded_progress() {
    let mut adv = AdvState::default();
    let mut stats = Stats::default();
    adv.loaded = true;
    stats.loaded = true;
    adv.progress
        .criteria
        .insert("minecraft:story/root".into(), Default::default());
    stats.add("mined", "minecraft:stone", 4);
    reset_world_progress(&mut adv, &mut stats);
    assert!(!adv.loaded && !stats.loaded);
    assert!(adv.progress.criteria.is_empty());
    assert!(stats.values.is_empty());
}
