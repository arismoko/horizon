//! Hosting the open world on the local network ("Open to LAN"): starts the
//! server threads and the play-state hub, feeds it the world's entities each
//! tick and turns what the guests do into gameplay. A world that is not open
//! costs one `Option` check per tick.
pub(in crate::game) mod event_timings;
#[cfg(test)]
use sunrise_gameplay::lan::level_event_targets;
pub(in crate::game) mod mob_states;
pub(in crate::game) mod raw_inbox;
pub(in crate::game) mod residency;
pub(in crate::game) mod viewed_areas;

use super::shield::OFFHAND_SLOT;
use event_timings::{EventTimings, UseDetail, describe_burst, event_kind, note_event};
use std::{collections::HashMap, sync::Arc, time::Instant};
use sunrise_game_data::{Result, text::Text};
pub use sunrise_gameplay_common::game::lan_common::{LanSettings, sync};
use sunrise_gameplay_common::game::{
    api_values::TryIntoApi,
    cauldron_prediction::publish_registrations,
    commands_common::CommandsCommonGameExt2 as _,
    lan_mob_metadata::mob_effects,
    native_living_body::{active_effect, mob_effect},
};
use sunrise_level::{
    effects::{
        EffectInstance, ModifiedEffectParticles, ShownEffect, effect_particle,
        visible_effect_particles,
    },
    world::World,
};
use sunrise_protocol::{
    entity_data::{Value, serializer},
    packets::play::{self, Clientbound as C},
};
use sunrise_sim::{
    lan::{
        self, NetConfig,
        chunks::ChunkEncoder,
        configuration_tasks::{Configured, PreparedSpawn},
        data::{BIOME_REGISTRY, SyncData},
        entity::{EntityState, States},
        hub::{HostEvent, Hub, HubConfig, TickCtx},
        playerdata,
    },
    mobs::{Mob, Species, leash::Holder},
};
use sunrise_world::block_collision::BlockCollisionRegistry;

pub struct LanHost {
    pub hub: Hub,
    /// Acknowledged joins whose runtime destination closed before placement.
    pub(in crate::game) spawn_joins: Vec<(u32, Configured)>,
    /// The mods heard that everyone still here leaves with the server.
    pub players_told_left: bool,
    pub settings: LanSettings,
    pub play: HashMap<u32, crate::game::server_side::lan_actions_server::GuestPlay>,
    /// Each guest's menu owners, outside `play` (`as_guest` lifts that).
    pub menus: HashMap<u32, super::lan_menus::GuestMenus>,
    /// Blocks broken this tick (their sound is a level event for the guests).
    pub broken: Vec<[f64; 3]>,
    /// Hurt time, death and swing time (while swinging) of the mobs guests
    /// see; the players' hurt time by entity id.
    pub(in crate::game) mob_state: HashMap<super::Dim, HashMap<u32, (i32, bool, Option<i32>)>>,
    /// Each placed guest's pose as the mods last heard it
    /// (`native_entity_pose_change`), by connection.
    pub poses: HashMap<u32, sunrise_sim::pose::Pose>,
    /// The guest each mob sees as its player, when not the host (game/network/lan_mobs.rs).
    /// Looked up per mob each tick: a fast hasher (never iterated).
    pub mob_players: sunrise_level::fast_hash::FxHashMap<u32, u32>,
    /// An integrated server's owner (game/world/integrated.rs).
    pub owner: Option<Owner>,
    /// The players' locator bars, one waypoint manager per level by
    /// dimension id (game/network/lan_waypoints.rs), and the guests who are dead,
    /// whose respawn replaces their waypoint.
    pub waypoints: std::collections::HashMap<String, lan::waypoints::Waypoints>,
    pub waypoint_dead: std::collections::HashSet<u128>,
    /// Guests' commands waiting for the hub to be back in `self.lan`, where
    /// a command can reach every player (game/network/lan_actions/commands.rs).
    pub commands: Vec<sunrise_gameplay_common::game::lan_actions::GuestCommand>,
    /// Mods' quiet commands waiting to run a few a tick (game/network/lan_quiet.rs).
    pub(in crate::game) quiet: std::collections::VecDeque<super::lan_quiet::QuietBatch>,
    /// Far terrain sampled for Sunrise clients (game/network/lan_far.rs); None when
    /// `sunrise-lod-distance` is 0.
    pub far: Option<sunrise_sim::lod_net::Service>,
    /// Native mods' commands and the level each needs, once asked (game/modding/native_server.rs).
    /// The generation of the session's mod commands the hub was given.
    pub(in crate::game) native_commands_sent: Option<u64>,
    /// The players a native mod leaves out of the sleep count when they
    /// aren't in bed (`native_sleep_ignored`).
    pub sleep_ignored: std::collections::HashSet<u128>,
    /// The entities the guests track, described again every tick
    /// (`lan_entities`) over last tick's states.
    pub states: lan::entity::States,
    /// The entity id counter when this tick's states were taken for guests
    /// (`lan_note_joined`), None when no guest tracked them.
    pub(in crate::game) states_next_entity: Option<u32>,
    /// Last tick's states of the idle mobs and items (game/network/lan_idle.rs).
    pub(in crate::game) idle_states: super::lan_idle::IdleStates,
}

impl crate::game::ServerGame<'_> {
    /// The commands' `minecraft:dialog` ids as the clients get them in the
    /// configuration phase (`sync`, the world's packs' dialogs among them):
    /// `/dialog show` parses an id against these and sends its index.
    pub(in crate::game::server_side) fn sync_dialog_ids(&mut self, sync: &SyncData) {
        let Some(names) = sync.dynamic.registries.get("minecraft:dialog") else {
            return;
        };
        let ids = names.iter().map(|(name, _)| name.clone()).collect();
        self.common
            .ui
            .registries
            .by_registry
            .entry("minecraft:dialog".to_owned())
            .or_default()
            .ids = ids;
    }
}

impl crate::game::ServerGame<'_> {
    #[inline(always)]
    #[allow(dead_code)]
    pub(in crate::game) fn lan_guest_acting(&self) -> bool {
        self.server.lan_acting.is_some()
    }

    /// `PlayerList.remove`: `player.awardStat(Stats.LEAVE_GAME)` before the
    /// player is saved, heard by native mods first. Once per guest, however
    /// often they are saved.
    pub fn award_leave_game(&mut self, conn: u32, player: u128, name: &str) {
        let Some(play) = self
            .server
            .lan
            .as_mut()
            .and_then(|host| host.play.get_mut(&conn))
        else {
            return;
        };
        if std::mem::replace(&mut play.left_game, true) {
            return;
        }
        let award = if mod_api::__private::listened(mod_api::HookId::PlayerStatAward) {
            self.edited_stat_award(player, name, "minecraft:custom", "minecraft:leave_game", 1)
        } else {
            play.stats.custom("leave_game", 1);
            return;
        };
        if let Some((ty, key, n)) = award
            && let Some(play) = self
                .server
                .lan
                .as_mut()
                .and_then(|host| host.play.get_mut(&conn))
        {
            play.stats
                .add(ty.strip_prefix("minecraft:").unwrap_or(&ty), &key, n);
        }
    }

    /// `PlayerList.removeAll`: every player who joined leaves with the world.
    pub fn award_leave_game_to_all(&mut self) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let players: Vec<(u32, u128, String)> = host
            .hub
            .guests
            .iter()
            .filter(|g| g.placed && g.participates_online())
            .map(|g| (g.conn, g.uuid, g.name.clone()))
            .collect();
        for (conn, player, name) in players {
            self.award_leave_game(conn, player, &name);
        }
    }
}

/// The player who owns an integrated server's world: a guest whose state is
/// the world's own player (`players/data/<singleplayer_uuid>.dat`), as in
/// single player.
pub struct Owner {
    pub uuid: u128,
    /// The world's player as loaded, until the owner joins.
    pub saved: Option<Box<sunrise_sim::save::Level>>,
}

/// A stable UUID for an entity that has none.
pub(in crate::game::server_side) fn entity_uuid(id: u32) -> u128 {
    0x4856_5a4e_0000_0000_0000_0000_0000_0000u128 | u128::from(id)
}

/// The equipment a mob shows (ClientboundSetEquipmentPacket slots: main hand,
/// off hand, feet, legs, chest, head, body, saddle).
pub(in crate::game::server_side) fn mob_equipment(
    m: &Mob,
    hub: &mut Hub,
    out: &mut Vec<(u8, Option<sunrise_protocol::item::Stack>)>,
) {
    for (slot, stack) in m.equipment_stacks().into_iter().enumerate() {
        if stack.is_some() {
            out.push((slot as u8, hub.stack(&stack)));
        }
    }
}

/// With the game mode forced, the guests online take the default mode
/// (setting a mode does nothing when it is already theirs). A world open to
/// LAN is an integrated server, which leaves out the players who have game
/// master permission; a dedicated server does not.
pub(in crate::game::server_side) fn enforce_guest_game_mode(
    host: &mut LanHost,
    world: &World,
    collision: &BlockCollisionRegistry<'_>,
) {
    if !host.hub.config.force_game_mode {
        return;
    }
    let mode = host.hub.config.default_game_mode;
    let spares_operators = host.hub.config.integrated;
    let stale: Vec<u32> = host
        .hub
        .guests
        .iter()
        .filter(|g| g.game_mode != mode && !(spares_operators && g.permission >= 2))
        .map(|g| g.conn)
        .collect();
    for conn in stale {
        host.hub.set_game_mode(conn, mode, world, Some(collision));
    }
}

