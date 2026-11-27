#[cfg(all(test, feature = "client"))]
use client_side::resource_load_client::load_resources_for_mods;
#[cfg(feature = "client")]
pub use client_side::resource_load_client::load_resources_for_mods_with_limit;
#[cfg(feature = "client")]
use client_side::resource_tables_client::{ClientPackResources, ClientResources};
#[cfg(test)]
use client_side::window_handler::follow_keymap;
#[cfg(feature = "client")]
#[allow(unused_imports)]
use loading_screen::{App, Loading, leave_fullscreen_after, run_event_loop};
#[cfg(feature = "client")]
#[allow(unused_imports)]
use motion_ids::{motion_at, motion_block_entity};
use std::cell::RefCell;
use std::sync::Arc;
use std::time::Instant;
#[cfg(all(test, feature = "client"))]
use sunrise_client::block_entity_render::CacheOptions;
use sunrise_client::{
    assets::RenderAssets,
    audio::Audio,
    block_entity_render::{self, BlockEntityRenderer},
    creative::Creative,
    gfx::{Frame, Renderer},
    hud::{HudAtlas, HudBatch, rgba},
    hud_layout, hud_overlays,
    keymapping::{KeyAction, SNEAK, SPRINT},
    mesher::{MeshTables, Vertex},
    models,
    mouse::{LookOptions, MouseLook},
    particles::Particle,
    settings::Settings,
    title::{Action, Title},
};
use sunrise_gameplay_common::game::integrated_link;
use sunrise_gameplay_common::game::saving;
#[cfg(feature = "client")]
macro_rules! client_game_side {
    ($s:expr, $meta:expr) => {
        sunrise_gameplay_common::game_side!(
            $s.common,
            $meta,
            match &$s.client.remote {
                Some(remote) => &remote.features,
                None => sunrise_gameplay_common::game::tables::default_client_features(),
            },
            $s.client
                .remote
                .as_ref()
                .and_then(|remote| remote.tags.as_ref())
        )
    };
}
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::activity;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::advancements_screen;
#[allow(unused_imports)]
pub(in crate::game) use client_side::agent;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::agent_settings;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::beacon;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::block_breaking;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::block_editor;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::block_particles;
#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::block_screen_effect_draw_tests;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::boat_rider_fixture;
#[allow(unused_imports)]
pub use client_side::books;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::bookshelves;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::bundles;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::camera_hud;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::chat;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::client_ambient;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::client_attack_targets;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::client_cracks;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::client_executor;
#[allow(unused_imports)]
pub use client_side::client_item_extensions;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub use client_side::client_reload;
#[cfg(feature = "client")]
pub(in crate::game) use client_side::hotbar_items;
pub(in crate::game) use client_side::stock_layer_mask;
#[path = "game/client_side.rs"]
pub mod client_side;
#[cfg(all(test, feature = "client"))]
#[path = "game/interface/furnace_take_tests_client.rs"]
mod furnace_take_tests_client;
#[allow(unused_imports)]
pub(in crate::game) use client_side as client_state;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::client_tick_steps;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::clipboard;
#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::cloud_height_frame_tests;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::code_of_conduct;
#[allow(unused_imports)]
pub use client_side::command_blocks;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::command_minecarts;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::container_hooks;
#[allow(unused_imports)]
pub(in crate::game) use client_side::container_layout;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::control;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::creative_hotbar;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::custom_arm_pose;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::custom_sky;
#[allow(unused_imports)]
pub(in crate::game) use client_side::debug_keys;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::debug_options;
#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::debug_renderer_tests;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::debug_renderers;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::demo_intro;
#[allow(unused_imports)]
pub(in crate::game) use client_side::dialogs;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::dynamic_lights;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::entity_culling;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::entity_part_state;
#[allow(unused_imports)]
pub(in crate::game) use client_side::entity_sounds;
#[allow(unused_imports)]
pub(in crate::game) use client_side::equipment_piece;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub use client_side::event_loop;
#[allow(unused_imports)]
pub(in crate::game) use client_side::events;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub use client_side::game_client::WindowKeyInput;
#[cfg(feature = "client")]
/// Video options a mod set for this session only, over the player's saved ones.
#[derive(Default)]
pub(in crate::game) struct SessionVideo {
    pub(in crate::game) vsync: Option<bool>,
    /// As `options.txt` names it: `false`, `fast` or `true`.
    pub(in crate::game) clouds: Option<&'static str>,
    pub(in crate::game) view_bobbing: Option<bool>,
}

