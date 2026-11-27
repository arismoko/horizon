//! Player settings persisted in `options.txt` with vanilla's keys and value
//! encodings (Options.processOptions); lines Sunrise does not know are kept.
//! The option screens live in `options_ui`.
use crate::keybinds::KeyBinds;
use crate::options_file::{OptionsFile, V};
use std::{fs, path::Path};
use sunrise_protocol::packets::configuration::ClientInformation;

pub use sunrise_game_data::sound_source::SOUND_SOURCES;
pub use sunrise_level::sim_settings::SimSettings;

/// Per-source volumes for the mixer (soundCategory_* except master).
static SOURCE_VOLUMES: std::sync::RwLock<[f32; 11]> = std::sync::RwLock::new([1.0; 11]);
pub fn set_source_volumes(volumes: [f32; 11]) {
    *SOURCE_VOLUMES.write().unwrap_or_else(|e| e.into_inner()) = volumes;
}
/// The SoundSource an event plays on. Vanilla callers pick the source; this
/// follows their conventions by event family.
pub use sunrise_game_data::sound_source::sound_source_for_event as sound_source;
/// Volume multiplier of a sound source by index.
pub fn source_volume(i: usize) -> f32 {
    if i == 0 {
        1.0
    } else {
        SOURCE_VOLUMES.read().unwrap_or_else(|e| e.into_inner())[i]
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    /// Degrees, 30..=110 (options.txt stores (fov - 70) / 40).
    pub fov: f32,
    /// Chunks, 2..=32.
    pub render_distance: i32,
    /// Chunks, 5..=32.
    pub simulation_distance: i32,
    /// 0..1, vanilla default 0.5.
    pub sensitivity: f64,
    /// 0..1 (soundCategory_master).
    pub master_volume: f64,
    /// Volumes of every other sound source, indexed like SOUND_SOURCES.
    pub volumes: [f64; 11],
    /// Empty for system default, or a specific audio device name.
    pub sound_device: String,
    /// Requested scale (0 = auto); rendering caps it to the current framebuffer.
    pub gui_scale: u32,
    pub view_bobbing: bool,
    /// Brightness 0..1 (gamma).
    pub gamma: f64,
    /// 10..=260 (260 = unlimited).
    pub max_fps: u32,
    pub vsync: bool,
    pub fullscreen: bool,
    /// Smooth lighting (ao).
    pub smooth_lighting: bool,
    /// "false", "fast" or "true" (renderClouds).
    pub clouds: String,
    pub cloud_range: i32,
    /// 0 all, 1 decreased, 2 minimal.
    pub particles: u8,
    pub entity_distance: f64,
    pub entity_shadows: bool,
    pub biome_blend: i32,
    /// "fast", "fancy", "fabulous" or "custom".
    pub graphics_preset: String,
    pub mipmap_levels: i32,
    pub weather_radius: i32,
    pub chunk_fade: f64,
    pub vignette: bool,
    pub autosave_indicator: bool,
    pub cutout_leaves: bool,
    pub improved_transparency: bool,
    /// 0 off, 1 crosshair, 2 hotbar.
    pub attack_indicator: u8,
    pub inactivity_fps_limit: String,
    pub prioritize_chunk_updates: u8,
    pub menu_blur: i32,
    pub invert_y: bool,
    pub mouse_wheel_sensitivity: f64,
    pub discrete_scroll: bool,
    pub toggle_crouch: bool,
    pub toggle_sprint: bool,
    pub auto_jump: bool,
    pub language: String,
    pub force_unicode: bool,
    pub japanese_glyphs: bool,
    pub chat_visibility: u8,
    pub chat_colors: bool,
    pub chat_links: bool,
    pub chat_links_prompt: bool,
    pub chat_opacity: f64,
    /// Chat Delay in seconds (`chatDelay`): player chat shows this far apart.
    pub chat_delay: f64,
    pub text_background_opacity: f64,
    pub chat_scale: f64,
    pub chat_width: f64,
    pub chat_height_focused: f64,
    pub chat_height_unfocused: f64,
    pub chat_line_spacing: f64,
    pub auto_suggestions: bool,
    pub reduced_debug_info: bool,
    pub high_contrast: bool,
    /// `showSubtitles`: the closed-caption overlay.
    pub show_subtitles: bool,
    /// `notificationDisplayTime`: a multiplier for subtitle and toast times.
    pub notification_display_time: f64,
    pub main_hand_left: bool,
    /// Enabled resource packs, bottom first (resourcePacks).
    pub resource_packs: Vec<String>,
    pub keys: KeyBinds,
    /// Every option in its stored form (the typed fields above mirror it).
    pub file: OptionsFile,
    pub invert_x: bool,
    /// Allow widgets to replace the default mouse cursor icon.
    pub allow_cursor_changes: bool,
    pub toggle_attack: bool,
    pub toggle_use: bool,
    /// Taps of the sprint key counted as double taps (sprintWindow).
    pub sprint_window: i32,
    /// Show the operator items tab in a creative inventory.
    pub operator_items_tab: bool,
    /// Allow platform quit shortcuts to close the game window.
    pub quit_shortcuts: bool,
    /// On macOS, turn Control-click into a right click.
    pub ctrl_click_emulates_right_click: bool,
    /// Sunrise's own graphics settings (`sunrise-graphics.txt`, not options.txt).
    pub sunrise: crate::graphics::Graphics,
    /// The graphics API the game started with: `Options.preferredGraphicsBackendFromStartup`.
    pub startup_api: crate::graphics::GraphicsApi,
    /// `Monitor.videoModes` of the window's monitor, for the Video page (set by the game).
    pub video_modes: Vec<crate::window::VideoMode>,
    /// Fullbright is held off: this window plays on another's server (set by the game, not
    /// saved). The graphics screen hides its row; `extras.fullbright` keeps the choice.
    pub fullbright_held: bool,
    /// Controller settings (`sunrise-controller.txt`, Sunrise's own).
    pub controller: crate::gamepad::PadSettings,
    /// Sunrise's optional extras (`sunrise-extras.txt`), all off by default.
    pub extras: crate::extras::Extras,
    /// The Sunrise mods turned on (`sunrise-mods.txt`), all off by default.
    pub mods: crate::mod_list::ModList,
    /// The packs in `shaderpacks/`, listed when Sunrise Settings opens (Shaders); the pick is
    /// `sunrise.shader_pack`.
    pub shader_packs: Vec<crate::shader_pack_list::Entry>,
    /// The picked pack's options while its Shader Options page is open.
    pub shader_options: Option<crate::shader_options::ShaderOptions>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            fov: 70.0,
            render_distance: 12,
            simulation_distance: 12,
            sensitivity: 0.5,
            master_volume: 1.0,
            volumes: [1.0; 11],
            sound_device: String::new(),
            gui_scale: 0,
            view_bobbing: true,
            gamma: 0.5,
            // Vanilla's default, as for an options.txt without the key: a
            // capped frame rate leaves the cores idle when VSync is off.
            max_fps: 120,
            vsync: true,
            fullscreen: false,
            smooth_lighting: true,
            clouds: "true".into(),
            cloud_range: 128,
            particles: 0,
            entity_distance: 1.0,
            entity_shadows: true,
            biome_blend: 2,
            graphics_preset: "fancy".into(),
            mipmap_levels: 4,
            weather_radius: 10,
            chunk_fade: 0.75,
            vignette: true,
            autosave_indicator: true,
            cutout_leaves: true,
            improved_transparency: false,
            attack_indicator: 1,
            inactivity_fps_limit: "afk".into(),
            prioritize_chunk_updates: 0,
            menu_blur: 5,
            invert_y: false,
            mouse_wheel_sensitivity: 1.0,
            discrete_scroll: false,
            allow_cursor_changes: true,
            toggle_crouch: false,
            toggle_sprint: false,
            auto_jump: false,
            language: "en_us".into(),
            force_unicode: false,
            japanese_glyphs: false,
            chat_visibility: 0,
            chat_colors: true,
            chat_links: true,
            chat_links_prompt: true,
            chat_opacity: 1.0,
            chat_delay: 0.0,
            text_background_opacity: 0.5,
            chat_scale: 1.0,
            chat_width: 1.0,
            chat_height_focused: 1.0,
            chat_height_unfocused: 70.0 / 160.0,
            chat_line_spacing: 0.0,
            auto_suggestions: true,
            reduced_debug_info: false,
            high_contrast: false,
            show_subtitles: false,
            notification_display_time: 1.0,
            main_hand_left: false,
            resource_packs: Vec::new(),
            keys: KeyBinds::default(),
            file: OptionsFile::default(),
            invert_x: false,
            toggle_attack: false,
            toggle_use: false,
            sprint_window: 7,
            operator_items_tab: false,
            quit_shortcuts: true,
            ctrl_click_emulates_right_click: false,
            sunrise: Default::default(),
            startup_api: Default::default(),
            video_modes: Vec::new(),
            fullbright_held: false,
            controller: Default::default(),
            extras: Default::default(),
            mods: Default::default(),
            shader_packs: Vec::new(),
            shader_options: None,
        }
    }
}
fn put_d(f: &mut OptionsFile, key: &str, v: f64) {
    if v != f.double(key) {
        f.set(key, V::Double(v));
    }
}
fn put_i(f: &mut OptionsFile, key: &str, v: i32) {
    if v != f.int(key) {
        f.set(key, V::Int(v));
    }
}
fn put_b(f: &mut OptionsFile, key: &str, v: bool) {
    if v != f.bool(key) {
        f.set(key, V::Bool(v));
    }
}
fn put_name(f: &mut OptionsFile, key: &str, v: &str) {
    if v != f.name(key) {
        f.set_name(key, v);
    }
}
fn put_s(f: &mut OptionsFile, key: &str, v: &str) {
    if v != f.string(key) {
        f.set(key, V::Str(v.to_owned()));
    }
}
impl Settings {
    /// The index in `video_modes` of the preferred fullscreen mode, -1 (the desktop's own mode)
    /// when none is set or the monitor lacks it: `Monitor.indexOfMode`.
    pub fn video_mode_index(&self) -> i32 {
        self.file
            .fullscreen_resolution
            .as_deref()
            .and_then(crate::window::VideoMode::read)
            .and_then(|mode| self.video_modes.iter().position(|m| *m == mode))
            .map_or(-1, |i| i as i32)
    }
    pub fn load(path: &Path) -> Self {
        let (extras_text, keep_extras) = path
            .parent()
            .map_or((String::new(), false), crate::extras::Extras::read_text);
        let graphics_text = path.parent().and_then(|dir| {
            sunrise_hooks::settings::engine::read(&dir.join(crate::graphics::FILE))
                .ok()
                .flatten()
                .map(|read| read.text)
        });
        // The settings stores of the crates above the engine: their lines of `sunrise-extras.txt`
        // and `sunrise-graphics.txt`, or of their own files (docs/layers.md, "Platform
        // registries"). First, as the graphics settings below may be saved again with them.
        sunrise_hooks::settings::engine::load(&mut |file| match path.parent() {
            _ if file == crate::extras::FILE => Some(extras_text.clone()),
            _ if file == crate::graphics::FILE => graphics_text.clone(),
            Some(dir) => sunrise_hooks::settings::engine::read(&dir.join(file))
                .ok()
                .flatten()
                .map(|read| read.text),
            None => None,
        });
        let sunrise = path
            .parent()
            .map(|dir| {
                crate::graphics::Graphics::load_text(
                    dir,
                    graphics_text.as_deref().unwrap_or_default(),
                )
            })
            .unwrap_or_default();
        crate::mipmap::set_dither_cutouts(sunrise.dither_cutout_mips);
        let controller = path
            .parent()
            .map(crate::gamepad::PadSettings::load)
            .unwrap_or_default();
        let extras = crate::extras::Extras::from_text(&extras_text, keep_extras);
        // The enabled mods are the game folder's, shared by its players (`--player-dir`).
        let mods = path
            .parent()
            .map(|dir| crate::mod_list::ModList::load(&crate::logging::shared_dir(dir)))
            .unwrap_or_default();
        let settings = match fs::read(path) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                Self {
                    sunrise,
                    controller,
                    extras,
                    mods,
                    ..Self::parse(&text)
                }
            }
            Err(_) => Self {
                sunrise,
                controller,
                extras,
                mods,
                ..Self::default()
            },
        };
        // The loaded set is the active one: `keybind` text components show its keys.
        settings.keys.sync_action_bindings();
        // `Options.load`: `sharePresence` decides what the friends service posts.
        crate::social::set_share_presence(crate::social::SharePresence::of(
            settings.file.int("sharePresence").clamp(0, 2) as u8,
        ));
        settings.describe_for_crashes();
        // Options.load: the backend asked for at startup, what a restart is measured from.
        Self {
            startup_api: settings.sunrise.api,
            ..settings
        }
    }
    /// The render settings and resource packs a crash report lists.
    fn describe_for_crashes(&self) {
        let on = |b: bool| if b { "on" } else { "off" };
        let scale = match self.gui_scale {
            0 => "auto".to_owned(),
            n => n.to_string(),
        };
        crate::crash::set_detail(
            "Render Settings",
            format!(
                "render distance {}, simulation distance {}, graphics {}, FOV {:.0}, max FPS {}, VSync {}, fullscreen {}, GUI scale {scale}, smooth lighting {}, clouds {}, biome blend {}, mipmap levels {}",
                self.render_distance,
                self.simulation_distance,
                self.graphics_preset,
                self.fov,
                if self.max_fps >= 260 {
                    "unlimited".to_owned()
                } else {
                    self.max_fps.to_string()
                },
                on(self.vsync),
                on(self.fullscreen),
                on(self.smooth_lighting),
                self.clouds,
                self.biome_blend,
                self.mipmap_levels,
            ),
        );
        crate::crash::set_detail(
            "Resource Packs",
            if self.resource_packs.is_empty() {
                "vanilla".to_owned()
            } else {
                self.resource_packs.join(", ")
            },
        );
    }
    pub fn parse(text: &str) -> Self {
        Self::from_file(OptionsFile::parse(text))
    }
    /// Typed view of an options file.
    pub fn from_file(f: OptionsFile) -> Self {
        let mut keys = KeyBinds::default();
        for (name, _, _) in crate::keybinds::MAPPINGS {
            keys.set(name, &f.key_binding(name));
        }
        let mut volumes = [1.0; 11];
        for (i, source) in SOUND_SOURCES.iter().enumerate() {
            volumes[i] = f.double(&format!("soundCategory_{source}"));
        }
        Self {
            fov: f.int("fov") as f32,
            render_distance: f.int("renderDistance"),
            simulation_distance: f.int("simulationDistance"),
            sensitivity: f.double("mouseSensitivity"),
            master_volume: volumes[0],
            volumes,
            sound_device: f.string("soundDevice"),
            gui_scale: f.int("guiScale").max(0) as u32,
            view_bobbing: f.bool("bobView"),
            gamma: f.double("gamma"),
            max_fps: f.int("maxFps") as u32,
            vsync: f.bool("enableVsync"),
            fullscreen: f.bool("fullscreen"),
            smooth_lighting: f.bool("ao"),
            clouds: f.name("renderClouds"),
            cloud_range: f.int("cloudRange"),
            particles: f.int("particles") as u8,
            entity_distance: f.double("entityDistanceScaling"),
            entity_shadows: f.bool("entityShadows"),
            biome_blend: f.int("biomeBlendRadius"),
            graphics_preset: f.name("graphicsPreset"),
            mipmap_levels: f.int("mipmapLevels"),
            weather_radius: f.int("weatherRadius"),
            chunk_fade: f.double("chunkSectionFadeInTime"),
            vignette: f.bool("vignette"),
            autosave_indicator: f.bool("showAutosaveIndicator"),
            cutout_leaves: f.bool("cutoutLeaves"),
            improved_transparency: f.bool("improvedTransparency"),
            attack_indicator: f.int("attackIndicator") as u8,
            inactivity_fps_limit: f.name("inactivityFpsLimit"),
            prioritize_chunk_updates: f.int("prioritizeChunkUpdates") as u8,
            menu_blur: f.int("menuBackgroundBlurriness"),
            invert_x: f.bool("invertXMouse"),
            invert_y: f.bool("invertYMouse"),
            mouse_wheel_sensitivity: f.double("mouseWheelSensitivity"),
            discrete_scroll: f.bool("discrete_mouse_scroll"),
            allow_cursor_changes: f.bool("allowCursorChanges"),
            toggle_crouch: f.bool("toggleCrouch"),
            toggle_sprint: f.bool("toggleSprint"),
            toggle_attack: f.bool("toggleAttack"),
            toggle_use: f.bool("toggleUse"),
            sprint_window: f.int("sprintWindow"),
            operator_items_tab: f.bool("operatorItemsTab"),
            quit_shortcuts: f.bool("quitShortcuts"),
            ctrl_click_emulates_right_click: f.bool("ctrlClickEmulatesRightClick"),
            sunrise: Default::default(),
            startup_api: Default::default(),
            video_modes: Vec::new(),
            fullbright_held: false,
            controller: Default::default(),
            extras: Default::default(),
            mods: Default::default(),
            shader_packs: Vec::new(),
            shader_options: None,
            auto_jump: f.bool("autoJump"),
            language: f.string("lang"),
            force_unicode: f.bool("forceUnicodeFont"),
            japanese_glyphs: f.bool("japaneseGlyphVariants"),
            chat_visibility: f.int("chatVisibility") as u8,
            chat_colors: f.bool("chatColors"),
            chat_links: f.bool("chatLinks"),
            chat_links_prompt: f.bool("chatLinksPrompt"),
            chat_opacity: f.double("chatOpacity"),
            chat_delay: f.double("chatDelay"),
            text_background_opacity: f.double("textBackgroundOpacity"),
            chat_scale: f.double("chatScale"),
            chat_width: f.double("chatWidth"),
            chat_height_focused: f.double("chatHeightFocused"),
            chat_height_unfocused: f.double("chatHeightUnfocused"),
            chat_line_spacing: f.double("chatLineSpacing"),
            auto_suggestions: f.bool("autoSuggestions"),
            reduced_debug_info: f.bool("reducedDebugInfo"),
            high_contrast: f.bool("highContrast"),
            show_subtitles: f.bool("showSubtitles"),
            notification_display_time: f.double("notificationDisplayTime"),
            main_hand_left: f.name("mainHand") == "left",
            resource_packs: f.list("resourcePacks"),
            keys,
            file: f,
        }
    }
    /// Validated live `(textureFiltering, maxAnisotropyBit, mipmapLevels)` for atlas stitching.
    pub fn texture_atlas_settings(&self) -> (i32, i32, i32) {
        (
            self.file.int("textureFiltering").clamp(0, 2),
            self.file.int("maxAnisotropyBit").clamp(1, 3),
            self.mipmap_levels.clamp(0, 4),
        )
    }
    /// `Options.createInformation`: what the server learns about this player.
    pub fn client_information(&self, view_distance: i8) -> ClientInformation {
        ClientInformation {
            language: self.language.clone(),
            chat_visibility: self.chat_visibility as i32,
            chat_colors: self.chat_colors,
            model_customisation: self.file.model_customisation(),
            main_hand: i32::from(!self.main_hand_left),
            allows_listing: self.file.bool("allowServerListing"),
            particle_status: i32::from(self.particles),
            ..ClientInformation::new(view_distance)
        }
    }
    /// The options file reflecting the typed fields; values the game did not
    /// change keep their stored form.
    pub fn to_file(&self) -> OptionsFile {
        let mut f = self.file.clone();
        put_i(&mut f, "fov", self.fov as i32);
        put_i(&mut f, "renderDistance", self.render_distance);
        put_i(&mut f, "simulationDistance", self.simulation_distance);
        put_d(&mut f, "mouseSensitivity", self.sensitivity);
        for (i, source) in SOUND_SOURCES.iter().enumerate() {
            let v = if i == 0 {
                self.master_volume
            } else {
                self.volumes[i]
            };
            put_d(&mut f, &format!("soundCategory_{source}"), v);
        }
        put_s(&mut f, "soundDevice", &self.sound_device);
        put_i(&mut f, "guiScale", self.gui_scale as i32);
        put_b(&mut f, "bobView", self.view_bobbing);
        put_d(&mut f, "gamma", self.gamma);
        put_i(&mut f, "maxFps", self.max_fps as i32);
        put_b(&mut f, "enableVsync", self.vsync);
        put_b(&mut f, "fullscreen", self.fullscreen);
        put_b(&mut f, "ao", self.smooth_lighting);
        put_name(&mut f, "renderClouds", &self.clouds);
        put_i(&mut f, "cloudRange", self.cloud_range);
        put_i(&mut f, "particles", i32::from(self.particles));
        put_d(&mut f, "entityDistanceScaling", self.entity_distance);
        put_b(&mut f, "entityShadows", self.entity_shadows);
        put_i(&mut f, "biomeBlendRadius", self.biome_blend);
        put_name(&mut f, "graphicsPreset", &self.graphics_preset);
        put_i(&mut f, "mipmapLevels", self.mipmap_levels);
        put_i(&mut f, "weatherRadius", self.weather_radius);
        put_d(&mut f, "chunkSectionFadeInTime", self.chunk_fade);
        put_b(&mut f, "vignette", self.vignette);
        put_b(&mut f, "showAutosaveIndicator", self.autosave_indicator);
        put_b(&mut f, "cutoutLeaves", self.cutout_leaves);
        put_b(&mut f, "improvedTransparency", self.improved_transparency);
        put_i(&mut f, "attackIndicator", i32::from(self.attack_indicator));
        put_name(&mut f, "inactivityFpsLimit", &self.inactivity_fps_limit);
        put_i(
            &mut f,
            "prioritizeChunkUpdates",
            i32::from(self.prioritize_chunk_updates),
        );
        put_i(&mut f, "menuBackgroundBlurriness", self.menu_blur);
        put_b(&mut f, "invertXMouse", self.invert_x);
        put_b(&mut f, "invertYMouse", self.invert_y);
        put_d(
            &mut f,
            "mouseWheelSensitivity",
            self.mouse_wheel_sensitivity,
        );
        put_b(&mut f, "discrete_mouse_scroll", self.discrete_scroll);
        put_b(&mut f, "allowCursorChanges", self.allow_cursor_changes);
        put_b(&mut f, "toggleCrouch", self.toggle_crouch);
        put_b(&mut f, "toggleSprint", self.toggle_sprint);
        put_b(&mut f, "toggleAttack", self.toggle_attack);
        put_b(&mut f, "toggleUse", self.toggle_use);
        put_i(&mut f, "sprintWindow", self.sprint_window);
        put_b(&mut f, "operatorItemsTab", self.operator_items_tab);
        put_b(&mut f, "quitShortcuts", self.quit_shortcuts);
        put_b(
            &mut f,
            "ctrlClickEmulatesRightClick",
            self.ctrl_click_emulates_right_click,
        );
        put_b(&mut f, "autoJump", self.auto_jump);
        put_s(&mut f, "lang", &self.language);
        put_b(&mut f, "forceUnicodeFont", self.force_unicode);
        put_b(&mut f, "japaneseGlyphVariants", self.japanese_glyphs);
        put_i(&mut f, "chatVisibility", i32::from(self.chat_visibility));
        put_b(&mut f, "chatColors", self.chat_colors);
        put_b(&mut f, "chatLinks", self.chat_links);
        put_b(&mut f, "chatLinksPrompt", self.chat_links_prompt);
        put_d(&mut f, "chatOpacity", self.chat_opacity);
        put_d(&mut f, "chatDelay", self.chat_delay);
        put_d(
            &mut f,
            "textBackgroundOpacity",
            self.text_background_opacity,
        );
        put_d(&mut f, "chatScale", self.chat_scale);
        put_d(&mut f, "chatWidth", self.chat_width);
        put_d(&mut f, "chatHeightFocused", self.chat_height_focused);
        put_d(&mut f, "chatHeightUnfocused", self.chat_height_unfocused);
        put_d(&mut f, "chatLineSpacing", self.chat_line_spacing);
        put_b(&mut f, "autoSuggestions", self.auto_suggestions);
        put_b(&mut f, "reducedDebugInfo", self.reduced_debug_info);
        put_b(&mut f, "highContrast", self.high_contrast);
        put_b(&mut f, "showSubtitles", self.show_subtitles);
        put_d(
            &mut f,
            "notificationDisplayTime",
            self.notification_display_time,
        );
        put_name(
            &mut f,
            "mainHand",
            if self.main_hand_left { "left" } else { "right" },
        );
        if self.resource_packs != f.list("resourcePacks") {
            f.set("resourcePacks", V::List(self.resource_packs.clone()));
        }
        for (name, key) in self.keys.saved() {
            put_s(&mut f, &format!("key_{name}"), &key);
        }
        f
    }
    /// These settings with one `options.txt` value changed (`sunrise-ctl
    /// options-set`, as `OptionsFile::set_text` reads it). Sunrise's own
    /// files (graphics, controller, extras, the mod list) aren't in
    /// `options.txt`, so they stay as they are, as do the monitor's video modes.
    pub fn with_option(&self, key: &str, value: &str) -> Result<Self, String> {
        let mut file = self.to_file();
        file.set_text(key, value)?;
        Ok(Self {
            sunrise: self.sunrise.clone(),
            controller: self.controller.clone(),
            extras: self.extras.clone(),
            mods: self.mods.clone(),
            video_modes: self.video_modes.clone(),
            ..Self::from_file(file)
        })
    }
    /// The `options.txt` text (Options.save).
    pub fn serialize(&self) -> String {
        self.to_file().serialize()
    }
    pub fn save(&self, path: &Path) {
        self.describe_for_crashes();
        if let Err(e) = fs::write(path, self.serialize()) {
            log::warn!("Sunrise: options.txt is not saved: {e}");
        }
        if let Some(dir) = path.parent() {
            self.sunrise.save(dir);
            self.controller.save(dir);
            self.extras.save(dir);
            for (file, e) in sunrise_hooks::settings::engine::save_own(
                dir,
                &[crate::extras::FILE, crate::graphics::FILE],
            ) {
                log::warn!("Sunrise: {file} is not saved: {e}");
            }
            if let Err(e) = self.mods.save(&crate::logging::shared_dir(dir)) {
                log::warn!("Sunrise: {} is not saved: {e}", crate::mod_list::FILE);
            }
        }
    }
    /// Sets the picked shader pack, for the pack list and the toggle key alike: an empty pick
    /// turns shaders off and keeps the last non-empty pick, a non-empty pick (or a switch to
    /// another pack) becomes the last one too. The toggle key resumes exactly this last pick.
    pub fn set_shader_pick(&mut self, new_key: &str) {
        if new_key.is_empty() {
            if !self.sunrise.shader_pack.is_empty() {
                self.extras.last_shader_pack = self.sunrise.shader_pack.clone();
            }
            self.sunrise.shader_pack.clear();
        } else {
            self.sunrise.shader_pack = new_key.to_owned();
            self.extras.last_shader_pack = new_key.to_owned();
        }
    }
    /// Volume of a sound source (master excluded).
    pub fn volume(&self, source: &str) -> f64 {
        SOUND_SOURCES
            .iter()
            .position(|n| *n == source)
            .map_or(1.0, |i| {
                if i == 0 {
                    self.master_volume
                } else {
                    self.volumes[i]
                }
            })
    }
    /// `Options.getFinalSoundSourceVolume`: a source's volume times master's.
    pub fn final_volume(&self, source: &str) -> f32 {
        if source == "master" {
            self.master_volume as f32
        } else {
            (self.volume(source) * self.master_volume) as f32
        }
    }
    /// `Options.narrator`: the `narrator` id in options.txt.
    pub fn narrator(&self) -> sunrise_narrator::Status {
        sunrise_narrator::Status::by_id(self.file.int("narrator"))
    }
    pub fn set_narrator(&mut self, status: sunrise_narrator::Status) {
        self.file
            .set("narrator", crate::options_file::V::Int(status.id()));
    }
    /// `Options.getBackgroundColor(float)`: the text backdrop colour, `default`
    /// opacity unless `backgroundForChatOnly` is off.
    pub fn text_background(&self, default: f32) -> u32 {
        crate::hud_layout::background_color(
            self.file.bool("backgroundForChatOnly"),
            self.text_background_opacity,
            default,
        )
    }
    /// The faded name-tag text alpha that goes with [`Self::text_background`]
    /// (`hud_layout::faded_text_alpha`).
    pub fn faded_text_alpha(&self, default: f32) -> u32 {
        crate::hud_layout::faded_text_alpha(
            self.file.bool("backgroundForChatOnly"),
            self.text_background_opacity,
            default,
        )
    }
    pub fn chat_options(&self) -> crate::chat::ChatOptions {
        crate::chat::ChatOptions {
            opacity: self.chat_opacity,
            background_opacity: self.text_background_opacity,
            scale: self.chat_scale,
            width: self.chat_width,
            height_focused: self.chat_height_focused,
            height_unfocused: self.chat_height_unfocused,
            line_spacing: self.chat_line_spacing,
            colors: self.chat_colors,
        }
    }
}

