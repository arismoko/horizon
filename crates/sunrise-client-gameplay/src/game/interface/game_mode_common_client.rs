use crate::game::ClientGame;
use sunrise_command::args::GameMode;
use sunrise_items::items::ItemStack;

impl ClientGame<'_> {
    /// `MultiPlayerGameMode.canHurtPlayer()` and `hasExperience()` (`localPlayerMode.isSurvival()`).
    pub fn can_hurt_player(&self) -> bool {
        matches!(self.mode, GameMode::Survival | GameMode::Adventure)
    }
    /// `ItemStack.canPlaceOnBlockInAdventureMode` of the stack a use names.
    pub(in crate::game::client_side) fn stack_can_place_on_in_adventure(
        &self,
        stack: &ItemStack,
        clicked: [i32; 3],
    ) -> bool {
        self.stack_adventure_predicate(stack, "minecraft:can_place_on", clicked)
    }
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;
