//! WorldOptionsScreen: the in-world options with the multiplayer section
//! that opens the world to the local network (the 26.3 replacement of the
//! old "Open to LAN" screen): default and personal game mode, cheats,
//! difficulty, LAN on/off with its port, command access for guests and
//! forcing the game mode. Its difficulty lock asks first (PopupScreen), and
//! Restrictions opens RestrictionsScreen.
//! Parity reference: https://mcsrc.dev/2/26.3/net/minecraft/client/gui/screens/WorldOptionsScreen
use super::Screen;
use sunrise_client::{
    font::Font,
    game_rules_screen::{Exit, RulesScreen},
    hud::{HudBatch, rgba},
    ui::{self, EditBox},
    widgets::{self, Narratable, join_for_narration},
};
use sunrise_game_data::text::{Text, named_color};
use sunrise_gameplay_common::game::integrated_link::ServerOptions;
use sunrise_gameplay_common::game::owner_link::OwnerRequest;
use sunrise_protocol::packets::play::{self, Serverbound as S};
use winit::keyboard::KeyCode;

pub(in crate::game) struct WorldOptions {
    remote: bool,
    gamemaster: bool,
    default_mode: i32,
    personal_mode: i32,
    cheats: bool,
    difficulty: u8,
    lan: bool,
    port: EditBox,
    /// The free port suggested while the box is empty.
    hint: u16,
    guest_commands: bool,
    force: bool,
    hardcore: bool,
    /// The level's own lock (LevelData.isDifficultyLocked).
    difficulty_locked: bool,
    /// wantedDifficultyLocked: the lock icon, from the lock or hardcore.
    locked: bool,
    initial: Snapshot,
    /// ScrollableLayout's scroll amount.
    scroll: f32,
    view: View,
    /// InWorldGameRulesScreen, while it is open.
    rules: Option<RulesScreen>,
    focused_button: Option<Button>,
    popup_focus: Option<usize>,
}
/// The screen on show: the options, the lock's question over them, or the
/// RestrictionsScreen or InWorldGameRulesScreen they opened (all return to
/// the same options).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    Options,
    LockQuestion,
    Restrictions,
    GameRules,
}
pub(in crate::game) use sunrise_gameplay_common::game::owner_link::Snapshot;

const MODES: [&str; 4] = ["survival", "creative", "adventure", "spectator"];
const DIFFICULTIES: [&str; 4] = ["peaceful", "easy", "normal", "hard"];

/// HeaderAndFooterLayout's header and footer heights.
const HEADER: f32 = 33.0;
const FOOTER: f32 = 33.0;
/// The scrolled content: both grids, each padded by 8 and 8 apart.
const CONTENT: f32 = 218.0;
/// The category headings' tops (FocusableTextWidget, padding 4).
const GENERAL_TOP: f32 = 8.0;
const MULTIPLAYER_TOP: f32 = 145.0;
/// The port box's top beside the LAN button.
const PORT_TOP: f32 = 166.0;
/// PopupScreen: 250 wide less its 18 pixel border each side, two buttons
/// `min((214 - 6) / 2, 150)` wide and 6 apart, rows 12 apart.
const POPUP_CONTENT: f32 = 214.0;
const POPUP_BUTTON: f32 = 104.0;
/// RestrictionsScreen's text widgets: 250 wide, padded by 4.
const RESTRICTION_WIDTH: f32 = 250.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Button {
    DefaultMode,
    PersonalMode,
    Cheats,
    Difficulty,
    /// DifficultyButtons.lockButton (LockIconButton).
    Lock,
    GameRules,
    Restrictions,
    Lan,
    CommandAccess,
    Force,
    Apply,
    Cancel,
}

fn on_off(on: bool) -> Text {
    Text::translate(if on { "options.on" } else { "options.off" }, &[])
}
fn labelled(key: &str, value: Text) -> Text {
    Text::translate("options.generic_value", &[Text::translate(key, &[]), value])
}

impl WorldOptions {
    /// No button keeps the focus a click gave it.
    pub(in crate::game::client_side) fn clear_click_focus(&mut self) {
        self.focused_button = None;
    }
    /// Vanilla 26.3 WorldOptionsScreen.generalOptions: guests see difficulty,
    /// game rules and restrictions; game modes, commands and LAN are host-only.
    fn visible_buttons(&self, w: f32) -> Vec<(Button, f32, f32, f32)> {
        if !self.remote {
            return Self::buttons(w, self.top());
        }
        let x = Self::left(w);
        let top = self.top();
        vec![
            (Button::Difficulty, x, top + 29.0, 130.0),
            (Button::Lock, x + 130.0, top + 29.0, 20.0),
            (Button::GameRules, x + 158.0, top + 29.0, 150.0),
            (Button::Restrictions, x, top + 53.0, 150.0),
        ]
    }
    fn content_height(&self) -> f32 {
        if self.remote { 81.0 } else { CONTENT }
    }
    fn scroll_limit(&self, h: f32) -> f32 {
        (self.content_height() - (h - HEADER - FOOTER)).max(0.0)
    }
    fn server_difficulty(&mut self, difficulty: u8, locked: bool) {
        // A server update is the new baseline, not a local edit. In particular,
        // the reply to Apply may arrive after the screen has already reopened.
        self.difficulty = difficulty;
        self.difficulty_locked = locked;
        self.locked = locked || self.hardcore;
        self.initial.difficulty = self.difficulty;
        self.initial.locked = self.locked;
    }
    /// `WorldOptionsScreen.updatePersonalGameModeButton`: refresh both value and baseline.
    fn update_personal_mode(&mut self, mode: i32) {
        self.personal_mode = mode;
        self.initial.personal_mode = mode;
    }
    /// `WorldOptionsScreen.updatePortControlsState`: the port box takes an
    /// edit only with LAN on; it stays focusable either way, so its value can
    /// be read, selected and copied while LAN is off.
    fn update_port_controls(&mut self) {
        self.port.editable = self.lan;
    }
    fn parsed_port(&self) -> Option<u16> {
        if self.port.value.is_empty() {
            return Some(self.hint);
        }
        self.port.value.parse::<u16>().ok().filter(|p| *p >= 1024)
    }

