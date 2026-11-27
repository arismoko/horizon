//! Native debug renderer selection, refresh, and world geometry, with prepared client resources.
use super::{
    debug_options::{Row, rows},
    native_entity_refs::tests::with_client_game,
};
use glam::Mat4;
use mod_api::{
    __private::game::RegistrationContextNew, DebugEntry, DebugRenderer, Hooks, ModEntry,
    RegisterDebugEntriesEvent, RegisterDebugRenderersEvent, RegistrationContext, WorldDepth,
    WorldLine, content::crash::current_native_mod,
};
use std::sync::{Arc, Mutex, Weak};
use sunrise_client::{
    debug_entries::{self, Category, DebugEntries, Profile, Status},
    world_geometry::WorldGeometry,
};
use sunrise_protocol::packets::play::{self, Clientbound as C};

const CAMERA: [f64; 3] = [100.0, 64.0, 100.0];
// Deliberately not the namespace of the switches.
const OWNER: &str = "debug_test";
const SHOWN: &str = "marker:pipes";
const OTHER: &str = "marker:wires";
const UNDECLARED: &str = "marker:ghost";
const REDUCED_ALLOWED: &str = "minecraft:custom_marker";
const ALWAYS: &str = "always";

#[derive(Default)]
struct Fixture {
    asked: Vec<&'static str>,
    pushed: Vec<&'static str>,
    refresh: usize,
    checked_refresh: usize,
}

/// The dispatcher fixture retains its registered mods; don't let it retain this test's state.
struct Marker(Weak<Mutex<Fixture>>);

#[mod_api::__subscriptions]
impl Hooks for Marker {
    fn register_debug_entries(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterDebugEntriesEvent<'_>,
    ) {
        for id in [SHOWN, OTHER] {
            event.entries.push(DebugEntry::renderer(id.into()));
        }
        event
            .entries
            .push(DebugEntry::renderer(REDUCED_ALLOWED.into()).with_reduced_debug_info(true));
    }

    fn register_debug_renderers(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterDebugRenderersEvent<'_>,
    ) {
        let owner = current_native_mod();
        assert_eq!(owner, Some(OWNER));
        let fixture = self.0.upgrade().expect("active debug renderer fixture");
        let mut state = fixture.lock().unwrap();
        state.refresh += 1;
        for (slot, &id) in state.pushed.iter().enumerate() {
            if id != ALWAYS && !event.is_debug_entry_enabled(id) {
                continue;
            }
            // Height identifies the refresh; depth distinguishes registrations for the same ID.
            let y = 64.0 + state.refresh as f64;
            let z = 99.0 - slot as f64;
            let fixture = Arc::clone(&fixture);
            event.renderers.push(DebugRenderer::new(move |_, _, draw| {
                assert_eq!(current_native_mod(), owner);
                fixture.lock().unwrap().asked.push(id);
                let marker = WorldLine::new(
                    [100.25, y, z],
                    [101.0, y, z],
                    0xFF00FF00,
                    2.0,
                    WorldDepth::Tested,
                );
                draw.lines(&[marker]).unwrap();
            }));
        }
    }
}

/// Dispatch the real registration hooks with only this test's listener, and keep the validated
/// startup declarations on the test game instead of publishing them to the client registry.
fn with_debug_renderers(test: impl FnOnce(&mut crate::game::ClientGame<'_>, &Mutex<Fixture>)) {
    let fixture = Arc::new(Mutex::new(Fixture::default()));
    let mods = vec![ModEntry::new(
        OWNER,
        Box::new(Marker(Arc::downgrade(&fixture))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let mut entries = Vec::new();
            {
                sunrise_gameplay_common::game::native_hooks::native_register_debug_entries(
                    &RegistrationContext::new(),
                    &mut entries,
                );
            }
            game.client.registered_debug_entries.0 = debug_entries::collect_registrations(entries);
            // Status changes stay out of debug-profile.json.
            game.client.debug_entries.0 = DebugEntries::default();
            test(game, &fixture);
        });
    });
}

