//! Trait mods implement `Hooks` and link into the game executable.
//! The executable passes the ordered `Mods` to `entry::main_with`.
//! Trait mods are process-wide, like mixin module statics.
//! Hooks use `&self + Send + Sync` for shared and reentrant calls; see `docs/native-mods.md`.
//! Generate the trait and dispatch with `cargo run -p xtask -- mod-api`.
use std::sync::{Arc, OnceLock};

mod file_to_id_converter;
pub use file_to_id_converter::{FileToIdConverter, FileToIdConverterError};
mod reload_order;
pub(crate) mod server_reload;
pub use horizon_world::identifier::Identifier;
pub use reload_order::ReloadOrderError;
pub use server_reload::{
    ServerReloadApply, ServerReloadContext, ServerReloadError, ServerReloadListener,
    ServerReloadListeners, VanillaServerListeners,
};

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
    NativeBlockEdit, NativeBlockRun, NativeClock, NativeColumnBlocks, NativeConfig,
    NativeConfigFile, NativeConfigKind, NativeDataMapType, NativeExperienceOrb, NativeFalling,
    NativeHeightmap, NativeHeldItem, NativeHit, NativeMapDecoration, NativeMapInfo,
    NativeMobRender, NativeModEvent, NativePlayerRender, NativeShake, PhantomSpawnResult,
    SleepProblem,
};
pub use crate::gamerules::Value as GameRuleValue;
pub use crate::gfx::native_render::{
    BlockSpriteArrays, RenderHookId, RenderHooks, RenderNeeds, RenderNeedsEvent, WorldDrawn,
    WorldDrawnEvent, render_hook_ids, wgpu,
};
pub use crate::health::{HurtSource, ShieldBlock};
pub use crate::hud::HudBatch;
pub use crate::hud_layout::{GuiDraw, HealthInputs};
pub use crate::items::ItemStack;
pub use crate::menu::{LogicalSide, StackedOnOther};
pub use crate::merchant::MerchantOffer;
pub use crate::mesher::TintSource;
pub use crate::mod_chunk_tickets::TicketOwner as NativeTicketOwner;
pub use crate::mod_chunk_tickets::{TicketSet, TicketValidation};
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

/// A hook's mutable block entity payload, with a read-only type identity.
/// The private handle cannot be constructed or taken out by a mod.
///
/// ```compile_fail
/// use horizon_client::mod_api::{BlockEntityTickEvent, ModBlockEntity};
/// fn replace(event: &mut BlockEntityTickEvent<'_>) {
///     *event.entity = ModBlockEntity::default();
/// }
/// ```
///
/// ```compile_fail
/// use horizon_client::mod_api::BlockEntityTickEvent;
/// fn take(event: &mut BlockEntityTickEvent<'_>) {
///     let _ = std::mem::take(&mut event.entity);
/// }
/// ```
///
/// ```compile_fail
/// use horizon_client::mod_api::{BlockEntityTickEvent, ModBlockEntity};
/// fn access(event: &mut BlockEntityTickEvent<'_>) {
///     let _: &mut ModBlockEntity = event.entity.entity;
/// }
/// ```
pub struct ModBlockEntityView<'a> {
    entity: &'a mut ModBlockEntity,
}
impl<'a> ModBlockEntityView<'a> {
    pub(crate) fn new(entity: &'a mut ModBlockEntity) -> Self {
        Self { entity }
    }
    pub fn id(&self) -> &str {
        self.entity.id()
    }
}
impl std::ops::Deref for ModBlockEntityView<'_> {
    type Target = crate::containers::ModBlockEntityFields;
    fn deref(&self) -> &Self::Target {
        self.entity
    }
}
impl std::ops::DerefMut for ModBlockEntityView<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.entity
    }
}

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
    /// dropped them. A mob that a player killed while
    /// leashed drops its lead on its next tick as a call of its own (`entity`
    /// the mob, `player` its killer, `table` empty, `used` false).
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
#[non_exhaustive]
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
    /// `slots`. The title's own screens report theirs: `title`,
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
    /// Health points, 20 at full; 0 once dead.
    pub health: f32,
    /// Where they look, in degrees (`yRot`, `xRot`), as their last move told
    /// the server.
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

/// One server world open or client level; it rejects a previous world's handles.
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
/// are shared by every server level and never reused while the server world is
/// open, and an entity that changes dimension gets a new one, as in vanilla.
/// On a client the epoch identifies one client level and changes when that level
/// is replaced. Server reads and writes reject a handle from an earlier epoch,
/// of an entity that is gone, or used on a client. `Host::entity` makes a server
/// handle from an event's entity id; `ClientHost::crosshair_target` supplies a
/// client handle for an entity hit.
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

/// The ender dragon of the open world, narrowed with `host.dragon(entity)`.
/// It is also a [`MobRef`] and a [`LivingRef`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DragonRef(EntityRef);

impl DragonRef {
    /// Create a handle for the game.
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
    /// The same entity as a mob handle.
    pub fn mob(self) -> MobRef {
        MobRef::new(self.0)
    }
}

/// An ender dragon phase (`EnderDragonPhase`), in vanilla's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NativeDragonPhase {
    HoldingPattern,
    StrafePlayer,
    LandingApproach,
    Landing,
    Takeoff,
    SittingFlaming,
    SittingScanning,
    SittingAttacking,
    ChargingPlayer,
    Dying,
    Hover,
}

