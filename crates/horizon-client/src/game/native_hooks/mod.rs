//! Native hook implementations, split by catalog area.
//!
//! Hook documentation and contract versions live in the catalog, not a shared changelog.
//!
//! Native mods are built against an exact game revision, rather than a hook API revision.
//! Bodies have no `cfg` and no macros: horizon-mixins refuses a target with
//! either (`tests::hook_points_stay_hookable`).

/// Generated dispatch entry points, not hand-written hook implementations.
mod fire;

use super::native_data_maps::NativeDataMapType;
use super::{
    Game, GameLevel, LevelDimension, Screen,
    execution_role::{self, ExecutionRole},
    maps::{NativeMapDecoration, NativeMapInfo},
    mod_api_host,
    native_block_entity_draw::BlockEntityDraw,
    native_configs::{NativeConfig, NativeConfigFile},
    native_mob_draw::{NativeMobRender, NativePlayerRender},
    native_panel_recipes::RecipePage,
};
use horizon_client::block_contact::Pending;
use horizon_client::creative::Creative;
use horizon_client::{
    atlas_sources::qualified,
    biome::{BiomeColors, SpawnEntry},
    block_entities::Pos,
    chat::ChatSource,
    containers::{
        BlockEntity, MOD_MENUS, ModBlockEntityFields, ModContainer, ModEnergyBuffer, ModFluidTanks,
    },
    crafting::{Ingredient, recipe_result},
    extras::Corner,
    extras_hud::Corners,
    fast_hash::FxHashMap,
    first_person::Hand,
    health::{
        Attack, DamageContainer, DamageHooks, DamageLevel, HurtSource, Hurting, Knock, ShieldBlock,
        Victim,
    },
    hud::{HudBatch, rgba},
    hud_layout::GuiDraw,
    items::ItemStack,
    keybinds,
    lan::menu::Slots,
    merchant::MerchantOffer,
    mobs::{Damage, Who},
    mod_list::{self, Installed, KeyMapping, NativePage},
    music::{Music, MusicLayer},
    particles::{ParticleOptions, ParticleType},
    text::Text,
};
use horizon_client_core::gamerules::Value as GameRuleValue;
use horizon_client_core::suggestion::Suggestion;
use horizon_core::{
    geometry::Direction,
    position::BlockPos,
    random::LegacyRandom,
    ticks::{ScheduleResult, TickPriority},
};
use horizon_world::level::{BonemealCheck, PlaceContext};
use mod_api::CommandSource;
use mod_api::GameMode;
pub use mod_api::game::{NativeClock, NativeShake, PhantomSpawnResult, SleepProblem};
use mod_api::{
    self, Answered, BlockPlacement, BlockTintSources, ClientAudioOptions, ClientChatOptions,
    ClientGraphicsOptions, ClientGuiOptions, ClientInputOptions, ClientModifiers,
    ClientReloadListeners, CommandInvocation, EnchantmentInfo, EntityRef, GameRuleActor,
    InteractionKey, LayerDefinition, LivingRef, NativeCamera, NativeCommand, NativeDispensed,
    NativeHandRender, NativeKeyMapping, NativeLifecycleContext, NativeNoteBlockChange,
    PathComputationType, Placement, PlayerRef, RecipeBookCategory, ScreenId, ScreenOpening,
    ScreenshotImage, ServerReloadListeners, ServerTickPhase, TicketController, ToolUse, WorldPhase,
};
use mod_api::{
    AttachmentStore, BonemealSource, ChunkRef, FluidStack, LandingWrite, LevelRef, LogicalSide,
    NativeActiveEffect, NativeBlockClick, NativeBlockUpdateFlags, NativeInteractionResult,
    NativeItemClick, NativeTriState, NeighbourUpdate, TooltipElement, UsePhase,
};
use mod_api::{BlockKey, DebugEntry, DebugRenderer, EffectKey};
use mod_api::{
    CameraType, Identifier, MobRef, NativeDespawn, NativePermissionLevel, NativeSpawnCheck,
    NativeSpawnReason, SoundPlay, SoundRef,
};
pub(crate) use mod_api::{
    ClientPacket, ClientPayload, NativeBlockRim, NativeColoredLight, NativeExplosionStart,
    NativeItemFished, NativeLootDrops, NativeMinimap, NativePanelEvent, NativePortalShape,
    NativeScreen, NativeScreenView, NativeStack, NativeZoom,
};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use winit::{event::MouseButton, keyboard::KeyCode};
use {
    horizon_core::{
        nbt::{Compound, Tag},
        use_animation::UseAnimation,
    },
    horizon_world::{
        fluid::FluidKind,
        level::{BlockId, LevelAccess, PlacementContext, Ticks},
        registry::{BlockRegistry, BlockStateId},
    },
};

include!(concat!(env!("OUT_DIR"), "/native_hook_modules.rs"));
include!("shared.rs");

use mod_api::ModBlockEntity;
