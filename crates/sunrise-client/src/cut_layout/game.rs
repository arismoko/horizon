pub use crate::game_facade::*;
#[cfg(test)]
pub(crate) use resource_load::{load_resources, load_resources_skip_lang, load_resources_uncached};
#[cfg(all(test, feature = "client"))]
pub(crate) use sunrise_client_gameplay::game::client_side::test_key_codes::{
    drop_held_behind_dialog, gameplay_has_keyboard, glfw_button, glfw_modifiers, keyboard_button,
};
#[cfg(all(test, feature = "client"))]
pub(crate) use sunrise_client_gameplay::hud_gates::{block_outline_drawn, debug_crosshair_drawn};
#[path = "../benchmark_retention.rs"]
pub(crate) mod benchmark_retention;
#[cfg(test)]
#[path = "game/modding/container_loot_table_tests.rs"]
mod container_loot_table_tests;
#[cfg(all(test, feature = "client"))]
#[path = "game/network/remote_mob_data/frog_variant_meta_tests.rs"]
mod frog_variant_meta_tests;
#[cfg(all(test, feature = "client"))]
#[path = "game/modding/inventory_menu_parts_tests.rs"]
mod inventory_menu_parts_tests;
#[cfg(all(test, feature = "client"))]
#[path = "game/modding/native_datapack_values_tests.rs"]
mod native_datapack_values_tests;
#[cfg(all(test, feature = "client"))]
#[path = "game/modding/native_living_model_posed_tests.rs"]
mod native_living_model_posed_tests;
#[cfg(all(test, feature = "client"))]
#[path = "game/modding/native_player_post_effects_tests.rs"]
mod native_player_post_effects_tests;
#[cfg(all(test, feature = "client"))]
#[path = "game/interface/pack_layer_frame_tests.rs"]
mod pack_layer_frame_tests;
#[cfg(test)]
#[path = "game/modding/quiet_write_tests.rs"]
mod quiet_write_tests;
#[cfg(any(test, not(feature = "client")))]
#[path = "../server_startup_settings.rs"]
mod server_startup_settings;
#[cfg(any(test, feature = "client"))]
use crate::settings::Settings;
#[cfg(feature = "client")]
use sunrise_client_gameplay::game::client_side::activity;
#[cfg(feature = "client")]
use sunrise_client_gameplay::game::client_side::launch_options_client::read_client_auth;
#[cfg(feature = "client")]
use sunrise_client_gameplay::game::client_side::loading_screen::run_event_loop;
#[cfg(feature = "client")]
use sunrise_client_gameplay::game::client_side::loading_screen::unclean_start_recovery;
#[cfg(feature = "client")]
use sunrise_client_gameplay::game::client_side::loading_window::start_client_game;
#[cfg(feature = "client")]
use sunrise_client_gameplay::game::client_side::narrator;
#[cfg(feature = "client")]
use sunrise_client_gameplay::game::client_side::no_resources;
#[cfg(feature = "client")]
use sunrise_client_gameplay::game::client_side::resource_load_client::StartupLoadContext;
#[cfg(feature = "client")]
use sunrise_client_gameplay::game::client_side::resource_load_client::load_resources_sync;
#[cfg(feature = "client")]
use sunrise_client_gameplay::game::client_side::resource_loader::run_loading;
#[cfg(feature = "client")]
use sunrise_client_gameplay::game::client_side::{mods_client, packs::startup_root};
#[cfg(feature = "client")]
use sunrise_client_lib::audio::Audio;
#[cfg(feature = "client")]
use sunrise_gameplay::game::server_side::integrated::{
    run_component_harness_on_server, serve_with_ticket_activation,
};
use sunrise_gameplay::game::server_side::{
    loading_server::{configure_launch_spans, start_dedicated_game},
    resource_load_server::load_resources_server,
};
use sunrise_gameplay_common::game::api_values::package_snapshot;
use sunrise_gameplay_common::game::native_hooks::bounce_restitution::install_block_contact_hooks;
use sunrise_gameplay_common::game::pack_discovery::data_packs;