impl NativeDragonPhase {
    const ALL: [Self; 11] = [
        Self::HoldingPattern,
        Self::StrafePlayer,
        Self::LandingApproach,
        Self::Landing,
        Self::Takeoff,
        Self::SittingFlaming,
        Self::SittingScanning,
        Self::SittingAttacking,
        Self::ChargingPlayer,
        Self::Dying,
        Self::Hover,
    ];
    /// The phase's vanilla id, as the `DragonPhase` tag and the synced data
    /// carry it.
    pub fn id(self) -> i32 {
        self as i32
    }
    /// The phase with a vanilla id (`EnderDragonPhase.getById`).
    pub fn from_id(id: i32) -> Option<Self> {
        usize::try_from(id)
            .ok()
            .and_then(|i| Self::ALL.get(i))
            .copied()
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

/// A display entity of the open world (`block_display`, `item_display` or
/// `text_display`), narrowed with `host.display`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DisplayRef(EntityRef);

impl DisplayRef {
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

/// A projectile of the open world (vanilla's `Projectile`), narrowed with
/// `host.projectile`: arrows, tridents (a player's too), firework rockets,
/// thrown items, fireballs, wind charges, skulls, shulker bullets and mods'
/// thrown types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProjectileRef(EntityRef);

impl ProjectileRef {
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

/// Which `Display` subclass an entity is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NativeDisplayKind {
    Block,
    Item,
    Text,
}

impl NativeDisplayKind {
    /// The entity type id: `minecraft:block_display`, `minecraft:item_display` or `minecraft:text_display`.
    pub fn entity_type(self) -> &'static str {
        match self {
            Self::Block => "minecraft:block_display",
            Self::Item => "minecraft:item_display",
            Self::Text => "minecraft:text_display",
        }
    }
    /// The kind of an entity type id, or None for another entity type.
    pub fn from_entity_type(id: &str) -> Option<Self> {
        [Self::Block, Self::Item, Self::Text]
            .into_iter()
            .find(|kind| kind.entity_type() == id)
    }
}

/// `Display.BillboardConstraints`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NativeBillboard {
    Fixed,
    Vertical,
    Horizontal,
    Center,
}

impl NativeBillboard {
    /// Every constraint, in ordinal order.
    pub const ALL: [Self; 4] = [Self::Fixed, Self::Vertical, Self::Horizontal, Self::Center];
    /// `getSerializedName`: `fixed`, `vertical`, `horizontal` or `center`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
            Self::Center => "center",
        }
    }
    /// The constraint with that name, or None for one vanilla doesn't have.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.name() == name)
    }
}

/// `ItemDisplayContext`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NativeItemDisplayContext {
    None,
    ThirdPersonLeftHand,
    ThirdPersonRightHand,
    FirstPersonLeftHand,
    FirstPersonRightHand,
    Head,
    Gui,
    Ground,
    Fixed,
    OnShelf,
}

impl NativeItemDisplayContext {
    /// Every context, in ordinal order (vanilla's id is the index).
    pub const ALL: [Self; 10] = [
        Self::None,
        Self::ThirdPersonLeftHand,
        Self::ThirdPersonRightHand,
        Self::FirstPersonLeftHand,
        Self::FirstPersonRightHand,
        Self::Head,
        Self::Gui,
        Self::Ground,
        Self::Fixed,
        Self::OnShelf,
    ];
    /// `getSerializedName`: `none`, `thirdperson_lefthand`, ... `on_shelf`.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ThirdPersonLeftHand => "thirdperson_lefthand",
            Self::ThirdPersonRightHand => "thirdperson_righthand",
            Self::FirstPersonLeftHand => "firstperson_lefthand",
            Self::FirstPersonRightHand => "firstperson_righthand",
            Self::Head => "head",
            Self::Gui => "gui",
            Self::Ground => "ground",
            Self::Fixed => "fixed",
            Self::OnShelf => "on_shelf",
        }
    }
    /// The context with that name, or None for one vanilla doesn't have.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// `Display.TextDisplay.Align`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NativeTextAlignment {
    Center,
    Left,
    Right,
}

impl NativeTextAlignment {
    /// `getSerializedName`: `center`, `left` or `right`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// A display's `Transformation`: a translation, a left rotation, a scale and a
/// right rotation. Quaternions are `[x, y, z, w]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeTransformation {
    pub translation: [f32; 3],
    pub left_rotation: [f32; 4],
    pub scale: [f32; 3],
    pub right_rotation: [f32; 4],
}

impl NativeTransformation {
    /// `Transformation.IDENTITY`.
    pub const IDENTITY: Self = Self {
        translation: [0.0; 3],
        left_rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
        right_rotation: [0.0, 0.0, 0.0, 1.0],
    };
}

/// `ProjectileDeflection`: how `ProjectileRef::deflect` turns a projectile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeDeflection {
    /// `REVERSE`: backwards at `-power / 2` of the speed, turned about 180
    /// degrees by a random amount.
    Reverse,
    /// `AIM_DEFLECT`: along the deflecting entity's look.
    AimDeflect,
    /// `MOMENTUM_DEFLECT`: along the deflecting entity's movement.
    MomentumDeflect,
}

/// `AbstractArrow.Pickup`: who may pick an arrow up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeArrowPickup {
    Disallowed,
    Allowed,
    CreativeOnly,
}

/// A vanilla `AbstractMinecart`, narrowed from an entity with `host.minecart(entity)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MinecartRef(EntityRef);

impl MinecartRef {
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

/// Which minecart an entity is: one per vanilla `AbstractMinecart` subclass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NativeMinecartKind {
    /// `Minecart`.
    Rideable,
    /// `MinecartChest`.
    Chest,
    /// `MinecartFurnace`.
    Furnace,
    /// `MinecartTNT`.
    Tnt,
    /// `MinecartHopper`.
    Hopper,
    /// `MinecartCommandBlock`.
    CommandBlock,
    /// `MinecartSpawner`.
    Spawner,
}

