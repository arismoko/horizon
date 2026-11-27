//! The spectator menu (`SpectatorGui`): its players and teams from the
//! server's player list and teams, and the teleport request.
use crate::game::ClientGame;
use sunrise_client::{
    hud::HudBatch,
    hud_layout, keybinds,
    spectator_menu::{Member, Outcome, SpectatorMenu, Team},
};
use sunrise_game_data::{
    scoreboard::{comp_text, team_color_rgb},
    text::Text,
};
use sunrise_level::profile_lookup::{SPECTATOR, TabPlayer};
use sunrise_protocol::packets::play::{self, Serverbound as S};

impl ClientGame<'_> {
    /// `Util.getMillis` for the menu's fade.
    fn spectator_now(&self) -> i64 {
        self.client.started.elapsed().as_millis() as i64
    }
    /// `TeleportToPlayerMenuCategory` starts from listed players;
    /// `TeleportToTeamMenuCategory.TeamSelectionItem.create` looks up every team member.
    pub fn new_spectator_menu(&mut self) -> SpectatorMenu {
        let tab = self
            .remote_tab_players()
            .map_or_else(Vec::new, |(players, _, _)| players);
        let member = |p: &TabPlayer, skin: String| Member {
            uuid: p.uuid,
            name: p.name.clone(),
            skin,
            enabled: p.game_mode != SPECTATOR,
        };
        let players = tab
            .iter()
            .map(|p| member(p, self.face_skin(p.uuid, &p.skin)))
            .collect();
        let teams = self.common.score.board.teams_streamed();
        let rosters: Vec<_> = self
            .client
            .remote
            .iter()
            .flat_map(|remote| teams.iter().copied().map(move |team| (remote, team)))
            .filter_map(|(remote, team)| {
                let members: Vec<_> = team
                    .players
                    .iter()
                    .filter_map(|name| remote.player_info_by_name(name))
                    .filter(|p| p.game_mode != SPECTATOR)
                    .map(|p| member(p, remote.player_info_skin(p.uuid).to_owned()))
                    .collect();
                (!members.is_empty()).then(|| {
                    (
                        comp_text(&team.display_name),
                        team.color.as_deref().and_then(team_color_rgb),
                        members,
                    )
                })
            })
            .collect();
        let teams = rosters
            .into_iter()
            .map(|(name, color, mut players)| {
                for p in &mut players {
                    p.skin = self.face_skin(p.uuid, &p.skin);
                }
                // A random member's face is the team's icon.
                let pick = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.subsec_nanos() as usize)
                    % players.len();
                Team {
                    name,
                    color,
                    icon: players[pick].skin.clone(),
                    players,
                }
            })
            .collect();
        SpectatorMenu::new(players, teams)
    }
    pub fn spectator_outcome(&mut self, outcome: Outcome) {
        if let Outcome::Teleport(uuid) = outcome {
            // PlayerMenuItem.selectItem.
            self.send(S::TeleportToEntity(play::TeleportToEntity { uuid }));
        }
    }
    /// A hotbar key as a spectator (`SpectatorGui.onHotbarSelected`).
    pub(in crate::game::client_side) fn spectator_hotbar_key(&mut self, slot: i32) {
        let now = self.spectator_now();
        let menu = self.spectator_menu_if_needed();
        let outcome = self
            .client
            .spectator_gui
            .on_hotbar_selected(slot, now, menu);
        self.spectator_outcome(outcome);
    }
    /// `key.spectatorHotbar`: opens the menu, or uses the selected item.
    pub(in crate::game::client_side) fn spectator_action_key(&mut self) {
        let now = self.spectator_now();
        let menu = self.spectator_menu_if_needed();
        let outcome = self.client.spectator_gui.on_action_key(now, menu);
        self.spectator_outcome(outcome);
    }
    /// The menu a press would open, built only when none is up.
    fn spectator_menu_if_needed(&mut self) -> Option<SpectatorMenu> {
        (!self.client.spectator_gui.is_menu_active()).then(|| self.new_spectator_menu())
    }
    /// `MouseHandler.onScroll` in spectator mode with the menu up.
    pub(in crate::game::client_side) fn spectator_scroll(&mut self, wheel: i32) {
        let now = self.spectator_now();
        self.client.spectator_gui.on_mouse_scrolled(wheel, now);
    }
    /// `Hud.extractHotbar` for a spectator, and `extractAction`'s prompt.
    pub(in crate::game::client_side) fn draw_spectator_hotbar(
        &mut self,
        batch: &mut HudBatch,
        show_action: bool,
    ) {
        let (w, h) = (batch.width as i32, batch.height as i32);
        let now = self.spectator_now();
        let labels: Vec<Text> = (1..=9)
            .map(|n| {
                keybinds::display_name(self.client.settings.keys.get(&format!("key.hotbar.{n}")))
            })
            .collect();
        let font = &self.client.gui.atlas.font;
        let mut draws =
            self.client
                .spectator_gui
                .hotbar(w, h, now, &|slot| labels[slot].clone(), font);
        if show_action {
            draws.extend(self.client.spectator_gui.action(
                w,
                h,
                now,
                font,
                self.client.settings.text_background(0.0),
            ));
        }
        hud_layout::paint(batch, &self.client.gui.atlas, &draws);
    }
}
