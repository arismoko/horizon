use std::sync::Arc;
use std::sync::Mutex;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::boats_common::Boat;
use sunrise_mod_runtime::attachments::{
    AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
};

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "boat_lifetime",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync_codec: None,
};

#[test]
fn a_tick_clone_follows_its_join_but_cannot_replace_a_later_resident() {
    let mut boats = sunrise_sim::join_seq::JoinList::<Boat, false>::default();
    for id in [1, 2] {
        boats.push(Boat::new(id, "oak".into(), Vec3::ZERO, 0.0));
    }
    let mut clone = boats[1].clone();
    clone.age = 7;
    boats.remove(0);
    assert!(boats.store_back(1, clone));
    assert_eq!(boats[0].id, 2);
    assert_eq!(boats[0].age, 7);
    let stale = boats.pop().unwrap().clone();
    boats.push(Boat::new(3, "oak".into(), Vec3::ZERO, 0.0));
    assert!(!boats.store_back(0, stale));
    assert_eq!(boats[0].id, 3);
    assert_eq!(boats[0].age, 0);
}

#[test]
#[ignore = "daily"]
fn a_bound_boat_keeps_its_registration_through_ticks_until_discard() {
    bound_boat_server_case(|game, step| {
        game.tick_boats();
        assert_eq!(game.boats[0].age, step);
        assert_eq!(game.boats[0].hurt_time, 6 - step);
        assert!(game.boats[0].position.y < 70.0);
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_bound_boat_commits_the_driven_client_tick() {
    bound_boat_client_case(|game, step| {
        game.riding = Some(71);
        assert!(game.remote_ride(sunrise_sim::sprint::Keys {
            forward: true,
            ..Default::default()
        },));
        assert_eq!(game.boats[0].hurt_time, 6 - step);
        assert!(game.boats[0].input[2]);
        assert!(game.boats[0].position.y < 70.0);
    });
}

#[test]
#[ignore = "daily"]
fn a_bound_ridden_boat_commits_the_server_leash_tick() {
    bound_boat_server_case(|game, step| {
        game.riding = Some(71);
        game.tick_boats();
        assert_eq!(game.boats[0].age, step);
        assert_eq!(game.boats[0].hurt_time, 6);
        assert_eq!(game.boats[0].position.y, 70.0);
    });
}

fn bound_boat_server_case(tick: impl Fn(&mut sunrise_gameplay::game::ServerGame<'_>, i32)) {
    sunrise_gameplay::game::native_entity_refs::tests::with_resident_game(|game| {
        game.server.tickets.0.update(None);
        game.boats
            .push(Boat::new(71, "oak".into(), Vec3::new(0.5, 70.0, 0.5), 0.0));
        game.boats.update(0, |boat| boat.hurt_time = 6);
        let holder = {
            let host: &dyn mod_api::ServerHost = game;
            host.entity(71).unwrap()
        };
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut values = Attachments::default();
        VALUE.set(&mut values, Holder::Entity(71), 10).unwrap();
        let original = game.boats.update(0, |boat| {
            boat.attachment_lifetime().bind(&mut values, &index, holder);
            boat.attachment_lifetime().registration().unwrap()
        });
        let key = ("test".into(), "boat_lifetime".into());
        let before = index.lock().unwrap().snapshot(&key);
        assert_eq!(before.len(), 1);
        for step in 1..=3 {
            tick(game, step);
            let current = game
                .boats
                .update(0, |boat| boat.attachment_lifetime().registration().unwrap());
            assert!(original.same_registration(&current));
            assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
            assert_eq!(VALUE.get(&mut values, Holder::Entity(71)), Ok(10));
        }
        let taken = game.take_entities();
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        game.restore_entities(taken);
        VALUE.set(&mut values, Holder::Entity(71), 20).unwrap();
        let after = index.lock().unwrap().snapshot(&key);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].holder, holder);
        assert_eq!(after[0].entry.read(&VALUE), Ok(&20));
        game.persist
            .removals
            .push(sunrise_gameplay_common::game::commands_common::Target::Boat(71));
        let clone = game.boats[0].clone();
        assert!(!game.store_boat_back(0, clone));
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        game.native_removals_flush();
        assert!(game.boats.is_empty());
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        VALUE.set(&mut values, Holder::Entity(71), 30).unwrap();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
        assert_eq!(after[0].entry.read(&VALUE), Ok(&20));
    });
}

#[cfg(feature = "client")]
fn bound_boat_client_case(tick: impl Fn(&mut sunrise_client_gameplay::game::ClientGame<'_>, i32)) {
    super::super::native_entity_refs::tests::with_client_game(|game| {
        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "BoatLifetimeTest").unwrap();
        game.client.remote.as_mut().unwrap().joined = true;
        game.client.remote.as_mut().unwrap().first_passenger = true;
        game.boats
            .push(Boat::new(71, "oak".into(), Vec3::new(0.5, 70.0, 0.5), 0.0));
        game.boats.update(0, |boat| boat.hurt_time = 6);
        let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 71);
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut values = Attachments::default();
        VALUE.set(&mut values, Holder::Entity(71), 10).unwrap();
        let original = game.boats.update(0, |boat| {
            boat.attachment_lifetime().bind(&mut values, &index, holder);
            boat.attachment_lifetime().registration().unwrap()
        });
        let key = ("test".into(), "boat_lifetime".into());
        let before = index.lock().unwrap().snapshot(&key);
        assert_eq!(before.len(), 1);
        for step in 1..=3 {
            tick(game, step);
            let current = game
                .boats
                .update(0, |boat| boat.attachment_lifetime().registration().unwrap());
            assert!(original.same_registration(&current));
            assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
            assert_eq!(VALUE.get(&mut values, Holder::Entity(71)), Ok(10));
        }
        VALUE.set(&mut values, Holder::Entity(71), 20).unwrap();
        let after = index.lock().unwrap().snapshot(&key);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].holder, holder);
        assert_eq!(after[0].entry.read(&VALUE), Ok(&20));
        game.persist
            .removals
            .push(sunrise_gameplay_common::game::commands_common::Target::Boat(71));
        let clone = game.boats[0].clone();
        assert!(!game.store_boat_back(0, clone));
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        game.boats.clear();
        assert!(game.boats.is_empty());
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        VALUE.set(&mut values, Holder::Entity(71), 30).unwrap();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
        assert_eq!(after[0].entry.read(&VALUE), Ok(&20));
    });
}