/// A block in a dimension, vanilla's `GlobalPos`: the dimension's key
/// (`minecraft:the_nether`) and the block. The dimension is data, so it need
/// not be open; `host.level(&pos.dimension().to_string())` gives its level when it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalPos {
    dimension: Identifier,
    pos: BlockPos,
}

impl GlobalPos {
    /// A block in the dimension with key `dimension`.
    pub fn new(dimension: Identifier, pos: BlockPos) -> Self {
        Self { dimension, pos }
    }
    /// The dimension's key (`minecraft:overworld`).
    pub fn dimension(&self) -> &Identifier {
        &self.dimension
    }
    /// The block.
    pub fn pos(&self) -> BlockPos {
        self.pos
    }
}

impl std::hash::Hash for GlobalPos {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(
            &(self.dimension.namespace(), self.dimension.path(), self.pos),
            state,
        );
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

/// A raid of a level (`Raid`), by the id `Raids` keeps it under, which the
/// raids file saves. `level.raid_at(host, pos)`, `level.raids(host)`,
/// `level.raid(host, id)` and `mob.raid(host)` make one. A handle resolves
/// again on each call: it reads as `None` or `false` once the raid is gone,
/// for a handle of an earlier open, for a level that is not in place, and on a
/// client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RaidRef {
    level: LevelRef,
    id: i32,
}

impl RaidRef {
    /// Create a handle for the game.
    #[doc(hidden)]
    pub fn new(level: LevelRef, id: i32) -> Self {
        Self { level, id }
    }
    /// The level the raid belongs to.
    pub fn level(self) -> LevelRef {
        self.level
    }
    /// The raid's id in its level (the `raid` argument of the raid hooks).
    pub fn id(self) -> i32 {
        self.id
    }
}

/// Where a raid stands (`Raid.RaidStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NativeRaidStatus {
    /// Waiting or fighting (`ONGOING`).
    Ongoing,
    /// Every wave was beaten (`VICTORY`).
    Victory,
    /// No villager is left or the time ran out (`LOSS`).
    Loss,
    /// Ended by `stop`, the `/raid stop` command or the `raids` rule (`STOPPED`).
    Stopped,
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
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
pub enum NativeBlockShapeKind {
    /// Selection outline (`BlockState.getShape`).
    Outline,
    /// Collision shape (`BlockState.getCollisionShape`).
    Collision,
}

/// `ChatVisiblity`: which chat a client shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeChatVisibility {
    Full,
    System,
    Hidden,
}

/// `PermissionLevel`: how much a player may do, as `op-permission-level` and
/// the operator list name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
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
#[non_exhaustive]
pub enum NativeMainHand {
    Left,
    Right,
}

/// `ParticleStatus`: how many particles a client shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
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
#[non_exhaustive]
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

/// The client's ordinary crosshair hit (`Minecraft.hitResult`), within the
/// current interaction ranges. Entity handles belong to the current client
/// level's epoch, including dragon parts; a replacement level invalidates them.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum NativeCrosshairTarget {
    Block {
        pos: BlockPos,
        state: BlockStateId,
        hit: BlockHit,
    },
    Entity {
        entity: EntityRef,
        point: [f64; 3],
    },
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
/// A nested guest turn cannot re-enter a player active in an outer turn;
/// operations requiring that player's swapped state report them unavailable.
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
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
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

/// One native mod, with its bundle's `[mod] id` and hook implementation.
pub struct ModEntry {
    id: &'static str,
    pub(crate) hooks: Box<dyn Hooks>,
}

impl ModEntry {
    /// Bind a hook implementation to its nonempty bundle id.
    pub fn new(id: &'static str, hooks: Box<dyn Hooks>) -> Self {
        assert!(!id.is_empty(), "native mod id must not be empty");
        Self { id, hooks }
    }
}

/// The ordered list of named mods shared by the client and server.
pub type Mods = Arc<[ModEntry]>;

pub(crate) struct Installed {
    mods: Mods,
    subscribers: horizon_client_core::subscribers::Subscribers,
}

impl Installed {
    fn new(mods: Mods) -> Self {
        let subscriptions = mods.iter().enumerate().flat_map(|(index, entry)| {
            entry
                .hooks
                .subscribed()
                .iter()
                .flatten()
                .map(move |hook| (index, *hook as usize))
        });
        let subscribers = horizon_client_core::subscribers::Subscribers::new(
            generated::HOOK_COUNT,
            mods.len(),
            subscriptions,
        );
        Self { mods, subscribers }
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
    use super::ticket_controller::TEST_CONTROLLERS;
    use super::{Installed, Mods, TEST_MODS, TicketController};

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
        let controllers: &'static [_] =
            Box::leak(super::part_table::registered_controllers(&mods).into_boxed_slice());
        let installed = Box::leak(Box::new(Installed::new(mods)));
        let _restore = Restore(TEST_MODS.replace(Some(installed)));
        let _controllers = RestoreControllers(TEST_CONTROLLERS.replace(Some(controllers)));
        f()
    }

    /// The ticket controller with this id that the mods of the enclosing `with_mods` registered.
    pub fn ticket_controller(id: &str) -> Option<TicketController> {
        TicketController::from_id(id)
    }

