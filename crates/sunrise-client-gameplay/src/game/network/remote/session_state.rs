use super::{break_prediction::Prediction, general, level_format};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use sunrise_client_lib::net::Session;
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_level::{
    chunk::Chunk,
    chunk_streamer::{ChunkGenerator, Proto},
};
use sunrise_mod_runtime::attachments::{Holder, ResidentAttachments};
#[cfg(feature = "client")]
use sunrise_protocol::item::Stack;
use sunrise_protocol::{
    packets::play,
    registry::{Builtin, DimensionType, Dynamic},
    secure_chat,
    text::Language,
};
use sunrise_server_files::profiles::ServicesKeySet;
use sunrise_sim::{first_person_sim::Hand, join_seq::JoinSeq, mobs::MobType};
use sunrise_world::stepped_interpolation::SteppedInterpolation;
use {
    crate::game::client_side::raw_backlog::Received,
    crate::game::client_side::remote_chunks::ChunkDecoder,
    crate::game::client_side::remote_chunks::ChunkFormat,
    crate::game::client_side::remote_chunks::RemoteLight, sunrise_gameplay_common::game::Dim,
    sunrise_gameplay_common::game::Hit, sunrise_gameplay_common::game::remote::Lerp,
    sunrise_gameplay_common::game::remote::chat_received::QueuedChat,
    sunrise_gameplay_common::game::remote::minecarts::CartLerp,
    sunrise_gameplay_common::game::remote::passengers::PassengerSeat,
};

/// Streaming needs a generator; a server world never generates.
pub(in crate::game) struct NoTerrain;
impl ChunkGenerator for NoTerrain {
    type Worker = ();
    fn worker(&self) {}
    fn proto(&self, _: &mut (), _: i32, _: i32) -> std::result::Result<Proto, String> {
        Ok(Box::new(()))
    }
    fn finish(&self, _: Proto) -> Chunk {
        Chunk::empty(0, 0)
    }
}

/// A server entity the client tracks (position as last sent).
pub struct Tracked {
    pub kind: String,
    pub position: [f64; 3],
    pub previous: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub previous_yaw: f32,
    pub previous_pitch: f32,
    /// `Entity.getPositionCodec`'s base: the position the server last
    /// reported (AddEntity, MoveEntity, EntityPositionSync), which a move
    /// packet's delta counts from. A TeleportEntity moves the entity but not
    /// this base, so the server's next delta still lands where it means.
    pub codec_base: [f64; 3],
    pub on_fire: bool,
    pub mod_fire_immune: bool,
    pub silent: bool,
    /// `Entity.vehicle`: the entity it rides (`SetPassengers`), whatever either one's type.
    pub vehicle: Option<u32>,
    /// `Entity.tickCount` of this client's copy: the ticks the client level gave it since the
    /// server added it (`Entity.commonTick`), counted in `tick_remote_entities`.
    pub tick_count: i32,
    /// What `entity_size` answered at this copy's last size refresh, with the synced pose it
    /// was asked for, unless the copy is a mob, which keeps its own (`Mob::size_answer`).
    pub size_answer: Option<sunrise_sim::pose::SizeAnswer>,
}

/// An item has joined the client level, but has no stack to draw yet.
pub(in crate::game) struct PendingItem {
    pub(in crate::game) join: JoinSeq,
    pub(in crate::game) position: [f64; 3],
    pub(in crate::game) velocity: Vec3,
    /// Entity.DATA_NO_GRAVITY, if it came before the stack.
    pub(in crate::game) no_gravity: bool,
}

/// `Entity.startRiding` / `removePassenger` for a `SetPassengers` packet: the
/// listed entities ride `vehicle`, the ones that rode it and are no longer
/// listed got off. Every tracked entity counts, whatever its type, so a
/// server's item display or marker on a player hides that player's name tag
/// as vanilla's `isVehicle` does (#5666).
pub fn seat_passengers(entities: &mut HashMap<i32, Tracked>, vehicle: u32, passengers: &[i32]) {
    for (id, t) in entities.iter_mut() {
        if passengers.contains(id) {
            t.vehicle = Some(vehicle);
        } else if t.vehicle == Some(vehicle) {
            t.vehicle = None;
        }
    }
}
/// `Minecraft.services()` trust keys, fetched once per connection off the game thread.
pub enum ServicesKeys {
    Pending,
    Ready(ServicesKeySet),
}

/// One `PlayerInfo` property, shared by the list entry and the entities that kept it.
pub struct PlayerTextures(pub(in crate::game) sunrise_client_render::skins::PackedTextures);

