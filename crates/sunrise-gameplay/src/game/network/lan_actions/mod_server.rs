use super::{
    Dim, GuestUse, Hit, InteractTarget, Mining,
    building_players::{BuildingPlayer, HubBodies, shared},
    lan::LanHost,
    lan_crafting::MenuRemoval,
    respawn_anchor::RespawnConfig,
};
use mod_api::{ActionRefused, GameMode, SoundSource};
use sunrise_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};
use sunrise_gameplay_common::game::lan_actions::GuestCommand;
use sunrise_gameplay_common::game::lan_actions::MenuOpen;
use sunrise_gameplay_common::game::lan_actions::RECIPE_CHECK_LIMIT;
use sunrise_gameplay_common::game::lan_actions::TurnRefused;
use sunrise_gameplay_common::game::lan_actions::bundle_inventory_slot;
use sunrise_gameplay_common::game::lan_actions::eye_columns;
use sunrise_gameplay_common::game::lan_actions::first_spin_target;
use sunrise_gameplay_common::game::lan_actions::guest_within_entity_range;
use sunrise_gameplay_common::game::lan_actions::reset_use_attack_strength;
use sunrise_gameplay_common::game::lan_actions::swing_packet;
use sunrise_gameplay_common::game::lan_actions::touching_unloaded_chunk;
use sunrise_gameplay_common::game::lan_actions::visible_aim_point;
use sunrise_gameplay_common::game::level_sounds_common::LevelSound;
use sunrise_protocol::{
    packets::play::{self, Clientbound as C, Serverbound as S},
    text::literal,
};
use sunrise_world::entity_move::EntityMoveWorld;
use {
    sunrise_items::items::ItemStack, sunrise_level::containers::BlockEntity,
    sunrise_sim::entities::Experience, sunrise_sim::first_person_sim::Hand,
    sunrise_sim::first_person_sim::SwingAnimation, sunrise_sim::lan::guest::SPECTATOR,
    sunrise_sim::lan::hub::TickCtx, sunrise_sim::lan::menu::Rules, sunrise_sim::lan::menu::Slots,
    sunrise_sim::motion::MotionEvent, sunrise_sim::motion::MotionWorld,
    sunrise_sim::motion::PlayerMotion, sunrise_sim::pose::Dimensions,
};

#[path = "attack_checks_server.rs"]
pub(in crate::game) mod attack_checks;

#[path = "commands_server.rs"]
pub(in crate::game) mod commands;

#[path = "death_state_server.rs"]
pub(in crate::game) mod death_state;

#[path = "effects_server.rs"]
pub(in crate::game) mod effects;

#[path = "guest_body_server.rs"]
pub(in crate::game) mod guest_body;

#[path = "hub_click_server.rs"]
pub(in crate::game) mod hub_click;

#[path = "interaction_server.rs"]
pub(in crate::game) mod interaction;

#[path = "menus_server.rs"]
pub(in crate::game) mod menus;
#[path = "menus.rs"]
pub(in crate::game) mod menus_host;

#[path = "mining_server.rs"]
pub(in crate::game) mod mining;
#[path = "mining.rs"]
pub(in crate::game) mod mining_host;

#[path = "movement_server.rs"]
pub(in crate::game) mod movement;

#[path = "packets_server.rs"]
pub(in crate::game) mod packets;

#[path = "pickups_server.rs"]
pub(in crate::game) mod pickups;

#[path = "respawn_server.rs"]
pub(in crate::game) mod respawn;
#[path = "respawn.rs"]
pub(in crate::game) mod respawn_host;

#[path = "sleep_server.rs"]
pub(in crate::game) mod sleep;

#[path = "tick_server.rs"]
pub mod tick;

#[path = "turns.rs"]
pub mod turn_state;
#[path = "turns_server.rs"]
pub(in crate::game) mod turns;
pub(in crate::game) use interaction::Struck;
pub(in crate::game) use turn_state::{CombatPlayer, DisplacedPlayer};

include!("shared_server.rs");
