//! Glowing entities (`EntityRenderer.extractRenderState`'s outlineColor): which entities
//! appear glowing, in which team colour, and their silhouettes for the renderer's entity
//! outline (`OutlineBufferSource`).
use super::Vertex;
use sunrise_game_data::scoreboard as sb;
use sunrise_sim::mobs::Mob;

/// `Entity.getTeamColor`: the team's colour, else white.
pub(in crate::game::client_side) fn team_color(team: Option<&sb::Team>) -> u32 {
    team.and_then(|t| t.color.as_deref())
        .and_then(sb::team_color_rgb)
        .unwrap_or(0xFF_FFFF)
}

impl crate::game::ClientGame<'_> {
    /// `Entity.isCurrentlyGlowing` of an entity that is not living (an item, a projectile):
    /// shared flag 6 as a server sent it.
    pub(in crate::game) fn tracked_entity_glowing(&self, id: u32) -> bool {
        self.remote_glowing(id).unwrap_or(false)
    }

    /// The outline an entity is drawn with: vanilla's (`Some` team colour when it glows),
    /// then what mods made of it (`native_entity_outline`).
    pub(in crate::game::client_side) fn entity_outline(
        &self,
        id: u32,
        kind: &str,
        vanilla: Option<u32>,
    ) -> Option<u32> {
        let mut outline = vanilla;
        self.native_entity_outline(id, kind, &mut outline);
        outline.map(|rgb| rgb & 0xFF_FFFF)
    }

    /// The outline of a dropped item or projectile: its `Glowing` flag, in its team's colour.
    pub(in crate::game::client_side) fn thing_outline(&self, id: u32, kind: &str) -> Option<u32> {
        let vanilla = self
            .tracked_entity_glowing(id)
            .then(|| team_color(self.client_mob_team(id)));
        self.entity_outline(id, kind, vanilla)
    }

    /// `Minecraft.shouldEntityAppearGlowing` of a mob or another player, with its outline
    /// colour (`getTeamColor`; another player's team goes by name).
    pub(in crate::game::client_side) fn mob_outline(&self, m: &Mob) -> Option<u32> {
        let vanilla = m.is_currently_glowing().then(|| {
            let team = if &*m.kind.id == "minecraft:player" {
                let names = self.remote_player_names();
                let name = names.into_iter().find(|(id, _)| *id == m.id);
                name.and_then(|(_, name)| self.common.score.board.players_team(&name))
            } else {
                self.client_mob_team_of(m)
            };
            team_color(team)
        });
        self.entity_outline(m.id, &m.kind.id, vanilla)
    }

    /// Adds one glowing entity's geometry to the outline pass in `rgb`, the textures'
    /// alpha kept (`OutlineBufferSource`'s team-coloured copy of each draw).
    pub(in crate::game::client_side) fn mark_outline(&self, vertices: &[Vertex], rgb: u32) {
        let color = sunrise_client::mobs::render::abgr(rgb);
        self.client.glowing_vertices.borrow_mut().extend(
            vertices
                .iter()
                .filter(|v| {
                    v.layer & 0xffff != sunrise_client_render::gfx::ARMOR_GLINT_ONLY_OVERLAY
                })
                .map(|v| Vertex {
                    color,
                    normal: [0; 4],
                    ..*v
                }),
        );
    }
}
