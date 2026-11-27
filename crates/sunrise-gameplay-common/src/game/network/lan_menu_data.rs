use crate::game::{
    Dim, native_menu_data::HeardMenu, stations_common::Kind as StationKind,
    trading_common::MerchantState,
};
use std::collections::{BTreeSet, HashMap};
use sunrise_core::random::LegacyRandom;
use sunrise_gameplay_common::stats::Stats;
use sunrise_items::items::ItemStack;
use sunrise_sim::{
    advancements::{Advancement, Progress},
    lan::guest::Guest,
    menu::{Menu, QuickCraftDrag},
    merchant::{self, Trader},
    mobs::villager::{Ext, ProfessionState},
    recipe_book::Settings,
};

/// The non-storage effects from one guest menu action.
#[derive(Default)]
pub struct GuestMenuEffects {
    pub dropped: Vec<ItemStack>,
    pub equipped: Vec<(&'static str, Option<ItemStack>)>,
    pub brewed: Vec<ItemStack>,
    pub brewed_potion_takes: Vec<ItemStack>,
    pub crafted: Vec<(String, Vec<Option<ItemStack>>)>,
    pub item_crafted: Vec<(String, ItemStack, Vec<Option<ItemStack>>)>,
    pub crafted_counts: Vec<(std::sync::Arc<str>, i32)>,
}

pub struct FurnaceMenuEffects {
    pub uuid: u128,
    pub pos: (i32, i32, i32),
    pub at: [f64; 3],
    pub award_recipes: bool,
    pub smelted: Vec<(ItemStack, u32)>,
}

impl GuestMenuEffects {
    pub fn dropped(dropped: Vec<ItemStack>) -> Self {
        Self {
            dropped,
            ..Self::default()
        }
    }

    pub fn take(menu: &mut Menu) -> Self {
        let dropped = std::mem::take(&mut menu.dropped);
        menu.drops.clear();
        Self {
            dropped,
            equipped: std::mem::take(&mut menu.equipped),
            brewed: std::mem::take(&mut menu.brewed),
            brewed_potion_takes: std::mem::take(&mut menu.brewed_potion_takes),
            crafted: std::mem::take(&mut menu.crafted),
            item_crafted: std::mem::take(&mut menu.item_crafted),
            crafted_counts: std::mem::take(&mut menu.crafted_counts),
        }
    }
}

/// An authoritative click reply, published only once the click's transaction has committed.
pub struct MenuClickReply {
    pub conn: u32,
    pub generation: u64,
    pub reply: ClickReply,
}

impl MenuClickReply {
    pub fn closes_menu(&self) -> bool {
        matches!(
            self.reply,
            ClickReply::Close(_) | ClickReply::ClientClose(_)
        )
    }
}

pub enum ClickReply {
    Predicted(sunrise_protocol::packets::play::ContainerClick, Vec<usize>),
    Resend(i32),
    Close(i32),
    ClientClose(i32),
    Inventory,
    Offhand,
}

/// Identity of a mount window across a tentative click and its settlement.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct MountClickWindow {
    pub dim: Dim,
    pub conn: u32,
    pub generation: u64,
    pub window: i32,
    pub mob: u32,
}

/// A guest's open workstation menu.
#[derive(Clone)]
pub struct GuestStation {
    pub window: i32,
    pub state: i32,
    pub kind: StationKind,
    pub pos: [i32; 3],
    pub menu: Box<Menu>,
    /// The data slots the client was last sent. `None` is not sent yet,
    /// which is not a cost: an anvil's signed cost may be `i32::MIN`.
    pub sent_data: Vec<Option<i32>>,
    /// The game time of the menu's last sound (`lastSoundTime`).
    pub last_sound: u64,
}

/// A guest's open trading screen.
#[derive(Clone)]
pub struct GuestMerchant {
    pub window: i32,
    pub state: i32,
    /// The payment slots and the result.
    pub slots: [Option<ItemStack>; 3],
    /// Persistent quick-craft (drag) state across rebuilt merchant menus.
    pub drag: QuickCraftDrag,
    pub merchant: Box<MerchantState>,
    /// The offers the client was last sent.
    pub sent: Vec<merchant::MerchantOffer>,
    pub sent_level: i32,
    pub sent_xp: i32,
    /// The result slot last synchronized to the client (`AbstractContainerMenu.remoteSlots`).
    pub sent_result: Option<ItemStack>,
}

pub struct KeptMerchantGuest {
    pub merchant: GuestMerchant,
    pub stats: Option<Stats>,
    pub progress: Progress,
    pub changed: BTreeSet<usize>,
    pub unlocked: Vec<String>,
    pub book: Settings,
}

pub struct KeptMerchantWindow {
    pub window: Box<dyn FnOnce(&mut Guest)>,
    pub heard: Option<HeardMenu>,
    pub owned: Option<(u64, Box<Menu>)>,
}

pub struct KeptMerchantMob {
    pub random: LegacyRandom,
    pub timer: i32,
    pub trade: Option<Box<Trader>>,
    pub trading: bool,
    pub profession: ProfessionState,
    pub level: i32,
    pub variant: u8,
    pub villager: Option<Box<Ext>>,
}

#[derive(Default)]
pub struct Journal {
    pub guests: HashMap<u32, Box<KeptMerchantGuest>>,
    pub windows: HashMap<u32, KeptMerchantWindow>,
    pub mobs: HashMap<u32, Box<KeptMerchantMob>>,
    pub sequences: Option<sunrise_items::random_sequences::RandomSequences>,
    pub active: Option<u32>,
    pub correct_on_abort: std::collections::BTreeSet<(u32, i32)>,
    pub refused_clicks: std::collections::BTreeSet<(u32, i32)>,
}

pub enum Effect {
    Orb([f64; 3], u32),
    Drops(u32, Vec<sunrise_items::items::ItemStack>),
    Reward(u128, Advancement),
}

impl Journal {
    pub fn merge(&mut self, child: Self) {
        for (conn, kept) in child.guests {
            self.guests.entry(conn).or_insert(kept);
        }
        for (conn, kept) in child.windows {
            self.windows.entry(conn).or_insert(kept);
        }
        for (id, kept) in child.mobs {
            self.mobs.entry(id).or_insert(kept);
        }
        if self.sequences.is_none() {
            self.sequences = child.sequences;
        }
        self.correct_on_abort.extend(child.correct_on_abort);
    }
}
