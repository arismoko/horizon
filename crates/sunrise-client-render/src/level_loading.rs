//! LevelLoadingScreen: "Loading terrain..." over the chunk status map and
//! the server's load progress while a world opens or a portal is crossed.
use crate::{
    dimension_transition::ActiveTransitionScreen,
    hud::{HudAtlas, HudBatch},
    text::Text,
};

pub use sunrise_level::level_loading::{STATUS_RADIUS, ServerLoadView, Stage, Status, progress};

/// `LevelLoadingScreen.Reason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    NetherPortal,
    EndPortal,
    Other,
}

impl From<Reason> for mod_api::DimensionTransitionReason {
    fn from(reason: Reason) -> Self {
        match reason {
            Reason::NetherPortal => Self::NetherPortal,
            Reason::EndPortal => Self::EndPortal,
            Reason::Other => Self::Other,
        }
    }
}

/// `LevelLoadTracker.CLIENT_WAIT_TIMEOUT_MS`.
pub const CLIENT_WAIT_TIMEOUT_MS: u64 = 30_000;
/// `LevelLoadTracker.LEVEL_LOAD_CLOSE_DELAY_MS`: how long a new world's loading
/// screen stays once the level is ready.
pub const LEVEL_LOAD_CLOSE_DELAY_MS: u64 = 500;

/// `ClientPacketListener.determineLevelLoadingReason`.
pub fn reason_for_respawn(player_died: bool, dimension: &str, old_dimension: &str) -> Reason {
    const NETHER: &str = "minecraft:the_nether";
    const END: &str = "minecraft:the_end";
    if player_died {
        Reason::Other
    } else if dimension == NETHER || old_dimension == NETHER {
        Reason::NetherPortal
    } else if dimension == END || old_dimension == END {
        Reason::EndPortal
    } else {
        Reason::Other
    }
}

/// `LevelLoadTracker.ClientState`, with the times in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClientState {
    /// `WaitingForServer`: until the loading start game event.
    WaitingForServer { timeout_after: u64 },
    /// `WaitingForPlayerChunk`: until the section around the player is shown.
    WaitingForPlayerChunk {
        player_section_ready: bool,
        /// Without a renderer: the player's section and when it was first
        /// seen compiled, its `RenderSection.uploadedTime` that its fade-in
        /// counts from (`player_section_compiled`).
        section_compiled_at: Option<([i32; 3], u64)>,
        timeout_after: u64,
    },
    /// `ClientLevelReady`.
    Ready { ready_at: u64 },
}

/// What `WaitingForPlayerChunk.isReady` asks of the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerCheck {
    /// `ClientLevel.isOutsideBuildHeight(camera y)`.
    pub outside_build_height: bool,
    pub spectator: bool,
    pub alive: bool,
}

