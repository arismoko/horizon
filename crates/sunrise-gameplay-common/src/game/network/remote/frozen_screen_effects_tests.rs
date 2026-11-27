//! `/tick freeze` stops a client's level, not its HUD: vanilla 26.3's `Minecraft.tick` runs
//! `Gui.tick` on every tick the game isn't paused, and `Hud.tick` eases the vignette there
//! (`updateVignetteBrightness`). Only the vignette moved off the level's tick: the player's own
//! screen effects (the portal's, Nausea's, the water vision) still tick with it.

/// `Hud.vignetteBrightness` in vanilla 26.3 at the end of matched client ticks, as float bits:
/// the player's eye at sky light 15 at noon (the target is 0), read by a temporary hook in the
/// bench mod (`.local/evidence/VanillaBench-vignette.java`) every tick of a run that froze the
/// level (`tick freeze`), then unfroze it (`tick unfreeze`). Ticks 206 to 218 of that run: the
/// last running tick, then twelve frozen ones; and 266 to 271: two frozen, four running again.
pub const JAVA_FREEZING: [u32; 13] = [
    0x3dff_bfeb,
    0x3dfd_3133,
    0x3dfa_a907,
    0x3df8_2756,
    0x3df5_ac10,
    0x3df3_3724,
    0x3df0_c882,
    0x3dee_601a,
    0x3deb_fddc,
    0x3de9_a1b9,
    0x3de7_4ba0,
    0x3de4_fb82,
    0x3de2_b150,
];
pub const JAVA_UNFREEZING: [u32; 6] = [
    0x3d8b_ef63,
    0x3d8a_8927,
    0x3d89_2680,
    0x3d87_c765,
    0x3d86_6bcd,
    0x3d85_13af,
];
