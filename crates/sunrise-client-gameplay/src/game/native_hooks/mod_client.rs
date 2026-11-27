pub use mod_api::ExperienceOrbRef;
pub use mod_api::NameTagLine;
pub use mod_api::ShaderPackState;
mod client_visual_text;
pub mod translate;

use self::screen_view::{screen_view_for_mod, screen_view_with_id};
use crate::game::client_side::key_codes::{glfw_button, glfw_modifiers};
#[allow(unused_imports)]
use crate::game::client_side::native_panel_recipes::RecipePage;
use crate::game::client_side::screen_build::ObservedScreen;
use mod_api::CameraType;
use mod_api::{
    self, Answered, ClientModifiers, EntityRef, PlayerRef, RecipeBookCategory, ScreenId,
    ScreenOpening, ScreenTarget,
};
pub use mod_api::{BlockRim, ColoredLight, EnchantmentEffects, PanelEvent, ScreenView};
use mod_api::{FluidStack, LogicalSide, TriState};
use std::{sync::Arc, time::Duration};
use sunrise_client::{
    extras::Corner,
    hud::{HudBatch, rgba},
    keybinds,
    music::{Music, MusicLayer},
};
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
pub use sunrise_gameplay_common::game::native_hooks::ExecutionRole;
pub use sunrise_gameplay_common::game::native_hooks::NativeRecipeView;
use sunrise_gameplay_common::game::native_hooks::client_packet_view;
#[cfg(test)]
use sunrise_gameplay_common::game::native_hooks::mod_menu;
use sunrise_gameplay_common::game::native_hooks::mod_settings::ModSettingsGameExt as _;
use sunrise_gameplay_common::game::native_hooks::screen_chars::chars_to_type;
use sunrise_gameplay_common::game::native_hooks::serverbound_packet_view;
use sunrise_gameplay_common::game::native_hooks::sound_and_shader_helpers::sound_id;
pub use sunrise_gameplay_common::game::native_hooks::{
    ActiveEffect, AttachmentStore, Attack, BLOCK_CONTACT_HOOKS, BlockClick, BlockClickAnswer,
    BlockEntityDataSource, BlockEntityRef, BlockKey, BlockPos, BlockRegistry,
    BlockStateDefinitionRegistration, BlockTintSources, BlockUpdateFlags, BonemealAsk,
    BonemealCheck, BonemealSource, CauldronFluidContents, CauldronInteractions, ChunkRef,
    ClientMiningPrediction, ClientPacket, ClientPayload, ClientReloadListeners, ClockAdjustment,
    ColorResolvers, Compound, ConnectionKind, ConnectsToCall, ContainerLootFilled, CreativeCache,
    DAMAGE_HOOKS, DamageContainer, DamageHooks, DamageLevel, DataMapType, DebugEntry,
    DebugRenderer, Despawn, Direction, EffectKey, EnchantmentInfo, EntityLayers, EntityRenderers,
    ExplosionStart, FluidKind, FrameModel, FrameModelEdit, FxHashMap, GameLevel, GameMode,
    GameRuleValue, GameTests, GuiLayerRegistrar, HandDrawContext, HandRender, HitLevel, HurtSource,
    Hurting, Identifier, Ingredient, InsideEffects, InteractionAttempt, InteractionKey,
    ItemAfterVerticalCollisionCall, ItemClick, ItemEntityRef, ItemFished, ItemModelRegistration,
    ItemTintSourceRegistration, JsonAnimationTypeRegistration, Knock, LayerDefinitionRegistration,
    LevelAccess, LevelDimension, LevelRef, LevelView, LifecycleContext, LiveDamageContainer,
    LiveDamageRead, LivingRef, LoadConditions, LootDrops, MOD_MENUS, MapDecorationType, MobRef,
    ModBlockEntity, ModBlockEntityFields, ModGameExt1, ModelLoaderRegistration, NativeHookHost,
    NativeRecipeLayout, NeighbourUpdate, NoteBlockChange, ParticleGroupRegistration,
    ParticleOptions, ParticleProviders, ParticleType, Path, PathBuf, PathComputationType,
    PermissionLevel, PhantomSpawnResult, Placement, PlacementContext, PoiTypeExtensions,
    PortalShape, PresetEditorFactoryRegistration, RailShape, RangeSelectPropertyRegistration,
    ScheduleResult, Screen, ScreenWidgets, ScreenshotImage, SelectPropertyRegistration,
    ServerReloadListeners, ServerTickPhase, SharedGameExt1, ShieldBlock, SkullModel, SleepProblem,
    SpawnEntry, SpecialModelRenderers, SpriteSourceRegistration, StandaloneModelRegistration,
    Suggestion, Tag, TextureAtlasRegistration, TickPriority, Ticks, ToolUse, TooltipElement,
    TryIntoApi, TryIntoEngine, UpdateShapeCall, UseAnimation, UsePhase, Victim,
    VillagerProfessions, Who, add_pack_finders, add_section_geometry, add_server_reload_listeners,
    adjacent_block_path_type, affect_neighbors_after_removal, animal_tame, anvil_update,
    apply_enchantments, apply_menu_button_cursor, armor_hurt, arrow_nock,
    authoritative_world_for_mods, baby_entity_spawn, beacon_color_multiplier, block_drops,
    block_entity_load_additional, block_entity_neighbors, block_entity_on_load_routes_tests,
    block_entity_on_load_tests, block_entity_read, block_entity_save_additional,
    block_entity_save_additional_tests, block_entity_type_add_blocks, block_grow_feature,
    block_landing, block_path_type, block_sticky, block_tool_modification, block_use_item_on,
    block_use_result, block_use_without_item, bonemeal, bounce_restitution, camera_math,
    can_be_connected_to, can_be_hydrated, can_continue_sleeping, can_drop_from_explosion,
    can_entity_destroy, can_equip, can_harvest_block, can_perform_action, can_place_item,
    can_player_sleep, can_sustain_plant, chat_recipient, chunk_ticket_level_updated, chunk_watch,
    client_information_updated, client_pause_read, clone_item_stack, config_dir, connects_to,
    console_line, container_loot_filled, crafting_remainder, create_fluid_source,
    create_skull_models, create_spawn_position, creator_mod_id, crop_grow, custom_click_action,
    custom_spawners, damage_item, damage_post, damage_pre, data_maps_updated,
    default_components_bound, dimension_id, dispense_method, effect_particle_modification,
    empty_contents, enchant_power_bonus, enchanted_block_loot, enchanted_entity_loot,
    enchantment_level_set, enchantments, enderman_anger, engine_reduction, entity_change_block_pre,
    entity_constructing, entity_entering_section, entity_inside, entity_interact_specific,
    entity_invulnerability_check, entity_mob_griefing, entity_place, entity_size,
    entity_struck_by_lightning, entity_travel_to_dimension, execution_role, exp_drop,
    explosion_detonate, explosion_knockback, explosion_notice_containment, explosion_resistance,
    explosion_start, extend_poi_types, falling_tick, farmland_trample, fence_gate_in_wall,
    finalize_spawn, fire_behaviour, fire_odds, fishing_medium, flags, fluid_place_block,
    fluid_rules, fluid_tick, friction, fs, game_rule_changed, game_shutting_down, grindstone_place,
    grindstone_take, hand_pose_matrix, hurt_server_args, ignited_by_lava, incoming_damage,
    incoming_tests, ingredient_test, install_block_contact_hooks, install_client_hooks,
    invulnerability_check, is_fertile, is_fire_source, is_not_replaceable_by_pick_action,
    is_pathfindable, is_piglin_currency, is_possible_to_respawn_in, is_primary_item_for,
    item_after_vertical_collision, item_attack, item_container, item_crafted, item_destroyed,
    item_fallthrough, item_fished, item_interact_mob, item_smelted, item_stacked_on_other,
    item_toss, knockback, level_helpers, level_save, level_sounds, level_tick_tests, level_world,
    light_emission, living_breathe, living_change_target, living_conversion_post,
    living_conversion_pre, living_damage_post, living_damage_pre, living_death,
    living_death_message, living_death_post, living_destroy_block, living_drops,
    living_experience_drop, living_get_projectile, living_shield_block, living_use_totem,
    living_visibility, loot, makes_piglins_neutral, map_decoration_types, math, menu_links,
    menu_slot_changed, menu_slot_rules, menu_slot_take, menu_transfer, minecart_pass,
    mob_effect_events, mob_kind, mob_source, mob_spawn_checks, mob_split, mod_api_host,
    mod_data_root, mod_list, mod_mismatch, mod_particles_ok, mod_settings, must_survive,
    native_add_section_geometry, native_block_entity_on_load, native_block_entity_type_add_blocks,
    native_block_entity_update_tag, native_blocks, native_can_place_item, native_component_types,
    native_create_skull_models, native_extend_poi_types, native_items, native_map_decoration_types,
    native_menu_button, native_menu_quick_move, native_menu_slot_take, native_note_block_change,
    native_piston_push_reaction, native_potential_spawns, native_register_block_state_models,
    native_register_block_tint_sources, native_register_cauldron_fluid_content,
    native_register_cauldron_interaction, native_register_color_resolvers,
    native_register_debug_entries, native_register_debug_renderers,
    native_register_dimension_transition_screen, native_register_entity_renderers,
    native_register_entity_spectator_shaders, native_register_fluid_models,
    native_register_item_models, native_register_item_tint_sources,
    native_register_json_animation_types, native_register_layer_definitions,
    native_register_load_conditions, native_register_model_loaders,
    native_register_particle_groups, native_register_particle_providers,
    native_register_preset_editors, native_register_range_select_item_model_property,
    native_register_render_pipelines, native_register_select_item_model_property,
    native_register_special_model_renderer, native_register_sprite_sources,
    native_register_texture_atlases, native_register_villager_professions,
    native_standalone_models, native_village_siege, neighbor_notify, note_block_play,
    on_block_exploded, on_block_placed_by, on_block_state_change, on_caught_fire,
    on_chunk_unloaded, on_destroyed_by_player, on_destroyed_by_push_reaction, on_dropped_by_player,
    on_item_use_first, packet_types, packet_view, permissions_changed, piston_carries_block_entity,
    piston_push_reaction, place_liquid, placement_context, player_bed_leave, player_break_speed,
    player_brewed_potion, player_destroy_item, player_destroyed_block, player_enchant_item,
    player_flyable_fall, player_game_mode_changed, player_harvest_check, player_inventory_changed,
    player_item_smithed, player_leave, player_move, player_passage_cross, player_pong,
    player_set_spawn, player_spawn_phantoms, player_start_spectating_entity, player_swap_hands,
    player_xp_change, player_xp_level_change, portal_spawn, potion_brew_post, potion_brew_pre,
    pre_remove_side_effects, property_change, rails, recipe_book_categories, recipe_book_place,
    recipe_result, recipe_sources, register_block_state_models, register_block_tint_sources,
    register_cauldron_fluid_content, register_cauldron_interaction, register_color_resolvers,
    register_debug_entries, register_debug_renderers, register_dimension_transition_screen,
    register_entity_renderers, register_entity_spectator_shaders, register_fluid_models,
    register_game_tests, register_item_models, register_item_tint_sources,
    register_json_animation_types, register_layer_definitions, register_load_conditions,
    register_model_loaders, register_particle_groups, register_particle_providers,
    register_range_select_item_model_property, register_render_pipelines,
    register_select_item_model_property, register_special_model_renderer, register_sprite_sources,
    register_texture_atlases, register_villager_professions, registered_potions, remove_block,
    respawn_position, right_click_item, runtime_environment, screen_chars, server_chat,
    server_tick_rate_changed, set_placed_by, shield_block, should_check_weak_power,
    should_redstone_wire_connect_to, sound_and_shader_helpers, spread_players_teleport,
    startup_math_profile, stateful_block_entity, supported, supports_enchantment, sweep_attack,
    tags_updated, teleport_command, tool_modified_state, use_item_on_block, use_totem,
    vanilla_game_event, wind_burst, with_property, world_for_mods, xp_orb_targeting,
    xp_repair_ratio,
};
#[cfg(feature = "test-support")]
pub use sunrise_gameplay_common::game::native_hooks::{block_entity_for_block, item_fall_on_tests};
#[cfg(all(feature = "test-support", not(test)))]
pub use sunrise_gameplay_common::game::native_hooks::{slot_icon_tests, slot_tooltip_tests};
use sunrise_items::items::ItemStack;
use sunrise_level::{biome::BiomeColors, block_entities::Pos, containers::BlockEntity};
use sunrise_mod_package::mod_list::{
    Installed, KeyMapping, NativePage, NativeSetting, NativeSettingValue,
};
use sunrise_sim::first_person_sim::Hand;
use sunrise_world::registry::BlockStateId;
use winit::{event::MouseButton, keyboard::KeyCode};

