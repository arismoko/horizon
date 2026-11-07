//! Trait mods implement `Hooks` and link into the game executable.
//! The executable passes the ordered `Mods` to `entry::main_with`.
//! Trait mods are process-wide, like mixin module statics.
//! Hooks use `&self + Send + Sync` for shared and reentrant calls; see `docs/modding/src/how-to/native-mods.md`.
//! Generate the trait and dispatch with `cargo run -p xtask -- mod-api`.
#![deny(missing_docs)]
// The generated `impl_*_areas!` macros are exported for test doubles outside the crate, and this crate's
// own test doubles use them by path too.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
extern crate self as mod_api;

/// A saved fluid name retained for the lifetime of the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only trusted engine boundaries create handles
#[horizon_mod_api_macros::wasm(host_constructor = from_slot)]
pub struct UnresolvedFluid(u64);

/// A block requested once through [`Registrar::block_key`](BlocksRegistrar::block_key). Its load-owned slot survives reload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its field is private, so only registration creates a key
#[horizon_mod_api_macros::wasm(host_constructor = from_slot)]
pub struct BlockKey(u32);

/// A baked standalone model resolved through [`RenderAssets::model`].
/// Retain it between frames; reacquire it after a successful resource reload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: private fields; only the engine resolves model handles
#[horizon_mod_api_macros::wasm(host_constructor = from_parts)]
pub struct ModelRef(u64, u32);

impl ModelRef {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished model generation");
        }
        Ok(Self(generation, slot))
    }
}

/// Why a standalone model could not be resolved or drawn. Errors append no geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModelError {
    /// The name is not a registered, successfully baked standalone model, or its slot is absent.
    UnknownModel,
    /// A successful resource replacement superseded the handle's generation.
    StaleModel,
}

/// A dimension (`minecraft:the_nether`), as the game's table numbers it: a `LevelRef`'s identity.
/// A key stays the same dimension until the game exits; `key.id(host)` gives the name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its field is private, so only the game's table creates a key
#[horizon_mod_api_macros::wasm(host_constructor = from_slot)]
pub struct DimensionKey(u32);

/// Why a game rule category declaration was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GameRuleError {
    /// The identifier is invalid.
    InvalidId,
    /// The namespace does not belong to the declaring mod.
    ForeignNamespace,
    /// The category is already declared.
    Duplicate,
}

/// Why an entity type's attribute declaration was refused
/// ([`Registrar::declare_entity_attributes`](EntityAttributesRegistrar::declare_entity_attributes)).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntityAttributeError {
    /// The entity type's id is invalid, empty, or in a namespace the declaring
    /// mod does not own.
    InvalidId,
    /// The set declares the same attribute twice.
    DuplicateAttribute,
    /// A base value is not finite.
    NotFinite,
    /// Another declaration already covers the entity type.
    Duplicate,
}

/// A boolean game rule declared by a mod.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the registration table creates keys
#[horizon_mod_api_macros::wasm(host_constructor = from_slot)]
pub struct BoolGameRuleKey(u32);
/// A load-owned fluid identity requested through [`Registrar::fluid_key`](FluidsRegistrar::fluid_key) or resolved through
/// [`DataLoadContext::fluid_key`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the registration table creates keys
#[horizon_mod_api_macros::wasm(host_constructor = from_slot)]
pub struct FluidKey(u32);
/// An item requested at registration; its load-owned slot identifies the canonical name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only registration creates a key
#[horizon_mod_api_macros::wasm(host_constructor = from_slot)]
pub struct ItemKey(u32);

/// An effect requested once through [`Registrar::effect_key`](EffectsRegistrar::effect_key). The registry is fixed until restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its field is private, so only registration creates a key
#[horizon_mod_api_macros::wasm(host_constructor = from_slot)]
pub struct EffectKey(u32);

/// A block entity type requested at registration. Its slot remains stable for the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the load-owned table creates keys
#[horizon_mod_api_macros::wasm(host_constructor = from_slot)]
pub struct BlockEntityTypeKey(u32);

use std::sync::{Arc, OnceLock};

/// The family of a client notification entering the toast queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToastKind {
    /// An advancement completed by the local player.
    Advancement,
    /// Newly unlocked recipe displays.
    Recipe,
    /// A tutorial hint, including its progress indicator.
    Tutorial,
    /// A system notice, such as narrator status, backups or pack downloads.
    System,
    /// A toast without one of the above families, including now-playing music.
    Other,
}

/// Build range and sky properties of a loaded level's dimension type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LevelDimensionShape {
    /// Lowest buildable block Y.
    pub min_y: i32,
    /// Build height in blocks.
    pub height: i32,
    /// Height used by portals and chorus fruit.
    pub logical_height: i32,
    /// Whether the dimension has skylight.
    pub sky_light: bool,
    /// Whether the dimension has a ceiling.
    pub has_ceiling: bool,
}
impl LevelDimensionShape {
    /// Construct a dimension shape from its build and sky properties.
    pub fn new(
        min_y: i32,
        height: i32,
        logical_height: i32,
        sky_light: bool,
        has_ceiling: bool,
    ) -> Self {
        Self {
            min_y,
            height,
            logical_height,
            sky_light,
            has_ceiling,
        }
    }
}

/// The registry facts a recipe viewer needs for an enchantment.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EnchantmentInfo {
    /// Registry name, including namespace.
    pub id: String,
    /// Largest supported level.
    pub max_level: u32,
    /// Whether the enchantment is in the curse tag.
    pub curse: bool,
}
impl EnchantmentInfo {
    /// Construct registry facts for an enchantment.
    pub fn new(id: String, max_level: u32, curse: bool) -> Self {
        Self {
            id,
            max_level,
            curse,
        }
    }
}

/// A fluid state copied from the engine. For engine-built values, amount is zero exactly for
/// the empty fluid. A mod may change a placement event's copy; that mod-provided data is
/// forwarded to later Parts and is never consumed by the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FluidState {
    /// Registered fluid identity.
    pub fluid: FluidKey,
    /// Fluid amount; zero only for empty, otherwise 1 through 8.
    pub amount: u8,
    /// Whether the fluid is falling.
    pub falling: bool,
    /// Whether the engine identifies this as a source.
    pub source: bool,
}
impl FluidState {
    /// Whether this is the empty fluid.
    pub fn is_empty(self) -> bool {
        self.amount == 0
    }
    /// Whether the engine identifies this state as a source.
    pub fn is_source(self) -> bool {
        self.source
    }
    /// Match carried vanilla properties locally. Unknown names or noncanonical values
    /// do not match. Validate against `host.fluid_states()` separately for indexed errors.
    pub fn matches(self, constraints: &[StateConstraint]) -> bool {
        constraints.iter().all(|c| match c.name.as_str() {
            "falling" if !self.is_empty() => c.value == if self.falling { "true" } else { "false" },
            "level" if !self.is_empty() && !self.source => {
                (1..=8).contains(&self.amount) && c.value.as_bytes() == [b'0' + self.amount]
            }
            _ => false,
        })
    }
}

/// A hook's mutable block entity payload, with a read-only type identity.
/// The private handle cannot be constructed or taken out by a mod.
///
/// ```compile_fail
/// use mod_api::{BlockEntityTickEvent, ModBlockEntity};
/// fn replace(event: &mut BlockEntityTickEvent<'_>) {
///     *event.entity = ModBlockEntity::default();
/// }
/// ```
///
/// ```compile_fail
/// use mod_api::BlockEntityTickEvent;
/// fn take(event: &mut BlockEntityTickEvent<'_>) {
///     let _ = std::mem::take(&mut event.entity);
/// }
/// ```
///
/// ```compile_fail
/// use mod_api::{BlockEntityTickEvent, ModBlockEntity};
/// fn access(event: &mut BlockEntityTickEvent<'_>) {
///     let _: &mut ModBlockEntity = event.entity.entity;
/// }
/// ```
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ModBlockEntityView<'a> {
    entity: &'a mut ModBlockEntity,
}
impl<'a> ModBlockEntityView<'a> {
    pub(crate) fn new(entity: &'a mut ModBlockEntity) -> Self {
        Self { entity }
    }
    /// Id.
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
/// What a key did in an `input_key` event (GLFW's `action`: release 0, press 1, repeat 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
pub enum KeyAction {
    /// The key was let go.
    Release,
    /// The key went down.
    Press,
    /// The key is held and the operating system repeats it.
    Repeat,
}

/// One armor slot of an `ArmorHurtEvent`: the piece in `slot`, the durability
/// damage vanilla works out for it (`damage`, `max(1, hit / 4)`) and the damage it
/// takes (`new_damage`, starts equal; a mod may change it, below 1 for none; the
/// game casts it to a whole number as vanilla does).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct NativeArmorHurt {
    /// The equipment slot the armor piece sits in.
    pub slot: EquipmentSlot,
    /// The armor piece being damaged.
    pub stack: ItemStack,
    /// The durability damage vanilla computed.
    pub damage: f32,
    /// The durability damage to apply instead; starts equal to `damage`.
    pub new_damage: f32,
}

/// A snapshot of the client window's modifier keys.
///
/// The bits use GLFW's input-event encoding: shift 1, control 2, alt 4 and super 8.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClientModifiers {
    /// Modifier bits in GLFW's input-event encoding.
    pub bits: u32,
}

impl ClientModifiers {
    /// Construct a snapshot from GLFW modifier bits.
    pub fn new(bits: u32) -> Self {
        Self { bits }
    }
    /// Whether shift is held.
    pub fn shift(self) -> bool {
        self.bits & 1 != 0
    }
    /// Whether control is held.
    pub fn control(self) -> bool {
        self.bits & 2 != 0
    }
    /// Whether alt is held.
    pub fn alt(self) -> bool {
        self.bits & 4 != 0
    }
    /// Whether super is held.
    pub fn super_key(self) -> bool {
        self.bits & 8 != 0
    }
}

/// A vanilla key mapping as the client holds it (`ClientHost::mapping`).
#[derive(Clone, Debug)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct NativeKeyMapping {
    /// Whether it is held (`KeyMapping.isDown`).
    pub down: bool,
    /// The name of the key it is bound to, as Controls shows it (`KeyMapping.getTranslatedKeyMessage`).
    pub label: Text,
}

/// The metadata of a server play packet observed by `Hooks::client_packet`.
///
/// Fields beyond the type id are exposed only for custom payloads. This is
/// plain mod API data, not the game's decoded protocol enum.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ClientPacket {
    /// The protocol type, such as `minecraft:ping` or `minecraft:custom_payload`.
    /// An id outside the protocol's packet table is reported as `horizon:unknown`.
    pub id: Identifier,
    /// The channel and bytes of a custom payload; `None` for other packet types.
    pub custom_payload: Option<ClientPayload>,
}

impl ClientPacket {
    /// Construct packet metadata with an optional custom payload.
    pub fn new(id: Identifier, custom_payload: Option<ClientPayload>) -> Self {
        Self { id, custom_payload }
    }
}

/// The channel and uninterpreted bytes of a server custom payload.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ClientPayload {
    /// The namespaced plugin channel, exactly as received.
    pub channel: String,
    /// The payload bytes, without the packet id or channel prefix.
    pub data: Vec<u8>,
}

impl ClientPayload {
    /// Construct a custom payload without interpreting its bytes.
    pub fn new(channel: String, data: Vec<u8>) -> Self {
        Self { channel, data }
    }
}

/// Camera values from the most recently extracted world frame.
#[derive(Clone, Debug)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct NativeCamera {
    /// The camera position in blocks (x, y, z).
    pub position: [f64; 3],
    /// The eye height above the position, in blocks.
    pub eye_height: f32,
    /// The fraction of the current tick elapsed, 0 to 1.
    pub partial_tick: f32,
    /// The horizontal rotation in degrees.
    pub yaw: f32,
    /// The vertical rotation in degrees; negative looks up.
    pub pitch: f32,
    /// The vertical field of view in degrees.
    pub fov_degrees: f32,
    /// The near clip distance in blocks.
    pub near: f32,
    /// The far clip distance in blocks.
    pub far: f32,
    /// Camera-relative view matrix in column-major order.
    pub view: [f32; 16],
    /// Reversed-Z projection matrix in column-major order.
    pub projection: [f32; 16],
    /// The id of the dimension drawn, such as `minecraft:overworld`.
    pub dimension: String,
}

/// A snapshot of the client's input preferences.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ClientInputOptions {
    /// Mouse sensitivity, normally between 0 and 1.
    pub sensitivity: f32,
    /// Whether horizontal mouse movement is inverted.
    pub invert_x: bool,
    /// Whether vertical mouse movement is inverted.
    pub invert_y: bool,
    /// Multiplier applied to mouse wheel movement.
    pub mouse_wheel_sensitivity: f32,
    /// Whether scroll input is reduced to discrete steps.
    pub discrete_scroll: bool,
    /// Whether crouching toggles instead of requiring a held key.
    pub toggle_crouch: bool,
    /// Whether sprinting toggles instead of requiring a held key.
    pub toggle_sprint: bool,
    /// Whether automatic jumping is enabled.
    pub auto_jump: bool,
}

/// A snapshot of the client's graphics preferences.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ClientGraphicsOptions {
    /// Field of view in degrees, 30 to 110.
    pub fov: f32,
    /// Render distance in chunks.
    pub render_distance: i32,
    /// Simulation distance in chunks.
    pub simulation_distance: i32,
    /// Brightness, 0 to 1.
    pub brightness: f32,
    /// Frame rate limit; 260 means unlimited.
    pub max_fps: u32,
    /// Whether vertical sync is on.
    pub vsync: bool,
    /// Whether the window is fullscreen.
    pub fullscreen: bool,
    /// Whether the view bobs while walking.
    pub view_bobbing: bool,
    /// Whether smooth lighting is on.
    pub smooth_lighting: bool,
    /// Particle setting: 0 all, 1 decreased, 2 minimal.
    pub particles: u8,
    /// Entity distance scale.
    pub entity_distance: f32,
    /// Biome blend radius.
    pub biome_blend: i32,
}

/// A snapshot of the client's GUI and accessibility preferences.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ClientGuiOptions {
    /// Requested GUI scale; 0 means automatic.
    pub gui_scale: u32,
    /// Attack indicator: 0 off, 1 crosshair, 2 hotbar.
    pub attack_indicator: u8,
    /// Whether the main hand is the left one.
    pub main_hand_left: bool,
    /// Whether the debug screen shows reduced information.
    pub reduced_debug_info: bool,
    /// Whether the high contrast resource pack is on.
    pub high_contrast: bool,
    /// Whether subtitles are shown.
    pub show_subtitles: bool,
    /// Multiplier for how long subtitles and toasts stay.
    pub notification_display_time: f32,
    /// Opacity of the background behind text, 0 to 1.
    pub text_background_opacity: f32,
    /// Menu background blur radius.
    pub menu_blur: i32,
}

/// A snapshot of the client's chat preferences.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ClientChatOptions {
    /// Which chat is shown: 0 all, 1 commands only, 2 hidden.
    pub visibility: u8,
    /// Whether colours in chat show.
    pub colors: bool,
    /// Whether links in chat are clickable.
    pub links: bool,
    /// Raw `chatOpacity` preference, 0 to 1; text opacity is `value * 0.9 + 0.1`.
    /// Background opacity is [`ClientGuiOptions::text_background_opacity`].
    pub opacity: f32,
    /// Chat scale.
    pub scale: f32,
    /// Chat width, 0 to 1 of its range.
    pub width: f32,
    /// Chat height while the chat is open, 0 to 1 of its range.
    pub height_focused: f32,
    /// Chat height while the chat is closed, 0 to 1 of its range.
    pub height_unfocused: f32,
    /// Chat line spacing.
    pub line_spacing: f32,
    /// Seconds between player chat messages showing.
    pub delay: f64,
}

/// A snapshot of the client's audio preferences.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ClientAudioOptions {
    /// Master volume, 0 to 1.
    pub master_volume: f32,
}