#[cfg(feature = "client")]
/// Read-only renderer state cached while `Game::renderer` is temporarily taken for `Renderer::render` so `ClientHost` queries keep working inside stage callbacks.
#[derive(Clone, Copy)]
pub(in crate::game) struct RenderSnapshot {
    pub(in crate::game) framebuffer: [u32; 2],
    pub(in crate::game) prepared_generation: Option<u64>,
    pub(in crate::game) staged_prepared_draws: u64,
}

#[cfg(feature = "client")]
impl ClientGame<'_> {
    pub(in crate::game) fn has_renderer(&self) -> bool {
        self.client.renderer.0.is_some() || self.client.render_snapshot.get().is_some()
    }

    pub(in crate::game) fn renderer_framebuffer(&self) -> Option<[u32; 2]> {
        self.client
            .renderer
            .0
            .as_ref()
            .map(|renderer| [renderer.config.width, renderer.config.height])
            .or_else(|| {
                self.client
                    .render_snapshot
                    .get()
                    .map(|snapshot| snapshot.framebuffer)
            })
    }

    pub(in crate::game) fn renderer_prepared_generation(&self) -> Option<u64> {
        self.client.renderer.0.as_ref().map_or_else(
            || {
                self.client
                    .render_snapshot
                    .get()
                    .and_then(|snapshot| snapshot.prepared_generation)
            },
            |renderer| renderer.prepared_generation(),
        )
    }

    pub(in crate::game) fn renderer_prepared_room_info(&self) -> Option<(u64, u64)> {
        let max_buffer_size = self.client.mod_gpu.as_ref()?.max_buffer_size();
        let staged = if self.client.renderer.0.is_some() {
            0
        } else {
            self.client
                .render_snapshot
                .get()
                .map_or(0, |snapshot| snapshot.staged_prepared_draws)
        };
        Some((max_buffer_size, staged))
    }

    pub(in crate::game) fn mod_gpu(&self) -> Option<&sunrise_client_render::gfx::ModGpuResources> {
        self.client.mod_gpu.as_ref()
    }

    pub fn set_renderer(&mut self, renderer: Box<Renderer>) {
        self.client.mod_gpu = Some(renderer.mod_gpu().clone());
        self.client.renderer.0 = Some(renderer);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_set_renderer(&mut self, renderer: Box<Renderer>) {
        self.set_renderer(renderer);
    }
}

#[allow(dead_code)]
#[cfg(feature = "client")]
pub struct ClientGame<'a> {
    pub common: sunrise_gameplay_common::game::CommonGame<'a>,
    pub client: client_side::ClientState,
}
#[cfg(feature = "client")]
impl<'a> std::ops::Deref for ClientGame<'a> {
    type Target = sunrise_gameplay_common::game::CommonGame<'a>;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.common
    }
}

#[cfg(feature = "client")]
impl std::ops::DerefMut for ClientGame<'_> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.common
    }
}

