use crate::game::ServerGame;
use sunrise_game_data::Result;

impl ServerGame<'_> {
    /// `sunrise-ctl save` and `save-all`.
    pub(in crate::game) fn control_save(&mut self, all: bool) -> Result<()> {
        if all {
            let keys: Vec<_> = self.common.world.chunks.keys().copied().collect();
            self.common.world.modified.extend(keys);
        }
        self.save_game()
    }
}
