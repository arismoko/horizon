use crate::game::server_side::lan::tracking_hook_tests::Heard;
use crate::game::server_side::lan::tracking_hook_tests::Sent;
use crate::game::server_side::lan::tracking_hook_tests::at;
use crate::game::server_side::lan::tracking_hook_tests::tick_until_heard;
use crate::game::server_side::lan::tracking_hook_tests::with_two_items;
use sunrise_items::items::ItemStack;

/// A stop whose removal waits for a captured sound: the item's sound comes after one from an
/// emitter the players must be paired with first, so its removal is held until both sounds are out
/// (`Hub::prepare_captured_sounds`, `finish_captured_sounds`). The mod hears the stop only then,
/// and its word reaches each player after that removal.
#[test]
#[ignore = "daily"]
fn a_stop_waiting_for_a_sound_is_heard_after_its_removal() {
    with_two_items(|game, heard, [first, second], [_, near]| {
        let _ = tick_until_heard(game, heard);
        game.spawn_item(
            ItemStack::new("minecraft:diamond", 1),
            [8.5, 64.0, 12.5],
            [0.0; 3],
            32767,
        );
        let fresh = game.dropped.last().unwrap().id;
        let index = game.dropped.iter().position(|d| d.id == near).unwrap();
        let _ = game.dropped.remove(index);
        let sound = |entity: u32| sunrise_game_data::sound_source::ServerSound {
            event: "minecraft:entity.item.pickup".into(),
            source: 7,
            position: [8.5, 64.0, 8.5],
            volume: 1.0,
            pitch: 1.0,
            seed: Some(0),
            entity: Some(entity),
            except_entity: None,
        };
        game.start_audio_capture();
        game.server.pending_sounds = vec![sound(fresh), sound(near)];
        let told = tick_until_heard(game, heard);
        assert_eq!(
            told.iter().filter(|h| **h == Heard::Stop(near)).count(),
            2,
            "{told:?}"
        );
        for sent in [first(), second()] {
            let removed = sent
                .iter()
                .position(|s| matches!(s, Sent::Remove(ids) if ids.contains(&near)))
                .unwrap_or_else(|| panic!("no removal of {near}: {sent:?}"));
            let said = at(&sent, &Sent::Said(format!("stop {near}")));
            assert!(removed < said, "the word came before the removal: {sent:?}");
        }
    });
}