/// Context and adjustable pose for a first-person hand render.
///
/// Translation is in blocks and rotation is in degrees (pitch, yaw, roll).
/// The renderer applies the transform before the ordinary hand and item poses.
// exhaustive: the game builds it with a literal; mods only read it
pub struct NativeHandRender<'a> {
    /// The UUID of the player whose hand is drawn.
    pub player: u128,
    /// The network id of that player.
    pub player_entity_id: i32,
    /// Which hand is drawn.
    pub hand: Hand,
    /// The stack shown in the hand; `None` for the empty main-hand arm.
    pub stack: Option<&'a ItemStack>,
    /// The fraction of the current tick elapsed, 0 to 1.
    pub partial: f32,
    /// The player pitch interpolated for this frame, in degrees.
    pub interpolated_pitch: f32,
    /// The swing progress, 0 to 1.
    pub swing: f32,
    /// The inverse equip height, including the swap animation scale.
    pub equip: f32,
    /// Vanilla packed light at the eyes (`LightCoordsUtil.pack`: block << 4 | sky << 20).
    pub packed_light: u32,
    /// A mod-set offset in blocks; applied before the hand is drawn.
    pub translation: [f32; 3],
    /// A mod-set rotation as pitch, yaw and roll in degrees.
    pub rotation: [f32; 3],
    /// A mod-set uniform scale; 1 leaves the size alone.
    pub scale: f32,
}

/// A dispenser hook's answer: the slot's remainder (None empties it) and the
/// level event to play, `CLICK` (1000), `FAIL` (1001) or 0 for none.
pub type NativeDispensed = (Option<ItemStack>, i32);

/// Context for a player tuning a note block.
#[derive(Clone, Copy, Debug)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct NativeNoteBlockChange {
    /// The level the note block is in.
    pub dimension: LevelDimension,
    /// The note block's position.
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
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NativeItemDrop {
    /// The dropped stack.
    pub stack: ItemStack,
    /// Where the item entity appears, in blocks.
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
    /// The harp, over most blocks.
    Harp,
    /// The bass drum, over stone.
    Basedrum,
    /// The snare drum, over sand and gravel.
    Snare,
    /// The clicks, over glass.
    Hat,
    /// The double bass, over wood.
    Bass,
    /// The flute, over clay.
    Flute,
    /// The bell, over a gold block.
    Bell,
    /// The guitar, over wool.
    Guitar,
    /// The chime, over packed ice.
    Chime,
    /// The xylophone, over a bone block.
    Xylophone,
    /// The iron xylophone, over an iron block.
    IronXylophone,
    /// The cow bell, over soul sand.
    CowBell,
    /// The didgeridoo, over a pumpkin.
    Didgeridoo,
    /// The bit, over an emerald block.
    Bit,
    /// The banjo, over hay.
    Banjo,
    /// The pling, over glowstone.
    Pling,
    /// The trumpet, over a copper block.
    Trumpet,
    /// The trumpet, over an exposed copper block.
    TrumpetExposed,
    /// The trumpet, over an oxidized copper block.
    TrumpetOxidized,
    /// The trumpet, over a weathered copper block.
    TrumpetWeathered,
    /// A zombie's sound, over a zombie head.
    Zombie,
    /// A skeleton's sound, over a skeleton skull.
    Skeleton,
    /// A creeper's sound, over a creeper head.
    Creeper,
    /// The ender dragon's sound, over a dragon head.
    Dragon,
    /// A wither skeleton's sound, over its skull.
    WitherSkeleton,
    /// A piglin's sound, over a piglin head.
    Piglin,
    /// The sound set by a player head's own note block sound.
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
    /// The portal frame's inner width in blocks.
    pub width: i32,
    /// The portal frame's inner height in blocks.
    pub height: i32,
}

/// The mixer group a sound plays on (`SoundSource`), in vanilla's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SoundSource {
    /// The master volume, which scales every other source.
    Master,
    /// Music discs aside, the game's music.
    Music,
    /// Jukeboxes and music discs.
    Records,
    /// Rain and thunder.
    Weather,
    /// Sounds blocks make.
    Blocks,
    /// Hostile creatures.
    Hostile,
    /// Friendly and neutral creatures.
    Neutral,
    /// Players.
    Players,
    /// Ambient and cave sounds.
    Ambient,
    /// Voice sounds.
    Voice,
    /// Interface sounds, such as a button click.
    Ui,
}

impl SoundSource {
    /// Its position in vanilla's `SoundSource` order, as the network sends it.
    pub fn index(self) -> usize {
        self as usize
    }
    /// Every source, in vanilla's order (`SoundSource.values()`).
    pub const ALL: [SoundSource; 11] = [
        Self::Master,
        Self::Music,
        Self::Records,
        Self::Weather,
        Self::Blocks,
        Self::Hostile,
        Self::Neutral,
        Self::Players,
        Self::Ambient,
        Self::Voice,
        Self::Ui,
    ];
}

/// How a sound fades with distance (`SoundInstance.Attenuation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SoundAttenuation {
    /// Heard at full volume everywhere.
    None,
    /// Fades linearly to silence at the sound's range.
    Linear,
}

/// A sound about to play, for `play_sound` (`SoundInstance`'s own settings:
/// vanilla's `getIdentifier`, `getSource`, `getVolume`, `getPitch`, `getX/Y/Z`,
/// `isLooping`, `getDelay`, `getAttenuation` and `isRelative`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SoundPlay {
    /// The sound event (`minecraft:entity.pig.ambient`).
    pub identifier: String,
    /// The mixer group it plays on.
    pub source: SoundSource,
    /// Its own volume, before the source's and the master's.
    pub volume: f32,
    /// Its pitch, 1.0 unchanged.
    pub pitch: f32,
    /// Where it plays, in blocks; meaningless when `relative`.
    pub position: [f64; 3],
    /// It restarts when it ends.
    pub looping: bool,
    /// Ticks between a looping sound's repeats (`SoundInstance.getDelay`): 0 loops it
    /// seamlessly, more plays it again that many ticks after it ends.
    pub delay: i32,
    /// How it fades with distance.
    pub attenuation: SoundAttenuation,
    /// It plays at the listener, wherever they move.
    pub relative: bool,
}
impl SoundPlay {
    /// Sound event `identifier` at the listener, not fading with distance, on the interface
    /// source at volume and pitch 1 (`SimpleSoundInstance.forUI`).
    pub fn ui(identifier: &str) -> Self {
        Self {
            identifier: identifier.to_owned(),
            source: SoundSource::Ui,
            volume: 1.0,
            pitch: 1.0,
            position: [0.0; 3],
            looping: false,
            delay: 0,
            attenuation: SoundAttenuation::None,
            relative: true,
        }
    }
    /// Sound event `identifier` at `position` on `source`, fading with distance, at volume
    /// and pitch 1 (`SimpleSoundInstance`'s positioned constructor).
    pub fn at(identifier: &str, source: SoundSource, position: [f64; 3]) -> Self {
        Self {
            identifier: identifier.to_owned(),
            source,
            volume: 1.0,
            pitch: 1.0,
            position,
            looping: false,
            delay: 0,
            attenuation: SoundAttenuation::Linear,
            relative: false,
        }
    }
    /// The same sound at `volume`.
    pub fn with_volume(self, volume: f32) -> Self {
        Self { volume, ..self }
    }
    /// The same sound at `pitch`.
    pub fn with_pitch(self, pitch: f32) -> Self {
        Self { pitch, ..self }
    }
    /// The same sound looping, `delay` ticks between its repeats.
    pub fn looping(self, delay: i32) -> Self {
        Self {
            looping: true,
            delay,
            ..self
        }
    }
}

/// A sound a client mod started with `play_local_sound`, to follow and control: `state`, `stop`
/// and `fade`. Each handle names one play and is never reused. Once the sound has ended, been
/// stopped or been cleared (leaving or changing levels stops every sound), the handle is stale: it
/// reads [`SoundState::Stopped`] and its writes do nothing. It stays valid across a change of
/// output device, which keeps the sounds playing.
///
/// Dropping or overwriting the handle doesn't stop the sound: keep a looping sound's handle while
/// it plays (stop it before starting another in its place), or nothing can stop it until the
/// player leaves the level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its field is private, so mods cannot build or destructure it
pub struct SoundRef {
    pub(crate) id: u64,
}

/// Where a mod's sound is ([`SoundRef`]'s `state`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SoundState {
    /// Not started yet: queued for the next audio update, waiting out `play_local_sound`'s
    /// `after` ticks, or between a looping sound's repeats (`SoundPlay::looping`'s `delay`).
    Waiting,
    /// Playing (`SoundEngine.isActive`).
    Playing,
    /// Over: it ended, was stopped, faded to 0 or was cleared. It never plays again.
    Stopped,
}

/// A playing sound's channel, for `play_sound_source` and
/// `play_streaming_source`: an id only, to tell sounds apart across the two
/// hooks. It gives no control over the channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ChannelRef {
    /// The channel's id, unique among the live ones.
    pub id: u64,
}

/// A server explosion about to happen, for `explosion_start`
/// (`ServerLevel.explode`'s `ServerExplosion`).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct NativeExplosionStart<'a> {
    /// The level it happens in (`minecraft:overworld`).
    pub dimension: &'a str,
    /// The explosion's center in blocks.
    pub center: [f64; 3],
    /// The explosion's radius in blocks.
    pub radius: f32,
    /// It sets fires (`createFire`).
    pub fire: bool,
    /// What the explosion does to blocks.
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
    /// Blocks are left alone.
    Keep,
    /// Blocks are destroyed and drop everything.
    Destroy,
    /// Blocks are destroyed and their drops may decay.
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
    /// The position.
    pub position: [f64; 3],
    /// The items the fishing loot table rolled, which the player receives
    /// (`getDrops`: they can't be changed here; loot tables change them).
    pub drops: &'a [ItemStack],
}

/// A replace hook's answer with the mod that gave it: the dispatch fills in `mod_id` (the
/// answering mod's `[mod] id`), so no mod can answer for another.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Answered<T> {
    /// The replace hook's answer.
    pub answer: T,
    /// The `[mod] id` of the mod that answered.
    pub mod_id: &'static str,
}

/// What a mod answers to `screen_opening` (`ScreenEvent.Opening`): the screen
/// that was about to open does not. The first mod to answer wins.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScreenOpening {
    /// `Opening.setCanceled(true)`: the current screen stays open.
    Cancel,
    /// `Opening.setNewScreen(..)`: the answering mod's own screen
    /// (`Host::mod_screen_open`) opens instead, if it can open there (not over
    /// the title screen, a book, a prompt or while a world loads; the
    /// screen that was about to open does then).
    ModScreen,
}

/// The stable identifier reported by screen hooks for an observed screen.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ScreenId {
    /// The same identifier as [`NativeScreenView::id`], such as `inventory` or `paused`.
    pub id: String,
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
    /// window's top left), as `cursor` gives it: for hover highlights.
    pub mouse: [f32; 2],
    /// This frame's partial tick (`getGameTimeDeltaPartialTick(false)`, 0 to
    /// 1): what the screen draws animation against.
    pub partial_tick: f32,
}

/// A depth-tested outline of the client's block selection shape.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct NativeBlockRim {
    /// The block whose outline is drawn.
    pub pos: [i32; 3],
    /// The outline colour as `0xAARRGGBB`.
    pub argb: u32,
    /// The line width in pixels.
    pub width: f32,
}
impl NativeBlockRim {
    /// An outline of the block at `pos` in `argb`, `width` pixels wide.
    pub const fn new(pos: [i32; 3], argb: u32, width: f32) -> Self {
        Self { pos, argb, width }
    }
}

/// A camera-local visual light, with no gameplay light or shadow writes.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct NativeColoredLight {
    /// The light's position in blocks.
    pub at: [f64; 3],
    /// The colour as `0xRRGGBB`.
    pub rgb: u32,
    /// How far it reaches, in blocks.
    pub radius: f32,
    /// The light's strength.
    pub intensity: f32,
}
impl NativeColoredLight {
    /// A light at `at` in `rgb`, reaching `radius` blocks at strength `intensity`.
    pub const fn new(at: [f64; 3], rgb: u32, radius: f32, intensity: f32) -> Self {
        Self {
            at,
            rgb,
            radius,
            intensity,
        }
    }
}

/// The minimap a native mod shows (`native_minimap`), drawn by Horizon's
/// minimap: its corner, its side in GUI pixels, blocks per map pixel,
/// turning with the player or north up, round or square.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct NativeMinimap {
    /// The screen corner the map sits in.
    pub corner: Corner,
    /// The map's side length in GUI pixels.
    pub size: f32,
    /// How many blocks one pixel covers; larger zooms out.
    pub blocks_per_pixel: f32,
    /// Turn the map with the player's facing instead of keeping north up.
    pub rotate: bool,
    /// Draw the map as a circle instead of a square.
    pub round: bool,
}

/// A mod block's screen over its menu (`native_screen`): its size, where
/// each of the container's slots is (`Slot.x`, `Slot.y`; none hides the
/// slot, and so do slots past the list; the server's menu does not have a
/// hidden slot either, so clicks and shift-clicks skip it), and the player inventory's first
/// row (vanilla's is 84; the hotbar is 58 below it) and left column, from
/// its top left.
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: mods build it with a literal and the game reads every field
pub struct NativeScreen {
    /// Vanilla's container screens are 176 wide.
    pub width: f32,
    /// The screen's height in GUI pixels; vanilla's is 166.
    pub height: f32,
    /// Each menu slot's top-left (x, y), or `None` to hide it.
    pub slots: Vec<Option<[f32; 2]>>,
    /// The y of the player inventory's first row; vanilla's is 84.
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

/// An item the panel shows (`panel_items`): its id
/// (`minecraft:diamond`), a count (1 draws none) and its components beyond
/// the item's own, in vanilla's item syntax, or "".
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct NativeStack {
    /// The item id, such as `minecraft:stick`.
    pub item: String,
    /// How many are in the stack.
    pub count: u32,
    /// The stack's data components, as the game's JSON form.
    pub components: String,
}

/// What the player did on a native mod's panel (`native_panel_event`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum NativePanelEvent {
    /// A click on an item of the panel, by its index in
    /// `panel_items`' list.
    Clicked {
        /// The item's index in the panel's list.
        index: u32,
        /// Whether it was a right click.
        right: bool,
    },
    /// The search box's text, each time the player changes it.
    Search(String),
    /// A physical key (`key.keyboard.r`) went down
    /// while the pointer was on an item: a slot of the screen's, or the
    /// panel's.
    Key {
        /// The key's name, such as `key.keyboard.r`.
        key: String,
        /// The item the pointer was on.
        stack: NativeStack,
    },
    /// A click on a slot of a recipe the panel shows: the recipe's index in
    /// `native_panel_recipes`' list and the slot's in it (the results come
    /// after the slots), and the stack it showed then.
    SlotClicked {
        /// The recipe's index in the panel's list.
        recipe: u32,
        /// The slot's index in the recipe; results come after the slots.
        slot: u32,
        /// The stack the slot showed then.
        stack: NativeStack,
        /// Whether it was a right click.
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
// exhaustive: the game builds it with a literal; mods only read it
pub struct NativeZoom {
    /// The zoom as a multiple of the normal view; 1 is no zoom.
    pub factor: f32,
    /// Whether the camera eases into the zoom.
    pub smooth: bool,
    /// Scale applied to mouse turning while zoomed.
    pub turn: f32,
}

/// A player on the server, as a server hook sees them.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NativeServerPlayer {
    /// The player's UUID.
    pub uuid: u128,
    /// The player's name.
    pub name: String,
    /// The operator level, 0 to 4 (`ops.json`; single player's owner with
    /// cheats on is 4).
    pub permission: u8,
    /// `minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`.
    pub dimension: String,
    /// The player's position in blocks.
    pub position: [f64; 3],
    /// 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: i32,
    /// Health points, 20 at full; 0 once dead.
    pub health: f32,
    /// Where they look, in degrees (`yRot`, `xRot`), as their last move told
    /// the server.
    pub yaw: f32,
    /// The player's pitch in degrees.
    pub pitch: f32,
    /// The network entity id the player has this session, the one hooks that only know entity ids
    /// (`living_incoming_damage`, `living_knockback`) give for the player; 0 when not known.
    /// API @API:server_player_entity_id@.
    pub entity_id: i32,
    /// The IP address the player connects from, without the port (behind
    /// a Velocity proxy, the address the proxy forwards; empty when not
    /// known).
    pub address: String,
}

/// A command a native mod adds, as one usage: words, then arguments
/// `<name:kind>`, optional ones last in brackets `[name:kind]`:
/// `pay <player:player> <amount:integer>`, `balance [player:player]`,
/// `eco give <player:player> <amount:integer>`. Kinds: `word`, `text` (the
/// rest of the line, last), `integer`, `number`, `boolean`, `player`,
/// `item`, `block_pos`. Usages of one command share their words; the
/// server merges them into one tree. Each executable usage keeps its own level:
/// a public `foo bar` lets everyone traverse `foo`, but does not make an
/// operator-only `foo` executable. Optional arguments also allow execution at
/// their preceding prefixes, at that usage's level.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NativeCommand {
    /// The usage.
    pub usage: String,
    /// The operator level a player needs to run this usage (0: everyone).
    /// Shared prefixes stay visible when needed to reach a lower-level usage.
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
    /// A single-player game with no server thread; never fires today.
    LocalAuthoritative,
    /// The server thread of a single-player or LAN world.
    IntegratedServer,
    /// A dedicated server.
    DedicatedServer,
    /// A client of someone else's server; never fires today.
    RemoteClient,
}

