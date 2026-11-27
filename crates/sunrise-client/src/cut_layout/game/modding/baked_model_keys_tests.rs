//! A client mod borrows a block state's baked variants and quads in `model_baking_completed`.
//! It keeps keys, and a reload supersedes them.
#![cfg(feature = "client")]
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::{
    AtlasSpriteError, BakedModelKey, ClientHost, Hooks, ModEntry, ModelBakingCompletedEvent,
    ModelError, ModelRef,
};
use mod_platform::testing::with_mods;
use std::sync::{Arc, Mutex};
use sunrise_client_gameplay::game::client_side::model_baking_completed_tests::reload;

/// What the mod retained in each `model_baking_completed`: keys and refusals, not views.
#[derive(Default)]
struct Read {
    pub keys: Vec<Result<Vec<BakedModelKey>, ModelError>>,
    invalid_keys: Vec<[ModelError; 2]>,
    /// An oak log lying along x and standing along y: two states of one block, two bindings.
    log_keys: Vec<[Result<Vec<BakedModelKey>, ModelError>; 2]>,
    /// Stone's drawable model, then lookups for an invalid ordinal and an unknown block.
    models: Vec<[Result<ModelRef, ModelError>; 3]>,
}
struct Reader(Arc<Mutex<Read>>);
#[mod_api::__subscriptions]
impl Hooks for Reader {
    fn model_baking_completed(&self, host: &mut dyn ClientHost, _: &mut ModelBakingCompletedEvent) {
        let mut read = self.0.lock().unwrap();
        let stone = host.block_state_id("minecraft:stone").expect("stone");
        let unknown = mod_api::__private::parts_installation::block_key(u32::MAX);
        let assets = host.render_assets();
        let keys = assets.block_state_models(stone).map(|keys| {
            let keys = keys.iter();
            assert_eq!(keys.len(), 4, "stone has four weighted variants");
            keys.inspect(|&key| {
                let view = assets.baked_model(key).expect("stone's baked model");
                assert_eq!(view.quads().count(), 6, "a full cube");
                assert!(
                    view.quads()
                        .all(|quad| quad.sprite == "minecraft:block/stone")
                );
            })
            .collect()
        });
        read.invalid_keys.push([
            assets
                .block_state_models(mod_api::__private::block_state_from_ordinal(
                    stone.block(),
                    u16::MAX,
                ))
                .expect_err("past the last state"),
            assets
                .block_state_models(mod_api::__private::block_state_from_ordinal(unknown, 0))
                .expect_err("unknown block"),
        ]);
        read.log_keys.push(["x", "y"].map(|axis| {
            let state = host
                .block_state_id(&format!("minecraft:oak_log[axis={axis}]"))
                .expect("an oak log state");
            assets
                .block_state_models(state)
                .map(|keys| keys.iter().collect())
        }));
        read.models.push([
            assets.block_state_model(stone),
            assets.block_state_model(mod_api::__private::block_state_from_ordinal(
                stone.block(),
                u16::MAX,
            )),
            assets.block_state_model(mod_api::__private::block_state_from_ordinal(unknown, 0)),
        ]);
        read.keys.push(keys);
    }
}

