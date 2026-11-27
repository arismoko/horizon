use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::server_side::combat::bee_sting_tests::spawn;
use sunrise_gameplay::game::server_side::combat::bee_sting_tests::sting;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::{Who, bee};
use sunrise_sim::motion::PlayerMotion;
use sunrise_world::food::Difficulty;

/// A sting that lands sets the flag, calms the bee, plays the sound at its
/// position and poisons the victim on normal and hard.
#[test]
#[ignore = "daily"]
fn a_landed_sting_sets_the_stinger_and_poisons_on_normal_and_hard() {
    let mut resources = None;
    let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
    for (difficulty, poison_ticks) in [(Difficulty::Normal, 200), (Difficulty::Hard, 360)] {
        game.mobs.clear();
        game.world.health_rules.difficulty = difficulty;
        game.start_audio_capture();
        let bee = spawn(&mut game, 41, "minecraft:bee", Vec3::new(1.5, 64.0, 0.5));
        let pig = spawn(&mut game, 42, "minecraft:pig", Vec3::new(0.5, 64.0, 0.5));
        let bee_mob = game.mobs.iter_mut().find(|m| m.id == bee).unwrap();
        let b = bee_mob.extra.bee.get_or_insert_with(Default::default);
        b.angry_at = Some(Who::Mob(pig));
        b.anger_end = 400;
        sting(&mut game, bee, Who::Mob(pig));
        let bee_mob = game.mobs.iter().find(|m| m.id == bee).unwrap();
        assert!(bee::has_stung(bee_mob), "{difficulty:?}");
        assert_eq!(bee::get(bee_mob).unwrap().angry_at, None, "{difficulty:?}");
        let pig_mob = game.mobs.iter().find(|m| m.id == pig).unwrap();
        assert!(pig_mob.health < 10.0, "{difficulty:?}");
        let poison = pig_mob
            .extra
            .effects
            .iter()
            .find(|effect| effect.id == "minecraft:poison");
        assert_eq!(
            poison.map(|effect| (effect.duration, effect.amplifier)),
            Some((poison_ticks, 0)),
            "{difficulty:?}"
        );
        assert_eq!(
            sting_sounds(&mut game),
            [(1.0, [1.5, 64.0, 0.5])],
            "{difficulty:?}"
        );
    }
}

/// An angry bee that stings a victim which refuses the hit keeps its stinger,
/// its anger and its sound.
#[test]
#[ignore = "daily"]
fn a_refused_sting_leaves_the_bee_alone() {
    let mut resources = None;
    let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
    game.world.health_rules.difficulty = Difficulty::Normal;
    // `Invulnerable:1b`: `hurtServer` reports no hurt.
    game.start_audio_capture();
    let bee = spawn(&mut game, 41, "minecraft:bee", Vec3::new(1.5, 64.0, 0.5));
    let pig = spawn(&mut game, 42, "minecraft:pig", Vec3::new(0.5, 64.0, 0.5));
    game.mobs
        .iter_mut()
        .find(|m| m.id == pig)
        .unwrap()
        .invulnerable_flag = true;
    let bee_mob = game.mobs.iter_mut().find(|m| m.id == bee).unwrap();
    let b = bee_mob.extra.bee.get_or_insert_with(Default::default);
    b.angry_at = Some(Who::Mob(pig));
    b.anger_end = 400;
    sting(&mut game, bee, Who::Mob(pig));
    let bee_mob = game.mobs.iter().find(|m| m.id == bee).unwrap();
    assert!(!bee::has_stung(bee_mob), "invulnerable mob");
    assert_eq!(bee::get(bee_mob).unwrap().angry_at, Some(Who::Mob(pig)));
    assert!(sting_sounds(&mut game).is_empty());

    game.mobs.clear();
    game.start_audio_capture();
    game.creative = true;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.player.as_mut().unwrap().invulnerable = true;
    let bee = spawn(&mut game, 43, "minecraft:bee", Vec3::new(1.5, 64.0, 0.5));
    sting(&mut game, bee, Who::Player);
    let bee_mob = game.mobs.iter().find(|m| m.id == bee).unwrap();
    assert!(!bee::has_stung(bee_mob));
    assert!(sting_sounds(&mut game).is_empty());

    game.mobs.clear();
    game.start_audio_capture();
    game.creative = false;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.yaw = 0.0;
    game.offhand = Some(ItemStack::new("minecraft:shield", 1));
    // `BlocksAttacks.blockDelayTicks` of the shield: 0.25s.
    game.using = Some((sunrise_gameplay::game::server_side::shield::OFFHAND_SLOT, 5));
    let bee = spawn(&mut game, 44, "minecraft:bee", Vec3::new(0.5, 64.0, 2.5));
    sting(&mut game, bee, Who::Player);
    let bee_mob = game.mobs.iter().find(|m| m.id == bee).unwrap();
    assert!(!bee::has_stung(bee_mob));
    assert!(sting_sounds(&mut game).is_empty());
}

pub(crate) fn sting_sounds(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
) -> Vec<(f32, [f64; 3])> {
    std::mem::take(&mut game.server.pending_sounds)
        .into_iter()
        .filter(|s| s.event == "minecraft:entity.bee.sting")
        .map(|s| (s.pitch, s.position))
        .collect()
}