/// One option instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opt {
    Fov,
    RenderDistance,
    /// `RenderDistance` on the classic Video Settings page: vanilla's range (2..=32), same saved value.
    VanillaRenderDistance,
    SimulationDistance,
    Sensitivity,
    WheelSensitivity,
    DiscreteScroll,
    InvertX,
    InvertY,
    AllowCursorChanges,
    Volume(usize),
    /// Setting `.1` of the native mod in row `.0` of Options > Mods, a range on a slider.
    ModSetting(u16, u16),
    SoundDevice,
    GuiScale,
    ViewBobbing,
    Gamma,
    MaxFps,
    Vsync,
    Fullscreen,
    SmoothLighting,
    Clouds,
    CloudRange,
    Particles,
    EntityDistance,
    EntityShadows,
    BiomeBlend,
    Preset,
    Mipmaps,
    WeatherRadius,
    ChunkFade,
    Vignette,
    Autosave,
    CutoutLeaves,
    ImprovedTransparency,
    AttackIndicator,
    InactivityFps,
    ChunkBuilder,
    MenuBlur,
    ToggleCrouch,
    ToggleSprint,
    ToggleAttack,
    ToggleUse,
    AutoJump,
    SprintWindow,
    QuitShortcuts,
    CtrlClickEmulatesRightClick,
    OperatorItemsTab,
    ChatVisibility,
    ChatColors,
    ChatLinks,
    ChatLinksPrompt,
    ChatOpacity,
    TextBackgroundOpacity,
    ChatScale,
    ChatWidth,
    ChatHeightFocused,
    ChatHeightUnfocused,
    ChatLineSpacing,
    AutoSuggestions,
    HideMatchedNames,
    ReducedDebugInfo,
    OnlySecureChat,
    SaveChatDrafts,
    HighContrast,
    ShowSubtitles,
    MusicFrequency,
    NotificationDisplayTime,
    Narrator,
    TextBackground,
    ChatDelay,
    DistortionEffects,
    FovEffects,
    DarknessPulsing,
    DamageTilt,
    GlintSpeed,
    GlintStrength,
    HideLightningFlashes,
    MonochromeLogo,
    PanoramaSpeed,
    HideSplashTexts,
    NarratorHotkey,
    RotateWithMinecarts,
    HighContrastBlockOutlines,
    ForceUnicode,
    JapaneseGlyphs,
    /// Sunrise's controller settings (`sunrise-controller.txt`).
    PadEnabled,
    PadVibration,
    PadPlayInBackground,
    PadAdaptiveTriggers,
    PadLightbar,
    PadLookSensitivity,
    PadPointerSpeed,
    PadScrollSpeed,
    PadRepeatDelay,
    PadRepeatInterval,
    PadInvertX,
    PadInvertY,
    PadCraftOnServers,
    PadToggleSneak,
    PadMoveDeadzone,
    PadLookDeadzone,
    PadTriggerThreshold,
    PadGlyphs,
    /// A Nintendo pad's face buttons by place or by letter (`PadSettings::nintendo_letters`).
    PadNintendoButtons,
    PadKeyboard,
    PadGameplayGuide,
    /// The radial menu's opener held or pressed once (`PadSettings::radial_mode`).
    PadRadialMode,
    /// The stick the radial menu selects with (`PadSettings::radial_stick`).
    PadRadialStick,
    /// `options.modelPart.*`: one skin layer (index into `options_file::MODEL_PARTS`).
    ModelPart(usize),
    /// A slider of a page a crate registered (`sunrise_hooks::pages`): (page, row).
    Page(usize, usize),
    MainHand,
    AllowServerListing,
    RealmsNotifications,
    InGameNotification,
    SharePresence,
    DirectionalAudio,
    MusicToast,
    /// `Options.exclusiveFullscreen`.
    ExclusiveFullscreen,
    /// VideoSettingsScreen's "Exclusive Fullscreen Mode": the monitor's mode to take.
    ExclusiveMode,
    /// `Options.preferredGraphicsBackend`.
    GraphicsBackend,
    /// `Options.textureFiltering`.
    TextureFiltering,
    /// `Options.maxAnisotropyBit`.
    MaxAnisotropy,
}