/// Supported vanilla `Block.UPDATE_*` flags for a level block write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct NativeBlockUpdateFlags(i32);

impl NativeBlockUpdateFlags {
    /// No update flags.
    pub const NONE: Self = Self(0);
    /// Notify neighboring blocks.
    pub const NEIGHBORS: Self = Self(1);
    /// Send the block update to clients.
    pub const CLIENTS: Self = Self(2);
    /// Suppress the client-side block update notification.
    pub const INVISIBLE: Self = Self(4);
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
    // hidden: a game-side helper, not part of the mod surface
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
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct NativeWorldEpoch(u64);

impl NativeWorldEpoch {
    /// Create an epoch for the game.
    // hidden: a game-side helper, not part of the mod surface
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
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct EntityRef {
    epoch: NativeWorldEpoch,
    id: u32,
}

impl EntityRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
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

/// The resolved holder of a mob's leash, or its absence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MobLeashHolder {
    /// The mob is not leashed.
    Unleashed,
    /// A player, mob or boat holds the leash.
    Entity(EntityRef),
    /// A fence knot holds the leash at this block.
    Knot([i32; 3]),
}

/// A living entity of the open world: a mob, a player, an armor stand or the
/// dragon (vanilla's `LivingEntity`). `host.living(entity)` makes one from an
/// [`EntityRef`] of such an entity, and [`LivingRef::entity`] goes back, so the
/// entity's methods work on it too. Stale handles read as `None`, as an
/// `EntityRef`'s do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct LivingRef(EntityRef);

impl LivingRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
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

/// How an attribute modifier changes its attribute
/// (`AttributeModifier.Operation`). The value is the base plus every
/// `AddValue`, then grows by that sum times each `AddMultipliedBase`, then is
/// multiplied by one plus each `AddMultipliedTotal`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttributeOperation {
    /// `add_value`: adds the amount.
    AddValue,
    /// `add_multiplied_base`: adds the amount times the base.
    AddMultipliedBase,
    /// `add_multiplied_total`: multiplies by one plus the amount.
    AddMultipliedTotal,
}

/// An attribute modifier (vanilla's `AttributeModifier`): its id, which is
/// unique on one attribute of one entity, the amount and the operation.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeModifier {
    /// The modifier's id (`mymod:sprint_boost`).
    pub id: Identifier,
    /// The amount the operation applies.
    pub amount: f64,
    /// How it applies.
    pub operation: AttributeOperation,
}

impl AttributeModifier {
    /// A modifier with id `id`.
    pub fn new(id: Identifier, amount: f64, operation: AttributeOperation) -> Self {
        Self {
            id,
            amount,
            operation,
        }
    }
}

/// Whether an attribute modifier is saved with its entity
/// (`AttributeInstance.addPermanentModifier`) or lasts until the entity
/// unloads (`addTransientModifier`), as the modifiers of worn items and
/// effects do.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModifierLifetime {
    /// Not saved: gone when the entity is saved and loaded again.
    Transient,
    /// Saved with the entity.
    Permanent,
}

/// The slots an item's attribute modifier applies in (vanilla's
/// `EquipmentSlotGroup`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EquipmentSlotGroup {
    /// `any`: every slot.
    Any,
    /// `mainhand`.
    MainHand,
    /// `offhand`.
    OffHand,
    /// `hand`: either hand.
    Hand,
    /// `feet`.
    Feet,
    /// `legs`.
    Legs,
    /// `chest`.
    Chest,
    /// `head`.
    Head,
    /// `armor`: feet, legs, chest, head and body.
    Armor,
    /// `body`: an animal's armor slot.
    Body,
    /// `saddle`.
    Saddle,
}

/// One attribute modifier an item stack grants (vanilla's
/// `ItemAttributeModifiers.Entry`): the attribute (`minecraft:armor`), the
/// modifier, and the slots it applies in.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct ItemAttributeModifier {
    /// The attribute's id.
    pub attribute: Identifier,
    /// The modifier.
    pub modifier: AttributeModifier,
    /// The slots it applies in.
    pub slot: EquipmentSlotGroup,
}

impl ItemAttributeModifier {
    /// `modifier` on `attribute`, in the slots of `slot`.
    pub fn new(
        attribute: Identifier,
        modifier: AttributeModifier,
        slot: EquipmentSlotGroup,
    ) -> Self {
        Self {
            attribute,
            modifier,
            slot,
        }
    }
}

/// The attribute modifiers of one item stack, as `item_attribute_modifier`
/// edits them (`ItemAttributeModifierEvent`'s list). It starts as the
/// stack's own `minecraft:attribute_modifiers`, or its item's, in order.
// exhaustive: its fields are private, so mods only edit it through its methods
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemAttributeModifiers {
    entries: Vec<ItemAttributeModifier>,
    changed: bool,
}

impl ItemAttributeModifiers {
    /// The modifiers, in order.
    pub fn iter(&self) -> std::slice::Iter<'_, ItemAttributeModifier> {
        self.entries.iter()
    }
    /// How many modifiers there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// `addModifier`: adds `modifier` on `attribute` in the slots of `slot`,
    /// unless `attribute` already has a modifier with its id. Whether it
    /// was added.
    pub fn add(
        &mut self,
        attribute: Identifier,
        modifier: AttributeModifier,
        slot: EquipmentSlotGroup,
    ) -> bool {
        if self.find(&attribute, &modifier.id).is_some() {
            return false;
        }
        self.entries
            .push(ItemAttributeModifier::new(attribute, modifier, slot));
        self.changed = true;
        true
    }
    /// `replaceModifier`: puts `modifier` on `attribute` in the slots of
    /// `slot`, in place of the one with its id, or at the end.
    pub fn replace(
        &mut self,
        attribute: Identifier,
        modifier: AttributeModifier,
        slot: EquipmentSlotGroup,
    ) {
        let entry = ItemAttributeModifier::new(attribute, modifier, slot);
        match self.find(&entry.attribute, &entry.modifier.id) {
            Some(i) => self.entries[i] = entry,
            None => self.entries.push(entry),
        }
        self.changed = true;
    }
    /// `removeModifier`: removes the modifier `id` of `attribute`. Whether
    /// there was one.
    pub fn remove(&mut self, attribute: &Identifier, id: &Identifier) -> bool {
        let Some(i) = self.find(attribute, id) else {
            return false;
        };
        self.entries.remove(i);
        self.changed = true;
        true
    }
    /// `removeIf`: keeps only the modifiers `keep` accepts. Whether any went.
    pub fn retain(&mut self, mut keep: impl FnMut(&ItemAttributeModifier) -> bool) -> bool {
        let len = self.entries.len();
        self.entries.retain(|entry| keep(entry));
        let removed = self.entries.len() != len;
        self.changed |= removed;
        removed
    }
    /// `removeAllModifiersFor`: removes every modifier of `attribute`.
    /// Whether any went.
    pub fn remove_all_for(&mut self, attribute: &Identifier) -> bool {
        self.retain(|entry| entry.attribute != *attribute)
    }
    /// `clearModifiers`: removes every modifier.
    pub fn clear(&mut self) {
        self.changed |= !self.entries.is_empty();
        self.entries.clear();
    }
    fn find(&self, attribute: &Identifier, id: &Identifier) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.attribute == *attribute && e.modifier.id == *id)
    }
}

/// The enchantments `get_enchantment_level` (`GetEnchantmentLevelEvent`)
/// hands a mod: NeoForge's `ItemEnchantments.Mutable`, each enchantment's id
/// (`minecraft:sharpness`) with its level, in order. Private fields, edited
/// through methods, so the game knows whether a mod changed them.
// exhaustive: its fields are private, so mods only edit it through its methods
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemEnchantments {
    entries: Vec<(Identifier, u32)>,
    changed: bool,
}

impl ItemEnchantments {
    /// The highest level an enchantment can have (`ItemEnchantments.MAX_LEVEL`).
    pub const MAX_LEVEL: u32 = 255;

    /// The enchantments and their levels, in order (`keySet` and `getLevel`).
    pub fn iter(&self) -> impl Iterator<Item = (&Identifier, u32)> {
        self.entries.iter().map(|(id, level)| (id, *level))
    }
    /// How many enchantments there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// `getLevel`: `enchantment`'s level, 0 when it isn't there.
    pub fn level(&self, enchantment: &Identifier) -> u32 {
        self.find(enchantment).map_or(0, |i| self.entries[i].1)
    }
    /// `set`: gives `enchantment` `level`, at most [`Self::MAX_LEVEL`]; 0
    /// removes it.
    pub fn set(&mut self, enchantment: Identifier, level: u32) {
        let level = level.min(Self::MAX_LEVEL);
        match (self.find(&enchantment), level) {
            (Some(i), 0) => {
                self.entries.remove(i);
            }
            (Some(i), _) if self.entries[i].1 == level => return,
            (Some(i), _) => self.entries[i].1 = level,
            (None, 0) => return,
            (None, _) => self.entries.push((enchantment, level)),
        }
        self.changed = true;
    }
    /// `upgrade`: raises `enchantment` to `level` (at most
    /// [`Self::MAX_LEVEL`]) when that is higher than its own; 0 does nothing.
    pub fn upgrade(&mut self, enchantment: Identifier, level: u32) {
        if level > self.level(&enchantment) {
            self.set(enchantment, level);
        }
    }
    /// `removeIf`: keeps only the enchantments `keep` accepts. Whether any
    /// went.
    pub fn retain(&mut self, mut keep: impl FnMut(&Identifier, u32) -> bool) -> bool {
        let len = self.entries.len();
        self.entries.retain(|(id, level)| keep(id, *level));
        let removed = self.entries.len() != len;
        self.changed |= removed;
        removed
    }
    fn find(&self, enchantment: &Identifier) -> Option<usize> {
        self.entries.iter().position(|(id, _)| id == enchantment)
    }
}

/// A living entity's active effect (vanilla's `MobEffectInstance`), as
/// [`LivingRef`] reads its effects and as `add_effect` gives one: the
/// effect's id, an [`Identifier`] namespaced as the registry has it
/// (`minecraft:speed`, a content mod's `mod:custom`),
/// the ticks left (-1 is infinite), the amplifier from 0, and the flags a
/// beacon-style effect carries: `ambient` (an ambient one gives no
/// particles) and `visible` (whether its particles show). Horizon keeps no
/// separate show-icon flag: the icon always shows.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NativeActiveEffect {
    /// The effect's load-owned registry key; `id.id(host)` reads its canonical name.
    pub id: EffectKey,
    /// The ticks left; -1 is infinite.
    pub duration: i32,
    /// The amplifier, from 0.
    pub amplifier: i32,
    /// `MobEffectInstance.isAmbient`: an ambient effect shows no particles.
    pub ambient: bool,
    /// `MobEffectInstance.isVisible`: whether its particles show.
    pub visible: bool,
}

impl NativeActiveEffect {
    /// Construct an effect with its duration, amplifier and visual flags.
    pub fn new(id: EffectKey, duration: i32, amplifier: i32, ambient: bool, visible: bool) -> Self {
        Self {
            id,
            duration,
            amplifier,
            ambient,
            visible,
        }
    }
}

/// The answer to `mob_effect_applicable`: whether an effect can land on an
/// entity. An unanswered hook defers to the vanilla tag check; `Allow` and
/// `Deny` override it, as NeoForge's `MobEffectEvent.Applicable.Result`
/// does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EffectApplicable {
    /// The effect lands, whatever the tags say.
    Allow,
    /// The effect is refused, whatever the tags say.
    Deny,
}

/// A vanilla `Mob`, narrowed from an entity with `host.mob(entity)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct MobRef(EntityRef);

impl MobRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
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

/// The player, or mobs of the given species: the class filter of a
/// [`NativeGoal::NearestTarget`] and of a [`NativeGoal::AvoidEntity`]. The
/// species ids are namespaced (`minecraft:zombie`); a bare id reads as
/// `minecraft:`'s, like the effects' ids.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum NativeMobFilter {
    /// The nearest player.
    Player,
    /// Mobs of these species (`Vec` of namespaced ids).
    Species(Vec<String>),
}

/// A vanilla AI goal, as a mob's `add_goal` and `add_target_goal` give one:
/// only the kinds Horizon's own species code runs, with the vanilla
/// constructor's parameters (`MeleeAttackGoal(speed, followUnseen)`). Goals
/// with behavior of their own (a `MoveToBlockGoal` subclass's block test)
/// stay out: they take code, not parameters.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum NativeGoal {
    /// `FloatGoal`: swim up in water or lava. Vanilla parameters: none.
    Float,
    /// `ClimbOnTopOfPowderSnowGoal`: jump onto powder snow. Parameters: none.
    ClimbOnTopOfPowderSnow,
    /// `PanicGoal(speed)`, or `TurtlePanicGoal(speed)` with `water_range`
    /// set: always head for water this far away, not only when on fire.
    Panic {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
        /// Search distance for water in blocks, as `TurtlePanicGoal`'s.
        water_range: Option<i32>,
    },
    /// `RandomStrollGoal(speed, interval)`; `water_avoiding` set to its
    /// probability makes `WaterAvoidingRandomStrollGoal`, `swim`
    /// `RandomSwimmingGoal`, `flying` `WaterAvoidingRandomFlyingGoal`.
    Stroll {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
        /// Ticks between tries; the goal starts with chance 1 in `interval` each tick.
        interval: i32,
        /// Probability, 0 to 1, of preferring a spot away from water.
        water_avoiding: Option<f32>,
        /// Swim around in water instead of walking.
        swim: bool,
        /// Pick flying destinations.
        flying: bool,
    },
    /// `LookAtPlayerGoal(range, probability, onlyHorizontal)`: the nearest
    /// player, the nearest mob with `mobs`, or only mobs of `only`'s
    /// species.
    LookAtPlayer {
        /// Distance in blocks the goal looks for something to watch.
        range: f32,
        /// Chance, 0 to 1, each tick that the goal starts.
        probability: f32,
        /// Turn the head only sideways, never up or down.
        only_horizontal: bool,
        /// Also watch the nearest mob, not only the nearest player.
        mobs: bool,
        /// A mob type id (`minecraft:cow`) that restricts the mobs watched to that species.
        only: Option<String>,
    },
    /// `RandomLookAroundGoal`. Vanilla parameters: none.
    RandomLookAround,
    /// `MeleeAttackGoal(speed, followUnseen)`.
    MeleeAttack {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
        /// Keep chasing a target the mob can no longer see.
        follow_unseen: bool,
    },
    /// `LeapAtTargetGoal(leapMotion)`.
    LeapAtTarget {
        /// The upward speed of the leap, in blocks per tick.
        yd: f32,
    },
    /// `AvoidEntityGoal(what, maxDist, walk, sprint)`: sprint away from
    /// `what` once it is closer than `max_dist`.
    AvoidEntity {
        /// Which entities the mob avoids.
        what: NativeMobFilter,
        /// Distance in blocks within which the mob flees.
        max_dist: f32,
        /// Speed multiplier while fleeing at a distance.
        walk: f64,
        /// Speed multiplier while fleeing close by.
        sprint: f64,
    },
    /// `TemptGoal(speed, items, canScare)`: `items` are item ids
    /// (`minecraft:wheat`; a bare id reads as `minecraft:`'s).
    Tempt {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
        /// Item ids that tempt the mob.
        items: Vec<String>,
        /// Whether a moving player holding the item scares the mob off.
        can_scare: bool,
    },
    /// `BreedGoal(speed)`. An animal keeps it; other mobs refuse it.
    Breed {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
    },
    /// `FollowParentGoal(speed)`. An animal keeps it; other mobs refuse it.
    FollowParent {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
    },
    /// `FollowMobGoal(speed, stopDistance, areaSize)`.
    FollowMob {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
        /// Distance in blocks at which the mob stops following.
        stop: f32,
        /// Radius in blocks searched for a mob to follow.
        area: f32,
    },
    /// `RestrictSunGoal`. Vanilla parameters: none.
    RestrictSun,
    /// `FleeSunGoal(speed)`.
    FleeSun {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
    },
    /// `EatBlockGoal`. Vanilla parameters: none.
    EatBlock,
    /// `BreathAirGoal`. Vanilla parameters: none.
    BreathAir,
    /// `TryFindWaterGoal`. Vanilla parameters: none.
    TryFindWater,
    /// `NearestAttackableTargetGoal` (the target selector): hunt `prey`,
    /// checking every `interval` ticks, `must_see` (`mustSee`) and
    /// `must_reach` (`mustReach`).
    NearestTarget {
        /// Which entities the mob hunts.
        prey: NativeMobFilter,
        /// Only pick a target the mob can see.
        must_see: bool,
        /// Only pick a target the mob can path to.
        must_reach: bool,
        /// Ticks between tries; the goal starts with chance 1 in `interval` each tick.
        interval: i32,
    },
    /// `HurtByTargetGoal`, `alert` as its `setAlert()`.
    HurtByTarget {
        /// Also alert nearby mobs of the same kind, as `setAlert()`.
        alert: bool,
    },
}

