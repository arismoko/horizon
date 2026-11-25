use super::super::pickups::{Collector, Taken};
use mod_api::ClientHost;
use std::time::Duration;
use sunrise_client::entities::ExperienceOrb;
use sunrise_client::particles::{FireworkExplosion, ParticleOptions, ParticleType};
use sunrise_core::geometry::Vec3;

fn orb(at: Vec3) -> ExperienceOrb {
    ExperienceOrb {
        size_answer: None,
        attachment_lifetime: Default::default(),
        fall_distance: 0.0,
        rotation: [0.0, 0.0],
        join: Default::default(),
        no_gravity: false,
        id: 1,
        position: at,
        previous: at,
        velocity: Vec3::ZERO,
        value: 1,
        age: 0,
        ground: Default::default(),
        count: 1,
        fire_ticks: 0,
        ticks: 0,
        health: 5,
        air: 300,
        following: None,
    }
}

/// Fills every particle group Sunrise keeps apart: three flames, a multi-burst firework
/// starter (`NO_RENDER`), an item pickup (`ITEM_PICKUP`) and the elder guardian
/// apparition (`ELDER_GUARDIANS`), each live after one particle tick.
pub(in crate::game) fn fill(game: &mut super::Game<'_>) {
    let at = game.particles.camera;
    for _ in 0..3 {
        game.particles
            .add(&ParticleOptions::simple(ParticleType::Flame), at, [0.0; 3]);
    }
    let bursts = vec![FireworkExplosion::default(); 4];
    game.particles.fireworks(at, [0.0; 3], bursts, at, true);
    let v = Vec3::new(at[0], at[1], at[2]);
    game.add_pickup_toward(Taken::Orb(Box::new(orb(v))), Collector::Player, at);
    game.particles.elder_guardian();
    game.tick_particles();
}

/// `particle_count` and `clear_particles` through the client host. The count covers every
/// live group. A clear reads 0 at once and empties every group, its queue and emitters
/// before the next frame is drawn, with no tick in between (a paused or frozen game),
/// or at the next particle tick. A server answers 0 and refuses the clear.
#[test]
#[ignore = "daily"]
fn a_client_mod_counts_and_clears_particles() {
    super::super::native_entity_refs::tests::with_game(|game| {
        game.options.execution_role = super::ExecutionRole::Client;
        let host: &dyn ClientHost = game;
        assert_eq!(host.particle_count(), 0);
        fill(game);
        assert_eq!(game.pickups.live.len(), 1, "the pickup is live");
        let engine = game.particles.group_count();
        assert_eq!(engine, 5, "three flames, one starter and one guardian");
        let host: &dyn ClientHost = game;
        assert_eq!(host.particle_count() as usize, engine + 1);
        // A clear asked between frames: it reads 0 at once, and the next frame clears
        // every group without a particle tick.
        game.particles.tracking_emitter(
            ParticleOptions::simple(ParticleType::Crit),
            sunrise_client::particles::TrackingTarget::Entity(0),
            sunrise_client::particles::TrackingBody {
                feet: [0.0; 3],
                width: 1.0,
                height: 1.0,
            },
            None,
        );
        assert!(host_clear(game));
        let host: &dyn ClientHost = game;
        assert_eq!(host.particle_count(), 0);
        game.extras_frame(Duration::ZERO);
        assert_eq!(game.particles.group_count(), 0);
        assert!(game.pickups.live.is_empty());
        assert!(game.particles.elder_guardians().is_empty());
        // The emitter and the queue went too: a tick brings nothing back.
        game.tick_particles();
        assert_eq!(game.client_particle_count(), 0);
        // A clear asked before a particle tick lands at that tick.
        fill(game);
        assert!(host_clear(game));
        game.tick_particles();
        assert_eq!(game.particles.group_count(), 0);
        assert!(game.pickups.live.is_empty());
        let host: &dyn ClientHost = game;
        assert_eq!(host.particle_count(), 0);
        game.options.execution_role = super::ExecutionRole::IntegratedServer;
        let host: &dyn ClientHost = game;
        assert!(!host.clear_particles());
        assert_eq!(host.particle_count(), 0);
    });
}

