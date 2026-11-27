//! The interactive title before world resources exist. Uses only the menu
//! atlas/panorama; transfers the actual screen and queued entry to Game.
use super::event_loop::EventLoopCtl;
use super::*;
use crate::game::client_side::key_codes::glfw_modifiers;
use crate::game::client_side::{
    key_focus::{self, Act},
    pack_discovery_client::{data_packs_with_icons, into_resource_pack, selection_with_icon},
};
use sunrise_client::{
    clipboard::{self, AppendSelection, SystemClipboard},
    options_ui::{Event, OptionsUi, Page},
    widgets::{Kind, Target},
    window::{GuiSize, PointerProjection, PointerSample, PointerSpace},
};
use sunrise_gameplay_common::game::{api_values::game_rule_registry, pack_discovery::discover};
use winit::{event::KeyEvent, keyboard::ModifiersState};

#[derive(Default)]
pub struct StartupMenu {
    pub title: Option<Title>,
    pub options: OptionsUi,
    pub options_open: bool,
    /// The credits from Credits & Attribution, and their last frame.
    win: Option<(sunrise_client::win_screen::WinScreen, Instant)>,
    root: PathBuf,
    pub cursor: (f32, f32),
    pub pointer: PointerSample,
    window_pointer_space: std::cell::OnceCell<PointerSpace>,
    modifiers: ModifiersState,
    physical_keys: HashSet<KeyCode>,
    dragging: bool,
    clipboard: SystemClipboard,
    selection: AppendSelection,
    /// Narrator candidate, admitted only once Game can supply a ClientHost.
    pending_narrator_toast: Option<sunrise_client::toasts::SystemToast>,
    /// The screens' narration, handed to the game with the title (which
    /// stays the screen already opened).
    narration: sunrise_client::screen_narrator::ScreenNarrator,
    /// Frames drawn, for `widgets::end_frame`.
    frames: u64,
    /// The keyboard's focus (`key_focus`: Tab, the arrows, Enter); the
    /// pointer sits on it, as in the game.
    key_focus: Option<Target>,
    /// The focused slider's arrows change its value (`canChangeValue`).
    key_slider_edit: bool,
    /// What the last click landed on, for the next Tab or arrow.
    key_origin: Option<[f32; 4]>,
    /// The focused slider being nudged and where the keyboard put its handle.
    key_nudge: Option<([f32; 4], f32)>,
    /// A click the keyboard sends is being handled.
    key_synthetic: bool,
    /// The screen the keyboard's focus is on (`key_screen`).
    key_screen: u64,
    /// The screen (`Self::key_screen`) the title's click focus belongs to:
    /// another one in front drops it, as `Gui.setScreen` drops the focus of
    /// the screen it leaves (`Screen.clearFocus`).
    focus_screen: u64,
    pub pending: Option<PendingWorldEntry>,
    pub reload_packs: bool,
    /// A committed client pack selection, independent of other reload requests.
    pub pack_selection_revision: u64,
    perf: sunrise_client::telemetry::Session,
}

impl StartupMenu {
    pub(in crate::game::client_side) fn new(
        options: &Options,
        pack_error: Option<&str>,
    ) -> Result<Self> {
        let pending = if let Some(address) = &options.server {
            Some(PendingWorldEntry::Server {
                address: address.clone(),
                name: options.player_name(),
                list_name: None,
                cookies: None,
                lan: false,
                realm: false,
            })
        } else {
            options
                .world
                .as_ref()
                .map(|folder| PendingWorldEntry::Local {
                    folder: folder.clone(),
                    name: None,
                    seed: options.seed,
                    creative: options.creative,
                    create: None,
                })
        };
        let mut menu = Self {
            pending,
            ..Default::default()
        };
        menu.root = options.resources.clone();
        if let Some(error) = pack_error {
            menu.options.open_at(false, Page::Packs);
            menu.options.resource_error = Some(error.to_owned());
            menu.options_open = true;
        }
        Ok(menu)
    }

