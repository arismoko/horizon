//! Trait mods implement `Hooks` and link into the game executable.
//! The executable passes the ordered `Mods` to `entry::main_with`.
//! Trait mods are process-wide, like mixin module statics.
//! Hooks use `&self + Send + Sync` for shared and reentrant calls; see `docs/native-mods.md`.
//! Generate the trait and dispatch with `cargo run -p xtask -- mod-api`.
use std::sync::{Arc, OnceLock};

pub use crate::biome::{BiomeColors, SpawnEntry};
pub use crate::block_sprites::{BlockSprites, FaceSprite, SpriteRef};
pub use crate::camera::{CameraEntity, FovPlayer};
pub use crate::command::Suggestion;
pub use crate::containers::{ModBlockEntity, ModContainer, ModEnergyBuffer, ModFluidTanks};
pub use crate::crafting::{CustomRecipe, Ingredient};
pub use crate::extras::Corner;
pub use crate::extras_hud::Corners;
pub use crate::first_person::Hand;
pub use crate::fog::{FogData, FogType};
pub use crate::game::native_block_entity_draw::{BlockEntityDraw, CubeFace};
pub use crate::game::{
    NativeBlockEdit, NativeBlockRun, NativeClock, NativeColumnBlocks, NativeDataMapType,
    NativeExperienceOrb, NativeFalling, NativeHeightmap, NativeHeldItem, NativeHit,
    NativeMapDecoration, NativeMapInfo, NativeMobRender, NativeModEvent, NativePlayerRender,
    NativeShake, PhantomSpawnResult, SleepProblem,
};
pub use crate::gamerules::Value as GameRuleValue;
pub use crate::gfx::native_render::{
    BlockSpriteArrays, RenderHooks, RenderNeeds, RenderNeedsEvent, WorldDrawn, WorldDrawnEvent,
    wgpu,
};
pub use crate::health::{HurtSource, ShieldBlock};
pub use crate::hud::HudBatch;
pub use crate::hud_layout::{GuiDraw, HealthInputs};
pub use crate::items::ItemStack;
pub use crate::menu::{LogicalSide, StackedOnOther};
pub use crate::merchant::MerchantOffer;
pub use crate::mesher::TintSource;
pub use crate::music::{Music, MusicLayer};
pub use crate::particles::{ParticleOptions, ParticleType};
pub use crate::recipe_book::RecipeBookCategory;
pub use crate::text::Text;
pub use horizon_assets::item_model::{DisplayContext, StackView};
pub use horizon_core::{
    geometry::Direction,
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
pub use horizon_world::consumable::UseAnimation;
pub use horizon_world::player_inventory::EquipmentSlot;
pub use horizon_world::{
    attachments::{
        AttachmentCodec, AttachmentError, AttachmentHost, AttachmentType, Attachments, Holder,
    },
    fluid::FluidState,
    level::{BlockId, Dimension as LevelDimension, PlacementContext},
    loot::{LootStack, NativeLootContext},
    nbt::{Compound, Tag},
    registry::BlockStateId,
};
pub use serde_json;

use serde_json::{Map, Value};
use std::time::Duration;
/// The window events `input_key` hands over (`winit::event::KeyEvent`) and their key types.
pub use winit;

/// One armor slot of an `ArmorHurtEvent`: the piece in `slot`, the durability
/// damage vanilla works out for it (`damage`, `max(1, hit / 4)`) and the damage it
/// takes (`new_damage`, starts equal; a mod may change it, below 1 for none; the
/// game casts it to a whole number as vanilla does).
#[derive(Clone, Debug)]
pub struct NativeArmorHurt {
    pub slot: EquipmentSlot,
    pub stack: ItemStack,
    pub damage: f32,
    pub new_damage: f32,
}

/// Camera values from the most recently extracted world frame.
#[derive(Clone, Debug)]
pub struct NativeCamera {
    pub position: [f64; 3],
    pub eye_height: f32,
    pub partial_tick: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_degrees: f32,
    pub near: f32,
    pub far: f32,
    /// Camera-relative view matrix in column-major order.
    pub view: [f32; 16],
    /// Reversed-Z projection matrix in column-major order.
    pub projection: [f32; 16],
    pub dimension: String,
}

/// Context and adjustable pose for a first-person hand render.
///
/// Translation is in blocks and rotation is in degrees (pitch, yaw, roll).
/// The renderer applies the transform before the ordinary hand and item poses.
pub struct NativeHandRender<'a> {
    pub player: u128,
    pub player_entity_id: i32,
    pub hand: Hand,
    pub stack: Option<&'a ItemStack>,
    pub partial: f32,
    pub interpolated_pitch: f32,
    pub swing: f32,
    pub equip: f32,
    /// Vanilla packed light at the eyes (`LightCoordsUtil.pack`: block << 4 | sky << 20).
    pub packed_light: u32,
    pub translation: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: f32,
}

/// A dispenser hook's answer: the slot's remainder (None empties it) and the
/// level event to play, `CLICK` (1000), `FAIL` (1001) or 0 for none.
pub type NativeDispensed = (Option<ItemStack>, i32);

/// Context for a player tuning a note block.
#[derive(Clone, Copy, Debug)]
pub struct NativeNoteBlockChange {
    pub dimension: LevelDimension,
    pub pos: BlockPos,
    /// The block's instrument before tuning.
    pub instrument: NativeNoteInstrument,
    /// The note before tuning, in the range 0 through 24.
    pub old_note: i32,
}

/// What dropped the items heard by the `loot_drops` hook.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct NativeLootDrops<'a> {
    /// The block's or mob's own loot table (`minecraft:blocks/stone`, `minecraft:entities/zombie`);
    /// empty when none rolled: the block or mob has none, the `mob_drops` rule is off, a baby
    /// animal. A mob's drops may hold more: a charged creeper's head, a fox's mouth item.
    pub table: &'a str,
    /// The level they drop in (`minecraft:overworld`).
    pub dimension: &'a str,
    /// `LootContextParams.ORIGIN`: a broken block's centre, a mob's position.
    pub origin: [f64; 3],
    /// The block broken: its position and state.
    pub block: Option<([i32; 3], BlockStateId)>,
    /// The mob that died, or the entity a player's use dropped them from (the
    /// sheep sheared, the item frame broken): its entity ID and type
    /// (`minecraft:zombie`).
    pub entity: Option<(u32, &'a str)>,
    /// The player who broke the block or killed the mob, when known. A mob
    /// names one only while it remembers the player's hit (100 ticks), so one
    /// that dies to lava long after names none.
    pub player: Option<u128>,
    /// What broke the block (`LootContextParams.TOOL`).
    pub tool: Option<&'a ItemStack>,
    /// The radius of the explosion that broke the block.
    pub explosion: Option<f32>,
    /// A player's use dropped them rather than a break or a death: shears on a
    /// sheep, a mooshroom, a snow golem or a bogged, a brush on an armadillo or
    /// a suspicious block, shears carving a pumpkin, picking sweet berries or
    /// cave vines, a hit taking an item frame or its item, or untying a lead
    /// or cutting it with shears. `player` names them, or is `None` when a
    /// dispenser's shears sheared a mooshroom; `block` or `entity` says what
    /// dropped them. API 100. A mob that a player killed while
    /// leashed drops its lead on its next tick as a call of its own (`entity`
    /// the mob, `player` its killer, `table` empty, `used` false). API
    /// 102.
    pub used: bool,
}

/// An item that drops where a living entity died, before it is an item entity
/// in the level (`living_drops`): what `ItemEntity`'s constructor was given.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct NativeItemDrop {
    pub stack: ItemStack,
    /// Where the item entity appears.
    pub position: [f64; 3],
    /// Its motion in blocks per tick, as vanilla's constructor drew it.
    pub velocity: [f64; 3],
    /// Ticks before a player may pick it up.
    pub pickup_delay: u32,
}

impl NativeItemDrop {
    /// A drop of `stack` at `position` with a `velocity` and `pickup_delay`.
    pub fn new(
        stack: ItemStack,
        position: [f64; 3],
        velocity: [f64; 3],
        pickup_delay: u32,
    ) -> Self {
        Self {
            stack,
            position,
            velocity,
            pickup_delay,
        }
    }
}

/// A statistic type (`BuiltInRegistries.STAT_TYPE`), for a player's
/// statistics: what the stat's key names (a block, an item, an entity type,
/// or a `minecraft:custom` stat).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NativeStatType {
    /// `minecraft:mined`, keyed by block.
    Mined,
    /// `minecraft:crafted`, keyed by item.
    Crafted,
    /// `minecraft:used`, keyed by item.
    Used,
    /// `minecraft:broken`, keyed by item.
    Broken,
    /// `minecraft:picked_up`, keyed by item.
    PickedUp,
    /// `minecraft:dropped`, keyed by item.
    Dropped,
    /// `minecraft:killed`, keyed by entity type.
    Killed,
    /// `minecraft:killed_by`, keyed by entity type.
    KilledBy,
    /// `minecraft:custom`, keyed by custom stat (`minecraft:jump`...).
    Custom,
}

impl NativeStatType {
    /// The type's registry path (`mined`, `killed_by`...).
    pub fn path(self) -> &'static str {
        match self {
            Self::Mined => "mined",
            Self::Crafted => "crafted",
            Self::Used => "used",
            Self::Broken => "broken",
            Self::PickedUp => "picked_up",
            Self::Dropped => "dropped",
            Self::Killed => "killed",
            Self::KilledBy => "killed_by",
            Self::Custom => "custom",
        }
    }
}

/// A note block's instrument (`NoteBlockInstrument`), for `note_block_play`,
/// in the `instrument` property's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NativeNoteInstrument {
    Harp,
    Basedrum,
    Snare,
    Hat,
    Bass,
    Flute,
    Bell,
    Guitar,
    Chime,
    Xylophone,
    IronXylophone,
    CowBell,
    Didgeridoo,
    Bit,
    Banjo,
    Pling,
    Trumpet,
    TrumpetExposed,
    TrumpetOxidized,
    TrumpetWeathered,
    Zombie,
    Skeleton,
    Creeper,
    Dragon,
    WitherSkeleton,
    Piglin,
    CustomHead,
}

impl NativeNoteInstrument {
    /// Every instrument, in the `instrument` property's order.
    pub const ALL: [Self; 27] = {
        use NativeNoteInstrument::*;
        [
            Harp,
            Basedrum,
            Snare,
            Hat,
            Bass,
            Flute,
            Bell,
            Guitar,
            Chime,
            Xylophone,
            IronXylophone,
            CowBell,
            Didgeridoo,
            Bit,
            Banjo,
            Pling,
            Trumpet,
            TrumpetExposed,
            TrumpetOxidized,
            TrumpetWeathered,
            Zombie,
            Skeleton,
            Creeper,
            Dragon,
            WitherSkeleton,
            Piglin,
            CustomHead,
        ]
    };
    /// The `instrument` property's value (`harp`, `cow_bell`).
    pub fn name(self) -> &'static str {
        horizon_world::level::data::INSTRUMENTS[self as usize]
    }
}