    struct RestoreControllers(Option<&'static [super::ticket_controller::ControllerInfo]>);
    impl Drop for RestoreControllers {
        fn drop(&mut self) {
            TEST_CONTROLLERS.set(self.0);
        }
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
pub fn mods() -> &'static [ModEntry] {
    installed().map_or(&[], |installed| &installed.mods)
}

/// Whether a mod with this `[mod] id` is installed.
pub fn is_loaded(id: &str) -> bool {
    mods().iter().any(|entry| entry.id == id)
}

pub(crate) fn listened(hook: generated::HookId) -> bool {
    installed().is_some_and(|installed| installed.subscribers.contains_any(hook as usize))
}

/// Whether an installed mod subscribes to an injected hook.
#[doc(hidden)]
pub fn subscribed_by(index: usize, hook: Option<generated::HookId>) -> bool {
    hook.is_some_and(|hook| {
        installed().is_some_and(|installed| {
            index < installed.mods.len() && installed.subscribers.contains(hook as usize, index)
        })
    })
}

/// `mods` in order, each marked as running `hook` until the next one is (or the loop ends), so a
/// panic in it names the mod. The generated dispatch loops over it.
fn entered_registration<'a>(mods: &'a [ModEntry], hook: &'static str) -> Entered<'a> {
    Entered {
        mods: mods.iter(),
        hook,
        guard: None,
    }
}

fn entered(
    hook_id: generated::HookId,
    hook: &'static str,
) -> impl Iterator<Item = &'static dyn Hooks> {
    installed().into_iter().flat_map(move |installed| {
        let mut crash_guard = None;
        installed
            .subscribers
            .indices(hook_id as usize)
            .map(move |index| {
                drop(crash_guard.take());
                let entry = &installed.mods[index];
                let hooks = &*entry.hooks;
                crash_guard = Some(mark_mod(entry.id, hook));
                hooks
            })
    })
}

fn mark_mod(id: &'static str, hook: &'static str) -> crate::crash::NativeModGuard {
    crate::crash::enter_native_mod(crate::crash::NativeModCall {
        id,
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
                id: attached.mod_id,
                hook,
                part: Some(attached.type_name),
            },
        ));
        &*attached.part
    })
}

struct Entered<'a> {
    mods: std::slice::Iter<'a, ModEntry>,
    hook: &'static str,
    guard: Option<crate::crash::NativeModGuard>,
}

impl<'a> Iterator for Entered<'a> {
    type Item = &'a dyn Hooks;

    fn next(&mut self) -> Option<Self::Item> {
        // The last mod's mark goes first, so the next one's puts back what was there before.
        drop(self.guard.take());
        let entry = self.mods.next()?;
        self.guard = Some(mark_mod(entry.id, self.hook));
        Some(&*entry.hooks)
    }
}

/// Whether a mod claims a cancellable hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
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

/// The game's read-only native backend, as [`Host::backend`] hands it out.
///
/// The backend is reached only through [`Host::backend`] and
/// `Host::backend_mut`, which return opaque `BackendRef`/`BackendMutRef`
/// values that only `mod_api` can build or open. A foreign [`Host`] gets
/// `None` by default, and forwarding a real host's value adds nothing beyond
/// calling with the real host. A handle asked on a host without a backend
/// answers its typed refusal (`None` or `false`).
///
/// Opening one is reserved for this API:
///
/// ```compile_fail
/// use horizon_client::mod_api::{Backend, BackendRef};
/// fn reach(backend: BackendRef<'_>) { let _: &dyn Backend = backend.open(); }
/// ```
///
/// So is reading the backend it holds:
///
/// ```compile_fail
/// use horizon_client::mod_api::BackendRef;
/// fn reach(backend: BackendRef<'_>) { let _ = backend.inner; }
/// ```
///
/// And building one:
///
/// ```compile_fail
/// use horizon_client::mod_api::BackendRef;
/// fn build() { let _ = BackendRef::of; }
/// ```
#[doc(hidden)]
pub struct BackendRef<'a> {
    inner: &'a dyn Backend,
}

/// The game's mutable native backend, as `Host::backend_mut` hands it out;
/// the same opaque rule as [`BackendRef`], and a wrapper exposes no backend
/// method of its own:
///
/// ```compile_fail
/// use horizon_client::mod_api::BackendMutRef;
/// fn reach(backend: BackendMutRef<'_>) { backend.set_loaded(false); }
/// ```
#[doc(hidden)]
pub struct BackendMutRef<'a> {
    inner: &'a mut dyn BackendMut,
}

impl<'a> BackendRef<'a> {
    /// Wraps a read backend; only the generated game impl and this module's
    /// test doubles build a wrapper, always around themselves.
    pub(crate) fn of(inner: &'a dyn Backend) -> Self {
        Self { inner }
    }

    /// The backend inside; visible only inside `mod_api`, which the
    /// generated handle code and the capability tables are part of.
    fn open(self) -> &'a dyn Backend {
        self.inner
    }
}

impl<'a> BackendMutRef<'a> {
    /// Wraps a write backend; only the generated game impl and this module's
    /// test doubles build a wrapper, always around themselves.
    pub(crate) fn of(inner: &'a mut dyn BackendMut) -> Self {
        Self { inner }
    }

    /// The backend inside; visible only inside `mod_api`.
    fn open(self) -> &'a mut dyn BackendMut {
        self.inner
    }
}

/// The read backend behind `host`, or `None` where the host has none; the
/// caller answers its typed refusal instead of running on.
pub(in crate::mod_api) fn backend_of(host: &dyn Host) -> Option<&dyn Backend> {
    host.backend().map(|wrapper| wrapper.open())
}