/// The client half of `LevelLoadTracker`: what the loading screen waits for
/// after a login or a respawn, and the screen it is shown on.
#[derive(Debug)]
pub struct LevelLoadTracker {
    state: Option<ClientState>,
    transition_screen: Option<ActiveTransitionScreen>,
    close_delay_ms: u64,
    /// `LevelLoadingScreen.reason`.
    pub reason: Reason,
}
impl LevelLoadTracker {
    pub fn new(close_delay_ms: u64) -> Self {
        Self {
            state: None,
            transition_screen: None,
            close_delay_ms,
            reason: Reason::Other,
        }
    }
    /// `startClientLoad`, for the screen `reason` (`startWaitingForNewLevel`).
    /// Restarting the tracker while the screen is open keeps its presentation.
    pub fn start_client_load(&mut self, now: u64, reason: Reason) {
        self.state = Some(ClientState::WaitingForServer {
            timeout_after: now + CLIENT_WAIT_TIMEOUT_MS,
        });
        self.reason = reason;
    }
    /// The tracker's work is over and the screen has closed
    /// (`ClientPacketListener.tick` drops it once the level is ready).
    pub fn finish(&mut self) {
        self.state = None;
        self.transition_screen = None;
    }
    /// Install the presentation chosen for a newly opened loading screen.
    pub fn set_transition_screen(&mut self, screen: ActiveTransitionScreen) {
        if self.is_active() {
            self.transition_screen = Some(screen);
        }
    }
    /// The selected presentation, absent for vanilla or after completion.
    pub fn transition_screen(&self) -> Option<&ActiveTransitionScreen> {
        self.transition_screen.as_ref()
    }
    /// A load is under way, so its screen is up.
    pub fn is_active(&self) -> bool {
        self.state.is_some()
    }
    /// `loadingPacketsReceived`: `LEVEL_CHUNKS_LOAD_START` came.
    pub fn loading_packets_received(&mut self) {
        if let Some(ClientState::WaitingForServer { timeout_after }) = self.state {
            self.state = Some(ClientState::WaitingForPlayerChunk {
                player_section_ready: false,
                section_compiled_at: None,
                timeout_after,
            });
        }
    }
    /// `getPlayerCompiledSectionCallback` is not null: the renderer is to say
    /// when the player's section is shown.
    pub fn wants_player_section(&self) -> bool {
        matches!(self.state, Some(ClientState::WaitingForPlayerChunk { .. }))
    }
    /// The callback: the player's section is compiled and visible.
    pub fn player_section_ready(&mut self) {
        if let Some(ClientState::WaitingForPlayerChunk {
            player_section_ready,
            ..
        }) = &mut self.state
        {
            *player_section_ready = true;
        }
    }
    /// `LevelRenderer`'s check before that callback, for a client without a
    /// renderer: the player's section `section` (block coordinates of the
    /// camera, `>> 4`) is compiled while `compiled`, and
    /// `isSectionCompiledAndVisible` holds once `RenderSection.getVisibility`
    /// has reached 0.3 since that section was first seen compiled (225 ms at
    /// the default 0.75 s `chunkSectionFadeInTime`). Another section, or one
    /// no longer compiled, starts its fade-in again.
    pub fn player_section_compiled(
        &mut self,
        section: [i32; 3],
        compiled: bool,
        now: u64,
        fade_ms: u64,
    ) {
        let Some(ClientState::WaitingForPlayerChunk {
            player_section_ready,
            section_compiled_at,
            ..
        }) = &mut self.state
        else {
            return;
        };
        if !compiled {
            *section_compiled_at = None;
            return;
        }
        let uploaded = match *section_compiled_at {
            Some((at, uploaded)) if at == section => uploaded,
            _ => {
                *section_compiled_at = Some((section, now));
                now
            }
        };
        let visibility = crate::gfx::chunk_visibility(now.saturating_sub(uploaded), fade_ms);
        if visibility >= crate::gfx::CHUNK_VISIBILITY_THRESHOLD {
            *player_section_ready = true;
        }
    }
    /// `tickClientLoad`.
    pub fn tick_client_load(&mut self, now: u64, player: PlayerCheck) {
        if let Some(ClientState::WaitingForPlayerChunk {
            player_section_ready,
            timeout_after,
            ..
        }) = self.state
        {
            let ready = if now > timeout_after {
                log::warn!(
                    "Timed out while waiting for the client to load chunks, letting the player into the world anyway"
                );
                true
            } else if !player.outside_build_height && !player.spectator && player.alive {
                player_section_ready
            } else {
                true
            };
            if ready {
                self.state = Some(ClientState::Ready { ready_at: now });
            }
        }
    }
    /// `isLevelReady`.
    pub fn is_level_ready(&self, now: u64) -> bool {
        matches!(self.state, Some(ClientState::Ready { ready_at }) if now >= ready_at + self.close_delay_ms)
    }
}

/// `LevelLoadingScreen.tick`: the bar eases a fifth of the way each tick.
pub fn smooth(smoothed: f32, progress: f32) -> f32 {
    smoothed + (progress - smoothed) * 0.2
}

/// `LoadingOverlay.extractRenderState`: each frame the bar moves 5% of the
/// way to the reload's actual progress, so a jump in stages slides in.
pub fn ease_overlay(current: f32, actual: f32) -> f32 {
    (current * 0.95 + actual * 0.050000012).clamp(0.0, 1.0)
}

/// `ARGB.opaque` as the batch's RGBA.
pub const fn opaque(rgb: u32) -> u32 {
    0xFF00_0000 | (rgb & 0xFF) << 16 | (rgb & 0xFF00) | (rgb >> 16 & 0xFF)
}

/// `LoadingOverlay.LOGO_BACKGROUND_COLOR` (RGB: 239, 50, 61).
pub const LOGO_BACKGROUND_COLOR: u32 = opaque(0xEF323D);
/// `LoadingOverlay.LOGO_BACKGROUND_COLOR_DARK` (black).
pub const LOGO_BACKGROUND_COLOR_DARK: u32 = opaque(0x000000);

/// `LoadingOverlay.BRAND_BACKGROUND`: black when `darkMojangStudiosBackground` is set,
/// Mojang red otherwise.
pub fn brand_background(dark: bool) -> u32 {
    if dark {
        LOGO_BACKGROUND_COLOR_DARK
    } else {
        LOGO_BACKGROUND_COLOR
    }
}

