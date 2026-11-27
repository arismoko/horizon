//! Common `OminousItemSpawner` insertion shared by client packets and server tests.
use sunrise_items::items::ItemStack;

/// `OminousItemSpawnerRenderer`: grows over its first 50 ticks, turning 40
/// degrees a tick.
pub const TICKS_SCALING: f32 = 50.0;
pub const ROTATION_SPEED: f32 = 40.0;

pub struct OminousItemSpawner {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub join: sunrise_sim::join_seq::JoinSeq,
    /// Its entity id: from the entity counter on the server, the server's on
    /// a client.
    pub id: u32,
    /// `DATA_ITEM`; `minecraft:air` stands for `ItemStack.EMPTY`.
    pub item: ItemStack,
    pub position: [f64; 3],
    pub spawn_item_after: i64,
    pub tick_count: i64,
    /// `Entity.remainingFireTicks`, counted down by `baseTick`.
    pub fire_ticks: i32,
    /// `Entity.DATA_SHARED_FLAGS_ID` on-fire bit, set after `baseTick` fire processing.
    pub synced_fire: bool,
    /// `Entity.hasVisualFire` (`HasVisualFire` in NBT).
    pub has_visual_fire: bool,
    /// Its UUID, and the compound it was loaded from (fields Sunrise doesn't
    /// model survive a save).
    pub uuid: [i32; 4],
    pub raw: Option<Box<sunrise_core::nbt::Tag>>,
}

sunrise_sim::impl_joined!(OminousItemSpawner);
sunrise_mod_runtime::impl_attachment_lifetime!(OminousItemSpawner);

impl crate::game::CommonGame<'_> {
    /// `handleAddEntity` for a spawner a server sent: its item comes with the
    /// entity data (`ItemStack.EMPTY` until then, which draws nothing).
    pub fn add_remote_ominous_item_spawner(&mut self, id: u32, position: [f64; 3]) {
        self.ominous_spawners.push(OminousItemSpawner {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            item: ItemStack::new("minecraft:air", 1),
            position,
            spawn_item_after: 0,
            tick_count: 0,
            fire_ticks: 0,
            synced_fire: false,
            has_visual_fire: false,
            uuid: [0; 4],
            raw: None,
        });
    }
}