/// The empty frame a fire is about to fill with a nether portal, for
/// `portal_spawn` (`BlockEvent.PortalSpawnEvent.getPortalSize`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NativePortalShape {
    /// The portal runs along X (else along Z).
    pub x_axis: bool,
    /// Its bottom-left inside block.
    pub bottom_left: [i32; 3],
    pub width: i32,
    pub height: i32,
}

/// A server explosion about to happen, for `explosion_start`
/// (`ServerLevel.explode`'s `ServerExplosion`).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct NativeExplosionStart<'a> {
    /// The level it happens in (`minecraft:overworld`).
    pub dimension: &'a str,
    pub center: [f64; 3],
    pub radius: f32,
    /// It sets fires (`createFire`).
    pub fire: bool,
    pub interaction: NativeBlockInteraction,
    /// The damage type of its damage source (`minecraft:explosion`,
    /// `minecraft:player_explosion`, `minecraft:bad_respawn_point`).
    pub damage_type: &'a str,
    /// The entity that explodes (`getDirectSourceEntity`): a creeper, a wind
    /// charge, the bearer of Wind Charged, when Horizon tracks it.
    pub source: Option<u32>,
    /// `getIndirectSourceEntity`: a living source itself, a primed TNT's
    /// owner, or a projectile's living owner (a ghast for its fireball, a
    /// wither for its skull, the thrower of a wind charge).
    pub owner: Option<u32>,
}

/// What an explosion does to blocks (`Explosion.BlockInteraction`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeBlockInteraction {
    Keep,
    Destroy,
    DestroyWithDecay,
    /// A wind burst: it pushes, and triggers doors, buttons, bells and the like.
    TriggerBlock,
}

/// A fishing catch, for `item_fished` (`ItemFishedEvent`).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct NativeItemFished<'a> {
    /// The bobber's entity id and position.
    pub hook: u32,
    pub position: [f64; 3],
    /// The items the fishing loot table rolled, which the player receives
    /// (`getDrops`: they can't be changed here; loot tables change them).
    pub drops: &'a [ItemStack],
}

/// The open screen, for the screen hooks (`native_screen_render_post`, the
/// mouse ones `native_screen_mouse_button_pressed_pre`, `..._released_pre` and
/// `native_screen_mouse_scrolled_pre`, and the key ones
/// `native_screen_key_pressed_pre` and `native_screen_char_typed_pre`): what
/// it is, its menu and title, where its container panel is, how many slots its
/// menu has, and where the mouse and the frame's partial tick are.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct NativeScreenView<'a> {
    /// One id per `Screen` (`Screen::id`): `inventory`, `creative`, `paused`,
    /// `stats`, `options`, `win`… Stable across releases, so a mod can match
    /// on it. Every container, station and mod window draws on the inventory
    /// screen and so reports `inventory`: tell them apart by `menu` and
    /// `title` (both None on a window the client opened itself) and by
    /// `slots`. The title's own screens report theirs (API 326): `title`,
    /// `world_select`, `world_create`, `world_create_flat`, `world_edit`,
    /// `multiplayer`, `add_server`, `direct_connect`…, with no menu, no title
    /// and no slots.
    pub id: &'static str,
    /// The type id of the menu the server opened for this screen
    /// (`minecraft:generic_9x3`, `minecraft:furnace`, a mod's own id; a mount's
    /// inventory is `horizon:mount<n>`). None on a screen without one (the
    /// client's own inventory, creative, the pause menu, stats…) and for a
    /// menu type the client does not know.
    pub menu: Option<&'a str>,
    /// The title the server gave the open menu, if it has one: the component
    /// itself, not a copy (`.plain()` it to draw, `chars()` for the styles).
    /// None when the server gave none, on a screen the client opened, and
    /// while playing.
    pub title: Option<&'a Text>,
    /// The screen's container panel in GUI pixels, `[x, y, width, height]`;
    /// the whole window for a screen with none. The panel is where the game
    /// draws the window: 176 wide for most, wider for the merchant and the
    /// beacon, a mod screen's own size for a mod's.
    pub rect: [f32; 4],
    /// The slots the open menu has, as `Game::slots` counts them: the player
    /// inventory's 36, then the container's, the station's, the mod screen's
    /// or the crafting grid's (a screen with no menu of its own still counts
    /// the inventory menu's). Counted, not listed.
    pub slots: usize,
    /// The pointer in GUI pixels (`Screen.mouseX`, `mouseY`, from the
    /// window's top left), as `native_cursor` gives it: for hover highlights.
    pub mouse: [f32; 2],
    /// This frame's partial tick (`getGameTimeDeltaPartialTick(false)`, 0 to
    /// 1): what the screen draws animation against.
    pub partial_tick: f32,
}

/// A depth-tested outline of the client's block selection shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeBlockRim {
    pub pos: [i32; 3],
    pub argb: u32,
    pub width: f32,
}

/// A camera-local visual light, with no gameplay light or shadow writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeColoredLight {
    pub at: [f64; 3],
    pub rgb: u32,
    pub radius: f32,
    pub intensity: f32,
}

/// The minimap a native mod shows (`native_minimap`), drawn by Horizon's
/// minimap: its corner, its side in GUI pixels, blocks per map pixel,
/// turning with the player or north up, round or square.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeMinimap {
    pub corner: Corner,
    pub size: f32,
    pub blocks_per_pixel: f32,
    pub rotate: bool,
    pub round: bool,
}

/// A mod block's screen over its menu (`native_screen`): its size, where
/// each of the container's slots is (`Slot.x`, `Slot.y`; none hides the
/// slot, and so do slots past the list; the server's menu does not have a
/// hidden slot either, so clicks and shift-clicks skip it), and the player inventory's first
/// row (vanilla's is 84; the hotbar is 58 below it) and left column, from
/// its top left.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeScreen {
    /// Vanilla's container screens are 176 wide.
    pub width: f32,
    pub height: f32,
    pub slots: Vec<Option<[f32; 2]>>,
    pub inventory_y: f32,
    /// The inventory's left column (vanilla's is 8).
    pub inventory_x: f32,
    /// The mod draws the title and the "Inventory" label itself
    /// (`native_screen_draw`); Horizon leaves them out.
    pub own_labels: bool,
    /// The mod draws its own background (`native_screen_draw`), so Horizon
    /// leaves out the plain panel and the slot frames.
    pub own_background: bool,
}

/// A native mod as Options > Mods lists it (`native_mods`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeMod {
    /// Its id: its settings folder and its keys' lines.
    pub id: String,
    /// `Name 1.0`.
    pub title: String,
    pub description: String,
    /// Its keys: name, label and default key (`key.keyboard.z`). The page
    /// lists them first; `native_mod_key` reads the player's choice.
    pub keys: Vec<[String; 3]>,
}

/// An item the panel shows (`native_panel_items`): its id
/// (`minecraft:diamond`), a count (1 draws none) and its components beyond
/// the item's own, in vanilla's item syntax, or "".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeStack {
    pub item: String,
    pub count: u32,
    pub components: String,
}

/// What the player did on a native mod's panel (`native_panel_event`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum NativePanelEvent {
    /// A click on an item of the panel, by its index in
    /// `native_panel_items`' list.
    Clicked { index: u32, right: bool },
    /// The search box's text, each time the player changes it.
    Search(String),
    /// A key (`key.keyboard.r`; compare it with `native_mod_key`) went down
    /// while the pointer was on an item: a slot of the screen's, or the
    /// panel's.
    Key { key: String, stack: NativeStack },
    /// A click on a slot of a recipe the panel shows: the recipe's index in
    /// `native_panel_recipes`' list and the slot's in it (the results come
    /// after the slots), and the stack it showed then.
    SlotClicked {
        recipe: u32,
        slot: u32,
        stack: NativeStack,
        right: bool,
    },
    /// The action button of the recipe at this index.
    Action(u32),
    /// The player turned the recipes' page: the index of the first recipe
    /// now shown.
    Page(u32),
    /// The recipe page's back button.
    Back,
    /// The player picked the tab at this index.
    Tab(u32),
}

/// The zoom a native mod asks for (`native_zoom`): the factor (1 is none), eased through the
/// field of view like the spyglass when `smooth`, and the scale of mouse turning.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeZoom {
    pub factor: f32,
    pub smooth: bool,
    pub turn: f32,
}

/// A player on the server, as a server hook sees them.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NativeServerPlayer {
    pub uuid: u128,
    pub name: String,
    /// The operator level, 0 to 4 (`ops.json`; single player's owner with
    /// cheats on is 4).
    pub permission: u8,
    /// `minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`.
    pub dimension: String,
    pub position: [f64; 3],
    /// 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: i32,
    /// Health points, 20 at full; 0 once dead. API 21.
    pub health: f32,
    /// Where they look, in degrees (`yRot`, `xRot`), as their last move told
    /// the server. API 49.
    pub yaw: f32,
    pub pitch: f32,
}

/// A command a native mod adds, as one usage: words, then arguments
/// `<name:kind>`, optional ones last in brackets `[name:kind]`:
/// `pay <player:player> <amount:integer>`, `balance [player:player]`,
/// `eco give <player:player> <amount:integer>`. Kinds: `word`, `text` (the
/// rest of the line, last), `integer`, `number`, `boolean`, `player`,
/// `item`, `block_pos`. Usages of one command share their words; the
/// server merges them into one tree.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NativeCommand {
    pub usage: String,
    /// The operator level a player needs to see and run it (0: everyone).
    pub level: u8,
}

impl NativeCommand {
    /// A usage every player may run.
    pub fn new(usage: &str) -> Self {
        Self {
            usage: usage.to_owned(),
            level: 0,
        }
    }
    /// The same, for operators of `level` and above.
    pub fn op(usage: &str, level: u8) -> Self {
        Self {
            usage: usage.to_owned(),
            level: level.min(4),
        }
    }
}

/// Which `Game` observes the phase; a mirror `Game` never fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeAuthority {
    LocalAuthoritative,
    IntegratedServer,
    DedicatedServer,
    RemoteClient,
}

/// Supported vanilla `Block.UPDATE_*` flags for a level block write.
/// Horizon currently sends all writes to clients regardless of client flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeBlockUpdateFlags(i32);

