use crate::game::ServerGame;
use sunrise_core::position::BlockPos;
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::powder_snow::extinguish_pitch;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::Who;

impl ServerGame<'_> {
    /// Entity.playEntityOnFireExtinguishedSound
    pub(in crate::game) fn extinguish_sound(&mut self, at: [f64; 3]) {
        let pitch = extinguish_pitch(&mut self.server.effect_rng);
        self.server_play_sound("minecraft:entity.generic.extinguish_fire", at, 0.7, pitch);
    }
    /// `PowderSnowBlock.entityInside` (runBefore EXTINGUISH) for a burning player.
    pub(in crate::game) fn powder_snow_melt(&mut self, pos: BlockPos) {
        let at = [pos.x, pos.y, pos.z];
        if !self.may_interact_at(pos) {
            return;
        }
        self.with_level(|b, level, ticks| b.destroy_block(level, ticks, pos, false));
        let _ = self.server_process_level_events();
        self.common.world.modified.insert((at[0] >> 4, at[2] >> 4));
    }
    /// `BucketItem.use` with an empty bucket on powder snow:
    /// `PowderSnowBlock.pickupBlock` (air, level event 2001), the used
    /// statistic, the fill sound, FLUID_PICKUP, `ItemUtils.createFilledResult`
    /// and the filled_bucket trigger.
    pub(in crate::game) fn pick_up_powder_snow(&mut self, pos: [i32; 3]) -> Result<()> {
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let creative = self.common.creative;
        let picked = self.with_level(|b, level, ticks| b.bucket_pickup(level, ticks, at, creative));
        self.server_process_level_events()?;
        let Some(filled) = picked else {
            return Ok(());
        };
        self.common
            .world
            .modified
            .insert((pos[0] >> 4, pos[2] >> 4));
        self.award_stat("used", "minecraft:bucket", 1);
        if let Some(eye) = self
            .common
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z])
        {
            self.server_play_sound("minecraft:item.bucket.fill_powder_snow", eye, 1.0, 1.0);
        }
        self.post_block_game_event("minecraft:fluid_pickup", pos, Some(Who::Player), None);
        let filled = ItemStack::new(filled, 1);
        self.filled_result(filled.clone());
        self.trigger_item("minecraft:filled_bucket", &filled);
        self.swing_arm();
        Ok(())
    }
    /// `Entity.applyEffectsFromBlocks`: a burning player in the rain
    /// (`isInRain`) has the fire put out (`clearFire`), with the extinguish
    /// sound that everyone near hears.
    pub(in crate::game::server_side) fn rain_extinguish_player(&mut self) {
        let Some(player) = self.common.player.as_ref() else {
            return;
        };
        let (at, top) = (player.position, player.bounds().max_position().y);
        if player.fire_ticks <= 0 || !self.in_rain(at, top) {
            return;
        }
        if let Some(player) = self.common.player.as_mut() {
            player.fire_ticks = 0;
        }
        self.extinguish_sound([at.x, at.y, at.z]);
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::powder_snow::FilledResultGameExt as _;
