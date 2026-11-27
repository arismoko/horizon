//! `MobEvent::Convert`: `Mob.convertTo` with `ConversionType.SINGLE` and the
//! `ConversionParams.AfterConversion` step the converting mob asked for.
use super::*;
use sunrise_sim::mobs::conversion;

impl crate::game::ServerGame<'_> {
    /// A mob turned into `kind` in its place (`conversion::single`). The
    /// after-step (`Pig.thunderHit`'s gear and persistence, the finalization of
    /// `Villager.thunderHit` and `Tadpole.ageUp`, the nausea a zombifying
    /// piglin or hoglin leaves behind) runs inside [`Game::convert_to`], before
    /// the new mob joins, by what the mob converted from and into; `after`
    /// only says whether the caller already asked `LivingConversionEvent.Pre`.
    pub(in crate::game) fn convert_event(
        &mut self,
        from: &Mob,
        kind: &str,
        params: conversion::Params,
        after: conversion::After,
        before_travel: bool,
    ) {
        // A conversion vanilla makes before the tick's travel takes the place the mob had then,
        // which the event's snapshot holds; any other, the live mob's.
        self.convert_to_moved(
            from,
            kind,
            params,
            after.pre_checked(),
            before_travel.then_some(from),
            |_, _| {},
        );
    }
}