/// `LevelLoadingScreen.extractRenderState` over its background: the text,
/// the progress bar and the chunk map, `status(x, z)` for x and z in
/// `0..=2 * STATUS_RADIUS` from the map's north-west corner.
pub fn draw(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    status: Option<&dyn Fn(i32, i32) -> Option<Status>>,
    progress: Option<f32>,
) {
    let (x_center, y_center) = ((batch.width as i32) / 2, (batch.height as i32) / 2);
    let text_top = match status {
        Some(status) => {
            // extractChunksForRendering(size 2, margin 0).
            let diameter = STATUS_RADIUS * 2 + 1;
            let start = (x_center - diameter, y_center - diameter);
            for x in 0..diameter {
                for z in 0..diameter {
                    let color = opaque(status(x, z).map_or(0, Status::color));
                    let (cx, cy) = (start.0 + x * 2, start.1 + z * 2);
                    batch.fill(atlas, cx as f32, cy as f32, 2.0, 2.0, color);
                }
            }
            y_center - STATUS_RADIUS * 2 - 9 * 3
        }
        None => y_center - 50,
    };
    let text = Text::translate("multiplayer.downloadingTerrain", &[]);
    let tw = atlas.font.width_text(&text) as i32;
    batch.text_styled(
        atlas,
        &text,
        (x_center - tw / 2) as f32,
        text_top as f32,
        u32::MAX,
        true,
    );
    if let Some(progress) = progress {
        // drawProgressBar(xCenter - 100, textTop + 12, 200, 2).
        let (left, top) = ((x_center - 100) as f32, (text_top + 9 + 3) as f32);
        batch.fill(atlas, left, top, 200.0, 2.0, opaque(0));
        let done = (progress * 200.0).round();
        batch.fill(atlas, left, top, done, 2.0, opaque(0x00FF00));
    }
}

/// `LoadingOverlay.MOJANG_STUDIOS_LOGO_LOCATION`, from the player's own resources.
pub const MOJANG_STUDIOS_LOGO: &str = "gui/title/mojangstudios";

/// `LoadingOverlay.extractRenderState`'s logo on a GUI `w` x `h`: the left
/// half's corner, then each half's width and height.
pub fn logo_layout(w: f32, h: f32) -> (i32, i32, i32, i32) {
    let (w, h) = (f64::from(w), f64::from(h));
    let (content_x, logo_y) = ((w * 0.5) as i32, (h * 0.5) as i32);
    let logo_height = (w * 0.75).min(h) * 0.25;
    let half_height = (logo_height * 0.5) as i32;
    let half_width = (logo_height * 4.0 * 0.5) as i32;
    (
        content_x - half_width,
        logo_y - half_height,
        half_width,
        logo_height as i32,
    )
}

/// `GuiGraphicsExtractor.fill(x0, y0, x1, y1, color)`: the corners in either order.
fn fill_corners(batch: &mut HudBatch, atlas: &HudAtlas, [x0, y0, x1, y1]: [i32; 4], color: u32) {
    let (left, right) = (x0.min(x1), x0.max(x1));
    let (top, bottom) = (y0.min(y1), y0.max(y1));
    let (w, h) = ((right - left) as f32, (bottom - top) as f32);
    batch.fill(atlas, left as f32, top as f32, w, h, color);
}

/// How far `LoadingOverlay` has faded: the background's alpha, the logo's
/// opacity, and the bar's, which is gone once the fade-out's first second ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayFade {
    pub background: u8,
    pub logo: f32,
    pub bar: Option<f32>,
}
impl OverlayFade {
    /// The startup overlay, which does not fade in.
    pub const OPAQUE: Self = Self {
        background: 255,
        logo: 1.0,
        bar: Some(1.0),
    };
    /// `LoadingOverlay.extractRenderState` for an overlay that fades in
    /// (`fadeIn`): `fade_in_ms` since its first frame (`fadeInStart`),
    /// `fade_out_ms` since the reload finished (`fadeOutStart`). It fades in
    /// over half a second, then on finishing fades its bar out for a second
    /// and itself for another; `None` once gone (`setOverlay(null)`).
    pub fn at(fade_in_ms: u64, fade_out_ms: Option<u64>) -> Option<Self> {
        let fade_out = fade_out_ms.map_or(-1.0, |ms| ms as f32 / 1000.0);
        if fade_out >= 2.0 {
            return None;
        }
        let fade_in = fade_in_ms as f32 / 500.0;
        let (background, logo) = if fade_out >= 1.0 {
            let logo = 1.0 - (fade_out - 1.0).clamp(0.0, 1.0);
            ((logo * 255.0).ceil() as u8, logo)
        } else {
            let background = (f64::from(fade_in).clamp(0.15, 1.0) * 255.0).ceil() as u8;
            (background, fade_in.clamp(0.0, 1.0))
        };
        let bar = (fade_out < 1.0).then(|| 1.0 - fade_out.clamp(0.0, 1.0));
        Some(Self {
            background,
            logo,
            bar,
        })
    }
    /// Whether the background hides the frame under it.
    pub fn opaque(&self) -> bool {
        self.background == 255
    }
}

