use super::*;
use std::path::Path;
use sunrise_assets::resource_source::ResourceRoot;
use sunrise_protocol::registry::Builtin;
use sunrise_sim::mobs::spawner::SpawnerTypeFacts;
use sunrise_world::{
    attributes::entity::Catalog as Attributes, block_render::FaceVisibility,
    item_defaults::ItemDefaults,
};
use {
    crate::game::client_side::resource_tables_client::ClientMobTables,
    crate::game::client_side::resource_tables_client::ClientResources,
    crate::game::client_side::resource_tables_client::load_animate_tables,
    sunrise_gameplay_common::game::world_rest_common,
    sunrise_gameplay_common::game::world_rest_common::RestPresentation,
    sunrise_gameplay_common::game::world_rest_common::ServerRestPresentation,
};

pub(in crate::game) type LoadedWorldRest = (WorldRest, Option<ClientResources>);

struct ClientRestPresentation<'a> {
    entity_models: Option<&'a sunrise_client::mobs::EntityModelDocument>,
    visibility: FaceVisibility,
    builtin: Arc<Builtin>,
}

type PlayModels = (
    Option<Arc<MobType>>,
    [Option<sunrise_client::mobs::Part>; 2],
    [Option<sunrise_client::mobs::Part>; 2],
    Option<sunrise_client::mobs::Part>,
);
type ClientTables = (
    std::result::Result<BlockEntityRenderer, String>,
    std::result::Result<
        (
            sunrise_client::animate_tick::AnimateTables,
            sunrise_client::fog::FogTables,
        ),
        String,
    >,
    sunrise_client::music::MusicTables,
    sunrise_client::ambience::AmbientTables,
);

impl RestPresentation for ClientRestPresentation<'_> {
    type MobTables = ClientMobTables;
    type PlayModels = PlayModels;
    type ClientTables = ClientTables;
    type Output = ClientResources;

    fn mob_types(
        &self,
        root: &Path,
        defaults: &ItemDefaults,
        catalog: &Arc<Attributes>,
    ) -> Result<MobTypes> {
        MobTypes::load_with_defaults(root, defaults.entries(), catalog)
    }

    fn mob_tables(
        &self,
        resource_root: &ResourceRoot,
        _: &HashMap<String, Arc<MobType>>,
    ) -> Result<Self::MobTables> {
        let root = resource_root.prepared();
        let models = match self.entity_models {
            Some(document) => {
                sunrise_client::mobs_render::ClientMobModels::load_with_document(root, document)?
            }
            None => sunrise_client::mobs_render::ClientMobModels::load(root)?,
        };
        let data = sunrise_client::mobs_render::ClientMobData::load(resource_root, &models)?;
        Ok(ClientMobTables { models, data })
    }

    fn play_models(&self, root: &Path) -> Self::PlayModels {
        let player_type = match self.entity_models {
            Some(document) => perspective::player_type_with_document(root, document),
            None => perspective::player_type(root),
        };
        (
            player_type,
            player_arms(root, "minecraft:player#main", self.entity_models),
            player_arms(root, "minecraft:player_slim#main", self.entity_models),
            match self.entity_models {
                Some(document) => document.model("minecraft:spin_attack#main"),
                None => sunrise_client::mobs::load_model(root, "minecraft:spin_attack#main"),
            },
        )
    }

    fn client_tables<'scope>(
        &'scope self,
        scope: Option<&'scope std::thread::Scope<'scope, '_>>,
        resource_root: &'scope ResourceRoot,
        resources: &'scope Resources,
        assets: &'scope Assets,
    ) -> Self::ClientTables {
        let root = resource_root.prepared();
        let registry = assets.registry;
        let facts = resources.facts();
        let biomes = resources.biomes();
        let builtin = &self.builtin;
        fn text<T, E: std::fmt::Display>(
            result: std::result::Result<T, E>,
        ) -> std::result::Result<T, String> {
            result.map_err(|e| e.to_string())
        }
        let load_block_entity_render = move || -> Result<_> {
            let _step = startup::step("rest.block_entity_render");
            let mut renderer = BlockEntityRenderer::load_with_visibility(
                resource_root,
                registry,
                self.visibility.clone(),
            )?;
            renderer.add_mod_signs(facts, registry);
            Ok(renderer)
        };
        let load_animate = move || load_animate_tables(root, registry, biomes);

        if let Some(scope) = scope {
            let block_entity_render = named_scope(scope, "block_entity_render", move || {
                text(load_block_entity_render())
            });
            let animate = named_scope(scope, "animate", move || text(load_animate()));
            let music = named_scope(scope, "music", move || {
                startup::time("rest.music", || {
                    sunrise_client::music::MusicTables::load(root, builtin.clone())
                })
            });
            let ambient = named_scope(scope, "ambient", move || {
                startup::time("rest.ambient", || {
                    sunrise_client::ambience::AmbientTables::load(root, builtin.clone())
                })
            });
            (
                block_entity_render.join().unwrap(),
                animate.join().unwrap(),
                music.join().unwrap(),
                ambient.join().unwrap(),
            )
        } else {
            (
                text(load_block_entity_render()),
                text(load_animate()),
                startup::time("rest.music", || {
                    sunrise_client::music::MusicTables::load(root, builtin.clone())
                }),
                startup::time("rest.ambient", || {
                    sunrise_client::ambience::AmbientTables::load(root, builtin.clone())
                }),
            )
        }
    }

    fn finish(
        &self,
        tables: Self::ClientTables,
        client_mobs: Self::MobTables,
        models: Self::PlayModels,
    ) -> Result<Self::Output> {
        let (block_entity_render, animate, music, ambient) = tables;
        let (animate, fog) = animate?;
        let (player_type, player_arm, player_arm_slim, spin_attack) = models;
        Ok(ClientResources {
            block_entity_render: block_entity_render?,
            animate,
            fog,
            music,
            ambient,
            player_type,
            player_arm,
            player_arm_slim,
            spin_attack,
            client_mobs,
        })
    }
}

