//! A remote player: connection, position, chunk and entity views, and the
//! player state (inventory, health, game mode) that the host's gameplay code
//! operates on when it acts for them.
use super::{Joined, PlayerSender, entity::Tracker};
use crate::item_sync::net_stack::network_stack_projecting;
pub use crate::item_sync::net_stack::{
    canonicalize_stack_patch, firework_components, stack_to_network, stack_to_network_projecting,
};
use crate::{
    items::ItemStack,
    motion::PlayerMotion,
    pose::{Dimensions, Pose},
};
use std::{
    collections::VecDeque,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicI32, AtomicU64, Ordering},
    },
};
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_protocol::{
    attachment_sync,
    entity_data::{Value, serializer},
    item::Stack,
    packets::{
        configuration::{ClientInformation, CustomPayload},
        play,
    },
    passage_sync,
};
use sunrise_world::component_value::consume_effects::Admitted;
use sunrise_world::native_load_context::LoopContext;

/// Unique across reconnects and server instances in this process, even when
/// the protocol's container id wraps back to the same number.
static NEXT_MENU_GENERATION: AtomicU64 = AtomicU64::new(1);

fn menu_generation() -> u64 {
    let mut generation = NEXT_MENU_GENERATION.load(Ordering::Relaxed);
    loop {
        let next = generation
            .checked_add(1)
            .expect("menu generation exhausted");
        match NEXT_MENU_GENERATION.compare_exchange_weak(
            generation,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return generation,
            Err(current) => generation = current,
        }
    }
}

/// GameType ids.
pub const SURVIVAL: i32 = 0;
pub const CREATIVE: i32 = 1;
pub const SPECTATOR: i32 = 3;

/// The body state a player's file keeps that lives on `PlayerMotion`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SavedBody {
    pub fire_ticks: i32,
    pub air: i32,
    pub fall_distance: f64,
    /// `Motion`: the delta movement, blocks a tick.
    pub velocity: [f64; 3],
    /// `foodExhaustionLevel` and `foodTickTimer` (FoodData); the food level
    /// and saturation live on the guest.
    pub food_exhaustion: f32,
    pub food_tick_timer: i32,
    /// `TicksFrozen`: the time spent in powder snow.
    pub ticks_frozen: i32,
    /// `Invulnerable` and its `invulnerableTime` window (#916), the same
    /// tags the host's own save keeps.
    pub invulnerable: bool,
    pub invulnerable_time: i32,
}

impl SavedBody {
    /// The body state of `motion`, to save.
    pub fn of(motion: &PlayerMotion) -> Self {
        let v = motion.velocity;
        Self {
            fire_ticks: motion.fire_ticks,
            air: motion.air,
            fall_distance: motion.fall_distance,
            velocity: [v.x, v.y, v.z],
            food_exhaustion: motion.health.food.exhaustion,
            food_tick_timer: motion.health.food.timer,
            ticks_frozen: motion.ticks_frozen,
            invulnerable: motion.health.permanently_invulnerable,
            invulnerable_time: motion.health.invulnerable_time,
        }
    }
    /// Put it on a new body.
    pub fn apply(self, motion: &mut PlayerMotion) {
        motion.fire_ticks = self.fire_ticks;
        motion.air = self.air;
        motion.fall_distance = self.fall_distance;
        let [x, y, z] = self.velocity;
        motion.velocity = Vec3::new(x, y, z);
        motion.health.food.exhaustion = self.food_exhaustion;
        motion.health.food.timer = self.food_tick_timer;
        motion.ticks_frozen = self.ticks_frozen;
        motion.health.permanently_invulnerable = self.invulnerable;
        motion.health.invulnerable_time = self.invulnerable_time;
    }
}

/// An effect as last sent in ClientboundUpdateMobEffectPacket.
#[derive(Clone, Debug, PartialEq)]
pub struct SentEffect {
    pub id: &'static str,
    pub amplifier: i32,
    pub flags: i8,
    pub duration: i32,
    pub tick_count: i32,
}

/// ServerGamePacketListenerImpl.CLIENT_LOADED_TIMEOUT_TIME.
pub const CLIENT_LOADED_TIMEOUT: u8 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestKind {
    Network,
    Fake,
}

struct GuestInit {
    conn: u32,
    uuid: u128,
    name: String,
    properties: Vec<sunrise_core::profile::AccountProperty>,
    info: ClientInformation,
    brand: String,
    sender: PlayerSender,
    latency: Arc<AtomicI32>,
    address: SocketAddr,
    expected: bool,
}

/// One physical connection's ordered main-thread input. Provider credit follows
/// the value through the inbox and derived HostEvent segments, never a wire-size
/// refund/re-reserve gap. Raw work is not subject to vanilla movement freezing.
pub enum InboxItem {
    Packet(
        play::Serverbound,
        Option<crate::raw_payloads::ingress::ReceiveLease<super::NetEvent>>,
    ),
    Raw(
        Box<crate::raw_payloads::Delivery>,
        crate::raw_payloads::ingress::ReceiveLease<super::NetEvent>,
    ),
}
#[cfg(test)]
impl InboxItem {
    /// Unit-test input without a network provider reservation.
    pub fn uncredited(packet: play::Serverbound) -> Self {
        Self::Packet(packet, None)
    }
}

/// The body state at a single accepted move, before later input or reports.
#[derive(Clone, Copy, Debug)]
pub struct GuestMove {
    pub position: [f64; 3],
    pub delta: [f64; 3],
    pub on_ground: bool,
    pub sneaking: bool,
}