#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::allays_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::composter_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::extras;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::far_terrain_edits;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::focused_text_field;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::fov;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::frame_geometry;
#[allow(unused_imports)]
pub(in crate::game) use client_side::friends;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::gamemode_switcher;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::general_draw;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::glowing;
#[allow(unused_imports)]
pub(in crate::game) use client_side::growth;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::gui_clips;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::gui_entity;
#[allow(unused_imports)]
pub(in crate::game) use client_side::hand;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::headless;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::hud_editor;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::hud_room;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::humanoid_setup_anim;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::ime;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::initial_client_models;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub use client_side::input_trace;
#[allow(unused_imports)]
pub(in crate::game) use client_side::interactions;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::inventory_input;
#[allow(unused_imports)]
pub use client_side::item_draw;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::item_model_context;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::item_particles;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::jigsaw_edit;
#[allow(unused_imports)]
pub(in crate::game) use client_side::jukebox;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::modapi_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::spawner_client;
#[cfg(test)]
#[path = "game/interface/keymap_tests.rs"]
mod keymap_tests;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::lan_screen;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::launch_timeout;
#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::lightmap_expression_tests;
#[allow(unused_imports)]
pub(in crate::game) use client_side::lightning;
#[allow(unused_imports)]
pub(in crate::game) use client_side::loading_screen;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::loading_window;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::locator_bar;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::look;
#[allow(unused_imports)]
pub(in crate::game) use client_side::loom;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::mob_blocks;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::mob_light;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::mob_sounds;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::mod_toasts;
#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::model_baking_completed_tests;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::motion_ids;
#[allow(unused_imports)]
pub use client_side::mount_menu;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::music;
#[allow(unused_imports)]
pub(in crate::game) use client_side::names;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::narrator;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_audio_streams;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_biome_colors;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub use client_side::native_block_entity_draw;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_block_sprites;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_book_receive;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_claimed_items;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_client_particles;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_client_particles_fixture;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_client_screens;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub use client_side::native_client_services;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_client_session;
#[allow(unused_imports)]
pub use client_side::native_connection_groups;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_custom_geometry;
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_disconnect;
#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_disconnect_tests;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_dynamic_textures;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub use client_side::native_entity_layers;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub use client_side::native_entity_renderers;
#[cfg(test)]
#[path = "game/modding/native_experience_orb_renderers_tests.rs"]
mod native_experience_orb_renderers_tests;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_fluid_render;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub use client_side::native_hotbar_slot;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_instance_groups;
#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_item_frame_renderers_tests;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_item_particles;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_key_mappings;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_local_player_writes;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_local_sounds;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_mapping_presses;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_menu_refs;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_menu_slot_kinds;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub use client_side::native_mob_draw;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_model_3d;
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_net;
#[cfg(test)]
#[path = "game/modding/native_painting_renderers_tests.rs"]
mod native_painting_renderers_tests;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_panel;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_panel_recipes;
#[cfg(test)]
#[path = "game/modding/native_panel_wheel_tests.rs"]
mod native_panel_wheel_tests;
#[allow(unused_imports)]
use crate::frame_autotune;
#[allow(unused_imports)]
use crate::occlusion_culling;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::advancements_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::block_entity_anims_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::boats_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::cauldron_prediction_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::commands_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::creatures_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::dragon_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::end_client;
#[allow(unused_imports)]
pub(in crate::game) use client_side::entity_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::entity_sizes_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::fangs_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::fireworks_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::fishing_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::game_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::game_mode_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::integrated_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::level_particles_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::machines_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::memory_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_book_net_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_book_placement_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_book_settings_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_entity_refs_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub use client_side::native_hooks_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_living_effects_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_part_tables;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_passage_cross;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_recipes;
#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_render_pair_tests;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_render_visibility;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_scene;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_scene_live;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_section_visibility;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_shaders;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_special_renderers;
#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_spectator_shader_tests;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_terrain_model;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_viewed_levels;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::native_visuals;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::nautili;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::net_harness;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::no_resources;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::ominous_item_spawner_client;
#[allow(unused_imports)]
pub(in crate::game) use client_side::options;
#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::pack_clouds_frame_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::pack_sources_tests;
#[allow(unused_imports)]
pub use client_side::packs;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::pad;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::pad_actions;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::pad_craft;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::pad_menu;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::paintings_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::particle_culling;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::perf_link;
#[allow(unused_imports)]
pub(in crate::game) use client_side::perspective;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::pick_accumulator;
#[allow(unused_imports)]
pub(in crate::game) use client_side::pick_entity;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::pickups;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::play_input;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::play_keys;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::player_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::pointer_capture;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::potions;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::profile_capture;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::pvp;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::quick_actions;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::random_textures;
#[allow(unused_imports)]
pub(in crate::game) use client_side::recipe_book;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::remote_block_writes;
#[allow(unused_imports)]
pub use client_side::remote_camera;
#[allow(unused_imports)]
pub(in crate::game) use client_side::remote_chat;
#[allow(unused_imports)]
pub(in crate::game) use client_side::remote_chunks;
#[allow(unused_imports)]
pub(in crate::game) use client_side::remote_mob_data;
#[cfg(all(feature = "client", any(test, feature = "test-support")))]
pub mod remote_mob_data_test_support {
    pub use super::remote_mob_data::{apply_registry_data, tests::fixture_mob};
}
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::remote_mob_particles;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::remote_mob_use;
#[allow(unused_imports)]
pub(in crate::game) use client_side::remote_objects;
#[allow(unused_imports)]
pub(in crate::game) use client_side::remote_projectiles;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::remote_removal;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::remote_stations;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::remote_tags;
#[allow(unused_imports)]
pub(in crate::game) use client_side::remote_vehicles;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::render_control;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::render_settings;
#[allow(unused_imports)]
pub(in crate::game) use client_side::render_tick;
#[allow(unused_imports)]
pub(in crate::game) use client_side::replay;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::repro;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::resource_loader;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::saves_control;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::saving_client;
#[allow(unused_imports)]
pub(in crate::game) use client_side::screen_build;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::screen_build_fixture;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::screen_room;
#[allow(unused_imports)]
pub use client_side::screen_widgets;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::screens;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::screenshot_sequence;
#[allow(unused_imports)]
pub(in crate::game) use client_side::screenshot_writer;
#[allow(unused_imports)]
pub use client_side::sculk;
#[allow(unused_imports)]
pub(in crate::game) use client_side::server_links;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::server_packs;
#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::shader_pack_switch_tests;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::shader_packs;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::shelf;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::sign_edit;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::sniffers;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::social;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::spear_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::spectator;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::stack_attribute_tooltip;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::startup_menu;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::station_control;
#[allow(unused_imports)]
pub use client_side::stations;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::stations_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::stats_screen;
#[allow(unused_imports)]
pub(in crate::game) use client_side::storage;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::structure_edit;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::studio;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::subtitle_hud;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::subtitles;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::swing_packet;
#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::sync_background_tests;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::test_block_edit;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::test_instance_edit;
#[allow(unused_imports)]
pub(in crate::game) use client_side::text_displays;
#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::text_expression_tests;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::thrown_item_draw;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::tick_script;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::titles;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::tnt_client;
#[allow(unused_imports)]
pub(in crate::game) use client_side::totem;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::transactions_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::transient_blocks;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::trial_visuals;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::trident_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::tutorial;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::ui_control;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::using_item_client;
#[allow(unused_imports)]
pub use client_side::viewed_worlds;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::win;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::window_handler;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::window_title;
#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::world_name_tag_tests;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::world_rest;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::world_session;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::world_update;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::world_view_client;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use client_side::zombie_villagers_client;