impl PlayerTextures {
    /// `SkinManager.createLookup(profile, requireSecure)`: a secure lookup shows
    /// only textures whose signature the keys verified, so it waits for keys that
    /// are still pending; the local player's shows its textures unverified.
    /// `SkinManager.getOrLoad` unpacks and verifies on a thread of its own, never
    /// the game thread: `None` until that answers, and the player shows the
    /// default skin.
    pub fn unpack(
        &self,
        downloads: &mut sunrise_client_render::skins::Downloader,
        services_keys: &ServicesKeys,
        require_secure: bool,
    ) -> Option<Arc<sunrise_client_render::skins::Textures>> {
        match services_keys {
            // Never cache an invalid result for a signature the pending keys could verify.
            ServicesKeys::Pending if self.0.signature.is_some() && require_secure => None,
            // `PlayerInfo.createSkinLookup`: requireSecure is false for the local player, whose
            // signed property shows unverified. An unsigned one reads the same whatever the keys.
            ServicesKeys::Pending => downloads.unpacked(&self.0, &[]),
            ServicesKeys::Ready(keys) => downloads.unpacked(&self.0, &keys.profile_property_keys),
        }
    }
}

/// The signed-in account's `/minecraft/profile` fetch after joining.
pub(in crate::game) enum OwnProfile {
    Idle,
    Fetching(std::sync::mpsc::Receiver<Option<sunrise_client_render::skins::Textures>>),
    Done(Option<sunrise_client_render::skins::Textures>),
}

pub(in crate::game) struct PendingCauldron {
    pub(in crate::game) hand: Hand,
    pub(in crate::game) hit: Hit,
    pub(in crate::game) attempt: mod_api::InteractionAttempt,
    pub(in crate::game) selected: usize,
    pub(in crate::game) main: Option<ItemStack>,
    pub(in crate::game) off: Option<ItemStack>,
    pub(in crate::game) aim: ItemUseAim,
    pub(in crate::game) swing: [bool; 2],
    pub(in crate::game) sequence: i32,
    pub(in crate::game) item_after_block: bool,
}

#[derive(PartialEq)]
pub(in crate::game) struct ItemUseAim {
    // UseItem carries these angles but not the server's ray origin.
    pub(in crate::game) rotation: (f32, f32),
    pub(in crate::game) eye: Option<[f64; 3]>,
    // The local ray can also come from a tracked input device.
    pub(in crate::game) ray: Option<([f64; 3], [f64; 3])>,
    pub(in crate::game) reach: f64,
}

/// The packet route that starts a remote level entry.
pub enum RemoteDimensionEntry {
    Login,
    Respawn,
    DeathRespawn,
}

pub struct ConstructingClientLevel {
    pub handle: mod_api::LevelRef,
    pub world: sunrise_level::world::World,
    pub facts: Option<Arc<sunrise_world::dimension_type::DimensionFacts>>,
}