impl Settings {
    /// GUI scaling inputs for this settings owner.
    pub fn gui_scale_inputs(&self) -> crate::hud::GuiScale {
        crate::hud::GuiScale {
            requested: self.gui_scale,
            unicode: self.force_unicode,
        }
    }

    pub fn sim_settings(&self) -> SimSettings {
        SimSettings {
            render_distance: self.render_distance,
            simulation_distance: self.simulation_distance,
            entity_distance: self.entity_distance,
            parallel_tick: self.sunrise.parallel_tick,
            auto_jump: self.auto_jump,
            main_hand_left: self.main_hand_left,
            chat_visibility: self.chat_visibility,
            operator_items_tab: self.operator_items_tab,
            reduced_debug_info: self.reduced_debug_info,
            language: self.language.clone(),
            mods: self.mods.clone(),
            no_ai: self.extras.no_ai,
            agent_control: self.extras.agent_control,
        }
    }

    pub fn apply_sim_settings(&mut self, sim: &SimSettings) {
        self.render_distance = sim.render_distance;
        self.simulation_distance = sim.simulation_distance;
        self.entity_distance = sim.entity_distance;
        self.sunrise.parallel_tick = sim.parallel_tick;
        self.auto_jump = sim.auto_jump;
        self.main_hand_left = sim.main_hand_left;
        self.chat_visibility = sim.chat_visibility;
        self.operator_items_tab = sim.operator_items_tab;
        self.reduced_debug_info = sim.reduced_debug_info;
        self.language.clone_from(&sim.language);
        self.mods.clone_from(&sim.mods);
        self.extras.no_ai = sim.no_ai;
        self.extras.agent_control = sim.agent_control;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_narrator_reads_its_status_and_the_voice_under_master() {
        let s = Settings::parse("narrator:2\nsoundCategory_master:0.5\nsoundCategory_voice:0.5\n");
        assert_eq!(s.narrator(), sunrise_narrator::Status::Chat);
        assert_eq!(s.final_volume("voice"), 0.25);
        assert_eq!(s.final_volume("master"), 0.5);
        let mut s = s;
        s.set_narrator(sunrise_narrator::Status::Off);
        assert_eq!(
            Settings::parse(&s.serialize()).narrator(),
            sunrise_narrator::Status::Off
        );
    }
    /// `Monster.getSoundSource`: the verified families and inherited common sounds.
    #[test]
    fn hostile_families_keep_their_sound_category() {
        for (event, source) in [
            ("entity.zombie.hurt", 5),
            ("entity.cow.hurt", 6),
            ("block.stone.break", 4),
            ("entity.player.hurt", 7),
            ("entity.arrow.hit", 7),
            ("music_disc.13", 2),
            ("item.goat_horn.sound.0", 2),
            ("weather.rain", 3),
            ("ambient.cave", 8),
        ] {
            assert_eq!(sound_source(event), source, "{event}");
        }
        // Explicit sources still take precedence, including H331's arrow route.
        let mut audio = crate::audio::Audio::silent(
            &sunrise_assets::resource_source::ResourceSource::directory(
                Path::new("/nonexistent/h068-audio").to_path_buf(),
            ),
        );
        audio.capture = Some(Vec::new());
        audio.play_on(6, "minecraft:entity.arrow.hit", Some([1.0; 3]), 1.0, 1.0);
        audio.play_on(4, "minecraft:entity.warden.hurt", Some([1.0; 3]), 1.0, 1.0);
        assert_eq!(
            audio
                .capture
                .as_ref()
                .unwrap()
                .iter()
                .map(|s| s.source)
                .collect::<Vec<_>>(),
            [6, 4]
        );
        for family in [
            "bogged",
            "breeze",
            "endermite",
            "parched",
            "warden",
            "zombie_villager",
            "vex",
            "wither",
            "hostile",
        ] {
            for suffix in ["hurt", "death"] {
                for prefix in ["", "minecraft:"] {
                    let event = format!("{prefix}entity.{family}.{suffix}");
                    assert_eq!(sound_source(&event), 5, "{event}");
                }
            }
        }
    }

    #[test]
    fn an_option_set_keeps_sunrise_files() {
        let mut s = Settings::parse("fov:70\n");
        s.mods.enabled.insert("zoom".into());
        s.extras.sort = true;
        s.sunrise.fog = !s.sunrise.fog;
        s.controller.vibration = !s.controller.vibration;
        let set = s.with_option("fov", "90").unwrap();
        assert_eq!(set.fov, 90.0);
        assert_eq!(set.mods, s.mods);
        assert_eq!(set.extras, s.extras);
        assert_eq!(set.sunrise, s.sunrise);
        assert_eq!(set.controller, s.controller);
        assert!(s.with_option("fov", "200").is_err());
    }
    #[test]
    fn gui_scale_survives_temporary_small_framebuffer() {
        for requested in [0, 1, 2, 6] {
            let settings = Settings::parse(&format!("guiScale:{requested}\n"));
            // Hyprland's initial surface can be smaller than the fullscreen one.
            for (framebuffer, maximum) in [([715, 860], 2), ([3400, 1913], 7)] {
                let effective = crate::window::calculate_scale(
                    framebuffer,
                    settings.gui_scale as i32,
                    settings.force_unicode,
                );
                assert_eq!(
                    effective,
                    if requested == 0 {
                        maximum
                    } else {
                        (requested as i32).min(maximum)
                    }
                );
                let saved = Settings::parse(&settings.serialize());
                assert_eq!(saved.gui_scale, requested);
            }
        }
    }

    #[test]
    fn round_trips_vanilla_options_txt() {
        let text = "version:4000\nfov:0.5\nrenderDistance:16\nmaxFps:260\nrenderClouds:\"fast\"\nkey_key.jump:key.keyboard.j\nsoundCategory_music:0.25\nresourcePacks:[\"vanilla\",\"file/x.zip\"]\nlang:fr_fr\nsomeFutureOption:42\n";
        let s = Settings::parse(text);
        assert_eq!(s.fov, 90.0);
        assert_eq!(s.render_distance, 16);
        assert_eq!(s.max_fps, 260);
        assert_eq!(s.clouds, "fast");
        assert_eq!(s.keys.get("key.jump"), "key.keyboard.j");
        assert_eq!(s.volume("music"), 0.25);
        assert_eq!(s.resource_packs, ["vanilla", "file/x.zip"]);
        assert_eq!(s.language, "fr_fr");
        let out = s.serialize();
        assert!(out.starts_with("version:"));
        assert!(out.contains("\nkey_key.jump:key.keyboard.j\n"));
        // Options.save writes only the options it knows.
        assert!(!out.contains("someFutureOption"));
        assert!(out.contains("\nrenderClouds:\"fast\"\n"));
        let again = Settings::parse(&out);
        assert_eq!(again.fov, 90.0);
        assert_eq!(again.keys.get("key.jump"), "key.keyboard.j");
    }
    #[test]
    fn client_information_maps_the_stored_main_hand() {
        // The Skin Customization option is stored by name; ClientInformation
        // sends HumanoidArm (LEFT = 0, RIGHT = 1).
        for (stored, arm) in [("left", 0), ("right", 1)] {
            let s = Settings::parse(&format!("mainHand:{stored}\n"));
            assert_eq!(s.client_information(10).main_hand, arm);
            assert!(s.serialize().contains(&format!("mainHand:\"{stored}\"")));
        }
        assert_eq!(Settings::default().client_information(10).main_hand, 1);
    }
    #[test]
    fn no_options_txt_paces_frames_like_an_empty_one() {
        let (fresh, empty) = (Settings::default(), Settings::parse(""));
        assert_eq!(fresh.max_fps, 120);
        assert_eq!(fresh.max_fps, empty.max_fps);
        assert_eq!(fresh.vsync, empty.vsync);
        assert_eq!(fresh.inactivity_fps_limit, empty.inactivity_fps_limit);
        assert_eq!(fresh.render_distance, empty.render_distance);
        assert_eq!(fresh.simulation_distance, empty.simulation_distance);
    }
    #[test]
    fn subtitle_options() {
        let s = Settings::parse("showSubtitles:true\nnotificationDisplayTime:2.5\n");
        assert!(s.show_subtitles);
        assert_eq!(s.notification_display_time, 2.5);
        let again = Settings::parse(&s.serialize());
        assert!(again.show_subtitles);
        assert_eq!(again.notification_display_time, 2.5);
        let d = Settings::parse("");
        assert!(!d.show_subtitles);
        assert_eq!(d.notification_display_time, 1.0);
        // Codec.doubleRange(0.5, 10.0) rejects other values; the default stays.
        assert_eq!(
            Settings::parse("notificationDisplayTime:50\n").notification_display_time,
            1.0
        );
    }
    #[test]
    fn sound_device_option() {
        let s = Settings::parse("soundDevice:\"Headphones\"\n");
        assert_eq!(s.sound_device, "Headphones");
        let serialized = s.serialize();
        assert!(serialized.contains("soundDevice:\"Headphones\""));
        let again = Settings::parse(&serialized);
        assert_eq!(again.sound_device, "Headphones");
        let empty = Settings::parse("");
        assert_eq!(empty.sound_device, "");
    }
    #[test]
    fn texture_atlas_settings_uses_live_validated_mipmap_and_filtering() {
        let mut s = Settings::default();
        assert_eq!(s.texture_atlas_settings(), (0, 2, 4));
        // Live `mipmap_levels` wins even before `file` is rewritten.
        s.mipmap_levels = 0;
        assert_eq!(s.file.int("mipmapLevels"), 4);
        assert_eq!(s.texture_atlas_settings(), (0, 2, 0));
        // Out-of-range stored values fail `OptionInstance` validation and keep the defaults.
        s.mipmap_levels = 4;
        s.file
            .set("textureFiltering", crate::options_file::V::Int(9));
        s.file
            .set("maxAnisotropyBit", crate::options_file::V::Int(0));
        assert_eq!(s.texture_atlas_settings(), (0, 2, 4));
    }
}