impl mod_api::PackDiscoveryContext for crate::game::ClientGame<'_> {
    fn live_host(&mut self) -> Option<mod_api::HostSide<'_>> {
        Some(mod_api::HostSide::Client(self))
    }

    fn client_text(&self) -> Option<&dyn mod_api::prelude::ClientTextClientHost> {
        Some(self)
    }
}

#[path = "base_client.rs"]
pub(in crate::game) mod base;

#[path = "block_entity_client_tick_client.rs"]
pub(in crate::game) mod block_entity_client_tick;

#[path = "block_entity_data_received.rs"]
pub(in crate::game) mod block_entity_data_received;

#[path = "block_entity_on_load_client.rs"]
pub(in crate::game) mod block_entity_on_load;

#[path = "build_creative_mode_tab_contents_client.rs"]
pub(in crate::game) mod build_creative_mode_tab_contents;

#[path = "can_start_swimming_client.rs"]
pub(in crate::game) mod can_start_swimming;

#[path = "camera_read_client.rs"]
pub(in crate::game) mod camera_read;

#[path = "client_gui_state_client.rs"]
pub(in crate::game) mod client_gui_state;

#[path = "client_input_reads_client.rs"]
pub(in crate::game) mod client_input_reads;

#[path = "client_input_reads_tests_client.rs"]
pub(in crate::game) mod client_input_reads_tests;

#[path = "client_pick_client.rs"]
pub(in crate::game) mod client_pick;

#[path = "client_text_client.rs"]
pub(in crate::game) mod client_text;

#[path = "creative_tabs_read_client.rs"]
pub(in crate::game) mod creative_tabs_read;

#[path = "draw_measures_client.rs"]
pub(in crate::game) mod draw_measures;

#[path = "fluid_draw_client.rs"]
pub(in crate::game) mod fluid_draw;

#[path = "gui_ticks_client.rs"]
pub(in crate::game) mod gui_ticks;

#[path = "hud_mode_client.rs"]
pub(in crate::game) mod hud_mode;

#[path = "item_picked_up_client.rs"]
pub(in crate::game) mod item_picked_up;

#[path = "menu_extra_data_client.rs"]
pub(in crate::game) mod menu_extra_data;

#[path = "muffle_sounds_client.rs"]
pub(in crate::game) mod muffle_sounds;

#[path = "panel_recipes_client.rs"]
pub(in crate::game) mod panel_recipes;

#[path = "preset_title_client.rs"]
pub(in crate::game) mod preset_title;

#[path = "register_client_item_extensions.rs"]
pub(in crate::game) mod register_client_item_extensions;

#[path = "register_client_tooltip_component_factories.rs"]
pub mod register_client_tooltip_component_factories;

#[path = "register_entity_layers.rs"]
pub(in crate::game) mod register_entity_layers;

#[path = "register_gui_layers.rs"]
pub(in crate::game) mod register_gui_layers;

#[path = "register_preset_editors_client.rs"]
pub(in crate::game) mod register_preset_editors;

#[path = "screen_hovered_stack_client.rs"]
pub(in crate::game) mod screen_hovered_stack;

#[path = "use_item_hand_client.rs"]
pub(in crate::game) mod use_item_hand;

#[path = "shared/book_screen_client.rs"]
pub mod book_screen;

#[path = "shared/system_helpers_client.rs"]
pub(in crate::game) mod system_helpers;

#[path = "shared/world_helpers_client.rs"]
pub(in crate::game) mod world_helpers;

#[cfg(any(test, feature = "test-support"))]
pub fn seed_native_menu(game: &mut crate::game::ClientGame<'_>, window: i32, name: &str) -> u64 {
    if game.client.remote.is_none() {
        let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Native screen fixture").unwrap();
    }
    if game.client.remote.as_ref().unwrap().menu.as_ref().is_some() {
        game.remote_close_local();
    }
    let menu = &mut game.client.remote.as_mut().unwrap().menu;
    menu.set(Some((window, name.to_owned())));
    menu.generation
}

impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn client_acting_id(&mut self) -> u128 {
        self.own_uuid()
    }
    pub(in crate::game) fn client_clicker(&mut self) -> (u128, i32) {
        (self.own_uuid(), self.own_entity_id())
    }
    pub(in crate::game) fn client_acting_or_local_player(&self) -> Option<PlayerRef> {
        self.local_player()
    }
    pub(in crate::game) fn client_chunks_loaded(&mut self) {
        self.common.world.loaded_chunks.clear();
    }
    pub(in crate::game) fn client_chunks_unloaded(
        &mut self,
        _unloaded: &[std::sync::Arc<sunrise_level::chunk::Chunk>],
    ) {
    }
    pub(in crate::game) fn client_apply_after_use_components(&mut self, stack: &ItemStack) {
        self.client_apply_use_cooldown(stack);
    }
    pub(in crate::game) fn observe_client_packet(
        &mut self,
        packet: &sunrise_protocol::packets::play::Clientbound,
    ) {
        if !mod_api::__private::listened(mod_api::HookId::ClientPacket) {
            return;
        }
        let view = client_packet_view(packet);
        self.native_client_packet(&view);
    }
    /// Whether a mod cancelled `packet` on its way to the server (`client_packet_send`).
    /// Builds the packet's view only when a mod listens.
    pub fn client_packet_send_canceled(
        &self,
        packet: &sunrise_protocol::packets::play::Serverbound,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ClientPacketSend) {
            return false;
        }
        let mut canceled = false;
        self.native_client_packet_send(&serverbound_packet_view(packet), &mut canceled);
        canceled
    }
    /// The staged or shown native screen belonging to this opening, not a
    /// prior opening of the same numeric window id.
    pub(in crate::game) fn active_native_screen(&self) -> Option<&OpenNativeScreen> {
        let screen = self.client.mod_screen.0.as_ref()?;
        let remote = &self.client.remote.as_ref()?.menu;
        (!screen.detached
            && remote.generation == screen.menu.generation
            && remote.as_ref()?.0 == screen.menu.window)
            .then_some(screen)
    }
    /// What `set_screen` does once the hooks have answered: `screen` opens,
    /// and the mod `owner`'s own screen when one answered `ModScreen`.
    pub(in crate::game) fn apply_screen(&mut self, screen: Screen, owner: Option<&str>) {
        self.client.pointer_grab_request.set(None);
        if self.client.screen_grab {
            self.capture(false);
        }
        // `before` may already have closed it. Overlays that keep Inventory
        // underneath don't pass through this replacement path.
        if screen != Screen::Inventory {
            if self.active_native_screen().is_some() {
                self.remote_close_container();
            } else if self
                .client
                .mod_screen
                .0
                .as_ref()
                .is_some_and(|s| s.detached)
            {
                self.remove_remote_menu_screen();
            }
        }
        if screen != Screen::DemoIntro {
            self.client.demo_intro = None;
        }
        if self.client.screen == Screen::Chat && screen != Screen::Chat {
            self.remove_chat_screen(crate::game::client_side::chat::ChatExit::Interrupted);
        }
        self.client.screen = screen;
        self.client.native_mod_screen_parent = false;
        if let Some(owner) = owner {
            self.client.native_mod_screen = Some(owner.to_owned());
        } else if screen != Screen::Mod {
            self.client.native_mod_screen = None;
        }
        if screen != Screen::Playing {
            self.release_mappings_on_screen_open();
        }
        if screen == Screen::Mod {
            self.clear_device_keys();
            self.set_mapping_held("key.attack", false);
            self.set_mapping_held("key.use", false);
            self.set_client_mining(None);
            self.capture(false);
        }
        self.commit_screen_transition();
    }
    /// Runs close cleanup for any screen or overlay being replaced by
    /// `mod_screen_open` once hooks allow the transition, matching vanilla's
    /// `Gui.setScreen` / `Screen.removed` so no container menu, editor, dialog,
    /// or prompt stays open over the new screen.
    pub(crate) fn close_replaced_screen_for_mod(&mut self) {
        let replacing = std::mem::replace(&mut self.client.screen_state.0.replacing, true);
        match self.client.screen {
            Screen::Inventory => self.close_container_client(),
            Screen::Creative => self.close_creative_menu(),
            Screen::Options => self.client.chat_ui.options.discard_draw(),
            _ => {}
        }
        if self.client.book.is_some() {
            self.close_book();
        }
        if self.client.sign_edit.is_some() {
            self.close_sign_editor();
        }
        if self.dialog_open() {
            self.close_dialogs();
        }
        self.client.link_prompt = None;
        self.client.command_prompt.0 = None;
        self.client.code_of_conduct = None;
        self.client.server_packs.0.prompt = None;
        self.client.screen_state.0.replacing = replacing;
    }
    /// `LocalPlayer.clientSideCloseContainer`: restore the logical inventory
    /// menu, then `Gui.setScreen(null)`. Keep the old screen and its view for
    /// `ScreenEvent.Closing` before running its removal, not its user close.
    pub(crate) fn close_screen_to_level(
        &mut self,
        show_death_screen: bool,
    ) -> sunrise_game_data::Result<()> {
        let overlay = self.observed_overlay_now();
        let inventory_screen = overlay.is_none()
            && self.client.screen == Screen::Inventory
            && self.remote_menu_type().is_none()
            && self.shown_native_screen().is_none();
        let heard = self.screen_hooks_heard();
        let outgoing =
            heard
                .then(|| {
                    if let Some(id) = overlay {
                        Some(self.observe_view(self.screen_overlay_view(id)))
                    } else if self.client.screen != Screen::Playing {
                        Some(self.observe_view(self.screen_change_view(self.client.screen)))
                    } else {
                        self.client.title.0.as_ref().map(|title| {
                            self.observe_view(self.screen_overlay_view(title.mode.id()))
                        })
                    }
                })
                .flatten();
        let detach_native = self.shown_native_screen().is_some();
        self.remote_set_menu(None, None);
        if detach_native && let Some(screen) = self.client.mod_screen.0.as_mut() {
            screen.detached = true;
        }
        let has_level = self.client.remote.as_ref().is_some_and(|r| r.has_level());
        let dead = self
            .common
            .player
            .as_ref()
            .is_some_and(|player| player.health.health() <= 0.0);
        let title = if has_level {
            None
        } else {
            Some(
                self.new_title(std::env::current_dir()?.join("saves"))
                    .with_user(|| self.common.options.player_name()),
            )
        };
        let screen = if has_level && dead && show_death_screen {
            Screen::Dead
        } else {
            if has_level && dead {
                self.respawn_player()?;
            }
            Screen::Playing
        };
        let decision = if heard {
            let none = self.screen_overlay_view(Screen::Playing.id());
            let old = outgoing.as_ref().map(ObservedScreen::view);
            let new = self
                .screen_overlay_view(title.as_ref().map_or(screen.id(), |title| title.mode.id()));
            let answer = self.screen_change(
                old.as_ref().unwrap_or(&none),
                &new,
                title.is_some() || screen != Screen::Playing,
                old.is_some(),
            );
            self.screen_decision_for_context(
                answer,
                true,
                title.is_some() || self.client.title.0.is_some(),
            )
        } else {
            ScreenDecision::Open
        };
        let (screen, owner, target) = match decision {
            ScreenDecision::Keep => return Ok(()),
            ScreenDecision::ModScreen(owner) => (Screen::Mod, Some(owner), None),
            ScreenDecision::Vanilla(target) => (screen, None, Some(target)),
            ScreenDecision::Open => (screen, None, None),
        };

        self.reset_inventory_clicks();
        self.remove_remote_menu_screen();
        if inventory_screen && let Some(remote) = self.client.remote.as_mut() {
            // InventoryMenu.removed clears the result, not the client input grid.
            remote.craft_result = None;
        }
        self.remove_sign_editor();
        self.remove_chat_screen(crate::game::client_side::chat::ChatExit::Interrupted);
        self.close_dialogs();
        self.client.book = None;
        self.client.link_prompt = None;
        self.client.command_prompt.0 = None;
        self.client.code_of_conduct = None;
        self.client.server_packs.0.prompt = None;
        self.client.pause_focus = None;
        if self.client.screen == Screen::Options {
            self.client.chat_ui.options.discard_draw();
        }
        if screen == Screen::Dead && self.client.screen != Screen::Dead {
            self.client.chat_ui.death_time = 0;
            self.client.chat_ui.death_confirm = None;
        }
        if let Some(mut title) = title {
            title.set_rule_registry(Arc::new(
                sunrise_gameplay_common::game::api_values::game_rule_registry(),
            ));
            self.stage_title(title);
            self.client.screen = Screen::Playing;
            self.client.native_mod_screen = None;
        }
        if let Some(target) = target {
            self.open_screen_target(target);
        } else {
            if self.client.title.0.is_some() && screen == Screen::Playing {
                self.release_mappings_on_screen_open();
            }
            self.apply_screen_with_mod_book(screen, owner);
        }
        self.capture(
            self.client.screen == Screen::Playing
                && self.client.title.0.is_none()
                && !self.screen_over_playing(),
        );
        Ok(())
    }
    /// Commit a screen transition synchronously (`Gui.setScreen` / `Screen.init`):
    /// bumps `screen_instance`, clears `last_click` and outgoing mounted widgets,
    /// rebuilds layout, publishes `screen_state.observed`, and mounts the
    /// incoming screen's widgets immediately.
    pub(in crate::game) fn commit_screen_transition(&mut self) {
        self.client.screen_state.0.bump_instance();
        self.clear_screen_widgets();
        self.build_screen();
        let observed = self.observed_now();
        self.client.screen_state.0.observed = observed.clone();
        if !self.common.options.execution_role.is_client() {
            return;
        }
        if let Some(obs) = observed {
            self.init_screen_widgets(&obs.view());
        } else if let Some(obs) = self.headless_screen_snapshot() {
            self.init_screen_widgets(&obs.view());
        }
    }
    /// Like `decide_screen`, for `mod_screen_open`, which clears any active
    /// overlay via `close_replaced_screen_for_mod` before applying the
    /// decision, so `ScreenOpening::ModScreen` and `ScreenOpening::Vanilla`
    /// answers remain eligible even when replacing an overlay.
    pub(in crate::game) fn decide_screen_for_mod_open(
        &self,
        current: &ScreenView<'_>,
        new: &ScreenView<'_>,
        closes: bool,
    ) -> ScreenDecision {
        self.decide_screen(current, new, closes, true)
    }
    /// The open mod screen: a plain panel with its slots' frames, the mod's
    /// drawing over it, then the title and the inventory's label. False when
    /// none is showing.
    pub fn draw_native_screen(&self, batch: &mut HudBatch, ox: f32, oy: f32) -> bool {
        let Some(s) = self.shown_native_screen() else {
            return false;
        };
        let atlas = &self.client.gui.atlas;
        if !s.layout().own_background {
            let (w, h) = (s.layout().width, s.layout().height);
            batch.fill(atlas, ox, oy, w, h, rgba(0, 0, 0, 255));
            batch.fill(
                atlas,
                ox + 1.0,
                oy + 1.0,
                w - 2.0,
                h - 2.0,
                rgba(198, 198, 198, 255),
            );
            for i in s.visible_slots() {
                let (x, y) = self.slot_position(i);
                let (x, y) = (ox + x - 1.0, oy + y - 1.0);
                batch.fill(atlas, x, y, 18.0, 18.0, rgba(55, 55, 55, 255));
                batch.fill(
                    atlas,
                    x + 1.0,
                    y + 1.0,
                    17.0,
                    17.0,
                    rgba(255, 255, 255, 255),
                );
                batch.fill(
                    atlas,
                    x + 1.0,
                    y + 1.0,
                    16.0,
                    16.0,
                    rgba(139, 139, 139, 255),
                );
            }
        }
        if s.screen.has_draw() {
            let frame = s.frame(batch, (ox, oy), self.client.cursor);
            let mut draw = crate::game::host::draw::GameDrawContext::new(self, batch);
            s.screen.draw(self, frame, &mut draw);
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        if s.layout().own_labels {
            return true;
        }
        let grey = rgba(64, 64, 64, 255);
        if let Some(title) = self.remote_menu_title() {
            let [x, y] = s.layout().title_label.unwrap_or([8.0, 6.0]);
            batch.text_styled(atlas, &title, ox + x, oy + y, grey, false);
        }
        if s.player_slots() != 0 {
            let inventory = self.translate_with_fallback("container.inventory", "Inventory");
            let [x, y] = s
                .layout()
                .inventory_label
                .unwrap_or([8.0, s.layout().height - 94.0]);
            batch.text(atlas, &inventory, ox + x, oy + y, grey, false);
        }
        true
    }
    /// The open mod screen's tooltip under the cursor.
    pub(crate) fn draw_native_screen_tooltip(&mut self, batch: &mut HudBatch) {
        // Over no item: an empty slot still gets the mod's hint (which fuel it takes).
        // A carried stack hides only slot tooltips, as AbstractContainerScreen's;
        // the screen's other parts keep theirs, as a vanilla widget's tooltip does.
        let slot = self.slot_at(batch, self.client.cursor.0, self.client.cursor.1);
        if slot.is_some_and(|i| self.slot_view(i).is_some()) {
            return;
        }
        let mut lines = Vec::new();
        self.native_screen_tooltip_lines(batch, slot, &mut lines);
        self.draw_tooltip_lines(batch, &lines, self.client.cursor.0, self.client.cursor.1);
    }
    pub(in crate::game) fn headless_screen_snapshot(&self) -> Option<ObservedScreen> {
        if self.client.renderer.0.is_some() || self.remote_loading() {
            return None;
        }
        if let Some(id) = self.observed_overlay_now() {
            let none = self.screen_change_view(Screen::Playing);
            return Some(self.observe_view(screen_view_with_id(&none, id)));
        }
        if self.client.screen != Screen::Playing {
            return Some(self.observe_view(self.screen_change_view(self.client.screen)));
        }
        if let Some(title) = self.client.title.0.as_ref() {
            let none = self.screen_change_view(Screen::Playing);
            return Some(self.observe_view(screen_view_with_id(&none, title.mode.id())));
        }
        None
    }
    /// Takes the title down as a world opens over it (`setScreen` to the
    /// loading screen, then null): `screen_opening` to no screen, then
    /// `screen_closing` for the title's screen, before it goes. The world
    /// is opening, so a `Cancel` doesn't keep it.
    pub fn hide_title(&mut self) {
        if self.client.title.0.is_none() {
            return;
        }
        if self.screen_hooks_heard() {
            self.title_change(None);
        }
        self.client.title.0 = None;
        self.commit_screen_transition();
    }
    /// A mod's own screen cannot open now: over a book, a sign editor, a
    /// dialog or a prompt, with neither the title nor a player, with a
    /// world loading, or on a guest's behalf.
    pub(crate) fn mod_screen_blocked(&self) -> bool {
        (self.common.player.is_none() && self.client.title.0.is_none())
            || self.screen_over_playing()
            || self.remote_loading()
    }
    /// Deliver a non-repeat key press to the native panel.
    pub(in crate::game) fn native_panel_key_event(
        &mut self,
        key: KeyCode,
        pressed: bool,
        repeat: bool,
    ) {
        if pressed
            && !repeat
            && let Some(name) = keybinds::key_name(key)
        {
            self.native_panel_key(name);
        }
    }
    /// The internal container slot base of the open mod screen (`MOD_MENU_SLOT_BASE`
    /// for a `ModMenuType`, `300` for a stock menu).
    pub(crate) fn native_screen_container_slot_base(&self) -> usize {
        self.shown_native_screen()
            .map_or(300, OpenNativeScreen::container_slot_base)
    }
    /// A data slot of window `window`'s menu (`ContainerSetData`); false
    /// when no mod screen is showing for that active remote window.
    pub fn native_screen_data(&mut self, window: i32, index: usize, value: i32) -> bool {
        if self.shown_active_native_screen().is_none() {
            return false;
        }
        self.client
            .mod_screen
            .0
            .as_mut()
            .is_some_and(|s| s.set_data(window, index, value))
    }
    /// The open mod screen's inventory's left column.
    pub(in crate::game) fn native_screen_inventory_x(&self) -> Option<f32> {
        self.shown_native_screen().map(|s| s.layout().inventory_x)
    }
    /// The first row of the player's inventory on the open mod screen.
    pub(in crate::game) fn native_screen_inventory_y(&self) -> Option<f32> {
        self.shown_native_screen().map(|s| s.layout().inventory_y)
    }
    /// The menu type resolved for this client's opening, including in ScreenOpening.
    pub(crate) fn native_screen_menu_type(
        &self,
        window: i32,
        name: &str,
    ) -> Option<&'static sunrise_level::containers::ModMenuType> {
        let menu = &self.active_native_screen()?.menu;
        (menu.window == window && menu.name == name).then_some(menu.mod_type)?
    }
    /// A press consumed by the shown screen goes to the server as a menu button click.
    pub(crate) fn native_screen_press(&mut self, right: bool) -> bool {
        let Some(r) = self.client.renderer.0.as_ref() else {
            return false;
        };
        let batch = HudBatch::new(
            [r.config.width, r.config.height],
            self.client.settings.gui_scale_inputs(),
        );
        self.native_screen_press_at(&batch, right)
    }
    /// Dispatch a press in the given GUI viewport, with inventory slots taking priority.
    pub fn native_screen_press_at(&mut self, batch: &HudBatch, right: bool) -> bool {
        let Some(screen) = self.shown_native_screen() else {
            return false;
        };
        if self
            .slot_at(batch, self.client.cursor.0, self.client.cursor.1)
            .is_some()
        {
            return false;
        }
        let opening = (screen.menu.window, screen.menu.generation);
        let frame = screen.frame(batch, self.inventory_origin(batch), self.client.cursor);
        let Some(button) = screen.screen.click(self, frame, right) else {
            return false;
        };
        // Numeric container IDs can be reused. Send only for the active opening that heard the press.
        if !self
            .shown_active_native_screen()
            .is_some_and(|screen| (screen.menu.window, screen.menu.generation) == opening)
            || !self.remote_button_click(button)
        {
            return false;
        }
        self.play_audio_button_click();
        true
    }
    /// Where the open mod screen puts container slot `slot` this frame: where
    /// a mod last moved it, else where the screen declared it; None when
    /// inactive/hidden.
    pub fn native_screen_slot(&self, slot: usize) -> Option<(f32, f32)> {
        let s = self.shown_native_screen()?;
        let declared = s
            .layout()
            .slots
            .get(slot)?
            .as_ref()
            .map(|slot| slot.position);
        let [x, y] = s.slot_moves.position(slot, declared)?;
        Some((x, y))
    }
    pub(in crate::game) fn native_screen_slot_icon(&self, slot: usize) -> Option<&str> {
        self.shown_native_screen()?
            .layout()
            .slots
            .get(slot)?
            .as_ref()?
            .no_item_icon
            .as_deref()
    }
    /// The container slots the open mod screen shows this frame.
    pub fn native_screen_slots(&self) -> Option<Vec<usize>> {
        Some(
            self.shown_native_screen()?
                .visible_container_slots()
                .collect(),
        )
    }
    /// The screen-specific line override, shared by filled-slot and empty-space tooltips.
    /// Item images and component gathering remain on the stack's normal drawing path.
    pub fn native_screen_tooltip_lines(
        &self,
        batch: &HudBatch,
        slot: Option<usize>,
        lines: &mut Vec<(sunrise_game_data::text::Text, u32)>,
    ) {
        let Some(screen) = self
            .shown_native_screen()
            .filter(|s| s.screen.has_tooltip())
        else {
            return;
        };
        use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEngine};
        // Keep the original tooltip if the callback or either conversion fails.
        // An item's text can fail on every frame, so do not log conversion refusals here.
        let Ok(mut snapshot) = lines.try_into_api() else {
            return;
        };
        let menu_slot = slot.and_then(|slot| {
            Self::menu_slot_index(&screen.menu.name, slot).map(|slot| slot as u32)
        });
        let frame = screen.frame(batch, self.inventory_origin(batch), self.client.cursor);
        if screen.screen.tooltip(self, frame, menu_slot, &mut snapshot)
            && let Ok(replacement) = snapshot.try_into_engine(&())
        {
            *lines = replacement;
        }
    }
    /// The open mod screen's width.
    pub(in crate::game) fn native_screen_width(&self) -> Option<f32> {
        self.shown_native_screen().map(|s| s.layout().width)
    }
    /// What the frame's screen changed since the last one. The `Screen` list,
    /// the overlays' opening and the title's screens announce their own
    /// changes before they apply (`set_screen`, `open_screen_overlay`,
    /// `settle_title_mode`, `show_title`, `hide_title`); what is left, an
    /// overlay closing, is announced here, through the same `screen_change`
    /// and in the same order, but after the change. `screen_init_post`
    /// follows on opening and when a resize rebuilds the receiver's children.
    pub fn observe_change(&mut self, current: Option<ObservedScreen>) {
        let changed = match (&self.client.screen_state.0.observed, &current) {
            (Some(old), Some(new)) => {
                old.id != new.id
                    || old.menu != new.menu
                    || old.menu_generation != new.menu_generation
                    || old.mod_screen != new.mod_screen
                    || old.dialog_key != new.dialog_key
            }
            (None, None) => false,
            _ => true,
        };
        if changed {
            let old = self
                .client
                .screen_state
                .0
                .observed
                .as_ref()
                .filter(|old| old.overlay);
            let opens = current.as_ref().is_some_and(|new| new.overlay)
                || (current.is_none() && old.is_some());
            if old.is_some() || opens {
                let none = self.screen_change_view(Screen::Playing);
                let from = old.map(ObservedScreen::view);
                let to = current.as_ref().map(ObservedScreen::view);
                self.screen_change(
                    from.as_ref().unwrap_or(&none),
                    to.as_ref().unwrap_or(&none),
                    opens,
                    from.is_some(),
                );
            }
            self.client.screen_state.0.bump_instance();
            self.clear_screen_widgets();
            self.client.screen_state.0.observed = current.clone();
            if let Some(new) = &current
                && self.common.options.execution_role.is_client()
            {
                self.init_screen_widgets(&new.view());
            }
            return;
        }
        if let (Some(old), Some(new)) = (&self.client.screen_state.0.observed, &current)
            && old.screen_size != new.screen_size
            // Screen.resize calls repositionElements. Screen's default rebuilds
            // widgets; these receivers override it to relayout existing children.
            // A panel rect can also move independently (e.g. opening a recipe book).
            && !matches!(
                new.id,
                "options"
                    | "world_options"
                    | "stats"
                    | "advancements"
                    | "social"
                    | "debug_options"
                    | "dialog"
                    | "dialog_warning"
                    | "confirm_link"
                    | "demo_intro"
                    | "link_prompt"
                    | "code_of_conduct"
                    | "world_select"
                    | "world_create"
                    | "world_create_flat"
                    | "world_create_buffet"
                    | "world_edit"
                    | "world_recover"
                    | "multiplayer"
                    | "multiplayer_warning"
                    | "disconnected"
                    | "onboarding"
            )
        {
            self.clear_screen_widgets();
            self.init_screen_widgets(&new.view());
        }
        self.client.screen_state.0.observed = current;
    }
    pub fn observe_screen(&mut self) {
        self.settle_title_mode();
        self.refresh_screen_widgets();
    }
    pub(in crate::game) fn observe_view(&self, view: ScreenView<'_>) -> ObservedScreen {
        let screen_size = if self.client.renderer.0.is_some() {
            self.gui_frame_now().1.size.map(|size| size as f32)
        } else {
            [view.rect[2], view.rect[3]]
        };
        ObservedScreen {
            id: view.id,
            overlay: self.client.screen == Screen::Playing || self.observed_overlay_now().is_some(),
            mod_screen: view.mod_screen.map(str::to_owned),
            menu: view.menu.map(str::to_owned),
            menu_generation: (self.client.screen == Screen::Inventory
                && !self.screen_over_playing())
            .then(|| self.client.remote.as_ref().map(|r| r.menu.generation))
            .flatten(),
            dialog_key: (view.id == "dialog").then(|| match self.dialog_screen_key() {
                (true, _, _) => (true, false, None),
                key => key,
            }),
            title: view
                .title
                .and_then(|_| self.client.remote.as_ref()?.shared_menu_title_snapshot()),
            rect: view.rect,
            screen_size,
            slots: view.slots,
            mouse: view.mouse,
            partial_tick: view.partial_tick,
            regions: Vec::new(),
        }
    }
    /// Snapshot of the receiving screen for screen input hooks, including overlays.
    pub(in crate::game) fn observed_input_screen(&self) -> Option<ObservedScreen> {
        let view = self.observed_screen_view()?;
        let regions = view.regions.to_vec();
        let mut snapshot = self.observe_view(view);
        snapshot.regions = regions;
        Some(snapshot)
    }
    /// What `observe_screen` compares with the last frame's.
    pub(in crate::game) fn observed_now(&self) -> Option<ObservedScreen> {
        // Only the title's screens and the overlays show over `Screen::Playing`.
        self.observed_screen_view()
            .map(|view| self.observe_view(view))
    }
    pub(crate) fn observed_overlay_now(&self) -> Option<&'static str> {
        let book = self.client.book.as_ref().map(super::books::BookScreen::id);
        observed_overlay_id(&[
            (self.client.command_prompt.0.is_some(), "command_prompt"),
            (self.client.link_prompt.is_some(), "link_prompt"),
            (self.client.code_of_conduct.is_some(), "code_of_conduct"),
            (
                self.client.server_packs.0.prompt.is_some(),
                "server_pack_prompt",
            ),
            (self.dialog_open(), "dialog"),
            (book.is_some(), book.unwrap_or("book_view")),
            (self.client.sign_edit.is_some(), "sign_editor"),
        ])
    }
    pub(in crate::game) fn observed_screen_view(&self) -> Option<ScreenView<'_>> {
        let Some(id) = self.observed_overlay_now() else {
            return self.screen_view_now();
        };
        self.renderer_framebuffer()?;
        Some(self.screen_overlay_view(id))
    }
    /// Opens an overlay, one of the screens kept beside `Screen` (the sign
    /// editor, a book or lectern, a dialog, the command, link,
    /// code-of-conduct and server-pack prompts), as `Gui.setScreen` opens
    /// a screen: `screen_opening` fires first with `id` as the new screen,
    /// then `screen_closing` for an overlay it replaces. A mod's `Cancel`
    /// keeps what is open and `open` never runs; `ModScreen` opens the
    /// mod's own screen instead, unless one can't open now, and `Vanilla` its
    /// vanilla screen. Otherwise `open`
    /// sets the overlay and `screen_init_post` follows. Returns whether
    /// `open` ran. With no mod loaded, on a server's `Game` and on a guest's
    /// behalf, `open` just runs.
    pub fn open_screen_overlay(&mut self, id: &'static str, open: impl FnOnce(&mut Self)) -> bool {
        if !self.screen_hooks_heard() {
            open(self);
            self.release_mappings_on_screen_open();
            self.client.screen_state.0.build.reopened();
            self.commit_screen_transition();
            return true;
        }
        // What changed earlier this frame is told first, so `screen_state`
        // is what shows now.
        self.observe_screen();
        let decision = {
            let none = self.screen_change_view(Screen::Playing);
            let from = self
                .client
                .screen_state
                .0
                .observed
                .as_ref()
                .filter(|old| old.overlay);
            let from = from.map(ObservedScreen::view);
            let to = screen_view_with_id(&none, id);
            self.decide_screen(from.as_ref().unwrap_or(&none), &to, from.is_some(), false)
        };
        match decision {
            ScreenDecision::Keep => return false,
            ScreenDecision::ModScreen(mod_id) => {
                self.apply_screen(Screen::Mod, Some(mod_id));
                return false;
            }
            ScreenDecision::Vanilla(target) => {
                self.open_screen_target(target);
                return false;
            }
            ScreenDecision::Open => {}
        }
        open(self);
        self.release_mappings_on_screen_open();
        self.client.screen_state.0.build.reopened();
        self.commit_screen_transition();
        true
    }
    /// Opens vanilla screen `target` as it opens on its own, in place of the
    /// one asked for (`ScreenOpening::Vanilla`, which `screen_decision` let
    /// through): the change fires no screen hook again, and commits once.
    pub(in crate::game) fn open_screen_target(&mut self, target: ScreenTarget) {
        let before_instance = self.client.screen_state.0.screen_instance;
        self.client.screen_state.0.replacing = true;
        self.client.screen_state.0.build.reopened();
        match target {
            ScreenTarget::Pause => {
                self.client.pause_focus = None;
                self.client.pause_menu = true;
                self.set_screen(Screen::Paused);
            }
            ScreenTarget::Options => self.open_options(false),
            ScreenTarget::Inventory => {
                // A menu the server opened for the screen asked for (a crafting
                // table) closes: the inventory shows the player's own. With
                // only the player's own open, there is nothing to close.
                if self.remote_menu_type().is_some() {
                    self.close_container_client();
                }
                self.open_player_inventory();
            }
            ScreenTarget::Advancements => self.open_advancements(false),
            ScreenTarget::Stats => self.open_stats(),
            ScreenTarget::Chat => {
                self.begin_chat_screen("", false);
            }
            _ => {
                // A title screen: set the title mode first so `set_screen(Playing)`
                // commits directly into the target title screen without double-init.
                if let (Some(title), Some(id)) =
                    (self.client.title.0.as_mut(), title_target_id(target))
                {
                    title.open_by_id(id);
                }
                self.release_mappings_on_screen_open();
                self.set_screen(Screen::Playing);
            }
        }
        self.build_screen();
        // Every target takes the pointer, even one opened in place of a
        // request that grabbed none (a mod's screen asked for while playing).
        self.capture(false);
        self.client.screen_state.0.replacing = false;
        if self.client.screen_state.0.screen_instance == before_instance {
            self.commit_screen_transition();
        }
    }
    /// Refresh the receiver's build before another input can reach it.
    pub(in crate::game) fn refresh_screen_widgets(&mut self) {
        if self.client.screen_state.0.replacing {
            return;
        }
        // Keeps the build on the frame's window size and drops it once nothing
        // shows; an unchanged frame compares and builds nothing.
        self.build_screen();
        if self.has_context_menu() {
            let _ = self.live_context_menu();
        }
        // Free with no native mod: the snapshot clones the title and menu.
        if mod_api::__private::mods().is_empty() {
            return;
        }
        let current = self.observed_now();
        self.observe_change(current);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_refresh_screen_widgets(&mut self) {
        self.refresh_screen_widgets();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_context_menu_open(&mut self) -> bool {
        self.live_context_menu()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_has_context_menu(&self) -> bool {
        self.has_context_menu()
    }
    /// `screen` as the screen hooks see it while it opens or closes: no
    /// renderer (a test, a window not yet up) leaves its rectangle empty.
    pub(crate) fn screen_change_view(&self, screen: Screen) -> ScreenView<'_> {
        let rect = match &self.client.renderer.0 {
            Some(renderer) => self.screen_rect(
                &HudBatch::new(
                    [renderer.config.width, renderer.config.height],
                    self.client.settings.gui_scale_inputs(),
                ),
                screen,
            ),
            None => [0.0; 4],
        };
        let mut view = self.screen_view_at(screen, rect);
        if screen == Screen::Playing {
            view.slots = 0;
        }
        view
    }
    /// `native_screen_char_typed_pre` for one character on `receiver`, with
    /// the focused box's edit: what the listeners leave of `field` goes back
    /// to the box before the character does. Whether they cancelled the character.
    pub(crate) fn screen_char_canceled_on(
        &mut self,
        receiver: Option<&ObservedScreen>,
        c: char,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ScreenCharTypedPre) {
            return false;
        }
        let Some(receiver) = receiver else {
            return false;
        };
        let modifiers = glfw_modifiers(self.client.modifiers);
        self.with_focused_text_edit(|game, field| {
            let screen = receiver.view();
            let mut canceled = false;
            game.native_screen_char_typed_pre(&screen, c, modifiers, field, &mut canceled);
            canceled
        })
    }
    /// Deliver each printable character of `text` through `CharacterTyped.Pre`,
    /// mounted screen widgets first, then `take` (the vanilla text box sink,
    /// returning whether it handled the character), and finally
    /// `CharacterTyped.Post` on the captured receiving screen ONLY when
    /// unhandled. Snapshots the receiving screen per character (matching
    /// vanilla `KeyboardHandler.charTyped` reading `minecraft.screen` each call)
    /// and never falls through into a replacement screen opened mid-character.
    pub(in crate::game) fn screen_chars_dispatch(
        &mut self,
        text: &str,
        mut take: impl FnMut(&mut Self, &str) -> bool,
    ) {
        let modifiers = glfw_modifiers(self.client.modifiers);
        let mut encoded = [0; 4];
        for c in text.chars() {
            if c.is_control() {
                continue;
            }
            if self.observed_screen_view().is_none() && self.headless_screen_snapshot().is_none() {
                break;
            }
            let receiver = self.observed_input_screen();
            let initial_instance = self.client.screen_state.0.screen_instance;
            if self.screen_char_canceled_on(receiver.as_ref(), c) {
                continue;
            }
            let mut handled = self.client.screen_state.0.screen_instance == initial_instance
                && self.mounted_widget_char_typed(c, modifiers);
            if !handled && self.client.screen_state.0.screen_instance == initial_instance {
                handled = take(self, c.encode_utf8(&mut encoded));
            }
            if !handled {
                self.screen_chars_post_on(receiver.as_ref(), c);
            }
        }
    }
    /// `ScreenEvent.CharacterTyped.Post` for a character that neither mounted
    /// widgets nor any text box took, on the captured receiving screen.
    pub(crate) fn screen_chars_post_on(&self, receiver: Option<&ObservedScreen>, c: char) {
        if !mod_api::__private::listened(mod_api::HookId::ScreenCharTypedPost) {
            return;
        }
        let Some(receiver) = receiver else {
            return;
        };
        let modifiers = glfw_modifiers(self.client.modifiers);
        self.native_screen_char_typed_post(&receiver.view(), c, modifiers);
    }
    /// A key press's characters (`ScreenEvent.CharacterTyped.Pre`, vanilla's
    /// `KeyboardHandler.charTyped`), dispatched once the screen has acted on
    /// the key: GLFW's char callback comes after its key callback, so the
    /// characters go to the screen that is open now — a key that closed the
    /// screen leaves none to hear them, as vanilla's closed screen hears
    /// nothing, and one that moved the focus types into the box it moved to.
    /// Each printable one snapshots the current screen, runs
    /// `native_screen_char_typed_pre` (when hooks apply), then mounted widgets,
    /// then the focused text box, and unhandled characters fire
    /// `native_screen_char_typed_post` on that character's captured receiver.
    pub fn screen_chars_typed(&mut self, text: Option<&str>) {
        let Some(text) = text else {
            return;
        };
        if self.observed_screen_view().is_none() && self.headless_screen_snapshot().is_none() {
            return;
        }
        // A held edit modifier makes a shortcut of the key, not text.
        let text = chars_to_type(text, self.types_text());
        if text.is_empty() {
            return;
        }
        self.screen_chars_dispatch(&text, |game, c| game.type_chars_into_screen(c).is_ok());
    }
    /// What a mod's `screen_opening` answer does here: a `ModScreen` only
    /// where a mod's screen can open (`mod_screen_blocked`), a `Vanilla`
    /// target only where it opens (a title screen while the title shows, a
    /// world's screen where a mod's could open in a world), else the change
    /// goes ahead and the refused target is logged. Records whether a mod's
    /// answer changed what shows (`ScreenState::answered`).
    pub(in crate::game) fn screen_decision(
        &self,
        answer: Option<Answered<ScreenOpening>>,
        clears_overlays: bool,
    ) -> ScreenDecision {
        self.screen_decision_for_context(answer, clears_overlays, self.client.title.0.is_some())
    }
    pub(in crate::game) fn screen_decision_for_context(
        &self,
        answer: Option<Answered<ScreenOpening>>,
        clears_overlays: bool,
        title_available: bool,
    ) -> ScreenDecision {
        let blocked = if clears_overlays {
            (self.common.player.is_none() && !title_available) || self.remote_loading()
        } else {
            self.mod_screen_blocked()
        };
        let decision = match answer {
            Some(Answered {
                answer: ScreenOpening::Cancel,
                ..
            }) => ScreenDecision::Keep,
            Some(Answered {
                answer: ScreenOpening::ModScreen,
                mod_id,
                ..
            }) if !blocked => ScreenDecision::ModScreen(mod_id),
            Some(Answered {
                answer: ScreenOpening::Vanilla(target),
                mod_id,
                ..
            }) => {
                let opens = match title_target_id(target) {
                    // An overlay over the title would stay in front of it unless cleared.
                    Some(_) => title_available && (clears_overlays || !self.screen_over_playing()),
                    None => !title_available && !blocked,
                };
                if opens {
                    ScreenDecision::Vanilla(target)
                } else {
                    log::warn!(
                        "Sunrise: mod {mod_id} answered screen_opening with {target:?}, which can't open here; the screen asked for opens"
                    );
                    ScreenDecision::Open
                }
            }
            _ => ScreenDecision::Open,
        };
        self.client
            .screen_state
            .0
            .answered
            .set(!matches!(decision, ScreenDecision::Open));
        decision
    }
    /// Whether a change of screen asks the screen hooks: a mod listens to
    /// `screen_opening` or `screen_closing`, on a client's own `Game` (not a
    /// server's, not on a guest's behalf, not the screen a mod's `Vanilla`
    /// answer opens). Otherwise it is a plain assignment.
    pub(in crate::game) fn screen_hooks_heard(&self) -> bool {
        (mod_api::__private::listened(mod_api::HookId::ScreenOpening)
            || mod_api::__private::listened(mod_api::HookId::ScreenClosing))
            && self.common.options.execution_role.is_client()
            && !self.client.screen_state.0.replacing
    }
    /// A screen the native mods' input hooks are for is open: the window has a
    /// renderer, either the title's own (`title`, `world_select`,
    /// `world_create`, `multiplayer`…) or a screen over the world
    /// (`Screen::Playing` behind it, so not a book, a sign editor, a server
    /// dialog or a prompt drawn over gameplay), and no connect or world load
    /// is in progress. The one gate they all ask before they fire.
    pub fn screen_hooks_open(&self) -> bool {
        self.has_renderer()
            && !self.screen_over_playing()
            && !self.remote_loading()
            && (self.client.screen != Screen::Playing || self.client.title.0.is_some())
    }
    /// Whether a Post of the key `key` over the receiving screen `receiver`
    /// cancelled it (`ScreenEvent.KeyPressed.Post` / `KeyReleased.Post`): asked
    /// where the key neither mounted widgets nor screen arms took reaches the
    /// game's own key handling, which a cancel then skips. False with no
    /// subscriber, no receiving screen, or a key with no vanilla name.
    pub(crate) fn screen_key_post_on(
        &self,
        receiver: Option<&ObservedScreen>,
        key: KeyCode,
        pressed: bool,
        repeat: bool,
    ) -> bool {
        let hook = if pressed {
            mod_api::HookId::ScreenKeyPressedPost
        } else {
            mod_api::HookId::ScreenKeyReleasedPost
        };
        if !mod_api::__private::listened(hook) {
            return false;
        }
        let (Some(name), Some(receiver)) = (keybinds::key_name(key), receiver) else {
            return false;
        };
        let modifiers = glfw_modifiers(self.client.modifiers);
        let mut canceled = false;
        let screen = receiver.view();
        if pressed {
            self.native_screen_key_pressed_post(&screen, name, repeat, modifiers, &mut canceled);
        } else {
            self.native_screen_key_released_post(&screen, name, modifiers, &mut canceled);
        }
        canceled
    }
    /// Whether the key press `key` over the receiving screen went no further
    /// (`native_screen_key_pressed_pre`): what `handle_window_event` and the
    /// control socket's `key` command both ask before a screen acts on a key.
    /// False when no receiving screen is open, and for a key with no vanilla name.
    pub(crate) fn screen_key_pressed_on(
        &self,
        receiver: Option<&ObservedScreen>,
        key: KeyCode,
        repeat: bool,
    ) -> bool {
        let (Some(name), Some(receiver)) = (keybinds::key_name(key), receiver) else {
            return false;
        };
        let mut canceled = false;
        self.native_screen_key_pressed_pre(
            &receiver.view(),
            name,
            repeat,
            glfw_modifiers(self.client.modifiers),
            &mut canceled,
        );
        canceled
    }
    pub(crate) fn screen_key_released_on(
        &self,
        receiver: Option<&ObservedScreen>,
        key: KeyCode,
    ) -> bool {
        let (Some(name), Some(receiver)) = (keybinds::key_name(key), receiver) else {
            return false;
        };
        let mut canceled = false;
        self.native_screen_key_released_pre(
            &receiver.view(),
            name,
            glfw_modifiers(self.client.modifiers),
            &mut canceled,
        );
        canceled
    }
    pub(in crate::game) fn screen_mouse_dragged(&self, delta: [f32; 2]) -> ScreenMousePre {
        if self.pad_synthetic_input() || delta == [0.0; 2] {
            return ScreenMousePre::NotFired;
        }
        let Some(button) = self
            .client
            .drag_button
            .filter(|b| self.client.physical_buttons.contains(b))
        else {
            return ScreenMousePre::NotFired;
        };
        let Some(receiver) = self.observed_input_screen() else {
            return ScreenMousePre::NotFired;
        };
        let button = glfw_button(button);
        let modifiers = glfw_modifiers(self.client.modifiers);
        let mut canceled = false;
        self.native_screen_mouse_dragged_pre(
            &receiver.view(),
            button,
            delta,
            modifiers,
            &mut canceled,
        );
        if canceled {
            ScreenMousePre::Canceled
        } else {
            ScreenMousePre::Passed(ScreenMousePost::new(
                receiver,
                modifiers,
                ScreenMousePostKind::Drag { button, delta },
            ))
        }
    }
    pub(crate) fn screen_overlay_view(&self, id: &'static str) -> ScreenView<'_> {
        let rect = self.renderer_framebuffer().map_or([0.0; 4], |framebuffer| {
            let batch = HudBatch::new(framebuffer, self.client.settings.gui_scale_inputs());
            [0.0, 0.0, batch.width, batch.height]
        });
        ScreenView::from(mod_api::__private::built::ScreenView {
            id,
            mod_screen: None,
            menu: None,
            title: None,
            rect,
            slots: 0,
            mouse: self.cursor(),
            partial_tick: self.common.clock.fraction() as f32,
            regions: &[],
        })
    }
    /// The open screen's container panel in GUI pixels (`screen`'s, the one
    /// this frame draws), or the whole window for a screen with none
    /// (`ScreenView.rect`).
    pub(in crate::game) fn screen_rect(&self, batch: &HudBatch, screen: Screen) -> [f32; 4] {
        if screen != Screen::Inventory && screen != Screen::Creative {
            return [0.0, 0.0, batch.width, batch.height];
        }
        // The window the game itself draws (`container_image`): the merchant
        // and the beacon are wider and shifted, creative is its own size.
        let image = self.container_image(batch);
        [
            image.left as f32,
            image.top as f32,
            image.width as f32,
            image.height as f32,
        ]
    }
    /// Text into a text box a character at a time (`Screen.charTyped`): each
    /// printable one snapshots the current receiving screen, is offered to
    /// `native_screen_char_typed_pre`, and `take` gets it unless cancelled,
    /// before the next is offered. A control character goes straight to `take`.
    pub(crate) fn screen_text_each(&mut self, text: &str, mut take: impl FnMut(&mut Self, &str)) {
        let mut encoded = [0; 4];
        for c in text.chars() {
            if c.is_control() {
                take(self, c.encode_utf8(&mut encoded));
                continue;
            }
            let receiver = self.observed_input_screen();
            let initial_instance = self.client.screen_state.0.screen_instance;
            if !self.screen_char_canceled_on(receiver.as_ref(), c)
                && self.client.screen_state.0.screen_instance == initial_instance
            {
                take(self, c.encode_utf8(&mut encoded));
            }
        }
    }
    /// The open screen as an input hook sees it (`ScreenView`), or None
    /// when `screen_hooks_open` says no screen is. `screen.mouse` is the
    /// pointer the hook's event is at, so a hook takes no position, and
    /// `partial_tick` is the clock's fraction at this event, not a frame's.
    /// The `HudBatch` built here is only what `screen.rect` needs: the
    /// window's GUI scale, no allocation.
    pub(crate) fn screen_view_now(&self) -> Option<ScreenView<'_>> {
        if !self.screen_hooks_open() {
            return None;
        }
        let framebuffer = self.renderer_framebuffer()?;
        let batch = HudBatch::new(framebuffer, self.client.settings.gui_scale_inputs());
        if self.client.screen != Screen::Playing {
            return Some(self.screen_view(&batch, self.client.screen));
        }
        self.title_screen_view(&batch)
    }
    /// `set_screen`, with `before` run once the hooks let the change go ahead and
    /// before it applies: what a screen's own `onClose` does only when it really
    /// closes (a container screen's close packet).
    pub(in crate::game) fn set_screen_with(
        &mut self,
        screen: Screen,
        before: impl FnOnce(&mut Self),
    ) {
        self.transition_screen(screen, false, |game, _| before(game));
    }

    /// Dismisses the current screen without a vetoable opening.
    pub(in crate::game) fn dismiss_screen_with(&mut self, before: impl FnOnce(&mut Self)) {
        if self.client.screen == Screen::Playing {
            before(self);
            return;
        }
        if self.screen_hooks_heard() {
            let current = self.screen_change_view(self.client.screen);
            self.native_screen_closing(&current);
        }
        before(self);
        if self.client.screen == Screen::Options {
            self.client.chat_ui.options.discard_draw();
        }
        self.apply_screen_with_mod_book(Screen::Playing, None);
        self.client.screen_state.0.observed = self.observed_now();
    }

    /// Runs preparation after admission, passing the accepted screen or None for a redirect.
    pub(in crate::game) fn transition_screen(
        &mut self,
        screen: Screen,
        new_instance: bool,
        before: impl FnOnce(&mut Self, Option<Screen>),
    ) {
        let opening_native =
            screen == Screen::Inventory && self.active_native_screen().is_some_and(|s| !s.shown);
        let new_menu_gen = (screen == Screen::Inventory)
            .then(|| self.client.remote.as_ref().map(|r| r.menu.generation))
            .flatten();
        let menu_replaced = screen == Screen::Inventory
            && new_menu_gen.is_some()
            && self
                .client
                .screen_state
                .0
                .observed
                .as_ref()
                .and_then(|o| o.menu_generation)
                != new_menu_gen;
        if self.client.screen == screen && !new_instance {
            if opening_native || menu_replaced {
                self.apply_screen_with_mod_book(screen, None);
            }
            return;
        }
        let mut screen = screen;
        let mut owner = None;
        if self.screen_hooks_heard() {
            let decision = {
                let observed = new_instance.then(|| self.observed_screen_view()).flatten();
                let current =
                    observed.unwrap_or_else(|| self.screen_change_view(self.client.screen));
                let new = self.screen_change_view(screen);
                self.decide_screen(&current, &new, current.id != "playing", new_instance)
            };
            match decision {
                ScreenDecision::Keep => {
                    // Reject only the opening just staged, never the menu
                    // that an unrelated veto is keeping on screen.
                    if opening_native {
                        self.remote_close_container();
                    }
                    return;
                }
                ScreenDecision::ModScreen(mod_id) => {
                    screen = Screen::Mod;
                    owner = Some(mod_id);
                }
                ScreenDecision::Vanilla(target) => {
                    if new_instance {
                        self.remove_replaced_overlays();
                    }
                    before(self, None);
                    if opening_native && self.active_native_screen().is_some() {
                        self.remote_close_container();
                    }
                    self.open_screen_target(target);
                    return;
                }
                ScreenDecision::Open => {}
            }
        }
        if self.client.screen == screen && owner.is_none() && !new_instance {
            return;
        }
        if new_instance {
            self.remove_replaced_overlays();
            self.client.screen_state.0.build.reopened();
        }
        before(self, Some(screen));
        if self.client.screen == Screen::Options {
            self.client.chat_ui.options.discard_draw();
        }
        self.apply_screen_with_mod_book(screen, owner);
    }

    /// Clears accepted replacements without invoking the overlays' user actions.
    fn remove_replaced_overlays(&mut self) {
        self.client.command_prompt.0 = None;
        self.client.link_prompt = None;
        self.client.code_of_conduct = None;
        self.client.server_packs.0.prompt = None;
        self.close_dialogs();
        self.client.book = None;
        self.remove_sign_editor();
    }

    pub fn settle_title_mode(&mut self) {
        let heard = self.screen_hooks_heard();
        let gated = heard
            || (self.common.options.execution_role.is_client()
                && !self.client.screen_state.0.replacing
                && mod_api::__private::listened(mod_api::HookId::ScreenInitPost));
        let Some(title) = self.client.title.0.as_mut() else {
            return;
        };
        title.gated = gated;
        let Some(mode) = title.take_requested() else {
            return;
        };
        let (current, id) = (title.mode.id(), mode.id());
        // What changed before is told first.
        let now = self.observed_now();
        self.observe_change(now);
        let decision = if heard {
            let none = self.screen_change_view(Screen::Playing);
            let from = screen_view_with_id(&none, current);
            let to = screen_view_with_id(&none, id);
            self.decide_screen(&from, &to, true, false)
        } else {
            ScreenDecision::Open
        };
        match decision {
            ScreenDecision::Keep => (),
            ScreenDecision::ModScreen(mod_id) => self.apply_screen(Screen::Mod, Some(mod_id)),
            ScreenDecision::Vanilla(target) => self.open_screen_target(target),
            ScreenDecision::Open => {
                if let Some(title) = self.client.title.0.as_mut() {
                    title.activate_mode(mode);
                }
                self.release_mappings_on_screen_open();
                self.commit_screen_transition();
            }
        }
    }
    pub(crate) fn settle_title_toasts(&mut self) {
        use super::toast_manager::SystemToastId;
        let pending = self
            .client
            .title
            .0
            .as_mut()
            .and_then(|title| {
                title.poll_backup();
                title.take_pending_toast()
            })
            .or_else(|| {
                // The launcher's "Playing offline" can sit behind this window: say it here too, once
                // the toast slot is free.
                if self.client.title.0.is_none()
                    || self
                        .client
                        .toast_manager
                        .system_mut(SystemToastId::Title)
                        .is_some()
                {
                    return None;
                }
                let reason = self.common.options.offline_reason.take()?;
                Some(
                    sunrise_client::toasts::SystemToast::new(
                        sunrise_game_data::text::Text::tr("sunrise.title.playing_offline", &[]),
                        Some(sunrise_game_data::text::Text::literal(reason)),
                    )
                    .with_display_time(sunrise_client::toasts::SystemToast::LONG_DISPLAY_TIME),
                )
            });
        if let Some(toast) = pending {
            self.add_system_toast(Some(SystemToastId::Title), toast);
        }
    }
    /// The shown native screen whose remote window is still the active logical menu.
    pub(in crate::game) fn shown_active_native_screen(&self) -> Option<&OpenNativeScreen> {
        self.active_native_screen().filter(|s| {
            s.shown
                && self.client.screen == Screen::Inventory
                && self.common.container == Some(s.menu.key)
        })
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_observed_title(&self) -> Option<std::sync::Arc<mod_api::Text>> {
        self.observe_view(self.screen_change_view(self.client.screen))
            .title
    }
    /// The hooks of the title appearing (`to` is its screen) or going
    /// (`None`), from what shows now, and a mod's answer. What changed before
    /// is told first.
    pub(in crate::game) fn title_change(
        &mut self,
        to: Option<&'static str>,
    ) -> Option<Answered<ScreenOpening>> {
        let now = self.observed_now();
        self.observe_change(now);
        let none = self.screen_change_view(Screen::Playing);
        // The title's screen or an overlay; a `Screen` told its own change.
        let from = self
            .client
            .screen_state
            .0
            .observed
            .as_ref()
            .filter(|old| old.overlay);
        let from = match (from, self.client.title.0.as_ref()) {
            (Some(old), _) => Some(old.view()),
            // No renderer yet (startup, a test): the title is still known.
            (None, Some(title)) => Some(screen_view_with_id(&none, title.mode.id())),
            (None, None) => None,
        };
        let to = to.map(|id| screen_view_with_id(&none, id));
        self.screen_change(
            from.as_ref().unwrap_or(&none),
            to.as_ref().unwrap_or(&none),
            true,
            from.is_some(),
        )
    }
    /// The title's own screen as a native mod sees it (`ScreenView`):
    /// its id from the title state machine (`title`, `world_select`,
    /// `world_create`, `multiplayer`, `add_server`…), no menu and no menu
    /// title, the whole window as its rectangle and no slots (no menu is
    /// open). None while no title screen is up.
    pub fn title_screen_view(&self, batch: &HudBatch) -> Option<ScreenView<'_>> {
        let title = self.client.title.0.as_ref()?;
        Some(ScreenView::from(mod_api::__private::built::ScreenView {
            id: title.mode.id(),
            mod_screen: None,
            menu: None,
            title: None,
            rect: [0.0, 0.0, batch.width, batch.height],
            slots: 0,
            mouse: self.cursor(),
            partial_tick: self.common.clock.fraction() as f32,
            regions: &[],
        }))
    }
    pub(crate) fn track_drag_button(&mut self, button: MouseButton, pressed: bool) {
        // Any release ends the drag, as vanilla's `MouseHandler.onButton` clears `activeButton`:
        // left down, right down, left up sends no drag while right is still held.
        self.client.drag_button = pressed.then_some(button);
    }
    /// Opens `screen` (`Gui.setScreen`; `Screen::Playing` is vanilla's null).
    pub fn set_screen(&mut self, screen: Screen) {
        self.set_screen_with(screen, |_| {});
    }
    /// Runs `screen_change` from `current` to `new`, then resolves the hooks' decision.
    pub(in crate::game) fn decide_screen(
        &self,
        current: &ScreenView<'_>,
        new: &ScreenView<'_>,
        closes: bool,
        removes_overlays: bool,
    ) -> ScreenDecision {
        self.screen_decision(
            self.screen_change(current, new, true, closes),
            removes_overlays,
        )
    }
    /// Fires `screen_opening` and `screen_closing` in order for a screen transition.
    pub(in crate::game) fn screen_change(
        &self,
        current: &ScreenView<'_>,
        new: &ScreenView<'_>,
        opens: bool,
        closes: bool,
    ) -> Option<Answered<ScreenOpening>> {
        // Asked before either screen draws: nothing beside it yet.
        let current = &crate::game::screen_backgrounds::without_regions(current);
        let new = &crate::game::screen_backgrounds::without_regions(new);
        let mut outcome = None;
        if opens {
            self.native_screen_opening(current, new, &mut outcome);
        }
        if closes
            && outcome
                .as_ref()
                .is_none_or(|a| a.answer != ScreenOpening::Cancel)
        {
            self.native_screen_closing(current);
        }
        outcome
    }
    /// Shows `title` (`Minecraft.setScreen(new TitleScreen())`) with the game-rule registry attached.
    pub fn show_title(&mut self, mut title: sunrise_client::title::Title) {
        title.set_rule_registry(Arc::new(
            sunrise_gameplay_common::game::api_values::game_rule_registry(),
        ));
        self.show_title_with_registry(title);
    }
    /// Admission for ordinary toasts; mandatory callback-panic diagnostics bypass mod admission.
    pub(in crate::game) fn admit_toast(
        &self,
        kind: mod_api::ToastKind,
        title: &mut Text,
        description: &mut Option<Text>,
    ) -> bool {
        // Only the receiving client owns toast presentation and its hook.
        if self.common.is_logical_server() {
            return false;
        }
        let mut cancel = false;
        self.native_toast_add(kind, title, description, &mut cancel);
        !cancel
    }
    pub(in crate::game) fn tick_rate(&self) -> Option<mod_api::ClientTickRate> {
        self.native_client_level()
            .is_some()
            .then(|| mod_api::ClientTickRate {
                tick_rate: self.common.clock.rate(),
                frozen: self.common.clock.frozen(),
                frozen_ticks_to_run: self.common.clock.step_state().remaining,
            })
    }

    /// Client tick target duration in ms (`Minecraft.getTickTargetMillis`), after mod hooks.
    pub fn tick_target_millis(&self) -> f32 {
        let vanilla = self.common.clock.tick_target_millis();
        let mut millis = vanilla;
        self.native_tick_target_millis(&mut millis);
        if millis.is_finite() && millis > 0.0 {
            millis
        } else {
            vanilla
        }
    }
    /// Local player's heart kind (`Hud.HeartType.forPlayer`, then `player_heart_type`).
    pub fn player_heart_kind(&mut self) -> Option<mod_api::client::hud_layout::HeartKind> {
        use mod_api::client::hud_layout::HeartKind;
        let p = self.common.player.as_ref()?;
        let effects = &self.common.potions.effects;
        let mut kind = if effects.get("poison").is_some() {
            HeartKind::Poisoned
        } else if effects.get("wither").is_some() {
            HeartKind::Withered
        } else if p.is_fully_frozen() {
            HeartKind::Frozen
        } else {
            HeartKind::Normal
        };
        self.native_player_heart_type(&mut kind);
        Some(kind)
    }
    /// Whether any mod listens to a hook that receives `ScreenView`.
    pub(in crate::game) fn screen_view_listened(&self) -> bool {
        use mod_api::HookId;
        [
            HookId::ScreenRenderPre,
            HookId::ScreenRenderBackground,
            HookId::ScreenRenderForeground,
            HookId::ScreenRenderPost,
            HookId::ScreenMouseButtonPressedPre,
            HookId::ScreenMouseButtonPressedPost,
            HookId::ScreenMouseButtonReleasedPre,
            HookId::ScreenMouseButtonReleasedPost,
            HookId::ScreenMouseDraggedPre,
            HookId::ScreenMouseDraggedPost,
            HookId::ScreenMouseScrolledPre,
            HookId::ScreenMouseScrolledPost,
            HookId::ScreenKeyPressedPre,
            HookId::ScreenKeyPressedPost,
            HookId::ScreenKeyReleasedPre,
            HookId::ScreenKeyReleasedPost,
            HookId::ScreenCharTypedPre,
            HookId::ScreenCharTypedPost,
            HookId::ScreenOpening,
            HookId::ScreenClosing,
            HookId::ScreenInitPost,
            HookId::ScreenRoom,
            HookId::ScreenExclusionAreas,
        ]
        .into_iter()
        .any(mod_api::__private::listened)
    }
    /// Whether a mod listens to one of the four screen render hooks.
    pub(crate) fn screen_render_listened(&self) -> bool {
        use mod_api::HookId;
        [
            HookId::ScreenRenderPre,
            HookId::ScreenRenderBackground,
            HookId::ScreenRenderForeground,
            HookId::ScreenRenderPost,
        ]
        .into_iter()
        .any(mod_api::__private::listened)
    }
    /// `Render.Pre`: whether the screen draws.
    pub(crate) fn screen_render_begin(&self, draw: &mut HudBatch, screen: &ScreenView<'_>) -> bool {
        let mut canceled = false;
        self.native_screen_render_pre(draw, screen, &mut canceled);
        !canceled
    }
    /// `Render.Post`, including when Pre cancelled the screen's drawing.
    pub(crate) fn screen_render_end(&self, draw: &mut HudBatch, screen: &ScreenView<'_>) {
        self.native_screen_render_post(draw, screen);
    }
    /// Borrowed `ScreenView` of the open screen for this frame.
    pub fn screen_view(&self, batch: &HudBatch, screen: Screen) -> ScreenView<'_> {
        self.screen_view_at(screen, self.screen_rect(batch, screen))
    }
    /// `screen_view` with an explicit panel `rect` for container layout.
    pub(in crate::game) fn screen_view_at(&self, screen: Screen, rect: [f32; 4]) -> ScreenView<'_> {
        // Only the inventory screen shows a server's menu.
        let menu_shown = screen == Screen::Inventory;
        let shown_native = menu_shown.then(|| self.shown_native_screen()).flatten();
        ScreenView::from(mod_api::__private::built::ScreenView {
            id: screen.id(),
            mod_screen: (screen == Screen::Mod).then(|| self.mod_screen()).flatten(),
            menu: self
                .remote_menu_type()
                .filter(|m| menu_shown && !m.is_empty())
                .or_else(|| {
                    shown_native
                        .map(|s| s.menu.name.as_str())
                        .filter(|m| !m.is_empty())
                }),
            title: self
                .client
                .remote
                .as_ref()
                .and_then(|r| r.menu_title_snapshot())
                .filter(|_| menu_shown)
                .or_else(|| shown_native.and_then(|s| s.menu.title.as_ref())),
            rect,
            slots: self.slot_count(),
            mouse: self.cursor(),
            partial_tick: self.common.clock.fraction() as f32,
            regions: self.screen_regions(screen),
        })
    }
    /// Button Post sees the original receiver and its answer; return the final handled bit.
    pub(in crate::game) fn screen_button_post(
        &self,
        receiver: &ObservedScreen,
        button: i32,
        pressed: bool,
        modifiers: u32,
        double_click: bool,
        handled: bool,
    ) -> bool {
        let screen = receiver.view();
        let mut result = TriState::Default;
        if pressed {
            self.native_screen_mouse_button_pressed_post(
                &screen,
                button,
                modifiers,
                double_click,
                handled,
                &mut result,
            );
        } else {
            self.native_screen_mouse_button_released_post(
                &screen,
                button,
                modifiers,
                handled,
                &mut result,
            );
        }
        result.or(handled)
    }
    /// Unhandled scroll/drag notices go to the receiver captured before handling.
    pub(in crate::game) fn screen_mouse_post(&self, post: ScreenMousePost) {
        if post.handled {
            return;
        }
        let screen = post.receiver.view();
        match post.kind {
            ScreenMousePostKind::Scroll(scroll) => {
                self.native_screen_mouse_scrolled_post(&screen, scroll, post.modifiers);
            }
            ScreenMousePostKind::Drag { button, delta } => {
                self.native_screen_mouse_dragged_post(&screen, button, delta, post.modifiers);
            }
        }
    }
    pub(in crate::game) fn native_screen_player_slot(&self, slot: usize) -> Option<(f32, f32)> {
        let [x, y] = self.shown_native_screen()?.player_slot_position(slot)?;
        Some((x, y))
    }

    /// Next-frame position of menu slot `slot` on open mod menu `menu`.
    pub fn screen_slot(&self, menu: &str, slot: usize) -> Option<[f32; 2]> {
        let s = self.shown_native_screen().filter(|s| s.menu.name == menu)?;
        s.slot_moves.latest(slot, s.declared_slot(slot))
    }
    /// Moves, activates, or hides menu slot `slot` on open mod menu `menu` from next frame.
    pub fn set_screen_slot(
        &self,
        menu: &str,
        slot: usize,
        pos: Option<[f32; 2]>,
    ) -> Result<(), mod_api::ScreenSlotRefusal> {
        let s = self
            .shown_native_screen()
            .filter(|s| s.menu.name == menu)
            .ok_or(mod_api::ScreenSlotRefusal::NoScreen)?;
        s.slot_moves.set(slot, s.declared_slot(slot), pos)
    }
    /// Options > Mods rows for loaded code mods, after the content-only rows.
    pub(in crate::game) fn list_native_mods(&mut self) {
        let registered = sunrise_gameplay_common::game::mods::collect_key_mappings(
            self.common.options.execution_role,
            mod_api::__private::key_mappings(),
        );
        let native = self.common.mods.native.clone();
        let mut rows: Vec<_> = self
            .client
            .settings
            .mods
            .installed
            .iter()
            .filter(|row| {
                mod_api::__private::mod_index(&row.id).is_some()
                    && !native.built().iter().any(|m| m.id == row.id)
            })
            .cloned()
            .collect();
        rows.extend(native.packages().iter().map(|package| {
            let m = &package.manifest;
            self.client.settings.mods.info.insert(
                m.id.clone(),
                sunrise_mod_package::mod_list::ModInfo::of(m, None),
            );
            Installed {
                id: m.id.clone(),
                title: format!("{} {}", m.name, m.version),
                description: m.description.clone(),
                problem: None,
                dev: false,
                native: None,
            }
        }));
        self.client
            .settings
            .mods
            .installed
            .retain(|row| !rows.iter().any(|code| code.id == row.id));
        for mut row in rows {
            let settings = self.native_page_settings(&row.id);
            let mappings: Vec<KeyMapping> = registered
                .iter()
                .filter(|(owner, _)| *owner == row.id)
                .map(|(_, k)| KeyMapping {
                    id: k.id.clone(),
                    label: k.label.clone(),
                    key: k.key.clone(),
                    modifiers: k.modifiers.bits,
                    context:
                        sunrise_gameplay_common::game::api_values::TryIntoEngine::try_into_engine(
                            &k.context,
                            &(),
                        )
                        .unwrap(),
                    category: k.category.clone(),
                })
                .collect();
            row.native = Some(NativePage {
                mappings,
                settings,
                own_screen: mod_api::__private::has_config_screen(&row.id),
            });
            self.client.settings.mods.installed.push(row);
        }
    }
    /// The settings a mod registered, with values from the live client store.
    pub(in crate::game) fn native_page_settings(&self, mod_id: &str) -> Vec<NativeSetting> {
        mod_api::__private::settings()
            .iter()
            .filter(|(owner, _)| *owner == mod_id)
            .map(|(owner, spec)| NativeSetting {
                key: spec.key.clone(),
                label: spec.label.clone(),
                tooltip: spec.tooltip.clone(),
                value: ui_setting_value(spec, self.live_setting(owner, spec)),
                category: mod_api::__private::setting_category(mod_id, &spec.key)
                    .map(str::to_owned),
            })
            .collect()
    }
    /// Persists a changed setting on native mod `i`'s page and fires `setting_changed`.
    pub(crate) fn native_setting_event(&mut self, i: usize, j: usize) {
        let Some(row) = self.client.settings.mods.installed.get(i) else {
            return;
        };
        let Some(setting) = row.native.as_ref().and_then(|page| page.settings.get(j)) else {
            return;
        };
        let (id, key) = (row.id.clone(), setting.key.clone());
        let value = match &setting.value {
            NativeSettingValue::Toggle(on) => mod_api::SettingValue::Toggle(*on),
            NativeSettingValue::Choice { index, .. } => mod_api::SettingValue::Choice(*index),
            NativeSettingValue::Range { value, .. } => mod_api::SettingValue::Range(*value),
            NativeSettingValue::Color(rgb) => mod_api::SettingValue::Color(*rgb),
        };
        let Some(handle) = mod_api::__private::setting_key_by_name(&id, &key) else {
            return;
        };
        let Some((owner, spec)) =
            mod_api::__private::setting_entry(handle).map(|(owner, spec)| (*owner, spec))
        else {
            return;
        };
        let root = self.native_settings_root();
        if self
            .apply_native_setting_write(&root, owner, spec, value)
            .is_err()
        {
            let live = self.live_setting(owner, spec);
            if let Some(setting) = self
                .client
                .settings
                .mods
                .installed
                .get_mut(i)
                .and_then(|row| row.native.as_mut())
                .and_then(|page| page.settings.get_mut(j))
            {
                setting.value = ui_setting_value(spec, live);
            }
        }
    }
    #[inline(always)]
    pub(in crate::game) fn sync_native_setting_ui_row(
        &mut self,
        owner: &str,
        spec: &mod_api::SettingSpec,
        value: mod_api::SettingValue,
    ) {
        let Some(page) = self
            .client
            .settings
            .mods
            .installed
            .iter_mut()
            .find(|row| row.id == owner)
            .and_then(|row| row.native.as_mut())
        else {
            return;
        };
        if let Some(setting) = page
            .settings
            .iter_mut()
            .find(|setting| setting.key == spec.key)
        {
            setting.value = ui_setting_value(spec, value);
        }
    }

    /// Refreshes the mod's live settings from disk and rebuilds its page rows.
    pub(crate) fn native_settings_refresh(&mut self, i: usize) {
        let Some(id) = self
            .client
            .settings
            .mods
            .installed
            .get(i)
            .map(|m| m.id.clone())
        else {
            return;
        };
        let root = self.native_settings_root();
        self.refresh_native_settings_owner(&root, &id);
        let settings = self.native_page_settings(&id);
        let row = self.client.settings.mods.installed.get_mut(i);
        if let Some(page) = row.and_then(|m| m.native.as_mut()) {
            page.settings = settings;
        }
    }
    /// Validated category and icon of a custom recipe type, or None before load or outside a world.
    pub(in crate::game) fn recipe_book_category(
        &mut self,
        kind: &str,
    ) -> Option<RecipeBookCategory> {
        if self.common.outside_world() || self.common.res.rest.get().is_none() {
            return None;
        }
        self.registered_recipe_book_category(kind)
    }
}