pub struct Guest {
    /// Identity generation and journal binding travel with the actual player record.
    pub residency: crate::join_seq::residency::member::Member,
    pub kind: GuestKind,
    pub conn: u32,
    address: SocketAddr,
    ip_address: String,
    /// The hub tick of the player's last action (ServerPlayer.lastActionTime),
    /// for player-idle-timeout.
    pub last_action_tick: u64,
    /// The next ClientboundPlayerChatPacket.globalIndex for this connection.
    pub chat_index: i32,
    /// Secure chat (ServerGamePacketListenerImpl): the player's message
    /// chain, keyed once it sends a chat session; the messages its
    /// acknowledgements cover; the signatures its client caches.
    pub chain: sunrise_protocol::secure_chat::Chain,
    pub last_seen: sunrise_protocol::secure_chat::LastSeenValidator,
    pub signatures: sunrise_protocol::secure_chat::SignatureCache,
    /// The hub tick the player was placed on: chunks go out from the tick
    /// after next, behind the first doTick's packets, as vanilla's do.
    pub placed_at: u64,
    /// The hub tick the guest entered its level (a join or a level change): `player_move` does not
    /// hear the tick after it.
    pub level_entered: u64,
    /// The last location a movement report was heard at, or a server-side teleport sent the player
    /// to (position, `[yaw, pitch]`): what `player_move` compares reports with and reports as `from`.
    /// It is kept apart from the accepted position and may differ from it.
    pub last_reported: ([f64; 3], [f32; 2]),
    /// The chunk cache centre placeNewPlayer already sent.
    pub center_sent: Option<(i32, i32)>,
    /// Why the connection closed (the console's "lost connection").
    pub left_reason: String,
    /// The connections listed ahead of this player when the hub took them out of its list
    /// (`Hub::remove_guest`): the game lists them again in that place while the mods hear them
    /// leave.
    pub listed_behind: Vec<u32>,
    /// A validated movement report the mods have not ruled on yet (`player_move`).
    pub pending_move: Option<super::hub::PendingMove>,
    /// A disconnect was started ([`Guest::begin_disconnect`]); the connection closes when the
    /// server next polls it, and until then no other disconnect is sent.
    pub disconnect_pending: bool,
    pub uuid: u128,
    pub name: String,
    /// The profile's properties, for the player list (skins).
    pub properties: Vec<sunrise_core::profile::AccountProperty>,
    pub entity_id: i32,
    pub sender: PlayerSender,
    pub latency: Arc<AtomicI32>,
    pub info: ClientInformation,
    pub brand: String,
    pub recipe_sync: super::recipes::PlayerRecipeSync,
    /// The chunk radius this client is sent.
    pub view_distance: i32,
    pub position: [f64; 3],
    /// Where the player's chunk tickets stay while they are not at `position`.
    /// `None`: they are at `position`'s chunk. `Some(chunk)`: a teleport sent
    /// the player away and they are still at the chunk it left. The hold ends
    /// (back to `None`) at an accepted move, the teleport's ack, a passenger or
    /// vehicle move, a camera change and a placement.
    pub held_ticket_chunk: Option<(i32, i32)>,
    pub previous_position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub head_yaw: f32,
    pub on_ground: bool,
    pub game_mode: i32,
    /// `previousPlayerGameType`: the mode left last, which the game mode
    /// switcher goes back to; none until the mode first changes.
    pub previous_game_mode: Option<i32>,
    /// The permission level (0 none, 2 game masters, 4 owners).
    pub permission: u8,
    pub sneaking: bool,
    /// The last `ServerboundPlayerInputPacket` flags (forward 1, backward 2,
    /// left 4, right 8, jump 16, shift 32, sprint 64).
    pub input: u8,
    /// The boat or minecart the player rides (`Entity.getVehicle`).
    pub vehicle: Option<u32>,
    /// The player's place in its vehicle's `getPassengers`, from `Game::boarding_order`, beside
    /// the mobs' `vehicle_order`: the lowest goes first, so the smallest among a boat's players
    /// is its controlling rider (`getControllingPassenger`).
    pub vehicle_order: i64,
    /// `ServerGamePacketListenerImpl.lastGoodX/Y/Z` of the vehicle it controls.
    pub vehicle_good: Option<[f64; 3]>,
    pub sprinting: bool,
    pub fall_flying: bool,
    pub sleeping: bool,
    /// `LivingEntity.DATA_LIVING_ENTITY_FLAGS`: 1 while using an item, 2 if
    /// it is in the off hand.
    pub living_flags: i8,
    /// `LivingEntity.DATA_ARROW_COUNT_ID` and `DATA_STINGER_COUNT_ID`.
    pub stuck: (i32, i32),
    /// `Player.DATA_SHOULDER_PARROT_LEFT` and `_RIGHT`: the parrots' variants.
    pub shoulders: [Option<i32>; 2],
    /// The bed (its head) the player sleeps in (`LivingEntity.getSleepingPos`),
    /// `Player.sleepCounter`, and the bed last told to the player itself.
    pub bed: Option<[i32; 3]>,
    pub woke_from: Option<[i32; 3]>,
    pub sleep_timer: i32,
    pub sent_bed: Option<[i32; 3]>,
    /// Where the player respawns (a bed's spot; `ServerPlayer.respawnConfig`),
    /// None for the world spawn.
    pub spawn: Option<[f64; 3]>,
    /// A block-resolved personal respawn point: its dimension, block, yaw,
    /// pitch and whether it is forced, as the player's save keeps it
    /// (`respawn`).
    pub respawn_elsewhere: Option<(String, [i32; 3], f32, f32, bool)>,
    /// Pending ClientboundPlayerPositionPacket id; movement is ignored until
    /// the client accepts it (ServerGamePacketListenerImpl.awaitingPositionFromClient).
    pub awaiting_teleport: Option<(i32, [f64; 3])>,
    /// Through a portal, until `LanHub::land` gives the client its position:
    /// the moves it sent from the old level are ignored, as they would be
    /// while it had a teleport to accept.
    pub landing: bool,
    /// `landing` was set by a `tp` hold waiting for chunks (`Game::hold_for_chunks`), not by a portal.
    pub tp_held: bool,
    pub teleport_counter: i32,
    /// The velocity in a travel correction still awaiting confirmation.
    pub(super) pending_travel_velocity: Option<[f64; 3]>,
    /// `ServerPlayer.getCamera() != this`.
    pub watching: bool,
    /// The hub tick the pending teleport was (re)sent on (awaitingTeleportTime).
    pub teleport_tick: u64,
    /// Where the player was when this tick's packets began (firstGoodX/Y/Z),
    /// for the moved-too-quickly check.
    pub first_good: [f64; 3],
    /// Position moves received in all and by the start of this tick
    /// (receivedMovePacketCount, knownMovePacketCount).
    pub received_moves: i32,
    pub known_moves: i32,
    /// The last accepted move left the player hovering over air
    /// (`clientIsFloating`), and the ticks it has (`aboveGroundTickCount`).
    pub client_is_floating: bool,
    pub above_ground_ticks: i32,
    /// The same for the vehicle the player drives (`clientVehicleIsFloating`,
    /// `aboveGroundVehicleTickCount`), with that vehicle's gravity
    /// (`getMaximumFlyingTicks(lastVehicle)`) and whether its last accepted
    /// move came down onto something (`verticalCollisionBelow`).
    pub vehicle_floating: bool,
    pub above_ground_vehicle_ticks: i32,
    pub vehicle_gravity: f64,
    pub vehicle_rests: bool,
    /// `Entity.verticalCollisionBelow` of the player's own body: its last
    /// accepted move came down onto something.
    pub rests_on_ground: bool,
    /// `ServerGamePacketListenerImpl.lastSupportedBlocksSend`: the connection
    /// tick the blocks under the player were last force-sent.
    pub support_blocks_sent: u64,
    /// A position move arrived since the client's last ClientTickEnd
    /// (receivedPositionThisTick): a second one is a protocol error.
    pub position_this_tick: bool,
    /// It accepted a move since the last entity range checks
    /// (ServerChunkCache.move), which then check all of its pairs again.
    pub moved: bool,
    /// The players a native mod hid from it (`Hub::hidden`) that its player
    /// list no longer has.
    pub tab_hidden: crate::fast_hash::FxHashSet<u128>,
    /// The client finished loading terrain (ServerboundPlayerLoadedPacket).
    pub loaded: bool,
    /// Since it joined or respawned, the client hasn't said it loaded the level
    /// (ServerboundPlayerLoadedPacket): it may still be on its loading screen, where it doesn't
    /// move, even once the server stops waiting for it (`load_timeout`).
    pub loading: bool,
    /// Ticks left before the server stops waiting for that packet
    /// (ServerGamePacketListenerImpl.clientLoadedTimeoutTimer): 60 at join and
    /// after a respawn, 0 once it comes.
    pub load_timeout: u8,
    // Chunks.
    pub center: Option<(i32, i32)>,
    /// View distance changed since `update_view` last ran, so the next view pass
    /// must prune/requeue even when `center` stayed the same.
    pub(super) view_dirty: bool,
    pub sent_chunks: crate::fast_hash::FxHashSet<(i32, i32)>,
    /// The near chunks the mods were told this player watches
    /// (`lan::chunk_watch`); empty while no mod listens.
    pub watched: crate::fast_hash::FxHashSet<(i32, i32)>,
    pub chunk_queue: Vec<(i32, i32)>,
    /// PlayerChunkSender: batches sent but not yet acknowledged, how many
    /// may be (1 until the client's first acknowledgement, then 10), the
    /// rate the client asks for and the chunks the next batch may hold.
    pub unacknowledged: u32,
    pub max_unacknowledged: u32,
    pub chunks_per_tick: f32,
    pub batch_quota: f32,
    /// A mod's boxes of its level in which the level's blocks don't stop it (#3717): its moves
    /// are checked through them, and its body takes them at each turn. Not saved.
    pub collision_exclusions: sunrise_world::collision_exclusions::CollisionExclusions,
    /// The moving collider carrying the player, and where the last accepted report put its
    /// feet in the collider's frame (`lan/hub/carried_check.rs`). Not saved.
    pub on_collider: Option<sunrise_core::geometry::carried::Carried>,
    /// What the player's client last said about the age of its copy of the collider
    /// (`sunrise:carry_age`): the collider and the ticks. Used by the next report only.
    pub carry_hint: Option<(u32, u8)>,
    /// `ServerPlayer.requestedDebugSubscriptions`: the `debug_subscription` registry ids the
    /// client asked for, kept whoever the player is (the permission check comes when they are read).
    pub debug_subscriptions: Vec<i32>,
    /// The tick a mod last displaced the player and how many ticks in a row it has (a development
    /// build warns at 20; a release build never reads it).
    pub displaced_streak: (u64, u32),
    /// Where a rejected report puts the player back: the feet as the server last accepted
    /// them, in the collider's frame, under the collider's transform now.
    pub carry_correction: Option<[f64; 3]>,
    /// Where the accepted report of a carried player is committed: its local position under
    /// the collider's transform now, after the world's collision. Read by the same report.
    pub carry_commit: Option<[f64; 3]>,
    /// The reports rejected while a collider carried the player. They never count toward the
    /// flying kick.
    pub carry_rejects: u32,
    // Entities.
    pub tracker: Tracker,
    // Player state the gameplay code reads and writes.
    pub motion: Option<Box<PlayerMotion>>,
    /// What the player's file says of the body (`Entity.load`: the fire
    /// and air ticks and the fall distance), for the first body it gets.
    pub loaded_body: Option<SavedBody>,
    /// The attribute list the player's last body hands to its next one
    /// (`ServerPlayer.restoreFrom`, `PlayerMotion::carried_attributes`),
    /// until that body exists: the scale a player has set stays through a
    /// respawn and a change of level (#3684).
    pub carried_attributes: Option<sunrise_core::nbt::Tag>,
    /// `Entity.tickCount` carried while the same player is between bodies.
    /// A respawn clears it; a teleport or level change restores it to the new body.
    pub carried_tick_count: Option<i32>,
    /// The same ServerPlayer keeps its body yaw across level travel. A
    /// replacement body, including one after death, starts at zero.
    pub carried_body_yaw: Option<f32>,
    /// The accepted hit follows the same living body through a full teleport restore.
    pub carried_hit: Option<std::sync::Arc<crate::health::AcceptedHit>>,
    pub carried_last_hurt_mob: Option<(u32, i32)>,
    /// The vehicle the player's file names (`ServerPlayer.saveParentVehicle`'s
    /// "RootVehicle"), for the ride to come back on the next login.
    pub saved_vehicle: Option<[i32; 4]>,
    /// The hurt and attack logs of the body it had before it respawned,
    /// until the new body takes them over (the control socket's `hurtlog`
    /// reads across a death).
    pub logs_kept: Option<(
        Vec<crate::health::HurtRecord>,
        Vec<crate::health::AttackRecord>,
    )>,
    /// The 36 inventory slots (hotbar first), armor (head..feet) and off hand.
    pub inventory: Vec<Option<ItemStack>>,
    pub armor: [Option<ItemStack>; 4],
    /// Armor as of this player's last `collectEquipmentChanges` look; swapped
    /// with `armor` so each player compares against their own snapshot.
    pub location_armor: [Option<ItemStack>; 4],
    pub offhand: Option<ItemStack>,
    /// `EquipmentSlot.BODY` and `SADDLE`, kept and saved like `Game::equipment`.
    pub equipment: sunrise_items::equipment::Equipment,
    /// Converted tracking stacks, invalidated when their full source stack changes.
    pub equipment_cache: super::equipment_cache::EquipmentCache,
    pub selected: usize,
    /// The active menu's cursor. The inventory menu keeps its own while another menu is open.
    pub carried: Option<ItemStack>,
    pub(super) inactive_inventory_carried: Option<ItemStack>,
    /// The inventory menu's crafting grid (2x2) and its result
    /// (`InventoryMenu.craftSlots`, `resultSlots`), which the game computes.
    pub craft_grid: [Option<ItemStack>; 4],
    pub craft_result: Option<ItemStack>,
    /// The ender chest inventory (`PlayerEnderChestContainer`, 27 slots).
    pub ender_items: Vec<Option<ItemStack>>,
    /// What a native mod's menu shows (`Slots::Native`), while one is open.
    pub native_menu: Vec<Option<ItemStack>>,
    pub health: f32,
    /// The saved `AbsorptionAmount`, until the player's first body on the
    /// server takes it (the body holds the live amount).
    pub saved_absorption: Option<f32>,
    pub food: i32,
    pub saturation: f32,
    pub xp_level: i32,
    pub xp_progress: f32,
    pub xp_total: i32,
    /// `Player.DATA_SCORE_ID`, independent of total experience.
    pub score: i32,
    /// `Player.enchantmentSeed` (the enchanting table's offers).
    pub xp_seed: i32,
    /// Active status effects (`LivingEntity.activeEffects`).
    pub effects: crate::effects::PlayerEffects,
    /// The swirls of `effects` as the native mods left them
    /// (`effect_particle_modification`), while one listens.
    pub modified_effect_particles: Option<crate::effects::ModifiedEffectParticles>,
    /// What the client was told of each effect: id, amplifier, flags, and
    /// the duration and `effects.tick_count` at the time, so a duration that
    /// only counted down is not sent again (`ServerPlayer.onEffectUpdated`).
    pub sent_effects: Vec<SentEffect>,
    /// What the client believes the inventory menu holds (for change detection).
    pub menu_sent: Vec<Option<ItemStack>>,
    /// `AbstractContainerMenu.remoteCarried` for the always-present inventory menu.
    pub(super) inventory_carried_sent: Option<ItemStack>,
    /// `remoteCarried` for the open window, including menus run by Game rather than Hub.
    pub(super) open_carried_sent: Option<(i32, Option<ItemStack>)>,
    pub menu_state: i32,
    /// `lastSentHealth`, `lastSentFood` and `lastFoodSaturationZero`.
    pub sent_health: Option<(f32, i32, bool)>,
    /// Whether the entity tracker has sent the initial health metadata to this player.
    pub(super) sent_health_data: bool,
    pub sent_xp: Option<(f32, i32, i32)>,
    pub(super) sent_score: i32,
    /// The air supply and absorption the player's client was last sent
    /// (Entity.DATA_AIR_SUPPLY_ID, Player.DATA_PLAYER_ABSORPTION_ID); their
    /// defaults until either changes.
    pub sent_air: (i32, f32),
    pub sent_frozen: i32,
    /// The shared flags (Entity.DATA_SHARED_FLAGS_ID) the player's client was
    /// last sent about itself.
    pub sent_shared: i8,
    /// Pose last delivered to this player's own entity data, initially Standing.
    pub sent_pose: Pose,
    /// The shoulder parrots (Player.DATA_SHOULDER_PARROT_LEFT and _RIGHT) the
    /// player's client was last sent about itself.
    pub sent_shoulders: [Option<i32>; 2],
    /// The arrows and stingers (`DATA_ARROW_COUNT_ID`, `DATA_STINGER_COUNT_ID`)
    /// the player's client was last sent about itself.
    pub sent_stuck: (i32, i32),
    /// Where the player last died (Player.lastDeathLocation), sent with the respawn.
    pub last_death: Option<(String, [i32; 3])>,
    /// The last action sequence to acknowledge at the next tick (-1: none).
    pub ack_up_to: std::cell::Cell<i32>,
    /// Packets waiting for the player's next turn, and their bytes on the wire.
    pub inbox: VecDeque<InboxItem>,
    pub joined_tick: u64,
    pub ticks: u64,
    /// The moves accepted since the game last looked, for ServerPlayer.doCheckFallDamage.
    pub moves: Vec<GuestMove>,
    /// Whether the guest was on the ground before the first of `moves`.
    pub moves_from_ground: bool,
    /// `ServerPlayer.lastKnownClientMovement`: the last move accepted from
    /// their client, zero after a client tick without one.
    pub known_movement: [f64; 3],
    pub dead: bool,
    /// A cancelled die at non-positive health belongs to this body, across full restores.
    pub death_canceled: bool,
    /// `LivingEntity.deathTime`: ticks since the guest died.
    pub death_time: u32,
    /// The player record is materialized for gameplay and handle lookup.
    /// Network guests reach this state after placeNewPlayer; fake actors do not
    /// log in. Online iterators must also check `participates_online`.
    pub placed: bool,
    /// Removed from its level for `showEndCredits` or `PlayerList.respawn`, but still joined.
    pub respawning: bool,
    /// The hand-over a proxy started by exporting it with `freeze`
    /// (`arrivals::Handover`): while it lasts, what it sends other than its
    /// acknowledgements is not acted on, and every export answers with the
    /// same record. It ends after `[transfer]`'s `max_freeze_seconds`.
    pub handover: Option<super::arrivals::Handover>,
    /// Its login started while a proxy said it would arrive from another
    /// server, and used that expectation up (`player-expect`).
    pub expected: bool,
    /// A player arriving from another server (`arrivals`): connected and
    /// receiving the world, but not placed, so no one else sees, hears or
    /// hurts it until the hand-over finishes.
    pub held: bool,
    /// Its areas in any level and the other levels its client views (#3716): apart from what
    /// makes it one of a level's players (`placed_in`, `watches_level`).
    pub viewed: super::viewed_levels::Viewed,
    /// The hub tick at which it was held, to drop it if its hand-over never comes.
    pub held_since: u64,
    /// A held player whose record has been applied but that is not yet let into the world
    /// (`player-import` with `"place": false`): `player-confirm` places it, `player-drop` removes it.
    pub imported: bool,
    /// Whether the others are told when it is placed, as the import asked.
    pub arrival_announce: bool,
    /// The hand-over its record came with (`player-import`): `player-confirm` and
    /// `player-drop` name it, and another hand-over's record is not taken.
    pub arrival_handover: Option<u128>,
    /// The import of that hand-over moved the view to another chunk (`recentered` in its reply),
    /// kept so a retried import or a confirm after it answers the same.
    pub arrival_recentered: bool,
    /// Leaving for another server (`player-release moved`) without the "left the
    /// game" line.
    pub quiet_leave: bool,
    /// Taken out of the world because a proxy moved it to another server: the game
    /// saves neither its player file nor its records, since the other server has the
    /// player now.
    pub released: bool,
    /// `Player.getAbilities`: set from the game mode when it changes, and
    /// written by server mods. Saved with the player.
    pub abilities: super::abilities::PlayerAbilities,
    /// The level the player is in (`ServerPlayer.level().dimension()`): the
    /// hub's level passes only reach the players in theirs.
    pub dimension: String,
    /// The open block container, if any.
    pub menu: Option<super::menu::Menu>,
    pub next_window: i32,
    /// Identity of the current menu, distinct even when a window id is reused.
    pub menu_generation: u64,
    /// An unfinished drag in the inventory menu.
    pub drag: Option<(u8, Vec<usize>)>,
    /// The compound this player was loaded from, for fields Sunrise does not model.
    pub saved: Option<sunrise_core::nbt::Tag>,
    /// Server-owned attachment data for this player.
    pub attachments: sunrise_mod_runtime::attachments::ResidentAttachments,
    /// `ServerPlayer.seenCredits`: the end poem has been shown.
    pub seen_credits: bool,
    /// `ServerPlayer.wonGame`: the End poem is showing until PERFORM_RESPAWN.
    pub won_game: bool,
    /// `ServerPlayer.postEffects` and `postEffectsDirty`.
    pub post_effects: Vec<String>,
    pub post_effects_dirty: bool,
    /// `ServerPlayer.recipeBook`.
    pub recipe_book: crate::recipe_book::Settings,
    /// `PlayerAdvancements.lastSelectedTab`: the advancement tab the player
    /// last opened, as its id, when the server kept it (a root with a display).
    pub selected_tab: Option<String>,
    /// `PlayerAdvancements`: the criteria obtained, whether they came from
    /// the player's file, the advancements the client was last told it may
    /// see (empty before the first packet), and those whose progress
    /// changed since (`progressChanged`).
    pub advancements: crate::advancements::Progress,
    pub advancements_loaded: bool,
    /// Whether the first advancement snapshot reached this connection.
    pub advancements_sent: bool,
    pub advancements_visible: Vec<bool>,
    pub advancements_changed: std::collections::BTreeSet<usize>,
    /// The world board last published to this connection, excluding its private overlay.
    pub scoreboard: super::scoreboard::View,
    /// Whether the joining sync (`PlayerList.updateEntireScoreboard`) went out.
    pub(super) scoreboard_synced: bool,
    pub(super) personal_sidebar: Option<Box<super::scoreboard::PersonalSidebar>>,
    /// Recipes unlocked since the client was last told
    /// (`ServerRecipeBook.addRecipes` sends them).
    pub recipes_unlocked: Vec<String>,
}

