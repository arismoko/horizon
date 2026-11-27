use crate::game::ClientGame;
#[allow(unused_imports)]
use mod_api::__private::game::*;
use mod_api::RegistrationContext;
use sunrise_gameplay_common::game::mods::ModsReload;
use sunrise_gameplay_common::game::mods::ModsState;
use sunrise_gameplay_common::game::owner_link::OwnerRequest;
use sunrise_gameplay_common::game::{
    execution_role::ExecutionRole, mods::standalone::collect_standalone_models,
};

#[path = "package_files_client.rs"]
pub(in crate::game) mod package_files;

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// The content mods the resources load with: the ones applied, which a
    /// switch that needs a restart doesn't change.
    pub(in crate::game) fn applied_mods(&self) -> std::collections::BTreeSet<String> {
        self.common
            .mods
            .applied
            .clone()
            .unwrap_or_else(|| self.client.settings.mods.enabled.clone())
    }

    pub(in crate::game) fn client_set_audio_loop_context(
        &mut self,
        audio: Option<sunrise_world::native_load_context::LoopContext>,
    ) {
        if let Some(audio) = audio {
            self.client.audio.0.set_loop_context(audio);
        }
    }

    pub(in crate::game) fn client_retire_disabled_instance_groups(
        &self,
        effective: &std::collections::BTreeSet<String>,
    ) {
        self.client
            .instance_groups
            .borrow_mut()
            .retire_disabled(effective);
    }

    /// A live switch's resources applied: the server reloads its data packs
    /// from the new root, as `/reload` does (recipes, tags, loot tables,
    /// advancements, functions), and sends them to its players.
    pub(in crate::game) fn mods_reloaded(&mut self, applied: bool) {
        let Some(started) = self.common.mods.live.take() else {
            return;
        };
        let resources_ms = started.elapsed().as_millis();
        if !applied {
            return;
        }
        let reload = ModsReload {
            root: self.common.options.resources.clone(),
            data_source: self.common.pack.data_source.clone(),
            data_changes: self.common.pack.data_changes.clone(),
            additional_data: self.common.pack.additional_data.clone(),
            enabled: self.applied_mods(),
            selected: self.common.mods.selected.clone(),
            refused: self.common.mods.refused.clone(),
        };
        log::info!("Sunrise: content mods switched live: resources reloaded in {resources_ms} ms");
        if let Some(server) = &self.client.integrated {
            server.tell(OwnerRequest::ReloadMods(Box::new(reload)));
        }
    }
    /// The player turned content mods on or off (Options > Mods, or an
    /// install from Browse Mods): those with only assets and data apply now,
    /// through the resource reload and then the
    /// server's data reload (`mods_reloaded`); those that add blocks, items or
    /// worldgen wait for a restart.
    pub(in crate::game::client_side) fn switch_mods(&mut self) {
        let Some(applied) = self.common.mods.applied.clone() else {
            return;
        };
        let wanted = self.client.settings.mods.enabled.clone();
        // Ordinary native mods' `mod.<id>` lines switch live (`sync_native_switches`);
        // a serving host bound to active or withheld `served_by` packages, and any content
        // mod with blocks, items, worldgen, or `served_by`, keep their startup state until
        // a restart.
        let content = &self.common.mods.content_ids;
        let mut next = wanted.clone();

        for id in applied.symmetric_difference(&wanted) {
            if !content.contains(id) || self.common.mods.live_ids.contains(id) {
                continue;
            }
            log::info!("Sunrise: mod {id} needs a restart to switch");
            if applied.contains(id) {
                next.insert(id.clone());
            } else {
                next.remove(id);
            }
        }
        let (_, host_waiting) = self
            .common
            .mods
            .reconcile_code_switches(&self.client.settings.mods.disabled);
        for id in host_waiting {
            log::info!("Sunrise: serving host mod {id} needs a restart to switch");
        }
        let changed = applied
            .symmetric_difference(&next)
            .any(|id| content.contains(id));
        if !changed {
            self.common.mods.applied = Some(next);
            return;
        }
        log::info!(
            "Sunrise: content mods switched live: {:?} -> {:?}",
            applied,
            next
        );
        self.common.mods.applied = Some(next);
        self.common.mods.live = Some(std::time::Instant::now());
        self.reload_packs();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_add_hook_panic_toasts(&mut self) {
        self.add_hook_panic_toasts();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_open_title(&mut self, saves: std::path::PathBuf) {
        self.client.title.0 = Some(self.new_title(saves));
    }
}

/// The native mods' standalone models (`native_standalone_models`), set once
/// before the resources first load.
pub fn register_standalone_models(role: ExecutionRole) {
    if let Some(registrations) = collect_standalone_models(role) {
        sunrise_client::models::register_standalone(registrations);
    }
    register_fluid_models(role);
}

/// Recollect before each resource bake, retaining registration-phase attribution.
pub(in crate::game) fn register_fluid_models(role: ExecutionRole) {
    if let Some(models) = collect_fluid_models(role) {
        sunrise_client::fluid_models::register(models);
    }
}

pub(in crate::game) fn collect_fluid_models(
    role: ExecutionRole,
) -> Option<Vec<sunrise_mod_package::content::FluidModel>> {
    if !role.is_client() {
        return None;
    }
    let mut models = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_register_fluid_models(
        &RegistrationContext::new(),
        &mut models,
    );
    Some(
        models
            .into_iter()
            .map(|model| sunrise_mod_package::content::FluidModel {
                fluid: model.fluid,
                still: model.still,
                flowing: model.flowing,
                overlay: model.overlay,
            })
            .collect(),
    )
}

/// Freeze the native mods' entity model layer definitions at startup, then bake their skull
/// types (`native_create_skull_models`) from those definitions or vanilla's under `vanilla_root`.
pub fn register_layer_definitions(
    role: ExecutionRole,
    vanilla_root: &std::path::Path,
) -> Result<(), sunrise_client::mobs::model_layers::RegistrationError> {
    if !role.is_client() {
        return Ok(());
    }
    let mut suppliers = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_register_layer_definitions(
        &RegistrationContext::new(),
        &mut suppliers,
    );
    sunrise_client::mobs::model_layers::register(suppliers, vanilla_root)?;
    let mut models = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_create_skull_models(
        &RegistrationContext::new(),
        &mut models,
    );
    sunrise_client::skull_models::register(models, vanilla_root);
    Ok(())
}

/// Freeze the native mods' F3 text callbacks and renderer toggles at client startup.
pub fn register_debug_entries(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut entries = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_register_debug_entries(
        &RegistrationContext::new(),
        &mut entries,
    );
    sunrise_client::debug_entries::register(entries);
}

/// Install the native mods' HUD draw callbacks among the vanilla anchors once at client startup.
pub fn register_gui_layers(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut layers = sunrise_client::gui_layers::GuiLayers::default();
    crate::game::client_side::native_hooks_client::register_gui_layers::native_register_gui_layers(
        &RegistrationContext::new(),
        &mut layers,
    );
    sunrise_client::gui_layers::freeze(layers);
}

/// Install the native mods' client tooltip component factories once at client startup.
pub fn register_client_tooltip_component_factories(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut factories = sunrise_client::tooltip_components::TooltipComponentFactories::default();
    crate::game::client_side::native_hooks_client::register_client_tooltip_component_factories::native_register_client_tooltip_component_factories(
        &RegistrationContext::new(),
        &mut factories,
    );
    sunrise_client::tooltip_components::freeze(factories);
}

/// Collect the native mods' owned debug renderers and their owners on each list refresh.
pub(in crate::game) fn debug_renderers(
    role: ExecutionRole,
    entries: &dyn mod_api::__private::DebugEntryStates,
) -> Vec<mod_api::Answered<mod_api::DebugRenderer>> {
    let mut renderers = Vec::new();
    if role.is_client() {
        sunrise_gameplay_common::game::native_hooks::native_register_debug_renderers(
            &RegistrationContext::new(),
            entries,
            &mut renderers,
        );
    }
    renderers
}

/// The native mods' numeric item model properties
/// (`native_register_range_select_item_model_property`), kept once at start, before
/// resources load, so `range_dispatch` models on them read the mod's number.
pub fn register_range_select_item_model_properties(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut properties = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_register_range_select_item_model_property(
        &RegistrationContext::new(),
        &mut properties,
    );
    sunrise_client::item_render::install_range_property_factories(properties);
}

/// The native mods' categorical item model properties
/// (`native_register_select_item_model_property`), kept once at start, before resources
/// load, so `select` models on them read the mod's value.
pub fn register_select_item_model_properties(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut properties = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_register_select_item_model_property(
        &RegistrationContext::new(),
        &mut properties,
    );
    sunrise_client::item_render::install_select_property_factories(properties);
}

/// The types native mods register for resource definitions, kept once at start, before
/// resources load: item tint sources (`native_register_item_tint_sources`), so `tints` entries on
/// them read the mod's colour; atlas sprite sources (`native_register_sprite_sources`), so atlas
/// definitions' entries on them list the mod's sprites; model loaders
/// (`native_register_model_loaders`), so models naming one get the mod's quads; item model node
/// types (`native_register_item_models`), so item definitions' nodes of them draw what the mod
/// answers; and animation targets and interpolations (`native_register_json_animation_types`),
/// bound directly to their callbacks so animation files resolve and convert them at parse time.
/// Block-state
/// definition types (`native_register_block_state_models`), so blockstate files naming one get
/// the models the mod picks; and special item model factories
/// (`native_register_special_model_renderer`), so each special model node of a registered type
/// bakes its own renderer.
pub fn register_resource_sources(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut sources = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_register_item_tint_sources(
        &RegistrationContext::new(),
        &mut sources,
    );
    sunrise_client::item_render::install_item_tint_factories(sources);
    let mut sources = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_register_sprite_sources(
        &RegistrationContext::new(),
        &mut sources,
    );
    sunrise_client_render::mod_sprite_sources::register(sources);
    let mut loaders = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_register_model_loaders(
        &RegistrationContext::new(),
        &mut loaders,
    );
    sunrise_client_render::mod_model_loaders::register(loaders);
    let mut models = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_register_item_models(
        &RegistrationContext::new(),
        &mut models,
    );
    sunrise_client::item_render::install_item_model_factories(models);
    let mut types = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_register_json_animation_types(
        &RegistrationContext::new(),
        &mut types,
    );
    sunrise_client_render::mod_animations::register(types);
    let mut definitions = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_register_block_state_models(
        &RegistrationContext::new(),
        &mut definitions,
    );
    sunrise_client_render::mod_block_state_models::register(definitions);
    register_special_model_renderers();
}

/// The native mods' special item model renderers (`native_register_special_model_renderer`),
/// kept once at start, before resources load: their types are what a `minecraft:special` model
/// may name besides vanilla's, or in place of one. A later registration of a type replaces the
/// earlier one, and the log names both mods.
pub(in crate::game) fn register_special_model_renderers() {
    let mut registry = mod_api::SpecialModelRendererRegistry::default();
    sunrise_gameplay_common::game::native_hooks::native_register_special_model_renderer(
        &RegistrationContext::new(),
        &mut registry,
    );
    for (kind, owner) in registry.initial_owners() {
        if sunrise_assets::item_model::SpecialModel::is_builtin_type(&kind.to_string()) {
            log::warn!(
                "Sunrise: special model type {kind} registered by the game is replaced by {}'s registration",
                owner.unwrap_or("the game")
            );
        }
    }
    let (factories, replaced) = registry.into_renderers();
    for replaced in replaced {
        let name = |id: Option<&str>| id.unwrap_or("the game").to_owned();
        log::warn!(
            "Sunrise: special model type {} registered by {} is replaced by {}'s registration",
            replaced.kind,
            name(replaced.earlier),
            name(replaced.later)
        );
    }
    sunrise_client::item_render::install_special_model_factories(
        factories
            .into_iter()
            .filter_map(|(kind, (owner, factory))| {
                let owner = owner?;
                Some((
                    kind,
                    sunrise_client::item_render::OwnedSpecialModelFactory::new(owner, factory),
                ))
            })
            .collect(),
    );
}

/// The client presentation registrations, kept once at start, before resources load: the
/// dimension transition screens, the entity spectator shaders, the render pipelines and the entity
/// renderers.
pub fn register_client_presentations(
    role: ExecutionRole,
    vanilla_root: &std::path::Path,
) -> sunrise_game_data::Result<()> {
    register_preset_editors(role);
    register_dimension_transition_screens(role);
    register_entity_spectator_shaders(role);
    register_render_pipelines(role, vanilla_root)?;
    register_pipeline_modifiers(role)?;
    register_entity_renderers(role);
    Ok(())
}

/// Resolve client dimension-transition registrations once, before the first level opens.
pub(in crate::game) fn register_dimension_transition_screens(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut screens = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_register_dimension_transition_screen(
        &RegistrationContext::new(),
        &mut screens,
    );
    sunrise_client_render::dimension_transition::register(screens);
}

/// Resolve client spectator post-effect registrations once, before the first level opens.
pub(in crate::game) fn register_entity_spectator_shaders(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut shaders = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_register_entity_spectator_shaders(
        &RegistrationContext::new(),
        &mut shaders,
    );
    sunrise_client_render::spectator_shaders::register(shaders);
}

pub(in crate::game) fn register_preset_editors(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut editors = Vec::new();
    sunrise_gameplay_common::game::native_hooks::register_preset_editors::native_register_preset_editors(
        &RegistrationContext::new(),
        &mut editors,
    );
    sunrise_client::title::preset_editors::register(editors);
}

/// Keep the mods' render pipeline registrations once at startup, before resources load: every
/// load builds them from its own shader files.
pub(in crate::game) fn register_render_pipelines(
    role: ExecutionRole,
    vanilla_root: &std::path::Path,
) -> sunrise_game_data::Result<()> {
    if !role.is_client() {
        return Ok(());
    }
    let mut pipelines = Vec::new();
    let mut render_types = Vec::new();
    let mut compute_pipelines = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_register_render_pipelines(
        &RegistrationContext::new(),
        &mut pipelines,
        &mut render_types,
        &mut compute_pipelines,
    );
    sunrise_client_render::mod_pipelines::register(pipelines, vanilla_root)?;
    sunrise_client_render::mod_pipelines::register_render_types(render_types, vanilla_root)?;
    sunrise_client_render::mod_compute::register(compute_pipelines)?;
    Ok(())
}

/// Keep the mods' pipeline modifier registrations once at startup, before resources load.
fn register_pipeline_modifiers(role: ExecutionRole) -> sunrise_game_data::Result<()> {
    if !role.is_client() {
        return Ok(());
    }
    let mut modifiers = Vec::new();
    crate::game::client_side::native_hooks_client::native_register_pipeline_modifiers(
        &RegistrationContext::new(),
        &mut modifiers,
    );
    sunrise_client_render::mod_pipelines::register_modifiers(modifiers)?;
    Ok(())
}

/// The native mods' meshing registrations, kept once at start, before resources load: colour
/// resolvers, then the block tint sources that read them, then section geometry renderers.
pub fn register_meshing(role: ExecutionRole) {
    register_color_resolvers(role);
    register_block_tint_sources(role);
    add_section_geometry(role);
}

/// The native mods' colour resolvers (`native_register_color_resolvers`), kept once at start,
/// before the block tint sources that read them and before resources load.
pub(in crate::game) fn register_color_resolvers(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut registry = mod_api::ColorResolverRegistry::default();
    sunrise_gameplay_common::game::native_hooks::native_register_color_resolvers(
        &RegistrationContext::new(),
        &mut registry,
    );
    sunrise_client::mesher::register_color_resolvers(registry.into_resolvers());
}

/// The native mods' block tint sources (`native_register_block_tint_sources`), kept once at
/// start, before resources load; every resource load applies them to its mesh tables.
pub(in crate::game) fn register_block_tint_sources(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut registry = mod_api::BlockTintRegistry::default();
    sunrise_gameplay_common::game::native_hooks::native_register_block_tint_sources(
        &RegistrationContext::new(),
        &mut registry,
    );
    sunrise_client::mesher::register_block_tint_sources(registry.into_layers());
}

/// The native mods' section geometry renderers (`native_add_section_geometry`), kept once at
/// start, before resources load; every mesh of a section asks them to add their quads.
pub(in crate::game) fn add_section_geometry(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut registry = mod_api::SectionGeometryRegistry::default();
    sunrise_gameplay_common::game::native_hooks::native_add_section_geometry(
        &RegistrationContext::new(),
        &mut registry,
    );
    sunrise_client::mesher::register_section_geometry(registry.into_renderers());
}

/// The native mods' entity renderers (`native_register_entity_renderers`), kept once at start,
/// before resources load; every resource load resolves each type's renderer.
pub fn register_entity_renderers(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut registry = mod_api::EntityRendererRegistry::default();
    sunrise_gameplay_common::game::native_hooks::native_register_entity_renderers(
        &RegistrationContext::new(),
        &mut registry,
    );
    #[cfg(feature = "client")]
    crate::game::client_side::native_entity_renderers::register(registry.into_renderers());
}

/// The native mods' particle providers (`native_register_particle_providers`) and render groups
/// (`native_register_particle_groups`) are registered once at startup. Each particle engine
/// constructs its groups once; sprite-set loads associate types with those groups and providers.
pub fn register_particles(role: ExecutionRole) -> sunrise_game_data::Result<()> {
    if !role.is_client() {
        return Ok(());
    }
    let mut registry = mod_api::ParticleProviderRegistry::default();
    sunrise_gameplay_common::game::native_hooks::native_register_particle_providers(
        &RegistrationContext::new(),
        &mut registry,
    );
    sunrise_client::particles::register_particle_providers(registry.into_providers());
    let mut group_factories = Vec::new();
    sunrise_gameplay_common::game::native_hooks::native_register_particle_groups(
        &RegistrationContext::new(),
        &mut group_factories,
    );
    sunrise_client::particles::register_particle_groups(group_factories)?;
    Ok(())
}

pub(in crate::game) fn content_root(
    root: &std::path::Path,
    sources: &ModsState,
) -> sunrise_game_data::Result<sunrise_assets::resource_source::ResourceRoot> {
    let set_fluid_looks =
        if sources.installation_side == sunrise_mod_package::manifest::Side::Server {
            sunrise_sim::mod_content::set_fluid_looks
        } else {
            sunrise_client::mod_content_client::set_fluid_looks
        };
    sunrise_gameplay_common::game::mods::content_root_with(root, sources, set_fluid_looks)
}

#[cfg(test)]
mod living_type_tests {
    use sunrise_gameplay_common::game::mods::with_living_mod_type_fixture;

    /// A content mod's living type is a mob of its template's species under
    /// its own id, with its own attributes: it saves its id with every field
    /// its template's class saves, loads back as itself, and rolls its own
    /// loot table. A thrown type stays a projectile.
    #[test]
    fn a_mod_living_type_is_its_templates_mob_with_its_own_attributes() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::{Mob, Species, drops::death_loot_table};
        let (types, models) = with_living_mod_type_fixture(|root, types| {
            let models = sunrise_client_lib::mobs_render::ClientMobModels::load(root).unwrap();
            (types, models)
        });
        let (zombie, ghoul) = (&types["minecraft:zombie"], &types["ruby:ghoul"]);
        assert_eq!(ghoul.species, Species::Zombie);
        assert_eq!(ghoul.class_id(), "minecraft:zombie");
        assert_eq!(types["ruby:settler"].species, Species::Villager);
        assert!(!types.contains_key("ruby:bolt"), "a thrown type is no mob");
        assert_eq!((zombie.max_health, ghoul.max_health), (20.0, 40.0));
        assert_eq!(ghoul.speed.to_bits(), zombie.speed.to_bits());
        assert_eq!(
            (
                ghoul.sounds,
                ghoul.model_name(),
                models.texture("ruby:ghoul")
            ),
            (
                zombie.sounds,
                zombie.model_name(),
                models.texture("minecraft:zombie"),
            )
        );
        assert!(models.texture("ruby:ghoul").is_some());

        let at = Vec3::new(0.5, 64.0, 0.5);
        let (zombie, ghoul) = (
            Mob::new(1, zombie.clone(), at, 0.0),
            Mob::new(2, ghoul.clone(), at, 0.0),
        );
        assert_eq!(ghoul.health, 40.0);
        let saved = |m: &Mob| {
            sunrise_sim::entity_nbt::mob_to_nbt(
                m,
                [0, 0, 0, 2],
                None,
                None,
                |_| None,
                &Default::default(),
            )
        };
        let keys = |tag: &sunrise_core::nbt::Tag| -> Vec<String> {
            let sunrise_core::nbt::Tag::Compound(map) = tag else {
                panic!("a mob saves a compound");
            };
            map.keys().map(|k| k.to_string_lossy()).collect()
        };
        let (zombie_tag, ghoul_tag) = (saved(&zombie), saved(&ghoul));
        let Some(sunrise_core::nbt::Tag::String(id)) = ghoul_tag.get("id") else {
            panic!("a saved mob has its id");
        };
        assert_eq!(id.to_string_lossy(), "ruby:ghoul");
        assert_eq!(
            keys(&ghoul_tag),
            keys(&zombie_tag),
            "it saves what a zombie saves"
        );
        for key in ["IsBaby", "CanBreakDoors", "DrownedConversionTime"] {
            assert!(keys(&ghoul_tag).iter().any(|k| k == key), "{key}");
        }
        let loaded = sunrise_sim::entity_nbt::mob_from_nbt(&ghoul_tag, &types, 3, None)
            .expect("it loads back");
        assert_eq!(
            (loaded.kind.id.as_str(), loaded.health),
            ("ruby:ghoul", 40.0)
        );
        assert_eq!(
            death_loot_table(&ghoul, true).as_deref(),
            Some("ruby:entities/ghoul")
        );
        assert_eq!(
            death_loot_table(&zombie, true).as_deref(),
            Some("minecraft:entities/zombie")
        );
    }
}

