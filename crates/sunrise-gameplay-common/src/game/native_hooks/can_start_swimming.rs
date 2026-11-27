use super::*;
use mod_api::SwimmingCheck;
use sunrise_sim::{motion::SwimEntry, sprint};

impl crate::game::CommonGame<'_> {
    pub fn has_swim_eligibility_listener(&self) -> bool {
        mod_api::__private::listened(mod_api::HookId::CanStartSwimming)
    }
}

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// The first answer replaces the shared player swim predicate at this call site.
    fn native_can_start_swimming(
        &self,
        player: PlayerRef,
        side: LogicalSide,
        check: mod_api::SwimmingCheck,
        can_start_swimming: &mut Option<bool>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_can_start_swimming(
            self,
            player,
            side,
            check,
            can_start_swimming,
        );
    }

    fn ask_swim_entry(&self, step: Option<SwimEntry>) -> Option<(SwimEntry, bool)> {
        let step = step?;
        self.side_ask_swim_eligibility(SwimmingCheck::Entry)
            .map(|answer| (step, answer))
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}

use crate::game::side_ops::modapi_hooks::ModapiHooksSideGameExt as _;

pub trait SprintGameExt<'g>: crate::game::SideGame<'g> {
    /// `LocalPlayer.isSprintingPossible` asks about shallow water only once
    /// its earlier checks pass. A sprint setter can precede a second query.
    fn player_sprint_tick(
        &mut self,
        keys: sprint::Keys,
        window: i32,
        vehicle_sprint: Option<(bool, bool)>,
    ) -> sunrise_game_data::Result<()> {
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        if !mod_api::__private::listened(mod_api::HookId::CanStartSwimming) {
            return match vehicle_sprint {
                Some((can_sprint, local)) => {
                    player.sprint_input_tick_riding(keys, window, can_sprint, local)
                }
                None => player.sprint_input_tick_with_window(keys, window),
            };
        }
        let mut state = player.sprint_input;
        let mut conditions = player.sprint_conditions();
        if let Some((can_sprint, local)) = vehicle_sprint {
            conditions.passenger = true;
            conditions.vehicle_can_sprint = can_sprint;
            conditions.vehicle_local = local;
        }
        let result = state.tick_with_context(
            keys,
            conditions,
            window,
            self,
            |game, vanilla| {
                // Only water has a swimmable fluid type in the current fluid table.
                vanilla
                    || (game
                        .player
                        .as_ref()
                        .is_some_and(|p| p.fluid_height(sunrise_sim::motion::WATER) > 0.0)
                        && game.side_ask_swim_eligibility(SwimmingCheck::ShallowWater)
                            == Some(false))
            },
            |game, value| {
                game.player
                    .as_mut()
                    .expect("sprint owns the player")
                    .set_sprinting(value)
            },
        );
        self.player
            .as_mut()
            .expect("sprint owns the player")
            .sprint_input = state;
        result
    }
}
impl<'g, G: crate::game::SideGame<'g>> SprintGameExt<'g> for G {}
