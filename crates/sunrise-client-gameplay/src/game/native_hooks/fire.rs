use crate::game::native_mob_draw::{MobRender, PlayerRender};
use mod_api::{
    ActiveEffect, Answered, BlockEntityDataSource, BlockEntityDataView, BlockRim,
    ClientMiningPrediction, ClientPacket, ClientReloadListeners, ColoredLight,
    EmissiveSpriteRegistration, EntityLayers, EntityRef, FrameModel, FrameModelEdit, GameMode,
    GuiCanvas, GuiLayerRegistrar, HandDrawContext, HandRender, Identifier, InteractionAttempt,
    InteractionKey, LevelRef, NameTagLine, PanelEvent, PlayerRef, RandomSource, ScreenOpening,
    ScreenView, ScreenWidgets, ScreenshotImage, SoundPlay, TooltipElement, TriState,
};
use std::time::Duration;
use sunrise_client::music::{Music, MusicLayer};
use sunrise_core::{geometry::Direction, position::BlockPos};
use sunrise_game_data::text::Text;
use sunrise_items::items::ItemStack;
use sunrise_level::{biome::BiomeColors, particle_options::ParticleOptions};
use sunrise_sim::first_person_sim::Hand;
use sunrise_world::registry::BlockStateId;

include!(concat!(env!("OUT_DIR"), "/mod_api_fire_generated.rs"));

#[cfg(feature = "test-support")]
impl crate::game::ClientGame<'_> {
    pub fn test_fire_block_highlight(
        &self,
        pos: [i32; 3],
        state: BlockStateId,
        partial: f32,
        canceled: &mut bool,
    ) {
        self.fire_block_highlight(pos, state, partial, canceled);
    }

    pub fn test_fire_screen_exclusion_areas(
        &self,
        screen: &ScreenView<'_>,
        areas: &mut Vec<[f32; 4]>,
    ) {
        self.fire_screen_exclusion_areas(screen, areas);
    }

    pub fn test_fire_client_tick_pre(&mut self) {
        self.fire_client_tick_pre();
    }
}
