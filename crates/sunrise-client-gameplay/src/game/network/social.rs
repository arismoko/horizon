//! `SocialInteractionsScreen` and `PlayerSocialManager` on the game side: the players Sunrise
//! knows (the tab list's), the ones whose chat the player hid, the screen's page, search and
//! scroll, and what hiding does to chat (`Minecraft.isBlocked`). The screen is
//! `social_screen`. Blocking is Mojang's service (`UserApiService.isBlockedPlayer`), which
//! Sunrise does not offer: nobody is blocked, as in vanilla's offline mode.
use super::Screen;
use std::collections::{HashMap, HashSet};
use sunrise_client::{
    social_screen::{self, Action, Entry, Page, SocialView},
    ui::EditBox,
};
use sunrise_game_data::text::Text;
use sunrise_level::profile_lookup::TabPlayer;
use winit::keyboard::KeyCode;

/// `PlayerSocialManager.hiddenPlayers` and the screen's state.
pub struct Social {
    pub hidden: HashSet<u128>,
    /// `discoveredNamesToUUID`, the other way: the names of the players seen, so a hidden player
    /// who left is still listed by name.
    pub(in crate::game) names: HashMap<u128, String>,
    pub(in crate::game) page: Page,
    pub(in crate::game) search: EditBox,
    /// `PlayerEntry.TOOLTIP_DELAY`: the row buttons' tooltips wait 500 ms.
    pub(in crate::game) tooltip: social_screen::TooltipDelay,
    pub(in crate::game) scroll: f32,
    /// Opened from the pause menu: Done goes back there.
    pub(in crate::game) from_pause: bool,
}