/// White with `alpha` in the batch's RGBA.
fn white(alpha: u8) -> u32 {
    0x00FF_FFFF | u32::from(alpha) << 24
}

/// `LoadingOverlay.extractRenderState`: fills the frame with `BRAND_BACKGROUND`
/// (Mojang red, or black when `darkMojangStudiosBackground` is set), draws the
/// Mojang Studios logo as its two halves side by side, and the progress bar
/// at 83.25% of the height (`extractProgressBar`), each as faded as `fade`
/// says. The logo's `MOJANG_LOGO` pipeline adds it (`SRC_ALPHA, ONE`):
/// its texels are white, which alpha blending matches but for the red channel
/// of the logo's soft edges, a 16/255 difference at most.
pub fn draw_loading_overlay(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    progress: f32,
    fade: OverlayFade,
    dark: bool,
) {
    let (w, h) = (batch.width, batch.height);
    let background = brand_background(dark) & 0x00FF_FFFF | u32::from(fade.background) << 24;
    batch.fill(atlas, 0.0, 0.0, w, h, background);
    let (x, y, half_width, logo_height) = logo_layout(w, h);
    if let Some(r) = atlas.sprites.get(MOJANG_STUDIOS_LOGO).copied() {
        // blit(.., u, v, width, height, 120, 60, 120, 120): the left half
        // samples the texture's top half from u = -0.0625, the right half its
        // bottom half from u = 0.0625 (clamped, as the logo's sampler is).
        let at = |u: f32, v: f32| {
            let (u, v) = ((u / 120.0).clamp(0.0, 1.0), v / 120.0);
            (r.u0 + (r.u1 - r.u0) * u, r.v0 + (r.v1 - r.v0) * v)
        };
        // `ARGB.white(logoAlpha)`.
        let color = white((fade.logo * 255.0).floor() as u8);
        for (i, (u, v)) in [(-0.0625, 0.0), (0.0625, 60.0)].into_iter().enumerate() {
            let (u0, v0) = at(u, v);
            let (u1, v1) = at(u + 120.0, v + 60.0);
            let half = crate::hud::Rect {
                u0,
                v0,
                u1,
                v1,
                ..r
            };
            let left = x + i as i32 * half_width;
            let (lw, lh) = (half_width as f32, logo_height as f32);
            batch.rect_uv(left as f32, y as f32, lw, lh, half, color);
        }
    }
    let Some(bar) = fade.bar else {
        return;
    };
    let bar_y = (f64::from(h) * 0.8325) as i32;
    let x_center = (w as i32) / 2;
    let [x0, y0, x1, y1] = [
        x_center - half_width,
        bar_y - 5,
        x_center + half_width,
        bar_y + 5,
    ];
    let filled = (((x1 - x0) - 2) as f32 * progress.clamp(0.0, 1.0)).ceil() as i32;
    let color = white((bar * 255.0).round() as u8);
    fill_corners(batch, atlas, [x0 + 2, y0 + 2, x0 + filled, y1 - 2], color);
    fill_corners(batch, atlas, [x0 + 1, y0, x1 - 1, y0 + 1], color);
    fill_corners(batch, atlas, [x0 + 1, y1, x1 - 1, y1 - 1], color);
    fill_corners(batch, atlas, [x0, y0, x0 + 1, y1], color);
    fill_corners(batch, atlas, [x1, y0, x1 - 1, y1], color);
}

/// Mods' startup bars under the stage bar, newest first, each with its label above it, as many
/// as fit and at most four; and their lines in the top left corner (NeoForge's
/// `StartupNotificationManager`). Message lines are cut to one line; labels are stored verbatim.
pub fn draw_mod_progress(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    bars: &[(String, f32)],
    lines: &[String],
) {
    let (w, h) = (batch.width, batch.height);
    let (_, _, half_width, _) = logo_layout(w, h);
    let x_center = (w as i32) / 2;
    let white = u32::MAX;
    let track = crate::hud::rgba(255, 255, 255, 64);
    // Under the stage bar's bottom edge: a 9 px label and a 3 px bar for each mod bar.
    let mut top = (f64::from(h) * 0.8325) as i32 + 8;
    for (label, done) in bars.iter().take(4) {
        if top + 13 > h as i32 {
            break;
        }
        let label = crate::hud_kit::fit::ellipsize(atlas, label, (2 * half_width) as f32);
        let text = Text::literal(label.into_owned());
        let tw = atlas.font.width_text(&text) as i32;
        let left = (x_center - tw / 2) as f32;
        batch.text_styled(atlas, &text, left, top as f32, white, true);
        let [x0, y0, x1, y1] = [
            x_center - half_width,
            top + 10,
            x_center + half_width,
            top + 13,
        ];
        let filled = ((x1 - x0) as f32 * done.clamp(0.0, 1.0)).ceil() as i32;
        fill_corners(batch, atlas, [x0, y0, x1, y1], track);
        fill_corners(batch, atlas, [x0, y0, x0 + filled, y1], white);
        top += 16;
    }
    for (row, line) in lines.iter().enumerate() {
        let line = crate::hud_kit::fit::ellipsize(atlas, line, w - 8.0);
        let top = 4.0 + row as f32 * 10.0;
        batch.text_styled(
            atlas,
            &Text::literal(line.into_owned()),
            4.0,
            top,
            white,
            true,
        );
    }
}