#[cfg(feature = "client")]
#[allow(unused_imports)]
use client_state::ClientState;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub use event_loop::EventLoopCtl;
#[cfg(feature = "client")]
#[allow(unused_imports)]
use loading_window::{
    build_renderer, loading_frame, loading_renderer, prepare_renderer_textures, start_client_game,
};
#[cfg(feature = "client")]
pub use native_block_entity_draw::BlockEntityDraw;
#[cfg(feature = "client")]
pub use native_mob_draw::{MobRender, PlayerRender};
#[cfg(feature = "client")]
use resource_loader::{EntityModels, GameCpuResources, Loaded, LoaderMsg, PendingWorldEntry};
#[allow(unused_imports)]
#[cfg(feature = "client")]
use resource_loader::{run_loading, write_report};
#[allow(unused_imports)]
use sunrise_client::actions::Action as Mapping;
#[allow(unused_imports)]
use sunrise_client::held::Source;
#[allow(unused_imports)]
use sunrise_client_render::block_entity_reach::Reach;
#[allow(unused_imports)]
use winit::{
    application::ApplicationHandler,
    error::EventLoopError,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};

#[cfg(feature = "client")]
pub use client_side::resource_tables_client::UiResources;

pub fn select_creative_tab(
    creative: &mut Creative,
    clicks: &mut sunrise_client::container_screen::Clicks,
    tab: usize,
) {
    creative.select(tab);
    *clicks = Default::default();
    // Tabs are outside the panel; their release must not drop a carried stack.
    clicks.skip_release();
}

/// Physical and scaled GUI dimensions last reported by `window_resize`.
#[derive(Clone, Copy, PartialEq)]
pub struct ResizeState {
    pub size: winit::dpi::PhysicalSize<u32>,
    pub gui_size: [u32; 2],
}

impl ResizeState {
    /// Records `self` as the last report and says whether it differs from the previous one.
    pub fn differs_from(self, last: &mut Option<Self>) -> bool {
        last.replace(self) != Some(self)
    }
}

/// The depth-only view and camera axis used for falling blocks in one frame.
pub struct FallingSetup<'a> {
    pub(in crate::game) view: Option<&'a sunrise_client::gfx::JoinedFrusta>,
    pub(in crate::game) forward: glam::Vec3,
}

/// What the experience orbs of one frame share: their sprite and the camera's axes.
pub struct OrbFrame {
    sprite: Option<sunrise_client_render::hud::Rect>,
    forward: glam::Vec3,
    right: glam::Vec3,
    up: glam::Vec3,
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    fn draw_reload_overlay(
        &self,
        batch: &mut HudBatch,
        bar: f32,
        fade: sunrise_client::level_loading::OverlayFade,
    ) {
        sunrise_client::level_loading::draw_loading_overlay(
            batch,
            &self.client.gui.atlas,
            bar,
            fade,
            self.client
                .settings
                .file
                .bool("darkMojangStudiosBackground"),
        );
    }