#[cfg(test)]
#[path = "game/mobs/allay_jukebox_source_tests.rs"]
mod allay_jukebox_source_tests;
#[cfg(test)]
#[path = "game/mobs/allay_pickup_cooldown_tests.rs"]
mod allay_pickup_cooldown_tests;
#[cfg(test)]
#[path = "game/modding/attachment_instruction_bench.rs"]
mod attachment_instruction_bench;
#[cfg(test)]
#[path = "game/modding/baked_model_keys_tests.rs"]
mod baked_model_keys_tests;
#[cfg(test)]
#[path = "game/blocks/block_entity_reach_draw_tests.rs"]
pub(crate) mod block_entity_reach_draw_tests;
#[cfg(test)]
#[path = "game/blocks/block_part_tool_tests.rs"]
mod block_part_tool_tests;
#[cfg(test)]
#[path = "game/entities/boat_refind_tests.rs"]
mod boat_refind_tests;
#[cfg(test)]
#[path = "game/items/bundle_use_tests.rs"]
mod bundle_use_tests;
#[cfg(test)]
#[path = "game/interface/camera_submersion_tests.rs"]
mod camera_submersion_tests;
#[cfg(test)]
#[path = "game/blocks/clone_block_ticks_tests.rs"]
mod clone_block_ticks_tests;
#[cfg(test)]
#[path = "game/modding/consume_effect_dispatch_tests.rs"]
mod consume_effect_dispatch_tests;
#[cfg(test)]
#[path = "game/interface/contained_fluid_tests.rs"]
mod contained_fluid_tests;
#[cfg(test)]
#[path = "game/mobs/conversion_place_tests.rs"]
mod conversion_place_tests;
#[cfg(test)]
#[path = "game/runtime/core_tests.rs"]
mod core_tests;
#[cfg(test)]
#[path = "game/modding/crafting_recipe_kind_tests.rs"]
mod crafting_recipe_kind_tests;
#[cfg(test)]
#[path = "game/interface/create_fluid_source_tests.rs"]
mod create_fluid_source_tests;
#[cfg(test)]
#[path = "game/items/creator_mod_id_tests.rs"]
mod creator_mod_id_tests;
#[cfg(test)]
#[path = "game/interface/crop_grow_tests.rs"]
mod crop_grow_tests;
#[cfg(test)]
#[path = "game/entities/cushion_refind_tests.rs"]
mod cushion_refind_tests;
#[cfg(test)]
#[path = "game/modding/declared_vegetation_tests.rs"]
mod declared_vegetation_tests;
#[cfg(test)]
#[path = "game/blocks/dispense_container_tests.rs"]
mod dispense_container_tests;
#[cfg(test)]
#[path = "game/items/dropped_by_player_tests.rs"]
mod dropped_by_player_tests;
#[cfg(test)]
#[path = "game/world/dynamic_light_emission_tests.rs"]
mod dynamic_light_emission_tests;
#[cfg(test)]
#[path = "game/world/early_game_rules_tests.rs"]
mod early_game_rules_tests;
#[cfg(test)]
#[path = "game/blocks/empty_contents_tests.rs"]
mod empty_contents_tests;
#[cfg(test)]
#[path = "game/interface/enderman_placement_tests.rs"]
mod enderman_placement_tests;
#[cfg(test)]
#[path = "game/entities/entity_inside_tests.rs"]
mod entity_inside_tests;
#[cfg(test)]
#[path = "game/mobs/equipment_tick_tests.rs"]
mod equipment_tick_tests;
#[cfg(test)]
#[path = "game/interface/experience_points_tests.rs"]
mod experience_points_tests;
#[cfg(test)]
#[path = "game/runtime/falling_air_tick_tests.rs"]
mod falling_air_tick_tests;
#[cfg(test)]
#[path = "game/entities/falling_tick_tests.rs"]
mod falling_tick_tests;
#[cfg(test)]
#[path = "game/blocks/fertile_part_tests.rs"]
mod fertile_part_tests;
#[cfg(test)]
#[path = "game/interface/fluid_interaction_tests.rs"]
mod fluid_interaction_tests;
#[cfg(test)]
#[path = "game/items/glide_wear_tests.rs"]
mod glide_wear_tests;
#[cfg(test)]
#[path = "game/modding/gui_gradient_tests.rs"]
mod gui_gradient_tests;
#[cfg(test)]
#[path = "game/entities/hanging_refind_tests.rs"]
mod hanging_refind_tests;
#[cfg(test)]
#[path = "game/modding/initial_client_models_tests.rs"]
mod initial_client_models_tests;
#[cfg(test)]
#[path = "game/mobs/invulnerable_time_tests.rs"]
mod invulnerable_time_tests;
#[cfg(test)]
#[path = "game/items/item_attack_tests.rs"]
mod item_attack_tests;
#[cfg(test)]
#[path = "game/items/item_interact_mob_tests.rs"]
mod item_interact_mob_tests;
#[cfg(test)]
#[path = "game/items/item_menu_parts_tests.rs"]
mod item_menu_parts_tests;
#[cfg(all(test, feature = "client"))]
#[path = "game/items/item_name_tests.rs"]
mod item_name_tests;
#[cfg(test)]
#[path = "game/modding/item_particle_send_tests.rs"]
mod item_particle_send_tests;
#[cfg(test)]
#[path = "game/interface/item_slot_command_tests.rs"]
mod item_slot_command_tests;
#[cfg(test)]
#[path = "game/entities/kill_die_cancel_tests.rs"]
mod kill_die_cancel_tests;
#[cfg(test)]
#[path = "game/mobs/last_hurt_by_mob_tests.rs"]
mod last_hurt_by_mob_tests;
#[cfg(test)]
#[path = "game/entities/lightning_bolt_uniform_tests.rs"]
mod lightning_bolt_uniform_tests;
#[cfg(test)]
#[path = "game/entities/lightning_item_tests.rs"]
mod lightning_item_tests;
#[cfg(test)]
#[path = "game/entities/mannequin_save_tests.rs"]
mod mannequin_save_tests;
#[cfg(test)]
#[path = "game/mobs/mob_discard_tests.rs"]
mod mob_discard_tests;
#[cfg(test)]
#[path = "game/world/mob_hurt_cooldown_tests.rs"]
mod mob_hurt_cooldown_tests;
#[cfg(test)]
#[path = "game/mobs/mob_landing_support_tests.rs"]
mod mob_landing_support_tests;
#[cfg(test)]
#[path = "game/modding/native_biome_tag_tests.rs"]
mod native_biome_tag_tests;
#[cfg(test)]
#[path = "game/modding/native_block_class_tests.rs"]
mod native_block_class_tests;
#[cfg(test)]
#[path = "game/modding/native_block_use_criteria_tests.rs"]
mod native_block_use_criteria_tests;
#[cfg(test)]
#[path = "game/modding/native_capability_cache_tests.rs"]
mod native_capability_cache_tests;
#[cfg(test)]
#[path = "game/modding/native_chunk_relight_tests.rs"]
mod native_chunk_relight_tests;
#[cfg(test)]
#[path = "game/modding/native_command_completion_tests.rs"]
mod native_command_completion_tests;
#[cfg(test)]
#[path = "game/modding/native_datafix_tests.rs"]
mod native_datafix_tests;
#[cfg(test)]
#[path = "game/modding/native_dynamic_textures_tests.rs"]
mod native_dynamic_textures_tests;
#[cfg(test)]
#[path = "game/modding/native_entity_layers_tests.rs"]
mod native_entity_layers_tests;
#[cfg(test)]
#[path = "game/modding/native_entity_renderers_tests.rs"]
mod native_entity_renderers_tests;
#[cfg(test)]
#[path = "game/modding/native_explosion_destroy_tests.rs"]
mod native_explosion_destroy_tests;
#[cfg(test)]
#[path = "game/modding/native_explosion_resistance_tests.rs"]
mod native_explosion_resistance_tests;
#[cfg(test)]
#[path = "game/modding/native_fire_odds_tests.rs"]
mod native_fire_odds_tests;
#[cfg(test)]
#[path = "game/modding/native_give_stack_tests.rs"]
mod native_give_stack_tests;
#[cfg(test)]
#[path = "game/modding/native_held_out_tests.rs"]
mod native_held_out_tests;
#[cfg(test)]
#[path = "game/modding/native_hover_name_tests.rs"]
mod native_hover_name_tests;
#[cfg(test)]
#[path = "game/modding/native_humanoid_setup_anim_tests.rs"]
mod native_humanoid_setup_anim_tests;
#[cfg(test)]
#[path = "game/modding/native_ignited_by_lava_tests.rs"]
mod native_ignited_by_lava_tests;
#[cfg(test)]
#[path = "game/modding/native_is_fire_source_tests.rs"]
mod native_is_fire_source_tests;
#[cfg(test)]
#[path = "game/modding/native_is_piglin_currency_tests.rs"]
mod native_is_piglin_currency_tests;
#[cfg(test)]
#[path = "game/modding/native_item_bundle_tests.rs"]
mod native_item_bundle_tests;
#[cfg(test)]
#[path = "game/modding/native_knockback_credit_site_tests.rs"]
mod native_knockback_credit_site_tests;
#[cfg(test)]
#[path = "game/modding/native_level_block_states_tests.rs"]
mod native_level_block_states_tests;
#[cfg(test)]
#[path = "game/modding/native_live_holder_tests.rs"]
mod native_live_holder_tests;
#[cfg(test)]
#[path = "game/modding/native_living_renderers_tests.rs"]
mod native_living_renderers_tests;

