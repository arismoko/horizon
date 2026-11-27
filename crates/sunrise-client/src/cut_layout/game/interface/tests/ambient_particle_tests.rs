//! An ambient particle option that names a block or an item draws only what the level's own
//! registry or item table knows. Vanilla's codecs refuse any other while the biome file is
//! decoded (`Registry.byNameCodec`, `Item.CODEC`), and the attribute files are read before a
//! level's registries exist, so the level that draws the option makes the lookup.
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_core::nbt::Tag;
use sunrise_level::particle_options::ParticleType;
use sunrise_world::chunk::BlockState;
use sunrise_world::particle_options::ParticleOptions;
use sunrise_world::particle_type::ParticleType as PersistentType;

#[test]
#[ignore = "daily"]
fn an_ambient_block_or_item_particle_needs_a_name_of_the_level() {
    with_client_game(|game| {
        let item = |id: &str| {
            ParticleOptions::Item(Tag::compound([
                ("id", Tag::String(id.into())),
                ("count", Tag::Int(1)),
            ]))
        };
        let block = |name: &str| {
            ParticleOptions::Block(
                PersistentType::Block,
                BlockState {
                    name: name.to_owned(),
                    properties: Default::default(),
                },
            )
        };
        // At the camera, at `ParticleStatus.ALL`: the minimal setting admits none here.
        let at = game.client.particles.0.camera;
        game.client.settings.particles = 0;
        game.client.particles.0.particle_level = 0;
        game.client.particles.0.requested = Some(Vec::new());
        game.emit_model_ambient(&item("minecraft:apple"), at);
        game.emit_model_ambient(&block("minecraft:stone"), at);
        let drawn: Vec<_> = game
            .client
            .particles
            .0
            .requested
            .as_ref()
            .unwrap()
            .iter()
            .map(|(options, ..)| options.ty)
            .collect();
        assert_eq!(drawn, [ParticleType::Item, ParticleType::Block]);
        game.client.particles.0.requested = Some(Vec::new());
        for unknown in [
            item("test:no_such_item"),
            item("minecraft:air"),
            block("test:no_such_block"),
        ] {
            game.emit_model_ambient(&unknown, at);
        }
        assert!(
            game.client
                .particles
                .0
                .requested
                .as_ref()
                .unwrap()
                .is_empty(),
            "an unknown block or item draws nothing: {:?}",
            game.client.particles.0.requested.as_ref().unwrap(),
        );
    });
}
