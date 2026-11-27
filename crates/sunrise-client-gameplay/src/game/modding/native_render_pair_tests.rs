//! The render hook pair around a living entity (#3681): `render_living_pre` and
//! `render_living_post`, wrapped for a player by `render_player_pre` and `render_player_post`. A
//! canceled Pre skips the model, layers and the later name tag and leash passes, but not its
//! flames and shadow. Canceling only Living Pre still calls Player Post without a model. Culled
//! entities are not asked; the hooks run in draw queue order.
#[cfg(test)]
use mod_api::{
    BlockEntityDrawContext, Hooks, RenderLivingPostEvent, RenderLivingPreEvent,
    RenderPlayerPostEvent, RenderPlayerPreEvent,
};
#[cfg(test)]
use std::sync::{Arc, Mutex};
#[cfg(test)]
use sunrise_client::mesher::Vertex;
#[cfg(test)]
use sunrise_sim::mobs::Mob;

/// What the mod heard, in order, and which entities it cancels.
#[cfg(test)]
#[derive(Default)]
struct Heard {
    events: Vec<String>,
    /// Entity ids whose `render_living_pre` it cancels.
    cancel_living: Vec<u32>,
    /// Whether it cancels `render_player_pre` of this window's own player (`true`) or another.
    cancel_player: Vec<bool>,
}

/// Records the four callbacks and their model observations. Living Pre draws a marker 100 blocks
/// above the entity so the test can see which geometry survives cancellation.
#[cfg(test)]
struct Pair(Arc<Mutex<Heard>>);

#[cfg(test)]
fn marker(draw: &mut dyn BlockEntityDrawContext) {
    let corners = [
        [0.0, 101.0, 0.0],
        [0.0, 100.0, 0.0],
        [1.0, 100.0, 0.0],
        [1.0, 101.0, 0.0],
    ];
    draw.quad("block/stone", corners, [0.0, 0.0, 16.0, 16.0], 0xFF_FFFF);
}

#[cfg(test)]
#[mod_api::__subscriptions]
impl Hooks for Pair {
    fn render_player_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderPlayerPreEvent<'_>,
    ) -> mod_api::Flow {
        let mut heard = self.0.lock().unwrap();
        let head = event
            .model
            .part("head")
            .map(|part| (part.index, part.visible));
        let visible = head.map(|(_, visible)| visible);
        heard
            .events
            .push(format!("player_pre {} head {visible:?}", event.who.local));
        if event.who.local
            && let Some((index, _)) = head
        {
            event.model.hide(index);
        }
        event.cancelled = heard.cancel_player.contains(&event.who.local);
        mod_api::Flow::Continue
    }
    fn render_living_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderLivingPreEvent<'_>,
    ) -> mod_api::Flow {
        let mut heard = self.0.lock().unwrap();
        let id = event.entity.id;
        let observation = if id == 4321 {
            let visible = event.model.part("head").map(|part| part.visible);
            format!("living_pre {id} head {visible:?}")
        } else {
            format!("living_pre {id}")
        };
        heard.events.push(observation);
        marker(event.draw);
        event.cancelled = heard.cancel_living.contains(&event.entity.id);
        mod_api::Flow::Continue
    }
    fn render_living_post(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderLivingPostEvent<'_>,
    ) {
        let mut heard = self.0.lock().unwrap();
        heard
            .events
            .push(format!("living_post {}", event.entity.id));
    }
    fn render_player_post(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderPlayerPostEvent<'_>,
    ) {
        let mut heard = self.0.lock().unwrap();
        let visible = event
            .model
            .and_then(|model| model.part("head"))
            .map(|part| part.visible);
        heard.events.push(format!(
            "player_post {} model {} head {visible:?}",
            event.who.local,
            event.model.is_some()
        ));
    }
}

/// The camera, south of the entities and looking at them.
#[cfg(test)]
const CAMERA: glam::DVec3 = glam::DVec3::new(0.5, 64.0, -4.5);

