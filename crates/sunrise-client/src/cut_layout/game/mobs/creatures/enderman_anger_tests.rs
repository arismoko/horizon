use std::sync::{Arc, Mutex};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_level::chunk::Chunk;
use sunrise_sim::mobs::{Mob, enderman::stared_at};
use {
    sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
    sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
};

/// Keeps enderman `self.0` calm, and logs who stared at whom.
struct Calm(u32, Arc<Mutex<Vec<(u32, u128)>>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Calm {
    fn enderman_anger(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::EndermanAngerEvent,
    ) -> mod_api::Flow {
        let asked = (event.enderman.id(), event.player.uuid());
        self.1.lock().unwrap().push(asked);
        if event.enderman.id() == self.0 {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
}

/// `EnderManAngerEvent`: a guest stares straight at two endermen; the
/// mod keeps one calm, the other still counts the stare.
#[test]
#[ignore = "daily"]
fn a_mod_keeps_one_stared_at_enderman_calm() {
    let heard = Arc::default();
    let (calm, angry) = (900, 901);
    let mods = vec![mod_api::ModEntry::new(
        "calm",
        Box::new(Calm(calm, Arc::clone(&heard))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            open_to_lan(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let mut staring = guest(5);
            staring.loaded = true;
            staring.position = [8.0, 64.0, 2.0];
            game.server.lan.as_mut().unwrap().hub.guests.push(staring);
            let views = game.lan_guest_views();
            let mut view = views[0].1.clone();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:enderman")
                .unwrap()
                .clone();
            let endermen =
                [calm, angry].map(|id| Mob::new(id, kind.clone(), Vec3::new(8.0, 64.0, 10.0), 0.0));
            let eye = endermen[0].eye_y();
            view.position = Vec3::new(8.0, 64.0, 2.0);
            view.eye = [8.0, eye, 2.0];
            view.look = [0.0, 0.0, 1.0];
            view.head = None;
            let mut world = game.mob_world_base(&[]);
            assert!(world.mob_hooks.is_some(), "asked only while a mod listens");
            world.player = Some(view);
            assert!(!stared_at(&endermen[0], &world), "the mod keeps it calm");
            assert!(stared_at(&endermen[1], &world));
            assert_eq!(*heard.lock().unwrap(), [(calm, 127), (angry, 127)]);
        });
    });
}

/// Says the gaze is never disguised from mob `self.0`, and always from mob `self.1`.
struct Mask(u32, u32);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Mask {
    fn gaze_disguise(&self, _: &dyn mod_api::Host, event: &mut mod_api::GazeDisguiseEvent) {
        if event.observer.id() == self.0 {
            event.disguised = false;
        } else if event.observer.id() == self.1 {
            event.disguised = true;
        }
    }
}

/// `gaze_disguise` (#5948): under a carved pumpkin, the mod lets one enderman count the
/// stare and leaves another to vanilla's mask; bare-headed, the mod masks a third.
#[test]
#[ignore = "daily"]
fn a_mod_says_whether_a_worn_head_item_masks_the_stare() {
    let (seen, masked) = (900, 902);
    let mods = vec![mod_api::ModEntry::new("mask", Box::new(Mask(seen, masked)))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            open_to_lan(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let mut staring = guest(5);
            staring.loaded = true;
            staring.position = [8.0, 64.0, 2.0];
            game.server.lan.as_mut().unwrap().hub.guests.push(staring);
            let views = game.lan_guest_views();
            let mut view = views[0].1.clone();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:enderman")
                .unwrap()
                .clone();
            let endermen = [seen, 901, masked]
                .map(|id| Mob::new(id, kind.clone(), Vec3::new(8.0, 64.0, 10.0), 0.0));
            let eye = endermen[0].eye_y();
            view.position = Vec3::new(8.0, 64.0, 2.0);
            view.eye = [8.0, eye, 2.0];
            view.look = [0.0, 0.0, 1.0];
            view.head = Some("minecraft:carved_pumpkin".into());
            let mut world = game.mob_world_base(&[]);
            assert!(world.mob_hooks.is_some(), "asked only while a mod listens");
            world.player = Some(view.clone());
            assert!(
                stared_at(&endermen[0], &world),
                "the mod unmasks the pumpkin"
            );
            assert!(!stared_at(&endermen[1], &world), "vanilla's pumpkin masks");
            view.head = None;
            world.player = Some(view);
            assert!(
                stared_at(&endermen[1], &world),
                "bare-headed, vanilla counts it"
            );
            assert!(
                !stared_at(&endermen[2], &world),
                "the mod masks a bare head"
            );
        });
    });
}
