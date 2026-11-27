//! Player food and experience reads for native mods (`PlayerRef`).
use mod_api::{PlayerExperience, PlayerRef};
use sunrise_sim::entities::Experience;

pub trait NativePlayerFoodGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `FoodData.needsFood`: the food level is below 20.
    fn player_needs_food(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_food(player)?.level < 20)
    }

    /// `FoodData.hasEnoughFood`: the food level is above 6, enough to sprint.
    fn player_has_enough_food(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_food(player)?.level as f32 > 6.0)
    }

    /// `Player.canEat(always)`: an invulnerable player (creative), `always`
    /// (food that can always be eaten) or a food level below 20.
    fn player_can_eat(&self, player: PlayerRef, always: bool) -> Option<bool> {
        let invulnerable = self.player_abilities(player)?.invulnerable;
        Some(invulnerable || always || self.player_needs_food(player)?)
    }

    /// `Player.experienceLevel`, `experienceProgress` and `totalExperience`.
    fn player_experience(&self, player: PlayerRef) -> Option<PlayerExperience> {
        Some(match self.player_read_both(player)? {
            super::native_server::PlayerRead::Guest(g) => {
                PlayerExperience::from(mod_api::__private::built::PlayerExperience {
                    level: g.xp_level,
                    progress: g.xp_progress,
                    total: g.xp_total,
                })
            }
            super::native_server::PlayerRead::Acting { .. } => {
                PlayerExperience::from(mod_api::__private::built::PlayerExperience {
                    level: self.experience.level,
                    progress: self.experience.progress,
                    total: self.experience.total,
                })
            }
        })
    }

    /// `Player.getXpNeededForNextLevel`: the points the current level needs to fill.
    fn player_xp_needed_for_next_level(&self, player: PlayerRef) -> Option<i32> {
        Some(Experience::needed(self.player_experience(player)?.level))
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativePlayerFoodGameExt1<'__sg> for G {}

use crate::game::game_mode_common::GameModeCommonGameExt1 as _;

use crate::game::native_server::NativeServerGameExt1 as _;
