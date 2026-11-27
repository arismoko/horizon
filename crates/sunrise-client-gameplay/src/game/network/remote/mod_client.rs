use super::raw_backlog::Received;
use super::remote_chunks::{ChunkDecoder, ChunkFormat, Next, RemoteLight};
use super::render_tick::{mob_tracking_body, player_tracking_body};
use super::{Dim, Hit, Mining, Screen};
use mod_api::InteractionKey;
pub(in crate::game) use session_state::{
    BACKLOG_LIMIT, CHUNK_PACKETS_PER_FRAME, ClientMenu, ConstructingClientLevel, ItemUseAim,
    NoTerrain, OwnProfile, PendingCauldron, PendingItem, PlayerTextures, Remote,
    RemoteDimensionEntry, ServicesKeys, TICK_DECODE_WAIT, Tracked, client_millis, seat_passengers,
    skin_entries,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
#[allow(unused_imports)]
use sunrise_client::{
    hud::HudBatch, particles::TrackingTarget, screen_effects, streaming::MeshStreamer,
};
use sunrise_client_lib::net::{Event, Session};
use sunrise_core::{geometry::Vec3, profile::AccountProperty, random::RandomSource};
#[allow(unused_imports)]
use sunrise_game_data::{Result, text::Text};
use sunrise_gameplay_common::game::collision_source::Mover;
use sunrise_gameplay_common::game::native_hooks::base::GameHooks as _;
use sunrise_gameplay_common::game::remote::DryChanges;
use sunrise_gameplay_common::game::remote::DryPlace;
use sunrise_gameplay_common::game::remote::DrySound;
use sunrise_gameplay_common::game::remote::DryUse;
use sunrise_gameplay_common::game::remote::Lerp;
use sunrise_gameplay_common::game::remote::MOUNT_MENU;
use sunrise_gameplay_common::game::remote::SHIFT_INPUT;
use sunrise_gameplay_common::game::remote::boat_interact_passes;
use sunrise_gameplay_common::game::remote::chat_received::{QueuedChat, QueuedChatKind};
use sunrise_gameplay_common::game::remote::client_fluid_step;
use sunrise_gameplay_common::game::remote::client_view_distance;
use sunrise_gameplay_common::game::remote::dry_level::DryLevel;
use sunrise_gameplay_common::game::remote::face_direction;
use sunrise_gameplay_common::game::remote::hand_id;
use sunrise_gameplay_common::game::remote::hand_operated;
use sunrise_gameplay_common::game::remote::hold_server_fire;
use sunrise_gameplay_common::game::remote::input_ordinal;
use sunrise_gameplay_common::game::remote::inventory::{
    CREATIVE_SYNCED, Predicted, changed_slots, creative_sync_slot, creative_sync_stacks,
    mark_synced, menu_items, predict_clicks, predict_clicks_with,
};
use sunrise_gameplay_common::game::remote::item_frame_interact_passes;
use sunrise_gameplay_common::game::remote::keep_client_state;
use sunrise_gameplay_common::game::remote::look_at_rotation;
use sunrise_gameplay_common::game::remote::menu_title;
use sunrise_gameplay_common::game::remote::minecarts::CartLerp;
use sunrise_gameplay_common::game::remote::mob_pos_rot;
use sunrise_gameplay_common::game::remote::movement::MovePacket;
use sunrise_gameplay_common::game::remote::number_format;
use sunrise_gameplay_common::game::remote::object_skin_changed;
use sunrise_gameplay_common::game::remote::place_interpolated;
use sunrise_gameplay_common::game::remote::player_voice_pitch;
use sunrise_gameplay_common::game::remote::position_packet_pitch;
use sunrise_gameplay_common::game::remote::remote_object_swings;
use sunrise_gameplay_common::game::remote::remote_player_head_turn;
use sunrise_gameplay_common::game::remote::remote_text;
use sunrise_gameplay_common::game::remote::remote_use_swings;
use sunrise_gameplay_common::game::remote::selector_metadata::hurt_sound_for;
use sunrise_gameplay_common::game::remote::shape_meets;
use sunrise_gameplay_common::game::remote::snap_target;
use sunrise_gameplay_common::game::remote::synced_local_use;
use sunrise_gameplay_common::game::remote::use_ends_click;
use sunrise_gameplay_common::game::remote::wrap_degrees;
use sunrise_gameplay_common::game::remote::*;
use sunrise_gameplay_common::game::villager_client::builtin_registries;
use sunrise_gameplay_common::game::{item_use_common::ticks_left, level_sounds_common::LevelSound};
#[allow(unused_imports)]
use sunrise_items::{blocks::Fluid, items::ItemStack};
#[allow(unused_imports)]
use sunrise_level::{
    chunk::Chunk,
    containers::{BlockEntity, Furnace, FurnaceKind},
    profile_lookup::SPECTATOR,
    random_source,
    world::World,
};
use sunrise_protocol::{
    entity_data::{ITEM_ENTITY_ITEM, Value},
    item::{Hashed, Stack},
    packets::{
        Empty,
        play::{self, Clientbound as C, Serverbound as S},
    },
    registry::{DimensionType, Dynamic, SyncedNames, ceil_log2},
    secure_chat::{self, Signature},
    text::{plain, plain_with},
};
use sunrise_server_files::profiles::ServicesKeySet;
use sunrise_sim::first_person_sim::{Hand, SwingAnimation};
#[allow(unused_imports)]
use sunrise_sim::{
    entities::{Experience, ExperienceOrb, ItemEntity},
    join_seq::JoinSeq,
    lan::data::{BIOME_REGISTRY, client_biomes},
    menu::Input,
    mobs::Mob,
    motion::{MotionWorld, PlayerMotion},
    sprint::Keys,
};
use {
    sunrise_core::nbt::Tag,
    sunrise_mod_runtime::attachments::Holder,
    sunrise_world::{
        level::InteractionResult,
        registry::BlockStateId,
        stepped_interpolation::{Path, PosRot, Start, SteppedInterpolation},
    },
};

#[path = "selector_metadata_client.rs"]
pub(in crate::game) mod selector_metadata;

#[path = "attack_target_client.rs"]
pub(in crate::game) mod attack_target;

#[path = "break_prediction_client.rs"]
pub(in crate::game) mod break_prediction;

#[path = "connection_client.rs"]
pub(in crate::game) mod connection;

#[path = "fall_sound_client.rs"]
pub(in crate::game) mod fall_sound;

#[path = "general_client.rs"]
pub(in crate::game) mod general;

#[path = "interaction_actions_client.rs"]
pub(in crate::game) mod interaction_actions;

#[path = "interaction_prediction_client.rs"]
pub(in crate::game) mod interaction_prediction;

#[path = "level_format_client.rs"]
pub(in crate::game) mod level_format;

#[path = "raw_ingress_client.rs"]
pub(in crate::game) mod raw_ingress;
pub(in crate::game) use ingress_state::PacketError;

#[path = "recipe_properties_client.rs"]
pub(in crate::game) mod recipe_properties;

#[path = "use_item_client.rs"]
pub(in crate::game) mod use_item;

#[path = "block_entity_data_client.rs"]
pub(in crate::game) mod block_entity_data;

#[path = "abilities_client.rs"]
pub(in crate::game) mod abilities;

#[path = "chat_client.rs"]
pub(in crate::game) mod chat;

#[path = "chat_received_client.rs"]
pub(in crate::game) mod chat_received;

#[path = "commands_client.rs"]
pub(in crate::game) mod commands;

#[path = "client_level_tick_client.rs"]
pub(in crate::game) mod client_level_tick;

#[path = "effect_client.rs"]
pub(in crate::game) mod effect;

#[path = "entity_section_steps_client.rs"]
pub(in crate::game) mod entity_section_steps;

#[path = "inventory_client.rs"]
pub(in crate::game) mod inventory;

#[path = "item_use_client.rs"]
pub(in crate::game) mod item_use;

#[path = "kept_columns_client.rs"]
pub(in crate::game) mod kept_columns;

#[path = "menu_interactions_client.rs"]
pub(in crate::game) mod menu_interactions;

#[path = "minecarts_client.rs"]
pub(in crate::game) mod minecarts;

#[path = "motion_client.rs"]
pub(in crate::game) mod motion;

#[path = "movement_client.rs"]
pub(in crate::game) mod movement;

#[path = "packets_entity_client.rs"]
pub(in crate::game) mod packets_entity;

#[path = "packets_object_client.rs"]
pub(in crate::game) mod packets_object;

#[path = "packets_player_client.rs"]
pub(in crate::game) mod packets_player;

#[path = "packets_world_client.rs"]
pub(in crate::game) mod packets_world;

#[path = "remote_state_client.rs"]
pub(in crate::game) mod remote_state;

#[path = "session_client.rs"]
pub(in crate::game) mod session;

#[path = "skin_client.rs"]
pub(in crate::game) mod skin;

#[path = "status_client.rs"]
pub(in crate::game) mod status;

#[path = "sync_client.rs"]
pub mod sync;

#[path = "tables_client.rs"]
pub(in crate::game) mod tables;

#[path = "tick_client.rs"]
pub(in crate::game) mod tick;

#[path = "tracking_client.rs"]
pub(in crate::game) mod tracking;

#[path = "ui_client.rs"]
pub(in crate::game) mod ui;

#[path = "use_client.rs"]
pub(in crate::game) mod r#use;

#[path = "chat_queue_client.rs"]
pub(in crate::game) mod chat_queue;

#[path = "session_state.rs"]
pub mod session_state;

#[path = "break_prediction.rs"]
mod prediction_state;

#[path = "level_format.rs"]
mod format_state;

#[path = "raw_ingress.rs"]
pub mod ingress_state;

#[cfg(feature = "client")]
#[path = "remote_attributes.rs"]
mod remote_attributes;

#[path = "remote_state.rs"]
mod state_reads;

#[path = "session.rs"]
mod opening;

#[path = "sync.rs"]
mod sync_reads;

#[path = "tab_entries.rs"]
mod tab_entries;
#[cfg(test)]
pub(in crate::game) use tab_entries::listed_player_skins;

#[cfg(feature = "client")]
impl sunrise_client::interaction_input::InteractionWorld for RemoteInputWorld<'_, '_> {
    fn pick_hit_result(&mut self) {
        self.0.pick_tick_targets();
    }
    fn handle_keys(&mut self) {
        self.0.handle_play_keybinds();
    }
    fn pending_clicks(&self, button: sunrise_client::interaction_input::Button) -> u32 {
        self.0.pending_mapping_clicks(button.mapping())
    }
    fn consume_click(&mut self, button: sunrise_client::interaction_input::Button) -> bool {
        self.0.consume_mapping_click(button.mapping())
    }
    fn is_down(&self, button: sunrise_client::interaction_input::Button) -> bool {
        self.0.mapped_key_down(button.mapping())
    }
    fn can_continue_attack(&self) -> bool {
        !self.0.mod_mappings_screen_open()
    }
    fn spectator_hotbar(&mut self) {
        while self.0.own_player_is_spectator()
            && self.0.consume_mapping_click("key.spectatorHotbar")
        {
            self.0.spectator_action_key();
        }
    }
    fn release_hit_result(&mut self) {
        self.0.release_tick_targets();
    }
    fn using_item(&self) -> bool {
        self.0.client.client_item_use.0.is_some() || self.0.using.is_some()
    }
    fn release_using(&mut self) {
        if !self.0.end_client_item_use(true) && self.0.using.is_some() {
            self.0.release_remote_use();
        }
    }
    fn start_attack(&mut self) -> bool {
        self.0.remote_start_attack()
    }
    fn start_use(&mut self, held: bool) {
        let attempt = if held {
            mod_api::InteractionAttempt::Held
        } else {
            mod_api::InteractionAttempt::Press
        };
        self.0.remote_use(attempt);
    }
    fn pick(&mut self) {
        // Minecraft.pickBlockOrEntity: an entity before the block is asked
        // for (`handlePickItemFromEntity`), else the block.
        if let Some(hit) = self.0.crosshair_entity_hit() {
            if self
                .0
                .native_interaction(
                    InteractionKey::PickBlock,
                    Hand::Main,
                    mod_api::InteractionAttempt::Press,
                )
                .canceled
            {
                return;
            }
            self.0.send(S::PickItemFromEntity(play::PickItemFromEntity {
                id: hit.id,
                include_data: self.0.has_control_down(),
            }));
        } else if let Some(hit) = self.0.pick() {
            if self
                .0
                .native_interaction(
                    InteractionKey::PickBlock,
                    Hand::Main,
                    mod_api::InteractionAttempt::Press,
                )
                .canceled
            {
                return;
            }
            self.0.send(S::PickItemFromBlock(play::PickItemFromBlock {
                pos: hit.pos,
                include_data: self.0.has_control_down(),
            }));
        }
    }
    fn use_delay(&self) -> u32 {
        self.0.use_delay
    }
    fn continue_attack(&mut self, down: bool) {
        self.0.remote_continue_attack(down);
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub fn apply_packet(&mut self, packet: C) -> Result<()> {
        self.observe_client_packet(&packet);
        match packet {
            // ClientCommonPacketListenerImpl.handlePing runs on the game
            // thread, after all preceding packets (especially teleports).
            C::Ping(p) => self.send(S::Pong(p)),
            C::CustomPayload(p)
                if p.channel
                    == sunrise_gameplay_common::game::cauldron_prediction::ENTRIES_CHANNEL =>
            {
                let entries =
                    sunrise_gameplay_common::game::cauldron_prediction::decode_registrations(
                        &p.data,
                    )?;
                if let Some(remote) = &mut self.client.remote {
                    remote.cauldron_entries = Some(entries);
                }
            }
            C::CustomPayload(p)
                if p.channel == sunrise_gameplay_common::game::cauldron_prediction::USE_CHANNEL =>
            {
                self.receive_cauldron_use(&p.data)?;
            }
            // The server's next generation of mod registries: all of it or the connection.
            // Follows vanilla `ClientCommonPacketListenerImpl.onPacketError`: an invalid
            // snapshot or vanilla-value decode disconnects cleanly without replacing live state.
            C::CustomPayload(p) if p.channel == sunrise_sim::datapack_net::CHANNEL => {
                let decoded = if let Some(remote) = &mut self.client.remote {
                    remote.datapacks.decode_chunk(&p.data).map_err(|error| {
                        log::warn!("Sunrise: invalid datapack snapshot payload: {error}");
                        PacketError("Invalid datapack registry snapshot.")
                    })?
                } else {
                    None
                };
                if let Some(set) = decoded {
                    self.install_client_datapack_set(&set).map_err(|error| {
                        log::warn!("Sunrise: invalid datapack vanilla value: {error}");
                        PacketError("Invalid datapack vanilla value.")
                    })?;
                }
            }
            C::CustomPayload(p)
                if p.channel == sunrise_sim::lan::commands::selector_sync::CHANNEL =>
            {
                self.receive_selector_tokens(&p.data)?;
            }
            // Another level's world: its open, its packets and its close (`viewed_worlds`).
            C::CustomPayload(p) if p.channel == sunrise_protocol::viewed_levels::CHANNEL => {
                self.viewed_levels_payload(&p.data)?;
            }
            // Private custom recipe-book settings and placement protocol.
            C::CustomPayload(p)
                if p.channel == sunrise_sim::recipe_book::settings_sync::CHANNEL =>
            {
                self.remote_book_settings(&p.data);
            }
            // A native mod's general row the server starts showing the window.
            C::CustomPayload(p) if p.channel == sunrise_protocol::general_entity::CHANNEL => {
                self.remote_general_entity(&p.data);
            }
            // The collider boxes of an entity the server shows the window.
            C::CustomPayload(p) if p.channel == sunrise_protocol::collider_sync::CHANNEL => {
                self.remote_collider_sync(&p.data);
            }
            // Per-position connected-texture groups synced for the window's chunks.
            C::CustomPayload(p) if p.channel == sunrise_protocol::connection_groups::CHANNEL => {
                self.refuse_remote_input("connection groups: payload outside its bundle")?;
            }
            // Full-width data slots for the open blockless mod menu.
            C::CustomPayload(p) if p.channel == sunrise_protocol::menu_extra::DATA_CHANNEL => {
                use sunrise_protocol::menu_extra::{DataSlot, LIVE_DATA_REVISION};
                if self
                    .client
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.registries.menu_extra_revision >= LIVE_DATA_REVISION)
                    && let Ok(slot) = DataSlot::decode(&p.data)
                {
                    self.native_screen_data(slot.window, usize::from(slot.index), slot.value);
                }
            }
            // A mod menu type's opening context, just before its OpenScreen.
            C::CustomPayload(p) if p.channel == sunrise_protocol::menu_extra::CHANNEL => {
                self.receive_menu_extra(&p.data)?;
            }
            C::CustomPayload(p) if sunrise_sim::lan::recipes::viewer_channel(&p.channel) => {
                self.remote_recipe_viewer(&p.channel, &p.data);
            }
            // An anvil cost the data-slot short cannot hold, after that short.
            C::CustomPayload(p) if p.channel == sunrise_protocol::anvil_cost::CHANNEL => {
                self.remote_anvil_wide_cost(&p.data);
            }
            C::CustomPayload(p)
                if p.channel == sunrise_sim::lan::data::OPEN_COMMAND_BLOCK_CHANNEL =>
            {
                self.remote_open_command_block(&p.data);
            }
            // The server's recipes its native mods asked to send.
            C::CustomPayload(p) if p.channel == sunrise_sim::lan::recipes::CONTENT_CHANNEL => {
                self.remote_recipe_content(&p.data);
            }
            // The passages of the chunks the server shows the window.
            C::CustomPayload(p) if p.channel == sunrise_protocol::passage_sync::CHANNEL => {
                self.remote_passage_sync(&p.data);
            }
            // PingDebugMonitor.onPongReceived.
            C::PongResponse(p) => {
                let delay = sunrise_client_lib::net::ping_millis().wrapping_sub(p.time);
                self.client.debug_charts.ping.log(delay);
            }
            C::DebugSample(p) => self.client.debug_charts.log_remote_sample(&p.sample)?,
            C::Login(p) => self.apply_login_packet(p),
            C::Respawn(p) => self.apply_respawn_packet(p),
            C::PlayerPosition(p) => self.remote_teleport(&p)?,
            C::PlayerRotation(p) => self.remote_rotation(&p),
            C::PlayerLookAt(p) => self.remote_look_at(&p),
            // ClientCommonPacketListenerImpl.handlePostEffects.
            C::PostEffects(p) => self.apply_post_effects_packet(p)?,
            C::SetCamera(p) => self.remote_set_camera(p.camera_id),
            C::LevelChunkWithLight(p) => self.remote_chunk(&p, None),
            C::LightUpdate(p) => self.remote_light_update(&p),
            C::ChunksBiomes(p) => self.remote_chunks_biomes(&p),
            C::ForgetLevelChunk(p) => self.apply_forget_level_chunk_packet(p),
            C::BlockUpdate(p) => self.server_block(p.pos, p.state, None),
            C::AddTransientBlock(p) => self.add_transient_block(p.pos, p.state),
            C::BlockDestruction(p) => self.apply_block_destruction_packet(p),
            C::SectionBlocksUpdate(p) => {
                self.apply_section_blocks_update_packet(&p, Default::default())
            }
            C::BlockChangedAck(p) => self.end_predictions(p.sequence)?,
            C::BlockEvent(p) => self.apply_block_event_packet(p)?,
            // ClientLevel.levelEvent: LevelEventHandler, as this client's own
            // level plays them (particles and sounds: a spawner's burst, bone
            // meal, waxing, a block broken by someone else).
            // ClientPacketListener.handleLevelEvent: a global one is heard
            // from anywhere (ClientLevel.globalLevelEvent).
            C::LevelEvent(p) if p.global => self.global_level_event_sound(p.kind, p.pos),
            C::LevelEvent(p) => self.client_level_event_at(p.kind, p.pos, p.data),
            C::LevelParticles(p) => self.remote_level_particles(&p),
            C::Explode(p) => self.remote_explode(&p),
            C::Sound(p) => self.apply_sound_packet(p),
            C::SoundEntity(p) => self.apply_sound_entity_packet(p),
            C::StopSound(p) => self
                .client
                .audio
                .0
                .stop_sounds(p.sound.as_deref(), p.source.map(|s| s as usize)),
            C::AddEntity(p) => self.apply_add_entity_packet(p),
            C::RemoveEntities(p) => self.apply_remove_entities_packet(p),
            C::MoveEntityPos(play::MoveEntityPos(m))
            | C::MoveEntityPosRot(play::MoveEntityPosRot(m))
            | C::MoveEntityRot(play::MoveEntityRot(m)) => {
                // handleMoveEntity: the delta counts from the position codec's
                // base, which then moves to the delta's end.
                let base = self
                    .client
                    .remote
                    .as_ref()
                    .and_then(|r| r.entities.get(&m.id))
                    .map(|t| t.codec_base);
                let Some(base) = base else { return Ok(()) };
                // VecDeltaCodec.decode: each step's position from the last.
                let path = m.delta.as_ref().map(|d| match d {
                    play::VecDelta::Linear(_) => Path::Linear(d.end(base)),
                    play::VecDelta::Stepped(steps) => {
                        let mut at = base;
                        Path::Stepped(
                            steps
                                .iter()
                                .map(|(delta, ticks)| {
                                    at = play::VecDelta::apply(at, *delta);
                                    (at, *ticks)
                                })
                                .collect(),
                        )
                    }
                });
                let rotation = m.rotation.map(|(y, x)| {
                    (
                        sunrise_protocol::buf::unpack_degrees(y),
                        sunrise_protocol::buf::unpack_degrees(x),
                    )
                });
                if let (Some(path), Some(t)) = (
                    &path,
                    self.client
                        .remote
                        .as_mut()
                        .and_then(|r| r.entities.get_mut(&m.id)),
                ) {
                    t.codec_base = path.end();
                }
                self.remote_move_entity_and_file(m.id, path, rotation, m.on_ground);
            }
            C::EntityPositionSync(p) => self.apply_entity_position_sync_packet(p),
            C::TeleportEntity(p) => self.apply_teleport_entity_packet(p)?,
            C::SetPassengers(p) => self.remote_set_passengers(p.vehicle, p.passengers),
            C::SetEntityLink(p) => self.apply_set_entity_link_packet(p),
            C::MoveVehicle(p) => self.apply_move_vehicle_packet(p),
            C::MoveMinecartAlongTrack(p) => self.remote_move_minecart(p),
            C::RotateHead(p) => self.apply_rotate_head_packet(p),
            C::SetEntityData(p) => self.apply_set_entity_data_packet(p)?,
            C::SetEquipment(p) => self.apply_set_equipment_packet(p),
            C::TakeItemEntity(p) => self.apply_take_item_entity_packet(p),
            C::SetEntityMotion(p) => self.apply_set_entity_motion_packet(p)?,
            C::ProjectilePower(p) => self.apply_projectile_power_packet(p),
            C::DamageEvent(p) => self.apply_damage_event_packet(p),
            C::HurtAnimation(p) => self.apply_hurt_animation_packet(p),
            C::EntityEvent(p) => self.apply_entity_event_packet(p)?,
            C::Animate(p) => self.remote_animate(p.id, p.action),
            C::SwingAnimation(p) => self.apply_swing_animation_packet(p),
            C::UpdateAttributes(p) => self.apply_update_attributes_packet(p)?,
            C::UpdateMobEffect(p) => self.remote_effect(&p),
            C::RemoveMobEffect(p) => self.apply_remove_mob_effect_packet(p),
            C::ChangeDifficulty(p) => {
                self.common.ui.difficulty = p.difficulty.clamp(0, 3) as u8;
                self.common.level.difficulty_locked = p.locked;
                self.remote_world_options_difficulty(self.common.ui.difficulty, p.locked);
            }
            C::GameRuleValues(p) => self.remote_world_options_rules(p.entries),
            C::SetHealth(p) => self.apply_set_health_packet(p),
            C::PlayerCombatKill(p) => self.apply_player_combat_kill_packet(p)?,
            C::TrackedWaypoint(p) => self.handle_waypoint(&p),
            C::SetExperience(p) => self.apply_set_experience_packet(p),
            C::SetTime(p) => self.apply_set_time_packet(p),
            C::GameEvent(p) => self.apply_game_event_packet(p),
            C::PlayerAbilities(p) => self.apply_player_abilities_packet(p),
            C::SetHeldSlot(p) => self.apply_set_held_slot_packet(p),
            C::ContainerSetContent(p) => self.apply_container_set_content_packet(p),
            C::ContainerSetSlot(p) => self.apply_container_set_slot_packet(p),
            C::SetCursorItem(p) => self.apply_set_cursor_item_packet(p),
            C::SetPlayerInventory(p) => self.apply_set_player_inventory_packet(p),
            C::ContainerSetData(p) => self.apply_container_set_data_packet(p)?,
            C::OpenScreen(p) => self.remote_open_screen(p),
            C::MountScreenOpen(p) => self.remote_open_mount(p)?,
            C::UpdateRecipes(p) => self.remote_recipe_properties(p),

            C::RecipeBookAdd(p) => self.remote_recipe_book_add(p),
            C::RecipeBookRemove(p) => self.remote_recipe_book_remove(p),
            C::RecipeBookSettings(p) => self.remote_recipe_book_settings(p),
            C::PlaceGhostRecipe(p) => self.remote_ghost_recipe(p),
            C::MerchantOffers(p) => self.remote_merchant_offers(p),
            C::ContainerClose(_) => {
                let show_death = self
                    .client
                    .remote
                    .as_ref()
                    .is_none_or(|r| r.show_death_screen);
                self.close_screen_to_level(show_death)?;
            }
            C::TagQuery(p) => self.answer_tag_query(p.transaction, p.tag),
            C::CommandSuggestions(p) => self.handle_command_suggestions(p),
            C::CustomChatCompletions(p) => {
                // ClientPacketListener.handleCustomChatCompletions.
                if let Some(r) = self.client.remote.as_mut() {
                    sunrise_command::modify_custom_completions(
                        &mut r.custom_completions,
                        sunrise_command::CustomCompletionAction::from_id(p.action),
                        &p.entries,
                    );
                    self.common.ui.registries.custom_completions = r.custom_completions.clone();
                }
            }
            C::UpdateTags(p) => self.remote_update_tags(&p),
            C::Commands(p) => self.apply_commands_packet(p),
            C::Cooldown(p) => {
                // ClientPacketListener.handleItemCooldown: the server starts
                // (or confirms) a group's cooldown; 0 ends it.
                let group: Arc<str> = Arc::from(p.group.as_str());
                if p.duration == 0 {
                    self.common.cooldowns.remove(&group);
                } else {
                    self.common.cooldowns.add(group, p.duration);
                }
            }
            // ClientPacketListener.handleGameTestHighlightPos.
            C::GameTestHighlightPos(p) => self.highlight_test_pos(p.absolute, p.relative),
            // ClientPacketListener.handleTestInstanceBlockStatus.
            C::TestInstanceBlockStatus(p) => {
                self.block_editor_status(remote_text(&p.status), p.size);
            }
            C::SystemChat(p) => self.apply_system_chat_packet(p)?,
            C::PlayerChat(p) => self.apply_player_chat_packet(p)?,
            C::DisguisedChat(p) => self.apply_disguised_chat_packet(p),
            C::DeleteChat(p) => self.handle_delete_chat(&p.signature)?,
            C::PlayerInfoUpdate(p) => self.apply_player_info_update_packet(p),
            C::PlayerInfoRemove(p) => self.apply_player_info_remove_packet(p),
            C::TabList(p) => self.apply_tab_list_packet(p),
            C::InitializeBorder(p) => {
                if let Err(error) = self.apply_initialize_border_packet(p) {
                    log::warn!("invalid initialize_border packet: {error}");
                }
            }
            C::SetBorderCenter(p) => {
                if let Err(error) = self.common.border.set_center([p.x, p.z]) {
                    log::warn!("invalid set_border_center packet: {error}");
                }
            }
            C::SetBorderLerpSize(p) => {
                if let Err(error) =
                    self.common
                        .border
                        .lerp_between(p.old_size, p.new_size, p.lerp_time)
                {
                    log::warn!("invalid set_border_lerp_size packet: {error}");
                }
            }
            C::SetBorderSize(p) => {
                if let Err(error) = self.common.border.set_size(p.size) {
                    log::warn!("invalid set_border_size packet: {error}");
                }
            }
            C::SetBorderWarningDelay(p) => self.common.border.warning_time = p.delay,
            C::SetBorderWarningDistance(p) => self.common.border.warning_distance = p.blocks,
            C::SetObjective(p) => self.remote_objective(p),
            C::SetDisplayObjective(p) => self.apply_set_display_objective_packet(p),
            C::SetScore(p) => self.apply_set_score_packet(p),
            C::ResetScore(p) => match &p.objective {
                Some(o) => self.common.score.board.reset_one(&p.owner, o),
                None => self.common.score.board.reset_all(&p.owner),
            },
            C::SetPlayerTeam(p) => self.remote_team(p),
            C::SetSimulationDistance(p) => {
                if let Some(r) = self.client.remote.as_mut() {
                    r.simulation_distance = p.distance;
                }
            }
            C::SetChunkCacheRadius(p) => self.set_remote_chunk_cache_radius(p.distance),
            C::SetChunkCacheCenter(p) => self.set_remote_chunk_cache_center((p.x, p.z)),
            C::BlockEntityData(p) => self.remote_block_entity_packet(p),
            C::OpenSignEditor(p) => self.remote_open_sign_editor(p),
            C::OpenBook(p) => self.remote_open_book(p.hand),
            C::SetDefaultSpawnPosition(p) => self.apply_set_default_spawn_position_packet(p),
            C::ServerData(p) => self.remote_server_data(p.icon.as_deref()),
            C::MapItemData(p) => self.remote_map_item_data(&p),
            // handleTickingState / handleTickingStep do nothing without a ClientLevel.
            C::TickingState(p) if self.client.remote.as_ref().is_some_and(Remote::has_level) => {
                self.common.clock.set_rate(p.rate);
                self.common.clock.set_frozen(p.frozen);
            }
            C::TickingStep(p) if self.client.remote.as_ref().is_some_and(Remote::has_level) => {
                self.common.clock.set_frozen_ticks_to_run(p.steps);
            }
            C::TickingState(_) | C::TickingStep(_) => {}
            C::UpdateAdvancements(p) => self.remote_advancements(p),
            C::AwardStats(p) => self.apply_award_stats_packet(p)?,
            // ClientPacketListener.setActionBarText, setTitleText,
            // setSubtitleText, setTitlesAnimation and handleTitlesClear.
            C::BossEvent(p) => self.apply_boss_event(&p),
            C::SetActionBarText(p) => self.client.titles.set_overlay(remote_text(&p.text), false),
            C::SetTitleText(p) => self.client.titles.set_title(remote_text(&p.text)),
            C::SetSubtitleText(p) => self.client.titles.set_subtitle(remote_text(&p.text)),
            C::SetTitlesAnimation(p) => self.client.titles.set_times(p.fade_in, p.stay, p.fade_out),
            // ClientCommonPacketListenerImpl.handleShowDialog / handleClearDialog.
            C::ShowDialog(p) => self.show_play_dialog(&p.dialog),
            C::ClearDialog(_) => self.clear_dialog(),
            // ClientPacketListener.handleLowDiskSpaceWarning.
            C::LowDiskSpaceWarning(_) => self.client.client_reports.0.report_low_disk_space(),
            C::ClearTitles(p) => self.apply_clear_titles_packet(p),
            _ => {}
        }
        Ok(())
    }
    /// One vanilla active cache: `(2 * (max(2, client_view_distance) + 3) + 1)^2` columns
    /// (`ClientChunkCache.calculateStorageRange`), times the active dimension's sections,
    /// times 4,096 cells. Viewed levels share that footprint; a legitimate server need not
    /// group more cells than one full active world.
    /// The server's view-radius packet and per-view column caps cannot grow the ceiling.
    pub(in crate::game) fn connection_group_limit(&self) -> usize {
        self.client.remote.as_ref().map_or(0, |remote| {
            let radius = remote.sent_info.view_distance.max(2) as usize + 3;
            let width = 2 * radius + 1;
            (width * width).saturating_mul(self.common.world.shape.section_count() * 4096)
        })
    }
    pub(in crate::game) fn decode_remote_stack(&self, stack: &Stack) -> Option<ItemStack> {
        let r = self.client.remote.as_ref()?;
        if !sunrise_protocol::item::custom::permitted_with_names(
            stack,
            r.registries.custom_components,
            &*r.registries,
        )
        .unwrap_or(false)
        {
            log::warn!("Dropping a stack with unnegotiated custom components");
            return None;
        }
        Some(sunrise_sim::item_sync::from_network(
            stack,
            &r.builtin,
            &r.registries,
        ))
    }
    /// `ServerReconfigScreen` over the panorama; Disconnect wakes up after
    /// `DISCONNECT_TIME` (600 ticks).
    pub(in crate::game::client_side) fn draw_reconfiguring(
        &self,
        batch: &mut HudBatch,
        since: u64,
    ) {
        let atlas = &self.client.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        sunrise_client::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
        let title = sunrise_game_data::text::Text::translate("connect.reconfiguring", &[]);
        let ((tx, ty), (bx, by)) = self.reconfigure_layout((w, h));
        batch.text_styled(atlas, &title, tx, ty, u32::MAX, true);
        // CommonComponents.GUI_DISCONNECT
        let label = sunrise_game_data::text::Text::translate("menu.disconnect", &[]);
        let active = self.common.ticks.saturating_sub(since) >= 600;
        let hover = sunrise_client::ui::hovered(self.client.cursor, bx, by, 150.0, 20.0);
        sunrise_client::ui::button(batch, atlas, bx, by, 150.0, 20.0, &label, active, hover);
    }
    /// `HashedStack.create` with the connection's registries (the client's
    /// `decoratedHashOpsGenerator`).
    pub fn hashed_stack(&self, s: &Option<ItemStack>) -> sunrise_protocol::Result<Option<Hashed>> {
        if s.is_none() {
            return Ok(None);
        }
        let r = self
            .client
            .remote
            .as_ref()
            .ok_or_else(|| sunrise_protocol::Error::new("no live menu connection"))?;
        let names = SyncedNames {
            builtin: &r.builtin,
            dynamic: &r.registries,
        };
        let projected = std::cell::RefCell::new(Vec::new());
        let stack = self
            .to_network_projecting(s, &|effect| projected.borrow_mut().push(effect.clone()))
            .ok_or_else(|| sunrise_protocol::Error::new("cannot encode a nonempty stack"))?;
        let projected = projected.into_inner();
        let hash = if let Some(data) = s.as_ref().and_then(ItemStack::data) {
            Hashed::create_with_admitted(&stack, &names, &|name, bytes| {
                let matches =
                    |effect: &sunrise_world::component_value::consume_effects::Admitted| {
                        &effect.name == name && effect.wire.as_ref() == bytes
                    };
                if stack.wire.0.is_some() {
                    data.wire
                        .admitted_effects()
                        .iter()
                        .find(|effect| matches(effect))
                        .cloned()
                } else {
                    data.find_admitted_consume_effect(&matches)
                        .or_else(|| projected.iter().find(|effect| matches(effect)).cloned())
                }
            })
        } else {
            Hashed::create(&stack, &names)
        };
        hash.map(Some)
            .map_err(|e| sunrise_protocol::Error::new(e.to_string()))
    }
    pub(in crate::game) fn item_name(&self, id: i32) -> Arc<str> {
        let r = self.client.remote.as_ref().expect("remote");
        Arc::from(
            r.builtin
                .get("minecraft:item")
                .and_then(|i| i.name(id))
                .unwrap_or("minecraft:air"),
        )
    }
    pub(in crate::game) fn next_sequence(&mut self) -> i32 {
        let r = self.client.remote.as_mut().expect("remote");
        r.sequence += 1;
        r.sequence
    }
    /// The latency the server's player list gives this player
    /// (`PlayerInfo.getLatency`), none before the server lists it. A new entry reads 0 until the
    /// server's first latency update (every 600 ticks).
    pub(in crate::game) fn own_latency(&self) -> Option<i32> {
        let r = self.client.remote.as_ref()?;
        r.tab_players.get(&r.profile).map(|me| me.latency)
    }
    /// Whether the server's patch gives `stack`, an item with no consumable
    /// of its own, a `consumable` with a use time or `blocks_attacks` (a
    /// sword made to block): `Item.use` starts a use of it
    /// (`Consumable.startConsuming`, `startUsingItem`) that lasts until the
    /// button lets go or the server stops it, as the client's
    /// `LivingEntity.updateUsingItem` never completes one.
    pub(in crate::game::client_side) fn patched_held_use(&self, stack: &ItemStack) -> bool {
        let (ticks, blocks) = sunrise_sim::item_sync::patched_use(stack);
        ticks.map_or(blocks, |ticks| ticks > 0)
            && self
                .items()
                .get(&stack.item)
                .is_none_or(|i| i.consumable.is_none())
    }
    pub(in crate::game) fn play_predicted_sounds(&mut self, sounds: &[DrySound]) {
        for (pos, sound, source, volume, pitch) in sounds {
            let at = [
                f64::from(pos.x) + 0.5,
                f64::from(pos.y) + 0.5,
                f64::from(pos.z) + 0.5,
            ];
            let name = if sound.contains(':') {
                sound.to_string()
            } else {
                format!("minecraft:{sound}")
            };
            self.play_client_local_sound(
                LevelSound::new(&name, at, *volume, *pitch).in_source(*source),
            );
        }
    }
    /// The player's position, for `retainKnownServerState`'s `player.position()`.
    pub(in crate::game) fn player_position(&self) -> [f64; 3] {
        let at = self
            .common
            .player
            .as_ref()
            .map(|p| p.position)
            .unwrap_or(Vec3::ZERO);
        [at.x, at.y, at.z]
    }
    /// Process what the network thread received (once per frame, like the
    /// client's packet processor), in order. Chunk packets get
    /// `CHUNK_PACKETS_PER_FRAME`: once a frame has spent it, the events from
    /// the next chunk packet on wait for the next frame, unless they have
    /// already waited `BACKLOG_LIMIT`. A chunk packet still decoding holds
    /// back what follows it.
    pub fn poll_remote(&mut self) -> Result<()> {
        self.poll_remote_with(false)
    }
    /// `Minecraft.runTick` handles every packet that came (`runAllTasks`)
    /// before the frame's ticks. What this frame's poll left for later (chunks
    /// past its budget or still decoding, and everything after them) is taken
    /// when a tick is about to run, so the tick moves through those chunks, as
    /// vanilla's does. A batch's `ChunkBatchReceived` is not among them: the
    /// network thread answers it (`net::receive_play`).
    pub fn poll_remote_before_tick(&mut self) -> Result<()> {
        if self
            .client
            .remote
            .as_ref()
            .is_none_or(|r| r.backlog.is_empty())
        {
            return Ok(());
        }
        self.poll_remote_with(true)
    }
    pub(in crate::game) fn poll_remote_with(&mut self, before_tick: bool) -> Result<()> {
        let polled = self.poll_remote_events(before_tick);
        // The block events and block changes of a piston clock start and end
        // thousands of moves at once: the collision view and the renderer
        // take them all in one sync, before the tick or frame reads them.
        if std::mem::take(&mut self.client.moving_pistons_stale) {
            self.sync_moving_pistons();
        }
        polled
    }
    /// `MapItem.useOn` on this client: a map used on a banner is SUCCESS
    /// whatever its data says (the server toggles the marker, or fails
    /// quietly), so this hand swings and the click ends. The caller applies
    /// ItemStack.useOn's adventure predicate before selecting this phase.
    pub(in crate::game::client_side) fn predict_map_on_banner(
        &self,
        stack: &ItemStack,
        hit: &super::Hit,
    ) -> bool {
        sunrise_gameplay_common::game::maps::map_id(stack).is_some()
            && self.named_block_in_tag("minecraft:banners", &self.block_name(hit.pos))
    }
    /// MultiPlayerGameMode.useItemOn inside startPrediction: the blocks the
    /// use changed on this client's level at once, each kept as predicted
    /// under the use's sequence until the server acknowledges it (a use the
    /// server refused goes back). Only the blocks a hand opens or flips: a
    /// wooden or copper door, trapdoor or fence gate, a lever or a button;
    /// whatever else the use changes waits for the server.
    pub fn predict_use(&mut self, (changes, sounds): &DryUse, seq: i32) {
        let behaviours = self.common.res.behaviours();
        let predicted = |state: BlockStateId| hand_operated(behaviours, state);
        if changes.is_empty()
            || !changes
                .iter()
                .all(|(p, s)| predicted(*s) && predicted(self.common.world.block(p.x, p.y, p.z)))
        {
            return;
        }
        self.predict_blocks(changes, seq);
        // `level.playSound(player, ...)`: the clicker hears it from its own use; the
        // server sends it to the others only.
        self.play_predicted_sounds(sounds);
    }
    /// `ServerReconfigScreen`: a vertical layout of cells padded by 10, the
    /// title (9 high) over Disconnect (150 x 20), centred. The title's and the
    /// button's top left corners.
    pub fn reconfigure_layout(&self, (w, h): (f32, f32)) -> ((f32, f32), (f32, f32)) {
        let title = sunrise_game_data::text::Text::translate("connect.reconfiguring", &[]);
        let title_w = self.client.gui.atlas.font.width_text(&title).ceil() as i32;
        let width = (title_w + 20).max(170);
        let left = ((w as i32 - width) as f32 * 0.5) as i32;
        let top = ((h as i32 - 69) as f32 * 0.5) as i32;
        let across = |child: i32| (10.0 + ((width - child - 10) as f32 - 10.0) * 0.5) as i32;
        (
            ((left + across(title_w)) as f32, (top + 10) as f32),
            ((left + across(150)) as f32, (top + 39) as f32),
        )
    }
    /// Live client `Abilities` (`LocalPlayer.abilities`) when connected to a server.
    pub fn remote_abilities(&self) -> Option<&sunrise_sim::lan::abilities::PlayerAbilities> {
        self.client.remote.as_ref().map(|r| &r.abilities)
    }
    /// The attack clicks queued for the next tick (`KeyMapping.click`, not yet handled).
    pub(in crate::game) fn remote_attack_clicks_queued(&self) -> u32 {
        self.pending_mapping_clicks("key.attack")
    }
    /// Received events left for the next frame (`poll_remote`): until they
    /// are handled, the terrain the server sent is not all there.
    pub(in crate::game) fn remote_backlog(&self) -> usize {
        self.client.remote.as_ref().map_or(0, |r| r.backlog.len())
    }
    /// `BoneMealItem.useOn` with the acting hand's `stack` on this client's
    /// level (a copy: nothing grows here): whether it succeeds.
    pub(in crate::game::client_side) fn remote_bone_meal_succeeds(
        &mut self,
        hit: &super::Hit,
        stack: &ItemStack,
    ) -> bool {
        let pos = sunrise_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
        let face = sunrise_core::geometry::Direction::ALL[face_direction(hit.face) as usize];
        let (ask, _) = self.held_bonemeal(pos, stack);
        self.with_level(|b, level, _| {
            b.bone_meal_succeeds_on_client(
                &mut DryLevel::new(level),
                pos,
                face,
                ask.crop,
                ask.check,
            )
        })
    }
    /// The minecarts the server sent, for the control socket's `status`
    /// (their velocity is not tracked here).
    pub(in crate::game::client_side) fn remote_carts(&self) -> Option<Vec<serde_json::Value>> {
        let r = self.client.remote.as_ref()?;
        let mut carts: Vec<&Tracked> = r
            .entities
            .values()
            .filter(|t| t.kind.ends_with("minecart"))
            .collect();
        carts.sort_by(|a, b| {
            a.position
                .partial_cmp(&b.position)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Some(
            carts
                .into_iter()
                .map(|t| serde_json::json!({"kind": t.kind, "position": t.position, "velocity": [0.0, 0.0, 0.0]}))
                .collect(),
        )
    }
    /// MultiPlayerGameMode.dropItem (key.drop): predict the removal, then send
    /// the action; swing only if a nonempty stack was removed.
    pub(in crate::game) fn remote_drop(&mut self, all: bool) {
        // ensureHasSentCarriedItem: the server drops from the slot just chosen.
        self.remote_send_carried();
        // MultiPlayerGameMode.dropItem predicts the removal and sends only the drop: the
        // server's stack agrees, so no creative slot edit follows at the end of the tick.
        let before = self.creative_stacks();
        let dropped = sunrise_items::items::take_held_drop(
            &mut self.common.inventory[self.common.selected],
            all,
            false,
        );
        self.remote_synced_since(&before);
        self.send_action(if all { 4 } else { 5 });
        if dropped.is_some() {
            self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
        }
    }
    /// The hurt sound the hurt player's own client plays from
    /// `ClientboundDamageEventPacket` (`LivingEntity.handleDamageEvent`):
    /// `Player.getHurtSound` is the damage type's effects, with the codec's
    /// `hurt` default when the type is unknown.
    pub(in crate::game::client_side) fn remote_hurt_sound(&self, source_type: i32) -> &'static str {
        match self.client.remote.as_ref() {
            Some(r) => hurt_sound_for(
                &r.registries,
                &self.common.health_resources.damage,
                source_type,
            ),
            None => sunrise_sim::health::hurt_sound("hurt"),
        }
    }
    // remote_tab_players: remote/sync.rs
    // update_remote_skins: remote/status.rs
    pub(in crate::game::client_side) fn remote_item_data(
        &mut self,
        id: i32,
        stack: &Option<Stack>,
    ) {
        let Some(stack) = self.to_stack(stack) else {
            return;
        };
        let pending = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.pending_items.remove(&id));
        if let Some(item) = self.common.dropped.iter_mut().find(|d| d.id == id as u32) {
            item.stack = stack;
            return;
        }
        let Some(PendingItem {
            join,
            position: [x, y, z],
            velocity,
            no_gravity,
        }) = pending
        else {
            return;
        };
        let at = Vec3::new(x, y, z);
        // ItemEntity's constructor draws bobOffs from the item's own random.
        let mut random = sunrise_sim::mobs::entity_random(id as u32, at);
        let bob = crate::game::client_side::item_entities_client::bob_offset(
            at,
            random.next_float(),
            self.common.clock.still(),
        );
        // Metadata can arrive after newer entities (including other items). Insert at the
        // reserved ClientLevel.addEntity slot, not the tail at metadata time.
        let index = self.common.dropped.partition_point(|item| item.join < join);
        self.common.dropped.insert(
            index,
            ItemEntity {
                join,
                random,
                no_gravity,
                ..ItemEntity::new(id as u32, stack, at, velocity, bob)
            },
        );
    }
    /// `AbstractMinecartRenderer.newExtractState`: sample the same weighted
    /// steps at the frame's partial tick, including corners within one tick.
    pub(in crate::game::client_side) fn remote_minecart_lerp(
        &self,
        id: u32,
        partial: f32,
    ) -> Option<([f64; 3], f32, f32)> {
        let lerp = self.client.remote.as_ref()?.cart_lerps.get(&(id as i32))?;
        if lerp.current.is_empty() {
            return None;
        }
        let (p, _, yaw, pitch) = lerp.at(partial);
        Some(([p.x, p.y, p.z], yaw, pitch))
    }
    /// Status for `sunrise-ctl status`.
    /// The other players' entities in the level and their names (their
    /// profiles', which outlive the player list's entries).
    pub(in crate::game::client_side) fn remote_player_names(&self) -> Vec<(u32, String)> {
        let Some(r) = self.client.remote.as_ref() else {
            return Vec::new();
        };
        r.level_players
            .iter()
            .filter(|(_, uuid)| **uuid != r.profile)
            .filter_map(|(id, uuid)| Some((*id as u32, r.level_player_name(*id, uuid)?.to_owned())))
            .collect()
    }
    pub(in crate::game) fn remote_player_uuid(&self, id: u32) -> Option<u128> {
        let remote = self.client.remote.as_ref()?;
        if self
            .client
            .player_model
            .as_ref()
            .is_some_and(|m| m.id == id)
        {
            Some(remote.profile)
        } else {
            remote.level_players.get(&(id as i32)).copied()
        }
    }
    pub(in crate::game::client_side) fn remote_respawn(&mut self) {
        self.send(S::ClientCommand(play::ClientCommand { action: 0 }));
    }
    /// A connection's screen is over the world: connecting, reconfiguring, or
    /// the level loading screen, a portal's included (`remote_overlay`).
    pub(in crate::game::client_side) fn remote_screen_up(&self) -> bool {
        !self.client.replay.playing()
            && self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| self.common.player.is_none() || r.level_load.is_active())
    }
    /// The `SetPassengers` bookkeeping of the tracked entities.
    pub(in crate::game::client_side) fn remote_seat_passengers(
        &mut self,
        vehicle: u32,
        passengers: &[i32],
    ) {
        if let Some(r) = self.client.remote.as_mut() {
            seat_passengers(&mut r.entities, vehicle, passengers);
        }
    }
    /// Whether secondary use skips a block's default interaction on this client.
    pub(in crate::game::client_side) fn remote_secondary_use_skips_block(&self) -> bool {
        self.remote_shift_down()
            && (self.common.stack_in(self.common.selected).is_some()
                || self.common.offhand.is_some())
    }
    /// `handleServerData`: the server list entry of the server joined keeps
    /// the icon the server sends (`ServerList.saveSingleServer`). Single
    /// player has no entry.
    pub(in crate::game) fn remote_server_data(&mut self, icon: Option<&[u8]>) {
        let (Some(r), Some(png)) = (self.client.remote.as_ref(), icon) else {
            return;
        };
        if self.client.integrated.is_some() {
            return;
        }
        if let Ok(dir) = std::env::current_dir() {
            sunrise_client::multiplayer::save_server_icon(
                &dir.join("saves"),
                &r.session.address,
                png,
            );
        }
    }
    /// `LocalPlayer.isShiftKeyDown` (and so `isSecondaryUseActive`): the input as of the player's
    /// last tick (`input.keyPresses`, what the last ServerboundPlayerInputPacket said), not the key
    /// now: a sneak pressed in the frame of a click acts from the next tick, as on the server.
    pub(in crate::game) fn remote_shift_down(&self) -> bool {
        self.client
            .remote
            .as_ref()
            .is_some_and(|r| r.last_input & SHIFT_INPUT != 0)
    }
    /// ServerboundPlayerCommandPacket STOP_SLEEPING.
    pub(in crate::game) fn remote_stop_sleeping(&mut self) {
        let id = self.client.remote.as_ref().map_or(0, |r| r.entity_id);
        self.send(S::PlayerCommand(play::PlayerCommand {
            id,
            action: 0,
            data: 0,
        }));
    }
    // remote_player_eating: remote/use.rs
    /// Inventory as the server last described it.
    pub fn remote_synced(&mut self) {
        let stacks = self.creative_stacks();
        if let Some(r) = self.client.remote.as_mut() {
            r.synced = stacks;
        }
    }
    /// The slots a server packet changed since `before` are as the server has them; the
    /// others keep what was last sent, so a creative click made since still goes out.
    pub(in crate::game) fn remote_synced_since(&mut self, before: &[Option<ItemStack>]) {
        let now = self.creative_stacks();
        if let Some(r) = self.client.remote.as_mut() {
            mark_synced(&mut r.synced, before, now);
        }
    }
    /// `Entity.isVehicle` on a client: the entities a tracked entity rides.
    pub(in crate::game::client_side) fn remote_vehicle_ids(
        &self,
    ) -> impl Iterator<Item = u32> + '_ {
        self.client
            .remote
            .iter()
            .flat_map(|r| r.entities.values().filter_map(|t| t.vehicle))
    }
    pub(in crate::game) fn send_action(&self, action: i32) {
        self.send(S::PlayerAction(play::PlayerAction {
            action,
            pos: [0; 3],
            direction: 0,
            sequence: 0,
        }));
    }
    /// ClientSuggestionProvider.customSuggestion: the chat's completion
    /// request for an `ask_server` node goes to the server.
    pub(in crate::game::client_side) fn send_suggestion_request(&mut self) {
        if self.client.remote.is_none() {
            return;
        }
        let Some((id, command)) = self
            .client
            .chat_ui
            .chat_screen
            .as_mut()
            .and_then(|s| s.take_request())
            .or_else(|| self.command_block_suggestion_request())
        else {
            return;
        };
        let mut w = ::sunrise_core::vanilla_bytes::Writer::new();
        w.varint(id);
        w.string(&command);
        self.send(S::Unknown(
            sunrise_protocol::ids::play::serverbound::COMMAND_SUGGESTION,
            w.data,
        ));
    }
    /// `Entity.absSnapTo`: the remembered point, clamped to the level's edge.
    /// The velocity is left alone, and the player has no separate previous
    /// position to reset (its box is drawn from `position`).
    pub(in crate::game) fn snap_player_to(&mut self, at: [f64; 3]) {
        let Some(player) = self.common.player.as_mut() else {
            return;
        };
        let x = at[0].clamp(-3.0e7, 3.0e7);
        let z = at[2].clamp(-3.0e7, 3.0e7);
        player.position = Vec3::new(x, at[1], z);
    }
    /// A stack from the server, decoded as the saved and command item NBT
    /// are, so tooltips and models see every field.
    pub fn to_stack(&self, s: &Option<Stack>) -> Option<ItemStack> {
        self.decode_remote_stack(s.as_ref()?)
    }

    pub fn send(&self, packet: S) {
        if self.client.remote.is_some() {
            // MultiPlayerGameMode's attack, interact, useItem, releaseUsingItem, dropItem and
            // piercingAttack sync the carried slot before their packet. useItemOn does this
            // separately, before prediction. A miss, a dig's start or a swap waits for the next tick.
            if let S::Attack(_) | S::Interact(_) | S::UseItem(_) = &packet {
                self.remote_send_carried();
            } else if let S::PlayerAction(a) = &packet
                && matches!(a.action, 4 | 5 | 6 | 8)
            {
                self.remote_send_carried();
            }
            self.send_play(packet);
        }
    }

    /// `MultiPlayerGameMode.attack` runs `Player.attack` on this client too: its
    /// `isSweepAttack` for a damaging hit on `target` (`living`: a
    /// `LivingEntity`, which may be hit critically), heard by native mods on
    /// both sides. Only the server sweeps.
    pub(in crate::game) fn remote_sweep_event(&mut self, target: &str, living: bool) {
        if mod_api::__private::mods().is_empty() {
            return;
        }
        let (damage, scale) = self.remote_attack_damage();
        // Player.getEnchantedDamage returns baseDamage on a client, so magic bonus is 0.
        if damage <= 0.0 {
            return;
        }
        let mut crit = living && self.common.critical_attack(scale);
        if living {
            let vanilla = crit;
            let mut multiplier = if vanilla { 1.5 } else { 1.0 };
            self.native_critical_hit(target, vanilla, &mut crit, &mut multiplier);
        }
        self.sweep_decision(scale, crit, target);
    }

    /// Minecraft.handleKeybinds, key.swapOffhand: the server swaps the hands
    /// (ServerboundPlayerActionPacket SWAP_ITEM_WITH_OFFHAND at BlockPos.ZERO, facing down) and
    /// sends both slots back; nothing is predicted.
    pub(in crate::game) fn remote_swap_hands(&mut self) {
        self.send_action(7);
        // Equal stacks swap with no slot update to wait for.
        let (main, off) = (
            self.common.stack_in(self.common.selected),
            self.common.offhand.as_ref(),
        );
        let moves = !super::hand::matches_for_swap(main, off);
        if let Some(r) = self.client.remote.as_mut() {
            r.swap_answer = [moves; 2];
        }
    }
}

