#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::InteractTarget;
use sunrise_items::items::ItemStack;
use sunrise_sim::entities::ItemFrame;
#[cfg(test)]
pub(crate) mod map_limit_tests {
    use super::*;
    use sunrise_gameplay_common::game::maps::{Decoration, Key, MapData};

    fn frame(id: u32) -> ItemFrame {
        ItemFrame {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            pos: [0, 64, id as i32],
            facing: 3,
            glow: false,
            item: None,
            rotation: 0,
            drop_chance: 1.0,
            fixed: false,
            invisible: false,
            check_interval: 0,
            ticks: 0,
            custom_name: None,
            random: sunrise_core::random::LegacyRandom::new(0),
        }
    }
    /// A map whose `count` frame markers all count toward the limit.
    fn map_tracking(count: i32) -> MapData {
        let mut map = MapData::fresh(0.0, 0.0, 0, "minecraft:overworld");
        map.decorations = (0..count)
            .map(|i| Decoration {
                sprite: "map/decorations/frame",
                x: 0,
                y: 0,
                rot: 0,
                name: None,
                key: Key::Frame(1000 + i),
            })
            .collect();
        map
    }
    fn filled_map(id: i32) -> ItemStack {
        let mut stack = ItemStack::new("minecraft:filled_map", 1);
        stack.data_mut().map_id = Some(id);
        stack
    }
    fn at(id: u32) -> Option<InteractTarget> {
        Some(InteractTarget {
            id: id as i32,
            location: [0.0; 3],
        })
    }

    #[test]
    #[ignore = "daily"]
    fn a_map_tracking_more_than_256_markers_stays_in_the_hand() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.item_frames.extend([frame(1), frame(2)]);
            game.maps.data.insert(7, map_tracking(257));
            game.maps.data.insert(8, map_tracking(256));

            // ItemFrame.interact: FAIL, the frame stays empty and the map is kept.
            *game.held_mut() = Some(filled_map(7));
            assert!(
                game.interact_frame(at(1)),
                "the refusal ends the interaction"
            );
            assert!(game.item_frames[0].item.is_none());
            assert_eq!(game.held().and_then(|s| s.data()?.map_id), Some(7));

            // At the limit (not over it) the map goes in.
            *game.held_mut() = Some(filled_map(8));
            assert!(game.interact_frame(at(2)));
            let framed = game.item_frames[1].item.as_ref();
            assert_eq!(framed.and_then(|s| s.data()?.map_id), Some(8));
        });
    }
}

#[cfg(test)]
pub(crate) mod game_event_tests {
    use super::super::game_tests::tests::server_game;
    use super::*;
    use sunrise_sim::mobs::Who;

    fn frame(id: u32, pos: [i32; 3], item: Option<ItemStack>) -> ItemFrame {
        ItemFrame {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            pos,
            facing: 3,
            glow: false,
            item,
            rotation: 0,
            drop_chance: 1.0,
            fixed: false,
            invisible: false,
            check_interval: 0,
            ticks: 0,
            custom_name: None,
            random: sunrise_core::random::LegacyRandom::new(0),
        }
    }
    fn at(id: u32) -> Option<InteractTarget> {
        Some(InteractTarget {
            id: id as i32,
            location: [0.0; 3],
        })
    }
    /// The events posted since [`reset`].
    fn posted() -> Vec<(&'static str, [f64; 3], Option<Who>)> {
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
            .with(|e| e.borrow().clone().unwrap_or_default())
    }
    fn reset() {
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
            .with(|e| *e.borrow_mut() = Some(Vec::new()));
    }

    /// `ItemFrame.interact`: putting an item in and turning it each post
    /// `GameEvent.BLOCK_CHANGE` from the player, at the frame's position.
    #[test]
    #[ignore = "daily"]
    fn putting_and_rotating_a_frame_item_posts_block_change() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.item_frames.clear();
        game.item_frames.extend(vec![frame(1, [0, 64, 1], None)]);
        let center = [0.5, 64.5, 1.03125];
        *game.held_mut() = Some(ItemStack::new("minecraft:stick", 3));

        let expected = [("minecraft:block_change", center, Some(Who::Player))];
        reset();
        assert!(game.interact_frame(at(1)));
        assert_eq!(posted(), expected);
        assert_eq!(game.held().map(|s| s.count), Some(2), "one goes in");
        assert_eq!(game.item_frames[0].item.as_ref().map(|s| s.count), Some(1));

        reset();
        assert!(game.interact_frame(at(1)));
        assert_eq!(posted(), expected);
        assert_eq!(game.item_frames[0].rotation, 1);
    }

    /// `ItemFrame.hurtServer`: the item coming out posts `BLOCK_CHANGE` from
    /// the player hitting the frame.
    #[test]
    #[ignore = "daily"]
    fn taking_a_frame_item_out_posts_block_change_from_the_player() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.item_frames.clear();
        game.item_frames.extend(vec![frame(
            1,
            [0, 64, 1],
            Some(ItemStack::new("minecraft:stick", 1)),
        )]);
        let center = [0.5, 64.5, 1.03125];
        // A player looking straight at the frame's centre from two blocks away
        // (eye height 1.62 above the feet).
        let feet = Vec3::new(0.5, 64.5 - 1.62, -1.0);
        game.player =
            Some(sunrise_sim::motion::PlayerMotion::new(game.motion, feet).expect("a player"));
        game.yaw = 0.0;
        game.pitch = 0.0;

        let expected = [("minecraft:block_change", center, Some(Who::Player))];
        reset();
        assert!(game.attack_frame(), "the frame is in reach");
        assert!(game.item_frames[0].item.is_none());
        assert_eq!(posted(), expected);

        // The empty frame itself then breaks (`BlockAttachedEntity.hurtServer`
        // then `ItemFrame.dropItem`), with the same event.
        reset();
        assert!(game.attack_frame());
        assert!(game.item_frames.is_empty());
        assert_eq!(
            posted(),
            [
                ("minecraft:entity_die", center, Some(Who::Player)),
                ("minecraft:block_change", center, Some(Who::Player))
            ]
        );
    }

    /// `ItemFrame.dropItem` from `BlockAttachedEntity.tick`, when a frame
    /// loses the block it hangs on: the event has no source.
    #[test]
    #[ignore = "daily"]
    fn a_frame_that_falls_posts_block_change_without_source() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let mut f = frame(1, [0, 64, 1], None);
        // BlockAttachedEntity.tick checks every 100 ticks.
        f.check_interval = 99;
        game.item_frames.clear();
        game.item_frames.extend(vec![f]);
        let center = [0.5, 64.5, 1.03125];

        reset();
        game.tick_item_frames();
        assert!(game.item_frames.is_empty(), "nothing holds the frame");
        assert_eq!(posted(), [("minecraft:block_change", center, None)]);
    }
}

#[cfg(test)]
#[path = "tests/lodestone_compass_tests.rs"]
pub(crate) mod lodestone_compass_tests;
