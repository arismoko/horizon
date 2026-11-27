//! The random behind a thrown item and behind an item's sound: the thrower's
//! own (`Entity.random`, `LivingEntity.createItemStackToDrop`) or a client's
//! (`ClientPacketListener.random`), never the level's, whose draws are what
//! random ticks, leaf decay and spawns come from.
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::game_tests::tests::server_game;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
#[cfg(feature = "client")]
use sunrise_protocol::packets::play::{self, Clientbound as C};
#[cfg(feature = "client")]
use sunrise_sim::entities::{ExperienceOrb, ItemEntity};

/// A game with the player standing at 8.5 70 8.5 in a ticking chunk, with the
/// positional sounds it plays recorded.
pub(crate) fn standing_game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> sunrise_gameplay::game::ServerGame<'a> {
    let mut game = server_game(resources);
    let feet = Vec3::new(8.5, 70.0, 8.5);
    game.player = Some(sunrise_sim::motion::PlayerMotion::new(game.motion, feet).unwrap());
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.load_ticking_neighbourhood(0, 0);
    game.update_chunk_tickets();
    assert!(game.entity_ticking_area()(8, 8));
    game.server.ticked_entities_through = u32::MAX;
    game.start_audio_capture();
    game
}

#[cfg(feature = "client")]
fn server_sound(game: &mut sunrise_gameplay::game::ServerGame<'_>, event: &str) -> (f32, f32) {
    let heard = std::mem::take(&mut game.server.pending_sounds);
    let sound = heard
        .iter()
        .find(|s| s.event == event)
        .unwrap_or_else(|| panic!("no {event} sound among {heard:?}"));
    (sound.volume, sound.pitch)
}

#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(crate) use crate::game::client_side::item_entities_client::thrown_items_tests::sound;

/// `Player.createItemStackToDrop(false)`: the drop key's throw is 0.3 along
/// the view (level facing, so +z) and 0.1 up, scattered by the thrower's own
/// random, which the level's draws never see.
#[test]
#[ignore = "daily"]
fn a_thrown_item_takes_the_throwers_random_not_the_levels() {
    let mut resources = None;
    let mut game = standing_game(&mut resources);
    game.yaw = 0.0;
    game.pitch = 0.0;
    game.common.inventory[game.common.selected] = Some(ItemStack::new("minecraft:stone", 4));
    let level = game.world.random.state();
    game.drop_held_item(false);
    let thrown = game.dropped.last().expect("the drop key threw the stack");
    let velocity = thrown.velocity;
    assert!(
        (velocity.z - 0.3).abs() <= 0.02,
        "the eye throw goes 0.3 along the view: {velocity:?}"
    );
    assert!(velocity.x.abs() <= 0.02, "{velocity:?}");
    assert!((0.0..=0.2).contains(&velocity.y), "{velocity:?}");
    assert_eq!(
        game.world.random.state(),
        level,
        "the throw drew from the level's random"
    );
    // The next throw scatters differently: the thrower's stream is live.
    game.drop_held_item(false);
    assert_ne!(game.dropped.last().unwrap().velocity, velocity);
    assert_eq!(game.world.random.state(), level);
}

/// `Player.dropEquipment` -> `Inventory.dropAll(randomly)`: each stack a dying
/// player lets go rises 0.2, scattered by the thrower's own random.
#[test]
#[ignore = "daily"]
fn a_death_scatter_takes_the_throwers_random_not_the_levels() {
    let mut resources = None;
    let mut game = standing_game(&mut resources);
    game.inventory[0] = Some(ItemStack::new("minecraft:stone", 2));
    let level = game.world.random.state();
    game.drop_inventory(
        [8.5, 70.0, 8.5],
        &sunrise_gameplay_common::game::creatures::DyingDrops {
            entity: 0,
            kind: "minecraft:player",
            player: None,
            source: None,
            recently_hit: false,
        },
    );
    let thrown = game.dropped.first().expect("the stack was thrown");
    assert_eq!(thrown.velocity.y, f64::from(0.2f32), "a scatter rises 0.2");
    assert!(thrown.velocity.x.abs() <= 0.5 && thrown.velocity.z.abs() <= 0.5);
    assert_eq!(
        game.world.random.state(),
        level,
        "the scatter drew from the level's random"
    );
}

