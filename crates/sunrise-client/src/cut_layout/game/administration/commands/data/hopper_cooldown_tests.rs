//! `/data` on a hopper reads `HopperBlockEntity.saveAdditional`, which always
//! writes `TransferCooldown` (#2356).

use sunrise_core::nbt::Tag;
use sunrise_core::position::BlockPos;
use sunrise_gameplay::game::commands::data::DataRef;
use sunrise_gameplay::game::game_tests::tests::server_game;
use sunrise_level::containers::BlockEntity;

/// The result of `/data get block <hopper> TransferCooldown`
/// (`DataCommands.getData`'s numeric value), or `None` when it failed.
fn cooldown_shown(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> Option<i32> {
    let (ok, outcome) = game.recording_command_outcome(|game| {
        game.run_command("data get block 4 200 4 TransferCooldown")
    });
    match outcome {
        Some((true, value)) if ok => Some(value),
        _ => None,
    }
}

/// A fresh hopper shows vanilla's -1 (`cooldownTime` starts at -1), a
/// transfer's 8 and its countdown, and a `/data merge` value; merging the
/// value it already has changes nothing (`commands.data.merge.failed`), and
/// merging other fields keeps the running cooldown.
#[test]
#[ignore = "daily"]
fn data_reads_a_hoppers_live_transfer_cooldown() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    let game = &mut game;
    game.build_registries();
    game.set_allow_commands(true);
    game.block_ticks
        .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
        .unwrap();
    assert!(game.run_command(
        "setblock 4 200 4 hopper[facing=east,enabled=true]{Items:[{Slot:0b,id:\"minecraft:stone\",count:2}]}"
    ));
    assert!(game.run_command("setblock 5 200 4 chest[facing=north]"));
    assert_eq!(cooldown_shown(game), Some(-1));
    let full = game.data_of(&DataRef::Block([4, 200, 4])).unwrap();
    assert_eq!(full.get("TransferCooldown"), Some(&Tag::Int(-1)));
    // Only a hopper's saveAdditional writes it: not the chest beside it.
    let chest = game.data_of(&DataRef::Block([5, 200, 4])).unwrap();
    assert_eq!(chest.get("TransferCooldown"), None);
    assert!(!game.run_command("data merge block 4 200 4 {TransferCooldown:-1}"));

    // HopperBlockEntity.pushItemsTick: a move sets the cooldown to 8.
    game.world.game_time += 1;
    game.tick_hopper((4, 200, 4));
    let BlockEntity::Chest(chest) = &game.world.block_entities[&(5, 200, 4)] else {
        panic!("chest")
    };
    assert_eq!(chest.iter().flatten().map(|s| s.count).sum::<u32>(), 1);
    assert_eq!(cooldown_shown(game), Some(8));
    // The saved tag is an Int, as `putInt` writes it.
    let moved = game.data_of(&DataRef::Block([4, 200, 4])).unwrap();
    assert_eq!(moved.get("TransferCooldown"), Some(&Tag::Int(8)));
    game.world.game_time += 1;
    game.tick_hopper((4, 200, 4));
    assert_eq!(cooldown_shown(game), Some(7));
    // BlockDataAccessor.setData loads the merged tag: the cooldown it read
    // back stays.
    assert!(game.run_command("data merge block 4 200 4 {Items:[]}"));
    assert_eq!(cooldown_shown(game), Some(7));

    assert!(game.run_command("data merge block 4 200 4 {TransferCooldown:5}"));
    assert_eq!(cooldown_shown(game), Some(5));
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
