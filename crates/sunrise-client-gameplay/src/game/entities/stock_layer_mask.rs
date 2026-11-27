//! The renderer's fixed-size set of stock layers: one bit per `StockLayer`.
use mod_api::{Hand, StockLayer};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::game) struct StockLayerMask(u32);

const fn bit(layer: StockLayer) -> u32 {
    1 << match layer {
        StockLayer::Armor(slot, pass) => 3 * slot as u32 + pass as u32,
        StockLayer::Wings => 12,
        StockLayer::Cape => 13,
        StockLayer::CustomHead => 14,
        StockLayer::HeldItem(Hand::Main) => 15,
        StockLayer::HeldItem(Hand::Off) => 16,
        StockLayer::SnowGolemHead => 17,
        StockLayer::ParrotOnShoulder => 18,
        StockLayer::Arrow => 19,
        StockLayer::BeeStinger => 20,
        StockLayer::SpinAttackEffect => 21,
        StockLayer::Deadmau5Ears => 22,
    }
}

impl StockLayerMask {
    /// The empty mask: no stock layers selected.
    pub(in crate::game) const EMPTY: Self = Self(0);

    /// Every `StockLayer::Armor(slot, pass)` bit set.
    pub(in crate::game) const ALL_ARMOR: Self = Self((1 << 12) - 1);

    /// Whether no bits are set.
    pub(in crate::game) const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Inserts `layer` into this mask.
    pub(in crate::game) fn insert(&mut self, layer: StockLayer) {
        self.0 |= bit(layer);
    }

    /// Returns a copy of this mask with `layer` set.
    pub(in crate::game) const fn with(self, layer: StockLayer) -> Self {
        Self(self.0 | bit(layer))
    }

    /// Returns a copy of this mask with `layer` cleared.
    pub(in crate::game) const fn without(self, layer: StockLayer) -> Self {
        Self(self.0 & !bit(layer))
    }

    /// Returns a mask containing only `layer`.
    pub(in crate::game) const fn only(layer: StockLayer) -> Self {
        Self(bit(layer))
    }

    /// Returns the union of two masks.
    pub(in crate::game) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether `layer` is set in this mask.
    pub(in crate::game) const fn contains(self, layer: StockLayer) -> bool {
        self.0 & bit(layer) != 0
    }
}