impl crate::game::ServerGame<'_> {
    /// The level's default point, independently of the acting player's bed or spawnpoint.
    pub(in crate::game) fn world_spawn_position(&self) -> [f64; 3] {
        self.common
            .level
            .world_spawn
            .map_or([0.5, 80.0, 0.5], |[x, y, z]| {
                [f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5]
            })
    }

    /// MinecraftServer.findRespawnDimension: the world spawn's level.
    pub(in crate::game) fn world_spawn_level(&self) -> super::Dim {
        super::Dim::of_world_spawn(&self.common.level)
    }

    /// MinecraftServer.setRespawnData: every placed player hears a changed world spawn.
    pub(in crate::game) fn lan_broadcast_world_spawn(&self) {
        if let Some(host) = &self.server.lan {
            for guest in host.hub.guests.iter().filter(|guest| guest.placed) {
                guest.send(&host.hub.default_spawn(
                    self.world_spawn_position(),
                    self.common.level.world_spawn_angle,
                    self.world_spawn_level().id(),
                ));
            }
        }
    }

    /// `sunrise-ctl lan status`.
    pub(in crate::game) fn lan_status(&self) -> serde_json::Value {
        use serde_json::json;
        match &self.server.lan {
            Some(h) if self.open_to_others() => json!({
                "open": true,
                // The owner's profile name.
                "host": self.options.player_name(),
                "port": h.settings.port,
                // The server's own world: its finished chunks and how many it
                // has generated or loaded so far.
                "world": {
                    "chunks": self.world.chunks.len(),
                    "generated": self.server.streaming.generated_total,
                },
                "guests": h.hub.guests.iter().map(|g| json!({
                    "name": g.name,
                    "position": g.position,
                    "placed": g.placed,
                    "loaded": g.loaded,
                    "game_mode": g.game_mode,
                    // ItemCooldowns: the ticks the shield an axe disabled stays down.
                    "shield_cooldown": h.play.get(&g.conn).map_or(0, |p| p.shield_cooldown),
                    // The item the server has this guest using: its slot and ticks.
                    "using": h.play.get(&g.conn).and_then(|p| p.using())
                        .map(|(slot, ticks)| json!({ "slot": slot, "ticks": ticks })),
                    "sneaking": g.sneaking,
                    // The server's Elytra glide (shared flag 7).
                    "fall_flying": g.fall_flying,
                    "sleeping": g.bed,
                    "health": g.health,
                    // The server's body: its air supply and golden hearts.
                    "air": g.motion.as_ref().map(|m| m.air),
                    "absorption": g.motion.as_ref().map(|m| m.health.absorption()),
                    // FoodData: food level, saturation and exhaustion.
                    "food": [
                        f64::from(g.food),
                        f64::from(g.saturation),
                        g.motion.as_ref().map_or(0.0, |m| f64::from(m.health.food.exhaustion)),
                    ],
                    "chunks": g.sent_chunks.len(),
                    "tracked": g.tracker.len(),
                })).collect::<Vec<_>>(),
            }),
            _ => json!({ "open": false }),
        }
    }

    #[allow(dead_code)]
    pub(in crate::game) fn lan_open(&mut self, settings: LanSettings) -> Result<()> {
        if self.serving() {
            self.publish(settings).map(drop).map_err(Into::into)
        } else {
            Err("no world of this game to open".into())
        }
    }
    pub(in crate::game::server_side) fn start_lan_hub(
        &mut self,
        settings: LanSettings,
        net: NetConfig,
        config: HubConfig,
    ) -> Result<()> {
        let mut sync = sync::sync_data(
            &self.common.options.resources,
            &self.common.res.core.item_data,
            self.items(),
            &self.pack_files(),
        )?;
        let features = self.enabled_feature_ids().to_vec();
        if sync.features.iter().any(|f| !features.contains(f))
            || features.iter().any(|f| !sync.features.contains(f))
        {
            // The registries are already frozen. Publish this world's flags
            // rather than the flags of the resource preload.
            let mut world_sync = SyncData::load_over(
                &self.common.options.resources,
                &self.pack_files(),
                None,
                &sync,
                self.common.res.item_defaults(),
                sync::server_sources(
                    &self.common.options.resources,
                    &self.common.res.core.item_data,
                    self.items(),
                    &self.pack_files(),
                )?
                .as_ref(),
            )?;
            world_sync.features = features;
            sync = Arc::new(world_sync);
        }
        self.sync_dialog_ids(&sync);
        // The table the world plays with (`world_biomes!`): a world with
        // biome data packs sends their ids past the built-in list, as the
        // encoder's `SectionCoder` expects.
        let biome_names: Vec<String> = world_biomes!(self)
            .list
            .iter()
            .map(|b| b.name.clone())
            .collect();
        let registry: Vec<String> = sync
            .dynamic
            .registries
            .get(BIOME_REGISTRY)
            .map(|v| v.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default();
        let mut encoder = ChunkEncoder::with_signs(
            self.common.res.facts().states.iter().map(|s| {
                (
                    s.name.clone(),
                    s.fluid_level > 0 || s.waterlogged,
                    s.sign_form(),
                )
            }),
            &sync.builtin,
            &biome_names,
            &registry,
            self.common.res.shared_items(),
        );
        encoder.tables = Some(
            self.server
                .save
                .0
                .tables
                .get_or_init(|| {
                    Arc::new(sunrise_sim::save::SaveTables::new(&self.common.pack.assets))
                })
                .clone(),
        );
        self.lan_update_tag_sources(&mut encoder);
        let far = (net.lod_distance > 0)
            .then(|| sunrise_sim::lod_net::Service::new(net.lod_distance, biome_names.clone()));
        let lan = super::lan_configuration_tasks::start(sync.clone(), net)?;
        let mut hub = Hub::new(lan, sync, encoder, config, self.common.loop_context.clone());
        hub.allow_commands = self.allow_commands();
        hub.dispatch_move_effects = true;
        self.bind_guest_residency(&mut hub);
        self.common.passages.record_on(&mut hub);
        hub.rule_registry = self.common.level.rule_registry.clone();
        hub.sync_attachments = !mod_api::__private::mods().is_empty();
        hub.flat_world = self.common.world_flat;
        hub.debug_world = self.server.world_debug;
        let port = hub.lan.port;
        self.common.world.block_log = Some(Vec::new());
        self.common.world.connection_group_log = Some(Default::default());
        self.common.world.light_log = Some(Default::default());
        self.server.lan = Some(LanHost {
            players_told_left: false,
            hub,
            spawn_joins: Vec::new(),
            settings: LanSettings { port, ..settings },
            play: HashMap::new(),
            menus: HashMap::new(),
            broken: Vec::new(),
            mob_state: HashMap::new(),
            poses: HashMap::new(),
            mob_players: Default::default(),
            owner: None,
            waypoints: Default::default(),
            waypoint_dead: Default::default(),
            commands: Vec::new(),
            quiet: std::collections::VecDeque::new(),
            far,
            native_commands_sent: None,
            sleep_ignored: Default::default(),
            states: Default::default(),
            states_next_entity: None,
            idle_states: Default::default(),
        });
        self.start_audio_capture();
        Ok(())
    }

    /// `ServerConnectionListener.stop`: stop accepting new connections and
    /// refuse incoming logins before player and world shutdown saves run,
    /// while keeping connected guest entries alive for `save_leaving_guests`
    /// and `lan_disconnect`.
    pub(in crate::game) fn stop_network_transport(&mut self) {
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.lan.shared.refuse_logins();
            host.hub.lan.stop();
            let _ = host.hub.lan.poll();
        }
    }

    /// The transport close, separate from player callbacks and saves so that
    /// server shutdown can still disconnect after one of those unwinds.
    pub fn lan_disconnect(&mut self, announce: bool) {
        let Some(mut host) = self.server.lan.take() else {
            return;
        };
        let had_guests = !host.hub.guests.is_empty();
        host.hub.disconnect_all();
        if had_guests {
            // Give the network threads a moment to flush the disconnect packets.
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        self.common.world.block_log = None;
        self.common.world.connection_group_log = None;
        self.common.world.light_log = None;
        self.stop_audio_capture();
        self.server.bound_sounds.clear();
        self.server.lan_events.clear();
        self.server.lan_songs.clear();
        self.server.lan_critical_hits.clear();
        self.server.lan_cushion_breaks.clear();
        self.server.streaming.extra.clear();
        if announce {
            self.server_system_message(Text::translate(
                "menu.multiplayerOptions.publish.stopped",
                &[],
            ));
        }
    }

    /// `PlayerList.removeAll`: every guest leaves in turn, the vehicle it
    /// alone rides with it (`PlayerList.remove`), and is saved.
    pub fn save_leaving_guests(&mut self) -> bool {
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        // Remove the open menu and window 0 in the guest's own level before
        // saving, so held stacks and inputs return to their inventory or drop
        // there if full. An interrupted guest's file is not written.
        let back = self.common.dim;
        let open: Vec<(u32, super::Dim)> = host
            .hub
            .guests
            .iter()
            .filter(|g| {
                g.placed
                    && g.participates_in_player_simulation()
                    && !self.server.crash_save.excludes(g.conn)
            })
            .map(|g| (g.conn, super::Dim::from_id(&g.dimension)))
            .collect();
        let mut holds = Vec::new();
        for (conn, dim) in open {
            if self.enter_level(dim).is_ok() {
                holds.push(self.guest_close_menus_for_save(conn));
            }
        }
        if let Err(e) = self.enter_level(back) {
            log::error!("could not reopen {}: {e}", back.id());
        }
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        let count = host.hub.guests.len();
        let owner = host.owner.as_ref().map(|owner| owner.uuid);
        let mut owner_saved = false;
        for i in 0..count {
            let Some(host) = self.server.lan.as_mut() else {
                break;
            };
            if !host.hub.guests[i].participates_in_player_simulation() {
                continue;
            }
            let mut g = host.hub.guests.take_for_save(i);
            self.take_parent_vehicle(&mut g);
            if self.save_guest(&mut g) && owner == Some(g.uuid) {
                owner_saved = true;
            }
            let Some(host) = self.server.lan.as_mut() else {
                break;
            };
            host.hub.guests.insert(i, g);
        }
        // These players are leaving: a menu a closing listener asked for never opens.
        for hold in holds {
            self.discard_menu_opens(hold);
        }
        owner_saved
    }
    /// Save a guest when they leave or the world closes (after
    /// `take_parent_vehicle`, `PlayerList.save`): an integrated server's owner
    /// into the level (`/data`) and both its files (`playerdata::save_owner`),
    /// anyone else into their own file.
    pub fn save_guest(&mut self, guest: &mut sunrise_sim::lan::guest::Guest) -> bool {
        if !guest.participates_in_player_simulation() {
            return false;
        }
        // A proxy released this record to another server, which now owns saving it.
        let saved =
            if guest.joined() && !guest.released && !self.server.crash_save.excludes(guest.conn) {
                let _ = self.finish_player_autosaves(true);
                if self
                    .server
                    .lan
                    .as_ref()
                    .and_then(|host| host.owner.as_ref())
                    .is_some_and(|o| o.uuid == guest.uuid)
                {
                    playerdata::save_owner(
                        guest,
                        &mut self.common.level,
                        &self.server.world_root,
                        self.common.res.items(),
                    )
                } else {
                    playerdata::save(guest, &self.server.world_root, self.common.res.items())
                }
            } else {
                false
            };
        if saved {
            let directory = playerdata::directory(&self.server.world_root);
            self.native_player_save_to_file(guest.uuid, &guest.name, &directory);
        }
        if let Some(host) = self.server.lan.as_ref() {
            self.save_guest_records(host, guest);
        }
        saved
    }
    /// The vehicles the other players in level `dim` ride, all but the
    /// guest `but` (`Entity.hasExactlyOnePlayerPassenger`); `host` adds the
    /// host's own player, in the level in place.
    pub(in crate::game::server_side) fn ridden_by_others(
        &self,
        guests: &[sunrise_sim::lan::guest::Guest],
        dim: &str,
        but: Option<u128>,
        host: bool,
    ) -> Vec<u32> {
        let mut others: Vec<u32> = guests
            .iter()
            .filter(|g| g.placed && g.dimension == dim && Some(g.uuid) != but)
            .filter_map(|g| g.vehicle)
            .collect();
        if host && super::Dim::from_id(dim) == self.common.dim {
            others.extend(self.common.riding);
        }
        others
    }
    /// `ServerPlayer.saveParentVehicle` of a player riding `vehicle` in level
    /// `dim`, put in place for it if need be; with `remove`, the vehicle tree
    /// leaves the world with the player (`PlayerList.remove`).
    pub(in crate::game::server_side) fn parent_vehicle_in(
        &mut self,
        dim: &str,
        vehicle: Option<u32>,
        others: &[u32],
        remove: bool,
    ) -> Option<(sunrise_core::nbt::Tag, [i32; 4])> {
        let vehicle = vehicle?;
        let back = self.common.dim;
        let theirs = super::Dim::from_id(dim);
        if theirs != back
            && let Err(e) = self.enter_level(theirs)
        {
            log::error!("could not open {}: {e}", theirs.id());
            return None;
        }
        let saved = if self.common.dim == theirs {
            self.snapshot_parent_vehicle(vehicle, others, remove)
        } else {
            None
        };
        if self.common.dim != back
            && let Err(e) = self.enter_level(back)
        {
            log::error!("could not reopen {}: {e}", back.id());
        }
        saved
    }
    /// `PlayerList.remove` of a guest out of the hub: the vehicle it alone rides
    /// leaves the world, and the guest stops riding (`unRide`). The file takes
    /// a persistent tree as `RootVehicle` (`saved` holds it until the next ride).
    /// A guest the crash save excludes keeps the vehicle in the world: its file
    /// is not written, so nothing would hold it.
    pub(in crate::game::server_side) fn take_parent_vehicle(
        &mut self,
        guest: &mut sunrise_sim::lan::guest::Guest,
    ) {
        // An interrupted player's file is not written, so the vehicle stays in the world.
        if self.server.crash_save.excludes(guest.conn) {
            return;
        }
        let Some(vehicle) = guest.vehicle.take() else {
            return;
        };
        if !guest.placed {
            return;
        }
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let others = self.ridden_by_others(&host.hub.guests, &guest.dimension, None, true);
        let Some((tag, attach)) =
            self.parent_vehicle_in(&guest.dimension, Some(vehicle), &others, true)
        else {
            return;
        };
        let saved = guest
            .saved
            .get_or_insert_with(|| sunrise_core::nbt::Tag::Compound(Default::default()));
        if let sunrise_core::nbt::Tag::Compound(p) = saved {
            p.insert("RootVehicle".into(), tag);
        }
        guest.saved_vehicle = Some(attach);
    }
    /// `ServerPlayer.loadAndSpawnParentVehicle`: a guest whose file names the
    /// vehicle it rode rides it again, once the vehicle is in the world.
    pub(in crate::game::server_side) fn restore_guest_vehicles(&mut self) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let waiting: Vec<(usize, [i32; 4])> = host
            .hub
            .guests
            .iter()
            .enumerate()
            .filter_map(|(i, g)| Some((i, g.saved_vehicle?)))
            .filter(|(i, _)| {
                let g = &host.hub.guests[*i];
                let chunk = (
                    (g.position[0].floor() as i32).div_euclid(16),
                    (g.position[2].floor() as i32).div_euclid(16),
                );
                g.placed
                    && g.vehicle.is_none()
                    && g.dimension == self.common.dim.id()
                    && g.sent_chunks.contains(&chunk)
            })
            .collect();
        for (i, uuid) in waiting {
            let Some(host) = self.server.lan.as_ref() else {
                return;
            };
            let raw = host.hub.guests[i].saved.clone();
            let entity = host.hub.guests[i].entity_id as u32;
            let Some(id) = self.restore_saved_vehicle(uuid, raw.as_ref()) else {
                continue;
            };
            let order = self.boarding_order(id, entity, true);
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            host.hub.guests.update(i, |g| {
                if let Some(sunrise_core::nbt::Tag::Compound(p)) = &mut g.saved {
                    p.remove(&"RootVehicle".into());
                }
                g.vehicle = Some(id);
                g.vehicle_order = order;
                g.saved_vehicle = None;
                // Pair the stationary restored entities before their passenger packet.
                g.moved = true;
            });
        }
    }
    /// The file name of a player's advancements and statistics: an
    /// integrated server's owner keeps single player's.
    pub(in crate::game::server_side) fn records_uuid(
        &self,
        host: &LanHost,
        uuid: u128,
    ) -> Option<[i32; 4]> {
        if host.owner.as_ref().is_some_and(|o| o.uuid == uuid) {
            self.common.level.player_uuid
        } else {
            Some(playerdata::uuid_ints(uuid))
        }
    }
    /// `PlayerList.saveAll`: statistics belong to each connected player's
    /// live record, independently of the dirty entries sent to their client.
    pub fn save_connected_guest_statistics(&self, host: &LanHost) {
        for guest in &host.hub.guests {
            if !guest.joined() || guest.released || self.server.crash_save.excludes(guest.conn) {
                continue;
            }
            let Some(uuid) = self.records_uuid(host, guest.uuid) else {
                continue;
            };
            if let Some(play) = host.play.get(&guest.conn) {
                play.stats
                    .write(&sunrise_gameplay_common::stats::player_json(
                        &self.server.world_root,
                        "stats",
                        uuid,
                    ));
            }
        }
    }

    /// The advancements and statistics files of a held arrival, before it is placed
    /// (`player-confirm`): each through a new file, synced and renamed into place
    /// (`save::write_durable`), so the reply's `ok` means both are on the disk whole.
    pub(in crate::game::server_side) fn write_arrival_records(
        &self,
        host: &LanHost,
        guest: &sunrise_sim::lan::guest::Guest,
    ) -> std::result::Result<(), String> {
        if self.common.adv.tree.list.is_empty() {
            return Ok(());
        }
        let Some(uuid) = self.records_uuid(host, guest.uuid) else {
            return Ok(());
        };
        let write = |dir: &str, json: &serde_json::Value| {
            let path =
                sunrise_gameplay_common::stats::player_json(&self.server.world_root, dir, uuid);
            let text = serde_json::to_string_pretty(json).map_err(|e| e.to_string())?;
            sunrise_sim::save::write_durable(&path, text.as_bytes())
                .map_err(|e| format!("{} is not saved: {e}", path.display()))
        };
        write(
            "advancements",
            &guest
                .advancements
                .to_json(&self.common.adv.tree, sunrise_world::DATA_VERSION),
        )?;
        if let Some(play) = host.play.get(&guest.conn) {
            write("stats", &play.stats.to_json())?;
        }
        Ok(())
    }
    /// `PlayerAdvancements.load` and `ServerStatsCounter.parseLocal` of a
    /// network player who joined; every statistic goes out on the first
    /// request (`PlayerList.placeNewPlayer`'s `markAllDirty`).
    pub(in crate::game::server_side) fn load_guest_records(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some((uuid, name)) = host
            .hub
            .guest(conn)
            .and_then(|g| Some((self.records_uuid(host, g.uuid)?, g.name.clone())))
        else {
            return;
        };
        let tree = self.advancement_tree();
        let path = sunrise_gameplay_common::stats::player_json(
            &self.server.world_root,
            "advancements",
            uuid,
        );
        let saved = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        if let Some(json) = saved {
            let _ = host.hub.update_guest(conn, |g| {
                let mut progress = sunrise_sim::advancements::Progress::from_json(&json);
                // PlayerAdvancements.applyFrom: advancements this server lacks are dropped.
                progress
                    .criteria
                    .retain(|id, _| tree.by_id.contains_key(id));
                g.advancements = progress;
                g.advancements_loaded = true;
            });
        }
        // PlayerList.getPlayerStats: locateStatsFile moves an old name-based file first.
        let mut stats = sunrise_gameplay_common::stats::Stats::read(
            &sunrise_gameplay_common::stats::locate_player_json(
                &self.server.world_root,
                "stats",
                uuid,
                &name,
            ),
        );
        stats.mark_all_dirty();
        host.play.entry(conn).or_default().stats = stats;
    }
    /// The integrated server's owner into the world's player, for `Level::write`.
    /// Returns the owner's uuid and name when they were stored.
    pub fn store_owner(&mut self) -> Option<(u128, String)> {
        let uuid = self
            .server
            .lan
            .as_ref()
            .and_then(|h| h.owner.as_ref())?
            .uuid;
        let (conn, riding, dim, others) = self.server.lan.as_ref().and_then(|h| {
            let g = h.hub.online_players().find(|g| {
                g.uuid == uuid && !g.released && !self.server.crash_save.excludes(g.conn)
            })?;
            let others = self.ridden_by_others(&h.hub.guests, &g.dimension, Some(uuid), true);
            Some((g.conn, g.vehicle, g.dimension.clone(), others))
        })?;
        // The vehicle the owner rides (`ServerPlayer.saveParentVehicle`).
        let vehicle = self
            .parent_vehicle_in(&dim, riding, &others, false)
            .map(|(tag, _)| tag);
        let mut host = self.server.lan.take()?;
        let mut stored = None;
        if let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) {
            // The owner's map is the game's only during their own turn
            // (`guest_turn`); otherwise their record holds it, whoever acts.
            let live = (self.server.lan_acting_uuid == Some(uuid))
                .then(|| self.common.player_state.attachments.clone());
            let mut store_ok = true;
            host.hub.guests.update(i, |g| {
                let held = live.map(|a| std::mem::replace(&mut g.attachments, a));
                if let Err(error) = playerdata::try_store(g, &mut self.common.level, vehicle) {
                    log::error!("Sunrise: the owner's playerdata is not saved: {error}");
                    store_ok = false;
                }
                if let Some(held) = held {
                    g.attachments = held;
                }
            });
            let g = &host.hub.guests[i];
            self.save_guest_records(&host, g);
            if store_ok {
                stored = Some(g.name.clone());
            }
        }
        self.server.lan = Some(host);
        stored.map(|name| (uuid, name))
    }

    /// The world's default game mode (`defaultgamemode`).
    pub(in crate::game) fn set_default_game_mode(&mut self, mode: i32) {
        self.common.level.game_mode = mode;
        self.common.level.creative = mode == 1;
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.config.default_game_mode = mode;
            // `setWorldGameType` and `DefaultGameModeCommands.setMode`: a
            // world that forces its mode puts the guests online in it.
            enforce_guest_game_mode(host, &self.common.world, self.common.collision);
        }
    }

    /// Apply the default mode to connected players when forcing it is enabled.
    pub(in crate::game::server_side) fn set_force_game_mode(&mut self, force: bool) {
        if let Some(host) = self.server.lan.as_mut() {
            host.settings.force_game_mode = force;
            host.hub.config.force_game_mode = force;
            enforce_guest_game_mode(host, &self.common.world, self.common.collision);
        }
    }

    /// Update the live view distance used for player chunk tracking.
    pub(in crate::game) fn set_view_distance(&mut self, distance: i32) -> bool {
        if !self.is_logical_server() {
            return false;
        }
        let Some(host) = self.server.lan.as_mut() else {
            return false;
        };
        host.hub.set_view_distance(distance);
        self.server.settings.render_distance =
            sunrise_sim::lan::hub::effective_view_distance(distance);
        true
    }

    /// Update the live spawn-protection radius.
    pub fn set_spawn_protection(&mut self, radius: i32) {
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.spawn_protection = radius;
        }
    }

    /// Update the live simulation distance sent to player clients.
    pub(in crate::game) fn set_simulation_distance(&mut self, distance: i32) -> bool {
        if !self.is_logical_server() {
            return false;
        }
        let Some(host) = self.server.lan.as_mut() else {
            return false;
        };
        host.hub.set_simulation_distance(distance);
        self.server.settings.simulation_distance = distance;
        true
    }

    /// Update the live entity tracking range percentage.
    pub(in crate::game::server_side) fn set_entity_broadcast_range_percentage(
        &mut self,
        percentage: i32,
    ) {
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.entity_range_percent = percentage;
        }
    }

    /// Update the live movement check's flight policy.
    pub(in crate::game::server_side) fn set_allow_flight(&mut self, allow: bool) {
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.config.allow_flight = allow;
        }
    }

    /// Update whether status responses include online player names.
    pub(in crate::game::server_side) fn set_hide_online_players(&mut self, hide: bool) {
        if let Some(host) = self.server.lan.as_ref() {
            host.hub.lan.shared.set_hide_online_players(hide);
        }
    }

    /// Update whether the server answers connection status requests.
    pub(in crate::game::server_side) fn set_status_replies(&mut self, enable: bool) {
        if let Some(host) = self.server.lan.as_ref() {
            host.hub.lan.shared.set_enable_status(enable);
        }
    }

    /// New command access and game mode forcing settings of the open world.
    pub(in crate::game) fn lan_apply_settings(&mut self, guest_commands: bool, force: bool) {
        if let Some(host) = self.server.lan.as_mut() {
            host.settings.guest_commands = guest_commands;
            host.hub.config.guest_commands = guest_commands;
        }
        self.set_force_game_mode(force);
        self.resend_commands_filtered();
    }

    /// The entities guests track: mobs, items and the rest (game/network/lan_entities.rs).
    /// Only the entities whose id is above `after` (0: all of them). With
    /// `kept`, idle mobs and items no client can pair with are described bare
    /// (game/network/lan_idle.rs).
    pub fn lan_entities(
        &mut self,
        out: &mut States,
        drain_minecart_steps: bool,
        after: u32,
        mut kept: Option<&mut super::lan_idle::IdlePass<'_>>,
    ) {
        out.clear();
        let mut speeds = super::lan_attributes::SpeedByType::default();
        if mod_api::__private::listened(mod_api::HookId::EffectParticleModification) {
            self.lan_effect_particles();
        }
        // ChunkMap.tick sends the changes of an entity outside the
        // entity-ticking range only when it changed section or needs sync.
        let levels = self.server.tickets.0.levels();
        let idle_at = |p: sunrise_core::geometry::Vec3| {
            !levels.in_entity_ticking_range((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4)
        };
        for m in self
            .common
            .mobs
            .iter()
            .filter(|m| !m.removed && m.id > after)
        {
            let Some(host) = self.server.lan.as_ref() else {
                return;
            };
            let (mob_players, hub) = (&host.mob_players, &host.hub);
            // The lead's holder: the player the mob sees, another mob or a
            // fence knot (game/network/lan_entities.rs).
            let player_entity = || {
                (mob_players.get(&m.id))
                    .and_then(|conn| hub.guest(*conn))
                    .map(|g| g.entity_id)
            };
            let leash = m.extra.leash.and_then(|h| match h {
                Holder::Player => match m.extra.leash_player {
                    Some(u) => hub
                        .guests
                        .iter()
                        .find(|g| playerdata::uuid_ints(g.uuid) == u)
                        .map(|g| g.entity_id),
                    None => player_entity(),
                },
                Holder::Mob(id) | Holder::Boat(id) => Some(id as i32),
                Holder::Knot(p) => self.server.knot_ids.get(&p).map(|&id| id as i32),
            });
            let idle = idle_at(m.position);
            let place = super::lan_idle::Place {
                position: [m.position.x, m.position.y, m.position.z],
                rotation: (m.yaw, m.head_pitch, m.head_yaw),
                on_ground: m.ground.on_ground,
                velocity: [m.velocity.x, m.velocity.y, m.velocity.z],
            };
            let kind = m.kind.id.as_str();
            let bare = idle
                && (kept.as_deref_mut()).is_some_and(|k| k.bare(hub, m.id as i32, kind, &place))
                && !m.needs_sync;
            let e = out.add(m.id as i32, entity_uuid(m.id), kind);
            e.position = place.position;
            (e.yaw, e.pitch, e.head_yaw) = place.rotation;
            e.on_ground = place.on_ground;
            e.velocity = place.velocity;
            e.needs_sync = m.needs_sync;
            e.sync_position = m.sync_position;
            e.stepped = true;
            (e.idle, e.bare) = (idle, bare);
            e.leash = leash;
            if bare {
                continue;
            }
            self.fill_mob_state(m, hub, player_entity(), &mut speeds, e);
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            mob_equipment(m, &mut host.hub, &mut e.equipment);
        }
        for d in self.common.dropped.iter().filter(|d| d.id > after) {
            let Some(host) = self.server.lan.as_ref() else {
                return;
            };
            let idle = idle_at(d.position);
            let place = super::lan_idle::Place {
                position: [d.position.x, d.position.y, d.position.z],
                rotation: (0.0, 0.0, 0.0),
                on_ground: d.ground.on_ground,
                velocity: [d.velocity.x, d.velocity.y, d.velocity.z],
            };
            let bare = idle
                && (kept.as_deref_mut())
                    .is_some_and(|k| k.bare(&host.hub, d.id as i32, "minecraft:item", &place))
                && !d.needs_sync;
            let e = out.add(d.id as i32, entity_uuid(d.id), "minecraft:item");
            e.position = place.position;
            e.on_ground = place.on_ground;
            e.velocity = place.velocity;
            (e.idle, e.bare) = (idle, bare);
            if bare {
                continue;
            }
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            let stack = host.hub.stack(
                &Some(d.stack.clone()).filter(|s| s.count > 0 && &*s.item != "minecraft:air"),
            );
            // `ServerEntity.getRequiresPrecisePosition` is false for an item:
            // `needs_sync` asks for its data again, not for an exact position.
            // The collision case is `full_precision_required` in the tracker.
            e.precise = false;
            e.needs_sync = d.needs_sync;
            e.vertical_collision = d.vertical_collision;
            e.horizontal_collision = d.ground.horizontal_collision;
            // Entity.DATA_SHARED_FLAGS_ID: glowing 0x40 (the Glowing tag).
            if self
                .common
                .persist_raw(d.id)
                .and_then(|t| sunrise_sim::save::number(t.get("Glowing")))
                .is_some_and(|b| b != 0)
            {
                e.meta.push((0, serializer::BYTE, Value::Byte(0x40)));
            }
            // Entity.DATA_NO_GRAVITY, which `setNoGravity` writes: also false by
            // default, so a flag taken away goes out too.
            if d.no_gravity {
                e.meta.push((5, serializer::BOOLEAN, Value::Bool(true)));
            }
            e.defaults
                .push((5, serializer::BOOLEAN, Value::Bool(false)));
            e.meta.push((8, serializer::ITEM_STACK, Value::Item(stack)));
        }
        for o in self.common.orbs.iter().filter(|o| o.id > after) {
            let e = out.add(o.id as i32, entity_uuid(o.id), "minecraft:experience_orb");
            e.position = [o.position.x, o.position.y, o.position.z];
            e.idle = idle_at(o.position);
            e.on_ground = o.ground.on_ground;
            e.velocity = [o.velocity.x, o.velocity.y, o.velocity.z];
            // ExperienceOrb.DATA_VALUE.
            e.meta.push((8, serializer::INT, Value::Int(o.value)));
        }
        self.lan_object_states(out, drain_minecart_steps, after);
        // PersistentEntitySectionManager only tracks accessible sections.
        out.retain(|entity| self.entity_position_accessible(entity.position));
    }

    /// `LivingEntity.updateSynchronizedMobEffectParticles` while a native mod
    /// listens to `effect_particle_modification`: the mobs and players whose
    /// effects changed since the mods were last asked (vanilla's
    /// `effectsDirty`) are asked again, and what they leave is kept and sent
    /// until the next change. An entity whose effects are gone has none
    /// (`removeEffectParticles`).
    pub(in crate::game::server_side) fn lan_effect_particles(&mut self) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        for i in 0..self.common.mobs.len() {
            let m = &self.common.mobs[i];
            if m.removed {
                continue;
            }
            if let Some(update) = self.modified_effect_particles(
                &host.hub,
                m.id,
                m.extra.modified_effect_particles.as_ref(),
                || mob_effects(m),
                |i| mob_effect(m, &m.extra.effects[i]),
            ) {
                self.common.mobs[i].extra.modified_effect_particles = update;
            }
        }
        for i in 0..host.hub.guests.len() {
            let Some(host) = self.server.lan.as_ref() else {
                return;
            };
            let (hub, g) = (&host.hub, &host.hub.guests[i]);
            if !hub.here(g) {
                continue;
            }
            if let Some(update) = self.modified_effect_particles(
                hub,
                g.entity_id as u32,
                g.modified_effect_particles.as_ref(),
                || g.effects.list.iter().map(EffectInstance::shown),
                |i| active_effect(&g.effects.list[i]),
            ) && let Some(host) = self.server.lan.as_mut()
            {
                host.hub
                    .guests
                    .update(i, |g| g.modified_effect_particles = update);
            }
        }
    }

    /// What the living entity `entity`, with `effects` (`active` names the
    /// nth for the mods) and `kept` from the last time, keeps now: None while
    /// its effects are unchanged, else the mods' answers for them
    /// (`effect_particle_modification`, once per effect), or none when it has
    /// no effects.
    pub(in crate::game::server_side) fn modified_effect_particles<
        I: Iterator<Item = ShownEffect>,
    >(
        &self,
        hub: &Hub,
        entity: u32,
        kept: Option<&ModifiedEffectParticles>,
        effects: impl Fn() -> I,
        active: impl Fn(usize) -> mod_api::ActiveEffect,
    ) -> Option<Option<ModifiedEffectParticles>> {
        if effects().next().is_none() {
            return kept.is_some().then_some(None);
        }
        if kept.is_some_and(|k| k.computed_for(effects())) {
            return None;
        }
        // Before the world has an epoch no handle can name the entity.
        let living = (self.common.native_lifecycle.epoch())
            .map(|epoch| mod_api::LivingRef::new(mod_api::EntityRef::new(epoch, entity)));
        let (particles, ambient) =
            visible_effect_particles(effects().enumerate().filter_map(|(i, e)| {
                let original = effect_particle(e.effect, e.ambient);
                let mut particle = original.clone();
                let mut visible = e.visible;
                // An effect with no particle has no registry entry to name it by.
                if let (Some(living), Some(original)) = (living, &original) {
                    self.native_effect_particle_modification(
                        living,
                        &active(i),
                        original,
                        &mut particle,
                        &mut visible,
                    );
                }
                visible.then(|| (e.ambient, particle.or(original)))
            }));
        let meta = hub.particle_meta(&particles, ambient);
        Some(Some(ModifiedEffectParticles::new(effects(), meta)))
    }

    /// Apply gameplay events produced while the hub handles guest packets.
    ///
    /// Both the pre-block packet phase and the normal LAN tick receive the
    /// same event stream. Keep their side effects in one place so joins,
    /// departures, chat, and queued guest commands cannot drift apart. None when a native hook's
    /// command closed the hub.
    pub fn lan_process_events(
        &mut self,
        host: LanHost,
        events: impl IntoIterator<Item = HostEvent>,
        entities: &[EntityState],
        spawn: [f64; 3],
    ) -> Option<LanHost> {
        // A slow tick's line says which event took it, and whose (`spans::note`): each event's
        // time is noted when the next one starts (an event's arm may `continue`), the last's
        // after the loop.
        let timed = sunrise_level::spans::recording_here();
        let mut previous = None;
        // The arms below borrow the hub through `self.lan` (their helpers take no host:
        // the fork's borrow shape), so the host is put back for the match and taken out
        // again on the way through.
        self.server.lan = Some(host);
        let mut timings = EventTimings::new(sunrise_level::spans::NOTE_MS);
        for event in events {
            if timed && let Some(host) = self.server.lan.as_ref() {
                note_event(host, previous.take(), &mut timings);
            }
            if timed {
                let detail = self
                    .server
                    .lan
                    .as_ref()
                    .and_then(|host| self.event_detail(host, &event));
                previous = Some((Instant::now(), event_kind(&event), detail));
            }
            match event {
                HostEvent::InputCredits(credits) => drop(credits),
                HostEvent::RawPayload(delivery) => {
                    self.run_payload(delivery.endpoint, delivery.incoming);
                }
                HostEvent::PlayerPong { conn, id } => {
                    let Some(guest) = self.server.lan.as_ref()?.hub.guest(conn).filter(|g| {
                        g.joined() && g.participates_online() && g.play_connection_open()
                    }) else {
                        continue;
                    };
                    let Some(epoch) = self.common.native_lifecycle.epoch() else {
                        continue;
                    };
                    let player = mod_api::__private::game::player_ref(
                        mod_api::EntityRef::new(epoch, guest.entity_id as u32),
                        guest.uuid,
                    );
                    let name = guest.name.clone();
                    self.native_player_pong(crate::game::native_hooks::fire::PlayerPongCall {
                        player,
                        name: &name,
                        id,
                    });
                }
                event @ (HostEvent::Warn(_) | HostEvent::Error(_)) => {
                    self.lan_log_event(event);
                }
                HostEvent::ChatSubmitted {
                    message,
                    bound,
                    name,
                } => {
                    let host = self.server.lan.as_ref()?;
                    let Some((id, conn)) = host
                        .hub
                        .guests
                        .iter()
                        .find(|g| g.placed && g.participates_online() && g.uuid == message.sender)
                        .map(|g| (g.entity_id, g.conn))
                    else {
                        continue;
                    };
                    let mut message = *message;
                    let sender = message.sender;
                    let raw = message.content.clone();
                    // `PlayerChatMessage.decoratedContent`: the text as a literal.
                    let original = Text::untrusted(raw.clone());
                    let mut text = original.clone();
                    let mut cancel = false;
                    let mut replies = Vec::new();
                    let mut engine_replies = Vec::new();
                    if mod_api::__private::listened(mod_api::HookId::ServerChat)
                        && let Ok(mut api_text) = text.try_into_api()
                    {
                        self.native_server_chat(mod_api::__private::ServerChatCall {
                            player_id: id,
                            player: sender,
                            username: &name,
                            raw_text: &raw,
                            message: &mut api_text,
                            replies: &mut replies,
                            cancel: &mut cancel,
                        });
                        // Validate all fields before applying any event writes.
                        let converted: std::result::Result<_, mod_api::text::TextFormError> =
                            (|| {
                                Ok((
                                    sunrise_sim::api_text::to_serialized_engine(&api_text)?,
                                    replies
                                        .iter()
                                        .map(sunrise_sim::api_text::to_serialized_engine)
                                        .collect::<std::result::Result<Vec<_>, _>>()?,
                                ))
                            })();
                        match converted {
                            Ok((message, answers)) => {
                                text = message;
                                engine_replies = answers;
                            }
                            Err(_) => {
                                cancel = false;
                            }
                        }
                    }
                    // The mods' replies reach the sender alone, as a
                    // command's do, before the message (if any) goes out.
                    if !engine_replies.is_empty() {
                        let host = self.server.lan.as_ref()?;
                        for line in engine_replies {
                            host.hub.send_to(
                                conn,
                                &C::SystemChat(play::SystemChat {
                                    content: sunrise_sim::lan::data::component_nbt(
                                        &line.component(),
                                    ),
                                    overlay: false,
                                }),
                            );
                        }
                    }
                    // Canceled: stop before `detectRateSpam`.
                    if cancel {
                        continue;
                    }
                    if text != original {
                        // `withUnsignedContent`: clients show the mod's text,
                        // the signature still covers what was typed.
                        message.unsigned_content =
                            Some(sunrise_sim::lan::data::component_nbt(&text.component()));
                    }
                    let shown_name = sunrise_game_data::scoreboard::comp_text(
                        &sunrise_game_data::scoreboard::tag_to_json(&bound.name),
                    );
                    let line = Text::translate("chat.type.text", &[shown_name, text.clone()]);
                    self.lan_broadcast_chat(
                        (sender, &name),
                        &text.plain(),
                        line,
                        &bound,
                        super::lan_chat::ChatDelivery::Player(&message),
                    );
                    self.lan_player_chatted(sender, &raw);
                }
                HostEvent::ChatSessionStarted(update) => {
                    let host = self.server.lan.as_mut()?;
                    host.hub.broadcast(&C::PlayerInfoUpdate(*update));
                }
                HostEvent::Joined(conn, configured) => {
                    let host = self.server.lan.as_ref()?;
                    if host
                        .hub
                        .guest(conn)
                        .is_none_or(|g| !g.left_reason.is_empty())
                    {
                        continue;
                    }
                    let prepared = if let Some(ref lease) = configured {
                        if lease.session().closed() {
                            continue;
                        }
                        match self.acknowledged_spawn(lease) {
                            Some(Ok(spawn)) => spawn,
                            Some(Err(error)) => {
                                log::warn!(
                                    "Could not prepare spawn after acknowledgement: {error}"
                                );
                                self.server.lan.as_mut()?.hub.kick(
                                    conn,
                                    sunrise_protocol::text::translatable(
                                        "multiplayer.disconnect.invalid_player_data",
                                        &[],
                                    ),
                                );
                                continue;
                            }
                            None => {
                                self.server
                                    .lan
                                    .as_mut()?
                                    .spawn_joins
                                    .push((conn, configured.unwrap()));
                                continue;
                            }
                        }
                    } else {
                        None
                    };
                    // The mods may refuse the login before the player is placed.
                    if let Some((uuid, name)) = self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| (g.uuid, g.name.clone()))
                    {
                        let mut cancel = false;
                        let mut reason = None;
                        self.native_player_login(uuid, &name, &mut cancel, &mut reason);
                        if cancel {
                            let reason = match reason {
                                Some(text) => {
                                    sunrise_sim::lan::data::component_nbt(&text.component())
                                }
                                None => sunrise_protocol::text::translatable(
                                    "multiplayer.disconnect.generic",
                                    &[],
                                ),
                            };
                            let host = self.server.lan.as_mut()?;
                            host.hub.kick(conn, reason);
                            continue;
                        }
                    }
                    let host = self.server.lan.as_mut()?;
                    // PlayerList.placeNewPlayer: the name the user cache knew,
                    // before it learns this one, for a renamed player's join line.
                    let mut renamed = None;
                    if self.common.options.execution_role.is_dedicated_server()
                        && let Some((uuid, name)) =
                            host.hub.guest(conn).map(|g| (g.uuid, g.name.clone()))
                        && let Some(old) = self
                            .server
                            .user_cache
                            .as_mut()
                            .and_then(|c| c.get_by_id(uuid))
                    {
                        if !old.name.eq_ignore_ascii_case(&name) {
                            renamed = Some(old.name.clone());
                        }
                        host.hub.former_names.insert(uuid, old.name);
                    }
                    self.lan_guest_joined(conn, entities, spawn, prepared);
                    let host = self.server.lan.as_ref()?;
                    let guest = host.hub.guest(conn);
                    publish_registrations(&super::cauldron_interactions::snapshot(), guest);
                    // PlayerList.placeNewPlayer remembers the joined identity on either server.
                    if let (Some(cache), Some(g)) =
                        (self.server.user_cache.as_mut(), guest.filter(|g| g.placed))
                    {
                        cache.add(sunrise_server_files::user_lists::NameAndId {
                            id: g.uuid,
                            name: g.name.clone(),
                        });
                    }
                    if self.common.is_dedicated_server()
                        && let Some(g) = guest
                    {
                        super::dedicated::events::record(
                            "join",
                            &g.name,
                            g.uuid,
                            serde_json::json!({ "address": g.ip_address() }),
                        );
                        let [x, y, z] = g.position;
                        super::dedicated::log(
                            "INFO",
                            &format!(
                                "{}[/{}] logged in with entity id {} at ({}, {}, {})",
                                g.name,
                                g.address(),
                                g.entity_id,
                                sunrise_core::java_number::double_to_string(x),
                                sunrise_core::java_number::double_to_string(y),
                                sunrise_core::java_number::double_to_string(z)
                            ),
                        );
                        // A held player is not announced: it is not in the world yet.
                        if g.held {
                            super::dedicated::log(
                                "INFO",
                                &sunrise_sim::lan::arrivals::held_line(&g.name),
                            );
                        } else if let Some(old) = renamed {
                            let line = Text::translate(
                                "multiplayer.player.joined.renamed",
                                &[Text::literal(g.name.clone()), Text::literal(old)],
                            );
                            super::dedicated::log(
                                "INFO",
                                &format!("System chat: {}", line.plain()),
                            );
                        } else {
                            super::dedicated::log(
                                "INFO",
                                &format!("System chat: {} joined the game", g.name),
                            );
                        }
                    }
                    if let Some((uuid, name)) = self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .filter(|g| g.placed)
                        .map(|g| (g.uuid, g.name.clone()))
                    {
                        self.announce_login(uuid, &name);
                    }
                }
                HostEvent::Left(mut guest) => {
                    if !guest.participates_online() {
                        let mut host = self.server.lan.take()?;
                        self.cleanup_fake_player(&mut host, &guest);
                        self.server.lan = Some(host);
                        if self.announce_guest_left(&guest) {
                            self.native_player_logged_out(guest.uuid, &guest.name);
                        }
                        continue;
                    }
                    let tell_logout = self.announce_guest_left(&guest);
                    // `ServerPlayer.disconnect` ejects passengers and wakes a sleeping player,
                    // then `PlayerList.remove` posts logout at entry (player still listed in
                    // join order so listener edits are saved), then
                    // `ServerLevel.removePlayerImmediately` -> `Player.remove` closes the menu.
                    if guest.placed {
                        let back = self.common.dim;
                        let theirs = super::Dim::from_id(&guest.dimension);
                        if let Err(error) = self.enter_level(theirs) {
                            log::error!(
                                "Could not remove player {} in {}: {error}",
                                guest.name,
                                theirs.id()
                            );
                            if tell_logout {
                                guest = self.native_departing_guest_logout(guest);
                            }
                            let held = guest.menu.as_ref().map(|menu| menu.window);
                            if let Some(menu) = guest.menu.as_mut()
                                && let Some(closed) = menu.native_removed(
                                    guest.uuid,
                                    guest.menu_generation,
                                    &mut guest.native_menu,
                                    false,
                                )
                            {
                                self.deliver_menu_removed(closed);
                            }
                            // The Open a listener heard gets its matching Close before the
                            // tracked menu is forgotten below.
                            if let Some(window) = held {
                                let generation = guest.menu_generation;
                                guest = self
                                    .native_departing_guest_menu_closed(guest, window, generation);
                            }
                        } else {
                            let (conn, uuid, name, carrier) = (
                                guest.conn,
                                guest.uuid,
                                guest.name.clone(),
                                guest.entity_id as u32,
                            );
                            let asleep = guest.bed.is_some();
                            guest.disconnect_pending = true;
                            let host = self.server.lan.as_mut()?;
                            // Right behind the last of the players listed ahead of them who are still here.
                            let at = host
                                .hub
                                .guests
                                .iter()
                                .rposition(|g| guest.listed_behind.contains(&g.conn))
                                .map_or(0, |i| i + 1);
                            host.hub.guests.insert(at, *guest);
                            host.hub.departing = Some(conn);
                            // A player logging out carries no mob a mod seated on it.
                            self.eject_player_riders(Some(carrier));
                            // ServerPlayer.disconnect: stopSleepInBed(true, false).
                            if asleep && let Some(gi) = self.guest_index(conn) {
                                self.wake_guest(gi, true, false);
                            }
                            if tell_logout {
                                self.native_player_logged_out(uuid, &name);
                            }
                            self.guest_disconnected(conn);
                            let Some(host) = self.server.lan.as_mut() else {
                                unreachable!("LanHost dropped inside a tick; closes are deferred")
                            };
                            host.hub.departing = None;
                            let at =
                                host.hub.guests.connection_index(conn).expect(
                                    "a leaving player stays listed while their menu closes",
                                );
                            *guest = host.hub.guests.remove(at);
                        }
                        if let Err(e) = self.enter_level(back) {
                            log::error!("could not reopen {}: {e}", back.id());
                        }
                    } else if tell_logout {
                        guest = self.native_departing_guest_logout(guest);
                    }
                    self.lan_crafting_left(&mut guest);
                    if guest.joined() {
                        self.award_leave_game(guest.conn, guest.uuid, &guest.name);
                    }
                    // PlayerList.remove: the vehicle it alone rides goes too.
                    self.take_parent_vehicle(&mut guest);
                    self.save_guest(&mut guest);
                    let host = self.server.lan.as_mut()?;
                    host.play.remove(&guest.conn);
                    host.forget_menus(guest.conn);
                    self.clear_guest_book_connection(guest.conn);
                    // PlayerList.remove after the logout: what it tracked stops (removePairing).
                    for e in guest.tracker.take_events() {
                        self.tell_tracking(guest.uuid, &guest.name, &e);
                    }
                }
                HostEvent::CloseInvalidContainer {
                    conn,
                    window,
                    generation,
                } => {
                    self.close_guest_window(conn, window, generation, true);
                }
                HostEvent::Flight { conn, flying } => {
                    self.lan_flight_toggled(conn, flying);
                }
                HostEvent::Sneak { conn, sneaking } => {
                    if !mod_api::__private::listened(mod_api::HookId::PlayerToggleSneak) {
                        continue;
                    }
                    if let Some(g) = self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                    {
                        let (uuid, name) = (g.uuid, g.name.clone());
                        self.as_guest(conn, |game| {
                            game.native_player_toggle_sneak(uuid, &name, sneaking);
                        });
                    }
                }
                HostEvent::Kicked {
                    conn,
                    player,
                    name,
                    reason,
                } => {
                    if !mod_api::__private::listened(mod_api::HookId::PlayerKicked) {
                        continue;
                    }
                    // The same poll can have removed the guest (their `Left`): the mods hear the
                    // kick anyway, without the guest's state swapped in.
                    if self
                        .server
                        .lan
                        .as_ref()
                        .is_some_and(|host| host.hub.guest(conn).is_some())
                    {
                        self.as_guest(conn, |game| {
                            game.native_player_kicked(player, &name, &reason);
                        });
                    } else {
                        self.native_player_kicked(player, &name, &reason);
                    }
                }
                HostEvent::PlayerMove(conn) => self.lan_player_move(conn),
                HostEvent::Input { conn, flags } => {
                    if !mod_api::__private::listened(mod_api::HookId::PlayerInput) {
                        continue;
                    }
                    if let Some(g) = self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                    {
                        let (uuid, name) = (g.uuid, g.name.clone());
                        self.as_guest(conn, |game| {
                            game.native_player_input(uuid, &name, flags);
                        });
                    }
                }
                HostEvent::ClientInformation(conn, old, new) => {
                    if let Some(g) = self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                    {
                        let (uuid, entity_id, name) = (g.uuid, g.entity_id, g.name.clone());
                        // The packet's raw ids become the enums once, here.
                        let old = client_info(&old);
                        let new = client_info(&new);
                        self.native_client_information_updated(uuid, entity_id, &name, &old, &new);
                    }
                }
                HostEvent::PlayerLoaded(conn) => {
                    if let Some(g) = self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                    {
                        let (uuid, name) = (g.uuid, g.name.clone());
                        self.native_player_client_loaded_world(uuid, &name);
                    }
                }
                HostEvent::RecipeBookSettings {
                    conn,
                    book,
                    open,
                    filtering,
                } => {
                    if let Some(g) = self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                    {
                        let (uuid, name) = (g.uuid, g.name.clone());
                        self.notify_vanilla_book_settings(uuid, &name, book, open, filtering);
                    }
                }
                HostEvent::Packet(conn, packet) => {
                    self.lan_guest_packet(conn, *packet);
                    if self
                        .server
                        .lan
                        .as_ref()
                        .is_some_and(|host| !host.commands.is_empty())
                    {
                        self.run_guest_commands();
                    }
                }
                HostEvent::AcceptedMove(conn, movement) => {
                    self.guest_accepted_move(conn, movement);
                }
            }
        }
        if timed && let Some(host) = self.server.lan.as_ref() {
            note_event(host, previous, &mut timings);
            for burst in timings.bursts() {
                sunrise_level::spans::note(burst.ms, || describe_burst(host, &burst));
            }
        }
        self.server.lan.take()
    }

    /// Handle this level's player packets before scheduled block ticks
    /// (`ServerGamePacketListenerImpl` ahead of `ServerLevel`'s block phase).
    /// A sand place in the same tick as another sand's fall schedule still
    /// sees the support (#4510). `tick_lan` then syncs the level; a paused
    /// tick that never calls this still drains inside `tick_lan`.
    pub fn tick_lan_packets(&mut self) {
        if self.server.level_pass.packets_drained {
            return;
        }
        let Some(mut host) = self.server.lan.take() else {
            return;
        };
        // ServerLoginPacketListenerImpl waits for PlayerList.remove, which saves the old
        // guest. Snapshot the new requests before polling play events: the old connection
        // publishes Left before releasing the reservation that admitted its replacement.
        let configuration_requests = self
            .server
            .level_pass
            .first
            .then(|| host.hub.lan.configuration_requests().collect::<Vec<_>>());
        let handoffs = self
            .server
            .level_pass
            .first
            .then(|| host.hub.lan.take_handoffs());
        let login_profiles = self
            .server
            .level_pass
            .first
            .then(|| host.hub.lan.login_profile_requests().collect::<Vec<_>>());
        let holding = lan::Holding::start();
        // Last tick's tracked entities: enough for place collision and joins
        // before this tick moves anyone.
        let entities = std::mem::take(&mut host.states);
        let spawn = self.world_spawn_position();
        let mut solids = std::mem::take(&mut host.hub.move_solids);
        self.fill_move_solids(&mut solids, !host.hub.guests.is_empty());
        host.hub.move_solids = solids;
        let events = {
            let clock_rate = self.network_clock_rate();
            let mut ctx = TickCtx {
                sea_level: self
                    .common
                    .end
                    .locator()
                    .map_or(63, |terrain| terrain.sea_level()),
                world: &mut self.common.world,
                entities: entities.as_slice(),
                next_entity: &mut self.server.next_entity,
                spawn,
                spawn_angle: self.common.level.world_spawn_angle,
                spawn_dimension: sunrise_gameplay_common::game::Dim::of_world_spawn(
                    &self.common.level,
                )
                .id(),
                difficulty: i32::from(self.common.ui.difficulty),
                difficulty_locked: self.common.level.difficulty_locked,
                clock_rate,
                tick_rate: self.common.clock.rate(),
                tick_frozen: self.common.clock.frozen(),
                tick_steps: self.common.clock.step_state(),
                collision: Some(self.common.collision),
                border: &self.common.border,
                streaming: Some(&self.server.streaming),
            };
            host.hub.movement_checks = (
                self.common
                    .ui
                    .rules
                    .bool_at(sunrise_game_data::gamerules::vanilla::PLAYER_MOVEMENT_CHECK),
                self.common
                    .ui
                    .rules
                    .bool_at(sunrise_game_data::gamerules::vanilla::ELYTRA_MOVEMENT_CHECK),
            );
            Self::sync_hub_mod_flags(&mut host.hub);
            let pass = self.server.level_pass;
            let mut events = if pass.first {
                sunrise_level::spans::time("lan_poll", || host.hub.poll(&mut ctx))
            } else {
                Vec::new()
            };
            if pass.first {
                events.extend(
                    host.spawn_joins
                        .drain(..)
                        .map(|(conn, lease)| HostEvent::Joined(conn, Some(lease))),
                );
            }
            events.extend(sunrise_level::spans::time("lan_drain", || {
                host.hub.drain_packets(&mut ctx)
            }));
            events
        };
        let Some(back) = self.drain_raw_turns(host, events, entities.as_slice(), spawn) else {
            return;
        };
        host = back;
        if !host.quiet.is_empty() {
            self.server.lan = Some(host);
            sunrise_level::spans::time("lan_quiet_commands", || self.run_quiet_commands());
            host = self
                .server
                .lan
                .take()
                .expect("the hub is back after the quiet commands");
        }
        if !host.hub.native_advancement_grants.is_empty() {
            self.server.lan = Some(host);
            sunrise_level::spans::time("lan_advancement_grants", || {
                self.run_native_advancement_grants()
            });
            host = self
                .server
                .lan
                .take()
                .expect("the hub is back after the advancement grants");
        }
        if !host.hub.native_menu_events.is_empty() {
            self.server.lan = Some(host);
            sunrise_level::spans::time("lan_menu_events", || self.run_native_menu_events());
            host = self
                .server
                .lan
                .take()
                .expect("the hub is back after the menu events");
        }
        host.states = entities;
        self.server.level_pass.packets_drained = true;
        drop(holding);
        self.server.lan = Some(host);
        if let Some(handoffs) = handoffs {
            self.tick_lan_player_handshakes(handoffs);
        }
        for request in login_profiles.into_iter().flatten() {
            self.answer_login_profile(request);
        }
        if let Some(requests) = configuration_requests {
            sunrise_level::spans::time("lan_configuration_tasks", || {
                self.tick_lan_configuration_tasks(requests)
            });
        }
    }

    /// `player_login_profile` for one admitted login: the mods' list when the packet can carry
    /// it, else the login's own, sent back to its connection.
    pub fn answer_login_profile(&mut self, request: lan::login_profile::LoginProfileRequest) {
        use sunrise_core::profile::AccountProperty;
        let lan::login_profile::LoginProfileRequest {
            uuid,
            name,
            properties,
            reply,
        } = request;
        let mut edited: Vec<_> = properties
            .iter()
            .map(|property| mod_api::AccountProfileProperty {
                name: property.name.clone(),
                value: property.value.clone(),
                signature: property.signature.clone(),
            })
            .collect();
        self.native_player_login_profile(uuid, &name, &mut edited);
        let edited: Vec<_> = edited
            .into_iter()
            .map(|property| AccountProperty {
                name: property.name,
                value: property.value,
                signature: property.signature,
            })
            .collect();
        let chosen = if lan::login_profile::properties_fit(&edited) {
            edited
        } else {
            log::warn!(
                "player_login_profile: the properties for {name} don't fit the login packet; \
                 keeping the login's own"
            );
            properties
        };
        // A connection that closed meanwhile no longer waits.
        let _ = reply.send(chosen);
    }

    pub(in crate::game::server_side) fn tick_lan_player_handshakes(
        &mut self,
        handoffs: Vec<lan::Handoff>,
    ) {
        for mut handoff in handoffs {
            let peer_ip = handoff.peer_ip.map(|ip| ip.to_string());
            let mut cancel = false;
            let mut reason = None;
            self.native_player_handshake(
                &mut handoff.intention.host,
                handoff.intention.port,
                handoff.intention.protocol,
                peer_ip.as_deref(),
                &mut cancel,
                &mut reason,
            );
            let refusal = cancel.then(|| {
                // The reason is checked once more where it becomes the packet's JSON.
                let reason = reason.and_then(|text| {
                    let json = sunrise_sim::api_text::to_serialized_engine(&text)
                        .map_err(|error| error.to_string())
                        .and_then(|text| {
                            lan::conn::login_disconnect_json(&text.component_view())
                                .ok_or_else(|| "too long for a login disconnect".to_owned())
                        });
                    json.inspect_err(|error| {
                        log::warn!("player_handshake disconnect reason refused: {error}");
                    })
                    .ok()
                });
                reason.unwrap_or_else(|| {
                    lan::conn::component_json(&sunrise_protocol::text::translatable(
                        "multiplayer.disconnect.generic",
                        &[],
                    ))
                })
            });
            handoff.answer(refusal);
        }
    }

    /// The entity id counter, once the entities Sunrise keeps without one
    /// have theirs, while guests track the level (None otherwise).
    pub(in crate::game::server_side) fn lan_ids_so_far(&mut self) -> Option<i32> {
        if self
            .server
            .lan
            .as_ref()
            .is_none_or(|h| h.hub.guests.is_empty())
        {
            return None;
        }
        self.lan_assign_ids();
        Some(self.server.next_entity as i32)
    }

    /// ServerLevel.tick's ChunkMap.tick point, after the block ticks and
    /// before the entities': what joined the level up to here (a falling
    /// block, a dispensed arrow, the last tick's late joins) is paired and
    /// first updated in this tick's pass (`Entities::synced`).
    /// `ServerChunkCache.tick`'s broadcast, where ServerLevel.tick runs it:
    /// after the scheduled and random ticks, before the block events, so the
    /// players get the blocks as they are then. What the tick writes from
    /// there on goes out with the next tick's.
    pub fn lan_broadcast_changes(&mut self) -> Option<usize> {
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.broadcast_changes(&mut self.common.world);
        }
        self.common.world.block_log.as_ref().map(Vec::len)
    }

    pub(in crate::game::server_side) fn lan_mark_synced(&mut self) {
        if let Some(next) = self.lan_ids_so_far()
            && let Some(host) = self.server.lan.as_mut()
        {
            host.hub.synced = vec![(i32::MIN, next)];
        }
    }

    /// The end of such a phase: what joined during it is first updated in
    /// this tick's pass too.
    pub(in crate::game::server_side) fn lan_synced_since(&mut self, from: Option<i32>) {
        if let Some(from) = from
            && let Some(next) = self.lan_ids_so_far()
            && next > from
            && let Some(host) = self.server.lan.as_mut()
        {
            host.hub.synced.push((from, next));
        }
    }

    /// The end of the level's tick: the entities added since its states were
    /// taken for the guests (a falling block's drop, a spawner's mob, the
    /// host's snowball) are paired without another tracking tick
    /// (`Hub::note_joined`). Only they are
    /// described (their ids are above the counter the states were taken at),
    /// and nothing when the counter has not moved or no guest is in the level.
    pub(in crate::game::server_side) fn lan_note_joined(&mut self) {
        if self.server.lan.is_none() {
            return;
        }
        self.lan_flush_tracking();
        if let Some(host) = self.server.lan.as_mut() {
            host.states_next_entity = None;
        }
        self.flush_attachment_sync();
    }

    /// Deliver start/stop callbacks before sounds and attachment snapshots.
    /// A callback can spawn another entity, whose pairing is part of the same
    /// operation, as a nested ChunkMap.addEntity would be in vanilla.
    pub(in crate::game::server_side) fn lan_flush_tracking(&mut self) {
        loop {
            self.lan_pair_joined();
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            host.hub.collect_tracking();
            if host.hub.native_tracking.is_empty() {
                return;
            }
            self.run_native_tracking();
        }
    }

    /// Pair late arrivals before a sound flush as well as at the end of the
    /// tick. The counter prevents taking or draining an entity's state twice.
    pub(in crate::game::server_side) fn lan_pair_joined(&mut self) {
        let Some(next) = self
            .server
            .lan
            .as_ref()
            .filter(|host| host.hub.guests.iter().any(|g| host.hub.watches(g)))
            .and_then(|host| host.states_next_entity)
        else {
            return;
        };
        self.lan_assign_ids();
        if self.server.next_entity != next {
            let mut states = States::default();
            self.lan_entities(&mut states, false, next, None);
            self.lan_hook_states(&mut states);
            self.lan_entity_uuids(&mut states);
            if let Some(host) = self.server.lan.as_mut() {
                host.hub
                    .note_joined(states.as_slice(), Some(&*self.common.entity_attachments));
            }
        }
        if let Some(host) = self.server.lan.as_mut() {
            host.states_next_entity = Some(self.server.next_entity);
        }
    }

    /// Spend the current level's forced marks after its tracker snapshot.
    pub fn spend_forced_meta(&mut self) {
        self.common.persist.forced_meta.clear();
    }
    pub fn tick_lan(&mut self, paused: bool) {
        // A frozen level does not reach tick_lan_packets through tick_world.
        // Apply its packets before taking the snapshot too.
        self.tick_lan_packets();
        if self.server.lan.is_none() {
            // No one else to tell: the host's `broadcastToAdmins` lines go.
            self.server.admin_feedback.clear();
            if self.server.level_pass.running.is_none() {
                self.server.level_pass.packets_drained = false;
            }
            return;
        }
        // What the tick sends a guest goes to its connection at the end.
        let holding = lan::Holding::start();
        self.register_native_commands();
        // The host's own commands' (and command blocks') players moving
        // to another level, and their broadcastToAdmins lines.
        if !self.server.level_teleports.is_empty() {
            self.run_level_teleports();
        }
        if !self.server.admin_feedback.is_empty() {
            self.send_admin_feedback();
        }
        // An animal whose rider left the game (or the level) stops taking
        // its input.
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        for m in &mut self.common.mobs {
            if m.rider.is_some()
                && self.common.riding != Some(m.id)
                && !host.hub.guests.iter().any(|g| g.vehicle == Some(m.id))
            {
                m.rider = None;
            }
        }
        // A guest whose file names the vehicle it rode rides it again.
        self.restore_guest_vehicles();
        // Keep chunk and entity publication in the level pass. A real server
        // then runs each connection's player body after all levels; a standalone
        // level tick (e.g. a fixture) has no outer server pass.
        let defer_connections = self.server.level_pass.running.is_some();
        if !paused {
            sunrise_level::spans::time("lan_guests", || self.tick_level_guests());
            if !defer_connections {
                self.tick_guest_connections(false)
                    .expect("the current-level connection pass does not switch levels");
            }
            // Ridden vehicles enter their portals on the level path.
            let Some(host) = self.server.lan.as_ref() else {
                return;
            };
            let ridden: Vec<u32> = host.hub.guests.iter().filter_map(|g| g.vehicle).collect();
            self.find_ridden_vehicle_portals(&ridden);
        }
        // LevelChunk.registerAllBlockEntitiesAfterLevelLoad publishes saved data before
        // the first send, not on the first entity tick (which may be frozen or later).
        if self.common.is_logical_server() {
            self.fire_pending_block_entity_loads();
        }
        // A guest's mount fired its `started_riding` with that guest alone in
        // place (the hub held by its action): now every player on the vehicle
        // hears it (`Entity.startRiding`).
        if !self.server.started_riding.is_empty() {
            self.flush_started_riding();
        }
        {
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            let has_guests = !host.hub.guests.is_empty();
            let mut states = std::mem::take(&mut host.states);
            if has_guests {
                self.lan_assign_ids();
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                let mut guests: Vec<(i64, u32, i32)> = host
                    .hub
                    .guests
                    .iter()
                    .filter_map(|g| Some((g.vehicle_order, g.vehicle?, g.entity_id)))
                    .collect();
                guests.sort_by_key(|&(order, ..)| order);
                let guests: Vec<(u32, i32)> = guests
                    .into_iter()
                    .map(|(_, vehicle, id)| (vehicle, id))
                    .collect();
                let mut riding = std::mem::take(&mut host.hub.riding);
                self.lan_riding(&guests, &mut riding);
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                host.hub.riding = riding;
                // Idle states are kept only when the players' moves are in:
                // the clients that could pair are known. The range scale is
                // the one the tracker gets below.
                let mut idle_states = std::mem::take(&mut host.idle_states);
                let mut kept = if self.server.level_pass.packets_drained {
                    let percent = if host.hub.config.integrated {
                        let scale = self.server.serving.as_ref().map_or(
                            self.server.settings.entity_distance,
                            super::integrated::Serving::entity_distance,
                        );
                        (scale * 100.0).round() as i32
                    } else {
                        host.hub.entity_range_percent
                    };
                    idle_states.pass(&host.hub, percent)
                } else {
                    None
                };
                sunrise_level::spans::time("lan_entity_states", || {
                    self.lan_entities(&mut states, true, 0, kept.as_mut());
                    if let Some(kept) = kept {
                        kept.finish();
                    }
                    self.lan_hook_states(&mut states);
                    self.lan_entity_uuids(&mut states);
                });
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                host.idle_states = idle_states;
                host.states_next_entity = Some(self.server.next_entity);
            } else {
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                host.states_next_entity = None;
                states.clear();
                // Nobody can receive past movement. A returning viewer pairs
                // from the cart's current position/rotation, as AddEntity does.
                self.common
                    .carts
                    .for_each_mut(|cart| cart.lerp_steps.clear());
            }
            // Persist is swapped per level, so parked dimensions retain their marks.
            self.spend_forced_meta();
            self.lan_syncs_looked();

            // The step command reaches clients before the events its tick
            // generated, as ServerTickRateManager.updateStepTicks does.
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            host.hub.sync_ticking(
                self.common.clock.rate(),
                self.common.clock.frozen(),
                self.common.clock.step_state(),
            );
            // `runBlockEvents`' broadcast precedes what the events change (held
            // for the next tick, `tick_world`), and follows the blocks changed
            // before them (`lan_broadcast_changes`): the guest re-runs a
            // piston's `triggerEvent` on the blocks as they were before the
            // move, a base the last tick restored included.
            for (pos, b0, b1, state) in std::mem::take(&mut self.server.lan_block_events) {
                host.hub.block_event(pos, b0, b1, state);
            }
            let entities = states.as_slice();
            self.lan_prepare_captured_sounds(entities);
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            // An entity that left the level since the last tick (a snowball
            // that hit) is dropped by the tracker below: its event goes out
            // first, as `Level.broadcastEntityEvent` precedes the discard.
            let (gone, later) = partition_lan_entity_events(
                std::mem::take(&mut self.server.lan_entity_events),
                entities.iter().map(|entity| entity.id),
            );
            self.server.lan_entity_events = later;
            for (id, event) in gone {
                host.hub.entity_event(id, event);
            }
            let spawn = self.world_spawn_position();
            let mut solids = self
                .server
                .lan
                .as_mut()
                .map(|host| std::mem::take(&mut host.hub.move_solids))
                .unwrap_or_default();
            let guests = self
                .server
                .lan
                .as_ref()
                .is_some_and(|host| !host.hub.guests.is_empty());
            self.fill_move_solids(&mut solids, guests);
            let (events, last) = {
                let clock_rate = self.network_clock_rate();
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                host.hub.move_solids = solids;
                let mut ctx = TickCtx {
                    sea_level: self
                        .common
                        .end
                        .locator()
                        .map_or(63, |terrain| terrain.sea_level()),
                    world: &mut self.common.world,
                    entities,
                    next_entity: &mut self.server.next_entity,
                    spawn,
                    spawn_angle: self.common.level.world_spawn_angle,
                    spawn_dimension: sunrise_gameplay_common::game::Dim::of_world_spawn(
                        &self.common.level,
                    )
                    .id(),
                    difficulty: i32::from(self.common.ui.difficulty),
                    difficulty_locked: self.common.level.difficulty_locked,
                    clock_rate,
                    tick_rate: self.common.clock.rate(),
                    tick_frozen: self.common.clock.frozen(),
                    tick_steps: self.common.clock.step_state(),
                    collision: Some(self.common.collision),
                    border: &self.common.border,
                    streaming: Some(&self.server.streaming),
                };
                host.hub.movement_checks = (
                    self.common
                        .ui
                        .rules
                        .bool_at(sunrise_game_data::gamerules::vanilla::PLAYER_MOVEMENT_CHECK),
                    self.common
                        .ui
                        .rules
                        .bool_at(sunrise_game_data::gamerules::vanilla::ELYTRA_MOVEMENT_CHECK),
                );
                // IntegratedServer.getScaledTrackingDistance: the Entity Distance
                // option scales what an integrated server or a LAN host tracks (a
                // dedicated one keeps its entity-broadcast-range-percentage).
                if host.hub.config.integrated {
                    let scale = self.server.serving.as_ref().map_or(
                        self.server.settings.entity_distance,
                        super::integrated::Serving::entity_distance,
                    );
                    host.hub.entity_range_percent = (scale * 100.0).round() as i32;
                }
                host.hub.set_immediate_respawn(
                    self.common
                        .ui
                        .rules
                        .bool_at(sunrise_game_data::gamerules::vanilla::IMMEDIATE_RESPAWN),
                );
                host.hub.set_limited_crafting(
                    self.common
                        .ui
                        .rules
                        .bool_at(sunrise_game_data::gamerules::vanilla::LIMITED_CRAFTING),
                );
                // A server with several levels runs a pass for each
                // (`tick_server`): the network is read once, before the first.
                // Player packets may already have been drained before
                // scheduled block ticks (`tick_lan_packets`).
                Self::sync_hub_mod_flags(&mut host.hub);
                let pass = self.server.level_pass;
                // Even paused worlds completed their ordered input segments at
                // entry, before world sync/physics or any scheduled work.
                let mut events = Vec::new();
                events.extend(sunrise_level::spans::time("lan_tick_level", || {
                    if defer_connections {
                        host.hub.tick_level_before_connections(
                            &mut ctx,
                            Some(&*self.common.entity_attachments),
                        )
                    } else {
                        host.hub
                            .tick_level(&mut ctx, Some(&*self.common.entity_attachments))
                    }
                }));
                (events, pass.last && !defer_connections)
            };
            // The mods hear the pass's starts and stops of tracking at each one's packet, before
            // the players' own state and the end of the tick (#4522).
            self.run_native_tracking();
            if let Some(host) = self.server.lan.as_mut() {
                host.hub.finish_tracking_pass();
                if last {
                    sunrise_level::spans::time("lan_end_tick", || host.hub.end_tick());
                }
            }
            self.wake_woken_guests();
            let Some(host) = self.server.lan.take() else {
                return;
            };
            let processed = sunrise_level::spans::time("lan_process_events", || {
                self.lan_process_events(host, events, entities, spawn)
            });
            let Some(back) = processed else {
                return;
            };
            self.server.lan = Some(back);
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            host.states = states;
        }
        if self
            .server
            .lan
            .as_ref()
            .is_some_and(|host| !host.quiet.is_empty())
        {
            self.run_quiet_commands();
        }
        if self
            .server
            .lan
            .as_ref()
            .is_some_and(|host| !host.hub.native_advancement_grants.is_empty())
        {
            self.run_native_advancement_grants();
        }
        if self
            .server
            .lan
            .as_ref()
            .is_some_and(|host| !host.hub.native_menu_events.is_empty())
        {
            self.run_native_menu_events();
        }
        self.run_chunk_watches();
        self.flush_passages();
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.collect_tracking();
        }
        if self
            .server
            .lan
            .as_ref()
            .is_some_and(|host| !host.hub.native_tracking.is_empty())
        {
            self.run_native_tracking();
        }
        self.flush_guest_connection_effects();
        self.lan_stuck(paused);
        self.tick_server_travel();
        self.lan_swings();
        self.lan_update_streaming();
        self.lan_guest_poses();
        self.lan_update_waypoints();
        self.lan_sync_scoreboard();
        drop(holding);
        self.flush_attachment_sync();
        self.flush_far_terrain();
        if let Some(host) = self.server.lan.as_ref() {
            lan::flush_all(host.hub.guests.iter().map(|g| &g.sender));
        }
        self.flush_native_sounds();
        // `tick_server` resets the pass after all dimensions; outside a pass
        // the packet gate is per-tick.
        if self.server.level_pass.running.is_none() {
            self.server.level_pass.packets_drained = false;
        }
    }

    /// Flush actions made by guest turns while their level is still in place.
    pub(in crate::game) fn flush_guest_connection_effects(&mut self) {
        self.flush_lan_explosions();
        let _ = self.flush_wind_bursts();
        self.flush_sweeps();
        self.flush_hook_pulls();
        self.flush_lan_particles();
        self.lan_flush_effects();
        self.lan_living_flags();
    }

    pub fn lan_guest_joined(
        &mut self,
        conn: u32,
        entities: &[EntityState],
        spawn: [f64; 3],
        prepared: Option<PreparedSpawn>,
    ) {
        let Some(uuid) = self.command_guest(conn).map(|guest| guest.uuid) else {
            return;
        };
        let loaded = self.loaded_level_order();
        let spawn_level = self.world_spawn_level();
        let spawn_level = if loaded.contains(&spawn_level) {
            spawn_level
        } else {
            super::Dim::Overworld
        };
        let back = self.common.dim;
        // `PrepareSpawnTask.Ready.spawn` waits for saved entities before the player loads.
        // Streaming commits terrain and the corresponding entity records together.
        if let Some(prepared) = prepared {
            let level = super::Dim::from_id(prepared.level.id);
            if let Err(e) = self.enter_level(level) {
                log::error!("could not open {}: {e}", level.id());
                return;
            }
            self.server.tickets.0.add_spawn(
                prepared.chunk(),
                sunrise_sim::ticket_levels::SpawnTicketKind::Player,
            );
            self.spawn_loaded_entities();
            self.common.world.entities_pending.clear();
            if let Err(e) = self.enter_level(back) {
                log::error!("could not reopen {}: {e}", back.id());
                return;
            }
        }
        self.initialize_server_player(uuid, conn);
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let default_mode = host.hub.config.default_game_mode;
        let force = host.hub.config.force_game_mode;
        let root = self.server.world_root.clone();
        let in_place = host.hub.config.dimension.clone();
        let owner = host.owner.as_mut();
        let Some((known, name)) = host.hub.update_guest(conn, |g| {
            // PlayerList.loadPlayerData: the owner gets the world's player file,
            // everyone else their own.
            let known = match owner.filter(|o| o.uuid == g.uuid) {
                Some(o) => playerdata::load_owner(g, &root, o.saved.take().as_deref()),
                None => playerdata::load(g, &root),
            };
            // `PrepareSpawnTask.start`: a saved level the world no longer has is the world
            // spawn's (the Overworld when that one is gone too); the saved position and
            // rotation stay.
            if !loaded.iter().any(|dim| dim.id() == g.dimension) {
                g.dimension = spawn_level.id().to_owned();
            }
            // PlayerEvent.LoadFromFile: the player's data is read and applied,
            // the player not yet placed.
            (known, g.name.clone())
        }) else {
            return;
        };
        self.native_player_load_from_file(uuid, &name, &playerdata::directory(&root));
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        if host
            .hub
            .update_guest(conn, |g| {
                // Ready.spawn loads the player data again, but snaps to the prepared place.
                if let Some(prepared) = prepared {
                    g.dimension = prepared.level.id.to_owned();
                    g.place(prepared.position);
                    [g.yaw, g.pitch] = prepared.rotation;
                    g.head_yaw = g.yaw;
                } else if !known {
                    g.dimension = spawn_level.id().to_owned();
                    playerdata::face_world_spawn(g, false, self.common.level.world_spawn_angle);
                }
                if !known || force {
                    g.game_mode = default_mode;
                    // ServerPlayerGameMode.setGameModeForPlayer: the abilities of the mode chosen.
                    g.abilities.update_for_mode(default_mode);
                }
            })
            .is_none()
        {
            return;
        }
        // A server places each player in their own level (`PlayerList.placeNewPlayer`).
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let back = self.common.dim;
        let Some((level, mut elsewhere)) = host.hub.update_guest(conn, |g| {
            let level = super::Dim::from_id(&g.dimension);
            let elsewhere = level != back;
            if !elsewhere {
                g.dimension = in_place;
            }
            (level, elsewhere)
        }) else {
            return;
        };
        self.load_guest_records(conn);
        // Collect the recipient's recipe types and filter their command tree.
        self.prepare_recipe_join(conn);
        // A level that will not open, its generator gone with a mod, is one the
        // world no longer has: left in it, the player would fail the next tick.
        // It goes to the world spawn's, as one missing from the world does.
        if elsewhere && let Err(e) = self.enter_level(level) {
            log::error!("could not open {}: {e}", level.id());
            let fallback = if spawn_level == level {
                super::Dim::Overworld
            } else {
                spawn_level
            };
            let moved = self.server.lan.as_mut().and_then(|host| {
                host.hub
                    .update_guest(conn, |g| g.dimension = fallback.id().to_owned())
            });
            elsewhere = back != fallback;
            if moved.is_none() || (elsewhere && self.enter_level(fallback).is_err()) {
                return;
            }
        }
        let mut theirs = States::default();
        let entities = if elsewhere {
            // A dedicated server has no host player.
            self.lan_entities(&mut theirs, false, 0, None);
            self.lan_hook_states(&mut theirs);
            self.lan_entity_uuids(&mut theirs);
            theirs.as_slice()
        } else {
            entities
        };
        macro_rules! join_ctx {
            () => {{
                let clock_rate = self.network_clock_rate();
                TickCtx {
                    sea_level: self
                        .common
                        .end
                        .locator()
                        .map_or(63, |terrain| terrain.sea_level()),
                    world: &mut self.common.world,
                    entities,
                    next_entity: &mut self.server.next_entity,
                    spawn,
                    spawn_angle: self.common.level.world_spawn_angle,
                    spawn_dimension: sunrise_gameplay_common::game::Dim::of_world_spawn(
                        &self.common.level,
                    )
                    .id(),
                    difficulty: i32::from(self.common.ui.difficulty),
                    difficulty_locked: self.common.level.difficulty_locked,
                    clock_rate,
                    tick_rate: self.common.clock.rate(),
                    tick_frozen: self.common.clock.frozen(),
                    tick_steps: self.common.clock.step_state(),
                    collision: Some(self.common.collision),
                    border: &self.common.border,
                    streaming: Some(&self.server.streaming),
                }
            }};
        }
        let joining = {
            let ctx = join_ctx!();
            self.server
                .lan
                .as_mut()
                .and_then(|host| host.hub.begin_player_join(conn, &ctx))
                .is_some()
        };
        if joining {
            self.send_player_command_tree(conn);
            let ctx = join_ctx!();
            if let Some(host) = self.server.lan.as_mut()
                && let Some(i) = host.hub.guests.connection_index(conn)
            {
                host.hub.finish_player_join(i, &ctx);
            }
        }
        self.bind_player_attachments(conn, false);
        self.lan_register_player_waypoint(conn);
        self.lan_update_waypoints();
        if elsewhere && let Err(e) = self.enter_level(back) {
            log::error!("could not reopen {}: {e}", back.id());
        }
    }

    /// Level events the game queued, for the guests (`process_level_events`).
    pub fn lan_record_events(&mut self, events: &[sunrise_world::level::LevelEvent]) {
        if !self.server.audio_capture_active {
            return;
        }
        for e in events {
            match e {
                sunrise_world::level::LevelEvent::LevelEvent {
                    id,
                    pos,
                    data,
                    source,
                } => {
                    self.server.lan_events.push(super::QueuedLevelEvent {
                        kind: *id,
                        pos: [pos.x, pos.y, pos.z],
                        data: *data,
                        source: *source,
                    });
                }
                // `ServerLevel.runBlockEvents` broadcasts a handled block
                // event (`ClientboundBlockEventPacket`): the guest re-runs
                // the block's `triggerEvent` — a piston's `moveBlocks` is
                // what animates the move there. `state` is the block the
                // event named, read before it ran (the packet's `block`).
                sunrise_world::level::LevelEvent::BlockEntityEvent { pos, state, a, b } => {
                    self.server
                        .lan_block_events
                        .push(([pos.x, pos.y, pos.z], *a, *b, state.0));
                }
                _ => {}
            }
        }
    }

    /// `Level.blockEvent` for the guests (`ClientboundBlockEventPacket`).
    pub(in crate::game) fn lan_block_event(&mut self, pos: [i32; 3], b0: i32, b1: i32) {
        if self.server.audio_capture_active {
            let state = self.common.world.block(pos[0], pos[1], pos[2]).0;
            self.server.lan_block_events.push((pos, b0, b1, state));
        }
    }
    /// `Level.broadcastEntityEvent` for the guests.
    pub(in crate::game) fn lan_entity_event(&mut self, id: i32, event: i8, _at: [f64; 3]) {
        if self.server.audio_capture_active {
            self.server.lan_entity_events.push((id, event));
        }
    }
    /// Deliver one committed entity event immediately to tracking guests and the entity itself,
    /// without touching unrelated entries in `self.lan_entity_events`.
    pub(in crate::game::server_side) fn deliver_entity_event(
        &mut self,
        id: i32,
        event: i8,
        _at: [f64; 3],
    ) {
        if let Some(host) = self.server.lan.as_ref() {
            host.hub.entity_event(id, event);
        } else if self.server.audio_capture_active {
            self.server.lan_entity_events.push((id, event));
        }
    }
    /// `ServerPlayer.onEquippedItemBroken` for the acting player (the guest whose turn it
    /// is, else the host): the broken statistic, then `level.broadcastEntityEvent` of the
    /// slot's break event, which each client answers with `breakItem` (the sound and the
    /// crumbs, `remote_equipment_break`).
    pub fn player_equipment_broke(
        &mut self,
        slot: sunrise_core::equipment_slot::EquipmentSlot,
        stack: &sunrise_items::items::ItemStack,
    ) {
        use sunrise_core::equipment_slot::EquipmentSlot;
        self.award_stat("broken", &stack.item, 1);
        let event = match slot {
            EquipmentSlot::MainHand => 47,
            EquipmentSlot::OffHand => 48,
            EquipmentSlot::Head => 49,
            EquipmentSlot::Chest => 50,
            EquipmentSlot::Legs => 51,
            EquipmentSlot::Feet => 52,
            EquipmentSlot::Body => 65,
            EquipmentSlot::Saddle => 68,
        };
        if self.common.player.is_none() {
            return;
        }
        if self.server.lan_acting.is_some() {
            self.server.guest_equipment_breaks.push(event);
        }
    }
    /// `ServerPlayer.crit` (or `magicCrit` when `magic`): queues an animation
    /// for the attacker's client and observers once the hub is back in place.
    pub(in crate::game::server_side) fn critical_hit(
        &mut self,
        attacker: Option<u32>,
        id: i32,
        _feet: [f64; 3],
        _width: f64,
        _height: f64,
        magic: bool,
    ) {
        if self.server.audio_capture_active {
            self.server.lan_critical_hits.push((attacker, id, magic));
        }
    }

    /// Send the tick's level events, sounds and mob animations.
    pub fn lan_flush_effects(&mut self) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let critical_hits = std::mem::take(&mut self.server.lan_critical_hits);
        let kinetic_hits = std::mem::take(&mut self.server.lan_kinetic_hits);
        for (state, at) in std::mem::take(&mut self.server.lan_cushion_breaks) {
            if let Some(id) = host
                .hub
                .registry_id("minecraft:particle_type", "minecraft:block")
            {
                host.hub.broadcast_near(
                    at,
                    32.0,
                    &C::LevelParticles(play::LevelParticles {
                        particle: sunrise_protocol::entity_data::Particle {
                            id,
                            data: Some(sunrise_protocol::entity_data::ParticleData::State(
                                state.0 as i32,
                            )),
                        },
                        override_limiter: false,
                        always_show: false,
                        pos: at,
                        dist: [0.25, 0.0625, 0.25],
                        max_speed: [0.05; 3],
                        count: 10,
                        randomization: 0,
                    }),
                    None,
                );
            }
        }
        for super::QueuedLevelEvent {
            kind,
            pos,
            data,
            source,
        } in std::mem::take(&mut self.server.lan_events)
        {
            if kind == 2001 {
                host.broken
                    .push([f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])]);
            }
            let except = source.and_then(|player| {
                host.hub
                    .guests
                    .iter()
                    .find(|guest| guest.uuid == player.0)
                    .map(|guest| guest.conn)
            });
            host.hub.level_event(kind, pos, data, except);
        }
        // Level events 1010 and 1011: the guests' parrots dance or stop.
        for (pos, song) in std::mem::take(&mut self.server.lan_songs) {
            match song {
                Some(name) => {
                    let id = host.hub.registry_id("minecraft:jukebox_song", &name);
                    host.hub.level_event(1010, pos, id.unwrap_or(0), None);
                }
                None => host.hub.level_event(1011, pos, 0, None),
            }
        }
        for (pos, b0, b1, state) in std::mem::take(&mut self.server.lan_block_events) {
            host.hub.block_event(pos, b0, b1, state);
        }
        for (id, event) in std::mem::take(&mut self.server.lan_entity_events) {
            host.hub.entity_event(id, event);
        }
        for (level, kind, pos, data) in std::mem::take(&mut self.server.lan_global_events) {
            host.hub.global_level_event(level, kind, pos, data);
        }
        self.lan_broadcast_effects();
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        if host.hub.guests.is_empty() {
            host.mob_state.clear();
            return;
        }
        // Hurt and death animations of mobs.
        let mut seen = HashMap::new();
        for m in &self.common.mobs {
            let dead = !m.alive();
            let old = host
                .mob_state
                .get(&self.common.dim)
                .and_then(|s| s.get(&m.id))
                .copied();
            let at = [m.position.x, m.position.y, m.position.z];
            // Villager hits already sent their damage in accepted-hit order.
            // Other mobs still use the restarted hurt time at the normal flush.
            if m.species() != Species::Villager
                && m.hurt_time > old.map_or(0, |o| o.0)
                && !dead
                && let Some((source, _)) = m.last_damage
            {
                use sunrise_sim::mobs::Who;
                let kind = sunrise_gameplay_common::game::creatures::source_damage_type(source);
                // The player a mob sees is the guest acting.
                let cause = source.attacker.and_then(|who| match who {
                    Who::Player => host
                        .mob_players
                        .get(&m.id)
                        .and_then(|conn| host.hub.guest(*conn))
                        .map(|g| g.entity_id),
                    Who::Mob(id) => Some(id as i32),
                });
                // A projectile is the direct entity of its hit: not tracked here.
                let direct = cause.filter(|_| super::combat::melee(source.kind));
                host.hub.damage_event(m.id as i32, kind, cause, direct, at);
            }
            if dead && !old.is_some_and(|o| o.1) {
                host.hub.entity_event(m.id as i32, 3);
            }
            // LivingEntity.swing on a server (a melee hit, a thrown potion):
            // ClientboundSwingAnimationPacket for the players around. A swing
            // started since the last flush, or restarted (its time went back).
            let swing = m.ai.swinging.then_some(m.ai.swing_time);
            if let Some(time) = swing
                && !dead
                && old.and_then(|o| o.2).is_none_or(|before| time < before)
            {
                host.hub
                    .swing(m.id as i32, i32::from(m.swing_is_off_hand()), at, None);
            }
            seen.insert(m.id, (m.hurt_time, dead, swing));
        }
        if let Some(d) = self.common.dragon.as_ref().filter(|d| d.id != 0) {
            let id = d.id;
            let dead = !d.alive();
            let old = host
                .mob_state
                .get(&self.common.dim)
                .and_then(|s| s.get(&id))
                .copied();
            let at = [d.position.x, d.position.y, d.position.z];
            if d.hurt_time > old.map_or(0, |o| o.0) && !dead {
                let (kind, cause, direct) =
                    d.last_source.unwrap_or(("minecraft:generic", None, None));
                host.hub.damage_event(id as i32, kind, cause, direct, at);
            }
            seen.insert(id, (d.hurt_time, dead, None));
        }
        host.mob_state.insert(self.common.dim, seen);
        // Level.broadcastEntityEvent(player, 2) of a spear's stab.
        for attacker in kinetic_hits {
            if let Some(g) = attacker.and_then(|conn| host.hub.guest(conn)) {
                host.hub.entity_event(g.entity_id, 2);
            }
        }
        for (attacker, target, magic) in critical_hits {
            let Some(attacker) =
                attacker.and_then(|conn| host.hub.guest(conn).map(|g| g.entity_id))
            else {
                continue;
            };
            host.hub.critical_hit(attacker, target, magic);
        }
    }

    /// Keep the chunks around every guest loaded.
    pub fn lan_update_streaming(&mut self) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        // DistanceManager.PlayerTicketTracker: every chunk within the server's view distance
        // (Chebyshev) of a player holds a PLAYER_LOADING ticket at the entity-ticking level (31),
        // whatever the player asked for; levels rise by one a chunk, so FULL (33) reaches two
        // chunks past it. An extra area finishes its chunks out to its radius + 1. That covers what
        // the player tracks (`ChunkTrackingView`, within its view distance + 1) and those chunks'
        // lit neighbours. The viewed areas (#3716) are squares of their own.
        let radius =
            sunrise_sim::lan::hub::effective_view_distance(host.hub.config.view_distance) + 1;
        let rule = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::SPECTATORS_GENERATE_CHUNKS);
        // The held players are sent the level, so their chunks are kept as the placed ones'.
        self.server.streaming.extra = host
            .hub
            .loading_chunks(|g| {
                sunrise_gameplay_common::game_modes::loads_chunks(rule, g.spectator())
            })
            .map(|chunk| (chunk, radius))
            .chain(
                self.arrival_chunks(self.common.dim)
                    .into_iter()
                    // An arrival keeps only its landing spot lit enough to resolve (the 3x3
                    // `resolve_arrivals` checks), where it then ticks without a player.
                    .map(|chunk| (chunk, 1)),
            )
            .chain(self.portal_ticket_areas())
            .chain(self.viewed_extra())
            .collect();
    }

    /// The bed at `head` as a sleeping model lies in it (`ints[1]`,
    /// `villager::sleep_pose`): its `facing`.
    /// `Entity.setPose` for the server's players: the mods hear each placed
    /// guest's change of pose. Nothing is kept while no mod listens.
    pub(in crate::game::server_side) fn lan_guest_poses(&mut self) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        if !mod_api::__private::listened(mod_api::HookId::EntityPoseChange) {
            host.poses.clear();
            return;
        }
        host.poses
            .retain(|conn, _| host.hub.guest(*conn).is_some_and(|g| g.placed));
        let mut changed = Vec::new();
        for g in host.hub.guests.iter().filter(|g| g.placed) {
            let to = g.pose();
            let from = host.poses.insert(g.conn, to).unwrap_or_default();
            if from != to {
                changed.push((g.uuid, g.name.clone(), from, to));
            }
        }
        for (uuid, name, from, to) in changed {
            self.native_entity_pose_change(uuid, &name, from.name(), to.name());
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// After a player's chat was broadcast: the dedicated server's event
    /// log, and `detectRateSpam`.
    pub(in crate::game::server_side) fn lan_player_chatted(&mut self, sender: u128, text: &str) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(g) = host.hub.online_players().find(|g| g.uuid == sender) else {
            return;
        };
        if self.common.is_dedicated_server() {
            super::dedicated::events::record(
                "chat",
                &g.name,
                sender,
                serde_json::json!({ "message": text }),
            );
        }
        let conn = g.conn;
        self.detect_rate_spam(conn, false);
    }

    /// `MinecraftServer.logChatMessage`: public chat is logged, not captured as command feedback.
    pub(super) fn record_public_chat(&mut self, line: Text, secure: bool) {
        let mark = if secure { "" } else { "[Not Secure] " };
        if self.common.is_dedicated_server() {
            super::dedicated::log_public_chat(&format!("{mark}{}", line.plain()));
        } else {
            log::info!("{mark}{}", line.plain());
        }
    }
    /// Warnings and errors from the hub's validation and connection paths.
    pub(in crate::game::server_side) fn lan_log_event(&mut self, event: HostEvent) {
        let dedicated = self.common.is_dedicated_server();
        match event {
            HostEvent::Warn(line) if dedicated => super::dedicated::log("WARN", &line),
            HostEvent::Warn(line) => log::warn!("{line}"),
            HostEvent::Error(line) if dedicated => super::dedicated::log("ERROR", &line),
            HostEvent::Error(line) => log::error!("{line}"),
            _ => {}
        }
    }
}