impl NativeBlockUpdateFlags {
    /// No update flags.
    pub const NONE: Self = Self(0);
    /// Notify neighboring blocks.
    pub const NEIGHBORS: Self = Self(1);
    /// Skip neighbor shape updates.
    pub const KNOWN_SHAPE: Self = Self(16);
    /// Suppress drops caused by neighbor shape updates.
    pub const SUPPRESS_DROPS: Self = Self(32);
    /// The replacement is caused by a piston.
    pub const MOVE_BY_PISTON: Self = Self(64);
    /// Skip wire shape updates.
    pub const SKIP_SHAPE_UPDATE_ON_WIRE: Self = Self(128);
    /// Skip the removed block entity's side effects, including container spills.
    pub const SKIP_BLOCK_ENTITY_SIDE_EFFECTS: Self = Self(256);
    /// Skip the new block's on-place behavior.
    pub const SKIP_ON_PLACE: Self = Self(512);
    #[doc(hidden)]
    pub fn bits(self) -> i32 {
        self.0
    }
}

impl std::ops::BitOr for NativeBlockUpdateFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// One open attempt of one world; it rejects a previous open's commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeWorldEpoch(u64);

impl NativeWorldEpoch {
    /// Create an epoch for the game.
    #[doc(hidden)]
    pub fn new(n: u64) -> Self {
        Self(n)
    }
    /// Read the epoch number.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// An entity of the open world, for the `Host` methods that read or change one
/// (`entity.position(host)`). It holds the world's epoch and the entity's id: ids
/// are shared by every level and never reused while the world is open, and an
/// entity that changes dimension gets a new one, as in vanilla. A handle from an
/// earlier open, of an entity that is gone, or used on a client reads as `None`
/// and changes nothing. `Host::entity` makes one from an event's entity id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntityRef {
    epoch: NativeWorldEpoch,
    id: u32,
}

impl EntityRef {
    /// Create a handle for the game.
    #[doc(hidden)]
    pub fn new(epoch: NativeWorldEpoch, id: u32) -> Self {
        Self { epoch, id }
    }
    /// The entity's id (the network id events carry).
    pub fn id(self) -> u32 {
        self.id
    }
    /// The open of the world the handle belongs to.
    pub fn epoch(self) -> NativeWorldEpoch {
        self.epoch
    }
}

/// A living entity of the open world: a mob, a player, an armor stand or the
/// dragon (vanilla's `LivingEntity`). `host.living(entity)` makes one from an
/// [`EntityRef`] of such an entity, and [`LivingRef::entity`] goes back, so the
/// entity's methods work on it too. Stale handles read as `None`, as an
/// `EntityRef`'s do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LivingRef(EntityRef);

impl LivingRef {
    /// Create a handle for the game.
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
    /// The entity's id (the network id events carry).
    pub fn id(self) -> u32 {
        self.0.id()
    }
}

impl From<LivingRef> for EntityRef {
    fn from(living: LivingRef) -> Self {
        living.0
    }
}

/// A vanilla `Mob`, narrowed from an entity with `host.mob(entity)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MobRef(EntityRef);

impl MobRef {
    /// Create a handle for the game.
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an entity handle.
    pub fn entity(self) -> EntityRef {
        self.0
    }
    /// The same entity as a living handle.
    pub fn living(self) -> LivingRef {
        LivingRef::new(self.0)
    }
}

/// A dropped item entity of the open world, narrowed with `host.item_entity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemEntityRef(EntityRef);

impl ItemEntityRef {
    /// Create a handle for the game.
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
}

/// A villager or wandering trader (vanilla's `AbstractVillager`, a `Merchant`),
/// narrowed from an entity with `host.merchant(entity)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MerchantRef(EntityRef);

impl MerchantRef {
    /// Create a handle for the game.
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
}

/// A level of the open world, by its dimension (`minecraft:the_nether`).
/// `Host::level` makes one, and `entity.level(host)` gives an entity's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LevelRef {
    epoch: NativeWorldEpoch,
    dimension: &'static str,
}

impl LevelRef {
    /// Create a handle for the game.
    #[doc(hidden)]
    pub fn new(epoch: NativeWorldEpoch, dimension: &'static str) -> Self {
        Self { epoch, dimension }
    }
    /// The level's dimension (`minecraft:overworld`).
    pub fn dimension(self) -> &'static str {
        self.dimension
    }
    /// The open of the world the handle belongs to.
    pub fn epoch(self) -> NativeWorldEpoch {
        self.epoch
    }
}

/// A chunk position in an open level. A handle may be kept after unloading;
/// reads resolve it again, returning `false` or `None` when it is no longer loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChunkRef {
    pub(crate) level: LevelRef,
    pub(crate) pos: [i32; 2],
}

impl ChunkRef {
    /// Create a handle for the game.
    #[doc(hidden)]
    pub fn new(level: LevelRef, pos: [i32; 2]) -> Self {
        Self { level, pos }
    }
    /// The chunk coordinates, `[x, z]`.
    pub fn pos(self) -> [i32; 2] {
        self.pos
    }
    /// The level containing the chunk.
    pub fn level(self) -> LevelRef {
        self.level
    }
}

/// The status of a loaded chunk that Horizon can expose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChunkStatus {
    /// A full chunk in a level's live world.
    Full,
}

/// What an explosion a mod sets off does to blocks
/// (`Level.ExplosionInteraction`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NativeExplosionInteraction {
    /// Blocks stay; entities are hurt and pushed (`NONE`).
    None,
    /// Blocks break and drop by the `block_explosion_drop_decay` rule (`BLOCK`).
    #[default]
    Block,
    /// Blocks break and drop by the `tnt_explosion_drop_decay` rule (`TNT`).
    Tnt,
}

/// Which volume slider a sound is played under (`SoundSource`). A sound source
/// is the group a client's sound options control: an entity's is
/// `Entity.getSoundSource`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeSoundSource {
    Master,
    Music,
    /// Jukeboxes and note blocks (`RECORDS`).
    Records,
    Weather,
    /// Blocks (`BLOCKS`).
    Blocks,
    Hostile,
    Neutral,
    /// Players (`PLAYERS`).
    Players,
    Ambient,
    Voice,
    Ui,
}

impl NativeSoundSource {
    /// Its position in vanilla's `SoundSource` order, as the network sends it.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// The empty-context block shape requested from a level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeBlockShapeKind {
    /// Selection outline (`BlockState.getShape`).
    Outline,
    /// Collision shape (`BlockState.getCollisionShape`).
    Collision,
}

/// `ChatVisiblity`: which chat a client shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeChatVisibility {
    Full,
    System,
    Hidden,
}

/// `PermissionLevel`: how much a player may do, as `op-permission-level` and
/// the operator list name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NativePermissionLevel {
    /// 0: every player.
    All,
    /// 1: moderators.
    Moderators,
    /// 2: game masters.
    Gamemasters,
    /// 3: admins.
    Admins,
    /// 4: owners.
    Owners,
}

impl NativePermissionLevel {
    /// The level a list entry's number names; out of range keeps to 0 or 4.
    pub fn from_level(level: i32) -> Self {
        match level {
            i32::MIN..=0 => Self::All,
            1 => Self::Moderators,
            2 => Self::Gamemasters,
            3 => Self::Admins,
            _ => Self::Owners,
        }
    }
    /// The level as the list stores it, 0 to 4.
    pub fn level(self) -> u8 {
        self as u8
    }
}

/// `HumanoidArm`: a player's main hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeMainHand {
    Left,
    Right,
}

/// `ParticleStatus`: how many particles a client shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeParticleStatus {
    All,
    Decreased,
    Minimal,
}

/// A player's client options, as `ServerboundClientInformationPacket` carries them
/// (vanilla's `ClientInformation`). Raw ids map as vanilla's `ByIdMap` does: chat
/// visibility and particle status wrap (`floorMod(id, 3)`), the main hand reads an
/// unknown id as left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeClientInfo {
    pub language: String,
    pub view_distance: i8,
    pub chat_visibility: NativeChatVisibility,
    pub chat_colors: bool,
    /// Skin layer bits (`PlayerModelPart`).
    pub model_customisation: u8,
    pub main_hand: NativeMainHand,
    pub text_filtering: bool,
    pub allows_listing: bool,
    pub particle_status: NativeParticleStatus,
}

impl From<&horizon_protocol::packets::configuration::ClientInformation> for NativeClientInfo {
    fn from(i: &horizon_protocol::packets::configuration::ClientInformation) -> Self {
        Self {
            language: i.language.clone(),
            view_distance: i.view_distance,
            chat_visibility: match i.chat_visibility.rem_euclid(3) {
                0 => NativeChatVisibility::Full,
                1 => NativeChatVisibility::System,
                _ => NativeChatVisibility::Hidden,
            },
            chat_colors: i.chat_colors,
            model_customisation: i.model_customisation,
            main_hand: if i.main_hand == 1 {
                NativeMainHand::Right
            } else {
                NativeMainHand::Left
            },
            text_filtering: i.text_filtering,
            allows_listing: i.allows_listing,
            particle_status: match i.particle_status.rem_euclid(3) {
                0 => NativeParticleStatus::All,
                1 => NativeParticleStatus::Decreased,
                _ => NativeParticleStatus::Minimal,
            },
        }
    }
}

/// A block face for a level support query.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeBlockFace {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

/// The support area tested on a block face.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeSupportType {
    Full,
    Center,
    Rigid,
}

/// Why a mod removes an entity (`Entity.RemovalReason`): the two reasons
/// that destroy it. The unloading and dimension-change reasons belong to the
/// game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeRemovalReason {
    /// `KILLED`: a mob's effects that act on removal run.
    Killed,
    /// `DISCARDED`, as `Entity.discard`.
    Discarded,
}

/// Block shape used by an empty-context level ray.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeClipBlock {
    Collider,
    Outline,
}

/// Fluid mode for a level ray. More modes are a follow-up.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeClipFluid {
    None,
}

/// Result of a block ray, including the vanilla miss result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeClipHit {
    pub miss: bool,
    pub block: [i32; 3],
    pub face: NativeBlockFace,
    pub location: [f64; 3],
    pub inside: bool,
    pub world_border: bool,
}

/// One item cooldown a player waits on (`ItemCooldowns.cooldowns`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeItemCooldown {
    /// The cooldown group: an item id, or the group an item's
    /// `use_cooldown` component names.
    pub group: String,
    /// Ticks until the cooldown ends.
    pub ticks_left: i32,
}

/// An online player in one open server world, keyed by UUID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlayerRef {
    pub(crate) entity: EntityRef,
    pub(crate) uuid: u128,
}

impl PlayerRef {
    /// The same player as an entity handle.
    pub fn entity(self) -> EntityRef {
        self.entity
    }
    /// The player's UUID.
    pub fn uuid(self) -> u128 {
        self.uuid
    }
}