struct ClearInGui;
#[mod_api::__subscriptions]
impl mod_api::Hooks for ClearInGui {
    fn render_gui_pre(
        &self,
        host: &dyn ClientHost,
        _: &mut mod_api::RenderGuiPreEvent,
    ) -> mod_api::Flow {
        assert!(host.clear_particles());
        mod_api::Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn late_gui_clear_prevents_same_frame_particle_geometry_without_a_tick() {
    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("clear_gui", Box::new(ClearInGui))].into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.options.execution_role = super::ExecutionRole::Client;
            fill(game);
            game.extras_frame(Duration::ZERO);
            let camera = glam::DVec3::from_array(game.particles.camera);
            let mut before = Vec::new();
            game.client_particle_geometry(camera, 0.0, 70.0, None, &mut before);
            assert!(
                !before.is_empty(),
                "live guardian, pickup and ordinary particles draw"
            );
            game.native_render_gui_pre(&mut false);
            assert_eq!(mod_api::prelude::EffectsClientHost::particle_count(game), 0);
            let (mut after, _, _, _) =
                game.entity_geometry(camera, 0.0, None, Vec::new(), &mut Vec::new());
            game.client_particle_geometry(camera, 0.0, 70.0, None, &mut after);
            assert!(
                after.is_empty(),
                "no stale pickup, guardian or ordinary quads"
            );
            assert_eq!(game.client_particle_count(), 0);
            game.tick_particles();
            assert_eq!(game.client_particle_count(), 0);
        });
    });
}

struct ClearInModel(std::sync::Arc<std::sync::atomic::AtomicUsize>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for ClearInModel {
    fn item_model_property(
        &self,
        host: &dyn ClientHost,
        _: &mut mod_api::ItemModelPropertyEvent<'_>,
    ) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        assert!(host.clear_particles());
    }
}

#[test]
#[ignore = "daily"]
fn a_model_callback_clear_discards_particle_geometry_already_extracted() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "clear_model",
        Box::new(ClearInModel(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.options.execution_role = super::ExecutionRole::Client;
            fill(game);
            let camera = glam::DVec3::from_array(game.particles.camera);
            let mut out = Vec::new();
            game.client_particle_geometry(camera, 0.0, 70.0, None, &mut out);
            let previous = out.len();
            assert!(previous > 0);
            let positions: Vec<_> = out.iter().map(|v| v.pos).collect();
            let motion = game.entity_motion.borrow().clone();
            // An actual picked-up item resolves a conditional model while the
            // preceding ordinary-particle pass has already produced geometry.
            std::sync::Arc::get_mut(&mut game.pack.item_renderer)
                .unwrap()
                .models
                .definitions
                .insert(
                    "minecraft:stick".into(),
                    sunrise_assets::item_model::ClientItem::from_json(
                        r#"{"model":{
                    "type":"minecraft:condition", "property":"test:clear",
                    "on_true":{"type":"minecraft:model","model":"minecraft:item/stick"},
                    "on_false":{"type":"minecraft:model","model":"minecraft:item/stick"}
                }}"#,
                    )
                    .unwrap(),
                );
            let at = game.particles.camera;
            game.spawn_item(
                sunrise_client::items::ItemStack::new("minecraft:stick", 1),
                at,
                [0.0; 3],
                0,
            );
            let item = game.dropped.pop().unwrap();
            game.add_pickup_toward(Taken::Item(Box::new(item)), Collector::Player, at);
            game.tick_particles();
            game.client_particle_geometry(camera, 0.0, 70.0, None, &mut out);
            assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
            assert_eq!(out.iter().map(|v| v.pos).collect::<Vec<_>>(), positions);
            assert_eq!(&game.entity_motion.borrow()[..motion.len()], &motion);
            assert_eq!(game.client_particle_count(), 0);
            game.tick_particles();
            assert_eq!(game.client_particle_count(), 0);
        });
    });
}