/// Stone's four weighted variants (26.3's blockstate lists four models, two of them turned half a
/// way round), each a full cube of six quads: read on the first load and again after a reload; the
/// first load's key answers `StaleModel` afterwards and the untouched model reads the same.
#[test]
#[ignore = "daily"]
fn a_client_mod_reads_a_block_states_baked_models_once_per_load() {
    let read = Arc::new(Mutex::new(Read::default()));
    let mods = vec![ModEntry::new("reader", Box::new(Reader(Arc::clone(&read))))].into();
    with_mods(mods, || {
        with_client_game(|game| {
            game.publish_initial_client_models(vec![
                sunrise_client_gameplay::game::client_side::client_reload::PreparedClientReload::Vanilla(()),
            ])
            .unwrap();
            let first = {
                let read = read.lock().unwrap();
                let Ok(keys) = &read.keys[0] else {
                    panic!("stone's keys: {:?}", read.keys);
                };
                assert_eq!(
                    read.invalid_keys,
                    [[ModelError::UnknownModel, ModelError::UnknownModel]],
                    "unknown ordinals and block keys are refused",
                );
                let [Ok(along_x), Ok(along_y)] = &read.log_keys[0] else {
                    panic!("oak log's keys: {:?}", read.log_keys);
                };
                assert!(!along_x.is_empty() && !along_y.is_empty());
                assert_ne!(along_x, along_y, "each axis state selects its own binding");
                let [stone_model, past_ordinal, unknown_block] = &read.models[0];
                let model = stone_model.as_ref().expect("stone's drawable model");
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                assert_eq!(*model, game.native_block_state_model(stone).unwrap());
                let slot = mod_api::__private::game::Fields::fields(model).slot;
                let faces = game.model_faces(slot).expect("drawable faces");
                assert_eq!(faces.len(), 6);
                assert!(
                    faces
                        .iter()
                        .all(|face| face.quad.material.sprite == "minecraft:block/stone")
                );
                assert_eq!(
                    past_ordinal,
                    &Err(ModelError::UnknownModel),
                    "an ordinal past the block's states is refused"
                );
                assert_eq!(
                    unknown_block,
                    &Err(ModelError::UnknownModel),
                    "an unknown block is refused"
                );
                keys[0]
            };
            let previous = Arc::clone(&game.client.client_pack.render);
            reload(game);
            let next_key = read.lock().unwrap().keys[1].as_ref().unwrap()[0];
            assert_ne!(next_key, first, "a new generation's key");
            assert_eq!(
                game.client
                    .client_pack
                    .render
                    .baked_model(first)
                    .unwrap_err(),
                ModelError::StaleModel,
                "the first load's key is superseded"
            );
            let before = previous.baked_model(first).unwrap();
            let after = game
                .client
                .client_pack
                .render
                .baked_model(next_key)
                .unwrap();
            let slot = mod_api::__private::game::Fields::fields(&first).slot;
            let particle_layer = previous
                .model_catalog
                .model_by_slot(slot)
                .unwrap()
                .particle
                .unwrap();
            let expected_particle = previous
                .sprite_layers
                .iter()
                .find_map(|(name, &layer)| (layer == particle_layer).then_some(name.as_str()))
                .expect("stone's particle sprite has an atlas name");
            assert_eq!(before.particle, Some(expected_particle));
            assert_eq!(before, after, "untouched: the same quads and metadata");
            let unknown = BakedModelKey::from(mod_api::__private::built::model_ref(
                game.client.client_pack.render.generation,
                u32::MAX,
            ));
            assert_eq!(
                game.client
                    .client_pack
                    .render
                    .baked_model(unknown)
                    .unwrap_err(),
                ModelError::UnknownModel,
                "an unknown slot in the current generation is refused",
            );

            // Keep the live registry but give the fixture an asset store that bakes no models.
            let cache = tempfile::tempdir().unwrap();
            let root = sunrise_assets::resource_source::ResourceRoot::new(
                game.options.resources.clone(),
                cache.path().to_owned(),
                game.pack.source.clone(),
            );
            game.client.client_pack.render = Arc::new(
                sunrise_client::assets::RenderAssets::load_for_server(&root, &game.pack.assets)
                    .unwrap(),
            );
            let host: &dyn ClientHost = game;
            let stone = host.block_state_id("minecraft:stone").expect("stone");
            let keys = host.render_assets().block_state_models(stone).unwrap();
            assert!(keys.iter().next().is_none());
        });
    });
}

/// `atlas_sprite_pixels` answers a block sprite's frame at its own size: stone's 16 x 16 pixels,
/// one frame of the animated water strip (not the whole strip), and `NoSuchSprite` for a layer no
/// atlas holds.
#[test]
#[ignore = "daily"]
fn a_client_mod_reads_atlas_sprite_pixels() {
    let mods = vec![ModEntry::new(
        "reader",
        Box::new(Reader(Default::default())),
    )]
    .into();
    with_mods(mods, || {
        with_client_game(|game| {
            game.publish_initial_client_models(vec![
                sunrise_client_gameplay::game::client_side::client_reload::PreparedClientReload::Vanilla(()),
            ])
            .unwrap();
            let host: &dyn ClientHost = game;
            let assets = host.render_assets();
            let sprites =
                host.texture_sprites(&["minecraft:block/stone", "minecraft:block/water_still"]);
            let [Some(stone), Some(water)] = sprites[..] else {
                panic!("stone and water are in the blocks atlas: {sprites:?}");
            };
            let read = assets.atlas_sprite_pixels(stone).unwrap();
            assert_eq!((read.width, read.height), (16, 16));
            assert_eq!(read.rgba.len(), 16 * 16 * 4);
            assert!(read.rgba.chunks(4).all(|p| p[3] == 255), "stone is opaque");
            let read = assets.atlas_sprite_pixels(water).unwrap();
            assert!(water.animated);
            assert_eq!((read.width, read.height), (16, 16));
            assert_eq!(read.rgba.len(), 16 * 16 * 4);
            let mut past = stone;
            past.array = u32::MAX;
            assert_eq!(
                assets.atlas_sprite_pixels(past),
                Err(AtlasSpriteError::NoSuchSprite)
            );
        });
    });
}
