//! A drinking witch is not using an item: `LivingEntity.setLivingEntityFlag(1, …)` has only
//! `startUsingItem` and `stopUsingItem` as callers, and `Witch.aiStep` counts her drink in its
//! own `usingTime` and `DATA_USING_ITEM`. So a host sends her living flags as 0 while she
//! drinks (a vanilla client would otherwise start a use of her potion), and her drink on her
//! own flag. A use she really starts is sent as one.
use super::pet_pose_meta_tests::meta_hub;
use crate::game::client_side::remote_mob_data::tests::fixture_mob;
use sunrise_gameplay_common::game::lan_mob_metadata::mob_meta;
use sunrise_protocol::entity_data::Value;
use sunrise_sim::mobs::Mob;

/// What the host sends for `m` at `index`, if anything.
fn sent_at(m: &Mob, hub: &sunrise_sim::lan::hub::Hub, index: u8) -> Option<Value> {
    let (mut meta, mut defaults) = (Vec::new(), Vec::new());
    mob_meta(m, hub, None, 0, &mut meta, &mut defaults);
    meta.into_iter()
        .find(|(at, _, _)| *at == index)
        .map(|(_, _, value)| value)
}

#[test]
fn a_drinking_witch_sends_her_own_flag_and_no_item_use() {
    let hub = meta_hub();
    let mut witch = fixture_mob("minecraft:witch");
    // `Witch.aiStep` as it starts a drink: 32 ticks, her flag set.
    witch.extra.using_item = 32;
    witch.extra.flag = true;
    assert_eq!(sent_at(&witch, &hub, 8), None, "living flags stay 0");
    assert_eq!(sent_at(&witch, &hub, 17), Some(Value::Bool(true)));

    // The drink is done: neither is sent past its default.
    witch.extra.using_item = 0;
    witch.extra.flag = false;
    assert_eq!(sent_at(&witch, &hub, 8), None);
    assert_eq!(sent_at(&witch, &hub, 17), None);

    // A use she starts while not drinking (`start_mob_using_item`) is one.
    witch.extra.using_item = 20;
    assert_eq!(sent_at(&witch, &hub, 8), Some(Value::Byte(1)));
    assert_eq!(sent_at(&witch, &hub, 17), None);
}

/// An admiring piglin is not using an item either (#5132): `PiglinAi.holdInOffHand` sets no
/// living flag, and a client draws `PiglinArmPose.ADMIRING_ITEM` from the off-hand item.
#[test]
fn an_admiring_piglin_sends_no_item_use() {
    let hub = meta_hub();
    let mut piglin = fixture_mob("minecraft:piglin");
    piglin.extra.set_slot_id(
        sunrise_items::equipment::OFFHAND,
        Some("minecraft:gold_ingot".into()),
    );
    piglin.extra.using_item = sunrise_sim::mobs::piglin::ADMIRE + 1;
    assert_eq!(sent_at(&piglin, &hub, 8), None, "living flags stay 0");
}