#[cfg(test)]
#[path = "game/modding/native_living_travel_sweep_tests.rs"]
mod native_living_travel_sweep_tests;
#[cfg(test)]
#[path = "game/modding/native_makes_piglins_neutral_tests.rs"]
mod native_makes_piglins_neutral_tests;
#[cfg(test)]
#[path = "game/modding/native_mob_navigation_flags_tests.rs"]
mod native_mob_navigation_flags_tests;
#[cfg(test)]
#[path = "game/modding/native_on_caught_fire_tests.rs"]
mod native_on_caught_fire_tests;
#[cfg(test)]
#[path = "game/modding/native_part_tables_tests.rs"]
mod native_part_tables_tests;
#[cfg(test)]
#[path = "game/modding/native_place_feature_tests.rs"]
mod native_place_feature_tests;
#[cfg(test)]
#[path = "game/modding/native_player_disconnect_tests.rs"]
mod native_player_disconnect_tests;
#[cfg(test)]
#[path = "game/modding/native_player_persistent_data_tests.rs"]
mod native_player_persistent_data_tests;
#[cfg(test)]
#[path = "game/modding/native_player_place_tests.rs"]
mod native_player_place_tests;
#[cfg(test)]
#[path = "game/modding/native_player_respawn_tests.rs"]
mod native_player_respawn_tests;
#[cfg(test)]
#[path = "game/modding/native_registry_entries_tests.rs"]
mod native_registry_entries_tests;
#[cfg(test)]
#[path = "game/modding/native_registry_history_tests.rs"]
mod native_registry_history_tests;
#[cfg(test)]
#[path = "game/modding/native_registry_processor_tests.rs"]
mod native_registry_processor_tests;
#[cfg(test)]
#[path = "game/modding/native_requires_tool_tests.rs"]
mod native_requires_tool_tests;
#[cfg(test)]
#[path = "game/modding/native_server_max_players_tests.rs"]
mod native_server_max_players_tests;
#[cfg(test)]
#[path = "game/modding/native_smoke_source_tests.rs"]
mod native_smoke_source_tests;
#[cfg(test)]
#[path = "game/modding/native_sound_seed_tests.rs"]
mod native_sound_seed_tests;
#[cfg(test)]
#[path = "game/modding/native_tnt_renderers_tests.rs"]
mod native_tnt_renderers_tests;
#[cfg(test)]
#[path = "game/interface/neighbor_notify_tests.rs"]
mod neighbor_notify_tests;
#[cfg(test)]
#[path = "game/modding/orb_merge_tests.rs"]
mod orb_merge_tests;
#[cfg(test)]
#[path = "game/entities/orb_score_store_tests.rs"]
mod orb_score_store_tests;
#[cfg(test)]
#[path = "game/items/player_destroy_item_tests.rs"]
mod player_destroy_item_tests;
#[cfg(test)]
#[path = "game/modding/player_equipment_change_tests.rs"]
mod player_equipment_change_tests;
#[cfg(test)]
#[path = "game/network/player_kill_credit_tests.rs"]
mod player_kill_credit_tests;
#[cfg(test)]
#[path = "game/world/portal_ticking_tests.rs"]
mod portal_ticking_tests;
#[cfg(test)]
#[path = "game/modding/published_collision_mob_tests.rs"]
mod published_collision_mob_tests;
#[cfg(test)]
#[path = "game/modding/published_shape_game_tests.rs"]
mod published_shape_game_tests;
#[cfg(test)]
#[path = "game/blocks/random_order_tests.rs"]
mod random_order_tests;
#[cfg(test)]
#[path = "game/network/remote_mob_use_tests.rs"]
mod remote_mob_use_tests;
#[cfg(test)]
#[path = "game/blocks/respawn_position_tests.rs"]
mod respawn_position_tests;
#[cfg(test)]
#[path = "game/world/save_format_tests.rs"]
mod save_format_tests;
#[cfg(test)]
#[path = "game/administration/setblock_order_tests.rs"]
mod setblock_order_tests;
#[cfg(test)]
#[path = "game/modding/slot_pickup_tests.rs"]
mod slot_pickup_tests;
#[cfg(test)]
#[path = "game/mobs/spawn_yaw_tests.rs"]
mod spawn_yaw_tests;
#[cfg(test)]
#[path = "game/interface/stand_equipment_change_tests.rs"]
mod stand_equipment_change_tests;
#[cfg(test)]
#[path = "game/entities/stand_low_health_tests.rs"]
mod stand_low_health_tests;
#[cfg(test)]
#[path = "game/modding/start_riding_refusal_tests.rs"]
mod start_riding_refusal_tests;
#[cfg(test)]
#[path = "game/interface/state_at_viewpoint_tests.rs"]
mod state_at_viewpoint_tests;
#[cfg(test)]
#[path = "game/runtime/stateful_block_entity_tests.rs"]
mod stateful_block_entity_tests;
#[cfg(test)]
#[path = "game/mobs/sulfur_prime_phase_tests.rs"]
mod sulfur_prime_phase_tests;
#[cfg(test)]
#[path = "game/blocks/waxing_tests.rs"]
mod waxing_tests;
#[cfg(test)]
#[path = "game/interface/world_fluid_tests.rs"]
mod world_fluid_tests;