    /// `portEdit.setTooltip(PORT_INFO_TEXT)`: the port box explains itself while
    /// the cursor is over it. The box is drawn even with LAN off, so the tooltip
    /// shows there too; a remote world has no such box.
    fn port_tooltip(port: &EditBox, cursor: (f32, f32), remote: bool) -> Option<Text> {
        if remote || !ui::hovered(cursor, port.x, port.y, port.width, port.height) {
            return None;
        }
        Some(match port_error(&port.value) {
            Some(key) => Text::translate(key, &[Text::literal("1024"), Text::literal("65535")]),
            None => Text::translate("lanServer.port", &[]),
        })
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            default_mode: self.default_mode,
            personal_mode: self.personal_mode,
            cheats: self.cheats,
            difficulty: self.difficulty,
            locked: self.locked,
            lan: self.lan,
            port: if self.lan {
                self.parsed_port().unwrap_or(0)
            } else {
                0
            },
            guest_commands: self.guest_commands && self.lan && self.cheats,
            force: self.force,
        }
    }
    fn changed(&self) -> bool {
        let valid = !self.lan || self.parsed_port().is_some();
        valid && self.snapshot() != self.initial
    }
    /// The scrolled content's top: under the header, less the scroll.
    fn top(&self) -> f32 {
        HEADER - self.scroll.trunc()
    }
    /// The grids' left edge: the content (308 wide, padded by 8) centred
    /// in a ScrollableLayout reserving 10 pixels for the scrollbar each side.
    fn left(w: f32) -> f32 {
        (w / 2.0).floor() - 154.0
    }
    /// The scrolled widgets: each grid's rows 4 apart, columns 8 apart.
    fn buttons(w: f32, top: f32) -> Vec<(Button, f32, f32, f32)> {
        let x = Self::left(w);
        let right = x + 158.0;
        vec![
            (Button::DefaultMode, x, top + 29.0, 308.0),
            (Button::PersonalMode, x, top + 53.0, 308.0),
            (Button::Cheats, x, top + 77.0, 150.0),
            // DifficultyButtons: the cycle button gives the lock its 20 pixels.
            (Button::Difficulty, right, top + 77.0, 130.0),
            (Button::Lock, right + 130.0, top + 77.0, 20.0),
            (Button::GameRules, x, top + 101.0, 150.0),
            (Button::Restrictions, right, top + 101.0, 150.0),
            (Button::Lan, x, top + 166.0, 150.0),
            (Button::CommandAccess, x, top + 190.0, 150.0),
            (Button::Force, right, top + 190.0, 150.0),
        ]
    }
    /// The footer's two buttons, centred in its 33 pixels (FrameLayout
    /// truncates the 6.5 pixel offset).
    fn footer(w: f32, h: f32) -> [(Button, f32, f32, f32); 2] {
        let x = Self::left(w);
        [
            (Button::Apply, x, h - FOOTER + 6.0, 150.0),
            (Button::Cancel, x + 158.0, h - FOOTER + 6.0, 150.0),
        ]
    }
    /// The widgets under the pointer's row: scrolled ones only inside the
    /// scroll area.
    fn hit(&self, (x, y): (f32, f32), (w, h): (f32, f32)) -> Option<Button> {
        let inside = y >= HEADER && y < h - FOOTER;
        self.visible_buttons(w)
            .into_iter()
            .filter(|_| inside)
            .chain(Self::footer(w, h))
            .find(|(b, bx, by, bw)| self.active(*b) && ui::hovered((x, y), *bx, *by, *bw, 20.0))
            .map(|b| b.0)
    }
    fn label(&self, b: Button) -> Text {
        let mode = |m: i32| {
            Text::translate(
                &format!("selectWorld.gameMode.{}", MODES[m as usize & 3]),
                &[],
            )
        };
        match b {
            Button::DefaultMode => {
                labelled("options.worldOptions.game_mode", mode(self.default_mode))
            }
            Button::PersonalMode => labelled(
                "options.worldOptions.personal_game_mode",
                mode(self.personal_mode),
            ),
            Button::Cheats => labelled("selectWorld.allowCommands", on_off(self.cheats)),
            Button::Lock => Text::translate("narrator.button.difficulty_lock", &[]),
            Button::GameRules => Text::translate("editGamerule.inGame.button", &[]),
            Button::Restrictions => Text::translate("restrictions_screen.button", &[]),
            Button::Difficulty => labelled(
                "options.difficulty",
                Text::translate(
                    &format!(
                        "options.difficulty.{}",
                        DIFFICULTIES[usize::from(self.difficulty) & 3]
                    ),
                    &[],
                ),
            ),
            Button::Lan => labelled("menu.multiplayerOptions.lan", on_off(self.lan)),
            Button::CommandAccess => labelled(
                "options.worldOptions.guest.command_access",
                on_off(self.guest_commands && self.lan && self.cheats),
            ),
            Button::Force => labelled(
                "options.worldOptions.guest.force_game_mode",
                on_off(self.force || !self.lan || self.hardcore),
            ),
            Button::Apply => Text::translate("menu.multiplayerOptions.applyChanges", &[]),
            Button::Cancel => Text::translate("gui.cancel", &[]),
        }
    }
    /// What button `b` narrates: the switches are `CycleButton`s ("<name>:
    /// <value> button" and the value a click moves to), the lock a
    /// `LockIconButton` (its message, then whether it is locked).
    fn narration(&self, b: Button, rect: [f32; 4], tr: super::screen_info::Tr<'_>) -> Narratable {
        let active = self.active(b);
        let mode = |m: i32| {
            tr(
                &format!("selectWorld.gameMode.{}", MODES[m as usize & 3]),
                &[],
            )
        };
        let on_off = |on: bool| tr(if on { "options.on" } else { "options.off" }, &[]);
        let labelled =
            |key: &str, value: String| tr("options.generic_value", &[&tr(key, &[]), &value]);
        let difficulty = |d: u8| {
            let name = DIFFICULTIES[usize::from(d) & 3];
            labelled(
                "options.difficulty",
                tr(&format!("options.difficulty.{name}"), &[]),
            )
        };
        let cycle = |message: String, next: String| Narratable::cycle(message, next, rect, active);
        let shown_access = self.guest_commands && self.lan && self.cheats;
        let shown_force = self.force || !self.lan || self.hardcore;
        match b {
            Button::DefaultMode => {
                let key = "options.worldOptions.game_mode";
                cycle(
                    labelled(key, mode(self.default_mode)),
                    labelled(key, mode(self.default_mode + 1)),
                )
            }
            Button::PersonalMode => {
                let key = "options.worldOptions.personal_game_mode";
                cycle(
                    labelled(key, mode(self.personal_mode)),
                    labelled(key, mode(self.personal_mode + 1)),
                )
            }
            Button::Cheats => {
                let key = "selectWorld.allowCommands";
                cycle(
                    labelled(key, on_off(self.cheats)),
                    labelled(key, on_off(!self.cheats)),
                )
            }
            Button::Difficulty => cycle(
                difficulty(self.difficulty),
                difficulty(self.difficulty.wrapping_add(1)),
            ),
            Button::Lan => {
                let key = "menu.multiplayerOptions.lan";
                cycle(
                    labelled(key, on_off(self.lan)),
                    labelled(key, on_off(!self.lan)),
                )
            }
            Button::CommandAccess => {
                let key = "options.worldOptions.guest.command_access";
                cycle(
                    labelled(key, on_off(shown_access)),
                    labelled(key, on_off(!shown_access)),
                )
            }
            Button::Force => {
                let key = "options.worldOptions.guest.force_game_mode";
                cycle(
                    labelled(key, on_off(shown_force)),
                    labelled(key, on_off(!shown_force)),
                )
            }
            Button::Lock => {
                let message = tr(
                    "gui.narrate.button",
                    &[&tr("narrator.button.difficulty_lock", &[])],
                );
                let state = if self.locked {
                    "narrator.button.difficulty_lock.locked"
                } else {
                    "narrator.button.difficulty_lock.unlocked"
                };
                Narratable::named(
                    join_for_narration(&[&message, &tr(state, &[])]),
                    rect,
                    active,
                )
            }
            _ => Narratable::button(self.label(b).plain(), rect, active),
        }
    }
    /// WorldOptionsScreen.updateButton and the update*Button methods.
    fn active(&self, b: Button) -> bool {
        if self.remote {
            return match b {
                Button::Difficulty | Button::Lock => {
                    self.gamemaster && !self.hardcore && !self.difficulty_locked
                }
                Button::GameRules => self.gamemaster,
                Button::Restrictions | Button::Cancel => true,
                Button::Apply => self.gamemaster && self.changed(),
                _ => false,
            };
        }
        match b {
            Button::DefaultMode | Button::PersonalMode => self.cheats && !self.hardcore,
            // An owner's commands come with Allow Commands.
            Button::Cheats => !self.hardcore || self.initial.cheats,
            // DifficultyButtons.updateDifficultyButtonsState: the level's
            // lock or hardcore, not the wanted lock.
            Button::Difficulty | Button::Lock => !self.hardcore && !self.difficulty_locked,
            // updateButton from the player's permission: the Allow Commands
            // switch leaves it (updatePermissionDependentButtons(.., false)).
            Button::GameRules => !self.hardcore && self.initial.cheats,
            Button::Restrictions => true,
            Button::CommandAccess => self.lan && self.cheats,
            Button::Force => self.lan && !(self.guest_commands && self.cheats) && !self.hardcore,
            Button::Apply => self.changed(),
            Button::Lan | Button::Cancel => true,
        }
    }
}

/// MultiLineLabel: the lines of `t` wrapped to `width`.
fn wrap(font: &Font, t: &Text, width: f32) -> Vec<Text> {
    font.split_lines(t, width)
        .into_iter()
        .map(|(line, _)| line)
        .collect()
}
fn widest(font: &Font, lines: &[Text]) -> f32 {
    lines.iter().map(|l| font.width_text(l)).fold(0.0, f32::max)
}
/// Centred MultiLineTextWidget lines: each centred on `mid` from `top`, 9 apart.
fn centred_lines(font: &Font, lines: Vec<Text>, mid: f32, top: f32) -> Vec<(Text, f32, f32)> {
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let x = mid - (font.width_text(&line) / 2.0).trunc();
            (line, x, top + 9.0 * i as f32)
        })
        .collect()
}

/// PopupScreen centred on the screen: its title (bold) and message lines,
/// the background sprite's rectangle and its two buttons.
struct Popup {
    lines: Vec<(Text, f32, f32)>,
    frame: (f32, f32, f32, f32),
    buttons: [(f32, f32); 2],
}
impl Popup {
    fn new(font: &Font, title: &Text, message: &Text, (w, h): (f32, f32)) -> Self {
        let blocks = [
            wrap(font, title, POPUP_CONTENT),
            wrap(font, message, POPUP_CONTENT),
        ];
        let row = 2.0 * POPUP_BUTTON + 6.0;
        let lw = blocks.iter().map(|b| widest(font, b)).fold(row, f32::max);
        let lh = blocks
            .iter()
            .map(|b| 9.0 * b.len() as f32 + 12.0)
            .sum::<f32>()
            + 20.0;
        let (x, y) = (((w - lw) / 2.0).trunc(), ((h - lh) / 2.0).trunc());
        let mut lines = Vec::new();
        let mut top = y;
        for block in blocks {
            let bw = widest(font, &block);
            let mid = x + ((lw - bw) / 2.0).trunc() + (bw / 2.0).trunc();
            let n = block.len() as f32;
            lines.extend(centred_lines(font, block, mid, top));
            top += 9.0 * n + 12.0;
        }
        let bx = x + ((lw - row) / 2.0).trunc();
        Self {
            lines,
            frame: (x - 18.0, y - 18.0, lw + 36.0, lh + 36.0),
            buttons: [(bx, top), (bx + POPUP_BUTTON + 6.0, top)],
        }
    }
    /// The lock's question: WorldOptionsScreen's Apply when the lock changed.
    fn lock_question(font: &Font, difficulty: u8, size: (f32, f32)) -> Self {
        let name = Text::translate(
            &format!(
                "options.difficulty.{}",
                DIFFICULTIES[usize::from(difficulty) & 3]
            ),
            &[],
        );
        Self::new(
            font,
            &Text::translate("difficulty.lock.title", &[]).bold(),
            &Text::translate("difficulty.lock.question", &[name]),
            size,
        )
    }
}