/// The logical side on which a hook is executing.
///
/// A mutable client hook can use client services:
/// ```
/// use horizon_client::mod_api::{self, ClientHost, Hooks, InputKeyEvent};
/// #[derive(Default)]
/// struct ScreenMod;
/// #[mod_api::horizon_mod]
/// impl Hooks for ScreenMod {
///     fn input_key(&self, host: &mut dyn ClientHost, _: &mut InputKeyEvent<'_>) {
///         host.mod_screen_open("example");
///     }
/// }
/// ```
///
/// Side enums are non-exhaustive: external matches must include a wildcard.
/// ```compile_fail
/// use horizon_client::mod_api::HostSide;
/// fn side(host: HostSide<'_>) {
///     match host { HostSide::Server(_) => (), HostSide::Client(_) => () }
/// }
/// ```
///
/// Wrong-side services do not compile:
/// ```compile_fail
/// use horizon_client::mod_api::ClientHost;
/// fn wrong(host: &mut dyn ClientHost) { host.action_bar(0, todo!()); }
/// ```
/// ```compile_fail
/// use horizon_client::mod_api::ServerHost;
/// fn wrong(host: &mut dyn ServerHost) { host.mod_screen_open("example"); }
/// ```
///
/// A read-only hook receives only the base host, which cannot be used for a
/// server or client operation, or opened as a backend by a mod:
/// ```compile_fail
/// use horizon_client::mod_api::{Host, ServerHost};
/// fn server(_: &mut dyn ServerHost) {}
/// fn read(host: &dyn Host) { server(host); }
/// ```
/// ```compile_fail
/// use horizon_client::mod_api::{ClientHost, Host};
/// fn client(_: &dyn ClientHost) {}
/// fn read(host: &dyn Host) { client(host); }
/// ```
/// ```compile_fail
/// use horizon_client::mod_api::{backend_of, Host};
/// fn read(host: &dyn Host) { let _ = backend_of(host); }
/// ```
/// ```compile_fail
/// use horizon_client::mod_api::{Host, ServerHost};
/// fn read(host: &dyn Host) { let _ = host.downcast_ref::<dyn ServerHost>(); }
/// ```
impl HostSide<'_> {
    fn reborrow(&mut self) -> HostSide<'_> {
        match self {
            Self::Server(host) => HostSide::Server(&mut **host),
            Self::Client(host) => HostSide::Client(&mut **host),
        }
    }
}

impl LevelSide<'_> {
    fn reborrow(&mut self) -> LevelSide<'_> {
        match self {
            Self::Server(host) => LevelSide::Server(*host),
            Self::Client(host) => LevelSide::Client(*host),
        }
    }
}

/// The write backend behind `host`, or `None` where the host has none; the
/// caller answers its typed refusal instead of running on.
pub(in crate::mod_api) fn backend_mut_of(host: &mut dyn ServerHost) -> Option<&mut dyn BackendMut> {
    host.backend_mut().map(|wrapper| wrapper.open())
}

/// A player's right click on a block, shared by the block's use behaviours
/// and `right_click_block` (`PlayerInteractEvent.RightClickBlock`). The
/// event hears it once per hand on the clicking client and on the server.
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
    /// `TRY_WITH_EMPTY_HAND`: for a block's `use_item_on`, try its
    /// `use_without_item` next, on the main hand only.
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
#[non_exhaustive]
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
#[doc(hidden)]
pub use generated_host::{Backend, BackendMut};
pub use generated_host::{ClientHost, Host, HostSide, LevelSide, ServerHost};

/// Result of accessing or decoding an attachment.
pub type AttachmentResult<T> = std::result::Result<T, AttachmentError>;

macro_rules! host_attachment_host {
    ($($host:ident),+) => { $(
        impl AttachmentHost for dyn $host + '_ {
            fn attachments(&mut self, holder: Holder) -> Result<&mut Attachments, AttachmentError> {
                Host::attachments(self, holder)
            }
        }
    )+ };
}
host_attachment_host!(Host, ServerHost, ClientHost);

/// Read-only level services, like vanilla's `LevelReader`.
///
/// A level host cannot be used as a game host:
/// ```compile_fail
/// use horizon_client::mod_api::{Host, LevelHost};
/// fn game(_: &dyn Host) {}
/// fn level(host: &dyn LevelHost) { game(host); }
/// ```
/// ```compile_fail
/// use horizon_client::mod_api::{ClientHost, LevelHost, ServerHost};
/// fn server(_: &mut dyn ServerHost) {}
/// fn client(_: &dyn ClientHost) {}
/// fn level(host: &mut dyn LevelHost) { server(host); client(host); }
/// ```
/// A client-side level cannot write:
/// ```compile_fail
/// use horizon_client::mod_api::LevelSide;
/// fn write(side: LevelSide<'_>) {
///     if let LevelSide::Client(level) = side { level.set_block_property([0, 0, 0], "a", "b", 0); }
/// }
/// ```
pub trait LevelHost {
    /// Read a block through `LevelAccess::block_state`.
    fn block_state(&self, _pos: [i32; 3]) -> BlockStateId {
        unimplemented!("LevelHost::block_state")
    }
    /// The block `state` is a state of (`BlockState.getBlock()`): `mymod:shaft`. None for an
    /// unknown state.
    fn state_block(&self, _state: BlockStateId) -> Option<&str> {
        unimplemented!("LevelHost::state_block")
    }
    /// `BlockState.setValue`, through `native_state_with_property`; None if invalid.
    fn state_with_property(
        &self,
        _state: BlockStateId,
        _key: &str,
        _value: &str,
    ) -> Option<BlockStateId> {
        unimplemented!("LevelHost::state_with_property")
    }
}

/// Access vanilla's `Level.random`, so draws come in vanilla's order.
pub trait LevelRandomHost: LevelHost {
    /// Access a holder's server-owned attachment map.
    fn attachments(&mut self, _holder: Holder) -> Result<&mut Attachments, AttachmentError> {
        unimplemented!("LevelRandomHost::attachments")
    }
    /// The level's random stream through `native_level_random`, or None on a non-authoritative level.
    fn random(&mut self) -> Option<&mut LegacyRandom> {
        unimplemented!("LevelRandomHost::random")
    }
}

