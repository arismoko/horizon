//! The parentless demo popup and hints sent by `ClientPacketListener.handleGameEvent`.
use super::Screen;
use sunrise_client::{
    hud::{HudAtlas, HudBatch},
    keybinds::{KeyBinds, display_name},
    ui, widgets,
};
use sunrise_game_data::text::Text;

const MOVEMENT: [&str; 4] = ["key.forward", "key.left", "key.back", "key.right"];
const BUY_MINECRAFT_JAVA: &str = "https://aka.ms/BuyMinecraftJava";
// PopupScreen's default width minus two 18-pixel borders, and its row spacing.
const CONTENT_WIDTH: i32 = 214;
const BUTTON_SPACE: f32 = 6.0;
const BUTTON_WIDTH: f32 = 104.0;
const BUTTON_HEIGHT: f32 = 20.0;
const SECTION_SPACE: f32 = 12.0;
const LINE_HEIGHT: f32 = 9.0;

pub struct DemoIntro {
    pub title: Text,
    pub messages: [Text; 2],
    pub buttons: [Text; 2],
}

impl DemoIntro {
    /// `ClientPacketListener.openDemoIntroScreen` captures the current bindings.
    fn new(keys: &KeyBinds) -> Self {
        let key = |name| display_name(keys.get(name));
        let controls = Text::empty()
            .append(Text::translate(
                "demo.help.movementShort",
                &MOVEMENT.map(key),
            ))
            .append("\n")
            .append(Text::translate("demo.help.movementMouse", &[]))
            .append("\n")
            .append(Text::translate("demo.help.jump", &[key("key.jump")]))
            .append("\n")
            .append(Text::translate(
                "demo.help.inventory",
                &[key("key.inventory")],
            ));
        Self {
            title: Text::translate("demo.help.title", &[]),
            messages: [controls, Text::translate("demo.help.fullWrapped", &[])],
            buttons: [
                Text::translate("demo.help.buy", &[]),
                Text::translate("demo.help.later", &[]),
            ],
        }
    }

    pub fn layout(&self, atlas: &HudAtlas, width: f32, height: f32) -> Layout {
        let sections = [
            atlas
                .font
                .wrap_components(&self.title.clone().bold(), CONTENT_WIDTH),
            atlas.font.wrap_components(&self.messages[0], CONTENT_WIDTH),
            atlas.font.wrap_components(&self.messages[1], CONTENT_WIDTH),
        ];
        let content_width = sections
            .iter()
            .flatten()
            .map(|line| atlas.font.width_text(line).ceil())
            .fold(2.0 * BUTTON_WIDTH + BUTTON_SPACE, f32::max);
        let button_y =
            sections.iter().map(Vec::len).sum::<usize>() as f32 * LINE_HEIGHT + 3.0 * SECTION_SPACE;
        let content_height = button_y + BUTTON_HEIGHT;
        let x = ((width - content_width) / 2.0).trunc();
        let y = ((height - content_height) / 2.0).trunc();
        let button_x = x + ((content_width - 2.0 * BUTTON_WIDTH - BUTTON_SPACE) / 2.0).trunc();
        Layout {
            sections,
            panel: [
                x - 18.0,
                y - 18.0,
                content_width + 36.0,
                content_height + 36.0,
            ],
            center: x + (content_width / 2.0).trunc(),
            top: y,
            button_y: y + button_y,
            button_x: [button_x, button_x + BUTTON_WIDTH + BUTTON_SPACE],
        }
    }
}

pub struct Layout {
    sections: [Vec<Text>; 3],
    panel: [f32; 4],
    center: f32,
    top: f32,
    pub button_y: f32,
    pub button_x: [f32; 2],
}

impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn apply_demo_event(&mut self, param: f32) {
        // DEMO_EVENT compares the raw float; signed zero matches, but rounded
        // neighbors, unknown phases and NaN do not.
        if param == 0.0 {
            let intro = DemoIntro::new(&self.client.settings.keys);
            if self.client.screen == Screen::DemoIntro {
                self.client.demo_intro = Some(intro);
                self.commit_screen_transition();
            } else {
                self.set_screen_with(Screen::DemoIntro, |game| {
                    game.client.demo_intro = Some(intro)
                });
            }
            if self.client.screen == Screen::DemoIntro {
                self.capture(false);
            }
            return;
        }
        let (message, mappings): (&str, &[&str]) = match param {
            101.0 => ("demo.help.movement", &MOVEMENT),
            102.0 => ("demo.help.jump", &["key.jump"]),
            103.0 => ("demo.help.inventory", &["key.inventory"]),
            104.0 => ("demo.day.6", &["key.screenshot"]),
            _ => return,
        };
        let args: Vec<_> = mappings
            .iter()
            .map(|mapping| display_name(self.client.settings.keys.get(mapping)))
            .collect();
        let message = Text::translate(message, &args);
        self.add_chat_line(message.clone(), sunrise_client::chat::ChatSource::Client);
        self.say_system_queued(&message);
    }

    pub(in crate::game::client_side) fn draw_demo_intro(&self, batch: &mut HudBatch) {
        let Some(intro) = &self.client.demo_intro else {
            return;
        };
        let atlas = &self.client.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        let layout = intro.layout(atlas, w, h);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        let [x, y, width, height] = layout.panel;
        ui::nine_slice(
            batch,
            atlas,
            "popup/background",
            x,
            y,
            width,
            height,
            u32::MAX,
        );
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        widgets::begin_screen(|| {
            // PopupScreen.getNarrationMessage joins the title with both sections.
            let [controls, info] = &intro.messages;
            let message = format!(
                "{}. {}\n{}",
                intro.title.plain(),
                controls.plain(),
                info.plain()
            );
            widgets::ScreenInfo::new(Screen::DemoIntro.id(), message)
        });
        let mut y = layout.top;
        for section in layout.sections {
            for line in section {
                let x = layout.center - (atlas.font.width_text(&line) / 2.0).trunc();
                batch.text_styled(atlas, &line, x, y, u32::MAX, true);
                y += LINE_HEIGHT;
            }
            y += SECTION_SPACE;
        }
        for (x, label) in layout.button_x.into_iter().zip(&intro.buttons) {
            let y = layout.button_y;
            ui::button(
                batch,
                atlas,
                x,
                y,
                BUTTON_WIDTH,
                BUTTON_HEIGHT,
                label,
                true,
                ui::hovered(self.client.cursor, x, y, BUTTON_WIDTH, BUTTON_HEIGHT),
            );
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
    }

    pub fn demo_intro_click(&mut self) -> bool {
        let Some(intro) = &self.client.demo_intro else {
            return false;
        };
        let (w, h) = self.gui_size();
        let layout = intro.layout(&self.client.gui.atlas, w, h);
        let Some(button) = layout.button_x.into_iter().position(|x| {
            ui::hovered(
                self.client.cursor,
                x,
                layout.button_y,
                BUTTON_WIDTH,
                BUTTON_HEIGHT,
            )
        }) else {
            return false;
        };
        self.play_audio_button_click();
        if button == 0 {
            self.confirm_trusted_link(BUY_MINECRAFT_JAVA, Screen::Playing);
        } else {
            self.close_demo_intro();
        }
        true
    }
    /// Closes the parentless popup to the HUD: `PopupScreen.onClose` is
    /// `Gui.setScreen(null)` for a popup without a parent, whose tail is
    /// `MouseHandler.grabMouse`, and Later's button calls `grabMouse` before
    /// `onClose`. The grab is asked for once the screen hooks let the close go
    /// ahead, as every other close to the HUD asks for it.
    pub(in crate::game::client_side) fn close_demo_intro(&mut self) {
        self.set_screen(Screen::Playing);
        if self.client.screen == Screen::Playing {
            self.capture(true);
        }
    }
}
