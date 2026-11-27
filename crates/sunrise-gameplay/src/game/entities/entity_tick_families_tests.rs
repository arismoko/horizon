//! Per-family entity tick notifications and Pre cancellation on the server.
use super::{
    ServerGame,
    native_entity_refs::tests::{prepare_entity_ticking_neighborhood, with_game},
};
use mod_api::{Hooks, HostSide};
use std::sync::{Arc, Mutex};
use sunrise_core::{geometry::Vec3, position::ChunkPos};
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
use sunrise_sim::entities::ArmorStand;
use sunrise_sim::thrown_trident::ThrownTrident;

const AT: Vec3 = Vec3::new(4.5, 64.0, 4.5);
const STAND: u32 = 71;
const TRIDENT: u32 = 72;

/// What the listener heard, in order: the phase, the entity and its type.
type Heard = Arc<Mutex<Vec<(&'static str, u32, String)>>>;
/// The entities the listener cancels in Pre.
type Cancel = Arc<Mutex<Vec<u32>>>;
/// The entities the listener discards when told their Post.
type Discard = Arc<Mutex<Vec<u32>>>;

struct Listen {
    heard: Heard,
    cancel: Cancel,
    discard: Discard,
}

#[mod_api::__subscriptions]
impl Hooks for Listen {
    fn entity_tick_pre(
        &self,
        _: HostSide<'_>,
        event: &mut mod_api::EntityTickPreEvent<'_>,
    ) -> mod_api::Flow {
        let cancel = self.cancel.lock().unwrap();
        let mut heard = self.heard.lock().unwrap();
        heard.push(("pre", event.entity, event.kind.to_owned()));
        if cancel.contains(&event.entity) {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }

    fn entity_tick_post(
        &self,
        mut host: HostSide<'_>,
        event: &mut mod_api::EntityTickPostEvent<'_>,
    ) {
        self.heard
            .lock()
            .unwrap()
            .push(("post", event.entity, event.kind.to_owned()));
        let HostSide::Server(server) = &mut host else {
            return;
        };
        if self.discard.lock().unwrap().contains(&event.entity) {
            let entity = mod_api::prelude::EntitiesHost::entity(&**server, event.entity)
                .expect("the told entity is still there");
            assert!(entity.discard(*server));
        }
    }
}

fn with_families(test: impl FnOnce(&mut ServerGame<'_>, &Heard, &Cancel, &Discard)) {
    let heard = Heard::default();
    let cancel = Cancel::default();
    let discard = Discard::default();
    let listen = Listen {
        heard: heard.clone(),
        cancel: cancel.clone(),
        discard: discard.clone(),
    };
    let entry = mod_api::ModEntry::new("tick-families", Box::new(listen));
    mod_platform::testing::with_mods(Arc::from([entry]), || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            game.build_registries();
            assert!(game.as_console(|g| g.run_command("forceload add 0 0")));
            game.sync_tick_chunks();
            assert!(game.entity_ticking_area()(4, 4));
            game.server.next_entity = 100;
            assert!(game.as_console(|g| {
                g.run_command("summon minecraft:item_frame 8 100 8 {Facing:3b}")
                    && g.run_command("summon minecraft:glow_item_frame 10 100 8 {Facing:3b}")
                    && g.run_command("summon minecraft:painting 12 100 8 {facing:0b}")
            }));
            game.armor_stands.push(ArmorStand::new(STAND, AT, 0.0));
            game.tridents
                .push(ThrownTrident::remote(TRIDENT, AT, 0.0, 0.0));
            test(game, &heard, &cancel, &discard);
        });
    });
}

fn heard(heard: &Heard) -> Vec<(&'static str, u32, String)> {
    std::mem::take(&mut *heard.lock().unwrap())
}

