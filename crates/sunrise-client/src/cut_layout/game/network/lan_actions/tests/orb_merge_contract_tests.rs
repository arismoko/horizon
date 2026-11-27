//! The issue #3977 merge contract through a real `Game`: `3 x 2` and `5 x 1` become
//! `11 x 1`, which saves unchanged and pays 11 once on pickup.
use mod_api::{
    ExperienceOrbCanMergeEvent, ExperienceOrbMergeEvent, ExperienceOrbRef, Flow, Hooks, ServerHost,
};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::native_entity_refs::tests::{with_game, with_resident_game};
use sunrise_sim::entities::ExperienceOrb;
use sunrise_sim::entity_nbt::{orb_to_nbt, orbs_from_nbt};
use {
    sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
    sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
};

fn orb(id: u32, x: f64, value: i32, count: u32) -> ExperienceOrb {
    let at = Vec3::new(x, 70.0, 0.5);
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

fn open(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    open_to_lan(game);
    let mut g = guest(1);
    g.loaded = true;
    g.position = [0.5, 70.0, 0.5];
    game.server.lan.as_mut().unwrap().hub.guests.push(g);
}

fn orb_handle(host: &dyn ServerHost, id: u32) -> ExperienceOrbRef {
    let entity = host.entity(id).expect("live orb entity");
    host.experience_orb(entity)
        .expect("server orb lookup")
        .expect("experience orb handle")
}

/// Forced merge of different values, then the exact total written back.
struct Normalize;
#[mod_api::__subscriptions]
impl Hooks for Normalize {
    fn experience_orb_can_merge(
        &self,
        _: &mut dyn ServerHost,
        event: &mut ExperienceOrbCanMergeEvent<'_>,
    ) {
        event.can_merge = true;
    }
    fn experience_orb_merge(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ExperienceOrbMergeEvent<'_>,
    ) -> Flow {
        if (
            event.into_value,
            event.into_count,
            event.from_value,
            event.from_count,
        ) == (3, 2, 5, 1)
        {
            let into = orb_handle(&*host, event.into);
            assert_eq!(into.set_value(host, 11), Ok(()));
            assert_eq!(into.set_count(host, 1), Ok(()));
        }
        Flow::Continue
    }
}

/// The merge hook writes the total to `into` (with an age), then says no: both orbs stay as they were.
struct Cancel;
#[mod_api::__subscriptions]
impl Hooks for Cancel {
    fn experience_orb_can_merge(
        &self,
        _: &mut dyn ServerHost,
        event: &mut ExperienceOrbCanMergeEvent<'_>,
    ) {
        event.can_merge = true;
    }
    fn experience_orb_merge(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ExperienceOrbMergeEvent<'_>,
    ) -> Flow {
        let into = orb_handle(&*host, event.into);
        assert_eq!(into.set_value(host, 11), Ok(()));
        assert_eq!(into.set_count(host, 1), Ok(()));
        assert_eq!(into.set_age(host, 99), Ok(()));
        Flow::Cancel
    }
}

/// Lets every merge through and records what the hook was told: (`into_value`, `into_count`,
/// `from_value`, `from_count`).
struct Record;
static SEEN: std::sync::Mutex<Vec<(i32, u32, i32, u32)>> = std::sync::Mutex::new(Vec::new());
#[mod_api::__subscriptions]
impl Hooks for Record {
    fn experience_orb_can_merge(
        &self,
        _: &mut dyn ServerHost,
        event: &mut ExperienceOrbCanMergeEvent<'_>,
    ) {
        event.can_merge = true;
    }
    fn experience_orb_merge(
        &self,
        _: &mut dyn ServerHost,
        event: &mut ExperienceOrbMergeEvent<'_>,
    ) -> Flow {
        SEEN.lock().unwrap().push((
            event.into_value,
            event.into_count,
            event.from_value,
            event.from_count,
        ));
        Flow::Continue
    }
}

/// Writes the total, then changes the source so the decision no longer applies.
struct Stale;
#[mod_api::__subscriptions]
impl Hooks for Stale {
    fn experience_orb_can_merge(
        &self,
        _: &mut dyn ServerHost,
        event: &mut ExperienceOrbCanMergeEvent<'_>,
    ) {
        event.can_merge = true;
    }
    fn experience_orb_merge(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ExperienceOrbMergeEvent<'_>,
    ) -> Flow {
        let into = orb_handle(&*host, event.into);
        assert_eq!(into.set_value(host, 11), Ok(()));
        assert_eq!(into.set_count(host, 1), Ok(()));
        let from = orb_handle(&*host, event.from);
        assert_eq!(from.set_value(host, 6), Ok(()));
        Flow::Continue
    }
}

fn mods(hooks: impl Hooks + 'static) -> mod_api::Mods {
    vec![mod_api::ModEntry::new("orb-contract", Box::new(hooks))].into()
}

#[test]
#[ignore = "daily"]
fn a_resident_merge_of_three_times_two_and_five_saves_and_pays_eleven() {
    let mut paid = 0;
    mod_platform::testing::with_mods(mods(Normalize), || {
        with_resident_game(|game| {
            open(game);
            game.orbs.push(orb(1, 0.5, 3, 2));
            game.orbs.push(orb(41, 0.7, 5, 1));
            game.merge_orb(0);
            assert_eq!(game.orbs.len(), 1);
            assert_eq!((game.orbs[0].value, game.orbs[0].count), (11, 1));
            let saved = orb_to_nbt(&game.orbs[0], [0; 4]);
            let back = orbs_from_nbt(&saved);
            assert_eq!((back[0].value, back[0].count), (11, 1));
            game.guest_orbs(0);
            assert!(game.orbs.is_empty());
            paid = game.server.lan.as_ref().unwrap().hub.guests[0].xp_total;
        });
    });
    assert_eq!(paid, 11);
}

#[test]
#[ignore = "daily"]
fn a_spawning_merge_of_three_times_two_and_five_saves_and_pays_eleven() {
    let mut paid = 0;
    mod_platform::testing::with_mods(mods(Normalize), || {
        with_resident_game(|game| {
            open(game);
            game.orbs.push(orb(41, 0.5, 3, 2));
            game.award_orb([0.5, 70.0, 0.5], 5, 1);
            assert_eq!(game.orbs.len(), 1);
            assert_eq!((game.orbs[0].value, game.orbs[0].count), (11, 1));
            let back = orbs_from_nbt(&orb_to_nbt(&game.orbs[0], [0; 4]));
            assert_eq!((back[0].value, back[0].count), (11, 1));
            game.guest_orbs(0);
            assert!(game.orbs.is_empty());
            paid = game.server.lan.as_ref().unwrap().hub.guests[0].xp_total;
        });
    });
    assert_eq!(paid, 11);
}

#[test]
#[ignore = "daily"]
fn a_canceled_merge_keeps_both_orbs_as_they_were() {
    mod_platform::testing::with_mods(mods(Cancel), || {
        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.orbs.push(orb(1, 0.5, 3, 2));
            game.orbs.push(orb(41, 0.7, 5, 1));
            game.merge_orb(0);
            let left: Vec<_> = game
                .orbs
                .iter()
                .map(|o| (o.id, o.value, o.count, o.age))
                .collect();
            assert_eq!(left, [(1, 3, 2, 0), (41, 5, 1, 0)]);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_canceled_spawning_merge_puts_the_destination_back_and_spawns_its_own_orb() {
    mod_platform::testing::with_mods(mods(Cancel), || {
        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.orbs.push(orb(41, 0.5, 3, 2));
            game.award_orb([0.5, 70.0, 0.5], 5, 1);
            let left: Vec<_> = game
                .orbs
                .iter()
                .map(|o| (o.value, o.count, o.age))
                .collect();
            assert_eq!(left[0], (3, 2, 0));
            assert_eq!(left.len(), 2, "the refused orb spawns on its own");
            assert_eq!((left[1].0, left[1].1), (5, 1));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_spawning_merge_tells_the_hook_the_signed_destination_value() {
    SEEN.lock().unwrap().clear();
    mod_platform::testing::with_mods(mods(Record), || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.orbs.push(orb(41, 0.5, -32768, 1));
            game.award_orb([0.5, 70.0, 0.5], 5, 1);
        });
    });
    assert_eq!(*SEEN.lock().unwrap(), [(-32768, 1, 5, 1)]);
}

#[test]
#[ignore = "daily"]
fn a_changed_source_refuses_the_merge_and_puts_the_written_orb_back() {
    mod_platform::testing::with_mods(mods(Stale), || {
        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.orbs.push(orb(1, 0.5, 3, 2));
            game.orbs.push(orb(41, 0.7, 5, 1));
            game.merge_orb(0);
            let left: Vec<_> = game.orbs.iter().map(|o| (o.value, o.count)).collect();
            assert_eq!(left, [(3, 2), (6, 1)]);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_count_past_what_a_save_holds_refuses_the_merge() {
    with_game(|game| {
        game.orbs.push(orb(1, 0.5, 3, i32::MAX as u32));
        game.orbs.push(orb(41, 0.7, 3, 1));
        game.merge_orb(0);
        assert_eq!(game.orbs.len(), 2);
        assert_eq!(game.orbs[0].count, i32::MAX as u32);
    });
}