fn test_camera(position: [f64; 3]) -> mod_api::Camera {
    mod_api::Camera {
        position,
        eye_height: 1.62,
        partial_tick: 0.0,
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
        fov_degrees: 70.0,
        near: 0.05,
        far: 128.0,
        view: Mat4::IDENTITY.to_cols_array(),
        projection: Mat4::IDENTITY.to_cols_array(),
        dimension: "minecraft:overworld".into(),
    }
}

/// One frame's camera-relative lines and the IDs of the callbacks that ran.
fn frame(
    game: &mut crate::game::ClientGame<'_>,
    fixture: &Mutex<Fixture>,
    title: bool,
    draw: &mut WorldGeometry,
) -> (Vec<[f32; 6]>, Vec<&'static str>) {
    let frame_camera = (!title).then(|| test_camera(CAMERA));
    game.fill_world_geometry(
        title,
        frame_camera.as_ref(),
        CAMERA,
        Mat4::IDENTITY,
        0.0,
        false,
        &[],
        draw,
        &mut Vec::new(),
    );
    let lines = draw
        .batch()
        .lines
        .iter()
        .map(|line| std::array::from_fn(|i| line[i]));
    (
        lines.collect(),
        std::mem::take(&mut fixture.lock().unwrap().asked),
    )
}

/// How many times the renderer list refreshed since the last observation.
fn refreshes(fixture: &Mutex<Fixture>) -> usize {
    let mut state = fixture.lock().unwrap();
    let count = state.refresh - state.checked_refresh;
    state.checked_refresh = state.refresh;
    count
}