impl From<PlayerRef> for EntityRef {
    fn from(player: PlayerRef) -> Self {
        player.entity
    }
}

/// A player's game mode (vanilla's `GameType`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
}

impl GameMode {
    /// Every mode, in id order.
    pub const ALL: [Self; 4] = [
        Self::Survival,
        Self::Creative,
        Self::Adventure,
        Self::Spectator,
    ];
    /// `GameType.getId`: 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub fn id(self) -> i32 {
        match self {
            Self::Survival => 0,
            Self::Creative => 1,
            Self::Adventure => 2,
            Self::Spectator => 3,
        }
    }
    /// The mode with `GameType` id `id`, or None for an id vanilla doesn't
    /// have.
    pub fn from_id(id: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.id() == id)
    }
    /// `GameType.getName`: `survival`, `creative`, `adventure`, `spectator`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Survival => "survival",
            Self::Creative => "creative",
            Self::Adventure => "adventure",
            Self::Spectator => "spectator",
        }
    }
}

/// A custom boss bar (vanilla's `CustomBossEvent`, the `/bossbar` ones) of an
/// open server world. It holds the world's epoch and the bar's UUID, so a bar
/// that was removed (even if its id was added again) or a handle from an
/// earlier open reads as `None` and changes nothing. Mob and raid bars are not
/// custom boss events and have no handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BossBarRef {
    pub(crate) epoch: NativeWorldEpoch,
    pub(crate) uuid: u128,
}

/// `BossEvent.BossBarColor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BossBarColor {
    Pink,
    Blue,
    Red,
    Green,
    Yellow,
    Purple,
    White,
}

impl BossBarColor {
    /// Every color, in ordinal order.
    pub const ALL: [Self; 7] = [
        Self::Pink,
        Self::Blue,
        Self::Red,
        Self::Green,
        Self::Yellow,
        Self::Purple,
        Self::White,
    ];
    /// `BossBarColor.getName`: `pink`, `blue`, `red`, `green`, `yellow`,
    /// `purple` or `white`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Pink => "pink",
            Self::Blue => "blue",
            Self::Red => "red",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Purple => "purple",
            Self::White => "white",
        }
    }
    /// The color with that name, or None for one vanilla doesn't have.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|color| color.name() == name)
    }
}

/// `BossEvent.BossBarOverlay`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BossBarOverlay {
    Progress,
    Notched6,
    Notched10,
    Notched12,
    Notched20,
}

impl BossBarOverlay {
    /// Every overlay, in ordinal order.
    pub const ALL: [Self; 5] = [
        Self::Progress,
        Self::Notched6,
        Self::Notched10,
        Self::Notched12,
        Self::Notched20,
    ];
    /// `BossBarOverlay.getName`: `progress`, `notched_6`, `notched_10`,
    /// `notched_12` or `notched_20`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Progress => "progress",
            Self::Notched6 => "notched_6",
            Self::Notched10 => "notched_10",
            Self::Notched12 => "notched_12",
            Self::Notched20 => "notched_20",
        }
    }
    /// The overlay with that name, or None for one vanilla doesn't have.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|overlay| overlay.name() == name)
    }
}

/// One player's open menu, invalidated on close, replacement or reconnect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MenuRef {
    pub(crate) player: PlayerRef,
    pub(crate) window: i32,
    pub(crate) generation: u64,
}

/// A player's food data.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct NativePlayerFood {
    /// Food level.
    pub level: i32,
    /// Food saturation.
    pub saturation: f32,
    /// Food exhaustion.
    pub exhaustion: f32,
}

/// A player's experience (`Player.experienceLevel`, `experienceProgress`,
/// `totalExperience`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct NativePlayerExperience {
    /// Experience level.
    pub level: i32,
    /// Progress toward the next level, from 0 to 1.
    pub progress: f32,
    /// Points gained in all.
    pub total: i32,
}

/// Damage sources supported for a mod effect tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeEffectDamage {
    /// Poison's magic damage.
    Magic,
    /// The Wither effect's damage.
    Wither,
}
impl NativeEffectDamage {
    pub(crate) fn mob_kind(self) -> crate::mobs::DamageKind {
        match self {
            Self::Magic => crate::mobs::DamageKind::Magic,
            Self::Wither => crate::mobs::DamageKind::Wither,
        }
    }
}

/// A change requested by an effect tick, applied after its hooks return.
#[doc(hidden)]
pub enum EffectTickChange {
    Heal(f32),
    Hurt(NativeEffectDamage, f32),
    AddExhaustion(f32),
    Eat(i32, f32),
}

impl ApplyEffectTickEvent<'_> {
    /// Queue `LivingEntity.heal` on the holder.
    pub fn heal(&mut self, amount: f32) {
        self.changes.push(EffectTickChange::Heal(amount));
    }

    /// Queue damage with a supported source.
    pub fn hurt(&mut self, damage_type: NativeEffectDamage, amount: f32) {
        self.changes
            .push(EffectTickChange::Hurt(damage_type, amount));
    }

    /// Queue food exhaustion; a non-player holder ignores it.
    pub fn add_exhaustion(&mut self, amount: f32) {
        self.changes.push(EffectTickChange::AddExhaustion(amount));
    }

    /// Queue `FoodData.eat`; a non-player holder ignores it.
    pub fn eat(&mut self, nutrition: i32, saturation_modifier: f32) {
        self.changes
            .push(EffectTickChange::Eat(nutrition, saturation_modifier));
    }
}

/// A player's server-side abilities (`Abilities`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct NativePlayerAbilities {
    /// Whether ordinary damage is ignored.
    pub invulnerable: bool,
    /// Whether the player is flying.
    pub flying: bool,
    /// Whether the player may fly.
    pub may_fly: bool,
    /// Whether items are used without consumption.
    pub instabuild: bool,
    /// Whether the player may build.
    pub may_build: bool,
    /// Flying speed.
    pub fly_speed: f32,
    /// Walking speed (`Abilities.walkingSpeed`, vanilla's 0.1).
    pub walk_speed: f32,
}

/// A block entity at a position in an open level. A replacement at the same
/// position resolves to the new entity, unlike vanilla's object identity.
/// A removed entity or a handle from an earlier world open reads as `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockEntityRef {
    pub(crate) level: LevelRef,
    pub(crate) position: [i32; 3],
}

/// Non-paused authoritative server ticks since open (not game or frame time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeTickId(u64);

impl NativeTickId {
    /// Create a tick ID for the game.
    #[doc(hidden)]
    pub fn new(n: u64) -> Self {
        Self(n)
    }
    /// Read the tick number.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// The world phases (`native_world_lifecycle`'s `phase`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The phase names are the hook log's stable vocabulary (the Debug text
/// the mixins write); keep them despite the shared prefix.
#[allow(clippy::enum_variant_names)]
#[non_exhaustive]
pub enum WorldPhase {
    WorldOpenReady,
    /// The open failed; `error_text` is the engine's own error text.
    WorldOpenFailed,
    WorldCloseFinal,
    /// The server loop or final save failed; `error_text` carries the error.
    WorldCloseFailed,
}

/// The tick phases carry whether the world runs, including frozen steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ServerTickPhase {
    ServerTickStart { world_running: bool },
    ServerTickEnd { world_running: bool },
}

/// Context for every world lifecycle phase.
/// `authority` is the server role. LocalAuthoritative and RemoteClient never
/// fire today. `epoch` identifies the open attempt, including failed opens;
/// the same epoch identifies its ready, final-close, or failed-close event.
/// `tick` counts non-paused server ticks since the world opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NativeLifecycleContext {
    pub authority: NativeAuthority,
    pub epoch: NativeWorldEpoch,
    pub tick: NativeTickId,
}

/// Tool action requested by a block interaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolAbility {
    AxeStrip,
    AxeScrape,
    AxeWaxOff,
    ShovelFlatten,
    ShovelDouse,
    HoeTill,
}

/// The player responsible for a game rule change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameRuleActor<'a> {
    /// Player UUID.
    pub uuid: u128,
    /// Server entity ID.
    pub entity_id: i32,
    /// Player name.
    pub name: &'a str,
}

/// How a criterion's advancement progress changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeProgressType {
    Grant,
    Revoke,
}

/// Read-only context for a tool modification.
#[non_exhaustive]
pub struct ToolUse {
    /// The acting entity ID, if present.
    pub player: Option<u32>,
    /// The hand holding the tool.
    pub hand: crate::first_person::Hand,
    /// The clicked block position.
    pub pos: [i32; 3],
    /// The clicked face's direction.
    pub face: [i32; 3],
    /// The click position in world coordinates.
    pub click: [f64; 3],
    /// The action being considered.
    pub ability: ToolAbility,
    /// Whether this is a simulation. Current vanilla calls pass false.
    pub simulated: bool,
}

/// The ordered list of mods shared by the client and server.
pub type Mods = Arc<[Box<dyn Hooks>]>;

pub(crate) struct Installed {
    mods: Mods,
    subscribers: horizon_client_core::subscribers::Subscribers,
    types: Box<[std::any::TypeId]>,
}

impl Installed {
    fn new(mods: Mods) -> Self {
        let types = mods
            .iter()
            .map(|m| (&**m as &dyn std::any::Any).type_id())
            .collect();
        let subscribers =
            horizon_client_core::subscribers::Subscribers::new(generated::HOOK_COUNT, mods.len());
        Self {
            mods,
            subscribers,
            types,
        }
    }
}

static MODS: OnceLock<Installed> = OnceLock::new();

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static TEST_MODS: std::cell::Cell<Option<&'static Installed>> = const { std::cell::Cell::new(None) };
}

fn installed() -> Option<&'static Installed> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(installed) = TEST_MODS.get() {
        return Some(installed);
    }
    MODS.get()
}

/// Scoped trait-mod installation for tests, enabled by the `testing` feature.
#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use super::{Installed, Mods, TEST_MODS};

    /// Run a closure with these owned mods on this thread, restoring the previous list on exit.
    ///
    /// The test allocation is leaked because dispatch exposes a static slice. Nested scopes and
    /// panic unwinding restore the preceding list; other threads are unaffected.
    pub fn with_mods<R>(mods: Mods, f: impl FnOnce() -> R) -> R {
        struct Restore(Option<&'static Installed>);
        impl Drop for Restore {
            fn drop(&mut self) {
                TEST_MODS.set(self.0);
            }
        }
        let installed = Box::leak(Box::new(Installed::new(mods)));
        let _restore = Restore(TEST_MODS.replace(Some(installed)));
        f()
    }
}