#[cfg(all(test, feature = "client"))]
mod tests {
    use super::{ExecutionRole, RegistrationContext};

    #[test]
    fn fluid_models_recollection_changes_model_identifiers() {
        use mod_api::{FluidModel, Hooks, ModEntry, RegisterFluidModelsEvent};
        use mod_platform::testing::with_mods;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        #[derive(Default)]
        struct Models(Arc<AtomicUsize>);
        #[mod_api::sunrise_mod]
        impl Hooks for Models {
            fn register_fluid_models(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterFluidModelsEvent<'_>,
            ) {
                let generation = self.0.load(Ordering::SeqCst);
                let mut model = FluidModel::default();
                model.fluid = "reload_fluid:oil".into();
                model.still = Some(format!("reload_fluid:block/oil_{generation}"));
                event.register(model);
            }
        }
        let generation = Arc::new(AtomicUsize::new(0));
        with_mods(
            vec![ModEntry::new(
                "reload_fluid",
                Box::new(Models(generation.clone())),
            )]
            .into(),
            || {
                mod_platform::__private::with_test_packages(&["reload_fluid"], || {
                    sunrise_client::fluid_models::set_world_looks([
                        &sunrise_mod_package::content::FluidLook {
                            id: "reload_fluid:oil".into(),
                            sprite: None,
                            tint: None,
                            world: Some(Default::default()),
                            bucket_item: None,
                            measures: Default::default(),
                        },
                    ]);
                    super::register_fluid_models(ExecutionRole::Client);
                    assert_eq!(
                        sunrise_client::fluid_models::resolved()[0].still,
                        "reload_fluid:block/oil_0"
                    );
                    generation.store(1, Ordering::SeqCst);
                    super::register_fluid_models(ExecutionRole::Client);
                    assert_eq!(
                        sunrise_client::fluid_models::resolved()[0].still,
                        "reload_fluid:block/oil_1"
                    );
                    // An item's fluid layer follows the same reload.
                    let item_still = || {
                        sunrise_client::fluid_models::item_appearance("reload_fluid:oil")
                            .map(|appearance| appearance.still)
                    };
                    assert_eq!(item_still().as_deref(), Some("reload_fluid:block/oil_1"));
                    sunrise_client::fluid_models::register(vec![]);
                    sunrise_client::fluid_models::set_world_looks([]);
                    assert_eq!(item_still(), None);
                });
            },
        );
    }