    pub fn set_root(
        &mut self,
        root: PathBuf,
        settings: &Settings,
        additional_data: Arc<[Arc<sunrise_level::datapack::DataPack>]>,
        splashes: &str,
    ) {
        sunrise_game_data::lang::set(sunrise_game_data::lang::Language::load(
            &[&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )],
            &settings.language,
        ));
        self.root = root;
        if self.title.is_none() {
            let mut title = Title::new(
                std::env::current_dir().unwrap_or_default().join("saves"),
                &self.root,
                splashes,
            );
            title.last_server = settings.file.string("lastServer");
            title.skip_multiplayer_warning = settings.file.bool("skipMultiplayerWarning");
            // The mods' parts are installed before this menu opens, so Create New World's
            // Game Rules page lists their rules from the first frame.
            title.set_rule_registry(Arc::new(game_rule_registry()));
            // Gui.addInitialScreens: the accessibility onboarding comes first.
            if settings.file.bool("onboardAccessibility") {
                title.start_onboarding();
            }
            self.title = Some(title);
        }
        if let Some(title) = &mut self.title {
            title.set_additional_data_packs(additional_data);
        }
    }

    pub fn handoff(&mut self, game: &mut crate::game::ClientGame<'_>) {
        let registry = Arc::new(game_rule_registry());
        if let Some(PendingWorldEntry::Local {
            create: Some(options),
            ..
        }) = &mut self.pending
        {
            options.rules.rebind_registry(registry.clone());
        }
        if let Some(title) = &mut self.title {
            title.set_rule_registry(registry);
        }
        // The startup menu's title is the game's from here: its hooks fire now.
        match self.title.take() {
            Some(title) => {
                let title = title.with_user(|| game.options.player_name());
                game.show_title_with_registry(title);
            }
            None if game.client.remote.is_some() => game.client.title.0 = None,
            None => {
                // Never leave a client game with neither a title screen nor a remote session.
                log::warn!(
                    "Sunrise: startup menu handoff had no title; opening a fresh title screen"
                );
                let mut title = game
                    .new_title(std::env::current_dir().unwrap_or_default().join("saves"))
                    .with_user(|| game.options.player_name());
                title.last_server = game.client.settings.file.string("lastServer");
                title.skip_multiplayer_warning =
                    game.client.settings.file.bool("skipMultiplayerWarning");
                game.show_title(title);
            }
        }
        game.client.pending_entry.0 = self.pending.take();
        game.client.perf = std::mem::take(&mut self.perf);
        game.client.cursor = self.cursor;
        if self.key_focus.is_none() {
            game.client.window_cursor = Some(self.pointer);
        }
        game.client.modifiers = self.modifiers;
        game.client.physical_keys = std::mem::take(&mut self.physical_keys);
        game.client.clipboard.0 = std::mem::take(&mut self.clipboard);
        game.client.text_selection = std::mem::take(&mut self.selection);
        if let Some(toast) = self.pending_narrator_toast.take() {
            game.add_system_toast(Some(super::toast_manager::SystemToastId::Narrator), toast);
        }
        game.client.narration = std::mem::take(&mut self.narration);
        // Preserve an existing resource-error screen opened by start_game.
        if self.options_open {
            let resource_error = game.client.chat_ui.options.resource_error.take();
            game.client.chat_ui.options = std::mem::take(&mut self.options);
            game.client.chat_ui.options.resource_error = game
                .client
                .chat_ui
                .options
                .resource_error
                .take()
                .or(resource_error);
            game.set_screen(Screen::Options);
            game.client.options_from_title = true;
            if self.dragging {
                game.client.physical_buttons.insert(MouseButton::Left);
                game.track_drag_button(MouseButton::Left, true);
            }
        }
        if let Some(focus) = self.key_focus.take() {
            game.key_focus_adopt(focus, self.key_slider_edit);
        }
    }

    /// After a frame is drawn: its widgets become the last frame's, and the
    /// open screen's narration reads them (`Gui.update`'s
    /// `handleDelayedNarration`). `gui` is the GUI size.
    pub(in crate::game::client_side) fn frame_drawn(&mut self, gui: (f32, f32)) {
        self.frames += 1;
        sunrise_client::widgets::end_frame(self.frames);
        // The keyboard's focus follows its widget, or starts again on a new
        // screen.
        if let Some(focus) = self.key_focus {
            let targets = key_focus::keyboard_targets(
                key_focus::on_screen(key_focus::drawn_targets(), gui),
                false,
            );
            let screen = self.key_screen();
            self.key_focus = if screen == self.key_screen {
                key_focus::follow(focus, &targets)
            } else {
                // Screen.setInitialFocus: a forward Tab from nothing.
                self.key_screen = screen;
                key_focus::tab_step(&targets, None, true).map(|i| targets[i])
            };
            if let Some(now) = self.key_focus
                && now.rect != focus.rect
            {
                self.cursor = now.center();
                self.key_nudge = None;
            }
        }
        self.narration
            .frame(gui, self.cursor, self.key_focus.map(|t| t.rect));
    }

    /// Which screen is open, as far as the keyboard's focus cares.
    fn key_screen(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (self.options_open, self.win.is_some()).hash(&mut hasher);
        std::mem::discriminant(&self.options.page()).hash(&mut hasher);
        self.title.as_ref().map(Title::screen_key).hash(&mut hasher);
        hasher.finish()
    }

    /// The keyboard lets go of the focus (the real mouse is used): it stays
    /// where the next Tab or arrow goes on from. Whether it had it.
    fn key_focus_release(&mut self) -> bool {
        let Some(focus) = self.key_focus.take() else {
            return false;
        };
        self.key_origin = Some(focus.rect);
        self.key_nudge = None;
        true
    }

    pub fn queue_entry(&mut self, action: Action, player_name: String) {
        let entry = match action {
            Action::Play { folder } => PendingWorldEntry::Local {
                folder,
                name: None,
                seed: None,
                creative: false,
                create: None,
            },
            Action::Create {
                folder,
                name,
                seed,
                creative,
                options,
            } => PendingWorldEntry::Local {
                folder,
                name: Some(name),
                seed,
                creative,
                create: Some(Box::new(options)),
            },
            Action::Connect {
                address,
                name,
                lan,
                realm,
            } => PendingWorldEntry::Server {
                address,
                name: player_name,
                list_name: name,
                cookies: None,
                lan,
                realm,
            },
            _ => return,
        };
        self.pending = Some(entry);
        self.perf.world_opening();
    }

    fn shortcut(&mut self, event: &KeyEvent) -> Option<clipboard::EditShortcut> {
        let edit = if cfg!(target_os = "macos") {
            self.modifiers.super_key()
        } else {
            self.modifiers.control_key()
        };
        clipboard::edit_shortcut(
            clipboard::shortcut_letter(&event.logical_key, event.physical_key),
            edit,
            self.modifiers.shift_key(),
            self.modifiers.alt_key(),
        )
    }

    /// `Screen.keyPressed` on the title screens: the action a key starts (Enter confirms, see `confirm_key` in title.rs).
    fn title_key(&mut self, event: &KeyEvent, viewport: (f32, f32)) -> Action {
        let shortcut = self.shortcut(event);
        let Some(title) = &mut self.title else {
            return Action::None;
        };
        if let Some(shortcut) = shortcut {
            if title.has_caret_field() {
                title.field_shortcut(shortcut, &mut self.clipboard);
            } else if let Some((id, value, max)) = title.focused_field()
                && self
                    .selection
                    .apply(id, value, shortcut, &mut self.clipboard, |v, t| {
                        clipboard::append_limited(v, t, max)
                    })
            {
                title.search_changed();
            }
            return Action::None;
        }
        // `TabNavigationBar.keyPressed` comes before the open tab's own
        // widgets: Ctrl plus a digit or Tab picks a tab instead of reaching
        // them.
        let ctrl = if cfg!(target_os = "macos") {
            self.modifiers.super_key()
        } else {
            self.modifiers.control_key()
        };
        if let PhysicalKey::Code(key) = event.physical_key
            && title
                .tab_bar_key(key, ctrl, self.modifiers.shift_key())
                .is_some()
        {
            return Action::None;
        }
        match event.physical_key {
            PhysicalKey::Code(KeyCode::Escape) => title.escape(),
            PhysicalKey::Code(KeyCode::F5) => title.refresh_servers(),
            PhysicalKey::Code(key) if title.page_missing_mods(key, viewport) => {}
            PhysicalKey::Code(KeyCode::Tab) => {
                title.tab_dir(self.modifiers.shift_key(), viewport);
            }
            PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter) => {
                return title.confirm_key_size(viewport).0;
            }
            PhysicalKey::Code(KeyCode::Space)
                if !title.has_caret_field()
                    && (title.list_focused() || title.focused_button.is_some()) =>
            {
                return title.confirm_key_size(viewport).0;
            }
            PhysicalKey::Code(
                key @ (KeyCode::Backspace
                | KeyCode::Delete
                | KeyCode::ArrowLeft
                | KeyCode::ArrowRight
                | KeyCode::Home
                | KeyCode::End),
            ) if title.has_caret_field() => {
                title.field_key(
                    key,
                    self.modifiers.control_key(),
                    self.modifiers.shift_key(),
                );
            }
            PhysicalKey::Code(
                key @ (KeyCode::ArrowLeft
                | KeyCode::ArrowRight
                | KeyCode::ArrowUp
                | KeyCode::ArrowDown),
            ) => {
                title.arrow_key(key, viewport, self.modifiers.shift_key());
            }
            _ => {
                let cleared = !title.has_caret_field()
                    && title
                        .focused_field()
                        .is_some_and(|(id, value, _)| self.selection.take(id, value));
                if cleared {
                    title.search_changed();
                }
                if event.physical_key == PhysicalKey::Code(KeyCode::Backspace) {
                    if !cleared {
                        title.backspace();
                    }
                } else if let Some(text) = &event.text {
                    title.type_text(text);
                }
            }
        }
        Action::None
    }
}

/// A panic's message: its `&str` or `String` payload.
fn panic_text(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("Box<dyn Any>")
}

