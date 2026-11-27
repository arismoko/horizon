//! Portable simulation/server settings subset extracted from `settings::Settings`.
use crate::tick_mode::ParallelTickMode;

/// Sunrise's render distance cap (vanilla stops at 32): the server protocol's
/// view distance tops out at 127 anyway.
pub const MAX_RENDER_DISTANCE: i32 = 128;

pub use sunrise_game_data::sound_source::{
    SOUND_SOURCES, ServerSound, sound_source_for_event as sound_source,
};

/// Simulation- and server-relevant player settings (`entity_distance` stays `f64` per vanilla `Options.entityDistanceScaling`).
#[derive(Clone, Debug, PartialEq)]
pub struct SimSettings {
    pub render_distance: i32,
    pub simulation_distance: i32,
    pub entity_distance: f64,
    pub parallel_tick: ParallelTickMode,
    pub auto_jump: bool,
    pub main_hand_left: bool,
    pub chat_visibility: u8,
    pub operator_items_tab: bool,
    pub reduced_debug_info: bool,
    pub language: String,
    pub mods: sunrise_mod_package::mod_list::ModList,
    pub no_ai: bool,
    pub agent_control: bool,
}

impl Default for SimSettings {
    fn default() -> Self {
        Self {
            render_distance: 12,
            simulation_distance: 12,
            entity_distance: 1.0,
            parallel_tick: ParallelTickMode::Off,
            auto_jump: false,
            main_hand_left: false,
            chat_visibility: 0,
            operator_items_tab: false,
            reduced_debug_info: false,
            language: "en_us".into(),
            mods: Default::default(),
            no_ai: false,
            agent_control: false,
        }
    }
}