    /// The orb's sprite and the camera's axes, once a frame however many runs of orbs draw.
    pub(super) fn orb_frame(&self) -> OrbFrame {
        let sprite = self
            .client
            .gui
            .atlas
            .sprites
            .get("entity/experience/experience_orb")
            .copied();
        let (yaw, pitch) = (self.common.yaw.to_radians(), self.common.pitch.to_radians());
        let forward = glam::Vec3::new(
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        );
        let right = forward.cross(glam::Vec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        OrbFrame {
            sprite,
            forward,
            right,
            up,
        }
    }
}

#[cfg(feature = "client")]
#[cfg(any(test, feature = "test-support"))]
impl crate::game::ClientGame<'_> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn hud(&mut self, size: [u32; 2]) -> HudBatch {
        self.hud_with_reload(size, None)
    }
}
#[cfg(feature = "client")]
pub(in crate::game) struct ClientSidePrep {
    pub(in crate::game) audio: client_side::ClientShared<Audio>,
    pub(in crate::game) particles:
        client_side::ClientShared<sunrise_client::particles::ParticleEngine>,
    pub(in crate::game) armor_stand_assets:
        client_side::ClientOnly<client_side::armor_stands_client::ArmorStandAssets>,
    pub(in crate::game) creative_screen: client_side::ClientOnly<Creative>,
    pub(in crate::game) skin_downloads:
        client_side::ClientOnly<RefCell<sunrise_client::skins::Downloader>>,
    pub(in crate::game) registered_debug_entries: client_side::ClientOnly<
        std::sync::Arc<[sunrise_client::debug_entries::RegisteredDebugEntry]>,
    >,
    pub(in crate::game) debug_entries:
        client_side::ClientOnly<sunrise_client::debug_entries::DebugEntries>,
}
#[cfg(feature = "client")]
pub(in crate::game) struct WrappedClientInit {
    pending_entry: client_side::ClientOnly<Option<PendingWorldEntry>>,
    pack_reload: client_side::ClientOnly<Option<packs::PackReload>>,
    server_packs: client_side::ClientOnly<server_packs::ServerPackState>,
    client_reports: client_side::ClientOnly<Arc<sunrise_sim::chunk_io::Reports>>,
    command_prompt: client_side::ClientOnly<Option<remote_chat::CommandPrompt>>,
    dialogs: client_side::ClientOnly<dialogs::DialogState>,
    pending_initial_models:
        client_side::ClientOnly<Option<Box<initial_client_models::PendingClientStart>>>,
    stitching_block_atlas: client_side::ClientOnly<Option<Arc<RenderAssets>>>,
    lod: client_side::ClientOnly<Option<sunrise_client::lod::Lod>>,
    lod_blue_map: client_side::ClientOnly<Option<Arc<sunrise_client::bluemap::BlueMap>>>,
    video_modes: client_side::ClientOnly<sunrise_client::window::VideoModeList>,
    screen_state: client_side::ClientOnly<screen_build::ScreenState>,
    window: client_side::ClientShared<Option<Arc<Window>>>,
    skin_models:
        client_side::ClientOnly<std::cell::RefCell<sunrise_client::mobs::skin::SkinModels>>,
    lent_level_random: client_side::ClientOnly<
        Option<std::rc::Rc<std::cell::RefCell<mod_api::LegacyRandomSource>>>,
    >,
    pad: client_side::ClientShared<pad::Pads>,
    live_scenes: client_side::ClientOnly<std::cell::RefCell<native_scene_live::LiveScenes>>,
    debug_renderers:
        client_side::ClientOnly<Vec<sunrise_client::debug_entries::ActiveDebugRenderer>>,
    pickups: client_side::ClientOnly<pickups::Pickups>,
    title: client_side::ClientShared<Option<Title>>,
    text_displays:
        client_side::ClientOnly<indexmap::IndexMap<i32, text_displays::RemoteTextDisplay>>,
    recipe_ui: client_side::ClientOnly<recipe_book::RecipeUi>,
    spawners: client_side::ClientOnly<client_side::spawner_client::SpawnerState>,
    mod_screen: client_side::ClientOnly<Option<client_side::native_hooks_client::OpenNativeScreen>>,
    pending_screenshot: client_side::ClientOnly<Option<std::sync::mpsc::Sender<String>>>,
    screenshot_sequence: client_side::ClientOnly<Option<screenshot_sequence::Sequence>>,
    pending_render_capture: client_side::ClientOnly<Option<std::sync::mpsc::Sender<String>>>,
    pending_bench:
        client_side::ClientOnly<Option<(Instant, std::sync::mpsc::Sender<String>, bool)>>,
    flight: client_side::ClientOnly<Option<control::Flight>>,
    ui_wait: client_side::ClientOnly<ui_control::UiWait>,
    input_trace: client_side::ClientOnly<input_trace::InputTrace>,
    repro: client_side::ClientOnly<repro::Repro>,
    occlusion: client_side::ClientOnly<entity_culling::EntityOcclusion>,
    async_particle_culler: client_side::ClientOnly<RefCell<particle_culling::ParticleAsyncState>>,
    pending_uploads: client_side::ClientOnly<Vec<sunrise_client::streaming::MeshUpload>>,
    pending_uploads_merge: client_side::ClientOnly<Vec<sunrise_client::streaming::MeshUpload>>,
    client_item_use: client_side::ClientShared<Option<native_claimed_items::ActiveClientItemUse>>,
    client_tasks: client_side::ClientOnly<sunrise_client::net::ClientTasks>,
    pending_client_tasks:
        client_side::ClientOnly<std::collections::VecDeque<sunrise_client::net::ClientTask>>,
    world_options: client_side::ClientOnly<Option<lan_screen::WorldOptions>>,
    command_block_screen: client_side::ClientOnly<Option<command_blocks::CommandBlockScreen>>,
    block_editor: client_side::ClientOnly<Option<block_editor::BlockEditScreen>>,
    clipboard: client_side::ClientOnly<sunrise_client::clipboard::SystemClipboard>,
    pending_save: client_side::ClientOnly<Option<saving::ClosingSave>>,
    tab_list: client_side::ClientOnly<sunrise_client::tab_list::TabList>,
    adv_ui: client_side::ClientOnly<advancements_screen::AdvUi>,
}