/// Each generation has its own channel and progress. Pack changes wait for the
/// old tail before starting another generation, including cache/global writes.
pub(in crate::game::client_side) fn spawn_loader(
    options: &Options,
    settings: &Settings,
    pack_selection_revision: u64,
    load_role: mods::LoadRole,
    vanilla_root: &std::path::Path,
    started: Instant,
    progress: &Progress,
    max_atlas_size: &Arc<AtomicU32>,
) -> Result<(Receiver<LoaderMsg>, JoinHandle<()>)> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let settings = settings.clone();
    let vanilla_root = vanilla_root.to_path_buf();
    let mut mod_sources = options.mods.clone();
    mod_sources.capture_code_switches(&settings.mods.disabled);
    let world = launch_world(options).map(str::to_owned);
    let progress = progress.clone();
    let max_atlas_size = max_atlas_size.clone();
    let handle = std::thread::Builder::new()
        .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
        .name("resource-loader".into())
        .spawn(move || {
            let result =
                packs::startup_root(&vanilla_root, &settings.resource_packs, &[], &mod_sources)
                    .and_then(|roots| {
                        load_resources_for_mods_with_limit(
                            roots,
                            data_packs_with_icons(&mod_sources),
                            world.as_deref(),
                            &vanilla_root,
                            &settings,
                            started,
                            true,
                            load_role,
                            &progress,
                            &max_atlas_size,
                            &mut |gui, root, source| {
                                let _ = sender.send(LoaderMsg::Ui(
                                    gui,
                                    root.to_path_buf(),
                                    source.clone(),
                                ));
                            },
                            &mut || {
                                // The menu owns live settings; do not save the worker's snapshot.
                                let (ack, wait) = std::sync::mpsc::channel();
                                sender
                                    .send(LoaderMsg::ClearPackSelection {
                                        revision: pack_selection_revision,
                                        ack,
                                    })
                                    .map_err(|_| "loading screen closed during pack recovery")?;
                                wait.recv()?;
                                Ok(())
                            },
                            &mod_sources,
                        )
                    });
            let _ = sender.send(LoaderMsg::done(result));
        })?;
    Ok((receiver, handle))
}

impl Loading {
    pub fn poll_menu_resources(&mut self) -> Result<()> {
        // Once `Done` arrived the generation is over: the caller may have taken
        // its result, and the finished loader's channel is closed.
        if self.loaded.is_none() && !self.delivered {
            loop {
                match self.receiver.try_recv() {
                    Ok(LoaderMsg::Ui(gui, root, source)) => self.set_ui(gui, root, source),
                    Ok(LoaderMsg::ClearPackSelection { revision, ack }) => {
                        if revision == self.menu.pack_selection_revision {
                            packs::clear_selection(&mut self.settings);
                            self.menu.options.set_resource_pack_selection(&[]);
                        }
                        // Let even a superseded worker finish so the queued load can start.
                        let _ = ack.send(());
                    }
                    Ok(LoaderMsg::Done(result)) => {
                        self.loaded = Some(result);
                        self.delivered = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        // The loader sends `Done` before it returns: it panicked.
                        let reason = match self.handle.take().map(JoinHandle::join) {
                            Some(Err(payload)) => panic_text(payload.as_ref()).to_owned(),
                            Some(Ok(())) => "it returned without a result".into(),
                            None => "no loader thread was running".into(),
                        };
                        return Err(
                            format!("resource loader stopped before finishing: {reason}").into(),
                        );
                    }
                }
            }
        }
        self.restart_menu_resources()
    }
    pub fn apply_menu_settings(&mut self) {
        if let Some(renderer) = &mut self.renderer {
            renderer.gamma = self.settings.gamma as f32;
            renderer.set_present_mode(self.settings.vsync, self.settings.sunrise.present_mode);
        }

        super::narrator::sync(&self.settings);
        if let Some(gui) = &mut self.gui {
            gui.atlas.font.uniform = self.settings.force_unicode;
            gui.atlas.font.japanese = self.settings.japanese_glyphs;
        }
        if let Some(window) = self.window.clone() {
            let monitor = window
                .current_monitor()
                .or_else(|| window.primary_monitor());
            if let Some(modes) = self
                .video_modes
                .refresh(monitor, sunrise_client::window::monitor_video_modes)
            {
                self.settings.video_modes = modes;
            }
            let preferred = self
                .settings
                .file
                .fullscreen_resolution
                .as_deref()
                .and_then(sunrise_client::window::VideoMode::read);
            self.fullscreen.apply(
                &window,
                self.settings.fullscreen,
                self.settings.file.bool("exclusiveFullscreen"),
                preferred,
                false,
            );
        }
        self.update_menu_cursor();
    }

    pub fn menu_pointer_projection(&self) -> PointerProjection {
        let inputs = self.settings.gui_scale_inputs();
        let size = self.menu_size();
        let window = self.window.as_deref().map_or_else(
            || PointerSpace::new(size, 1.0),
            |window| {
                *self
                    .menu
                    .window_pointer_space
                    .get_or_init(|| PointerSpace::from_window(window))
            },
        );
        PointerProjection {
            gui: GuiSize::new(size, inputs.requested, inputs.unicode),
            window,
        }
    }

    fn projected_menu_pointer(&self) -> sunrise_client::window::GuiPointer {
        self.menu_pointer_projection()
            .project_pointer(self.menu.pointer)
    }

    fn update_menu_cursor(&mut self) {
        self.menu.cursor = self.projected_menu_pointer().subpixel();
    }

    pub fn menu_size(&self) -> [u32; 2] {
        self.renderer
            .as_ref()
            .map_or([1280, 720], |r| [r.config.width, r.config.height])
    }

    fn save_menu_address(&mut self) {
        if self.menu.title.as_ref().is_some_and(|title| {
            title.last_server != self.settings.file.string("lastServer")
                || title.skip_multiplayer_warning
                    != self.settings.file.bool("skipMultiplayerWarning")
        }) {
            self.save_menu_settings();
        }
    }

    fn save_menu_settings(&mut self) {
        if let Some(title) = &self.menu.title {
            self.settings.file.set(
                "lastServer",
                sunrise_client::options_file::V::Str(title.last_server.clone()),
            );
            self.settings.file.set(
                "skipMultiplayerWarning",
                sunrise_client::options_file::V::Bool(title.skip_multiplayer_warning),
            );
        }
        self.settings
            .save(&sunrise_gameplay_common::game::options_path());
    }

    pub fn open_menu_options(&mut self, page: Page) {
        self.menu.options.open_at(false, page);
        self.menu.options.languages = sunrise_game_data::lang::available(&[
            &sunrise_assets::resource_source::ResourceSource::directory(self.menu.root.clone()),
        ]);
        let additional = self.options.as_ref().map_or_else(Vec::new, |options| {
            discover(mod_api::PackType::ClientResources, &options.mods)
                .into_iter()
                .map(into_resource_pack)
                .collect()
        });
        self.menu.options.pack_infos = packs::pack_infos(&self.vanilla_root, &[], &additional);
        self.menu
            .options
            .set_resource_pack_selection(&self.settings.resource_packs);
        self.menu.options_open = true;
    }

