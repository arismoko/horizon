//! Portable entity base state, NBT encoding, and built-in fire facts.

use sunrise_core::nbt::{Compound, Tag};
use sunrise_sim::entity_nbt::{fire_ticks_from_nbt, flag};

/// Bits for base values stored in the entity body instead of its saved compound.
pub mod owned {
    pub const FIRE: u8 = 1;
    pub const INVULNERABLE: u8 = 1 << 1;
    pub const NO_GRAVITY: u8 = 1 << 2;
    pub const SILENT: u8 = 1 << 3;
    pub const GLOWING: u8 = 1 << 4;
    pub const ALL: u8 = FIRE | INVULNERABLE | NO_GRAVITY | SILENT | GLOWING;
}

/// The base state vanilla gives every entity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntityBase {
    /// `Entity.getRemainingFireTicks`, saved as the short `Fire`.
    pub fire_ticks: i32,
    /// `Entity.isInvulnerable`, saved as `Invulnerable`.
    pub invulnerable: bool,
    /// `Entity.isNoGravity`, saved as `NoGravity` when set.
    pub no_gravity: bool,
    /// `Entity.isSilent`, saved as `Silent` when set.
    pub silent: bool,
    /// `Entity.hasGlowingTag`, saved as `Glowing` when set.
    pub glowing: bool,
}

impl EntityBase {
    /// The values a saved entity compound holds, as `Entity.load` reads them.
    pub fn read(tag: Option<&Tag>) -> Self {
        let Some(tag) = tag else {
            return Self::default();
        };
        Self {
            fire_ticks: fire_ticks_from_nbt(tag),
            invulnerable: flag(tag, "Invulnerable"),
            no_gravity: flag(tag, "NoGravity"),
            silent: flag(tag, "Silent"),
            glowing: flag(tag, "Glowing"),
        }
    }

    /// Which values differ from `other`, as [`owned`] bits.
    pub fn differing(&self, other: &Self) -> u8 {
        [
            (owned::FIRE, self.fire_ticks != other.fire_ticks),
            (owned::INVULNERABLE, self.invulnerable != other.invulnerable),
            (owned::NO_GRAVITY, self.no_gravity != other.no_gravity),
            (owned::SILENT, self.silent != other.silent),
            (owned::GLOWING, self.glowing != other.glowing),
        ]
        .into_iter()
        .filter(|(_, differs)| *differs)
        .fold(0, |bits, (bit, _)| bits | bit)
    }

    /// Writes masked base values with the same tag types and defaults as `Entity.saveWithoutId`.
    pub fn write(&self, map: &mut Compound, mask: u8) {
        if mask & owned::FIRE != 0 {
            map.insert("Fire".into(), Tag::Short(self.fire_ticks as i16));
        }
        if mask & owned::INVULNERABLE != 0 {
            map.insert(
                "Invulnerable".into(),
                Tag::Byte(i8::from(self.invulnerable)),
            );
        }
        for (bit, key, value) in [
            (owned::NO_GRAVITY, "NoGravity", self.no_gravity),
            (owned::SILENT, "Silent", self.silent),
            (owned::GLOWING, "Glowing", self.glowing),
        ] {
            if mask & bit == 0 {
                continue;
            }
            let key = key.into();
            if value {
                map.insert(key, Tag::Byte(1));
            } else {
                map.remove(&key);
            }
        }
    }
}

/// The non-mob built-in kinds with special fire behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinKind {
    Tnt,
    EndCrystal,
    EnderDragon,
    AreaEffectCloud,
    WitherSkull,
    ShulkerBullet,
}

impl BuiltinKind {
    /// The kind a client's tracked copy names by its type id, converted once at the boundary.
    pub fn from_type_id(id: &str) -> Option<Self> {
        Some(match id {
            "minecraft:tnt" => Self::Tnt,
            "minecraft:end_crystal" => Self::EndCrystal,
            "minecraft:ender_dragon" => Self::EnderDragon,
            "minecraft:area_effect_cloud" => Self::AreaEffectCloud,
            "minecraft:wither_skull" => Self::WitherSkull,
            "minecraft:shulker_bullet" => Self::ShulkerBullet,
            _ => return None,
        })
    }

    /// Whether vanilla builds this type with `fireImmune()`.
    pub fn fire_immune(self) -> bool {
        matches!(
            self,
            Self::Tnt | Self::EndCrystal | Self::EnderDragon | Self::AreaEffectCloud
        )
    }

    /// Whether the class reports false from `Entity.isOnFire` regardless of fire ticks.
    pub fn is_on_fire_overridden_false(self) -> bool {
        matches!(self, Self::WitherSkull | Self::ShulkerBullet)
    }
}