/// The game as the tick's input sees it.
#[cfg(feature = "client")]
pub struct RemoteInputWorld<'g, 'a>(pub &'g mut crate::game::ClientGame<'a>);

#[cfg(feature = "client")]
pub struct RemoteUseWorld<'g, 'a>(
    pub &'g mut crate::game::ClientGame<'a>,
    pub mod_api::InteractionAttempt,
);

#[cfg(feature = "client")]
impl sunrise_client::interaction_input::TargetUseWorld for RemoteUseWorld<'_, '_> {
    fn use_entity(&mut self) -> bool {
        self.0.remote_use_entity(Hand::Main)
    }
    fn use_block(&mut self) -> bool {
        self.0.remote_use_block(Hand::Main, self.1, None)
    }
    fn use_item(&mut self) {
        self.0.remote_use_hands(Hand::Main, self.1, None);
    }
}

#[cfg(feature = "client")]
#[cfg(any(test, feature = "test-support"))]
impl crate::game::ClientGame<'_> {
    pub fn test_audio_mut(&mut self) -> &mut sunrise_client_render::audio::Audio {
        &mut self.client.audio.0
    }

    pub fn test_play_audio_button_click(&mut self) {
        self.play_audio_button_click();
    }

    pub fn test_remote_pick_click(&mut self) {
        use sunrise_client::interaction_input::InteractionWorld as _;
        RemoteInputWorld(self).pick();
    }
    pub fn test_has_client_item_use(&self) -> bool {
        self.client.client_item_use.0.is_some()
    }
    pub fn test_show_title_screen(&mut self) {
        let saves = std::env::current_dir().unwrap_or_default().join("saves");
        self.client.title.0 = Some(self.new_title(saves));
    }
    pub fn test_hand_strength_ticker(&self) -> i32 {
        self.client.hand_strength.ticker
    }
    pub fn test_set_hand_strength_ticker(&mut self, ticker: i32) {
        self.client.hand_strength.ticker = ticker;
    }
}