    pub fn menu_option_events(&mut self, events: Vec<Event>) {
        for event in events {
            match event {
                // No world debug list exists before Game construction.
                Event::DebugEntriesChanged => {}
                Event::Changed => {
                    self.save_menu_settings();
                    self.settings.keys.sync_action_bindings();
                    self.apply_menu_settings();
                }
                Event::SliderDrag => {
                    self.apply_menu_settings();
                }
                Event::Close if self.menu.options.resource_error.is_some() => {}
                Event::Close => {
                    self.menu.options_open = false;
                    self.menu.dragging = false;
                    self.save_menu_settings();
                }
                Event::Language(code) => {
                    sunrise_game_data::lang::set(sunrise_game_data::lang::Language::load(
                        &[&sunrise_assets::resource_source::ResourceSource::directory(
                            self.menu.root.clone(),
                        )],
                        &code,
                    ));
                    self.save_menu_settings();
                }
                Event::Packs(_) => {
                    self.menu.pack_selection_revision =
                        self.menu.pack_selection_revision.wrapping_add(1);
                    self.save_menu_settings();
                    self.menu.reload_packs = true;
                }
                Event::ReloadTextures => {
                    self.save_menu_settings();
                    self.menu.reload_packs = true;
                }
                Event::PackOptions { entries, chosen } => {
                    packs::save_pack_options(&entries, chosen);
                    self.menu.options.pack_configs = packs::pack_configs(&self.vanilla_root);
                    self.menu.reload_packs = true;
                }
                Event::OpenPackFolder => {
                    let dir = packs::packs_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    packs::open_folder(&dir);
                }
                Event::ImportPacks(paths) => {
                    let additional = self.options.as_ref().map_or_else(Vec::new, |options| {
                        discover(mod_api::PackType::ClientResources, &options.mods)
                            .into_iter()
                            .map(into_resource_pack)
                            .collect()
                    });
                    packs::import_resource_packs(
                        &mut self.menu.options,
                        &self.vanilla_root,
                        &[],
                        &paths,
                        &additional,
                    );
                }
                Event::DownloadPack(i) => {
                    let dir = packs::packs_dir();
                    if let Some(screen) = &mut self.menu.options.pack_screen
                        && let Some(browser) = &mut screen.browser
                    {
                        browser.download(i, &dir, sunrise_core::VANILLA_VERSION);
                    }
                }
                Event::DataPacks(ids) => {
                    let resources = self.vanilla_root.clone();
                    if let Some(title) = &mut self.menu.title {
                        title.offer_data_packs(&resources, ids);
                    }
                }
                Event::OpenDataPackFolder(dir) => {
                    let _ = std::fs::create_dir_all(&dir);
                    packs::open_folder(&dir);
                }
                Event::OpenModFolder => {
                    let dir = sunrise_gameplay_common::game::game_dir()
                        .join(sunrise_mod_package::mod_list::DIR);
                    let _ = std::fs::create_dir_all(&dir);
                    packs::open_folder(&dir);
                }
                // The startup menu has no screen stack for ConfirmLinkScreen; the link opens
                // directly, as the menu's other links do.
                Event::OpenLink(url) => packs::open_uri(&url),
                Event::CopyText(text) => {
                    use sunrise_client::clipboard::Clipboard;
                    self.menu.clipboard.set(&text);
                }
                Event::ListShaderPacks => {
                    self.settings.shader_packs = sunrise_client::shader_pack_list::scan(
                        &crate::game::ClientGame::shader_packs_dir(),
                    );
                }
                Event::ShaderPack => self.save_menu_settings(),
                Event::OpenShaderOptions => {
                    self.settings.shader_options = super::shader_packs::read_shader_options(
                        &self.settings.sunrise.shader_pack,
                        &self.settings.language,
                        &sunrise_shaders::Macros::standard(),
                    );
                }
                Event::ShaderOptions => {
                    if let Some(options) = self.settings.shader_options.take() {
                        super::shader_packs::write_shader_options(&options);
                    }
                }
                Event::OpenShaderPackFolder => {
                    let dir = crate::game::ClientGame::shader_packs_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    packs::open_folder(&dir);
                }
                Event::Credits => {
                    if let Some(gui) = &self.gui {
                        let name = self.options.as_ref().map_or_else(
                            || sunrise_sim::offline_name::generate(0),
                            Options::player_name,
                        );
                        let screen = sunrise_client::win_screen::WinScreen::new(
                            false,
                            &sunrise_assets::resource_source::ResourceSource::directory(
                                self.menu.root.clone(),
                            ),
                            &name,
                            &gui.atlas.font,
                        );
                        self.menu.win = Some((screen, Instant::now()));
                    }
                }
                Event::Narrator => self.menu_narrator_status_changed(),
                // The startup menu has no screen stack for ConfirmLinkScreen;
                // the link opens directly, as the menu's other links do.
                Event::AccessibilityHelp => {
                    super::packs::open_uri(super::options::ACCESSIBILITY_HELP_LINK)
                }
                // Mods' settings pages and config screens open only in a running game.
                // The HUD editor needs a world: its button is inactive here.
                Event::NativeSettings(_)
                | Event::NativeSetting(..)
                | Event::ModConfigScreen(_)
                | Event::HudEditor => {}
                // The startup menu runs before the game opens a control socket,
                // and before its sound engine (no clicks, no volume previews,
                // and no music tracks to list or play).
                Event::ButtonClick
                | Event::CopyAgentDetails
                | Event::ToggleRecording
                | Event::ExportRepro
                | Event::ListMusicTracks
                | Event::PlayMusicTrack(..)
                | Event::Volume(..) => {}
                Event::OpenStudioFolder => {
                    let dir = std::path::PathBuf::from("recordings");
                    let _ = std::fs::create_dir_all(&dir);
                    packs::open_folder(&dir);
                }
                Event::OpenStudioReplay(path) => {
                    self.menu.options_open = false;
                    self.menu.dragging = false;
                    self.menu.pending = Some(PendingWorldEntry::StudioReplay(path));
                }
            }
        }
    }

    fn menu_narrator_status_changed(&mut self) {
        if let Some(toast) = super::narrator::update_status(
            self.settings.narrator(),
            self.menu.pending_narrator_toast.as_mut(),
        ) {
            self.menu.pending_narrator_toast = Some(toast);
        }
    }

