//! Role-neutral workstation state (`Kind`, `Station`) and common `Game` station accessors.
use sunrise_items::items::ItemStack;
use sunrise_sim::stations::{AnvilResult, EnchantOffers};

/// Slot ids of the open station: inputs from 600, the result at 650.
pub const INPUT: usize = 600;
pub const RESULT: usize = 650;

/// `AnvilScreen`: `name.setMaxLength(50)`.
pub const NAME_CHARS: usize = 50;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Enchanting,
    Anvil,
    Grindstone,
    Smithing,
    Stonecutter,
    /// Villager trading (game/items/trading.rs).
    Merchant,
    /// The beacon menu (game/blocks/beacon.rs).
    Beacon,
    /// The cartography table (game/blocks/cartography.rs).
    Cartography,
    /// The loom (game/blocks/loom.rs).
    Loom,
}

impl Kind {
    pub fn from_block(name: &str) -> Option<Self> {
        Some(match name {
            "minecraft:enchanting_table" => Self::Enchanting,
            "minecraft:anvil" | "minecraft:chipped_anvil" | "minecraft:damaged_anvil" => {
                Self::Anvil
            }
            "minecraft:grindstone" => Self::Grindstone,
            "minecraft:smithing_table" => Self::Smithing,
            "minecraft:stonecutter" => Self::Stonecutter,
            "minecraft:beacon" => Self::Beacon,
            "minecraft:cartography_table" => Self::Cartography,
            "minecraft:loom" => Self::Loom,
            _ => return None,
        })
    }

    pub(in crate::game) fn inputs(self) -> usize {
        match self {
            Self::Smithing | Self::Loom => 3,
            Self::Stonecutter | Self::Beacon => 1,
            _ => 2,
        }
    }

    pub fn title_key(self) -> &'static str {
        match self {
            Self::Enchanting => "container.enchant",
            Self::Anvil => "container.repair",
            Self::Grindstone => "container.grindstone_title",
            Self::Smithing => "container.upgrade",
            Self::Stonecutter => "container.stonecutter",
            Self::Merchant => "merchant.trades",
            Self::Beacon => "container.beacon",
            Self::Cartography => "container.cartography_table",
            Self::Loom => "container.loom",
        }
    }

    /// Whether the menu has a result slot after its inputs.
    pub(in crate::game) fn has_result(self) -> bool {
        !matches!(self, Self::Enchanting | Self::Beacon)
    }
}

impl Kind {
    pub fn texture(self) -> &'static str {
        match self {
            Self::Enchanting => "gui/container/enchanting_table",
            Self::Anvil => "gui/container/anvil",
            Self::Grindstone => "gui/container/grindstone",
            Self::Smithing => "gui/container/smithing",
            Self::Stonecutter => "gui/container/stonecutter",
            Self::Merchant => "gui/container/villager",
            Self::Beacon => "gui/container/beacon",
            Self::Cartography => "gui/container/cartography_table",
            Self::Loom => "gui/container/loom",
        }
    }
    /// (titleLabelX, titleLabelY).
    pub fn title_pos(self) -> (f32, f32) {
        match self {
            Self::Anvil => (60.0, 6.0),
            Self::Smithing => (44.0, 15.0),
            Self::Stonecutter => (8.0, 5.0),
            Self::Cartography | Self::Loom => (8.0, 4.0),
            _ => (8.0, 6.0),
        }
    }
    pub fn input_pos(self, i: usize) -> (f32, f32) {
        let table: &[(f32, f32)] = match self {
            Self::Enchanting => &[(15.0, 47.0), (35.0, 47.0)],
            Self::Anvil => &[(27.0, 47.0), (76.0, 47.0)],
            Self::Grindstone => &[(49.0, 19.0), (49.0, 40.0)],
            Self::Smithing => &[(8.0, 48.0), (26.0, 48.0), (44.0, 48.0)],
            Self::Stonecutter => &[(20.0, 33.0)],
            Self::Merchant => &[(86.0, 37.0), (112.0, 37.0)],
            Self::Beacon => &[(109.0, 110.0)],
            Self::Cartography => &[(15.0, 15.0), (15.0, 52.0)],
            Self::Loom => &[(13.0, 26.0), (33.0, 26.0), (23.0, 45.0)],
        };
        table[i.min(table.len() - 1)]
    }
    pub fn result_pos(self) -> Option<(f32, f32)> {
        match self {
            Self::Enchanting => None,
            Self::Anvil => Some((134.0, 47.0)),
            Self::Grindstone => Some((129.0, 34.0)),
            Self::Smithing => Some((98.0, 48.0)),
            Self::Stonecutter => Some((143.0, 33.0)),
            Self::Merchant => Some((170.0, 37.0)),
            Self::Beacon => None,
            Self::Cartography => Some((145.0, 39.0)),
            Self::Loom => Some((143.0, 57.0)),
        }
    }
}

pub struct Station {
    pub kind: Kind,
    pub pos: [i32; 3],
    pub inputs: Vec<Option<ItemStack>>,
    pub result: Option<ItemStack>,
    pub offers: EnchantOffers,
    pub bookcases: i32,
    pub anvil: AnvilResult,
    /// Anvil rename box text (`AnvilMenu.itemName`).
    pub name: String,
    /// Stonecutter: the menu's recipe list (sunrise_client::menu) and `selectedRecipeIndex`.
    pub stone: sunrise_sim::menu::Stonecutter,
    pub selected: Option<usize>,
    #[allow(dead_code)]
    pub(in crate::game) last_sound: u64,
    /// Villager trading state.
    pub merchant: Option<Box<crate::game::trading_common::MerchantState>>,
    /// Beacon menu: the selected primary and secondary power (EFFECTS index).
    pub beacon: [Option<usize>; 2],
}

impl Station {
    /// `StonecutterScreen.isScrollBarActive`.
    pub fn stonecutter_scrolls(&self) -> bool {
        self.inputs[0].is_some() && self.stone.recipes.len() > 12
    }

    pub fn new(kind: Kind, pos: [i32; 3]) -> Self {
        Self {
            kind,
            pos,
            inputs: vec![None; kind.inputs()],
            result: None,
            offers: EnchantOffers::default(),
            bookcases: 0,
            anvil: AnvilResult::default(),
            name: String::new(),
            stone: Default::default(),
            selected: None,
            last_sound: 0,
            merchant: None,
            beacon: [None; 2],
        }
    }

    pub fn new_kind(kind: Kind, pos: [i32; 3]) -> Self {
        Self::new(kind, pos)
    }
}

impl crate::game::CommonGame<'_> {
    /// The anvil name the menu works with (`AnvilScreen.onNameChanged`).
    pub fn anvil_item_name(&self, s: &Station) -> String {
        match &s.inputs[0] {
            Some(input)
                if input.custom_name().is_none()
                    && s.name
                        == sunrise_game_data::lang::returned_to_server(|| {
                            self.stack_hover_name_text(input).plain()
                        }) =>
            {
                String::new()
            }
            Some(_) => s.name.clone(),
            None => String::new(),
        }
    }

    pub fn max_stack(&self, stack: &ItemStack) -> u32 {
        self.res
            .items()
            .get(&stack.item)
            .map_or(64, |i| i.max_stack)
    }
}

pub trait StationsCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `slotsChanged` / `createResult` for the open station.
    fn station_changed(&mut self) {
        self.side_station_changed();
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> StationsCommonGameExt1<'__sg> for G {}

use crate::game::side_ops::blocks::BlocksSideGameExt as _;