/// `LivingEntity.DATA_LIVING_ENTITY_FLAGS` for an item in use: 1, and 2 when
/// it is the off hand's, and 4 for a Riptide spin (`isAutoSpinAttack`). A stop
/// is 0 here; vanilla leaves bit 1 set, so its stop of an off-hand use is 2.
/// Both mean "not using" (bit 0 is clear), and nothing reads bit 1 without
/// bit 0.
pub(in crate::game) fn living_flags(using: Option<(usize, u32)>, spinning: bool) -> i8 {
    using.map_or(0, |(slot, _)| if slot == OFFHAND_SLOT { 3 } else { 1 })
        | if spinning { 4 } else { 0 }
}

/// Tracker updates go between the two groups. Partition preserves the
/// enqueue order within each group.
pub(in crate::game::server_side) fn partition_lan_entity_events(
    events: Vec<(i32, i8)>,
    mut live_ids: impl Iterator<Item = i32>,
) -> (Vec<(i32, i8)>, Vec<(i32, i8)>) {
    match events.as_slice() {
        [] => (Vec::new(), Vec::new()),
        [(id, _)] if live_ids.any(|live| live == *id) => (Vec::new(), events),
        [_] => (events, Vec::new()),
        _ => {
            let present: sunrise_core::fast_hash::FxHashSet<_> = live_ids.collect();
            events
                .into_iter()
                .partition(|(id, _)| !present.contains(id))
        }
    }
}