    #[test]
    fn fluid_models_collect_only_on_the_client() {
        use mod_api::FluidModel;
        use mod_api::{Hooks, ModEntry, RegisterFluidModelsEvent};
        use mod_platform::testing::with_mods;
        #[derive(Default)]
        struct Models;
        #[mod_api::sunrise_mod]
        impl Hooks for Models {
            fn register_fluid_models(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterFluidModelsEvent<'_>,
            ) {
                let mut model = FluidModel::default();
                model.fluid = "registration_test:oil".into();
                model.flowing = Some("registration_test:block/oil_flow".into());
                model.overlay = Some("registration_test:block/oil_overlay".into());
                event.register(model);
            }
        }
        with_mods(
            vec![ModEntry::new("registration_test", Box::new(Models))].into(),
            || {
                mod_platform::__private::with_test_packages(&["registration_test"], || {
                    for server in [
                        ExecutionRole::IntegratedServer,
                        ExecutionRole::DedicatedServer,
                    ] {
                        assert_eq!(super::collect_fluid_models(server), None);
                    }
                    let models = super::collect_fluid_models(ExecutionRole::Client).unwrap();
                    assert_eq!(models.len(), 1);
                    assert_eq!(models[0].fluid, "registration_test:oil");
                    assert_eq!(
                        models[0].flowing.as_deref(),
                        Some("registration_test:block/oil_flow")
                    );
                    assert_eq!(
                        models[0].overlay.as_deref(),
                        Some("registration_test:block/oil_overlay")
                    );
                });
            },
        );
    }
}