impl NativeGoal {
    /// The goal's kind id: the same snake_case spelling `goals` and
    /// `target_goals` read and `remove_goal` and `remove_target_goal` take
    /// (`melee_attack`). A species variant of a vanilla goal (a bee's
    /// `BeeHurtByOtherGoal`) keeps its own vanilla name instead.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::ClimbOnTopOfPowderSnow => "climb_on_top_of_powder_snow",
            Self::Panic { .. } => "panic",
            Self::Stroll { .. } => "stroll",
            Self::LookAtPlayer { .. } => "look_at_player",
            Self::RandomLookAround => "random_look_around",
            Self::MeleeAttack { .. } => "melee_attack",
            Self::LeapAtTarget { .. } => "leap_at_target",
            Self::AvoidEntity { .. } => "avoid_entity",
            Self::Tempt { .. } => "tempt",
            Self::Breed { .. } => "breed",
            Self::FollowParent { .. } => "follow_parent",
            Self::FollowMob { .. } => "follow_mob",
            Self::RestrictSun => "restrict_sun",
            Self::FleeSun { .. } => "flee_sun",
            Self::EatBlock => "eat_block",
            Self::BreathAir => "breath_air",
            Self::TryFindWater => "try_find_water",
            Self::NearestTarget { .. } => "nearest_target",
            Self::HurtByTarget { .. } => "hurt_by_target",
        }
    }
}

/// One goal of a mob's selector, as `goals` and `target_goals` read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct NativeMobGoalEntry {
    /// The goal's priority: a lower number preempts a higher one.
    pub priority: i32,
    /// The goal's kind id (see [`NativeGoal::id`]).
    pub id: &'static str,
    /// Whether the goal is running now.
    pub running: bool,
}

/// The ender dragon of the open world, narrowed with `host.dragon(entity)`.
/// It is also a [`MobRef`] and a [`LivingRef`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct DragonRef(EntityRef);

impl DragonRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
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
    /// Circling the end island (`DragonPhaseHoldingPattern`).
    HoldingPattern,
    /// Flying at a player and shooting fireballs.
    StrafePlayer,
    /// Flying toward the fountain to land.
    LandingApproach,
    /// Descending onto the fountain.
    Landing,
    /// Lifting off the fountain.
    Takeoff,
    /// Perched, breathing fire.
    SittingFlaming,
    /// Perched, looking for a target.
    SittingScanning,
    /// Perched, about to breathe.
    SittingAttacking,
    /// Charging a player in melee.
    ChargingPlayer,
    /// Dying, flying to the fountain.
    Dying,
    /// Hovering in place.
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
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ItemEntityRef(EntityRef);

impl ItemEntityRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
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
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct DisplayRef(EntityRef);

impl DisplayRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
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
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ProjectileRef(EntityRef);

impl ProjectileRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
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
/// narrowed from an entity with `host.merchant(entity)`. If a villager's saved
/// profession is unresolved, offers read empty and merchant mutations are
/// refused until its profession is available again; saved offers are retained.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct MerchantRef(EntityRef);

impl MerchantRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
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
    /// Block.
    Block,
    /// Item.
    Item,
    /// Text.
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
    /// Fixed.
    Fixed,
    /// Vertical.
    Vertical,
    /// Horizontal.
    Horizontal,
    /// Center.
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
    /// No transform; the model as authored.
    None,
    /// Held in the left hand, seen from outside.
    ThirdPersonLeftHand,
    /// Held in the right hand, seen from outside.
    ThirdPersonRightHand,
    /// Held in the left hand, seen by its owner.
    FirstPersonLeftHand,
    /// Held in the right hand, seen by its owner.
    FirstPersonRightHand,
    /// Worn on the head.
    Head,
    /// Drawn in an inventory slot.
    Gui,
    /// Dropped as an item entity.
    Ground,
    /// Hung in an item frame.
    Fixed,
    /// Standing on a shelf.
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
    /// Center.
    Center,
    /// Left.
    Left,
    /// Right.
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
// exhaustive: the game builds it with a literal; mods only read it
pub struct NativeTransformation {
    /// The offset in blocks.
    pub translation: [f32; 3],
    /// The rotation applied after the scale, as a quaternion (x, y, z, w).
    pub left_rotation: [f32; 4],
    /// The scale on each axis.
    pub scale: [f32; 3],
    /// The rotation applied before the scale, as a quaternion (x, y, z, w).
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
    /// Disallowed.
    Disallowed,
    /// Allowed.
    Allowed,
    /// Creative only.
    CreativeOnly,
}

/// Why a projectile's pierce level is unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PierceLevelError {
    /// The host is not a logical server.
    WrongSide,
    /// The projectile handle is stale or unavailable.
    Stale,
    /// The projectile has no arrow state.
    NotArrow,
}

impl NoBackend for PierceLevelError {
    fn no_backend() -> Self {
        Self::Stale
    }
}

/// A vanilla `AbstractMinecart`, narrowed from an entity with `host.minecart(entity)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct MinecartRef(EntityRef);

impl MinecartRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
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
// exhaustive: its fields are private, so mods cannot build or destructure it
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
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct LevelRef {
    epoch: NativeWorldEpoch,
    dimension: DimensionKey,
}

impl LevelRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(epoch: NativeWorldEpoch, dimension: DimensionKey) -> Self {
        Self { epoch, dimension }
    }
    /// The level's dimension. `level.dimension().id(host)` gives its name.
    pub fn dimension(self) -> DimensionKey {
        self.dimension
    }
    /// The name of the level's dimension, for the game's own tables.
    // hidden: a game-side helper; mods ask the host
    #[doc(hidden)]
    pub fn dimension_id(self) -> &'static str {
        registry_keys::dimension_id(self.dimension).expect("the game's table made the key")
    }
    /// Draw from the level's own random stream (vanilla's `Level.random`), in vanilla's order:
    /// `draw` gets the stream and the level keeps it after. None where `random` is None.
    pub fn with_random<T>(
        self,
        host: &mut dyn ServerHost,
        draw: impl FnOnce(&mut LegacyRandom) -> T,
    ) -> Option<T> {
        let mut random = self.random(host)?;
        let drawn = draw(&mut random);
        self.set_random(host, random);
        Some(drawn)
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
/// for a handle of an earlier open, for a closed level, and on a client.
/// Reads and writes reach parked server levels without moving them into place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct RaidRef {
    level: LevelRef,
    id: i32,
}

impl RaidRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
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
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ChunkRef {
    pub(crate) level: LevelRef,
    pub(crate) pos: [i32; 2],
}

impl ChunkRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
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

/// A kind of light (`LightLayer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: a closed set the game matches without a wildcard
pub enum LightLayer {
    /// Light from emitting blocks (`BLOCK`).
    Block,
    /// Light from the sky (`SKY`).
    Sky,
}

/// The feature flags a world enables (`FeatureFlagSet`, `LevelReader.enabledFeatures`), each
/// as its id (`minecraft:vanilla`, `minecraft:redstone_experiments`), shared with the world.
/// A world that names none enables `minecraft:vanilla` alone, as vanilla's default does.
/// Getting and reading it allocates nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
#[horizon_mod_api_macros::wasm(via = Vec<String>)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct FeatureFlags(Arc<[String]>);

impl FeatureFlags {
    /// The flags of a world's `enabled_features` list; an empty list is vanilla's default.
    pub fn new(ids: Arc<[String]>) -> Self {
        Self(ids)
    }
    /// `FeatureFlagSet.contains`: whether the flag `flag` (`minecraft:redstone_experiments`, or
    /// `redstone_experiments`) is enabled.
    pub fn contains(&self, flag: &str) -> bool {
        let bare = !flag.contains(':');
        self.iter()
            .any(|id| id == flag || (bare && id.strip_prefix("minecraft:") == Some(flag)))
    }
    /// The enabled flag ids.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        let ids = &self.0;
        ids.iter()
            .map(String::as_str)
            .chain(ids.is_empty().then_some("minecraft:vanilla"))
    }
}

/// The flags as their ids, the form they cross into a wasm mod as.
impl From<&FeatureFlags> for Vec<String> {
    fn from(flags: &FeatureFlags) -> Self {
        flags.0.to_vec()
    }
}

impl From<Vec<String>> for FeatureFlags {
    fn from(ids: Vec<String>) -> Self {
        Self(ids.into())
    }
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
    /// All chat.
    Full,
    /// Only system messages and commands.
    System,
    /// No chat.
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
    /// The left hand.
    Left,
    /// The right hand.
    Right,
}

/// `ParticleStatus`: how many particles a client shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeParticleStatus {
    /// All particles.
    All,
    /// Fewer particles.
    Decreased,
    /// Only the essential particles.
    Minimal,
}

/// A player's client options, as `ServerboundClientInformationPacket` carries them
/// (vanilla's `ClientInformation`). Raw ids map as vanilla's `ByIdMap` does: chat
/// visibility and particle status wrap (`floorMod(id, 3)`), the main hand reads an
/// unknown id as left.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct NativeClientInfo {
    /// The client's language code, such as `en_us`.
    pub language: String,
    /// The client's view distance in chunks.
    pub view_distance: i8,
    /// Which chat the client shows.
    pub chat_visibility: NativeChatVisibility,
    /// Whether the client shows chat colours.
    pub chat_colors: bool,
    /// Skin layer bits (`PlayerModelPart`).
    pub model_customisation: u8,
    /// The player's main hand.
    pub main_hand: NativeMainHand,
    /// Whether the client asked for filtered text.
    pub text_filtering: bool,
    /// Whether the player shows in the server list sample.
    pub allows_listing: bool,
    /// How many particles the client draws.
    pub particle_status: NativeParticleStatus,
}

/// A block face for a level support query.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeBlockFace {
    /// Facing down (-y).
    Down,
    /// Facing up (+y).
    Up,
    /// Facing north (-z).
    North,
    /// Facing south (+z).
    South,
    /// Facing west (-x).
    West,
    /// Facing east (+x).
    East,
}

/// The support area tested on a block face.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeSupportType {
    /// Full.
    Full,
    /// Center.
    Center,
    /// Rigid.
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

/// The category of an entity's type (`MobCategory`, `EntityType.getCategory`):
/// what natural spawning counts it as. Every kind that isn't a mob is `Misc`,
/// except the ender dragon, a `Monster`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MobCategory {
    /// `MONSTER`.
    Monster,
    /// `CREATURE`.
    Creature,
    /// `AMBIENT`.
    Ambient,
    /// `AXOLOTLS`.
    Axolotls,
    /// `UNDERGROUND_WATER_CREATURE`.
    UndergroundWaterCreature,
    /// `WATER_CREATURE`.
    WaterCreature,
    /// `WATER_AMBIENT`.
    WaterAmbient,
    /// `MISC`.
    Misc,
}

/// Why a mob spawns (`EntitySpawnReason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeSpawnReason {
    /// `NATURAL`: the level's own spawning each tick.
    Natural,
    /// `CHUNK_GENERATION`: a new chunk's creature packs.
    ChunkGeneration,
    /// `SPAWNER`: a mob spawner block.
    Spawner,
    /// `TRIAL_SPAWNER`: a trial spawner.
    TrialSpawner,
    /// `REINFORCEMENT`: a zombie calling for help.
    Reinforcement,
    /// `PATROL`: a pillager patrol.
    Patrol,
    /// `STRUCTURE`: a structure's own mobs.
    Structure,
    /// `EVENT`: a world event, such as a village siege.
    Event,
    /// `COMMAND`: `/summon`.
    Command,
    /// `SPAWN_ITEM_USE`: a spawn egg.
    SpawnItemUse,
    /// `MOB_SUMMONED`: a mob that summons another (an evoker's vexes).
    MobSummoned,
    /// `CONVERSION`: a mob that became another.
    Conversion,
    /// `BREEDING`: a baby.
    Breeding,
    /// `TRIGGERED`: a block or item set it off (a sculk shrieker's warden).
    Triggered,
    /// `JOCKEY`: a rider spawned on a mount.
    Jockey,
    /// `DISPENSER`: a dispenser fired a spawn egg.
    Dispenser,
    /// `BUCKET`: a bucketed mob released.
    Bucket,
    /// `LOAD`: loaded from saved data.
    Load,
    /// `DIMENSION_TRAVEL`: a mob that crossed a portal.
    DimensionTravel,
}

/// The local difficulty at a spawn (`DifficultyInstance`): what `finalize_spawn` reads and
/// may change before the mob is finalized.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct NativeDifficulty {
    /// `Difficulty.getId`: 0 peaceful, 1 easy, 2 normal, 3 hard. A larger value is taken as 3.
    pub base: u8,
    /// `DifficultyInstance.getEffectiveDifficulty`: the base scaled by world age, the chunk's
    /// inhabited time and the moon.
    pub effective: f32,
}

impl NativeDifficulty {
    /// A difficulty of `base` and `effective`.
    pub fn new(base: u8, effective: f32) -> Self {
        Self { base, effective }
    }
}

/// What spawned a mob, where `EntitySpawnReason` alone does not say (`FinalizeSpawnEvent.getSpawner`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeSpawner {
    /// A spawner or trial spawner block at this position.
    Block(BlockPos),
}

/// What a mod answers a spawn check (`MobSpawnEvent.PositionCheck.Result` and
/// `MobSpawnEvent.SpawnPlacementCheck.Result`, one type for both).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeSpawnCheck {
    /// `SUCCEED`: the check passes.
    Succeed,
    /// `DEFAULT`: vanilla's own answer stands.
    #[default]
    Default,
    /// `FAIL`: the check fails.
    Fail,
}

/// What a mod answers a despawn check (`MobDespawnEvent.Result`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeDespawn {
    /// `ALLOW`: the mob despawns now.
    Allow,
    /// `DEFAULT`: vanilla's `Mob.checkDespawn` decides.
    #[default]
    Default,
    /// `DENY`: the mob does not despawn.
    Deny,
}

/// Block shape used by an empty-context level ray.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeClipBlock {
    /// Collider.
    Collider,
    /// Outline.
    Outline,
}

/// Fluid mode for a level ray. More modes are a follow-up.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeClipFluid {
    /// None.
    None,
}

/// Result of a block ray, including the vanilla miss result.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct NativeClipHit {
    /// Whether the ray hit nothing; the other fields are then the vanilla miss values.
    pub miss: bool,
    /// The hit block's position.
    pub block: [i32; 3],
    /// The face the ray entered.
    pub face: NativeBlockFace,
    /// The exact hit point in blocks.
    pub location: [f64; 3],
    /// Whether the ray started inside the block.
    pub inside: bool,
    /// Whether the hit is the world border.
    pub world_border: bool,
}