/// Install the process-wide ordered list of mods.
pub(crate) fn install(mods: Mods) -> &'static Installed {
    let installed = Installed::new(mods);
    #[cfg(test)]
    {
        // Tests can replace the list; mods() keeps its static slice lifetime.
        let installed = Box::leak(Box::new(installed));
        TEST_MODS.set(Some(installed));
        installed
    }
    #[cfg(not(test))]
    {
        assert!(MODS.set(installed).is_ok(), "trait mods are installed once");
        MODS.get().unwrap()
    }
}

/// The process-wide list in dispatch order, or an empty list before installation.
pub fn mods() -> &'static [Box<dyn Hooks>] {
    installed().map_or(&[], |installed| &installed.mods)
}

fn listened(hook: generated::HookId) -> bool {
    installed().is_some_and(|installed| installed.subscribers.contains_any(hook as usize))
}

/// The bundles' `[mod] id`s, one a [`mods`] entry in the same order.
static MOD_IDS: OnceLock<&'static [&'static str]> = OnceLock::new();

/// Names the entries of [`mods`] by their bundles' `[mod] id`s, so a panic in a hook names its mod.
/// The launcher's generated executable calls it before `entry::main_with`.
pub fn name_mods(ids: &'static [&'static str]) {
    if MOD_IDS.set(ids).is_err() {
        log::warn!("Trait mods already named; ignoring the second list");
    }
}

/// `mods` in order, each marked as running `hook` until the next one is (or the loop ends), so a
/// panic in it names the mod. The generated dispatch loops over it.
fn entered_registration<'a>(mods: &'a [Box<dyn Hooks>], hook: &'static str) -> Entered<'a> {
    Entered {
        mods: mods.iter().enumerate(),
        hook,
        guard: None,
    }
}

fn entered(
    hook_id: generated::HookId,
    hook: &'static str,
) -> impl Iterator<Item = &'static Box<dyn Hooks>> {
    installed().into_iter().flat_map(move |installed| {
        let mut crash_guard = None;
        let mut subscriber_guard = None;
        installed
            .subscribers
            .indices(hook_id as usize)
            .map(move |index| {
                drop(subscriber_guard.take());
                drop(crash_guard.take());
                let hooks = &installed.mods[index];
                subscriber_guard = Some(horizon_client_core::subscribers::enter(
                    &installed.subscribers,
                    hook_id as usize,
                    index,
                    (&**hooks as *const dyn Hooks).cast(),
                    installed.types[index],
                ));
                crash_guard = Some(mark_mod(index, hook));
                hooks
            })
    })
}

fn mark_mod(index: usize, hook: &'static str) -> crate::crash::NativeModGuard {
    crate::crash::enter_native_mod(crate::crash::NativeModCall {
        id: MOD_IDS.get().and_then(|ids| ids.get(index)).copied(),
        index,
        hook,
        part: None,
    })
}

/// The parts in `parts`, each marked as running `hook` as `entered` marks a mod, with its
/// type, so a panic in it names the mod, the hook and the part.
fn entered_parts<'a, P: ?Sized>(
    parts: &'a [Attached<P>],
    hook: &'static str,
) -> impl Iterator<Item = &'a P> {
    let mut guard = None;
    parts.iter().map(move |attached| {
        // The last part's mark goes first, as in `Entered`.
        drop(guard.take());
        guard = Some(crate::crash::enter_native_mod(
            crate::crash::NativeModCall {
                id: MOD_IDS
                    .get()
                    .and_then(|ids| ids.get(attached.mod_index))
                    .copied(),
                index: attached.mod_index,
                hook,
                part: Some(attached.type_name),
            },
        ));
        &*attached.part
    })
}

/// Mod `index` of [`mods`] by its `[mod] id`, or by its place when the build didn't name them.
fn mod_name(index: usize) -> String {
    MOD_IDS
        .get()
        .and_then(|ids| ids.get(index))
        .map_or_else(|| format!("native mod #{index}"), |id| (*id).to_owned())
}

struct Entered<'a> {
    mods: std::iter::Enumerate<std::slice::Iter<'a, Box<dyn Hooks>>>,
    hook: &'static str,
    guard: Option<crate::crash::NativeModGuard>,
}

impl<'a> Iterator for Entered<'a> {
    type Item = &'a Box<dyn Hooks>;

    fn next(&mut self) -> Option<Self::Item> {
        // The last mod's mark goes first, so the next one's puts back what was there before.
        drop(self.guard.take());
        let (index, hooks) = self.mods.next()?;
        self.guard = Some(mark_mod(index, self.hook));
        Some(hooks)
    }
}

/// Whether a mod claims a cancellable hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Later mods and the game continue.
    Continue,
    /// Set the hook flag and skip later mods.
    Cancel,
}

/// Where a block use clicked, as vanilla's `BlockHitResult` says it. Fields
/// may be added: build one with [`BlockHit::new`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BlockHit {
    /// The point clicked, in world coordinates (`getLocation`).
    pub point: [f64; 3],
    /// The face clicked, `[0, 1, 0]` for the top (`getDirection`).
    pub face: [i32; 3],
    /// The ray started inside the block's shape (`isInside`).
    pub inside: bool,
}
impl BlockHit {
    pub const fn new(point: [f64; 3], face: [i32; 3], inside: bool) -> Self {
        Self {
            point,
            face,
            inside,
        }
    }
}

/// A player's right click on a block, for `right_click_block`
/// (`PlayerInteractEvent.RightClickBlock`): once per hand, where
/// `useItemOn` starts, on the clicking client and on the server.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct NativeBlockClick<'a> {
    /// The side running the click (`getSide`).
    pub side: LogicalSide,
    /// The clicking player's level (`getLevel`).
    pub dimension: LevelDimension,
    /// The block clicked (`getPos`).
    pub pos: [i32; 3],
    /// Where it was clicked (`getHitVec`); `hit.face` is `getFace`.
    pub hit: BlockHit,
    /// The hand clicking (`getHand`) and what it holds (`getItemStack`).
    pub hand: Hand,
    pub stack: Option<&'a ItemStack>,
    /// The clicking player's UUID and entity id (`getEntity`).
    pub player: u128,
    pub player_entity_id: i32,
}

/// Vanilla's `InteractionResult`, as a hook answers it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeInteractionResult {
    /// `SUCCESS`: the action is used and the hand swings on the clicking
    /// client (`SwingSource.PREDICTED`).
    Success,
    /// `SUCCESS_SERVER`: the action is used and the server swings the hand.
    SuccessServer,
    /// `CONSUME`: the action is used, with no swing.
    Consume,
    /// `FAIL`: nothing happens and the click ends.
    Fail,
    /// `PASS`: nothing happens and the click goes on.
    #[default]
    Pass,
    /// `TRY_WITH_EMPTY_HAND`: passes, as `PASS` does here.
    TryWithEmptyHand,
}
impl NativeInteractionResult {
    /// `consumesAction`: the click is used up.
    pub fn consumes_action(self) -> bool {
        matches!(self, Self::Success | Self::SuccessServer | Self::Consume)
    }
}

/// NeoForge's `TriState`: force a part of an action off or on, or leave it
/// to vanilla.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NativeTriState {
    False,
    #[default]
    Default,
    True,
}
impl NativeTriState {
    /// Whether the part runs, `vanilla` deciding for `Default`.
    pub fn or(self, vanilla: bool) -> bool {
        match self {
            Self::False => false,
            Self::Default => vanilla,
            Self::True => true,
        }
    }
}

mod generated_host {
    include!(concat!(env!("OUT_DIR"), "/mod_api_generated_host.rs"));
}
pub use generated_host::Host;

/// Result of accessing or decoding an attachment.
pub type AttachmentResult<T> = std::result::Result<T, AttachmentError>;

impl AttachmentHost for dyn Host + '_ {
    fn attachments(&mut self, holder: Holder) -> Result<&mut Attachments, AttachmentError> {
        Host::attachments(self, holder)
    }
}

/// Read-only level services, like vanilla's `LevelReader`.
pub trait LevelHost {
    /// Read a block through `LevelAccess::block_state`.
    fn block_state(&self, pos: [i32; 3]) -> BlockStateId;
    /// `Level.isClientSide`, through `LevelAccess::is_client_side`.
    fn is_client_side(&self) -> bool;
    /// The block `state` is a state of (`BlockState.getBlock()`): `mymod:shaft`. None for an
    /// unknown state.
    fn state_block(&self, state: BlockStateId) -> Option<&str>;
    /// `BlockState.setValue`, through `native_state_with_property`; None if invalid.
    fn state_with_property(
        &self,
        state: BlockStateId,
        key: &str,
        value: &str,
    ) -> Option<BlockStateId>;
}

/// Access vanilla's `Level.random`, so draws come in vanilla's order.
pub trait LevelRandomHost: LevelHost {
    /// Access a holder's server-owned attachment map.
    fn attachments(&mut self, holder: Holder) -> Result<&mut Attachments, AttachmentError>;
    /// The level's random stream through `native_level_random`, or None on a non-authoritative level.
    fn random(&mut self) -> Option<&mut LegacyRandom>;
}

/// A `LevelReader` with vanilla's `ScheduledTickAccess`. Shape updates cannot write blocks.
pub trait LevelTicksHost: LevelHost {
    /// Schedule a block tick through `native_schedule_block_tick`.
    fn schedule_block_tick(&mut self, pos: [i32; 3], block: &str, delay: i32) -> bool;
    /// Schedule a fluid tick through `native_schedule_fluid_tick`.
    fn schedule_fluid_tick(&mut self, pos: [i32; 3], fluid: &str, delay: i32) -> bool;
}

/// Mutable level services, like vanilla's `ServerLevel`. Whole-block writes wait for block entity support.
pub trait LevelWriteHost: LevelTicksHost + LevelRandomHost {
    /// Change a block property through `native_set_block_property`.
    fn set_block_property(&mut self, pos: [i32; 3], key: &str, value: &str, flags: i32) -> bool;
    /// Play a sound for nearby players through `native_play_sound`.
    fn play_sound(
        &mut self,
        sound: &str,
        at: [f64; 3],
        source: usize,
        volume: f32,
        pitch: f32,
    ) -> bool;
    /// `ServerLevel.sendParticles` through `native_send_particles`.
    fn send_particles(
        &mut self,
        options: &ParticleOptions,
        state: Option<BlockStateId>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool;
}

macro_rules! level_attachment_host {
    ($($host:ident),+) => { $(
        impl AttachmentHost for dyn $host + '_ {
            fn attachments(&mut self, holder: Holder) -> Result<&mut Attachments, AttachmentError> {
                LevelRandomHost::attachments(self, holder)
            }
        }
    )+ };
}
level_attachment_host!(LevelRandomHost, LevelWriteHost);