impl crate::game::ClientGame<'_> {
    /// The players of the level, as the tab list shows them, and the local player's id.
    pub(in crate::game::client_side) fn social_players(&mut self) -> (Vec<TabPlayer>, u128) {
        match self.remote_tab_players() {
            Some((players, _, _)) => {
                let me = self.client.remote.as_ref().map_or(0, |r| r.profile());
                (players, me)
            }
            None => (Vec::new(), 0),
        }
    }
    /// `Minecraft.isBlocked`: whether a player's chat is hidden (never the player's own).
    pub(in crate::game::client_side) fn social_hides(&self, id: u128) -> bool {
        if !self.client.social.hidden.contains(&id) {
            return false;
        }
        let me = self.client.remote.as_ref().map_or(0, |r| r.profile());
        id != me
    }
    /// `new SocialInteractionsScreen(lastScreen)`: it opens on All, with an empty search.
    pub(in crate::game::client_side) fn open_social(&mut self, from_pause: bool) {
        self.client.social.page = Page::All;
        self.client.social.search.set_value("");
        self.client.social.tooltip = Default::default();
        self.client.social.scroll = 0.0;
        self.client.social.from_pause = from_pause;
        self.set_screen(Screen::Social);
        self.capture(false);
    }
    pub fn close_social(&mut self) {
        if self.client.social.from_pause {
            self.set_screen(Screen::Paused);
        } else {
            self.set_screen(Screen::Playing);
            self.capture(true);
        }
    }
    /// The page's rows, sorted and filtered as `SocialInteractionsPlayerList` does.
    pub(in crate::game::client_side) fn social_view(&mut self) -> SocialView {
        let (players, me) = self.social_players();
        for p in &players {
            self.client.social.names.insert(p.uuid, p.name.clone());
        }
        let hidden = self.client.social.hidden.clone();
        let search = self.client.social.search.value.to_lowercase();
        let mut entries: Vec<Entry> = Vec::new();
        let online = |id: u128| players.iter().find(|p| p.uuid == id);
        let page = self.client.social.page;
        let ids: Vec<u128> = match page {
            Page::All => players.iter().map(|p| p.uuid).collect(),
            Page::Hidden => hidden.iter().copied().collect(),
            // `UserApiService.isBlockedPlayer`: nobody, without the service.
            Page::Blocked => Vec::new(),
        };
        for id in ids {
            let (name, skin, removed) = match online(id) {
                Some(p) => (p.name.clone(), p.skin.clone(), false),
                None => (
                    self.client
                        .social
                        .names
                        .get(&id)
                        .cloned()
                        .unwrap_or_default(),
                    sunrise_level::profile_lookup::default_skin(id).to_owned(),
                    true,
                ),
            };
            if !search.is_empty() && !name.to_lowercase().contains(&search) {
                continue;
            }
            let skin = self.face_skin(id, &skin);
            entries.push(Entry {
                id,
                name,
                skin,
                hidden: hidden.contains(&id),
                removed,
                local: id == me,
            });
        }
        social_screen::sort_entries(&mut entries);
        let count = players.len();
        let server = self.social_server_name();
        let key = if count > 1 {
            "gui.socialInteractions.server_label.multiple"
        } else {
            "gui.socialInteractions.server_label.single"
        };
        let args = [Text::literal(server), Text::literal(count.to_string())];
        let max = social_screen::max_scroll(self.gui_size().1, entries.len());
        self.client.social.scroll = self.client.social.scroll.clamp(0.0, max);
        SocialView {
            page,
            search: self.client.social.search.value.clone(),
            entries,
            server_label: Text::translate(key, &args),
            scroll: self.client.social.scroll,
        }
    }
    /// The server's name: a local world's name, a server's entry in the list.
    pub(in crate::game::client_side) fn social_server_name(&self) -> String {
        match (&self.client.integrated, self.server_address()) {
            (Some(server), _) => server.world_name.clone(),
            (None, Some(address)) if self.client.remote.is_some() => address.to_owned(),
            _ => self.common.level.name.clone(),
        }
    }
    pub(in crate::game::client_side) fn draw_social_screen(
        &mut self,
        batch: &mut sunrise_client::hud::HudBatch,
    ) {
        self.flush_skin_slots();
        let view = self.social_view();
        sunrise_client::widgets::begin_screen(|| {
            sunrise_client::widgets::ScreenInfo::new("social", social_screen::TITLE)
        });
        social_screen::draw_background(batch, &self.client.gui.atlas, true);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        let tooltip = social_screen::draw_contents(
            batch,
            &self.client.gui.atlas,
            self.client.cursor,
            &view,
            &mut self.client.social.search,
            &mut self.client.social.tooltip,
        );
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        if let Some(tooltip) = tooltip {
            sunrise_client::ui::tooltip(
                batch,
                &self.client.gui.atlas,
                &[tooltip],
                self.client.cursor,
            );
        }
    }
    pub(in crate::game::client_side) fn social_click(&mut self) -> bool {
        let size = self.gui_size();
        let cursor = self.client.cursor;
        let view = self.social_view();
        let Some(kind) = social_screen::click_kind(size, cursor, &view) else {
            return false;
        };
        // The widget first: the search box's `onClick` puts the caret under the
        // pointer, before any list button is considered.
        if let social_screen::Click::Search = kind {
            let shift = self.has_shift_down();
            self.client
                .social
                .search
                .click(&self.client.gui.atlas, cursor.0, shift, false);
            return true;
        }
        let social_screen::Click::Action(action) = kind else {
            return false;
        };
        self.play_audio_button_click();
        match action {
            Action::Page(page) => {
                self.client.social.page = page;
                self.client.social.scroll = 0.0;
            }
            Action::Hide(id) | Action::Show(id) => {
                let hiding = matches!(action, Action::Hide(_));
                let name = self
                    .client
                    .social
                    .names
                    .get(&id)
                    .cloned()
                    .unwrap_or_default();
                if hiding {
                    self.client.social.hidden.insert(id);
                } else {
                    self.client.social.hidden.remove(&id);
                }
                // `PlayerEntry.onHiddenOrShown`: a line in the chat, and the narrator says it.
                let key = if hiding {
                    "gui.socialInteractions.hidden_in_chat"
                } else {
                    "gui.socialInteractions.shown_in_chat"
                };
                let line = Text::translate(key, &[Text::literal(name)]);
                self.say_system_now(&line);
                self.client_system_message(line);
            }
            Action::Done => self.close_social(),
        }
        true
    }
    /// `AbstractSelectionList.mouseScrolled`: a row's height and a bit a notch.
    pub(in crate::game::client_side) fn social_scroll(&mut self, delta: f32) -> bool {
        if !self.pad_stick_scroll()
            && !self.pad_step_scroll()
            && !social_screen::list_contains(self.client.cursor, self.gui_size())
        {
            return false;
        }
        self.client.social.scroll -= delta * 18.0;
        // `social_view` clamps it to the list.
        let _ = self.social_view();
        true
    }
    /// The search box has the focus from the start (`setInitialFocus`): typing fills it.
    pub(in crate::game::client_side) fn social_key(&mut self, key: KeyCode) -> bool {
        if key == KeyCode::Escape {
            self.close_social();
            return true;
        }
        let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
        let handled = self.client.social.search.key(key, ctrl, shift);
        self.client.social.scroll = 0.0;
        handled
    }
    pub(in crate::game::client_side) fn social_backspace(&mut self) {
        self.client
            .social
            .search
            .key(KeyCode::Backspace, false, false);
        self.client.social.scroll = 0.0;
    }
    /// Characters into the search box (`searchBox.setMaxLength(16)`).
    pub(in crate::game::client_side) fn social_type(&mut self, text: &str) {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        self.client.social.search.insert_text(&text);
        self.client.social.scroll = 0.0;
    }
}

#[cfg(feature = "client")]
impl Default for Social {
    fn default() -> Self {
        let mut search = EditBox::new(0.0, 0.0, 200.0, 15.0);
        search.max_length = 16;
        Self {
            hidden: HashSet::new(),
            names: HashMap::new(),
            page: Page::All,
            search,
            tooltip: Default::default(),
            scroll: 0.0,
            from_pause: false,
        }
    }
}