fn profile_resolver() -> Box<dyn sunrise_level::profile_lookup::ProfileResolver> {
    Box::new(sunrise_server_profiles_http::CachedProfileResolver::new())
}

fn session_verifier() -> sunrise_sim::lan::SessionVerifier {
    Arc::new(|name, digest, ip| {
        sunrise_server_profiles_http::has_joined(name, digest, ip)
            .map(|opt| {
                opt.map(|p| sunrise_sim::lan::VerifiedSession {
                    uuid: p.uuid,
                    properties: p.properties,
                })
            })
            .map_err(|_| sunrise_sim::lan::SessionVerifyError::Unavailable)
    })
}

fn dedicated_services() -> dedicated::Services {
    dedicated::Services {
        find_name: |name| {
            sunrise_server_profiles_http::find_by_name(
                &sunrise_server_files::profiles::discovery_url(),
                name,
            )
        },
        find_id: |id| {
            sunrise_server_profiles_http::find_by_id(
                &sunrise_server_files::profiles::discovery_url(),
                id,
            )
        },
        profile_keys: || {
            let discovery = sunrise_server_files::profiles::discovery_url();
            sunrise_server_profiles_http::fetch_profile_key_keys(&discovery)
                .map(|keys| keys.player_certificate_keys)
                .unwrap_or_else(|e| {
                    ::log::error!("{e}");
                    Vec::new()
                })
        },
        verify_session: session_verifier(),
    }
}