/// The client's ordinary crosshair hit (`Minecraft.hitResult`), within the
/// current interaction ranges. Entity handles belong to the current client
/// level's epoch, including dragon parts; a replacement level invalidates them.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum NativeCrosshairTarget {
    /// The crosshair is on a block.
    Block {
        /// The block's position.
        pos: BlockPos,
        /// The block's state.
        state: BlockStateId,
        /// Where on the block the ray hit.
        hit: BlockHit,
    },
    /// The crosshair is on an entity.
    Entity {
        /// The entity under the crosshair.
        entity: EntityRef,
        /// The hit point in blocks.
        point: [f64; 3],
    },
}

/// One item cooldown a player waits on (`ItemCooldowns.cooldowns`).
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
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
// exhaustive: its fields are private, so mods cannot build or destructure it
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

/// A custom boss bar (vanilla's `CustomBossEvent`, the `/bossbar` ones) of an
/// open server world. It holds the world's epoch and the bar's UUID, so a bar
/// that was removed (even if its id was added again) or a handle from an
/// earlier open reads as `None` and changes nothing. Mob and raid bars are not
/// custom boss events and have no handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BossBarRef {
    pub(crate) epoch: NativeWorldEpoch,
    pub(crate) uuid: u128,
}

/// `BossEvent.BossBarColor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BossBarColor {
    /// A pink bar.
    Pink,
    /// A blue bar.
    Blue,
    /// A red bar.
    Red,
    /// A green bar.
    Green,
    /// A yellow bar.
    Yellow,
    /// A purple bar.
    Purple,
    /// A white bar.
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
    /// Progress.
    Progress,
    /// Notched6.
    Notched6,
    /// Notched10.
    Notched10,
    /// Notched12.
    Notched12,
    /// Notched20.
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

/// One boss bar the client shows (vanilla's `LerpingBossEvent`), as
/// `customize_boss_event_progress` reads it. Any bar the server sends: mob,
/// raid and `/bossbar` ones alike.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ClientBossBar {
    /// The bar's UUID, as the server's boss event packets name it.
    pub uuid: u128,
    /// The title drawn above the bar.
    pub name: Text,
    /// The fill drawn this frame, 0 to 1 (`getProgress`, eased toward the
    /// server's value).
    pub progress: f32,
    /// The bar's color.
    pub color: BossBarColor,
    /// The notches drawn over it.
    pub overlay: BossBarOverlay,
    /// Whether it darkens the sky (`shouldDarkenScreen`).
    pub darken_screen: bool,
    /// Whether it plays the boss music (`shouldPlayBossMusic`).
    pub play_boss_music: bool,
    /// Whether it thickens the fog (`shouldCreateWorldFog`).
    pub create_world_fog: bool,
}

/// One player's open menu, invalidated on close, replacement or reconnect.
/// Client menu handles are also invalidated on respawn.
///
/// Clicks require a mutable client host, not a server or a read-only hook:
/// ```compile_fail
/// use mod_api::{MenuRef, ServerHost};
/// fn wrong(menu: MenuRef, host: &mut dyn ServerHost) {
///     menu.click(host, &[]);
/// }
/// ```
/// ```compile_fail
/// use mod_api::{MenuRef, ClientHost};
/// fn wrong(menu: MenuRef, host: &dyn ClientHost) {
///     menu.click(host, &[]);
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct MenuRef {
    pub(crate) player: PlayerRef,
    pub(crate) window: i32,
    pub(crate) generation: u64,
}

/// A player's food data.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct NativePlayerFood {
    /// The food level, 0 to 20.
    pub level: i32,
    /// The saturation, up to the food level.
    pub saturation: f32,
    /// The exhaustion; the game takes a saturation or food point at 4.
    pub exhaustion: f32,
}

/// A player's experience (`Player.experienceLevel`, `experienceProgress`,
/// `totalExperience`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct NativePlayerExperience {
    /// The experience level.
    pub level: i32,
    /// Progress toward the next level, from 0 to 1.
    pub progress: f32,
    /// The total experience points.
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

/// A change requested by an effect tick, applied after its hooks return.
// hidden: a game-side type, not part of the mod surface
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
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
    /// Takes no damage.
    pub invulnerable: bool,
    /// Is flying now.
    pub flying: bool,
    /// May start flying.
    pub may_fly: bool,
    /// Breaks blocks at once and uses no items (creative).
    pub instabuild: bool,
    /// May break and place blocks.
    pub may_build: bool,
    /// The flying speed.
    pub fly_speed: f32,
    /// Walking speed (`Abilities.walkingSpeed`, vanilla's 0.1).
    pub walk_speed: f32,
}

/// One block entity in an open level, identified by its lifecycle generation.
/// Removal, replacement or chunk reload invalidates the handle, even at the same
/// position. In-place data edits and temporary tick take/put preserve identity.
/// Stale handles read as `None`; Result-returning methods report `Stale`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BlockEntityRef {
    pub(crate) level: LevelRef,
    pub(crate) position: [i32; 3],
    pub(crate) generation: u64,
}

/// Why a block-entity operation could not resolve its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockEntityError {
    /// The entity was removed, replaced, unloaded or belongs to another level epoch.
    Stale,
}

impl NoBackend for BlockEntityError {
    fn no_backend() -> Self {
        Self::Stale
    }
}

/// Non-paused authoritative server ticks since open (not game or frame time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct NativeTickId(u64);

impl NativeTickId {
    /// Create a tick ID for the game.
    // hidden: a game-side helper, not part of the mod surface
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
    /// The open failed; `error_text` is the engine's own error text.
    WorldOpenFailed,
    /// World close final.
    WorldCloseFinal,
    /// The server loop or final save failed; `error_text` carries the error.
    WorldCloseFailed,
}

/// The tick phases carry whether the world runs, including frozen steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ServerTickPhase {
    /// The server tick is about to begin.
    ServerTickStart {
        /// False for a frozen step, when the world does not advance.
        world_running: bool,
    },
    /// The server tick has finished.
    ServerTickEnd {
        /// False for a frozen step, when the world does not advance.
        world_running: bool,
    },
}

/// Context for every world lifecycle phase.
/// `authority` is the server role. LocalAuthoritative and RemoteClient never
/// fire today. `epoch` identifies the open attempt, including failed opens;
/// the same epoch identifies its ready, final-close, or failed-close event.
/// `tick` counts non-paused server ticks since the world opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NativeLifecycleContext {
    /// The server role that fired the phase.
    pub authority: NativeAuthority,
    /// Identifies the open attempt this phase belongs to.
    pub epoch: NativeWorldEpoch,
    /// Non-paused server ticks since the world opened.
    pub tick: NativeTickId,
}

/// Tool action requested by a block interaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolAbility {
    /// An axe strips a log.
    AxeStrip,
    /// An axe scrapes oxidation off copper.
    AxeScrape,
    /// An axe removes wax from copper.
    AxeWaxOff,
    /// A shovel makes a path block.
    ShovelFlatten,
    /// A shovel puts out a campfire.
    ShovelDouse,
    /// A hoe tills soil.
    HoeTill,
}

/// The player responsible for a game rule change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
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
    /// Grant.
    Grant,
    /// Revoke.
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
    /// Whether this is a simulation. Preview calls pass true; actual use passes false.
    pub simulated: bool,
}

/// One native mod, with its bundle's `[mod] id` and hook implementation.
// exhaustive: its fields are private, so mods cannot build or destructure it
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

    /// Marks the thread as inside this mod's `hook` until the guard drops, so a panic in it
    /// names both. A trait injection's generated dispatch calls it.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn enter(&self, hook: &'static str) -> crate::crash::NativeModGuard {
        mark_mod(self.id, hook)
    }
}

/// The ordered list of named mods shared by the client and server.
pub type Mods = Arc<[ModEntry]>;

pub(crate) struct Installed {
    mods: Mods,
    subscribers: horizon_client_core::subscribers::Subscribers,
    disabled: Box<[std::sync::atomic::AtomicBool]>,
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
        let disabled = mods
            .iter()
            .map(|_| std::sync::atomic::AtomicBool::new(false))
            .collect();
        Self {
            mods,
            subscribers,
            disabled,
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
            Box::leak(super::parts::registered_controllers(&mods).into_boxed_slice());
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
    #[cfg(any(test, feature = "testing"))]
    {
        // Tests can replace the list; mods() keeps its static slice lifetime.
        let installed = Box::leak(Box::new(installed));
        TEST_MODS.set(Some(installed));
        installed
    }
    #[cfg(not(any(test, feature = "testing")))]
    {
        assert!(MODS.set(installed).is_ok(), "trait mods are installed once");
        MODS.get().unwrap()
    }
}

/// The process-wide list in dispatch order, or an empty list before installation.
pub(crate) fn mods() -> &'static [ModEntry] {
    installed().map_or(&[], |installed| &installed.mods)
}

pub(crate) fn listened(hook: generated::HookId) -> bool {
    installed().is_some_and(|installed| installed.subscribers.contains_any(hook as usize))
}

/// Switch an installed trait mod's subsequent dispatches on or off.
pub(crate) fn set_mod_enabled(id: &str, on: bool) -> bool {
    let Some(installed) = installed() else {
        return false;
    };
    let mut found = false;
    for (entry, disabled) in installed.mods.iter().zip(&*installed.disabled) {
        if entry.id == id {
            disabled.store(!on, std::sync::atomic::Ordering::Relaxed);
            found = true;
        }
    }
    found
}

/// Whether an installed mod subscribes to an injected hook.
// hidden: a game-side helper, not part of the mod surface
#[doc(hidden)]
pub fn subscribed_by(index: usize, hook: Option<generated::HookId>) -> bool {
    hook.is_some_and(|hook| {
        installed().is_some_and(|installed| {
            index < installed.mods.len()
                && !installed.disabled[index].load(std::sync::atomic::Ordering::Relaxed)
                && installed.subscribers.contains(hook as usize, index)
        })
    })
}

fn mod_index_on(index: usize) -> bool {
    installed().is_none_or(|installed| {
        installed
            .disabled
            .get(index)
            .is_none_or(|disabled| !disabled.load(std::sync::atomic::Ordering::Relaxed))
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
    entered_with_id(hook_id, hook).map(|(_, hooks)| hooks)
}

/// [`entered`] with each mod's own `[mod] id`, for a hook that names the mod that answered.
fn entered_with_id(
    hook_id: generated::HookId,
    hook: &'static str,
) -> impl Iterator<Item = (&'static str, &'static dyn Hooks)> {
    installed().into_iter().flat_map(move |installed| {
        let mut crash_guard = None;
        installed
            .subscribers
            .indices(hook_id as usize)
            .filter(move |&index| {
                !installed.disabled[index].load(std::sync::atomic::Ordering::Relaxed)
            })
            .map(move |index| {
                drop(crash_guard.take());
                let entry = &installed.mods[index];
                let hooks = &*entry.hooks;
                crash_guard = Some(mark_mod(entry.id, hook));
                (entry.id, hooks)
            })
    })
}

pub(crate) fn mark_mod(id: &'static str, hook: &'static str) -> crate::crash::NativeModGuard {
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
    parts
        .iter()
        .filter(|attached| mod_index_on(attached.mod_index))
        .map(move |attached| {
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
    /// Go on to the next listener and then the game's own behaviour.
    Continue,
    /// Stop here; the game's own behaviour does not run.
    Cancel,
}

/// A block state's sounds (`BlockState.getSoundType()`, vanilla's `SoundType`): the sound
/// events, as `LevelRef::play_sound` names them, and the volume and pitch they play at.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct BlockSoundType {
    /// `SoundType.getVolume`.
    pub volume: f32,
    /// `SoundType.getPitch`.
    pub pitch: f32,
    /// `getBreakSound`: the block broken.
    pub break_sound: std::sync::Arc<str>,
    /// `getStepSound`: walked on.
    pub step: std::sync::Arc<str>,
    /// `getPlaceSound`: placed.
    pub place: std::sync::Arc<str>,
    /// `getHitSound`: hit while being mined.
    pub hit: std::sync::Arc<str>,
    /// `getFallSound`: landed on.
    pub fall: std::sync::Arc<str>,
}

/// The states of one block (`StateDefinition.getPossibleStates`): consecutive ids, as the
/// registry numbers a block's states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with `new`; mods read it
pub struct BlockStateRange {
    first: BlockStateId,
    count: u32,
}
impl BlockStateRange {
    /// `count` states from `first`.
    pub const fn new(first: BlockStateId, count: u32) -> Self {
        Self { first, count }
    }
    /// The first state's id.
    pub const fn first(self) -> BlockStateId {
        self.first
    }
    /// How many states the block has.
    pub const fn len(self) -> u32 {
        self.count
    }
    /// Never true: a block has at least one state.
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }
    /// Whether `state` is one of them.
    pub const fn contains(self, state: BlockStateId) -> bool {
        state.0 >= self.first.0 && state.0 - self.first.0 < self.count
    }
    /// Each state, in id order.
    pub fn iter(self) -> impl Iterator<Item = BlockStateId> {
        (self.first.0..self.first.0 + self.count).map(BlockStateId)
    }
}

/// What a fake player does in `PlayerRef::perform_action`: a player's own action, run through
/// the game's path for it, as a client's packet is. Its answer is an [`ActionOutcome`], or an
/// [`ActionRefused`] when none of it ran.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: mods build one; the game runs every kind, so a new kind must be handled there
pub enum PlayerAction {
    /// Use the item in `hand` on the block at `pos`, clicked at `hit`
    /// (`ServerPlayerGameMode.useItemOn`): the block's use, then the item's `useOn`, which
    /// places a block item.
    UseOn {
        /// The hand whose item is used.
        hand: Hand,
        /// The block clicked (`BlockHitResult.getBlockPos`).
        pos: [i32; 3],
        /// Where on it.
        hit: BlockHit,
    },
    /// Use the item in `hand` in the air (`ServerPlayerGameMode.useItem`).
    UseItem {
        /// The hand whose item is used.
        hand: Hand,
    },
    /// Interact with `target` using one hand (`Player.interactOn`), without falling back to
    /// using the item in the air when the entity passes the click.
    InteractOn {
        /// The hand whose item is used.
        hand: Hand,
        /// The entity clicked, in the fake's level.
        target: EntityRef,
        /// The hit relative to the entity's origin, as in an entity interaction packet.
        location: [f64; 3],
    },
    /// Attack `target` with the main hand (`ServerGamePacketListenerImpl.handleInteract`'s
    /// ATTACK, then `Player.attack`): a player, a mob or another entity, or a fireball, a wind
    /// charge or a shulker bullet turned back. Mobs don't take the fake as the player to answer.
    Attack {
        /// What is hit, in the fake's level.
        target: EntityRef,
    },
    /// Break the block at `pos` at once (`ServerPlayerGameMode.destroyBlock`), with the game
    /// mode's and the held tool's rules, the tool's wear and the drops, and no digging time:
    /// a mod that digs over time counts the progress itself. Outside creative, a block no
    /// digging breaks (destroy time -1, such as bedrock) stays.
    BreakBlock {
        /// The block broken.
        pos: [i32; 3],
    },
}

/// Temporary state for [`PlayerRef::perform_action_scoped`]. The existing fake acts in
/// survival with a fresh standing body, holding only `stack` in its main hand; its persistent
/// body, items, look, secondary-use state and action timers are restored afterwards.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PlayerActionSetup {
    /// The main-hand stack, or `None` for an empty hand.
    pub stack: Option<ItemStack>,
    /// Eye position in world coordinates, using the standing eye height (1.62).
    pub eye: [f64; 3],
    /// Vanilla yaw in degrees.
    pub yaw: f32,
    /// Vanilla pitch in degrees.
    pub pitch: f32,
    /// Secondary use, as when holding sneak; this does not change the standing eye height.
    pub secondary_use: bool,
}
impl PlayerActionSetup {
    /// A scoped survival setup with the supplied main-hand stack and viewpoint.
    pub fn new(
        stack: Option<ItemStack>,
        eye: [f64; 3],
        yaw: f32,
        pitch: f32,
        secondary_use: bool,
    ) -> Self {
        Self {
            stack,
            eye,
            yaw,
            pitch,
            secondary_use,
        }
    }
}

/// A scoped action's result. The returned stacks belong to the caller, not to the fake's
/// restored inventory. World changes and normal action hooks have already run.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ScopedActionResult {
    /// What the existing player action path did, including a stopped fake-player effect.
    pub outcome: ActionOutcome,
    /// The remaining main-hand stack, if nonempty.
    pub held: Option<ItemStack>,
    /// Other nonempty inventory stacks, then offhand, carried stack and armor, in that order.
    pub other: Vec<ItemStack>,
}
impl ScopedActionResult {
    /// The outcome and inventory returned by a scoped action.
    pub fn new(outcome: ActionOutcome, held: Option<ItemStack>, other: Vec<ItemStack>) -> Self {
        Self {
            outcome,
            held,
            other,
        }
    }
}