fn told(id: u32, kind: &str) -> [(&'static str, u32, String); 2] {
    [("pre", id, kind.to_owned()), ("post", id, kind.to_owned())]
}

#[test]
#[ignore = "daily"]
fn item_and_glow_item_frames_are_told_their_tick() {
    with_families(|game, listened, _, _| {
        assert_eq!(game.item_frames.len(), 2);
        // Summoned in this order: the item frame, then the glow item frame.
        let (frame, glow) = (game.item_frames[0].id, game.item_frames[1].id);
        heard(listened);
        game.tick_item_frames();
        let expected = [
            told(frame, "minecraft:item_frame"),
            told(glow, "minecraft:glow_item_frame"),
        ]
        .concat();
        assert_eq!(heard(listened), expected);
        assert!(game.item_frames.iter().all(|f| f.ticks == 1));
    });
}

#[test]
#[ignore = "daily"]
fn a_painting_is_told_its_tick() {
    with_families(|game, listened, _, _| {
        let id = game.paintings[0].id;
        heard(listened);
        game.tick_paintings();
        assert_eq!(heard(listened), told(id, "minecraft:painting"));
        assert_eq!(game.paintings[0].ticks, 1);
    });
}

#[test]
#[ignore = "daily"]
fn an_armor_stand_is_told_its_tick() {
    with_families(|game, listened, _, _| {
        heard(listened);
        game.tick_armor_stands().unwrap();
        assert_eq!(heard(listened), told(STAND, "minecraft:armor_stand"));
        assert_eq!(game.armor_stands[0].tick_count, 1);
    });
}

#[test]
#[ignore = "daily"]
fn a_thrown_trident_is_told_its_tick() {
    with_families(|game, listened, _, _| {
        let age = game.tridents[0].age;
        heard(listened);
        game.tick_tridents();
        assert_eq!(heard(listened), told(TRIDENT, "minecraft:trident"));
        assert_eq!(game.tridents[0].age, age + 1);
    });
}

#[test]
#[ignore = "daily"]
fn a_canceled_pre_skips_the_tick_and_the_post_of_each_family() {
    with_families(|game, listened, cancel, _| {
        let frames = game.item_frames.iter().map(|f| f.id).collect::<Vec<_>>();
        let painting = game.paintings[0].id;
        let age = game.tridents[0].age;
        cancel
            .lock()
            .unwrap()
            .extend(frames.iter().copied().chain([painting, STAND, TRIDENT]));
        heard(listened);
        game.tick_item_frames();
        game.tick_paintings();
        game.tick_armor_stands().unwrap();
        game.tick_tridents();
        let phases = heard(listened);
        assert_eq!(phases.len(), frames.len() + 3);
        assert!(phases.iter().all(|(phase, _, _)| *phase == "pre"));
        assert!(game.item_frames.iter().all(|f| f.ticks == 0));
        assert_eq!(game.paintings[0].ticks, 0);
        assert_eq!(game.armor_stands[0].tick_count, 0);
        assert_eq!(game.tridents[0].age, age);
    });
}

/// How often `id` was told `phase` in `phases`.
fn times(phases: &[(&'static str, u32, String)], phase: &str, id: u32) -> usize {
    phases
        .iter()
        .filter(|(told, entity, _)| *told == phase && *entity == id)
        .count()
}

/// The first of three entities is discarded by the listener when told its Post: the tick does not
/// panic, and the other two are each told their Pre and their Post once.
#[test]
#[ignore = "daily"]
fn discarding_the_first_in_its_post_spares_the_other_frames_paintings_and_tridents() {
    with_families(|game, listened, _, discard| {
        assert!(game.as_console(|g| {
            g.run_command("summon minecraft:item_frame 6 102 8 {Facing:3b}")
                && g.run_command("summon minecraft:painting 6 100 8 {facing:0b}")
                && g.run_command("summon minecraft:painting 2 100 8 {facing:0b}")
        }));
        game.tridents
            .push(ThrownTrident::remote(TRIDENT + 1, AT, 0.0, 0.0));
        game.tridents
            .push(ThrownTrident::remote(TRIDENT + 2, AT, 0.0, 0.0));
        let frames = game.item_frames.iter().map(|f| f.id).collect::<Vec<_>>();
        let paintings = game.paintings.iter().map(|p| p.id).collect::<Vec<_>>();
        let tridents = game.tridents.iter().map(|t| t.id).collect::<Vec<_>>();
        for family in [&frames, &paintings, &tridents] {
            assert_eq!(family.len(), 3);
            discard.lock().unwrap().push(family[0]);
        }
        heard(listened);
        game.tick_item_frames();
        game.tick_paintings();
        game.tick_tridents();
        let phases = heard(listened);
        for family in [&frames, &paintings, &tridents] {
            for &id in &family[1..] {
                assert_eq!(times(&phases, "pre", id), 1);
                assert_eq!(times(&phases, "post", id), 1);
            }
        }
    });
}
