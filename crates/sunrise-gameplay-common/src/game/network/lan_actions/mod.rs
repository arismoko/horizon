//! What guests do in a hosted world. The host's gameplay code is written for
//! the one local player, so a guest's action runs through it with the guest's
//! state swapped in for the duration of the call (position, inventory, game
//! mode, experience, mining and item use): digging, placing, using items and
//! blocks, attacking and the commands all behave exactly as they do for the
//! host, and every world change they cause reaches all players through the
//! block log, the level events and the entity tracker.
use sunrise_core::geometry::Aabb;
#[allow(unused_imports)]
pub use sunrise_protocol::packets::play::{self, Clientbound as C, Serverbound as S};
#[allow(unused_imports)]
use {
    sunrise_items::items::ItemStack, sunrise_sim::entities::Experience,
    sunrise_sim::first_person_sim::Hand, sunrise_sim::first_person_sim::SwingAnimation,
    sunrise_sim::lan::guest::SPECTATOR, sunrise_sim::lan::menu::Slots,
    sunrise_sim::motion::PlayerMotion, sunrise_sim::pose::Dimensions,
};

pub mod attack_checks;
pub(in crate::game) mod commands;
pub mod death_state;
pub(in crate::game) mod effects;
pub mod guest_body;
pub(in crate::game) mod hub_click;
pub mod movement;
pub mod packets;
pub(in crate::game) mod pickups;
pub(in crate::game) mod sleep;
pub mod tick;
include!("shared.rs");