#[cfg(test)]
#[test]
fn entity_event_partition_keeps_delivery_order() {
    let live = [1, 3, 5];
    let events = [(2, 17), (1, 18), (2, 19), (5, 20), (4, 21)];
    for batch in [events.as_slice(), &events[..1], &events[1..2], &[]] {
        let expected = batch
            .iter()
            .copied()
            .partition(|(id, _)| !live.contains(id));
        assert_eq!(
            partition_lan_entity_events(batch.to_vec(), live.into_iter()),
            expected
        );
    }
}

#[cfg(test)]
use level_event_targets::level_event_except;

/// A client's settings as the mod API names them.
pub(in crate::game) fn client_info(
    i: &sunrise_protocol::packets::configuration::ClientInformation,
) -> mod_api::ClientInfo {
    mod_api::ClientInfo {
        language: i.language.clone(),
        view_distance: i.view_distance,
        chat_visibility: match i.chat_visibility.rem_euclid(3) {
            0 => mod_api::ChatVisibility::Full,
            1 => mod_api::ChatVisibility::System,
            _ => mod_api::ChatVisibility::Hidden,
        },
        chat_colors: i.chat_colors,
        model_customisation: i.model_customisation,
        main_hand: main_hand(i.main_hand),
        text_filtering: i.text_filtering,
        allows_listing: i.allows_listing,
        particle_status: match i.particle_status.rem_euclid(3) {
            0 => mod_api::ParticleStatus::All,
            1 => mod_api::ParticleStatus::Decreased,
            _ => mod_api::ParticleStatus::Minimal,
        },
    }
}