/// Why a fake player's `perform_action` did nothing at all: it was refused before any of the
/// action ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActionRefused {
    /// The handle is a real player's.
    NotFake,
    /// The handle is stale, or its fake is gone.
    Stale,
    /// Another player's turn, an open transaction, or another `perform_action` is running.
    Busy,
    /// The block or entity is out of the player's reach, or the hit isn't on the block
    /// (`ServerGamePacketListenerImpl`'s `handleUseItemOn`, `handleInteract` and
    /// `handleBlockBreakAction` checks).
    OutOfReach,
    /// The block or entity is above or below the world, under spawn protection or outside the
    /// world border (`ServerLevel.mayInteract`): nothing is done there.
    Protected,
    /// The scoped setup's eye position, rotation or entity hit is not finite.
    InvalidSetup,
    /// The action's target isn't an entity in the fake's level, or is the fake itself
    /// (`handleInteract`'s "Attempting to attack an invalid entity").
    InvalidTarget,
    /// The setup's held stack was refused before swapping player state or running the action.
    InvalidStack(StackWriteError),
}

/// An error a handle method answers when its host has no game behind it (a mod's test double):
/// what a `Result`-returning helper refuses with, as `None` and `false` are for the others.
pub trait NoBackend {
    /// The error for a host without a game.
    fn no_backend() -> Self;
}
impl NoBackend for ActionRefused {
    /// No game: as for a handle whose fake is gone.
    fn no_backend() -> Self {
        Self::Stale
    }
}

/// What a fake player's action did once it ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActionOutcome {
    /// Vanilla's `InteractionResult` consumed (a success or a consume), and no effect was
    /// blocked.
    Consumed,
    /// Nothing consumed the click: vanilla passed or failed, or a mod stopped the action.
    Passed,
    /// The click ran (the swing, the hooks before the use, vanilla's `Success`), but the effect
    /// it would start was stopped, as a fake can't take part in it.
    EffectBlocked(FakeEffect),
}

/// An effect a fake player's action is stopped from starting. Kinds may be added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FakeEffect {
    /// A menu or screen: a container, a station, a sign's editor.
    Menu,
    /// Sleeping in a bed.
    Sleep,
    /// Riding an entity or a vehicle.
    Ride,
    /// Going to another dimension.
    ChangeDimension,
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
    /// New.
    pub const fn new(point: [f64; 3], face: [i32; 3], inside: bool) -> Self {
        Self {
            point,
            face,
            inside,
        }
    }
}

/// The result of previewing a tool's block transformation. Applying it is a separate action.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct BlockTransformPreview {
    /// The resulting block state.
    pub state: BlockStateId,
    /// Sound played by the action.
    pub sound: String,
    /// Optional loot table generated by the action.
    pub loot: Option<String>,
    /// Durability cost of the action.
    pub durability: u32,
    /// Particle or level-event identifier.
    pub particle: String,
}
impl BlockTransformPreview {
    /// Construct a transformation result from the engine's decision.
    pub fn new(
        state: BlockStateId,
        sound: String,
        loot: Option<String>,
        durability: u32,
        particle: String,
    ) -> Self {
        Self {
            state,
            sound,
            loot,
            durability,
            particle,
        }
    }
}

/// A vector of doubles, as vanilla's `Vec3`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
// exhaustive: a vector is its three components, as vanilla's `Vec3`
pub struct Vec3 {
    /// East (+) and west (-).
    pub x: f64,
    /// Up (+) and down (-).
    pub y: f64,
    /// South (+) and north (-).
    pub z: f64,
}
impl Vec3 {
    /// The zero vector (`Vec3.ZERO`).
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    /// New.
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
}
impl From<glam::DVec3> for Vec3 {
    fn from(v: glam::DVec3) -> Self {
        Self::new(v.x, v.y, v.z)
    }
}
impl From<Vec3> for glam::DVec3 {
    fn from(v: Vec3) -> Self {
        Self::new(v.x, v.y, v.z)
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
/// use mod_api::{Backend, BackendRef};
/// fn reach(backend: BackendRef<'_>) { let _: &dyn Backend = backend.open(); }
/// ```
///
/// So is reading the backend it holds:
///
/// ```compile_fail
/// use mod_api::BackendRef;
/// fn reach(backend: BackendRef<'_>) { let _ = backend.inner; }
/// ```
// hidden: a game-side type, not part of the mod surface
#[doc(hidden)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BackendRef<'a> {
    inner: &'a dyn Backend,
}

/// The game's mutable native backend, as `Host::backend_mut` hands it out;
/// the same opaque rule as [`BackendRef`], and a wrapper exposes no backend
/// method of its own:
///
/// ```compile_fail
/// use mod_api::BackendMutRef;
/// fn reach(backend: BackendMutRef<'_>) { backend.set_loaded(false); }
/// ```
// hidden: a game-side type, not part of the mod surface
#[doc(hidden)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BackendMutRef<'a> {
    inner: &'a mut dyn BackendMut,
}

/// The opaque client-write backend, with no access to server writes.
///
/// Only the API can construct or open this value.
// hidden: a game-side type, not part of the mod surface
#[doc(hidden)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ClientBackendMutRef<'a> {
    inner: &'a mut dyn ClientBackendMut,
}

impl<'a> ClientBackendMutRef<'a> {
    pub(crate) fn of(inner: &'a mut dyn ClientBackendMut) -> Self {
        Self { inner }
    }

    fn open(self) -> &'a mut dyn ClientBackendMut {
        self.inner
    }
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
pub(crate) fn backend_of(host: &dyn Host) -> Option<&dyn Backend> {
    host.backend().map(|wrapper| wrapper.open())
}

/// The logical side on which a hook is executing.
///
/// A mutable client hook can use client services:
/// ```
/// use mod_api::{self, ClientHost, Hooks, InputKeyEvent};
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
/// A hook that fires on both sides matches on the two:
/// ```
/// use mod_api::HostSide;
/// fn side(host: HostSide<'_>) -> &'static str {
///     match host { HostSide::Server(_) => "server", HostSide::Client(_) => "client" }
/// }
/// ```
///
/// Wrong-side services do not compile:
/// ```compile_fail
/// use mod_api::Host;
/// fn wrong(host: &dyn Host) { host.messages().broadcast(todo!()); }
/// ```
/// ```compile_fail
/// use mod_api::LevelHost;
/// fn wrong(host: &dyn LevelHost) { host.players().server_players(); }
/// ```
/// ```compile_fail
/// use mod_api::ClientHost;
/// fn wrong(host: &mut dyn ClientHost) { host.commands().run_command("list"); }
/// ```
/// ```compile_fail
/// use mod_api::ServerHost;
/// fn wrong(host: &mut dyn ServerHost) { host.mod_screen_open("example"); }
/// ```
///
/// A read-only hook receives only the base host, which cannot be used for a
/// server or client operation, or opened as a backend by a mod:
/// ```compile_fail
/// use mod_api::{Host, ServerHost};
/// fn server(_: &mut dyn ServerHost) {}
/// fn read(host: &dyn Host) { server(host); }
/// ```
/// ```compile_fail
/// use mod_api::{ClientHost, Host};
/// fn client(_: &dyn ClientHost) {}
/// fn read(host: &dyn Host) { client(host); }
/// ```
/// ```compile_fail
/// use mod_api::{Host, ServerHost};
/// fn read(host: &dyn Host) { let _ = host.downcast_ref::<dyn ServerHost>(); }
/// ```
impl HostSide<'_> {
    /// Borrows this side's host again for a shorter call.
    pub fn reborrow(&mut self) -> HostSide<'_> {
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
pub(crate) fn backend_mut_of(host: &mut dyn ServerHost) -> Option<&mut dyn BackendMut> {
    host.backend_mut().map(|wrapper| wrapper.open())
}

pub(crate) fn client_backend_mut_of(
    host: &mut dyn ClientHost,
) -> Option<&mut dyn ClientBackendMut> {
    host.client_backend_mut().map(|wrapper| wrapper.open())
}

/// What used bone meal on a block (`BonemealSource`), for `bonemeal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BonemealSource {
    /// Bone meal used through `BoneMealItem.growCrop`: by a player, a
    /// dispenser or a villager.
    Interaction,
    /// A mob growing the block itself.
    Mob,
}

/// One block a placement writes (`entity_place`'s `Placement::Multi`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BlockPlacement {
    /// Where the block goes.
    pub pos: BlockPos,
    /// The state it is placed in.
    pub state: BlockStateId,
}

impl BlockPlacement {
    /// Create a placement for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(pos: BlockPos, state: BlockStateId) -> Self {
        Self { pos, state }
    }
}

/// What an `entity_place` writes: one block, or the blocks of a multi-block
/// placement (`EntityMultiPlaceEvent`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Placement {
    /// The placed block alone.
    Single,
    /// Every block the placement writes, the primary block first: a bed's
    /// foot and head, a door's halves, a tall plant's two halves.
    Multi(Vec<BlockPlacement>),
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
    /// The stack.
    pub stack: Option<&'a ItemStack>,
    /// The clicking player's UUID and entity id (`getEntity`).
    pub player: u128,
    /// The player entity id.
    pub player_entity_id: i32,
}

/// The key of `InteractionKeyMappingTriggered`: what the player's click was about to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InteractionKey {
    /// `key.attack`: an attack, or a held key digging on.
    Attack,
    /// `key.use`: once for each hand tried, the main hand first.
    Use,
    /// `key.pickItem`: only with a block or entity aimed at.
    PickBlock,
}

/// The behaviour about to run in `UseItemOnBlockEvent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UsePhase {
    /// `IItemExtension.onItemUseFirst`, before the item's Part and the
    /// block's behaviour.
    ItemBeforeBlock,
    /// `BlockBehaviour.useItemOn`, before the block's Part and template.
    Block,
    /// `Item.useOn`, after the block passed and before the item's Part.
    ItemAfterBlock,
}

/// A player's item use without a block or entity target (`RightClickItem`).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct NativeItemClick<'a> {
    /// The side the click is handled on.
    pub side: LogicalSide,
    /// The dimension the player is in.
    pub dimension: LevelDimension,
    /// The block containing the clicking player's feet (`getPos`).
    pub pos: [i32; 3],
    /// The hand that holds the stack.
    pub hand: Hand,
    /// The stack in that hand.
    pub stack: &'a ItemStack,
    /// The clicking player's UUID.
    pub player: u128,
    /// The clicking player's network id.
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
    /// Deny.
    False,
    #[default]
    /// Leave the game's own answer.
    Default,
    /// Allow.
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

/// Result of accessing or decoding an attachment.
pub type AttachmentResult<T> = std::result::Result<T, AttachmentError>;

/// A host is the store of its holders' attachments, so `TYPE.get(host, holder)` reads one.
macro_rules! host_attachment_store {
    ($($host:ident),+) => { $(
        impl AttachmentStore for dyn $host + '_ {
            fn holds(&mut self, holder: Holder, namespace: &str, name: &str) -> AttachmentResult<bool> {
                self.attachments().holds(holder, namespace, name)
            }
            fn decoded(&mut self, holder: Holder, namespace: &str, name: &str) -> AttachmentResult<Option<AttachmentValue>> {
                self.attachments().decoded(holder, namespace, name)
            }
            fn saved(&mut self, holder: Holder, namespace: &str, name: &str) -> AttachmentResult<Option<Compound>> {
                self.attachments().saved(holder, namespace, name)
            }
            fn store(&mut self, holder: Holder, namespace: &str, name: &str, value: AttachmentValue, form: AttachmentForm) -> AttachmentResult<()> {
                self.attachments().store(holder, namespace, name, value, form)
            }
            fn discard(&mut self, holder: Holder, namespace: &str, name: &str, sync: bool) -> AttachmentResult<()> {
                self.attachments().discard(holder, namespace, name, sync)
            }
            fn queue_sync(&mut self, holder: Holder, namespace: &str, name: &str) -> AttachmentResult<()> {
                self.attachments().queue_sync(holder, namespace, name)
            }
        }
    )+ };
}
host_attachment_store!(
    Host,
    ServerHost,
    ClientHost,
    LevelRandomHost,
    LevelWriteHost
);

/// Read-only level services, like vanilla's `LevelReader`.
///
/// A level host cannot be used as a game host:
/// ```compile_fail
/// use mod_api::{Host, LevelHost};
/// fn game(_: &dyn Host) {}
/// fn level(host: &dyn LevelHost) { game(host); }
/// ```
/// ```compile_fail
/// use mod_api::{ClientHost, LevelHost, ServerHost};
/// fn server(_: &mut dyn ServerHost) {}
/// fn client(_: &dyn ClientHost) {}
/// fn level(host: &mut dyn LevelHost) { server(host); client(host); }
/// ```
/// A client-side level cannot write:
/// ```compile_fail
/// use mod_api::LevelSide;
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
    /// Parse a block name and properties as `BlockStateParser.parseForBlock`; None for invalid input or unavailable context.
    fn block_state_id(&self, _name: &str) -> Option<BlockStateId> {
        unimplemented!("LevelHost::block_state_id")
    }
    /// The block's default state (`Block.defaultBlockState`), or None for an unknown block.
    fn block_state_default(&self, _block: BlockKey) -> Option<BlockStateId> {
        unimplemented!("LevelHost::block_state_default")
    }
    /// The full block state name and properties, as `BlockStateParser.serialize`; None for an unknown id.
    fn state_string(&self, _state: BlockStateId) -> Option<String> {
        unimplemented!("LevelHost::state_string")
    }
    /// Lowest build Y (`LevelHeightAccessor.getMinY`).
    fn min_y(&self) -> i32 {
        unimplemented!("LevelHost::min_y")
    }
    /// Build height (`LevelHeightAccessor.getHeight`).
    fn height(&self) -> i32 {
        unimplemented!("LevelHost::height")
    }
    /// `BlockState.is(TagKey)`: numeric membership of the resolved block in the merged block tag.
    /// Unknown tags return false; unavailable authoritative tags or block mappings return None.
    fn block_in_tag(&self, _block: BlockKey, _tag: &str) -> Option<bool> {
        unimplemented!("LevelHost::block_in_tag")
    }
    /// The merged-tag service for this level's owning-thread context.
    /// Not on wasm: a frame resource; its methods are the imports.
    fn tags(&self) -> &dyn Tags {
        unimplemented!("LevelHost::tags")
    }
    /// `BlockGetter.getFluidState`: the fluid at `pos`, the empty fluid where there is none.
    /// None where the position is not loaded or outside the build height; it never loads a chunk.
    fn fluid_state(&self, _pos: [i32; 3]) -> Option<FluidState> {
        unimplemented!("LevelHost::fluid_state")
    }
    /// `BlockState.getFluidState`: the fluid a state holds (water for a waterlogged one). None
    /// for an id no state has.
    fn state_fluid_state(&self, _state: BlockStateId) -> Option<FluidState> {
        unimplemented!("LevelHost::state_fluid_state")
    }
    /// `FluidState.is(TagKey)`: whether the fluid registry name is in the merged fluid tag, as `block_in_tag` reads.
    fn fluid_in_tag(&self, _fluid: &str, _tag: &str) -> Option<bool> {
        unimplemented!("LevelHost::fluid_in_tag")
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
    /// The stored light of one layer at `pos` (`BlockAndLightGetter.getBrightness`), 0 to 15.
    /// It reads the game's own light query: above the build height the sky is 15 and below it 0,
    /// and a chunk that isn't loaded reads as full sky and no block light. It never loads a chunk.
    fn brightness(&self, _layer: LightLayer, _pos: [i32; 3]) -> i32 {
        unimplemented!("LevelHost::brightness")
    }
    /// `BlockAndLightGetter.getRawBrightness(pos, darkening)`: the brighter of the block light
    /// and the sky light less `sky_darken`. `CropBlock.randomTick` asks it with 0.
    fn raw_brightness(&self, _pos: [i32; 3], _sky_darken: i32) -> i32 {
        unimplemented!("LevelHost::raw_brightness")
    }
    /// `Level.isLoaded`: `pos` is inside the build height and its chunk is loaded. Asking never
    /// loads the chunk.
    fn is_loaded(&self, _pos: [i32; 3]) -> bool {
        unimplemented!("LevelHost::is_loaded")
    }
    /// `Level.isRainingAt`: it rains in this level and `pos` is open to the sky (full sky light,
    /// nothing motion-blocking or fluid above it) in a biome whose precipitation there is rain,
    /// not snow or none. False where `pos` is not loaded, the chunk is never loaded for it.
    fn is_raining_at(&self, _pos: [i32; 3]) -> bool {
        unimplemented!("LevelHost::is_raining_at")
    }
    /// `LevelReader.enabledFeatures`: the feature flags the world enables.
    fn enabled_features(&self) -> FeatureFlags {
        unimplemented!("LevelHost::enabled_features")
    }
}

