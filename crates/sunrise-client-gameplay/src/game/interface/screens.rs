//! The pause menu (PauseScreen) and death screen (DeathScreen) layouts.
use super::Screen;
use super::screen_build::{PauseInputs, PauseLayout};
use sunrise_client::{
    confirm_screen::Confirm,
    hud::{HudBatch, rgba},
    ui,
    widgets::{self, ScreenInfo},
};
use sunrise_core::nbt::Tag;
use sunrise_game_data::{
    Result,
    text::{self, ClickEvent, Text},
};
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use winit::keyboard::KeyCode;

pub(in crate::game) trait ScreenClientExt: Sized {
    fn is_pause_screen(&self) -> bool;
    fn allowed_in_portal(self) -> bool;
    fn blurs_background(self, pause_menu: bool) -> bool;
}
impl ScreenClientExt for Screen {
    /// `Screen.isPauseScreen`: whether this screen, open in a single-player
    /// world, stops the world ticking (`Gui.isPausing` -> `Minecraft.pause`).
    /// The class default is true. 26.3's false overrides are
    /// `AbstractContainerScreen` (creative and inventory), `ChatScreen`,
    /// `DeathScreen`, `GameModeSwitcherScreen`, `SocialInteractionsScreen`
    /// and `LecternScreen`; Sunrise's own screens over the world follow
    /// them. The book in hand (`BookViewScreen`, true), the win screen
    /// (`WinScreen`, `!poem`) and a dialog (`DialogScreen`, its `pause`)
    /// answer from their own state, so `Game::pauses_game` reads those where
    /// they live and `Win` is not decided here.
    fn is_pause_screen(&self) -> bool {
        match self {
            // AbstractContainerScreen.isPauseScreen.
            Self::Creative | Self::Inventory
            // ChatScreen.isPauseScreen.
            | Self::Chat
            // DeathScreen.isPauseScreen.
            | Self::Dead
            // GameModeSwitcherScreen.isPauseScreen.
            | Self::GameModeSwitcher
            // SocialInteractionsScreen.isPauseScreen.
            | Self::Social
            // No screen over the world, and a native mod's screen that draws
            // over it, leave it running as they always have.
            | Self::Playing | Self::Mod
            // WinScreen.isPauseScreen is `!poem`: Game::win_screen_pauses.
            | Self::Win => false,
            // Every other screen, and the game-master block editors
            // (StructureBlockEditScreen and its siblings), inherit Screen's
            // true.
            Self::Paused
            | Self::Options
            | Self::Stats
            | Self::Advancements
            | Self::WorldOptions
            | Self::Friends
            | Self::ServerLinks
            | Self::ConfirmLink
            | Self::DemoIntro
            | Self::DialogWarning
            | Self::CommandBlock
            | Self::BlockEditor
            | Self::DebugOptions
            | Self::HudEditor => true,
        }
    }
    /// `Screen.isAllowedInPortal`: `LocalPlayer.handlePortalTransitionEffect`
    /// leaves a screen that allows it open. The class default is
    /// `isPauseScreen`, and only `ChatScreen`, `DeathScreen` and `WinScreen`
    /// override it true (the win screen whatever its poem says).
    fn allowed_in_portal(self) -> bool {
        matches!(self, Self::Chat | Self::Dead | Self::Win) || self.is_pause_screen()
    }
    /// Screen.extractBackground for non-isInGameUi screens, and
    /// PauseScreen.extractBackground only when showPauseMenu is true.
    fn blurs_background(self, pause_menu: bool) -> bool {
        match self {
            Self::Paused => pause_menu,
            Self::Options
            | Self::Stats
            | Self::Advancements
            | Self::WorldOptions
            | Self::Friends
            | Self::Social
            | Self::ServerLinks
            | Self::ConfirmLink
            | Self::DemoIntro
            | Self::DialogWarning
            | Self::DebugOptions => true,
            Self::Playing
            | Self::GameModeSwitcher
            | Self::Chat
            | Self::Creative
            | Self::Inventory
            | Self::Dead
            | Self::CommandBlock
            | Self::BlockEditor
            | Self::Mod
            // The HUD editor shows the HUD it moves.
            | Self::HudEditor => false,
            // WinScreen.extractBackground: the poem fills the screen with the End portal, the
            // credits alone blur. Game::menu_blur knows which one rolls.
            Self::Win => false,
        }
    }
}

/// The pause menu grid's origin: GridLayout of two 98px columns (padding 4),
/// aligned at (0.5, 0.25). Extra rows hold a server dialog or links
/// (`PauseScreen.getCustomAdditions`) or registered pages.
fn pause_grid(w: f32, h: f32, extra_rows: u8) -> (f32, f32) {
    let (grid_w, grid_h) = (212.0, 166.0 + 24.0 * f32::from(extra_rows));
    (((w - grid_w) * 0.5).floor(), ((h - grid_h) * 0.25).floor())
}