pub struct Remote {
    pub(in crate::game) professions: Arc<sunrise_sim::mobs::villager::ResolvedProfessions>,
    pub session: Session,
    /// Display-only address computed at connection setup. Never holds the raw friend invite.
    pub(in crate::game) display_address: Option<String>,
    pub brand: String,
    /// `ClientPacketListener.random` draws packet spread, not provider RNG.
    pub(in crate::game) particle_packet_random: sunrise_core::random::LegacyRandom,
    pub recipe_viewer: crate::game::client_side::recipe_book::ViewerChannels,
    /// This connection's configured feature flags, carried through reconfiguration.
    pub features: std::sync::Arc<[String]>,
    /// The server announced it runs mods' quiet commands.
    pub quiet_commands: bool,
    /// The server advertised private custom recipe-book synchronization.
    pub custom_books: bool,
    /// The synced attachments the server sent, by holder.
    pub attachments: HashMap<Holder, ResidentAttachments>,
    /// The colliders the server sent, by entity id (`sunrise:collider_sync`).
    pub colliders: HashMap<u32, sunrise_sim::colliders::ColliderState>,
    /// The colliders at the last client tick, before any packets handled between ticks.
    pub(in crate::game) collider_shove_start:
        Option<(Dim, sunrise_world::moving_colliders::Prepared)>,
    /// How old the copy of the collider carrying the player is (`sunrise:carry_age`).
    pub carry_age: sunrise_sim::colliders::CarryAge,
    /// The server samples far terrain for us, this far at most (`lod_net`).
    pub far_terrain: Option<sunrise_sim::lod_net::Offer>,
    pub builtin: Arc<Builtin>,
    pub(in crate::game) lang: Arc<Language>,
    pub registries: Arc<Dynamic>,
    /// Data-driven mob variant tables decoded from `registries` in server sync order.
    pub variants: Arc<sunrise_sim::mobs::variants::VariantTables>,
    /// The block, item and fluid tags of `registries` by name, as native mods read them
    /// (`install_remote_tags`).
    pub tags: Option<sunrise_items::items::Items>,
    /// The game's enchantments and the server's that it lacks (a data
    /// pack's), from their synced data; `None` when the server has none.
    pub enchantments: Option<Arc<sunrise_items::enchant::Enchantments>>,
    pub joined: bool,
    /// `ServerReconfigScreen` is up since this tick.
    pub reconfiguring: Option<u64>,
    /// `handleLogin` ran: vanilla's `Minecraft.player` exists from then on,
    /// while here the player is made at the first teleport.
    pub logged_in: bool,
    /// Whether a logical `LocalPlayer` already exists on this connection (retained across
    /// a second `Login`, replaced on `Respawn`, cleared on reconfiguration).
    pub has_logical_player: bool,
    pub(in crate::game) status: String,
    pub entity_id: i32,
    pub local_entity_silent: bool,
    /// `Minecraft.cameraEntity` when the server put the view on another
    /// entity (ClientboundSetCameraPacket); None while it is the player.
    pub camera: Option<i32>,
    /// Detached camera snapshot retained after `RemoveEntities` until `SetCamera` replaces it.
    pub(in crate::game) detached_camera:
        Option<Box<crate::game::client_side::remote_camera::ResolvedCameraEntity>>,
    pub(in crate::game) dimension: DimensionType,
    /// The level's key (`minecraft:overworld`): where its far terrain is kept.
    pub level: String,
    /// `level`'s key, as mods' handles name the level.
    pub level_key: mod_api::DimensionKey,
    /// The level's facts from the dimension type this connection's server sent (`None` when it
    /// sent none the game takes): owned by the connection, so entering a level touches no other
    /// level's facts, the integrated server's included.
    pub level_facts: Option<Arc<sunrise_world::dimension_type::DimensionFacts>>,
    /// The sky, fog and cloud attributes (and timelines) of the level from the server's type over
    /// the client's own environment of that type: what gameplay reads. `None` when the type is
    /// refused, or has no data at all: neither the server sent it nor the client's resources hold
    /// it (`fill_known_entries` fills a known pack's entries from the client's own files).
    pub level_environment: Option<Arc<sunrise_level::environment::Environment>>,
    /// The same over the packs' Polytone stack of that type, what the frame draws (`None` without
    /// pack modifiers, or for a type the packs have no stack of: any but the three vanilla IDs).
    pub level_pack_environment: Option<Arc<sunrise_level::environment::Environment>>,
    /// The level's synchronized dimension type, which both environments are built from again
    /// when the packs change.
    pub(in crate::game) synced_type: Option<(String, serde_json::Value)>,
    /// Both environments by synchronized type name, with what they were built from, so that a
    /// level of an unchanged type is entered again without reading the biome files.
    pub(in crate::game) level_environments: HashMap<String, level_format::LevelEnvironments>,
    /// Connection-owned facts, cached by level and the full named type identity.
    pub(in crate::game) facts_cache: HashMap<
        String,
        (
            String,
            serde_json::Value,
            Arc<sunrise_world::dimension_type::DimensionFacts>,
        ),
    >,
    /// The levels this connection added to the process's table of levels: the only ones it drops.
    pub(in crate::game) added_levels: std::collections::HashSet<String>,
    /// `ClientPlayerNetworkEvent.LoggingOut` was fired for this connection: every way out
    /// of a world reports it once.
    pub(in crate::game) logged_out: bool,
    /// The other levels' worlds the server opened on `sunrise:levels` (`viewed_worlds`).
    pub viewed: crate::game::client_side::viewed_worlds::ViewedWorlds,
    /// The level's hashed seed (login, respawn): the folder of its kept chunks
    /// (`server_terrain::level_dir`).
    pub(in crate::game) hashed_seed: i64,
    /// `ClientLevel.getSeaLevel`: from the packet that created this level.
    pub(in crate::game) sea_level: i32,
    /// What chunk packets decode with (the dimension's height and sky, the
    /// biome registry); replaced whenever one of them changes.
    pub chunk_format: Arc<ChunkFormat>,
    /// Decodes chunk packets as they arrive, off the frame.
    pub decoder: ChunkDecoder,
    /// Options.serverRenderDistance: the server's view distance from the
    /// login or the last chunk-cache-radius packet.
    pub view_distance: i32,
    /// `ClientLevel.serverSimulationDistance`: from the login or the last
    /// simulation-distance packet.
    pub(in crate::game) simulation_distance: i32,
    /// `BlockStatePredictionHandler.currentSequenceNr`: the level's, so a new level (a login,
    /// another dimension) counts from 0 again.
    pub sequence: i32,
    /// `BlockStatePredictionHandler.lastTeleportSequence`: the sequence of the last
    /// teleport, so a prediction made before it keeps its block but not its snap.
    pub last_teleport_sequence: i32,
    pub(in crate::game) position_sender: sunrise_client_lib::player_sync::PositionSender,
    pub last_input: u8,
    pub was_sprinting: bool,
    /// The slot the server last heard (`MultiPlayerGameMode.carriedIndex`): a cell, so `send` can
    /// send a new one first (`ensureHasSentCarriedItem`).
    pub(in crate::game) carried: std::cell::Cell<Option<usize>>,
    pub(in crate::game) was_shared_flags: u8,
    pub(in crate::game) was_living_flags: u8,
    pub(in crate::game) last_recorded_pose: Option<sunrise_sim::pose::Pose>,
    /// Live `LocalPlayer.abilities`, mutated in arrival order by `CHANGE_GAME_MODE` and `PlayerAbilities`.
    pub(in crate::game) abilities: sunrise_sim::lan::abilities::PlayerAbilities,
    /// The old player's motion, from a Respawn with KEEP_ENTITY_DATA
    /// (`handleRespawn`: `newPlayer.setDeltaMovement(oldPlayer.getDeltaMovement())`),
    /// for the player made at the next teleport: a portal's teleport adds to it.
    pub(in crate::game) respawn_velocity: Option<Vec3>,
    /// ServerboundPlayerLoadedPacket went out: the tick's keybinds act from then on.
    pub loaded_sent: bool,
    /// The dimension of the client level the mods heard made
    /// (`native_client_level_load`), to be told dropped once.
    pub(in crate::game) heard_level: Option<String>,
    pub datapacks: sunrise_sim::datapack_net::Receiver,
    /// Decoded vanilla codec values for [`Self::datapacks`], swapped on join and reload.
    pub datapack_vanilla_values:
        Option<Arc<sunrise_gameplay_common::game::native_datapack_values::DatapackVanillaValues>>,
    /// The constructor's level is visible by name during Load, before Minecraft.setLevel.
    pub constructing_level: Option<ConstructingClientLevel>,
    /// `ClientPacketListener.levelLoadTracker`: the loading screen after a login
    /// or respawn, until the player's section is shown.
    pub level_load: sunrise_client_render::level_loading::LevelLoadTracker,
    /// The ClientInformation last sent (`Options.broadcastOptions` sends it
    /// again when a view distance, Chat, skin or Main Hand option changes).
    pub(in crate::game) sent_info: sunrise_protocol::packets::configuration::ClientInformation,
    pub group_count: sunrise_level::chunk_map::ConnectionGroupCount,
    /// BlockStatePredictionHandler: blocks changed ahead of the server (a predicted break),
    /// with what it remembers of each of them. Until the server acknowledges the
    /// sequence its block updates only replace the remembered state; then the block
    /// takes it.
    pub(in crate::game) predictions: HashMap<[i32; 3], Prediction>,
    /// The entities the level held when its `LocalPlayer` was added
    /// (`ClientLevel.addEntity` on login or respawn): `ClientLevel.tickEntities`
    /// ticks in the order entities were added, so these tick before the
    /// player and every later one after it.
    pub ticked_before_player: std::collections::HashSet<u32>,
    /// Open container: (window id, menu name).
    pub menu: ClientMenu,
    /// The crafting result slot of the open menu as the server last set it
    /// (`CraftingMenu.slotChangedCraftingGrid`): the client never computes it.
    pub craft_result: Option<ItemStack>,
    /// The open menu's title as the server named it, with its styles.
    pub menu_title: Option<sunrise_game_data::text::Text>,
    pub(in crate::game) menu_title_snapshot: Option<Arc<mod_api::Text>>,
    pub state_id: i32,
    /// Window 0's state and carried stack while another menu is active.
    pub(in crate::game) inventory_state_id: i32,
    pub(in crate::game) inventory_carried: Option<ItemStack>,
    pub(in crate::game) inventory_craft_grid: [Option<ItemStack>; 4],
    pub(in crate::game) inventory_craft_result: Option<ItemStack>,
    pub(in crate::game) use_target: Option<[i32; 3]>,
    pub(in crate::game) cauldron_entries:
        Option<sunrise_sim::cauldron::interactions::Dispatcher<()>>,
    /// Receipts are connection-scoped; unlike block prediction sequences, they
    /// are never reset on a level change.
    pub(in crate::game) cauldron_receipt: i64,
    pub(in crate::game) pending_cauldrons: std::cell::RefCell<HashMap<i64, PendingCauldron>>,
    /// A mod menu's extra data, kept for the `OpenScreen` of its window that
    /// follows (`sunrise:menu_extra`).
    pub(in crate::game) menu_extra: Option<sunrise_protocol::menu_extra::Message>,
    pub entities: HashMap<i32, Tracked>,
    /// Synced entity attributes keyed by entity id and attribute id.
    pub(in crate::game) synced_attributes:
        HashMap<i32, HashMap<sunrise_world::attributes::entity::AttributeId, f64>>,
    /// Per-tick remote entity scratch retained between frames.
    pub(in crate::game) tick_fluids: Vec<Option<sunrise_sim::mobs::living::Fluids>>,
    pub(in crate::game) tick_rain: Vec<bool>,
    pub(in crate::game) tick_standings: Vec<Option<String>>,
    pub(in crate::game) tick_water_fx: Vec<sunrise_sim::mobs::MobEvent>,
    /// Complete network riding graph, including invisible plugin seat entities.
    pub passenger_vehicles: HashMap<u32, PassengerSeat>,
    pub(in crate::game) lerps: HashMap<i32, Lerp>,
    /// A native mod's general rows the server shows the window (`general.rs`).
    pub general: general::RemoteGenerals,
    /// The minecarts the server moves with `ClientboundMoveMinecartPacket`
    /// (`NewMinecartBehavior`), which have no interpolation handler.
    pub(in crate::game) cart_lerps: HashMap<i32, CartLerp>,
    /// Living entities' `SteppedInterpolationHandler`s.
    pub(in crate::game) stepped: HashMap<i32, SteppedInterpolation>,
    /// `LivingEntity.lerpHeadTo`: the head yaw and the ticks left to reach it.
    pub(in crate::game) head_lerps: HashMap<i32, (f32, u32)>,
    /// The happy ghast the player just got off, until the server's teleport
    /// puts the player on top of it (`place_ghast_under`).
    pub ghast_dismount: Option<u32>,
    /// Whether the player is the first passenger of its vehicle, the one
    /// `getControllingPassenger` names.
    pub first_passenger: bool,
    /// `Entity.tickCount` of the local player: its ticks in the client level since the player
    /// was made (Login or Respawn, then the server's first teleport).
    pub(in crate::game) own_tick_count: i32,
    /// The player's place in its vehicle's `getPassengers`.
    pub(in crate::game) passenger_index: usize,
    /// Item entities waiting for their item data: where they are and their motion.
    pub(in crate::game) pending_items: HashMap<i32, PendingItem>,
    pub health: (f32, i32, f32),
    /// `LocalPlayer.flashOnSetHealth`: set by the first health the server
    /// sends; from then on a drop starts the hurt animation.
    pub(in crate::game) flash_on_set_health: bool,
    pub game_mode: i32,
    pub chat: Vec<(String, Instant)>,
    /// Lines received since joining, for the control socket's events.
    pub chat_received: u64,
    /// `ChatListener.delayedMessageQueue`: player chat held back by Chat
    /// Delay, with the signature a deletion names it by and what the
    /// narrator says once it shows.
    pub(in crate::game) chat_queue: VecDeque<QueuedChat>,
    /// `ClientPacketListener.messageSignatureCache`: the signatures packed
    /// ones index, as the server's copy for this connection holds them.
    pub(in crate::game) signatures: sunrise_protocol::secure_chat::SignatureCache,
    /// `ChatListener.previousMessageTime`: when a message last showed, if one has.
    pub(in crate::game) chat_previous: Option<Instant>,
    /// `ClientPacketListener.nextChatIndex`: the global index the next chat packet must carry.
    pub(in crate::game) next_chat_index: i32,
    /// `serverEnforcesSecureChat`, from the login packet.
    pub(in crate::game) enforces_secure_chat: bool,
    pub(in crate::game) services_keys: Arc<std::sync::Mutex<ServicesKeys>>,
    /// The `RemoteChatSession.Data` each player's list entry last carried (INITIALIZE_CHAT).
    pub(in crate::game) chat_sessions: HashMap<u128, Option<(u128, i64, Vec<u8>, Vec<u8>)>>,
    /// `PlayerInfo.messageValidator` of the players whose chat has been read.
    pub chat_validators: HashMap<u128, secure_chat::MessageValidator>,
    /// `ConnectScreen.lastNarration`: when "Joining the world" was last said.
    pub(in crate::game) joining_narrated: Option<Instant>,
    /// The tree the chat colours, completes and parses with: the server's
    /// (ClientboundCommandsPacket) once it has sent one, else the game's own,
    /// plus the commands native mods own (`chat::chat_tree_for`). Rebuilt where
    /// that base changes, not at each keystroke.
    pub chat_tree: sunrise_command::Dispatcher,
    pub(in crate::game) selector_metadata_supported: bool,
    /// The server's original tree, used for signing and verifying remote commands.
    /// Vanilla uses an empty tree until it receives `ClientboundCommandsPacket`.
    pub(in crate::game) server_chat_tree: Option<sunrise_command::Dispatcher>,
    /// The chat session, the last-seen window and the signature cache.
    pub secure_chat: crate::game::client_side::remote_chat::SecureChat,
    /// `ClientSuggestionProvider.customCompletionSuggestions`: the extra chat
    /// completions this server sent (ClientboundCustomChatCompletionsPacket).
    pub(in crate::game) custom_completions: std::collections::BTreeSet<String>,
    pub(in crate::game) players: HashMap<u128, String>,
    /// `ClientPacketListener.playerInfoMap` as player-list entries, and which of them are listed.
    pub tab_players: HashMap<u128, sunrise_level::profile_lookup::TabPlayer>,
    /// Live `PlayerInfo.gameMode` cell per active tab entry; `PlayerInfoRemove` drops the map
    /// slot while leaving any entity's cached `Arc` untouched (`AbstractClientPlayer.playerInfo`).
    pub tab_game_modes: HashMap<u128, Arc<std::sync::atomic::AtomicI32>>,
    /// First resolved `PlayerInfo` game mode cell of the current `LocalPlayer` entity.
    pub own_player_info: std::cell::OnceCell<Arc<std::sync::atomic::AtomicI32>>,
    /// Each listed profile's `textures` property, verified once the keys are ready.
    pub(in crate::game) skin_textures: HashMap<u128, Arc<PlayerTextures>>,
    /// Active skin and cape from the services profile when the server sent none.
    pub(in crate::game) own_profile: OwnProfile,
    /// The accepted body, cape and elytra of each profile after its downloads settle.
    pub(in crate::game) skins: HashMap<u128, sunrise_client_render::skins::ResolvedSkin>,
    pub(in crate::game) skins_revision: u64,
    pub(in crate::game) published_skins: Option<(usize, u128, u64, u64)>,
    pub listed: std::collections::HashSet<u128>,
    /// `ClientboundLoginPacket.onlineMode`: the player list shows heads.
    pub(in crate::game) online_mode: bool,
    /// `ClientboundLoginPacket.levels`: the server's level keys, which
    /// `ClientPacketListener.levels()` hands to command suggestions.
    pub(in crate::game) levels: Vec<String>,
    /// Player entities in the level: entity id to profile UUID.
    pub level_players: HashMap<i32, u128>,
    /// Authoritative entity UUID index scoped to the current client level (`EntityLookup.byUuid`).
    pub(in crate::game) level_uuids: HashMap<u128, i32>,
    /// Reverse entity-id-to-UUID index for O(1) removal and replacement in the current client level.
    pub(in crate::game) level_entity_uuids: HashMap<i32, u128>,
    /// First resolved `PlayerInfo` game mode cell per remote `AbstractClientPlayer` entity.
    pub(in crate::game) level_player_infos:
        HashMap<i32, std::cell::OnceCell<Arc<std::sync::atomic::AtomicI32>>>,
    /// The profile name each player entity was created with (`RemotePlayer.getGameProfile`): it outlives the
    /// player-list entry, which servers remove to hide their NPCs from the list.
    pub level_player_names: HashMap<i32, String>,
    /// The `textures` each player entity found in the player list first
    /// (`AbstractClientPlayer.getPlayerInfo` keeps that `PlayerInfo`): its skin
    /// outlives the entry, which NPC plugins remove a moment after spawning.
    pub(in crate::game) level_player_textures: HashMap<i32, Arc<PlayerTextures>>,
    /// Entities with shared flag 6 (`Entity.isCurrentlyGlowing` on a client); mobs keep
    /// theirs in `Mob::glowing`.
    pub glowing: std::collections::HashSet<i32>,
    /// The player's own shared flag 5 (`Entity.isInvisible`) as the server last set it.
    pub(in crate::game) own_invisible: bool,
    /// The player's own shared flag 0 (`Entity.isOnFire`) as the server last
    /// set it: its fire (blaze fireballs, fire aspect, lightning) never
    /// reaches the window's own `fire_ticks` otherwise.
    pub(in crate::game) own_on_fire: bool,
    /// `LivingEntity.DATA_ARROW_COUNT_ID` and `DATA_STINGER_COUNT_ID` of the
    /// own player, as the server synced them.
    pub(in crate::game) own_arrows: i32,
    pub(in crate::game) own_stingers: i32,
    /// `Player.DATA_SHOULDER_PARROT_LEFT` and `_RIGHT` of the own player.
    pub(in crate::game) own_shoulders: [Option<i32>; 2],
    /// The local player's profile UUID (always in the level).
    pub profile: u128,
    /// The local player's profile name: its score holder and team member.
    pub profile_name: String,
    /// `MultiPlayerGameMode.destroyBlockPos`, `.destroyProgress` and the face last sent: the block
    /// the break targets, which outlives the break itself.
    pub destroying: Option<([i32; 3], u8, f32)>,
    /// `MultiPlayerGameMode.destroyingItem`: the main-hand stack the break began with.
    pub destroying_item: Option<ItemStack>,
    /// `MultiPlayerGameMode.isDestroying`: a break is running. `destroying` keeps the target after
    /// one ends (`destroyBlockPos` is written only when a break starts), so a dig resumed on the
    /// same block and item sends no new START and the server keeps its clock (#4124).
    pub is_destroying: bool,
    /// Per-hand swing policy, restored when the current input attempt ends.
    pub interaction_swing: [bool; 2],
    pub destroy_delay: u32,
    /// Whether the main and off hand still wait for the server's answer to
    /// a SWAP_ITEM_WITH_OFFHAND (`remote_hand_slot_set`).
    pub(in crate::game) swap_answer: [bool; 2],
    pub(in crate::game) stop_mining: bool,
    /// `CommonPlayerSpawnInfo.isFlat`: the level is a superflat one.
    pub(in crate::game) flat: bool,
    pub(in crate::game) death_message: Option<sunrise_game_data::text::Text>,
    /// `LocalPlayer.showDeathScreen`: off with `immediate_respawn`.
    pub show_death_screen: bool,
    /// `Player.isReducedDebugInfo`: the server's `reduced_debug_info` rule.
    pub(in crate::game) reduced_debug_info: bool,
    /// `LocalPlayer.doLimitedCrafting`: whether limited crafting is enabled.
    pub(in crate::game) do_limited_crafting: bool,
    pub player_type: Option<Arc<MobType>>,
    /// Inventory, off hand and armour as last synchronised, for creative-mode
    /// slot updates (`creative_sync_stacks`).
    pub synced: Vec<Option<ItemStack>>,
    pub(in crate::game) decode_errors: u32,
    /// Connection-owned `ClientClockManager` state, retained across dimensions.
    pub(in crate::game) clocks: HashMap<i32, play::ClockState>,
    pub(in crate::game) default_clock: Option<i32>,
    pub(in crate::game) overworld_clock: Option<i32>,
    pub(in crate::game) clock_game_time: i64,
    /// `ClientChunkCache.Storage`'s view centre (ClientboundSetChunkCacheCenterPacket).
    pub(in crate::game) chunk_center: (i32, i32),
    /// A viewed-level swap adopts columns before the server sends the active cache centre.
    pub(in crate::game) awaiting_viewed_chunk_center: bool,
    pub chunk_light: RemoteLight,
    /// Received events a frame left for the next one, in order, and when
    /// the oldest of them first waited (`poll_remote`).
    pub backlog: VecDeque<Received>,
    pub(in crate::game) backlog_since: Option<Instant>,
    /// `ClientCommonPacketListenerImpl.serverLinks`: the links "Server
    /// Links..." in the pause menu lists (only `http`/`https` URIs kept).
    pub(in crate::game) server_links: Vec<sunrise_protocol::packets::configuration::ServerLink>,
    /// `ClientPacketListener.getDebugQueryHandler`: the F3+I query waiting for the server's data.
    pub(in crate::game) debug_query: crate::game::client_side::debug_keys::DebugQuery,
}