fn push(fixture: &Mutex<Fixture>, ids: &[&'static str]) {
    let mut state = fixture.lock().unwrap();
    state.pushed.clear();
    state.pushed.extend_from_slice(ids);
}

#[test]
#[ignore = "daily"]
fn the_renderer_list_refreshes_with_the_selection_and_leaves_nothing_behind() {
    with_debug_renderers(|game, fixture| {
        push(fixture, &[SHOWN, OTHER, UNDECLARED]);
        let toggles: Vec<_> = rows(&game.client.registered_debug_entries.0, "marker:")
            .into_iter()
            .filter_map(|row| match row {
                Row::Option(entry) if entry.category == Category::Renderer => Some(entry.path),
                _ => None,
            })
            .collect();
        assert_eq!(toggles, [SHOWN, OTHER]);
        assert_eq!(refreshes(fixture), 0);
        game.client
            .debug_entries
            .0
            .set_status(UNDECLARED, Status::AlwaysOn);
        game.client.title.0 = None;
        let mut draw = WorldGeometry::default();
        let none = (Vec::new(), Vec::new());

        // Off by default. The first view refreshes, as `lastDebugEntriesVersion` starts behind;
        // the next one does not.
        assert_eq!(frame(game, fixture, false, &mut draw), none);
        assert_eq!(refreshes(fixture), 1);
        assert_eq!(frame(game, fixture, false, &mut draw), none);
        assert_eq!(refreshes(fixture), 0);

        // Selected: one refresh, and the enabled renderer runs.
        game.client
            .debug_entries
            .0
            .set_status(SHOWN, Status::AlwaysOn);
        let shown = |y| (vec![[0.25, y, -1.0, 1.0, y, -1.0]], vec![SHOWN]);
        assert_eq!(frame(game, fixture, false, &mut draw), shown(2.0));
        assert_eq!(refreshes(fixture), 1);
        // The next frame clears its batch and calls the same installed object.
        assert_eq!(frame(game, fixture, false, &mut draw), shown(2.0));
        assert_eq!(refreshes(fixture), 0);

        // The overlay changes the selection both ways.
        game.client
            .debug_entries
            .0
            .set_status(SHOWN, Status::InOverlay);
        assert_eq!(frame(game, fixture, false, &mut draw), none);
        assert_eq!(refreshes(fixture), 1);
        toggle_overlay(game);
        assert_eq!(frame(game, fixture, false, &mut draw), shown(4.0));
        assert_eq!(refreshes(fixture), 1);
        toggle_overlay(game);
        assert_eq!(frame(game, fixture, false, &mut draw), none);
        assert_eq!(refreshes(fixture), 1);

        // Reduced debug info hides default entries, while an entry opted in with
        // `with_reduced_debug_info(true)` stays enabled.
        game.client
            .debug_entries
            .0
            .set_status(SHOWN, Status::AlwaysOn);
        assert_eq!(frame(game, fixture, false, &mut draw), shown(6.0));
        assert_eq!(refreshes(fixture), 1);
        push(fixture, &[SHOWN, REDUCED_ALLOWED]);
        game.client
            .debug_entries
            .0
            .set_status(REDUCED_ALLOWED, Status::AlwaysOn);
        game.options_control(
            &["options-set", "reducedDebugInfo", "true"],
            "options-set reducedDebugInfo true",
        )
        .unwrap();
        assert_eq!(
            frame(game, fixture, false, &mut draw),
            (
                vec![[0.25, 7.0, -2.0, 1.0, 7.0, -2.0]],
                vec![REDUCED_ALLOWED]
            )
        );
        assert_eq!(refreshes(fixture), 1);
        game.client
            .debug_entries
            .0
            .set_status(REDUCED_ALLOWED, Status::Never);
        push(fixture, &[SHOWN, OTHER, UNDECLARED]);
        game.options_control(
            &["options-set", "reducedDebugInfo", "false"],
            "options-set reducedDebugInfo false",
        )
        .unwrap();
        assert_eq!(frame(game, fixture, false, &mut draw), shown(8.0));
        assert_eq!(refreshes(fixture), 1);

        // A built-in profile leaves it out.
        game.client.debug_entries.0.load_profile(Profile::Default);
        assert_eq!(frame(game, fixture, false, &mut draw), none);
        assert_eq!(refreshes(fixture), 1);

        // The installed objects keep their captured geometry until refresh, even when the mod
        // would now push something else. Multiple renderers (including repeated switches and
        // unconditional ones) all run in registration order with no deduplication.
        push(fixture, &[SHOWN, SHOWN, OTHER, ALWAYS]);
        game.client
            .debug_entries
            .0
            .set_status(SHOWN, Status::AlwaysOn);
        game.client
            .debug_entries
            .0
            .set_status(OTHER, Status::AlwaysOn);
        let all_four = (
            vec![
                [0.25, 10.0, -1.0, 1.0, 10.0, -1.0],
                [0.25, 10.0, -2.0, 1.0, 10.0, -2.0],
                [0.25, 10.0, -3.0, 1.0, 10.0, -3.0],
                [0.25, 10.0, -4.0, 1.0, 10.0, -4.0],
            ],
            vec![SHOWN, SHOWN, OTHER, ALWAYS],
        );
        assert_eq!(frame(game, fixture, false, &mut draw), all_four);
        assert_eq!(refreshes(fixture), 1);
        push(fixture, &[OTHER]);
        assert_eq!(frame(game, fixture, false, &mut draw), all_four);
        assert_eq!(refreshes(fixture), 0);
        game.client
            .debug_entries
            .0
            .set_status(SHOWN, Status::AlwaysOn);
        assert_eq!(
            frame(game, fixture, false, &mut draw),
            (vec![[0.25, 11.0, -1.0, 1.0, 11.0, -1.0]], vec![OTHER])
        );
        assert_eq!(refreshes(fixture), 1);
        push(fixture, &[SHOWN]);

        // Disabled: the marker goes with the next frame.
        game.client.debug_entries.0.set_status(OTHER, Status::Never);
        game.client.debug_entries.0.set_status(SHOWN, Status::Never);
        assert_eq!(frame(game, fixture, false, &mut draw), none);
        assert_eq!(refreshes(fixture), 1);

        // Leaving the world with it on: the title frame asks nothing and shows nothing, and
        // refreshes nothing, as it has no `emitGizmos`.
        game.client
            .debug_entries
            .0
            .set_status(SHOWN, Status::AlwaysOn);
        assert_eq!(frame(game, fixture, false, &mut draw), shown(13.0));
        assert_eq!(refreshes(fixture), 1);
        game.leave_server(None).unwrap();
        game.client.debug_entries.0.set_status(SHOWN, Status::Never);
        assert_eq!(frame(game, fixture, true, &mut draw), none);
        assert_eq!(refreshes(fixture), 0);

        // `handleLogin` refreshes the list, with no change of selection.
        let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Steve").unwrap();
        game.handle_packet(login()).unwrap();
        assert_eq!(refreshes(fixture), 1);
        let frame_camera = test_camera(CAMERA);
        game.fill_world_geometry(
            false,
            Some(&frame_camera),
            CAMERA,
            Mat4::IDENTITY,
            0.0,
            false,
            &[],
            &mut draw,
            &mut Vec::new(),
        );
        assert!(
            fixture.lock().unwrap().asked.is_empty(),
            "the switch is still off"
        );
        assert_eq!(refreshes(fixture), 1, "first world view after login");
    });
}

fn login() -> C {
    C::Login(play::Login {
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
    })
}

fn toggle_overlay(game: &mut crate::game::ClientGame<'_>) {
    use sunrise_client::debug_keys::KeyAction;
    use winit::keyboard::KeyCode;
    game.debug_key(KeyCode::F3, KeyAction::Press);
    game.debug_key(KeyCode::F3, KeyAction::Release);
}

/// Prime a real world's view, change only the mod's renderer state, then invalidate through
/// the real input/packet path. The next view must collect the new list exactly once.
fn invalidation_refreshes(
    expected: &[&'static str],
    change: impl FnOnce(&mut crate::game::ClientGame<'_>, &Mutex<Fixture>),
) {
    with_debug_renderers(|game, fixture| {
        game.client.title.0 = None;
        let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Steve").unwrap();
        game.handle_packet(login()).unwrap();
        game.client
            .debug_entries
            .0
            .set_status(SHOWN, Status::AlwaysOn);
        game.client
            .debug_entries
            .0
            .set_status(OTHER, Status::AlwaysOn);
        let mut draw = WorldGeometry::default();
        push(fixture, &[SHOWN]);
        assert_eq!(frame(game, fixture, false, &mut draw).1, [SHOWN]);
        refreshes(fixture);
        push(fixture, &[OTHER]);
        change(game, fixture);
        assert_eq!(
            frame(game, fixture, false, &mut draw).1.as_slice(),
            expected,
            "next world view must replace the stale renderer list"
        );
        assert_eq!(refreshes(fixture), 1);
        assert_eq!(
            frame(game, fixture, false, &mut draw).1.as_slice(),
            expected
        );
        assert_eq!(refreshes(fixture), 0, "no work on an unchanged frame");
    });
}

#[test]
#[ignore = "daily"]
fn debug_renderer_respawn_invalidates_unchanged_selection() {
    invalidation_refreshes(&[OTHER], |game, _| {
        let C::Login(login) = login() else {
            unreachable!()
        };
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: login.spawn,
            keep: 3,
        }))
        .unwrap();
    });
}

