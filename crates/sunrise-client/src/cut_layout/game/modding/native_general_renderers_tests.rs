//! A mod's general entity types on the client (#792, stage 5): a renderer registered for one
//! resolves to its catalog slot, a type with none is named; each row shown on the window is drawn
//! by its type's renderer at the frame's interpolated pose, with its synced attachments; a renderer that fails is dropped until resources reload; a type
//! with no renderer draws nothing.
use mod_api::{
    AttachmentCodec, AttachmentDescriptor, AttachmentError, AttachmentKey, BlockEntityDrawContext,
    EntityRenderState, EntityRenderer,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};
use sunrise_client::mesher::Vertex;
use sunrise_client_gameplay::game::client_side::native_entity_renderers::{
    EntityRendererTable, RegisteredEntityRenderer,
};
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::general::GeneralTypeCatalog;
use sunrise_protocol::packets::play::{self, Clientbound as C};

fn declaration(id: &str) -> mod_api::__private::GeneralEntityTypeDeclaration {
    mod_api::__private::GeneralEntityTypeDeclaration {
        mod_id: "test".into(),
        id: id.into(),
        category: mod_api::GeneralEntityCategory::Misc,
        width: 0.5,
        height: 0.5,
        passenger_attachment: None,
        save_policy: mod_api::GeneralSavePolicy::Save,
        summonable: true,
        pickable: false,
        fire_immune: false,
        piston_push_reaction: mod_api::PushReaction::PushPull,
        tracking_chunks: 8,
        update_interval: 1,
        payload_version: 1,
        required_features: Vec::new(),
    }
}