/// The time a frame spends on chunk packets before it leaves the rest of
/// the received events for the next frame. A batch of 25 to 60 columns
/// (0.1 ms each on an M4 Pro) all in one frame was a 3 to 8 ms frame every
/// time the player crossed a chunk border at 32 chunks; they decode on the
/// chunk decoder's thread now, and the frame only inserts them.
pub(in crate::game) const CHUNK_PACKETS_PER_FRAME: Duration = Duration::from_millis(1);
/// Events that have waited this long are all handled in the next frame,
/// whatever inserting their columns costs, up to a column still decoding:
/// the packets after a slow machine's chunks don't wait on its frames.
pub(in crate::game) const BACKLOG_LIMIT: Duration = Duration::from_millis(50);
/// How long a tick about to run waits for one chunk column still decoding;
/// past it the column, and what came after it, wait for the next frame.
pub(in crate::game) const TICK_DECODE_WAIT: Duration = Duration::from_millis(250);
#[derive(Default)]
pub struct ClientMenu {
    pub(in crate::game) current: Option<(i32, String)>,
    pub generation: u64,
}

impl ClientMenu {
    pub fn set(&mut self, current: Option<(i32, String)>) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("menu generation exhausted");
        self.current = current;
    }

    pub fn as_ref(&self) -> Option<&(i32, String)> {
        self.current.as_ref()
    }
}