#[derive(Clone, Copy, PartialEq)]
enum ClearAt {
    Never,
    Gui,
    Model,
    FogColor,
    Fog,
}

struct FrameMod {
    at: std::sync::Arc<std::sync::Mutex<ClearAt>>,
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl FrameMod {
    fn clear(&self, at: ClearAt, host: &dyn ClientHost) {
        if *self.at.lock().unwrap() == at {
            assert!(host.clear_particles());
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for FrameMod {
    fn visuals(&self, _: &dyn ClientHost, event: &mut mod_api::VisualsEvent<'_>) {
        event.lights.push(mod_api::ColoredLight::new(
            [0.5, 65.5, 5.5],
            0x0000ff,
            4.0,
            1.0,
        ));
    }
    fn render_gui_pre(
        &self,
        host: &dyn ClientHost,
        _: &mut mod_api::RenderGuiPreEvent,
    ) -> mod_api::Flow {
        self.clear(ClearAt::Gui, host);
        mod_api::Flow::Continue
    }
    fn item_model_property(
        &self,
        host: &dyn ClientHost,
        _: &mut mod_api::ItemModelPropertyEvent<'_>,
    ) {
        self.clear(ClearAt::Model, host);
    }
    fn compute_fog_color(&self, host: &dyn ClientHost, _: &mut mod_api::ComputeFogColorEvent<'_>) {
        self.clear(ClearAt::FogColor, host);
    }
    fn render_fog(&self, host: &dyn ClientHost, _: &mut mod_api::RenderFogEvent<'_>) {
        self.clear(ClearAt::Fog, host);
    }
}

// A real renderer supplies the native pack's rules and the GPU skinning path.
// The test consumer inspects the exact Frame passed to Renderer::render, without
// comparing pixels or bypassing any GUI, model, hand, fog or extraction callbacks.
fn with_frame_scene(
    test: impl FnOnce(&mut super::Game<'_>, &std::sync::Mutex<ClearAt>, &std::sync::atomic::AtomicUsize),
) {
    let at = std::sync::Arc::new(std::sync::Mutex::new(ClearAt::Never));
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "frame_clear",
        Box::new(FrameMod {
            at: at.clone(),
            calls: calls.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _event_loop = frame_scene(game);
            test(game, &at, &calls);
        });
    });
}

/// A hidden 320 by 240 window and the event loop that holds it.
pub(in crate::game) fn hidden_window() -> (
    winit::event_loop::EventLoop<()>,
    std::sync::Arc<winit::window::Window>,
) {
    let mut builder = winit::event_loop::EventLoop::builder();
    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_x11().with_any_thread(true);
    }
    let event_loop = builder.build().unwrap();
    #[allow(deprecated)]
    let window = std::sync::Arc::new(
        event_loop
            .create_window(
                winit::window::Window::default_attributes()
                    .with_visible(false)
                    .with_inner_size(winit::dpi::PhysicalSize::new(320, 240)),
            )
            .unwrap(),
    );
    (event_loop, window)
}

/// A real offscreen renderer and a world for it to draw; the event loop that returns
/// holds the window.
pub(in crate::game) fn frame_scene(game: &mut super::Game<'_>) -> winit::event_loop::EventLoop<()> {
    let (event_loop, window) = hidden_window();
    game.options.offscreen = true;
    {
        let renderer = game.new_renderer(window).unwrap();
        game.set_renderer(renderer);
    }
    game.title = None;
    game.settings.extras.dynamic_lights = true;
    game.settings.sunrise.particle_culling = sunrise_client::graphics::ParticleCullingMode::Off;
    let mut chunk = sunrise_client::chunk::Chunk::empty(0, 0);
    chunk.lit = true;
    game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
    let mut player = sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
    player.position = Vec3::new(0.5, 64.0, 0.5);
    game.previous_position = player.position;
    game.player = Some(player);
    game.particles.camera = [0.5, 65.5, 1.5];
    // Ordinary (non-particle) entity geometry and an unrelated item light.
    game.spawn_item(
        sunrise_client::items::ItemStack::new("minecraft:torch", 1),
        [0.5, 65.0, 4.5],
        [0.0; 3],
        0,
    );
    // Only the picked-up stick resolves this custom property. The torch's
    // ordinary model must survive the clear unchanged.
    std::sync::Arc::get_mut(&mut game.pack.item_renderer)
        .unwrap()
        .models
        .definitions
        .insert(
            "minecraft:stick".into(),
            sunrise_assets::item_model::ClientItem::from_json(
                r#"{"model":{
            "type":"minecraft:condition", "property":"test:clear",
            "on_true":{"type":"minecraft:model","model":"minecraft:item/stick"},
            "on_false":{"type":"minecraft:model","model":"minecraft:item/stick"}
        }}"#,
            )
            .unwrap(),
        );
    event_loop
}