/// PauseScreen buttons: (x, y, width, label key, action id, active).
/// `links`: the `minecraft:server_links` dialog's button, which
/// `PauseScreen.getCustomAdditions` falls back to when `pause_screen_additions`
/// is empty and the server sent links — a full-width row that pushes the last
/// two rows down.
/// `additions`: the `pause_screen_additions` button instead of the links one
/// (vanilla shows one or the other, never both); its label is the dialog's
/// external title, so the caller draws it (id 14).
/// https://mcsrc.dev/2/26.3/net/minecraft/client/gui/screens/PauseScreen
fn pause_buttons(
    w: f32,
    h: f32,
    remote: bool,
    links: bool,
    additions: bool,
    rows: &[(&'static str, u8)],
) -> Vec<(f32, f32, f32, &'static str, u8, bool)> {
    let row3 = links || additions;
    let extra = u8::from(row3) + rows.len() as u8;
    let (x0, y0) = pause_grid(w, h, extra);
    let down = 24.0 * f32::from(extra);
    let mut v = vec![
        (x0 + 4.0, y0 + 50.0, 204.0, "menu.returnToGame", 0, true),
        (x0 + 4.0, y0 + 74.0, 98.0, "gui.advancements", 4, true),
        (x0 + 110.0, y0 + 74.0, 98.0, "gui.stats", 3, true),
    ];
    if additions {
        // addCustomDialogButtons: one full-width button, labelled with the
        // dialog's CommonDialogData.computeExternalTitle.
        v.push((x0 + 4.0, y0 + 122.0, 204.0, "", 14, true));
    } else if links {
        // CommonDialogData.computeExternalTitle of Dialogs.SERVER_LINKS.
        v.push((x0 + 4.0, y0 + 122.0, 204.0, "menu.server_links", 11, true));
    }
    for (k, &(key, id)) in rows.iter().enumerate() {
        let y = y0 + 122.0 + 24.0 * (f32::from(u8::from(row3)) + k as f32);
        v.push((x0 + 4.0, y, 204.0, key, id, true));
    }
    v.extend([
        (x0 + 4.0, y0 + 122.0 + down, 98.0, "menu.options", 1, true),
        (
            x0 + 110.0,
            y0 + 122.0 + down,
            98.0,
            "options.worldOptions.button",
            9,
            true,
        ),
        (
            x0 + 4.0,
            y0 + 146.0 + down,
            204.0,
            if remote {
                "menu.disconnect"
            } else {
                "menu.returnToMenu"
            },
            2,
            true,
        ),
    ]);
    v
}

/// The icon row: report bugs, feedback, friends, player reporting.
/// `FRIENDS_ICON` (vanilla's `CommonButtons.friends`) opens Sunrise's
/// Play with Friends; the first two open their link through
/// `ConfirmLinkScreen`; player reporting opens `SocialInteractionsScreen`.
const FRIENDS_ICON: usize = 2;
/// `CommonLinks.SNAPSHOT_BUGS_FEEDBACK`, behind `PauseScreen.REPORT_BUGS`.
const BUGS_LINK: &str = "https://aka.ms/snapshotbugs?ref=game";
/// `CommonLinks.RELEASE_FEEDBACK` / `SNAPSHOT_FEEDBACK`, by `WorldVersion.stable()`.
fn feedback_link() -> &'static str {
    if sunrise_core::version::STABLE {
        "https://aka.ms/javafeedback?ref=game"
    } else {
        "https://aka.ms/snapshotfeedback?ref=game"
    }
}
/// `PauseScreen.createPauseMenu`'s icon tooltips (`SpriteIconButton.withTootip`).
const ICON_LABELS: [&str; 4] = [
    "menu.reportBugs",
    "menu.sendFeedback",
    "gui.friends.open",
    "menu.playerReporting",
];
/// Whether an icon button is active: `reportBugsButton.active =
/// !dataVersion().isSideSeries()` (26.3 is the `main` series), the feedback
/// button always, friends when the friends list is shown.
fn icon_active(icon: usize, friends: bool) -> bool {
    match icon {
        0 | 1 | 3 => true,
        FRIENDS_ICON => friends,
        _ => false,
    }
}
fn pause_icon_focus_ids(friends: bool) -> Vec<u8> {
    (0..4)
        .filter(|&i| icon_active(i, friends))
        .map(|i| if i == FRIENDS_ICON { 10 } else { 12 + i as u8 })
        .collect()
}
/// The pause menu's first id for the pages crates registered there: id `ADDED_PAGES + i` opens
/// registered page `i`.
const ADDED_PAGES: u8 = 40;
fn pause_icons(
    w: f32,
    h: f32,
    links: bool,
    additions: bool,
    rows: &[(&'static str, u8)],
) -> [(f32, f32, &'static str); 4] {
    let (x0, y0) = pause_grid(w, h, u8::from(links || additions) + rows.len() as u8);
    let (x0, y) = (x0 + 60.0, y0 + 98.0);
    [
        (x0, y, "pause_menu/bug"),
        (x0 + 24.0, y, "pause_menu/social_interactions"),
        (x0 + 48.0, y, "friends/friends"),
        (x0 + 72.0, y, "pause_menu/player_reporting"),
    ]
}
/// DeathScreen.init: a hardcore player can only spectate.
/// `DeathScreen.TitleConfirmScreen`: its title and the yes and no labels.
fn death_confirm_texts() -> (Text, [Text; 2]) {
    let t = |key: &str| Text::translate(key, &[]);
    (
        t("deathScreen.quit.confirm"),
        [t("deathScreen.titleScreen"), t("deathScreen.respawn")],
    )
}
fn death_buttons(w: f32, h: f32, hardcore: bool) -> [(f32, f32, &'static str, u8); 2] {
    let respawn = if hardcore {
        "deathScreen.spectate"
    } else {
        "deathScreen.respawn"
    };
    // DeathScreen.init: `width / 2 - 100`, `height / 4 + 72` and `+ 96`, in
    // int GUI pixels.
    let (x, y) = ((w.ceil() / 2.0).floor() - 100.0, (h.ceil() / 4.0).floor());
    [
        (x, y + 72.0, respawn, 0),
        (x, y + 96.0, "deathScreen.titleScreen", 1),
    ]
}
/// `ActiveTextCollector`'s width of one line: every glyph's `advance`.
fn line_width(advance: &dyn Fn(char, &text::Style) -> f32, line: &Text) -> f32 {
    line.resolve_legacy()
        .chars()
        .map(|(c, style)| advance(c, style))
        .sum()
}

/// `DeathScreen.mouseClicked`'s `ClickableStyleFinder` over the cause line,
/// which `DeathScreen.visitText` draws centred at y 85: the style of the glyph
/// under `cursor`.
fn death_style_at(
    advance: &dyn Fn(char, &text::Style) -> f32,
    cause: &Text,
    width: f32,
    cursor: (f32, f32),
) -> Option<text::Style> {
    let left = ((width - line_width(advance, cause)) / 2.0).floor();
    let top = 85.0;
    if cursor.1 < top || cursor.1 >= top + 9.0 || cursor.0 < left {
        return None;
    }
    let mut x = cursor.0 - left;
    for (c, style) in cause.resolve_legacy().chars() {
        x -= advance(c, style);
        if x < 0.0 {
            return Some(style.clone());
        }
    }
    None
}
impl crate::game::ClientGame<'_> {
    /// `PauseScreen.getCustomAdditions`'s additions: the
    /// `minecraft:pause_screen_additions` tag's one dialog, or the
    /// `minecraft:custom_options` `dialog_list` when it names several. `None`
    /// when the tag is empty: the caller falls back to the server links
    /// button, as vanilla does.
    fn pause_screen_additions(&self) -> Option<(Text, Tag)> {
        let registries = &self.client.remote.as_ref()?.registries;
        let dialogs = registries.registries.get("minecraft:dialog")?;
        let tag = registries.tag("minecraft:dialog", "minecraft:pause_screen_additions")?;
        // addCustomDialogButtons' label: computeExternalTitle falls back to the title.
        let button = |dialog_tag: Tag| -> Option<(Text, Tag)> {
            let dialog = self.decode_dialog(&dialog_tag, Some(registries))?;
            let label = dialog.external_title.unwrap_or(dialog.title);
            Some((label, dialog_tag))
        };
        match tag.len() {
            0 => None,
            1 => {
                let i = usize::try_from(tag[0]).ok()?;
                button(dialogs.get(i)?.1.clone()?)
            }
            // `Dialogs.CUSTOM_OPTIONS`: the dialog list over the same tag.
            _ => button(
                dialogs
                    .iter()
                    .find(|(name, _)| name == "minecraft:custom_options")?
                    .1
                    .clone()?,
            ),
        }
    }

    /// Singleplayer also has a connection, backed by the integrated server.
    pub(in crate::game::client_side) fn external_server(&self) -> bool {
        self.client.remote.is_some() && self.client.integrated.is_none()
    }
    /// What `PauseScreen.init` reads and this screen can see change while it
    /// is open: the connection, a server's links, the custom-additions dialog
    /// and the rows the registered pages add.
    pub(in crate::game::client_side) fn pause_inputs(&self) -> PauseInputs {
        PauseInputs {
            remote: self.external_server(),
            links: !self.server_links().is_empty(),
            additions: self.pause_screen_additions().is_some(),
            rows: self.pause_rows().collect(),
        }
    }
    pub(in crate::game::client_side) fn pause_inputs_match(&self, old: &PauseInputs) -> bool {
        old.remote == self.external_server()
            && old.links == !self.server_links().is_empty()
            && old.additions == self.pause_screen_additions().is_some()
            && self.pause_rows().eq(old.rows.iter().copied())
    }
    /// The pause menu's widgets for a `ScreenBuild` at `(w, h)`: the buttons
    /// and the icon row. The build makes it once per open, resize and input
    /// change; the drawing, the click hit test and the key focus list read it.
    pub fn pause_layout(&self, w: f32, h: f32) -> PauseLayout {
        self.pause_layout_for(&self.pause_inputs(), w, h)
    }
    /// `pause_layout` from inputs already read, as the build compares them.
    pub(in crate::game::client_side) fn pause_layout_for(
        &self,
        inputs: &PauseInputs,
        w: f32,
        h: f32,
    ) -> PauseLayout {
        PauseLayout {
            inputs: inputs.clone(),
            buttons: pause_buttons(
                w,
                h,
                inputs.remote,
                inputs.links,
                inputs.additions,
                &inputs.rows,
            ),
            icons: pause_icons(w, h, inputs.links, inputs.additions, &inputs.rows),
        }
    }
    /// Native controls built by PauseScreen.init, in the grid's registration order.
    pub(in crate::game::client_side) fn pause_init_children(
        &self,
    ) -> Vec<(u8, [f32; 4], Option<mod_api::Text>)> {
        if self.client.screen != Screen::Paused || !self.client.pause_menu {
            return Vec::new();
        }
        let [w, h] = self.gui_frame_now().1.size.map(|axis| axis as f32);
        let Some(layout) = self.built_pause(w, h) else {
            return Vec::new();
        };
        let buttons = layout.buttons.iter().map(|&(x, y, bw, key, id, _)| {
            let label = if id == 14 {
                self.pause_screen_additions()
                    .and_then(|(text, _)| text.try_into_api().ok())
            } else {
                Some(mod_api::Text::translate(key, Vec::new()))
            };
            (id, [x, y, bw, 20.0], label)
        });
        let icons = layout.icons.iter().enumerate().map(|(i, &(x, y, _))| {
            let id = if i == FRIENDS_ICON { 10 } else { 12 + i as u8 };
            (
                id,
                [x, y, 20.0, 20.0],
                Some(mod_api::Text::translate(ICON_LABELS[i], Vec::new())),
            )
        });
        buttons
            .clone()
            .take(3)
            .chain(icons)
            .chain(buttons.skip(3))
            .collect()
    }

    pub(in crate::game::client_side) fn draw_pause_menu(&mut self, batch: &mut HudBatch) {
        let friends = self.friends_shown();
        let atlas = &self.client.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        let title = Text::translate("menu.game", &[]);
        widgets::begin_screen(|| ScreenInfo::new("pause", title.plain()));
        let tw = atlas.font.width_text(&title);
        batch.text_styled(
            atlas,
            &title,
            ((w - tw) / 2.0).floor(),
            40.0,
            u32::MAX,
            true,
        );
        let additions = self.pause_screen_additions();
        let fallback;
        let layout = match self.built_pause(w, h) {
            Some(layout) => layout,
            None => {
                fallback = self.pause_layout(w, h);
                &fallback
            }
        };
        let mut tooltip = None;
        let mut icon_narration_index = 0;
        for (index, &(x, y, bw, key, id, active)) in layout.buttons.iter().enumerate() {
            if self.pause_child_removed(id) {
                continue;
            }
            icon_narration_index += usize::from(index < 3);
            let is_focused = self.client.pause_focus == Some(id);
            let hover = active && (ui::hovered(self.client.cursor, x, y, bw, 20.0) || is_focused);
            let label = match (id, &additions) {
                (14, Some((label, _))) => label.clone(),
                _ => Text::translate(key, &[]),
            };
            let action = match id {
                0 => Some(sunrise_client::widgets::Action::Back),
                1 => Some(sunrise_client::widgets::Action::Options),
                2 => Some(sunrise_client::widgets::Action::Exit),
                _ => None,
            };
            if let Some(action) = action {
                ui::action_button(batch, atlas, action, x, y, bw, 20.0, &label, active, hover);
            } else {
                ui::button(batch, atlas, x, y, bw, 20.0, &label, active, hover);
            }
            // PauseScreen's customAdditions button: CUSTOM_OPTIONS_TOOLTIP.
            if (id == 11 || id == 14) && hover {
                tooltip = Some(Text::translate("menu.custom_options.tooltip", &[]));
            }
        }
        for (i, &(x, y, sprite)) in layout.icons.iter().enumerate() {
            let id = if i == FRIENDS_ICON { 10 } else { 12 + i as u8 };
            if self.pause_child_removed(id) {
                continue;
            }
            let active = icon_active(i, friends);
            let is_focused = self.client.pause_focus == Some(id);
            let hover = active && (ui::hovered(self.client.cursor, x, y, 20.0, 20.0) || is_focused);
            // The icon has no label: name it for controller focus and the
            // control socket's widget tree.
            if i == FRIENDS_ICON {
                sunrise_client::widgets::record(super::friends::TITLE, [x, y, 20.0, 20.0], active);
            }
            ui::button(
                batch,
                atlas,
                x,
                y,
                20.0,
                20.0,
                &Text::empty(),
                active,
                hover,
            );
            // PauseScreen adds the icon row after Statistics: the icons'
            // messages (their tooltips too), the friends button's own.
            widgets::last_narratable_at(icon_narration_index, |n| {
                let t = |key: &str| Text::translate(key, &[]).plain();
                let key = [
                    "menu.reportBugs",
                    "menu.sendFeedback",
                    "",
                    "menu.playerReporting",
                ][i];
                *n = if i == FRIENDS_ICON {
                    widgets::Narratable::named(t("gui.friends.open.narration"), n.rect, active)
                        .hint(t("gui.friends.open.tooltip"))
                } else {
                    widgets::Narratable::button(t(key), n.rect, active).hint(t(key))
                }
                .hovered(hover);
            });
            icon_narration_index += 1;
            // SpriteIconButton.CenteredIcon: the 15x15 sprite at
            // x + width / 2 - 15 / 2 in integer pixels, untinted even
            // when the button is inactive.
            batch.sprite_scaled(atlas, sprite, x + 3.0, y + 3.0, 15.0, 15.0, u32::MAX);
            if hover {
                tooltip = Some(if i == FRIENDS_ICON {
                    Text::literal(super::friends::title())
                } else {
                    Text::translate(ICON_LABELS[i], &[])
                });
            }
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        // SpriteIconButton.withTootip: the label as a tooltip.
        if let Some(tip) = tooltip {
            let lines: Vec<Text> = atlas
                .font
                .split_lines(&tip, batch.width)
                .into_iter()
                .map(|(line, _)| line)
                .collect();
            ui::tooltip(batch, atlas, &lines, self.client.cursor);
        }
    }
    pub fn execute_pause_action(&mut self, hit: u8) -> Result<()> {
        match hit {
            0 => {
                self.client.pause_focus = None;
                self.set_screen(Screen::Playing);
                self.capture(true);
            }
            1 => self.open_options(false),
            3 => self.open_stats(),
            4 => self.open_advancements(true),
            9 => self.open_world_options(),
            10 => self.set_screen(Screen::Friends),
            // DialogConnectionAccess.openDialog(Dialogs.SERVER_LINKS).
            11 => {
                self.client.links_ui.scroll = 0.0;
                self.set_screen(Screen::ServerLinks);
            }
            // ConfirmLinkScreen.confirmLink(this, CommonLinks...) on the pause screen.
            12 => self.confirm_trusted_link(BUGS_LINK, Screen::Paused),
            13 => self.confirm_trusted_link(feedback_link(), Screen::Paused),
            // PauseScreen: `new SocialInteractionsScreen(this)`.
            15 => self.open_social(true),
            id @ ADDED_PAGES.. => {
                let page = usize::from(id - ADDED_PAGES);
                self.open_options_at(false, sunrise_client::options_ui::Page::Added(page));
            }
            // addCustomDialogButtons: DialogConnectionAccess.openDialog of the
            // additions dialog, from the pause menu.
            14 => {
                if let Some((_, dialog)) = self.pause_screen_additions() {
                    self.client.pause_focus = None;
                    self.set_screen(Screen::Playing);
                    self.capture(true);
                    self.show_dialog(&dialog);
                }
            }
            2 => self.close_world()?,
            _ => {}
        }
        Ok(())
    }
    /// The registered full-width rows above Options, by their title.
    fn pause_rows(&self) -> impl Iterator<Item = (&'static str, u8)> {
        sunrise_hooks::pages::engine::all()
            .iter()
            .enumerate()
            .filter(|(i, page)| {
                page.place() == sunrise_hooks::pages::Place::Pause
                    && *i < usize::from(u8::MAX - ADDED_PAGES)
            })
            .map(|(i, page)| (page.title(), ADDED_PAGES + i as u8))
    }
    pub fn pause_click(&mut self) -> Result<bool> {
        let (w, h) = {
            let size = self
                .client
                .renderer
                .0
                .as_ref()
                .map_or([1, 1], |r| [r.config.width, r.config.height]);
            let b = HudBatch::new(size, self.client.settings.gui_scale_inputs());
            (b.width, b.height)
        };
        let fallback;
        let layout = match self.built_pause(w, h) {
            Some(layout) => layout,
            None => {
                fallback = self.pause_layout(w, h);
                &fallback
            }
        };
        let icons = layout.icons;
        let hit = layout
            .buttons
            .iter()
            .copied()
            .find(|b| {
                b.5 && !self.pause_child_removed(b.4)
                    && ui::hovered(self.client.cursor, b.0, b.1, b.2, 20.0)
            })
            .map(|b| b.4)
            .or_else(|| {
                // The report bugs, feedback and player reporting icons (ids 12, 13 and 15).
                [0, 1, 3]
                    .into_iter()
                    .find(|&i| {
                        !self.pause_child_removed(12 + i as u8)
                            && ui::hovered(self.client.cursor, icons[i].0, icons[i].1, 20.0, 20.0)
                    })
                    .map(|i| 12 + i as u8)
            })
            .or_else(|| {
                (self.friends_shown()
                    && !self.pause_child_removed(10)
                    && ui::hovered(
                        self.client.cursor,
                        icons[FRIENDS_ICON].0,
                        icons[FRIENDS_ICON].1,
                        20.0,
                        20.0,
                    ))
                .then_some(10)
            });
        if let Some(id) = hit {
            self.client.pause_focus = Some(id);
            self.play_audio_button_click();
            self.execute_pause_action(id)?;
            return Ok(true);
        }
        Ok(hit.is_some())
    }
    pub(in crate::game::client_side) fn pause_key(&mut self, key: KeyCode) -> Result<bool> {
        let (w, h) = {
            let size = self
                .client
                .renderer
                .0
                .as_ref()
                .map_or([1, 1], |r| [r.config.width, r.config.height]);
            let b = HudBatch::new(size, self.client.settings.gui_scale_inputs());
            (b.width, b.height)
        };
        let fallback;
        let layout = match self.built_pause(w, h) {
            Some(layout) => layout,
            None => {
                fallback = self.pause_layout(w, h);
                &fallback
            }
        };
        // The widget tuples are `Copy`: one small copy per press keeps the
        // build readable while the focus moves.
        let buttons: Vec<_> = layout
            .buttons
            .iter()
            .copied()
            .filter(|b| b.5 && !self.pause_child_removed(b.4))
            .collect();
        let icons = layout.icons;
        let mut active_ids: Vec<u8> = buttons.iter().map(|b| b.4).collect();
        let friends = self.friends_shown();
        active_ids.extend(
            pause_icon_focus_ids(friends)
                .into_iter()
                .filter(|&id| !self.pause_child_removed(id)),
        );
        if active_ids.is_empty() {
            return Ok(false);
        }
        match key {
            KeyCode::Tab => {
                let reverse = self.has_shift_down();
                let next = match self.client.pause_focus {
                    Some(cur) => {
                        if let Some(pos) = active_ids.iter().position(|&id| id == cur) {
                            if reverse {
                                if pos == 0 {
                                    active_ids.last().copied()
                                } else {
                                    Some(active_ids[pos - 1])
                                }
                            } else if pos + 1 >= active_ids.len() {
                                active_ids.first().copied()
                            } else {
                                Some(active_ids[pos + 1])
                            }
                        } else if reverse {
                            active_ids.last().copied()
                        } else {
                            active_ids.first().copied()
                        }
                    }
                    None => {
                        if reverse {
                            active_ids.last().copied()
                        } else {
                            active_ids.first().copied()
                        }
                    }
                };
                self.client.pause_focus = next;
                Ok(true)
            }
            KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let mut rects: Vec<(u8, f32, f32, f32)> =
                    buttons.iter().map(|b| (b.4, b.0, b.1, b.2)).collect();
                let friends = self.friends_shown();
                for (i, &(x, y, _)) in icons.iter().enumerate() {
                    let id = if i == FRIENDS_ICON { 10 } else { 12 + i as u8 };
                    if icon_active(i, friends) && !self.pause_child_removed(id) {
                        rects.push((id, x, y, 20.0));
                    }
                }
                let cur_id = self.client.pause_focus.unwrap_or(0);
                if let Some(cur) = rects.iter().find(|r| r.0 == cur_id) {
                    let (cx, cy) = (cur.1 + cur.3 / 2.0, cur.2 + 10.0);
                    let mut best: Option<(u8, f32)> = None;
                    for r in &rects {
                        if r.0 == cur_id {
                            continue;
                        }
                        let (rx, ry) = (r.1 + r.3 / 2.0, r.2 + 10.0);
                        let (dx, dy) = (rx - cx, ry - cy);
                        let valid = match key {
                            KeyCode::ArrowUp => dy < -4.0,
                            KeyCode::ArrowDown => dy > 4.0,
                            KeyCode::ArrowLeft => dx < -4.0,
                            KeyCode::ArrowRight => dx > 4.0,
                            _ => false,
                        };
                        if !valid {
                            continue;
                        }
                        let dist = match key {
                            KeyCode::ArrowUp | KeyCode::ArrowDown => dy.abs() + dx.abs() * 2.0,
                            KeyCode::ArrowLeft | KeyCode::ArrowRight => dx.abs() + dy.abs() * 2.0,
                            _ => f32::MAX,
                        };
                        if best.as_ref().is_none_or(|(_, d)| dist < *d) {
                            best = Some((r.0, dist));
                        }
                    }
                    if let Some((next_id, _)) = best {
                        self.client.pause_focus = Some(next_id);
                    }
                } else {
                    self.client.pause_focus = Some(active_ids[0]);
                }
                Ok(true)
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                if let Some(hit) = self.client.pause_focus.filter(|id| active_ids.contains(id)) {
                    self.play_audio_button_click();
                    self.execute_pause_action(hit)?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            _ => Ok(false),
        }
    }
    /// DeathScreen: gradient, the title at double size, the cause and score.
    pub(in crate::game::client_side) fn draw_death_screen(&mut self, batch: &mut HudBatch) {
        let atlas = &self.client.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        // fillGradient(0x60500000 -> 0xA0803030).
        let rows = h.ceil() as i32;
        for y in 0..rows {
            let t = y as f32 / h.max(1.0);
            let lerp = |a: f32, b: f32| (a + (b - a) * t) as u8;
            batch.fill(
                atlas,
                0.0,
                y as f32,
                w,
                1.0,
                rgba(
                    lerp(80.0, 128.0),
                    lerp(0.0, 48.0),
                    lerp(0.0, 48.0),
                    lerp(96.0, 160.0),
                ),
            );
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        // DeathScreen.handleExitToTitleScreen: the TitleConfirmScreen takes the
        // screen over, on the same background.
        if let Some(opened) = self.client.chat_ui.death_confirm {
            let (title, labels) = death_confirm_texts();
            // ConfirmScreen.getNarrationMessage: its title and its (empty)
            // message.
            widgets::begin_screen(|| {
                ScreenInfo::new(
                    "death/confirm",
                    widgets::join_for_narration(&[&title.plain(), ""]),
                )
            });
            let size = self.gui_size();
            let mut confirm = Confirm::new(&atlas.font, size, &title, &Text::literal(""));
            confirm.focus = self.client.death_focus.map(|f| f as usize);
            confirm.draw_active(
                batch,
                atlas,
                self.client.cursor,
                [&labels[0], &labels[1]],
                opened.elapsed().as_millis() >= 1000,
            );
            self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
            return;
        }
        let title = if self.common.level.hardcore {
            "deathScreen.title.hardcore"
        } else {
            "deathScreen.title"
        };
        let title = Text::translate(title, &[]);
        widgets::begin_screen(|| ScreenInfo::new("death", title.plain()));
        let tw = atlas.font.width_text(&title);
        ui::text_scaled(
            batch,
            atlas,
            &title,
            (w / 2.0 - tw).floor(),
            60.0,
            2.0,
            u32::MAX,
        );
        if let Some(cause) = &self.client.chat_ui.death_text {
            let cw = atlas.font.width_text(cause);
            batch.text_styled(atlas, cause, ((w - cw) / 2.0).floor(), 85.0, u32::MAX, true);
        }
        let score = Text::translate(
            "deathScreen.score.value",
            &[Text::colored(self.player_score.to_string(), text::YELLOW)],
        );
        let sw = atlas.font.width_text(&score);
        batch.text_styled(
            atlas,
            &score,
            ((w - sw) / 2.0).floor(),
            100.0,
            u32::MAX,
            true,
        );
        let active = self
            .client
            .chat_ui
            .died_at
            .is_none_or(|t| t.elapsed().as_millis() >= 1000);
        for (x, y, key, id) in death_buttons(w, h, self.common.level.hardcore) {
            let is_focused = self.client.death_focus == Some(id);
            let hover =
                active && (ui::hovered(self.client.cursor, x, y, 200.0, 20.0) || is_focused);
            ui::button(
                batch,
                atlas,
                x,
                y,
                200.0,
                20.0,
                &Text::translate(key, &[]),
                active,
                hover,
            );
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        // DeathScreen.visitText(HoveredTextEffects.TOOLTIP_AND_CURSOR): a
        // styled cause shows its hover tooltip, and a clickable one asks for
        // the hand.
        if let Some(cause) = &self.client.chat_ui.death_text
            && let Some(style) = death_style_at(
                &|c, style| atlas.font.advance(c, style),
                cause,
                w,
                self.client.cursor,
            )
        {
            if let Some(hover) = &style.hover {
                ui::tooltip(batch, atlas, &[(**hover).clone()], self.client.cursor);
            }
            if style.click.is_some() {
                widgets::request_cursor(widgets::Cursor::PointingHand);
            }
        }
    }
    /// LocalPlayer.respawn: the player comes back (the death screen's
    /// button, or at once with `immediate_respawn`).
    pub fn respawn_player(&mut self) -> Result<()> {
        self.client.death_focus = None;
        self.client.chat_ui.death_text = None;
        // LocalPlayer.respawn: KeyMapping.resetToggleKeys.
        self.reset_toggle_keys();
        self.client.vanilla_mappings.reset_toggle_keys();
        self.remote_respawn();
        Ok(())
    }
    pub(in crate::game::client_side) fn death_click(&mut self) -> Result<bool> {
        if self
            .client
            .chat_ui
            .died_at
            .is_some_and(|t| t.elapsed().as_millis() < 1000)
        {
            return Ok(false);
        }
        if let Some(opened) = self.client.chat_ui.death_confirm {
            // ConfirmScreen.setDelay: both buttons wait 20 ticks.
            if opened.elapsed().as_millis() < 1000 {
                return Ok(false);
            }
            let (title, _) = death_confirm_texts();
            let confirm = Confirm::new(
                &self.client.gui.atlas.font,
                self.gui_size(),
                &title,
                &Text::literal(""),
            );
            if let Some(yes) = confirm.answer_at(self.client.cursor) {
                self.client.death_focus = Some(if yes { 0 } else { 1 });
                self.death_confirm_answer(yes)?;
                return Ok(true);
            }
            return Ok(false);
        }
        // DeathScreen.mouseClicked first looks for a click event on the
        // cause line: an open_url link goes through the chat link flow.
        if let Some(cause) = &self.client.chat_ui.death_text {
            let atlas = &self.client.gui.atlas;
            let mid = self.gui_size().0;
            if let Some(text::Style {
                click: Some(ClickEvent::OpenUrl(url)),
                ..
            }) = death_style_at(
                &|c, style| atlas.font.advance(c, style),
                cause,
                mid,
                self.client.cursor,
            ) {
                self.click_url(url);
                return Ok(true);
            }
        }
        let size = self
            .client
            .renderer
            .0
            .as_ref()
            .map_or([1, 1], |r| [r.config.width, r.config.height]);
        let b = HudBatch::new(size, self.client.settings.gui_scale_inputs());
        let hit = death_buttons(b.width, b.height, self.common.level.hardcore)
            .into_iter()
            .find(|d| ui::hovered(self.client.cursor, d.0, d.1, 200.0, 20.0))
            .map(|d| d.3);
        if let Some(id) = hit {
            self.client.death_focus = Some(id);
        }
        // handleExitToTitleScreen: outside hardcore, Title Screen asks first.
        if hit == Some(1) && !self.common.level.hardcore {
            self.client.chat_ui.death_confirm = Some(std::time::Instant::now());
            self.play_audio_button_click();
            return Ok(true);
        }
        self.death_action(hit)?;
        Ok(hit.is_some())
    }
    pub(in crate::game::client_side) fn death_key(&mut self, key: KeyCode) -> Result<bool> {
        if self
            .client
            .chat_ui
            .died_at
            .is_some_and(|t| t.elapsed().as_millis() < 1000)
        {
            return Ok(false);
        }
        if let Some(opened) = self.client.chat_ui.death_confirm {
            if opened.elapsed().as_millis() < 1000 {
                return Ok(false);
            }
            return match key {
                KeyCode::Escape => {
                    self.play_audio_button_click();
                    self.death_confirm_answer(false)?;
                    Ok(true)
                }
                KeyCode::Tab => {
                    let reverse = self.has_shift_down();
                    self.client.death_focus = match self.client.death_focus {
                        Some(0) => Some(1),
                        Some(1) => Some(0),
                        _ => {
                            if reverse {
                                Some(1)
                            } else {
                                Some(0)
                            }
                        }
                    };
                    Ok(true)
                }
                KeyCode::ArrowLeft => {
                    self.client.death_focus = Some(0);
                    Ok(true)
                }
                KeyCode::ArrowRight => {
                    self.client.death_focus = Some(1);
                    Ok(true)
                }
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                    let yes = self.client.death_focus != Some(1);
                    self.play_audio_button_click();
                    self.death_confirm_answer(yes)?;
                    Ok(true)
                }
                _ => Ok(false),
            };
        }
        match key {
            KeyCode::Tab => {
                let reverse = self.has_shift_down();
                self.client.death_focus = match self.client.death_focus {
                    Some(0) => Some(1),
                    Some(1) => Some(0),
                    _ => {
                        if reverse {
                            Some(1)
                        } else {
                            Some(0)
                        }
                    }
                };
                Ok(true)
            }
            KeyCode::ArrowUp => {
                self.client.death_focus = Some(0);
                Ok(true)
            }
            KeyCode::ArrowDown => {
                self.client.death_focus = Some(1);
                Ok(true)
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                let Some(hit) = self.client.death_focus else {
                    return Ok(false);
                };
                self.play_audio_button_click();
                if hit == 1 && !self.common.level.hardcore {
                    self.client.chat_ui.death_confirm = Some(std::time::Instant::now());
                    self.client.death_focus = Some(0);
                    return Ok(true);
                }
                self.death_action(Some(hit))?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    /// The confirm screen's answer: yes leaves the world, no respawns
    /// (`DeathScreen.handleExitToTitleScreen`'s callback).
    pub(in crate::game::client_side) fn death_confirm_answer(&mut self, yes: bool) -> Result<()> {
        self.client.chat_ui.death_confirm = None;
        if yes {
            self.play_audio_button_click();
            self.close_world()
        } else {
            self.death_action(Some(0))
        }
    }
    /// A death screen button: 0 respawns (or spectates in hardcore), 1 leaves.
    fn death_action(&mut self, hit: Option<u8>) -> Result<()> {
        match hit {
            Some(0) => self.respawn_player()?,
            Some(1) => self.close_world()?,
            _ => {}
        }
        if hit.is_some() {
            self.play_audio_button_click();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ADDED_PAGES, Screen, ScreenClientExt as _, death_buttons, pause_buttons,
        pause_icon_focus_ids, pause_icons,
    };

    #[test]
    fn death_buttons_sit_on_whole_gui_pixels() {
        // DeathScreen.init in ints: a 427x253 GUI (1709x1009 at scale 4) puts
        // Respawn at (113, 63 + 72), not (113.5, 63.25 + 72).
        let at = |w, h| death_buttons(w, h, false).map(|b| (b.0, b.1));
        assert_eq!(at(427.0, 253.0), [(113.0, 135.0), (113.0, 159.0)]);
        assert_eq!(at(320.0, 334.0), [(60.0, 155.0), (60.0, 179.0)]);
    }

    #[test]
    fn menu_blur_matches_vanilla_screen_backgrounds() {
        for screen in [
            Screen::Options,
            Screen::DemoIntro,
            Screen::Stats,
            Screen::Advancements,
            Screen::WorldOptions,
        ] {
            assert!(screen.blurs_background(false));
            assert!(screen.blurs_background(true));
        }
        assert!(Screen::Paused.blurs_background(true));
        assert!(!Screen::Paused.blurs_background(false));
        for screen in [
            Screen::Playing,
            Screen::GameModeSwitcher,
            Screen::Chat,
            Screen::Creative,
            Screen::Inventory,
            Screen::Dead,
            Screen::CommandBlock,
            Screen::BlockEditor,
            Screen::Win,
        ] {
            assert!(!screen.blurs_background(true));
            assert!(!screen.blurs_background(false));
        }
    }

    /// `Screen.isPauseScreen` per 26.3: the class default is true, and only
    /// these classes override it false; the game-master editors keep the
    /// default. A world with one of the true screens open stops ticking
    /// (`Minecraft.pause` through `Gui.isPausing`).
    #[test]
    fn only_the_screens_that_override_is_pause_screen_keep_the_world_running() {
        for screen in [
            Screen::Creative,
            Screen::Inventory,
            Screen::Chat,
            Screen::Dead,
            Screen::GameModeSwitcher,
            Screen::Social,
            Screen::Playing,
            Screen::Mod,
            Screen::Win,
        ] {
            assert!(!screen.is_pause_screen(), "{}", screen.id());
        }
        for screen in [
            Screen::Paused,
            Screen::Options,
            Screen::Stats,
            Screen::Advancements,
            Screen::WorldOptions,
            Screen::Friends,
            Screen::ServerLinks,
            Screen::ConfirmLink,
            Screen::DemoIntro,
            Screen::DialogWarning,
            Screen::CommandBlock,
            Screen::BlockEditor,
            Screen::DebugOptions,
            Screen::HudEditor,
        ] {
            assert!(screen.is_pause_screen(), "{}", screen.id());
        }
    }

    #[test]
    fn pause_menu_uses_disconnect_for_servers_and_keeps_world_options() {
        for (remote, label) in [(false, "menu.returnToMenu"), (true, "menu.disconnect")] {
            let buttons = pause_buttons(427.0, 240.0, remote, false, false, &[]);
            let quit = buttons.iter().find(|b| b.4 == 2).unwrap();
            assert_eq!(quit.3, label);
            assert!(quit.5);
            assert!(buttons.iter().any(|b| b.4 == 9 && b.5));
        }
    }

    #[test]
    fn pause_icons_report_bugs_and_feedback_are_active_like_vanilla() {
        use super::{FRIENDS_ICON, icon_active};
        // PauseScreen.createPauseMenu: the bug and feedback buttons are live,
        // friends follows its list, player reporting opens the social screen.
        assert!(icon_active(0, false) && icon_active(1, false) && icon_active(3, false));
        assert!(icon_active(FRIENDS_ICON, true) && !icon_active(FRIENDS_ICON, false));
    }

    #[test]
    fn pause_keyboard_focus_includes_the_active_icon_buttons() {
        assert_eq!(pause_icon_focus_ids(false), [12, 13, 15]);
        assert_eq!(pause_icon_focus_ids(true), [12, 13, 10, 15]);
    }

    #[test]
    fn server_links_adds_a_full_width_row_above_options() {
        let with = pause_buttons(427.0, 240.0, true, true, false, &[]);
        let without = pause_buttons(427.0, 240.0, true, false, false, &[]);
        let links = with.iter().find(|b| b.4 == 11).unwrap();
        assert_eq!(links.2, 204.0);
        assert_eq!(links.3, "menu.server_links");
        assert!(links.5);
        // The icon row sits above it; Options and Disconnect move down 24.
        let icons = pause_icons(427.0, 240.0, true, false, &[]);
        let y0_icons = ((240.0f32 - 190.0) * 0.25).floor() + 98.0;
        assert_eq!(icons[0].1, y0_icons);
        assert!(links.1 > icons[0].1);
        for b in &with {
            if b.4 == 1 || b.4 == 9 || b.4 == 2 {
                let same = without.iter().find(|o| o.4 == b.4).unwrap();
                assert_eq!(b.1, same.1 + 24.0 - 6.0, "{}", b.3);
            }
        }
    }

    #[test]
    fn pause_screen_additions_takes_the_server_links_row() {
        // getCustomAdditions: one additions button in row 3, never next to the
        // links button, so the grid keeps one extra row either way.
        let with = pause_buttons(427.0, 240.0, true, true, true, &[]);
        let links = pause_buttons(427.0, 240.0, true, true, false, &[]);
        assert!(with.iter().all(|b| b.4 != 11));
        let button = with.iter().find(|b| b.4 == 14).unwrap();
        assert_eq!(
            (button.0, button.1, button.2, button.5),
            (links[3].0, links[3].1, 204.0, true)
        );
        for id in [1, 9, 2] {
            let add = with.iter().find(|b| b.4 == id).unwrap();
            let link = links.iter().find(|b| b.4 == id).unwrap();
            assert_eq!(add.1, link.1);
        }
        assert_eq!(
            pause_icons(427.0, 240.0, true, true, &[])[0].1,
            pause_icons(427.0, 240.0, true, false, &[])[0].1
        );
    }
    #[test]
    fn registered_pages_stack_above_options_in_the_pause_menu() {
        let rows = [
            ("test.page.a", ADDED_PAGES),
            ("test.page.b", ADDED_PAGES + 2),
        ];
        let buttons = pause_buttons(427.0, 300.0, true, false, false, &rows);
        let y = |id| buttons.iter().find(|b| b.4 == id).unwrap().1;
        assert_eq!(y(ADDED_PAGES + 2), y(ADDED_PAGES) + 24.0);
        assert_eq!(
            y(1),
            y(ADDED_PAGES + 2) + 24.0,
            "Options under the last one"
        );
        // The grid stays centred: two more rows start it higher.
        let without = pause_buttons(427.0, 300.0, true, false, false, &[]);
        let back = |b: &[(f32, f32, f32, &str, u8, bool)]| b.iter().find(|b| b.4 == 0).unwrap().1;
        assert!(back(&buttons) < back(&without));
    }
    #[test]
    fn registered_pages_preserve_dialog_rows_and_icon_actions() {
        for (links, additions) in [(false, false), (true, false), (false, true), (true, true)] {
            let buttons = pause_buttons(
                427.0,
                300.0,
                true,
                links,
                additions,
                &[("test.page", ADDED_PAGES)],
            );
            let page = buttons.iter().find(|b| b.4 == ADDED_PAGES).unwrap();
            let options = buttons.iter().find(|b| b.4 == 1).unwrap();
            assert_eq!(options.1, page.1 + 24.0);
            if links || additions {
                let custom = buttons
                    .iter()
                    .find(|b| b.4 == if additions { 14 } else { 11 })
                    .unwrap();
                assert_eq!(page.1, custom.1 + 24.0);
            }
            assert!(buttons.iter().all(|b| b.4 != 12 && b.4 != 13));
            assert!(
                !pause_buttons(427.0, 300.0, true, links, additions, &[])
                    .iter()
                    .any(|b| b.4 == ADDED_PAGES)
            );
        }
    }

    /// `DeathScreen.mouseClicked`'s `ClickableStyleFinder`: the cause line's
    /// glyph under the pointer decides the link, and the line's edges are no
    /// link.
    #[test]
    fn a_death_causes_link_is_the_glyph_under_the_pointer() {
        use sunrise_game_data::text::{ClickEvent, Style, Text};
        let link = Style {
            click: Some(ClickEvent::OpenUrl("https://example.com".into())),
            ..Style::default()
        };
        let cause = Text::literal("ab").append(Text::styled("cd", link));
        let advance = |_: char, _: &Style| 6.0;
        let at = |x: f32, y: f32| {
            super::death_style_at(&advance, &cause, 427.0, (x, y)).and_then(|s| s.click)
        };
        // Four 6-pixel glyphs centred in 427: the line starts at 201.
        assert_eq!(at(203.0, 86.0), None);
        assert_eq!(at(209.0, 86.0), None);
        assert_eq!(
            at(215.0, 86.0),
            Some(ClickEvent::OpenUrl("https://example.com".into()))
        );
        assert_eq!(
            at(221.0, 86.0),
            Some(ClickEvent::OpenUrl("https://example.com".into()))
        );
        assert_eq!(at(227.0, 86.0), None);
        assert_eq!(at(215.0, 84.0), None);
    }
}
