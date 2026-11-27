//! `RaidCommand`: `/raid start|stop|check|spawnleader|setomen|glow|sound`.
use super::{R, Text, err};
use sunrise_command::{Arg, Context};
use sunrise_core::random::RandomSource;
use sunrise_sim::mobs::raider;

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn cmd_raid(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let Some(player) = self.common.player.as_ref().map(|p| p.position) else {
            return Ok(0);
        };
        let at = [
            player.x.floor() as i32,
            player.y.floor() as i32,
            player.z.floor() as i32,
        ];
        let fail = |game: &mut Self, message: &str| {
            game.command_failure(Text::literal(message));
        };
        match id {
            "raid.start" => {
                if self.raiding_nearby(at).is_some() {
                    fail(self, "Raid already started close by");
                    return Ok(-1);
                }
                let level = Self::int(ctx, "omenlvl").unwrap_or(0);
                match self.raid_command_create(at, level) {
                    true => self.feedback(Text::literal("Created a raid in your local village")),
                    false => fail(self, "Failed to create a raid in your local village"),
                }
                Ok(1)
            }
            "raid.stop" => match self.raiding_nearby(at) {
                Some(raid) => {
                    self.raid_command_stop(raid);
                    self.feedback(Text::literal("Stopped raid"));
                    Ok(1)
                }
                None => {
                    fail(self, "No raid here");
                    Ok(-1)
                }
            },
            // RaidCommand.getRaid is getRaidAt: a raid still counting down is found too (#11372).
            "raid.check" => match self.raiding_nearby(at) {
                Some(raid) => {
                    self.feedback(Text::literal("Found a started raid!"));
                    let status = self.raid_status_text(raid);
                    self.feedback(Text::literal(&status));
                    Ok(1)
                }
                None => {
                    fail(self, "Found no started raids");
                    Ok(0)
                }
            },
            "raid.spawnleader" => {
                let kind = "minecraft:pillager";
                match self.spawn_mob(
                    kind,
                    [player.x, player.y, player.z],
                    true,
                    sunrise_gameplay_common::game::creatures::SpawnSite::Finalizing(
                        sunrise_sim::mobs::spawn_rules::Reason::Command,
                    ),
                ) {
                    Some(id) => {
                        if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                            raider::set_patrol_leader(m, true);
                            raider::finalize(m, raider::Reason::Other);
                        }
                        self.feedback(Text::literal("Spawned a raid captain"));
                        Ok(1)
                    }
                    None => {
                        fail(self, "Pillager failed to spawn");
                        Ok(0)
                    }
                }
            }
            "raid.setomen" => {
                let level = Self::int(ctx, "level").unwrap_or(0);
                let Some(raid) = self.raiding_nearby(at) else {
                    fail(self, "No raid found here");
                    return Ok(1);
                };
                if level > sunrise_sim::raid::MAX_OMEN_LEVEL {
                    fail(
                        self,
                        &format!(
                            "Sorry, the max raid omen level you can set is {}",
                            sunrise_sim::raid::MAX_OMEN_LEVEL
                        ),
                    );
                } else {
                    let before = self.raid_set_omen(raid, level);
                    self.feedback(Text::literal(format!(
                        "Changed village's raid omen level from {before} to {level}"
                    )));
                }
                Ok(1)
            }
            "raid.glow" => {
                if let Some(raid) = self.raiding_nearby(at) {
                    self.raid_glow(raid);
                }
                Ok(1)
            }
            // RaidCommand.playSound: only for "local", at volume 2, with a seeded level random.
            "raid.sound" => {
                // ComponentArgument: `local`, "local" and 'local' all mean "local".
                let kind = match ctx.get("type") {
                    Some(Arg::Str(s)) => s.trim(),
                    _ => "",
                };
                if matches!(kind, "local" | "\"local\"" | "'local'") {
                    let seed = self.common.world.random.next_long();
                    let at = [player.x + 5.0, player.y, player.z];
                    self.server_play_sound_seeded("minecraft:event.raid.horn", at, 2.0, 1.0, seed);
                }
                Ok(1)
            }
            _ => Err(err("commands.help.failed", &[])),
        }
    }
}