/// A mob of `kind` with id `id`, standing `x` blocks east of the camera's line.
#[cfg(test)]
fn mob(game: &crate::game::ClientGame<'_>, kind: &str, id: u32, x: f64) -> Mob {
    let kind = if kind == "minecraft:player" {
        game.client_res().player_type().clone().unwrap()
    } else {
        game.common.res.mob_types().get(kind).unwrap().clone()
    };
    let at = sunrise_core::geometry::Vec3::new(0.5 + x, 64.0, 0.5);
    Mob::new(id, kind, at, 0.0)
}

/// How many of `out`'s vertices are 100 or more blocks up: the pre hooks' markers.
#[cfg(test)]
fn markers(out: &[Vertex]) -> usize {
    out.iter()
        .filter(|v| v.pos[1] - (64.0 - CAMERA.y) as f32 >= 99.0)
        .count()
}

/// One frame of the level's mobs, begun as the entity pass begins it.
#[cfg(test)]
fn mob_frame(game: &mut crate::game::ClientGame<'_>) -> Vec<Vertex> {
    game.prepare_entity_culling();
    let mut out = Vec::new();
    game.mob_geometry(CAMERA, 1.0, &mut out);
    out
}

/// Runs `check` on a client game with prepared resources and the [`Pair`] mod.
#[cfg(test)]
fn with_pair(heard: &Arc<Mutex<Heard>>, check: impl FnOnce(&mut crate::game::ClientGame<'_>)) {
    let mods = vec![mod_api::ModEntry::new(
        "pair",
        Box::new(Pair(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::native_entity_refs::tests::with_client_game(|game| {
            game.mobs.clear();
            check(game);
        });
    });
}

#[cfg(test)]
fn with_player_pair(
    heard: &Arc<Mutex<Heard>>,
    check: impl FnOnce(&mut crate::game::ClientGame<'_>),
) {
    with_pair(heard, |game| {
        game.init_remote_for_replay().unwrap();
        check(game);
    });
}

#[cfg(test)]
fn events(heard: &Arc<Mutex<Heard>>) -> Vec<String> {
    std::mem::take(&mut heard.lock().unwrap().events)
}

#[cfg(test)]
#[test]
#[ignore = "daily"]
fn two_entities_are_asked_in_the_draw_queues_order() {
    if super::native_entity_refs::tests::run_fresh_cases(
        "game::client_side::native_render_pair_tests::two_entities_are_asked_in_the_draw_queues_order",
        "SUNRISE_RENDER_PAIR_ORDER_CHILD",
        &["draw-order"],
    ) {
        return;
    }
    let heard = Arc::default();
    with_player_pair(&heard, |game| {
        game.client.pack_ids = Some(Default::default());
        game.common
            .mobs
            .push(mob(game, "minecraft:zombie", 91, 0.0));
        admit_own_player(game, false);
        game.common.mobs.push(mob(game, "minecraft:cow", 92, 2.0));
        let (out, ..) = game.entity_geometry(CAMERA, 1.0, None, Vec::new(), &mut Vec::new());
        assert_eq!(
            events(&heard),
            [
                "living_pre 91",
                "living_post 91",
                "player_pre true head Some(true)",
                "living_pre 4321 head Some(false)",
                "living_post 4321",
                "player_post true model true head Some(false)",
                "living_pre 92",
                "living_post 92"
            ]
        );
        assert_eq!(markers(&out), 12, "each pre hook's marker is kept");
        let motion = game.client.entity_motion.borrow();
        let tracked: Vec<_> = motion.iter().copied().filter(|&(_, id)| id != 0).collect();
        assert_eq!(
            tracked.iter().map(|&(_, id)| id).collect::<Vec<_>>(),
            [
                (1 << 32) | 91,
                sunrise_gameplay_common::game::MOTION_OWN_PLAYER,
                (1 << 32) | 92
            ]
        );
        assert!(tracked[0].0 < tracked[1].0 && tracked[1].0 < tracked[2].0);
        let spans = game.client.entity_spans.borrow();
        let local = spans.iter().find(|span| span.player).unwrap();
        assert_eq!(local.start, tracked[1].0);
        assert!(!local.hidden);
    });
}

#[cfg(test)]
#[test]
#[ignore = "daily"]
fn a_spectator_following_an_entity_submits_no_local_body() {
    use sunrise_command::args::GameMode;
    use sunrise_level::camera_math::CameraType;
    use sunrise_protocol::packets::play::{Clientbound, SetCamera};

    let heard = Arc::default();
    with_pair(&heard, |game| {
        let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Steve").unwrap();
        admit_own_player(game, true);
        game.mode = GameMode::Spectator;
        game.common
            .mobs
            .push(mob(game, "minecraft:zombie", 91, 0.0));
        game.handle_packet(Clientbound::SetCamera(SetCamera { camera_id: 91 }))
            .unwrap();
        assert!(!game.controls_camera());

        // First test the third-person body, then the separate first-person shadow route.
        // Native shadows can take a spectator model, so this exercises the camera guard.
        for (camera, player_shadow) in [
            (CameraType::ThirdPersonBack, false),
            (CameraType::FirstPerson, true),
        ] {
            game.client.camera_type = camera;
            game.client.occlusion.0.shadow_casters.1 = player_shadow;
            game.entity_geometry(CAMERA, 1.0, None, Vec::new(), &mut Vec::new());
            assert!(!game.client.entity_runs.iter().any(|run| run.family
                == crate::game::client_side::entity_client::DrawFamily::LocalPlayer));
            assert!(!events(&heard).iter().any(|event| matches!(
                event.as_str(),
                "player_pre true" | "player_post true" | "living_pre 4321" | "living_post 4321"
            )));
        }
    });
}

pub struct BodyWitness {
    pub id: u32,
    pub vertices: std::ops::Range<usize>,
}

thread_local! {
    // The new body families do not need motion-history buffers. Observe them only in tests,
    // after the mesh is emitted, keyed by the same vertex offsets as existing body marks.
    static BODY_WITNESSES: std::cell::RefCell<Option<Vec<BodyWitness>>> = const { std::cell::RefCell::new(None) };
}

pub(in crate::game) fn witness_body(id: u32, start: usize, end: usize) {
    if start < end {
        BODY_WITNESSES.with_borrow_mut(|witnesses| {
            if let Some(witnesses) = witnesses {
                witnesses.push(BodyWitness {
                    id,
                    vertices: start..end,
                });
            }
        });
    }
}

pub fn capture_body_witnesses<T>(draw: impl FnOnce() -> T) -> (T, Vec<BodyWitness>) {
    BODY_WITNESSES.with_borrow_mut(|witnesses| *witnesses = Some(Vec::new()));
    let output = draw();
    let witnesses = BODY_WITNESSES.with_borrow_mut(|witnesses| witnesses.take().unwrap());
    (output, witnesses)
}

pub fn submitted_bodies<T>(
    game: &mut crate::game::ClientGame<'_>,
    draw: impl FnOnce(&mut crate::game::ClientGame<'_>) -> T,
) -> (T, Vec<u32>) {
    let (output, witnesses) = capture_body_witnesses(|| draw(game));
    let mut bodies: Vec<_> = witnesses
        .into_iter()
        .map(|w| (w.vertices.start as u32, w.id))
        .collect();
    bodies.extend(
        game.client
            .entity_motion
            .borrow()
            .windows(2)
            .filter_map(|marks| {
                let (start, id) = marks[0];
                (id != 0 && start < marks[1].0).then(|| {
                    let id = if id == sunrise_gameplay_common::game::MOTION_OWN_PLAYER {
                        game.own_entity_id() as u32
                    } else {
                        id as u32
                    };
                    (start, id)
                })
            }),
    );
    bodies.sort_unstable_by_key(|&(start, _)| start);
    (output, bodies.into_iter().map(|(_, id)| id).collect())
}

/// Exercise the real packet and draw paths; the parity harness reads the same scene
/// and compares these submitted body IDs with the vanilla renderer's dispatch order.
#[cfg(test)]
#[test]
#[ignore = "daily"]
fn entity_draw_order_after_client_packets() {
    run_entity_draw_order_after_client_packets();
}

#[cfg(test)]
pub(in crate::game::client_side) fn run_entity_draw_order_after_client_packets() {
    use sunrise_level::camera_math::CameraType;
    use sunrise_protocol::{
        entity_data::{
            ITEM_ENTITY_ITEM, Value,
            serializer::{ITEM_STACK, OPTIONAL_BLOCK_POS},
        },
        item::Stack,
        packets::play::{self, Clientbound as C},
    };

    super::native_entity_refs::tests::with_client_game(|game| {
        let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Steve").unwrap();
        let builtin = game.client.remote.as_ref().unwrap().builtin.clone();
        let types = builtin.get("minecraft:entity_type").unwrap();
        let stone = builtin
            .get("minecraft:item")
            .unwrap()
            .id("minecraft:stone")
            .unwrap();
        let spawn = play::SpawnInfo {
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
        };
        game.client.pack_ids = Some(super::shader_packs::PackIds::with_entity_ids([
            ("minecraft:ender_dragon", 101),
            ("minecraft:end_crystal", 102),
        ]));
        let mut snapshots = Vec::new();
        let mut check_beams = false;
        for line in include_str!("entity-draw-order.txt").lines() {
            let words: Vec<_> = line.split_whitespace().collect();
            let packet = match words.as_slice() {
                ["login", id] => C::Login(play::Login {
                    player_id: id.parse().unwrap(),
                    hardcore: false,
                    levels: vec![spawn.dimension.clone()],
                    max_players: 20,
                    chunk_radius: 8,
                    simulation_distance: 8,
                    reduced_debug_info: false,
                    show_death_screen: true,
                    limited_crafting: false,
                    spawn: spawn.clone(),
                    online_mode: false,
                    enforces_secure_chat: false,
                }),
                ["respawn", dimension @ ..] => C::Respawn(play::Respawn {
                    spawn: play::SpawnInfo {
                        dimension: dimension
                            .first()
                            .map_or_else(|| spawn.dimension.clone(), |key| (*key).into()),
                        ..spawn.clone()
                    },
                    keep: 0,
                }),
                ["beams", crystal] => {
                    let id: u32 = crystal.parse().unwrap();
                    let index = game.crystals.iter().position(|c| c.id == id).unwrap();
                    game.dragon.as_mut().unwrap().nearest_crystal = Some(index);
                    check_beams = true;
                    C::SetEntityData(play::SetEntityData {
                        id: id as i32,
                        values: vec![(8, OPTIONAL_BLOCK_POS, Value::OptPos(Some([3, 325, 3])))],
                    })
                }
                ["add", id, kind] => {
                    let id: i32 = id.parse().unwrap();
                    C::AddEntity(play::AddEntity {
                        id,
                        uuid: id as u128,
                        kind: types.id(kind).unwrap(),
                        position: [id as f64 % 5.0, 321.0, 0.5],
                        velocity: [0.0; 3],
                        pitch: 0,
                        yaw: 0,
                        head_yaw: 0,
                        data: 0,
                    })
                }
                ["item", id] => C::SetEntityData(play::SetEntityData {
                    id: id.parse().unwrap(),
                    values: vec![(
                        ITEM_ENTITY_ITEM,
                        ITEM_STACK,
                        Value::Item(Some(Stack::new(stone, 1))),
                    )],
                }),
                ["remove", ids @ ..] => C::RemoveEntities(play::RemoveEntities {
                    ids: ids.iter().map(|id| id.parse().unwrap()).collect(),
                }),
                ["passengers", vehicle, passengers @ ..] => {
                    let vehicle = vehicle.parse().unwrap();
                    let passengers: Vec<i32> =
                        passengers.iter().map(|id| id.parse().unwrap()).collect();
                    game.handle_packet(C::SetPassengers(play::SetPassengers {
                        vehicle,
                        passengers: passengers.clone(),
                    }))
                    .unwrap();
                    let remote = game.client.remote.as_ref().unwrap();
                    assert_eq!(
                        remote.passenger_seats(vehicle as u32).count(),
                        passengers.len()
                    );
                    for id in passengers {
                        assert_eq!(remote.passenger_vehicle(id as u32), Some(vehicle as u32));
                    }
                    continue;
                }
                ["camera", camera_id] => C::SetCamera(play::SetCamera {
                    camera_id: camera_id.parse().unwrap(),
                }),
                ["position"] => C::PlayerPosition(play::PlayerPosition {
                    teleport: 1,
                    position: [0.5, 321.0, 0.5],
                    delta: [0.0; 3],
                    yaw: 0.0,
                    pitch: 0.0,
                    relatives: 0,
                }),
                ["frame"] => {
                    game.client.camera_type = CameraType::ThirdPersonBack;
                    game.tick_player_model();
                    let ((vertices, ..), ids) = submitted_bodies(game, |game| {
                        game.entity_geometry(
                            glam::DVec3::new(0.5, 321.0, -4.5),
                            1.0,
                            None,
                            Vec::new(),
                            &mut Vec::new(),
                        )
                    });
                    if std::mem::take(&mut check_beams) {
                        let spans = game.client.entity_spans.borrow();
                        let entities = sunrise_shaders::render_stage("ENTITIES");
                        let none = sunrise_shaders::render_stage("NONE");
                        let beams: Vec<_> = spans
                            .windows(3)
                            .filter_map(|w| {
                                let [body, beam, next] = w else {
                                    unreachable!()
                                };
                                let mesh = &vertices[beam.start as usize..next.start as usize];
                                (!mesh.is_empty() && mesh.iter().all(|v| v.layer & 0xffff == 6))
                                    .then_some((body, beam))
                            })
                            .collect();
                        assert_eq!(beams.len(), 2);
                        for ((body, beam), pack_id) in beams.into_iter().zip([101, 102]) {
                            assert_eq!((body.stage, body.id), (entities, pack_id));
                            assert!(body.start < beam.start);
                            assert_eq!(beam.stage, none);
                        }
                    }
                    println!("entity-draw-order: {ids:?}");
                    snapshots.push(ids);
                    continue;
                }
                _ => panic!("unknown scene operation: {line}"),
            };
            game.handle_packet(packet).unwrap();
        }
        assert_eq!(
            snapshots,
            [
                vec![4321, 91],
                vec![91, 40, 4321, 3],
                vec![4321, 3],
                vec![4321, 3, 40, 91],
                vec![4321, 3, 91, 40],
                vec![4321, 91, 40, 3],
                vec![4321, 91, 40, 3],
                vec![4321, 91, 40, 3],
                vec![91, 40, 3],
                vec![4321, 91, 40, 3],
                vec![4321, 91, 40, 3, 7, 8, 9],
                vec![4321, 91, 40, 3, 8, 9, 7],
                vec![4321, 23, 82, 110, 20],
                vec![4321, 23, 82, 110, 20],
                vec![4321, 82, 110, 20, 23],
                vec![4321, 82, 23, 110, 20],
                vec![4321],
                vec![4321, 110, 23],
            ]
        );
    });
}

#[cfg(test)]
#[test]
#[ignore = "daily"]
fn a_canceled_living_pre_skips_the_model_and_the_post_hook() {
    if super::native_entity_refs::tests::run_fresh_cases(
        "game::client_side::native_render_pair_tests::a_canceled_living_pre_skips_the_model_and_the_post_hook",
        "SUNRISE_RENDER_PAIR_CHILD",
        &["game"],
    ) {
        return;
    }
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_pair(&heard, |game| {
        game.common
            .mobs
            .push(mob(game, "minecraft:zombie", 91, 0.0));
        let drawn = mob_frame(game);
        assert!(drawn.len() > 4, "its model, beside the marker");
        events(&heard);
        heard.lock().unwrap().cancel_living.push(91);
        let canceled = mob_frame(game);
        assert_eq!(events(&heard), ["living_pre 91"], "no post hook");
        assert_eq!(
            canceled.len(),
            4,
            "the marker alone, no model: {}",
            canceled.len()
        );
        assert_eq!(markers(&canceled), 4);
        // The frame's later name tag and leash passes leave it out.
        assert!(game.render_canceled(91));
    });
}

#[cfg(test)]
#[test]
#[ignore = "daily"]
fn a_canceled_entity_draws_no_name_tag() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_pair(&heard, |game| {
        let mut named = mob(game, "minecraft:zombie", 91, 0.0);
        named.custom_name = Some("Bob".into());
        named.name_visible = true;
        game.mobs.push(named);
        let tag = |game: &mut crate::game::ClientGame<'_>| {
            mob_frame(game);
            let (mut out, mut see_through) = (Vec::new(), Vec::new());
            let mut collector = std::mem::take(&mut game.client.see_through_collector);
            collector.clear();
            game.name_tag_geometry(CAMERA, 1.0, &mut out, &mut collector);
            collector.flush(&mut see_through);
            game.client.see_through_collector = collector;
            out.len() + see_through.len()
        };
        assert!(tag(game) > 0, "a named zombie's tag");
        heard.lock().unwrap().cancel_living.push(91);
        assert_eq!(tag(game), 0, "none once its pre hook cancels");
    });
}

/// A stone floor under the origin, so a standing entity has something to cast a shadow on.
#[cfg(test)]
fn place_stone_floor(game: &mut crate::game::ClientGame<'_>) {
    game.common
        .world
        .chunks
        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
    let stone = game
        .common
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.common.world.set(0, 63, 0, stone).unwrap();
}

/// Vertices of one whole entity frame (`entity_geometry`).
#[cfg(test)]
fn entity_frame(game: &mut crate::game::ClientGame<'_>) -> usize {
    let (out, ..) = game.entity_geometry(CAMERA, 1.0, None, Vec::new(), &mut Vec::new());
    out.len()
}

/// The vertices of one whole entity frame with a zombie standing on stone, burning when
/// `burning`, with entity shadows on when `shadows`.
#[cfg(test)]
fn zombie_frame(game: &mut crate::game::ClientGame<'_>, burning: bool, shadows: bool) -> usize {
    let mut zombie = mob(game, "minecraft:zombie", 91, 0.0);
    zombie.fire_ticks = if burning { 100 } else { 0 };
    game.common.mobs.clear();
    game.common.mobs.push(zombie);
    game.client.settings.entity_shadows = shadows;
    entity_frame(game)
}

#[cfg(test)]
#[test]
#[ignore = "daily"]
fn a_canceled_pre_keeps_the_flames_and_the_shadow() {
    if super::native_entity_refs::tests::run_fresh_cases(
        "game::client_side::native_render_pair_tests::a_canceled_pre_keeps_the_flames_and_the_shadow",
        "SUNRISE_RENDER_PAIR_CHILD",
        &["game"],
    ) {
        return;
    }
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_pair(&heard, |game| {
        place_stone_floor(game);
        // FlameFeatureRenderer submits six quads for an adult zombie; the stone gives one shadow quad.
        let (flames, shadow) = (24, 4);
        let plain = zombie_frame(game, false, false);
        assert_eq!(zombie_frame(game, true, false) - plain, flames);
        assert_eq!(zombie_frame(game, false, true) - plain, shadow);
        heard.lock().unwrap().cancel_living.push(91);
        let plain = zombie_frame(game, false, false);
        assert_eq!(zombie_frame(game, true, false) - plain, flames);
        assert_eq!(zombie_frame(game, false, true) - plain, shadow);
        assert!(game.render_canceled(91));
    });
}

#[cfg(test)]
#[test]
#[ignore = "daily"]
fn a_canceled_pre_keeps_the_local_players_flames() {
    if super::native_entity_refs::tests::run_fresh_cases(
        "game::client_side::native_render_pair_tests::a_canceled_pre_keeps_the_local_players_flames",
        "SUNRISE_RENDER_PAIR_CHILD",
        &["game"],
    ) {
        return;
    }
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_player_pair(&heard, |game| {
        place_stone_floor(game);
        admit_own_player(game, false);
        game.client.settings.entity_shadows = false;
        let expected = 20; // FlameFeatureRenderer submits five quads for a standing player.
        let added = |game: &mut crate::game::ClientGame<'_>| {
            game.player.as_mut().unwrap().fire_ticks = 0;
            let plain = entity_frame(game);
            game.player.as_mut().unwrap().fire_ticks = 100;
            entity_frame(game) - plain
        };
        assert_eq!(added(game), expected);
        heard.lock().unwrap().cancel_living.push(4321);
        assert_eq!(added(game), expected);
        assert!(game.own_render_canceled());
        heard.lock().unwrap().cancel_player.push(true);
        assert_eq!(added(game), expected);
        assert!(game.own_render_canceled());
    });
}

#[cfg(test)]
#[test]
#[ignore = "daily"]
fn a_culled_entity_is_asked_neither_hook() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_pair(&heard, |game| {
        // Far past any render distance.
        game.common
            .mobs
            .push(mob(game, "minecraft:zombie", 91, 100_000.0));
        let out = mob_frame(game);
        assert_eq!(events(&heard), Vec::<String>::new());
        assert_eq!(markers(&out), 0);
    });
}

/// Admit this window's player in third person with entity id 4321.
#[cfg(test)]
fn admit_own_player(game: &mut crate::game::ClientGame<'_>, spectator: bool) {
    let at = sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5);
    let mut player = sunrise_sim::motion::PlayerMotion::new(game.common.motion, at).unwrap();
    player.entity_id = 4321;
    game.common.player = Some(player);
    game.common.previous_position = at;
    if let Some(remote) = game.client.remote.as_mut() {
        remote.entity_id = 4321;
    }
    game.client.camera_type = sunrise_level::camera_math::CameraType::ThirdPersonBack;
    game.common.resident_entities.local_player.renew();
    game.tick_player_model();
    game.client.player_model.as_mut().unwrap().extra.spectator = spectator;
}

