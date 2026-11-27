//! A game started without its prepared files (the game build run by hand, not
//! through a launcher): a window that says where to get them
//! (`sunrise_client::no_resources`) and quits when it is closed. No loader, no
//! crash report: nothing went wrong in the game.
use super::*;
use sunrise_client::{
    no_resources::{Action, NoResources},
    window::{GuiSize, PointerProjection, PointerSample, PointerSpace},
};
use winit::event::MouseButton;

struct Missing {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    gui: UiResources,
    screen: NoResources,
    pointer: PointerSample,
    window_pointer_space: std::cell::OnceCell<PointerSpace>,
    api: sunrise_client::graphics::GraphicsApi,
    settings: Settings,
    error: Option<Box<dyn std::error::Error>>,
}

/// Shows the screen until the player closes it.
pub fn show(looked_in: &std::path::Path, settings: &Settings) -> Result<()> {
    // The player's language, from the strings compiled in: there are no prepared ones.
    sunrise_game_data::lang::set(sunrise_game_data::lang::Language::load(
        &[],
        &settings.language,
    ));
    let mut app = Missing {
        window: None,
        renderer: None,
        gui: UiResources {
            atlas: HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
                looked_in.to_path_buf(),
            ))?,
            panorama: Arc::new(sunrise_client::gfx::panorama::Faces::colour([0; 4])),
            splashes: Arc::from(""),
            plain_texts: Default::default(),
        },
        screen: NoResources::new(looked_in, sunrise_hooks::engine::get_the_game()),
        pointer: PointerSample::default(),
        window_pointer_space: std::cell::OnceCell::new(),
        api: settings.sunrise.api,
        settings: settings.clone(),
        error: None,
    };

    EventLoop::new()?.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

impl Missing {
    fn size(&self) -> [u32; 2] {
        self.renderer
            .as_ref()
            .map_or([1280, 720], |r| [r.config.width, r.config.height])
    }

    fn projected_pointer(&self) -> sunrise_client::window::GuiPointer {
        let inputs = self.settings.gui_scale_inputs();
        let size = self.size();
        let window = self.window.as_deref().map_or_else(
            || PointerSpace::new(size, 1.0),
            |window| {
                *self
                    .window_pointer_space
                    .get_or_init(|| PointerSpace::from_window(window))
            },
        );
        PointerProjection {
            gui: GuiSize::new(size, inputs.requested, inputs.unicode),
            window,
        }
        .project_pointer(self.pointer)
    }

    /// `f` over a Ui of the window's size, the cursor where the pointer is.
    fn with_ui<R>(
        &self,
        batch: &mut HudBatch,
        cursor: (f32, f32),
        f: impl FnOnce(&mut sunrise_client::ui_theme::Ui) -> R,
    ) -> R {
        let mut ui = sunrise_client::ui_theme::Ui::new(&self.gui.atlas, batch, cursor);
        let result = f(&mut ui);
        ui.finish();
        result
    }

    fn draw(&mut self) {
        let mut batch = HudBatch::new(self.size(), self.settings.gui_scale_inputs());
        self.with_ui(&mut batch, self.projected_pointer().pixel(), |ui| {
            self.screen.draw(ui)
        });
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        match renderer.render(&loading_frame(&batch), None) {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => log::warn!("Sunrise: frame failed: {e}"),
        }
        // The frame was not shown (the first one often isn't): the glyphs it
        // rasterized never reached the atlas, so draw them again next frame.
        sunrise_client::ui_theme::reset_atlas();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn click(&mut self, event_loop: &ActiveEventLoop) {
        let mut batch = HudBatch::new(self.size(), self.settings.gui_scale_inputs());
        match self.with_ui(&mut batch, self.projected_pointer().subpixel(), |ui| {
            self.screen.click(ui)
        }) {
            Some(Action::Open(address)) => packs::open_uri(address),
            Some(Action::Quit) => event_loop.exit(),
            None => {}
        }
    }
}

impl ApplicationHandler for Missing {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = sunrise_client::window::with_icon(
            Window::default_attributes().with_title(super::window_title::base()),
        )
        .with_inner_size(winit::dpi::LogicalSize::new(1040.0, 700.0));
        let made = event_loop
            .create_window(attributes)
            .map_err(Into::into)
            .and_then(|window| {
                let window = Arc::new(window);
                let renderer = loading_renderer(
                    window.clone(),
                    false,
                    Some(&self.gui.atlas),
                    Some(&self.gui.panorama),
                    None,
                    self.api,
                )?;
                Ok((window, renderer))
            });
        match made {
            Ok((window, renderer)) => {
                window.request_redraw();
                self.window = Some(window);
                self.renderer = Some(renderer);
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone().filter(|w| w.id() == id) else {
            return;
        };
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            self.window_pointer_space.take();
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = self
                    .window_pointer_space
                    .get_or_init(|| PointerSpace::from_window(&window))
                    .os_sample(position);
                window.request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => self.click(event_loop),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::Escape) =>
            {
                event_loop.exit()
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if sunrise_threads::stop_requests() > 0 {
            event_loop.exit();
            return;
        }
        // Wake now and then for Ctrl+C in the console; redraws come with events.
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(250),
        ));
    }
}
