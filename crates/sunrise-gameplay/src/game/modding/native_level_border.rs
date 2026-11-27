//! Native server writes to the resident border named by a `LevelRef`.
use super::Result;
use crate::game::{ServerGame, world_border::BorderTarget};
use mod_api::LevelRef;

fn report_border_write(level: LevelRef, operation: &str, result: Result<()>) -> bool {
    if let Err(error) = result {
        let mod_id = mod_api::content::crash::current_native_mod().unwrap_or("unknown");
        let dimension = level.dimension_id();
        log::warn!("mod {mod_id}: {operation} refused for {dimension}: {error}");
        return false;
    }
    true
}

impl ServerGame<'_> {
    /// `WorldBorder.setCenter(x, z)`; the border-center hook hears it. The centre it already
    /// has is true with nothing heard, as `/worldborder center` changes nothing. False for a
    /// centre that isn't finite or lies past 2.9999984E7 on x or z, inside the border-center
    /// hook, and for a stale handle, a level the server has not opened or a client.
    pub(in crate::game) fn level_set_world_border_center(
        &mut self,
        level: LevelRef,
        center: [f64; 2],
    ) -> bool {
        let result = self.set_border_center(BorderTarget::Handle(level), center);
        report_border_write(level, "set_world_border_center", result)
    }
    /// `setSize(size)` when `ticks` is 0, otherwise `lerpSizeBetween(current, size, ticks)`; the
    /// border-bounds hook hears it. A still border set to the size it has is true with nothing
    /// heard. False for a size outside 1 to 5.9999968E7, a negative `ticks`, inside the
    /// border-bounds hook, a stale handle, a level the server has not opened or a client.
    pub(in crate::game) fn level_set_world_border_size(
        &mut self,
        level: LevelRef,
        size: f64,
        ticks: i64,
    ) -> bool {
        let result = self.set_border_size(BorderTarget::Handle(level), size, ticks);
        report_border_write(level, "set_world_border_size", result)
    }
    /// `setDamagePerBlock` and `setSafeZone`: the damage a player outside takes per block past
    /// the safe zone, and how far outside they are safe. False for a value that is negative
    /// or not finite, a stale handle, a level the server has not opened or a client.
    pub(in crate::game) fn level_set_world_border_damage(
        &mut self,
        level: LevelRef,
        per_block: f64,
        safe_zone: f64,
    ) -> bool {
        let result = self
            .border_mut(BorderTarget::Handle(level))
            .and_then(|mut border| {
                if !per_block.is_finite() || !safe_zone.is_finite() {
                    return Err("non-finite damage or safe zone".into());
                }
                Self::set_border_damage_per_block(&mut border, per_block);
                Self::set_border_safe_zone(&mut border, safe_zone);
                Ok(())
            });
        report_border_write(level, "set_world_border_damage", result)
    }
    /// `setWarningBlocks` and `setWarningTime`: how near the border, in blocks, and how soon a
    /// shrinking border reaches the player, in ticks, before their screen tints red. False
    /// for a negative value, a stale handle, a level the server has not opened or a client.
    pub(in crate::game) fn level_set_world_border_warning(
        &mut self,
        level: LevelRef,
        distance: i32,
        ticks: i32,
    ) -> bool {
        let result = self
            .border_mut(BorderTarget::Handle(level))
            .map(|mut border| {
                Self::set_border_warning_blocks(&mut border, distance);
                Self::set_border_warning_time(&mut border, ticks);
            });
        report_border_write(level, "set_world_border_warning", result)
    }
}