impl Guest {
    /// The client's socket address, including the port.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The client's IP address as text, without the port.
    pub fn ip_address(&self) -> &str {
        &self.ip_address
    }

    /// Replaces the address and keeps its cached IP text in sync.
    pub fn set_address(&mut self, address: SocketAddr) {
        self.address = address;
        self.ip_address = address.ip().to_string();
    }

    /// `ServerPlayer.nextContainerCounter`: advance the window counter to `counter % 100 + 1`
    /// and return it. Every open takes its window id from here.
    pub fn next_container_counter(&mut self) -> i32 {
        self.next_window = self.next_window % 100 + 1;
        self.next_window
    }

    /// Allocate the next protocol window and a non-reusable menu identity.
    pub fn next_menu_window(&mut self) -> i32 {
        self.next_container_counter();
        self.menu_generation = menu_generation();
        self.inactive_inventory_carried = self.carried.take();
        self.open_carried_sent = Some((self.next_window, None));
        self.next_window
    }

    /// The wire window and its parked cursor, before a transactional replacement.
    /// The guest's active cursor is saved with their inventory through its loan.
    pub fn menu_before_replacement(&self) -> impl FnOnce(&mut Self) + use<> {
        let menu = self.menu.clone();
        let native = self.native_menu.clone();
        let window = self.next_window;
        let generation = self.menu_generation;
        let inactive = self.inactive_inventory_carried.clone();
        let sent = self.open_carried_sent.clone();
        move |guest| {
            guest.menu = menu;
            guest.native_menu = native;
            guest.next_window = window;
            guest.menu_generation = generation;
            guest.inactive_inventory_carried = inactive;
            guest.open_carried_sent = sent;
        }
    }