/// `ClientPacketListener.handleTakeItemEntity` pitches the pickup from the
/// client's own random; `GiveCommand.giveItem` uses the player's. Neither is
/// the level's, which random ticks draw from.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_pickup_pitch_is_not_drawn_from_the_levels_random() {
    use sunrise_core::random::{EntityRandom, LegacyRandom};
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
    use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
    use {
        sunrise_gameplay_common::game::server_side::native_sound_refs::tests::guest,
        sunrise_gameplay_common::game::server_side::native_sound_refs::tests::packets,
    };

    if run_fresh_cases(
        "cut_layout::game::server_side::item_entities::thrown_items_tests::a_pickup_pitch_is_not_drawn_from_the_levels_random",
        "SUNRISE_PICKUP_PITCH_CASE",
        &["pickup"],
    ) {
        return;
    }
    let mut resources = None;
    let mut game = standing_game(&mut resources);
    game.spawn_item(
        ItemStack::new("minecraft:stone", 2),
        [8.5, 70.0, 8.5],
        [0.0; 3],
        0,
    );
    let level = game.world.random.state();
    game.tick_items();
    assert!(game.dropped.is_empty(), "the item was picked up");
    assert_eq!(
        game.world.random.state(),
        level,
        "the pickup drew from the level's random"
    );

    with_guests(|game| {
        let at = [3.0, 64.0, 5.0];
        let (listener, rx) = guest(14, 14, at);
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        hub.guests.push(listener);
        let C::Sound(pickup) = hub.sound_packet("minecraft:entity.item.pickup", at, 0.2, 2.0, 7, 0)
        else {
            unreachable!();
        };
        game.build_registries();
        game.as_guest(12, |game| {
            game.player.as_mut().unwrap().random = EntityRandom::own(LegacyRandom::new(0x5eed));
        })
        .unwrap();
        let level = game.world.random.state();
        assert!(game.as_console(|game| game.run_command("give guest12 minecraft:stone 2")));
        let sounds: Vec<_> = packets(&rx)
            .into_iter()
            .filter_map(|p| match p {
                C::Sound(p) if p.sound == pickup.sound => Some((p.volume, p.pitch.to_bits())),
                _ => None,
            })
            .collect();
        // Java Random(0x5eed): four draws for the fake item, then two for the pitch.
        assert_eq!(sounds, [(0.2, 0x3fdcbf84)]);
        assert_eq!(
            game.world.random.state(),
            level,
            "/give drew from the level's random"
        );
    });
}

#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(crate) use crate::game::client_side::item_entities_client::thrown_items_tests::with_remote_game;

/// `ClientPacketListener.handleTakeItemEntity`: a client pitches an item's and
/// an orb's pickup from its own random (`ClientPacketListener.random`), so a
/// pickup never draws the level's stream, which its particles and ambient
/// sounds share.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_client_pickup_sound_takes_the_listeners_random() {
    with_remote_game(|game| {
        let at = Vec3::new(2.5, 64.0, 2.5);
        game.dropped.push(ItemEntity::new(
            9,
            ItemStack::new("minecraft:stone", 1),
            at,
            Vec3::ZERO,
            0.0,
        ));
        game.orbs.push(ExperienceOrb::new(10, at, Vec3::ZERO, 1));
        let take = |item: i32| {
            C::TakeItemEntity(play::TakeItemEntity {
                item,
                player: 1,
                amount: 1,
            })
        };
        let level = game.world.random.state();
        game.handle_packet(take(9)).unwrap();
        let (volume, pitch) = sound(game, "minecraft:entity.item.pickup");
        assert_eq!(volume, 0.2);
        assert!((0.6..5.4).contains(&pitch), "pitch {pitch}");
        assert_eq!(
            game.world.random.state(),
            level,
            "the client's pickup drew from the level's random"
        );
        game.handle_packet(take(10)).unwrap();
        let (volume, pitch) = sound(game, "minecraft:entity.experience_orb.pickup");
        assert_eq!(volume, 0.1);
        assert!((0.55..1.25).contains(&pitch), "pitch {pitch}");
        assert_eq!(
            game.world.random.state(),
            level,
            "the client's orb pickup drew from the level's random"
        );
    });
}

/// `ClientPacketListener.handleTakeItemEntity` shrinks an item's stack by the amount taken and
/// removes the entity only once it is empty, so a survivor or a replacement stays in view.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_client_take_shrinks_an_item_and_removes_it_only_when_empty() {
    with_remote_game(|game| {
        let at = Vec3::new(2.5, 64.0, 2.5);
        game.dropped.push(ItemEntity::new(
            9,
            ItemStack::new("minecraft:emerald", 8),
            at,
            Vec3::ZERO,
            0.0,
        ));
        let take = |amount: i32| {
            C::TakeItemEntity(play::TakeItemEntity {
                item: 9,
                player: 1,
                amount,
            })
        };
        game.handle_packet(take(0)).unwrap();
        assert_eq!(
            game.dropped[0].stack.count, 8,
            "an animation of nothing taken"
        );
        game.handle_packet(take(3)).unwrap();
        assert_eq!(
            game.dropped[0].stack.count, 5,
            "a partial pickup leaves the rest"
        );
        game.handle_packet(take(5)).unwrap();
        assert!(
            game.dropped.iter().all(|d| d.id != 9),
            "an emptied stack goes"
        );
    });
}

/// `Entity.lavaHurt` pitches the burn sound from the item's own random, the
/// one its constructor drew from.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_burning_items_sound_is_not_drawn_from_the_levels_random() {
    let mut resources = None;
    let mut game = standing_game(&mut resources);
    let lava = game
        .pack
        .assets
        .registry
        .default_state("minecraft:lava")
        .unwrap();
    game.world.set(12, 70, 8, lava).unwrap();
    game.spawn_item(
        ItemStack::new("minecraft:stone", 1),
        [12.5, 70.0, 8.5],
        [0.0; 3],
        0,
    );
    // `Entity.lavaHurt` sounds on the tenth tick of a contact that leaves the
    // item alive (`should_playLavaHurtSound`), so the item is about to be 10.
    game.dropped[0].ticks = 9;
    let level = game.world.random.state();
    game.tick_items();
    let (volume, pitch) = server_sound(&mut game, "minecraft:entity.generic.burn");
    assert_eq!(volume, 0.4);
    // `2.0 + nextFloat * 0.4`.
    assert!((2.0..2.4).contains(&pitch), "pitch {pitch}");
    assert_eq!(
        game.world.random.state(),
        level,
        "the burn sound drew from the level's random"
    );
}

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