fn add_frame_particles(game: &mut super::Game<'_>) {
    fill(game);
    for _ in 0..20 {
        game.particles.add(
            &ParticleOptions::simple(ParticleType::Flame),
            game.particles.camera,
            [0.0; 3],
        );
    }
    game.spawn_item(
        sunrise_client::items::ItemStack::new("minecraft:stick", 1),
        game.particles.camera,
        [0.0; 3],
        0,
    );
    let item = game.dropped.pop().unwrap();
    game.add_pickup_toward(
        Taken::Item(Box::new(item)),
        Collector::Player,
        game.particles.camera,
    );
    game.tick_particles();
}

#[derive(Debug, PartialEq)]
struct FrameGeometry {
    entities: Vec<u8>,
    glowing: Vec<u8>,
    gpu: (usize, usize, u32),
    motion: Vec<(u32, u64)>,
    spans: Vec<sunrise_client_render::render_geometry::EntitySpan>,
    gpu_spans: Vec<sunrise_client_render::render_geometry::EntitySpan>,
    outlines: (usize, u32),
}
fn frame_geometry(frame: &super::super::Frame<'_>) -> FrameGeometry {
    FrameGeometry {
        entities: bytemuck::cast_slice(frame.entities).to_vec(),
        glowing: bytemuck::cast_slice(frame.glowing).to_vec(),
        gpu: frame.skin.map_or((0, 0, 0), |skin| skin.mark()),
        motion: frame.entity_motion.to_vec(),
        spans: frame.entity_spans.to_vec(),
        gpu_spans: frame.skin.map_or_else(Vec::new, |skin| skin.spans.clone()),
        outlines: frame.skin.map_or((0, 0), |skin| {
            (skin.outline_jobs.len(), skin.outline_vertices)
        }),
    }
}

#[test]
#[ignore = "daily"]
fn fog_clear_reaches_the_complete_frame_consumer() {
    let name =
        "game::native_hooks::particle_count_tests::fog_clear_reaches_the_complete_frame_consumer";
    if super::super::native_entity_refs::tests::run_fresh_cases(
        name,
        "SUNRISE_PARTICLE_FOG_CHILD",
        &["color", "fog"],
    ) {
        return;
    }
    let clear_at = if std::env::var("SUNRISE_PARTICLE_FOG_CHILD").unwrap() == "color" {
        ClearAt::FogColor
    } else {
        ClearAt::Fog
    };
    with_frame_scene(|game, at, calls| {
        let mut baseline = None;
        game.render_with(|_, frame| {
            baseline = Some(frame_geometry(frame));
            Ok(true)
        })
        .unwrap();
        let baseline = baseline.unwrap();
        assert!(
            !baseline.entities.is_empty() || baseline.gpu.2 > 0,
            "ordinary entity must actually draw"
        );
        add_frame_particles(game);
        let mut consumed = false;
        game.render_with(|_, frame| {
            consumed = true;
            assert_ne!(
                frame_geometry(frame),
                baseline,
                "live particles reach the consumer"
            );
            Ok(true)
        })
        .unwrap();
        assert!(consumed);
        *at.lock().unwrap() = clear_at;
        game.render_with(|_, frame| {
            assert_eq!(
                calls.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "fog hook ran in this frame"
            );
            assert_eq!(
                frame_geometry(frame),
                baseline,
                "no stale particles, ordinary entities retained"
            );
            Ok(true)
        })
        .unwrap();
        assert_eq!(game.client_particle_count(), 0);
        game.tick_particles();
        assert_eq!(game.client_particle_count(), 0);
    });
}