    /// Retire an open menu, returning its carried stack for `AbstractContainerMenu.removed`.
    pub fn close_menu_cursor(&mut self, window: i32) -> Option<ItemStack> {
        self.open_carried_sent.take_if(|(id, _)| *id == window)?;
        std::mem::replace(&mut self.carried, self.inactive_inventory_carried.take())
    }

    pub fn take_inventory_carried(&mut self) -> Option<ItemStack> {
        if self.open_carried_sent.is_some() {
            self.inactive_inventory_carried.take()
        } else {
            self.carried.take()
        }
    }

    /// `Player.remove` removes both menus, dropping both cursors.
    pub fn take_menu_cursors(&mut self) -> impl Iterator<Item = ItemStack> {
        [self.inactive_inventory_carried.take(), self.carried.take()]
            .into_iter()
            .flatten()
    }

    pub(super) fn acknowledge_menu_cursor(&mut self, window: i32, carried: Option<ItemStack>) {
        if window == 0 {
            self.inventory_carried_sent = carried;
        } else if let Some((id, sent)) = &mut self.open_carried_sent
            && *id == window
        {
            *sent = carried;
        }
    }

    pub fn new(j: Joined, entity_id: i32, position: [f64; 3], view_distance: i32) -> Self {
        let Joined {
            conn,
            uuid,
            name,
            properties,
            info,
            brand,
            sender,
            latency,
            address,
            expected,
        } = j;
        Self::with_defaults(
            GuestKind::Network,
            GuestInit {
                conn,
                uuid,
                name,
                properties,
                info,
                brand,
                sender,
                latency,
                address,
                expected,
            },
            entity_id,
            position,
            view_distance,
        )
    }

    pub fn connectionless(
        conn: u32,
        uuid: u128,
        name: String,
        entity_id: i32,
        position: [f64; 3],
        motion: PlayerMotion,
    ) -> Self {
        let init = GuestInit {
            conn,
            uuid,
            name,
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: String::new(),
            sender: PlayerSender::disabled(),
            latency: Arc::default(),
            address: ([0, 0, 0, 0], 0).into(),
            expected: false,
        };
        let mut guest = Self::with_defaults(GuestKind::Fake, init, entity_id, position, 2);
        guest.placed = true;
        // No connection ever reports the world loaded: the record is
        // material, and the body's damage policy treats it as loaded from
        // the start instead of invulnerable forever.
        guest.loaded = true;
        guest.loading = false;
        guest.load_timeout = 0;
        guest.motion = Some(Box::new(motion));
        guest
    }

    /// Sends the play disconnect with `reason` and keeps `text` (the reason as the log's
    /// "lost connection" line gives it, as vanilla's), once: while the disconnect is pending a
    /// second one is refused and the first reason stands. Whether this call started it.
    pub fn begin_disconnect(&mut self, text: String, reason: sunrise_core::nbt::Tag) -> bool {
        if self.disconnect_pending {
            return false;
        }
        self.disconnect_pending = true;
        self.left_reason = text;
        self.sender.disconnect(reason);
        true
    }

    pub fn participates_online(&self) -> bool {
        self.kind == GuestKind::Network
    }
    pub fn participates_in_player_simulation(&self) -> bool {
        self.kind == GuestKind::Network
    }
    /// A connectionless player a mod made (`Host::fake_player`): what `PlayerRef::is_fake`
    /// answers, and what may `perform_action`.
    pub fn is_fake(&self) -> bool {
        self.kind == GuestKind::Fake
    }
    /// The connection has joined; unloading a body for respawn does not log it out.
    pub fn joined(&self) -> bool {
        self.placed || self.respawning
    }
    /// Whether this guest's sender still accepts play packets. Does not require a placed body.
    pub fn play_connection_open(&self) -> bool {
        !self.disconnect_pending
            && self.left_reason.is_empty()
            && !self.sender.payload_transport().closed()
            && self.sender.is_live()
    }
    /// Whether the guest is online and placed in level `dimension`: one of the
    /// level's players (`ServerLevel.players`).
    pub fn placed_in(&self, dimension: &str) -> bool {
        self.participates_online() && self.placed && self.dimension == dimension
    }
    /// Whether the guest is sent level `dimension`: placed in it, or held, which
    /// looks at it without being part of it.
    pub fn watches_level(&self, dimension: &str) -> bool {
        self.participates_online() && (self.placed || self.held) && self.dimension == dimension
    }
    /// Whether the guest's client views level `dimension` besides its own (#3716): its areas
    /// name it, or its client holds chunks of it still. It is not one of that level's players.
    pub fn views_level(&self, dimension: &str) -> bool {
        self.participates_online()
            && self.placed
            && self.dimension != dimension
            && self.viewed.in_level(dimension)
    }
    /// `ChunkMap.isChunkTracked`: whether the guest watches chunk `(x, z)` of
    /// level `dimension`, inside its view (`ChunkTrackingView.contains`) or kept
    /// past it for an area (`viewed_levels::own`), and sent rather than pending.
    pub fn tracks_chunk(&self, dimension: &str, (x, z): (i32, i32)) -> bool {
        self.watches_level(dimension)
            && self.sent_chunks.contains(&(x, z))
            && (self
                .center
                .is_some_and(|(cx, cz)| super::hub::tracked(cx, cz, self.view_distance, x, z))
                || self.viewed.own.kept.contains(&(x, z)))
    }