mod capability;
pub use capability::{BlockSide, Capability, CapabilityKey, ITEMS};
mod entity_capability;
pub use entity_capability::{ENTITY_ITEMS, EntityCapabilityKey};
mod fluid_capability;
pub use fluid_capability::{FLUIDS, FluidResource, FluidStack, ITEM_FLUIDS};
mod item_place;
mod menu_slot;
pub use menu_slot::MenuSlotBacking;
mod transaction;
pub use item_place::{ITEM_ITEMS, ItemCapabilityKey, ItemPlace, PlaceProvider};
mod resource;
pub use resource::{ComponentId, ItemResource, Overanswered, ResourceHandler, move_resources};
pub use transaction::{Transaction, TransactionSeal, run_transaction};
mod part_table;
pub use part_table::{
    Attached, BlockEntityProviders, BlockProvider, EntityProvider, ItemProvider, Parts, Registrar,
    block_entity_providers,
};
pub(crate) use part_table::{
    KnownIds, ProvidedBy, answered, ask_providers, install_parts, mark_game_thread, parts,
    place_provider, providers, resolve_parts,
};

mod generated {
    include!(concat!(env!("OUT_DIR"), "/mod_api_generated.rs"));
}
pub use generated::Hooks;
#[doc(hidden)]
pub use generated::*;

#[cfg(test)]
pub(crate) use part_table::tests as part_tests;