pub(crate) fn run_with(
    options: Options,
    native: sunrise_mod_package::runtime::Runtime,
) -> Result<()> {
    #[cfg(feature = "client")]
    let read_auth = Some(read_client_auth as fn(&mut Options));
    #[cfg(not(feature = "client"))]
    let read_auth = None;
    let mut options = Options::read(options, read_auth)?;
    #[cfg(feature = "client")]
    {
        options.read_creation_packs =
            Some(sunrise_gameplay::game::creation_packs::read_creation_packs);
    }
    #[cfg(not(feature = "client"))]
    if !options.execution_role.is_dedicated_server() {
        return Err("this binary was built without the `client` feature; pass --dedicated".into());
    }
    options.mods.native = Arc::new(native);
    #[cfg(feature = "client")]
    if options.mods.installation_side != sunrise_mod_package::manifest::Side::Server {
        options.mods.content_materializer =
            Some(sunrise_client::mod_content_client::resolved_resources);
    }
    if options.execution_role == execution_role::ExecutionRole::DedicatedServer
        && !dedicated::preflight(&mut options)?
    {
        return Ok(());
    }
    install_block_contact_hooks();
    // Before any worker starts: on Linux they go behind the server thread
    // and the render thread.
    sunrise_threads::put_helpers_behind();
    if options.execution_role.is_client() {
        // The render thread, like the server thread, ahead of the workers on
        // macOS and Windows. While terrain loads, the workers
        // (sunrise_threads::jobs) and other programs outnumber the cores, and a frame that gave its core up (in a GPU submit) waited
        // 10 to 200 ms to get one back.
        sunrise_threads::favour_current();
    }
    // A dedicated server too: vanilla's "Server Shutdown Thread" (`DedicatedServer.initServer`'s
    // shutdown hook) halts the server and saves its players and levels on SIGTERM, SIGINT or SIGHUP.
    if options.execution_role.is_client()
        || options.execution_role == execution_role::ExecutionRole::DedicatedServer
    {
        sunrise_threads::catch_stop_signals();
    }
    let started = Instant::now();
    // A dedicated server times its spans always, so its slow ring holds the steps of a lag spike (microseconds a tick).
    if options.spikes.is_none()
        && options.execution_role == execution_role::ExecutionRole::DedicatedServer
    {
        options.spikes = Some(dedicated::SLOW_TICK_MS);
    }
    configure_launch_spans(options.spikes.is_some());
    // A player who allows performance statistics also says what their slow frames spent their
    // time in: this thread, which draws the frames, times its steps (wall time only).
    if options.execution_role.is_client()
        && sunrise_telemetry::Consent::load(&sunrise_level::logging::game_home())
            .allows(sunrise_telemetry::Kind::Perf)
    {
        sunrise_level::spans::record_here(true);
    }
    let vanilla_root = options.resources.clone();
    // Saves from before 1.13 upgrade with vanilla's tables, prepared from the player's jar.
    sunrise_world::datafix::set_resources(
        &std::path::absolute(&vanilla_root).unwrap_or_else(|_| vanilla_root.clone()),
    );
    let preparation = match preflight_resources(
        &vanilla_root,
        sunrise_hooks::engine::get_the_game(),
        options.execution_role.is_dedicated_server(),
    ) {
        Ok(profile) => profile,
        Err(error) => {
            // Keep the introductory GUI for an installation with no manifest.
            // A present but unusable preparation needs the exact re-prepare error.
            if !options.offscreen
                && !synchronous_startup(&options)
                && !vanilla_root
                    .join(sunrise_core::prepared_profile::MANIFEST)
                    .is_file()
            {
                log::error!("Sunrise: {error}");
                #[cfg(feature = "client")]
                {
                    let settings = Settings::load(&sunrise_gameplay_common::game::options_path());
                    return no_resources::show(&vanilla_root, &settings);
                }
            }
            return Err(error.into());
        }
    };
    #[cfg(feature = "client")]
    let mut early_settings = Settings::load(&sunrise_gameplay_common::game::options_path());
    #[cfg(not(feature = "client"))]
    let (early_settings, workers) =
        server_startup_settings::load(&sunrise_gameplay_common::game::options_path());
    options
        .mods
        .capture_code_switches(&early_settings.mods.disabled);
    options
        .mods
        .select(&std::env::current_dir()?, &early_settings.mods.enabled);
    mods::validate_startup_packages(&options.mods)?;
    let jar_store = sunrise_level::logging::game_home().join("jars");
    let admission = options.mods.admit_startup_packages(&jar_store)?;
    package_snapshot::install_packages(admission.native_packages(), admission.content_packages())?;
    let instance_dir = sunrise_gameplay_common::game::option_paths::game_dir();
    if let Err(error) = sunrise_gameplay_common::game::native_hooks::config_dir(&instance_dir) {
        log::warn!("Sunrise: config folder: {error}");
    }
    mods::install_registration_data(
        &vanilla_root,
        instance_dir,
        mods::runtime_files::sources(&options.mods),
        mods::package_files::sources(&admission),
    )?;
    {
        sunrise_gameplay_common::game::part_admission::admit_parts(mod_api::__private::mods())?;
        // The structure types those mods registered, handed to the engine before
        // any world opens, as its StructureRegistry reads them.
        if sunrise_world::worldgen::structure::mod_types::install_mod_structure_types(
            native_structure_types::installed_structure_types(),
        )
        .is_err()
        {
            log::warn!("Mod structure types already installed; keeping the first table");
        }
        // Their pool element types, before any world decodes its template pools.
        if sunrise_world::worldgen::structure::mod_elements::install_mod_pool_element_types(
            native_pool_elements::installed_pool_element_types(),
        )
        .is_err()
        {
            log::warn!("Mod pool element types already installed; keeping the first table");
        }
        if sunrise_world::worldgen::structure::mod_processors::install_mod_structure_processor_types(
            native_structure_processors::installed_structure_processor_types(),
        )
        .is_err()
        {
            log::warn!("Mod structure processor types already installed; keeping the first table");
        }
        if sunrise_world::worldgen::mod_rule_tests::install_mod_rule_test_types(
            native_rule_tests::installed_rule_test_types(),
        )
        .is_err()
        {
            log::warn!("Mod rule test types already installed; keeping the first table");
        }
        if sunrise_world::worldgen::mod_rule_tests::install_mod_pos_rule_test_types(
            native_rule_tests::installed_pos_rule_test_types(),
        )
        .is_err()
        {
            log::warn!("Mod position rule test types already installed; keeping the first table");
        }
        // Their feature types, before any world decodes its configured features.
        if sunrise_world::worldgen::mod_features::install_mod_feature_types(
            native_feature_types::installed_feature_types(),
        )
        .is_err()
        {
            log::warn!("Mod feature types already installed; keeping the first table");
        }
        if sunrise_world::worldgen::placement::mod_placements::install_mod_placement_modifier_types(
            native_placement_modifiers::installed_placement_modifier_types(),
        )
        .is_err()
        {
            log::warn!("Mod placement modifier types already installed; keeping the first table");
        }
        // The levels they registered, below the data packs of every world opened.
        if sunrise_world::worldgen::overrides::install_mod_dimensions(
            native_dimensions::installed_dimensions(),
        )
        .is_err()
        {
            log::warn!("Mod dimensions already installed; keeping the first table");
        }
        // Their chunk generators and biome sources, which those levels' and the packs' stems name.
        native_chunk_generator::install();
        native_biome_source::install();
        mods::register_components(&vanilla_root)?;
        sunrise_sim::item_sync::install_consume_effect_codecs()?;
        #[cfg(feature = "client")]
        mods_client::register_standalone_models(options.execution_role);
        #[cfg(feature = "client")]
        mods_client::register_layer_definitions(options.execution_role, &vanilla_root)?;
        #[cfg(feature = "client")]
        mods_client::register_debug_entries(options.execution_role);
        #[cfg(feature = "client")]
        mods_client::register_gui_layers(options.execution_role);
        #[cfg(feature = "client")]
        mods_client::register_client_tooltip_component_factories(options.execution_role);
        mods::register_map_decoration_types();
        #[cfg(feature = "client")]
        mods_client::register_range_select_item_model_properties(options.execution_role);
        #[cfg(feature = "client")]
        mods_client::register_select_item_model_properties(options.execution_role);
        #[cfg(feature = "client")]
        mods_client::register_resource_sources(options.execution_role);
        sunrise_gameplay::game::mods::register_enchantment_effects();
        #[cfg(feature = "client")]
        mods_client::register_client_presentations(options.execution_role, &vanilla_root)?;
        #[cfg(feature = "client")]
        mods_client::register_meshing(options.execution_role);
        #[cfg(feature = "client")]
        mods_client::register_particles(options.execution_role)?;
        mods::freeze_declarations();
    }
    // Minecraft.<init>: read the last start's flag, then mark this one as not
    // finished until onGameLoadFinished. A dedicated server has no options.txt.
    #[cfg(feature = "client")]
    if options.execution_role.is_client() {
        if unclean_start_recovery(&mut early_settings) {
            log::warn!(
                "Detected unexpected shutdown during last game startup: resetting fullscreen mode"
            );
        }
        early_settings.file.set(
            "startedCleanly",
            sunrise_client::options_file::V::Bool(false),
        );
        early_settings.save(&sunrise_gameplay_common::game::options_path());
    }
    // Minecraft's constructor opens the narrator; a server has none.
    #[cfg(feature = "client")]
    if options.execution_role.is_client() {
        narrator::start(&early_settings);
    }
    // Someone is playing: the anonymous ping behind the launcher's "N playing now", and the
    // official server in their list (servers.dat in the game folder, the working directory). Not
    // for a server, a benchmark, a run that quits by itself, a harness's fixture or a run that
    // sunrise-ctl drives: those start in a source checkout and wrote the list there.
    if !options.offscreen && !synchronous_startup(&options) {
        let version = benchmark_retention::version();
        let game_dir = sunrise_level::logging::player_dir();
        sunrise_hooks::engine::started(&sunrise_hooks::Started::new(&version, &game_dir));
    }
    // A game window's hooks (Discord's presence): never for a server.
    #[cfg(feature = "client")]
    activity::client_started(&options);
    // Snapshots of derived tables live in the game folder, never beside the
    // shared resources.
    Behaviours::set_cache_dir(std::env::current_dir()?.join(".sunrise-cache"));
    let ticket_activation = crate::entry::ticket_activation();
    #[cfg(feature = "client")]
    let serve_integrated: sunrise_gameplay_common::game::integrated_link::Serve =
        |res, pack, options, settings, launch, flags, tasks, ready, resolver| {
            serve_with_ticket_activation(
                res,
                pack,
                options,
                settings,
                launch,
                flags,
                tasks,
                ready,
                resolver,
                crate::entry::ticket_activation(),
            )
        };
    #[cfg(feature = "client")]
    if !synchronous_startup(&options) {
        return run_loading(
            options,
            early_settings,
            vanilla_root,
            None,
            started,
            serve_integrated,
            run_component_harness_on_server,
            session_verifier(),
            profile_resolver,
        );
    }
    #[cfg(feature = "client")]
    if !options.execution_role.is_dedicated_server() {
        let roots = startup_root(
            &vanilla_root,
            &early_settings.resource_packs,
            &[],
            &options.mods,
        )?;
        let mut loaded = load_resources_sync(
            roots,
            data_packs(&options.mods),
            StartupLoadContext {
                vanilla_root: &vanilla_root,
                launch_world: launch_world(&options),
                started,
            },
            &mut early_settings,
            mods::LoadRole::startup(options.execution_role),
            &Progress::new(),
            &options.mods,
        )?;
        options.resources = loaded.root.clone();
        let resources = loaded.resources;
        let audio = if !options.execution_role.is_client() || options.headless {
            Audio::silent(&loaded.pack.source)
        } else {
            Audio::with_device(&loaded.pack.source, &early_settings.sound_device)
        };
        let game = start_client_game(
            &resources,
            loaded.client_res.ok_or("missing client resource slot")?,
            loaded.pack,
            loaded.client_pack.ok_or("missing client pack")?,
            loaded.gui.ok_or("missing menu bundle")?,
            options,
            loaded.pack_error,
            audio,
            vanilla_root,
            started,
            loaded.rest_loader,
            early_settings,
            None,
            std::mem::take(&mut loaded.client_applies),
            loaded.entity_models,
            None,
            serve_integrated,
            run_component_harness_on_server,
            session_verifier(),
            profile_resolver,
        )?;
        if let Some(game) = game {
            run_event_loop(game)?;
        }
        return Ok(());
    }
    let base = mods::content_root(&vanilla_root, &options.mods)?;
    let root = match preparation {
        sunrise_core::prepared_profile::Profile::Full => base,
        sunrise_core::prepared_profile::Profile::Server => base.with_server_profile(),
    };
    let prepared_root = root.prepared().to_path_buf();
    #[cfg(feature = "client")]
    let settings = early_settings.sim_settings();
    #[cfg(not(feature = "client"))]
    let settings = early_settings;
    let (resources, pack) = load_resources_server(
        root,
        data_packs(&options.mods),
        &vanilla_root,
        &settings,
        #[cfg(feature = "client")]
        early_settings.sunrise.worker_threads(),
        #[cfg(not(feature = "client"))]
        workers.count(),
        started,
        &options.mods,
        &Progress::new(),
        cfg!(feature = "client"),
    )?;
    options.resources = prepared_root;
    start_dedicated_game(
        &resources,
        pack,
        options,
        settings,
        started,
        None,
        dedicated_services(),
        profile_resolver,
        benchmark_retention::keep_benchmark,
        ticket_activation,
    )
}