    fn with_defaults(
        kind: GuestKind,
        j: GuestInit,
        entity_id: i32,
        position: [f64; 3],
        view_distance: i32,
    ) -> Self {
        Self {
            residency: Default::default(),
            kind,
            conn: j.conn,
            address: j.address,
            ip_address: j.address.ip().to_string(),
            last_action_tick: 0,
            chat_index: 0,
            chain: Default::default(),
            last_seen: Default::default(),
            signatures: Default::default(),
            placed_at: 0,
            level_entered: 0,
            last_reported: (position, [0.0, 0.0]),
            center_sent: None,
            left_reason: String::new(),
            listed_behind: Vec::new(),
            pending_move: None,
            disconnect_pending: false,
            uuid: j.uuid,
            name: j.name,
            properties: j.properties,
            entity_id,
            sender: j.sender,
            latency: j.latency,
            view_distance: i32::from(j.info.view_distance).max(2).min(view_distance),
            info: j.info,
            brand: j.brand,
            recipe_sync: Default::default(),
            position,
            held_ticket_chunk: None,
            previous_position: position,
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            on_ground: false,
            game_mode: SURVIVAL,
            previous_game_mode: None,
            permission: 0,
            sneaking: false,
            input: 0,
            vehicle: None,
            vehicle_order: 0,
            vehicle_good: None,
            sprinting: false,
            fall_flying: false,
            sleeping: false,
            living_flags: 0,
            stuck: (0, 0),
            shoulders: [None; 2],
            bed: None,
            woke_from: None,
            sleep_timer: 0,
            sent_bed: None,
            spawn: None,
            respawn_elsewhere: None,
            awaiting_teleport: None,
            landing: false,
            tp_held: false,
            teleport_counter: 0,
            pending_travel_velocity: None,
            watching: false,
            teleport_tick: 0,
            first_good: position,
            received_moves: 0,
            known_moves: 0,
            client_is_floating: false,
            above_ground_ticks: 0,
            vehicle_floating: false,
            above_ground_vehicle_ticks: 0,
            vehicle_gravity: 0.0,
            vehicle_rests: false,
            rests_on_ground: false,
            support_blocks_sent: 0,
            position_this_tick: false,
            moved: false,
            tab_hidden: Default::default(),
            loaded: false,
            loading: true,
            load_timeout: CLIENT_LOADED_TIMEOUT,
            center: None,
            view_dirty: false,
            sent_chunks: Default::default(),
            watched: Default::default(),
            chunk_queue: Vec::new(),
            unacknowledged: 0,
            max_unacknowledged: 1,
            chunks_per_tick: 9.0,
            batch_quota: 0.0,
            collision_exclusions: Default::default(),
            on_collider: None,
            carry_hint: None,
            debug_subscriptions: Vec::new(),
            displaced_streak: (0, 0),
            carry_correction: None,
            carry_commit: None,
            carry_rejects: 0,
            tracker: Tracker::default(),
            motion: None,
            loaded_body: None,
            carried_attributes: None,
            carried_tick_count: None,
            carried_body_yaw: None,
            carried_last_hurt_mob: None,
            carried_hit: None,
            saved_vehicle: None,
            logs_kept: None,
            inventory: vec![None; 36],
            armor: [None, None, None, None],
            location_armor: [None, None, None, None],
            offhand: None,
            equipment: Default::default(),
            equipment_cache: Default::default(),
            selected: 0,
            carried: None,
            inactive_inventory_carried: None,
            craft_grid: Default::default(),
            craft_result: None,
            ender_items: vec![None; 27],
            native_menu: Vec::new(),
            health: 20.0,
            saved_absorption: None,
            food: 20,
            saturation: 5.0,
            xp_level: 0,
            xp_progress: 0.0,
            xp_total: 0,
            score: 0,
            xp_seed: 0,
            effects: Default::default(),
            modified_effect_particles: None,
            sent_effects: Vec::new(),
            menu_sent: vec![None; 46],
            inventory_carried_sent: None,
            open_carried_sent: None,
            menu_state: 0,
            sent_health: None,
            sent_health_data: false,
            sent_xp: None,
            sent_score: 0,
            sent_air: (300, 0.0),
            sent_frozen: 0,
            sent_shared: 0,
            sent_pose: Pose::Standing,
            sent_shoulders: [None; 2],
            sent_stuck: (0, 0),
            last_death: None,
            ack_up_to: std::cell::Cell::new(-1),
            inbox: VecDeque::new(),
            joined_tick: 0,
            ticks: 0,
            moves: Vec::new(),
            moves_from_ground: false,
            known_movement: [0.0; 3],
            dead: false,
            death_canceled: false,
            death_time: 0,
            placed: false,
            respawning: false,
            handover: None,
            expected: j.expected,
            held: false,
            viewed: Default::default(),
            held_since: 0,
            imported: false,
            arrival_announce: true,
            arrival_handover: None,
            arrival_recentered: false,
            quiet_leave: false,
            released: false,
            abilities: Default::default(),
            dimension: "minecraft:overworld".into(),
            menu: None,
            next_window: 0,
            menu_generation: menu_generation(),
            drag: None,
            saved: None,
            attachments: Default::default(),
            seen_credits: false,
            won_game: false,
            post_effects: Vec::new(),
            post_effects_dirty: true,
            recipe_book: Default::default(),
            selected_tab: None,
            advancements: Default::default(),
            advancements_loaded: false,
            advancements_sent: false,
            advancements_visible: Vec::new(),
            advancements_changed: Default::default(),
            scoreboard: Default::default(),
            scoreboard_synced: false,
            personal_sidebar: None,
            recipes_unlocked: Vec::new(),
        }
    }
    /// `ServerPlayer.onUpdateAbilities`: the guest's abilities, speeds included.
    pub fn abilities_packet(&self) -> play::Clientbound {
        play::Clientbound::PlayerAbilities(self.abilities.packet())
    }
    pub fn send(&self, packet: &play::Clientbound) {
        if !self.participates_online() {
            return;
        }
        match packet.try_encode() {
            Ok(body) => self.sender.send(body),
            Err(error) => eprintln!("[lan] send refused: {error}"),
        }
    }
    /// Send `Player.DATA_SCORE_ID` when its synced value changes, including before a death screen.
    pub fn sync_score(&mut self) {
        if self.score != self.sent_score {
            self.sent_score = self.score;
            self.send(&play::Clientbound::SetEntityData(play::SetEntityData {
                id: self.entity_id,
                values: vec![(18, serializer::INT, Value::Int(self.score))],
            }));
        }
    }
    /// Send the committed data that fits the negotiated blockless packet's index.
    pub fn send_menu_data_snapshot(&self) {
        let Some(menu) = &self.menu else { return };
        for (index, &value) in (0..=u16::MAX).zip(&menu.blockless_data) {
            self.send_menu_data(menu.window, index, value);
        }
        let wire_len = usize::from(u16::MAX) + 1;
        if menu.blockless_data.len() > wire_len {
            log::warn!(
                "Sunrise: blockless menu window {} cannot sync data slots at indices {wire_len}..{} (DataSlot index max is {})",
                menu.window,
                menu.blockless_data.len(),
                u16::MAX,
            );
        }
    }

    /// Commit one blockless-menu data slot. The caller resolves the window generation
    /// and validates the index before supplying a write.
    pub fn set_menu_data(&mut self, index: usize, value: i32) {
        let Some(menu) = &mut self.menu else { return };
        let slot = &mut menu.blockless_data[index];
        if *slot != value {
            *slot = value;
            let window = menu.window;
            if let Ok(wire_index) = u16::try_from(index) {
                self.send_menu_data(window, wire_index, value);
            } else {
                log::warn!(
                    "Sunrise: blockless menu window {window} cannot sync data slot {index} (DataSlot index max is {})",
                    u16::MAX,
                );
            }
        }
    }

