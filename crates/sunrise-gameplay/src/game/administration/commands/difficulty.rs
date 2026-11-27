use crate::game::server_side::commands::*;

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn cmd_difficulty(&mut self, ctx: &Context) -> R<i32> {
        let name = |d: u8| {
            Text::translate(
                &format!("options.difficulty.{}", DIFFICULTIES[d as usize]),
                &[],
            )
        };
        let Some(i) = DIFFICULTIES.iter().position(|d| ctx.has(d)) else {
            let current = self.common.ui.difficulty;
            self.feedback_unbroadcast(Text::translate(
                "commands.difficulty.query",
                &[name(current)],
            ));
            return Ok(i32::from(current));
        };
        let i = i as u8;
        if self.common.ui.difficulty == i {
            return Err(CommandError::new(Text::translate(
                "commands.difficulty.failure",
                &[name(i)],
            )));
        }
        self.set_difficulty_forced(i);
        self.feedback(Text::translate("commands.difficulty.success", &[name(i)]));
        Ok(0)
    }
    /// `MinecraftServer.setDifficulty(difficulty, true)`.
    pub fn set_difficulty_forced(&mut self, i: u8) {
        let from = self.common.ui.difficulty;
        self.common.ui.difficulty = i;
        // The level data holds the world's difficulty: it survives the
        // reopen, so it follows the command.
        self.common.level.set_difficulty(i);
        self.update_mob_spawning_flags();
        self.common.sync_rules();
        // MinecraftServer.setDifficulty: every player, guests included, is
        // sent the new difficulty.
        if let Some(host) = self.server.lan.as_ref() {
            host.hub
                .broadcast(&C::ChangeDifficulty(play::ChangeDifficulty {
                    difficulty: i32::from(i),
                    locked: self.common.level.difficulty_locked,
                }));
        }
        if from != i {
            let (from, to) = difficulty_names(from, i);
            self.native_difficulty_changed(from, to);
        }
    }
}