#[path = "game/server_side.rs"]
#[doc(hidden)]
pub mod server_side;
#[cfg(test)]
#[path = "game/interface/tests/sign_use_tests.rs"]
mod sign_use_tests;

#[cfg(test)]
pub(crate) use server_side::{
    entity_server, game_tests, native_entity_refs, native_guest_turn_tests,
};

#[path = "game/runtime/transactions.rs"]
#[doc(hidden)]
pub mod transactions;

#[cfg(all(test, feature = "client"))]
#[path = "game/runtime/resource_load.rs"]
#[doc(hidden)]
pub mod resource_load;
#[cfg(all(test, not(feature = "client")))]
#[path = "game/runtime/resource_load_server_tests.rs"]
#[doc(hidden)]
pub mod resource_load;

#[path = "game/network/remote/mod.rs"]
#[doc(hidden)]
pub mod remote;

#[path = "game/network/lan_actions/mod.rs"]
#[doc(hidden)]
pub mod lan_actions;

#[doc(hidden)]
pub mod native_hooks;

#[path = "game/modding/payload_transport.rs"]
#[doc(hidden)]
pub mod payload_transport;

#[path = "game/modding/pack_finders/mod.rs"]
#[doc(hidden)]
pub mod pack_discovery;

#[path = "game/modding/native_world_read.rs"]
#[doc(hidden)]
pub mod native_world_read_common;