impl PlayerTextures {
    pub fn new(profile: u128, property: AccountProperty) -> Self {
        Self(sunrise_client_render::skins::PackedTextures {
            profile,
            value: property.value,
            signature: property.signature,
        })
    }
}

impl Remote {
    pub(in crate::game) fn constructing(
        &self,
        level: mod_api::LevelRef,
    ) -> Option<&ConstructingClientLevel> {
        self.constructing_level
            .as_ref()
            .filter(|pending| pending.handle == level)
    }

    /// Project the session address for display once, dropping the memory transport's marker.
    pub(in crate::game) fn display_address_for(address: &str) -> Option<String> {
        (address != "local").then(|| sunrise_client_lib::net::shown_address(address))
    }
}

impl ServicesKeys {
    /// Minecraft.services() has the keys wherever the game is played, single player's own
    /// server included: its guests' chat keys are checked against them.
    pub(in crate::game::client_side) fn fetching() -> Arc<std::sync::Mutex<Self>> {
        Self::fetching_with(|| {
            sunrise_server_profiles_http::fetch_profile_key_keys(
                &sunrise_server_files::profiles::discovery_url(),
            )
        })
    }
    /// The keys start `Pending` and arrive from `fetch` on a thread of their own, so a
    /// listener never caches a verdict on keys it does not have yet. A thread that cannot
    /// start leaves them pending.
    pub fn fetching_with<E: std::fmt::Display>(
        fetch: impl FnOnce() -> std::result::Result<ServicesKeySet, E> + Send + 'static,
    ) -> Arc<std::sync::Mutex<Self>> {
        let keys = Arc::new(std::sync::Mutex::new(ServicesKeys::Pending));
        let slot = keys.clone();
        let _ = std::thread::Builder::new()
            .name("Services keys".into())
            .spawn(move || {
                // The error is the line vanilla logs, after which it has no keys.
                let keys = fetch().unwrap_or_else(|why| {
                    log::error!("Sunrise: {why}");
                    ServicesKeySet::default()
                });
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = ServicesKeys::Ready(keys);
            });
        keys
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "level_sound_tests_client.rs"]
pub(in crate::game) mod level_sound_tests;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "frozen_screen_effects_tests_client.rs"]
pub(in crate::game) mod frozen_screen_effects_tests;

#[cfg(all(test, feature = "client"))]
#[path = "client_break_progress_tests.rs"]
mod client_break_progress_tests;

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

use crate::game::client_side::remote_client::minecarts::CartLerpClientExt as _;