pub(in crate::game) fn ui_setting_value(
    spec: &mod_api::SettingSpec,
    value: mod_api::SettingValue,
) -> NativeSettingValue {
    use mod_api::{SettingKind, SettingValue};

    match (&spec.kind, value) {
        (_, SettingValue::Toggle(on)) => NativeSettingValue::Toggle(on),
        (SettingKind::Choice { options, .. }, SettingValue::Choice(index)) => {
            NativeSettingValue::Choice {
                options: options.clone(),
                index,
            }
        }
        (SettingKind::Range { min, max, step, .. }, SettingValue::Range(value)) => {
            NativeSettingValue::Range {
                min: *min,
                max: *max,
                step: *step,
                value,
            }
        }
        (_, SettingValue::Color(rgb)) => NativeSettingValue::Color(rgb),
        _ => NativeSettingValue::Toggle(false),
    }
}

#[cfg(any(test, feature = "test-support"))]
impl crate::game::ClientGame<'_> {
    #[inline(always)]
    pub fn test_frame_camera(&mut self, partial: f32) -> Option<(glam::DVec3, f32, f32, f32)> {
        self.frame_camera(partial)
    }

    #[inline(always)]
    pub fn test_commit_screen_transition(&mut self) {
        self.commit_screen_transition();
    }

    pub fn test_title(&self) -> Option<&sunrise_client::title::Title> {
        self.client.title.0.as_ref()
    }
    pub fn test_title_mut(&mut self) -> &mut Option<sunrise_client::title::Title> {
        &mut self.client.title.0
    }
    pub fn test_renderer(&self) -> Option<&sunrise_client_render::gfx::Renderer> {
        self.client.renderer.0.as_deref()
    }
    pub fn test_renderer_mut(&mut self) -> &mut Option<Box<sunrise_client_render::gfx::Renderer>> {
        &mut self.client.renderer.0
    }
    pub fn test_window_mut(&mut self) -> &mut Option<std::sync::Arc<winit::window::Window>> {
        &mut self.client.window.0
    }
    pub fn test_pad(&self) -> &crate::game::pad::Pads {
        &self.client.pad.0
    }
}