#[path = "game/modding/native_tags.rs"]
#[doc(hidden)]
pub mod native_tags;

#[path = "game/modding/native_surface.rs"]
#[doc(hidden)]
pub mod native_surface;

#[path = "game/modding/native_state_shapes.rs"]
#[doc(hidden)]
pub mod native_state_shapes;

#[path = "game/modding/native_server.rs"]
#[doc(hidden)]
pub mod native_server;

#[path = "game/modding/native_rotate.rs"]
#[doc(hidden)]
pub mod native_rotate;

#[path = "game/modding/native_resources.rs"]
#[doc(hidden)]
pub mod native_resources;

#[path = "game/modding/native_player_reach.rs"]
#[doc(hidden)]
pub mod native_player_reach;

#[path = "game/modding/native_player_death.rs"]
#[doc(hidden)]
pub mod native_player_death;

#[path = "game/modding/native_passages.rs"]
#[doc(hidden)]
pub mod native_passages;

#[path = "game/modding/native_living_blocking.rs"]
#[doc(hidden)]
pub mod native_living_blocking;

#[path = "game/modding/native_item_registry.rs"]
#[doc(hidden)]
pub mod native_item_registry;

#[path = "game/modding/native_entity_selectors.rs"]
#[doc(hidden)]
pub mod native_entity_selectors;

