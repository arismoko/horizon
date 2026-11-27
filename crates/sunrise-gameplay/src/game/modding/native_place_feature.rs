//! A feature placed by a native mod (`LevelRef::place_feature`), as `/place feature` places one:
//! `PlaceCommand.placeFeature` without the command's feedback.
use crate::game::ServerGame;
use mod_api::{LevelReadError, LevelRef, PlaceFeatureRefusal};

impl ServerGame<'_> {
    /// `Feature.place` of `feature` at `pos` in the level in place: the feature's own report of
    /// success. Blocks it set before reporting `false` are kept, as the command keeps them.
    pub(in crate::game) fn level_place_feature(
        &mut self,
        level: LevelRef,
        feature: &str,
        pos: [i32; 3],
    ) -> Result<bool, PlaceFeatureRefusal> {
        self.place_feature_by_id(level, feature, pos, false)
    }

    /// `PlacedFeature.place` of `placed_feature` at `pos` in the level in place:
    /// its modifiers from `pos`, then its feature at each position.
    /// `Ok(false)` when no position remains or every feature invocation reports failure.
    pub(in crate::game) fn level_place_placed_feature(
        &mut self,
        level: LevelRef,
        placed_feature: &str,
        pos: [i32; 3],
    ) -> Result<bool, PlaceFeatureRefusal> {
        self.place_feature_by_id(level, placed_feature, pos, true)
    }

    /// The refusals and the placement both calls share; `placed` picks the registry.
    fn place_feature_by_id(
        &mut self,
        level: LevelRef,
        feature: &str,
        pos: [i32; 3],
        placed: bool,
    ) -> Result<bool, PlaceFeatureRefusal> {
        match self.native_level_world_result(level) {
            Err(LevelReadError::Unavailable) => return Err(PlaceFeatureRefusal::Client),
            Err(_) => return Err(PlaceFeatureRefusal::Stale),
            Ok(_) if level.dimension() != self.common.dim.key() => {
                return Err(PlaceFeatureRefusal::NotInPlace);
            }
            Ok(_) => {}
        }
        if !self.common.is_logical_server() {
            return Err(PlaceFeatureRefusal::Client);
        }
        // `PlaceCommand.checkLoaded` over the feature's chunk and its ring, then the level's own
        // write gate at `pos` (height, a carrier fence).
        let (cx, cz) = (pos[0] >> 4, pos[2] >> 4);
        let ring_loaded = (cx - 1..=cx + 1)
            .all(|x| (cz - 1..=cz + 1).all(|z| self.common.world.chunks.contains_key(&(x, z))));
        if !ring_loaded || !self.native_level_write_allowed(level, pos) {
            return Err(PlaceFeatureRefusal::NotLoaded);
        }
        // The live features the world installed as it opened, as `/place feature` reads them.
        let known = feature.contains(':')
            && self
                .common
                .res
                .behaviours()
                .live_features()
                .is_some_and(|features| {
                    if placed {
                        features.placed(feature).is_some()
                    } else {
                        features.configured(feature).is_some()
                    }
                });
        if !known {
            return Err(PlaceFeatureRefusal::UnknownFeature);
        }
        let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        let answer = self
            .with_level(|b, level, ticks| {
                if placed {
                    b.place_placed_feature(level, ticks, feature, at)
                        .map(|result| result.map_err(|_| PlaceFeatureRefusal::BiomeFilter))
                } else {
                    b.place_feature(level, ticks, feature, at).map(Ok)
                }
            })
            .ok_or(PlaceFeatureRefusal::UnknownFeature)?;
        let _ = self.server_process_level_events();
        answer
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