/// RestrictionsScreen for the player of a single-player world, whose chat
/// abilities come from the Chat option alone (Minecraft.computeChatAbilities).
struct Restrictions {
    lines: Vec<(Text, f32, f32)>,
    /// The restriction's action, 200 wide: Chat Settings.
    action: Option<(f32, f32)>,
    /// The ScrollableLayout's top and bottom.
    area: (f32, f32),
}
impl Restrictions {
    fn new(font: &Font, chat_visibility: u8, (w, h): (f32, f32)) -> Self {
        let red = named_color("red").unwrap_or(0xFF5555);
        let green = named_color("green").unwrap_or(0x55FF55);
        let inner = RESTRICTION_WIDTH - 8.0;
        // ChatVisiblity HIDDEN takes chat and commands, SYSTEM the chat.
        let restriction = match chat_visibility {
            2 => Some("chat_restriction.chat_and_commands_disabled_by_options"),
            1 => Some("chat_restriction.chat_disabled_by_options"),
            _ => None,
        };
        let abilities = sunrise_client::chat::ChatAbilities::new(
            sunrise_client::chat::ChatVisibility::from_option(chat_visibility),
        );
        let (chat, commands) = (abilities.send_messages, abilities.send_commands);
        // The body's widgets: text blocks (lines, top) and the action's top.
        let mut blocks = Vec::new();
        let mut action = None;
        let mut y = 0.0;
        if let Some(key) = restriction {
            let lines = wrap(
                font,
                &Text::translate(key, &[]).bold().with_color(red),
                inner,
            );
            let n = lines.len() as f32;
            blocks.push((lines, y));
            y += 9.0 * n + 8.0;
            action = Some(y);
            // The button, then a 10 pixel SpacerElement.
            y += 20.0 + 10.0;
        }
        let permissions = [
            ("send_messages", chat),
            ("send_commands", commands),
            ("receive_system_messages", commands),
            ("receive_player_messages", chat),
        ];
        let lines: Vec<Text> = permissions
            .iter()
            .flat_map(|&(id, allowed)| {
                let state = if allowed { "allowed" } else { "denied" };
                let t =
                    Text::translate(&format!("restrictions_screen.permission.{id}.{state}"), &[])
                        .with_color(if allowed { green } else { red });
                wrap(font, &t, inner)
            })
            .collect();
        let n = lines.len() as f32;
        blocks.push((lines, y));
        y += 9.0 * n + 8.0;
        // HeaderAndFooterLayout: 30 under the header if it fits; the
        // ScrollableLayout keeps 10 pixels each side for its scrollbar.
        let height = y.min(h - HEADER - FOOTER);
        let top = (HEADER + 30.0).min(h - FOOTER - height);
        let x = ((w - (RESTRICTION_WIDTH + 20.0)) / 2.0).trunc() + 10.0;
        let mid = x + RESTRICTION_WIDTH / 2.0;
        Self {
            lines: blocks
                .into_iter()
                .flat_map(|(lines, by)| centred_lines(font, lines, mid, top + by + 4.0))
                .collect(),
            action: action.map(|ay| (x + 25.0, top + ay)),
            area: (top, top + height),
        }
    }
    /// The footer's Done, 200 wide.
    fn done(w: f32, h: f32) -> (f32, f32) {
        (((w - 200.0) / 2.0).trunc(), h - FOOTER + 6.0)
    }
}

