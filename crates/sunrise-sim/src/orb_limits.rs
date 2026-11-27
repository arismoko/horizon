//! Limits on experience orb state enforced by vanilla's own types and codecs.
//!
//! Vanilla's `ExperienceOrb.count` uses `ExtraCodecs.POSITIVE_INT` (`1..=i32::MAX`).
//! Live `ExperienceOrb.DATA_VALUE` (`SynchedEntityData<Integer>`) and `ExperienceOrb.age`
//! (Java `int`) hold full `i32` values; only `ExperienceOrb.addAdditionalSaveData`
//! (`entity_nbt::orb_to_nbt`) truncates `Value` and `Age` to `(short)` at serialization.

/// The most orbs one orb can count (`ExperienceOrb.count`, `ExtraCodecs.POSITIVE_INT`: `1..=i32::MAX`).
pub const MAX_COUNT: u32 = i32::MAX as u32;
