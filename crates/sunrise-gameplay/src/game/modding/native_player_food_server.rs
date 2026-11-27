//! Server-side player food and experience writes for native mods.
use crate::game::ServerGame;
use mod_api::PlayerRef;
use sunrise_gameplay_common::game::GameGameExt1 as _;
use sunrise_sim::entities::Experience;

impl ServerGame<'_> {
    /// Whether `player` still names the same player on this server.
    fn native_player_valid(&self, player: PlayerRef) -> bool {
        self.player_handle_read(player).is_some()
    }

    /// Runs `f` with `player`'s state in place, when the handle is current and they are alive.
    fn native_player_write(&mut self, player: PlayerRef, f: impl FnOnce(&mut Self)) -> bool {
        self.native_player_valid(player) && self.native_as_player_handle(player, f)
    }

    /// The acting player's food data, inside `native_player_write`.
    fn native_food_mut(&mut self) -> Option<&mut sunrise_world::food::FoodData> {
        self.common.player.as_mut().map(|p| &mut p.health.food)
    }

    /// `FoodData.eat(nutrition, saturationModifier)`.
    pub fn player_feed(
        &mut self,
        player: PlayerRef,
        nutrition: i32,
        saturation_modifier: f32,
    ) -> bool {
        self.native_player_write(player, |game| {
            if let Some(food) = game.native_food_mut() {
                food.eat(nutrition, saturation_modifier);
            }
        })
    }

    /// `Player.causeFoodExhaustion`.
    pub(in crate::game) fn player_add_exhaustion(
        &mut self,
        player: PlayerRef,
        amount: f32,
    ) -> bool {
        self.native_player_write(player, |game| {
            if let Some(p) = game.player.as_mut() {
                p.cause_food_exhaustion(amount);
            }
        })
    }

    /// `FoodData.setFoodLevel`.
    pub fn player_set_food_level(&mut self, player: PlayerRef, level: i32) -> bool {
        self.native_player_write(player, |game| {
            if let Some(food) = game.native_food_mut() {
                food.food = level;
            }
        })
    }

    /// `FoodData.setSaturation`.
    pub(in crate::game) fn player_set_saturation(
        &mut self,
        player: PlayerRef,
        saturation: f32,
    ) -> bool {
        self.native_player_write(player, |game| {
            if let Some(food) = game.native_food_mut() {
                food.saturation = saturation;
            }
        })
    }

    /// Sets the exhaustion as it is (Paper's `HumanEntity#setExhaustion`).
    pub(in crate::game) fn player_set_exhaustion(
        &mut self,
        player: PlayerRef,
        exhaustion: f32,
    ) -> bool {
        self.native_player_write(player, |game| {
            if let Some(food) = game.native_food_mut() {
                food.exhaustion = exhaustion;
            }
        })
    }

    /// `ServerPlayer.setExperiencePoints`.
    pub(in crate::game) fn player_set_experience_points(
        &mut self,
        player: PlayerRef,
        points: i32,
    ) -> bool {
        self.native_player_write(player, |game| {
            let limit = Experience::needed(game.experience.level) as f32;
            let max = (limit - 1.0) / limit;
            // Mth.clamp delegates to Math.min for the upper bound, preserving NaN.
            let progress = points as f32 / limit;
            game.experience.progress = if progress < 0.0 {
                0.0
            } else if progress.is_nan() {
                progress
            } else {
                progress.min(max)
            };
        })
    }

    /// `ServerPlayer.setExperienceLevels`.
    pub fn player_set_experience_levels(&mut self, player: PlayerRef, levels: i32) -> bool {
        self.native_player_write(player, |game| game.set_experience_levels(levels))
    }

    /// `Player.totalExperience`.
    pub fn player_set_total_experience(&mut self, player: PlayerRef, total: i32) -> bool {
        self.native_player_write(player, |game| game.experience.total = total)
    }

    /// `ServerPlayer.giveExperiencePoints`: adds or takes signed points. Mending is part of
    /// `ExperienceOrb.playerTouch`, not this write.
    pub fn player_give_experience_points(&mut self, player: PlayerRef, points: i32) -> bool {
        self.native_player_write(player, |game| game.give_experience_points(points))
    }

    /// `ExperienceOrb.repairPlayerItems` for `player` (#5593): the orb pickup's Mending, the
    /// points left over.
    pub fn player_repair_items_with_xp(&mut self, player: PlayerRef, points: i32) -> Option<i32> {
        let mut rest = None;
        self.native_player_write(player, |game| rest = Some(game.mending(points)));
        rest
    }

    /// `Player.giveExperienceLevels`.
    pub fn player_give_levels(&mut self, player: PlayerRef, levels: i32) -> bool {
        self.native_player_write(player, |game| game.give_levels(levels))
    }
}