#[test]
#[ignore = "daily"]
fn late_clear_removes_native_particle_lights_at_the_frame_consumer() {
    let name = "game::native_hooks::particle_count_tests::late_clear_removes_native_particle_lights_at_the_frame_consumer";
    if super::super::native_entity_refs::tests::run_fresh_cases(
        name,
        "SUNRISE_PARTICLE_LIGHT_CHILD",
        &["gui", "model"],
    ) {
        return;
    }
    let clear_at = if std::env::var("SUNRISE_PARTICLE_LIGHT_CHILD").unwrap() == "gui" {
        ClearAt::Gui
    } else {
        ClearAt::Model
    };
    with_frame_scene(|game, at, calls| {
        let manifest = br#"schema = 1
id = "particle_frame"
name = "Particle Frame"
version = "1.0.0"
api = "0.1"
[material]
file = "material.wgsl"
[[particle_lights]]
particles = ["flame"]
color = [1.0, 0.0, 0.0]
reach = 4.0
[[lights]]
blocks = ["minecraft:torch"]
color = [0.0, 1.0, 0.0]
"#;
        let pack = sunrise_shader_api::Pack::from_files(std::collections::BTreeMap::from([
            (sunrise_shader_api::MANIFEST.into(), manifest.to_vec()),
            (
                "material.wgsl".into(),
                b"fn material(input: MaterialInput) -> Surface { return Surface(input.albedo); }"
                    .to_vec(),
            ),
        ]))
        .unwrap();
        let report = sunrise_shader_api::check(&pack).unwrap();
        game.renderer
            .as_mut()
            .unwrap()
            .set_native_shader_pack(Some((
                report,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Default::default(),
            )))
            .unwrap();
        let mut baseline = None;
        game.render_with(|_, frame| {
            assert_eq!(frame.colored_lights.len(), 2, "item and mod light");
            assert!(
                frame
                    .colored_lights
                    .iter()
                    .any(|l| l[1] == [0.0, 1.0, 0.0, 1.0])
            );
            assert!(
                frame
                    .colored_lights
                    .iter()
                    .any(|l| l[1] == [0.0, 0.0, 1.0, 1.0])
            );
            baseline = Some((frame_geometry(frame), frame.colored_lights.to_vec()));
            Ok(true)
        })
        .unwrap();
        let baseline = baseline.unwrap();
        add_frame_particles(game);
        game.render_with(|_, frame| {
            assert_eq!(
                frame.colored_lights.len(),
                sunrise_client::dynamic_light::MAX
            );
            assert!(
                frame
                    .colored_lights
                    .iter()
                    .all(|l| l[1] == [1.0, 0.0, 0.0, 1.0]),
                "nearby flames displace unrelated lights before clearing"
            );
            Ok(true)
        })
        .unwrap();
        *at.lock().unwrap() = clear_at;
        game.render_with(|_, frame| {
            assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
            assert_eq!(
                frame.colored_lights, baseline.1,
                "cleared flames contribute no light; item/mod lights restored"
            );
            assert_eq!(
                frame_geometry(frame),
                baseline.0,
                "particle tail rolls back, ordinary torch stays"
            );
            Ok(true)
        })
        .unwrap();
        assert_eq!(game.client_particle_count(), 0);
    });
}

fn host_clear(game: &super::Game<'_>) -> bool {
    let host: &dyn ClientHost = game;
    host.clear_particles()
}