#[cfg(all(test, feature = "client"))]
mod startup_registration_tests {
    use super::*;
    use mod_api::{
        CubeListBuilder, Hooks, Identifier, LayerDefinition, MeshDefinition, ModEntry, PartPose,
        PipelineBlend, RegisterLayerDefinitionsEvent, RegisterRenderPipelinesEvent, RenderPipeline,
        WorldDepth,
    };
    use std::sync::{Arc, Mutex};
    use sunrise_client::mobs::model_layers::{self, RegistrationError as LayerError};
    use sunrise_client_render::mod_pipelines::{self, RegistrationError as PipelineError};

    #[test]
    #[ignore = "daily"]
    fn startup_register_layer_definitions_fails_closed_before_publishing() {
        if sunrise_gameplay_common::game::test_fixtures::run_fresh_cases(
            "game::client_side::mods_client::startup_registration_tests::startup_register_layer_definitions_fails_closed_before_publishing",
            "SUNRISE_LAYER_REGISTRATION_CHILD",
            &["startup"],
        ) {
            return;
        }
        struct LayerMod(&'static [(&'static str, i32)], Arc<Mutex<Vec<i32>>>);
        #[mod_api::__subscriptions]
        impl Hooks for LayerMod {
            fn register_layer_definitions(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterLayerDefinitionsEvent<'_>,
            ) {
                for &(id, offset) in self.0 {
                    let calls = self.1.clone();
                    event.register(id, move || {
                        calls.lock().unwrap().push(offset);
                        let mut mesh = MeshDefinition::new();
                        mesh.add_or_replace_child(
                            mesh.root(),
                            "head",
                            CubeListBuilder::create(),
                            PartPose::offset(0.0, offset as f32, 0.0),
                        );
                        LayerDefinition::create(mesh, 64, 32)
                    });
                }
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        let table = dir.join("entity-models.json");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let failure = || register_layer_definitions(ExecutionRole::Client, dir).unwrap_err();
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "alpha",
                Box::new(LayerMod(&[("alpha:golem#outer:armor", 4)], calls.clone())),
            )]
            .into(),
            || {
                assert!(matches!(failure(), LayerError::Load(e) if e.is::<std::io::Error>()));
                std::fs::write(&table, "not json").unwrap();
                assert!(matches!(failure(), LayerError::Load(e) if e.is::<serde_json::Error>()));
                for input in ["[]", "{}"] {
                    std::fs::write(&table, input).unwrap();
                    assert!(matches!(failure(), LayerError::NotNonemptyObject));
                }
                std::fs::write(&table, r#"{"invalid": {}}"#).unwrap();
                assert!(
                    matches!(failure(), LayerError::InvalidVanillaLayer(id) if id == "invalid")
                );
                assert!(calls.lock().unwrap().is_empty());
                assert!(model_layers::registered("alpha:golem#outer:armor").is_none());
            },
        );

        std::fs::write(
            &table,
            r#"{"minecraft:zombie#main": {"pose": [0,0,0,0,0,0,1,1,1], "children": {}}}"#,
        )
        .unwrap();

        for id in ["minecraft:zombie#main", "BadMod:golem#main"] {
            let layers = if id == "minecraft:zombie#main" {
                &[("alpha:valid#main", 1), ("minecraft:zombie#main", 2)][..]
            } else {
                &[("alpha:valid#main", 1), ("BadMod:golem#main", 2)][..]
            };
            mod_platform::testing::with_mods(
                vec![ModEntry::new(
                    "alpha",
                    Box::new(LayerMod(layers, calls.clone())),
                )]
                .into(),
                || {
                    let err = failure();
                    match err {
                        LayerError::VanillaCollision {
                            owner: "alpha",
                            id: failed,
                        } => {
                            assert_eq!(id, "minecraft:zombie#main");
                            assert_eq!(failed, id);
                        }
                        LayerError::InvalidModLayer {
                            owner: "alpha",
                            id: failed,
                        } => {
                            assert_eq!(id, "BadMod:golem#main");
                            assert_eq!(failed, id);
                        }
                        other => panic!("unexpected registration refusal: {other:?}"),
                    }
                    assert!(calls.lock().unwrap().is_empty());
                    assert!(model_layers::registered("alpha:valid#main").is_none());
                },
            );
        }

        // Valid startup registration freezes with last-writer-wins and unrestricted suffix.
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new(
                    "alpha",
                    Box::new(LayerMod(&[("alpha:golem#outer:armor", 4)], calls.clone())),
                ),
                ModEntry::new(
                    "beta",
                    Box::new(LayerMod(&[("alpha:golem#outer:armor", 8)], calls.clone())),
                ),
            ]
            .into(),
            || {
                register_layer_definitions(ExecutionRole::Client, dir).unwrap();
                assert_eq!(*calls.lock().unwrap(), [8]);
                assert_eq!(
                    model_layers::registered("alpha:golem#outer:armor")
                        .unwrap()
                        .bake_root()
                        .get_child("head")
                        .unwrap()
                        .y,
                    8.0
                );
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn startup_register_render_pipelines_rejects_collisions_before_publishing() {
        if sunrise_gameplay_common::game::test_fixtures::run_fresh_cases(
            "game::client_side::mods_client::startup_registration_tests::startup_register_render_pipelines_rejects_collisions_before_publishing",
            "SUNRISE_PIPELINE_REGISTRATION_CHILD",
            &["startup"],
        ) {
            return;
        }
        struct PipeMod(&'static [(&'static str, bool)]);
        #[mod_api::__subscriptions]
        impl Hooks for PipeMod {
            fn register_render_pipelines(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterRenderPipelinesEvent<'_>,
            ) {
                for &(id, optional) in self.0 {
                    let parsed = Identifier::parse(id).unwrap();
                    let mut p = RenderPipeline::new(
                        parsed.clone(),
                        parsed,
                        PipelineBlend::Translucent,
                        WorldDepth::Tested,
                    );
                    if optional {
                        p = p.optional();
                    }
                    event.register(p);
                }
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        let table = dir.join("render-pipelines.json");
        let register = || register_client_presentations(ExecutionRole::Client, dir);
        let failure = || *register().unwrap_err().downcast::<PipelineError>().unwrap();
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "example",
                Box::new(PipeMod(&[("example:glow", false)])),
            )]
            .into(),
            || {
                assert!(matches!(failure(), PipelineError::Read { path, error }
                    if path == table && error.kind() == std::io::ErrorKind::NotFound));
                std::fs::write(&table, r#"{"required":["INVALID"],"optional":[]}"#).unwrap();
                assert!(matches!(failure(), PipelineError::InvalidIdTable(path) if path == table));
                assert!(mod_pipelines::registered().is_empty());
            },
        );
        std::fs::write(
            &table,
            r#"{"required":["minecraft:pipeline/solid_block"],"optional":["minecraft:pipeline/wireframe"]}"#,
        )
        .unwrap();

        // Vanilla collisions apply in the corresponding policy table.
        const VANILLA_COLLISIONS: &[&[(&str, bool)]] = &[
            &[("minecraft:pipeline/solid_block", false)],
            &[("minecraft:pipeline/wireframe", true)],
        ];
        for &colliding in VANILLA_COLLISIONS {
            mod_platform::testing::with_mods(
                vec![ModEntry::new("bad", Box::new(PipeMod(colliding)))].into(),
                || {
                    let (expected_id, expected_optional) = colliding[0];
                    assert!(
                        matches!(failure(), PipelineError::VanillaCollision { owner, id, optional }
                        if owner == "bad" && id == Identifier::parse(expected_id).unwrap() && optional == expected_optional)
                    );
                    assert!(mod_pipelines::registered().is_empty());
                },
            );
        }

        for duplicate in [&[("shared:glow", false)][..], &[("shared:glow", true)][..]] {
            mod_platform::testing::with_mods(
                vec![
                    ModEntry::new("first", Box::new(PipeMod(&[("shared:glow", false)]))),
                    ModEntry::new("second", Box::new(PipeMod(duplicate))),
                ]
                .into(),
                || {
                    assert!(
                        matches!(failure(), PipelineError::DuplicateModId { earlier, owner, id }
                        if earlier == "first" && owner == "second" && id == Identifier::parse("shared:glow").unwrap())
                    );
                    assert!(mod_pipelines::registered().is_empty());
                },
            );
        }

        // A mod ID may match the opposite vanilla table, but cannot name two mod pipelines.
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new(
                    "first",
                    Box::new(PipeMod(&[
                        ("shared:glow", false),
                        ("minecraft:pipeline/wireframe", false),
                    ])),
                ),
                ModEntry::new(
                    "second",
                    Box::new(PipeMod(&[
                        ("shared:other_glow", true),
                        ("minecraft:pipeline/solid_block", true),
                    ])),
                ),
            ]
            .into(),
            || {
                register().unwrap();
                let published = mod_pipelines::registered();
                assert_eq!(published.len(), 4);
                assert!(!published[0].optional);
                assert!(published[2].optional);
            },
        );
    }
}