/// `HumanoidArm.BY_ID`: 1 is the right arm, and every other id the left (0, and the id an
/// unknown one reads as).
pub(in crate::game::server_side) fn main_hand(id: i32) -> mod_api::MainHand {
    if id == 1 {
        mod_api::MainHand::Right
    } else {
        mod_api::MainHand::Left
    }
}

impl crate::game::ServerGame<'_> {
    /// For a slow tick's note (#8669): the item a `UseItemOn` or `UseItem` uses and the block
    /// it aims at. None for other events.
    pub(in crate::game::server_side) fn event_detail(
        &self,
        host: &LanHost,
        event: &HostEvent,
    ) -> Option<UseDetail> {
        let HostEvent::Packet(conn, packet) = event else {
            return None;
        };
        let (hand, block) = match &**packet {
            play::Serverbound::UseItemOn(p) => (p.hand, Some((p.pos, p.face))),
            play::Serverbound::UseItem(p) => (p.hand, None),
            _ => return None,
        };
        let guest = host.hub.guest(*conn)?;
        let held = if hand == 1 {
            guest.offhand.as_ref()
        } else {
            guest.inventory.get(guest.selected).and_then(Option::as_ref)
        };
        Some(UseDetail {
            item: held.map(|s| Arc::clone(&s.item)),
            block: block.map(|(pos, face)| (self.block_name(pos), pos, face)),
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl LanHost {
    /// Install an already configured fixture hub without touching the process-wide sync cache.
    pub fn for_test(mut hub: Hub) -> Self {
        hub.dispatch_move_effects = true;
        Self {
            settings: LanSettings {
                port: hub.lan.port,
                guest_commands: false,
                force_game_mode: false,
            },
            hub,
            players_told_left: false,
            spawn_joins: Vec::new(),
            play: Default::default(),
            menus: Default::default(),
            broken: Default::default(),
            mob_state: Default::default(),
            poses: Default::default(),
            mob_players: Default::default(),
            owner: None,
            waypoints: Default::default(),
            waypoint_dead: Default::default(),
            commands: Default::default(),
            quiet: Default::default(),
            far: None,
            native_commands_sent: None,
            sleep_ignored: Default::default(),
            states: Default::default(),
            states_next_entity: None,
            idle_states: Default::default(),
        }
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::{EventTimings, Hub, LanHost, OFFHAND_SLOT, living_flags};
    use sunrise_gameplay_common::game::lan_mob_metadata::{mob_freeze_meta, mob_meta};
    use sunrise_gameplay_common::game::{
        native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
        tables::{GameItems as _, WorldTablesExt as _},
    };

    #[test]
    #[ignore = "daily"]
    fn player_handshake_dispatch_before_login() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use mod_api::{Flow, Hooks, ModEntry, PlayerHandshakeEvent, ServerHost, Text};
        use serde_json::{Value, json};
        use std::{
            net::{Shutdown, TcpStream},
            sync::{Arc, Mutex, mpsc},
            thread,
            time::{Duration, Instant},
        };
        use sunrise_core::nbt::Tag;
        use sunrise_protocol::{
            PROTOCOL_VERSION,
            connection::Connection,
            crypto::offline_uuid,
            packets::{Empty, handshake, login, status},
        };
        use sunrise_sim::lan::{Lan, NetConfig, chunks::ChunkEncoder, data::SyncData};

        const WAIT: Duration = Duration::from_secs(5);
        const ORIGINAL: &str = "entry.test\0suffix";
        const REWRITTEN: &str = "route.test\0kept";
        if run_fresh_cases(
            "game::server_side::lan::tests::player_handshake_dispatch_before_login",
            "SUNRISE_PLAYER_HANDSHAKE_CHILD",
            &["login"],
        ) {
            return;
        }

        #[derive(Debug, PartialEq, Eq)]
        struct Seen {
            host: String,
            port: u16,
            protocol: i32,
            peer_ip: Option<String>,
            thread: thread::ThreadId,
        }
        struct Listener {
            rewrite: bool,
            seen: Arc<Mutex<Vec<Seen>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn player_handshake(
                &self,
                _: &mut dyn ServerHost,
                event: &mut PlayerHandshakeEvent<'_>,
            ) -> Flow {
                self.seen.lock().unwrap().push(Seen {
                    host: event.requested_host.clone(),
                    port: event.requested_port,
                    protocol: event.protocol_version,
                    peer_ip: event.peer_ip.map(str::to_owned),
                    thread: thread::current().id(),
                });
                if self.rewrite {
                    // The handshake's own values are this listener's copy: the next one
                    // still reads what the connection sent.
                    event.requested_port = 1;
                    event.protocol_version = 0;
                    event.peer_ip = Some("10.0.0.1");
                    match event.requested_host.as_str() {
                        "closed.test" => {
                            event.cancelled = true;
                            *event.reason = Some(Text::literal("Fixture refused"));
                        }
                        "default.test" => event.cancelled = true,
                        "item.test" => {
                            event.cancelled = true;
                            let mut style = mod_api::text::TextStyle::default();
                            style.hover = Some(mod_api::text::TextHover::Item {
                                id: "minecraft:stone".into(),
                                count: 1,
                                components: Some(Tag::compound([(
                                    "minecraft:custom_model_data",
                                    Tag::compound([(
                                        "floats",
                                        Tag::List(vec![Tag::Float(0.5f32.to_bits())]),
                                    )]),
                                )])),
                            });
                            *event.reason = Some(Text::literal("Fixture item").with_style(style));
                        }
                        "long.test" => {
                            event.cancelled = true;
                            // Valid Text, longer than the login disconnect's 262144-unit string.
                            *event.reason = Some(Text::literal("a".repeat(262_145)));
                        }
                        "deep.test" => {
                            event.cancelled = true;
                            let mut text = Text::literal("too deep to serialize");
                            for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
                                text = Text::literal("").append(text);
                            }
                            *event.reason = Some(text);
                        }
                        _ if event.requested_host.starts_with("local") => {
                            *event.requested_host = format!("{REWRITTEN}{}", "a".repeat(255));
                        }
                        _ => *event.requested_host = REWRITTEN.into(),
                    }
                }
                Flow::Continue
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = [("handshake_rewrite", true), ("handshake_observe", false)]
            .into_iter()
            .map(|(id, rewrite)| {
                ModEntry::new(
                    id,
                    Box::new(Listener {
                        rewrite,
                        seen: seen.clone(),
                    }),
                )
            })
            .collect::<Vec<_>>()
            .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let (gate, gate_calls) = mpsc::channel();
                let net = NetConfig {
                    compression_threshold: -1,
                    gate: Some(Arc::new(move |addr, uuid, name| {
                        let _ = gate.send((addr, uuid, name.to_owned()));
                        None
                    })),
                    ..NetConfig::loopback(0, "Handshake fixture".into(), 8)
                };
                let sync = Arc::new(SyncData::empty());
                let lan = Lan::start_with_spawn_preparation(sync.clone(), net).unwrap();
                let port = lan.port;
                let shared = lan.shared.clone();
                let config = super::HubConfig {
                    default_game_mode: 0,
                    guest_commands: false,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 10,
                    simulation_distance: 10,
                    dimension: "minecraft:overworld".into(),
                    allow_flight: false,
                    seed: 0,
                    max_players: 8,
                    integrated: true,
                    levels: Default::default(),
                    level_seeds: Default::default(),
                };
                let encoder = ChunkEncoder::new(
                    [] as [(Arc<str>, bool); 0],
                    &sync.builtin,
                    &[],
                    &[],
                    Default::default(),
                );
                game.server.lan = Some(LanHost::for_test(Hub::new(
                    lan,
                    sync,
                    encoder,
                    config,
                    game.loop_context.clone(),
                )));
                let tcp = || {
                    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
                    let connection = Connection::from_stream(stream.try_clone().unwrap()).unwrap();
                    connection.reader.set_read_timeout(Some(WAIT)).unwrap();
                    (connection, stream)
                };
                let intention = |c: &mut Connection, host: &str, port, protocol, intent| {
                    c.send(
                        &handshake::Serverbound::Intention(handshake::Intention {
                            host: host.into(),
                            port,
                            protocol,
                            intent,
                        })
                        .encode(),
                    )
                    .unwrap();
                };
                let hello = |c: &mut Connection, name: &str| {
                    c.send(
                        &login::Serverbound::Hello(login::ServerHello {
                            name: name.into(),
                            uuid: 1, // Offline admission derives the UUID from the name instead.
                        })
                        .encode(),
                    )
                    .unwrap();
                };
                let login_packet = |c: &mut Connection| {
                    let (id, data) = c.recv().unwrap();
                    login::Clientbound::decode(id, &data).unwrap()
                };
                let assert_login = |c: &mut Connection, name: &str, addr| {
                    let login::Clientbound::LoginFinished(done) = login_packet(c) else {
                        panic!("expected LoginFinished for {name}");
                    };
                    assert_eq!(done.profile.name, name);
                    assert_eq!(done.profile.uuid, offline_uuid(name));
                    assert_eq!(
                        gate_calls.try_recv().unwrap(),
                        (addr, offline_uuid(name), name.into())
                    );
                };
                let assert_closed = |c: &mut Connection| {
                    assert!(c.recv().unwrap_err().is_end_of_stream());
                };
                let observed = |host: &str, port, peer_ip: Option<&str>| Seen {
                    host: host.into(),
                    port,
                    protocol: PROTOCOL_VERSION,
                    peer_ip: peer_ip.map(str::to_owned),
                    thread: thread::current().id(),
                };
                let pump = |game: &mut crate::game::ServerGame<'_>, count| {
                    let deadline = Instant::now() + WAIT;
                    while seen.lock().unwrap().len() < count {
                        assert!(Instant::now() < deadline, "handshake was not dispatched");
                        game.server.level_pass = Default::default();
                        game.tick_lan_packets();
                        thread::yield_now();
                    }
                    std::mem::take(&mut *seen.lock().unwrap())
                };
                let tcp_ip = Some("127.0.0.1");
                let (mut c, stream) = tcp();
                intention(&mut c, ORIGINAL, 25565, PROTOCOL_VERSION, handshake::LOGIN);
                hello(&mut c, "FirstPeer");
                assert_eq!(
                    pump(game, 2),
                    [
                        observed(ORIGINAL, 25565, tcp_ip),
                        observed(REWRITTEN, 25565, tcp_ip)
                    ]
                );
                assert_login(&mut c, "FirstPeer", stream.local_addr().unwrap());
                drop((c, stream));

                let generic = json!({"translate": "multiplayer.disconnect.generic"});
                for (host, expected) in [
                    ("closed.test", json!("Fixture refused")),
                    ("default.test", generic.clone()),
                    ("deep.test", generic.clone()),
                    ("long.test", generic.clone()),
                    ("item.test", Value::Null),
                ] {
                    let (mut c, _stream) = tcp();
                    intention(&mut c, host, 25565, PROTOCOL_VERSION, handshake::LOGIN);
                    assert_eq!(pump(game, 1), [observed(host, 25565, tcp_ip)]);
                    let login::Clientbound::Disconnect(refusal) = login_packet(&mut c) else {
                        panic!("expected login disconnect");
                    };
                    let reason: Value = serde_json::from_str(&refusal.reason_json).unwrap();
                    if expected.is_null() {
                        assert_eq!(reason["text"], "Fixture item");
                        let hover = &reason["hover_event"];
                        assert_eq!(
                            (&hover["action"], &hover["id"]),
                            (&json!("show_item"), &json!("minecraft:stone"))
                        );
                        assert_eq!(
                            hover["components"]["minecraft:custom_model_data"]["floats"],
                            json!([0.5])
                        );
                    } else {
                        assert_eq!(reason, expected);
                    }
                    assert_closed(&mut c);
                }

                let mut c = Connection::local(
                    game.server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .lan
                        .connect_local()
                        .unwrap(),
                );
                c.reader.set_read_timeout(Some(WAIT)).unwrap();
                intention(&mut c, "local.test", 0, PROTOCOL_VERSION, handshake::LOGIN);
                let long_host = format!("{REWRITTEN}{}", "a".repeat(255));
                assert_eq!(
                    pump(game, 2),
                    [
                        observed("local.test", 0, None),
                        observed(&long_host, 0, None)
                    ]
                );
                hello(&mut c, "LocalPeer");
                assert_login(&mut c, "LocalPeer", sunrise_sim::lan::conn::MEMORY_ADDRESS);
                drop(c);

                // A peer that leaves a whole Hello behind once its handshake was taken reaches
                // no admission.
                let mut c = Connection::local(
                    game.server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .lan
                        .connect_local()
                        .unwrap(),
                );
                intention(&mut c, "local.left", 0, PROTOCOL_VERSION, handshake::LOGIN);
                let deadline = Instant::now() + WAIT;
                let handoffs = loop {
                    let handoffs = game.server.lan.as_ref().unwrap().hub.lan.take_handoffs();
                    if !handoffs.is_empty() {
                        break handoffs;
                    }
                    assert!(Instant::now() < deadline, "memory handshake was not queued");
                    thread::yield_now();
                };
                hello(&mut c, "LeftPeer");
                drop(c);
                game.tick_lan_player_handshakes(handoffs);
                assert_eq!(seen.lock().unwrap().len(), 2);
                seen.lock().unwrap().clear();
                assert_eq!(
                    gate_calls.recv_timeout(Duration::from_millis(500)),
                    Err(mpsc::RecvTimeoutError::Timeout)
                );

                let (mut c, _stream) = tcp();
                intention(&mut c, ORIGINAL, 25565, PROTOCOL_VERSION, handshake::STATUS);
                c.send(&status::Serverbound::StatusRequest(Empty).encode())
                    .unwrap();
                let (id, bytes) = c.recv().unwrap();
                assert!(matches!(
                    status::Clientbound::decode(id, &bytes).unwrap(),
                    status::Clientbound::StatusResponse(_)
                ));
                c.send(&status::Serverbound::PingRequest(status::Ping { time: 4242 }).encode())
                    .unwrap();
                let (id, bytes) = c.recv().unwrap();
                assert_eq!(
                    status::Clientbound::decode(id, &bytes).unwrap(),
                    status::Clientbound::PongResponse(status::Ping { time: 4242 })
                );
                assert_closed(&mut c);

                let (mut c, _stream) = tcp();
                intention(
                    &mut c,
                    &"a".repeat(256),
                    25565,
                    PROTOCOL_VERSION,
                    handshake::LOGIN,
                );
                assert_closed(&mut c);

                for (protocol, intent, key) in [
                    (
                        PROTOCOL_VERSION - 1,
                        handshake::LOGIN,
                        "multiplayer.disconnect.incompatible",
                    ),
                    (
                        PROTOCOL_VERSION,
                        handshake::TRANSFER,
                        "multiplayer.disconnect.transfers_disabled",
                    ),
                ] {
                    let (mut c, _stream) = tcp();
                    intention(&mut c, ORIGINAL, 25565, protocol, intent);
                    let login::Clientbound::Disconnect(reason) = login_packet(&mut c) else {
                        panic!("expected vanilla refusal");
                    };
                    assert!(reason.reason_json.contains(key));
                    assert_closed(&mut c);
                }
                game.server.level_pass = Default::default();
                game.tick_lan_packets();
                assert!(seen.lock().unwrap().is_empty());
                assert_eq!(gate_calls.try_recv(), Err(mpsc::TryRecvError::Empty));

                shared.set_accepts_transfers(true);
                let (mut c, stream) = tcp();
                intention(
                    &mut c,
                    ORIGINAL,
                    25565,
                    PROTOCOL_VERSION,
                    handshake::TRANSFER,
                );
                hello(&mut c, "TransferPeer");
                assert_eq!(
                    pump(game, 2),
                    [
                        observed(ORIGINAL, 25565, tcp_ip),
                        observed(REWRITTEN, 25565, tcp_ip)
                    ]
                );
                assert_login(&mut c, "TransferPeer", stream.local_addr().unwrap());
                drop((c, stream));

                let (mut c, stream) = tcp();
                intention(&mut c, ORIGINAL, 25565, PROTOCOL_VERSION, handshake::LOGIN);
                hello(&mut c, "Abandoned");
                // Signal EOF to the server without giving up our ability to observe its reply.
                stream.shutdown(Shutdown::Write).unwrap();
                assert_eq!(
                    pump(game, 2),
                    [
                        observed(ORIGINAL, 25565, tcp_ip),
                        observed(REWRITTEN, 25565, tcp_ip)
                    ]
                );
                assert_closed(&mut c);
                assert_eq!(gate_calls.try_recv(), Err(mpsc::TryRecvError::Empty));

                assert!(mod_api::__private::set_mod_enabled(
                    "handshake_rewrite",
                    false
                ));
                assert!(mod_api::__private::set_mod_enabled(
                    "handshake_observe",
                    false
                ));
                let (mut c, stream) = tcp();
                intention(
                    &mut c,
                    "closed.test",
                    25565,
                    PROTOCOL_VERSION,
                    handshake::LOGIN,
                );
                hello(&mut c, "NoListenerPeer");
                // No tick is needed; the worker uses the captured table's current enabled set.
                assert_login(&mut c, "NoListenerPeer", stream.local_addr().unwrap());
                assert!(seen.lock().unwrap().is_empty());
                drop((c, stream));

                // A server draining for a stop still listens and refuses an accepted login.
                for id in ["handshake_rewrite", "handshake_observe"] {
                    assert!(mod_api::__private::set_mod_enabled(id, true));
                }
                shared.refuse_logins();
                let (mut c, _stream) = tcp();
                intention(&mut c, ORIGINAL, 25565, PROTOCOL_VERSION, handshake::LOGIN);
                hello(&mut c, "DrainPeer");
                assert_eq!(
                    pump(game, 2),
                    [
                        observed(ORIGINAL, 25565, tcp_ip),
                        observed(REWRITTEN, 25565, tcp_ip)
                    ]
                );
                let login::Clientbound::Disconnect(refusal) = login_packet(&mut c) else {
                    panic!("expected the shutdown refusal");
                };
                assert_eq!(
                    serde_json::from_str::<Value>(&refusal.reason_json).unwrap(),
                    json!({"translate": "multiplayer.disconnect.server_shutdown"})
                );
                assert_closed(&mut c);
                assert_eq!(gate_calls.try_recv(), Err(mpsc::TryRecvError::Empty));
            });
        });
    }

    #[test]
    fn a_burst_of_short_events_still_has_a_slow_tick_note() {
        use sunrise_level::spans;
        // Thread-local recording avoids changing instrumentation on other test threads.
        spans::record_here(true);
        spans::take_notes();
        let mut timings = EventTimings::new(spans::NOTE_MS);
        for _ in 0..200 {
            spans::note(5.0, || "UseItemOn of fixture player".into());
            timings.add("UseItemOn", "fixture player", None::<()>, 5.0);
        }
        assert!(
            spans::take_notes().is_empty(),
            "every individual event is below NOTE_MS"
        );
        for burst in timings.bursts() {
            spans::note(burst.ms, || format!("{} x {}", burst.count, burst.kind));
        }
        let notes = spans::take_notes();
        spans::record_here(false);
        assert_eq!(notes, vec![(1000.0, "200 x UseItemOn".into())]);
    }
    use sunrise_protocol::entity_data::{Value, serializer};
    use sunrise_sim::mobs::Mob;

    fn sync_test_items(
        root: &std::path::Path,
    ) -> std::sync::Arc<sunrise_world::item_defaults::ItemData> {
        use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
        let inputs = std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default()));
        let context = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, std::iter::empty());
        std::sync::Arc::new(sunrise_world::item_defaults::ItemData::load(root, context).unwrap())
    }

    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn guest_climbable_glide() {
        use std::sync::{Arc, atomic::AtomicI32, mpsc};
        use sunrise_gameplay_common::game::{Options, Progress, Settings, load_resources};
        use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest};
        use sunrise_sim::motion::MotionWorld;

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let res = loaded.resources;
        let mut game = crate::game::ServerGame::new(
            &res,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        let render = sunrise_client::assets::RenderAssets::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            &game.pack.assets,
        )
        .unwrap();
        game.world =
            sunrise_client::assets::test_scene_world(&game.pack.assets, &render, 2).unwrap();

        let sync = Arc::new(sunrise_sim::lan::data::SyncData::empty());
        let net = NetConfig {
            listen: false,
            ..NetConfig::loopback(0, String::new(), 2)
        };
        let lan = sunrise_sim::lan::Lan::start_with(sync.clone(), net).unwrap();
        let encoder = sunrise_sim::lan::chunks::ChunkEncoder::new(
            [] as [(Arc<str>, bool); 0],
            &sunrise_protocol::registry::Builtin::default(),
            &[],
            &[],
            Default::default(),
        );
        let config = sunrise_sim::lan::hub::HubConfig {
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 10,
            simulation_distance: 10,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 20,
            integrated: true,
            levels: Default::default(),
            level_seeds: Default::default(),
        };
        let mut hub =
            sunrise_sim::lan::hub::Hub::new(lan, sync, encoder, config, game.loop_context.clone());
        let (tx, _rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 1,
                uuid: 1,
                name: "Test".into(),
                properties: Vec::new(),
                info: sunrise_protocol::packets::configuration::ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            1,
            [0.5, 4.0, 0.5],
            8,
        );
        guest.placed = true;
        guest.dimension = "minecraft:overworld".into();
        hub.guests.push(guest);
        game.server.lan = Some(LanHost::for_test(hub));
        for (block, expected) in [
            ("minecraft:ladder", false),
            ("minecraft:scaffolding", false),
            ("minecraft:vine", true),
        ] {
            let state = game.pack.assets.registry.default_state(block).unwrap();
            game.world.set(0, 4, 0, state).unwrap();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.fall_flying = true);
            game.as_guest(1, |game| {
                let player = game.player.as_mut().unwrap();
                let side = game_side!(game);
                MotionWorld {
                    side: &side,
                    entities: &[],
                    player,
                    scene: &mut game.world,
                    assets: &game.pack.assets,
                    collision_binding: game.collision,
                    resources: game.motion,
                    health: &game.health_resources,
                    tag_items: Some(world_items!(game)),
                    loop_context: &game.loop_context,
                    damage_hooks: &game.damage_hooks,
                    block_registries: Some(&game.pack.block_registries),
                }
                .server_travel()
                .unwrap();
            })
            .unwrap();
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].fall_flying,
                expected,
                "{block}"
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn datapack_damage_world_open_reuses_the_no_pack_preload() {
        run_datapack_damage_world_open_reuses_the_no_pack_preload();
    }

    pub(in crate::game::server_side) fn run_datapack_damage_world_open_reuses_the_no_pack_preload()
    {
        use super::{
            SyncData,
            sync::{SyncCache, cached_sync_data, initialize_world_sync_data},
        };
        use std::{path::Path, sync::Arc};
        use sunrise_world::loot::PackOverlay;
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        let item_data = sync_test_items(&root);
        let items =
            sunrise_items::items::Items::catalog(&root, &PackOverlay::NONE, item_data.defaults())
                .unwrap();
        let preloaded = Arc::new(
            SyncData::load(
                &root,
                &PackOverlay::NONE,
                item_data.defaults(),
                &super::sync::server_sources(&root, &item_data, &items, &PackOverlay::NONE)
                    .unwrap(),
            )
            .unwrap(),
        );
        let mut cache = SyncCache {
            data: Some(super::sync::Synced {
                root: root.join("no-such-resources"),
                with_packs: false,
                generation: item_data.defaults().generation(),
                result: Ok(preloaded.clone()),
            }),
            ..Default::default()
        };
        let damage =
            sunrise_world::damage_source::Catalog::load(&root, &PackOverlay::NONE).unwrap();
        // A missing root makes an accidental second disk read fail, rather
        // than merely comparing equal snapshots loaded twice.
        let absent = root.join("no-such-resources");
        initialize_world_sync_data(
            &mut cache,
            &absent,
            &PackOverlay::NONE,
            &damage,
            &item_data,
            &items,
        )
        .unwrap();
        let opened =
            cached_sync_data(&mut cache, &absent, &PackOverlay::NONE, &item_data, &items).unwrap();
        assert!(Arc::ptr_eq(&preloaded, &opened));
        assert!(Arc::ptr_eq(
            &opened,
            &cached_sync_data(&mut cache, &absent, &PackOverlay::NONE, &item_data, &items).unwrap()
        ));
    }

    #[test]
    #[ignore = "daily"]
    fn datapack_damage_world_open_replaces_preload_atomically() {
        run_datapack_damage_world_open_replaces_preload_atomically();
    }

    pub(in crate::game::server_side) fn run_datapack_damage_world_open_replaces_preload_atomically()
    {
        use super::{
            SyncData,
            sync::{SyncCache, cached_sync_data, initialize_world_sync_data},
        };
        use std::{
            path::Path,
            sync::{Arc, Barrier, Mutex},
        };
        use sunrise_world::loot::PackOverlay;
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        let item_data = sync_test_items(&root);
        let items = Arc::new(
            sunrise_items::items::Items::catalog(&root, &PackOverlay::NONE, item_data.defaults())
                .unwrap(),
        );
        let preloaded = Arc::new(
            SyncData::load(
                &root,
                &PackOverlay::NONE,
                item_data.defaults(),
                &super::sync::server_sources(&root, &item_data, &items, &PackOverlay::NONE)
                    .unwrap(),
            )
            .unwrap(),
        );
        let cache = Arc::new(Mutex::new(SyncCache {
            data: Some(super::sync::Synced {
                root: root.clone(),
                with_packs: false,
                generation: item_data.defaults().generation(),
                result: Ok(preloaded.clone()),
            }),
            ..Default::default()
        }));
        let barrier = Arc::new(Barrier::new(2));
        let worker = {
            let cache = cache.clone();
            let barrier = barrier.clone();
            let root = root.clone();
            let item_data = item_data.clone();
            let items = items.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..32 {
                    cached_sync_data(
                        &mut cache.lock().unwrap(),
                        &root,
                        &PackOverlay::NONE,
                        &item_data,
                        &items,
                    )
                    .unwrap();
                }
            })
        };
        let files = vec![(
            "damage_type/demo:world_heat".into(),
            vec![br#"{"message_id":"world_heat","scaling":"never","exhaustion":0.25}"#.to_vec()],
        )];
        barrier.wait();
        let overlay = PackOverlay::new(&files);
        let damage = sunrise_world::damage_source::Catalog::load(&root, &overlay).unwrap();
        initialize_world_sync_data(
            &mut cache.lock().unwrap(),
            &root,
            &overlay,
            &damage,
            &item_data,
            &items,
        )
        .unwrap();
        let opened = cached_sync_data(
            &mut cache.lock().unwrap(),
            &root,
            &overlay,
            &item_data,
            &items,
        )
        .unwrap();
        worker.join().unwrap();
        assert!(!Arc::ptr_eq(&preloaded, &opened));
        let id = opened
            .id("minecraft:damage_type", "demo:world_heat")
            .unwrap();
        let later = cached_sync_data(
            &mut cache.lock().unwrap(),
            &root,
            &PackOverlay::NONE,
            &item_data,
            &items,
        )
        .unwrap();
        assert!(
            Arc::ptr_eq(&opened, &later),
            "a late helper or disabled pack cannot replace a frozen world"
        );
        let packet = later.registry_packets(true, &sunrise_level::chunk::LevelShapes::VANILLA);
        let packet = packet
            .iter()
            .find(|p| p.registry == "minecraft:damage_type")
            .unwrap();
        assert_eq!(packet.entries[id as usize].id, "demo:world_heat");
        assert!(packet.entries[id as usize].data.is_some());
    }

    #[test]
    #[ignore = "daily"]
    fn datapack_damage_unrelated_packs_do_not_load_sync_data_at_open() {
        run_datapack_damage_unrelated_packs_do_not_load_sync_data_at_open();
    }

    pub(in crate::game::server_side) fn run_datapack_damage_unrelated_packs_do_not_load_sync_data_at_open()
     {
        use super::{
            SyncData,
            sync::{SyncCache, initialize_world_sync_data},
        };
        use std::{path::Path, sync::Arc};
        use sunrise_world::{damage_source::Catalog, loot::PackOverlay};
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        let item_data = sync_test_items(&root);
        let items =
            sunrise_items::items::Items::catalog(&root, &PackOverlay::NONE, item_data.defaults())
                .unwrap();
        let damage = Catalog::load(&root, &PackOverlay::NONE).unwrap();
        let preloaded = Arc::new(
            SyncData::load(
                &root,
                &PackOverlay::NONE,
                item_data.defaults(),
                &super::sync::server_sources(&root, &item_data, &items, &PackOverlay::NONE)
                    .unwrap(),
            )
            .unwrap(),
        );
        // A recipe-only pack and a damage-tag-only pack must retain main's
        // lazy wire-data path. An absent root proves no full load happens.
        for key in ["recipe/demo:test", "tags/minecraft:damage_type/is_fire"] {
            let files = vec![(key.into(), vec![br#"{"values":[]}"#.to_vec()])];
            let mut cache = SyncCache {
                data: Some(super::sync::Synced {
                    root: root.clone(),
                    with_packs: false,
                    generation: item_data.defaults().generation(),
                    result: Ok(preloaded.clone()),
                }),
                ..Default::default()
            };
            initialize_world_sync_data(
                &mut cache,
                &root.join("absent"),
                &PackOverlay::new(&files),
                &damage,
                &item_data,
                &items,
            )
            .unwrap();
            assert!(cache.data.is_none(), "{key} must not eagerly load SyncData");
            assert!(cache.damage.is_some());
        }
        let mut empty = SyncCache::default();
        initialize_world_sync_data(
            &mut empty,
            &root.join("absent"),
            &PackOverlay::NONE,
            &damage,
            &item_data,
            &items,
        )
        .unwrap();
        assert!(
            empty.data.is_none(),
            "no-pack worlds need no synchronous fallback load either"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn datapack_damage_lazy_wire_snapshot_survives_reload_before_first_hub() {
        run_datapack_damage_lazy_wire_snapshot_survives_reload_before_first_hub();
    }

    pub(in crate::game::server_side) fn run_datapack_damage_lazy_wire_snapshot_survives_reload_before_first_hub()
     {
        use super::sync::{
            SyncCache, cached_sync_data, initialize_world_sync_data, prepare_sync_reload,
        };
        use std::path::{Path, PathBuf};
        use sunrise_world::{damage_source::Catalog, loot::PackOverlay};
        let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client")
                    .join(sunrise_core::VANILLA_VERSION)
            });
        let item_data = sync_test_items(&root);
        let items =
            sunrise_items::items::Items::catalog(&root, &PackOverlay::NONE, item_data.defaults())
                .unwrap();
        let damage = Catalog::load(&root, &PackOverlay::NONE).unwrap();
        let initial = vec![(
            "dialog/demo:before".into(),
            vec![br#"{"type":"minecraft:notice","title":"Before"}"#.to_vec()],
        )];
        let initial = PackOverlay::new(&initial);
        let (packed, admitted) = sunrise_sim::lan::data::SyncData::stage_registry_definitions(
            &root,
            item_data.defaults(),
            &initial,
            &Default::default(),
            &Default::default(),
            |_| true,
        );
        admitted.unwrap();
        let mut cache = SyncCache {
            packed_registries: Some(std::sync::Arc::new(packed)),
            ..SyncCache::default()
        };
        initialize_world_sync_data(&mut cache, &root, &initial, &damage, &item_data, &items)
            .unwrap();
        assert!(cache.data.is_none());
        // A late vanilla helper cannot install its overlay over this world.
        cached_sync_data(&mut cache, &root, &PackOverlay::NONE, &item_data, &items).unwrap();
        assert!(cache.data.is_none());
        let changed = vec![
            (
                "damage_type/demo:too_late".into(),
                vec![br#"{"message_id":"too_late","scaling":"never","exhaustion":0.5}"#.to_vec()],
            ),
            (
                "damage_type/minecraft:fall".into(),
                vec![br#"{"message_id":"changed","scaling":"never","exhaustion":0.9}"#.to_vec()],
            ),
            (
                "dialog/demo:after".into(),
                vec![br#"{"type":"minecraft:notice","title":"After"}"#.to_vec()],
            ),
            (
                "tags/demo:damage_type/reloaded".into(),
                vec![
                    br#"{"values":["minecraft:fall",{"id":"demo:too_late","required":false}]}"#
                        .to_vec(),
                ],
            ),
        ];
        let overlay = PackOverlay::new(&changed);
        assert!(prepare_sync_reload(&mut cache, &overlay).is_none());
        let health = damage.reload_tags(&root, &overlay).unwrap();
        let wire = cached_sync_data(&mut cache, &root, &overlay, &item_data, &items).unwrap();
        assert!(wire.id("minecraft:damage_type", "demo:too_late").is_none());
        assert!(health.id("demo:too_late").is_none());
        assert!(wire.id("minecraft:dialog", "demo:before").is_some());
        assert!(wire.id("minecraft:dialog", "demo:after").is_none());
        let fall = wire.id("minecraft:damage_type", "minecraft:fall").unwrap();
        let packets = wire.registry_packets(false, &sunrise_level::chunk::LevelShapes::VANILLA);
        let packet = packets
            .iter()
            .find(|p| p.registry == "minecraft:damage_type")
            .unwrap();
        let definition = health
            .definition(health.id("minecraft:fall").unwrap())
            .unwrap();
        assert_eq!(
            packet.entries[fall as usize].data.as_ref(),
            Some(&definition.to_nbt())
        );
        let tags = wire
            .tags
            .registries
            .iter()
            .find(|r| r.registry == "minecraft:damage_type")
            .unwrap();
        assert_eq!(
            tags.tags
                .iter()
                .find(|t| t.name == "demo:reloaded")
                .unwrap()
                .ids,
            vec![fall]
        );
        assert_eq!(
            health.tag_members("demo:reloaded").collect::<Vec<_>>(),
            vec!["minecraft:fall"]
        );
        // The next world's initial load can register the previously-late type.
        let next = Catalog::load(&root, &overlay).unwrap();
        let mut next_cache = SyncCache::default();
        initialize_world_sync_data(&mut next_cache, &root, &overlay, &next, &item_data, &items)
            .unwrap();
        assert!(
            cached_sync_data(&mut next_cache, &root, &overlay, &item_data, &items)
                .unwrap()
                .id("minecraft:damage_type", "demo:too_late")
                .is_some()
        );
    }

    #[test]
    #[ignore = "daily"]
    fn datapack_damage_lazy_wire_load_caches_after_all_packs_are_removed() {
        run_datapack_damage_lazy_wire_load_caches_after_all_packs_are_removed();
    }

    pub(in crate::game::server_side) fn run_datapack_damage_lazy_wire_load_caches_after_all_packs_are_removed()
     {
        if crate::game::native_entity_refs::tests::run_fresh_cases(
            "game::server_side::lan::tests::datapack_damage_lazy_wire_load_caches_after_all_packs_are_removed",
            "SUNRISE_LAZY_DAMAGE_CUSTOM_COMPONENTS",
            &["custom"],
        ) {
            return;
        }
        // Component registration is process-global. Exercise the identity path
        // deterministically, without depending on another test's registration.
        sunrise_protocol::item::custom::install(1000, vec!["lazy_damage:test".into()]).unwrap();
        use super::sync::{
            SyncCache, cached_sync_data, initialize_world_sync_data, prepare_sync_reload,
        };
        use std::{path::Path, sync::Arc};
        use sunrise_world::{damage_source::Catalog, loot::PackOverlay};

        fn copy_tree(source: &Path, destination: &Path) {
            std::fs::create_dir_all(destination).unwrap();
            for entry in std::fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                let target = destination.join(entry.file_name());
                if entry.path().is_dir() {
                    copy_tree(&entry.path(), &target);
                } else {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }

        let prepared = std::env::var_os("SUNRISE_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client")
                    .join(sunrise_core::VANILLA_VERSION)
            });
        let root = std::env::temp_dir().join(format!(
            "sunrise-lazy-wire-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        // Keep the cache's resource-root identity stable while making a reload fail.
        // Copy data and the identity inputs, not prepared assets. Custom components
        // make wire loading hash facts.json even when this world's packs are empty.
        copy_tree(&prepared.join("data"), &root.join("data"));
        std::fs::copy(prepared.join("facts.json"), root.join("facts.json")).unwrap();
        let report = sunrise_protocol::registry::Builtin::find(&prepared)
            .expect("prepared test resources require registries.json");
        std::fs::copy(report, root.join("registries.json")).unwrap();
        let item_data = sync_test_items(&prepared);
        let items = sunrise_items::items::Items::catalog(
            &prepared,
            &PackOverlay::NONE,
            item_data.defaults(),
        )
        .unwrap();
        let damage = Catalog::load(&prepared, &PackOverlay::NONE).unwrap();
        let files = vec![(
            "recipe/demo:unrelated".into(),
            vec![br#"{"type":"minecraft:crafting_special_mapcloning"}"#.to_vec()],
        )];
        let mut cache = SyncCache::default();
        initialize_world_sync_data(
            &mut cache,
            &root,
            &PackOverlay::new(&files),
            &damage,
            &item_data,
            &items,
        )
        .unwrap();
        assert!(cache.data.is_none());
        assert!(prepare_sync_reload(&mut cache, &PackOverlay::NONE).is_none());
        let opened =
            cached_sync_data(&mut cache, &root, &PackOverlay::NONE, &item_data, &items).unwrap();
        assert!(opened.content_identity.is_some());
        // Changing the root invalidates the cache. Instead remove the private
        // fixture's files: an accidental reload at the same root must now fail.
        std::fs::remove_dir_all(&root).unwrap();
        assert!(sunrise_protocol::registry::Builtin::find(&root).is_none());
        let reused =
            cached_sync_data(&mut cache, &root, &PackOverlay::NONE, &item_data, &items).unwrap();
        assert!(Arc::ptr_eq(&opened, &reused));
    }

    fn mob_of(kind: &str) -> Option<Mob> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        let types = sunrise_sim::mobs::MobTypes::load(&root).ok()?;
        let t = std::sync::Arc::new(types.types.get(kind)?.clone());
        Some(Mob::new(
            1,
            t,
            sunrise_core::geometry::Vec3::new(0.0, 64.0, 0.0),
            0.0,
        ))
    }

    fn freeze_meta(m: &Mob) -> (Vec<(u8, i32, Value)>, Vec<(u8, i32, Value)>) {
        let (mut meta, mut defaults) = (Vec::new(), Vec::new());
        mob_freeze_meta(m, &mut meta, &mut defaults);
        (meta, defaults)
    }

    /// Entity.setTicksFrozen can go negative; index 7 must carry it. Index 1
    /// (air supply) goes out for every mob, with the species' own default.
    #[test]
    fn negative_frozen_ticks_and_axolotl_air_are_synced() {
        let Some(mut cow) = mob_of("minecraft:cow") else {
            return;
        };
        cow.ticks_frozen = -3;
        let (meta, _) = freeze_meta(&cow);
        assert!(meta.contains(&(7, serializer::INT, Value::Int(-3))));
        cow.air = 100;
        assert!(
            freeze_meta(&cow)
                .0
                .contains(&(1, serializer::INT, Value::Int(100)))
        );

        let Some(mut axolotl) = mob_of("minecraft:axolotl") else {
            return;
        };
        axolotl.air = 5000;
        let (meta, defaults) = freeze_meta(&axolotl);
        assert!(meta.contains(&(1, serializer::INT, Value::Int(5000))));
        assert!(defaults.contains(&(1, serializer::INT, Value::Int(6000))));
        axolotl.air = 6000;
        assert!(freeze_meta(&axolotl).0.iter().all(|e| e.0 != 1));
    }

    /// Raider.IS_CELEBRATING goes out through the tracker's own metadata
    /// (`mob_meta`), not only through the helper: false is listed as a
    /// default so the end of a celebration is sent, and true is sent while
    /// the raider celebrates.
    #[test]
    fn a_celebrating_raider_is_sent_by_mob_meta() {
        let Some(mut pillager) = mob_of("minecraft:pillager") else {
            return;
        };
        let hub = meta_hub("RaiderMetaTest");
        let meta_of = |m: &Mob| {
            let (mut meta, mut defaults) = (Vec::new(), Vec::new());
            mob_meta(m, &hub, None, 0, &mut meta, &mut defaults);
            (meta, defaults)
        };

        let (meta, defaults) = meta_of(&pillager);
        assert!(defaults.contains(&(16, serializer::BOOLEAN, Value::Bool(false))));
        assert!(meta.iter().all(|e| e.0 != 16), "{meta:?}");
        sunrise_sim::mobs::raider::state_mut(&mut pillager).celebrating = true;
        let (meta, _) = meta_of(&pillager);
        assert!(
            meta.contains(&(16, serializer::BOOLEAN, Value::Bool(true))),
            "{meta:?}"
        );
    }

    /// `Enderman.setTarget` and `setBeingStaredAt` reach normal tracker data,
    /// including defaults needed to send the clear.
    #[test]
    #[ignore = "daily"]
    fn enderman_target_flags_are_tracked_and_cleared() {
        let mut enderman = mob_of("minecraft:enderman").expect("prepared mob types");
        let hub = meta_hub("EndermanTargetMetaTest");
        let metadata = |m: &Mob| {
            let (mut values, mut defaults) = (Vec::new(), Vec::new());
            mob_meta(m, &hub, None, 0, &mut values, &mut defaults);
            (values, defaults)
        };
        enderman.extra.flag2 = true;
        enderman.set_target(Some(sunrise_sim::mobs::Who::Mob(2)));
        let (values, defaults) = metadata(&enderman);
        for index in [17, 18] {
            assert!(values.contains(&(index, serializer::BOOLEAN, Value::Bool(true))));
            assert!(defaults.contains(&(index, serializer::BOOLEAN, Value::Bool(false))));
        }
        enderman.set_target(None);
        let (values, defaults) = metadata(&enderman);
        for index in [17, 18] {
            assert!(!values.iter().any(|entry| entry.0 == index));
            assert!(defaults.contains(&(index, serializer::BOOLEAN, Value::Bool(false))));
        }
    }

    /// What is stuck in a mob goes out through the tracker's metadata
    /// (`LivingEntity.DATA_ARROW_COUNT_ID`, `DATA_STINGER_COUNT_ID`): the
    /// counts that are set, and zero as the default so a count that falls
    /// back to none is sent.
    #[test]
    fn a_mobs_stuck_counts_are_sent_by_mob_meta() {
        let types = sunrise_gameplay_common::game::mods::living_mod_types();
        let hub = meta_hub("StuckMetaTest");
        let mut mob = Mob::new(
            1,
            types["minecraft:zombie"].clone(),
            sunrise_core::geometry::Vec3::ZERO,
            0.0,
        );
        let meta_of = |m: &Mob| {
            let (mut meta, mut defaults) = (Vec::new(), Vec::new());
            mob_meta(m, &hub, None, 0, &mut meta, &mut defaults);
            (meta, defaults)
        };
        let count = |index| (index, serializer::INT, Value::Int(0));

        let (meta, defaults) = meta_of(&mob);
        assert!(meta.iter().all(|e| e.0 != 12 && e.0 != 13), "{meta:?}");
        assert!(defaults.contains(&count(12)) && defaults.contains(&count(13)));
        mob.extra.stuck.arrows = 4;
        mob.extra.stuck.stingers = 2;
        let (meta, _) = meta_of(&mob);
        assert!(
            meta.contains(&(12, serializer::INT, Value::Int(4))),
            "{meta:?}"
        );
        assert!(
            meta.contains(&(13, serializer::INT, Value::Int(2))),
            "{meta:?}"
        );
    }

    /// A mod's custom name and its visibility reach the tracking players
    /// (`Entity.DATA_CUSTOM_NAME`, `DATA_CUSTOM_NAME_VISIBLE`), and taking
    /// them away is sent as none / false.
    #[test]
    #[ignore = "daily"]
    fn a_mods_custom_name_and_its_visibility_are_sent() {
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let at = sunrise_core::geometry::Vec3::new(0.5, 70.0, 0.5);
            game.mobs.push(Mob::new(12, kind, at, 0.0));
            let mob = game.entity(12).unwrap();
            let hub = meta_hub("CustomNameMetaTest");
            let meta_of = |game: &crate::game::ServerGame<'_>| {
                let m = game.mobs.iter().find(|m| m.id == 12).unwrap();
                let (mut meta, mut defaults) = (Vec::new(), Vec::new());
                mob_meta(m, &hub, None, 0, &mut meta, &mut defaults);
                (meta, defaults)
            };
            assert!(
                mob.set_custom_name(game, Some(mod_api::Text::literal("Grumbles")))
                    .unwrap()
            );
            assert!(mob.set_custom_name_visible(game, true));
            let (meta, _) = meta_of(game);
            let sent = sunrise_protocol::text::literal("Grumbles");
            assert!(meta.contains(&(
                2,
                serializer::OPTIONAL_COMPONENT,
                Value::OptText(Some(sent))
            )));
            assert!(meta.contains(&(3, serializer::BOOLEAN, Value::Bool(true))));
            assert!(mob.set_custom_name(game, None).unwrap());
            assert!(mob.set_custom_name_visible(game, false));
            let (meta, defaults) = meta_of(game);
            assert!(meta.iter().all(|e| e.0 != 2 && e.0 != 3), "{meta:?}");
            assert!(defaults.contains(&(2, serializer::OPTIONAL_COMPONENT, Value::OptText(None))));
            assert!(defaults.contains(&(3, serializer::BOOLEAN, Value::Bool(false))));
        });
    }

    #[test]
    fn unresolved_profession_sends_none_without_changing_saved_identity() {
        let types = sunrise_gameplay_common::game::mods::living_mod_types();
        let hub = meta_hub("MissingProfessionMeta");
        let mut mob = Mob::new(
            1,
            types["minecraft:villager"].clone(),
            sunrise_core::geometry::Vec3::ZERO,
            0.0,
        );
        let metadata = |mob: &Mob| {
            let (mut entries, mut defaults) = (Vec::new(), Vec::new());
            mob_meta(mob, &hub, None, 0, &mut entries, &mut defaults);
            entries
                .into_iter()
                .find(|(_, serializer, _)| *serializer == serializer::VILLAGER_DATA)
        };
        let expected = metadata(&mob);
        assert!(expected.is_some());
        mob.extra.profession =
            sunrise_sim::mobs::villager::ProfessionState::from_id("example:engineer");
        assert_eq!(metadata(&mob), expected);
        assert_eq!(mob.extra.profession.id(), "example:engineer");
    }

    /// A content mod's living type sends the entity data of its template's
    /// class (`Zombie.DATA_BABY_ID`, the villager's unhappy counter), entry
    /// for entry what its template sends.
    #[test]
    fn a_mod_living_type_sends_its_templates_entity_data() {
        let types = sunrise_gameplay_common::game::mods::living_mod_types();
        let hub = meta_hub("ModMobMetaTest");
        let meta_of = |id: &str| {
            let at = sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5);
            let mut m = Mob::new(1, types[id].clone(), at, 0.0);
            m.extra.age = -24000;
            // The same health: the ghoul's own maximum is 40, the zombie's 20.
            m.health = 10.0;
            let (mut meta, mut defaults) = (Vec::new(), Vec::new());
            mob_meta(&m, &hub, None, 0, &mut meta, &mut defaults);
            (meta, defaults)
        };
        for (template, id) in [
            ("minecraft:zombie", "ruby:ghoul"),
            ("minecraft:villager", "ruby:settler"),
            ("minecraft:wandering_trader", "ruby:merchant"),
        ] {
            assert_eq!(
                meta_of(id),
                meta_of(template),
                "{id} sends what {template} sends"
            );
        }
    }

    pub(in crate::game) use sunrise_gameplay_common::game::lan_meta_fixture::meta_hub;

    /// `LivingEntity.startUsingItem` sets flag 1 for the use and flag 2 for
    /// the off hand, so an off-hand use is 3, not 2. A reader takes bit 0 as
    /// `isUsingItem` and bit 1 as `getUsedItemHand`.
    #[test]
    fn an_off_hand_use_flags_both_the_use_and_the_hand() {
        assert_eq!(living_flags(None, false), 0);
        assert_eq!(living_flags(Some((0, 12)), false), 1);
        assert_eq!(living_flags(Some((OFFHAND_SLOT, 12)), false), 3);
    }

    /// `LivingEntity.startAutoSpinAttack` sets flag 4, which a use of an item
    /// leaves as it is (a trident is charged before it launches, and a
    /// spinning player can use another item).
    #[test]
    fn a_riptide_spin_flags_bit_four() {
        assert_eq!(living_flags(None, true), 4);
        assert_eq!(living_flags(Some((0, 12)), true), 5);
        assert_eq!(living_flags(Some((OFFHAND_SLOT, 12)), true), 7);
    }

    /// `ComposterBlock.useItemOn` calls `levelEvent(1500, pos, data)`, the
    /// null-source overload, so the player who filled it still receives
    /// `handleFill`. `Block.playerWillDestroy`'s 2001 names that player.
    #[test]
    fn the_composter_fill_reaches_the_player_who_filled_it() {
        let actor = Some(4);
        assert_eq!(super::level_event_except(1500, actor, false), None);
        assert_eq!(super::level_event_except(2001, actor, false), actor);
        assert_eq!(super::level_event_except(2001, actor, true), None);
    }

    /// `server_chat`'s replies reach the sender alone, as system chat, whether
    /// or not the message goes out; a canceled message reaches nobody.
    #[test]
    #[ignore = "daily"]
    fn server_chat_replies_reach_the_sender_alone() {
        use super::super::native_entity_refs::tests::with_game;
        use mod_api::{Flow, Hooks, ServerChatEvent, ServerHost};
        use std::sync::{Arc, atomic::AtomicI32, mpsc};
        use sunrise_game_data::text::Text;
        use sunrise_protocol::packets::{configuration::ClientInformation, play};
        use sunrise_sim::lan::hub::{HostEvent, HubConfig, chat::ChatMessage};
        use sunrise_sim::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest};

        struct Replier;
        #[mod_api::__subscriptions]
        impl Hooks for Replier {
            fn server_chat(
                &self,
                _host: &mut dyn ServerHost,
                event: &mut ServerChatEvent<'_>,
            ) -> Flow {
                event
                    .replies
                    .push(mod_api::Text::literal(format!("heard {}", event.raw_text)));
                if event.raw_text == "secret" {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("replier", Box::new(Replier))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let settings = super::LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                };
                let net = NetConfig {
                    listen: false,
                    announce: false,
                    lod_distance: 0,
                    ..NetConfig::lan(0, String::new(), 3)
                };
                let config = HubConfig {
                    default_game_mode: 0,
                    guest_commands: false,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 2,
                    simulation_distance: 2,
                    dimension: game.dim.id().to_owned(),
                    allow_flight: true,
                    seed: 0,
                    max_players: 3,
                    integrated: true,
                    levels: game.server.level_shapes.clone(),
                    level_seeds: game.server.levels.0.runtime.seeds(),
                };
                game.lan_host(settings, net, config).unwrap();
                let mut join = |conn: u32| {
                    let (tx, rx) = mpsc::channel();
                    let mut guest = Guest::new(
                        Joined {
                            conn,
                            uuid: conn as u128,
                            name: format!("guest{conn}"),
                            properties: Vec::new(),
                            info: ClientInformation::new(8),
                            brand: String::new(),
                            sender: PlayerSender::new(tx, Arc::default()),
                            latency: Arc::new(AtomicI32::new(0)),
                            address: "127.0.0.1:1".parse().unwrap(),
                            expected: false,
                        },
                        conn as i32,
                        [0.5, 64.0, 0.5],
                        2,
                    );
                    guest.placed = true;
                    game.server.lan.as_mut().unwrap().hub.guests.push(guest);
                    rx
                };
                let (sender, other) = (join(12), join(13));
                // The chat packets each guest got, in order.
                let received = |rx: &mpsc::Receiver<Out>| {
                    let mut seen = Vec::new();
                    while let Ok(out) = rx.try_recv() {
                        let bodies = match out {
                            Out::Packet(body) => vec![body],
                            Out::Packets(bodies) => bodies,
                            Out::Disconnect(_) => continue,
                        };
                        for body in bodies {
                            let mut reader = sunrise_protocol::buf::Reader::new(&body);
                            let id = reader.varint().unwrap();
                            match play::Clientbound::decode(id, reader.rest()) {
                                Ok(play::Clientbound::SystemChat(chat)) => seen.push(format!(
                                    "system {}",
                                    sunrise_game_data::component::name_text(&chat.content).plain()
                                )),
                                Ok(play::Clientbound::PlayerChat(_)) => {
                                    seen.push("player".to_owned())
                                }
                                _ => {}
                            }
                        }
                    }
                    seen
                };
                received(&sender);
                received(&other);
                let say = |game: &mut crate::game::ServerGame<'_>, content: &str| {
                    let message = ChatMessage {
                        sender: 12,
                        index: 0,
                        signature: None,
                        content: content.to_owned(),
                        timestamp: 0,
                        salt: 0,
                        last_seen: Vec::new(),
                        unsigned_content: None,
                    };
                    let bound = play::ChatTypeBound {
                        chat_type: 0,
                        name: sunrise_sim::lan::data::component_nbt(
                            &Text::literal("guest12").component(),
                        ),
                        target: None,
                    };
                    let event = HostEvent::ChatSubmitted {
                        message: Box::new(message),
                        bound: Box::new(bound),
                        name: "guest12".into(),
                    };
                    let host = game.server.lan.take().unwrap();
                    game.server.lan = game.lan_process_events(host, [event], &[], [0.5, 64.0, 0.5]);
                };

                say(game, "secret");
                assert_eq!(received(&sender), ["system heard secret"]);
                assert!(received(&other).is_empty());

                say(game, "hello");
                assert_eq!(received(&sender), ["system heard hello", "player"]);
                assert_eq!(received(&other), ["player"]);
            });
        });
    }

    /// A guest who left takes its menu owners with it: the hub's leave event (`HostEvent::Left`)
    /// removes them with the play state, and another guest's stay.
    #[test]
    #[ignore = "daily"]
    fn a_guest_who_left_has_no_menu_owners() {
        use sunrise_sim::lan::hub::HostEvent;
        use sunrise_sim::menu::{Kind, Menu};
        use {
            crate::game::server_side::decision_engine_server::ServerDecisionTarget,
            sunrise_gameplay_common::game::transactions::decision::Admission,
        };
        super::super::native_guest_turn_tests::with_guests(|game| {
            let host = game.server.lan.as_mut().unwrap();
            for conn in [12, 13] {
                *host.menus.entry(conn).or_default().crafting.inventory =
                    Some(Box::new(Menu::new(Kind::Inventory)));
                host.play.entry(conn).or_default();
            }
            // The hub takes a guest out of its list before it reports the leave.
            let guest = host.hub.guests.remove(0);
            assert_eq!(guest.conn, 12);
            let event = HostEvent::Left(Box::new(guest));
            let host = game.server.lan.take().unwrap();
            game.server.lan = game.lan_process_events(host, [event], &[], [0.5, 64.0, 0.5]);
            let host = game.server.lan.as_ref().unwrap();
            assert!(!host.menus.contains_key(&12));
            assert!(!host.play.contains_key(&12));
            assert!(host.menus.contains_key(&13));
            assert!(matches!(
                ServerDecisionTarget::admit(game, 12),
                Err(Admission::MissingHolder)
            ));
            assert!(ServerDecisionTarget::admit(game, 13).is_ok());
        });
    }

    /// `effect_particle_modification` is asked once per effect of a mob and a
    /// guest, hidden ones included, and what it leaves is what players are
    /// sent: a replaced particle, the default again, a visible effect hidden
    /// and a hidden one shown, with the ambient flag decided from the result.
    /// With no listener, vanilla's list goes out. As vanilla, the mods are
    /// asked when an entity's effects change, not each tick, and their last
    /// answers go out until then (a guest whose effects just changed included).
    #[test]
    #[ignore = "daily"]
    fn a_mod_changes_the_effect_particles_players_are_sent() {
        use super::super::lan::LanSettings;
        use mod_api::{EffectParticleModificationEvent, Hooks, Host, ModEntry, Mods};
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicI32, AtomicUsize},
        };
        use sunrise_core::geometry::Vec3;
        use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEngine};
        use sunrise_level::effects::EffectInstance;
        use sunrise_protocol::entity_data::{Particle, ParticleData};
        use sunrise_protocol::packets::configuration::ClientInformation;
        use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        use {
            sunrise_level::particle_options::ParticleOptions,
            sunrise_level::particle_options::ParticleType,
        };
        const ZOMBIE: i32 = 40;
        const PLAYER: i32 = 12;
        // What the first mod was asked: the entity, the effect's name and
        // amplifier, the original and current particle, and `visible`.
        type Asked = (
            u32,
            String,
            i32,
            ParticleOptions,
            Option<ParticleOptions>,
            bool,
        );
        static ASKED: Mutex<Vec<Asked>> = Mutex::new(Vec::new());
        // MobEffects' colours as `entity_effect` takes them, with their alpha.
        let swirl = |alpha: u32, rgb: u32| ParticleData::Color(((alpha << 24) | rgb) as i32);
        let options = |alpha: f32, rgb: u32| {
            let channel = |shift: u32| ((rgb >> shift) & 255) as f32 / 255.0;
            ParticleOptions::colored(
                ParticleType::EntityEffect,
                [channel(16), channel(8), channel(0), alpha],
            )
        };
        const SPEED: u32 = 0x33_EB_FF;
        const STRENGTH: u32 = 0xFF_C7_00;
        const REGENERATION: u32 = 0xCD_5C_AB;
        const SLOWNESS: u32 = 0x8B_AF_E0;
        const WEAKNESS: u32 = 0x48_4D_48;
        const FAINT: f32 = 38.0 / 255.0;
        // Replaces, shows and hides, and tries to replace the omen.
        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn effect_particle_modification(
                &self,
                host: &dyn Host,
                event: &mut EffectParticleModificationEvent<'_>,
            ) {
                let id = event.entity.id();
                let name = event
                    .effect
                    .id
                    .id(host)
                    .expect("active effect is registered");
                assert_eq!(event.visible, event.effect.visible);
                ASKED.lock().unwrap().push((
                    id,
                    name.to_owned(),
                    event.effect.amplifier,
                    event.original_particle.try_into_engine(&()).unwrap(),
                    event.particle.try_into_engine(&()).unwrap(),
                    event.visible,
                ));
                match name {
                    "minecraft:speed" if id == ZOMBIE as u32 => {
                        *event.particle = Some(
                            ParticleOptions::simple(ParticleType::Flame)
                                .try_into_api()
                                .unwrap(),
                        )
                    }
                    "minecraft:trial_omen" => {
                        *event.particle = Some(
                            ParticleOptions::simple(ParticleType::Flame)
                                .try_into_api()
                                .unwrap(),
                        )
                    }
                    "minecraft:strength" | "minecraft:weakness" => event.visible = true,
                    "minecraft:regeneration" | "minecraft:slowness" => event.visible = false,
                    _ => {}
                }
            }
        }
        // Puts the omen back to its default after the first mod.
        struct Reset;
        #[mod_api::__subscriptions]
        impl Hooks for Reset {
            fn effect_particle_modification(
                &self,
                host: &dyn Host,
                event: &mut EffectParticleModificationEvent<'_>,
            ) {
                if event.effect.id.id(host) == Some("minecraft:trial_omen") {
                    assert_eq!(
                        *event.particle,
                        Some(
                            ParticleOptions::simple(ParticleType::Flame)
                                .try_into_api()
                                .unwrap()
                        )
                    );
                    *event.particle = None;
                }
            }
        }
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.server.level_shapes.clone(),
                level_seeds: game.server.levels.0.runtime.seeds(),
            };
            game.lan_host(settings, net, config).unwrap();
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 12,
                    uuid: 12,
                    name: "guest12".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                PLAYER,
                [3.0, 64.0, 5.0],
                10,
            );
            guest.placed = true;
            let mut faint_speed = EffectInstance::new("speed", 600, 0);
            faint_speed.ambient = true;
            guest.effects.list = vec![faint_speed.clone(), EffectInstance::new("slowness", 600, 0)];
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut zombie = Mob::new(ZOMBIE as u32, kind, Vec3::ZERO, 0.0);
            zombie.extra.effects.extend(
                [
                    ("speed", 1),
                    ("strength", 0),
                    ("trial_omen", 0),
                    ("regeneration", 0),
                ]
                .map(|(id, amplifier)| sunrise_sim::mobs::StoredEffect::new(id, 600, amplifier)),
            );
            zombie.extra.hidden_effects.push("strength");
            game.mobs.push(zombie);

            let builtin = super::sync::sync_data(
                &game.options.resources,
                &game.res.core.item_data,
                game.items(),
                &game.pack_files(),
            )
            .unwrap()
            .builtin
            .clone();
            let named = |p: &Particle| {
                let types = builtin.get("minecraft:particle_type").unwrap();
                (types.name(p.id).unwrap().to_owned(), p.data.clone())
            };
            let effect_entries = |meta: &[(u8, i32, Value)]| {
                (meta.iter())
                    .filter_map(|(index, _, value)| match value {
                        Value::Particles(list) if *index == 10 => {
                            Some((10, list.iter().map(|p| named(&p)).collect::<Vec<_>>()))
                        }
                        Value::Bool(ambient) if *index == 11 => {
                            assert!(*ambient);
                            Some((11, Vec::new()))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            // One pass of the entities players are sent, then the guest's
            // swirls as the hub sends them.
            let sent = |game: &mut crate::game::ServerGame<'_>| {
                let mut states = super::States::default();
                game.lan_entities(&mut states, false, 0, None);
                let zombie = states.as_slice().iter().find(|e| e.id == ZOMBIE).unwrap();
                let hub = &game.server.lan.as_ref().unwrap().hub;
                let listened =
                    mod_api::__private::listened(mod_api::HookId::EffectParticleModification);
                let player = hub.player_effect_meta(&hub.guests[0], listened);
                (effect_entries(&zombie.meta), effect_entries(&player))
            };
            let asked = || std::mem::take(&mut *ASKED.lock().unwrap());
            let particle = |name: &str, data| (format!("minecraft:{name}"), data);
            let entity_effect = |alpha, rgb| particle("entity_effect", Some(swirl(alpha, rgb)));
            let omen = particle("trial_omen", None);
            assert_eq!(
                sent(game),
                (
                    vec![(
                        10,
                        vec![
                            entity_effect(255, SPEED),
                            omen.clone(),
                            entity_effect(255, REGENERATION),
                        ],
                    )],
                    vec![(
                        10,
                        vec![entity_effect(38, SPEED), entity_effect(255, SLOWNESS)],
                    )],
                )
            );

            let mods: Mods = vec![
                ModEntry::new("effect-swirls", Box::new(Mod)),
                ModEntry::new("effect-reset", Box::new(Reset)),
            ]
            .into();
            mod_platform::testing::with_mods(mods, || {
                // The zombie's speed is replaced, its hidden strength shown,
                // its omen back at the default and its regeneration hidden,
                // and the strength it shows keeps the flag off. The guest's
                // slowness is hidden, which leaves only ambient swirls.
                let zombie_swirls = vec![(
                    10,
                    vec![
                        particle("flame", None),
                        entity_effect(255, STRENGTH),
                        omen.clone(),
                    ],
                )];
                let guest_swirls = vec![(10, vec![entity_effect(38, SPEED)]), (11, Vec::new())];
                assert_eq!(sent(game), (zombie_swirls.clone(), guest_swirls.clone()));
                let omen_options = ParticleOptions::simple(ParticleType::TrialOmen);
                let asked_for =
                    |id: i32, name: &str, amplifier, original: &ParticleOptions, visible| {
                        (
                            id as u32,
                            format!("minecraft:{name}"),
                            amplifier,
                            original.clone(),
                            Some(original.clone()),
                            visible,
                        )
                    };
                let mut first = asked();
                first.sort_by_key(|a| a.0);
                assert_eq!(
                    first,
                    [
                        asked_for(PLAYER, "speed", 0, &options(FAINT, SPEED), true),
                        asked_for(PLAYER, "slowness", 0, &options(1.0, SLOWNESS), true),
                        asked_for(ZOMBIE, "speed", 1, &options(1.0, SPEED), true),
                        asked_for(ZOMBIE, "strength", 0, &options(1.0, STRENGTH), false),
                        asked_for(ZOMBIE, "trial_omen", 0, &omen_options, true),
                        asked_for(ZOMBIE, "regeneration", 0, &options(1.0, REGENERATION), true),
                    ]
                );

                // Nothing changed: not asked again, and the answers still go
                // out. A duration counting down is no change either.
                assert_eq!(sent(game), (zombie_swirls.clone(), guest_swirls.clone()));
                assert!(asked().is_empty());
                let zombie = game
                    .mobs
                    .iter_mut()
                    .find(|m| m.id == ZOMBIE as u32)
                    .unwrap();
                zombie.extra.effects[0].duration -= 1;
                assert_eq!(sent(game), (zombie_swirls.clone(), guest_swirls.clone()));
                assert!(asked().is_empty());

                // The guest's slowness goes and a hidden weakness comes: their
                // answers still go out until the next pass, which asks about
                // both effects. The weakness the mod shows is visible and not
                // ambient, so the flag goes, where vanilla would have set it.
                let hub = &mut game.server.lan.as_mut().unwrap().hub;
                let mut weakness = EffectInstance::new("weakness", 600, 0);
                weakness.visible = false;
                hub.guests
                    .update(0, |g| g.effects.list = vec![faint_speed, weakness]);
                assert_eq!(
                    effect_entries(&hub.player_effect_meta(&hub.guests[0], true)),
                    guest_swirls
                );
                assert_eq!(
                    sent(game),
                    (
                        zombie_swirls.clone(),
                        vec![(
                            10,
                            vec![entity_effect(38, SPEED), entity_effect(255, WEAKNESS)],
                        )],
                    )
                );
                assert_eq!(
                    asked(),
                    [
                        asked_for(PLAYER, "speed", 0, &options(FAINT, SPEED), true),
                        asked_for(PLAYER, "weakness", 0, &options(1.0, WEAKNESS), false),
                    ]
                );

                // The guest's effects run out: nothing is kept or sent.
                let hub = &mut game.server.lan.as_mut().unwrap().hub;
                hub.guests.update(0, |g| g.effects.list.clear());
                assert_eq!(sent(game), (zombie_swirls, Vec::new()));
                assert!(asked().is_empty());
            });
        });
    }
}

