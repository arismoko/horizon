#![cfg(test)]

use mod_api::{EntityEnteringSectionEvent, EntityResidency, Hooks, HostSide};
use mod_api::{EntityRef, WorldEpoch};
#[cfg(feature = "client")]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::entity_sections::{
    Crossing, EntitySections, LiveSectionHost, Scope, SectionWriter, section_coord, section_of,
    tell_live,
};
#[cfg(feature = "client")]
use sunrise_sim::mobs::living::section_writes::PositionWrite;
#[cfg(feature = "client")]
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::{fixture_resident_column, with_game},
    sunrise_gameplay_common::game::commands_common::Target,
};

#[test]
fn section_read_backend_refuses_reads_outside_its_snapshot() {
    let host = LiveSectionHost::new(None);
    let entity = EntityRef::new(mod_api::WorldEpoch::new(1), 1);
    assert_eq!(
        entity.piston_push_reaction(&host),
        Err(mod_api::LevelReadError::Unavailable)
    );
    assert_eq!(
        entity.is_hanging(&host),
        Err(mod_api::LevelReadError::Unavailable)
    );
    let player = mod_api::__private::game::player_ref(entity, 1);
    assert_eq!(
        player.collision_flags(&host),
        Err(mod_api::LevelReadError::Unavailable)
    );
    let menu = mod_api::__private::game::menu_ref(player, 1, 1);

    assert_eq!(entity.bounding_box(&host), None);
    assert_eq!(entity.residency(&host), EntityResidency::Gone);
    assert_eq!(
        player.supporting_block(&host),
        Err(mod_api::LevelReadError::Unavailable)
    );
    assert_eq!(
        player.on_block(&host),
        Err(mod_api::LevelReadError::Unavailable)
    );
    assert!(matches!(
        menu.merchant_offers(&host),
        Err(mod_api::LevelReadError::Unavailable)
    ));
}

#[test]
fn live_section_host_reports_moved_entity_residency_in_callback() {
    type Observed = Arc<Mutex<Vec<(bool, EntityResidency, EntityResidency)>>>;
    struct ResidencyListener(Observed, EntityRef);

    #[mod_api::__subscriptions]
    impl Hooks for ResidencyListener {
        fn entity_entering_section(
            &self,
            host: HostSide<'_>,
            event: &mut EntityEnteringSectionEvent,
        ) {
            let (client, moved, other) = match host {
                HostSide::Server(host) => {
                    (false, event.entity.residency(host), self.1.residency(host))
                }
                HostSide::Client(host) => {
                    (true, event.entity.residency(host), self.1.residency(host))
                }
            };
            self.0.lock().unwrap().push((client, moved, other));
        }
    }

    let observed = Observed::default();
    let other = EntityRef::new(WorldEpoch::new(1), 99);
    let mods = vec![mod_api::ModEntry::new(
        "section-residency",
        Box::new(ResidencyListener(observed.clone(), other)),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let sections = std::cell::RefCell::new(EntitySections::default());
        let w = scope(Overworld);

        let player = EntityRef::new(w.epoch, 7);
        sections
            .borrow_mut()
            .moved(w, 7, Some([8.0, 64.0, 8.0]), [24.0, 64.0, 8.0]);
        tell_live(
            &sections,
            SectionWriter {
                entity: player,
                residency: EntityResidency::InPlace,
                position: [24.0, 64.0, 8.0],
                player: None,
            },
            false,
        );

        let mob = EntityRef::new(w.epoch, 12);
        sections
            .borrow_mut()
            .moved(w, 12, Some([8.0, 64.0, 8.0]), [24.0, 64.0, 8.0]);
        tell_live(
            &sections,
            SectionWriter {
                entity: mob,
                residency: EntityResidency::Busy,
                position: [24.0, 64.0, 8.0],
                player: None,
            },
            true,
        );
    });

    assert_eq!(
        *observed.lock().unwrap(),
        vec![
            (true, EntityResidency::InPlace, EntityResidency::Gone),
            (false, EntityResidency::Busy, EntityResidency::Gone),
        ]
    );
}

/// What each told crossing carried: client side, entity id, old, new, chunk changed.
#[cfg(feature = "client")]
type Calls = Arc<Mutex<Vec<(bool, u32, [i32; 3], [i32; 3], bool)>>>;

#[cfg(feature = "client")]
struct Listener(Calls);

#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn entity_entering_section(&self, host: HostSide<'_>, event: &mut EntityEnteringSectionEvent) {
        self.0.lock().unwrap().push((
            matches!(host, HostSide::Client(_)),
            event.entity.id(),
            event.old_section,
            event.new_section,
            event.chunk_changed,
        ));
    }
}

#[cfg(feature = "client")]
type Positions = Arc<Mutex<Vec<(Option<[f64; 3]>, EntityResidency)>>>;

#[cfg(feature = "client")]
struct PositionListener(Positions);

#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for PositionListener {
    fn entity_entering_section(&self, host: HostSide<'_>, event: &mut EntityEnteringSectionEvent) {
        let (position, residency) = match host {
            HostSide::Server(host) => (event.entity.position(host), event.entity.residency(host)),
            HostSide::Client(host) => (event.entity.position(host), event.entity.residency(host)),
        };
        self.0.lock().unwrap().push((position, residency));
    }
}