/// Access vanilla's `Level.random`, so draws come in vanilla's order.
pub trait LevelRandomHost: LevelHost {
    /// The holders' server-owned attachments; `TYPE.get(host, holder)` reads one through it.
    fn attachments(&mut self) -> &mut dyn AttachmentStore {
        unimplemented!("LevelRandomHost::attachments")
    }
    /// A copy of the level's random stream through `native_level_random`, or None on a
    /// non-authoritative level. Draw with `with_random`, which stores it back.
    fn random(&mut self) -> Option<LegacyRandom> {
        unimplemented!("LevelRandomHost::random")
    }
    /// Replace the level's random stream with `random`, one `random` gave and the mod drew from.
    /// False on a non-authoritative level.
    fn set_random(&mut self, _random: LegacyRandom) -> bool {
        unimplemented!("LevelRandomHost::set_random")
    }
}

macro_rules! level_random_draws {
    ($($host:ident),+) => { $(
        impl dyn $host + '_ {
            /// Draw from the level's own random stream, in vanilla's order: `draw` gets the
            /// stream and the level keeps it after. None on a non-authoritative level.
            pub fn with_random<T>(&mut self, draw: impl FnOnce(&mut LegacyRandom) -> T) -> Option<T> {
                let mut random = self.random()?;
                let drawn = draw(&mut random);
                self.set_random(random);
                Some(drawn)
            }
        }
    )+ };
}
level_random_draws!(LevelRandomHost, LevelWriteHost);

/// A `LevelReader` with vanilla's `ScheduledTickAccess`. Shape updates cannot write blocks.
pub trait LevelTicksHost: LevelHost {
    /// Schedule a block tick through `native_schedule_block_tick`.
    fn schedule_block_tick(&mut self, _pos: [i32; 3], _block: BlockKey, _delay: i32) -> bool {
        unimplemented!("LevelTicksHost::schedule_block_tick")
    }
    /// Schedule a fluid tick through `native_schedule_fluid_tick`.
    fn schedule_fluid_tick(&mut self, _pos: [i32; 3], _fluid: &str, _delay: i32) -> bool {
        unimplemented!("LevelTicksHost::schedule_fluid_tick")
    }
    /// How strongly the block at `pos` powers its neighbour on the `direction` side, power it
    /// conducts included (`SignalGetter.getSignal(pos, direction)`), 0 to 15: the game's own
    /// read, mods' `block_signal` and `should_check_weak_power` parts included, as
    /// `LevelRef::signal` reads it. It writes nothing and draws nothing, so it leaves the
    /// write it is read in as it was.
    fn signal(&self, _pos: [i32; 3], _direction: Direction) -> i32 {
        unimplemented!("LevelTicksHost::signal")
    }
    /// The strong power alone that the block at `pos` gives that way
    /// (`SignalGetter.getDirectSignal(pos, direction)`), 0 to 15.
    fn direct_signal(&self, _pos: [i32; 3], _direction: Direction) -> i32 {
        unimplemented!("LevelTicksHost::direct_signal")
    }
    /// The strongest strong power reaching the block at `pos` (`SignalGetter.getDirectSignalTo`).
    fn direct_signal_to(&self, _pos: [i32; 3]) -> i32 {
        unimplemented!("LevelTicksHost::direct_signal_to")
    }
    /// The strongest power reaching the block at `pos` (`SignalGetter.getBestNeighborSignal`).
    fn best_neighbor_signal(&self, _pos: [i32; 3]) -> i32 {
        unimplemented!("LevelTicksHost::best_neighbor_signal")
    }
    /// Whether any neighbour powers the block at `pos` (`SignalGetter.hasNeighborSignal`),
    /// asking no side past the first that does.
    fn has_neighbor_signal(&self, _pos: [i32; 3]) -> bool {
        unimplemented!("LevelTicksHost::has_neighbor_signal")
    }
}

/// Mutable level services, like vanilla's `ServerLevel`.
pub trait LevelWriteHost: LevelTicksHost + LevelRandomHost {
    /// Replace the block at `pos` with `state` through `Level.setBlock(pos, state, flags)`, as
    /// `LevelRef::set_block` does: the old block's block entity goes and the new block's is
    /// created in the write, its neighbour and shape updates run per `flags`, and its level events
    /// queue in order with the caller's. A mod block entity starts with the data its
    /// `block_entity` hook gives, which gets this level. True when the block changed; false off an
    /// authoritative server, for a position that is not loaded or outside the build height, an
    /// invalid state, or the state it already is. It never loads a chunk.
    fn set_block(
        &mut self,
        _pos: [i32; 3],
        _state: BlockStateId,
        _flags: NativeBlockUpdateFlags,
    ) -> bool {
        unimplemented!("LevelWriteHost::set_block")
    }
    /// Remove a block through `Level.removeBlock`, preserving its fluid. `moved` marks a piston move.
    fn remove_block(&mut self, _pos: [i32; 3], _moved: bool) -> bool {
        unimplemented!("LevelWriteHost::remove_block")
    }
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

mod generated {
    include!(concat!(env!("OUT_DIR"), "/mod_api_generated.rs"));
}
impl FallOnEvent<'_> {
    /// Ask for a write at the block the entity landed on. It is applied once the entity's
    /// batch has ticked, in the order asked, if the block is still the state it landed on.
    pub fn request(&mut self, write: LandingWrite) {
        self.writes.push(write);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn dimension_keys_name_one_dimension_each() {
        use crate::registry_keys::{dimension_id, dimension_key};
        assert_eq!(
            dimension_key("minecraft:overworld"),
            DimensionKey::OVERWORLD
        );
        assert_eq!(
            dimension_id(DimensionKey::OVERWORLD),
            Some("minecraft:overworld")
        );
        assert_eq!(dimension_key("minecraft:the_nether"), DimensionKey::NETHER);
        assert_eq!(dimension_id(DimensionKey::END), Some("minecraft:the_end"));
        let first = dimension_key("test:first");
        let second = dimension_key("test:second");
        assert_ne!(first, second);
        assert_eq!(dimension_key("test:first"), first);
        assert_eq!(dimension_id(second), Some("test:second"));
        assert_eq!(DimensionKey::from_slot(first.0), Ok(first));
        assert!(DimensionKey::from_slot(u32::MAX).is_err());
        assert_eq!(dimension_id(DimensionKey(u32::MAX)), None);
    }

    #[test]
    fn native_host_returns_typed_recipe_rules() {
        struct Recipes(Arc<[Recipe]>);
        impl mod_api::prelude::RecipesHost for Recipes {
            fn recipes_of_type(&self, kind: &str) -> Arc<[Recipe]> {
                if kind == "minecraft:crafting_transmute" {
                    self.0.clone()
                } else {
                    Arc::from([])
                }
            }
        }
        impl Host for Recipes {}
        mod_api::impl_host_areas!(Recipes, except(recipes));
        let mut recipe = Recipe::new(
            "test:recipe",
            "minecraft:crafting_transmute",
            Arc::new(serde_json::json!({"source": "test"})),
        );
        recipe.rule = Some(RecipeRule::Transmute {
            input: Ingredient::of(vec!["test:input".into()]),
            material: Ingredient::of(vec!["test:material".into()]),
            min: 2,
            max: 4,
            add_count: true,
            result: RecipeTemplate::new(None, 3, None),
        });
        let host = Recipes(Arc::from([recipe]));
        let view = (&host as &dyn Host).recipes_of_type("minecraft:crafting_transmute");
        let Some(RecipeRule::Transmute {
            input,
            material,
            min,
            max,
            add_count,
            result,
        }) = &view[0].rule
        else {
            panic!("wrong rule")
        };
        assert_eq!(input.order, ["test:input"]);
        assert_eq!(material.order, ["test:material"]);
        assert_eq!((*min, *max, *add_count, result.count), (2, 4, true, 3));
        assert_eq!(view[0].json["source"], "test");
        assert!(
            (&host as &dyn Host)
                .recipes_of_type("test:other")
                .is_empty()
        );
    }

    #[test]
    fn client_menu_click_refuses_a_host_without_its_backend() {
        struct Empty;
        impl Host for Empty {}
        mod_api::impl_host_areas!(Empty);
        impl ClientHost for Empty {}
        mod_api::impl_client_host_areas!(Empty);
        let player = __private::game::player_ref(EntityRef::new(NativeWorldEpoch::new(1), 7), 42);
        let menu = __private::game::menu_ref(player, 0, 1);
        assert!(!menu.click(
            &mut Empty,
            &[MenuClick::Pickup {
                slot: Some(0),
                button: MouseButton::Left,
            }]
        ));
    }

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
    fn an_optional_dependency_runs_only_when_built_with_and_installed() {
        struct Installed(&'static str);
        impl mod_api::prelude::LevelsHost for Installed {
            fn is_loaded(&self, id: &str) -> bool {
                id == self.0
            }
        }
        impl Host for Installed {}
        mod_api::impl_host_areas!(Installed, except(levels));
        // The `testing` feature stands for a dependency's: without it the block isn't compiled,
        // with it the game is asked.
        let ran = std::cell::Cell::new(0);
        for host in [Installed("testing"), Installed("other")] {
            let host: &dyn Host = &host;
            if_installed!(host, testing, {
                ran.set(ran.get() + 1);
            });
        }
        assert_eq!(ran.get(), usize::from(cfg!(feature = "testing")));
    }

    struct Quiet;
    #[mod_api::__subscriptions]
    impl Hooks for Quiet {}

    /// A double with the `Host` methods the probe calls.
    struct Reader {
        block: &'static str,
        lines: Vec<Text>,
    }

    impl mod_api::prelude::LevelsBackend for Reader {
        fn level_block(&self, _level: LevelRef, _pos: [i32; 3]) -> Option<String> {
            Some(self.block.to_string())
        }
    }
    impl Backend for Reader {}
    mod_api::impl_backend_areas!(Reader, except(levels));

    impl mod_api::prelude::EntitiesHost for Reader {
        fn level(&self, dimension: &str) -> Option<LevelRef> {
            (dimension == "minecraft:overworld")
                .then(|| LevelRef::new(NativeWorldEpoch::new(1), crate::DimensionKey::OVERWORLD))
        }
    }
    impl Host for Reader {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(BackendRef::of(self))
        }
    }
    mod_api::impl_host_areas!(Reader, except(entities));
    impl ClientHost for Reader {}
    mod_api::impl_client_host_areas!(Reader);
    impl mod_api::prelude::ServicesServerHost for Reader {
        fn players(&self) -> &dyn Players {
            self
        }
        fn commands(&mut self) -> &mut dyn Commands {
            self
        }
    }
    impl ServerHost for Reader {
        fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
            Some(BackendMutRef::of(self))
        }
    }
    mod_api::impl_server_host_areas!(Reader, except(services));
    impl Commands for Reader {}
    impl Players for Reader {
        fn acting_player(&self) -> Option<PlayerRef> {
            Some(crate::__private::game::player_ref(
                EntityRef::new(NativeWorldEpoch::new(1), 41),
                7,
            ))
        }
    }
    impl mod_api::prelude::PlayersBackendMut for Reader {
        fn player_action_bar(&mut self, player: PlayerRef, line: Text) -> bool {
            assert_eq!(player.uuid(), 7);
            self.lines.push(line);
            true
        }
    }
    impl BackendMut for Reader {}
    mod_api::impl_backend_mut_areas!(Reader, except(players));

