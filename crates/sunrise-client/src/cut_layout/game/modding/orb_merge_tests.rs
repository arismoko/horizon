//! The experience orb's saved form (docs: `entity_nbt::orb_to_nbt`) and live
//! `ExperienceOrbRef` reads and writes.
use mod_api::ExperienceOrbWriteError;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_gameplay::game::native_entity_refs::tests::{fixture_resident_column, with_game};
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_sim::entities::ExperienceOrb;
use sunrise_sim::entity_nbt::{orb_to_nbt, orbs_from_nbt};

fn orb(id: u32, value: i32, count: u32) -> ExperienceOrb {
    let at = Vec3::new(0.5, 70.0, 0.5);
    ExperienceOrb {
        size_answer: None,
        attachment_lifetime: Default::default(),
        fall_distance: 0.0,
        rotation: [0.0, 0.0],
        join: Default::default(),
        id,
        position: at,
        previous: at,
        velocity: Vec3::ZERO,
        value,
        age: 0,
        ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
        count,
        fire_ticks: 0,
        ticks: 0,
        health: 5,
        air: 300,
        no_gravity: true,
        following: None,
    }
}

/// Values within `i16::MIN..=i16::MAX` and valid counts survive save and reload unchanged:
/// `Value` is saved as a short and `Count` as an int.
#[test]
fn every_accepted_orb_value_and_count_survives_save_and_reload() {
    for (value, count) in [(11, 1), (32767, i32::MAX as u32), (-32768, 1)] {
        let back = orbs_from_nbt(&orb_to_nbt(&orb(1, value, count), [0; 4]));
        assert_eq!((back[0].value, back[0].count), (value, count));
    }
}

/// Live `ExperienceOrbRef` writes accept full `i32` `value` and `age` (truncating to `(short)`
/// only at NBT serialization, as vanilla's `ExperienceOrb.addAdditionalSaveData` does), while
/// invalid counts (`0` or above `i32::MAX`) return `Err(ExperienceOrbWriteError::InvalidCount)`
/// without mutating the orb.
#[test]
#[ignore = "daily"]
fn experience_orb_ref_writes_live_ints_and_refuses_invalid_count() {
    with_game(|game| {
        fixture_resident_column(&mut game.world, (0, 0));
        game.options.dedicated = Some(Vec::new());
        game.orbs.push(orb(7, 3, 2));
        let entity = game.entity(7).expect("live orb entity");
        let orb_ref = game
            .experience_orb(entity)
            .expect("server orb lookup")
            .expect("experience orb handle");

        assert_eq!(orb_ref.set_value(game, 40_000), Ok(()));
        assert_eq!(orb_ref.set_age(game, 100_000), Ok(()));
        assert_eq!(orb_ref.value(&*game), Ok(40_000));
        assert_eq!(orb_ref.age(&*game), Ok(100_000));
        assert_eq!((game.orbs[0].value, game.orbs[0].age), (40_000, 100_000));

        // Serialization (`ExperienceOrb.addAdditionalSaveData`) truncates Value and Age to (short).
        let saved = orb_to_nbt(&game.orbs[0], [0; 4]);
        assert_eq!(saved.get("Value"), Some(&Tag::Short(40_000_i32 as i16)));
        assert_eq!(saved.get("Age"), Some(&Tag::Short(100_000_i32 as i16)));

        // Count validation (`ExtraCodecs.POSITIVE_INT`: 1..=i32::MAX) leaves the orb untouched.
        assert_eq!(
            orb_ref.set_count(game, 0),
            Err(ExperienceOrbWriteError::InvalidCount)
        );
        assert_eq!(
            orb_ref.set_count(game, i32::MAX as u32 + 1),
            Err(ExperienceOrbWriteError::InvalidCount)
        );
        assert_eq!(orb_ref.count(&*game), Ok(2));
        assert_eq!(game.orbs[0].count, 2);

        assert_eq!(orb_ref.set_value(game, 11), Ok(()));
        assert_eq!(orb_ref.set_count(game, 1), Ok(()));
        assert_eq!((game.orbs[0].value, game.orbs[0].count), (11, 1));
    });
}