/// The startup loading screen: the stage's line and a progress bar, centred
/// like `LevelLoadingScreen`'s text and bar.
pub fn draw_progress(batch: &mut HudBatch, atlas: &HudAtlas, line: &str, progress: f32) {
    let (x_center, y_center) = ((batch.width as i32) / 2, (batch.height as i32) / 2);
    let text_top = y_center - 50;
    let text = Text::literal(line);
    let tw = atlas.font.width_text(&text) as i32;
    batch.text_styled(
        atlas,
        &text,
        (x_center - tw / 2) as f32,
        text_top as f32,
        u32::MAX,
        true,
    );
    let (left, top) = ((x_center - 100) as f32, (text_top + 9 + 3) as f32);
    batch.fill(atlas, left, top, 200.0, 2.0, opaque(0));
    let done = (progress.clamp(0.0, 1.0) * 200.0).round();
    batch.fill(atlas, left, top, done, 2.0, opaque(0x00FF00));
}

/// `ConnectScreen`'s Cancel button corner on a GUI `w` x `h`:
/// (width / 2 - 100, height / 4 + 120 + 12), 200 x 20.
pub fn connect_cancel(w: f32, h: f32) -> (f32, f32) {
    ((w / 2.0).trunc() - 100.0, (h / 4.0).trunc() + 132.0)
}

/// `ConnectScreen.extractRenderState` over the menu background: the status
/// centred at (width / 2, height / 2 - 50) and, while the connection can
/// still be cancelled, Cancel.
pub fn draw_connecting(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    status: &Text,
    cursor: (f32, f32),
    cancel: bool,
) {
    let (w, h) = (batch.width, batch.height);
    crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
    let tw = atlas.font.width_text(status);
    let (x, y) = (((w - tw) / 2.0).trunc(), (h / 2.0).trunc() - 50.0);
    batch.text_styled(atlas, status, x, y, u32::MAX, true);
    if cancel {
        let (x, y) = connect_cancel(w, h);
        let hover = crate::ui::hovered(cursor, x, y, 200.0, 20.0);
        let label = Text::translate("gui.cancel", &[]);
        crate::ui::button(batch, atlas, x, y, 200.0, 20.0, &label, true, hover);
    }
}

/// `FileFixerProgressScreen`'s Cancel button corner on a GUI `w` x `h`:
/// ((width - 200) / 2, height / 2 + 100), 200 x 20.
pub fn file_fixer_cancel(w: f32, h: f32) -> (f32, f32) {
    (((w as i32 - 200) / 2) as f32, (h as i32 / 2 + 100) as f32)
}

/// `GuiGraphicsExtractor.centeredText` with the screen's grey (`-6250336`) or white.
fn centered_line(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &Text,
    x_center: i32,
    y: i32,
    color: u32,
) {
    let tw = atlas.font.width_text(text) as i32;
    batch.text_styled(
        atlas,
        text,
        (x_center - tw / 2) as f32,
        y as f32,
        color,
        true,
    );
}

/// `FileFixerProgressScreen.extractProgressBar`.
fn file_fixer_bar(batch: &mut HudBatch, atlas: &HudAtlas, x_center: i32, y: i32, progress: f32) {
    let left = x_center - 100;
    batch.fill(atlas, left as f32, y as f32, 200.0, 2.0, opaque(0));
    batch.fill(
        atlas,
        left as f32,
        y as f32,
        (progress * 200.0).round(),
        2.0,
        opaque(0x00FF00),
    );
}