/// Draws nothing.
struct Blank;
impl EntityRenderer for Blank {
    fn submit(
        &self,
        _: &EntityRenderState,
        _: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn a_general_types_renderer_resolves_to_its_slot_and_a_type_without_one_is_named() {
    let catalog =
        GeneralTypeCatalog::freeze(&[declaration("test:missile"), declaration("test:plain")])
            .unwrap();
    let registered: BTreeMap<String, RegisteredEntityRenderer> = [(
        "test:missile".to_owned(),
        RegisteredEntityRenderer::new(Arc::new(Blank), None),
    )]
    .into();
    let (table, unknown) =
        EntityRendererTable::resolve(&registered, &[], &Default::default(), &catalog, |_| false);
    assert!(
        unknown.is_empty(),
        "a general key is an entity type: {unknown:?}"
    );
    let missile = catalog.slot("test:missile").unwrap();
    assert!(table.general(missile).is_some());
    assert!(table.general(catalog.slot("test:plain").unwrap()).is_none());
    assert_eq!(table.unrendered_general(&catalog), ["test:plain"]);
}

const FUEL_CODEC: AttachmentCodec<i32> = AttachmentCodec {
    encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
    decode: |c| match c.get(&"n".into()) {
        Some(Tag::Int(n)) => Ok(*n),
        _ => Err(AttachmentError::Decode),
    },
};

/// `test:missile`'s renderer: draws one stone quad from its pose's origin, and keeps the last
/// state it was handed and the fuel it read from the snapshot.
#[derive(Default)]
struct Missile {
    fuel_key: Mutex<Option<AttachmentKey<i32>>>,
    seen: Mutex<Option<EntityRenderState>>,
    fuel: Mutex<Option<i32>>,
}
impl EntityRenderer for Missile {
    fn submit(
        &self,
        entity: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        *self.seen.lock().unwrap() = Some(entity.clone());
        let fuel_key = self
            .fuel_key
            .lock()
            .unwrap()
            .clone()
            .expect("test:fuel registered");
        let fuel = match &entity.attachments {
            Some(snapshot) => snapshot
                .get(&fuel_key)
                .map_err(|e| format!("{e:?}"))?
                .copied(),
            None => None,
        };
        *self.fuel.lock().unwrap() = fuel;
        let extent = fuel.unwrap_or(1) as f32;
        let corners = [
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            [extent, 0.0, 0.0],
            [extent, 1.0, 0.0],
        ];
        draw.quad("block/stone", corners, [0.0, 0.0, 16.0, 16.0], 0xFF_FFFF);
        Ok(())
    }
}

/// `test:bomb`'s renderer: panics, and counts its calls.
#[derive(Default)]
struct Bomb {
    calls: AtomicU32,
}
impl EntityRenderer for Bomb {
    fn submit(
        &self,
        _: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        draw.quad(
            "block/stone",
            [
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
            [0.0, 0.0, 16.0, 16.0],
            0xFF_FFFF,
        );
        panic!("the bomb's renderer");
    }
}

/// The scene's mod: `test:missile`, `test:bomb` and `test:plain` declared, `test:fuel` synced,
/// renderers for the first two.
struct Scene {
    missile: Arc<Missile>,
    bomb: Arc<Bomb>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Scene {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let fuel = r
            .register_attachment_type(
                AttachmentDescriptor::new("test:fuel", || 0i32).sync(FUEL_CODEC),
            )
            .unwrap();
        *self.missile.fuel_key.lock().unwrap() = Some(fuel);
        for path in ["missile", "bomb", "plain"] {
            r.declare_general_entity(
                path,
                mod_api::GeneralEntityCategory::Misc,
                0.5,
                0.5,
                None,
                mod_api::GeneralSavePolicy::Save,
                true,
                false,
                false,
                None,
                8,
                1,
                1,
                &[],
            )
            .unwrap();
        }
    }
    fn register_entity_renderers(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterEntityRenderersEvent<'_>,
    ) {
        event
            .renderers
            .register("test:missile".into(), self.missile.clone());
        event
            .renderers
            .register("test:bomb".into(), self.bomb.clone());
    }
}

/// The headless camera just above build height, where vanilla needs no compiled section.
const CAMERA: glam::DVec3 = glam::DVec3::new(0.0, 321.0, -5.0);

/// The server's spawn of row `id` of `key` at `at`.
fn spawn(id: i32, key: &str, at: [f64; 3]) -> C {
    use sunrise_protocol::general_entity::{CHANNEL, Message, encode};
    C::CustomPayload(sunrise_protocol::packets::configuration::CustomPayload {
        channel: CHANNEL.into(),
        data: encode(&Message::Spawn {
            id,
            uuid: 0x100 + id as u128,
            type_key: key.into(),
            position: at,
            rotation: [0.0, 0.0],
            velocity: [0.0; 3],
        })
        .unwrap(),
    })
}

/// The server's snapshot of row `id`'s synced fuel.
fn snapshot(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, id: u32, fuel: i32) {
    use sunrise_mod_runtime::attachments::AttachmentSync;
    apply_attachment_snapshot(
        game,
        id,
        vec![AttachmentSync {
            namespace: "test".into(),
            name: "fuel".into(),
            value: Some([("n".into(), Tag::Int(fuel))].into()),
        }],
    );
}

fn apply_attachment_snapshot(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    id: u32,
    changes: Vec<sunrise_mod_runtime::attachments::AttachmentSync>,
) {
    use sunrise_mod_runtime::attachments::Holder;
    use sunrise_protocol::attachment_sync::{CHANNEL, Message, decode_staged, encode};
    let payload = sunrise_protocol::packets::configuration::CustomPayload {
        channel: CHANNEL.into(),
        data: encode(&Message {
            holder: Holder::Entity(id),
            replace: true,
            changes,
        })
        .unwrap(),
    };
    // Attachment sync is staged at play ingress, not handled as a raw game packet.
    game.apply_staged_attachment_sync(decode_staged(payload).unwrap());
}

/// One frame of the entity pass at `alpha`.
fn frame(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, alpha: f64) -> Vec<Vertex> {
    game.entity_geometry(CAMERA, alpha, None, Vec::new(), &mut Vec::new())
        .0
}

/// The scene on a client (its block atlas loaded) joined to a server through memory, with its
/// rows shown: `test:missile`
/// (id 5) going from (0, 321, 0) to (4, 321, 0) with fuel 7, `test:bomb` (6) and `test:plain` (7).
/// In a child process running only `test`: the registrations are kept once per process.
fn in_scene(
    test: &str,
    check: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>, &Scene),
) {
    if super::native_entity_refs::tests::run_fresh_cases(
        &format!("cut_layout::game::client_side::native_general_renderers_tests::{test}"),
        "SUNRISE_GENERAL_RENDERERS_CHILD",
        &["native"],
    ) {
        return;
    }
    let held = Scene {
        missile: Arc::default(),
        bomb: Arc::default(),
    };
    let (missile, bomb) = (held.missile.clone(), held.bomb.clone());
    let mods = move || {
        vec![mod_api::ModEntry::new(
            "test",
            Box::new(Scene {
                missile: missile.clone(),
                bomb: bomb.clone(),
            }),
        )]
    };
    mod_platform::__private::initialize().unwrap();
    // A general type is declared only while the registration phase is current.
    let registration = mod_platform::__private::RegistrationGuard::open();
    mod_platform::testing::with_mods(mods().into(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods()).unwrap();
        sunrise_client_gameplay::game::client_side::mods_client::register_entity_renderers(
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        );
        // A frame draws with a world open, as the window does.
        drop(registration);
        super::native_entity_refs::tests::with_client_game(|game| {
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "Steve").unwrap();
            game.resolve_entity_renderers();
            // Logged in to the overworld: the client's level, whose rows its host names.
            game.handle_packet(C::Login(play::Login {
                player_id: 1,
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
            for (id, key) in [(5, "test:missile"), (6, "test:bomb"), (7, "test:plain")] {
                game.handle_packet(spawn(id, key, [0.0, 321.0, 0.0]))
                    .unwrap();
            }
            snapshot(game, 5, 7);
            let row = game
                .client
                .remote
                .as_mut()
                .unwrap()
                .general
                .rows
                .get_mut(&5)
                .unwrap();
            (row.previous, row.position) = ([0.0, 321.0, 0.0], [4.0, 321.0, 0.0]);
            (row.previous_rotation, row.rotation) = ([0.0, -20.0], [90.0, 20.0]);
            row.age = 40;
            check(game, &held);
        });
    });
}

/// A quarter of the way between ticks: the pose stack starts at x = 1, and the state is
/// interpolated the same way; the row's synced fuel sets the quad width.
#[test]
#[ignore = "daily"]
fn a_row_is_drawn_by_its_types_renderer_at_the_frames_pose_reading_its_synced_fields() {
    in_scene(
        "a_row_is_drawn_by_its_types_renderer_at_the_frames_pose_reading_its_synced_fields",
        |game, scene| {
            // A one-plane test frustum: +Z in camera space is in front.
            game.client.view_frustum = Some(sunrise_client::gfx::test_frustum([glam::Vec4::Z; 6]));
            for (id, at) in [
                (8, [0.0, 321.0, -8.0]),  // behind the camera
                (9, [0.0, 321.0, 300.0]), // beyond the draw range
                (10, [0.0, 319.0, 0.0]),  // in build height, no compiled section
            ] {
                game.handle_packet(spawn(id, "test:missile", at)).unwrap();
                snapshot(game, id as u32, 99);
            }
            let out = frame(game, 0.25);
            assert_eq!(
                out.len(),
                4,
                "the missile's one quad, nothing for the others: {out:?}"
            );
            let xs: Vec<f32> = out.iter().map(|v| v.pos[0]).collect();
            let min = xs.iter().copied().fold(f32::INFINITY, f32::min);
            assert_eq!(min, 1.0 - CAMERA.x as f32);
            assert_eq!(xs.iter().copied().fold(f32::NEG_INFINITY, f32::max), 8.0);
            let seen = scene.missile.seen.lock().unwrap().clone().unwrap();
            assert_eq!((seen.id, seen.partial, seen.age), (5, 0.25, 40.25));
            assert_eq!((seen.yaw, seen.pitch), (22.5, -10.0));
            assert!(seen.living.is_none());
            assert_eq!(*scene.missile.fuel.lock().unwrap(), Some(7));
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_synced_field_larger_than_the_old_render_cap_reaches_submit() {
    use sunrise_mod_runtime::attachments::AttachmentSync;
    in_scene(
        "a_synced_field_larger_than_the_old_render_cap_reaches_submit",
        |game, scene| {
            apply_attachment_snapshot(
                game,
                5,
                vec![AttachmentSync {
                    namespace: "test".into(),
                    name: "fuel".into(),
                    value: Some(
                        [
                            ("bytes".into(), Tag::ByteArray(vec![0; 70 * 1024])),
                            ("n".into(), Tag::Int(11)),
                        ]
                        .into(),
                    ),
                }],
            );
            let out = frame(game, 1.0);
            assert_eq!(out.len(), 4);
            assert_eq!(
                out.iter()
                    .map(|v| v.pos[0])
                    .fold(f32::NEG_INFINITY, f32::max),
                15.0,
            );
            assert_eq!(*scene.missile.fuel.lock().unwrap(), Some(11));
        },
    );
}

#[test]
#[ignore = "daily"]
fn general_submission_keeps_lookup_order_across_removal_and_reentry() {
    in_scene(
        "general_submission_keeps_lookup_order_across_removal_and_reentry",
        |game, _| {
            let builtin = game.client.remote.as_ref().unwrap().builtin.clone();
            let types = builtin.get("minecraft:entity_type").unwrap();
            let vanilla = |id, kind| {
                C::AddEntity(play::AddEntity {
                    id,
                    uuid: id as u128,
                    kind: types.id(kind).unwrap(),
                    position: [0.0, 321.0, 0.0],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                })
            };
            game.client.pack_ids = Some(
                sunrise_client_gameplay::game::client_side::shader_packs::PackIds::with_entity_ids(
                    [("test:missile", 77)],
                ),
            );
            for packet in [
                vanilla(71, "minecraft:armor_stand"),
                spawn(12, "test:missile", [6.0, 321.0, 0.0]),
                vanilla(42, "minecraft:zombie"),
                spawn(8, "test:missile", [8.0, 321.0, 0.0]),
            ] {
                game.handle_packet(packet).unwrap();
            }
            snapshot(game, 8, 1);
            let submitted = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                sunrise_client_gameplay::game::client_side::native_render_pair_tests::submitted_bodies(game, |game| frame(game, 1.0)).1
            };
            assert_eq!(submitted(game), [5, 71, 12, 42, 8]);
            // The row is drawable from spawn. Sync updates its fields without moving its slot.
            snapshot(game, 12, 1);
            assert_eq!(submitted(game), [5, 71, 12, 42, 8]);
            let own_type_spans = game
                .client
                .entity_spans
                .borrow()
                .windows(2)
                .filter(|w| w[0].id == 77 && w[0].start < w[1].start)
                .count();
            assert_eq!(own_type_spans, 3);
            // The later zombie is the row's vehicle, not its render slot.
            game.handle_packet(C::SetPassengers(play::SetPassengers {
                vehicle: 42,
                passengers: vec![12],
            }))
            .unwrap();
            game.position_remote_general_passengers();
            assert_eq!(
                game.client.remote.as_ref().unwrap().passenger_vehicle(12),
                Some(42)
            );
            assert_eq!(submitted(game), [5, 71, 12, 42, 8]);
            game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![12] }))
                .unwrap();
            game.handle_packet(spawn(12, "test:missile", [6.0, 321.0, 0.0]))
                .unwrap();
            snapshot(game, 12, 1);
            assert_eq!(submitted(game), [5, 71, 42, 8, 12]);

            game.handle_packet(C::Respawn(play::Respawn {
                spawn: play::SpawnInfo {
                    dimension_type: 0,
                    dimension: "minecraft:the_end".into(),
                    seed: 1,
                    game_type: 0,
                    previous_game_type: None,
                    debug: false,
                    flat: false,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 0,
                },
                keep: 0,
            }))
            .unwrap();
            assert_eq!(submitted(game), [] as [u32; 0]);
            game.handle_packet(vanilla(42, "minecraft:zombie")).unwrap();
            game.handle_packet(spawn(12, "test:missile", [6.0, 321.0, 0.0]))
                .unwrap();
            assert_eq!(submitted(game), [42, 12]);
            snapshot(game, 12, 1);
            assert_eq!(submitted(game), [42, 12]);
        },
    );
}

/// A panicking renderer rolls back its draw, stays disabled for the session (including across a
/// resource reload), and queues an error toast naming the mod; a type with no renderer draws
/// nothing, and neither stops the other renderers from the same mod.
#[test]
#[ignore = "daily"]
fn a_panicking_renderer_stays_disabled_across_reload_and_a_type_without_one_draws_nothing() {
    in_scene(
        "a_panicking_renderer_stays_disabled_across_reload_and_a_type_without_one_draws_nothing",
        |game, scene| {
            let _ = mod_api::__private::take_hook_panics();
            assert_eq!(frame(game, 1.0).len(), 4, "the missile alone");
            assert_eq!(scene.bomb.calls.load(Ordering::Relaxed), 1);
            assert_eq!(
                mod_api::__private::take_hook_panics(),
                [mod_api::__private::HookPanicReport {
                    mod_id: "test",
                    hook: "EntityRenderer::submit",
                }],
                "the shared journal records the mod and retained operation once"
            );

            assert_eq!(frame(game, 1.0).len(), 4, "the missile alone on next frame");
            assert_eq!(scene.bomb.calls.load(Ordering::Relaxed), 1);
            assert!(mod_api::__private::take_hook_panics().is_empty());

            sunrise_client_gameplay::game::client_side::model_baking_completed_tests::reload(game);
            assert_eq!(frame(game, 1.0).len(), 4);
            assert_eq!(
                scene.bomb.calls.load(Ordering::Relaxed),
                1,
                "stays disabled across resource reload for the rest of the session"
            );
            assert!(mod_api::__private::take_hook_panics().is_empty());
            // The reload kept the rows.
            assert_eq!(game.client.remote.as_ref().unwrap().general.rows.len(), 3);
        },
    );
}

#[cfg(any())]
#[path = "native_general_renderers_tests/fixture.rs"]
mod fixture;

#[cfg(any())]
struct NativeProbe {
    keys: Arc<fixture::Keys>,
}
#[cfg(any())]
#[mod_api::__subscriptions]
impl mod_api::Hooks for NativeProbe {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        fixture::register(registrar, &self.keys);
    }

    fn register_entity_renderers(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterEntityRenderersEvent<'_>,
    ) {
        fixture::register_renderer(event.renderers, &self.keys);
    }
}

/// Loads a saved row of the fixture's type at `position` with fuel 7: the chunk-load route.
#[cfg(any())]
fn load_saved_row(
    server: &mut sunrise_gameplay::game::ServerGame<'_>,
    position: [f64; 3],
    fuel: &AttachmentKey<i32>,
) -> u32 {
    use mod_api::{Holder, Host};
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
    let uuid = [380, 39, 9, 57];
    let saved = Tag::Compound(
        [
            ("id".into(), Tag::String(fixture::KIND.into())),
            (
                "Pos".into(),
                Tag::List(position.map(|axis| Tag::Double(axis.to_bits())).into()),
            ),
            ("UUID".into(), Tag::IntArray(uuid.into())),
            ("PayloadVersion".into(), Tag::Short(1)),
        ]
        .into(),
    );
    server
        .load_listed(fixture::KIND, Some(uuid), None, saved)
        .unwrap();
    let id = server
        .resident_entities
        .general
        .rows
        .iter()
        .find(|row| row.uuid == uuid)
        .unwrap()
        .id;
    server.storage_scope(|server, _| {
        fuel.set(server as &mut dyn Host, Holder::Entity(id), 7)
            .unwrap();
        ((), true)
    });
    id
}

/// Spawns a row of the fixture's type at `position` through `LevelRef::spawn_entity`, writing
/// its fuel of 7 on the returned handle in the same transaction: the route a mod's own code
/// takes, so the row's first tracking snapshot holds the field.
#[cfg(any())]
fn spawn_row(
    server: &mut sunrise_gameplay::game::ServerGame<'_>,
    position: [f64; 3],
    fuel: &AttachmentKey<i32>,
) -> u32 {
    use mod_api::{EntitySpawn, Holder, Host, SpawnBody};
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
    server.storage_scope(|server, _| {
        let level = (server as &mut dyn Host)
            .level("minecraft:overworld")
            .expect("the overworld is in place");
        let row = level
            .spawn_entity(
                server as &mut dyn mod_api::ServerHost,
                fixture::KIND,
                position,
                &EntitySpawn::new(SpawnBody::General),
            )
            .expect("the spawn is admitted");
        fuel.set(server as &mut dyn Host, Holder::Entity(row.id()), 7)
            .unwrap();
        (row.id(), true)
    })
}

/// Opens an integrated flat world whose window reads the fixture's renderer, has `place` put a row of the fixture's type with fuel 7 on the server above
/// build height, and checks that the window shows it: the tracker syncs the row and its field
/// with no injected client packets, and the renderer's quad, whose width is the fuel, is in the
/// frame's entity pass, before and after a resource reload that leaves the row in place.
#[cfg(any())]
fn integrated_scene(
    test: &str,
    place: fn(&mut sunrise_gameplay::game::ServerGame<'_>, [f64; 3], &AttachmentKey<i32>) -> u32,
) {
    const CASE: &str = "SUNRISE_GENERAL_RENDERER_TARGET";
    if super::native_entity_refs::tests::run_fresh_cases(
        &format!("cut_layout::game::client_side::native_general_renderers_tests::{test}"),
        CASE,
        &["game"],
    ) {
        return;
    }
    sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack(move || {
        mod_platform::__private::initialize().unwrap();
        let keys = Arc::new(fixture::Keys::default());
        let mods = vec![mod_api::ModEntry::new(
            "general_render_fixture",
            Box::new(NativeProbe { keys: keys.clone() }),
        )]
        .into();
        let registration = mod_platform::__private::RegistrationGuard::open();
        mod_platform::testing::with_mods(mods, || {
            super::native_entity_refs::tests::with_client_game(|game| {
                sunrise_client_gameplay::game::client_side::mods_client::register_entity_renderers(
                    sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
                );
                drop(registration);
                game.resolve_entity_renderers();
                game.client.settings.render_distance = 2;
                game.client.settings.simulation_distance = 2;
                let options = sunrise_sim::world_options::WorldOptions {
                    world_type: sunrise_sim::save::WorldPreset::Flat,
                    ..Default::default()
                };
                game.open_integrated("general_render", None, Some(1), true, Some(&options))
                    .unwrap();
                poll_until(game, |g| {
                    g.client
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.logged_in && r.joined)
                });
                let fuel = keys.fuel();
                let (id, at) = game
                    .client
                    .integrated
                    .as_ref()
                    .unwrap()
                    .call(move |server| {
                        let [x, _, z] = server.server.lan.as_ref().unwrap().hub.guests[0].position;
                        // The headless fixture has no GPU to compile the section. Keep the row
                        // above build height, where the vanilla section gate does not apply.
                        let position = [
                            x.floor(),
                            f64::from(server.world.shape.max_y() + 1),
                            z.floor(),
                        ];
                        (place(server, position, &fuel), position)
                    })
                    .unwrap();
                let camera = glam::DVec3::from_array(at) - glam::DVec3::Z * 5.0;
                let ready = |g: &sunrise_client_gameplay::game::ClientGame<'_>| {
                    g.client
                        .remote
                        .as_ref()
                        .unwrap()
                        .general
                        .rows
                        .contains_key(&(id as i32))
                };
                poll_until(game, ready);
                let check_quad = |g: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                    let ((frame, ..), witnesses) =
                        sunrise_client_gameplay::game::client_side::native_render_pair_tests::capture_body_witnesses(|| {
                            g.entity_geometry(camera, 1.0, None, Vec::new(), &mut Vec::new())
                        });
                    let run = witnesses
                        .into_iter()
                        .find(|w| w.id == id)
                        .expect("the frame's entity pass carries the row's quad");
                    let vertices = &frame[run.vertices];
                    assert_eq!(vertices.len(), 4);
                    let x = vertices.iter().map(|v| v.pos[0]);
                    assert_eq!(x.clone().fold(f32::INFINITY, f32::min), 0.0);
                    assert_eq!(x.fold(f32::NEG_INFINITY, f32::max), 7.0);
                };
                check_quad(game);
                sunrise_client_gameplay::game::client_side::model_baking_completed_tests::reload(
                    game,
                );
                assert!(ready(game), "resource reload must preserve the tracked row");
                check_quad(game);
                game.leave_server(None).unwrap();
            });
        });
    });
}

/// The integrated server loads a real general row and syncs its fuel to its window through
/// the tracker. The renderer turns that field into geometry before and after resource reload.
#[test]
#[ignore = "daily"]
#[cfg(any())]
fn integrated_general_renderer_reads_synced_fuel() {
    integrated_scene(
        "integrated_general_renderer_reads_synced_fuel",
        load_saved_row,
    );
}

/// #2642 end to end: a test mod's general row spawned by `LevelRef::spawn_entity` in an
/// integrated world is drawn in its window by the type's renderer.
#[test]
#[ignore = "daily"]
#[cfg(any())]
fn a_spawned_general_row_is_drawn_in_its_window() {
    integrated_scene("a_spawned_general_row_is_drawn_in_its_window", spawn_row);
}

#[cfg(any())]
fn poll_until(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    ready: impl Fn(&sunrise_client_gameplay::game::ClientGame<'_>) -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !ready(game) {
        game.poll_remote().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "integrated general entity scene did not become ready"
        );
        std::thread::yield_now();
    }
}