/// `WorldOptionsScreen.tryParsePort`'s invalid value: not a number in
/// `PORT_LOWER_BOUND..=PORT_HIGHER_BOUND`. A number another process holds
/// (`HttpUtil.isPortAvailable`) is not checked: the port map records that gap.
fn port_error(value: &str) -> Option<&'static str> {
    if value.is_empty() {
        return None;
    }
    match value.parse::<u32>() {
        Ok(port) if (1024..=65535).contains(&port) => None,
        _ => Some("lanServer.port.invalid"),
    }
}
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn remote_world_options_permission(
        &mut self,
        permission: bool,
    ) {
        if let Some(o) = self.client.world_options.0.as_mut() {
            let lost = o.gamemaster && !permission;
            o.gamemaster = permission;
            if lost && o.remote {
                self.close_world_options();
            }
        }
    }
    pub(in crate::game::client_side) fn remote_world_options_difficulty(
        &mut self,
        difficulty: u8,
        locked: bool,
    ) {
        if let Some(o) = self.client.world_options.0.as_mut() {
            o.server_difficulty(difficulty, locked);
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_open_remote_rules(&mut self, entries: Vec<play::GameRuleEntry>) -> bool {
        self.client.world_options.0 = Some(options(true, false));
        self.client.world_options.0.as_mut().unwrap().view = View::GameRules;
        self.remote_world_options_rules(entries);
        self.client
            .world_options
            .0
            .as_ref()
            .unwrap()
            .rules
            .is_some()
    }

    pub(in crate::game::client_side) fn remote_world_options_rules(
        &mut self,
        entries: Vec<play::GameRuleEntry>,
    ) {
        let Some(o) = self
            .client
            .world_options
            .0
            .as_mut()
            .filter(|o| o.view == View::GameRules)
        else {
            return;
        };
        // The response itself lists the server's available rules; the client
        // may not own this world's level.dat (including on the server thread).
        let features = if entries.iter().any(|e| {
            sunrise_game_data::gamerules::rule_registry()
                .rule(&e.name)
                .is_some_and(|r| r.name == "minecraft:max_minecart_speed")
        }) {
            vec![sunrise_game_data::gamerules::MINECART_IMPROVEMENTS.to_owned()]
        } else {
            Vec::new()
        };
        let registry = self
            .client
            .integrated
            .as_ref()
            .map_or_else(sunrise_game_data::gamerules::rule_registry, |server| {
                server.rule_registry.clone()
            });
        let mut rules = sunrise_game_data::gamerules::GameRules::with_registry(registry, &features);
        for entry in entries {
            rules.set_string(&entry.name, &entry.value);
        }
        o.rules = Some(RulesScreen::new(&rules));
    }
    pub fn open_world_options(&mut self) {
        let hint = sunrise_sim::lan::available_port();
        let mut port = EditBox::new(0.0, 0.0, 150.0, 20.0);
        port.message = super::screen_info::tr("lanServer.port", &[]);
        port.max_length = 5;
        // The world is the server thread's when there is one.
        let state = match &self.client.integrated {
            Some(server) => match server.ask(OwnerRequest::ServerOptions) {
                Some(state) => state,
                None => return,
            },
            None if self.external_server() => ServerOptions {
                default_mode: self.common.mode_id(),
                personal_mode: self.common.mode_id(),
                cheats: self.common.ui.registries.permission >= 2,
                difficulty: self.common.ui.difficulty,
                hardcore: self.common.level.hardcore,
                difficulty_locked: self.common.level.difficulty_locked,
                port: None,
                guest_commands: false,
                force: true,
            },
            None => return,
        };
        if let Some(p) = state.port {
            port.set_value(&p.to_string());
        }
        let mut options = WorldOptions {
            remote: self.external_server(),
            gamemaster: self.common.ui.registries.permission >= 2,
            default_mode: state.default_mode,
            personal_mode: state.personal_mode,
            cheats: state.cheats,
            difficulty: state.difficulty,
            lan: state.port.is_some(),
            port,
            hint,
            guest_commands: state.guest_commands,
            force: state.force,
            hardcore: state.hardcore,
            difficulty_locked: state.difficulty_locked,
            locked: state.difficulty_locked || state.hardcore,
            scroll: 0.0,
            view: View::Options,
            rules: None,
            focused_button: None,
            popup_focus: None,
            initial: Snapshot {
                default_mode: 0,
                personal_mode: 0,
                cheats: false,
                difficulty: 0,
                locked: false,
                lan: false,
                port: 0,
                guest_commands: false,
                force: false,
            },
        };
        options.initial = options.snapshot();
        self.client.world_options.0 = Some(options);
        self.set_screen(Screen::WorldOptions);
    }

    pub fn close_world_options(&mut self) {
        self.client.world_options.0 = None;
        self.set_screen(Screen::Paused);
    }

    /// `WorldOptionsScreen.updatePersonalGameModeButton`: update both the displayed
    /// personal mode and its baseline without discarding unrelated edits.
    pub(in crate::game::client_side) fn update_world_options_personal_mode(&mut self, mode: i32) {
        if self.client.screen == Screen::WorldOptions
            && let Some(o) = self.client.world_options.0.as_mut()
        {
            o.update_personal_mode(mode);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn world_options_personal_state(&self) -> Option<(i32, i32, u8)> {
        self.client
            .world_options
            .0
            .as_ref()
            .map(|o| (o.personal_mode, o.initial.personal_mode, o.difficulty))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_world_options_difficulty_for_test(&mut self, difficulty: u8) {
        if let Some(o) = self.client.world_options.0.as_mut() {
            o.difficulty = difficulty;
        }
    }

    /// Escape: the question and RestrictionsScreen return to the options
    /// (their onClose), which close; the game rules ask first.
    pub(in crate::game::client_side) fn world_options_escape(&mut self) {
        match self.client.world_options.0.as_mut() {
            Some(o) if o.view == View::GameRules => {
                if o.rules.is_none() {
                    // A server may be slow to answer REQUEST_GAMERULE_VALUES.
                    o.view = View::Options;
                } else if let Some(exit) = o.rules.as_mut().and_then(RulesScreen::escape) {
                    self.close_game_rules(exit);
                }
            }
            Some(o) if o.view != View::Options => o.view = View::Options,
            _ => self.close_world_options(),
        }
    }

    /// InWorldGameRulesScreen's exit, back to the options. Done sends the
    /// changed rules (ServerboundSetGameRulePacket), which the server's
    /// handleSetGameRule sets for a gamemaster and tells its operators.
    fn close_game_rules(&mut self, exit: Exit) {
        if let Some(o) = self.client.world_options.0.as_mut() {
            o.view = View::Options;
            o.rules = None;
        }
        let Exit::Apply(changes) = exit else {
            return;
        };
        if self.client.remote.is_some() {
            if self.common.ui.registries.permission >= 2 && !changes.is_empty() {
                self.send(S::SetGameRule(play::SetGameRule {
                    entries: changes
                        .into_iter()
                        .map(|(name, value)| play::GameRuleEntry {
                            name,
                            value: value.to_string(),
                        })
                        .collect(),
                }));
            }
            return;
        }
        if !self.common.allow_commands() || changes.is_empty() {
            return;
        }
        for (name, value) in changes {
            self.common.ui.rules.set(&name, value);
            self.client_system_message(Text::tr("commands.gamerule.set", &[&name, &value]));
        }
        self.common.sync_rules();
    }

    pub(in crate::game::client_side) fn draw_world_options(&mut self, batch: &mut HudBatch) {
        let Some(view) = self.client.world_options.0.as_ref().map(|o| o.view) else {
            self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
            self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
            return;
        };
        match view {
            View::Options => self.draw_options_view(batch, self.client.cursor, true),
            View::LockQuestion => {
                // PopupScreen: the screen behind without the pointer, then the popup.
                self.draw_options_view(batch, (-1.0, -1.0), false);
                self.draw_lock_question(batch);
            }
            View::Restrictions => self.draw_restrictions(batch),
            View::GameRules => {
                if self
                    .client
                    .world_options
                    .0
                    .as_ref()
                    .unwrap()
                    .rules
                    .is_some()
                {
                    self.client
                        .world_options
                        .0
                        .as_ref()
                        .unwrap()
                        .rules
                        .as_ref()
                        .unwrap()
                        .draw_background(batch, &self.client.gui.atlas);
                    self.draw_screen_phase(
                        batch,
                        sunrise_client::screen_draw::RenderPhase::Background,
                    );
                    let tooltip = self
                        .client
                        .world_options
                        .0
                        .as_mut()
                        .unwrap()
                        .rules
                        .as_mut()
                        .unwrap()
                        .draw_contents(batch, &self.client.gui.atlas, self.client.cursor);
                    self.draw_screen_phase(
                        batch,
                        sunrise_client::screen_draw::RenderPhase::Foreground,
                    );
                    if let Some(tooltip) = tooltip {
                        ui::tooltip(batch, &self.client.gui.atlas, &tooltip, self.client.cursor);
                    }
                } else {
                    ui::menu_background(
                        batch,
                        &self.client.gui.atlas,
                        0.0,
                        0.0,
                        batch.width,
                        batch.height,
                        true,
                    );
                    self.draw_screen_phase(
                        batch,
                        sunrise_client::screen_draw::RenderPhase::Background,
                    );
                    self.draw_screen_phase(
                        batch,
                        sunrise_client::screen_draw::RenderPhase::Foreground,
                    );
                }
            }
        }
    }

    fn draw_lock_question(&self, batch: &mut HudBatch) {
        let o = self.client.world_options.0.as_ref().unwrap();
        // PopupScreen over WorldOptionsScreen: its title and message.
        sunrise_client::widgets::begin_screen(|| {
            let name = Text::translate(
                &format!(
                    "options.difficulty.{}",
                    DIFFICULTIES[usize::from(o.difficulty) & 3]
                ),
                &[],
            );
            super::screen_info::popup(
                "world_options/lock",
                &Text::translate("difficulty.lock.title", &[]).plain(),
                &[&Text::translate("difficulty.lock.question", &[name]).plain()],
            )
        });
        let atlas = &self.client.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        // Screen.extractTransparentBackground.
        batch.fill_gradient(
            atlas,
            (0.0, 0.0, w, h),
            rgba(0x10, 0x10, 0x10, 0xC0),
            rgba(0x10, 0x10, 0x10, 0xD0),
        );
        let p = Popup::lock_question(&atlas.font, o.difficulty, (w, h));
        let (fx, fy, fw, fh) = p.frame;
        ui::nine_slice(batch, atlas, "popup/background", fx, fy, fw, fh, u32::MAX);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        for (line, x, y) in &p.lines {
            batch.text_styled(atlas, line, *x, *y, u32::MAX, true);
        }
        for (i, ((bx, by), key)) in p.buttons.into_iter().zip(["gui.yes", "gui.no"]).enumerate() {
            let is_focused = o.popup_focus == Some(i);
            let hover = ui::hovered(self.client.cursor, bx, by, POPUP_BUTTON, 20.0) || is_focused;
            let label = Text::translate(key, &[]);
            ui::button(
                batch,
                atlas,
                bx,
                by,
                POPUP_BUTTON,
                20.0,
                &label,
                true,
                hover,
            );
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
    }

    fn draw_restrictions(&self, batch: &mut HudBatch) {
        sunrise_client::widgets::begin_screen(|| {
            sunrise_client::widgets::ScreenInfo::new(
                "restrictions",
                super::screen_info::tr("restrictions_screen.title", &[]),
            )
        });
        let atlas = &self.client.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        let title = Text::translate("restrictions_screen.title", &[]);
        let tw = atlas.font.width_text(&title);
        batch.text_styled(
            atlas,
            &title,
            ((w - tw) / 2.0).floor(),
            12.0,
            u32::MAX,
            true,
        );
        let r = Restrictions::new(&atlas.font, self.client.settings.chat_visibility, (w, h));
        let first = batch.vertices.len();
        for (line, x, y) in &r.lines {
            batch.text_styled(atlas, line, *x, *y, u32::MAX, true);
        }
        if let Some((ax, ay)) = r.action {
            let is_focused = self
                .client
                .world_options
                .0
                .as_ref()
                .and_then(|o| o.popup_focus)
                == Some(0);
            let hover = ui::hovered(self.client.cursor, ax, ay, 200.0, 20.0) || is_focused;
            let label = Text::translate("chat_restriction.chat_disabled_by_options.action", &[]);
            ui::button(batch, atlas, ax, ay, 200.0, 20.0, &label, true, hover);
        }
        batch.clip_since(first, 0.0, r.area.0, w, r.area.1);
        let (dx, dy) = Restrictions::done(w, h);
        let is_focused = self.client.world_options.0.as_ref().is_some_and(|o| {
            o.popup_focus == Some(1) || (o.popup_focus == Some(0) && r.action.is_none())
        });
        let hover = ui::hovered(self.client.cursor, dx, dy, 200.0, 20.0) || is_focused;
        let label = Text::translate("gui.done", &[]);
        ui::action_button(
            batch,
            atlas,
            sunrise_client::widgets::Action::Done,
            dx,
            dy,
            200.0,
            20.0,
            &label,
            true,
            hover,
        );
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
    }

    fn draw_options_view(&mut self, batch: &mut HudBatch, cursor: (f32, f32), active_screen: bool) {
        sunrise_client::widgets::begin_screen(|| {
            sunrise_client::widgets::ScreenInfo::new(
                "world_options",
                super::screen_info::tr("options.worldOptions.title", &[]),
            )
        });
        let atlas = &self.client.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        // extractMenuBackground: the in-world background, the list
        // background under the content, then the separators over it.
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        ui::list_background(batch, atlas, HEADER, h - FOOTER, true);
        ui::separators(batch, atlas, HEADER, h - FOOTER, true);
        if active_screen {
            self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        }
        let o = self.client.world_options.0.as_mut().unwrap();
        o.scroll = o.scroll.clamp(0.0, o.scroll_limit(h));
        let title = Text::translate("options.worldOptions.title", &[]);
        let tw = atlas.font.width_text(&title);
        batch.text_styled(
            atlas,
            &title,
            ((w - tw) / 2.0).floor(),
            12.0,
            u32::MAX,
            true,
        );
        let top = o.top();
        let first = batch.vertices.len();
        let x = WorldOptions::left(w);
        for (key, y) in [
            ("options.worldOptions.general.title", GENERAL_TOP),
            ("options.worldOptions.multiplayer.title", MULTIPLAYER_TOP),
        ] {
            if o.remote && key == "options.worldOptions.multiplayer.title" {
                continue;
            }
            // A FocusableTextWidget padded by 4, centred over both columns.
            let t = Text::translate(key, &[]).underlined().bold();
            let tw = atlas.font.width_text(&t);
            let tx = x + ((308.0 - (tw + 8.0)) / 2.0).trunc() + 4.0;
            batch.text_styled(atlas, &t, tx, top + y + 4.0, u32::MAX, true);
        }
        let inside = cursor.1 >= HEADER && cursor.1 < h - FOOTER;
        let tr = &super::screen_info::tr;
        // The categories' FocusableTextWidgets, each before its grid.
        let header = |key: &str| {
            let t = Text::translate(key, &[]).underlined().bold();
            let tw = atlas.font.width_text(&t);
            let y = if key == "options.worldOptions.general.title" {
                GENERAL_TOP
            } else {
                MULTIPLAYER_TOP
            };
            let tx = x + ((308.0 - (tw + 8.0)) / 2.0).trunc();
            let rect = [tx, top + y, tw + 8.0, 17.0];
            widgets::narratable(|| Narratable::text(t.plain(), rect));
        };
        header("options.worldOptions.general.title");
        let mut port_at = None;
        for (b, bx, by, bw) in o.visible_buttons(w) {
            let active = o.active(b);
            let is_focused = o.focused_button == Some(b);
            let hover = active && ((inside && ui::hovered(cursor, bx, by, bw, 20.0)) || is_focused);
            if b == Button::Lan {
                header("options.worldOptions.multiplayer.title");
            }
            if b == Button::Lock {
                widgets::narratable(|| o.narration(b, [bx, by, 20.0, 20.0], tr).hovered(hover));
                // An icon, but a button: the D-pad and Tab stop on it as on the others.
                if active {
                    widgets::target([bx, by, 20.0, 20.0]);
                }
                let lock = if o.locked { "locked" } else { "unlocked" };
                let state = match (active, hover) {
                    (false, _) => "_disabled",
                    (true, true) => "_highlighted",
                    (true, false) => "",
                };
                let sprite = format!("widget/{lock}_button{state}");
                batch.sprite_scaled(atlas, &sprite, bx, by, 20.0, 20.0, u32::MAX);
                continue;
            }
            ui::button_as(
                batch,
                atlas,
                [bx, by, bw, 20.0],
                &o.label(b),
                active,
                hover,
                None,
                |rect| o.narration(b, rect, tr),
            );
            if b == Button::Lan {
                // The port box is added after it.
                port_at = Some(widgets::narratables_mark());
            }
        }
        // The port box beside the LAN button: editable only with LAN on,
        // its hint the port to use (setPortError colours the text).
        if !o.remote {
            o.port.x = x + 158.0;
            o.port.y = top + PORT_TOP;
            o.update_port_controls();
            o.port.text_color = if !o.lan {
                rgba(0x70, 0x70, 0x70, 255)
            } else if o.parsed_port().is_some() {
                rgba(0xE0, 0xE0, 0xE0, 255)
            } else {
                rgba(0xDF, 0x50, 0x50, 255)
            };
            let mark = widgets::narratables_mark();
            o.port.draw(batch, atlas, None);
            // updatePortControlsState: the box is active with LAN only.
            let lan = o.lan;
            widgets::last_narratable(|n| {
                if let sunrise_client::widgets::What::EditBox { active, .. } = &mut n.what {
                    *active = lan;
                }
            });
            if let Some(at) = port_at {
                widgets::move_narratables(mark, at);
            }
            if o.port.value.is_empty() && !o.port.focused {
                let hint = if o.lan {
                    Text::literal(o.hint.to_string())
                } else {
                    Text::translate("lanServer.port", &[])
                };
                // EditBox.DEFAULT_HINT_STYLE: dark gray.
                batch.text_styled(
                    atlas,
                    &hint,
                    o.port.text_x(),
                    o.port.text_y(),
                    rgba(0x55, 0x55, 0x55, 255),
                    true,
                );
            }
        }
        batch.clip_since(first, 0.0, HEADER, w, h - FOOTER);
        ui::scrollbar(
            batch,
            atlas,
            (w / 2.0).floor() + 166.0,
            HEADER,
            h - FOOTER,
            o.content_height(),
            o.scroll.trunc(),
        );
        for (b, bx, by, bw) in WorldOptions::footer(w, h) {
            let active = o.active(b);
            let is_focused = o.focused_button == Some(b);
            let hover = active && (ui::hovered(cursor, bx, by, bw, 20.0) || is_focused);
            let action = match b {
                Button::Apply => sunrise_client::widgets::Action::Apply,
                _ => sunrise_client::widgets::Action::Cancel,
            };
            ui::action_button(
                batch,
                atlas,
                action,
                bx,
                by,
                bw,
                20.0,
                &o.label(b),
                active,
                hover,
            );
        }
        let tooltip = WorldOptions::port_tooltip(&o.port, cursor, o.remote);
        if active_screen {
            self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        }
        // The widgets are drawn; the port box's tooltip goes over them.
        if let Some(tip) = tooltip {
            // Tooltip.splitTooltip: font.split(message, 170).
            ui::tooltip(batch, atlas, &atlas.font.split(&tip, 170.0), cursor);
        }
    }

    /// The mouse wheel over the screen (ScrollableLayout, 10 pixels a step).
    pub(in crate::game::client_side) fn world_options_scroll(&mut self, delta: f32) -> bool {
        let (w, h) = self.gui_size();
        let stick_scroll = self.pad_stick_scroll();
        match self.client.world_options.0.as_mut() {
            Some(o) if o.view == View::Options => {
                let hit_btn = o.hit(self.client.cursor, (w, h));
                if let Some(btn) = hit_btn.filter(|_| !stick_scroll) {
                    let step = if delta > 0.0 { -1 } else { 1 };
                    match btn {
                        Button::DefaultMode => {
                            o.default_mode = (o.default_mode + step).rem_euclid(4);
                            self.play_audio_button_click();
                            return true;
                        }
                        Button::PersonalMode => {
                            o.personal_mode = (o.personal_mode + step).rem_euclid(4);
                            self.play_audio_button_click();
                            return true;
                        }
                        Button::Difficulty => {
                            o.difficulty = ((o.difficulty as i32 + step).rem_euclid(4)) as u8;
                            self.play_audio_button_click();
                            return true;
                        }
                        Button::Cheats => {
                            o.cheats = !o.cheats;
                            if !o.cheats {
                                o.default_mode = o.initial.default_mode;
                                o.personal_mode = o.default_mode;
                            }
                            self.play_audio_button_click();
                            return true;
                        }
                        Button::Lan => {
                            o.lan = !o.lan;
                            o.port.focused = false;
                            self.play_audio_button_click();
                            return true;
                        }
                        Button::CommandAccess => {
                            o.guest_commands = !o.guest_commands;
                            self.play_audio_button_click();
                            return true;
                        }
                        Button::Force => {
                            o.force = !o.force;
                            self.play_audio_button_click();
                            return true;
                        }
                        _ => {}
                    }
                }
                if stick_scroll || (HEADER..h - FOOTER).contains(&self.client.cursor.1) {
                    o.scroll = (o.scroll - delta * 10.0).clamp(0.0, o.scroll_limit(h));
                    return true;
                }
            }
            Some(o) => {
                return o
                    .rules
                    .as_mut()
                    .is_some_and(|rules| rules.wheel_at(delta, self.client.cursor.1, h));
            }
            None => {}
        }
        false
    }

    /// The pointer moved while a list's scrollbar is held: the game rules'
    /// list follows it (AbstractSelectionList.mouseDragged).
    pub(in crate::game::client_side) fn world_options_drag(&mut self) -> bool {
        let h = self.gui_size().1;
        if let Some(o) = self.client.world_options.0.as_mut()
            && o.view == View::GameRules
            && let Some(rules) = o.rules.as_mut()
        {
            rules.drag(self.client.cursor.1, h);
            return true;
        }
        false
    }

    /// The button came up: a held scrollbar lets go (onRelease).
    pub(in crate::game::client_side) fn world_options_release(&mut self) -> bool {
        if let Some(o) = self.client.world_options.0.as_mut()
            && let Some(rules) = o.rules.as_mut()
        {
            return rules.release();
        }
        false
    }

    pub(in crate::game::client_side) fn world_options_click(&mut self) -> bool {
        let Some(mut o) = self.client.world_options.0.take() else {
            return false;
        };
        let (w, h) = self.gui_size();
        match o.view {
            View::Options => {}
            View::GameRules => {
                let atlas = &self.client.gui.atlas;
                let (clicked, exit, handled) =
                    o.rules.as_mut().map_or((false, None, false), |rules| {
                        let clicked = rules.clicks_button(atlas, self.client.cursor, (w, h));
                        let (exit, handled) = rules.click(atlas, self.client.cursor, (w, h));
                        (clicked, exit, handled)
                    });
                self.client.world_options.0 = Some(o);
                if clicked {
                    self.play_audio_button_click();
                }
                if let Some(exit) = exit {
                    self.close_game_rules(exit);
                }
                return handled;
            }
            View::LockQuestion => {
                let p = Popup::lock_question(&self.client.gui.atlas.font, o.difficulty, (w, h));
                let hit = p.buttons.iter().position(|&(bx, by)| {
                    ui::hovered(self.client.cursor, bx, by, POPUP_BUTTON, 20.0)
                });
                if hit.is_some() {
                    self.play_audio_button_click();
                }
                match hit {
                    // Yes: apply, then back to the pause menu; No: back to the options.
                    Some(0) => {
                        self.apply_and_close_world_options(o);
                        return true;
                    }
                    Some(_) => o.view = View::Options,
                    None => {}
                }
                self.client.world_options.0 = Some(o);
                return hit.is_some();
            }
            View::Restrictions => {
                let r = Restrictions::new(
                    &self.client.gui.atlas.font,
                    self.client.settings.chat_visibility,
                    (w, h),
                );
                let in_body = self.client.cursor.1 >= r.area.0 && self.client.cursor.1 < r.area.1;
                let action = r.action.is_some_and(|(ax, ay)| {
                    in_body && ui::hovered(self.client.cursor, ax, ay, 200.0, 20.0)
                });
                let (dx, dy) = Restrictions::done(w, h);
                let done = ui::hovered(self.client.cursor, dx, dy, 200.0, 20.0);
                if action || done {
                    self.play_audio_button_click();
                    o.view = View::Options;
                }
                self.client.world_options.0 = Some(o);
                if action {
                    // ChatRestriction.Action.GO_TO_CHAT_SETTINGS: Chat Settings,
                    // which returns to the screen behind RestrictionsScreen.
                    self.open_options_at(false, sunrise_client::options_ui::Page::Chat);
                    self.client.options_to_world_options = true;
                }
                return action || done;
            }
        }
        let inside = self.client.cursor.1 >= HEADER && self.client.cursor.1 < h - FOOTER;
        o.port.focused = !o.remote
            && inside
            && ui::hovered(
                self.client.cursor,
                WorldOptions::left(w) + 158.0,
                o.top() + PORT_TOP,
                150.0,
                20.0,
            );
        let hit = o.hit(self.client.cursor, (w, h));
        if let Some(b) = hit {
            o.focused_button = Some(b);
            self.play_audio_button_click();
        }
        match hit {
            Some(Button::DefaultMode) => o.default_mode = (o.default_mode + 1) % 4,
            Some(Button::PersonalMode) => o.personal_mode = (o.personal_mode + 1) % 4,
            Some(Button::Cheats) => {
                o.cheats = !o.cheats;
                // updatePermissionDependentButtons: without commands both
                // game modes return to the world's default.
                if !o.cheats {
                    o.default_mode = o.initial.default_mode;
                    o.personal_mode = o.default_mode;
                }
            }
            Some(Button::Difficulty) => o.difficulty = (o.difficulty + 1) % 4,
            Some(Button::Lock) => o.locked = !o.locked,
            Some(Button::Restrictions) => o.view = View::Restrictions,
            Some(Button::GameRules) => {
                if self.client.remote.is_some() {
                    o.rules = None;
                    self.send(S::ClientCommand(play::ClientCommand { action: 2 }));
                } else {
                    o.rules = Some(RulesScreen::new(&self.common.ui.rules));
                }
                o.view = View::GameRules;
            }
            Some(Button::Lan) => {
                o.lan = !o.lan;
                o.port.focused = false;
            }
            Some(Button::CommandAccess) => o.guest_commands = !o.guest_commands,
            Some(Button::Force) => o.force = !o.force,
            Some(Button::Cancel) => {
                self.client.world_options.0 = Some(o);
                self.close_world_options();
                return true;
            }
            // A new lock asks first.
            Some(Button::Apply) if o.locked != o.initial.locked => o.view = View::LockQuestion,
            Some(Button::Apply) => {
                self.apply_and_close_world_options(o);
                return true;
            }
            None => {}
        }
        let handled = hit.is_some() || o.port.focused;
        self.client.world_options.0 = Some(o);
        handled
    }

    fn apply_and_close_world_options(&mut self, o: WorldOptions) {
        let wanted = o.snapshot();
        let before = o.initial;
        self.client.world_options.0 = Some(o);
        self.apply_world_options_client(&before, &wanted);
        self.close_world_options();
    }

    /// `WorldOptionsScreen.sendPublishMessage`: a client system message the
    /// narrator says.
    fn send_publish_message(&mut self, message: Text) {
        self.say_system_queued(&message);
        self.client_system_message(message);
    }

    pub(in crate::game::client_side) fn world_options_key(
        &mut self,
        key: KeyCode,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
    ) -> bool {
        let ctrl = self.has_control_down_with_quirk();
        let shift = self.has_shift_down();
        let (w, h) = self.gui_size();
        let chat_vis = self.client.settings.chat_visibility;
        let Some(o) = self.client.world_options.0.as_mut() else {
            return false;
        };
        if let Some(rules) = o.rules.as_mut().filter(|r| r.typing()) {
            let mut handled = true;
            rules.edit(|b| match shortcut {
                Some(shortcut) => {
                    b.shortcut(shortcut, &mut self.client.clipboard.0);
                }
                None => handled = b.key(key, ctrl, false),
            });
            return handled;
        }
        if o.view == View::LockQuestion {
            match key {
                winit::keyboard::KeyCode::Tab => {
                    let reverse = shift;
                    o.popup_focus = match o.popup_focus {
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
                }
                winit::keyboard::KeyCode::ArrowLeft | winit::keyboard::KeyCode::ArrowUp => {
                    o.popup_focus = Some(0)
                }
                winit::keyboard::KeyCode::ArrowRight | winit::keyboard::KeyCode::ArrowDown => {
                    o.popup_focus = Some(1)
                }
                winit::keyboard::KeyCode::Enter
                | winit::keyboard::KeyCode::NumpadEnter
                | winit::keyboard::KeyCode::Space => {
                    let yes = o.popup_focus == Some(0);
                    self.play_audio_button_click();
                    let mut o = self.client.world_options.0.take().unwrap();
                    if yes {
                        self.apply_and_close_world_options(o);
                        return true;
                    } else {
                        o.view = View::Options;
                        self.client.world_options.0 = Some(o);
                    }
                    return true;
                }
                _ => {}
            }
            return false;
        }
        if o.view == View::Restrictions {
            let r = Restrictions::new(&self.client.gui.atlas.font, chat_vis, (w, h));
            match key {
                winit::keyboard::KeyCode::Tab => {
                    let reverse = shift;
                    if r.action.is_some() {
                        o.popup_focus = match o.popup_focus {
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
                    } else {
                        o.popup_focus = Some(1);
                    }
                }
                winit::keyboard::KeyCode::ArrowUp => {
                    if r.action.is_some() {
                        o.popup_focus = Some(0);
                    } else {
                        o.popup_focus = Some(1);
                    }
                }
                winit::keyboard::KeyCode::ArrowDown => o.popup_focus = Some(1),
                winit::keyboard::KeyCode::Enter
                | winit::keyboard::KeyCode::NumpadEnter
                | winit::keyboard::KeyCode::Space => {
                    let action = r.action.is_some() && o.popup_focus == Some(0);
                    o.view = View::Options;
                    self.play_audio_button_click();
                    if action {
                        self.open_options_at(false, sunrise_client::options_ui::Page::Chat);
                        self.client.options_to_world_options = true;
                    }
                    return true;
                }
                _ => {}
            }
            return false;
        }
        if o.view != View::Options {
            return false;
        }
        if o.port.focused {
            if key == winit::keyboard::KeyCode::Tab {
                o.port.focused = false;
                if shift {
                    o.focused_button = Some(Button::Lan);
                } else {
                    o.focused_button = Some(Button::CommandAccess);
                }
                return false;
            }
            if let Some(shortcut) = shortcut {
                o.port.shortcut(shortcut, &mut self.client.clipboard.0);
                return true;
            }
            return o.port.key(key, ctrl, false);
        }
        let visible = o.visible_buttons(w);
        let footer = WorldOptions::footer(w, h);
        let active_buttons: Vec<Button> = visible
            .iter()
            .filter(|b| o.active(b.0))
            .map(|b| b.0)
            .chain(footer.iter().filter(|b| o.active(b.0)).map(|b| b.0))
            .collect();
        if active_buttons.is_empty() {
            return false;
        }
        match key {
            winit::keyboard::KeyCode::Tab => {
                let reverse = shift;
                let next = match o.focused_button {
                    Some(cur) => {
                        if !o.remote {
                            if !reverse && cur == Button::Lan {
                                o.port.focused = true;
                                o.focused_button = None;
                                return false;
                            }
                            if reverse && cur == Button::CommandAccess {
                                o.port.focused = true;
                                o.focused_button = None;
                                return false;
                            }
                        }
                        if let Some(pos) = active_buttons.iter().position(|&b| b == cur) {
                            if reverse {
                                if pos == 0 {
                                    active_buttons.last().copied()
                                } else {
                                    Some(active_buttons[pos - 1])
                                }
                            } else if pos + 1 >= active_buttons.len() {
                                active_buttons.first().copied()
                            } else {
                                Some(active_buttons[pos + 1])
                            }
                        } else if reverse {
                            active_buttons.last().copied()
                        } else {
                            active_buttons.first().copied()
                        }
                    }
                    None => {
                        if reverse {
                            active_buttons.last().copied()
                        } else {
                            active_buttons.first().copied()
                        }
                    }
                };
                o.focused_button = next;
            }
            winit::keyboard::KeyCode::ArrowUp
            | winit::keyboard::KeyCode::ArrowDown
            | winit::keyboard::KeyCode::ArrowLeft
            | winit::keyboard::KeyCode::ArrowRight => {
                let rects: Vec<(Button, f32, f32, f32)> = visible
                    .into_iter()
                    .filter(|b| o.active(b.0))
                    .chain(footer.into_iter().filter(|b| o.active(b.0)))
                    .collect();
                let cur_b = o.focused_button.unwrap_or(active_buttons[0]);
                if let Some(cur) = rects.iter().find(|r| r.0 == cur_b) {
                    let (cx, cy) = (cur.1 + cur.3 / 2.0, cur.2 + 10.0);
                    let mut best: Option<(Button, f32)> = None;
                    for r in &rects {
                        if r.0 == cur_b {
                            continue;
                        }
                        let (rx, ry) = (r.1 + r.3 / 2.0, r.2 + 10.0);
                        let (dx, dy) = (rx - cx, ry - cy);
                        let valid = match key {
                            winit::keyboard::KeyCode::ArrowUp => dy < -4.0,
                            winit::keyboard::KeyCode::ArrowDown => dy > 4.0,
                            winit::keyboard::KeyCode::ArrowLeft => dx < -4.0,
                            winit::keyboard::KeyCode::ArrowRight => dx > 4.0,
                            _ => false,
                        };
                        if !valid {
                            continue;
                        }
                        let dist = match key {
                            winit::keyboard::KeyCode::ArrowUp
                            | winit::keyboard::KeyCode::ArrowDown => dy.abs() + dx.abs() * 2.0,
                            winit::keyboard::KeyCode::ArrowLeft
                            | winit::keyboard::KeyCode::ArrowRight => dx.abs() + dy.abs() * 2.0,
                            _ => f32::MAX,
                        };
                        if best.as_ref().is_none_or(|(_, d)| dist < *d) {
                            best = Some((r.0, dist));
                        }
                    }
                    if let Some((next_b, _)) = best {
                        o.focused_button = Some(next_b);
                    }
                } else {
                    o.focused_button = Some(active_buttons[0]);
                }
            }
            winit::keyboard::KeyCode::Enter
            | winit::keyboard::KeyCode::NumpadEnter
            | winit::keyboard::KeyCode::Space => {
                if let Some(btn) = o.focused_button {
                    self.play_audio_button_click();
                    let mut o = self.client.world_options.0.take().unwrap();
                    match btn {
                        Button::DefaultMode => o.default_mode = (o.default_mode + 1) % 4,
                        Button::PersonalMode => o.personal_mode = (o.personal_mode + 1) % 4,
                        Button::Cheats => {
                            o.cheats = !o.cheats;
                            if !o.cheats {
                                o.default_mode = o.initial.default_mode;
                                o.personal_mode = o.default_mode;
                            }
                        }
                        Button::Difficulty => o.difficulty = (o.difficulty + 1) % 4,
                        Button::Lock => o.locked = !o.locked,
                        Button::Restrictions => o.view = View::Restrictions,
                        Button::GameRules => {
                            if self.client.remote.is_some() {
                                o.rules = None;
                                self.send(S::ClientCommand(play::ClientCommand { action: 2 }));
                            } else {
                                o.rules = Some(RulesScreen::new(&self.common.ui.rules));
                            }
                            o.view = View::GameRules;
                        }
                        Button::Lan => {
                            o.lan = !o.lan;
                            o.port.focused = false;
                        }
                        Button::CommandAccess => o.guest_commands = !o.guest_commands,
                        Button::Force => o.force = !o.force,
                        Button::Cancel => {
                            self.client.world_options.0 = Some(o);
                            self.close_world_options();
                            return true;
                        }
                        Button::Apply if o.locked != o.initial.locked => {
                            o.view = View::LockQuestion
                        }
                        Button::Apply => {
                            self.apply_and_close_world_options(o);
                            return true;
                        }
                    }
                    self.client.world_options.0 = Some(o);
                    return true;
                }
            }
            _ => {}
        }
        false
    }
    /// Whether a World Options text box has focus (the port or a game rule box).
    pub(in crate::game::client_side) fn world_options_typing(&self) -> bool {
        self.client.world_options.0.as_ref().is_some_and(|o| {
            o.rules.as_ref().is_some_and(|r| r.typing())
                || (o.port.focused && o.view == View::Options)
        })
    }
    /// Digits typed into the port field (the keyboard, or a controller's
    /// on-screen keyboard), and its backspace.
    pub(in crate::game::client_side) fn world_options_text(&mut self, text: Option<&str>) {
        if let Some(rules) = self
            .client
            .world_options
            .0
            .as_mut()
            .and_then(|o| o.rules.as_mut())
            .filter(|r| r.typing())
        {
            match text {
                Some(text) => {
                    let typed: String = text.chars().filter(|c| !c.is_control()).collect();
                    if !typed.is_empty() {
                        rules.edit(|b| b.insert_text(&typed));
                    }
                }
                None => rules.edit(|b| {
                    b.key(winit::keyboard::KeyCode::Backspace, false, false);
                }),
            }
            return;
        }
        let Some(o) = self
            .client
            .world_options
            .0
            .as_mut()
            .filter(|o| o.port.focused)
        else {
            return;
        };
        match text {
            Some(text) => {
                let digits: String = text.chars().filter(char::is_ascii_digit).collect();
                if !digits.is_empty() {
                    o.port.insert_text(&digits);
                }
            }
            None => {
                o.port
                    .key(winit::keyboard::KeyCode::Backspace, false, false);
            }
        }
    }

    pub(in crate::game::client_side) fn apply_world_options_client(
        &mut self,
        before: &Snapshot,
        wanted: &Snapshot,
    ) {
        if self.external_server() {
            if self.common.ui.registries.permission >= 2 {
                if wanted.difficulty != before.difficulty {
                    self.send(S::ChangeDifficulty(play::ChangeDifficultyRequest {
                        difficulty: i32::from(wanted.difficulty),
                    }));
                }
                if wanted.locked && !before.locked {
                    self.send(S::LockDifficulty(play::LockDifficulty { locked: true }));
                }
            }
            return;
        }
        if let Some(server) = &self.client.integrated {
            // The server applies them; the window tells what came of
            // Open to LAN (ShareToLanScreen's messages).
            let (b, w) = (*before, *wanted);
            if let Some((closed, published_port)) =
                server.ask(|done| OwnerRequest::ApplyWorldOptions {
                    before: b,
                    wanted: w,
                    done,
                })
            {
                if closed {
                    self.friends_lan_closed();
                }
                if w.lan != b.lan || (w.lan && w.port != b.port) {
                    let message = match (w.lan, published_port) {
                        (true, Some(port)) => Text::translate(
                            "menu.multiplayerOptions.publish.started.lan",
                            &[Text::literal(port.to_string())],
                        ),
                        (true, None) => Text::translate("commands.publish.failed", &[]),
                        (false, _) => {
                            Text::translate("menu.multiplayerOptions.publish.stopped", &[])
                        }
                    };
                    self.send_publish_message(message);
                }
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
fn options(cheats: bool, hardcore: bool) -> WorldOptions {
    let mut o = WorldOptions {
        remote: false,
        gamemaster: cheats,
        default_mode: 1,
        personal_mode: 1,
        cheats,
        difficulty: 2,
        lan: false,
        port: EditBox::new(0.0, 0.0, 150.0, 20.0),
        hint: 25565,
        guest_commands: false,
        force: true,
        hardcore,
        difficulty_locked: false,
        locked: hardcore,
        scroll: 0.0,
        view: View::Options,
        rules: None,
        focused_button: None,
        popup_focus: None,
        initial: Snapshot {
            default_mode: 0,
            personal_mode: 0,
            cheats: false,
            difficulty: 0,
            locked: false,
            lan: false,
            port: 0,
            guest_commands: false,
            force: false,
        },
    };
    o.initial = o.snapshot();
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(key: &str, args: &[&str]) -> String {
        if args.is_empty() {
            key.to_owned()
        } else {
            format!("{key}({})", args.join(", "))
        }
    }

    #[test]
    fn the_switches_narrate_as_cycle_buttons_and_the_lock_as_a_lock() {
        use sunrise_client::widgets::What;
        let o = options(true, false);
        let what = |b| o.narration(b, [0.0; 4], &keys).what;
        let generic = |name: &str, value: &str| format!("options.generic_value({name}, {value})");
        assert_eq!(
            what(Button::Difficulty),
            What::Cycle {
                message: generic("options.difficulty", "options.difficulty.normal"),
                next: generic("options.difficulty", "options.difficulty.hard"),
                active: true,
            }
        );
        assert_eq!(
            what(Button::DefaultMode),
            What::Cycle {
                message: generic(
                    "options.worldOptions.game_mode",
                    "selectWorld.gameMode.creative"
                ),
                next: generic(
                    "options.worldOptions.game_mode",
                    "selectWorld.gameMode.adventure"
                ),
                active: true,
            }
        );
        assert_eq!(
            what(Button::Lock),
            What::Named {
                title: "gui.narrate.button(narrator.button.difficulty_lock). narrator.button.difficulty_lock.unlocked"
                    .into(),
                active: true,
            }
        );
        let mut locked = options(true, true);
        locked.locked = true;
        assert_eq!(
            locked.narration(Button::Lock, [0.0; 4], &keys).what,
            What::Named {
                title: "gui.narrate.button(narrator.button.difficulty_lock). narrator.button.difficulty_lock.locked"
                    .into(),
                active: false,
            }
        );
        assert!(matches!(what(Button::Apply), What::Button { .. }));
    }

    #[test]
    fn layout_follows_the_header_footer_and_grids() {
        let at = |b: Button| {
            let o = options(true, false);
            WorldOptions::buttons(427.0, o.top())
                .into_iter()
                .chain(WorldOptions::footer(427.0, 240.0))
                .find(|e| e.0 == b)
                .map(|(_, x, y, w)| (x, y, w))
                .unwrap()
        };
        // Header 33, content padding 8, heading 17, rows 4 apart.
        assert_eq!(at(Button::DefaultMode), (59.0, 62.0, 308.0));
        assert_eq!(at(Button::Cheats), (59.0, 110.0, 150.0));
        assert_eq!(at(Button::Difficulty), (217.0, 110.0, 130.0));
        assert_eq!(at(Button::Lock), (347.0, 110.0, 20.0));
        assert_eq!(at(Button::Restrictions), (217.0, 134.0, 150.0));
        // The Multiplayer grid 24 below: padding, spacing, padding.
        assert_eq!(at(Button::Lan), (59.0, 199.0, 150.0));
        assert_eq!(at(Button::Force), (217.0, 223.0, 150.0));
        assert_eq!(at(Button::Apply), (59.0, 213.0, 150.0));
        // 218 pixels of content over 174 between the separators.
        assert_eq!(options(true, false).scroll_limit(240.0), 44.0);
        assert_eq!(options(true, false).scroll_limit(400.0), 0.0);
    }

    #[test]
    fn buttons_follow_commands_hardcore_and_lan() {
        let o = options(true, false);
        assert!(o.active(Button::DefaultMode) && o.active(Button::Difficulty));
        assert!(!o.active(Button::CommandAccess) && !o.active(Button::Force));
        assert!(!o.active(Button::Apply));
        assert!(o.active(Button::GameRules));
        let mut o = options(false, false);
        assert!(!o.active(Button::DefaultMode) && !o.active(Button::PersonalMode));
        // Game Rules follows the commands the player has, not the switch.
        assert!(!o.active(Button::GameRules));
        o.cheats = true;
        assert!(!o.active(Button::GameRules));
        let o = options(true, true);
        assert!(!o.active(Button::GameRules));
        let o = options(false, true);
        assert!(!o.active(Button::Cheats) && !o.active(Button::Difficulty));
        assert!(!o.active(Button::Force));
        // Scrolled widgets only take clicks between the separators.
        let mut o = options(true, false);
        o.scroll = 44.0;
        assert_eq!(o.hit((100.0, 30.0), (427.0, 240.0)), None);
        assert_eq!(o.hit((100.0, 70.0), (427.0, 240.0)), Some(Button::Cheats));
    }

    #[test]
    fn the_lock_follows_the_level_and_counts_as_a_change() {
        let mut o = options(true, false);
        assert!(o.active(Button::Lock) && o.active(Button::Restrictions));
        assert!(!o.locked && !o.changed());
        // The wanted lock alone enables Apply (hasSettingsChanges).
        o.locked = true;
        assert!(o.changed() && o.active(Button::Difficulty));
        // A locked level: neither the difficulty nor the lock.
        let mut o = options(true, false);
        o.difficulty_locked = true;
        assert!(!o.active(Button::Lock) && !o.active(Button::Difficulty));
        // Hardcore shows the lock closed, and inactive.
        let o = options(true, true);
        assert!(o.locked && !o.active(Button::Lock));
    }

    #[test]
    fn remote_options_follow_vanilla_guest_layout_and_permissions() {
        let mut o = options(false, false);
        o.remote = true;
        // Vanilla omits host-only rows instead of disabling the whole screen.
        let rows = o.visible_buttons(427.0);
        assert_eq!(
            rows.iter().map(|b| b.0).collect::<Vec<_>>(),
            vec![
                Button::Difficulty,
                Button::Lock,
                Button::GameRules,
                Button::Restrictions,
            ]
        );
        assert_eq!(rows[0], (Button::Difficulty, 59.0, 62.0, 130.0));
        assert_eq!(rows[2], (Button::GameRules, 217.0, 62.0, 150.0));
        assert_eq!(rows[3], (Button::Restrictions, 59.0, 86.0, 150.0));
        assert_eq!(o.scroll_limit(240.0), 0.0);
        // A non-operator cannot activate the difficulty or rule controls.
        assert_eq!(o.hit((100.0, 70.0), (427.0, 240.0)), None);
        assert_eq!(o.hit((240.0, 70.0), (427.0, 240.0)), None);
        assert_eq!(
            o.hit((100.0, 90.0), (427.0, 240.0)),
            Some(Button::Restrictions)
        );
        o.gamemaster = true;
        assert_eq!(
            o.hit((100.0, 70.0), (427.0, 240.0)),
            Some(Button::Difficulty)
        );
        assert_eq!(
            o.hit((240.0, 70.0), (427.0, 240.0)),
            Some(Button::GameRules)
        );
        o.difficulty_locked = true;
        assert_eq!(o.hit((100.0, 70.0), (427.0, 240.0)), None);
        // Remote hardcore does not hide an operator's Game Rules screen.
        o.hardcore = true;
        assert!(o.active(Button::GameRules));
        assert!(!o.active(Button::Lock));
        for b in [
            Button::DefaultMode,
            Button::PersonalMode,
            Button::Cheats,
            Button::Lan,
            Button::CommandAccess,
            Button::Force,
        ] {
            assert!(!o.active(b));
        }
    }

    #[test]
    fn a_delayed_server_reply_does_not_mark_reopened_options_as_edited() {
        let mut o = options(false, false);
        o.remote = true;
        o.gamemaster = true;
        // Reopened while the previous Apply's reply was still in flight.
        o.server_difficulty(3, false);
        assert_eq!(o.difficulty, 3);
        assert!(!o.active(Button::Apply));
        o.difficulty = 0;
        assert!(o.active(Button::Apply));
        o.server_difficulty(3, true);
        assert!(!o.active(Button::Apply));
        assert!(o.locked);
        assert!(!o.active(Button::Difficulty));
    }

    /// `portEdit.setTooltip(PORT_INFO_TEXT)`: hovering the port box shows its
    /// label; `setPortError` turns that into the reason a number is rejected.
    /// A remote world (`o.remote`) has no port box to hover.
    #[test]
    fn the_port_box_explains_itself_on_hover() {
        let mut port = EditBox::new(158.0, 100.0, 40.0, 20.0);
        let tip = |port: &EditBox| {
            // Observe the component key, independent of the loaded UI language.
            WorldOptions::port_tooltip(port, (159.0, 101.0), false)
                .and_then(|t| t.translation_key().map(str::to_owned))
        };
        assert_eq!(tip(&port).as_deref(), Some("lanServer.port"));
        // Empty takes the suggestion, a number in range is fine.
        assert_eq!(tip(&port).as_deref(), Some("lanServer.port"));
        for bad in ["0", "80", "70000", "abc", "-1"] {
            port.set_value(bad);
            assert_eq!(
                tip(&port).as_deref(),
                Some("lanServer.port.invalid"),
                "{bad}"
            );
        }
        for good in ["1024", "25565", "65535"] {
            port.set_value(good);
            assert_eq!(tip(&port).as_deref(), Some("lanServer.port"), "{good}");
        }
        assert!(WorldOptions::port_tooltip(&port, (157.0, 101.0), false).is_none());
        assert!(WorldOptions::port_tooltip(&port, (159.0, 121.0), false).is_none());
        assert!(WorldOptions::port_tooltip(&port, (159.0, 101.0), true).is_none());
    }

    /// `WorldOptionsScreen.updatePortControlsState`: with LAN off the port box
    /// is read-only but keeps the focus, so its value can be selected and
    /// copied; the caret moves, Backspace is consumed without a change, and
    /// with LAN on it edits again.
    #[test]
    fn the_port_box_is_read_only_but_focusable_with_lan_off() {
        use winit::keyboard::KeyCode;
        let mut o = options(false, false);
        assert!(!o.lan);
        o.port.focused = true;
        o.update_port_controls();
        assert!(!o.port.editable);
        assert!(o.port.focused);
        o.port.set_value("25565");
        assert!(o.port.key(KeyCode::Backspace, false, false));
        assert_eq!(o.port.value, "25565");
        assert!(o.port.key(KeyCode::ArrowLeft, false, false));
        assert_eq!(o.port.cursor, 4);
        // LAN on: the box takes the edit.
        o.lan = true;
        o.update_port_controls();
        assert!(o.port.editable);
        assert!(o.port.key(KeyCode::Backspace, false, false));
        assert_eq!(o.port.value, "2555");
    }

    /// `WorldOptionsScreen.updatePersonalGameModeButton`: external local-mode updates refresh
    /// both `personal_mode` and its baseline without discarding unrelated edits.
    #[test]
    fn updating_personal_mode_refreshes_baseline_without_discarding_other_edits() {
        let mut o = options(false, false);
        assert_eq!((o.personal_mode, o.initial.personal_mode), (1, 1));
        assert!(!o.active(Button::Apply));
        o.difficulty = 3;
        assert!(o.active(Button::Apply));
        o.update_personal_mode(0);
        assert_eq!((o.personal_mode, o.initial.personal_mode), (0, 0));
        assert_eq!(o.difficulty, 3);
        assert!(o.active(Button::Apply));
        o.difficulty = o.initial.difficulty;
        assert!(!o.active(Button::Apply));
        o.personal_mode = 2;
        assert!(o.active(Button::Apply));
        o.update_personal_mode(3);
        assert_eq!((o.personal_mode, o.initial.personal_mode), (3, 3));
        assert!(!o.active(Button::Apply));
    }
}