/// A `LevelReader` with vanilla's `ScheduledTickAccess`. Shape updates cannot write blocks.
pub trait LevelTicksHost: LevelHost {
    /// Schedule a block tick through `native_schedule_block_tick`.
    fn schedule_block_tick(&mut self, _pos: [i32; 3], _block: &str, _delay: i32) -> bool {
        unimplemented!("LevelTicksHost::schedule_block_tick")
    }
    /// Schedule a fluid tick through `native_schedule_fluid_tick`.
    fn schedule_fluid_tick(&mut self, _pos: [i32; 3], _fluid: &str, _delay: i32) -> bool {
        unimplemented!("LevelTicksHost::schedule_fluid_tick")
    }
}

/// Mutable level services, like vanilla's `ServerLevel`. Whole-block writes wait for block entity support.
pub trait LevelWriteHost: LevelTicksHost + LevelRandomHost {
    /// Change a block property through `native_set_block_property`.
    fn set_block_property(
        &mut self,
        _pos: [i32; 3],
        _key: &str,
        _value: &str,
        _flags: i32,
    ) -> bool {
        unimplemented!("LevelWriteHost::set_block_property")
    }
    /// Play a sound for nearby players through `native_play_sound`.
    fn play_sound(
        &mut self,
        _sound: &str,
        _at: [f64; 3],
        _source: usize,
        _volume: f32,
        _pitch: f32,
    ) -> bool {
        unimplemented!("LevelWriteHost::play_sound")
    }
    /// `ServerLevel.sendParticles` through `native_send_particles`.
    fn send_particles(
        &mut self,
        _options: &ParticleOptions,
        _state: Option<BlockStateId>,
        _at: [f64; 3],
        _spread: [f64; 3],
        _speed: f64,
        _count: i32,
    ) -> bool {
        unimplemented!("LevelWriteHost::send_particles")
    }
    /// `LevelAccessor.levelEvent(null, id, pos, data)` through `native_level_event`: the event
    /// is queued with the write's own events, in order, and plays when the write is done.
    /// `LevelRef::level_event` takes the same `id` and `data`.
    fn level_event(&mut self, _id: i32, _pos: [i32; 3], _data: i32) -> bool {
        unimplemented!("LevelWriteHost::level_event")
    }
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
mod energy_capability;
pub use energy_capability::{ENERGY, EnergyHandler, ITEM_ENERGY, move_energy};
mod fluid_capability;
pub use fluid_capability::{FLUIDS, FluidResource, FluidStack, ITEM_FLUIDS};
mod item_place;
mod menu_slot;
pub use menu_slot::MenuSlotBacking;
mod transaction;
pub use item_place::{ITEM_ITEMS, ItemCapabilityKey, ItemPlace, PlaceProvider};
mod resource;
pub use resource::{
    ComponentId, EnchantmentEffect, ItemResource, Overanswered, PREVENT_ARMOR_CHANGE,
    ResourceHandler, move_resources,
};
pub use transaction::{HostTransaction, Transaction, TransactionSeal, run_transaction};
mod ticket_controller;
pub use ticket_controller::TicketController;
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
/// Internal test-fixture attribute for modules with multiple hook sets; mods use `#[mod_api::horizon_mod]`.
#[doc(hidden)]
pub use horizon_mod_api_macros::__subscriptions;
/// Mark a hook implementation and derive its subscriptions.
///
/// On `impl Hooks`, also exports `pub fn hooks()` using the mod's `Default`
/// implementation. Put that impl at the crate root so the launcher finds it.
/// On `impl RenderHooks`, derives subscriptions without another export.
///
/// An implementation without the attribute does not compile:
/// ```compile_fail
/// use horizon_client::mod_api::Hooks;
/// struct Mod;
/// impl Hooks for Mod {}
/// ```
/// Rustc rejects a misspelled trait method (E0407):
/// ```compile_fail,E0407
/// use horizon_client::mod_api::{self, Hooks};
/// #[derive(Default)]
/// struct Mod;
/// #[mod_api::horizon_mod]
/// impl Hooks for Mod {
///     fn baby_entity_spwan(&self) {}
/// }
/// ```
/// Two mod markers in one module define the same export:
/// ```compile_fail,E0428
/// use horizon_client::mod_api::{self, Hooks};
/// #[derive(Default)] struct A;
/// #[derive(Default)] struct B;
/// #[mod_api::horizon_mod] impl Hooks for A {}
/// #[mod_api::horizon_mod] impl Hooks for B {}
/// ```
pub use horizon_mod_api_macros::horizon_mod;

#[cfg(test)]
pub(crate) use part_table::tests as part_tests;

#[cfg(test)]
mod hooks_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_api;
    use std::sync::Mutex;

    #[test]
    fn level_family_accepts_empty_doubles() {
        struct Empty;
        impl LevelHost for Empty {}
        impl LevelRandomHost for Empty {}
        impl LevelTicksHost for Empty {}
        impl LevelWriteHost for Empty {}

        let _: &dyn LevelHost = &Empty;
        let _: &mut dyn LevelRandomHost = &mut Empty;
        let _: &mut dyn LevelTicksHost = &mut Empty;
        let _: &mut dyn LevelWriteHost = &mut Empty;
    }

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
    #[mod_api::__subscriptions]
    impl Hooks for Quiet {}

    struct Broken;
    #[mod_api::__subscriptions]
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

    impl Backend for Reader {
        fn level_block(&self, _level: LevelRef, _pos: [i32; 3]) -> Option<String> {
            Some(self.block.to_string())
        }
    }