#[cfg(feature = "client")]
pub(in crate::game) struct WrappedRendererInit {
    renderer: client_side::ClientShared<Option<Box<Renderer>>>,
}

use sunrise_gameplay_common::game::WrappedPrepInit;
struct ConstructInit<const NO_SLOT: bool>;
impl<const NO_SLOT: bool> ConstructInit<NO_SLOT> {
    #[cfg(feature = "client")]
    #[inline(never)]
    fn build_wrapped_client() -> Box<WrappedClientInit> {
        Box::new(WrappedClientInit {
            pending_entry: client_side::ClientOnly::new(None),
            pack_reload: client_side::ClientOnly::new(None),
            server_packs: client_side::ClientOnly::new(Default::default()),
            client_reports: client_side::ClientOnly::new(Arc::default()),
            command_prompt: client_side::ClientOnly::new(None),
            dialogs: client_side::ClientOnly::new(Default::default()),
            pending_initial_models: client_side::ClientOnly::new(None),
            stitching_block_atlas: client_side::ClientOnly::new(None),
            lod: client_side::ClientOnly::new(None),
            lod_blue_map: client_side::ClientOnly::new(None),
            video_modes: client_side::ClientOnly::new(Default::default()),
            screen_state: client_side::ClientOnly::new(screen_build::ScreenState::default()),
            window: client_side::ClientShared::new(None),
            skin_models: client_side::ClientOnly::new(Default::default()),
            lent_level_random: client_side::ClientOnly::new(None),
            pad: client_side::ClientShared::new(Default::default()),
            live_scenes: client_side::ClientOnly::new(Default::default()),
            debug_renderers: client_side::ClientOnly::new(Vec::new()),
            pickups: client_side::ClientOnly::new(Default::default()),
            title: client_side::ClientShared::new(None),
            text_displays: client_side::ClientOnly::new(Default::default()),
            recipe_ui: client_side::ClientOnly::new(Default::default()),
            spawners: client_side::ClientOnly::new(Default::default()),
            mod_screen: client_side::ClientOnly::new(None),
            pending_screenshot: client_side::ClientOnly::new(None),
            screenshot_sequence: client_side::ClientOnly::new(None),
            pending_render_capture: client_side::ClientOnly::new(None),
            pending_bench: client_side::ClientOnly::new(None),
            flight: client_side::ClientOnly::new(None),
            ui_wait: client_side::ClientOnly::new(Default::default()),
            input_trace: client_side::ClientOnly::new(Default::default()),
            repro: client_side::ClientOnly::new(Default::default()),
            occlusion: client_side::ClientOnly::new(Default::default()),
            async_particle_culler: client_side::ClientOnly::new(RefCell::new(Default::default())),
            pending_uploads: client_side::ClientOnly::new(Vec::new()),
            pending_uploads_merge: client_side::ClientOnly::new(Vec::new()),
            client_item_use: client_side::ClientShared::new(None),
            client_tasks: client_side::ClientOnly::new(Default::default()),
            pending_client_tasks: client_side::ClientOnly::new(Default::default()),
            world_options: client_side::ClientOnly::new(None),
            command_block_screen: client_side::ClientOnly::new(None),
            block_editor: client_side::ClientOnly::new(None),
            clipboard: client_side::ClientOnly::new(Default::default()),
            pending_save: client_side::ClientOnly::new(None),
            tab_list: client_side::ClientOnly::new(Default::default()),
            adv_ui: client_side::ClientOnly::new(Default::default()),
        })
    }
    #[cfg(feature = "client")]
    #[inline(never)]
    fn build_wrapped_renderer() -> Box<WrappedRendererInit> {
        Box::new(WrappedRendererInit {
            renderer: client_side::ClientShared::new(None),
        })
    }
    #[cfg(feature = "client")]
    #[inline(never)]
    fn build_client_be_anims(
        registry: &'static sunrise_world::registry::BlockRegistry,
    ) -> sunrise_client_render::block_entity_anim::BlockEntityAnims {
        let mut anims = sunrise_client_render::block_entity_anim::BlockEntityAnims::default();
        anims.classify(registry);
        anims
    }
    #[cfg(feature = "client")]
    #[inline(never)]
    fn wrap_client_prep_and_effects(
        prep: Box<client_side::ClientConstructPrep>,
        effects: Box<client_side::ClientConstructEffectsBundle>,
        serve_integrated: integrated_link::Serve,
        run_component_harness: integrated_link::RunComponentHarness,
        session_verifier: sunrise_sim::lan::SessionVerifier,
        profile_resolver: integrated_link::ProfileResolverFactory,
    ) -> (Box<WrappedPrepInit>, Box<ClientState>) {
        let music_toast = prep.settings.file.int("musicToast").clamp(0, 2) as u8;
        let wc = Self::build_wrapped_client();
        let wr = Self::build_wrapped_renderer();
        let extra = Box::new(ClientSidePrep {
            audio: client_side::ClientShared::new(prep.audio),
            particles: client_side::ClientShared::new(prep.particles),
            armor_stand_assets: client_side::ClientOnly::new(prep.armor_stand_assets),
            creative_screen: client_side::ClientOnly::new(prep.creative_screen),
            skin_downloads: client_side::ClientOnly::new(effects.skin_downloads),
            registered_debug_entries: client_side::ClientOnly::new(
                effects.registered_debug_entries,
            ),
            debug_entries: client_side::ClientOnly::new(effects.debug_entries),
        });
        let client = ClientState::build(
            prep.client_res,
            prep.client_pack,
            prep.gui,
            prep.streaming,
            Self::build_client_be_anims(prep.pack.assets.registry),
            prep.settings,
            effects.profile,
            prep.now,
            music_toast,
            prep.options.execution_role.is_client(),
            prep.tutorial,
            prep.standalone_geometry,
            prep.client_prepared,
            effects.client,
            extra,
            wc,
            wr,
            serve_integrated,
            run_component_harness,
            session_verifier,
            profile_resolver,
        );
        let wp = Box::new(WrappedPrepInit {
            item_features: prep.item_features,
            level: prep.level,
            painting_variants: prep.painting_variants,
            world: prep.world,
            now: prep.now,
            pack: prep.pack,
            options: prep.options,
            clock: effects.clock,
            tick: effects.tick,
            ui: effects.ui,
        });
        (wp, client)
    }
}

