use super::*;
use sunrise_gameplay_common::stats::Stats;
use sunrise_sim::lan::guest::Guest;

/// The player that was in place before this guest's turn. Stats stay with the
/// displaced body so a synchronous death can credit it before the put-back.
pub(in crate::game) struct DisplacedPlayer {
    pub(in crate::game) body: Option<PlayerMotion>,
    pub(in crate::game) name: Option<String>,
    pub(in crate::game) uuid: Option<u128>,
    pub(in crate::game) fake: bool,
    pub(in crate::game) mode: GameMode,
    pub(in crate::game) input_sneak: bool,
    /// The borrower temporarily holds the outer player's equipment, effects,
    /// advancement progress and recipe book in its hub row.
    pub(in crate::game) record_conn: u32,
    pub(in crate::game) dimension: Dim,
    pub(in crate::game) selected: usize,
    pub(in crate::game) experience: Experience,
    pub(in crate::game) adv_loaded: bool,
    pub(in crate::game) riding: Option<u32>,
    pub(in crate::game) yaw: f32,
    pub(in crate::game) pitch: f32,
    pub(in crate::game) stats: Stats,
}

pub enum CombatPlayer<'a> {
    InPlace,
    Displaced(usize, &'a PlayerMotion),
    Parked(&'a Guest),
}