    impl Host for Reader {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(BackendRef::of(self))
        }
        fn level(&self, dimension: &str) -> Option<LevelRef> {
            (dimension == "minecraft:overworld")
                .then(|| LevelRef::new(NativeWorldEpoch::new(1), "minecraft:overworld"))
        }
    }
    impl ClientHost for Reader {}
    impl ServerHost for Reader {
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
        #[mod_api::__subscriptions]
        impl Hooks for HandMod {
            fn render_hand(
                &self,
                _host: &dyn ClientHost,
                event: &mut RenderHandEvent<'_, '_>,
            ) -> Flow {
                assert_eq!((event.render.player, event.render.player_entity_id), (7, 9));
                event.render.translation[0] += self.offset;
                if self.cancel {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        let mods = [
            ModEntry::new(
                "hand-first",
                Box::new(HandMod {
                    offset: 1.0,
                    cancel: false,
                }),
            ),
            ModEntry::new(
                "hand-cancel",
                Box::new(HandMod {
                    offset: 2.0,
                    cancel: true,
                }),
            ),
            ModEntry::new(
                "hand-last",
                Box::new(HandMod {
                    offset: 4.0,
                    cancel: false,
                }),
            ),
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

    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn use_on(&self, host: &mut dyn ServerHost, event: &mut UseOnEvent<'_>) -> Flow {
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
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn living_conversion_post(
                &self,
                _host: &mut dyn ServerHost,
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
        let mods = [ModEntry::new(
            "conversion-observer",
            Box::new(Observer(observed.clone())),
        )];
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
        #[mod_api::__subscriptions]
        impl Hooks for XpHook {
            fn player_xp_change(
                &self,
                _host: &mut dyn ServerHost,
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
            let mods = [ModEntry::new("xp-hook", Box::new(XpHook(cancelled)))];
            install(mods.into());
            let (mut amount, mut cancel) = (10, false);
            dispatch_player_xp_change(&mut host, 7, 42, "Probe", &mut amount, &mut cancel);
            assert_eq!((amount, cancel), (-5, cancelled));
        }
    }

    struct MorePhantoms;

    #[mod_api::__subscriptions]
    impl Hooks for MorePhantoms {
        fn player_spawn_phantoms(
            &self,
            _host: &mut dyn ServerHost,
            event: &mut PlayerSpawnPhantomsEvent<'_>,
        ) -> Option<PhantomSpawnResult> {
            *event.phantoms = event.phantoms.map(|count| count + 2);
            None
        }
    }

    #[test]
    fn phantom_listener_can_increase_the_default_count() {
        let mods = [ModEntry::new("phantoms", Box::new(MorePhantoms))];
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

    #[mod_api::__subscriptions]
    impl Hooks for Chatty {
        fn use_on(&self, host: &mut dyn ServerHost, _event: &mut UseOnEvent<'_>) -> Flow {
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
    fn dispatch_before_installation_is_empty() {
        let mut components = Vec::new();
        dispatch_component_types(&mut components);
        assert!(mods().is_empty());
        assert!(components.is_empty());
    }

    #[test]
    #[should_panic(expected = "native mod id must not be empty")]
    fn an_entry_requires_a_nonempty_id() {
        ModEntry::new("", Box::new(Quiet));
    }

    #[test]
    fn a_panic_in_a_hook_names_the_mod_and_the_hook() {
        let mods: Vec<ModEntry> = vec![
            ModEntry::new("quiet", Box::new(Quiet)),
            ModEntry::new("broken", Box::new(Broken)),
        ];
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
        #[mod_api::__subscriptions]
        impl Hooks for Turn {
            fn calculate_player_turn(
                &self,
                _host: &dyn ClientHost,
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
        let mods: Vec<ModEntry> = vec![
            ModEntry::new("turn-1", Box::new(Turn)),
            ModEntry::new("turn-2", Box::new(Turn)),
        ];
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
        let mods = [ModEntry::new("probe", Box::new(Probe))];
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
            dispatch_use_on(
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

    #[mod_api::__subscriptions]
    impl Hooks for Backstab {
        fn living_incoming_damage_v2(
            &self,
            host: &mut dyn ServerHost,
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
        let mods = [ModEntry::new("backstab", Box::new(Backstab))];
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
        #[mod_api::__subscriptions]
        impl Hooks for MiningProbe {
            fn player_harvest_check(
                &self,
                _host: crate::mod_api::HostSide,
                event: &mut PlayerHarvestCheckEvent,
            ) {
                self.0.lock().unwrap().push(event.player);
                event.can_harvest = false;
            }
            fn player_break_speed(
                &self,
                _host: crate::mod_api::HostSide,
                event: &mut PlayerBreakSpeedEvent,
            ) -> Flow {
                self.0.lock().unwrap().push(event.player);
                event.new_speed = 2.0;
                Flow::Continue
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = [ModEntry::new(
            "mining-probe",
            Box::new(MiningProbe(seen.clone())),
        )];
        install(mods.into());
        let mut host = Reader {
            block: "minecraft:stone",
            lines: Vec::new(),
        };
        let mut harvest = true;
        let mut speed = 1.0;
        let mut canceled = false;
        dispatch_player_harvest_check(HostSide::Server(&mut host), 42, [1, 2, 3], 7, &mut harvest);
        dispatch_player_break_speed(
            HostSide::Server(&mut host),
            42,
            [1, 2, 3],
            7,
            1.0,
            &mut speed,
            &mut canceled,
        );
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
        let mods = [ModEntry::new("chatty", Box::new(Chatty))];
        install(mods.into());
        let mut host = Reader {
            block: "minecraft:stone",
            lines: Vec::new(),
        };
        let mut claimed = false;
        let (pos, face) = ([0; 3], [0, 1, 0]);
        dispatch_use_on(
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

    #[mod_api::__subscriptions]
    impl Hooks for OffHandTop {
        fn use_on(&self, _: &mut dyn ServerHost, event: &mut UseOnEvent<'_>) -> Flow {
            assert_eq!((event.player, event.player_entity_id), (7, 41));
            assert_eq!((event.hand, event.hit), (Hand::Off, top_hit()));
            Flow::Continue
        }

        fn right_click_block(
            &self,
            _: crate::mod_api::HostSide,
            event: &mut RightClickBlockEvent,
        ) -> Flow {
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
        let mods = [ModEntry::new("off-hand-top", Box::new(OffHandTop))];
        install(mods.into());
        let mut host = Reader {
            block: "minecraft:stone",
            lines: Vec::new(),
        };
        let mut claimed = false;
        dispatch_use_on(
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
                HostSide::Server(&mut host),
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

    #[mod_api::__subscriptions]
    impl Hooks for BrewSwapper {
        fn potion_brew_pre(
            &self,
            _host: &mut dyn ServerHost,
            event: &mut PotionBrewPreEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push("pre".into());
            event.stacks[0] = Some(ItemStack::new("minecraft:diamond", 1));
            Flow::Cancel
        }
        fn potion_brew_post(
            &self,
            _host: &mut dyn ServerHost,
            event: &mut PotionBrewPostEvent<'_>,
        ) {
            let first = event.stacks[0].as_ref().map_or("", |s| &s.item);
            self.0.lock().unwrap().push(format!("post {first}"));
            event.stacks[1] = Some(ItemStack::new("minecraft:emerald", 1));
        }
    }

    /// Edits the slots in `PotionBrewEvent.Pre` without cancelling.
    struct BrewEditor;

    #[mod_api::__subscriptions]
    impl Hooks for BrewEditor {
        fn potion_brew_pre(
            &self,
            _host: &mut dyn ServerHost,
            event: &mut PotionBrewPreEvent<'_>,
        ) -> Flow {
            event.stacks[0] = Some(ItemStack::new("minecraft:diamond", 1));
            Flow::Continue
        }
        fn potion_brew_post(
            &self,
            _host: &mut dyn ServerHost,
            event: &mut PotionBrewPostEvent<'_>,
        ) {
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
        let mods: Vec<ModEntry> = vec![ModEntry::new(
            "brew-swapper",
            Box::new(BrewSwapper(seen.clone())),
        )];
        install(mods.into());
        let mut stand = fresh();
        assert!(!brew(&mut host, &mut stand));
        assert_eq!(*seen.lock().unwrap(), ["pre", "post minecraft:diamond"]);
        assert_eq!(&*stand.items[0].as_ref().unwrap().item, "minecraft:diamond");
        assert_eq!(&*stand.items[1].as_ref().unwrap().item, "minecraft:emerald");
        assert_eq!(stand.items[3].as_ref().map(|s| s.count), Some(2));

        let mods: Vec<ModEntry> = vec![ModEntry::new("brew-editor", Box::new(BrewEditor))];
        install(mods.into());
        let mut stand = fresh();
        assert!(brew(&mut host, &mut stand));
        assert_eq!(stand.items[0], Some(awkward));
        assert_eq!(&*stand.items[1].as_ref().unwrap().item, "minecraft:emerald");
        assert_eq!(stand.items[3].as_ref().map(|s| s.count), Some(1));
    }

    /// Writes down every game rule change and level save it hears.
    struct Recorder(Arc<Mutex<Vec<String>>>);

    #[mod_api::__subscriptions]
    impl Hooks for Recorder {
        fn game_rule_changed(
            &self,
            _host: &mut dyn ServerHost,
            event: &mut GameRuleChangedEvent<'_>,
        ) {
            self.0
                .lock()
                .unwrap()
                .push(format!("{} {}", event.rule, event.value));
        }
        fn level_save(&self, _host: &mut dyn ServerHost, event: &mut LevelSaveEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("saved {}", event.dimension));
        }
    }

    #[test]
    fn game_rule_change_reports_optional_actor() {
        struct ActorRecorder(Arc<Mutex<Vec<Option<(u128, i32, String)>>>>);
        #[mod_api::__subscriptions]
        impl Hooks for ActorRecorder {
            fn game_rule_changed(
                &self,
                _: &mut dyn ServerHost,
                event: &mut GameRuleChangedEvent<'_>,
            ) {
                self.0.lock().unwrap().push(
                    event
                        .actor
                        .map(|actor| (actor.uuid, actor.entity_id, actor.name.to_owned())),
                );
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = [ModEntry::new(
            "actor-recorder",
            Box::new(ActorRecorder(seen.clone())),
        )];
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
        let mods: Vec<ModEntry> = vec![
            ModEntry::new("recorder-10", Box::new(Recorder(seen.clone()))),
            ModEntry::new("recorder-11", Box::new(Recorder(seen.clone()))),
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
        let mods: Vec<ModEntry> = vec![
            ModEntry::new("recorder-12", Box::new(Recorder(seen.clone()))),
            ModEntry::new("recorder-13", Box::new(Recorder(seen.clone()))),
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

    #[mod_api::__subscriptions]
    impl Hooks for CodKeeper {
        fn item_fished(&self, _host: &mut dyn ServerHost, event: &mut ItemFishedEvent<'_>) -> Flow {
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

    #[mod_api::__subscriptions]
    impl Hooks for Unreached {
        fn item_fished(
            &self,
            _host: &mut dyn ServerHost,
            _event: &mut ItemFishedEvent<'_>,
        ) -> Flow {
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
        let mods: Vec<ModEntry> = vec![
            ModEntry::new("cod-keeper", Box::new(CodKeeper)),
            ModEntry::new("unreached", Box::new(Unreached)),
        ];
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
        let mods: Vec<ModEntry> = vec![ModEntry::new("cod-keeper", Box::new(CodKeeper))];
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

    #[mod_api::__subscriptions]
    impl Hooks for BrewWatcher {
        fn player_brewed_potion(
            &self,
            _host: &mut dyn ServerHost,
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
        let mods: Vec<ModEntry> = vec![ModEntry::new(
            "brew-watcher",
            Box::new(BrewWatcher(seen.clone())),
        )];
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
