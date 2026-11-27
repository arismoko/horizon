pub(in crate::game) struct ClearInGui;
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

pub(in crate::game) struct ClearInModel(
    pub(in crate::game) std::sync::Arc<std::sync::atomic::AtomicUsize>,
);
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

#[derive(Clone, Copy, PartialEq)]
pub(in crate::game) enum ClearAt {
    Never,
    Gui,
    Model,
    FogColor,
    Fog,
}

pub(in crate::game) struct FrameMod {
    pub(in crate::game) at: std::sync::Arc<std::sync::Mutex<ClearAt>>,
    pub(in crate::game) calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl FrameMod {
    pub(in crate::game) fn clear(&self, at: ClearAt, host: &dyn ClientHost) {
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

// A real renderer supplies the GPU skinning path.
// The test consumer inspects the exact Frame passed to Renderer::render, without
// comparing pixels or bypassing any GUI, model, hand, fog or extraction callbacks.
pub(in crate::game) fn with_frame_scene(
    test: impl FnOnce(
        &mut crate::game::ClientGame<'_>,
        &std::sync::Mutex<ClearAt>,
        &std::sync::atomic::AtomicUsize,
    ),
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

#[derive(Debug, PartialEq)]
pub(in crate::game) struct FrameGeometry {
    pub(in crate::game) entities: Vec<u8>,
    pub(in crate::game) glowing: Vec<u8>,
    pub(in crate::game) gpu: (usize, usize, u32),
    pub(in crate::game) motion: Vec<(u32, u64)>,
    pub(in crate::game) spans: Vec<sunrise_client_render::render_geometry::EntitySpan>,
    pub(in crate::game) gpu_spans: Vec<sunrise_client_render::render_geometry::EntitySpan>,
    pub(in crate::game) outlines: (usize, u32),
}
pub(in crate::game) fn frame_geometry(
    frame: &sunrise_client_render::gfx::Frame<'_>,
) -> FrameGeometry {
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

pub(in crate::game) fn host_clear(game: &crate::game::ClientGame<'_>) -> bool {
    let host: &dyn ClientHost = game;
    host.clear_particles()
}
