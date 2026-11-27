//! `CatSpawner.spawnCat`: the cat is created and finalized for a natural
//! spawn, then snapped to the spawn block with `snapTo(spawnPos, 0.0F, 0.0F)`,
//! so it faces yaw 0 and its facing takes no draw from the level's random.
use sunrise_core::random::RandomSource;
use sunrise_gameplay::game::game_tests::tests::server_game;
use sunrise_sim::mobs::Species;

#[test]
#[ignore = "daily"]
fn a_cat_spawner_cat_faces_south_without_a_level_draw() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    // Day zero is a full moon (brightness 1.0): all_black plus the ten
    // unconditional coats. There is no black-cat structure in this fixture.
    game.world.day_time = 0;
    let coats = [
        "minecraft:all_black",
        "minecraft:black",
        "minecraft:british_shorthair",
        "minecraft:calico",
        "minecraft:jellie",
        "minecraft:persian",
        "minecraft:ragdoll",
        "minecraft:red",
        "minecraft:siamese",
        "minecraft:tabby",
        "minecraft:white",
    ];
    let sounds: Vec<String> = game
        .world_variants()
        .cat_sound
        .iter()
        .map(|(_, id, _)| id.to_owned())
        .collect();
    assert_eq!(sounds, ["minecraft:classic", "minecraft:royal"]);
    let sound_count = sounds.len() as i32;
    let mut level = game.world.random.clone();
    for _ in 0..8 {
        // Mob.finalizeSpawn: triangle follow-range bonus and left hand;
        // Cat.finalizeSpawn: variant and sound. A lone cat has no baby
        // chance draw, and the fixed facing adds no yaw draw.
        level.next_double();
        level.next_double();
        level.next_float();
        let coat = level.next_int_bound(coats.len() as i32).unwrap() as usize;
        let sound = level.next_int_bound(sound_count).unwrap() as usize;
        let before = game.mobs.len();
        sunrise_gameplay::cat_spawner::spawn_cat(&mut game, [4, 100, 8], false);
        assert_eq!(game.mobs.len(), before + 1, "one cat");
        let variants = game.world_variants();
        let cat = game.mobs.last().expect("cat");
        assert_eq!(cat.species(), Species::Cat);
        assert_eq!(
            cat.cat_variant(&variants.cat)
                .and_then(|r| variants.cat.id(r)),
            Some(coats[coat])
        );
        assert_eq!(
            cat.cat_sound_variant(&variants.cat_sound)
                .and_then(|r| variants.cat_sound.id(r)),
            Some(sounds[sound].as_str())
        );
        assert_eq!(cat.yaw, 0.0);
        assert_eq!(cat.body_yaw, 0.0);
        assert_eq!(cat.head_yaw, 0.0);
        // `snapTo(BlockPos, ..)` is `Vec3.atBottomCenterOf`: x + 0.5, y, z + 0.5.
        assert_eq!(
            [cat.position.x, cat.position.y, cat.position.z],
            [4.5, 100.0, 8.5]
        );
    }
    // Finalization draws remain, but the fixed facing adds none.
    assert_eq!(game.world.random.state(), level.state());
}