#[path = "game/modding/native_effect_category.rs"]
#[doc(hidden)]
pub mod native_effect_category;

#[path = "game/modding/native_datapack_refs.rs"]
#[doc(hidden)]
pub mod native_datapack_refs;

#[path = "game/modding/native_data_bound.rs"]
#[doc(hidden)]
pub mod native_data_bound;

#[path = "game/modding/native_configs.rs"]
#[doc(hidden)]
pub mod native_configs;

#[path = "game/modding/native_components.rs"]
#[doc(hidden)]
pub mod native_components;

#[path = "game/modding/native_cauldron_contents.rs"]
#[doc(hidden)]
pub mod native_cauldron_contents;

#[path = "game/modding/native_block_states.rs"]
#[doc(hidden)]
pub mod native_block_states;

#[path = "game/modding/native_block_sounds.rs"]
#[doc(hidden)]
pub mod native_block_sounds;

#[path = "game/modding/native_block_extra_models.rs"]
#[doc(hidden)]
pub mod native_block_extra_models;

#[path = "game/modding/mod_api_host.rs"]
#[doc(hidden)]
pub mod mod_api_host;

#[path = "game/modding/load_conditions.rs"]
#[doc(hidden)]
pub mod load_conditions;

#[path = "game/modding/entity_sections.rs"]
#[doc(hidden)]
pub mod entity_sections;

#[path = "game/modding/api_values.rs"]
#[doc(hidden)]
pub mod api_values;

#[path = "game/mobs/creatures/mod.rs"]
#[doc(hidden)]
pub mod creatures;

#[path = "game/mobs/breezes.rs"]
#[doc(hidden)]
pub mod breezes;

#[path = "game/items/equip_answers.rs"]
#[doc(hidden)]
pub mod equip_answers;

#[path = "game/interface/chat_common.rs"]
#[doc(hidden)]
pub mod chat_common;

#[cfg(all(test, feature = "client"))]
#[doc(hidden)]
pub mod host;

#[path = "game/entities/residency_tick.rs"]
#[doc(hidden)]
pub mod residency_tick;

#[path = "game/entities/minecarts.rs"]
#[doc(hidden)]
pub mod minecarts;

#[path = "game/entities/general.rs"]
#[doc(hidden)]
pub mod general;

#[path = "game/entities/fireworks.rs"]
#[doc(hidden)]
pub mod fireworks;

#[path = "game/entities/closest_space.rs"]
#[doc(hidden)]
pub mod closest_space;

#[cfg(all(test, feature = "client"))]
#[path = "game/client_side.rs"]
#[doc(hidden)]
pub mod client_side;

#[path = "game/blocks/piston_push.rs"]
#[doc(hidden)]
pub mod piston_push;

#[path = "game/blocks/piston_carry.rs"]
#[doc(hidden)]
pub mod piston_carry;

#[path = "game/blocks/cauldron_interactions.rs"]
#[doc(hidden)]
pub mod cauldron_interactions;

#[path = "game/blocks/block_items.rs"]
#[doc(hidden)]
pub mod block_items;

#[cfg(all(test, feature = "client"))]
#[path = "game/integrated_pair.rs"]
mod integrated_pair;