#[cfg(test)]
fn own_player_frame(game: &mut crate::game::ClientGame<'_>, spectator: bool) -> Vec<Vertex> {
    admit_own_player(game, spectator);
    game.prepare_entity_culling();
    let mut out = Vec::new();
    game.player_model_geometry(CAMERA, 1.0, &mut out);
    out
}

#[cfg(test)]
#[test]
#[ignore = "daily"]
fn the_player_pair_wraps_the_living_pair() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_player_pair(&heard, |game| {
        own_player_frame(game, false);
        assert_eq!(
            events(&heard),
            [
                "player_pre true head Some(true)",
                "living_pre 4321 head Some(false)",
                "living_post 4321",
                "player_post true model true head Some(false)"
            ],
            "Living Pre and the final Player Post model both see the Player Pre edit"
        );
        heard.lock().unwrap().cancel_living.push(4321);
        let out = own_player_frame(game, false);
        assert_eq!(
            events(&heard),
            [
                "player_pre true head Some(true)",
                "living_pre 4321 head Some(false)",
                "player_post true model false head None"
            ],
            "the edit is draw-local, and canceling only Living Pre still calls Player Post without a model"
        );
        assert_eq!((out.len(), markers(&out)), (4, 4), "only the marker draws");
        assert!(game.own_render_canceled());
        heard.lock().unwrap().cancel_player.push(true);
        let out = own_player_frame(game, false);
        assert_eq!(
            events(&heard),
            ["player_pre true head Some(true)"],
            "a canceled player pre skips the rest"
        );
        assert!(out.is_empty());
        assert!(game.own_render_canceled());
    });
}

#[cfg(test)]
#[test]
#[ignore = "daily"]
fn a_spectator_draws_no_layers_but_gets_its_post_hooks() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_player_pair(&heard, |game| {
        // This window's player, spectating, in third person.
        own_player_frame(game, true);
        assert_eq!(
            events(&heard),
            [
                "player_pre true head Some(true)",
                "living_pre 4321 head Some(false)",
                "living_post 4321",
                "player_post true model true head Some(false)"
            ]
        );
        // A spectating mannequin in the level's draw.
        let mut mannequin = mob(game, "minecraft:mannequin", 94, 0.0);
        mannequin.extra.spectator = true;
        game.mobs.push(mannequin);
        mob_frame(game);
        assert_eq!(events(&heard), ["living_pre 94", "living_post 94"]);
    });
}

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