#[cfg(feature = "client")]
fn observe(test: impl FnOnce(Calls)) {
    let calls = Calls::default();
    let mods = vec![mod_api::ModEntry::new(
        "sections",
        Box::new(Listener(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || test(calls));
}

#[cfg(feature = "client")]
fn observe_positions(test: impl FnOnce(Positions)) {
    let positions = Positions::default();
    let mods = vec![mod_api::ModEntry::new(
        "section-positions",
        Box::new(PositionListener(positions.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || test(positions));
}

#[cfg(feature = "client")]
type CallbackCount = Arc<AtomicUsize>;

#[cfg(feature = "client")]
struct CountListener(CallbackCount);

#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for CountListener {
    fn entity_entering_section(&self, _: HostSide<'_>, _: &mut EntityEnteringSectionEvent) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(feature = "client")]
fn observe_count(test: impl FnOnce(CallbackCount)) {
    let count = Arc::new(AtomicUsize::new(0));
    let mods = vec![mod_api::ModEntry::new(
        "section-count",
        Box::new(CountListener(count.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || test(count));
}

fn scope(level: sunrise_gameplay_common::game::Dim) -> Scope {
    Scope {
        epoch: WorldEpoch::new(1),
        level: level.key(),
    }
}

use {sunrise_gameplay_common::game::Dim::Nether, sunrise_gameplay_common::game::Dim::Overworld};

/// The crossings queued since the last call, as a telling takes them.
fn told(sections: &mut EntitySections) -> Vec<Seen> {
    if !sections.begin_telling() {
        return Vec::new();
    }
    let crossings = sections.queue.drain(..).map(seen).collect();
    sections.end_telling();
    crossings
}

/// A crossing's entity and its two sections.
type Seen = (u32, [i32; 3], [i32; 3]);

fn seen(crossing: Crossing) -> Seen {
    (crossing.id, crossing.old, crossing.new)
}

fn crossing(id: u32, old: [i32; 3], new: [i32; 3]) -> Seen {
    (id, old, new)
}

#[test]
fn a_section_is_the_floored_block_shifted_by_four() {
    // Blocks 0..=15 are section 0, -1..=-16 section -1, -17 section -2.
    assert_eq!(section_of([0.0, 15.999, 15.0]), [0, 0, 0]);
    assert_eq!(section_of([16.0, 64.0, 31.5]), [1, 4, 1]);
    assert_eq!(section_of([-0.001, -16.0, -16.001]), [-1, -1, -2]);
    // `Mth.floor` is `(int) Math.floor(v)`: Java's cast saturates and NaN is 0.
    assert_eq!(section_coord(f64::NAN), 0);
    assert_eq!(section_coord(1e300), i32::MAX >> 4);
    assert_eq!(section_coord(f64::from(i32::MIN)), -134_217_728);
    assert_eq!(section_coord(f64::from(i32::MIN) - 0.5), -134_217_728);
    assert_eq!(section_coord(f64::NEG_INFINITY), -134_217_728);
}

#[test]
fn moving_inside_a_section_is_quiet_and_each_edge_is_one_crossing() {
    let mut s = EntitySections::default();
    let w = scope(Overworld);
    s.placed(w, 5, [15.5, 70.0, 3.0]);
    s.moved(w, 5, None, [1.0, 64.0, 15.9]);
    assert!(told(&mut s).is_empty());
    // East over x = 16, then down over y = 64, then north into negative z.
    s.moved(w, 5, None, [16.2, 64.0, 15.9]);
    s.moved(w, 5, None, [16.2, 63.9, 15.9]);
    s.moved(w, 5, None, [16.2, 63.9, -0.1]);
    assert_eq!(
        told(&mut s),
        [
            crossing(5, [0, 4, 0], [1, 4, 0]),
            crossing(5, [1, 4, 0], [1, 3, 0]),
            crossing(5, [1, 3, 0], [1, 3, -1]),
        ]
    );
}

#[test]
fn two_moves_in_one_tick_through_three_sections_are_two_crossings() {
    let mut s = EntitySections::default();
    let w = scope(Overworld);
    // A, then B and C before anything is told: A -> B and B -> C, not A -> C.
    s.placed(w, 9, [8.0, 70.0, 8.0]);
    s.moved(w, 9, None, [24.0, 70.0, 8.0]);
    s.moved(w, 9, None, [-1000.0, -40.0, 5000.0]);
    assert_eq!(
        told(&mut s),
        [
            crossing(9, [0, 4, 0], [1, 4, 0]),
            crossing(9, [1, 4, 0], [-63, -3, 312]),
        ]
    );
}

#[test]
fn a_move_out_and_back_in_one_tick_is_two_crossings() {
    let mut s = EntitySections::default();
    let w = scope(Overworld);
    s.placed(w, 3, [0.5, 70.0, 0.5]);
    s.moved(w, 3, None, [-0.5, 70.0, 0.5]);
    s.moved(w, 3, None, [0.5, 70.0, 0.5]);
    assert_eq!(
        told(&mut s),
        [
            crossing(3, [0, 4, 0], [-1, 4, 0]),
            crossing(3, [-1, 4, 0], [0, 4, 0]),
        ]
    );
}

#[test]
fn a_move_before_the_first_tick_is_told() {
    let mut s = EntitySections::default();
    let w = scope(Overworld);
    // Filed where it joined, then moved before its first tick.
    s.placed(w, 1, [0.5, 70.0, 0.5]);
    s.moved(w, 1, None, [0.5, 90.0, 0.5]);
    // Moved before its join was filed: the boundary's own start is where it was.
    s.moved(w, 2, Some([40.0, 0.0, 40.0]), [56.0, 0.0, 40.0]);
    // Neither filed nor a known start: filed untold.
    s.moved(w, 4, None, [56.0, 0.0, 40.0]);
    assert_eq!(
        told(&mut s),
        [
            crossing(1, [0, 4, 0], [0, 5, 0]),
            crossing(2, [2, 0, 2], [3, 0, 2]),
        ]
    );
}

#[test]
fn another_level_a_respawn_and_another_world_start_untold() {
    let mut s = EntitySections::default();
    s.placed(scope(Overworld), 4, [0.5, 70.0, 0.5]);
    // Changing level files it in the new one, untold; a move there is told.
    s.moved(scope(Nether), 4, None, [100.0, 70.0, 0.5]);
    s.moved(scope(Nether), 4, None, [116.0, 70.0, 0.5]);
    assert_eq!(told(&mut s), [crossing(4, [6, 4, 0], [7, 4, 0])]);
    // Forgotten (a respawn, a removal): filed again untold.
    s.forget(4);
    s.moved(scope(Nether), 4, None, [0.5, 70.0, 0.5]);
    assert!(told(&mut s).is_empty());
    // Another world open reuses ids: what the last one filed is dropped.
    let next = Scope {
        epoch: WorldEpoch::new(2),
        level: Nether.key(),
    };
    s.moved(next, 4, None, [100.0, 70.0, 0.5]);
    assert!(told(&mut s).is_empty());
}

#[test]
fn nested_writes_stay_queued_until_the_current_event_returns() {
    let mut s = EntitySections::default();
    let w = scope(Overworld);
    s.placed(w, 6, [0.5, 70.0, 0.5]);
    s.moved(w, 6, None, [16.5, 70.0, 0.5]);
    assert!(s.begin_telling());
    assert_eq!(
        s.queue.pop_front().map(seen),
        Some(crossing(6, [0, 4, 0], [1, 4, 0]))
    );
    // The listener moves two entities, then removes the second before its event runs.
    s.moved(w, 6, None, [32.5, 70.0, 0.5]);
    s.moved(w, 7, Some([0.5, 70.0, 0.5]), [16.5, 70.0, 0.5]);
    assert!(!s.begin_telling());
    s.forget(7);
    // Both filed crossings stay: a removal deletes none.
    assert_eq!(
        s.queue.pop_front().map(seen),
        Some(crossing(6, [1, 4, 0], [2, 4, 0]))
    );
    assert_eq!(
        s.queue.pop_front().map(seen),
        Some(crossing(7, [0, 4, 0], [1, 4, 0]))
    );
    assert!(s.queue.is_empty());
    s.end_telling();
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn no_listener_files_no_entity() {
    with_game(|game| {
        let id = game
            .spawn_mob_spawned(
                "cow",
                [1.0, 70.0, 3.0],
                sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
            )
            .unwrap();
        game.teleport_to(Target::Mob(id), [40.0, 70.0, 3.0], None, true);
        assert!(game.native_lifecycle.entity_sections.filed.is_empty());
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_server_mob_moved_twice_in_one_tick_is_told_each_move() {
    observe(|calls| {
        with_game(|game| {
            let id = game
                .spawn_mob_spawned(
                    "cow",
                    [1.0, 70.0, 3.0],
                    sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                )
                .unwrap();
            // Before its first tick, A -> B -> C by two teleports.
            game.teleport_to(Target::Mob(id), [-0.25, 70.0, 3.0], None, true);
            game.teleport_to(Target::Mob(id), [-0.25, 130.0, 3.0], None, true);
            // Out of the section and back: two more.
            game.teleport_to(Target::Mob(id), [-0.25, 130.0, 20.0], None, true);
            game.teleport_to(Target::Mob(id), [-0.25, 130.0, 3.0], None, true);
            // Inside the section: none.
            game.teleport_to(Target::Mob(id), [-8.0, 129.0, 9.0], None, true);
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[
                    (false, id, [0, 4, 0], [-1, 4, 0], true),
                    (false, id, [-1, 4, 0], [-1, 8, 0], false),
                    (false, id, [-1, 8, 0], [-1, 8, 1], true),
                    (false, id, [-1, 8, 1], [-1, 8, 0], true),
                ]
            );
        })
    });
}

/// A cow joined at `at` in a loaded, empty chunk (0, 0), floating (`NoGravity`): filed by its
/// join, as `addEntity` files it.
#[cfg(feature = "client")]
fn floating_cow(game: &mut sunrise_gameplay::game::ServerGame<'_>, at: [f64; 3]) -> u32 {
    if !game.world.chunks.contains_key(&(0, 0)) {
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
    }
    let id = game
        .spawn_mob_spawned(
            "cow",
            at,
            sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
        )
        .unwrap();
    assert!(game.native_joins_flush(false, 0));
    let i = game.mobs.iter().position(|m| m.id == id).unwrap();
    game.mobs.update(i, |m| m.no_gravity = true);
    id
}

#[cfg(feature = "client")]
fn rising_cow(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    at: [f64; 3],
    no_physics: bool,
) -> u32 {
    let id = floating_cow(game, at);
    let entity = game.entity(id).unwrap();
    assert!(entity.set_no_physics(game, no_physics));
    let i = game.mobs.iter().position(|m| m.id == id).unwrap();
    game.mobs.update(i, |m| {
        m.ai.goals.clear();
        m.velocity = sunrise_core::geometry::Vec3::new(0.0, 0.5, 0.0);
    });
    id
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn borrowed_no_physics_moves_with_a_passive_listener_keep_their_order() {
    observe_positions(|positions| {
        with_game(|game| {
            let first = rising_cow(game, [4.5, 111.75, 8.5], true);
            let same_section = rising_cow(game, [8.5, 100.0, 8.5], true);
            let last = rising_cow(game, [12.5, 127.75, 8.5], true);
            game.tick_creature_entities().unwrap();
            assert_eq!(
                positions.lock().unwrap().as_slice(),
                &[
                    (Some([4.5, 112.25, 8.5]), EntityResidency::InPlace),
                    (Some([12.5, 128.25, 8.5]), EntityResidency::InPlace),
                ]
            );
            for (id, at) in [
                (first, [4.5, 112.25, 8.5]),
                (same_section, [8.5, 100.5, 8.5]),
                (last, [12.5, 128.25, 8.5]),
            ] {
                assert_eq!(game.mob_object(id).unwrap().position.components(), at);
            }
        })
    });
}

/// A goal that, when it starts, teleports its mob to each of its targets in turn, once: the
/// `snapTo` an enderman's, a pet's or a shulker's teleport makes in its tick.
#[cfg(feature = "client")]
#[derive(Clone, Debug)]
struct Teleports(Vec<[f64; 3]>);

#[cfg(feature = "client")]
impl sunrise_sim::mobs::goals::Goal for Teleports {
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(
        &mut self,
        _: &mut sunrise_sim::mobs::Mob,
        _: &mut sunrise_sim::mobs::goals::Cx<'_, '_, '_>,
    ) -> bool {
        !self.0.is_empty()
    }
    fn start(
        &mut self,
        m: &mut sunrise_sim::mobs::Mob,
        cx: &mut sunrise_sim::mobs::goals::Cx<'_, '_, '_>,
    ) {
        for [x, y, z] in self.0.drain(..) {
            let at = sunrise_core::geometry::Vec3::new(x, y, z);
            sunrise_sim::mobs::living::teleports::snap_to_with(m, at, cx.w.section_writer.clone());
        }
    }
    fn box_clone(&self) -> Box<dyn sunrise_sim::mobs::goals::Goal> {
        Box::new(self.clone())
    }
}

/// The cow `id`, moving at `velocity`, runs real mob ticks until one has teleported it through
/// `targets` (its [`Teleports`] goal starting) and then walked it on, in that same tick.
#[cfg(feature = "client")]
fn teleport_then_walk(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    id: u32,
    targets: &[[f64; 3]],
    velocity: [f64; 3],
) {
    use sunrise_core::geometry::Vec3;
    let i = game.mobs.iter().position(|m| m.id == id).unwrap();
    let start = game.mobs[i].position.y;
    game.mobs.update(i, |m| {
        m.ai.goals.add(0, Teleports(targets.to_vec()));
        m.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
    });
    for _ in 0..4 {
        game.tick_creature_entities().unwrap();
        let at = game.mobs.iter().find(|m| m.id == id).unwrap().position;
        if (at.y - start).abs() > 5.0 {
            return;
        }
    }
    panic!("the cow's tick never teleported it");
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_mobs_teleport_and_its_walk_in_one_tick_are_two_crossings() {
    observe(|calls| {
        with_game(|game| {
            // Section y 6 (blocks 96 to 111), teleported near the top of y 7, then 0.6 up into y 8.
            let id = floating_cow(game, [8.5, 100.0, 8.5]);
            teleport_then_walk(game, id, &[[8.5, 127.75, 8.5]], [0.0, 0.6, 0.0]);
            let at = game.mobs.iter().find(|m| m.id == id).unwrap().position;
            assert!(at.y >= 128.0, "the walk took it into y 8: {at:?}");
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[
                    (false, id, [0, 6, 0], [0, 7, 0], false),
                    (false, id, [0, 7, 0], [0, 8, 0], false),
                ]
            );
        })
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_real_mob_tick_exposes_each_intermediate_position_to_the_listener() {
    observe_positions(|positions| {
        with_game(|game| {
            let id = floating_cow(game, [8.5, 100.0, 8.5]);
            teleport_then_walk(game, id, &[[8.5, 127.75, 8.5]], [0.0, 0.6, 0.0]);
            let positions = positions.lock().unwrap();
            assert_eq!(positions.len(), 2);
            assert_eq!(
                positions[0],
                (Some([8.5, 127.75, 8.5]), EntityResidency::Busy)
            );
            assert!(positions[1].0.is_some_and(|at| at[1] >= 128.0));
            assert_eq!(positions[1].1, EntityResidency::Busy);
        })
    });
}

#[cfg(feature = "client")]
#[derive(Clone, Debug)]
struct TeleportsAndChecks {
    callbacks: CallbackCount,
    failed: Arc<AtomicBool>,
    ran: bool,
}

#[cfg(feature = "client")]
impl sunrise_sim::mobs::goals::Goal for TeleportsAndChecks {
    fn flags(&self) -> u8 {
        0
    }

    fn can_use(
        &mut self,
        _: &mut sunrise_sim::mobs::Mob,
        _: &mut sunrise_sim::mobs::goals::Cx<'_, '_, '_>,
    ) -> bool {
        !self.ran
    }

    fn start(
        &mut self,
        m: &mut sunrise_sim::mobs::Mob,
        cx: &mut sunrise_sim::mobs::goals::Cx<'_, '_, '_>,
    ) {
        self.ran = true;
        let first = sunrise_core::geometry::Vec3::new(8.5, 120.0, 8.5);
        sunrise_sim::mobs::living::teleports::set_pos_with(m, first, cx.w.section_writer.clone());
        if self.callbacks.load(Ordering::SeqCst) != 1 {
            self.failed.store(true, Ordering::SeqCst);
        }
        let second = sunrise_core::geometry::Vec3::new(8.5, 143.75, 8.5);
        sunrise_sim::mobs::living::teleports::set_pos_with(m, second, cx.w.section_writer.clone());
        if self.callbacks.load(Ordering::SeqCst) != 2 {
            self.failed.store(true, Ordering::SeqCst);
        }
    }

    fn box_clone(&self) -> Box<dyn sunrise_sim::mobs::goals::Goal> {
        Box::new(self.clone())
    }
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_real_mob_tick_tells_inside_each_position_write() {
    observe_count(|callbacks| {
        with_game(|game| {
            let id = floating_cow(game, [8.5, 100.0, 8.5]);
            game.tick_creature_entities().unwrap();
            callbacks.store(0, Ordering::SeqCst);
            let failed = Arc::new(AtomicBool::new(false));
            let i = game.mobs.iter().position(|m| m.id == id).unwrap();
            game.mobs.update(i, |m| {
                m.ai.goals.add(
                    0,
                    TeleportsAndChecks {
                        callbacks: callbacks.clone(),
                        failed: failed.clone(),
                        ran: false,
                    },
                )
            });
            // Mob.serverAiStep only scans for new goals on alternating ticks.
            // The first tick after installing this goal may tick running goals only.
            for _ in 0..2 {
                game.tick_creature_entities().unwrap();
                if callbacks.load(Ordering::SeqCst) != 0 {
                    break;
                }
            }
            assert!(!failed.load(Ordering::SeqCst));
            assert_eq!(callbacks.load(Ordering::SeqCst), 2);
        })
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_mobs_teleport_out_and_its_walk_back_in_one_tick_are_two_crossings() {
    observe(|calls| {
        with_game(|game| {
            // Teleported just over y = 112 into section y 7, then 0.6 down, back into y 6.
            let id = floating_cow(game, [8.5, 100.0, 8.5]);
            teleport_then_walk(game, id, &[[8.5, 112.25, 8.5]], [0.0, -0.6, 0.0]);
            let at = game.mobs.iter().find(|m| m.id == id).unwrap().position;
            assert!(at.y < 112.0, "the walk took it back into y 6: {at:?}");
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[
                    (false, id, [0, 6, 0], [0, 7, 0], false),
                    (false, id, [0, 7, 0], [0, 6, 0], false),
                ]
            );
        })
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_mob_that_teleports_twice_in_its_tick_then_walks_is_three_crossings() {
    observe(|calls| {
        with_game(|game| {
            // y 6, then in one tick teleported into y 7 and on near the top of y 8, then 0.6 up
            // into y 9: A -> B, B -> C and C -> D, not only the last teleport.
            let id = floating_cow(game, [8.5, 100.0, 8.5]);
            let targets = [[8.5, 120.0, 8.5], [8.5, 143.75, 8.5]];
            teleport_then_walk(game, id, &targets, [0.0, 0.6, 0.0]);
            let at = game.mobs.iter().find(|m| m.id == id).unwrap().position;
            assert!(at.y >= 144.0, "the walk took it into y 9: {at:?}");
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[
                    (false, id, [0, 6, 0], [0, 7, 0], false),
                    (false, id, [0, 7, 0], [0, 8, 0], false),
                    (false, id, [0, 8, 0], [0, 9, 0], false),
                ]
            );
        })
    });
}

/// A goal that, when it starts, runs `LivingEntity.refreshDimensions`' nudge for a mob that grew
/// from a 0.1 by 0.1 box (`fudgePositionAfterSizeChange`, a `setPos`), then sets the mob moving
/// up at 0.6, once.
#[cfg(feature = "client")]
#[derive(Clone, Debug)]
struct Grows(bool);

#[cfg(feature = "client")]
impl sunrise_sim::mobs::goals::Goal for Grows {
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(
        &mut self,
        _: &mut sunrise_sim::mobs::Mob,
        _: &mut sunrise_sim::mobs::goals::Cx<'_, '_, '_>,
    ) -> bool {
        !self.0
    }
    fn start(
        &mut self,
        m: &mut sunrise_sim::mobs::Mob,
        cx: &mut sunrise_sim::mobs::goals::Cx<'_, '_, '_>,
    ) {
        self.0 = true;
        m.fudge_position_after_size_change(cx.w, 0.1, 0.1);
        m.velocity = sunrise_core::geometry::Vec3::new(0.0, 0.6, 0.0);
    }
    fn box_clone(&self) -> Box<dyn sunrise_sim::mobs::goals::Goal> {
        Box::new(self.clone())
    }
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_size_change_nudge_in_a_mobs_tick_then_its_walk_are_two_crossings() {
    observe(|calls| {
        with_game(|game| {
            // y 7, 0.3 over its floor. Growing to an adult cow's 1.4 from 0.1 keeps its centre, so
            // the nudge sets it 0.65 lower, into y 6; the same tick walks it 0.6 up, back into y 7.
            // Told only where the tick ended, it would be no crossing at all.
            let id = floating_cow(game, [8.5, 112.3, 8.5]);
            game.tick_creature_entities().unwrap();
            let i = game.mobs.iter().position(|m| m.id == id).unwrap();
            game.mobs.update(i, |m| m.ai.goals.add(0, Grows(false)));
            for _ in 0..4 {
                game.tick_creature_entities().unwrap();
                if !calls.lock().unwrap().is_empty() {
                    break;
                }
            }
            let at = game.mobs.iter().find(|m| m.id == id).unwrap().position;
            assert!(
                (at.y - 112.25).abs() < 1e-3,
                "nudged to 111.65, walked to 112.25: {at:?}"
            );
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[
                    (false, id, [0, 7, 0], [0, 6, 0], false),
                    (false, id, [0, 6, 0], [0, 7, 0], false),
                ]
            );
        })
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_mods_random_teleport_of_a_mob_is_told_at_once() {
    observe(|calls| {
        with_game(|game| {
            crate::game::server_side::native_living_position::tests::floor_and_zombies(game);
            // The zombie at (0.5, 64, 0.5) is filed where it stands, then sent to land on the
            // floor at (-3.5, 64, -3.5): one section west and one north.
            game.teleport_to(Target::Mob(12), [0.5, 64.0, 0.5], None, true);
            assert!(calls.lock().unwrap().is_empty());
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            assert!(game.living_random_teleport(zombie, [-3.5, 66.0, -3.5], None));
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[(false, 12, [0, 4, 0], [-1, 4, -1], true)]
            );
        })
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_server_entity_that_leaves_is_forgotten() {
    observe(|_| {
        with_game(|game| {
            let id = floating_cow(game, [8.5, 100.0, 8.5]);
            assert!(
                game.native_lifecycle
                    .entity_sections
                    .filed
                    .contains_key(&id)
            );
            // Despawned or unloaded: no longer in any list, so it leaves the level.
            game.mobs.clear();
            game.native_leaves_flush();
            assert!(
                !game
                    .native_lifecycle
                    .entity_sections
                    .filed
                    .contains_key(&id)
            );
        })
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_server_mobs_own_move_is_not_reconstructed_after_its_post() {
    use sunrise_core::geometry::Vec3;
    observe(|calls| {
        with_game(|game| {
            let id = game
                .spawn_mob_spawned(
                    "cow",
                    [1.0, 70.0, 3.0],
                    sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                )
                .unwrap();
            let kind = game.mobs.iter().find(|m| m.id == id).unwrap().kind.clone();
            // The mob's round: Pre, the tick (its own move over z = 16), then the Post.
            let mut round = game.native_tick_round();
            assert!(game.native_tick_pre_one(&mut round, id, &kind.id));
            let i = game.mobs.iter().position(|m| m.id == id).unwrap();
            game.mobs
                .update(i, |m| m.position = Vec3::new(1.0, 70.0, 17.0));
            assert!(calls.lock().unwrap().is_empty());
            game.native_tick_post_one(&round, id, &kind.id);
            // A raw field edit is not a position-write boundary. The Post hook must not
            // synthesize a crossing after the fact; real ticks record their writes before Post.
            assert!(calls.lock().unwrap().is_empty());
        })
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_client_is_told_each_teleport_and_where_its_entities_really_are() {
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::packets::play::{self, Clientbound as C};
    observe(|calls| {
        with_client_game(|game| {
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "Steve").unwrap();
            game.handle_packet(C::Login(play::Login {
                player_id: 7,
                hardcore: false,
                levels: vec!["minecraft:overworld".into()],
                max_players: 20,
                chunk_radius: 8,
                simulation_distance: 8,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn: play::SpawnInfo {
                    dimension_type: 0,
                    dimension: "minecraft:overworld".into(),
                    seed: 1,
                    game_type: 0,
                    previous_game_type: None,
                    debug: false,
                    flat: false,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                },
                online_mode: false,
                enforces_secure_chat: false,
            }))
            .unwrap();
            let teleport = |id: i32, at: [f64; 3]| {
                C::PlayerPosition(play::PlayerPosition {
                    teleport: id,
                    position: at,
                    delta: [0.0; 3],
                    yaw: 0.0,
                    pitch: 0.0,
                    relatives: 0,
                })
            };
            // The player is placed, then sent A -> B -> A before the client ticks.
            game.handle_packet(teleport(1, [8.0, 70.0, 8.0])).unwrap();
            game.handle_packet(teleport(2, [8.0, 70.0, 24.0])).unwrap();
            game.handle_packet(teleport(3, [8.0, 70.0, 8.0])).unwrap();
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[
                    (true, 7, [0, 4, 0], [0, 4, 1], true),
                    (true, 7, [0, 4, 1], [0, 4, 0], true),
                ]
            );
            calls.lock().unwrap().clear();
            let pig = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:entity_type")
                .unwrap()
                .id("minecraft:pig")
                .unwrap();
            game.handle_packet(C::AddEntity(play::AddEntity {
                id: 20,
                uuid: 20,
                kind: pig,
                position: [1.0, 70.0, 3.0],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            }))
            .unwrap();
            // Its own move (the interpolation the client steps), not the packet's position.
            let i = game.mobs.iter().position(|m| m.id == 20).unwrap();
            game.mobs
                .update(i, |m| m.position = Vec3::new(1.0, 70.0, -2.0));
            assert!(calls.lock().unwrap().is_empty());
            game.client_entity_sections_stepped();
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[(true, 20, [0, 4, 0], [0, 4, -1], true)]
            );
        })
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_refused_player_add_does_not_hide_the_resident_entities_crossing() {
    use crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::logged_in;
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::packets::play::{self, Clientbound as C};

    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::entity_sections::tests::a_refused_player_add_does_not_hide_the_resident_entities_crossing",
        || {
            observe(|calls| {
                with_client_game(|game| {
                    let _reader = logged_in(game);
                    let add = |game: &sunrise_client_gameplay::game::ClientGame<'_>, uuid, kind| {
                        C::AddEntity(play::AddEntity {
                            id: 20,
                            uuid,
                            kind: game
                                .client
                                .remote
                                .as_ref()
                                .unwrap()
                                .builtin
                                .get("minecraft:entity_type")
                                .unwrap()
                                .id(kind)
                                .unwrap(),
                            position: [1.0, 70.0, 3.0],
                            velocity: [0.0; 3],
                            pitch: 0,
                            yaw: 0,
                            head_yaw: 0,
                            data: 0,
                        })
                    };
                    game.handle_packet(add(game, 20, "minecraft:pig")).unwrap();
                    let i = game.mobs.iter().position(|mob| mob.id == 20).unwrap();
                    game.mobs
                        .update(i, |mob| mob.position = Vec3::new(17.0, 70.0, 3.0));
                    // The unfiled move is still pending when an unknown player tries the same id.
                    calls.lock().unwrap().clear();
                    game.handle_packet(add(game, 21, "minecraft:player"))
                        .unwrap();
                    assert!(calls.lock().unwrap().is_empty());
                    game.client_entity_sections_stepped();
                    assert_eq!(
                        calls.lock().unwrap().as_slice(),
                        &[(true, 20, [0, 4, 0], [1, 4, 0], true)]
                    );
                });
            })
        },
    );
}

/// A mod's `teleport_to_level` into another level is a change of level, not a crossing: the
/// recreated entity is filed in the new level untold (its join), this call tells nothing of its
/// own, and its next move there over a section edge is told once.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn an_entity_moved_into_another_level_is_told_once_at_its_next_crossing() {
    observe(|calls| {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            game.native_lifecycle.world_open_ready().unwrap();
            game.summon_with_nbt(
                "minecraft:item",
                [8.5, 80.0, 8.5],
                r#"{Item:{id:"minecraft:stick",count:1},NoGravity:1b}"#,
            )
            .unwrap();
            game.lan_assign_ids();
            let item = game.entity(game.dropped[0].id).unwrap();
            assert!(game.entity_teleport_to_level(
                item,
                "minecraft:the_nether",
                [8.5, 80.0, 8.5],
                None
            ));
            assert!(
                calls.lock().unwrap().is_empty(),
                "the change of level is not told"
            );
            game.enter_level(Nether).unwrap();
            let moved = game.dropped[0].id;
            game.teleport_to(Target::Item(moved), [24.5, 80.0, 8.5], None, true);
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[(false, moved, [0, 5, 0], [1, 5, 0], true)]
            );
        })
    });
}

/// #3944: the entity-type default read is metadata every host answers from the published
/// suppliers. The section host, which has no game behind it, answers (None for an unknown type)
/// and never unwinds, whether or not a catalog has been published.
#[test]
fn section_host_reads_entity_type_defaults_without_unwinding() {
    let host = LiveSectionHost::new(None);
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        (
            mod_api::prelude::ItemsHost::entity_type_default_attribute(
                &host,
                "minecraft:zombie",
                "minecraft:max_health",
            ),
            mod_api::prelude::ItemsHost::entity_type_default_attribute(
                &host,
                "ruby:no_such_type",
                "minecraft:max_health",
            ),
        )
    }));
    assert!(
        matches!(read, Ok((None | Some(_), None))),
        "the section host unwound: {read:?}"
    );
}

/// #3944 on a running game: the section host, game host and reload context answer from the
/// same published suppliers. Vanilla's Zombie has 20 health, 3 attack damage and 2 armor.
/// A mod-owned type with a 5000 health base answers 1024, while its spawn base stays 5000.
/// An attribute a mod adds to a type (a Skeleton's luck) answers too. No read answers for
/// `minecraft:item` (no supplier) or a type no one registered.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn section_host_and_game_read_the_same_entity_type_defaults() {
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::entity_sections::tests::section_host_and_game_read_the_same_entity_type_defaults",
        "SUNRISE_ENTITY_TYPE_DEFAULTS_CHILD",
        &["host"],
    ) {
        return;
    }
    with_game(|game| {
        let host = LiveSectionHost::new(None);
        let read = |game: &sunrise_gameplay::game::ServerGame<'_>,
                    entity_type: &str,
                    attribute: &str| {
            let scoped = mod_api::prelude::ItemsHost::entity_type_default_attribute(
                &host,
                entity_type,
                attribute,
            );
            let live = mod_api::prelude::ItemsHost::entity_type_default_attribute(
                game,
                entity_type,
                attribute,
            );
            let reload =
                mod_api::ReloadContext::entity_type_default_attribute(game, entity_type, attribute);
            assert_eq!(live, scoped, "{entity_type} {attribute}");
            assert_eq!(reload, scoped, "{entity_type} {attribute}");
            scoped
        };
        for (entity_type, attribute, expected) in [
            ("minecraft:zombie", "minecraft:max_health", Some(20.0)),
            ("zombie", "attack_damage", Some(3.0)),
            (":zombie", ":armor", Some(2.0)),
            ("minecraft:item", "minecraft:max_health", None),
            ("ruby:no_such_type", "minecraft:max_health", None),
            ("minecraft:zombie", "minecraft:no_such", None),
        ] {
            assert_eq!(
                read(&*game, entity_type, attribute),
                expected,
                "{entity_type} {attribute}"
            );
        }
        // A new installation resolves its own keys and sees no old suppliers before publication.
        sunrise_gameplay_common::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_entity_types(
            [
                "minecraft:zombie",
                "minecraft:skeleton",
                "minecraft:item",
                "ruby:titan",
            ],
            |_| None,
        )
        .unwrap();
        assert_eq!(read(&*game, "minecraft:zombie", "max_health"), None);
        let modded = std::sync::Arc::new(
            game.motion
                .attributes
                .with_entity_row(
                    "ruby:titan",
                    "minecraft:zombie",
                    &[("minecraft:max_health".into(), 5000.0)],
                )
                .unwrap()
                .with_entity_additions(
                    "minecraft:skeleton",
                    &[("minecraft:luck".into(), Some(1.5))],
                )
                .unwrap(),
        );
        sunrise_gameplay_common::game::resource_load::publish_entity_type_defaults(&modded);
        assert_eq!(read(&*game, "ruby:titan", "max_health"), Some(1024.0));
        let health = modded.id("minecraft:max_health").unwrap();
        assert_eq!(
            modded
                .create("ruby:titan")
                .unwrap()
                .base_value(health)
                .unwrap(),
            5000.0,
            "the spawn base stays the declared one"
        );
        assert_eq!(read(&*game, "minecraft:skeleton", "luck"), Some(1.5));
        assert_eq!(read(&*game, "minecraft:zombie", "max_health"), Some(20.0));
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn ticked_live_preserves_order_removals_and_reindexed_rows() {
    use std::sync::atomic::AtomicU32;
    use sunrise_core::geometry::Vec3;

    type Seen = Arc<Mutex<Vec<(u32, [i32; 3], [i32; 3], Option<[f64; 3]>)>>>;

    struct MutatingListener {
        mob_a: Arc<AtomicU32>,
        mob_b: Arc<AtomicU32>,
        seen: Seen,
    }

    #[mod_api::__subscriptions]
    impl Hooks for MutatingListener {
        fn entity_entering_section(
            &self,
            host: HostSide<'_>,
            event: &mut EntityEnteringSectionEvent,
        ) {
            let HostSide::Server(host) = host else {
                return;
            };
            let id = event.entity.id();
            let pos = event.entity.position(host);
            self.seen
                .lock()
                .unwrap()
                .push((id, event.old_section, event.new_section, pos));
            if id == self.mob_a.load(Ordering::Relaxed) && event.new_section == [1, 4, 0] {
                let b = self.mob_b.load(Ordering::Relaxed);
                assert!(host.entity(b).unwrap().discard(host));
                // This fixture has no console host; spawn directly in the level.
                let level = host.level("minecraft:overworld").unwrap();
                assert!(
                    level
                        .spawn_mob(
                            host,
                            "minecraft:cow",
                            [8.5, 70.0, 8.5],
                            mod_api::SpawnReason::Command
                        )
                        .is_some()
                );
            }
        }
    }

    let mob_a_id = Arc::new(AtomicU32::new(0));
    let mob_b_id = Arc::new(AtomicU32::new(0));
    let seen = Seen::default();
    let mods = vec![mod_api::ModEntry::new(
        "ticked-live-order",
        Box::new(MutatingListener {
            mob_a: mob_a_id.clone(),
            mob_b: mob_b_id.clone(),
            seen: seen.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let start = Vec3::new(8.5, 70.0, 8.5);
            let mid = Vec3::new(24.5, 70.0, 8.5);
            let end = Vec3::new(40.5, 70.0, 8.5);
            fixture_resident_column(&mut game.world, (1, 0));
            fixture_resident_column(&mut game.world, (2, 0));
            let mob_a = floating_cow(game, [start.x, start.y, start.z]);
            let mob_b = floating_cow(game, [start.x, start.y, start.z]);
            let mob_c = floating_cow(game, [start.x, start.y, start.z]);
            let mob_d = floating_cow(game, [start.x, start.y, start.z]);
            mob_a_id.store(mob_a, Ordering::Relaxed);
            mob_b_id.store(mob_b, Ordering::Relaxed);

            let join_of = |game: &sunrise_gameplay::game::ServerGame<'_>, id: u32| {
                game.mobs.iter().find(|m| m.id == id).unwrap().join
            };
            let join_a = join_of(game, mob_a);
            let join_b = join_of(game, mob_b);
            let join_c = join_of(game, mob_c);
            let join_d = join_of(game, mob_d);

            for (id, final_pos) in [(mob_a, end), (mob_b, mid), (mob_c, mid), (mob_d, mid)] {
                let i = game.mobs.iter().position(|m| m.id == id).unwrap();
                game.mobs.update(i, |m| m.position = final_pos);
            }
            // Reorder the list out of join-seq order and pass a stale hint so resolution must
            // recover without rebuilding a map.
            game.mobs.swap(0, 3);
            game.mobs.swap(1, 2);

            let mut delivered = PositionWrite::captured(mob_d, join_d, start, mid);
            delivered.delivered = true;
            let mut moves = vec![
                PositionWrite::captured(mob_a, join_a, start, mid),
                PositionWrite::captured(mob_b, join_b, start, mid),
                PositionWrite::captured(mob_a, join_a, mid, end),
                PositionWrite::captured(mob_c, join_c, start, mid),
                delivered,
            ];
            assert!(game.entity_sections_ticked_live(&mut moves, Some(999)));

            assert!(moves.is_empty());
            assert!(
                game.mobs
                    .iter()
                    .any(|m| ![mob_a, mob_b, mob_c, mob_d].contains(&m.id)),
                "the listener must add a row during dispatch"
            );
            assert_eq!(
                seen.lock().unwrap().as_slice(),
                &[
                    (mob_a, [0, 4, 0], [1, 4, 0], Some([mid.x, mid.y, mid.z])),
                    (mob_a, [1, 4, 0], [2, 4, 0], Some([end.x, end.y, end.z])),
                    (mob_c, [0, 4, 0], [1, 4, 0], Some([mid.x, mid.y, mid.z])),
                ]
            );
            assert_eq!(
                game.mobs.iter().find(|m| m.id == mob_a).unwrap().position,
                end
            );
            assert!(
                !game
                    .native_lifecycle
                    .entity_sections
                    .filed
                    .contains_key(&mob_b),
                "discarded mob must stay forgotten"
            );
        })
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn borrowed_tick_tells_nested_passenger_and_vehicle_writes_before_resuming() {
    use std::sync::atomic::AtomicU32;

    type Seen = Arc<Mutex<Vec<(u32, [i32; 3], [i32; 3], Option<[f64; 3]>)>>>;

    struct TeleportVehicleListener {
        first: Arc<AtomicU32>,
        vehicle: Arc<AtomicU32>,
        teleported: Arc<AtomicBool>,
        seen: Seen,
    }

    #[mod_api::__subscriptions]
    impl Hooks for TeleportVehicleListener {
        fn entity_entering_section(
            &self,
            host: HostSide<'_>,
            event: &mut EntityEnteringSectionEvent,
        ) {
            let HostSide::Server(host) = host else { return };
            self.seen.lock().unwrap().push((
                event.entity.id(),
                event.old_section,
                event.new_section,
                event.entity.position(host),
            ));
            if event.entity.id() == self.first.load(Ordering::Relaxed) {
                let vehicle = host.entity(self.vehicle.load(Ordering::Relaxed)).unwrap();
                self.teleported.store(
                    vehicle.teleport_to(host, [56.5, 100.0, 8.5], None),
                    Ordering::Relaxed,
                );
            }
        }
    }

    let first_id = Arc::new(AtomicU32::new(0));
    let vehicle_id = Arc::new(AtomicU32::new(0));
    let teleported = Arc::new(AtomicBool::new(false));
    let seen = Seen::default();
    let mods = vec![mod_api::ModEntry::new(
        "borrowed-tick-vehicle-tp",
        Box::new(TeleportVehicleListener {
            first: first_id.clone(),
            vehicle: vehicle_id.clone(),
            teleported: teleported.clone(),
            seen: seen.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            // The first cow yields to the owning Host. The next uses the live
            // borrowed observer, which must not inherit that Host's crossings.
            let first = rising_cow(game, [4.5, 111.75, 8.5], true);
            let next = rising_cow(game, [12.5, 127.75, 8.5], false);
            let vehicle = floating_cow(game, [8.5, 100.0, 8.5]);
            let passenger = floating_cow(game, [8.5, 100.0, 8.5]);
            first_id.store(first, Ordering::Relaxed);
            vehicle_id.store(vehicle, Ordering::Relaxed);
            assert!(game.try_mount_mob(passenger, vehicle));
            let [passenger_x, passenger_y, passenger_z] =
                game.mob_object(passenger).unwrap().position.components();

            game.tick_creature_entities().unwrap();
            assert!(teleported.load(Ordering::Relaxed));
            assert_eq!(
                seen.lock().unwrap().as_slice(),
                &[
                    (first, [0, 6, 0], [0, 7, 0], Some([4.5, 112.25, 8.5])),
                    (
                        passenger,
                        [0, 6, 0],
                        [3, 6, 0],
                        Some([passenger_x + 48.0, passenger_y, passenger_z])
                    ),
                    (vehicle, [0, 6, 0], [3, 6, 0], Some([56.5, 100.0, 8.5])),
                    (next, [0, 7, 0], [0, 8, 0], Some([12.5, 128.25, 8.5])),
                ]
            );
        })
    });
}

#[test]
fn live_section_host_builds_play_packets_and_refuses_container_click() {
    use mod_api::{
        ContainerClickRequest, PacketBuildError, PlayPacketRequest, prelude::NetworkClientHost,
    };
    use sunrise_protocol::{
        ids::play::serverbound as sb,
        packets::play::{Attack, Serverbound},
    };

    let host = LiveSectionHost::new(None);
    let packet = host
        .build_play_packet(&PlayPacketRequest::Attack { entity: 9 })
        .unwrap();
    assert_eq!(packet.id.to_string(), "minecraft:attack");
    assert_eq!(
        Serverbound::decode(sb::ATTACK, &packet.body).unwrap(),
        Serverbound::Attack(Attack { id: 9 })
    );
    assert_eq!(
        host.container_click(&ContainerClickRequest::default()),
        Err(PacketBuildError::Unavailable {
            packet: "container_click".into(),
            field: "host".into(),
        })
    );
}

#[test]
fn each_filed_crossing_keeps_the_placement_of_its_own_move() {
    let mut s = EntitySections::default();
    let w = scope(Overworld);
    s.placed(w, 8, [0.5, 70.0, 0.5]);
    // Two nested moves of one entity, filed before either is told.
    s.moved(w, 8, None, [16.5, 70.0, 0.5]);
    s.moved(w, 8, None, [32.5, 70.0, 0.5]);
    assert!(s.begin_telling());
    let first = s.queue.pop_front().unwrap();
    let second = s.queue.pop_front().unwrap();
    assert_eq!(
        (first.at, second.at),
        ([16.5, 70.0, 0.5], [32.5, 70.0, 0.5])
    );
}

#[test]
fn a_move_followed_by_a_discard_still_tells_its_crossing() {
    let mut s = EntitySections::default();
    let w = scope(Overworld);
    s.placed(w, 8, [0.5, 70.0, 0.5]);
    s.moved(w, 8, None, [16.5, 70.0, 0.5]);
    s.forget(8);
    assert_eq!(told(&mut s), [crossing(8, [0, 4, 0], [1, 4, 0])]);
}

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;