/// The textures to resolve skins from: every list entry's, and for a player
/// entity whose entry is gone the profile it kept. `kept` first takes, for each
/// player entity without one, its entry's textures (`AbstractClientPlayer.getPlayerInfo`
/// looks the PlayerInfo up once and holds it, removed from the list or not).
pub fn skin_entries(
    level_players: &HashMap<i32, u128>,
    listed: &HashMap<u128, Arc<PlayerTextures>>,
    kept: &mut HashMap<i32, Arc<PlayerTextures>>,
) -> Vec<(u128, Arc<PlayerTextures>)> {
    for (id, uuid) in level_players {
        if !kept.contains_key(id)
            && let Some(textures) = listed.get(uuid)
        {
            kept.insert(*id, textures.clone());
        }
    }
    let mut entries: Vec<_> = listed.iter().map(|(u, t)| (*u, t.clone())).collect();
    for (id, textures) in kept.iter() {
        if let Some(uuid) = level_players.get(id)
            && !listed.contains_key(uuid)
            && !entries.iter().any(|(u, _)| u == uuid)
        {
            entries.push((*uuid, textures.clone()));
        }
    }
    entries
}

/// `Util.getMillis` for the level load tracker's timeouts.
pub(in crate::game) fn client_millis() -> u64 {
    static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    ORIGIN
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    // remote_effect_network_id: remote/sync.rs
    pub fn item_id(&self, name: &str) -> i32 {
        let r = self.client.remote.as_ref().expect("remote");
        r.builtin
            .get("minecraft:item")
            .and_then(|i| i.id(name))
            .unwrap_or(0)
    }

    pub(in crate::game) fn to_network_projecting(
        &self,
        s: &Option<ItemStack>,
        projected: &dyn Fn(&sunrise_world::component_value::consume_effects::Admitted),
    ) -> Option<Stack> {
        let r = self.client.remote.as_ref()?;
        // A stack as the server sent it, only its count changed: its own
        // components go back, as a vanilla client keeps them.
        let effect_mapping = r
            .registries
            .consume_effect_codecs
            .as_ref()
            .map(|snapshot| snapshot.identity());
        let source = s.as_ref()?;
        if let Some(patch) = source.wire_patch_for(effect_mapping) {
            let mut out = Stack::new(self.item_id(&source.item), source.count as i32);
            let safe = r.registries.custom_components
                || sunrise_protocol::item::custom::entries().next().is_none()
                || sunrise_protocol::item::WirePatch::decode_reader(
                    sunrise_protocol::buf::Reader::new(&patch).with_names(&*r.registries),
                )
                .and_then(|(added, removed)| {
                    let decoded = Stack {
                        added,
                        removed,
                        ..Stack::new(out.item, out.count)
                    };
                    sunrise_protocol::item::custom::permitted_with_names(
                        &decoded,
                        false,
                        &*r.registries,
                    )
                })
                .unwrap_or(false);
            if safe {
                out.wire = sunrise_protocol::item::WirePatch(Some(patch));
                return Some(out);
            }
            log::warn!("Cannot send cached custom components to this peer");
        }
        let mut wire = sunrise_sim::lan::guest::stack_to_network_projecting(
            |name| self.item_id(name),
            // Built-in first, as a hub's `SyncData::id` does: potions are one.
            |registry, name| {
                if registry == sunrise_protocol::item::consume_effects::REGISTRY {
                    return r.registries.consume_effect_id(name);
                }
                r.builtin
                    .get(registry)
                    .and_then(|ids| ids.id(name))
                    .or_else(|| r.registries.id(registry, name))
                    .or_else(|| {
                        (r.registries.custom_components
                            && registry == "minecraft:data_component_type")
                            .then(|| sunrise_protocol::item::custom::id(name))
                            .flatten()
                    })
            },
            s,
            projected,
        )?;
        sunrise_sim::lan::guest::canonicalize_stack_patch(
            &mut wire,
            source,
            self.items(),
            &r.builtin,
        );
        Some(wire)
    }

    /// The account playing this game.
    pub(in crate::game) fn auth(
        &self,
    ) -> Option<std::sync::Arc<sunrise_client_render::auth::AuthSession>> {
        sunrise_client_render::auth::current()
    }

    /// `ClientSuggestionProvider.customCompletionSuggestions` of the server
    /// connection (none without one).
    pub(in crate::game) fn remote_custom_completions(&self) -> std::collections::BTreeSet<String> {
        self.client
            .remote
            .as_ref()
            .map(|r| r.custom_completions.clone())
            .unwrap_or_default()
    }

    /// The names in the server's player list, sorted.
    pub(in crate::game) fn remote_online_names(&self) -> Option<Vec<String>> {
        let r = self.client.remote.as_ref()?;
        let mut names: Vec<String> = r.players.values().cloned().collect();
        names.sort();
        Some(names)
    }

    /// The entity type of the boat, minecart, armor stand or End crystal with
    /// server id `id` that an attack reaches `Player.attack`'s sweep check on,
    /// and whether it is living. A painting or item frame is not: on this
    /// client `skipAttackInteraction` hurts it and stops the attack there.
    pub fn remote_attacked_object(&self, id: i32) -> Option<(String, bool)> {
        let id = id as u32;
        if let Some(b) = self.common.boats.iter().find(|b| b.id == id) {
            return Some((b.entity_type(), false));
        }
        if let Some(c) = self.common.carts.iter().find(|c| c.id == id) {
            return Some((c.kind.id().to_owned(), false));
        }
        if self.common.armor_stands.iter().any(|a| a.id == id) {
            return Some(("minecraft:armor_stand".to_owned(), true));
        }
        self.common
            .crystals
            .iter()
            .any(|c| c.id == id)
            .then(|| ("minecraft:end_crystal".to_owned(), false))
    }

    /// An attribute's value on this client's player, if its map has the attribute.
    pub(in crate::game) fn own_attribute(&mut self, id: &str) -> Option<f64> {
        let p = self.common.player.as_mut()?;
        let key = p.attributes.key(id)?;
        p.attributes.lookup(Some(&key))?;
        p.attributes
            .value(Some(key), sunrise_world::attribute_map::ValueKind::Computed)
            .ok()
    }

    /// Connected or connecting with no player yet: `ConnectScreen`, then
    /// `LevelLoadingScreen`, over the panorama with its blur.
    pub fn remote_loading(&self) -> bool {
        if self.client.replay.playing() {
            return false;
        }
        use sunrise_client_render::level_loading::Reason;
        self.client.remote.as_ref().is_some_and(|r| {
            if r.level_load.is_active() {
                // The portal screens have no panorama, so no blur.
                r.level_load.reason == Reason::Other
            } else {
                self.common.player.is_none()
            }
        })
    }

    /// Shared flag 6 of an entity on a server's client (`None` without a server).
    pub(in crate::game) fn remote_glowing(&self, id: u32) -> Option<bool> {
        let r = self.client.remote.as_ref()?;
        Some(r.glowing.contains(&(id as i32)))
    }

    /// The profile name of another player the server sent, by entity id.
    pub(in crate::game) fn remote_player_name(&self, id: u32) -> Option<&str> {
        sunrise_gameplay_common::game::side_ops::player::SideOps::side_remote_player_name(
            &self.client,
            id,
        )
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;
