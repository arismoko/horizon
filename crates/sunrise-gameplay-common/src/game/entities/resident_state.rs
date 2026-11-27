//! Game-owned residency state; no second membership directory.
use super::*;
use sunrise_gameplay_common::leash_knot::Knot;

pub struct ResidentEntities {
    pub table: Box<RefCell<sunrise_sim::join_seq::residency::table::EntityTable>>,
    pub local_player: sunrise_sim::join_seq::residency::member::Member,
    pub knots: sunrise_sim::join_seq::JoinList<Knot>,
    /// The game's general entity rows (#792), beside the knots this store
    /// already keeps for the resident table's lookups.
    pub general: super::general::GeneralStore,
}

pub trait ResidentStateGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `Player.drop(stack, false)`, as `ItemUtils.createFilledResult` drops what no longer fits
    /// in the inventory: thrown from the eyes, with no thrower, statistic or drop hook.
    fn drop_plain(&mut self, stack: ItemStack) -> Option<u32> {
        self.drop_stack_from_eyes(stack, false)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> ResidentStateGameExt1<'__sg> for G {}

impl Default for ResidentEntities {
    fn default() -> Self {
        Self {
            table: Box::new(RefCell::new(
                sunrise_sim::join_seq::residency::table::EntityTable::new(Default::default()),
            )),
            local_player: Default::default(),
            knots: Default::default(),
            general: Default::default(),
        }
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! bind_resident_list {
    (Mob, $list:expr, $journal:expr, $owner:expr) => {
        $list.bind_residency_with_subtype(
            $journal,
            $owner,
            |entity| (entity.id, entity.join_seq()),
            |entity| {
                if matches!(
                    entity.species(),
                    sunrise_sim::mobs::Species::Villager
                        | sunrise_sim::mobs::Species::WanderingTrader
                ) {
                    sunrise_sim::join_seq::residency::EntitySubtype::Merchant
                } else {
                    sunrise_sim::join_seq::residency::EntitySubtype::Ordinary
                }
            },
        )
    };
    (Other, $list:expr, $journal:expr, $owner:expr) => {
        $list.bind_residency_with_subtype(
            $journal,
            $owner,
            |entity| (entity.id, entity.join_seq()),
            |entity| {
                use sunrise_sim::join_seq::residency::EntitySubtype;
                match entity.kind.as_str() {
                    "minecraft:block_display" => EntitySubtype::BlockDisplay,
                    "minecraft:item_display" => EntitySubtype::ItemDisplay,
                    "minecraft:text_display" => EntitySubtype::TextDisplay,
                    _ => EntitySubtype::Ordinary,
                }
            },
        )
    };
    (Cart, $list:expr, $journal:expr, $owner:expr) => {
        $list.bind_residency_with_subtype(
            $journal,
            $owner,
            |entity| (entity.id, entity.join_seq()),
            |entity| {
                use sunrise_sim::join_seq::residency::EntitySubtype;
                use $crate::game::minecarts::CartKind;
                match entity.kind {
                    CartKind::Chest => EntitySubtype::ChestCart,
                    CartKind::Furnace => EntitySubtype::FurnaceCart,
                    CartKind::Hopper => EntitySubtype::HopperCart,
                    CartKind::Tnt => EntitySubtype::TntCart,
                    CartKind::CommandBlock => EntitySubtype::CommandCart,
                    _ => EntitySubtype::Ordinary,
                }
            },
        )
    };
    ($kind:ident, $list:expr, $journal:expr, $owner:expr) => {
        $list.bind_residency($journal, $owner, |entity| (entity.id, entity.join_seq()))
    };
}
pub use crate::bind_resident_list;