    /// `Game::narrator_hotkey` on the startup menu's screens.
    fn menu_narrator_hotkey(&mut self, event: &KeyEvent) {
        let letter = clipboard::shortcut_letter(&event.logical_key, event.physical_key);
        if letter != Some('b') {
            return;
        }
        let menu = &mut self.menu;
        let control_with_quirk = if cfg!(target_os = "macos") {
            menu.modifiers.super_key()
        } else {
            menu.modifiers.control_key()
        };
        let edit_box_focused = if menu.options_open {
            menu.options.typing()
        } else if menu.win.is_some() || menu.pending.is_some() {
            false
        } else {
            menu.title
                .as_mut()
                .is_some_and(|t| t.has_caret_field() || t.focused_field().is_some())
        };
        let hotkey = super::narrator::is_hotkey(
            event.state == ElementState::Pressed,
            control_with_quirk,
            letter,
            edit_box_focused,
            sunrise_narrator::is_active(),
            self.settings.file.bool("narratorHotkey"),
        );
        if !hotkey {
            return;
        }
        let was_disabled = super::narrator::cycle_status(&mut self.settings);
        self.menu.options.layout_changed();
        self.menu_narrator_status_changed();
        self.save_menu_settings();
        self.menu
            .narration
            .status_changed(was_disabled, sunrise_narrator::screen::Trigger::Keyboard);
    }

    fn menu_options_click(&mut self, drag: bool, right: bool) {
        let size = self.menu_size();
        let batch = HudBatch::new(size, self.settings.gui_scale_inputs());
        let Some(gui) = &self.gui else { return };
        self.menu.options.shift = self.menu.modifiers.shift_key();
        let events = self.menu.options.click(
            &mut self.settings,
            &gui.atlas,
            self.menu.cursor.0,
            self.menu.cursor.1,
            (batch.width, batch.height),
            [size[0] as i32, size[1] as i32],
            drag,
            right,
        );
        self.menu_option_events(events);
    }

    /// The Realms screen's join, once its address came back: queued like a click on a server.
    pub(in crate::game::client_side) fn poll_menu_realms(&mut self, event_loop: &dyn EventLoopCtl) {
        if let Some(action) = self
            .menu
            .title
            .as_mut()
            .and_then(Title::take_connect_request)
        {
            self.menu_action(event_loop, action);
        }
    }

    /// The title's installed-package count: the buttons crates add to the title screen
    /// (`sunrise_hooks::title`) show by it, with the same eligibility the game's own title
    /// screen uses. Before the menu draws or takes input.
    fn sync_title_mods(&mut self) {
        if let Some(title) = &mut self.menu.title {
            title.mods = self.settings.mods.installed.len();
        }
    }

    fn menu_action(&mut self, event_loop: &dyn EventLoopCtl, action: Action) {
        match action {
            Action::Quit => event_loop.exit(),
            Action::Options => self.open_menu_options(Page::Main),
            Action::Accessibility => self.open_menu_options(Page::Accessibility),
            Action::Language => self.open_menu_options(Page::Language),
            Action::Sounds => self.open_menu_options(Page::Sound),
            Action::Credits => self.open_menu_options(Page::Credits),
            Action::Mods => {
                list_menu_mods(&mut self.settings.mods);
                self.open_menu_options(Page::Mods);
            }
            Action::OpenUri(uri) => super::packs::open_uri(&uri),
            Action::CopyText(text) => {
                use sunrise_client::clipboard::Clipboard;
                self.menu.clipboard.set(&text);
            }
            Action::DataPacks { dir, selected } => {
                let resources = self.vanilla_root.clone();
                let additional = self.options.as_ref().map_or_else(Vec::new, |options| {
                    discover(mod_api::PackType::ServerData, &options.mods)
                        .into_iter()
                        .filter_map(|pack| selection_with_icon(&pack.meta))
                        .collect()
                });
                self.menu
                    .options
                    .open_data_packs(dir, selected, resources, additional);
                self.menu.options_open = true;
            }
            action => {
                let name = self
                    .options
                    .as_ref()
                    .map_or_else(|| "Player".into(), |o| o.player_name());
                self.menu.queue_entry(action, name);
            }
        }
        self.save_menu_address();
    }