    fn send_menu_data(&self, window: i32, index: u16, value: i32) {
        use sunrise_protocol::menu_extra::{DATA_CHANNEL, DataSlot};
        self.send(&play::Clientbound::CustomPayload(CustomPayload {
            channel: DATA_CHANNEL.into(),
            data: DataSlot {
                window,
                index,
                value,
            }
            .encode(),
        }));
    }
    /// Whether this connection reads collider sync (`sunrise:collider_sync`).
    pub fn negotiated_collider_sync(&self) -> bool {
        crate::raw_payloads::negotiated(
            self.sender.payload_transport(),
            sunrise_protocol::collider_sync::CHANNEL,
        )
    }
    /// Whether this connection reads connection-group sync (`sunrise:connection_groups`).
    pub fn negotiated_connection_groups(&self) -> bool {
        crate::raw_payloads::negotiated(
            self.sender.payload_transport(),
            sunrise_protocol::connection_groups::CHANNEL,
        )
    }
    /// Whether this connection reads general entity spawns (`sunrise:general_entity`): only
    /// such a guest is paired with a mod's general row.
    pub fn negotiated_general_entity(&self) -> bool {
        crate::raw_payloads::negotiated(
            self.sender.payload_transport(),
            sunrise_protocol::general_entity::CHANNEL,
        )
    }
    /// Whether the guest's play epoch has attachment sync negotiated (a vanilla client never
    /// offers it).
    pub fn negotiated_attachment_sync(&self) -> bool {
        crate::raw_payloads::negotiated(self.sender.payload_transport(), attachment_sync::CHANNEL)
    }
    /// Passages to hold or forget (#3775), if the guest is on a Sunrise client, the only
    /// one that reads them, and placed or held (a hand-over's arrival is sent its chunks,
    /// so their passages too); a message that cannot be written is not sent.
    pub fn send_passage_sync(&self, message: &passage_sync::Message) {
        if !(self.placed || self.held) || self.brand != "sunrise" {
            return;
        }
        match passage_sync::encode(message) {
            Ok(data) => self.send(&play::Clientbound::CustomPayload(CustomPayload {
                channel: passage_sync::CHANNEL.into(),
                data,
            })),
            Err(e) => log::debug!("Sunrise: passage sync not sent: {e}"),
        }
    }
    /// [`Self::send`] of a `ChunksBiomes` whose body is `packet.encode()`, the same
    /// bytes for each guest with those columns.
    pub fn send_chunks_biomes_body(&self, body: Vec<u8>) {
        if !self.participates_online() {
            return;
        }
        self.sender.send(body);
    }
    /// `Entity.stopRiding`: out of the vehicle it rode.
    pub fn stop_riding(&mut self) {
        (self.vehicle, self.vehicle_good) = (None, None);
    }
    /// The body remains paired while alive, or until its twentieth death tick.
    pub fn body_present(&self) -> bool {
        self.health > 0.0 || self.death_time < 20
    }
    /// `LivingEntity.tickDeath` for a dead player: true once its body is
    /// removed, 20 ticks after it died (`remove(KILLED)`, whose
    /// `Entity.setRemoved` stops its riding). The body sits in its vehicle
    /// until then.
    pub fn tick_death(&mut self) -> bool {
        self.death_time += 1;
        let removed = self.death_time >= 20;
        if removed {
            self.stop_riding();
        }
        removed
    }
    pub fn send_all(&self, packets: &[play::Clientbound]) {
        for p in packets {
            self.send(p);
        }
    }
    /// A new player at `at` (`ChunkMap.addEntity`; the tickets start there).
    pub fn place(&mut self, at: [f64; 3]) {
        (
            self.position,
            self.previous_position,
            self.held_ticket_chunk,
        ) = (at, at, None);
    }
    /// `ServerChunkCache.move` after an accepted move: the tickets follow.
    pub fn accept_move(&mut self, to: [f64; 3]) {
        self.previous_position = self.position;
        (self.position, self.held_ticket_chunk) = (to, None);
    }
    /// The chunk the player's `DistanceManager` tickets are at. Teleporting
    /// does not move them: `ServerChunkCache.move` does, and its callers are an
    /// accepted move or the ack of a teleport (`handlePlayerPositionChange`),
    /// a vehicle or passenger move (`handleMoveVehicle`) and a camera
    /// (`ServerPlayer.setCamera` and its tick), whose chunk is `position`'s.
    pub fn ticket_chunk(&self) -> (i32, i32) {
        self.held_ticket_chunk
            .filter(|_| !self.watching)
            .unwrap_or_else(|| self.chunk())
    }
    /// A camera change or a passenger's move: the tickets follow `position`.
    pub fn release_tickets(&mut self) {
        self.held_ticket_chunk = None;
    }
    pub fn chunk(&self) -> (i32, i32) {
        (
            (self.position[0].floor() as i32) >> 4,
            (self.position[2].floor() as i32) >> 4,
        )
    }
    pub fn creative(&self) -> bool {
        self.game_mode == CREATIVE
    }
    /// ServerGamePacketListenerImpl.hasClientLoaded: not dead (waitingForRespawn) and
    /// loaded, or waited for long enough. Until then the player's actions are ignored.
    pub fn client_loaded(&self) -> bool {
        !self.dead && self.load_timeout == 0
    }
    /// `ServerPlayerGameMode.changeGameModeForPlayer`: the mode left becomes
    /// the previous one; the mode already in use changes nothing.
    pub fn change_game_mode(&mut self, mode: i32) {
        if mode != self.game_mode {
            self.previous_game_mode = Some(self.game_mode);
            self.game_mode = mode;
        }
    }
    pub fn spectator(&self) -> bool {
        self.game_mode == SPECTATOR
    }
    /// Its client's `minecraft:brand` is Sunrise's: it reads the protocol's
    /// extensions, such as a native mod's particle (`Particle::sunrise_only`).
    pub fn is_sunrise(&self) -> bool {
        self.brand == "sunrise"
    }
    /// The pose the server shows of this player: in bed or gliding as its
    /// client announced, before the body's own tick has taken it; else the
    /// body's own pose (`Entity.getPose`) once its server tick has run
    /// (swimming, crawling, crouching); else crouching while the shift key is
    /// held and it isn't flying.
    pub fn pose(&self) -> Pose {
        if self.sleeping {
            Pose::Sleeping
        } else if self.fall_flying {
            Pose::FallFlying
        } else if let Some(m) = &self.motion {
            m.pose
        } else if self.sneaking && !self.abilities.flying {
            Pose::Crouching
        } else {
            Pose::Standing
        }
    }
    /// `Avatar.POSES` for the guest's pose, with its scale attribute: also for
    /// a pose the client has announced (gliding) before the body's own tick
    /// has taken it.
    pub fn dimensions(&self) -> Dimensions {
        let pose = self.pose();
        match &self.motion {
            Some(m) if m.pose == pose => m.dimensions,
            Some(m) => pose.dimensions(m.scale),
            None => pose.dimensions(1.0),
        }
    }
    /// The player's body is replaced (`ServerPlayer.restoreFrom`): the next
    /// one starts from the old one's attributes, all of them when it is the
    /// same player in another level (`restore_all`), only their base values
    /// after a death. The old body, for what else it hands over.
    pub fn new_body(&mut self, restore_all: bool) -> Option<Box<PlayerMotion>> {
        if !restore_all {
            self.carried_tick_count = None;
            self.carried_body_yaw = Some(0.0);
            self.carried_hit = None;
            self.carried_last_hurt_mob = None;
            self.death_canceled = false;
            self.death_time = 0;
        }
        let mut m = self.motion.take()?;
        self.carried_attributes = Some(m.carried_attributes(restore_all));
        if restore_all {
            self.carried_tick_count = Some(m.health.tick_count);
            self.carried_body_yaw = Some(m.body_yaw);
            self.carried_hit = m.health.accepted_hit.take();
            self.carried_last_hurt_mob = m.health.last_hurt_mob.take();
        } else {
            self.carried_tick_count = None;
        }
        Some(m)
    }
    /// `ServerPlayer.restoreFrom(old, false)`'s `setHealth(getMaxHealth())`:
    /// the health a player comes back from a death with, the max health of
    /// the base values their next body takes over, before its equipment
    /// modifies it (40 after `/attribute @s minecraft:max_health base set
    /// 40`, not 20).
    pub fn respawn_health(&self, resources: &crate::motion::MotionResources) -> f32 {
        let Ok(mut body) = PlayerMotion::new(resources, Vec3::ZERO) else {
            return 20.0;
        };
        let _ = body.apply_saved_attributes(self.carried_attributes.as_ref());
        body.attribute("minecraft:max_health")
            .map_or(20.0, |max| max as f32)
    }
    /// `Entity.getBoundingBox`.
    pub fn bounds(&self) -> Aabb {
        let [x, y, z] = self.position;
        let d = self.dimensions();
        let half = f64::from(d.width) / 2.0;
        Aabb::new(
            Vec3::new(x - half, y, z - half),
            Vec3::new(x + half, y + f64::from(d.height), z + half),
        )
    }
    pub fn eye_height(&self) -> f64 {
        f64::from(self.dimensions().eye_height)
    }
    /// `Player.isWithinBlockInteractionRange(pos, buffer)` for the guest:
    /// its block interaction range is 4.5, 5 in creative.
    pub fn within_block_range(&self, pos: [i32; 3], buffer: f64) -> bool {
        let range = if self.creative() { 5.0 } else { 4.5 };
        within_block_range(self.eye(), range + buffer, pos)
    }
    /// [`Self::within_block_range`], then a mod's `block_interaction_range`: what a menu's
    /// `stillValid` asks (`buffer` 4).
    pub fn within_interaction_range(
        &self,
        context: &LoopContext,
        pos: [i32; 3],
        buffer: f64,
    ) -> bool {
        let mut within = self.within_block_range(pos, buffer);
        native_block_interaction_range(context, self.uuid, block_pos(pos), buffer, &mut within);
        within
    }
    /// [`Self::may_reach_block`], then a mod's `block_interaction_range`: what a use or a
    /// break request asks (`buffer` 1).
    pub fn reaches_block(&mut self, context: &LoopContext, pos: [i32; 3]) -> bool {
        let mut within = self.may_reach_block(pos);
        native_block_interaction_range(context, self.uuid, block_pos(pos), 1.0, &mut within);
        within
    }
    /// Where the guest's eyes are: its feet raised by its pose's eye height.
    pub fn eye(&self) -> [f64; 3] {
        [
            self.position[0],
            self.position[1] + self.eye_height(),
            self.position[2],
        ]
    }
    /// `Player.isWithinBlockInteractionRange(pos, 1.0)`, which the server
    /// checks a block action against (ServerPlayerGameMode.handleBlockBreakAction,
    /// ServerGamePacketListenerImpl.handleUseItemOn): from the eyes of the
    /// guest's body in its pose (crouching, swimming, gliding), with its
    /// `block_interaction_range` (4.5, 5 in creative, more with `/attribute`).
    pub fn may_reach_block(&mut self, pos: [i32; 3]) -> bool {
        let eye = self
            .motion
            .as_ref()
            .map_or_else(|| self.eye_height(), |m| f64::from(m.dimensions.eye_height));
        let otherwise = if self.creative() { 5.0 } else { 4.5 };
        let range = self
            .motion
            .as_mut()
            .and_then(|m| m.attribute("minecraft:block_interaction_range").ok())
            .unwrap_or(otherwise);
        within_block_range(
            [self.position[0], self.position[1] + eye, self.position[2]],
            range + 1.0,
            pos,
        )
    }
    /// The inventory menu's 46 slots (ServerPlayer.inventoryMenu): 0 crafting
    /// result, 1..=4 crafting grid, 5..=8 armor, 9..=35 main, 36..=44 hotbar,
    /// 45 off hand.
    pub fn menu_slot(&self, slot: usize) -> Option<ItemStack> {
        match slot {
            0 => self.craft_result.clone(),
            1..=4 => self.craft_grid[slot - 1].clone(),
            5..=8 => self.armor[slot - 5].clone(),
            9..=35 => self.inventory.get(slot).cloned().flatten(),
            36..=44 => self.inventory.get(slot - 36).cloned().flatten(),
            45 => self.offhand.clone(),
            _ => None,
        }
    }
    /// The open menu's packets told the client every inventory slot: the
    /// inventory menu's own sync has nothing to add. Sent again, it could land
    /// on clicks the client has predicted since.
    pub fn inventory_told(&mut self) {
        for slot in 9..45 {
            self.menu_sent[slot] = self.menu_slot(slot);
        }
    }
    /// `ServerPlayer.drop(boolean)`: `setRemoteSlot` on the selected hotbar
    /// slot of the open menu (the inventory menu without one) records what the
    /// dropping client already predicted, so the next broadcast sends the slot
    /// again once it changes back (a pickup refilling the stack).
    pub fn told_selected(&mut self) {
        let slot = 36 + self.selected;
        self.menu_sent[slot] = self.menu_slot(slot);
        let told = self.inventory.get(self.selected).cloned().flatten();
        if let Some(m) = self.menu.as_mut()
            && m.player_slots() != 0
        {
            let at = m.size + 27 + self.selected;
            if m.sent.len() <= at {
                m.sent.resize(at + 1, None);
            }
            m.sent[at] = told;
        }
    }
    pub fn set_menu_slot(&mut self, slot: usize, item: Option<ItemStack>) {
        match slot {
            0 => self.craft_result = item,
            1..=4 => self.craft_grid[slot - 1] = item,
            5..=8 => self.armor[slot - 5] = item,
            9..=35 => self.inventory[slot] = item,
            36..=44 => self.inventory[slot - 36] = item,
            45 => self.offhand = item,
            _ => {}
        }
    }
}

