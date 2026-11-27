//! Minecarts (`AbstractMinecart` with `OldMinecartBehavior`): following
//! rails and slopes, powered and activator rails, coming off the track,
//! riding with the rider's push, pushing by players and other carts,
//! `MinecartItem` placement, `VehicleEntity` breaking, the chest, hopper,
//! furnace and TNT variants, saving, and `AbstractMinecartRenderer`'s old
//! rail-following render with the display block.

#[path = "minecarts/save_lending.rs"]
pub(in crate::game) mod save_lending;

#[allow(unused_imports)]
pub use super::{InteractTarget, Keys};
pub use std::sync::Arc;
pub use sunrise_core::{
    geometry::{Aabb, Vec3},
    random::LegacyRandom,
};
#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay_common::minecart_cells::CartCells;
use sunrise_gameplay_common::minecart_seat::*;
use sunrise_items::{
    blocks::Fluid,
    items::{ItemStack, Items},
};
use sunrise_level::{containers::BlockEntity, random_source};
pub use sunrise_sim::mobs::{Species, riders};
pub use sunrise_world::registry::BlockStateId;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CartKind {
    Rideable,
    Chest,
    Hopper,
    Furnace,
    Tnt,
    /// `MinecartCommandBlock`: its `BaseCommandBlock` is `Minecart::command`,
    /// run by a powered activator rail (`game/administration/command_minecarts.rs`).
    CommandBlock,
    /// `MinecartSpawner`: a `BaseSpawner` that ticks where the cart is
    /// (`Minecart::spawner`). Only `/summon` makes one: there is no item.
    Spawner,
}
impl CartKind {
    /// Whether this cart's `interact` passes on the client, so that
    /// `Minecraft.startUseItem` uses the held item instead: `MinecartTNT` has
    /// none of its own (`Entity.interact`), `MinecartCommandBlock.interact`
    /// needs a game master, and `Minecart.interact` a player not sneaking at
    /// an empty cart.
    pub fn interact_passes(self, sneaking: bool, ridden: bool, game_master: bool) -> bool {
        match self {
            Self::Tnt | Self::Spawner => true,
            Self::CommandBlock => !game_master,
            Self::Rideable => sneaking || ridden,
            Self::Chest | Self::Hopper | Self::Furnace => false,
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id.strip_prefix("minecraft:")? {
            "minecart" => Self::Rideable,
            "chest_minecart" => Self::Chest,
            "hopper_minecart" => Self::Hopper,
            "furnace_minecart" => Self::Furnace,
            "tnt_minecart" => Self::Tnt,
            "command_block_minecart" => Self::CommandBlock,
            "spawner_minecart" => Self::Spawner,
            _ => return None,
        })
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Rideable => "minecraft:minecart",
            Self::Chest => "minecraft:chest_minecart",
            Self::Hopper => "minecraft:hopper_minecart",
            Self::Furnace => "minecraft:furnace_minecart",
            Self::Tnt => "minecraft:tnt_minecart",
            Self::CommandBlock => "minecraft:command_block_minecart",
            Self::Spawner => "minecraft:spawner_minecart",
        }
    }
    /// `AbstractMinecart.getDropItem` and `getPickResult`: the item a cart drops and
    /// is picked as (a spawner minecart's is a plain minecart: there is no item of its own).
    pub fn item(self) -> &'static str {
        match self {
            Self::Spawner => "minecraft:minecart",
            kind => kind.id(),
        }
    }
    pub fn slots(self) -> usize {
        match self {
            Self::Chest => 27,
            Self::Hopper => 5,
            _ => 0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Minecart {
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    /// When it joined its list (`sunrise_client::join_seq`); set by the list.
    pub join: sunrise_sim::join_seq::JoinSeq,
    pub id: u32,
    pub kind: CartKind,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub previous_yaw: f32,
    pub pitch: f32,
    pub previous_pitch: f32,
    /// Minecart's client-only accumulated turn and the passenger's eased turn.
    pub rotation_offset: f32,
    pub player_rotation_offset: f32,
    pub passenger_yaw_delta: f32,
    /// Experimental track steps, drained by the server entity tracker.
    pub lerp_steps: Vec<sunrise_protocol::packets::play::MinecartStep>,
    pub flipped: bool,
    /// `AbstractMinecart.onRails`: transitions into rails need a position snap.
    pub on_rails: bool,
    pub ground: sunrise_world::ground_support::GroundState,
    pub fall_distance: f64,
    /// The squared speed `MinecartTNT.causeFallDamage` blows up with, set by
    /// a landing of three blocks or more and taken by the cart's next tick
    /// `Entity.stuckSpeedMultiplier`: set by a cobweb the cart's box is in,
    /// scaled into its next move.
    pub stuck: Vec3,
    pub hurt_time: i32,
    pub hurt_dir: i32,
    pub damage: f32,
    /// `Entity.noGravity` (`NoGravity` in its save).
    pub no_gravity: bool,
    /// `Entity.ticksFrozen` (`TicksFrozen` in its save): powder snow builds
    /// it up (`InsideBlockEffectType.FREEZE`), a cart never melts it.
    pub ticks_frozen: i32,
    pub items: Vec<Option<ItemStack>>,
    /// Furnace minecarts: fuel ticks and `MinecartFurnace.push`. Its y part
    /// stays between ticks (it sets the push's length), only saving drops it.
    pub fuel: i32,
    pub push: Vec3,
    /// TNT minecarts: ticks until the explosion, -1 when not primed.
    pub fuse: i32,
    /// TNT minecarts: `explosionPowerBase` and `explosionSpeedFactor`, the
    /// blast's power at rest and its gain with the cart's speed.
    pub explosion_power: f32,
    pub explosion_speed_factor: f32,
    /// Hopper minecarts: an unpowered activator rail enables them.
    pub enabled: bool,
    /// Chest minecarts: the loot table rolled into `items` on load, saved
    /// again while the items are still that roll.
    pub loot: Option<sunrise_sim::save::PendingContainerLoot>,
    /// `Entity.firstTick`: not ticked yet (saved as `HasTicked`).
    pub first_tick: bool,
    /// `Entity.isInWater` (`wasTouchingWater`): `AbstractMinecart.tick`
    /// updates it after the move (`updateFluidInteraction`), so the move
    /// goes by where the cart was a tick before, and a new cart's first
    /// move is a dry one.
    pub in_water: bool,
    /// Command block minecarts: the cart's `BaseCommandBlock` (command,
    /// success count, last output, output tracking). The block-only flags of
    /// `CommandBlock` stay unset.
    pub command: sunrise_level::command_block::CommandBlock,
    /// `Entity.tickCount`, which `MinecartCommandBlock.activateMinecart`
    /// measures its `ACTIVATION_DELAY` against.
    pub tick_count: i32,
    /// `MinecartCommandBlock.lastActivated`.
    pub last_activated: i32,
    /// An activator rail asked for the command during this tick; it runs once
    /// the cart is back in the level (`command_minecarts.rs`).
    pub command_due: bool,
    /// Spawner minecarts: the cart's `BaseSpawner` (`Delay`, `SpawnData`,
    /// `SpawnPotentials` and the rest as a spawner block saves them).
    pub spawner: Option<Box<sunrise_sim::mobs::spawner::Spawner>>,
    /// The synched `DATA_ID_COMMAND_NAME` and `DATA_ID_LAST_OUTPUT`, which
    /// follow the command block only at `onUpdated`.
    pub synced_command: String,
    pub synced_output: Option<sunrise_core::nbt::Tag>,
    /// `AbstractMinecart.customDisplayBlockState` (`DisplayState`): the block
    /// state's NBT form, resolved against the registry where it draws.
    pub custom_display: Option<sunrise_core::nbt::Tag>,
    /// `AbstractMinecart.customDisplayOffset` (`DisplayOffset`), in sixteenths.
    pub custom_offset: Option<i32>,
    pub random: LegacyRandom,
    /// What `entity_size` answered at its last size refresh (`EntityEvent.Size`): its box takes
    /// the answer while it holds for its vanilla size.
    pub size_answer: Option<sunrise_sim::pose::SizeAnswer>,
}
/// `EntityType.MINECART`'s `sized(0.98F, 0.7F)`: float dimensions, widened.
pub const WIDTH: f64 = 0.98_f32 as f64;
pub const HEIGHT: f64 = 0.7_f32 as f64;
/// `EntityEvent.TNT_PRIME`. `MinecartTNT.primeFuse` broadcasts it;
/// `MinecartTNT.handleEntityEvent` answers by calling `primeFuse`.
pub const TNT_PRIME_EVENT: i8 = 70;
/// `AbstractMinecart.getDefaultDisplayOffset` (6), in sixteenths of a block, as
/// `MinecartChest` (8) and `MinecartHopper` (1) override it.
pub fn default_display_offset(kind: CartKind) -> f64 {
    match kind {
        CartKind::Chest => 8.0,
        CartKind::Hopper => 1.0,
        CartKind::Rideable
        | CartKind::Furnace
        | CartKind::Tnt
        | CartKind::CommandBlock
        | CartKind::Spawner => 6.0,
    }
}
/// `AbstractMinecart.getDisplayOffset`: the saved `DisplayOffset`, else the default.
pub fn display_offset(c: &Minecart) -> f64 {
    c.custom_offset
        .map_or_else(|| default_display_offset(c.kind), f64::from)
}

/// World-space envelope of the cart model and its optional displayed block,
/// based on the same root position and display offset used by the renderer.
pub fn cart_render_bounds(position: Vec3, offset: f64, has_display: bool) -> Aabb {
    let (bottom, top) = if has_display {
        let display_shift = 0.75 * (offset - 8.0) / 16.0;
        (
            (position.y - 0.5).min(position.y + 0.375 - 0.4 + display_shift),
            (position.y + 0.9).max(position.y + 0.375 + 0.4 + display_shift + 0.9),
        )
    } else {
        (position.y - 0.5, position.y + 0.9)
    };
    Aabb::new(
        Vec3::new(position.x - 1.0, bottom, position.z - 1.0),
        Vec3::new(position.x + 1.0, top, position.z + 1.0),
    )
}
/// A display state that is air shows nothing.
pub fn shown_unless_air(
    state: Option<BlockStateId>,
    is_air: impl Fn(BlockStateId) -> bool,
) -> Option<BlockStateId> {
    state.filter(|&s| !is_air(s))
}
/// `readAdditionalSaveData`'s `input.read("DisplayState", CODEC)` is empty
/// when the state does not decode: the cart forgets it (and no longer saves it).
pub fn forget_unreadable_display(
    c: &mut Minecart,
    resolves: impl Fn(&sunrise_core::nbt::Tag) -> bool,
) {
    if c.custom_display.as_ref().is_some_and(|t| !resolves(t)) {
        c.custom_display = None;
    }
}

/// Where the cart `id` is in `carts`: at `hint` (where it was a moment ago,
/// the usual case while carts tick in order) or found by a scan. A world
/// with hundreds of mineshaft chest carts asked this of every cart several
/// times a tick.
pub fn cart_at(carts: &[Minecart], id: u32, hint: usize) -> Option<usize> {
    if carts.get(hint).is_some_and(|c| c.id == id) {
        return Some(hint);
    }
    carts.iter().position(|c| c.id == id)
}
/// `AbstractMinecart.push(entity)` for a non-cart `entity` at `feet`:
/// the push the cart loses (the entity gets a quarter of it back).
pub fn cart_push_from(feet: Vec3, cart: Vec3) -> Option<(f64, f64)> {
    let (xa, za) = (feet.x - cart.x, feet.z - cart.z);
    let dd = xa * xa + za * za;
    if dd < f64::from(1.0e-4f32) {
        return None;
    }
    let d = dd.sqrt();
    let pow = (1.0 / d).min(1.0);
    let k = f64::from(0.1f32) * 0.5;
    Some((xa / d * pow * k, za / d * pow * k))
}
/// `MinecartFurnace.calculateNewPushAlong`: project the push onto movement.
pub fn push_along(push: Vec3, movement: Vec3) -> Vec3 {
    if push.horizontal_distance_squared() > 1.0e-4 && movement.horizontal_distance_squared() > 0.001
    {
        push.projected_on(movement).normalize().scale(push.length())
    } else {
        push
    }
}
/// The minecart's passenger attachment minus Player.DEFAULT_VEHICLE_ATTACHMENT.
pub const SEAT: f64 = PASSENGER_Y - 0.6;
/// The placeholder container key under which an open cart's items live.
pub const CART_KEY_Y: i32 = sunrise_sim::lan::menu::VEHICLE_KEY_Y;

/// `OldMinecartBehavior.pushAndPickupEntities`: living entities the cart
/// offers `startRiding` instead of `Entity.push`. Players click to board;
/// iron golems always push; a cart that already has a rider is `isVehicle`.
pub fn pickup_instead_of_push(species: Species, is_passenger: bool, cart_is_vehicle: bool) -> bool {
    !cart_is_vehicle && !is_passenger && species != Species::IronGolem
}

/// `EntitySelector.pushableBy` asks `isPushable`: `Shulker` takes no push
/// (`Mob::pushable` leaves it out) but still counts, so a cart offers it
/// `startRiding` like any other living entity.
pub fn offered_pickup(pushable: bool, species: Species, alive: bool) -> bool {
    pushable || species == Species::Shulker && alive
}

/// `Shulker.push(Entity)` is empty, so a shulker that is not picked up (it
/// already rides something) neither moves nor slows the cart.
pub fn takes_push_from_cart(species: Species) -> bool {
    species != Species::Shulker
}

/// `Entity.canRide` for a cart: the boarding cooldown, and a warden or the
/// wither never rides (`canRide` false).
pub fn can_ride_cart(species: Species, boarding_cooldown: i32) -> bool {
    boarding_cooldown <= 0 && !matches!(species, Species::Warden | Species::Wither)
}

/// `Entity.positionRider` on an `AbstractMinecart`: the seat offset
/// (`cart_seat_offset`) above the cart.
pub fn cart_seat(cart: Vec3, seat_offset: f64) -> Vec3 {
    Vec3::new(cart.x, cart.y + seat_offset, cart.z)
}

/// A general rider's seat on a cart: the cart's passenger attachment with no vehicle attachment of its own.
pub fn cart_general_seat(cart: Vec3) -> Vec3 {
    cart_seat(cart, PASSENGER_Y)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    NorthSouth,
    EastWest,
    AscendingEast,
    AscendingWest,
    AscendingNorth,
    AscendingSouth,
    SouthEast,
    SouthWest,
    NorthWest,
    NorthEast,
}
impl Shape {
    pub fn parse(s: &str) -> Self {
        match s {
            "east_west" => Self::EastWest,
            "ascending_east" => Self::AscendingEast,
            "ascending_west" => Self::AscendingWest,
            "ascending_north" => Self::AscendingNorth,
            "ascending_south" => Self::AscendingSouth,
            "south_east" => Self::SouthEast,
            "south_west" => Self::SouthWest,
            "north_west" => Self::NorthWest,
            "north_east" => Self::NorthEast,
            _ => Self::NorthSouth,
        }
    }
    pub fn is_slope(self) -> bool {
        matches!(
            self,
            Self::AscendingEast | Self::AscendingWest | Self::AscendingNorth | Self::AscendingSouth
        )
    }
    /// `AbstractMinecart.EXITS`.
    pub fn exits(self) -> ([i32; 3], [i32; 3]) {
        let (w, e, n, s) = ([-1, 0, 0], [1, 0, 0], [0, 0, -1], [0, 0, 1]);
        let below = |v: [i32; 3]| [v[0], -1, v[2]];
        match self {
            Self::NorthSouth => (n, s),
            Self::EastWest => (w, e),
            Self::AscendingEast => (below(w), e),
            Self::AscendingWest => (w, below(e)),
            Self::AscendingNorth => (n, below(s)),
            Self::AscendingSouth => (below(n), s),
            Self::SouthEast => (s, e),
            Self::SouthWest => (s, w),
            Self::NorthWest => (n, w),
            Self::NorthEast => (n, e),
        }
    }
}
pub struct Rail {
    pub name: Arc<str>,
    pub shape: Shape,
    pub powered: bool,
}
impl Minecart {
    /// `AbstractContainerMenu.getRedstoneSignalFromContainer` of `slots`.
    pub fn container_signal(slots: &[Option<ItemStack>], items: &Items) -> i32 {
        let mut total = 0.0f32;
        for s in slots.iter().flatten() {
            let max = items.get(&s.item).map_or(64, |i| i.max_stack).min(64);
            total += s.count as f32 / max as f32;
        }
        total /= slots.len().max(1) as f32;
        // Mth.lerpDiscrete(total, 0, 15)
        (total * 14.0).floor() as i32 + i32::from(total > 0.0)
    }

    /// Snapshot the component codec used for saved output and failure-text admission checks.
    pub fn sync_command_output(&mut self) {
        self.synced_output = self
            .command
            .last_output
            .as_ref()
            .map(|text| sunrise_game_data::scoreboard::component_tag(&text.component_view()));
    }
    /// `Entity.onGround`.
    pub fn on_ground(&self) -> bool {
        self.ground.on_ground
    }
    pub fn new(id: u32, kind: CartKind, position: Vec3) -> Self {
        Self {
            size_answer: None,
            join: Default::default(),
            attachment_lifetime: Default::default(),
            id,
            kind,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            yaw: 0.0,
            previous_yaw: 0.0,
            pitch: 0.0,
            previous_pitch: 0.0,
            rotation_offset: 0.0,
            player_rotation_offset: 0.0,
            passenger_yaw_delta: 0.0,
            lerp_steps: Vec::new(),
            flipped: false,
            on_rails: false,
            ground: Default::default(),
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            hurt_time: 0,
            hurt_dir: 1,
            damage: 0.0,
            no_gravity: false,
            ticks_frozen: 0,
            items: vec![None; kind.slots()],
            fuel: 0,
            push: Vec3::ZERO,
            fuse: -1,
            explosion_power: DEFAULT_EXPLOSION_POWER,
            explosion_speed_factor: DEFAULT_EXPLOSION_SPEED_FACTOR,
            enabled: true,
            loot: None,
            first_tick: true,
            in_water: false,
            command: sunrise_level::command_block::CommandBlock::default(),
            tick_count: 0,
            last_activated: 0,
            command_due: false,
            // A fresh `BaseSpawner`, as `MinecartSpawner`'s constructor makes.
            spawner: (kind == CartKind::Spawner).then(Default::default),
            synced_command: String::new(),
            synced_output: None,
            custom_display: None,
            custom_offset: None,
            random: random_source::entity(),
        }
    }
    /// `EntityType.MINECART & co`'s `getDimensions()`: width, height and eye height.
    pub(in crate::game) const DIMENSIONS: [f32; 3] = [0.98, 0.7, 0.595];
    pub fn bounds(&self) -> Aabb {
        self.bounds_at(self.position)
    }
    /// The box of this cart were its position `p`.
    pub fn bounds_at(&self, p: Vec3) -> Aabb {
        let w = WIDTH / 2.0;
        let vanilla = Aabb::new(
            Vec3::new(p.x - w, p.y, p.z - w),
            Vec3::new(p.x + w, p.y + HEIGHT, p.z + w),
        );
        sunrise_sim::pose::SizeAnswer::sized_box(self.size_answer, Self::DIMENSIONS, p, vanilla)
    }
}
/// `AbstractMinecartContainer.applyNaturalSlowdown`'s keep factor, in the
/// float arithmetic vanilla uses: 0.98 plus a thousandth per empty step of the
/// comparator signal (none while a loot table is still unrolled), times 0.95
/// in water.
pub fn container_keep(signal: i32, loot_pending: bool, in_water: bool) -> f64 {
    let mut keep = 0.98_f32;
    if !loot_pending {
        keep += (15 - signal) as f32 * 0.001_f32;
    }
    if in_water {
        keep *= 0.95_f32;
    }
    f64::from(keep)
}

impl crate::game::CommonGame<'_> {
    /// `positionRider` for the mobs in minecarts: each sits on the cart's
    /// passenger attachment, less its own vehicle attachment.
    pub fn seat_cart_mobs(&mut self, riding: &crate::game::passengers_common::Riders) {
        for i in 0..self.carts.len() {
            let id = self.carts[i].id;
            let Some(js) = riding.get(&id) else {
                continue;
            };
            let (pos, prev) = (self.carts[i].position, self.carts[i].previous);
            let cart_box = self.carts[i].bounds();
            let seats: Vec<(usize, f64, f64)> = js
                .iter()
                .map(|&j| {
                    let m = &self.mobs[j];
                    // Minecart's box is 0.98 wide.
                    let attachment = riders::vehicle_attachment(m, 0.98);
                    (j, attachment, cart_seat_offset(m.species(), attachment))
                })
                .collect();
            for (j, attachment, offset) in seats {
                let m = &mut self.mobs[j];
                m.seated(id, attachment, cart_box);
                sunrise_sim::mobs::living::teleports::set_pos(m, cart_seat(pos, offset));
                m.previous = cart_seat(prev, offset);
                m.velocity = Vec3::ZERO;
                m.fall_distance = 0.0;
                m.ground.on_ground = false;
                m.extra.passenger = true;
                m.ai.nav.stop();
            }
        }
    }
    /// Refuse a removed target before carrying its owning slot into a tick clone.
    pub fn store_cart_back(&mut self, index: usize, cart: Minecart) -> bool {
        !self
            .persist
            .removals
            .contains(&crate::game::commands_common::Target::Cart(cart.id))
            && self.carts.store_back(index, cart)
    }

    /// A chest or hopper minecart or a chest boat is about to be removed
    /// (`Containers.dropContents` / `AbstractChestBoat.remove`): what an open
    /// menu edited goes back to the vehicle, so it is what drops or travels
    /// with it, and the menu's stand-in leaves the level. A menu that is
    /// still open over it then has nothing left to take.
    pub fn release_vehicle_container(&mut self, id: u32) {
        let key = (i32::MAX - 15, CART_KEY_Y, id as i32);
        self.store_cart_items(key);
        self.world.block_entities.remove(&key);
        if self.container == Some(key) {
            self.container = None;
        }
    }
    /// The menu of a chest or hopper minecart or a chest boat works on the
    /// vehicle's own items (`ContainerEntity`): the placeholder under `key`
    /// that the menu edits is copied back to the vehicle.
    pub fn store_cart_items(&mut self, key: (i32, i32, i32)) {
        if key.1 != CART_KEY_Y {
            return;
        }
        let Some(BlockEntity::Chest(items)) = self.world.block_entities.get(&key) else {
            return;
        };
        let items = items.clone();
        if let Some(i) = self.carts.iter().position(|c| c.id as i32 == key.2) {
            self.carts.update(i, |c| c.items = items);
        } else if let Some(i) = self.boats.iter().position(|b| b.id as i32 == key.2) {
            self.boats.update(i, |b| {
                b.items = items;
            });
        }
    }
    /// The open cart's kind when a cart container is shown.
    pub fn open_cart_kind(&self) -> Option<CartKind> {
        let key = self.container.filter(|k| k.1 == CART_KEY_Y)?;
        // A chest boat's chest shows like a chest minecart's.
        self.carts
            .iter()
            .find(|c| c.id as i32 == key.2)
            .map(|c| c.kind)
            .or_else(|| {
                self.boats
                    .iter()
                    .any(|b| b.chest && b.id as i32 == key.2)
                    .then_some(CartKind::Chest)
            })
    }
    /// The seed vanilla picks a block state model's weighted alternatives with outside the
    /// world (`BlockModelResolver.MODEL_SEED`).
    pub const BLOCK_MODEL_SEED: i64 = 42;
}
/// Saving and loading.
impl Minecart {
    pub fn to_nbt(
        &self,
        uuid: [i32; 4],
        raw: Option<&sunrise_core::nbt::Tag>,
        items: &sunrise_items::items::Items,
    ) -> sunrise_core::nbt::Tag {
        use sunrise_core::nbt::{JavaString, Tag};
        let mut tag = sunrise_sim::entity_nbt::vehicle_to_nbt(
            self.kind.id(),
            self.position,
            self.velocity,
            [self.yaw, self.pitch],
            (self.ground.on_ground, self.fall_distance),
            uuid,
            raw,
        );
        sunrise_sim::entity_nbt::write_saved_flags(
            &mut tag,
            self.ground.on_ground,
            self.no_gravity,
        );
        if let Tag::Compound(map) = &mut tag {
            // ContainerEntity.addChestVehicleSaveData belongs only to the
            // container subclasses, never to a plain or specialized cart.
            if self.kind.slots() == 0 {
                map.remove(&JavaString::from("Items"));
            }
            if self.kind == CartKind::CommandBlock {
                for key in ["LastOutput", "LastExecution"] {
                    map.remove(&JavaString::from(key));
                }
            }
            map.remove(&JavaString::from("DisplayOffset"));
            map.remove(&JavaString::from("DisplayState"));
            map.remove(&JavaString::from("explosion_power"));
            map.remove(&JavaString::from("explosion_speed_factor"));
            crate::game::boats_common::put_ticks_frozen(map, self.ticks_frozen);
            let mut put = |k: &str, v: Tag| {
                map.insert(JavaString::from(k), v);
            };
            put("FlippedRotation", Tag::Byte(i8::from(self.flipped)));
            // AbstractMinecart saves firstTick as HasTicked. A cart never burns:
            // Entity.move sets -getFireImmuneTicks(), 0 for all but players.
            put("HasTicked", Tag::Byte(i8::from(self.first_tick)));
            put("Fire", Tag::Short(0));
            // addAdditionalSaveData: the offset only off its default, the state only when set.
            if let Some(offset) = self.custom_offset
                && f64::from(offset) != default_display_offset(self.kind)
            {
                put("DisplayOffset", Tag::Int(offset));
            }
            if let Some(state) = &self.custom_display {
                put("DisplayState", state.clone());
            }
            match self.kind {
                CartKind::Chest | CartKind::Hopper => {
                    put("Items", sunrise_sim::save::encode_items(&self.items, items));
                    if self.kind == CartKind::Hopper {
                        put("Enabled", Tag::Byte(i8::from(self.enabled)));
                    }
                }
                CartKind::Furnace => {
                    put("PushX", Tag::Double(self.push.x.to_bits()));
                    put("PushZ", Tag::Double(self.push.z.to_bits()));
                    put("Fuel", Tag::Short(self.fuel as i16));
                }
                // MinecartTNT.addAdditionalSaveData: the powers only off their defaults.
                CartKind::Tnt => {
                    put("fuse", Tag::Int(self.fuse));
                    for (key, value, default) in [
                        (
                            "explosion_power",
                            self.explosion_power,
                            DEFAULT_EXPLOSION_POWER,
                        ),
                        (
                            "explosion_speed_factor",
                            self.explosion_speed_factor,
                            DEFAULT_EXPLOSION_SPEED_FACTOR,
                        ),
                    ] {
                        if value != default {
                            put(key, Tag::Float(value.to_bits()));
                        }
                    }
                }
                // BaseCommandBlock.save (its CustomName shares the entity's key).
                CartKind::CommandBlock => {
                    for (key, value) in self.command.save() {
                        if !matches!(key, "CustomName" | "powered" | "conditionMet" | "auto") {
                            put(key, value);
                        }
                    }
                }
                // BaseSpawner.save: the delays, ranges, SpawnData and SpawnPotentials.
                CartKind::Spawner => {
                    if let Some(spawner) = &self.spawner {
                        for (key, value) in sunrise_sim::mobs::spawner::SpawnerExt::save(&**spawner)
                        {
                            put(key, value);
                        }
                    }
                }
                CartKind::Rideable => {}
            }
            map.remove(&JavaString::from("LootTable"));
            map.remove(&JavaString::from("LootTableSeed"));
            if let Some(pending) = &self.loot {
                let items = map.get(&"Items".into()).cloned();
                pending.write_tags(map);
                if self.items.iter().any(Option::is_some) {
                    map.extend(items.map(|items| ("Items".into(), items)));
                }
            }
        }
        tag
    }
    pub fn from_nbt(tag: &sunrise_core::nbt::Tag, id: u32, kind: CartKind) -> Option<Self> {
        let (position, _, _) = sunrise_sim::entity_nbt::boat_from_nbt(tag)?;
        let mut c = Self::new(id, kind, position);
        c.load(tag)?;
        Some(c)
    }
    /// `Entity.load` on this cart: the saved fields are read over the live
    /// ones, the rest (riders, lerp, hurt state) stays; its kind never changes.
    /// `None`, with nothing changed, when the compound has no position.
    pub fn load(&mut self, tag: &sunrise_core::nbt::Tag) -> Option<()> {
        use sunrise_core::nbt::Tag;
        let (position, velocity, yaw) = sunrise_sim::entity_nbt::boat_from_nbt(tag)?;
        let kind = self.kind;
        let c = self;
        c.position = position;
        c.previous = position;
        c.velocity = velocity;
        c.yaw = yaw;
        c.previous_yaw = yaw;
        c.no_gravity = sunrise_sim::entity_nbt::flag(tag, "NoGravity");
        c.ground.on_ground = sunrise_sim::entity_nbt::flag(tag, "OnGround");
        c.pitch = sunrise_sim::entity_nbt::rotation(tag)[1];
        c.previous_pitch = c.pitch;
        // ValueInput's numeric getters take any NumericTag: getIntOr is its
        // intValue, getShortOr the low 16 bits of it, getBooleanOr
        // `byteValue() != 0` (the low 8), getDoubleOr its doubleValue.
        let int = |k: &str| tag.get(k).and_then(sunrise_game_data::nbt_path::int_value);
        let flag = |k: &str, default: bool| int(k).map_or(default, |v| v as i8 != 0);
        let double = |k: &str| match tag.get(k) {
            Some(Tag::Byte(v)) => f64::from(*v),
            Some(Tag::Short(v)) => f64::from(*v),
            Some(Tag::Int(v)) => f64::from(*v),
            Some(Tag::Long(v)) => *v as f64,
            Some(Tag::Float(v)) => f64::from(f32::from_bits(*v)),
            Some(Tag::Double(v)) => f64::from_bits(*v),
            _ => 0.0,
        };
        // Entity.load: OnGround and fall_distance, as saved.
        (c.ground.on_ground, c.fall_distance) = sunrise_sim::entity_nbt::ground_from_nbt(tag);
        c.ticks_frozen = crate::game::boats_common::saved_ticks_frozen(tag);
        c.flipped = flag("FlippedRotation", false);
        c.first_tick = flag("HasTicked", false);
        if kind.slots() > 0 {
            c.loot = sunrise_sim::save::PendingContainerLoot::from_tag(tag);
            c.items = if c.loot.is_some() {
                vec![None; kind.slots()]
            } else {
                sunrise_sim::save::decode_items(tag.get("Items"), kind.slots())
            };
        }
        c.enabled = flag("Enabled", true);
        c.fuel = int("Fuel").map_or(0, |v| i32::from(v as i16));
        // readAdditionalSaveData: the saved push has no y.
        c.push = Vec3::new(double("PushX"), 0.0, double("PushZ"));
        c.fuse = int("fuse").unwrap_or(-1);
        if kind == CartKind::Tnt {
            // MinecartTNT.readAdditionalSaveData: getFloatOr, kept to 0..128.
            let float = |k: &str, default: f32| match tag.get(k) {
                Some(Tag::Float(v)) => f32::from_bits(*v),
                Some(Tag::Double(v)) => f64::from_bits(*v) as f32,
                Some(Tag::Long(v)) => *v as f32,
                Some(other) => {
                    sunrise_game_data::nbt_path::int_value(other).map_or(default, |v| v as f32)
                }
                None => default,
            };
            c.explosion_power = float("explosion_power", DEFAULT_EXPLOSION_POWER).clamp(0.0, 128.0);
            c.explosion_speed_factor =
                float("explosion_speed_factor", DEFAULT_EXPLOSION_SPEED_FACTOR).clamp(0.0, 128.0);
        }
        // readAdditionalSaveData: DisplayState, DisplayOffset (default offset when absent).
        c.custom_display = tag.get("DisplayState").cloned();
        c.custom_offset = int("DisplayOffset");
        if kind == CartKind::CommandBlock {
            c.command = sunrise_level::command_block::CommandBlock::load(tag);
            // MinecartCommandBlock.readAdditionalSaveData sets the synched command.
            // A changed value invokes onSyncedDataUpdated -> setCommand, resetting successes.
            if c.synced_command != c.command.command {
                c.synced_command.clone_from(&c.command.command);
                c.command.set_command(&c.synced_command);
            }
            c.sync_command_output();
        }
        // MinecartSpawner.readAdditionalSaveData: BaseSpawner.load.
        if kind == CartKind::Spawner {
            c.spawner = Some(Box::new(sunrise_sim::mobs::spawner::SpawnerExt::load(tag)));
        }
        Some(())
    }
}

/// `MinecartTNT` default explosion powers.
pub(in crate::game) const DEFAULT_EXPLOSION_POWER: f32 = 4.0;
pub(in crate::game) const DEFAULT_EXPLOSION_SPEED_FACTOR: f32 = 1.0;

/// `VehicleEntity.hurtServer`'s destroy test: a creative player, damage past
/// 40, or (`MinecartTNT.shouldSourceDestroy`) fire or an explosion hitting a
/// TNT cart.
pub fn cart_is_destroyed(creative: bool, damage: f32, ignites_tnt: bool) -> bool {
    creative || damage > 40.0 || ignites_tnt
}

/// `MinecartTNT.tick`'s collision detonation: a cart still moving at a tenth
/// of a block a tick or more when a block stops it blows up.
pub fn tnt_ignites_by_collision(horizontal_collision: bool, speed_sqr: f64) -> bool {
    horizontal_collision && speed_sqr >= 0.009_999_999_776_482_582
}

/// NewMinecartBehavior's `(float) (Math.atan2(y, x) * 180.0 / Math.PI)` before
/// the cast: HotSpot's atan2, multiplied then divided in Java's order.
pub fn java_degrees(y: f64, x: f64) -> f64 {
    sunrise_core::jmath::atan2(y, x) * 180.0 / std::f64::consts::PI
}

/// `MinecartFurnace.addFuel`: Java int arithmetic, so a fuel a mod set near
/// `i32::MAX` wraps negative and takes more.
pub fn furnace_fuel_after(fuel: i32) -> Option<i32> {
    let next = fuel.wrapping_add(3600);
    (next <= 32000).then_some(next)
}

/// `MinecartFurnace.getMaxSpeed`: half the behavior's speed, three quarters
/// in water.
pub fn furnace_speed(c: &Minecart, base: f64) -> f64 {
    if c.kind == CartKind::Furnace {
        base * if c.in_water { 0.75 } else { 0.5 }
    } else {
        base
    }
}
sunrise_sim::impl_joined!(Minecart);
sunrise_mod_runtime::impl_attachment_lifetime!(Minecart);

#[cfg(test)]
pub(in crate::game) mod tnt_cart_power_tests {
    use super::*;

    #[test]
    fn tnt_cart_power_round_trips_live_state_and_clears_stale_defaults() {
        use sunrise_core::nbt::{Tag, parse_snbt};
        let raw = parse_snbt("{Pos:[1d,200d,1d],explosion_power:200f,explosion_speed_factor:-2d}")
            .unwrap()
            .0;
        let mut cart = Minecart::from_nbt(&raw, 1, CartKind::Tnt).unwrap();
        assert_eq!(cart.explosion_power, 128.0);
        assert_eq!(cart.explosion_speed_factor, 0.0);
        let saved = cart.to_nbt([0; 4], Some(&raw), &Default::default());
        assert_eq!(
            saved.get("explosion_power"),
            Some(&Tag::Float(128.0f32.to_bits()))
        );
        assert_eq!(
            saved.get("explosion_speed_factor"),
            Some(&Tag::Float(0.0f32.to_bits()))
        );
        cart.explosion_power = DEFAULT_EXPLOSION_POWER;
        cart.explosion_speed_factor = DEFAULT_EXPLOSION_SPEED_FACTOR;
        let saved = cart.to_nbt([0; 4], Some(&raw), &Default::default());
        assert!(saved.get("explosion_power").is_none());
        assert!(saved.get("explosion_speed_factor").is_none());
        let restored = Minecart::from_nbt(&saved, 2, CartKind::Tnt).unwrap();
        assert_eq!(restored.explosion_power, DEFAULT_EXPLOSION_POWER);
        assert_eq!(
            restored.explosion_speed_factor,
            DEFAULT_EXPLOSION_SPEED_FACTOR
        );
    }
}

#[cfg(test)]
#[path = "minecarts/minecart_riders_tests.rs"]
pub(in crate::game) mod minecart_riders_tests;

impl sunrise_sim::entity_rules::EntityRules for Minecart {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `AbstractMinecart.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        if self.in_water { 0.005 } else { 0.04 }
    }
}

use crate::game::tables::WorldTablesExt as _;

use crate::game::cart_rails::CartRailsGameExt1 as _;

pub trait MinecartRailGameExt<'g>: crate::game::SideGame<'g> {
    /// `getCurrentBlockPosOrRailBelow`.
    fn rail_block(&self, p: Vec3, improved: bool) -> [i32; 3] {
        let (x, y, z) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        let below = if improved {
            (p.y - 0.1 - f64::from(1.0e-5_f32)).floor() as i32
        } else {
            y - 1
        };
        if self.tagged_rail(self.world.block(x, below, z)) {
            [x, below, z]
        } else {
            [x, y, z]
        }
    }

    /// `OldMinecartBehavior.getPos`: the point on the rail under (x, y, z), on the track the
    /// rail lays for `cart`.
    fn rail_pos(&self, cart: Option<u32>, x: f64, y: f64, z: f64) -> Option<[f64; 3]> {
        let [xt, yt, zt] = self.rail_block(Vec3::new(x, y, z), self.minecart_improvements());
        let rail = self.rail(cart, xt, yt, zt)?;
        let (e0, e1) = rail.shape.exits();
        let (fx, fy, fz) = (f64::from(xt), f64::from(yt), f64::from(zt));
        let x0 = fx + 0.5 + f64::from(e0[0]) * 0.5;
        let y0 = fy + 0.0625 + f64::from(e0[1]) * 0.5;
        let z0 = fz + 0.5 + f64::from(e0[2]) * 0.5;
        let x1 = fx + 0.5 + f64::from(e1[0]) * 0.5;
        let y1 = fy + 0.0625 + f64::from(e1[1]) * 0.5;
        let z1 = fz + 0.5 + f64::from(e1[2]) * 0.5;
        let (xd, yd, zd) = (x1 - x0, (y1 - y0) * 2.0, z1 - z0);
        let progress = if xd == 0.0 {
            z - fz
        } else if zd == 0.0 {
            x - fx
        } else {
            ((x - x0) * xd + (z - z0) * zd) * 2.0
        };
        let mut out = [x0 + xd * progress, y0 + yd * progress, z0 + zd * progress];
        if yd < 0.0 {
            out[1] += 1.0;
        } else if yd > 0.0 {
            out[1] += 0.5;
        }
        Some(out)
    }

    /// `AbstractMinecart`: `minecart_improvements` picks the smoother
    /// physics.
    fn minecart_improvements(&self) -> bool {
        self.enabled_feature_ids()
            .iter()
            .any(|f| f == "minecraft:minecart_improvements")
    }
}
impl<'g, G: crate::game::SideGame<'g>> MinecartRailGameExt<'g> for G {}

pub trait CartDismountGameExt<'g>: crate::game::SideGame<'g> {
    /// Where the cart at `i` lets a player off.
    fn cart_player_dismount_target(&self, i: usize) -> Vec3 {
        let c = &self.carts[i];
        let (bx, by, bz) = (
            c.position.x.floor() as i32,
            c.position.y.floor() as i32,
            c.position.z.floor() as i32,
        );
        let mut target = None;
        let face = cart_motion_face(c.yaw, c.flipped, self.minecart_improvements());
        'search: for dy in [0, 1, -1] {
            for (dx, dz) in dismount_offsets(face) {
                let (x, y, z) = (bx + dx, by + dy, bz + dz);
                let below = self.res.facts().get(self.world.block(x, y - 1, z));
                let here = self.res.facts().get(self.world.block(x, y, z));
                let head = self.res.facts().get(self.world.block(x, y + 1, z));
                if (below.full_collision || below.legacy_solid)
                    && !here.full_collision
                    && !head.full_collision
                    && here.fluid == Fluid::None
                {
                    target = Some(Vec3::new(
                        f64::from(x) + 0.5,
                        f64::from(y),
                        f64::from(z) + 0.5,
                    ));
                    break 'search;
                }
            }
        }
        let top = c.bounds().max_position().y;
        target.unwrap_or(Vec3::new(c.position.x, top, c.position.z))
    }
}
impl<'g, G: crate::game::SideGame<'g>> CartDismountGameExt<'g> for G {}