    /// The real mouse before the screen takes it: a move, a press or a
    /// scroll hands the pointer back from the keyboard (a press or a scroll
    /// where the mouse left it), and a left press focuses what it lands on.
    fn menu_key_focus_mouse(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::CursorMoved { .. } => {
                self.menu.key_focus_release();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if self.menu.key_focus_release() {
                    self.update_menu_cursor();
                }
                if *state == ElementState::Pressed && *button == MouseButton::Left {
                    let (cx, cy) = self.menu.cursor;
                    self.menu.key_origin = self
                        .menu_key_targets()
                        .into_iter()
                        .find(|t| {
                            let [x, y, w, h] = t.rect;
                            cx >= x && cy >= y && cx < x + w && cy < y + h
                        })
                        .map(|t| t.rect);
                }
            }
            WindowEvent::MouseWheel { .. } if self.menu.key_focus_release() => {
                self.update_menu_cursor();
            }
            _ => {}
        }
    }

    /// What the keyboard can focus on the screen last drawn.
    fn menu_key_targets(&self) -> Vec<Target> {
        let batch = HudBatch::new(self.menu_size(), self.settings.gui_scale_inputs());
        key_focus::keyboard_targets(
            key_focus::on_screen(key_focus::drawn_targets(), (batch.width, batch.height)),
            false,
        )
    }

    /// `Screen.keyPressed`'s focus navigation on the title shown while
    /// resources load, as `Game::key_focus_key` does it in the game (the
    /// pointer sits on the focus; a step past a list's visible rows doesn't
    /// scroll it here). Whether the key was used.
    fn menu_key_focus(&mut self, event_loop: &dyn EventLoopCtl, key: KeyCode) -> bool {
        // This safety prompt owns its default and spatial navigation in Title.
        if !self.menu.options_open
            && self
                .menu
                .title
                .as_ref()
                .is_some_and(|t| matches!(t.mode, sunrise_client::title::Mode::MissingMods { .. }))
        {
            return false;
        }
        let Some(nav) = key_focus::nav_of(key, self.menu.modifiers.shift_key()) else {
            return false;
        };
        let options = &self.menu.options;
        if self.gui.is_none()
            || self.menu.pending.is_some()
            || self.menu.win.is_some()
            || (self.menu.options_open
                && (options.selected_key.is_some() || options.selected_pad_action.is_some()))
        {
            return false;
        }
        let targets = self.menu_key_targets();
        if targets.is_empty() {
            return false;
        }
        let find = |rect: [f32; 4]| {
            targets
                .iter()
                .find(|t| t.rect.iter().zip(rect).all(|(a, b)| (a - b).abs() < 0.5))
                .copied()
        };
        let mut slider_edit = self.menu.key_slider_edit;
        let mut focus = self.menu.key_focus.and_then(|f| find(f.rect));
        if focus.is_none()
            && let Some(clicked) = self.menu.key_origin.and_then(find)
        {
            focus = Some(clicked);
            slider_edit = true;
        }
        let (typing, rows_take_enter) = if self.menu.options_open {
            (self.menu.options.typing(), false)
        } else if let Some(title) = &mut self.menu.title {
            (
                title.has_caret_field() || title.focused_field().is_some(),
                title.list_focused(),
            )
        } else {
            (false, false)
        };
        let batch = HudBatch::new(self.menu_size(), self.settings.gui_scale_inputs());
        let ctx = key_focus::Context {
            slider_edit,
            typing,
            rows_take_enter,
            gui: (batch.width, batch.height),
        };
        let act = key_focus::act(nav, focus, &targets, ctx);
        if act == Act::Pass {
            return false;
        }
        if self.menu.key_focus.is_none() {
            self.menu.key_screen = self.menu.key_screen();
        }
        self.menu.key_origin = None;
        self.menu.key_focus = focus;
        self.menu.key_slider_edit = slider_edit;
        match (act, focus) {
            (Act::Focus(i), _) => {
                self.menu_key_focus_on(event_loop, targets[i]);
                self.menu.key_slider_edit = key_focus::slider_edit_on_focus(nav);
            }
            (Act::Step(button), Some(focus)) => {
                let rects: Vec<[f32; 4]> = targets.iter().map(|t| t.rect).collect();
                if let Some(i) =
                    sunrise_client::gamepad::focus_step_rect(focus.rect, button, &rects)
                {
                    self.menu_key_focus_on(event_loop, targets[i]);
                }
            }
            (Act::Press, Some(focus)) => self.menu_key_click(event_loop, focus.center()),
            (Act::ToggleSlider, _) => self.menu.key_slider_edit = !slider_edit,
            (
                Act::Nudge(sign),
                Some(
                    focus @ Target {
                        kind: Kind::Slider { value, track },
                        ..
                    },
                ),
            ) => {
                let from = match self.menu.key_nudge {
                    Some((rect, x)) if rect == focus.rect => x,
                    _ => sunrise_client::gamepad::slider_x(track, value),
                };
                let x = (from + sign).clamp(track[0], track[0] + track[1]);
                self.menu.key_nudge = Some((focus.rect, x));
                self.menu_key_click(event_loop, (x, focus.center().1));
            }
            _ => {}
        }
        if let Some(Target {
            kind: Kind::Slider { .. },
            ..
        }) = self.menu.key_focus
        {
            use sunrise_client::screen_narrator::InputType;
            self.menu.narration.set_input(if self.menu.key_slider_edit {
                InputType::KeyboardTab
            } else {
                InputType::KeyboardArrow
            });
        }
        true
    }

    /// The keyboard's focus lands on `target`, the pointer with it (see
    /// `Game::key_focus_landed`).
    fn menu_key_focus_on(&mut self, event_loop: &dyn EventLoopCtl, target: Target) {
        let left = self.menu.key_focus.replace(target);
        self.menu.key_nudge = None;
        self.menu.key_slider_edit = false;
        self.menu.cursor = target.center();
        if left.is_some_and(|l| l.rect == target.rect) {
            return;
        }
        if left.is_some_and(|l| l.kind == Kind::Field)
            && target.kind != Kind::Field
            && !self.menu.options_open
            && let Some(title) = &mut self.menu.title
        {
            title.blur_field();
        }
        match target.kind {
            Kind::Field => self.menu_key_click(event_loop, key_focus::field_click(target.rect)),
            Kind::Row if !self.menu.options_open => {
                let batch = HudBatch::new(self.menu_size(), self.settings.gui_scale_inputs());
                if let Some(title) = &mut self.menu.title {
                    title.focus_world_row(target.center(), (batch.width, batch.height));
                }
            }
            _ => {}
        }
    }

    /// A left click at `at` (GUI pixels), the pointer staying there.
    fn menu_key_click(&mut self, event_loop: &dyn EventLoopCtl, at: (f32, f32)) {
        self.menu.cursor = at;
        let was = std::mem::replace(&mut self.menu.key_synthetic, true);
        for state in [ElementState::Pressed, ElementState::Released] {
            let click = WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button: MouseButton::Left,
            };
            if let Err(e) = self.menu_event(event_loop, click) {
                log::warn!("Sunrise: keyboard focus click: {e}");
            }
        }
        self.menu.key_synthetic = was;
    }

    pub fn menu_event(&mut self, event_loop: &dyn EventLoopCtl, event: WindowEvent) -> Result<()> {
        // The buttons crates add to the title screen show by what is installed now
        // (`sunrise_hooks::title`), as on the game's own title screen.
        self.sync_title_mods();
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && let PhysicalKey::Code(key) = event.physical_key
        {
            if event.state == ElementState::Pressed {
                self.menu.physical_keys.insert(key);
            } else {
                self.menu.physical_keys.remove(&key);
            }
        }
        if let WindowEvent::KeyboardInput { event, .. } = &event {
            self.menu_narrator_hotkey(event);
        }
        // The real mouse takes the pointer back from the keyboard's focus.
        let synthetic = self.menu.key_synthetic;
        if !synthetic {
            self.menu_key_focus_mouse(&event);
        }
        // Fullscreen (F11) is the window's: not the screen's input; the
        // keyboard's own clicks aren't the mouse.
        let f11 = matches!(&event, WindowEvent::KeyboardInput { event, .. }
            if event.physical_key == PhysicalKey::Code(KeyCode::F11));
        if !f11 && !synthetic {
            super::narrator::narrate_input(&mut self.menu.narration, &event, self.menu.cursor);
        }
        // Screen.keyPressed: Tab, the arrows and Enter move and press the
        // focus, unless the focused widget or the screen keeps them.
        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = &event
            && key_event.state == ElementState::Pressed
            && let PhysicalKey::Code(key) = key_event.physical_key
            && self.menu_key_focus(event_loop, key)
        {
            return Ok(());
        }
        match event {
            WindowEvent::CloseRequested => {
                if !self.settings.quit_shortcuts
                    && sunrise_client::input::is_quit_shortcut_down(
                        self.menu.modifiers,
                        &self.menu.physical_keys,
                    )
                {
                    return Ok(());
                }
                self.save_menu_settings();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.menu.window_pointer_space.take();
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
                self.apply_menu_settings();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                self.menu.window_pointer_space.take();
                self.update_menu_cursor();
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::ModifiersChanged(modifiers) => self.menu.modifiers = modifiers.state(),
            // `PackSelectionScreen.onFilesDrop`: a pack dropped on the window
            // joins the Create New World screen's data packs.
            WindowEvent::DroppedFile(path) => {
                let batch = HudBatch::new(self.menu_size(), self.settings.gui_scale_inputs());
                let handled = self.menu.options_open
                    && self.gui.as_ref().is_some_and(|gui| {
                        self.menu.options.drop_resource_pack(
                            path.clone(),
                            &gui.atlas,
                            (batch.width, batch.height),
                        )
                    });
                if !handled && let Some(title) = &mut self.menu.title {
                    title.drop_file(path);
                }
            }
            WindowEvent::Focused(false) => {
                self.menu.modifiers = ModifiersState::empty();
                self.menu.physical_keys.clear();
                self.menu.dragging = false;
                if let Some(title) = &mut self.menu.title {
                    title.release();
                    title.release_field();
                }
                let events = self.menu.options.release();
                self.menu_option_events(events);
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && self.menu.pending.is_some() =>
            {
                if event.physical_key == PhysicalKey::Code(KeyCode::Escape) {
                    self.menu.pending = None;
                }
            }
            _ if self.gui.is_none() || self.menu.pending.is_some() => {}
            WindowEvent::KeyboardInput { event, .. } if self.menu.win.is_some() => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let pressed = event.state == ElementState::Pressed;
                    if code == KeyCode::Escape && pressed {
                        self.menu.win = None;
                    } else if let Some(key) = sunrise_client::win_screen::Key::from_code(code)
                        && let Some((win, _)) = &mut self.menu.win
                    {
                        win.key(key, pressed);
                    }
                }
            }
            WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. }
                if self.menu.win.is_some() => {}
            WindowEvent::CursorMoved { position, .. } => {
                let projection = self.menu_pointer_projection();
                self.menu.pointer = projection.window.os_sample(position);
                self.menu.cursor = projection.project_pointer(self.menu.pointer).subpixel();
                if self.menu.options_open && self.menu.dragging {
                    self.menu_options_click(true, false);
                } else if let Some(title) = &mut self.menu.title {
                    let size = self
                        .renderer
                        .as_ref()
                        .map_or([1280, 720], |r| [r.config.width, r.config.height]);
                    let b = HudBatch::new(size, self.settings.gui_scale_inputs());
                    title.drag(self.menu.cursor, (b.width, b.height));
                    title.drag_field(&self.gui.as_ref().unwrap().atlas, self.menu.cursor.0);
                }
            }
            WindowEvent::MouseInput { state, button, .. } if self.menu.options_open => {
                if state == ElementState::Pressed && self.menu.options.selected_key.is_some() {
                    self.menu.options.modifiers = glfw_modifiers(self.menu.modifiers);
                    let events = self.menu.options.bind_mouse(&mut self.settings, button);
                    self.menu_option_events(events);
                } else if matches!(button, MouseButton::Left | MouseButton::Right) {
                    if state == ElementState::Pressed {
                        self.menu.dragging = button == MouseButton::Left;
                        self.menu_options_click(false, button == MouseButton::Right);
                    } else if button == MouseButton::Left {
                        self.menu.dragging = false;
                        let events = self.menu.options.release();
                        self.menu_option_events(events);
                    }
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let batch = HudBatch::new(self.menu_size(), self.settings.gui_scale_inputs());
                if let (Some(title), Some(gui)) = (&mut self.menu.title, &self.gui) {
                    if state == ElementState::Released {
                        title.release();
                        title.release_field();
                    } else {
                        let (action, _) = title.click(
                            self.menu.cursor.0,
                            self.menu.cursor.1,
                            (batch.width, batch.height),
                        );
                        title.press_field(
                            &gui.atlas,
                            self.menu.cursor,
                            (batch.width, batch.height),
                            self.menu.modifiers.shift_key(),
                        );
                        title.press_rules(
                            &gui.atlas,
                            self.menu.cursor,
                            (batch.width, batch.height),
                        );
                        self.menu_action(event_loop, action);
                    }
                }
            }
            // AbstractSelectionList.mouseClicked: a world row takes a click
            // of any button, so the middle one selects it without joining.
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Middle,
                ..
            } => {
                let batch = HudBatch::new(self.menu_size(), self.settings.gui_scale_inputs());
                if let Some(title) = &mut self.menu.title {
                    title.select_world_at(
                        self.menu.cursor.0,
                        self.menu.cursor.1,
                        (batch.width, batch.height),
                    );
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let raw = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 30.0) as f32,
                };
                let amount = if self.settings.discrete_scroll {
                    sunrise_client::mouse::java_signum(f64::from(raw)) as f32
                } else {
                    raw
                } * self.settings.mouse_wheel_sensitivity as f32;
                let size = self.menu_size();
                let batch = HudBatch::new(size, self.settings.gui_scale_inputs());
                if self.menu.options_open {
                    let events = self.menu.options.wheel(
                        &mut self.settings,
                        (batch.width, batch.height),
                        self.menu.cursor,
                        [size[0] as i32, size[1] as i32],
                        amount,
                    );
                    self.menu_option_events(events);
                } else if let Some(title) = &mut self.menu.title {
                    title.scroll_worlds(amount, self.menu.cursor, (batch.width, batch.height));
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Released && self.menu.options_open =>
            {
                if let PhysicalKey::Code(key) = event.physical_key {
                    self.menu.options.modifiers = glfw_modifiers(self.menu.modifiers);
                    let events = self.menu.options.key_released(&mut self.settings, key);
                    self.menu_option_events(events);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if event.physical_key == PhysicalKey::Code(KeyCode::F11) {
                    self.settings.fullscreen = !self.settings.fullscreen;
                    self.apply_menu_settings();
                    self.save_menu_settings();
                } else if self.menu.options_open {
                    let shortcut = self.menu.shortcut(&event);
                    let atlas = &self.gui.as_ref().unwrap().atlas;
                    let handled = shortcut.is_some_and(|s| {
                        self.menu
                            .options
                            .shortcut(atlas, s, &mut self.menu.clipboard)
                    });
                    if !handled && let PhysicalKey::Code(key) = event.physical_key {
                        self.menu.options.modifiers = glfw_modifiers(self.menu.modifiers);
                        let events = self.menu.options.key(
                            &mut self.settings,
                            atlas,
                            key,
                            event.text.as_deref(),
                        );
                        self.menu_option_events(events);
                    }
                } else {
                    let batch = HudBatch::new(self.menu_size(), self.settings.gui_scale_inputs());
                    let action = self.menu.title_key(&event, (batch.width, batch.height));
                    self.menu_action(event_loop, action);
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn draw_menu(&mut self, batch: &mut HudBatch) {
        // The buttons crates add to the title screen show by what is installed now
        // (`sunrise_hooks::title`), as on the game's own title screen.
        self.sync_title_mods();
        let logs = sunrise_level::logging::recent_lines();
        let Some(gui) = &self.gui else {
            // The pack's screens do not exist yet: the boot screen holds the
            // window, the wordmark and the console's lines while they load.
            sunrise_client::ui_theme::draw_boot(&self.boot, batch, &logs);
            return;
        };
        let cursor = if self.menu.key_focus.is_some() {
            self.menu.cursor
        } else {
            self.projected_menu_pointer().pixel()
        };
        // `Gui.setScreen` clears the focus of the screen it leaves
        // (`Screen.clearFocus`): the title behind the options, and the one
        // that comes back, start with no button highlighted.
        let screen = self.menu.key_screen();
        if super::pad_menu::another_screen_is_front(&mut self.menu.focus_screen, screen) {
            self.menu.options.clear_click_focus();
            if let Some(title) = self.menu.title.as_mut()
                && !matches!(title.mode, sunrise_client::title::Mode::MissingMods { .. })
            {
                title.focused_button = None;
            }
        }
        if self.menu.pending.is_some() {
            self.bar =
                sunrise_client::level_loading::ease_overlay(self.bar, self.progress.fraction());
            sunrise_client::level_loading::draw_loading_overlay(
                batch,
                &gui.atlas,
                self.bar,
                sunrise_client::level_loading::OverlayFade::OPAQUE,
                self.settings.file.bool("darkMojangStudiosBackground"),
            );
            let (bars, lines) = super::startup_progress::shown();
            sunrise_client::level_loading::draw_mod_progress(batch, &gui.atlas, &bars, &lines);
        } else if let Some((win, frame)) = &mut self.menu.win {
            let now = Instant::now();
            // 20 ticks a second: the startup menu runs no tick rate.
            let delta_ticks = now.duration_since(*frame).as_secs_f32() * 20.0;
            *frame = now;
            sunrise_client::widgets::begin_screen(|| win.screen_info());
            win.draw_background(batch, &gui.atlas, Default::default());
            win.draw_contents(batch, &gui.atlas, delta_ticks);
            if win.finished(batch.height) {
                self.menu.win = None;
            }
        } else if self.menu.options_open {
            let output = self
                .renderer
                .as_ref()
                .map_or(Default::default(), |r| r.output());
            self.menu
                .options
                .draw_background(&self.settings, output, batch, &gui.atlas);
            self.menu
                .options
                .draw_contents(&self.settings, batch, &gui.atlas, cursor);
            self.menu.options.draw_overlays(batch, &gui.atlas, cursor);
        } else if let Some(title) = &mut self.menu.title {
            title.poll_backup();
            if std::mem::take(&mut title.onboarding_finished) {
                // Options.onboardingAccessibilityFinished.
                self.settings.file.set(
                    "onboardAccessibility",
                    sunrise_client::options_file::V::Bool(false),
                );
                self.settings
                    .save(&sunrise_gameplay_common::game::options_path());
            }
            for found in title.lan_found_narrations((batch.width, batch.height)) {
                sunrise_narrator::say_system_queued(&found.plain());
            }
            let items = match &self.loaded {
                Some(Ok(loaded)) => loaded.client_pack.as_ref().map(|p| &*p.item_renderer),
                _ => None,
            };
            title.draw_with_items(
                &gui.atlas,
                items,
                batch,
                cursor,
                self.settings.file.bool("hideSplashTexts"),
                self.settings.file.bool("hideServerAddress"),
                &mut |_, _| {},
            );
        }
        if let Some(focus) = self.menu.key_focus
            && self.menu.pending.is_none()
            && self.menu.win.is_none()
        {
            key_focus::draw_outline(&gui.atlas, batch, focus);
        }
        // No ClientHost exists while resources load. Toast candidates are
        // admitted on handoff to Game, before their first animation or sound.
    }

    pub(in crate::game::client_side) fn restart_menu_resources(&mut self) -> Result<()> {
        if !self.menu.reload_packs || self.loaded.is_none() {
            return Ok(());
        }
        if let Some(Ok(loaded)) = &mut self.loaded {
            if let Some(loader) = &loaded.rest_loader
                && !loader.ready()?
            {
                return Ok(());
            }
            if let Some(loader) = loaded.rest_loader.take()
                && let Err(error) = loader.finish()
            {
                log::warn!("Sunrise: replaced resource generation failed: {error}");
            }
        }
        self.loaded = None;
        self.delivered = false;
        if let Some(handle) = self.handle.take() {
            handle.join().map_err(|payload| {
                format!("resource loader panicked: {}", panic_text(payload.as_ref()))
            })?;
        }
        self.progress = Progress::new();
        let options = self.options.as_ref().ok_or("loading already finished")?;
        let (receiver, handle) = spawn_loader(
            options,
            &self.settings,
            self.menu.pack_selection_revision,
            options.execution_role.into(),
            &self.vanilla_root,
            self.started,
            &self.progress,
            &self.max_atlas_size,
        )?;
        self.receiver = receiver;
        self.handle = Some(handle);
        self.menu.reload_packs = false;
        Ok(())
    }
}

/// The Mods screen before a game exists lists the installed packages as `Game::start_mods`
/// does, without what loading finds out (refusals, the native mods built in).
fn list_menu_mods(mods: &mut sunrise_mod_package::mod_list::ModList) {
    use sunrise_mod_package::mod_list::{DIR, Installed, ModInfo};
    mods.installed.clear();
    mods.info.clear();
    for path in sunrise_mod_package::find(&sunrise_gameplay_common::game::game_dir().join(DIR)) {
        let Ok(package) = sunrise_mod_package::Package::read(&path) else {
            continue;
        };
        let m = &package.manifest;
        mods.info
            .insert(m.id.clone(), ModInfo::of(m, package.icon.as_ref()));
        mods.installed.push(Installed {
            id: m.id.clone(),
            title: format!("{} {}", m.name, m.version),
            description: m.description.clone(),
            problem: None,
            dev: false,
            native: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn queued_creation_is_reconciled_at_handoff() {
        use mod_api::{Hooks, Registrar, RegistrationContext};
        use sunrise_game_data::gamerules::Value;
        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                registrar.game_rule_category("machines").unwrap();
                registrar
                    .game_rule_bool("enabled", "example:machines", true)
                    .unwrap();
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("example", Box::new(Mod))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                let saves = std::env::temp_dir()
                    .join(format!("sunrise-startup-rules-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&saves);
                let mut title = game.new_title(saves);
                title.enter_singleplayer();
                let Action::Create { mut options, .. } = title.create_action() else {
                    panic!("creation")
                };
                options.rules.set("keep_inventory", Value::Bool(true));
                let mut menu = StartupMenu {
                    title: Some(title),
                    pending: Some(PendingWorldEntry::Local {
                        folder: "test".into(),
                        name: None,
                        seed: Some(1),
                        creative: true,
                        create: Some(Box::new(options)),
                    }),
                    ..Default::default()
                };
                menu.handoff(game);
                let Some(PendingWorldEntry::Local {
                    create: Some(options),
                    ..
                }) = &game.client.pending_entry.0
                else {
                    panic!("pending")
                };
                assert_eq!(
                    options.rules.get("example:enabled"),
                    Some(Value::Bool(true))
                );
                assert_eq!(options.rules.get("keep_inventory"), Some(Value::Bool(true)));
                let request = super::super::world_loading::WorldRequest::new(
                    "test",
                    None,
                    None,
                    true,
                    Some(options),
                    options.rules.registry().clone(),
                )
                .unwrap();
                assert!(Arc::ptr_eq(
                    &request.rule_registry,
                    options.rules.registry()
                ));
            })
        });
    }

    #[test]
    fn queued_server_keeps_address_player_identity_and_lan_kind() {
        let mut menu = StartupMenu::default();
        menu.queue_entry(
            Action::Connect {
                address: "localhost:25565".into(),
                name: None,
                lan: true,
                realm: false,
            },
            "Guest".into(),
        );
        assert!(
            matches!(menu.pending.take(), Some(PendingWorldEntry::Server { address, name, lan: true, .. })
            if address == "localhost:25565" && name == "Guest")
        );
        assert!(menu.pending.take().is_none(), "an entry is consumed once");
    }
}