/// Encode a live stack, dropping component patch rows already in its prototype.
pub(super) fn resident_stack(
    source: &ItemStack,
    item_id: &dyn Fn(&str) -> i32,
    sync: &super::data::SyncData,
    items: &crate::items::Items,
    projected: &dyn Fn(&Admitted),
) -> Stack {
    let mut wire = network_stack_projecting(
        item_id,
        &|registry, name| sync.id(registry, name),
        source,
        projected,
    );
    canonicalize_stack_patch(&mut wire, source, items, &sync.builtin);
    wire
}

/// Convert a borrowed item template with the same component codecs as inventory slots.
pub(super) fn network_stack(
    item_id: &dyn Fn(&str) -> i32,
    synced_id: &dyn Fn(&str, &str) -> Option<i32>,
    s: &ItemStack,
) -> Stack {
    network_stack_projecting(item_id, synced_id, s, &|_| ())
}

fn block_pos([x, y, z]: [i32; 3]) -> sunrise_core::position::BlockPos {
    sunrise_core::position::BlockPos::new(x, y, z)
}

/// A mod's answer to `Player.isWithinBlockInteractionRange(pos, buffer)` for the player
/// `player`: `within` arrives with the game's own (`block_interaction_range`).
#[inline(always)]
pub fn native_block_interaction_range(
    context: &LoopContext,
    player: u128,
    pos: sunrise_core::position::BlockPos,
    buffer: f64,
    within: &mut bool,
) {
    crate::native_client_hooks::block_interaction_range(context, player, pos, buffer, within);
}

/// `Player.isWithinBlockInteractionRange`: `eye` closer than `range` to the
/// box of the block at `pos` (`AABB.distanceToSqr`).
pub fn within_block_range(eye: [f64; 3], range: f64, pos: [i32; 3]) -> bool {
    (0..3)
        .map(|k| {
            let lo = f64::from(pos[k]);
            (lo - eye[k]).max(eye[k] - lo - 1.0).max(0.0).powi(2)
        })
        .sum::<f64>()
        < range * range
}