    #[test]
    fn server_services_forward_commands_messages_and_player_identity() {
        use std::cell::RefCell;
        #[derive(Default)]
        struct Server {
            lines: RefCell<Vec<String>>,
        }
        impl Host for Server {}
        mod_api::impl_host_areas!(Server);
        impl mod_api::prelude::ServicesServerHost for Server {
            fn commands(&mut self) -> &mut dyn Commands {
                self
            }
            fn messages(&self) -> &dyn Messages {
                self
            }
            fn players(&self) -> &dyn Players {
                self
            }
        }
        impl ServerHost for Server {}
        mod_api::impl_server_host_areas!(Server, except(services));
        impl Commands for Server {
            fn run_command(&mut self, line: &str) -> bool {
                self.lines.get_mut().push(line.to_owned());
                line == "list"
            }
            fn feedback(&mut self, line: Text) {
                self.lines.get_mut().push(line.plain());
            }
        }
        impl Messages for Server {
            fn broadcast(&self, line: Text) {
                self.lines.borrow_mut().push(line.plain());
            }
            fn text_allowed(&self, line: &str) -> Option<bool> {
                Some(line != "blocked")
            }
        }
        impl Players for Server {
            fn acting_player(&self) -> Option<PlayerRef> {
                Some(crate::__private::game::player_ref(
                    EntityRef::new(NativeWorldEpoch::new(1), 42),
                    7,
                ))
            }
            fn server_players(&self) -> Vec<PlayerRef> {
                vec![self.acting_player().unwrap()]
            }
        }
        let mut server = Server::default();
        let host: &mut dyn ServerHost = &mut server;
        assert!(host.commands().run_command("list"));
        assert!(!host.commands().run_command("missing"));
        host.commands().feedback(Text::literal("answer"));
        host.messages().broadcast(Text::literal("everyone"));
        assert_eq!(host.messages().text_allowed("blocked"), Some(false));
        let player = host.players().acting_player().unwrap();
        assert_eq!(host.players().server_players(), [player]);
        assert_eq!((player.uuid(), player.entity().id()), (7, 42));
        assert_eq!(
            *server.lines.borrow(),
            ["list", "missing", "answer", "everyone"]
        );
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

    /// Installs one mod, `id`, that attaches `part` to sticks.
    fn stick_part<P: ItemPart + Clone>(id: &'static str, part: P) {
        struct StickMod<P>(P);
        #[mod_api::__subscriptions]
        impl<P: ItemPart + Clone> Hooks for StickMod<P> {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let item = r.item_key("minecraft:stick");
                r.item_part(item, self.0.clone());
            }
        }
        let mods: Mods = vec![ModEntry::new(id, Box::new(StickMod(part)))].into();
        install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema()).unwrap();
    }

    /// Claims a use on stone, after telling the player what the block is.
    #[derive(Clone)]
    struct Probe;

    #[mod_api::horizon_part]
    impl ItemPart for Probe {
        fn use_on(&self, host: &mut dyn ServerHost, event: &mut UseOnEvent<'_>) -> Flow {
            let player = host.players().acting_player().expect("a player acts");
            let level = host.level("minecraft:overworld").expect("a loaded level");
            let block = level.block(host, event.pos).expect("a loaded block");
            player.action_bar(host, Text::literal(block.clone()));
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
    fn game_mode_change_dispatches_the_shared_type_and_honors_cancellation() {
        struct VetoSpectator;
        #[mod_api::__subscriptions]
        impl Hooks for VetoSpectator {
            fn player_game_mode_changed(
                &self,
                _host: &mut dyn ServerHost,
                event: &mut PlayerGameModeChangedEvent<'_>,
            ) -> Flow {
                assert_eq!((event.player, event.name), (7, "Probe"));
                assert_eq!(event.from, horizon_core::game_mode::GameMode::Adventure);
                if event.to == GameMode::Spectator {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        install(vec![ModEntry::new("mode-veto", Box::new(VetoSpectator))].into());
        let mut host = Reader {
            block: "minecraft:air",
            lines: Vec::new(),
        };
        for (to, expected_cancel) in [(GameMode::Spectator, true), (GameMode::Survival, false)] {
            let mut cancel = false;
            dispatch_player_game_mode_changed(
                &mut host,
                7,
                "Probe",
                horizon_core::game_mode::GameMode::Adventure,
                to,
                &mut cancel,
            );
            assert_eq!(cancel, expected_cancel);
        }
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
    #[derive(Clone)]
    struct Chatty;

    #[mod_api::horizon_part]
    impl ItemPart for Chatty {
        fn use_on(&self, host: &mut dyn ServerHost, _event: &mut UseOnEvent<'_>) -> Flow {
            host.commands().run_command("say hello");
            Flow::Continue
        }
    }

    fn stick() -> ItemStack {
        crate::parts::tests::stack("minecraft:stick", 1, 0)
    }

    #[test]
    fn dispatch_before_installation_is_empty() {
        let mut components = Vec::new();
        dispatch_component_types(&RegistrationContext::new(), &mut components);
        assert!(mods().is_empty());
        assert!(components.is_empty());
    }

    #[test]
    #[should_panic(expected = "native mod id must not be empty")]
    fn an_entry_requires_a_nonempty_id() {
        ModEntry::new("", Box::new(Quiet));
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
        stick_part("probe", Probe);
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
                calls::UseOnCall {
                    stack: &stack,
                    pos,
                    face,
                    hand: Hand::Main,
                    hit,
                    player: 7,
                    player_entity_id: 41,
                    claimed: &mut claimed,
                },
            );
            assert_eq!(claimed, expected, "{block}");
            let told: Vec<String> = host.lines.iter().map(Text::plain).collect();
            assert_eq!(told, [block], "{block}");
        }
    }

    #[test]
    fn mining_events_identify_the_player_and_keep_mod_decisions() {
        struct MiningProbe(Arc<Mutex<Vec<u32>>>);
        #[mod_api::__subscriptions]
        impl Hooks for MiningProbe {
            fn player_harvest_check(
                &self,
                _host: crate::HostSide,
                event: &mut PlayerHarvestCheckEvent,
            ) {
                self.0.lock().unwrap().push(event.player);
                event.can_harvest = false;
            }
            fn player_break_speed(
                &self,
                _host: crate::HostSide,
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
    #[should_panic(expected = "Commands::run_command")]
    fn an_unexpected_host_call_names_the_method() {
        // Another test installs parts with tags: a dispatch must run on a game thread.
        mark_game_thread();
        stick_part("chatty", Chatty);
        let mut host = Reader {
            block: "minecraft:stone",
            lines: Vec::new(),
        };
        let mut claimed = false;
        let (pos, face) = ([0; 3], [0, 1, 0]);
        dispatch_use_on(
            &mut host,
            calls::UseOnCall {
                stack: &stick(),
                pos,
                face,
                hand: Hand::Main,
                hit: top_hit(),
                player: 7,
                player_entity_id: 41,
                claimed: &mut claimed,
            },
        );
    }

    /// The top of the block at the origin, clicked in its middle.
    fn top_hit() -> BlockHit {
        BlockHit::new([0.5, 1.0, 0.5], [0, 1, 0], false)
    }

    /// Takes a click with the off hand on a block's top half; its sticks check what they hear.
    struct OffHandTop;

    /// A stick's use, checking the hand, the hit and the player.
    #[derive(Clone)]
    struct HeardStick;

    #[mod_api::horizon_part]
    impl ItemPart for HeardStick {
        fn use_on(&self, _: &mut dyn ServerHost, event: &mut UseOnEvent<'_>) -> Flow {
            assert_eq!((event.player, event.player_entity_id), (7, 41));
            assert_eq!((event.hand, event.hit), (Hand::Off, top_hit()));
            HEARD_STICK.store(true, std::sync::atomic::Ordering::SeqCst);
            Flow::Continue
        }
    }

    static HEARD_STICK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    #[mod_api::__subscriptions]
    impl Hooks for OffHandTop {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let item = r.item_key("minecraft:stick");
            r.item_part(item, HeardStick);
        }

        fn right_click_block(&self, _: crate::HostSide, event: &mut RightClickBlockEvent) -> Flow {
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
        let mods: Mods = vec![ModEntry::new("off-hand-top", Box::new(OffHandTop))].into();
        install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema()).unwrap();
        let mut host = Reader {
            block: "minecraft:stone",
            lines: Vec::new(),
        };
        let mut claimed = false;
        dispatch_use_on(
            &mut host,
            calls::UseOnCall {
                stack: &stick(),
                pos: [0; 3],
                face: [0, 1, 0],
                hand: Hand::Off,
                hit: top_hit(),
                player: 7,
                player_entity_id: 41,
                claimed: &mut claimed,
            },
        );
        assert!(!claimed);
        assert!(HEARD_STICK.load(std::sync::atomic::Ordering::SeqCst));
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
        let mut rules = horizon_client_core::gamerules::GameRules::default();
        // What `set_game_rule` and the other setters do.
        use horizon_client_core::gamerules::Value as EngineValue;
        // Independent engine inputs and expected API values: production conversion lives in Game.
        for (name, engine, value) in [
            (
                "keep_inventory",
                EngineValue::Bool(true),
                GameRuleValue::Bool(true),
            ),
            (
                "random_tick_speed",
                EngineValue::Int(-1),
                GameRuleValue::Int(-1),
            ),
            ("keep_inventory", EngineValue::Int(1), GameRuleValue::Int(1)),
            (
                "no_such_rule",
                EngineValue::Bool(true),
                GameRuleValue::Bool(true),
            ),
            (
                "random_tick_speed",
                EngineValue::Int(7),
                GameRuleValue::Int(7),
            ),
        ] {
            if rules.set(name, engine) {
                dispatch_game_rule_changed(&mut host, &["minecraft:", name].concat(), value, None);
            }
        }
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
                .any(|s| s.item() == crate::parts::tests::fixture_item_key("minecraft:cod"));
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
        install_parts(&[], &crate::parts::tests::rule_schema()).unwrap();
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
            let drops = [crate::parts::tests::stack(item, 1, 0)];
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
        let drops = [crate::parts::tests::stack("minecraft:salmon", 1, 0)];
        let fished = NativeItemFished {
            hook: 9,
            position: [0.5, 62.0, 0.5],
            drops: &drops,
        };
        let (mut rod_damage, mut cancel) = (2, false);
        dispatch_item_fished(&mut host, 7, 3, &fished, &mut rod_damage, &mut cancel);
        assert_eq!((rod_damage, cancel), (4, false));
    }
}

#[doc(hidden)]
pub(crate) mod built {
    #[allow(unused_imports)]
    use super::*;
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct ToolUse {
        pub player: Option<u32>,
        pub hand: crate::first_person::Hand,
        pub pos: [i32; 3],
        pub face: [i32; 3],
        pub click: [f64; 3],
        pub ability: ToolAbility,
        pub simulated: bool,
    }
    impl From<ToolUse> for super::ToolUse {
        fn from(v: ToolUse) -> Self {
            let ToolUse {
                player,
                hand,
                pos,
                face,
                click,
                ability,
                simulated,
            } = v;
            Self {
                player,
                hand,
                pos,
                face,
                click,
                ability,
                simulated,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct NativeLootDrops<'a> {
        pub table: &'a str,
        pub dimension: &'a str,
        pub origin: [f64; 3],
        pub block: Option<([i32; 3], BlockStateId)>,
        pub entity: Option<(u32, &'a str)>,
        pub player: Option<u128>,
        pub tool: Option<&'a ItemStack>,
        pub explosion: Option<f32>,
        pub used: bool,
    }
    impl<'a> From<NativeLootDrops<'a>> for super::NativeLootDrops<'a> {
        fn from(v: NativeLootDrops<'a>) -> Self {
            let NativeLootDrops {
                table,
                dimension,
                origin,
                block,
                entity,
                player,
                tool,
                explosion,
                used,
            } = v;
            Self {
                table,
                dimension,
                origin,
                block,
                entity,
                player,
                tool,
                explosion,
                used,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct NativeExplosionStart<'a> {
        pub dimension: &'a str,
        pub center: [f64; 3],
        pub radius: f32,
        pub fire: bool,
        pub interaction: NativeBlockInteraction,
        pub damage_type: &'a str,
        pub source: Option<u32>,
        pub owner: Option<u32>,
    }
    impl<'a> From<NativeExplosionStart<'a>> for super::NativeExplosionStart<'a> {
        fn from(v: NativeExplosionStart<'a>) -> Self {
            let NativeExplosionStart {
                dimension,
                center,
                radius,
                fire,
                interaction,
                damage_type,
                source,
                owner,
            } = v;
            Self {
                dimension,
                center,
                radius,
                fire,
                interaction,
                damage_type,
                source,
                owner,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct NativeItemFished<'a> {
        pub hook: u32,
        pub position: [f64; 3],
        pub drops: &'a [ItemStack],
    }
    impl<'a> From<NativeItemFished<'a>> for super::NativeItemFished<'a> {
        fn from(v: NativeItemFished<'a>) -> Self {
            let NativeItemFished {
                hook,
                position,
                drops,
            } = v;
            Self {
                hook,
                position,
                drops,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct NativeScreenView<'a> {
        pub id: &'static str,
        pub menu: Option<&'a str>,
        pub title: Option<&'a Text>,
        pub rect: [f32; 4],
        pub slots: usize,
        pub mouse: [f32; 2],
        pub partial_tick: f32,
    }
    impl<'a> From<NativeScreenView<'a>> for super::NativeScreenView<'a> {
        fn from(v: NativeScreenView<'a>) -> Self {
            let NativeScreenView {
                id,
                menu,
                title,
                rect,
                slots,
                mouse,
                partial_tick,
            } = v;
            Self {
                id,
                menu,
                title,
                rect,
                slots,
                mouse,
                partial_tick,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct NativeItemClick<'a> {
        pub side: LogicalSide,
        pub dimension: LevelDimension,
        pub pos: [i32; 3],
        pub hand: Hand,
        pub stack: &'a ItemStack,
        pub player: u128,
        pub player_entity_id: i32,
    }
    impl<'a> From<NativeItemClick<'a>> for super::NativeItemClick<'a> {
        fn from(v: NativeItemClick<'a>) -> Self {
            let NativeItemClick {
                side,
                dimension,
                pos,
                hand,
                stack,
                player,
                player_entity_id,
            } = v;
            Self {
                side,
                dimension,
                pos,
                hand,
                stack,
                player,
                player_entity_id,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct NativeBlockClick<'a> {
        pub side: LogicalSide,
        pub dimension: LevelDimension,
        pub pos: [i32; 3],
        pub hit: super::BlockHit,
        pub hand: Hand,
        pub stack: Option<&'a ItemStack>,
        pub player: u128,
        pub player_entity_id: i32,
    }
    impl<'a> From<NativeBlockClick<'a>> for super::NativeBlockClick<'a> {
        fn from(v: NativeBlockClick<'a>) -> Self {
            let NativeBlockClick {
                side,
                dimension,
                pos,
                hit,
                hand,
                stack,
                player,
                player_entity_id,
            } = v;
            Self {
                side,
                dimension,
                pos,
                hit,
                hand,
                stack,
                player,
                player_entity_id,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct NativeLifecycleContext {
        pub authority: NativeAuthority,
        pub epoch: super::NativeWorldEpoch,
        pub tick: super::NativeTickId,
    }
    impl From<NativeLifecycleContext> for super::NativeLifecycleContext {
        fn from(v: NativeLifecycleContext) -> Self {
            let NativeLifecycleContext {
                authority,
                epoch,
                tick,
            } = v;
            Self {
                authority,
                epoch,
                tick,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct NativeActiveEffect {
        pub id: super::EffectKey,
        pub duration: i32,
        pub amplifier: i32,
        pub ambient: bool,
        pub visible: bool,
    }
    impl From<NativeActiveEffect> for super::NativeActiveEffect {
        fn from(v: NativeActiveEffect) -> Self {
            let NativeActiveEffect {
                id,
                duration,
                amplifier,
                ambient,
                visible,
            } = v;
            Self {
                id,
                duration,
                amplifier,
                ambient,
                visible,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct NativePlayerExperience {
        pub level: i32,
        pub progress: f32,
        pub total: i32,
    }
    impl From<NativePlayerExperience> for super::NativePlayerExperience {
        fn from(v: NativePlayerExperience) -> Self {
            let NativePlayerExperience {
                level,
                progress,
                total,
            } = v;
            Self {
                level,
                progress,
                total,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct NativePlayerFood {
        pub level: i32,
        pub saturation: f32,
        pub exhaustion: f32,
    }
    impl From<NativePlayerFood> for super::NativePlayerFood {
        fn from(v: NativePlayerFood) -> Self {
            let NativePlayerFood {
                level,
                saturation,
                exhaustion,
            } = v;
            Self {
                level,
                saturation,
                exhaustion,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct NativePlayerAbilities {
        pub invulnerable: bool,
        pub flying: bool,
        pub may_fly: bool,
        pub instabuild: bool,
        pub may_build: bool,
        pub fly_speed: f32,
        pub walk_speed: f32,
    }
    impl From<NativePlayerAbilities> for super::NativePlayerAbilities {
        fn from(v: NativePlayerAbilities) -> Self {
            let NativePlayerAbilities {
                invulnerable,
                flying,
                may_fly,
                instabuild,
                may_build,
                fly_speed,
                walk_speed,
            } = v;
            Self {
                invulnerable,
                flying,
                may_fly,
                instabuild,
                may_build,
                fly_speed,
                walk_speed,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct NativeServerPlayer {
        pub uuid: u128,
        pub name: String,
        pub permission: u8,
        pub dimension: String,
        pub position: [f64; 3],
        pub game_mode: i32,
        pub health: f32,
        pub yaw: f32,
        pub pitch: f32,
        pub entity_id: i32,
        pub address: String,
    }
    impl From<NativeServerPlayer> for super::NativeServerPlayer {
        fn from(v: NativeServerPlayer) -> Self {
            let NativeServerPlayer {
                uuid,
                name,
                permission,
                dimension,
                position,
                game_mode,
                health,
                yaw,
                pitch,
                entity_id,
                address,
            } = v;
            Self {
                uuid,
                name,
                permission,
                dimension,
                position,
                game_mode,
                health,
                yaw,
                pitch,
                entity_id,
                address,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct ChannelRef {
        pub id: u64,
    }
    impl From<ChannelRef> for super::ChannelRef {
        fn from(v: ChannelRef) -> Self {
            let ChannelRef { id } = v;
            Self { id }
        }
    }
    // exhaustive: the game builds it with a literal; mods read it and edit its fields
    pub struct SoundPlay {
        pub identifier: String,
        pub source: super::SoundSource,
        pub volume: f32,
        pub pitch: f32,
        pub position: [f64; 3],
        pub looping: bool,
        pub delay: i32,
        pub attenuation: super::SoundAttenuation,
        pub relative: bool,
    }
    impl From<SoundPlay> for super::SoundPlay {
        fn from(v: SoundPlay) -> Self {
            let SoundPlay {
                identifier,
                source,
                volume,
                pitch,
                position,
                looping,
                delay,
                attenuation,
                relative,
            } = v;
            Self {
                identifier,
                source,
                volume,
                pitch,
                position,
                looping,
                delay,
                attenuation,
                relative,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct NativePortalShape {
        pub x_axis: bool,
        pub bottom_left: [i32; 3],
        pub width: i32,
        pub height: i32,
    }
    impl From<NativePortalShape> for super::NativePortalShape {
        fn from(v: NativePortalShape) -> Self {
            let NativePortalShape {
                x_axis,
                bottom_left,
                width,
                height,
            } = v;
            Self {
                x_axis,
                bottom_left,
                width,
                height,
            }
        }
    }
}

include!(concat!(env!("OUT_DIR"), "/mod_api_exports_root.rs"));

#[cfg(test)]
mod brewing_dispatch_tests {
    include!("brewing_dispatch_tests.rs");
}

#[cfg(test)]
mod item_model_dispatch_tests {
    include!("item_model_dispatch_tests.rs");
}