#[cfg(test)]
#[cfg(any())]
#[path = "block_particle_effects_tests_client.rs"]
pub(in crate::game) mod block_particle_effects_tests;

#[cfg(test)]
#[path = "shared/slot_icon_tests_client.rs"]
pub(in crate::game) mod slot_icon_tests;

#[cfg(test)]
#[path = "shared/slot_tooltip_tests_client.rs"]
pub(in crate::game) mod slot_tooltip_tests;

/// Which mouse Post hook owes dispatch (`ScreenMousePost`).
pub(in crate::game) enum ScreenMousePostKind {
    Scroll([f64; 2]),
    Drag { button: i32, delta: [f32; 2] },
}

/// The Post a mouse input owes once the screen has handled it, built when
/// its Pre let the input through (`Game::screen_mouse_post`), carrying the
/// receiving screen and modifiers captured at Pre time.
pub(in crate::game) struct ScreenMousePost {
    pub(in crate::game) receiver: ObservedScreen,
    pub(in crate::game) modifiers: u32,
    pub(in crate::game) kind: ScreenMousePostKind,
    pub(in crate::game) handled: bool,
}

impl ScreenMousePost {
    pub(in crate::game) fn new(
        receiver: ObservedScreen,
        modifiers: u32,
        kind: ScreenMousePostKind,
    ) -> Self {
        Self {
            receiver,
            modifiers,
            kind,
            handled: false,
        }
    }
}

/// What a drag's Pre decided (`Game::screen_mouse_dragged`).
pub(in crate::game) enum ScreenMousePre {
    /// No screen hook ran: no drag, a synthetic move or no screen open.
    NotFired,
    Canceled,
    Passed(ScreenMousePost),
}

include!("shared_client.rs");

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