#[cfg(feature = "client")]
#[allow(unused_imports)]
#[cfg(test)]
use inventory_input::drop_whole_stack;
#[cfg(feature = "client")]
#[allow(unused_imports)]
use subtitles::{Subtitles, in_game_ui, subtitles};

#[cfg(feature = "client")]
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::game) use client_side::sign_outline_tests;

#[allow(unused_imports)]
#[cfg(feature = "client")]
pub(in crate::game) use native_client_particles::particle_block_states;

#[cfg(feature = "client")]
use client_side::atlas_slots;

#[cfg(feature = "client")]
use client_side::screen_info;

use sunrise_gameplay_common::game::SideGame;
impl<'g> SideGame<'g> for ClientGame<'g> {
    type State = client_side::ClientState;
    fn side_state(&self) -> &Self::State {
        &self.client
    }
    fn side_state_mut(&mut self) -> &mut Self::State {
        &mut self.client
    }

    fn common_and_state_mut(
        &mut self,
    ) -> (
        &mut sunrise_gameplay_common::game::CommonGame<'g>,
        &mut Self::State,
    ) {
        (&mut self.common, &mut self.client)
    }
}

#[path = "game/side_ops/mod.rs"]
pub(in crate::game) mod side_ops;

#[path = "game/server_side.rs"]
pub(in crate::game) mod server_side;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "game/runtime/ui_core_shader_tests.rs"]
mod ui_core_shader_tests;