#[cfg(test)]
pub(in crate::game) mod sources_tests {
    use super::sync::overlay_key;
    use sunrise_world::loot::PackOverlay;

    /// A pack set is told apart by its bytes, so a preload read without
    /// packs is not adopted by a world that has them.
    #[test]
    fn the_server_sources_key_follows_the_packs_bytes() {
        let files = vec![("recipe/a:b".to_owned(), vec![b"{}".to_vec()])];
        let changed = vec![("recipe/a:b".to_owned(), vec![b"{ }".to_vec()])];
        let key = |files: &[(String, Vec<Vec<u8>>)]| {
            overlay_key(&PackOverlay {
                conditions: &sunrise_world::conditions::LoadConditions::vanilla(),
                files,
                ..PackOverlay::default()
            })
        };
        assert_eq!(key(&files), key(&files.clone()));
        assert_ne!(key(&files), key(&changed));
        assert_ne!(key(&files), key(&[]));
    }
    include!("sulfur_cube_meta_tests.rs");
}

#[cfg(any(test, feature = "test-support"))]
pub mod world_spawn_sync_tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_sim::lan::{Joined, Out, PlayerSender, guest::Guest};

    pub fn host(game: &mut crate::game::ServerGame<'_>) -> mpsc::Receiver<Out> {
        host_for(game)
    }

    /// An integrated LAN host with one placed guest (uuid 1) in the Nether.
    pub fn host_for(game: &mut crate::game::ServerGame<'_>) -> mpsc::Receiver<Out> {
        game.lan_host(
            LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                listen: false,
                announce: false,
                ..NetConfig::loopback(0, String::new(), 2)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 20,
                integrated: true,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 1,
                uuid: 1,
                name: "SpawnTest".into(),
                properties: Vec::new(),
                info: sunrise_protocol::packets::configuration::ClientInformation::new(2),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            1,
            [0.5, 80.0, 0.5],
            2,
        );
        guest.placed = true;
        guest.dimension = "minecraft:the_nether".into();
        game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        rx
    }
}