/// `FileFixerProgressScreen.extractRenderState` (with `extractProgress`,
/// `extractProgressBar`, `extractFileStats`, `extractFileFixerCount`,
/// `extractTypeText`, `extractTypeProgress` and `extractScanning`) over the menu background: the
/// title, then either the scanning line or the running fixer's bar, the
/// totals, the stage, the type's label and its bar and percentage; and
/// Cancel, inactive once pressed.
pub fn draw_file_fixer(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    progress: &sunrise_world::datafix::progress::UpgradeProgress,
    cursor: (f32, f32),
) {
    const GREY: u32 = 0xFFA0_A0A0;
    let (w, h) = (batch.width, batch.height);
    crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
    let (x_center, y_center) = ((w as i32) / 2, (h as i32) / 2);
    let text_top = y_center - 50;
    centered_line(
        batch,
        atlas,
        &Text::translate("upgradeWorld.title", &[]),
        x_center,
        text_top,
        u32::MAX,
    );
    let total = progress.total_file_fix_stats();
    if total.total_operations() > 0 {
        let of_type = progress.type_file_fix_stats();
        let running = progress.running_file_fixer_stats();
        let mut y = text_top + 9 + 3;
        file_fixer_bar(batch, atlas, x_center, y, running.progress());
        y += 7;
        // extractFileStats
        let converted = Text::tr(
            "upgradeWorld.info.converted",
            &[&total.finished_operations()],
        );
        centered_line(batch, atlas, &converted, x_center, y, GREY);
        let sum = Text::tr("upgradeWorld.info.total", &[&total.total_operations()]);
        centered_line(batch, atlas, &sum, x_center, y + 9 + 3, GREY);
        y += 9 * 2 + 6;
        // extractFileFixerCount
        let stage = Text::tr(
            "upgradeWorld.info.file_fix_stage",
            &[&running.finished_operations(), &running.total_operations()],
        );
        centered_line(batch, atlas, &stage, x_center, y, GREY);
        y += 9 + 30 - 5;
        // extractTypeText
        if let Some(ty) = progress.upgrade_type() {
            let label = Text::translate(&ty.label_key(), &[]);
            centered_line(batch, atlas, &label, x_center, y, GREY);
        }
        y += 9 + 3;
        file_fixer_bar(batch, atlas, x_center, y, of_type.progress());
        y += 7;
        // extractTypeProgress: Mth.floor(progress * 100)
        let percent = (of_type.progress() * 100.0).floor() as i32;
        let text = Text::tr("upgradeWorld.progress.percentage", &[&percent]);
        centered_line(batch, atlas, &text, x_center, y, GREY);
    } else {
        // extractScanning
        let scanning = Text::translate("upgradeWorld.info.scanning", &[]);
        centered_line(batch, atlas, &scanning, x_center, text_top + 9 + 3, GREY);
    }
    let (x, y) = file_fixer_cancel(w, h);
    let active = !progress.is_canceled();
    let hover = crate::ui::hovered(cursor, x, y, 200.0, 20.0);
    let label = Text::translate("gui.cancel", &[]);
    crate::ui::button(batch, atlas, x, y, 200.0, 20.0, &label, active, hover);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_bars_draw_under_the_stage_bar_at_most_four() {
        let atlas = HudAtlas::build(&crate::resource_source::ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let drawn = |bars: usize, lines: &[String]| {
            let mut batch = HudBatch::scratch([854, 4000]);
            let bars: Vec<_> = (0..bars).map(|i| (format!("bar {i}"), 0.5)).collect();
            draw_mod_progress(&mut batch, &atlas, &bars, lines);
            batch.vertices.len()
        };
        assert_eq!(drawn(0, &[]), 0);
        assert!(drawn(1, &[]) > 0);
        assert!(drawn(2, &[]) > drawn(1, &[]));
        // A fifth bar at once is not drawn.
        assert_eq!(drawn(5, &[]), drawn(4, &[]));
        // A short screen draws only the bars that fit.
        let mut batch = HudBatch::scratch([854, 480]);
        let bars = vec![("bar".to_owned(), 1.0); 4];
        draw_mod_progress(&mut batch, &atlas, &bars, &[]);
        let bottom = batch.vertices.iter().map(|v| v.pos[1]).fold(0.0, f32::max);
        assert!(bottom <= batch.height, "{bottom} > {}", batch.height);
    }

    #[test]
    fn the_mojang_logo_is_a_quarter_of_the_shorter_side_and_four_times_as_wide() {
        // 320x240: min(240, 240) / 4 = 60 high, two halves of 120.
        assert_eq!(logo_layout(320.0, 240.0), (40, 90, 120, 60));
        // A tall frame sizes the logo by three quarters of its width.
        assert_eq!(logo_layout(400.0, 500.0), (50, 213, 150, 75));
    }

    #[test]
    fn progress_weighs_the_prepared_server_and_the_player_chunks() {
        assert_eq!(progress(0), 10.0 / 59.0);
        assert_eq!(progress(49), 1.0);
        assert_eq!(progress(100), 1.0);
        let eased = smooth(0.0, 1.0);
        assert_eq!(eased, 0.2);
        assert!((smooth(eased, 1.0) - 0.36).abs() < 1e-6);
    }

    #[test]
    fn the_overlay_bar_slides_toward_a_jump_in_progress() {
        // A stage jump from 0 to 1/3: the first frame moves 5% of the way.
        let first = ease_overlay(0.0, 1.0 / 3.0);
        assert!((first - 1.0 / 60.0).abs() < 1e-6, "{first}");
        // After 60 frames it covers 1 - 0.95^60, about 95% of the jump.
        let mut bar = 0.0;
        for _ in 0..60 {
            bar = ease_overlay(bar, 1.0);
        }
        assert!((bar - 0.953_930).abs() < 1e-4, "{bar}");
        assert_eq!(ease_overlay(1.0, 1.0), 1.0);
    }

    #[test]
    fn the_reload_overlay_fades_in_then_its_bar_then_itself() {
        let fade = |background, logo, bar| {
            Some(OverlayFade {
                background,
                logo,
                bar,
            })
        };
        // From 15% over the frame to opaque in half a second, the logo from nothing.
        assert_eq!(OverlayFade::at(0, None), fade(39, 0.0, Some(1.0)));
        assert_eq!(OverlayFade::at(250, None), fade(128, 0.5, Some(1.0)));
        assert_eq!(OverlayFade::at(600, None), fade(255, 1.0, Some(1.0)));
        // Finished: the bar fades out over a second on the full background...
        assert_eq!(OverlayFade::at(3000, Some(500)), fade(255, 1.0, Some(0.5)));
        // ...then the background and logo over the next, without the bar.
        assert_eq!(OverlayFade::at(3000, Some(1500)), fade(128, 0.5, None));
        assert_eq!(OverlayFade::at(3000, Some(2000)), None);
        assert!(!OverlayFade::at(250, None).unwrap().opaque());
        assert!(OverlayFade::at(600, None).unwrap().opaque());
    }

    #[test]
    fn brand_background_matches_dark_mojang_studios_setting() {
        assert_eq!(brand_background(true), LOGO_BACKGROUND_COLOR_DARK);
        assert_eq!(brand_background(false), LOGO_BACKGROUND_COLOR);
        assert_eq!(LOGO_BACKGROUND_COLOR_DARK, 0xFF00_0000);
        assert_eq!(LOGO_BACKGROUND_COLOR, crate::hud::rgba(239, 50, 61, 255));
    }

    #[test]
    fn file_fixer_cancel_sits_below_the_centre() {
        assert_eq!(file_fixer_cancel(427.0, 240.0), (113.0, 220.0));
        assert_eq!(file_fixer_cancel(426.0, 241.0), (113.0, 220.0));
    }

    #[test]
    fn colours_are_opaque_rgba() {
        assert_eq!(opaque(Status::Full.color()), u32::MAX);
        assert_eq!(opaque(0), 0xFF00_0000);
        assert_eq!(opaque(0x00FF00), crate::hud::rgba(0, 255, 0, 255));
        assert_eq!(
            opaque(Status::Biomes.color()),
            crate::hud::rgba(0x80, 0xB2, 0x52, 255)
        );
    }
}

#[cfg(test)]
mod tracker_tests {
    use super::*;

    const ALIVE: PlayerCheck = PlayerCheck {
        outside_build_height: false,
        spectator: false,
        alive: true,
    };

    #[test]
    fn the_load_waits_for_the_server_then_the_players_section() {
        let mut tracker = LevelLoadTracker::new(0);
        assert!(!tracker.is_active());
        tracker.start_client_load(1_000, Reason::NetherPortal);
        assert!(tracker.is_active() && tracker.reason == Reason::NetherPortal);
        // Nothing is asked of the renderer before the start event, and a
        // section shown early counts for nothing.
        assert!(!tracker.wants_player_section());
        tracker.player_section_ready();
        tracker.tick_client_load(1_050, ALIVE);
        assert!(!tracker.is_level_ready(1_050));
        tracker.loading_packets_received();
        assert!(tracker.wants_player_section());
        tracker.tick_client_load(1_100, ALIVE);
        assert!(!tracker.is_level_ready(1_100));
        tracker.player_section_ready();
        tracker.tick_client_load(1_150, ALIVE);
        assert!(tracker.is_level_ready(1_150));
        tracker.finish();
        assert!(!tracker.is_active() && !tracker.is_level_ready(2_000));
    }

    #[test]
    fn without_a_renderer_the_section_shows_once_it_has_faded_in_to_three_tenths() {
        // RenderSection.getVisibility(now, 750) >= 0.3: 225 ms after the upload.
        const AT: [i32; 3] = [0, 4, 0];
        let mut tracker = LevelLoadTracker::new(0);
        tracker.start_client_load(0, Reason::EndPortal);
        tracker.loading_packets_received();
        tracker.player_section_compiled(AT, true, 1_000, 750);
        tracker.player_section_compiled(AT, true, 1_224, 750);
        tracker.tick_client_load(1_224, ALIVE);
        assert!(!tracker.is_level_ready(1_224));
        tracker.player_section_compiled(AT, true, 1_225, 750);
        tracker.tick_client_load(1_225, ALIVE);
        assert!(tracker.is_level_ready(1_225));
        // With the fade off, at once.
        let mut tracker = LevelLoadTracker::new(0);
        tracker.start_client_load(0, Reason::Other);
        tracker.loading_packets_received();
        tracker.player_section_compiled(AT, true, 500, 0);
        tracker.tick_client_load(500, ALIVE);
        assert!(tracker.is_level_ready(500));
    }

    #[test]
    fn without_a_renderer_a_new_section_does_not_take_the_old_ones_age() {
        let mut tracker = LevelLoadTracker::new(0);
        tracker.start_client_load(0, Reason::EndPortal);
        tracker.loading_packets_received();
        tracker.player_section_compiled([0, 4, 0], true, 1_000, 750);
        // Teleported one section up, then 100 chunks away, while waiting.
        tracker.player_section_compiled([0, 5, 0], true, 1_200, 750);
        tracker.player_section_compiled([100, 5, 0], true, 1_300, 750);
        tracker.tick_client_load(1_300, ALIVE);
        assert!(!tracker.is_level_ready(1_300));
        tracker.player_section_compiled([100, 5, 0], true, 1_524, 750);
        tracker.tick_client_load(1_524, ALIVE);
        assert!(!tracker.is_level_ready(1_524));
        // A missing neighbour: no longer compiled, so its fade starts again.
        tracker.player_section_compiled([100, 5, 0], false, 1_525, 750);
        tracker.player_section_compiled([100, 5, 0], true, 1_600, 750);
        tracker.player_section_compiled([100, 5, 0], true, 1_824, 750);
        tracker.tick_client_load(1_824, ALIVE);
        assert!(!tracker.is_level_ready(1_824));
        tracker.player_section_compiled([100, 5, 0], true, 1_825, 750);
        tracker.tick_client_load(1_825, ALIVE);
        assert!(tracker.is_level_ready(1_825));
    }

    #[test]
    fn a_player_who_cannot_see_the_section_does_not_wait_for_it() {
        for player in [
            PlayerCheck {
                outside_build_height: true,
                ..ALIVE
            },
            PlayerCheck {
                spectator: true,
                ..ALIVE
            },
            PlayerCheck {
                alive: false,
                ..ALIVE
            },
        ] {
            let mut tracker = LevelLoadTracker::new(0);
            tracker.start_client_load(0, Reason::Other);
            tracker.loading_packets_received();
            tracker.tick_client_load(10, player);
            assert!(tracker.is_level_ready(10));
        }
    }

    #[test]
    fn the_wait_times_out_after_thirty_seconds_but_only_once_the_server_has_started() {
        let mut tracker = LevelLoadTracker::new(0);
        tracker.start_client_load(0, Reason::Other);
        tracker.tick_client_load(CLIENT_WAIT_TIMEOUT_MS + 1, ALIVE);
        assert!(!tracker.is_level_ready(CLIENT_WAIT_TIMEOUT_MS + 1));
        tracker.loading_packets_received();
        tracker.tick_client_load(CLIENT_WAIT_TIMEOUT_MS, ALIVE);
        assert!(!tracker.is_level_ready(CLIENT_WAIT_TIMEOUT_MS));
        tracker.tick_client_load(CLIENT_WAIT_TIMEOUT_MS + 1, ALIVE);
        assert!(tracker.is_level_ready(CLIENT_WAIT_TIMEOUT_MS + 1));
    }

    #[test]
    fn a_new_worlds_screen_stays_for_the_close_delay() {
        let mut tracker = LevelLoadTracker::new(LEVEL_LOAD_CLOSE_DELAY_MS);
        tracker.start_client_load(0, Reason::Other);
        tracker.loading_packets_received();
        tracker.player_section_ready();
        tracker.tick_client_load(100, ALIVE);
        assert!(!tracker.is_level_ready(599));
        assert!(tracker.is_level_ready(600));
    }

    #[test]
    fn a_respawn_names_its_screen_by_the_dimensions() {
        let (overworld, nether, end) = (
            "minecraft:overworld",
            "minecraft:the_nether",
            "minecraft:the_end",
        );
        assert_eq!(
            reason_for_respawn(false, nether, overworld),
            Reason::NetherPortal
        );
        assert_eq!(
            reason_for_respawn(false, overworld, nether),
            Reason::NetherPortal
        );
        assert_eq!(reason_for_respawn(false, end, overworld), Reason::EndPortal);
        assert_eq!(reason_for_respawn(false, end, nether), Reason::NetherPortal);
        assert_eq!(
            reason_for_respawn(false, overworld, overworld),
            Reason::Other
        );
        // A death never shows a portal.
        assert_eq!(reason_for_respawn(true, nether, overworld), Reason::Other);
    }
}