#[path = "game/runtime/resource_tables.rs"]
pub mod resource_tables;

#[path = "game/runtime/resource_load.rs"]
mod resource_load;

#[path = "game/runtime/launch_options.rs"]
mod launch_options;

#[path = "game/network/villager_client.rs"]
pub mod villager_client;

#[path = "game/network/remote/mod.rs"]
pub mod remote;

pub mod native_hooks;

#[path = "game/modding/payload_transport.rs"]
pub(in crate::game) mod payload_transport;

#[path = "game/modding/native_block_extra_models.rs"]
mod native_block_extra_models;

#[path = "game/modding/load_conditions.rs"]
pub(in crate::game) mod load_conditions;

#[path = "game/modding/api_values.rs"]
pub(crate) mod api_values;

#[cfg(feature = "client")]
#[path = "game/interface/toast_manager.rs"]
pub mod toast_manager;

#[cfg(feature = "client")]
#[path = "game/interface/screen_backgrounds.rs"]
pub(in crate::game) mod screen_backgrounds;

pub mod host;

impl ClientGame<'_> {
    /// An item frame's held item, apart from the frame (one id may mark one run).
    pub(super) const MOTION_FRAME_ITEM: u64 = 4 << 32;

    /// A block a piston moves, by the cell it moves to (`motion_at`).
    pub(super) const MOTION_MOVING_BLOCK: u64 = 6 << 32;

    /// The local player's flame vertices are tracked separately from the body run.
    const MOTION_OWN_PLAYER_FIRE: u64 = sunrise_gameplay_common::game::MOTION_OWN_PLAYER | 1;
}

impl sunrise_sim::crafting::CraftingRemainder for ClientGame<'_> {
    fn crafting_remainder(
        &self,
        stack: &sunrise_items::items::ItemStack,
    ) -> Option<sunrise_items::items::ItemStack> {
        sunrise_gameplay_common::game::native_block_states::NativeBlockStatesGameExt1::crafting_remainder(self, stack)
    }
}

impl sunrise_sim::menu::SlotRules for ClientGame<'_> {
    fn listens(&self, menu: &str) -> bool {
        sunrise_gameplay_common::game::slot_rules::GameSlotRules::listens(self, menu)
    }

    fn rule(
        &self,
        menu: &str,
        slot: usize,
        stack: &sunrise_items::items::ItemStack,
        context: sunrise_sim::menu::SlotContext<'_>,
        may_place: &mut bool,
        may_pickup: &mut bool,
        max_stack: &mut u32,
    ) {
        sunrise_gameplay_common::game::slot_rules::GameSlotRules::rule(
            self, menu, slot, stack, context, may_place, may_pickup, max_stack,
        );
    }

    fn pick_all_listens(&self, menu: &str) -> bool {
        sunrise_gameplay_common::game::slot_rules::GameSlotRules::pick_all_listens(self, menu)
    }

    fn pick_all(
        &self,
        menu: &str,
        slot: usize,
        carried: &sunrise_items::items::ItemStack,
        stack: &sunrise_items::items::ItemStack,
        context: sunrise_sim::menu::SlotContext<'_>,
        allowed: &mut bool,
    ) {
        sunrise_gameplay_common::game::slot_rules::GameSlotRules::pick_all(
            self, menu, slot, carried, stack, context, allowed,
        );
    }
}

impl mod_api::AttachmentHost for ClientGame<'_> {
    fn validate_attachment_key(
        &self,
        write: &mod_api::AttachmentStoreKey<'_>,
    ) -> std::result::Result<(), mod_api::AttachmentError> {
        sunrise_gameplay_common::game::mod_api_host_common::GameAttachmentHost::validate_attachment_key(
            self, write,
        )
    }

    fn validate_attachment(
        &self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> std::result::Result<(), mod_api::AttachmentError> {
        sunrise_gameplay_common::game::mod_api_host_common::GameAttachmentHost::validate_attachment(
            self, namespace, name, sync,
        )
    }

    fn attachments(
        &mut self,
        holder: mod_api::Holder,
        access: mod_api::AttachmentAccess,
    ) -> std::result::Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        sunrise_gameplay_common::game::mod_api_host_common::GameAttachmentHost::attachments(
            self, holder, access,
        )
    }
}

#[cfg(all(feature = "client", any(test, feature = "test-support")))]
pub use client_side::native_entity_refs;