impl crate::menu::MenuGuest for Guest {
    fn uuid(&self) -> u128 {
        self.uuid
    }
    fn native_menu(&self) -> &[Option<ItemStack>] {
        &self.native_menu
    }
    fn inventory(&self) -> &[Option<ItemStack>] {
        &self.inventory
    }
    fn armor(&self) -> &[Option<ItemStack>; 4] {
        &self.armor
    }
    fn offhand(&self) -> &Option<ItemStack> {
        &self.offhand
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::nbt::Tag;
    use sunrise_core::vanilla_bytes::Writer;
    use sunrise_protocol::buf::{Codec, Reader};
    use sunrise_protocol::item::{Component, component};

    fn guest_at(position: [f64; 3]) -> Guest {
        let (tx, _rx) = std::sync::mpsc::channel();
        Guest::new(
            Joined {
                conn: 0,
                uuid: 0,
                name: "guest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(std::sync::atomic::AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            1,
            position,
            10,
        )
    }

    #[test]
    fn a_player_handle_keeps_its_identity_when_respawn_resets_the_body() {
        use mod_api::{__private::game::player_ref, EntityRef, WorldEpoch};
        let mut guest = guest_at([0.5, 64.0, 0.5]);
        guest.death_time = 20;
        guest.death_canceled = true;
        let epoch = WorldEpoch::new(7);
        let before = player_ref(EntityRef::new(epoch, guest.entity_id as u32), guest.uuid);

        // This is the respawn reset, with no motion/resources needed for identity.
        let _ = guest.new_body(false);
        let after = player_ref(EntityRef::new(epoch, guest.entity_id as u32), guest.uuid);
        assert_eq!(before, after);
        assert_eq!(guest.death_time, 0);
        assert!(!guest.death_canceled);
    }

    #[test]
    fn the_client_has_loaded_once_it_says_so_or_after_60_ticks_and_not_while_dead() {
        let mut g = guest_at([0.0; 3]);
        assert!(!g.client_loaded());
        g.load_timeout -= 59;
        assert!(!g.client_loaded());
        g.load_timeout -= 1;
        assert!(g.client_loaded());
        g.dead = true;
        assert!(!g.client_loaded());
    }

    #[test]
    fn a_dead_player_leaves_its_vehicle_when_its_body_is_removed() {
        let mut g = guest_at([0.5, -120.0, 0.5]);
        g.vehicle = Some(7);
        g.vehicle_good = Some([0.5, -120.0, 0.5]);
        g.dead = true;
        // LivingEntity.tickDeath: the body sits in the boat for 19 ticks...
        for tick in 1..20 {
            assert!(!g.tick_death(), "tick {tick}");
            assert_eq!(g.vehicle, Some(7), "tick {tick}");
        }
        // ...then remove(KILLED)'s setRemoved stops its riding, so the
        // respawned player is not put back into the boat.
        assert!(g.tick_death());
        assert_eq!((g.vehicle, g.vehicle_good), (None, None));
    }

    #[test]
    fn a_block_action_must_be_within_reach() {
        let mut g = guest_at([0.5, 64.0, 0.5]);
        // Survival (4.5 + 1 from the eyes): five blocks off horizontally,
        // not one further.
        assert!(g.may_reach_block([5, 64, 0]));
        assert!(!g.may_reach_block([6, 64, 0]));
        // Creative's 5.0 reaches one block further.
        g.game_mode = CREATIVE;
        assert!(g.may_reach_block([6, 64, 0]));
        assert!(!g.may_reach_block([7, 64, 0]));
        // With no body yet the sneak flag gives the eye height: a block at
        // the edge above is gone once crouched.
        g.game_mode = SURVIVAL;
        assert!(g.may_reach_block([0, 71, 0]));
        g.sneaking = true;
        assert!(!g.may_reach_block([0, 71, 0]));
    }

    /// A shulker box's `minecraft:container` crosses to a guest, so its
    /// tooltip can list the items it holds (#2979).
    #[test]
    fn shulker_contents_survive_inventory_sync() {
        let name = Tag::compound([("text", Tag::String("Gold bit".into()))]);
        let mut inner = ItemStack::new("minecraft:diamond", 2);
        inner.data_mut().custom_name = Some(name);
        let mut boxed = ItemStack::new("minecraft:shulker_box", 1);
        boxed.data_mut().container = vec![(0, inner)];
        let id = |name: &str| if name.ends_with(":diamond") { 22 } else { 7 };

        let sent = stack_to_network(id, |_, _| None, &Some(boxed));
        let mut writer = Writer::new();
        sunrise_protocol::item::Slot::write(&sent, &mut writer);
        let mut reader = Reader::new(&writer.data);
        let received = sunrise_protocol::item::Slot::read(&mut reader)
            .unwrap()
            .unwrap();
        assert_eq!(reader.remaining(), 0);

        let Some(Component::Raw(bytes)) = received.component(component::CONTAINER) else {
            panic!("the stack carried no container component");
        };
        // ItemContainerContents.STREAM_CODEC: the slot count, then one
        // optional template per slot.
        let mut r = Reader::new(bytes);
        assert_eq!(r.count(256).unwrap(), 1);
        assert!(r.bool().unwrap());
        let template = sunrise_protocol::item::Template::read(&mut r).unwrap();
        assert_eq!(r.remaining(), 0);
        assert_eq!((template.item, template.count), (22, 2));
        assert!(matches!(
            template.component(component::CUSTOM_NAME),
            Some(Component::Text(_))
        ));
    }

    /// The prepared game data a body needs, or `None` without it.
    fn resources() -> Option<(crate::assets::Assets, crate::motion::MotionResources)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            eprintln!("skipped: prepared resources unavailable");
            return None;
        }
        let assets = crate::assets::Assets::load(&root).ok()?;
        let resources = crate::motion::MotionResources::load(&root, &assets).ok()?;
        Some((assets, resources))
    }

    /// `/attribute @s <attribute> base set`, as `AttributeCommand` runs it,
    /// and the next tick's `refreshDirtyAttributes` (the box of a new scale).
    fn set_base(m: &mut PlayerMotion, attribute: &str, value: f64) {
        let key = m.attributes.key(attribute).unwrap();
        let handle = m.attributes.get_instance(Some(key)).unwrap().unwrap();
        m.attributes
            .edit(&handle, |attribute, observer| {
                attribute.set_base_value(value, observer)
            })
            .unwrap();
        m.refresh_attributes().unwrap();
    }

    fn assert_dims(got: Dimensions, want: (f32, f32, f32), what: &str) {
        let close = |a: f32, b: f32| (a - b).abs() <= 1e-5 * b.abs().max(1.0);
        assert!(
            close(got.width, want.0) && close(got.height, want.1) && close(got.eye_height, want.2),
            "{what}: {got:?}, want {want:?}"
        );
    }

    /// `ServerPlayer.restoreFrom(old, false)` takes the base values
    /// (`assignBaseValues`), then `setHealth(getMaxHealth())`: a player whose
    /// max health base was set comes back with that much, not 20, and a
    /// permanent modifier (`/attribute ... modifier add`) does not count, as
    /// it stays with the dead body. The attribute's range is 1..1024.
    #[test]
    fn a_respawned_player_has_the_max_health_of_their_base_values() {
        use sunrise_world::attributes::{Modifier, Operation};
        let Some((_assets, resources)) = resources() else {
            return;
        };
        for (base, bonus, want) in [
            (40.0, 0.0, 40.0),
            (10.0, 0.0, 10.0),
            (20.0, 20.0, 20.0),
            (0.0, 0.0, 1.0),
            (2048.0, 0.0, 1024.0),
        ] {
            let mut g = guest_at([0.5, 64.0, 0.5]);
            let mut m = PlayerMotion::new(&resources, Vec3::new(0.5, 64.0, 0.5)).unwrap();
            set_base(&mut m, "minecraft:max_health", base);
            let key = m.attributes.key("minecraft:max_health").unwrap();
            let handle = m.attributes.get_instance(Some(key)).unwrap().unwrap();
            let bonus = Modifier::new("test:bonus", bonus, Operation::AddValue).unwrap();
            m.attributes
                .edit(&handle, |a, o| a.add_permanent(bonus, o))
                .unwrap();
            g.motion = Some(Box::new(m));
            assert!(g.new_body(false).is_some());
            assert_eq!(g.respawn_health(&resources), want, "base {base}");
        }
        // No body to take anything from: the default.
        let g = guest_at([0.5, 64.0, 0.5]);
        assert_eq!(g.respawn_health(&resources), 20.0);
    }

    /// #3684: a glide the client announced (`START_FALL_FLYING`) is the
    /// player's pose before its body's own tick has taken it: the box in
    /// between is the gliding one at the player's scale, not at scale 1 (a
    /// 0.6 box on a player 0.1 or 2 times as big sent them back from a wall).
    #[test]
    fn a_glide_announced_before_the_body_ticks_has_the_scaled_box() {
        let Some((_assets, resources)) = resources() else {
            return;
        };
        for (scale, side) in [(0.1_f64, 0.06_f32), (2.0, 1.2)] {
            let mut g = guest_at([0.5, 64.0, 0.5]);
            let mut m = PlayerMotion::new(&resources, Vec3::new(0.5, 64.0, 0.5)).unwrap();
            set_base(&mut m, "minecraft:scale", scale);
            m.set_pose(Pose::Standing).unwrap();
            g.motion = Some(Box::new(m));
            g.fall_flying = true;
            assert_eq!(g.pose(), Pose::FallFlying);
            assert_ne!(g.motion.as_ref().unwrap().pose, Pose::FallFlying);
            let d = g.dimensions();
            assert_dims(
                d,
                (side, side, side * (0.4 / 0.6)),
                &format!("scale {scale}"),
            );
        }
    }

    #[test]
    fn told_selected_records_the_predicted_drop() {
        use crate::lan::PlayerSender;
        use std::sync::Arc;
        use std::sync::atomic::AtomicI32;
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 2,
                uuid: 2,
                name: "Dropper".into(),
                properties: Vec::new(),
                info: sunrise_protocol::packets::configuration::ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            2,
            [0.5, 64.0, 0.5],
            0,
        );
        guest.selected = 3;
        guest.inventory[3] = Some(ItemStack::new("minecraft:netherrack", 63));
        guest.told_selected();
        // Dropping one is already known to the client (63); the pickup that
        // refills the slot must then differ from `menu_sent` and be sent.
        assert_eq!(guest.menu_sent[39].as_ref().map(|s| s.count), Some(63));
        guest.inventory[3].as_mut().unwrap().count = 64;
        assert_ne!(guest.menu_sent[39], guest.menu_slot(39));
    }
}