pub(in crate::game) mod advancement_records;

#[cfg(test)]
include!("advancement_save_tests.rs");

#[cfg(test)]
mod late_pairing_tests {
    #[test]
    #[ignore = "daily"]
    fn late_paired_item_display_sends_its_stack_on_pairing() {
        use super::LanSettings;
        use mod_api::ItemDisplayContext;
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        use sunrise_core::geometry::Vec3;
        use sunrise_gameplay_common::game::commands_common::OtherEntity;
        use sunrise_items::items::ItemStack;
        use sunrise_protocol::entity_data::{Value, serializer};
        use sunrise_protocol::packets::{configuration::ClientInformation, play::Clientbound};
        use sunrise_sim::lan::{
            Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig,
        };
        use sunrise_sim::text_display_data::resident::{DisplayContents, DisplayState};

        crate::game::native_entity_refs::tests::with_game(|game| {
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.server.level_shapes.clone(),
                level_seeds: game.server.levels.0.runtime.seeds(),
            };
            game.lan_host(settings, net, config).unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 12,
                    uuid: 12,
                    name: "guest12".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                12,
                [3.0, 64.0, 5.0],
                10,
            );
            guest.placed = true;
            guest.dimension = game.dim.id().to_owned();
            guest.sent_chunks.insert((0, 0));
            let diamond_id = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .sync_data()
                .builtin
                .get("minecraft:item")
                .unwrap()
                .id("minecraft:diamond")
                .unwrap();
            {
                let host = game.server.lan.as_mut().unwrap();
                host.hub.guests.push(guest);
                host.states_next_entity = Some(game.server.next_entity);
            }
            let world = &mut game.common.world;
            if !world.chunks.contains_key(&(0, 0)) {
                let chunk = sunrise_level::chunk::Chunk::with_layout(0, 0, world.layout());
                world.chunks.insert((0, 0), Arc::new(chunk));
            }
            let display_id = game.server.next_entity + 1;
            game.server.next_entity = display_id;
            game.server.others.0.push(OtherEntity {
                attachment_lifetime: Default::default(),
                knot_check: None,
                size_answer: None,
                eye: None,
                join: Default::default(),
                display: Some(Arc::new(DisplayState::new(DisplayContents::Item {
                    stack: Some(Arc::new(ItemStack::new("minecraft:diamond", 3))),
                    context: ItemDisplayContext::Head,
                }))),
                interaction: None,
                id: display_id,
                kind: "minecraft:item_display".into(),
                position: Vec3::new(3.0, 64.0, 5.0),
                yaw: 0.0,
                pitch: 0.0,
            });
            game.lan_note_joined();

            let mut values = Vec::new();
            for out in rx.try_iter() {
                let bodies = match out {
                    Out::Packet(body) => vec![body],
                    Out::Packets(bodies) => bodies,
                    Out::Disconnect(_) => Vec::new(),
                };
                for body in bodies {
                    let mut reader = sunrise_protocol::buf::Reader::new(&body);
                    let id = reader.varint().unwrap();
                    if let Clientbound::SetEntityData(data) =
                        Clientbound::decode(id, reader.rest()).unwrap()
                        && data.id == display_id as i32
                    {
                        values.extend(data.values);
                    }
                }
            }
            assert!(values.iter().any(|entry| matches!(
                entry,
                (23, kind, Value::Item(Some(stack)))
                    if *kind == serializer::ITEM_STACK
                        && stack.item == diamond_id
                        && stack.count == 3
            )));
            assert!(values.contains(&(
                24,
                serializer::BYTE,
                Value::Byte(ItemDisplayContext::Head as i8),
            )));
        });
    }
}