#[cfg(test)]
mod hooks_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn game_modes_round_trip_through_their_ids() {
        for mode in GameMode::ALL {
            assert_eq!(GameMode::from_id(mode.id()), Some(mode));
        }
        assert_eq!(GameMode::from_id(-1), None);
        assert_eq!(GameMode::from_id(4), None);
        assert_eq!(GameMode::Adventure.name(), "adventure");
    }

    struct Quiet;
    impl Hooks for Quiet {}

    struct Broken;
    impl Hooks for Broken {
        fn component_types(&self, _event: &mut ComponentTypesEvent<'_>) {
            panic!("a broken mod");
        }
    }

    /// A double with the `Host` methods the probe calls.
    struct Reader {
        block: &'static str,
        lines: Vec<Text>,
    }

    impl Host for Reader {
        fn level(&self, dimension: &str) -> Option<LevelRef> {
            (dimension == "minecraft:overworld")
                .then(|| LevelRef::new(NativeWorldEpoch::new(1), "minecraft:overworld"))
        }
        fn level_block(&self, _level: LevelRef, _pos: [i32; 3]) -> Option<String> {
            Some(self.block.to_string())
        }
        fn acting_player(&self) -> Option<NativeServerPlayer> {
            Some(NativeServerPlayer {
                uuid: 7,
                name: "Probe".to_string(),
                permission: 0,
                dimension: "minecraft:overworld".to_string(),
                position: [0.0; 3],
                game_mode: 0,
                health: 20.0,
                yaw: 0.0,
                pitch: 0.0,
            })
        }
        fn action_bar(&mut self, _uuid: u128, line: Text) -> bool {
            self.lines.push(line);
            true
        }
    }

    #[test]
    fn render_hand_chains_pose_until_cancelled() {
        struct HandMod {
            offset: f32,
            cancel: bool,
        }
        impl Hooks for HandMod {
            fn render_hand(&self, _host: &dyn Host, event: &mut RenderHandEvent<'_, '_>) -> Flow {
                assert_eq!((event.render.player, event.render.player_entity_id), (7, 9));
                event.render.translation[0] += self.offset;
                if self.cancel {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        let mods: Vec<Box<dyn Hooks>> = vec![
            Box::new(HandMod {
                offset: 1.0,
                cancel: false,
            }),
            Box::new(HandMod {
                offset: 2.0,
                cancel: true,
            }),
            Box::new(HandMod {
                offset: 4.0,
                cancel: false,
            }),
        ];
        install(mods.into());
        let host = Reader {
            block: "minecraft:air",
            lines: Vec::new(),
        };
        let mut render = NativeHandRender {
            player: 7,
            player_entity_id: 9,
            hand: Hand::Main,
            stack: None,
            partial: 0.5,
            interpolated_pitch: 0.0,
            swing: 0.0,
            equip: 0.0,
            packed_light: 0,
            translation: [0.0; 3],
            rotation: [0.0; 3],
            scale: 1.0,
        };
        let mut canceled = false;
        dispatch_render_hand(&host, &mut render, &mut canceled);
        assert!(canceled);
        assert_eq!(render.translation, [3.0, 0.0, 0.0]);
    }

    /// Claims a use on stone, after telling the player what the block is.
    struct Probe;

    impl Hooks for Probe {
        fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
            let player = host.acting_player().expect("a player acts");
            let level = host.level(&player.dimension).expect("a loaded level");
            let block = level.block(host, event.pos).expect("a loaded block");
            host.action_bar(player.uuid, Text::literal(block.clone()));
            if block == "minecraft:stone" {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    #[test]
    fn conversion_post_reports_both_entities_once() {
        struct Observer(Arc<Mutex<Vec<(u32, String, u32, String)>>>);
        impl Hooks for Observer {
            fn living_conversion_post(
                &self,
                _host: &mut dyn Host,
                event: &mut LivingConversionPostEvent<'_>,
            ) {
                self.0.lock().unwrap().push((
                    event.entity,
                    event.kind.to_owned(),
                    event.converted,
                    event.converted_kind.to_owned(),
                ));
            }
        }
        let observed = Arc::new(Mutex::new(Vec::new()));
        let mods = [Box::new(Observer(observed.clone())) as Box<dyn Hooks>];
        install(mods.into());
        let mut host = Reader {
            block: "minecraft:air",
            lines: Vec::new(),
        };
        dispatch_living_conversion_post(
            &mut host,
            12,
            "minecraft:pig",
            13,
            "minecraft:zombified_piglin",
        );
        assert_eq!(
            *observed.lock().unwrap(),
            [(
                12,
                "minecraft:pig".into(),
                13,
                "minecraft:zombified_piglin".into()
            )]
        );
    }

    #[test]
    fn xp_change_passes_identity_and_allows_signed_amount_or_cancellation() {
        struct XpHook(bool);
        impl Hooks for XpHook {
            fn player_xp_change(
                &self,
                _host: &mut dyn Host,
                event: &mut PlayerXpChangeEvent<'_>,
            ) -> Flow {
                assert_eq!(
                    (event.player, event.player_entity_id, event.name),
                    (7, 42, "Probe")
                );
                event.amount = -5;
                if self.0 { Flow::Cancel } else { Flow::Continue }
            }
        }
        let mut host = Reader {
            block: "minecraft:air",
            lines: Vec::new(),
        };
        for cancelled in [false, true] {
            let mods = [Box::new(XpHook(cancelled)) as Box<dyn Hooks>];
            install(mods.into());
            let (mut amount, mut cancel) = (10, false);
            dispatch_player_xp_change(&mut host, 7, 42, "Probe", &mut amount, &mut cancel);
            assert_eq!((amount, cancel), (-5, cancelled));
        }
    }

    struct MorePhantoms;

    impl Hooks for MorePhantoms {
        fn player_spawn_phantoms(
            &self,
            _host: &mut dyn Host,
            event: &mut PlayerSpawnPhantomsEvent<'_>,
        ) -> Option<PhantomSpawnResult> {
            *event.phantoms = event.phantoms.map(|count| count + 2);
            None
        }
    }

    #[test]
    fn phantom_listener_can_increase_the_default_count() {
        let mods = [Box::new(MorePhantoms) as Box<dyn Hooks>];
        install(mods.into());
        let mut host = Reader {
            block: "minecraft:air",
            lines: Vec::new(),
        };
        let mut count = Some(3);
        let mut result = None;
        dispatch_player_spawn_phantoms(&mut host, 7, "Probe", [0; 3], &mut count, &mut result);
        assert_eq!(count, Some(5));
        assert!(result.is_none());
    }

    /// Asks the host to tell a player without anyone having implemented it.
    struct Chatty;

    impl Hooks for Chatty {
        fn use_item_on(&self, host: &mut dyn Host, _event: &mut UseItemOnEvent<'_>) -> Flow {
            host.tell(7, Text::literal("hello"));
            Flow::Continue
        }
    }

    fn stick() -> ItemStack {
        ItemStack {
            item: Arc::from("minecraft:stick"),
            count: 1,
            damage: 0,
            extra: None,
        }
    }

    #[test]
    fn a_panic_in_a_hook_names_the_mod_and_the_hook() {
        name_mods(&["quiet", "broken"]);
        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(Quiet), Box::new(Broken)];
        install(mods.into());
        let caught = Arc::new(Mutex::new(Vec::new()));
        let sink = caught.clone();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let error = crate::crash::panic_report(info, "client").error;
            if error.contains("a broken mod") {
                sink.lock().unwrap().push(error);
            }
        }));
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            dispatch_component_types(&mut Vec::new())
        }));
        // Once the dispatch has unwound, the thread is in no mod.
        let _ = std::panic::catch_unwind(|| panic!("a broken mod's caller"));
        std::panic::set_hook(previous);
        // The game stops as before: the panic goes on to the dispatch's caller.
        assert!(unwound.is_err());
        let errors = caught.lock().unwrap();
        assert_eq!(errors.len(), 2);
        assert!(
            errors[0].ends_with("\nin native mod broken, hook component_types"),
            "{}",
            errors[0]
        );
        assert!(!errors[1].contains("in native mod"), "{}", errors[1]);
    }

    #[test]
    fn player_turn_mods_receive_identity_and_chain_changes() {
        struct Turn;
        impl Hooks for Turn {
            fn calculate_player_turn(
                &self,
                _host: &dyn Host,
                event: &mut CalculatePlayerTurnEvent,
            ) {
                assert_eq!((event.player_entity_id, event.player_uuid), (42, 123));
                event.mouse_sensitivity *= 2.0;
                event.cinematic_camera_enabled = !event.cinematic_camera_enabled;
            }
        }
        let host = Reader {
            block: "",
            lines: Vec::new(),
        };
        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(Turn), Box::new(Turn)];
        install(mods.into());
        let mut sensitivity = 0.25;
        let mut cinematic = false;
        dispatch_calculate_player_turn(&host, 42, 123, &mut sensitivity, &mut cinematic);
        assert_eq!(sensitivity, 1.0);
        assert!(!cinematic);
    }

    #[test]
    fn a_test_host_implements_only_what_its_hook_calls() {
        // Another test installs parts with tags: a dispatch must run on a game thread.
        mark_game_thread();
        let mods = [Box::new(Probe) as Box<dyn Hooks>];
        install(mods.into());
        let stack = stick();
        for (block, expected) in [("minecraft:stone", true), ("minecraft:dirt", false)] {
            let mut host = Reader {
                block,
                lines: Vec::new(),
            };
            let mut claimed = false;
            let (pos, face) = ([1, 2, 3], [0, 1, 0]);
            let hit = top_hit();
            dispatch_use_item_on(
                &mut host,
                &stack,
                pos,
                face,
                Hand::Main,
                hit,
                7,
                41,
                &mut claimed,
            );
            assert_eq!(claimed, expected, "{block}");
            let told: Vec<String> = host.lines.iter().map(Text::plain).collect();
            assert_eq!(told, [block], "{block}");
        }
    }

    struct Backstab;

    impl Hooks for Backstab {
        fn living_incoming_damage_v2(
            &self,
            host: &mut dyn Host,
            event: &mut LivingIncomingDamageV2Event<'_>,
        ) -> Flow {
            assert_eq!(event.causing, 7);
            assert_eq!(event.kind, "minecraft:zombie");
            assert_eq!(event.damage_type, "minecraft:player_attack");
            if event.entity == 0 {
                return Flow::Cancel;
            }
            let from = event.source_position.unwrap();
            let behind = (0..3)
                .map(|axis| (from[axis] - event.victim_position[axis]) * event.victim_view[axis])
                .sum::<f64>()
                < 0.0;
            if behind {
                let weapon = event.weapon.unwrap();
                let level = weapon.enchantment_level("probe:backstab");
                assert_eq!(weapon.enchantments().len(), 1);
                event.amount *= level as f32;
                let player = host.acting_player().unwrap();
                host.action_bar(player.uuid, Text::literal("backstab"));
            }
            Flow::Continue
        }
    }

    #[test]
    fn damage_v2_exposes_weapon_geometry_and_host_without_changing_v1() {
        let mods = [Box::new(Backstab) as Box<dyn Hooks>];
        install(mods.into());
        let mut weapon = stick();
        weapon.enchant("probe:backstab", 3);
        for (entity, source_z, expected, canceled_expected) in [
            (1, -1.0, 12.0, false),
            (1, 1.0, 4.0, false),
            (0, -1.0, 4.0, true),
        ] {
            let mut host = Reader {
                block: "minecraft:air",
                lines: Vec::new(),
            };
            let mut amount = 4.0;
            let mut canceled = false;
            dispatch_living_incoming_damage_v2(
                &mut host,
                entity,
                "minecraft:zombie",
                "minecraft:player_attack",
                7,
                Some([0.0, 0.0, source_z]),
                [0.0; 3],
                [0.0, 0.0, 1.0],
                Some(&weapon),
                &mut amount,
                &mut canceled,
            );
            assert_eq!((amount, canceled), (expected, canceled_expected));
            assert_eq!(host.lines.len(), usize::from(expected == 12.0));
        }
        let mut amount = 4.0;
        let mut canceled = false;
        dispatch_living_incoming_damage(
            1,
            "minecraft:zombie",
            "minecraft:player_attack",
            7,
            7,
            &mut amount,
            &mut canceled,
        );
        assert_eq!((amount, canceled), (4.0, false));
    }

    #[test]
    fn mining_events_identify_the_player_and_keep_mod_decisions() {
        struct MiningProbe(Arc<Mutex<Vec<u32>>>);
        impl Hooks for MiningProbe {
            fn player_harvest_check(
                &self,
                _host: &mut dyn Host,
                event: &mut PlayerHarvestCheckEvent,
            ) {
                self.0.lock().unwrap().push(event.player);
                event.can_harvest = false;
            }
            fn player_break_speed(
                &self,
                _host: &mut dyn Host,
                event: &mut PlayerBreakSpeedEvent,
            ) -> Flow {
                self.0.lock().unwrap().push(event.player);
                event.new_speed = 2.0;
                Flow::Continue
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = [Box::new(MiningProbe(seen.clone())) as Box<dyn Hooks>];
        install(mods.into());
        let mut host = Reader {
            block: "minecraft:stone",
            lines: Vec::new(),
        };
        let mut harvest = true;
        let mut speed = 1.0;
        let mut canceled = false;
        dispatch_player_harvest_check(&mut host, 42, [1, 2, 3], 7, &mut harvest);
        dispatch_player_break_speed(&mut host, 42, [1, 2, 3], 7, 1.0, &mut speed, &mut canceled);
        assert_eq!(*seen.lock().unwrap(), [42, 42]);
        assert!(!harvest);
        assert_eq!(speed, 2.0);
        assert!(!canceled);
    }

    #[test]
    #[should_panic(expected = "Host::tell")]
    fn an_unexpected_host_call_names_the_method() {
        // Another test installs parts with tags: a dispatch must run on a game thread.
        mark_game_thread();
        let mods = [Box::new(Chatty) as Box<dyn Hooks>];
        install(mods.into());
        let mut host = Reader {
            block: "minecraft:stone",
            lines: Vec::new(),
        };
        let mut claimed = false;
        let (pos, face) = ([0; 3], [0, 1, 0]);
        dispatch_use_item_on(
            &mut host,
            &stick(),
            pos,
            face,
            Hand::Main,
            top_hit(),
            7,
            41,
            &mut claimed,
        );
    }

    /// The top of the block at the origin, clicked in its middle.
    fn top_hit() -> BlockHit {
        BlockHit::new([0.5, 1.0, 0.5], [0, 1, 0], false)
    }

    /// Takes a click with the off hand on a block's top half.
    struct OffHandTop;

    impl Hooks for OffHandTop {
        fn use_item_on(&self, _: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
            assert_eq!((event.player, event.player_entity_id), (7, 41));
            assert_eq!((event.hand, event.hit), (Hand::Off, top_hit()));
            Flow::Continue
        }

        fn right_click_block(&self, _: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
            let click = event.click;
            let top_half = click.hit.point[1] - f64::from(click.pos[1]) >= 0.5;
            if click.hand == Hand::Off
                && top_half
                && click.player == 7
                && click.player_entity_id == 41
                && !click.hit.inside
            {
                *event.cancellation_result = NativeInteractionResult::Fail;
                Flow::Cancel
            } else {
                // A held stick never uses the block; an empty hand always does.
                *event.use_block = if click.stack.is_some() {
                    NativeTriState::False
                } else {
                    NativeTriState::True
                };
                Flow::Continue
            }
        }
    }

    #[test]
    fn a_block_use_hears_the_hand_the_hit_and_the_player() {
        mark_game_thread();
        let mods = [Box::new(OffHandTop) as Box<dyn Hooks>];
        install(mods.into());
        let mut host = Reader {
            block: "minecraft:stone",
            lines: Vec::new(),
        };
        let mut claimed = false;
        dispatch_use_item_on(
            &mut host,
            &stick(),
            [0; 3],
            [0, 1, 0],
            Hand::Off,
            top_hit(),
            7,
            41,
            &mut claimed,
        );
        assert!(!claimed);
        let low = BlockHit::new([0.5, 0.25, 0.0], [0, 0, -1], false);
        let held = stick();
        for (hand, hit, stack, canceled, answer, use_block) in [
            (
                Hand::Off,
                top_hit(),
                None,
                true,
                NativeInteractionResult::Fail,
                NativeTriState::Default,
            ),
            (
                Hand::Main,
                top_hit(),
                Some(&held),
                false,
                NativeInteractionResult::Pass,
                NativeTriState::False,
            ),
            (
                Hand::Off,
                low,
                None,
                false,
                NativeInteractionResult::Pass,
                NativeTriState::True,
            ),
        ] {
            let click = NativeBlockClick {
                side: LogicalSide::Server,
                dimension: LevelDimension::Overworld,
                pos: [0; 3],
                hit,
                hand,
                stack,
                player: 7,
                player_entity_id: 41,
            };
            let (mut block, mut item) = (NativeTriState::Default, NativeTriState::Default);
            let (mut result, mut was_canceled) = (NativeInteractionResult::Pass, false);
            dispatch_right_click_block(
                &mut host,
                &click,
                &mut block,
                &mut item,
                &mut result,
                &mut was_canceled,
            );
            assert_eq!(
                (was_canceled, result, block, item),
                (canceled, answer, use_block, NativeTriState::Default),
                "{hand:?} {hit:?}"
            );
        }
    }

    /// Cancels every brew and leaves its own slots; records what it saw.
    struct BrewSwapper(Arc<Mutex<Vec<String>>>);

    impl Hooks for BrewSwapper {
        fn potion_brew_pre(
            &self,
            _host: &mut dyn Host,
            event: &mut PotionBrewPreEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push("pre".into());
            event.stacks[0] = Some(ItemStack::new("minecraft:diamond", 1));
            Flow::Cancel
        }
        fn potion_brew_post(&self, _host: &mut dyn Host, event: &mut PotionBrewPostEvent<'_>) {
            let first = event.stacks[0].as_ref().map_or("", |s| &s.item);
            self.0.lock().unwrap().push(format!("post {first}"));
            event.stacks[1] = Some(ItemStack::new("minecraft:emerald", 1));
        }
    }

    /// Edits the slots in `PotionBrewEvent.Pre` without cancelling.
    struct BrewEditor;

    impl Hooks for BrewEditor {
        fn potion_brew_pre(
            &self,
            _host: &mut dyn Host,
            event: &mut PotionBrewPreEvent<'_>,
        ) -> Flow {
            event.stacks[0] = Some(ItemStack::new("minecraft:diamond", 1));
            Flow::Continue
        }
        fn potion_brew_post(&self, _host: &mut dyn Host, event: &mut PotionBrewPostEvent<'_>) {
            event.stacks[1] = Some(ItemStack::new("minecraft:emerald", 1));
        }
    }

    /// `PotionBrewEvent.Pre` and `.Post` around `doBrew`: a cancelling mod's
    /// slots replace the brew, with no ingredient used, and Post fires once
    /// with them; a mod that doesn't cancel has its Pre edits dropped,
    /// vanilla brews, and its Post edits stay.
    #[test]
    fn a_cancelled_brew_takes_the_mods_slots_and_an_uncancelled_one_brews() {
        use crate::brewing::{Brewing, BrewingRecipe, BrewingStand, PotionIngredient};
        let set = |ids: &[&str]| ids.iter().map(|s| (*s).to_owned()).collect();
        let brewing = Brewing {
            reagents: set(&["minecraft:nether_wart"]),
            inputs: Default::default(),
            recipes: vec![BrewingRecipe {
                id: "awkward".into(),
                input: PotionIngredient {
                    items: set(&["minecraft:potion"]),
                    potions: Some(set(&["minecraft:water"])),
                },
                reagent: PotionIngredient {
                    items: set(&["minecraft:nether_wart"]),
                    potions: None,
                },
                output: ItemStack::new("minecraft:potion", 1).with_potion("minecraft:awkward"),
            }],
        };
        let items = crate::items::Items::for_tests(Default::default());
        let water = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water");
        let awkward = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:awkward");
        let fresh = || {
            let mut stand = BrewingStand::new();
            stand.items[0] = Some(water.clone());
            stand.items[3] = Some(ItemStack::new("minecraft:nether_wart", 2));
            stand
        };
        let mut host = Reader {
            block: "minecraft:brewing_stand",
            lines: Vec::new(),
        };
        // What `tick_brewing_stand` does once the brew is due.
        let brew = |host: &mut Reader, stand: &mut BrewingStand| {
            let mut stacks = stand.items.clone();
            let mut cancel = false;
            dispatch_potion_brew_pre(host, [1, 2, 3], &mut stacks, &mut cancel);
            let mut out = crate::brewing::BrewTick::default();
            stand.brew_after_pre(Some((stacks, cancel)), &brewing, &items, &mut out);
            if out.brewed || cancel {
                dispatch_potion_brew_post(host, [1, 2, 3], &mut stand.items);
            }
            out.brewed
        };

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(BrewSwapper(seen.clone()))];
        install(mods.into());
        let mut stand = fresh();
        assert!(!brew(&mut host, &mut stand));
        assert_eq!(*seen.lock().unwrap(), ["pre", "post minecraft:diamond"]);
        assert_eq!(&*stand.items[0].as_ref().unwrap().item, "minecraft:diamond");
        assert_eq!(&*stand.items[1].as_ref().unwrap().item, "minecraft:emerald");
        assert_eq!(stand.items[3].as_ref().map(|s| s.count), Some(2));

        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(BrewEditor)];
        install(mods.into());
        let mut stand = fresh();
        assert!(brew(&mut host, &mut stand));
        assert_eq!(stand.items[0], Some(awkward));
        assert_eq!(&*stand.items[1].as_ref().unwrap().item, "minecraft:emerald");
        assert_eq!(stand.items[3].as_ref().map(|s| s.count), Some(1));
    }

    /// Writes down every game rule change and level save it hears.
    struct Recorder(Arc<Mutex<Vec<String>>>);

    impl Hooks for Recorder {
        fn game_rule_changed(&self, _host: &mut dyn Host, event: &mut GameRuleChangedEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("{} {}", event.rule, event.value));
        }
        fn level_save(&self, _host: &mut dyn Host, event: &mut LevelSaveEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("saved {}", event.dimension));
        }
    }

    #[test]
    fn game_rule_change_reports_optional_actor() {
        struct ActorRecorder(Arc<Mutex<Vec<Option<(u128, i32, String)>>>>);
        impl Hooks for ActorRecorder {
            fn game_rule_changed(&self, _: &mut dyn Host, event: &mut GameRuleChangedEvent<'_>) {
                self.0.lock().unwrap().push(
                    event
                        .actor
                        .map(|actor| (actor.uuid, actor.entity_id, actor.name.to_owned())),
                );
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(ActorRecorder(seen.clone()))];
        install(mods.into());
        let mut host = Reader {
            block: "minecraft:air",
            lines: Vec::new(),
        };
        let actor = GameRuleActor {
            uuid: 123,
            entity_id: 42,
            name: "Alex",
        };
        dispatch_game_rule_changed(
            &mut host,
            "minecraft:keep_inventory",
            GameRuleValue::Bool(true),
            Some(&actor),
        );
        dispatch_game_rule_changed(
            &mut host,
            "minecraft:keep_inventory",
            GameRuleValue::Bool(false),
            None,
        );
        assert_eq!(
            *seen.lock().unwrap(),
            [Some((123, 42, "Alex".into())), None]
        );
    }

    /// `GameRuleChangedEvent` as the server's setters fire it: only for a
    /// rule `GameRules.set` took, with its id and new value, to every mod.
    #[test]
    fn a_game_rule_change_reaches_every_mod_once_the_rule_is_set() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: Vec<Box<dyn Hooks>> = vec![
            Box::new(Recorder(seen.clone())),
            Box::new(Recorder(seen.clone())),
        ];
        install(mods.into());
        let mut host = Reader {
            block: "minecraft:air",
            lines: Vec::new(),
        };
        let mut rules = crate::gamerules::GameRules::default();
        // What `set_game_rule` and the other setters do.
        let mut set = |name: &str, value: GameRuleValue| {
            if rules.set(name, value) {
                dispatch_game_rule_changed(&mut host, &["minecraft:", name].concat(), value, None);
            }
        };
        set("keep_inventory", GameRuleValue::Bool(true));
        // Out of range, of the wrong type, unknown: GameRules.set refuses them.
        set("random_tick_speed", GameRuleValue::Int(-1));
        set("keep_inventory", GameRuleValue::Int(1));
        set("no_such_rule", GameRuleValue::Bool(true));
        set("random_tick_speed", GameRuleValue::Int(7));
        assert_eq!(
            *seen.lock().unwrap(),
            [
                "minecraft:keep_inventory true",
                "minecraft:keep_inventory true",
                "minecraft:random_tick_speed 7",
                "minecraft:random_tick_speed 7",
            ]
        );
    }

    /// `LevelEvent.Save` as a world save fires it: once for each level, in
    /// the order they are saved, to every mod.
    #[test]
    fn each_saved_level_reaches_every_mod() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: Vec<Box<dyn Hooks>> = vec![
            Box::new(Recorder(seen.clone())),
            Box::new(Recorder(seen.clone())),
        ];
        install(mods.into());
        let mut host = Reader {
            block: "minecraft:air",
            lines: Vec::new(),
        };
        for dimension in ["minecraft:the_nether", "minecraft:overworld"] {
            dispatch_level_save(&mut host, dimension);
        }
        assert_eq!(
            *seen.lock().unwrap(),
            [
                "saved minecraft:the_nether",
                "saved minecraft:the_nether",
                "saved minecraft:overworld",
                "saved minecraft:overworld",
            ]
        );
    }

    /// Keeps the cod: reads the catch, sets the rod's damage, and cancels a
    /// catch with cod in it.
    struct CodKeeper;

    impl Hooks for CodKeeper {
        fn item_fished(&self, _host: &mut dyn Host, event: &mut ItemFishedEvent<'_>) -> Flow {
            event.rod_damage = 4;
            let cod = event
                .fished
                .drops
                .iter()
                .any(|s| &*s.item == "minecraft:cod");
            if cod { Flow::Cancel } else { Flow::Continue }
        }
    }

    /// Fails the test if a mod after a cancel is asked.
    struct Unreached;

    impl Hooks for Unreached {
        fn item_fished(&self, _host: &mut dyn Host, _event: &mut ItemFishedEvent<'_>) -> Flow {
            panic!("asked after a cancel");
        }
    }

    /// `ItemFishedEvent`: a mod reads the catch and sets the rod's damage,
    /// which stays whether or not it cancels; a cancel stops later mods.
    #[test]
    fn a_fishing_catch_keeps_the_mods_rod_damage_and_its_cancel() {
        let mut host = Reader {
            block: "minecraft:water",
            lines: Vec::new(),
        };
        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(CodKeeper), Box::new(Unreached)];
        install(mods.into());
        let ask = |host: &mut Reader, item: &str| {
            let drops = [ItemStack::new(item, 1)];
            let fished = NativeItemFished {
                hook: 9,
                position: [0.5, 62.0, 0.5],
                drops: &drops,
            };
            let (mut rod_damage, mut cancel) = (1, false);
            dispatch_item_fished(host, 7, 3, &fished, &mut rod_damage, &mut cancel);
            (rod_damage, cancel)
        };
        assert_eq!(ask(&mut host, "minecraft:cod"), (4, true));
        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(CodKeeper)];
        install(mods.into());
        let drops = [ItemStack::new("minecraft:salmon", 1)];
        let fished = NativeItemFished {
            hook: 9,
            position: [0.5, 62.0, 0.5],
            drops: &drops,
        };
        let (mut rod_damage, mut cancel) = (2, false);
        dispatch_item_fished(&mut host, 7, 3, &fished, &mut rod_damage, &mut cancel);
        assert_eq!((rod_damage, cancel), (4, false));
    }

    /// Records what `player_brewed_potion` told it.
    struct BrewWatcher(Arc<Mutex<Vec<(u128, i32, ItemStack)>>>);

    impl Hooks for BrewWatcher {
        fn player_brewed_potion(
            &self,
            _host: &mut dyn Host,
            event: &mut PlayerBrewedPotionEvent<'_>,
        ) {
            let seen = (event.player, event.player_entity_id, event.stack.clone());
            self.0.lock().unwrap().push(seen);
        }
    }

    /// `BrewingStandMenu.PotionSlot.onTake`: a pickup and a shift-click on a
    /// bottle slot each tell a listening mod once, with the taken stack and
    /// the player's UUID and entity id; with no listener nothing is kept.
    #[test]
    fn a_bottle_slot_take_tells_the_listener_once_with_the_player() {
        use crate::menu::{Env, Input, Kind, LogicalSide, Menu};
        let root = std::env::temp_dir().join(format!("horizon-brewed-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        let items = crate::items::Items::for_tests(Default::default());
        let overlay = &horizon_world::loot::PackOverlay::NONE;
        let recipes = crate::crafting::Recipes::load(&root, &items, overlay).unwrap();
        let brewing = crate::brewing::Brewing::load(&root, &items, overlay);
        let _ = std::fs::remove_dir_all(&root);
        let (banners, station_recipes) = (Default::default(), Default::default());
        let (enchantments, names) = (Default::default(), Default::default());
        let env = &Env {
            player: Some(7),
            player_entity_id: Some(41),
            side: LogicalSide::Server,
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
        };
        let potion = ItemStack::new("minecraft:potion", 1);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(BrewWatcher(seen.clone()))];
        install(mods.into());
        let mut host = Reader {
            block: "minecraft:air",
            lines: Vec::new(),
        };
        for input in [Input::Pickup, Input::QuickMove] {
            let mut menu = Menu::new(Kind::Brewing);
            menu.set_slot(env, 0, Some(potion.clone()));
            menu.click(env, 0, 0, input).unwrap();
            // Without a listener the take is only the advancement's.
            assert!(menu.brewed_potion_takes.is_empty(), "{input:?}");
            assert_eq!(menu.brewed, std::slice::from_ref(&potion), "{input:?}");

            let mut menu = Menu::new(Kind::Brewing);
            menu.brewed_potion_hook = true;
            menu.set_slot(env, 0, Some(potion.clone()));
            menu.click(env, 0, 0, input).unwrap();
            assert_eq!(
                menu.brewed_potion_takes,
                std::slice::from_ref(&potion),
                "{input:?}"
            );
            for stack in &menu.brewed_potion_takes {
                dispatch_player_brewed_potion(&mut host, 7, 41, stack);
            }
        }
        let told = (7, 41, potion);
        assert_eq!(*seen.lock().unwrap(), [told.clone(), told]);
    }
}