fn same_value_event(event: i8, expected: &[&'static str]) {
    invalidation_refreshes(expected, |game, fixture| {
        push(fixture, &[SHOWN]);
        game.handle_packet(C::EntityEvent(play::EntityEvent { id: 7, event }))
            .unwrap();
        let mut draw = WorldGeometry::default();
        frame(game, fixture, false, &mut draw);
        assert_eq!(refreshes(fixture), 1);
        push(fixture, &[OTHER]);
        game.handle_packet(C::EntityEvent(play::EntityEvent { id: 7, event }))
            .unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn debug_renderer_same_value_reduced_info_on_invalidates() {
    same_value_event(22, &[]);
}

#[test]
#[ignore = "daily"]
fn debug_renderer_same_value_reduced_info_off_invalidates() {
    same_value_event(23, &[OTHER]);
}

#[test]
#[ignore = "daily"]
fn debug_renderer_overlay_round_trip_invalidates() {
    invalidation_refreshes(&[OTHER], |game, _| {
        let before = game.client.debug;
        toggle_overlay(game);
        assert_ne!(game.client.debug, before);
        toggle_overlay(game);
        assert_eq!(game.client.debug, before);
    });
}

#[test]
#[ignore = "daily"]
fn debug_renderer_second_login_invalidates_first_world_view() {
    invalidation_refreshes(&[OTHER], |game, fixture| {
        push(fixture, &[SHOWN]);
        game.handle_packet(login()).unwrap();
        assert_eq!(refreshes(fixture), 1, "immediate handleLogin refresh");
        // A logging-in listener can update its state after the immediate refresh.
        push(fixture, &[OTHER]);
    });
}