#[cfg(test)]
pub(in crate::game) mod merchant_packet_tests {
    //! Vanilla notifyTrade changes the live offer while its frozen owner retains
    //! the last outbound offer packet. No sockets, server loop or world generation.

    use mod_api::HostTransaction;
    use sunrise_gameplay_common::game::{
        chat_common::ChatCommonGameExt1 as _, tables::GameItems as _,
    };

    pub(in crate::game) struct ReplaceOnTradeVoice {
        buyer: mod_api::PlayerRef,
        pub(in crate::game) abort: bool,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ReplaceOnTradeVoice {
        fn play_level_sound_at_position(
            &self,
            host: mod_api::HostSide<'_>,
            event: &mut mod_api::PlayLevelSoundAtPositionEvent,
        ) -> mod_api::Flow {
            if event.sound.to_string() == "minecraft:entity.villager.no" {
                let mod_api::HostSide::Server(host) = host else {
                    panic!("the trade voice is a server level sound");
                };
                let open = |host: &mut dyn mod_api::ServerHost| {
                    self.buyer
                        .open_chest_menu(
                            host,
                            1,
                            mod_api::Text::literal("Replacement"),
                            Vec::new(),
                            mod_api::MenuReplace::InPlace,
                        )
                        .unwrap();
                };
                if self.abort {
                    let aborted = host.transaction(|host, _| {
                        open(host);
                        Err::<(), _>(())
                    });
                    assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(())));
                } else {
                    open(host);
                }
            }
            mod_api::Flow::Continue
        }
    }

    #[test]
    #[ignore = "daily"]
    fn merchant_take_keeps_packet_snapshot_until_price_refresh() {
        use crate::game::native_entity_refs::tests::{
            bind_local_player, run_fresh_cases, with_game,
        };
        use std::sync::{Arc, atomic::AtomicI32, mpsc};
        use sunrise_core::{
            geometry::Vec3,
            random::{LegacyRandom, RandomSource},
        };
        use sunrise_protocol::packets::play::{self, Clientbound};
        use sunrise_sim::{
            merchant::TradeFeedback,
            mobs::{Damage, Mob},
        };

        const CHILD: &str = "SUNRISE_MERCHANT_HURT_PACKETS_CHILD";
        if run_fresh_cases(
            "game::server_side::lan::merchant_packet_tests::merchant_take_keeps_packet_snapshot_until_price_refresh",
            CHILD,
            &[
                "nonbuyer-first",
                "buyer-first",
                "sound-replaces-menu",
                "sound-replaces-suspended-menu",
                "sound-replaces-suspended-menu-abort",
            ],
        ) {
            return;
        }
        let case = std::env::var(CHILD).unwrap();
        let buyer_first = case == "buyer-first";
        use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest};

        with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);

            let mut sync = sunrise_sim::lan::data::SyncData::empty();
            for (registry, names) in [
                (
                    "minecraft:menu",
                    vec![
                        "minecraft:merchant".to_owned(),
                        "minecraft:generic_9x1".to_owned(),
                    ],
                ),
                (
                    "minecraft:entity_type",
                    vec!["minecraft:villager".to_owned()],
                ),
                (
                    "minecraft:damage_type",
                    vec!["minecraft:player_attack".to_owned()],
                ),
                (
                    "minecraft:item",
                    vec![
                        "minecraft:air".to_owned(),
                        "minecraft:emerald".to_owned(),
                        "minecraft:glass".to_owned(),
                        "minecraft:rabbit_foot".to_owned(),
                        "minecraft:glowstone".to_owned(),
                    ],
                ),
            ] {
                sync.builtin.registries.insert(
                    registry.to_owned(),
                    sunrise_protocol::registry::Ids::from_names(names),
                );
            }
            let sync = Arc::new(sync);
            let net = NetConfig {
                listen: false,
                announce: false,
                ..NetConfig::loopback(0, String::new(), 2)
            };
            let lan = sunrise_sim::lan::Lan::start_with(sync.clone(), net).unwrap();
            let encoder = sunrise_sim::lan::chunks::ChunkEncoder::new(
                [] as [(Arc<str>, bool); 0],
                &sunrise_protocol::registry::Builtin::default(),
                &[],
                &[],
                Default::default(),
            );
            let config = sunrise_sim::lan::hub::HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 10,
                simulation_distance: 10,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 20,
                integrated: true,
                levels: Default::default(),
                level_seeds: Default::default(),
            };
            let mut hub = sunrise_sim::lan::hub::Hub::new(
                lan,
                sync,
                encoder,
                config,
                game.loop_context.clone(),
            );
            let (tx, rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 1,
                    uuid: 2,
                    name: "Test".into(),
                    properties: Vec::new(),
                    info: sunrise_protocol::packets::configuration::ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                1,
                [0.5, 4.0, 0.5],
                8,
            );
            guest.placed = true;
            guest.dimension = "minecraft:overworld".into();
            guest.sent_chunks.insert((0, 0));
            if !game.common.world.chunks.contains_key(&(0, 0)) {
                game.common
                    .world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            }
            guest.inventory[0] = Some(sunrise_items::items::ItemStack::new("minecraft:emerald", 8));
            hub.guests.push(guest);
            game.server.lan = Some(super::LanHost {
                hub,
                spawn_joins: Vec::new(),
                players_told_left: false,
                settings: super::LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                play: Default::default(),
                menus: Default::default(),
                broken: Default::default(),
                mob_state: Default::default(),
                poses: Default::default(),
                mob_players: Default::default(),
                owner: None,
                waypoints: Default::default(),
                waypoint_dead: Default::default(),
                commands: Default::default(),
                quiet: Default::default(),
                far: None,
                native_commands_sent: None,
                sleep_ignored: Default::default(),
                states: Default::default(),
                states_next_entity: None,
                idle_states: Default::default(),
            });

            use sunrise_gameplay_common::game::trading_common::MerchantState;
            use sunrise_items::items::ItemStack;
            use sunrise_sim::merchant::{ItemCost, MerchantOffer, Session};
            let offer = MerchantOffer {
                base_cost_a: ItemCost {
                    stack: ItemStack::new("minecraft:emerald", 4),
                },
                cost_b: None,
                result: ItemStack::new("minecraft:glass", 3),
                uses: 0,
                max_uses: 100,
                reward_exp: false,
                special_price_diff: 0,
                demand: 0,
                price_multiplier: 0.0,
                xp: 1,
            };
            let mut session = Session::new(vec![offer], true);
            session.xp = 20;
            session.level = 2;
            let kind = game
                .res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone();
            let mut mob = sunrise_sim::mobs::Mob::new(
                77,
                kind,
                sunrise_core::geometry::Vec3::new(0.5, 4.0, 2.5),
                0.0,
            );
            mob.extra.trade = Some(Box::new(sunrise_sim::merchant::Trader {
                xp: 20,
                offers: session.offers.clone(),
                ..Default::default()
            }));
            game.mobs.push(mob);
            game.lan_open_merchant(
                1,
                MerchantState {
                    session: Box::new(session),
                    scroll: 0,
                    dragging: false,
                    focused: None,
                    mob: Some(77),
                    name: Some(sunrise_game_data::text::Text::literal("Packet timing")),
                },
            );
            fn packets(
                rx: &std::sync::mpsc::Receiver<sunrise_sim::lan::Out>,
            ) -> (
                Vec<sunrise_protocol::packets::play::MerchantOffers>,
                Vec<sunrise_protocol::packets::play::ContainerSetContent>,
            ) {
                let mut offers = Vec::new();
                let mut contents = Vec::new();
                for out in rx.try_iter() {
                    let sunrise_sim::lan::Out::Packet(body) = out else {
                        continue;
                    };
                    let mut reader = sunrise_protocol::buf::Reader::new(&body);
                    let id = reader.varint().unwrap();
                    match Clientbound::decode(id, reader.rest()).unwrap() {
                        sunrise_protocol::packets::play::Clientbound::MerchantOffers(packet) => {
                            offers.push(packet)
                        }
                        sunrise_protocol::packets::play::Clientbound::ContainerSetContent(
                            packet,
                        ) => contents.push(packet),
                        _ => {}
                    }
                }
                (offers, contents)
            }
            let initial = packets(&rx).0;
            assert_eq!(initial.len(), 1);
            assert_eq!(initial[0].offers[0].uses, 0);
            game.lan_select_trade(1, 0);
            assert!(
                packets(&rx).0.is_empty(),
                "selecting does not resend offers"
            );
            let window = initial[0].container;
            // Real ContainerClick on the merchant result slot, through the same
            // native path as the parity harness. Vanilla notifyTrade increments
            // uses/XP but sends no offers until the later reputation AI phase.
            let click = sunrise_protocol::packets::play::ContainerClick {
                container: window,
                state: 0,
                slot: 2,
                button: 0,
                input: 0,
                changed: Vec::new(),
                carried: None,
            };
            assert!(game.lan_merchant_click(1, &click));
            let state = &game.server.lan.as_ref().unwrap().menus[&1]
                .merchant
                .as_ref()
                .unwrap()
                .merchant
                .session;
            assert_eq!(state.offers[0].uses, 1);
            assert_eq!(state.xp, 21);
            let (offers, contents) = packets(&rx);
            assert!(
                offers.is_empty(),
                "a take leaves the last offer packet untouched"
            );
            let content = contents.last().expect("take sends real inventory contents");
            assert_eq!(content.items[0].as_ref().unwrap().count, 4);
            assert_eq!(content.items[2].as_ref().unwrap().count, 3);
            assert_eq!(content.carried.as_ref().unwrap().count, 3);
            let second = sunrise_protocol::packets::play::ContainerClick {
                state: content.state,
                ..click
            };
            assert!(game.lan_merchant_click(1, &second));
            let (offers, contents) = packets(&rx);
            assert!(offers.is_empty(), "the second take also keeps the snapshot");
            let content = contents.last().expect("second take sends contents");
            assert!(content.items[0].is_none());
            assert!(content.items[2].is_none());
            assert_eq!(content.carried.as_ref().unwrap().count, 6);
            let state = &game.server.lan.as_ref().unwrap().menus[&1]
                .merchant
                .as_ref()
                .unwrap()
                .merchant
                .session;
            assert_eq!(state.offers[0].uses, 2);
            assert_eq!(state.xp, 22);
            // An actual price refresh sends the snapshot even if only uses/XP
            // changed, and even a repeated refresh must reach the client.
            game.lan_merchant_reputation(0, None, TradeFeedback::Current);
            assert!(
                packets(&rx).0.is_empty(),
                "unrelated merchants do not refresh this menu"
            );
            game.lan_merchant_reputation(77, None, TradeFeedback::Current);
            let refreshed = packets(&rx).0;
            assert_eq!(refreshed.len(), 1);
            assert_eq!(refreshed[0].offers[0].uses, 2);
            assert_eq!(refreshed[0].xp, 22);
            game.lan_merchant_reputation(77, None, TradeFeedback::Current);
            assert_eq!(
                packets(&rx).0.len(),
                1,
                "price refresh sends even an unchanged offer"
            );
            game.lan_guest_packet(
                1,
                play::Serverbound::ContainerClose(play::ContainerClose { container: window }),
            );
            assert!(
                game.server.lan.as_ref().unwrap().menus[&1]
                    .merchant
                    .is_none()
            );
            assert!(packets(&rx).0.is_empty(), "closing does not resend offers");

            // Crossing the real level-two boundary runs updateSpecialPrices while
            // taking the result, so the new level's offers must reach the guest.
            let mut session = Session::new(
                game.mobs[0].extra.trade.as_ref().unwrap().offers.clone(),
                true,
            );
            session.offers[0].uses = 0;
            session.xp = 69;
            session.level = 2;
            session.profession = "minecraft:cleric".into();
            let sequences = Session::sequences_needed(
                &game.stations().trades,
                &session.profession,
                session.level,
            );
            assert!(!sequences.is_empty(), "prepared cleric trade sets exist");
            for id in sequences {
                let random = sunrise_sim::merchant::sequence_random(0, &id);
                session.sequences.insert(id, random);
            }
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.inventory[0] = Some(ItemStack::new("minecraft:emerald", 8));
                g.carried = None;
            });
            game.lan_open_merchant(
                1,
                MerchantState {
                    session: Box::new(session),
                    scroll: 0,
                    dragging: false,
                    focused: None,
                    mob: Some(77),
                    name: Some(sunrise_game_data::text::Text::literal(
                        "Level-up packet timing",
                    )),
                },
            );
            let opened = packets(&rx).0;
            assert_eq!(opened.len(), 1);
            game.lan_select_trade(1, 0);
            assert!(packets(&rx).0.is_empty());
            let click = sunrise_protocol::packets::play::ContainerClick {
                container: opened[0].container,
                state: 0,
                ..second
            };
            assert!(game.lan_merchant_click(1, &click));
            let (offers, contents) = packets(&rx);
            assert_eq!(offers.len(), 1, "level-up price refresh sends new offers");
            assert_eq!(offers[0].level, 3);
            assert_eq!(offers[0].xp, 70);
            assert_eq!(offers[0].offers[0].uses, 1);
            assert!(offers[0].offers.len() > opened[0].offers.len());
            let content = contents.last().expect("level-up take sends contents");
            assert_eq!(content.items[0].as_ref().unwrap().count, 4);
            assert_eq!(content.carried.as_ref().unwrap().count, 3);

            // `setLastHurtByMob` refreshes the matching buyer before event 13 and the
            // damage packet. Another player gets anger but no offer; mobs and lethal
            // blows show no anger. Keep the victim's same-tick cooldown throughout.
            let gm = game
                .server
                .lan
                .as_mut()
                .unwrap()
                .menus
                .get_mut(&1)
                .unwrap()
                .merchant
                .as_mut()
                .unwrap();
            assert!(
                gm.slots[2].is_some(),
                "the buyer could still afford the old price"
            );
            assert_eq!(gm.slots[0].as_ref().unwrap().count, 4);
            gm.merchant.session.offers[0].price_multiplier = 0.2;
            let merchant_window = gm.window;
            let mut states = super::States::default();
            game.lan_entities(&mut states, false, 0, None);
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.note_joined(states.as_slice(), None);
            assert!(hub.guests[0].tracker.is_tracking(77));
            rx.try_iter().for_each(drop);

            let at = [0.5, 4.0, 0.5];
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(at[0], at[1], at[2]))
                    .unwrap(),
            );
            bind_local_player(game, 300, 3);
            game.mobs[0].ambient_timer = 0;
            game.mobs[0].random = LegacyRandom::new(42);
            let mut random = LegacyRandom::new(42);
            let pitch = (random.next_float() - random.next_float()) * 0.2 + 1.0;
            if buyer_first {
                // The yes/no voice consumes the first pitch pair, then hurt consumes the second.
                random.next_float();
                random.next_float();
            }
            let record = || {
                let mut actual = Vec::new();
                for out in rx.try_iter() {
                    let sunrise_sim::lan::Out::Packet(body) = out else {
                        continue;
                    };
                    let mut reader = sunrise_protocol::buf::Reader::new(&body);
                    let id = reader.varint().unwrap();
                    match Clientbound::decode(id, reader.rest()).unwrap() {
                        Clientbound::MerchantOffers(packet) => {
                            let offer = &packet.offers[0];
                            actual.push(format!(
                                "offer:{}",
                                offer.cost_a.count + offer.special_price
                            ));
                        }
                        Clientbound::EntityEvent(packet)
                            if packet.id == 77 && packet.event == 13 =>
                        {
                            actual.push("entity:13".to_owned());
                        }
                        Clientbound::DamageEvent(packet) if packet.id == 77 => {
                            actual.push("damage".to_owned());
                        }
                        Clientbound::Sound(packet)
                            if packet.sound.1.as_ref().is_some_and(|(name, _)| {
                                name == "minecraft:entity.villager.no"
                            }) =>
                        {
                            assert_eq!(packet.pitch.to_bits(), pitch.to_bits());
                            actual.push("no".to_owned());
                        }
                        Clientbound::Sound(packet)
                            if packet.sound.1.as_ref().is_some_and(|(name, _)| {
                                name == "minecraft:entity.villager.hurt"
                            }) =>
                        {
                            actual.push("sound".to_owned());
                        }
                        Clientbound::ContainerSetSlot(packet)
                            if packet.container == merchant_window =>
                        {
                            assert_eq!(packet.slot, 2, "only the result slot changed");
                            assert!(packet.item.is_none());
                            actual.push("result:empty".to_owned());
                        }
                        Clientbound::ContainerSetContent(packet)
                            if packet.container == merchant_window =>
                        {
                            panic!("a price refresh must not send another player's inventory");
                        }
                        _ => {}
                    }
                }
                actual
            };
            let other_hit = |game: &mut crate::game::ServerGame<'_>, amount| {
                assert_eq!(
                    game.hurt_by_player(0, amount, Damage::player(at)),
                    (true, false)
                );
            };
            let buyer_hit = |game: &mut crate::game::ServerGame<'_>, amount| {
                assert_eq!(
                    game.as_guest(1, |g| g.hurt_by_player(0, amount, Damage::player(at))),
                    Some((true, false))
                );
            };
            if case.starts_with("sound-replaces-") {
                let buyer = game.server_player(2).expect("buyer player handle");
                let abort = case.ends_with("-abort");
                let mods = vec![mod_api::ModEntry::new(
                    "replace_on_trade_voice",
                    Box::new(ReplaceOnTradeVoice { buyer, abort }),
                )]
                .into();
                mod_platform::testing::with_mods(mods, || {
                    if case.starts_with("sound-replaces-suspended-menu") {
                        let mut inner = sunrise_gameplay_common::game::server_side::native_item_places::tests::guest(2);
                        inner.dimension = "minecraft:overworld".into();
                        game.server.lan.as_mut().unwrap().hub.guests.push(inner);
                        game.as_guest(1, |game| {
                            game.as_guest(2, |game| {
                                // A player-owned event (such as a cloud) can refresh prices
                                // even when that player isn't the current actor.
                                sunrise_sim::mobs::villager::ext_mut(&mut game.mobs[0])
                                    .gossips
                                    .add(
                                        [0, 0, 0, 2],
                                        sunrise_sim::gossip::GossipType::MinorNegative,
                                        sunrise_sim::gossip::REPUTATION_CHANGE_PER_EVENT,
                                    );
                                game.lan_merchant_reputation(
                                    77,
                                    Some([0, 0, 0, 2]),
                                    TradeFeedback::AtHurt(Some(1.0)),
                                );
                            })
                            .unwrap();
                        })
                        .unwrap();
                    } else {
                        buyer_hit(game, 1.0);
                    }
                });
                let host = game.server.lan.as_ref().unwrap();
                let buyer = host.hub.guest(1).unwrap();
                let emeralds = buyer
                    .inventory
                    .iter()
                    .flatten()
                    .filter(|item| item.item.as_ref() == "minecraft:emerald")
                    .map(|item| item.count)
                    .sum::<u32>();
                let (offers, contents) = packets(&rx);
                if abort {
                    assert!(
                        buyer.menu.is_none(),
                        "the aborted replacement did not remain open"
                    );
                    assert_eq!(
                        emeralds, 0,
                        "aborting did not duplicate the payment into inventory"
                    );
                    assert_eq!(
                        buyer.carried.as_ref().unwrap().count,
                        3,
                        "the old cursor was restored"
                    );
                    let gm = host.menus[&1]
                        .merchant
                        .as_ref()
                        .expect("merchant restored on abort");
                    assert_eq!(gm.window, merchant_window);
                    assert_eq!(gm.slots[0].as_ref().unwrap().count, 4);
                    assert!(
                        gm.slots[2].is_none(),
                        "the hit's earlier reprice still applies"
                    );
                    assert!(
                        contents.is_empty(),
                        "no tentative replacement snapshot escaped"
                    );
                    assert_eq!(
                        offers.len(),
                        1,
                        "the restored merchant window still receives its reprice"
                    );
                    assert_eq!(offers[0].container, merchant_window);
                    return;
                }
                let replacement = buyer.menu.as_ref().expect("sound hook opened a new window");
                assert_ne!(replacement.window, merchant_window);
                assert!(
                    host.menus[&1].merchant.is_none(),
                    "the old merchant window was retired"
                );
                assert_eq!(emeralds, 4, "the payment went back to the buyer");
                assert!(offers.is_empty(), "no offer packet for the retired window");
                let content = contents
                    .iter()
                    .find(|packet| packet.container == replacement.window)
                    .expect("replacement inventory snapshot");
                let emerald = host
                    .hub
                    .registry_id("minecraft:item", "minecraft:emerald")
                    .unwrap();
                assert_eq!(
                    content.items[9..]
                        .iter()
                        .flatten()
                        .filter(|item| item.item == emerald)
                        .map(|item| item.count)
                        .sum::<i32>(),
                    4
                );
                return;
            }
            // Use two accepted hits in one turn with no flush or tick between them.
            if buyer_first {
                buyer_hit(game, 1.0);
                other_hit(game, 3.0);
            } else {
                other_hit(game, 1.0);
                buyer_hit(game, 3.0);
            }
            assert_eq!(game.mobs[0].ambient_timer, -80);
            assert_eq!(game.mobs[0].random.clone().next_long(), random.next_long());
            let mut actual = record();
            assert!(!actual.iter().any(|event| event == "result:empty"));
            game.lan_flush_effects();
            assert!(
                record().is_empty(),
                "no hurt or slot packet waited for the effects flush"
            );
            game.lan_merchant_tick();
            let sync = record();
            assert_eq!(
                sync,
                ["result:empty"],
                "the result must reach the client at menu broadcast"
            );
            actual.extend(sync);
            game.lan_merchant_tick();
            assert!(record().is_empty(), "an unchanged result is not resent");
            let hub = &game.server.lan.as_ref().unwrap().hub;
            assert_eq!(hub.guests[0].carried.as_ref().unwrap().count, 3);
            let gm = game.server.lan.as_ref().unwrap().menus[&1]
                .merchant
                .as_ref()
                .unwrap();
            assert!(
                gm.slots[2].is_none(),
                "the buyer cannot afford the refreshed offer"
            );
            assert_eq!(
                gm.slots[0].as_ref().unwrap().count,
                4,
                "payment stayed in the menu"
            );
            let kind = game.mobs[0].kind.clone();
            game.mobs
                .push(Mob::new(78, kind.clone(), Vec3::new(1.0, 4.0, 1.0), 0.0));
            game.mobs
                .push(Mob::new(79, kind, Vec3::new(1.5, 4.0, 1.0), 0.0));
            game.admit_new_mobs(0);
            assert!(game.mob_hits_mob(77, 5.0, Damage::mob(78, at)));
            assert!(!game.mob_hits_mob(77, 2.0, Damage::mob(79, at)));
            assert!(game.mob_hits_mob(77, 7.0, Damage::mob(78, at)));
            bind_local_player(game, 300, 5);
            assert_eq!(
                game.hurt_by_player(0, 1000.0, Damage::player(at)),
                (true, true)
            );
            game.lan_flush_effects();
            actual.extend(record());
            let expected = match std::env::var_os("SUNRISE_REPUTATION_ORACLE") {
                Some(oracle) => std::fs::read_to_string(oracle)
                    .unwrap()
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix(if buyer_first {
                            "hurt_order_buyer "
                        } else {
                            "hurt_order "
                        })
                    })
                    .unwrap()
                    .to_owned(),
                None if buyer_first => {
                    "no,offer:9,entity:13,damage,sound,entity:13,result:empty".to_owned()
                }
                None => "entity:13,damage,sound,offer:9,entity:13,result:empty".to_owned(),
            };
            assert_eq!(actual.join(","), expected);
        });
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::native_hooks::block_entity_on_load::BlockEntityOnLoadGameExt1 as _;

use sunrise_gameplay_common::game::payload_transport::PayloadTransportGameExt1 as _;