pub(in crate::game) fn load_world_rest(
    resource_root: &sunrise_assets::resource_source::ResourceRoot,
    conditions: &sunrise_world::conditions::LoadConditions,
    resources: &Resources,
    spawner_types: HashMap<String, SpawnerTypeFacts>,
    assets: &Assets,
    behaviour_json: &serde_json::Value,
    gameplay_projection: &[sunrise_items::blocks::GameplayProjection],
    entity_models: Option<&sunrise_client::mobs::EntityModelDocument>,
    item_defaults: &sunrise_world::item_defaults::ItemDefaults,
    visibility: sunrise_world::block_render::FaceVisibility,
    progress: &Progress,
    started: Instant,
    draws: bool,
) -> Result<LoadedWorldRest> {
    if draws {
        let presentation = ClientRestPresentation {
            entity_models,
            visibility,
            builtin: sunrise_gameplay_common::game::villager_client::builtin_registries(
                resource_root.prepared(),
            )
            .unwrap_or_default(),
        };
        world_rest_common::load_world_rest(
            resource_root,
            conditions,
            resources,
            spawner_types,
            assets,
            behaviour_json,
            gameplay_projection,
            item_defaults,
            progress,
            started,
            true,
            &presentation,
        )
        .map(|(rest, client)| (rest, Some(client)))
    } else {
        world_rest_common::load_world_rest(
            resource_root,
            conditions,
            resources,
            spawner_types,
            assets,
            behaviour_json,
            gameplay_projection,
            item_defaults,
            progress,
            started,
            true,
            &ServerRestPresentation,
        )
        .map(|(rest, ())| (rest, None))
    }
}

/// The right and left arms of a player model layer (`PlayerModel.rightArm`
/// and `leftArm`, with their sleeves), each keeping the layer.
fn player_arms(
    root: &std::path::Path,
    key: &str,
    document: Option<&sunrise_client::mobs::EntityModelDocument>,
) -> [Option<sunrise_client::mobs::Part>; 2] {
    let model = match document {
        Some(document) => document.model(key),
        None => sunrise_client::mobs::load_model(root, key),
    };
    ["right_arm", "left_arm"].map(|name| model.as_ref().and_then(|m| m.detached_child(name)))
}
